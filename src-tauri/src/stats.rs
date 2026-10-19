//! How many people use the program: an anonymous count, nothing more.
//!
//! Once a day the program tells the sync server (the one rooms use) three things: a random number of this
//! installation (made here, tells nothing about who or where), the version and the system. The server counts each
//! number once a day. Not sent: a name, the account, an address (the server does not keep it), what is played.
//! It can be switched off in Settings; off, nothing leaves.

use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// The first try a while after the start (the start itself stays light), then every few hours until it went.
const FIRST_WAIT: Duration = Duration::from_secs(90);
const EVERY: Duration = Duration::from_secs(6 * 3600);

pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_WAIT).await;
        loop {
            let _ = once(&app).await;
            tokio::time::sleep(EVERY).await;
        }
    });
}

/** The date (UTC) as 2026-10-19, from the clock, without a crate for it. */
fn today() -> String {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0) as i64;
    // days since 1970-01-01 to a calendar date (the civil-from-days way)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn today_has_the_shape_of_a_date() {
        let d = super::today();
        assert_eq!(d.len(), 10);
        assert!(d.starts_with("20"));
        assert_eq!(&d[4..5], "-");
    }
}

fn new_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}

/// Sends the number if it is on and it has not been sent today.
async fn once(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let cfg = state.config.get();
    if !cfg.stats_anon || cfg.stats_day == today() {
        return Ok(());
    }
    // a development copy does not count as a user
    if cfg!(debug_assertions) || std::env::var_os("SCDESK_NO_STATS").is_some() {
        return Ok(());
    }
    let id = if cfg.install_id.len() == 32 { cfg.install_id.clone() } else { new_id() };
    let body = serde_json::json!({ "id": id, "v": env!("CARGO_PKG_VERSION"), "os": "win" });
    let url = format!("{}/stat", crate::config::sync_base(&cfg.sync_url));
    state.http().post_json(&url, &[], &body).await.map_err(|e| e.to_string())?;
    let day = today();
    state
        .config
        .update(|c| {
            c.install_id = id;
            c.stats_day = day;
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}
