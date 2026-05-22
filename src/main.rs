use claude_usage_tray::api::credentials::load_from_default_path;

fn main() -> anyhow::Result<()> {
    let creds = load_from_default_path()?;
    println!("Loaded credentials for sub `{}` tier `{}` (token len {})",
        creds.subscription_type, creds.rate_limit_tier, creds.access_token.len());
    Ok(())
}
