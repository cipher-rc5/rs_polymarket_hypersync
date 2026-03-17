// file: src/main.rs
// description: Query Polymarket CTF contract logs on Polygon for a given condition ID via HyperSync
// reference: https://github.com/enviodev/hypersync-client-rust

use anyhow::{Context, Result};
use hypersync_client::{
    Client, StreamConfig,
    net_types::{LogField, LogFilter, Query},
};

mod contracts;
mod enrich;
mod exchange;
mod storage;

use contracts::{
    DEFAULT_POLYGON_HYPERSYNC_URL, POLYGON_CHAIN_ID,
    address::{CONDITIONAL_TOKENS, EXCHANGE, NEG_RISK_ADAPTER, NEG_RISK_EXCHANGE},
    topic::{
        CONDITION_RESOLUTION, NEG_RISK_PAYOUT_REDEMPTION, NEG_RISK_POSITION_SPLIT,
        NEG_RISK_POSITIONS_MERGE, ORDER_FILLED, ORDERS_MATCHED, PAYOUT_REDEMPTION, POSITION_SPLIT,
        POSITIONS_MERGE, TOKEN_REGISTERED,
    },
};
use exchange::{
    ExchangeTracker, extract_first_word_hex, normalize_condition_id_word, normalize_topic_word,
    parse_seed_env, topic_u256_to_decimal,
};
use enrich::{OffchainConfig, OffchainEnricher};
use storage::{EventStore, StoredEvent};

// The condition ID for this specific BTC Up/Down market.
// In CTF logs, the condition ID appears as topic2 on most events.
const CONDITION_ID: &str = "0x7b49294de4f325f82b071631ed8222ac5bba5ce95948018aff5a3c2ef6c5e595";

// Polygon block at approximately 2026-03-10 (market creation).
// Polygon runs ~2 blocks/sec so this gives a safe lookback buffer.
const FROM_BLOCK: u64 = 68_000_000;

fn normalize_hex(value: &str) -> String {
    value
        .strip_prefix("0x")
        .unwrap_or(value)
        .to_ascii_lowercase()
}

