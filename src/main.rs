use anyhow::{Context, Result};
use hypersync_client::{
    Client, StreamConfig,
    net_types::{LogField, LogFilter, Query},
};
use tokio::time::{Duration, timeout};
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, fmt};

mod config;
mod contracts;
mod enrich;
mod exchange;
mod matcher;
mod storage;

use config::{AppConfig, DEFAULT_BLOCKS_PER_DAY, DEFAULT_FROM_BLOCK, RetryPolicy};
use contracts::{
    DEFAULT_POLYGON_HYPERSYNC_URL, POLYGON_CHAIN_ID,
    address::{CONDITIONAL_TOKENS, EXCHANGE, NEG_RISK_ADAPTER, NEG_RISK_EXCHANGE},
    topic::{
        CONDITION_RESOLUTION, NEG_RISK_PAYOUT_REDEMPTION, NEG_RISK_POSITION_SPLIT,
        NEG_RISK_POSITIONS_MERGE, ORDER_FILLED, ORDERS_MATCHED, PAYOUT_REDEMPTION, POSITION_SPLIT,
        POSITIONS_MERGE, TOKEN_REGISTERED,
    },
};
use enrich::{OffchainConfig, OffchainEnricher};
use exchange::{
    ExchangeTracker, decode_first_two_asset_ids_decimal, normalize_condition_id_word,
    normalize_topic_word, parse_seed_env,
};
use matcher::{CtfTopicMatchers, ctf_matches_condition, normalize_hex, topic_contains_hex};
use storage::{EventStore, StoredEvent};

struct RunStats {
    total_ctf: usize,
    total_neg_risk: usize,
    total_exchange: usize,
    batches_without_matches: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let _ = dotenvy::dotenv();

    let cfg = AppConfig::from_env()?;
    let offchain_cfg = OffchainConfig::from_env();
    let offchain = OffchainEnricher::new(offchain_cfg.clone());
    let _rtds_listener = offchain.spawn_rtds_listener();

    let seed_token_ids = parse_seed_env()?;
    let mut exchange_tracker = ExchangeTracker::from_seed_csv(seed_token_ids.as_deref())?;
    let mut event_store = EventStore::from_env()?;

    let mut client_builder = Client::builder().api_token(cfg.api_token.clone());
    if let Some(url) = &cfg.hypersync_url {
        client_builder = client_builder.url(url);
    } else {
        client_builder = client_builder.chain_id(POLYGON_CHAIN_ID);
    }

    let client = client_builder
        .build()
        .context("failed to create HyperSync client")?;

    let condition_id = if let Some(v) = cfg.condition_id.clone() {
        v
    } else {
        let recent = offchain
            .fetch_recent_market(cfg.auto_condition_lookback_hours)
            .await?
            .context(
                "failed to auto-select condition id from recent markets; set CONDITION_ID explicitly",
            )?;
        info!(
            lookback_hours = cfg.auto_condition_lookback_hours,
            auto_condition_id = %recent.condition_id,
            auto_market_slug = %recent.slug,
            auto_market_start_date = %recent.start_date,
            "auto-selected market from recent window"
        );
        recent.condition_id
    };

    if let Some(meta) = offchain.fetch_market_by_condition(&condition_id).await? {
        info!(market_question = %meta.question, "market metadata");
        info!(market_slug = %meta.slug, "market metadata");
        if !meta.outcomes.is_empty() {
            info!(market_outcomes = %meta.outcomes, "market metadata");
        }
    }

    let (from_block, to_block_excl) = resolve_block_bounds(&cfg, &client).await?;
    let effective_hypersync_url = cfg.effective_hypersync_url(DEFAULT_POLYGON_HYPERSYNC_URL);
    let effective_polygon_rpc = cfg.effective_polygon_rpc(DEFAULT_POLYGON_HYPERSYNC_URL);

