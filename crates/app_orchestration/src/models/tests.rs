use std::{collections::BTreeMap, str::FromStr};

use semver::Version;

use super::*;
use crate::schedule::{
    DEFAULT_SCHEDULE_TIMEOUT_MS, MAX_SCHEDULE_TIMEOUT_MS, MAX_SCHEDULED_SERVICES,
};

#[test]
fn test_manifest_parsing_toml() {
    let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"
            description = "Professional Services Guild App"

            [services.identity]
            service_type = "wasm"
            source = "crates/sandbox_wasm/benches/identity.wasm"
            interfaces = ["syneroym:identity/identity"]
            depends_on = []

            [services.echo]
            service_type = "wasm"
            source = "crates/sandbox_wasm/benches/echo.wasm"
            interfaces = ["syneroym:echo/echo"]
            depends_on = ["identity"]

            [dependencies.db]
            mode = "spawn"
            blueprint = "syneroym:db-app"
            manifest_path = "path/to/db.toml"
        "#;

    let manifest = SynAppManifest::from_toml(toml_str).unwrap();
    assert_eq!(manifest.id.as_str(), "syneroym:guild-app");
    assert_eq!(manifest.services.len(), 2);
    assert_eq!(manifest.dependencies.len(), 1);

    let identity = manifest.services.get(&LogicalServiceName::new("identity")).unwrap();
    assert_eq!(identity.config.service_type, ServiceType::Wasm);
    assert_eq!(identity.config.source, "crates/sandbox_wasm/benches/identity.wasm");

    let db_dep = manifest.dependencies.get(&DependencyName::new("db")).unwrap();
    match db_dep {
        AppDependencySpec::Spawn { blueprint, manifest_path } => {
            assert_eq!(blueprint.as_str(), "syneroym:db-app");
            assert_eq!(manifest_path.as_deref(), Some("path/to/db.toml"));
        }
        _ => panic!("Expected Spawn dependency"),
    }

    // Test serialization roundtrip
    let serialized = manifest.to_toml().unwrap();
    let deserialized = SynAppManifest::from_toml(&serialized).unwrap();
    assert_eq!(manifest, deserialized);

    // A manifest with no [services.x.fdae] block parses with fdae: None.
    assert_eq!(identity.config.fdae, None);
}

#[test]
fn test_manifest_parsing_toml_with_fdae_policy() {
    let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.identity]
            service_type = "wasm"
            source = "crates/sandbox_wasm/benches/identity.wasm"
            interfaces = ["syneroym:identity/identity"]
            depends_on = []

            [services.identity.fdae]
            policy = "fdae-policy.json"
        "#;

    let manifest = SynAppManifest::from_toml(toml_str).unwrap();
    let identity = manifest.services.get(&LogicalServiceName::new("identity")).unwrap();
    assert_eq!(
        identity.config.fdae,
        Some(FdaeManifest { policy: DocumentRef::Local("fdae-policy.json".to_string()) })
    );

    let serialized = manifest.to_toml().unwrap();
    let deserialized = SynAppManifest::from_toml(&serialized).unwrap();
    assert_eq!(manifest, deserialized);
}

/// The bare-string and explicit-`remote_path` forms are one field, so a
/// manifest can say "ship this with the deploy" or "the substrate already
/// has it" without a second key that has to be kept mutually exclusive.
#[test]
fn test_manifest_parsing_toml_with_remote_document_refs() {
    let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.identity]
            service_type = "wasm"
            source = "crates/sandbox_wasm/benches/identity.wasm"
            schema = { remote_path = "/etc/syneroym/schemas/shared.json" }

            [services.identity.fdae]
            policy = { remote_path = "/etc/syneroym/policies/guild.json" }
        "#;

    let manifest = SynAppManifest::from_toml(toml_str).unwrap();
    let identity = manifest.services.get(&LogicalServiceName::new("identity")).unwrap();
    assert_eq!(
        identity.config.schema,
        Some(DocumentRef::Remote { remote_path: "/etc/syneroym/schemas/shared.json".to_string() })
    );
    assert_eq!(
        identity.config.fdae,
        Some(FdaeManifest {
            policy: DocumentRef::Remote {
                remote_path: "/etc/syneroym/policies/guild.json".to_string()
            }
        })
    );

    let serialized = manifest.to_toml().unwrap();
    let deserialized = SynAppManifest::from_toml(&serialized).unwrap();
    assert_eq!(manifest, deserialized);
}

