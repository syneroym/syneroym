//! Transaction service application logic, target-independent.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap},
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost, AppSigning,
    types::{
        data_layer::{
            CollectionSchema, IndexDefinition, IndexType, QueryOptions, RecordWriteValue,
        },
        proxy::CallTarget,
        signing::{Principal, RecordDraft},
    },
};
use syneroym_roym_core::{
    admit,
    area::Area,
    backup::{
        BUNDLE_VERSION, Bundle, BundleManifest, SECTION_AGREEMENTS, SECTION_CARDS, SECTION_QUOTES,
        SECTION_REQUESTS,
    },
    card::{self, CARD_CONTENT_TYPE},
    clock,
    conversation::Direction,
    envelope::{Request, Response},
    record::{Envelope, RECORD_AGREEMENT_RECEIPT, RECORD_QUOTE, RECORD_REQUEST},
    services,
    signing::{self, CertificateError},
    transaction::{
        self, AGREEMENT_RECEIPT_VERSION, AgreedTerms, AgreementReceiptPayload,
        MAX_CARDS_PER_CONVERSATION, MAX_QUOTE_LIFETIME_SECS, MIN_QUOTE_LIFETIME_SECS,
        QUOTE_VERSION, QuotePayload, REQUEST_VERSION, ReceiptHalf, RecordVerdict, RequestPayload,
        Role, SYNC_OVERLAP, SYNC_WINDOW, TimeWindow, pair_state,
    },
};

// Schema version 2: the service gains its first state.
pub const SCHEMA_VERSION: u32 = 2;

