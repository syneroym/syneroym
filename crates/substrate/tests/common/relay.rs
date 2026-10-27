//! Stand-ins for the parts of a coordinator that the home-relay tests need:
//! a relay server, a `/v1/info` answer, and a parent registry that serves a
//! coordinator list. Every listener is bound to `127.0.0.1:0`.

#![allow(dead_code)]

use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use iroh::RelayUrl;
use iroh_relay::server::{RelayConfig as ServerRelayConfig, Server, ServerConfig};
use syneroym_core::{
    coordinator_info::{CoordinatorInfo, CoordinatorStatus, INFO_PATH, RelayInfo},
    curated_list::{CoordinatorEntry, CoordinatorTransport, CuratedList},
    dht_registry::{EndpointMechanism, RegistryClient, SignedEndpointInfo},
};
use tokio::{net::TcpListener, task::JoinHandle, time};

/// Spawns a plain-HTTP relay server on a dynamic local port.
pub async fn spawn_relay() -> Result<(Server, RelayUrl)> {
    let http_bind_addr = SocketAddr::from(([127, 0, 0, 1], 0));
    let mut relay_config = ServerRelayConfig::new(http_bind_addr);
    relay_config.key_cache_capacity = Some(1024);
    let mut server_config = ServerConfig::default();
    server_config.relay = Some(relay_config);
    let server = Server::spawn(server_config).await.context("failed to spawn relay server")?;
    let http_addr = server.http_addr().context("relay server has no HTTP address")?;
    let url: RelayUrl =
        format!("http://{http_addr}").parse().context("failed to parse relay url")?;
    Ok((server, url))
}

/// A listener on 127.0.0.1:0 that accepts TCP but never answers.
pub async fn dead_relay() -> Result<(TcpListener, RelayUrl)> {
    let listener = TcpListener::bind("127.0.0.1:0").await.context("failed to bind dead relay")?;
    let port = listener.local_addr()?.port();
    let url: RelayUrl =
        format!("http://127.0.0.1:{port}").parse().context("parse dead relay url")?;
    Ok((listener, url))
}

pub fn get_iroh_relay_url(record: &SignedEndpointInfo) -> Option<String> {
    for mechanism in &record.info.mechanisms {
        if let EndpointMechanism::Iroh { relay_url, .. } = mechanism {
            return relay_url.clone();
        }
    }
    None
}

pub async fn wait_for_registry_relay(
    client: &RegistryClient,
    did: &str,
    expected_relay: &str,
    timeout: Duration,
) -> Result<()> {
    let expected_url: RelayUrl = expected_relay.parse().context("parse expected relay")?;
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Ok(rec) = client.lookup(did, false).await
            && let Some(actual) = get_iroh_relay_url(&rec)
            && let Ok(actual_url) = actual.parse::<RelayUrl>()
            && actual_url == expected_url
        {
            return Ok(());
        }
        time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("timed out waiting for registry record to list relay {expected_relay}")
}

/// What a [`FakeInfo`] answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoMode {
    Healthy,
    AtCapacity,
    /// `503`: the coordinator does not answer.
    Unavailable,
}

/// A stand-in for a coordinator's `/v1/info`.
pub struct FakeInfo {
    /// Where the server listens, as an `info_url`.
    pub url: String,
    mode: Arc<Mutex<InfoMode>>,
    task: JoinHandle<()>,
}

impl FakeInfo {
    pub async fn spawn(mode: InfoMode) -> Result<Self> {
        let mode = Arc::new(Mutex::new(mode));
        let app = Router::new().route(INFO_PATH, get(serve_info)).with_state(mode.clone());
        let (url, task) = serve(app).await?;
        Ok(Self { url, mode, task })
    }

    pub fn set(&self, mode: InfoMode) {
        *self.mode.lock().unwrap() = mode;
    }
}

impl Drop for FakeInfo {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve_info(
    State(mode): State<Arc<Mutex<InfoMode>>>,
) -> Result<Json<CoordinatorInfo>, StatusCode> {
    let status = match *mode.lock().unwrap() {
        InfoMode::Healthy => CoordinatorStatus::Healthy,
        InfoMode::AtCapacity => CoordinatorStatus::AtCapacity,
        InfoMode::Unavailable => return Err(StatusCode::SERVICE_UNAVAILABLE),
    };
    Ok(Json(CoordinatorInfo {
        endpoint_addr_bytes: Vec::new(),
        substrate_id: "fake".into(),
        relay_url: None,
        parent_coordinator_url: None,
        status,
        identity: None,
        connections: None,
        tls: None,
        relay: Some(RelayInfo { online: true, clients: None, max_clients: None }),
        registry: None,
    }))
}

type SharedList = Arc<Mutex<Option<CuratedList<CoordinatorEntry>>>>;

/// A parent registry. `GET /coordinators` answers the list, or `503` when
/// there is none; only these requests are counted. `POST /register` answers
/// `200` and is not counted: a substrate's own registry forwards every
/// registration to its parent, so counting it would hide whether the
/// substrate asked for the list.
pub struct FakeListRegistry {
    pub url: String,
    list: SharedList,
    requests: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

#[derive(Clone)]
struct ListState {
    list: SharedList,
    requests: Arc<AtomicUsize>,
}

impl FakeListRegistry {
    pub async fn spawn(list: Option<CuratedList<CoordinatorEntry>>) -> Result<Self> {
        let list = Arc::new(Mutex::new(list));
        let requests = Arc::new(AtomicUsize::new(0));
        let state = ListState { list: list.clone(), requests: requests.clone() };
        let app = Router::new()
            .route("/coordinators", get(serve_list))
            .route("/register", post(|| async { StatusCode::OK }))
            .with_state(state);
        let (url, task) = serve(app).await?;
        Ok(Self { url, list, requests, task })
    }

    pub fn set(&self, list: Option<CuratedList<CoordinatorEntry>>) {
        *self.list.lock().unwrap() = list;
    }

    /// How many `GET /coordinators` requests came in.
    pub fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl Drop for FakeListRegistry {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve_list(
    State(state): State<ListState>,
) -> Result<Json<CuratedList<CoordinatorEntry>>, StatusCode> {
    state.requests.fetch_add(1, Ordering::SeqCst);
    state.list.lock().unwrap().clone().map(Json).ok_or(StatusCode::SERVICE_UNAVAILABLE)
}

/// Serves `app` on 127.0.0.1:0 and returns its base URL.
async fn serve(app: Router) -> Result<(String, JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await.context("bind fake server")?;
    let url = format!("http://{}", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((url, task))
}

/// A relay plus its fake info server, and the list entry for both.
pub struct ListedRelay {
    pub relay_url: RelayUrl,
    pub entry: CoordinatorEntry,
    pub info: FakeInfo,
    _server: Server,
}

pub async fn listed_relay(mode: InfoMode) -> Result<ListedRelay> {
    let (server, relay_url) = spawn_relay().await?;
    let info = FakeInfo::spawn(mode).await?;
    let entry = entry_for(relay_url.as_str(), &info);
    Ok(ListedRelay { relay_url, entry, info, _server: server })
}

pub fn entry_for(relay_url: &str, info: &FakeInfo) -> CoordinatorEntry {
    CoordinatorEntry {
        transport: CoordinatorTransport::IrohRelay,
        url: relay_url.to_owned(),
        info_url: info.url.clone(),
        operator: "test".into(),
    }
}

pub fn list_of(entries: &[CoordinatorEntry], max_age_secs: u64) -> CuratedList<CoordinatorEntry> {
    CuratedList { max_age_secs, entries: entries.to_vec() }
}
