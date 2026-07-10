//! Configuration types for the Syneroym substrate.

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::util;

pub const DEFAULT_SUBSTRATE_KEY_FILE: &str = "substrate.key";

fn default_app_config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("syneroym")
}

fn default_app_local_data_dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("syneroym")
}

fn default_app_data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("syneroym")
}

fn default_app_cache_dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".")).join("syneroym")
}

fn default_app_log_dir() -> PathBuf {
    default_app_local_data_dir().join("logs")
}

const fn default_config_version() -> u32 {
    1
}

fn default_profile() -> String {
    "full".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SubstrateConfig {
    pub config_version: u32,

    pub app_config_dir: PathBuf,
    pub app_local_data_dir: PathBuf,
    pub app_data_dir: PathBuf,
    pub app_cache_dir: PathBuf,
    pub app_log_dir: PathBuf,

    pub profile: String,

    pub identity: IdentityConfig,

    pub storage: StorageConfig,
    pub logging: LoggingConfig,

    pub parent_coordinator: ParentCoordinatorConfig,

    pub profiles: HashMap<String, ProfileConfig>,

    pub roles: RolesConfig,
    pub substrate: SubstrateGlobalConfig,
    pub retry: RetryPolicy,
    pub tls: Option<SubstrateTlsConfig>,
    /// Embedded MQTT broker for `syneroym:messaging` (M3B Slice 6A,
    /// ADR-0010). A core, always-on capability -- not an optional
    /// deployment role like `RolesConfig`'s members.
    pub mqtt: MessagingConfig,
}

/// Useful helper functions
impl SubstrateConfig {
    /// Returns the directory where hosted app certificates are stored
    pub fn hosted_apps_dir(&self) -> PathBuf {
        self.app_local_data_dir.join("hosted_apps")
    }

    /// Resolves relative storage paths by prepending `app_local_data_dir`.
    pub fn resolve_paths(&mut self) {
        if self.storage.db_dir.is_relative() {
            self.storage.db_dir = self.app_local_data_dir.join(&self.storage.db_dir);
        }

        if self.storage.blobs_dir.is_relative() {
            self.storage.blobs_dir = self.app_local_data_dir.join(&self.storage.blobs_dir);
        }

        if self.storage.blob_store.local_root.is_relative() {
            self.storage.blob_store.local_root =
                self.app_local_data_dir.join(&self.storage.blob_store.local_root);
        }

        if let Some(key) = &self.identity.key
            && key.is_relative()
        {
            self.identity.key = Some(self.app_data_dir.join(key));
        }

        if let Some(agreement) = &self.identity.agreement
            && agreement.is_relative()
        {
            self.identity.agreement = Some(self.app_data_dir.join(agreement));
        }

        if let Some(coordinator) = &mut self.roles.coordinator
            && let Some(tls) = &mut coordinator.tls
        {
            if tls.cert_path.is_relative() {
                tls.cert_path = self.app_config_dir.join(&tls.cert_path);
            }
            if tls.key_path.is_relative() {
                tls.key_path = self.app_config_dir.join(&tls.key_path);
            }
        }

        if let Some(tls) = &mut self.tls {
            if tls.cert_path.is_relative() {
                tls.cert_path = self.app_config_dir.join(&tls.cert_path);
            }
            if tls.key_path.is_relative() {
                tls.key_path = self.app_config_dir.join(&tls.key_path);
            }
        }
    }
}

impl Default for SubstrateConfig {
    fn default() -> Self {
        Self {
            config_version: default_config_version(),
            app_config_dir: default_app_config_dir(),
            app_local_data_dir: default_app_local_data_dir(),
            app_data_dir: default_app_data_dir(),
            app_cache_dir: default_app_cache_dir(),
            app_log_dir: default_app_log_dir(),
            profile: default_profile(),
            identity: Default::default(),
            storage: Default::default(),
            logging: Default::default(),
            parent_coordinator: Default::default(),
            profiles: Default::default(),
            roles: Default::default(),
            substrate: Default::default(),
            retry: Default::default(),
            tls: None,
            mqtt: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstrateTlsConfig {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub reload_on_sigusr1: bool,
}

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
    /// below. Kept as-is; the name collision with the M3B object/blob
    /// service is unfortunate but pre-existing, so the new config lives
    /// under a distinctly-named `blob_store` field instead.
    pub blobs_dir: PathBuf,
    pub encryption: bool,
    pub services_dir: PathBuf,
    /// M3B blob object service configuration (Slice 5).
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RolesConfig {
    pub app_sandbox: Option<AppSandboxRole>,
    pub podman_sandbox: Option<PodmanSandboxRole>,
    pub community_registry: Option<ServiceRegistryRole>,
    pub coordinator: Option<CoordinatorRole>,
    pub client_gateway: Option<ClientGatewayRole>,
    pub observability: Option<ObservabilityRole>,
}

fn default_podman_path() -> String {
    "podman".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PodmanSandboxRole {
    pub podman_path: String,
}

impl Default for PodmanSandboxRole {
    fn default() -> Self {
        Self { podman_path: default_podman_path() }
    }
}

fn default_communication_interfaces() -> Vec<String> {
    vec!["iroh".to_string(), "webrtc".to_string()]
}
const fn default_wasm_sandbox() -> bool {
    true
}
const fn default_cpu_limit() -> u32 {
    1
}
fn default_memory_limit() -> String {
    "1Gi".to_string()
}
const fn default_max_concurrent_instances() -> u32 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSandboxRole {
    /// Enables the WASM component sandbox.
    pub wasm_sandbox: bool,
    pub cpu_limit: u32,
    pub memory_limit: String,
    pub max_concurrent_instances: u32,
    pub default_max_instructions: Option<u64>,
    pub default_max_memory_bytes: Option<u64>,
}

impl AppSandboxRole {
    #[must_use]
    pub fn memory_limit_bytes(&self) -> u64 {
        util::parse_size_string(&self.memory_limit, 128 * 1024 * 1024)
    }
}

impl Default for AppSandboxRole {
    fn default() -> Self {
        Self {
            wasm_sandbox: default_wasm_sandbox(),
            cpu_limit: default_cpu_limit(),
            memory_limit: default_memory_limit(),
            max_concurrent_instances: default_max_concurrent_instances(),
            default_max_instructions: Some(10_000_000_000),
            default_max_memory_bytes: Some(256 * 1024 * 1024),
        }
    }
}

fn default_registry_http_bind_address() -> String {
    "0.0.0.0:7961".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceRegistryRole {
    pub access: AccessControl,
    pub http_bind_address: String,
    pub parent_registry_url: Option<String>,
}

impl Default for ServiceRegistryRole {
    fn default() -> Self {
        Self {
            access: Default::default(),
            http_bind_address: default_registry_http_bind_address(),
            parent_registry_url: None,
        }
    }
}

/// Represents configurations like `access = "everyone"` OR `access = ["did1",
/// "did2"]`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AccessControl {
    String(String),
    List(Vec<String>),
}

impl Default for AccessControl {
    fn default() -> Self {
        Self::String("everyone".to_string())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CoordinatorRole {
    pub access: AccessControl,
    pub tls: Option<TlsConfig>,
    pub iroh: Option<CoordinatorIrohConfig>,
    pub webrtc: Option<CoordinatorWebRtcConfig>,
    pub transport_bridge: Option<TransportBridgeRole>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TlsConfig {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

fn default_iroh_http_bind_address() -> String {
    "0.0.0.0:7964".to_string()
}
fn default_iroh_quic_bind_address() -> String {
    "0.0.0.0:7965".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CoordinatorIrohConfig {
    pub enable_signalling: bool,
    pub enable_relay: bool,
    pub http_bind_address: String,
    pub quic_bind_address: String,
    pub community_registry_url: Option<String>,
    pub share_in_registry: bool,
    pub idle_timeout_secs: Option<u64>,
    pub max_connections: Option<usize>,
}

impl Default for CoordinatorIrohConfig {
    fn default() -> Self {
        Self {
            enable_signalling: false,
            enable_relay: false,
            http_bind_address: default_iroh_http_bind_address(),
            quic_bind_address: default_iroh_quic_bind_address(),
            community_registry_url: None,
            share_in_registry: false,
            idle_timeout_secs: None,
            max_connections: None,
        }
    }
}

fn default_webrtc_signalling_bind_address() -> String {
    "0.0.0.0:7963".to_string()
}
fn default_webrtc_bootstrap_page_bind_address() -> String {
    "0.0.0.0:7962".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CoordinatorWebRtcConfig {
    pub enable_signalling: bool,
    pub enable_relay: bool,
    pub signalling_bind_address: String,
    pub bootstrap_page_bind_address: String,
    pub external_host: Option<String>,
}

impl Default for CoordinatorWebRtcConfig {
    fn default() -> Self {
        Self {
            enable_signalling: false,
            enable_relay: false,
            signalling_bind_address: default_webrtc_signalling_bind_address(),
            bootstrap_page_bind_address: default_webrtc_bootstrap_page_bind_address(),
            external_host: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct TransportBridgeRole {
    pub translations: Vec<ProtocolTranslation>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProtocolTranslation {
    pub from: String,
    pub to: String,
}

const fn default_http_port() -> u16 {
    7960
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientGatewayRole {
    pub http_port: u16,
}

impl Default for ClientGatewayRole {
    fn default() -> Self {
        Self { http_port: default_http_port() }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ObservabilityRole {
    pub health: Option<EndpointConfig>,
    pub metrics: Option<EndpointConfig>,
    pub tracing: Option<TracingConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointConfig {
    pub enabled: bool,
    pub bind_address: String,
    pub endpoint: String,
}

fn default_service_name() -> String {
    "syneroym_substrate".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TracingConfig {
    pub enabled: bool,
    pub service_name: String,
    pub otlp: Option<OtlpConfig>,
    pub sampling: Option<SamplingConfig>,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            service_name: default_service_name(),
            otlp: None,
            sampling: Some(SamplingConfig::default()),
        }
    }
}

fn default_otlp_endpoint() -> String {
    "http://localhost:4318".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OtlpConfig {
    pub endpoint: String,
    pub protocol: OtlpProtocol,
}

impl Default for OtlpConfig {
    fn default() -> Self {
        Self { endpoint: default_otlp_endpoint(), protocol: Default::default() }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OtlpProtocol {
    #[default]
    Http,
    Grpc,
}

const fn default_sampling_ratio() -> f32 {
    0.1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SamplingConfig {
    pub strategy: SamplingStrategy,
    pub ratio: f32,
}

impl Default for SamplingConfig {
    fn default() -> Self {
        Self { strategy: Default::default(), ratio: default_sampling_ratio() }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplingStrategy {
    #[default]
    ParentBased,
    AlwaysOn,
    AlwaysOff,
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
    /// is explicitly dropped (Finding A5); the broker is reachable only
    /// in-process, via `Broker::link`.
    pub channel_capacity: u64,
}

impl Default for MessagingConfig {
    fn default() -> Self {
        Self { channel_capacity: default_mqtt_channel_capacity() }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn test_resolve_paths() {
        let mut config = SubstrateConfig {
            app_data_dir: PathBuf::from("/tmp/app_data"),
            app_local_data_dir: PathBuf::from("/tmp/local_data"),
            app_config_dir: PathBuf::from("/tmp/config"),
            ..Default::default()
        };

        config.identity.key = Some(PathBuf::from("substrate.key"));
        config.identity.agreement = Some(PathBuf::from("agreement.json"));
        config.storage.db_dir = PathBuf::from("db");

        config.roles.coordinator = Some(CoordinatorRole {
            tls: Some(TlsConfig {
                cert_path: PathBuf::from("cert.pem"),
                key_path: PathBuf::from("key.pem"),
            }),
            ..Default::default()
        });

        config.resolve_paths();

        assert_eq!(config.identity.key.unwrap(), Path::new("/tmp/app_data/substrate.key"));
        assert_eq!(config.identity.agreement.unwrap(), Path::new("/tmp/app_data/agreement.json"));
        assert_eq!(config.storage.db_dir, Path::new("/tmp/local_data/db"));
        assert_eq!(config.storage.blob_store.local_root, Path::new("/tmp/local_data/blob_objects"));

        let tls = config.roles.coordinator.unwrap().tls.unwrap();
        assert_eq!(tls.cert_path, Path::new("/tmp/config/cert.pem"));
        assert_eq!(tls.key_path, Path::new("/tmp/config/key.pem"));
    }

    #[test]
    fn test_blob_store_config_defaults() {
        let config = BlobStoreConfig::default();
        assert_eq!(config.backend, BlobBackend::Local);
        assert_eq!(config.local_root, Path::new("blob_objects"));
        assert_eq!(config.max_blob_bytes, 100 * 1024 * 1024);
        assert_eq!(config.max_service_total_bytes, None);
        assert!(config.s3.is_none());
    }

    #[test]
    fn test_messaging_config_defaults() {
        let config = MessagingConfig::default();
        assert_eq!(config.channel_capacity, 1024);
    }

    #[test]
    fn test_resolve_paths_absolute_untouched() {
        let mut config =
            SubstrateConfig { app_data_dir: PathBuf::from("/tmp/app_data"), ..Default::default() };
        let abs_path = if cfg!(windows) { "C:\\abs\\key" } else { "/abs/key" };
        config.identity.key = Some(PathBuf::from(abs_path));

        config.resolve_paths();

        assert_eq!(config.identity.key.unwrap(), Path::new(abs_path));
    }
}
