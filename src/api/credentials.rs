use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Deserialize)]
struct CredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: OAuthBlock,
}

#[derive(Debug, Deserialize)]
struct OAuthBlock {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
    #[serde(rename = "subscriptionType", default)]
    subscription_type: Option<String>,
    #[serde(rename = "rateLimitTier", default)]
    rate_limit_tier: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Credentials {
    pub access_token: String,
    pub subscription_type: String,
    pub rate_limit_tier: String,
}

pub fn parse_credentials(raw: &str) -> Result<Credentials> {
    let file: CredentialsFile = serde_json::from_str(raw).context("invalid credentials JSON")?;
    let oauth = file.claude_ai_oauth;

    let access_token = oauth.access_token.context("missing accessToken in credentials.json")?;

    if let Some(expires_at_ms) = oauth.expires_at {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        if expires_at_ms < now_ms {
            bail!("OAuth access token is expired. Run any Claude Code command to refresh it.");
        }
    }

    Ok(Credentials {
        access_token,
        subscription_type: oauth.subscription_type.unwrap_or_else(|| "unknown".to_string()),
        rate_limit_tier: oauth.rate_limit_tier.unwrap_or_else(|| "unknown".to_string()),
    })
}

/// Convenience: read the file from disk and parse. Used by main; tests pass strings directly.
pub fn load_from_default_path() -> Result<Credentials> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    let path = home.join(".claude").join(".credentials.json");
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("could not read {}", path.display()))?;
    parse_credentials(&raw)
}
