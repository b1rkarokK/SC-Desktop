//! Second playback engine: SoundCloud's official embedded player.
//!
//! Some tracks (monetized ones, more and more) are served only as DRM-protected
//! streams (Widevine / PlayReady). We do not decrypt anything: such tracks are
//! played by SoundCloud's own widget, which the system WebView supports, in a
//! hidden window. The app drives it like the native engine (play, pause,
//! seek, volume) and gets position / end back.
//!
//! The window exists only while it is needed: it is closed `IDLE` after the
//! last widget track. Native tracks never touch it. The host page is
//! soundcloud.com/robots.txt (tiny, no site JS); the page reports events by
//! navigating to marker URLs that `on_navigation` catches and cancels. It has
//! no IPC access.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tauri::{webview::PageLoadEvent, AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::{mpsc::UnboundedSender, watch};
use url::Url;

use super::audio::{AudioCmd, AudioEvent};
use crate::error::{AppError, AppResult};

const LABEL: &str = "sc-widget";
const PAGE: &str = "https://soundcloud.com/robots.txt";
/// kept ready this long after the last widget track: the next one then starts in 1-2 s, not 6-10
const IDLE: Duration = Duration::from_secs(10 * 60);
/// no "play" from the widget within this time → the track can't be played
const START_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Default)]
struct St {
    generation: u64,
    active: bool,
    started: bool,
    playing: bool,
    /// position at `since`
    base_ms: u64,
    since: Option<Instant>,
    volume: f32,
    last_use: Option<Instant>,
    /// warmed up for the session: never closed for being idle
    keep: bool,
}

pub struct Widget {
    app: AppHandle,
    events: UnboundedSender<AudioEvent>,
    st: Arc<Mutex<St>>,
    ready: Arc<Mutex<Option<watch::Sender<bool>>>>,
}

fn host_script() -> String {
    format!(
        r#"window.scw = window.scw || (() => {{
  const PAGE = {page};
  const mark = (q) => {{ location.href = PAGE + '?scw=1&' + q; }};
  const api = new Promise((res) => {{
    const s = document.createElement('script');
    s.src = 'https://w.soundcloud.com/player/api.js';
    s.onload = res;
    s.onerror = res;
    (document.head || document.documentElement).append(s);
  }});
  // one widget per track; `pre` = the next track, loaded and paused in advance
  const make = (id) => {{
    const f = document.createElement('iframe');
    f.allow = 'autoplay; encrypted-media';
    f.src = 'https://w.soundcloud.com/player/?url=' + encodeURIComponent('https://api.soundcloud.com/tracks/' + id) + '&auto_play=false&visual=false';
    document.body.append(f);
    const w = SC.Widget(f);
    const o = {{ id, f, w }};
    o.ready = new Promise((r) => w.bind(SC.Widget.Events.READY, r));
    return o;
  }};
  let cur = null, pre = null, gen = -1;
  return {{
    async preload(id) {{
      await api;
      if (!window.SC || (pre && pre.id === id) || (cur && cur.id === id)) return;
      if (pre) pre.f.remove();
      pre = make(id);
    }},
    async load(id, g, vol, at) {{
      gen = g;
      await api;
      if (gen !== g) return;
      if (!window.SC) {{ mark('ev=error&g=' + g); return; }}
      if (cur) {{ cur.f.remove(); cur = null; }}
      let o;
      if (pre && pre.id === id) {{ o = pre; pre = null; }} else o = make(id);
      cur = o;
      const me = o.w, E = SC.Widget.Events;
      let last = 0;
      const mine = () => gen === g && cur === o;
      me.bind(E.PLAY, () => mine() && me.getPosition((p) => mark('ev=play&g=' + g + '&pos=' + Math.round(p))));
      me.bind(E.PAUSE, (e) => mine() && mark('ev=pause&g=' + g + '&pos=' + Math.round(e.currentPosition)));
      me.bind(E.FINISH, () => mine() && mark('ev=finish&g=' + g));
      me.bind(E.ERROR, () => mine() && mark('ev=error&g=' + g));
      me.bind(E.SEEK, (e) => mine() && mark('ev=seek&g=' + g + '&pos=' + Math.round(e.currentPosition)));
      // PLAY fires before the sound (buffering, DRM licence): only moving
      // progress means it really plays. First one at once, then every 5 s.
      let moving = false;
      me.bind(E.PLAY, () => {{ moving = false; }});
      me.bind(E.PAUSE, () => {{ moving = false; }});
      me.bind(E.PLAY_PROGRESS, (e) => {{
        if (!mine() || e.currentPosition <= 0) return;
        const now = Date.now();
        if (!moving || now - last > 5000) {{ moving = true; last = now; mark('ev=prog&g=' + g + '&pos=' + Math.round(e.currentPosition)); }}
      }});
      await o.ready;
      if (!mine()) return;
      me.setVolume(vol);
      if (at > 0) me.seekTo(at);
      me.play();
    }},
    play() {{ cur && cur.w.play(); }},
    pause() {{ cur && cur.w.pause(); }},
    seek(ms) {{ cur && cur.w.seekTo(ms); }},
    volume(v) {{ cur && cur.w.setVolume(v); }},
    stop() {{ gen = -1; if (cur) {{ cur.f.remove(); cur = null; }} }},
    // play a DRM track silently for a moment: loads the player and Windows'
    // DRM module, so the user's first protected track starts as fast as a repeat
    async warm(id) {{
      await api;
      if (!window.SC || cur) return;
      const o = make(id);
      await o.ready;
      o.w.setVolume(0);
      o.w.bind(SC.Widget.Events.PLAY_PROGRESS, (e) => {{
        if (e.currentPosition > 300) {{ o.w.pause(); setTimeout(() => o.f.remove(), 500); }}
      }});
      o.w.play();
      setTimeout(() => o.f.isConnected && o.f.remove(), 20000);
    }},
  }};
}})();"#,
        page = serde_json::to_string(PAGE).unwrap_or_default(),
    )
}

