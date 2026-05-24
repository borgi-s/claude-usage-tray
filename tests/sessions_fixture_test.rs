use claude_usage_tray::data::parser::iter_rows;
use claude_usage_tray::data::sessions::session_summaries;
use std::path::Path;

#[test]
fn fixture_sessions_aggregate_with_subagent() {
    let mut turns = Vec::new();
    turns.extend(iter_rows(Path::new("tests/fixtures/sessions_multi.jsonl")));
    turns.extend(iter_rows(Path::new(
        "tests/fixtures/subagents/agent-deadbeef.jsonl",
    )));

    let summaries = session_summaries(&turns);
    // sess-A (2 main + 1 subagent) and sess-B (1 main), sorted by start.
    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0].session_id, "sess-A");
    assert_eq!(summaries[1].session_id, "sess-B");

    let a = &summaries[0];
    assert_eq!(a.main_turns, 2);
    assert_eq!(a.subagent_count, 1); // agent-deadbeef
                                     // peak_prompt_tokens = max(1000+0+500, 2000+0+1000) = 3000
    assert_eq!(a.peak_prompt_tokens, 3000);
    // opus-4-7 window 1_000_000 → peak ctx = 3000 / 1_000_000
    assert!((a.peak_context_pct - 3000.0 / 1_000_000.0).abs() < 1e-12);

    let b = &summaries[1];
    assert_eq!(b.main_turns, 1);
    assert_eq!(b.subagent_count, 0);
}
