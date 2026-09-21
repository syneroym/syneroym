use super::*;

#[test]
fn submitting_twice_replaces_desired_state_and_keeps_one_row() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.submit("inst-1", "{\"v\":1}", "{}", "did:key:owner", 0).unwrap();
    store.submit("inst-1", "{\"v\":2}", "{}", "did:key:owner", 0).unwrap();

    let state = store.get("inst-1").unwrap().unwrap();
    assert_eq!(state.plan_json, "{\"v\":2}");

    let conn = store.conn.lock().unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM desired_state WHERE app_instance_id = 'inst-1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

/// The queue's tables live in the same database file `desired_state`
/// does, not one of their own, so they inherit its protection posture
/// rather than becoming a second unencrypted store beside it.
#[test]
fn the_queue_lives_in_supervisor_db_under_the_same_protection_as_desired_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
    store.queue.enqueue("g", "k", b"payload", 1_000).unwrap();
    // A schedule's durable state is a table in this same file, not a
    // store of its own.
    store.record_schedule_evaluated("inst-1", "inst-1/worker", 1_000).unwrap();

    assert!(
        !dir.path().join("supervisor.db-outbox").exists()
            && !dir.path().join("queue.db").exists()
            && !dir.path().join("supervisor.db-schedules").exists(),
        "the queue and the schedule store must not open a database file of their own"
    );
    let conn = store.conn.lock().unwrap();
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 1, "the outbox table must live in supervisor.db's own connection");
    let scheduled: i64 =
        conn.query_row("SELECT COUNT(*) FROM scheduled_runs", [], |r| r.get(0)).unwrap();
    assert_eq!(
        scheduled, 1,
        "the scheduled_runs table must live in supervisor.db's own connection"
    );
}

/// Before the generation check, `submit` wrote whatever generation it
/// was given straight over the stored one, so a caller could take an
/// instance by submitting a large number rather than adopting --
/// `adopt` was not really the only minter (ADR-0021 §4). A resubmit at
/// the *current* generation must still
/// work (`submitting_twice_replaces_desired_state_and_keeps_one_row`
/// above pins that at generation 0).
#[test]
fn submit_refuses_a_generation_that_does_not_match_the_one_on_record() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.submit("inst-1", "{}", "{}", "did:key:owner", 0).unwrap();
    store.set_generation("inst-1", 3).unwrap();

    let err = store.submit("inst-1", "{}", "{}", "did:key:owner", 4).unwrap_err();
    assert!(err.to_string().contains("generation"), "{err}");
    let err = store.submit("inst-1", "{}", "{}", "did:key:owner", 0).unwrap_err();
    assert!(err.to_string().contains("generation"), "{err}");

    store.submit("inst-1", "{\"v\":2}", "{}", "did:key:owner", 3).unwrap();
    assert_eq!(store.get("inst-1").unwrap().unwrap().plan_json, "{\"v\":2}");
}

#[test]
fn pause_and_resume_round_trip() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.submit("inst-1", "{}", "{}", "did:key:owner", 0).unwrap();
    assert!(!store.get("inst-1").unwrap().unwrap().paused);

    store.pause("inst-1").unwrap();
    assert!(store.get("inst-1").unwrap().unwrap().paused);

    store.resume("inst-1").unwrap();
    assert!(!store.get("inst-1").unwrap().unwrap().paused);
}

/// The doc comment on `all_active` promises "every non-retired,
/// non-paused instance", but the query used to filter `retired` only
/// -- a live bug the moment the resident loop uses this as its work
/// list.
#[test]
fn all_active_excludes_both_retired_and_paused_instances() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.submit("running", "{}", "{}", "did:key:owner", 0).unwrap();
    store.submit("paused", "{}", "{}", "did:key:owner", 0).unwrap();
    store.pause("paused").unwrap();
    store.submit("retired", "{}", "{}", "did:key:owner", 0).unwrap();
    store.retire("retired").unwrap();

    let active: Vec<String> =
        store.all_active().unwrap().into_iter().map(|s| s.app_instance_id).collect();
    assert_eq!(active, vec!["running".to_string()]);
}

