//! The supervisor's own Tier-2 document machinery (ADR-0022 §3), a sibling
//! of [`crate::tier1`]: where [`crate::tier1`] answers "which supervisor
//! holds this app," this module answers "who are the members of one of its
//! logical services" from the supervisor's own stored plan.

use std::fmt;

use syneroym_app_orchestration::{
    DeploymentPlan, LogicalServiceName, ServiceId, ShardingStrategy, TopologyMode,
};

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
        }
    }
}

impl std::error::Error for TopologyBuildError {}

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
        ServiceType,
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
        }
    }

    fn member(
        service_name: &str,
        member_index: u32,
        mode: TopologyMode,
        sharding_strategy: Option<ShardingStrategy>,
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
