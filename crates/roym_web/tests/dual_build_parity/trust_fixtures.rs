//! Cross-installation trust fixtures: membership credentials, the second
//! directory's signing enrolment, and the directory verb lists scenario
//! 118 checks. Split out of `fixtures.rs` (792 lines before this slice, no
//! room to grow) and kept out of `directory.rs` (already at its own cap).

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use syneroym_data_db::host_store::QueryOptions;
use syneroym_identity::{
    Identity,
    delegation::{DelegationCertificate, SCOPE_RECORD_SIGNING},
    substrate::{derive_did_key, resolve_did_key},
};

use super::{fixtures::*, helpers::*};

/// The categories `full_listing_params` (and so `publish_listing_to_*`)
/// signs. A SynOrg that does not list these itself refuses `credential.
/// issue` for them, so every fixture that expects a listing to
/// publish grants exactly these.
pub(crate) const FIXTURE_CATEGORIES: &[&str] = &["gardening", "outdoor"];

/// A year out from wall time -- comfortably inside
/// `MAX_CREDENTIAL_LIFETIME_SECS` (2 years) past the pinned `RecordClock`
/// (`wall_now + 240`, see `Harness::new`) and never hit by a scenario's
/// own clock arithmetic. Computed, not a literal epoch: a hardcoded
/// future date eventually stops being far enough in the future.
pub(crate) fn fixture_credential_expires_at_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 365 * 24 * 3600
}

/// A third person `ensure_synorg` never grants a credential -- `owner_did()`
/// and `peer_did()` both get one, so a scenario asserting the "unknown
/// member" path needs someone else entirely. Fixed bytes so both builds
/// mint the same DID.
pub(crate) fn stranger_identity() -> Identity {
    Identity::from_bytes(&[99; 32])
}

pub(crate) fn stranger_did() -> String {
    derive_did_key(&stranger_identity().public_key())
}

/// The second directory's own owner: a distinct SynOrg, so a credential
/// one directory issues is provably not the other's. `directory2` is
/// registered under this DID (`helpers.rs`), which is also the master its
/// signing certificate must chain to. Fixed bytes so both builds mint the
/// same DID.
pub(crate) fn dir2_owner_identity() -> Identity {
    Identity::from_bytes(&[43; 32])
}

pub(crate) fn dir2_owner_did() -> String {
    derive_did_key(&dir2_owner_identity().public_key())
}

/// Enrols the primary directory's signing certificate and issues
/// `owner_did()` a `FIXTURE_CATEGORIES` credential on it -- the one-liner
/// a scenario that builds its own (non-`ensure_synorg`) settings needs
/// before its first publish.
pub(crate) async fn grant_owner_credential(h: &Harness) {
    enrol_signing(h, "directory").await;
    issue_credential(h, &owner_did()).await;
}

/// Issues a `FIXTURE_CATEGORIES` credential to `member` on the primary
/// directory, through `both_rpc` (asserting it succeeded on both builds).
pub(crate) async fn issue_credential(h: &Harness, member: &str) -> (Value, Value) {
    let (w, n) = both_rpc(
        h,
        "credential.issue",
        json!({
            "member_did": member,
            "categories": FIXTURE_CATEGORIES,
            "expires_at_secs": fixture_credential_expires_at_secs(),
        }),
    )
    .await;
    assert!(w["result"]["record_id"].is_string(), "credential.issue wasm: {w}");
    assert!(n["result"]["record_id"].is_string(), "credential.issue native: {n}");
    (w, n)
}

/// The same, on the second directory, through `dir2_local`.
pub(crate) async fn issue_dir2_credential(h: &Harness, member: &str) -> (Value, Value) {
    let (w, n) = h
        .dir2_local(
            "credential.issue",
            json!({
                "member_did": member,
                "categories": FIXTURE_CATEGORIES,
                "expires_at_secs": fixture_credential_expires_at_secs(),
            }),
        )
        .await;
    assert!(w["result"]["record_id"].is_string(), "dir2 credential.issue wasm: {w}");
    assert!(n["result"]["record_id"].is_string(), "dir2 credential.issue native: {n}");
    (w, n)
}

