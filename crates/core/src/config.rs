//! Configuration types for the Syneroym substrate.

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::util;

pub const DEFAULT_SUBSTRATE_KEY_FILE: &str = "substrate.key";
/// Implicitly discovered under `app_data_dir` when `[identity].agreement`
/// is unset -- `roymctl substrate claim`'s default output path, so claiming
/// a node and restarting it establishes ownership with no config edit.
pub const DEFAULT_CONTROLLER_AGREEMENT_FILE: &str = "agreement.json";

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
    /// Bidirectional stream protocols (M3B Slice 6B, ADR-0014). A core,
    /// always-on capability, mirroring `mqtt`'s placement above.
    pub streaming: StreamingConfig,
    /// Identity/capability admission (M04A Slice B0, ADR-0015/0016).
    pub iam: IamConfig,
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
            streaming: Default::default(),
            iam: Default::default(),
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
    /// The App Supervisor (ADR-0021 §8). Absent = this node runs no
    /// supervisor, which is every deployment through A4.
    pub supervisor: Option<SupervisorRole>,
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
const fn default_dispatch_epoch_timeout_secs() -> u64 {
    5
}
const fn default_lifecycle_hook_epoch_timeout_secs() -> u64 {
    30
}
const fn default_abac_max_instructions() -> u64 {
    50_000_000
}
const fn default_abac_epoch_timeout_secs() -> u64 {
    2
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
    /// Wall-clock budget (Wasmtime epoch interruption) for an ordinary
    /// dispatch call -- RPC/proxy invocation, message delivery, or one
    /// streaming chunk. Tight by design: this is the hot path a stuck or
    /// hostile guest would otherwise hang forever.
    pub dispatch_epoch_timeout_secs: u64,
    /// Wall-clock budget for a component's `init()`/`migrate()` lifecycle
    /// hook (`AppSandboxEngine::invoke_lifecycle_hook`, called once per
    /// deploy). Deliberately larger than `dispatch_epoch_timeout_secs`: this
    /// hook does real one-time work (e.g. `store::create_collection`
    /// opening the service's SQLCipher DB), not a hot path repeatedly hit by
    /// a request, so a generous budget doesn't trade away the same
    /// protection the tighter dispatch budget buys.
    pub lifecycle_hook_epoch_timeout_secs: u64,
    /// Fuel ceiling for one stage-4 ABAC after-step invocation (ADR-0017 §7's
    /// "fuel-metered"). Deliberately a small fraction of
    /// `default_max_instructions`: the after-step runs once per read on the
    /// hot path, and §7's optional read-only lookups are the thing this
    /// bounds. Overrun denies the whole batch, never returns partially-
    /// checked rows. A starting point, to be re-tuned against a measured
    /// `criterion` bench.
    ///
    /// Deliberately **not** `Option<u64>` (review finding B4-07): the
    /// after-step always overrides the service's own fuel via
    /// `InstanceOptions::fuel_override`, which treats `None` as "keep
    /// whatever the caller already had" -- for every *other*
    /// `fuel_override` use that means "no override", but here it would
    /// silently fall through to the service's own `default_max_instructions`
    /// (10 billion by default, ~200x this field's own default), the exact
    /// opposite of what an operator clearing this field to disable a limit
    /// would expect. A plain `u64` makes that fallback unreachable.
    pub abac_max_instructions: u64,
    /// Wall-clock budget for one after-step. Tighter than
    /// `dispatch_epoch_timeout_secs` for the same reason: the after-step
    /// runs on the hot read path, not once per deploy. A starting point, to
    /// be re-tuned against a measured `criterion` bench.
    pub abac_epoch_timeout_secs: u64,
    /// The guest proxy outbox worker's own tick. Recovery after an
    /// unreachable target returns is bounded by this, and nothing finer
    /// helps when the wait is for a peer to come back.
    pub queue_tick_secs: u64,
    /// The outbox's attempt budget before an item dead-letters. See
    /// [`default_proxy_queue_max_attempts`] for the ~10-hour window this
    /// and `queue_max_backoff_secs` together produce.
    pub queue_max_attempts: u8,
    /// The ceiling the outbox's backoff curve settles at. The early
    /// retries stay sub-second, so a peer that only blipped is served
    /// immediately.
    pub queue_max_backoff_secs: u64,
    /// How long a claimed outbox item stays invisible to a second claim
    /// before a crashed worker's hold on it is assumed gone.
    pub queue_visibility_timeout_secs: u64,
    /// Dead letters are pruned oldest-first past this row count, within
    /// one target: a permanently broken recipient must not be able to
    /// evict every other conversation's dead letters.
    pub queue_dlq_max_rows: u32,
    /// How many sagas one service may have open at once. Refuses `begin`
    /// above it: an open saga is work somebody expects to finish, so the
    /// bound refuses rather than evicts.
    pub saga_max_open: u32,
    /// How many steps one saga may record. Refuses `step` above it, for the
    /// same reason as `saga_max_open`.
    pub saga_max_steps: u32,
    /// Terminal (`compensated`/`failed`) saga rows are pruned oldest-first
    /// past this count, exactly as `queue_dlq_max_rows` prunes dead letters.
    pub saga_max_terminal_rows: u32,
    /// A `begin` with no explicit deadline takes this many seconds.
    pub saga_default_deadline_secs: u64,
    /// The ceiling a guest may request at `begin`. Above it, `begin` refuses
    /// rather than clamping -- a workflow must not silently run under a
    /// deadline it did not ask for.
    pub saga_max_deadline_secs: u64,
}

