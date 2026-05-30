//! Headless Linux collector: refresh local turns, poll the usage API, and
//! upload to Supabase under the prefix from `.env` (e.g. `borgi-linux`).
//!
//! This is a SEPARATE binary from the Windows GUI (`src/main.rs`). It only uses
//! the platform-agnostic library modules, so it builds on x86_64-unknown-linux-gnu.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;

use claude_usage_tray::api::credentials::{load_from_default_path, Credentials};
use claude_usage_tray::data::cache;
use claude_usage_tray::log::calibration;
use claude_usage_tray::poll::poll_once;
use claude_usage_tray::shared::snapshot::AppSnapshot;
use claude_usage_tray::sync::Syncer;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Headless Claude Code usage collector (Linux server)."
)]
struct CollectorCli {
    /// Run one collect+upload cycle and exit (for testing).
    #[arg(long)]
    once: bool,

    /// Seconds between cycles in daemon mode. Keep >= 60 to respect the
    /// ~1 req/min usage-endpoint rate limit.
    #[arg(long, default_value_t = 120)]
    interval: u64,

    /// Log level: trace | debug | info | warn | error.
    #[arg(long, default_value = "info")]
    log_level: String,
}

fn main() -> Result<()> {
    let cli = CollectorCli::parse();
    init_tracing(&cli.log_level);

    let syncer = match Syncer::from_env() {
        Ok(Some(s)) => s,
        Ok(None) => {
            anyhow::bail!(
                "Supabase sync is not configured. Create a .env with SUPABASE_URL, \
                 SUPABASE_SERVICE_ROLE_KEY, and SUPABASE_USER_PREFIX in the working directory."
            );
        }
        Err(e) => return Err(e.context("invalid Supabase sync config")),
    };

    if cli.once {
        run_cycle(&syncer);
        return Ok(());
    }

    // Clamp to the documented 60s floor: the usage endpoint is rate-limited to
    // ~1 req/min, and an interval of 0 would otherwise busy-loop hammering it.
    let interval_secs = cli.interval.max(60);
    if cli.interval < 60 {
        tracing::warn!(
            requested = cli.interval,
            "interval below the 60s rate-limit floor; clamping to 60s"
        );
    }
    tracing::info!(interval_secs, "collector starting");
    let interval = Duration::from_secs(interval_secs);
    loop {
        let started = Instant::now();
        run_cycle(&syncer);
        let elapsed = started.elapsed();
        if elapsed < interval {
            std::thread::sleep(interval - elapsed);
        }
    }
}

/// One collect+upload cycle. Best-effort: every failure is logged, never fatal,
/// so the daemon keeps running.
fn run_cycle<S: claude_usage_tray::sync::storage::ObjectStore>(syncer: &Syncer<S>) {
    // 1. Refresh local turns (no token required).
    let turns = match cache::refresh() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "cache refresh failed; skipping this cycle");
            return;
        }
    };
    let turns_arc = Arc::new(turns);

    // 2. Try to load creds + poll the usage API. On any failure, fall back to a
    //    cache-only upload so we still push the local turns and never overwrite a
    //    good caps.json with empty data.
    let creds: Option<Credentials> = match load_from_default_path() {
        Ok(c) => Some(c),
        Err(e) => {
            tracing::warn!(error = %e, "credentials unavailable; uploading turns only");
            None
        }
    };

    let poll_ok = match &creds {
        Some(c) => match poll_once(c) {
            Ok(snap) => Some(snap),
            Err(e) => {
                tracing::warn!(error = %e, "usage poll failed; uploading turns only");
                None
            }
        },
        None => None,
    };

    match (poll_ok, &creds) {
        // Poll succeeded: upload all three objects (cache + caps + calibration).
        (Some(snap), Some(c)) => {
            let samples = calibration::read_all_default().unwrap_or_else(|e| {
                tracing::warn!(error = %e, "calibration log read failed; uploading empty samples");
                Vec::new()
            });
            let snapshot = AppSnapshot {
                turns: turns_arc,
                last_sample: Some((snap, chrono::Utc::now())),
                ..Default::default()
            };
            syncer.run_once(&snapshot, c, &samples);
            tracing::info!("cycle complete (full upload)");
        }
        // Poll failed or no creds: upload turns only.
        _ => {
            let snapshot = AppSnapshot {
                turns: turns_arc,
                ..Default::default()
            };
            syncer.upload_cache_only(&snapshot);
            tracing::info!("cycle complete (cache-only upload)");
        }
    }
}

fn init_tracing(level: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
}
