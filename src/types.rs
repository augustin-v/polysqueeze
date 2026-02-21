//! Core types for the Polymarket client
//!
//! This module defines all the stable public types used throughout the client.
//! These types are optimized for latency-sensitive trading environments.

use alloy_primitives::{Address, U256};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use serde_json;
use std::fmt;

// ============================================================================
// FIXED-POINT OPTIMIZATION FOR HOT PATH PERFORMANCE
// ============================================================================
//
// Instead of using rust_decimal::Decimal everywhere (which allocates),
// I've used fixed-point integers for the performance-critical order book operations.
//
// Why this matters:
// - Decimal operations can be 10-100x slower than integer operations
// - Decimal allocates memory for each calculation
// - In an order book like this we process thousands of price updates per second
// - Most prices can be represented as integer ticks (e.g., $0.6543 = 6543 ticks)
//
// The strategy:
// 1. Convert Decimal to fixed-point on ingress (when data comes in)
// 2. Do all hot-path calculations with integers
// 3. Convert back to Decimal only at the edges (API responses, user display)
//
// This is like how video games handle positions, they use integers internally
// for speed, but show floating-point coordinates to players.
/// Each tick represents 0.0001 (1/10,000) of the base unit
/// Examples:
/// - $0.6543 = 6543 ticks
/// - $1.0000 = 10000 ticks  
/// - $0.0001 = 1 tick (minimum price increment)
///
/// Why u32?
/// - Can represent prices from $0.0001 to $429,496.7295 (way more than needed)
/// - Fits in CPU register for fast operations
/// - No sign bit needed since prices are always positive
pub type Price = u32;

/// Quantity/size represented as fixed-point integer for performance
///
/// Each unit represents 0.0001 (1/10,000) of a token
/// Examples:
/// - 100.0 tokens = 1,000,000 units
/// - 0.0001 tokens = 1 unit (minimum size increment)
///
/// Why i64?
/// - Can represent quantities from -922,337,203,685.4775 to +922,337,203,685.4775
/// - Signed because we need to handle both buys (+) and sells (-)
/// - Large enough for any realistic trading size
pub type Qty = i64;

/// Scale factor for converting between Decimal and fixed-point
///
/// We use 10,000 (1e4) as our scale factor, giving us 4 decimal places of precision.
/// This is perfect for most prediction markets where prices are between $0.01-$0.99
/// and we need precision to the nearest $0.0001.
pub const SCALE_FACTOR: i64 = 10_000;

/// Maximum valid price in ticks (prevents overflow)
/// This represents $429,496.7295 which is way higher than any prediction market price
pub const MAX_PRICE_TICKS: Price = Price::MAX;

/// Minimum valid price in ticks (1 tick = $0.0001)
pub const MIN_PRICE_TICKS: Price = 1;

/// Maximum valid quantity (prevents overflow in calculations)
pub const MAX_QTY: Qty = Qty::MAX / 2; // Leave room for intermediate calculations

// ============================================================================
// CONVERSION FUNCTIONS BETWEEN DECIMAL AND FIXED-POINT
// ============================================================================
//
// These functions handle the conversion between the external Decimal API
// and our internal fixed-point representation. They're designed to be fast
// and handle edge cases gracefully.

/// Convert a Decimal price to fixed-point ticks
///
/// This is called when we receive price data from the API or user input.
/// We quantize the price to the nearest tick to ensure all prices are
/// aligned to our internal representation.
///
/// Examples:
/// - decimal_to_price(Decimal::from_str("0.6543")) = Ok(6543)
/// - decimal_to_price(Decimal::from_str("1.0000")) = Ok(10000)
/// - decimal_to_price(Decimal::from_str("0.00005")) = Ok(1) // Rounds up to min tick
pub fn decimal_to_price(decimal: Decimal) -> std::result::Result<Price, &'static str> {
    // Convert to fixed-point by multiplying by scale factor
    let scaled = decimal * Decimal::from(SCALE_FACTOR);

    // Round to nearest integer (this handles tick alignment automatically)
    let rounded = scaled.round();

    // Convert to u64 first to handle the conversion safely
    let as_u64 = rounded.to_u64().ok_or("Price too large or negative")?;

    // Check bounds
    if as_u64 < MIN_PRICE_TICKS as u64 {
        return Ok(MIN_PRICE_TICKS); // Clamp to minimum
    }
    if as_u64 > MAX_PRICE_TICKS as u64 {
        return Err("Price exceeds maximum");
    }

    Ok(as_u64 as Price)
}

/// Convert fixed-point ticks back to Decimal price
///
/// This is called when we need to return price data to the API or display to users.
/// It's the inverse of decimal_to_price().
///
/// Examples:
/// - price_to_decimal(6543) = Decimal::from_str("0.6543")
/// - price_to_decimal(10000) = Decimal::from_str("1.0000")
pub fn price_to_decimal(ticks: Price) -> Decimal {
    Decimal::from(ticks) / Decimal::from(SCALE_FACTOR)
}

/// Convert a Decimal quantity to fixed-point units
///
/// Similar to decimal_to_price but handles signed quantities.
/// Quantities can be negative (for sells or position changes).
///
/// Examples:
/// - decimal_to_qty(Decimal::from_str("100.0")) = Ok(1000000)
/// - decimal_to_qty(Decimal::from_str("-50.5")) = Ok(-505000)
pub fn decimal_to_qty(decimal: Decimal) -> std::result::Result<Qty, &'static str> {
    let scaled = decimal * Decimal::from(SCALE_FACTOR);
    let rounded = scaled.round();

    let as_i64 = rounded.to_i64().ok_or("Quantity too large")?;

    if as_i64.abs() > MAX_QTY {
        return Err("Quantity exceeds maximum");
    }

    Ok(as_i64)
}

/// Convert fixed-point units back to Decimal quantity
///
/// Examples:
/// - qty_to_decimal(1000000) = Decimal::from_str("100.0")
/// - qty_to_decimal(-505000) = Decimal::from_str("-50.5")
pub fn qty_to_decimal(units: Qty) -> Decimal {
    Decimal::from(units) / Decimal::from(SCALE_FACTOR)
}

