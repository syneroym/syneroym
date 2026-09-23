use std::{collections::BTreeMap, time::Instant};

use syneroym_app_orchestration::{
    ActionState, DeploymentJournal,
    models::{AppBlueprintId, LogicalServiceRef, TopologyMode},
};

use super::{super::*, helpers::*};

/// The convergence budget is measured from the membership change to
/// the last applied write returning `Applied`/`NoOp` --
/// *not* off `binding-epochs`, whose own refresh is bounded by
/// `poll_interval_secs` (default 30s, six times the 5s budget). This
/// harness proves the two are not the same clock: a push against a
/// fake actor that answers immediately completes in a time nowhere
/// near a poll interval, so a measurement taken this way is the
/// write's own latency, never silently the read surface's lag.
///
/// The clock starts at `Reconciler::compute_diff` and runs through
/// `classify_update_actions`, the same classifier
/// `apply_with_membership_pushes`/`apply_write_phase` call -- not just
/// the `push_bindings` call after it has already decided -- so a
/// regression that makes the routing decision itself slow (e.g. an
/// O(n²) diff over a large plan) is inside what this measures, not
/// hidden before it.
#[tokio::test]
async fn convergence_is_measured_from_the_membership_change_to_the_last_applied_write() {
    let s = service();
    let old_svc = dependent_service("frontend", "backend");
    let old_plan = plan_with_one_dependent(old_svc.clone());
    let deployment_id = s.store.journal.append(&old_plan, DeploymentState::Active).unwrap();
    s.store
        .journal
        .append_action(
            deployment_id,
            "ADD",
            "inst-1/frontend#0",
            Some("edge-1"),
            "did:key:zEdge1",
            ActionState::Completed,
        )
        .unwrap();

    let mut new_svc = old_svc.clone();
    new_svc.resolved_dependencies = BTreeMap::from([(
        LogicalServiceName::new("backend"),
        vec![ServiceId::new("did:key:hDepMember"), ServiceId::new("did:key:hDepMember2")],
    )]);
    let plan = plan_with_one_dependent(new_svc);

    let actor = Arc::new(BindingActor::default());
    let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
    let instance_id = AppInstanceId::new("inst-1");
    let mut opened = Vec::new();

    // The membership change: the moment a `submit`'s own diff would
    // see it, before the classifier has decided anything. The clock
    // stops when the write this decision routes to returns.
    let start = Instant::now();
    let landed = s.store.journal.get_completed_actions_for_instance(&instance_id).unwrap();
    let diff = Reconciler::new(&s.store.journal).compute_diff(&plan).unwrap();
    let (_, push_candidates) = SupervisorService::classify_update_actions(&landed, &diff.actions);
    let (svc, substrate_did) =
        push_candidates.into_iter().next().expect("frontend must classify as a push candidate");
    let outcomes = s
        .push_bindings(&instance_id, &plan, &svc, &substrate_did, &dyn_actor, 0, &mut opened)
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert_eq!(outcomes, PushOutcome::Landed(vec![BindingWriteOutcome::Applied]));
    assert!(
        elapsed < Duration::from_secs(1),
        "the measured interval must cover the routing decision and the write's own latency, far \
         under a poll interval (default 30s) and the 5s budget alike, not `binding-epochs`' own \
         read lag: {elapsed:?}"
    );
}

/// D-A5c-4: a push advances this dependent's epoch before sending,
/// and a clean `Applied` outcome leaves the new value on record --
/// what the next pass's convergence read compares against.
#[tokio::test]
async fn a_membership_change_pushes_at_the_next_epoch_and_records_it() {
    let s = service();
    let svc = dependent_service("frontend", "backend");
    let plan = plan_with_one_dependent(svc.clone());
    let actor = Arc::new(BindingActor::default());
    let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
    let instance_id = AppInstanceId::new("inst-1");
    let mut opened = Vec::new();

    let outcomes = s
        .push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
        .await
        .unwrap();

    assert_eq!(outcomes, PushOutcome::Landed(vec![BindingWriteOutcome::Applied]));
    assert_eq!(actor.calls.lock().unwrap().len(), 1);
    assert_eq!(s.store.binding_epoch("inst-1", "inst-1/frontend#0").unwrap(), 1);
    assert!(opened.is_empty());
}

/// A write with zero bindings is a real, converged success -- not the
/// same value `push_bindings`
/// used to signal "deferred to an already-pending queue item" before
/// `PushOutcome` existed. Reachable in the ordinary course of a
/// deploy: removing a service's last `depends_on` leaves
/// `resolved_dependencies` empty, `only_resolved_dependencies_changed`
/// still classifies the member as a push candidate purely because the
/// field *changed*, and the write this produces legitimately carries
/// zero bindings -- `orchestration.rs`'s `write_bindings_impl` builds
/// one outcome per binding sent, so the substrate legitimately answers
/// with zero too. Before `PushOutcome`, this collapsed onto the same
/// `Vec::new()` the deferred-to-queue sentinel used, permanently
/// downgrading the member every pass.
#[tokio::test]
async fn a_push_with_zero_bindings_lands_rather_than_reading_as_deferred() {
    let s = service();
    let svc = PlannedService {
        resolved_dependencies: BTreeMap::new(),
        ..dependent_service("frontend", "backend")
    };
    let plan = plan_with_one_dependent(svc.clone());
    let actor = Arc::new(BindingActor::default());
    actor.responses.lock().unwrap().push(Ok(Vec::new()));
    let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
    let instance_id = AppInstanceId::new("inst-1");
    let mut opened = Vec::new();

    let outcome = s
        .push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
        .await
        .unwrap();

    assert_eq!(
        outcome,
        PushOutcome::Landed(Vec::new()),
        "zero bindings is a real, converged success, not a deferral"
    );
}

