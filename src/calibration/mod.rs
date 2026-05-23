//! Local calibration math: derives 5h + weekly caps from the calibration log,
//! computes the current live util, and (ahead of Stage 6) a per-hour cap series.

pub mod anchors;
pub mod hourly;
pub mod live;

/// Which window kind to compute against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    FiveHour,
    Weekly,
}
