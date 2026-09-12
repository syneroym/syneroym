use std::{collections::BTreeMap, future::Future, pin::Pin};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::{
    catalog::ManifestCatalog,
    models::{
        AppBlueprintId, AppDependencySpec, AppInstanceId, DeploymentPlan, LogicalServiceName,
        LogicalServiceRef, PlacementSelector, PlannedService, ServiceId, ServiceSpec,
        SynAppManifest, TopologyMode, TopologyVisibility, Visibility,
    },
};

/// Output of the manifest compiler: a set of deployment plans in
/// topological order (dependencies before dependents).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompiledDeployment {
    /// Deployment plans in topological order. Spawned child apps appear
    /// before the apps that depend on them.
    pub plans: Vec<DeploymentPlan>,
}

/// Compiles a `SynAppManifest` into a `CompiledDeployment` plan.
pub async fn compile(
    root_instance_id: AppInstanceId,
    root_manifest: &SynAppManifest,
    catalog: &dyn ManifestCatalog,
) -> Result<CompiledDeployment> {
    let mut plans = Vec::new();
    let mut blueprint_stack = Vec::new();
    let mut compilation_stack = Vec::new();

    compile_recursive(
        &root_instance_id,
        root_manifest,
        catalog,
        None,
        &mut blueprint_stack,
        &mut compilation_stack,
        &mut plans,
    )
    .await?;

    for plan in &plans {
        validate_plan_visibility(plan)
            .map_err(|errs| anyhow!("Plan visibility validation failed: {}", errs.join("; ")))?;
    }

    Ok(CompiledDeployment { plans })
}

/// Refuses a plan whose visibility declarations contradict its own placement
/// (ADR-0018 + ADR-0022 §5). Operates on the **plan**, not the
/// manifest: a supervisor receives `plan-json` through `submit` and never
/// sees a `SynAppManifest`, so a manifest-level check would leave that whole
/// path silent.
///
/// Two checks:
/// (a) a service placed on a **different, explicitly named** substrate from
///     a service that `depends_on` it, while that dependency declares
///     `visibility = private` -- a private record is never registered, so
///     the dependency could never resolve to an address.
/// (b) a service declaring `topology_visibility = open` while declaring
///     `visibility = private` -- a caller receives a signed member list and
///     then has nothing it can dial.
pub fn validate_plan_visibility(plan: &DeploymentPlan) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    let by_id: BTreeMap<&ServiceId, &PlannedService> =
        plan.services.iter().map(|s| (&s.service_id, s)).collect();

    for service in &plan.services {
        // (a) -- only fires when both placements are explicitly named and
        // differ. `None` means "wherever this deploy was aimed", which this
        // function cannot resolve; refusing on a maybe would reject working
        // plans, so the runtime failure stays the backstop for that case.
        for members in service.resolved_dependencies.values() {
            for member_id in members {
                let Some(dep) = by_id.get(member_id) else {
                    continue; // a cross-app member; not this check's business
                };
                if let (Some(svc_substrate), Some(dep_substrate)) =
                    (&service.substrate, &dep.substrate)
                    && svc_substrate != dep_substrate
                    && dep.config.visibility == Visibility::Private
                {
                    errors.push(format!(
                        "'{}' on substrate '{svc_substrate}' depends on '{}' on substrate \
                         '{dep_substrate}', but '{}' declares visibility 'private' -- its \
                         endpoint record is never registered, so this dependency could never \
                         resolve to an address. Declare 'internal': registered with the community \
                         registry, not propagated upward, which is what a cross-substrate member \
                         needs",
                        service.logical_ref.service_name,
                        dep.logical_ref.service_name,
                        dep.logical_ref.service_name
                    ));
                }
            }
        }

        // (b) -- open topology visibility with a private service.
        if service.topology_visibility == TopologyVisibility::Open
            && service.config.visibility == Visibility::Private
        {
            errors.push(format!(
                "'{}' declares topology_visibility 'open' but visibility '{}' -- an outside \
                 caller would receive its member list and then be unable to resolve any member to \
                 an address, because a private member is never registered. Declare visibility \
                 'internal' alongside it",
                service.logical_ref.service_name,
                service.config.visibility.as_str()
            ));
        }
    }

    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