pub const REQUESTS: &str = "requests";
pub const REQUEST_HISTORY: &str = "request_history";
pub const QUOTES: &str = "quotes";
pub const QUOTE_HISTORY: &str = "quote_history";
pub const AGREEMENTS: &str = "agreements";
pub const CARDS: &str = "cards";
pub const SYNC_STATE: &str = "sync_state";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecordPointerRow {
    envelope: String,
    record_id: String,
    id: String,
    conversation: String,
    sequence: u32,
    issuer: String,
    mine: bool,
    updated_at_secs: u64,
    version_count: u64,
    #[serde(default)]
    issued_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_record_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    consumer_did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    declined_at_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    decline_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgreementRow {
    quote_record_id: String,
    conversation: String,
    consumer_did: String,
    provider_did: String,
    terms: AgreedTerms,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    consumer: Option<ReceiptHalf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider: Option<ReceiptHalf>,
    updated_at_secs: u64,
}

impl AgreementRow {
    fn half(&self, role: Role) -> Option<&ReceiptHalf> {
        match role {
            Role::Consumer => self.consumer.as_ref(),
            Role::Provider => self.provider.as_ref(),
        }
    }

    fn set_half(&mut self, role: Role, half: ReceiptHalf) {
        match role {
            Role::Consumer => self.consumer = Some(half),
            Role::Provider => self.provider = Some(half),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CardRow {
    message_id: String,
    conversation: String,
    direction: Direction,
    sender_timestamp_ms: i64,
    card_type: String,
    version: u32,
    known: bool,
    verified: bool,
    #[serde(default)]
    expired: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    issuer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    record_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revocation_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    stored_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    declined: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SyncStateRow {
    scanned_count: u64,
}

pub async fn status<H: AppHost>(_host: &H) -> Result<String, String> {
    Ok(json!({
        "service": services::TRANSACTION.name,
        "schema_version": SCHEMA_VERSION,
    })
    .to_string())
}

pub async fn invoke<H: AppHost>(host: &H, req: Request) -> Response {
    if let Some(resp) = admit::require_internal(host).await {
        return resp;
    }
    if let Some(resp) = signing::handle_certificate_verb(host, "transaction.", &req).await {
        return resp;
    }

    match req.method.as_str() {
        "request.ping" | "quote.ping" | "agreement.ping" | "receipt.ping" => {
            Response::ok(json!({ "service": services::TRANSACTION.name }))
        }
        "request.set" => request_set(host, &req).await,
        "request.get" => request_get(host, &req).await,
        "request.list" => request_list(host, &req).await,
        "request.history" => request_history(host, &req).await,
        "request.verify" => verify_verb(host, &req, RecordKind::Request).await,

        "quote.set" => quote_set(host, &req).await,
        "quote.get" => quote_get(host, &req).await,
        "quote.list" => quote_list(host, &req).await,
        "quote.history" => quote_history(host, &req).await,
        "quote.verify" => verify_verb(host, &req, RecordKind::Quote).await,
        "quote.decline" => quote_decline(host, &req).await,

        "agreement.accept" => agreement_accept(host, &req).await,
        "agreement.get" => agreement_get(host, &req).await,
        "agreement.list" => agreement_list(host, &req).await,
        "agreement.verify" => verify_verb(host, &req, RecordKind::AgreementReceipt).await,

        "transaction.sync" => sync(host, &req).await,
        "transaction.thread" => thread(host, &req).await,
        "transaction.export" => export(host).await,
        "transaction.import" => import(host, &req).await,
        other => Response::method_not_found(other),
    }
}

async fn resolve_principal_and_owner<H: AppHost>(
    host: &H,
    now: u64,
) -> Result<(Principal, String), Response> {
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(CertificateError::NoOwner) => {
            return Err(Response::invalid_params("this installation has no recorded owner"));
        }
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let (principal, _master) = match signing::person_principal(host, now).await {
        Ok(res) => res,
        Err(CertificateError::NotEnrolled) => {
            return Err(Response::invalid_params("signing-not-enrolled"));
        }
        Err(CertificateError::Expired(t)) => {
            return Err(Response::invalid_params(format!("signing-certificate-expired at {t}")));
        }
        Err(CertificateError::Stale { installed_for, current }) => {
            return Err(Response::invalid_params(format!(
                "signing-certificate-stale: {installed_for} vs {current}"
            )));
        }
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    Ok((principal, owner))
}

async fn ensure_coll<H: AppHost>(
    host: &H,
    name: &str,
    indexes: &[IndexDefinition],
) -> Result<(), String> {
    AppDataLayer::create_collection(
        host,
        CollectionSchema { name: name.to_string(), indexes: indexes.to_vec() },
    )
    .await
    .map_err(|e| e.to_string())
}

fn idx(field: &str, ty: IndexType) -> IndexDefinition {
    IndexDefinition { field_name: field.to_string(), type_: ty }
}

async fn ensure_collections<H: AppHost>(host: &H) -> Result<(), String> {
    ensure_coll(
        host,
        REQUESTS,
        &[idx("conversation", IndexType::String), idx("updated_at_secs", IndexType::Numeric)],
    )
    .await?;
    ensure_coll(host, REQUEST_HISTORY, &[]).await?;
    ensure_coll(
        host,
        QUOTES,
        &[idx("conversation", IndexType::String), idx("updated_at_secs", IndexType::Numeric)],
    )
    .await?;
    ensure_coll(host, QUOTE_HISTORY, &[]).await?;
    ensure_coll(host, AGREEMENTS, &[idx("conversation", IndexType::String)]).await?;
    ensure_coll(
        host,
        CARDS,
        &[idx("conversation", IndexType::String), idx("sender_timestamp_ms", IndexType::Numeric)],
    )
    .await?;
    ensure_coll(host, SYNC_STATE, &[]).await?;
    Ok(())
}

async fn put_row<H: AppHost, T: Serialize>(
    host: &H,
    collection: &str,
    id: &str,
    val: &T,
) -> Result<(), String> {
    let payload = serde_json::to_vec(val).map_err(|e| e.to_string())?;
    AppDataLayer::put(
        host,
        collection.to_string(),
        RecordWriteValue { id: id.to_string(), payload },
    )
    .await
    .map_err(|e| e.to_string())
}

async fn get_row<T: for<'a> Deserialize<'a>, H: AppHost>(
    host: &H,
    collection: &str,
    id: &str,
) -> Result<Option<T>, String> {
    let row = AppDataLayer::get(host, collection.to_string(), id.to_string())
        .await
        .map_err(|e| e.to_string())?;
    match row {
        Some(r) => serde_json::from_slice(&r.payload).map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

async fn put_bytes<H: AppHost>(
    host: &H,
    collection: &str,
    id: &str,
    bytes: &[u8],
) -> Result<(), String> {
    AppDataLayer::put(
        host,
        collection.to_string(),
        RecordWriteValue { id: id.to_string(), payload: bytes.to_vec() },
    )
    .await
    .map_err(|e| e.to_string())
}

async fn get_bytes<H: AppHost>(
    host: &H,
    collection: &str,
    id: &str,
) -> Result<Option<Vec<u8>>, String> {
    let row = AppDataLayer::get(host, collection.to_string(), id.to_string())
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.map(|r| r.payload))
}

async fn collect<H: AppHost>(host: &H, collection: &str) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let page = AppDataLayer::query(
            host,
            collection.to_string(),
            QueryOptions { filter: None, limit: Some(500), cursor: cursor.clone() },
        )
        .await
        .map_err(|e| e.to_string())?;
        for r in page.records {
            if let Ok(parsed) = serde_json::from_slice::<Value>(&r.payload) {
                out.push(json!({ "id": r.id, "payload": parsed }));
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(out)
}

async fn count_mine<H: AppHost>(
    host: &H,
    collection: &str,
    conversation: &str,
    owner: &str,
) -> Result<u32, String> {
    let filter = json!({
        "conversation": conversation,
        "issuer": owner,
    })
    .to_string();
    let mut count = 0;
    let mut cursor = None;
    loop {
        let page = AppDataLayer::query(
            host,
            collection.to_string(),
            QueryOptions { filter: Some(filter.clone()), limit: Some(500), cursor: cursor.clone() },
        )
        .await
        .map_err(|e| e.to_string())?;
        count += page.records.len() as u32;
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(count)
}

async fn count_cards_for_conversation<H: AppHost>(
    host: &H,
    conversation: &str,
) -> Result<usize, String> {
    let filter = json!({ "conversation": conversation }).to_string();
    let mut count = 0;
    let mut cursor = None;
    loop {
        let page = AppDataLayer::query(
            host,
            CARDS.to_string(),
            QueryOptions { filter: Some(filter.clone()), limit: Some(500), cursor: cursor.clone() },
        )
        .await
        .map_err(|e| e.to_string())?;
        count += page.records.len();
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(count)
}

async fn conversation_call<H: AppHost>(
    host: &H,
    method: &str,
    params: Value,
) -> Result<Response, String> {
    let req = json!({ "method": method, "params": params }).to_string();
    let raw = host
        .call(
            CallTarget::Dependency(services::CONVERSATION.name.to_string()),
            services::CONVERSATION.interface.to_string(),
            "invoke".to_string(),
            json!([req]).to_string(),
            None,
        )
        .await
        .map_err(|e| format!("{method}: {e:?}"))?;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
async fn file_own_card<H: AppHost>(
    host: &H,
    message_id: &str,
    conversation: &str,
    card_type: &str,
    version: u32,
    envelope_json: &str,
    sender_timestamp_ms: i64,
    now: u64,
    version_count: Option<u64>,
) -> Result<(), String> {
    let envelope = Envelope::from_json(envelope_json).map_err(|e| e.to_string())?;
    let record_id = envelope.record_id().map_err(|e| e.to_string())?;
    let card_row = CardRow {
        message_id: message_id.to_string(),
        conversation: conversation.to_string(),
        direction: Direction::Outgoing,
        sender_timestamp_ms,
        card_type: card_type.to_string(),
        version,
        known: true,
        verified: true,
        expired: false,
        reason: None,
        issuer: Some(envelope.issuer),
        record_id: Some(record_id),
        revocation_status: Some("not-revoked".to_string()),
        data: Some(envelope.payload),
        stored_at_secs: now,
        declined: None,
        version_count,
    };
    put_row(host, CARDS, message_id, &card_row).await
}

#[derive(Debug, Deserialize)]
struct RequestSetParams {
    conversation: String,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    listing_id: Option<String>,
    #[serde(default)]
    categories: Vec<String>,
    description: String,
    #[serde(default)]
    area: Option<Area>,
    #[serde(default)]
    window: Option<TimeWindow>,
    data_use_notice: String,
}

async fn request_set<H: AppHost>(host: &H, req: &Request) -> Response {
    let now = clock::now_secs();
    let (principal, owner) = match resolve_principal_and_owner(host, now).await {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };

    let params: RequestSetParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let (sequence, supersedes, next_count) = if let Some(ref id) = params.request_id {
        let prior: RecordPointerRow = match get_row(host, REQUESTS, id).await {
            Ok(Some(p)) => p,
            Ok(None) => return Response::invalid_params("no such request"),
            Err(e) => return Response::internal_error(e),
        };
        if !prior.mine {
            return Response::invalid_params("this request is not yours to revise");
        }
        if prior.conversation != params.conversation {
            return Response::invalid_params("request belongs to another conversation");
        }
        (prior.sequence, Some(prior.record_id), prior.version_count + 1)
    } else {
        let count = match count_mine(host, REQUESTS, &params.conversation, &owner).await {
            Ok(c) => c,
            Err(e) => return Response::internal_error(e),
        };
        (count + 1, None, 1)
    };

    let request_id = match transaction::derive_request_id(&params.conversation, &owner, sequence) {
        Ok(id) => id,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let payload = RequestPayload {
        request_id: request_id.clone(),
        conversation: params.conversation.clone(),
        sequence,
        listing_id: params.listing_id,
        categories: params.categories,
        description: params.description,
        area: params.area,
        window: params.window,
        data_use_notice: params.data_use_notice,
    };
    if let Err(e) = payload.validate() {
        return Response::invalid_params(e.to_string());
    }

    let payload_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let draft = RecordDraft {
        version: REQUEST_VERSION,
        record_type: RECORD_REQUEST.to_string(),
        subject: request_id.clone(),
        payload: payload_str,
        expires_at_secs: None,
        supersedes,
    };

    let envelope_json = match AppSigning::sign_record(host, draft, principal).await {
        Ok(json) => json,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let envelope = match Envelope::from_json(&envelope_json) {
        Ok(env) => env,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    if envelope.issuer != owner {
        return Response::internal_error(
            "the host signed under an issuer this service did not ask for",
        );
    }

    let record_id = match envelope.record_id() {
        Ok(id) => id,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    // The record is stored before the card is sent. A send that fails leaves a
    // signed record this node holds and the peer does not, which a later `set`
    // or a retry repairs; the reverse -- a card the peer holds and this node
    // cannot show -- has no repair.
    if let Err(e) = put_bytes(host, REQUEST_HISTORY, &record_id, envelope_json.as_bytes()).await {
        return Response::internal_error(e);
    }

    let pointer = RecordPointerRow {
        envelope: envelope_json.clone(),
        record_id: record_id.clone(),
        id: request_id.clone(),
        conversation: params.conversation.clone(),
        sequence,
        issuer: owner.clone(),
        mine: true,
        updated_at_secs: now,
        version_count: next_count,
        issued_at_secs: now,
        request_record_id: None,
        consumer_did: None,
        declined_at_secs: None,
        decline_note: None,
    };
    if let Err(e) = put_row(host, REQUESTS, &request_id, &pointer).await {
        return Response::internal_error(e);
    }

    let body = match card::card_body(RECORD_REQUEST, REQUEST_VERSION, &envelope_json) {
        Ok(b) => b,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let send_resp = conversation_call(
        host,
        "conversation.send",
        json!({
            "conversation": params.conversation,
            "body": body,
            "content_type": CARD_CONTENT_TYPE,
        }),
    )
    .await;

    let (message_id, state, sender_timestamp_ms, mut send_error) = match send_resp {
        Ok(r) if r.error.is_none() => {
            let res = r.result.unwrap_or(Value::Null);
            let mid = res.get("message_id").and_then(Value::as_str).unwrap_or("").to_string();
            let st = res.get("state").and_then(Value::as_str).unwrap_or("").to_string();
            let ts =
                res.get("sender_timestamp_ms").and_then(Value::as_i64).unwrap_or(now as i64 * 1000);
            (mid, st, ts, None)
        }
        Ok(r) => {
            let err_msg = r.error.map(|e| e.message).unwrap_or_else(|| "send error".to_string());
            (String::new(), "not-sent".to_string(), now as i64 * 1000, Some(err_msg))
        }
        Err(e) => (String::new(), "not-sent".to_string(), now as i64 * 1000, Some(e)),
    };

    if !message_id.is_empty()
        && let Err(e) = file_own_card(
            host,
            &message_id,
            &params.conversation,
            RECORD_REQUEST,
            REQUEST_VERSION,
            &envelope_json,
            sender_timestamp_ms,
            now,
            Some(next_count),
        )
        .await
    {
        send_error = send_error.or(Some(format!("file own card failed: {e}")));
    }

    let mut out = json!({
        "request_id": request_id,
        "record_id": record_id,
        "version_count": next_count,
        "message_id": message_id,
        "state": state,
    });
    if let Some(err) = send_error {
        out["send_error"] = json!(err);
    }
    Response::ok(out)
}

#[derive(Debug, Deserialize)]
struct QuoteSetParams {
    request_record_id: String,
    #[serde(default)]
    quote_id: Option<String>,
    #[serde(default)]
    listing_id: Option<String>,
    expires_in_secs: u64,
    terms: Value,
}

async fn quote_set<H: AppHost>(host: &H, req: &Request) -> Response {
    let now = clock::now_secs();
    let (principal, owner) = match resolve_principal_and_owner(host, now).await {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };

    let params: QuoteSetParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    if !(MIN_QUOTE_LIFETIME_SECS..=MAX_QUOTE_LIFETIME_SECS).contains(&params.expires_in_secs) {
        return Response::invalid_params(format!(
            "expires_in_secs must be between {MIN_QUOTE_LIFETIME_SECS} and \
             {MAX_QUOTE_LIFETIME_SECS}"
        ));
    }

    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let req_envelope_bytes = match get_bytes(host, REQUEST_HISTORY, &params.request_record_id).await
    {
        Ok(Some(b)) => b,
        Ok(None) => return Response::invalid_params("no such request"),
        Err(e) => return Response::internal_error(e),
    };

    let req_envelope_str = match String::from_utf8(req_envelope_bytes) {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let req_verdict = transaction::verify_request(&req_envelope_str, now);
    if !req_verdict.verified {
        return Response::invalid_params(format!(
            "the request this quote answers does not verify: {}",
            req_verdict.reason.as_deref().unwrap_or("unknown")
        ));
    }
    let req_payload = match req_verdict.payload {
        Some(p) => p,
        None => return Response::internal_error("missing request payload"),
    };
    let consumer_did = match req_verdict.issuer {
        Some(i) => i,
        None => return Response::internal_error("missing request issuer"),
    };
    if consumer_did == owner {
        return Response::invalid_params("a request cannot be quoted by the person who made it");
    }
    let conversation = req_payload.conversation;

    let (sequence, supersedes, next_count) = if let Some(ref id) = params.quote_id {
        let prior: RecordPointerRow = match get_row(host, QUOTES, id).await {
            Ok(Some(p)) => p,
            Ok(None) => return Response::invalid_params("no such quote"),
            Err(e) => return Response::internal_error(e),
        };
        if !prior.mine {
            return Response::invalid_params("this quote is not yours to revise");
        }
        if prior.conversation != conversation {
            return Response::invalid_params("quote belongs to another conversation");
        }
        if prior.request_record_id.as_deref() != Some(&params.request_record_id) {
            return Response::invalid_params("a new version of a quote answers the same request");
        }
        (prior.sequence, Some(prior.record_id), prior.version_count + 1)
    } else {
        let count = match count_mine(host, QUOTES, &conversation, &owner).await {
            Ok(c) => c,
            Err(e) => return Response::internal_error(e),
        };
        (count + 1, None, 1)
    };

    let expires_at_secs = now + params.expires_in_secs;
    let quote_id = match transaction::derive_quote_id(&conversation, &owner, sequence) {
        Ok(id) => id,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let mut terms_map = match params.terms {
        Value::Object(m) => m,
        _ => return Response::invalid_params("terms must be an object"),
    };
    terms_map.insert("quote_expires_at_secs".to_string(), json!(expires_at_secs));
    let terms: AgreedTerms = match serde_json::from_value(Value::Object(terms_map)) {
        Ok(t) => t,
        Err(e) => return Response::invalid_params(format!("invalid terms: {e}")),
    };

    let payload = QuotePayload {
        quote_id: quote_id.clone(),
        conversation: conversation.clone(),
        sequence,
        request_record_id: params.request_record_id.clone(),
        listing_id: params.listing_id,
        consumer_did: consumer_did.clone(),
        terms,
    };
    if let Err(e) = payload.validate() {
        return Response::invalid_params(e.to_string());
    }

    let payload_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let draft = RecordDraft {
        version: QUOTE_VERSION,
        record_type: RECORD_QUOTE.to_string(),
        subject: quote_id.clone(),
        payload: payload_str,
        expires_at_secs: Some(expires_at_secs),
        supersedes,
    };

    let envelope_json = match AppSigning::sign_record(host, draft, principal).await {
        Ok(json) => json,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let envelope = match Envelope::from_json(&envelope_json) {
        Ok(env) => env,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    if envelope.issuer != owner {
        return Response::internal_error(
            "the host signed under an issuer this service did not ask for",
        );
    }

    let record_id = match envelope.record_id() {
        Ok(id) => id,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    if let Err(e) = put_bytes(host, QUOTE_HISTORY, &record_id, envelope_json.as_bytes()).await {
        return Response::internal_error(e);
    }

    let pointer = RecordPointerRow {
        envelope: envelope_json.clone(),
        record_id: record_id.clone(),
        id: quote_id.clone(),
        conversation: conversation.clone(),
        sequence,
        issuer: owner.clone(),
        mine: true,
        updated_at_secs: now,
        version_count: next_count,
        issued_at_secs: now,
        request_record_id: Some(params.request_record_id),
        consumer_did: Some(consumer_did),
        declined_at_secs: None,
        decline_note: None,
    };
    if let Err(e) = put_row(host, QUOTES, &quote_id, &pointer).await {
        return Response::internal_error(e);
    }

    let body = match card::card_body(RECORD_QUOTE, QUOTE_VERSION, &envelope_json) {
        Ok(b) => b,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let send_resp = conversation_call(
        host,
        "conversation.send",
        json!({
            "conversation": conversation,
            "body": body,
            "content_type": CARD_CONTENT_TYPE,
        }),
    )
    .await;

    let (message_id, state, sender_timestamp_ms, mut send_error) = match send_resp {
        Ok(r) if r.error.is_none() => {
            let res = r.result.unwrap_or(Value::Null);
            let mid = res.get("message_id").and_then(Value::as_str).unwrap_or("").to_string();
            let st = res.get("state").and_then(Value::as_str).unwrap_or("").to_string();
            let ts =
                res.get("sender_timestamp_ms").and_then(Value::as_i64).unwrap_or(now as i64 * 1000);
            (mid, st, ts, None)
        }
        Ok(r) => {
            let err_msg = r.error.map(|e| e.message).unwrap_or_else(|| "send error".to_string());
            (String::new(), "not-sent".to_string(), now as i64 * 1000, Some(err_msg))
        }
        Err(e) => (String::new(), "not-sent".to_string(), now as i64 * 1000, Some(e)),
    };

    if !message_id.is_empty()
        && let Err(e) = file_own_card(
            host,
            &message_id,
            &conversation,
            RECORD_QUOTE,
            QUOTE_VERSION,
            &envelope_json,
            sender_timestamp_ms,
            now,
            Some(next_count),
        )
        .await
    {
        send_error = send_error.or(Some(format!("file own card failed: {e}")));
    }

    let mut out = json!({
        "quote_id": quote_id,
        "record_id": record_id,
        "version_count": next_count,
        "expires_at_secs": expires_at_secs,
        "message_id": message_id,
        "state": state,
    });
    if let Some(err) = send_error {
        out["send_error"] = json!(err);
    }
    Response::ok(out)
}

#[derive(Debug, Deserialize)]
struct AgreementAcceptParams {
    quote_record_id: String,
}

async fn agreement_accept<H: AppHost>(host: &H, req: &Request) -> Response {
    let now = clock::now_secs();
    let (principal, owner) = match resolve_principal_and_owner(host, now).await {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };

    let params: AgreementAcceptParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let quote_envelope_bytes = match get_bytes(host, QUOTE_HISTORY, &params.quote_record_id).await {
        Ok(Some(b)) => b,
        Ok(None) => return Response::invalid_params("no such quote"),
        Err(e) => return Response::internal_error(e),
    };

    let quote_envelope_str = match String::from_utf8(quote_envelope_bytes) {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let verdict = transaction::verify_quote(&quote_envelope_str, now);
    if !verdict.verified {
        return Response::invalid_params(format!(
            "the quote does not verify: {}",
            verdict.reason.as_deref().unwrap_or("unknown")
        ));
    }
    if verdict.expired {
        return Response::invalid_params("quote-expired");
    }

    let quote_payload = match verdict.payload {
        Some(p) => p,
        None => return Response::internal_error("missing quote payload"),
    };
    let provider_did = match verdict.issuer {
        Some(i) => i,
        None => return Response::internal_error("missing quote issuer"),
    };
    let consumer_did = quote_payload.consumer_did;

    let role = if owner == consumer_did {
        Role::Consumer
    } else if owner == provider_did {
        Role::Provider
    } else {
        return Response::invalid_params("this installation is neither party to that quote");
    };

    let mut row: AgreementRow = match get_row(host, AGREEMENTS, &params.quote_record_id).await {
        Ok(Some(r)) => r,
        Ok(None) => AgreementRow {
            quote_record_id: params.quote_record_id.clone(),
            conversation: quote_payload.conversation.clone(),
            consumer_did: consumer_did.clone(),
            provider_did: provider_did.clone(),
            terms: quote_payload.terms.clone(),
            consumer: None,
            provider: None,
            updated_at_secs: now,
        },
        Err(e) => return Response::internal_error(e),
    };

    if let Some(existing) = row.half(role) {
        let pair = pair_state(row.consumer.as_ref(), row.provider.as_ref());
        return Response::ok(json!({
            "quote_record_id": params.quote_record_id,
            "role": role,
            "record_id": existing.record_id,
            "pair": pair,
            "message_id": "",
            "state": "already-accepted",
        }));
    }

    let payload = AgreementReceiptPayload {
        quote_record_id: params.quote_record_id.clone(),
        consumer_did: consumer_did.clone(),
        provider_did: provider_did.clone(),
        role,
        terms: quote_payload.terms.clone(),
    };
    if let Err(e) = payload.validate() {
        return Response::invalid_params(e.to_string());
    }

    let payload_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let draft = RecordDraft {
        version: AGREEMENT_RECEIPT_VERSION,
        record_type: RECORD_AGREEMENT_RECEIPT.to_string(),
        subject: params.quote_record_id.clone(),
        payload: payload_str,
        expires_at_secs: None,
        supersedes: None,
    };

    let envelope_json = match AppSigning::sign_record(host, draft, principal).await {
        Ok(json) => json,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let envelope = match Envelope::from_json(&envelope_json) {
        Ok(env) => env,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    if envelope.issuer != owner {
        return Response::internal_error(
            "the host signed under an issuer this service did not ask for",
        );
    }

    let record_id = match envelope.record_id() {
        Ok(id) => id,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let half = ReceiptHalf {
        envelope: envelope_json.clone(),
        record_id: record_id.clone(),
        issuer: owner.clone(),
        issued_at_secs: envelope.issued_at_secs,
    };

    row.set_half(role, half);
    row.updated_at_secs = now;
    if let Err(e) = put_row(host, AGREEMENTS, &params.quote_record_id, &row).await {
        return Response::internal_error(e);
    }

    let body = match card::card_body(
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &envelope_json,
    ) {
        Ok(b) => b,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let send_resp = conversation_call(
        host,
        "conversation.send",
        json!({
            "conversation": quote_payload.conversation,
            "body": body,
            "content_type": CARD_CONTENT_TYPE,
        }),
    )
    .await;

    let (message_id, state, sender_timestamp_ms, mut send_error) = match send_resp {
        Ok(r) if r.error.is_none() => {
            let res = r.result.unwrap_or(Value::Null);
            let mid = res.get("message_id").and_then(Value::as_str).unwrap_or("").to_string();
            let st = res.get("state").and_then(Value::as_str).unwrap_or("").to_string();
            let ts =
                res.get("sender_timestamp_ms").and_then(Value::as_i64).unwrap_or(now as i64 * 1000);
            (mid, st, ts, None)
        }
        Ok(r) => {
            let err_msg = r.error.map(|e| e.message).unwrap_or_else(|| "send error".to_string());
            (String::new(), "not-sent".to_string(), now as i64 * 1000, Some(err_msg))
        }
        Err(e) => (String::new(), "not-sent".to_string(), now as i64 * 1000, Some(e)),
    };

    if !message_id.is_empty()
        && let Err(e) = file_own_card(
            host,
            &message_id,
            &quote_payload.conversation,
            RECORD_AGREEMENT_RECEIPT,
            AGREEMENT_RECEIPT_VERSION,
            &envelope_json,
            sender_timestamp_ms,
            now,
            None,
        )
        .await
    {
        send_error = send_error.or(Some(format!("file own card failed: {e}")));
    }

    let pair = pair_state(row.consumer.as_ref(), row.provider.as_ref());
    let mut out = json!({
        "quote_record_id": params.quote_record_id,
        "role": role,
        "record_id": record_id,
        "pair": pair,
        "message_id": message_id,
        "state": state,
    });
    if let Some(err) = send_error {
        out["send_error"] = json!(err);
    }
    Response::ok(out)
}

#[derive(Debug, Deserialize)]
struct QuoteDeclineParams {
    quote_record_id: String,
    #[serde(default)]
    note: Option<String>,
}

async fn quote_decline<H: AppHost>(host: &H, req: &Request) -> Response {
    let now = clock::now_secs();
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(CertificateError::NoOwner) => {
            return Response::invalid_params("this installation has no recorded owner");
        }
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let params: QuoteDeclineParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let quote_envelope_bytes = match get_bytes(host, QUOTE_HISTORY, &params.quote_record_id).await {
        Ok(Some(b)) => b,
        Ok(None) => return Response::invalid_params("no such quote"),
        Err(e) => return Response::internal_error(e),
    };

    let quote_envelope_str = match String::from_utf8(quote_envelope_bytes) {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let verdict = transaction::verify_quote(&quote_envelope_str, now);
    if !verdict.verified {
        return Response::invalid_params(format!(
            "the quote does not verify: {}",
            verdict.reason.as_deref().unwrap_or("unknown")
        ));
    }
    let quote_payload = match verdict.payload {
        Some(p) => p,
        None => return Response::internal_error("missing quote payload"),
    };

    if owner != quote_payload.consumer_did {
        return Response::invalid_params("only the consumer can decline this quote");
    }

    if let Ok(Some(agr)) =
        get_row::<AgreementRow, _>(host, AGREEMENTS, &params.quote_record_id).await
        && (agr.consumer.is_some() || agr.provider.is_some())
    {
        return Response::invalid_params("cannot decline a quote that has already been accepted");
    }

    let mut pointer: RecordPointerRow = match get_row(host, QUOTES, &quote_payload.quote_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return Response::invalid_params("no such quote"),
        Err(e) => return Response::internal_error(e),
    };

    pointer.declined_at_secs = Some(now);
    pointer.decline_note = params.note.clone();
    pointer.updated_at_secs = now;
    if let Err(e) = put_row(host, QUOTES, &quote_payload.quote_id, &pointer).await {
        return Response::internal_error(e);
    }

    Response::ok(json!({
        "quote_record_id": params.quote_record_id,
        "declined": true,
        "note": params.note,
    }))
}

#[derive(Debug, Deserialize)]
struct SyncParams {
    conversation: String,
    #[serde(default)]
    full: bool,
}

struct FileCardResult {
    filed: bool,
    refused: bool,
    unknown: bool,
    countersigned: bool,
}

async fn sync<H: AppHost>(host: &H, req: &Request) -> Response {
    let now = clock::now_secs();
    let params: SyncParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let sync_state: SyncStateRow = match get_row(host, SYNC_STATE, &params.conversation).await {
        Ok(Some(s)) => s,
        Ok(None) => SyncStateRow { scanned_count: 0 },
        Err(e) => return Response::internal_error(e),
    };

    let start = if params.full { 0 } else { sync_state.scanned_count.saturating_sub(SYNC_OVERLAP) };

    let page_resp = match conversation_call(
        host,
        "conversation.history",
        json!({
            "conversation": params.conversation,
            "limit": SYNC_WINDOW,
            "cursor": start,
        }),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return Response::internal_error(e),
    };

    if let Some(err) = page_resp.error {
        return Response::internal_error(err.message);
    }

    let res = page_resp.result.unwrap_or(Value::Null);
    let messages = res.get("messages").and_then(Value::as_array).cloned().unwrap_or_default();

    let mut card_count = match count_cards_for_conversation(host, &params.conversation).await {
        Ok(c) => c,
        Err(e) => return Response::internal_error(e),
    };

    let mut scanned: u32 = 0;
    let mut filed: u32 = 0;
    let mut refused: u32 = 0;
    let mut unknown: u32 = 0;
    let mut countersigned: u32 = 0;

    let mut offset = start;
    let mut first_declined: Option<u64> = None;
    let owner = signing::owner_did(host).await.unwrap_or_default();

    for m in messages {
        offset += 1;
        scanned += 1;
        let content_type = m.get("content_type").and_then(Value::as_str).unwrap_or("");
        if content_type != CARD_CONTENT_TYPE {
            continue;
        }
        if m.get("deleted_at_secs").and_then(Value::as_u64).is_some() {
            continue;
        }
        let msg_id = match m.get("id").and_then(Value::as_str) {
            Some(id) => id,
            None => continue,
        };
        let exists = match AppDataLayer::get(host, CARDS.to_string(), msg_id.to_string()).await {
            Ok(o) => o.is_some(),
            Err(e) => return Response::internal_error(e.to_string()),
        };
        if exists {
            continue;
        }
        if card_count >= MAX_CARDS_PER_CONVERSATION {
            first_declined = first_declined.or(Some(offset - 1));
            continue;
        }

        let file_res = match file_incoming_card(host, &m, &params.conversation, now, &owner).await {
            Ok(res) => res,
            Err(e) => return Response::internal_error(e),
        };
        card_count += 1;
        if file_res.filed {
            filed += 1;
        }
        if file_res.refused {
            refused += 1;
        }
        if file_res.unknown {
            unknown += 1;
        }
        if file_res.countersigned {
            countersigned += 1;
        }
    }

    let new_scanned_count = match first_declined {
        Some(o) => o,
        None => sync_state.scanned_count.max(offset),
    };
    if let Err(e) = put_row(
        host,
        SYNC_STATE,
        &params.conversation,
        &SyncStateRow { scanned_count: new_scanned_count },
    )
    .await
    {
        return Response::internal_error(e);
    }

    Response::ok(json!({
        "scanned": scanned,
        "filed": filed,
        "refused": refused,
        "unknown": unknown,
        "countersigned": countersigned,
        "scanned_count": new_scanned_count,
    }))
}

async fn file_incoming_card<H: AppHost>(
    host: &H,
    m: &Value,
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let msg_id = m.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let direction_str = m.get("direction").and_then(Value::as_str).unwrap_or("incoming");
    let direction =
        if direction_str == "outgoing" { Direction::Outgoing } else { Direction::Incoming };
    let sender_timestamp_ms =
        m.get("sender_timestamp_ms").and_then(Value::as_i64).unwrap_or(now as i64 * 1000);

    let mut row = CardRow {
        message_id: msg_id.clone(),
        conversation: conversation.to_string(),
        direction,
        sender_timestamp_ms,
        card_type: String::new(),
        version: 0,
        known: false,
        verified: false,
        expired: false,
        reason: None,
        issuer: None,
        record_id: None,
        revocation_status: None,
        data: None,
        stored_at_secs: now,
        declined: None,
        version_count: None,
    };

    let body = match m.get("body").and_then(Value::as_str) {
        Some(b) => b,
        None => {
            row.reason = Some("no body".to_string());
            put_row(host, CARDS, &msg_id, &row).await?;
            return Ok(FileCardResult {
                filed: true,
                refused: true,
                unknown: false,
                countersigned: false,
            });
        }
    };

    let card = match card::parse_card(body) {
        Ok(c) => c,
        Err(e) => {
            row.reason = Some(e.to_string());
            put_row(host, CARDS, &msg_id, &row).await?;
            return Ok(FileCardResult {
                filed: true,
                refused: true,
                unknown: false,
                countersigned: false,
            });
        }
    };

    row.card_type = card.card_type.clone();
    row.version = card.version;
    row.known = card::is_known_card(&card.card_type, card.version);
    if !row.known {
        put_row(host, CARDS, &msg_id, &row).await?;
        return Ok(FileCardResult {
            filed: true,
            refused: false,
            unknown: true,
            countersigned: false,
        });
    }

    match card.card_type.as_str() {
        "request" => {
            let v = transaction::verify_request(&card.envelope, now);
            if !v.verified {
                row.reason = v.reason.or_else(|| Some("request does not verify".to_string()));
                put_row(host, CARDS, &msg_id, &row).await?;
                return Ok(FileCardResult {
                    filed: true,
                    refused: true,
                    unknown: false,
                    countersigned: false,
                });
            }
            let payload = match v.payload.as_ref() {
                Some(p) => p,
                None => {
                    row.reason = Some("missing payload".to_string());
                    put_row(host, CARDS, &msg_id, &row).await?;
                    return Ok(FileCardResult {
                        filed: true,
                        refused: true,
                        unknown: false,
                        countersigned: false,
                    });
                }
            };
            if payload.conversation != conversation {
                row.reason = Some("card names another conversation".to_string());
                put_row(host, CARDS, &msg_id, &row).await?;
                return Ok(FileCardResult {
                    filed: true,
                    refused: true,
                    unknown: false,
                    countersigned: false,
                });
            }
            let version_count =
                match store_received_request(host, &card.envelope, &v, payload, now, owner).await {
                    Ok(vc) => vc,
                    Err(e) => {
                        row.reason = Some(e);
                        put_row(host, CARDS, &msg_id, &row).await?;
                        return Ok(FileCardResult {
                            filed: true,
                            refused: true,
                            unknown: false,
                            countersigned: false,
                        });
                    }
                };
            row.verified = true;
            row.version_count = Some(version_count);
            row.data = Some(serde_json::to_value(payload).unwrap_or(Value::Null));
            row.issuer = v.issuer;
            row.record_id = v.record_id;
            row.revocation_status = v.revocation_status;
            put_row(host, CARDS, &msg_id, &row).await?;
            Ok(FileCardResult { filed: true, refused: false, unknown: false, countersigned: false })
        }
        "quote" => {
            let v = transaction::verify_quote(&card.envelope, now);
            if !v.verified {
                row.reason = v.reason.or_else(|| Some("quote does not verify".to_string()));
                put_row(host, CARDS, &msg_id, &row).await?;
                return Ok(FileCardResult {
                    filed: true,
                    refused: true,
                    unknown: false,
                    countersigned: false,
                });
            }
            let payload = match v.payload.as_ref() {
                Some(p) => p,
                None => {
                    row.reason = Some("missing payload".to_string());
                    put_row(host, CARDS, &msg_id, &row).await?;
                    return Ok(FileCardResult {
                        filed: true,
                        refused: true,
                        unknown: false,
                        countersigned: false,
                    });
                }
            };
            if payload.conversation != conversation {
                row.reason = Some("card names another conversation".to_string());
                put_row(host, CARDS, &msg_id, &row).await?;
                return Ok(FileCardResult {
                    filed: true,
                    refused: true,
                    unknown: false,
                    countersigned: false,
                });
            }
            let req_bytes =
                match get_bytes(host, REQUEST_HISTORY, &payload.request_record_id).await? {
                    Some(b) => b,
                    None => {
                        row.reason = Some("answers a request this node does not hold".to_string());
                        put_row(host, CARDS, &msg_id, &row).await?;
                        return Ok(FileCardResult {
                            filed: true,
                            refused: true,
                            unknown: false,
                            countersigned: false,
                        });
                    }
                };
            let req_str = match String::from_utf8(req_bytes) {
                Ok(s) => s,
                Err(_) => {
                    row.reason = Some("stored request envelope is invalid utf8".to_string());
                    put_row(host, CARDS, &msg_id, &row).await?;
                    return Ok(FileCardResult {
                        filed: true,
                        refused: true,
                        unknown: false,
                        countersigned: false,
                    });
                }
            };
            let req_v = transaction::verify_request(&req_str, now);
            if !req_v.verified || req_v.issuer.as_deref() != Some(payload.consumer_did.as_str()) {
                row.reason = Some("quote consumer_did does not match request issuer".to_string());
                put_row(host, CARDS, &msg_id, &row).await?;
                return Ok(FileCardResult {
                    filed: true,
                    refused: true,
                    unknown: false,
                    countersigned: false,
                });
            }
            let version_count =
                match store_received_quote(host, &card.envelope, &v, payload, now, owner).await {
                    Ok(vc) => vc,
                    Err(e) => {
                        row.reason = Some(e);
                        put_row(host, CARDS, &msg_id, &row).await?;
                        return Ok(FileCardResult {
                            filed: true,
                            refused: true,
                            unknown: false,
                            countersigned: false,
                        });
                    }
                };
            row.verified = true;
            row.expired = v.expired;
            row.version_count = Some(version_count);
            row.data = Some(serde_json::to_value(payload).unwrap_or(Value::Null));
            row.issuer = v.issuer;
            row.record_id = v.record_id;
            row.revocation_status = v.revocation_status;
            put_row(host, CARDS, &msg_id, &row).await?;
            Ok(FileCardResult { filed: true, refused: false, unknown: false, countersigned: false })
        }
        "agreement-receipt" => {
            let v = transaction::verify_agreement_receipt(&card.envelope, now);
            if !v.verified {
                row.reason =
                    v.reason.or_else(|| Some("agreement receipt does not verify".to_string()));
                put_row(host, CARDS, &msg_id, &row).await?;
                return Ok(FileCardResult {
                    filed: true,
                    refused: true,
                    unknown: false,
                    countersigned: false,
                });
            }
            let payload = match v.payload.as_ref() {
                Some(p) => p,
                None => {
                    row.reason = Some("missing payload".to_string());
                    put_row(host, CARDS, &msg_id, &row).await?;
                    return Ok(FileCardResult {
                        filed: true,
                        refused: true,
                        unknown: false,
                        countersigned: false,
                    });
                }
            };
            let q_bytes = match get_bytes(host, QUOTE_HISTORY, &payload.quote_record_id).await? {
                Some(b) => b,
                None => {
                    row.reason = Some("attests a quote this node does not hold".to_string());
                    put_row(host, CARDS, &msg_id, &row).await?;
                    return Ok(FileCardResult {
                        filed: true,
                        refused: true,
                        unknown: false,
                        countersigned: false,
                    });
                }
            };
            let q_str = match String::from_utf8(q_bytes) {
                Ok(s) => s,
                Err(_) => {
                    row.reason = Some("stored quote envelope is invalid utf8".to_string());
                    put_row(host, CARDS, &msg_id, &row).await?;
                    return Ok(FileCardResult {
                        filed: true,
                        refused: true,
                        unknown: false,
                        countersigned: false,
                    });
                }
            };
            let qv = transaction::verify_quote(&q_str, now);
            if !qv.verified {
                row.reason = Some("the quote it attests does not verify".to_string());
                put_row(host, CARDS, &msg_id, &row).await?;
                return Ok(FileCardResult {
                    filed: true,
                    refused: true,
                    unknown: false,
                    countersigned: false,
                });
            }
            let q_payload = match qv.payload.as_ref() {
                Some(qp) => qp,
                None => {
                    row.reason = Some("quote missing payload".to_string());
                    put_row(host, CARDS, &msg_id, &row).await?;
                    return Ok(FileCardResult {
                        filed: true,
                        refused: true,
                        unknown: false,
                        countersigned: false,
                    });
                }
            };
            if q_payload.terms != payload.terms {
                row.reason = Some("terms differ from the quote".to_string());
                put_row(host, CARDS, &msg_id, &row).await?;
                return Ok(FileCardResult {
                    filed: true,
                    refused: true,
                    unknown: false,
                    countersigned: false,
                });
            }
            if payload.consumer_did != q_payload.consumer_did
                || payload.provider_did != qv.issuer.as_deref().unwrap_or("")
            {
                row.reason = Some("names the wrong parties".to_string());
                put_row(host, CARDS, &msg_id, &row).await?;
                return Ok(FileCardResult {
                    filed: true,
                    refused: true,
                    unknown: false,
                    countersigned: false,
                });
            }
            let issued_at = v.issued_at_secs.unwrap_or(0);
            if issued_at >= payload.terms.quote_expires_at_secs {
                row.reason = Some("accepted after the quote expired".to_string());
                put_row(host, CARDS, &msg_id, &row).await?;
                return Ok(FileCardResult {
                    filed: true,
                    refused: true,
                    unknown: false,
                    countersigned: false,
                });
            }
            let mut row_agr: AgreementRow =
                match get_row(host, AGREEMENTS, &payload.quote_record_id).await? {
                    Some(r) => r,
                    None => AgreementRow {
                        quote_record_id: payload.quote_record_id.clone(),
                        conversation: conversation.to_string(),
                        consumer_did: payload.consumer_did.clone(),
                        provider_did: payload.provider_did.clone(),
                        terms: q_payload.terms.clone(),
                        consumer: None,
                        provider: None,
                        updated_at_secs: now,
                    },
                };
            if row_agr.half(payload.role).is_none() {
                let half = ReceiptHalf {
                    envelope: card.envelope.clone(),
                    record_id: v.record_id.clone().unwrap_or_default(),
                    issuer: v.issuer.clone().unwrap_or_default(),
                    issued_at_secs: issued_at,
                };
                row_agr.set_half(payload.role, half);
                row_agr.updated_at_secs = now;
                put_row(host, AGREEMENTS, &payload.quote_record_id, &row_agr).await?;
            }
            row.verified = true;
            row.data = Some(serde_json::to_value(payload).unwrap_or(Value::Null));
            row.issuer = v.issuer;
            row.record_id = v.record_id;
            row.revocation_status = v.revocation_status;
            put_row(host, CARDS, &msg_id, &row).await?;
            let countersigned = maybe_countersign(host, &mut row_agr, &qv, now, owner).await?;
            Ok(FileCardResult { filed: true, refused: false, unknown: false, countersigned })
        }
        _ => {
            row.reason = Some("a known card type with no producer in this build".to_string());
            put_row(host, CARDS, &msg_id, &row).await?;
            Ok(FileCardResult { filed: true, refused: true, unknown: false, countersigned: false })
        }
    }
}

async fn store_received_request<H: AppHost>(
    host: &H,
    envelope: &str,
    v: &RecordVerdict<RequestPayload>,
    payload: &RequestPayload,
    now: u64,
    owner: &str,
) -> Result<u64, String> {
    let record_id = v.record_id.as_deref().unwrap_or_default();
    let prior: Option<RecordPointerRow> = get_row(host, REQUESTS, &payload.request_id).await?;
    let incoming_issued = v.issued_at_secs.unwrap_or(now);
    if let Some(existing) = &prior
        && existing.record_id != record_id
        && incoming_issued <= existing.issued_at_secs
        && v.supersedes.as_deref() != Some(existing.record_id.as_str())
    {
        return Err("a newer or equal version of this request is already held here".to_string());
    }
    put_bytes(host, REQUEST_HISTORY, record_id, envelope.as_bytes()).await?;
    let version_count = if let Some(existing) = &prior {
        if existing.record_id == record_id {
            existing.version_count
        } else {
            existing.version_count + 1
        }
    } else {
        1
    };
    let mine = !owner.is_empty() && v.issuer.as_deref() == Some(owner);
    let pointer = RecordPointerRow {
        envelope: envelope.to_string(),
        record_id: record_id.to_string(),
        id: payload.request_id.clone(),
        conversation: payload.conversation.clone(),
        sequence: payload.sequence,
        issuer: v.issuer.clone().unwrap_or_default(),
        mine,
        updated_at_secs: now,
        version_count,
        issued_at_secs: incoming_issued,
        request_record_id: None,
        consumer_did: None,
        declined_at_secs: None,
        decline_note: None,
    };
    put_row(host, REQUESTS, &payload.request_id, &pointer).await?;
    Ok(version_count)
}

async fn store_received_quote<H: AppHost>(
    host: &H,
    envelope: &str,
    v: &RecordVerdict<QuotePayload>,
    payload: &QuotePayload,
    now: u64,
    owner: &str,
) -> Result<u64, String> {
    let record_id = v.record_id.as_deref().unwrap_or_default();
    let prior: Option<RecordPointerRow> = get_row(host, QUOTES, &payload.quote_id).await?;
    let incoming_issued = v.issued_at_secs.unwrap_or(now);
    if let Some(existing) = &prior
        && existing.record_id != record_id
        && incoming_issued <= existing.issued_at_secs
        && v.supersedes.as_deref() != Some(existing.record_id.as_str())
    {
        return Err("a newer or equal version of this quote is already held here".to_string());
    }
    put_bytes(host, QUOTE_HISTORY, record_id, envelope.as_bytes()).await?;
    let (version_count, declined_at_secs, decline_note) = if let Some(existing) = &prior {
        if existing.record_id == record_id {
            // Idempotent replay: preserve decline and version count
            (existing.version_count, existing.declined_at_secs, existing.decline_note.clone())
        } else {
            // New version: fresh offer, not declined
            (existing.version_count + 1, None, None)
        }
    } else {
        (1, None, None)
    };
    let mine = !owner.is_empty() && v.issuer.as_deref() == Some(owner);
    let pointer = RecordPointerRow {
        envelope: envelope.to_string(),
        record_id: record_id.to_string(),
        id: payload.quote_id.clone(),
        conversation: payload.conversation.clone(),
        sequence: payload.sequence,
        issuer: v.issuer.clone().unwrap_or_default(),
        mine,
        updated_at_secs: now,
        version_count,
        issued_at_secs: incoming_issued,
        request_record_id: Some(payload.request_record_id.clone()),
        consumer_did: Some(payload.consumer_did.clone()),
        declined_at_secs,
        decline_note,
    };
    put_row(host, QUOTES, &payload.quote_id, &pointer).await?;
    Ok(version_count)
}

async fn maybe_countersign<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    qv: &RecordVerdict<QuotePayload>,
    now: u64,
    owner: &str,
) -> Result<bool, String> {
    if row.provider.is_some() {
        return Ok(false);
    }
    if row.consumer.is_none() {
        return Ok(false);
    }
    if owner.is_empty() || owner != row.provider_did {
        return Ok(false);
    }
    let q_payload = match qv.payload.as_ref() {
        Some(p) => p,
        None => return Ok(false),
    };
    if now >= row.terms.quote_expires_at_secs {
        return Ok(false);
    }
    let (principal, _master) = match signing::person_principal(host, now).await {
        Ok(p) => p,
        Err(_) => return Ok(false),
    };

    let payload = AgreementReceiptPayload {
        quote_record_id: row.quote_record_id.clone(),
        consumer_did: row.consumer_did.clone(),
        provider_did: row.provider_did.clone(),
        role: Role::Provider,
        terms: q_payload.terms.clone(),
    };
    if payload.validate().is_err() {
        return Ok(false);
    }
    let payload_str = serde_json::to_string(&payload).map_err(|e| e.to_string())?;

    let draft = RecordDraft {
        version: AGREEMENT_RECEIPT_VERSION,
        record_type: RECORD_AGREEMENT_RECEIPT.to_string(),
        subject: row.quote_record_id.clone(),
        payload: payload_str,
        expires_at_secs: None,
        supersedes: None,
    };

    let envelope_json = match AppSigning::sign_record(host, draft, principal).await {
        Ok(json) => json,
        Err(e) => return Err(e.to_string()),
    };

    let envelope = Envelope::from_json(&envelope_json).map_err(|e| e.to_string())?;
    let record_id = envelope.record_id().map_err(|e| e.to_string())?;

    let half = ReceiptHalf {
        envelope: envelope_json.clone(),
        record_id: record_id.clone(),
        issuer: owner.to_string(),
        issued_at_secs: envelope.issued_at_secs,
    };
    row.provider = Some(half);
    row.updated_at_secs = now;
    put_row(host, AGREEMENTS, &row.quote_record_id, row).await?;

    let body = card::card_body(RECORD_AGREEMENT_RECEIPT, AGREEMENT_RECEIPT_VERSION, &envelope_json)
        .map_err(|e| e.to_string())?;

    let send_resp = conversation_call(
        host,
        "conversation.send",
        json!({
            "conversation": row.conversation,
            "body": body,
            "content_type": CARD_CONTENT_TYPE,
        }),
    )
    .await;

    let (message_id, sender_timestamp_ms) = match send_resp {
        Ok(r) if r.error.is_none() => {
            let res = r.result.unwrap_or(Value::Null);
            let mid = res.get("message_id").and_then(Value::as_str).unwrap_or("").to_string();
            let ts =
                res.get("sender_timestamp_ms").and_then(Value::as_i64).unwrap_or(now as i64 * 1000);
            (mid, ts)
        }
        _ => (String::new(), now as i64 * 1000),
    };

    if !message_id.is_empty() {
        file_own_card(
            host,
            &message_id,
            &row.conversation,
            RECORD_AGREEMENT_RECEIPT,
            AGREEMENT_RECEIPT_VERSION,
            &envelope_json,
            sender_timestamp_ms,
            now,
            None,
        )
        .await?;
    }

    Ok(true)
}

#[derive(Debug, Deserialize)]
struct ThreadParams {
    conversation: String,
    #[serde(default = "default_thread_limit")]
    limit: usize,
    #[serde(default)]
    cursor: usize,
}

fn default_thread_limit() -> usize {
    200
}

async fn thread<H: AppHost>(host: &H, req: &Request) -> Response {
    let params: ThreadParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let now = clock::now_secs();
    let filter = json!({ "conversation": params.conversation }).to_string();

    let mut quote_map: HashMap<String, RecordPointerRow> = HashMap::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            QUOTES.to_string(),
            QueryOptions { filter: Some(filter.clone()), limit: Some(500), cursor: cursor.clone() },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            if let Ok(pointer) = serde_json::from_slice::<RecordPointerRow>(&r.payload) {
                quote_map.insert(pointer.id.clone(), pointer);
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }

    let mut request_map: HashMap<String, RecordPointerRow> = HashMap::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            REQUESTS.to_string(),
            QueryOptions { filter: Some(filter.clone()), limit: Some(500), cursor: cursor.clone() },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            if let Ok(pointer) = serde_json::from_slice::<RecordPointerRow>(&r.payload) {
                request_map.insert(pointer.id.clone(), pointer);
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }

    let mut rows = Vec::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            CARDS.to_string(),
            QueryOptions { filter: Some(filter.clone()), limit: Some(500), cursor: cursor.clone() },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            if let Ok(mut row) = serde_json::from_slice::<CardRow>(&r.payload) {
                if row.card_type == "quote" && row.verified {
                    if let Some(qid) =
                        row.data.as_ref().and_then(|q| q.get("quote_id")).and_then(Value::as_str)
                        && let Some(pointer) = quote_map.get(qid)
                    {
                        if pointer.declined_at_secs.is_some() {
                            row.declined = Some(true);
                        }
                        if row.version_count.is_none() {
                            row.version_count = Some(pointer.version_count);
                        }
                    }
                    if let Some(exp) = row
                        .data
                        .as_ref()
                        .and_then(|q| q.get("terms"))
                        .and_then(|t| t.get("quote_expires_at_secs"))
                        .and_then(Value::as_u64)
                    {
                        row.expired = now >= exp;
                    }
                } else if row.card_type == "request"
                    && row.verified
                    && let Some(rid) =
                        row.data.as_ref().and_then(|r| r.get("request_id")).and_then(Value::as_str)
                    && let Some(pointer) = request_map.get(rid)
                    && row.version_count.is_none()
                {
                    row.version_count = Some(pointer.version_count);
                }
                rows.push(row);
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }

    // Sort by (sender_timestamp_ms, message_id). The author component is
    // absent because a card row's author is its issuer (a person DID), not the
    // conversation author, so mixing them would order transcripts differently.
    rows.sort_by(|a, b| {
        a.sender_timestamp_ms
            .cmp(&b.sender_timestamp_ms)
            .then_with(|| a.message_id.cmp(&b.message_id))
    });

    let page: Vec<CardRow> = rows.into_iter().skip(params.cursor).take(params.limit).collect();
    Response::ok(json!({ "cards": page }))
}

#[derive(Debug, Deserialize)]
struct RequestGetParams {
    request_id: String,
}

async fn request_get<H: AppHost>(host: &H, req: &Request) -> Response {
    let params: RequestGetParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };
    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }
    let pointer: Option<RecordPointerRow> = match get_row(host, REQUESTS, &params.request_id).await
    {
        Ok(p) => p,
        Err(e) => return Response::internal_error(e),
    };
    match pointer {
        Some(p) => Response::ok(json!({
            "envelope": p.envelope,
            "record_id": p.record_id,
            "request_id": p.id,
            "conversation": p.conversation,
            "mine": p.mine,
            "updated_at_secs": p.updated_at_secs,
            "version_count": p.version_count,
        })),
        None => Response::ok(Value::Null),
    }
}

#[derive(Debug, Deserialize)]
struct QuoteGetParams {
    quote_id: String,
}

async fn quote_get<H: AppHost>(host: &H, req: &Request) -> Response {
    let params: QuoteGetParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };
    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }
    let pointer: Option<RecordPointerRow> = match get_row(host, QUOTES, &params.quote_id).await {
        Ok(p) => p,
        Err(e) => return Response::internal_error(e),
    };
    match pointer {
        Some(p) => Response::ok(json!({
            "envelope": p.envelope,
            "record_id": p.record_id,
            "quote_id": p.id,
            "conversation": p.conversation,
            "mine": p.mine,
            "updated_at_secs": p.updated_at_secs,
            "version_count": p.version_count,
            "request_record_id": p.request_record_id,
            "consumer_did": p.consumer_did,
            "declined_at_secs": p.declined_at_secs,
            "decline_note": p.decline_note,
        })),
        None => Response::ok(Value::Null),
    }
}

#[derive(Debug, Deserialize)]
struct ListParams {
    #[serde(default)]
    conversation: Option<String>,
    #[serde(default)]
    mine: Option<bool>,
    #[serde(default = "default_list_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
}

fn default_list_limit() -> usize {
    200
}

async fn request_list<H: AppHost>(host: &H, req: &Request) -> Response {
    let params: ListParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };
    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let mut filter_obj = Map::new();
    if let Some(ref c) = params.conversation {
        filter_obj.insert("conversation".to_string(), json!(c));
    }
    if let Some(m) = params.mine {
        filter_obj.insert("mine".to_string(), json!(m));
    }
    let filter =
        if filter_obj.is_empty() { None } else { Some(Value::Object(filter_obj).to_string()) };

    let mut rows = Vec::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            REQUESTS.to_string(),
            QueryOptions { filter: filter.clone(), limit: Some(500), cursor: cursor.clone() },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            if let Ok(p) = serde_json::from_slice::<RecordPointerRow>(&r.payload) {
                rows.push(p);
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    rows.sort_by_key(|p| Reverse(p.updated_at_secs));
    let page: Vec<Value> = rows
        .into_iter()
        .skip(params.offset)
        .take(params.limit)
        .map(|p| {
            json!({
                "envelope": p.envelope,
                "record_id": p.record_id,
                "request_id": p.id,
                "conversation": p.conversation,
                "mine": p.mine,
                "updated_at_secs": p.updated_at_secs,
                "version_count": p.version_count,
            })
        })
        .collect();
    Response::ok(json!({ "requests": page }))
}

async fn quote_list<H: AppHost>(host: &H, req: &Request) -> Response {
    let params: ListParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };
    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let mut filter_obj = Map::new();
    if let Some(ref c) = params.conversation {
        filter_obj.insert("conversation".to_string(), json!(c));
    }
    if let Some(m) = params.mine {
        filter_obj.insert("mine".to_string(), json!(m));
    }
    let filter =
        if filter_obj.is_empty() { None } else { Some(Value::Object(filter_obj).to_string()) };

    let mut rows = Vec::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            QUOTES.to_string(),
            QueryOptions { filter: filter.clone(), limit: Some(500), cursor: cursor.clone() },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            if let Ok(p) = serde_json::from_slice::<RecordPointerRow>(&r.payload) {
                rows.push(p);
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    rows.sort_by_key(|p| Reverse(p.updated_at_secs));
    let page: Vec<Value> = rows
        .into_iter()
        .skip(params.offset)
        .take(params.limit)
        .map(|p| {
            json!({
                "envelope": p.envelope,
                "record_id": p.record_id,
                "quote_id": p.id,
                "conversation": p.conversation,
                "mine": p.mine,
                "updated_at_secs": p.updated_at_secs,
                "version_count": p.version_count,
                "request_record_id": p.request_record_id,
                "consumer_did": p.consumer_did,
            })
        })
        .collect();
    Response::ok(json!({ "quotes": page }))
}

#[derive(Debug, Deserialize)]
struct RequestHistoryParams {
    request_id: String,
}

async fn request_history<H: AppHost>(host: &H, req: &Request) -> Response {
    let params: RequestHistoryParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };
    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }
    let history_filter = json!({ "payload.request_id": params.request_id }).to_string();
    let mut envelopes: Vec<(u64, String)> = Vec::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            REQUEST_HISTORY.to_string(),
            QueryOptions {
                filter: Some(history_filter.clone()),
                limit: Some(500),
                cursor: cursor.clone(),
            },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            let env_str = String::from_utf8_lossy(&r.payload).into_owned();
            if let Ok(env) = Envelope::from_json(&env_str) {
                let matches = env
                    .payload
                    .get("request_id")
                    .and_then(Value::as_str)
                    .map(|id| id == params.request_id)
                    .unwrap_or(false);
                if matches {
                    envelopes.push((env.issued_at_secs, env_str));
                }
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    envelopes.sort_by_key(|(t, _)| *t);
    let out: Vec<Value> = envelopes.into_iter().map(|(_, e)| Value::String(e)).collect();
    Response::ok(json!({ "history": out }))
}

#[derive(Debug, Deserialize)]
struct QuoteHistoryParams {
    quote_id: String,
}

async fn quote_history<H: AppHost>(host: &H, req: &Request) -> Response {
    let params: QuoteHistoryParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };
    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }
    let history_filter = json!({ "payload.quote_id": params.quote_id }).to_string();
    let mut envelopes: Vec<(u64, String)> = Vec::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            QUOTE_HISTORY.to_string(),
            QueryOptions {
                filter: Some(history_filter.clone()),
                limit: Some(500),
                cursor: cursor.clone(),
            },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            let env_str = String::from_utf8_lossy(&r.payload).into_owned();
            if let Ok(env) = Envelope::from_json(&env_str) {
                let matches = env
                    .payload
                    .get("quote_id")
                    .and_then(Value::as_str)
                    .map(|id| id == params.quote_id)
                    .unwrap_or(false);
                if matches {
                    envelopes.push((env.issued_at_secs, env_str));
                }
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    envelopes.sort_by_key(|(t, _)| *t);
    let out: Vec<Value> = envelopes.into_iter().map(|(_, e)| Value::String(e)).collect();
    Response::ok(json!({ "history": out }))
}

#[derive(Debug, Deserialize)]
struct AgreementGetParams {
    quote_record_id: String,
}

async fn agreement_get<H: AppHost>(host: &H, req: &Request) -> Response {
    let params: AgreementGetParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };
    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }
    let row: Option<AgreementRow> = match get_row(host, AGREEMENTS, &params.quote_record_id).await {
        Ok(r) => r,
        Err(e) => return Response::internal_error(e),
    };
    match row {
        Some(r) => {
            let pair = pair_state(r.consumer.as_ref(), r.provider.as_ref());
            Response::ok(json!({
                "quote_record_id": r.quote_record_id,
                "conversation": r.conversation,
                "consumer_did": r.consumer_did,
                "provider_did": r.provider_did,
                "consumer": r.consumer,
                "provider": r.provider,
                "pair": pair,
                "terms": r.terms,
            }))
        }
        None => Response::ok(Value::Null),
    }
}

#[derive(Debug, Deserialize)]
struct AgreementListParams {
    #[serde(default)]
    conversation: Option<String>,
    #[serde(default = "default_list_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
}

async fn agreement_list<H: AppHost>(host: &H, req: &Request) -> Response {
    let params: AgreementListParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };
    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let filter = params.conversation.map(|c| json!({ "conversation": c }).to_string());
    let mut rows = Vec::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            AGREEMENTS.to_string(),
            QueryOptions { filter: filter.clone(), limit: Some(500), cursor: cursor.clone() },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            if let Ok(a) = serde_json::from_slice::<AgreementRow>(&r.payload) {
                rows.push(a);
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    rows.sort_by_key(|a| Reverse(a.updated_at_secs));
    let page: Vec<Value> = rows
        .into_iter()
        .skip(params.offset)
        .take(params.limit)
        .map(|r| {
            let pair = pair_state(r.consumer.as_ref(), r.provider.as_ref());
            json!({
                "quote_record_id": r.quote_record_id,
                "conversation": r.conversation,
                "consumer_did": r.consumer_did,
                "provider_did": r.provider_did,
                "consumer": r.consumer,
                "provider": r.provider,
                "pair": pair,
                "terms": r.terms,
            })
        })
        .collect();
    Response::ok(json!({ "agreements": page }))
}

enum RecordKind {
    Request,
    Quote,
    AgreementReceipt,
}

async fn verify_verb<H: AppHost>(host: &H, req: &Request, kind: RecordKind) -> Response {
    let _ = host;
    let now = clock::now_secs();
    let env_val = match req.params.get("envelope") {
        Some(v) => v,
        None => return Response::invalid_params("envelope is required"),
    };
    let env_str = match env_val {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    match kind {
        RecordKind::Request => Response::ok(json!(transaction::verify_request(&env_str, now))),
        RecordKind::Quote => Response::ok(json!(transaction::verify_quote(&env_str, now))),
        RecordKind::AgreementReceipt => {
            Response::ok(json!(transaction::verify_agreement_receipt(&env_str, now)))
        }
    }
}

/// Exports transaction data: requests, quotes, agreements, and cards.
///
/// Note: `request_history` and `quote_history` are not exported as their
/// own sections because every envelope in them is reachable from a
/// `requests`/`quotes` pointer row or an `agreements` half. On import,
/// they are re-populated from the imported rows.
async fn export<H: AppHost>(host: &H) -> Response {
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    let now = clock::now_secs();
    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }
    let requests = match collect(host, REQUESTS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let quotes = match collect(host, QUOTES).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let agreements = match collect(host, AGREEMENTS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let cards = match collect(host, CARDS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let sections = BTreeMap::from([
        (SECTION_REQUESTS.to_string(), requests),
        (SECTION_QUOTES.to_string(), quotes),
        (SECTION_AGREEMENTS.to_string(), agreements),
        (SECTION_CARDS.to_string(), cards),
    ]);
    let mut manifest_sections = BTreeMap::new();
    for (k, v) in &sections {
        match Bundle::digest(SCHEMA_VERSION, v) {
            Ok(d) => {
                manifest_sections.insert(k.clone(), d);
            }
            Err(e) => return Response::internal_error(e.to_string()),
        }
    }
    let bundle = Bundle {
        manifest: BundleManifest {
            bundle_version: BUNDLE_VERSION,
            produced_at_secs: now,
            subject_did: owner,
            sections: manifest_sections,
        },
        sections,
    };
    match serde_json::to_value(&bundle) {
        Ok(v) => Response::ok(v),
        Err(e) => Response::internal_error(e.to_string()),
    }
}

async fn import<H: AppHost>(host: &H, req: &Request) -> Response {
    let bundle_val = match req.params.get("bundle").cloned().or_else(|| Some(req.params.clone())) {
        Some(v) => v,
        None => return Response::invalid_params("bundle is required"),
    };
    let bundle = match Bundle::from_json(&bundle_val.to_string()) {
        Ok(b) => b,
        Err(e) => return Response::invalid_params(format!("invalid bundle: {e}")),
    };
    if let Err(e) = bundle.check_integrity() {
        return Response::invalid_params(e.to_string());
    }
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    if bundle.manifest.subject_did != owner {
        return Response::invalid_params(format!(
            "bundle belongs to '{}', this node holds '{}'",
            bundle.manifest.subject_did, owner
        ));
    }
    for (name, declared) in &bundle.manifest.sections {
        if declared.schema_version != SCHEMA_VERSION {
            return Response::invalid_params(format!(
                "section '{name}' has schema version {}, this node requires {SCHEMA_VERSION}",
                declared.schema_version
            ));
        }
    }

    let now = clock::now_secs();
    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let req_rows = bundle.sections.get(SECTION_REQUESTS).cloned().unwrap_or_default();
    let quote_rows = bundle.sections.get(SECTION_QUOTES).cloned().unwrap_or_default();
    let agr_rows = bundle.sections.get(SECTION_AGREEMENTS).cloned().unwrap_or_default();
    let card_rows = bundle.sections.get(SECTION_CARDS).cloned().unwrap_or_default();

    let mut verified_requests = Vec::new();
    for r in req_rows {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        let mut row: RecordPointerRow = match serde_json::from_value(payload) {
            Ok(row) => row,
            Err(e) => return Response::invalid_params(format!("request '{id}': invalid row: {e}")),
        };
        let v = transaction::verify_request(&row.envelope, now);
        if !v.verified {
            return Response::invalid_params(format!(
                "request '{id}' envelope does not verify: {}",
                v.reason.as_deref().unwrap_or("unknown")
            ));
        }
        let p = match v.payload.as_ref() {
            Some(p) => p,
            None => return Response::invalid_params(format!("request '{id}' has missing payload")),
        };
        if id != p.request_id {
            return Response::invalid_params(format!(
                "request '{id}' declared id does not match payload request_id '{}'",
                p.request_id
            ));
        }
        let verified_record_id = match v.record_id.as_deref() {
            Some(rid) => rid,
            None => return Response::invalid_params(format!("request '{id}' has no record_id")),
        };
        row.record_id = verified_record_id.to_string();
        row.id = p.request_id.clone();
        row.conversation = p.conversation.clone();
        row.sequence = p.sequence;
        row.issuer = v.issuer.clone().unwrap_or_default();
        row.mine = v.issuer.as_deref() == Some(&owner);
        row.issued_at_secs = v.issued_at_secs.unwrap_or(row.issued_at_secs);
        verified_requests.push((p.request_id.clone(), row));
    }

    let mut verified_quotes = Vec::new();
    let mut quotes_by_record_id: HashMap<String, (String, QuotePayload)> = HashMap::new();
    for r in quote_rows {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        let mut row: RecordPointerRow = match serde_json::from_value(payload) {
            Ok(row) => row,
            Err(e) => return Response::invalid_params(format!("quote '{id}': invalid row: {e}")),
        };
        let v = transaction::verify_quote(&row.envelope, now);
        if !v.verified {
            return Response::invalid_params(format!(
                "quote '{id}' envelope does not verify: {}",
                v.reason.as_deref().unwrap_or("unknown")
            ));
        }
        let p = match v.payload.as_ref() {
            Some(p) => p,
            None => return Response::invalid_params(format!("quote '{id}' has missing payload")),
        };
        if id != p.quote_id {
            return Response::invalid_params(format!(
                "quote '{id}' declared id does not match payload quote_id '{}'",
                p.quote_id
            ));
        }
        let verified_record_id = match v.record_id.as_deref() {
            Some(rid) => rid,
            None => return Response::invalid_params(format!("quote '{id}' has no record_id")),
        };
        row.record_id = verified_record_id.to_string();
        row.id = p.quote_id.clone();
        row.conversation = p.conversation.clone();
        row.sequence = p.sequence;
        row.issuer = v.issuer.clone().unwrap_or_default();
        row.mine = v.issuer.as_deref() == Some(&owner);
        row.issued_at_secs = v.issued_at_secs.unwrap_or(row.issued_at_secs);
        row.request_record_id = Some(p.request_record_id.clone());
        row.consumer_did = Some(p.consumer_did.clone());

        quotes_by_record_id.insert(verified_record_id.to_string(), (row.issuer.clone(), p.clone()));
        verified_quotes.push((p.quote_id.clone(), row));
    }

    let mut verified_agreements = Vec::new();
    for r in agr_rows {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        let mut row: AgreementRow = match serde_json::from_value(payload) {
            Ok(row) => row,
            Err(e) => {
                return Response::invalid_params(format!("agreement '{id}': invalid row: {e}"));
            }
        };
        if id != row.quote_record_id {
            return Response::invalid_params(format!(
                "agreement '{id}' declared id does not match quote_record_id '{}'",
                row.quote_record_id
            ));
        }
        if row.consumer.is_none() && row.provider.is_none() {
            return Response::invalid_params(format!(
                "agreement '{id}' has neither consumer nor provider receipt"
            ));
        }

        let (quote_provider_did, quote_payload) = if let Some(entry) =
            quotes_by_record_id.get(&row.quote_record_id)
        {
            entry.clone()
        } else if let Ok(Some(bytes)) = get_bytes(host, QUOTE_HISTORY, &row.quote_record_id).await {
            let q_str = match String::from_utf8(bytes) {
                Ok(s) => s,
                Err(_) => {
                    return Response::invalid_params(format!(
                        "agreement '{id}': quote in history is invalid utf8"
                    ));
                }
            };
            let qv = transaction::verify_quote(&q_str, now);
            if !qv.verified {
                return Response::invalid_params(format!(
                    "agreement '{id}': quote in history does not verify"
                ));
            }
            match (qv.issuer, qv.payload) {
                (Some(iss), Some(qp)) => (iss, qp),
                _ => {
                    return Response::invalid_params(format!(
                        "agreement '{id}': quote in history has missing issuer or payload"
                    ));
                }
            }
        } else {
            return Response::invalid_params(format!(
                "agreement '{id}' references quote '{}' not present in bundle or node",
                row.quote_record_id
            ));
        };

        if let Some(ref mut c) = row.consumer {
            let cv = transaction::verify_agreement_receipt(&c.envelope, now);
            if !cv.verified {
                return Response::invalid_params(format!(
                    "agreement '{id}' consumer receipt does not verify: {}",
                    cv.reason.as_deref().unwrap_or("unknown")
                ));
            }
            let cp = match cv.payload.as_ref() {
                Some(p) => p,
                None => {
                    return Response::invalid_params(format!(
                        "agreement '{id}' consumer receipt missing payload"
                    ));
                }
            };
            if cp.role != Role::Consumer {
                return Response::invalid_params(format!(
                    "agreement '{id}' consumer receipt has role {:?}",
                    cp.role
                ));
            }
            if cp.quote_record_id != row.quote_record_id {
                return Response::invalid_params(format!(
                    "agreement '{id}' consumer receipt names different quote_record_id '{}'",
                    cp.quote_record_id
                ));
            }
            if cp.consumer_did != quote_payload.consumer_did
                || cp.provider_did != quote_provider_did
            {
                return Response::invalid_params(format!(
                    "agreement '{id}' consumer receipt names wrong parties"
                ));
            }
            if cp.terms != quote_payload.terms {
                return Response::invalid_params(format!(
                    "agreement '{id}' consumer receipt terms differ from quote"
                ));
            }
            let issued_at = cv.issued_at_secs.unwrap_or(0);
            if issued_at >= quote_payload.terms.quote_expires_at_secs {
                return Response::invalid_params(format!(
                    "agreement '{id}' consumer receipt accepted after quote expired"
                ));
            }
            if cv.issuer.as_deref() != Some(cp.consumer_did.as_str()) {
                return Response::invalid_params(format!(
                    "agreement '{id}' consumer receipt issuer does not match consumer_did"
                ));
            }
            let rec_id = cv.record_id.as_deref().unwrap_or_default();
            if c.record_id != rec_id {
                return Response::invalid_params(format!(
                    "agreement '{id}' consumer receipt record_id mismatch"
                ));
            }
            c.issuer = cp.consumer_did.clone();
            c.issued_at_secs = issued_at;
        }

        if let Some(ref mut p) = row.provider {
            let pv = transaction::verify_agreement_receipt(&p.envelope, now);
            if !pv.verified {
                return Response::invalid_params(format!(
                    "agreement '{id}' provider receipt does not verify: {}",
                    pv.reason.as_deref().unwrap_or("unknown")
                ));
            }
            let pp = match pv.payload.as_ref() {
                Some(p) => p,
                None => {
                    return Response::invalid_params(format!(
                        "agreement '{id}' provider receipt missing payload"
                    ));
                }
            };
            if pp.role != Role::Provider {
                return Response::invalid_params(format!(
                    "agreement '{id}' provider receipt has role {:?}",
                    pp.role
                ));
            }
            if pp.quote_record_id != row.quote_record_id {
                return Response::invalid_params(format!(
                    "agreement '{id}' provider receipt names different quote_record_id '{}'",
                    pp.quote_record_id
                ));
            }
            if pp.consumer_did != quote_payload.consumer_did
                || pp.provider_did != quote_provider_did
            {
                return Response::invalid_params(format!(
                    "agreement '{id}' provider receipt names wrong parties"
                ));
            }
            if pp.terms != quote_payload.terms {
                return Response::invalid_params(format!(
                    "agreement '{id}' provider receipt terms differ from quote"
                ));
            }
            let issued_at = pv.issued_at_secs.unwrap_or(0);
            if issued_at >= quote_payload.terms.quote_expires_at_secs {
                return Response::invalid_params(format!(
                    "agreement '{id}' provider receipt accepted after quote expired"
                ));
            }
            if pv.issuer.as_deref() != Some(pp.provider_did.as_str()) {
                return Response::invalid_params(format!(
                    "agreement '{id}' provider receipt issuer does not match provider_did"
                ));
            }
            let rec_id = pv.record_id.as_deref().unwrap_or_default();
            if p.record_id != rec_id {
                return Response::invalid_params(format!(
                    "agreement '{id}' provider receipt record_id mismatch"
                ));
            }
            p.issuer = pp.provider_did.clone();
            p.issued_at_secs = issued_at;
        }

        row.consumer_did = quote_payload.consumer_did.clone();
        row.provider_did = quote_provider_did;
        row.terms = quote_payload.terms;

        verified_agreements.push((row.quote_record_id.clone(), row));
    }

    let mut imported_history: HashMap<String, (String, String)> = HashMap::new();

    for (id, row) in verified_requests {
        imported_history
            .insert(row.record_id.clone(), ("request".to_string(), row.envelope.clone()));
        if let Err(e) =
            put_bytes(host, REQUEST_HISTORY, &row.record_id, row.envelope.as_bytes()).await
        {
            return Response::internal_error(e);
        }
        if let Err(e) = put_row(host, REQUESTS, &id, &row).await {
            return Response::internal_error(e);
        }
    }

    for (id, row) in verified_quotes {
        imported_history.insert(row.record_id.clone(), ("quote".to_string(), row.envelope.clone()));
        if let Err(e) =
            put_bytes(host, QUOTE_HISTORY, &row.record_id, row.envelope.as_bytes()).await
        {
            return Response::internal_error(e);
        }
        if let Err(e) = put_row(host, QUOTES, &id, &row).await {
            return Response::internal_error(e);
        }
    }

    for (id, row) in verified_agreements {
        if let Some(ref c) = row.consumer {
            imported_history
                .insert(c.record_id.clone(), ("agreement-receipt".to_string(), c.envelope.clone()));
        }
        if let Some(ref p) = row.provider {
            imported_history
                .insert(p.record_id.clone(), ("agreement-receipt".to_string(), p.envelope.clone()));
        }
        if let Err(e) = put_row(host, AGREEMENTS, &id, &row).await {
            return Response::internal_error(e);
        }
    }

    for r in card_rows {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        let mut row: CardRow = match serde_json::from_value(payload) {
            Ok(row) => row,
            Err(e) => return Response::invalid_params(format!("card '{id}': invalid row: {e}")),
        };
        if let Some(ref rec_id) = row.record_id {
            if let Some((kind, env)) = imported_history.get(rec_id) {
                let is_verified = match kind.as_str() {
                    "request" => transaction::verify_request(env, now).verified,
                    "quote" => transaction::verify_quote(env, now).verified,
                    "agreement-receipt" => transaction::verify_agreement_receipt(env, now).verified,
                    _ => false,
                };
                row.verified = is_verified;
            } else {
                row.verified = false;
                if row.reason.is_none() {
                    row.reason =
                        Some("referenced record history not present in import".to_string());
                }
            }
        } else {
            row.verified = false;
        }
        if let Err(e) = put_row(host, CARDS, id, &row).await {
            return Response::internal_error(e);
        }
    }

    Response::ok(json!({ "imported": true }))
}
