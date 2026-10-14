//! «Мои файлы»: the user's own audio kept inside the app and played like any track, never uploaded anywhere.
//!
//! A song gets in only through the app and only if it is really music: its signature is one of the audio
//! formats the player reads (MP3, M4A/AAC, Ogg Opus, FLAC, WAV) and it decodes as sound from start to end.
//! The songs are not files in a folder: they live inside one storage file (`local.sqlite3`), so nothing can be
//! dropped in by hand. The list of songs and the SHA-256 of each sit apart, in the app's own database: a song
//! slipped into the storage some other way is not on the list, and one changed there no longer matches its
//! checksum, so neither is ever played (the changed one is removed).
//!
//! Inside the storage every song is packed as Opus 96 kbit/s (unless the file is small already) and encrypted
//! (ChaCha20-Poly1305) with a key kept in the Windows credential store, the song's id bound into it: without
//! the key the storage is noise, and a row edited, swapped or slipped in fails its tag and is removed.
//!
//! A local track has an id of its own range (2^52 and up, still exact in JavaScript): SoundCloud's ids are far
//! below, so the player, likes and rooms can tell one from the other.

use std::path::PathBuf;

use rusqlite::{params, Connection, OptionalExtension};
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
/// a song is never bigger than this
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
    /// SHA-256 of the song as added: one changed in the storage is not played
    #[serde(default)]
    pub sha: String,
}

/// The key of the storage, made once and kept in the Windows credential store.
fn key() -> AppResult<[u8; 32]> {
    use base64::Engine as _;
    let service = match crate::secrets::profile() {
        Some(p) => format!("dev.scdesk.app.{p}.local"),
        None => "dev.scdesk.app.local".to_owned(),
    };
    let entry = keyring::Entry::new(&service, "storage").map_err(|e| AppError::Keyring(e.to_string()))?;
    if let Ok(k) = entry.get_password() {
        if let Ok(b) = base64::engine::general_purpose::STANDARD.decode(k) {
            if let Ok(k) = <[u8; 32]>::try_from(b.as_slice()) {
                return Ok(k);
            }
        }
    }
    let k: [u8; 32] = rand::random();
    entry.set_password(&base64::engine::general_purpose::STANDARD.encode(k)).map_err(|e| AppError::Keyring(e.to_string()))?;
    Ok(k)
}

fn seal(id: u64, plain: &[u8]) -> AppResult<Vec<u8>> {
    use chacha20poly1305::{aead::{Aead, KeyInit, Payload}, ChaCha20Poly1305, Nonce};
    let cipher = ChaCha20Poly1305::new(&key()?.into());
    let nonce: [u8; 12] = rand::random();
    let mut out = nonce.to_vec();
    out.extend(cipher.encrypt(Nonce::from_slice(&nonce), Payload { msg: plain, aad: &id.to_be_bytes() }).map_err(|_| AppError::Other("шифрование".into()))?);
    Ok(out)
}

fn open_sealed(id: u64, sealed: &[u8]) -> Option<Vec<u8>> {
    use chacha20poly1305::{aead::{Aead, KeyInit, Payload}, ChaCha20Poly1305, Nonce};
    if sealed.len() < 12 {
        return None;
    }
    let cipher = ChaCha20Poly1305::new(&key().ok()?.into());
    cipher.decrypt(Nonce::from_slice(&sealed[..12]), Payload { msg: &sealed[12..], aad: &id.to_be_bytes() }).ok()
}

/// The storage file of «Мои файлы».
pub fn store_path(app: &AppHandle) -> AppResult<PathBuf> {
    Ok(app.path().app_data_dir()?.join("local.sqlite3"))
}

fn open(path: &PathBuf) -> AppResult<Connection> {
    let c = Connection::open(path)?;
    c.execute_batch("CREATE TABLE IF NOT EXISTS songs (id INTEGER PRIMARY KEY, data BLOB NOT NULL);")?;
    Ok(c)
}

/// Runs on a blocking thread with the storage open.
async fn with_store<T: Send + 'static>(app: &AppHandle, f: impl FnOnce(&Connection) -> AppResult<T> + Send + 'static) -> AppResult<T> {
    let path = store_path(app)?;
    tokio::task::spawn_blocking(move || f(&open(&path)?)).await.map_err(|e| AppError::Other(e.to_string()))?
}

pub async fn list(state: &AppState) -> AppResult<Vec<LocalTrack>> {
    Ok(state.db.kv_get::<Vec<LocalTrack>>(KEY).await?.map(|(v, _)| v).unwrap_or_default())
}