    info!("querying Polygon via HyperSync");
    info!(condition_id = %condition_id, "runtime");
    info!(ctf_contract = %CONDITIONAL_TOKENS, "runtime");
    info!(neg_risk = %NEG_RISK_ADAPTER, "runtime");
    info!(exchange = %EXCHANGE, "runtime");
    info!(neg_exchange = %NEG_RISK_EXCHANGE, "runtime");
    info!(from_block, "runtime");
    info!(hypersync_url = %effective_hypersync_url, "runtime");
    info!(polygon_rpc = %effective_polygon_rpc, "runtime");
    info!(include_exchange_logs = cfg.include_exchange_logs, "runtime");
    info!(include_order_filled = cfg.include_order_filled, "runtime");
    info!(
        include_orders_matched = cfg.include_orders_matched,
        "runtime"
    );
    info!(include_neg_risk_logs = cfg.include_neg_risk_logs, "runtime");
    info!(
        enable_http_enrichment = offchain.config().enable_http,
        "runtime"
    );
    info!(
        enable_rtds_websocket = offchain.config().enable_rtds,
        "runtime"
    );
    info!(
        rtds_strict_tls = offchain.config().rtds_strict_tls,
        "runtime"
    );
    info!(
        rtds_tls_logging = offchain.config().rtds_log_tls_details,
        "runtime"
    );
    info!(
        rtds_tls_pins = offchain.config().rtds_cert_sha256_allowlist.len(),
        "runtime"
    );
    info!(follow_tail = cfg.follow_tail, "runtime");
    if let Some(v) = to_block_excl {
        info!(to_block_excl = v, "runtime");
    }
    info!(
        seed_tokens = exchange_tracker.tracked_tokens_len(),
        "runtime"
    );
    info!(duckdb_export_enabled = event_store.is_some(), "runtime");
    if let Some(store) = &event_store {
        info!(duckdb_path = %store.duckdb_path(), "runtime");
        if let Some(path) = store.parquet_path() {
            info!(parquet_path = %path, "runtime");
        }
    }

    let topic_matchers = CtfTopicMatchers {
        condition_resolution_topic: normalize_hex(CONDITION_RESOLUTION),
        position_split_topic: normalize_hex(POSITION_SPLIT),
        positions_merge_topic: normalize_hex(POSITIONS_MERGE),
        payout_redemption_topic: normalize_hex(PAYOUT_REDEMPTION),
    };

    let token_registered_topic = normalize_hex(TOKEN_REGISTERED);
    let order_filled_topic = normalize_hex(ORDER_FILLED);
    let orders_matched_topic = normalize_hex(ORDERS_MATCHED);
    let ctf_contract_hex = normalize_hex(CONDITIONAL_TOKENS);
    let neg_risk_adapter_hex = normalize_hex(NEG_RISK_ADAPTER);
    let exchange_hex = normalize_hex(EXCHANGE);
    let neg_risk_exchange_hex = normalize_hex(NEG_RISK_EXCHANGE);
    let condition_id_hex = condition_id
        .strip_prefix("0x")
        .unwrap_or(condition_id.as_str())
        .to_ascii_lowercase();
    let condition_id_word = normalize_condition_id_word(&condition_id)?;

    let mut next_from_block = from_block;
    let mut stats = RunStats {
        total_ctf: 0,
        total_neg_risk: 0,
        total_exchange: 0,
        batches_without_matches: 0,
    };
    let mut stream_attempt = 0u32;
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    'stream_outer: loop {
        let query = build_query(next_from_block, to_block_excl, &cfg)?;

        let receiver = timeout(
            Duration::from_millis(cfg.io_timeout_ms),
            client.stream(query, StreamConfig::default()),
        )
        .await
        .context("timed out starting HyperSync stream")?
        .context("failed to start stream")?;

        info!(from_block = next_from_block, "streaming logs");

