use std::collections::BTreeMap;

use syneroym_app_orchestration::{ActionState, DEFAULT_SCHEDULE_TIMEOUT_MS, models::InterfaceName};

use super::{super::*, helpers::*};

#[test]
fn a_schedule_seen_for_the_first_time_does_not_fire_for_the_past() {
    let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "* * * * *")]);
    let report = report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
    let decisions =
        SupervisorService::schedule_decisions(&plan, &BTreeMap::new(), &report, NOW, 60);
    assert_eq!(
        decisions,
        vec![ScheduleDecision::Watermark { logical_ref: "inst-1/worker".to_string() }],
        "a schedule with no state row must not fire on the pass that first sees it"
    );
}

#[test]
fn a_due_schedule_runs_exactly_one_member() {
    let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "* * * * * *")]);
    let report = report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
    let mut states = BTreeMap::new();
    states.insert(
        "inst-1/worker".to_string(),
        ScheduleState { evaluated_at: (NOW - 3600) as i64, ..Default::default() },
    );
    let decisions = SupervisorService::schedule_decisions(&plan, &states, &report, NOW, 3600);
    assert_eq!(decisions.len(), 1);
    match &decisions[0] {
        ScheduleDecision::Run { logical_ref, member_index, service_id, substrate_did, .. } => {
            assert_eq!(logical_ref, "inst-1/worker");
            assert_eq!(*member_index, 0);
            assert_eq!(service_id, "did:key:hworker0");
            assert_eq!(substrate_did, "did:key:zEdge1");
        }
        other => panic!("expected Run, got {other:?}"),
    }
}

#[test]
fn a_schedule_evaluated_twice_inside_one_cron_minute_runs_once() {
    let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "* * * * *")]);
    let report = report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);

    // First pass, exactly on the minute boundary: due.
    let mut states = BTreeMap::new();
    states.insert(
        "inst-1/worker".to_string(),
        ScheduleState { evaluated_at: (minute(0) - 30) as i64, ..Default::default() },
    );
    let first = SupervisorService::schedule_decisions(&plan, &states, &report, minute(0), 60);
    assert!(matches!(first[0], ScheduleDecision::Run { .. }));

    // A second pass ten seconds later, inside the same cron minute,
    // with the state a real `record_schedule_started` would have left.
    states.insert(
        "inst-1/worker".to_string(),
        ScheduleState {
            evaluated_at: minute(0) as i64,
            last_run_at: Some(minute(0) as i64),
            last_member_index: Some(0),
            last_error: None,
        },
    );
    let second = SupervisorService::schedule_decisions(&plan, &states, &report, minute(0) + 10, 60);
    assert_eq!(
        second,
        vec![ScheduleDecision::Watermark { logical_ref: "inst-1/worker".to_string() }],
        "a second look inside the same cron minute must not run again"
    );
}

#[test]
fn a_tick_missed_while_the_supervisor_was_down_is_skipped_not_run_late() {
    let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "* * * * *")]);
    let report = report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
    let mut states = BTreeMap::new();
    // Down for an hour before minute(0)'s own occurrence.
    states.insert(
        "inst-1/worker".to_string(),
        ScheduleState { evaluated_at: (minute(0) - 3600) as i64, ..Default::default() },
    );
    // Comes back 40s after the boundary, with a grace window smaller
    // than the gap -- the boundary itself has already fallen outside
    // the window this pass computes.
    let now = minute(0) + 40;
    let decisions = SupervisorService::schedule_decisions(&plan, &states, &report, now, 30);
    assert_eq!(
        decisions,
        vec![ScheduleDecision::Watermark { logical_ref: "inst-1/worker".to_string() }],
        "an hour-long gap must not fire a burst of catch-up runs"
    );
}

