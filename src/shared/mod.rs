//! State shared between the polling thread, the tray UI thread, and the
//! dashboard window thread. Wrapped in Arc<RwLock<...>> for safe concurrent
//! access.

pub mod snapshot;

use std::sync::{Arc, RwLock};

pub type SharedSnapshot = Arc<RwLock<snapshot::AppSnapshot>>;

pub fn new_shared_snapshot() -> SharedSnapshot {
    Arc::new(RwLock::new(snapshot::AppSnapshot::default()))
}
