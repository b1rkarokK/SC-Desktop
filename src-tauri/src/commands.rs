//! Tauri IPC surface. Every command returns `Result<_, AppError>`; the error
//! serialises to `{ kind, message }` and is shown in the UI — never a panic.

use std::{
    sync::{atomic::Ordering, Arc},
    time::Instant,
};

use serde::{de::DeserializeOwned, Serialize};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;
use url::Url;

use crate::{
    api::{
        genius,
        lyrics::{self, Query},
        proxy,
        soundcloud::{SoundCloud, UserSection},
    },
    config::{AppConfig, DiscordConfig, EqConfig, NetMode},
    db::cache::now,
    error::{AppError, AppResult},
    models::{
        ArtistSection, AuthStatus, DislikedArtist, LibraryCounts, LyricsDto, NetCheck, PlaylistDto, PlaylistPage,
        SearchAll, SearchPage, TrackDto, TrackPage, UserDto, WaveInfo,
    },
    net::{HttpClient, Profile},
    player::{Player, QueueSource, RepeatMode, Snapshot},
    secrets::{self, Credentials},
    state::AppState,
    wave,
};

type Cmd<T> = AppResult<T>;

/// "Not found" lyrics are re-checked after 3 days.
const LYRICS_NEGATIVE_TTL: i64 = 3 * 86_400;
/// Library lists (liked playlists, followings) are refreshed after 6 h.
const LIBRARY_TTL: i64 = 6 * 3600;

// ------------------------------------------------------------------ auth

pub fn auth_status_of(state: &AppState) -> AuthStatus {
    let me = state.me();
    AuthStatus {
        has_credentials: state.credentials().is_some(),
        username: me.as_ref().map(|m| m.username.clone()),
        user_id: me.as_ref().map(|m| m.id),
        avatar_url: me.and_then(|m| m.avatar_url),
    }
}

#[tauri::command]
pub fn auth_status(state: State<'_, AppState>) -> AuthStatus {
    auth_status_of(&state)
}

/// Checks the token against `/me`.
#[tauri::command]
pub async fn auth_verify(state: State<'_, AppState>) -> Cmd<AuthStatus> {
    state.current_user().await?;
    Ok(auth_status_of(&state))
}

#[tauri::command]
pub async fn auth_save(state: State<'_, AppState>, client_id: String, oauth_token: String) -> Cmd<AuthStatus> {
    let client_id = client_id.trim().to_owned();
    // people paste the whole header value
    let oauth_token = oauth_token.trim().trim_start_matches("OAuth ").trim().to_owned();
    if client_id.is_empty() || oauth_token.is_empty() {
        return Err(AppError::Config("client_id и oauth_token обязательны".into()));
    }
    let creds = Credentials { client_id, oauth_token, genius_token: None };
    let me = SoundCloud::new(state.http(), &creds).me().await?;
    let to_save = creds.clone();
    tauri::async_runtime::spawn_blocking(move || secrets::save(&to_save))
        .await
        .map_err(|e| AppError::Keyring(e.to_string()))??;
    state.set_credentials(Some(creds));
    tracing::info!(user = %me.username, "credentials saved");
    state.set_me(me);
    Ok(auth_status_of(&state))
}

/// Opens the soundcloud.com sign-in window; result arrives as `auth:changed`.
/// Must be async: creating a webview window from a sync command deadlocks on
/// Windows (the window stays white).
#[tauri::command]
pub async fn auth_login(app: AppHandle) -> Cmd<()> {
    crate::login::open(&app)
}

#[tauri::command]
pub async fn auth_clear(state: State<'_, AppState>) -> Cmd<()> {
    tauri::async_runtime::spawn_blocking(secrets::clear)
        .await
        .map_err(|e| AppError::Keyring(e.to_string()))??;
    state.set_credentials(None);
    Ok(())
}

// --------------------------------------------------------------- library

