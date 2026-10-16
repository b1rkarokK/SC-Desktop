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
    /// "127.0.0.1:10808" while an explicit proxy is on (for a plain message when it does not answer)
    proxy: Option<String>,
}

impl HttpClient {
    pub fn new(cfg: &AppConfig) -> AppResult<Self> {
        let mut builder = Client::builder()
            .use_rustls_tls()
            // SoundCloud over DoH while the block bypass is on (doh.rs), the system's DNS otherwise
            .dns_resolver(std::sync::Arc::new(super::doh::Resolver))
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
                // SoundCloud blocked by the provider: its connections may go through the cutting proxy (bypass.rs)
                super::bypass::set_mode_from_ui(&cfg.ui);
                super::bypass::start();
                builder.no_proxy().proxy(reqwest::Proxy::custom(|url| super::bypass::proxy_for(url)))
            }
        };

        let proxy_addr = cfg.active_proxy().map(proxy::redact).and_then(|r| {
            // "socks5h://127.0.0.1:10808" or with a login in front of the @: the address after it
            let after = r.split("://").last().unwrap_or(&r).to_owned();
            Some(after.rsplit('@').next().unwrap_or(&after).trim_end_matches('/').to_owned())
        });
        Ok(Self {
            client: builder.build()?,
            id: BrowserIdentity::new(cfg.chrome_version),
            proxy: proxy_addr,
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
                    let body: String = resp.text().await.unwrap_or_default().chars().take(300).collect();
                    tracing::warn!(%status, host = host_of(url), path = path_of(url), %method, body = %body, "HTTP request failed");
                    return Err(AppError::from_status(status));
                }
                // SoundCloud broke off right at the connection: in «Авто» the bypass turns on, asked again at once
                Err(e) if (e.is_connect() || e.is_timeout()) && super::bypass::on_failure(url) => continue,
                Err(e) if is_transient(&e) && attempt < MAX_RETRIES => {
                    let wait = backoff(attempt);
                    tracing::warn!(error = %e.without_url(), attempt, wait_ms = wait.as_millis() as u64, host = host_of(url), "network retry");
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                }
                Err(e) => return Err(self.proxy_hint(&e).unwrap_or_else(|| cut_off(url, e.into()))),
            }
        }
    }

    /// An explicit proxy that does not answer (the VPN program is off): said plainly, with what to do.
    fn proxy_hint(&self, e: &reqwest::Error) -> Option<AppError> {
        let addr = self.proxy.as_deref()?;
        if !(e.is_connect() || e.is_timeout()) {
            return None;
        }
        Some(AppError::Network(format!("Прокси {addr} не отвечает: включите VPN-программу или выберите «Прямое подключение» в Настройки → Сеть")))
    }

    /// POST of a JSON body, JSON answer (GraphQL of a lyrics provider).
    pub async fn post_json(&self, url: &str, extra: &[(&'static str, &str)], body: &serde_json::Value) -> AppResult<serde_json::Value> {
        let mut req = self
            .client
            .post(url)
            .timeout(Duration::from_secs(20))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(body)?);
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(AppError::from_status(status));
        }
        let bytes = resp.bytes().await?;
        serde_json::from_slice(&bytes).map_err(|e| AppError::Parse(format!("{} ({})", e, host_of(url))))
    }

    /// Uploads a local file with a plain PUT (a pre-signed storage URL: no
    /// cookies, no auth of ours). `progress(sent, total)` is called as it goes.
    pub async fn put_file(
        &self,
        url: &str,
        headers: &[(String, String)],
        path: &std::path::Path,
        progress: impl Fn(u64, u64) + Send + Sync + 'static,
    ) -> AppResult<()> {
        use futures::StreamExt;
        let file = tokio::fs::File::open(path).await?;
        let total = file.metadata().await?.len();
        let progress = std::sync::Arc::new(progress);
        let sent = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let stream = futures::stream::unfold(file, |mut f| async move {
            use tokio::io::AsyncReadExt;
            let mut buf = vec![0u8; 256 * 1024];
            match f.read(&mut buf).await {
                Ok(0) => None,
                Ok(n) => {
                    buf.truncate(n);
                    Some((Ok::<_, std::io::Error>(buf), f))
                }
                Err(e) => Some((Err(e), f)),
            }
        })
        .map(move |chunk| {
            if let Ok(c) = &chunk {
                let now = sent.fetch_add(c.len() as u64, std::sync::atomic::Ordering::Relaxed) + c.len() as u64;
                progress(now, total);
            }
            chunk
        });
        let mut req = self
            .client
            .put(url)
            .timeout(Duration::from_secs(60 * 60))
            .header(reqwest::header::CONTENT_LENGTH, total)
            .body(reqwest::Body::wrap_stream(stream));
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send().await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let body: String = resp.text().await.unwrap_or_default().chars().take(300).collect();
        tracing::warn!(%status, host = host_of(url), body = %body, "file upload failed");
        Err(AppError::from_status(status))
    }

    /// Body download also retried: CDN connections drop mid-body more often than on connect.
    /// A body cut off midway is resumed from where it stopped (Range), not fetched again from the start: a
    /// provider that kills a connection after some seconds then costs a reconnect, not the whole download.
    pub async fn get_bytes(&self, url: &str, profile: Profile, auth: Option<&str>) -> AppResult<Vec<u8>> {
        let mut data: Vec<u8> = Vec::new();
        let mut breaks = 0u32;
        loop {
            let range = format!("bytes={}-", data.len());
            let extra: &[(&'static str, &str)] = if data.is_empty() { &[] } else { &[("range", range.as_str())] };
            let mut resp = self.request_with(Method::GET, url, profile, auth, extra).await?;
            // the server ignored the range: from the start again
            if !data.is_empty() && resp.status() != StatusCode::PARTIAL_CONTENT {
                data.clear();
            }
            let cut = loop {
                match resp.chunk().await {
                    Ok(Some(c)) => data.extend_from_slice(&c),
                    Ok(None) => break None,
                    Err(e) => break Some(e),
                }
            };
            let Some(e) = cut else { return Ok(data) };
            // the address may carry a one-time key: never in the log
            let e = e.without_url();
            breaks += 1;
            tracing::warn!(error = %e, host = host_of(url), have = data.len(), breaks, "body cut off, resuming");
            if breaks > 30 {
                return Err(cut_off(url, e.into()));
            }
            tokio::time::sleep(backoff(breaks.min(2))).await;
        }
    }

    /// A big file (a model) straight to disk, chunk by chunk: never the whole of it in memory, no 45 s limit.
    /// Returns the SHA-256 of what was written.
    pub async fn download_file(&self, url: &str, path: &std::path::Path) -> AppResult<String> {
        use sha2::Digest;
        use tokio::io::AsyncWriteExt;
        let mut resp = self.client.get(url).timeout(Duration::from_secs(30 * 60)).header("user-agent", &self.id.user_agent).send().await?;
        if !resp.status().is_success() {
            return Err(AppError::from_status(resp.status()));
        }
        let mut file = tokio::fs::File::create(path).await?;
        let mut hash = sha2::Sha256::new();
        while let Some(chunk) = resp.chunk().await? {
            hash.update(&chunk);
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(hex::encode(hash.finalize()))
    }

    /// A plain GET with this user agent (a VPN panel answers by the client it sees).
    pub async fn get_text_ua(&self, url: &str, ua: &str, extra: &[(&str, &str)]) -> AppResult<String> {
        let mut req = self.client.get(url).header("user-agent", ua);
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(AppError::from_status(resp.status()));
        }
        Ok(resp.text().await?)
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
/// The path of a URL without the query (no ids or tokens in the log).
fn path_of(url: &str) -> String {
    url::Url::parse(url).ok().map(|u| u.path().to_owned()).unwrap_or_default()
}

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

/// SoundCloud keeps breaking off even with the built-in bypass on: the provider blocks it by address, which
/// cutting the greeting cannot get around. Said plainly, with what helps.
fn cut_off(url: &str, e: AppError) -> AppError {
    let sc = url::Url::parse(url).ok().and_then(|u| u.host_str().map(super::bypass::is_soundcloud)).unwrap_or(false);
    if sc && super::bypass::is_on() {
        AppError::Network("SoundCloud обрывает соединение даже через встроенный обход: провайдер блокирует его по адресу. Поможет «Свой сервер» или VPN (Настройки → Сеть)".into())
    } else {
        e
    }
}

#[cfg(test)]
mod resume_tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A server that breaks off every connection after 3000 bytes (as a provider does): the download still
    /// arrives whole, resumed by Range each time instead of starting over.
    #[tokio::test]
    async fn a_body_cut_off_again_and_again_arrives_whole() {
        let body: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let served = body.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else { return };
                let body = served.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = s.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                    let from: usize = req.split("range: bytes=").nth(1).and_then(|r| r.split('-').next()).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                    let rest = &body[from..];
                    let head = if from > 0 {
                        format!("HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: bytes {}-{}/{}\r\n\r\n", rest.len(), from, body.len() - 1, body.len())
                    } else {
                        format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", rest.len())
                    };
                    let _ = s.write_all(head.as_bytes()).await;
                    let _ = s.write_all(&rest[..rest.len().min(3000)]).await;
                    // and then the line goes dead
                });
            }
        });
        let http = super::HttpClient::new(&crate::config::AppConfig::default()).unwrap();
        let got = http.get_bytes(&format!("http://127.0.0.1:{port}/a.mp3"), super::Profile::Media, None).await.unwrap();
        assert_eq!(got.len(), body.len());
        assert!(got == body, "the pieces join into the same file");
    }
}
