# Contributing to polysqueeze

Thanks for contributing! This project is a Rust SDK for Polymarket APIs, and we welcome bug fixes, docs improvements, and new endpoint coverage.

## Quick links

- Bugs / feature requests: [Issues](https://github.com/augustin-v/polysqueeze/issues)
- PRs: [Pull Requests](https://github.com/augustin-v/polysqueeze/pulls)

## Development setup

### Prerequisites

- Rust (stable toolchain)
- `git`

### Get the code

1. Fork the repo and clone your fork.
2. Create a branch for your change:

```bash
git checkout -b feat/short-description
```

## Build, test, and verify

### Run tests (fast, offline)

```bash
cargo test --all
```

CI runs `cargo fmt --all -- --check` and `cargo test --all`.

### Formatting

```bash
cargo fmt --all
```

### Lints (recommended)

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

## Live/integration tests (uses real credentials)

Some tests and examples can hit Polymarket services and may place real orders. These are opt-in via environment variables.

1. Copy the example env file:

```bash
cp .env.example .env
```

2. Fill in values in `.env` (never commit this file; it is git-ignored).

3. Run the live order placement test (WARNING: places a tiny real order when enabled):

```bash
RUN_PLACE_ORDER_TEST=1 cargo test place_order -- --nocapture
```

If a test is skipped, check the test output for which env vars are required (for example `POLY_PRIVATE_KEY`, `POLY_FUNDER`, and/or API creds).

## Running examples

```bash
cargo run --example order
cargo run --example wss_market
```

Examples use the same environment variables described in `.env.example`.

## What makes a PR easy to review

- Keep changes focused (one fix/feature per PR where possible).
- Add or update tests when changing behavior.
- Update docs/comments when adding new public API surface.
- Ensure `cargo fmt --all` and `cargo test --all` pass locally.
- Avoid committing secrets (private keys, API keys, `.env`, etc.).

## Reporting bugs

When filing an issue, please include:

- What you expected to happen vs. what happened
- Repro steps or a small code snippet
- Your Rust version (`rustc --version`)
- Any relevant logs (redact secrets)

## Security

If you believe you have found a security issue, please avoid filing a public issue. Prefer using GitHub’s private security advisory / reporting flow for the repository.

## License

By contributing, you agree that your contributions will be licensed under the project’s license terms (see `Cargo.toml` and `LICENSE`).
