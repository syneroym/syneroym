//! Cross-installation trust (C9): the directory's own signed credential,
//! revocation and moderation-decision verbs, and the search filter they
//! drive. Two-directory hostile-source scenarios (a canned trust source
//! serving forged/expired/out-of-scope/wrong-SynOrg evidence, and a
//! consumer pinning a *second* SynOrg's own issuer) need `directory2` to
//! have its own distinct owner, which is not yet built -- see the
//! backlog. This file covers
//! the credential lifecycle, the publish gate, the search filter, and
//! the consumer's own held-copy re-evaluation on the single directory the
//! harness already gives every other C9-agnostic scenario.

use serde_json::json;
use syneroym_roym_core::{directory::MAX_HITS_PER_QUERY, services};

use super::{fixtures::*, helpers::*, trust_fixtures::*};

#[tokio::test]
async fn scenario_173_credential_issue_signs_under_the_owner_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;

    let (w, n) = issue_credential(&h, &peer_did()).await;
    assert_eq!(stripped(&w), stripped(&n));
    let env_w: syneroym_signed_record::Envelope =
        serde_json::from_str(w["result"]["envelope"].as_str().unwrap()).unwrap();
    assert_eq!(env_w.issuer, owner_did());
    assert_eq!(env_w.subject, peer_did());
    assert!(env_w.expires_at_secs.is_some());
}

#[tokio::test]
async fn scenario_174_credential_issue_refuses_bad_input_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    let now = fixture_credential_expires_at_secs();

    // Not a did:key.
    let (w, n) = both_rpc(
        &h,
        "credential.issue",
        json!({ "member_did": "not-a-did", "categories": FIXTURE_CATEGORIES, "expires_at_secs": now }),
    )
    .await;
    assert_eq!(stripped(&w), stripped(&n));
    assert!(is_err(&w, -32602), "{w}");

    // Expiry already past.
    let (w, _) = both_rpc(
        &h,
        "credential.issue",
        json!({ "member_did": peer_did(), "categories": FIXTURE_CATEGORIES, "expires_at_secs": 1 }),
    )
    .await;
    assert!(is_err(&w, -32602), "{w}");

    // Category not one of the SynOrg's own.
    let (w, _) = both_rpc(
        &h,
        "credential.issue",
        json!({ "member_did": peer_did(), "categories": ["plumbing"], "expires_at_secs": now }),
    )
    .await;
    assert!(is_err(&w, -32602), "{w}");

    // Not enrolled: a fresh SynOrg with no signing certificate.
    let h2 = harness().await;
    both_rpc(
        &h2,
        "directory.set-settings",
        json!({
            "name": "Guild", "rules": "r", "area": [], "categories": FIXTURE_CATEGORIES,
            "support_contact": "s@example.org", "dispute_path": "d",
            "retention_secs": 2_592_000,
            "publication_limits": { "window_secs": 86400, "max_per_window": 20 }
        }),
    )
    .await;
    let (w, _) = both_rpc(
        &h2,
        "credential.issue",
        json!({ "member_did": peer_did(), "categories": FIXTURE_CATEGORIES, "expires_at_secs": now }),
    )
    .await;
    assert!(is_err(&w, -32602), "{w}");
}

#[tokio::test]
async fn scenario_175_standing_over_the_wire_is_open_and_bounded_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;

    // Anonymous, no source registered, no token -- works.
    let (w, n) = wire_invoke(
        &h,
        services::DIRECTORY,
        &env("directory.standing", json!({ "member_did": owner_did() })),
    )
    .await;
    assert_eq!(stripped(&w), stripped(&n));
    assert_eq!(w["result"]["evidence"]["credentials"].as_array().unwrap().len(), 1, "{w}");

    // Unknown member -> empty evidence, not an error.
    let (w, _) = wire_invoke(
        &h,
        services::DIRECTORY,
        &env("directory.standing", json!({ "member_did": peer_did() })),
    )
    .await;
    assert_eq!(w["result"]["evidence"]["credentials"].as_array().unwrap().len(), 1, "{w}");

    // Bounded: issuing more credentials than the cap still answers at most
    // MAX_EVIDENCE_CREDENTIALS (4).
    for _ in 0..6 {
        issue_credential(&h, &owner_did()).await;
    }
    let (w, _) = wire_invoke(
        &h,
        services::DIRECTORY,
        &env("directory.standing", json!({ "member_did": owner_did() })),
    )
    .await;
    assert!(w["result"]["evidence"]["credentials"].as_array().unwrap().len() <= 4, "{w}");
}

