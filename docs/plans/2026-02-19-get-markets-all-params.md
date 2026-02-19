# Add All Query Parameters for get_markets Function

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add all missing query parameters to `GammaListParams` struct and update `get_markets` function to support the complete Polymarket Gamma API.

**Architecture:** Extend the existing `GammaListParams` struct with new fields per the [Polymarket API documentation](https://docs.polymarket.com/api-reference/markets/list-markets). Update both the struct definition and the `to_query_params()` method, then update `get_markets` in client.rs to pass the new parameters.

**Tech Stack:** Rust, serde for serialization, rust_decimal::Decimal for numeric values

**Reference:** Issue #12 - feat(gamma_api): Add every query parameters for get_markets function

---

## Missing Parameters (from API docs)

Current params: limit, offset, closed, tag_id, exclude_tag_id, related_tags, order, ascending, liquidity_num_min, end_date_max, start_date_min

New params to add:
- `id` - Vec<i64> - Filter by market IDs
- `slug` - Vec<String> - Filter by market slugs
- `clob_token_ids` - Vec<String> - Filter by CLOB token IDs
- `condition_ids` - Vec<String> - Filter by condition IDs
- `market_maker_address` - Vec<String> - Filter by market maker addresses
- `liquidity_num_max` - Decimal - Maximum liquidity
- `volume_num_min` - Decimal - Minimum volume
- `volume_num_max` - Decimal - Maximum volume
- `start_date_max` - DateTime<Utc> - Maximum start date
- `end_date_min` - DateTime<Utc> - Minimum end date
- `cyom` - bool - Create Your Own Market filter
- `uma_resolution_status` - String - UMA resolution status
- `game_id` - String - Game ID filter
- `sports_market_types` - Vec<String> - Sports market types
- `rewards_min_size` - Decimal - Minimum rewards size
- `question_ids` - Vec<String> - Filter by question IDs
- `include_tag` - bool - Include tag in response

---

## Task 1: Update GammaListParams Struct Definition

**Files:**
- Modify: `src/types.rs:1172-1184`

**Step 1: Add new fields to GammaListParams struct**

```rust
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
```

**Step 2: Verify compilation**

Run: `cargo check`
Expected: No errors related to struct fields

**Step 3: Commit**

```bash
git add src/types.rs
git commit -m "feat(types): add all missing query parameters to GammaListParams struct

Add 17 new fields per Polymarket Gamma API documentation:
- Array filters: id, slug, clob_token_ids, condition_ids, market_maker_address
- Numeric ranges: liquidity_num_max, volume_num_min, volume_num_max, rewards_min_size
- Date ranges: start_date_max, end_date_min
- Boolean flags: cyom, include_tag
- String filters: uma_resolution_status, game_id, sports_market_types, question_ids

Refs: #12"
```

---

## Task 2: Update to_query_params Method

**Files:**
- Modify: `src/types.rs:1191-1227`

**Step 1: Extend to_query_params to handle new parameters**

Replace the entire method. See the full implementation in the source file.

**Step 2: Verify compilation**

Run: `cargo check`
Expected: No errors

**Step 3: Run tests**

Run: `cargo test --lib`
Expected: All tests pass

**Step 4: Commit**

```bash
git add src/types.rs
git commit -m "feat(types): implement to_query_params for all GammaListParams fields

Add serialization for all 17 new query parameters:
- Boolean flags, String filters, Numeric, Date ranges, Arrays

Refs: #12"
```

---

## Task 3: Update get_markets Function

**Files:**
- Modify: `src/client.rs:1707-1799`

**Step 1: Review current get_markets implementation**

Read the current function at lines 1707-1800 to understand existing logic.

**Step 2: Update query building logic to include all parameters**

Replace the query building section (lines 1721-1769) with updated logic.

**Step 3: Verify compilation**

Run: `cargo check`
Expected: No errors

**Step 4: Commit**

```bash
git add src/client.rs
git commit -m "feat(client): add all query parameters to get_markets function

Update query building logic to support all 17 new parameters:
- Boolean flags, String filters, Numeric ranges, Date ranges, Array filters

Refs: #12"
```

---

## Task 4: Add Builder Methods (Optional Enhancement)

**Files:**
- Modify: `src/types.rs:1186-1228`

**Step 1: Add builder-style methods for new parameters**

Add after the `builder()` method.

**Step 2: Verify compilation**

Run: `cargo check`
Expected: No errors

**Step 3: Commit**

```bash
git add src/types.rs
git commit -m "feat(types): add builder methods for new GammaListParams fields

Add fluent builder API for all 17 new parameters.

Refs: #12"
```

---

## Task 5: Update Examples

**Files:**
- Modify: `examples/order.rs`
- Modify: `examples/wss_market.rs`
- Modify: `examples/wss_user.rs`

**Step 1: Add example usage of new parameters**

Update examples to demonstrate new parameters.

**Step 2: Verify examples compile**

Run: `cargo check --examples`
Expected: No errors

**Step 3: Commit**

```bash
git add examples/
git commit -m "docs(examples): demonstrate new GammaListParams usage

Add examples showing how to use new query parameters.

Refs: #12"
```

---

## Task 6: Update Tests

**Files:**
- Modify: `tests/live_gamma_client.rs`

**Step 1: Add test for new parameters**

Add a new test function after the existing one.

**Step 2: Run tests**

Run: `cargo test --test live_gamma_client -- --ignored`
Expected: Test passes (requires RUN_GAMMA_TESTS=1)

**Step 3: Commit**

```bash
git add tests/live_gamma_client.rs
git commit -m "test(gamma): add test for all query parameters

Add live test verifying get_markets works with all new parameters.

Refs: #12"
```

---

## Task 7: Update Documentation

**Files:**
- Modify: `README.md`

**Step 1: Update README to document new parameters**

Update the GammaListParams example with new parameters.

**Step 2: Commit**

```bash
git add README.md
git commit -m "docs(readme): document all new GammaListParams options

Update GammaListParams example with new parameters.

Refs: #12"
```

---

## Task 8: Final Verification and PR

**Step 1: Run full test suite**

```bash
cargo test --lib
cargo check --all-targets
cargo clippy -- -D warnings  # if available
```

Expected: All tests pass, no warnings

**Step 2: Create summary of changes**

```bash
git log --oneline --no-decorate HEAD~8..HEAD
```

**Step 3: Push branch and create PR**

```bash
# Push to remote
git push origin HEAD

# Create PR linking to issue #12
gh pr create --title "feat(gamma_api): add all query parameters for get_markets" --body "Closes #12

## Summary
Adds complete support for all Polymarket Gamma API query parameters in the get_markets function.

## Changes
- Extended GammaListParams struct with 17 new fields
- Updated to_query_params() to serialize all parameters
- Updated get_markets() to pass all parameters to the API
- Added builder methods for fluent API
- Updated examples and documentation
- Added live test for new parameters

## New Parameters
- Array filters: id, slug, clob_token_ids, condition_ids, market_maker_address, sports_market_types, question_ids
- Numeric ranges: liquidity_num_max, volume_num_min, volume_num_max, rewards_min_size
- Date ranges: start_date_max, end_date_min
- Boolean flags: cyom, include_tag
- String filters: uma_resolution_status, game_id

## Testing
- [x] Unit tests pass
- [x] Compilation successful
- [x] Examples compile
- [x] Live test added (requires RUN_GAMMA_TESTS=1)"
```

**Step 4: Verify PR created successfully**

Expected: PR created linking to issue #12

---

## Completion Checklist

- [ ] Task 1: GammaListParams struct updated with 17 new fields
- [ ] Task 2: to_query_params method updated to serialize all fields
- [ ] Task 3: get_markets function updated to use all parameters
- [ ] Task 4: Builder methods added for fluent API
- [ ] Task 5: Examples updated to demonstrate usage
- [ ] Task 6: Tests added for new parameters
- [ ] Task 7: README documentation updated
- [ ] Task 8: PR created linking to issue #12

**Total Tasks:** 8
**Estimated Time:** 45-60 minutes
