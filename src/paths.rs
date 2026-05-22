use anyhow::{Context, Result};
use std::path::PathBuf;

const APP_DIR_NAME: &str = ".claude-usage-tray";

/// Returns ~/.claude-usage-tray/. Does NOT create the directory.
pub fn app_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    Ok(home.join(APP_DIR_NAME))
}

/// Returns ~/.claude-usage-tray/calibration_log.jsonl. Does NOT create the file.
pub fn calibration_log_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("calibration_log.jsonl"))
}

/// Ensures the parent directory of `path` exists, creating it (and ancestors) if needed.
/// Idempotent: no-op if already present.
pub fn ensure_parent_dir(path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create directory {}", parent.display()))?;
    }
    Ok(())
}