#[tauri::command]
pub async fn likes_sync(app: AppHandle, state: State<'_, AppState>) -> Cmd<u64> {
    if state.likes_syncing.swap(true, Ordering::AcqRel) {
        return Err(AppError::Other("Синхронизация уже идёт".into()));
    }
    let result = async {
        let me = state.current_user().await?;
        let progress_app = app.clone();
        let items = state
            .sc()?
            .likes_all(me.id, move |n| {
                let _ = progress_app.emit("likes:progress", n);
            })
            .await?;
        let ids: Vec<(u64, String)> = items.iter().map(|(t, at)| (t.id, at.clone())).collect();
        state.db.upsert_tracks(items.into_iter().map(|(t, _)| t).collect()).await?;
        state.db.replace_likes(ids).await?;
        state.db.likes_count().await
    }
    .await;
    state.likes_syncing.store(false, Ordering::Release);
    let n = result?;
    tracing::info!(count = n, "likes synced");
    Ok(n)
}

#[tauri::command]
pub async fn likes_count(state: State<'_, AppState>) -> Cmd<u64> {
    state.db.likes_count().await
}

#[tauri::command]
pub async fn likes_page(state: State<'_, AppState>, offset: u32, limit: u32) -> Cmd<Vec<TrackDto>> {
    state.db.likes_page(offset, limit.min(500)).await
}

#[tauri::command]
pub async fn likes_ids(state: State<'_, AppState>) -> Cmd<Vec<u64>> {
    Ok(state.db.likes_ids().await?.into_iter().collect())
}

/// Like / unlike on SoundCloud, then mirror it in the local cache.
#[tauri::command]
pub async fn like_set(state: State<'_, AppState>, track: TrackDto, liked: bool) -> Cmd<()> {
    let me = state.current_user().await?;
    state.sc()?.set_like(me.id, track.id, liked).await?;
    if liked {
        if state.db.tracks_by_ids(vec![track.id]).await?.is_empty() {
            let full = state.sc()?.track(track.id).await?;
            state.db.upsert_tracks(vec![full]).await?;
        }
        state.db.like_add(track.id).await
    } else {
        state.db.like_remove(track.id).await
    }
}

/// Cached-with-TTL fetch for library lists; serves stale data if offline.
async fn cached_list<T, F, Fut>(state: &AppState, key: &str, force: bool, fetch: F) -> AppResult<Vec<T>>
where
    T: Serialize + DeserializeOwned + Send + 'static,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = AppResult<Vec<T>>>,
{
    let cached = state.db.kv_get::<Vec<T>>(key).await?;
    let fresh_enough = cached.as_ref().is_some_and(|(_, at)| now() - at < LIBRARY_TTL);
    if !force && fresh_enough {
        if let Some((v, _)) = cached {
            return Ok(v);
        }
    }
    match fetch().await {
        Ok(fresh) => {
            state.db.kv_put(key, &fresh).await?;
            Ok(fresh)
        }
        Err(e) => match cached {
            Some((v, _)) => {
                tracing::warn!(error = %e, key, "serving stale library cache");
                Ok(v)
            }
            None => Err(e),
        },
    }
}

/// Liked playlists AND albums; the UI splits them by `is_album`.
#[tauri::command]
pub async fn library_playlists(state: State<'_, AppState>, force: bool) -> Cmd<Vec<PlaylistDto>> {
    let me = state.current_user().await?;
    let sc = state.sc()?;
    cached_list(&state, "lib:playlists", force, || async move {
        Ok(sc.playlist_likes(me.id).await?.iter().map(PlaylistDto::from).collect())
    })
    .await
}

#[tauri::command]
pub async fn library_artists(state: State<'_, AppState>, force: bool) -> Cmd<Vec<UserDto>> {
    let me = state.current_user().await?;
    let sc = state.sc()?;
    cached_list(&state, "lib:artists", force, || async move {
        Ok(sc.followings(me.id).await?.iter().map(UserDto::from).collect())
    })
    .await
}

#[tauri::command]
pub async fn library_counts(state: State<'_, AppState>) -> Cmd<LibraryCounts> {
    let tracks = state.db.likes_count().await?;
    let pls = state.db.kv_get::<Vec<PlaylistDto>>("lib:playlists").await?.map(|(v, _)| v).unwrap_or_default();
    let artists = state.db.kv_get::<Vec<UserDto>>("lib:artists").await?.map(|(v, _)| v.len() as u64).unwrap_or(0);
    let albums = pls.iter().filter(|p| p.is_album).count() as u64;
    Ok(LibraryCounts { tracks, playlists: pls.len() as u64 - albums, albums, artists })
}

