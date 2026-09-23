use std::collections::BTreeMap;

use syneroym_app_orchestration::{
    ActionState,
    models::{AppBlueprintId, TopologyMode},
};

use super::{super::*, helpers::*};

#[tokio::test]
async fn every_verb_is_refused_without_substrate_admin() {
    let s = service();
    for (method, params) in [
        (
            "submit",
            serde_json::json!([{"app_instance_id": "i", "plan_json": "{}", "inventory_json": "{}", "generation": 0}]),
        ),
        ("adopt", serde_json::json!(["i"])),
        ("release", serde_json::json!(["i"])),
        ("pause", serde_json::json!(["i"])),
        ("resume", serde_json::json!(["i"])),
        ("retire", serde_json::json!(["i"])),
        ("force-reconcile", serde_json::json!(["i"])),
        ("export-master", serde_json::json!(["m"])),
        ("import-master", serde_json::json!(["m"])),
        ("status", serde_json::json!(["i"])),
        ("alerts", serde_json::json!(["i", false])),
        // No new resource namespace -- gated exactly like the
        // neighbouring verbs above.
        ("outbox", serde_json::json!(["i"])),
        ("dead-letters", serde_json::json!(["i"])),
        ("replay", serde_json::json!(["i", 1])),
        // No new resource namespace here either.
        ("schedules", serde_json::json!(["i"])),
        // `resolve` checks `synapp:<app-did>`, not `substrate:<node>`,
        // but a caller with no capabilities at all still denies on
        // either resource -- a syntactically valid DID is needed so
        // the call reaches the capability check rather than failing at
        // `InvalidParams` first.
        ("resolve", serde_json::json!(["did:key:zX", "backend"])),
    ] {
        let err = dispatch(&s, unauthenticated_caller(), method, params).await.unwrap_err();
        assert_eq!(err.code(), PERMISSION_DENIED_CODE, "{method} must deny without admin");
    }
}

/// The plan's own `app_instance_id` and the submission's outer
/// `app_instance_id` are both caller-supplied and, before this check,
/// never compared. A mismatch would key the journal and vault under
/// one instance while `status`/`adopt`/`retire` key on the other,
/// splitting the instance in two.
#[tokio::test]
async fn submit_is_refused_when_the_outer_instance_id_does_not_match_the_plans_own() {
    let s = service();
    let plan_json = plan_json_no_services("plan-says-inst-1");

    let err = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "submit",
        serde_json::json!([{
            "app_instance_id": "outer-says-inst-2",
            "plan_json": plan_json,
            "inventory_json": "{}",
            "generation": 0,
        }]),
    )
    .await
    .unwrap_err();
    let err = err.to_string();
    assert!(err.contains("plan-says-inst-1") && err.contains("outer-says-inst-2"), "{err}");
    assert!(s.store.get("outer-says-inst-2").unwrap().is_none());
    assert!(s.store.get("plan-says-inst-1").unwrap().is_none());
}

/// `submit` is the backstop entry point for the open-topology /
/// private-service contradiction check -- a plan reaching the
/// supervisor was never necessarily compiled through `compile()` (a
/// hand-built plan, or a client other than `roymctl`), so the same
/// refusal must fire here too, not only inside the compiler.
#[tokio::test]
async fn submit_is_refused_when_the_plan_declares_open_topology_over_a_private_service() {
    let s = service();
    let plan_json = serde_json::json!({
        "app_instance_id": "inst-contradiction",
        "blueprint_id": "syneroym:test",
        "version": "1.0.0",
        "services": [{
            "service_id": "did:key:hFabricated",
            "logical_ref": "inst-contradiction/backend",
            "substrate": null,
            "service_type": "tcp", "source": "127.0.0.1:9000",
            "rotation_policy": "none",
            "resolved_dependencies": {},
            "topology_mode": "singleton",
            "visibility": "private",
            "topology_visibility": "open",
        }]
    })
    .to_string();

    let err = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "submit",
        serde_json::json!([{
            "app_instance_id": "inst-contradiction",
            "plan_json": plan_json,
            "inventory_json": "{}",
            "generation": 0,
        }]),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, RpcError::InvalidParams(_)), "expected InvalidParams, got {err:?}");
    let err = err.to_string();
    assert!(err.contains("open") && err.contains("private"), "{err}");
    assert!(s.store.get("inst-contradiction").unwrap().is_none());
}

