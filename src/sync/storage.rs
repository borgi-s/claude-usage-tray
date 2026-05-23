//! Uploads objects to Supabase Storage over HTTP. The `ObjectStore` trait lets
//! the orchestration in `sync::mod` be tested with a fake (no network).

use crate::sync::config::SyncConfig;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("network error: {0}")]
    Network(String),
    #[error("storage returned HTTP {0}")]
    Http(u16),
}

/// Abstract object sink. `object_path` is the full key including the user
/// prefix, e.g. "borgi/cache.parquet".
pub trait ObjectStore {
    fn put(&self, object_path: &str, content_type: &str, bytes: &[u8]) -> Result<(), StorageError>;
}

/// Supabase Storage REST client. Uploads via `PUT /storage/v1/object/{bucket}/{key}`
/// with `x-upsert: true` so existing objects are overwritten.
// No `derive(Debug)`: holds the service_role_key; see SyncConfig's redacted Debug.
pub struct SupabaseStore {
    agent: ureq::Agent,
    base_url: String, // trimmed, no trailing slash
    key: String,
    bucket: String,
}

impl SupabaseStore {
    pub fn new(cfg: &SyncConfig) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(30))
            .build();
        SupabaseStore {
            agent,
            base_url: cfg.url.trim_end_matches('/').to_string(),
            key: cfg.service_role_key.clone(),
            bucket: cfg.bucket.clone(),
        }
    }

    pub fn object_url(&self, object_path: &str) -> String {
        format!("{}/storage/v1/object/{}/{}", self.base_url, self.bucket, object_path)
    }
}

impl ObjectStore for SupabaseStore {
    fn put(&self, object_path: &str, content_type: &str, bytes: &[u8]) -> Result<(), StorageError> {
        let url = self.object_url(object_path);
        let resp = self
            .agent
            .put(&url)
            .set("Authorization", &format!("Bearer {}", self.key))
            .set("apikey", &self.key)
            .set("x-upsert", "true")
            .set("Content-Type", content_type)
            .send_bytes(bytes);

        match resp {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(code, _)) => Err(StorageError::Http(code)),
            Err(e) => Err(StorageError::Network(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::config::SyncConfig;

    #[test]
    fn supabase_store_builds_object_url() {
        let cfg = SyncConfig {
            url: "https://x.supabase.co".into(),
            service_role_key: "key123".into(),
            bucket: "usage-tracker".into(),
            prefix: "borgi".into(),
        };
        let store = SupabaseStore::new(&cfg);
        assert_eq!(
            store.object_url("borgi/cache.parquet"),
            "https://x.supabase.co/storage/v1/object/usage-tracker/borgi/cache.parquet"
        );
    }

    #[test]
    fn trailing_slash_in_url_is_trimmed() {
        let cfg = SyncConfig {
            url: "https://x.supabase.co/".into(),
            service_role_key: "k".into(),
            bucket: "b".into(),
            prefix: "p".into(),
        };
        let store = SupabaseStore::new(&cfg);
        assert_eq!(store.object_url("p/caps.json"), "https://x.supabase.co/storage/v1/object/b/p/caps.json");
    }
}
