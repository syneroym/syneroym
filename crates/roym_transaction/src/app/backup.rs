//! Transaction data backup: export and import.

pub(crate) mod sections;

use std::collections::{BTreeMap, HashMap};

use serde_json::{Value, json};
use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::QueryOptions};
use syneroym_roym_core::{
    backup::{
        BUNDLE_VERSION, Bundle, BundleManifest, SECTION_AGREEMENTS, SECTION_BOOKINGS,
        SECTION_CARDS, SECTION_FULFILMENTS, SECTION_LEDGER, SECTION_PAYMENTS, SECTION_PROGRESS,
        SECTION_QUOTE_HISTORY, SECTION_QUOTES, SECTION_REQUEST_HISTORY, SECTION_REQUESTS,
    },
    booking, clock,
    envelope::{Request, Response},
    fulfilment, payment,
    signing::{self, CertificateError},
    transaction::{self, QuotePayload, ReceiptHalf, Role},
};

use super::{
    AGREEMENTS, AgreementRow, BOOKINGS, CARDS, CardRow, FULFILMENTS, LEDGER, PAYMENTS, PROGRESS,
    QUOTE_HISTORY, QUOTES, REQUEST_HISTORY, REQUESTS, RecordPointerRow, SCHEMA_VERSION, collect,
    ensure_collections, get_bytes, put_bytes, put_row,
};

