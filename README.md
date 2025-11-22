# Polysqueeze

Polysqueeze is a Rust SDK for interacting with Polymarket's REST APIs; the markets,
orders, and authentication helpers are implemented today, wss only implements the public `MARKETS` channel. If you hit a 401 or other auth pain point, raise
an issue and the implementation is guaranteed to keep evolving.



## Highlights

- Fully working REST helpers for the Gamma and CLOB APIs (markets, fills, single orders,
  authentication, helper types).
- Order creation flows have live regression coverage for the single-order path;
  batch/multi-order flows still need more testing and contributions are welcome.
- Gamma data types for markets, tokens, order books, rewards, etc.
- Configuration helpers for Polygon mainnet + testnet (testing has only been done on mainnet), plus shared utils for signing,
  math, and fills.

## Quickstart

1. Add `polysqueeze` to your deps:
```bash
cargo add polysqueeze
```

2. Create a client, derive API keys, and post a sample order. This is the one path
   that currently runs against the live API (single-order placement); batch order
   flows and broader WebSocket handling still need more testing and are open for
   contributions.

```rust
use polysqueeze::{types::GammaListParams, ClobClient, OrderArgs, OrderType, Side};
use rust_decimal::Decimal;

#[tokio::main]
async fn main() -> polysqueeze::Result<()> {
    let base_url = "https://clob.polymarket.com";
    let private_key = std::env::var("POLY_PRIVATE_KEY")?;

    let l1_client = ClobClient::with_l1_headers(base_url, &private_key, 137);
    let creds = l1_client.create_or_derive_api_key(None).await?;

    let mut client = ClobClient::with_l2_headers(base_url, &private_key, 137, creds.clone());
    client.set_api_creds(creds.clone());

    let gamma_params = GammaListParams {
        limit: Some(5),
        ..Default::default()
    };
    let market_data = client.get_markets(None, Some(&gamma_params)).await?;
    println!("{} markets fetched", market_data.data.len());

    let order_args = OrderArgs::new(
        "token-example",
        Decimal::new(5000, 4),
        Decimal::new(1, 0),
        Side::BUY,
    );

    let signed_order = client.create_order(&order_args, None, None, None).await?;
    client.post_order(signed_order, OrderType::GTC).await?;

    Ok(())
}
```

3. Explore `ws::WebSocketStream` (WIP TODO)  for real-time
   updates, or use `book::OrderBookCache` for a cached view of the order book.

## Example

The smoke-testing logic from `tests/place_order.rs` is also available as
`examples/order.rs`. To run it locally:

```bash
cargo run --example order
```

It expects the same environment variables as the test (private key, API creds,
network, etc.) and drops a microscopic order when `RUN_PLACE_ORDER_TEST=1` is set.

Copy `.env.example` to `.env` and fill in your wallet key plus any overrides
(`POLY_CHAIN_ID`, `POLY_API_URL`, `POLY_FUNDER`, `POLY_TEST_TOKEN`). Only the
private key is strictly required; the rest are optional fallbacks.

`examples/wss_market.rs` shows how to consume the public MARKET channel for
price/book updates. Set `POLY_WSS_MARKETS` and/or `POLY_WSS_ASSET_IDS` to the
condition/asset IDs you care about, then run:

```bash
cargo run --example wss_market
```

The example prints `book`, `price_change`, `tick_size_change`, and
`last_trade_price` events for the subscribed markets.

For authenticated events, `examples/wss_user.rs` shows how to derive an API key,
construct `WssUserClient`, and stream `WssUserEvent::Order`/`Trade` messages.
Run it via `cargo run --example wss_user` once `POLY_PRIVATE_KEY` is set. It
places a tiny limit order on a ≥1M-liquidity market (you can tweak
`POLY_WSS_MIN_LIQUIDITY`) and prints the resulting order ID, then waits for user
events so you can observe partial fills/cancellations. While that example
runs, start `POLY_WSS_ORDER_ID=… cargo run --example wss_cancel` from another
terminal to cancel the order and trigger a `WssUserEvent::Order` update.

If you already have API credentials, `WssUserClient` exposes the authenticated
user channel so you can react to your own orders and trades. Construct
`WssUserClient::new(api_creds.api_key.clone())`, subscribe to the markets you're
trading, and drive `next_event()` to consume `WssUserEvent::Order` and
`WssUserEvent::Trade` payloads that mirror the data shown above.

## Gamma and Data APIs

Use the `client` module to call Gamma endpoints such as `/markets`, `/events`,
`/tags`, and `NegRisk` data. Only the Gamma markets and single-order creation
paths have live regression coverage today; feel free to extend coverage to more
endpoints or submit fixes. The `types` module contains strongly typed responses
such as `Market`, `MarketOrderArgs`, and `OrderBookSummary`.

## Testing

Test order placement with this command (make sure env variables are set). This
exercise is what we currently rely on as basic coverage for the order APIs:
```bash
# WARNING: this test placese a tiny order on Polymarket ~0.006 USDC
RUN_PLACE_ORDER_TEST=1 cargo test place_order -- --nocapture
```

### Formatting and Lints

```
cargo fmt
cargo clippy
```


## Contributing

Contributions are welcome!
Look for any open issue in the [Issues](https://github.com/augustin-v/polysqueeze/issues/) page, or open a pull request after forking the project.