#[tokio::test]
async fn submit_is_refused_when_a_placed_alias_carries_no_credential() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
    let inventory_json =
        serde_json::json!({"edge-1": {"did": "did:key:zEdge1", "api_url": "http://127.0.0.1:1"}})
            .to_string();

    let err = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "submit",
        serde_json::json!([{
            "app_instance_id": "inst-1",
            "plan_json": plan_json,
            "inventory_json": inventory_json,
            "generation": 0,
        }]),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("no credential"), "{err}");
}

/// `deploy_submission` used to run the whole mint/certify/apply
/// pipeline *before* `store.submit`'s retired guard
/// ever ran, so a submit against a retired instance redeployed every
/// service and only then reported the rejection. The inventory here
/// carries no credential for the placed alias -- exactly
/// `submit_is_refused_when_a_placed_alias_carries_no_credential`'s
/// fixture -- so if the retired check did not run first, this would
/// fail with "no credential" instead, proving the ordering rather than
/// merely the outcome.
#[tokio::test]
async fn submit_against_a_retired_instance_is_refused_before_any_deploy_work_runs() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
    let inventory_json =
        serde_json::json!({"edge-1": {"did": "did:key:zEdge1", "api_url": "http://127.0.0.1:1"}})
            .to_string();
    s.store.submit("inst-1", &plan_json, &inventory_json, "did:key:zAdmin", 0).unwrap();
    s.store.retire("inst-1").unwrap();

    let err = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "submit",
        serde_json::json!([{
            "app_instance_id": "inst-1",
            "plan_json": plan_json,
            "inventory_json": inventory_json,
            "generation": 1,
        }]),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("retired"), "{err}");
    assert!(!err.to_string().contains("credential"), "{err}");
}

/// The generation check lived only at `store.submit`, which still ran
/// *after* `deploy_submission` --
/// including after that pipeline presented `s.generation` to the
/// substrate's own `check_generation`, which *accepts* a higher
/// generation and advances its stamp. So a wrong upward
/// `--generation` would have left the substrate ahead of this
/// supervisor's own store the moment the store then refused to
/// record it. The inventory here carries no credential, exactly
/// `submit_against_a_retired_instance_is_refused_before_any_deploy_
/// work_runs`'s fixture: a "generation" failure rather than a
/// "credential" one proves the check ran before any deploy work, not
/// merely that the submit failed for some other reason.
#[tokio::test]
async fn submit_at_the_wrong_generation_is_refused_before_any_deploy_work_runs() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
    let inventory_json =
        serde_json::json!({"edge-1": {"did": "did:key:zEdge1", "api_url": "http://127.0.0.1:1"}})
            .to_string();
    s.store.submit("inst-1", &plan_json, &inventory_json, "did:key:zAdmin", 0).unwrap();
    s.store.set_generation("inst-1", 3).unwrap();

    let err = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "submit",
        serde_json::json!([{
            "app_instance_id": "inst-1",
            "plan_json": plan_json,
            "inventory_json": inventory_json,
            "generation": 5,
        }]),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("generation"), "{err}");
    assert!(!err.to_string().contains("credential"), "{err}");
    // Store state must be untouched by the rejected attempt.
    assert_eq!(s.store.get("inst-1").unwrap().unwrap().generation, 3);
}

/// Same defect, `force-reconcile`'s side: it never calls `store.submit`
/// at all, so nothing on it refused a retired instance -- it would
/// just redeploy indefinitely.
#[tokio::test]
async fn force_reconcile_against_a_retired_instance_is_refused() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
    let inventory_json =
        serde_json::json!({"edge-1": {"did": "did:key:zEdge1", "api_url": "http://127.0.0.1:1"}})
            .to_string();
    s.store.submit("inst-1", &plan_json, &inventory_json, "did:key:zAdmin", 0).unwrap();
    s.store.retire("inst-1").unwrap();

    let err = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "force-reconcile",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("retired"), "{err}");
    assert!(!err.to_string().contains("credential"), "{err}");
}

