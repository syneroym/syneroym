#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;

fn config() -> DedupConfig {
    DedupConfig {
        ttl_ms: 600_000,
        claim_window_ms: 60_000,
        max_rows: 10_000,
        max_result_bytes: 64 * 1024,
    }
}

fn store() -> DedupStore {
    DedupStore::open_in_memory(config()).unwrap()
}

/// Whichever attempt holds the claim right now.
fn claim_of(store: &DedupStore, caller: &str, key: &str) -> ClaimToken {
    store.current_claim(caller, key)
}

/// Asserts a fresh claim was granted, and hands back its token.
fn expect_claim(store: &DedupStore, caller: &str, key: &str, now: i64) -> ClaimToken {
    match store.begin(caller, key, now, store.config.window()).unwrap() {
        DedupDecision::Execute(token) => token,
        other => panic!("expected a fresh claim, got {other:?}"),
    }
}

#[test]
fn a_call_with_a_fresh_key_executes_and_records_its_result() {
    let store = store();
    assert!(matches!(
        store.begin("did:key:zA", "k1", 1_000, store.config.window()).unwrap(),
        DedupDecision::Execute(_)
    ));
    store
        .finish(
            "did:key:zA",
            "k1",
            claim_of(&store, "did:key:zA", "k1"),
            &FirstOutcome::Success(b"42".to_vec()),
            1_500,
        )
        .unwrap();
    assert_eq!(
        store.begin("did:key:zA", "k1", 2_000, store.config.window()).unwrap(),
        DedupDecision::Replay(FirstOutcome::Success(b"42".to_vec()))
    );
}

#[test]
fn a_repeat_of_a_completed_call_that_failed_returns_the_same_callee_error() {
    let store = store();
    store.begin("did:key:zA", "k1", 1_000, store.config.window()).unwrap();
    let err = FirstOutcome::CalleeError { code: -32010, message: "denied".to_string() };
    store.finish("did:key:zA", "k1", claim_of(&store, "did:key:zA", "k1"), &err, 1_500).unwrap();
    assert_eq!(
        store.begin("did:key:zA", "k1", 2_000, store.config.window()).unwrap(),
        DedupDecision::Replay(err)
    );
}

#[test]
fn a_repeat_while_the_first_call_is_still_running_is_refused() {
    let store = store();
    store.begin("did:key:zA", "k1", 1_000, store.config.window()).unwrap();
    assert_eq!(
        store.begin("did:key:zA", "k1", 30_000, store.config.window()).unwrap(),
        DedupDecision::AlreadyRunning
    );
}

/// The honest at-least-once case, asserted against a supplied clock
/// rather than by sleeping.
#[test]
fn a_claim_whose_expiry_passed_is_retaken_and_the_call_runs_again() {
    let store = store();
    store.begin("did:key:zA", "k1", 1_000, store.config.window()).unwrap();
    assert!(matches!(
        store.begin("did:key:zA", "k1", 61_001, store.config.window()).unwrap(),
        DedupDecision::Execute(_)
    ));
}

#[test]
fn a_key_is_scoped_to_its_caller() {
    let store = store();
    store.begin("did:key:zA", "k1", 1_000, store.config.window()).unwrap();
    store
        .finish(
            "did:key:zA",
            "k1",
            claim_of(&store, "did:key:zA", "k1"),
            &FirstOutcome::Success(b"a".to_vec()),
            1_100,
        )
        .unwrap();
    assert!(
        matches!(
            store.begin("did:key:zB", "k1", 1_200, store.config.window()).unwrap(),
            DedupDecision::Execute(_)
        ),
        "the same key string from a different caller is a different call"
    );
}

#[test]
fn a_record_past_its_ttl_does_not_answer_and_is_pruned_on_the_next_write() {
    let store = store();
    store.begin("did:key:zA", "k1", 1_000, store.config.window()).unwrap();
    store
        .finish(
            "did:key:zA",
            "k1",
            claim_of(&store, "did:key:zA", "k1"),
            &FirstOutcome::Success(b"a".to_vec()),
            1_100,
        )
        .unwrap();
    assert_eq!(store.len().unwrap(), 1);

    // Past `finish`'s refreshed expiry: no longer an answer.
    let now = 1_100 + 600_001;
    assert!(matches!(
        store.begin("did:key:zB", "other", now, store.config.window()).unwrap(),
        DedupDecision::Execute(_)
    ));
    assert_eq!(store.len().unwrap(), 1, "the expired record must have been swept");
}

