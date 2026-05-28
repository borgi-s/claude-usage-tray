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

#[derive(Debug, Clone, Default)]
pub struct UsageBucket {
    /// Normalized to 0.0-1.0 (Anthropic returns 0-100 percentage; we divide by 100).
    pub utilization: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
pub struct UsageSnapshot {
    pub five_hour: Option<UsageBucket>,
    pub seven_day: Option<UsageBucket>,
}

fn convert(b: RawBucket) -> Option<UsageBucket> {
    let util = b.utilization?;
    Some(UsageBucket {
        utilization: util / 100.0,
        resets_at: b.resets_at.and_then(|s| {
            DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        }),
    })
}

pub fn parse_usage_response(raw: &str) -> Result<UsageSnapshot> {
    let r: RawResponse = serde_json::from_str(raw).context("invalid usage JSON")?;
    Ok(UsageSnapshot {
        five_hour: r.five_hour.and_then(convert),
        seven_day: r.seven_day.and_then(convert),
    })
}

use crate::api::credentials::Credentials;
use thiserror::Error;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const ANTHROPIC_BETA: &str = "oauth-2025-04-20";

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("rate-limited by usage endpoint (HTTP 429)")]
    RateLimited,
    #[error("usage endpoint rejected the token (HTTP {0}); re-authenticate with `claude login`")]
    Unauthorized(u16),
    #[error("usage endpoint returned HTTP {0}")]
    Http(u16),
    #[error("network error: {0}")]
    Network(String),
    #[error("response parsing failed: {0}")]
    Parse(String),
}

pub fn fetch_usage(creds: &Credentials) -> Result<UsageSnapshot, FetchError> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .build();

    let req = agent
        .get(USAGE_URL)
        .set("Authorization", &format!("Bearer {}", creds.access_token))
        .set("anthropic-beta", ANTHROPIC_BETA)
        .set("Accept", "application/json")
        .set(
            "User-Agent",
            &format!("claude-usage-tray/{}", env!("CARGO_PKG_VERSION")),
        );

    let response = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(429, _)) => return Err(FetchError::RateLimited),
        // 401/403 means the OAuth token is expired/revoked server-side — surface
        // an actionable re-auth hint instead of a bare "HTTP 401".
        Err(ureq::Error::Status(code @ (401 | 403), _)) => {
            return Err(FetchError::Unauthorized(code))
        }
        Err(ureq::Error::Status(code, _)) => return Err(FetchError::Http(code)),
        Err(ureq::Error::Transport(t)) => return Err(FetchError::Network(t.to_string())),
    };

    let body = response
        .into_string()
        .map_err(|e| FetchError::Network(e.to_string()))?;
    parse_usage_response(&body).map_err(|e| FetchError::Parse(e.to_string()))
}