/// Before `connect_best_effort`, `release_on_every_substrate` used
/// `build_clients`, whose contract
/// fails the whole call the moment one placed alias cannot be
/// reached -- so retiring an instance placed on even one unreachable
/// substrate was permanently impossible. `ucan: null` fails fast at
/// the credential check rather than waiting out a real connect
/// timeout; either way is "cannot reach it" for this purpose.
#[tokio::test]
async fn retire_succeeds_and_marks_the_store_retired_even_when_a_placed_substrate_is_unreachable() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
    let inventory_json = serde_json::json!({
        "edge-1": {"did": "did:key:zEdge1", "api_url": "http://127.0.0.1:1", "ucan": null}
    })
    .to_string();
    s.store.submit("inst-1", &plan_json, &inventory_json, "did:key:zAdmin", 0).unwrap();

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "retire",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    assert_eq!(res.payload.get("status").and_then(|v| v.as_str()), Some("retired"));
    let unreleased = res.payload.get("unreleased_substrates").and_then(|v| v.as_array()).unwrap();
    assert_eq!(unreleased.len(), 1, "{unreleased:?}");

    assert!(
        s.store.get("inst-1").unwrap().unwrap().retired,
        "the local store must still mark the instance retired"
    );
}

#[tokio::test]
async fn submit_against_a_locked_vault_names_inject_kek() {
    let s = service_with_locked_vault();
    let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
    let inventory_json = serde_json::json!({
        "edge-1": {"did": "did:key:zEdge1", "api_url": "http://127.0.0.1:1", "ucan": null}
    })
    .to_string();

    let err = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "submit",
        serde_json::json!([{
            "app_instance_id": "inst-1",
            "plan_json": plan_json,
            "inventory_json": inventory_json,
            "generation": 0,
        }]),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("inject-kek"), "{err}");
}

#[tokio::test]
async fn status_reports_the_delivery_note_rather_than_implying_convergence() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "status",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();
    assert!(status.delivery_note.contains("best-effort"));
    assert!(status.bindings.is_empty());
}

#[tokio::test]
async fn status_polls_on_demand_so_its_signals_are_not_empty() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", None);
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "status",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();
    // No completed placement was ever journaled, so the sweep reports
    // exactly one `not-deployed` signal rather than an empty list --
    // it really ran, not merely echoed stored rows.
    assert_eq!(status.services.len(), 1);
    assert_eq!(status.services[0].signal, "not-deployed");
}

/// A revocation with nothing else changed used to be
/// invisible on `status` until some unrelated write reached the
/// member and raised `InstanceRevoked` inside `apply_with_clients`.
/// `revoked_placements` is a local table read, so it belongs on the
/// read surface directly, not gated behind a write pass ever
/// happening to touch this member again.
#[tokio::test]
async fn status_reports_a_revoked_placement_with_nothing_else_changed() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", None);
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
    s.store.revoke_placement("inst-1", "inst-1/backend#0", 1_000).unwrap();

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "status",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();

    assert_eq!(status.revoked_placements, vec!["inst-1/backend#0".to_string()]);
}

/// The journal keys every completed action row on a `MemberRef`, not a
/// bare `LogicalServiceRef` -- if `handle_status`'s own
/// expected-service builder (one of three, alongside the loop's sweep
/// and `roymctl`'s two) ever went back to reading it by the old key,
/// member 1's placement would silently stop matching and this service
/// would report `substrate_did` empty and land in `missing_placement`
/// even though it is fully landed. Scaled (index 1, not 0) on purpose:
/// an unscaled member's `MemberRef` string is unchanged from the
/// bare-ref era and would not catch a regression to the old key.
#[tokio::test]
async fn a_members_placement_is_found_after_the_journal_is_re_keyed() {
    let s = service();
    let plan_json = serde_json::json!({
        "app_instance_id": "inst-1",
        "blueprint_id": "syneroym:test",
        "version": "1.0.0",
        "services": [{
            "service_id": "did:key:hFabricated",
            "logical_ref": "inst-1/backend",
            "substrate": "edge-1",
            "service_type": "tcp", "source": "127.0.0.1:9000",
            "rotation_policy": "none",
            "resolved_dependencies": {},
            "topology_mode": "redundant",
            "member_index": 1
        }]
    })
    .to_string();
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    let deployment_id = s.store.journal.append(&plan, DeploymentState::Active).unwrap();
    s.store
        .journal
        .append_action(
            deployment_id,
            "ADD",
            "inst-1/backend#1",
            Some("edge-1"),
            "did:key:zEdge1",
            ActionState::Completed,
        )
        .unwrap();

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "status",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();

    assert_eq!(status.services.len(), 1, "{:?}", status.services);
    assert_eq!(status.services[0].logical_ref, "inst-1/backend#1");
    assert_eq!(
        status.services[0].substrate_did, "did:key:zEdge1",
        "member 1's completed placement must be found by its own MemberRef, not read as missing: \
         {:?}",
        status.services[0]
    );
    assert_ne!(
        status.services[0].signal, "instance-not-running",
        "a landed member must not be reported as never-deployed: {:?}",
        status.services[0]
    );
}

