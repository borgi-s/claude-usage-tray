use clap::{ArgGroup, Parser, ValueEnum};

/// Polling interval choices for `--watch`. Values constrained to keep us
/// above the ~1 req/min rate limit of the /api/oauth/usage endpoint.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum Interval {
    #[value(name = "60")]
    I60,
    #[value(name = "120")]
    I120,
    #[value(name = "300")]
    I300,
}

impl Interval {
    pub fn as_secs(self) -> u64 {
        match self {
            Self::I60 => 60,
            Self::I120 => 120,
            Self::I300 => 300,
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Native Windows tray widget for Claude Code usage tracking."
)]
#[command(group(
    ArgGroup::new("mode")
        .required(true)
        .multiple(false)
        .args(["once", "watch"])
))]
pub struct Cli {
    /// Fetch once, print, exit.
    #[arg(long)]
    pub once: bool,

    /// Loop forever with a live-redraw view in the terminal.
    #[arg(long)]
    pub watch: bool,

    /// Polling interval (only used with --watch). One of: 60, 120, 300.
    #[arg(long, value_enum, default_value_t = Interval::I120)]
    pub interval: Interval,

    /// Log level: trace | debug | info | warn | error.
    #[arg(long, default_value = "info")]
    pub log_level: String,
}
