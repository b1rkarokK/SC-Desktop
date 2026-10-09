//! SQLite cache: tracks, likes, related lists, lyrics, dislikes, play history,
//! stream quota log and cover LRU index.
//!
//! rusqlite is blocking, so every call hops onto the blocking pool and the
//! UI / async runtime never waits on disk.

use std::{
    collections::HashSet,
    path::Path,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};

use crate::{
    api::soundcloud::ScTrack,
    error::{AppError, AppResult},
    models::{DislikedArtist, LyricsDto, TrackDto},
};

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA temp_store = MEMORY;

CREATE TABLE IF NOT EXISTS tracks (
    id            INTEGER PRIMARY KEY,
    user_id       INTEGER NOT NULL,
    title         TEXT    NOT NULL,
    artist        TEXT    NOT NULL,
    genre         TEXT,
    duration_ms   INTEGER NOT NULL,
    artwork_url   TEXT,
    permalink_url TEXT,
    json          TEXT    NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS likes (
    track_id INTEGER PRIMARY KEY,
    position INTEGER NOT NULL,
    liked_at TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS likes_position ON likes(position);

CREATE TABLE IF NOT EXISTS related_cache (
    seed_id    INTEGER PRIMARY KEY,
    ids        TEXT    NOT NULL,
    fetched_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS lyrics2 (
    track_id   INTEGER PRIMARY KEY,
    json       TEXT    NOT NULL,
    found      INTEGER NOT NULL,
    fetched_at INTEGER NOT NULL
);
DROP TABLE IF EXISTS lyrics;

CREATE TABLE IF NOT EXISTS kv (
    key        TEXT    PRIMARY KEY,
    json       TEXT    NOT NULL,
    fetched_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS disliked_artists (
    user_id INTEGER PRIMARY KEY,
    name    TEXT    NOT NULL,
    at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS disliked_tracks (
    track_id INTEGER PRIMARY KEY,
    at       INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS plays (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id INTEGER NOT NULL,
    at       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS plays_at ON plays(at);

CREATE TABLE IF NOT EXISTS playlist_plays (
    playlist_id INTEGER PRIMARY KEY,
    json        TEXT    NOT NULL,
    at          INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS playlist_plays_at ON playlist_plays(at);

CREATE TABLE IF NOT EXISTS stream_log (at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS stream_log_at ON stream_log(at);

CREATE TABLE IF NOT EXISTS covers (
    key         TEXT    PRIMARY KEY,
    size        INTEGER NOT NULL,
    last_access INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS covers_access ON covers(last_access);
"#;

const TRACK_COLS: &str = "t.id, t.title, t.artist, t.user_id, t.duration_ms, t.artwork_url, t.permalink_url, t.genre";

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn row_to_dto(r: &Row<'_>) -> rusqlite::Result<TrackDto> {
    Ok(TrackDto {
        id: r.get::<_, i64>(0)? as u64,
        title: r.get(1)?,
        artist: r.get(2)?,
        user_id: r.get::<_, i64>(3)? as u64,
        duration_ms: r.get::<_, i64>(4)? as u64,
        artwork_url: r.get(5)?,
        permalink_url: r.get(6)?,
        genre: r.get(7)?,
    })
}

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn open(path: &Path) -> AppResult<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        // "lyrics not found" is retried once per launch: sources and matching improve
        conn.execute("DELETE FROM lyrics2 WHERE found = 0", [])?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    async fn run<R, F>(&self, f: F) -> AppResult<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Connection) -> rusqlite::Result<R> + Send + 'static,
    {
        let conn = self.conn.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let mut guard = conn.lock().unwrap_or_else(|p| p.into_inner());
            f(&mut guard)
        })
        .await
        .map_err(|e| AppError::Db(format!("worker: {e}")))?
        .map_err(AppError::from)
    }

    // ------------------------------------------------------------ tracks

    pub async fn upsert_tracks(&self, tracks: Vec<ScTrack>) -> AppResult<()> {
        if tracks.is_empty() {
            return Ok(());
        }
        let rows: Vec<(TrackDto, String)> = tracks
            .iter()
            .map(|t| Ok((TrackDto::from(t), serde_json::to_string(t)?)))
            .collect::<Result<_, serde_json::Error>>()?;
        self.run(move |c| {
            let tx = c.transaction()?;
            {
                let mut st = tx.prepare_cached(
                    "INSERT INTO tracks (id, user_id, title, artist, genre, duration_ms, artwork_url, permalink_url, json, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                     ON CONFLICT(id) DO UPDATE SET user_id=excluded.user_id, title=excluded.title, artist=excluded.artist,
                       genre=excluded.genre, duration_ms=excluded.duration_ms, artwork_url=excluded.artwork_url,
                       permalink_url=excluded.permalink_url, json=excluded.json, updated_at=excluded.updated_at",
                )?;
                let ts = now();
                for (d, json) in &rows {
                    st.execute(params![
                        d.id as i64,
                        d.user_id as i64,
                        d.title,
                        d.artist,
                        d.genre,
                        d.duration_ms as i64,
                        d.artwork_url,
                        d.permalink_url,
                        json,
                        ts
                    ])?;
                }
            }
            tx.commit()
        })
        .await
    }

    /// Full tracks in the order of `ids` (missing ids are skipped).
    pub async fn tracks_by_ids(&self, ids: Vec<u64>) -> AppResult<Vec<ScTrack>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let jsons = self
            .run(move |c| {
                let mut out = Vec::with_capacity(ids.len());
                let mut st = c.prepare_cached("SELECT json FROM tracks WHERE id = ?1")?;
                for id in ids {
                    if let Some(j) = st.query_row([id as i64], |r| r.get::<_, String>(0)).optional()? {
                        out.push(j);
                    }
                }
                Ok(out)
            })
            .await?;
        Ok(jsons.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
    }

    // ------------------------------------------------------------- likes

    /// Replaces the like list (newest first) atomically.
    pub async fn replace_likes(&self, likes: Vec<(u64, String)>) -> AppResult<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM likes", [])?;
            {
                let mut st = tx.prepare_cached("INSERT OR IGNORE INTO likes (track_id, position, liked_at) VALUES (?1, ?2, ?3)")?;
                for (pos, (id, at)) in likes.iter().enumerate() {
                    st.execute(params![*id as i64, pos as i64, at])?;
                }
            }
            tx.commit()
        })
        .await
    }

    pub async fn likes_ids(&self) -> AppResult<HashSet<u64>> {
        self.ids("SELECT track_id FROM likes").await
    }

    /// New like goes to the top (smallest position), like on soundcloud.com.
    pub async fn like_add(&self, track_id: u64) -> AppResult<()> {
        self.run(move |c| {
            c.execute(
                "INSERT OR IGNORE INTO likes (track_id, position, liked_at)
                 VALUES (?1, (SELECT COALESCE(MIN(position), 0) - 1 FROM likes), datetime('now'))",
                [track_id as i64],
            )
            .map(|_| ())
        })
        .await
    }

    pub async fn like_remove(&self, track_id: u64) -> AppResult<()> {
        self.run(move |c| c.execute("DELETE FROM likes WHERE track_id = ?1", [track_id as i64]).map(|_| ()))
            .await
    }

    pub async fn likes_count(&self) -> AppResult<u64> {
        self.run(|c| c.query_row("SELECT COUNT(*) FROM likes", [], |r| r.get::<_, i64>(0)))
            .await
            .map(|n| n as u64)
    }

    pub async fn likes_page(&self, offset: u32, limit: u32) -> AppResult<Vec<TrackDto>> {
        self.run(move |c| {
            let mut st = c.prepare_cached(&format!(
                "SELECT {TRACK_COLS} FROM likes l JOIN tracks t ON t.id = l.track_id ORDER BY l.position LIMIT ?1 OFFSET ?2"
            ))?;
            let rows = st.query_map(params![limit as i64, offset as i64], row_to_dto)?;
            rows.collect()
        })
        .await
    }

    /// Likes not played in the app since `since` (unix s), oldest likes first.
    pub async fn forgotten_likes(&self, since: i64, limit: u32) -> AppResult<Vec<TrackDto>> {
        self.run(move |c| {
            let mut st = c.prepare_cached(&format!(
                "SELECT {TRACK_COLS} FROM likes l JOIN tracks t ON t.id = l.track_id
                 WHERE NOT EXISTS (SELECT 1 FROM plays p WHERE p.track_id = l.track_id AND p.at > ?1)
                 ORDER BY l.position DESC LIMIT ?2"
            ))?;
            let rows = st.query_map(params![since, limit as i64], row_to_dto)?;
            rows.collect()
        })
        .await
    }

    /// Likes whose date (ISO, compared as text) falls in `[from, to)`, e.g. "2025-09-20".."2025-10-30".
    pub async fn likes_between(&self, from: String, to: String) -> AppResult<Vec<TrackDto>> {
        self.run(move |c| {
            let mut st = c.prepare_cached(&format!(
                "SELECT {TRACK_COLS} FROM likes l JOIN tracks t ON t.id = l.track_id
                 WHERE l.liked_at >= ?1 AND l.liked_at < ?2 ORDER BY l.liked_at"
            ))?;
            let rows = st.query_map(params![from, to], row_to_dto)?;
            rows.collect()
        })
        .await
    }

    /// Most played since `since` (2+ plays), most played first.
    pub async fn on_repeat(&self, since: i64, limit: u32) -> AppResult<Vec<TrackDto>> {
        self.run(move |c| {
            let mut st = c.prepare_cached(&format!(
                "SELECT {TRACK_COLS}, COUNT(*) AS n FROM plays p JOIN tracks t ON t.id = p.track_id
                 WHERE p.at > ?1 GROUP BY p.track_id HAVING n >= 2 ORDER BY n DESC, MAX(p.at) DESC LIMIT ?2"
            ))?;
            let rows = st.query_map(params![since, limit as i64], row_to_dto)?;
            rows.collect()
        })
        .await
    }

    pub async fn likes_all(&self) -> AppResult<Vec<TrackDto>> {
        self.likes_page(0, u32::MAX >> 1).await
    }

    /// Liked tracks with full metadata (genre/tags), newest first — the wave profile.
    pub async fn liked_full(&self, limit: u32) -> AppResult<Vec<ScTrack>> {
        let jsons = self
            .run(move |c| {
                let mut st = c.prepare_cached(
                    "SELECT t.json FROM likes l JOIN tracks t ON t.id = l.track_id ORDER BY l.position LIMIT ?1",
                )?;
                let rows = st.query_map([limit as i64], |r| r.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        Ok(jsons.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
    }

    // ----------------------------------------------------------- related

    pub async fn related_get(&self, seed: u64, max_age_secs: i64) -> AppResult<Option<Vec<u64>>> {
        let row = self
            .run(move |c| {
                c.query_row(
                    "SELECT ids FROM related_cache WHERE seed_id = ?1 AND fetched_at > ?2",
                    params![seed as i64, now() - max_age_secs],
                    |r| r.get::<_, String>(0),
                )
                .optional()
            })
            .await?;
        Ok(row.and_then(|j| serde_json::from_str(&j).ok()))
    }

    pub async fn related_put(&self, seed: u64, ids: Vec<u64>) -> AppResult<()> {
        let json = serde_json::to_string(&ids)?;
        self.run(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO related_cache (seed_id, ids, fetched_at) VALUES (?1, ?2, ?3)",
                params![seed as i64, json, now()],
            )
            .map(|_| ())
        })
        .await
    }

    // ------------------------------------------------------------ lyrics

    pub async fn lyrics_get(&self, track_id: u64) -> AppResult<Option<(LyricsDto, i64)>> {
        let row = self
            .run(move |c| {
                c.query_row(
                    "SELECT json, fetched_at FROM lyrics2 WHERE track_id = ?1",
                    [track_id as i64],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
                )
                .optional()
            })
            .await?;
        Ok(row.and_then(|(json, at)| {
            serde_json::from_str::<LyricsDto>(&json).ok().map(|mut l| {
                l.cached = true;
                (l, at)
            })
        }))
    }

    pub async fn lyrics_put(&self, l: LyricsDto) -> AppResult<()> {
        let json = serde_json::to_string(&l)?;
        self.run(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO lyrics2 (track_id, json, found, fetched_at) VALUES (?1, ?2, ?3, ?4)",
                params![l.track_id as i64, json, l.found as i64, now()],
            )
            .map(|_| ())
        })
        .await
    }

    // --------------------------------------------------------------- kv

    pub async fn kv_get<T: serde::de::DeserializeOwned + Send + 'static>(&self, key: &str) -> AppResult<Option<(T, i64)>> {
        let key = key.to_owned();
        let row = self
            .run(move |c| {
                c.query_row("SELECT json, fetched_at FROM kv WHERE key = ?1", [key], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })
                .optional()
            })
            .await?;
        Ok(row.and_then(|(j, at)| serde_json::from_str(&j).ok().map(|v| (v, at))))
    }

    pub async fn kv_put<T: serde::Serialize>(&self, key: &str, value: &T) -> AppResult<()> {
        let (key, json) = (key.to_owned(), serde_json::to_string(value)?);
        self.run(move |c| {
            c.execute("INSERT OR REPLACE INTO kv (key, json, fetched_at) VALUES (?1, ?2, ?3)", params![key, json, now()])
                .map(|_| ())
        })
        .await
    }

    pub async fn disliked_tracks_count(&self) -> AppResult<u64> {
        self.run(|c| c.query_row("SELECT COUNT(*) FROM disliked_tracks", [], |r| r.get::<_, i64>(0)))
            .await
            .map(|n| n as u64)
    }

    // ---------------------------------------------------------- dislikes

    pub async fn dislike_artist(&self, user_id: u64, name: String) -> AppResult<()> {
        self.run(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO disliked_artists (user_id, name, at) VALUES (?1, ?2, ?3)",
                params![user_id as i64, name, now()],
            )
            .map(|_| ())
        })
        .await
    }

    pub async fn undislike_artist(&self, user_id: u64) -> AppResult<()> {
        self.run(move |c| c.execute("DELETE FROM disliked_artists WHERE user_id = ?1", [user_id as i64]).map(|_| ()))
            .await
    }

    pub async fn dislike_track(&self, track_id: u64) -> AppResult<()> {
        self.run(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO disliked_tracks (track_id, at) VALUES (?1, ?2)",
                params![track_id as i64, now()],
            )
            .map(|_| ())
        })
        .await
    }

    pub async fn undislike_track(&self, track_id: u64) -> AppResult<()> {
        self.run(move |c| c.execute("DELETE FROM disliked_tracks WHERE track_id = ?1", [track_id as i64]).map(|_| ()))
            .await
    }

    pub async fn clear_dislikes(&self) -> AppResult<()> {
        self.run(|c| c.execute_batch("DELETE FROM disliked_artists; DELETE FROM disliked_tracks;")).await
    }

    pub async fn disliked_artists(&self) -> AppResult<Vec<DislikedArtist>> {
        self.run(|c| {
            let mut st = c.prepare_cached("SELECT user_id, name FROM disliked_artists ORDER BY at DESC")?;
            let rows = st.query_map([], |r| {
                Ok(DislikedArtist { user_id: r.get::<_, i64>(0)? as u64, name: r.get(1)? })
            })?;
            rows.collect()
        })
        .await
    }

    /// Disliked tracks with metadata, newest first (for per-track "Вернуть").
    pub async fn disliked_tracks(&self) -> AppResult<Vec<TrackDto>> {
        self.run(|c| {
            let mut st = c.prepare_cached(&format!(
                "SELECT {TRACK_COLS} FROM disliked_tracks d JOIN tracks t ON t.id = d.track_id ORDER BY d.at DESC"
            ))?;
            let rows = st.query_map([], row_to_dto)?;
            rows.collect()
        })
        .await
    }

    pub async fn disliked_track_ids(&self) -> AppResult<HashSet<u64>> {
        self.ids("SELECT track_id FROM disliked_tracks").await
    }

    // ------------------------------------------------------------- plays

    pub async fn record_play(&self, track_id: u64) -> AppResult<()> {
        self.run(move |c| {
            c.execute("INSERT INTO plays (track_id, at) VALUES (?1, ?2)", params![track_id as i64, now()])?;
            // keep history bounded
            c.execute("DELETE FROM plays WHERE id <= (SELECT MAX(id) FROM plays) - 5000", [])?;
            Ok(())
        })
        .await
    }

    /// Listening history, newest first; a track repeated back-to-back counts once.
    pub async fn history(&self, offset: u32, limit: u32) -> AppResult<Vec<(TrackDto, i64)>> {
        let rows = self
            .run(move |c| {
                let mut st = c.prepare_cached(&format!(
                    "SELECT {TRACK_COLS}, p.at FROM plays p JOIN tracks t ON t.id = p.track_id
                     ORDER BY p.id DESC LIMIT ?1 OFFSET ?2"
                ))?;
                let rows = st.query_map(params![limit as i64, offset as i64], |r| Ok((row_to_dto(r)?, r.get::<_, i64>(8)?)))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        let mut out: Vec<(TrackDto, i64)> = Vec::with_capacity(rows.len());
        for (t, at) in rows {
            if out.last().is_some_and(|(prev, _)| prev.id == t.id) {
                continue;
            }
            out.push((t, at));
        }
        Ok(out)
    }

    pub async fn history_clear(&self) -> AppResult<()> {
        self.run(|c| c.execute_batch("DELETE FROM plays; DELETE FROM playlist_plays;")).await
    }

    /// Playlist/album was started: keep only its latest play.
    pub async fn playlist_played(&self, playlist_id: u64, json: String) -> AppResult<()> {
        self.run(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO playlist_plays (playlist_id, json, at) VALUES (?1, ?2, ?3)",
                params![playlist_id as i64, json, now()],
            )?;
            c.execute("DELETE FROM playlist_plays WHERE playlist_id NOT IN (SELECT playlist_id FROM playlist_plays ORDER BY at DESC LIMIT 300)", [])?;
            Ok(())
        })
        .await
    }

    pub async fn playlist_history(&self, limit: u32) -> AppResult<Vec<(String, i64)>> {
        self.run(move |c| {
            let mut st = c.prepare_cached("SELECT json, at FROM playlist_plays ORDER BY at DESC LIMIT ?1")?;
            let rows = st.query_map([limit as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
            rows.collect()
        })
        .await
    }

    pub async fn recent_plays(&self, n: u32) -> AppResult<HashSet<u64>> {
        self.ids(&format!("SELECT track_id FROM plays ORDER BY id DESC LIMIT {n}")).await
    }

    async fn ids(&self, sql: &str) -> AppResult<HashSet<u64>> {
        let sql = sql.to_owned();
        self.run(move |c| {
            let mut st = c.prepare_cached(&sql)?;
            let rows = st.query_map([], |r| r.get::<_, i64>(0).map(|v| v as u64))?;
            rows.collect()
        })
        .await
    }

    // ------------------------------------------------------ stream quota

    pub async fn log_stream(&self) -> AppResult<()> {
        self.run(|c| {
            let ts = now();
            c.execute("INSERT INTO stream_log (at) VALUES (?1)", [ts])?;
            c.execute("DELETE FROM stream_log WHERE at < ?1", [ts - 2 * 86_400])?;
            Ok(())
        })
        .await
    }

    pub async fn streams_last_24h(&self) -> AppResult<u32> {
        self.run(|c| {
            c.query_row("SELECT COUNT(*) FROM stream_log WHERE at > ?1", [now() - 86_400], |r| r.get::<_, i64>(0))
        })
        .await
        .map(|n| n as u32)
    }

    // ------------------------------------------------------- cover index

    pub async fn cover_touch(&self, key: String) -> AppResult<()> {
        self.run(move |c| {
            c.execute("UPDATE covers SET last_access = ?1 WHERE key = ?2", params![now(), key]).map(|_| ())
        })
        .await
    }

    pub async fn cover_insert(&self, key: String, size: u64) -> AppResult<()> {
        self.run(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO covers (key, size, last_access) VALUES (?1, ?2, ?3)",
                params![key, size as i64, now()],
            )
            .map(|_| ())
        })
        .await
    }

    pub async fn cover_total_size(&self) -> AppResult<u64> {
        self.run(|c| c.query_row("SELECT COALESCE(SUM(size), 0) FROM covers", [], |r| r.get::<_, i64>(0)))
            .await
            .map(|n| n as u64)
    }

    /// Least recently used covers, oldest first.
    pub async fn cover_lru(&self, limit: u32) -> AppResult<Vec<(String, u64)>> {
        self.run(move |c| {
            let mut st = c.prepare_cached("SELECT key, size FROM covers ORDER BY last_access ASC LIMIT ?1")?;
            let rows = st.query_map([limit as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64)))?;
            rows.collect()
        })
        .await
    }

    pub async fn cover_delete(&self, keys: Vec<String>) -> AppResult<()> {
        if keys.is_empty() {
            return Ok(());
        }
        self.run(move |c| {
            let placeholders = vec!["?"; keys.len()].join(",");
            c.execute(&format!("DELETE FROM covers WHERE key IN ({placeholders})"), params_from_iter(keys.iter()))
                .map(|_| ())
        })
        .await
    }
}
