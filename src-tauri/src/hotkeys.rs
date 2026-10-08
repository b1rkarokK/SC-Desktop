//! Global hotkeys: hardware media keys + Ctrl+Alt+P / Ctrl+Alt+→ / Ctrl+Alt+←.
//! Registration failures (key taken by another app) are logged, not fatal.

use std::sync::Arc;

use tauri::{plugin::TauriPlugin, AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::player::Player;

fn shortcuts() -> Vec<Shortcut> {
    let ca = Some(Modifiers::CONTROL | Modifiers::ALT);
    vec![
        Shortcut::new(None, Code::MediaPlayPause),
        Shortcut::new(None, Code::MediaTrackNext),
        Shortcut::new(None, Code::MediaTrackPrevious),
        Shortcut::new(None, Code::MediaStop),
        Shortcut::new(ca, Code::KeyP),
        Shortcut::new(ca, Code::ArrowRight),
        Shortcut::new(ca, Code::ArrowLeft),
    ]
}

pub fn plugin() -> TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            let Some(player) = app.try_state::<Arc<Player>>() else { return };
            let player = player.inner().clone();
            match shortcut.key {
                Code::MediaPlayPause | Code::KeyP => player.toggle(),
                Code::MediaTrackNext | Code::ArrowRight => player.next(),
                Code::MediaTrackPrevious | Code::ArrowLeft => player.prev(),
                Code::MediaStop => player.pause(),
                _ => {}
            }
        })
        .build()
}

pub fn register(app: &AppHandle) {
    let gs = app.global_shortcut();
    for sc in shortcuts() {
        if let Err(e) = gs.register(sc) {
            tracing::warn!(shortcut = %sc, error = %e, "global shortcut not registered");
        }
    }
}
