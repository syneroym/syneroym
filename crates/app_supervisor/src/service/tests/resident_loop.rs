use std::collections::BTreeMap;

use syneroym_app_orchestration::{ActionState, models::AppBlueprintId};

use super::{super::*, helpers::*};

/// The property `record_report`'s newly-opened return value provides:
/// an alert already active before this sweep is not published again,
/// so an operator subscribed to the topic sees one message per
/// incident, not one per poll.
#[tokio::test]
async fn an_already_open_alert_is_not_republished_on_the_next_sweep() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", None);
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

    let topic = expected_alert_topic("inst-1");
    let (_handle, mut receiver) = s.messaging_broker.subscribe(topic).await.unwrap();

    for _ in 0..2 {
        dispatch(
            &s,
            admin_caller("did:key:zSupervisorNode"),
            "status",
            serde_json::json!(["inst-1"]),
        )
        .await
        .unwrap();
    }

    // Exactly one message must have arrived, from the first sweep.
    let _first = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .expect("did not time out waiting for the first publish")
        .expect("broker channel closed");
    let second = tokio::time::timeout(Duration::from_millis(300), receiver.recv()).await;
    assert!(second.is_err(), "the second sweep must not republish the still-open alert");
}

#[tokio::test]
async fn the_supervisor_wit_dispatch_table_covers_every_declared_function() {
    let (resolve, iface_id) = supervisor_interface();
    let iface = &resolve.interfaces[iface_id];
    assert!(!iface.functions.is_empty(), "supervisor interface should have functions");

    let s = service();
    for name in iface.functions.keys() {
        let method_name = name.strip_prefix('%').unwrap_or(name);
        let res = dispatch(&s, unauthenticated_caller(), method_name, Value::Null).await;
        if let Err(RpcError::MethodNotFound(m)) = res {
            panic!("WIT function '{name}' maps to method name '{m}' but was not dispatched");
        }
    }
}

/// A by-construction property, pinned so a later change cannot quietly
/// reintroduce a key-bearing verb: walks the WIT interface and asserts
/// no function or record field is named like key material.
#[test]
fn no_supervisor_verb_accepts_or_returns_key_material() {
    let (resolve, iface_id) = supervisor_interface();
    let iface = &resolve.interfaces[iface_id];

    let suspicious = |name: &str| {
        let lower = name.to_lowercase();
        (lower.contains("key") && !lower.contains("key-hex")) || lower.contains("secret")
    };
    for (type_name, ty) in &iface.types {
        assert!(!suspicious(type_name), "type '{type_name}' looks like key material");
        if let wit_parser::TypeDefKind::Record(record) = &resolve.types[*ty].kind {
            for field in &record.fields {
                assert!(!suspicious(&field.name), "field '{}' looks like key material", field.name);
            }
        }
    }
    for func_name in iface.functions.keys() {
        assert!(
            !suspicious(func_name),
            "function '{func_name}' looks like it handles key material"
        );
    }
}

/// `all_active` already excludes both flags from the loop's own work
/// list, so a pass over either instance never runs at
/// all -- proven from the outside by the alert `reconcile_instance_
/// pass` would otherwise raise: `plan_json_one_service(..., None)` has
/// no placement, which every other alert test in this file uses as
/// the cheapest fixture that opens `InstanceNotRunning` the moment a
/// pass actually processes the instance.
#[tokio::test]
async fn the_loop_skips_paused_and_retired_instances() {
    let s = service();
    let paused_plan = plan_json_one_service("paused-inst", "backend", None);
    let retired_plan = plan_json_one_service("retired-inst", "backend", None);
    s.store.submit("paused-inst", &paused_plan, "{}", "did:key:owner", 0).unwrap();
    s.store.submit("retired-inst", &retired_plan, "{}", "did:key:owner", 0).unwrap();
    s.store.pause("paused-inst").unwrap();
    s.store.retire("retired-inst").unwrap();

    s.run_pass().await;

    assert!(s.store.alerts.active(&AppInstanceId::new("paused-inst")).unwrap().is_empty());
    assert!(s.store.alerts.active(&AppInstanceId::new("retired-inst")).unwrap().is_empty());
}

