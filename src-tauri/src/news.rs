//! «Новинки подписок»: a quiet Windows notification when a followed artist
//! posts a new track (reposts don't count).
//!
//! Light: 2 minutes after start, then once an hour, one request to the feed.
//! The first check only remembers where the feed is, so nobody gets a flood
//! of old tracks; after that only what appeared since the last check.

use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::{home, lang::pick, state::AppState};

const FIRST_CHECK: Duration = Duration::from_secs(120);
const EVERY: Duration = Duration::from_secs(60 * 60);
/// RFC 3339 of the newest post already seen
const SEEN_KEY: &str = "news:seen";
/// tracks listed in the summary notification
const SHOWN: usize = 3;

pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            let state = app.state::<AppState>();
            if state.config.get().notify_new && state.credentials().is_some() {
                if let Err(e) = check(&app, &state).await {
                    tracing::debug!(error = %e, "news check failed");
                }
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}

async fn check(app: &AppHandle, state: &AppState) -> crate::error::AppResult<()> {
    let me = state.current_user().await?;
    let page = state.sc()?.stream(None).await?;
    let (items, _) = home::feed_items(&page);
    // the artists' own posts, newest first; RFC 3339 strings sort as times
    let mut posts: Vec<_> = items.into_iter().filter(|i| i.reposted_by.is_empty() && i.track.user_id != me.id && !i.at.is_empty()).collect();
    posts.sort_by(|a, b| b.at.cmp(&a.at));
    let Some(newest) = posts.first().map(|p| p.at.clone()) else { return Ok(()) };
    let seen = state.db.kv_get::<String>(SEEN_KEY).await?.map(|(v, _)| v);
    state.db.kv_put(SEEN_KEY, &newest).await?;
    let Some(seen) = seen else {
        tracing::info!("news: first check, remembering the feed");
        return Ok(());
    };
    let fresh: Vec<_> = posts.into_iter().filter(|p| p.at > seen).collect();
    if fresh.is_empty() {
        return Ok(());
    }
    tracing::info!(n = fresh.len(), "news: new tracks from followings");
    // always one notification: several at once pile up and Windows shows one banner
    if let [p] = fresh.as_slice() {
        show(app, &format!("{} · {}", pick("Новый трек", "New track"), p.track.artist), &p.track.title);
    } else {
        let mut lines: Vec<String> = fresh.iter().take(SHOWN).map(|p| format!("{} - {}", p.track.artist, p.track.title)).collect();
        if fresh.len() > SHOWN {
            lines.push(format!("{} {}", pick("и ещё", "and"), fresh.len() - SHOWN));
        }
        show(app, &format!("{}: {}", pick("Новые треки подписок", "New tracks from followings"), fresh.len()), &lines.join("\n"));
    }
    Ok(())
}

/// Debug builds only: pretend the last check was at `since` (RFC 3339) and check now.
#[cfg(debug_assertions)]
pub async fn debug_check(app: &AppHandle, since: String) -> crate::error::AppResult<()> {
    let state = app.state::<AppState>();
    state.db.kv_put(SEEN_KEY, &since).await?;
    check(app, &state).await
}

fn show(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!(error = %e, "notification failed");
    }
}
