//! Mtime-diff incremental cache for `Vec<Turn>`. Persists to
//! `~/.claude-usage-tray/cache.bincode` + `cache_manifest.json`.

use crate::data::parser::Turn;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;

// v2: manifest stores `(mtime, len)` per file instead of mtime alone, so two
// rapid appends that share an mtime (coarse-resolution filesystems: FAT/exFAT
// 2s, some network shares 1s) are still detected via the length change. Old v1
// manifests/caches are discarded on load → one-time full reparse on upgrade.
const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CacheFile {
    pub schema_version: u32,
    pub turns: Vec<Turn>,
}

/// Change-detection signature for a JSONL file: modification time plus byte
/// length. Equality means "unchanged → skip reparse".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct FileSig {
    pub mtime_ms: i64,
    pub len: u64,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub(crate) struct Manifest {
    pub schema_version: u32,
    pub sigs: HashMap<PathBuf, FileSig>,
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

use crate::data::parser::{iter_rows, walk_jsonl};
use std::path::Path;
use std::time::UNIX_EPOCH;

/// Refresh the cache against `projects_root`, writing cache + manifest into `app_dir`.
///
/// Returns the full sorted-by-`ts` `Vec<Turn>` covering every JSONL file under root.
/// First run reads everything; subsequent runs reparse only files whose mtime changed
/// (and drop rows from files that have been deleted).
pub fn refresh_at(projects_root: &Path, app_dir: &Path) -> Result<Vec<Turn>, CacheError> {
    std::fs::create_dir_all(app_dir)?;

    // 1. Load prior cache + manifest, tolerating any failure (treat as empty).
    let cache_result = load_cache(app_dir);
    let mut prior_turns: Vec<Turn> = cache_result.as_ref().unwrap_or(&Vec::new()).clone();
    // If cache is corrupt, also reset manifest so we re-scan everything.
    let mut prior_sigs: HashMap<PathBuf, FileSig> = if cache_result.is_ok() {
        load_manifest(app_dir).unwrap_or_default()
    } else {
        HashMap::new()
    };

    // 2. Walk root for *.jsonl and read current (mtime, len) signatures.
    let current: Vec<PathBuf> = walk_jsonl(projects_root).collect();
    let mut current_sigs: HashMap<PathBuf, FileSig> = HashMap::new();
    for p in &current {
        let sig = file_sig(p).unwrap_or(FileSig {
            mtime_ms: 0,
            len: 0,
        });
        current_sigs.insert(p.clone(), sig);
    }

    // 3. Compute diff sets.
    let new_or_changed: Vec<PathBuf> = current
        .iter()
        .filter(|p| prior_sigs.get(*p) != current_sigs.get(*p))
        .cloned()
        .collect();
    let deleted: Vec<PathBuf> = prior_sigs
        .keys()
        .filter(|p| !current_sigs.contains_key(*p))
        .cloned()
        .collect();

    // 4. Fast path: nothing changed → return prior turns.
    if new_or_changed.is_empty() && deleted.is_empty() {
        prior_turns.sort_by_key(|t| t.ts);
        return Ok(prior_turns);
    }

    // 5. Drop stale rows.
    let stale: std::collections::HashSet<PathBuf> = new_or_changed
        .iter()
        .chain(deleted.iter())
        .cloned()
        .collect();
    prior_turns.retain(|t| !stale.contains(&t.source_file));

    // 6. Reparse changed files, append.
    for p in &new_or_changed {
        for row in iter_rows(p) {
            prior_turns.push(row);
        }
    }

    // 7. Sort by ts.
    prior_turns.sort_by_key(|t| t.ts);

    // 8. Write out (atomic).
    prior_sigs = current_sigs;
    write_cache(app_dir, &prior_turns)?;
    write_manifest(app_dir, &prior_sigs)?;

    Ok(prior_turns)
}

fn file_sig(p: &Path) -> Option<FileSig> {
    let meta = std::fs::metadata(p).ok()?;
    let modified = meta.modified().ok()?;
    let dur = modified.duration_since(UNIX_EPOCH).ok()?;
    Some(FileSig {
        mtime_ms: dur.as_millis() as i64,
        len: meta.len(),
    })
}

fn load_cache(app_dir: &Path) -> Result<Vec<Turn>, CacheError> {
    let path = app_dir.join("cache.bincode");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(&path)?;
    let file: CacheFile = bincode::deserialize(&bytes)?;
    if file.schema_version != SCHEMA_VERSION {
        return Ok(Vec::new());
    }
    Ok(file.turns)
}

fn load_manifest(app_dir: &Path) -> Result<HashMap<PathBuf, FileSig>, CacheError> {
    let path = app_dir.join("cache_manifest.json");
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let bytes = std::fs::read(&path)?;
    let m: Manifest = serde_json::from_slice(&bytes)?;
    if m.schema_version != SCHEMA_VERSION {
        return Ok(HashMap::new());
    }
    Ok(m.sigs)
}

fn write_cache(app_dir: &Path, turns: &[Turn]) -> Result<(), CacheError> {
    let file = CacheFile {
        schema_version: SCHEMA_VERSION,
        turns: turns.to_vec(),
    };
    let bytes = bincode::serialize(&file)?;
    let final_path = app_dir.join("cache.bincode");
    let tmp_path = app_dir.join("cache.bincode.tmp");
    std::fs::write(&tmp_path, &bytes)?;
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

fn write_manifest(app_dir: &Path, sigs: &HashMap<PathBuf, FileSig>) -> Result<(), CacheError> {
    let m = Manifest {
        schema_version: SCHEMA_VERSION,
        sigs: sigs.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&m)?;
    let final_path = app_dir.join("cache_manifest.json");
    let tmp_path = app_dir.join("cache_manifest.json.tmp");
    std::fs::write(&tmp_path, &bytes)?;
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

/// Convenience wrapper: refreshes against `~/.claude/projects/` and writes the
/// cache under `~/.claude-usage-tray/`. Used by the polling thread.
pub fn refresh() -> Result<Vec<Turn>, CacheError> {
    let projects_root = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?
        .join(".claude")
        .join("projects");
    let app_dir = crate::paths::app_dir()?;
    refresh_at(&projects_root, &app_dir)
}
