//! Tauri IPC surface. Every command returns `Result<_, AppError>`; the error
//! serialises to `{ kind, message }` and is shown in the UI — never a panic.

use std::{
    sync::{atomic::Ordering, Arc},
    time::Instant,
};

use serde::{de::DeserializeOwned, Serialize};
use tauri::{AppHandle, Emitter, State};
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
    let result = sync_likes(&app, &state).await;
    state.likes_syncing.store(false, Ordering::Release);
    let n = result?;
    tracing::info!(count = n, "likes synced");
    Ok(n)
}

pub async fn sync_likes(app: &AppHandle, state: &AppState) -> AppResult<u64> {
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
    for (id, liked) in pending_likes(state).await? {
        if liked {
            state.db.like_add(id).await?;
        } else {
            state.db.like_remove(id).await?;
        }
    }
    state.db.likes_count().await
}

/// Background: likes made on the website show up without pressing "Синхронизировать".
/// Every 3 min the first page is compared with the cache; a full sync runs only on change.
/// Tracks referenced by likes / history without their metadata get it back.
async fn heal_orphan_tracks(state: &AppState) {
    let Ok(ids) = state.db.orphan_track_ids().await else { return };
    if ids.is_empty() || state.credentials().is_none() {
        return;
    }
    let result = async {
        let tracks = state.sc()?.tracks_by_ids(&ids).await?;
        let n = tracks.len();
        state.db.upsert_tracks(tracks).await?;
        AppResult::Ok(n)
    }
    .await;
    match result {
        Ok(n) => tracing::info!(missing = ids.len(), restored = n, "track metadata restored"),
        Err(e) => tracing::warn!(error = %e, "restoring track metadata failed"),
    }
}

pub fn start_likes_watch(app: &AppHandle) {
    use tauri::Manager;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        heal_orphan_tracks(&app.state::<AppState>()).await;
        // pauses between retries of queued likes, growing while SoundCloud refuses
        const BACKOFF: [u64; 5] = [60, 120, 300, 600, 900];
        let mut fails = 0usize;
        loop {
            let state = app.state::<AppState>();
            // while a captcha waits for the user, quiet retries would only annoy DataDome
            let queued = if state.credentials().is_some() && !crate::bridge::captcha_waiting() {
                flush_pending_likes(&app, &state, true).await.unwrap_or(1) + flush_pending_follows(&app, &state, true).await.unwrap_or(1)
            } else {
                pending_likes(&state).await.map(|v| v.len()).unwrap_or(0) + pending_follows(&state).await.map(|v| v.len()).unwrap_or(0)
            };
            // while likes are queued the server list legitimately differs from ours
            let likes_queued = !pending_likes(&state).await.unwrap_or_default().is_empty();
            if !likes_queued && state.credentials().is_some() && !state.likes_syncing.load(Ordering::Acquire) {
                let changed = async {
                    let me = state.current_user().await?;
                    let remote = state.sc()?.likes_first_ids(me.id).await?;
                    let local: Vec<u64> = state.db.likes_page(0, remote.len() as u32).await?.iter().map(|t| t.id).collect();
                    AppResult::Ok(remote != local)
                }
                .await;
                match changed {
                    Ok(true) => {
                        state.likes_syncing.store(true, Ordering::Release);
                        let r = sync_likes(&app, &state).await;
                        state.likes_syncing.store(false, Ordering::Release);
                        match r {
                            Ok(n) => {
                                tracing::info!(count = n, "likes changed on SoundCloud, synced");
                                let _ = app.emit("likes:changed", n);
                            }
                            Err(e) => tracing::debug!(error = %e, "background likes sync failed"),
                        }
                    }
                    Ok(false) => {}
                    Err(e) => tracing::debug!(error = %e, "likes check failed"),
                }
            }
            let pause = if queued > 0 {
                fails += 1;
                BACKOFF[(fails - 1).min(BACKOFF.len() - 1)]
            } else {
                fails = 0;
                180
            };
            tokio::time::sleep(std::time::Duration::from_secs(pause)).await;
        }
    });
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

