use polysqueeze::api::GammaClient;

#[test]
fn test_gamma_client_default() {
    let client = GammaClient::new();
    assert_eq!(
        client.gamma_url("markets"),
        "https://gamma-api.polymarket.com/markets"
    );
}

#[test]
fn test_gamma_client_custom_url() {
    let client = GammaClient::new().with_base_url("http://localhost:8080");
    assert_eq!(
        client.gamma_url("markets"),
        "http://localhost:8080/markets"
    );
}

#[test]
fn test_gamma_client_url_building() {
    let client = GammaClient::new();
    
    // Test various paths
    assert_eq!(client.gamma_url("events"), "https://gamma-api.polymarket.com/events");
    assert_eq!(client.gamma_url("tags"), "https://gamma-api.polymarket.com/tags");
    assert_eq!(client.gamma_url("sports"), "https://gamma-api.polymarket.com/sports");
    
    // Test trailing slash handling
    assert_eq!(client.gamma_url("/markets/"), "https://gamma-api.polymarket.com/markets/");
}

#[test]
fn test_gamma_client_clone() {
    let client = GammaClient::new();
    let cloned = client.clone();
    assert_eq!(
        cloned.gamma_url("markets"),
        "https://gamma-api.polymarket.com/markets"
    );
}
