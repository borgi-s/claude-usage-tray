use claude_usage_tray::api::credentials::{parse_credentials, Credentials};
use std::path::Path;

#[test]
fn parses_well_formed_credentials() {
    let path = Path::new("tests/fixtures/credentials_sample.json");
    let raw = std::fs::read_to_string(path).expect("fixture should exist");
    let creds: Credentials = parse_credentials(&raw).expect("should parse");
    assert_eq!(creds.access_token, "sk-ant-oat01-FAKE_TOKEN_FOR_TESTS");
    assert_eq!(creds.subscription_type, "pro");
    assert_eq!(creds.rate_limit_tier, "default_claude_ai");
}

#[test]
fn rejects_credentials_with_expired_token() {
    let raw = r#"{"claudeAiOauth": {"accessToken":"x","expiresAt":1,"subscriptionType":"pro","rateLimitTier":"default_claude_ai"}}"#;
    let err = parse_credentials(raw).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("expired"),
        "expected 'expired' in error message, got: {msg}"
    );
}

#[test]
fn rejects_credentials_missing_access_token() {
    let raw = r#"{"claudeAiOauth": {"expiresAt":9999999999999,"subscriptionType":"pro","rateLimitTier":"default_claude_ai"}}"#;
    let err = parse_credentials(raw).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("accessToken") || msg.contains("missing"),
        "expected 'accessToken' or 'missing' in error message, got: {msg}",
    );
}
