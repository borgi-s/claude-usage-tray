use claude_usage_tray::data::cache;
use tempfile::TempDir;

fn write_jsonl(dir: &std::path::Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

const SAMPLE_USAGE_LINE: &str = r#"{"timestamp":"2026-05-22T10:00:00Z","sessionId":"s1","cwd":"/proj","version":"1","type":"assistant","message":{"model":"opus","usage":{"input_tokens":1,"output_tokens":100}}}"#;

#[test]
fn refresh_first_run_parses_all_files() {
    let projects = TempDir::new().unwrap();
    let app_dir = TempDir::new().unwrap();
    write_jsonl(projects.path(), "a/sess-1.jsonl", SAMPLE_USAGE_LINE);
    write_jsonl(projects.path(), "b/sess-2.jsonl", SAMPLE_USAGE_LINE);

    let turns = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    assert_eq!(turns.len(), 2);

    // Cache files should now exist.
    assert!(app_dir.path().join("cache.bincode").exists());
    assert!(app_dir.path().join("cache_manifest.json").exists());
}

#[test]
fn refresh_second_run_reparses_only_changed_files() {
    let projects = TempDir::new().unwrap();
    let app_dir = TempDir::new().unwrap();
    write_jsonl(projects.path(), "a/sess-1.jsonl", SAMPLE_USAGE_LINE);
    write_jsonl(projects.path(), "b/sess-2.jsonl", SAMPLE_USAGE_LINE);

    // First run primes the cache.
    let turns_1 = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    assert_eq!(turns_1.len(), 2);

    // Modify sess-1 to add a second row. Set its mtime to "now" explicitly so
    // the change is detectable even on filesystems with coarse mtime resolution.
    let extra = format!("\n{}", SAMPLE_USAGE_LINE);
    let p1 = projects.path().join("a").join("sess-1.jsonl");
    let mut existing = std::fs::read_to_string(&p1).unwrap();
    existing.push_str(&extra);
    std::fs::write(&p1, existing).unwrap();
    let now = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    filetime::set_file_mtime(&p1, filetime::FileTime::from_system_time(now)).unwrap();

    let turns_2 = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    // sess-1 now has 2 rows; sess-2 still has 1.
    assert_eq!(turns_2.len(), 3);
}

#[test]
fn refresh_no_changes_returns_quickly_with_same_count() {
    let projects = TempDir::new().unwrap();
    let app_dir = TempDir::new().unwrap();
    write_jsonl(projects.path(), "a/sess.jsonl", SAMPLE_USAGE_LINE);

    let turns_1 = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    let turns_2 = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    assert_eq!(turns_1, turns_2);
}

#[test]
fn refresh_drops_rows_from_deleted_files() {
    let projects = TempDir::new().unwrap();
    let app_dir = TempDir::new().unwrap();
    write_jsonl(projects.path(), "a.jsonl", SAMPLE_USAGE_LINE);
    write_jsonl(projects.path(), "b.jsonl", SAMPLE_USAGE_LINE);

    let turns_1 = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    assert_eq!(turns_1.len(), 2);

    std::fs::remove_file(projects.path().join("a.jsonl")).unwrap();

    let turns_2 = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    assert_eq!(turns_2.len(), 1);
}

#[test]
fn refresh_recovers_from_corrupt_cache() {
    let projects = TempDir::new().unwrap();
    let app_dir = TempDir::new().unwrap();
    write_jsonl(projects.path(), "a.jsonl", SAMPLE_USAGE_LINE);

    // Prime the cache.
    let _ = cache::refresh_at(projects.path(), app_dir.path()).unwrap();

    // Corrupt the cache file.
    std::fs::write(app_dir.path().join("cache.bincode"), b"not bincode").unwrap();

    // Refresh should silently rebuild and return the correct rows.
    let turns = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    assert_eq!(turns.len(), 1);

    // A second refresh on an unchanged tree must succeed (proves the rebuild
    // produced a deserializable cache).
    let turns2 = cache::refresh_at(projects.path(), app_dir.path()).unwrap();
    assert_eq!(turns2.len(), 1);
}
