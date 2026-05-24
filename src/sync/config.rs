//! Reads Supabase sync settings from environment (.env via dotenvy).

use anyhow::{bail, Result};

/// Validated Supabase sync configuration. Absent => sync disabled.
#[derive(Clone, PartialEq)]
pub struct SyncConfig {
    pub url: String,
    pub service_role_key: String,
    pub bucket: String,
    pub prefix: String,
}

// Manual Debug so the service_role_key (a Supabase superuser credential) never
// reaches a log line or error chain via {:?}.
impl std::fmt::Debug for SyncConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncConfig")
            .field("url", &self.url)
            .field("service_role_key", &"[redacted]")
            .field("bucket", &self.bucket)
            .field("prefix", &self.prefix)
            .finish()
    }
}

const DEFAULT_BUCKET: &str = "usage-tracker";

/// A prefix is path-safe if non-empty and only ASCII alphanumerics, `-`, `_`.
/// This is the per-user object-key segment, so it must not contain slashes,
/// spaces, or `.` runs that could escape the prefix or confuse the storage API.
fn is_valid_prefix(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Public entry: load `.env` (best-effort) then read from the environment.
/// Returns `Ok(None)` when sync is not configured (a required var is absent),
/// `Err` when configured but invalid (bad prefix).
pub fn from_env() -> Result<Option<SyncConfig>> {
    // Loads ./.env into the process environment if present. Missing file is fine.
    let _ = dotenvy::dotenv();
    from_env_inner()
}

/// Pure-ish core: reads only from `std::env`, so tests can set vars directly
/// without touching the filesystem.
fn from_env_inner() -> Result<Option<SyncConfig>> {
    let (url, key, prefix) = match (
        std::env::var("SUPABASE_URL").ok(),
        std::env::var("SUPABASE_SERVICE_ROLE_KEY").ok(),
        std::env::var("SUPABASE_USER_PREFIX").ok(),
    ) {
        (Some(u), Some(k), Some(p)) if !u.trim().is_empty() && !k.trim().is_empty() => (u, k, p),
        _ => return Ok(None),
    };

    if !is_valid_prefix(&prefix) {
        bail!("SUPABASE_USER_PREFIX '{prefix}' is invalid: use only letters, digits, '-', '_'");
    }

    let bucket = std::env::var("SUPABASE_BUCKET")
        .ok()
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| DEFAULT_BUCKET.to_string());

    Ok(Some(SyncConfig {
        url,
        service_role_key: key,
        bucket,
        prefix,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_prefixes_accepted() {
        assert!(is_valid_prefix("borgi"));
        assert!(is_valid_prefix("user-1_x"));
    }

    #[test]
    fn invalid_prefixes_rejected() {
        assert!(!is_valid_prefix(""));
        assert!(!is_valid_prefix("a/b"));
        assert!(!is_valid_prefix("has space"));
        assert!(!is_valid_prefix("dots.here"));
    }

    fn clear_env() {
        for k in [
            "SUPABASE_URL",
            "SUPABASE_SERVICE_ROLE_KEY",
            "SUPABASE_BUCKET",
            "SUPABASE_USER_PREFIX",
        ] {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn debug_redacts_service_role_key() {
        let cfg = SyncConfig {
            url: "https://x.supabase.co".into(),
            service_role_key: "super-secret-key".into(),
            bucket: "b".into(),
            prefix: "p".into(),
        };
        let dbg = format!("{cfg:?}");
        assert!(dbg.contains("[redacted]"));
        assert!(!dbg.contains("super-secret-key"));
    }

    #[test]
    fn from_env_behaviors() {
        // 1. Missing required vars => Ok(None).
        clear_env();
        assert_eq!(from_env_inner().unwrap(), None);

        // 2. All required present, default bucket.
        clear_env();
        std::env::set_var("SUPABASE_URL", "https://x.supabase.co");
        std::env::set_var("SUPABASE_SERVICE_ROLE_KEY", "key123");
        std::env::set_var("SUPABASE_USER_PREFIX", "borgi");
        let cfg = from_env_inner().unwrap().unwrap();
        assert_eq!(cfg.url, "https://x.supabase.co");
        assert_eq!(cfg.bucket, "usage-tracker");
        assert_eq!(cfg.prefix, "borgi");

        // 3. Custom bucket honored.
        std::env::set_var("SUPABASE_BUCKET", "team-bucket");
        assert_eq!(from_env_inner().unwrap().unwrap().bucket, "team-bucket");

        // 4. Invalid prefix => Err.
        clear_env();
        std::env::set_var("SUPABASE_URL", "https://x.supabase.co");
        std::env::set_var("SUPABASE_SERVICE_ROLE_KEY", "key123");
        std::env::set_var("SUPABASE_USER_PREFIX", "bad/prefix");
        assert!(from_env_inner().is_err());

        // 5. Whitespace-only URL => Ok(None), treated as absent.
        clear_env();
        std::env::set_var("SUPABASE_URL", "   ");
        std::env::set_var("SUPABASE_SERVICE_ROLE_KEY", "key123");
        std::env::set_var("SUPABASE_USER_PREFIX", "borgi");
        assert_eq!(from_env_inner().unwrap(), None);

        // 6. Present-but-empty SUPABASE_BUCKET falls back to the default.
        clear_env();
        std::env::set_var("SUPABASE_URL", "https://x.supabase.co");
        std::env::set_var("SUPABASE_SERVICE_ROLE_KEY", "key123");
        std::env::set_var("SUPABASE_USER_PREFIX", "borgi");
        std::env::set_var("SUPABASE_BUCKET", "");
        assert_eq!(from_env_inner().unwrap().unwrap().bucket, "usage-tracker");

        clear_env();
    }
}
