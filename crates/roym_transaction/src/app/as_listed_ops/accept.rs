//! The counterparty's automatic acceptance of a quote that matches the
//! offering it tapped.

use serde::{Deserialize, Serialize};
use serde_json::json;
use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    as_listed::{self, AsListed, MAX_CONSENT_SECS},
    deal::DealTerms,
    envelope::Response,
    listing, paging,
};

use super::super::{
    AS_LISTED, ENQUIRIES, LEDGER, QUOTES, RecordPointerRow,
    agreement_ops::{self, AgreementAcceptInput},
    decline_ops, disclosures,
    enquiry_ops::as_listed::{AsListedRow, AsListedState},
    get_row,
    ledger::LedgerKind,
    put_row,
};

pub(crate) fn auto_accept_id(enquiry_record_id: &str) -> String {
    format!("autoaccept:{enquiry_record_id}")
}

/// The ledger row that makes one tap accept at most one quote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutoAcceptRow {
    pub(crate) kind: LedgerKind,
    pub(crate) enquiry_record_id: String,
    pub(crate) quote_record_id: String,
    pub(crate) created_at_secs: u64,
}

/// What one attempt did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AutoAccept {
    /// The acceptance half was signed and sent.
    Accepted,
    /// Signing is not ready. The row stays waiting, and the next drive tries.
    Waiting,
    /// The quote is shown as a normal one, with this reason.
    Shown(String),
    /// This tap is not waiting for an answer.
    NotWaiting,
}

/// Signs the acceptance half when the quote's terms are exactly the terms the
/// offering builds. Every check runs here, immediately before signing.
pub(crate) async fn auto_accept<H: AppHost>(
    host: &H,
    enquiry_record_id: &str,
    quote_record_id: &str,
    now: u64,
) -> Result<AutoAccept, Response> {
    let Some(row) = get_row::<AsListedRow, _>(host, AS_LISTED, enquiry_record_id)
        .await
        .map_err(Response::internal_error)?
    else {
        return Ok(AutoAccept::NotWaiting);
    };
    if row.state != AsListedState::Waiting {
        return Ok(AutoAccept::NotWaiting);
    }

    let quote = match disclosures::load_quote(host, quote_record_id, now).await {
        Ok(quote) => quote,
        Err(disclosures::LoadQuoteError::Missing) => {
            return Ok(AutoAccept::Shown("not-held".into()));
        }
        Err(disclosures::LoadQuoteError::Storage(e)) => return Err(Response::internal_error(e)),
        Err(disclosures::LoadQuoteError::Invalid(_)) => {
            return Ok(shown(host, &row, "not-disclosed", now).await);
        }
    };
    let issued_at = quote.verdict.issued_at_secs.unwrap_or(now);
    if issued_at > row.valid_until_secs {
        return Ok(shown(host, &row, "consent-expired", now).await);
    }
    if declined(host, enquiry_record_id).await? {
        return Ok(shown(host, &row, "withdrawn", now).await);
    }
    if !latest_version(host, &quote.signed.quote_id, quote_record_id).await? {
        return Ok(shown(host, &row, "superseded", now).await);
    }

    let verdict = listing::verify_envelope(&row.envelope, now);
    let (Some(payload), Some(issuer)) = (verdict.payload, quote.verdict.issuer.clone()) else {
        return Ok(shown(host, &row, "not-complete", now).await);
    };
    let consent = AsListed {
        record_id: row.record_id.clone(),
        slot: row.slot.clone(),
        variant: None,
        valid_until_secs: row.valid_until_secs,
    };
    let built = match as_listed::terms_from_listing(&payload, &issuer, &consent, issued_at) {
        Ok(terms) => terms,
        Err(_) => return Ok(shown(host, &row, "not-complete", now).await),
    };
    if !same_terms(&built, &quote.terms, issued_at, now) {
        return Ok(shown(host, &row, "terms-differ", now).await);
    }

    match claim(host, enquiry_record_id, quote_record_id, now).await? {
        Fence::Lost(other) => return Ok(shown(host, &row, &other, now).await),
        Fence::Held => {}
    }

    let input = AgreementAcceptInput {
        quote_record_id: quote_record_id.to_string(),
        viewed_lang: Some(row.viewed_lang.clone()),
        viewed_via_mt: Some(row.viewed_via_mt),
    };
    match agreement_ops::accept_at(host, input, now).await {
        Ok(_) => {
            mark(host, &row, AsListedState::Accepted, None, Some(quote_record_id), now).await?;
            Ok(AutoAccept::Accepted)
        }
        Err(response) => {
            let code = error_code(&response);
            if is_signing_code(&code) {
                mark(host, &row, AsListedState::Waiting, Some("signing-not-ready"), None, now)
                    .await?;
                return Ok(AutoAccept::Waiting);
            }
            Ok(shown(host, &row, &code, now).await)
        }
    }
}

/// The two values agree as one whole, except the quote's own expiry, which
/// must lie after `now` and at most seven days after the quote was signed.
#[must_use]
pub(crate) fn same_terms(
    built: &DealTerms,
    quote: &DealTerms,
    issued_at_secs: u64,
    now: u64,
) -> bool {
    let expiry = quote.quote_expires_at_secs;
    if expiry <= now || expiry > issued_at_secs.saturating_add(MAX_CONSENT_SECS) {
        return false;
    }
    let mut expected = built.clone();
    expected.quote_expires_at_secs = expiry;
    expected == *quote
}