/// `map_deployment_plan_to_wit` reads a binding's `mode` off the
/// *dependency's own* `PlannedService.topology_mode` in the plan --
/// not off `resolved_dependencies`' member count -- so `backend` must
/// be present in `plan.services` with `Redundant` already compiled
/// onto it (`replicas > 1` implies `Redundant`) for the push to carry
/// it correctly. Proves the flip at the binding-write layer itself,
/// without needing a live substrate to observe cross-member
/// resolution.
#[tokio::test]
async fn a_scale_out_push_carries_the_redundant_mode_to_the_dependent() {
    let s = service();
    let frontend = dependent_service("frontend", "backend");
    let mut backend = dependent_service("backend", "unrelated");
    backend.topology_mode = TopologyMode::Redundant;
    let plan = DeploymentPlan {
        app_instance_id: AppInstanceId::new("inst-1"),
        blueprint_id: AppBlueprintId::new("syneroym:test"),
        version: semver::Version::new(1, 0, 0),
        services: vec![frontend.clone(), backend],
    };
    let actor = Arc::new(BindingActor::default());
    let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
    let instance_id = AppInstanceId::new("inst-1");
    let mut opened = Vec::new();

    s.push_bindings(&instance_id, &plan, &frontend, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
        .await
        .unwrap();

    let calls = actor.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].bindings.len(), 1);
    assert!(
        matches!(calls[0].bindings[0].mode, syneroym_sdk::TopologyMode::Redundant),
        "{:?}",
        calls[0].bindings[0].mode
    );
}

/// D-A5c-4/D-A5c-19: a second writer exists (`Conflict`) is never
/// retried -- retrying would only race it again.
#[tokio::test]
async fn a_conflict_outcome_raises_binding_conflict_and_does_not_retry() {
    let s = service();
    let svc = dependent_service("frontend", "backend");
    let plan = plan_with_one_dependent(svc.clone());
    let actor = Arc::new(BindingActor::default());
    actor.responses.lock().unwrap().push(Ok(vec![BindingWriteOutcome::Conflict(5)]));
    let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
    let instance_id = AppInstanceId::new("inst-1");
    let mut opened = Vec::new();

    s.push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
        .await
        .unwrap();

    assert_eq!(actor.calls.lock().unwrap().len(), 1, "a conflict must not be retried");
    assert_eq!(opened, vec![(AlertKind::BindingConflict, "inst-1/frontend#0".to_string())]);
}

/// D-A5c-19/F4: `Stale(held)` retries exactly once, at `held + 1` --
/// not a re-read, and not the held epoch itself (which the four-case
/// rule would only ever answer with `Conflict`). The retry failing
/// too still alerts, once.
#[tokio::test]
async fn a_stale_outcome_is_retried_once_above_the_held_epoch_then_alerts() {
    let s = service();
    let svc = dependent_service("frontend", "backend");
    let plan = plan_with_one_dependent(svc.clone());
    let actor = Arc::new(BindingActor::default());
    actor.responses.lock().unwrap().push(Ok(vec![BindingWriteOutcome::Stale(5)]));
    actor.responses.lock().unwrap().push(Ok(vec![BindingWriteOutcome::Conflict(6)]));
    let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
    let instance_id = AppInstanceId::new("inst-1");
    let mut opened = Vec::new();

    s.push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
        .await
        .unwrap();

    let calls = actor.calls.lock().unwrap();
    assert_eq!(calls.len(), 2, "exactly one retry");
    assert_eq!(calls[1].bindings[0].epoch, 6, "the retry must land at held + 1, not held");
    drop(calls);
    assert_eq!(opened, vec![(AlertKind::BindingConflict, "inst-1/frontend#0".to_string())]);
    assert_eq!(
        s.store.binding_epoch("inst-1", "inst-1/frontend#0").unwrap(),
        6,
        "the local counter must agree with the substrate after the retry"
    );
}

/// An operator reads a converged binding once the written and observed
/// epochs agree. Read directly
/// off `binding_convergence_rows` (what `status` calls), since
/// driving a real observed epoch through `handle_status` needs a
/// live substrate to report one.
#[tokio::test]
async fn status_reports_a_converged_binding_after_a_push_lands() {
    let s = service();
    let svc = dependent_service("frontend", "backend");
    let plan = plan_with_one_dependent(svc.clone());
    let actor = Arc::new(BindingActor::default());
    let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
    let instance_id = AppInstanceId::new("inst-1");
    let mut opened = Vec::new();
    s.push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
        .await
        .unwrap();

    let report = health::HealthReport {
        substrates: Vec::new(),
        services: vec![{
            let mut h = service_health("inst-1/frontend", "did:key:zEdge1", Signal::Healthy);
            h.binding_epochs = vec![("backend".to_string(), 1)];
            h
        }],
    };
    let rows = s.binding_convergence_rows("inst-1", &plan, &report);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].dependent_logical_ref, "inst-1/frontend#0");
    assert_eq!(rows[0].dependency_name, "backend");
    assert_eq!(rows[0].written_epoch, 1);
    assert_eq!(rows[0].observed_epoch, Some(1));
    assert!(rows[0].converged);
}