#[test]
fn test_manifest_parsing_json() {
    let json_str = r#"{
            "id": "syneroym:guild-app",
            "version": "0.1.0",
            "description": "Professional Services Guild App",
            "services": {
                "identity": {
                    "service_type": "wasm",
                    "source": "crates/sandbox_wasm/benches/identity.wasm",
                    "interfaces": ["syneroym:identity/identity"],
                    "depends_on": []
                }
            },
            "dependencies": {
                "db": {
                    "mode": "bind",
                    "instance": "inst-1234"
                }
            }
        }"#;

    let manifest = SynAppManifest::from_json(json_str).unwrap();
    assert_eq!(manifest.id.as_str(), "syneroym:guild-app");
    assert_eq!(manifest.services.len(), 1);
    assert_eq!(manifest.dependencies.len(), 1);

    let db_dep = manifest.dependencies.get(&DependencyName::new("db")).unwrap();
    match db_dep {
        AppDependencySpec::Bind { instance } => {
            assert_eq!(instance.as_str(), "inst-1234");
        }
        _ => panic!("Expected Bind dependency"),
    }

    // Test serialization roundtrip
    let serialized = manifest.to_json().unwrap();
    let deserialized = SynAppManifest::from_json(&serialized).unwrap();
    assert_eq!(manifest, deserialized);
}

#[test]
fn test_deployment_plan_serialization() {
    let mut env_map = BTreeMap::new();
    env_map.insert("KEY".to_string(), "VAL".to_string());

    let plan = DeploymentPlan {
        app_instance_id: AppInstanceId::new("guild-instance-1"),
        blueprint_id: AppBlueprintId::new("syneroym:guild-app"),
        version: Version::parse("0.1.0").unwrap(),
        services: vec![PlannedService {
            service_id: ServiceId::new("did:key:h123"),
            logical_ref: LogicalServiceRef {
                app_instance_id: AppInstanceId::new("guild-instance-1"),
                service_name: LogicalServiceName::new("identity"),
            },
            substrate: None,
            config: ServiceConfig {
                service_type: ServiceType::Wasm,
                source: "crates/sandbox_wasm/benches/identity.wasm".to_string(),
                hash: None,
                interfaces: vec![InterfaceName::new("syneroym:identity/identity")],
                env: env_map,
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: RotationPolicy::RestartOnRotation,
                fdae: None,
                health_check: None,
                assets: None,
                visibility: Visibility::Private,
            },
            resolved_dependencies: BTreeMap::new(),
            topology_mode: TopologyMode::Singleton,
            member_index: 0,
            schedule: None,
            sharding_strategy: None,
            topology_visibility: TopologyVisibility::Restricted,
        }],
    };

    let toml_str = plan.to_toml().unwrap();
    let plan_toml = DeploymentPlan::from_toml(&toml_str).unwrap();
    assert_eq!(plan, plan_toml);

    let json_str = plan.to_json().unwrap();
    let plan_json = DeploymentPlan::from_json(&json_str).unwrap();
    assert_eq!(plan, plan_json);

    // Detailed field assertion test
    assert_eq!(plan_toml.app_instance_id, AppInstanceId::new("guild-instance-1"));
    assert_eq!(plan_toml.version.to_string(), "0.1.0");
    assert_eq!(plan_toml.services.len(), 1);
    let service = &plan_toml.services[0];
    assert_eq!(service.service_id, ServiceId::new("did:key:h123"));
    assert_eq!(service.topology_mode, TopologyMode::Singleton);
    assert_eq!(service.config.service_type, ServiceType::Wasm);
}

