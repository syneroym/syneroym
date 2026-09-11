use std::collections::BTreeMap;

use semver::Version;
use syneroym_app_orchestration::models::{
    AppBlueprintId, AppInstanceId, FdaeManifest, HttpProbe, InterfaceName, LogicalServiceName,
    LogicalServiceRef, PlannedService, RpcProbe, ServiceConfig, ServiceId, ServiceType, TcpProbe,
    TopologyMode, TopologyVisibility,
};

use super::*;

fn base_config() -> ServiceConfig {
    ServiceConfig {
        service_type: ServiceType::Tcp,
        source: "127.0.0.1:9000".to_string(),
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
        assets: None,
        visibility: ModelVisibility::Private,
    }
}

fn plan_with_config(config: ServiceConfig) -> DeploymentPlan {
    DeploymentPlan {
        app_instance_id: AppInstanceId::new("inst-1"),
        blueprint_id: AppBlueprintId::new("syneroym:test-app"),
        version: Version::parse("0.1.0").unwrap(),
        services: vec![PlannedService {
            service_id: ServiceId::new("did:key:h123"),
            logical_ref: LogicalServiceRef {
                app_instance_id: AppInstanceId::new("inst-1"),
                service_name: LogicalServiceName::new("svc"),
            },
            substrate: None,
            config,
            resolved_dependencies: BTreeMap::new(),
            topology_mode: TopologyMode::Singleton,
            member_index: 0,
            schedule: None,
            sharding_strategy: None,
            topology_visibility: TopologyVisibility::Restricted,
        }],
    }
}

/// Whole-plan mapping, which is what every test but the subset one wants.
fn map_all(
    plan: &DeploymentPlan,
    instance_certificates: &BTreeMap<ServiceId, String>,
    registry_certificates: &BTreeMap<ServiceId, String>,
    emit_bindings: bool,
) -> anyhow::Result<WitDeploymentPlan> {
    let all: Vec<&PlannedService> = plan.services.iter().collect();
    // None of these tests exercise the
    // generation gate, which is `map_deployment_plan_to_wit`'s own
    // concern to unit-test. Epoch defaults to empty (every binding maps
    // at 0) for the same reason -- the epoch map is its own test's
    // concern.
    map_deployment_plan_to_wit(
        plan,
        &all,
        instance_certificates,
        registry_certificates,
        emit_bindings,
        0,
        &BTreeMap::new(),
    )
}

