//! Per-session aggregation for the dashboard's sessions table. Pure functions
//! ported from the Python `metrics.session_summaries`. No egui here.

use crate::data::parser::Turn;

/// Context tokens fed into the model for one turn:
/// input + cache_creation + cache_read. Output is excluded (it's the response,
/// not the context). Mirrors the Python `prompt_tokens` derivation.
pub fn prompt_tokens(t: &Turn) -> u64 {
    t.input_tokens + t.cache_creation_input_tokens + t.cache_read_input_tokens
}

use crate::settings::CostWeights;
use crate::shared::snapshot::cost_weighted;
use chrono::{DateTime, Utc};
use std::collections::{BTreeSet, HashMap};

/// One aggregated row in the sessions table. Mirrors the Python
/// `metrics.session_summaries` output schema.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub session_id: String,
    /// Min/max timestamp over MAIN-thread turns only.
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// Project + model from the last (latest-ts) main-thread turn.
    pub project_cwd: String,
    pub model: String,
    pub main_turns: usize,
    /// Count of distinct `subagent_id` within this session.
    pub subagent_count: usize,
    /// max(prompt_tokens / context_window_for(model)) over main turns.
    pub peak_context_pct: f64,
    pub peak_prompt_tokens: u64,
    pub main_cost_weighted: f64,
    pub subagent_cost_weighted: f64,
    pub total_cost_weighted: f64,
    /// (end - start) in seconds; used by the degenerate-session filter.
    pub duration_s: f64,
}

/// Group `turns` by `session_id` and aggregate. Main-thread rows drive
/// start/end/model/project/peak/main-cost; subagent rows (same session_id)
/// contribute cost + distinct count. Sessions with no main-thread rows are
/// dropped. Output is sorted by `start` ascending.
pub fn session_summaries(turns: &[Turn], w: &CostWeights) -> Vec<SessionSummary> {
    let mut groups: HashMap<&str, Vec<&Turn>> = HashMap::new();
    for t in turns {
        groups.entry(t.session_id.as_str()).or_default().push(t);
    }

    let mut out: Vec<SessionSummary> = Vec::new();
    for (sid, rows) in groups {
        let mains: Vec<&Turn> = rows.iter().copied().filter(|t| !t.is_subagent).collect();
        if mains.is_empty() {
            continue; // a session with only subagent rows is dropped
        }

        let start = mains.iter().map(|t| t.ts).min().unwrap();
        let end = mains.iter().map(|t| t.ts).max().unwrap();
        let last_main = mains.iter().max_by_key(|t| t.ts).unwrap();
        let project_cwd = last_main.project_cwd.clone();
        let model = last_main.model.clone();
        let main_turns = mains.len();
        let peak_prompt_tokens = mains.iter().map(|t| prompt_tokens(t)).max().unwrap_or(0);
        let peak_context_pct = mains
            .iter()
            .map(|t| {
                let window = crate::config::context_window_for(&t.model);
                prompt_tokens(t) as f64 / window as f64
            })
            .fold(0.0_f64, f64::max);
        let main_cost_weighted: f64 = mains.iter().map(|t| cost_weighted(t, w)).sum();

        let subs: Vec<&Turn> = rows.iter().copied().filter(|t| t.is_subagent).collect();
        let subagent_cost_weighted: f64 = subs.iter().map(|t| cost_weighted(t, w)).sum();
        let mut sub_ids: BTreeSet<&str> = BTreeSet::new();
        for t in &subs {
            if let Some(id) = &t.subagent_id {
                sub_ids.insert(id.as_str());
            }
        }
        let subagent_count = sub_ids.len();

        let total_cost_weighted = main_cost_weighted + subagent_cost_weighted;
        let duration_s = (end - start).num_milliseconds() as f64 / 1000.0;

        out.push(SessionSummary {
            session_id: sid.to_string(),
            start,
            end,
            project_cwd,
            model,
            main_turns,
            subagent_count,
            peak_context_pct,
            peak_prompt_tokens,
            main_cost_weighted,
            subagent_cost_weighted,
            total_cost_weighted,
            duration_s,
        });
    }

    out.sort_by_key(|s| s.start);
    out
}

use std::cmp::Ordering;

/// How the sessions table is ordered. Selected in the table's sort control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Chronological, // by start ascending
    PeakCtx,       // by peak_context_pct descending
    TotalCost,     // by total_cost_weighted descending
}

/// Sort `sessions` in place per `key`. Descending modes use a NaN-safe
/// comparison (NaN never appears here, but partial_cmp must be unwrapped).
pub fn sort_sessions(sessions: &mut [SessionSummary], key: SortKey) {
    match key {
        SortKey::Chronological => sessions.sort_by_key(|a| a.start),
        SortKey::PeakCtx => sessions.sort_by(|a, b| {
            b.peak_context_pct
                .partial_cmp(&a.peak_context_pct)
                .unwrap_or(Ordering::Equal)
        }),
        SortKey::TotalCost => sessions.sort_by(|a, b| {
            b.total_cost_weighted
                .partial_cmp(&a.total_cost_weighted)
                .unwrap_or(Ordering::Equal)
        }),
    }
}

