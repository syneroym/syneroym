use clap::CommandFactory;
use syneroym_app_orchestration::{
    DEFAULT_SCHEDULE_TIMEOUT_MS,
    models::{InterfaceName, ScheduleSpec},
};
use syneroym_identity::substrate;

use super::*;

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