#[test]
fn retire_refuses_a_later_submit_until_un_retired() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.submit("inst-1", "{}", "{}", "did:key:owner", 0).unwrap();
    store.retire("inst-1").unwrap();
    assert!(store.get("inst-1").unwrap().unwrap().retired);

    let err = store.submit("inst-1", "{}", "{}", "did:key:owner", 1).unwrap_err();
    assert!(err.to_string().contains("retired"), "{err}");

    // `retire` is not a dead end -- `record_adopt` (called by
    // `handle_adopt` on a successful claim) un-retires as part of its
    // combined write, which is the "run `supervisor adopt`" the
    // refusal above names.
    store.record_adopt("inst-1", 0, "did:key:zAppMaster").unwrap();
    assert!(!store.get("inst-1").unwrap().unwrap().retired);
    store.submit("inst-1", "{\"v\":2}", "{}", "did:key:owner", 0).unwrap();
    assert_eq!(store.get("inst-1").unwrap().unwrap().plan_json, "{\"v\":2}");
}

// ── Binding epochs ─────────────────────────────────────────────────

/// The epoch is held **per dependent service**, not per dependency:
/// there is only one row for
/// "frontend", so every dependency it declares reads the identical
/// value -- there is no per-dependency key to diverge in the first
/// place.
#[test]
fn a_written_epoch_is_held_per_dependent_and_shared_by_its_dependencies() {
    let store = SupervisorStore::open_in_memory().unwrap();
    let epoch = store.advance_binding_epoch("inst-1", "inst-1/frontend").unwrap();
    assert_eq!(epoch, 1);
    assert_eq!(store.binding_epoch("inst-1", "inst-1/frontend").unwrap(), 1);
}

/// The first draft's scalar `ApplyRequest.epoch` let a redeploy write a
/// *lower* epoch over a higher one a push had already reached, so the
/// next push then conflicted at an epoch the substrate had already
/// served. The counter must only ever advance.
#[test]
fn a_redeploy_after_a_push_carries_an_epoch_above_what_was_pushed() {
    let store = SupervisorStore::open_in_memory().unwrap();
    let pushed = store.advance_binding_epoch("inst-1", "inst-1/frontend").unwrap();
    let redeployed = store.advance_binding_epoch("inst-1", "inst-1/frontend").unwrap();
    assert!(redeployed > pushed, "redeploy epoch {redeployed} must exceed push epoch {pushed}");
}

/// An instance this supervisor has never
/// written a binding for must read epoch 0, which is also what a
/// hand-deployed substrate reports (`roymctl app deploy` emits every
/// binding at epoch 0) -- so the pair reads converged, not stale.
#[test]
fn an_absent_row_reads_as_epoch_zero_so_a_hand_deployed_binding_converges() {
    let store = SupervisorStore::open_in_memory().unwrap();
    assert_eq!(store.binding_epoch("inst-1", "inst-1/frontend").unwrap(), 0);
}

// ── Remediation bookkeeping ────────────────────────────────────────

/// A supervisor restart must resume remediation state, not reset every
/// service's attempt count back to zero and re-earn the same restart
/// budget again.
#[test]
fn remediation_attempts_survive_a_store_reopen() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
        store.record_restart_attempt("inst-1", "inst-1/backend", 1000).unwrap();
        store.record_restart_attempt("inst-1", "inst-1/backend", 1030).unwrap();
    }
    let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
    let state = store.remediation_state("inst-1", "inst-1/backend").unwrap().unwrap();
    assert_eq!(state.attempts, 2);
    assert_eq!(state.last_attempt_at, Some(1030));
    assert!(!state.terminal);
}

#[test]
fn a_healthy_sweep_clears_the_remediation_row() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.record_restart_attempt("inst-1", "inst-1/backend", 1000).unwrap();
    assert!(store.remediation_state("inst-1", "inst-1/backend").unwrap().is_some());

    store.clear_remediation("inst-1", "inst-1/backend").unwrap();
    assert!(store.remediation_state("inst-1", "inst-1/backend").unwrap().is_none());
}

/// `force-reconcile` is the escape hatch from a
/// terminal remediation row -- a service nothing will restart again
/// cannot become healthy on its own, so the healthy-sweep clearing
/// path above never fires for it.
#[test]
fn force_reconcile_clears_a_terminal_remediation_row() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.record_restart_attempt("inst-1", "inst-1/backend", 1000).unwrap();
    store.mark_remediation_terminal("inst-1", "inst-1/backend").unwrap();
    assert!(store.remediation_state("inst-1", "inst-1/backend").unwrap().unwrap().terminal);

    // `force-reconcile`'s own clearing path: every remediation row for
    // the instance, not just this one service.
    store.clear_remediation_for_instance("inst-1").unwrap();
    assert!(store.remediation_state("inst-1", "inst-1/backend").unwrap().is_none());
}

