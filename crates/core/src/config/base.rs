use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct IdentityConfig {
    pub key: Option<PathBuf>,
    pub controller_did: Option<String>,
    pub agreement: Option<PathBuf>,
    pub require_agreement: bool,
    pub nickname: Option<String>,
}

fn default_db_dir() -> PathBuf {
    PathBuf::from("db")
}
fn default_blobs_dir() -> PathBuf {
    PathBuf::from("blobs")
}

fn default_services_dir() -> PathBuf {
    PathBuf::from("services")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub engine: StorageEngine,
    pub db_dir: PathBuf,
    /// Compiled WASM component binary cache -- unrelated to `blob_store`
    /// below. Kept as-is; the name collision with the object/blob
    /// service is unfortunate but pre-existing, so the new config lives
    /// under a distinctly-named `blob_store` field instead.
    pub blobs_dir: PathBuf,
    pub encryption: bool,
    pub services_dir: PathBuf,
    /// Blob object service configuration.
    pub blob_store: BlobStoreConfig,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            engine: Default::default(),
            db_dir: default_db_dir(),
            blobs_dir: default_blobs_dir(),
            encryption: true,
            services_dir: default_services_dir(),
            blob_store: Default::default(),
        }
    }
}

fn default_blob_store_local_root() -> PathBuf {
    PathBuf::from("blob_objects")
}

