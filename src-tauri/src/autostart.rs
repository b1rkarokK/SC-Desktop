//! "Запускать вместе с системой" (HKCU\…\Run on Windows).
//!
//! * The entry always points at the *running installed* exe: on every start of
//!   a release build an enabled entry is re-written, so an old path (e.g. from
//!   a dev build or another install folder) is fixed automatically.
//! * Dev builds never touch the entry — otherwise they'd point it at target\debug.
//! * Disabling removes the Run value and Windows' "StartupApproved" flag.
//! * First launch of an installed build enables it (starts minimized to tray).

use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;

use crate::{
    error::{AppError, AppResult},
    state::AppState,
};

const DEV: bool = cfg!(debug_assertions);

pub fn is_enabled(app: &AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

pub fn set(app: &AppHandle, enabled: bool) -> AppResult<()> {
    if DEV {
        return Err(AppError::Other(
            "В тестовой (отладочной) версии автозапуск не меняется — проверьте в установленной программе".into(),
        ));
    }
    let al = app.autolaunch();
    let result = if enabled {
        // disable first so the value is re-written with the current exe path
        let _ = al.disable();
        al.enable()
    } else {
        al.disable()
    };
    result.map_err(|e| AppError::Other(format!("автозапуск: {e}")))?;
    if !enabled {
        clear_startup_approved();
    }
    Ok(())
}

/// Called once at startup.
pub fn sync(app: &AppHandle) {
    if DEV {
        return;
    }
    let state = app.state::<AppState>();
    let cfg = state.config.get();
    let want = if cfg.autostart_initialized { is_enabled(app) } else { true };
    if want {
        if let Err(e) = set(app, true) {
            tracing::warn!(error = %e, "autostart refresh failed");
        }
    }
    if !cfg.autostart_initialized {
        let _ = state.config.update(|c| c.autostart_initialized = true);
    }
}

#[cfg(windows)]
fn clear_startup_approved() {
    // Task Manager keeps its own on/off flag per Run value; drop it with the value.
    let _ = std::process::Command::new("reg")
        .args([
            "delete",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run",
            "/v",
            "SC Desk",
            "/f",
        ])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output();
}

#[cfg(not(windows))]
fn clear_startup_approved() {}

#[cfg(windows)]
use std::os::windows::process::CommandExt;
