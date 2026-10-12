//! "A friend opened a room": a quiet notification.
//!
//! Every 90 s one request to the relay's directory with the people you follow (their list is the one
//! the Library already keeps). The first look only remembers who is live, so opening the app
//! does not announce rooms that were open for hours.

use std::{collections::HashSet, time::Duration};

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::{lang::pick, models::UserDto, state::AppState};

const FIRST: Duration = Duration::from_secs(60);
const EVERY: Duration = Duration::from_secs(90);

pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST).await;
        let mut seen: Option<HashSet<String>> = None;
        loop {
            let state = app.state::<AppState>();
            if state.config.get().notify_rooms && state.credentials().is_some() {
                match look(&state).await {
                    Ok(rooms) => {
                        let now: HashSet<String> = rooms.iter().map(|r| r.channel.clone()).collect();
                        if let Some(before) = &seen {
                            for r in rooms.iter().filter(|r| !before.contains(&r.channel)) {
                                tracing::info!(room = %r.name, "friend opened a room");
                                let title = format!("{} {}", r.host_name, pick("открыл комнату", "opened a room"));
                                let _ = app.notification().builder().title(title).body(&r.name).show();
                            }
                        }
                        seen = Some(now);
                    }
                    Err(e) => tracing::debug!(error = %e, "rooms watch failed"),
                }
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}

async fn look(state: &AppState) -> crate::error::AppResult<Vec<super::service::LiveRoom>> {
    let people = state.db.kv_get::<Vec<UserDto>>("lib:artists").await?.map(|(v, _)| v).unwrap_or_default();
    let me = state.me().map(|m| m.id);
    let ids: Vec<u64> = people.iter().map(|u| u.id).filter(|id| Some(*id) != me).collect();
    super::service::live_rooms(state, ids).await
}
