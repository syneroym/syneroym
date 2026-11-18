//! Shared test builders and helpers for selective disclosure.

use syneroym_signed_record::{Salt, seeded_salt};

use crate::{
    card,
    deal::{Answers, DealTerms, LocalText, SettlementRole},
    disclosure::{FULFILMENT_SET, PAYMENT_ACK_SET, PAYMENT_REQUEST_SET, conceal_hidden},
    fulfilment::{FulfilmentHidden, FulfilmentReceiptPayload},
    payment::{
        PaymentAckHidden, PaymentAcknowledgementPayload, PaymentRequestHidden,
        PaymentRequestPayload,
    },
    transaction::QuotePayload,
};

/// Salts 0, 1, 2, … from `seed`: `(0..).map(|n| seeded_salt(seed, n))`.
pub fn seeded_salts(seed: u64) -> impl Iterator<Item = Salt> {
    (0..).map(move |n| seeded_salt(seed, n))
}

/// Serializes a card body with disclosures, expecting success.
pub fn deal_card_body(card_type: &str, envelope: &str, disclosures: &[String]) -> String {
    card::card_body(card_type, 1, envelope, disclosures).expect("serialize deal card body")
}

#[derive(Debug, Clone)]
pub struct PlainQuote {
    pub quote_id: String,
    pub conversation: String,
    pub sequence: u32,
    pub answers: Answers,
    pub counterparty_did: String,
    pub terms: DealTerms,
}

#[derive(Debug, Clone)]
pub struct PlainPaymentRequest {
    pub agreement: String,
    pub conversation: String,
    pub commitment_id: String,
    pub quote_author_did: String,
    pub counterparty_did: String,
    pub note: Option<LocalText>,
}

#[derive(Debug, Clone)]
pub struct PlainPaymentAck {
    pub agreement: String,
    pub conversation: String,
    pub commitment_id: String,
    pub quote_author_did: String,
    pub counterparty_did: String,
    pub role: SettlementRole,
    pub observed_at_secs: u64,
    pub method: Option<String>,
    pub reference: Option<String>,
    pub settlement_note: Option<LocalText>,
}

#[derive(Debug, Clone)]
pub struct PlainFulfilment {
    pub agreement: String,
    pub conversation: String,
    pub commitment_id: String,
    pub counterparty_did: String,
    pub quote_author_did: String,
    pub role: SettlementRole,
    pub settlement_note: Option<LocalText>,
}

pub fn conceal_quote(payload: PlainQuote, seed: u64) -> (QuotePayload, Vec<String>) {
    let mut salts = seeded_salts(seed);
    let concealed = payload.terms.conceal(&mut salts).expect("conceal quote terms");
    (
        QuotePayload {
            quote_id: payload.quote_id,
            conversation: payload.conversation,
            sequence: payload.sequence,
            answers: payload.answers,
            counterparty_did: payload.counterparty_did,
            terms: concealed.signed,
        },
        concealed.disclosures,
    )
}

pub fn conceal_payment_request(
    p: PlainPaymentRequest,
    seed: u64,
) -> (PaymentRequestPayload, Vec<String>) {
    let mut salts = seeded_salts(seed);
    let hidden = PaymentRequestHidden { note: p.note };
    let (sd, disclosures) =
        conceal_hidden(&hidden, &PAYMENT_REQUEST_SET, &mut salts).expect("conceal payment request");
    (
        PaymentRequestPayload {
            agreement: p.agreement,
            conversation: p.conversation,
            commitment_id: p.commitment_id,
            quote_author_did: p.quote_author_did,
            counterparty_did: p.counterparty_did,
            sd,
        },
        disclosures,
    )
}

pub fn conceal_payment_ack(
    p: PlainPaymentAck,
    seed: u64,
) -> (PaymentAcknowledgementPayload, Vec<String>) {
    let mut salts = seeded_salts(seed);
    let hidden = PaymentAckHidden {
        method: p.method,
        reference: p.reference,
        settlement_note: p.settlement_note,
    };
    let (sd, disclosures) =
        conceal_hidden(&hidden, &PAYMENT_ACK_SET, &mut salts).expect("conceal payment ack");
    (
        PaymentAcknowledgementPayload {
            agreement: p.agreement,
            conversation: p.conversation,
            commitment_id: p.commitment_id,
            quote_author_did: p.quote_author_did,
            counterparty_did: p.counterparty_did,
            role: p.role,
            observed_at_secs: p.observed_at_secs,
            sd,
        },
        disclosures,
    )
}

pub fn conceal_fulfilment(
    p: PlainFulfilment,
    seed: u64,
) -> (FulfilmentReceiptPayload, Vec<String>) {
    let mut salts = seeded_salts(seed);
    let hidden = FulfilmentHidden { settlement_note: p.settlement_note };
    let (sd, disclosures) =
        conceal_hidden(&hidden, &FULFILMENT_SET, &mut salts).expect("conceal fulfilment");
    (
        FulfilmentReceiptPayload {
            agreement: p.agreement,
            conversation: p.conversation,
            commitment_id: p.commitment_id,
            counterparty_did: p.counterparty_did,
            quote_author_did: p.quote_author_did,
            role: p.role,
            sd,
        },
        disclosures,
    )
}
