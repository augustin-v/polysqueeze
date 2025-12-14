//! Polysqueeze is a Rust SDK for interacting with Polymarket's REST, Gamma, and
//! WebSocket APIs.
//! Use it to authenticate, build signed orders, stream live book data, or query
//! historical fills and markets.

pub mod auth;
pub mod book;
pub mod client;
pub mod config;
pub mod decode;
pub mod errors;
pub mod fill;
pub mod orders;
pub mod types;
pub mod utils;
pub mod ws;
pub mod wss;

pub use crate::client::{
    ClobClient, CreateOrderOptions, DataApiClient, MarketClient, OrderArgs, PolyClient,
};
pub use crate::errors::{PolyError, Result};
pub use crate::types::{ApiCredentials, SignedOrderRequest};
pub use crate::wss::{WssMarketClient, WssMarketEvent, WssUserClient, WssUserEvent};
