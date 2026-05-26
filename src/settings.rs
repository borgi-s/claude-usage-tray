//! Runtime, user-editable configuration. Persisted to
//! `~/.claude-usage-tray/settings.toml`. Defaults mirror the compile-time
//! consts in `crate::config`, so an absent file behaves exactly like the
//! hardcoded build.

use crate::config;
use chrono::Weekday;
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

/// The four cost-weight coefficients for the spend / cost-weighted view.
/// `Copy` so it threads cheaply into hot loops. Display-only — never used for
/// cap calibration.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CostWeights {
    pub input: f64,
    pub cache_creation: f64,
    pub cache_read: f64,
    pub output: f64,
}

impl Default for CostWeights {
    fn default() -> Self {
        Self {
            input: config::COST_WEIGHT_INPUT,
            cache_creation: config::COST_WEIGHT_CACHE_CREATION,
            cache_read: config::COST_WEIGHT_CACHE_READ,
            output: config::COST_WEIGHT_OUTPUT,
        }
    }
}

/// Parsed bundle for calibration math: the local zone plus the weekly-reset
/// anchor. `Copy` so deep functions (some called per-turn) take it by value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalParams {
    pub tz: Tz,
    pub reset_weekday: Weekday,
    pub reset_hour: u32,
}

impl Default for CalParams {
    fn default() -> Self {
        Self {
            tz: config::LOCAL_TZ
                .parse()
                .expect("config::LOCAL_TZ must be a valid IANA name"),
            reset_weekday: config::WEEKLY_RESET_WEEKDAY,
            reset_hour: config::WEEKLY_RESET_HOUR_LOCAL,
        }
    }
}

/// All user-editable settings. `#[serde(default)]` fills any field missing from
/// the TOML file from `Default`, so partial/old files still load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub local_tz: String,
    pub weekly_reset_weekday: Weekday,
    pub weekly_reset_hour: u32,
    pub poll_interval_secs: u64,
    pub cost_weights: CostWeights,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            local_tz: config::LOCAL_TZ.to_string(),
            weekly_reset_weekday: config::WEEKLY_RESET_WEEKDAY,
            weekly_reset_hour: config::WEEKLY_RESET_HOUR_LOCAL,
            poll_interval_secs: 120,
            cost_weights: CostWeights::default(),
        }
    }
}

impl Settings {
    /// Parse `local_tz`, falling back to the default zone if somehow invalid
    /// (defense in depth; the UI combo only offers valid zones).
    pub fn tz(&self) -> Tz {
        self.local_tz
            .parse()
            .unwrap_or_else(|_| CalParams::default().tz)
    }

    /// Bundle the calibration-relevant fields.
    pub fn cal_params(&self) -> CalParams {
        CalParams {
            tz: self.tz(),
            reset_weekday: self.weekly_reset_weekday,
            reset_hour: self.weekly_reset_hour,
        }
    }
}

/// The allowed poll intervals (seconds). Mirrors the CLI `--interval` choices;
/// constrained to stay above the ~1 req/min endpoint rate limit.
pub const POLL_INTERVAL_CHOICES: [u64; 3] = [60, 120, 300];

/// Validate a settings struct. `Ok(())` if usable; `Err(message)` otherwise.
/// Used by the file-load path and the UI's Save gate.
pub fn validate(s: &Settings) -> Result<(), String> {
    if s.local_tz.parse::<Tz>().is_err() {
        return Err(format!("invalid timezone: '{}'", s.local_tz));
    }
    if s.weekly_reset_hour > 23 {
        return Err(format!(
            "weekly reset hour must be 0..=23, got {}",
            s.weekly_reset_hour
        ));
    }
    if !POLL_INTERVAL_CHOICES.contains(&s.poll_interval_secs) {
        return Err(format!(
            "poll interval must be one of {:?}, got {}",
            POLL_INTERVAL_CHOICES, s.poll_interval_secs
        ));
    }
    let w = &s.cost_weights;
    for (name, v) in [
        ("input", w.input),
        ("cache_creation", w.cache_creation),
        ("cache_read", w.cache_read),
        ("output", w.output),
    ] {
        if !v.is_finite() || v < 0.0 {
            return Err(format!(
                "cost weight '{name}' must be a finite value >= 0.0"
            ));
        }
    }
    Ok(())
}

