//! The member-standing decision cap: which suspensions survive into the
//! bounded evidence a consumer or a search hit receives, across several
//! rounds of tightening (an old unlifted suspension outliving newer
//! decisions, a membership-wide suspension outranking listing-scoped
//! ones, and a permanent suspension outranking a newer timed one). Kept
//! apart from the other trust scenarios in `trust.rs`.

use serde_json::json;
use syneroym_roym_core::{membership::MAX_EVIDENCE_DECISIONS, services};

use super::{fixtures::*, helpers::*, trust_fixtures::*};

#[tokio::test]
async fn scenario_194_an_unlifted_suspension_survives_past_the_decision_cap_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    publish_listing_to_primary(&h, "hedge-trimming-194", "Hedge trimming").await;

    let (sw, _) = both_rpc(
        &h,
        "member.suspend",
        json!({ "member_did": owner_did(), "rule": "permanent", "reason": "t" }),
    )
    .await;
    let permanent_id = sw["result"]["record_id"].as_str().unwrap().to_string();

    // Push more decisions than the standing cap through afterwards -- a
    // naive newest-first truncation would drop the never-lifted suspension
    // above, and the member would look valid again with nobody having
    // signed a lift.
    for i in 0..=MAX_EVIDENCE_DECISIONS {
        let (sw2, _) = both_rpc(
            &h,
            "member.suspend",
            json!({
                "member_did": owner_did(), "rule": "temp", "reason": "t",
                "scope": { "kind": "listing", "listing_id": format!("noise-{i}") },
            }),
        )
        .await;
        let id2 = sw2["result"]["record_id"].as_str().unwrap().to_string();
        both_rpc(&h, "member.lift", json!({ "decision_record_id": id2, "reason": "ok" })).await;
    }

    let (w, n) = wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({}))).await;
    assert_eq!(stripped(&w), stripped(&n));
    assert_eq!(
        w["result"]["hits"].as_array().unwrap().len(),
        0,
        "the never-lifted suspension must still hide the listing: {w}"
    );

    let (stw, stn) = both_rpc(&h, "directory.standing", json!({ "member_did": owner_did() })).await;
    assert_eq!(stripped(&stw), stripped(&stn));
    let still_present =
        stw["result"]["evidence"]["decisions"].as_array().unwrap().iter().any(|d| {
            syneroym_signed_record::Envelope::from_json(d.as_str().unwrap()).unwrap().record_id()
                == Ok(permanent_id.clone())
        });
    assert!(still_present, "the permanent suspension's own record must survive: {stw}");
}

#[tokio::test]
async fn scenario_195_revoking_a_superseded_credential_is_refused_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;

    let (cw1, _) = both_rpc(&h, "credential.list", json!({ "member_did": owner_did() })).await;
    let old_id = cw1["result"]["records"][0]["record_id"].as_str().unwrap().to_string();

    // Reissuing supersedes the old credential; `pick_current` already
    // ignores it, so revoking it would change no verdict anywhere.
    let (issue_w, _) = issue_credential(&h, &owner_did()).await;
    let new_id = issue_w["result"]["record_id"].as_str().unwrap().to_string();

    let (w, n) = both_rpc(
        &h,
        "revocation.issue",
        json!({ "credential_record_id": old_id, "reason": "stale" }),
    )
    .await;
    assert_eq!(stripped(&w), stripped(&n));
    assert!(is_err(&w, -32602), "revoking a superseded credential must be refused: {w}");

    let (w2, n2) = both_rpc(
        &h,
        "revocation.issue",
        json!({ "credential_record_id": new_id, "reason": "gone" }),
    )
    .await;
    assert_eq!(stripped(&w2), stripped(&n2));
    assert!(w2["result"]["record_id"].is_string(), "the current credential must revoke: {w2}");
}

#[tokio::test]
async fn scenario_196_a_non_numeric_until_secs_is_refused_not_silently_permanent_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;

    let (w, n) = both_rpc(
        &h,
        "member.suspend",
        json!({
            "member_did": owner_did(), "rule": "r1", "reason": "t",
            "until_secs": "1790000000",
        }),
    )
    .await;
    assert_eq!(stripped(&w), stripped(&n));
    assert!(
        is_err(&w, -32602),
        "a non-numeric until_secs must be refused, not silently become 'until lifted': {w}"
    );
}

#[tokio::test]
async fn scenario_197_directory_standing_refuses_a_member_did_that_is_not_a_did_key_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;

    let (w, n) = wire_invoke(
        &h,
        services::DIRECTORY,
        &env("directory.standing", json!({ "member_did": "not-a-did" })),
    )
    .await;
    assert_eq!(stripped(&w), stripped(&n));
    assert!(is_err(&w, -32602), "{w}");
}

