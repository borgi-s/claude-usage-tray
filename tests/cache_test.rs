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
