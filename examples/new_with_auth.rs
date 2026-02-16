use polysqueeze::{client::ClobClient, errors::Result};
use std::env;

/// Helper to fail fast if a required environment variable is missing.
fn env_var(key: &str) -> String {
    env::var(key).expect(&format!("{} must be set for the new_with_auth example", key))
}

#[tokio::main]
async fn main() -> Result<()> {
    let private_key = env_var("POLY_PRIVATE_KEY");
    let funder = env::var("POLY_FUNDER").ok();

    let client = ClobClient::new_with_auth(&private_key, funder.as_deref()).await?;
    let keys = client.get_api_keys().await?;

    println!("api keys: {}", keys.len());
    Ok(())
}
