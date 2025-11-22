//! Lightweight WSS client for Polymarket market channel updates.
//!
//! This module focuses on the public market channel exposed at
//! `wss://ws-subscriptions-clob.polymarket.com/ws/`. It maintains a single
//! reconnecting connection, replays the most recent market/asset subscriptions,
//! and exposes typed events for books, price changes, tick size changes, and
//! last trade notifications.

use crate::errors::{PolyError, Result};
use crate::types::{OrderSummary, Side};
use chrono::{DateTime, Utc};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::sleep;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async, tungstenite::protocol::Message,
};
use tracing::warn;

const DEFAULT_WSS_BASE: &str = "wss://ws-subscriptions-clob.polymarket.com";
const MARKET_CHANNEL_PATH: &str = "/ws/market";
const BASE_RECONNECT_DELAY: Duration = Duration::from_millis(250);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(10);
const MAX_RECONNECT_ATTEMPTS: u32 = 8;

/// Represents a parsed market broadcast from the public market channel.
#[derive(Debug, Clone)]
pub enum WssMarketEvent {
    Book(MarketBook),
    PriceChange(PriceChangeMessage),
    TickSizeChange(TickSizeChangeMessage),
    LastTrade(LastTradeMessage),
}

/// Book summary message
#[derive(Debug, Clone, Deserialize)]
pub struct MarketBook {
    #[serde(rename = "event_type")]
    pub event_type: String,
    pub asset_id: String,
    pub market: String,
    pub timestamp: String,
    pub hash: String,
    pub bids: Vec<OrderSummary>,
    pub asks: Vec<OrderSummary>,
}

/// Payload for price change notifications.
#[derive(Debug, Clone, Deserialize)]
pub struct PriceChangeMessage {
    #[serde(rename = "event_type")]
    pub event_type: String,
    pub market: String,
    #[serde(rename = "price_changes")]
    pub price_changes: Vec<PriceChangeEntry>,
    pub timestamp: String,
}

/// Individual price change entry.
#[derive(Debug, Clone, Deserialize)]
pub struct PriceChangeEntry {
    pub asset_id: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub price: rust_decimal::Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub size: rust_decimal::Decimal,
    pub side: Side,
    pub hash: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub best_bid: rust_decimal::Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub best_ask: rust_decimal::Decimal,
}

/// Tick size change events.
#[derive(Debug, Clone, Deserialize)]
pub struct TickSizeChangeMessage {
    #[serde(rename = "event_type")]
    pub event_type: String,
    pub asset_id: String,
    pub market: String,
    #[serde(rename = "old_tick_size", with = "rust_decimal::serde::str")]
    pub old_tick_size: rust_decimal::Decimal,
    #[serde(rename = "new_tick_size", with = "rust_decimal::serde::str")]
    pub new_tick_size: rust_decimal::Decimal,
    pub side: String,
    pub timestamp: String,
}

/// Trade events emitted when a trade settles.
#[derive(Debug, Clone, Deserialize)]
pub struct LastTradeMessage {
    #[serde(rename = "event_type")]
    pub event_type: String,
    pub asset_id: String,
    pub fee_rate_bps: String,
    pub market: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub price: rust_decimal::Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub size: rust_decimal::Decimal,
    pub side: Side,
    pub timestamp: String,
}

/// Simple stats for monitoring connection health.
#[derive(Debug, Clone)]
pub struct WssStats {
    pub messages_received: u64,
    pub errors: u64,
    pub reconnect_count: u32,
    pub last_message_time: Option<DateTime<Utc>>,
}

impl Default for WssStats {
    fn default() -> Self {
        Self {
            messages_received: 0,
            errors: 0,
            reconnect_count: 0,
            last_message_time: None,
        }
    }
}

/// Reconnecting client for the market channel.
pub struct WssMarketClient {
    connect_url: String,
    connection: Option<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    subscribed_asset_ids: Vec<String>,
    stats: WssStats,
    disconnect_history: VecDeque<DateTime<Utc>>,
    pending_events: VecDeque<WssMarketEvent>,
}

impl WssMarketClient {
    /// Create a new instance using the default Polymarket WSS base.
    pub fn new() -> Self {
        Self::with_url(DEFAULT_WSS_BASE)
    }

    /// Create a new client against a custom endpoint (useful for tests).
    pub fn with_url(url: &str) -> Self {
        let trimmed = url.trim_end_matches('/');
        let connect_url = format!("{}{}", trimmed, MARKET_CHANNEL_PATH);
        Self {
            connection: None,
            subscribed_asset_ids: Vec::new(),
            stats: WssStats::default(),
            disconnect_history: VecDeque::with_capacity(5),
            connect_url,
            pending_events: VecDeque::new(),
        }
    }

    /// Access connection stats for observability.
    pub fn stats(&self) -> WssStats {
        self.stats.clone()
    }

    fn format_subscription(&self) -> Value {
        json!({
            "type": "market",
            "assets_ids": self.subscribed_asset_ids,
        })
    }

    async fn send_subscription(&mut self) -> Result<()> {
        if self.subscribed_asset_ids.is_empty() {
            return Ok(());
        }

        let message = self.format_subscription();
        self.send_raw_message(message).await
    }

    async fn send_raw_message(&mut self, message: Value) -> Result<()> {
        if let Some(connection) = self.connection.as_mut() {
            let text = serde_json::to_string(&message).map_err(|e| {
                PolyError::parse(
                    format!("Failed to serialize subscription message: {}", e),
                    None,
                )
            })?;
            connection
                .send(Message::Text(text.into()))
                .await
                .map_err(|e| {
                    PolyError::stream(
                        format!("Failed to send message: {}", e),
                        crate::errors::StreamErrorKind::MessageCorrupted,
                    )
                })?;
            return Ok(());
        }
        Err(PolyError::stream(
            "WebSocket connection not established",
            crate::errors::StreamErrorKind::ConnectionFailed,
        ))
    }

