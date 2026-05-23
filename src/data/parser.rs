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

/// Recursively yields every `*.jsonl` file under `root` (any depth).
///
/// Returns an empty iterator if `root` doesn't exist or can't be read —
/// callers don't need to special-case the first-run case.
pub fn walk_jsonl(root: &std::path::Path) -> impl Iterator<Item = PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    walk_inner(root, &mut out);
    out.into_iter()
}

fn walk_inner(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_inner(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            out.push(p);
        }
    }
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

    use tempfile::TempDir;

    fn touch(dir: &Path, rel: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "").unwrap();
    }

    #[test]
    fn walk_jsonl_recurses_and_filters_by_extension() {
        let td = TempDir::new().unwrap();
        let root = td.path();
        touch(root, "sess-a.jsonl");
        touch(root, "proj1/sess-b.jsonl");
        touch(root, "proj1/subagents/agent-1.jsonl");
        touch(root, "proj1/notes.txt");    // should be filtered
        touch(root, "proj2/sub/sub/c.jsonl");

        let mut found: Vec<_> = walk_jsonl(root).collect();
        found.sort();

        assert_eq!(found.len(), 4);
        assert!(found.iter().all(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl")));
    }

    #[test]
    fn walk_jsonl_returns_empty_for_missing_root() {
        let td = TempDir::new().unwrap();
        let missing = td.path().join("does-not-exist");
        let found: Vec<_> = walk_jsonl(&missing).collect();
        assert!(found.is_empty());
    }
}
