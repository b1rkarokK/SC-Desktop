//! In-app updates from GitHub Releases (signed with minisign; the public key is
//! in tauri.conf.json).
//!
//! The Rust core checks shortly after start and then every hour — also while the
//! app lives in the tray without a window. A found update is kept here and
//! pushed to the window (`update:available`); a window opened later asks for it
//! via `update_pending`. "Позже" silences that version until the next launch.
//! "Обновить" downloads with progress (`update:progress`) and restarts.

use std::{sync::Mutex, time::Duration};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

use crate::error::{AppError, AppResult};

const FIRST_CHECK: Duration = Duration::from_secs(5);
const RECHECK: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    /// RFC 3339 from latest.json, if present
    pub date: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

#[derive(Default)]
pub struct UpdateState {
    pending: Mutex<Option<UpdateInfo>>,
    dismissed: Mutex<Option<String>>,
}

impl UpdateState {
    fn pending_for_ui(&self) -> Option<UpdateInfo> {
        let pending = self.pending.lock().unwrap_or_else(|p| p.into_inner()).clone()?;
        let dismissed = self.dismissed.lock().unwrap_or_else(|p| p.into_inner()).clone();
        (dismissed.as_deref() != Some(pending.version.as_str())).then_some(pending)
    }
}

fn err(e: tauri_plugin_updater::Error) -> AppError {
    AppError::Other(format!("обновление: {e}"))
}

async fn check(app: &AppHandle) -> AppResult<Option<UpdateInfo>> {
    let update = app.updater().map_err(err)?.check().await.map_err(err)?;
    Ok(update.map(|u| UpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        date: u.raw_json["pub_date"].as_str().map(str::to_owned),
        notes: u.body.clone().filter(|b| !b.trim().is_empty()),
    }))
}

/// Background checker; one sleeping task, wakes once an hour.
pub fn start(app: &AppHandle) {
    app.manage(UpdateState::default());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            match check(&app).await {
                Ok(Some(info)) => {
                    let state = app.state::<UpdateState>();
                    let is_new = state
                        .pending
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .replace(info.clone())
                        .is_none_or(|old| old.version != info.version);
                    if is_new {
                        tracing::info!(version = %info.version, "update available");
                        crate::tray::set_update_hint(&app, Some(&info.version));
                    }
                    if let Some(show) = state.pending_for_ui() {
                        let _ = app.emit("update:available", show);
                    }
                }
                Ok(None) => {}
                Err(e) => tracing::debug!(error = %e, "update check failed"),
            }
            tokio::time::sleep(RECHECK).await;
        }
    });
}

/// The window asks on startup whether an update is waiting.
#[tauri::command]
pub fn update_pending(state: tauri::State<'_, UpdateState>) -> Option<UpdateInfo> {
    state.pending_for_ui()
}

/// "Позже": don't nag about this version again until the next launch.
#[tauri::command]
pub fn update_dismiss(state: tauri::State<'_, UpdateState>, version: String) {
    *state.dismissed.lock().unwrap_or_else(|p| p.into_inner()) = Some(version);
}

/// Manual check from Settings.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> AppResult<Option<UpdateInfo>> {
    let info = check(&app).await?;
    if let Some(i) = &info {
        *app.state::<UpdateState>().pending.lock().unwrap_or_else(|p| p.into_inner()) = Some(i.clone());
    }
    Ok(info)
}

/// Downloads and installs; emits `update:progress` and `update:installing`.
/// On success the app restarts (on Windows the installer relaunches it).
#[tauri::command]
pub async fn update_install(app: AppHandle) -> AppResult<()> {
    let Some(update) = app.updater().map_err(err)?.check().await.map_err(err)? else {
        return Err(AppError::Other("обновление больше не доступно".into()));
    };
    tracing::info!(to = %update.version, "installing update");
    let mut downloaded = 0u64;
    let progress_app = app.clone();
    let done_app = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = progress_app.emit("update:progress", Progress { downloaded, total });
            },
            move || {
                let _ = done_app.emit("update:installing", ());
            },
        )
        .await
        .map_err(err)?;
    tracing::info!("update installed, restarting");
    app.restart();
}

#[tauri::command]
pub fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}
