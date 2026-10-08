//! In-app updates from GitHub Releases (signed with minisign; the public key is
//! in tauri.conf.json). The UI shows version, date and changelog, and on
//! "Обновить" downloads with progress and restarts — no browser involved.

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

use crate::error::{AppError, AppResult};

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

fn err(e: tauri_plugin_updater::Error) -> AppError {
    AppError::Other(format!("обновление: {e}"))
}

#[tauri::command]
pub async fn update_check(app: AppHandle) -> AppResult<Option<UpdateInfo>> {
    let update = app.updater().map_err(err)?.check().await.map_err(err)?;
    Ok(update.map(|u| UpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        date: u.raw_json["pub_date"].as_str().map(str::to_owned),
        notes: u.body.clone().filter(|b| !b.trim().is_empty()),
    }))
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
