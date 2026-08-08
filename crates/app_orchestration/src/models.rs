use std::{collections::BTreeMap, error, fmt, ops::Deref, str::FromStr};

use anyhow::{Result, anyhow};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::schedule::{MAX_SCHEDULE_TIMEOUT_MS, MAX_SCHEDULED_SERVICES};
pub use crate::{resolver::ShardingStrategy, schedule::ScheduleSpec};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl error::Error for ParseError {}

macro_rules! define_string_wrapper {
    ($name:ident, $doc:expr) => {
        define_string_wrapper!($name, $doc, |s: &str| {
            if s.is_empty() {
                Err(anyhow!("{} cannot be empty", stringify!($name)))
            } else {
                Ok(())
            }
        });
    };
    ($name:ident, $doc:expr, $validate:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn try_new<S: Into<String>>(s: S) -> Result<Self> {
                let s = s.into();
                let validator: fn(&str) -> Result<()> = $validate;
                validator(&s)?;
                Ok($name(s))
            }

            pub fn new<S: Into<String>>(s: S) -> Self {
                Self::try_new(s)
                    .unwrap_or_else(|e| panic!("Invalid value for {}: {}", stringify!($name), e))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl FromStr for $name {
            type Err = anyhow::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::try_new(s)
            }
        }

        impl TryFrom<String> for $name {
            type Error = anyhow::Error;

            fn try_from(s: String) -> Result<Self, Self::Error> {
                Self::try_new(s)
            }
        }

        impl From<$name> for String {
            fn from(wrapper: $name) -> Self {
                wrapper.0
            }
        }

        impl Deref for $name {
            type Target = String;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
    };
}

define_string_wrapper!(
    AppBlueprintId,
    "Unique identifier for an application blueprint/definition."
);
define_string_wrapper!(
    AppInstanceId,
    "Unique identifier for a running application instance.",
    |s: &str| {
        if s.is_empty() {
            return Err(anyhow!("AppInstanceId cannot be empty"));
        }
        if s.contains('/') {
            return Err(anyhow!("AppInstanceId cannot contain '/'"));
        }
        // M05A A5e (D-A5e-2/D-A5e-12): `#` is `MemberRef`'s own index
        // separator. Forbidding it here, on the instance-id half of the
        // boundary it splits on, is what closes the pre-existing
        // `member_master_name` collision between instance `a` + service
        // `b-c` and instance `a-b` + service `c` -- the last segment of a
        // valid `MemberRef` display form is always a bare `u32`, so only
        // the instance/service boundary needs the guard.
        if s.contains('#') {
            return Err(anyhow!("AppInstanceId cannot contain '#'"));
        }
        // M05A A7 review finding 5: `..` and `\` are valid here but not as
        // a vault backup name (`validate_backup_name`,
        // `crates/app_supervisor/src/keys.rs`) -- an instance id built
        // from one of these was refused only at `adopt`'s app-master mint,
        // by which point a services-less plan had already been accepted
        // by `submit`. Closed at construction instead: an id that can
        // never be backed up must never exist, not merely fail late and
        // permanently on the one verb (`adopt`) that is also the only way
        // back from `retired`.
        if s.contains("..") {
            return Err(anyhow!("AppInstanceId cannot contain '..'"));
        }
        if s.contains('\\') {
            return Err(anyhow!("AppInstanceId cannot contain '\\'"));
        }
        Ok(())
    }
);

define_string_wrapper!(
    LogicalServiceName,
    "Logical name of a service within an application.",
    |s: &str| {
        if s.is_empty() {
            return Err(anyhow!("LogicalServiceName cannot be empty"));
        }
        if s.contains('/') {
            return Err(anyhow!("LogicalServiceName cannot contain '/'"));
        }
        // M05A A5e (D-A5e-2): `#` is `MemberRef`'s own index separator --
        // forbidden here so a `MemberRef` display string can always be
        // parsed back by splitting on the last `#`, unambiguously.
        if s.contains('#') {
            return Err(anyhow!("LogicalServiceName cannot contain '#'"));
        }
        Ok(())
    }
);

define_string_wrapper!(
    ServiceId,
    "Physical identifier of a service (usually a did:key DID).",
    |s: &str| {
        if !s.starts_with("did:key:") {
            return Err(anyhow!("ServiceId must start with 'did:key:'"));
        }
        Ok(())
    }
);

define_string_wrapper!(
    AppDid,
    "An app instance's own master DID (ADR-0022 §1) -- its network identity, as distinct from \
     `AppInstanceId`, its human name.",
    |s: &str| {
        if !s.starts_with("did:key:") {
            return Err(anyhow!("AppDid must start with 'did:key:'"));
        }
        Ok(())
    }
);

define_string_wrapper!(InterfaceName, "Name of the interface a service implements.");
define_string_wrapper!(DependencyName, "Name of a dependency within an application.");

define_string_wrapper!(
    SubstrateAlias,
    "Operator-chosen name for a substrate in the deploy inventory.",
    |s: &str| {
        if s.is_empty() {
            return Err(anyhow!("SubstrateAlias cannot be empty"));
        }
        if s.contains('/') {
            return Err(anyhow!("SubstrateAlias cannot contain '/'"));
        }
        // Placement names an inventory alias, never a bare DID: an alias is
        // the indirection that lets one manifest deploy against different
        // operators' topologies, and a DID written here would defeat it.
        if s.starts_with("did:") {
            return Err(anyhow!(
                "SubstrateAlias '{s}' looks like a DID; placement names an inventory alias"
            ));
        }
        Ok(())
    }
);

/// How a service's hosting substrate is chosen.
///
/// One variant today. It is an enum rather than a bare alias so a later
/// pool- or constraint-based selector is an added variant instead of a
/// schema change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlacementSelector {
    /// Place on the substrate registered in the deploy inventory under this
    /// alias.
    Substrate(SubstrateAlias),
}

impl PlacementSelector {
    pub fn alias(&self) -> &SubstrateAlias {
        match self {
            Self::Substrate(alias) => alias,
        }
    }
}

/// Logical reference to a service, fully identifying it within a specific
/// application instance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct LogicalServiceRef {
    pub app_instance_id: AppInstanceId,
    pub service_name: LogicalServiceName,
}

impl fmt::Display for LogicalServiceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.app_instance_id, self.service_name)
    }
}

impl FromStr for LogicalServiceRef {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 2 {
            return Err(anyhow!("LogicalServiceRef must format as 'app_instance_id/service_name'"));
        }
        let app_instance_id = AppInstanceId::try_new(parts[0])?;
        let service_name = LogicalServiceName::try_new(parts[1])?;
        Ok(LogicalServiceRef { app_instance_id, service_name })
    }
}