#[test]
fn a_pass_delayed_by_less_than_the_grace_window_still_runs_its_tick() {
    let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "* * * * *")]);
    let report = report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
    let mut states = BTreeMap::new();
    states.insert(
        "inst-1/worker".to_string(),
        ScheduleState { evaluated_at: (minute(0) - 3600) as i64, ..Default::default() },
    );
    // 40s late, but the grace window (60s) still covers minute(0)'s
    // own occurrence.
    let now = minute(0) + 40;
    let decisions = SupervisorService::schedule_decisions(&plan, &states, &report, now, 60);
    assert!(
        matches!(&decisions[0], ScheduleDecision::Run { .. }),
        "ordinary jitter under the grace window must not silently drop the tick: {decisions:?}"
    );
}

/// Before the loop has completed a second sweep there is no observed
/// gap to read, so the window is the configured floor -- which is also
/// what a fresh process gets, so downtime can never widen it.
#[test]
fn the_grace_window_is_two_poll_intervals_until_a_sweep_has_been_timed() {
    let s = Fixture { poll_interval_secs: Some(30), ..Fixture::default() }.build();
    assert_eq!(s.schedule_grace_secs(NOW), 60);
}

/// The defect this rule exists for: a sweep that takes longer than two
/// poll intervals -- routine, since every pass rebuilds an iroh client
/// per substrate -- used to leave a hole between the last evaluation
/// and the start of the window, and every occurrence inside that hole
/// was dropped while the supervisor was awake the whole time.
#[test]
fn a_sweep_slower_than_two_poll_intervals_widens_the_grace_window_to_match() {
    let s = Fixture { poll_interval_secs: Some(30), ..Fixture::default() }.build();
    s.previous_pass_started_at.store(NOW - 300, Ordering::Relaxed);
    assert_eq!(s.schedule_grace_secs(NOW), 300);
}

/// The same defect, end to end through the decision it feeds: the
/// watermark is 100s old because the previous sweep was 100s ago, and
/// the tick in between must still fire even though the *configured*
/// interval says a pass should have happened four times over.
#[test]
fn a_sweep_slower_than_two_poll_intervals_still_fires_the_tick_it_covered() {
    let s = Fixture { poll_interval_secs: Some(10), ..Fixture::default() }.build();
    let now = minute(0) + 40;
    s.previous_pass_started_at.store(now - 100, Ordering::Relaxed);

    let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "* * * * *")]);
    let report = report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
    let states = BTreeMap::from([(
        "inst-1/worker".to_string(),
        ScheduleState { evaluated_at: (now - 100) as i64, ..Default::default() },
    )]);

    let decisions = SupervisorService::schedule_decisions(
        &plan,
        &states,
        &report,
        now,
        s.schedule_grace_secs(now),
    );
    assert!(
        matches!(&decisions[0], ScheduleDecision::Run { .. }),
        "a tick inside the real gap between two sweeps must not be dropped: {decisions:?}"
    );
}

/// The other side of the same rule. A paused instance is skipped before
/// the health sweep, so its watermark goes stale -- but the loop keeps
/// sweeping the whole time, so the observed gap stays one poll interval
/// and the window on resume is still the floor. Nothing catches up.
#[tokio::test]
async fn a_paused_instance_fires_no_backlog_when_it_resumes() {
    let s = Fixture { poll_interval_secs: Some(10), ..Fixture::default() }.build();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
    s.store.pause("inst-1").unwrap();

    // A sweep over nothing but paused instances still times itself:
    // the liveness signal belongs to the loop, not to any instance.
    s.run_pass().await;
    assert_ne!(s.previous_pass_started_at.load(Ordering::Relaxed), 0);
    s.store.resume("inst-1").unwrap();

    // So on resume the observed gap is one sweep, however long the
    // pause was.
    let now = minute(0) + 40;
    s.previous_pass_started_at.store(now - 10, Ordering::Relaxed);
    assert_eq!(s.schedule_grace_secs(now), 20, "a pause must not widen the window");

    let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "* * * * *")]);
    let report = report_of(vec![scheduled_health("worker", 0, "did:key:zEdge1", Signal::Healthy)]);
    // The watermark an hour-long pause leaves behind.
    let states = BTreeMap::from([(
        "inst-1/worker".to_string(),
        ScheduleState { evaluated_at: (now - 3600) as i64, ..Default::default() },
    )]);

    let decisions = SupervisorService::schedule_decisions(
        &plan,
        &states,
        &report,
        now,
        s.schedule_grace_secs(now),
    );
    assert_eq!(
        decisions,
        vec![ScheduleDecision::Watermark { logical_ref: "inst-1/worker".to_string() }],
        "resuming must not fire the ticks that fell inside the pause: {decisions:?}"
    );
}