/// Slider position → widget volume 0..100 (same curve as the native engine).
fn widget_volume(v: f32) -> u32 {
    let v = v.clamp(0.0, 1.0);
    (v * v * 100.0).round() as u32
}

impl Widget {
    pub fn new(app: AppHandle, events: UnboundedSender<AudioEvent>, volume: f32) -> Self {
        Self {
            app,
            events,
            st: Arc::new(Mutex::new(St { volume, ..Default::default() })),
            ready: Arc::new(Mutex::new(None)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, St> {
        self.st.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn position_ms(&self) -> u64 {
        let s = self.lock();
        match (s.playing, s.since) {
            (true, Some(t)) => s.base_ms + t.elapsed().as_millis() as u64,
            _ => s.base_ms,
        }
    }

    fn eval(&self, js: &str) {
        if let Some(w) = self.app.get_webview_window(LABEL) {
            if let Err(e) = w.eval(&format!("{}\n{js}", host_script())) {
                tracing::warn!(error = %e, "widget eval failed");
            }
        }
    }

    /// Starts `track_id` in the widget (opens the hidden window if needed).
    pub fn load(&self, track_id: u64, generation: u64) {
        let volume = {
            let mut s = self.lock();
            *s = St { generation, active: true, volume: s.volume, keep: s.keep, last_use: Some(Instant::now()), ..Default::default() };
            s.volume
        };
        let app = self.app.clone();
        let st = self.st.clone();
        let ready = self.ready.clone();
        let events = self.events.clone();
        tauri::async_runtime::spawn(async move {
            let t0 = Instant::now();
            if let Err(e) = ensure_window(&app, &st, &ready, &events).await {
                let _ = events.send(AudioEvent::Error { generation, message: e.to_string() });
                return;
            }
            tracing::debug!(ms = t0.elapsed().as_millis() as u64, "widget window ready");
            st.lock().unwrap_or_else(|p| p.into_inner()).last_use = Some(Instant::now());
            if let Some(w) = app.get_webview_window(LABEL) {
                let js = format!("{}\nwindow.scw.load({track_id}, {generation}, {}, 0);", host_script(), widget_volume(volume));
                let _ = w.eval(&js);
            }
            tokio::time::sleep(START_TIMEOUT).await;
            let failed = {
                let s = st.lock().unwrap_or_else(|p| p.into_inner());
                s.generation == generation && s.active && !s.started
            };
            if failed {
                let _ = events.send(AudioEvent::Error { generation, message: "SoundCloud не отдал этот трек".into() });
            }
        });
    }

    /// Gets SoundCloud's player ready before the user needs it: the window, the
    /// player script and the DRM module are loaded by a silent half-second play
    /// of a protected track. The window then stays for the whole session.
    pub fn warm_up(&self, sample_track: u64) {
        self.lock().keep = true;
        let app = self.app.clone();
        let st = self.st.clone();
        let ready = self.ready.clone();
        let events = self.events.clone();
        tauri::async_runtime::spawn(async move {
            if ensure_window(&app, &st, &ready, &events).await.is_ok() {
                if let Some(w) = app.get_webview_window(LABEL) {
                    let _ = w.eval(&format!("{}\nwindow.scw.warm({sample_track});", host_script()));
                    tracing::debug!(track = sample_track, "widget: warmed up");
                }
            }
        });
    }

    /// The warm-up is switched off: the window goes away once idle, as before.
    pub fn cool_down(&self) {
        let mut s = self.lock();
        s.keep = false;
        s.last_use = Some(Instant::now() - IDLE);
    }

    /// Loads `track_id` paused in advance (the next track is DRM-only too),
    /// so switching to it is instant.
    pub fn preload(&self, track_id: u64) {
        self.lock().last_use = Some(Instant::now());
        let app = self.app.clone();
        let st = self.st.clone();
        let ready = self.ready.clone();
        let events = self.events.clone();
        tauri::async_runtime::spawn(async move {
            if ensure_window(&app, &st, &ready, &events).await.is_ok() {
                if let Some(w) = app.get_webview_window(LABEL) {
                    let _ = w.eval(&format!("{}\nwindow.scw.preload({track_id});", host_script()));
                    tracing::debug!(track = track_id, "widget: next track preloaded");
                }
            }
        });
    }

    pub fn send(&self, cmd: AudioCmd) {
        match cmd {
            AudioCmd::Play => self.eval("window.scw.play();"),
            AudioCmd::Pause => {
                let mut s = self.lock();
                if s.playing {
                    s.base_ms += s.since.map_or(0, |t| t.elapsed().as_millis() as u64);
                    s.playing = false;
                }
                drop(s);
                self.eval("window.scw.pause();");
            }
            AudioCmd::Seek(to) => {
                let ms = to.as_millis() as u64;
                {
                    let mut s = self.lock();
                    s.base_ms = ms;
                    s.since = Some(Instant::now());
                }
                self.eval(&format!("window.scw.seek({ms});"));
            }
            AudioCmd::Replay { generation } => {
                if self.lock().generation == generation {
                    {
                        let mut s = self.lock();
                        s.base_ms = 0;
                        s.since = Some(Instant::now());
                    }
                    self.eval("window.scw.seek(0); window.scw.play();");
                }
            }
            AudioCmd::Volume(v) => {
                self.lock().volume = v;
                self.eval(&format!("window.scw.volume({});", widget_volume(v)));
            }
            AudioCmd::Stop => self.stop(),
            AudioCmd::Load { .. } => {}
        }
    }

    pub fn stop(&self) {
        let was_active = {
            let mut s = self.lock();
            let a = s.active;
            s.active = false;
            s.playing = false;
            s.base_ms = 0;
            s.last_use = Some(Instant::now());
            a
        };
        if was_active {
            self.eval("window.scw.stop();");
        }
    }
}

async fn ensure_window(
    app: &AppHandle,
    st: &Arc<Mutex<St>>,
    ready: &Arc<Mutex<Option<watch::Sender<bool>>>>,
    events: &UnboundedSender<AudioEvent>,
) -> AppResult<()> {
    let existing = app.get_webview_window(LABEL);
    let mut rx = {
        let mut r = ready.lock().unwrap_or_else(|p| p.into_inner());
        match (&existing, r.as_ref()) {
            (Some(_), Some(tx)) if *tx.borrow() => return Ok(()),
            (Some(_), Some(tx)) => tx.subscribe(),
            _ => {
                let (tx, rx) = watch::channel(false);
                *r = Some(tx);
                rx
            }
        }
    };
    if existing.is_none() {
        let st_nav = st.clone();
        let ev_nav = events.clone();
        let ready_load = ready.clone();
        WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(Url::parse(PAGE)?))
            .title("SoundCloud")
            .visible(false)
            .skip_taskbar(true)
            .inner_size(400.0, 200.0)
            .on_navigation(move |url| {
                if url.query_pairs().any(|(k, _)| k == "scw") {
                    on_event(&st_nav, &ev_nav, url);
                    false
                } else {
                    true
                }
            })
            .on_page_load(move |_, p| {
                if p.event() == PageLoadEvent::Finished {
                    if let Some(tx) = ready_load.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
                        let _ = tx.send(true);
                    }
                }
            })
            .build()?;
        spawn_idle_closer(app.clone(), st.clone(), ready.clone());
    }
    tokio::time::timeout(Duration::from_secs(20), rx.wait_for(|r| *r))
        .await
        .map_err(|_| AppError::Other("SoundCloud не отвечает".into()))?
        .map_err(|_| AppError::Other("SoundCloud не отвечает".into()))?;
    Ok(())
}

fn on_event(st: &Arc<Mutex<St>>, events: &UnboundedSender<AudioEvent>, url: &Url) {
    let (mut ev, mut g, mut pos) = (String::new(), None::<u64>, None::<u64>);
    for (k, v) in url.query_pairs() {
        match &*k {
            "ev" => ev = v.into_owned(),
            "g" => g = v.parse().ok(),
            "pos" => pos = v.parse().ok(),
            _ => {}
        }
    }
    let Some(g) = g else { return };
    let mut s = st.lock().unwrap_or_else(|p| p.into_inner());
    if s.generation != g || !s.active {
        return;
    }
    let since_load = s.last_use;
    s.last_use = Some(Instant::now());
    match ev.as_str() {
        // "play" only means the widget wants to play; the clock starts with "prog"
        "play" => {}
        "prog" => {
            let p = pos.unwrap_or(s.base_ms);
            if !s.playing {
                s.playing = true;
                s.base_ms = p;
                s.since = Some(Instant::now());
            } else if p.abs_diff(s.base_ms + s.since.map_or(0, |t| t.elapsed().as_millis() as u64)) > 300 {
                s.base_ms = p;
                s.since = Some(Instant::now());
            }
            if !s.started {
                s.started = true;
                tracing::debug!(ms = since_load.map_or(0, |t| t.elapsed().as_millis() as u64), "widget sound started");
                drop(s);
                let _ = events.send(AudioEvent::Started { generation: g });
                return;
            }
        }
        "seek" => {
            if let Some(p) = pos {
                s.base_ms = p;
                s.since = Some(Instant::now());
            }
        }
        "pause" => {
            s.playing = false;
            s.base_ms = pos.unwrap_or(s.base_ms);
        }
        "finish" => {
            s.playing = false;
            drop(s);
            let _ = events.send(AudioEvent::Ended { generation: g });
        }
        "error" => {
            s.active = false;
            drop(s);
            let _ = events.send(AudioEvent::Error { generation: g, message: "SoundCloud не отдал этот трек".into() });
        }
        _ => {}
    }
}

fn spawn_idle_closer(app: AppHandle, st: Arc<Mutex<St>>, ready: Arc<Mutex<Option<watch::Sender<bool>>>>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            let Some(w) = app.get_webview_window(LABEL) else {
                *ready.lock().unwrap_or_else(|p| p.into_inner()) = None;
                return;
            };
            let idle = {
                let s = st.lock().unwrap_or_else(|p| p.into_inner());
                !s.keep && !s.active && s.last_use.map_or(true, |t| t.elapsed() > IDLE)
            };
            if idle {
                *ready.lock().unwrap_or_else(|p| p.into_inner()) = None;
                let _ = w.destroy();
                tracing::debug!("widget window closed");
                return;
            }
        }
    });
}
