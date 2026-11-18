//! Local storage and operations for selective disclosures.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use syneroym_app_host::{AppDataLayer, AppHost, AppSigning, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    deal::{DealTerms, sd::reveal_terms_strict},
    disclosure::{Salt, decode_disclosure, disclosure_digest},
    envelope::Response,
    transaction::{self, QuotePayload, RecordVerdict},
};

use super::get_bytes;
use crate::app::QUOTE_HISTORY;

pub const DISCLOSURES: &str = "disclosures";

/// Row stored in the `disclosures` collection for a signed record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisclosureRow {
    pub record_id: String,
    pub record_type: String,
    pub disclosures: Vec<String>,
    pub stored_at_secs: u64,
}

/// Stores the disclosures for a signed record before taking a fence or writing
/// history.
///
/// If a row already exists, the stored set is kept.
pub(crate) async fn store<H: AppHost>(
    host: &H,
    record_id: &str,
    record_type: &str,
    disclosures: &[String],
    now: u64,
) -> Result<(), String> {
    let row = DisclosureRow {
        record_id: record_id.to_string(),
        record_type: record_type.to_string(),
        disclosures: disclosures.to_vec(),
        stored_at_secs: now,
    };
    let bytes = serde_json::to_vec(&row).map_err(|e| e.to_string())?;
    let write = RecordWriteValue { id: row.record_id, payload: bytes };
    match AppDataLayer::create(host, DISCLOSURES.to_string(), vec![write]).await {
        Ok(None) => Ok(()),
        Ok(Some(_collision)) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Loads the stored disclosures for a signed record.
pub(crate) async fn load<H: AppHost>(
    host: &H,
    record_id: &str,
) -> Result<Option<Vec<String>>, String> {
    let bytes = match get_bytes(host, DISCLOSURES, record_id).await? {
        Some(b) => b,
        None => return Ok(None),
    };
    let row: DisclosureRow = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(Some(row.disclosures))
}

/// Fetches `count` fresh salts from the host.
pub(crate) async fn fresh_salts<H: AppHost>(host: &H, count: u32) -> Result<Vec<Salt>, Response> {
    let slices = match AppSigning::fresh_salts(host, count).await {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let mut salts = Vec::with_capacity(slices.len());
    for s in slices {
        let salt = Salt::try_from_slice(&s)
            .map_err(|e| Response::internal_error(format!("invalid salt from host: {e:?}")))?;
        salts.push(salt);
    }
    Ok(salts)
}

/// A stored quote verified against its envelope and its stored disclosures.
#[derive(Debug, Clone)]
pub(crate) struct HeldQuote {
    pub(crate) verdict: RecordVerdict<QuotePayload>,
    pub(crate) signed: QuotePayload,
    pub(crate) terms: DealTerms,
    pub(crate) disclosures: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum LoadQuoteError {
    Missing,
    Storage(String),
    Invalid(String),
}

/// Loads a stored quote, verifies its envelope, and strictly reveals its terms.
pub(crate) async fn load_quote<H: AppHost>(
    host: &H,
    record_id: &str,
    now: u64,
) -> Result<HeldQuote, LoadQuoteError> {
    let envelope_bytes = match get_bytes(host, QUOTE_HISTORY, record_id).await {
        Ok(Some(b)) => b,
        Ok(None) => return Err(LoadQuoteError::Missing),
        Err(e) => return Err(LoadQuoteError::Storage(e)),
    };
    let envelope_str = match String::from_utf8(envelope_bytes) {
        Ok(s) => s,
        Err(e) => return Err(LoadQuoteError::Invalid(e.to_string())),
    };
    let verdict = transaction::verify_quote(&envelope_str, now);
    if !verdict.verified {
        return Err(LoadQuoteError::Invalid(
            verdict.reason.unwrap_or_else(|| "quote does not verify".to_string()),
        ));
    }
    let signed = match verdict.payload.clone() {
        Some(p) => p,
        None => return Err(LoadQuoteError::Invalid("missing quote payload".to_string())),
    };
    let disclosures = match load(host, record_id).await {
        Ok(Some(d)) => d,
        Ok(None) => return Err(LoadQuoteError::Invalid("disclosures missing".to_string())),
        Err(e) => return Err(LoadQuoteError::Storage(e)),
    };
    let terms = match reveal_terms_strict(&signed.terms, &disclosures) {
        Ok(t) => t,
        Err(e) => {
            return Err(LoadQuoteError::Invalid(format!(
                "quote disclosures invalid: {}",
                e.code()
            )));
        }
    };
    Ok(HeldQuote { verdict, signed, terms, disclosures })
}

/// Reconstructs the plain JSON view of a payload by replacing `_sd` with the
/// disclosed fields.
pub(crate) fn plain_view(payload: &Value, disclosures: &[String]) -> Value {
    let mut map = HashMap::with_capacity(disclosures.len());
    for d in disclosures {
        if let Ok(decoded) = decode_disclosure(d) {
            let digest = disclosure_digest(d);
            map.insert(digest, (decoded.name, decoded.value));
        }
    }

    fn walk(val: &mut Value, map: &HashMap<String, (String, Value)>) {
        match val {
            Value::Object(obj) => {
                for v in obj.values_mut() {
                    walk(v, map);
                }
                if let Some(Value::Array(sd_arr)) = obj.remove("_sd") {
                    for digest_val in sd_arr {
                        if let Value::String(d) = digest_val
                            && let Some((name, val)) = map.get(&d)
                            && !val.is_null()
                        {
                            obj.insert(name.clone(), val.clone());
                        }
                    }
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    walk(item, map);
                }
            }
            _ => {}
        }
    }

    let mut out = payload.clone();
    walk(&mut out, &map);
    out
}