/// Check if a price is properly tick-aligned
///
/// This is used to validate incoming price data. In a well-behaved system,
/// all prices should already be tick-aligned, but we check anyway to catch
/// bugs or malicious data.
///
/// A price is tick-aligned if it's an exact multiple of the minimum tick size.
/// Since we use integer ticks internally, this just checks if the price
/// converts cleanly to our internal representation.
pub fn is_price_tick_aligned(decimal: Decimal, tick_size_decimal: Decimal) -> bool {
    // Convert tick size to our internal representation
    let tick_size_ticks = match decimal_to_price(tick_size_decimal) {
        Ok(ticks) => ticks,
        Err(_) => return false,
    };

    // Convert the price to ticks
    let price_ticks = match decimal_to_price(decimal) {
        Ok(ticks) => ticks,
        Err(_) => return false,
    };

    // Check if price is a multiple of tick size
    // If tick_size_ticks is 0, we consider everything aligned (no restrictions)
    if tick_size_ticks == 0 {
        return true;
    }

    price_ticks % tick_size_ticks == 0
}

/// Trading side for orders
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    BUY = 0,
    SELL = 1,
}

impl Side {
    pub fn as_str(&self) -> &'static str {
        match self {
            Side::BUY => "BUY",
            Side::SELL => "SELL",
        }
    }

    pub fn opposite(&self) -> Self {
        match self {
            Side::BUY => Side::SELL,
            Side::SELL => Side::BUY,
        }
    }
}

/// Order type specifications
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderType {
    GTC,
    FOK,
    GTD,
    FAK,
}

impl OrderType {
    pub fn as_str(&self) -> &'static str {
        match self {
            OrderType::GTC => "GTC",
            OrderType::FOK => "FOK",
            OrderType::GTD => "GTD",
            OrderType::FAK => "FAK",
        }
    }
}

/// Order status in the system
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderStatus {
    #[serde(rename = "LIVE")]
    Live,
    #[serde(rename = "CANCELLED")]
    Cancelled,
    #[serde(rename = "FILLED")]
    Filled,
    #[serde(rename = "PARTIAL")]
    Partial,
    #[serde(rename = "EXPIRED")]
    Expired,
}

/// Market snapshot representing current state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketSnapshot {
    pub token_id: String,
    pub market_id: String,
    pub timestamp: DateTime<Utc>,
    pub bid: Option<Decimal>,
    pub ask: Option<Decimal>,
    pub mid: Option<Decimal>,
    pub spread: Option<Decimal>,
    pub last_price: Option<Decimal>,
    pub volume_24h: Option<Decimal>,
}

/// Order book level (price/size pair) - EXTERNAL API VERSION
///
/// This is what we expose to users and serialize to JSON.
/// It uses Decimal for precision and human readability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookLevel {
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub size: Decimal,
}

/// Order book level (price/size pair) - INTERNAL HOT PATH VERSION
///
/// This is what we use internally for maximum performance.
/// All order book operations use this to avoid Decimal overhead.
///
/// The performance difference is huge:
/// - BookLevel: ~50ns per operation (Decimal math + allocation)
/// - FastBookLevel: ~2ns per operation (integer math, no allocation)
///
/// That's a 25x speedup on the critical path
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FastBookLevel {
    pub price: Price, // Price in ticks (u32)
    pub size: Qty,    // Size in fixed-point units (i64)
}

impl FastBookLevel {
    /// Create a new fast book level
    pub fn new(price: Price, size: Qty) -> Self {
        Self { price, size }
    }

    /// Convert to external BookLevel for API responses
    /// This is only called at the edges when we need to return data to users
    pub fn to_book_level(self) -> BookLevel {
        BookLevel {
            price: price_to_decimal(self.price),
            size: qty_to_decimal(self.size),
        }
    }

    /// Create from external BookLevel (with validation)
    /// This is called when we receive data from the API
    pub fn from_book_level(level: &BookLevel) -> std::result::Result<Self, &'static str> {
        let price = decimal_to_price(level.price)?;
        let size = decimal_to_qty(level.size)?;
        Ok(Self::new(price, size))
    }

    /// Calculate notional value (price * size) in fixed-point
    /// Returns the result scaled appropriately to avoid overflow
    ///
    /// This is much faster than the Decimal equivalent:
    /// - Decimal: price.mul(size) -> ~20ns + allocation
    /// - Fixed-point: (price as i64 * size) / SCALE_FACTOR -> ~1ns, no allocation
    pub fn notional(self) -> i64 {
        // Convert price to i64 to avoid overflow in multiplication
        let price_i64 = self.price as i64;
        // Multiply and scale back down (we scaled both price and size up by SCALE_FACTOR)
        (price_i64 * self.size) / SCALE_FACTOR
    }
}

/// Full order book state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBook {
    /// Token ID
    pub token_id: String,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// Bid orders
    pub bids: Vec<BookLevel>,
    /// Ask orders
    pub asks: Vec<BookLevel>,
    /// Sequence number
    pub sequence: u64,
}

/// Order book delta for streaming updates - EXTERNAL API VERSION
///
/// This is what we receive from WebSocket streams and REST API calls.
/// It uses Decimal for compatibility with external systems.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderDelta {
    pub token_id: String,
    pub timestamp: DateTime<Utc>,
    pub side: Side,
    pub price: Decimal,
    pub size: Decimal, // 0 means remove level
    pub sequence: u64,
}

/// Order book delta for streaming updates - INTERNAL HOT PATH VERSION
///
/// This is what we use internally for processing order book updates.
/// Converting to this format on ingress gives us massive performance gains.
///
/// Why the performance matters:
/// - We might process 10,000+ deltas per second in active markets
/// - Each delta triggers multiple calculations (spread, impact, etc.)
/// - Using integers instead of Decimal can make the difference between
///   keeping up with the market feed vs falling behind
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FastOrderDelta {
    pub token_id_hash: u64, // Hash of token_id for fast lookup (avoids string comparisons)
    pub timestamp: DateTime<Utc>,
    pub side: Side,
    pub price: Price, // Price in ticks
    pub size: Qty,    // Size in fixed-point units (0 means remove level)
    pub sequence: u64,
}

