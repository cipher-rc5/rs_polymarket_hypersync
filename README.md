# rs_polymarket_hypersync

Rust CLI for querying Polymarket Polygon logs through HyperSync, focused on a single `conditionId` across:

- `ConditionalTokens` (CTF)
- `NegRiskAdapter`
- `Exchange` + `NegRiskExchange`

It streams logs, applies condition-aware filtering, and prints a compact event feed plus summary counts.

## What this indexes

The current binary tracks these Polygon contracts from `_dev/polymarket-indexer.md`:

- `ConditionalTokens`: `0x4D97DCd97eC945f40cF65F87097ACe5EA0476045`
- `NegRiskAdapter`: `0xd91E80cF2E7be2e162c6513ceD06f1dD0dA35296`
- `Exchange`: `0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E`
- `NegRiskExchange`: `0xC5d563A36AE78145C45a50134d48A1215220f80a`

Default HyperSync endpoint: `https://137.hypersync.xyz`

## Project layout

- `src/main.rs`: app entrypoint, query construction, stream loop, log routing
- `src/contracts.rs`: chain id, endpoint, contract addresses, event topics
- `src/exchange.rs`: exchange token tracking + `OrderFilled`/`OrdersMatched` condition matching
- `src/enrich.rs`: optional HTTP + RTDS websocket enrichment (fastwebsockets)
- `src/storage.rs`: optional DuckDB persistence and Parquet export
- `abis/`: ABI JSON files for reference (not runtime-decoded in this CLI)
- `justfile`: runnable presets for common workflows

## Requirements

- Rust toolchain `1.93` (edition `2024`)
- `ENVIO_API_TOKEN` (required)

## Agent compliance checklist

For contributors using coding agents (or reviewing agent-generated changes), keep the following aligned with repository policy:

- Use strictly idiomatic Rust.
- Target Rust edition `2024`.
- Target Rust toolchain `1.93` (also enforced in `Cargo.toml` via `rust-version`).
- Refer to the project specification sources before implementing behavior-sensitive changes:
  - `https://docs.polymarket.com/llms.txt`
  - `https://docs.polymarket.com/market-data/websocket/rtds.md`
  - `https://duckdb.org/docs/stable/clients/rust`
  - `https://docs.rs/parquet/latest/parquet`
  - `https://github.com/denoland/fastwebsockets`
  - `https://rust-unofficial.github.io/patterns/idioms`

Create `.env` (or use shell env):

```bash
ENVIO_API_TOKEN=your_token
ENVIO_HYPERSYNC_URL=https://137.hypersync.xyz
```

## Run quickly

```bash
cargo run --quiet
```

or with `just`:

```bash
just run
```

`just run` defaults to a recent bounded window so you see real data quickly.

## CI quality gates

This repository enforces Rust quality checks in GitHub Actions (`.github/workflows/ci.yml`):