#[test]
fn test_logical_service_ref_from_str() {
    let s = "guild-instance-1/identity";
    let r = LogicalServiceRef::from_str(s).unwrap();
    assert_eq!(r.app_instance_id, AppInstanceId::new("guild-instance-1"));
    assert_eq!(r.service_name, LogicalServiceName::new("identity"));
    assert_eq!(r.to_string(), s);

    assert!(LogicalServiceRef::from_str("invalid").is_err());
    assert!(LogicalServiceRef::from_str("too/many/parts").is_err());
}

#[test]
fn test_id_validations() {
    assert!(LogicalServiceName::try_new("").is_err());
    assert!(LogicalServiceName::try_new("some/name").is_err());
    assert!(LogicalServiceName::try_new("good-name").is_ok());

    assert!(ServiceId::try_new("not-did-key").is_err());
    assert!(ServiceId::try_new("did:key:123").is_ok());
}

// ── `MemberRef` and the `#`/`/` validators ──────

#[test]
fn member_ref_round_trips_through_display_and_from_str() {
    let m = MemberRef {
        logical_ref: LogicalServiceRef {
            app_instance_id: AppInstanceId::new("inst-1"),
            service_name: LogicalServiceName::new("backend"),
        },
        index: 2,
    };
    assert_eq!(m.to_string(), "inst-1/backend#2");
    let parsed = MemberRef::from_str("inst-1/backend#2").unwrap();
    assert_eq!(parsed, m);

    assert!(MemberRef::from_str("inst-1/backend").is_err(), "no index separator at all");
    assert!(MemberRef::from_str("inst-1/backend#not-a-number").is_err());
}

/// A service name that (illegally, pre-validation) carried the index
/// separator itself must not parse as if the `#` split were the real
/// one -- `LogicalServiceName`'s own validator is what actually
/// prevents this from ever being stored, but `MemberRef::from_str`
/// must independently reject it too, since it re-derives
/// `LogicalServiceRef::try_new` on whatever sits before the last `#`.
#[test]
fn member_ref_parse_rejects_a_service_name_carrying_the_index_separator() {
    let err = MemberRef::from_str("inst-1/back#end#3").unwrap_err();
    assert!(err.to_string().contains('#'), "{err}");
}

#[test]
fn a_logical_service_name_containing_the_index_separator_is_refused() {
    assert!(LogicalServiceName::try_new("back#end").is_err());
    assert!(LogicalServiceName::try_new("backend").is_ok());
}

#[test]
fn an_app_instance_id_containing_a_separator_is_refused() {
    assert!(AppInstanceId::try_new("inst/1").is_err());
    assert!(AppInstanceId::try_new("inst#1").is_err());
    assert!(AppInstanceId::try_new("inst-1").is_ok());
}

/// `AppDid` is interpolated straight into a `synapp:<app-did>`
/// `ResourceUri` (ADR-0022 §5) -- a `/` or `#` in it would produce a
/// selector-bearing resource `covers_resource` treats under a
/// different rule, the same reason its two siblings above forbid them.
#[test]
fn an_app_did_containing_a_separator_is_refused() {
    assert!(AppDid::try_new("did:key:zAbc/evil").is_err());
    assert!(AppDid::try_new("did:key:zAbc#evil").is_err());
    assert!(AppDid::try_new("did:key:zAbc").is_ok());
}

/// `..` and `\` are refused at construction now, not only later at
/// `crates/app_supervisor/src/keys.rs`'s
/// `validate_backup_name` -- an id that can never be a vault backup
/// name must never exist, rather than being accepted here and only
/// failing, permanently, the first time `adopt` tries to mint the
/// app-instance master under it.
#[test]
fn an_app_instance_id_that_could_never_be_backed_up_is_refused_at_construction() {
    assert!(AppInstanceId::try_new("..").is_err());
    assert!(AppInstanceId::try_new("inst..1").is_err());
    assert!(AppInstanceId::try_new("back\\slash").is_err());
    assert!(AppInstanceId::try_new("inst-1").is_ok());
}

