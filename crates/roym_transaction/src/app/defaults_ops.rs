//! Saved deal defaults for the node's owner.

use serde::{Deserialize, Serialize};
use syneroym_roym_core::{
    deal::{self, LocalText, check_record_text},
    envelope::Response,
    method::Reply,
    transaction::{MAX_QUOTE_LIFETIME_SECS, MIN_QUOTE_LIFETIME_SECS},
};

use crate::app::{AppHost, DEAL_DEFAULTS, ensure_collections, get_row, put_row, refusal::refusal};

pub const DEFAULT_ROW_ID: &str = "defaults";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DealDefaults {
    pub cancellation_terms: LocalText,
    pub refund_terms: LocalText,
    pub dispute_path: LocalText,
    pub quote_valid_secs: u64,
}

pub fn validate_defaults(d: &DealDefaults) -> Result<(), Response> {
    if !(MIN_QUOTE_LIFETIME_SECS..=MAX_QUOTE_LIFETIME_SECS).contains(&d.quote_valid_secs) {
        return Err(refusal("bad-deal-defaults"));
    }
    d.cancellation_terms
        .validate("cancellation_terms", deal::MAX_TERMS_TEXT_LEN)
        .map_err(|_| refusal("bad-deal-defaults"))?;
    d.refund_terms
        .validate("refund_terms", deal::MAX_TERMS_TEXT_LEN)
        .map_err(|_| refusal("bad-deal-defaults"))?;
    d.dispute_path
        .validate("dispute_path", deal::MAX_TERMS_TEXT_LEN)
        .map_err(|_| refusal("bad-deal-defaults"))?;

    check_record_text([&d.cancellation_terms, &d.refund_terms, &d.dispute_path].into_iter())
        .map_err(|_| refusal("bad-deal-defaults"))?;

    Ok(())
}

pub async fn get<H: AppHost>(
    host: &H,
    _input: syneroym_roym_core::method::NoParams,
) -> Reply<Option<DealDefaults>> {
    ensure_collections(host).await.map_err(Response::internal_error)?;
    match get_row(host, DEAL_DEFAULTS, DEFAULT_ROW_ID).await {
        Ok(v) => Ok(v),
        Err(e) => Err(Response::internal_error(e)),
    }
}

pub async fn set<H: AppHost>(host: &H, input: DealDefaults) -> Reply<DealDefaults> {
    ensure_collections(host).await.map_err(Response::internal_error)?;
    validate_defaults(&input)?;
    put_row(host, DEAL_DEFAULTS, DEFAULT_ROW_ID, &input).await.map_err(Response::internal_error)?;
    Ok(input)
}
