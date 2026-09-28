#![allow(clippy::cognitive_complexity)]

use std::{fs, path::PathBuf};

use syneroym_identity::{Identity, substrate};
use syneroym_signed_record::{Envelope, RecordDraft};

use super::*;

fn generate() -> (Identity, String) {
    let key = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&key.public_key());
    (key, did)
}

fn did() -> String {
    generate().1
}

fn sign_record(
    identity: &Identity,
    issuer: &str,
    draft: RecordDraft,
    issued_at_secs: u64,
) -> String {
    let (mut env, bytes) =
        Envelope::unsigned(draft, issuer.to_string(), None, issued_at_secs).unwrap();
    let sig = z32::encode(&identity.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    env.to_json().unwrap()
}

fn record_id_of(env_json: &str) -> String {
    Envelope::from_json(env_json).unwrap().record_id().unwrap()
}

fn credential_env(
    issuer_key: &Identity,
    issuer_did: &str,
    member_did: &str,
    categories: &[&str],
    expires_at_secs: u64,
    issued_at_secs: u64,
    supersedes: Option<String>,
) -> String {
    let payload = MembershipCredentialPayload {
        synorg_name: "Test Guild".to_string(),
        member_did: member_did.to_string(),
        scope: MembershipScope {
            categories: categories.iter().map(|c| c.to_string()).collect(),
            area: vec![],
        },
    };
    let draft = RecordDraft {
        version: MEMBERSHIP_CREDENTIAL_VERSION,
        record_type: record::RECORD_MEMBERSHIP_CREDENTIAL.to_string(),
        subject: member_did.to_string(),
        payload: serde_json::to_value(&payload).unwrap(),
        expires_at_secs: Some(expires_at_secs),
        supersedes,
    };
    sign_record(issuer_key, issuer_did, draft, issued_at_secs)
}

fn credential_env_with_area(
    issuer_key: &Identity,
    issuer_did: &str,
    member_did: &str,
    categories: &[&str],
    area: Vec<Area>,
    expires_at_secs: u64,
    issued_at_secs: u64,
) -> String {
    let payload = MembershipCredentialPayload {
        synorg_name: "Test Guild".to_string(),
        member_did: member_did.to_string(),
        scope: MembershipScope {
            categories: categories.iter().map(|c| c.to_string()).collect(),
            area,
        },
    };
    let draft = RecordDraft {
        version: MEMBERSHIP_CREDENTIAL_VERSION,
        record_type: record::RECORD_MEMBERSHIP_CREDENTIAL.to_string(),
        subject: member_did.to_string(),
        payload: serde_json::to_value(&payload).unwrap(),
        expires_at_secs: Some(expires_at_secs),
        supersedes: None,
    };
    sign_record(issuer_key, issuer_did, draft, issued_at_secs)
}

fn revocation_env(
    issuer_key: &Identity,
    issuer_did: &str,
    credential_record_id: &str,
    member_did: &str,
    issued_at_secs: u64,
) -> String {
    let payload = RevocationPayload {
        credential_record_id: credential_record_id.to_string(),
        member_did: member_did.to_string(),
        reason: "test".to_string(),
    };
    let draft = RecordDraft {
        version: REVOCATION_VERSION,
        record_type: record::RECORD_REVOCATION.to_string(),
        subject: credential_record_id.to_string(),
        payload: serde_json::to_value(&payload).unwrap(),
        expires_at_secs: None,
        supersedes: None,
    };
    sign_record(issuer_key, issuer_did, draft, issued_at_secs)
}

#[allow(clippy::too_many_arguments)]
fn decision_env(
    issuer_key: &Identity,
    issuer_did: &str,
    member_did: &str,
    action: ModerationAction,
    scope: ModerationScope,
    rule: &str,
    until_secs: Option<u64>,
    issued_at_secs: u64,
    supersedes: Option<String>,
) -> String {
    let payload = ModerationDecisionPayload {
        action,
        member_did: member_did.to_string(),
        scope,
        rule: rule.to_string(),
        reason: "test".to_string(),
        until_secs,
    };
    let draft = RecordDraft {
        version: MODERATION_DECISION_VERSION,
        record_type: record::RECORD_MODERATION_DECISION.to_string(),
        subject: member_did.to_string(),
        payload: serde_json::to_value(&payload).unwrap(),
        expires_at_secs: None,
        supersedes,
    };
    sign_record(issuer_key, issuer_did, draft, issued_at_secs)
}

fn base_input<'a>(issuer: &'a str, member_did: &'a str, now: u64) -> CheckInput<'a> {
    CheckInput {
        pinned_issuer: Some(issuer),
        member_did,
        listing: None,
        now_secs: now,
        evidence_as_of_secs: now,
    }
}

#[test]
fn valid_credential_with_no_revocations_or_decisions_is_valid() {
    let (issuer_key, issuer_did) = generate();
    let member = did();
    let now = 1_000_000;
    let cred =
        credential_env(&issuer_key, &issuer_did, &member, &["cycling"], now + 1_000, now, None);
    let evidence = MembershipEvidence { credentials: vec![cred], ..Default::default() };
    let input =
        CheckInput { evidence_as_of_secs: now + 5, ..base_input(&issuer_did, &member, now) };
    match evaluate(&evidence, &input) {
        MembershipVerdict::Valid { revocations_checked_as_of_secs, scope, .. } => {
            assert_eq!(revocations_checked_as_of_secs, now + 5);
            assert_eq!(scope.categories, vec!["cycling".to_string()]);
        }
        other => panic!("expected Valid, got {other:?}"),
    }
}

#[test]
fn no_pinned_issuer_is_unknown() {
    let member = did();
    let evidence = MembershipEvidence::default();
    let input = CheckInput {
        pinned_issuer: None,
        member_did: &member,
        listing: None,
        now_secs: 0,
        evidence_as_of_secs: 0,
    };
    assert!(matches!(
        evaluate(&evidence, &input),
        MembershipVerdict::Unknown { reason } if reason == "issuer-not-pinned"
    ));
}

#[test]
fn credential_signed_by_a_different_master_is_refused() {
    let (issuer_key, issuer_did) = generate();
    let (_other_key, other_did) = generate();
    let member = did();
    let now = 1_000_000;
    let cred =
        credential_env(&issuer_key, &issuer_did, &member, &["cycling"], now + 1_000, now, None);
    let evidence = MembershipEvidence { credentials: vec![cred], ..Default::default() };
    // Pin the *other* DID as the issuer: the credential was signed as `issuer_did`.
    let input = base_input(&other_did, &member, now);
    assert!(matches!(evaluate(&evidence, &input), MembershipVerdict::Refused { .. }));
}

#[test]
fn credential_whose_subject_is_another_did_is_refused() {
    let (issuer_key, issuer_did) = generate();
    let member = did();
    let stranger = did();
    let now = 1_000_000;
    let cred =
        credential_env(&issuer_key, &issuer_did, &member, &["cycling"], now + 1_000, now, None);
    let evidence = MembershipEvidence { credentials: vec![cred], ..Default::default() };
    let input = base_input(&issuer_did, &stranger, now);
    assert!(matches!(evaluate(&evidence, &input), MembershipVerdict::Refused { .. }));
}

#[test]
fn wrong_record_type_or_version_is_refused() {
    let (issuer_key, issuer_did) = generate();
    let member = did();
    let now = 1_000_000;
    let payload = MembershipCredentialPayload {
        synorg_name: "Test Guild".to_string(),
        member_did: member.clone(),
        scope: MembershipScope { categories: vec!["cycling".to_string()], area: vec![] },
    };
    let draft = RecordDraft {
        version: 2,
        record_type: record::RECORD_MEMBERSHIP_CREDENTIAL.to_string(),
        subject: member.clone(),
        payload: serde_json::to_value(&payload).unwrap(),
        expires_at_secs: Some(now + 1_000),
        supersedes: None,
    };
    let wrong_version = sign_record(&issuer_key, &issuer_did, draft, now);
    let evidence = MembershipEvidence { credentials: vec![wrong_version], ..Default::default() };
    let input = base_input(&issuer_did, &member, now);
    assert!(matches!(evaluate(&evidence, &input), MembershipVerdict::Refused { .. }));
}

#[test]
fn empty_evidence_is_none() {
    let (_issuer_key, issuer_did) = generate();
    let member = did();
    let evidence = MembershipEvidence::default();
    let input = base_input(&issuer_did, &member, 1_000_000);
    assert!(matches!(evaluate(&evidence, &input), MembershipVerdict::None));
}

#[test]
fn a_revocation_of_this_credential_wins_but_another_credentials_revocation_does_not() {
    let (issuer_key, issuer_did) = generate();
    let member = did();
    let now = 1_000_000;
    let cred =
        credential_env(&issuer_key, &issuer_did, &member, &["cycling"], now + 1_000, now, None);
    let cred_id = record_id_of(&cred);
    let rev = revocation_env(&issuer_key, &issuer_did, &cred_id, &member, now + 1);
    let evidence = MembershipEvidence {
        credentials: vec![cred.clone()],
        revocations: vec![rev],
        ..Default::default()
    };
    let input = base_input(&issuer_did, &member, now + 10);
    assert!(matches!(evaluate(&evidence, &input), MembershipVerdict::Revoked { .. }));

    let unrelated_rev =
        revocation_env(&issuer_key, &issuer_did, "rec_nonexistent", &member, now + 1);
    let evidence2 = MembershipEvidence {
        credentials: vec![cred],
        revocations: vec![unrelated_rev],
        ..Default::default()
    };
    assert!(matches!(evaluate(&evidence2, &input), MembershipVerdict::Valid { .. }));
}

#[test]
fn revocations_past_the_credential_cap_are_never_checked() {
    let (issuer_key, issuer_did) = generate();
    let member = did();
    let now = 1_000_000;
    let cred =
        credential_env(&issuer_key, &issuer_did, &member, &["cycling"], now + 1_000, now, None);
    let cred_id = record_id_of(&cred);
    // `MAX_EVIDENCE_CREDENTIALS` unrelated revocations ahead of the real
    // one in the array: `find_revocation` takes only the same number of
    // entries the credentials themselves are bounded to (one revocation
    // per surviving credential is ever meaningful), so a source cannot
    // make a consumer pay for an unbounded number of signature checks per
    // reply.
    let mut revocations: Vec<String> = (0..MAX_EVIDENCE_CREDENTIALS)
        .map(|i| {
            revocation_env(&issuer_key, &issuer_did, &format!("rec_padding{i}"), &member, now + 1)
        })
        .collect();
    revocations.push(revocation_env(&issuer_key, &issuer_did, &cred_id, &member, now + 1));
    let evidence =
        MembershipEvidence { credentials: vec![cred], revocations, ..Default::default() };
    let input = base_input(&issuer_did, &member, now + 10);
    assert!(matches!(evaluate(&evidence, &input), MembershipVerdict::Valid { .. }));
}

#[test]
fn a_revocation_signed_by_a_different_issuer_is_ignored() {
    let (issuer_key, issuer_did) = generate();
    let (impostor_key, _impostor_did) = generate();
    let member = did();
    let now = 1_000_000;
    let cred =
        credential_env(&issuer_key, &issuer_did, &member, &["cycling"], now + 1_000, now, None);
    let cred_id = record_id_of(&cred);
    // Signed by the impostor but *claiming* to be issued by `issuer_did`:
    // `expecting(issuer_did)` in `verify_revocation` will reject this as a
    // bad signature, so it is ignored, exactly like a revocation from an
    // unrelated issuer.
    let rev = revocation_env(&impostor_key, &issuer_did, &cred_id, &member, now + 1);
    let evidence = MembershipEvidence {
        credentials: vec![cred],
        revocations: vec![rev],
        ..Default::default()
    };
    let input = base_input(&issuer_did, &member, now + 10);
    assert!(matches!(evaluate(&evidence, &input), MembershipVerdict::Valid { .. }));
}

#[test]
fn suspend_hides_membership_and_a_superseding_lift_restores_it() {
    let (issuer_key, issuer_did) = generate();
    let member = did();
    let now = 1_000_000;
    let cred =
        credential_env(&issuer_key, &issuer_did, &member, &["cycling"], now + 1_000, now, None);
    let suspend = decision_env(
        &issuer_key,
        &issuer_did,
        &member,
        ModerationAction::Suspend,
        ModerationScope::Membership,
        "no-shows",
        None,
        now + 1,
        None,
    );
    let suspend_id = record_id_of(&suspend);
    let evidence = MembershipEvidence {
        credentials: vec![cred.clone()],
        decisions: vec![suspend.clone()],
        ..Default::default()
    };
    let input = base_input(&issuer_did, &member, now + 2);
    assert!(matches!(evaluate(&evidence, &input), MembershipVerdict::Suspended { .. }));

    let lift = decision_env(
        &issuer_key,
        &issuer_did,
        &member,
        ModerationAction::Lift,
        ModerationScope::Membership,
        "",
        None,
        now + 3,
        Some(suspend_id),
    );
    let evidence2 = MembershipEvidence {
        credentials: vec![cred],
        decisions: vec![suspend, lift],
        ..Default::default()
    };
    assert!(matches!(evaluate(&evidence2, &input), MembershipVerdict::Valid { .. }));
}

#[test]
fn a_suspension_whose_until_secs_has_already_passed_no_longer_applies() {
    let (issuer_key, issuer_did) = generate();
    let member = did();
    let now = 1_000_000;
    let cred =
        credential_env(&issuer_key, &issuer_did, &member, &["cycling"], now + 10_000, now, None);
    let suspend = decision_env(
        &issuer_key,
        &issuer_did,
        &member,
        ModerationAction::Suspend,
        ModerationScope::Membership,
        "no-shows",
        Some(now + 5),
        now + 1,
        None,
    );
    let evidence = MembershipEvidence {
        credentials: vec![cred],
        decisions: vec![suspend],
        ..Default::default()
    };
    let input = base_input(&issuer_did, &member, now + 100);
    assert!(matches!(evaluate(&evidence, &input), MembershipVerdict::Valid { .. }));
}

#[test]
fn a_listing_scoped_suspension_hides_only_that_listing() {
    let (issuer_key, issuer_did) = generate();
    let member = did();
    let now = 1_000_000;
    let cred =
        credential_env(&issuer_key, &issuer_did, &member, &["cycling"], now + 10_000, now, None);
    let suspend = decision_env(
        &issuer_key,
        &issuer_did,
        &member,
        ModerationAction::Suspend,
        ModerationScope::Listing { listing_id: "lst_a".to_string() },
        "bad-listing",
        None,
        now + 1,
        None,
    );
    let evidence = MembershipEvidence {
        credentials: vec![cred],
        decisions: vec![suspend],
        ..Default::default()
    };

    let listing_a = ListingRef { listing_id: "lst_a", categories: None, areas: None };
    let input_a =
        CheckInput { listing: Some(listing_a), ..base_input(&issuer_did, &member, now + 2) };
    assert!(matches!(evaluate(&evidence, &input_a), MembershipVerdict::Suspended { .. }));

    let listing_b = ListingRef { listing_id: "lst_b", categories: None, areas: None };
    let input_b =
        CheckInput { listing: Some(listing_b), ..base_input(&issuer_did, &member, now + 2) };
    assert!(matches!(evaluate(&evidence, &input_b), MembershipVerdict::Valid { .. }));

    let input_none = base_input(&issuer_did, &member, now + 2);
    assert!(matches!(evaluate(&evidence, &input_none), MembershipVerdict::Valid { .. }));
}

#[test]
fn expiry_and_revoked_plus_expired_precedence() {
    let (issuer_key, issuer_did) = generate();
    let member = did();
    let now = 1_000_000;
    let lifetime = 1_000;
    let cred =
        credential_env(&issuer_key, &issuer_did, &member, &["cycling"], now + lifetime, now, None);
    let evidence = MembershipEvidence { credentials: vec![cred.clone()], ..Default::default() };
    let expired_input = base_input(&issuer_did, &member, now + lifetime);
    assert!(matches!(evaluate(&evidence, &expired_input), MembershipVerdict::Expired { .. }));

    let cred_id = record_id_of(&cred);
    let rev = revocation_env(&issuer_key, &issuer_did, &cred_id, &member, now + lifetime + 1);
    let evidence2 = MembershipEvidence {
        credentials: vec![cred],
        revocations: vec![rev],
        ..Default::default()
    };
    let after_input = base_input(&issuer_did, &member, now + lifetime + 10);
    assert!(matches!(evaluate(&evidence2, &after_input), MembershipVerdict::Revoked { .. }));
}

#[test]
fn category_and_area_outside_scope_and_scope_not_judged_when_none() {
    let (issuer_key, issuer_did) = generate();
    let member = did();
    let now = 1_000_000;
    let named = Area::Named { label: "Bengaluru".to_string(), code: None };
    let cred = credential_env_with_area(
        &issuer_key,
        &issuer_did,
        &member,
        &["cycling"],
        vec![named.clone()],
        now + 1_000,
        now,
    );
    let evidence = MembershipEvidence { credentials: vec![cred], ..Default::default() };

    let out_of_category = ListingRef {
        listing_id: "lst_a",
        categories: Some(&["plumbing".to_string()]),
        areas: Some(std::slice::from_ref(&named)),
    };
    let input =
        CheckInput { listing: Some(out_of_category), ..base_input(&issuer_did, &member, now) };
    match evaluate(&evidence, &input) {
        MembershipVerdict::OutOfScope { outside, .. } => {
            assert_eq!(outside, vec!["plumbing".to_string()])
        }
        other => panic!("expected OutOfScope, got {other:?}"),
    }

    let other_area = Area::Named { label: "Mumbai".to_string(), code: None };
    let out_of_area = ListingRef {
        listing_id: "lst_a",
        categories: Some(&["cycling".to_string()]),
        areas: Some(&[other_area]),
    };
    let input2 = CheckInput { listing: Some(out_of_area), ..base_input(&issuer_did, &member, now) };
    match evaluate(&evidence, &input2) {
        MembershipVerdict::OutOfScope { outside, .. } => {
            assert_eq!(outside, vec!["area".to_string()])
        }
        other => panic!("expected OutOfScope, got {other:?}"),
    }

    let input_no_scope = base_input(&issuer_did, &member, now);
    assert!(matches!(evaluate(&evidence, &input_no_scope), MembershipVerdict::Valid { .. }));
}

#[test]
fn a_newer_credential_supersedes_the_older_and_its_revocation_no_longer_matters() {
    let (issuer_key, issuer_did) = generate();
    let member = did();
    let now = 1_000_000;
    let old =
        credential_env(&issuer_key, &issuer_did, &member, &["cycling"], now + 1_000, now, None);
    let old_id = record_id_of(&old);
    let old_rev = revocation_env(&issuer_key, &issuer_did, &old_id, &member, now + 1);
    let new = credential_env(
        &issuer_key,
        &issuer_did,
        &member,
        &["cycling"],
        now + 2_000,
        now + 5,
        Some(old_id),
    );
    let evidence = MembershipEvidence {
        credentials: vec![old, new.clone()],
        revocations: vec![old_rev],
        ..Default::default()
    };
    let input = base_input(&issuer_did, &member, now + 10);
    match evaluate(&evidence, &input) {
        MembershipVerdict::Valid { credential_record_id, .. } => {
            assert_eq!(credential_record_id, record_id_of(&new));
        }
        other => panic!("expected Valid, got {other:?}"),
    }
}

#[test]
fn no_instant_removal_and_withheld_revocation_notices_match_the_hub_verbatim() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest_dir.join("../roym_web/ui/src/directory/membership.ts");
    assert!(path.exists(), "missing ../roym_web/ui/src/directory/membership.ts");
    let content = fs::read_to_string(&path).unwrap();
    assert!(
        content.contains(NO_INSTANT_REMOVAL_NOTICE),
        "NO_INSTANT_REMOVAL_NOTICE not found verbatim"
    );
    assert!(
        content.contains(WITHHELD_REVOCATION_NOTICE),
        "WITHHELD_REVOCATION_NOTICE not found verbatim"
    );
}

