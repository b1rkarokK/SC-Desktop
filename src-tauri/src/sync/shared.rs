//! Shared playlists in the app: the list of the ones we are in (saved in the database), a relay
//! connection for each, and the data the screens ask for.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::{
    crypto,
    hub::{self, HubClient},
    playlists::{self, empty_stored, Pl, Stored, Store},
};
use crate::{
    error::{AppError, AppResult},
    models::TrackDto,
    shared_playlist::{Access, Role},
    state::AppState,
};

const INDEX: &str = "sp:index";
/// connections kept open at once (each one is an idle WebSocket)
const MAX_OPEN: usize = 30;

struct Entry {
    pl: Arc<Pl>,
    hub: HubClient,
}

#[derive(Default)]
pub struct SharedPlaylists {
    map: Mutex<HashMap<String, Entry>>,
}

struct DbStore {
    app: AppHandle,
}

impl Store for DbStore {
    fn save(&self, channel: &str, s: &Stored) {
        let (app, key, s) = (self.app.clone(), format!("sp:{channel}"), s.clone());
        tauri::async_runtime::spawn(async move {
            let _ = app.state::<AppState>().db.kv_put(&key, &s).await;
        });
    }
}

#[derive(Serialize, Clone)]
pub struct SpInfo {
    pub channel: String,
    pub title: String,
    pub role: Option<Role>,
    pub access: Option<Access>,
    pub count: usize,
    /// tracks added since this playlist was last opened here
    pub unseen: usize,
    pub mine: bool,
    pub can_edit: bool,
    pub members: Vec<playlists::Member>,
}

#[derive(Serialize)]
pub struct SpTrack {
    pub entry: String,
    pub track_id: u64,
    pub track: Option<TrackDto>,
    pub added_by: u64,
    pub added_by_name: String,
}

#[derive(Serialize)]
pub struct SpEvent {
    pub removed: bool,
    pub who: String,
    pub title: String,
}

#[derive(Serialize)]
pub struct SpDetail {
    pub info: SpInfo,
    pub tracks: Vec<SpTrack>,
    pub history: Vec<SpEvent>,
    pub link: String,
}

fn info(channel: &str, pl: &Pl) -> SpInfo {
    let st = pl.stored();
    SpInfo {
        channel: channel.to_owned(),
        title: pl.title(),
        role: pl.role(),
        access: pl.settings().map(|s| s.access),
        count: st.playlist.len(),
        unseen: 0,
        mine: st.mine,
        can_edit: pl.can_edit(),
        members: pl.members(),
    }
}

fn server(state: &AppState) -> AppResult<String> {
    let url = crate::config::sync_base(&state.config.get().sync_url);
    if url.trim().is_empty() {
        return Err(AppError::Other(crate::lang::pick(
            "Укажите адрес сервера синхронизации в Настройках: без него общие плейлисты не работают.",
            "Set the sync server address in Settings: shared playlists do not work without it.",
        )
        .into()));
    }
    Ok(url.trim().trim_end_matches('/').to_owned())
}

