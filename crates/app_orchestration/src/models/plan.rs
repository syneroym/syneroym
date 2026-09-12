use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use semver::Version;
use serde::{Deserialize, Serialize};

use super::{
    identifiers::{
        AppBlueprintId, AppInstanceId, LogicalServiceName, LogicalServiceRef, MemberRef, ServiceId,
        SubstrateAlias,
    },
    service::{
        ScheduleSpec, ServiceConfig, ShardingStrategy, TopologyMode, TopologyVisibility,
        is_restricted,
    },
};

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
    /// one entry per member of that dependency.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resolved_dependencies: BTreeMap<LogicalServiceName, Vec<ServiceId>>,
    #[serde(default)]
    pub topology_mode: TopologyMode,
    /// This member's ordinal within its logical service. `0` for every plan
    /// compiled before `replicas` existed -- a stored field, not a
    /// `plan.services` vector position, since two
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
    /// Who may fetch this logical service's topology document (ADR-0022 §5).
    /// Cloned from `ServiceSpec.topology_visibility` -- every member of a
    /// scaled service carries the identical value, exactly as `topology_mode`,
    /// `schedule`, and `sharding_strategy` already do. Needed on the *plan*,
    /// not only the manifest: the supervisor holds no manifest, so the
    /// stored plan is the only place it can read it from.
    #[serde(default, skip_serializing_if = "is_restricted")]
    pub topology_visibility: TopologyVisibility,
}

impl PlannedService {
    /// This member's identity as a managed unit -- the key every per-member
    /// stored or reported fact uses, as distinct from
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
        toml::from_str(s).map_err(|e| anyhow!("Failed to parse TOML deployment plan: {e}"))
    }

    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).map_err(|e| anyhow!("Failed to parse JSON deployment plan: {e}"))
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string(self)
            .map_err(|e| anyhow!("Failed to serialize to TOML deployment plan: {e}"))
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| anyhow!("Failed to serialize to JSON deployment plan: {e}"))
    }
}
