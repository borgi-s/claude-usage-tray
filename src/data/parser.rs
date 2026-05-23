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

/// Returns `(is_subagent, subagent_id)` for a JSONL file path.
///
/// `is_subagent` is true if any path component is literally "subagents".
/// `subagent_id` is `Some(hex)` only when the filename is `agent-<hex>.jsonl`.
pub fn classify_subagent(path: &std::path::Path) -> (bool, Option<String>) {
    let is_sub = path
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case("subagents"));
    if !is_sub {
        return (false, None);
    }
    let id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix("agent-"))
        .map(|s| s.to_string());
    (true, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn classify_subagent_recognizes_subagent_path() {
        let p = Path::new("/home/u/.claude/projects/foo/subagents/agent-deadbeef.jsonl");
        let (is_sub, id) = classify_subagent(p);
        assert!(is_sub);
        assert_eq!(id, Some("deadbeef".to_string()));
    }

    #[test]
    fn classify_subagent_recognizes_windows_path() {
        let p = Path::new(r"C:\Users\u\.claude\projects\foo\subagents\agent-cafe1234.jsonl");
        let (is_sub, id) = classify_subagent(p);
        assert!(is_sub);
        assert_eq!(id, Some("cafe1234".to_string()));
    }

    #[test]
    fn classify_subagent_rejects_main_session_path() {
        let p = Path::new("/home/u/.claude/projects/foo/sess-1234.jsonl");
        let (is_sub, id) = classify_subagent(p);
        assert!(!is_sub);
        assert_eq!(id, None);
    }

    #[test]
    fn classify_subagent_rejects_subagents_dir_without_agent_prefix() {
        // Path is in subagents/ but the filename doesn't match agent-<hex>.jsonl.
        // We mark is_subagent=true (the path classification) but id=None.
        let p = Path::new("/home/u/subagents/garbage.jsonl");
        let (is_sub, id) = classify_subagent(p);
        assert!(is_sub);
        assert_eq!(id, None);
    }
}
