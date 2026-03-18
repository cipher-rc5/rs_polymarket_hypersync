set dotenv-load := true

default:
  @just --list

# Compile-check the project.
check:
  cargo check

# Strict local quality gate.
check-strict:
  cargo fmt -- --check
  cargo check
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test

# Quickstart run for recent data (best default for day-to-day use).
run: run-recent

# Full run using values from .env/.env.example.
run-full:
  cargo run --quiet

# Run recent window with HTTP + RTDS enrichment enabled.
run-enriched lookback="20000":
  ENABLE_HTTP_ENRICHMENT=true ENABLE_RTDS_WEBSOCKET=true just run-recent {{lookback}}

# Run a recent bounded window.
# Tries to compute `from` as `head - lookback` using POLYGON_RPC_URL (or polygon-rpc.com),
# and falls back to app defaults (auto condition + last ~24h) if RPC probing fails.
run-recent lookback="20000":
  #!/usr/bin/env bash
  set -euo pipefail
  RPC_URL="${POLYGON_RPC_URL:-https://polygon-rpc.com}"
  HEAD="$(cast block-number --rpc-url "$RPC_URL" 2>/dev/null || true)"
  if [[ -n "$HEAD" ]]; then
    FROM="$((HEAD - lookback))"
    if [[ "$FROM" -lt 1 ]]; then FROM=1; fi
    echo "Running recent window: FROM_BLOCK=$FROM TO_BLOCK_EXCL=$((HEAD + 1))"
    FROM_BLOCK="$FROM" TO_BLOCK_EXCL="$((HEAD + 1))" FOLLOW_TAIL=false cargo run --quiet
  else
    echo "Could not query head block from RPC; falling back to app defaults"
    FOLLOW_TAIL=false cargo run --quiet
  fi

# Historical bounded scan.
run-range from to:
  FROM_BLOCK={{from}} TO_BLOCK_EXCL={{to}} FOLLOW_TAIL=false cargo run --quiet

# Tail live chain head from a given block.
run-tail from:
  FROM_BLOCK={{from}} FOLLOW_TAIL=true cargo run --quiet

# Condition-focused scan with explicit controls.
run-condition condition from to include_exchange="true" include_nra="true" include_fills="true" include_matches="true":
  CONDITION_ID={{condition}} FROM_BLOCK={{from}} TO_BLOCK_EXCL={{to}} FOLLOW_TAIL=false INCLUDE_EXCHANGE_LOGS={{include_exchange}} INCLUDE_NEG_RISK_LOGS={{include_nra}} INCLUDE_ORDER_FILLED={{include_fills}} INCLUDE_ORDERS_MATCHED={{include_matches}} cargo run --quiet

# Same as run-condition, with optional pre-seeded token IDs.
# token_ids format: 0xabc...,0xdef...
run-condition-seeded condition from to token_ids include_exchange="true" include_nra="true" include_fills="true" include_matches="true":
  CONDITION_ID={{condition}} FROM_BLOCK={{from}} TO_BLOCK_EXCL={{to}} FOLLOW_TAIL=false MARKET_TOKEN_IDS={{token_ids}} INCLUDE_EXCHANGE_LOGS={{include_exchange}} INCLUDE_NEG_RISK_LOGS={{include_nra}} INCLUDE_ORDER_FILLED={{include_fills}} INCLUDE_ORDERS_MATCHED={{include_matches}} cargo run --quiet
