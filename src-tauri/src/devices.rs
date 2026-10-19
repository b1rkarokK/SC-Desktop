//! «Телефон»: the user's own devices (this computer and the phone with the SC Desk app) find each other over the
//! same encrypted relay the rooms use, and one of them can play for the other, like Spotify Connect.
//!
//! * **Pairing** (a QR code): the computer opens a one-off channel `pr-…` with a random key and shows both as a QR.
//!   The phone scans it, joins the channel and says hello. After the user allows it here, the computer sends over
//!   that channel the login of the SoundCloud account and the key of the permanent link `dl-…`. Everything on the
//!   wire is sealed (sync/crypto.rs), the relay sees only ciphertext.
//!   Two ways round: when this computer is signed in, it GIVES the account to the phone (above); when it is not,
//!   its code is marked `m=login` and a phone that is signed in GIVES its account to the computer (`account`, after
//!   the user here confirms it); either way the two get the link at the end.
//! * **The link** `dl-…`: the devices say they are there (`pres`), the one that plays tells what (`st`), any of them
//!   can order another (`cmd`: play, pause, next, prev, seek, vol) or hand the music over (`xfer`: the queue, the
//!   place in it, the position; the receiver fetches the tracks with its own login).
//!
//! * **The servers of the subscription for the phone** (`vpnreq` → `vpn`): the phone does not fetch the VPN
//!   subscription itself (that would count as one more device there); it asks this computer, which has it, and after
//!   the user here agrees sends the servers (as sing-box outbounds) over the sealed link.
//!
//! The phone's side is android/…/Link.kt; the messages are the same JSON.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Listener, Manager};

use crate::{
    error::{AppError, AppResult},
    models::TrackDto,
    player::{Player, QueueSource},
    state::AppState,
    sync::{
        crypto::{self, Key},
        hub::{self, HubClient, HubEvent},
    },
};

const LINK_KEY: &str = "devices:link";
/// a device that has not been heard for this long is shown as away
const AWAY_SECS: i64 = 100;
const PRESENCE_EVERY: Duration = Duration::from_secs(30);
/// while it plays, the position is told this often (the others count on from it)
const STATE_EVERY_MS: i64 = 5_000;
/// the queue handed over is cut to this many tracks
const XFER_MAX: usize = 100;

#[derive(Serialize, Deserialize, Clone)]
struct Link {
    chan: String,
    /// base64
    key: String,
    /// this device
    dev: String,
    name: String,
}

#[derive(Clone)]
struct Peer {
    name: String,
    kind: String,
    seen: i64,
    remote: Option<Value>,
}

struct Pairing {
    key: Key,
    chan: String,
    client: HubClient,
    /// the phone that asked: (device, name), until the user answers
    asking: Option<(String, String)>,
    /// this computer is not signed in: the code asks the phone for its account
    take: bool,
    /// the account a phone offered, until the user here answers: (device, phone name, message)
    offer: Option<(String, String, Value)>,
}

#[derive(Default)]
struct Inner {
    link: Option<Link>,
    client: Option<HubClient>,
    peers: HashMap<String, Peer>,
    pairing: Option<Pairing>,
    /// what was told about this player last: (track, playing, volume in %, when ms)
    told: (Option<u64>, bool, u32, i64),
    /// a phone that asked for the servers of the subscription, until the user answers: (device, name)
    vpn_ask: Option<(String, String)>,
    /// this computer is silent because a phone listens to the room instead of it: (the phone, the volume it had)
    muted: Option<(String, f32)>,
}

pub struct Devices {
    app: AppHandle,
    inner: Mutex<Inner>,
}

#[derive(Serialize)]
pub struct DeviceDto {
    pub dev: String,
    pub name: String,
    pub kind: String,
    pub online: bool,
    /// what it plays now (for a device that is the player): the state it last told
    pub remote: Option<Value>,
}