/// The write phase's own re-read (D-A5c-14) covers scheduled work too:
/// a `pause` that lands between the health sweep and the write phase
/// must stop the tick, not merely the deploys -- the run is dispatched
/// from inside that phase, after the re-read.
#[tokio::test]
async fn a_pause_landing_mid_pass_stops_that_passs_scheduled_run() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "worker", None);
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    s.store.pause("inst-1").unwrap();

    s.apply_write_phase(WritePhase {
        instance_id: &AppInstanceId::new("inst-1"),
        app_instance_id: "inst-1",
        plan: &plan,
        needs_work: &BTreeSet::new(),
        restart_candidates: &[],
        renewal_candidates: &[],
        pending_rotation_restarts: &BTreeSet::new(),
        push_candidates: &[],
        schedule_decisions: &[ScheduleDecision::Watermark {
            logical_ref: "inst-1/worker".to_string(),
        }],
        did_to_alias: &BTreeMap::new(),
        clients: &BTreeMap::new(),
        now: 100,
    })
    .await;

    assert!(
        s.store.schedule_states("inst-1").unwrap().is_empty(),
        "a paused instance must not even advance a watermark"
    );
}

/// A schedule dropped from a resubmitted manifest leaves a row behind
/// that nothing short of retiring the instance would ever delete, and
/// that `schedules` cannot show, since it reads the plan. The pass that
/// knows the declared set reclaims it.
#[tokio::test]
async fn a_schedule_the_plan_no_longer_declares_loses_its_state_row() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "worker", None);
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
    s.store.record_schedule_started("inst-1", "inst-1/worker", 100, 0).unwrap();

    s.reconcile_instance_pass("inst-1").await;

    assert!(
        s.store.schedule_states("inst-1").unwrap().is_empty(),
        "the plan declares no schedule, so no schedule state may survive the pass"
    );
}

/// `last_member_index` is absent, not 0, until a run has happened --
/// otherwise the round-robin reads a fresh row as "member 0 already
/// ran" and sends the very first tick of a multi-member service to
/// member 1.
#[test]
fn the_first_tick_of_a_multi_member_service_runs_member_zero() {
    let plan = plan_with_schedule(vec![
        scheduled_service("worker", 0, "* * * * * *"),
        scheduled_service("worker", 1, "* * * * * *"),
    ]);
    let report = report_of(vec![
        scheduled_health("worker", 0, "did:key:zEdgeA", Signal::Healthy),
        scheduled_health("worker", 1, "did:key:zEdgeB", Signal::Healthy),
    ]);
    let states = BTreeMap::from([(
        "inst-1/worker".to_string(),
        ScheduleState { evaluated_at: 0, ..Default::default() },
    )]);

    let decisions = SupervisorService::schedule_decisions(&plan, &states, &report, NOW, 3600);
    match &decisions[0] {
        ScheduleDecision::Run { member_index, .. } => assert_eq!(*member_index, 0),
        other => panic!("expected Run, got {other:?}"),
    }
}