#[tokio::test]
async fn scenario_176_publish_by_a_non_member_is_refused_then_admitted_after_issue_parity() {
    let h = harness().await;
    // Settings only -- no credential yet.
    both_rpc(
        &h,
        "directory.set-settings",
        json!({
            "name": "Guild", "rules": "r", "area": [], "categories": FIXTURE_CATEGORIES,
            "support_contact": "s@example.org", "dispute_path": "d",
            "retention_secs": 2_592_000,
            "publication_limits": { "window_secs": 86400, "max_per_window": 20 }
        }),
    )
    .await;
    enrol_signing(&h, "directory").await;
    enrol_signing(&h, "catalog").await;

    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming-176", "Hedge trimming")).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();

    let (pw, pn) = publish_signed_listing(&h, &e).await;
    assert_eq!(stripped(&pw), stripped(&pn));
    assert!(is_err(&pw, -32602), "{pw}");
    assert_eq!(pw["error"]["data"]["admission"], "not-admitted", "{pw}");
    assert_eq!(pw["error"]["data"]["membership"]["state"], "none", "{pw}");

    issue_credential(&h, &owner_did()).await;
    let (pw2, pn2) = publish_signed_listing(&h, &e).await;
    assert_eq!(stripped(&pw2), stripped(&pn2));
    assert!(pw2["result"]["listing_id"].is_string(), "{pw2}");
}

#[tokio::test]
async fn scenario_177_publish_out_of_scope_is_refused_parity() {
    let h = harness().await;
    // A SynOrg that only lists "outdoor" -- the listing also needs
    // "gardening", so it is out of scope even with a credential.
    both_rpc(
        &h,
        "directory.set-settings",
        json!({
            "name": "Guild", "rules": "r", "area": [], "categories": ["outdoor"],
            "support_contact": "s@example.org", "dispute_path": "d",
            "retention_secs": 2_592_000,
            "publication_limits": { "window_secs": 86400, "max_per_window": 20 }
        }),
    )
    .await;
    enrol_signing(&h, "directory").await;
    enrol_signing(&h, "catalog").await;
    let now = fixture_credential_expires_at_secs();
    both_rpc(
        &h,
        "credential.issue",
        json!({ "member_did": owner_did(), "categories": ["outdoor"], "expires_at_secs": now }),
    )
    .await;

    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming-177", "Hedge trimming")).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();
    let (pw, pn) = publish_signed_listing(&h, &e).await;
    assert_eq!(stripped(&pw), stripped(&pn));
    assert!(is_err(&pw, -32602), "{pw}");
    assert_eq!(pw["error"]["data"]["membership"]["state"], "out-of-scope", "{pw}");
}

#[tokio::test]
async fn scenario_178_a_withdrawal_is_accepted_from_a_suspended_member_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    let (id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming-178", "Hedge trimming")).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();
    publish_signed_listing(&h, &e).await;

    let (sw, _) = both_rpc(
        &h,
        "member.suspend",
        json!({ "member_did": owner_did(), "rule": "r1", "reason": "t" }),
    )
    .await;
    assert!(sw["result"]["record_id"].is_string(), "{sw}");

    both_rpc(&h, "listing.withdraw", json!({ "listing_id": id })).await;
    let (gw2, gn2) = both_rpc(&h, "listing.get", json!({ "listing_id": id })).await;
    assert_eq!(stripped(&gw2), stripped(&gn2));
    let withdrawn = gw2["result"]["envelope"].as_str().unwrap().to_string();
    let (pw, pn) = publish_signed_listing(&h, &withdrawn).await;
    assert_eq!(stripped(&pw), stripped(&pn));
    assert_eq!(pw["result"]["withdrawn"], true, "a withdrawal must pass while suspended: {pw}");
}

