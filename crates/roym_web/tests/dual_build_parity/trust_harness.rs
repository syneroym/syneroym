//! Canned fake directories the consumer's fan-out talks to instead of a real
//! second directory: hostile sources that serve forgeries, and a trust
//! source that serves a validly signed listing with membership evidence of
//! a chosen defect. A real directory cannot serve either -- its own
//! `directory.publish` verifies every envelope at the door.

use serde_json::{Value, json};
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_roym_core::{
    listing,
    membership::{self, MembershipCredentialPayload, MembershipScope},
    record,
};
use syneroym_signed_record::{Envelope, RecordDraft};

use super::{fixtures::peer_identity, helpers::owner_identity, trust_fixtures::dir2_owner_did};

/// How many forged hits a hostile fake source returns per `query-source`.
pub(crate) const HOSTILE_SOURCE_FORGERIES: usize = 15;

/// A few shapes a forged envelope can take, cycled across a hostile
/// source's hits so the consumer's verification is exercised past its
/// outermost JSON parse: not-an-object, an object with no signature, an
/// object with an unusable signature, a wrong record type, and an
/// issued-at far in the future.
pub(crate) const FORGED_ENVELOPE_SHAPES: &[&str] = &[
    "{\"not\":\"a signed listing envelope\"}",
    "\"just a string\"",
    "{\"payload\":{\"record_type\":\"listing\"},\"delegation\":null}",
    "{\"payload\":{\"record_type\":\"listing\"},\"signature\":\"!!not-base64!!\"}",
    "{\"payload\":{\"record_type\":\"profile\"},\"signature\":\"AAAA\"}",
    "{\"payload\":{\"record_type\":\"listing\",\"issued_at_secs\":9999999999},\"signature\":\"\
     AAAA\"}",
];

/// Canned response for the hostile fake sources `did:key:hForge1` /
/// `did:key:hForge2`, and the `did:key:hTrunc` source that answers with
/// no hits but `truncated: true`. A real second directory cannot serve
/// forgeries -- its own `directory.publish` verifies every envelope at
/// the door -- so a canned page is the only way to drive "a source that
/// returns nothing but forgeries" or "a source that had more matches
/// than it would return". The consumer's own verification in
/// `query-source`, not the source, is what must reject a forgery.
/// Returned as the `Value::String` shape both real directory calls
/// produce, so the two builds see byte-identical input.
pub(crate) fn hostile_source_response(target: &str) -> Option<Value> {
    if target == "did:key:hTrunc" {
        return Some(Value::String(
            json!({ "result": { "hits": [], "truncated": true } }).to_string(),
        ));
    }
    let tag = match target {
        "did:key:hForge1" => "a",
        "did:key:hForge2" => "b",
        _ => return None,
    };
    let hits: Vec<Value> = (0..HOSTILE_SOURCE_FORGERIES)
        .map(|i| {
            json!({
                "listing_id": format!("forged-{tag}-{i}"),
                "record_id": format!("forged-rec-{tag}-{i}"),
                "envelope": FORGED_ENVELOPE_SHAPES[i % FORGED_ENVELOPE_SHAPES.len()],
                "issued_at_secs": 4_000_000_000u64,
                "received_at_secs": 4_000_000_000u64,
                "area_match": { "kind": "not-queried" }
            })
        })
        .collect();
    Some(Value::String(json!({ "result": { "hits": hits } }).to_string()))
}

/// The five canned trust sources. A trust source differs from a hostile
/// one in that its listing is genuine: it verifies on the consumer's node,
/// so the hit reaches the membership check, which is where these sources
/// differ. `hTrustValid` is the control: without it, every other target
/// could be "refused" for a reason in the fixture instead of the evidence.
pub(crate) const TRUST_SOURCES: &[&str] = &[
    "did:key:hTrustValid",
    "did:key:hTrustForged",
    "did:key:hTrustExpired",
    "did:key:hTrustOutOfScope",
    "did:key:hTrustWrongSynOrg",
];

