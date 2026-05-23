//! Mtime-diff incremental cache for `Vec<Turn>`. Persists to
//! `~/.claude-usage-tray/cache.bincode` + `cache_manifest.json`.

use crate::data::parser::Turn;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CacheFile {
    pub schema_version: u32,
    pub turns: Vec<Turn>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub(crate) struct Manifest {
    pub schema_version: u32,
    pub mtimes: HashMap<PathBuf, i64>,
}

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bincode error: {0}")]
    Bincode(#[from] bincode::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("anyhow error: {0}")]
    Anyhow(#[from] anyhow::Error),
}