#[test]
fn listed_window_every_case() {
    let (issuer_key, issuer_did) = generate();
    let member = did();
    let now = 1_000_000;
    let listing_id = "lst_a";

    // No credential at all.
    assert_eq!(
        listed_window(&MembershipEvidence::default(), &issuer_did, &member, listing_id, now),
        (0, 0)
    );

    // Valid: (0, expires_at).
    let expires_at = now + 1_000;
    let cred =
        credential_env(&issuer_key, &issuer_did, &member, &["cycling"], expires_at, now, None);
    let ev_valid = MembershipEvidence { credentials: vec![cred.clone()], ..Default::default() };
    assert_eq!(listed_window(&ev_valid, &issuer_did, &member, listing_id, now), (0, expires_at));

    // Revoked: (0, 0).
    let cred_id = record_id_of(&cred);
    let rev = revocation_env(&issuer_key, &issuer_did, &cred_id, &member, now + 1);
    let ev_revoked = MembershipEvidence {
        credentials: vec![cred.clone()],
        revocations: vec![rev],
        ..Default::default()
    };
    assert_eq!(listed_window(&ev_revoked, &issuer_did, &member, listing_id, now), (0, 0));

    // Suspended until lifted (no until_secs): (0, 0).
    let suspend_indef = decision_env(
        &issuer_key,
        &issuer_did,
        &member,
        ModerationAction::Suspend,
        ModerationScope::Membership,
        "r",
        None,
        now + 1,
        None,
    );
    let ev_susp = MembershipEvidence {
        credentials: vec![cred.clone()],
        decisions: vec![suspend_indef],
        ..Default::default()
    };
    assert_eq!(listed_window(&ev_susp, &issuer_did, &member, listing_id, now), (0, 0));

    // Suspended until u: (u, expires_at).
    let until = now + 100;
    let suspend_until = decision_env(
        &issuer_key,
        &issuer_did,
        &member,
        ModerationAction::Suspend,
        ModerationScope::Membership,
        "r",
        Some(until),
        now + 1,
        None,
    );
    let ev_susp_until = MembershipEvidence {
        credentials: vec![cred.clone()],
        decisions: vec![suspend_until],
        ..Default::default()
    };
    assert_eq!(
        listed_window(&ev_susp_until, &issuer_did, &member, listing_id, now),
        (until, expires_at)
    );

    // A listing-scoped suspension affects only its own listing.
    let suspend_listing = decision_env(
        &issuer_key,
        &issuer_did,
        &member,
        ModerationAction::Suspend,
        ModerationScope::Listing { listing_id: listing_id.to_string() },
        "r",
        None,
        now + 1,
        None,
    );
    let ev_susp_listing = MembershipEvidence {
        credentials: vec![cred.clone()],
        decisions: vec![suspend_listing],
        ..Default::default()
    };
    assert_eq!(listed_window(&ev_susp_listing, &issuer_did, &member, listing_id, now), (0, 0));
    assert_eq!(
        listed_window(&ev_susp_listing, &issuer_did, &member, "lst_other", now),
        (0, expires_at)
    );

    // u >= expires_at collapses to (0, 0).
    let suspend_past_expiry = decision_env(
        &issuer_key,
        &issuer_did,
        &member,
        ModerationAction::Suspend,
        ModerationScope::Membership,
        "r",
        Some(expires_at + 10),
        now + 1,
        None,
    );
    let ev_past = MembershipEvidence {
        credentials: vec![cred],
        decisions: vec![suspend_past_expiry],
        ..Default::default()
    };
    assert_eq!(listed_window(&ev_past, &issuer_did, &member, listing_id, now), (0, 0));
}