impl TryFrom<String> for LogicalServiceRef {
    type Error = anyhow::Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::from_str(&s)
    }
}

impl From<LogicalServiceRef> for String {
    fn from(r: LogicalServiceRef) -> Self {
        r.to_string()
    }
}

/// Identifies one managed **member** of a logical service, as distinct from
/// the logical service itself (M05A A5e, D-A5e-2).
///
/// `LogicalServiceRef` is the key of a logical service -- what the resolver,
/// `TopologyEntry`, and a binding's dependency name are about. `replicas`
/// makes that key stop being unique for anything the supervisor stores or
/// reports per *managed unit*: a placement row, a binding epoch, a
/// restart-attempt counter, an alert, a `revoke-instance` argument. Those
/// sites key on `MemberRef` instead; `LogicalServiceRef` is unchanged and
/// keeps its own meaning.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MemberRef {
    pub logical_ref: LogicalServiceRef,
    pub index: u32,
}

impl fmt::Display for MemberRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.logical_ref, self.index)
    }
}

impl FromStr for MemberRef {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (l_ref_part, index_part) = s.rsplit_once('#').ok_or_else(|| {
            anyhow!("MemberRef must format as 'app_instance_id/service_name#index'")
        })?;
        let index = index_part
            .parse::<u32>()
            .map_err(|e| anyhow!("MemberRef index '{index_part}' is not a valid u32: {e}"))?;
        let logical_ref = LogicalServiceRef::from_str(l_ref_part)?;
        Ok(MemberRef { logical_ref, index })
    }
}

impl TryFrom<String> for MemberRef {
    type Error = anyhow::Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::from_str(&s)
    }
}

impl From<MemberRef> for String {
    fn from(r: MemberRef) -> Self {
        r.to_string()
    }
}

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
    /// The service types this probe kind can address (D-A4-6). Read by the
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
    /// Author-declared readiness probe (M05A A4). Absent = liveness only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check: Option<HealthCheck>,
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

/// A `replicas` value above this is refused at `validate()` (M05A A5e,
/// D-A5e-14): a bound set before the first measurement can never fail. 16
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
    /// How many members the compiler emits for this service (M05A A5e,
    /// D-A5e-4). `1` (the default) compiles exactly what every manifest
    /// written before `replicas` existed still compiles. Above `1`, the
    /// compiled `topology_mode` becomes `Redundant` -- `Sharded` stays
    /// unreachable until a `ShardingStrategy` manifest surface exists.
    /// `#[serde(skip_serializing_if)]` so an unscaled manifest's TOML/JSON
    /// is byte-for-byte unchanged.
    #[serde(default = "default_replicas", skip_serializing_if = "is_one_u32")]
    pub replicas: u32,
    /// Which sub-strategy a `Sharded` selection uses (ADR-0022 §6). Read by
    /// nothing yet: the compiler never emits `TopologyMode::Sharded` today
    /// (`replicas` alone only ever produces `Redundant`), and shard
    /// rebalancing -- the first actual consumer -- is a later milestone's
    /// work. Declared now anyway, on the same reasoning `replicas` itself
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
}

/// Defines a dependency on another application.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum AppDependencySpec {
    Spawn { blueprint: AppBlueprintId, manifest_path: Option<String> },
    Bind { instance: AppInstanceId },
}

/// Declarative manifest specifying the structure and dependencies of a SynApp.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SynAppManifest {
    pub id: AppBlueprintId,
    pub version: Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Default for every service this manifest declares, and for every
    /// spawned child manifest that declares none of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<PlacementSelector>,
    #[serde(default)]
    pub services: BTreeMap<LogicalServiceName, ServiceSpec>,
    #[serde(default)]
    pub dependencies: BTreeMap<DependencyName, AppDependencySpec>,
}

impl SynAppManifest {
    pub fn from_toml(s: &str) -> Result<Self> {
        let manifest: Self =
            toml::from_str(s).map_err(|e| anyhow!("Failed to parse TOML manifest: {}", e))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn from_json(s: &str) -> Result<Self> {
        let manifest: Self =
            serde_json::from_str(s).map_err(|e| anyhow!("Failed to parse JSON manifest: {}", e))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string(self).map_err(|e| anyhow!("Failed to serialize to TOML manifest: {}", e))
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| anyhow!("Failed to serialize to JSON manifest: {}", e))
    }

    pub fn validate(&self) -> Result<()> {
        // 1. Verify that depends_on references actual services within the manifest.
        for (name, spec) in &self.services {
            for dep in &spec.depends_on {
                if !self.services.contains_key(dep) {
                    return Err(anyhow!(
                        "Service '{}' depends on undefined service '{}'",
                        name,
                        dep
                    ));
                }
            }
        }

        // 2. Perform cycle detection.
        let mut visited = BTreeMap::new();
        let mut stack = BTreeMap::new();
        for name in self.services.keys() {
            visited.insert(name, false);
            stack.insert(name, false);
        }

        fn has_cycle<'a>(
            node: &'a LogicalServiceName,
            services: &'a BTreeMap<LogicalServiceName, ServiceSpec>,
            visited: &mut BTreeMap<&'a LogicalServiceName, bool>,
            stack: &mut BTreeMap<&'a LogicalServiceName, bool>,
        ) -> bool {
            if *stack.get(node).unwrap_or(&false) {
                return true;
            }
            if *visited.get(node).unwrap_or(&false) {
                return false;
            }

            visited.insert(node, true);
            stack.insert(node, true);

            if let Some(spec) = services.get(node) {
                for dep in &spec.depends_on {
                    if has_cycle(dep, services, visited, stack) {
                        return true;
                    }
                }
            }

            stack.insert(node, false);
            false
        }

        for name in self.services.keys() {
            if has_cycle(name, &self.services, &mut visited, &mut stack) {
                return Err(anyhow!("Circular dependency detected in services"));
            }
        }

