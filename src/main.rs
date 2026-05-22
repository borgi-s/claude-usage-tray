use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use clap::Parser;
use claude_usage_tray::api::credentials::{load_from_default_path, Credentials};
use claude_usage_tray::api::usage::{fetch_usage, UsageBucket, UsageSnapshot};
use claude_usage_tray::cli::Cli;

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.once {
        run_once()?;
    } else if cli.watch {
        anyhow::bail!("--watch not yet implemented (Task 7 wires this up)");
    }
    Ok(())
}

fn run_once() -> Result<()> {
    let creds = load_from_default_path()?;
    let snap = fetch_usage(&creds)?;
    print_snapshot(&snap, &creds);
    Ok(())
}

fn print_snapshot(snap: &UsageSnapshot, creds: &Credentials) {
    let now = Utc::now();
    if let Some(b) = &snap.five_hour {
        println!("5h: {}", format_bucket(b, now));
    } else {
        println!("5h: (no data)");
    }
    if let Some(b) = &snap.seven_day {
        println!("7d: {}", format_bucket(b, now));
    } else {
        println!("7d: (no data)");
    }
    println!(
        "sub: {} / tier: {}",
        creds.subscription_type, creds.rate_limit_tier
    );
}

fn format_bucket(b: &UsageBucket, now: DateTime<Utc>) -> String {
    let pct = (b.utilization * 100.0).round() as i64;
    match b.resets_at {
        Some(when) => format!("{}% (resets in {})", pct, format_duration(when - now)),
        None => format!("{}% (no reset time)", pct),
    }
}

fn format_duration(d: Duration) -> String {
    let secs = d.num_seconds().max(0);
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3600;
    let mins = (secs % 3600) / 60;
    if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    }
}