#[derive(Serialize)]
pub struct DevicesView {
    pub linked: bool,
    pub name: String,
    pub peers: Vec<DeviceDto>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn computer_name() -> String {
    std::env::var("COMPUTERNAME").ok().filter(|n| !n.is_empty()).unwrap_or_else(|| "Компьютер".into())
}

/// `scdesk://pair?c=<channel>&k=<key, base64url>`: what the QR code holds.
/// `take`: this computer has no account and asks the phone for one (`&m=login`).
pub fn pair_url(chan: &str, key: &Key, take: bool) -> String {
    use base64::Engine;
    format!("scdesk://pair?c={chan}&k={}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key), if take { "&m=login" } else { "" })
}

fn track_json(t: &TrackDto) -> Value {
    json!({ "id": t.id, "title": t.title, "artist": t.artist, "dur": t.duration_ms, "art": t.artwork_url })
}

impl Devices {
    /// At the start: picks up the saved link, listens to the player and says "I am here" from time to time.
    pub async fn start(app: &AppHandle) {
        let me = Arc::new(Devices { app: app.clone(), inner: Mutex::new(Inner::default()) });
        let saved = app.state::<AppState>().db.kv_get::<Link>(LINK_KEY).await.ok().flatten().map(|(l, _)| l);
        if let Some(link) = saved {
            me.inner.lock().unwrap_or_else(|e| e.into_inner()).link = Some(link);
            me.connect_link();
        }
        app.manage(me.clone());
        let for_state = me.clone();
        app.listen("player:state", move |ev| {
            if let Ok(v) = serde_json::from_str::<Value>(ev.payload()) {
                for_state.on_player_state(&v);
            }
        });
        let ticker = me.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(PRESENCE_EVERY).await;
                ticker.say_present();
                ticker.unmute_stale();
            }
        });
        // a test aid of the development build only: a flag file in the temp folder starts the pairing and
        // the address of the QR is written next to it (a phone on a cable or Wi-Fi ADB can then open it)
        #[cfg(debug_assertions)]
        {
            let tester = me.clone();
            tauri::async_runtime::spawn(async move {
                let flag = std::env::temp_dir().join("scdesk-debug-pair");
                loop {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    if flag.exists() {
                        let _ = std::fs::remove_file(&flag);
                        match tester.pair_start() {
                            Ok(url) => {
                                let _ = std::fs::write(flag.with_extension("url"), url);
                            }
                            Err(e) => {
                                let _ = std::fs::write(flag.with_extension("url"), format!("error: {e}"));
                            }
                        }
                    }
                }
            });
        }
    }

    fn key_of(link: &Link) -> Option<Key> {
        crypto::key_from_b64(&link.key)
    }

    fn connect_link(self: &Arc<Self>) {
        let Some(link) = self.inner.lock().unwrap_or_else(|e| e.into_inner()).link.clone() else { return };
        let Some(key) = Self::key_of(&link) else { return };
        let base = crate::config::sync_base(&self.app.state::<AppState>().config.get().sync_url);
        let (client, mut rx) = hub::connect(base, link.chan.clone(), 0);
        if let Some(old) = self.inner.lock().unwrap_or_else(|e| e.into_inner()).client.replace(client) {
            old.close();
        }
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(ev) = rx.recv().await {
                match ev {
                    HubEvent::Connected { .. } => this.say_present(),
                    HubEvent::Msg { blob, .. } => {
                        if let Some(plain) = crypto::open(&key, &link.chan, &blob) {
                            if let Ok(v) = serde_json::from_slice::<Value>(&plain) {
                                this.on_link_message(&link.dev, v);
                            }
                        }
                    }
                    HubEvent::Disconnected => {}
                }
            }
        });
    }

    fn send_link(&self, v: Value) {
        let (client, link) = {
            let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            (g.client.clone(), g.link.clone())
        };
        let (Some(client), Some(link)) = (client, link) else { return };
        let Some(key) = Self::key_of(&link) else { return };
        client.send(crypto::seal(&key, &link.chan, v.to_string().as_bytes()), false, None);
    }

    /// Like send_link, but the relay keeps the message for the phone that is not connected right now (the answer to a
    /// request that took long: the user went to the computer to confirm while the phone screen went off); a newer message
    /// with the same id replaces it.
    fn send_link_kept(&self, v: Value, id: &str) {
        let (client, link) = {
            let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            (g.client.clone(), g.link.clone())
        };
        let (Some(client), Some(link)) = (client, link) else { return };
        let Some(key) = Self::key_of(&link) else { return };
        client.send(crypto::seal(&key, &link.chan, v.to_string().as_bytes()), true, Some(id.to_owned()));
    }

    fn say_present(&self) {
        let Some(link) = self.inner.lock().unwrap_or_else(|e| e.into_inner()).link.clone() else { return };
        self.send_link(json!({ "t": "pres", "dev": link.dev, "name": link.name, "kind": "pc", "ts": now_ms() }));
    }

    fn changed(&self) {
        let _ = self.app.emit("devices:changed", ());
    }

    // ------------------------------------------------------------ the link

    fn on_link_message(self: &Arc<Self>, me: &str, v: Value) {
        let from = v["dev"].as_str().unwrap_or("").to_owned();
        if from.is_empty() || from == me {
            return;
        }
        // addressed to somebody else
        if let Some(to) = v["to"].as_str() {
            if to != me {
                return;
            }
        }
        match v["t"].as_str().unwrap_or("") {
            "pres" => {
                let new = {
                    let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                    let new = !g.peers.contains_key(&from);
                    let p = g.peers.entry(from.clone()).or_insert(Peer { name: String::new(), kind: "phone".into(), seen: 0, remote: None });
                    p.name = v["name"].as_str().unwrap_or("").to_owned();
                    p.kind = v["kind"].as_str().unwrap_or("phone").to_owned();
                    p.seen = now_ms() / 1000;
                    new
                };
                if new {
                    // a device that has just come: it should know about us at once
                    self.say_present();
                    self.tell_state(true);
                } else {
                    // a known device came back to the screen (the phone opened the app): it wants to see what plays now
                    self.tell_state(false);
                }
                self.changed();
            }
            "st" => {
                {
                    let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(p) = g.peers.get_mut(&from) {
                        p.seen = now_ms() / 1000;
                        p.remote = Some(v.clone());
                    }
                }
                self.changed();
            }
            "cmd" => self.run_command(&v),
            // the equalizer is the same on the devices: what one sets, the others take (10 bands, ±12 dB)
            "eq" => self.take_eq(&v["eq"]),
            "vpnreq" => {
                let name = self.inner.lock().unwrap_or_else(|e| e.into_inner()).peers.get(&from).map(|p| p.name.clone()).unwrap_or_else(|| "Телефон".into());
                self.inner.lock().unwrap_or_else(|e| e.into_inner()).vpn_ask = Some((from.clone(), name.clone()));
                let _ = self.app.emit("devices:vpn-request", json!({ "dev": from, "name": name }));
                // a test aid of the development build only: a flag file in the temp folder answers "yes" by itself
                #[cfg(debug_assertions)]
                if std::env::temp_dir().join("scdesk-debug-vpn").exists() {
                    tracing::info!("devices: vpn request answered by the debug flag");
                    self.vpn_answer(true);
                }
            }
            "xfer" => {
                let this = self.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = this.take_over(&v).await {
                        tracing::warn!(error = %e, "devices: handover failed");
                    }
                });
            }
            // the phone asks this computer for what only it has (friends, shared playlists, rooms, the search inside
            // the lyrics): the answer goes back sealed over the same link
            "rpc" => {
                let this = self.clone();
                let id = v["id"].as_str().unwrap_or_default().to_owned();
                tauri::async_runtime::spawn(async move {
                    let reply = match this.rpc(&from, v["op"].as_str().unwrap_or(""), &v["args"]).await {
                        Ok(data) => json!({ "ok": true, "data": data }),
                        Err(e) => json!({ "ok": false, "err": e.to_string() }),
                    };
                    let Some(link) = this.inner.lock().unwrap_or_else(|e| e.into_inner()).link.clone() else { return };
                    this.send_link(json!({ "t": "rpcr", "dev": link.dev, "to": from, "id": id, "r": reply }));
                });
            }
            "pull" => {
                // the other one wants to play what plays here
                let this = self.clone();
                tauri::async_runtime::spawn(async move { this.hand_over(&from) });
            }
            _ => {}
        }
    }

    /// What the phone may ask for. Every answer is small JSON; a track is {id, title, artist, dur, art}.
    async fn rpc(&self, from: &str, op: &str, args: &Value) -> AppResult<Value> {
        use crate::commands as c;
        let app = &self.app;
        let text = |k: &str| args[k].as_str().unwrap_or_default().to_owned();
        let tracks = |list: Vec<TrackDto>| Value::Array(list.iter().map(track_json).collect::<Vec<_>>());
        let sp = || app.state::<crate::sync::shared::SharedPlaylists>();
        let friends = || app.state::<crate::sync::friends::Friends>();
        let rooms = || app.state::<crate::sync::service::Rooms>();
        Ok(match op {
            // «Импорт» of a playlist (Yandex Music, Spotify …) started on the phone: the computer does it into the account
            "import_preview" => {
                let (name, n) = c::import_preview(app.state(), text("input")).await?;
                json!({ "name": name, "n": n })
            }
            "import_run" => {
                let r = c::import_run(app.clone(), app.state(), text("input"), text("title"), args["private"].as_bool().unwrap_or(false)).await?;
                json!({ "title": r.title, "found": r.found, "total": r.total, "playlists": r.playlists, "missing": r.missing })
            }
            // the user's own playlists, and writing into them (the computer holds the session that writes to SoundCloud)
            "my_playlists" => {
                let list = c::library_playlists(app.state(), false).await?;
                Value::Array(list.iter().filter(|p| p.own && !p.is_album).map(|p| json!({ "id": p.id, "title": p.title, "n": p.track_count })).collect())
            }
            "pl_add" => {
                c::playlist_add_track(app.clone(), app.state(), args["playlist"].as_u64().unwrap_or(0), args["track"].as_u64().unwrap_or(0)).await?;
                Value::Null
            }
            "pl_create" => {
                c::playlist_create(app.clone(), app.state(), text("title"), args["track"].as_u64()).await?;
                Value::Null
            }
            "pl_delete" => {
                c::playlist_delete(app.clone(), app.state(), args["playlist"].as_u64().unwrap_or(0)).await?;
                Value::Null
            }
            // a track put into the queue of this computer, now or at the end
            "enqueue" => {
                let state = app.state::<AppState>();
                let found = state.db.tracks_by_ids(vec![args["track"].as_u64().unwrap_or(0)]).await?;
                let Some(t) = found.first() else { return Err(AppError::Other("track not known here".into())) };
                if let Some(p) = app.try_state::<Arc<Player>>() {
                    p.enqueue(TrackDto::from(t), args["next"].as_bool().unwrap_or(false));
                }
                Value::Null
            }
            // the likes that SoundCloud has not taken yet, and sending them again
            "likes_pending" => {
                let list = c::likes_pending(app.state()).await?;
                json!({
                    "check": c::captcha_waiting(),
                    "items": list.iter().map(|l| json!({ "t": track_json(&l.track), "liked": l.liked })).collect::<Vec<_>>(),
                })
            }
            "likes_flush" => json!(c::likes_pending_flush(app.clone(), app.state()).await?),
            // lines of a song in the language of the phone
            "translate" => {
                let lines: Vec<String> = args["lines"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
                json!(c::lyrics_translate(app.state(), args["track"].as_u64().unwrap_or(0), lines, text("to")).await?)
            }
            // the queue of this computer (what plays and what follows) for a phone that controls it
            "queue" => {
                let view = app.try_state::<Arc<Player>>().map(|p| p.queue_view(100));
                match view {
                    Some(v) => json!({
                        "current": v.current,
                        "items": v.items.iter().map(|i| json!({ "at": i.at, "t": track_json(&i.track) })).collect::<Vec<_>>(),
                    }),
                    None => json!({ "current": null, "items": [] }),
                }
            }
            // words from a song: the liked tracks that have them, then SoundCloud's tracks found by the lyrics base
            "lsearch" => {
                let q = text("q");
                let mine = c::likes_search_lyrics(app.state(), q.clone()).await.unwrap_or_default();
                let mut out: Vec<Value> = mine
                    .iter()
                    .map(|h| {
                        let mut t = track_json(&h.track);
                        t["line"] = json!(h.line);
                        t
                    })
                    .collect();
                if let Ok(global) = c::lyrics_search_global(app.state(), q).await {
                    for t in global.iter().filter(|t| !mine.iter().any(|h| h.track.id == t.id)) {
                        out.push(track_json(t));
                    }
                }
                Value::Array(out)
            }
            "friends" => {
                let view = friends().view();
                let listening = friends().listening(app).await.unwrap_or_default();
                json!({
                    "friends": view.friends.iter().map(|f| json!({ "id": f.id, "name": f.name, "avatar": f.avatar_url })).collect::<Vec<_>>(),
                    "incoming": view.incoming.iter().map(|f| json!({ "id": f.id, "name": f.name, "avatar": f.avatar_url })).collect::<Vec<_>>(),
                    "listening": listening.iter().map(|l| json!({ "id": l.id, "track": { "id": l.track_id, "title": l.title, "artist": l.artist, "dur": 0, "art": l.artwork_url } })).collect::<Vec<_>>(),
                })
            }
            "liked" => json!(c::likes_ids(app.state()).await?),
            "like" => {
                let id = args["track"].as_u64().unwrap_or(0);
                let state = app.state::<AppState>();
                let found = state.db.tracks_by_ids(vec![id]).await?;
                let Some(t) = found.first() else { return Err(AppError::Other("track not known here".into())) };
                c::like_set(app.clone(), app.state(), TrackDto::from(t), args["on"].as_bool().unwrap_or(true)).await?;
                Value::Null
            }
            // the room this computer is in, as the phone shows it
            "room_view" => {
                let v = rooms().view();
                json!({
                    "active": v.active, "channel": v.channel, "name": v.name, "host": v.host, "connected": v.connected, "ended": v.ended,
                    "members": v.members.iter().map(|m| json!({ "id": m.id, "name": m.name, "avatar": m.avatar_url, "you": m.you })).collect::<Vec<_>>(),
                    "current": v.current.as_ref().map(|c| json!({ "id": c.track_id, "title": c.title, "artist": c.artist, "by": c.added_by })),
                    "next": v.queue.first().map(|c| json!({ "title": c.title, "by": c.added_by })),
                    "queue": v.queue.len(),
                    "vote_skip": v.vote_skip, "skip_votes": v.skip_votes, "skip_need": v.skip_need,
                })
            }
            "rooms_live" => {
                let list = c::rooms_live(app.state(), friends().ids()).await?;
                json!(list.iter().map(|l| json!({ "channel": l.channel, "name": l.name, "locked": l.has_password, "members": l.members, "host": l.host_name })).collect::<Vec<_>>())
            }
            "room_create" => {
                let room: c::NewRoom = serde_json::from_value(json!({
                    "name": text("name"), "password": text("password"), "access": text("access"), "queue_everyone": args["queue_everyone"].as_bool().unwrap_or(false),
                }))
                .map_err(|e| AppError::Other(e.to_string()))?;
                let made = c::room_create(app.clone(), app.state(), rooms(), room).await?;
                json!(made.link)
            }
            "room_join" => {
                c::room_join(app.clone(), app.state(), rooms(), text("link"), text("password")).await?;
                Value::Null
            }
            "room_vote" => {
                c::room_cmd(app.state(), rooms(), c::RoomCmd::VoteSkip).await?;
                Value::Null
            }
            "room_next" => {
                c::room_cmd(app.state(), rooms(), c::RoomCmd::Next).await?;
                Value::Null
            }
            "room_enqueue" => {
                let state = app.state::<AppState>();
                let found = state.db.tracks_by_ids(vec![args["track"].as_u64().unwrap_or(0)]).await?;
                let Some(t) = found.first() else { return Err(AppError::Other("track not known here".into())) };
                c::room_cmd(app.state(), rooms(), c::RoomCmd::Enqueue { track: TrackDto::from(t) }).await?;
                Value::Null
            }
            // the phone listens to the room instead of this computer: this one is silent meanwhile (and comes back by itself
            // when the phone is gone, see unmute_stale)
            "mute" => {
                let Some(p) = self.player() else { return Ok(Value::Null) };
                let on = args["on"].as_bool().unwrap_or(false);
                let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                if on && g.muted.is_none() {
                    g.muted = Some((from.to_owned(), p.snapshot().volume));
                    p.set_volume(0.0);
                } else if !on {
                    if let Some((_, v)) = g.muted.take() {
                        p.set_volume(v);
                    }
                }
                Value::Null
            }
            "lyrics" => {
                let state = app.state::<AppState>();
                let found = state.db.tracks_by_ids(vec![args["track"].as_u64().unwrap_or(0)]).await?;
                let Some(t) = found.first() else { return Err(AppError::Other("track not known here".into())) };
                let d = c::lyrics_get(app.state(), TrackDto::from(t), args["force"].as_bool().unwrap_or(false)).await?;
                json!({
                    "found": d.found, "synced": d.synced, "word": d.word_level,
                    "lines": d.lines.iter().map(|l| json!({
                        "s": l.start_ms, "t": l.text,
                        "w": l.words.iter().map(|w| json!({ "s": w.start_ms, "e": w.end_ms, "t": w.text })).collect::<Vec<_>>(),
                    })).collect::<Vec<_>>(),
                })
            }
            "picks" => tracks(c::picks(app.state(), text("kind")).await?),
            "chart" => {
                let p = c::chart_page(app.state(), text("kind")).await?;
                tracks(p.tracks)
            }
            "friend_accept" => {
                friends().accept(app, args["id"].as_u64().unwrap_or(0)).await?;
                Value::Null
            }
            "friend_decline" => {
                friends().decline(app, args["id"].as_u64().unwrap_or(0));
                Value::Null
            }
            "sp_list" => {
                let list = c::sp_list(app.clone(), sp()).await?;
                Value::Array(list.iter().map(|i| json!({ "channel": i.channel, "title": i.title, "count": i.count, "can_edit": i.can_edit, "mine": i.mine, "art": i.artworks.first() })).collect())
            }
            "sp_get" => {
                let d = c::sp_get(app.clone(), sp(), text("channel")).await?;
                json!({ "title": d.info.title, "can_edit": d.info.can_edit, "tracks": d.tracks.iter().filter_map(|t| t.track.as_ref().map(|x| { let mut j = track_json(x); j["by"] = json!(t.added_by_name); j })).collect::<Vec<_>>() })
            }
            "sp_add" => {
                c::sp_add(sp(), text("channel"), args["track"].as_u64().unwrap_or(0))?;
                Value::Null
            }
            "sp_join" => json!(c::sp_join(app.clone(), sp(), text("link")).await?),
            "rooms" => {
                let state = app.state::<AppState>();
                let mine = rooms().mine(&state).await;
                let now = rooms().view();
                json!({
                    "now": now.channel,
                    "mine": mine.iter().map(|m| json!({ "channel": m.channel, "name": m.name, "online": m.online, "active": m.active, "current": m.current, "locked": m.has_password, "queue": m.queue_len })).collect::<Vec<_>>(),
                })
            }
            "room_enter" => {
                let state = app.state::<AppState>();
                rooms().enter(app, &state, &text("channel")).await?;
                Value::Null
            }
            "room_leave" => {
                let state = app.state::<AppState>();
                rooms().leave(&state).await;
                Value::Null
            }
            _ => return Err(AppError::Other("unknown request".into())),
        })
    }

    /// The phone that silenced this computer is not heard for a while: the sound comes back.
    fn unmute_stale(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some((phone, vol)) = g.muted.clone() else { return };
        let seen = g.peers.get(&phone).map(|p| p.seen).unwrap_or(0);
        if now_ms() / 1000 - seen > 90 {
            g.muted = None;
            drop(g);
            if let Some(p) = self.player() {
                p.set_volume(vol);
            }
        }
    }

    /// The equalizer another device set: applied here and remembered; the window shows it.
    fn take_eq(&self, eq: &Value) {
        let Ok(cfg) = serde_json::from_value::<crate::config::EqConfig>(eq.clone()) else { return };
        if cfg.gains.iter().any(|g| !g.is_finite()) {
            return;
        }
        let Some(p) = self.player() else { return };
        p.set_eq(cfg.clone());
        let state = self.app.state::<AppState>();
        let device = p.current_device();
        let _ = state.config.update(|c| {
            c.eq = cfg.clone();
            if c.eq_per_device {
                if let Some(d) = device.clone() {
                    c.eq_by_device.insert(d, cfg.clone());
                }
            }
        });
        let _ = self.app.emit("eq:changed", (device.unwrap_or_default(), cfg));
    }

    /// The user answered "give the phone the servers of the subscription?".
    pub fn vpn_answer(&self, allow: bool) {
        let Some((phone, _)) = self.inner.lock().unwrap_or_else(|e| e.into_inner()).vpn_ask.take() else { return };
        let Some(link) = self.inner.lock().unwrap_or_else(|e| e.into_inner()).link.clone() else { return };
        if !allow {
            self.send_link_kept(json!({ "t": "vpn", "dev": link.dev, "to": phone, "servers": [], "err": "denied" }), "vpn");
            return;
        }
        let saved = crate::net::vpn::load();
        let servers: Vec<Value> = saved
            .servers
            .iter()
            .filter(|s| s.ok)
            .filter_map(|s| crate::net::vpn::outbound(&s.link).map(|o| json!({ "n": s.name, "o": o })))
            .take(150)
            .collect();
        let err = if servers.is_empty() { Some("none") } else { None };
        tracing::info!(servers = servers.len(), "devices: servers of the subscription go to the phone");
        self.send_link_kept(json!({ "t": "vpn", "dev": link.dev, "to": phone, "servers": servers, "err": err }), "vpn");
    }

    /// This computer set its equalizer: the others are told.
    pub fn tell_eq(&self, cfg: &crate::config::EqConfig) {
        let Some(link) = self.inner.lock().unwrap_or_else(|e| e.into_inner()).link.clone() else { return };
        self.send_link(json!({ "t": "eq", "dev": link.dev, "eq": cfg }));
    }

    fn player(&self) -> Option<Arc<Player>> {
        self.app.try_state::<Arc<Player>>().map(|p| p.inner().clone())
    }

    fn run_command(&self, v: &Value) {
        let Some(p) = self.player() else { return };
        match v["op"].as_str().unwrap_or("") {
            "play" => p.resume(),
            "pause" => p.pause(),
            "toggle" => p.toggle(),
            "next" => p.next(),
            "prev" => p.prev(),
            "seek" => p.seek(v["v"].as_u64().unwrap_or(0)),
            "vol" => p.set_volume(v["v"].as_f64().unwrap_or(1.0).clamp(0.0, 1.0) as f32),
            // a track of the queue picked on the phone: play it now / take it out
            "qplay" => p.queue_play(v["v"].as_u64().unwrap_or(0) as usize),
            "qremove" => p.queue_remove(v["v"].as_u64().unwrap_or(0) as usize),
            _ => {}
        }
    }

    /// Tells the others what plays here (when it changed, or every few seconds while it plays).
    fn on_player_state(&self, snap: &Value) {
        if self.inner.lock().unwrap_or_else(|e| e.into_inner()).link.is_none() {
            return;
        }
        let id = snap["track"]["id"].as_u64();
        let playing = snap["playing"].as_bool().unwrap_or(false);
        let vol = (snap["volume"].as_f64().unwrap_or(1.0) * 100.0).round() as u32;
        let now = now_ms();
        let due = {
            let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let (t_id, t_play, t_vol, t_at) = g.told;
            let due = t_id != id || t_play != playing || t_vol != vol || (playing && now - t_at >= STATE_EVERY_MS);
            if due {
                g.told = (id, playing, vol, now);
            }
            due
        };
        if due {
            self.tell_state(false);
        }
    }

    fn tell_state(&self, force: bool) {
        let Some(p) = self.player() else { return };
        let Some(link) = self.inner.lock().unwrap_or_else(|e| e.into_inner()).link.clone() else { return };
        let s = p.snapshot();
        // nobody to tell and nothing playing: stay quiet
        if !force && s.track.is_none() {
            return;
        }
        self.send_link(json!({
            "t": "st", "dev": link.dev, "ts": now_ms(),
            "track": s.track.as_ref().map(track_json), "pos": s.position_ms, "playing": s.playing, "vol": s.volume,
        }));
    }

    /// Gives the music to `to`: the queue, the place and the position go over, this player stops.
    fn hand_over(&self, to: &str) {
        let (Some(p), Some(link)) = (self.player(), self.inner.lock().unwrap_or_else(|e| e.into_inner()).link.clone()) else { return };
        let s = p.snapshot();
        let Some(cur) = s.track.clone() else { return };
        let mut ids = vec![cur.id];
        ids.extend(p.upcoming(XFER_MAX - 1).iter().map(|t| t.id));
        self.send_link(json!({
            "t": "xfer", "dev": link.dev, "to": to, "ids": ids, "idx": 0, "pos": p.position_ms(),
            "playing": s.playing, "vol": s.volume,
        }));
        p.pause();
    }

    /// The music comes here: fetch the tracks, play them from the same place.
    async fn take_over(&self, v: &Value) -> AppResult<()> {
        let ids: Vec<u64> = v["ids"].as_array().map(|a| a.iter().filter_map(Value::as_u64).take(XFER_MAX).collect()).unwrap_or_default();
        if ids.is_empty() {
            return Ok(());
        }
        let Some(p) = self.player() else { return Ok(()) };
        let state = self.app.state::<AppState>();
        let mut have: HashMap<u64, TrackDto> =
            state.db.tracks_by_ids(ids.clone()).await?.iter().map(|t| (t.id, TrackDto::from(t))).collect();
        let missing: Vec<u64> = ids.iter().filter(|i| !have.contains_key(i)).copied().collect();
        if !missing.is_empty() {
            let fetched = state.sc()?.tracks_by_ids(&missing).await?;
            state.db.upsert_tracks(fetched.clone()).await?;
            have.extend(fetched.iter().map(|t| (t.id, TrackDto::from(t))));
        }
        let tracks: Vec<TrackDto> = ids.iter().filter_map(|i| have.get(i).cloned()).collect();
        if tracks.is_empty() {
            return Err(AppError::NotFound);
        }
        let first = tracks[0].id;
        let (pos, playing) = (v["pos"].as_u64().unwrap_or(0), v["playing"].as_bool().unwrap_or(true));
        if let Some(vol) = v["vol"].as_f64() {
            p.set_volume(vol.clamp(0.0, 1.0) as f32);
        }
        p.play_queue(tracks, 0, QueueSource::List);
        // the track is fetched first: then it is put at the position, and paused if it was paused over there
        for _ in 0..75 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let s = p.snapshot();
            if s.track.as_ref().is_some_and(|t| t.id == first) && !s.loading {
                if pos > 2_000 {
                    p.seek(pos);
                }
                if !playing {
                    p.pause();
                }
                break;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------- pairing

    /// Opens the pairing channel; returns what the QR code holds.
    pub fn pair_start(self: &Arc<Self>) -> AppResult<String> {
        // with an account here the phone gets it from this computer; without one the phone gives its own
        let take = self.app.state::<AppState>().credentials().is_none();
        self.pair_stop();
        let chan = format!("pr-{}", crypto::new_channel_id());
        let key = crypto::random_key();
        let base = crate::config::sync_base(&self.app.state::<AppState>().config.get().sync_url);
        let (client, mut rx) = hub::connect(base, chan.clone(), 0);
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).pairing = Some(Pairing { key, chan: chan.clone(), client, asking: None, take, offer: None });
        let this = self.clone();
        let url = pair_url(&chan, &key, take);
        let watched = chan.clone();
        tauri::async_runtime::spawn(async move {
            // five minutes, then the code is dead
            let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
            loop {
                let ev = match tokio::time::timeout_at(deadline, rx.recv()).await {
                    Ok(Some(e)) => e,
                    _ => break,
                };
                if let HubEvent::Msg { blob, .. } = ev {
                    let Some(plain) = crypto::open(&key, &watched, &blob) else { continue };
                    let Ok(v) = serde_json::from_slice::<Value>(&plain) else { continue };
                    // a phone gives its account to this computer: the user here has to agree
                    if v["t"] == "account" && this.inner.lock().unwrap_or_else(|e| e.into_inner()).pairing.as_ref().is_some_and(|p| p.take) {
                        let dev = v["dev"].as_str().unwrap_or("").to_owned();
                        let name = v["name"].as_str().unwrap_or("Телефон").to_owned();
                        let user = v["me"]["username"].as_str().unwrap_or("").to_owned();
                        if let Some(p) = this.inner.lock().unwrap_or_else(|e| e.into_inner()).pairing.as_mut() {
                            p.offer = Some((dev.clone(), name.clone(), v.clone()));
                        }
                        let _ = this.app.emit("devices:account-offer", json!({ "dev": dev, "name": name, "user": user }));
                    }
                    if v["t"] == "hello" {
                        let (dev, name) = (v["dev"].as_str().unwrap_or("").to_owned(), v["name"].as_str().unwrap_or("Телефон").to_owned());
                        if let Some(p) = this.inner.lock().unwrap_or_else(|e| e.into_inner()).pairing.as_mut() {
                            p.asking = Some((dev.clone(), name.clone()));
                        }
                        let _ = this.app.emit("devices:pair-request", json!({ "dev": dev, "name": name }));
                    }
                }
            }
            this.pair_stop_if(&watched);
        });
        Ok(url)
    }

    fn pair_stop_if(&self, chan: &str) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.pairing.as_ref().is_some_and(|p| p.chan == chan) {
            if let Some(p) = g.pairing.take() {
                p.client.close();
            }
        }
    }

    pub fn pair_stop(&self) {
        if let Some(p) = self.inner.lock().unwrap_or_else(|e| e.into_inner()).pairing.take() {
            p.client.close();
        }
    }

    /// The user answered "allow this phone?": on yes the phone gets the login and the key of the link.
    pub async fn pair_answer(self: &Arc<Self>, allow: bool) -> AppResult<()> {
        let (key, chan, asking) = {
            let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let Some(p) = g.pairing.as_ref() else { return Ok(()) };
            (p.key, p.chan.clone(), p.asking.clone())
        };
        let Some((phone_dev, phone_name)) = asking else { return Ok(()) };
        if !allow {
            self.pair_stop();
            return Ok(());
        }
        let link = self.ensure_link();
        let state = self.app.state::<AppState>();
        state.db.kv_put(LINK_KEY, &link).await?;
        let creds = state.credentials().ok_or(AppError::NotAuthorized)?;
        let me = state.sc()?.me().await?;
        let msg = json!({
            "t": "link",
            "creds": { "client_id": creds.client_id, "oauth_token": creds.oauth_token, "stream_token": creds.stream_token },
            "me": { "id": me.id, "username": me.username },
            "link": { "chan": link.chan, "key": link.key },
            "pc": { "dev": link.dev, "name": link.name },
            "phone": phone_dev,
        });
        if let Some(p) = self.inner.lock().unwrap_or_else(|e| e.into_inner()).pairing.as_ref() {
            p.client.send(crypto::seal(&key, &chan, msg.to_string().as_bytes()), false, None);
        }
        tracing::info!(phone = %phone_name, "devices: a phone was linked");
        self.connect_link();
        // the code has done its work: closed a little later, when the message has gone out
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(4)).await;
            this.pair_stop_if(&chan);
        });
        self.changed();
        Ok(())
    }

    /// The link: made at the first phone, the same for the next ones.
    fn ensure_link(&self) -> Link {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.link
            .get_or_insert_with(|| {
                let mut id = [0u8; 9];
                rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut id);
                Link { chan: format!("dl-{}", crypto::new_channel_id()), key: crypto::key_to_b64(&crypto::random_key()), dev: format!("pc-{}", hex::encode(id)), name: computer_name() }
            })
            .clone()
    }

    /// The user answered "sign in here as the phone's account?": on yes the account is taken and the link goes back.
    pub async fn account_answer(self: &Arc<Self>, allow: bool) -> AppResult<()> {
        let (key, chan, offer) = {
            let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let Some(p) = g.pairing.as_ref() else { return Ok(()) };
            (p.key, p.chan.clone(), p.offer.clone())
        };
        let Some((phone_dev, phone_name, msg)) = offer else { return Ok(()) };
        if !allow {
            self.pair_stop();
            return Ok(());
        }
        let state = self.app.state::<AppState>();
        let http = state.http();
        let old = state.credentials();
        let creds = crate::secrets::Credentials {
            client_id: msg["creds"]["client_id"].as_str().ok_or(AppError::NotAuthorized)?.to_owned(),
            oauth_token: msg["creds"]["oauth_token"].as_str().ok_or(AppError::NotAuthorized)?.to_owned(),
            genius_token: old.as_ref().and_then(|c| c.genius_token.clone()),
            stream_token: msg["creds"]["stream_token"].as_str().map(str::to_owned).or_else(|| old.and_then(|c| c.stream_token)),
        };
        // the account is checked with SoundCloud before it is kept
        let me = crate::api::soundcloud::SoundCloud::new(http, &creds).me().await?;
        let to_save = creds.clone();
        tauri::async_runtime::spawn_blocking(move || crate::secrets::save(&to_save)).await.map_err(|e| AppError::Keyring(e.to_string()))??;
        state.set_credentials(Some(creds));
        tracing::info!(user = %me.username, phone = %phone_name, "signed in with the account of a phone");
        state.set_me(me);
        {
            let app = self.app.clone();
            tauri::async_runtime::spawn(async move { crate::sync::start_account_services(&app).await });
        }
        let _ = self.app.emit("auth:changed", crate::commands::auth_status_of(&state));

        let link = self.ensure_link();
        state.db.kv_put(LINK_KEY, &link).await?;
        let back = json!({ "t": "link", "link": { "chan": link.chan, "key": link.key }, "pc": { "dev": link.dev, "name": link.name }, "phone": phone_dev });
        if let Some(p) = self.inner.lock().unwrap_or_else(|e| e.into_inner()).pairing.as_ref() {
            p.client.send(crypto::seal(&key, &chan, back.to_string().as_bytes()), false, None);
        }
        self.connect_link();
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(4)).await;
            this.pair_stop_if(&chan);
        });
        self.changed();
        Ok(())
    }

    // ---------------------------------------------------------------- views

    pub fn view(&self) -> DevicesView {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_ms() / 1000;
        let mut peers: Vec<DeviceDto> = g
            .peers
            .iter()
            .map(|(dev, p)| DeviceDto { dev: dev.clone(), name: p.name.clone(), kind: p.kind.clone(), online: now - p.seen < AWAY_SECS, remote: p.remote.clone() })
            .collect();
        peers.sort_by(|a, b| b.online.cmp(&a.online).then(a.name.cmp(&b.name)));
        DevicesView { linked: g.link.is_some(), name: g.link.as_ref().map_or_else(computer_name, |l| l.name.clone()), peers }
    }

    pub async fn unlink(&self) -> AppResult<()> {
        let old = {
            let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            g.peers.clear();
            g.link = None;
            g.client.take()
        };
        if let Some(c) = old {
            c.close();
        }
        self.app.state::<AppState>().db.kv_delete(LINK_KEY).await?;
        self.changed();
        Ok(())
    }

    pub fn command(&self, to: &str, op: &str, value: Option<Value>) {
        let Some(link) = self.inner.lock().unwrap_or_else(|e| e.into_inner()).link.clone() else { return };
        self.send_link(json!({ "t": "cmd", "dev": link.dev, "to": to, "op": op, "v": value }));
    }

    /// `pull`: ask `to` to hand its music over to this computer; otherwise send ours there.
    pub fn transfer(&self, to: &str, pull: bool) {
        if pull {
            let Some(link) = self.inner.lock().unwrap_or_else(|e| e.into_inner()).link.clone() else { return };
            self.send_link(json!({ "t": "pull", "dev": link.dev, "to": to }));
        } else {
            self.hand_over(to);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pairing_url_holds_the_channel_and_a_key_that_reads_back() {
        let key = crypto::random_key();
        let chan = format!("pr-{}", crypto::new_channel_id());
        let url = pair_url(&chan, &key, false);
        assert!(url.starts_with("scdesk://pair?c=pr-") && !url.contains("m=login"));
        assert!(pair_url(&chan, &key, true).ends_with("&m=login"));
        // what the relay accepts as a channel id
        assert!(chan.len() >= 16 && chan.len() <= 64 && chan.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        let k = url.split("&k=").nth(1).unwrap();
        use base64::Engine;
        assert_eq!(base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(k).unwrap(), key.to_vec());
    }

    #[test]
    fn a_track_goes_over_as_a_small_json() {
        let t = TrackDto { id: 7, title: "T".into(), artist: "A".into(), user_id: 1, duration_ms: 1000, artwork_url: None, permalink_url: None, genre: None };
        let v = track_json(&t);
        assert_eq!(v["id"], 7);
        assert_eq!(v["dur"], 1000);
    }

    #[test]
    fn the_link_channel_is_a_valid_id_too() {
        let c = format!("dl-{}", crypto::new_channel_id());
        assert!(c.len() >= 16 && c.len() <= 64);
    }
}