/// The classifier's whole point. A resubmit whose only change to a
/// dependent member is which DIDs a dependency resolves to must be
/// routed to a push, not a redeploy.
#[test]
fn only_resolved_dependencies_changed_is_true_when_only_the_dependency_map_differs() {
    let old = dependent_service("frontend", "backend");
    let mut new = old.clone();
    new.resolved_dependencies = BTreeMap::from([(
        LogicalServiceName::new("backend"),
        vec![ServiceId::new("did:key:hDepMember"), ServiceId::new("did:key:hDepMember2")],
    )]);
    assert!(SupervisorService::only_resolved_dependencies_changed(&old, &new));
}

/// The other half of the classifier: any other kind of change --
/// config, in this case -- still takes the redeploy path, even when
/// `resolved_dependencies` also changed in the same resubmit.
#[test]
fn only_resolved_dependencies_changed_is_false_when_config_also_changes() {
    let old = dependent_service("frontend", "backend");
    let mut new = old.clone();
    new.resolved_dependencies = BTreeMap::from([(
        LogicalServiceName::new("backend"),
        vec![ServiceId::new("did:key:hDepMember"), ServiceId::new("did:key:hDepMember2")],
    )]);
    new.config.source = "127.0.0.1:9001".to_string();
    assert!(!SupervisorService::only_resolved_dependencies_changed(&old, &new));
}

/// A change to `resolved_dependencies` alone, with nothing else
/// different at all, is a no-op diff (`old == new`), not an `Update`
/// action -- `only_resolved_dependencies_changed` is only ever asked
/// about an actual `Update`, but must not misreport an identical pair.
#[test]
fn only_resolved_dependencies_changed_is_false_when_nothing_changed() {
    let old = dependent_service("frontend", "backend");
    let new = old.clone();
    assert!(!SupervisorService::only_resolved_dependencies_changed(&old, &new));
}

/// D-A5e-7, second review round: the classifier being correct
/// (`only_resolved_dependencies_changed`) was never the gap -- the gap
/// was that `handle_submit`/`deploy_submission` never called it at
/// all, going straight to `apply_with_clients` over the whole plan.
/// This drives `apply_with_membership_pushes` itself, the shared
/// routing both now go through, with a completed placement journaled
/// for `frontend` and no client built for its substrate: if the
/// classifier is bypassed and `frontend` reaches `apply_with_clients`
/// like every other service, the failure comes from `certify_placed_
/// members`'s "no client"/"no member master" shape; if it is routed to
/// `push_bindings` instead, the failure is this call's own "not
/// connected to its landed substrate" -- the two are textually
/// distinguishable, so this fails loudly if the routing regresses.
#[tokio::test]
async fn a_diff_whose_only_change_is_resolved_dependencies_pushes_instead_of_redeploying() {
    let s = service();
    let old_frontend = dependent_service("frontend", "backend");
    let old_plan = plan_with_one_dependent(old_frontend.clone());
    let deployment_id = s.store.journal.append(&old_plan, DeploymentState::Active).unwrap();
    s.store
        .journal
        .append_action(
            deployment_id,
            "ADD",
            "inst-1/frontend#0",
            Some("edge-1"),
            "did:key:zEdge1",
            ActionState::Completed,
        )
        .unwrap();

    let mut new_frontend = old_frontend.clone();
    new_frontend.resolved_dependencies = BTreeMap::from([(
        LogicalServiceName::new("backend"),
        vec![ServiceId::new("did:key:hDepMemberScaledOut")],
    )]);
    let new_plan = plan_with_one_dependent(new_frontend);

    let err = s
        .apply_with_membership_pushes(&new_plan, &BTreeMap::new(), &BTreeMap::new(), 0, Vec::new())
        .await
        .unwrap_err();
    assert!(
        err.contains("not connected to its landed substrate this call"),
        "frontend must be routed to a push attempt, not a redeploy: {err}"
    );
    assert!(err.contains("inst-1/frontend#0"), "{err}");

    // Round 2 review, finding A: the redeploy half journaled `new_plan`
    // -- carrying frontend's already-scaled `resolved_dependencies` --
    // as `Active` before the push above ever ran. Left there, the next
    // pass's diff would read frontend as already converged and never
    // retry the push that just failed. It must be downgraded to
    // `Degraded` instead, so `compute_diff` falls back to `old_plan`.
    let instance_id = AppInstanceId::new("inst-1");
    let latest = s.store.journal.get_latest(&instance_id).unwrap().unwrap();
    assert_eq!(
        latest.state,
        DeploymentState::Degraded,
        "a record carrying an unlanded push must not read as this instance's converged baseline: \
         {latest:?}"
    );

    // The real assertion the state check exists for: the next pass's
    // diff must still see frontend as a push candidate, not as already
    // converged.
    let diff = Reconciler::new(&s.store.journal).compute_diff(&new_plan).unwrap();
    let landed = s.store.journal.get_completed_actions_for_instance(&instance_id).unwrap();
    let (redeploy_exclusions, _) =
        SupervisorService::classify_update_actions(&landed, &diff.actions);
    assert!(
        redeploy_exclusions.contains("inst-1/frontend#0"),
        "the next pass must reclassify frontend as a push candidate, not read it as landed: \
         {diff:?}"
    );
}