/// The point of the whole change: a bare manifest path is resolved here,
/// on the client, so the deploy call carries the document and the
/// substrate needs nothing pre-staged.
#[test]
fn local_document_ref_is_read_and_shipped_inline() {
    let dir = tempfile::tempdir().unwrap();
    let policy = dir.path().join("fdae-policy.json");
    std::fs::write(&policy, r#"{"version":"fdae/v1"}"#).unwrap();

    let mut config = base_config();
    config.fdae =
        Some(FdaeManifest { policy: DocumentRef::Local(policy.to_string_lossy().into_owned()) });

    let wit_plan =
        map_all(&plan_with_config(config), &BTreeMap::new(), &BTreeMap::new(), true).unwrap();
    match &wit_plan.services[0].manifest.config.fdae_policy {
        Some(DocumentSource::Inline(content)) => {
            assert_eq!(content, r#"{"version":"fdae/v1"}"#);
        }
        other => panic!("expected inline content, got {other:?}"),
    }
}

#[test]
fn remote_document_ref_passes_through_for_the_substrate_to_resolve() {
    let mut config = base_config();
    config.fdae = Some(FdaeManifest {
        policy: DocumentRef::Remote { remote_path: "policies/shared.json".to_string() },
    });

    let wit_plan =
        map_all(&plan_with_config(config), &BTreeMap::new(), &BTreeMap::new(), true).unwrap();
    match &wit_plan.services[0].manifest.config.fdae_policy {
        Some(DocumentSource::Path(path)) => assert_eq!(path, "policies/shared.json"),
        other => panic!("expected a host path, got {other:?}"),
    }
}

#[test]
fn local_document_ref_missing_file_fails_the_deploy() {
    let mut config = base_config();
    config.fdae =
        Some(FdaeManifest { policy: DocumentRef::Local("does-not-exist.json".to_string()) });

    assert!(map_all(&plan_with_config(config), &BTreeMap::new(), &BTreeMap::new(), true,).is_err());
}

#[test]
fn map_deployment_plan_to_wit_maps_absent_fdae_to_none() {
    let wit_plan =
        map_all(&plan_with_config(base_config()), &BTreeMap::new(), &BTreeMap::new(), true)
            .unwrap();
    assert!(wit_plan.services[0].manifest.config.fdae_policy.is_none());
}

#[test]
fn no_health_check_maps_to_none() {
    let wit_plan =
        map_all(&plan_with_config(base_config()), &BTreeMap::new(), &BTreeMap::new(), true)
            .unwrap();
    assert!(wit_plan.services[0].manifest.config.health_check.is_none());
}

/// A TCP service with no declared interfaces gets `DEFAULT_INTERFACE_NAME`
/// -- the one name this mapper and `roymctl svc deploy`'s own
/// `--interfaces` fallback now share, rather than each independently
/// picking a different string.
#[test]
fn a_tcp_service_with_no_declared_interfaces_gets_the_shared_default_name() {
    let wit_plan =
        map_all(&plan_with_config(base_config()), &BTreeMap::new(), &BTreeMap::new(), true)
            .unwrap();
    match &wit_plan.services[0].manifest.service_type {
        WitServiceType::Tcp(m) => {
            assert_eq!(m.endpoints.len(), 1);
            assert_eq!(m.endpoints[0].interface_name, DEFAULT_INTERFACE_NAME);
        }
        other => panic!("expected a TCP manifest, got {other:?}"),
    }
}

/// An explicitly declared interface is used verbatim, never overridden
/// by the default.
#[test]
fn a_tcp_services_declared_interface_name_is_used_verbatim() {
    let mut config = base_config();
    config.interfaces = vec![InterfaceName::new("admin")];
    let wit_plan =
        map_all(&plan_with_config(config), &BTreeMap::new(), &BTreeMap::new(), true).unwrap();
    match &wit_plan.services[0].manifest.service_type {
        WitServiceType::Tcp(m) => {
            assert_eq!(m.endpoints.len(), 1);
            assert_eq!(m.endpoints[0].interface_name, "admin");
        }
        other => panic!("expected a TCP manifest, got {other:?}"),
    }
}

#[test]
fn a_health_check_maps_onto_the_wire() {
    let mut tcp_config = base_config();
    tcp_config.health_check = Some(HealthCheck::TcpConnect(TcpProbe {
        interface: InterfaceName::new("main"),
        timeout_ms: 1234,
    }));
    let wit_plan =
        map_all(&plan_with_config(tcp_config), &BTreeMap::new(), &BTreeMap::new(), true).unwrap();
    match wit_plan.services[0].manifest.config.health_check.as_ref().unwrap() {
        WitHealthCheck::TcpConnect(p) => {
            assert_eq!(p.interface_name, "main");
            assert_eq!(p.timeout_ms, 1234);
        }
        other => panic!("expected TcpConnect, got {other:?}"),
    }

    let mut http_config = base_config();
    http_config.health_check = Some(HealthCheck::HttpGet(HttpProbe {
        interface: InterfaceName::new("http"),
        path: "/healthz".to_string(),
        expect_status: 204,
        timeout_ms: 1500,
    }));
    let wit_plan =
        map_all(&plan_with_config(http_config), &BTreeMap::new(), &BTreeMap::new(), true).unwrap();
    match wit_plan.services[0].manifest.config.health_check.as_ref().unwrap() {
        WitHealthCheck::HttpGet(p) => {
            assert_eq!(p.interface_name, "http");
            assert_eq!(p.path, "/healthz");
            assert_eq!(p.expect_status, 204);
            assert_eq!(p.timeout_ms, 1500);
        }
        other => panic!("expected HttpGet, got {other:?}"),
    }

    let mut rpc_config = base_config();
    rpc_config.health_check = Some(HealthCheck::Rpc(RpcProbe {
        interface: InterfaceName::new("rpc"),
        method: "ping".to_string(),
        timeout_ms: 2000,
    }));
    let wit_plan =
        map_all(&plan_with_config(rpc_config), &BTreeMap::new(), &BTreeMap::new(), true).unwrap();
    match wit_plan.services[0].manifest.config.health_check.as_ref().unwrap() {
        WitHealthCheck::Rpc(p) => {
            assert_eq!(p.interface_name, "rpc");
            assert_eq!(p.method, "ping");
            assert_eq!(p.timeout_ms, 2000);
        }
        other => panic!("expected Rpc, got {other:?}"),
    }
}

fn container_config(custom: &str) -> ServiceConfig {
    let mut config = base_config();
    config.service_type = ServiceType::Container;
    config.source = "docker.io/library/nginx:1.27".to_string();
    config.custom_config = Some(custom.to_string());
    config
}

fn container_manifest_of(plan: &WitDeploymentPlan) -> &ContainerManifest {
    match &plan.services[0].manifest.service_type {
        WitServiceType::Container(m) => m,
        other => panic!("expected a container manifest, got {other:?}"),
    }
}

/// Guards the field names the developer guide documents: a mismatch here
/// would only surface at a live deploy.
#[test]
fn container_volume_files_are_parsed_and_inlined() {
    let dir = tempfile::tempdir().unwrap();
    let conf = dir.path().join("nginx.conf");
    std::fs::write(&conf, "server { listen 80; }").unwrap();

    let custom = format!(
        r#"{{"volumes":[{{"host_path":"conf","container_path":"/etc/nginx/conf.d",
             "files":[{{"relative_path":"default.conf","content":{}}}]}}]}}"#,
        serde_json::to_string(&conf.to_string_lossy().into_owned()).unwrap()
    );

    let wit_plan = map_all(
        &plan_with_config(container_config(&custom)),
        &BTreeMap::new(),
        &BTreeMap::new(),
        true,
    )
    .expect("volumes should parse");
    let volumes = &container_manifest_of(&wit_plan).volumes;

    assert_eq!(volumes.len(), 1);
    assert_eq!(volumes[0].host_path, "conf");
    assert_eq!(volumes[0].container_path, "/etc/nginx/conf.d");
    assert_eq!(volumes[0].files.len(), 1);
    assert_eq!(volumes[0].files[0].relative_path, "default.conf");
    match &volumes[0].files[0].content {
        DocumentSource::Inline(c) => assert_eq!(c, "server { listen 80; }"),
        other => panic!("expected inline content, got {other:?}"),
    }
}

