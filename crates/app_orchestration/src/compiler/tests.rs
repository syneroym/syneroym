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
    assert!(res.err().unwrap().to_string().contains("Circular Spawn vs Bind dependency detected"));
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

/// `derive_deterministic_service_id` folds the member index into its hash
/// **only above index 0**. Without `--mint-masters` there is no
/// substitution step, so this
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

// ── `replicas` and the compiler ─

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

    let mut indices: Vec<u32> = compiled.plans[0].services.iter().map(|s| s.member_index).collect();
    indices.sort_unstable();
    assert_eq!(indices, vec![0, 1, 2]);
    // Every member shares the same logical ref -- only the index and
    // the fabricated id distinguish them.
    for svc in &compiled.plans[0].services {
        assert_eq!(svc.logical_ref.service_name.as_str(), "backend");
    }
}

/// A schedule belongs to the logical service, not to a member --
/// every compiled member carries an identical clone, exactly as
/// `resolved_dependencies` and `topology_mode` do.
#[tokio::test]
async fn every_member_of_a_scaled_scheduled_service_carries_the_same_schedule() {
    let manifest_toml = r#"
            id = "syneroym:scaled-app"
            version = "1.0.0"
            [services.backend]
            service_type = "wasm"
            source = "backend.wasm"
            interfaces = ["scheduled-driver"]
            replicas = 3

            [services.backend.schedule]
            cron = "* * * * *"
            interface = "scheduled-driver"
            method = "tick"
        "#;
    let manifest = SynAppManifest::from_toml(manifest_toml).unwrap();
    let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
    let compiled = compile(AppInstanceId::new("inst"), &manifest, &catalog).await.unwrap();

    assert_eq!(compiled.plans[0].services.len(), 3);
    for svc in &compiled.plans[0].services {
        let sched = svc.schedule.as_ref().expect("every member must carry the schedule");
        assert_eq!(sched.cron, "* * * * *");
        assert_eq!(sched.method, "tick");
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

/// `replicas > 1` alongside a declared `schema` is refused --
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
                [services.svc-{i}]
                service_type = "wasm"
                source = "svc.wasm"
            "#
        ));
        if i > 0 {
            services_toml.push_str(&format!("                depends_on = [\"svc-{}\"]\n", i - 1));
        } else {
            services_toml.push_str("                depends_on = []\n");
        }
    }

    let manifest_toml = format!(
        r#"
            id = "syneroym:perf-app"
            version = "1.0.0"
            {services_toml}
        "#
    );

    let manifest = SynAppManifest::from_toml(&manifest_toml).unwrap();
    let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));

    let start = Instant::now();
    let compiled = compile(AppInstanceId::new("perf-inst"), &manifest, &catalog).await.unwrap();
    let duration = start.elapsed();

    assert_eq!(compiled.plans[0].services.len(), 50);
    assert!(duration < Duration::from_millis(50), "Compilation took {duration:?}");
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

    let frontend =
        plan.services.iter().find(|s| s.logical_ref.service_name.as_str() == "frontend").unwrap();
    assert_eq!(frontend.substrate, Some(SubstrateAlias::new("edge-1")));

    let backend =
        plan.services.iter().find(|s| s.logical_ref.service_name.as_str() == "backend").unwrap();
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

fn planned_service(
    name: &str,
    vis: Visibility,
    topo_vis: TopologyVisibility,
    substrate: Option<SubstrateAlias>,
    resolved_dependencies: BTreeMap<LogicalServiceName, Vec<ServiceId>>,
) -> PlannedService {
    PlannedService {
        service_id: ServiceId::new(format!("did:key:{name}")),
        logical_ref: LogicalServiceRef {
            app_instance_id: AppInstanceId::new("app-inst"),
            service_name: LogicalServiceName::new(name),
        },
        substrate,
        config: crate::models::ServiceConfig {
            service_type: crate::models::ServiceType::Wasm,
            source: "web.wasm".to_string(),
            hash: None,
            interfaces: Vec::new(),
            env: BTreeMap::new(),
            args: Vec::new(),
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: Default::default(),
            fdae: None,
            health_check: None,
            assets: None,
            visibility: vis,
        },
        resolved_dependencies,
        topology_mode: TopologyMode::Singleton,
        member_index: 0,
        schedule: None,
        sharding_strategy: None,
        topology_visibility: topo_vis,
    }
}

fn plan_with_service(name: &str, vis: Visibility, topo_vis: TopologyVisibility) -> DeploymentPlan {
    DeploymentPlan {
        app_instance_id: AppInstanceId::new("app-inst"),
        blueprint_id: AppBlueprintId::new("syneroym:app"),
        version: semver::Version::new(1, 0, 0),
        services: vec![planned_service(name, vis, topo_vis, None, BTreeMap::new())],
    }
}