/// A re-submit that moves a landed service to a different substrate
/// must be refused before anything is deployed -- an early version
/// shipped `submit` with no such check, so this silently ran a second
/// live copy of the same member.
#[tokio::test]
async fn submit_is_refused_when_the_plan_moves_a_service_to_another_substrate() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    let deployment_id = s.store.journal.append(&plan, DeploymentState::Active).unwrap();
    s.store
        .journal
        .append_action(
            deployment_id,
            "ADD",
            "inst-1/backend#0",
            Some("edge-1"),
            "did:key:zEdge1",
            ActionState::Completed,
        )
        .unwrap();

    let moved_plan_json = plan_json_one_service("inst-1", "backend", Some("edge-2"));
    let inventory_json =
        serde_json::json!({"edge-2": {"did": "did:key:zEdge2", "api_url": "http://127.0.0.1:1"}})
            .to_string();

    let err = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "submit",
        serde_json::json!([{
            "app_instance_id": "inst-1",
            "plan_json": moved_plan_json,
            "inventory_json": inventory_json,
            "generation": 0,
        }]),
    )
    .await
    .unwrap_err();
    let err = err.to_string();
    assert!(err.contains("did:key:zEdge1") && err.contains("did:key:zEdge2"), "{err}");
}

/// Same fixture trick as `submit_against_a_retired_instance_is_refused_
/// before_any_deploy_work_runs`: the inventory carries no credential
/// for the new alias, so a "placement" failure (not a "credential"
/// one) proves the refusal runs before `deploy_submission`.
#[tokio::test]
async fn submit_with_a_changed_placement_is_refused_before_any_deploy_work_runs() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    let deployment_id = s.store.journal.append(&plan, DeploymentState::Active).unwrap();
    s.store
        .journal
        .append_action(
            deployment_id,
            "ADD",
            "inst-1/backend#0",
            Some("edge-1"),
            "did:key:zEdge1",
            ActionState::Completed,
        )
        .unwrap();

    let moved_plan_json = plan_json_one_service("inst-1", "backend", Some("edge-2"));
    // No credential for edge-2: if the placement refusal did not run
    // first, this would fail with "no credential" instead.
    let inventory_json = serde_json::json!({
        "edge-2": {"did": "did:key:zEdge2", "api_url": "http://127.0.0.1:1", "ucan": null}
    })
    .to_string();

    let err = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "submit",
        serde_json::json!([{
            "app_instance_id": "inst-1",
            "plan_json": moved_plan_json,
            "inventory_json": inventory_json,
            "generation": 0,
        }]),
    )
    .await
    .unwrap_err();
    let err = err.to_string();
    assert!(err.contains("did:key:zEdge1") && err.contains("did:key:zEdge2"), "{err}");
    assert!(!err.contains("credential"), "{err}");
}

/// `force-reconcile` never calls `store.submit`, so it needs its own
/// placement check -- without one
/// it would just keep redeploying the moved service indefinitely.
#[tokio::test]
async fn force_reconcile_is_refused_when_the_stored_plan_moves_a_service() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    let deployment_id = s.store.journal.append(&plan, DeploymentState::Active).unwrap();
    s.store
        .journal
        .append_action(
            deployment_id,
            "ADD",
            "inst-1/backend#0",
            Some("edge-1"),
            "did:key:zEdge1",
            ActionState::Completed,
        )
        .unwrap();

    let moved_plan_json = plan_json_one_service("inst-1", "backend", Some("edge-2"));
    let inventory_json =
        serde_json::json!({"edge-2": {"did": "did:key:zEdge2", "api_url": "http://127.0.0.1:1"}})
            .to_string();
    s.store.submit("inst-1", &moved_plan_json, &inventory_json, "did:key:owner", 0).unwrap();

    let err = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "force-reconcile",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap_err();
    let err = err.to_string();
    assert!(err.contains("did:key:zEdge1") && err.contains("did:key:zEdge2"), "{err}");
}