#[test]
fn a_planned_services_member_ref_combines_its_logical_ref_and_index() {
    let svc = PlannedService {
        service_id: ServiceId::new("did:key:h123"),
        logical_ref: LogicalServiceRef {
            app_instance_id: AppInstanceId::new("inst-1"),
            service_name: LogicalServiceName::new("backend"),
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
            rotation_policy: RotationPolicy::RestartOnRotation,
            fdae: None,
            health_check: None,
            assets: None,
            visibility: Visibility::Private,
        },
        resolved_dependencies: BTreeMap::new(),
        topology_mode: TopologyMode::Singleton,
        member_index: 3,
        schedule: None,
        sharding_strategy: None,
        topology_visibility: TopologyVisibility::Restricted,
    };
    assert_eq!(svc.member_ref().to_string(), "inst-1/backend#3");
}

/// An unscaled plan's JSON must stay byte-for-byte what it was before
/// `member_index` existed -- the field is skip-if-zero, so an older
/// stored plan and a fresh single-member one serialize identically.
#[test]
fn member_index_zero_emits_no_key_in_serialized_output() {
    let svc = PlannedService {
        service_id: ServiceId::new("did:key:h123"),
        logical_ref: LogicalServiceRef {
            app_instance_id: AppInstanceId::new("inst-1"),
            service_name: LogicalServiceName::new("backend"),
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
            rotation_policy: RotationPolicy::RestartOnRotation,
            fdae: None,
            health_check: None,
            assets: None,
            visibility: Visibility::Private,
        },
        resolved_dependencies: BTreeMap::new(),
        topology_mode: TopologyMode::Singleton,
        member_index: 0,
        schedule: None,
        sharding_strategy: None,
        topology_visibility: TopologyVisibility::Restricted,
    };
    let toml = toml::to_string(&svc).unwrap();
    assert!(!toml.contains("member_index"));
}

#[test]
fn test_negative_parsing_and_validation() {
    // Missing required field
    let malformed_toml = r#"
            id = "syneroym:bad"
        "#;
    assert!(SynAppManifest::from_toml(malformed_toml).is_err());

    // Circular dependency
    let circular_toml = r#"
            id = "syneroym:bad"
            version = "0.1.0"
            [services.a]
            service_type = "wasm"
            source = "a"
            depends_on = ["b"]

            [services.b]
            service_type = "wasm"
            source = "b"
            depends_on = ["a"]
        "#;
    let manifest_res = SynAppManifest::from_toml(circular_toml);
    assert!(manifest_res.is_err());
    assert!(manifest_res.err().unwrap().to_string().contains("Circular dependency"));

    // Undefined dependency
    let missing_dep_toml = r#"
            id = "syneroym:bad"
            version = "0.1.0"
            [services.a]
            service_type = "wasm"
            source = "a"
            depends_on = ["nonexistent"]
        "#;
    let manifest_res2 = SynAppManifest::from_toml(missing_dep_toml);
    assert!(manifest_res2.is_err());
    assert!(manifest_res2.err().unwrap().to_string().contains("undefined service"));
}