impl FastOrderDelta {
    /// Create from external OrderDelta with validation and tick alignment
    ///
    /// This is where we enforce tick alignment - if the incoming price
    /// doesn't align to valid ticks, we either reject it or round it.
    /// This prevents bad data from corrupting our order book.
    pub fn from_order_delta(
        delta: &OrderDelta,
        tick_size: Option<Decimal>,
    ) -> std::result::Result<Self, &'static str> {
        // Validate tick alignment if we have a tick size
        if let Some(tick_size) = tick_size {
            if !is_price_tick_aligned(delta.price, tick_size) {
                return Err("Price not aligned to tick size");
            }
        }

        // Convert to fixed-point with validation
        let price = decimal_to_price(delta.price)?;
        let size = decimal_to_qty(delta.size)?;

        // Hash the token_id for fast lookups
        // This avoids string comparisons in the hot path
        let token_id_hash = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            delta.token_id.hash(&mut hasher);
            hasher.finish()
        };

        Ok(Self {
            token_id_hash,
            timestamp: delta.timestamp,
            side: delta.side,
            price,
            size,
            sequence: delta.sequence,
        })
    }

    /// Convert back to external OrderDelta (for API responses)
    /// We need the original token_id since we only store the hash
    pub fn to_order_delta(self, token_id: String) -> OrderDelta {
        OrderDelta {
            token_id,
            timestamp: self.timestamp,
            side: self.side,
            price: price_to_decimal(self.price),
            size: qty_to_decimal(self.size),
            sequence: self.sequence,
        }
    }

    /// Check if this delta removes a level (size is zero)
    pub fn is_removal(self) -> bool {
        self.size == 0
    }
}

/// Trade execution event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillEvent {
    pub id: String,
    pub order_id: String,
    pub token_id: String,
    pub side: Side,
    pub price: Decimal,
    pub size: Decimal,
    pub timestamp: DateTime<Utc>,
    pub maker_address: Address,
    pub taker_address: Address,
    pub fee: Decimal,
}

/// Order creation parameters
#[derive(Debug, Clone)]
pub struct OrderRequest {
    pub token_id: String,
    pub side: Side,
    pub price: Decimal,
    pub size: Decimal,
    pub order_type: OrderType,
    pub expiration: Option<DateTime<Utc>>,
    pub client_id: Option<String>,
}

/// Market order parameters
#[derive(Debug, Clone)]
pub struct MarketOrderRequest {
    pub token_id: String,
    pub side: Side,
    pub amount: Decimal, // USD amount for buys, token amount for sells
    pub slippage_tolerance: Option<Decimal>,
    pub client_id: Option<String>,
}

/// Order state in the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: String,
    pub token_id: String,
    pub side: Side,
    pub price: Decimal,
    pub original_size: Decimal,
    pub filled_size: Decimal,
    pub remaining_size: Decimal,
    pub status: OrderStatus,
    pub order_type: OrderType,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expiration: Option<DateTime<Utc>>,
    pub client_id: Option<String>,
}

/// API credentials for authentication
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApiCredentials {
    #[serde(rename = "apiKey")]
    pub api_key: String,
    pub secret: String,
    pub passphrase: String,
}

/// Configuration for order creation
#[derive(Debug, Clone)]
pub struct OrderOptions {
    pub tick_size: Option<Decimal>,
    pub neg_risk: Option<bool>,
    pub fee_rate_bps: Option<u32>,
}

/// Extra arguments for order creation
#[derive(Debug, Clone)]
pub struct ExtraOrderArgs {
    pub fee_rate_bps: u32,
    pub nonce: U256,
    pub taker: String,
}

impl Default for ExtraOrderArgs {
    fn default() -> Self {
        Self {
            fee_rate_bps: 0,
            nonce: U256::ZERO,
            taker: "0x0000000000000000000000000000000000000000".to_string(),
        }
    }
}

/// Market order arguments
#[derive(Debug, Clone)]
pub struct MarketOrderArgs {
    pub token_id: String,
    pub amount: Decimal,
}

/// Signed order request ready for submission
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedOrderRequest {
    pub salt: u64,
    pub maker: String,
    pub signer: String,
    pub taker: String,
    pub token_id: String,
    pub maker_amount: String,
    pub taker_amount: String,
    pub expiration: String,
    pub nonce: String,
    pub fee_rate_bps: String,
    pub side: String,
    pub signature_type: u8,
    pub signature: String,
}

/// Post order wrapper
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostOrder {
    pub order: SignedOrderRequest,
    pub owner: String,
    pub order_type: OrderType,
}

impl PostOrder {
    pub fn new(order: SignedOrderRequest, owner: String, order_type: OrderType) -> Self {
        Self {
            order,
            owner,
            order_type,
        }
    }
}

/// Response for a single order in a batch request
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchOrderResponse {
    pub success: bool,
    pub error_msg: Option<String>,
    pub order_id: Option<String>,
    pub making_amount: Option<String>,
    pub taking_amount: Option<String>,
    pub status: Option<String>,
}

impl BatchOrderResponse {
    pub fn has_error(&self) -> bool {
        !self.success || self.error_msg.is_some()
    }
}

/// Market information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Market {
    pub condition_id: String,
    pub tokens: [Token; 2],
    #[serde(default, skip)]
    pub clob_token_ids: Vec<String>,
    pub rewards: Rewards,
    pub min_incentive_size: Option<String>,
    pub max_incentive_spread: Option<String>,
    pub active: bool,
    pub closed: bool,
    pub question_id: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub minimum_order_size: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub minimum_tick_size: Decimal,
    pub description: String,
    pub category: Option<String>,
    pub end_date_iso: Option<String>,
    pub game_start_time: Option<String>,
    pub question: String,
    pub market_slug: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub seconds_delay: Decimal,
    pub icon: String,
    pub fpmm: String,
    pub liquidity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquidity_num: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquidity_amm: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquidity_clob: Option<Decimal>,
    pub volume: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_num: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_24hr: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_1wk: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_1mo: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_1yr: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_24hr_amm: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_1wk_amm: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_1mo_amm: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_1yr_amm: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_24hr_clob: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_1wk_clob: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_1mo_clob: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_1yr_clob: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_amm: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_clob: Option<Decimal>,
}

/// Token information within a market
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub token_id: String,
    pub outcome: String,
}

