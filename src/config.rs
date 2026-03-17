use anyhow::{Context, Result};

pub const DEFAULT_CONDITION_ID: &str =
    "0x7b49294de4f325f82b071631ed8222ac5bba5ce95948018aff5a3c2ef6c5e595";
pub const DEFAULT_FROM_BLOCK: u64 = 68_000_000;

#[derive(Clone, Debug)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub api_token: String,
    pub hypersync_url: Option<String>,
    pub polygon_rpc_url: Option<String>,
    pub condition_id: String,
    pub from_block: u64,
    pub to_block_excl: Option<u64>,
    pub follow_tail: bool,
    pub include_exchange_logs: bool,
    pub include_order_filled: bool,
    pub include_orders_matched: bool,
    pub include_neg_risk_logs: bool,
    pub stream_retry: RetryPolicy,
    pub io_timeout_ms: u64,
    pub progress_log_every_batches: usize,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let api_token =
            std::env::var("ENVIO_API_TOKEN").context("ENVIO_API_TOKEN env var not set")?;

        let include_exchange_logs =
            env_bool("INCLUDE_EXCHANGE_LOGS", true) || env_bool("INCLUDE_CLOB_LOGS", false);

        let cfg = Self {
            api_token,
            hypersync_url: std::env::var("ENVIO_HYPERSYNC_URL").ok(),
            polygon_rpc_url: std::env::var("POLYGON_RPC_URL").ok(),
            condition_id: std::env::var("CONDITION_ID")
                .unwrap_or_else(|_| DEFAULT_CONDITION_ID.to_string()),
            from_block: env_u64("FROM_BLOCK")?.unwrap_or(DEFAULT_FROM_BLOCK),
            to_block_excl: env_u64("TO_BLOCK_EXCL")?,
            follow_tail: env_bool("FOLLOW_TAIL", false),
            include_exchange_logs,
            include_order_filled: env_bool("INCLUDE_ORDER_FILLED", true),
            include_orders_matched: env_bool("INCLUDE_ORDERS_MATCHED", true),
            include_neg_risk_logs: env_bool("INCLUDE_NEG_RISK_LOGS", true),
            stream_retry: RetryPolicy {
                max_attempts: env_u32("STREAM_RETRY_MAX_ATTEMPTS")?.unwrap_or(6),
                base_delay_ms: env_u64("STREAM_RETRY_BASE_DELAY_MS")?.unwrap_or(500),
                max_delay_ms: env_u64("STREAM_RETRY_MAX_DELAY_MS")?.unwrap_or(10_000),
            },
            io_timeout_ms: env_u64("IO_TIMEOUT_MS")?.unwrap_or(15_000),
            progress_log_every_batches: env_usize("PROGRESS_LOG_EVERY_BATCHES")?.unwrap_or(50),
        };

        cfg.validate()?;
        Ok(cfg)
    }

    pub fn effective_hypersync_url(&self, default_url: &str) -> String {
        self.hypersync_url
            .clone()
            .unwrap_or_else(|| default_url.to_string())
    }

    pub fn effective_polygon_rpc(&self, default_url: &str) -> String {
        self.polygon_rpc_url
            .clone()
            .unwrap_or_else(|| self.effective_hypersync_url(default_url))
    }

    fn validate(&self) -> Result<()> {
        if self.stream_retry.max_attempts == 0 {
            anyhow::bail!("STREAM_RETRY_MAX_ATTEMPTS must be >= 1");
        }
        if self.stream_retry.base_delay_ms == 0 {
            anyhow::bail!("STREAM_RETRY_BASE_DELAY_MS must be >= 1");
        }
        if self.stream_retry.max_delay_ms < self.stream_retry.base_delay_ms {
            anyhow::bail!("STREAM_RETRY_MAX_DELAY_MS must be >= STREAM_RETRY_BASE_DELAY_MS");
        }
        if self.io_timeout_ms < 100 {
            anyhow::bail!("IO_TIMEOUT_MS must be >= 100");
        }
        if self.progress_log_every_batches == 0 {
            anyhow::bail!("PROGRESS_LOG_EVERY_BATCHES must be >= 1");
        }
        if !self.include_exchange_logs && (self.include_order_filled || self.include_orders_matched)
        {
            anyhow::bail!(
                "INCLUDE_ORDER_FILLED/INCLUDE_ORDERS_MATCHED require INCLUDE_EXCHANGE_LOGS=true"
            );
        }
        Ok(())
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(default)
}

fn env_u64(key: &str) -> Result<Option<u64>> {
    match std::env::var(key) {
        Ok(v) => v
            .parse::<u64>()
            .with_context(|| format!("{key} must be an unsigned integer"))
            .map(Some),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(err) => Err(err).with_context(|| format!("failed reading {key}")),
    }
}

fn env_u32(key: &str) -> Result<Option<u32>> {
    match std::env::var(key) {
        Ok(v) => v
            .parse::<u32>()
            .with_context(|| format!("{key} must be an unsigned integer"))
            .map(Some),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(err) => Err(err).with_context(|| format!("failed reading {key}")),
    }
}

fn env_usize(key: &str) -> Result<Option<usize>> {
    match std::env::var(key) {
        Ok(v) => v
            .parse::<usize>()
            .with_context(|| format!("{key} must be an unsigned integer"))
            .map(Some),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(err) => Err(err).with_context(|| format!("failed reading {key}")),
    }
}