#[test]
fn test_toml_env_serialization() {
    let mut env_map = BTreeMap::new();
    env_map.insert("DATABASE_URL".to_string(), "postgres://...".to_string());
    env_map.insert("PORT".to_string(), "8080".to_string());

    let plan = DeploymentPlan {
        app_instance_id: AppInstanceId::new("guild-instance-1"),
        blueprint_id: AppBlueprintId::new("syneroym:guild-app"),
        version: Version::parse("0.1.0").unwrap(),
        services: vec![PlannedService {
            service_id: ServiceId::new("did:key:h123"),
            logical_ref: LogicalServiceRef {
                app_instance_id: AppInstanceId::new("guild-instance-1"),
                service_name: LogicalServiceName::new("identity"),
            },
            substrate: None,
            config: ServiceConfig {
                service_type: ServiceType::Wasm,
                source: "crates/sandbox_wasm/benches/identity.wasm".to_string(),
                hash: None,
                interfaces: vec![],
                env: env_map,
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: RotationPolicy::RestartOnRotation,
                fdae: None,
                health_check: None,
                assets: None,
                visibility: Visibility::Private,
            },
            resolved_dependencies: BTreeMap::new(),
            topology_mode: TopologyMode::Singleton,
            member_index: 0,
            schedule: None,
            sharding_strategy: None,
            topology_visibility: TopologyVisibility::Restricted,
        }],
    };

    let toml_str = plan.to_toml().unwrap();
    assert!(toml_str.contains("DATABASE_URL"));
    assert!(toml_str.contains("PORT"));
}

#[test]
fn a_manifest_default_placement_round_trips_through_toml_and_json() {
    let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [placement]
            substrate = "edge-1"

            [services.identity]
            service_type = "wasm"
            source = "identity.wasm"
        "#;

    let manifest = SynAppManifest::from_toml(toml_str).unwrap();
    assert_eq!(
        manifest.placement,
        Some(PlacementSelector::Substrate(SubstrateAlias::new("edge-1")))
    );

    let toml_round = manifest.to_toml().unwrap();
    assert_eq!(SynAppManifest::from_toml(&toml_round).unwrap(), manifest);

    let json_round = manifest.to_json().unwrap();
    assert_eq!(SynAppManifest::from_json(&json_round).unwrap(), manifest);

    // Absent placement stays absent -- pre-placement manifests are unaffected.
    let no_placement = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"
        "#;
    assert_eq!(SynAppManifest::from_toml(no_placement).unwrap().placement, None);
}

#[test]
fn a_per_service_placement_override_round_trips_through_toml_and_json() {
    let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [placement]
            substrate = "edge-1"

            [services.identity]
            service_type = "wasm"
            source = "identity.wasm"

            [services.identity.placement]
            substrate = "edge-2"
        "#;

    let manifest = SynAppManifest::from_toml(toml_str).unwrap();
    let identity = manifest.services.get(&LogicalServiceName::new("identity")).unwrap();
    assert_eq!(
        identity.placement,
        Some(PlacementSelector::Substrate(SubstrateAlias::new("edge-2")))
    );

    // This is the round trip that would catch a #[serde(flatten)] regression:
    // an externally-tagged enum nested inside a struct also using `flatten`.
    let toml_round = manifest.to_toml().unwrap();
    assert_eq!(SynAppManifest::from_toml(&toml_round).unwrap(), manifest);

    let json_round = manifest.to_json().unwrap();
    assert_eq!(SynAppManifest::from_json(&json_round).unwrap(), manifest);
}

#[test]
fn a_substrate_alias_rejects_a_bare_did() {
    let err = SubstrateAlias::try_new("did:key:z6MkExample").unwrap_err();
    assert!(err.to_string().contains("looks like a DID"));
}

#[test]
fn a_substrate_alias_rejects_an_empty_name() {
    assert!(SubstrateAlias::try_new("").is_err());
}

#[test]
fn a_substrate_alias_rejects_a_path_separator() {
    assert!(SubstrateAlias::try_new("edge/1").is_err());
}

