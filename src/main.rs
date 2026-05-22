use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use claude_usage_tray::api::credentials::{load_from_default_path, Credentials};
use claude_usage_tray::api::usage::{fetch_usage, UsageBucket, UsageSnapshot};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }
    if !args.iter().any(|a| a == "--once") {
        eprintln!("Stage 1 only supports --once. Use --help to see usage.");
        std::process::exit(2);
    }

    let creds = load_from_default_path()?;
    let snap = fetch_usage(&creds)?;
    print_snapshot(&snap, &creds);
    Ok(())
}

fn print_help() {
    println!(
        "claude-usage-tray v{}\n\
         \n\
         USAGE:\n  \
             claude-usage-tray --once\n  \
             claude-usage-tray --help\n\
         \n\
         Reads OAuth from ~/.claude/.credentials.json and queries Anthropic's\n\
         /api/oauth/usage endpoint, printing current 5h and 7d utilization.\n",
        env!("CARGO_PKG_VERSION"),
    );
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
