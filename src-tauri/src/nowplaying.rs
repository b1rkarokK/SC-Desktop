//! «Что играет» over a game: a hotkey shows a small plate with the song (or the current and the next line of its
//! lyrics) for a few seconds. A window of its own that takes no focus and no clicks, so the game does not notice
//! it. It is made when needed and destroyed a while after the last use: no WebView stays in memory for it.

use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "nowplaying";
const WIDTH: f64 = 480.0;
const HEIGHT: f64 = 112.0;
/// how long the plate stays on the screen
const SHOW_MS: u64 = 3500;
/// how long the window itself is kept after that (a second press then shows at once)
const KEEP_MS: u64 = 90_000;

static GEN: AtomicU64 = AtomicU64::new(0);

/// Shows the plate: `lyrics` = the lines of the song, otherwise the song's name.
pub async fn show(app: &AppHandle, lyrics: bool) {
    let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let existing = app.get_webview_window(LABEL);
    let w = match existing {
        Some(w) => {
            let _ = app.emit_to(LABEL, "np:show", serde_json::json!({ "lyrics": lyrics, "ms": SHOW_MS }));
            w
        }
        None => {
            let url = format!("index.html?np=1&mode={}&ms={SHOW_MS}", if lyrics { "lyrics" } else { "track" });
            let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(url.into()))
                .additional_browser_args(crate::net::bypass::browser_args()).data_directory(crate::portable::web_dir())
                .title("SC Desk")
                .inner_size(WIDTH, HEIGHT)
                .resizable(false)
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .focused(false)
                .visible(false)
                .build();
            match built {
                Ok(w) => w,
                Err(e) => return tracing::warn!(error = %e, "now playing: no window"),
            }
        }
    };
    place(app, &w);
    no_focus(&w);
    let _ = w.set_ignore_cursor_events(true);
    let _ = w.show();

    // hide after the time; destroy after a longer one, unless pressed again
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(SHOW_MS + 500)).await;
        if GEN.load(Ordering::SeqCst) != gen {
            return;
        }
        if let Some(w) = app.get_webview_window(LABEL) {
            let _ = w.hide();
        }
        tokio::time::sleep(std::time::Duration::from_millis(KEEP_MS)).await;
        if GEN.load(Ordering::SeqCst) == gen {
            if let Some(w) = app.get_webview_window(LABEL) {
                let _ = w.destroy();
            }
        }
    });
}

/// Top centre of the screen the mouse is on (that is where a game is played).
fn place(app: &AppHandle, w: &tauri::WebviewWindow) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    if let Some(m) = monitor {
        let scale = m.scale_factor();
        let (pos, size) = (m.position(), m.size());
        let x = pos.x + (size.width as i32 - (WIDTH * scale) as i32) / 2;
        let y = pos.y + (size.height as f64 * 0.06) as i32;
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
}

/// The window never takes the focus away from the game (WS_EX_NOACTIVATE) and does not show in Alt+Tab (TOOLWINDOW).
#[cfg(windows)]
fn no_focus(w: &tauri::WebviewWindow) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW};
    let Ok(hwnd) = w.hwnd() else { return };
    // SAFETY: plain style change on our own window handle.
    unsafe {
        let h = hwnd.0 as _;
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        SetWindowLongPtrW(h, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE as isize | WS_EX_TOOLWINDOW as isize);
    }
}

#[cfg(not(windows))]
fn no_focus(_w: &tauri::WebviewWindow) {}