// ── Anchor-refresh bookkeeping and revoked placements ──────────────

/// The refresh cadence is evaluated against this persisted fact on the
/// ordinary pass tick rather than by a timer of its own, so the fact
/// has to survive a supervisor restart -- otherwise every restart
/// republishes every anchor immediately, which is load without
/// benefit.
#[test]
fn store_persists_and_reads_back_last_refreshed_at_per_master() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
        assert_eq!(
            store.last_master_anchor_refresh("did:key:zMasterA").unwrap(),
            None,
            "a master this supervisor has never published for must read as overdue"
        );
        store.record_master_anchor_refresh("did:key:zMasterA", 1_000).unwrap();
        store.record_master_anchor_refresh("did:key:zMasterB", 2_000).unwrap();
        store.record_master_anchor_refresh("did:key:zMasterA", 3_000).unwrap();
    }
    let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
    assert_eq!(store.last_master_anchor_refresh("did:key:zMasterA").unwrap(), Some(3_000));
    assert_eq!(store.last_master_anchor_refresh("did:key:zMasterB").unwrap(), Some(2_000));
}

/// D-A5d-15: the revocation exclusion is a persisted fact because more
/// than one caller reads it -- the renewal work-list, `submit`, and
/// `force-reconcile` -- and because it must outlive the process that
/// recorded it.
#[test]
fn a_revoked_placement_is_recorded_per_member_and_survives_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
        assert!(store.revoked_placements("inst-1").unwrap().is_empty());
        store.revoke_placement("inst-1", "inst-1/backend", 1_000).unwrap();
        // Idempotent, and scoped to the one member named.
        store.revoke_placement("inst-1", "inst-1/backend", 2_000).unwrap();
        store.revoke_placement("inst-2", "inst-2/backend", 1_000).unwrap();
    }
    let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
    assert_eq!(
        store.revoked_placements("inst-1").unwrap(),
        BTreeSet::from(["inst-1/backend".to_string()])
    );
    assert_eq!(
        store.revoked_placements("inst-2").unwrap(),
        BTreeSet::from(["inst-2/backend".to_string()])
    );
}

/// F5 / D-A5c-20's second clearing path: `adopt` mints a new
/// generation, a fresh start by construction, so it clears the same
/// way `force-reconcile` does.
#[test]
fn adopt_clears_a_terminal_remediation_row() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.record_restart_attempt("inst-1", "inst-1/backend", 1000).unwrap();
    store.mark_remediation_terminal("inst-1", "inst-1/backend").unwrap();

    // `adopt`'s clearing path is the identical store method --
    // both verbs mean "start over", so both call it.
    store.clear_remediation_for_instance("inst-1").unwrap();
    assert!(store.remediation_state("inst-1", "inst-1/backend").unwrap().is_none());
}

// ── The app master column ──────────────────────────────────────────

/// `CREATE TABLE IF NOT EXISTS` is a no-op on a database that already
/// has `desired_state`, so a column added only there never reaches a
/// pre-existing file -- every `desired_state` read then fails at
/// runtime with "no such column". Opens a store, drops the column back
/// out (simulating a database that predates the column), reopens, and
/// confirms the idempotent `ALTER TABLE` puts it back.
#[test]
fn a_database_that_predates_the_app_master_column_gains_it_on_open() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
        store.submit("inst-1", "{}", "{}", "did:key:owner", 0).unwrap();
        let conn = store.conn.lock().unwrap();
        conn.execute("ALTER TABLE desired_state DROP COLUMN app_master_did", [])
            .expect("this rusqlite's bundled sqlite must support DROP COLUMN");
    }
    // Reopening must not fail, and the reader must see the column back,
    // defaulted empty for the row that predates it.
    let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
    let state = store.get("inst-1").unwrap().unwrap();
    assert_eq!(state.app_master_did, "");
}

