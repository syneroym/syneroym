//! The consumer's own verdict on membership evidence a source serves, when
//! the source is a *second* SynOrg (a real directory with its own owner) or
//! a canned trust source whose evidence has one chosen defect. Everything a
//! directory says about itself is a claim; each scenario here is a claim
//! that must not become a verdict on the consumer's node.

use serde_json::{Value, json};

use super::{
    fixtures::*,
    helpers::*,
    trust_fixtures::*,
    trust_harness::{claimed_issuer_did, provider_did, trust_issuer_did},
};

const CONSUMER_SEARCH_HIT: &str = "the one listing the trust source serves";

/// Adds `target` as a source, searches it and asks it for the provider's
/// standing, on both builds. Returns `(search hit's membership verdict,
/// check-standing's reply)`, having asserted the two builds agree.
async fn canned_verdicts(h: &Harness, target: &str) -> (Value, Value) {
    let (_rw, _rn, mw, mn) = fan_out(h, &[target]).await;
    assert_eq!(stripped(&mw), stripped(&mn));
    let hits = mw["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "{CONSUMER_SEARCH_HIT}: {mw}");
    assert_eq!(hits[0]["verified"], true, "the listing itself is genuine: {mw}");
    let sources = hits[0]["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 1, "{mw}");
    let search_verdict = sources[0]["membership"].clone();

    let (cw, cn) = both_rpc(
        h,
        "directory.check-standing",
        json!({ "source": target, "member_did": provider_did() }),
    )
    .await;
    assert_eq!(stripped(&cw), stripped(&cn));
    (search_verdict, cw["result"].clone())
}

#[tokio::test]
async fn scenario_182_a_consumer_sees_valid_membership_per_source_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    ensure_dir2_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    publish_listing_to_dir2(&h, "hedge-trimming-182", "Version one").await;
    publish_listing_to_primary(&h, "hedge-trimming-182", "Version two").await;

    let (_rw, _rn, mw, mn) = fan_out(&h, &["did:key:hForeignWire", "did:key:hForeignWire2"]).await;
    assert_eq!(stripped(&mw), stripped(&mn));
    let hits = mw["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "one listing, two sources: {mw}");
    let issuer_of = |directory: &str| {
        let sources = hits[0]["sources"].as_array().unwrap();
        let s = sources.iter().find(|s| s["directory"] == directory).expect("source present");
        assert_eq!(s["membership"]["state"], "valid", "{directory}: {mw}");
        s["membership"]["issuer"].as_str().unwrap().to_string()
    };
    assert_eq!(issuer_of("did:key:hForeignWire"), owner_did());
    assert_eq!(issuer_of("did:key:hForeignWire2"), dir2_owner_did());
    assert_ne!(owner_did(), dir2_owner_did(), "the two directories are two different SynOrgs");

    // Each pin came from that directory's own `info`, not from the other.
    let (sw, sn) = both_rpc(&h, "directory.sources", json!({})).await;
    assert_eq!(stripped(&sw), stripped(&sn));
    let pin = |did: &str| {
        let rows = sw["result"]["sources"].as_array().unwrap();
        rows.iter().find(|r| r["did"] == did).unwrap()["issuer_did"].clone()
    };
    assert_eq!(pin("did:key:hForeignWire"), json!(owner_did()));
    assert_eq!(pin("did:key:hForeignWire2"), json!(dir2_owner_did()));
}

#[tokio::test]
async fn scenario_183_a_forged_credential_is_refused_on_the_consumers_node_parity() {
    let h = harness().await;

    // Control: a source whose evidence is sound is `valid`, so the
    // failures below are the evidence's and not the fixture's.
    let control = "did:key:hTrustValid";
    let (search, standing) = canned_verdicts(&h, control).await;
    assert_eq!(search["state"], "valid", "{search}");
    assert_eq!(search["issuer"], json!(trust_issuer_did(control)), "{search}");
    assert_eq!(standing["verdict"]["state"], "valid", "{standing}");

    // A validly signed listing, and a credential whose signature is the
    // *peer's*, not the pinned issuer's.
    let forged = "did:key:hTrustForged";
    let (search, standing) = canned_verdicts(&h, forged).await;
    assert_eq!(search["state"], "refused", "{search}");
    assert_eq!(standing["verdict"]["state"], "refused", "{standing}");
    assert_ne!(peer_did(), claimed_issuer_did(forged));
}

#[tokio::test]
async fn scenario_184_a_directory_asserting_an_expired_credential_does_not_win_parity() {
    let h = harness().await;
    let target = "did:key:hTrustExpired";
    let (search, standing) = canned_verdicts(&h, target).await;
    // Correctly signed by the pinned issuer, so this is `expired`, not `refused`.
    assert_eq!(search["state"], "expired", "{search}");
    assert_eq!(standing["verdict"]["state"], "expired", "{standing}");
    assert!(search["expires_at_secs"].as_u64().unwrap() < 4_000_000_000, "{search}");
}

#[tokio::test]
async fn scenario_185_a_directory_asserting_an_out_of_scope_credential_does_not_win_parity() {
    let h = harness().await;
    let target = "did:key:hTrustOutOfScope";
    let (search, standing) = canned_verdicts(&h, target).await;
    assert_eq!(search["state"], "out-of-scope", "{search}");
    assert_eq!(search["outside"], json!(["gardening"]), "names the category: {search}");
    // A listing-free membership check does not judge scope: the credential
    // is real and current, so it stays `valid` there. Scope is the
    // listing's question, asked only on a search hit.
    assert_eq!(standing["verdict"]["state"], "valid", "{standing}");
}

#[tokio::test]
async fn scenario_187_a_changed_issuer_is_never_re_pinned_parity() {
    let h = harness().await;
    ensure_synorg(&h).await;
    ensure_dir2_synorg(&h).await;
    enrol_signing(&h, "catalog").await;
    publish_listing_to_dir2(&h, "pin-187", "Pinned").await;
    let second = "did:key:hForeignWire2";

    // The wrong SynOrg's owner, chosen explicitly at add time.
    let (aw, an) =
        both_rpc(&h, "directory.add-source", json!({ "did": second, "issuer_did": owner_did() }))
            .await;
    assert_eq!(stripped(&aw), stripped(&an));
    assert_eq!(aw["result"]["source"]["issuer_did"], json!(owner_did()), "{aw}");
    assert!(aw["result"]["probe"].as_str().unwrap().contains(&dir2_owner_did()), "{aw}");

    // (a) The search reply names no issuer, so the mismatch shows as a
    // refused check, never as "issuer changed".
    let (_rw, mw) = fan_out_one(&h, true, &[second]).await;
    let (_rn, mn) = fan_out_one(&h, false, &[second]).await;
    assert_eq!(stripped(&mw), stripped(&mn));
    assert_eq!(mw["result"]["hits"][0]["sources"][0]["membership"]["state"], "refused", "{mw}");

    // (b) `directory.standing` does name its issuer, and it is not the pin.
    let (cw, cn) = both_rpc(
        &h,
        "directory.check-standing",
        json!({ "source": second, "member_did": owner_did() }),
    )
    .await;
    assert_eq!(stripped(&cw), stripped(&cn));
    assert_eq!(cw["result"]["verdict"]["state"], "unknown", "{cw}");
    assert_eq!(cw["result"]["verdict"]["reason"], "issuer-changed", "{cw}");
    assert_eq!(cw["result"]["refreshed"], false, "{cw}");

    // (c) Neither reply, and not a second `add-source` without an explicit
    // issuer, moved the pin.
    both_rpc(&h, "directory.add-source", json!({ "did": second })).await;
    let (sw, sn) = both_rpc(&h, "directory.sources", json!({})).await;
    assert_eq!(stripped(&sw), stripped(&sn));
    let row = &sw["result"]["sources"].as_array().unwrap()[0];
    assert_eq!(row["issuer_did"], json!(owner_did()), "{sw}");
}

#[tokio::test]
async fn scenario_193_a_credential_from_one_synorg_served_by_another_directory_is_refused_parity() {
    let h = harness().await;
    let target = "did:key:hTrustWrongSynOrg";
    // The source is pinned to the second SynOrg's owner, but serves a
    // credential the first SynOrg's owner really signed.
    assert_eq!(claimed_issuer_did(target), dir2_owner_did());
    let (search, standing) = canned_verdicts(&h, target).await;
    assert_eq!(search["state"], "refused", "{search}");
    assert_eq!(standing["verdict"]["state"], "refused", "{standing}");
    assert_eq!(standing["refreshed"], true, "the reply named the pinned issuer: {standing}");
}