fn topic_contains_hex(topic: &str, needle_hex_without_0x: &str) -> bool {
    normalize_hex(topic).contains(needle_hex_without_0x)
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();

    let api_token = std::env::var("ENVIO_API_TOKEN").context("ENVIO_API_TOKEN env var not set")?;
    let hypersync_url = std::env::var("ENVIO_HYPERSYNC_URL").ok();
    let polygon_rpc_url = std::env::var("POLYGON_RPC_URL").ok();
    let effective_hypersync_url = hypersync_url
        .clone()
        .unwrap_or_else(|| DEFAULT_POLYGON_HYPERSYNC_URL.to_string());
    let effective_polygon_rpc = polygon_rpc_url
        .clone()
        .unwrap_or_else(|| effective_hypersync_url.clone());

    let include_exchange_logs =
        env_bool("INCLUDE_EXCHANGE_LOGS", true) || env_bool("INCLUDE_CLOB_LOGS", false);
    let include_order_filled = env_bool("INCLUDE_ORDER_FILLED", true);
    let include_orders_matched = env_bool("INCLUDE_ORDERS_MATCHED", true);
    let include_neg_risk_logs = env_bool("INCLUDE_NEG_RISK_LOGS", true);
    let follow_tail = env_bool("FOLLOW_TAIL", false);

    let offchain_cfg = OffchainConfig::from_env();
    let offchain = OffchainEnricher::new(offchain_cfg.clone());
    let _rtds_listener = offchain.spawn_rtds_listener();

    let seed_token_ids = parse_seed_env()?;
    let mut exchange_tracker = ExchangeTracker::from_seed_csv(seed_token_ids.as_deref())?;
    let event_store = EventStore::from_env()?;

    let condition_id = std::env::var("CONDITION_ID").unwrap_or_else(|_| CONDITION_ID.to_string());
    let from_block = std::env::var("FROM_BLOCK")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(FROM_BLOCK);
    let to_block_excl = std::env::var("TO_BLOCK_EXCL")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());

    if let Some(meta) = offchain.fetch_market_by_condition(&condition_id).await? {
        println!("market question: {}", meta.question);
        println!("market slug:     {}", meta.slug);
        if !meta.outcomes.is_empty() {
            println!("market outcomes: {}", meta.outcomes);
        }
    }

    let mut client_builder = Client::builder().api_token(api_token);
    if let Some(url) = &hypersync_url {
        client_builder = client_builder.url(url);
    } else {
        client_builder = client_builder.chain_id(POLYGON_CHAIN_ID);
    }

    let client = client_builder
        .build()
        .context("failed to create HyperSync client")?;

    let to_block_excl = if follow_tail {
        to_block_excl
    } else {
        Some(match to_block_excl {
            Some(v) => v,
            None => {
                client
                    .get_height()
                    .await
                    .context("failed to fetch chain height")?
                    + 1
            }
        })
    };

    println!("querying Polygon via HyperSync");
    println!("condition id:   {}", condition_id);
    println!("ctf contract:   {}", CONDITIONAL_TOKENS);
    println!("neg risk:       {}", NEG_RISK_ADAPTER);
    println!("exchange:       {}", EXCHANGE);
    println!("neg exchange:   {}", NEG_RISK_EXCHANGE);
    println!("from block:     {}", from_block);
    println!("hypersync url:  {}", effective_hypersync_url);
    println!("polygon rpc:    {}", effective_polygon_rpc);
    println!(
        "include exch:   {}",
        if include_exchange_logs { "yes" } else { "no" }
    );
    println!(
        "include fills:  {}",
        if include_order_filled { "yes" } else { "no" }
    );
    println!(
        "include match:  {}",
        if include_orders_matched { "yes" } else { "no" }
    );
    println!(
        "include nra:    {}",
        if include_neg_risk_logs { "yes" } else { "no" }
    );
    println!(
        "http enrich:    {}",
        if offchain.config().enable_http {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "rtds ws:        {}",
        if offchain.config().enable_rtds {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "rtds tls:       {}",
        if offchain.config().rtds_strict_tls {
            "strict"
        } else {
            "relaxed"
        }
    );
    println!(
        "rtds tls log:   {}",
        if offchain.config().rtds_log_tls_details {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "rtds tls pins:  {}",
        offchain.config().rtds_cert_sha256_allowlist.len()
    );
    println!("follow tail:    {}", if follow_tail { "yes" } else { "no" });
    if let Some(v) = to_block_excl {
        println!("to block excl:  {}", v);
    }
    println!("seed tokens:    {}", exchange_tracker.tracked_tokens_len());
    println!(
        "duckdb export:  {}",
        if event_store.is_some() { "enabled" } else { "off" }
    );
    if let Some(store) = &event_store {
        println!("duckdb path:    {}", store.duckdb_path());
        if let Some(path) = store.parquet_path() {
            println!("parquet path:   {}", path);
        }
    }
    println!();

    // CTF filter: match any of the four key event types on the CTF contract.
    // ConditionResolution encodes conditionId as topic1 (first indexed param).
    // PositionSplit, PositionsMerge, PayoutRedemption encode it as topic2.
    // We do a broad topic0 filter here and filter by condition ID in post-processing
    // since topic position differs per event type. For a tighter query you can
    // split into two LogFilter entries with matching topic positions.
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
    if include_order_filled {
        exchange_topics.push(ORDER_FILLED);
    }
    if include_orders_matched {
        exchange_topics.push(ORDERS_MATCHED);
    }

    // Exchange filter includes token registration and optionally fills/matches.
    // Fills/matches are filtered in-process using tracked token ids that are
    // discovered from TokenRegistered for the target condition.
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
    if include_neg_risk_logs {
        query = query.where_logs(neg_risk_filter);
    }
    if include_exchange_logs {
        query = query.where_logs(exchange_filter);
    }
    if let Some(v) = to_block_excl {
        query = query.to_block_excl(v);
    }
    query = query.select_log_fields(log_fields);

    println!("streaming logs...");
    println!();

    let ctf_contract_hex = normalize_hex(CONDITIONAL_TOKENS);
    let neg_risk_adapter_hex = normalize_hex(NEG_RISK_ADAPTER);
    let exchange_hex = normalize_hex(EXCHANGE);
    let neg_risk_exchange_hex = normalize_hex(NEG_RISK_EXCHANGE);
    let condition_id_hex = condition_id
        .strip_prefix("0x")
        .unwrap_or(condition_id.as_str())
        .to_ascii_lowercase();
    let condition_id_word = normalize_condition_id_word(&condition_id)?;

    let condition_resolution_topic = normalize_hex(CONDITION_RESOLUTION);
    let position_split_topic = normalize_hex(POSITION_SPLIT);
    let positions_merge_topic = normalize_hex(POSITIONS_MERGE);
    let payout_redemption_topic = normalize_hex(PAYOUT_REDEMPTION);

    let token_registered_topic = normalize_hex(TOKEN_REGISTERED);
    let order_filled_topic = normalize_hex(ORDER_FILLED);
    let orders_matched_topic = normalize_hex(ORDERS_MATCHED);

    let mut total_ctf = 0usize;
    let mut total_neg_risk = 0usize;
    let mut total_exchange = 0usize;
    let mut batches_without_matches = 0usize;

    let mut receiver = client
        .stream(query, StreamConfig::default())
        .await
        .context("failed to start stream")?;

    while let Some(response) = receiver.recv().await {
        let response = response.context("stream error")?;

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
                let mut offchain_hint = String::new();

                let tx_hash = log
                    .transaction_hash
                    .as_ref()
                    .map(std::string::ToString::to_string)
                    .unwrap_or_default();

                let data = log.data.as_ref().map(|v| v.as_ref()).unwrap_or_default();

                let block = log.block_number.map(u64::from).unwrap_or_default();
                let log_idx = log.log_index.map(u64::from).unwrap_or_default();

                let address_hex = normalize_hex(&address);

                let is_ctf = address_hex == ctf_contract_hex;
                let is_neg_risk = address_hex == neg_risk_adapter_hex;
                let is_exchange =
                    address_hex == exchange_hex || address_hex == neg_risk_exchange_hex;

                let source = if is_ctf {
                    let topic0_hex = normalize_hex(&topic0);

                    let matches_condition = if topic0_hex == condition_resolution_topic {
                        normalize_topic_word(&topic1) == condition_id_word
                    } else if topic0_hex == position_split_topic
                        || topic0_hex == positions_merge_topic
                    {
                        normalize_topic_word(&topic3) == condition_id_word
                    } else if topic0_hex == payout_redemption_topic {
                        extract_first_word_hex(data)
                            .map(|v| v == condition_id_word)
                            .unwrap_or(false)
                    } else {
                        false
                    };

                    if !matches_condition {
                        continue;
                    }
                    total_ctf += 1;
                    matched_in_batch += 1;
                    "CTF "
                } else if is_neg_risk {
                    if !topic_contains_hex(&topic2, &condition_id_hex) {
                        continue;
                    }
                    total_neg_risk += 1;
                    matched_in_batch += 1;
                    "NRA "
                } else if is_exchange {
                    let topic0_hex = normalize_hex(&topic0);

                    if topic0_hex == token_registered_topic {
                        if normalize_topic_word(&topic3) != condition_id_word {
                            continue;
                        }
                        exchange_tracker.register_token_pair(&topic1, &topic2);

                        if offchain.config().enable_http {
                            let t0 = topic_u256_to_decimal(&topic1);
                            let t1 = topic_u256_to_decimal(&topic2);
                            if let Some(token_id) = t0
                                && let Some(price) = offchain.fetch_last_trade_price(&token_id).await?
                            {
                                offchain_hint.push_str(&format!("token0_price={price} "));
                            }
                            if let Some(token_id) = t1
                                && let Some(price) = offchain.fetch_last_trade_price(&token_id).await?
                            {
                                offchain_hint.push_str(&format!("token1_price={price}"));
                            }
                        }
                    } else if topic0_hex == order_filled_topic {
                        if !include_order_filled || !exchange_tracker.matches_order_filled(data) {
                            continue;
                        }
                    } else if topic0_hex == orders_matched_topic {
                        if !include_orders_matched || !exchange_tracker.matches_orders_matched(data)
                        {
                            continue;
                        }
                    } else {
                        continue;
                    }

                    total_exchange += 1;
                    matched_in_batch += 1;
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

                println!(
                    "[{}] block={} log={} tx={}... topic0={} topic2={} tokens={} {}",
                    source,
                    block,
                    log_idx,
                    tx_short,
                    &topic0[..12.min(topic0.len())],
                    topic2,
                    exchange_tracker.tracked_tokens_len(),
                    offchain_hint
                );

                if let Some(store) = &event_store {
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
            batches_without_matches += 1;
            if batches_without_matches.is_multiple_of(50) {
                println!("  -- progress next_block: {}", response.next_block);
            }
        } else {
            batches_without_matches = 0;
        }
    }

    println!();
    println!("done.");
    println!("  ctf logs:  {}", total_ctf);
    println!("  nra logs:  {}", total_neg_risk);
    println!("  exch logs: {}", total_exchange);

    if let Some(store) = event_store {
        store.finalize()?;
    }

    Ok(())
}