/// An `app-<instance>` vault key must never be forgotten -- the same
/// standing constraint that already holds for member masters. Covers
/// the two store-level paths that could
/// plausibly clear it: a later `submit` (whose `ON CONFLICT` update
/// list must leave `app_master_did` out) and `retire` (a `set_flag`
/// call touching only its own column). `release` needs no case here --
/// `SupervisorService::handle_release` never writes to `desired_state`
/// at all, so it cannot clear this column any more than it clears
/// anything else on the row.
#[test]
fn a_recorded_app_master_survives_a_resubmit_and_a_retire() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.submit("inst-1", "{\"v\":1}", "{}", "did:key:owner", 0).unwrap();
    store.record_adopt("inst-1", 0, "did:key:zAppMaster").unwrap();
    assert_eq!(store.get("inst-1").unwrap().unwrap().app_master_did, "did:key:zAppMaster");

    // A later submit at the current generation replaces the plan but
    // must leave the app master alone.
    store.submit("inst-1", "{\"v\":2}", "{}", "did:key:owner", 0).unwrap();
    let state = store.get("inst-1").unwrap().unwrap();
    assert_eq!(state.plan_json, "{\"v\":2}");
    assert_eq!(state.app_master_did, "did:key:zAppMaster");

    store.retire("inst-1").unwrap();
    assert_eq!(store.get("inst-1").unwrap().unwrap().app_master_did, "did:key:zAppMaster");
}

/// `handle_adopt`'s combined write -- the
/// generation, the un-retired flag, and the app master DID all land
/// in one statement, so there is no window where a crash could leave
/// a claimed generation with no recorded DID.
#[test]
fn record_adopt_writes_generation_retired_and_app_master_did_together() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.submit("inst-1", "{}", "{}", "did:key:owner", 0).unwrap();
    store.retire("inst-1").unwrap();
    assert!(store.get("inst-1").unwrap().unwrap().retired);

    store.record_adopt("inst-1", 3, "did:key:zAppMaster").unwrap();

    let state = store.get("inst-1").unwrap().unwrap();
    assert_eq!(state.generation, 3);
    assert!(!state.retired, "record_adopt must also un-retire, like un_retire did");
    assert_eq!(state.app_master_did, "did:key:zAppMaster");
}

#[test]
fn record_adopt_fails_for_an_instance_with_no_desired_state() {
    let store = SupervisorStore::open_in_memory().unwrap();
    let err = store.record_adopt("never-submitted", 1, "did:key:zAppMaster").unwrap_err();
    assert!(err.to_string().contains("no desired state"), "{err}");
}

// ── `scheduled_runs` ─────────────────────────────────────────────────

#[test]
fn a_schedule_with_no_state_row_reads_as_absent() {
    let store = SupervisorStore::open_in_memory().unwrap();
    assert!(store.schedule_states("inst-1").unwrap().is_empty());
}

#[test]
fn record_schedule_evaluated_advances_only_the_watermark() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.record_schedule_evaluated("inst-1", "inst-1/worker", 100).unwrap();
    let states = store.schedule_states("inst-1").unwrap();
    let state = states.get("inst-1/worker").unwrap();
    assert_eq!(state.evaluated_at, 100);
    assert_eq!(state.last_run_at, None);
    assert_eq!(
        state.last_member_index, None,
        "a watermark-only pass has picked no member, which is not the same as member 0"
    );

    store.record_schedule_evaluated("inst-1", "inst-1/worker", 200).unwrap();
    let states = store.schedule_states("inst-1").unwrap();
    let state = states.get("inst-1/worker").unwrap();
    assert_eq!(state.evaluated_at, 200, "a later evaluation must advance the watermark");
    assert_eq!(state.last_run_at, None, "a watermark-only pass must not fabricate a run");
}

#[test]
fn record_schedule_started_writes_the_watermark_run_time_and_member_together() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.record_schedule_started("inst-1", "inst-1/worker", 100, 2).unwrap();
    let states = store.schedule_states("inst-1").unwrap();
    let state = states.get("inst-1/worker").unwrap();
    assert_eq!(state.evaluated_at, 100);
    assert_eq!(state.last_run_at, Some(100));
    assert_eq!(state.last_member_index, Some(2));
}

#[test]
fn record_schedule_outcome_sets_and_clears_the_last_error_only() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.record_schedule_started("inst-1", "inst-1/worker", 100, 0).unwrap();

    store.record_schedule_outcome("inst-1", "inst-1/worker", Some("boom")).unwrap();
    let states = store.schedule_states("inst-1").unwrap();
    let state = states.get("inst-1/worker").unwrap();
    assert_eq!(state.last_error.as_deref(), Some("boom"));
    assert_eq!(state.last_run_at, Some(100), "the outcome must not touch the run time");

    store.record_schedule_outcome("inst-1", "inst-1/worker", None).unwrap();
    let states = store.schedule_states("inst-1").unwrap();
    assert_eq!(states.get("inst-1/worker").unwrap().last_error, None);
}