fn compile_recursive<'a>(
    instance_id: &'a AppInstanceId,
    manifest: &'a SynAppManifest,
    catalog: &'a dyn ManifestCatalog,
    inherited_placement: Option<&'a PlacementSelector>,
    blueprint_stack: &'a mut Vec<AppBlueprintId>,
    compilation_stack: &'a mut Vec<AppInstanceId>,
    plans: &'a mut Vec<DeploymentPlan>,
) -> Pin<Box<dyn Future<Output = Result<()>> + 'a + Send>> {
    Box::pin(async move {
        // Check blueprint cycle (recursive Spawn cycle)
        if blueprint_stack.contains(&manifest.id) {
            return Err(anyhow!(
                "Circular Spawn dependency detected for blueprint '{}'",
                manifest.id
            ));
        }
        // Check instance cycle (Bind cycle)
        if compilation_stack.contains(instance_id) {
            return Err(anyhow!("Circular dependency detected involving instance '{instance_id}'"));
        }

        blueprint_stack.push(manifest.id.clone());
        compilation_stack.push(instance_id.clone());

        // This manifest's own default wins; otherwise the root's cascades in.
        let default_placement = manifest.placement.as_ref().or(inherited_placement);

        // Recursively compile spawned dependencies first
        for (dep_name, dep_spec) in &manifest.dependencies {
            match dep_spec {
                AppDependencySpec::Spawn { blueprint, manifest_path } => {
                    let child_instance_id = AppInstanceId::new(format!("{instance_id}:{dep_name}"));
                    let child_manifest =
                        catalog.resolve(blueprint, manifest_path.as_deref()).await?;
                    compile_recursive(
                        &child_instance_id,
                        &child_manifest,
                        catalog,
                        default_placement,
                        blueprint_stack,
                        compilation_stack,
                        plans,
                    )
                    .await?;
                }
                AppDependencySpec::Bind { instance } => {
                    // If the target instance we bind to is in the active compilation stack, that's
                    // a cycle!
                    if compilation_stack.contains(instance) {
                        return Err(anyhow!(
                            "Circular Spawn vs Bind dependency detected: instance '{instance_id}' \
                             binds to '{instance}' which is still compiling"
                        ));
                    }
                }
            }
        }

        // Now compile the services for this app instance
        let mut services = Vec::new();

        // Sort local services topologically based on depends_on
        let sorted_service_names = sort_services(&manifest.services)?;

        for name in sorted_service_names {
            let spec = manifest
                .services
                .get(&name)
                .ok_or_else(|| anyhow!("Service spec not found for '{name}'"))?;

            let logical_ref = LogicalServiceRef {
                app_instance_id: instance_id.clone(),
                service_name: name.clone(),
            };

            // `replicas > 1` compiles to `Redundant`; `Sharded` stays
            // unreachable until a `ShardingStrategy` manifest surface
            // exists. `validate()` already refused `replicas == 0`, so this
            // is exactly the member count to emit.
            let topology_mode =
                if spec.replicas > 1 { TopologyMode::Redundant } else { TopologyMode::default() };

            // A dependent's `resolved_dependencies` names *every* member of
            // its dependency, since a binding write reaches
            // one member's `service_bindings` row at a time and each is its
            // own `service_id`.
            let resolved_dependencies: BTreeMap<LogicalServiceName, Vec<ServiceId>> = spec
                .depends_on
                .iter()
                .map(|dep| {
                    let dep_ref = LogicalServiceRef {
                        app_instance_id: instance_id.clone(),
                        service_name: dep.clone(),
                    };
                    // `dep` is guaranteed present: `validate()` already
                    // refused any `depends_on` naming an undefined service.
                    let dep_member_count =
                        manifest.services.get(dep).map_or(1, |dep_spec| dep_spec.replicas);
                    let members = (0..dep_member_count)
                        .map(|dep_index| derive_deterministic_service_id(&dep_ref, dep_index))
                        .collect();
                    (dep.clone(), members)
                })
                .collect();

            for member_index in 0..spec.replicas {
                // Deterministic ServiceId generation via sha2 + z32
                let service_id = derive_deterministic_service_id(&logical_ref, member_index);
                services.push(PlannedService {
                    service_id,
                    logical_ref: logical_ref.clone(),
                    substrate: spec
                        .placement
                        .as_ref()
                        .or(default_placement)
                        .map(|p| p.alias().clone()),
                    config: spec.config.clone(),
                    resolved_dependencies: resolved_dependencies.clone(),
                    topology_mode,
                    member_index,
                    schedule: spec.schedule.clone(),
                    sharding_strategy: spec.sharding_strategy.clone(),
                    topology_visibility: spec.topology_visibility,
                });
            }
        }

        plans.push(DeploymentPlan {
            app_instance_id: instance_id.clone(),
            blueprint_id: manifest.id.clone(),
            version: manifest.version.clone(),
            services,
        });

        compilation_stack.pop();
        blueprint_stack.pop();

        Ok(())
    })
}

