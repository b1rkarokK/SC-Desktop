//! "Войти через SoundCloud": opens soundcloud.com/signin in a separate window
//! (system WebView, the user signs in as usual — captcha and 2FA included),
//! then picks up the user's own `oauth_token` session cookie and the public
//! web `client_id`, verifies them via `/me` and stores them in the keyring.
//!
//! The login window has no IPC access: it is not listed in capabilities.

use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use url::Url;

use crate::{
    api::soundcloud::{discover_client_id, SoundCloud},
    commands::auth_status_of,
    error::{AppError, AppResult},
    secrets::{self, Credentials},
    state::AppState,
};

const LABEL: &str = "sc-login";
const POLL: Duration = Duration::from_millis(1000);
const GIVE_UP_AFTER: Duration = Duration::from_secs(15 * 60);

pub fn open(app: &AppHandle) -> AppResult<()> {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.set_focus();
        return Ok(());
    }
    let url = Url::parse("https://soundcloud.com/signin")?;
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url))
        .title("Вход в SoundCloud")
        // SoundCloud's desktop layout needs ~860px, narrower windows scroll sideways
        .inner_size(900.0, 780.0)
        .min_inner_size(860.0, 600.0)
        .background_color(tauri::window::Color(18, 18, 18, 255))
        // "Continue with Google / Facebook / Apple" opens an OAuth popup; allow it
        // (only in this window) so window.opener keeps working.
        .on_new_window(|_url, _features| tauri::webview::NewWindowResponse::Allow)
        .center()
        .build()?;
    let _ = window.set_focus();

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = watch(&app).await {
            tracing::warn!(error = %e, "login failed");
            let _ = app.emit("player:error", &e);
        }
    });
    Ok(())
}

async fn watch(app: &AppHandle) -> AppResult<()> {
    let sc_url = Url::parse("https://soundcloud.com/")?;
    let started = std::time::Instant::now();
    let token = loop {
        tokio::time::sleep(POLL).await;
        let Some(window) = app.get_webview_window(LABEL) else {
            return Ok(()); // user closed the window
        };
        if started.elapsed() > GIVE_UP_AFTER {
            let _ = window.close();
            return Err(AppError::Other("Время входа истекло".into()));
        }
        // must not run on the main thread on Windows: we are in an async task
        let cookies = window.cookies_for_url(sc_url.clone()).unwrap_or_default();
        if let Some(c) = cookies.iter().find(|c| c.name() == "oauth_token" && !c.value().is_empty()) {
            break c.value().to_owned();
        }
    };

    let state = app.state::<AppState>();
    let http = state.http();
    let client_id = discover_client_id(&http).await?;
    let genius_token = state.credentials().and_then(|c| c.genius_token);
    let creds = Credentials { client_id, oauth_token: token, genius_token };
    let me = SoundCloud::new(http, &creds).me().await?;

    let to_save = creds.clone();
    tauri::async_runtime::spawn_blocking(move || secrets::save(&to_save))
        .await
        .map_err(|e| AppError::Keyring(e.to_string()))??;
    state.set_credentials(Some(creds));
    tracing::info!(user = %me.username, "signed in via login window");
    state.set_me(me);

    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.close();
    }
    let _ = app.emit("auth:changed", auth_status_of(&state));
    crate::tray::show_main(app);
    Ok(())
}