fn default_max_blob_bytes() -> u64 {
    100 * 1024 * 1024 // 100 MiB
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BlobStoreConfig {
    pub backend: BlobBackend,
    /// Resolved relative to `app_local_data_dir` by `resolve_paths`, same
    /// as `db_dir`/`blobs_dir`. Only meaningful for `backend = "local"`.
    pub local_root: PathBuf,
    /// Only meaningful (and required) for `backend = "s3"`.
    pub s3: Option<S3BlobConfig>,
    /// Single-blob size cap, checked incrementally as an upload streams in.
    pub max_blob_bytes: u64,
    /// Optional aggregate per-service cap across all of a service's blobs.
    /// `None` means unlimited.
    pub max_service_total_bytes: Option<u64>,
}

impl Default for BlobStoreConfig {
    fn default() -> Self {
        Self {
            backend: Default::default(),
            local_root: default_blob_store_local_root(),
            s3: None,
            max_blob_bytes: default_max_blob_bytes(),
            max_service_total_bytes: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlobBackend {
    #[default]
    Local,
    S3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3BlobConfig {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageEngine {
    #[default]
    Sqlite,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: LogLevel,
    pub format: LogFormat,
    pub target: LogTarget,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Json,
    #[default]
    Pretty,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogTarget {
    #[default]
    Stdout,
    File,
}

fn default_relay_url() -> String {
    "http://localhost:7964".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IrohParentConfig {
    pub url: String,
}

impl Default for IrohParentConfig {
    fn default() -> Self {
        Self { url: default_relay_url() }
    }
}

fn default_signaling_server_url() -> String {
    "ws://localhost:7963/ws".to_string()
}
fn default_bootstrap_page_url() -> String {
    "ws://localhost:7962".to_string()
}
fn default_stun_servers() -> Vec<String> {
    vec!["stun:stun.l.google.com:19302".to_string()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebRtcParentConfig {
    pub signaling_url: String,
    pub bootstrap_url: String,
    pub stun_servers: Vec<String>,
}

impl Default for WebRtcParentConfig {
    fn default() -> Self {
        Self {
            signaling_url: default_signaling_server_url(),
            bootstrap_url: default_bootstrap_page_url(),
            stun_servers: default_stun_servers(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ParentCoordinatorConfig {
    pub iroh: Option<IrohParentConfig>,
    pub webrtc: Option<WebRtcParentConfig>,
    pub ble: Option<BridgeConfig>,
    pub lora: Option<BridgeConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BridgeConfig {
    pub enabled: bool,
    pub bridge: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileConfig {
    pub roles: Vec<String>,
}

fn default_communication_interfaces() -> Vec<String> {
    vec!["iroh".to_string(), "webrtc".to_string()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SubstrateGlobalConfig {
    pub communication_interfaces: Vec<String>,
    pub registry_url: Option<String>,
    pub coordinator_discovery_url: Option<String>,
    pub enable_bep0044_dht: bool,
}

impl Default for SubstrateGlobalConfig {
    fn default() -> Self {
        Self {
            communication_interfaces: default_communication_interfaces(),
            registry_url: None,
            coordinator_discovery_url: None,
            enable_bep0044_dht: !cfg!(test),
        }
    }
}

const fn default_max_attempts() -> u8 {
    3
}
const fn default_initial_backoff_ms() -> u64 {
    100
}
const fn default_backoff_multiplier() -> f64 {
    2.0
}
const fn default_max_backoff_ms() -> u64 {
    30_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RetryPolicy {
    pub max_attempts: u8,
    pub initial_backoff_ms: u64,
    pub backoff_multiplier: f64,
    pub max_backoff_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            initial_backoff_ms: default_initial_backoff_ms(),
            backoff_multiplier: default_backoff_multiplier(),
            max_backoff_ms: default_max_backoff_ms(),
        }
    }
}

const fn default_mqtt_channel_capacity() -> u64 {
    1024
}

// Mirrors `syneroym_mqtt_broker::MqttBrokerConfig` (same `channel_capacity`
// field, `u64` here vs. `usize` there, bridged with an `as usize` cast at
// the one call site in `crates/router/src/route_handler.rs`) -- `core`
// can't depend on `mqtt_broker`, so this is intentional duplication, not
// accidental drift.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MessagingConfig {
    /// Messages in flight between the host and the embedded broker (and
    /// separately, per-subscriber forwarding capacity). No `bind_addr`
    /// field -- ADR-0010's aspirational `[mqtt] bind_addr` network listener
    /// is explicitly dropped; the broker is reachable only
    /// in-process, via `Broker::link`.
    pub channel_capacity: u64,
}

impl Default for MessagingConfig {
    fn default() -> Self {
        Self { channel_capacity: default_mqtt_channel_capacity() }
    }
}

const fn default_max_concurrent_streams_per_service() -> u32 {
    8
}

/// Bidirectional streaming (ADR-0014). Each open stream holds a
/// live `Store`/`Instance` for its duration, so this caps per-service memory
/// use.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamingConfig {
    pub max_concurrent_streams_per_service: u32,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self { max_concurrent_streams_per_service: default_max_concurrent_streams_per_service() }
    }
}

/// Identity/capability admission. A caller whose
/// verified DID equals `admin_ucan_root` is granted `substrate/admin`
/// directly. UCAN chain verification is also rooted here: any
/// `CapabilityToken` chain presented at ingress must attenuate back to a
/// token issued by this same DID to be admitted (`build_caller`,
/// `crates/router/src/route_handler/io.rs`). Owner-rooted *service*
/// capability chains (owner != node admin) are verified here too, rooted
/// at the service's recorded owner rather than at `admin_ucan_root`
/// (ADR-0015 A6).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct IamConfig {
    /// Root DID authorized to issue Admin UCANs. At runtime
    /// (`substrate::runtime::setup_connection_router`) this is overridden by
    /// the substrate's verified `ControllerAgreement` controller when one is
    /// mutually signed (see `syneroym_identity::substrate`) -- this config
    /// value is only the fallback for deployments with no such agreement.
    pub admin_ucan_root: Option<String>,
    /// Grants a caller whose verified DID is **this node's own** the
    /// ability `supervisor/resolve`, node-wide (ADR-0022 §7).
    ///
    /// This is what lets a same-node client gateway or WebRTC coordinator
    /// resolve a logical (`-a…-s…`) hostname for an app whose supervisor
    /// runs here, with no credential file. Deliberately **not**
    /// `substrate/admin`: the grant is a bare `substrate:<node_did>`
    /// resource, which short-circuits `Capability::grants` and therefore
    /// covers `synapp:<any-app-did>` -- but its *ability* is only
    /// `supervisor/resolve`, so the node's own key gains resolution and
    /// nothing else. Says nothing about apps supervised elsewhere; those
    /// need `resolve_ucan` (`ClientGatewayRole`/`CoordinatorRole`),
    /// because the check runs on the remote supervisor. Defaults to
    /// `false`: a grant is asked for, not assumed, matching
    /// `admin_ucan_root`'s own symmetry.
    ///
    /// This gate is the *operator-side* answer for a node's own apps: it
    /// resolves an app's own `restricted` services with no token, for
    /// whoever is running on this node. It is unrelated to a service the
    /// app itself declares `topology_visibility = open` (ADR-0022 §5)
    /// -- that declaration needs neither this grant nor a
    /// `resolve_ucan`, and works for any caller on any installation.
    #[serde(default)]
    pub grant_resolve_to_node_did: bool,
}