        let mut receiver = receiver;
        loop {
            tokio::select! {
                _ = &mut shutdown => {
                    info!("received shutdown signal; finalizing");
                    break 'stream_outer;
                }
                maybe_response = receiver.recv() => {
                    match maybe_response {
                        Some(Ok(response)) => {
                            stream_attempt = 0;
                            next_from_block = response.next_block;
                            let mut matched_in_batch = 0usize;

                            for logs in &response.data.logs {
                                for log in logs {
                                    let address = log
                                        .address
                                        .as_ref()
                                        .map(std::string::ToString::to_string)
                                        .unwrap_or_default();

                                    let topic0 = log
                                        .topics
                                        .first()
                                        .and_then(|t| t.as_ref())
                                        .map(std::string::ToString::to_string)
                                        .unwrap_or_default();

                                    let topic2 = log
                                        .topics
                                        .get(2)
                                        .and_then(|t| t.as_ref())
                                        .map(std::string::ToString::to_string)
                                        .unwrap_or_default();

                                    let topic1 = log
                                        .topics
                                        .get(1)
                                        .and_then(|t| t.as_ref())
                                        .map(std::string::ToString::to_string)
                                        .unwrap_or_default();

                                    let topic3 = log
                                        .topics
                                        .get(3)
                                        .and_then(|t| t.as_ref())
                                        .map(std::string::ToString::to_string)
                                        .unwrap_or_default();

                                    let tx_hash = log
                                        .transaction_hash
                                        .as_ref()
                                        .map(std::string::ToString::to_string)
                                        .unwrap_or_default();

                                    let data = log.data.as_ref().map(|v| v.as_ref()).unwrap_or_default();
                                    let block = log.block_number.map(u64::from).unwrap_or_default();
                                    let log_idx = log.log_index.map(u64::from).unwrap_or_default();
                                    let address_hex = normalize_hex(&address);
                                    let mut offchain_hint = String::new();

                                    let is_ctf = address_hex == ctf_contract_hex;
                                    let is_neg_risk = address_hex == neg_risk_adapter_hex;
                                    let is_exchange =
                                        address_hex == exchange_hex || address_hex == neg_risk_exchange_hex;

                                    let source = if is_ctf {
                                        if !ctf_matches_condition(
                                            &topic0,
                                            &topic1,
                                            &topic3,
                                            data,
                                            &condition_id_word,
                                            &topic_matchers,
                                        ) {
                                            continue;
                                        }
                                        stats.total_ctf = stats.total_ctf.saturating_add(1);
                                        matched_in_batch = matched_in_batch.saturating_add(1);
                                        "CTF "
                                    } else if is_neg_risk {
                                        if !topic_contains_hex(&topic2, &condition_id_hex) {
                                            continue;
                                        }
                                        stats.total_neg_risk = stats.total_neg_risk.saturating_add(1);
                                        matched_in_batch = matched_in_batch.saturating_add(1);
                                        "NRA "
                                    } else if is_exchange {
                                        let topic0_hex = normalize_hex(&topic0);

                                        if topic0_hex == token_registered_topic {
                                            if normalize_topic_word(&topic3) != condition_id_word {
                                                continue;
                                            }
                                            exchange_tracker.register_token_pair(&topic1, &topic2);

                                            append_registered_ids_hint(&mut offchain_hint, &topic1, &topic2);

                                            if offchain.config().enable_http {
                                                enrich_token_pair_hint(&mut offchain_hint, &offchain, &topic1, &topic2)
                                                    .await;
                                            }
                                        } else if topic0_hex == order_filled_topic {
                                            if !cfg.include_order_filled
                                                || !exchange_tracker.matches_order_filled(data)
                                            {
                                                continue;
                                            }
                                            append_asset_ids_hint(&mut offchain_hint, data);
                                        } else if topic0_hex == orders_matched_topic {
                                            if !cfg.include_orders_matched
                                                || !exchange_tracker.matches_orders_matched(data)
                                            {
                                                continue;
                                            }
                                            append_asset_ids_hint(&mut offchain_hint, data);
                                        } else {
                                            continue;
                                        }

                                        stats.total_exchange = stats.total_exchange.saturating_add(1);
                                        matched_in_batch = matched_in_batch.saturating_add(1);
                                        if address_hex == exchange_hex {
                                            "EXCH"
                                        } else {
                                            "NREX"
                                        }
                                    } else {
                                        continue;
                                    };

                                    let tx_short = if tx_hash.len() > 12 {
                                        &tx_hash[..12]
                                    } else {
                                        &tx_hash
                                    };

                                    info!(
                                        source,
                                        block,
                                        log_index = log_idx,
                                        tx = %tx_short,
                                        topic0 = %topic0[..12.min(topic0.len())],
                                        topic2 = %topic2,
                                        tracked_tokens = exchange_tracker.tracked_tokens_len(),
                                        offchain_hint = %offchain_hint,
                                        "matched"
                                    );

                                    if let Some(store) = &mut event_store {
                                        store.insert_event(&StoredEvent {
                                            source,
                                            block_number: block,
                                            log_index: log_idx,
                                            tx_hash: &tx_hash,
                                            address: &address,
                                            topic0: &topic0,
                                            topic1: &topic1,
                                            topic2: &topic2,
                                            topic3: &topic3,
                                            tracked_tokens: exchange_tracker.tracked_tokens_len(),
                                            offchain_hint: &offchain_hint,
                                        })?;
                                    }
                                }
                            }

                            if matched_in_batch == 0 {
                                stats.batches_without_matches = stats.batches_without_matches.saturating_add(1);
                                if stats.batches_without_matches.is_multiple_of(cfg.progress_log_every_batches) {
                                    info!(next_block = response.next_block, "progress without matches");
                                }
                            } else {
                                stats.batches_without_matches = 0;
                            }
                        }
                        Some(Err(err)) => {
                            stream_attempt = stream_attempt.saturating_add(1);
                            if stream_attempt >= cfg.stream_retry.max_attempts {
                                return Err(err).context("stream retries exhausted");
                            }

                            let delay_ms = backoff_delay_ms(&cfg.stream_retry, stream_attempt);
                            warn!(
                                attempt = stream_attempt,
                                delay_ms,
                                next_from_block,
                                error = %err,
                                "stream error; reconnecting"
                            );
                            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                            continue 'stream_outer;
                        }
                        None => {
                            info!(next_from_block, "stream finished");
                            break 'stream_outer;
                        }
                    }
                }
            }
        }
    }

    if let Some(store) = event_store {
        store.finalize()?;
    }

    info!("done");
    info!(ctf_logs = stats.total_ctf, "summary");
    info!(nra_logs = stats.total_neg_risk, "summary");
    info!(exchange_logs = stats.total_exchange, "summary");

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).with_target(false).try_init();
}