/// The guest proxy outbox lives wherever a guest does, so its knobs live on
/// the sandbox role rather than on the supervisor's -- a substrate hosting
/// guests may run no supervisor at all.
const fn default_proxy_queue_tick_secs() -> u64 {
    5
}
/// 54 attempts with a 100 ms initial backoff, x2 multiplier and a 900 s
/// ceiling sum to roughly 10.2 hours of retrying. A message queued at 22:00
/// must still be deliverable at 07:00.
const fn default_proxy_queue_max_attempts() -> u8 {
    54
}
const fn default_proxy_queue_max_backoff_secs() -> u64 {
    900
}
/// Four times the proxy's own 30 s per-call budget, which bounds a single
/// delivery attempt. Too short re-delivers work still in flight; too long
/// strands a crashed worker's item for no reason.
const fn default_proxy_queue_visibility_timeout_secs() -> u64 {
    120
}
const fn default_proxy_queue_dlq_max_rows() -> u32 {
    1000
}
/// One workflow per open saga; a service with 64 in flight has a design
/// problem, not a capacity one.
const fn default_saga_max_open() -> u32 {
    64
}
const fn default_saga_max_steps() -> u32 {
    64
}
/// The same number `queue_dlq_max_rows` uses, for the same
/// operator-visibility reason.
const fn default_saga_max_terminal_rows() -> u32 {
    1000
}
/// An hour: long enough for a human-paced multi-provider workflow, short
/// enough that a crashed one compensates the same day.
const fn default_saga_default_deadline_secs() -> u64 {
    3600
}
/// A day. Both are honest first guesses, not measurements -- config fields,
/// so a deployment that finds them wrong changes them without a rebuild.
const fn default_saga_max_deadline_secs() -> u64 {
    86400
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
            dispatch_epoch_timeout_secs: default_dispatch_epoch_timeout_secs(),
            lifecycle_hook_epoch_timeout_secs: default_lifecycle_hook_epoch_timeout_secs(),
            abac_max_instructions: default_abac_max_instructions(),
            abac_epoch_timeout_secs: default_abac_epoch_timeout_secs(),
            queue_tick_secs: default_proxy_queue_tick_secs(),
            queue_max_attempts: default_proxy_queue_max_attempts(),
            queue_max_backoff_secs: default_proxy_queue_max_backoff_secs(),
            queue_visibility_timeout_secs: default_proxy_queue_visibility_timeout_secs(),
            queue_dlq_max_rows: default_proxy_queue_dlq_max_rows(),
            saga_max_open: default_saga_max_open(),
            saga_max_steps: default_saga_max_steps(),
            saga_max_terminal_rows: default_saga_max_terminal_rows(),
            saga_default_deadline_secs: default_saga_default_deadline_secs(),
            saga_max_deadline_secs: default_saga_max_deadline_secs(),
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
/// is convergence within one worker tick, not one poll interval (M05B
/// D-B1-13, task.md's performance budget). Finer buys nothing -- the wait is
/// for a substrate to come back, not for the queue to notice.
const fn default_supervisor_queue_tick_secs() -> u64 {
    5
}
/// The primary bound on the outbox's attempt budget. Chosen, together with
/// `queue_max_backoff_secs`, so the combined window covers roughly a
/// 10-hour outage -- see the M05B B1 plan §0.12 for the arithmetic. Must
/// outlast a human noticing an outage, not a transient socket error.
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
    /// `poll_interval_secs` (M05B B1, D-B1-13). Recovery after a target
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
    /// `ClientGatewayRole::resolve_ucan` (D-S3-6). Same default, same
    /// warning shape.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientGatewayRole {
    pub http_port: u16,
    /// Path to a `CapabilityToken` granting `supervisor/resolve` on apps
    /// supervised by *other* nodes (D-S3-6). Not needed for apps
    /// supervised by this node -- `[iam].grant_resolve_to_node_did`
    /// covers those. Absent, with that gate off too, means every logical
    /// hostname is refused by the supervisor it reaches; a startup
    /// warning names both keys. Unscoped (`-s` only) hostnames are
    /// unaffected either way.
    #[serde(default)]
    pub resolve_ucan: Option<PathBuf>,
}

