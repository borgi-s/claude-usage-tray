use claude_usage_tray::data::parser::iter_rows;
use std::path::Path;

#[test]
fn iter_rows_yields_two_usage_rows_from_fixture() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample_session.jsonl");
    let rows: Vec<_> = iter_rows(&p).collect();

    // For now we expect exactly 2 — the two assistant-with-usage rows.
    // (The rate-limit row will be added in the next task.)
    let usage_rows: Vec<_> = rows.iter().filter(|r| !r.is_rate_limit_error).collect();
    assert_eq!(usage_rows.len(), 2);

    let r0 = &usage_rows[0];
    assert_eq!(r0.session_id, "sess-abc");
    assert_eq!(r0.project_cwd, "/proj/foo");
    assert_eq!(r0.model, "claude-opus-4-7");
    assert_eq!(r0.version, "1.2.3");
    assert_eq!(r0.input_tokens, 100);
    assert_eq!(r0.output_tokens, 2000);
    assert_eq!(r0.cache_creation_input_tokens, 50);
    assert_eq!(r0.cache_read_input_tokens, 10);
    assert!(!r0.is_subagent);
    assert_eq!(r0.subagent_id, None);

    let r1 = &usage_rows[1];
    assert_eq!(r1.output_tokens, 3000);
}

#[test]
fn iter_rows_yields_rate_limit_error_row() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample_session.jsonl");
    let rows: Vec<_> = iter_rows(&p).collect();

    let rl_rows: Vec<_> = rows.iter().filter(|r| r.is_rate_limit_error).collect();
    assert_eq!(rl_rows.len(), 1);
    assert_eq!(rl_rows[0].session_id, "sess-abc");
    assert_eq!(rl_rows[0].input_tokens, 0); // no usage on error rows
}
