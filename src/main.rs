use anyhow::Result;
use claude_usage_tray::api::credentials::load_from_default_path;
use claude_usage_tray::api::usage::fetch_usage;

fn main() -> Result<()> {
    let creds = load_from_default_path()?;
    let snap = fetch_usage(&creds)?;
    println!("Fetched usage: {snap:#?}");
    Ok(())
}