- `cargo fmt -- --check`
- `cargo check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

Run the same checks locally with:

```bash
just check-strict
```

## Environment variables

Core:

- `ENVIO_API_TOKEN` (required)
- `ENVIO_HYPERSYNC_URL` (optional; defaults to Polygon HyperSync)
- `CONDITION_ID` (optional; if unset, app auto-selects a recent active market from the last 24h)
- `AUTO_CONDITION_LOOKBACK_HOURS` (default `24`; window used when auto-selecting `CONDITION_ID`)
- `FROM_BLOCK` (optional; if unset, defaults to approximately last 24h from current chain height)
- `TO_BLOCK_EXCL` (optional; if unset and `FOLLOW_TAIL=false`, app uses `height + 1`)
- `FOLLOW_TAIL` (`true`/`false`, default `false`)

Runtime resilience / flow control:

- `IO_TIMEOUT_MS` (default `15000`)
- `STREAM_RETRY_MAX_ATTEMPTS` (default `6`)
- `STREAM_RETRY_BASE_DELAY_MS` (default `500`)
- `STREAM_RETRY_MAX_DELAY_MS` (default `10000`)
- `PROGRESS_LOG_EVERY_BATCHES` (default `50`)

Filtering toggles:

- `INCLUDE_NEG_RISK_LOGS` (`true`/`false`, default `true`)
- `INCLUDE_EXCHANGE_LOGS` (`true`/`false`, default `true`)
- `INCLUDE_CLOB_LOGS` backward-compatible alias for `INCLUDE_EXCHANGE_LOGS`
- `INCLUDE_ORDER_FILLED` (`true`/`false`, default `true`)
- `INCLUDE_ORDERS_MATCHED` (`true`/`false`, default `true`)

Exchange token seeding:

- `MARKET_TOKEN_IDS` (optional CSV of hex token IDs, e.g. `0xabc...,0xdef...`)

Why token seeding exists:

- `OrderFilled`/`OrdersMatched` do not include `conditionId` in indexed topics.
- The app links fills/matches to your condition by tracking token IDs from `TokenRegistered` for that condition.
- If your `FROM_BLOCK` starts after token registration occurred, set `MARKET_TOKEN_IDS` so fills/matches can still match.

Offchain enrichment:

- `ENABLE_HTTP_ENRICHMENT` (`true`/`false`, default `true`)
- `POLY_GAMMA_BASE_URL` (default `https://gamma-api.polymarket.com`)
- `POLY_CLOB_BASE_URL` (default `https://clob.polymarket.com`)
- `ENABLE_RTDS_WEBSOCKET` (`true`/`false`, default `false`)
- `POLY_RTDS_URL` (default `wss://ws-live-data.polymarket.com`)
- `RTDS_FILTERS` (default `btcusdt,ethusdt,solusdt,xrpusdt`)
- `RTDS_PRINT_UPDATES` (`true`/`false`, default `false`)
- `ENABLE_RTDS_STRICT_TLS` (`true`/`false`, default `true`)
- `RTDS_LOG_TLS_DETAILS` (`true`/`false`, default `true`)
- `RTDS_CERT_SHA256_ALLOWLIST` (optional CSV of SHA-256 fingerprints; fail-closed when set)
- `HTTP_TIMEOUT_MS` (default `4000`)
- `HTTP_RETRY_MAX_ATTEMPTS` (default `3`)
- `HTTP_RETRY_BASE_DELAY_MS` (default `150`)
- `HTTP_RETRY_MAX_DELAY_MS` (default `2000`)
- `RTDS_RETRY_MAX_ATTEMPTS` (default `50`)
- `RTDS_RETRY_BASE_DELAY_MS` (default `500`)
- `RTDS_RETRY_MAX_DELAY_MS` (default `10000`)
- `ENRICHMENT_MAX_IN_FLIGHT` (default `16`; bounded prefetch concurrency)

Persistence / export:

- `EXPORT_DUCKDB_PATH` (optional path to DuckDB file)
- `EXPORT_PARQUET_PATH` (optional path for Parquet export at end of run)
- `DATA_DIR` (optional base directory for storage output, default `./data`)
- `STORAGE_BATCH_SIZE` (default `200`; buffered insert batch size)

Storage path behavior:

- When persistence is enabled, the app always writes under `DATA_DIR`.
- For `EXPORT_DUCKDB_PATH` and `EXPORT_PARQUET_PATH`, only the filename component is used.
- Example: `EXPORT_PARQUET_PATH=/tmp/custom/output.parquet` writes to `./data/output.parquet` (or `<DATA_DIR>/output.parquet`).

## Condition matching rules

### CTF (`ConditionalTokens`)

- `ConditionResolution`: compare `conditionId` in `topic1`
- `PositionSplit`/`PositionsMerge`: compare `conditionId` in `topic3`
- `PayoutRedemption`: compare first ABI word in `data` (non-indexed `conditionId`)

### NegRiskAdapter

- `PositionSplit`/`PositionsMerge`/`PayoutRedemption`: compare `conditionId` in `topic2`

### Exchange + NegRiskExchange

- `TokenRegistered`: compare `conditionId` in `topic3`, then store `token0`/`token1`
- `OrderFilled`/`OrdersMatched`: decode `makerAssetId` and `takerAssetId` from `data`; match if either token is tracked

## Just commands

List commands:

```bash
just
```

Common recipes:

- `just check`
- `just run`
- `just run-recent`
- `just run-recent 50000`
- `just run-full`
- `just run-enriched`
- `just run-range <from> <to>`
- `just run-tail <from>`
- `just run-condition <condition> <from> <to>`
- `just run-condition-seeded <condition> <from> <to> <token_ids_csv>`

Examples:

```bash
just run
just run-recent 50000
just run-full
just run-enriched
just run-range 84000000 84001000
just run-tail 84300000
just run-condition 0x7b49294de4f325f82b071631ed8222ac5bba5ce95948018aff5a3c2ef6c5e595 84000000 84300000
just run-condition-seeded 0x7b49294de4f325f82b071631ed8222ac5bba5ce95948018aff5a3c2ef6c5e595 84000000 84300000 0x1234,0xabcd
```

## Troubleshooting

- `api_token is required`
  - Ensure `.env` exists and has `ENVIO_API_TOKEN`, or export it in shell.
- No logs returned
  - Expand block range.
  - Verify `CONDITION_ID` is correct (or set it explicitly instead of auto-selection).
  - Confirm exchange logs are enabled (`INCLUDE_EXCHANGE_LOGS=true`).
  - If no `TokenRegistered` in your window, seed `MARKET_TOKEN_IDS`.
- Too much output
  - Set a tighter `TO_BLOCK_EXCL`.
  - Disable fills/matches with `INCLUDE_ORDER_FILLED=false` / `INCLUDE_ORDERS_MATCHED=false`.

### Offchain past-results query fails with missing params

- Use `curl -G` with `--data-urlencode` to avoid malformed query strings:

```bash
curl -G "https://polymarket.com/api/past-results" \
  --data-urlencode "symbol=BTC" \
  --data-urlencode "variant=fiveminute" \
  --data-urlencode "assetType=crypto" \
  --data-urlencode "currentEventStartTime=2026-03-17T23:35:00Z" \
  --data-urlencode "count=10"
```

- If you see `Missing required parameters`, re-check for URL typos in `currentEventStartTime` and separators.

### Onchain lookup is slow or shows no matches

- For 5-minute markets, `OrderFilled`/`OrdersMatched` matching depends on tracked token IDs.
- If your range starts after `TokenRegistered`, seed token IDs from `clobTokenIds`:

```bash
just run-condition-seeded \
  0x97d530e766a3a3d37fe5c05bb7fdad433ff1d3920026314c721155a1f87cc27d \
  84000000 \
  84360000 \
  0x7a6848b6d4185e1ef98761399584c076d6568c6871058dff8ebd6c5037d7c33,0x25e0f65ad5de689a3a6fe619ad72b0c3e6fdf6be9f0654aeff220d22acc91704
```

- If progress still advances with no matches, tighten the window around the market lifecycle (creation, trading, and settlement blocks).

## Operator runbook

- Stream repeatedly reconnecting
  - Check `ENVIO_HYPERSYNC_URL`, API token validity, and network reachability.
  - Increase `IO_TIMEOUT_MS` for slow networks.
  - Tune stream retry/backoff with `STREAM_RETRY_*` env vars.
- RTDS reconnect storms
  - Verify `POLY_RTDS_URL` and TLS settings.
  - Keep `ENABLE_RTDS_STRICT_TLS=true` unless debugging in a controlled environment.
  - If pinning is enabled, confirm `RTDS_CERT_SHA256_ALLOWLIST` fingerprints are current.
- Enrichment lag under load
  - Increase `ENRICHMENT_MAX_IN_FLIGHT` cautiously.
  - Reduce retry aggressiveness via `HTTP_RETRY_*` and check endpoint health.
- Storage bottlenecks
  - Increase `STORAGE_BATCH_SIZE` to reduce insert overhead.
  - Ensure `DATA_DIR` is on a disk with sufficient write throughput.
- Graceful shutdown
  - Press `Ctrl+C`; the process finalizes buffered storage and exits.

## Development

```bash
cargo check
cargo fmt
```

This repository supports optional enrichment via Polymarket HTTP/RTDS, plus optional DuckDB + Parquet output for downstream analytics.
