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

/// Supabase Storage REST client. Uploads via `POST /storage/v1/object/{bucket}/{key}`
/// with `x-upsert: true` so existing objects are overwritten (the standard
/// Supabase upload; PUT is update-only and won't create new objects).
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
        format!(
            "{}/storage/v1/object/{}/{}",
            self.base_url, self.bucket, object_path
        )
    }
}

/// Max upload attempts before giving up on a transient (429/5xx/network) error.
const MAX_ATTEMPTS: u32 = 3;

impl ObjectStore for SupabaseStore {
    fn put(&self, object_path: &str, content_type: &str, bytes: &[u8]) -> Result<(), StorageError> {
        let url = self.object_url(object_path);
        // Supabase's standard upload is POST (create) with `x-upsert: true` to
        // overwrite — matching supabase-py's `.upload(upsert=true)`. A PUT here
        // hits the "update existing object" path and won't create a new file.
        //
        // Transient failures (429 throttling, 5xx, network blips) are routine
        // for Storage; retry with exponential backoff. 4xx other than 429 are
        // permanent (auth/validation) and fail immediately.
        for attempt in 1..=MAX_ATTEMPTS {
            let resp = self
                .agent
                .post(&url)
                .set("Authorization", &format!("Bearer {}", self.key))
                .set("apikey", &self.key)
                .set("x-upsert", "true")
                .set("Content-Type", content_type)
                .send_bytes(bytes);

            let retryable = match resp {
                Ok(r) => {
                    // ureq only routes non-2xx to `Error::Status`, but Supabase
                    // Storage can return HTTP 200 with an error JSON body (e.g.
                    // RLS/policy rejections). Inspect the body so a logical
                    // failure isn't silently reported as success.
                    let body = r.into_string().unwrap_or_default();
                    if body_indicates_error(&body) {
                        return Err(StorageError::Network(format!(
                            "upload rejected with 2xx + error body: {}",
                            body.chars().take(200).collect::<String>()
                        )));
                    }
                    return Ok(());
                }
                Err(ureq::Error::Status(code, _)) => {
                    let transient = code == 429 || (500..=599).contains(&code);
                    if !transient || attempt == MAX_ATTEMPTS {
                        return Err(StorageError::Http(code));
                    }
                    true
                }
                Err(e) => {
                    if attempt == MAX_ATTEMPTS {
                        return Err(StorageError::Network(e.to_string()));
                    }
                    true
                }
            };

            if retryable {
                // 500ms, 1000ms backoff between the (≤2) retries.
                std::thread::sleep(std::time::Duration::from_millis(
                    500 * 2u64.pow(attempt - 1),
                ));
            }
        }
        unreachable!("loop returns on the final attempt");
    }
}

/// Heuristic: does a Supabase Storage 2xx body actually carry an error payload?
/// A successful upload returns `{"Key":"bucket/path"}` (or `{"Id":..,"Key":..}`);
/// a failure can come back 200 with `{"statusCode":..,"error":..,"message":..}`.
fn body_indicates_error(body: &str) -> bool {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return false;
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(v) => v.get("error").is_some() || v.get("statusCode").is_some(),
        // A non-JSON 2xx body isn't a recognized error shape — treat as success
        // to avoid false positives.
        Err(_) => false,
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
        assert_eq!(
            store.object_url("p/caps.json"),
            "https://x.supabase.co/storage/v1/object/b/p/caps.json"
        );
    }

    #[test]
    fn body_indicates_error_detects_supabase_error_shapes() {
        assert!(body_indicates_error(
            r#"{"statusCode":"403","error":"Unauthorized","message":"new row violates RLS"}"#
        ));
        assert!(body_indicates_error(r#"{"error":"Bucket not found"}"#));
        // Success shapes and empty/non-JSON bodies are NOT errors.
        assert!(!body_indicates_error(
            r#"{"Key":"usage-tracker/borgi/cache.parquet"}"#
        ));
        assert!(!body_indicates_error(
            r#"{"Id":"abc","Key":"b/p/caps.json"}"#
        ));
        assert!(!body_indicates_error(""));
        assert!(!body_indicates_error("OK"));
    }
}
