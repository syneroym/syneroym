use clap::{CommandFactory, Parser};
use syneroym_app_orchestration::{
    DEFAULT_SCHEDULE_TIMEOUT_MS,
    models::{InterfaceName, ScheduleSpec, ServiceId, TopologyMode},
};
use syneroym_identity::substrate;
use syneroym_sdk::{
    BindingWrite, BindingWriteOutcome, DeploymentPlan as WitDeploymentPlan, deploy::SubstrateActor,
};

use super::*;

#[derive(Parser)]
struct DummyCli {
    #[command(subcommand)]
    command: AppCommands,
}

#[test]
fn test_app_reconcile_command_parsing() {
    let cli = DummyCli::try_parse_from([
        "dummy",
        "reconcile",
        "inst-1",
        "--manifest-path",
        "test.toml",
        "--journal-path",
        "test.db",
    ])
    .unwrap();

    match cli.command {
        AppCommands::Reconcile { instance_id, manifest_path, journal_path } => {
            assert_eq!(instance_id, "inst-1");
            assert_eq!(manifest_path, Some(PathBuf::from("test.toml")));
            assert_eq!(journal_path, PathBuf::from("test.db"));
        }
        _ => panic!("Expected Reconcile command"),
    }
}

#[test]
fn test_app_forget_command_parsing() {
    let cli = DummyCli::try_parse_from([
        "dummy",
        "forget",
        "inst-1",
        "--service",
        "backend",
        "--journal-path",
        "test.db",
    ])
    .unwrap();

    match cli.command {
        AppCommands::Forget { instance_id, service, journal_path } => {
            assert_eq!(instance_id, "inst-1");
            assert_eq!(service, "backend");
            assert_eq!(journal_path, PathBuf::from("test.db"));
        }
        _ => panic!("Expected Forget command"),
    }
}

#[test]
fn test_app_health_command_parsing() {
    let cli = DummyCli::try_parse_from([
        "dummy",
        "health",
        "inst-1",
        "--journal-path",
        "test.db",
        "--watch",
        "5",
        "--strict",
    ])
    .unwrap();

    match cli.command {
        AppCommands::Health { instance_id, journal_path, watch, strict, no_record, .. } => {
            assert_eq!(instance_id, "inst-1");
            assert_eq!(journal_path, PathBuf::from("test.db"));
            assert_eq!(watch, Some(5));
            assert!(strict);
            assert!(!no_record);
        }
        _ => panic!("Expected Health command"),
    }
}

#[test]
fn test_app_alerts_command_parsing() {
    let cli = DummyCli::try_parse_from(["dummy", "alerts", "inst-1", "--all"]).unwrap();

    match cli.command {
        AppCommands::Alerts { instance_id, all, .. } => {
            assert_eq!(instance_id, "inst-1");
            assert!(all);
        }
        _ => panic!("Expected Alerts command"),
    }
}

#[test]
fn health_help_lists_no_record_watch_and_strict() {
    let mut cmd = DummyCli::command();
    let help = cmd
        .get_subcommands_mut()
        .find(|c| c.get_name() == "health")
        .expect("health subcommand")
        .render_help()
        .to_string();
    assert!(help.contains("--no-record"), "{help}");
    assert!(help.contains("--watch"), "{help}");
    assert!(help.contains("--strict"), "{help}");
}

#[test]
fn deploy_help_lists_inventory() {
    let mut cmd = DummyCli::command();
    let help = cmd
        .get_subcommands_mut()
        .find(|c| c.get_name() == "deploy")
        .expect("deploy subcommand")
        .render_help()
        .to_string();
    assert!(help.contains("--inventory"), "{help}");
}

#[test]
fn resolve_under_leaves_an_absolute_path_untouched() {
    let dir = Path::new("/roymctl/dir");
    let abs = Path::new("/etc/grants/edge-1.json");
    assert_eq!(resolve_under(dir, abs), abs);
}