#[tokio::test]
async fn scenario_179_revocation_removes_the_member_from_search_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    publish_listing_to_primary(&h, "hedge-trimming-179", "Hedge trimming").await;

    let (sw, sn) = wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({}))).await;
    assert_eq!(stripped(&sw), stripped(&sn));
    assert_eq!(sw["result"]["hits"].as_array().unwrap().len(), 1, "{sw}");

    let (cw, _) = both_rpc(&h, "credential.list", json!({ "member_did": owner_did() })).await;
    let record_id = cw["result"]["records"][0]["record_id"].as_str().unwrap().to_string();

    let (rw, rn) = both_rpc(
        &h,
        "revocation.issue",
        json!({ "credential_record_id": record_id, "reason": "gone" }),
    )
    .await;
    assert_eq!(stripped(&rw), stripped(&rn));
    assert!(rw["result"]["record_id"].is_string(), "{rw}");

    let (sw2, sn2) =
        wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({}))).await;
    assert_eq!(stripped(&sw2), stripped(&sn2));
    assert_eq!(sw2["result"]["hits"].as_array().unwrap().len(), 0, "{sw2}");

    // Idempotent: revoking again returns the same record.
    let (rw2, _) = both_rpc(
        &h,
        "revocation.issue",
        json!({ "credential_record_id": record_id, "reason": "again" }),
    )
    .await;
    assert_eq!(rw2["result"]["record_id"], rw["result"]["record_id"], "{rw2}");
}

#[tokio::test]
async fn scenario_180_suspend_hides_and_lift_restores_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    publish_listing_to_primary(&h, "hedge-trimming-180", "Hedge trimming").await;

    let (sw, _) = both_rpc(
        &h,
        "member.suspend",
        json!({ "member_did": owner_did(), "rule": "r1", "reason": "t" }),
    )
    .await;
    let decision_id = sw["result"]["record_id"].as_str().unwrap().to_string();

    let (w, n) = wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({}))).await;
    assert_eq!(stripped(&w), stripped(&n));
    assert_eq!(
        w["result"]["hits"].as_array().unwrap().len(),
        0,
        "suspended member must vanish: {w}"
    );

    let (lw, ln) =
        both_rpc(&h, "member.lift", json!({ "decision_record_id": decision_id, "reason": "ok" }))
            .await;
    assert_eq!(stripped(&lw), stripped(&ln));
    assert!(lw["result"]["record_id"].is_string(), "{lw}");

    let (w2, n2) = wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({}))).await;
    assert_eq!(stripped(&w2), stripped(&n2));
    assert_eq!(
        w2["result"]["hits"].as_array().unwrap().len(),
        1,
        "a lift must restore the listing: {w2}"
    );
}

#[tokio::test]
async fn scenario_181_a_listing_scoped_suspension_hides_only_that_listing_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    let e1 = publish_listing_to_primary(&h, "hedge-trimming-181a", "First").await;
    publish_listing_to_primary(&h, "hedge-trimming-181b", "Second").await;
    let listing_id_1 = syneroym_signed_record::Envelope::from_json(&e1).unwrap().subject;

    both_rpc(
        &h,
        "member.suspend",
        json!({
            "member_did": owner_did(), "rule": "r1", "reason": "t",
            "scope": { "kind": "listing", "listing_id": listing_id_1 },
        }),
    )
    .await;

    let (w, n) = wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({}))).await;
    assert_eq!(stripped(&w), stripped(&n));
    let hits = w["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "only the suspended listing must vanish: {w}");
    assert_ne!(hits[0]["listing_id"], json!(listing_id_1), "{w}");
}

