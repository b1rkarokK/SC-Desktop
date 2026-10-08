//! HTTP client that looks like a regular Chrome tab.
//!
//! * Direct connection by default: `no_proxy()` also disables the OS / env proxy
//!   auto-detection reqwest does on its own, so traffic leaves from the user's
//!   own ISP address unless a proxy is set explicitly in Settings.
//! * Headers mirror what Chrome sends for the same kind of request
//!   (client hints, sec-fetch-*, Origin/Referer of soundcloud.com) and are
//!   inserted in Chrome's order. Nothing identifies this as a third-party app.
//! * Retries with exponential backoff + jitter on connect/timeout errors, 429
//!   and 5xx; honours `Retry-After`. 401/403/404 are never retried.

use std::time::Duration;

use rand::Rng;
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue, RETRY_AFTER},
    Client, Method, Response, StatusCode,
};
use serde::de::DeserializeOwned;

use crate::{
    api::proxy,
    config::AppConfig,
    error::{AppError, AppResult},
};

const MAX_RETRIES: u32 = 4;
const BASE_DELAY_MS: u64 = 400;
const MAX_DELAY_MS: u64 = 8_000;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

/// What kind of request this is, from the browser's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// XHR from soundcloud.com to api-v2.soundcloud.com
    ScApi,
    /// Audio / HLS fetch from the SoundCloud player (CDN)
    Media,
    /// `<img>` load of artwork from the CDN
    Image,
    /// Top-level page navigation (Genius lyrics page)
    Document,
    /// Plain JSON API (Genius API)
    Json,
}

struct BrowserIdentity {
    user_agent: String,
    sec_ch_ua: String,
    platform: &'static str,
}

impl BrowserIdentity {
    fn new(major: u32) -> Self {
        let (os, platform) = if cfg!(target_os = "windows") {
            ("Windows NT 10.0; Win64; x64", "\"Windows\"")
        } else if cfg!(target_os = "macos") {
            ("Macintosh; Intel Mac OS X 10_15_7", "\"macOS\"")
        } else {
            ("X11; Linux x86_64", "\"Linux\"")
        };
        Self {
            user_agent: format!(
                "Mozilla/5.0 ({os}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36"
            ),
            sec_ch_ua: format!(
                "\"Chromium\";v=\"{major}\", \"Google Chrome\";v=\"{major}\", \"Not_A Brand\";v=\"99\""
            ),
            platform,
        }
    }
}

pub struct HttpClient {
    client: Client,
    id: BrowserIdentity,
}

impl HttpClient {
    pub fn new(cfg: &AppConfig) -> AppResult<Self> {
        let mut builder = Client::builder()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(45))
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(4)
            .gzip(true)
            .brotli(true)
            .zstd(true)
            .deflate(true);

        builder = match cfg.active_proxy() {
            Some(url) => {
                tracing::info!(proxy = %proxy::redact(url), "using explicit proxy");
                builder.proxy(proxy::build(url)?)
            }
            None => {
                tracing::info!("direct connection (no proxy)");
                builder.no_proxy()
            }
        };