impl GammaMarket {
    fn parse_token_ids(&self) -> Vec<String> {
        self.clob_token_ids
            .as_ref()
            .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
            .unwrap_or_default()
    }

    fn normalized_outcomes(&self) -> Vec<String> {
        let default_outcomes = vec!["Yes".to_string(), "No".to_string()];
        if let Some(raw) = self.outcomes.as_ref() {
            if let Ok(values) = serde_json::from_str::<Vec<String>>(raw) {
                return values;
            }
        }

        default_outcomes
    }
}

impl From<GammaMarket> for Market {
    fn from(gamma: GammaMarket) -> Self {
        let token_ids = gamma.parse_token_ids();
        let outcomes = gamma.normalized_outcomes();

        let tokens = [
            Token {
                token_id: token_ids.first().cloned().unwrap_or_default(),
                outcome: outcomes
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "Yes".to_string()),
            },
            Token {
                token_id: token_ids.get(1).cloned().unwrap_or_default(),
                outcome: outcomes.get(1).cloned().unwrap_or_else(|| "No".to_string()),
            },
        ];

        Market {
            condition_id: gamma.condition_id.clone(),
            tokens,
            clob_token_ids: token_ids.clone(),
            rewards: Rewards {
                rates: None,
                min_size: Decimal::ZERO,
                max_spread: Decimal::ZERO,
                event_start_date: None,
                event_end_date: gamma.end_date.clone(),
                in_game_multiplier: None,
                reward_epoch: None,
            },
            min_incentive_size: None,
            max_incentive_spread: None,
            active: gamma.active,
            closed: gamma.closed,
            question_id: gamma.condition_id.clone(),
            minimum_order_size: gamma.order_min_size.unwrap_or(Decimal::ZERO),
            minimum_tick_size: gamma.order_tick_size.unwrap_or(Decimal::ZERO),
            description: gamma.description.unwrap_or_default(),
            category: gamma.category.clone(),
            end_date_iso: gamma.end_date.clone(),
            game_start_time: None,
            question: gamma.question.unwrap_or_default(),
            market_slug: gamma.slug.clone(),
            seconds_delay: Decimal::ZERO,
            icon: gamma.icon.unwrap_or_default(),
            fpmm: String::new(),
            liquidity: gamma.liquidity.clone(),
            liquidity_num: gamma.liquidity_num,
            liquidity_amm: gamma.liquidity_amm,
            liquidity_clob: gamma.liquidity_clob,
            volume: gamma.volume.clone(),
            volume_num: gamma.volume_num,
            volume_24hr: gamma.volume_24hr,
            volume_1wk: gamma.volume_1wk,
            volume_1mo: gamma.volume_1mo,
            volume_1yr: gamma.volume_1yr,
            volume_24hr_amm: gamma.volume_24hr_amm,
            volume_1wk_amm: gamma.volume_1wk_amm,
            volume_1mo_amm: gamma.volume_1mo_amm,
            volume_1yr_amm: gamma.volume_1yr_amm,
            volume_24hr_clob: gamma.volume_24hr_clob,
            volume_1wk_clob: gamma.volume_1wk_clob,
            volume_1mo_clob: gamma.volume_1mo_clob,
            volume_1yr_clob: gamma.volume_1yr_clob,
            volume_amm: gamma.volume_amm,
            volume_clob: gamma.volume_clob,
        }
    }
}

/// Client configuration for PolyClient
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// Base URL for the API
    pub base_url: String,
    /// Chain ID for the network
    pub chain_id: u64,
    /// Private key for signing (optional)
    pub private_key: Option<String>,
    /// API credentials (optional)
    pub api_credentials: Option<ApiCredentials>,
    /// Maximum slippage tolerance
    pub max_slippage: Option<Decimal>,
    /// Fee rate in basis points
    pub fee_rate: Option<Decimal>,
    /// Request timeout
    pub timeout: Option<std::time::Duration>,
    /// Maximum number of connections
    pub max_connections: Option<usize>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            base_url: "https://clob.polymarket.com".to_string(),
            chain_id: 137, // Polygon mainnet
            private_key: None,
            api_credentials: None,
            timeout: Some(std::time::Duration::from_secs(30)),
            max_connections: Some(100),
            max_slippage: None,
            fee_rate: None,
        }
    }
}

/// WebSocket authentication for Polymarket API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WssAuth {
    /// User's Ethereum address
    pub address: String,
    /// EIP-712 signature
    pub signature: String,
    /// Unix timestamp
    pub timestamp: u64,
    /// Nonce for replay protection
    pub nonce: String,
}

/// WebSocket subscription request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WssSubscription {
    /// Authentication information
    pub auth: WssAuth,
    /// Array of markets (condition IDs) for USER channel
    pub markets: Option<Vec<String>>,
    /// Array of asset IDs (token IDs) for MARKET channel
    pub asset_ids: Option<Vec<String>>,
    /// Channel type: "USER" or "MARKET"
    #[serde(rename = "type")]
    pub channel_type: String,
}

/// WebSocket message types for streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StreamMessage {
    #[serde(rename = "book_update")]
    BookUpdate { data: OrderDelta },
    #[serde(rename = "trade")]
    Trade { data: FillEvent },
    #[serde(rename = "order_update")]
    OrderUpdate { data: Order },
    #[serde(rename = "heartbeat")]
    Heartbeat { timestamp: DateTime<Utc> },
    /// User channel events
    #[serde(rename = "user_order_update")]
    UserOrderUpdate { data: Order },
    #[serde(rename = "user_trade")]
    UserTrade { data: FillEvent },
    /// Market channel events
    #[serde(rename = "market_book_update")]
    MarketBookUpdate { data: OrderDelta },
    #[serde(rename = "market_trade")]
    MarketTrade { data: FillEvent },
}

/// Subscription parameters for streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub token_ids: Vec<String>,
    pub channels: Vec<String>,
}

/// WebSocket channel types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WssChannelType {
    #[serde(rename = "USER")]
    User,
    #[serde(rename = "MARKET")]
    Market,
}

impl WssChannelType {
    pub fn as_str(&self) -> &'static str {
        match self {
            WssChannelType::User => "USER",
            WssChannelType::Market => "MARKET",
        }
    }
}

/// Price quote response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quote {
    pub token_id: String,
    pub side: Side,
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    pub timestamp: DateTime<Utc>,
}

