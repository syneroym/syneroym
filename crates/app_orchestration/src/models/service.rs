use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::identifiers::{
    AppBlueprintId, AppInstanceId, InterfaceName, LogicalServiceName, PlacementSelector,
};
pub use crate::{resolver::ShardingStrategy, schedule::ScheduleSpec};

/// Supported service execution types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceType {
    Wasm,
    Container,
    Tcp,
    #[serde(rename = "nativehost")]
    NativeHost,
}

/// Service topology deployment modes.
#[derive(
    Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum TopologyMode {
    #[default]
    Singleton,
    Redundant,
    Sharded,
}

#[derive(
    Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum RotationPolicy {
    #[default]
    RestartOnRotation,
    None,
}

/// Default readiness-probe timeout, in milliseconds. Short on purpose: a
/// probe is a liveness question, and a slow answer is already a bad one.
pub const DEFAULT_PROBE_TIMEOUT_MS: u32 = 2_000;

const fn default_probe_timeout_ms() -> u32 {
    DEFAULT_PROBE_TIMEOUT_MS
}

const fn default_expect_status() -> u16 {
    200
}

/// Author-declared readiness probe (ADR-0021 §7's active signal, applied to a
/// service the supervisor manages rather than a bound external one).
///
/// Absent means **liveness only**: the substrate reports whether the instance
/// is running and nothing more. Present means the substrate additionally runs
/// this probe and reports its outcome as a distinct signal, because
/// remediation differs between "not running" and "running but not ready".
/// For a `tcp` service, where the process runs outside the substrate
/// entirely, this is the *only* evidence of liveness there is.
///
/// Externally tagged, one struct per variant -- the shape `PlacementSelector`
/// already proved round-trips through TOML and JSON while nested inside a
/// `#[serde(flatten)]`ed `ServiceConfig`. An internally tagged (`tag = "kind"`)
/// enum reads better in TOML but has no such proof under `flatten`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HealthCheck {
    /// Open a TCP connection to the host:port `interface` is registered on.
    /// Valid for `tcp` and `container` services.
    TcpConnect(TcpProbe),
    /// HTTP GET `path` against the host:port `interface` is registered on.
    /// Valid for `tcp` and `container` services.
    HttpGet(HttpProbe),
    /// Invoke `method` on `interface` in the deployed component. Valid for
    /// `wasm` services. Any non-error return is a pass -- the probe asks
    /// whether the guest can run, not what it answers.
    Rpc(RpcProbe),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TcpProbe {
    pub interface: InterfaceName,
    #[serde(default = "default_probe_timeout_ms")]
    pub timeout_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpProbe {
    pub interface: InterfaceName,
    pub path: String,
    #[serde(default = "default_expect_status")]
    pub expect_status: u16,
    #[serde(default = "default_probe_timeout_ms")]
    pub timeout_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcProbe {
    pub interface: InterfaceName,
    pub method: String,
    #[serde(default = "default_probe_timeout_ms")]
    pub timeout_ms: u32,
}

impl HealthCheck {
    /// The service types this probe kind can address. Read by the
    /// deploy-time validation, so a manifest error surfaces at deploy rather
    /// than as a permanently `failing` probe.
    #[must_use]
    pub const fn valid_for(&self) -> &'static [ServiceType] {
        match self {
            Self::TcpConnect(_) | Self::HttpGet(_) => &[ServiceType::Tcp, ServiceType::Container],
            Self::Rpc(_) => &[ServiceType::Wasm],
        }
    }

    #[must_use]
    pub fn interface(&self) -> &InterfaceName {
        match self {
            Self::TcpConnect(p) => &p.interface,
            Self::HttpGet(p) => &p.interface,
            Self::Rpc(p) => &p.interface,
        }
    }

    /// Kebab-case name of the variant, for error messages.
    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::TcpConnect(_) => "tcp-connect",
            Self::HttpGet(_) => "http-get",
            Self::Rpc(_) => "rpc",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceQuota {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_instructions: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_bytes: Option<u64>,
}

/// Shared execution configuration for a service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceConfig {
    pub service_type: ServiceType,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<InterfaceName>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_config: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota: Option<ResourceQuota>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<DocumentRef>,
    #[serde(default)]
    pub rotation_policy: RotationPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fdae: Option<FdaeManifest>,
    /// Author-declared readiness probe. Absent = liveness only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check: Option<HealthCheck>,
    /// Static assets served directly from blob storage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<AssetBundle>,
    /// Whether this service's endpoint record is published, and how far it
    /// travels (ADR-0018 §1). Declared, never inferred from whether a
    /// certificate was supplied. Absent means `private`: publication is a
    /// privacy decision, and a default of `public` would preserve the exact
    /// accident of publishing because someone held a key.
    #[serde(default)]
    pub visibility: Visibility,
}

/// Author-side declaration of a deploy-time document.
///
/// A bare string is read by the client and shipped inline -- the same thing a
/// bare `source` already means for a Wasm component, and what makes a deploy
/// work against a substrate with nothing pre-staged. `{ remote_path = "..." }`
/// defers resolution to the substrate host instead, for large or shared assets
/// and operator-managed directories.
///
/// A relative bare path resolves against the **client process's working
/// directory**, not the manifest's own directory, matching `source` exactly
/// (`util::read_local_artifact`). Rebasing both onto the manifest's parent
/// would be friendlier, but doing it for documents alone would leave two
/// sibling manifest fields resolving differently, which is worse than either
/// rule on its own.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum DocumentRef {
    Local(String),
    Remote { remote_path: String },
}