#[test]
fn resolve_under_joins_a_relative_path_under_dir() {
    let dir = Path::new("/roymctl/dir");
    let rel = Path::new("grants/edge-1.json");
    assert_eq!(resolve_under(dir, rel), Path::new("/roymctl/dir/grants/edge-1.json"));
}

fn entry(identity: Option<&str>, ucan: Option<&str>) -> SubstrateEntry {
    SubstrateEntry {
        did: "did:key:z6MkExampleNodeA".to_string(),
        api_url: None,
        identity: identity.map(str::to_string),
        ucan: ucan.map(PathBuf::from),
        capabilities: None,
    }
}

/// An entry overriding neither field inherits the global identity/ucan
/// pair as-is.
#[test]
fn resolve_credentials_falls_back_to_the_global_pair_when_the_entry_sets_neither() {
    let alias = SubstrateAlias::new("edge-1");
    let e = entry(None, None);
    let (id, ucan) = resolve_credentials(
        &alias,
        &e,
        Path::new("substrates.toml"),
        Path::new("/dir"),
        Some("global-op"),
        Some(Path::new("grants/global.json")),
    )
    .unwrap();
    assert_eq!(id, Some("global-op"));
    assert_eq!(ucan.as_deref(), Some(Path::new("grants/global.json")));
}

/// An entry overriding both fields together is always consistent,
/// regardless of what the globals are.
#[test]
fn resolve_credentials_uses_the_entrys_own_pair_when_it_sets_both() {
    let alias = SubstrateAlias::new("edge-1");
    let e = entry(Some("edge1-op"), Some("grants/edge-1.json"));
    let (id, ucan) = resolve_credentials(
        &alias,
        &e,
        Path::new("substrates.toml"),
        Path::new("/dir"),
        Some("global-op"),
        Some(Path::new("grants/global.json")),
    )
    .unwrap();
    assert_eq!(id, Some("edge1-op"));
    assert_eq!(ucan.as_deref(), Some(Path::new("/dir/grants/edge-1.json")));
}

/// The hazard: `identity` overridden, `ucan` left to fall back to a
/// *global* `--ucan` whose audience is the global identity, not this
/// entry's. Must be rejected, not silently paired.
#[test]
fn resolve_credentials_rejects_identity_override_with_a_global_ucan_present() {
    let alias = SubstrateAlias::new("edge-1");
    let e = entry(Some("edge1-op"), None);
    let err = resolve_credentials(
        &alias,
        &e,
        Path::new("substrates.toml"),
        Path::new("/dir"),
        Some("global-op"),
        Some(Path::new("grants/global.json")),
    )
    .unwrap_err();
    assert!(err.to_string().contains("edge-1"), "{err}");
}

/// The symmetric case: `ucan` overridden, `identity` left to fall back to
/// a global `--as` the entry's token was never minted for.
#[test]
fn resolve_credentials_rejects_ucan_override_with_a_global_identity_present() {
    let alias = SubstrateAlias::new("edge-1");
    let e = entry(None, Some("grants/edge-1.json"));
    let err = resolve_credentials(
        &alias,
        &e,
        Path::new("substrates.toml"),
        Path::new("/dir"),
        Some("global-op"),
        None,
    )
    .unwrap_err();
    assert!(err.to_string().contains("edge-1"), "{err}");
}

// The placement-change refusal never calls the actor -- it only reads
// `DeployTarget`'s own fields -- so a fake that panics if ever invoked is
// enough to keep these tests free of any live substrate.
#[derive(Debug)]
struct NoopApplier;

#[async_trait::async_trait]
impl SubstrateActor for NoopApplier {
    async fn apply_plan(&self, _plan: WitDeploymentPlan) -> Result<(), String> {
        unimplemented!("check_no_placement_change must never call apply_plan()")
    }

    async fn write_bindings(
        &self,
        _write: BindingWrite,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        unimplemented!("check_no_placement_change must never call write_bindings()")
    }

