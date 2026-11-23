use std::collections::BTreeMap;

use semver::Version;

use super::*;
use crate::schedule::{
    DEFAULT_SCHEDULE_TIMEOUT_MS, MAX_SCHEDULE_TIMEOUT_MS, MAX_SCHEDULED_SERVICES, ScheduleSpec,
};

fn scheduled_manifest_toml(schedule_block: &str) -> String {
    format!(
        r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.worker]
            service_type = "wasm"
            source = "unused"
            interfaces = ["scheduled-driver"]

            {schedule_block}
        "#
    )
}

#[test]
fn a_manifest_with_no_schedule_serializes_byte_for_byte_as_before() {
    let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.worker]
            service_type = "wasm"
            source = "unused"
        "#;
    let manifest = SynAppManifest::from_toml(toml_str).unwrap();
    let worker = manifest.services.get(&LogicalServiceName::new("worker")).unwrap();
    assert_eq!(worker.schedule, None);
    let serialized = manifest.to_toml().unwrap();
    assert!(!serialized.contains("schedule"));
}

#[test]
fn a_scheduled_service_round_trips_through_toml_and_json() {
    let toml_str = scheduled_manifest_toml(
        r#"
            [services.worker.schedule]
            cron = "* * * * *"
            interface = "scheduled-driver"
            method = "tick"
        "#,
    );
    let manifest = SynAppManifest::from_toml(&toml_str).unwrap();
    let worker = manifest.services.get(&LogicalServiceName::new("worker")).unwrap();
    let sched = worker.schedule.as_ref().unwrap();
    assert_eq!(sched.cron, "* * * * *");
    assert_eq!(sched.interface, InterfaceName::new("scheduled-driver"));
    assert_eq!(sched.method, "tick");
    assert_eq!(sched.params, None);
    assert_eq!(sched.timeout_ms, DEFAULT_SCHEDULE_TIMEOUT_MS);

    let toml_round = manifest.to_toml().unwrap();
    assert_eq!(SynAppManifest::from_toml(&toml_round).unwrap(), manifest);
    let json_round = manifest.to_json().unwrap();
    assert_eq!(SynAppManifest::from_json(&json_round).unwrap(), manifest);
}

#[test]
fn a_schedule_naming_an_undeclared_interface_is_refused_at_validation() {
    let toml_str = scheduled_manifest_toml(
        r#"
            [services.worker.schedule]
            cron = "* * * * *"
            interface = "not-declared"
            method = "tick"
        "#,
    );
    let err = SynAppManifest::from_toml(&toml_str).unwrap_err();
    assert!(err.to_string().contains("does not declare interface"), "{err}");
}

#[test]
fn a_schedule_with_an_unparseable_cron_is_refused_at_validation() {
    let toml_str = scheduled_manifest_toml(
        r#"
            [services.worker.schedule]
            cron = "not a cron"
            interface = "scheduled-driver"
            method = "tick"
        "#,
    );
    let err = SynAppManifest::from_toml(&toml_str).unwrap_err();
    assert!(err.to_string().contains("does not parse"), "{err}");
}

#[test]
fn a_schedule_whose_params_are_not_json_is_refused_at_validation() {
    let toml_str = scheduled_manifest_toml(
        r#"
            [services.worker.schedule]
            cron = "* * * * *"
            interface = "scheduled-driver"
            method = "tick"
            params = "not json"
        "#,
    );
    let err = SynAppManifest::from_toml(&toml_str).unwrap_err();
    assert!(err.to_string().contains("not JSON"), "{err}");
}

#[test]
fn a_schedule_with_a_zero_timeout_is_refused_at_validation() {
    let toml_str = scheduled_manifest_toml(
        r#"
            [services.worker.schedule]
            cron = "* * * * *"
            interface = "scheduled-driver"
            method = "tick"
            timeout_ms = 0
        "#,
    );
    let err = SynAppManifest::from_toml(&toml_str).unwrap_err();
    assert!(err.to_string().contains("must be between 1"), "{err}");
}

#[test]
fn a_schedule_timeout_above_the_ceiling_is_refused_rather_than_clamped() {
    let toml_str = scheduled_manifest_toml(&format!(
        r#"
            [services.worker.schedule]
            cron = "* * * * *"
            interface = "scheduled-driver"
            method = "tick"
            timeout_ms = {}
        "#,
        MAX_SCHEDULE_TIMEOUT_MS + 1
    ));
    let err = SynAppManifest::from_toml(&toml_str).unwrap_err();
    assert!(err.to_string().contains(&format!("{MAX_SCHEDULE_TIMEOUT_MS}ms")), "{err}");
}

#[test]
fn a_schedule_exactly_at_the_timeout_ceiling_is_accepted() {
    let toml_str = scheduled_manifest_toml(&format!(
        r#"
            [services.worker.schedule]
            cron = "* * * * *"
            interface = "scheduled-driver"
            method = "tick"
            timeout_ms = {MAX_SCHEDULE_TIMEOUT_MS}
        "#
    ));
    assert!(SynAppManifest::from_toml(&toml_str).is_ok());
}

#[test]
fn more_than_the_cap_of_scheduled_services_is_refused_at_validation() {
    let mut manifest = SynAppManifest {
        id: AppBlueprintId::new("syneroym:guild-app"),
        version: Version::parse("0.1.0").unwrap(),
        description: None,
        placement: None,
        services: BTreeMap::new(),
        dependencies: BTreeMap::new(),
    };
    for i in 0..=MAX_SCHEDULED_SERVICES {
        let name = LogicalServiceName::new(format!("worker-{i}"));
        manifest.services.insert(
            name,
            ServiceSpec {
                config: ServiceConfig {
                    service_type: ServiceType::Wasm,
                    source: "unused".to_string(),
                    hash: None,
                    interfaces: vec![InterfaceName::new("scheduled-driver")],
                    env: BTreeMap::new(),
                    args: vec![],
                    custom_config: None,
                    quota: None,
                    schema: None,
                    rotation_policy: RotationPolicy::RestartOnRotation,
                    fdae: None,
                    health_check: None,
                    wake: None,
                    assets: None,
                    visibility: Visibility::Private,
                },
                depends_on: vec![],
                placement: None,
                replicas: 1,
                sharding_strategy: None,
                schedule: Some(ScheduleSpec {
                    cron: "* * * * *".to_string(),
                    interface: InterfaceName::new("scheduled-driver"),
                    method: "tick".to_string(),
                    params: None,
                    timeout_ms: DEFAULT_SCHEDULE_TIMEOUT_MS,
                }),
                topology_visibility: TopologyVisibility::Restricted,
                mailbox: None,
                instance_cert_expires_hours: None,
                mailbox_methods: Vec::new(),
            },
        );
    }
    let err = manifest.validate().unwrap_err();
    assert!(err.to_string().contains("above the cap"), "{err}");
}