use std::path::Path;

/// Load settings from the default path. Never fails: any error (missing,
/// unreadable, malformed, or failing validation) logs a warning and yields
/// defaults.
pub fn load() -> Settings {
    match crate::paths::settings_path() {
        Ok(p) => load_from(&p),
        Err(e) => {
            tracing::warn!(error = %e, "could not resolve settings path; using defaults");
            Settings::default()
        }
    }
}

/// Testable core of `load`.
pub fn load_from(path: &Path) -> Settings {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return Settings::default(), // missing file is normal
    };
    let parsed: Settings = match toml::from_str(&text) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "settings.toml is malformed; using defaults");
            return Settings::default();
        }
    };
    if let Err(msg) = validate(&parsed) {
        tracing::warn!(reason = %msg, "settings.toml failed validation; using defaults");
        return Settings::default();
    }
    parsed
}

/// Save settings to the default path. Returns the error so the UI can show it.
pub fn save(s: &Settings) -> anyhow::Result<()> {
    let p = crate::paths::settings_path()?;
    save_to(&p, s)
}

/// Testable core of `save`. Writes atomically (temp file + rename).
pub fn save_to(path: &Path, s: &Settings) -> anyhow::Result<()> {
    use anyhow::Context;
    crate::paths::ensure_parent_dir(path)?;
    let text = toml::to_string_pretty(s).context("serializing settings to TOML")?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_match_config_consts() {
        let s = Settings::default();
        assert_eq!(s.local_tz, config::LOCAL_TZ);
        assert_eq!(s.weekly_reset_weekday, config::WEEKLY_RESET_WEEKDAY);
        assert_eq!(s.weekly_reset_hour, config::WEEKLY_RESET_HOUR_LOCAL);
        assert_eq!(s.poll_interval_secs, 120);
        assert_eq!(s.cost_weights.input, config::COST_WEIGHT_INPUT);
        assert_eq!(
            s.cost_weights.cache_creation,
            config::COST_WEIGHT_CACHE_CREATION
        );
        assert_eq!(s.cost_weights.cache_read, config::COST_WEIGHT_CACHE_READ);
        assert_eq!(s.cost_weights.output, config::COST_WEIGHT_OUTPUT);
    }

    #[test]
    fn validate_accepts_defaults() {
        assert!(validate(&Settings::default()).is_ok());
    }

    #[test]
    fn validate_rejects_bad_tz_hour_interval_and_weights() {
        assert!(validate(&Settings {
            local_tz: "Not/AZone".into(),
            ..Settings::default()
        })
        .is_err());

        assert!(validate(&Settings {
            weekly_reset_hour: 24,
            ..Settings::default()
        })
        .is_err());

        assert!(validate(&Settings {
            poll_interval_secs: 90,
            ..Settings::default()
        })
        .is_err());

        assert!(validate(&Settings {
            cost_weights: CostWeights {
                output: -1.0,
                ..CostWeights::default()
            },
            ..Settings::default()
        })
        .is_err());
    }

    #[test]
    fn cal_params_uses_parsed_tz() {
        let s = Settings {
            local_tz: "America/New_York".into(),
            ..Settings::default()
        };
        assert_eq!(s.cal_params().tz, chrono_tz::America::New_York);
    }

    #[test]
    fn load_from_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nope.toml");
        assert_eq!(load_from(&p), Settings::default());
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.toml");
        std::fs::write(&p, "this is not = valid = toml {{{").unwrap();
        assert_eq!(load_from(&p), Settings::default());
    }

    #[test]
    fn save_to_then_load_from_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.toml");
        let s = Settings {
            local_tz: "America/New_York".into(),
            poll_interval_secs: 300,
            cost_weights: CostWeights {
                output: 9.0,
                ..CostWeights::default()
            },
            ..Settings::default()
        };
        save_to(&p, &s).unwrap();
        assert_eq!(load_from(&p), s);
    }

    #[test]
    fn load_from_partial_toml_fills_missing_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("partial.toml");
        std::fs::write(&p, "poll_interval_secs = 60\n").unwrap();
        let loaded = load_from(&p);
        assert_eq!(loaded.poll_interval_secs, 60);
        assert_eq!(loaded.local_tz, config::LOCAL_TZ);
        assert_eq!(loaded.cost_weights, CostWeights::default());
    }
}
