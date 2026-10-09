//! Genius through a real browser page.
//!
//! genius.com sits behind a Cloudflare human check: plain HTTP from the app
//! gets 403. So, only when the other lyrics sources found nothing, requests go
//! through a hidden WebView on genius.com (same origin, same cookies as a
//! browser). If Cloudflare shows its check, the window is shown; once the user
//! passes it, the window hides and lyrics load on their own until Cloudflare
//! asks again (hours or days later).
//!
//! The window exists only while needed (closed after `IDLE`). The page talks
//! back by navigating to marker URLs that `on_navigation` catches and cancels.
//! It has no IPC access. Lyrics are extracted in the page, so only the text
//! travels back.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use serde_json::Value;
use tauri::{webview::PageLoadEvent, AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::sync::{oneshot, watch};
use url::Url;

use crate::{
    api::genius::{self, GeniusHit},
    error::{AppError, AppResult},
};

const LABEL: &str = "genius-web";
const PAGE: &str = "https://genius.com/robots.txt";
const MARKER: &str = "scgw";
const IDLE: Duration = Duration::from_secs(30);
const TIMEOUT: Duration = Duration::from_secs(20);
/// how long the user has to pass the check
const CHECK_TIMEOUT: Duration = Duration::from_secs(3 * 60);
/// Cloudflare often passes by itself in a few seconds: show the window only after that
const SHOW_AFTER: Duration = Duration::from_secs(4);
/// closed the check window without passing: don't ask again for a while
const DECLINE_PAUSE: Duration = Duration::from_secs(30 * 60);

static APP: OnceLock<AppHandle> = OnceLock::new();
/// plain HTTP got 403 this session: go straight to the browser page
static BLOCKED: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static PENDING: Mutex<Option<HashMap<u64, oneshot::Sender<(u16, String)>>>> = Mutex::new(None);
/// None = no window; Some(false) = loading / Cloudflare check; Some(true) = usable
static READY: Mutex<Option<watch::Sender<bool>>> = Mutex::new(None);
static IN_CHECK: AtomicBool = AtomicBool::new(false);
static LAST_USE: Mutex<Option<Instant>> = Mutex::new(None);
static DECLINED_AT: Mutex<Option<Instant>> = Mutex::new(None);

pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

/// Genius search: plain HTTP while it works, the browser page once it is blocked.
pub async fn search(http: &crate::net::HttpClient, artist: &str, title: &str) -> AppResult<Option<GeniusHit>> {
    if !BLOCKED.load(Ordering::Relaxed) {
        match genius::search(http, artist, title).await {
            Err(AppError::Forbidden) => BLOCKED.store(true, Ordering::Relaxed),
            other => return other,
        }
    }
    let url = genius::search_url(artist, title)?;
    let body = fetch(url.as_str(), "search").await?;
    let resp: Value = serde_json::from_str(&body).map_err(|e| AppError::Parse(e.to_string()))?;
    Ok(genius::pick(&resp, artist, title))
}

pub async fn lyrics(http: &crate::net::HttpClient, page_url: &str) -> AppResult<String> {
    if !BLOCKED.load(Ordering::Relaxed) {
        match genius::lyrics(http, page_url).await {
            Err(AppError::Forbidden) => BLOCKED.store(true, Ordering::Relaxed),
            other => return other,
        }
    }
    let url = Url::parse(page_url)?;
    if url.scheme() != "https" || url.host_str() != Some("genius.com") {
        return Err(AppError::Parse("unexpected lyrics host".into()));
    }
    Ok(genius::tidy(&fetch(url.as_str(), "lyrics").await?))
}

fn app() -> AppResult<&'static AppHandle> {
    APP.get().ok_or_else(|| AppError::Other("Genius: окно не готово".into()))
}

fn no_answer() -> AppError {
    AppError::Other("Genius не отвечает".into())
}

fn set_ready(v: bool) {
    if let Some(tx) = READY.lock().unwrap().as_ref() {
        let _ = tx.send(v);
    }
}

/// Runs after every page load. On the real page it reports "ok"; on
/// Cloudflare's check it must stay silent: any navigation from here would
/// cancel the check's own redirect after the user passes it.
const CHECK_JS: &str = r#"(() => {
  const t = (document.title || '') + ' ' + (document.body ? document.body.innerText.slice(0, 400) : '');
  const check = /just a moment|один момент|attention required|make sure you.re a human|verify you are human/i.test(t)
    || !!document.querySelector('#challenge-form, .cf-turnstile, #cf-challenge-running')
    || /__cf_chl/.test(location.href);
  if (!check) location.href = 'https://genius.com/robots.txt?scgw=0&page=ok';
})();"#;

