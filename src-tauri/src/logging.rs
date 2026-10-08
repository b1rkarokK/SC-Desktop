use std::path::Path;

use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::error::{AppError, AppResult};

/// Keeps the non-blocking writer alive for the whole app lifetime.
pub struct LogGuard(#[allow(dead_code)] pub WorkerGuard);

/// Daily rotating log files, 7 kept. Level via `SC_DESK_LOG` (EnvFilter syntax).
pub fn init(dir: &Path) -> AppResult<WorkerGuard> {
    std::fs::create_dir_all(dir)?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("sc-desk")
        .filename_suffix("log")
        .max_log_files(7)
        .build(dir)
        .map_err(|e| AppError::Other(format!("log init: {e}")))?;
    let (writer, guard) = tracing_appender::non_blocking(appender);

    let filter = EnvFilter::try_from_env("SC_DESK_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,sc_desk=debug,symphonia=warn"));
    let file_layer = fmt::layer().with_writer(writer).with_ansi(false).with_target(true);
    let stderr_layer = cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr));

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stderr_layer)
        .try_init()
        .map_err(|e| AppError::Other(format!("log init: {e}")))?;
    Ok(guard)
}
