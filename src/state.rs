//! Small persistent app state in ~/.claude-usage-tray/state.json.
//!
//! Single-writer invariant: only the poller thread writes this file (see the
//! Stage 6.5 design spec). `load` degrades to defaults on any read/parse error
//! so a missing or corrupt file never blocks startup.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    #[serde(default)]
    pub update: UpdateState,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateState {
    /// UTC time of the last GitHub check. Gates the daily auto-check.
    #[serde(default)]
    pub last_check: Option<DateTime<Utc>>,
    /// The version we last fired a balloon for, e.g. "0.8.0". Guards once-per-version balloons.
    #[serde(default)]
    pub last_notified_version: Option<String>,
}

/// Load state from `path`. Returns `AppState::default()` if the file is missing
/// or cannot be parsed (logging a warning in the corrupt case).
pub fn load_from(path: &Path) -> AppState {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            tracing::warn!(error = %e, path = %path.display(), "state.json corrupt; using defaults");
            AppState::default()
        }),
        // Missing file is the normal first-run case — no warning.
        Err(_) => AppState::default(),
    }
}

/// Write state to `path` (pretty JSON), creating the parent directory if needed.
pub fn save_to(path: &Path, state: &AppState) -> anyhow::Result<()> {
    crate::paths::ensure_parent_dir(path)?;
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Convenience: load from the default `~/.claude-usage-tray/state.json`.
pub fn load() -> AppState {
    match crate::paths::state_path() {
        Ok(p) => load_from(&p),
        Err(e) => {
            tracing::warn!(error = %e, "could not resolve state.json path; using defaults");
            AppState::default()
        }
    }
}

/// Convenience: save to the default `~/.claude-usage-tray/state.json`.
pub fn save(state: &AppState) -> anyhow::Result<()> {
    let path = crate::paths::state_path()?;
    save_to(&path, state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_through_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut s = AppState::default();
        s.update.last_check = Some("2026-05-24T12:00:00Z".parse().unwrap());
        s.update.last_notified_version = Some("0.8.0".to_string());
        save_to(&path, &s).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded, s);
    }

    #[test]
    fn missing_file_is_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does_not_exist.json");
        assert_eq!(load_from(&path), AppState::default());
    }

    #[test]
    fn corrupt_file_is_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, "{ not valid json").unwrap();
        assert_eq!(load_from(&path), AppState::default());
    }
}
