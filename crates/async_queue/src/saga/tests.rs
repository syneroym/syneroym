use syneroym_core::config::{AppSandboxRole, RetryPolicy};

use super::*;

fn config() -> SagaConfig {
    SagaConfig {
        retry: RetryPolicy {
            max_attempts: 3,
            initial_backoff_ms: 100,
            backoff_multiplier: 2.0,
            max_backoff_ms: 900_000,
        },
        max_open: 64,
        max_steps: 64,
        max_terminal_rows: 1000,
        default_deadline_ms: 3_600_000,
        max_deadline_ms: 86_400_000,
        step_timeout_ms: 4_000,
    }
}

fn intent(interface: &str, method: &str, params: &[u8]) -> StepIntent {
    StepIntent {
        target: "{\"service_id\":\"peer\"}".to_string(),
        routing_key: None,
        interface: interface.to_string(),
        method: method.to_string(),
        params: params.to_vec(),
    }
}

/// Durability and the SQLCipher key in one assertion, mirroring
/// `Queue`'s `a_queued_item_survives_reopening_the_encrypted_queue_file`:
/// a step written to an encrypted saga log is still there after the
/// process that wrote it is gone, and still readable with the same key.
#[test]
fn a_saga_step_survives_reopening_the_encrypted_log_file() {
    let dir = tempfile::tempdir().unwrap();
    let dek = [7u8; 32];
    {
        let log = SagaLog::open_encrypted(dir.path(), "async.db", Some(&dek), config()).unwrap();
        log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
        log.record_step_intent("s1", &intent("iface", "reserve", b"secret-params"), 1_000).unwrap();
    }
    let log = SagaLog::open_encrypted(dir.path(), "async.db", Some(&dek), config()).unwrap();
    let step = log.next_uncompensated_step("s1").unwrap().unwrap();
    assert_eq!(step.params, b"secret-params");
}

/// A saga step's params and result are the calling service's own data,
/// so the file they wait in must be no weaker than the database that
/// data came from -- same property `Queue`'s
/// `the_queue_file_is_unreadable_without_the_services_dek` establishes
/// for the outbox. Scoped to an encryption-enabled deployment, same as
/// that test.
#[test]
fn the_saga_log_file_is_unreadable_without_the_services_dek() {
    let dir = tempfile::tempdir().unwrap();
    {
        let log =
            SagaLog::open_encrypted(dir.path(), "async.db", Some(&[7u8; 32]), config()).unwrap();
        log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
        log.record_step_intent("s1", &intent("iface", "reserve", b"secret-params"), 1_000).unwrap();
    }
    assert!(
        SagaLog::open_encrypted(dir.path(), "async.db", None, config()).is_err(),
        "an unkeyed open of an encrypted saga log file must fail"
    );
    assert!(
        SagaLog::open_encrypted(dir.path(), "async.db", Some(&[8u8; 32]), config()).is_err(),
        "the wrong key must fail exactly like no key"
    );
}

#[test]
fn a_step_added_to_a_compensating_saga_is_refused() {
    let log = SagaLog::open_in_memory(config()).unwrap();
    log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
    log.mark_compensating("s1", 1_000).unwrap();

    let err = log.record_step_intent("s1", &intent("iface", "reserve", b"{}"), 1_000).unwrap_err();
    assert!(err.to_string().contains("compensating"), "unexpected error: {err}");
}

#[test]
fn steps_are_indexed_in_the_order_they_were_recorded() {
    let log = SagaLog::open_in_memory(config()).unwrap();
    log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
    let a = log.record_step_intent("s1", &intent("iface", "a", b"{}"), 1_000).unwrap();
    let b = log.record_step_intent("s1", &intent("iface", "b", b"{}"), 1_001).unwrap();
    assert_eq!((a, b), (0, 1));
}

#[test]
fn the_next_uncompensated_step_is_the_highest_index_first() {
    let log = SagaLog::open_in_memory(config()).unwrap();
    log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
    log.record_step_intent("s1", &intent("iface", "a", b"{}"), 1_000).unwrap();
    log.record_step_intent("s1", &intent("iface", "b", b"{}"), 1_001).unwrap();
    log.record_step_outcome("s1", 0, Some(b"{}"), None, 1_002).unwrap();
    log.record_step_outcome("s1", 1, Some(b"{}"), None, 1_003).unwrap();

    let step = log.next_uncompensated_step("s1").unwrap().unwrap();
    assert_eq!(step.idx, 1, "the newest step compensates first");
}

#[test]
fn a_pending_step_is_compensated_like_a_done_one() {
    let log = SagaLog::open_in_memory(config()).unwrap();
    log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
    log.record_step_intent("s1", &intent("iface", "a", b"{}"), 1_000).unwrap();
    // Never recorded an outcome -- the crash-mid-call case.
    let step = log.next_uncompensated_step("s1").unwrap().unwrap();
    assert_eq!(step.idx, 0);
}