/// Drop "degenerate" sessions (fewer than `min_turns` main turns OR shorter
/// than `min_duration_s`). Returns `(kept, hidden_count)`.
pub fn hide_degenerate(
    sessions: Vec<SessionSummary>,
    min_turns: usize,
    min_duration_s: f64,
) -> (Vec<SessionSummary>, usize) {
    let total = sessions.len();
    let kept: Vec<SessionSummary> = sessions
        .into_iter()
        .filter(|s| s.main_turns >= min_turns && s.duration_s >= min_duration_s)
        .collect();
    let hidden = total - kept.len();
    (kept, hidden)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};
    use std::path::PathBuf;

    fn turn(input: u64, cc: u64, cr: u64, output: u64) -> Turn {
        Turn {
            ts: Utc::now(),
            session_id: String::new(),
            subagent_id: None,
            is_subagent: false,
            project_cwd: String::new(),
            model: String::new(),
            version: String::new(),
            input_tokens: input,
            output_tokens: output,
            cache_creation_input_tokens: cc,
            cache_read_input_tokens: cr,
            source_file: PathBuf::new(),
            is_rate_limit_error: false,
        }
    }

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    /// Full turn builder for session tests.
    #[allow(clippy::too_many_arguments)]
    fn trow(
        session_id: &str,
        ts: DateTime<Utc>,
        is_subagent: bool,
        subagent_id: Option<&str>,
        project: &str,
        model: &str,
        input: u64,
        output: u64,
    ) -> Turn {
        Turn {
            ts,
            session_id: session_id.to_string(),
            subagent_id: subagent_id.map(|s| s.to_string()),
            is_subagent,
            project_cwd: project.to_string(),
            model: model.to_string(),
            version: String::new(),
            input_tokens: input,
            output_tokens: output,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            source_file: PathBuf::new(),
            is_rate_limit_error: false,
        }
    }

    #[test]
    fn prompt_tokens_sums_inputs_excludes_output() {
        // 100 + 200 + 300 = 600; output 999 ignored.
        let t = turn(100, 200, 300, 999);
        assert_eq!(prompt_tokens(&t), 600);
    }

    #[test]
    fn session_summaries_groups_and_aggregates_main_rows() {
        let turns = vec![
            trow(
                "s1",
                utc(2026, 5, 24, 10, 0),
                false,
                None,
                "/home/u/proj",
                "claude-opus-4-7",
                100,
                10,
            ),
            trow(
                "s1",
                utc(2026, 5, 24, 11, 0),
                false,
                None,
                "/home/u/proj",
                "claude-opus-4-7",
                300,
                20,
            ),
        ];
        let out = session_summaries(&turns, &CostWeights::default());
        assert_eq!(out.len(), 1);
        let s = &out[0];
        assert_eq!(s.session_id, "s1");
        assert_eq!(s.start, utc(2026, 5, 24, 10, 0));
        assert_eq!(s.end, utc(2026, 5, 24, 11, 0));
        assert_eq!(s.main_turns, 2);
        assert_eq!(s.subagent_count, 0);
        assert_eq!(s.peak_prompt_tokens, 300); // max(100, 300)
                                               // peak ctx% = 300 / 1_000_000 (opus 4-7 window)
        assert!((s.peak_context_pct - 300.0 / 1_000_000.0).abs() < 1e-12);
        // cost_weighted: input weight 1.0, output weight 5.0 → 100+50 and 300+100
        assert!((s.main_cost_weighted - (150.0 + 400.0)).abs() < 1e-9);
        assert!((s.total_cost_weighted - s.main_cost_weighted).abs() < 1e-9);
    }

    #[test]
    fn session_summaries_joins_subagents_by_distinct_id() {
        let turns = vec![
            trow(
                "s1",
                utc(2026, 5, 24, 10, 0),
                false,
                None,
                "/p",
                "claude-sonnet-4-5",
                100,
                0,
            ),
            trow(
                "s1",
                utc(2026, 5, 24, 10, 5),
                true,
                Some("a1"),
                "/p",
                "claude-sonnet-4-5",
                0,
                50,
            ),
            trow(
                "s1",
                utc(2026, 5, 24, 10, 6),
                true,
                Some("a1"),
                "/p",
                "claude-sonnet-4-5",
                0,
                50,
            ),
            trow(
                "s1",
                utc(2026, 5, 24, 10, 7),
                true,
                Some("a2"),
                "/p",
                "claude-sonnet-4-5",
                0,
                50,
            ),
        ];
        let out = session_summaries(&turns, &CostWeights::default());
        assert_eq!(out.len(), 1);
        let s = &out[0];
        assert_eq!(s.main_turns, 1);
        assert_eq!(s.subagent_count, 2); // a1, a2 distinct
                                         // subagent cost = 3 rows * output 50 * weight 5.0 = 750
        assert!((s.subagent_cost_weighted - 750.0).abs() < 1e-9);
        // total = main (100*1.0) + sub (750)
        assert!((s.total_cost_weighted - (100.0 + 750.0)).abs() < 1e-9);
    }

    #[test]
    fn session_summaries_drops_session_with_only_subagent_rows() {
        let turns = vec![trow(
            "s1",
            utc(2026, 5, 24, 10, 0),
            true,
            Some("a1"),
            "/p",
            "m",
            0,
            50,
        )];
        assert!(session_summaries(&turns, &CostWeights::default()).is_empty());
    }

    #[test]
    fn session_summaries_last_main_sets_project_and_model() {
        let turns = vec![
            trow(
                "s1",
                utc(2026, 5, 24, 10, 0),
                false,
                None,
                "/old",
                "claude-haiku-4-5",
                10,
                0,
            ),
            trow(
                "s1",
                utc(2026, 5, 24, 12, 0),
                false,
                None,
                "/new",
                "claude-opus-4-7",
                10,
                0,
            ),
        ];
        let s = &session_summaries(&turns, &CostWeights::default())[0];
        assert_eq!(s.project_cwd, "/new");
        assert_eq!(s.model, "claude-opus-4-7");
    }

    #[test]
    fn session_summaries_sorted_by_start_ascending() {
        let turns = vec![
            trow(
                "late",
                utc(2026, 5, 24, 15, 0),
                false,
                None,
                "/p",
                "m",
                1,
                0,
            ),
            trow(
                "early",
                utc(2026, 5, 24, 9, 0),
                false,
                None,
                "/p",
                "m",
                1,
                0,
            ),
        ];
        let out = session_summaries(&turns, &CostWeights::default());
        assert_eq!(out[0].session_id, "early");
        assert_eq!(out[1].session_id, "late");
    }

    fn summary(
        id: &str,
        start: DateTime<Utc>,
        main_turns: usize,
        duration_s: f64,
        ctx: f64,
        total: f64,
    ) -> SessionSummary {
        SessionSummary {
            session_id: id.to_string(),
            start,
            end: start,
            project_cwd: String::new(),
            model: String::new(),
            main_turns,
            subagent_count: 0,
            peak_context_pct: ctx,
            peak_prompt_tokens: 0,
            main_cost_weighted: total,
            subagent_cost_weighted: 0.0,
            total_cost_weighted: total,
            duration_s,
        }
    }

    #[test]
    fn sort_sessions_peak_ctx_descending() {
        let mut v = vec![
            summary("lo", utc(2026, 5, 24, 9, 0), 10, 100.0, 0.1, 1.0),
            summary("hi", utc(2026, 5, 24, 8, 0), 10, 100.0, 0.9, 1.0),
        ];
        sort_sessions(&mut v, SortKey::PeakCtx);
        assert_eq!(v[0].session_id, "hi");
    }

    #[test]
    fn sort_sessions_total_cost_descending() {
        let mut v = vec![
            summary("cheap", utc(2026, 5, 24, 9, 0), 10, 100.0, 0.1, 1.0),
            summary("pricey", utc(2026, 5, 24, 8, 0), 10, 100.0, 0.1, 9.0),
        ];
        sort_sessions(&mut v, SortKey::TotalCost);
        assert_eq!(v[0].session_id, "pricey");
    }

    #[test]
    fn sort_sessions_chronological_ascending() {
        let mut v = vec![
            summary("b", utc(2026, 5, 24, 15, 0), 10, 100.0, 0.1, 1.0),
            summary("a", utc(2026, 5, 24, 9, 0), 10, 100.0, 0.1, 1.0),
        ];
        sort_sessions(&mut v, SortKey::Chronological);
        assert_eq!(v[0].session_id, "a");
    }

    #[test]
    fn hide_degenerate_drops_short_and_low_turn_sessions() {
        let v = vec![
            summary("keep", utc(2026, 5, 24, 9, 0), 10, 120.0, 0.1, 1.0),
            summary("few_turns", utc(2026, 5, 24, 9, 0), 2, 120.0, 0.1, 1.0),
            summary("too_short", utc(2026, 5, 24, 9, 0), 10, 30.0, 0.1, 1.0),
        ];
        let (kept, hidden) = hide_degenerate(v, 5, 60.0);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].session_id, "keep");
        assert_eq!(hidden, 2);
    }
}