impl SharedPlaylists {
    fn pl(&self, channel: &str) -> AppResult<Arc<Pl>> {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).get(channel).map(|e| e.pl.clone()).ok_or_else(|| AppError::Other("такого общего плейлиста нет".into()))
    }

    async fn index(app: &AppHandle) -> Vec<String> {
        app.state::<AppState>().db.kv_get::<Vec<String>>(INDEX).await.ok().flatten().map(|(v, _)| v).unwrap_or_default()
    }

    async fn save_index(app: &AppHandle, list: &[String]) {
        let _ = app.state::<AppState>().db.kv_put(INDEX, &list.to_vec()).await;
    }

    fn connect(&self, app: &AppHandle, channel: String, stored: Stored) -> AppResult<Arc<Pl>> {
        let state = app.state::<AppState>();
        let base = server(&state)?;
        let Some(me) = state.me() else {
            return Err(AppError::Other(crate::lang::pick("Сначала войдите в SoundCloud", "Sign in to SoundCloud first").into()));
        };
        if self.map.lock().unwrap_or_else(|e| e.into_inner()).len() >= MAX_OPEN {
            return Err(AppError::Other("слишком много общих плейлистов".into()));
        }
        let (hub, events) = hub::connect(base, channel.clone(), stored.last_seq);
        let app2 = app.clone();
        let on_change: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |c| {
            let _ = app2.emit("playlist:changed", c);
        });
        let pl = Pl::new(channel.clone(), stored, me.id, me.username, Arc::new(hub.clone()), Arc::new(DbStore { app: app.clone() }), on_change);
        pl.spawn(events);
        self.map.lock().unwrap_or_else(|e| e.into_inner()).insert(channel, Entry { pl: pl.clone(), hub });
        Ok(pl)
    }

    /// At start: reopen every playlist we were in (needs the server address and the sign-in).
    pub async fn start(&self, app: &AppHandle) {
        for channel in Self::index(app).await {
            if let Ok(Some((stored, _))) = app.state::<AppState>().db.kv_get::<Stored>(&format!("sp:{channel}")).await {
                if let Err(e) = self.connect(app, channel.clone(), stored) {
                    tracing::debug!(error = %e, %channel, "shared playlist not opened");
                    break;
                }
            }
        }
    }

    /// The server address changed: connect again.
    pub async fn reconnect(&self, app: &AppHandle) {
        let old: Vec<Entry> = self.map.lock().unwrap_or_else(|e| e.into_inner()).drain().map(|(_, e)| e).collect();
        for e in old {
            e.hub.close();
        }
        self.start(app).await;
    }

    async fn seen(app: &AppHandle) -> HashMap<String, usize> {
        app.state::<AppState>().db.kv_get::<HashMap<String, usize>>("sp:seen").await.ok().flatten().map(|(v, _)| v).unwrap_or_default()
    }

    pub async fn list(&self, app: &AppHandle) -> Vec<SpInfo> {
        let seen = Self::seen(app).await;
        let mut v: Vec<SpInfo> = self.map.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(c, e)| info(c, &e.pl)).collect();
        for i in &mut v {
            // never opened here: nothing is "new" yet
            i.unseen = i.count.saturating_sub(seen.get(&i.channel).copied().unwrap_or(i.count));
        }
        v.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        v
    }

    pub async fn create(&self, app: &AppHandle, title: String, access: Access) -> AppResult<(SpInfo, String)> {
        let channel = crypto::new_channel_id();
        let key = playlists::new_key();
        let pl = self.connect(app, channel.clone(), empty_stored(key.clone(), true))?;
        pl.found(if title.trim().is_empty() { "Общий плейлист".into() } else { title.trim().to_owned() }, access);
        let mut idx = Self::index(app).await;
        idx.push(channel.clone());
        Self::save_index(app, &idx).await;
        let i = info(&channel, &pl);
        Ok((i, playlists::make_link(&channel, &key)))
    }

    pub async fn join(&self, app: &AppHandle, link: &str) -> AppResult<String> {
        let (channel, key) = playlists::parse_link(link).ok_or_else(|| AppError::Other(crate::lang::pick("Это не ссылка на общий плейлист", "This is not a shared playlist link").into()))?;
        if self.pl(&channel).is_ok() {
            return Ok(channel);
        }
        self.connect(app, channel.clone(), empty_stored(key, false))?;
        let mut idx = Self::index(app).await;
        idx.push(channel.clone());
        Self::save_index(app, &idx).await;
        Ok(channel)
    }

    pub async fn leave(&self, app: &AppHandle, channel: &str) {
        if let Some(e) = self.map.lock().unwrap_or_else(|e| e.into_inner()).remove(channel) {
            e.hub.close();
        }
        let mut idx = Self::index(app).await;
        idx.retain(|c| c != channel);
        Self::save_index(app, &idx).await;
        let _ = app.state::<AppState>().db.kv_put(&format!("sp:{channel}"), &serde_json::Value::Null).await;
    }

    pub async fn detail(&self, app: &AppHandle, channel: &str) -> AppResult<SpDetail> {
        let pl = self.pl(channel)?;
        let rows = pl.rows();
        let state = app.state::<AppState>();
        let past = pl.history(30);
        let ids: Vec<u64> = rows.iter().map(|r| r.track_id).chain(past.iter().map(|e| e.2)).collect();
        let mut have: HashMap<u64, TrackDto> = state.db.tracks_by_ids(ids.clone()).await?.iter().map(|t| (t.id, TrackDto::from(t))).collect();
        // tracks nobody here has seen yet: asked from SoundCloud, a few at a time
        let mut missing: Vec<u64> = ids.iter().copied().filter(|id| !have.contains_key(id)).collect();
        missing.sort_unstable();
        missing.dedup();
        missing.truncate(40);
        if !missing.is_empty() {
            if let Ok(sc) = state.sc() {
                let mut got = Vec::new();
                for id in missing {
                    if let Ok(t) = sc.track(id).await {
                        have.insert(id, TrackDto::from(&t));
                        got.push(t);
                    }
                }
                let _ = state.db.upsert_tracks(got).await;
            }
        }
        let tracks = rows.into_iter().map(|r| SpTrack { track: have.get(&r.track_id).cloned(), entry: r.entry, track_id: r.track_id, added_by: r.added_by, added_by_name: r.added_by_name }).collect();
        let link = playlists::make_link(channel, &pl.stored().key);
        let i = info(channel, &pl);
        // opened: everything in it is seen now
        let mut seen = Self::seen(app).await;
        if seen.get(channel) != Some(&i.count) {
            seen.insert(channel.to_owned(), i.count);
            let _ = state.db.kv_put("sp:seen", &seen).await;
        }
        let history = past
            .into_iter()
            .map(|(removed, who, track)| SpEvent { removed, who, title: have.get(&track).map(|t| format!("{} - {}", t.title, t.artist)).unwrap_or_default() })
            .collect();
        Ok(SpDetail { info: i, tracks, history, link })
    }

    pub fn add(&self, channel: &str, track_id: u64) -> AppResult<()> {
        self.pl(channel)?.add(track_id).map_err(AppError::Other)
    }

    pub fn remove(&self, channel: &str, entry: &str) -> AppResult<()> {
        self.pl(channel)?.remove(entry).map_err(AppError::Other)
    }

    pub fn move_to(&self, channel: &str, entry: &str, index: usize) -> AppResult<()> {
        self.pl(channel)?.move_to(entry, index).map_err(AppError::Other)
    }

    pub fn set_access(&self, channel: &str, access: Access) -> AppResult<()> {
        self.pl(channel)?.set_access(access).map_err(AppError::Other)
    }

    pub fn set_title(&self, channel: &str, title: String) -> AppResult<()> {
        self.pl(channel)?.set_title(title).map_err(AppError::Other)
    }

    pub fn invite(&self, channel: &str, id: u64, name: String, role: Role) -> AppResult<()> {
        self.pl(channel)?.invite(id, name, role).map_err(AppError::Other)
    }

    pub fn remove_member(&self, channel: &str, id: u64) -> AppResult<()> {
        self.pl(channel)?.remove_member(id).map_err(AppError::Other)
    }
}