impl Default for ClientGatewayRole {
    fn default() -> Self {
        Self { http_port: default_http_port(), resolve_ucan: None }
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

const fn default_max_concurrent_streams_per_service() -> u32 {
    8
}

/// M3B Slice 6B bidirectional streaming (ADR-0014). Each open stream holds a
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

/// Identity/capability admission (M04A Slices B0 + B1). A caller whose
/// verified DID equals `admin_ucan_root` is granted `substrate/admin`
/// directly (B0). B1 additionally roots UCAN chain verification here: any
/// `CapabilityToken` chain presented at ingress must attenuate back to a
/// token issued by this same DID to be admitted (`build_caller`,
/// `crates/router/src/route_handler/io.rs`) -- owner-rooted *service*
/// capability chains (owner != node admin) are not yet verifiable (Slice
/// B7).
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
    /// ability `supervisor/resolve`, node-wide (ADR-0022 §7, D-S3-6).
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
    #[serde(default)]
    pub grant_resolve_to_node_did: bool,
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
    fn test_streaming_config_defaults() {
        let config = StreamingConfig::default();
        assert_eq!(config.max_concurrent_streams_per_service, 8);
    }

    /// M05A A5d: the anchor-refresh cadence has to sit comfortably inside
    /// the 24-hour window after which an anchor stops verifying at every
    /// consumer -- a refresh interval at or above that window would let
    /// anchors lapse between passes no matter how reliably the loop runs.
    #[test]
    fn supervisor_role_master_anchor_refresh_interval_secs_has_a_day_scale_default() {
        const ANCHOR_VALIDITY_SECS: u64 = 24 * 3600;
        let role = SupervisorRole::default();
        assert!(
            role.master_anchor_refresh_interval_secs * 2 <= ANCHOR_VALIDITY_SECS,
            "the refresh interval ({}s) must leave at least 2x margin inside the anchor's \
             {ANCHOR_VALIDITY_SECS}s validity window",
            role.master_anchor_refresh_interval_secs
        );
        assert!(
            role.master_anchor_refresh_interval_secs >= 3600,
            "and must not be so short that a fact needing to move once a day is republished every \
             few minutes"
        );
    }

    /// ADR-0022 §2: an app instance's Tier-1 record reuses this same
    /// interval (no second config field), against `EndpointInfo`'s own
    /// 30-day `not_after`, not the anchor's 24-hour one -- the number that
    /// matters there is how many *consecutive failed* refreshes a
    /// previously published record survives before it lapses, not the
    /// interval alone. At the default 12h cadence against 30 days, sixty.
    #[test]
    fn tier1_refresh_survives_sixty_consecutive_failures_against_the_default_interval() {
        let role = SupervisorRole::default();
        let interval = role.master_anchor_refresh_interval_secs;
        let survivable = crate::dht_registry::DEFAULT_ENDPOINT_NOT_AFTER_SECS / interval;
        assert_eq!(
            survivable, 60,
            "sixty consecutive failed refreshes at the default cadence is the number an operator \
             is told they have before a locked vault costs discoverability"
        );
    }

    /// The supervisor's own certificate lifetime is short by design and is
    /// deliberately *not* `roymctl`'s attended-posture default (24h), which
    /// serves an operator with no renewal loop behind them.
    #[test]
    fn supervisor_role_renewed_cert_expires_hours_is_short_and_capped() {
        let role = SupervisorRole::default();
        assert!(role.renewed_cert_expires_hours < 24, "renewal makes short-lived the default");
        assert!(role.renewed_cert_expires_hours >= 1, "and it must still be a usable window");
        assert!(role.max_renewals_per_pass >= 1, "a cap of 0 would renew nothing, ever");
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
