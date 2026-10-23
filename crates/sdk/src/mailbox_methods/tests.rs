use std::collections::BTreeMap;

use semver::Version;
use syneroym_app_orchestration::{
    AppBlueprintId, AppInstanceId, DeploymentPlan, LogicalServiceName, MailboxMethod,
    PlannedService, ServiceConfig, ServiceId, ServiceType, TopologyMode, TopologyVisibility,
    Visibility, models::LogicalServiceRef,
};
use syneroym_core::util;

use super::refuse_reserved_mailbox_methods;

fn plan_with(methods: &[&str]) -> DeploymentPlan {
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
            config: ServiceConfig {
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
                visibility: Visibility::Public,
            },
            resolved_dependencies: BTreeMap::new(),
            topology_mode: TopologyMode::Singleton,
            member_index: 0,
            schedule: None,
            sharding_strategy: None,
            topology_visibility: TopologyVisibility::Restricted,
            mailbox: None,
            instance_cert_expires_hours: None,
            mailbox_methods: methods.iter().map(|m| m.parse::<MailboxMethod>().unwrap()).collect(),
        }],
    }
}

#[test]
fn a_guests_own_methods_pass() {
    assert!(refuse_reserved_mailbox_methods(&plan_with(&["greeter#greet", "api#invoke"])).is_ok());
    assert!(refuse_reserved_mailbox_methods(&plan_with(&[])).is_ok());
}

#[test]
fn a_node_or_platform_interface_is_refused_by_name_and_by_short_hash() {
    let hashed = format!("{}#x", util::short_hash("security"));
    for entry in
        ["orchestrator#x", "mailbox#x", "data-layer#x", "conversation#send", hashed.as_str()]
    {
        let errors = refuse_reserved_mailbox_methods(&plan_with(&[entry])).unwrap_err();
        assert_eq!(errors.len(), 1, "{entry}");
        assert!(errors[0].contains("node or platform interface"), "{}", errors[0]);
    }
}