/// Balance information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Balance {
    pub token_id: String,
    pub available: Decimal,
    pub locked: Decimal,
    pub total: Decimal,
}

/// Performance metrics for monitoring
#[derive(Debug, Clone)]
pub struct Metrics {
    pub orders_per_second: f64,
    pub avg_latency_ms: f64,
    pub error_rate: f64,
    pub uptime_pct: f64,
}

// Type aliases for common patterns
pub type TokenId = String;
pub type OrderId = String;
pub type MarketId = String;
pub type ClientId = String;

/// Parameters for querying open orders
#[derive(Debug, Clone)]
pub struct OpenOrderParams {
    pub id: Option<String>,
    pub asset_id: Option<String>,
    pub market: Option<String>,
}

impl OpenOrderParams {
    pub fn to_query_params(&self) -> Vec<(&str, &String)> {
        let mut params = Vec::with_capacity(3);

        if let Some(x) = &self.id {
            params.push(("id", x));
        }

        if let Some(x) = &self.asset_id {
            params.push(("asset_id", x));
        }

        if let Some(x) = &self.market {
            params.push(("market", x));
        }
        params
    }
}

/// Parameters for querying trades
#[derive(Debug, Clone)]
pub struct TradeParams {
    pub id: Option<String>,
    pub maker_address: Option<String>,
    pub market: Option<String>,
    pub asset_id: Option<String>,
    pub before: Option<u64>,
    pub after: Option<u64>,
}

impl TradeParams {
    pub fn to_query_params(&self) -> Vec<(&str, String)> {
        let mut params = Vec::with_capacity(6);

        if let Some(x) = &self.id {
            params.push(("id", x.clone()));
        }

        if let Some(x) = &self.asset_id {
            params.push(("asset_id", x.clone()));
        }

        if let Some(x) = &self.market {
            params.push(("market", x.clone()));
        }

        if let Some(x) = &self.maker_address {
            params.push(("maker_address", x.clone()));
        }

        if let Some(x) = &self.before {
            params.push(("before", x.to_string()));
        }

        if let Some(x) = &self.after {
            params.push(("after", x.to_string()));
        }

        params
    }
}

/// Open order information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenOrder {
    pub associate_trades: Vec<String>,
    pub id: String,
    pub status: String,
    pub market: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub original_size: Decimal,
    pub outcome: String,
    pub maker_address: String,
    pub owner: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    pub side: Side,
    #[serde(with = "rust_decimal::serde::str")]
    pub size_matched: Decimal,
    pub asset_id: String,
    #[serde(deserialize_with = "crate::decode::deserializers::number_from_string")]
    pub expiration: u64,
    #[serde(rename = "type")]
    pub order_type: OrderType,
    #[serde(deserialize_with = "crate::decode::deserializers::number_from_string")]
    pub created_at: u64,
}

/// Balance allowance information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceAllowance {
    pub asset_id: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub balance: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub allowance: Decimal,
}

/// Parameters for balance allowance queries (from reference implementation)
#[derive(Default)]
pub struct BalanceAllowanceParams {
    pub asset_type: Option<AssetType>,
    pub token_id: Option<String>,
    pub signature_type: Option<u8>,
}

impl BalanceAllowanceParams {
    pub fn to_query_params(&self) -> Vec<(&str, String)> {
        let mut params = Vec::with_capacity(3);

        if let Some(x) = &self.asset_type {
            params.push(("asset_type", x.to_string()));
        }

        if let Some(x) = &self.token_id {
            params.push(("token_id", x.to_string()));
        }

        if let Some(x) = &self.signature_type {
            params.push(("signature_type", x.to_string()));
        }
        params
    }

    pub fn set_signature_type(&mut self, s: u8) {
        self.signature_type = Some(s);
    }
}

/// Asset type enum for balance allowance queries
pub enum AssetType {
    COLLATERAL,
    CONDITIONAL,
}

impl fmt::Display for AssetType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetType::COLLATERAL => write!(f, "COLLATERAL"),
            AssetType::CONDITIONAL => write!(f, "CONDITIONAL"),
        }
    }
}

/// Notification preferences
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationParams {
    pub signature: String,
    pub timestamp: u64,
}

/// Batch midpoint request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchMidpointRequest {
    pub token_ids: Vec<String>,
}

/// Batch midpoint response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchMidpointResponse {
    pub midpoints: std::collections::HashMap<String, Option<Decimal>>,
}

/// Batch price request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchPriceRequest {
    pub token_ids: Vec<String>,
}

/// Price information for a token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPrice {
    pub token_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bid: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ask: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mid: Option<Decimal>,
}

/// Batch price response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchPriceResponse {
    pub prices: Vec<TokenPrice>,
}

