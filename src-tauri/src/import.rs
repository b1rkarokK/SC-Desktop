//! Import a playlist from Yandex Music or Spotify (or a plain «Артист - Трек»
//! list) into a new SoundCloud playlist.
//!
//! Sources, all without signing in:
//! * Yandex Music: public playlists (`/users/{owner}/playlists/{kind}` and
//!   the newer `/playlists/{uuid}` links) and albums, through the same open
//!   API the charts use;
//! * Spotify: playlists and albums from the public embed page (its first 100
//!   tracks — the rest needs a Spotify login);
//! * text: one «Артист - Трек» per line.
//!
//! Each song is looked up on SoundCloud with the strict chart matching (the
//! official upload or the exact title within ±4 s, no remixes or covers),
//! remembered per song, then the found tracks go into a new playlist.

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use url::Url;

use crate::{
    charts::{self, ChartSong},
    error::{AppError, AppResult},
    state::AppState,
};

const YA: &str = "https://api.music.yandex.net";
/// SoundCloud playlists hold up to 500 tracks
const MAX_TRACKS: usize = 500;

#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub title: String,
    pub found: usize,
    pub total: usize,
    /// «Артист - Трек» of the songs not found on SoundCloud
    pub missing: Vec<String>,
}

struct Source {
    title: String,
    songs: Vec<ChartSong>,
}

/// Reads the source behind a link (or a pasted list) without importing it.
async fn read_source(state: &AppState, input: &str) -> AppResult<Source> {
    let input = input.trim();
    let http = state.http();
    if let Ok(url) = Url::parse(input) {
        let host = url.host_str().unwrap_or("").to_lowercase();
        let parts: Vec<&str> = url.path_segments().map(|s| s.filter(|p| !p.is_empty()).collect()).unwrap_or_default();
        if host.contains("yandex.") || host.contains("music.ya") {
            return yandex(&http, &parts).await;
        }
        if host.contains("spotify.com") {
            return spotify(&http, &parts).await;
        }
        return Err(AppError::Other("Нужна ссылка на плейлист или альбом Яндекс Музыки или Spotify".into()));
    }
    // a pasted list: «Артист - Трек» per line
    let songs: Vec<ChartSong> = input
        .lines()
        .filter_map(|l| {
            let l = l.trim().trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')').trim();
            let (a, t) = l.split_once(" - ").or_else(|| l.split_once(" — ")).or_else(|| l.split_once(" – "))?;
            (!a.trim().is_empty() && !t.trim().is_empty()).then(|| song(&format!("txt:{}", l.to_lowercase()), a.trim(), t.trim(), 0))
        })
        .collect();
    if songs.is_empty() {
        return Err(AppError::Other("Вставьте ссылку на плейлист или список строк «Артист - Трек»".into()));
    }
    Ok(Source { title: crate::lang::pick("Импорт", "Import").into(), songs })
}

fn song(key: &str, artist: &str, title: &str, duration_ms: u64) -> ChartSong {
    ChartSong { key: key.into(), artist: artist.into(), title: title.into(), duration_ms, listeners: 0 }
}

async fn yandex(http: &crate::net::HttpClient, parts: &[&str]) -> AppResult<Source> {
    let get = |path: String| async move { http.get_json_with::<Value>(&format!("{YA}{path}"), &[]).await };
    let not_open = || AppError::Other("Яндекс Музыка не отдала плейлист: он закрытый или ссылка неверная".into());
    let (title, tracks): (String, Vec<Value>) = match parts {
        ["users", owner, "playlists", kind, ..] => {
            let v = get(format!("/users/{owner}/playlists/{kind}")).await.map_err(|_| not_open())?;
            let r = &v["result"];
            (r["title"].as_str().unwrap_or("").into(), r["tracks"].as_array().cloned().unwrap_or_default().iter().map(|e| e["track"].clone()).collect())
        }
        ["playlists", uuid, ..] => {
            let v = get(format!("/playlist/{uuid}?rich-tracks=true")).await.map_err(|_| not_open())?;
            let r = &v["result"];
            (r["title"].as_str().unwrap_or("").into(), r["tracks"].as_array().cloned().unwrap_or_default().iter().map(|e| e["track"].clone()).collect())
        }
        ["album", id, ..] => {
            let v = get(format!("/albums/{id}/with-tracks")).await.map_err(|_| not_open())?;
            let r = &v["result"];
            let tracks = r["volumes"].as_array().cloned().unwrap_or_default().into_iter().flat_map(|vol| vol.as_array().cloned().unwrap_or_default()).collect();
            (r["title"].as_str().unwrap_or("").into(), tracks)
        }
        _ => return Err(AppError::Other("Нужна ссылка на плейлист или альбом Яндекс Музыки".into())),
    };
    let songs: Vec<ChartSong> = tracks.iter().filter_map(charts::ya_song).collect();
    if songs.is_empty() {
        return Err(not_open());
    }
    Ok(Source { title, songs })
}

