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
//! desktop in front it is clickable again. One cheap system call a second.

use tauri::{AppHandle, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::state::AppState;

pub const LABEL: &str = "mini";
const WIDTH: f64 = 340.0;
const HEIGHT: f64 = 58.0;
const POS_KEY: &str = "mini:pos";

pub async fn open(app: &AppHandle) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.show();
        let _ = w.set_focus();
        crate::tray::hide_main(app);
        return Ok(());
    }
    let saved = app.state::<AppState>().db.kv_get::<(i32, i32)>(POS_KEY).await.ok().flatten().map(|(p, _)| p);
    let handle = app.clone();
    let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html?mini=1".into()))
        .title("SC Desk")
        .inner_size(WIDTH, HEIGHT)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
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

/// Click-through while a full-screen app is in front; stops with the window.
fn watch_fullscreen(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut through = false;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            let Some(w) = app.get_webview_window(LABEL) else { return };
            let own = w.hwnd().map(|h| h.0 as isize).unwrap_or(0);
            let now = fullscreen_in_front(own);
            if now != through {
                through = now;
                let _ = w.set_ignore_cursor_events(now);
                tracing::debug!(click_through = now, "mini player: full-screen app in front");
            }
        }
    });
}

#[cfg(windows)]
fn fullscreen_in_front(own: isize) -> bool {
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
fn fullscreen_in_front(_own: isize) -> bool {
    false
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