/// A `force-reconcile` or `adopt` clears what this supervisor has
/// *done* about a schedule and keeps how far it has *looked*. Dropping
/// the watermark too would make the next pass treat the schedule as
/// first-sight and fire nothing, so a `force-reconcile` seconds after
/// an occurrence would silently swallow that tick.
#[test]
fn clear_schedule_state_keeps_the_watermark_and_drops_only_the_run_record() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.record_schedule_started("inst-1", "inst-1/worker", 100, 2).unwrap();
    store.record_schedule_outcome("inst-1", "inst-1/worker", Some("boom")).unwrap();
    store.record_schedule_started("inst-2", "inst-2/worker", 100, 1).unwrap();

    store.clear_schedule_state_for_instance("inst-1").unwrap();

    let states = store.schedule_states("inst-1").unwrap();
    let state = states.get("inst-1/worker").unwrap();
    assert_eq!(state.evaluated_at, 100, "the watermark is not per-run state");
    assert_eq!(state.last_run_at, None);
    assert_eq!(state.last_member_index, None);
    assert_eq!(state.last_error, None);

    let untouched = store.schedule_states("inst-2").unwrap();
    assert_eq!(untouched.get("inst-2/worker").unwrap().last_member_index, Some(1));
}

/// A `schedule` block dropped from a manifest leaves a row nothing else
/// ever deletes -- invisible, since `schedules` reads the plan, and
/// stale if the same logical service is given a schedule again later.
#[test]
fn prune_schedule_states_drops_only_the_schedules_the_plan_no_longer_declares() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.record_schedule_evaluated("inst-1", "inst-1/kept", 100).unwrap();
    store.record_schedule_evaluated("inst-1", "inst-1/dropped", 100).unwrap();
    store.record_schedule_evaluated("inst-2", "inst-2/kept", 100).unwrap();

    let declared = BTreeSet::from(["inst-1/kept".to_string()]);
    store.prune_schedule_states("inst-1", &declared).unwrap();

    let states = store.schedule_states("inst-1").unwrap();
    assert!(states.contains_key("inst-1/kept"));
    assert!(!states.contains_key("inst-1/dropped"));
    assert!(
        store.schedule_states("inst-2").unwrap().contains_key("inst-2/kept"),
        "pruning is scoped to the instance whose plan was read"
    );
}

/// The Tier-1 refresh fact is keyed by app DID, not by
/// `app_instance_id` -- one row per app master, independent of every
/// other app instance's own row.
#[test]
fn a_refresh_fact_is_recorded_per_app_did() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.record_tier1_refresh("did:key:zAppA", 100).unwrap();
    store.record_tier1_refresh("did:key:zAppB", 200).unwrap();

    assert_eq!(store.last_tier1_refresh("did:key:zAppA").unwrap(), Some(100));
    assert_eq!(store.last_tier1_refresh("did:key:zAppB").unwrap(), Some(200));

    store.record_tier1_refresh("did:key:zAppA", 150).unwrap();
    assert_eq!(
        store.last_tier1_refresh("did:key:zAppA").unwrap(),
        Some(150),
        "a later refresh replaces the stamp rather than adding a row"
    );
    assert_eq!(
        store.last_tier1_refresh("did:key:zAppB").unwrap(),
        Some(200),
        "refreshing one app DID must not touch another's row"
    );
}

/// An app DID this supervisor has never published a Tier-1 record for
/// reads as overdue, the same first-pass reading
/// `last_master_anchor_refresh` gives an unpublished master anchor.
#[test]
fn an_app_did_with_no_refresh_fact_is_due_immediately() {
    let store = SupervisorStore::open_in_memory().unwrap();
    assert_eq!(store.last_tier1_refresh("did:key:zNeverPublished").unwrap(), None);
}

// ── topology_epochs ────────────────────────────────────────

/// A first submit starts every service's topology epoch at 1.
#[test]
fn a_first_submit_starts_every_services_topology_epoch_at_one() {
    let store = SupervisorStore::open_in_memory().unwrap();
    assert_eq!(store.topology_epoch("inst-1", "backend").unwrap(), 0);
    let epoch = store.record_topology_fingerprint("inst-1", "backend", "fp-a").unwrap();
    assert_eq!(epoch, 1);
    assert_eq!(store.topology_epoch("inst-1", "backend").unwrap(), 1);
}