fn on_marker(url: &Url) {
    let (mut id, mut status, mut data, mut page) = (None::<u64>, None::<u16>, String::new(), None::<String>);
    for (k, v) in url.query_pairs() {
        match &*k {
            MARKER => id = v.parse().ok(),
            "status" => status = v.parse().ok(),
            "data" => data = v.into_owned(),
            "page" => page = Some(v.into_owned()),
            _ => {}
        }
    }
    if page.is_some() {
        on_page_ok();
        return;
    }
    if let (Some(id), Some(status)) = (id, status) {
        if let Some(tx) = PENDING.lock().unwrap().as_mut().and_then(|m| m.remove(&id)) {
            let _ = tx.send((status, data));
        }
    }
}

fn on_page_ok() {
    if IN_CHECK.swap(false, Ordering::Relaxed) {
        tracing::info!("genius: human check passed");
        if let Some(w) = APP.get().and_then(|a| a.get_webview_window(LABEL)) {
            let _ = w.hide();
            let _ = w.set_skip_taskbar(true);
        }
    }
    set_ready(true);
}

/// After (re)loading the page: no "ok" within `SHOW_AFTER` means Cloudflare
/// shows its check and did not pass it by itself: show the window to the user.
fn arm_check_timer() {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SHOW_AFTER).await;
        let ready = READY.lock().unwrap().as_ref().is_some_and(|tx| *tx.borrow());
        if ready || IN_CHECK.swap(true, Ordering::Relaxed) {
            return;
        }
        tracing::info!("genius: Cloudflare check shown");
        if let Some(w) = APP.get().and_then(|a| a.get_webview_window(LABEL)) {
            let _ = w.set_skip_taskbar(false);
            let _ = w.center();
            let _ = w.show();
            let _ = w.set_focus();
        }
    });
}

/// The window, ready for requests (after the user passed the check, if any).
async fn window() -> AppResult<WebviewWindow> {
    let app = app()?;
    if DECLINED_AT.lock().unwrap().is_some_and(|t| t.elapsed() < DECLINE_PAUSE) {
        return Err(AppError::Other("Проверка Genius пропущена".into()));
    }
    let existing = app.get_webview_window(LABEL);
    let mut rx = {
        let mut ready = READY.lock().unwrap();
        match (&existing, ready.as_ref()) {
            (Some(w), Some(tx)) if *tx.borrow() => return Ok(w.clone()),
            (Some(_), Some(tx)) => tx.subscribe(),
            _ => {
                let (tx, rx) = watch::channel(false);
                *ready = Some(tx);
                rx
            }
        }
    };
    if existing.is_none() {
        WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(Url::parse(PAGE)?))
            .title("Проверка Genius")
            .visible(false)
            .skip_taskbar(true)
            .inner_size(480.0, 640.0)
            .center()
            .background_color(tauri::window::Color(18, 18, 18, 255))
            .on_navigation(|url| {
                if url.query_pairs().any(|(k, _)| k == MARKER) {
                    on_marker(url);
                    false
                } else {
                    true
                }
            })
            .on_page_load(|w, p| {
                if p.event() == PageLoadEvent::Finished && p.url().host_str() == Some("genius.com") {
                    let _ = w.eval(CHECK_JS);
                }
            })
            .build()?;
        spawn_idle_closer(app.clone());
        arm_check_timer();
    }
    // wait for a usable page; while the check is on screen, give the user time
    let started = Instant::now();
    loop {
        if *rx.borrow() {
            break;
        }
        let limit = if IN_CHECK.load(Ordering::Relaxed) { CHECK_TIMEOUT } else { TIMEOUT };
        if started.elapsed() > limit {
            return Err(no_answer());
        }
        if app.get_webview_window(LABEL).is_none() {
            // closed by the user without passing
            *DECLINED_AT.lock().unwrap() = Some(Instant::now());
            IN_CHECK.store(false, Ordering::Relaxed);
            *READY.lock().unwrap() = None;
            return Err(AppError::Other("Проверка Genius пропущена".into()));
        }
        let _ = tokio::time::timeout(Duration::from_millis(500), rx.changed()).await;
    }
    app.get_webview_window(LABEL).ok_or_else(no_answer)
}