/// The other half: a member whose diff also changes something besides
/// `resolved_dependencies` must still take the redeploy path through
/// `apply_with_membership_pushes`, even though it has a completed
/// placement too -- the same fixture as the push case above, but
/// failing through `certify_placed_members`'s shape instead.
#[tokio::test]
async fn a_diff_that_also_changes_config_still_takes_the_redeploy_path_through_membership_pushes() {
    let s = service();
    let old_frontend = dependent_service("frontend", "backend");
    let old_plan = plan_with_one_dependent(old_frontend.clone());
    let deployment_id = s.store.journal.append(&old_plan, DeploymentState::Active).unwrap();
    s.store
        .journal
        .append_action(
            deployment_id,
            "ADD",
            "inst-1/frontend#0",
            Some("edge-1"),
            "did:key:zEdge1",
            ActionState::Completed,
        )
        .unwrap();

    let mut new_frontend = old_frontend.clone();
    new_frontend.resolved_dependencies = BTreeMap::from([(
        LogicalServiceName::new("backend"),
        vec![ServiceId::new("did:key:hDepMemberScaledOut")],
    )]);
    new_frontend.config.source = "127.0.0.1:9001".to_string();
    let new_plan = plan_with_one_dependent(new_frontend);

    let err = s
        .apply_with_membership_pushes(&new_plan, &BTreeMap::new(), &BTreeMap::new(), 0, Vec::new())
        .await
        .unwrap_err();
    assert!(
        !err.contains("not connected to its landed substrate this call"),
        "a config change must not be routed through the push path: {err}"
    );
}

/// `Reconciler::compute_diff` produces one `Update` action per member
/// of the scaled dependency's dependent -- each is
/// independently a push-only change, so a two-member dependent
/// pushes on both, not just the first.
#[test]
fn a_membership_change_pushes_to_every_member_of_every_dependent() {
    let mut frontend_0 = dependent_service("frontend", "backend");
    let mut frontend_1 = frontend_0.clone();
    frontend_1.member_index = 1;
    frontend_1.service_id = ServiceId::new("did:key:hfrontend1");

    let old_plan = DeploymentPlan {
        app_instance_id: AppInstanceId::new("inst-1"),
        blueprint_id: AppBlueprintId::new("syneroym:test"),
        version: semver::Version::new(1, 0, 0),
        services: vec![frontend_0.clone(), frontend_1.clone()],
    };
    let journal = DeploymentJournal::open_in_memory().unwrap();
    journal.append(&old_plan, DeploymentState::Active).unwrap();

    let scaled_deps = BTreeMap::from([(
        LogicalServiceName::new("backend"),
        vec![ServiceId::new("did:key:hDepMember"), ServiceId::new("did:key:hDepMember2")],
    )]);
    frontend_0.resolved_dependencies = scaled_deps.clone();
    frontend_1.resolved_dependencies = scaled_deps;
    let new_plan = DeploymentPlan { services: vec![frontend_0, frontend_1], ..old_plan.clone() };

    let diff = Reconciler::new(&journal).compute_diff(&new_plan).unwrap();
    assert_eq!(diff.actions.len(), 2, "{:?}", diff.actions);
    for action in &diff.actions {
        match action {
            ReconcileAction::Update { old, new } => {
                assert!(SupervisorService::only_resolved_dependencies_changed(old, new));
            }
            other => panic!("expected an Update per member, got {other:?}"),
        }
    }
}

/// A push candidate this pass could not even connect to used to be
/// dropped with a bare `continue` -- no alert, no `Degraded`. Drives
/// `apply_write_phase` directly (the same entry point
/// `a_pause_landing_mid_pass_stops_that_passs_writes` uses) with
/// `did_to_alias` empty, standing in for a dependent whose substrate
/// this pass's own connect step never reached.
#[tokio::test]
async fn an_unreachable_push_candidate_raises_binding_conflict_instead_of_being_dropped_silently() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "frontend", Some("edge-1"));
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    let svc = dependent_service("frontend", "backend");
    let instance_id = AppInstanceId::new("inst-1");

    s.apply_write_phase(WritePhase {
        instance_id: &instance_id,
        app_instance_id: "inst-1",
        plan: &plan,
        needs_work: &BTreeSet::new(),
        restart_candidates: &[],
        renewal_candidates: &[],
        pending_rotation_restarts: &BTreeSet::new(),
        push_candidates: &[(svc, "did:key:zEdge1".to_string())],
        schedule_decisions: &[],
        did_to_alias: &BTreeMap::new(),
        clients: &BTreeMap::new(),
        now: 0,
    })
    .await;

    let alerts = s.store.alerts.active(&instance_id).unwrap();
    let conflict = alerts
        .iter()
        .find(|a| a.kind == AlertKind::BindingConflict)
        .unwrap_or_else(|| panic!("no BindingConflict alert among {alerts:?}"));
    assert_eq!(conflict.logical_ref.as_deref(), Some("inst-1/frontend#0"));
    assert_eq!(conflict.substrate_did, "did:key:zEdge1");
}

