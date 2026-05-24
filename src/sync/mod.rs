//! Stage 7: best-effort upload of cache + calibration log + caps to Supabase
//! Storage, so the polars cloud viewer reads them unchanged. See
//! docs/superpowers/specs/2026-05-23-stage-7-supabase-sync-design.md.

pub mod config;
pub mod export;
pub mod storage;

use crate::api::credentials::Credentials;
use crate::log::calibration::CalibrationSample;
use crate::shared::snapshot::AppSnapshot;
use crate::sync::config::SyncConfig;
use crate::sync::storage::{ObjectStore, SupabaseStore};

/// Builds the three buffers and uploads them under the configured prefix.
pub struct Syncer<S: ObjectStore> {
    config: SyncConfig,
    store: S,
}

impl Syncer<SupabaseStore> {
    /// Construct from `.env`. `Ok(None)` means sync is not configured.
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        match crate::sync::config::from_env()? {
            Some(config) => {
                let store = SupabaseStore::new(&config);
                Ok(Some(Syncer { config, store }))
            }
            None => Ok(None),
        }
    }
}

impl<S: ObjectStore> Syncer<S> {
    /// Best-effort: build + upload all three objects. Per-object errors are
    /// logged and skipped; this never returns an error to the poll loop.
    pub fn run_once(
        &self,
        snapshot: &AppSnapshot,
        creds: &Credentials,
        samples: &[CalibrationSample],
    ) {
        self.put_buffer(
            "cache.parquet",
            "application/octet-stream",
            crate::sync::export::cache_parquet(&snapshot.turns),
        );
        self.put_buffer(
            "calibration_log.parquet",
            "application/octet-stream",
            crate::sync::export::calibration_log_parquet(samples),
        );
        self.put_buffer(
            "caps.json",
            "application/json",
            crate::sync::export::caps_json(snapshot, creds),
        );
    }

    fn put_buffer(&self, name: &str, content_type: &str, built: anyhow::Result<Vec<u8>>) {
        let object_path = format!("{}/{}", self.config.prefix, name);
        match built {
            Ok(bytes) => match self.store.put(&object_path, content_type, &bytes) {
                Ok(()) => tracing::debug!(object = %object_path, bytes = bytes.len(), "synced"),
                Err(e) => tracing::warn!(object = %object_path, error = %e, "upload failed"),
            },
            Err(e) => tracing::warn!(object = %object_path, error = %e, "serialization failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::storage::StorageError;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeStore {
        puts: Mutex<Vec<(String, String, usize)>>, // (object_path, content_type, byte_len)
    }
    impl ObjectStore for FakeStore {
        fn put(
            &self,
            object_path: &str,
            content_type: &str,
            bytes: &[u8],
        ) -> Result<(), StorageError> {
            self.puts
                .lock()
                .unwrap()
                .push((object_path.into(), content_type.into(), bytes.len()));
            Ok(())
        }
    }

    fn cfg() -> SyncConfig {
        SyncConfig {
            url: "https://x.supabase.co".into(),
            service_role_key: "k".into(),
            bucket: "b".into(),
            prefix: "borgi".into(),
        }
    }

    #[test]
    fn run_once_uploads_three_prefixed_objects() {
        let syncer = Syncer {
            config: cfg(),
            store: FakeStore::default(),
        };
        let creds = Credentials {
            access_token: "t".into(),
            subscription_type: "pro".into(),
            rate_limit_tier: "default".into(),
        };

        syncer.run_once(&AppSnapshot::default(), &creds, &[]);

        let puts = syncer.store.puts.lock().unwrap();
        let paths: Vec<&str> = puts.iter().map(|(p, _, _)| p.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "borgi/cache.parquet",
                "borgi/calibration_log.parquet",
                "borgi/caps.json",
            ]
        );
        assert_eq!(puts[0].1, "application/octet-stream");
        assert_eq!(puts[2].1, "application/json");
        assert!(puts.iter().all(|(_, _, len)| *len > 0));
    }

    struct FailFirstStore {
        call: Mutex<u32>,
        attempts: Mutex<Vec<String>>,
    }
    impl ObjectStore for FailFirstStore {
        fn put(
            &self,
            object_path: &str,
            _content_type: &str,
            _bytes: &[u8],
        ) -> Result<(), StorageError> {
            self.attempts.lock().unwrap().push(object_path.into());
            let mut c = self.call.lock().unwrap();
            *c += 1;
            if *c == 1 {
                return Err(StorageError::Http(503));
            }
            Ok(())
        }
    }

    #[test]
    fn run_once_continues_after_a_failed_upload() {
        let store = FailFirstStore {
            call: Mutex::new(0),
            attempts: Mutex::new(Vec::new()),
        };
        let syncer = Syncer {
            config: cfg(),
            store,
        };
        let creds = Credentials {
            access_token: "t".into(),
            subscription_type: "pro".into(),
            rate_limit_tier: "default".into(),
        };

        syncer.run_once(&AppSnapshot::default(), &creds, &[]);

        // All three objects were attempted even though the first put failed.
        let attempts = syncer.store.attempts.lock().unwrap();
        assert_eq!(attempts.len(), 3);
        assert_eq!(attempts[0], "borgi/cache.parquet");
        assert_eq!(attempts[2], "borgi/caps.json");
    }
}