        Ok(Self {
            client: builder.build()?,
            id: BrowserIdentity::new(cfg.chrome_version),
        })
    }

    fn headers(&self, profile: Profile, auth: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::with_capacity(14);
        let mut put = |name: &'static str, value: &str| {
            if let Ok(v) = HeaderValue::from_str(value) {
                h.append(HeaderName::from_static(name), v);
            }
        };

        if profile != Profile::Json {
            put("sec-ch-ua", &self.id.sec_ch_ua);
            put("sec-ch-ua-mobile", "?0");
            put("sec-ch-ua-platform", self.id.platform);
        }
        if profile == Profile::Document {
            put("upgrade-insecure-requests", "1");
        }
        if let Some(a) = auth {
            put("authorization", a);
        }
        put("user-agent", &self.id.user_agent);

        match profile {
            Profile::ScApi => {
                put("accept", "application/json, text/javascript, */*; q=0.01");
                put("origin", "https://soundcloud.com");
                put("sec-fetch-site", "same-site");
                put("sec-fetch-mode", "cors");
                put("sec-fetch-dest", "empty");
                put("referer", "https://soundcloud.com/");
            }
            Profile::Media => {
                put("accept", "*/*");
                put("origin", "https://soundcloud.com");
                put("sec-fetch-site", "cross-site");
                put("sec-fetch-mode", "cors");
                put("sec-fetch-dest", "empty");
                put("referer", "https://soundcloud.com/");
            }
            Profile::Image => {
                put("accept", "image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8");
                put("sec-fetch-site", "cross-site");
                put("sec-fetch-mode", "no-cors");
                put("sec-fetch-dest", "image");
                put("referer", "https://soundcloud.com/");
            }
            Profile::Document => {
                put(
                    "accept",
                    "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8",
                );
                put("sec-fetch-site", "none");
                put("sec-fetch-mode", "navigate");
                put("sec-fetch-user", "?1");
                put("sec-fetch-dest", "document");
            }
            Profile::Json => {
                put("accept", "application/json");
            }
        }
        // accept-encoding is added by reqwest itself (needed for transparent decompression)
        put("accept-language", "en-US,en;q=0.9");
        h
    }

    /// GET with retry/backoff. Returns only 2xx responses.
    pub async fn get(&self, url: &str, profile: Profile, auth: Option<&str>) -> AppResult<Response> {
        self.request_with(Method::GET, url, profile, auth, &[]).await
    }

    /// Any method with retry/backoff (PUT/DELETE are idempotent for likes).
    pub async fn request(&self, method: Method, url: &str, profile: Profile, auth: Option<&str>) -> AppResult<Response> {
        self.request_with(method, url, profile, auth, &[]).await
    }

    /// GET JSON with extra headers (Referer / Cookie for lyrics providers).
    pub async fn get_json_with<T: DeserializeOwned>(&self, url: &str, extra: &[(&'static str, &str)]) -> AppResult<T> {
        let resp = self.request_with(Method::GET, url, Profile::Json, None, extra).await?;
        let bytes = resp.bytes().await?;
        serde_json::from_slice(&bytes).map_err(|e| AppError::Parse(format!("{} ({})", e, host_of(url))))
    }

    pub async fn request_with(
        &self,
        method: Method,
        url: &str,
        profile: Profile,
        auth: Option<&str>,
        extra: &[(&'static str, &str)],
    ) -> AppResult<Response> {
        let mut attempt = 0u32;
        loop {
            let mut headers = self.headers(profile, auth);
            for (k, v) in extra {
                if let Ok(v) = HeaderValue::from_str(v) {
                    headers.insert(HeaderName::from_static(k), v);
                }
            }
            let mut req = self.client.request(method.clone(), url).headers(headers);
            if method != Method::GET {
                req = req.header(reqwest::header::CONTENT_LENGTH, "0");
            }
            if profile == Profile::Media {
                req = req.timeout(Duration::from_secs(180));
            }
            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return Ok(resp);
                    }
                    let retryable = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
                    if retryable && attempt < MAX_RETRIES {
                        let wait = retry_after(&resp).unwrap_or_else(|| backoff(attempt));
                        tracing::warn!(%status, attempt, wait_ms = wait.as_millis() as u64, host = host_of(url), "HTTP retry");
                        tokio::time::sleep(wait).await;
                        attempt += 1;
                        continue;
                    }
                    tracing::warn!(%status, host = host_of(url), "HTTP request failed");
                    return Err(AppError::from_status(status));
                }
                Err(e) if is_transient(&e) && attempt < MAX_RETRIES => {
                    let wait = backoff(attempt);
                    tracing::warn!(error = %e.without_url(), attempt, wait_ms = wait.as_millis() as u64, host = host_of(url), "network retry");
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Body download also retried: CDN connections drop mid-body more often than on connect.
    pub async fn get_bytes(&self, url: &str, profile: Profile, auth: Option<&str>) -> AppResult<Vec<u8>> {
        let mut attempt = 0u32;
        loop {
            let resp = self.get(url, profile, auth).await?;
            match resp.bytes().await {
                Ok(b) => return Ok(b.to_vec()),
                Err(e) if attempt < 2 => {
                    tracing::warn!(error = %e.without_url(), host = host_of(url), "body read failed, retrying");
                    tokio::time::sleep(backoff(attempt)).await;
                    attempt += 1;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    pub async fn get_text(&self, url: &str, profile: Profile, auth: Option<&str>) -> AppResult<String> {
        Ok(self.get(url, profile, auth).await?.text().await?)
    }

    pub async fn get_json<T: DeserializeOwned>(&self, url: &str, profile: Profile, auth: Option<&str>) -> AppResult<T> {
        let bytes = self.get_bytes(url, profile, auth).await?;
        serde_json::from_slice(&bytes).map_err(|e| AppError::Parse(format!("{} ({})", e, host_of(url))))
    }
}

fn is_transient(e: &reqwest::Error) -> bool {
    e.is_timeout() || e.is_connect() || e.is_request() || e.is_body()
}

fn backoff(attempt: u32) -> Duration {
    let exp = BASE_DELAY_MS.saturating_mul(1u64 << attempt.min(10)).min(MAX_DELAY_MS);
    let jitter = rand::thread_rng().gen_range(0..=BASE_DELAY_MS);
    Duration::from_millis(exp + jitter)
}

fn retry_after(resp: &Response) -> Option<Duration> {
    let secs: u64 = resp.headers().get(RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()?;
    Some(Duration::from_secs(secs).min(MAX_RETRY_AFTER))
}

/// Host only — full URLs carry client_id / track_authorization and must not hit the logs.
fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_bounded() {
        for a in 0..20 {
            assert!(backoff(a) <= Duration::from_millis(MAX_DELAY_MS + BASE_DELAY_MS));
        }
    }

    #[test]
    fn identity_matches_version() {
        let id = BrowserIdentity::new(150);
        assert!(id.user_agent.contains("Chrome/150.0.0.0"));
        assert!(id.sec_ch_ua.contains("v=\"150\""));
    }
}
