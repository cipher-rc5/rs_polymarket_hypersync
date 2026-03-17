# rs_polymarket_hypersync Agent Rules

Project-level instructions for OpenCode-compatible LLM agents.

## Purpose

This repository is a Rust CLI that streams Polymarket onchain logs from HyperSync and optionally enriches with Polymarket HTTP/RTDS data.

## Critical workflow

1. Read `src/main.rs` first to understand runtime flow.
2. Use `src/contracts.rs` for addresses/topics (single source of truth).
3. Use `src/exchange.rs` for condition-aware exchange matching logic.
4. Use `src/enrich.rs` for offchain HTTP/WS enrichment behavior.
5. Use `src/storage.rs` for DuckDB/Parquet output behavior.

## Safety and boundaries

- Never edit `_dev/` reference docs; treat as read-only context.
- Do not hardcode secrets/tokens.
- Respect `DATA_DIR` for any storage output. Persisted files must stay under that directory.
- Keep defaults production-safe and low-noise.

## Coding conventions

- Prefer small, composable helper functions over deeply nested logic.
- Keep event topic/address constants in `src/contracts.rs`, not inline.
- Use `anyhow::Result` and add context to external I/O/network failures.
- Preserve current output style: concise status lines and matched-event lines.

## Performance and output

- Avoid noisy per-batch logs; print progress sparingly when no matches occur.
- Avoid unbounded historical scans in default UX paths.
- Keep `just run` fast and practical for recent windows.

## Verification checklist

After changes, run:

```bash
cargo fmt
cargo check
```

When stream behavior changes, run a bounded smoke test, for example:

```bash
FROM_BLOCK=84023890 TO_BLOCK_EXCL=84023910 FOLLOW_TAIL=false cargo run --quiet
```

## Documentation policy

- Update `README.md`, `.env.example`, and `justfile` when adding/changing env vars or run modes.
- Keep examples copy/paste ready.
