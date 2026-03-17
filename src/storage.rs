use anyhow::{Context, Result};
use duckdb::{params, Connection};
use std::path::{Path, PathBuf};

pub struct EventStore {
    conn: Connection,
    duckdb_path: String,
    export_parquet_path: Option<String>,
}

pub struct StoredEvent<'a> {
    pub source: &'a str,
    pub block_number: u64,
    pub log_index: u64,
    pub tx_hash: &'a str,
    pub address: &'a str,
    pub topic0: &'a str,
    pub topic1: &'a str,
    pub topic2: &'a str,
    pub topic3: &'a str,
    pub tracked_tokens: usize,
    pub offchain_hint: &'a str,
}

impl EventStore {
    pub fn from_env() -> Result<Option<Self>> {
        let duckdb_path = std::env::var("EXPORT_DUCKDB_PATH").ok();
        let parquet_path = std::env::var("EXPORT_PARQUET_PATH").ok();
        let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".to_string());

        if duckdb_path.is_none() && parquet_path.is_none() {
            return Ok(None);
        }

        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("failed to create data directory at {data_dir}"))?;

        let duckdb_file = duckdb_path.unwrap_or_else(|| "polymarket_enriched.duckdb".to_string());
        let duckdb_full_path = resolve_to_data_dir(&data_dir, &duckdb_file);
        let duckdb_full_path_str = duckdb_full_path.to_string_lossy().to_string();

        let conn = Connection::open(&duckdb_full_path_str)
            .with_context(|| format!("failed opening duckdb database at {duckdb_full_path_str}"))?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS matched_events (
                source TEXT,
                block_number BIGINT,
                log_index BIGINT,
                tx_hash TEXT,
                address TEXT,
                topic0 TEXT,
                topic1 TEXT,
                topic2 TEXT,
                topic3 TEXT,
                tracked_tokens BIGINT,
                offchain_hint TEXT,
                ingested_at TIMESTAMP DEFAULT current_timestamp
            );
            ",
        )
        .context("failed creating matched_events table")?;

        let export_parquet_path = parquet_path.map(|path| {
            resolve_to_data_dir(&data_dir, &path)
                .to_string_lossy()
                .to_string()
        });

        Ok(Some(Self {
            conn,
            duckdb_path: duckdb_full_path_str,
            export_parquet_path,
        }))
    }

    pub fn duckdb_path(&self) -> &str {
        &self.duckdb_path
    }

    pub fn parquet_path(&self) -> Option<&str> {
        self.export_parquet_path.as_deref()
    }

    pub fn insert_event(&self, ev: &StoredEvent<'_>) -> Result<()> {
        self.conn
            .execute(
                "
                INSERT INTO matched_events
                    (source, block_number, log_index, tx_hash, address, topic0, topic1, topic2, topic3, tracked_tokens, offchain_hint)
                VALUES
                    (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                ",
                params![
                    ev.source,
                    ev.block_number as i64,
                    ev.log_index as i64,
                    ev.tx_hash,
                    ev.address,
                    ev.topic0,
                    ev.topic1,
                    ev.topic2,
                    ev.topic3,
                    ev.tracked_tokens as i64,
                    ev.offchain_hint,
                ],
            )
            .context("failed inserting matched event")?;

        Ok(())
    }

    pub fn finalize(self) -> Result<()> {
        if let Some(path) = self.export_parquet_path {
            let escaped = path.replace('\'', "''");
            let sql = format!(
                "COPY (SELECT * FROM matched_events ORDER BY block_number, log_index) TO '{}' (FORMAT 'parquet');",
                escaped
            );
            self.conn
                .execute_batch(&sql)
                .with_context(|| format!("failed exporting parquet to {path}"))?;
        }

        Ok(())
    }
}

fn resolve_to_data_dir(data_dir: &str, requested: &str) -> PathBuf {
    let requested_path = Path::new(requested);
    let file_name = requested_path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| requested.to_string());

    Path::new(data_dir).join(file_name)
}