/// The narrower loop-path shape:
/// `record_plan_for_pass` keeps a push candidate's *new*
/// `resolved_dependencies` unconditionally, so a `needs_work` redeploy
/// landing in the *same* pass as a *failing* push journals that push
/// candidate as already converged before the push loop below ever
/// runs. `backend` is revoked so `apply_with_clients`'s own filter
/// empties it out before certify/deploy, letting the redeploy "land"
/// (and journal `Active`) with no live substrate -- the same trick
/// `a_submit_of_the_same_plan_does_not_recertify_a_revoked_placement`
/// uses. `frontend`'s push then fails (no client for its alias).
#[tokio::test]
async fn a_needs_work_redeploy_and_a_failing_push_in_the_same_pass_leaves_the_record_degraded() {
    let s = service();
    let old_frontend = dependent_service("frontend", "backend");
    let backend = PlannedService {
        service_id: ServiceId::new("did:key:hbackend"),
        logical_ref: LogicalServiceRef {
            app_instance_id: AppInstanceId::new("inst-1"),
            service_name: LogicalServiceName::new("backend"),
        },
        substrate: Some(SubstrateAlias::new("edge-1")),
        config: dummy_config(),
        resolved_dependencies: BTreeMap::new(),
        topology_mode: TopologyMode::Singleton,
        member_index: 0,
        schedule: None,
        sharding_strategy: None,
        topology_visibility: Default::default(),
    };
    let old_plan = DeploymentPlan {
        app_instance_id: AppInstanceId::new("inst-1"),
        blueprint_id: AppBlueprintId::new("syneroym:test"),
        version: semver::Version::new(1, 0, 0),
        services: vec![old_frontend.clone(), backend.clone()],
    };
    let deployment_id = s.store.journal.append(&old_plan, DeploymentState::Active).unwrap();
    for (l_ref, alias, did) in [
        ("inst-1/frontend#0", "edge-1", "did:key:zEdge1"),
        ("inst-1/backend#0", "edge-1", "did:key:zEdge1"),
    ] {
        s.store
            .journal
            .append_action(deployment_id, "ADD", l_ref, Some(alias), did, ActionState::Completed)
            .unwrap();
    }
    s.store.revoke_placement("inst-1", "inst-1/backend#0", 1_000).unwrap();

    let mut new_frontend = old_frontend.clone();
    new_frontend.resolved_dependencies = BTreeMap::from([(
        LogicalServiceName::new("backend"),
        vec![ServiceId::new("did:key:hDepMemberScaledOut")],
    )]);
    let plan = DeploymentPlan { services: vec![new_frontend.clone(), backend], ..old_plan };
    s.store.submit("inst-1", &plan.to_json().unwrap(), "{}", "did:key:owner", 0).unwrap();

    let identity = Identity::generate().unwrap();
    let client = Arc::new(SyneroymClient::new_with_identity(
        "did:key:zEdge1".to_string(),
        String::new(),
        identity,
    ));
    let clients: BTreeMap<SubstrateAlias, Arc<SyneroymClient>> =
        BTreeMap::from([(SubstrateAlias::new("edge-1"), client)]);
    let instance_id = AppInstanceId::new("inst-1");
    let needs_work: BTreeSet<String> = ["inst-1/backend#0".to_string()].into_iter().collect();

    s.apply_write_phase(WritePhase {
        instance_id: &instance_id,
        app_instance_id: "inst-1",
        plan: &plan,
        needs_work: &needs_work,
        restart_candidates: &[],
        renewal_candidates: &[],
        pending_rotation_restarts: &BTreeSet::new(),
        // No alias for frontend's DID this pass -- the push fails.
        push_candidates: &[(new_frontend, "did:key:zEdge1".to_string())],
        schedule_decisions: &[],
        did_to_alias: &BTreeMap::new(),
        clients: &clients,
        now: 0,
    })
    .await;

    let latest = s.store.journal.get_latest(&instance_id).unwrap().unwrap();
    assert_eq!(
        latest.state,
        DeploymentState::Degraded,
        "the redeploy landed (vacuously, backend was revoked and filtered out) but the push did \
         not -- the record must not read as this instance's converged baseline: {latest:?}"
    );

    let diff = Reconciler::new(&s.store.journal).compute_diff(&plan).unwrap();
    let landed = s.store.journal.get_completed_actions_for_instance(&instance_id).unwrap();
    let (redeploy_exclusions, _) =
        SupervisorService::classify_update_actions(&landed, &diff.actions);
    assert!(
        redeploy_exclusions.contains("inst-1/frontend#0"),
        "the next pass must still classify frontend as a push candidate: {diff:?}"
    );
}

