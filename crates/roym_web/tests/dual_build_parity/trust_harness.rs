//! Canned fake directories the searcher's fan-out talks to instead of a real
//! second directory: hostile sources that serve forgeries, and a trust
//! source that serves a validly signed listing with membership evidence of
//! a chosen defect. A real directory cannot serve either -- its own
//! `directory.publish` verifies every envelope at the door.

use serde_json::{Value, json};
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_roym_core::{
    listing,
    membership::{
        self, MAX_REASON_LEN, MEMBERSHIP_CREDENTIAL_VERSION, MODERATION_DECISION_VERSION,
        MembershipCredentialPayload, MembershipScope, ModerationAction, ModerationDecisionPayload,
        ModerationScope, REVOCATION_VERSION, RevocationPayload,
    },
    record,
};
use syneroym_signed_record::{Envelope, RecordDraft};

use super::{fixtures::peer_identity, helpers::owner_identity, trust_fixtures::dir2_owner_did};

/// How many forged hits a hostile fake source returns per query.
pub(crate) const HOSTILE_SOURCE_FORGERIES: usize = 15;

/// A few shapes a forged envelope can take, cycled across a hostile
/// source's hits so the searcher's verification is exercised past its
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
/// The searcher's own verification in `directory.find`, not the source, is
/// what must reject a forgery. Returned as the `Value::String` shape both
/// real directory calls produce, so the two builds see byte-identical input.
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
/// one in that its listing is genuine: it verifies on the searcher's node,
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
pub(crate) fn listing_owner_identity() -> Identity {
    Identity::from_bytes(&[61; 32])
}