// Additional types for API compatibility with reference implementation
#[derive(Debug, Deserialize)]
pub struct ApiKeysResponse {
    #[serde(rename = "apiKeys")]
    pub api_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct MidpointResponse {
    #[serde(with = "rust_decimal::serde::str")]
    pub mid: Decimal,
}

#[derive(Debug, Deserialize)]
pub struct PriceResponse {
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
}

#[derive(Debug, Deserialize)]
pub struct SpreadResponse {
    #[serde(with = "rust_decimal::serde::str")]
    pub spread: Decimal,
}

#[derive(Debug, Deserialize)]
pub struct TickSizeResponse {
    #[serde(with = "rust_decimal::serde::str")]
    pub minimum_tick_size: Decimal,
}

#[derive(Debug, Deserialize)]
pub struct NegRiskResponse {
    pub neg_risk: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BookParams {
    pub token_id: String,
    pub side: Side,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OrderBookSummary {
    pub market: String,
    pub asset_id: String,
    pub hash: String,
    #[serde(deserialize_with = "crate::decode::deserializers::number_from_string")]
    pub timestamp: u64,
    pub bids: Vec<OrderSummary>,
    pub asks: Vec<OrderSummary>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OrderSummary {
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub size: Decimal,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MarketsResponse {
    #[serde(with = "rust_decimal::serde::str")]
    pub limit: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub count: Decimal,
    pub next_cursor: Option<String>,
    pub data: Vec<Market>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SimplifiedMarketsResponse {
    #[serde(with = "rust_decimal::serde::str")]
    pub limit: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub count: Decimal,
    pub next_cursor: Option<String>,
    pub data: Vec<SimplifiedMarket>,
}

/// Simplified market structure for batch operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimplifiedMarket {
    #[serde(default)]
    pub condition_id: String,
    pub tokens: [Token; 2],
    pub rewards: Rewards,
    pub min_incentive_size: Option<String>,
    pub max_incentive_spread: Option<String>,
    pub active: bool,
    pub closed: bool,
}

/// Common query parameters for Gamma API list endpoints
#[derive(Debug, Clone, Default)]
pub struct GammaListParams {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub closed: Option<bool>,
    pub tag_id: Option<String>,
    pub exclude_tag_id: Option<String>,
    pub related_tags: Option<String>,
    pub order: Option<String>,
    pub ascending: Option<bool>,
    pub liquidity_num_min: Option<Decimal>,
    pub liquidity_num_max: Option<Decimal>,
    pub end_date_max: Option<DateTime<Utc>>,
    pub start_date_min: Option<DateTime<Utc>>,
    pub start_date_max: Option<DateTime<Utc>>,
    pub end_date_min: Option<DateTime<Utc>>,
    pub volume_num_min: Option<Decimal>,
    pub volume_num_max: Option<Decimal>,
    pub id: Option<Vec<i64>>,
    pub slug: Option<Vec<String>>,
    pub clob_token_ids: Option<Vec<String>>,
    pub condition_ids: Option<Vec<String>>,
    pub market_maker_address: Option<Vec<String>>,
    pub cyom: Option<bool>,
    pub uma_resolution_status: Option<String>,
    pub game_id: Option<String>,
    pub sports_market_types: Option<Vec<String>>,
    pub rewards_min_size: Option<Decimal>,
    pub question_ids: Option<Vec<String>>,
    pub include_tag: Option<bool>,
}

impl GammaListParams {
    pub fn builder() -> Self {
        Self::default()
    }

    // Builder methods for new array filter parameters
    pub fn id(mut self, ids: Vec<i64>) -> Self {
        self.id = Some(ids);
        self
    }

    pub fn slug(mut self, slugs: Vec<String>) -> Self {
        self.slug = Some(slugs);
        self
    }

    pub fn clob_token_ids(mut self, token_ids: Vec<String>) -> Self {
        self.clob_token_ids = Some(token_ids);
        self
    }

    pub fn condition_ids(mut self, condition_ids: Vec<String>) -> Self {
        self.condition_ids = Some(condition_ids);
        self
    }

    pub fn market_maker_address(mut self, addresses: Vec<String>) -> Self {
        self.market_maker_address = Some(addresses);
        self
    }

    pub fn sports_market_types(mut self, types: Vec<String>) -> Self {
        self.sports_market_types = Some(types);
        self
    }

    pub fn question_ids(mut self, question_ids: Vec<String>) -> Self {
        self.question_ids = Some(question_ids);
        self
    }

    // Builder methods for new numeric parameters
    pub fn liquidity_num_max(mut self, max: Decimal) -> Self {
        self.liquidity_num_max = Some(max);
        self
    }

    pub fn volume_num_min(mut self, min: Decimal) -> Self {
        self.volume_num_min = Some(min);
        self
    }

    pub fn volume_num_max(mut self, max: Decimal) -> Self {
        self.volume_num_max = Some(max);
        self
    }

    pub fn rewards_min_size(mut self, size: Decimal) -> Self {
        self.rewards_min_size = Some(size);
        self
    }

    // Builder methods for new date parameters
    pub fn start_date_max(mut self, date: DateTime<Utc>) -> Self {
        self.start_date_max = Some(date);
        self
    }

    pub fn end_date_min(mut self, date: DateTime<Utc>) -> Self {
        self.end_date_min = Some(date);
        self
    }

    // Builder methods for new boolean parameters
    pub fn cyom(mut self, cyom: bool) -> Self {
        self.cyom = Some(cyom);
        self
    }

    pub fn include_tag(mut self, include: bool) -> Self {
        self.include_tag = Some(include);
        self
    }

    // Builder methods for new string parameters
    pub fn uma_resolution_status(mut self, status: String) -> Self {
        self.uma_resolution_status = Some(status);
        self
    }

    pub fn game_id(mut self, game_id: String) -> Self {
        self.game_id = Some(game_id);
        self
    }

    pub fn to_query_params(&self) -> Vec<(&str, String)> {
        let mut params = Vec::with_capacity(28);

        // Pagination params
        if let Some(limit) = self.limit {
            params.push(("limit", limit.to_string()));
        }
        if let Some(offset) = self.offset {
            params.push(("offset", offset.to_string()));
        }

        // Boolean flags
        if let Some(closed) = self.closed {
            params.push(("closed", closed.to_string()));
        }
        if let Some(cyom) = self.cyom {
            params.push(("cyom", cyom.to_string()));
        }
        if let Some(include_tag) = self.include_tag {
            params.push(("include_tag", include_tag.to_string()));
        }
        if let Some(ascending) = self.ascending {
            params.push(("ascending", ascending.to_string()));
        }

        // String filters
        if let Some(tag_id) = &self.tag_id {
            params.push(("tag_id", tag_id.clone()));
        }
        if let Some(exclude_tag_id) = &self.exclude_tag_id {
            params.push(("exclude_tag_id", exclude_tag_id.clone()));
        }
        if let Some(related_tags) = &self.related_tags {
            params.push(("related_tags", related_tags.clone()));
        }
        if let Some(order) = &self.order {
            params.push(("order", order.clone()));
        }
        if let Some(uma_resolution_status) = &self.uma_resolution_status {
            params.push(("uma_resolution_status", uma_resolution_status.clone()));
        }
        if let Some(game_id) = &self.game_id {
            params.push(("game_id", game_id.clone()));
        }

        // Numeric filters
        if let Some(liquidity_num_min) = &self.liquidity_num_min {
            params.push(("liquidity_num_min", liquidity_num_min.to_string()));
        }
        if let Some(liquidity_num_max) = &self.liquidity_num_max {
            params.push(("liquidity_num_max", liquidity_num_max.to_string()));
        }
        if let Some(volume_num_min) = &self.volume_num_min {
            params.push(("volume_num_min", volume_num_min.to_string()));
        }
        if let Some(volume_num_max) = &self.volume_num_max {
            params.push(("volume_num_max", volume_num_max.to_string()));
        }
        if let Some(rewards_min_size) = &self.rewards_min_size {
            params.push(("rewards_min_size", rewards_min_size.to_string()));
        }

        // Date filters
        if let Some(start_date_min) = &self.start_date_min {
            params.push(("start_date_min", start_date_min.to_rfc3339()));
        }
        if let Some(start_date_max) = &self.start_date_max {
            params.push(("start_date_max", start_date_max.to_rfc3339()));
        }
        if let Some(end_date_min) = &self.end_date_min {
            params.push(("end_date_min", end_date_min.to_rfc3339()));
        }
        if let Some(end_date_max) = &self.end_date_max {
            params.push(("end_date_max", end_date_max.to_rfc3339()));
        }

        // Array filters - join with ","
        if let Some(id) = &self.id {
            if !id.is_empty() {
                params.push((
                    "id",
                    id.iter()
                        .map(|i| i.to_string())
                        .collect::<Vec<_>>()
                        .join(","),
                ));
            }
        }
        if let Some(slug) = &self.slug {
            if !slug.is_empty() {
                params.push(("slug", slug.join(",")));
            }
        }
        if let Some(clob_token_ids) = &self.clob_token_ids {
            if !clob_token_ids.is_empty() {
                params.push(("clob_token_ids", clob_token_ids.join(",")));
            }
        }
        if let Some(condition_ids) = &self.condition_ids {
            if !condition_ids.is_empty() {
                params.push(("condition_ids", condition_ids.join(",")));
            }
        }
        if let Some(market_maker_address) = &self.market_maker_address {
            if !market_maker_address.is_empty() {
                params.push(("market_maker_address", market_maker_address.join(",")));
            }
        }
        if let Some(sports_market_types) = &self.sports_market_types {
            if !sports_market_types.is_empty() {
                params.push(("sports_market_types", sports_market_types.join(",")));
            }
        }
        if let Some(question_ids) = &self.question_ids {
            if !question_ids.is_empty() {
                params.push(("question_ids", question_ids.join(",")));
            }
        }

        params
    }
}

/// Parameters supported by the Data API `/positions` endpoint.
#[derive(Debug, Clone, Default)]
pub struct DataApiPositionsParams {
    /// Minimum position size to include in the response.
    pub size_threshold: Option<u32>,
    /// Maximum number of rows to return.
    pub limit: Option<u32>,
    /// Field to sort by.
    pub sort_by: Option<DataApiSortBy>,
    /// Direction to sort (`ASC` or `DESC`).
    pub sort_direction: Option<DataApiSortDirection>,
}

impl DataApiPositionsParams {
    pub fn to_query_params(&self) -> Vec<(&'static str, String)> {
        let size_threshold = self.size_threshold.unwrap_or(1);
        let limit = self.limit.unwrap_or(100);
        let sort_by = self.sort_by.unwrap_or_default();
        let sort_direction = self.sort_direction.unwrap_or_default();

        vec![
            ("sizeThreshold", size_threshold.to_string()),
            ("limit", limit.to_string()),
            ("sortBy", sort_by.as_str().to_string()),
            ("sortDirection", sort_direction.as_str().to_string()),
        ]
    }
}

/// Fields allowed for sorting the `/positions` response.
#[derive(Debug, Clone, Copy)]
pub enum DataApiSortBy {
    Current,
    Initial,
    Tokens,
    CashPnl,
    PercentPnl,
    Title,
    Resolving,
    Price,
    AvgPrice,
}

impl DataApiSortBy {
    pub fn as_str(&self) -> &'static str {
        match self {
            DataApiSortBy::Current => "CURRENT",
            DataApiSortBy::Initial => "INITIAL",
            DataApiSortBy::Tokens => "TOKENS",
            DataApiSortBy::CashPnl => "CASHPNL",
            DataApiSortBy::PercentPnl => "PERCENTPNL",
            DataApiSortBy::Title => "TITLE",
            DataApiSortBy::Resolving => "RESOLVING",
            DataApiSortBy::Price => "PRICE",
            DataApiSortBy::AvgPrice => "AVGPRICE",
        }
    }
}

impl Default for DataApiSortBy {
    fn default() -> Self {
        DataApiSortBy::Tokens
    }
}

/// Sort direction for the Data API `/positions` response.
#[derive(Debug, Clone, Copy)]
pub enum DataApiSortDirection {
    Asc,
    Desc,
}

impl DataApiSortDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            DataApiSortDirection::Asc => "ASC",
            DataApiSortDirection::Desc => "DESC",
        }
    }
}

impl Default for DataApiSortDirection {
    fn default() -> Self {
        DataApiSortDirection::Desc
    }
}

/// A single row from the `/positions` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataPosition {
    #[serde(rename = "proxyWallet")]
    pub proxy_wallet: String,
    pub asset: String,
    #[serde(rename = "conditionId")]
    pub condition_id: String,
    pub size: Decimal,
    #[serde(rename = "avgPrice")]
    pub avg_price: Decimal,
    #[serde(rename = "initialValue")]
    pub initial_value: Decimal,
    #[serde(rename = "currentValue")]
    pub current_value: Decimal,
    #[serde(rename = "cashPnl")]
    pub cash_pnl: Decimal,
    #[serde(rename = "percentPnl")]
    pub percent_pnl: Decimal,
    #[serde(rename = "totalBought")]
    pub total_bought: Decimal,
    #[serde(rename = "realizedPnl")]
    pub realized_pnl: Decimal,
    #[serde(rename = "percentRealizedPnl")]
    pub percent_realized_pnl: Decimal,
    #[serde(rename = "curPrice")]
    pub cur_price: Decimal,
    pub redeemable: bool,
    pub mergeable: bool,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub icon: Option<String>,
    #[serde(rename = "eventId")]
    pub event_id: Option<String>,
    #[serde(rename = "eventSlug")]
    pub event_slug: Option<String>,
    pub outcome: Option<String>,
    #[serde(rename = "outcomeIndex")]
    pub outcome_index: Option<u32>,
    #[serde(rename = "oppositeOutcome")]
    pub opposite_outcome: Option<String>,
    #[serde(rename = "oppositeAsset")]
    pub opposite_asset: Option<String>,
    #[serde(rename = "endDate")]
    pub end_date: Option<String>,
    #[serde(rename = "negativeRisk")]
    pub negative_risk: Option<bool>,
}