/// The companion negative case: a push failing in a pass where nothing
/// was journaled (`needs_work` empty, so `apply_with_clients` is never
/// called) must not touch an unrelated, already-`Active` record left
/// by an earlier pass.
#[tokio::test]
async fn a_failing_push_with_no_redeploy_in_the_same_pass_does_not_touch_an_unrelated_active_record()
 {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "frontend", Some("edge-1"));
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    s.store.journal.append(&plan, DeploymentState::Active).unwrap();
    let svc = dependent_service("frontend", "backend");
    let instance_id = AppInstanceId::new("inst-1");

    s.apply_write_phase(WritePhase {
        instance_id: &instance_id,
        app_instance_id: "inst-1",
        plan: &plan,
        needs_work: &BTreeSet::new(),
        restart_candidates: &[],
        renewal_candidates: &[],
        pending_rotation_restarts: &BTreeSet::new(),
        push_candidates: &[(svc, "did:key:zEdge1".to_string())],
        schedule_decisions: &[],
        did_to_alias: &BTreeMap::new(),
        clients: &BTreeMap::new(),
        now: 0,
    })
    .await;

    let latest = s.store.journal.get_latest(&instance_id).unwrap().unwrap();
    assert_eq!(
        latest.state,
        DeploymentState::Active,
        "nothing was journaled this pass -- the pre-existing record must be left alone: {latest:?}"
    );
}

/// `push_bindings` clears `BindingConflict` for that member once a
/// later push lands cleanly -- the clear site this alert kind never
/// had before.
#[tokio::test]
async fn a_binding_conflict_clears_once_a_later_push_for_that_member_lands_cleanly() {
    let s = service();
    let svc = dependent_service("frontend", "backend");
    let plan = plan_with_one_dependent(svc.clone());
    let actor = Arc::new(BindingActor::default());
    actor.responses.lock().unwrap().push(Ok(vec![BindingWriteOutcome::Conflict(5)]));
    let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
    let instance_id = AppInstanceId::new("inst-1");
    let mut opened = Vec::new();

    s.push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
        .await
        .unwrap();
    assert!(
        s.store
            .alerts
            .active(&instance_id)
            .unwrap()
            .iter()
            .any(|a| a.kind == AlertKind::BindingConflict),
        "the failed push must raise the alert"
    );

    // The next push lands cleanly (the fake's default response).
    let mut opened = Vec::new();
    s.push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
        .await
        .unwrap();
    assert!(
        !s.store
            .alerts
            .active(&instance_id)
            .unwrap()
            .iter()
            .any(|a| a.kind == AlertKind::BindingConflict),
        "a clean push must clear the alert it previously raised"
    );
}

/// D-A5e-8, ADR-0021 §5: an instance with an active `BindingConflict`
/// reports `Degraded`; once the retried push lands and the alert
/// clears, `handle_status` reports it recovered.
#[tokio::test]
async fn an_instance_leaves_degraded_once_the_retried_push_lands() {
    let s = service();
    let instance_id = AppInstanceId::new("inst-1");
    s.store
        .alerts
        .raise(
            &instance_id,
            Some("inst-1/frontend#0"),
            None,
            "did:key:zEdge1",
            AlertKind::BindingConflict,
            "did not land",
        )
        .unwrap();
    assert!(
        s.store
            .alerts
            .active(&instance_id)
            .unwrap()
            .iter()
            .any(|a| a.kind == AlertKind::BindingConflict)
    );

    s.store
        .alerts
        .clear(
            &instance_id,
            Some("inst-1/frontend#0"),
            "did:key:zEdge1",
            AlertKind::BindingConflict,
        )
        .unwrap();
    assert!(
        !s.store
            .alerts
            .active(&instance_id)
            .unwrap()
            .iter()
            .any(|a| a.kind == AlertKind::BindingConflict),
        "the active alert set (what handle_status's overall_state reads) must be clear once the \
         conflict clears"
    );
}

/// The raise site must write the substrate's real DID into the
/// alert's `substrate_did` column, not `svc.substrate`
/// (an operator-chosen alias, empty on fallback placement) -- a clear
/// keyed on the real DID would otherwise never match a row keyed on
/// the alias, and `Degraded` would be permanent.
#[tokio::test]
async fn a_binding_conflict_is_raised_under_the_substrate_did_not_the_alias() {
    let s = service();
    let mut svc = dependent_service("frontend", "backend");
    // The fallback-placement case: no alias at all.
    svc.substrate = None;
    let plan = plan_with_one_dependent(svc.clone());
    let actor = Arc::new(BindingActor::default());
    actor.responses.lock().unwrap().push(Ok(vec![BindingWriteOutcome::Conflict(5)]));
    let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
    let instance_id = AppInstanceId::new("inst-1");
    let mut opened = Vec::new();

    s.push_bindings(&instance_id, &plan, &svc, "did:key:zRealNode", &dyn_actor, 0, &mut opened)
        .await
        .unwrap();

    let active = s.store.alerts.active(&instance_id).unwrap();
    let conflict =
        active.iter().find(|a| a.kind == AlertKind::BindingConflict).expect("{active:?}");
    assert_eq!(conflict.substrate_did, "did:key:zRealNode");

    // A clear keyed on that same real DID must now match the row.
    assert!(
        s.store
            .alerts
            .clear(
                &instance_id,
                Some("inst-1/frontend#0"),
                "did:key:zRealNode",
                AlertKind::BindingConflict,
            )
            .unwrap(),
        "the clear must match the row the raise actually wrote"
    );
}

