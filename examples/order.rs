use polysqueeze::{
    client::{ClobClient, OrderArgs},
    errors::Result,
    types::{GammaListParams, OrderType, Side},
};
use rust_decimal::Decimal;
use std::env;

/// Helper to fail fast if a required environment variable is missing.
fn env_var(key: &str) -> String {
    env::var(key).expect(&format!("{} must be set for the order example", key))
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

    // CLOB auth process, more info here https://docs.polymarket.com/developers/CLOB/authentication
    let l1_client = ClobClient::with_l1_headers(&base_url, &private_key, chain_id);
    let creds = l1_client.create_or_derive_api_key(None).await?;
    let client = ClobClient::with_l2_headers(&base_url, &private_key, chain_id, creds.clone());

    let gamma_params = GammaListParams {
        limit: Some(5),
        liquidity_num_min: Some(Decimal::from(1000)),
        volume_num_min: Some(Decimal::from(10000)),
        ..Default::default()
    };

    // Example using new parameters with builder pattern
    // let gamma_params = GammaListParams::builder()
    //     .limit(10)
    //     .liquidity_num_min(Decimal::from(1000))
    //     .volume_num_min(Decimal::from(10000))
    //     .cyom(false)
    //     .include_tag(true)
    //     .game_id("some-game-id".to_string())
    //     .build();
    let markets_response = client.get_markets(None, Some(&gamma_params)).await?;
    let market = markets_response
        .data
        .get(1) // TODO: for some reason market[0] doesnt countain token id, nor condition id. needs fixing
        .expect("Gamma markets returned no data");

    let token_id = market.clob_token_ids.first().unwrap();

    let book = client.get_order_book(&token_id).await?;
    let best_bid = book.bids.first().expect("order book has no bids").price;
    let best_ask = book.asks.first().expect("order book has no asks").price;
    let book_mid = (best_bid + best_ask) / Decimal::from(2);
    let min_order_size: Decimal = 2.into();
    let order_size = min_order_size;
    let order_price = book_mid;

    let args = OrderArgs::new(&token_id, order_price, order_size, Side::BUY);
    let signed_order = client.create_order(&args, None, None, None).await?;

    let response = client.post_order(signed_order, OrderType::GTC).await?;
    println!("order posted: {}", response);

    Ok(())
}
