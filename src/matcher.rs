use crate::exchange::{extract_first_word_hex, normalize_topic_word};

#[derive(Clone)]
pub struct CtfTopicMatchers {
    pub condition_resolution_topic: String,
    pub position_split_topic: String,
    pub positions_merge_topic: String,
    pub payout_redemption_topic: String,
}

pub fn topic_contains_hex(topic: &str, needle_hex_without_0x: &str) -> bool {
    normalize_hex(topic).contains(needle_hex_without_0x)
}

pub fn ctf_matches_condition(
    topic0: &str,
    topic1: &str,
    topic3: &str,
    data: &[u8],
    condition_id_word: &str,
    topics: &CtfTopicMatchers,
) -> bool {
    let topic0_hex = normalize_hex(topic0);

    if topic0_hex == topics.condition_resolution_topic {
        return normalize_topic_word(topic1) == condition_id_word;
    }

    if topic0_hex == topics.position_split_topic || topic0_hex == topics.positions_merge_topic {
        return normalize_topic_word(topic3) == condition_id_word;
    }

    if topic0_hex == topics.payout_redemption_topic {
        return extract_first_word_hex(data)
            .map(|v| v == condition_id_word)
            .unwrap_or(false);
    }

    false
}

pub fn normalize_hex(value: &str) -> String {
    value
        .strip_prefix("0x")
        .unwrap_or(value)
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{CtfTopicMatchers, ctf_matches_condition, normalize_hex, topic_contains_hex};

    fn bytes32_from_hex(hex_64: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(32);
        for i in (0..hex_64.len()).step_by(2) {
            let b = u8::from_str_radix(&hex_64[i..i + 2], 16).expect("valid hex");
            out.push(b);
        }
        out
    }

    #[test]
    fn topic_contains_hex_matches_case_insensitive() {
        let topic = "0x0000000000000000000000007B49294DE4F325F82B071631ED8222AC5BBA5CE9";
        let needle = "7b49294de4f325f82b071631ed8222ac5bba5ce9";
        assert!(topic_contains_hex(topic, needle));
    }

    #[test]
    fn ctf_matches_each_supported_shape() {
        let condition = "7b49294de4f325f82b071631ed8222ac5bba5ce95948018aff5a3c2ef6c5e595";
        let topics = CtfTopicMatchers {
            condition_resolution_topic: normalize_hex("0xaaa1"),
            position_split_topic: normalize_hex("0xaaa2"),
            positions_merge_topic: normalize_hex("0xaaa3"),
            payout_redemption_topic: normalize_hex("0xaaa4"),
        };

        assert!(ctf_matches_condition(
            "0xaaa1",
            &format!("0x{condition}"),
            "",
            &[],
            condition,
            &topics
        ));

        assert!(ctf_matches_condition(
            "0xaaa2",
            "",
            &format!("0x{condition}"),
            &[],
            condition,
            &topics
        ));

        assert!(ctf_matches_condition(
            "0xaaa3",
            "",
            &format!("0x{condition}"),
            &[],
            condition,
            &topics
        ));

        let mut data = bytes32_from_hex(condition);
        data.extend_from_slice(&[0u8; 32]);
        assert!(ctf_matches_condition(
            "0xaaa4", "", "", &data, condition, &topics
        ));
    }

    #[test]
    fn ctf_non_matching_inputs_return_false() {
        let condition = "7b49294de4f325f82b071631ed8222ac5bba5ce95948018aff5a3c2ef6c5e595";
        let topics = CtfTopicMatchers {
            condition_resolution_topic: normalize_hex("0xaaa1"),
            position_split_topic: normalize_hex("0xaaa2"),
            positions_merge_topic: normalize_hex("0xaaa3"),
            payout_redemption_topic: normalize_hex("0xaaa4"),
        };

        assert!(!ctf_matches_condition(
            "0xaaaa",
            "",
            "",
            &[],
            condition,
            &topics
        ));
        assert!(!ctf_matches_condition(
            "0xaaa1",
            "0x1111",
            "",
            &[],
            condition,
            &topics
        ));
    }
}