/// A volume that only wants an empty directory stays as terse as it was
/// before `files` existed.
#[test]
fn container_volume_without_files_still_parses() {
    let custom = r#"{"volumes":[{"host_path":"data","container_path":"/data"}]}"#;
    let wit_plan = map_all(
        &plan_with_config(container_config(custom)),
        &BTreeMap::new(),
        &BTreeMap::new(),
        true,
    )
    .unwrap();
    let volumes = &container_manifest_of(&wit_plan).volumes;

    assert_eq!(volumes.len(), 1);
    assert!(volumes[0].files.is_empty());
}

/// Both sibling keys fail loudly. Silently dropping either one deploys a
/// container that is broken in a way nothing reports.
#[test]
fn malformed_volumes_and_ports_both_fail_the_deploy() {
    let bad_volumes = r#"{"volumes":[{"host_path":"data"}]}"#;
    let err = map_all(
        &plan_with_config(container_config(bad_volumes)),
        &BTreeMap::new(),
        &BTreeMap::new(),
        true,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("invalid container volumes"), "{err}");

    let bad_ports = r#"{"ports":[{"interface_name":"default","port":80,"protocol":"tcp"}]}"#;
    let err = map_all(
        &plan_with_config(container_config(bad_ports)),
        &BTreeMap::new(),
        &BTreeMap::new(),
        true,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("invalid container ports"), "{err}");
}

