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
pub(crate) fn classify_subagent(path: &std::path::Path) -> (bool, Option<String>) {
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
pub(crate) fn walk_jsonl(root: &std::path::Path) -> impl Iterator<Item = PathBuf> {
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

/// Yields one `Turn` per JSONL line that contains usage telemetry or a
/// rate-limit error. Bad JSON lines, empty lines, and rows without a
/// `message.usage` field (and not rate-limit errors) are silently skipped.
pub fn iter_rows(path: &std::path::Path) -> impl Iterator<Item = Turn> {
    let (is_sub, sub_id) = classify_subagent(path);
    let path_owned = path.to_path_buf();
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();

    lines.into_iter().filter_map(move |raw_line| {
        let line = raw_line.trim();
        if line.is_empty() {
            return None;
        }
        let obj: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return None,
        };
        let obj = obj.as_object()?;

        let rate_limited = is_rate_limit_error(obj);
        let usage = obj.get("message").and_then(|m| m.get("usage"));
        if usage.is_none() && !rate_limited {
            return None;
        }

        let ts_raw = obj.get("timestamp").and_then(|v| v.as_str())?;
        let ts = chrono::DateTime::parse_from_rfc3339(ts_raw)
            .ok()?
            .with_timezone(&chrono::Utc);

        let session_id = obj
            .get("sessionId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let project_cwd = obj
            .get("cwd")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let version = obj
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let model = obj
            .get("message")
            .and_then(|m| m.get("model"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let usage_obj = usage.and_then(|u| u.as_object());
        let get_u64 = |key: &str| -> u64 {
            usage_obj
                .and_then(|u| u.get(key))
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
        };

        Some(Turn {
            ts,
            session_id,
            subagent_id: sub_id.clone(),
            is_subagent: is_sub,
            project_cwd,
            model,
            version,
            input_tokens: get_u64("input_tokens"),
            output_tokens: get_u64("output_tokens"),
            cache_creation_input_tokens: get_u64("cache_creation_input_tokens"),
            cache_read_input_tokens: get_u64("cache_read_input_tokens"),
            source_file: path_owned.clone(),
            is_rate_limit_error: rate_limited,
        })
    })
}

/// Returns true if `obj` represents an API error caused by rate limiting.
fn is_rate_limit_error(obj: &serde_json::Map<String, serde_json::Value>) -> bool {
    // Outer "type" must be an error variant.
    let outer_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if outer_type != "api-error" && outer_type != "error" {
        return false;
    }
    // Check obj.error directly.
    if let Some(err) = obj.get("error").and_then(|v| v.as_object()) {
        if error_indicates_rate_limit(err) {
            return true;
        }
    }
    // Check obj.message.error (some shapes nest it inside message).
    if let Some(err) = obj
        .get("message")
        .and_then(|m| m.get("error"))
        .and_then(|v| v.as_object())
    {
        if error_indicates_rate_limit(err) {
            return true;
        }
    }
    false
}

fn error_indicates_rate_limit(err: &serde_json::Map<String, serde_json::Value>) -> bool {
    if err.get("status").and_then(|v| v.as_u64()) == Some(429) {
        return true;
    }
    let t = err
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    t.contains("rate") || t.contains("limit")
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
        touch(root, "proj1/notes.txt"); // should be filtered
        touch(root, "proj2/sub/sub/c.jsonl");

        let mut found: Vec<_> = walk_jsonl(root).collect();
        found.sort();

        assert_eq!(found.len(), 4);
        assert!(found
            .iter()
            .all(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl")));
    }

    #[test]
    fn walk_jsonl_returns_empty_for_missing_root() {
        let td = TempDir::new().unwrap();
        let missing = td.path().join("does-not-exist");
        let found: Vec<_> = walk_jsonl(&missing).collect();
        assert!(found.is_empty());
    }
}