/// The boundary: a re-submit that keeps the same substrate must not
/// be caught by the placement refusal -- otherwise it refuses
/// everything, not just a real move.
#[tokio::test]
async fn submit_is_allowed_when_a_service_keeps_its_substrate() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    let deployment_id = s.store.journal.append(&plan, DeploymentState::Active).unwrap();
    s.store
        .journal
        .append_action(
            deployment_id,
            "ADD",
            "inst-1/backend#0",
            Some("edge-1"),
            "did:key:zEdge1",
            ActionState::Completed,
        )
        .unwrap();

    // Same alias, no credential -- so a run past the placement check
    // must fail later, at the credential gate, not be refused for
    // "placement".
    let inventory_json = serde_json::json!({
        "edge-1": {"did": "did:key:zEdge1", "api_url": "http://127.0.0.1:1", "ucan": null}
    })
    .to_string();

    let err = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "submit",
        serde_json::json!([{
            "app_instance_id": "inst-1",
            "plan_json": plan_json,
            "inventory_json": inventory_json,
            "generation": 0,
        }]),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("no credential"), "{}", err);
}

/// A planned service the journal has never recorded landed must report
/// the instance `Degraded`, not `Active` -- an earlier gap, since
/// `Signal::NotDeployed` is deliberately not a fault.
#[tokio::test]
async fn an_instance_with_a_planned_service_that_never_landed_reports_degraded() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", None);
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "status",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();
    assert!(matches!(status.state, ManagedState::Degraded), "{:?}", status.state);
}

/// Row 12's boundary: an instance whose only service is fully landed
/// and healthy must still report `Active` -- the fix above must not
/// degrade every instance.
#[tokio::test]
async fn a_fully_landed_healthy_instance_still_reports_active() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    let deployment_id = s.store.journal.append(&plan, DeploymentState::Active).unwrap();
    s.store
        .journal
        .append_action(
            deployment_id,
            "ADD",
            "inst-1/backend#0",
            Some("edge-1"),
            "did:key:zEdge1",
            ActionState::Completed,
        )
        .unwrap();
    // No inventory entry for edge-1: `poll_once` then reports
    // `Unknown` (no health target built), not a fault, so the
    // instance still reads `Active` -- exactly today's (A5b) behavior
    // for an unreachable target, unaffected by this fix.
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "status",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();
    assert!(matches!(status.state, ManagedState::Active), "{:?}", status.state);
}

/// D-A5e-8, ADR-0021 §5: the same fully-landed, otherwise-healthy
/// instance above must report `Degraded`, not `Active`, once one of
/// its dependents has an active `BindingConflict` -- a binding push
/// that has been attempted and did not land.
#[tokio::test]
async fn a_fully_landed_instance_with_an_active_binding_conflict_reports_degraded() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    let deployment_id = s.store.journal.append(&plan, DeploymentState::Active).unwrap();
    s.store
        .journal
        .append_action(
            deployment_id,
            "ADD",
            "inst-1/backend#0",
            Some("edge-1"),
            "did:key:zEdge1",
            ActionState::Completed,
        )
        .unwrap();
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
    s.store
        .alerts
        .raise(
            &AppInstanceId::new("inst-1"),
            Some("inst-1/backend#0"),
            None,
            "did:key:zEdge1",
            AlertKind::BindingConflict,
            "a binding push did not land cleanly after one retry",
        )
        .unwrap();

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "status",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();
    assert!(matches!(status.state, ManagedState::Degraded), "{:?}", status.state);
}

/// A reconcile in flight is now observable -- `apply_with_clients`
/// writes `Applying` before it
/// writes `Active`/`Degraded`, and `status` landing mid-pass must
/// read it rather than guessing from a half-applied plan's health.
#[tokio::test]
async fn status_reports_applying_while_a_reconcile_is_in_flight() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    s.store.journal.append(&plan, DeploymentState::Applying).unwrap();
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "status",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();
    assert!(matches!(status.state, ManagedState::Applying), "{:?}", status.state);
}

/// The whole point of the fix is that an alias serving double duty --
/// both a landed placement's alias and
/// the plan's own declared placement -- is connected to once, not
/// twice. Tested at the dedup itself, which is directly and
/// deterministically testable with no live substrate; the RPC-level
/// behavior (one client set shared by the sweep and the generation
/// read) has no network-free way to observe a connection count
/// through the public `status` call, since `SyneroymClient` connects
/// for real rather than through an injectable fake.
#[test]
fn status_connects_to_each_substrate_once() {
    let plan_aliases: BTreeSet<String> = ["edge-1".to_string()].into_iter().collect();
    let did_to_alias: BTreeMap<String, String> =
        BTreeMap::from([("did:key:zEdge1".to_string(), "edge-1".to_string())]);

    let aliases = SupervisorService::connect_aliases_for_pass(&plan_aliases, &did_to_alias);
    assert_eq!(aliases, vec!["edge-1".to_string()]);
}

