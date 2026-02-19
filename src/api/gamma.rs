//! Gamma API client for Polymarket markets, events, and tags

use crate::errors::{PolyError, Result};
use crate::types::{
    GammaEvent, GammaMarket, GammaTag, Market, SimplifiedMarketsResponse, SportsResponse,
};
use reqwest::Client;
use serde_json::Value;

const DEFAULT_GAMMA_BASE: &str = "https://gamma-api.polymarket.com";
const GAMMA_MARKETS_LIMIT: u32 = 50;

#[derive(Debug, Clone)]
pub struct GammaClient {
    http_client: Client,
    base_url: String,
}

impl GammaClient {
    pub fn new() -> Self {
        Self {
            http_client: Client::new(),
            base_url: DEFAULT_GAMMA_BASE.to_string(),
        }
    }

    pub fn with_base_url(mut self, url: &str) -> Self {
        self.base_url = url.to_string();
        self
    }

    fn build_url(&self, path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            base.to_string()
        } else {
            format!("{}/{}", base, path)
        }
    }

    pub fn gamma_url(&self, path: &str) -> String {
        self.build_url(path)
    }

    pub async fn get_markets(
        &self,
        next_cursor: Option<&str>,
        params: Option<&crate::types::GammaListParams>,
    ) -> Result<crate::types::MarketsResponse> {
        todo!()
    }

    pub async fn get_event(&self, condition_id: &str) -> Result<GammaEvent> {
        todo!()
    }

    pub async fn get_events(&self, limit: Option<u32>) -> Result<Vec<GammaEvent>> {
        todo!()
    }

    pub async fn get_event_by_slug(&self, slug: &str) -> Result<GammaEvent> {
        todo!()
    }

    pub async fn get_tags(&self) -> Result<Vec<GammaTag>> {
        todo!()
    }

    pub async fn get_sports(&self) -> Result<Vec<String>> {
        todo!()
    }
}

impl Default for GammaClient {
    fn default() -> Self {
        Self::new()
    }
}