#[test]
fn selection_rotates_across_healthy_members_on_consecutive_ticks() {
    let plan = plan_with_schedule(vec![
        scheduled_service("worker", 0, "* * * * * *"),
        scheduled_service("worker", 1, "* * * * * *"),
        scheduled_service("worker", 2, "* * * * * *"),
    ]);
    let report = report_of(vec![
        scheduled_health("worker", 0, "did:key:zEdgeA", Signal::Healthy),
        scheduled_health("worker", 1, "did:key:zEdgeB", Signal::Healthy),
        scheduled_health("worker", 2, "did:key:zEdgeC", Signal::Healthy),
    ]);
    for (last_index, expected_index) in [(0u32, 1u32), (1, 2), (2, 0)] {
        let mut states = BTreeMap::new();
        states.insert(
            "inst-1/worker".to_string(),
            ScheduleState {
                evaluated_at: 0,
                last_member_index: Some(last_index),
                ..Default::default()
            },
        );
        let decisions = SupervisorService::schedule_decisions(&plan, &states, &report, NOW, 3600);
        match &decisions[0] {
            ScheduleDecision::Run { member_index, .. } => {
                assert_eq!(*member_index, expected_index, "after member {last_index}");
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }
}

#[test]
fn an_unhealthy_member_is_never_selected_and_does_not_block_the_schedule() {
    let plan = plan_with_schedule(vec![
        scheduled_service("worker", 0, "* * * * * *"),
        scheduled_service("worker", 1, "* * * * * *"),
    ]);
    let report = report_of(vec![
        scheduled_health("worker", 0, "did:key:zEdgeA", Signal::Healthy),
        scheduled_health("worker", 1, "did:key:zEdgeB", Signal::ProbeFailing("down".to_string())),
    ]);
    let mut states = BTreeMap::new();
    states.insert(
        "inst-1/worker".to_string(),
        ScheduleState { evaluated_at: 0, ..Default::default() },
    );
    let decisions = SupervisorService::schedule_decisions(&plan, &states, &report, NOW, 3600);
    match &decisions[0] {
        ScheduleDecision::Run { member_index, substrate_did, .. } => {
            assert_eq!(*member_index, 0);
            assert_eq!(substrate_did, "did:key:zEdgeA");
        }
        other => panic!("expected Run, got {other:?}"),
    }
}

#[test]
fn a_schedule_with_no_healthy_member_advances_its_watermark_and_skips() {
    let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "* * * * * *")]);
    let report = report_of(vec![scheduled_health(
        "worker",
        0,
        "did:key:zEdgeA",
        Signal::SubstrateUnreachable("down".to_string()),
    )]);
    let mut states = BTreeMap::new();
    states.insert(
        "inst-1/worker".to_string(),
        ScheduleState { evaluated_at: 0, ..Default::default() },
    );
    let decisions = SupervisorService::schedule_decisions(&plan, &states, &report, NOW, 3600);
    assert_eq!(
        decisions,
        vec![ScheduleDecision::Watermark { logical_ref: "inst-1/worker".to_string() }]
    );
}

#[test]
fn a_schedule_only_update_is_excluded_from_redeploy_but_is_not_a_push_candidate() {
    let old = scheduled_service("worker", 0, "* * * * *");
    let mut new = old.clone();
    new.schedule.as_mut().unwrap().cron = "0 3 * * *".to_string();
    let actions = vec![ReconcileAction::Update { old: Box::new(old), new: Box::new(new) }];
    let (redeploy_exclusions, push_candidates) =
        SupervisorService::classify_update_actions(&[], &actions);
    assert!(redeploy_exclusions.contains("inst-1/worker#0"));
    assert!(push_candidates.is_empty());
}

/// The classifier's first call site: the loop's own work list. Testing
/// the classifier alone says nothing about whether either caller
/// honours it -- fixing one path and not the other is the exact gap an
/// earlier review round found.
#[test]
fn a_schedule_only_edit_does_not_redeploy_the_service() {
    let old = scheduled_service("worker", 0, "* * * * *");
    let mut new = old.clone();
    new.schedule.as_mut().unwrap().cron = "0 3 * * *".to_string();
    let actions = vec![ReconcileAction::Update { old: Box::new(old), new: Box::new(new) }];
    let (redeploy_exclusions, _) = SupervisorService::classify_update_actions(&[], &actions);

    let needs_work =
        SupervisorService::redeploy_work_list(&BTreeSet::new(), &actions, &redeploy_exclusions);
    assert!(needs_work.is_empty(), "a schedule-only edit is not redeploy work: {needs_work:?}");
}