/// Sends a write request through the browser bridge; "not found" on removal is fine.
async fn sc_write(app: &AppHandle, state: &AppState, req: (&'static str, Url), removing: bool) -> AppResult<()> {
    sc_write_body(app, state, req, None, removing).await
}

/// Queueable write (like, follow): a captcha is never popped up for it.
async fn sc_write_quiet(app: &AppHandle, state: &AppState, req: (&'static str, Url), removing: bool) -> AppResult<()> {
    if crate::bridge::captcha_waiting() {
        // it would only meet the same captcha: straight to the queue
        return write_result(crate::bridge::CAPTCHA, removing);
    }
    let sc = state.sc()?;
    let status = crate::bridge::send_quiet(app, req.0, &req.1, sc.auth_header()).await?;
    write_result(status, removing)
}

async fn sc_write_body(
    app: &AppHandle,
    state: &AppState,
    req: (&'static str, Url),
    body: Option<String>,
    removing: bool,
) -> AppResult<()> {
    let sc = state.sc()?;
    let status = crate::bridge::send(app, req.0, &req.1, sc.auth_header(), body.as_deref()).await?;
    write_result(status, removing)
}

fn write_result(status: u16, removing: bool) -> AppResult<()> {
    match status {
        200..=299 => Ok(()),
        404 if removing => Ok(()),
        0 => Err(AppError::Other("Нет связи с SoundCloud".into())),
        crate::bridge::BLOCKED => Err(AppError::Other("SoundCloud временно ограничил доступ".into())),
        crate::bridge::CAPTCHA => Err(AppError::Other("SoundCloud просит пройти проверку".into())),
        s => Err(AppError::from_status(reqwest::StatusCode::from_u16(s).unwrap_or(reqwest::StatusCode::BAD_GATEWAY))),
    }
}

const PENDING_LIKES: &str = "pending:likes";

async fn pending_likes(state: &AppState) -> AppResult<Vec<(u64, bool)>> {
    Ok(state.db.kv_get::<Vec<(u64, bool)>>(PENDING_LIKES).await?.map(|(v, _)| v).unwrap_or_default())
}

/// `quiet`: no captcha pops up (the like waits in the queue instead).
async fn send_like(app: &AppHandle, state: &AppState, track_id: u64, liked: bool, quiet: bool) -> AppResult<()> {
    let me = state.current_user().await?;
    let req = state.sc()?.like_request(me.id, track_id, liked)?;
    if quiet {
        sc_write_quiet(app, state, req, !liked).await
    } else {
        sc_write(app, state, req, !liked).await
    }
}

/// Like / unlike: applied locally at once, sent to SoundCloud right away or,
/// if SoundCloud refuses for now (anti-bot block, no network), queued and
/// retried in the background until it goes through. The user never waits.
#[tauri::command]
pub async fn like_set(app: AppHandle, state: State<'_, AppState>, track: TrackDto, liked: bool) -> Cmd<()> {
    if liked {
        if state.db.tracks_by_ids(vec![track.id]).await?.is_empty() {
            let full = state.sc()?.track(track.id).await?;
            state.db.upsert_tracks(vec![full]).await?;
        }
        state.db.like_add(track.id).await?;
    } else {
        state.db.like_remove(track.id).await?;
    }

    let mut pending = pending_likes(&state).await?;
    let was_pending = pending.iter().any(|(id, _)| *id == track.id);
    pending.retain(|(id, _)| *id != track.id);
    state.db.kv_put(PENDING_LIKES, &pending).await?;
    match send_like(&app, &state, track.id, liked, true).await {
        Ok(()) => {
            if was_pending {
                let _ = app.emit("likes:queue", ());
            }
            Ok(())
        }
        Err(e @ (AppError::AuthExpired | AppError::NotAuthorized)) => Err(e),
        Err(e) => {
            tracing::info!(error = %e, track = track.id, liked, "like queued");
            // undoing a like that never reached SoundCloud: nothing to send
            if !(was_pending && !liked) {
                pending.push((track.id, liked));
                state.db.kv_put(PENDING_LIKES, &pending).await?;
            }
            let _ = app.emit("likes:queue", ());
            Ok(())
        }
    }
}

#[derive(serde::Serialize)]
pub struct PendingLike {
    track: TrackDto,
    liked: bool,
}

/// Likes / unlikes not yet accepted by SoundCloud (shown in Профиль → В очереди).
#[tauri::command]
pub async fn likes_pending(state: State<'_, AppState>) -> Cmd<Vec<PendingLike>> {
    let pending = pending_likes(&state).await?;
    let tracks = state.db.tracks_by_ids(pending.iter().map(|(id, _)| *id).collect()).await?;
    Ok(pending
        .into_iter()
        .filter_map(|(id, liked)| tracks.iter().find(|t| t.id == id).map(|t| PendingLike { track: TrackDto::from(t), liked }))
        .collect())
}

/// SoundCloud wants a captcha before queued likes can go.
#[tauri::command]
pub fn captcha_waiting() -> bool {
    crate::bridge::captcha_waiting()
}

/// "Отправить сейчас": returns how many are still waiting.
#[tauri::command]
pub async fn likes_pending_flush(app: AppHandle, state: State<'_, AppState>) -> Cmd<usize> {
    // the user asked: a captcha may be shown now
    let left = flush_pending_likes(&app, &state, false).await? + flush_pending_follows(&app, &state, false).await?;
    let _ = app.emit("likes:queue", ());
    Ok(left)
}

// --------------------------------------------------------------- downloads

#[tauri::command]
pub async fn downloads_list(state: State<'_, AppState>) -> Cmd<Vec<crate::downloads::Download>> {
    crate::downloads::existing(&state).await
}

#[tauri::command]
pub async fn download_track(app: AppHandle, state: State<'_, AppState>, track: TrackDto) -> Cmd<crate::downloads::Download> {
    let d = crate::downloads::download(&app, &state, track).await?;
    let _ = app.emit("downloads:changed", ());
    Ok(d)
}

#[tauri::command]
pub async fn download_remove(app: AppHandle, state: State<'_, AppState>, track_id: u64) -> Cmd<()> {
    crate::downloads::remove(&state, track_id).await?;
    let _ = app.emit("downloads:changed", ());
    Ok(())
}

/// Opens Music\SC Desk in Explorer (or shows the given file there).
#[tauri::command]
pub async fn downloads_open(app: AppHandle, path: Option<String>) -> Cmd<()> {
    let target = match path {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let dir = crate::downloads::folder(&app).await?;
            let _ = tokio::fs::create_dir_all(&dir).await;
            return app.opener().open_path(dir.to_string_lossy(), None::<&str>).map_err(|e| AppError::Other(e.to_string()));
        }
    };
    app.opener().reveal_item_in_dir(target).map_err(|e| AppError::Other(e.to_string()))
}

#[tauri::command]
pub async fn downloads_dir(app: AppHandle) -> Cmd<String> {
    Ok(crate::downloads::folder(&app).await?.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn downloads_pick_dir(app: AppHandle) -> Cmd<String> {
    Ok(crate::downloads::pick_folder(&app).await?.to_string_lossy().into_owned())
}

/// Sends queued likes; returns how many are still waiting.
async fn flush_pending_likes(app: &AppHandle, state: &AppState, quiet: bool) -> AppResult<usize> {
    let pending = pending_likes(state).await?;
    if pending.is_empty() {
        return Ok(0);
    }
    let mut left = Vec::new();
    for (id, liked) in pending {
        // after one refusal the rest would be refused too: don't hammer SoundCloud
        if !left.is_empty() {
            left.push((id, liked));
        } else if let Err(e) = send_like(app, state, id, liked, quiet).await {
            tracing::debug!(error = %e, track = id, "queued like still refused");
            left.push((id, liked));
        } else {
            let _ = app.emit("likes:queue", ());
        }
    }
    state.db.kv_put(PENDING_LIKES, &left).await?;
    Ok(left.len())
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
    library_playlists_fresh(&state, force).await
}

async fn library_playlists_fresh(state: &AppState, force: bool) -> AppResult<Vec<PlaylistDto>> {
    let me = state.current_user().await?;
    let sc = state.sc()?;
    cached_list(&state, "lib:playlists2", force, || async move {
        // own playlists first (marked), then liked ones not already listed
        let (own, liked) = futures::join!(sc.own_playlists(me.id), sc.playlist_likes(me.id));
        let mut out: Vec<PlaylistDto> = own?
            .iter()
            .map(|p| PlaylistDto { own: true, ..PlaylistDto::from(p) })
            .collect();
        for p in liked?.iter().map(PlaylistDto::from) {
            if !out.iter().any(|o| o.id == p.id) {
                out.push(p);
            }
        }
        Ok(out)
    })
    .await
}

// ---------------------------------------------------------------- upload

#[tauri::command]
pub async fn upload_pick(app: AppHandle, image: bool) -> Cmd<Option<crate::upload::PickedFile>> {
    crate::upload::pick(&app, image).await
}

#[tauri::command]
pub async fn upload_track(app: AppHandle, state: State<'_, AppState>, form: crate::upload::UploadForm) -> Cmd<TrackDto> {
    crate::upload::upload(&app, &state, form).await
}

#[tauri::command]
pub async fn upload_delete(app: AppHandle, state: State<'_, AppState>, track_id: u64) -> Cmd<()> {
    crate::upload::delete(&app, &state, track_id).await
}

/// The user's own uploads (newest first), private ones too.
#[tauri::command]
pub async fn my_tracks(state: State<'_, AppState>) -> Cmd<Vec<MyTrack>> {
    let me = state.current_user().await?;
    let tracks = state.sc()?.own_tracks(me.id).await?;
    state.db.upsert_tracks(tracks.clone()).await?;
    Ok(tracks
        .iter()
        .map(|t| MyTrack {
            private: t.sharing.as_deref() == Some("private"),
            plays: t.playback_count.unwrap_or(0),
            created_at: t.created_at.clone(),
            track: TrackDto::from(t),
        })
        .collect())
}

#[derive(serde::Serialize)]
pub struct MyTrack {
    track: TrackDto,
    private: bool,
    plays: u64,
    created_at: Option<String>,
}

/// Adds a track to one of the user's playlists (to the end).
#[tauri::command]
pub async fn playlist_add_track(app: AppHandle, state: State<'_, AppState>, playlist_id: u64, track_id: u64) -> Cmd<()> {
    let sc = state.sc()?;
    let mut ids = sc.playlist_track_ids(playlist_id).await?;
    if ids.contains(&track_id) {
        return Err(AppError::Other("Этот трек уже есть в плейлисте".into()));
    }
    ids.push(track_id);
    let (method, url, body) = sc.playlist_tracks_request(playlist_id, &ids)?;
    sc_write_body(&app, &state, (method, url), Some(body), false).await?;
    set_cached_track_count(&state, playlist_id, ids.len() as u64).await
}

/// Removes a track from one of the user's playlists.
#[tauri::command]
pub async fn playlist_remove_track(app: AppHandle, state: State<'_, AppState>, playlist_id: u64, track_id: u64) -> Cmd<()> {
    let sc = state.sc()?;
    let mut ids = sc.playlist_track_ids(playlist_id).await?;
    let before = ids.len();
    ids.retain(|&id| id != track_id);
    if ids.len() == before {
        return Ok(());
    }
    let (method, url, body) = sc.playlist_tracks_request(playlist_id, &ids)?;
    sc_write_body(&app, &state, (method, url), Some(body), false).await?;
    set_cached_track_count(&state, playlist_id, ids.len() as u64).await
}

/// New private playlist, optionally with a first track.
#[tauri::command]
pub async fn playlist_create(app: AppHandle, state: State<'_, AppState>, title: String, track_id: Option<u64>) -> Cmd<()> {
    let title = title.trim();
    if title.is_empty() {
        return Err(AppError::Other("Введите название плейлиста".into()));
    }
    let tracks: Vec<u64> = track_id.into_iter().collect();
    create_playlist(&app, &state, title, &tracks, true).await
}

/// New playlist with these tracks; the library list is refreshed so it shows up at once.
pub(crate) async fn create_playlist(app: &AppHandle, state: &AppState, title: &str, tracks: &[u64], private: bool) -> AppResult<()> {
    let (method, url, body) = state.sc()?.playlist_create_request(title, tracks, private)?;
    sc_write_body(app, state, (method, url), Some(body), false).await?;
    library_playlists_fresh(state, true).await.map(|_| ())
}

/// «Импорт»: what the link points to (name, number of songs).
#[tauri::command]
pub async fn import_preview(state: State<'_, AppState>, input: String) -> Cmd<(String, usize)> {
    crate::import::preview(&state, &input).await
}

#[tauri::command]
pub async fn import_run(app: AppHandle, state: State<'_, AppState>, input: String, title: String, private: bool) -> Cmd<crate::import::ImportResult> {
    Box::pin(crate::import::run(&app, &state, &input, &title, private)).await
}

async fn set_cached_track_count(state: &AppState, playlist_id: u64, n: u64) -> AppResult<()> {
    if let Some((mut list, _)) = state.db.kv_get::<Vec<PlaylistDto>>("lib:playlists2").await? {
        if let Some(p) = list.iter_mut().find(|p| p.id == playlist_id) {
            p.track_count = n;
            state.db.kv_put("lib:playlists2", &list).await?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn library_artists(state: State<'_, AppState>, force: bool) -> Cmd<Vec<UserDto>> {
    let me = state.current_user().await?;
    let sc = state.sc()?;
    let pending = pending_follows(&state).await?;
    cached_list(&state, "lib:artists", force, || async move {
        let mut list: Vec<UserDto> = sc.followings(me.id).await?.iter().map(UserDto::from).collect();
        apply_pending_follows(&mut list, &pending);
        Ok(list)
    })
    .await
}

#[tauri::command]
pub async fn library_counts(state: State<'_, AppState>) -> Cmd<LibraryCounts> {
    let tracks = state.db.likes_count().await?;
    let pls = state.db.kv_get::<Vec<PlaylistDto>>("lib:playlists2").await?.map(|(v, _)| v).unwrap_or_default();
    let artists = state.db.kv_get::<Vec<UserDto>>("lib:artists").await?.map(|(v, _)| v.len() as u64).unwrap_or(0);
    let albums = pls.iter().filter(|p| p.is_album).count() as u64;
    Ok(LibraryCounts { tracks, playlists: pls.len() as u64 - albums, albums, artists })
}

const PENDING_FOLLOWS: &str = "pending:follows";

async fn pending_follows(state: &AppState) -> AppResult<Vec<(UserDto, bool)>> {
    Ok(state.db.kv_get::<Vec<(UserDto, bool)>>(PENDING_FOLLOWS).await?.map(|(v, _)| v).unwrap_or_default())
}

async fn send_follow(app: &AppHandle, state: &AppState, user_id: u64, follow: bool, quiet: bool) -> AppResult<()> {
    let req = state.sc()?.follow_request(user_id, follow)?;
    if quiet {
        sc_write_quiet(app, state, req, !follow).await
    } else {
        sc_write(app, state, req, !follow).await
    }
}

/// Puts queued follows / unfollows on top of a list from the server.
fn apply_pending_follows(list: &mut Vec<UserDto>, pending: &[(UserDto, bool)]) {
    for (user, follow) in pending {
        list.retain(|u| u.id != user.id);
        if *follow {
            list.insert(0, user.clone());
        }
    }
}

/// Follow / unfollow: like `like_set`, applied locally at once and queued
/// if SoundCloud refuses for now.
#[tauri::command]
pub async fn follow_set(app: AppHandle, state: State<'_, AppState>, user: UserDto, follow: bool) -> Cmd<()> {
    // keep the cached "Артисты" tab in sync without a refetch
    let mut list = state.db.kv_get::<Vec<UserDto>>("lib:artists").await?.map(|(v, _)| v).unwrap_or_default();
    apply_pending_follows(&mut list, &[(user.clone(), follow)]);
    state.db.kv_put("lib:artists", &list).await?;

    let mut pending = pending_follows(&state).await?;
    let was_pending = pending.iter().any(|(u, _)| u.id == user.id);
    pending.retain(|(u, _)| u.id != user.id);
    state.db.kv_put(PENDING_FOLLOWS, &pending).await?;
    match send_follow(&app, &state, user.id, follow, true).await {
        Ok(()) => {
            if was_pending {
                let _ = app.emit("likes:queue", ());
            }
            Ok(())
        }
        Err(e @ (AppError::AuthExpired | AppError::NotAuthorized)) => Err(e),
        Err(e) => {
            tracing::info!(error = %e, user = user.id, follow, "follow queued");
            if !(was_pending && !follow) {
                pending.push((user, follow));
                state.db.kv_put(PENDING_FOLLOWS, &pending).await?;
            }
            let _ = app.emit("likes:queue", ());
            Ok(())
        }
    }
}

async fn flush_pending_follows(app: &AppHandle, state: &AppState, quiet: bool) -> AppResult<usize> {
    let pending = pending_follows(state).await?;
    if pending.is_empty() {
        return Ok(0);
    }
    let mut left = Vec::new();
    for (user, follow) in pending {
        if !left.is_empty() {
            left.push((user, follow));
        } else if let Err(e) = send_follow(app, state, user.id, follow, quiet).await {
            tracing::debug!(error = %e, user = user.id, "queued follow still refused");
            left.push((user, follow));
        } else {
            let _ = app.emit("likes:queue", ());
        }
    }
    state.db.kv_put(PENDING_FOLLOWS, &left).await?;
    Ok(left.len())
}

#[derive(serde::Serialize)]
pub struct PendingFollow {
    user: UserDto,
    follow: bool,
}

#[tauri::command]
pub async fn follows_pending(state: State<'_, AppState>) -> Cmd<Vec<PendingFollow>> {
    Ok(pending_follows(&state).await?.into_iter().map(|(user, follow)| PendingFollow { user, follow }).collect())
}

// --------------------------------------------------------------- history

#[derive(serde::Serialize)]
pub struct HistoryItem {
    track: TrackDto,
    /// unix seconds
    played_at: i64,
}

#[tauri::command]
pub async fn history_page(state: State<'_, AppState>, offset: u32, limit: u32) -> Cmd<Vec<HistoryItem>> {
    Ok(state
        .db
        .history(offset, limit.min(500))
        .await?
        .into_iter()
        .map(|(track, played_at)| HistoryItem { track, played_at })
        .collect())
}

#[tauri::command]
pub async fn history_clear(state: State<'_, AppState>) -> Cmd<()> {
    state.db.history_clear().await
}

#[derive(serde::Serialize)]
pub struct PlaylistHistoryItem {
    playlist: PlaylistDto,
    played_at: i64,
}

#[tauri::command]
pub async fn history_playlist_add(state: State<'_, AppState>, playlist: PlaylistDto) -> Cmd<()> {
    state.db.playlist_played(playlist.id, serde_json::to_string(&playlist)?).await
}

#[tauri::command]
pub async fn history_playlists(state: State<'_, AppState>, limit: u32) -> Cmd<Vec<PlaylistHistoryItem>> {
    Ok(state
        .db
        .playlist_history(limit.min(300))
        .await?
        .into_iter()
        .filter_map(|(json, played_at)| serde_json::from_str(&json).ok().map(|playlist| PlaylistHistoryItem { playlist, played_at }))
        .collect())
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

// ---------------------------------------------------------------- home

/// SoundCloud's own shelves change a few times a day: 30 min cache.
const HOME_TTL: i64 = 30 * 60;

#[tauri::command]
pub async fn home_sections(state: State<'_, AppState>, force: bool) -> Cmd<Vec<crate::home::HomeSection>> {
    let cached = state.db.kv_get::<Vec<crate::home::HomeSection>>("home").await?;
    if !force {
        if let Some((v, at)) = &cached {
            if now() - at < HOME_TTL {
                return Ok(v.clone());
            }
        }
    }
    match state.sc()?.home().await {
        Ok(resp) => {
            let sections = crate::home::sections(&resp);
            state.db.kv_put("home", &sections).await?;
            Ok(sections)
        }
        // offline: yesterday's shelves are better than nothing
        Err(e) => cached.map(|(v, _)| v).ok_or(e),
    }
}

/// «Категории» tile backed by searches ("хиты 90-х", "suno ai" …): tracks the
/// most liked playlists for the theme agree on. Cached for a day.
#[tauri::command]
pub async fn category_tracks(state: State<'_, AppState>, key: String, queries: Vec<String>) -> Cmd<Vec<TrackDto>> {
    category_tracks_cached(&state, &key, &queries).await
}

async fn category_tracks_cached(state: &AppState, key: &str, queries: &[String]) -> AppResult<Vec<TrackDto>> {
    const TTL: i64 = 24 * 3600;
    let cache_key = format!("category2:{key}");
    if let Some((v, at)) = state.db.kv_get::<Vec<TrackDto>>(&cache_key).await? {
        if now() - at < TTL && !v.is_empty() {
            return Ok(v);
        }
    }
    let tracks = Box::pin(crate::home::category_tracks(&state.sc()?, queries)).await?;
    let list: Vec<TrackDto> = tracks.iter().map(TrackDto::from).collect();
    state.db.upsert_tracks(tracks).await?;
    state.db.kv_put(&cache_key, &list).await?;
    Ok(list)
}

#[derive(serde::Deserialize)]
pub struct CoverReq {
    key: String,
    mix: Option<String>,
    queries: Option<Vec<String>>,
    pick: Option<String>,
    #[serde(default)]
    chart: Option<String>,
}

/// Tile art for «Категории»: covers of the category's top-3 tracks (a fan of records).
/// Refreshed once a day in the background, one tile at a time; the page
/// listens to "categories:covers". Theme tracks get cached on the way, so
/// opening a theme is instant.
#[tauri::command]
pub async fn category_covers(app: AppHandle, state: State<'_, AppState>, tiles: Vec<CoverReq>) -> Cmd<std::collections::HashMap<String, Vec<String>>> {
    const TTL: i64 = 24 * 3600;
    let mut out = std::collections::HashMap::new();
    let mut stale = Vec::new();
    for t in tiles {
        match state.db.kv_get::<Vec<String>>(&format!("cover3:{}", t.key)).await? {
            Some((urls, at)) => {
                if !urls.is_empty() {
                    out.insert(t.key.clone(), urls);
                }
                if now() - at >= TTL {
                    stale.push(t);
                }
            }
            None => stale.push(t),
        }
    }
    if !stale.is_empty() && !state.covers_busy.swap(true, Ordering::AcqRel) {
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            use tauri::Manager;
            let state = app2.state::<AppState>();
            for t in stale {
                let tracks: Vec<TrackDto> = if let Some(urn) = &t.mix {
                    match state.sc() {
                        Ok(sc) => Box::pin(crate::home::mix(&sc, urn)).await.map(|(p, _)| p.tracks).unwrap_or_default(),
                        Err(_) => Vec::new(),
                    }
                } else if let Some(q) = &t.queries {
                    category_tracks_cached(&state, &t.key, q).await.unwrap_or_default()
                } else if let Some(kind) = &t.pick {
                    crate::picks::get(&state, kind).await.unwrap_or_default()
                } else if let Some(kind) = &t.chart {
                    Box::pin(crate::charts::page(&state, kind)).await.map(|p| p.tracks).unwrap_or_default()
                } else {
                    Vec::new()
                };
                // top-3 distinct covers
                let mut urls: Vec<String> = Vec::new();
                for u in tracks.into_iter().filter_map(|t| t.artwork_url) {
                    if !urls.contains(&u) {
                        urls.push(u);
                    }
                    if urls.len() == 3 {
                        break;
                    }
                }
                let _ = state.db.kv_put(&format!("cover3:{}", t.key), &urls).await;
                let _ = app2.emit("categories:covers", ());
                // gentle on SoundCloud and the network
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            }
            state.covers_busy.store(false, Ordering::Release);
        });
    }
    Ok(out)
}

/// Real charts on SoundCloud: "ru:day" | "world:day" | "ru:week" | "ru:month" | "ru:year" | "ru:2021" | "world:2019" …
#[tauri::command]
pub async fn chart_page(state: State<'_, AppState>, kind: String) -> Cmd<crate::charts::ChartPage> {
    Box::pin(crate::charts::page(&state, &kind)).await
}

/// Debug builds only: run the news check as if the last one was at `since`.
#[cfg(debug_assertions)]
#[tauri::command]
pub async fn debug_news_check(app: AppHandle, since: String) -> Cmd<()> {
    crate::news::debug_check(&app, since).await
}

/// Debug builds only: bit error rate between two tracks' audio (tuning fingerprint.rs).
#[cfg(debug_assertions)]
#[tauri::command]
pub async fn debug_fp_compare(state: State<'_, AppState>, a: u64, b: u64) -> Cmd<(f32, f32)> {
    let sc = state.sc()?;
    let mut fps = Vec::new();
    for id in [a, b] {
        let t = sc.track(id).await?;
        let bytes = Box::pin(sc.download_audio(&t)).await?;
        let fp = tauri::async_runtime::spawn_blocking(move || crate::fingerprint::compute(&bytes))
            .await
            .map_err(|e| AppError::Other(e.to_string()))?
            .ok_or_else(|| AppError::Other("no fingerprint".into()))?;
        fps.push(fp);
    }
    Ok(crate::fingerprint::best_match(&fps[0], &fps[1]))
}

/// Years with a Russian year-end chart.
#[tauri::command]
pub fn chart_years() -> Vec<u16> {
    crate::charts::ya_years()
}

/// «Только в SC Desk»: forgotten | year-ago | repeat | radar.
#[tauri::command]
pub async fn picks(state: State<'_, AppState>, kind: String) -> Cmd<Vec<TrackDto>> {
    crate::picks::get(&state, &kind).await
}

#[tauri::command]
pub async fn mix_page(state: State<'_, AppState>, urn: String) -> Cmd<crate::home::MixPage> {
    let (page, tracks) = crate::home::mix(&state.sc()?, &urn).await?;
    state.db.upsert_tracks(tracks).await?;
    Ok(page)
}

#[tauri::command]
pub async fn feed_page(state: State<'_, AppState>, next: Option<String>) -> Cmd<crate::home::FeedPage> {
    let resp = state.sc()?.stream(next.as_deref()).await?;
    let (items, tracks) = crate::home::feed_items(&resp);
    state.db.upsert_tracks(tracks).await?;
    let next = resp["next_href"].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    Ok(crate::home::FeedPage { items, next })
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
pub fn player_set_smart_shuffle(player: State<'_, Arc<Player>>, enabled: bool) {
    player.inner().set_smart_shuffle(enabled);
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
pub async fn wave_start_playlist(player: State<'_, Arc<Player>>, playlist_id: u64) -> Cmd<usize> {
    player.inner().start_wave_playlist(playlist_id).await
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
pub async fn wave_set_no_liked(state: State<'_, AppState>, player: State<'_, Arc<Player>>, enabled: bool) -> Cmd<()> {
    state.wave.set_no_liked(enabled);
    state.config.update(|c| c.wave_no_liked = enabled)?;
    if player.snapshot().source == QueueSource::Wave {
        player.inner().restart_wave().await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn wave_info(state: State<'_, AppState>, track_id: Option<u64>) -> Cmd<WaveInfo> {
    Ok(WaveInfo {
        mood: state.wave.mood(),
        no_liked: state.wave.no_liked(),
        reason: track_id.and_then(|id| state.wave.reason(id)),
        context: state.wave.context().map(|c| c.label()),
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
pub async fn disliked_tracks(state: State<'_, AppState>) -> Cmd<Vec<TrackDto>> {
    state.db.disliked_tracks().await
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
    // "love nwantiti slowed": searched without the mark; the original's text is the fallback
    let version_of = lyrics::version_base(&track.title).map(|b| genius::query_from(&track.artist, &b).1).filter(|b| !b.is_empty());
    let title = version_of.clone().unwrap_or(title);
    let q = Query { artist, title, duration_ms: track.duration_ms, version_of };
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
            original: l.original,
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
            original: false,
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
pub fn config_set_ui(app: AppHandle, state: State<'_, AppState>, ui: serde_json::Value) -> Cmd<()> {
    let lang_changed = crate::lang::set_from_ui(&ui);
    state.config.update(|c| c.ui = ui)?;
    if lang_changed {
        crate::tray::refresh_menu(&app);
    }
    Ok(())
}

/// Effects (speed, reverb, loudness, crossfade): live; `persist` on release of a slider.
#[tauri::command]
pub fn fx_set(state: State<'_, AppState>, player: State<'_, Arc<Player>>, fx: crate::config::FxConfig, persist: bool) -> Cmd<()> {
    player.set_fx(&fx);
    if persist {
        state.config.update(|c| c.fx = fx)?;
    }
    Ok(())
}

#[tauri::command]
pub fn sleep_set(player: State<'_, Arc<Player>>, minutes: Option<u32>, end_of_track: bool) -> crate::player::SleepState {
    player.inner().set_sleep(minutes, end_of_track);
    player.sleep_state()
}

#[tauri::command]
pub fn queue_get(player: State<'_, Arc<Player>>, limit: usize) -> crate::player::QueueView {
    player.queue_view(limit.clamp(1, 500))
}

#[tauri::command]
pub fn queue_move(player: State<'_, Arc<Player>>, from: usize, to: usize) {
    player.inner().queue_move(from, to);
}

#[tauri::command]
pub fn queue_remove(player: State<'_, Arc<Player>>, at: usize) {
    player.inner().queue_remove(at);
}

#[tauri::command]
pub fn queue_clear(player: State<'_, Arc<Player>>) {
    player.inner().queue_clear();
}

#[tauri::command]
pub fn queue_play(player: State<'_, Arc<Player>>, at: usize) {
    player.inner().queue_play(at);
}

/// Own playlist edited (order changed / tracks removed): the full new order.
#[tauri::command]
pub async fn playlist_set_tracks(app: AppHandle, state: State<'_, AppState>, playlist_id: u64, track_ids: Vec<u64>) -> Cmd<()> {
    let sc = state.sc()?;
    let (method, url, body) = sc.playlist_tracks_request(playlist_id, &track_ids)?;
    sc_write_body(&app, &state, (method, url), Some(body), false).await?;
    set_cached_track_count(&state, playlist_id, track_ids.len() as u64).await
}

/// Mini player: open (the big window goes to the tray).
#[tauri::command]
pub async fn mini_open(app: AppHandle) -> Cmd<()> {
    crate::mini::open(&app).await.map_err(AppError::from)
}

/// `restore`: back to the big window. Async: on Windows a window created from
/// a sync command hangs (blank) — the sign-in window had the same problem.
#[tauri::command]
pub async fn mini_close(app: AppHandle, restore: bool) -> Cmd<()> {
    crate::mini::close(&app, restore);
    Ok(())
}

#[tauri::command]
pub fn mini_resize(app: AppHandle, height: f64) {
    crate::mini::resize(&app, height);
}

/// «Итоги»: "week" | "month" | "year" | "all".
#[tauri::command]
pub async fn stats_get(state: State<'_, AppState>, period: String) -> Cmd<crate::db::cache::Stats> {
    let day = 86_400;
    let since = match period.as_str() {
        "week" => now() - 7 * day,
        "month" => now() - 30 * day,
        "year" => now() - 365 * day,
        _ => 0,
    };
    state.db.stats(since).await
}

#[tauri::command]
pub fn sleep_get(player: State<'_, Arc<Player>>) -> crate::player::SleepState {
    player.sleep_state()
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
    fast_protected: bool,
    notify_new: bool,
}

/// «Уведомлять о новых треках подписок».
#[tauri::command]
pub fn news_set(state: State<'_, AppState>, enabled: bool) -> Cmd<()> {
    state.config.update(|c| c.notify_new = enabled)?;
    Ok(())
}

#[tauri::command]
pub fn system_get(app: AppHandle, state: State<'_, AppState>) -> SystemPrefs {
    let cfg = state.config.get();
    SystemPrefs {
        autostart: crate::autostart::is_enabled(&app),
        start_minimized: cfg.start_minimized,
        fast_protected: cfg.fast_protected,
        notify_new: cfg.notify_new,
    }
}

#[tauri::command]
pub fn system_set(
    app: AppHandle,
    state: State<'_, AppState>,
    player: State<'_, Arc<Player>>,
    autostart: bool,
    start_minimized: bool,
    fast_protected: bool,
) -> Cmd<()> {
    let was = state.config.get().fast_protected;
    state.config.update(|c| {
        c.start_minimized = start_minimized;
        c.fast_protected = fast_protected;
    })?;
    if was != fast_protected {
        player.set_fast_protected(fast_protected);
    }
    if autostart != crate::autostart::is_enabled(&app) {
        crate::autostart::set(&app, autostart)?;
    }
    Ok(())
}
