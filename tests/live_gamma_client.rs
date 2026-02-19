use polysqueeze::client::ClobClient;
use polysqueeze::types::GammaListParams;
use std::env;

fn live_tests_enabled() -> bool {
    env::var("RUN_GAMMA_TESTS").is_ok()
}

#[tokio::test]
async fn clob_client_get_markets_live() {
    if !live_tests_enabled() {
        eprintln!("Skipping ClobClient::get_markets live test (set RUN_GAMMA_TESTS=1)");
        return;
    }

    // ClobClient uses the Gamma base internally, but we can set host to the real URL for clarity
    let client = ClobClient::new("https://clob.polymarket.com");
    let params = GammaListParams {
        limit: Some(5),
        ..Default::default()
    };
    let markets = client.get_markets(None, Some(&params)).await;
    assert!(
        markets.is_ok(),
        "get_markets responded with error {:?}",
        markets.map_err(|e| format!("{e}"))
    );

    let response = markets.unwrap();
    assert!(!response.data.is_empty(), "Expected at least one market");
    assert!(
        response.data.iter().any(|m| !m.clob_token_ids.is_empty()),
        "Markets should contain CLOB token ids"
    );
    println!("markets are: {:?}", response.data[1]);
}

#[tokio::test]
async fn clob_client_get_markets_with_all_params_live() {
    if std::env::var("RUN_GAMMA_TESTS").unwrap_or_default() != "1" {
        eprintln!("Skipping ClobClient::get_markets with all params live test (set RUN_GAMMA_TESTS=1)");
        return;
    }

    let client = ClobClient::new("https://clob.polymarket.com").with_gamma_base("https://gamma-api.polymarket.com");

    use rust_decimal::Decimal;

    let params = GammaListParams {
        limit: Some(5),
        closed: Some(false),
        liquidity_num_min: Some(Decimal::from(1000)),
        liquidity_num_max: Some(Decimal::from(1000000)),
        volume_num_min: Some(Decimal::from(10000)),
        cyom: Some(false),
        include_tag: Some(true),
        ..Default::default()
    };

    let markets = client.get_markets(None, Some(&params)).await;
    assert!(
        markets.is_ok(),
        "get_markets with all params responded with error {:?}",
        markets.err()
    );
}
