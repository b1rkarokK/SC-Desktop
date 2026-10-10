#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod autostart;
mod bridge;
mod charts;
mod commands;
mod config;
mod db;
mod dedupe;
mod discord;
mod downloads;
mod error;
mod fingerprint;
mod geniusweb;
mod home;
mod hotkeys;
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
mod mini;
mod smtc;
mod news;
mod upload;
mod wave;

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
    let app = tauri::Builder::default()
        // second launch (e.g. from the Start menu while running in the tray) → show the window
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_main(app)))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![MINIMIZED_FLAG])))
        .plugin(hotkeys::plugin())
        .register_asynchronous_uri_scheme_protocol("cover", db::covers::protocol_handler)
        .setup(|app| {
            let handle = app.handle().clone();
            let guard = logging::init(&app.path().app_log_dir()?)?;
            app.manage(logging::LogGuard(guard));
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting");

            let state = state::AppState::init(&handle)?;
            let start_hidden = state.config.get().start_minimized && std::env::args().any(|a| a == MINIMIZED_FLAG);
            app.manage(state);
            app.manage(player::Player::start(handle.clone())?);
            tray::build(&handle)?;
            hotkeys::register(&handle);
            updater::start(&handle);
            autostart::sync(&handle);
            commands::start_likes_watch(&handle);
            news::start(&handle);
            geniusweb::init(&handle);
            {
                let app = handle.clone();
                api::soundcloud::set_token_source(Box::new(move || {
                    let app = app.clone();
                    Box::pin(async move { bridge::session_token(&app).await })
                }));
            }
            charts::start_daily(&handle);
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
            commands::sleep_set,
            commands::sleep_get,
            commands::queue_get,
            commands::news_set,
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
            commands::eq_set,
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