/// Same edit, the other call site: `submit`/`force-reconcile`. The plan
/// applied must exclude the member, while the plan *journaled* still
/// carries the whole thing -- a baseline narrowed to this call's own
/// subset is what makes later passes redeploy everything it left out.
#[tokio::test]
async fn a_schedule_only_edit_on_submit_does_not_redeploy_the_service() {
    let s = service();
    let old = scheduled_service("worker", 0, "* * * * *");
    let old_plan = plan_with_schedule(vec![old.clone()]);
    let deployment_id = s.store.journal.append(&old_plan, DeploymentState::Active).unwrap();
    s.store
        .journal
        .append_action(
            deployment_id,
            "ADD",
            "inst-1/worker#0",
            Some("edge-1"),
            "did:key:zEdge1",
            ActionState::Completed,
        )
        .unwrap();

    let mut new = old.clone();
    new.schedule.as_mut().unwrap().cron = "0 3 * * *".to_string();
    let plan = plan_with_schedule(vec![new]);

    // No clients: a member that reached `apply_plan` would fail for
    // want of a target and journal `Degraded`, so an `Active` record
    // with no new action row is the direct evidence it was excluded.
    s.apply_with_membership_pushes(&plan, &BTreeMap::new(), &BTreeMap::new(), 0, Vec::new())
        .await
        .expect("a schedule-only resubmit must not fail for want of a substrate");

    let latest = s.store.journal.get_latest(&AppInstanceId::new("inst-1")).unwrap().unwrap();
    assert_eq!(latest.state, DeploymentState::Active);
    assert_eq!(
        latest.plan.services[0].schedule.as_ref().unwrap().cron,
        "0 3 * * *",
        "the journaled baseline must still carry the whole plan, new schedule included"
    );
    let actions =
        s.store.journal.get_completed_actions_for_instance(&AppInstanceId::new("inst-1")).unwrap();
    assert_eq!(actions.len(), 1, "no second placement action: {actions:?}");
}

#[test]
fn a_simultaneous_schedule_and_membership_edit_is_not_classified_as_membership_only() {
    let old = dependent_service("frontend", "backend");
    let mut new = old.clone();
    new.resolved_dependencies = BTreeMap::from([(
        LogicalServiceName::new("backend"),
        vec![ServiceId::new("did:key:hDepMember2")],
    )]);
    new.schedule = Some(ScheduleSpec {
        cron: "* * * * *".to_string(),
        interface: InterfaceName::new("scheduled-driver"),
        method: "tick".to_string(),
        params: None,
        timeout_ms: DEFAULT_SCHEDULE_TIMEOUT_MS,
    });
    assert!(!SupervisorService::only_resolved_dependencies_changed(&old, &new));
    assert!(!SupervisorService::only_schedule_changed(&old, &new));

    let actions = vec![ReconcileAction::Update { old: Box::new(old), new: Box::new(new) }];
    let (redeploy_exclusions, push_candidates) =
        SupervisorService::classify_update_actions(&[], &actions);
    assert!(
        redeploy_exclusions.is_empty(),
        "a schedule change alongside a membership change must not be excluded from redeploy"
    );
    assert!(push_candidates.is_empty());
}

#[test]
fn refuse_unrunnable_schedules_refuses_a_plan_naming_more_scheduled_services_than_the_cap() {
    let services: Vec<PlannedService> = (0..=MAX_SCHEDULED_SERVICES)
        .map(|i| scheduled_service(&format!("worker-{i}"), 0, "* * * * *"))
        .collect();
    let plan = plan_with_schedule(services);
    let err = SupervisorService::refuse_unrunnable_schedules(&plan).unwrap_err();
    assert!(err.contains(&format!("above the cap of {MAX_SCHEDULED_SERVICES}")), "{err}");
}

