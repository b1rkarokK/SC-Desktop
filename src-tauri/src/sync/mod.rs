//! Rooms and shared playlists talk through a small relay server (see /server): everything that
//! travels is encrypted on the devices (`crypto`), the hub keeps channels (`hub`).
pub mod crypto;
pub mod engine;
pub mod friends;
pub mod hub;
pub mod playlists;
pub mod shared;
pub mod watch;
pub mod service;

use std::sync::Mutex;
use tauri::{AppHandle, Manager};

/// Shared playlists and friends belong to the account that is signed in: they start when its sign-in is
/// known (at the start of the program, or later when the user signs in) and again when the account changes.
pub async fn start_account_services(app: &AppHandle) {
    static STARTED_FOR: Mutex<Option<u64>> = Mutex::new(None);
    let Some(me) = app.state::<crate::state::AppState>().me() else { return };
    let previous = {
        let mut g = STARTED_FOR.lock().unwrap_or_else(|e| e.into_inner());
        let previous = *g;
        *g = Some(me.id);
        previous
    };
    if previous == Some(me.id) {
        return;
    }
    let playlists = app.state::<shared::SharedPlaylists>();
    if previous.is_none() {
        playlists.start(app).await;
    } else {
        playlists.reconnect(app).await;
    }
    app.state::<friends::Friends>().start(app).await;
}