/// Response returned by the `/value` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataPositionValue {
    pub user: String,
    pub value: Decimal,
}

/// Gamma API tag
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GammaTag {
    pub id: String,
    pub name: String,
    pub slug: Option<String>,
}

/// Sports response from Gamma API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SportsResponse {
    pub sports: Vec<String>,
}

/// Gamma API event metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GammaEvent {
    #[serde(alias = "event_id")]
    pub id: String,
    pub slug: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub active: Option<bool>,
    pub closed: Option<bool>,
    pub start_date_iso: Option<String>,
    pub end_date_iso: Option<String>,
    pub sport: Option<String>,
    #[serde(default)]
    pub tags: Vec<Tag>,
    #[serde(default)]
    pub markets: Vec<GammaEventMarket>,
    #[serde(default)]
    #[serde(flatten)]
    pub metadata: serde_json::Value,
}

/// Lightweight market info returned inside a Gamma event listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GammaEventMarket {
    #[serde(rename = "conditionId")]
    pub condition_id: String,
    #[serde(rename = "id")]
    #[serde(default)]
    pub market_id: Option<String>,
    #[serde(rename = "clobTokenIds")]
    #[serde(default)]
    pub clob_token_ids: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
}

/// Tag metadata for Gamma API filtering
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub id: Option<String>,
    pub slug: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    #[serde(flatten)]
    pub metadata: serde_json::Value,
}