/// Derives a deterministic `ServiceId` for one member of a logical service
/// reference.
///
/// **TODO:** This is a temporary hack that forcefully prepends the
/// `ed25519-pub` multicodec prefix to a SHA-256 hash to forge a `did:key`. This
/// produces a mock key where we do not have the private key, and the 32 bytes
/// may not be a valid Curve25519 point.
///
/// This should later be replaced by actual deterministic derivation of valid
/// Ed25519 keypairs (e.g., via HKDF from a seed), where the public key goes
/// into the plan and the private key is injected into the service.
///
/// `member_index` folds into the hash **only above index 0**:
/// without `--mint-masters` there is no substitution step, so this
/// fabricated id *is* the deployed `service_id` for an unmastered deploy, and
/// changing what index 0 hashes to would silently re-identify every existing
/// unmastered deployment out from under `diff_plans`, which keys on the
/// logical ref alone and would read the new id as an `Update` rather than a
/// `Remove`+`Add`.
fn derive_deterministic_service_id(
    logical_ref: &LogicalServiceRef,
    member_index: u32,
) -> ServiceId {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(logical_ref.to_string().as_bytes());
    if member_index > 0 {
        hasher.update(b"#");
        hasher.update(member_index.to_string().as_bytes());
    }
    let hash = hasher.finalize();
    let mut bytes = vec![0xed, 0x01]; // multicodec ed25519-pub
    bytes.extend_from_slice(&hash);
    ServiceId::new(format!("did:key:h{}", z32::encode(&bytes)))
}

fn sort_services(
    services: &BTreeMap<LogicalServiceName, ServiceSpec>,
) -> Result<Vec<LogicalServiceName>> {
    let mut visited = BTreeMap::new();
    let mut order = Vec::new();

    for name in services.keys() {
        visited.insert(name.clone(), false);
    }

    fn dfs(
        node: &LogicalServiceName,
        services: &BTreeMap<LogicalServiceName, ServiceSpec>,
        visited: &mut BTreeMap<LogicalServiceName, bool>,
        order: &mut Vec<LogicalServiceName>,
    ) {
        if *visited.get(node).unwrap_or(&false) {
            return;
        }

        visited.insert(node.clone(), true);

        if let Some(spec) = services.get(node) {
            for dep in &spec.depends_on {
                dfs(dep, services, visited, order);
            }
        }

        order.push(node.clone());
    }

    for name in services.keys() {
        if !visited.get(name).unwrap_or(&false) {
            dfs(name, services, &mut visited, &mut order);
        }
    }

    Ok(order)
}

#[cfg(test)]
mod tests;
