//! The supervisor's own Tier-2 document machinery (ADR-0022 §3), a sibling
//! of [`crate::tier1`]: where [`crate::tier1`] answers "which supervisor
//! holds this app," this module answers "who are the members of one of its
//! logical services" from the supervisor's own stored plan.

use std::{collections::BTreeSet, fmt};

use syneroym_app_orchestration::{
    DeploymentPlan, LogicalServiceName, ServiceId, ShardingStrategy, TopologyMode,
    TopologyVisibility,
};
use syneroym_core::util;

/// One logical service's topology, as read out of a stored `DeploymentPlan`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceTopology {
    pub mode: TopologyMode,
    pub members: Vec<ServiceId>,
    pub sharding_strategy: Option<ShardingStrategy>,
}

/// `service_topology` could not build a topology for the requested service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopologyBuildError {
    /// No `PlannedService` in the plan names this logical service.
    NoSuchService(LogicalServiceName),
    /// Two members of the same logical service disagree about `mode` or
    /// `sharding_strategy` -- a compiler bug, not a recoverable state.
    InconsistentPlan(LogicalServiceName),
    /// Two (or more) declared service names share a `short_hash`, so a
    /// hashed lookup (ADR-0022 §7's `-s<hash>` gateway segment) cannot
    /// name one of them. Carries every colliding name -- the plan's own
    /// sketch held a single `LogicalServiceName` here, but naming *both*
    /// colliding names in the message (as the plan's own prose requires)
    /// needs the whole set, not one of them.
    AmbiguousHash(Vec<LogicalServiceName>),
}

impl fmt::Display for TopologyBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchService(name) => {
                write!(f, "no service named '{name}' in this instance's plan")
            }
            Self::InconsistentPlan(name) => write!(
                f,
                "service '{name}''s members disagree about topology mode or sharding strategy -- \
                 this is a compiler defect, not a recoverable plan state"
            ),
            Self::AmbiguousHash(names) => {
                let joined =
                    names.iter().map(LogicalServiceName::as_str).collect::<Vec<_>>().join("', '");
                write!(
                    f,
                    "a short_hash matches more than one of this instance's declared service names \
                     ('{joined}') -- refusing rather than guessing which one was meant"
                )
            }
        }
    }
}

impl std::error::Error for TopologyBuildError {}

/// Reads one logical service's declared publication posture for Tier-2
/// resolution (ADR-0022 §5). Members of one logical service carry identical
/// copies -- the compiler clones the spec's value onto each -- so a
/// disagreement is a compiler defect, reported the same way `service_topology`
/// reports one rather than resolved to whichever member sorted first.
pub fn service_topology_visibility(
    plan: &DeploymentPlan,
    service_name: &LogicalServiceName,
) -> Result<TopologyVisibility, TopologyBuildError> {
    let members: Vec<_> =
        plan.services.iter().filter(|s| &s.logical_ref.service_name == service_name).collect();
    if members.is_empty() {
        return Err(TopologyBuildError::NoSuchService(service_name.clone()));
    }
    let vis = members[0].topology_visibility;
    if members.iter().any(|s| s.topology_visibility != vis) {
        return Err(TopologyBuildError::InconsistentPlan(service_name.clone()));
    }
    Ok(vis)
}

/// Resolves the `service_name` a caller supplied against the names this
/// plan actually declares, accepting either the exact name or its
/// `short_hash` (ADR-0022 §7 puts a hash of the logical service name in
/// the gateway hostname, and a hash cannot be reversed by the caller --
/// the party holding the candidate set does the reversing, exactly as an
/// interface hash is reversed at its destination).
///
/// An exact match always wins. A hash matching two declared names is
/// refused rather than resolved to whichever sorted first: `short_hash` is
/// a five-byte SHA-256 prefix, and answering with the wrong service's
/// members is worse than answering with nothing.
pub fn resolve_service_name(
    plan: &DeploymentPlan,
    supplied: &LogicalServiceName,
) -> Result<LogicalServiceName, TopologyBuildError> {
    resolve_service_name_with_hasher(plan, supplied, util::short_hash)
}