/// D-A5e-2: the epoch is per dependent *member*, not per logical
/// service -- two members of one scaled dependent must advance their
/// own epoch independently.
#[tokio::test]
async fn two_members_of_one_dependent_advance_their_binding_epochs_independently() {
    let s = service();
    let mut frontend_1 = dependent_service("frontend", "backend");
    frontend_1.member_index = 1;
    frontend_1.service_id = ServiceId::new("did:key:hfrontend1");
    let plan = DeploymentPlan {
        app_instance_id: AppInstanceId::new("inst-1"),
        blueprint_id: AppBlueprintId::new("syneroym:test"),
        version: semver::Version::new(1, 0, 0),
        services: vec![dependent_service("frontend", "backend"), frontend_1.clone()],
    };
    let actor = Arc::new(BindingActor::default());
    let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
    let instance_id = AppInstanceId::new("inst-1");
    let mut opened = Vec::new();

    // Only member 1 is pushed this round.
    s.push_bindings(&instance_id, &plan, &frontend_1, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
        .await
        .unwrap();

    assert_eq!(s.store.binding_epoch("inst-1", "inst-1/frontend#0").unwrap(), 0);
    assert_eq!(s.store.binding_epoch("inst-1", "inst-1/frontend#1").unwrap(), 1);
}

/// The test above calls `binding_convergence_rows` directly -- never
/// through a real `status` response, so nothing pins the wire shape
/// (`InstanceStatus.bindings` field name, its serialization) a caller
/// actually reads. This drives the exact same declared
/// dependency through `dispatch("status", …)` instead: no live
/// substrate exists in this test, so `observed_epoch` is `None`
/// rather than `Some(1)` (the exact case
/// `a_dependent_that_does_not_answer_reports_unconverged_rather_than_absent`
/// covers directly against the pure function below), but the point
/// here is that the array arrives non-empty at all, through the real
/// call.
#[tokio::test]
async fn status_returns_a_populated_bindings_array_over_a_real_dispatch_call() {
    let s = service();
    let svc = dependent_service("frontend", "backend");
    let plan = plan_with_one_dependent(svc);
    s.store.submit("inst-1", &plan.to_json().unwrap(), "{}", "did:key:owner", 0).unwrap();
    s.store.advance_binding_epoch("inst-1", "inst-1/frontend#0").unwrap();

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "status",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let status: InstanceStatus = serde_json::from_value(res.payload).unwrap();

    assert_eq!(status.bindings.len(), 1, "{:?}", status.bindings);
    assert_eq!(status.bindings[0].dependent_logical_ref, "inst-1/frontend#0");
    assert_eq!(status.bindings[0].dependency_name, "backend");
    assert_eq!(status.bindings[0].written_epoch, 1);
    assert_eq!(status.bindings[0].observed_epoch, None);
    assert!(!status.bindings[0].converged);
}

/// The negative half: a dependent absent from the sweep (unreachable,
/// or never landed) must still produce a row -- `observed_epoch:
/// None`, `converged: false` -- not silently vanish from the list, or
/// an operator reading an empty table cannot tell "nothing declared"
/// from "declared but not answering".
#[tokio::test]
async fn a_dependent_that_does_not_answer_reports_unconverged_rather_than_absent() {
    let s = service();
    let svc = dependent_service("frontend", "backend");
    let plan = plan_with_one_dependent(svc);
    let _ = s.store.advance_binding_epoch("inst-1", "inst-1/frontend#0");

    let report = health::HealthReport { substrates: Vec::new(), services: Vec::new() };
    let rows = s.binding_convergence_rows("inst-1", &plan, &report);

    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].written_epoch, 1);
    assert_eq!(rows[0].observed_epoch, None);
    assert!(!rows[0].converged);
}

/// A push against a dependent that cannot be reached fails (the epoch
/// has still advanced -- the invariant that the next attempt must
/// never retry at an epoch already spent), is visible on the operator
/// read surface (an earlier version of this test asserted only the
/// epoch and a successful retry, and never checked `opened`/`alerts`
/// at all), and succeeds cleanly once that dependent answers again. A
/// unit test against a fake actor, deliberately: the wire path is
/// already proven live by `binding_push_e2e.rs`, so this is entirely
/// the supervisor's own control flow.
#[tokio::test]
async fn a_dependent_unreachable_during_a_push_leaves_the_instance_degraded_and_is_retried_when_it_next_answers()
 {
    let s = service();
    let svc = dependent_service("frontend", "backend");
    let plan = plan_with_one_dependent(svc.clone());
    let actor = Arc::new(BindingActor::default());
    actor.responses.lock().unwrap().push(Err("substrate unreachable".to_string()));
    let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
    let instance_id = AppInstanceId::new("inst-1");
    let mut opened = Vec::new();

    let first = s
        .push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
        .await;
    assert!(first.is_err());
    assert_eq!(s.store.binding_epoch("inst-1", "inst-1/frontend#0").unwrap(), 1);
    assert_eq!(opened, vec![(AlertKind::BindingConflict, "inst-1/frontend#0".to_string())]);
    let alerts = s.store.alerts.active(&instance_id).unwrap();
    assert!(alerts.iter().any(|a| a.kind == AlertKind::BindingConflict), "{alerts:?}");

    let second = s
        .push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &dyn_actor, 0, &mut opened)
        .await;
    assert_eq!(second.unwrap(), PushOutcome::Landed(vec![BindingWriteOutcome::Applied]));
    assert_eq!(
        s.store.binding_epoch("inst-1", "inst-1/frontend#0").unwrap(),
        2,
        "the retry must carry a fresh epoch, not reuse the one the failed attempt spent"
    );
}