#[test]
fn refuse_unrunnable_schedules_allows_a_plan_exactly_at_the_cap() {
    let services: Vec<PlannedService> = (0..MAX_SCHEDULED_SERVICES)
        .map(|i| scheduled_service(&format!("worker-{i}"), 0, "* * * * *"))
        .collect();
    let plan = plan_with_schedule(services);
    assert!(SupervisorService::refuse_unrunnable_schedules(&plan).is_ok());
}

/// The manifest's own bound is compile-time only, and `submit` takes an
/// already-compiled plan -- so without this check a hand-edited plan
/// reproduces the original defect exactly: the runtime clamp is a
/// `min`, which a zero survives, and the tick is then consumed by a
/// timeout that elapses before the call starts.
#[test]
fn refuse_unrunnable_schedules_refuses_a_submitted_plan_with_a_zero_timeout() {
    let mut svc = scheduled_service("worker", 0, "* * * * *");
    svc.schedule.as_mut().unwrap().timeout_ms = 0;
    let plan = plan_with_schedule(vec![svc]);
    let err = SupervisorService::refuse_unrunnable_schedules(&plan).unwrap_err();
    assert!(err.contains("must be between 1"), "{err}");
}

#[test]
fn refuse_unrunnable_schedules_refuses_a_submitted_plan_above_the_timeout_ceiling() {
    let mut svc = scheduled_service("worker", 0, "* * * * *");
    svc.schedule.as_mut().unwrap().timeout_ms = MAX_SCHEDULE_TIMEOUT_MS + 1;
    let plan = plan_with_schedule(vec![svc]);
    let err = SupervisorService::refuse_unrunnable_schedules(&plan).unwrap_err();
    assert!(err.contains(&format!("{MAX_SCHEDULE_TIMEOUT_MS}ms")), "{err}");
}

/// An unparseable cron is deliberately *not* a submission-level
/// refusal: it degrades to the watermark branch, which skips that one
/// schedule and leaves the rest of the instance reconciling. Pinned so
/// the asymmetry with the budget above is a decision, not a gap.
#[test]
fn refuse_unrunnable_schedules_allows_a_plan_whose_cron_does_not_parse() {
    let plan = plan_with_schedule(vec![scheduled_service("worker", 0, "not a cron")]);
    assert!(SupervisorService::refuse_unrunnable_schedules(&plan).is_ok());
}

#[tokio::test]
async fn a_failed_run_raises_scheduled_run_failed_and_the_next_success_clears_it() {
    let s = service();
    let instance_id = AppInstanceId::new("inst-1");
    let actor = Arc::new(ScheduledActor::default());
    *actor.error.lock().unwrap() = Some("boom".to_string());
    let actors: BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> =
        BTreeMap::from([(SubstrateAlias::new("edge-1"), deploy::build_actor(actor.clone()))]);
    let did_to_alias = edge_1_alias();
    let decisions = vec![run_decision("inst-1/worker", "did:key:hworker0", "did:key:zEdge1")];
    let mut opened = Vec::new();

    s.run_due_schedules(
        &instance_id,
        "inst-1",
        &decisions,
        &did_to_alias,
        &actors,
        0,
        NOW,
        &mut opened,
    )
    .await;

    let active = s.store.alerts.active(&instance_id).unwrap();
    assert!(
        active.iter().any(|a| a.kind == AlertKind::ScheduledRunFailed
            && a.substrate_did == SCHEDULE_SUBSTRATE_DID),
        "a failed run must raise ScheduledRunFailed under the sentinel, not the member's own \
         substrate: {active:?}"
    );
    assert_eq!(opened, vec![(AlertKind::ScheduledRunFailed, "inst-1/worker".to_string())]);

    *actor.error.lock().unwrap() = None;
    let mut opened2 = Vec::new();
    s.run_due_schedules(
        &instance_id,
        "inst-1",
        &decisions,
        &did_to_alias,
        &actors,
        0,
        NOW + 60,
        &mut opened2,
    )
    .await;
    let active = s.store.alerts.active(&instance_id).unwrap();
    assert!(
        !active.iter().any(|a| a.kind == AlertKind::ScheduledRunFailed),
        "the next successful run must clear the alert: {active:?}"
    );
}