async fn resolve_block_bounds(cfg: &AppConfig, client: &Client) -> Result<(u64, Option<u64>)> {
    let needs_height =
        cfg.from_block.is_none() || (!cfg.follow_tail && cfg.to_block_excl.is_none());

    let height = if needs_height {
        Some(
            timeout(
                Duration::from_millis(cfg.io_timeout_ms),
                client.get_height(),
            )
            .await
            .context("timed out fetching chain height")?
            .context("failed to fetch chain height")?,
        )
    } else {
        None
    };

    let from_block = cfg.from_block.unwrap_or_else(|| {
        height
            .unwrap_or(DEFAULT_FROM_BLOCK)
            .saturating_sub(DEFAULT_BLOCKS_PER_DAY)
    });

    let to_block_excl = if cfg.follow_tail {
        cfg.to_block_excl
    } else {
        Some(
            cfg.to_block_excl
                .unwrap_or_else(|| height.unwrap_or(from_block) + 1),
        )
    };

    Ok((from_block, to_block_excl))
}

fn build_query(from_block: u64, to_block_excl: Option<u64>, cfg: &AppConfig) -> Result<Query> {
    let ctf_filter = LogFilter::all()
        .and_address([CONDITIONAL_TOKENS])?
        .and_topic0([
            CONDITION_RESOLUTION,
            POSITION_SPLIT,
            POSITIONS_MERGE,
            PAYOUT_REDEMPTION,
        ])?;

    let neg_risk_filter = LogFilter::all()
        .and_address([NEG_RISK_ADAPTER])?
        .and_topic0([
            NEG_RISK_POSITION_SPLIT,
            NEG_RISK_POSITIONS_MERGE,
            NEG_RISK_PAYOUT_REDEMPTION,
        ])?;

    let mut exchange_topics = vec![TOKEN_REGISTERED];
    if cfg.include_order_filled {
        exchange_topics.push(ORDER_FILLED);
    }
    if cfg.include_orders_matched {
        exchange_topics.push(ORDERS_MATCHED);
    }

    let exchange_filter = LogFilter::all()
        .and_address([EXCHANGE, NEG_RISK_EXCHANGE])?
        .and_topic0(exchange_topics)?;

    let log_fields = [
        LogField::BlockNumber,
        LogField::TransactionHash,
        LogField::LogIndex,
        LogField::Address,
        LogField::Topic0,
        LogField::Topic1,
        LogField::Topic2,
        LogField::Topic3,
        LogField::Data,
    ];

    let mut query = Query::new().from_block(from_block).where_logs(ctf_filter);
    if cfg.include_neg_risk_logs {
        query = query.where_logs(neg_risk_filter);
    }
    if cfg.include_exchange_logs {
        query = query.where_logs(exchange_filter);
    }
    if let Some(v) = to_block_excl {
        query = query.to_block_excl(v);
    }
    Ok(query.select_log_fields(log_fields))
}

