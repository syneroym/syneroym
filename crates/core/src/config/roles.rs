use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::sandbox::{AppSandboxRole, PodmanSandboxRole, RoymRole};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RolesConfig {
    pub app_sandbox: Option<AppSandboxRole>,
    pub podman_sandbox: Option<PodmanSandboxRole>,
    pub community_registry: Option<ServiceRegistryRole>,
    pub coordinator: Option<CoordinatorRole>,
    pub client_gateway: Option<ClientGatewayRole>,
    pub auth: Option<AuthRole>,
    pub observability: Option<ObservabilityRole>,
    /// The App Supervisor (ADR-0021 §8). Absent = this node runs no
    /// supervisor.
    pub supervisor: Option<SupervisorRole>,
    /// Linked native Roym product role.
    pub roym: Option<RoymRole>,
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

fn default_supervisor_poll_interval_secs() -> u64 {
    30
}
fn default_supervisor_db_name() -> String {
    "supervisor.db".to_string()
}
const fn default_supervisor_max_restart_attempts() -> u32 {
    3
}
const fn default_supervisor_restart_backoff_secs() -> u64 {
    30
}
fn default_supervisor_alert_topic() -> String {
    "supervisor/alerts".to_string()
}
fn default_supervisor_master_backup_dir() -> String {
    "master-backups".to_string()
}
const fn default_supervisor_master_anchor_refresh_interval_secs() -> u64 {
    12 * 3600
}
const fn default_supervisor_renewed_cert_expires_hours() -> u64 {
    4
}
const fn default_supervisor_max_renewals_per_pass() -> u32 {
    5
}
/// Six times finer than `poll_interval_secs` (30): the queue's whole point
/// is convergence within one worker tick, not one poll interval. Finer
/// buys nothing -- the wait is for a substrate to come back, not for the
/// queue to notice.
const fn default_supervisor_queue_tick_secs() -> u64 {
    5
}
/// The primary bound on the outbox's attempt budget. Chosen, together with
/// `queue_max_backoff_secs`, so the combined window covers roughly a
/// 10-hour outage. Must outlast a human noticing an outage, not a
/// transient socket error.
const fn default_supervisor_queue_max_attempts() -> u8 {
    54
}
/// The ceiling the outbox's backoff curve settles at (15 minutes). Initial
/// backoff and multiplier stay `RetryPolicy`'s own defaults (100 ms, x2), so
/// the first few retries are fast -- a substrate that blipped is served in
/// under a second.
const fn default_supervisor_queue_max_backoff_secs() -> u64 {
    900
}
/// Four times `DEFAULT_PROXY_CALL_TIMEOUT` (30s), which bounds a single
/// delivery attempt. Too short re-delivers work still in flight; too long
/// strands a crashed worker's item for no reason.
const fn default_supervisor_queue_visibility_timeout_secs() -> u64 {
    120
}
/// Dead letters are pruned oldest-first on every write past this count -- a
/// bound and a trigger, not an adjective.
const fn default_supervisor_queue_dlq_max_rows() -> u32 {
    1000
}
/// One hour balances two things a signed topology document (ADR-0022 §3)
/// trades off: comfortably longer than any ordinary restart (the window a
/// caller with a cached document keeps routing while this supervisor is
/// down -- the availability property the document form exists for), and
/// far shorter than the Tier-1 record's own 30-day backstop, which answers
/// a slower question.
const fn default_supervisor_topology_document_not_after_secs() -> u64 {
    3_600
}
/// Five minutes: twelve re-asks inside one document's life, each a no-op
/// if nothing changed. Advice carried inside the signed document as
/// `cache_ttl_ms`, not authority -- the signer owns `not_after`, and a
/// reader may substitute its own TTL.
const fn default_supervisor_topology_document_cache_ttl_secs() -> u64 {
    300
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SupervisorRole {
    /// Reconcile + health sweep interval. Used by the resident loop; this
    /// role serves RPC only and sweeps on demand inside `status`.
    pub poll_interval_secs: u64,
    /// Desired state, journal, and alerts, under `app_data_dir`.
    pub db_name: String,
    /// Bounded remediation ceiling for the resident loop's restart policy.
    pub max_restart_attempts: u32,
    pub restart_backoff_secs: u64,
    /// MQTT topic prefix for published alerts.
    pub alert_topic: String,
    /// Where `export-master` writes and `import-master` reads. Operator-
    /// declared, never caller-supplied: the verbs take a master *name*, not
    /// a path, so no caller can steer a private key to a location of its
    /// choosing or read one from outside this directory. Relative to
    /// `app_data_dir`.
    pub master_backup_dir: String,
    /// How often the resident loop republishes each managed master's
    /// anchor. An anchor stops verifying at every consumer 24 hours after
    /// it was signed, so the default (12 hours) leaves 2x margin inside
    /// that window. Not a second timer: each ordinary pass compares this
    /// against a persisted "last refreshed" fact and only republishes when
    /// overdue.
    pub master_anchor_refresh_interval_secs: u64,
    /// The lifetime the supervisor mints instance certificates at -- both
    /// the first one at deploy and every unattended renewal, so a managed
    /// member has one certificate lifetime for its whole life. Short by
    /// design (4 hours): with renewal automated, a short-lived certificate
    /// costs nothing operationally and bounds what a leaked instance key is
    /// worth. Distinct from `roymctl`'s own attended-posture
    /// `--expires-hours` default, which is unaffected.
    ///
    /// **The operational cost, stated plainly:** the supervisor's vault is
    /// locked after every restart (the KEK arrives by `inject-kek` and does
    /// not survive one), and nothing renews while it is locked. So this
    /// number is also the window an operator has to re-inject the KEK
    /// before managed members start failing handshakes closed -- and the
    /// real window is shorter, since a member expires this long after *its
    /// own last renewal*, not after the restart. Between roughly a quarter
    /// of this value and all of it, depending where in the cycle the
    /// restart lands. The `VaultLocked` alert is what surfaces it.
    pub renewed_cert_expires_hours: u64,
    /// Ceiling on how many members one pass renews. Renewal is the one
    /// work-list whose arrivals are correlated by construction -- every
    /// member of an instance is minted in the same call at the same
    /// lifetime, so a whole instance reaches its near-expiry window in the
    /// same pass, every cycle. Uncapped, a large instance would hold the
    /// per-instance lock through N sequential mint/install/restart cycles,
    /// delaying every other write for that instance. Candidates not taken
    /// this pass are simply taken on the next one; the near-expiry window
    /// is wide relative to the pass interval, so nothing is at risk.
    pub max_renewals_per_pass: u32,
    /// The durable outbox worker's own tick, independent of
    /// `poll_interval_secs`. Recovery after a target
    /// returns is measured against this, not against the resident loop's
    /// poll interval -- see `default_supervisor_queue_tick_secs`'s doc.
    pub queue_tick_secs: u64,
    /// The outbox's attempt budget before an item dead-letters. See
    /// `default_supervisor_queue_max_attempts`'s doc for the arithmetic
    /// this and `queue_max_backoff_secs` together produce.
    pub queue_max_attempts: u8,
    /// The ceiling the outbox's backoff curve settles at.
    pub queue_max_backoff_secs: u64,
    /// How long a claimed outbox item stays invisible to a second claim
    /// before a crashed worker's hold on it is assumed gone.
    pub queue_visibility_timeout_secs: u64,
    /// Dead letters are pruned oldest-first past this row count.
    pub queue_dlq_max_rows: u32,
    /// How long a signed topology document (ADR-0022 §3) stays usable
    /// after it is signed. This is the window a caller with a cached
    /// document keeps routing while this supervisor is down -- the
    /// availability property the document form exists for -- and equally
    /// the window a caller may act on a member set this supervisor has
    /// already changed. One hour balances the two: comfortably longer than
    /// any restart, far shorter than the Tier-1 record's own 30-day
    /// backstop, which answers a slower question.
    pub topology_document_not_after_secs: u64,
    /// What a fetching caller is told to re-ask on, carried inside the
    /// signed document as `cache_ttl_ms`. Advice, not authority -- the
    /// signer owns `not_after`, and a reader may substitute its own TTL --
    /// but the supervisor is the only party that knows how often this
    /// app's topology actually moves, so it is the right party to advise.
    /// Five minutes: twelve re-asks inside one document's life, each of
    /// which is a no-op if nothing changed.
    pub topology_document_cache_ttl_secs: u64,
}

impl Default for SupervisorRole {
    fn default() -> Self {
        Self {
            poll_interval_secs: default_supervisor_poll_interval_secs(),
            db_name: default_supervisor_db_name(),
            max_restart_attempts: default_supervisor_max_restart_attempts(),
            restart_backoff_secs: default_supervisor_restart_backoff_secs(),
            alert_topic: default_supervisor_alert_topic(),
            master_backup_dir: default_supervisor_master_backup_dir(),
            master_anchor_refresh_interval_secs:
                default_supervisor_master_anchor_refresh_interval_secs(),
            renewed_cert_expires_hours: default_supervisor_renewed_cert_expires_hours(),
            max_renewals_per_pass: default_supervisor_max_renewals_per_pass(),
            queue_tick_secs: default_supervisor_queue_tick_secs(),
            queue_max_attempts: default_supervisor_queue_max_attempts(),
            queue_max_backoff_secs: default_supervisor_queue_max_backoff_secs(),
            queue_visibility_timeout_secs: default_supervisor_queue_visibility_timeout_secs(),
            queue_dlq_max_rows: default_supervisor_queue_dlq_max_rows(),
            topology_document_not_after_secs: default_supervisor_topology_document_not_after_secs(),
            topology_document_cache_ttl_secs: default_supervisor_topology_document_cache_ttl_secs(),
        }
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
    /// Path to a `CapabilityToken` granting `supervisor/resolve` on apps
    /// supervised by *other* nodes -- the WebRTC coordinator's own copy of
    /// `ClientGatewayRole::resolve_ucan`. Same default, same warning shape.
    #[serde(default)]
    pub resolve_ucan: Option<PathBuf>,
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

pub const fn default_session_ttl_secs() -> u64 {
    8 * 3600
}

fn default_auth_nonce_ttl_secs() -> u64 {
    60
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdentityMode {
    #[default]
    Open,
    Login,
    Fixed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientGatewayRole {
    pub http_port: u16,
    /// Path to a `CapabilityToken` granting `supervisor/resolve` on apps
    /// supervised by *other* nodes. Not needed for apps
    /// supervised by this node -- `[iam].grant_resolve_to_node_did`
    /// covers those. Absent, with that gate off too, means every logical
    /// hostname is refused by the supervisor it reaches; a startup
    /// warning names both keys. Unscoped (`-s` only) hostnames are
    /// unaffected either way.
    #[serde(default)]
    pub resolve_ucan: Option<PathBuf>,
    /// Identity mode for proxied client traffic: `open`, `login`, or `fixed`.
    #[serde(default)]
    pub identity_mode: IdentityMode,
    /// Person master DID injected on all proxied traffic in `fixed` mode.
    #[serde(default)]
    pub fixed_identity_did: Option<String>,
    /// Path to a delegation certificate injected on all proxied traffic in
    /// `fixed` mode.
    #[serde(default)]
    pub fixed_delegation: Option<PathBuf>,
    /// Optional connection gate in `login` mode: reject connections without a
    /// valid session with 401.
    #[serde(default)]
    pub connection_auth_gate: bool,
}

impl Default for ClientGatewayRole {
    fn default() -> Self {
        Self {
            http_port: default_http_port(),
            resolve_ucan: None,
            identity_mode: IdentityMode::Open,
            fixed_identity_did: None,
            fixed_delegation: None,
            connection_auth_gate: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthRole {
    /// Lifetime in seconds of minted session tokens.
    #[serde(default = "default_session_ttl_secs")]
    pub session_ttl_secs: u64,
    /// Lifetime in seconds of challenge nonces.
    #[serde(default = "default_auth_nonce_ttl_secs")]
    pub nonce_ttl_secs: u64,
    /// Optional path to the auth service's private key file.
    #[serde(default)]
    pub key_path: Option<PathBuf>,
    /// Directory containing person keys for the `local` login method.
    /// When unset, the `local` method is disabled.
    #[serde(default)]
    pub person_identities_dir: Option<PathBuf>,
    /// Additional origins permitted for CORS requests. Localhost patterns
    /// (`localhost`, `127.0.0.1`, `*.localhost`) are always allowed.
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    /// Whether to mark session cookies with `; Secure`.
    #[serde(default)]
    pub secure_cookies: bool,
}

impl Default for AuthRole {
    fn default() -> Self {
        Self {
            session_ttl_secs: default_session_ttl_secs(),
            nonce_ttl_secs: default_auth_nonce_ttl_secs(),
            key_path: None,
            person_identities_dir: None,
            allowed_origins: Vec::new(),
            secure_cookies: false,
        }
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