/// Review finding A-1: the whole fix in one assertion. `svc-a` is
/// outside `needs_work` (already landed, unchanged) and `svc-b` is
/// inside it and reachable this pass -- both must survive into the
/// record. Before this fix, `record_plan_for_pass` did not exist and
/// the filtered (`needs_work`-only) plan was journaled directly,
/// which is what `svc-a` dropping out of this assertion would
/// reproduce.
#[test]
fn record_plan_for_pass_keeps_untouched_services_alongside_this_passs_subset() {
    let plan_json = serde_json::json!({
        "app_instance_id": "inst-1",
        "blueprint_id": "syneroym:test",
        "version": "1.0.0",
        "services": [
            {
                "service_id": "did:key:hSvcA",
                "logical_ref": "inst-1/svc-a",
                "substrate": "edge-1",
                "service_type": "tcp", "source": "127.0.0.1:9000",
                "rotation_policy": "none",
                "resolved_dependencies": {},
                "topology_mode": "singleton"
            },
            {
                "service_id": "did:key:hSvcB",
                "logical_ref": "inst-1/svc-b",
                "substrate": "edge-2",
                "service_type": "tcp", "source": "127.0.0.1:9001",
                "rotation_policy": "none",
                "resolved_dependencies": {},
                "topology_mode": "singleton"
            }
        ]
    })
    .to_string();
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    let needs_work: BTreeSet<String> = ["inst-1/svc-b#0".to_string()].into_iter().collect();
    let identity = Identity::generate().unwrap();
    let client = Arc::new(SyneroymClient::new_with_identity(
        "did:key:zEdge2".to_string(),
        String::new(),
        identity,
    ));
    let clients: BTreeMap<SubstrateAlias, Arc<SyneroymClient>> =
        BTreeMap::from([(SubstrateAlias::new("edge-2"), client)]);

    let record_plan = SupervisorService::record_plan_for_pass(&plan, &needs_work, &clients);

    let refs: BTreeSet<String> =
        record_plan.services.iter().map(|s| s.logical_ref.to_string()).collect();
    assert_eq!(
        refs,
        BTreeSet::from(["inst-1/svc-a".to_string(), "inst-1/svc-b".to_string()]),
        "svc-a (untouched) and svc-b (this pass's subset) must both survive"
    );
}

/// The other half: a `needs_work` service whose substrate this pass
/// never reached has not landed and must not be recorded as if it
/// had -- recording it would make a later pass believe it is already
/// active and never retry it.
#[test]
fn record_plan_for_pass_drops_a_needs_work_service_still_unreachable_this_pass() {
    let plan_json = serde_json::json!({
        "app_instance_id": "inst-1",
        "blueprint_id": "syneroym:test",
        "version": "1.0.0",
        "services": [{
            "service_id": "did:key:hSvcB",
            "logical_ref": "inst-1/svc-b",
            "substrate": "edge-2",
            "service_type": "tcp", "source": "127.0.0.1:9001",
            "rotation_policy": "none",
            "resolved_dependencies": {},
            "topology_mode": "singleton"
        }]
    })
    .to_string();
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    let needs_work: BTreeSet<String> = ["inst-1/svc-b#0".to_string()].into_iter().collect();

    let record_plan = SupervisorService::record_plan_for_pass(&plan, &needs_work, &BTreeMap::new());

    assert!(record_plan.services.is_empty(), "an unreachable needs_work service must not land");
}