#[tauri::command]
pub async fn follow_set(state: State<'_, AppState>, user: UserDto, follow: bool) -> Cmd<()> {
    state.sc()?.set_follow(user.id, follow).await?;
    // keep the cached "Артисты" tab in sync without a refetch
    let mut list = state.db.kv_get::<Vec<UserDto>>("lib:artists").await?.map(|(v, _)| v).unwrap_or_default();
    list.retain(|u| u.id != user.id);
    if follow {
        list.insert(0, user);
    }
    state.db.kv_put("lib:artists", &list).await
}

// ---------------------------------------------------------------- search

#[tauri::command]
pub async fn search_tracks(state: State<'_, AppState>, query: String, offset: u32) -> Cmd<SearchPage> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(SearchPage { tracks: Vec::new(), next_offset: None });
    }
    let (tracks, next_offset) = state.sc()?.search_tracks(query, offset).await?;
    let dtos = tracks.iter().map(TrackDto::from).collect();
    state.db.upsert_tracks(tracks).await?;
    Ok(SearchPage { tracks: dtos, next_offset })
}

#[tauri::command]
pub async fn search_all(state: State<'_, AppState>, query: String) -> Cmd<SearchAll> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(SearchAll { users: vec![], tracks: vec![], playlists: vec![] });
    }
    let sc = state.sc()?;
    let (users, tracks, playlists) = futures::join!(sc.search_users(q, 6), sc.search_tracks(q, 0), sc.search_playlists(q, 6));
    let (tracks, _) = tracks?;
    let dtos = tracks.iter().map(TrackDto::from).collect();
    state.db.upsert_tracks(tracks).await?;
    Ok(SearchAll {
        users: users.unwrap_or_default().iter().map(UserDto::from).collect(),
        tracks: dtos,
        playlists: playlists.unwrap_or_default().iter().map(PlaylistDto::from).collect(),
    })
}

#[tauri::command]
pub async fn search_users(state: State<'_, AppState>, query: String) -> Cmd<Vec<UserDto>> {
    Ok(state.sc()?.search_users(query.trim(), 50).await?.iter().map(UserDto::from).collect())
}

#[tauri::command]
pub async fn search_playlists(state: State<'_, AppState>, query: String) -> Cmd<Vec<PlaylistDto>> {
    Ok(state.sc()?.search_playlists(query.trim(), 50).await?.iter().map(PlaylistDto::from).collect())
}

// ----------------------------------------------------------------- pages

#[tauri::command]
pub async fn track_page(state: State<'_, AppState>, id: u64) -> Cmd<TrackPage> {
    let sc = state.sc()?;
    let (track, related) = futures::join!(sc.track(id), sc.related(id));
    let track = track?;
    let related = related.unwrap_or_default();
    let page = TrackPage {
        track: TrackDto::from(&track),
        tags: wave::parse_tags(track.tag_list.as_deref().unwrap_or("")),
        year: track.release_date.as_deref().or(track.created_at.as_deref()).and_then(|d| d.get(..4)).map(str::to_owned),
        plays: track.playback_count,
        likes: track.likes_count,
        related: related.iter().map(TrackDto::from).collect(),
    };
    let mut all = related;
    all.push(track);
    state.db.upsert_tracks(all).await?;
    Ok(page)
}

#[tauri::command]
pub async fn artist_get(state: State<'_, AppState>, id: u64) -> Cmd<UserDto> {
    Ok(UserDto::from(&state.sc()?.user(id).await?))
}

#[tauri::command]
pub async fn artist_section(state: State<'_, AppState>, id: u64, section: String) -> Cmd<ArtistSection> {
    Ok(match state.sc()?.user_section(id, &section).await? {
        UserSection::Tracks(t) => {
            let dtos = t.iter().map(TrackDto::from).collect();
            state.db.upsert_tracks(t).await?;
            ArtistSection { tracks: dtos, playlists: vec![] }
        }
        UserSection::Playlists(p) => ArtistSection { tracks: vec![], playlists: p.iter().map(PlaylistDto::from).collect() },
    })
}

