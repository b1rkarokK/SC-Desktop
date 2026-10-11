//! Резервная копия одним файлом: лайки, дизлайки, история, сигналы «Моей волны», настройки.
//! Не входят: вход в SoundCloud (он в Диспетчере учётных данных), прокси, сохранённые тексты
//! и обложки (они докачиваются сами). Восстановление только добавляет: ничего не стирает.

use serde::Serialize;
use serde_json::{json, Value};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::{
    error::{AppError, AppResult},
    lang::pick,
    state::AppState,
};

const FORMAT: u32 = 1;

#[derive(Serialize)]
pub struct BackupStats {
    pub likes: usize,
    pub plays: usize,
    pub tracks: usize,
}

fn count(v: &Value, key: &str) -> usize {
    v[key].as_array().map_or(0, |a| a.len())
}

pub async fn export(app: &AppHandle, state: &AppState) -> AppResult<Option<String>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let name = format!("sc-desk-backup-{}.json", chrono_free_date());
    app.dialog()
        .file()
        .set_title(pick("Сохранить резервную копию", "Save a backup"))
        .set_file_name(name)
        .add_filter(pick("Резервная копия SC Desk", "SC Desk backup"), &["json"])
        .save_file(move |p| {
            let _ = tx.send(p.and_then(|p| p.into_path().ok()));
        });
    let Some(path) = rx.await.ok().flatten() else { return Ok(None) };
    let mut dump = state.db.backup_dump().await?;
    let cfg = state.config.get();
    dump["format"] = json!(FORMAT);
    dump["app"] = json!(env!("CARGO_PKG_VERSION"));
    dump["config"] = json!({
        "ui": cfg.ui, "eq": cfg.eq, "fx": cfg.fx, "volume": cfg.volume,
        "wave_no_liked": cfg.wave_no_liked, "notify_new": cfg.notify_new, "fast_protected": cfg.fast_protected,
        "discord": { "enabled": cfg.discord.enabled, "progress": cfg.discord.progress, "button": cfg.discord.button, "hide_on_pause": cfg.discord.hide_on_pause },
    });
    tokio::fs::write(&path, serde_json::to_vec(&dump)?).await?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

pub async fn import(app: &AppHandle, state: &AppState) -> AppResult<Option<BackupStats>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title(pick("Восстановить из резервной копии", "Restore from a backup"))
        .add_filter(pick("Резервная копия SC Desk", "SC Desk backup"), &["json"])
        .pick_file(move |p| {
            let _ = tx.send(p.and_then(|p| p.into_path().ok()));
        });
    let Some(path) = rx.await.ok().flatten() else { return Ok(None) };
    let bytes = tokio::fs::read(&path).await?;
    let dump: Value = serde_json::from_slice(&bytes).map_err(|_| AppError::Other(pick("Это не резервная копия SC Desk", "This is not an SC Desk backup").into()))?;
    if dump["format"].as_u64() != Some(FORMAT as u64) {
        return Err(AppError::Other(pick("Резервная копия другой версии, такую открыть нельзя", "This backup is of another version and cannot be opened").into()));
    }
    state.db.backup_restore(&dump).await?;
    // settings: only what makes sense on another machine
    let c = &dump["config"];
    state.config.update(|cfg| {
        if !c["ui"].is_null() {
            cfg.ui = c["ui"].clone();
        }
        if let Ok(eq) = serde_json::from_value(c["eq"].clone()) {
            cfg.eq = eq;
        }
        if let Ok(fx) = serde_json::from_value(c["fx"].clone()) {
            cfg.fx = fx;
        }
        if let Some(v) = c["volume"].as_f64() {
            cfg.volume = v as f32;
        }
        for (key, slot) in [("wave_no_liked", &mut cfg.wave_no_liked), ("notify_new", &mut cfg.notify_new), ("fast_protected", &mut cfg.fast_protected)] {
            if let Some(b) = c[key].as_bool() {
                *slot = b;
            }
        }
    })?;
    Ok(Some(BackupStats { likes: count(&dump, "likes"), plays: count(&dump, "plays"), tracks: count(&dump, "tracks") }))
}

/// YYYY-MM-DD of today, local time, without a date crate.
fn chrono_free_date() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    // civil-from-days (Howard Hinnant); UTC is close enough for a file name
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", y + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_for_a_file_name() {
        // 2026-10-12 00:00:00 UTC is 1791763200
        let d = chrono_free_date();
        assert_eq!(d.len(), 10);
        assert!(d.starts_with("20"));
    }
}

#[cfg(test)]
mod roundtrip {
    use crate::{api::soundcloud::ScTrack, db::Db};

    fn track(id: u64) -> ScTrack {
        serde_json::from_value(serde_json::json!({ "id": id, "title": format!("t{id}"), "duration": 120000, "user_id": id % 3 })).unwrap()
    }

    fn temp_db(name: &str) -> (Db, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("scdesk-backup-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (Db::open(&dir.join("cache.sqlite3")).unwrap(), dir)
    }

    #[tokio::test]
    async fn a_backup_restores_into_an_empty_database_and_twice_changes_nothing() {
        let (a, dir_a) = temp_db("a");
        a.upsert_tracks((1..=5).map(track).collect()).await.unwrap();
        for id in [1, 2, 3] {
            a.like_add(id).await.unwrap();
        }
        a.record_play(1).await.unwrap();
        a.record_play(4).await.unwrap();
        a.dislike_track(5).await.unwrap();
        a.wave_signal_put(2, 1).await.unwrap();
        let dump = a.backup_dump().await.unwrap();
        assert_eq!(dump["likes"].as_array().unwrap().len(), 3);
        assert_eq!(dump["tracks"].as_array().unwrap().len(), 5);

        let (b, dir_b) = temp_db("b");
        b.backup_restore(&dump).await.unwrap();
        b.backup_restore(&dump).await.unwrap(); // the same file again: no duplicates
        let again = b.backup_dump().await.unwrap();
        for key in ["likes", "plays", "disliked_tracks", "wave_signals", "tracks"] {
            assert_eq!(again[key].as_array().unwrap().len(), dump[key].as_array().unwrap().len(), "{key}");
        }
        assert_eq!(b.likes_ids().await.unwrap().len(), 3);
        let _ = (std::fs::remove_dir_all(dir_a), std::fs::remove_dir_all(dir_b));
    }
}
