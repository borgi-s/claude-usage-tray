//! JSONL parser for `~/.claude/projects/**/*.jsonl`. One [`Turn`] per
//! assistant turn (or per rate-limit error row).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One row per assistant turn. Mirrors the Python `parser.TurnRow`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Turn {
    pub ts: DateTime<Utc>,
    pub session_id: String,
    pub subagent_id: Option<String>,
    pub is_subagent: bool,
    pub project_cwd: String,
    pub model: String,
    pub version: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub source_file: PathBuf,
    pub is_rate_limit_error: bool,
}