#[tauri::command]
pub async fn playlist_page(state: State<'_, AppState>, id: u64) -> Cmd<PlaylistPage> {
    let (pl, tracks) = state.sc()?.playlist(id).await?;
    let dtos = tracks.iter().map(TrackDto::from).collect();
    state.db.upsert_tracks(tracks).await?;
    Ok(PlaylistPage { playlist: PlaylistDto::from(&pl), tracks: dtos })
}

// ---------------------------------------------------------------- player

#[tauri::command]
pub async fn player_play_likes(state: State<'_, AppState>, player: State<'_, Arc<Player>>, index: usize) -> Cmd<()> {
    let tracks = state.db.likes_all().await?;
    player.play_queue(tracks, index, QueueSource::Likes);
    Ok(())
}

#[tauri::command]
pub fn player_play_tracks(player: State<'_, Arc<Player>>, tracks: Vec<TrackDto>, index: usize) {
    player.play_queue(tracks, index, QueueSource::List);
}

#[tauri::command]
pub fn player_enqueue(player: State<'_, Arc<Player>>, track: TrackDto, next: bool) {
    player.enqueue(track, next);
}

#[tauri::command]
pub fn player_toggle(player: State<'_, Arc<Player>>) {
    player.toggle();
}

#[tauri::command]
pub fn player_next(player: State<'_, Arc<Player>>) {
    player.next();
}

#[tauri::command]
pub fn player_prev(player: State<'_, Arc<Player>>) {
    player.prev();
}

#[tauri::command]
pub fn player_seek(player: State<'_, Arc<Player>>, position_ms: u64) {
    player.seek(position_ms);
}

/// `persist` = false while dragging the slider, true on release (one disk write).
#[tauri::command]
pub fn player_set_volume(
    state: State<'_, AppState>,
    player: State<'_, Arc<Player>>,
    volume: f32,
    persist: bool,
) -> Cmd<()> {
    player.set_volume(volume);
    if persist {
        state.config.update(|c| c.volume = volume.clamp(0.0, 1.0))?;
    }
    Ok(())
}

#[tauri::command]
pub fn player_set_shuffle(player: State<'_, Arc<Player>>, enabled: bool) {
    player.set_shuffle(enabled);
}

#[tauri::command]
pub fn player_set_repeat(player: State<'_, Arc<Player>>, mode: RepeatMode) {
    player.set_repeat(mode);
}

#[tauri::command]
pub fn player_snapshot(player: State<'_, Arc<Player>>) -> Snapshot {
    player.snapshot()
}

#[tauri::command]
pub fn player_position(player: State<'_, Arc<Player>>) -> u64 {
    player.position_ms()
}

#[tauri::command]
pub fn player_upcoming(player: State<'_, Arc<Player>>, limit: usize) -> Vec<TrackDto> {
    player.upcoming(limit.min(100))
}

// ------------------------------------------------------------------ wave

#[tauri::command]
pub async fn wave_start(player: State<'_, Arc<Player>>) -> Cmd<usize> {
    player.inner().start_wave().await
}

#[tauri::command]
pub async fn wave_start_from(player: State<'_, Arc<Player>>, track_id: Option<u64>, artist_id: Option<u64>) -> Cmd<usize> {
    player.inner().start_wave_seeded(track_id, artist_id).await
}