    async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
        unimplemented!("check_no_placement_change must never call restart()")
    }

    async fn renew_cert(
        &self,
        _service_id: String,
        _generation: u64,
        _instance_certificate: String,
    ) -> Result<(), String> {
        unimplemented!("check_no_placement_change must never call renew_cert()")
    }

    async fn instance_identity(
        &self,
        _service_id: &str,
    ) -> Result<syneroym_sdk::InstanceIdentity, String> {
        unimplemented!("check_no_placement_change must never call instance_identity()")
    }

    async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
        unimplemented!("check_no_placement_change must never call held_generation()")
    }
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
        visibility: Default::default(),
    }
}

fn planned_service(
    logical_ref: LogicalServiceRef,
    service_id: &str,
    alias: &str,
) -> PlannedService {
    PlannedService {
        service_id: ServiceId::new(service_id),
        logical_ref,
        substrate: Some(SubstrateAlias::new(alias)),
        config: dummy_config(),
        resolved_dependencies: BTreeMap::new(),
        topology_mode: TopologyMode::Singleton,
        member_index: 0,
        schedule: None,
        sharding_strategy: None,
        topology_visibility: Default::default(),
    }
}

fn deploy_target(did: &str, alias: &str) -> DeployTarget {
    DeployTarget {
        alias: Some(SubstrateAlias::new(alias)),
        substrate_did: did.to_string(),
        actor: Arc::new(NoopApplier),
    }
}

/// The message must resolve the real, deployed member-master DID -- the
/// journal's plan JSON stores only the compiler's fabricated id, which
/// the operator cannot act on.
#[test]
fn a_placement_change_is_refused_naming_the_deployed_service_id() {
    let dir = tempfile::tempdir().unwrap();
    let logical_ref = LogicalServiceRef {
        app_instance_id: AppInstanceId::new("inst-1"),
        service_name: LogicalServiceName::new("backend"),
    };
    let name = member_identity::member_master_name(&logical_ref, 0);
    let master = member_identity::resolve_or_mint_member_master(dir.path(), &name).unwrap();
    let real_did = substrate::derive_did_key(&master.public_key());

    let svc = planned_service(logical_ref.clone(), "did:key:hFabricated", "edge-2");
    let target = deploy_target("did:key:zNewNode", "edge-2");
    let placed = vec![(&svc, &target)];
    let landed = vec![ActionRecord {
        action_type: "ADD".to_string(),
        logical_ref: format!("{logical_ref}#0"),
        substrate_alias: Some("edge-1".to_string()),
        substrate_did: "did:key:zOldNode".to_string(),
    }];

    let err = check_no_placement_change(dir.path(), &placed, &landed).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains(&real_did), "{msg}");
    assert!(msg.contains("edge-1"), "{msg}");
    assert!(msg.contains("edge-2"), "{msg}");
}

fn dummy_deployment_plan(instance_id: &AppInstanceId, svc: PlannedService) -> DeploymentPlan {
    DeploymentPlan {
        app_instance_id: instance_id.clone(),
        blueprint_id: AppBlueprintId::new("syneroym:test"),
        version: Version::new(1, 0, 0),
        services: vec![svc],
    }
}

