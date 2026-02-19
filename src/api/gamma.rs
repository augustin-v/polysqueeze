//! Gamma API client for Polymarket markets, events, and tags

use crate::errors::{PolyError, Result};
use crate::types::{
    GammaEvent, GammaListParams, GammaMarket, GammaTag, Market, SimplifiedMarketsResponse,
    SportsResponse,
};
use base64::Engine;
use chrono::{Duration, Utc};
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use serde::de::DeserializeOwned;
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

    fn encode_cursor(cursor: u64) -> String {
        base64::engine::general_purpose::STANDARD.encode(cursor.to_string())
    }

    fn decode_cursor(cursor: &str) -> Option<u64> {
        base64::engine::general_purpose::STANDARD
            .decode(cursor)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|s| s.parse::<u64>().ok())
    }

    pub async fn get_markets(
        &self,
        next_cursor: Option<&str>,
        params: Option<&crate::types::GammaListParams>,
    ) -> Result<crate::types::MarketsResponse> {
        let offset = params
            .and_then(|options| options.offset.map(u64::from))
            .or_else(|| next_cursor.and_then(Self::decode_cursor))
            .unwrap_or(0);

        let limit = params
            .and_then(|options| options.limit)
            .unwrap_or(GAMMA_MARKETS_LIMIT);

        let mut query = vec![("limit", limit.to_string()), ("offset", offset.to_string())];

        let liquidity_min = params
            .and_then(|options| options.liquidity_num_min)
            .unwrap_or_else(|| Decimal::from(10_000));
        query.push(("liquidity_num_min", liquidity_min.to_string()));

        let min_end_date = Utc::now() + Duration::weeks(3);
        let end_date_max = params
            .and_then(|options| options.end_date_max)
            .unwrap_or(min_end_date);
        let end_date_max = if end_date_max < min_end_date {
            min_end_date
        } else {
            end_date_max
        };
        query.push(("end_date_max", end_date_max.to_rfc3339()));

        if let Some(start_date_min) = params.and_then(|options| options.start_date_min) {
            query.push(("start_date_min", start_date_min.to_rfc3339()));
        }

        if let Some(options) = params {
            if let Some(closed) = options.closed {
                query.push(("closed", closed.to_string()));
            } else {
                query.push(("closed", "false".to_string()));
            }

            if let Some(tag_id) = &options.tag_id {
                query.push(("tag_id", tag_id.clone()));
            }
            if let Some(exclude_tag_id) = &options.exclude_tag_id {
                query.push(("exclude_tag_id", exclude_tag_id.clone()));
            }
            if let Some(related_tags) = &options.related_tags {
                query.push(("related_tags", related_tags.clone()));
            }
            if let Some(order) = &options.order {
                query.push(("order", order.clone()));
            }
            if let Some(ascending) = options.ascending {
                query.push(("ascending", ascending.to_string()));
            }

            if let Some(cyom) = options.cyom {
                query.push(("cyom", cyom.to_string()));
            }
            if let Some(include_tag) = options.include_tag {
                query.push(("include_tag", include_tag.to_string()));
            }

            if let Some(uma_resolution_status) = &options.uma_resolution_status {
                query.push(("uma_resolution_status", uma_resolution_status.clone()));
            }
            if let Some(game_id) = &options.game_id {
                query.push(("game_id", game_id.clone()));
            }

            if let Some(liquidity_num_max) = options.liquidity_num_max {
                query.push(("liquidity_num_max", liquidity_num_max.to_string()));
            }
            if let Some(volume_num_min) = options.volume_num_min {
                query.push(("volume_num_min", volume_num_min.to_string()));
            }
            if let Some(volume_num_max) = options.volume_num_max {
                query.push(("volume_num_max", volume_num_max.to_string()));
            }
            if let Some(rewards_min_size) = options.rewards_min_size {
                query.push(("rewards_min_size", rewards_min_size.to_string()));
            }

            if let Some(start_date_max) = options.start_date_max {
                query.push(("start_date_max", start_date_max.to_rfc3339()));
            }
            if let Some(end_date_min) = options.end_date_min {
                query.push(("end_date_min", end_date_min.to_rfc3339()));
            }

            if let Some(id) = &options.id {
                if !id.is_empty() {
                    query.push((
                        "id",
                        id.iter()
                            .map(|i| i.to_string())
                            .collect::<Vec<_>>()
                            .join(","),
                    ));
                }
            }
            if let Some(slug) = &options.slug {
                if !slug.is_empty() {
                    query.push(("slug", slug.join(",")));
                }
            }
            if let Some(clob_token_ids) = &options.clob_token_ids {
                if !clob_token_ids.is_empty() {
                    query.push(("clob_token_ids", clob_token_ids.join(",")));
                }
            }
            if let Some(condition_ids) = &options.condition_ids {
                if !condition_ids.is_empty() {
                    query.push(("condition_ids", condition_ids.join(",")));
                }
            }
            if let Some(market_maker_address) = &options.market_maker_address {
                if !market_maker_address.is_empty() {
                    query.push(("market_maker_address", market_maker_address.join(",")));
                }
            }
            if let Some(sports_market_types) = &options.sports_market_types {
                if !sports_market_types.is_empty() {
                    query.push(("sports_market_types", sports_market_types.join(",")));
                }
            }
            if let Some(question_ids) = &options.question_ids {
                if !question_ids.is_empty() {
                    query.push(("question_ids", question_ids.join(",")));
                }
            }
        } else {
            query.push(("closed", "false".to_string()));
        }

        let response = self
            .http_client
            .get(self.gamma_url("markets"))
            .query(&query)
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to fetch markets",
            ));
        }

        let body = response
            .text()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to read response body: {}", e), None))?;

        let gamma_markets: Vec<crate::types::GammaMarket> = serde_json::from_str(&body)
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))?;

        let count = gamma_markets.len();
        let next_cursor = if count < limit as usize {
            None
        } else {
            Some(Self::encode_cursor(offset + count as u64))
        };
        let markets = gamma_markets
            .into_iter()
            .map(|gamma| gamma.into())
            .collect::<Vec<_>>();

        Ok(crate::types::MarketsResponse {
            limit: Decimal::from(limit),
            count: Decimal::from_i64(count as i64).unwrap_or(Decimal::ZERO),
            next_cursor,
            data: markets,
        })
    }

    pub async fn get_event(&self, condition_id: &str) -> Result<GammaEvent> {
        todo!()
    }

    pub async fn get_events(&self, params: Option<&GammaListParams>) -> Result<Vec<GammaEvent>> {
        let mut request = self.http_client.get(self.gamma_url("events"));

        if let Some(options) = params {
            request = request.query(&options.to_query_params());
        }

        let response = request
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to fetch Gamma events",
            ));
        }

        let payload: Value = response
            .json()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))?;

        self.parse_gamma_list(payload, "Gamma events")
    }

    pub async fn get_event_by_slug(&self, slug: &str) -> Result<GammaEvent> {
        let response = self
            .http_client
            .get(self.gamma_url(&format!("events/slug/{}", slug)))
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to fetch Gamma event",
            ));
        }

        response
            .json::<GammaEvent>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    pub async fn get_event_by_id(&self, event_id: &str) -> Result<GammaEvent> {
        let response = self
            .http_client
            .get(self.gamma_url(&format!("events/{}", event_id)))
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to fetch Gamma event",
            ));
        }

        response
            .json::<GammaEvent>()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))
    }

    pub async fn get_tags(&self) -> Result<Vec<GammaTag>> {
        let response = self
            .http_client
            .get(self.gamma_url("tags"))
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to fetch Gamma tags",
            ));
        }

        let payload: Value = response
            .json()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))?;

        self.parse_gamma_list(payload, "Gamma tags")
    }

    pub async fn get_sports(&self) -> Result<Vec<crate::types::Sport>> {
        let response = self
            .http_client
            .get(self.gamma_url("sports"))
            .send()
            .await
            .map_err(|e| PolyError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyError::api(
                response.status().as_u16(),
                "Failed to fetch Gamma sports",
            ));
        }

        let payload: Value = response
            .json()
            .await
            .map_err(|e| PolyError::parse(format!("Failed to parse response: {}", e), None))?;

        self.parse_gamma_list(payload, "Gamma sports")
    }

    fn parse_gamma_list<T>(&self, value: Value, ctx: &str) -> Result<Vec<T>>
    where
        T: DeserializeOwned,
    {
        let payload = if let Some(data) = value.get("data") {
            data.clone()
        } else {
            value
        };

        serde_json::from_value::<Vec<T>>(payload)
            .map_err(|err| PolyError::parse(format!("Failed to parse {}: {}", ctx, err), None))
    }
}

impl Default for GammaClient {
    fn default() -> Self {
        Self::new()
    }
}