async fn spotify(http: &crate::net::HttpClient, parts: &[&str]) -> AppResult<Source> {
    // "/intl-ru/playlist/…" links carry a locale first
    let parts: Vec<&str> = parts.iter().copied().filter(|p| !p.starts_with("intl-")).collect();
    let (kind, id) = match parts.as_slice() {
        ["playlist", id, ..] => ("playlist", *id),
        ["album", id, ..] => ("album", *id),
        _ => return Err(AppError::Other("Нужна ссылка на плейлист или альбом Spotify".into())),
    };
    let html = http.get_text(&format!("https://open.spotify.com/embed/{kind}/{id}"), crate::net::Profile::Document, None).await?;
    let json = html
        .split(r#"<script id="__NEXT_DATA__" type="application/json">"#)
        .nth(1)
        .and_then(|s| s.split("</script>").next())
        .ok_or_else(|| AppError::Other("Spotify не отдал плейлист: он закрытый или ссылка неверная".into()))?;
    let v: Value = serde_json::from_str(json)?;
    let e = &v["props"]["pageProps"]["state"]["data"]["entity"];
    let title = e["name"].as_str().or(e["title"].as_str()).unwrap_or("Spotify").to_owned();
    let songs: Vec<ChartSong> = e["trackList"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let uri = t["uri"].as_str()?;
            // "Artist 1, Artist 2": the first is the main one
            let artist = t["subtitle"].as_str()?.replace('\u{a0}', " ");
            Some(song(&format!("sp:{uri}"), &artist, t["title"].as_str()?, t["duration"].as_u64().unwrap_or(0)))
        })
        .collect();
    if songs.is_empty() {
        return Err(AppError::Other("Spotify не отдал плейлист: он закрытый или ссылка неверная".into()));
    }
    Ok(Source { title, songs })
}

/// Name of what the link points to and how many songs (the dialog shows it before importing).
pub async fn preview(state: &AppState, input: &str) -> AppResult<(String, usize)> {
    let src = read_source(state, input).await?;
    Ok((src.title, src.songs.len()))
}

/// Finds the songs on SoundCloud and creates the playlist. `import:progress`
/// {done, total} while looking up.
pub async fn run(app: &AppHandle, state: &AppState, input: &str, title: &str, private: bool) -> AppResult<ImportResult> {
    let mut src = read_source(state, input).await?;
    src.songs.truncate(MAX_TRACKS);
    let total = src.songs.len();
    let progress = |done: usize| {
        let _ = app.emit("import:progress", serde_json::json!({ "done": done, "total": total }));
    };
    progress(0);
    let (tracks, missing) = charts::find_all(state, &src.songs, progress).await?;
    if tracks.is_empty() {
        return Err(AppError::Other("Ни одной песни не нашлось на SoundCloud".into()));
    }
    let title = Some(title.trim()).filter(|t| !t.is_empty()).map(str::to_owned).unwrap_or(src.title);
    let ids: Vec<u64> = tracks.iter().map(|t| t.id).collect();
    crate::commands::create_playlist(app, state, &title, &ids, private).await?;
    tracing::info!(found = ids.len(), total, "import: playlist created");
    Ok(ImportResult {
        title,
        found: ids.len(),
        total,
        missing: missing.iter().map(|s| format!("{} - {}", s.artist, s.title)).collect(),
    })
}