/// Review finding A-8: `last_reconciled_at` used to be hardcoded
/// `None` forever, under a stale comment claiming no loop existed to
/// fill it. A paused/retired instance is skipped before the health
/// sweep even runs (see the test above), so it must stay unstamped;
/// a plain instance with an empty service list still gets a full
/// pass (the health sweep and the diff both run over zero services)
/// and must be stamped by it.
#[tokio::test]
async fn a_loop_pass_stamps_last_reconciled_at_but_a_skipped_instance_is_untouched() {
    let s = service();
    let plan = plan_json_no_services("inst-1");
    let paused_plan = plan_json_no_services("paused-inst");
    s.store.submit("inst-1", &plan, "{}", "did:key:owner", 0).unwrap();
    s.store.submit("paused-inst", &paused_plan, "{}", "did:key:owner", 0).unwrap();
    s.store.pause("paused-inst").unwrap();

    s.run_pass().await;

    assert!(s.last_reconciled.contains_key("inst-1"));
    assert!(!s.last_reconciled.contains_key("paused-inst"));
}

/// `apply_write_phase` is the write phase `reconcile_instance_pass`
/// calls after its health sweep -- this tests its own re-read
/// directly, standing in for a `pause` that lands during the sweep
/// (which does not hold the per-instance lock a pass otherwise holds
/// for its whole duration). If the write phase used the state the pass
/// started with instead of re-reading, this would append a journal
/// record; it must not.
#[tokio::test]
async fn a_pause_landing_mid_pass_stops_that_passs_writes() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", None);
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    let needs_work: BTreeSet<String> = ["inst-1/backend".to_string()].into_iter().collect();

    // The pause lands before the write phase's own re-read -- exactly
    // the F6 window, simulated directly rather than raced.
    s.store.pause("inst-1").unwrap();

    s.apply_write_phase(WritePhase {
        instance_id: &AppInstanceId::new("inst-1"),
        app_instance_id: "inst-1",
        plan: &plan,
        needs_work: &needs_work,
        restart_candidates: &[],
        renewal_candidates: &[],
        pending_rotation_restarts: &BTreeSet::new(),
        push_candidates: &[],
        schedule_decisions: &[],
        did_to_alias: &BTreeMap::new(),
        clients: &BTreeMap::new(),
        now: 0,
    })
    .await;

    assert!(
        s.store.journal.get_latest(&AppInstanceId::new("inst-1")).unwrap().is_none(),
        "a paused instance must not have had a deploy attempted"
    );
}

/// Review finding A-7: a record left `Applying` by a process that
/// crashed between `journal.append` and `journal.update_state` must
/// not pin `handle_status` to "Applying" forever. The per-instance
/// lock a pass holds is what makes this safe to recover on sight --
/// nothing can genuinely still be applying for this instance while
/// the pass itself holds that lock.
#[tokio::test]
async fn a_pass_recovers_a_deployment_record_stuck_in_applying() {
    let s = service();
    let plan_json = plan_json_no_services("inst-1");
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    s.store.journal.append(&plan, DeploymentState::Applying).unwrap();

    s.reconcile_instance_pass("inst-1").await;

    let latest = s.store.journal.get_latest(&AppInstanceId::new("inst-1")).unwrap().unwrap();
    assert_eq!(latest.state, DeploymentState::Degraded);
}

/// A placement change was already refused, but nothing raised an alert
/// for it -- only `Display`/`FromStr` ever touched the variant. A
/// refusal must now be visible on `alerts`, not only as this call's
/// own `Err`.
#[tokio::test]
async fn refuse_placement_change_raises_and_stores_placement_change_refused() {
    let s = service();
    let landed_plan =
        DeploymentPlan::from_json(&plan_json_one_service("inst-1", "backend", Some("edge-1")))
            .unwrap();
    let deployment_id = s.store.journal.append(&landed_plan, DeploymentState::Active).unwrap();
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

    let moved_plan =
        DeploymentPlan::from_json(&plan_json_one_service("inst-1", "backend", Some("edge-2")))
            .unwrap();
    let inventory = SupervisorInventory::from([(
        "edge-2".to_string(),
        SupervisorInventoryEntry { did: "did:key:zEdge2".to_string(), api_url: None, ucan: None },
    )]);

    let err = s.refuse_placement_change(&moved_plan, &inventory).await.unwrap_err();
    assert!(err.contains("does not relocate"), "{err}");

    let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
    assert!(alerts.iter().any(|a| a.kind == AlertKind::PlacementChangeRefused), "{alerts:?}");
}

