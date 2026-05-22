use crate::paths;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;

/// Initialize a file-backed tracing subscriber for tray mode.
///
/// Writes to `~/.claude-usage-tray/tray.YYYY-MM-DD.log` with daily rotation,
/// keeping the last 7 days. The returned `WorkerGuard` must be held for the
/// lifetime of the process — dropping it flushes the appender.
pub fn init_file_subscriber(level: &str) -> anyhow::Result<WorkerGuard> {
    let dir = paths::app_dir()?;
    std::fs::create_dir_all(&dir)?;

    let appender: RollingFileAppender = tracing_appender::rolling::Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("tray")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&dir)
        .map_err(|e| anyhow::anyhow!("could not build log appender: {e}"))?;

    let (non_blocking, guard) = tracing_appender::non_blocking(appender);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(non_blocking)
        .with_target(false)
        .with_ansi(false)
        .init();

    Ok(guard)
}
