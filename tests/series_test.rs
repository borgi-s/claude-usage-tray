use chrono::{TimeZone, Utc};
use claude_usage_tray::dashboard::series::{
    cumulative_share_series_5h, cumulative_share_series_weekly,
};
use claude_usage_tray::data::parser::Turn;
use claude_usage_tray::settings::CalParams;
use std::path::PathBuf;

fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<chrono::Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
}

fn turn(ts: chrono::DateTime<chrono::Utc>, output: u64) -> Turn {
    Turn {
        ts,
        session_id: String::new(),
        subagent_id: None,
        is_subagent: false,
        project_cwd: String::new(),
        model: String::new(),
        version: String::new(),
        input_tokens: 0,
        output_tokens: output,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 0,
        source_file: PathBuf::new(),
        is_rate_limit_error: false,
    }
}

#[test]
fn cumulative_share_5h_single_window_stepped_growth() {
    let turns = vec![
        turn(utc(2026, 5, 24, 10, 0), 100),
        turn(utc(2026, 5, 24, 11, 0), 200),
        turn(utc(2026, 5, 24, 12, 0), 300),
    ];
    // cap = 1000 → shares: 0.1, 0.3, 0.6
    let series = cumulative_share_series_5h(&turns, Some(1000.0));
    assert_eq!(series.len(), 3);
    assert!((series[0].cumulative_share - 0.1).abs() < 0.001);
    assert!((series[1].cumulative_share - 0.3).abs() < 0.001);
    assert!((series[2].cumulative_share - 0.6).abs() < 0.001);
    assert_eq!(series[0].window_idx, 0);
    assert_eq!(series[1].window_idx, 0);
    assert_eq!(series[2].window_idx, 0);
}

#[test]
fn cumulative_share_5h_new_window_after_gap_resets_share() {
    let turns = vec![
        turn(utc(2026, 5, 24, 8, 0), 500),  // window 0
        turn(utc(2026, 5, 24, 14, 0), 200), // 6h gap → window 1
        turn(utc(2026, 5, 24, 15, 0), 300), // still window 1
    ];
    let series = cumulative_share_series_5h(&turns, Some(1000.0));
    assert_eq!(series.len(), 3);
    assert_eq!(series[0].window_idx, 0);
    assert_eq!(series[1].window_idx, 1);
    assert_eq!(series[2].window_idx, 1);
    assert!((series[0].cumulative_share - 0.5).abs() < 0.001);
    // Window 1 cumulative resets, so turn at 14:00 = 200/1000 = 0.2.
    assert!((series[1].cumulative_share - 0.2).abs() < 0.001);
    // Turn at 15:00 cumulates within window 1: (200+300)/1000 = 0.5
    assert!((series[2].cumulative_share - 0.5).abs() < 0.001);
}

#[test]
fn cumulative_share_5h_no_cap_uses_raw_output() {
    let turns = vec![turn(utc(2026, 5, 24, 10, 0), 100)];
    let series = cumulative_share_series_5h(&turns, None);
    assert_eq!(series.len(), 1);
    // cumulative_share is raw output tokens when cap is None.
    assert_eq!(series[0].cumulative_share, 100.0);
}

#[test]
fn cumulative_share_weekly_resets_at_sunday_0700_local() {
    // 2026-05-17 is a Sunday. CEST (May) = UTC+2. Sun 07:00 CEST = Sun 05:00 UTC.
    let turns = vec![
        turn(utc(2026, 5, 17, 4, 0), 999), // before reset → its own (prior-week) window
        turn(utc(2026, 5, 17, 6, 0), 100), // after reset → fresh week → window_idx incremented
        turn(utc(2026, 5, 23, 12, 0), 200), // still same week
        turn(utc(2026, 5, 24, 6, 0), 50),  // after next reset → fresh week → window_idx incremented
    ];
    let series = cumulative_share_series_weekly(&turns, Some(1000.0), CalParams::default());
    assert_eq!(series.len(), 4);

    // The first turn (pre-reset) starts the series in its own "prior" week.
    // Subsequent turns in a new week increment window_idx.
    // We don't test the exact window_idx of the first turn — just that they
    // increment across week boundaries.
    assert!(series[1].window_idx > series[0].window_idx);
    assert_eq!(series[1].window_idx, series[2].window_idx);
    assert!(series[3].window_idx > series[2].window_idx);

    // Mid-week turn (5/23 12:00 UTC, same week as 5/17 06:00 turn):
    // cumulative = (100 + 200) / 1000 = 0.3
    let mid_share = series
        .iter()
        .find(|w| w.ts == utc(2026, 5, 23, 12, 0))
        .unwrap();
    assert!((mid_share.cumulative_share - 0.3).abs() < 0.001);

    // Next-week turn (5/24 06:00 UTC, after Sun 05:00 UTC reset):
    // cumulative = 50 / 1000 = 0.05 (resets in the new week)
    let next_share = series
        .iter()
        .find(|w| w.ts == utc(2026, 5, 24, 6, 0))
        .unwrap();
    assert!((next_share.cumulative_share - 0.05).abs() < 0.001);
}

#[test]
fn daily_aggregates_groups_by_local_date() {
    use chrono::Datelike;
    use claude_usage_tray::dashboard::series::daily_aggregates;

    // Each turn input=10, cache_create=10, cache_read=10, output=10.
    // cost_weighted per turn = 10*1 + 10*1.25 + 10*0.1 + 10*5 = 73.5.
    let mk = |ts: chrono::DateTime<chrono::Utc>| {
        let mut t = turn(ts, 10);
        t.input_tokens = 10;
        t.cache_creation_input_tokens = 10;
        t.cache_read_input_tokens = 10;
        t
    };
    // 2026-05-24 10:00 UTC = 12:00 CEST = May 24 local.
    // 2026-05-24 23:30 UTC = 01:30 CEST May 25 → May 25 local.
    // 2026-05-25 10:00 UTC = 12:00 CEST May 25 → May 25 local.
    let turns = vec![
        mk(utc(2026, 5, 24, 10, 0)),
        mk(utc(2026, 5, 24, 23, 30)),
        mk(utc(2026, 5, 25, 10, 0)),
    ];
    let daily = daily_aggregates(
        &turns,
        &claude_usage_tray::settings::CostWeights::default(),
        claude_usage_tray::settings::CalParams::default().tz,
    );
    // May 24 local: 1 turn = 73.5. May 25 local: 2 turns = 147.0.
    assert_eq!(daily.len(), 2);
    let may24 = daily.iter().find(|(d, _)| d.day() == 24).unwrap();
    let may25 = daily.iter().find(|(d, _)| d.day() == 25).unwrap();
    assert!((may24.1 - 73.5).abs() < 0.01);
    assert!((may25.1 - 147.0).abs() < 0.01);
}

#[test]
fn daily_aggregates_empty_returns_empty() {
    use claude_usage_tray::dashboard::series::daily_aggregates;

    let out = daily_aggregates(
        &[],
        &claude_usage_tray::settings::CostWeights::default(),
        claude_usage_tray::settings::CalParams::default().tz,
    );
    assert!(out.is_empty());
}