/// `evaluate(.., listing: Some(listing_id only))` agrees with
/// `listed_window`'s own computed window on the two cases whose window is
/// not simply `(0, 0)`: `Valid` exactly when `from <= now < until`.
#[test]
fn listed_window_agrees_with_evaluate_at_its_own_boundaries() {
    let (issuer_key, issuer_did) = generate();
    let member = did();
    let now = 1_000_000;
    let listing_id = "lst_a";
    let expires_at = now + 1_000;
    let cred =
        credential_env(&issuer_key, &issuer_did, &member, &["cycling"], expires_at, now, None);
    let ev_valid = MembershipEvidence { credentials: vec![cred.clone()], ..Default::default() };

    let until = now + 100;
    let suspend_until = decision_env(
        &issuer_key,
        &issuer_did,
        &member,
        ModerationAction::Suspend,
        ModerationScope::Membership,
        "r",
        Some(until),
        now + 1,
        None,
    );
    let ev_susp_until = MembershipEvidence {
        credentials: vec![cred],
        decisions: vec![suspend_until],
        ..Default::default()
    };

    // `from - 1` is deliberately not probed here: for the unsuspended
    // window it sits inside `verify`'s own clock-skew tolerance around the
    // credential's `issued_at_secs`, which is a signature-freshness
    // allowance, not part of `listed_window`'s own floor.
    for (evidence, from, until) in
        [(&ev_valid, now, expires_at), (&ev_susp_until, until, expires_at)]
    {
        let listing = ListingRef { listing_id, categories: None, areas: None };
        for probe in [from, until.saturating_sub(1), until] {
            let input =
                CheckInput { listing: Some(listing), ..base_input(&issuer_did, &member, probe) };
            let is_valid = matches!(evaluate(evidence, &input), MembershipVerdict::Valid { .. });
            assert_eq!(
                is_valid,
                from <= probe && probe < until,
                "probe {probe} for window ({from}, {until})"
            );
        }
    }
}