/// `enrol_signing`'s own shape (mint against the signing key `<service>.
/// signing-status` reports, install through `both_rpc`), but through
/// `dir2_local` since the second directory is a separate instance, not
/// reachable through `both_rpc`'s single `/rpc` per stack, and minted by
/// `dir2_owner_identity()`: the signing host refuses a certificate whose
/// master is not the instance's own recorded owner.
pub(crate) async fn dir2_enrol_signing(h: &Harness) {
    let (w, _) = h.dir2_local("directory.signing-status", json!({})).await;
    let signing_did = w["result"]["signing_did"]
        .as_str()
        .unwrap_or_else(|| panic!("no signing_did from dir2 directory.signing-status: {w}"));
    let signing_pubkey = resolve_did_key(signing_did).unwrap();
    let cert = DelegationCertificate::issue(
        &dir2_owner_identity(),
        signing_pubkey,
        86_400 * 365 * 4,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();
    let (iw, inat) = h
        .dir2_local(
            "directory.install-signing-certificate",
            json!({ "certificate": cert.to_json().unwrap() }),
        )
        .await;
    assert!(iw["result"].is_object(), "dir2 install-signing-certificate wasm: {iw}");
    assert!(inat["result"].is_object(), "dir2 install-signing-certificate native: {inat}");
}

/// Every arm of `directory`'s `invoke` dispatch, maintained by hand:
/// nothing links this list to the `match` in `app.rs` at compile time.
/// Scenario 118 asserts each of these dispatches locally (a typo or a
/// removed verb fails there) and has exactly the wire posture below -- a
/// verb *added* to `app.rs` and not added here is simply untested, the
/// risk this shape accepts. A real guarantee would need `invoke` to
/// dispatch through a `const` table the test could import.
pub(crate) const ALL_DIRECTORY_VERBS: &[&str] = &[
    "directory.ping",
    "directory.settings",
    "directory.set-settings",
    "directory.info",
    "member.add",
    "member.remove",
    "member.list",
    "directory.publish",
    "directory.unpublish",
    "directory.publications",
    "directory.search",
    "directory.limits",
    "directory.set-limits",
    "directory.reindex",
    "directory.export",
    "directory.import",
    "directory.add-source",
    "directory.probe-info",
    "directory.remove-source",
    "directory.sources",
    "directory.start-run",
    "directory.query-source",
    "directory.merge",
    "directory.run-envelope",
    "directory.publish-to-source",
    "credential.issue",
    "credential.list",
    "revocation.issue",
    "revocation.list",
    "member.suspend",
    "member.lift",
    "member.decisions",
    "directory.standing",
    "directory.check-standing",
    "directory.memberships",
    "directory.signing-status",
    "directory.install-signing-certificate",
];

/// The whole security claim of this slice: exactly these four verbs
/// answer anything other than `-32013` over the wire.
pub(crate) const WIRE_REACHABLE_DIRECTORY_VERBS: &[&str] =
    &["directory.search", "directory.info", "directory.publish", "directory.standing"];

/// A directory's own search hit must carry no verification verdict, only
/// the issuer's evidence: no `verified`/`revocation_status`/`credential`
/// at the hit's own level, and `membership` itself is arrays of envelope
/// strings, never a computed `state`.
pub(crate) fn assert_hit_carries_no_verdict(hit: &Value, context: &Value) {
    for key in ["verified", "revocation_status", "credential"] {
        assert!(
            hit.get(key).is_none(),
            "a directory's own answer must carry no '{key}': {context}"
        );
    }
    let membership = &hit["membership"];
    assert!(membership.get("state").is_none(), "membership must not carry a verdict: {context}");
    for key in ["credentials", "revocations", "decisions"] {
        assert!(membership[key].is_array(), "{context}");
    }
}

/// Every section name `directory.export`'s manifest carries -- `standing`
/// is derived and not exported.
pub(crate) const DIRECTORY_BUNDLE_SECTIONS: &[&str] = &[
    "synorg",
    "publications",
    "members",
    "publication_log",
    "sources",
    "credentials",
    "revocations",
    "moderation_decisions",
    "held_memberships",
];

/// Every row of `collection` in `service`'s own store, on the chosen
/// stack. For collections no verb exposes (`search_index`). Same body as
/// `Harness::conv_rows`, with `did_for_service(service)` in place of
/// `did_for_service("conversation")`.
pub(crate) async fn service_rows(
    h: &Harness,
    wasm: bool,
    service: &str,
    collection: &str,
) -> Vec<Value> {
    let (storage, ks) =
        if wasm { (&h.wasm_storage, &h.wasm_ks) } else { (&h.native_storage, &h.native_ks) };
    let db = storage.open_service_db(&did_for_service(service), ks).await.expect("open service db");
    let opts = QueryOptions { filter: None, limit: Some(500), cursor: None };
    let page = match db.query(collection, &opts, None).await {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    page.value
        .records
        .into_iter()
        .filter_map(|r| serde_json::from_slice::<Value>(&r.payload).ok())
        .collect()
}
