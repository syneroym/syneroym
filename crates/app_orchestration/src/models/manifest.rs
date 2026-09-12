use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    identifiers::{AppBlueprintId, DependencyName, LogicalServiceName, PlacementSelector},
    service::{AppDependencySpec, MAX_REPLICAS, ServiceSpec, ShardingStrategy},
};
use crate::schedule::{MAX_SCHEDULE_TIMEOUT_MS, MAX_SCHEDULED_SERVICES};

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
            toml::from_str(s).map_err(|e| anyhow!("Failed to parse TOML manifest: {e}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn from_json(s: &str) -> Result<Self> {
        let manifest: Self =
            serde_json::from_str(s).map_err(|e| anyhow!("Failed to parse JSON manifest: {e}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string(self).map_err(|e| anyhow!("Failed to serialize to TOML manifest: {e}"))
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| anyhow!("Failed to serialize to JSON manifest: {e}"))
    }

    pub fn validate(&self) -> Result<()> {
        // 1. Verify that depends_on references actual services within the manifest.
        for (name, spec) in &self.services {
            for dep in &spec.depends_on {
                if !self.services.contains_key(dep) {
                    return Err(anyhow!("Service '{name}' depends on undefined service '{dep}'"));
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

        // 3. `replicas`, all three rules in one place: `>= 1`,
        // `<= MAX_REPLICAS`, and not alongside a declared `schema`.
        for (name, spec) in &self.services {
            if spec.replicas < 1 {
                return Err(anyhow!("Service '{name}' declares replicas = 0; the minimum is 1"));
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
                    "Service '{name}' declares a range_sharding strategy; range sharding names \
                     concrete members by ServiceId, which a manifest cannot express before those \
                     members are minted -- it is reachable only once shard rebalancing assigns \
                     them, not from a manifest"
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
                .map_err(|e| anyhow!("Service '{name}' declares an invalid schedule: {e}"))?;
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
                return Err(anyhow!("Service '{name}' declares a schedule with an empty method"));
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
                    anyhow!("Service '{name}' declares a schedule whose params are not JSON: {e}")
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
