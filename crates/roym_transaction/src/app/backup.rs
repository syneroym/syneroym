//! Transaction data backup: export and import.

use std::collections::{BTreeMap, HashMap};

use serde_json::{Value, json};
use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    backup::{
        BUNDLE_VERSION, Bundle, BundleManifest, SECTION_AGREEMENTS, SECTION_CARDS, SECTION_QUOTES,
        SECTION_REQUESTS,
    },
    clock,
    envelope::{Request, Response},
    signing,
    transaction::{self, QuotePayload, Role},
};

use super::{
    AGREEMENTS, AgreementRow, CARDS, CardRow, QUOTE_HISTORY, QUOTES, REQUEST_HISTORY, REQUESTS,
    RecordPointerRow, SCHEMA_VERSION, collect, ensure_collections, get_bytes, put_bytes, put_row,
};

/// Exports transaction data: requests, quotes, agreements, and cards.
///
/// Note: `request_history` and `quote_history` are not exported as their
/// own sections because every envelope in them is reachable from a
/// `requests`/`quotes` pointer row or an `agreements` half. On import,
/// they are re-populated from the imported rows.
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

pub(crate) async fn import<H: AppHost>(host: &H, req: &Request) -> Response {
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