/// Optional declarative ReBAC policy for this service (ADR-0017 §1).
/// `#[serde(default)]` on the field above keeps every existing manifest
/// parsing unchanged -- a service with no policy is unfiltered, which is
/// the policy layer's default-*absent* (ADR-0017 §2.1).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FdaeManifest {
    pub policy: DocumentRef,
}

/// Endpoint/asset visibility (ADR-0018). Defaults to the most private.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    Internal,
    #[default]
    Private,
}

impl Visibility {
    /// The wire/display form, matching the `#[serde(rename_all = "lowercase")]`
    /// encoding above -- one definition instead of a hand-written match
    /// repeated at every call site that needs to print or log this value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Private => "private",
        }
    }
}

/// Who may fetch a logical service's Tier-2 topology document (ADR-0022 §5).
///
/// Binary by construction, not three-valued like [`Visibility`]: a topology
/// document is never registered anywhere, so `internal`'s "registered here,
/// not propagated" has nothing to mean. ADR-0022 §5 also forbids a filtered
/// member list -- a caller receives the whole member set and mode, or a clean
/// denial -- so there is no third answer to express.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TopologyVisibility {
    /// The caller must hold `supervisor/resolve` on `synapp:<app-did>`.
    /// Today's behaviour, and the default: access is asked for, not assumed.
    #[default]
    Restricted,
    /// Any verified caller may fetch this service's topology document, with
    /// no capability and no pre-installed token.
    Open,
}

pub(crate) fn is_restricted(v: &TopologyVisibility) -> bool {
    matches!(v, TopologyVisibility::Restricted)
}

/// Author-side declaration of a static asset bundle.
///
/// `archive` resolves **against the manifest's own directory** when it is a
/// bare relative path -- deliberately *not* `ServiceConfig::source`'s rule,
/// which resolves against the client process's cwd. Two rules already exist
/// in the tree (`mapper` uses cwd, `roymctl supervisor submit` uses
/// `manifest_dir`); a new field picks the one that is not surprising and
/// says so, rather than claiming a single rule exists.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssetBundle {
    pub archive: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(default)]
    pub visibility: Visibility,
}

/// A `replicas` value above this is refused at `validate()`: a bound set
/// before the first measurement can never fail. 16
/// members of one service on one node is already past what a single
/// substrate's per-service database and instance-certificate budget make
/// sensible, and per-member placement -- the reason to want more -- does
/// not exist.
pub const MAX_REPLICAS: u32 = 16;

const fn default_replicas() -> u32 {
    1
}

fn is_one_u32(n: &u32) -> bool {
    *n == 1
}

/// Represents the spec of a service in the application manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceSpec {
    #[serde(flatten)]
    pub config: ServiceConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<LogicalServiceName>,
    /// Overrides the manifest-level default for this service only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<PlacementSelector>,
    /// How many members the compiler emits for this service. `1` (the
    /// default) compiles exactly what every manifest written before
    /// `replicas` existed still compiles. Above `1`, the
    /// compiled `topology_mode` becomes `Redundant` -- `Sharded` stays
    /// unreachable until a `ShardingStrategy` manifest surface exists.
    /// `#[serde(skip_serializing_if)]` so an unscaled manifest's TOML/JSON
    /// is byte-for-byte unchanged.
    #[serde(default = "default_replicas", skip_serializing_if = "is_one_u32")]
    pub replicas: u32,
    /// Which sub-strategy a `Sharded` selection uses (ADR-0022 §6). Read by
    /// nothing yet: the compiler never emits `TopologyMode::Sharded` today
    /// (`replicas` alone only ever produces `Redundant`), and shard
    /// rebalancing -- the first actual consumer -- is later work.
    /// Declared now anyway, on the same reasoning `replicas` itself
    /// was added under: a manifest field is free to add before anything
    /// depends on the format, and expensive after.
    /// `#[serde(skip_serializing_if)]` so a manifest that never mentions it
    /// is byte-for-byte unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharding_strategy: Option<ShardingStrategy>,
    /// When and what to run on exactly one member of this logical service
    /// (ADR-0023 §6). Absent means unscheduled, which is
    /// every manifest that exists before this field. Lives here and on
    /// `PlannedService`, never inside `ServiceConfig`: `ServiceConfig` maps
    /// onto the substrate-side deploy manifest, and the substrate dedups an
    /// `apply_plan` by content hash over it -- a schedule the substrate has
    /// no use for would make editing a cron string restart the service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<ScheduleSpec>,
    /// Who may fetch this logical service's topology document (ADR-0022 §5).
    /// Part of the desired state, so it survives a supervisor handover --
    /// node-local supervisor config would be neither reproducible nor
    /// portable. Absent means `restricted`, which is what every manifest
    /// written before this field already means.
    #[serde(default, skip_serializing_if = "is_restricted")]
    pub topology_visibility: TopologyVisibility,
}

/// Defines a dependency on another application.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum AppDependencySpec {
    Spawn { blueprint: AppBlueprintId, manifest_path: Option<String> },
    Bind { instance: AppInstanceId },
}
