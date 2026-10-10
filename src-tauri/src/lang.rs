//! Interface language for the few texts the core shows by itself: tray menu,
//! Discord button, file pickers, sign-in / check windows. Everything else is
//! translated in the UI (it gets Russian messages and translates them).
//! The UI keeps the choice in config.json → `ui.lang`.

use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

/// From config.json `ui` (on start and whenever the UI saves it).
pub fn set_from_ui(ui: &serde_json::Value) -> bool {
    let en = ui.get("lang").and_then(|v| v.as_str()) == Some("en");
    ENGLISH.swap(en, Ordering::Relaxed) != en
}

/// The Russian or the English text.
pub fn pick<'a>(ru: &'a str, en: &'a str) -> &'a str {
    if ENGLISH.load(Ordering::Relaxed) {
        en
    } else {
        ru
    }
}
