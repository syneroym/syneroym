use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

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

fn default_config_version() -> u32 {
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

    pub storage: StorageConfig,
    pub logging: LoggingConfig,

    pub uplink: UplinkConfig,

    pub profiles: HashMap<String, ProfileConfig>,

    pub roles: RolesConfig,
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
            storage: Default::default(),
            logging: Default::default(),
            uplink: Default::default(),
            profiles: Default::default(),
            roles: Default::default(),
        }
    }
}

fn default_db_dir() -> PathBuf {
    PathBuf::from("db")
}
fn default_blobs_dir() -> PathBuf {
    PathBuf::from("blobs")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub engine: StorageEngine,
    pub db_dir: PathBuf,
    pub blobs_dir: PathBuf,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            engine: Default::default(),
            db_dir: default_db_dir(),
            blobs_dir: default_blobs_dir(),
        }
    }
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
    #[default]
    Json,
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
    "http://localhost:3340".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IrohRelayConfig {
    pub relay_url: String,
}

impl Default for IrohRelayConfig {
    fn default() -> Self {
        Self { relay_url: default_relay_url() }
    }
}

fn default_signaling_server_url() -> String {
    "ws://localhost:7444".to_string()
}
fn default_bootstrap_page_url() -> String {
    "ws://localhost:7002".to_string()
}
fn default_stun_servers() -> Vec<String> {
    vec!["stun:stun.l.google.com:19302".to_string()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebRtcRelayConfig {
    pub signaling_server_url: String,
    pub bootstrap_page_url: String,
    pub stun_servers: Vec<String>,
}

impl Default for WebRtcRelayConfig {
    fn default() -> Self {
        Self {
            signaling_server_url: default_signaling_server_url(),
            bootstrap_page_url: default_bootstrap_page_url(),
            stun_servers: default_stun_servers(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UplinkConfig {
    pub iroh: Option<IrohRelayConfig>,
    pub webrtc: Option<WebRtcRelayConfig>,
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
    pub service_registry: Option<ServiceRegistryRole>,
    pub coordinator: Option<CoordinatorRole>,
    pub http_proxy: Option<HttpProxyRole>,
    pub observability: Option<ObservabilityRole>,
}

fn default_communication_interfaces() -> Vec<String> {
    vec!["iroh".to_string(), "webrtc".to_string()]
}
fn default_wrpc_sandbox() -> bool {
    true
}
fn default_cpu_limit() -> u32 {
    1
}
fn default_memory_limit() -> String {
    "1Gi".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSandboxRole {
    pub communication_interfaces: Vec<String>,
    pub wrpc_sandbox: bool,
    pub cpu_limit: u32,
    pub memory_limit: String,
}

impl Default for AppSandboxRole {
    fn default() -> Self {
        Self {
            communication_interfaces: default_communication_interfaces(),
            wrpc_sandbox: default_wrpc_sandbox(),
            cpu_limit: default_cpu_limit(),
            memory_limit: default_memory_limit(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceRegistryRole {
    pub access: AccessControl,
}

/// Represents configurations like `access = "everyone"` OR `access = ["did1", "did2"]`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AccessControl {
    String(String),
    List(Vec<String>),
}

impl Default for AccessControl {
    fn default() -> Self {
        AccessControl::String("everyone".to_string())
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
    "0.0.0.0:7443".to_string()
}
fn default_iroh_quic_bind_address() -> String {
    "0.0.0.0:7842".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CoordinatorIrohConfig {
    pub enable_signalling: bool,
    pub enable_relay: bool,
    pub http_bind_address: String,
    pub quic_bind_address: String,
}

impl Default for CoordinatorIrohConfig {
    fn default() -> Self {
        Self {
            enable_signalling: false,
            enable_relay: false,
            http_bind_address: default_iroh_http_bind_address(),
            quic_bind_address: default_iroh_quic_bind_address(),
        }
    }
}

fn default_webrtc_signalling_bind_address() -> String {
    "0.0.0.0:7444".to_string()
}
fn default_webrtc_bootstrap_page_bind_address() -> String {
    "0.0.0.0:7002".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CoordinatorWebRtcConfig {
    pub enable_signalling: bool,
    pub enable_relay: bool,
    pub signalling_bind_address: String,
    pub bootstrap_page_bind_address: String,
}

impl Default for CoordinatorWebRtcConfig {
    fn default() -> Self {
        Self {
            enable_signalling: false,
            enable_relay: false,
            signalling_bind_address: default_webrtc_signalling_bind_address(),
            bootstrap_page_bind_address: default_webrtc_bootstrap_page_bind_address(),
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

fn default_http_port() -> u16 {
    7000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpProxyRole {
    pub http_port: u16,
}

impl Default for HttpProxyRole {
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

fn default_sampling_ratio() -> f32 {
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