        // 3. `replicas`, all three rules in one place (M05A A5e,
        // D-A5e-14/D-A5e-16): `>= 1`, `<= MAX_REPLICAS`, and not alongside a
        // declared `schema`.
        for (name, spec) in &self.services {
            if spec.replicas < 1 {
                return Err(anyhow!("Service '{}' declares replicas = 0; the minimum is 1", name));
            }
            if spec.replicas > MAX_REPLICAS {
                return Err(anyhow!(
                    "Service '{}' declares replicas = {}, above the cap of {MAX_REPLICAS}",
                    name,
                    spec.replicas
                ));
            }
            if spec.replicas > 1 && spec.config.schema.is_some() {
                return Err(anyhow!(
                    "Service '{}' declares replicas = {} alongside a schema: each member is its \
                     own service_id and therefore its own database, so a stateful service's data \
                     would silently split across members. `replicas` is for stateless members \
                     until M7's state replication lands; this refusal relaxes then.",
                    name,
                    spec.replicas
                ));
            }
            if spec.replicas <= 1 && spec.sharding_strategy.is_some() {
                return Err(anyhow!(
                    "Service '{}' declares a sharding_strategy with replicas = {}; a strategy \
                     over one member is not a selection",
                    name,
                    spec.replicas
                ));
            }
            if matches!(spec.sharding_strategy, Some(ShardingStrategy::RangeSharding(_))) {
                return Err(anyhow!(
                    "Service '{}' declares a range_sharding strategy; range sharding names \
                     concrete members by ServiceId, which a manifest cannot express before those \
                     members are minted -- it is reachable only once shard rebalancing assigns \
                     them, not from a manifest",
                    name
                ));
            }
        }

        // 4. `schedule`: the cron must parse, the named interface must be
        // one the service actually declares, `method` must be non-empty,
        // `params` (if present) must be JSON, `timeout_ms` must be a
        // budget a run can actually finish inside, and the count of
        // scheduled services must not exceed the cap. The last two are
        // re-checked at `submit`, since that path takes an already-compiled
        // plan and would otherwise apply neither
        // (`refuse_unrunnable_schedules`, `syneroym-app-supervisor`).
        let mut scheduled = 0usize;
        for (name, spec) in &self.services {
            let Some(sched) = &spec.schedule else { continue };
            scheduled += 1;
            sched
                .parsed()
                .map_err(|e| anyhow!("Service '{}' declares an invalid schedule: {e}", name))?;
            if !spec.config.interfaces.contains(&sched.interface) {
                return Err(anyhow!(
                    "Service '{}' schedules '{}/{}' but does not declare interface '{}'",
                    name,
                    sched.interface,
                    sched.method,
                    sched.interface
                ));
            }
            if sched.method.trim().is_empty() {
                return Err(anyhow!("Service '{}' declares a schedule with an empty method", name));
            }
            // A zero budget is not "no limit": the supervisor's
            // `tokio::time::timeout` elapses before the call is even made,
            // and the watermark is already written by then -- so the tick
            // is consumed, an alert is raised, and every later tick repeats
            // the cycle. Above the ceiling is refused rather than silently
            // clamped, so a manifest never runs under a budget different
            // from the one it asks for.
            if sched.timeout_ms == 0 || sched.timeout_ms > MAX_SCHEDULE_TIMEOUT_MS {
                return Err(anyhow!(
                    "Service '{}' declares a schedule timeout of {}ms; it must be between 1 and \
                     {MAX_SCHEDULE_TIMEOUT_MS}ms",
                    name,
                    sched.timeout_ms
                ));
            }
            if let Some(params) = &sched.params {
                serde_json::from_str::<Value>(params).map_err(|e| {
                    anyhow!("Service '{}' declares a schedule whose params are not JSON: {e}", name)
                })?;
            }
        }
        if scheduled > MAX_SCHEDULED_SERVICES {
            return Err(anyhow!(
                "{scheduled} services declare a schedule, above the cap of \
                 {MAX_SCHEDULED_SERVICES}"
            ));
        }

        Ok(())
    }
}

const fn is_zero_u32(n: &u32) -> bool {
    *n == 0
}

/// A planned, compiled service instance within a deployment plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannedService {
    pub service_id: ServiceId,
    pub logical_ref: LogicalServiceRef,
    /// The substrate this service is placed on, after the manifest default
    /// and any per-service override have been folded together. `None` means
    /// the substrate the deploy was aimed at, which is what every manifest
    /// written before placement existed still means.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub substrate: Option<SubstrateAlias>,
    #[serde(flatten)]
    pub config: ServiceConfig,
    /// Declared dependency name -> the member master DIDs currently serving
    /// it. One entry per `ServiceSpec.depends_on` name; the member list has
    /// one entry per member of that dependency (M05A A5e `replicas`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resolved_dependencies: BTreeMap<LogicalServiceName, Vec<ServiceId>>,
    #[serde(default)]
    pub topology_mode: TopologyMode,
    /// This member's ordinal within its logical service (M05A A5e,
    /// D-A5e-3). `0` for every plan compiled before `replicas` existed --
    /// a stored field, not a `plan.services` vector position, since two
    /// live code paths filter and rebuild that vector and an index derived
    /// from position would change a member's identity depending on which
    /// pass looked at it. `#[serde(skip_serializing_if)]` so an unscaled
    /// plan's JSON is byte-for-byte unchanged.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub member_index: u32,
    /// Cloned from `ServiceSpec.schedule` -- every member of
    /// a scaled scheduled service carries the identical spec, exactly as
    /// `resolved_dependencies` and `topology_mode` already do. Never mapped
    /// onto the wire; see `ServiceSpec.schedule`'s own doc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<ScheduleSpec>,
    /// Cloned from `ServiceSpec.sharding_strategy` -- every member of a
    /// scaled sharded service carries the identical value, exactly as
    /// `topology_mode` and `schedule` already do. Needed on the *plan*, not
    /// only the manifest: the supervisor holds no manifest, so a Tier-2
    /// topology document (ADR-0022 §3) built from the stored plan could
    /// otherwise never name a strategy at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharding_strategy: Option<ShardingStrategy>,
}

impl PlannedService {
    /// This member's identity as a managed unit (M05A A5e, D-A5e-2) -- the
    /// key every per-member stored or reported fact uses, as distinct from
    /// `logical_ref`, the key of the *logical service* it belongs to.
    #[must_use]
    pub fn member_ref(&self) -> MemberRef {
        MemberRef { logical_ref: self.logical_ref.clone(), index: self.member_index }
    }
}

/// Compiled, immutable deployment plan for the active controller or local
/// roymctl runner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeploymentPlan {
    pub app_instance_id: AppInstanceId,
    pub blueprint_id: AppBlueprintId,
    pub version: Version,
    #[serde(default)]
    pub services: Vec<PlannedService>,
}