/// The row bound, as the code actually promises it: a ceiling checked
/// every `CAP_CHECK_INTERVAL` claims, not an exact limit enforced on
/// every single write -- the `COUNT` behind it is amortised off the
/// hot path. Overshoot up to that interval is expected; unbounded
/// growth is not.
#[test]
fn the_dedup_table_is_bounded_and_prunes_oldest_first() {
    let store = DedupStore::open_in_memory(DedupConfig { max_rows: 3, ..config() }).unwrap();
    let claims = CAP_CHECK_INTERVAL * 5;
    for i in 0..claims {
        store
            .begin("did:key:zA", &format!("k{i}"), 1_000 + i64::from(i), store.config.window())
            .unwrap();
    }
    let held = store.len().unwrap();
    assert!(
        held <= 3 + CAP_CHECK_INTERVAL,
        "the table must stay bounded; held {held} against a cap of 3 checked every \
         {CAP_CHECK_INTERVAL} claims"
    );
    assert!(held < claims, "and must actually prune, not merely grow more slowly");

    // Oldest-first: the very first key is long gone.
    assert!(matches!(
        store.begin("did:key:zA", "k0", 90_000, store.config.window()).unwrap(),
        DedupDecision::Execute(_)
    ));
}

/// A late attempt must not stamp its outcome over a claim a newer
/// attempt already retook, or a duplicate arriving in that gap is
/// served the stale answer and the newer attempt then overwrites it.
#[test]
fn a_finish_from_an_attempt_that_lost_its_claim_records_nothing() {
    let store = store();
    let stale = expect_claim(&store, "did:key:zA", "k1", 1_000);
    // Attempt 1 outruns its claim window; attempt 2 retakes the key.
    let fresh = expect_claim(&store, "did:key:zA", "k1", 61_001);
    assert_ne!(stale, fresh, "the retake must mint a new claim");

    // Attempt 1 finally finishes, far too late, quoting its own claim.
    store
        .finish("did:key:zA", "k1", stale, &FirstOutcome::Success(b"stale".to_vec()), 61_500)
        .unwrap();

    assert_eq!(
        store.begin("did:key:zA", "k1", 61_600, store.config.window()).unwrap(),
        DedupDecision::AlreadyRunning,
        "attempt 2 still holds the key; the late finish must not have marked it done"
    );
}

/// The attempt that holds the claim records normally -- the guard must
/// not break the ordinary path it is protecting.
#[test]
fn a_finish_from_the_attempt_holding_the_claim_records_its_outcome() {
    let store = store();
    store.begin("did:key:zA", "k1", 1_000, store.config.window()).unwrap();
    store
        .finish(
            "did:key:zA",
            "k1",
            claim_of(&store, "did:key:zA", "k1"),
            &FirstOutcome::Success(b"ok".to_vec()),
            1_500,
        )
        .unwrap();
    assert_eq!(
        store.begin("did:key:zA", "k1", 2_000, store.config.window()).unwrap(),
        DedupDecision::Replay(FirstOutcome::Success(b"ok".to_vec()))
    );
}

