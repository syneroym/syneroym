//! The operator-held substrate inventory: the alias -> DID/address/credential/
//! capability map that a manifest's `placement` selectors name.
//!
//! Named `substrate_inventory`, not `inventory`, because this crate already
//! has `StaticInventory` (`resolver.rs`), the logical-name -> member-set
//! registry -- two unrelated "inventories" in one crate is a naming trap.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::models::{DeploymentPlan, ServiceType, SubstrateAlias};

/// Operator-declared map of substrate aliases to the substrates a manifest's
/// placement selectors may name.
///
/// Deliberately not discoverable: nothing on the wire reports a substrate's
/// DID, address, or what it can run, so this is the operator's own record of
/// the fleet they hold deploy capability on.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubstrateInventory {
    #[serde(default)]
    pub substrates: BTreeMap<SubstrateAlias, SubstrateEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubstrateEntry {
    pub did: String,
    /// How the *client* reaches this substrate; overrides the global
    /// `--api-url`. Deliberately not called `registry_url`: the registry a
    /// substrate publishes endpoint records into is its own configured one,
    /// which nothing here can see or set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    /// Local identity to present against this substrate. Overrides `--as`:
    /// one global credential cannot be right for N substrates owned by
    /// different controllers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ucan: Option<PathBuf>,
    /// Service types this substrate can run. Operator-declared, because
    /// nothing reports it: container support is a compile-time Cargo
    /// feature, invisible over the wire. Absent = unconstrained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<BTreeSet<ServiceType>>,
}

impl SubstrateInventory {
    /// Parses and validates. Every `did` must be a `did:key:` -- an alias
    /// pointing at a malformed DID fails here rather than as an opaque
    /// registry lookup miss at deploy.
    pub fn from_toml(s: &str) -> Result<Self> {
        let inventory: Self =
            toml::from_str(s).map_err(|e| anyhow!("failed to parse substrate inventory: {e}"))?;
        for (alias, entry) in &inventory.substrates {
            if !entry.did.starts_with("did:key:") {
                return Err(anyhow!(
                    "substrate '{alias}' has a malformed did '{}': must start with 'did:key:'",
                    entry.did
                ));
            }
        }
        Ok(inventory)
    }

    /// Reads and parses, with the path in every error message.
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| anyhow!("failed to read substrate inventory {}: {e}", path.display()))?;
        Self::from_toml(&contents)
            .map_err(|e| anyhow!("failed to parse substrate inventory {}: {e}", path.display()))
    }

    /// Looks up one alias, or errors naming the missing alias, the file it
    /// was looked for in, and every alias the file does define.
    pub fn get(&self, alias: &SubstrateAlias, source: &Path) -> Result<&SubstrateEntry> {
        self.substrates.get(alias).ok_or_else(|| {
            let known: Vec<&str> = self.substrates.keys().map(SubstrateAlias::as_str).collect();
            anyhow!(
                "no substrate '{alias}' in {} (known: {})",
                source.display(),
                if known.is_empty() { "none".to_string() } else { known.join(", ") }
            )
        })
    }
}

/// Every alias the plan places a service on, with the service types placed
/// there. The whole input to the deploy-time preflight: unknown aliases and
/// capability mismatches are both decided from this before any deploy call
/// is made.
pub fn placement_demand(plan: &DeploymentPlan) -> BTreeMap<SubstrateAlias, BTreeSet<ServiceType>> {
    let mut demand: BTreeMap<SubstrateAlias, BTreeSet<ServiceType>> = BTreeMap::new();
    for svc in &plan.services {
        if let Some(alias) = &svc.substrate {
            demand.entry(alias.clone()).or_default().insert(svc.config.service_type);
        }
    }
    demand
}

