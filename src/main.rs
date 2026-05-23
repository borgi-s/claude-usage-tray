#![windows_subsystem = "windows"]

use anyhow::Result;
use chrono::Utc;
use clap::Parser;
use claude_usage_tray::api::credentials::{load_from_default_path, Credentials};
use claude_usage_tray::api::usage::{fetch_usage, UsageBucket, UsageSnapshot};
use claude_usage_tray::cli::Cli;
use claude_usage_tray::render::format_duration;
use tracing_subscriber::EnvFilter;
use windows::Win32::Graphics::GdiPlus::{
    GdiplusShutdown, GdiplusStartup, GdiplusStartupInput, Status,
};

/// RAII guard that initializes GDI+ in `init()` and shuts it down on drop.
/// We hold one for the whole process lifetime so cleanup runs on every exit path
/// (including `?` early-returns and panic unwinding).
struct GdiplusGuard(usize);

impl GdiplusGuard {
    fn init() -> Result<Self> {
        let mut token: usize = 0;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        // SAFETY: token is on the stack and the input pointer is valid.
        // GdiplusStartup writes the token and returns a Status code.
        let status = unsafe { GdiplusStartup(&mut token, &input, std::ptr::null_mut()) };
        if status != Status(0) {
            anyhow::bail!("GdiplusStartup failed with status {:?}", status);
        }
        Ok(Self(token))
    }
}

impl Drop for GdiplusGuard {
    fn drop(&mut self) {
        // SAFETY: token was obtained from a successful GdiplusStartup and we are
        // the sole owner. After shutdown, no more GDI+ calls happen.
        unsafe { GdiplusShutdown(self.0) };
    }
}

fn main() -> Result<()> {
    // Attach to parent console (if any) so --once/--watch can still print to a terminal.
    // Harmlessly fails when launched from Explorer.
    let _ = unsafe {
        windows::Win32::System::Console::AttachConsole(
            windows::Win32::System::Console::ATTACH_PARENT_PROCESS,
        )
    };

    let cli = Cli::parse();

    let _gdiplus = GdiplusGuard::init()?;

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