/// `refuse_placement_change` used to compare a member's plan entry
/// against `current_placement(&landed, &l_ref)` keyed on the bare
/// logical ref, so with two members placed on different substrates,
/// member 1's entry was compared against member 0's landed row --
/// different DIDs, refused as a relocation though nothing moved.
/// Keying on `member_ref()` is what makes cross-substrate `replicas`
/// even expressible.
#[tokio::test]
async fn a_second_member_placed_on_a_different_substrate_is_not_refused_as_a_relocation() {
    let s = service();
    let landed_plan_json = serde_json::json!({
        "app_instance_id": "inst-1",
        "blueprint_id": "syneroym:test",
        "version": "1.0.0",
        "services": [
            {
                "service_id": "did:key:hFabricated0",
                "logical_ref": "inst-1/backend",
                "substrate": "edge-1",
                "service_type": "tcp", "source": "127.0.0.1:9000",
                "rotation_policy": "none",
                "resolved_dependencies": {},
                "topology_mode": "redundant",
                "member_index": 0
            },
            {
                "service_id": "did:key:hFabricated1",
                "logical_ref": "inst-1/backend",
                "substrate": "edge-2",
                "service_type": "tcp", "source": "127.0.0.1:9000",
                "rotation_policy": "none",
                "resolved_dependencies": {},
                "topology_mode": "redundant",
                "member_index": 1
            }
        ]
    })
    .to_string();
    let landed_plan = DeploymentPlan::from_json(&landed_plan_json).unwrap();
    let deployment_id = s.store.journal.append(&landed_plan, DeploymentState::Active).unwrap();
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
    s.store
        .journal
        .append_action(
            deployment_id,
            "ADD",
            "inst-1/backend#1",
            Some("edge-2"),
            "did:key:zEdge2",
            ActionState::Completed,
        )
        .unwrap();

    // The same plan resubmitted -- neither member's substrate changed,
    // but member 1 sits on a substrate distinct from member 0's, the
    // exact shape that used to compare it against the wrong sibling.
    let inventory = SupervisorInventory::from([
        (
            "edge-1".to_string(),
            SupervisorInventoryEntry {
                did: "did:key:zEdge1".to_string(),
                api_url: None,
                ucan: None,
            },
        ),
        (
            "edge-2".to_string(),
            SupervisorInventoryEntry {
                did: "did:key:zEdge2".to_string(),
                api_url: None,
                ucan: None,
            },
        ),
    ]);

    s.refuse_placement_change(&landed_plan, &inventory).await.unwrap();

    let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
    assert!(
        !alerts.iter().any(|a| a.kind == AlertKind::PlacementChangeRefused),
        "neither member actually moved: {alerts:?}"
    );
}

/// D-A5e-14: `SynAppManifest::validate()`'s cap is a compile-time
/// check on a manifest `submit`/`force-reconcile` never see -- they
/// take an already-compiled plan straight as JSON, so this is the
/// re-check at the interface that actually accepts one.
#[test]
fn refuse_replicas_above_cap_refuses_a_plan_naming_more_members_than_the_cap() {
    let services: Vec<PlannedService> = (0..=MAX_REPLICAS)
        .map(|i| {
            let mut svc = dependent_service("backend", "unrelated");
            svc.member_index = i;
            svc
        })
        .collect();
    let plan = DeploymentPlan {
        app_instance_id: AppInstanceId::new("inst-1"),
        blueprint_id: AppBlueprintId::new("syneroym:test"),
        version: semver::Version::new(1, 0, 0),
        services,
    };
    let err = SupervisorService::refuse_replicas_above_cap(&plan).unwrap_err();
    assert!(err.contains(&format!("above the cap of {MAX_REPLICAS}")), "{err}");
}