/// A first partial deploy leaves the record `Degraded` with one
/// `COMPLETED` row and no `ACTIVE` record at all. A refusal that read
/// only the `ACTIVE` record would pass this run silently and leave the
/// service running on two nodes.
///
/// `landed` is built through a real journal here, the same way `handle`
/// does, and read back with the real query -- not a hand-typed literal.
/// `check_no_placement_change` takes `landed` as a plain slice by design
/// (so it needs no live substrate), so building the state for real is
/// the only way to pin that the rows come from `COMPLETED` actions
/// across every record rather than the last `ACTIVE` plan.
#[test]
fn a_placement_change_is_refused_after_a_degraded_run_not_only_an_active_one() {
    let dir = tempfile::tempdir().unwrap();
    let instance_id = AppInstanceId::new("inst-1");
    let logical_ref = LogicalServiceRef {
        app_instance_id: instance_id.clone(),
        service_name: LogicalServiceName::new("backend"),
    };

    let journal = DeploymentJournal::open_in_memory().unwrap();
    let plan = dummy_deployment_plan(
        &instance_id,
        planned_service(logical_ref.clone(), "did:key:hFabricated", "edge-1"),
    );
    let deployment_id = journal.append(&plan, DeploymentState::Applying).unwrap();
    journal
        .append_action(
            deployment_id,
            "ADD",
            &format!("{logical_ref}#0"),
            Some("edge-1"),
            "did:key:zOldNode",
            ActionState::Completed,
        )
        .unwrap();
    journal.update_state(deployment_id, DeploymentState::Degraded).unwrap();

    // No ACTIVE record exists for this instance at all -- an
    // `ACTIVE`-sourced refusal would find nothing and pass silently.
    assert!(journal.get_last_state(&instance_id, DeploymentState::Active).unwrap().is_none());

    let landed = journal.get_completed_actions_for_instance(&instance_id).unwrap();

    let svc = planned_service(logical_ref, "did:key:hFabricated", "edge-2");
    let target = deploy_target("did:key:zNewNode", "edge-2");
    let placed = vec![(&svc, &target)];

    let err = check_no_placement_change(dir.path(), &placed, &landed).unwrap_err();
    assert!(err.to_string().contains("already deployed"));
}

#[test]
fn no_placement_change_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let logical_ref = LogicalServiceRef {
        app_instance_id: AppInstanceId::new("inst-1"),
        service_name: LogicalServiceName::new("backend"),
    };

    let svc = planned_service(logical_ref.clone(), "did:key:hFabricated", "edge-1");
    let target = deploy_target("did:key:zSameNode", "edge-1");
    let placed = vec![(&svc, &target)];
    let landed = vec![ActionRecord {
        action_type: "ADD".to_string(),
        logical_ref: logical_ref.to_string(),
        substrate_alias: Some("edge-1".to_string()),
        substrate_did: "did:key:zSameNode".to_string(),
    }];

    check_no_placement_change(dir.path(), &placed, &landed).unwrap();
}

/// A most-recent `REMOVE` row (what `app forget` appends) must clear the
/// refusal, even though an older `ADD` row for the same logical ref
/// still sits underneath it -- a `rfind` scoped to `ADD` alone would
/// miss the `REMOVE` and refuse forever.
#[test]
fn a_remove_row_after_an_add_clears_the_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let logical_ref = LogicalServiceRef {
        app_instance_id: AppInstanceId::new("inst-1"),
        service_name: LogicalServiceName::new("backend"),
    };

    let svc = planned_service(logical_ref.clone(), "did:key:hFabricated", "edge-2");
    let target = deploy_target("did:key:zNewNode", "edge-2");
    let placed = vec![(&svc, &target)];
    let landed = vec![
        ActionRecord {
            action_type: "ADD".to_string(),
            logical_ref: logical_ref.to_string(),
            substrate_alias: Some("edge-1".to_string()),
            substrate_did: "did:key:zOldNode".to_string(),
        },
        ActionRecord {
            action_type: "REMOVE".to_string(),
            logical_ref: logical_ref.to_string(),
            substrate_alias: Some("edge-1".to_string()),
            substrate_did: "did:key:zOldNode".to_string(),
        },
    ];

    check_no_placement_change(dir.path(), &placed, &landed).unwrap();
}

