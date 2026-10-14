//! «Мои файлы»: the user's own audio files kept inside the app's folder and played like any track, never
//! uploaded anywhere. A file gets in only if it is really music: its signature is one of the audio formats the
//! player reads (MP3, M4A/AAC, Ogg Opus, FLAC, WAV) and it decodes as sound from start to end. It is kept
//! under a number with no extension of its own and is only ever read by the decoder, never opened by Windows.
//!
//! A local track has an id of its own range (2^52 and up, still exact in JavaScript): SoundCloud's ids are far
//! below, so the player, likes and rooms can tell one from the other.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::Digest;
use tauri::{AppHandle, Manager};

use crate::{
    error::{AppError, AppResult},
    models::TrackDto,
    state::AppState,
};

pub const BASE: u64 = 1 << 52;
const KEY: &str = "local:tracks";
/// a song is never bigger than this (a two-hour WAV mix is about 1.2 GB: WAV is not for keeping here)
const MAX_BYTES: u64 = 200 * 1024 * 1024;

pub fn is_local(id: u64) -> bool {
    id >= BASE
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalTrack {
    pub track: TrackDto,
    /// the format, for the label: MP3, M4A, OPUS, FLAC, WAV
    pub kind: String,
    pub bytes: u64,
    /// "mine": added here; "shared": received in a room and kept
    pub origin: String,
}

pub fn folder(app: &AppHandle) -> AppResult<PathBuf> {
    Ok(app.path().app_data_dir()?.join("local"))
}

fn file(dir: &Path, id: u64) -> PathBuf {
    dir.join(format!("{id}.bin"))
}

pub async fn list(state: &AppState) -> AppResult<Vec<LocalTrack>> {
    Ok(state.db.kv_get::<Vec<LocalTrack>>(KEY).await?.map(|(v, _)| v).unwrap_or_default())
}

async fn save_list(state: &AppState, l: &[LocalTrack]) -> AppResult<()> {
    state.db.kv_put(KEY, &l.to_vec()).await
}

/// The audio of a local track, if it is here.
pub async fn audio(app: &AppHandle, id: u64) -> Option<Vec<u8>> {
    tokio::fs::read(file(&folder(app).ok()?, id)).await.ok()
}

/// The format by its first bytes; None: not an audio file the player reads.
pub fn kind_of(data: &[u8]) -> Option<&'static str> {
    let head = |s: &[u8]| data.starts_with(s);
    if head(b"ID3") || (data.len() > 2 && data[0] == 0xFF && data[1] & 0xE0 == 0xE0) {
        Some("MP3")
    } else if data.len() > 12 && &data[4..8] == b"ftyp" {
        Some("M4A")
    } else if head(b"OggS") && crate::player::decode::is_opus(data) {
        Some("OPUS")
    } else if head(b"fLaC") {
        Some("FLAC")
    } else if head(b"RIFF") && data.len() > 12 && &data[8..12] == b"WAVE" {
        Some("WAV")
    } else {
        None
    }
}

/// How long the sound lasts, decoding it all: None if it is not sound from start to end.
fn decoded_ms(data: std::sync::Arc<Vec<u8>>) -> Option<u64> {
    use rodio::Source;
    let src = crate::player::decode::open(data).ok()?;
    let (ch, rate) = (u64::from(src.channels().max(1)), u64::from(src.sample_rate().max(1)));
    let n = src.count() as u64;
    let ms = n * 1000 / ch / rate;
    // at least 5 seconds of sound
    (ms >= 5_000).then_some(ms)
}

/// "Artist - Title.mp3" → (artist, title); a bare name is the title.
fn names(path: &Path) -> (String, String) {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    match stem.split_once(" - ") {
        Some((a, t)) => (a.trim().to_owned(), t.trim().to_owned()),
        None => (String::new(), stem.trim().to_owned()),
    }
}

/// Adds a file: checked, copied into the app's folder. The same file twice is one track.
pub async fn add(app: &AppHandle, state: &AppState, path: &Path) -> AppResult<LocalTrack> {
    let size = tokio::fs::metadata(path).await?.len();
    if size > MAX_BYTES {
        return Err(AppError::Other("файл больше 200 МБ".into()));
    }
    let data = tokio::fs::read(path).await?;
    let (artist, title) = names(path);
    let me = state.me().map(|m| m.username).unwrap_or_default();
    add_bytes(app, state, data, if artist.is_empty() { me } else { artist }, title, "mine").await
}

/// Adds audio already in hand (a file, or one received in a room).
pub async fn add_bytes(app: &AppHandle, state: &AppState, data: Vec<u8>, artist: String, title: String, origin: &str) -> AppResult<LocalTrack> {
    let kind = kind_of(&data).ok_or_else(|| AppError::Other("это не аудиофайл: подходят MP3, M4A, Opus, FLAC и WAV".into()))?;
    let hash = sha2::Sha256::digest(&data);
    let id = BASE | (u64::from_be_bytes(hash[..8].try_into().unwrap_or([0; 8])) & (BASE - 1));
    if let Some(t) = list(state).await?.into_iter().find(|t| t.track.id == id) {
        return Ok(t);
    }
    let data = std::sync::Arc::new(data);
    let probe = data.clone();
    let ms = tokio::task::spawn_blocking(move || decoded_ms(probe))
        .await
        .map_err(|e| AppError::Other(e.to_string()))?
        .ok_or_else(|| AppError::Other("файл не читается как музыка (повреждён или это не звук)".into()))?;
    let dir = folder(app)?;
    tokio::fs::create_dir_all(&dir).await?;
    tokio::fs::write(file(&dir, id), data.as_slice()).await?;
    let t = LocalTrack {
        track: TrackDto { id, title, artist, user_id: 0, duration_ms: ms, artwork_url: None, permalink_url: None, genre: None },
        kind: kind.to_owned(),
        bytes: data.len() as u64,
        origin: origin.to_owned(),
    };
    let mut all = list(state).await?;
    all.insert(0, t.clone());
    save_list(state, &all).await?;
    tracing::info!(id, kind, bytes = t.bytes, "local track added");
    Ok(t)
}

/// Every local track gone: the files and the list.
pub async fn clear(app: &AppHandle, state: &AppState) -> AppResult<()> {
    let dir = folder(app)?;
    for t in list(state).await? {
        let _ = tokio::fs::remove_file(file(&dir, t.track.id)).await;
    }
    save_list(state, &[]).await
}

pub async fn remove(app: &AppHandle, state: &AppState, id: u64) -> AppResult<()> {
    let _ = tokio::fs::remove_file(file(&folder(app)?, id)).await;
    let mut all = list(state).await?;
    all.retain(|t| t.track.id != id);
    save_list(state, &all).await
}

#[cfg(test)]
mod tests {
    use super::kind_of;

    #[test]
    fn only_audio_gets_in() {
        assert_eq!(kind_of(b"ID3\x04\x00rest"), Some("MP3"));
        assert_eq!(kind_of(b"\x00\x00\x00\x20ftypM4A rest"), Some("M4A"));
        assert_eq!(kind_of(b"fLaC...."), Some("FLAC"));
        assert_eq!(kind_of(b"RIFF\x00\x00\x00\x00WAVEfmt "), Some("WAV"));
        assert_eq!(kind_of(b"MZ\x90\x00 program"), None);
        assert_eq!(kind_of(b"PK\x03\x04 zip"), None);
        assert_eq!(kind_of(b"just text"), None);
    }
}