async fn enrich_token_pair_hint(
    offchain_hint: &mut String,
    offchain: &OffchainEnricher,
    token0_topic: &str,
    token1_topic: &str,
) {
    for (label, maybe_token_id) in [
        (
            "token0_price",
            exchange::topic_u256_to_decimal(token0_topic),
        ),
        (
            "token1_price",
            exchange::topic_u256_to_decimal(token1_topic),
        ),
    ] {
        if let Some(token_id) = maybe_token_id {
            if let Some(price) = offchain.cached_last_trade_price(&token_id).await {
                if !offchain_hint.is_empty() {
                    offchain_hint.push(' ');
                }
                offchain_hint.push_str(&format!("{label}={price}"));
            } else {
                offchain.prefetch_last_trade_price(token_id);
            }
        }
    }
}

fn append_registered_ids_hint(offchain_hint: &mut String, token0_topic: &str, token1_topic: &str) {
    if let Some(token0_id) = exchange::topic_u256_to_decimal(token0_topic) {
        if !offchain_hint.is_empty() {
            offchain_hint.push(' ');
        }
        offchain_hint.push_str(&format!("token0_id={token0_id}"));
    }

    if let Some(token1_id) = exchange::topic_u256_to_decimal(token1_topic) {
        if !offchain_hint.is_empty() {
            offchain_hint.push(' ');
        }
        offchain_hint.push_str(&format!("token1_id={token1_id}"));
    }
}

fn append_asset_ids_hint(offchain_hint: &mut String, data: &[u8]) {
    if let Some((maker_asset_id, taker_asset_id)) = decode_first_two_asset_ids_decimal(data) {
        if !offchain_hint.is_empty() {
            offchain_hint.push(' ');
        }
        offchain_hint.push_str(&format!(
            "maker_id={maker_asset_id} taker_id={taker_asset_id}"
        ));
    }
}

fn backoff_delay_ms(policy: &RetryPolicy, attempt: u32) -> u64 {
    let exp = 2u64.saturating_pow(attempt.saturating_sub(1));
    let base = policy.base_delay_ms.saturating_mul(exp);
    let jitter = ((attempt as u64).wrapping_mul(79)) % 41;
    base.saturating_add(jitter).min(policy.max_delay_ms)
}
