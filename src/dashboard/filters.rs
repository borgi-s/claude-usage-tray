//! Global dashboard filters (date / project / model) + display helpers. Pure
//! logic; the rendering of the filter bar lives in `filter_bar.rs`.

use crate::data::parser::Turn;
use chrono::NaiveDate;
use chrono_tz::Tz;
use std::collections::BTreeSet;

/// Display label for a project cwd: its last path component, or "(unknown)"
/// for an empty cwd. Mirrors the Python `short_project`.
pub fn short_project(cwd: &str) -> String {
    if cwd.is_empty() {
        return "(unknown)".to_string();
    }
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    let base = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    if base.is_empty() {
        cwd.to_string()
    } else {
        base.to_string()
    }
}

/// Global filter applied to the whole dashboard. Empty project/model sets mean
/// "all". Date bounds are gated by their `use_*` flag so the date pickers can
/// hold a buffered date even while the bound is inactive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterState {
    pub use_date_from: bool,
    pub date_from: NaiveDate,
    pub use_date_to: bool,
    pub date_to: NaiveDate,
    pub projects: BTreeSet<String>,
    pub models: BTreeSet<String>,
}

impl Default for FilterState {
    fn default() -> Self {
        // Buffer the date pickers at "today" (local) but inactive by default.
        let today = chrono::Local::now().date_naive();
        Self {
            use_date_from: false,
            date_from: today,
            use_date_to: false,
            date_to: today,
            projects: BTreeSet::new(),
            models: BTreeSet::new(),
        }
    }
}

impl FilterState {
    /// Returns the subset of `turns` matching every active filter dimension.
    pub fn apply(&self, turns: &[Turn], tz: Tz) -> Vec<Turn> {
        turns
            .iter()
            .filter(|t| {
                let local_date = t.ts.with_timezone(&tz).date_naive();
                if self.use_date_from && local_date < self.date_from {
                    return false;
                }
                if self.use_date_to && local_date > self.date_to {
                    return false;
                }
                if !self.projects.is_empty() && !self.projects.contains(&t.project_cwd) {
                    return false;
                }
                if !self.models.is_empty() && !self.models.contains(&t.model) {
                    return false;
                }
                true
            })
            .cloned()
            .collect()
    }
}

/// Distinct project cwds present in `turns`, sorted by display label (basename).
pub fn distinct_projects(turns: &[Turn]) -> Vec<String> {
    let set: BTreeSet<&str> = turns.iter().map(|t| t.project_cwd.as_str()).collect();
    let mut v: Vec<String> = set.into_iter().map(|s| s.to_string()).collect();
    v.sort_by_key(|a| short_project(a));
    v
}

/// Distinct model strings present in `turns`, sorted lexically.
pub fn distinct_models(turns: &[Turn]) -> Vec<String> {
    let set: BTreeSet<&str> = turns.iter().map(|t| t.model.as_str()).collect();
    set.into_iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, NaiveDate, TimeZone, Utc};
    use std::path::PathBuf;

    fn tz() -> chrono_tz::Tz {
        crate::settings::CalParams::default().tz
    }

    fn turn_at(session: &str, ts: DateTime<Utc>, project: &str, model: &str) -> Turn {
        Turn {
            ts,
            session_id: session.to_string(),
            subagent_id: None,
            is_subagent: false,
            project_cwd: project.to_string(),
            model: model.to_string(),
            version: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            source_file: PathBuf::new(),
            is_rate_limit_error: false,
        }
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn sample() -> Vec<Turn> {
        vec![
            turn_at(
                "a",
                Utc.with_ymd_and_hms(2026, 5, 20, 12, 0, 0).unwrap(),
                "/p/alpha",
                "claude-opus-4-7",
            ),
            turn_at(
                "b",
                Utc.with_ymd_and_hms(2026, 5, 22, 12, 0, 0).unwrap(),
                "/p/beta",
                "claude-sonnet-4-5",
            ),
            turn_at(
                "c",
                Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap(),
                "/p/alpha",
                "claude-sonnet-4-5",
            ),
        ]
    }

    #[test]
    fn short_project_basename_unix_and_windows() {
        assert_eq!(short_project("/home/u/myproj"), "myproj");
        assert_eq!(short_project(r"C:\Users\u\widget\"), "widget");
    }

    #[test]
    fn short_project_empty_is_unknown() {
        assert_eq!(short_project(""), "(unknown)");
    }

    #[test]
    fn short_project_root_returns_input() {
        assert_eq!(short_project("/"), "/");
    }

    #[test]
    fn apply_empty_filter_keeps_all() {
        let turns = sample();
        let f = FilterState::default();
        assert_eq!(f.apply(&turns, tz()).len(), 3);
    }

    #[test]
    fn apply_date_bounds_inclusive_local() {
        let turns = sample();
        let f = FilterState {
            use_date_from: true,
            date_from: d(2026, 5, 22),
            use_date_to: true,
            date_to: d(2026, 5, 24),
            ..Default::default()
        };
        // keeps 22nd and 24th, drops 20th.
        assert_eq!(f.apply(&turns, tz()).len(), 2);
    }

    #[test]
    fn apply_project_set_filters() {
        let turns = sample();
        let f = FilterState {
            projects: ["/p/alpha".to_string()].into(),
            ..Default::default()
        };
        let kept = f.apply(&turns, tz());
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().all(|t| t.project_cwd == "/p/alpha"));
    }

    #[test]
    fn apply_model_set_filters() {
        let turns = sample();
        let f = FilterState {
            models: ["claude-sonnet-4-5".to_string()].into(),
            ..Default::default()
        };
        assert_eq!(f.apply(&turns, tz()).len(), 2);
    }

    #[test]
    fn distinct_projects_and_models_dedup() {
        let turns = sample();
        assert_eq!(
            distinct_projects(&turns),
            vec!["/p/alpha".to_string(), "/p/beta".to_string()]
        );
        assert_eq!(
            distinct_models(&turns),
            vec![
                "claude-opus-4-7".to_string(),
                "claude-sonnet-4-5".to_string()
            ]
        );
    }
}
