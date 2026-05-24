//! GitHub Releases API client. `parse_release` is pure (unit-tested against a
//! fixture); `fetch_latest_release` performs the network call (not unit-tested).

use crate::updater::version::parse_tag;
use semver::Version;
use serde::Deserialize;
use std::time::Duration;
use thiserror::Error;

const LATEST_URL: &str = "https://api.github.com/repos/borgi-s/claude-usage-tray/releases/latest";

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("github request failed: {0}")]
    Http(String),
    #[error("github response parsing failed: {0}")]
    Parse(String),
    #[error("could not parse release tag: {0}")]
    Version(String),
}

/// Parsed subset of a GitHub release we care about.
#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub tag: String,
    pub version: Version,
    pub html_url: String,
}

#[derive(Debug, Deserialize)]
struct RawRelease {
    tag_name: String,
    html_url: String,
}

/// Parse a `releases/latest` JSON body into a `ReleaseInfo`. Pure.
pub fn parse_release(json: &str) -> Result<ReleaseInfo, UpdateError> {
    let raw: RawRelease =
        serde_json::from_str(json).map_err(|e| UpdateError::Parse(e.to_string()))?;
    let version = parse_tag(&raw.tag_name).map_err(|e| UpdateError::Version(e.to_string()))?;
    Ok(ReleaseInfo {
        tag: raw.tag_name,
        version,
        html_url: raw.html_url,
    })
}

/// GET the latest release JSON from GitHub. GitHub 403s without a User-Agent;
/// `releases/latest` already excludes drafts and prereleases.
pub fn fetch_latest_release() -> Result<String, UpdateError> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();
    let resp = agent
        .get(LATEST_URL)
        .set(
            "User-Agent",
            &format!("claude-usage-tray/{}", env!("CARGO_PKG_VERSION")),
        )
        .set("Accept", "application/vnd.github+json")
        .call();
    match resp {
        Ok(r) => r
            .into_string()
            .map_err(|e| UpdateError::Http(e.to_string())),
        Err(ureq::Error::Status(code, _)) => Err(UpdateError::Http(format!("HTTP {code}"))),
        Err(ureq::Error::Transport(t)) => Err(UpdateError::Http(t.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_release_extracts_tag_and_url() {
        let json = include_str!("../../tests/fixtures/github_release_latest.json");
        let info = parse_release(json).unwrap();
        assert_eq!(info.tag, "v0.8.0");
        assert_eq!(info.version, Version::new(0, 8, 0));
        assert_eq!(
            info.html_url,
            "https://github.com/borgi-s/claude-usage-tray/releases/tag/v0.8.0"
        );
    }

    #[test]
    fn parse_release_rejects_bad_json() {
        assert!(matches!(
            parse_release("{ nope"),
            Err(UpdateError::Parse(_))
        ));
    }

    #[test]
    fn parse_release_rejects_bad_tag() {
        let json = r#"{"tag_name":"banana","html_url":"http://x"}"#;
        assert!(matches!(parse_release(json), Err(UpdateError::Version(_))));
    }
}
