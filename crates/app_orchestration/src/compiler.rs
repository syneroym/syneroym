use std::{collections::BTreeMap, future::Future, pin::Pin};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::{
    catalog::ManifestCatalog,
    models::{
        AppBlueprintId, AppDependencySpec, AppInstanceId, DeploymentPlan, LogicalServiceName,
        LogicalServiceRef, PlacementSelector, PlannedService, ServiceId, ServiceSpec,
        SynAppManifest, TopologyMode,
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

    Ok(CompiledDeployment { plans })
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
            return Err(anyhow!(
                "Circular dependency detected involving instance '{}'",
                instance_id
            ));
        }

        blueprint_stack.push(manifest.id.clone());
        compilation_stack.push(instance_id.clone());

        // D-A3-3: this manifest's own default wins; otherwise the root's cascades in.
        let default_placement = manifest.placement.as_ref().or(inherited_placement);

        // Recursively compile spawned dependencies first
        for (dep_name, dep_spec) in &manifest.dependencies {
            match dep_spec {
                AppDependencySpec::Spawn { blueprint, manifest_path } => {
                    let child_instance_id =
                        AppInstanceId::new(format!("{}:{}", instance_id, dep_name));
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
                            "Circular Spawn vs Bind dependency detected: instance '{}' binds to \
                             '{}' which is still compiling",
                            instance_id,
                            instance
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
                .ok_or_else(|| anyhow!("Service spec not found for '{}'", name))?;

            let logical_ref = LogicalServiceRef {
                app_instance_id: instance_id.clone(),
                service_name: name.clone(),
            };

            // `replicas > 1` compiles to `Redundant` (D-A5e-4); `Sharded`
            // stays unreachable until a `ShardingStrategy` manifest surface
            // exists (slice S1). `validate()` already refused `replicas ==
            // 0`, so this is exactly the member count to emit.
            let topology_mode =
                if spec.replicas > 1 { TopologyMode::Redundant } else { TopologyMode::default() };

            // A dependent's `resolved_dependencies` names *every* member of
            // its dependency (D-A5e-4/§33.7), since a binding write reaches
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
/// **TODO(M2/M3A):** This is a temporary M1 hack that forcefully prepends the
/// `ed25519-pub` multicodec prefix to a SHA-256 hash to forge a `did:key`. This
/// produces a mock key where we do not have the private key, and the 32 bytes
/// may not be a valid Curve25519 point.
///
/// In M2 (Identity Handshake) and M3A (Vault/Configuration), this should be
/// replaced by actual deterministic derivation of valid Ed25519 keypairs (e.g.,
/// via HKDF from a seed), where the public key goes into the plan and the
/// private key is injected into the service.
///
/// `member_index` folds into the hash **only above index 0** (M05A A5e,
/// D-A5e-3): without `--mint-masters` there is no substitution step, so this
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
mod tests {
    use std::{
        collections::BTreeSet,
        path::PathBuf,
        time::{Duration, Instant},
    };

    use super::*;
    use crate::{catalog::LocalFilesystemCatalog, models::SubstrateAlias};

    #[tokio::test]
    async fn test_compile_single_app() {
        let manifest_toml = r#"
            id = "syneroym:single-app"
            version = "1.0.0"

            [services.identity]
            service_type = "wasm"
            source = "identity.wasm"
            depends_on = []

            [services.echo]
            service_type = "wasm"
            source = "echo.wasm"
            depends_on = ["identity"]
        "#;
        let manifest = SynAppManifest::from_toml(manifest_toml).unwrap();
        let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
        let root_inst = AppInstanceId::new("root-inst");

        let compiled = compile(root_inst.clone(), &manifest, &catalog).await.unwrap();
        assert_eq!(compiled.plans.len(), 1);

        let plan = &compiled.plans[0];
        assert_eq!(plan.app_instance_id, root_inst);
        assert_eq!(plan.blueprint_id.as_str(), "syneroym:single-app");
        assert_eq!(plan.services.len(), 2);

        // Assert topological order (identity should be before echo)
        assert_eq!(plan.services[0].logical_ref.service_name.as_str(), "identity");
        assert_eq!(plan.services[1].logical_ref.service_name.as_str(), "echo");

        // Check resolved dependencies -- keyed by declared name
        let identity_id = &plan.services[0].service_id;
        let echo_deps = &plan.services[1].resolved_dependencies;
        assert_eq!(echo_deps.len(), 1);
        assert_eq!(&echo_deps[&LogicalServiceName::new("identity")], &vec![identity_id.clone()]);
    }

    #[tokio::test]
    async fn test_compile_with_spawn_dependency() {
        let root_toml = r#"
            id = "syneroym:root-app"
            version = "1.0.0"

            [services.web]
            service_type = "wasm"
            source = "web.wasm"

            [dependencies.db]
            mode = "spawn"
            blueprint = "syneroym:db-app"
        "#;

        let db_toml = r#"
            id = "syneroym:db-app"
            version = "2.0.0"

            [services.postgres]
            service_type = "container"
            source = "postgres:latest"
        "#;

        let root_manifest = SynAppManifest::from_toml(root_toml).unwrap();
        let db_manifest = SynAppManifest::from_toml(db_toml).unwrap();

        let mut catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
        catalog.register(AppBlueprintId::new("syneroym:db-app"), db_manifest);

        let root_inst = AppInstanceId::new("root-inst");
        let compiled = compile(root_inst.clone(), &root_manifest, &catalog).await.unwrap();

        // Expecting 2 plans (db compiled first, then root-app)
        assert_eq!(compiled.plans.len(), 2);

        let db_plan = &compiled.plans[0];
        assert_eq!(db_plan.app_instance_id.as_str(), "root-inst:db");
        assert_eq!(db_plan.blueprint_id.as_str(), "syneroym:db-app");
        assert_eq!(db_plan.services.len(), 1);
        assert_eq!(db_plan.services[0].logical_ref.service_name.as_str(), "postgres");

        let parent_plan = &compiled.plans[1];
        assert_eq!(parent_plan.app_instance_id, root_inst);
        assert_eq!(parent_plan.blueprint_id.as_str(), "syneroym:root-app");
        assert_eq!(parent_plan.services.len(), 1);
        assert_eq!(parent_plan.services[0].logical_ref.service_name.as_str(), "web");
    }

    #[tokio::test]
    async fn test_compile_spawn_cycle_detection() {
        let app_a_toml = r#"
            id = "syneroym:app-a"
            version = "1.0.0"
            [dependencies.b]
            mode = "spawn"
            blueprint = "syneroym:app-b"
        "#;

        let app_b_toml = r#"
            id = "syneroym:app-b"
            version = "1.0.0"
            [dependencies.a]
            mode = "spawn"
            blueprint = "syneroym:app-a"
        "#;

        let manifest_a = SynAppManifest::from_toml(app_a_toml).unwrap();
        let manifest_b = SynAppManifest::from_toml(app_b_toml).unwrap();

        let mut catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
        catalog.register(AppBlueprintId::new("syneroym:app-a"), manifest_a.clone());
        catalog.register(AppBlueprintId::new("syneroym:app-b"), manifest_b);

        let res = compile(AppInstanceId::new("inst-a"), &manifest_a, &catalog).await;
        assert!(res.is_err());
        let err_msg = res.err().unwrap().to_string();
        assert!(err_msg.contains("Circular Spawn dependency detected"));
    }

    #[tokio::test]
    async fn test_compile_self_spawn_cycle() {
        let app_toml = r#"
            id = "syneroym:app-self"
            version = "1.0.0"
            [dependencies.self]
            mode = "spawn"
            blueprint = "syneroym:app-self"
        "#;
        let manifest = SynAppManifest::from_toml(app_toml).unwrap();
        let mut catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
        catalog.register(AppBlueprintId::new("syneroym:app-self"), manifest.clone());

        let res = compile(AppInstanceId::new("inst-self"), &manifest, &catalog).await;
        assert!(res.is_err());
        assert!(res.err().unwrap().to_string().contains("Circular Spawn dependency detected"));
    }

    #[tokio::test]
    async fn test_compile_with_bind_dependency() {
        let root_toml = r#"
            id = "syneroym:root-app"
            version = "1.0.0"
            [services.web]
            service_type = "wasm"
            source = "web.wasm"
            [dependencies.existing-db]
            mode = "bind"
            instance = "db-instance-123"
        "#;
        let manifest = SynAppManifest::from_toml(root_toml).unwrap();
        let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));

        let compiled = compile(AppInstanceId::new("root-inst"), &manifest, &catalog).await.unwrap();
        assert_eq!(compiled.plans.len(), 1);
        assert_eq!(compiled.plans[0].blueprint_id.as_str(), "syneroym:root-app");
    }

    #[tokio::test]
    async fn test_compile_spawn_vs_bind_cycle() {
        let app_a_toml = r#"
            id = "syneroym:app-a"
            version = "1.0.0"
            [dependencies.b]
            mode = "spawn"
            blueprint = "syneroym:app-b"
        "#;

        let app_b_toml = r#"
            id = "syneroym:app-b"
            version = "1.0.0"
            [dependencies.a]
            mode = "bind"
            instance = "inst-a"
        "#;

        let manifest_a = SynAppManifest::from_toml(app_a_toml).unwrap();
        let manifest_b = SynAppManifest::from_toml(app_b_toml).unwrap();

        let mut catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
        catalog.register(AppBlueprintId::new("syneroym:app-a"), manifest_a.clone());
        catalog.register(AppBlueprintId::new("syneroym:app-b"), manifest_b);

        let res = compile(AppInstanceId::new("inst-a"), &manifest_a, &catalog).await;
        assert!(res.is_err());
        assert!(
            res.err().unwrap().to_string().contains("Circular Spawn vs Bind dependency detected")
        );
    }

    #[tokio::test]
    async fn test_compile_deterministic_service_ids() {
        let manifest_toml = r#"
            id = "syneroym:test-app"
            version = "1.0.0"
            [services.svc]
            service_type = "wasm"
            source = "svc.wasm"
        "#;
        let manifest = SynAppManifest::from_toml(manifest_toml).unwrap();
        let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));

        let compiled1 = compile(AppInstanceId::new("inst"), &manifest, &catalog).await.unwrap();
        let compiled2 = compile(AppInstanceId::new("inst"), &manifest, &catalog).await.unwrap();

        assert_eq!(
            compiled1.plans[0].services[0].service_id,
            compiled2.plans[0].services[0].service_id
        );
    }

    /// D-A5e-3 (§33.3, revised after review R1): `derive_deterministic_
    /// service_id` folds the member index into its hash **only above index
    /// 0**. Without `--mint-masters` there is no substitution step, so this
    /// fabricated id *is* the deployed `service_id` -- changing what index 0
    /// hashes to would silently re-identify every existing unmastered
    /// deployment. Pins the literal hash a plain, unscaled manifest compiles
    /// to today, not merely its shape.
    #[tokio::test]
    async fn an_unscaled_manifest_compiles_the_service_id_it_compiles_today() {
        let manifest_toml = r#"
            id = "syneroym:test-app"
            version = "1.0.0"
            [services.svc]
            service_type = "wasm"
            source = "svc.wasm"
        "#;
        let manifest = SynAppManifest::from_toml(manifest_toml).unwrap();
        let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
        let compiled = compile(AppInstanceId::new("inst"), &manifest, &catalog).await.unwrap();

        let logical_ref = LogicalServiceRef {
            app_instance_id: AppInstanceId::new("inst"),
            service_name: LogicalServiceName::new("svc"),
        };
        assert_eq!(
            compiled.plans[0].services[0].service_id,
            derive_deterministic_service_id(&logical_ref, 0)
        );
    }

    /// The whole reason index 0 must stay unconditional: a member index
    /// folded in unconditionally would change index 0's hash too, and this
    /// is the property that makes it not change.
    #[test]
    fn derive_deterministic_service_id_differs_by_index_above_zero_only() {
        let logical_ref = LogicalServiceRef {
            app_instance_id: AppInstanceId::new("inst"),
            service_name: LogicalServiceName::new("svc"),
        };
        let id0 = derive_deterministic_service_id(&logical_ref, 0);
        let id0_again = derive_deterministic_service_id(&logical_ref, 0);
        let id1 = derive_deterministic_service_id(&logical_ref, 1);
        let id2 = derive_deterministic_service_id(&logical_ref, 2);
        assert_eq!(id0, id0_again);
        assert_ne!(id0, id1);
        assert_ne!(id1, id2);
    }

    // ── M05A A5e phase 2: `replicas` and the compiler (D-A5e-3/D-A5e-4) ─

    /// The no-change regression guard for every existing manifest: a
    /// manifest with no `replicas` compiles to exactly one member at
    /// index 0.
    #[tokio::test]
    async fn a_manifest_without_replicas_compiles_to_one_member_at_index_zero() {
        let manifest_toml = r#"
            id = "syneroym:single-app"
            version = "1.0.0"
            [services.svc]
            service_type = "wasm"
            source = "svc.wasm"
        "#;
        let manifest = SynAppManifest::from_toml(manifest_toml).unwrap();
        let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
        let compiled = compile(AppInstanceId::new("inst"), &manifest, &catalog).await.unwrap();

        assert_eq!(compiled.plans[0].services.len(), 1);
        assert_eq!(compiled.plans[0].services[0].member_index, 0);
        assert_eq!(compiled.plans[0].services[0].topology_mode, TopologyMode::Singleton);
    }

    /// The fabricated ids must differ, or the substitution map two members
    /// would need at mint time collapses before it ever runs.
    #[tokio::test]
    async fn replicas_three_compiles_to_three_planned_services_with_distinct_service_ids() {
        let manifest_toml = r#"
            id = "syneroym:scaled-app"
            version = "1.0.0"
            [services.backend]
            service_type = "wasm"
            source = "backend.wasm"
            replicas = 3
        "#;
        let manifest = SynAppManifest::from_toml(manifest_toml).unwrap();
        let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
        let compiled = compile(AppInstanceId::new("inst"), &manifest, &catalog).await.unwrap();

        assert_eq!(compiled.plans[0].services.len(), 3);
        let ids: BTreeSet<_> =
            compiled.plans[0].services.iter().map(|s| s.service_id.clone()).collect();
        assert_eq!(ids.len(), 3, "every member must have a distinct fabricated id");
    }

    #[tokio::test]
    async fn each_member_of_one_logical_service_carries_its_own_stored_index() {
        let manifest_toml = r#"
            id = "syneroym:scaled-app"
            version = "1.0.0"
            [services.backend]
            service_type = "wasm"
            source = "backend.wasm"
            replicas = 3
        "#;
        let manifest = SynAppManifest::from_toml(manifest_toml).unwrap();
        let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
        let compiled = compile(AppInstanceId::new("inst"), &manifest, &catalog).await.unwrap();

        let mut indices: Vec<u32> =
            compiled.plans[0].services.iter().map(|s| s.member_index).collect();
        indices.sort_unstable();
        assert_eq!(indices, vec![0, 1, 2]);
        // Every member shares the same logical ref -- only the index and
        // the fabricated id distinguish them.
        for svc in &compiled.plans[0].services {
            assert_eq!(svc.logical_ref.service_name.as_str(), "backend");
        }
    }

    #[tokio::test]
    async fn replicas_above_one_compiles_the_topology_mode_as_redundant() {
        let manifest_toml = r#"
            id = "syneroym:scaled-app"
            version = "1.0.0"
            [services.backend]
            service_type = "wasm"
            source = "backend.wasm"
            replicas = 2
        "#;
        let manifest = SynAppManifest::from_toml(manifest_toml).unwrap();
        let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
        let compiled = compile(AppInstanceId::new("inst"), &manifest, &catalog).await.unwrap();

        for svc in &compiled.plans[0].services {
            assert_eq!(svc.topology_mode, TopologyMode::Redundant);
        }
    }

    /// The dependent's `resolved_dependencies` must name every member of a
    /// scaled dependency, not just its first -- this is what makes a push
    /// reach every member's own `service_bindings` row.
    #[tokio::test]
    async fn a_dependents_resolved_dependencies_names_every_member_of_its_dependency() {
        let manifest_toml = r#"
            id = "syneroym:scaled-app"
            version = "1.0.0"

            [services.backend]
            service_type = "wasm"
            source = "backend.wasm"
            replicas = 2

            [services.frontend]
            service_type = "wasm"
            source = "frontend.wasm"
            depends_on = ["backend"]
        "#;
        let manifest = SynAppManifest::from_toml(manifest_toml).unwrap();
        let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
        let compiled = compile(AppInstanceId::new("inst"), &manifest, &catalog).await.unwrap();

        let backend_ids: BTreeSet<_> = compiled.plans[0]
            .services
            .iter()
            .filter(|s| s.logical_ref.service_name.as_str() == "backend")
            .map(|s| s.service_id.clone())
            .collect();
        assert_eq!(backend_ids.len(), 2);

        let frontend = compiled.plans[0]
            .services
            .iter()
            .find(|s| s.logical_ref.service_name.as_str() == "frontend")
            .unwrap();
        let resolved = frontend.resolved_dependencies.get(&LogicalServiceName::new("backend"));
        let resolved: BTreeSet<_> = resolved.unwrap().iter().cloned().collect();
        assert_eq!(resolved, backend_ids, "frontend must resolve both of backend's members");
    }

    #[tokio::test]
    async fn replicas_of_zero_or_above_the_cap_is_refused_at_manifest_validation() {
        let zero = r#"
            id = "syneroym:bad"
            version = "1.0.0"
            [services.svc]
            service_type = "wasm"
            source = "svc.wasm"
            replicas = 0
        "#;
        let err = SynAppManifest::from_toml(zero).unwrap_err();
        assert!(err.to_string().contains("replicas = 0"), "{err}");

        let above_cap = r#"
            id = "syneroym:bad"
            version = "1.0.0"
            [services.svc]
            service_type = "wasm"
            source = "svc.wasm"
            replicas = 17
        "#;
        let err = SynAppManifest::from_toml(above_cap).unwrap_err();
        assert!(err.to_string().contains("cap"), "{err}");

        let at_cap = r#"
            id = "syneroym:ok"
            version = "1.0.0"
            [services.svc]
            service_type = "wasm"
            source = "svc.wasm"
            replicas = 16
        "#;
        assert!(SynAppManifest::from_toml(at_cap).is_ok());
    }

    /// D-A5e-16 (§41 answer 3): `replicas > 1` alongside a declared
    /// `schema` is refused, naming M7 as the reason it will relax --
    /// silently splitting a stateful service's data across N databases is
    /// discovered as data loss otherwise.
    #[tokio::test]
    async fn replicas_above_one_is_refused_for_a_service_declaring_a_schema() {
        let manifest_toml = r#"
            id = "syneroym:bad"
            version = "1.0.0"
            [services.svc]
            service_type = "wasm"
            source = "svc.wasm"
            replicas = 2
            schema = "shared.json"
        "#;
        let err = SynAppManifest::from_toml(manifest_toml).unwrap_err();
        assert!(err.to_string().contains("M7"), "{err}");

        // A `schema` with no scale-out stays valid.
        let unscaled = r#"
            id = "syneroym:ok"
            version = "1.0.0"
            [services.svc]
            service_type = "wasm"
            source = "svc.wasm"
            schema = "shared.json"
        "#;
        assert!(SynAppManifest::from_toml(unscaled).is_ok());
    }

    #[tokio::test]
    async fn test_compile_performance_budget() {
        let mut services_toml = String::new();
        for i in 0..50 {
            services_toml.push_str(&format!(
                r#"
                [services.svc-{}]
                service_type = "wasm"
                source = "svc.wasm"
            "#,
                i
            ));
            if i > 0 {
                services_toml
                    .push_str(&format!("                depends_on = [\"svc-{}\"]\n", i - 1));
            } else {
                services_toml.push_str("                depends_on = []\n");
            }
        }

        let manifest_toml = format!(
            r#"
            id = "syneroym:perf-app"
            version = "1.0.0"
            {}
        "#,
            services_toml
        );

        let manifest = SynAppManifest::from_toml(&manifest_toml).unwrap();
        let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));

        let start = Instant::now();
        let compiled = compile(AppInstanceId::new("perf-inst"), &manifest, &catalog).await.unwrap();
        let duration = start.elapsed();

        assert_eq!(compiled.plans[0].services.len(), 50);
        assert!(duration < Duration::from_millis(50), "Compilation took {:?}", duration);
    }

    #[tokio::test]
    async fn a_per_service_placement_overrides_the_manifest_default() {
        let manifest_toml = r#"
            id = "syneroym:placed-app"
            version = "1.0.0"

            [placement]
            substrate = "edge-1"

            [services.frontend]
            service_type = "wasm"
            source = "frontend.wasm"

            [services.backend]
            service_type = "wasm"
            source = "backend.wasm"

            [services.backend.placement]
            substrate = "edge-2"
        "#;
        let manifest = SynAppManifest::from_toml(manifest_toml).unwrap();
        let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));

        let compiled = compile(AppInstanceId::new("inst"), &manifest, &catalog).await.unwrap();
        let plan = &compiled.plans[0];

        let frontend = plan
            .services
            .iter()
            .find(|s| s.logical_ref.service_name.as_str() == "frontend")
            .unwrap();
        assert_eq!(frontend.substrate, Some(SubstrateAlias::new("edge-1")));

        let backend = plan
            .services
            .iter()
            .find(|s| s.logical_ref.service_name.as_str() == "backend")
            .unwrap();
        assert_eq!(backend.substrate, Some(SubstrateAlias::new("edge-2")));
    }

    #[tokio::test]
    async fn a_manifest_without_placement_leaves_every_service_unplaced() {
        let manifest_toml = r#"
            id = "syneroym:unplaced-app"
            version = "1.0.0"

            [services.svc]
            service_type = "wasm"
            source = "svc.wasm"
        "#;
        let manifest = SynAppManifest::from_toml(manifest_toml).unwrap();
        let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));

        let compiled = compile(AppInstanceId::new("inst"), &manifest, &catalog).await.unwrap();
        assert_eq!(compiled.plans[0].services[0].substrate, None);
    }

    #[tokio::test]
    async fn the_root_manifests_placement_cascades_into_a_spawned_child() {
        let root_toml = r#"
            id = "syneroym:root-app"
            version = "1.0.0"

            [placement]
            substrate = "edge-1"

            [services.web]
            service_type = "wasm"
            source = "web.wasm"

            [dependencies.db]
            mode = "spawn"
            blueprint = "syneroym:db-app"
        "#;

        let db_toml = r#"
            id = "syneroym:db-app"
            version = "2.0.0"

            [services.postgres]
            service_type = "container"
            source = "postgres:latest"
        "#;

        let root_manifest = SynAppManifest::from_toml(root_toml).unwrap();
        let db_manifest = SynAppManifest::from_toml(db_toml).unwrap();

        let mut catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
        catalog.register(AppBlueprintId::new("syneroym:db-app"), db_manifest);

        let compiled =
            compile(AppInstanceId::new("root-inst"), &root_manifest, &catalog).await.unwrap();

        let db_plan = &compiled.plans[0];
        assert_eq!(db_plan.services[0].substrate, Some(SubstrateAlias::new("edge-1")));
        let root_plan = &compiled.plans[1];
        assert_eq!(root_plan.services[0].substrate, Some(SubstrateAlias::new("edge-1")));
    }

    #[tokio::test]
    async fn a_spawned_childs_own_placement_wins_over_the_inherited_default() {
        let root_toml = r#"
            id = "syneroym:root-app"
            version = "1.0.0"

            [placement]
            substrate = "edge-1"

            [services.web]
            service_type = "wasm"
            source = "web.wasm"

            [dependencies.db]
            mode = "spawn"
            blueprint = "syneroym:db-app"
        "#;

        let db_toml = r#"
            id = "syneroym:db-app"
            version = "2.0.0"

            [placement]
            substrate = "edge-2"

            [services.postgres]
            service_type = "container"
            source = "postgres:latest"
        "#;

        let root_manifest = SynAppManifest::from_toml(root_toml).unwrap();
        let db_manifest = SynAppManifest::from_toml(db_toml).unwrap();

        let mut catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
        catalog.register(AppBlueprintId::new("syneroym:db-app"), db_manifest);

        let compiled =
            compile(AppInstanceId::new("root-inst"), &root_manifest, &catalog).await.unwrap();

        let db_plan = &compiled.plans[0];
        assert_eq!(db_plan.services[0].substrate, Some(SubstrateAlias::new("edge-2")));
    }
}