#[test]
fn oversize_local_document_fails_before_the_deploy_call() {
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big-policy.json");
    std::fs::write(&big, "x".repeat(deploy_docs::MAX_DEPLOY_DOCUMENT_BYTES as usize + 1)).unwrap();

    let mut config = base_config();
    config.fdae =
        Some(FdaeManifest { policy: DocumentRef::Local(big.to_string_lossy().into_owned()) });

    let err = map_all(&plan_with_config(config), &BTreeMap::new(), &BTreeMap::new(), true)
        .unwrap_err()
        .to_string();
    assert!(err.contains("exceeding the"), "{err}");
}

/// A plan with `frontend` depending on `backend`, `backend` deployed
/// `Redundant` with two members -- so a binding assertion exercises
/// both "one binding per `depends_on` entry" and "the mode is the
/// *target's* own topology mode, not the dependent's". Every service in
/// this fixture is otherwise `Singleton`.
fn plan_with_a_dependency() -> DeploymentPlan {
    let app_instance_id = AppInstanceId::new("inst-1");
    let backend_ref = LogicalServiceRef {
        app_instance_id: app_instance_id.clone(),
        service_name: LogicalServiceName::new("backend"),
    };
    let frontend_ref = LogicalServiceRef {
        app_instance_id: app_instance_id.clone(),
        service_name: LogicalServiceName::new("frontend"),
    };
    DeploymentPlan {
        app_instance_id,
        blueprint_id: AppBlueprintId::new("syneroym:test-app"),
        version: Version::parse("0.1.0").unwrap(),
        services: vec![
            PlannedService {
                service_id: ServiceId::new("did:key:hBackend"),
                logical_ref: backend_ref,
                substrate: None,
                config: base_config(),
                resolved_dependencies: BTreeMap::new(),
                topology_mode: TopologyMode::Redundant,
                member_index: 0,
                schedule: None,
                sharding_strategy: None,
                topology_visibility: TopologyVisibility::Restricted,
            },
            PlannedService {
                service_id: ServiceId::new("did:key:hFrontend"),
                logical_ref: frontend_ref,
                substrate: None,
                config: base_config(),
                resolved_dependencies: BTreeMap::from([(
                    LogicalServiceName::new("backend"),
                    vec![
                        ServiceId::new("did:key:hBackendMember1"),
                        ServiceId::new("did:key:hBackendMember2"),
                    ],
                )]),
                topology_mode: TopologyMode::Singleton,
                member_index: 0,
                schedule: None,
                sharding_strategy: None,
                topology_visibility: TopologyVisibility::Restricted,
            },
        ],
    }
}

#[test]
fn the_app_context_carries_one_binding_per_depends_on_entry_with_the_targets_mode() {
    let wit_plan =
        map_all(&plan_with_a_dependency(), &BTreeMap::new(), &BTreeMap::new(), true).unwrap();

    let frontend = wit_plan.services.iter().find(|s| s.logical_ref.ends_with("frontend")).unwrap();
    let ctx = frontend.app_context.as_ref().expect("frontend has an app context");
    assert_eq!(ctx.app_instance_id, "inst-1");
    assert_eq!(ctx.service_name, "frontend");
    assert_eq!(ctx.bindings.len(), 1);
    let binding = &ctx.bindings[0];
    assert_eq!(binding.dependency_name, "backend");
    assert_eq!(binding.app_instance_id, "inst-1");
    assert!(
        matches!(binding.mode, WitTopologyMode::Redundant),
        "the binding's mode must be the *target's* topology mode, not the dependent's -- backend \
         is Redundant, frontend (the dependent) is Singleton"
    );
    assert_eq!(binding.members, vec!["did:key:hBackendMember1", "did:key:hBackendMember2"]);

    let backend = wit_plan.services.iter().find(|s| s.logical_ref.ends_with("backend")).unwrap();
    assert!(
        backend.app_context.as_ref().unwrap().bindings.is_empty(),
        "a service with no depends_on entry gets an empty binding list, not one for itself"
    );
}

