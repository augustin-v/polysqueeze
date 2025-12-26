use polysqueeze::{
    client::ClobClient,
    errors::Result,
    types::{AssetType, BalanceAllowanceParams},
};
use std::env;

/// Helper to fail fast if a required environment variable is missing.
fn env_var(key: &str) -> String {
    env::var(key).expect(&format!(
        "{} must be set for the balance allowance example",
        key
    ))
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

    let l1_client = ClobClient::with_l1_headers(&base_url, &private_key, chain_id);
    let creds = l1_client.create_or_derive_api_key(None).await?;
    let client = ClobClient::with_l2_headers(&base_url, &private_key, chain_id, creds.clone());

    let mut params = BalanceAllowanceParams::default();
    params.asset_type = Some(AssetType::COLLATERAL);

    let balances = client.get_balance_allowance(Some(params)).await?;
    println!("balance allowance response: {balances:#}");

    Ok(())
}
