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

- Rust toolchain
- `ENVIO_API_TOKEN` (required)

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

## Environment variables

Core:

- `ENVIO_API_TOKEN` (required)
- `ENVIO_HYPERSYNC_URL` (optional; defaults to Polygon HyperSync)
- `CONDITION_ID` (optional; defaults to value in `src/main.rs`)
- `FROM_BLOCK` (optional; default in `src/main.rs`)
- `TO_BLOCK_EXCL` (optional; if unset and `FOLLOW_TAIL=false`, app uses `height + 1`)
- `FOLLOW_TAIL` (`true`/`false`, default `false`)

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

Persistence / export:

- `EXPORT_DUCKDB_PATH` (optional path to DuckDB file)
- `EXPORT_PARQUET_PATH` (optional path for Parquet export at end of run)
- `DATA_DIR` (optional base directory for storage output, default `./data`)

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
  - Verify `CONDITION_ID` is correct.
  - Confirm exchange logs are enabled (`INCLUDE_EXCHANGE_LOGS=true`).
  - If no `TokenRegistered` in your window, seed `MARKET_TOKEN_IDS`.
- Too much output
  - Set a tighter `TO_BLOCK_EXCL`.
  - Disable fills/matches with `INCLUDE_ORDER_FILLED=false` / `INCLUDE_ORDERS_MATCHED=false`.

## Development

```bash
cargo check
cargo fmt
```

This repository supports optional enrichment via Polymarket HTTP/RTDS, plus optional DuckDB + Parquet output for downstream analytics.