/// Exports transaction data: requests, quotes, agreements, cards, and history.
pub(crate) async fn export<H: AppHost>(host: &H) -> Response {
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
    let request_history = match collect_history(host, REQUEST_HISTORY).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let quotes = match collect(host, QUOTES).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let quote_history = match collect_history(host, QUOTE_HISTORY).await {
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
    let ledger = match collect(host, LEDGER).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let bookings = match collect(host, BOOKINGS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let progress = match collect(host, PROGRESS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let payments = match collect(host, PAYMENTS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let fulfilments = match collect(host, FULFILMENTS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let sections = BTreeMap::from([
        (SECTION_REQUESTS.to_string(), requests),
        (SECTION_REQUEST_HISTORY.to_string(), request_history),
        (SECTION_QUOTES.to_string(), quotes),
        (SECTION_QUOTE_HISTORY.to_string(), quote_history),
        (SECTION_AGREEMENTS.to_string(), agreements),
        (SECTION_CARDS.to_string(), cards),
        (SECTION_LEDGER.to_string(), ledger),
        (SECTION_BOOKINGS.to_string(), bookings),
        (SECTION_PROGRESS.to_string(), progress),
        (SECTION_PAYMENTS.to_string(), payments),
        (SECTION_FULFILMENTS.to_string(), fulfilments),
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
    let mut bundle = Bundle {
        manifest: BundleManifest {
            bundle_version: BUNDLE_VERSION,
            produced_at_secs: now,
            subject_did: owner,
            sections: manifest_sections,
        },
        sections,
        manifest_signature: None,
    };
    if let Err(e) = signing::sign_bundle(host, &mut bundle, now).await {
        if matches!(e, CertificateError::NotEnrolled) {
            return Response::invalid_params("signing-not-enrolled");
        }
        return Response::internal_error(e.to_string());
    }
    match serde_json::to_value(&bundle) {
        Ok(v) => Response::ok(v),
        Err(e) => Response::internal_error(e.to_string()),
    }
}

pub(crate) async fn import<H: AppHost>(host: &H, req: &Request) -> Response {
    let bundle_val = match req.params.get("bundle").cloned().or_else(|| Some(req.params.clone())) {
        Some(v) => v,
        None => return Response::invalid_params("bundle is required"),
    };
    let bundle = match Bundle::from_json(&bundle_val.to_string()) {
        Ok(b) => b,
        Err(e) => return Response::invalid_params(format!("invalid bundle: {e}")),
    };
    let now = clock::now_secs();
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    if let Err(resp) = validate_bundle_header(&bundle, &owner, now) {
        return resp;
    }

    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let req_rows = bundle.sections.get(SECTION_REQUESTS).cloned().unwrap_or_default();
    let quote_rows = bundle.sections.get(SECTION_QUOTES).cloned().unwrap_or_default();
    let agr_rows = bundle.sections.get(SECTION_AGREEMENTS).cloned().unwrap_or_default();
    let card_rows = bundle.sections.get(SECTION_CARDS).cloned().unwrap_or_default();

    let verified_requests = match verify_imported_requests(req_rows, &owner, now) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let (verified_quotes, quotes_by_record_id) =
        match verify_imported_quotes(quote_rows, &owner, now) {
            Ok(v) => v,
            Err(e) => return e,
        };
    let verified_agreements =
        match verify_imported_agreements(host, agr_rows, &quotes_by_record_id, now).await {
            Ok(v) => v,
            Err(e) => return e,
        };

    let mut imported_history: HashMap<String, (String, String)> = HashMap::new();
    if let Err(resp) = import_negotiation_sections(
        host,
        &bundle.sections,
        verified_requests,
        verified_quotes,
        &verified_agreements,
        &mut imported_history,
        now,
    )
    .await
    {
        return resp;
    }

    let agreements_map: HashMap<String, AgreementRow> = verified_agreements.into_iter().collect();
    if let Err(resp) = sections::import_vertical_sections(
        host,
        &bundle.sections,
        &agreements_map,
        &quotes_by_record_id,
        &mut imported_history,
        &owner,
        now,
    )
    .await
    {
        return resp;
    }

    if let Err(e) = write_imported_cards(host, card_rows, &imported_history, now).await {
        return e;
    }

    Response::ok(json!({ "imported": true }))
}

fn validate_bundle_header(bundle: &Bundle, owner: &str, now: u64) -> Result<(), Response> {
    if let Err(e) = syneroym_roym_core::backup::check_signed_bundle(bundle, owner, now) {
        return Err(Response::invalid_params(e.to_string()));
    }
    for (name, declared) in &bundle.manifest.sections {
        if declared.schema_version != SCHEMA_VERSION {
            return Err(Response::invalid_params(format!(
                "section '{name}' has schema version {}, this node requires {SCHEMA_VERSION}",
                declared.schema_version
            )));
        }
    }
    Ok(())
}

async fn import_negotiation_sections<H: AppHost>(
    host: &H,
    sections: &BTreeMap<String, Vec<Value>>,
    verified_requests: Vec<(String, RecordPointerRow)>,
    verified_quotes: Vec<(String, RecordPointerRow)>,
    verified_agreements: &[(String, AgreementRow)],
    imported_history: &mut HashMap<String, (String, String)>,
    now: u64,
) -> Result<(), Response> {
    let req_hist_rows = sections.get(SECTION_REQUEST_HISTORY).cloned().unwrap_or_default();
    let quote_hist_rows = sections.get(SECTION_QUOTE_HISTORY).cloned().unwrap_or_default();

    import_history_section(host, REQUEST_HISTORY, "request", req_hist_rows, imported_history, now)
        .await?;
    import_history_section(host, QUOTE_HISTORY, "quote", quote_hist_rows, imported_history, now)
        .await?;
    write_imported_pointers(
        host,
        REQUESTS,
        REQUEST_HISTORY,
        "request",
        verified_requests,
        imported_history,
    )
    .await?;
    write_imported_pointers(
        host,
        QUOTES,
        QUOTE_HISTORY,
        "quote",
        verified_quotes,
        imported_history,
    )
    .await?;
    write_imported_agreements(host, verified_agreements.to_vec(), imported_history).await?;
    Ok(())
}

async fn collect_history<H: AppHost>(host: &H, collection: &str) -> Result<Vec<Value>, String> {
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
            let env_str = String::from_utf8_lossy(&r.payload).into_owned();
            out.push(json!({ "id": r.id, "payload": { "envelope": env_str } }));
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(out)
}

async fn import_history_section<H: AppHost>(
    host: &H,
    collection: &str,
    kind: &str,
    rows: Vec<Value>,
    imported_history: &mut HashMap<String, (String, String)>,
    now: u64,
) -> Result<(), Response> {
    for r in rows {
        let row_id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        let env_str = match payload.get("envelope").and_then(Value::as_str) {
            Some(s) => s,
            None => {
                return Err(Response::invalid_params(format!(
                    "{kind}_history '{row_id}': missing envelope"
                )));
            }
        };

        let (verified, reason, rec_id) = match kind {
            "request" => {
                let v = transaction::verify_request(env_str, now);
                (v.verified, v.reason, v.record_id)
            }
            "quote" => {
                let v = transaction::verify_quote(env_str, now);
                (v.verified, v.reason, v.record_id)
            }
            _ => (false, Some("unknown kind".to_string()), None),
        };

        if !verified {
            return Err(Response::invalid_params(format!(
                "{kind}_history '{row_id}' does not verify: {}",
                reason.as_deref().unwrap_or("unknown")
            )));
        }
        if rec_id.as_deref() != Some(row_id) {
            return Err(Response::invalid_params(format!(
                "{kind}_history id '{row_id}' does not match envelope record_id '{:?}'",
                rec_id
            )));
        }

        if let Err(e) = put_bytes(host, collection, row_id, env_str.as_bytes()).await {
            return Err(Response::internal_error(e));
        }
        imported_history.insert(row_id.to_string(), (kind.to_string(), env_str.to_string()));
    }
    Ok(())
}

/// Verifies each bundled request row's signed envelope and refreshes its
/// pointer fields from the verified payload.
fn verify_imported_requests(
    req_rows: Vec<Value>,
    owner: &str,
    now: u64,
) -> Result<Vec<(String, RecordPointerRow)>, Response> {
    let mut verified_requests = Vec::new();
    for r in req_rows {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        let mut row: RecordPointerRow = match serde_json::from_value(payload) {
            Ok(row) => row,
            Err(e) => {
                return Err(Response::invalid_params(format!("request '{id}': invalid row: {e}")));
            }
        };
        let v = transaction::verify_request(&row.envelope, now);
        if !v.verified {
            return Err(Response::invalid_params(format!(
                "request '{id}' envelope does not verify: {}",
                v.reason.as_deref().unwrap_or("unknown")
            )));
        }
        let p = match v.payload.as_ref() {
            Some(p) => p,
            None => {
                return Err(Response::invalid_params(format!(
                    "request '{id}' has missing payload"
                )));
            }
        };
        if id != p.request_id {
            return Err(Response::invalid_params(format!(
                "request '{id}' declared id does not match payload request_id '{}'",
                p.request_id
            )));
        }
        let verified_record_id = match v.record_id.as_deref() {
            Some(rid) => rid,
            None => {
                return Err(Response::invalid_params(format!("request '{id}' has no record_id")));
            }
        };
        row.record_id = verified_record_id.to_string();
        row.id = p.request_id.clone();
        row.conversation = p.conversation.clone();
        row.sequence = p.sequence;
        row.issuer = v.issuer.clone().unwrap_or_default();
        row.mine = v.issuer.as_deref() == Some(owner);
        row.issued_at_secs = v.issued_at_secs.unwrap_or(row.issued_at_secs);
        verified_requests.push((p.request_id.clone(), row));
    }
    Ok(verified_requests)
}

/// A verified quote's issuer DID, payload, and signer DID, indexed by its
/// record id.
#[derive(Debug, Clone)]
pub(crate) struct ImportedQuote {
    pub(crate) provider_did: String,
    pub(crate) payload: QuotePayload,
    pub(crate) signer_did: Option<String>,
}

pub(crate) type QuotesByRecordId = HashMap<String, ImportedQuote>;

/// Verifies each bundled quote row's signed envelope, refreshes its pointer
/// fields, and indexes it by record id for agreement-row cross-checks.
fn verify_imported_quotes(
    quote_rows: Vec<Value>,
    owner: &str,
    now: u64,
) -> Result<(Vec<(String, RecordPointerRow)>, QuotesByRecordId), Response> {
    let mut verified_quotes = Vec::new();
    let mut quotes_by_record_id: QuotesByRecordId = HashMap::new();
    for r in quote_rows {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        let mut row: RecordPointerRow = match serde_json::from_value(payload) {
            Ok(row) => row,
            Err(e) => {
                return Err(Response::invalid_params(format!("quote '{id}': invalid row: {e}")));
            }
        };
        let v = transaction::verify_quote(&row.envelope, now);
        if !v.verified {
            return Err(Response::invalid_params(format!(
                "quote '{id}' envelope does not verify: {}",
                v.reason.as_deref().unwrap_or("unknown")
            )));
        }
        let p = match v.payload.as_ref() {
            Some(p) => p,
            None => {
                return Err(Response::invalid_params(format!("quote '{id}' has missing payload")));
            }
        };
        if id != p.quote_id {
            return Err(Response::invalid_params(format!(
                "quote '{id}' declared id does not match payload quote_id '{}'",
                p.quote_id
            )));
        }
        let verified_record_id = match v.record_id.as_deref() {
            Some(rid) => rid,
            None => return Err(Response::invalid_params(format!("quote '{id}' has no record_id"))),
        };
        row.record_id = verified_record_id.to_string();
        row.id = p.quote_id.clone();
        row.conversation = p.conversation.clone();
        row.sequence = p.sequence;
        row.issuer = v.issuer.clone().unwrap_or_default();
        row.mine = v.issuer.as_deref() == Some(owner);
        row.issued_at_secs = v.issued_at_secs.unwrap_or(row.issued_at_secs);
        row.request_record_id = Some(p.request_record_id.clone());
        row.consumer_did = Some(p.consumer_did.clone());

        quotes_by_record_id.insert(
            verified_record_id.to_string(),
            ImportedQuote {
                provider_did: row.issuer.clone(),
                payload: p.clone(),
                signer_did: v.signer_did,
            },
        );
        verified_quotes.push((p.quote_id.clone(), row));
    }
    Ok((verified_quotes, quotes_by_record_id))
}

/// Resolves the quote an agreement row's `quote_record_id` answers, first
/// from the bundle's own quotes section and otherwise from this node's
/// quote history.
async fn resolve_quote_for_agreement<H: AppHost>(
    host: &H,
    id: &str,
    quote_record_id: &str,
    quotes_by_record_id: &QuotesByRecordId,
    now: u64,
) -> Result<(String, QuotePayload), Response> {
    if let Some(entry) = quotes_by_record_id.get(quote_record_id) {
        return Ok((entry.provider_did.clone(), entry.payload.clone()));
    }
    let bytes = match get_bytes(host, QUOTE_HISTORY, quote_record_id).await {
        Ok(Some(b)) => b,
        _ => {
            return Err(Response::invalid_params(format!(
                "agreement '{id}' references quote '{quote_record_id}' not present in bundle or \
                 node"
            )));
        }
    };
    let q_str = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => {
            return Err(Response::invalid_params(format!(
                "agreement '{id}': quote in history is invalid utf8"
            )));
        }
    };
    let qv = transaction::verify_quote(&q_str, now);
    if !qv.verified {
        return Err(Response::invalid_params(format!(
            "agreement '{id}': quote in history does not verify"
        )));
    }
    match (qv.issuer, qv.payload) {
        (Some(iss), Some(qp)) => Ok((iss, qp)),
        _ => Err(Response::invalid_params(format!(
            "agreement '{id}': quote in history has missing issuer or payload"
        ))),
    }
}

/// The quote an imported agreement row's receipt halves must agree with.
struct QuoteContext<'a> {
    row_quote_record_id: &'a str,
    payload: &'a QuotePayload,
    provider_did: &'a str,
}

/// Verifies one half (consumer or provider) of an imported agreement
/// receipt against the quote it attests, and refreshes its issuer/timestamp.
fn verify_receipt_half(
    half: &mut ReceiptHalf,
    id: &str,
    label: &str,
    expected_role: Role,
    quote: &QuoteContext<'_>,
    now: u64,
) -> Result<(), Response> {
    let v = transaction::verify_agreement_receipt(&half.envelope, now);
    if !v.verified {
        return Err(Response::invalid_params(format!(
            "agreement '{id}' {label} receipt does not verify: {}",
            v.reason.as_deref().unwrap_or("unknown")
        )));
    }
    let receipt_payload = match v.payload.as_ref() {
        Some(p) => p,
        None => {
            return Err(Response::invalid_params(format!(
                "agreement '{id}' {label} receipt missing payload"
            )));
        }
    };
    if receipt_payload.role != expected_role {
        return Err(Response::invalid_params(format!(
            "agreement '{id}' {label} receipt has role {:?}",
            receipt_payload.role
        )));
    }
    if receipt_payload.quote_record_id != quote.row_quote_record_id {
        return Err(Response::invalid_params(format!(
            "agreement '{id}' {label} receipt names different quote_record_id '{}'",
            receipt_payload.quote_record_id
        )));
    }
    if receipt_payload.consumer_did != quote.payload.consumer_did
        || receipt_payload.provider_did != quote.provider_did
    {
        return Err(Response::invalid_params(format!(
            "agreement '{id}' {label} receipt names wrong parties"
        )));
    }
    if receipt_payload.terms != quote.payload.terms {
        return Err(Response::invalid_params(format!(
            "agreement '{id}' {label} receipt terms differ from quote"
        )));
    }
    let issued_at = v.issued_at_secs.unwrap_or(0);
    if issued_at >= quote.payload.terms.quote_expires_at_secs {
        return Err(Response::invalid_params(format!(
            "agreement '{id}' {label} receipt accepted after quote expired"
        )));
    }
    let expected_issuer = match expected_role {
        Role::Consumer => &receipt_payload.consumer_did,
        Role::Provider => &receipt_payload.provider_did,
    };
    if v.issuer.as_deref() != Some(expected_issuer.as_str()) {
        return Err(Response::invalid_params(format!(
            "agreement '{id}' {label} receipt issuer does not match {label}_did"
        )));
    }
    let rec_id = v.record_id.as_deref().unwrap_or_default();
    if half.record_id != rec_id {
        return Err(Response::invalid_params(format!(
            "agreement '{id}' {label} receipt record_id mismatch"
        )));
    }
    half.issuer = expected_issuer.clone();
    half.issued_at_secs = issued_at;
    Ok(())
}

/// Verifies each bundled agreement row: its own shape, the quote it
/// answers, and whichever of the consumer/provider receipt halves it holds.
async fn verify_imported_agreements<H: AppHost>(
    host: &H,
    agr_rows: Vec<Value>,
    quotes_by_record_id: &QuotesByRecordId,
    now: u64,
) -> Result<Vec<(String, AgreementRow)>, Response> {
    let mut verified_agreements = Vec::new();
    for r in agr_rows {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        let mut row: AgreementRow = match serde_json::from_value(payload) {
            Ok(row) => row,
            Err(e) => {
                return Err(Response::invalid_params(format!(
                    "agreement '{id}': invalid row: {e}"
                )));
            }
        };
        if id != row.quote_record_id {
            return Err(Response::invalid_params(format!(
                "agreement '{id}' declared id does not match quote_record_id '{}'",
                row.quote_record_id
            )));
        }
        if row.consumer.is_none() && row.provider.is_none() {
            return Err(Response::invalid_params(format!(
                "agreement '{id}' has neither consumer nor provider receipt"
            )));
        }

        let (quote_provider_did, quote_payload) =
            resolve_quote_for_agreement(host, id, &row.quote_record_id, quotes_by_record_id, now)
                .await?;
        let quote_ctx = QuoteContext {
            row_quote_record_id: &row.quote_record_id,
            payload: &quote_payload,
            provider_did: &quote_provider_did,
        };

        if let Some(ref mut c) = row.consumer {
            verify_receipt_half(c, id, "consumer", Role::Consumer, &quote_ctx, now)?;
        }
        if let Some(ref mut p) = row.provider {
            verify_receipt_half(p, id, "provider", Role::Provider, &quote_ctx, now)?;
        }

        row.consumer_did = quote_payload.consumer_did.clone();
        row.provider_did = quote_provider_did;
        row.terms = quote_payload.terms;

        verified_agreements.push((row.quote_record_id.clone(), row));
    }
    Ok(verified_agreements)
}

/// Writes a verified pointer row's envelope into `history_collection` and
/// its pointer into `collection`, and indexes it under `kind` for the
/// card-verification pass.
async fn write_imported_pointers<H: AppHost>(
    host: &H,
    collection: &str,
    history_collection: &str,
    kind: &str,
    verified_rows: Vec<(String, RecordPointerRow)>,
    imported_history: &mut HashMap<String, (String, String)>,
) -> Result<(), Response> {
    for (id, row) in verified_rows {
        imported_history.insert(row.record_id.clone(), (kind.to_string(), row.envelope.clone()));
        if let Err(e) =
            put_bytes(host, history_collection, &row.record_id, row.envelope.as_bytes()).await
        {
            return Err(Response::internal_error(e));
        }
        if let Err(e) = put_row(host, collection, &id, &row).await {
            return Err(Response::internal_error(e));
        }
    }
    Ok(())
}

async fn write_imported_agreements<H: AppHost>(
    host: &H,
    verified_agreements: Vec<(String, AgreementRow)>,
    imported_history: &mut HashMap<String, (String, String)>,
) -> Result<(), Response> {
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
            return Err(Response::internal_error(e));
        }
    }
    Ok(())
}

async fn write_imported_cards<H: AppHost>(
    host: &H,
    card_rows: Vec<Value>,
    imported_history: &HashMap<String, (String, String)>,
    now: u64,
) -> Result<(), Response> {
    for r in card_rows {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        let mut row: CardRow = match serde_json::from_value(payload) {
            Ok(row) => row,
            Err(e) => {
                return Err(Response::invalid_params(format!("card '{id}': invalid row: {e}")));
            }
        };
        if let Some(ref rec_id) = row.record_id {
            if let Some((kind, env)) = imported_history.get(rec_id) {
                let is_verified = match kind.as_str() {
                    "request" => transaction::verify_request(env, now).verified,
                    "quote" => transaction::verify_quote(env, now).verified,
                    "agreement-receipt" => transaction::verify_agreement_receipt(env, now).verified,
                    "booking-progress" => booking::verify_booking_progress(env, now).verified,
                    "payment-request" => payment::verify_payment_request(env, now).verified,
                    "payment-acknowledgement" => {
                        payment::verify_payment_acknowledgement(env, now).verified
                    }
                    "fulfilment-receipt" => {
                        fulfilment::verify_fulfilment_receipt(env, now).verified
                    }
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
            return Err(Response::internal_error(e));
        }
    }
    Ok(())
}
