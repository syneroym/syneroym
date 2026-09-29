//! `select_evidence_decisions` is pure, so these drive it directly with
//! hand-built rows carrying distinct `issued_at_secs` values. The dual-build
//! parity harness pins one signing clock for every record in a scenario
//! (`RecordClock::Fixed`), so every row there shares the same
//! `issued_at_secs` and the function's recency ordering degrades to
//! comparing `record_id` hashes -- fine as a wasm/native agreement check,
//! not as a test of which decision the function actually prefers. These
//! tests are that check.

use super::*;

const MEMBER: &str = "did:key:zTestMember";

fn suspend(
    record_id: &str,
    issued_at_secs: u64,
    until_secs: Option<u64>,
    membership: bool,
) -> IssuedRecordRow {
    IssuedRecordRow {
        record_id: record_id.to_string(),
        member_did: MEMBER.to_string(),
        about: String::new(),
        issued_at_secs,
        until_secs,
        is_membership_scope: membership,
        envelope: format!("env-{record_id}"),
    }
}

fn lift(record_id: &str, issued_at_secs: u64, about: &str) -> IssuedRecordRow {
    IssuedRecordRow {
        record_id: record_id.to_string(),
        member_did: MEMBER.to_string(),
        about: about.to_string(),
        issued_at_secs,
        until_secs: None,
        is_membership_scope: false,
        envelope: format!("env-{record_id}"),
    }
}

fn ids(rows: &[IssuedRecordRow]) -> Vec<&str> {
    rows.iter().map(|r| r.record_id.as_str()).collect()
}

#[test]
fn empty_input_selects_nothing() {
    assert!(select_evidence_decisions(&[], 1_000).is_empty());
}

/// An old, never-lifted, no-end-date membership suspension must not be
/// dropped just because many newer decisions about the same member exist.
#[test]
fn an_old_permanent_membership_suspension_survives_many_newer_lifted_pairs() {
    let mut all = vec![suspend("permanent", 100, None, true)];
    for i in 0..(membership::MAX_EVIDENCE_DECISIONS + 2) {
        let s = format!("s{i}");
        let l = format!("l{i}");
        all.push(suspend(&s, 1_000 + i as u64, None, false));
        all.push(lift(&l, 2_000 + i as u64, &s));
    }
    let selected = select_evidence_decisions(&all, 10_000);
    assert!(selected.len() <= membership::MAX_EVIDENCE_DECISIONS, "{}", selected.len());
    assert!(ids(&selected).contains(&"permanent"), "{:?}", ids(&selected));
}

/// A *timed* membership suspension must never be treated as making an
/// older *permanent* one redundant, however much newer it is -- once the
/// timed one ends, the permanent one is the only thing still governing
/// the member, and it must still be in the evidence to be seen. The
/// filler decisions below are what makes this a real test: with room to
/// spare, a shortcut that wrongly picked the timed suspension alone would
/// still leave enough slots for the permanent one to ride along anyway,
/// and the bug would go unnoticed.
#[test]
fn a_permanent_membership_suspension_outranks_a_newer_timed_one() {
    let mut all =
        vec![suspend("permanent", 100, None, true), suspend("timed", 200, Some(50_000), true)];
    for i in 0..membership::MAX_EVIDENCE_DECISIONS {
        all.push(suspend(&format!("filler{i}"), 1_000 + i as u64, None, false));
    }
    let selected = select_evidence_decisions(&all, 10_000);
    assert!(selected.len() <= membership::MAX_EVIDENCE_DECISIONS, "{}", selected.len());
    assert!(ids(&selected).contains(&"permanent"), "{:?}", ids(&selected));
}

