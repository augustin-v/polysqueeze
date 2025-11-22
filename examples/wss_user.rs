use polysqueeze::{
    OrderArgs,
    client::ClobClient,
    errors::{PolyError, Result},
    types::{GammaListParams, OrderType, Side},
    wss::{WssUserClient, WssUserEvent},
};
use rust_decimal::{Decimal, prelude::FromPrimitive};
use std::{env, str::FromStr};

/// Fail fast when an expected environment variable is missing.
fn env_var(key: &str) -> String {
    env::var(key).expect(&format!("{} must be set for the user-channel example", key))
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

    let l1_client = ClobClient::with_l1_headers(&base_url, &private_key, chain_id);
    let creds = l1_client.create_or_derive_api_key(None).await?;
    let mut l2_client =
        ClobClient::with_l2_headers(&base_url, &private_key, chain_id, creds.clone());

    if let Ok(funder) = env::var("POLY_FUNDER") {
        l2_client.set_funder(&funder)?;
    }

    let min_liquidity = env::var("POLY_WSS_MIN_LIQUIDITY")
        .ok()
        .and_then(|value| Decimal::from_str(&value).ok())
        .unwrap_or_else(|| Decimal::from(1_000_000));

    let gamma_params = GammaListParams {
        limit: Some(5),
        liquidity_num_min: Some(min_liquidity),
        ..Default::default()
    };
    let markets_response = l2_client.get_markets(None, Some(&gamma_params)).await?;

    let market_ids: Vec<String> = markets_response
        .data
        .iter()
        .filter(|market| market.liquidity_num.unwrap_or(Decimal::ZERO) >= min_liquidity)
        .map(|market| market.condition_id.clone())
        .filter(|id| !id.is_empty())
        .take(2)
        .collect();

    if market_ids.is_empty() {
        return Err(PolyError::validation(
            "Gamma did not return any markets with condition_ids",
        ));
    }

    let primary_market = markets_response
        .data
        .iter()
        .find(|market| {
            !market.clob_token_ids.is_empty()
                && market.liquidity_num.unwrap_or(Decimal::ZERO) >= min_liquidity
        })
        .ok_or_else(|| {
            PolyError::validation("No Gamma markets returned a CLOB token id for trading")
        })?;
    let token_id = primary_market.clob_token_ids.first().unwrap().clone();

    let book = l2_client.get_order_book(&token_id).await?;
    let _best_ask = book.asks.first().ok_or_else(|| {
        PolyError::validation("Order book has no asks; cannot derive a safe price")
    })?;
    let tick_size = primary_market.minimum_tick_size;
    let max_bid_level = book
        .bids
        .iter()
        .max_by(|a, b| {
            a.size
                .partial_cmp(&b.size)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .ok_or_else(|| PolyError::validation("Order book has no depth data"))?;
    let far_price =
        max_bid_level.price - tick_size * Decimal::from_u32(10).unwrap_or(Decimal::ZERO);
    let order_price = if far_price > Decimal::ZERO {
        far_price
    } else {
        max_bid_level.price - tick_size
    };
    let order_size = Decimal::new(5, 0); // 0.1 size
    let order_args = OrderArgs::new(&token_id, order_price, order_size, Side::BUY);

    let mut user_client = WssUserClient::new(creds.clone());
    user_client.subscribe(market_ids.clone()).await?;

    println!(
        "Subscribed to user channel for markets {market_ids:?} (waiting for your cancel/update)..."
    );
    let signed_order = l2_client
        .create_order(&order_args, None, None, None)
        .await?;
    let response = l2_client.post_order(signed_order, OrderType::GTC).await?;
    let order_id = response
        .get("orderID")
        .and_then(|value| value.as_str())
        .ok_or_else(|| PolyError::validation("Post order response missing orderID"))?
        .to_string();
    println!(
        "Placed order on {} @ {}: {response:#}",
        token_id, order_price
    );
    println!(
        "Order id {} - cancel it via the `wss_cancel` example while this stream is running.",
        order_id
    );

    loop {
        match user_client.next_event().await {
            Ok(WssUserEvent::Order(order)) => {
                println!(
                    "order {} {} matched={} price={} side={}",
                    order.id,
                    order.message_type,
                    order.size_matched,
                    order.price,
                    order.side.as_str()
                );
                if order.message_type.eq_ignore_ascii_case("CANCELLATION") {
                    println!("Order {} cancelled; exiting.", order.id);
                    break;
                }
            }
            Ok(WssUserEvent::Trade(trade)) => {
                println!(
                    "trade {} {} {}@{} status={}",
                    trade.id,
                    trade.side.as_str(),
                    trade.size,
                    trade.price,
                    trade.status
                );
            }
            Err(err) => {
                eprintln!("user stream error: {}", err);
                break;
            }
        }
    }

    Ok(())
}