/// A plan naming exactly `MAX_REPLICAS` members is not refused --
/// only strictly above the cap is, matching `validate()`'s own rule.
#[test]
fn refuse_replicas_above_cap_allows_a_plan_exactly_at_the_cap() {
    let services: Vec<PlannedService> = (0..MAX_REPLICAS)
        .map(|i| {
            let mut svc = dependent_service("backend", "unrelated");
            svc.member_index = i;
            svc
        })
        .collect();
    let plan = DeploymentPlan {
        app_instance_id: AppInstanceId::new("inst-1"),
        blueprint_id: AppBlueprintId::new("syneroym:test"),
        version: semver::Version::new(1, 0, 0),
        services,
    };
    assert!(SupervisorService::refuse_replicas_above_cap(&plan).is_ok());
}

/// `update_superseded_alert` is the exact decision
/// `reconcile_instance_pass` gates its write phase on (`if superseded
/// { return }`, before `apply_write_phase` is ever reached) -- tested
/// directly, since driving a real higher `held_max` through a full
/// pass needs a live substrate actually reporting one. The "still
/// polls it" half is `an_instance_with_a_planned_service_that_never_
/// landed_reports_degraded` and this file's other alert tests: the
/// health sweep in `reconcile_instance_pass` always runs before this
/// check, unconditionally, so nothing about being superseded can
/// suppress it.
#[test]
fn the_loop_skips_every_write_for_a_superseded_instance_but_still_polls_it() {
    let s = service();
    let instance_id = AppInstanceId::new("inst-1");

    let superseded = s.update_superseded_alert(&instance_id, "inst-1", Some(5), 2).unwrap();
    assert!(superseded);
    assert!(
        s.store
            .alerts
            .active(&instance_id)
            .unwrap()
            .iter()
            .any(|a| a.kind == AlertKind::SupervisorSuperseded)
    );
}

/// The boundary `max_held_generation_from_clients`'s own doc names:
/// nothing reachable must not be confused with "reachable and behind"
/// -- `reconcile_instance_pass` never even computes a `Some` held-max
/// when every alias for this plan-only-placed instance is
/// unreachable (no clients ever connect, since the plan places
/// nothing), so `superseded` stays `false` and the pass is not
/// short-circuited: its health sweep still ran and raised
/// `InstanceNotRunning`.
#[tokio::test]
async fn an_unreachable_generation_read_does_not_mark_an_instance_superseded() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", None);
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

    s.reconcile_instance_pass("inst-1").await;

    let instance_id = AppInstanceId::new("inst-1");
    let active = s.store.alerts.active(&instance_id).unwrap();
    assert!(!active.iter().any(|a| a.kind == AlertKind::SupervisorSuperseded), "{active:?}");
    assert!(active.iter().any(|a| a.kind == AlertKind::InstanceNotRunning), "{active:?}");
}

/// Review finding C-3: D-A5c-12's poll-cost budget is "at most 2 RPCs
/// per substrate per pass" (one batched `status`, one
/// `app-instance-management-of`) -- the shipped budget test
/// (`orchestration.rs`) measures wall-clock duration only, and
/// nothing anywhere asserted the RPC-count half as a number that
/// could regress. This pins the `app-instance-management-of` half
/// directly: `max_held_generation_from_clients` must call
/// `held_generation` exactly once per *alias* (one call per
/// substrate), never once per service placed on it -- three aliases
/// here stand in for a substrate hosting many services, and the
/// count must stay 3, not grow with however many services this test
/// does not even bother placing. The "one batched status" half has
/// no equivalent unit seam (`SyneroymClient` is concrete, not
/// injectable into the health-poll path) and stays a duration-only
/// regression guard; recorded in the deferred backlog.
#[tokio::test]
async fn max_held_generation_from_clients_calls_held_generation_once_per_alias() {
    let actor = Arc::new(CountingActor::default());
    let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
    let aliases: BTreeSet<String> =
        ["edge-1", "edge-2", "edge-3"].into_iter().map(String::from).collect();
    let clients: BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>> =
        aliases.iter().map(|a| (SubstrateAlias::new(a.clone()), dyn_actor.clone())).collect();

    let held_max =
        SupervisorService::max_held_generation_from_clients("inst-1", &aliases, &clients).await;

    assert_eq!(held_max, Some(0));
    assert_eq!(*actor.held_generation_calls.lock().unwrap(), 3);
}

