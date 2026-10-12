#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod audio_feat;
mod autostart;
mod bridge;
mod charts;
mod commands;
mod config;
mod deeplink;
mod db;
mod dedupe;
mod discord;
mod downloads;
mod error;
mod fingerprint;
mod geniusweb;
mod home;
mod autopause;
mod backup;
mod hotkeys;
mod viz;
mod rooms;
mod sync;
mod shared_playlist;
mod logging;
mod login;
mod models;
mod picks;
mod net;
mod player;
mod secrets;
mod state;
mod tray;
mod updater;
mod import;
mod lang;
mod lyrics_align;
mod mini;
mod smtc;
mod news;
mod upload;
mod wave;
mod wave_model;

use tauri::{Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

/// Passed by the OS autostart entry: start hidden in the tray.
const MINIMIZED_FLAG: &str = "--minimized";

fn main() {
    // the installer's shortcuts carry this id: with it Windows names the app
    // "SC Desk" (with its icon) in the media panel and in notifications
    #[cfg(windows)]
    // SAFETY: a static wide string, called once before any window exists
    unsafe {
        let _ = windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(windows::core::w!("dev.scdesk.app"));
    }
    let mut builder = tauri::Builder::default();
    if secrets::profile().is_none() {
        // second launch (e.g. from the Start menu while running in the tray) → show the window
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // a link (scdesk://…) goes to the running program; a plain second launch shows the window
            match deeplink::from_args(args) {
                Some(url) => deeplink::handle(app, url),
                None => tray::show_main(app),
            }
        }));
    }
    let app = builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![MINIMIZED_FLAG])))
        .plugin(hotkeys::plugin())
        .register_asynchronous_uri_scheme_protocol("cover", db::covers::protocol_handler)
        .setup(|app| {
            let handle = app.handle().clone();
            deeplink::register();
            if let Some(url) = deeplink::from_args(std::env::args()) {
                deeplink::handle(&handle, url);
            }
            app.manage(sync::service::Rooms::default());
            app.manage(sync::shared::SharedPlaylists::default());
            app.manage(sync::friends::Friends::default());
            let guard = logging::init(&app.path().app_log_dir()?)?;
            app.manage(logging::LogGuard(guard));
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting");

            let state = state::AppState::init(&handle)?;
            let start_hidden = state.config.get().start_minimized && std::env::args().any(|a| a == MINIMIZED_FLAG);
            app.manage(state);
            app.manage(player::Player::start(handle.clone())?);
            // the output device picked last time (it follows the system's if it is gone)
            if let Some(name) = app.state::<state::AppState>().config.get().output_device {
                app.state::<std::sync::Arc<player::Player>>().set_output_device(Some(name));
            }
            tray::build(&handle)?;
            hotkeys::register(&handle);
            updater::start(&handle);
            autostart::sync(&handle);
            commands::start_likes_watch(&handle);
            {
                // with a proxy the connection pool may have been opened while the proxy program (a VPN client that
                // starts later than this one) was not there yet: a fresh client a few seconds after the start, twice
                let app = handle.clone();
                tauri::async_runtime::spawn(async move {
                    if app.state::<state::AppState>().config.get().active_proxy().is_none() {
                        return;
                    }
                    for wait in [4u64, 20, 60] {
                        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                        match app.state::<state::AppState>().rebuild_http() {
                            Ok(()) => tracing::info!(after_s = wait, "http client rebuilt (proxy in use)"),
                            Err(e) => tracing::warn!(error = %e, "http client rebuild failed"),
                        }
                    }
                });
            }
            news::start(&handle);
            sync::watch::start(&handle);
            geniusweb::init(&handle);
            {
                let app = handle.clone();
                api::soundcloud::set_token_source(Box::new(move || {
                    let app = app.clone();
                    Box::pin(async move { bridge::session_token(&app).await })
                }));
            }
            charts::start_daily(&handle);
            autopause::start(handle.clone());
            {
                // the shared playlists we are in: reopened once the sign-in is known
                let app = handle.clone();
                tauri::async_runtime::spawn(async move {
                    // the sign-in is not known right at the start (or there is no network yet): try again for a while
                    for _ in 0..40 {
                        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                        if app.state::<state::AppState>().me().is_some() {
                            app.state::<sync::shared::SharedPlaylists>().start(&app).await;
                            app.state::<sync::friends::Friends>().start(&app).await;
                            break;
                        }
                    }
                });
            }
            {
                // lyrics cached before the search table existed
                let app = handle.clone();
                tauri::async_runtime::spawn(async move {
                    match app.state::<state::AppState>().db.lyrics_text_backfill().await {
                        Ok(n) => tracing::info!(rows = n, "lyrics search table filled"),
                        Err(e) => tracing::warn!(error = %e, "lyrics search table backfill failed"),
                    }
                });
            }
            // the window is created hidden (tauri.conf.json) to avoid a flash on autostart
            if !start_hidden {
                tray::show_main(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // closing the window keeps playback going in the tray
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    tray::hide_main(window.app_handle());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::auth_status,
            commands::auth_verify,
            commands::auth_save,
            commands::auth_login,
            commands::auth_clear,
            commands::likes_sync,
            commands::likes_count,
            commands::likes_page,
            commands::likes_ids,
            commands::like_set,
            commands::likes_pending,
            commands::likes_pending_flush,
            commands::follows_pending,
            commands::downloads_list,
            commands::download_track,
            commands::download_many,
            commands::download_many_cancel,
            commands::download_remove,
            commands::downloads_open,
            commands::downloads_dir,
            commands::downloads_pick_dir,
            commands::library_playlists,
            commands::library_artists,
            commands::library_counts,
            commands::follow_set,
            commands::history_page,
            commands::history_clear,
            commands::history_playlist_add,
            commands::history_playlists,
            commands::search_tracks,
            commands::search_all,
            commands::search_users,
            commands::search_playlists,
            commands::track_page,
            commands::artist_get,
            commands::artist_section,
            commands::playlist_page,
            commands::home_sections,
            commands::mix_page,
            commands::category_tracks,
            commands::category_covers,
            commands::picks,
            commands::chart_page,
            commands::chart_years,
            #[cfg(debug_assertions)]
            commands::debug_fp_compare,
            #[cfg(debug_assertions)]
            commands::debug_news_check,
            commands::feed_page,
            commands::player_play_likes,
            commands::player_play_tracks,
            commands::player_enqueue,
            commands::player_toggle,
            commands::player_next,
            commands::player_prev,
            commands::player_seek,
            commands::player_set_volume,
            commands::player_set_shuffle,
            commands::player_set_repeat,
            commands::player_snapshot,
            commands::player_position,
            commands::player_upcoming,
            commands::wave_start,
            commands::wave_start_from,
            commands::wave_start_playlist,
            commands::playlist_add_track,
            commands::playlist_remove_track,
            commands::playlist_create,
            commands::captcha_waiting,
            commands::fx_set,
            commands::speed_preset,
            commands::sleep_set,
            commands::sleep_get,
            commands::queue_get,
            commands::news_set,
            commands::rooms_notify_set,
            commands::stats_get,
            commands::mini_open,
            commands::mini_close,
            commands::mini_resize,
            commands::import_preview,
            commands::import_run,
            commands::queue_move,
            commands::queue_remove,
            commands::queue_clear,
            commands::queue_play,
            commands::playlist_set_tracks,
            commands::upload_pick,
            commands::upload_track,
            commands::upload_delete,
            commands::my_tracks,
            commands::player_set_smart_shuffle,
            commands::wave_set_mood,
            commands::wave_info,
            commands::wave_set_no_liked,
            commands::dislike_set,
            commands::disliked_ids,
            commands::disliked_tracks,
            commands::wave_dislike_artist,
            commands::wave_disliked_artists,
            commands::wave_undislike_artist,
            commands::wave_clear_dislikes,
            commands::lyrics_get,
            commands::open_external,
            commands::config_get,
            commands::config_set_network,
            commands::net_check,
            commands::config_set_ui,
            commands::hotkeys_apply,
            commands::sp_list,
            commands::sp_create,
            commands::sp_join,
            commands::sp_leave,
            commands::sp_get,
            commands::sp_add,
            commands::sp_remove,
            commands::sp_move,
            commands::sp_access,
            commands::sp_title,
            commands::sp_cover,
            commands::sp_invite,
            commands::sp_remove_member,
            commands::sp_play,
            commands::sync_url_get,
            commands::sync_url_set,
            commands::room_create,
            commands::room_join,
            commands::room_leave,
            commands::deeplink_take,
            commands::discord_section,
            commands::rooms_mine,
            commands::room_enter,
            commands::room_forget,
            commands::room_view,
            commands::room_cmd,
            commands::rooms_live,
            commands::theme_bg_get,
            commands::theme_bg_set,
            commands::theme_bg_delete,
            commands::friends_list,
            commands::friend_ids,
            commands::friend_request,
            commands::friend_accept,
            commands::friend_decline,
            commands::friend_cancel,
            commands::friend_remove,
            commands::friend_invite,
            commands::friend_invite_dismiss,
            commands::autopause_get,
            commands::autopause_set,
            commands::backup_export,
            commands::backup_import,
            commands::log_tail,
            commands::likes_search_lyrics,
            commands::lyrics_index_status,
            commands::lyrics_index_run,
            commands::viz_set,
            commands::eq_set,
            commands::audio_devices,
            commands::audio_device_set,
            commands::eq_device_get,
            commands::eq_per_device_set,
            commands::discord_set,
            commands::system_get,
            commands::system_set,
            updater::update_check,
            updater::update_pending,
            updater::update_dismiss,
            updater::update_install,
            updater::app_version,
        ])
        .build(tauri::generate_context!());

    match app {
        // the main window is destroyed in tray mode: keep running until "Выход" (app.exit)
        Ok(app) => app.run(|_, event| {
            if let tauri::RunEvent::ExitRequested { api, code: None, .. } = event {
                api.prevent_exit();
            }
        }),
        Err(e) => {
            eprintln!("fatal: {e}");
            tracing::error!(error = %e, "fatal startup error");
            std::process::exit(1);
        }
    }
}
