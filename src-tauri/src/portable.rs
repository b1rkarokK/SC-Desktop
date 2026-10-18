//! Where the program keeps its data. An installed copy keeps everything in the `data` folder next to `sc-desk.exe`
//! (settings, library cache, covers, VPN engine, models, logs, the built-in browser's profile): one folder, nothing
//! left in AppData, and deleting the program folder leaves no trace. A development build (run from `target`) and a
//! program folder that cannot be written to keep to the usual places in AppData.
//! The first start after the move carries over what the older versions had put in AppData.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tauri::{AppHandle, Manager};

/// `<program folder>\data`, when the program lives in a folder it can write to (and is not a development build).
pub fn root() -> Option<&'static PathBuf> {
    static ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();
    ROOT.get_or_init(|| {
        if std::env::var_os("SCDESK_APPDATA").is_some() {
            return None;
        }
        let exe = std::env::current_exe().ok()?;
        let dir = exe.parent()?;
        if dir.components().any(|c| c.as_os_str().eq_ignore_ascii_case("target")) {
            return None;
        }
        let root = dir.join("data");
        std::fs::create_dir_all(&root).ok()?;
        let probe = root.join(".write-test");
        std::fs::write(&probe, b"").ok()?;
        let _ = std::fs::remove_file(probe);
        Some(root)
    })
    .as_ref()
}

/// settings and the library cache (AppData\Roaming before)
pub fn data(app: &AppHandle) -> tauri::Result<PathBuf> {
    match root() {
        Some(r) => Ok(r.join("roaming")),
        None => app.path().app_data_dir(),
    }
}

/// covers, the browser's profile, logs (AppData\Local before)
pub fn local(app: &AppHandle) -> tauri::Result<PathBuf> {
    match root() {
        Some(r) => Ok(r.join("local")),
        None => app.path().app_local_data_dir(),
    }
}

pub fn log(app: &AppHandle) -> tauri::Result<PathBuf> {
    match root() {
        Some(_) => Ok(local(app)?.join("logs")),
        None => app.path().app_log_dir(),
    }
}

static WEB: OnceLock<PathBuf> = OnceLock::new();

/// the folder of the built-in browser's profile for every window (it makes `EBWebView` inside)
pub fn web_dir() -> PathBuf {
    WEB.get().cloned().unwrap_or_default()
}

/// At the start, before the log and any window: fixes the browser folder and carries the old data over.
pub fn prepare(app: &AppHandle) {
    if let Ok(l) = local(app) {
        let _ = WEB.set(l);
    }
    let Some(root) = root() else { return };
    let (Ok(old_data), Ok(old_local)) = (app.path().app_data_dir(), app.path().app_local_data_dir()) else { return };
    for (old, new) in [(old_data, root.join("roaming")), (old_local, root.join("local"))] {
        if new.exists() || !old.exists() {
            continue;
        }
        if std::fs::rename(&old, &new).is_err() {
            // another drive, or something in it is open: copy, and remove what could be copied
            if copy_dir(&old, &new).is_ok() {
                let _ = std::fs::remove_dir_all(&old);
            }
        }
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            // a file held open by another copy of the program is left behind
            let _ = std::fs::copy(entry.path(), &target);
        }
    }
    Ok(())
}
