//! Tests for reconcile and forget CLI parsing and integration behaviour.

use std::path::PathBuf;

use super::*;

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
