use anyhow::{Context, Result};
use fastwebsockets::{Frame, OpCode, Payload, WebSocket, handshake};
use http_body_util::Empty;
use hyper::{Request, body::Bytes, header::CONNECTION, header::UPGRADE};
use reqwest::Client;
use reqwest::Url;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{Duration, interval};
use tokio_native_tls::TlsConnector as TokioTlsConnector;

#[derive(Clone)]
pub struct OffchainEnricher {
    cfg: OffchainConfig,
    http: Client,
    token_price_cache: Arc<Mutex<HashMap<String, String>>>,
}

#[derive(Clone)]
pub struct OffchainConfig {
    pub enable_http: bool,
    pub enable_rtds: bool,
    pub gamma_base_url: String,
    pub clob_base_url: String,
    pub rtds_url: String,
    pub rtds_filters: String,
    pub rtds_print_updates: bool,
    pub rtds_strict_tls: bool,
    pub rtds_log_tls_details: bool,
    pub rtds_cert_sha256_allowlist: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketMetadata {
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub outcomes: String,
}

#[derive(Debug, Deserialize)]
struct LastTradePriceResponse {
    #[serde(default)]
    price: String,
}

impl OffchainConfig {
    pub fn from_env() -> Self {
        let enable_http = env_bool("ENABLE_HTTP_ENRICHMENT", true);
        let enable_rtds = env_bool("ENABLE_RTDS_WEBSOCKET", false);

        Self {
            enable_http,
            enable_rtds,
            gamma_base_url: std::env::var("POLY_GAMMA_BASE_URL")
                .unwrap_or_else(|_| "https://gamma-api.polymarket.com".to_string()),
            clob_base_url: std::env::var("POLY_CLOB_BASE_URL")
                .unwrap_or_else(|_| "https://clob.polymarket.com".to_string()),
            rtds_url: std::env::var("POLY_RTDS_URL")
                .unwrap_or_else(|_| "wss://ws-live-data.polymarket.com".to_string()),
            rtds_filters: std::env::var("RTDS_FILTERS")
                .unwrap_or_else(|_| "btcusdt,ethusdt,solusdt,xrpusdt".to_string()),
            rtds_print_updates: env_bool("RTDS_PRINT_UPDATES", false),
            rtds_strict_tls: env_bool("ENABLE_RTDS_STRICT_TLS", true),
            rtds_log_tls_details: env_bool("RTDS_LOG_TLS_DETAILS", true),
            rtds_cert_sha256_allowlist: parse_csv_env("RTDS_CERT_SHA256_ALLOWLIST"),
        }
    }
}

impl OffchainEnricher {
    pub fn new(cfg: OffchainConfig) -> Self {
        Self {
            cfg,
            http: Client::new(),
            token_price_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn config(&self) -> &OffchainConfig {
        &self.cfg
    }

    pub async fn fetch_market_by_condition(
        &self,
        condition_id: &str,
    ) -> Result<Option<MarketMetadata>> {
        if !self.cfg.enable_http {
            return Ok(None);
        }

        let mut url =
            Url::parse(&format!("{}/markets", self.cfg.gamma_base_url.trim_end_matches('/')))
                .context("failed to build gamma markets URL")?;
        url.query_pairs_mut()
            .append_pair("condition_ids", condition_id);

        let res = self
            .http
            .get(url)
            .send()
            .await
            .context("failed to call gamma markets endpoint")?;

        if !res.status().is_success() {
            return Ok(None);
        }

        let body = res
            .json::<Vec<MarketMetadata>>()
            .await
            .context("failed to decode gamma markets response")?;

        Ok(body.into_iter().next())
    }

    pub async fn fetch_last_trade_price(&self, token_id_decimal: &str) -> Result<Option<String>> {
        if !self.cfg.enable_http {
            return Ok(None);
        }

        {
            let cache = self.token_price_cache.lock().await;
            if let Some(price) = cache.get(token_id_decimal) {
                return Ok(Some(price.clone()));
            }
        }

        let mut url = Url::parse(&format!(
            "{}/last-trade-price",
            self.cfg.clob_base_url.trim_end_matches('/')
        ))
        .context("failed to build clob last-trade-price URL")?;
        url.query_pairs_mut()
            .append_pair("token_id", token_id_decimal);

        let res = self
            .http
            .get(url)
            .send()
            .await
            .context("failed to call clob last-trade-price endpoint")?;

        if !res.status().is_success() {
            return Ok(None);
        }

        let body = res
            .json::<LastTradePriceResponse>()
            .await
            .context("failed to decode last-trade-price response")?;

        if body.price.is_empty() {
            return Ok(None);
        }

        let mut cache = self.token_price_cache.lock().await;
        cache.insert(token_id_decimal.to_string(), body.price.clone());
        Ok(Some(body.price))
    }

    pub fn spawn_rtds_listener(&self) -> Option<JoinHandle<()>> {
        if !self.cfg.enable_rtds {
            return None;
        }

        let cfg = self.cfg.clone();
        Some(tokio::spawn(async move {
            if let Err(err) = run_rtds(cfg).await {
                eprintln!("[RTDS] listener stopped: {err:#}");
            }
        }))
    }
}

async fn run_rtds(cfg: OffchainConfig) -> Result<()> {
    let mut ws = connect_rtds(&cfg)
        .await
        .context("failed to connect to Polymarket RTDS")?;
    ws.set_auto_close(true);
    ws.set_auto_pong(true);
    ws.set_writev(false);

    let subscribe = json!({
        "action": "subscribe",
        "subscriptions": [
            {
                "topic": "crypto_prices",
                "type": "update",
                "filters": cfg.rtds_filters
            }
        ]
    });

    ws.write_frame(Frame::text(Payload::Owned(
        subscribe.to_string().into_bytes(),
    )))
        .await
        .context("failed to send RTDS subscribe")?;

    let mut ping = interval(Duration::from_secs(5));
    let mut printed = 0usize;

    loop {
        tokio::select! {
            _ = ping.tick() => {
                ws.write_frame(Frame::new(true, OpCode::Ping, None, Payload::Borrowed(&[])))
                    .await
                    .context("failed to send RTDS ping")?;
            }
            frame = ws.read_frame() => {
                let frame = frame.context("RTDS read error")?;
                if cfg.rtds_print_updates {
                    match frame.opcode {
                        OpCode::Text | OpCode::Binary => {
                            let txt = String::from_utf8_lossy(frame.payload.as_ref());
                            if printed < 50 {
                                println!("[RTDS] {txt}");
                                printed += 1;
                                if printed == 50 {
                                    println!("[RTDS] print limit reached (50); suppressing further updates");
                                }
                            }
                        }
                        OpCode::Ping | OpCode::Pong => {}
                        OpCode::Close => break,
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(())
}

async fn connect_rtds(
    cfg: &OffchainConfig,
) -> Result<WebSocket<hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>>> {
    let parsed = Url::parse(&cfg.rtds_url)
        .with_context(|| format!("invalid RTDS URL: {}", cfg.rtds_url))?;
    let host = parsed
        .host_str()
        .context("RTDS URL missing host")?
        .to_string();
    let scheme = parsed.scheme().to_string();

    let port = parsed
        .port_or_known_default()
        .context("RTDS URL missing known default port")?;
    let addr = format!("{host}:{port}");

    let path = {
        let p = parsed.path();
        let p = if p.is_empty() { "/" } else { p };
        match parsed.query() {
            Some(q) => format!("{p}?{q}"),
            None => p.to_string(),
        }
    };

    let ws_scheme = if scheme == "wss" { "https" } else { "http" };
    let uri = format!("{ws_scheme}://{addr}{path}");

    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .header("Host", addr)
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "upgrade")
        .header("Sec-WebSocket-Key", handshake::generate_key())
        .header("Sec-WebSocket-Version", "13")
        .body(Empty::<Bytes>::new())
        .context("failed to build RTDS websocket request")?;

    let tcp = TcpStream::connect((host.as_str(), port))
        .await
        .with_context(|| format!("failed to connect RTDS tcp socket at {host}:{port}"))?;

    if scheme == "wss" {
        let mut builder = native_tls::TlsConnector::builder();
        builder
            .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
            .danger_accept_invalid_certs(!cfg.rtds_strict_tls)
            .danger_accept_invalid_hostnames(!cfg.rtds_strict_tls);

        let tls_connector = builder
            .build()
            .context("failed to build TLS connector for RTDS")?;
        let tls_connector = TokioTlsConnector::from(tls_connector);
        let tls = tls_connector
            .connect(&host, tcp)
            .await
            .with_context(|| format!("failed TLS handshake for RTDS host {host}"))?;

        let cert_sha256 = tls_peer_cert_sha256(&tls)
            .context("failed reading RTDS peer certificate fingerprint")?;

        if cfg.rtds_log_tls_details {
            log_tls_peer_details(&host, cert_sha256.as_deref());
        }

        if !cfg.rtds_cert_sha256_allowlist.is_empty() {
            let Some(actual) = cert_sha256.as_deref() else {
                anyhow::bail!(
                    "RTDS certificate allowlist is configured but peer certificate is missing"
                );
            };

            let allowed = cfg
                .rtds_cert_sha256_allowlist
                .iter()
                .any(|p| p.eq_ignore_ascii_case(actual));
            if !allowed {
                anyhow::bail!(
                    "RTDS peer cert sha256 {} is not in RTDS_CERT_SHA256_ALLOWLIST",
                    actual
                );
            }
        }

        if !cfg.rtds_strict_tls {
            eprintln!("[RTDS] WARNING: strict TLS disabled (cert/hostname checks relaxed)");
        }

        let (ws, _) = client_handshake(req, tls).await?;
        Ok(ws)
    } else if scheme == "ws" {
        let (ws, _) = client_handshake(req, tcp).await?;
        Ok(ws)
    } else {
        anyhow::bail!("unsupported RTDS websocket scheme: {scheme}");
    }
}

fn tls_peer_cert_sha256(tls: &tokio_native_tls::TlsStream<TcpStream>) -> Result<Option<String>> {
    let cert_fingerprint = tls
        .get_ref()
        .peer_certificate()
        .context("failed reading RTDS peer certificate")?
        .and_then(|cert| cert.to_der().ok())
        .map(|der| sha256_hex(&der));

    Ok(cert_fingerprint)
}

fn log_tls_peer_details(host: &str, cert_fingerprint: Option<&str>) {
    println!(
        "[RTDS] tls host={} cert_sha256={}",
        host,
        cert_fingerprint.unwrap_or("none")
    );
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

async fn client_handshake<S>(
    req: Request<Empty<Bytes>>,
    stream: S,
) -> Result<(
    WebSocket<hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>>,
    hyper::Response<hyper::body::Incoming>,
)>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    handshake::client(&SpawnExecutor, req, stream)
        .await
        .context("fastwebsockets client handshake failed")
}

struct SpawnExecutor;

impl hyper::rt::Executor<Pin<Box<dyn Future<Output = ()> + Send>>> for SpawnExecutor {
    fn execute(&self, fut: Pin<Box<dyn Future<Output = ()> + Send>>) {
        tokio::task::spawn(fut);
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(default)
}

fn parse_csv_env(key: &str) -> Vec<String> {
    std::env::var(key)
        .ok()
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}
