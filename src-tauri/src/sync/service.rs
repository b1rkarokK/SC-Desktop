//! Rooms in the app: ties the engine (engine.rs) to the real player, the relay (hub.rs) and the
//! list of friends' live rooms (/dir). One room at a time.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Listener, Manager};

use super::{
    crypto,
    engine::{self, Cmd, Handle, Params, Port, RoomView},
    hub,
};
use crate::{
    error::{AppError, AppResult},
    models::TrackDto,
    player::Player,
    rooms::{Access, Member, QueueItem, Settings, Status, Who},
    state::AppState,
};

pub const LINK_PREFIX: &str = "scdesk://room/";

static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

struct Session {
    handle: Handle,
    hub: hub::HubClient,
    channel: String,
    host: bool,
    host_hash: Option<String>,
    /// `Some(password)` for the link we hand out
    announce: Arc<std::sync::atomic::AtomicBool>,
    listeners: Vec<tauri::EventId>,
}

const MINE: &str = "rooms:mine";

/// A room in "Мои комнаты": it stays when we leave it, nobody has to be there.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MyRoom {
    pub channel: String,
    pub name: String,
    pub password: String,
    #[serde(default)]
    pub known: Vec<Known>,
    #[serde(default)]
    pub queue_len: usize,
    #[serde(default)]
    pub current: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Known {
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
pub struct MineView {
    pub channel: String,
    pub name: String,
    pub has_password: bool,
    /// people in the room right now
    pub online: u64,
    pub active: bool,
    pub queue_len: usize,
    pub current: Option<String>,
    pub known: Vec<Known>,
}

pub async fn mine_load(state: &AppState) -> Vec<MyRoom> {
    state.db.kv_get::<Vec<MyRoom>>(MINE).await.ok().flatten().map(|(v, _)| v).unwrap_or_default()
}

async fn mine_save(state: &AppState, list: &[MyRoom]) {
    let _ = state.db.kv_put(MINE, &list.to_vec()).await;
}

/// Add the room or refresh what is known about it.
async fn mine_upsert(state: &AppState, room: MyRoom) {
    let mut list = mine_load(state).await;
    match list.iter_mut().find(|r| r.channel == room.channel) {
        Some(r) => *r = room,
        None => list.push(room),
    }
    mine_save(state, &list).await;
}

pub async fn mine_forget(state: &AppState, channel: &str) {
    let mut list = mine_load(state).await;
    list.retain(|r| r.channel != channel);
    mine_save(state, &list).await;
}

/// How many are in a room right now (the relay says it in its greeting); None if it does not answer.
async fn peers_in(base: &str, channel: &str) -> Option<u64> {
    let (client, mut events) = hub::connect(base.to_owned(), channel.to_owned(), u64::MAX);
    let got = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(e) = events.recv().await {
            if let hub::HubEvent::Connected { peers, .. } = e {
                return Some(peers);
            }
        }
        None
    })
    .await
    .ok()
    .flatten();
    client.close();
    got
}

#[derive(Default)]
pub struct Rooms {
    session: Mutex<Option<Session>>,
}

// ------------------------------------------------------------------- port

struct PlayerPort {
    app: AppHandle,
}

impl PlayerPort {
    fn player(&self) -> Arc<Player> {
        Arc::clone(self.app.state::<Arc<Player>>().inner())
    }
}

impl Port for PlayerPort {
    fn prepare(&self, item: &QueueItem) {
        let (app, id) = (self.app.clone(), item.track_id);
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            let track = match state.db.tracks_by_ids(vec![id]).await.ok().and_then(|v| v.into_iter().next()) {
                Some(t) => Some(t),
                None => match state.sc() {
                    Ok(sc) => sc.track(id).await.ok(),
                    Err(_) => None,
                },
            };
            let Some(t) = track else {
                let _ = app.emit("room:loaded", serde_json::json!({ "ok": false }));
                return;
            };
            let _ = state.db.upsert_tracks(vec![t.clone()]).await;
            app.state::<Arc<Player>>().inner().clone().room_prepare(TrackDto::from(&t));
        });
    }

    fn start_after(&self, delay: Duration) {
        let player = self.player();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            player.resume();
        });
    }

    fn set_playing(&self, playing: bool, pos_ms: i64) {
        let p = self.player();
        p.seek(pos_ms.max(0) as u64);
        if playing {
            p.resume();
        } else {
            p.pause();
        }
    }

    fn position_ms(&self) -> Option<i64> {
        let p = self.player();
        let s = p.snapshot();
        s.track.as_ref().map(|_| p.position_ms() as i64)
    }

    fn trim(&self, factor: f32) {
        self.player().room_trim(factor);
    }

    fn stop(&self) {
        self.player().room_trim(1.0);
        self.player().pause();
    }
}

