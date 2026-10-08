use std::sync::Arc;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};

use crate::{models::TrackDto, player::Player};

const TRAY_ID: &str = "main";

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle", "Пуск / Пауза", true, None::<&str>)?;
    let prev = MenuItem::with_id(app, "prev", "Предыдущий", true, None::<&str>)?;
    let next = MenuItem::with_id(app, "next", "Следующий", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "Открыть окно", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&toggle, &prev, &next, &sep1, &show, &sep2, &quit])?;

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
                ("show", _) => show_main(app),
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

pub fn set_now_playing(app: &AppHandle, track: Option<&TrackDto>, playing: bool) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let text = match track {
        Some(t) => {
            let mark = if playing { "" } else { " (пауза)" };
            // Windows tray tooltips are limited to 127 chars
            let s = format!("{} — {}{}", t.artist, t.title, mark);
            s.chars().take(120).collect()
        }
        None => "SC Desk".to_owned(),
    };
    let _ = tray.set_tooltip(Some(text));
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        let _ = app.emit("window:visibility", true);
    }
}

pub fn hide_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
        let _ = app.emit("window:visibility", false);
    }
}

fn toggle_main(app: &AppHandle) {
    let visible = app
        .get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    if visible {
        hide_main(app);
    } else {
        show_main(app);
    }
}
