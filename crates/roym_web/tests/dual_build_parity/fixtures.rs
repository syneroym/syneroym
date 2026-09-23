use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use syneroym_identity::{
    Identity,
    delegation::{DelegationCertificate, SCOPE_RECORD_SIGNING},
    substrate::{derive_did_key, resolve_did_key},
};
use syneroym_roym_core::{card, services, transaction};
use syneroym_rpc::{
    AuthLevel, CallerContext, ConversationDeliveryState, ConversationMessage, JsonRpcRequest,
    NativeInvocation, SessionContext,
};
use syneroym_signed_record::{Envelope, RecordDraft};

use super::helpers::*;

pub(crate) struct Mutant<'a, D>(pub(crate) &'a D);

impl<D: Driver> Driver for Mutant<'_, D> {
    async fn invoke_web(&self, request: &str) -> Result<String, String> {
        self.0.invoke_web(request).await.map(|s| s.replace("\"result\"", "\"mutated_result\""))
    }
    async fn status(&self, service_name: &str) -> Result<String, String> {
        self.0.status(service_name).await.map(|s| s.replace("\"service\"", "\"mutated_service\""))
    }
}

/// POSTs one JSON-RPC method to both stacks' `/rpc` with the owner session
/// and returns each build's parsed response.
pub(crate) async fn both_rpc(h: &Harness, method: &str, params: Value) -> (Value, Value) {
    let req = json!({ "method": method, "params": params }).to_string().into_bytes();
    let w = h.wasm_http.post("/rpc", req.clone(), Some(h.caller())).await;
    let n = h.native_http.post("/rpc", req, Some(h.caller())).await;
    (serde_json::from_slice(&w.body).unwrap(), serde_json::from_slice(&n.body).unwrap())
}

/// One JSON-RPC method to a single stack -- for the cases where the two
/// builds are driven with different arguments (a per-stack message id).
pub(crate) async fn one_rpc(h: &Harness, wasm: bool, method: &str, params: Value) -> Value {
    let req = json!({ "method": method, "params": params }).to_string().into_bytes();
    let r = if wasm {
        h.wasm_http.post("/rpc", req, Some(h.caller())).await
    } else {
        h.native_http.post("/rpc", req, Some(h.caller())).await
    };
    serde_json::from_slice(&r.body).unwrap()
}