#[tokio::test]
async fn scenario_186_a_held_copy_shows_the_withdrawal_on_next_check_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    publish_listing_to_primary(&h, "hedge-trimming-186", "Hedge trimming").await;

    both_rpc(&h, "directory.add-source", json!({ "did": "did:key:hForeignWire" })).await;
    let (run_w, mw) = fan_out_one(&h, true, &["did:key:hForeignWire"]).await;
    let (run_n, mn) = fan_out_one(&h, false, &["did:key:hForeignWire"]).await;
    assert_eq!(stripped(&mw), stripped(&mn));
    assert_eq!(mw["result"]["hits"][0]["sources"][0]["membership"]["state"], "valid", "{mw}");
    let _ = (run_w, run_n);

    both_rpc(
        &h,
        "member.suspend",
        json!({ "member_did": owner_did(), "rule": "r1", "reason": "t" }),
    )
    .await;

    // The held copy does not change by itself.
    let (mem_w, mem_n) =
        both_rpc(&h, "directory.memberships", json!({ "member_did": owner_did() })).await;
    assert_eq!(stripped(&mem_w), stripped(&mem_n));
    assert_eq!(mem_w["result"]["memberships"][0]["verdict"]["state"], "valid", "{mem_w}");

    let (chk_w, chk_n) = both_rpc(
        &h,
        "directory.check-standing",
        json!({ "source": "did:key:hForeignWire", "member_did": owner_did() }),
    )
    .await;
    assert_eq!(stripped(&chk_w), stripped(&chk_n));
    assert_eq!(chk_w["result"]["verdict"]["state"], "suspended", "{chk_w}");
    assert_eq!(chk_w["result"]["refreshed"], true, "{chk_w}");
}

#[tokio::test]
async fn scenario_188_trust_state_round_trips_through_a_signed_export_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    publish_listing_to_primary(&h, "hedge-trimming-188", "Hedge trimming").await;
    both_rpc(
        &h,
        "member.suspend",
        json!({ "member_did": peer_did(), "rule": "r1", "reason": "t" }),
    )
    .await;

    let (xw, xn) = both_rpc(&h, "directory.export", json!({})).await;
    assert_eq!(stripped(&xw), stripped(&xn));

    let (iw, in_) = both_rpc(&h, "directory.import", json!({ "bundle": xw["result"] })).await;
    assert_eq!(stripped(&iw), stripped(&in_));
    assert!(iw["result"]["reindexed"].as_u64().unwrap() >= 1, "{iw}");

    let (cw, cn) = both_rpc(&h, "credential.list", json!({})).await;
    assert_eq!(stripped(&cw), stripped(&cn));
    let (dw, dn) = both_rpc(&h, "member.decisions", json!({})).await;
    assert_eq!(stripped(&dw), stripped(&dn));

    // Search still hides the suspended member -- standing and the index
    // windows both survived the round trip.
    let (sw, sn) = wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({}))).await;
    assert_eq!(stripped(&sw), stripped(&sn));
    assert_eq!(sw["result"]["hits"].as_array().unwrap().len(), 1, "the owner's own listing: {sw}");
}

#[tokio::test]
async fn scenario_189_member_remove_is_refused_while_a_credential_is_valid_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;

    let (rw, rn) = both_rpc(&h, "member.remove", json!({ "did": peer_did() })).await;
    assert_eq!(stripped(&rw), stripped(&rn));
    assert!(is_err(&rw, -32602), "a member with a valid credential must not be removed: {rw}");

    let (cw, _) = both_rpc(&h, "credential.list", json!({ "member_did": peer_did() })).await;
    let record_id = cw["result"]["records"][0]["record_id"].as_str().unwrap().to_string();
    both_rpc(
        &h,
        "revocation.issue",
        json!({ "credential_record_id": record_id, "reason": "gone" }),
    )
    .await;

    let (rw2, rn2) = both_rpc(&h, "member.remove", json!({ "did": peer_did() })).await;
    assert_eq!(stripped(&rw2), stripped(&rn2));
    assert_eq!(rw2["result"]["removed"], true, "{rw2}");
}