/// A sweep that opens a new alert publishes it under the supervisor's
/// own topic -- `<alert_topic>/<app_instance_id>`,
/// namespaced with the publish-side rule under
/// `SUPERVISOR_RESERVED_SERVICE_ID`, the exact string the router's own
/// subscribe-side fix (`dispatch.rs::subscribe_namespaced_topic`)
/// produces for the same service id.
#[tokio::test]
async fn a_newly_opened_alert_is_published_under_the_supervisors_own_topic() {
    let s = service();
    // No substrate placement: the sweep reports `not-deployed`, which
    // becomes a raised `InstanceNotRunning` alert -- the cheapest
    // fixture that opens a real alert with no live substrate.
    let plan_json = plan_json_one_service("inst-1", "backend", None);
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

    let topic = expected_alert_topic("inst-1");
    let (_handle, mut receiver) = s.messaging_broker.subscribe(topic.clone()).await.unwrap();

    dispatch(&s, admin_caller("did:key:zSupervisorNode"), "status", serde_json::json!(["inst-1"]))
        .await
        .unwrap();

    let (received_topic, payload) = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .expect("did not time out waiting for the published alert")
        .expect("broker channel closed");
    assert_eq!(received_topic, topic);
    let value: Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(value["app_instance_id"], "inst-1");
    assert_eq!(value["kind"], AlertKind::InstanceNotRunning.to_string());
}

/// `publish_opened_alerts` returns `()`, not a `Result` -- there is no
/// `?` for a publish failure to propagate through, by
/// construction. This is the observable half of that guarantee: the
/// `status` call succeeds and the alert is stored and readable
/// through `alerts`, regardless of what publication itself did.
#[tokio::test]
async fn a_publish_failure_leaves_the_alert_stored_and_the_pass_running() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", None);
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "status",
        serde_json::json!(["inst-1"]),
    )
    .await;
    assert!(res.is_ok(), "the status call must succeed even if MQTT publication does not");

    let instance_id = AppInstanceId::new("inst-1");
    let active = s.store.alerts.active(&instance_id).unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].kind, AlertKind::InstanceNotRunning);
}

#[test]
fn refuse_unshardable_plan_refuses_a_strategy_over_a_single_member() {
    let mut svc = dependent_service("backend", "unrelated");
    svc.topology_mode = TopologyMode::Sharded;
    svc.sharding_strategy = Some(ShardingStrategy::HashSharding);
    let plan = DeploymentPlan {
        app_instance_id: AppInstanceId::new("inst-1"),
        blueprint_id: AppBlueprintId::new("syneroym:test"),
        version: semver::Version::new(1, 0, 0),
        services: vec![svc],
    };
    let err = SupervisorService::refuse_unshardable_plan(&plan).unwrap_err();
    assert!(err.contains("one member"), "{err}");
}

#[test]
fn refuse_unshardable_plan_allows_a_plan_with_no_strategy() {
    let svc = dependent_service("backend", "unrelated");
    let plan = DeploymentPlan {
        app_instance_id: AppInstanceId::new("inst-1"),
        blueprint_id: AppBlueprintId::new("syneroym:test"),
        version: semver::Version::new(1, 0, 0),
        services: vec![svc],
    };
    assert!(SupervisorService::refuse_unshardable_plan(&plan).is_ok());
}

/// The refusal runs beside its two siblings, ahead of `store.submit`,
/// so nothing durable is written -- the test that makes the WIT's
/// `option<string>` a checked property rather than a comment.
#[tokio::test]
async fn a_submitted_plan_declaring_range_sharding_is_refused_before_anything_is_stored() {
    let s = service();
    let plan_json = serde_json::json!({
        "app_instance_id": "inst-1",
        "blueprint_id": "syneroym:test",
        "version": "1.0.0",
        "services": [{
            "service_id": "did:key:hFabricated0",
            "logical_ref": "inst-1/backend",
            "substrate": "edge-1",
            "service_type": "tcp", "source": "127.0.0.1:9000",
            "rotation_policy": "none",
            "resolved_dependencies": {},
            "topology_mode": "sharded",
            "sharding_strategy": {"range_sharding": {"chunks": [
                {"start_key": null, "end_key": null, "target": "did:key:hShard0"}
            ]}},
        }, {
            "service_id": "did:key:hFabricated1",
            "logical_ref": "inst-1/backend",
            "substrate": "edge-1",
            "service_type": "tcp", "source": "127.0.0.1:9000",
            "rotation_policy": "none",
            "resolved_dependencies": {},
            "topology_mode": "sharded",
            "member_index": 1,
            "sharding_strategy": {"range_sharding": {"chunks": [
                {"start_key": null, "end_key": null, "target": "did:key:hShard0"}
            ]}},
        }],
    })
    .to_string();

    let err = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "submit",
        serde_json::json!([{
            "app_instance_id": "inst-1",
            "plan_json": plan_json,
            "inventory_json": "{}",
            "generation": 0,
        }]),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("range_sharding"), "{err}");
    assert!(s.store.get("inst-1").unwrap().is_none(), "nothing must be stored on refusal");
}
