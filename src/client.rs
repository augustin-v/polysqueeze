//! High-performance Rust client for Polymarket
//!
//! This module provides a production-ready client for interacting with
//! Polymarket, optimized for high-frequency trading environments.

use crate::api::gamma::GammaClient;
use crate::auth::{create_l1_headers, create_l2_headers};
use crate::errors::{PolyError, Result};
use crate::types::{OrderOptions, PostOrder, SignedOrderRequest};
use alloy_primitives::{Address, U256};
use alloy_signer_local::PrivateKeySigner;
use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_ENGINE;
use chrono::{Duration, Utc};
use reqwest::Client;
use reqwest::header::HeaderName;
use reqwest::{Method, RequestBuilder};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use serde_json::{self, Value};
use std::env;
use std::str::FromStr;
use tracing::warn;

const DEFAULT_CLOB_BASE: &str = "https://clob.polymarket.com";
const DEFAULT_GAMMA_BASE: &str = "https://gamma-api.polymarket.com";
const DEFAULT_WS_BASE: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/";
const DEFAULT_RTDS_BASE: &str = "wss://ws-live-data.polymarket.com";
const DEFAULT_DATA_API_BASE: &str = "https://data-api.polymarket.com";
const GAMMA_MARKETS_LIMIT: u32 = 50;

// Re-export types for compatibility
pub use crate::types::{ApiCredentials as ApiCreds, OrderType, Side};

// Compatibility types
#[derive(Debug, Clone)]
pub struct OrderArgs {
    pub token_id: String,
    pub price: Decimal,
    pub size: Decimal,
    pub side: Side,
}

impl OrderArgs {
    pub fn new(token_id: &str, price: Decimal, size: Decimal, side: Side) -> Self {
        Self {
            token_id: token_id.to_string(),
            price,
            size,
            side,
        }
    }
}

impl Default for OrderArgs {
    fn default() -> Self {
        Self {
            token_id: "".to_string(),
            price: Decimal::ZERO,
            size: Decimal::ZERO,
            side: Side::BUY,
        }
    }
}

/// Client for Polymarket's public data API.
///
/// This client is intentionally light-weight and only serves the
/// public `/value` and `/positions` endpoints, which do not require
/// authentication.
#[derive(Debug, Clone)]
pub struct DataApiClient {
    http_client: Client,
    base_url: String,
}

impl DataApiClient {
    /// Create a data API client using the default base URL.
    pub fn new() -> Self {
        Self {
            http_client: Client::new(),
            base_url: DEFAULT_DATA_API_BASE.to_string(),
        }
    }

    /// Override the base URL (useful for testing or staging).
    pub fn with_base_url(mut self, url: &str) -> Self {
        self.base_url = url.to_string();
        self
    }

    fn data_api_url(&self, path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            base.to_string()
        } else {
            format!("{}/{}", base, path)
        }
    }

    /// Fetch the total positions value for a single user wallet.
    ///
    /// Returns the same structure as the `GET /value` public endpoint.
    pub async fn get_total_positions_value(
        &self,
        user: &str,
    ) -> Result<Vec<crate::types::DataPositionValue>> {
        let response = self
            .http_client
            .get(self.data_api_url("value"))
            .query(&[("user", user)])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to fetch total positions value",
            ));
        }

        response
            .json::<Vec<crate::types::DataPositionValue>>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Retrieve the current open positions for a wallet.
    ///
    /// This wraps the `GET /positions` endpoint and automatically applies
    /// sane defaults when the optional `params` argument is omitted.
    pub async fn get_positions(
        &self,
        user: &str,
        params: Option<crate::types::DataApiPositionsParams>,
    ) -> Result<Vec<crate::types::DataPosition>> {
        let params = params.unwrap_or_default();
        let mut query = params.to_query_params();
        query.push(("user", user.to_string()));

        let response = self
            .http_client
            .get(self.data_api_url("positions"))
            .query(&query)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to fetch positions",
            ));
        }

        response
            .json::<Vec<crate::types::DataPosition>>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }
}

/// Main client for interacting with Polymarket API
pub struct ClobClient {
    http_client: Client,
    base_url: String,
    gamma_base_url: String,
    ws_base_url: String,
    rtds_base_url: String,
    chain_id: u64,
    signer: Option<PrivateKeySigner>,
    api_creds: Option<ApiCreds>,
    order_builder: Option<crate::orders::OrderBuilder>,
    gamma_client: GammaClient,
}

impl ClobClient {
    /// Create a new client
    pub fn new(host: &str) -> Self {
        Self {
            http_client: Client::new(),
            base_url: host.to_string(),
            gamma_base_url: DEFAULT_GAMMA_BASE.to_string(),
            ws_base_url: DEFAULT_WS_BASE.to_string(),
            rtds_base_url: DEFAULT_RTDS_BASE.to_string(),
            chain_id: 137, // Default to Polygon
            signer: None,
            api_creds: None,
            order_builder: None,
            gamma_client: GammaClient::new(),
        }
    }

    /// Create an authenticated client using default base URL and chain ID.
    ///
    /// This performs L1 auth to create/derive API creds, then constructs the L2 client.
    pub async fn new_with_auth(private_key: &str, funder: Option<&str>) -> Result<Self> {
        let l1_client = ClobClient::with_l1_headers(DEFAULT_CLOB_BASE, private_key, 137);
        let creds = l1_client.create_or_derive_api_key(None).await?;
        let mut client = ClobClient::with_l2_headers(DEFAULT_CLOB_BASE, private_key, 137, creds);

        if let Some(funder) = funder {
            client.set_funder(funder)?;
        }

        Ok(client)
    }

    fn encode_cursor(cursor: u64) -> String {
        BASE64_ENGINE.encode(cursor.to_string())
    }

