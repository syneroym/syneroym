use std::{collections::BTreeMap, future::Future, pin::Pin};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::{
    catalog::ManifestCatalog,
    models::{
        AppBlueprintId, AppDependencySpec, AppInstanceId, DeploymentPlan, LogicalServiceName,
        LogicalServiceRef, PlannedService, ServiceId, ServiceSpec, SynAppManifest, TopologyMode,
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

            // Deterministic ServiceId generation via sha2 + z32
            let service_id = derive_deterministic_service_id(&logical_ref);

            let resolved_dependencies = spec
                .depends_on
                .iter()
                .map(|dep| {
                    let dep_ref = LogicalServiceRef {
                        app_instance_id: instance_id.clone(),
                        service_name: dep.clone(),
                    };
                    derive_deterministic_service_id(&dep_ref)
                })
                .collect();

            services.push(PlannedService {
                service_id,
                logical_ref,
                config: spec.config.clone(),
                resolved_dependencies,
                topology_mode: TopologyMode::default(),
            });
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

/// Derives a deterministic `ServiceId` for a logical service reference.
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
fn derive_deterministic_service_id(logical_ref: &LogicalServiceRef) -> ServiceId {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(logical_ref.to_string().as_bytes());
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
        path::PathBuf,
        time::{Duration, Instant},
    };

    use super::*;
    use crate::catalog::LocalFilesystemCatalog;

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

        // Check resolved dependencies
        let identity_id = &plan.services[0].service_id;
        let echo_deps = &plan.services[1].resolved_dependencies;
        assert_eq!(echo_deps.len(), 1);
        assert_eq!(&echo_deps[0], identity_id);
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
}