pub(crate) fn listing_owner_did() -> String {
    derive_did_key(&listing_owner_identity().public_key())
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

/// The one listing every trust source serves, signed by `listing_owner_did()`.
/// Category `gardening`: what `hTrustOutOfScope`'s credential leaves out.
fn canned_listing_envelope() -> String {
    let issuer = listing_owner_did();
    let slug = "trust-hedges";
    let payload = json!({
        "listing_id": listing::derive_listing_id(&issuer, slug).unwrap(),
        "slug": slug,
        "title": "Hedge trimming",
        "summary": "Neat hedges, fortnightly.",
        "categories": ["gardening"],
        "conversation_address": "did:key:zListingOwnerConv",
        "status": "active",
        "booking": {
            "mode": "enquiry",
            "lead_time_secs": 0,
            "cancellation_window_secs": 0,
            "max_per_booking": 1
        },
    });
    sign_directly(
        &listing_owner_identity(),
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

/// The membership credential a trust source serves for `listing_owner_did()`.
fn canned_credential_envelope(target: &str) -> String {
    let categories =
        if target == "did:key:hTrustOutOfScope" { vec!["plumbing"] } else { vec!["gardening"] };
    let expires_at_secs = if target == "did:key:hTrustExpired" {
        CANNED_EXPIRED_AT_SECS
    } else {
        CANNED_VALID_UNTIL_SECS
    };
    let member = listing_owner_did();
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
        "listing_id": listing::derive_listing_id(&listing_owner_did(), "trust-hedges").unwrap(),
        "record_id": parsed.record_id().unwrap(),
        "envelope": envelope,
        "issued_at_secs": CANNED_ISSUED_AT_SECS,
        "received_at_secs": CANNED_ISSUED_AT_SECS,
        "area_match": { "kind": "not-queried" },
        "membership": canned_evidence(target),
    })
}

/// The JSON-RPC frame a `directory.*` proxy call carries: `params` is
/// `[ "<json>" ]` (possibly still a JSON string of that array), and the
/// element inside is the JSON-RPC request frame.
fn inner_frame(params: &Value) -> Option<Value> {
    let parsed;
    let array = match params {
        Value::String(s) => {
            parsed = serde_json::from_str::<Value>(s).ok()?;
            &parsed
        }
        other => other,
    };
    serde_json::from_str(array.get(0)?.as_str()?).ok()
}

fn inner_method(params: &Value) -> Option<String> {
    let frame = inner_frame(params)?;
    frame.get("method")?.as_str().map(str::to_string)
}

fn published_ids(params: &Value) -> Option<(String, String)> {
    let frame = inner_frame(params)?;
    let env_str = frame.get("params")?.get("envelope")?.as_str()?;
    let env = Envelope::from_json(env_str).ok()?;
    let lid = env.payload.get("listing_id")?.as_str()?.to_string();
    let rid = env.record_id().ok()?;
    Some((lid, rid))
}

fn newer_source_response(params: &Value) -> Option<Value> {
    let method = inner_method(params)?;
    let result = match method.as_str() {
        "directory.info" => json!({
            "name": "Newer Guild",
            "retention_secs": 2_592_000u64,
            "member_count": 10u64,
            "issuer_did": claimed_issuer_did("did:key:hTrustValid"),
            "extra": 1,
        }),
        "directory.search" => {
            let mut hit = canned_hit("did:key:hTrustValid");
            hit["extra_hit_field"] = json!("newer-hit-v2");
            json!({ "hits": [hit], "ranking": "v2" })
        }
        "directory.standing" => json!({
            "issuer_did": claimed_issuer_did("did:key:hTrustValid"),
            "member_did": listing_owner_did(),
            "evidence": canned_evidence("did:key:hTrustValid"),
            "answered_at_secs": CANNED_ISSUED_AT_SECS,
            "extra": 1,
        }),
        "directory.publish" => match published_ids(params) {
            Some((listing_id, record_id)) => json!({
                "listing_id": listing_id,
                "record_id": record_id,
                "receipt": "x",
            }),
            None => {
                let error = json!({ "code": -32602, "message": "invalid publish envelope" });
                return Some(Value::String(json!({ "error": error }).to_string()));
            }
        },
        other => {
            let error = json!({ "code": -32601, "message": format!("no such method: {other}") });
            return Some(Value::String(json!({ "error": error }).to_string()));
        }
    };
    Some(Value::String(json!({ "result": result }).to_string()))
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
    let method = inner_method(params)?;
    let result = match method.as_str() {
        "directory.info" => {
            json!({ "name": "Canned Guild", "issuer_did": claimed_issuer_did(target) })
        }
        "directory.search" => json!({ "hits": [canned_hit(target)] }),
        "directory.standing" => json!({
            "issuer_did": claimed_issuer_did(target),
            "member_did": listing_owner_did(),
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

/// Returns the 0-based index for `did:key:hCeiling1` .. `did:key:hCeiling8`.
pub(crate) fn ceiling_source_index(target: &str) -> Option<usize> {
    let suffix = target.strip_prefix("did:key:hCeiling")?;
    let num: usize = suffix.parse().ok()?;
    if (1..=8).contains(&num) { Some(num - 1) } else { None }
}

pub(crate) fn ceiling_issuer_identity(idx: usize) -> Identity {
    Identity::from_bytes(&[81 + idx as u8; 32])
}

pub(crate) fn ceiling_issuer_did(idx: usize) -> String {
    derive_did_key(&ceiling_issuer_identity(idx).public_key())
}

static CEILING_RESPONSES: std::sync::OnceLock<[String; 8]> = std::sync::OnceLock::new();

fn get_ceiling_response(idx: usize) -> String {
    let responses = CEILING_RESPONSES.get_or_init(build_all_ceiling_responses);
    responses[idx].clone()
}

fn build_ceiling_evidence(
    issuer_key: &Identity,
    issuer_did: &str,
    member_did: &str,
    listing_id: &str,
    src_idx: usize,
    hit_idx: usize,
) -> Value {
    let mut credentials = Vec::with_capacity(4);
    for c in 0..4 {
        let payload = MembershipCredentialPayload {
            synorg_name: "Ceiling Guild".to_string(),
            member_did: member_did.to_string(),
            scope: MembershipScope {
                categories: vec!["gardening".to_string(), format!("extra-{c}")],
                area: vec![],
            },
        };
        credentials.push(sign_claiming(
            issuer_key,
            issuer_did,
            RecordDraft {
                version: MEMBERSHIP_CREDENTIAL_VERSION,
                record_type: record::RECORD_MEMBERSHIP_CREDENTIAL.to_string(),
                subject: member_did.to_string(),
                payload: serde_json::to_value(&payload).unwrap(),
                expires_at_secs: Some(CANNED_VALID_UNTIL_SECS),
                supersedes: None,
            },
        ));
    }

    let mut revocations = Vec::with_capacity(4);
    for r in 0..4 {
        let mut fake_cred_bytes = [0u8; 32];
        fake_cred_bytes[0] = 0xfd;
        fake_cred_bytes[1] = src_idx as u8;
        fake_cred_bytes[2] = hit_idx as u8;
        fake_cred_bytes[3] = r as u8;
        let other_id = format!("rec_{}", z32::encode(&fake_cred_bytes));
        let payload = RevocationPayload {
            credential_record_id: other_id.clone(),
            member_did: member_did.to_string(),
            reason: "x".repeat(MAX_REASON_LEN),
        };
        revocations.push(sign_claiming(
            issuer_key,
            issuer_did,
            RecordDraft {
                version: REVOCATION_VERSION,
                record_type: record::RECORD_REVOCATION.to_string(),
                subject: other_id,
                payload: serde_json::to_value(&payload).unwrap(),
                expires_at_secs: None,
                supersedes: None,
            },
        ));
    }

    let mut decisions = Vec::with_capacity(8);
    for d in 0..8 {
        let mut fake_bytes = [0u8; 32];
        fake_bytes[0] = 0xfe;
        fake_bytes[1] = src_idx as u8;
        fake_bytes[2] = hit_idx as u8;
        fake_bytes[3] = d as u8;
        let fake_rec_id = format!("rec_{}", z32::encode(&fake_bytes));
        let payload = ModerationDecisionPayload {
            action: ModerationAction::Lift,
            member_did: member_did.to_string(),
            scope: ModerationScope::Listing { listing_id: listing_id.to_string() },
            rule: String::new(),
            reason: "m".repeat(MAX_REASON_LEN),
            until_secs: None,
        };
        decisions.push(sign_claiming(
            issuer_key,
            issuer_did,
            RecordDraft {
                version: MODERATION_DECISION_VERSION,
                record_type: record::RECORD_MODERATION_DECISION.to_string(),
                subject: member_did.to_string(),
                payload: serde_json::to_value(&payload).unwrap(),
                expires_at_secs: None,
                supersedes: Some(fake_rec_id),
            },
        ));
    }

    json!({
        "credentials": credentials,
        "revocations": revocations,
        "decisions": decisions,
    })
}

fn signed_canned_listing(
    slug: &str,
    title: &str,
    summary: &str,
    issued_at: u64,
    supersedes: Option<String>,
) -> (String, String) {
    let member_key = listing_owner_identity();
    let member_did = listing_owner_did();
    let lid = listing::derive_listing_id(&member_did, slug).unwrap();
    let payload = json!({
        "listing_id": lid,
        "slug": slug,
        "title": title,
        "summary": summary,
        "categories": ["gardening"],
        "conversation_address": "did:key:zConv",
        "status": "active",
        "booking": {
            "mode": "enquiry",
            "lead_time_secs": 0,
            "cancellation_window_secs": 0,
            "max_per_booking": 1,
        },
    });
    let (mut env, bytes) = Envelope::unsigned(
        RecordDraft {
            version: listing::LISTING_VERSION,
            record_type: record::RECORD_LISTING.to_string(),
            subject: member_did.clone(),
            payload,
            expires_at_secs: None,
            supersedes,
        },
        member_did,
        None,
        issued_at,
    )
    .unwrap();
    env.attach_signature(z32::encode(&member_key.sign(&bytes).to_bytes())).unwrap();
    let rec_id = env.record_id().unwrap();
    let env_json = env.to_json().unwrap();
    (rec_id, env_json)
}

fn build_all_ceiling_responses() -> [String; 8] {
    let member_did = listing_owner_did();
    let ceiling_summary = "text \"quoted\" \\\n ".repeat(2000 / 18);

    core::array::from_fn(|idx| {
        let issuer_key = ceiling_issuer_identity(idx);
        let issuer_did = ceiling_issuer_did(idx);

        let hits: Vec<Value> = (0..40)
            .map(|h| {
                let slug = format!("ceiling-listing-{idx}-{h}");
                let lid = listing::derive_listing_id(&member_did, &slug).unwrap();
                let (rec_id, env_json) = signed_canned_listing(
                    &slug,
                    &format!("Ceiling Title {idx}-{h}"),
                    &ceiling_summary,
                    CANNED_ISSUED_AT_SECS,
                    None,
                );

                let evidence =
                    build_ceiling_evidence(&issuer_key, &issuer_did, &member_did, &lid, idx, h);

                json!({
                    "listing_id": lid,
                    "record_id": rec_id,
                    "envelope": env_json,
                    "issued_at_secs": CANNED_ISSUED_AT_SECS,
                    "received_at_secs": CANNED_ISSUED_AT_SECS,
                    "area_match": { "kind": "not-queried" },
                    "membership": evidence,
                })
            })
            .collect();

        json!({ "result": { "hits": hits } }).to_string()
    })
}

fn two_versions_search_response() -> String {
    let slug = "two-versions-hedge";
    let lid = listing::derive_listing_id(&listing_owner_did(), slug).unwrap();

    let (rec1, env1_json) = signed_canned_listing(
        slug,
        "Hedge trimming v1",
        "Neat hedges, fortnightly.",
        CANNED_ISSUED_AT_SECS,
        None,
    );

    let (rec2, env2_json) = signed_canned_listing(
        slug,
        "Hedge trimming v2",
        "Neat hedges, fortnightly.",
        CANNED_ISSUED_AT_SECS + 10,
        Some(rec1.clone()),
    );

    let hit1 = json!({
        "listing_id": lid,
        "record_id": rec1,
        "envelope": env1_json,
        "issued_at_secs": CANNED_ISSUED_AT_SECS,
        "received_at_secs": CANNED_ISSUED_AT_SECS,
        "area_match": { "kind": "not-queried" },
        "membership": { "credentials": [], "revocations": [], "decisions": [] },
    });
    let hit2 = json!({
        "listing_id": lid,
        "record_id": rec2,
        "envelope": env2_json,
        "issued_at_secs": CANNED_ISSUED_AT_SECS + 10,
        "received_at_secs": CANNED_ISSUED_AT_SECS + 10,
        "area_match": { "kind": "not-queried" },
        "membership": { "credentials": [], "revocations": [], "decisions": [] },
    });

    json!({ "result": { "hits": [hit1, hit2] } }).to_string()
}

/// What both stacks' service proxies try before any real routing: the
/// hostile forgers first, the `hNewer` source, then the trust sources.
pub(crate) async fn canned_source_response(target: &str, params: &Value) -> Option<Value> {
    if target == "did:key:hSlow" {
        if inner_method(params).as_deref() == Some("directory.search") {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            return Some(Value::String(json!({ "result": { "hits": [] } }).to_string()));
        }
        return Some(Value::String(json!({ "result": { "name": "Slow Guild" } }).to_string()));
    }
    if target == "did:key:hOversized" {
        if inner_method(params).as_deref() == Some("directory.search") {
            let padding = "x".repeat(8 * 1024 * 1024 + 1024);
            return Some(Value::String(
                json!({ "result": { "hits": [], "padding": padding } }).to_string(),
            ));
        }
        return Some(Value::String(json!({ "result": { "name": "Oversized Guild" } }).to_string()));
    }
    if target == "did:key:hTwoVersions" {
        if inner_method(params).as_deref() == Some("directory.search") {
            return Some(Value::String(two_versions_search_response()));
        }
        return Some(Value::String(
            json!({ "result": { "name": "Two Versions Guild" } }).to_string(),
        ));
    }
    if let Some(idx) = ceiling_source_index(target) {
        let method = inner_method(params)?;
        let result = match method.as_str() {
            "directory.info" => json!({
                "name": format!("Ceiling Guild {}", idx + 1),
                "issuer_did": ceiling_issuer_did(idx),
            }),
            "directory.search" => return Some(Value::String(get_ceiling_response(idx))),
            "directory.standing" => json!({
                "issuer_did": ceiling_issuer_did(idx),
                "member_did": listing_owner_did(),
                "evidence": json!({ "credentials": [], "revocations": [], "decisions": [] }),
                "answered_at_secs": CANNED_ISSUED_AT_SECS,
            }),
            other => {
                let error =
                    json!({ "code": -32601, "message": format!("no such method: {other}") });
                return Some(Value::String(json!({ "error": error }).to_string()));
            }
        };
        return Some(Value::String(json!({ "result": result }).to_string()));
    }

    if target == "did:key:hNewer" {
        return newer_source_response(params);
    }
    hostile_source_response(target).or_else(|| trust_source_response(target, params))
}
