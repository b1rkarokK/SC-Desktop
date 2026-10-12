//! Global hotkeys: hardware media keys (always) + the user's own combinations
//! (Настройки → Горячие клавиши). The combinations are global, i.e. work over
//! games, only while «Работают в играх» is on; otherwise the window itself
//! listens (src/ui/hotkeys.ts). Registration failures (key taken by another
//! app) are logged, not fatal.

use std::{collections::HashMap, str::FromStr, sync::{Arc, Mutex, OnceLock}};

use serde_json::Value;
use tauri::{plugin::TauriPlugin, AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::{commands, player::Player, state::AppState};

/// (action, default combination)
pub const DEFAULTS: [(&str, &str); 9] = [
    ("toggle", "Ctrl+Alt+KeyP"),
    ("next", "Ctrl+Alt+ArrowRight"),
    ("prev", "Ctrl+Alt+ArrowLeft"),
    ("like", "Ctrl+Alt+KeyL"),
    ("volUp", "Ctrl+Alt+ArrowUp"),
    ("volDown", "Ctrl+Alt+ArrowDown"),
    ("mini", "Ctrl+Alt+KeyM"),
    ("slowed", "Ctrl+Alt+Comma"),
    ("sped", "Ctrl+Alt+Period"),
];

/// Slowed / sped up from a key: the same key again goes back to normal. Not in a room (speed is locked there).
pub fn speed_preset(app: &AppHandle, player: &Arc<Player>, action: &str) {
    if player.speed_locked() {
        return;
    }
    let state = app.state::<AppState>();
    let mut fx = state.config.get().fx;
    let target = if action == "slowed" { 0.85 } else { 1.2 };
    fx.speed = if (fx.speed - target).abs() < 0.01 { 1.0 } else { target };
    player.set_fx(&fx);
    let _ = state.config.update(|c| c.fx = fx.clone());
    let _ = tauri::Emitter::emit(app, "fx:changed", &fx);
}

fn bound() -> &'static Mutex<HashMap<Shortcut, String>> {
    static M: OnceLock<Mutex<HashMap<Shortcut, String>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

/// "Ctrl+Alt+KeyP" (modifiers + `KeyboardEvent.code`) → shortcut.
fn parse(combo: &str) -> Option<Shortcut> {
    let mut mods = Modifiers::empty();
    let mut key = None;
    for part in combo.split('+') {
        match part {
            "Ctrl" => mods |= Modifiers::CONTROL,
            "Alt" => mods |= Modifiers::ALT,
            "Shift" => mods |= Modifiers::SHIFT,
            "Meta" => mods |= Modifiers::SUPER,
            code => key = Code::from_str(code).ok(),
        }
    }
    Some(Shortcut::new(if mods.is_empty() { None } else { Some(mods) }, key?))
}

fn run(app: &AppHandle, player: &Arc<Player>, action: &str) {
    match action {
        "toggle" => player.toggle(),
        "next" => player.next(),
        "prev" => player.prev(),
        "stop" => player.pause(),
        "volUp" | "volDown" => {
            let d = if action == "volUp" { 0.05 } else { -0.05 };
            let v = (player.snapshot().volume + d).clamp(0.0, 1.0);
            player.set_volume(v);
            let _ = tauri::Emitter::emit(app, "player:state", &player.snapshot());
        }
        "slowed" | "sped" => speed_preset(app, player, action),
        "mini" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { let _ = crate::mini::open(&app).await; });
        }
        "like" => {
            let app = app.clone();
            let Some(track) = player.current() else { return };
            tauri::async_runtime::spawn(async move {
                let state = app.state::<AppState>();
                let liked = state.db.likes_ids().await.map(|s| s.contains(&track.id)).unwrap_or(false);
                let _ = commands::like_set(app.clone(), app.state::<AppState>(), track, !liked).await;
                let _ = tauri::Emitter::emit(&app, "likes:changed", ());
            });
        }
        _ => {}
    }
}

pub fn plugin() -> TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            let Some(player) = app.try_state::<Arc<Player>>() else { return };
            let player = player.inner().clone();
            let action = match shortcut.key {
                Code::MediaPlayPause => Some("toggle".to_owned()),
                Code::MediaTrackNext => Some("next".to_owned()),
                Code::MediaTrackPrevious => Some("prev".to_owned()),
                Code::MediaStop => Some("stop".to_owned()),
                _ => bound().lock().ok().and_then(|m| m.get(shortcut).cloned()),
            };
            if let Some(a) = action {
                run(app, &player, &a);
            }
        })
        .build()
}

/// `hk`: `{inGames: bool, keys: {action: combo}}` (missing = defaults).
pub fn apply(app: &AppHandle, hk: &Value) {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let reg = |sc: Shortcut| {
        if let Err(e) = gs.register(sc) {
            tracing::warn!(shortcut = %sc, error = %e, "global shortcut not registered");
        }
    };
    for code in [Code::MediaPlayPause, Code::MediaTrackNext, Code::MediaTrackPrevious, Code::MediaStop] {
        reg(Shortcut::new(None, code));
    }
    let mut map = HashMap::new();
    if hk["inGames"].as_bool().unwrap_or(true) {
        for (action, default) in DEFAULTS {
            let combo = hk["keys"][action].as_str().unwrap_or(default);
            if combo.is_empty() {
                continue; // cleared by the user
            }
            if let Some(sc) = parse(combo) {
                reg(sc);
                map.insert(sc, action.to_owned());
            }
        }
    }
    if let Ok(mut m) = bound().lock() {
        *m = map;
    }
}

pub fn register(app: &AppHandle) {
    let hk = app.state::<AppState>().config.get().ui["hotkeys"].clone();
    apply(app, &hk);
}