#[test]
fn a_planned_service_round_trips_its_substrate() {
    let mut svc = PlannedService {
        service_id: ServiceId::new("did:key:h123"),
        logical_ref: LogicalServiceRef {
            app_instance_id: AppInstanceId::new("guild-instance-1"),
            service_name: LogicalServiceName::new("identity"),
        },
        substrate: Some(SubstrateAlias::new("edge-1")),
        config: ServiceConfig {
            service_type: ServiceType::Wasm,
            source: "identity.wasm".to_string(),
            hash: None,
            interfaces: vec![],
            env: BTreeMap::new(),
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: RotationPolicy::RestartOnRotation,
            fdae: None,
            health_check: None,
            assets: None,
            visibility: Visibility::Private,
        },
        resolved_dependencies: BTreeMap::new(),
        topology_mode: TopologyMode::Singleton,
        member_index: 0,
        schedule: None,
        sharding_strategy: None,
        topology_visibility: TopologyVisibility::Restricted,
    };

    let toml_round = toml::to_string(&svc).unwrap();
    assert!(toml_round.contains("edge-1"));
    let parsed: PlannedService = toml::from_str(&toml_round).unwrap();
    assert_eq!(parsed, svc);

    // `None` serializes with no `substrate` key at all, matching
    // every manifest written before placement existed.
    svc.substrate = None;
    let toml_round = toml::to_string(&svc).unwrap();
    assert!(!toml_round.contains("substrate"));
}

#[test]
fn a_health_check_round_trips_through_toml_and_json() {
    let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.backend]
            service_type = "container"
            source = "unused"
            interfaces = ["http"]

            [services.backend.health_check.http-get]
            interface = "http"
            path = "/healthz"
            expect_status = 200
            timeout_ms = 1500

            [services.tcpsvc]
            service_type = "tcp"
            source = "unused"
            interfaces = ["main"]

            [services.tcpsvc.health_check.tcp-connect]
            interface = "main"

            [services.wasmsvc]
            service_type = "wasm"
            source = "unused"
            interfaces = ["rpc"]

            [services.wasmsvc.health_check.rpc]
            interface = "rpc"
            method = "ping"
        "#;

    let manifest = SynAppManifest::from_toml(toml_str).unwrap();
    let backend = manifest.services.get(&LogicalServiceName::new("backend")).unwrap();
    assert_eq!(
        backend.config.health_check,
        Some(HealthCheck::HttpGet(HttpProbe {
            interface: InterfaceName::new("http"),
            path: "/healthz".to_string(),
            expect_status: 200,
            timeout_ms: 1500,
        }))
    );

    // This is the round trip that would catch a #[serde(flatten)] regression:
    // an externally-tagged enum nested inside a struct also using `flatten`.
    let toml_round = manifest.to_toml().unwrap();
    assert_eq!(SynAppManifest::from_toml(&toml_round).unwrap(), manifest);

    let json_round = manifest.to_json().unwrap();
    assert_eq!(SynAppManifest::from_json(&json_round).unwrap(), manifest);
}

#[test]
fn an_absent_health_check_emits_no_key() {
    let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.identity]
            service_type = "wasm"
            source = "unused"
        "#;
    let manifest = SynAppManifest::from_toml(toml_str).unwrap();
    let identity = manifest.services.get(&LogicalServiceName::new("identity")).unwrap();
    assert_eq!(identity.config.health_check, None);
    let serialized = manifest.to_toml().unwrap();
    assert!(!serialized.contains("health_check"));
}

#[test]
fn probe_defaults_apply_when_omitted() {
    let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.backend]
            service_type = "container"
            source = "unused"

            [services.backend.health_check.http-get]
            interface = "http"
            path = "/healthz"
        "#;
    let manifest = SynAppManifest::from_toml(toml_str).unwrap();
    let backend = manifest.services.get(&LogicalServiceName::new("backend")).unwrap();
    match backend.config.health_check.as_ref().unwrap() {
        HealthCheck::HttpGet(p) => {
            assert_eq!(p.expect_status, 200);
            assert_eq!(p.timeout_ms, DEFAULT_PROBE_TIMEOUT_MS);
        }
        other => panic!("expected HttpGet, got {other:?}"),
    }
}