/// A time that is already past when the tests run, so a listing or a
/// credential issued "then" is never from the future.
const CANNED_ISSUED_AT_SECS: u64 = 1_700_000_000;
/// A credential that outlives any test run and is not near the
/// two-year lifetime cap a real `credential.issue` enforces.
const CANNED_VALID_UNTIL_SECS: u64 = 4_000_000_000;
const CANNED_EXPIRED_AT_SECS: u64 = CANNED_ISSUED_AT_SECS + 100;

/// The person a trust source lists. Not `owner_did()` or `peer_did()`:
/// nothing about this member is known to any real directory in the run.
pub(crate) fn provider_identity() -> Identity {
    Identity::from_bytes(&[61; 32])
}

pub(crate) fn provider_did() -> String {
    derive_did_key(&provider_identity().public_key())
}

/// The key each trust source's own `info` names as its issuer. One
/// distinct fixed key per source, so no source's evidence is valid for
/// another's pin.
pub(crate) fn trust_issuer_identity(target: &str) -> Identity {
    let seed = match target {
        "did:key:hTrustValid" => 71,
        "did:key:hTrustForged" => 72,
        "did:key:hTrustExpired" => 73,
        "did:key:hTrustOutOfScope" => 74,
        _ => 75,
    };
    Identity::from_bytes(&[seed; 32])
}

pub(crate) fn trust_issuer_did(target: &str) -> String {
    derive_did_key(&trust_issuer_identity(target).public_key())
}

/// The issuer a trust source's `directory.info` and `directory.standing`
/// claim: its own key, except the wrong-SynOrg source, which claims the
/// second directory's owner.
pub(crate) fn claimed_issuer_did(target: &str) -> String {
    if target == "did:key:hTrustWrongSynOrg" { dir2_owner_did() } else { trust_issuer_did(target) }
}

/// Who actually signs the credential a trust source serves. Equal to the
/// claimed issuer for the honest defects (expired, out of scope), and a
/// different key for the two where the source lies about whose credential
/// this is.
fn credential_signer(target: &str) -> Identity {
    match target {
        "did:key:hTrustForged" => peer_identity(),
        "did:key:hTrustWrongSynOrg" => owner_identity(),
        _ => trust_issuer_identity(target),
    }
}

/// The issuer named inside the credential. The forged source names the
/// pinned issuer while signing with another key, so only the signature check
/// can refuse it; every other source names its own signer.
fn credential_claimed_issuer(target: &str) -> String {
    if target == "did:key:hTrustForged" {
        claimed_issuer_did(target)
    } else {
        derive_did_key(&credential_signer(target).public_key())
    }
}

fn sign_directly(identity: &Identity, draft: RecordDraft) -> String {
    sign_claiming(identity, &derive_did_key(&identity.public_key()), draft)
}

/// Signs with `identity` but names `issuer` in the envelope: honest only when
/// they are the same key.
fn sign_claiming(identity: &Identity, issuer: &str, draft: RecordDraft) -> String {
    let (mut env, bytes) =
        Envelope::unsigned(draft, issuer.to_string(), None, CANNED_ISSUED_AT_SECS).unwrap();
    env.attach_signature(z32::encode(&identity.sign(&bytes).to_bytes())).unwrap();
    env.to_json().unwrap()
}

/// The one listing every trust source serves, signed by `provider_did()`.
/// Category `gardening`: what `hTrustOutOfScope`'s credential leaves out.
fn canned_listing_envelope() -> String {
    let issuer = provider_did();
    let slug = "trust-hedges";
    let payload = json!({
        "listing_id": listing::derive_listing_id(&issuer, slug).unwrap(),
        "slug": slug,
        "title": "Hedge trimming",
        "summary": "Neat hedges, fortnightly.",
        "categories": ["gardening"],
        "conversation_address": "did:key:zProviderConv",
        "status": "active",
        "booking": {
            "mode": "enquiry",
            "lead_time_secs": 0,
            "cancellation_window_secs": 0,
            "max_per_booking": 1
        },
    });
    sign_directly(
        &provider_identity(),
        RecordDraft {
            version: listing::LISTING_VERSION,
            record_type: record::RECORD_LISTING.to_string(),
            subject: issuer,
            payload,
            expires_at_secs: None,
            supersedes: None,
        },
    )
}

