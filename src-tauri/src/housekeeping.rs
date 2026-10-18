//! Keeping the disk tidy while the app runs:
//! * the installers of past updates (the updater leaves `SC Desk-<version>-updater-XXXX\SC Desk-<version>-installer.exe`
//!   in the temp folder, about 7 MB each, forever) are deleted;
//! * the built-in browser's cache is held to a size (its own `--disk-cache-size` limit is not kept by WebView2:
//!   200 MB were found), the oldest files going first. Files that are in use are skipped, nothing is lost: it is a cache.

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use tauri::{AppHandle, Manager};

/// The browser cache is trimmed when it is over `HIGH`, down to `LOW`.
const HIGH: u64 = 24 * 1024 * 1024;
const LOW: u64 = 12 * 1024 * 1024;

/// Folders of the browser's profile that are only a cache (what «Очистить кэш» removes), relative to `EBWebView/Default`.
pub const PROFILE_CACHE_DIRS: [&str; 6] = ["Cache", "Code Cache", "GPUCache", "Service Worker", "DawnWebGPUCache", "DawnGraphiteCache"];
/// … and relative to `EBWebView`.
pub const ROOT_CACHE_DIRS: [&str; 3] = ["Subresource Filter", "component_crx_cache", "GrShaderCache"];

/// «SC Desk-1.3.0-updater-GV7KAX»: the folder the updater unpacks the installer into.
fn is_update_leftover(name: &str) -> bool {
    name.starts_with("SC Desk-") && name.contains("-updater-")
}

/// Deletes the leftovers in `temp`; the number of folders removed.
fn remove_update_leftovers(temp: &Path) -> usize {
    let Ok(rd) = std::fs::read_dir(temp) else { return 0 };
    rd.flatten()
        .filter(|e| e.file_name().to_str().is_some_and(is_update_leftover) && e.path().is_dir())
        .filter(|e| std::fs::remove_dir_all(e.path()).is_ok())
        .count()
}

fn files_under(dir: &Path, out: &mut Vec<(PathBuf, u64, SystemTime)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        if m.is_dir() {
            files_under(&e.path(), out);
        } else {
            out.push((e.path(), m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
        }
    }
}

/// Over `high` bytes: deletes the oldest files of `dirs` until `low` is left. Returns the bytes freed.
fn trim(dirs: &[PathBuf], high: u64, low: u64) -> u64 {
    let mut files = Vec::new();
    for d in dirs {
        files_under(d, &mut files);
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= high {
        return 0;
    }
    files.sort_by_key(|f| f.2);
    let mut freed = 0;
    for (path, size, _) in files {
        if total <= low {
            break;
        }
        // a file the browser holds open cannot be deleted: leave it
        if std::fs::remove_file(&path).is_ok() {
            total -= size;
            freed += size;
        }
    }
    freed
}

fn cache_dirs(app: &AppHandle) -> Vec<PathBuf> {
    let Ok(local) = crate::portable::local(app) else { return Vec::new() };
    let web = local.join("EBWebView");
    let profile = web.join("Default");
    PROFILE_CACHE_DIRS.iter().map(|d| profile.join(d)).chain(ROOT_CACHE_DIRS.iter().map(|d| web.join(d))).collect()
}

/// Called once at the start: tidies a little later (the installer that has just relaunched the app is still
/// finishing at the first moment), then every few hours while the app stays open.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            let handle = app.clone();
            let (folders, freed) = tokio::task::spawn_blocking(move || {
                let n = remove_update_leftovers(&std::env::temp_dir());
                (n, trim(&cache_dirs(&handle), HIGH, LOW))
            })
            .await
            .unwrap_or((0, 0));
            if folders > 0 || freed > 0 {
                tracing::info!(installers = folders, cache_freed = freed, "housekeeping");
            }
            tokio::time::sleep(Duration::from_secs(4 * 3600)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("scdesk-hk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn only_the_updaters_folders_are_leftovers() {
        assert!(is_update_leftover("SC Desk-1.3.0-updater-GV7KAX"));
        assert!(!is_update_leftover("SC Desk"));
        assert!(!is_update_leftover("vs_setup_bootstrapper"));
        assert!(!is_update_leftover("other-updater-1"));
    }

    #[test]
    fn leftovers_go_and_other_files_stay() {
        let t = scratch("left");
        let old = t.join("SC Desk-1.2.0-updater-AAAAAA");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("SC Desk-1.2.0-installer.exe"), b"x").unwrap();
        std::fs::create_dir_all(t.join("something-else")).unwrap();
        std::fs::write(t.join("note.txt"), b"x").unwrap();
        assert_eq!(remove_update_leftovers(&t), 1);
        assert!(!old.exists() && t.join("something-else").exists() && t.join("note.txt").exists());
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn the_oldest_cache_files_go_first_and_a_small_cache_is_left_alone() {
        let t = scratch("trim");
        let now = SystemTime::now();
        for (i, age_h) in [30u64, 20, 10, 1].into_iter().enumerate() {
            let p = t.join(format!("f{i}"));
            std::fs::write(&p, vec![0u8; 100]).unwrap();
            let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
            f.set_modified(now - Duration::from_secs(age_h * 3600)).unwrap();
        }
        // 400 bytes, limit 1000: untouched
        assert_eq!(trim(&[t.clone()], 1000, 500), 0);
        // limit 300, down to 200: the two oldest go
        assert_eq!(trim(&[t.clone()], 300, 200), 200);
        assert!(!t.join("f0").exists() && !t.join("f1").exists());
        assert!(t.join("f2").exists() && t.join("f3").exists());
        let _ = std::fs::remove_dir_all(&t);
    }
}
