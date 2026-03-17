use anyhow::{Context, Result};
use duckdb::{Connection, params};
use std::path::{Path, PathBuf};

pub struct EventStore {
    conn: Connection,
    duckdb_path: String,
    export_parquet_path: Option<String>,
    batch_size: usize,
    pending: Vec<OwnedStoredEvent>,
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

struct OwnedStoredEvent {
    source: String,
    block_number: u64,
    log_index: u64,
    tx_hash: String,
    address: String,
    topic0: String,
    topic1: String,
    topic2: String,
    topic3: String,
    tracked_tokens: usize,
    offchain_hint: String,
}

impl OwnedStoredEvent {
    fn from_borrowed(ev: &StoredEvent<'_>) -> Self {
        Self {
            source: ev.source.to_string(),
            block_number: ev.block_number,
            log_index: ev.log_index,
            tx_hash: ev.tx_hash.to_string(),
            address: ev.address.to_string(),
            topic0: ev.topic0.to_string(),
            topic1: ev.topic1.to_string(),
            topic2: ev.topic2.to_string(),
            topic3: ev.topic3.to_string(),
            tracked_tokens: ev.tracked_tokens,
            offchain_hint: ev.offchain_hint.to_string(),
        }
    }
}

impl EventStore {
    pub fn from_env() -> Result<Option<Self>> {
        let duckdb_path = std::env::var("EXPORT_DUCKDB_PATH").ok();
        let parquet_path = std::env::var("EXPORT_PARQUET_PATH").ok();
        let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".to_string());
        let batch_size = std::env::var("STORAGE_BATCH_SIZE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(200);

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
            batch_size: batch_size.max(1),
            pending: Vec::new(),
        }))
    }

    pub fn duckdb_path(&self) -> &str {
        &self.duckdb_path
    }

    pub fn parquet_path(&self) -> Option<&str> {
        self.export_parquet_path.as_deref()
    }

    pub fn insert_event(&mut self, ev: &StoredEvent<'_>) -> Result<()> {
        self.pending.push(OwnedStoredEvent::from_borrowed(ev));
        if self.pending.len() >= self.batch_size {
            self.flush_pending()?;
        }
        Ok(())
    }

    pub fn flush_pending(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }

        self.conn
            .execute_batch("BEGIN TRANSACTION;")
            .context("failed starting storage transaction")?;

        for ev in &self.pending {
            if let Err(err) = self.conn.execute(
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
            ) {
                let _ = self.conn.execute_batch("ROLLBACK;");
                return Err(err).context("failed inserting matched event");
            }
        }

        self.conn
            .execute_batch("COMMIT;")
            .context("failed committing storage transaction")?;
        self.pending.clear();
        Ok(())
    }

    pub fn finalize(mut self) -> Result<()> {
        self.flush_pending()?;

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
