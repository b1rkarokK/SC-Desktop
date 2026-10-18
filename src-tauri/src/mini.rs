//! Mini player (C2): a small always-on-top capsule with the karaoke line.
//! While it is open the main window is destroyed (tray mode), so the app
//! costs one small WebView instead of the big one.
//!
//! The window is transparent and undecorated (the rounded capsule is drawn by
//! the page), grows downward on hover for the controls and the queue
//! (`mini_resize`), and remembers where it was dragged (kv "mini:pos").
//!
//! Games: while a full-screen app is in front (a game, a full-screen video)
//! the capsule stays visible but lets every click through, so the mouse in
//! the game never catches it. With an ordinary window, the Start menu or the
//! desktop in front it is clickable again. It is clickable too:
//! * while the mouse pointer is the plain Windows arrow (a pause menu, a console or a
//!   settings screen of a game that shows the system pointer; a game's own picture of a
//!   pointer, like the one in Dota in play, is never mistaken for it);
//! * after the hotkey «Мини-плеер: нажимать в игре» (Ctrl+Alt+K by default) for the time set in
//!   Settings (20 seconds by default) or until the hotkey is pressed again; pressing it during that time
//!   switches it back at once.
//! * while the pointer rests on the capsule for a third of a second (the user reaches for it; a quick pass of
//!   the mouse in the game is not caught).
//! A few cheap system calls a second.
//!
//! Pinned (the default): on top of every window. Unpinned: an ordinary window
//! that other windows can cover.

use tauri::{AppHandle, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::state::AppState;

pub const LABEL: &str = "mini";
/// wide enough for the whole control row with like + pin and the volume slider
const WIDTH: f64 = 380.0;
const HEIGHT: f64 = 58.0;
const POS_KEY: &str = "mini:pos";
const PIN_KEY: &str = "mini:pinned";
/// the pin as the watcher sees it (no database read three times a second)
static PIN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
/// clickable on demand until this moment (ms since the Unix epoch), 0 = not
static CLICKABLE_UNTIL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// how long it stays clickable when the setting is missing
const CLICKABLE_DEFAULT_SECS: u64 = 20;

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The hotkey: the capsule takes clicks for a while (over a game), or goes back to letting them through.
pub fn toggle_interactive(app: &AppHandle) {
    use std::sync::atomic::Ordering::Relaxed;
    let on = CLICKABLE_UNTIL.load(Relaxed) <= now_ms();
    // the setting: seconds, or 0 = until the hotkey is pressed again
    let secs = app.state::<AppState>().config.get().ui["hotkeys"]["miniClickSecs"].as_u64().unwrap_or(CLICKABLE_DEFAULT_SECS);
    let until = if secs == 0 { u64::MAX / 2 } else { now_ms() + secs * 1000 };
    CLICKABLE_UNTIL.store(if on { until } else { 0 }, Relaxed);
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.set_ignore_cursor_events(!on && fullscreen_in_front(w.hwnd().map(|h| h.0 as isize).unwrap_or(0)));
    }
}

/// Pinned on top of everything (the default).
pub async fn pinned(app: &AppHandle) -> bool {
    app.state::<AppState>().db.kv_get::<bool>(PIN_KEY).await.ok().flatten().map(|(v, _)| v).unwrap_or(true)
}

pub async fn set_pinned(app: &AppHandle, on: bool) {
    PIN.store(on, std::sync::atomic::Ordering::Relaxed);
    let _ = app.state::<AppState>().db.kv_put(PIN_KEY, &on).await;
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.set_always_on_top(on);
        if !on {
            let _ = w.set_ignore_cursor_events(false);
        }
    }
}

pub async fn open(app: &AppHandle) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.show();
        let _ = w.set_focus();
        crate::tray::hide_main(app);
        return Ok(());
    }
    let saved = app.state::<AppState>().db.kv_get::<(i32, i32)>(POS_KEY).await.ok().flatten().map(|(p, _)| p);
    let handle = app.clone();
    let pin = pinned(app).await;
    PIN.store(pin, std::sync::atomic::Ordering::Relaxed);
    let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html?mini=1".into()))
        .additional_browser_args(crate::net::bypass::browser_args()).data_directory(crate::portable::web_dir())
        .title("SC Desk")
        .inner_size(WIDTH, HEIGHT)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(pin)
        .skip_taskbar(true)
        .visible(false)
        .build()?;
    // where it was left, or the bottom-right corner of the screen
    match saved {
        Some((x, y)) if on_some_monitor(&w, x, y) => {
            let _ = w.set_position(PhysicalPosition::new(x, y));
        }
        _ => {
            if let Ok(Some(m)) = w.current_monitor() {
                let scale = m.scale_factor();
                let area = m.work_area();
                let x = area.position.x + area.size.width as i32 - ((WIDTH + 24.0) * scale) as i32;
                let y = area.position.y + area.size.height as i32 - ((HEIGHT + 220.0) * scale) as i32;
                let _ = w.set_position(PhysicalPosition::new(x, y));
            }
        }
    }
    w.on_window_event(move |e| {
        if let WindowEvent::Moved(p) = e {
            let (app, p) = (handle.clone(), (p.x, p.y));
            tauri::async_runtime::spawn(async move {
                let _ = app.state::<AppState>().db.kv_put(POS_KEY, &p).await;
            });
        }
    });
    let _ = w.show();
    crate::tray::hide_main(app);
    watch_fullscreen(app.clone());
    Ok(())
}