/// A resubmit whose fingerprint is unchanged leaves the epoch alone.
#[test]
fn a_resubmit_that_does_not_change_membership_leaves_the_epoch_alone() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.record_topology_fingerprint("inst-1", "backend", "fp-a").unwrap();
    let epoch = store.record_topology_fingerprint("inst-1", "backend", "fp-a").unwrap();
    assert_eq!(epoch, 1, "an unchanged fingerprint must not advance the epoch");
}

/// A resubmit that scales one service out advances only that service's
/// epoch.
#[test]
fn a_resubmit_that_scales_a_service_out_increments_only_that_services_epoch() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.record_topology_fingerprint("inst-1", "backend", "fp-a").unwrap();
    store.record_topology_fingerprint("inst-1", "frontend", "fp-b").unwrap();

    let epoch = store.record_topology_fingerprint("inst-1", "backend", "fp-a-scaled").unwrap();
    assert_eq!(epoch, 2);
    assert_eq!(
        store.topology_epoch("inst-1", "frontend").unwrap(),
        1,
        "an unrelated service's epoch must not move"
    );
}

/// A service removed and re-added later must not reuse a lower epoch.
/// There is no delete path, so this pins that re-recording the same
/// fingerprint after several changes still only reflects forward
/// movement.
#[test]
fn a_topology_epoch_never_goes_backwards_when_a_service_is_removed_and_re_added() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.record_topology_fingerprint("inst-1", "backend", "fp-a").unwrap();
    store.record_topology_fingerprint("inst-1", "backend", "fp-b").unwrap();
    let epoch_before_removal = store.topology_epoch("inst-1", "backend").unwrap();
    assert_eq!(epoch_before_removal, 2);

    // "Re-added" means a later submit that fingerprints this service
    // again -- there is no row-delete path at all, so the epoch can
    // only ever advance from here, never reset.
    let epoch = store.record_topology_fingerprint("inst-1", "backend", "fp-a").unwrap();
    assert_eq!(epoch, 3, "a re-added service must not reuse a lower epoch");
}

/// `resolve`'s backfill -- a store row with no `topology_epochs` entry
/// yet resolves at epoch 1, not 0, and a second call does not advance
/// it again.
#[test]
fn resolve_backfills_a_topology_epoch_for_an_instance_with_no_prior_record() {
    let store = SupervisorStore::open_in_memory().unwrap();
    let (epoch, fp) = store.initialise_topology_epoch("inst-1", "backend", "fp-a").unwrap();
    assert_eq!(epoch, 1, "an instance with no prior record must backfill at epoch 1, not 0");
    assert_eq!(fp, "fp-a");

    let (epoch2, fp2) = store.initialise_topology_epoch("inst-1", "backend", "fp-a").unwrap();
    assert_eq!(epoch2, 1, "a second insert-only call must not advance the epoch");
    assert_eq!(fp2, "fp-a");
}

/// The insert-only rule's whole safety property: a `submit` landing
/// between a lock-free reader's plan read and its fingerprint write
/// must not let that reader advance the epoch. Recording fingerprint A
/// through the advancing form (epoch 1), then calling the insert-only
/// form with a *different* fingerprint B (the stale-plan case) must
/// leave the epoch at 1 and the stored fingerprint at A -- and must
/// return A, so the caller can see it lost the race.
#[test]
fn a_resolve_never_advances_a_topology_epoch() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.record_topology_fingerprint("inst-1", "backend", "fp-a").unwrap();

    let (epoch, stored_fp) = store.initialise_topology_epoch("inst-1", "backend", "fp-b").unwrap();
    assert_eq!(epoch, 1, "the insert-only form must never advance an existing row's epoch");
    assert_eq!(stored_fp, "fp-a", "the caller must see the fingerprint it lost the race to");
}

#[test]
fn instance_by_app_master_did_finds_the_adopted_instance() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.submit("inst-1", "{}", "{}", "did:key:owner", 0).unwrap();
    store.record_adopt("inst-1", 1, "did:key:zAppMaster").unwrap();

    let found = store.instance_by_app_master_did("did:key:zAppMaster").unwrap().unwrap();
    assert_eq!(found.app_instance_id, "inst-1");
}

#[test]
fn instance_by_app_master_did_reports_none_for_an_unknown_did() {
    let store = SupervisorStore::open_in_memory().unwrap();
    store.submit("inst-1", "{}", "{}", "did:key:owner", 0).unwrap();
    assert_eq!(
        store.instance_by_app_master_did("did:key:zNeverAdopted").unwrap(),
        None,
        "an instance with no app master recorded must not match an empty-string lookup"
    );
}
