//! Uploading the user's own tracks, the way soundcloud.com/upload does it:
//!
//! 1. `POST /uploads/track-upload-policy {filename, filesize}` → a pre-signed
//!    storage URL (+ headers) and an upload `uid`;
//! 2. the file goes straight to that storage URL (`PUT`, streamed, progress);
//! 3. `POST /uploads/{uid}/track-transcoding`, then `GET` it until "finished";
//! 4. `POST /tracks {"track": {title, sharing, genre, tag_list, uid, …}}`;
//! 5. artwork, if chosen: `PUT /tracks/{urn}/artwork {"image_data": base64}`.
//!
//! Writes to api-v2 go through the browser bridge (DataDome), like likes do;
//! the file itself and the status polling go from Rust.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::{
    api::soundcloud::ScTrack,
    error::{AppError, AppResult},
    models::TrackDto,
    state::AppState,
};

const AUDIO_EXT: &[&str] = &["mp3", "wav", "flac", "aiff", "aif", "ogg", "oga", "m4a", "aac", "mp4", "wma", "opus", "amr", "alac"];
const IMAGE_EXT: &[&str] = &["jpg", "jpeg", "png", "gif", "webp"];
const MAX_AUDIO: u64 = 4 * 1024 * 1024 * 1024;
const MAX_IMAGE: u64 = 8 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub struct UploadForm {
    pub path: String,
    pub title: String,
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
    pub private: bool,
    #[serde(default)]
    pub artwork: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PickedFile {
    pub path: String,
    pub name: String,
    pub size: u64,
    /// file name without extension, a starting point for the title
    pub title: String,
}

#[derive(Clone, Serialize)]
struct Progress {
    /// "upload" | "transcode" | "save"
    stage: &'static str,
    /// 0..=100
    percent: u8,
}

fn emit(app: &AppHandle, stage: &'static str, percent: u8) {
    let _ = app.emit("upload:progress", Progress { stage, percent });
}

fn ext_of(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase()
}

/// Windows file picker for the audio or the artwork.
pub async fn pick(app: &AppHandle, image: bool) -> AppResult<Option<PickedFile>> {
    use crate::lang::pick;
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let dialog = app.dialog().file();
    let dialog = if image {
        dialog.set_title(pick("Обложка трека", "Track artwork")).add_filter(pick("Изображения", "Images"), IMAGE_EXT)
    } else {
        dialog.set_title(pick("Трек для загрузки", "Track to upload")).add_filter(pick("Аудио", "Audio"), AUDIO_EXT)
    };
    dialog.pick_file(move |p| {
        let _ = tx.send(p.and_then(|p| p.into_path().ok()));
    });
    let Some(path) = rx.await.ok().flatten() else { return Ok(None) };
    let size = tokio::fs::metadata(&path).await?.len();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let title = path.file_stem().map(|n| n.to_string_lossy().replace('_', " ").trim().to_owned()).unwrap_or_default();
    Ok(Some(PickedFile { path: path.to_string_lossy().into_owned(), name, size, title }))
}

/// api-v2 write through the bridge; the JSON answer back.
async fn write(app: &AppHandle, state: &AppState, method: &str, path: &str, body: Value) -> AppResult<Value> {
    let sc = state.sc()?;
    let url = sc.write_url(path)?;
    let (status, text) = crate::bridge::send_for_body(app, method, &url, sc.auth_header(), Some(&body.to_string())).await?;
    match status {
        200..=299 => Ok(serde_json::from_str(&text).unwrap_or(Value::Null)),
        0 => Err(AppError::Other("Нет связи с SoundCloud".into())),
        crate::bridge::BLOCKED => Err(AppError::Other("SoundCloud временно ограничил доступ".into())),
        s => {
            tracing::warn!(status = s, path, body = %text.chars().take(300).collect::<String>(), "upload step failed");
            // SoundCloud explains quota / format problems in the body
            let reason = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["errors"][0]["error_message"].as_str().or(v["error"].as_str()).map(str::to_owned));
            Err(AppError::Other(match reason {
                Some(r) => format!("SoundCloud не принял загрузку: {r}"),
                None => format!("SoundCloud не принял загрузку (код {s})"),
            }))
        }
    }
}