/// The membership credential a trust source serves for `provider_did()`.
fn canned_credential_envelope(target: &str) -> String {
    let categories =
        if target == "did:key:hTrustOutOfScope" { vec!["plumbing"] } else { vec!["gardening"] };
    let expires_at_secs = if target == "did:key:hTrustExpired" {
        CANNED_EXPIRED_AT_SECS
    } else {
        CANNED_VALID_UNTIL_SECS
    };
    let member = provider_did();
    let payload = MembershipCredentialPayload {
        synorg_name: "Canned Guild".to_string(),
        member_did: member.clone(),
        scope: MembershipScope {
            categories: categories.into_iter().map(String::from).collect(),
            area: vec![],
        },
    };
    sign_claiming(
        &credential_signer(target),
        &credential_claimed_issuer(target),
        RecordDraft {
            version: membership::MEMBERSHIP_CREDENTIAL_VERSION,
            record_type: record::RECORD_MEMBERSHIP_CREDENTIAL.to_string(),
            subject: member,
            payload: serde_json::to_value(&payload).unwrap(),
            expires_at_secs: Some(expires_at_secs),
            supersedes: None,
        },
    )
}

fn canned_evidence(target: &str) -> Value {
    json!({
        "credentials": [canned_credential_envelope(target)],
        "revocations": [],
        "decisions": [],
    })
}

fn canned_hit(target: &str) -> Value {
    let envelope = canned_listing_envelope();
    let parsed = Envelope::from_json(&envelope).unwrap();
    json!({
        "listing_id": listing::derive_listing_id(&provider_did(), "trust-hedges").unwrap(),
        "record_id": parsed.record_id().unwrap(),
        "envelope": envelope,
        "issued_at_secs": CANNED_ISSUED_AT_SECS,
        "received_at_secs": CANNED_ISSUED_AT_SECS,
        "area_match": { "kind": "not-queried" },
        "membership": canned_evidence(target),
    })
}

/// The `method` of the JSON-RPC frame a `directory.*` proxy call carries:
/// `params` is `[ "<json>" ]` (possibly still a JSON string of that array),
/// and the frame inside names the verb.
fn inner_method(params: &Value) -> Option<String> {
    let parsed;
    let array = match params {
        Value::String(s) => {
            parsed = serde_json::from_str::<Value>(s).ok()?;
            &parsed
        }
        other => other,
    };
    let frame: Value = serde_json::from_str(array.get(0)?.as_str()?).ok()?;
    frame.get("method")?.as_str().map(str::to_string)
}

/// Canned replies for `TRUST_SOURCES`, as the `Value::String` shape a real
/// directory call produces (so both builds see identical bytes):
/// `directory.info` names the claimed issuer, `directory.search` serves one
/// genuine listing with this source's evidence, `directory.standing`
/// serves the same evidence. Anything else answers method-not-found.
pub(crate) fn trust_source_response(target: &str, params: &Value) -> Option<Value> {
    if !TRUST_SOURCES.contains(&target) {
        return None;
    }
    let result = match inner_method(params)?.as_str() {
        "directory.info" => {
            json!({ "name": "Canned Guild", "issuer_did": claimed_issuer_did(target) })
        }
        "directory.search" => json!({ "hits": [canned_hit(target)] }),
        "directory.standing" => json!({
            "issuer_did": claimed_issuer_did(target),
            "member_did": provider_did(),
            "evidence": canned_evidence(target),
            "answered_at_secs": CANNED_ISSUED_AT_SECS,
        }),
        other => {
            let error = json!({ "code": -32601, "message": format!("no such method: {other}") });
            return Some(Value::String(json!({ "error": error }).to_string()));
        }
    };
    Some(Value::String(json!({ "result": result }).to_string()))
}

/// What both stacks' service proxies try before any real routing: the
/// hostile forgers first, then the trust sources.
pub(crate) fn canned_source_response(target: &str, params: &Value) -> Option<Value> {
    hostile_source_response(target).or_else(|| trust_source_response(target, params))
}