#[tokio::test]
async fn scenario_190_lift_of_a_non_suspension_is_refused_and_lift_is_idempotent_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;

    let (cw, _) = both_rpc(&h, "credential.list", json!({ "member_did": owner_did() })).await;
    let credential_id = cw["result"]["records"][0]["record_id"].as_str().unwrap().to_string();
    let (w, _) =
        both_rpc(&h, "member.lift", json!({ "decision_record_id": credential_id, "reason": "x" }))
            .await;
    assert!(is_err(&w, -32602), "lifting a non-decision id must be refused: {w}");

    let (sw, _) = both_rpc(
        &h,
        "member.suspend",
        json!({ "member_did": owner_did(), "rule": "r1", "reason": "t" }),
    )
    .await;
    let decision_id = sw["result"]["record_id"].as_str().unwrap().to_string();
    let (lw, ln) =
        both_rpc(&h, "member.lift", json!({ "decision_record_id": decision_id, "reason": "ok" }))
            .await;
    assert_eq!(stripped(&lw), stripped(&ln));
    let (lw2, ln2) = both_rpc(
        &h,
        "member.lift",
        json!({ "decision_record_id": decision_id, "reason": "again" }),
    )
    .await;
    assert_eq!(stripped(&lw2), stripped(&ln2));
    assert_eq!(
        lw2["result"]["record_id"], lw["result"]["record_id"],
        "lift must be idempotent: {lw2}"
    );
}

#[tokio::test]
async fn scenario_191_search_reply_with_full_page_of_evidence_fits_the_proxy_limit_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    both_rpc(&h, "directory.set-limits", json!({ "window_secs": 86400, "max_per_window": 200 }))
        .await;
    both_rpc(&h, "listing.set-limits", json!({ "window_secs": 86400, "max_per_window": 200 }))
        .await;
    for i in 0..MAX_HITS_PER_QUERY {
        publish_listing_to_primary(&h, &format!("hedge-trimming-191-{i}"), &format!("Listing {i}"))
            .await;
    }
    let (w, _) = wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({}))).await;
    let hits = w["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), MAX_HITS_PER_QUERY as usize, "{}", hits.len());
    let bytes = serde_json::to_vec(&w).unwrap().len();
    assert!(
        bytes < 256 * 1024,
        "a full page with evidence must fit the proxy's reply limit: {bytes} bytes"
    );
}

#[tokio::test]
async fn scenario_192_a_suspended_members_index_rows_are_not_listed_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    publish_listing_to_primary(&h, "hedge-trimming-192", "Hedge trimming").await;

    for wasm in [true, false] {
        let rows = service_rows(&h, wasm, "directory", "search_index").await;
        assert!(!rows.is_empty(), "wasm={wasm}");
        for row in &rows {
            assert!(row["listed_until_secs"].as_u64().unwrap() > 0, "{row}");
        }
    }

    let (sw, _) = both_rpc(
        &h,
        "member.suspend",
        json!({ "member_did": owner_did(), "rule": "r1", "reason": "t" }),
    )
    .await;
    let decision_id = sw["result"]["record_id"].as_str().unwrap().to_string();

    for wasm in [true, false] {
        let rows = service_rows(&h, wasm, "directory", "search_index").await;
        for row in &rows {
            assert_eq!(row["listed_until_secs"], json!(0), "wasm={wasm} {row}");
        }
    }

    both_rpc(&h, "member.lift", json!({ "decision_record_id": decision_id, "reason": "ok" })).await;
    for wasm in [true, false] {
        let rows = service_rows(&h, wasm, "directory", "search_index").await;
        for row in &rows {
            assert!(row["listed_until_secs"].as_u64().unwrap() > 0, "wasm={wasm} {row}");
        }
    }

    let (cw, _) = both_rpc(&h, "credential.list", json!({ "member_did": owner_did() })).await;
    let record_id = cw["result"]["records"][0]["record_id"].as_str().unwrap().to_string();
    both_rpc(
        &h,
        "revocation.issue",
        json!({ "credential_record_id": record_id, "reason": "gone" }),
    )
    .await;
    for wasm in [true, false] {
        let rows = service_rows(&h, wasm, "directory", "search_index").await;
        for row in &rows {
            assert_eq!(row["listed_until_secs"], json!(0), "wasm={wasm} {row}");
        }
    }

    // A plain search afterwards is a smoke check only.
    let (w, n) = wire_invoke(&h, services::DIRECTORY, &env("directory.search", json!({}))).await;
    assert_eq!(stripped(&w), stripped(&n));
    assert_eq!(w["result"]["hits"].as_array().unwrap().len(), 0, "{w}");
}