#[tokio::test]
async fn scenario_198_a_membership_wide_suspension_survives_many_concurrent_listing_suspensions_parity()
 {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    publish_listing_to_primary(&h, "hedge-trimming-198", "Hedge trimming").await;

    let (sw, _) = both_rpc(
        &h,
        "member.suspend",
        json!({ "member_did": owner_did(), "rule": "permanent", "reason": "t" }),
    )
    .await;
    let membership_suspension_id = sw["result"]["record_id"].as_str().unwrap().to_string();

    // MAX_EVIDENCE_DECISIONS listing-scoped suspensions, left active (never
    // lifted) -- together with the membership-wide one above, that is one
    // more decision than the cap, and every single one is active.
    for i in 0..MAX_EVIDENCE_DECISIONS {
        both_rpc(
            &h,
            "member.suspend",
            json!({
                "member_did": owner_did(), "rule": "temp", "reason": "t",
                "scope": { "kind": "listing", "listing_id": format!("other-listing-{i}") },
            }),
        )
        .await;
    }

    // The membership-wide suspension alone must still hide every listing,
    // however many listing-scoped suspensions also exist.
    let (w, n) = wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({}))).await;
    assert_eq!(stripped(&w), stripped(&n));
    assert_eq!(
        w["result"]["hits"].as_array().unwrap().len(),
        0,
        "the membership-wide suspension must still hide every listing: {w}"
    );

    let (stw, stn) = both_rpc(&h, "directory.standing", json!({ "member_did": owner_did() })).await;
    assert_eq!(stripped(&stw), stripped(&stn));
    let decisions = stw["result"]["evidence"]["decisions"].as_array().unwrap();
    assert!(
        decisions.len() <= MAX_EVIDENCE_DECISIONS,
        "the standing reply must stay within the cap: {stw}"
    );
    let membership_decision_present = decisions.iter().any(|d| {
        syneroym_signed_record::Envelope::from_json(d.as_str().unwrap()).unwrap().record_id()
            == Ok(membership_suspension_id.clone())
    });
    assert!(
        membership_decision_present,
        "the membership-wide suspension's own record must ride in the evidence: {stw}"
    );
}

#[tokio::test]
async fn scenario_199_a_permanent_membership_suspension_survives_a_later_timed_one_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    publish_listing_to_primary(&h, "hedge-trimming-199", "Hedge trimming").await;

    let (sw, _) = both_rpc(
        &h,
        "member.suspend",
        json!({ "member_did": owner_did(), "rule": "permanent", "reason": "t" }),
    )
    .await;
    let permanent_id = sw["result"]["record_id"].as_str().unwrap().to_string();

    for i in 0..(MAX_EVIDENCE_DECISIONS - 1) {
        both_rpc(
            &h,
            "member.suspend",
            json!({
                "member_did": owner_did(), "rule": "temp", "reason": "t",
                "scope": { "kind": "listing", "listing_id": format!("other-listing-{i}") },
            }),
        )
        .await;
    }

    // A second, *newer* membership-scope suspension, but a timed one --
    // it must never be treated as making the permanent suspension above
    // redundant, or once it ends nothing is left to keep the member
    // hidden and nobody ever lifted the permanent one.
    both_rpc(
        &h,
        "member.suspend",
        json!({
            "member_did": owner_did(), "rule": "temp-membership", "reason": "t",
            "until_secs": fixture_credential_expires_at_secs(),
        }),
    )
    .await;

    let (stw, stn) = both_rpc(&h, "directory.standing", json!({ "member_did": owner_did() })).await;
    assert_eq!(stripped(&stw), stripped(&stn));
    let decisions = stw["result"]["evidence"]["decisions"].as_array().unwrap();
    assert!(decisions.len() <= MAX_EVIDENCE_DECISIONS, "{stw}");
    let permanent_present = decisions.iter().any(|d| {
        syneroym_signed_record::Envelope::from_json(d.as_str().unwrap()).unwrap().record_id()
            == Ok(permanent_id.clone())
    });
    assert!(
        permanent_present,
        "the permanent membership suspension must survive a newer timed one: {stw}"
    );
}

#[tokio::test]
async fn scenario_200_a_timed_membership_suspension_outranks_newer_listing_suspensions_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    publish_listing_to_primary(&h, "hedge-trimming-200", "Hedge trimming").await;

    // A timed membership-wide suspension, issued first -- no permanent
    // suspension exists anywhere here, so the shortcut above never fires
    // and this member's fallback ordering is the only thing protecting it.
    let (sw, _) = both_rpc(
        &h,
        "member.suspend",
        json!({
            "member_did": owner_did(), "rule": "temp-membership", "reason": "t",
            "until_secs": fixture_credential_expires_at_secs(),
        }),
    )
    .await;
    let membership_suspension_id = sw["result"]["record_id"].as_str().unwrap().to_string();

    // MAX_EVIDENCE_DECISIONS listing-scoped suspensions, all issued after
    // it and left active -- by age alone every one of these outranks the
    // membership suspension above and pushes it out of the cap.
    for i in 0..MAX_EVIDENCE_DECISIONS {
        both_rpc(
            &h,
            "member.suspend",
            json!({
                "member_did": owner_did(), "rule": "temp", "reason": "t",
                "scope": { "kind": "listing", "listing_id": format!("other-listing-200-{i}") },
            }),
        )
        .await;
    }

    let (stw, stn) = both_rpc(&h, "directory.standing", json!({ "member_did": owner_did() })).await;
    assert_eq!(stripped(&stw), stripped(&stn));
    let decisions = stw["result"]["evidence"]["decisions"].as_array().unwrap();
    assert!(decisions.len() <= MAX_EVIDENCE_DECISIONS, "{stw}");
    let membership_present = decisions.iter().any(|d| {
        syneroym_signed_record::Envelope::from_json(d.as_str().unwrap()).unwrap().record_id()
            == Ok(membership_suspension_id.clone())
    });
    assert!(
        membership_present,
        "the timed membership suspension must outrank newer listing-scope ones: {stw}"
    );

    // While it holds, the member must be hidden entirely, not just from
    // the listings suspended individually.
    let (w, n) = wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({}))).await;
    assert_eq!(stripped(&w), stripped(&n));
    assert_eq!(w["result"]["hits"].as_array().unwrap().len(), 0, "{w}");
}