#[test]
fn valid_for_pairs_each_kind_with_its_service_types() {
    let tcp = HealthCheck::TcpConnect(TcpProbe {
        interface: InterfaceName::new("main"),
        timeout_ms: DEFAULT_PROBE_TIMEOUT_MS,
    });
    assert_eq!(tcp.valid_for(), &[ServiceType::Tcp, ServiceType::Container]);

    let http = HealthCheck::HttpGet(HttpProbe {
        interface: InterfaceName::new("main"),
        path: "/healthz".to_string(),
        expect_status: 200,
        timeout_ms: DEFAULT_PROBE_TIMEOUT_MS,
    });
    assert_eq!(http.valid_for(), &[ServiceType::Tcp, ServiceType::Container]);

    let rpc = HealthCheck::Rpc(RpcProbe {
        interface: InterfaceName::new("main"),
        method: "ping".to_string(),
        timeout_ms: DEFAULT_PROBE_TIMEOUT_MS,
    });
    assert_eq!(rpc.valid_for(), &[ServiceType::Wasm]);
}

// ── `ScheduleSpec` on the manifest surface ──────────────────────────

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

/// A zero budget is not "no limit": the supervisor's timeout elapses
/// before the call is made, and the watermark is already written by
/// then -- so the tick is consumed, an alert is raised, and every later
/// tick repeats the cycle. A schedule that can never run and never says
/// why must be refused where the author can still read the refusal.
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

/// Above the ceiling is refused rather than silently clamped: a
/// manifest must never run under a budget different from the one it
/// asks for.
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
            },
        );
    }
    let err = manifest.validate().unwrap_err();
    assert!(err.to_string().contains("above the cap"), "{err}");
}

// ── `ShardingStrategy` on the manifest surface (ADR-0022 §6) ────────

#[test]
fn a_sharding_strategy_round_trips_through_a_manifest() {
    let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.worker]
            service_type = "wasm"
            source = "unused"
            replicas = 2
            sharding_strategy = "hash_sharding"
        "#;
    let manifest = SynAppManifest::from_toml(toml_str).unwrap();
    let worker = manifest.services.get(&LogicalServiceName::new("worker")).unwrap();
    assert_eq!(worker.sharding_strategy, Some(ShardingStrategy::HashSharding));

    let toml_round = manifest.to_toml().unwrap();
    assert_eq!(SynAppManifest::from_toml(&toml_round).unwrap(), manifest);
    let json_round = manifest.to_json().unwrap();
    assert_eq!(SynAppManifest::from_json(&json_round).unwrap(), manifest);
}

#[test]
fn a_manifest_with_no_sharding_strategy_parses_as_it_does_today() {
    let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.worker]
            service_type = "wasm"
            source = "unused"
        "#;
    let manifest = SynAppManifest::from_toml(toml_str).unwrap();
    let worker = manifest.services.get(&LogicalServiceName::new("worker")).unwrap();
    assert_eq!(worker.sharding_strategy, None);
    let serialized = manifest.to_toml().unwrap();
    assert!(!serialized.contains("sharding_strategy"));
}

#[test]
fn a_sharding_strategy_with_replicas_of_one_is_refused_at_validation() {
    let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.worker]
            service_type = "wasm"
            source = "unused"
            sharding_strategy = "hash_sharding"
        "#;
    let err = SynAppManifest::from_toml(toml_str).unwrap_err();
    assert!(err.to_string().contains("not a selection"), "{err}");
}

/// `RangeSharding` names concrete `ServiceId`s, which a manifest is
/// authored long before any of a scaled service's members are minted --
/// so it is refused here rather than accepted and left permanently
/// unreachable.
#[test]
fn a_range_sharding_strategy_is_refused_at_validation() {
    let toml_str = r#"
            id = "syneroym:guild-app"
            version = "0.1.0"

            [services.worker]
            service_type = "wasm"
            source = "unused"
            replicas = 2

            [services.worker.sharding_strategy]
            range_sharding = { chunks = [] }
        "#;
    let err = SynAppManifest::from_toml(toml_str).unwrap_err();
    assert!(err.to_string().contains("range_sharding"), "{err}");
}