/// `app forget` end to end: a real journal, on disk, exactly as `handle`
/// itself opens it -- proving the whole `REMOVE`-row escape, not just
/// `check_no_placement_change`'s half of it.
#[tokio::test]
async fn app_forget_appends_a_remove_row_that_clears_a_later_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let instance_id = AppInstanceId::new("inst-forget");
    let logical_ref = LogicalServiceRef {
        app_instance_id: instance_id.clone(),
        service_name: LogicalServiceName::new("backend"),
    };
    let journal_path = dir.path().join("deployments.db");

    {
        let journal = DeploymentJournal::open(dir.path(), "deployments.db").unwrap();
        let plan = dummy_deployment_plan(
            &instance_id,
            planned_service(logical_ref.clone(), "did:key:hFabricated", "edge-1"),
        );
        let deployment_id = journal.append(&plan, DeploymentState::Active).unwrap();
        journal
            .append_action(
                deployment_id,
                "ADD",
                &format!("{logical_ref}#0"),
                Some("edge-1"),
                "did:key:zOldNode",
                ActionState::Completed,
            )
            .unwrap();
    }

    handle(
        &AppCommands::Forget {
            instance_id: instance_id.to_string(),
            service: "backend".to_string(),
            journal_path: journal_path.clone(),
        },
        "http://localhost:1",
        None,
        dir.path(),
        None,
        None,
    )
    .await
    .unwrap();

    let journal = DeploymentJournal::open(dir.path(), "deployments.db").unwrap();
    let landed = journal.get_completed_actions_for_instance(&instance_id).unwrap();
    let last = landed.iter().rev().find(|r| r.logical_ref == format!("{logical_ref}#0")).unwrap();
    assert_eq!(last.action_type, "REMOVE");
    assert_eq!(last.substrate_did, "did:key:zOldNode");

    // A redeploy naming a different substrate is no longer refused.
    let svc = planned_service(logical_ref, "did:key:hFabricated", "edge-2");
    let target = deploy_target("did:key:zNewNode", "edge-2");
    let placed = vec![(&svc, &target)];
    check_no_placement_change(dir.path(), &placed, &landed).unwrap();

    // Forgetting again is a no-op, not a second REMOVE row.
    handle(
        &AppCommands::Forget {
            instance_id: instance_id.to_string(),
            service: "backend".to_string(),
            journal_path,
        },
        "http://localhost:1",
        None,
        dir.path(),
        None,
        None,
    )
    .await
    .unwrap();
    let landed_again = journal.get_completed_actions_for_instance(&instance_id).unwrap();
    assert_eq!(landed_again.len(), landed.len(), "{landed_again:?}");
}

/// `--service` names a logical service, and this command hardcodes
/// member 0 -- forgetting a scaled service used to silently forget
/// member 0 alone while its siblings stayed tracked as if nothing had
/// happened. Refused instead, naming the count.
#[tokio::test]
async fn app_forget_refuses_a_service_with_more_than_one_landed_member() {
    let dir = tempfile::tempdir().unwrap();
    let instance_id = AppInstanceId::new("inst-forget-scaled");
    let logical_ref = LogicalServiceRef {
        app_instance_id: instance_id.clone(),
        service_name: LogicalServiceName::new("backend"),
    };
    let journal_path = dir.path().join("deployments.db");

    {
        let journal = DeploymentJournal::open(dir.path(), "deployments.db").unwrap();
        let plan = dummy_deployment_plan(
            &instance_id,
            planned_service(logical_ref.clone(), "did:key:hFabricated", "edge-1"),
        );
        let deployment_id = journal.append(&plan, DeploymentState::Active).unwrap();
        journal
            .append_action(
                deployment_id,
                "ADD",
                &format!("{logical_ref}#0"),
                Some("edge-1"),
                "did:key:zNode0",
                ActionState::Completed,
            )
            .unwrap();
        journal
            .append_action(
                deployment_id,
                "ADD",
                &format!("{logical_ref}#1"),
                Some("edge-2"),
                "did:key:zNode1",
                ActionState::Completed,
            )
            .unwrap();
    }

    let err = handle(
        &AppCommands::Forget {
            instance_id: instance_id.to_string(),
            service: "backend".to_string(),
            journal_path: journal_path.clone(),
        },
        "http://localhost:1",
        None,
        dir.path(),
        None,
        None,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("2 landed members"), "{err}");

    // Neither member was touched.
    let journal = DeploymentJournal::open(dir.path(), "deployments.db").unwrap();
    let landed = journal.get_completed_actions_for_instance(&instance_id).unwrap();
    assert!(landed.iter().all(|r| r.action_type == "ADD"), "{landed:?}");
}

