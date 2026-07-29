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

/// Decodes the second ABI word as `tokenId` from CTF Exchange V2
/// `OrderFilled` / `OrdersMatched` event data.
///
/// V2 layout (non-indexed words):
/// - `OrderFilled`:    [0]=side, [1]=tokenId, [2]=makerAmountFilled, [3]=takerAmountFilled, [4]=fee, [5]=builder, [6]=metadata
/// - `OrdersMatched`:  [0]=side, [1]=tokenId, [2]=makerAmountFilled, [3]=takerAmountFilled
pub fn decode_token_id_decimal(data: &[u8]) -> Option<String> {
    let words = split_abi_words(data);
    let token_word = words.get(1)?;
    Some(BigUint::from_bytes_be(token_word).to_str_radix(10))
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

    pub fn insert_token_id_decimal(&mut self, decimal: &str) -> Result<()> {
        let value = BigUint::parse_bytes(decimal.as_bytes(), 10)
            .with_context(|| format!("invalid decimal token id: {decimal}"))?;
        self.tracked_token_ids.insert(value.to_str_radix(16));
        Ok(())
    }

    pub fn tracked_tokens_len(&self) -> usize {
        self.tracked_token_ids.len()
    }

    /// Matches V2 `OrderFilled` / `OrdersMatched` by checking the single
    /// `tokenId` ABI word (word index 1).
    pub fn matches_token_event(&self, data: &[u8]) -> bool {
        if self.tracked_token_ids.is_empty() {
            return false;
        }

        let words = split_abi_words(data);
        let Some(token_word) = words.get(1) else {
            return false;
        };
        let token_hex = compact_u256_hex_from_word(token_word);
        self.tracked_token_ids.contains(&token_hex)
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

#[cfg(test)]
mod tests {
    use super::{
        ExchangeTracker, decode_token_id_decimal, extract_first_word_hex,
        normalize_condition_id_word, normalize_topic_word,
    };

    fn abi_words(words: &[u128]) -> Vec<u8> {
        let mut out = Vec::with_capacity(words.len() * 32);
        for word in words {
            out.extend_from_slice(&[0u8; 16]);
            out.extend_from_slice(&word.to_be_bytes());
        }
        out
    }

    #[test]
    fn normalize_topic_word_left_pads_to_32_bytes() {
        let topic = "0x1234";
        let normalized = normalize_topic_word(topic);
        assert_eq!(normalized.len(), 64);
        assert!(normalized.ends_with("1234"));
    }

    #[test]
    fn normalize_condition_id_word_rejects_too_long_value() {
        let too_long = format!("0x{}", "a".repeat(65));
        assert!(normalize_condition_id_word(&too_long).is_err());
    }

    #[test]
    fn extract_first_word_hex_handles_short_and_valid_data() {
        assert!(extract_first_word_hex(&[1u8; 31]).is_none());
        let data = vec![0xabu8; 64];
        let first = extract_first_word_hex(&data).expect("first word");
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn decode_token_id_decimal_returns_second_word() {
        // V2 OrderFilled data: [side=0, tokenId=1337, ...]
        let data = abi_words(&[0, 1337]);
        assert_eq!(decode_token_id_decimal(&data).as_deref(), Some("1337"));
        assert!(decode_token_id_decimal(&[0u8; 10]).is_none());
        assert!(decode_token_id_decimal(&[0u8; 32]).is_none());
    }

    #[test]
    fn matches_token_event_uses_word_one() {
        let tracker = ExchangeTracker::from_seed_csv(Some("0x2a")).expect("seed");

        // word[0] = side, word[1] = tokenId
        let matching = abi_words(&[1, 0x2a]);
        let non_matching = abi_words(&[0, 0x999]);
        let short = vec![0u8; 31];

        assert!(tracker.matches_token_event(&matching));
        assert!(!tracker.matches_token_event(&non_matching));
        assert!(!tracker.matches_token_event(&short));
    }

    #[test]
    fn insert_token_id_decimal_normalizes_for_match() {
        let mut tracker = ExchangeTracker::from_seed_csv(None).expect("empty");
        tracker.insert_token_id_decimal("42").expect("insert");
        let matching = abi_words(&[0, 42]);
        assert!(tracker.matches_token_event(&matching));
    }
}
