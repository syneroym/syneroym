//! Issuing a quote from finished terms.
//!
//! One signing path for every quote: `quote.set` prepares the terms and calls
//! this, and the owner's as-listed answer builds the terms and calls it too.
//! It never runs an extension hook, so a caller that already ran one does not
//! run it twice.

use syneroym_app_host::{AppHost, types::signing::Principal};
use syneroym_roym_core::{
    deal::DealTerms,
    envelope::Response,
    method::Reply,
    progress::FormedAs,
    record::RECORD_QUOTE,
    transaction::{self, Answers, QUOTE_VERSION, QuotePayload},
};

use super::{
    QuoteSetOutput, QuoteSigningInput, build_quote_pointer, sign_and_store_quote,
    validate_and_conceal_terms,
};
use crate::app::{OutgoingCard, QUOTES, put_row, send_card_and_file};

/// Everything one issued quote needs. The caller already resolved the
/// principal, the conversation and the sequence.
pub(crate) struct IssueQuote<'a> {
    pub(crate) principal: Principal,
    pub(crate) owner: &'a str,
    pub(crate) enquiry_record_id: &'a str,
    pub(crate) quote_id: &'a str,
    pub(crate) conversation: &'a str,
    pub(crate) counterparty_did: &'a str,
    pub(crate) terms: DealTerms,
    pub(crate) sequence: u32,
    pub(crate) supersedes: Option<String>,
    pub(crate) next_count: u64,
    /// How the author answered the enquiry. `None` for a normal quote.
    pub(crate) formed_as: Option<FormedAs>,
    pub(crate) expires_at_secs: u64,
    pub(crate) now: u64,
}

pub(crate) async fn issue_quote<H: AppHost>(
    host: &H,
    quote: IssueQuote<'_>,
) -> Reply<QuoteSetOutput> {
    let concealed = validate_and_conceal_terms(host, &quote.terms).await?;
    let payload = QuotePayload {
        quote_id: quote.quote_id.to_string(),
        conversation: quote.conversation.to_string(),
        sequence: quote.sequence,
        answers: Answers::Enquiry { record_id: quote.enquiry_record_id.to_string() },
        counterparty_did: quote.counterparty_did.to_string(),
        terms: concealed.signed,
    };
    if let Err(e) = payload.validate() {
        let resp = Response::invalid_params(e.to_string());
        let resp = match &e {
            transaction::TransactionError::Deal(de) => {
                resp.with_data(serde_json::json!({ "code": de.code() }))
            }
            _ => resp,
        };
        return Err(resp);
    }

    let signed = sign_and_store_quote(
        host,
        &payload,
        QuoteSigningInput {
            principal: quote.principal,
            owner: quote.owner,
            expires_at_secs: quote.expires_at_secs,
            supersedes: quote.supersedes,
            disclosures: &concealed.disclosures,
            now: quote.now,
        },
    )
    .await?;

    let pointer = build_quote_pointer(
        &payload,
        &signed,
        quote.owner,
        quote.next_count,
        quote.now,
        quote.formed_as,
    );
    if let Err(e) = put_row(host, QUOTES, quote.quote_id, &pointer).await {
        return Err(Response::internal_error(e));
    }

    let card = OutgoingCard {
        card_type: RECORD_QUOTE,
        version: QUOTE_VERSION,
        envelope: &signed.envelope_json,
        send: &concealed.disclosures,
        view: &concealed.disclosures,
    };
    let (message_id, state, send_error) =
        send_card_and_file(host, quote.conversation, &card, quote.now, Some(quote.next_count))
            .await;

    Ok(QuoteSetOutput {
        quote_id: quote.quote_id.to_string(),
        record_id: signed.record_id,
        version_count: quote.next_count,
        expires_at_secs: quote.expires_at_secs,
        message_id,
        state,
        send_error: send_error.map(|e| e.to_string()),
    })
}