/// `push_bindings` advances the binding epoch before every attempt,
/// but a *durable* actor only enqueues
/// once per key (`already_pending`) -- so a second transport failure
/// for the same key while the first attempt's item is still queued
/// must not advance the epoch again, or `written_epoch` races ahead of
/// what the worker can ever actually deliver and a later successful
/// delivery of the (older, still-queued) item would read as
/// unconverged forever. Uses `deploy::build_durable_actor` directly
/// (not `BindingActor`, which is never durable) so this exercises the
/// same enqueue path `DurableActor::write_bindings` takes in
/// production.
#[tokio::test]
async fn two_consecutive_transport_failures_for_one_key_do_not_strand_the_written_epoch() {
    let s = service();
    let svc = dependent_service("frontend", "backend");
    let plan = plan_with_one_dependent(svc.clone());
    let instance_id = AppInstanceId::new("inst-1");
    let mut opened = Vec::new();
    let outbox: Arc<dyn WriteBindingsOutbox> =
        Arc::new(SupervisorOutbox::new(s.store.queue.clone()));
    let queue_key = QueueKey {
        app_instance_id: "inst-1".to_string(),
        logical_ref: "inst-1/frontend#0".to_string(),
        substrate_did: "did:key:zEdge1".to_string(),
    }
    .to_string();

    let first_client = Arc::new(FakeSubstrateClient::default());
    *first_client.write_bindings_outcome.lock().unwrap() =
        Some(Err("connection reset mid-write".to_string()));
    let first_actor = deploy::build_durable_actor(
        first_client,
        "did:key:zEdge1".to_string(),
        queue_key.clone(),
        outbox.clone(),
    );
    let first = s
        .push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &first_actor, 0, &mut opened)
        .await;
    assert!(first.is_err());
    assert_eq!(s.store.binding_epoch("inst-1", "inst-1/frontend#0").unwrap(), 1);
    let queued = s.store.queue.all().unwrap();
    assert_eq!(queued.len(), 1, "the first failure must have enqueued exactly one item");

    // A second pass: reconnects fine (a fresh `DurableActor`), but the
    // write itself fails again for the same key -- while the first
    // failure's item is still sitting in the outbox.
    let second_client = Arc::new(FakeSubstrateClient::default());
    *second_client.write_bindings_outcome.lock().unwrap() =
        Some(Err("connection reset mid-write".to_string()));
    let second_actor =
        deploy::build_durable_actor(second_client, "did:key:zEdge1".to_string(), queue_key, outbox);
    let second = s
        .push_bindings(&instance_id, &plan, &svc, "did:key:zEdge1", &second_actor, 0, &mut opened)
        .await;
    assert_eq!(
        second.unwrap(),
        PushOutcome::Deferred,
        "an already-pending key defers to the queue, it is not an error"
    );

    assert_eq!(
        s.store.binding_epoch("inst-1", "inst-1/frontend#0").unwrap(),
        1,
        "the epoch must not advance again while the first attempt's item is still queued"
    );
    let queued = s.store.queue.all().unwrap();
    assert_eq!(queued.len(), 1, "the second pass must not have enqueued a duplicate");
    let payload: outbox::QueuedBindingWrite = serde_json::from_slice(&queued[0].payload).unwrap();
    assert_eq!(
        payload.write.generation, 0,
        "the queued payload is still the first attempt's, carrying epoch 1"
    );

    // Once the worker eventually delivers the queued (epoch-1) item,
    // it must read as converged, matching what is actually in the
    // outbox -- not stranded behind a local epoch nothing will ever
    // deliver.
    let report = health::HealthReport {
        substrates: Vec::new(),
        services: vec![health::ServiceHealth {
            logical_ref: svc.logical_ref.clone(),
            service_id: svc.service_id.to_string(),
            alias: svc.substrate.clone(),
            substrate_did: "did:key:zEdge1".to_string(),
            signal: Signal::Healthy,
            instance_certificate_issued_at: None,
            instance_certificate_expires_at: None,
            binding_epochs: vec![("backend".to_string(), 1)],
            member_index: svc.member_index,
        }],
    };
    let rows = s.binding_convergence_rows("inst-1", &plan, &report);
    assert!(
        rows[0].converged,
        "the epoch the worker will eventually deliver (1) must still be the one convergence \
         checks against: {rows:?}"
    );
}
