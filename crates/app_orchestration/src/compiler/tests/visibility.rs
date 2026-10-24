//! `validate_plan_visibility` and the plan builders its tests share.

use super::*;

pub(super) fn planned_service(
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
        mailbox: None,
        instance_cert_expires_hours: None,
        mailbox_methods: Vec::new(),
    }
}

pub(super) fn plan_with_service(
    name: &str,
    vis: Visibility,
    topo_vis: TopologyVisibility,
) -> DeploymentPlan {
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