#[tokio::test]
async fn app_forget_with_nothing_deployed_names_the_service_and_instance() {
    let dir = tempfile::tempdir().unwrap();
    let journal_path = dir.path().join("deployments.db");
    let err = handle(
        &AppCommands::Forget {
            instance_id: "inst-empty".to_string(),
            service: "backend".to_string(),
            journal_path,
        },
        "http://localhost:1",
        None,
        dir.path(),
        None,
        None,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("nothing to forget"), "{err}");
}

// ── unmastered deploy refused when deps are declared ──

/// A manifest declaring a dependency has no unmastered deploy path --
/// the plan carries the compiler's fabricated ids, which resolve to no
/// real key, so binding it would only push the failure from deploy
/// time to the guest's first `dependency(...)` call.
#[test]
fn a_manifest_declaring_depends_on_is_refused_without_mint_masters() {
    let instance_id = AppInstanceId::new("inst-1");
    let logical_ref = LogicalServiceRef {
        app_instance_id: instance_id.clone(),
        service_name: LogicalServiceName::new("frontend"),
    };
    let mut svc = planned_service(logical_ref, "did:key:hFabricated", "edge-1");
    svc.resolved_dependencies.insert(
        LogicalServiceName::new("backend"),
        vec![ServiceId::new("did:key:hBackendFabricated")],
    );
    let plan = dummy_deployment_plan(&instance_id, svc);

    let err = refuse_unmastered_dependencies(&plan, false).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("backend"), "{msg}");
    assert!(msg.contains("--mint-masters"), "{msg}");
}

/// The boundary: an unmastered deploy of an independent service (no
/// declared dependencies) stays valid -- `svc deploy` and every
/// dependency-free manifest rely on this.
#[test]
fn a_manifest_with_no_dependencies_still_deploys_without_mint_masters() {
    let instance_id = AppInstanceId::new("inst-1");
    let logical_ref = LogicalServiceRef {
        app_instance_id: instance_id.clone(),
        service_name: LogicalServiceName::new("frontend"),
    };
    let svc = planned_service(logical_ref, "did:key:hFabricated", "edge-1");
    let plan = dummy_deployment_plan(&instance_id, svc);

    assert!(refuse_unmastered_dependencies(&plan, false).is_ok());
}

/// The check behind `app deploy`'s own warning: a
/// schedule declared without a supervisor behind it never runs, and
/// this is the logic that decides whether to say so.
#[test]
fn plan_declares_a_schedule_is_true_only_when_a_service_carries_one() {
    let instance_id = AppInstanceId::new("inst-1");
    let logical_ref = LogicalServiceRef {
        app_instance_id: instance_id.clone(),
        service_name: LogicalServiceName::new("worker"),
    };
    let unscheduled = planned_service(logical_ref.clone(), "did:key:hFabricated", "edge-1");
    assert!(!plan_declares_a_schedule(&dummy_deployment_plan(&instance_id, unscheduled)));

    let mut scheduled = planned_service(logical_ref, "did:key:hFabricated", "edge-1");
    scheduled.schedule = Some(ScheduleSpec {
        cron: "* * * * *".to_string(),
        interface: InterfaceName::new("scheduled-driver"),
        method: "tick".to_string(),
        params: None,
        timeout_ms: DEFAULT_SCHEDULE_TIMEOUT_MS,
    });
    assert!(plan_declares_a_schedule(&dummy_deployment_plan(&instance_id, scheduled)));
}

#[test]
fn test_app_resolve_command_parsing() {
    let cli =
        DummyCli::try_parse_from(["dummy", "resolve", "did:key:zAppMaster", "backend"]).unwrap();

    match cli.command {
        AppCommands::Resolve { app_did, service_name } => {
            assert_eq!(app_did, "did:key:zAppMaster");
            assert_eq!(service_name, "backend");
        }
        _ => panic!("Expected Resolve command"),
    }
}