/// The row cap is per caller, so a noisy peer can only evict its own
/// records. A shared cap would be an eviction channel: flush someone
/// else's fence and their next duplicate re-executes.
#[test]
fn one_caller_cannot_evict_another_callers_records() {
    let store = DedupStore::open_in_memory(DedupConfig { max_rows: 2, ..config() }).unwrap();
    store.begin("did:key:zVictim", "important", 1_000, store.config.window()).unwrap();
    store
        .finish(
            "did:key:zVictim",
            "important",
            claim_of(&store, "did:key:zVictim", "important"),
            &FirstOutcome::Success(b"v".to_vec()),
            1_010,
        )
        .unwrap();

    // Far past the cap *and* past several cap-check intervals. Twenty
    // claims would not do it: the check is amortised, so the eviction
    // branch would never run and this would pass against a global cap
    // too -- proving nothing about the per-caller scoping it exists to
    // pin.
    let noisy = CAP_CHECK_INTERVAL * 5;
    for i in 0..noisy {
        store
            .begin("did:key:zNoisy", &format!("n{i}"), 1_100 + i64::from(i), store.config.window())
            .unwrap();
    }
    assert!(
        store.len().unwrap() < u32::from(u16::try_from(noisy).unwrap()),
        "precondition: the noisy caller must actually have triggered eviction"
    );

    assert!(
        matches!(
            store.begin("did:key:zVictim", "important", 2_000, store.config.window()).unwrap(),
            DedupDecision::Replay(_)
        ),
        "the victim's fence must survive another caller's overflow"
    );
}

#[test]
fn an_oversized_result_is_recorded_as_done_without_its_body() {
    let store =
        DedupStore::open_in_memory(DedupConfig { max_result_bytes: 8, ..config() }).unwrap();
    store.begin("did:key:zA", "k1", 1_000, store.config.window()).unwrap();
    store
        .finish(
            "did:key:zA",
            "k1",
            claim_of(&store, "did:key:zA", "k1"),
            &FirstOutcome::Success(b"far too long".to_vec()),
            1_100,
        )
        .unwrap();
    assert_eq!(
        store.begin("did:key:zA", "k1", 1_200, store.config.window()).unwrap(),
        DedupDecision::Replay(FirstOutcome::SuccessNotRetained),
        "the call must still not be re-executed, even with no body to return"
    );
}

#[test]
fn a_failure_before_the_target_ran_leaves_no_claim_behind() {
    let store = store();
    store.begin("did:key:zA", "k1", 1_000, store.config.window()).unwrap();
    store.release("did:key:zA", "k1", claim_of(&store, "did:key:zA", "k1")).unwrap();
    assert!(
        matches!(
            store.begin("did:key:zA", "k1", 1_100, store.config.window()).unwrap(),
            DedupDecision::Execute(_)
        ),
        "a corrected retry must not be blocked by a claim nothing executed under"
    );
}

/// A finished record must survive `release`: an outcome is not a claim.
#[test]
fn releasing_a_finished_record_does_not_erase_its_outcome() {
    let store = store();
    store.begin("did:key:zA", "k1", 1_000, store.config.window()).unwrap();
    store
        .finish(
            "did:key:zA",
            "k1",
            claim_of(&store, "did:key:zA", "k1"),
            &FirstOutcome::Success(b"a".to_vec()),
            1_100,
        )
        .unwrap();
    store.release("did:key:zA", "k1", claim_of(&store, "did:key:zA", "k1")).unwrap();
    assert!(matches!(
        store.begin("did:key:zA", "k1", 1_200, store.config.window()).unwrap(),
        DedupDecision::Replay(_)
    ));
}

#[test]
fn the_dedup_lookup_is_one_indexed_probe_and_no_scan() {
    let store = store();
    let plan = store.explain_lookup_plan().unwrap();
    assert!(
        plan.contains("USING INDEX") || plan.contains("USING PRIMARY KEY"),
        "expected the lookup to use the primary key, got: {plan}"
    );
    assert!(!plan.to_uppercase().contains("SCAN TABLE"), "expected no table scan, got: {plan}");
}

#[test]
fn the_ttl_is_at_least_the_senders_own_total_retry_window() {
    let queue = crate::QueueConfig::from(&syneroym_core::config::AppSandboxRole::default());
    let cfg =
        DedupConfig::derive(queue.total_retry_window_ms(), queue.visibility_timeout_ms, 30_000);
    assert!(
        cfg.ttl_ms >= queue.total_retry_window_ms(),
        "a TTL shorter than the retry window silently converts dedup into no dedup"
    );
    assert_eq!(cfg.claim_window_ms, 60_000);
}

fn long_window(store: &DedupStore, max_total_rows: u32) -> DedupWindow {
    DedupWindow {
        ttl_ms: store.config.ttl_ms * 100,
        max_rows: store.config.max_rows,
        max_total_rows,
    }
}

