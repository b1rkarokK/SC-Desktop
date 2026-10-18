//! Where the program keeps its data. An installed copy keeps everything in the `data` folder next to `sc-desk.exe`
//! (settings, library cache, covers, VPN engine, models, logs, the built-in browser's profile): one folder, nothing
//! left in AppData, and deleting the program folder leaves no trace. A development build (run from `target`) and a
//! program folder that cannot be written to keep to the usual places in AppData.
//! The first start after the move carries over what the older versions had put in AppData.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tauri::{AppHandle, Manager};

/// Set by `prepare`: the data is in the program folder (the move is done, or there was nothing to move).
static MOVED: OnceLock<bool> = OnceLock::new();
/// The file that says the move was finished.
const MARKER: &str = ".moved";

/// `<program folder>/data`, once the data is there.
pub fn root() -> Option<&'static PathBuf> {
    folder().filter(|_| MOVED.get().copied().unwrap_or(false))
}

/// `<program folder>/data`, when the program lives in a folder it can write to (and is not a development build).
fn folder() -> Option<&'static PathBuf> {
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

/// At the start, before the log and any window: carries the old data over. Nothing is half done: the data is
/// copied whole into a side folder and put in place only when every file went; if some file is held by another
/// copy of the program (an update that has not finished closing the old one), this run keeps to AppData
/// and the move is tried again at the next start.
pub fn prepare(app: &AppHandle) {
    let moved = move_data(app);
    let _ = MOVED.set(moved);
    if let Ok(l) = local(app) {
        let _ = WEB.set(l);
    }
}

fn move_data(app: &AppHandle) -> bool {
    let Some(root) = folder() else { return false };
    if root.join(MARKER).exists() {
        return true;
    }
    let (Ok(old_data), Ok(old_local)) = (app.path().app_data_dir(), app.path().app_local_data_dir()) else { return false };
    let pairs = [(old_data, root.join("roaming")), (old_local, root.join("local"))];
    transfer(&pairs, root)
}

/// Copies every old folder whole into its new place; the old ones go only when all of it is there.
fn transfer(pairs: &[(PathBuf, PathBuf)], root: &Path) -> bool {
    // a half move of an earlier version or of a failed try: start over from the old folders (kept until the end)
    for (old, new) in pairs {
        if !old.exists() {
            continue;
        }
        let _ = std::fs::remove_dir_all(new);
        let side = new.with_extension("moving");
        let _ = std::fs::remove_dir_all(&side);
        if copy_dir(old, &side).is_err() || std::fs::rename(&side, new).is_err() {
            let _ = std::fs::remove_dir_all(&side);
            return false;
        }
    }
    if std::fs::write(root.join(MARKER), b"").is_err() {
        return false;
    }
    // the old copies are not needed any more
    for (old, _) in pairs {
        let _ = std::fs::remove_dir_all(old);
    }
    true
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("scdesk-portable-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_old_data_moves_whole_and_the_old_folder_goes() {
        let t = scratch("ok");
        let (old, root) = (t.join("old"), t.join("data"));
        std::fs::create_dir_all(old.join("bin")).unwrap();
        std::fs::write(old.join("config.json"), b"{}").unwrap();
        std::fs::write(old.join("bin").join("engine.exe"), b"x").unwrap();
        std::fs::create_dir_all(&root).unwrap();
        assert!(transfer(&[(old.clone(), root.join("roaming"))], &root));
        assert!(root.join("roaming").join("bin").join("engine.exe").exists());
        assert!(root.join(MARKER).exists());
        assert!(!old.exists());
    }

    #[cfg(windows)]
    #[test]
    fn a_file_in_use_postpones_the_move_and_leaves_nothing_half_done() {
        use std::os::windows::fs::OpenOptionsExt;
        let t = scratch("busy");
        let (old, root) = (t.join("old"), t.join("data"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("a.txt"), b"a").unwrap();
        std::fs::write(old.join("held.sqlite3"), b"db").unwrap();
        std::fs::create_dir_all(&root).unwrap();
        // another copy of the program holds the file exclusively
        let _held = std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(old.join("held.sqlite3")).unwrap();
        assert!(!transfer(&[(old.clone(), root.join("roaming"))], &root));
        assert!(!root.join("roaming").exists());
        assert!(!root.join("roaming.moving").exists());
        assert!(!root.join(MARKER).exists());
        assert!(old.join("a.txt").exists());
    }
}
