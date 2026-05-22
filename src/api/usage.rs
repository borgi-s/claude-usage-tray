use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RawResponse {
    five_hour: Option<RawBucket>,
    seven_day: Option<RawBucket>,
}

#[derive(Debug, Deserialize)]
struct RawBucket {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UsageBucket {
    /// Normalized to 0.0-1.0 (Anthropic returns 0-100 percentage; we divide by 100).
    pub utilization: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct UsageSnapshot {
    pub five_hour: Option<UsageBucket>,
    pub seven_day: Option<UsageBucket>,
}

fn convert(b: RawBucket) -> Option<UsageBucket> {
    let util = b.utilization?;
    Some(UsageBucket {
        utilization: util / 100.0,
        resets_at: b.resets_at.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc))),
    })
}

pub fn parse_usage_response(raw: &str) -> Result<UsageSnapshot> {
    let r: RawResponse = serde_json::from_str(raw).context("invalid usage JSON")?;
    Ok(UsageSnapshot {
        five_hour: r.five_hour.and_then(convert),
        seven_day: r.seven_day.and_then(convert),
    })
}