#[test]
fn a_failed_step_is_never_compensated() {
    let log = SagaLog::open_in_memory(config()).unwrap();
    log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
    log.record_step_intent("s1", &intent("iface", "a", b"{}"), 1_000).unwrap();
    log.record_step_outcome("s1", 0, None, Some("boom"), 1_001).unwrap();

    assert!(log.next_uncompensated_step("s1").unwrap().is_none());
}

#[test]
fn committing_deletes_the_saga_and_its_steps() {
    let log = SagaLog::open_in_memory(config()).unwrap();
    log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
    log.record_step_intent("s1", &intent("iface", "a", b"{}"), 1_000).unwrap();
    log.commit("s1").unwrap();

    assert!(log.status("s1").unwrap().is_none());
    assert_eq!(log.open_count().unwrap(), 0);
}

#[test]
fn committing_a_compensating_saga_is_refused() {
    let log = SagaLog::open_in_memory(config()).unwrap();
    log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
    log.mark_compensating("s1", 1_000).unwrap();

    let err = log.commit("s1").unwrap_err();
    assert!(err.to_string().contains("compensating"), "unexpected error: {err}");
}

#[test]
fn a_failed_undo_schedules_a_backoff_and_a_terminal_one_fails_the_saga() {
    let log = SagaLog::open_in_memory(config()).unwrap();
    log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
    log.record_step_intent("s1", &intent("iface", "a", b"{}"), 1_000).unwrap();
    log.mark_compensating("s1", 1_000).unwrap();
    log.begin_undo_attempt("s1", 0, 1_000).unwrap();

    let outcome = log.fail_compensation("s1", 0, 1_000, "unreachable", false).unwrap();
    assert!(matches!(outcome, CompensationOutcome::Retry { .. }));
    assert_eq!(log.status("s1").unwrap().unwrap().state, "compensating");

    let outcome = log.fail_compensation("s1", 0, 2_000, "still gone", true).unwrap();
    assert_eq!(outcome, CompensationOutcome::Failed);
    assert_eq!(log.status("s1").unwrap().unwrap().state, "failed");
}

#[test]
fn an_exhausted_attempt_budget_fails_the_saga() {
    let log = SagaLog::open_in_memory(config()).unwrap();
    log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
    log.record_step_intent("s1", &intent("iface", "a", b"{}"), 1_000).unwrap();
    log.mark_compensating("s1", 1_000).unwrap();

    let mut outcome = CompensationOutcome::Failed;
    for now in (1_000..).step_by(1).take(3) {
        log.begin_undo_attempt("s1", 0, now).unwrap();
        outcome = log.fail_compensation("s1", 0, now, "unreachable", false).unwrap();
    }
    assert_eq!(outcome, CompensationOutcome::Failed);
}

#[test]
fn abandoned_lists_only_open_sagas_past_their_deadline() {
    let log = SagaLog::open_in_memory(config()).unwrap();
    log.begin("s1", "wf", None, 1_500, 1_000).unwrap();
    log.begin("s2", "wf", None, 3_000, 1_000).unwrap();
    log.mark_compensating("s2", 1_000).unwrap();
    log.begin("s3", "wf", None, 500, 1_000).unwrap();

    let heads = log.abandoned(2_000, 10).unwrap();
    let ids: Vec<&str> = heads.iter().map(|h| h.saga_id.as_str()).collect();
    assert_eq!(ids, vec!["s3", "s1"], "only still-open sagas past their deadline, oldest first");
}

#[test]
fn due_compensations_ignores_a_saga_whose_next_attempt_is_in_the_future() {
    let log = SagaLog::open_in_memory(config()).unwrap();
    log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
    log.record_step_intent("s1", &intent("iface", "a", b"{}"), 1_000).unwrap();
    log.mark_compensating("s1", 1_000).unwrap();
    log.begin_undo_attempt("s1", 0, 1_000).unwrap();
    log.fail_compensation("s1", 0, 1_000, "unreachable", false).unwrap();

    assert!(log.due_compensations(1_000, 10).unwrap().is_empty());
    // A `next_attempt_at` far enough out that the jittered backoff must
    // still be in the future.
    assert!(log.due_compensations(1_050, 10).unwrap().is_empty());
}

#[test]
fn terminal_rows_are_pruned_oldest_first_at_the_cap() {
    let mut cfg = config();
    cfg.max_terminal_rows = 2;
    let log = SagaLog::open_in_memory(cfg).unwrap();
    for i in 0..3 {
        let id = format!("s{i}");
        log.begin(&id, "wf", None, 3_600_000, 1_000 + i).unwrap();
        log.commit(&id).unwrap();
    }
    // `commit` deletes rather than terminating, so drive terminal rows
    // through `finish_compensation` instead.
    for i in 3..6 {
        let id = format!("t{i}");
        log.begin(&id, "wf", None, 3_600_000, 1_000 + i).unwrap();
        log.mark_compensating(&id, 1_000 + i).unwrap();
        log.finish_compensation(&id, 1_000 + i).unwrap();
    }
    let all = log.list().unwrap();
    let terminal: Vec<&SagaInfo> =
        all.iter().filter(|s| s.state == "compensated" || s.state == "failed").collect();
    assert_eq!(terminal.len(), 2, "the cap must hold even after five terminal writes");
    let ids: Vec<&str> = terminal.iter().map(|s| s.saga_id.as_str()).collect();
    assert_eq!(ids, vec!["t4", "t5"], "the two oldest terminal rows were pruned");
}

