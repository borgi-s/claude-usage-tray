//! GitHub-release update checking. See the Stage 6.5 design spec.

pub mod github;
pub mod version;

pub use github::{fetch_latest_release, parse_release, ReleaseInfo, UpdateError};

use semver::Version;
use std::time::{Duration, Instant};

/// The currently-running version, from the compiled-in Cargo metadata.
pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("CARGO_PKG_VERSION must be valid semver")
}

/// Outcome of a single update check.
#[derive(Debug, Clone)]
pub struct UpdateCheck {
    pub latest: ReleaseInfo,
    /// `latest.version` is strictly newer than the running version.
    pub is_newer: bool,
    /// True if this check was triggered manually ("Check for updates now").
    pub manual: bool,
}

/// Event sent from a checking thread to the UI thread. `notify` is computed by
/// the sender (the sole owner of state.json), so the UI thread only renders.
#[derive(Debug, Clone)]
pub enum UpdateEvent {
    Result { check: UpdateCheck, notify: bool },
    Failed { manual: bool, msg: String },
}

/// Fetch the latest release and compare against `current`.
pub fn check_latest(current: &Version, manual: bool) -> Result<UpdateCheck, UpdateError> {
    let body = fetch_latest_release()?;
    let latest = parse_release(&body)?;
    let is_newer = version::is_update_available(current, &latest.version);
    Ok(UpdateCheck {
        latest,
        is_newer,
        manual,
    })
}

/// Rolling-window rate limit for manual checks: at most `MANUAL_CHECK_MAX` within
/// `MANUAL_CHECK_WINDOW`. Prunes expired timestamps, and if there is room, records
/// `now` and returns `true`; otherwise returns `false` without recording.
pub const MANUAL_CHECK_WINDOW: Duration = Duration::from_secs(3600);
pub const MANUAL_CHECK_MAX: usize = 5;

pub fn manual_check_allowed(history: &mut Vec<Instant>, now: Instant) -> bool {
    history.retain(|t| now.saturating_duration_since(*t) < MANUAL_CHECK_WINDOW);
    if history.len() < MANUAL_CHECK_MAX {
        history.push(now);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_five_then_blocks_sixth() {
        let base = Instant::now();
        let mut hist = Vec::new();
        for i in 0u64..5 {
            assert!(
                manual_check_allowed(&mut hist, base + Duration::from_secs(i * 60)),
                "check {i} should be allowed"
            );
        }
        // 6th within the hour is blocked.
        assert!(!manual_check_allowed(&mut hist, base + Duration::from_secs(5 * 60)));
    }

    #[test]
    fn allows_again_after_window_expires() {
        let base = Instant::now();
        let mut hist = Vec::new();
        for i in 0u64..5 {
            assert!(manual_check_allowed(&mut hist, base + Duration::from_secs(i)));
        }
        assert!(!manual_check_allowed(&mut hist, base + Duration::from_secs(10)));
        // Past the 1h window, all prior entries are evicted → allowed again.
        assert!(manual_check_allowed(&mut hist, base + Duration::from_secs(3601)));
    }

    #[test]
    fn current_version_parses() {
        // Must not panic.
        let _ = current_version();
    }
}
