//! Write requests (like, follow) sent from a real browser page.
//!
//! SoundCloud's anti-bot (DataDome) answers PUT/POST/DELETE from a non-browser
//! TLS stack with a captcha, cookies or not. So these few requests go through a
//! hidden WebView sitting on soundcloud.com/robots.txt (tiny, no site JS):
//! same origin, same cookies and TLS as the site itself.
//!
//! If DataDome still wants a captcha, the page embeds it in an iframe exactly
//! like soundcloud.com does and the window is shown. Once solved, the captcha
//! posts the new `datadome` cookie to the page, the page stores it, the window
//! hides and the request is repeated.
//!
//! The window exists only while needed: it is closed after `IDLE` without
//! requests. The page talks to Rust by navigating to marker URLs, which
//! `on_navigation` catches and cancels. It has no IPC access.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

use tauri::{webview::PageLoadEvent, AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::sync::{oneshot, watch};
use url::Url;

use crate::error::{AppError, AppResult};

const LABEL: &str = "sc-bridge";
const PAGE: &str = "https://soundcloud.com/robots.txt";
const MARKER: &str = "scdesk";
const IDLE: Duration = Duration::from_secs(30);
const TIMEOUT: Duration = Duration::from_secs(20);
/// Status reported when DataDome blocks outright (no captcha to solve).
pub const BLOCKED: u16 = 1;
const CAPTCHA_TIMEOUT: Duration = Duration::from_secs(5 * 60);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static PENDING: Mutex<Option<HashMap<u64, oneshot::Sender<u16>>>> = Mutex::new(None);
static LAST_USE: Mutex<Option<Instant>> = Mutex::new(None);
static READY: Mutex<Option<watch::Sender<bool>>> = Mutex::new(None);
/// The captcha is on screen: no idle close, longer wait.
static IN_CAPTCHA: AtomicBool = AtomicBool::new(false);

fn no_answer() -> AppError {
    AppError::Other("SoundCloud не отвечает".into())
}

enum Marker {
    /// final HTTP status of request `id`
    Done(u64, u16),
    /// captcha shown (true) / solved and hidden (false)
    Captcha(bool),
}

fn marker_of(url: &Url) -> Option<Marker> {
    let (mut id, mut status, mut captcha) = (None, None, None);
    for (k, v) in url.query_pairs() {
        match &*k {
            MARKER => id = v.parse().ok(),
            "status" => status = v.parse().ok(),
            "captcha" => captcha = Some(v == "1"),
            _ => {}
        }
    }
    match (id, status, captcha) {
        (_, _, Some(show)) => Some(Marker::Captcha(show)),
        (Some(id), Some(s), None) => Some(Marker::Done(id, s)),
        _ => None,
    }
}

fn on_marker(app: &AppHandle, m: Marker) {
    match m {
        Marker::Done(id, status) => {
            if let Some(tx) = PENDING.lock().unwrap().as_mut().and_then(|p| p.remove(&id)) {
                let _ = tx.send(status);
            }
        }
        Marker::Captcha(show) => {
            IN_CAPTCHA.store(show, Ordering::Relaxed);
            *LAST_USE.lock().unwrap() = Some(Instant::now());
            let Some(w) = app.get_webview_window(LABEL) else { return };
            if show {
                tracing::info!("bridge: captcha shown");
                let _ = w.set_skip_taskbar(false);
                let _ = w.center();
                let _ = w.show();
                let _ = w.set_focus();
            } else {
                tracing::info!("bridge: captcha solved");
                let _ = w.hide();
                let _ = w.set_skip_taskbar(true);
            }
        }
    }
}

async fn window(app: &AppHandle) -> AppResult<WebviewWindow> {
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
        let handle = app.clone();
        WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(Url::parse(PAGE)?))
            .title("Проверка SoundCloud")
            .visible(false)
            .skip_taskbar(true)
            .inner_size(440.0, 620.0)
            .center()
            .background_color(tauri::window::Color(18, 18, 18, 255))
            .on_navigation(move |url| match marker_of(url) {
                Some(m) => {
                    on_marker(&handle, m);
                    false
                }
                None => true,
            })
            .on_page_load(|_, p| {
                if p.event() == PageLoadEvent::Finished {
                    if let Some(tx) = READY.lock().unwrap().as_ref() {
                        let _ = tx.send(true);
                    }
                }
            })
            .build()?;
        spawn_idle_closer(app.clone());
    }
    tokio::time::timeout(TIMEOUT, rx.wait_for(|r| *r)).await.map_err(|_| no_answer())?.map_err(|_| no_answer())?;
    app.get_webview_window(LABEL).ok_or_else(no_answer)
}