fn spawn_idle_closer(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let Some(w) = app.get_webview_window(LABEL) else {
                *READY.lock().unwrap() = None;
                return;
            };
            let idle = LAST_USE.lock().unwrap().map_or(true, |t| t.elapsed() > IDLE);
            let busy = IN_CHECK.load(Ordering::Relaxed) || PENDING.lock().unwrap().as_ref().is_some_and(|m| !m.is_empty());
            if idle && !busy {
                *READY.lock().unwrap() = None;
                let _ = w.destroy();
                tracing::debug!("genius window closed");
                return;
            }
        }
    });
}

/// In-page request. `kind`: "search" returns compact JSON of song hits,
/// "lyrics" returns the lyrics text extracted from the page.
fn script(id: u64, url: &str, kind: &str) -> AppResult<String> {
    Ok(format!(
        r#"(async () => {{
  const mark = (status, data) => {{
    location.href = 'https://genius.com/robots.txt?{MARKER}={id}&status=' + status + '&data=' + encodeURIComponent(data || '');
  }};
  try {{
    const r = await fetch({url}, {{ credentials: 'include', headers: {{ accept: {accept} }} }});
    if (!r.ok) {{ mark(r.status); return; }}
    if ({kind} === 'search') {{
      const j = await r.json();
      const hits = [];
      for (const s of (j.response && j.response.sections) || [])
        for (const h of s.hits || [])
          if (h.type === 'song' && h.result)
            hits.push({{ type: 'song', result: {{ url: h.result.url, title: h.result.title, primary_artist: {{ name: (h.result.primary_artist || {{}}).name || '' }} }} }});
      mark(200, JSON.stringify({{ response: {{ sections: [{{ hits }}] }} }}));
      return;
    }}
    const doc = new DOMParser().parseFromString(await r.text(), 'text/html');
    let out = '';
    const walk = (el) => {{
      for (const n of el.childNodes) {{
        if (n.nodeType === 3) out += n.nodeValue;
        else if (n.nodeType === 1) {{
          if (n.tagName === 'BR') {{ out += '\n'; continue; }}
          const c = n.getAttribute('class') || '';
          if (n.hasAttribute('data-exclude-from-selection') || c.includes('LyricsHeader') || c.includes('SongBioPreview')) continue;
          walk(n);
        }}
      }}
    }};
    for (const c of doc.querySelectorAll('div[data-lyrics-container="true"]')) {{
      if (out && !out.endsWith('\n')) out += '\n';
      walk(c);
    }}
    mark(200, out);
  }} catch (_) {{ mark(0); }}
}})();"#,
        url = serde_json::to_string(url)?,
        kind = serde_json::to_string(kind)?,
        accept = serde_json::to_string(if kind == "search" { "application/json" } else { "text/html" })?,
    ))
}

async fn fetch_once(url: &str, kind: &str) -> AppResult<(u16, String)> {
    *LAST_USE.lock().unwrap() = Some(Instant::now());
    let w = window().await?;
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    PENDING.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, tx);
    w.eval(&script(id, url, kind)?)?;
    let res = tokio::time::timeout(TIMEOUT, rx).await;
    PENDING.lock().unwrap().as_mut().map(|m| m.remove(&id));
    *LAST_USE.lock().unwrap() = Some(Instant::now());
    match res {
        Ok(Ok(r)) => Ok(r),
        _ => Err(no_answer()),
    }
}

async fn fetch(url: &str, kind: &str) -> AppResult<String> {
    let (status, data) = fetch_once(url, kind).await?;
    let (status, data) = if status == 403 {
        // Cloudflare wants the check again: reload the page (shows it) and retry
        let app = app()?;
        if let Some(w) = app.get_webview_window(LABEL) {
            set_ready(false);
            w.navigate(Url::parse(PAGE)?)?;
            arm_check_timer();
        }
        fetch_once(url, kind).await?
    } else {
        (status, data)
    };
    match status {
        200 => Ok(data),
        404 => Err(AppError::NotFound),
        403 => Err(AppError::Forbidden),
        0 => Err(no_answer()),
        s => Err(AppError::Http(s)),
    }
}