    async fn connect(&mut self) -> Result<()> {
        let mut attempts = 0;
        loop {
            match connect_async(&self.connect_url).await {
                Ok((socket, _)) => {
                    self.connection = Some(socket);
                    if attempts > 0 {
                        self.stats.reconnect_count += 1;
                    }
                    return Ok(());
                }
                Err(err) => {
                    attempts += 1;
                    let delay = self.reconnect_delay(attempts);
                    self.stats.errors += 1;
                    if attempts >= MAX_RECONNECT_ATTEMPTS {
                        return Err(PolyError::stream(
                            format!("Failed to connect after {} attempts: {}", attempts, err),
                            crate::errors::StreamErrorKind::ConnectionFailed,
                        ));
                    }
                    sleep(delay).await;
                }
            }
        }
    }

    fn reconnect_delay(&self, attempts: u32) -> Duration {
        let millis = BASE_RECONNECT_DELAY.as_millis() as u128 * attempts as u128;
        let desired =
            Duration::from_millis(millis.min(MAX_RECONNECT_DELAY.as_millis() as u128) as u64);
        desired
    }

    async fn ensure_connection(&mut self) -> Result<()> {
        if self.connection.is_none() {
            self.connect().await?;
            self.send_subscription().await?;
        }
        Ok(())
    }

    /// Subscribe to the market channel for the provided token/market IDs.
    pub async fn subscribe(&mut self, asset_ids: Vec<String>) -> Result<()> {
        self.subscribed_asset_ids = asset_ids;
        self.ensure_connection().await?;
        self.send_subscription().await
    }

    /// Read the next market channel event, reconnecting transparently when
    /// the socket drops.
    pub async fn next_event(&mut self) -> Result<WssMarketEvent> {
        loop {
            if let Some(evt) = self.pending_events.pop_front() {
                return Ok(evt);
            }
            self.ensure_connection().await?;

            match self.connection.as_mut().unwrap().next().await {
                Some(Ok(Message::Text(text))) => {
                    let trimmed = text.trim();
                    if trimmed.eq_ignore_ascii_case("ping") || trimmed.eq_ignore_ascii_case("pong")
                    {
                        continue;
                    }
                    let first_char = trimmed.chars().next();
                    if first_char != Some('{') && first_char != Some('[') {
                        warn!("ignoring unexpected text frame: {}", trimmed);
                        continue;
                    }
                    let events = parse_market_events(&text)?;
                    self.stats.messages_received += events.len() as u64;
                    self.stats.last_message_time = Some(Utc::now());
                    for evt in events {
                        self.pending_events.push_back(evt);
                    }
                    if let Some(evt) = self.pending_events.pop_front() {
                        return Ok(evt);
                    }
                    continue;
                }
                Some(Ok(Message::Ping(payload))) => {
                    if let Some(connection) = self.connection.as_mut() {
                        let _ = connection.send(Message::Pong(payload)).await;
                    }
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) => {
                    self.disconnect_history.push_back(Utc::now());
                    if self.disconnect_history.len() > 5 {
                        self.disconnect_history.pop_front();
                    }
                    self.connection = None;
                }
                Some(Ok(_)) => {}
                Some(Err(err)) => {
                    warn!("WebSocket error: {}", err);
                    self.connection = None;
                    self.stats.errors += 1;
                    continue;
                }
                None => {
                    self.connection = None;
                }
            }
        }
    }
}

fn parse_market_events(text: &str) -> Result<Vec<WssMarketEvent>> {
    let value: Value = serde_json::from_str(text)
        .map_err(|err| PolyError::parse(format!("Invalid JSON: {}", err), Some(Box::new(err))))?;

    if let Some(array) = value.as_array() {
        array
            .iter()
            .map(parse_market_event_value)
            .collect::<Result<Vec<_>>>()
    } else {
        Ok(vec![parse_market_event_value(&value)?])
    }
}

fn parse_market_event_value(value: &Value) -> Result<WssMarketEvent> {
    let event_type = value
        .get("event_type")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("type").and_then(|v| v.as_str()))
        .ok_or_else(|| PolyError::parse("Missing event_type/type in market message", None))?;

    match event_type {
        "book" => {
            let parsed: MarketBook = serde_json::from_value(value.clone()).map_err(|err| {
                PolyError::parse(
                    format!("Failed to parse book message: {}", err),
                    Some(Box::new(err)),
                )
            })?;
            Ok(WssMarketEvent::Book(parsed))
        }
        "price_change" => {
            let parsed =
                serde_json::from_value::<PriceChangeMessage>(value.clone()).map_err(|err| {
                    PolyError::parse(
                        format!("Failed to parse price_change: {}", err),
                        Some(Box::new(err)),
                    )
                })?;
            Ok(WssMarketEvent::PriceChange(parsed))
        }
        "tick_size_change" => {
            let parsed =
                serde_json::from_value::<TickSizeChangeMessage>(value.clone()).map_err(|err| {
                    PolyError::parse(
                        format!("Failed to parse tick_size_change: {}", err),
                        Some(Box::new(err)),
                    )
                })?;
            Ok(WssMarketEvent::TickSizeChange(parsed))
        }
        "last_trade_price" => {
            let parsed =
                serde_json::from_value::<LastTradeMessage>(value.clone()).map_err(|err| {
                    PolyError::parse(
                        format!("Failed to parse last_trade_price: {}", err),
                        Some(Box::new(err)),
                    )
                })?;
            Ok(WssMarketEvent::LastTrade(parsed))
        }
        other => Err(PolyError::parse(
            format!("Unknown market event_type: {}", other),
            None,
        )),
    }
}