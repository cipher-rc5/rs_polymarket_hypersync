use anyhow::{Context, Result};
use num_bigint::BigUint;
use std::collections::HashSet;

fn normalize_hex(value: &str) -> String {
    value
        .strip_prefix("0x")
        .unwrap_or(value)
        .to_ascii_lowercase()
}

fn compact_u256_hex(raw_hex: &str) -> String {
    let normalized = normalize_hex(raw_hex);
    let compact = normalized.trim_start_matches('0');
    if compact.is_empty() {
        "0".to_string()
    } else {
        compact.to_string()
    }
}

pub fn topic_u256_to_decimal(topic: &str) -> Option<String> {
    let compact = compact_u256_hex(topic);
    BigUint::parse_bytes(compact.as_bytes(), 16).map(|v| v.to_str_radix(10))
}

fn compact_u256_hex_from_word(word: &[u8]) -> String {
    let first_nonzero = word.iter().position(|b| *b != 0).unwrap_or(word.len());
    if first_nonzero == word.len() {
        return "0".to_string();
    }

    let mut out = String::with_capacity((word.len() - first_nonzero) * 2);
    for b in &word[first_nonzero..] {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

pub struct ExchangeTracker {
    tracked_token_ids: HashSet<String>,
}

impl ExchangeTracker {
    pub fn from_seed_csv(seed_csv: Option<&str>) -> Result<Self> {
        let mut tracked_token_ids = HashSet::new();

        if let Some(csv) = seed_csv {
            for token in csv.split(',') {
                let token = token.trim();
                if token.is_empty() {
                    continue;
                }

                if !token.starts_with("0x") {
                    anyhow::bail!(
                        "MARKET_TOKEN_IDS must use hex token ids, got '{token}'. Example: 0x1234,0xabcd"
                    );
                }

                tracked_token_ids.insert(compact_u256_hex(token));
            }
        }

        Ok(Self { tracked_token_ids })
    }

    pub fn register_token_pair(&mut self, token0_topic: &str, token1_topic: &str) {
        self.tracked_token_ids
            .insert(compact_u256_hex(token0_topic));
        self.tracked_token_ids
            .insert(compact_u256_hex(token1_topic));
    }

    pub fn tracked_tokens_len(&self) -> usize {
        self.tracked_token_ids.len()
    }

    pub fn matches_order_filled(&self, data: &[u8]) -> bool {
        if self.tracked_token_ids.is_empty() {
            return false;
        }

        // OrderFilled data words:
        // [0]=makerAssetId [1]=takerAssetId [2]=makerAmountFilled [3]=takerAmountFilled [4]=fee
        let words = split_abi_words(data);
        if words.len() < 2 {
            return false;
        }

        let maker_asset = compact_u256_hex_from_word(words[0]);
        let taker_asset = compact_u256_hex_from_word(words[1]);

        self.tracked_token_ids.contains(&maker_asset)
            || self.tracked_token_ids.contains(&taker_asset)
    }

    pub fn matches_orders_matched(&self, data: &[u8]) -> bool {
        if self.tracked_token_ids.is_empty() {
            return false;
        }

        // OrdersMatched data words:
        // [0]=makerAssetId [1]=takerAssetId [2]=makerAmountFilled [3]=takerAmountFilled
        let words = split_abi_words(data);
        if words.len() < 2 {
            return false;
        }

        let maker_asset = compact_u256_hex_from_word(words[0]);
        let taker_asset = compact_u256_hex_from_word(words[1]);

        self.tracked_token_ids.contains(&maker_asset)
            || self.tracked_token_ids.contains(&taker_asset)
    }
}

fn split_abi_words(data: &[u8]) -> Vec<&[u8]> {
    data.chunks_exact(32).collect()
}

pub fn extract_first_word_hex(data: &[u8]) -> Option<String> {
    data.get(..32).map(|word| {
        let mut out = String::with_capacity(64);
        for b in word {
            out.push_str(&format!("{b:02x}"));
        }
        out
    })
}

pub fn normalize_topic_word(topic: &str) -> String {
    let normalized = normalize_hex(topic);
    if normalized.len() >= 64 {
        normalized[normalized.len() - 64..].to_string()
    } else {
        format!("{normalized:0>64}")
    }
}

pub fn normalize_condition_id_word(condition_id: &str) -> Result<String> {
    let normalized = normalize_hex(condition_id);
    if normalized.len() > 64 {
        anyhow::bail!("condition id must be <= 32 bytes hex");
    }
    Ok(format!("{normalized:0>64}"))
}

pub fn parse_seed_env() -> Result<Option<String>> {
    match std::env::var("MARKET_TOKEN_IDS") {
        Ok(v) => {
            let trimmed = v.trim().to_string();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed))
            }
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(err) => Err(err).context("failed reading MARKET_TOKEN_IDS env var"),
    }
}
