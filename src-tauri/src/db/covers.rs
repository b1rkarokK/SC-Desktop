//! Artwork cache: files named by SHA-256 of the URL, LRU-evicted at 200 MB.
//! Served to the WebView through the `cover://` URI scheme, so images never
//! touch the network from the WebView itself and the CSP stays `'self'`-only.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

use sha2::{Digest, Sha256};
use tauri::{
    http::{header, Request, Response, StatusCode},
    Manager, UriSchemeContext, UriSchemeResponder, Wry,
};
use url::Url;

use crate::{
    db::Db,
    error::{AppError, AppResult},
    net::{HttpClient, Profile},
    state::AppState,
};

pub const MAX_BYTES: u64 = 200 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

pub struct CoverCache {
    dir: PathBuf,
    db: Db,
    max_bytes: u64,
    evicting: AtomicBool,
}

impl CoverCache {
    pub fn new(dir: PathBuf, db: Db, max_bytes: u64) -> AppResult<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir, db, max_bytes, evicting: AtomicBool::new(false) })
    }

    pub async fn get(&self, http: &HttpClient, url: &str) -> AppResult<Vec<u8>> {
        validate(url)?;
        let key = hex::encode(Sha256::digest(url.as_bytes()));
        let path = self.dir.join(&key);

        if let Ok(bytes) = tokio::fs::read(&path).await {
            self.db.cover_touch(key).await?;
            return Ok(bytes);
        }

        let bytes = http.get_bytes(url, Profile::Image, None).await?;
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(AppError::Other("cover too large".into()));
        }
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &path).await?;
        self.db.cover_insert(key, bytes.len() as u64).await?;
        self.evict_if_needed().await;
        Ok(bytes)
    }

    async fn evict_if_needed(&self) {
        if self.evicting.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Err(e) = self.evict().await {
            tracing::warn!(error = %e, "cover eviction failed");
        }
        self.evicting.store(false, Ordering::Release);
    }

    async fn evict(&self) -> AppResult<()> {
        let mut total = self.db.cover_total_size().await?;
        if total <= self.max_bytes {
            return Ok(());
        }
        // evict down to 90% so we don't run eviction on every new cover
        let target = self.max_bytes / 10 * 9;
        while total > target {
            let batch = self.db.cover_lru(64).await?;
            if batch.is_empty() {
                break;
            }
            let mut keys = Vec::with_capacity(batch.len());
            for (key, size) in batch {
                let _ = tokio::fs::remove_file(self.dir.join(&key)).await;
                total = total.saturating_sub(size);
                keys.push(key);
                if total <= target {
                    break;
                }
            }
            self.db.cover_delete(keys).await?;
        }
        Ok(())
    }
}

/// Only SoundCloud's image CDN — the scheme handler must not become an open proxy.
fn validate(url: &str) -> AppResult<()> {
    let u = Url::parse(url)?;
    let ok = u.scheme() == "https" && u.host_str().is_some_and(|h| h.ends_with(".sndcdn.com"));
    if ok {
        Ok(())
    } else {
        Err(AppError::Other("cover host not allowed".into()))
    }
}

fn mime(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        [b'G', b'I', b'F', ..] => "image/gif",
        _ => "image/jpeg",
    }
}

/// `cover://localhost/<urlencoded artwork url>` (on Windows: `http://cover.localhost/...`).
pub fn protocol_handler(ctx: UriSchemeContext<'_, Wry>, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let app = ctx.app_handle().clone();
    let encoded = request.uri().path().trim_start_matches('/').to_owned();
    tauri::async_runtime::spawn(async move {
        let result = async {
            let url = urlencoding::decode(&encoded).map_err(|e| AppError::Parse(e.to_string()))?.into_owned();
            let state = app.try_state::<AppState>().ok_or_else(|| AppError::Other("not ready".into()))?;
            let http = state.http();
            state.covers.get(&http, &url).await
        }
        .await;

        let response = match result {
            Ok(bytes) => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime(&bytes))
                .header(header::CACHE_CONTROL, "max-age=604800, immutable")
                // lets the fullscreen view read the cover's colour via <canvas>
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .body(bytes),
            Err(e) => {
                tracing::debug!(error = %e, "cover not served");
                Response::builder().status(StatusCode::NOT_FOUND).body(Vec::new())
            }
        };
        responder.respond(response.unwrap_or_default());
    });
}