impl DeploymentPlan {
    pub fn from_toml(s: &str) -> Result<Self> {
        toml::from_str(s).map_err(|e| anyhow!("Failed to parse TOML deployment plan: {}", e))
    }

    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).map_err(|e| anyhow!("Failed to parse JSON deployment plan: {}", e))
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string(self)
            .map_err(|e| anyhow!("Failed to serialize to TOML deployment plan: {}", e))
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| anyhow!("Failed to serialize to JSON deployment plan: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::DEFAULT_SCHEDULE_TIMEOUT_MS;

    #[test]
    fn test_manifest_parsing_toml() {
        let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"
            description = "Professional Services Guild App"

            [services.identity]
            service_type = "wasm"
            source = "crates/sandbox_wasm/benches/identity.wasm"
            interfaces = ["syneroym:identity/identity"]
            depends_on = []

            [services.echo]
            service_type = "wasm"
            source = "crates/sandbox_wasm/benches/echo.wasm"
            interfaces = ["syneroym:echo/echo"]
            depends_on = ["identity"]

            [dependencies.db]
            mode = "spawn"
            blueprint = "syneroym:db-app"
            manifest_path = "path/to/db.toml"
        "#;

        let manifest = SynAppManifest::from_toml(toml_str).unwrap();
        assert_eq!(manifest.id.as_str(), "syneroym:guild-app");
        assert_eq!(manifest.services.len(), 2);
        assert_eq!(manifest.dependencies.len(), 1);

        let identity = manifest.services.get(&LogicalServiceName::new("identity")).unwrap();
        assert_eq!(identity.config.service_type, ServiceType::Wasm);
        assert_eq!(identity.config.source, "crates/sandbox_wasm/benches/identity.wasm");

        let db_dep = manifest.dependencies.get(&DependencyName::new("db")).unwrap();
        match db_dep {
            AppDependencySpec::Spawn { blueprint, manifest_path } => {
                assert_eq!(blueprint.as_str(), "syneroym:db-app");
                assert_eq!(manifest_path.as_deref(), Some("path/to/db.toml"));
            }
            _ => panic!("Expected Spawn dependency"),
        }

        // Test serialization roundtrip
        let serialized = manifest.to_toml().unwrap();
        let deserialized = SynAppManifest::from_toml(&serialized).unwrap();
        assert_eq!(manifest, deserialized);

        // A manifest with no [services.x.fdae] block parses with fdae: None.
        assert_eq!(identity.config.fdae, None);
    }

    #[test]
    fn test_manifest_parsing_toml_with_fdae_policy() {
        let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.identity]
            service_type = "wasm"
            source = "crates/sandbox_wasm/benches/identity.wasm"
            interfaces = ["syneroym:identity/identity"]
            depends_on = []

            [services.identity.fdae]
            policy = "fdae-policy.json"
        "#;

        let manifest = SynAppManifest::from_toml(toml_str).unwrap();
        let identity = manifest.services.get(&LogicalServiceName::new("identity")).unwrap();
        assert_eq!(
            identity.config.fdae,
            Some(FdaeManifest { policy: DocumentRef::Local("fdae-policy.json".to_string()) })
        );

        let serialized = manifest.to_toml().unwrap();
        let deserialized = SynAppManifest::from_toml(&serialized).unwrap();
        assert_eq!(manifest, deserialized);
    }

    /// The bare-string and explicit-`remote_path` forms are one field, so a
    /// manifest can say "ship this with the deploy" or "the substrate already
    /// has it" without a second key that has to be kept mutually exclusive.
    #[test]
    fn test_manifest_parsing_toml_with_remote_document_refs() {
        let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.identity]
            service_type = "wasm"
            source = "crates/sandbox_wasm/benches/identity.wasm"
            schema = { remote_path = "/etc/syneroym/schemas/shared.json" }

            [services.identity.fdae]
            policy = { remote_path = "/etc/syneroym/policies/guild.json" }
        "#;

        let manifest = SynAppManifest::from_toml(toml_str).unwrap();
        let identity = manifest.services.get(&LogicalServiceName::new("identity")).unwrap();
        assert_eq!(
            identity.config.schema,
            Some(DocumentRef::Remote {
                remote_path: "/etc/syneroym/schemas/shared.json".to_string()
            })
        );
        assert_eq!(
            identity.config.fdae,
            Some(FdaeManifest {
                policy: DocumentRef::Remote {
                    remote_path: "/etc/syneroym/policies/guild.json".to_string()
                }
            })
        );

        let serialized = manifest.to_toml().unwrap();
        let deserialized = SynAppManifest::from_toml(&serialized).unwrap();
        assert_eq!(manifest, deserialized);
    }

    #[test]
    fn test_manifest_parsing_json() {
        let json_str = r#"{
            "id": "syneroym:guild-app",
            "version": "0.1.0",
            "description": "Professional Services Guild App",
            "services": {
                "identity": {
                    "service_type": "wasm",
                    "source": "crates/sandbox_wasm/benches/identity.wasm",
                    "interfaces": ["syneroym:identity/identity"],
                    "depends_on": []
                }
            },
            "dependencies": {
                "db": {
                    "mode": "bind",
                    "instance": "inst-1234"
                }
            }
        }"#;

        let manifest = SynAppManifest::from_json(json_str).unwrap();
        assert_eq!(manifest.id.as_str(), "syneroym:guild-app");
        assert_eq!(manifest.services.len(), 1);
        assert_eq!(manifest.dependencies.len(), 1);

        let db_dep = manifest.dependencies.get(&DependencyName::new("db")).unwrap();
        match db_dep {
            AppDependencySpec::Bind { instance } => {
                assert_eq!(instance.as_str(), "inst-1234");
            }
            _ => panic!("Expected Bind dependency"),
        }

        // Test serialization roundtrip
        let serialized = manifest.to_json().unwrap();
        let deserialized = SynAppManifest::from_json(&serialized).unwrap();
        assert_eq!(manifest, deserialized);
    }

    #[test]
    fn test_deployment_plan_serialization() {
        let mut env_map = BTreeMap::new();
        env_map.insert("KEY".to_string(), "VAL".to_string());

        let plan = DeploymentPlan {
            app_instance_id: AppInstanceId::new("guild-instance-1"),
            blueprint_id: AppBlueprintId::new("syneroym:guild-app"),
            version: Version::parse("0.1.0").unwrap(),
            services: vec![PlannedService {
                service_id: ServiceId::new("did:key:h123"),
                logical_ref: LogicalServiceRef {
                    app_instance_id: AppInstanceId::new("guild-instance-1"),
                    service_name: LogicalServiceName::new("identity"),
                },
                substrate: None,
                config: ServiceConfig {
                    service_type: ServiceType::Wasm,
                    source: "crates/sandbox_wasm/benches/identity.wasm".to_string(),
                    hash: None,
                    interfaces: vec![InterfaceName::new("syneroym:identity/identity")],
                    env: env_map,
                    args: vec![],
                    custom_config: None,
                    quota: None,
                    schema: None,
                    rotation_policy: RotationPolicy::RestartOnRotation,
                    fdae: None,
                    health_check: None,
                },
                resolved_dependencies: BTreeMap::new(),
                topology_mode: TopologyMode::Singleton,
                member_index: 0,
                schedule: None,
                sharding_strategy: None,
            }],
        };

        let toml_str = plan.to_toml().unwrap();
        let plan_toml = DeploymentPlan::from_toml(&toml_str).unwrap();
        assert_eq!(plan, plan_toml);

        let json_str = plan.to_json().unwrap();
        let plan_json = DeploymentPlan::from_json(&json_str).unwrap();
        assert_eq!(plan, plan_json);

        // Detailed field assertion test
        assert_eq!(plan_toml.app_instance_id, AppInstanceId::new("guild-instance-1"));
        assert_eq!(plan_toml.version.to_string(), "0.1.0");
        assert_eq!(plan_toml.services.len(), 1);
        let service = &plan_toml.services[0];
        assert_eq!(service.service_id, ServiceId::new("did:key:h123"));
        assert_eq!(service.topology_mode, TopologyMode::Singleton);
        assert_eq!(service.config.service_type, ServiceType::Wasm);
    }

    #[test]
    fn test_logical_service_ref_from_str() {
        let s = "guild-instance-1/identity";
        let r = LogicalServiceRef::from_str(s).unwrap();
        assert_eq!(r.app_instance_id, AppInstanceId::new("guild-instance-1"));
        assert_eq!(r.service_name, LogicalServiceName::new("identity"));
        assert_eq!(r.to_string(), s);

        assert!(LogicalServiceRef::from_str("invalid").is_err());
        assert!(LogicalServiceRef::from_str("too/many/parts").is_err());
    }

    #[test]
    fn test_id_validations() {
        assert!(LogicalServiceName::try_new("").is_err());
        assert!(LogicalServiceName::try_new("some/name").is_err());
        assert!(LogicalServiceName::try_new("good-name").is_ok());

        assert!(ServiceId::try_new("not-did-key").is_err());
        assert!(ServiceId::try_new("did:key:123").is_ok());
    }

    // ── M05A A5e: `MemberRef` and the `#`/`/` validators (D-A5e-2) ──────

    #[test]
    fn member_ref_round_trips_through_display_and_from_str() {
        let m = MemberRef {
            logical_ref: LogicalServiceRef {
                app_instance_id: AppInstanceId::new("inst-1"),
                service_name: LogicalServiceName::new("backend"),
            },
            index: 2,
        };
        assert_eq!(m.to_string(), "inst-1/backend#2");
        let parsed = MemberRef::from_str("inst-1/backend#2").unwrap();
        assert_eq!(parsed, m);

        assert!(MemberRef::from_str("inst-1/backend").is_err(), "no index separator at all");
        assert!(MemberRef::from_str("inst-1/backend#not-a-number").is_err());
    }

    /// A service name that (illegally, pre-validation) carried the index
    /// separator itself must not parse as if the `#` split were the real
    /// one -- `LogicalServiceName`'s own validator is what actually
    /// prevents this from ever being stored, but `MemberRef::from_str`
    /// must independently reject it too, since it re-derives
    /// `LogicalServiceRef::try_new` on whatever sits before the last `#`.
    #[test]
    fn member_ref_parse_rejects_a_service_name_carrying_the_index_separator() {
        let err = MemberRef::from_str("inst-1/back#end#3").unwrap_err();
        assert!(err.to_string().contains('#'), "{err}");
    }

    #[test]
    fn a_logical_service_name_containing_the_index_separator_is_refused() {
        assert!(LogicalServiceName::try_new("back#end").is_err());
        assert!(LogicalServiceName::try_new("backend").is_ok());
    }

    #[test]
    fn an_app_instance_id_containing_a_separator_is_refused() {
        assert!(AppInstanceId::try_new("inst/1").is_err());
        assert!(AppInstanceId::try_new("inst#1").is_err());
        assert!(AppInstanceId::try_new("inst-1").is_ok());
    }

    /// M05A A7 review finding 5: `..` and `\` are refused at construction
    /// now, not only later at `crates/app_supervisor/src/keys.rs`'s
    /// `validate_backup_name` -- an id that can never be a vault backup
    /// name must never exist, rather than being accepted here and only
    /// failing, permanently, the first time `adopt` tries to mint the
    /// app-instance master under it.
    #[test]
    fn an_app_instance_id_that_could_never_be_backed_up_is_refused_at_construction() {
        assert!(AppInstanceId::try_new("..").is_err());
        assert!(AppInstanceId::try_new("inst..1").is_err());
        assert!(AppInstanceId::try_new("back\\slash").is_err());
        assert!(AppInstanceId::try_new("inst-1").is_ok());
    }

    #[test]
    fn a_planned_services_member_ref_combines_its_logical_ref_and_index() {
        let svc = PlannedService {
            service_id: ServiceId::new("did:key:h123"),
            logical_ref: LogicalServiceRef {
                app_instance_id: AppInstanceId::new("inst-1"),
                service_name: LogicalServiceName::new("backend"),
            },
            substrate: None,
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:9000".to_string(),
                hash: None,
                interfaces: vec![],
                env: BTreeMap::new(),
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: RotationPolicy::RestartOnRotation,
                fdae: None,
                health_check: None,
            },
            resolved_dependencies: BTreeMap::new(),
            topology_mode: TopologyMode::Singleton,
            member_index: 3,
            schedule: None,
            sharding_strategy: None,
        };
        assert_eq!(svc.member_ref().to_string(), "inst-1/backend#3");
    }

    /// An unscaled plan's JSON must stay byte-for-byte what it was before
    /// `member_index` existed -- the field is skip-if-zero, so a pre-A5e
    /// stored plan and a fresh single-member one serialize identically.
    #[test]
    fn member_index_zero_emits_no_key_in_serialized_output() {
        let svc = PlannedService {
            service_id: ServiceId::new("did:key:h123"),
            logical_ref: LogicalServiceRef {
                app_instance_id: AppInstanceId::new("inst-1"),
                service_name: LogicalServiceName::new("backend"),
            },
            substrate: None,
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:9000".to_string(),
                hash: None,
                interfaces: vec![],
                env: BTreeMap::new(),
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: RotationPolicy::RestartOnRotation,
                fdae: None,
                health_check: None,
            },
            resolved_dependencies: BTreeMap::new(),
            topology_mode: TopologyMode::Singleton,
            member_index: 0,
            schedule: None,
            sharding_strategy: None,
        };
        let toml = toml::to_string(&svc).unwrap();
        assert!(!toml.contains("member_index"));
    }

    #[test]
    fn test_negative_parsing_and_validation() {
        // Missing required field
        let malformed_toml = r#"
            id = "syneroym:bad"
        "#;
        assert!(SynAppManifest::from_toml(malformed_toml).is_err());

        // Circular dependency
        let circular_toml = r#"
            id = "syneroym:bad"
            version = "0.1.0"
            [services.a]
            service_type = "wasm"
            source = "a"
            depends_on = ["b"]

            [services.b]
            service_type = "wasm"
            source = "b"
            depends_on = ["a"]
        "#;
        let manifest_res = SynAppManifest::from_toml(circular_toml);
        assert!(manifest_res.is_err());
        assert!(manifest_res.err().unwrap().to_string().contains("Circular dependency"));

        // Undefined dependency
        let missing_dep_toml = r#"
            id = "syneroym:bad"
            version = "0.1.0"
            [services.a]
            service_type = "wasm"
            source = "a"
            depends_on = ["nonexistent"]
        "#;
        let manifest_res2 = SynAppManifest::from_toml(missing_dep_toml);
        assert!(manifest_res2.is_err());
        assert!(manifest_res2.err().unwrap().to_string().contains("undefined service"));
    }

    #[test]
    fn test_toml_env_serialization() {
        let mut env_map = BTreeMap::new();
        env_map.insert("DATABASE_URL".to_string(), "postgres://...".to_string());
        env_map.insert("PORT".to_string(), "8080".to_string());

        let plan = DeploymentPlan {
            app_instance_id: AppInstanceId::new("guild-instance-1"),
            blueprint_id: AppBlueprintId::new("syneroym:guild-app"),
            version: Version::parse("0.1.0").unwrap(),
            services: vec![PlannedService {
                service_id: ServiceId::new("did:key:h123"),
                logical_ref: LogicalServiceRef {
                    app_instance_id: AppInstanceId::new("guild-instance-1"),
                    service_name: LogicalServiceName::new("identity"),
                },
                substrate: None,
                config: ServiceConfig {
                    service_type: ServiceType::Wasm,
                    source: "crates/sandbox_wasm/benches/identity.wasm".to_string(),
                    hash: None,
                    interfaces: vec![],
                    env: env_map,
                    args: vec![],
                    custom_config: None,
                    quota: None,
                    schema: None,
                    rotation_policy: RotationPolicy::RestartOnRotation,
                    fdae: None,
                    health_check: None,
                },
                resolved_dependencies: BTreeMap::new(),
                topology_mode: TopologyMode::Singleton,
                member_index: 0,
                schedule: None,
                sharding_strategy: None,
            }],
        };

        let toml_str = plan.to_toml().unwrap();
        assert!(toml_str.contains("DATABASE_URL"));
        assert!(toml_str.contains("PORT"));
    }

    #[test]
    fn a_manifest_default_placement_round_trips_through_toml_and_json() {
        let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [placement]
            substrate = "edge-1"

            [services.identity]
            service_type = "wasm"
            source = "identity.wasm"
        "#;

        let manifest = SynAppManifest::from_toml(toml_str).unwrap();
        assert_eq!(
            manifest.placement,
            Some(PlacementSelector::Substrate(SubstrateAlias::new("edge-1")))
        );

        let toml_round = manifest.to_toml().unwrap();
        assert_eq!(SynAppManifest::from_toml(&toml_round).unwrap(), manifest);

        let json_round = manifest.to_json().unwrap();
        assert_eq!(SynAppManifest::from_json(&json_round).unwrap(), manifest);

        // Absent placement stays absent -- pre-placement manifests are unaffected.
        let no_placement = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"
        "#;
        assert_eq!(SynAppManifest::from_toml(no_placement).unwrap().placement, None);
    }

    #[test]
    fn a_per_service_placement_override_round_trips_through_toml_and_json() {
        let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [placement]
            substrate = "edge-1"

            [services.identity]
            service_type = "wasm"
            source = "identity.wasm"

            [services.identity.placement]
            substrate = "edge-2"
        "#;

        let manifest = SynAppManifest::from_toml(toml_str).unwrap();
        let identity = manifest.services.get(&LogicalServiceName::new("identity")).unwrap();
        assert_eq!(
            identity.placement,
            Some(PlacementSelector::Substrate(SubstrateAlias::new("edge-2")))
        );

        // This is the round trip that would catch a #[serde(flatten)] regression:
        // an externally-tagged enum nested inside a struct also using `flatten`.
        let toml_round = manifest.to_toml().unwrap();
        assert_eq!(SynAppManifest::from_toml(&toml_round).unwrap(), manifest);

        let json_round = manifest.to_json().unwrap();
        assert_eq!(SynAppManifest::from_json(&json_round).unwrap(), manifest);
    }

    #[test]
    fn a_substrate_alias_rejects_a_bare_did() {
        let err = SubstrateAlias::try_new("did:key:z6MkExample").unwrap_err();
        assert!(err.to_string().contains("looks like a DID"));
    }

    #[test]
    fn a_substrate_alias_rejects_an_empty_name() {
        assert!(SubstrateAlias::try_new("").is_err());
    }

    #[test]
    fn a_substrate_alias_rejects_a_path_separator() {
        assert!(SubstrateAlias::try_new("edge/1").is_err());
    }

    #[test]
    fn a_planned_service_round_trips_its_substrate() {
        let mut svc = PlannedService {
            service_id: ServiceId::new("did:key:h123"),
            logical_ref: LogicalServiceRef {
                app_instance_id: AppInstanceId::new("guild-instance-1"),
                service_name: LogicalServiceName::new("identity"),
            },
            substrate: Some(SubstrateAlias::new("edge-1")),
            config: ServiceConfig {
                service_type: ServiceType::Wasm,
                source: "identity.wasm".to_string(),
                hash: None,
                interfaces: vec![],
                env: BTreeMap::new(),
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: RotationPolicy::RestartOnRotation,
                fdae: None,
                health_check: None,
            },
            resolved_dependencies: BTreeMap::new(),
            topology_mode: TopologyMode::Singleton,
            member_index: 0,
            schedule: None,
            sharding_strategy: None,
        };

        let toml_round = toml::to_string(&svc).unwrap();
        assert!(toml_round.contains("edge-1"));
        let parsed: PlannedService = toml::from_str(&toml_round).unwrap();
        assert_eq!(parsed, svc);

        // `None` serializes with no `substrate` key at all, matching
        // every manifest written before placement existed.
        svc.substrate = None;
        let toml_round = toml::to_string(&svc).unwrap();
        assert!(!toml_round.contains("substrate"));
    }

    #[test]
    fn a_health_check_round_trips_through_toml_and_json() {
        let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.backend]
            service_type = "container"
            source = "unused"
            interfaces = ["http"]

            [services.backend.health_check.http-get]
            interface = "http"
            path = "/healthz"
            expect_status = 200
            timeout_ms = 1500

            [services.tcpsvc]
            service_type = "tcp"
            source = "unused"
            interfaces = ["main"]

            [services.tcpsvc.health_check.tcp-connect]
            interface = "main"

            [services.wasmsvc]
            service_type = "wasm"
            source = "unused"
            interfaces = ["rpc"]

            [services.wasmsvc.health_check.rpc]
            interface = "rpc"
            method = "ping"
        "#;

        let manifest = SynAppManifest::from_toml(toml_str).unwrap();
        let backend = manifest.services.get(&LogicalServiceName::new("backend")).unwrap();
        assert_eq!(
            backend.config.health_check,
            Some(HealthCheck::HttpGet(HttpProbe {
                interface: InterfaceName::new("http"),
                path: "/healthz".to_string(),
                expect_status: 200,
                timeout_ms: 1500,
            }))
        );

        // This is the round trip that would catch a #[serde(flatten)] regression:
        // an externally-tagged enum nested inside a struct also using `flatten`.
        let toml_round = manifest.to_toml().unwrap();
        assert_eq!(SynAppManifest::from_toml(&toml_round).unwrap(), manifest);

        let json_round = manifest.to_json().unwrap();
        assert_eq!(SynAppManifest::from_json(&json_round).unwrap(), manifest);
    }

    #[test]
    fn an_absent_health_check_emits_no_key() {
        let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.identity]
            service_type = "wasm"
            source = "unused"
        "#;
        let manifest = SynAppManifest::from_toml(toml_str).unwrap();
        let identity = manifest.services.get(&LogicalServiceName::new("identity")).unwrap();
        assert_eq!(identity.config.health_check, None);
        let serialized = manifest.to_toml().unwrap();
        assert!(!serialized.contains("health_check"));
    }

    #[test]
    fn probe_defaults_apply_when_omitted() {
        let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.backend]
            service_type = "container"
            source = "unused"

            [services.backend.health_check.http-get]
            interface = "http"
            path = "/healthz"
        "#;
        let manifest = SynAppManifest::from_toml(toml_str).unwrap();
        let backend = manifest.services.get(&LogicalServiceName::new("backend")).unwrap();
        match backend.config.health_check.as_ref().unwrap() {
            HealthCheck::HttpGet(p) => {
                assert_eq!(p.expect_status, 200);
                assert_eq!(p.timeout_ms, DEFAULT_PROBE_TIMEOUT_MS);
            }
            other => panic!("expected HttpGet, got {other:?}"),
        }
    }

    #[test]
    fn valid_for_pairs_each_kind_with_its_service_types() {
        let tcp = HealthCheck::TcpConnect(TcpProbe {
            interface: InterfaceName::new("main"),
            timeout_ms: DEFAULT_PROBE_TIMEOUT_MS,
        });
        assert_eq!(tcp.valid_for(), &[ServiceType::Tcp, ServiceType::Container]);

        let http = HealthCheck::HttpGet(HttpProbe {
            interface: InterfaceName::new("main"),
            path: "/healthz".to_string(),
            expect_status: 200,
            timeout_ms: DEFAULT_PROBE_TIMEOUT_MS,
        });
        assert_eq!(http.valid_for(), &[ServiceType::Tcp, ServiceType::Container]);

        let rpc = HealthCheck::Rpc(RpcProbe {
            interface: InterfaceName::new("main"),
            method: "ping".to_string(),
            timeout_ms: DEFAULT_PROBE_TIMEOUT_MS,
        });
        assert_eq!(rpc.valid_for(), &[ServiceType::Wasm]);
    }

    // ── `ScheduleSpec` on the manifest surface ──────────────────────────

    fn scheduled_manifest_toml(schedule_block: &str) -> String {
        format!(
            r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.worker]
            service_type = "wasm"
            source = "unused"
            interfaces = ["scheduled-driver"]

            {schedule_block}
        "#
        )
    }

    #[test]
    fn a_manifest_with_no_schedule_serializes_byte_for_byte_as_before() {
        let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.worker]
            service_type = "wasm"
            source = "unused"
        "#;
        let manifest = SynAppManifest::from_toml(toml_str).unwrap();
        let worker = manifest.services.get(&LogicalServiceName::new("worker")).unwrap();
        assert_eq!(worker.schedule, None);
        let serialized = manifest.to_toml().unwrap();
        assert!(!serialized.contains("schedule"));
    }

    #[test]
    fn a_scheduled_service_round_trips_through_toml_and_json() {
        let toml_str = scheduled_manifest_toml(
            r#"
            [services.worker.schedule]
            cron = "* * * * *"
            interface = "scheduled-driver"
            method = "tick"
        "#,
        );
        let manifest = SynAppManifest::from_toml(&toml_str).unwrap();
        let worker = manifest.services.get(&LogicalServiceName::new("worker")).unwrap();
        let sched = worker.schedule.as_ref().unwrap();
        assert_eq!(sched.cron, "* * * * *");
        assert_eq!(sched.interface, InterfaceName::new("scheduled-driver"));
        assert_eq!(sched.method, "tick");
        assert_eq!(sched.params, None);
        assert_eq!(sched.timeout_ms, DEFAULT_SCHEDULE_TIMEOUT_MS);

        let toml_round = manifest.to_toml().unwrap();
        assert_eq!(SynAppManifest::from_toml(&toml_round).unwrap(), manifest);
        let json_round = manifest.to_json().unwrap();
        assert_eq!(SynAppManifest::from_json(&json_round).unwrap(), manifest);
    }

    #[test]
    fn a_schedule_naming_an_undeclared_interface_is_refused_at_validation() {
        let toml_str = scheduled_manifest_toml(
            r#"
            [services.worker.schedule]
            cron = "* * * * *"
            interface = "not-declared"
            method = "tick"
        "#,
        );
        let err = SynAppManifest::from_toml(&toml_str).unwrap_err();
        assert!(err.to_string().contains("does not declare interface"), "{err}");
    }

    #[test]
    fn a_schedule_with_an_unparseable_cron_is_refused_at_validation() {
        let toml_str = scheduled_manifest_toml(
            r#"
            [services.worker.schedule]
            cron = "not a cron"
            interface = "scheduled-driver"
            method = "tick"
        "#,
        );
        let err = SynAppManifest::from_toml(&toml_str).unwrap_err();
        assert!(err.to_string().contains("does not parse"), "{err}");
    }

    #[test]
    fn a_schedule_whose_params_are_not_json_is_refused_at_validation() {
        let toml_str = scheduled_manifest_toml(
            r#"
            [services.worker.schedule]
            cron = "* * * * *"
            interface = "scheduled-driver"
            method = "tick"
            params = "not json"
        "#,
        );
        let err = SynAppManifest::from_toml(&toml_str).unwrap_err();
        assert!(err.to_string().contains("not JSON"), "{err}");
    }

    /// A zero budget is not "no limit": the supervisor's timeout elapses
    /// before the call is made, and the watermark is already written by
    /// then -- so the tick is consumed, an alert is raised, and every later
    /// tick repeats the cycle. A schedule that can never run and never says
    /// why must be refused where the author can still read the refusal.
    #[test]
    fn a_schedule_with_a_zero_timeout_is_refused_at_validation() {
        let toml_str = scheduled_manifest_toml(
            r#"
            [services.worker.schedule]
            cron = "* * * * *"
            interface = "scheduled-driver"
            method = "tick"
            timeout_ms = 0
        "#,
        );
        let err = SynAppManifest::from_toml(&toml_str).unwrap_err();
        assert!(err.to_string().contains("must be between 1"), "{err}");
    }

    /// Above the ceiling is refused rather than silently clamped: a
    /// manifest must never run under a budget different from the one it
    /// asks for.
    #[test]
    fn a_schedule_timeout_above_the_ceiling_is_refused_rather_than_clamped() {
        let toml_str = scheduled_manifest_toml(&format!(
            r#"
            [services.worker.schedule]
            cron = "* * * * *"
            interface = "scheduled-driver"
            method = "tick"
            timeout_ms = {}
        "#,
            MAX_SCHEDULE_TIMEOUT_MS + 1
        ));
        let err = SynAppManifest::from_toml(&toml_str).unwrap_err();
        assert!(err.to_string().contains(&format!("{MAX_SCHEDULE_TIMEOUT_MS}ms")), "{err}");
    }

    #[test]
    fn a_schedule_exactly_at_the_timeout_ceiling_is_accepted() {
        let toml_str = scheduled_manifest_toml(&format!(
            r#"
            [services.worker.schedule]
            cron = "* * * * *"
            interface = "scheduled-driver"
            method = "tick"
            timeout_ms = {MAX_SCHEDULE_TIMEOUT_MS}
        "#
        ));
        assert!(SynAppManifest::from_toml(&toml_str).is_ok());
    }

    #[test]
    fn more_than_the_cap_of_scheduled_services_is_refused_at_validation() {
        let mut manifest = SynAppManifest {
            id: AppBlueprintId::new("syneroym:guild-app"),
            version: Version::parse("0.1.0").unwrap(),
            description: None,
            placement: None,
            services: BTreeMap::new(),
            dependencies: BTreeMap::new(),
        };
        for i in 0..=MAX_SCHEDULED_SERVICES {
            let name = LogicalServiceName::new(format!("worker-{i}"));
            manifest.services.insert(
                name,
                ServiceSpec {
                    config: ServiceConfig {
                        service_type: ServiceType::Wasm,
                        source: "unused".to_string(),
                        hash: None,
                        interfaces: vec![InterfaceName::new("scheduled-driver")],
                        env: BTreeMap::new(),
                        args: vec![],
                        custom_config: None,
                        quota: None,
                        schema: None,
                        rotation_policy: RotationPolicy::RestartOnRotation,
                        fdae: None,
                        health_check: None,
                    },
                    depends_on: vec![],
                    placement: None,
                    replicas: 1,
                    sharding_strategy: None,
                    schedule: Some(ScheduleSpec {
                        cron: "* * * * *".to_string(),
                        interface: InterfaceName::new("scheduled-driver"),
                        method: "tick".to_string(),
                        params: None,
                        timeout_ms: DEFAULT_SCHEDULE_TIMEOUT_MS,
                    }),
                },
            );
        }
        let err = manifest.validate().unwrap_err();
        assert!(err.to_string().contains("above the cap"), "{err}");
    }

    // ── `ShardingStrategy` on the manifest surface (ADR-0022 §6) ────────

    #[test]
    fn a_sharding_strategy_round_trips_through_a_manifest() {
        let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.worker]
            service_type = "wasm"
            source = "unused"
            replicas = 2
            sharding_strategy = "hash_sharding"
        "#;
        let manifest = SynAppManifest::from_toml(toml_str).unwrap();
        let worker = manifest.services.get(&LogicalServiceName::new("worker")).unwrap();
        assert_eq!(worker.sharding_strategy, Some(ShardingStrategy::HashSharding));

        let toml_round = manifest.to_toml().unwrap();
        assert_eq!(SynAppManifest::from_toml(&toml_round).unwrap(), manifest);
        let json_round = manifest.to_json().unwrap();
        assert_eq!(SynAppManifest::from_json(&json_round).unwrap(), manifest);
    }

    #[test]
    fn a_manifest_with_no_sharding_strategy_parses_as_it_does_today() {
        let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.worker]
            service_type = "wasm"
            source = "unused"
        "#;
        let manifest = SynAppManifest::from_toml(toml_str).unwrap();
        let worker = manifest.services.get(&LogicalServiceName::new("worker")).unwrap();
        assert_eq!(worker.sharding_strategy, None);
        let serialized = manifest.to_toml().unwrap();
        assert!(!serialized.contains("sharding_strategy"));
    }

    #[test]
    fn a_sharding_strategy_with_replicas_of_one_is_refused_at_validation() {
        let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.worker]
            service_type = "wasm"
            source = "unused"
            sharding_strategy = "hash_sharding"
        "#;
        let err = SynAppManifest::from_toml(toml_str).unwrap_err();
        assert!(err.to_string().contains("not a selection"), "{err}");
    }

    /// `RangeSharding` names concrete `ServiceId`s, which a manifest is
    /// authored long before any of a scaled service's members are minted --
    /// so it is refused here rather than accepted and left permanently
    /// unreachable.
    #[test]
    fn a_range_sharding_strategy_is_refused_at_validation() {
        let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.worker]
            service_type = "wasm"
            source = "unused"
            replicas = 2

            [services.worker.sharding_strategy]
            range_sharding = { chunks = [] }
        "#;
        let err = SynAppManifest::from_toml(toml_str).unwrap_err();
        assert!(err.to_string().contains("range_sharding"), "{err}");
    }
}