#[tauri::command]
pub async fn wave_set_mood(state: State<'_, AppState>, player: State<'_, Arc<Player>>, mood: String) -> Cmd<()> {
    if !["normal", "fresh", "familiar", "calm", "energetic"].contains(&mood.as_str()) {
        return Err(AppError::Config(format!("неизвестное настроение {mood}")));
    }
    state.wave.set_mood(&mood);
    if player.snapshot().source == QueueSource::Wave {
        player.inner().restart_wave().await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn wave_info(state: State<'_, AppState>, track_id: Option<u64>) -> Cmd<WaveInfo> {
    Ok(WaveInfo {
        mood: state.wave.mood(),
        reason: track_id.and_then(|id| state.wave.reason(id)),
        disliked_tracks: state.db.disliked_tracks_count().await?,
        disliked_artists: state.db.disliked_artists().await?,
    })
}

/// Persistent "не рекомендовать": excluded from the wave and recommendations forever.
#[tauri::command]
pub async fn dislike_set(
    state: State<'_, AppState>,
    player: State<'_, Arc<Player>>,
    track_id: u64,
    disliked: bool,
) -> Cmd<()> {
    if disliked {
        state.db.dislike_track(track_id).await?;
        if player.current().is_some_and(|t| t.id == track_id) {
            player.next();
        }
    } else {
        state.db.undislike_track(track_id).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn disliked_ids(state: State<'_, AppState>) -> Cmd<Vec<u64>> {
    Ok(state.db.disliked_track_ids().await?.into_iter().collect())
}

#[tauri::command]
pub async fn wave_dislike_artist(
    state: State<'_, AppState>,
    player: State<'_, Arc<Player>>,
    user_id: u64,
    name: String,
) -> Cmd<()> {
    state.db.dislike_artist(user_id, name).await?;
    player.purge_artist(user_id);
    if player.current().is_some_and(|t| t.user_id == user_id) {
        player.next();
    }
    Ok(())
}

#[tauri::command]
pub async fn wave_disliked_artists(state: State<'_, AppState>) -> Cmd<Vec<DislikedArtist>> {
    state.db.disliked_artists().await
}

#[tauri::command]
pub async fn wave_undislike_artist(state: State<'_, AppState>, user_id: u64) -> Cmd<()> {
    state.db.undislike_artist(user_id).await
}

#[tauri::command]
pub async fn wave_clear_dislikes(state: State<'_, AppState>) -> Cmd<()> {
    state.db.clear_dislikes().await
}

// ---------------------------------------------------------------- lyrics

#[tauri::command]
pub async fn lyrics_get(state: State<'_, AppState>, track: TrackDto, force: bool) -> Cmd<LyricsDto> {
    if !force {
        if let Some((cached, fetched_at)) = state.db.lyrics_get(track.id).await? {
            if cached.found || now() - fetched_at < LYRICS_NEGATIVE_TTL {
                return Ok(cached);
            }
        }
    }
    let (artist, title) = genius::query_from(&track.artist, &track.title);
    let q = Query { artist, title, duration_ms: track.duration_ms };
    let http = state.http();
    let found = lyrics::find(&http, &state.lyrics, &q).await?;
    let dto = match found {
        Some(l) => LyricsDto {
            track_id: track.id,
            found: true,
            source: Some(l.source),
            synced: l.synced,
            word_level: l.word_level,
            lines: l.lines,
            url: l.url,
            cached: false,
        },
        None => LyricsDto {
            track_id: track.id,
            found: false,
            source: None,
            synced: false,
            word_level: false,
            lines: vec![],
            url: None,
            cached: false,
        },
    };
    tracing::info!(track = track.id, source = ?dto.source, word = dto.word_level, "lyrics resolved");
    state.db.lyrics_put(dto.clone()).await?;
    Ok(dto)
}

// ---------------------------------------------------------------- system

/// Opens Genius / SoundCloud / Discord pages in the default browser. Other hosts are refused.
#[tauri::command]
pub fn open_external(app: AppHandle, url: String) -> Cmd<()> {
    let parsed = Url::parse(&url)?;
    let allowed = parsed.scheme() == "https"
        && parsed.host_str().is_some_and(|h| {
            ["genius.com", "soundcloud.com", "discord.com"].iter().any(|d| h == *d || h.ends_with(&format!(".{d}")))
        });
    if !allowed {
        return Err(AppError::Config("этот адрес нельзя открыть".into()));
    }
    app.opener().open_url(parsed.as_str(), None::<&str>).map_err(|e| AppError::Other(e.to_string()))
}

#[tauri::command]
pub fn config_get(state: State<'_, AppState>) -> AppConfig {
    state.config.get()
}

#[tauri::command]
pub fn config_set_network(
    state: State<'_, AppState>,
    net_mode: NetMode,
    proxy: Option<String>,
    chrome_version: u32,
) -> Cmd<AppConfig> {
    let proxy = proxy::validate(proxy.as_deref())?;
    if net_mode == NetMode::Proxy && proxy.is_none() {
        return Err(AppError::Config("укажите адрес прокси".into()));
    }
    if !(100..=400).contains(&chrome_version) {
        return Err(AppError::Config("версия Chrome должна быть числом 100–400".into()));
    }
    let previous = state.config.get();
    let cfg = state.config.update(|c| {
        c.net_mode = net_mode;
        c.proxy = proxy;
        c.chrome_version = chrome_version;
    })?;
    if let Err(e) = state.rebuild_http() {
        // roll back so a bad value can't leave the app without a client
        state.config.update(|c| *c = previous)?;
        return Err(e);
    }
    Ok(cfg)
}

/// Checks reachability of SoundCloud with the given (not yet saved) settings.
#[tauri::command]
pub async fn net_check(state: State<'_, AppState>, net_mode: NetMode, proxy: Option<String>) -> Cmd<NetCheck> {
    let mut cfg = state.config.get();
    cfg.net_mode = net_mode;
    cfg.proxy = proxy::validate(proxy.as_deref())?;
    let client = HttpClient::new(&cfg)?;
    let started = Instant::now();
    let result = client.get("https://api-v2.soundcloud.com/", Profile::ScApi, None).await;
    let ms = started.elapsed().as_millis() as u64;
    Ok(match result {
        Ok(r) => NetCheck { ok: true, status: Some(r.status().as_u16()), ms, message: "SoundCloud доступен".into() },
        Err(AppError::Forbidden) => NetCheck {
            ok: false,
            status: Some(403),
            ms,
            message: "403: SoundCloud не пускает этот адрес — нужен другой сервер".into(),
        },
        Err(AppError::NotFound | AppError::AuthExpired) => {
            NetCheck { ok: true, status: None, ms, message: "SoundCloud доступен".into() }
        }
        Err(AppError::Http(code)) => NetCheck {
            ok: code < 500,
            status: Some(code),
            ms,
            message: format!("ответ HTTP {code}"),
        },
        Err(e) => NetCheck { ok: false, status: None, ms, message: e.to_string() },
    })
}

#[tauri::command]
pub fn config_set_ui(state: State<'_, AppState>, ui: serde_json::Value) -> Cmd<()> {
    state.config.update(|c| c.ui = ui)?;
    Ok(())
}

#[tauri::command]
pub fn eq_set(state: State<'_, AppState>, player: State<'_, Arc<Player>>, eq: EqConfig, persist: bool) -> Cmd<()> {
    player.set_eq(eq.clone());
    if persist {
        state.config.update(|c| c.eq = eq)?;
    }
    Ok(())
}

#[tauri::command]
pub fn discord_set(state: State<'_, AppState>, player: State<'_, Arc<Player>>, discord: DiscordConfig) -> Cmd<()> {
    let discord = DiscordConfig { app_id: discord.app_id.trim().to_owned(), ..discord };
    if !discord.app_id.is_empty() && !discord.app_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::Config("Application ID — это число из discord.com/developers".into()));
    }
    state.config.update(|c| c.discord = discord.clone())?;
    player.set_discord(discord);
    Ok(())
}

#[derive(serde::Serialize)]
pub struct SystemPrefs {
    autostart: bool,
    start_minimized: bool,
}

#[tauri::command]
pub fn system_get(app: AppHandle, state: State<'_, AppState>) -> SystemPrefs {
    SystemPrefs {
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        start_minimized: state.config.get().start_minimized,
    }
}

#[tauri::command]
pub fn system_set(app: AppHandle, state: State<'_, AppState>, autostart: bool, start_minimized: bool) -> Cmd<()> {
    let al = app.autolaunch();
    let r = if autostart { al.enable() } else { al.disable() };
    r.map_err(|e| AppError::Other(format!("автозапуск: {e}")))?;
    state.config.update(|c| c.start_minimized = start_minimized)?;
    Ok(())
}