/// When no permanent membership suspension exists, an active *timed*
/// membership suspension must still outrank newer listing-scope ones in
/// the fallback -- it alone hides every listing while it lasts, unlike
/// any one of them.
#[test]
fn a_timed_membership_suspension_outranks_newer_listing_suspensions_in_the_fallback() {
    let mut all = vec![suspend("membership", 100, Some(50_000), true)];
    for i in 0..membership::MAX_EVIDENCE_DECISIONS {
        all.push(suspend(&format!("listing{i}"), 1_000 + i as u64, None, false));
    }
    let selected = select_evidence_decisions(&all, 10_000);
    assert_eq!(selected.len(), membership::MAX_EVIDENCE_DECISIONS, "{:?}", ids(&selected));
    assert!(ids(&selected).contains(&"membership"), "{:?}", ids(&selected));
}

/// The same fallback ranking also applies with no membership-scope
/// suspension in the mix at all: an old permanent *listing*-scope
/// suspension must outrank many newer *timed* listing-scope ones, not
/// just lose to them on age. This is the ranked-fallback path exercised
/// on its own, since every row here is equally not-membership-scope and
/// the shortcut above never fires for any of them.
#[test]
fn an_old_permanent_listing_suspension_outranks_many_newer_timed_ones_in_the_fallback() {
    let mut all = vec![suspend("permanent-listing", 100, None, false)];
    for i in 0..membership::MAX_EVIDENCE_DECISIONS {
        all.push(suspend(&format!("timed{i}"), 1_000 + i as u64, Some(50_000 + i as u64), false));
    }
    let selected = select_evidence_decisions(&all, 10_000);
    assert_eq!(selected.len(), membership::MAX_EVIDENCE_DECISIONS, "{:?}", ids(&selected));
    assert!(ids(&selected).contains(&"permanent-listing"), "{:?}", ids(&selected));
}

/// A suspend and its lift are one unit for the cap: if there is not room
/// for both, neither rides, rather than serving a lift with no decision
/// for it to lift (or, symmetrically, an apparently-still-active suspend
/// whose lift was silently dropped).
#[test]
fn a_suspend_and_its_lift_are_never_split() {
    let mut all = Vec::new();
    // Fill every slot but one with single-cost active listing suspensions.
    for i in 0..(membership::MAX_EVIDENCE_DECISIONS - 1) {
        all.push(suspend(&format!("filler{i}"), 1_000 + i as u64, None, false));
    }
    // The next candidate by recency is a lifted pair, costing two slots --
    // only one is left.
    all.push(suspend("old-suspend", 5_000, None, false));
    all.push(lift("old-lift", 5_500, "old-suspend"));

    let selected = select_evidence_decisions(&all, 10_000);
    assert_eq!(selected.len(), membership::MAX_EVIDENCE_DECISIONS - 1, "{:?}", ids(&selected));
    assert!(!ids(&selected).contains(&"old-suspend"), "{:?}", ids(&selected));
    assert!(!ids(&selected).contains(&"old-lift"), "{:?}", ids(&selected));
}

/// A lifted suspension that *does* fit is included with its lift, both or
/// neither never applying when there is room for both.
#[test]
fn a_suspend_and_its_lift_ride_together_when_there_is_room() {
    let all = vec![suspend("s", 100, None, false), lift("l", 200, "s")];
    let selected = select_evidence_decisions(&all, 10_000);
    assert_eq!(ids(&selected).len(), 2, "{:?}", ids(&selected));
    assert!(ids(&selected).contains(&"s") && ids(&selected).contains(&"l"));
}

/// The selection never exceeds the cap, however many active, unrelated
/// listing-scope suspensions exist and however they are ordered.
#[test]
fn selection_never_exceeds_the_cap() {
    let mut all = Vec::new();
    for i in 0..(membership::MAX_EVIDENCE_DECISIONS * 3) {
        all.push(suspend(&format!("listing{i}"), (i as u64).wrapping_mul(37), None, false));
    }
    let selected = select_evidence_decisions(&all, 10_000);
    assert!(selected.len() <= membership::MAX_EVIDENCE_DECISIONS, "{}", selected.len());
}
