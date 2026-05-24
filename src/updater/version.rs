//! Pure helpers: parse a GitHub tag into a semver Version, and compare versions.

use semver::Version;

/// Parse a release tag like "v0.8.0" or "0.8.0" into a semver `Version`.
/// Strips a single leading 'v'/'V' before parsing.
pub fn parse_tag(tag: &str) -> Result<Version, semver::Error> {
    let trimmed = tag
        .strip_prefix('v')
        .or_else(|| tag.strip_prefix('V'))
        .unwrap_or(tag);
    Version::parse(trimmed)
}

/// True if `latest` is strictly newer than `current`.
pub fn is_update_available(current: &Version, latest: &Version) -> bool {
    latest > current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tag_strips_leading_v() {
        assert_eq!(parse_tag("v0.8.0").unwrap(), Version::new(0, 8, 0));
        assert_eq!(parse_tag("0.8.0").unwrap(), Version::new(0, 8, 0));
    }

    #[test]
    fn parse_tag_rejects_garbage() {
        assert!(parse_tag("vX.Y.Z").is_err());
        assert!(parse_tag("not-a-version").is_err());
    }

    #[test]
    fn is_update_available_compares_versions() {
        let cur = Version::new(0, 7, 0);
        assert!(is_update_available(&cur, &Version::new(0, 8, 0)));
        assert!(!is_update_available(&cur, &Version::new(0, 7, 0)));
        assert!(!is_update_available(&cur, &Version::new(0, 6, 9)));
    }
}