#[tokio::test]
async fn a_failed_run_is_never_enqueued_onto_the_outbox() {
    let s = service();
    let instance_id = AppInstanceId::new("inst-1");
    let actor = Arc::new(ScheduledActor::default());
    *actor.error.lock().unwrap() = Some("boom".to_string());
    let actors: BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> =
        BTreeMap::from([(SubstrateAlias::new("edge-1"), deploy::build_actor(actor))]);
    let did_to_alias = edge_1_alias();
    let decisions = vec![run_decision("inst-1/worker", "did:key:hworker0", "did:key:zEdge1")];
    let mut opened = Vec::new();

    s.run_due_schedules(
        &instance_id,
        "inst-1",
        &decisions,
        &did_to_alias,
        &actors,
        0,
        NOW,
        &mut opened,
    )
    .await;

    assert_eq!(s.store.queue.pending_count().unwrap(), 0);
    assert!(s.store.queue.dead_letters().unwrap().is_empty());
}

#[tokio::test]
async fn the_run_is_recorded_before_the_call_so_a_crash_mid_run_skips_the_tick() {
    let s = service();
    let instance_id = AppInstanceId::new("inst-1");
    let actor = Arc::new(AssertsStartedBeforeCallActor {
        store: s.store.clone(),
        expected_run_at: NOW as i64,
    });
    let actors: BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> =
        BTreeMap::from([(SubstrateAlias::new("edge-1"), deploy::build_actor(actor))]);
    let did_to_alias = edge_1_alias();
    let decisions = vec![run_decision("inst-1/worker", "did:key:hworker0", "did:key:zEdge1")];
    let mut opened = Vec::new();

    s.run_due_schedules(
        &instance_id,
        "inst-1",
        &decisions,
        &did_to_alias,
        &actors,
        0,
        NOW,
        &mut opened,
    )
    .await;
}