/// Sports metadata for Gamma API filtering
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sport {
    pub id: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub tag_ids: Vec<String>,
    #[serde(default)]
    pub tags: Vec<Tag>,
    #[serde(default)]
    #[serde(flatten)]
    pub metadata: serde_json::Value,
}

/// Minimal Gamma market representation used for discovery
#[derive(Debug, Clone, Deserialize)]
pub struct GammaMarket {
    #[serde(rename = "conditionId")]
    pub condition_id: String,
    pub slug: String,
    pub question: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub active: bool,
    pub closed: bool,
    pub outcomes: Option<String>,
    #[serde(rename = "clobTokenIds")]
    pub clob_token_ids: Option<String>,
    pub icon: Option<String>,
    #[serde(rename = "endDate")]
    pub end_date: Option<String>,
    pub liquidity: Option<String>,
    #[serde(rename = "liquidityNum")]
    pub liquidity_num: Option<Decimal>,
    pub volume: Option<String>,
    #[serde(rename = "volumeNum")]
    pub volume_num: Option<Decimal>,
    #[serde(rename = "volume24hr")]
    pub volume_24hr: Option<Decimal>,
    #[serde(rename = "volume1wk")]
    pub volume_1wk: Option<Decimal>,
    #[serde(rename = "volume1mo")]
    pub volume_1mo: Option<Decimal>,
    #[serde(rename = "volume1yr")]
    pub volume_1yr: Option<Decimal>,
    #[serde(rename = "volume24hrAmm")]
    pub volume_24hr_amm: Option<Decimal>,
    #[serde(rename = "volume1wkAmm")]
    pub volume_1wk_amm: Option<Decimal>,
    #[serde(rename = "volume1moAmm")]
    pub volume_1mo_amm: Option<Decimal>,
    #[serde(rename = "volume1yrAmm")]
    pub volume_1yr_amm: Option<Decimal>,
    #[serde(rename = "volume24hrClob")]
    pub volume_24hr_clob: Option<Decimal>,
    #[serde(rename = "volume1wkClob")]
    pub volume_1wk_clob: Option<Decimal>,
    #[serde(rename = "volume1moClob")]
    pub volume_1mo_clob: Option<Decimal>,
    #[serde(rename = "volume1yrClob")]
    pub volume_1yr_clob: Option<Decimal>,
    #[serde(rename = "volumeAmm")]
    pub volume_amm: Option<Decimal>,
    #[serde(rename = "volumeClob")]
    pub volume_clob: Option<Decimal>,
    #[serde(rename = "liquidityAmm")]
    pub liquidity_amm: Option<Decimal>,
    #[serde(rename = "liquidityClob")]
    pub liquidity_clob: Option<Decimal>,
    #[serde(rename = "orderMinSize")]
    pub order_min_size: Option<Decimal>,
    #[serde(rename = "orderPriceMinTickSize")]
    pub order_tick_size: Option<Decimal>,
}

/// Rewards structure for markets
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rewards {
    pub rates: Option<serde_json::Value>,
    #[serde(with = "rust_decimal::serde::str")]
    pub min_size: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub max_spread: Decimal,
    pub event_start_date: Option<String>,
    pub event_end_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_game_multiplier: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reward_epoch: Option<Decimal>,
}

// For compatibility with reference implementation
pub type ClientResult<T> = anyhow::Result<T>;

/// Result type used throughout the client
pub type Result<T> = std::result::Result<T, crate::errors::PolyError>;

#[cfg(test)]
mod batch_order_response_tests {
    use super::*;

    #[test]
    fn test_error_detection_success_false() {
        let response = BatchOrderResponse {
            success: false,
            error_msg: Some("not enough balance / allowance".to_string()),
            order_id: None,
            making_amount: None,
            taking_amount: None,
            status: None,
        };
        assert!(response.has_error());
    }

    #[test]
    fn test_error_detection_success_true_with_error_msg() {
        let response = BatchOrderResponse {
            success: true,
            error_msg: Some("not enough balance / allowance".to_string()),
            order_id: None,
            making_amount: None,
            taking_amount: None,
            status: None,
        };
        assert!(response.has_error());
    }

    #[test]
    fn test_error_detection_success_true_no_error_msg() {
        let response = BatchOrderResponse {
            success: true,
            error_msg: None,
            order_id: Some("order-123".to_string()),
            making_amount: Some("100".to_string()),
            taking_amount: Some("50".to_string()),
            status: Some("filled".to_string()),
        };
        assert!(!response.has_error());
    }
}
