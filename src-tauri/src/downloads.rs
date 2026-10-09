//! "Скачать": a track saved as a plain audio file into Music\SC Desk.
//! Downloaded tracks also play from that file (offline, no stream quota).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::{
    error::{AppError, AppResult},
    models::TrackDto,
    state::AppState,
};

const KEY: &str = "downloads";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Download {
    pub track: TrackDto,
    pub path: String,
    /// unix seconds
    pub at: i64,
}

pub async fn list(state: &AppState) -> AppResult<Vec<Download>> {
    Ok(state.db.kv_get::<Vec<Download>>(KEY).await?.map(|(v, _)| v).unwrap_or_default())
}

async fn save_list(state: &AppState, list: &[Download]) -> AppResult<()> {
    state.db.kv_put(KEY, &list.to_vec()).await
}

/// Downloaded files that still exist (the user may have deleted some by hand).
pub async fn existing(state: &AppState) -> AppResult<Vec<Download>> {
    let all = list(state).await?;
    let kept: Vec<Download> = all.iter().filter(|d| Path::new(&d.path).is_file()).cloned().collect();
    if kept.len() != all.len() {
        save_list(state, &kept).await?;
    }
    Ok(kept)
}

/// Local file of a downloaded track, if any.
pub async fn file_of(state: &AppState, track_id: u64) -> Option<PathBuf> {
    let list = list(state).await.ok()?;
    let d = list.into_iter().find(|d| d.track.id == track_id)?;
    let p = PathBuf::from(d.path);
    p.is_file().then_some(p)
}

const DIR_KEY: &str = "downloads:dir";

fn default_folder(app: &AppHandle) -> AppResult<PathBuf> {
    let base = app.path().audio_dir().or_else(|_| app.path().download_dir()).map_err(|e| AppError::Other(e.to_string()))?;
    Ok(base.join("SC Desk"))
}

/// Where tracks are saved: the folder the user picked, or Music\SC Desk.
pub async fn folder(app: &AppHandle) -> AppResult<PathBuf> {
    let state = app.state::<AppState>();
    match state.db.kv_get::<String>(DIR_KEY).await? {
        Some((dir, _)) if !dir.is_empty() => Ok(PathBuf::from(dir)),
        _ => default_folder(app),
    }
}

/// Windows folder picker; returns the new folder (unchanged if cancelled).
pub async fn pick_folder(app: &AppHandle) -> AppResult<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    let current = folder(app).await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app.dialog().file().set_title("Куда сохранять музыку");
    if current.is_dir() {
        dialog = dialog.set_directory(&current);
    }
    dialog.pick_folder(move |p| {
        let _ = tx.send(p.and_then(|p| p.into_path().ok()));
    });
    let Some(dir) = rx.await.ok().flatten() else { return Ok(current) };
    app.state::<AppState>().db.kv_put(DIR_KEY, &dir.to_string_lossy().into_owned()).await?;
    tracing::info!(dir = %dir.display(), "download folder changed");
    Ok(dir)
}

/// Windows-safe file name: no reserved characters, sane length.
fn file_name(t: &TrackDto, ext: &str) -> String {
    let raw = if t.artist.trim().is_empty() || t.title.to_lowercase().contains(&t.artist.to_lowercase()) {
        t.title.clone()
    } else {
        format!("{} - {}", t.artist, t.title)
    };
    let mut name: String = raw
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect();
    name = name.trim().trim_end_matches(['.', ' ']).to_owned();
    if name.chars().count() > 150 {
        name = name.chars().take(150).collect();
    }
    if name.is_empty() {
        name = t.id.to_string();
    }
    format!("{name}.{ext}")
}

fn extension(data: &[u8]) -> &'static str {
    if data.len() > 8 && &data[4..8] == b"ftyp" {
        "m4a"
    } else {
        "mp3"
    }
}

pub async fn download(app: &AppHandle, state: &AppState, track: TrackDto) -> AppResult<Download> {
    let sc = state.sc()?;
    let full = sc.track(track.id).await?;
    let data = match Box::pin(sc.download_audio(&full)).await {
        Ok(d) => d,
        Err(AppError::NotFound | AppError::UnsupportedStream(_)) if crate::api::soundcloud::has_drm_stream(&full) => {
            return Err(AppError::Other("Этот трек SoundCloud отдаёт только с защитой (DRM): слушать в программе можно, скачать нельзя".into()))
        }
        Err(e) => return Err(e),
    };
    state.db.log_stream().await?;
    let track = TrackDto::from(&full);

    let dir = folder(app).await?;
    tokio::fs::create_dir_all(&dir).await.map_err(|e| AppError::Other(format!("папка {}: {e}", dir.display())))?;
    let path = dir.join(file_name(&track, extension(&data)));
    tokio::fs::write(&path, &data).await.map_err(|e| AppError::Other(format!("не удалось сохранить файл: {e}")))?;
    tracing::info!(track = track.id, bytes = data.len(), "track downloaded");

    let item = Download {
        track,
        path: path.to_string_lossy().into_owned(),
        at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0),
    };
    let mut all = list(state).await?;
    all.retain(|d| d.track.id != item.track.id);
    all.insert(0, item.clone());
    save_list(state, &all).await?;
    Ok(item)
}

/// Removes from the list and deletes the file (it is ours: we wrote it).
pub async fn remove(state: &AppState, track_id: u64) -> AppResult<()> {
    let mut all = list(state).await?;
    if let Some(d) = all.iter().find(|d| d.track.id == track_id) {
        let _ = tokio::fs::remove_file(&d.path).await;
    }
    all.retain(|d| d.track.id != track_id);
    save_list(state, &all).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(artist: &str, title: &str) -> TrackDto {
        TrackDto {
            id: 7,
            title: title.into(),
            artist: artist.into(),
            user_id: 1,
            duration_ms: 0,
            artwork_url: None,
            permalink_url: None,
            genre: None,
        }
    }

    #[test]
    fn safe_names() {
        assert_eq!(file_name(&t("dekma", "в узел"), "mp3"), "dekma - в узел.mp3");
        assert_eq!(file_name(&t("a", "x/y: z?"), "mp3"), "a - x_y_ z_.mp3");
        assert_eq!(file_name(&t("dekma", "dekma - tentacles"), "m4a"), "dekma - tentacles.m4a");
        assert_eq!(file_name(&t("", "..."), "mp3"), "7.mp3");
    }
}
