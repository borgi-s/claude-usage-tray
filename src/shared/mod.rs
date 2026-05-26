//! State shared between the polling thread, the tray UI thread, and the
//! dashboard window thread. Wrapped in Arc<RwLock<...>> for safe concurrent
//! access.

pub mod snapshot;

use std::sync::{Arc, RwLock};

pub type SharedSnapshot = Arc<RwLock<snapshot::AppSnapshot>>;

pub fn new_shared_snapshot() -> SharedSnapshot {
    Arc::new(RwLock::new(snapshot::AppSnapshot::default()))
}

use crate::settings::Settings;

pub type SharedSettings = Arc<RwLock<Settings>>;

/// Build the shared settings store by loading `settings.toml` (defaults on any
/// error). The sole place the file is read at startup.
pub fn new_shared_settings() -> SharedSettings {
    Arc::new(RwLock::new(crate::settings::load()))
}
