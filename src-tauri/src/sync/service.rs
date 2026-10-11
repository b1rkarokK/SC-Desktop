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

struct Session {
    handle: Handle,
    hub: hub::HubClient,
    channel: String,
    host: bool,
    host_hash: Option<String>,
    /// `Some(password)` for the link we hand out
    announce: Arc<std::sync::atomic::AtomicBool>,
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
    let url = state.config.get().sync_url;
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

    pub async fn leave(&self, state: &AppState) {
        let s = self.session.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(s) = s {
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

    pub async fn open(&self, app: &AppHandle, state: &AppState, channel: String, password: String, host: bool, settings: Settings) -> AppResult<()> {
        let base = server(state)?;
        self.leave(state).await;
        let me = me_member(state);
        let (hub, events) = hub::connect(base.clone(), channel.clone(), 0);
        let port: Arc<dyn Port> = Arc::new(PlayerPort { app: app.clone() });
        let app2 = app.clone();
        let on_view: Arc<dyn Fn(&RoomView) + Send + Sync> = Arc::new(move |v| {
            let _ = app2.emit("room:view", v);
        });
        let me_id = me.id.clone();
        let handle = engine::start(Params { channel: channel.clone(), password: password.clone(), me, host, settings: settings.clone() }, Arc::new(hub.clone()), events, port, on_view);

        // what the player reports goes to the engine
        let tx = handle.tx.clone();
        app.listen("room:loaded", move |ev| {
            let ok = serde_json::from_str::<serde_json::Value>(ev.payload()).ok().and_then(|v| v["ok"].as_bool()).unwrap_or(true);
            let _ = tx.send(Cmd::Loaded { ok });
        });
        if host {
            let tx = handle.tx.clone();
            app.listen("player:ended", move |_| {
                let _ = tx.send(Cmd::TrackEnded);
            });
        }

        // the host says "I am here" for the friends, every 30 s while the room is open
        let announce = Arc::new(std::sync::atomic::AtomicBool::new(host));
        let mut hash = None;
        if host {
            if let Some(u) = state.me() {
                let h = host_hash(u.id);
                hash = Some(h.clone());
                let (flag, view, base, name, ch) = (announce.clone(), handle.view.clone(), base, u.username.clone(), channel.clone());
                tauri::async_runtime::spawn(async move {
                    let Ok(client) = http() else { return };
                    while flag.load(std::sync::atomic::Ordering::SeqCst) {
                        let live = {
                            let v = view.lock().unwrap_or_else(|e| e.into_inner());
                            LiveRoom { channel: ch.clone(), name: v.name.clone(), has_password: v.has_password, members: v.members.len().max(1), host_name: name.clone(), host_id: u.id }
                        };
                        let blob = crypto::seal(&dir_key(&h), &h, &serde_json::to_vec(&live).unwrap_or_default());
                        let _ = client.put(format!("{base}/dir/{h}")).body(blob).send().await;
                        tokio::time::sleep(Duration::from_secs(30)).await;
                    }
                });
            }
        }
        let _ = me_id;
        *self.session.lock().unwrap_or_else(|e| e.into_inner()) = Some(Session { handle, hub, channel, host, host_hash: hash, announce });
        Ok(())
    }
}

pub fn default_settings(name: String, password: bool, access: Access, who_can_queue: Who) -> Settings {
    Settings { name, has_password: password, access, who_can_queue, ..Settings::default() }
}