enum Fence {
    Held,
    Lost(String),
}

async fn claim<H: AppHost>(
    host: &H,
    enquiry_record_id: &str,
    quote_record_id: &str,
    now: u64,
) -> Result<Fence, Response> {
    let row = AutoAcceptRow {
        kind: LedgerKind::AutoAccept,
        enquiry_record_id: enquiry_record_id.to_string(),
        quote_record_id: quote_record_id.to_string(),
        created_at_secs: now,
    };
    let write = RecordWriteValue {
        id: auto_accept_id(enquiry_record_id),
        payload: serde_json::to_vec(&row).map_err(|e| Response::internal_error(e.to_string()))?,
    };
    let exists = AppDataLayer::create(host, LEDGER.to_string(), vec![write])
        .await
        .map_err(|e| Response::internal_error(e.to_string()))?;
    match exists {
        None => Ok(Fence::Held),
        Some(_) => {
            let held: Option<AutoAcceptRow> =
                get_row(host, LEDGER, &auto_accept_id(enquiry_record_id))
                    .await
                    .map_err(Response::internal_error)?;
            match held {
                Some(held) if held.quote_record_id == quote_record_id => Ok(Fence::Held),
                _ => Ok(Fence::Lost("already-accepted".to_string())),
            }
        }
    }
}

async fn shown<H: AppHost>(host: &H, row: &AsListedRow, reason: &str, now: u64) -> AutoAccept {
    let _ = mark(host, row, AsListedState::Shown, Some(reason), None, now).await;
    AutoAccept::Shown(reason.to_string())
}

async fn mark<H: AppHost>(
    host: &H,
    row: &AsListedRow,
    state: AsListedState,
    reason: Option<&str>,
    quote_record_id: Option<&str>,
    now: u64,
) -> Result<(), Response> {
    let mut row = row.clone();
    row.state = state;
    row.reason = reason.map(str::to_string);
    if let Some(quote_record_id) = quote_record_id {
        row.quote_record_id = Some(quote_record_id.to_string());
    }
    row.created_at_secs = now;
    put_row(host, AS_LISTED, &row.enquiry_record_id, &row).await.map_err(Response::internal_error)
}

async fn declined<H: AppHost>(host: &H, enquiry_record_id: &str) -> Result<bool, Response> {
    if let Some(ptr) = get_row::<RecordPointerRow, _>(host, ENQUIRIES, enquiry_record_id)
        .await
        .map_err(Response::internal_error)?
        && ptr.declined_at_secs.is_some()
    {
        return Ok(true);
    }
    get_row::<decline_ops::DeclineRow, _>(host, LEDGER, &decline_ops::decline_id(enquiry_record_id))
        .await
        .map(|held| held.is_some())
        .map_err(Response::internal_error)
}

async fn latest_version<H: AppHost>(
    host: &H,
    quote_id: &str,
    quote_record_id: &str,
) -> Result<bool, Response> {
    Ok(get_row::<RecordPointerRow, _>(host, QUOTES, quote_id)
        .await
        .map_err(Response::internal_error)?
        .is_none_or(|ptr| ptr.record_id == quote_record_id))
}

/// The stable code an `accept_at` refusal carries, never its message. A
/// refusal with no code keeps one word so the row can still be shown.
fn error_code(response: &Response) -> String {
    response
        .error
        .as_ref()
        .and_then(|error| error.data.as_ref())
        .and_then(|data| data.get("code"))
        .and_then(|code| code.as_str())
        .unwrap_or("accept-failed")
        .to_string()
}

fn is_signing_code(code: &str) -> bool {
    matches!(
        code,
        "signing-not-enrolled" | "signing-certificate-expired" | "signing-certificate-stale"
    )
}

/// Accepts every waiting tap that holds a quote and has no fence yet. A
/// failure is a warning, so one tap cannot stop the others.
pub(crate) async fn accept_waiting<H: AppHost>(host: &H, now: u64, warnings: &mut Vec<String>) {
    let rows: Vec<AsListedRow> = match paging::query_all(host, AS_LISTED, None).await {
        Ok(rows) => rows,
        Err(e) => {
            warnings.push(format!("as-listed acceptance scan failed: {e}"));
            return;
        }
    };
    for row in rows {
        if row.state != AsListedState::Waiting {
            continue;
        }
        let quote_record_id = match get_row::<AutoAcceptRow, _>(
            host,
            LEDGER,
            &auto_accept_id(&row.enquiry_record_id),
        )
        .await
        {
            Ok(Some(fence)) => fence.quote_record_id,
            Ok(None) => {
                let filter = json!({ "request_record_id": row.enquiry_record_id }).to_string();
                let quotes: Vec<RecordPointerRow> =
                    match paging::query_all(host, QUOTES, Some(filter)).await {
                        Ok(quotes) => quotes,
                        Err(e) => {
                            warnings.push(format!("as-listed acceptance quote scan failed: {e}"));
                            continue;
                        }
                    };
                let Some(quote) = quotes.into_iter().next() else {
                    continue;
                };
                quote.record_id
            }
            Err(e) => {
                warnings.push(format!("as-listed acceptance fence read failed: {e}"));
                continue;
            }
        };
        let _ = auto_accept(host, &row.enquiry_record_id, &quote_record_id, now).await;
    }
}