async fn save_list(state: &AppState, l: &[LocalTrack]) -> AppResult<()> {
    state.db.kv_put(KEY, &l.to_vec()).await
}

/// The audio of a local track, if it is here and still the very song that was added.
pub async fn audio(app: &AppHandle, id: u64) -> Option<Vec<u8>> {
    let state = app.state::<AppState>();
    let t = list(&state).await.ok()?.into_iter().find(|t| t.track.id == id)?;
    let key = id as i64;
    let sealed = with_store(app, move |c| Ok(c.query_row("SELECT data FROM songs WHERE id = ?1", [key], |r| r.get::<_, Vec<u8>>(0)).optional()?)).await.ok()??;
    let data = open_sealed(id, &sealed).unwrap_or_default();
    if data.is_empty() || hex::encode(sha2::Sha256::digest(&data)) != t.sha {
        tracing::warn!(id, "local track changed in the storage: refused and removed");
        let _ = remove(app, &state, id).await;
        return None;
    }
    Some(data)
}

/// The storage and the list agree again: songs that are not on the list (slipped in some other way) are
/// dropped, list entries without their song leave. Done when the list is opened and at the start.
pub async fn sweep(app: &AppHandle, state: &AppState) -> AppResult<Vec<LocalTrack>> {
    let mut all = list(state).await?;
    let keep: Vec<i64> = all.iter().map(|t| t.track.id as i64).collect();
    let present: Vec<i64> = with_store(app, move |c| {
        let ids: Vec<i64> = c.prepare("SELECT id FROM songs")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        for id in ids.iter().filter(|id| !keep.contains(id)) {
            tracing::info!(id, "local storage: a song not added through the app, removed");
            c.execute("DELETE FROM songs WHERE id = ?1", [id])?;
        }
        Ok(ids)
    })
    .await?;
    let before = all.len();
    all.retain(|t| present.contains(&(t.track.id as i64)));
    if all.len() != before {
        save_list(state, &all).await?;
    }
    Ok(all)
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
fn names(path: &std::path::Path) -> (String, String) {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    match stem.split_once(" - ") {
        Some((a, t)) => (a.trim().to_owned(), t.trim().to_owned()),
        None => (String::new(), stem.trim().to_owned()),
    }
}

/// Adds a file: checked, copied into the storage. The same file twice is one track.
pub async fn add(app: &AppHandle, state: &AppState, path: &std::path::Path) -> AppResult<LocalTrack> {
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
    // packed small, unless it is small already (a second lossy pass would only cost quality)
    let src = data.clone();
    let packed = tokio::task::spawn_blocking(move || crate::player::decode::pack_opus(src)).await.map_err(|e| AppError::Other(e.to_string()))?;
    let (stored, kind): (Vec<u8>, &str) = match packed {
        Some(p) if (p.len() as f64) < data.len() as f64 * 0.7 => (p, "OPUS"),
        _ => (data.as_slice().to_vec(), kind),
    };
    let bytes = stored.len() as u64;
    let sha = hex::encode(sha2::Sha256::digest(&stored));
    let sealed = seal(id, &stored)?;
    with_store(app, move |c| {
        c.execute("INSERT OR REPLACE INTO songs (id, data) VALUES (?1, ?2)", params![id as i64, sealed])?;
        Ok(())
    })
    .await?;
    let t = LocalTrack {
        track: TrackDto { id, title, artist, user_id: 0, duration_ms: ms, artwork_url: None, permalink_url: None, genre: None },
        kind: kind.to_owned(),
        bytes,
        origin: origin.to_owned(),
        sha,
    };
    let mut all = list(state).await?;
    all.insert(0, t.clone());
    save_list(state, &all).await?;
    tracing::info!(id, kind, bytes, "local track added");
    Ok(t)
}

/// Every local track gone; the storage file shrinks back.
pub async fn clear(app: &AppHandle, state: &AppState) -> AppResult<()> {
    with_store(app, |c| {
        c.execute_batch("DELETE FROM songs; VACUUM;")?;
        Ok(())
    })
    .await?;
    save_list(state, &[]).await
}

pub async fn remove(app: &AppHandle, state: &AppState, id: u64) -> AppResult<()> {
    let key = id as i64;
    with_store(app, move |c| {
        c.execute("DELETE FROM songs WHERE id = ?1", [key])?;
        Ok(())
    })
    .await?;
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
