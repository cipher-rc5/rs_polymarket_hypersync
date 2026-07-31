pub const POLYGON_CHAIN_ID: u64 = 137;
pub const DEFAULT_POLYGON_HYPERSYNC_URL: &str = "https://137.hypersync.xyz";

pub mod address {
    pub const CONDITIONAL_TOKENS: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
    pub const EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";
    pub const NEG_RISK_EXCHANGE: &str = "0xe2222d279d744050d28e00520010520000310F59";
    pub const NEG_RISK_ADAPTER: &str = "0xd91E80cF2E7be2e162c6513ceD06f1dD0dA35296";
}

pub mod topic {
    pub const CONDITION_RESOLUTION: &str =
        "0xb44d84d3289691f71497564b85d4233648d9dbae8cbdbb4329f301c3a0185894";
    pub const POSITION_SPLIT: &str =
        "0x2e6bb91f8cbcda0c93623c54d0403a43514fabc40084ec96b6d5379a74786298";
    pub const POSITIONS_MERGE: &str =
        "0x6f13ca62553fcc2bcd2372180a43949c1e4cebba603901ede2f4e14f36b282ca";
    pub const PAYOUT_REDEMPTION: &str =
        "0x2682012a4a4f1973119f1c9b90745d1bd91fa2bab387344f044cb3586864d18d";

    pub const NEG_RISK_POSITION_SPLIT: &str =
        "0xbbed930dbfb7907ae2d60ddf78345610214f26419a0128df39b6cc3d9e5df9b0";
    pub const NEG_RISK_POSITIONS_MERGE: &str =
        "0xba33ac50d8894676597e6e35dc09cff59854708b642cd069d21eb9c7ca072a04";
    pub const NEG_RISK_PAYOUT_REDEMPTION: &str =
        "0x9140a6a270ef945260c03894b3c6b3b2695e9d5101feef0ff24fec960cfd3224";

    // CTF Exchange V2 — OrderFilled(bytes32,address,address,uint8,uint256,uint256,uint256,uint256,bytes32,bytes32)
    pub const ORDER_FILLED: &str =
        "0xd543adfd945773f1a62f74f0ee55a5e3b9b1a28262980ba90b1a89f2ea84d8ee";
    // CTF Exchange V2 — OrdersMatched(bytes32,address,uint8,uint256,uint256,uint256)
    pub const ORDERS_MATCHED: &str =
        "0x174b3811690657c217184f89418266767c87e4805d09680c39fc9c031c0cab7c";
}