    fn decode_cursor(cursor: &str) -> Option<u64> {
        BASE64_ENGINE
            .decode(cursor)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|s| s.parse::<u64>().ok())
    }

    fn build_url(base: &str, path: &str) -> String {
        let base = base.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            base.to_string()
        } else {
            format!("{}/{}", base, path)
        }
    }

    fn clob_url(&self, path: &str) -> String {
        Self::build_url(&self.base_url, path)
    }

    fn gamma_url(&self, path: &str) -> String {
        Self::build_url(&self.gamma_base_url, path)
    }

    /// Create a client with L1 headers (for authentication)
    pub fn with_l1_headers(host: &str, private_key: &str, chain_id: u64) -> Self {
        let signer = private_key
            .parse::<PrivateKeySigner>()
            .expect("Invalid private key");

        let order_builder = crate::orders::OrderBuilder::new(signer.clone(), None, None);

        Self {
            http_client: Client::new(),
            base_url: host.to_string(),
            gamma_base_url: DEFAULT_GAMMA_BASE.to_string(),
            ws_base_url: DEFAULT_WS_BASE.to_string(),
            rtds_base_url: DEFAULT_RTDS_BASE.to_string(),
            chain_id,
            signer: Some(signer),
            api_creds: None,
            order_builder: Some(order_builder),
            gamma_client: GammaClient::new(),
        }
    }

    /// Create a client with L2 headers (for API key authentication)
    pub fn with_l2_headers(
        host: &str,
        private_key: &str,
        chain_id: u64,
        api_creds: ApiCreds,
    ) -> Self {
        let signer = private_key
            .parse::<PrivateKeySigner>()
            .expect("Invalid private key");

        let order_builder = crate::orders::OrderBuilder::new(signer.clone(), None, None);

        Self {
            http_client: Client::new(),
            base_url: host.to_string(),
            gamma_base_url: DEFAULT_GAMMA_BASE.to_string(),
            ws_base_url: DEFAULT_WS_BASE.to_string(),
            rtds_base_url: DEFAULT_RTDS_BASE.to_string(),
            chain_id,
            signer: Some(signer),
            api_creds: Some(api_creds),
            order_builder: Some(order_builder),
            gamma_client: GammaClient::new(),
        }
        .with_env_funder()
    }

    /// Set API credentials
    pub fn set_api_creds(&mut self, api_creds: ApiCreds) {
        self.api_creds = Some(api_creds);
    }

    /// Override the funder/maker address used when creating signed orders.
    pub fn set_funder(&mut self, funder: &str) -> Result<()> {
        let address = Address::from_str(funder)
            .map_err(|err| PolyError::validation(format!("Invalid funder address: {}", err)))?;

        let order_builder = self
            .order_builder
            .as_mut()
            .ok_or_else(|| PolyError::config("Order builder not initialized"))?;

        order_builder.set_funder(address);
        Ok(())
    }

    fn with_env_funder(mut self) -> Self {
        if let Ok(funder) = env::var("POLY_FUNDER") {
            if let Err(err) = self.set_funder(&funder) {
                warn!("Failed to set funder from POLY_FUNDER: {}", err);
            }
        }
        self
    }

    /// Override the Gamma API base URL
    pub fn with_gamma_base(mut self, url: &str) -> Self {
        self.gamma_base_url = url.to_string();
        self
    }

    /// Override the WebSocket base URL
    pub fn with_ws_base(mut self, url: &str) -> Self {
        self.ws_base_url = url.to_string();
        self
    }

    /// Override the RTDS base URL
    pub fn with_rtds_base(mut self, url: &str) -> Self {
        self.rtds_base_url = url.to_string();
        self
    }

    /// Test basic connectivity
    pub async fn get_ok(&self) -> bool {
        match self.http_client.get(self.clob_url("ok")).send().await {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        }
    }

    /// Get server time
    pub async fn get_server_time(&self) -> Result<u64> {
        let response = self.http_client.get(self.clob_url("time")).send().await?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to get server time",
            ));
        }

        let time_text = response.text().await?;
        let timestamp = time_text
            .trim()
            .parse::<u64>()
            .map_err(|e| PolyError::parse(format!("Invalid timestamp format: {}", e), None))?;

        Ok(timestamp)
    }

    /// Get order book for a token
    pub async fn get_order_book(&self, token_id: &str) -> Result<OrderBookSummary> {
        let response = self
            .http_client
            .get(self.clob_url("book"))
            .query(&[("token_id", token_id)])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to get order book",
            ));
        }

        let order_book: OrderBookSummary = response.json().await?;
        Ok(order_book)
    }

    /// Get midpoint for a token
    pub async fn get_midpoint(&self, token_id: &str) -> Result<MidpointResponse> {
        let response = self
            .http_client
            .get(self.clob_url("midpoint"))
            .query(&[("token_id", token_id)])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to get midpoint",
            ));
        }

        let midpoint: MidpointResponse = response.json().await?;
        Ok(midpoint)
    }

    /// Get spread for a token
    pub async fn get_spread(&self, token_id: &str) -> Result<SpreadResponse> {
        let response = self
            .http_client
            .get(self.clob_url("spread"))
            .query(&[("token_id", token_id)])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to get spread",
            ));
        }

        let spread: SpreadResponse = response.json().await?;
        Ok(spread)
    }

    /// Get spreads for multiple tokens (batch)
    pub async fn get_spreads(
        &self,
        token_ids: &[String],
    ) -> Result<std::collections::HashMap<String, Decimal>> {
        let request_data: Vec<std::collections::HashMap<&str, String>> = token_ids
            .iter()
            .map(|id| {
                let mut map = std::collections::HashMap::new();
                map.insert("token_id", id.clone());
                map
            })
            .collect();

        let response = self
            .http_client
            .post(self.clob_url("spreads"))
            .json(&request_data)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to get batch spreads",
            ));
        }

        response
            .json::<std::collections::HashMap<String, Decimal>>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get price for a token and side
    pub async fn get_price(&self, token_id: &str, side: Side) -> Result<PriceResponse> {
        let response = self
            .http_client
            .get(self.clob_url("price"))
            .query(&[("token_id", token_id), ("side", side.as_str())])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to get price",
            ));
        }

        let price: PriceResponse = response.json().await?;
        Ok(price)
    }

    /// Get tick size for a token
    pub async fn get_tick_size(&self, token_id: &str) -> Result<Decimal> {
        let response = self
            .http_client
            .get(self.clob_url("tick-size"))
            .query(&[("token_id", token_id)])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to get tick size",
            ));
        }

        let tick_size_response: Value = response.json().await?;
        let tick_size = tick_size_response["minimum_tick_size"]
            .as_str()
            .and_then(|s| Decimal::from_str(s).ok())
            .or_else(|| {
                tick_size_response["minimum_tick_size"]
                    .as_f64()
                    .map(|f| Decimal::from_f64(f).unwrap_or(Decimal::ZERO))
            })
            .ok_or_else(|| PolyError::parse("Invalid tick size format", None))?;

        Ok(tick_size)
    }

    /// Create a new API key
    pub async fn create_api_key(&self, nonce: Option<U256>) -> Result<ApiCreds> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::auth("Signer not set"))?;

        let headers = create_l1_headers(signer, nonce)?;
        let req =
            self.create_request_with_headers(Method::POST, "/auth/api-key", headers.into_iter());

        let response = req.send().await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read body>".into());
            return Err(PolyError::api(
                status,
                format!("Failed to create API key: {}", body),
            ));
        }

        Ok(response.json::<ApiCreds>().await?)
    }

    /// Derive an existing API key
    pub async fn derive_api_key(&self, nonce: Option<U256>) -> Result<ApiCreds> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::auth("Signer not set"))?;

        let headers = create_l1_headers(signer, nonce)?;
        let req = self.create_request_with_headers(
            Method::GET,
            "/auth/derive-api-key",
            headers.into_iter(),
        );

        let response = req.send().await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read body>".into());
            return Err(PolyError::api(
                status,
                format!("Failed to derive API key: {}", body),
            ));
        }

        Ok(response.json::<ApiCreds>().await?)
    }

    /// Create or derive API key (try create first, fallback to derive)
    pub async fn create_or_derive_api_key(&self, nonce: Option<U256>) -> Result<ApiCreds> {
        match self.create_api_key(nonce).await {
            Ok(creds) => Ok(creds),
            Err(_) => self.derive_api_key(nonce).await,
        }
    }

    /// Get all API keys for the authenticated user
    pub async fn get_api_keys(&self) -> Result<Vec<String>> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::config("Signer not configured"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyError::config("API credentials not configured"))?;

        let method = Method::GET;
        let endpoint = "/auth/api-keys";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let response = self
            .http_client
            .request(method, self.clob_url(endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        let api_keys_response: crate::types::ApiKeysResponse = response
            .json()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))?;

        Ok(api_keys_response.api_keys)
    }

    /// Delete the current API key
    pub async fn delete_api_key(&self) -> Result<String> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::config("Signer not configured"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyError::config("API credentials not configured"))?;

        let method = Method::DELETE;
        let endpoint = "/auth/api-key";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let response = self
            .http_client
            .request(method, self.clob_url(endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        response
            .text()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Helper to create request with headers
    fn create_request_with_headers(
        &self,
        method: Method,
        endpoint: &str,
        headers: impl Iterator<Item = (&'static str, String)>,
    ) -> RequestBuilder {
        let req = self.http_client.request(method, self.clob_url(endpoint));
        headers.fold(req, |r, (k, v)| r.header(HeaderName::from_static(k), v))
    }

    /// Get neg risk for a token
    pub async fn get_neg_risk(&self, token_id: &str) -> Result<bool> {
        let response = self
            .http_client
            .get(self.clob_url("neg-risk"))
            .query(&[("token_id", token_id)])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to get neg risk",
            ));
        }

        let neg_risk_response: Value = response.json().await?;
        let neg_risk = neg_risk_response["neg_risk"]
            .as_bool()
            .ok_or_else(|| PolyError::parse("Invalid neg risk format", None))?;

        Ok(neg_risk)
    }

    /// Resolve tick size for an order
    async fn resolve_tick_size(
        &self,
        token_id: &str,
        tick_size: Option<Decimal>,
    ) -> Result<Decimal> {
        let min_tick_size = self.get_tick_size(token_id).await?;

        match tick_size {
            None => Ok(min_tick_size),
            Some(t) => {
                if t < min_tick_size {
                    Err(PolyError::validation(format!(
                        "Tick size {} is smaller than min_tick_size {} for token_id: {}",
                        t, min_tick_size, token_id
                    )))
                } else {
                    Ok(t)
                }
            }
        }
    }

    /// Get filled order options
    async fn get_filled_order_options(
        &self,
        token_id: &str,
        options: Option<&OrderOptions>,
    ) -> Result<OrderOptions> {
        let (tick_size, neg_risk, fee_rate_bps) = match options {
            Some(o) => (o.tick_size, o.neg_risk, o.fee_rate_bps),
            None => (None, None, None),
        };

        let tick_size = self.resolve_tick_size(token_id, tick_size).await?;
        let neg_risk = match neg_risk {
            Some(nr) => nr,
            None => self.get_neg_risk(token_id).await?,
        };

        Ok(OrderOptions {
            tick_size: Some(tick_size),
            neg_risk: Some(neg_risk),
            fee_rate_bps,
        })
    }

    /// Check if price is in valid range
    fn is_price_in_range(&self, price: Decimal, tick_size: Decimal) -> bool {
        let min_price = tick_size;
        let max_price = Decimal::ONE - tick_size;
        price >= min_price && price <= max_price
    }

    /// Create an order
    pub async fn create_order(
        &self,
        order_args: &OrderArgs,
        expiration: Option<u64>,
        extras: Option<crate::types::ExtraOrderArgs>,
        options: Option<&OrderOptions>,
    ) -> Result<SignedOrderRequest> {
        let order_builder = self
            .order_builder
            .as_ref()
            .ok_or_else(|| PolyError::auth("Order builder not initialized"))?;

        let create_order_options = self
            .get_filled_order_options(&order_args.token_id, options)
            .await?;

        let expiration = expiration.unwrap_or(0);
        let extras = extras.unwrap_or_default();

        if !self.is_price_in_range(
            order_args.price,
            create_order_options.tick_size.expect("Should be filled"),
        ) {
            return Err(PolyError::validation("Price is not in range of tick_size"));
        }

        order_builder.create_order(
            self.chain_id,
            order_args,
            expiration,
            &extras,
            &create_order_options,
        )
    }

    /// Calculate market price from order book
    async fn calculate_market_price(
        &self,
        token_id: &str,
        side: Side,
        amount: Decimal,
    ) -> Result<Decimal> {
        let book = self.get_order_book(token_id).await?;
        let order_builder = self
            .order_builder
            .as_ref()
            .ok_or_else(|| PolyError::auth("Order builder not initialized"))?;

        // Convert OrderSummary to BookLevel
        let levels: Vec<crate::types::BookLevel> = match side {
            Side::BUY => book
                .asks
                .into_iter()
                .map(|s| crate::types::BookLevel {
                    price: s.price,
                    size: s.size,
                })
                .collect(),
            Side::SELL => book
                .bids
                .into_iter()
                .map(|s| crate::types::BookLevel {
                    price: s.price,
                    size: s.size,
                })
                .collect(),
        };

        order_builder.calculate_market_price(&levels, amount)
    }

    /// Create a market order
    pub async fn create_market_order(
        &self,
        order_args: &crate::types::MarketOrderArgs,
        extras: Option<crate::types::ExtraOrderArgs>,
        options: Option<&OrderOptions>,
    ) -> Result<SignedOrderRequest> {
        let order_builder = self
            .order_builder
            .as_ref()
            .ok_or_else(|| PolyError::auth("Order builder not initialized"))?;

        let create_order_options = self
            .get_filled_order_options(&order_args.token_id, options)
            .await?;

        let extras = extras.unwrap_or_default();
        let price = self
            .calculate_market_price(&order_args.token_id, Side::BUY, order_args.amount)
            .await?;

        if !self.is_price_in_range(
            price,
            create_order_options.tick_size.expect("Should be filled"),
        ) {
            return Err(PolyError::validation("Price is not in range of tick_size"));
        }

        order_builder.create_market_order(
            self.chain_id,
            order_args,
            price,
            &extras,
            &create_order_options,
        )
    }

    /// Post an order to the exchange
    pub async fn post_order(
        &self,
        order: SignedOrderRequest,
        order_type: OrderType,
    ) -> Result<Value> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyError::auth("API credentials not set"))?;

        let body = PostOrder::new(order, api_creds.api_key.clone(), order_type);

        let headers = create_l2_headers(signer, api_creds, "POST", "/order", Some(&body))?;
        if env::var("POLY_LOG_REQUEST").is_ok() {
            if let Ok(body_text) = serde_json::to_string(&body) {
                println!("rust request url    : {}", self.clob_url("/order"));
                println!("rust request method : POST");
                println!("rust request headers: {:?}", headers);
                println!("rust request body   : {}", body_text);
            }
        }
        let req = self.create_request_with_headers(Method::POST, "/order", headers.into_iter());

        let response = req.json(&body).send().await?;
        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                format!("Failed to post order {}", response.text().await.unwrap()),
            ));
        }

        Ok(response.json::<Value>().await?)
    }

    /// Post multiple orders in a single batch request
    ///
    /// # Example
    /// ```ignore
    /// let orders = vec![order1, order2, order3];
    /// let results = client.post_orders(orders, OrderType::GTC).await?;
    /// ```
    pub async fn post_orders(
        &self,
        orders: Vec<SignedOrderRequest>,
        order_type: OrderType,
    ) -> Result<Vec<Value>> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyError::auth("API credentials not set"))?;

        let batch: Vec<PostOrder> = orders
            .into_iter()
            .map(|order| PostOrder::new(order, api_creds.api_key.clone(), order_type.clone()))
            .collect();

        let headers = create_l2_headers(signer, api_creds, "POST", "/orders", Some(&batch))?;

        if env::var("POLY_LOG_REQUEST").is_ok() {
            if let Ok(body_text) = serde_json::to_string(&batch) {
                println!("rust request url    : {}", self.clob_url("/orders"));
                println!("rust request method : POST");
                println!("rust request headers: {:?}", headers);
                println!("rust request body   : {}", body_text);
            }
        }

        let req = self.create_request_with_headers(Method::POST, "/orders", headers.into_iter());

        let response = req.json(&batch).send().await?;
        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                format!(
                    "Failed to post batch orders: {}",
                    response.text().await.unwrap()
                ),
            ));
        }

        Ok(response.json::<Vec<Value>>().await?)
    }

    /// Create and post an order in one call
    pub async fn create_and_post_order(&self, order_args: &OrderArgs) -> Result<Value> {
        let order = self.create_order(order_args, None, None, None).await?;
        self.post_order(order, OrderType::GTC).await
    }

    /// Cancel an order
    pub async fn cancel(&self, order_id: &str) -> Result<Value> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyError::auth("API credentials not set"))?;

        let body = std::collections::HashMap::from([("orderID", order_id)]);

        let headers = create_l2_headers(signer, api_creds, "DELETE", "/order", Some(&body))?;
        let req = self.create_request_with_headers(Method::DELETE, "/order", headers.into_iter());

        let response = req.json(&body).send().await?;
        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to cancel order",
            ));
        }

        Ok(response.json::<Value>().await?)
    }

    /// Cancel multiple orders
    pub async fn cancel_orders(&self, order_ids: &[String]) -> Result<Value> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyError::auth("API credentials not set"))?;

        let headers = create_l2_headers(signer, api_creds, "DELETE", "/orders", Some(order_ids))?;
        let req = self.create_request_with_headers(Method::DELETE, "/orders", headers.into_iter());

        let response = req.json(order_ids).send().await?;
        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to cancel orders",
            ));
        }

        Ok(response.json::<Value>().await?)
    }

    /// Cancel all orders
    pub async fn cancel_all(&self) -> Result<Value> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyError::auth("API credentials not set"))?;

        let headers = create_l2_headers::<Value>(signer, api_creds, "DELETE", "/cancel-all", None)?;
        let req =
            self.create_request_with_headers(Method::DELETE, "/cancel-all", headers.into_iter());

        let response = req.send().await?;
        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to cancel all orders",
            ));
        }

        Ok(response.json::<Value>().await?)
    }

    /// Get open orders with optional filtering
    ///
    /// This retrieves all open orders for the authenticated user. You can filter by:
    /// - Order ID (exact match)
    /// - Asset/Token ID (all orders for a specific token)
    /// - Market ID (all orders for a specific market)
    ///
    /// The response includes order status, fill information, and timestamps.
    pub async fn get_orders(
        &self,
        params: Option<&crate::types::OpenOrderParams>,
        next_cursor: Option<&str>,
    ) -> Result<Vec<crate::types::OpenOrder>> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyError::auth("API credentials not set"))?;

        let method = Method::GET;
        let endpoint = "/data/orders";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let query_params = match params {
            None => Vec::new(),
            Some(p) => p.to_query_params(),
        };

        let mut next_cursor = next_cursor.unwrap_or("MA==").to_string(); // INITIAL_CURSOR
        let mut output = Vec::new();

        while next_cursor != "LTE=" {
            // END_CURSOR
            let req = self
                .http_client
                .request(method.clone(), self.clob_url(endpoint))
                .query(&query_params)
                .query(&[("next_cursor", &next_cursor)]);

            let r = headers
                .clone()
                .into_iter()
                .fold(req, |r, (k, v)| r.header(HeaderName::from_static(k), v));

            let resp = r
                .send()
                .await
                .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?
                .json::<Value>()
                .await
                .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))?;

            let new_cursor = resp["next_cursor"]
                .as_str()
                .ok_or_else(|| PolyError::parse("Failed to parse next cursor".to_string(), None))?
                .to_owned();

            next_cursor = new_cursor;

            let results = resp["data"].clone();
            let orders =
                serde_json::from_value::<Vec<crate::types::OpenOrder>>(results).map_err(|e| {
                    PolyError::parse(
                        format!("Failed to parse data from order response: {}", e),
                        None,
                    )
                })?;
            output.extend(orders);
        }

        Ok(output)
    }

    /// Get trade history with optional filtering
    ///
    /// This retrieves historical trades for the authenticated user. You can filter by:
    /// - Trade ID (exact match)
    /// - Maker address (trades where you were the maker)
    /// - Market ID (trades in a specific market)
    /// - Asset/Token ID (trades for a specific token)
    /// - Time range (before/after timestamps)
    ///
    /// Trades are returned in reverse chronological order (newest first).
    pub async fn get_trades(
        &self,
        trade_params: Option<&crate::types::TradeParams>,
        next_cursor: Option<&str>,
    ) -> Result<Vec<Value>> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyError::auth("API credentials not set"))?;

        let method = Method::GET;
        let endpoint = "/data/trades";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let query_params = match trade_params {
            None => Vec::new(),
            Some(p) => p.to_query_params(),
        };

        let mut next_cursor = next_cursor.unwrap_or("MA==").to_string(); // INITIAL_CURSOR
        let mut output = Vec::new();

        while next_cursor != "LTE=" {
            // END_CURSOR
            let req = self
                .http_client
                .request(method.clone(), self.clob_url(endpoint))
                .query(&query_params)
                .query(&[("next_cursor", &next_cursor)]);

            let r = headers
                .clone()
                .into_iter()
                .fold(req, |r, (k, v)| r.header(HeaderName::from_static(k), v));

            let resp = r
                .send()
                .await
                .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?
                .json::<Value>()
                .await
                .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))?;

            let new_cursor = resp["next_cursor"]
                .as_str()
                .ok_or_else(|| PolyError::parse("Failed to parse next cursor".to_string(), None))?
                .to_owned();

            next_cursor = new_cursor;

            let results = resp["data"].clone();
            output.push(results);
        }

        Ok(output)
    }

    /// Get balance and allowance information for all assets
    ///
    /// This returns the current balance and allowance for each asset in your account.
    /// Balance is how much you own, allowance is how much the exchange can spend on your behalf.
    ///
    /// You need both balance and allowance to place orders - the exchange needs permission
    /// to move your tokens when orders are filled.
    pub async fn get_balance_allowance(
        &self,
        params: Option<crate::types::BalanceAllowanceParams>,
    ) -> Result<Value> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyError::auth("API credentials not set"))?;

        let mut params = params.unwrap_or_default();
        if params.signature_type.is_none() {
            params.set_signature_type(
                self.order_builder
                    .as_ref()
                    .expect("OrderBuilder not set")
                    .get_sig_type(),
            );
        }

        let query_params = params.to_query_params();

        let method = Method::GET;
        let endpoint = "/balance-allowance";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let response = self
            .http_client
            .request(method, self.clob_url(endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .query(&query_params)
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<Value>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Set up notifications for order fills and other events
    ///
    /// This configures push notifications so you get alerted when:
    /// - Your orders get filled
    /// - Your orders get cancelled
    /// - Market conditions change significantly
    ///
    /// The signature proves you own the account and want to receive notifications.
    pub async fn get_notifications(&self) -> Result<Value> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyError::auth("API credentials not set"))?;

        let method = Method::GET;
        let endpoint = "/notifications";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let response = self
            .http_client
            .request(method, self.clob_url(endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .query(&[(
                "signature_type",
                &self
                    .order_builder
                    .as_ref()
                    .expect("OrderBuilder not set")
                    .get_sig_type()
                    .to_string(),
            )])
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<Value>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get midpoints for multiple tokens in a single request
    ///
    /// This is much more efficient than calling get_midpoint() multiple times.
    /// Instead of N round trips, you make just 1 request and get all the midpoints back.
    ///
    /// Midpoints are returned as a HashMap where the key is the token_id and the value
    /// is the midpoint price (or None if there's no valid midpoint).
    pub async fn get_midpoints(
        &self,
        token_ids: &[String],
    ) -> Result<std::collections::HashMap<String, Decimal>> {
        let request_data: Vec<std::collections::HashMap<&str, String>> = token_ids
            .iter()
            .map(|id| {
                let mut map = std::collections::HashMap::new();
                map.insert("token_id", id.clone());
                map
            })
            .collect();

        let response = self
            .http_client
            .post(self.clob_url("midpoints"))
            .json(&request_data)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to get batch midpoints",
            ));
        }

        let midpoints: std::collections::HashMap<String, Decimal> = response.json().await?;
        Ok(midpoints)
    }

    /// Get bid/ask/mid prices for multiple tokens in a single request
    ///
    /// This gives you the full price picture for multiple tokens at once.
    /// Much more efficient than individual calls, especially when you're tracking
    /// a portfolio or comparing multiple markets.
    ///
    /// Returns bid (best buy price), ask (best sell price), and mid (average) for each token.
    pub async fn get_prices(
        &self,
        book_params: &[crate::types::BookParams],
    ) -> Result<std::collections::HashMap<String, std::collections::HashMap<Side, Decimal>>> {
        let request_data: Vec<std::collections::HashMap<&str, String>> = book_params
            .iter()
            .map(|params| {
                let mut map = std::collections::HashMap::new();
                map.insert("token_id", params.token_id.clone());
                map.insert("side", params.side.as_str().to_string());
                map
            })
            .collect();

        let response = self
            .http_client
            .post(self.clob_url("prices"))
            .json(&request_data)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to get batch prices",
            ));
        }

        let prices: std::collections::HashMap<String, std::collections::HashMap<Side, Decimal>> =
            response.json().await?;
        Ok(prices)
    }

    /// Get order book for multiple tokens (batch) - reference implementation compatible
    pub async fn get_order_books(&self, token_ids: &[String]) -> Result<Vec<OrderBookSummary>> {
        let request_data: Vec<std::collections::HashMap<&str, String>> = token_ids
            .iter()
            .map(|id| {
                let mut map = std::collections::HashMap::new();
                map.insert("token_id", id.clone());
                map
            })
            .collect();

        let response = self
            .http_client
            .post(self.clob_url("books"))
            .json(&request_data)
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<Vec<OrderBookSummary>>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get single order by ID
    pub async fn get_order(&self, order_id: &str) -> Result<crate::types::OpenOrder> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::config("Signer not configured"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyError::config("API credentials not configured"))?;

        let method = Method::GET;
        let endpoint = &format!("/data/order/{}", order_id);
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let response = self
            .http_client
            .request(method, self.clob_url(endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<crate::types::OpenOrder>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get last trade price for a token
    pub async fn get_last_trade_price(&self, token_id: &str) -> Result<Value> {
        let response = self
            .http_client
            .get(self.clob_url("last-trade-price"))
            .query(&[("token_id", token_id)])
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<Value>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get last trade prices for multiple tokens
    pub async fn get_last_trade_prices(&self, token_ids: &[String]) -> Result<Value> {
        let request_data: Vec<std::collections::HashMap<&str, String>> = token_ids
            .iter()
            .map(|id| {
                let mut map = std::collections::HashMap::new();
                map.insert("token_id", id.clone());
                map
            })
            .collect();

        let response = self
            .http_client
            .post(self.clob_url("last-trades-prices"))
            .json(&request_data)
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<Value>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Cancel market orders with optional filters
    pub async fn cancel_market_orders(
        &self,
        market: Option<&str>,
        asset_id: Option<&str>,
    ) -> Result<Value> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::config("Signer not configured"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyError::config("API credentials not configured"))?;

        let method = Method::DELETE;
        let endpoint = "/cancel-market-orders";
        let body = std::collections::HashMap::from([
            ("market", market.unwrap_or("")),
            ("asset_id", asset_id.unwrap_or("")),
        ]);

        let headers = create_l2_headers(signer, api_creds, method.as_str(), endpoint, Some(&body))?;

        let response = self
            .http_client
            .request(method, self.clob_url(endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<Value>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Drop (delete) notifications by IDs
    pub async fn drop_notifications(&self, ids: &[String]) -> Result<Value> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::config("Signer not configured"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyError::config("API credentials not configured"))?;

        let method = Method::DELETE;
        let endpoint = "/notifications";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let response = self
            .http_client
            .request(method, self.clob_url(endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .query(&[("ids", ids.join(","))])
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<Value>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Update balance allowance
    pub async fn update_balance_allowance(
        &self,
        params: Option<crate::types::BalanceAllowanceParams>,
    ) -> Result<Value> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::config("Signer not configured"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyError::config("API credentials not configured"))?;

        let mut params = params.unwrap_or_default();
        if params.signature_type.is_none() {
            params.set_signature_type(
                self.order_builder
                    .as_ref()
                    .expect("OrderBuilder not set")
                    .get_sig_type(),
            );
        }

        let query_params = params.to_query_params();

        let method = Method::GET;
        let endpoint = "/balance-allowance/update";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let response = self
            .http_client
            .request(method, self.clob_url(endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .query(&query_params)
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<Value>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Check if an order is scoring
    pub async fn is_order_scoring(&self, order_id: &str) -> Result<bool> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::config("Signer not configured"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyError::config("API credentials not configured"))?;

        let method = Method::GET;
        let endpoint = "/order-scoring";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let response = self
            .http_client
            .request(method, self.clob_url(endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .query(&[("order_id", order_id)])
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        let result: Value = response
            .json()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))?;

        Ok(result["scoring"].as_bool().unwrap_or(false))
    }

    /// Check if multiple orders are scoring
    pub async fn are_orders_scoring(
        &self,
        order_ids: &[&str],
    ) -> Result<std::collections::HashMap<String, bool>> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyError::config("Signer not configured"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyError::config("API credentials not configured"))?;

        let method = Method::POST;
        let endpoint = "/orders-scoring";
        let headers = create_l2_headers(
            signer,
            api_creds,
            method.as_str(),
            endpoint,
            Some(order_ids),
        )?;

        let response = self
            .http_client
            .request(method, self.clob_url(endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .json(order_ids)
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<std::collections::HashMap<String, bool>>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get sampling markets with pagination
    pub async fn get_sampling_markets(
        &self,
        next_cursor: Option<&str>,
    ) -> Result<crate::types::MarketsResponse> {
        let next_cursor = next_cursor.unwrap_or("MA=="); // INITIAL_CURSOR

        let response = self
            .http_client
            .get(self.gamma_url("sampling-markets"))
            .query(&[("next_cursor", next_cursor)])
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<crate::types::MarketsResponse>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get sampling simplified markets with pagination
    pub async fn get_sampling_simplified_markets(
        &self,
        next_cursor: Option<&str>,
    ) -> Result<crate::types::SimplifiedMarketsResponse> {
        let next_cursor = next_cursor.unwrap_or("MA=="); // INITIAL_CURSOR

        let response = self
            .http_client
            .get(self.gamma_url("sampling-simplified-markets"))
            .query(&[("next_cursor", next_cursor)])
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<crate::types::SimplifiedMarketsResponse>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get markets with pagination
    pub async fn get_markets(
        &self,
        next_cursor: Option<&str>,
        params: Option<&crate::types::GammaListParams>,
    ) -> Result<crate::types::MarketsResponse> {
        self.gamma_client.get_markets(next_cursor, params).await
    }

    /// Get simplified markets with pagination
    pub async fn get_simplified_markets(
        &self,
        next_cursor: Option<&str>,
    ) -> Result<crate::types::SimplifiedMarketsResponse> {
        let next_cursor = next_cursor.unwrap_or("MA=="); // INITIAL_CURSOR

        let response = self
            .http_client
            .get(self.gamma_url("simplified-markets"))
            .query(&[("next_cursor", next_cursor)])
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<crate::types::SimplifiedMarketsResponse>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get single market by condition ID
    pub async fn get_market(&self, market_id: &str) -> Result<crate::types::Market> {
        let response = self
            .http_client
            .get(self.gamma_url(&format!("markets/{}", market_id)))
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to fetch Gamma market",
            ));
        }

        let body = response
            .text()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to read response body: {}", e), None))?;

        let gamma_market =
            serde_json::from_str::<crate::types::GammaMarket>(&body).map_err(|err| {
                PolyError::parse(
                    format!("Failed to parse market {}: {}", market_id, err),
                    None,
                )
            })?;

        Ok(gamma_market.into())
    }

    /// Get market trades events
    pub async fn get_market_trades_events(&self, condition_id: &str) -> Result<Value> {
        let response = self
            .http_client
            .get(self.clob_url(&format!("live-activity/events/{}", condition_id)))
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<Value>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Fetch Gamma events with optional filtering
    pub async fn get_events(
        &self,
        params: Option<&crate::types::GammaListParams>,
    ) -> Result<Vec<crate::types::GammaEvent>> {
        self.gamma_client.get_events(params).await
    }

    /// Fetch a single Gamma event by slug
    pub async fn get_event_by_slug(&self, slug: &str) -> Result<crate::types::GammaEvent> {
        self.gamma_client.get_event_by_slug(slug).await
    }

    /// Fetch a single Gamma event by numeric ID
    pub async fn get_event_by_id(&self, event_id: &str) -> Result<crate::types::GammaEvent> {
        self.gamma_client.get_event_by_id(event_id).await
    }

    /// Fetch available Gamma tags
    pub async fn get_tags(&self) -> Result<Vec<crate::types::Tag>> {
        let gamma_tags = self.gamma_client.get_tags().await?;
        Ok(gamma_tags
            .into_iter()
            .map(|gt| crate::types::Tag {
                id: Some(gt.id),
                slug: gt.slug,
                name: Some(gt.name),
                description: None,
                metadata: serde_json::Value::default(),
            })
            .collect())
    }

    /// Fetch available Gamma sports metadata
    pub async fn get_sports(&self) -> Result<Vec<crate::types::Sport>> {
        let sport_names = self.gamma_client.get_sports().await?;
        Ok(sport_names
            .into_iter()
            .map(|name| crate::types::Sport {
                id: None,
                name: Some(name),
                description: None,
                tag_ids: vec![],
                tags: vec![],
                metadata: serde_json::Value::default(),
            })
            .collect())
    }
}

// Re-export types from the canonical location in types.rs
pub use crate::types::{
    DataApiPositionsParams, DataApiSortBy, DataApiSortDirection, DataPosition, DataPositionValue,
    ExtraOrderArgs, GammaEvent, GammaListParams, Market, MarketOrderArgs, MarketsResponse,
    MidpointResponse, NegRiskResponse, OrderBookSummary, OrderSummary, PriceResponse, Rewards,
    Sport, SpreadResponse, Tag, TickSizeResponse, Token,
};

// Compatibility types that need to stay in client.rs
#[derive(Debug, Default)]
pub struct CreateOrderOptions {
    pub tick_size: Option<Decimal>,
    pub neg_risk: Option<bool>,
}

// Re-export for compatibility
pub type PolyClient = ClobClient;

#[async_trait]
pub trait MarketClient: Send + Sync {
    async fn get_markets(
        &self,
        next_cursor: Option<&str>,
        params: Option<&crate::types::GammaListParams>,
    ) -> Result<crate::types::MarketsResponse>;
    async fn get_order_books(
        &self,
        token_ids: &[String],
    ) -> Result<Vec<crate::types::OrderBookSummary>>;
    async fn get_order_book(&self, token_id: &str) -> Result<crate::types::OrderBookSummary>;
    async fn cancel_market_orders(
        &self,
        market: Option<&str>,
        asset_id: Option<&str>,
    ) -> Result<serde_json::Value>;
    async fn create_order(
        &self,
        order_args: &OrderArgs,
        expiration: Option<u64>,
        extras: Option<crate::types::ExtraOrderArgs>,
        options: Option<&OrderOptions>,
    ) -> Result<SignedOrderRequest>;
    async fn post_order(
        &self,
        order: SignedOrderRequest,
        order_type: OrderType,
    ) -> Result<serde_json::Value>;
}

#[async_trait]
impl MarketClient for ClobClient {
    async fn get_markets(
        &self,
        next_cursor: Option<&str>,
        params: Option<&crate::types::GammaListParams>,
    ) -> Result<crate::types::MarketsResponse> {
        ClobClient::get_markets(self, next_cursor, params).await
    }

    async fn get_order_books(
        &self,
        token_ids: &[String],
    ) -> Result<Vec<crate::types::OrderBookSummary>> {
        ClobClient::get_order_books(self, token_ids).await
    }

    async fn get_order_book(&self, token_id: &str) -> Result<crate::types::OrderBookSummary> {
        ClobClient::get_order_book(self, token_id).await
    }

    async fn cancel_market_orders(
        &self,
        market: Option<&str>,
        asset_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        ClobClient::cancel_market_orders(self, market, asset_id).await
    }

    async fn create_order(
        &self,
        order_args: &OrderArgs,
        expiration: Option<u64>,
        extras: Option<crate::types::ExtraOrderArgs>,
        options: Option<&OrderOptions>,
    ) -> Result<SignedOrderRequest> {
        ClobClient::create_order(self, order_args, expiration, extras, options).await
    }

    async fn post_order(
        &self,
        order: SignedOrderRequest,
        order_type: OrderType,
    ) -> Result<serde_json::Value> {
        ClobClient::post_order(self, order, order_type).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use mockito::{Matcher, Server};
    use rust_decimal::Decimal;
    use std::str::FromStr;
    use tokio;

    fn create_test_client(base_url: &str) -> ClobClient {
        ClobClient::new(base_url).with_gamma_base(base_url)
    }

    fn create_test_client_with_auth(base_url: &str) -> ClobClient {
        ClobClient::with_l1_headers(
            base_url,
            "0x1234567890123456789012345678901234567890123456789012345678901234",
            137,
        )
        .with_gamma_base(base_url)
    }

    #[tokio::test]
    async fn test_client_creation() {
        let client = create_test_client("https://test.example.com");
        assert_eq!(client.base_url, "https://test.example.com");
        assert!(client.signer.is_none());
        assert!(client.api_creds.is_none());
    }

    #[tokio::test]
    async fn test_client_with_l1_headers() {
        let client = create_test_client_with_auth("https://test.example.com");
        assert_eq!(client.base_url, "https://test.example.com");
        assert!(client.signer.is_some());
        assert_eq!(client.chain_id, 137);
    }

    #[tokio::test]
    async fn test_client_with_l2_headers() {
        let api_creds = ApiCredentials {
            api_key: "test_key".to_string(),
            secret: "test_secret".to_string(),
            passphrase: "test_passphrase".to_string(),
        };

        let client = ClobClient::with_l2_headers(
            "https://test.example.com",
            "0x1234567890123456789012345678901234567890123456789012345678901234",
            137,
            api_creds.clone(),
        );

        assert_eq!(client.base_url, "https://test.example.com");
        assert!(client.signer.is_some());
        assert!(client.api_creds.is_some());
        assert_eq!(client.chain_id, 137);
    }

    #[tokio::test]
    async fn test_set_api_creds() {
        let mut client = create_test_client("https://test.example.com");
        assert!(client.api_creds.is_none());

        let api_creds = ApiCredentials {
            api_key: "test_key".to_string(),
            secret: "test_secret".to_string(),
            passphrase: "test_passphrase".to_string(),
        };

        client.set_api_creds(api_creds.clone());
        assert!(client.api_creds.is_some());
        assert_eq!(client.api_creds.unwrap().api_key, "test_key");
    }

    #[tokio::test]
    async fn test_get_sampling_markets_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "limit": "10",
            "count": "2", 
            "next_cursor": null,
            "data": [
                {
                    "condition_id": "0x123",
                    "tokens": [
                        {"token_id": "0x456", "outcome": "Yes"},
                        {"token_id": "0x789", "outcome": "No"}
                    ],
                    "rewards": {
                        "rates": null,
                        "min_size": "1.0",
                        "max_spread": "0.1",
                        "event_start_date": null,
                        "event_end_date": null,
                        "in_game_multiplier": null,
                        "reward_epoch": null
                    },
                    "min_incentive_size": null,
                    "max_incentive_spread": null,
                    "active": true,
                    "closed": false,
                    "question_id": "0x123",
                    "minimum_order_size": "1.0",
                    "minimum_tick_size": "0.01",
                    "description": "Test market",
                    "category": "test",
                    "end_date_iso": null,
                    "game_start_time": null,
                    "question": "Will this test pass?",
                    "market_slug": "test-market",
                    "seconds_delay": "0",
                    "icon": "",
                    "fpmm": ""
                }
            ]
        }"#;

        let mock = server
            .mock("GET", "/sampling-markets")
            .match_query(Matcher::UrlEncoded("next_cursor".into(), "MA==".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_sampling_markets(None).await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let markets = result.unwrap();
        assert_eq!(markets.data.len(), 1);
        assert_eq!(markets.data[0].question, "Will this test pass?");
    }

    #[tokio::test]
    async fn test_get_sampling_markets_with_cursor() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "limit": "5",
            "count": "0",
            "next_cursor": null,
            "data": []
        }"#;

        let mock = server
            .mock("GET", "/sampling-markets")
            .match_query(Matcher::AllOf(vec![Matcher::UrlEncoded(
                "next_cursor".into(),
                "test_cursor".into(),
            )]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_sampling_markets(Some("test_cursor")).await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let markets = result.unwrap();
        assert_eq!(markets.data.len(), 0);
    }

    #[tokio::test]
    async fn test_get_order_book_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "market": "0x123",
            "asset_id": "0x123",
            "hash": "0xabc123",
            "timestamp": "1234567890",
            "bids": [
                {"price": "0.75", "size": "100.0"}
            ],
            "asks": [
                {"price": "0.76", "size": "50.0"}
            ]
        }"#;

        let mock = server
            .mock("GET", "/book")
            .match_query(Matcher::UrlEncoded("token_id".into(), "0x123".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_order_book("0x123").await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let book = result.unwrap();
        assert_eq!(book.market, "0x123");
        assert_eq!(book.bids.len(), 1);
        assert_eq!(book.asks.len(), 1);
    }

    #[tokio::test]
    async fn test_get_midpoint_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "mid": "0.755"
        }"#;

        let mock = server
            .mock("GET", "/midpoint")
            .match_query(Matcher::UrlEncoded("token_id".into(), "0x123".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_midpoint("0x123").await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.mid, Decimal::from_str("0.755").unwrap());
    }

    #[tokio::test]
    async fn test_get_spread_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "spread": "0.01"
        }"#;

        let mock = server
            .mock("GET", "/spread")
            .match_query(Matcher::UrlEncoded("token_id".into(), "0x123".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_spread("0x123").await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.spread, Decimal::from_str("0.01").unwrap());
    }

    #[tokio::test]
    async fn test_get_price_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "price": "0.76"
        }"#;

        let mock = server
            .mock("GET", "/price")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("token_id".into(), "0x123".into()),
                Matcher::UrlEncoded("side".into(), "BUY".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_price("0x123", Side::BUY).await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.price, Decimal::from_str("0.76").unwrap());
    }

    #[tokio::test]
    async fn test_get_tick_size_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "minimum_tick_size": "0.01"
        }"#;

        let mock = server
            .mock("GET", "/tick-size")
            .match_query(Matcher::UrlEncoded("token_id".into(), "0x123".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_tick_size("0x123").await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let tick_size = result.unwrap();
        assert_eq!(tick_size, Decimal::from_str("0.01").unwrap());
    }

    #[tokio::test]
    async fn test_get_neg_risk_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "neg_risk": false
        }"#;

        let mock = server
            .mock("GET", "/neg-risk")
            .match_query(Matcher::UrlEncoded("token_id".into(), "0x123".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_neg_risk("0x123").await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let neg_risk = result.unwrap();
        assert!(!neg_risk);
    }

    #[tokio::test]
    async fn test_api_error_handling() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/book")
            .match_query(Matcher::UrlEncoded(
                "token_id".into(),
                "invalid_token".into(),
            ))
            .with_status(404)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error": "Market not found"}"#)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_order_book("invalid_token").await;

        mock.assert_async().await;
        assert!(result.is_err());

        let error = result.unwrap_err();
        // The error should be either Network or Api error
        assert!(
            matches!(error, PolyError::Network { .. }) || matches!(error, PolyError::Api { .. })
        );
    }

    #[tokio::test]
    async fn test_network_error_handling() {
        // Test with invalid URL to simulate network error
        let client = create_test_client("http://invalid-host-that-does-not-exist.com");
        let result = client.get_order_book("0x123").await;

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(matches!(error, PolyError::Network { .. }));
    }

    #[test]
    fn test_client_url_validation() {
        let client = create_test_client("https://test.example.com");
        assert_eq!(client.base_url, "https://test.example.com");

        let client2 = create_test_client("http://localhost:8080");
        assert_eq!(client2.base_url, "http://localhost:8080");
    }

    #[tokio::test]
    async fn test_get_midpoints_batch() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "0x123": "0.755",
            "0x456": "0.623"
        }"#;

        let mock = server
            .mock("POST", "/midpoints")
            .with_header("content-type", "application/json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let token_ids = vec!["0x123".to_string(), "0x456".to_string()];
        let result = client.get_midpoints(&token_ids).await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let midpoints = result.unwrap();
        assert_eq!(midpoints.len(), 2);
        assert_eq!(
            midpoints.get("0x123").unwrap(),
            &Decimal::from_str("0.755").unwrap()
        );
        assert_eq!(
            midpoints.get("0x456").unwrap(),
            &Decimal::from_str("0.623").unwrap()
        );
    }

    #[tokio::test]
    async fn test_get_gamma_events_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"[
            {"event_id": "evt-1", "slug": "event-one"}
        ]"#;

        let mock = server
            .mock("GET", "/events")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let events = client.get_events(None).await;

        mock.assert_async().await;
        assert!(events.is_ok());
        let events = events.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, "evt-1");
        assert_eq!(events[0].slug, "event-one");
    }

    #[tokio::test]
    async fn test_get_gamma_event_by_slug_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{ "event_id": "evt-2", "slug": "event-two" }"#;

        let mock = server
            .mock("GET", "/events/slug/event-two")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let event = client.get_event_by_slug("event-two").await;

        mock.assert_async().await;
        assert!(event.is_ok());
        let event = event.unwrap();
        assert_eq!(event.id, "evt-2");
        assert_eq!(event.slug, "event-two");
    }

    #[tokio::test]
    async fn test_get_gamma_tags_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"[
            {"id": "tag-1", "slug": "politics", "name": "Politics"}
        ]"#;

        let mock = server
            .mock("GET", "/tags")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let tags = client.get_tags().await;

        mock.assert_async().await;
        assert!(tags.is_ok());
        let tags = tags.unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].id.as_deref(), Some("tag-1"));
    }

    #[tokio::test]
    async fn test_get_gamma_sports_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"[
            {"id": "sport-1", "name": "Soccer"}
        ]"#;

        let mock = server
            .mock("GET", "/sports")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let sports = client.get_sports().await;

        mock.assert_async().await;
        assert!(sports.is_ok());
        let sports = sports.unwrap();
        assert_eq!(sports.len(), 1);
        assert_eq!(sports[0].name.as_deref(), Some("Soccer"));
    }

    #[test]
    fn test_client_configuration() {
        let client = create_test_client("https://test.example.com");

        // Test initial state
        assert!(client.signer.is_none());
        assert!(client.api_creds.is_none());

        // Test with auth
        let auth_client = create_test_client_with_auth("https://test.example.com");
        assert!(auth_client.signer.is_some());
        assert_eq!(auth_client.chain_id, 137);
    }
}