// -------------------------------------------------------------- directory

fn host_hash(user_id: u64) -> String {
    hex::encode(&Sha256::digest(format!("scdesk-host|{user_id}").as_bytes())[..16])
}

#[derive(Serialize, Deserialize, Clone)]
pub struct LiveRoom {
    pub channel: String,
    pub name: String,
    pub has_password: bool,
    pub members: usize,
    pub host_name: String,
    #[serde(default)]
    pub host_id: u64,
}

fn http() -> AppResult<reqwest::Client> {
    reqwest::Client::builder().timeout(Duration::from_secs(10)).build().map_err(|e| AppError::Other(e.to_string()))
}

fn server(state: &AppState) -> AppResult<String> {
    let url = crate::config::sync_base(&state.config.get().sync_url);
    if url.trim().is_empty() {
        return Err(AppError::Other(crate::lang::pick(
            "Укажите адрес сервера синхронизации в Настройках: без него комнаты не работают.",
            "Set the sync server address in Settings: rooms do not work without it.",
        )
        .into()));
    }
    Ok(url.trim().trim_end_matches('/').to_owned())
}

fn dir_key(hash: &str) -> crypto::Key {
    crypto::room_key(hash, "")
}

/// Rooms of the people given (the ones you follow) that are live right now.
pub async fn live_rooms(state: &AppState, host_ids: Vec<u64>) -> AppResult<Vec<LiveRoom>> {
    let base = server(state)?;
    let hashes: Vec<(String, u64)> = host_ids.into_iter().take(200).map(|id| (host_hash(id), id)).collect();
    if hashes.is_empty() {
        return Ok(Vec::new());
    }
    let q = hashes.iter().map(|(h, _)| h.as_str()).collect::<Vec<_>>().join(",");
    let text = http()?.get(format!("{base}/dir?hosts={q}")).send().await.map_err(|e| AppError::Other(e.to_string()))?.text().await.map_err(|e| AppError::Other(e.to_string()))?;
    let list: Vec<serde_json::Value> = serde_json::from_str(&text).map_err(|e| AppError::Other(e.to_string()))?;
    let mut out = Vec::new();
    for e in list {
        let (Some(host), Some(blob)) = (e["host"].as_str(), e["blob"].as_str()) else { continue };
        let Some(plain) = crypto::open(&dir_key(host), host, blob) else { continue };
        if let Ok(mut r) = serde_json::from_slice::<LiveRoom>(&plain) {
            r.host_id = hashes.iter().find(|(h, _)| h == host).map(|(_, id)| *id).unwrap_or(0);
            out.push(r);
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------ rooms

#[derive(Serialize)]
pub struct Created {
    pub channel: String,
    pub link: String,
}

fn me_member(state: &AppState) -> Member {
    let me = state.me();
    let (id, name, avatar) = match me {
        Some(u) => (u.id.to_string(), u.username, u.avatar_url),
        None => (crypto::new_channel_id(), "Гость".into(), None),
    };
    Member { id, name, avatar_url: avatar, status: Status::Connecting, pubkey: None, last_seen: 0 }
}

/// A link, a code, or a bare channel id → the channel id.
pub fn channel_of(input: &str) -> Option<String> {
    let t = input.trim();
    let id = t.strip_prefix(LINK_PREFIX).unwrap_or(t);
    let id = id.split(['?', '#', '/']).next().unwrap_or("");
    (id.len() >= 16 && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).then(|| id.to_owned())
}

impl Rooms {
    pub fn view(&self) -> RoomView {
        match &*self.session.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(s) => s.handle.view.lock().unwrap_or_else(|e| e.into_inner()).clone(),
            None => RoomView::default(),
        }
    }

    /// The settings as the room shows them now (to change one thing and send them back).
    pub fn view_settings(&self) -> Settings {
        let v = self.view();
        Settings { name: v.name, has_password: v.has_password, who_can_queue: if v.who_can_queue_all { Who::Everyone } else { Who::HostOnly }, ..Settings::default() }
    }

    pub fn cmd(&self, c: Cmd) {
        if let Some(s) = &*self.session.lock().unwrap_or_else(|e| e.into_inner()) {
            let _ = s.handle.tx.send(c);
        }
    }

    /// Our rooms with who is in them now (a short look at the relay for each one we are not in).
    pub async fn mine(&self, state: &AppState) -> Vec<MineView> {
        let list = mine_load(state).await;
        let active = {
            let v = self.view();
            v.active.then(|| (v.channel, v.members.iter().filter(|m| m.status != "connecting" || m.you).count() as u64))
        };
        let base = server(state).ok();
        let looks = list.iter().map(|r| {
            let (base, channel, here) = (base.clone(), r.channel.clone(), active.clone());
            async move {
                match (&here, base) {
                    (Some((c, n)), _) if *c == channel => *n,
                    (_, Some(base)) => peers_in(&base, &channel).await.unwrap_or(0),
                    _ => 0,
                }
            }
        });
        let online = futures::future::join_all(looks).await;
        list.into_iter()
            .zip(online)
            .map(|(r, online)| MineView {
                active: active.as_ref().is_some_and(|(c, _)| *c == r.channel),
                has_password: !r.password.is_empty(),
                channel: r.channel,
                name: r.name,
                online,
                queue_len: r.queue_len,
                current: r.current,
                known: r.known,
            })
            .collect()
    }

    /// Enter one of our rooms again (taking the host's place if nobody is there).
    pub async fn enter(&self, app: &AppHandle, state: &AppState, channel: &str) -> AppResult<()> {
        let Some(r) = mine_load(state).await.into_iter().find(|r| r.channel == channel) else {
            return Err(AppError::Other(crate::lang::pick("Такой комнаты нет в списке", "That room is not in your list").into()));
        };
        let settings = Settings { name: r.name.clone(), ..Settings::default() };
        self.open(app, state, r.channel, r.password, false, settings).await
    }

    pub async fn leave(&self, state: &AppState) {
        let s = self.session.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(s) = s {
            for id in &s.listeners {
                self.app_unlisten(id);
            }
            let _ = s.handle.tx.send(Cmd::Leave);
            s.announce.store(false, std::sync::atomic::Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(250)).await;
            s.hub.close();
            if let (Some(h), Ok(base)) = (s.host_hash, server(state)) {
                if let Ok(c) = http() {
                    let _ = c.delete(format!("{base}/dir/{h}")).send().await;
                }
            }
            let _ = &s.channel;
            let _ = s.host;
        }
    }

    fn app_unlisten(&self, id: &tauri::EventId) {
        if let Some(app) = APP.get() {
            app.unlisten(*id);
        }
    }

    pub async fn open(&self, app: &AppHandle, state: &AppState, channel: String, password: String, host: bool, settings: Settings) -> AppResult<()> {
        let base = server(state)?;
        self.leave(state).await;
        let me = me_member(state);
        let (hub, events) = hub::connect(base.clone(), channel.clone(), 0);
        let port: Arc<dyn Port> = Arc::new(PlayerPort { app: app.clone() });
        let me_id = me.id.clone();
        let _ = APP.set(app.clone());
        // what we know of the room, for "Мои комнаты": kept up to date from what the room shows
        let mine_state = Arc::new(Mutex::new(String::new()));
        let (app_v, ch_v, pw_v) = (app.clone(), channel.clone(), password.clone());
        let on_view: Arc<dyn Fn(&RoomView) + Send + Sync> = Arc::new(move |v: &RoomView| {
            let _ = app_v.emit("room:view", v);
            // the Discord status: in a room (or not any more)
            let info = (v.active && v.ended.is_none() && !v.name.is_empty()).then(|| crate::discord::RoomInfo {
                name: v.name.clone(),
                members: v.members.iter().filter(|m| m.status != "connecting" || m.you).count().max(1),
                open: !v.has_password,
                channel: v.channel.clone(),
            });
            app_v.state::<Arc<Player>>().discord_room(info);
            if v.ended.is_some() || v.name.is_empty() {
                return;
            }
            let summary = serde_json::to_string(&(&v.name, v.queue.len(), v.current.as_ref().map(|c| &c.title), v.members.iter().map(|m| (&m.id, &m.name)).collect::<Vec<_>>())).unwrap_or_default();
            {
                let mut last = mine_state.lock().unwrap_or_else(|e| e.into_inner());
                if *last == summary {
                    return;
                }
                *last = summary;
            }
            let (app, v, ch, pw) = (app_v.clone(), v.clone(), ch_v.clone(), pw_v.clone());
            tauri::async_runtime::spawn(async move {
                let state = app.state::<AppState>();
                let mut known: Vec<Known> = mine_load(&state).await.into_iter().find(|r| r.channel == ch).map(|r| r.known).unwrap_or_default();
                for m in &v.members {
                    match known.iter_mut().find(|k| k.id == m.id) {
                        Some(k) => k.name = m.name.clone(),
                        None => known.push(Known { id: m.id.clone(), name: m.name.clone() }),
                    }
                }
                mine_upsert(&state, MyRoom { channel: ch, name: v.name.clone(), password: pw, known, queue_len: v.queue.len(), current: v.current.as_ref().map(|c| c.title.clone()) }).await;
            });
        });
        let handle = engine::start(Params { channel: channel.clone(), password: password.clone(), me, host, settings: settings.clone() }, Arc::new(hub.clone()), events, port, on_view);
        // a room we open is ours from now on, even before it has said its name
        if mine_load(state).await.iter().all(|r| r.channel != channel) {
            mine_upsert(state, MyRoom { channel: channel.clone(), name: settings.name.clone(), password: password.clone(), ..Default::default() }).await;
        }

        // what the player reports goes to the engine (for as long as we are in the room)
        let mut listeners = Vec::new();
        let tx = handle.tx.clone();
        listeners.push(app.listen("room:loaded", move |ev| {
            let ok = serde_json::from_str::<serde_json::Value>(ev.payload()).ok().and_then(|v| v["ok"].as_bool()).unwrap_or(true);
            let _ = tx.send(Cmd::Loaded { ok });
        }));
        let tx = handle.tx.clone();
        listeners.push(app.listen("player:ended", move |_| {
            let _ = tx.send(Cmd::TrackEnded);
        }));

        // whoever is the host now says "I am here" for the friends, every 30 s
        let announce = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let mut hash = None;
        if let Some(u) = state.me() {
            let h = host_hash(u.id);
            hash = Some(h.clone());
            let (flag, view, base, name, ch) = (announce.clone(), handle.view.clone(), base, u.username.clone(), channel.clone());
            tauri::async_runtime::spawn(async move {
                let Ok(client) = http() else { return };
                let mut announcing = false;
                while flag.load(std::sync::atomic::Ordering::SeqCst) {
                    let (host_now, live) = {
                        let v = view.lock().unwrap_or_else(|e| e.into_inner());
                        (v.host, LiveRoom { channel: ch.clone(), name: v.name.clone(), has_password: v.has_password, members: v.members.len().max(1), host_name: name.clone(), host_id: u.id })
                    };
                    if host_now {
                        let blob = crypto::seal(&dir_key(&h), &h, &serde_json::to_vec(&live).unwrap_or_default());
                        let _ = client.put(format!("{base}/dir/{h}")).body(blob).send().await;
                        announcing = true;
                    } else if announcing {
                        let _ = client.delete(format!("{base}/dir/{h}")).send().await;
                        announcing = false;
                    }
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            });
        }
        let _ = me_id;
        *self.session.lock().unwrap_or_else(|e| e.into_inner()) = Some(Session { handle, hub, channel, host, host_hash: hash, announce, listeners });
        Ok(())
    }
}

pub fn default_settings(name: String, password: bool, access: Access, who_can_queue: Who) -> Settings {
    Settings { name, has_password: password, access, who_can_queue, ..Settings::default() }
}
