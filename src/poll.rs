use crate::api::credentials::Credentials;
use crate::api::usage::{fetch_usage, FetchError, UsageSnapshot};
use crate::log::calibration::append_to_default_path;

/// Shared by `--watch` and tray mode: do one poll, write a calibration sample on
/// success (errors are warned + swallowed), return the fetch result.
pub(crate) fn poll_once(creds: &Credentials) -> Result<UsageSnapshot, FetchError> {
    let snap = fetch_usage(creds)?;
    if let Err(e) = append_to_default_path(&snap, creds) {
        tracing::warn!(error = %e, "calibration log write failed");
    }
    Ok(snap)
}
