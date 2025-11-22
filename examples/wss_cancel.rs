use polysqueeze::{client::ClobClient, errors::Result};
use std::env;

/// Fail fast when an expected environment variable is missing.
fn env_var(key: &str) -> String {
    env::var(key).expect(&format!("{} must be set for the cancel example", key))
}

#[tokio::main]
async fn main() -> Result<()> {
    let base_url =
        env::var("POLY_API_URL").unwrap_or_else(|_| "https://clob.polymarket.com".into());
    let private_key = env_var("POLY_PRIVATE_KEY");
    let chain_id = env::var("POLY_CHAIN_ID")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(137);
    let order_id = env_var("POLY_WSS_ORDER_ID");

    let l1_client = ClobClient::with_l1_headers(&base_url, &private_key, chain_id);
    let creds = l1_client.create_or_derive_api_key(None).await?;
    let client = ClobClient::with_l2_headers(&base_url, &private_key, chain_id, creds);

    println!("Cancelling order {order_id}...");
    let response = client.cancel(&order_id).await?;
    println!("Cancel response: {response:#}");

    Ok(())
}
