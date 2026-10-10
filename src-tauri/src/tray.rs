use std::sync::Arc;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WebviewWindowBuilder,
};

use crate::{models::TrackDto, player::Player};

const TRAY_ID: &str = "main";
const MAIN: &str = "main";

fn build_menu(app: &AppHandle, update: Option<&MenuItem<tauri::Wry>>) -> tauri::Result<Menu<tauri::Wry>> {
    use crate::lang::pick;
    let toggle = MenuItem::with_id(app, "toggle", pick("Пуск / Пауза", "Play / Pause"), true, None::<&str>)?;
    let prev = MenuItem::with_id(app, "prev", pick("Предыдущий", "Previous"), true, None::<&str>)?;
    let next = MenuItem::with_id(app, "next", pick("Следующий", "Next"), true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", pick("Открыть окно", "Open window"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", pick("Выход", "Quit"), true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&toggle, &prev, &next, &sep1, &show, &sep2, &quit])?;
    if let Some(item) = update {
        menu.insert(item, 0)?;
        menu.insert(&PredefinedMenuItem::separator(app)?, 1)?;
    }
    Ok(menu)
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app, None)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("SC Desk")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let player = app.try_state::<Arc<Player>>().map(|p| p.inner().clone());
            match (event.id().as_ref(), player) {
                ("toggle", Some(p)) => p.toggle(),
                ("prev", Some(p)) => p.prev(),
                ("next", Some(p)) => p.next(),
                ("show" | "update", _) => show_main(app),
                ("quit", _) => app.exit(0),
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                toggle_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Tray menu/tooltip hint "Доступно обновление X" (clicking it opens the window,
/// which shows the update dialog).
pub fn set_update_hint(app: &AppHandle, version: Option<&str>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    if let Some(v) = version {
        let label = format!("{} {v}", crate::lang::pick("Доступно обновление", "Update available:"));
        if let Ok(item) = MenuItem::with_id(app, "update", label, true, None::<&str>) {
            if let Ok(menu) = build_menu(app, Some(&item)) {
                let _ = tray.set_menu(Some(menu));
            }
        }
    }
}

/// Language changed: the menu in the new language (the update item, if any,
/// comes back with the next update check).
pub fn refresh_menu(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    if let Ok(menu) = build_menu(app, None) {
        let _ = tray.set_menu(Some(menu));
    }
}

pub fn set_now_playing(app: &AppHandle, track: Option<&TrackDto>, playing: bool) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let text = match track {
        Some(t) => {
            let mark = if playing { "" } else { crate::lang::pick(" (пауза)", " (paused)") };
            // Windows tray tooltips are limited to 127 chars
            let s = format!("{} — {}{}", t.artist, t.title, mark);
            s.chars().take(120).collect()
        }
        None => "SC Desk".to_owned(),
    };
    let _ = tray.set_tooltip(Some(text));
}

/// Creates the main window on demand (from tauri.conf.json, where it has
/// `create: false`) and brings it to front.
pub fn show_main(app: &AppHandle) {
    let window = match app.get_webview_window(MAIN) {
        Some(w) => w,
        None => {
            let Some(cfg) = app.config().app.windows.iter().find(|w| w.label == MAIN).cloned() else {
                tracing::error!("main window config missing");
                return;
            };
            match WebviewWindowBuilder::from_config(app, &cfg).and_then(|b| b.build()) {
                Ok(w) => w,
                Err(e) => {
                    tracing::error!(error = %e, "main window not created");
                    return;
                }
            }
        }
    };
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

/// Tray mode: the window — and every WebView2 process with it — is destroyed,
/// so in the tray only the Rust core (player, Discord, hotkeys) is running.
pub fn hide_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.destroy();
        tracing::debug!("main window destroyed (tray mode)");
    }
}

fn toggle_main(app: &AppHandle) {
    let visible = app
        .get_webview_window(MAIN)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    if visible {
        hide_main(app);
    } else {
        show_main(app);
    }
}