#[test]
fn a_record_written_with_a_long_window_outlives_the_default_window() {
    let store = store();
    let long = long_window(&store, u32::MAX);
    let claim = match store.begin("did:key:zA", "k1", 1_000, long).unwrap() {
        DedupDecision::Execute(token) => token,
        other => panic!("expected a fresh claim, got {other:?}"),
    };
    let outcome = FirstOutcome::Success(b"a".to_vec());
    store.finish("did:key:zA", "k1", claim, &outcome, 1_100).unwrap();

    let after_default_ttl = 1_100 + store.config.ttl_ms as i64 + 1;
    assert_eq!(
        store.begin("did:key:zA", "k1", after_default_ttl, store.config.window()).unwrap(),
        DedupDecision::Replay(outcome),
        "the record keeps the window it was written with"
    );
}

#[test]
fn the_per_caller_cap_is_the_one_in_the_window_of_the_call() {
    let store = store();
    let tight = DedupWindow { max_rows: 2, ..store.config.window() };
    // Pruning counts rows once per cap-check interval, and the first claim
    // always checks, so write enough claims to pass one check after the
    // third row.
    for i in 0..CAP_CHECK_INTERVAL + 3 {
        store.begin("did:key:zA", &format!("k{i}"), 1_000 + i64::from(i), tight).unwrap();
    }
    assert!(
        store.len().unwrap() < CAP_CHECK_INTERVAL + 3,
        "rows past the window's cap were pruned, though the store's own cap is larger"
    );
}

fn expiry_of(store: &DedupStore, caller: &str) -> i64 {
    let conn = store.conn.lock().unwrap();
    conn.query_row("SELECT expires_at FROM call_dedup WHERE caller = ?1", [caller], |row| {
        row.get(0)
    })
    .unwrap()
}

#[test]
fn a_long_window_closes_when_the_store_holds_too_many_live_rows() {
    let store = store();
    let long = long_window(&store, 10);
    // The cap is read once per cap-check interval, so claim past two of them.
    let claims = 2 * CAP_CHECK_INTERVAL + 1;
    for i in 0..claims {
        store.begin(&format!("did:key:zCaller{i}"), "k", 1_000, long).unwrap();
    }

    assert_eq!(expiry_of(&store, "did:key:zCaller0"), 1_000 + long.ttl_ms as i64);
    let last = expiry_of(&store, &format!("did:key:zCaller{}", claims - 1));
    assert_eq!(last, 1_000 + store.config.ttl_ms as i64, "a late claim gets the default window");
    assert_eq!(store.len().unwrap(), claims, "no record was evicted to make room");
}

#[test]
fn a_record_keeps_its_long_window_when_the_call_finishes() {
    let store = store();
    let long = long_window(&store, u32::MAX);
    let claim = match store.begin("did:key:zA", "k1", 1_000, long).unwrap() {
        DedupDecision::Execute(token) => token,
        other => panic!("expected a fresh claim, got {other:?}"),
    };

    store.finish("did:key:zA", "k1", claim, &FirstOutcome::Success(b"a".to_vec()), 2_000).unwrap();

    assert_eq!(expiry_of(&store, "did:key:zA"), 1_000 + long.ttl_ms as i64);
}

#[test]
fn the_long_window_opens_again_once_the_records_have_expired() {
    let store = store();
    // Barely longer than the default, so the records are gone by `later`.
    let long = DedupWindow {
        ttl_ms: store.config.ttl_ms + 1,
        max_rows: store.config.max_rows,
        max_total_rows: 100,
    };
    for i in 0..=2 * CAP_CHECK_INTERVAL {
        store.begin(&format!("did:key:zCaller{i}"), "k", 1_000, long).unwrap();
    }
    assert!(store.long_window_closed(), "too many live records close the long window");

    let later = 10 * store.config.ttl_ms as i64;
    for i in 0..=CAP_CHECK_INTERVAL {
        store.begin(&format!("did:key:zLater{i}"), "k", later, long).unwrap();
    }

    assert!(!store.long_window_closed(), "after the records expired it opened again");
}