/// Checks every alias in `demand` against the inventory. Returns *all*
/// problems, not the first -- an operator fixing a five-substrate inventory
/// should not have to re-run five times.
pub fn check_placement(
    inventory: &SubstrateInventory,
    demand: &BTreeMap<SubstrateAlias, BTreeSet<ServiceType>>,
    source: &Path,
) -> Result<()> {
    let mut problems = Vec::new();
    for (alias, types) in demand {
        let Some(entry) = inventory.substrates.get(alias) else {
            let known: Vec<&str> =
                inventory.substrates.keys().map(SubstrateAlias::as_str).collect();
            problems.push(format!(
                "no substrate '{alias}' in {} (known: {})",
                source.display(),
                if known.is_empty() { "none".to_string() } else { known.join(", ") }
            ));
            continue;
        };
        if let Some(caps) = &entry.capabilities {
            for t in types {
                if !caps.contains(t) {
                    let declared: Vec<String> = caps.iter().map(|c| format!("{c:?}")).collect();
                    problems.push(format!(
                        "substrate '{alias}' does not declare '{t:?}' (declares: {})",
                        declared.join(", ")
                    ));
                }
            }
        }
    }
    if problems.is_empty() { Ok(()) } else { Err(anyhow!(problems.join("\n"))) }
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::*;
    use crate::models::{
        AppBlueprintId, AppInstanceId, LogicalServiceName, LogicalServiceRef, PlannedService,
        ServiceConfig, ServiceId, TopologyMode,
    };

    #[test]
    fn an_inventory_parses_every_optional_field() {
        let toml_str = r#"
            [substrates.edge-1]
            did = "did:key:z6MkExampleNodeA"
            api_url = "http://localhost:7961"
            identity = "operator"
            ucan = "grants/edge-1.json"
            capabilities = ["wasm", "tcp"]

            [substrates.edge-2]
            did = "did:key:z6MkExampleNodeB"
        "#;

        let inv = SubstrateInventory::from_toml(toml_str).unwrap();
        assert_eq!(inv.substrates.len(), 2);
        let edge1 = &inv.substrates[&SubstrateAlias::new("edge-1")];
        assert_eq!(edge1.did, "did:key:z6MkExampleNodeA");
        assert_eq!(edge1.api_url.as_deref(), Some("http://localhost:7961"));
        assert_eq!(edge1.identity.as_deref(), Some("operator"));
        assert_eq!(edge1.ucan, Some(PathBuf::from("grants/edge-1.json")));
        assert_eq!(edge1.capabilities, Some(BTreeSet::from([ServiceType::Wasm, ServiceType::Tcp])));

        let edge2 = &inv.substrates[&SubstrateAlias::new("edge-2")];
        assert_eq!(edge2.api_url, None);
        assert_eq!(edge2.capabilities, None);
    }

    #[test]
    fn an_unknown_alias_error_lists_the_aliases_the_file_does_define() {
        let toml_str = r#"
            [substrates.edge-1]
            did = "did:key:z6MkExampleNodeA"
        "#;
        let inv = SubstrateInventory::from_toml(toml_str).unwrap();
        let err =
            inv.get(&SubstrateAlias::new("edge-2"), Path::new("substrates.toml")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("edge-2"));
        assert!(msg.contains("substrates.toml"));
        assert!(msg.contains("edge-1"));
    }

    #[test]
    fn a_non_did_key_substrate_id_is_rejected_at_parse() {
        let toml_str = r#"
            [substrates.edge-1]
            did = "not-a-did"
        "#;
        let err = SubstrateInventory::from_toml(toml_str).unwrap_err();
        assert!(err.to_string().contains("did:key:"));
    }

    /// D-A3-2 ("never a bare DID") is a `SubstrateAlias` newtype invariant,
    /// and the inventory parses aliases as `BTreeMap` keys -- a different
    /// serde path than the manifest's `PlacementSelector` value, which is
    /// what every other rejection test here exercises. This pins that the
    /// validator still runs on the map-key path.
    #[test]
    fn a_substrate_alias_key_that_looks_like_a_did_is_rejected_at_parse() {
        let toml_str = r#"
            [substrates."did:key:z6MkExampleNodeA"]
            did = "did:key:z6MkExampleNodeB"
        "#;
        let err = SubstrateInventory::from_toml(toml_str).unwrap_err();
        assert!(err.to_string().contains("did:"), "{err}");
    }

    fn dummy_service(
        name: &str,
        service_type: ServiceType,
        substrate: Option<&str>,
    ) -> PlannedService {
        PlannedService {
            service_id: ServiceId::new(format!("did:key:{name}")),
            logical_ref: LogicalServiceRef {
                app_instance_id: AppInstanceId::new("inst"),
                service_name: LogicalServiceName::new(name),
            },
            substrate: substrate.map(SubstrateAlias::new),
            config: ServiceConfig {
                service_type,
                source: "src".to_string(),
                hash: None,
                interfaces: vec![],
                env: BTreeMap::new(),
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: Default::default(),
                fdae: None,
                health_check: None,
            },
            resolved_dependencies: BTreeMap::new(),
            topology_mode: TopologyMode::Singleton,
            member_index: 0,
            schedule: None,
            sharding_strategy: None,
        }
    }

    fn dummy_plan(services: Vec<PlannedService>) -> DeploymentPlan {
        DeploymentPlan {
            app_instance_id: AppInstanceId::new("inst"),
            blueprint_id: AppBlueprintId::new("syneroym:test"),
            version: Version::parse("1.0.0").unwrap(),
            services,
        }
    }

    #[test]
    fn a_service_type_the_substrate_does_not_declare_is_rejected() {
        let toml_str = r#"
            [substrates.edge-1]
            did = "did:key:z6MkExampleNodeA"
            capabilities = ["wasm"]
        "#;
        let inv = SubstrateInventory::from_toml(toml_str).unwrap();
        let plan = dummy_plan(vec![dummy_service("svc", ServiceType::Container, Some("edge-1"))]);
        let demand = placement_demand(&plan);
        let err = check_placement(&inv, &demand, Path::new("substrates.toml")).unwrap_err();
        assert!(err.to_string().contains("does not declare"));
    }

    #[test]
    fn check_placement_reports_every_problem_not_only_the_first() {
        let toml_str = r#"
            [substrates.edge-1]
            did = "did:key:z6MkExampleNodeA"
            capabilities = ["wasm"]
        "#;
        let inv = SubstrateInventory::from_toml(toml_str).unwrap();
        let plan = dummy_plan(vec![
            dummy_service("svc-a", ServiceType::Container, Some("edge-1")),
            dummy_service("svc-b", ServiceType::Wasm, Some("edge-2")),
        ]);
        let demand = placement_demand(&plan);
        let err = check_placement(&inv, &demand, Path::new("substrates.toml")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("does not declare"));
        assert!(msg.contains("no substrate 'edge-2'"));
    }
}