fn spawn_idle_closer(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let Some(w) = app.get_webview_window(LABEL) else {
                *READY.lock().unwrap() = None;
                IN_CAPTCHA.store(false, Ordering::Relaxed);
                return;
            };
            let idle = LAST_USE.lock().unwrap().map_or(true, |t| t.elapsed() > IDLE);
            let busy = IN_CAPTCHA.load(Ordering::Relaxed) || PENDING.lock().unwrap().as_ref().is_some_and(|m| !m.is_empty());
            if idle && !busy {
                *READY.lock().unwrap() = None;
                let _ = w.destroy();
                tracing::debug!("bridge window closed");
                return;
            }
        }
    });
}

/// SoundCloud's own DataDome tag setup (from soundcloud.com's HTML). With
/// `sessionByHeader` the tag adds a session header to these write requests:
/// without it every like looks like a bot and is blocked outright.
const DD_KEY: &str = "7FC6D561817844F25B65CDD97F28A1";
const DD_TAG: &str = "https://dwt.soundcloud.com/tags.js";
const DD_OPTIONS: &str = r#"{
  ajaxListenerPath: [
    {"host":"api-v2.soundcloud.com","path":"/me/followings/*","strict":true},
    {"host":"api-v2.soundcloud.com","path":"/users/*/track_likes/*","strict":true},
    {"host":"api-v2.soundcloud.com","path":"/users/*/playlist_likes/*","strict":true},
    {"host":"api-v2.soundcloud.com","path":"/me/track_reposts/*","strict":true},
    {"host":"api-v2.soundcloud.com","path":"/me","strict":true}
  ],
  overrideAbortFetch: true,
  sessionByHeader: true,
  cookieName: 'datadome',
  endpoint: 'https://dwt.soundcloud.com/js/',
  disableAutoRefreshOnCaptchaPassed: true,
  enableTagEvents: true,
  abortAsyncOnCaptchaDisplay: false
}"#;

/// In-page script: load the DataDome tag once, send the request; on a captcha
/// show the window until it is passed (the tag's own captcha, or ours in an
/// iframe if the tag did not show one), then send once more.
fn script(id: u64, method: &str, url: &Url, auth: &str) -> AppResult<String> {
    Ok(format!(
        r#"(async () => {{
  const page = {page};
  const BLOCKED = {BLOCKED};
  const mark = (q) => {{ location.href = page + '?' + q; }};
  if (!window.__scdDD) {{
    window.__scdDD = new Promise((ready) => {{
      window.ddjskey = {key};
      window.ddoptions = {options};
      const st = window.__scdState = {{ shown: false, blocked: false, passed: null }};
      const shown = () => setTimeout(() => {{ if (!st.blocked && !st.shown) {{ st.shown = true; mark('{MARKER}=0&captcha=1'); }} }}, 400);
      const passed = () => {{ if (st.shown) mark('{MARKER}=0&captcha=0'); st.shown = false; st.passed && st.passed(); }};
      for (const n of ['dd_captcha_displayed', 'dd_response_displayed']) addEventListener(n, shown);
      for (const n of ['dd_captcha_passed', 'dd_response_passed']) addEventListener(n, passed);
      const s = document.createElement('script');
      s.src = {tag};
      s.async = true;
      s.onload = () => setTimeout(ready, 1500);
      s.onerror = () => ready();
      (document.head || document.documentElement).append(s);
      setTimeout(ready, 8000);
    }});
  }}
  await window.__scdDD;
  const st = window.__scdState;
  st.blocked = false;
  const opts = {{ method: {method}, headers: {{ Authorization: {auth} }}, credentials: 'include' }};
  const send = async () => {{
    try {{ return await fetch({url}, opts); }}
    catch (_) {{ return await fetch({url}, {{ ...opts, credentials: 'omit' }}); }}
  }};
  const iframeCaptcha = (src) => new Promise((done) => {{
    document.documentElement.style.background = '#fff';
    const f = document.createElement('iframe');
    f.src = src;
    f.style.cssText = 'position:fixed;inset:0;width:100%;height:100%;border:0;background:#fff;z-index:2147483647';
    const onMsg = (e) => {{
      if (!/captcha-delivery\.com$/.test(new URL(e.origin).hostname)) return;
      let d = e.data;
      try {{ if (typeof d === 'string') d = JSON.parse(d); }} catch (_) {{ return; }}
      if (!d || typeof d.cookie !== 'string' || !d.cookie.includes('datadome=')) return;
      document.cookie = d.cookie;
      removeEventListener('message', onMsg);
      f.remove();
      st.shown = false;
      mark('{MARKER}=0&captcha=0');
      done();
    }};
    addEventListener('message', onMsg);
    document.body.append(f);
    st.shown = true;
    mark('{MARKER}=0&captcha=1');
  }});
  const solved = (src) => new Promise((done) => {{
    st.passed = () => {{ st.passed = null; done(); }};
    // the tag shows its captcha by itself; if it did not, show ours
    setTimeout(() => {{ if (!st.shown) {{ st.passed = null; iframeCaptcha(src).then(done); }} }}, 2500);
  }});
  let status = 0;
  try {{
    let r = await send();
    if (r.status === 403) {{
      let src = '';
      try {{ src = (await r.clone().json()).url || ''; }} catch (_) {{}}
      // t=bv: a hard block, not a puzzle: nothing to show the user
      if (/[?&]t=bv(&|$)/.test(src)) {{
        st.blocked = true;
        document.querySelectorAll('iframe').forEach((f) => f.remove());
        mark('{MARKER}={id}&status=' + BLOCKED);
        return;
      }}
      if (src.startsWith('https://')) {{ await solved(src); r = await send(); }}
    }}
    status = r.status;
  }} catch (_) {{}}
  mark('{MARKER}={id}&status=' + status);
}})();"#,
        page = serde_json::to_string(PAGE)?,
        key = serde_json::to_string(DD_KEY)?,
        tag = serde_json::to_string(DD_TAG)?,
        options = DD_OPTIONS,
        method = serde_json::to_string(method)?,
        auth = serde_json::to_string(auth)?,
        url = serde_json::to_string(url.as_str())?,
    ))
}