/// `instance_lock` itself, which two concurrently driven holders for
/// the *same* instance id must never both be inside at once.
/// `instance_lock` for two *different* ids would return two different
/// mutexes and is not what this proves. This pins the lock's own
/// mutual exclusion, not that `submit` and a loop pass actually reach
/// for it -- the two tests below drive the real methods.
#[tokio::test]
async fn a_submit_and_a_loop_pass_for_one_instance_do_not_interleave() {
    let s = service();
    let inside = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let overlapped = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let mut handles = Vec::new();
    for _ in 0..4 {
        let lock = s.instance_lock("inst-1");
        let inside = inside.clone();
        let overlapped = overlapped.clone();
        handles.push(tokio::spawn(async move {
            let _guard = lock.lock().await;
            if inside.fetch_add(1, std::sync::atomic::Ordering::SeqCst) != 0 {
                overlapped.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
            inside.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    assert!(!overlapped.load(std::sync::atomic::Ordering::SeqCst));
}

/// Review finding C-4: drives the real `run_pass` against a real
/// externally-held `instance_lock`, rather than four anonymous
/// holders of it -- proof that a loop pass genuinely blocks on the
/// same lock `instance_lock(app_instance_id)` returns, not merely
/// that the lock type is a working mutex.
#[tokio::test]
async fn a_loop_pass_blocks_on_this_instances_externally_held_lock() {
    let s = Arc::new(service());
    let plan_json = plan_json_no_services("inst-1");
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

    let held = s.instance_lock("inst-1");
    let guard = held.lock().await;

    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let pass_s = s.clone();
    let pass_done = done.clone();
    let handle = tokio::spawn(async move {
        pass_s.run_pass().await;
        pass_done.store(true, std::sync::atomic::Ordering::SeqCst);
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !done.load(std::sync::atomic::Ordering::SeqCst),
        "a loop pass must not proceed while this instance's lock is held elsewhere"
    );

    drop(guard);
    tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("the pass must proceed once the lock is released")
        .unwrap();
    assert!(done.load(std::sync::atomic::Ordering::SeqCst));
}

/// Review finding C-4's other half: `handle_submit` for the same
/// instance id must block on that instance's lock too, driven
/// through the real `dispatch("submit", …)` path rather than a
/// stand-in.
#[tokio::test]
async fn a_submit_blocks_on_this_instances_externally_held_lock() {
    let s = Arc::new(service());
    let plan_json = plan_json_no_services("inst-1");

    let held = s.instance_lock("inst-1");
    let guard = held.lock().await;

    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let submit_s = s.clone();
    let submit_done = done.clone();
    let submit_plan_json = plan_json.clone();
    let handle = tokio::spawn(async move {
        dispatch(
            &submit_s,
            admin_caller("did:key:zSupervisorNode"),
            "submit",
            serde_json::json!([{
                "app_instance_id": "inst-1",
                "plan_json": submit_plan_json,
                "inventory_json": "{}",
                "generation": 0,
            }]),
        )
        .await
        .unwrap();
        submit_done.store(true, std::sync::atomic::Ordering::SeqCst);
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !done.load(std::sync::atomic::Ordering::SeqCst),
        "submit must not proceed while this instance's lock is held elsewhere"
    );

    drop(guard);
    tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("submit must proceed once the lock is released")
        .unwrap();
    assert!(done.load(std::sync::atomic::Ordering::SeqCst));
}

/// The loop is spawned, not pinned in a `select!` that would drop it
/// mid-pass -- `shutdown` only cancels the token
/// (production's own `RuntimeServices` is what holds the
/// `JoinHandle`), so this test spawns and joins it the same way that
/// caller does, and asserts the join resolves promptly rather than
/// hanging or requiring a second cancellation.
#[tokio::test]
async fn shutdown_cancels_the_spawned_loop_and_waits_for_it_to_close_its_clients() {
    let s = Arc::new(service());
    let spawned = s.clone();
    let handle = tokio::spawn(async move { spawned.run().await });

    // Let the loop reach its first `interval.tick()` wait (the first
    // tick fires immediately and `run_pass` over an empty store
    // returns at once).
    tokio::time::sleep(Duration::from_millis(20)).await;

    s.shutdown().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("the spawned loop did not stop within 2s of shutdown")
        .unwrap()
        .unwrap();
}

/// Pins `run`'s interval configuration directly, under a paused clock
/// rather than a real slow pass -- `Skip` must let a
/// tick that arrives long after a missed period fire once,
/// immediately, rather than the default `Burst` behavior firing once
/// per period that elapsed.
#[tokio::test(start_paused = true)]
async fn a_pass_that_outruns_the_interval_does_not_queue_a_burst() {
    let mut interval = SupervisorService::build_pass_interval(1);
    interval.tick().await;

    // Ten missed periods' worth of virtual time elapses while a pass
    // is imagined to be still running.
    tokio::time::advance(Duration::from_secs(10)).await;

    let before_catchup = tokio::time::Instant::now();
    interval.tick().await;
    assert_eq!(
        tokio::time::Instant::now(),
        before_catchup,
        "a Skip interval must resolve the missed ticks immediately, not wait out each one"
    );

    let after_catchup = tokio::time::Instant::now();
    interval.tick().await;
    assert!(
        tokio::time::Instant::now() >= after_catchup + Duration::from_secs(1),
        "no burst of queued ticks should remain after catching up once"
    );
}

/// A landed service the sweep finds `InstanceNotRunning` gets one
/// bounded restart attempt.
#[tokio::test]
async fn instance_not_running_triggers_a_restart_on_the_next_pass() {
    let s = service();
    let actor = Arc::new(CountingActor::default());
    let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
    let mut opened = Vec::new();
    s.attempt_restart(
        &AppInstanceId::new("inst-1"),
        "inst-1",
        "inst-1/backend",
        "did:key:hBackend",
        "did:key:zEdge1",
        &dyn_actor,
        0,
        1_000,
        &mut opened,
    )
    .await;

    assert_eq!(*actor.restart_calls.lock().unwrap(), 1);
    let state = s.store.remediation_state("inst-1", "inst-1/backend").unwrap().unwrap();
    assert_eq!(state.attempts, 1);
    assert!(!state.terminal);
    assert!(opened.is_empty(), "one attempt must not exhaust a 3-attempt budget");
}

/// Two members of one scaled service must each spend their own
/// `max_restart_attempts` budget -- `restart_candidates` keys on
/// `ServiceHealth::member_ref()` (member 0 and member 1 are two
/// distinct candidates), and `attempt_restart`'s remediation row is
/// keyed on that same string.
/// A regression back to a bare logical ref would collapse the two
/// into one shared counter -- member 1's failures exhausting member
/// 0's budget, and vice versa.
#[tokio::test]
async fn restart_attempts_are_counted_per_member_not_per_logical_service() {
    let s = service();
    let report = report_of(vec![
        {
            let mut h = service_health(
                "inst-1/backend",
                "did:key:zEdge1",
                Signal::InstanceNotRunning(String::new()),
            );
            h.member_index = 0;
            h
        },
        {
            let mut h = service_health(
                "inst-1/backend",
                "did:key:zEdge2",
                Signal::InstanceNotRunning(String::new()),
            );
            h.member_index = 1;
            h
        },
    ]);
    let candidates = SupervisorService::restart_candidates(&report);
    assert_eq!(
        candidates.iter().map(|(l_ref, ..)| l_ref.as_str()).collect::<BTreeSet<_>>(),
        BTreeSet::from(["inst-1/backend#0", "inst-1/backend#1"]),
        "two members must be two distinct restart candidates: {candidates:?}"
    );

    let actor = Arc::new(CountingActor::default());
    let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
    let instance_id = AppInstanceId::new("inst-1");
    let mut opened = Vec::new();
    // Member 0 spends its whole 3-attempt budget (the fixture's
    // default), well past its own backoff each time.
    for now in [1_000u64, 1_100u64, 1_200u64] {
        s.attempt_restart(
            &instance_id,
            "inst-1",
            "inst-1/backend#0",
            "did:key:hbackend0",
            "did:key:zEdge1",
            &dyn_actor,
            0,
            now,
            &mut opened,
        )
        .await;
    }
    let member0 = s.store.remediation_state("inst-1", "inst-1/backend#0").unwrap().unwrap();
    assert_eq!(member0.attempts, 3);
    assert!(member0.terminal, "member 0 must be exhausted after 3 attempts");

    // Member 1 has never been attempted -- its own row must still
    // read fresh, not inherit member 0's exhausted state.
    let member1 = s.store.remediation_state("inst-1", "inst-1/backend#1").unwrap();
    assert!(member1.is_none(), "member 1 must have its own, untouched remediation row");

    let mut opened1 = Vec::new();
    s.attempt_restart(
        &instance_id,
        "inst-1",
        "inst-1/backend#1",
        "did:key:hbackend1",
        "did:key:zEdge2",
        &dyn_actor,
        0,
        1_000,
        &mut opened1,
    )
    .await;
    let member1 = s.store.remediation_state("inst-1", "inst-1/backend#1").unwrap().unwrap();
    assert_eq!(member1.attempts, 1, "member 1's first attempt must not be refused as terminal");
    assert!(!member1.terminal);
}

/// `restart_backoff_secs` (30 in the fixture, D-A5c-14's table): a
/// second attempt inside that window is refused before the actor is
/// ever called again.
#[tokio::test]
async fn a_restart_is_not_retried_before_the_backoff_elapses() {
    let s = service();
    let actor = Arc::new(CountingActor::default());
    let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
    let mut opened = Vec::new();
    for now in [1_000u64, 1_010u64] {
        s.attempt_restart(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            "inst-1/backend",
            "did:key:hBackend",
            "did:key:zEdge1",
            &dyn_actor,
            0,
            now,
            &mut opened,
        )
        .await;
    }
    assert_eq!(*actor.restart_calls.lock().unwrap(), 1, "the second attempt was inside backoff");
    assert_eq!(s.store.remediation_state("inst-1", "inst-1/backend").unwrap().unwrap().attempts, 1);
}

/// Matrix row 13: exceeding `max_restart_attempts` (3 in the fixture)
/// marks the service terminal and raises `RemediationExhausted`
/// exactly once, on the attempt that crosses the ceiling.
#[tokio::test]
async fn remediation_stops_after_max_attempts_and_alerts_once() {
    let s = service();
    let actor = Arc::new(CountingActor::default());
    let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
    let mut opened = Vec::new();
    // Each attempt spaced past `restart_backoff_secs` (30) so none is
    // refused for being too soon.
    for i in 0..3u64 {
        s.attempt_restart(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            "inst-1/backend",
            "did:key:hBackend",
            "did:key:zEdge1",
            &dyn_actor,
            0,
            1_000 + i * 100,
            &mut opened,
        )
        .await;
    }
    assert_eq!(*actor.restart_calls.lock().unwrap(), 3);
    let state = s.store.remediation_state("inst-1", "inst-1/backend").unwrap().unwrap();
    assert_eq!(state.attempts, 3);
    assert!(state.terminal);
    assert_eq!(
        opened.iter().filter(|(k, _)| *k == AlertKind::RemediationExhausted).count(),
        1,
        "{opened:?}"
    );
}

/// Row 13's other half: once terminal, a later pass's attempt must
/// not call the actor again, however long it has been.
#[tokio::test]
async fn a_terminal_degraded_service_is_never_restarted_again() {
    let s = service();
    let actor = Arc::new(CountingActor::default());
    let dyn_actor: Arc<dyn SubstrateActor> = actor.clone();
    let mut opened = Vec::new();
    for i in 0..3u64 {
        s.attempt_restart(
            &AppInstanceId::new("inst-1"),
            "inst-1",
            "inst-1/backend",
            "did:key:hBackend",
            "did:key:zEdge1",
            &dyn_actor,
            0,
            1_000 + i * 100,
            &mut opened,
        )
        .await;
    }
    assert!(s.store.remediation_state("inst-1", "inst-1/backend").unwrap().unwrap().terminal);

    s.attempt_restart(
        &AppInstanceId::new("inst-1"),
        "inst-1",
        "inst-1/backend",
        "did:key:hBackend",
        "did:key:zEdge1",
        &dyn_actor,
        0,
        1_000_000,
        &mut opened,
    )
    .await;
    assert_eq!(
        *actor.restart_calls.lock().unwrap(),
        3,
        "a terminal service must not be restarted again"
    );
}

/// Review finding C-2: tests 35-38 (above) call `attempt_restart`
/// directly, and 39-40 (below) test `restart_candidates` in
/// isolation -- nothing drove the wiring between them, the
/// `did_to_alias -> clients -> actor` lookup inside
/// `apply_write_phase` that a mis-keyed alias or DID would silently
/// `continue` past with no restart and no error. Uses a real,
/// never-connected `SyneroymClient` rather than a fake: its `restart`
/// fails fast with "Not connected" (no socket, no hang), which is
/// enough to prove the lookup found it and called it -- the point
/// here is the wiring, not the RPC outcome, which `attempt_restart`'s
/// own tests already cover.
#[tokio::test]
async fn a_restart_candidate_reaches_the_actor_through_apply_write_phases_own_lookup() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();

    let identity = Identity::generate().unwrap();
    let client = Arc::new(SyneroymClient::new_with_identity(
        "did:key:zEdge1".to_string(),
        String::new(),
        identity,
    ));
    let clients: BTreeMap<SubstrateAlias, Arc<SyneroymClient>> =
        BTreeMap::from([(SubstrateAlias::new("edge-1"), client)]);
    let did_to_alias: BTreeMap<String, String> =
        BTreeMap::from([("did:key:zEdge1".to_string(), "edge-1".to_string())]);
    let restart_candidates = vec![(
        "inst-1/backend".to_string(),
        "did:key:hFabricated".to_string(),
        "did:key:zEdge1".to_string(),
    )];

    s.apply_write_phase(WritePhase {
        instance_id: &AppInstanceId::new("inst-1"),
        app_instance_id: "inst-1",
        plan: &plan,
        needs_work: &BTreeSet::new(),
        restart_candidates: &restart_candidates,
        renewal_candidates: &[],
        pending_rotation_restarts: &BTreeSet::new(),
        push_candidates: &[],
        schedule_decisions: &[],
        did_to_alias: &did_to_alias,
        clients: &clients,
        now: 0,
    })
    .await;

    let state = s.store.remediation_state("inst-1", "inst-1/backend").unwrap();
    assert!(
        state.is_some_and(|r| r.attempts == 1),
        "the candidate must reach a real actor call and record an attempt, not be silently \
         dropped by the did_to_alias/clients lookup: {state:?}"
    );
}

/// A declared readiness probe failing is an author assertion this
/// supervisor cannot verify, not a substrate-verified fact -- alert
/// only, pinned so a later change cannot silently widen remediation
/// onto it.
#[test]
fn probe_failing_never_triggers_a_restart() {
    let report = health::HealthReport {
        substrates: Vec::new(),
        services: vec![service_health(
            "inst-1/backend",
            "did:key:zEdge1",
            Signal::ProbeFailing("readiness check failing".to_string()),
        )],
    };
    assert!(SupervisorService::restart_candidates(&report).is_empty());
}

/// A substrate that did not answer is never inferred to mean its
/// services are down, so restarting cannot be the fix for it either.
#[test]
fn substrate_unreachable_never_triggers_a_restart() {
    let report = health::HealthReport {
        substrates: Vec::new(),
        services: vec![service_health(
            "inst-1/backend",
            "did:key:zEdge1",
            Signal::SubstrateUnreachable("no answer".to_string()),
        )],
    };
    assert!(SupervisorService::restart_candidates(&report).is_empty());
}

/// A service the resubmitted plan no longer names, but that this
/// supervisor's own journal still shows
/// landed, is reported -- not undeployed. Undeploying a stateful
/// service because a manifest was edited is destructive, and
/// `retire` is deliberately not a teardown.
#[tokio::test]
async fn a_service_dropped_from_the_plan_raises_orphaned_service_and_is_not_undeployed() {
    let s = service();
    let old_plan_json = plan_json_two_services("inst-1", "backend", "frontend");
    let old_plan = DeploymentPlan::from_json(&old_plan_json).unwrap();
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

    // Resubmitted plan drops `frontend`.
    let new_plan_json = plan_json_one_service("inst-1", "backend", None);
    s.store.submit("inst-1", &new_plan_json, "{}", "did:key:owner", 0).unwrap();

    s.reconcile_instance_pass("inst-1").await;

    let instance_id = AppInstanceId::new("inst-1");
    let active = s.store.alerts.active(&instance_id).unwrap();
    let orphan = active
        .iter()
        .find(|a| a.kind == AlertKind::OrphanedService)
        .unwrap_or_else(|| panic!("no OrphanedService alert among {active:?}"));
    assert_eq!(orphan.logical_ref.as_deref(), Some("inst-1/frontend#0"));
    assert_eq!(orphan.substrate_did, "did:key:zEdge1");
}
