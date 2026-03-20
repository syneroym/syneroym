use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstrateConfig {
    pub config_version: u32,

    pub app_config_dir: Option<PathBuf>,
    pub app_local_data_dir: Option<PathBuf>,
    pub app_data_dir: Option<PathBuf>,
    pub app_cache_dir: Option<PathBuf>,
    pub app_log_dir: Option<PathBuf>,

    pub profile: String,

    pub storage: StorageConfig,
    pub logging: LoggingConfig,

    #[serde(default)]
    pub relay: RelayConfig,

    #[serde(default)]
    pub uplink: UplinkConfig,

    #[serde(default)]
    pub profiles: HashMap<String, ProfileConfig>,

    pub roles: RolesConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub engine: StorageEngine,
    pub db_dir: PathBuf,
    pub blobs_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageEngine {
    Sqlite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: LogLevel,
    pub format: LogFormat,
    pub target: LogTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Json,
    Pretty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogTarget {
    Stdout,
    File,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RelayConfig {
    pub iroh: Option<IrohRelayConfig>,
    pub webrtc: Option<WebRtcRelayConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrohRelayConfig {
    pub enabled: bool,
    pub relay_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebRtcRelayConfig {
    pub enabled: bool,
    pub signaling_server_url: String,
    pub bootstrap_page_url: String,
    pub stun_servers: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UplinkConfig {
    pub ble: Option<BridgeConfig>,
    pub lora: Option<BridgeConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeConfig {
    pub enabled: bool,
    pub bridge: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileConfig {
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RolesConfig {
    pub app_sandbox: Option<AppSandboxRole>,
    pub service_registry: Option<ServiceRegistryRole>,
    pub coordinator: Option<CoordinatorRole>,
    pub transport_bridge: Option<TransportBridgeRole>,
    pub http_proxy: Option<HttpProxyRole>,
    pub observability: Option<ObservabilityRole>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSandboxRole {
    pub communication_interfaces: Vec<String>,
    pub wrpc_sandbox: bool,
    pub cpu_limit: u32,
    pub memory_limit: String, // Kept as String to hold values like "1Gi" without manual parsing logic
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CoordinatorRole {
    pub tls: Option<TlsConfig>,
    pub iroh: Option<CoordinatorIrohConfig>,
    pub webrtc: Option<CoordinatorWebRtcConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorIrohConfig {
    pub enabled: bool,
    pub http_bind_address: String,
    pub quic_bind_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorWebRtcConfig {
    pub enabled: bool,
    pub signalling_bind_address: String,
    pub bootstrap_page_bind_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportBridgeRole {
    pub enabled: bool,
    pub access: AccessControl,
    pub translations: Vec<ProtocolTranslation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolTranslation {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpProxyRole {
    pub http_port: u16,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TracingConfig {
    pub enabled: bool,
    pub service_name: String,
    pub otlp: Option<OtlpConfig>,
    pub sampling: Option<SamplingConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtlpConfig {
    pub endpoint: String,
    pub protocol: OtlpProtocol,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OtlpProtocol {
    Http,
    Grpc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingConfig {
    pub strategy: SamplingStrategy,
    pub ratio: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplingStrategy {
    ParentBased,
    AlwaysOn,
    AlwaysOff,
}