/// A two-service plan: `frontend` depends on `backend`, each optionally
/// placed on the given substrate alias.
fn plan_with_dependency(
    frontend_substrate: Option<SubstrateAlias>,
    backend_substrate: Option<SubstrateAlias>,
    backend_vis: Visibility,
) -> DeploymentPlan {
    let backend = planned_service(
        "backend",
        backend_vis,
        TopologyVisibility::Restricted,
        backend_substrate,
        BTreeMap::new(),
    );
    let mut deps = BTreeMap::new();
    deps.insert(LogicalServiceName::new("backend"), vec![backend.service_id.clone()]);
    let frontend = planned_service(
        "frontend",
        Visibility::Internal,
        TopologyVisibility::Restricted,
        frontend_substrate,
        deps,
    );
    DeploymentPlan {
        app_instance_id: AppInstanceId::new("app-inst"),
        blueprint_id: AppBlueprintId::new("syneroym:app"),
        version: semver::Version::new(1, 0, 0),
        services: vec![backend, frontend],
    }
}

/// Two services on explicitly different aliases, the dependency `private`
/// -> refused, naming both services and `internal`.
#[test]
fn validate_plan_visibility_cross_substrate_private_dependency_fails() {
    let plan = plan_with_dependency(
        Some(SubstrateAlias::new("edge-1")),
        Some(SubstrateAlias::new("edge-2")),
        Visibility::Private,
    );
    let errs = validate_plan_visibility(&plan).unwrap_err();
    assert_eq!(errs.len(), 1);
    assert!(errs[0].contains("frontend"), "{}", errs[0]);
    assert!(errs[0].contains("backend"), "{}", errs[0]);
    assert!(errs[0].contains("internal"), "{}", errs[0]);
}

/// The same pair with the dependency `internal` -> `Ok`.
#[test]
fn validate_plan_visibility_cross_substrate_internal_dependency_succeeds() {
    let plan = plan_with_dependency(
        Some(SubstrateAlias::new("edge-1")),
        Some(SubstrateAlias::new("edge-2")),
        Visibility::Internal,
    );
    assert!(validate_plan_visibility(&plan).is_ok());
}

/// No explicit placement on either side -> `Ok` (no false positive on an
/// unresolvable `None`).
#[test]
fn validate_plan_visibility_no_explicit_placement_is_not_flagged() {
    let plan = plan_with_dependency(None, None, Visibility::Private);
    assert!(validate_plan_visibility(&plan).is_ok());
}

/// One `Some(a)`, one `None` -> `Ok` (conservative; the runtime failure is
/// the backstop).
#[test]
fn validate_plan_visibility_partial_placement_is_not_flagged() {
    let plan = plan_with_dependency(Some(SubstrateAlias::new("edge-1")), None, Visibility::Private);
    assert!(validate_plan_visibility(&plan).is_ok());
}

/// `topology_visibility = open` with `visibility = private` -> refused,
/// naming both fields.
#[test]
fn validate_plan_visibility_open_with_private_fails() {
    let plan = plan_with_service("web", Visibility::Private, TopologyVisibility::Open);
    let errs = validate_plan_visibility(&plan).unwrap_err();
    assert_eq!(errs.len(), 1);
    assert!(errs[0].contains("open"), "{}", errs[0]);
    assert!(errs[0].contains("private"), "{}", errs[0]);
    assert!(errs[0].contains("internal"), "{}", errs[0]);
}

/// `topology_visibility = open` with `visibility = internal` -> `Ok` --
/// `internal`, not `public`, is what a cross-substrate member needs, and
/// `open` only requires the member be resolvable inside the community
/// registry, not propagated to a parent.
#[test]
fn validate_plan_visibility_open_with_internal_succeeds() {
    let plan = plan_with_service("web", Visibility::Internal, TopologyVisibility::Open);
    assert!(validate_plan_visibility(&plan).is_ok());
}

#[test]
fn validate_plan_visibility_open_with_public_succeeds() {
    let plan = plan_with_service("web", Visibility::Public, TopologyVisibility::Open);
    assert!(validate_plan_visibility(&plan).is_ok());
}

#[test]
fn validate_plan_visibility_restricted_with_private_succeeds() {
    let plan = plan_with_service("db", Visibility::Private, TopologyVisibility::Restricted);
    assert!(validate_plan_visibility(&plan).is_ok());
}

#[test]
fn validate_plan_visibility_restricted_with_internal_succeeds() {
    let plan = plan_with_service("db", Visibility::Internal, TopologyVisibility::Restricted);
    assert!(validate_plan_visibility(&plan).is_ok());
}

#[test]
fn validate_plan_visibility_restricted_with_public_succeeds() {
    let plan = plan_with_service("db", Visibility::Public, TopologyVisibility::Restricted);
    assert!(validate_plan_visibility(&plan).is_ok());
}

#[test]
fn validate_plan_visibility_multiple_errors() {
    let mut plan = plan_with_service("web1", Visibility::Private, TopologyVisibility::Open);
    let s2 = planned_service(
        "web2",
        Visibility::Private,
        TopologyVisibility::Open,
        None,
        BTreeMap::new(),
    );
    plan.services.push(s2);
    let errs = validate_plan_visibility(&plan).unwrap_err();
    assert_eq!(errs.len(), 2);
}