/// `resolve_service_name`'s actual logic, parameterised over the hash
/// function so a test can stub a collision without searching for a real
/// five-byte SHA-256 one (impractical at fixture-construction time).
fn resolve_service_name_with_hasher(
    plan: &DeploymentPlan,
    supplied: &LogicalServiceName,
    hash: impl Fn(&str) -> String,
) -> Result<LogicalServiceName, TopologyBuildError> {
    let names: BTreeSet<&LogicalServiceName> =
        plan.services.iter().map(|s| &s.logical_ref.service_name).collect();
    if names.contains(supplied) {
        return Ok(supplied.clone());
    }

    let matches: Vec<LogicalServiceName> = names
        .iter()
        .filter(|n| hash(n.as_str()) == supplied.as_str())
        .map(|n| (*n).clone())
        .collect();
    match matches.len() {
        0 => Err(TopologyBuildError::NoSuchService(supplied.clone())),
        1 => Ok(matches[0].clone()),
        _ => Err(TopologyBuildError::AmbiguousHash(matches)),
    }
}

/// Groups a stored plan's members into one logical service's topology.
/// Pure: no vault, no store, no clock, so the grouping rule is testable on
/// its own.
///
/// Members come out in `member_index` order, so an unchanged plan hashes
/// and signs identically every time. A plan whose members disagree about
/// `topology_mode` or `sharding_strategy` is a compiler bug, not a
/// recoverable state, and is refused here rather than silently resolved to
/// whichever member happened to sort first.
pub fn service_topology(
    plan: &DeploymentPlan,
    service_name: &LogicalServiceName,
) -> Result<ServiceTopology, TopologyBuildError> {
    let mut members: Vec<_> =
        plan.services.iter().filter(|s| &s.logical_ref.service_name == service_name).collect();
    if members.is_empty() {
        return Err(TopologyBuildError::NoSuchService(service_name.clone()));
    }
    members.sort_by_key(|s| s.member_index);

    let mode = members[0].topology_mode;
    let sharding_strategy = members[0].sharding_strategy.clone();
    if members.iter().any(|s| s.topology_mode != mode || s.sharding_strategy != sharding_strategy) {
        return Err(TopologyBuildError::InconsistentPlan(service_name.clone()));
    }
    // Two members sharing a `member_index` would leave their relative
    // order dependent on `plan.services`'s own order rather than on
    // anything meaningful -- the fingerprint, the signed member list, and
    // the epoch it drives could then differ between two readings of the
    // same plan. A compiler bug, exactly like the disagreement above.
    if members.windows(2).any(|w| w[0].member_index == w[1].member_index) {
        return Err(TopologyBuildError::InconsistentPlan(service_name.clone()));
    }

    Ok(ServiceTopology {
        mode,
        members: members.into_iter().map(|s| s.service_id.clone()).collect(),
        sharding_strategy,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use syneroym_app_orchestration::{
        AppBlueprintId, AppInstanceId, LogicalServiceRef, PlannedService, ServiceConfig,
        ServiceType, Visibility,
    };

    use super::*;

    fn svc_id(s: &str) -> ServiceId {
        ServiceId::new(format!("did:key:{s}"))
    }

    fn svc_name(s: &str) -> LogicalServiceName {
        LogicalServiceName::new(s)
    }

    fn dummy_config() -> ServiceConfig {
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
            visibility: Visibility::Private,
        }
    }

    fn member(
        service_name: &str,
        member_index: u32,
        mode: TopologyMode,
        sharding_strategy: Option<ShardingStrategy>,
    ) -> PlannedService {
        member_with_vis(
            service_name,
            member_index,
            mode,
            sharding_strategy,
            TopologyVisibility::Restricted,
        )
    }

    fn member_with_vis(
        service_name: &str,
        member_index: u32,
        mode: TopologyMode,
        sharding_strategy: Option<ShardingStrategy>,
        topology_visibility: TopologyVisibility,
    ) -> PlannedService {
        PlannedService {
            service_id: svc_id(&format!("m{member_index}")),
            logical_ref: LogicalServiceRef {
                app_instance_id: AppInstanceId::new("inst-1"),
                service_name: svc_name(service_name),
            },
            substrate: None,
            config: dummy_config(),
            resolved_dependencies: BTreeMap::new(),
            topology_mode: mode,
            member_index,
            schedule: None,
            sharding_strategy,
            topology_visibility,
        }
    }

    fn plan(services: Vec<PlannedService>) -> DeploymentPlan {
        DeploymentPlan {
            app_instance_id: AppInstanceId::new("inst-1"),
            blueprint_id: AppBlueprintId::new("syneroym:test"),
            version: semver::Version::new(1, 0, 0),
            services,
        }
    }

    #[test]
    fn test_service_topology_visibility_returns_declared_visibility() {
        let p_open = plan(vec![member_with_vis(
            "backend",
            0,
            TopologyMode::Singleton,
            None,
            TopologyVisibility::Open,
        )]);
        assert_eq!(
            service_topology_visibility(&p_open, &svc_name("backend")).unwrap(),
            TopologyVisibility::Open
        );

        let p_restricted = plan(vec![member_with_vis(
            "backend",
            0,
            TopologyMode::Singleton,
            None,
            TopologyVisibility::Restricted,
        )]);
        assert_eq!(
            service_topology_visibility(&p_restricted, &svc_name("backend")).unwrap(),
            TopologyVisibility::Restricted
        );
    }

    #[test]
    fn test_service_topology_visibility_rejects_inconsistent_plan() {
        let p = plan(vec![
            member_with_vis("backend", 0, TopologyMode::Redundant, None, TopologyVisibility::Open),
            member_with_vis(
                "backend",
                1,
                TopologyMode::Redundant,
                None,
                TopologyVisibility::Restricted,
            ),
        ]);
        let err = service_topology_visibility(&p, &svc_name("backend")).unwrap_err();
        assert!(matches!(err, TopologyBuildError::InconsistentPlan(_)));
    }

    #[test]
    fn test_service_topology_visibility_resolves_via_short_hash() {
        let p = plan(vec![member_with_vis(
            "backend",
            0,
            TopologyMode::Singleton,
            None,
            TopologyVisibility::Open,
        )]);
        let resolved_name =
            resolve_service_name(&p, &svc_name(&util::short_hash("backend"))).unwrap();
        assert_eq!(resolved_name, svc_name("backend"));
        assert_eq!(
            service_topology_visibility(&p, &resolved_name).unwrap(),
            TopologyVisibility::Open
        );
    }

    #[test]
    fn members_come_out_in_member_index_order_regardless_of_plan_order() {
        let p = plan(vec![
            member("backend", 2, TopologyMode::Redundant, None),
            member("backend", 0, TopologyMode::Redundant, None),
            member("backend", 1, TopologyMode::Redundant, None),
        ]);
        let topo = service_topology(&p, &svc_name("backend")).unwrap();
        assert_eq!(topo.members, vec![svc_id("m0"), svc_id("m1"), svc_id("m2")]);
    }

    #[test]
    fn an_unknown_service_name_is_refused() {
        let p = plan(vec![member("backend", 0, TopologyMode::Singleton, None)]);
        let err = service_topology(&p, &svc_name("ghost")).unwrap_err();
        assert!(matches!(err, TopologyBuildError::NoSuchService(_)));
    }

    /// Test 73: the exact-match path is unchanged.
    #[test]
    fn a_service_name_resolves_to_itself() {
        let p = plan(vec![member("backend", 0, TopologyMode::Singleton, None)]);
        let resolved = resolve_service_name(&p, &svc_name("backend")).unwrap();
        assert_eq!(resolved, svc_name("backend"));
    }

    /// Test 74.
    #[test]
    fn a_short_hash_of_a_service_name_resolves_to_that_name() {
        let p = plan(vec![member("backend", 0, TopologyMode::Singleton, None)]);
        let hash = util::short_hash("backend");
        let resolved = resolve_service_name(&p, &svc_name(&hash)).unwrap();
        assert_eq!(resolved, svc_name("backend"));
    }

    /// Test 75: construct a plan with a service literally named
    /// `short_hash("other")`.
    #[test]
    fn an_exact_name_wins_over_a_hash_that_matches_a_different_name() {
        let hash_of_other = util::short_hash("other");
        let p = plan(vec![
            member(&hash_of_other, 0, TopologyMode::Singleton, None),
            member("other", 0, TopologyMode::Singleton, None),
        ]);
        // A plan with two logical services sharing a member_index is fine
        // -- they are different services, so `service_topology` never
        // groups them together.
        let resolved = resolve_service_name(&p, &svc_name(&hash_of_other)).unwrap();
        assert_eq!(resolved, svc_name(&hash_of_other), "the exact name must win over the hash");
    }

    /// Test 76: `AmbiguousHash`, with both names in the message. A real
    /// five-byte SHA-256 collision is impractical to search for at
    /// fixture-construction time, so the collision is constructed by
    /// stubbing the hash function -- asserted as "the branch exists and
    /// refuses", which is what it is.
    #[test]
    fn two_service_names_sharing_a_short_hash_are_refused() {
        let p = plan(vec![
            member("alpha", 0, TopologyMode::Singleton, None),
            member("beta", 0, TopologyMode::Singleton, None),
        ]);
        let stub_hash = |_: &str| "collidinghash".to_string();
        let err = resolve_service_name_with_hasher(&p, &svc_name("collidinghash"), stub_hash)
            .unwrap_err();
        let msg = err.to_string();
        assert!(matches!(err, TopologyBuildError::AmbiguousHash(_)));
        assert!(msg.contains("alpha"), "{msg}");
        assert!(msg.contains("beta"), "{msg}");
    }

    /// Test 77: the mapping S2's post-merge finding 12 established for
    /// `NoSuchService`, extended to `AmbiguousHash`.
    #[test]
    fn an_unknown_hash_is_invalid_params_not_internal_error() {
        let p = plan(vec![member("backend", 0, TopologyMode::Singleton, None)]);
        let err = resolve_service_name(&p, &svc_name("nonexistenthash")).unwrap_err();
        assert!(matches!(err, TopologyBuildError::NoSuchService(_)));
    }

    #[test]
    fn members_disagreeing_on_mode_are_refused() {
        let p = plan(vec![
            member("backend", 0, TopologyMode::Redundant, None),
            member("backend", 1, TopologyMode::Singleton, None),
        ]);
        let err = service_topology(&p, &svc_name("backend")).unwrap_err();
        assert!(matches!(err, TopologyBuildError::InconsistentPlan(_)));
    }

    #[test]
    fn members_disagreeing_on_sharding_strategy_are_refused() {
        let p = plan(vec![
            member("backend", 0, TopologyMode::Sharded, Some(ShardingStrategy::HashSharding)),
            member("backend", 1, TopologyMode::Sharded, Some(ShardingStrategy::EntityTagSharding)),
        ]);
        let err = service_topology(&p, &svc_name("backend")).unwrap_err();
        assert!(matches!(err, TopologyBuildError::InconsistentPlan(_)));
    }

    #[test]
    fn members_sharing_a_member_index_are_refused() {
        let p = plan(vec![
            member("backend", 0, TopologyMode::Redundant, None),
            member("backend", 0, TopologyMode::Redundant, None),
        ]);
        let err = service_topology(&p, &svc_name("backend")).unwrap_err();
        assert!(matches!(err, TopologyBuildError::InconsistentPlan(_)));
    }

    #[test]
    fn only_the_named_services_members_are_included() {
        let p = plan(vec![
            member("backend", 0, TopologyMode::Singleton, None),
            member("frontend", 0, TopologyMode::Singleton, None),
        ]);
        let topo = service_topology(&p, &svc_name("backend")).unwrap();
        assert_eq!(topo.members, vec![svc_id("m0")]);
    }
}