/// Click-through while a full-screen app is in front and the pointer is not the plain system arrow;
/// stops with the window.
fn watch_fullscreen(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut through = false;
        let mut over_since: Option<std::time::Instant> = None;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            let Some(w) = app.get_webview_window(LABEL) else { return };
            let own = w.hwnd().map(|h| h.0 as isize).unwrap_or(0);
            // the pointer rests on the capsule a moment: the user means it, whatever is under it (a game draws
            // its own pointer there, so the arrow test alone flickered and clicks went through to the game)
            let over = cursor_over(own);
            over_since = if over { over_since.or(Some(std::time::Instant::now())) } else { None };
            let resting = over_since.is_some_and(|t| t.elapsed() >= std::time::Duration::from_millis(330));
            // a window that is not pinned goes behind the game by itself: nothing to let through
            let now = fullscreen_in_front(own)
                && !resting
                && !system_arrow_shown()
                && CLICKABLE_UNTIL.load(std::sync::atomic::Ordering::Relaxed) <= now_ms()
                && PIN.load(std::sync::atomic::Ordering::Relaxed);
            if now != through {
                through = now;
                let _ = w.set_ignore_cursor_events(now);
                tracing::debug!(click_through = now, "mini player: full-screen app in front");
            }
        }
    });
}

#[cfg(windows)]
pub(crate) fn fullscreen_in_front(own: isize) -> bool {
    use windows_sys::Win32::{
        Foundation::RECT,
        Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST},
        UI::WindowsAndMessaging::{GetClassNameW, GetDesktopWindow, GetForegroundWindow, GetShellWindow, GetWindowRect},
    };
    // SAFETY: plain Win32 queries on the foreground window handle; every out
    // parameter is a zeroed local of the right size.
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_null() || fg as isize == own || fg == GetShellWindow() || fg == GetDesktopWindow() {
            return false;
        }
        // the desktop itself covers the screen too
        let mut cls = [0u16; 64];
        let n = GetClassNameW(fg, cls.as_mut_ptr(), cls.len() as i32);
        let class = String::from_utf16_lossy(&cls[..n.max(0) as usize]);
        if class == "Progman" || class == "WorkerW" {
            return false;
        }
        let mut r: RECT = std::mem::zeroed();
        if GetWindowRect(fg, &mut r) == 0 {
            return false;
        }
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST), &mut mi) == 0 {
            return false;
        }
        let m = mi.rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

#[cfg(not(windows))]
pub(crate) fn fullscreen_in_front(_own: isize) -> bool {
    false
}

/// Is the pointer shown right now AND the plain Windows arrow? A game in play hides it or draws its own pointer
/// (a different cursor handle); its menus, console and settings screens often show the arrow.
#[cfg(windows)]
fn system_arrow_shown() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorInfo, LoadCursorW, CURSORINFO, CURSOR_SHOWING, IDC_ARROW};
    // SAFETY: GetCursorInfo fills a zeroed local struct whose size is set first; LoadCursorW with no module and a
    // predefined id returns the shared system cursor.
    unsafe {
        let mut ci: CURSORINFO = std::mem::zeroed();
        ci.cbSize = std::mem::size_of::<CURSORINFO>() as u32;
        GetCursorInfo(&mut ci) != 0 && ci.flags & CURSOR_SHOWING != 0 && ci.hCursor == LoadCursorW(std::ptr::null_mut(), IDC_ARROW)
    }
}

/// The pointer is over this window (its rectangle on the screen).
#[cfg(windows)]
fn cursor_over(own: isize) -> bool {
    use windows_sys::Win32::{
        Foundation::{POINT, RECT},
        UI::WindowsAndMessaging::{GetCursorPos, GetWindowRect},
    };
    if own == 0 {
        return false;
    }
    // SAFETY: plain Win32 queries filling zeroed locals; the handle is our own live window.
    unsafe {
        let mut p: POINT = std::mem::zeroed();
        let mut r: RECT = std::mem::zeroed();
        GetCursorPos(&mut p) != 0 && GetWindowRect(own as _, &mut r) != 0 && p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
    }
}

#[cfg(not(windows))]
fn cursor_over(_own: isize) -> bool {
    false
}

#[cfg(not(windows))]
fn system_arrow_shown() -> bool {
    true
}

fn on_some_monitor(w: &tauri::WebviewWindow, x: i32, y: i32) -> bool {
    w.available_monitors().unwrap_or_default().iter().any(|m| {
        let (p, s) = (m.position(), m.size());
        x >= p.x - 50 && y >= p.y - 50 && x < p.x + s.width as i32 - 50 && y < p.y + s.height as i32 - 50
    })
}

/// `restore`: back to the big window; otherwise just close (tray).
pub fn close(app: &AppHandle, restore: bool) {
    // the big window first: the mini one is the caller and goes last
    if restore {
        crate::tray::show_main(app);
    }
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.destroy();
    }
}

/// The page grows downward on hover (controls) and for the queue.
pub fn resize(app: &AppHandle, height: f64) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.set_size(LogicalSize::new(WIDTH, height.clamp(HEIGHT, 560.0)));
    }
}