/// Mints one delegation certificate against `service`'s own signing key and
/// installs it on both stacks, so `<service>` can sign records.
pub(crate) async fn enrol_signing(h: &Harness, service: &str) {
    let status_m = format!("{service}.signing-status");
    let install_m = format!("{service}.install-signing-certificate");
    let (w, _) = both_rpc(h, &status_m, json!({})).await;
    let signing_did = w["result"]["signing_did"]
        .as_str()
        .unwrap_or_else(|| panic!("no signing_did from {status_m}: {w}"));
    let signing_pubkey = resolve_did_key(signing_did).unwrap();
    // The signing host verifies the certificate against the pinned
    // `RecordClock` (ahead of wall-clock time), so the certificate must
    // still be valid then -- yet under the 5-year lifetime ceiling the
    // install path enforces. A default short-lived one signs nothing here.
    let cert = DelegationCertificate::issue(
        &h.owner,
        signing_pubkey,
        86_400 * 365 * 4,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();
    let (iw, inat) =
        both_rpc(h, &install_m, json!({ "certificate": cert.to_json().unwrap() })).await;
    // Asserted, not ignored: a refused install leaves the service signing
    // nothing, and every later assertion in the scenario then fails on a
    // missing envelope instead of naming the real reason.
    assert!(iw["result"].is_object(), "{install_m} wasm: {iw}");
    assert!(inat["result"].is_object(), "{install_m} native: {inat}");
}

/// A `listing.set` params object with every one of the seven optional
/// blocks filled and no non-integer number anywhere.
pub(crate) fn full_listing_params(slug: &str, title: &str) -> Value {
    json!({
        "slug": slug,
        "title": title,
        "summary": "Neat hedges, fortnightly.",
        "categories": ["gardening", "outdoor"],
        "conversation_address": "did:key:zProviderConv",
        "booking": {
            "mode": "slots",
            "lead_time_secs": 3600,
            "cancellation_window_secs": 86400,
            "max_per_booking": 2
        },
        "payment": {
            "currency": "EUR",
            "model": "per-hour",
            "amount_minor": 3500,
            "tax_included": true,
            "methods": ["cash"],
            "payee": "A. Gardener"
        },
        "product": { "unit": "hour", "pack_size": 1, "condition": "new", "sku": "HT-1" },
        "service": { "duration_secs": 3600, "includes": ["clippings removed"] },
        "location": {
            "where": "at-customer",
            "service_area": [
                { "kind": "circle", "lat_e6": 52000000, "lon_e6": 13000000, "radius_m": 5000 }
            ],
            "address_disclosure": "on-agreement"
        },
        "relationship": { "open_to": "anyone" },
        "service_record": {
            "issues_fulfilment_receipt": true,
            "warranty_secs": 0,
            "retention_secs": 31536000
        }
    })
}

pub(crate) fn is_err(v: &Value, code: i64) -> bool {
    v["error"]["code"].as_i64() == Some(code)
}

/// `listing.set` on both stacks (asserting it succeeded), then `listing.get`
/// -- returns the listing id and each build's get response, which is where
/// the signed envelope lives (`listing.set` returns only ids and a count).
pub(crate) async fn set_and_get(h: &Harness, params: Value) -> (String, Value, Value) {
    let (sw, sn) = both_rpc(h, "listing.set", params).await;
    assert!(sw["result"]["listing_id"].is_string(), "listing.set wasm: {sw}");
    assert!(sn["result"]["listing_id"].is_string(), "listing.set native: {sn}");
    let id = sw["result"]["listing_id"].as_str().unwrap().to_string();
    let (gw, gn) = both_rpc(h, "listing.get", json!({ "listing_id": id })).await;
    (id, gw, gn)
}

/// Opens a direct conversation to `peer` on both stacks and returns the
/// (identical) host conversation id.
pub(crate) async fn open_conv(h: &Harness, peer: &str) -> String {
    let (w, n) = both_rpc(h, "conversation.open", json!({ "address": peer })).await;
    let cw = w["result"]["conversation_id"].as_str().unwrap().to_string();
    assert_eq!(cw, n["result"]["conversation_id"].as_str().unwrap());
    cw
}

pub(crate) fn inbound(
    id: &str,
    conversation: &str,
    author: &str,
    ts: i64,
    body: &str,
) -> ConversationMessage {
    ConversationMessage {
        id: id.to_string(),
        conversation: conversation.to_string(),
        author: author.to_string(),
        sender_timestamp: ts,
        received_at: ts,
        content_type: "text/plain".to_string(),
        body: body.as_bytes().to_vec(),
        state: ConversationDeliveryState::Delivered,
        verified: true,
        last_error: None,
    }
}

/// A second person, whose records this installation receives but never
/// signs. Fixed bytes so both builds mint the same DID.
pub(crate) fn peer_identity() -> Identity {
    Identity::from_bytes(&[7; 32])
}

pub(crate) fn peer_did() -> String {
    derive_did_key(&peer_identity().public_key())
}

/// Signs one envelope directly with `peer_identity`, at the same pinned
/// clock the two stacks use, so a scenario can deliver a card the local
/// node did not produce.
pub(crate) fn sign_as_peer(
    record_type: &str,
    version: u32,
    subject: &str,
    payload: Value,
    expires_at_secs: Option<u64>,
    issued_at_secs: u64,
) -> String {
    let peer = peer_identity();
    let issuer = peer_did();
    let draft = RecordDraft {
        version,
        record_type: record_type.to_string(),
        subject: subject.to_string(),
        payload,
        expires_at_secs,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer, None, issued_at_secs).unwrap();
    let sig = z32::encode(&peer.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    env.to_json().unwrap()
}

pub(crate) fn inbound_card(
    id: &str,
    conversation: &str,
    author: &str,
    ts: i64,
    card_type: &str,
    version: u32,
    envelope: &str,
) -> ConversationMessage {
    let body = card::card_body(card_type, version, envelope).unwrap();
    ConversationMessage {
        id: id.to_string(),
        conversation: conversation.to_string(),
        author: author.to_string(),
        sender_timestamp: ts,
        received_at: ts,
        content_type: card::CARD_CONTENT_TYPE.to_string(),
        body: body.into_bytes(),
        state: ConversationDeliveryState::Delivered,
        verified: true,
        last_error: None,
    }
}

pub(crate) fn sample_quote_terms() -> Value {
    json!({
        "scope": "Garden repair",
        "currency": "EUR",
        "amount_minor": 50000,
        "tax_minor": 5000,
        "fees_minor": 2000,
        "payment_methods": ["card", "cash"],
        "payee": "Garden Services Ltd",
        "payment_timing": "after-work",
        "location": {
            "where": "at-customer",
            "address": "123 Flower St",
        },
        "cancellation_terms": "Cancel 24h prior for full refund",
        "refund_terms": "Full refund if unsatisfactory",
        "dispute_path": "Contact disputes@example.com",
    })
}

pub(crate) fn peer_signed_request(conv: &str, seq: u32, issued_at: u64) -> (String, String) {
    let req_id = transaction::derive_request_id(conv, &peer_did(), seq).unwrap();
    let payload = json!({
        "request_id": req_id,
        "conversation": conv,
        "sequence": seq,
        "categories": ["gardening"],
        "description": "Prune apple tree",
        "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
    });
    let env_json = sign_as_peer(
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &req_id,
        payload,
        None,
        issued_at,
    );
    let env = Envelope::from_json(&env_json).unwrap();
    let record_id = env.record_id().unwrap();
    (record_id, env_json)
}

pub(crate) fn wall_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

pub(crate) fn peer_signed_quote(
    conv: &str,
    seq: u32,
    req_record_id: &str,
    consumer_did: &str,
    expires_at: Option<u64>,
    issued_at: u64,
) -> (String, String) {
    let now = wall_now();
    let (issued_at, exp_secs) = match expires_at {
        Some(exp) => (issued_at, exp),
        None => (now, now + 3600),
    };
    let quote_id = transaction::derive_quote_id(conv, &peer_did(), seq).unwrap();
    let mut terms = sample_quote_terms();
    terms["quote_expires_at_secs"] = json!(exp_secs);
    let payload = json!({
        "quote_id": quote_id,
        "conversation": conv,
        "sequence": seq,
        "request_record_id": req_record_id,
        "consumer_did": consumer_did,
        "terms": terms,
    });
    let env_json = sign_as_peer(
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &quote_id,
        payload,
        Some(exp_secs),
        issued_at,
    );
    let env = Envelope::from_json(&env_json).unwrap();
    let record_id = env.record_id().unwrap();
    (record_id, env_json)
}

pub(crate) fn peer_signed_consumer_receipt(
    conv: &str,
    quote_record_id: &str,
    consumer_did: &str,
    provider_did: &str,
    terms: Value,
    issued_at: u64,
) -> (String, String) {
    let _ = conv;
    let payload = json!({
        "quote_record_id": quote_record_id,
        "consumer_did": consumer_did,
        "provider_did": provider_did,
        "role": "consumer",
        "terms": terms,
    });
    let env_json = sign_as_peer(
        transaction::RECORD_AGREEMENT_RECEIPT,
        transaction::AGREEMENT_RECEIPT_VERSION,
        quote_record_id,
        payload,
        None,
        issued_at,
    );
    let env = Envelope::from_json(&env_json).unwrap();
    let record_id = env.record_id().unwrap();
    (record_id, env_json)
}

pub(crate) fn peer_signed_request_with(
    conv: &str,
    seq: u32,
    issued_at: u64,
    expires_at_secs: Option<u64>,
    supersedes: Option<String>,
) -> (String, String) {
    let req_id = transaction::derive_request_id(conv, &peer_did(), seq).unwrap();
    let payload = json!({
        "request_id": req_id,
        "conversation": conv,
        "sequence": seq,
        "categories": ["gardening"],
        "description": "Prune apple tree",
        "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
    });
    let peer = peer_identity();
    let issuer = peer_did();
    let draft = RecordDraft {
        version: transaction::REQUEST_VERSION,
        record_type: transaction::RECORD_REQUEST.to_string(),
        subject: req_id,
        payload,
        expires_at_secs,
        supersedes,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer, None, issued_at).unwrap();
    let sig = z32::encode(&peer.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    let env_json = env.to_json().unwrap();
    let record_id = env.record_id().unwrap();
    (record_id, env_json)
}

pub(crate) fn peer_signed_quote_with(
    conv: &str,
    seq: u32,
    req_record_id: &str,
    consumer_did: &str,
    expires_at: Option<u64>,
    issued_at: u64,
    supersedes: Option<String>,
) -> (String, String) {
    let quote_id = transaction::derive_quote_id(conv, &peer_did(), seq).unwrap();
    let exp_secs = expires_at.unwrap_or(issued_at + 3600);
    let mut terms = sample_quote_terms();
    terms["quote_expires_at_secs"] = json!(exp_secs);
    let payload = json!({
        "quote_id": quote_id,
        "conversation": conv,
        "sequence": seq,
        "request_record_id": req_record_id,
        "consumer_did": consumer_did,
        "terms": terms,
    });
    let peer = peer_identity();
    let issuer = peer_did();
    let draft = RecordDraft {
        version: transaction::QUOTE_VERSION,
        record_type: transaction::RECORD_QUOTE.to_string(),
        subject: quote_id,
        payload,
        expires_at_secs: expires_at,
        supersedes,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer, None, issued_at).unwrap();
    let sig = z32::encode(&peer.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    let env_json = env.to_json().unwrap();
    let record_id = env.record_id().unwrap();
    (record_id, env_json)
}

pub(crate) fn peer_signed_consumer_receipt_with_expiry(
    conv: &str,
    quote_record_id: &str,
    consumer_did: &str,
    provider_did: &str,
    terms: Value,
    issued_at: u64,
    expires_at_secs: Option<u64>,
) -> (String, String) {
    let _ = conv;
    let payload = json!({
        "quote_record_id": quote_record_id,
        "consumer_did": consumer_did,
        "provider_did": provider_did,
        "role": "consumer",
        "terms": terms,
    });
    let peer = peer_identity();
    let issuer = peer_did();
    let draft = RecordDraft {
        version: transaction::AGREEMENT_RECEIPT_VERSION,
        record_type: transaction::RECORD_AGREEMENT_RECEIPT.to_string(),
        subject: quote_record_id.to_string(),
        payload,
        expires_at_secs,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer, None, issued_at).unwrap();
    let sig = z32::encode(&peer.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    let env_json = env.to_json().unwrap();
    let record_id = env.record_id().unwrap();
    (record_id, env_json)
}

pub(crate) fn env(method: &str, params: Value) -> Value {
    json!({ "method": method, "params": params })
}

/// One representative verb each of the six services owns. `require_internal`
/// is the first statement of every `invoke`, so the verb only has to be a
/// string the service would otherwise route -- the refusal happens before
/// dispatch. Deleting `require_internal` from any one service's `invoke`
/// fails here rather than staying green (scenario 71 only proves
/// `api.status` is *not* refused).
pub(crate) const WIRE_REFUSED_VERBS: [(services::Service, &str); 6] = [
    (services::WEB, "session.whoami"),
    (services::CONVERSATION, "conversation.history"),
    (services::PROFILE, "profile.get"),
    (services::CATALOG, "listing.get"),
    (services::TRANSACTION, "request.ping"),
    (services::DIRECTORY, "directory.ping"),
];

/// `wire_invoke` with a caller other than the module's verified owner --
/// for proving `directory.publish`'s `VerifiedOnly` rule refuses an
/// anonymous wire caller. `AuthLevel::System` maps to `CallerOrigin::
/// Anonymous` (only `Delegated`/`Ucan` map to `Verified`).
pub(crate) async fn wire_invoke_as(
    h: &Harness,
    svc: services::Service,
    envelope: &Value,
    auth: AuthLevel,
) -> (Value, Value) {
    let env_str = envelope.to_string();
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "invoke".to_string(),
        params: json!([env_str]),
        id: None,
        idempotency_key: None,
    };
    let anon_caller = CallerContext {
        caller_did: String::new(),
        app_instance: None,
        session: SessionContext::default(),
        auth,
        proof: None,
    };
    let wasm = h
        .wasm
        .engine
        .execute_wasm_json_from_wire(
            &did_for_service(svc.name),
            svc.interface,
            &req,
            Some(anon_caller.clone()),
        )
        .await
        .expect("wasm wire invoke");
    let native = h
        .wire_native
        .iter()
        .find(|(n, _)| *n == svc.name)
        .expect("wire instance")
        .1
        .dispatch(NativeInvocation {
            interface: svc.interface.to_string(),
            method: "invoke".to_string(),
            params: json!([env_str]),
            caller: anon_caller,
        })
        .await
        .expect("native wire invoke");
    (unwrap_payload(wasm), unwrap_payload(native.payload))
}

pub(crate) async fn publish_signed_listing(h: &Harness, envelope: &str) -> (Value, Value) {
    wire_invoke(h, services::DIRECTORY, &env("directory.publish", json!({ "envelope": envelope })))
        .await
}

/// `directory.publish` refuses on a node that has never declared itself a
/// SynOrg (no `settings` row) -- call this before publishing in any
/// scenario that expects the publish to succeed.
pub(crate) async fn ensure_synorg(h: &Harness) {
    both_rpc(
        h,
        "directory.set-settings",
        json!({
            "name": "Guild", "rules": "r", "area": [], "categories": [],
            "support_contact": "s@example.org", "dispute_path": "d",
            "retention_secs": 2_592_000,
            "publication_limits": { "window_secs": 86400, "max_per_window": 20 }
        }),
    )
    .await;
}

/// `wire_invoke` is a `Harness` method; this free function exists only so
/// the helper above can call it with the same signature as `h.wire_invoke`
/// without borrowing conflicts inside this module's scenario functions.
pub(crate) async fn wire_invoke(
    h: &Harness,
    svc: services::Service,
    envelope: &Value,
) -> (Value, Value) {
    h.wire_invoke(svc, envelope).await
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
];

/// The whole security claim of this slice: exactly these three verbs
/// answer anything other than `-32013` over the wire.
pub(crate) const WIRE_REACHABLE_DIRECTORY_VERBS: &[&str] =
    &["directory.search", "directory.info", "directory.publish"];

/// `directory.set-settings` on the second directory, so its `directory.info`
/// probe answers and `directory.publish` is not refused as "no SynOrg".
pub(crate) async fn ensure_dir2_synorg(h: &Harness) {
    let (w, n) = h
        .dir2_local(
            "directory.set-settings",
            json!({
                "name": "Second Guild", "rules": "r", "area": [], "categories": [],
                "support_contact": "s@example.org", "dispute_path": "d",
                "retention_secs": 2_592_000,
                "publication_limits": { "window_secs": 86400, "max_per_window": 50 }
            }),
        )
        .await;
    assert!(w["result"].is_object(), "dir2 set-settings wasm: {w}");
    assert!(n["result"].is_object(), "dir2 set-settings native: {n}");
}

/// Signs one listing on this node and publishes it into the **second**
/// directory (a local dispatch on that instance -- this is setup, not the
/// path under test, and `directory.publish` over the wire needs a verified
/// caller). Returns the signed envelope string.
pub(crate) async fn publish_listing_to_dir2(h: &Harness, slug: &str, title: &str) -> String {
    let (_id, gw, _gn) = set_and_get(h, full_listing_params(slug, title)).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();
    let (pw, pn) = h.dir2_local("directory.publish", json!({ "envelope": e })).await;
    assert!(pw["result"]["listing_id"].is_string(), "dir2 publish wasm: {pw}");
    assert!(pn["result"]["listing_id"].is_string(), "dir2 publish native: {pn}");
    e
}

/// The same, into this node's own (primary) directory over the wire.
pub(crate) async fn publish_listing_to_primary(h: &Harness, slug: &str, title: &str) -> String {
    let (_id, gw, _gn) = set_and_get(h, full_listing_params(slug, title)).await;
    let e = gw["result"]["envelope"].as_str().unwrap().to_string();
    let (pw, pn) = publish_signed_listing(h, &e).await;
    assert!(pw["result"]["listing_id"].is_string(), "primary publish wasm: {pw}");
    assert!(pn["result"]["listing_id"].is_string(), "primary publish native: {pn}");
    e
}

/// One build's full fan-out: mint a run, drive one `query-source` per
/// source, merge. Returns `(run_id, merged)`. Each build must mint and use
/// its *own* run id -- `start-run` folds the guest's own wall clock into
/// the id (`run_{secs}_{n}`), and the two builds' clocks are unsynchronized
/// (permitted difference 7), so a run id minted on one build is not
/// guaranteed to exist on the other.
pub(crate) async fn fan_out_one(h: &Harness, wasm: bool, sources: &[&str]) -> (String, Value) {
    let start = one_rpc(h, wasm, "directory.start-run", json!({})).await;
    let run_id = start["result"]["run_id"].as_str().unwrap().to_string();
    for did in sources {
        one_rpc(
            h,
            wasm,
            "directory.query-source",
            json!({ "run_id": run_id, "source": did, "query": {} }),
        )
        .await;
    }
    let merged = one_rpc(h, wasm, "directory.merge", json!({ "run_id": run_id })).await;
    (run_id, merged)
}

/// Adds the sources, then runs `fan_out_one` on each build. Returns each
/// build's run id and merged response.
pub(crate) async fn fan_out(h: &Harness, sources: &[&str]) -> (String, String, Value, Value) {
    for did in sources {
        both_rpc(h, "directory.add-source", json!({ "did": did })).await;
    }
    let (rw, mw) = fan_out_one(h, true, sources).await;
    let (rn, mn) = fan_out_one(h, false, sources).await;
    (rw, rn, mw, mn)
}
