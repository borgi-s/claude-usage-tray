#![windows_subsystem = "windows"]

use anyhow::Result;
use chrono::Utc;
use clap::Parser;
use claude_usage_tray::api::credentials::{load_from_default_path, Credentials};
use claude_usage_tray::api::usage::{fetch_usage, UsageBucket, UsageSnapshot};
use claude_usage_tray::cli::Cli;
use claude_usage_tray::render::format_duration;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    // Attach to parent console (if any) so --once/--watch can still print to a terminal.
    // Harmlessly fails when launched from Explorer.
    let _ = unsafe {
        windows::Win32::System::Console::AttachConsole(
            windows::Win32::System::Console::ATTACH_PARENT_PROCESS,
        )
    };

    let cli = Cli::parse();

    if cli.once {
        init_tracing_stderr(&cli.log_level);
        run_once()?;
    } else if cli.watch {
        init_tracing_stderr(&cli.log_level);
        claude_usage_tray::watch::run(cli.interval.as_secs())?;
    } else {
        let _guard = claude_usage_tray::log::tray::init_file_subscriber(&cli.log_level)?;
        claude_usage_tray::tray::run(cli.interval.as_secs())?;
        // _guard drops at end of this branch → tracing-appender flushes pending events.
    }
    Ok(())
}

fn init_tracing_stderr(level: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
}

fn run_once() -> Result<()> {
    let creds = load_from_default_path()?;
    let snap = fetch_usage(&creds)?;
    tracing::info!(
        five_hour = ?snap.five_hour.as_ref().map(|b| b.utilization),
        seven_day = ?snap.seven_day.as_ref().map(|b| b.utilization),
        "fetched usage snapshot"
    );
    print_snapshot(&snap, &creds);
    Ok(())
}

fn print_snapshot(snap: &UsageSnapshot, creds: &Credentials) {
    let now = Utc::now();
    if let Some(b) = &snap.five_hour {
        println!("5h: {}", format_one(b, now));
    } else {
        println!("5h: (no data)");
    }
    if let Some(b) = &snap.seven_day {
        println!("7d: {}", format_one(b, now));
    } else {
        println!("7d: (no data)");
    }
    println!(
        "sub: {} / tier: {}",
        creds.subscription_type, creds.rate_limit_tier
    );
}

fn format_one(b: &UsageBucket, now: chrono::DateTime<Utc>) -> String {
    let pct = (b.utilization * 100.0).round() as i64;
    match b.resets_at {
        Some(when) => format!("{}% (resets in {})", pct, format_duration(when - now)),
        None => format!("{}% (no reset time)", pct),
    }
}
