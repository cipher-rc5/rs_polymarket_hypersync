use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use arrow::array::{Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use duckdb::{Connection, params};
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;

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
            export_parquet_zstd(&self.conn, &path)?;
        }

        Ok(())
    }
}

/// Arrow schema for matched_events rows (mirrors the DuckDB table, excluding
/// `ingested_at` which is a server-side default and not meaningful in export).
fn matched_events_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("source", DataType::Utf8, true),
        Field::new("block_number", DataType::Int64, true),
        Field::new("log_index", DataType::Int64, true),
        Field::new("tx_hash", DataType::Utf8, true),
        Field::new("address", DataType::Utf8, true),
        Field::new("topic0", DataType::Utf8, true),
        Field::new("topic1", DataType::Utf8, true),
        Field::new("topic2", DataType::Utf8, true),
        Field::new("topic3", DataType::Utf8, true),
        Field::new("tracked_tokens", DataType::Int64, true),
        Field::new("offchain_hint", DataType::Utf8, true),
    ]))
}

/// Query DuckDB for all matched events ordered by block / log index, then write
/// them to a Parquet file using Zstd compression via the native `parquet` crate.
fn export_parquet_zstd(conn: &Connection, path: &str) -> Result<()> {
    let schema = matched_events_schema();

    let file =
        File::create(path).with_context(|| format!("failed creating parquet file at {path}"))?;

    let props = WriterProperties::builder()
        .set_compression(Compression::ZSTD(
            ZstdLevel::try_new(3).context("invalid zstd compression level")?,
        ))
        .build();

    let mut writer = ArrowWriter::try_new(file, Arc::clone(&schema), Some(props))
        .context("failed creating parquet ArrowWriter")?;

    // Read all rows from DuckDB in a single ordered query.
    let mut stmt = conn
        .prepare(
            "SELECT source, block_number, log_index, tx_hash, address,
                    topic0, topic1, topic2, topic3, tracked_tokens, offchain_hint
             FROM matched_events
             ORDER BY block_number, log_index",
        )
        .context("failed preparing matched_events query")?;

    // Collect into columnar vecs so we can build Arrow arrays efficiently.
    let mut col_source: Vec<Option<String>> = Vec::new();
    let mut col_block_number: Vec<Option<i64>> = Vec::new();
    let mut col_log_index: Vec<Option<i64>> = Vec::new();
    let mut col_tx_hash: Vec<Option<String>> = Vec::new();
    let mut col_address: Vec<Option<String>> = Vec::new();
    let mut col_topic0: Vec<Option<String>> = Vec::new();
    let mut col_topic1: Vec<Option<String>> = Vec::new();
    let mut col_topic2: Vec<Option<String>> = Vec::new();
    let mut col_topic3: Vec<Option<String>> = Vec::new();
    let mut col_tracked_tokens: Vec<Option<i64>> = Vec::new();
    let mut col_offchain_hint: Vec<Option<String>> = Vec::new();

    let mut rows = stmt
        .query([])
        .context("failed querying matched_events for parquet export")?;

    while let Some(row) = rows.next().context("failed reading matched_events row")? {
        col_source.push(row.get(0).ok());
        col_block_number.push(row.get(1).ok());
        col_log_index.push(row.get(2).ok());
        col_tx_hash.push(row.get(3).ok());
        col_address.push(row.get(4).ok());
        col_topic0.push(row.get(5).ok());
        col_topic1.push(row.get(6).ok());
        col_topic2.push(row.get(7).ok());
        col_topic3.push(row.get(8).ok());
        col_tracked_tokens.push(row.get(9).ok());
        col_offchain_hint.push(row.get(10).ok());
    }

    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(StringArray::from(col_source)),
            Arc::new(Int64Array::from(col_block_number)),
            Arc::new(Int64Array::from(col_log_index)),
            Arc::new(StringArray::from(col_tx_hash)),
            Arc::new(StringArray::from(col_address)),
            Arc::new(StringArray::from(col_topic0)),
            Arc::new(StringArray::from(col_topic1)),
            Arc::new(StringArray::from(col_topic2)),
            Arc::new(StringArray::from(col_topic3)),
            Arc::new(Int64Array::from(col_tracked_tokens)),
            Arc::new(StringArray::from(col_offchain_hint)),
        ],
    )
    .context("failed building Arrow RecordBatch for parquet export")?;

    writer
        .write(&batch)
        .context("failed writing RecordBatch to parquet")?;

    writer.close().context("failed finalizing parquet file")?;

    Ok(())
}

fn resolve_to_data_dir(data_dir: &str, requested: &str) -> PathBuf {
    let requested_path = Path::new(requested);
    let file_name = requested_path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| requested.to_string());

    Path::new(data_dir).join(file_name)
}