#[tokio::test]
async fn a_failure_on_one_member_is_cleared_by_a_success_on_another_members_substrate() {
    let s = service();
    let instance_id = AppInstanceId::new("inst-1");
    let failing_actor = Arc::new(ScheduledActor::default());
    *failing_actor.error.lock().unwrap() = Some("boom".to_string());
    let succeeding_actor = Arc::new(ScheduledActor::default());
    let actors: BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> = BTreeMap::from([
        (SubstrateAlias::new("edge-a"), deploy::build_actor(failing_actor)),
        (SubstrateAlias::new("edge-b"), deploy::build_actor(succeeding_actor)),
    ]);
    let did_to_alias = BTreeMap::from([
        ("did:key:zEdgeA".to_string(), "edge-a".to_string()),
        ("did:key:zEdgeB".to_string(), "edge-b".to_string()),
    ]);
    let mut opened = Vec::new();

    let decisions_a = vec![run_decision("inst-1/worker", "did:key:hworkerA", "did:key:zEdgeA")];
    s.run_due_schedules(
        &instance_id,
        "inst-1",
        &decisions_a,
        &did_to_alias,
        &actors,
        0,
        NOW,
        &mut opened,
    )
    .await;
    let active = s.store.alerts.active(&instance_id).unwrap();
    assert!(active.iter().any(|a| a.kind == AlertKind::ScheduledRunFailed));

    let decisions_b = vec![run_decision("inst-1/worker", "did:key:hworkerB", "did:key:zEdgeB")];
    s.run_due_schedules(
        &instance_id,
        "inst-1",
        &decisions_b,
        &did_to_alias,
        &actors,
        0,
        NOW + 60,
        &mut opened,
    )
    .await;
    let active = s.store.alerts.active(&instance_id).unwrap();
    assert!(
        !active.iter().any(|a| a.kind == AlertKind::ScheduledRunFailed),
        "a success on a different member's substrate must clear the sentinel-keyed alert: \
         {active:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_schedule_timeout_is_clamped_to_the_ceiling() {
    let s = service();
    let instance_id = AppInstanceId::new("inst-1");
    let actor =
        Arc::new(ScheduledActor { delay: Some(Duration::from_secs(60)), ..Default::default() });
    let actors: BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> =
        BTreeMap::from([(SubstrateAlias::new("edge-1"), deploy::build_actor(actor))]);
    let did_to_alias = edge_1_alias();
    let decisions = vec![ScheduleDecision::Run {
        logical_ref: "inst-1/worker".to_string(),
        service_id: "did:key:hworker0".to_string(),
        substrate_did: "did:key:zEdge1".to_string(),
        member_index: 0,
        schedule: ScheduleSpec {
            cron: "* * * * *".to_string(),
            interface: InterfaceName::new("scheduled-driver"),
            method: "tick".to_string(),
            params: None,
            // Above the ceiling on purpose -- the ceiling must win.
            timeout_ms: 100_000,
        },
    }];
    let mut opened = Vec::new();

    let start = tokio::time::Instant::now();
    s.run_due_schedules(
        &instance_id,
        "inst-1",
        &decisions,
        &did_to_alias,
        &actors,
        0,
        NOW,
        &mut opened,
    )
    .await;
    let elapsed = start.elapsed();

    assert!(
        elapsed >= SCHEDULED_RUN_CEILING && elapsed < Duration::from_secs(60),
        "the run must time out at the 30s ceiling, not the configured 100s or the actor's own 60s \
         delay: {elapsed:?}"
    );
    let states = s.store.schedule_states("inst-1").unwrap();
    assert!(
        states
            .get("inst-1/worker")
            .unwrap()
            .last_error
            .as_deref()
            .unwrap_or("")
            .contains("timed out"),
        "{:?}",
        states.get("inst-1/worker")
    );
}

#[tokio::test]
async fn schedules_lists_a_declared_schedule_that_has_never_run() {
    let s = service();
    s.store
        .submit(
            "inst-1",
            &plan_json_with_schedule("worker", "did:key:hworker0"),
            "{}",
            "did:key:owner",
            0,
        )
        .unwrap();

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "schedules",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let tasks: Vec<ScheduledTask> = serde_json::from_value(res.payload).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].logical_ref, "inst-1/worker");
    assert_eq!(tasks[0].cron, "* * * * *");
    assert_eq!(tasks[0].interface, "scheduled-driver");
    assert_eq!(tasks[0].method, "tick");
    assert_eq!(tasks[0].evaluated_at, 0, "a schedule never evaluated must read as 0");
    assert_eq!(tasks[0].last_run_at, None);
    assert_eq!(tasks[0].last_member_index, None);
    assert_eq!(tasks[0].last_error, None);
}

#[tokio::test]
async fn schedules_reports_the_member_and_time_of_the_last_run() {
    let s = service();
    s.store
        .submit(
            "inst-1",
            &plan_json_with_schedule("worker", "did:key:hworker0"),
            "{}",
            "did:key:owner",
            0,
        )
        .unwrap();
    s.store.record_schedule_started("inst-1", "inst-1/worker", 12_345, 2).unwrap();

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "schedules",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let tasks: Vec<ScheduledTask> = serde_json::from_value(res.payload).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].evaluated_at, 12_345);
    assert_eq!(tasks[0].last_run_at, Some(12_345));
    assert_eq!(tasks[0].last_member_index, Some(2));
}