/// The latent bug this guards against: `backend`'s topology mode
/// must come from the *whole* plan, not from the subset being mapped.
/// Mapping only `frontend` (as a per-substrate deploy call does when
/// `backend` is placed elsewhere) must still emit `backend`'s real mode
/// on the binding -- a naive "filter the plan, then map" shape would
/// silently default it to `Singleton` since `backend` itself is absent
/// from the subset.
#[test]
fn mapping_one_service_resolves_a_dependencys_mode_from_the_whole_plan() {
    let plan = plan_with_a_dependency();
    let frontend_only: Vec<&PlannedService> = plan
        .services
        .iter()
        .filter(|s| s.logical_ref.service_name.as_str() == "frontend")
        .collect();

    let wit_plan = map_deployment_plan_to_wit(
        &plan,
        &frontend_only,
        &BTreeMap::new(),
        &BTreeMap::new(),
        true,
        0,
        &BTreeMap::new(),
    )
    .unwrap();

    assert_eq!(wit_plan.services.len(), 1);
    let ctx = wit_plan.services[0].app_context.as_ref().unwrap();
    let binding = &ctx.bindings[0];
    assert!(
        matches!(binding.mode, WitTopologyMode::Redundant),
        "backend's mode must be resolved from the whole plan even though only frontend was mapped"
    );
}

/// The epoch is keyed by the *dependent*
/// member's own ref, not by the dependency name -- frontend's one entry
/// in the map must land on every one of frontend's bindings.
#[test]
fn a_plan_mapped_at_a_nonzero_epoch_emits_that_epoch_on_every_binding() {
    let plan = plan_with_a_dependency();
    let frontend_member_ref = plan.services[1].member_ref();
    let epochs = BTreeMap::from([(frontend_member_ref, 7u64)]);

    let all: Vec<&PlannedService> = plan.services.iter().collect();
    let wit_plan = map_deployment_plan_to_wit(
        &plan,
        &all,
        &BTreeMap::new(),
        &BTreeMap::new(),
        true,
        0,
        &epochs,
    )
    .unwrap();

    let frontend = wit_plan.services.iter().find(|s| s.logical_ref.ends_with("frontend")).unwrap();
    let ctx = frontend.app_context.as_ref().unwrap();
    assert_eq!(ctx.bindings.len(), 1);
    assert_eq!(ctx.bindings[0].epoch, 7);

    // backend has no entry in the map, so it must fall back to 0 --
    // meaning "no supervisor has written here" -- rather than
    // inheriting frontend's value or panicking on a missing key.
    let backend = wit_plan.services.iter().find(|s| s.logical_ref.ends_with("backend")).unwrap();
    assert!(backend.app_context.as_ref().unwrap().bindings.is_empty());
}

#[test]
fn a_plan_with_no_dependencies_emits_an_empty_binding_list() {
    let wit_plan =
        map_all(&plan_with_config(base_config()), &BTreeMap::new(), &BTreeMap::new(), true)
            .unwrap();
    assert!(wit_plan.services[0].app_context.as_ref().unwrap().bindings.is_empty());
}

/// Without `--mint-masters`, `resolved_dependencies` still
/// holds the compiler's fabricated `did:key:h...` ids, which are not
/// real keys. Publishing them would let `dependency(...)` resolve and
/// then fail one layer down as `service-not-found`, destroying the
/// distinction `dependency-not-bound` exists to draw -- so
/// `emit_bindings: false` must publish no bindings at all, not the
/// fabricated ones.
#[test]
fn emit_bindings_false_publishes_no_fabricated_member_dids() {
    let wit_plan =
        map_all(&plan_with_a_dependency(), &BTreeMap::new(), &BTreeMap::new(), false).unwrap();

    for svc in &wit_plan.services {
        assert!(
            svc.app_context.as_ref().unwrap().bindings.is_empty(),
            "emit_bindings: false must publish no bindings for '{}', fabricated or otherwise",
            svc.logical_ref
        );
    }
}