pub async fn upload(app: &AppHandle, state: &AppState, form: UploadForm) -> AppResult<TrackDto> {
    let path = PathBuf::from(&form.path);
    let ext = ext_of(&path);
    if !AUDIO_EXT.contains(&ext.as_str()) {
        return Err(AppError::Other("Этот формат SoundCloud не принимает: нужен mp3, wav, flac, aiff, ogg, m4a или похожий".into()));
    }
    let size = tokio::fs::metadata(&path).await.map_err(|_| AppError::Other("Файл не найден".into()))?.len();
    if size == 0 || size > MAX_AUDIO {
        return Err(AppError::Other("Файл пустой или больше 4 ГБ".into()));
    }
    let title = form.title.trim();
    if title.is_empty() {
        return Err(AppError::Other("Введите название трека".into()));
    }
    let artwork = match form.artwork.as_deref().filter(|a| !a.is_empty()) {
        Some(a) => {
            let p = PathBuf::from(a);
            if !IMAGE_EXT.contains(&ext_of(&p).as_str()) {
                return Err(AppError::Other("Обложка должна быть jpg, png, gif или webp".into()));
            }
            let bytes = tokio::fs::read(&p).await.map_err(|_| AppError::Other("Файл обложки не найден".into()))?;
            if bytes.len() as u64 > MAX_IMAGE {
                return Err(AppError::Other("Обложка больше 8 МБ".into()));
            }
            Some(bytes)
        }
        None => None,
    };
    let filename = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| format!("track.{ext}"));

    // 1. where to put the file
    emit(app, "upload", 0);
    let policy = write(app, state, "POST", "/uploads/track-upload-policy", json!({ "filename": filename, "filesize": size })).await?;
    let (Some(put_url), Some(uid)) = (policy["url"].as_str(), policy["uid"].as_str()) else {
        tracing::warn!(?policy, "unexpected upload policy");
        return Err(AppError::Other("SoundCloud не дал места для загрузки".into()));
    };
    let headers: Vec<(String, String)> = policy["headers"]
        .as_object()
        .map(|h| {
            h.iter()
                .filter(|(k, _)| !["content-length", "host", "expect"].contains(&k.to_lowercase().as_str()))
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    let has_type = headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type"));
    let mut headers = headers;
    if !has_type {
        headers.push(("Content-Type".into(), "application/octet-stream".into()));
    }

    // 2. the file itself
    let progress_app = app.clone();
    let last = std::sync::atomic::AtomicU8::new(0);
    state
        .http()
        .put_file(put_url, &headers, &path, move |sent, total| {
            let pct = ((sent * 100) / total.max(1)).min(100) as u8;
            if last.swap(pct, std::sync::atomic::Ordering::Relaxed) != pct {
                emit(&progress_app, "upload", pct);
            }
        })
        .await?;
    tracing::info!(size, "upload: file stored");

    // 3. transcoding
    emit(app, "transcode", 0);
    let mut tries = 0;
    loop {
        match write(app, state, "POST", &format!("/uploads/{uid}/track-transcoding"), json!({})).await {
            Ok(_) => break,
            Err(e) if tries < 4 => {
                tries += 1;
                tracing::debug!(error = %e, "transcoding trigger retry");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Err(e) => return Err(e),
        }
    }
    let sc = state.sc()?;
    let started = std::time::Instant::now();
    let mut delay = 1.0f64;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs_f64(delay)).await;
        let st: Value = match sc.upload_status(uid).await {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(error = %e, "transcoding status");
                Value::Null
            }
        };
        match st["status"].as_str().unwrap_or("") {
            "finished" => break,
            "failure" | "not_found" => {
                return Err(AppError::Other("SoundCloud не смог обработать файл: проверьте, что он открывается и это аудио".into()))
            }
            "transcoding" => emit(app, "transcode", st["percentage"].as_f64().unwrap_or(0.0).clamp(0.0, 100.0) as u8),
            _ => {}
        }
        if started.elapsed() > std::time::Duration::from_secs(30 * 60) {
            return Err(AppError::Other("SoundCloud слишком долго обрабатывает файл, попробуйте позже".into()));
        }
        delay = (delay * 1.5).min(10.0);
    }

    // 4. the track
    emit(app, "save", 0);
    let mut track = json!({
        "title": title,
        "sharing": if form.private { "private" } else { "public" },
        "uid": uid,
        "original_filename": filename,
    });
    if let Some(g) = form.genre.as_deref().map(str::trim).filter(|g| !g.is_empty()) {
        track["genre"] = json!(g);
    }
    if let Some(t) = form.tags.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        track["tag_list"] = json!(tag_list(t));
    }
    let created = write(app, state, "POST", "/tracks", json!({ "track": track })).await?;
    let created: ScTrack = serde_json::from_value(created.clone()).map_err(|e| {
        tracing::warn!(error = %e, "unexpected created track");
        AppError::Other("Трек загружен, но ответ SoundCloud не разобран: проверьте профиль на сайте".into())
    })?;

    // 5. artwork
    if let Some(img) = artwork {
        let urn = format!("soundcloud:tracks:{}", created.id);
        if let Err(e) = write(app, state, "PUT", &format!("/tracks/{urn}/artwork"), json!({ "image_data": base64(&img) })).await {
            // the track is there; a missing cover is not worth failing the upload
            tracing::warn!(error = %e, "artwork upload failed");
            let _ = app.emit("upload:warning", "Трек загружен, но обложку SoundCloud не принял");
        }
    }
    emit(app, "save", 100);
    tracing::info!(track = created.id, private = form.private, "upload: track created");
    // the create answer is thin (no title yet): the full track as its owner sees it
    let full = sc.track_as_owner(created.id).await.unwrap_or(created);
    let _ = state.db.upsert_tracks(vec![full.clone()]).await;
    Ok(TrackDto::from(&full))
}

pub async fn delete(app: &AppHandle, state: &AppState, track_id: u64) -> AppResult<()> {
    let sc = state.sc()?;
    let url = sc.write_url(&format!("/tracks/soundcloud:tracks:{track_id}"))?;
    match crate::bridge::send(app, "DELETE", &url, sc.auth_header(), None).await? {
        200..=299 | 404 => Ok(()),
        s => Err(AppError::Other(format!("SoundCloud не удалил трек (код {s})"))),
    }
}

/// "drift memphis, lo fi" → `drift memphis "lo fi"`: comma separated words,
/// multi-word tags quoted as SoundCloud stores them.
fn tag_list(s: &str) -> String {
    s.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| if t.contains(' ') { format!("\"{}\"", t.replace('"', "")) } else { t.to_owned() })
        .collect::<Vec<_>>()
        .join(" ")
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn tags_quote_phrases() {
        assert_eq!(tag_list("drift, memphis , lo fi"), r#"drift memphis "lo fi""#);
    }
}
