use polysqueeze::DataApiClient;
use std::env;

fn data_api_user() -> Option<String> {
    if env::var("RUN_DATA_API_TESTS").is_err() {
        eprintln!("Skipping data API live tests (set RUN_DATA_API_TESTS=1 to enable)");
        return None;
    }

    match env::var("POLY_FUNDER") {
        Ok(funder) if !funder.is_empty() => Some(funder),
        _ => {
            eprintln!("Skipping data API live tests (set POLY_FUNDER to a funded wallet address)");
            None
        }
    }
}

#[tokio::test]
async fn data_api_value_endpoint() {
    let user = match data_api_user() {
        Some(user) => user,
        None => return,
    };

    let client = DataApiClient::new();
    let response = client
        .get_total_positions_value(&user)
        .await
        .expect("data-api /value request failed");

    assert!(
        response.is_empty()
            || response
                .iter()
                .all(|entry| entry.user.eq_ignore_ascii_case(&user)),
        "Only the requested user should be in the response"
    );
}

#[tokio::test]
async fn data_api_positions_endpoint() {
    let user = match data_api_user() {
        Some(user) => user,
        None => return,
    };

    let client = DataApiClient::new();
    let positions = client
        .get_positions(&user, None)
        .await
        .expect("data-api /positions request failed");

    assert!(
        positions
            .iter()
            .all(|position| position.proxy_wallet.eq_ignore_ascii_case(&user)),
        "Positions must belong to the requested user"
    );
}