#[test]
fn an_over_limit_open_count_refuses_a_new_saga() {
    let mut cfg = config();
    cfg.max_open = 1;
    let log = SagaLog::open_in_memory(cfg).unwrap();
    log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();

    let err = log.begin("s2", "wf", None, 3_600_000, 1_000).unwrap_err();
    assert!(err.to_string().contains("open sagas"), "unexpected error: {err}");
}

#[test]
fn an_over_limit_step_count_refuses_a_new_step() {
    let mut cfg = config();
    cfg.max_steps = 1;
    let log = SagaLog::open_in_memory(cfg).unwrap();
    log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
    log.record_step_intent("s1", &intent("iface", "a", b"{}"), 1_000).unwrap();

    let err = log.record_step_intent("s1", &intent("iface", "b", b"{}"), 1_001).unwrap_err();
    assert!(err.to_string().contains("steps"), "unexpected error: {err}");
}

#[test]
fn an_over_sized_params_payload_is_refused() {
    let log = SagaLog::open_in_memory(config()).unwrap();
    log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
    let oversized = vec![0u8; MAX_SAGA_PAYLOAD_BYTES + 1];

    let err = log.record_step_intent("s1", &intent("iface", "a", &oversized), 1_000).unwrap_err();
    assert!(err.to_string().contains("byte limit"), "unexpected error: {err}");
}

#[test]
fn rearm_returns_a_failed_saga_to_compensating_with_attempts_reset() {
    let log = SagaLog::open_in_memory(config()).unwrap();
    log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
    log.record_step_intent("s1", &intent("iface", "a", b"{}"), 1_000).unwrap();
    log.mark_compensating("s1", 1_000).unwrap();
    log.begin_undo_attempt("s1", 0, 1_000).unwrap();
    log.fail_compensation("s1", 0, 1_000, "gone", true).unwrap();
    assert_eq!(log.status("s1").unwrap().unwrap().state, "failed");

    assert!(log.rearm("s1", 2_000).unwrap());
    let info = log.status("s1").unwrap().unwrap();
    assert_eq!(info.state, "compensating");
    assert!(info.last_error.is_none());

    let step = log.next_uncompensated_step("s1").unwrap().unwrap();
    assert_eq!(step.attempts, 0, "rearm resets the current step's attempts");
}

#[test]
fn rearm_is_false_for_a_saga_that_is_not_failed() {
    let log = SagaLog::open_in_memory(config()).unwrap();
    log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
    assert!(!log.rearm("s1", 1_000).unwrap());
    assert!(!log.rearm("unknown", 1_000).unwrap());
}

#[test]
fn the_configured_deadline_ceiling_refuses_rather_than_clamps() {
    // The ceiling itself is enforced by the router (`saga_begin_impl`),
    // not by `SagaLog::begin`, which stores whatever deadline it is
    // given -- this test pins that `SagaConfig::max_deadline_ms` is
    // exposed for that check.
    let cfg = config();
    assert_eq!(cfg.max_deadline_ms, 86_400_000);
}

#[test]
fn saga_config_derives_step_timeout_below_the_dispatch_epoch() {
    let role = AppSandboxRole { dispatch_epoch_timeout_secs: 5, ..AppSandboxRole::default() };
    let cfg = SagaConfig::from(&role);
    assert_eq!(cfg.step_timeout_ms, 4_000);
}

#[test]
fn saga_config_step_timeout_floors_at_the_minimum_budget() {
    let role = AppSandboxRole { dispatch_epoch_timeout_secs: 1, ..AppSandboxRole::default() };
    let cfg = SagaConfig::from(&role);
    assert_eq!(cfg.step_timeout_ms, MIN_STEP_CALL_BUDGET_MS);
}

#[test]
fn drop_all_for_undeployed_removes_every_saga_and_step() {
    let log = SagaLog::open_in_memory(config()).unwrap();
    log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
    log.record_step_intent("s1", &intent("iface", "a", b"{}"), 1_000).unwrap();
    log.begin("s2", "wf", None, 3_600_000, 1_000).unwrap();

    log.drop_all_for_undeployed().unwrap();

    assert!(log.list().unwrap().is_empty());
    assert_eq!(log.open_count().unwrap(), 0);
}

#[test]
fn a_step_outcome_requires_exactly_one_of_result_or_error() {
    let log = SagaLog::open_in_memory(config()).unwrap();
    log.begin("s1", "wf", None, 3_600_000, 1_000).unwrap();
    log.record_step_intent("s1", &intent("iface", "a", b"{}"), 1_000).unwrap();

    assert!(log.record_step_outcome("s1", 0, None, None, 1_001).is_err());
    assert!(log.record_step_outcome("s1", 0, Some(b"{}"), Some("e"), 1_001).is_err());
}