/// Sends `method url` with the OAuth header from soundcloud.com; returns the HTTP status.
/// The login token of the SoundCloud session in the app's browser profile
/// (the sign-in window put it there and the site keeps renewing it).
pub async fn session_token(app: &AppHandle) -> Option<String> {
    let w = window(app).await.ok()?;
    let url = Url::parse("https://soundcloud.com/").ok()?;
    w.cookies_for_url(url).ok()?.into_iter().find(|c| c.name() == "oauth_token" && !c.value().is_empty()).map(|c| c.value().to_owned())
}

pub async fn send(app: &AppHandle, method: &str, url: &Url, auth: &str) -> AppResult<u16> {
    let status = send_once(app, method, url, auth).await?;
    if status != BLOCKED {
        return Ok(status);
    }
    // A hard block sticks to the DataDome session, not to the user: a session
    // flagged once stays flagged. Start a fresh one and try again.
    tracing::info!("bridge: DataDome session blocked, starting a new one");
    reset_session(app).await?;
    send_once(app, method, url, auth).await
}

async fn reset_session(app: &AppHandle) -> AppResult<()> {
    let w = window(app).await?;
    w.eval(
        "for (const d of ['.soundcloud.com', 'soundcloud.com', ''])            document.cookie = 'datadome=; Max-Age=0; Path=/' + (d ? '; Domain=' + d : '');          localStorage.removeItem('ddSession_datadome');",
    )?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut rx = {
        let ready = READY.lock().unwrap();
        let tx = ready.as_ref().ok_or_else(no_answer)?;
        let _ = tx.send(false);
        tx.subscribe()
    };
    w.navigate(Url::parse(PAGE)?)?;
    tokio::time::timeout(TIMEOUT, rx.wait_for(|r| *r)).await.map_err(|_| no_answer())?.map_err(|_| no_answer())?;
    Ok(())
}

async fn send_once(app: &AppHandle, method: &str, url: &Url, auth: &str) -> AppResult<u16> {
    *LAST_USE.lock().unwrap() = Some(Instant::now());
    let w = window(app).await?;
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (tx, mut rx) = oneshot::channel();
    PENDING.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, tx);
    w.eval(&script(id, method, url, auth)?)?;

    let started = Instant::now();
    let mut saw_captcha = false;
    let result = loop {
        match tokio::time::timeout(Duration::from_millis(500), &mut rx).await {
            Ok(Ok(status)) => break Ok(status),
            Ok(Err(_)) => break Err(no_answer()),
            Err(_) => {}
        }
        if app.get_webview_window(LABEL).is_none() {
            break Err(AppError::Other("Проверка SoundCloud не пройдена".into())); // closed by the user
        }
        saw_captcha |= IN_CAPTCHA.load(Ordering::Relaxed);
        let limit = if saw_captcha { CAPTCHA_TIMEOUT } else { TIMEOUT };
        if started.elapsed() > limit {
            break Err(no_answer());
        }
    };
    PENDING.lock().unwrap().as_mut().map(|m| m.remove(&id));
    if IN_CAPTCHA.swap(false, Ordering::Relaxed) {
        // gave up while the captcha was on screen
        if let Some(w) = app.get_webview_window(LABEL) {
            let _ = w.destroy();
        }
        *READY.lock().unwrap() = None;
    }
    *LAST_USE.lock().unwrap() = Some(Instant::now());
    let status = result?;
    tracing::debug!(%method, status, "bridge request");
    Ok(status)
}
