//! Borderless, always-on-top widget docked over the Windows taskbar. Shows two
//! usage bars (5h + 7d) with live % and reset countdowns. Painted with GDI+
//! (mirrors `crate::tray::icon`); self-driven by a 1-second WM_TIMER that
//! repaints, re-docks over the live taskbar rect, and shows/hides to match
//! `settings.widget_enabled`.

use crate::api::usage::UsageBucket;
use crate::render::LastStatus;
use chrono::{DateTime, Utc};
use windows::Win32::Foundation::RECT;

/// Logical widget size derived from the taskbar's pixel height (so it is
/// DPI-correct without querying DPI). Margin keeps it off the taskbar edges.
const MARGIN_PX: i32 = 4;
/// Widget width as a multiple of its height (two short rows of "label bar % time").
const WIDTH_RATIO: i32 = 6;

/// Compute the widget's screen rectangle from the live taskbar rect and the
/// saved right-anchored offset. Vertically centered in the taskbar band,
/// anchored near the right edge, shifted left by `offset_px`, clamped to stay
/// within the taskbar horizontally.
pub(crate) fn dock_rect(taskbar: RECT, offset_px: i32) -> RECT {
    let tb_h = (taskbar.bottom - taskbar.top).max(1);
    let h = (tb_h - 2 * MARGIN_PX).max(8);
    let w = h * WIDTH_RATIO;

    let y = taskbar.top + (tb_h - h) / 2;

    // Default anchor: right edge minus a margin. Offset shifts it further left.
    let mut x = taskbar.right - w - MARGIN_PX - offset_px;
    // Clamp within the taskbar so it never leaves the band.
    let min_x = taskbar.left + MARGIN_PX;
    let max_x = taskbar.right - w - MARGIN_PX;
    if max_x >= min_x {
        x = x.clamp(min_x, max_x);
    } else {
        x = min_x; // taskbar narrower than the widget; pin left
    }

    RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}

/// Given a widget's current left edge and the live taskbar rect, derive the
/// offset to persist (distance from the right-anchored default position).
pub(crate) fn offset_from_left(taskbar: RECT, left: i32) -> i32 {
    let tb_h = (taskbar.bottom - taskbar.top).max(1);
    let h = (tb_h - 2 * MARGIN_PX).max(8);
    let w = h * WIDTH_RATIO;
    // left = taskbar.right - w - MARGIN - offset  =>  offset = taskbar.right - w - MARGIN - left
    (taskbar.right - w - MARGIN_PX - left).max(0)
}

/// Filled width of a bar given its track width and a utilization in [0, ∞).
pub(crate) fn bar_fill_width(track_w: i32, util: f64) -> i32 {
    let u = util.clamp(0.0, 1.0);
    (track_w as f64 * u).round() as i32
}

/// What one row (5h or 7d) should paint.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RowState {
    /// Normal: util in [0,1], gradient color, "NN%", optional countdown text.
    Data {
        util: f64,
        pct: u8,
        countdown: Option<String>,
    },
    /// util > 100%: full red bar, "!".
    Bang { countdown: Option<String> },
    /// No usable data (rate-limited / error / initial / missing bucket).
    Question,
}

/// Decide a row's visual state, mirroring `icon::compute_visual` but per-bucket.
pub(crate) fn row_state(
    status: &LastStatus,
    bucket: Option<&UsageBucket>,
    now: DateTime<Utc>,
) -> RowState {
    match status {
        LastStatus::Initial | LastStatus::RateLimited | LastStatus::Error(_) => RowState::Question,
        LastStatus::Ok => match bucket {
            None => RowState::Question,
            Some(b) => {
                let countdown = b
                    .resets_at
                    .map(|when| crate::render::format_duration(when - now));
                if b.utilization > 1.0 {
                    RowState::Bang { countdown }
                } else {
                    let pct = (b.utilization.clamp(0.0, 1.0) * 100.0).round() as u8;
                    RowState::Data {
                        util: b.utilization,
                        pct,
                        countdown,
                    }
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn rect(l: i32, t: i32, r: i32, b: i32) -> RECT {
        RECT {
            left: l,
            top: t,
            right: r,
            bottom: b,
        }
    }

    #[test]
    fn dock_rect_centers_vertically_and_anchors_right() {
        // taskbar 0..1920 wide, 48px tall at the bottom of a 1080 screen.
        let tb = rect(0, 1032, 1920, 1080);
        let r = dock_rect(tb, 0);
        let h = 48 - 8; // 40
        let w = h * WIDTH_RATIO; // 240
        assert_eq!(r.bottom - r.top, h);
        assert_eq!(r.right - r.left, w);
        // right edge sits a margin in from the taskbar right edge.
        assert_eq!(r.right, 1920 - MARGIN_PX);
        // vertically centered: top margin == bottom margin == 4.
        assert_eq!(r.top, 1032 + 4);
    }

    #[test]
    fn dock_rect_offset_shifts_left() {
        let tb = rect(0, 1032, 1920, 1080);
        let base = dock_rect(tb, 0);
        let shifted = dock_rect(tb, 100);
        assert_eq!(base.left - shifted.left, 100);
    }

    #[test]
    fn dock_rect_clamps_within_taskbar() {
        let tb = rect(0, 1032, 1920, 1080);
        // Absurd offset would push it off the left edge; must clamp to left margin.
        let r = dock_rect(tb, 100_000);
        assert_eq!(r.left, MARGIN_PX);
    }

    #[test]
    fn offset_from_left_is_inverse_of_dock_rect() {
        let tb = rect(0, 1032, 1920, 1080);
        let r = dock_rect(tb, 137);
        assert_eq!(offset_from_left(tb, r.left), 137);
    }

    #[test]
    fn bar_fill_width_clamps() {
        assert_eq!(bar_fill_width(200, 0.0), 0);
        assert_eq!(bar_fill_width(200, 0.5), 100);
        assert_eq!(bar_fill_width(200, 1.0), 200);
        assert_eq!(bar_fill_width(200, 1.5), 200); // clamps over 100%
    }

    fn bucket(util: f64, resets_in_secs: Option<i64>, now: DateTime<Utc>) -> UsageBucket {
        UsageBucket {
            utilization: util,
            resets_at: resets_in_secs.map(|s| now + Duration::seconds(s)),
        }
    }

    #[test]
    fn row_state_ok_data_has_pct_and_countdown() {
        let now = Utc.with_ymd_and_hms(2026, 5, 27, 12, 0, 0).unwrap();
        let b = bucket(0.57, Some(3600 + 42 * 60), now); // 1h42m
        let rs = row_state(&LastStatus::Ok, Some(&b), now);
        match rs {
            RowState::Data { pct, countdown, .. } => {
                assert_eq!(pct, 57);
                assert_eq!(countdown.as_deref(), Some("1h 42m"));
            }
            other => panic!("expected Data, got {other:?}"),
        }
    }

    #[test]
    fn row_state_over_100_is_bang() {
        let now = Utc::now();
        let b = bucket(1.2, None, now);
        assert!(matches!(
            row_state(&LastStatus::Ok, Some(&b), now),
            RowState::Bang { .. }
        ));
    }

    #[test]
    fn row_state_rate_limited_is_question_even_with_bucket() {
        let now = Utc::now();
        let b = bucket(0.5, Some(60), now);
        assert_eq!(
            row_state(&LastStatus::RateLimited, Some(&b), now),
            RowState::Question
        );
    }

    #[test]
    fn row_state_missing_bucket_is_question() {
        let now = Utc::now();
        assert_eq!(row_state(&LastStatus::Ok, None, now), RowState::Question);
    }
}
