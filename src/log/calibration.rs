use crate::api::credentials::Credentials;
use crate::api::usage::UsageSnapshot;
use crate::paths;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use thiserror::Error;

const SCHEMA_VERSION: u32 = 1;

/// One record per JSONL line in ~/.claude-usage-tray/calibration_log.jsonl.
/// Field naming mirrors Anthropic's API (`five_hour`/`seven_day`) for clarity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationSample {
    pub schema_version: u32,
    pub ts: DateTime<Utc>,
    pub five_hour_util: Option<f64>,
    pub five_hour_resets_at: Option<DateTime<Utc>>,
    pub seven_day_util: Option<f64>,
    pub seven_day_resets_at: Option<DateTime<Utc>>,
    pub subscription_type: String,
    pub rate_limit_tier: String,
}

#[derive(Debug, Error)]
pub enum LogError {
    #[error("io error writing calibration log: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization failed: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Pure: build a sample record from a fresh snapshot + the active credentials.
pub fn sample_from(snap: &UsageSnapshot, creds: &Credentials) -> CalibrationSample {
    CalibrationSample {
        schema_version: SCHEMA_VERSION,
        ts: Utc::now(),
        five_hour_util: snap.five_hour.as_ref().map(|b| b.utilization),
        five_hour_resets_at: snap.five_hour.as_ref().and_then(|b| b.resets_at),
        seven_day_util: snap.seven_day.as_ref().map(|b| b.utilization),
        seven_day_resets_at: snap.seven_day.as_ref().and_then(|b| b.resets_at),
        subscription_type: creds.subscription_type.clone(),
        rate_limit_tier: creds.rate_limit_tier.clone(),
    }
}

/// I/O: append one JSONL record to `path`. Creates parent dirs and the file
/// if needed. Each call serializes, writes one line + `\n`, then flushes.
pub fn append(path: &Path, sample: &CalibrationSample) -> Result<(), LogError> {
    paths::ensure_parent_dir(path).map_err(|e| {
        LogError::Io(std::io::Error::other(e.to_string()))
    })?;

    let line = serde_json::to_string(sample)?;

    let mut file = OpenOptions::new().append(true).create(true).open(path)?;

    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

/// Glue: convenience wrapper for the watch loop. Resolves the default path,
/// builds a sample, and appends in one call.
pub fn append_to_default_path(snap: &UsageSnapshot, creds: &Credentials) -> Result<(), LogError> {
    let path = paths::calibration_log_path().map_err(|e| {
        LogError::Io(std::io::Error::other(e.to_string()))
    })?;
    let sample = sample_from(snap, creds);
    append(&path, &sample)
}
