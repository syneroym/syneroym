//! Service tests for signed deal-decline: idempotency and refusal of invalid
//! signers or records.

pub mod filing;

use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    card::{self, CARD_CONTENT_TYPE},
    deal::{Commitment, CommitmentKind, DealTerms, Details, ExtensionRef, LocalText, ResourceKind},
    lang::LangTag,
    listing::derive_slot_id,
    progress::AgreementState,
    record::{Envelope, RECORD_DEAL_DECLINE, RECORD_ENQUIRY, RECORD_QUOTE, RecordDraft},
    testing::sd::{PlainQuote, conceal_quote},
    transaction::{
        Answers, DEAL_DECLINE_VERSION, DealDeclinePayload, DealRole, ENQUIRY_VERSION,
        EnquiryPayload, QUOTE_VERSION, QuotePayload, ReceiptHalf, TimeWindow, derive_enquiry_id,
        derive_quote_id, verify_deal_decline,
    },
};

use crate::app::{
    AGREEMENTS, AgreementRow, CARDS, CardRow, ENQUIRIES, ENQUIRY_HISTORY, LEDGER, QUOTE_HISTORY,
    QUOTES, RecordPointerRow,
    decline_ops::{self, DealDeclineInput},
    disclosures, get_row, put_bytes, put_row,
    sync::{self, SyncInput},
    tests::{TestFixture, fixture},
};

fn sample_commitment(id: &str, listing_id: &str, start_secs: u64, end_secs: u64) -> Commitment {
    let slot_id = derive_slot_id(listing_id, start_secs, end_secs).expect("slot id");
    Commitment {
        commitment_id: id.to_string(),
        giver: DealRole::QuoteAuthor,
        resource_kind: ResourceKind::ServiceInSlot,
        kind: CommitmentKind::Fulfilment,
        extension: Some(ExtensionRef { id: "scheduling".to_string(), version: 1 }),
        after: vec![],
        description: LocalText::plain(
            LangTag::parse_canonical("en").expect("lang"),
            "Slot service",
        ),
        declared_value_minor: 5000,
        details: Some(Details {
            extension: "scheduling".to_string(),
            schema: "scheduling/slot@1".to_string(),
            data: json!({ "listing_id": listing_id, "slot_id": slot_id }),
            extra: vec![],
        }),
        window: Some(TimeWindow { earliest_secs: start_secs, latest_secs: end_secs }),
        location: None,
        payee: None,
        payment_methods: None,
        tax_minor: None,
        fees_minor: None,
        eep: None,
    }
}

fn make_quote_envelope(
    author: &Identity,
    cp_did: &str,
    now: u64,
) -> (String, String, QuotePayload, DealTerms, Vec<String>) {
    let author_did = substrate::derive_did_key(&author.public_key());
    let quote_id = derive_quote_id("conv_1", &author_did, 1).expect("quote id");
    let terms = DealTerms {
        currency: "EUR".to_string(),
        commitments: vec![sample_commitment("c1", "list_1", now + 1000, now + 4000)],
        home_exchange: None,
        listing_id: None,
        scope: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Scope"),
        cancellation_terms: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "None"),
        refund_terms: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "None"),
        dispute_path: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "None"),
        quote_expires_at_secs: now + 3600,
    };
    let plain = PlainQuote {
        quote_id,
        conversation: "conv_1".to_string(),
        sequence: 1,
        answers: Answers::Enquiry { record_id: "rec_enq_1".to_string() },
        counterparty_did: cp_did.to_string(),
        terms: terms.clone(),
    };
    let (payload, disclosures) = conceal_quote(plain, 1);
    let draft = RecordDraft {
        version: QUOTE_VERSION,
        record_type: "quote".to_string(),
        subject: "conv_1".to_string(),
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: Some(now + 3600),
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, author_did, None, now).expect("env");
    let sig = author.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("sig");
    let rec_id = env.record_id().expect("rec id");
    (env.to_json().expect("json"), rec_id, payload, terms, disclosures)
}

fn make_decline_envelope(
    signer: &Identity,
    declines: &str,
    deal: &str,
    conv: &str,
    now: u64,
) -> (String, String) {
    let signer_did = substrate::derive_did_key(&signer.public_key());
    let payload = DealDeclinePayload {
        declines: declines.to_string(),
        deal: deal.to_string(),
        conversation: conv.to_string(),
        reason: None,
    };
    let draft = RecordDraft {
        version: DEAL_DECLINE_VERSION,
        record_type: RECORD_DEAL_DECLINE.to_string(),
        subject: declines.to_string(),
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, signer_did, None, now).expect("env");
    let sig = signer.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("sig");
    let rec_id = env.record_id().expect("rec id");
    (env.to_json().expect("json"), rec_id)
}

fn make_enquiry_envelope(
    author: &Identity,
    conv: &str,
    now: u64,
) -> (String, String, EnquiryPayload) {
    let author_did = substrate::derive_did_key(&author.public_key());
    let enquiry_id = derive_enquiry_id(conv, &author_did, 1).expect("enquiry id");
    let payload = EnquiryPayload {
        enquiry_id: enquiry_id.clone(),
        conversation: conv.to_string(),
        sequence: 1,
        listing_id: None,
        categories: vec![],
        description: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Need repair"),
        area: None,
        window: None,
        data_use_notice: "notice".to_string(),
    };
    let draft = RecordDraft {
        version: ENQUIRY_VERSION,
        record_type: RECORD_ENQUIRY.to_string(),
        subject: enquiry_id,
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, author_did, None, now).expect("env");
    let sig = author.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("sig");
    let rec_id = env.record_id().expect("rec id");
    (env.to_json().expect("json"), rec_id, payload)
}

#[tokio::test]
async fn repeated_decline_returns_the_same_record() {
    let f = fixture();
    let now = f.host.now();
    let author = Identity::generate().expect("author");
    let author_did = substrate::derive_did_key(&author.public_key());

    // f is counterparty
    let (q_json, q_rec_id, quote, _terms, disc) = make_quote_envelope(&author, &f.master_did, now);
    put_bytes(&f.host, QUOTE_HISTORY, &q_rec_id, q_json.as_bytes()).await.unwrap();
    disclosures::store(&f.host, &q_rec_id, RECORD_QUOTE, &disc, now).await.unwrap();

    let ptr = RecordPointerRow {
        envelope: q_json,
        record_id: q_rec_id.clone(),
        id: quote.quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        issuer: author_did,
        mine: false,
        updated_at_secs: now,
        version_count: 1,
        issued_at_secs: now,
        request_record_id: None,
        counterparty_did: Some(f.master_did.clone()),
        declined_at_secs: None,
        decline_note: None,
    };
    put_row(&f.host, QUOTES, &quote.quote_id, &ptr).await.unwrap();

    let out1 = decline_ops::decline_at(
        &f.host,
        DealDeclineInput {
            declines: q_rec_id.clone(),
            note: Some("not today".to_string()),
            lang: None,
        },
        now,
    )
    .await
    .unwrap();
    assert!(out1.declined);

    let out2 = decline_ops::decline_at(
        &f.host,
        DealDeclineInput {
            declines: q_rec_id.clone(),
            note: Some("not today".to_string()),
            lang: None,
        },
        now + 10,
    )
    .await
    .unwrap();
    assert_eq!(out1.record_id, out2.record_id);
    assert_eq!(out1.declines, out2.declines);
    assert_eq!(out1.deal, out2.deal);
}

#[tokio::test]
async fn decline_with_hindi_lang() {
    let f = fixture();
    let now = f.host.now();
    let cp = Identity::generate().expect("cp");
    let (q_json, q_rec_id, quote, _terms, disc) = make_quote_envelope(&cp, &f.master_did, now);
    put_bytes(&f.host, QUOTE_HISTORY, &q_rec_id, q_json.as_bytes()).await.unwrap();
    disclosures::store(&f.host, &q_rec_id, RECORD_QUOTE, &disc, now).await.unwrap();

    let ptr = RecordPointerRow {
        envelope: q_json,
        record_id: q_rec_id.clone(),
        id: quote.quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        issuer: substrate::derive_did_key(&cp.public_key()),
        mine: false,
        updated_at_secs: now,
        version_count: 1,
        issued_at_secs: now,
        request_record_id: None,
        counterparty_did: Some(f.master_did.clone()),
        declined_at_secs: None,
        decline_note: None,
    };
    put_row(&f.host, QUOTES, &quote.quote_id, &ptr).await.unwrap();

    // Invalid lang tag fails
    let err = decline_ops::decline_at(
        &f.host,
        DealDeclineInput {
            declines: q_rec_id.clone(),
            note: Some("अस्वीकार".to_string()),
            lang: Some("invalid_lang_tag_!@#".to_string()),
        },
        now,
    )
    .await
    .unwrap_err();
    assert!(err.error.as_ref().unwrap().message.contains("invalid lang tag"));

    // Valid Hindi lang tag succeeds
    let out = decline_ops::decline_at(
        &f.host,
        DealDeclineInput {
            declines: q_rec_id.clone(),
            note: Some("अस्वीकार".to_string()),
            lang: Some("hi".to_string()),
        },
        now,
    )
    .await
    .unwrap();
    assert!(out.declined);

    let dec_row: decline_ops::DeclineRow =
        get_row(&f.host, LEDGER, &decline_ops::decline_id(&quote.quote_id)).await.unwrap().unwrap();
    let v = verify_deal_decline(&dec_row.envelope, now);
    assert!(v.verified);
    let payload = v.payload.unwrap();
    let reason = payload.reason.unwrap();
    assert_eq!(reason.lang.as_str(), "hi");
    assert_eq!(reason.text, "अस्वीकार");
}

#[tokio::test]
async fn decline_resends_if_message_id_is_empty() {
    let f = fixture();
    let now = f.host.now();
    let cp = Identity::generate().expect("cp");
    let (q_json, q_rec_id, quote, _terms, disc) = make_quote_envelope(&cp, &f.master_did, now);
    put_bytes(&f.host, QUOTE_HISTORY, &q_rec_id, q_json.as_bytes()).await.unwrap();
    disclosures::store(&f.host, &q_rec_id, RECORD_QUOTE, &disc, now).await.unwrap();

    let ptr = RecordPointerRow {
        envelope: q_json,
        record_id: q_rec_id.clone(),
        id: quote.quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        issuer: substrate::derive_did_key(&cp.public_key()),
        mine: false,
        updated_at_secs: now,
        version_count: 1,
        issued_at_secs: now,
        request_record_id: None,
        counterparty_did: Some(f.master_did.clone()),
        declined_at_secs: None,
        decline_note: None,
    };
    put_row(&f.host, QUOTES, &quote.quote_id, &ptr).await.unwrap();

    // Seed existing fence row with message_id: None
    let dec_key = decline_ops::decline_id(&quote.quote_id);
    let (dec_env, dec_rec_id) =
        make_decline_envelope(&f.master, &q_rec_id, &quote.quote_id, "conv_1", now);
    put_row(
        &f.host,
        LEDGER,
        &dec_key,
        &decline_ops::DeclineRow {
            kind: "decline".to_string(),
            deal: quote.quote_id.clone(),
            declines: q_rec_id.clone(),
            record_id: dec_rec_id,
            envelope: dec_env,
            created_at_secs: now,
            message_id: None,
        },
    )
    .await
    .unwrap();

    // Calling decline_at should resend and populate message_id
    let out = decline_ops::decline_at(
        &f.host,
        DealDeclineInput {
            declines: q_rec_id.clone(),
            note: Some("resend test".to_string()),
            lang: None,
        },
        now + 10,
    )
    .await
    .unwrap();
    assert!(out.declined);

    let updated: decline_ops::DeclineRow =
        get_row(&f.host, LEDGER, &dec_key).await.unwrap().unwrap();
    assert!(updated.message_id.is_some());
}

#[tokio::test]
async fn cannot_decline_enquiry_with_accepted_quote() {
    let f = fixture();
    let now = f.host.now();
    let (enq_json, enq_rec_id, enq_payload) = make_enquiry_envelope(&f.master, "conv_1", now);
    let enquiry_id = enq_payload.enquiry_id.clone();
    put_bytes(&f.host, ENQUIRY_HISTORY, &enq_rec_id, enq_json.as_bytes()).await.unwrap();

    let enq_ptr = RecordPointerRow {
        envelope: enq_json,
        record_id: enq_rec_id.clone(),
        id: enquiry_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        issuer: f.master_did.clone(),
        mine: true,
        updated_at_secs: now,
        version_count: 1,
        issued_at_secs: now,
        request_record_id: None,
        counterparty_did: None,
        declined_at_secs: None,
        decline_note: None,
    };
    put_row(&f.host, ENQUIRIES, &enquiry_id, &enq_ptr).await.unwrap();

    // Create quote answering enq_rec_id
    let cp = Identity::generate().expect("cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());
    let (q_json, q_rec_id, quote, terms, disc) = make_quote_envelope(&f.master, &cp_did, now);
    put_bytes(&f.host, QUOTE_HISTORY, &q_rec_id, q_json.as_bytes()).await.unwrap();
    disclosures::store(&f.host, &q_rec_id, RECORD_QUOTE, &disc, now).await.unwrap();

    let q_ptr = RecordPointerRow {
        envelope: q_json,
        record_id: q_rec_id.clone(),
        id: quote.quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        issuer: f.master_did.clone(),
        mine: true,
        updated_at_secs: now,
        version_count: 1,
        issued_at_secs: now,
        request_record_id: Some(enq_rec_id.clone()),
        counterparty_did: Some(cp_did.clone()),
        declined_at_secs: None,
        decline_note: None,
    };
    put_row(&f.host, QUOTES, &quote.quote_id, &q_ptr).await.unwrap();

    // Create unrefused counterparty half
    let agr_row = AgreementRow {
        quote_record_id: q_rec_id.clone(),
        quote_id: quote.quote_id.clone(),
        conversation: "conv_1".to_string(),
        counterparty_did: cp_did.clone(),
        quote_author_did: f.master_did.clone(),
        terms: terms.clone(),
        agreed_at_secs: None,
        counterparty: Some(ReceiptHalf {
            record_id: "rec_cp_half_enq".to_string(),
            envelope: "{}".to_string(),
            issuer: cp_did.clone(),
            issued_at_secs: now,
        }),
        quote_author: None,
        state: None,
        state_reason: None,
        cancel_reason: None,
        seq: None,
        commitments: vec![],
        updated_at_secs: now,
    };
    put_row(&f.host, AGREEMENTS, &q_rec_id, &agr_row).await.unwrap();

    // has_unrefused_counterparty_half_for_enquiry returns true
    assert!(
        decline_ops::has_unrefused_counterparty_half_for_enquiry(&f.host, &enquiry_id)
            .await
            .unwrap()
    );

    // Declining enquiry fails
    let err = decline_ops::decline_at(
        &f.host,
        DealDeclineInput {
            declines: enq_rec_id,
            note: Some("cancel enquiry".to_string()),
            lang: None,
        },
        now + 10,
    )
    .await
    .unwrap_err();
    assert!(
        err.error
            .as_ref()
            .unwrap()
            .message
            .contains("cannot decline an enquiry with an accepted quote")
    );
}

#[tokio::test]
async fn counterparty_can_decline_quote_v2_after_v1_refused() {
    let f = fixture();
    let now = f.host.now();
    let cp = Identity::generate().expect("cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());

    // Quote v1 and v2 have the same quote_id
    let (q1_json, q1_rec_id, q1_payload, q1_terms, q1_disc) =
        make_quote_envelope(&cp, &f.master_did, now);
    let quote_id = q1_payload.quote_id.clone();
    put_bytes(&f.host, QUOTE_HISTORY, &q1_rec_id, q1_json.as_bytes()).await.unwrap();
    disclosures::store(&f.host, &q1_rec_id, RECORD_QUOTE, &q1_disc, now).await.unwrap();

    let (mut env2, bytes2) = Envelope::unsigned(
        RecordDraft {
            version: QUOTE_VERSION,
            record_type: RECORD_QUOTE.to_string(),
            subject: quote_id.clone(),
            payload: serde_json::to_value(&q1_payload).unwrap(),
            expires_at_secs: Some(now + 3600),
            supersedes: Some(q1_rec_id.clone()),
        },
        cp_did.clone(),
        None,
        now + 10,
    )
    .unwrap();
    let sig2 = cp.sign(&bytes2).to_bytes();
    env2.attach_signature(z32::encode(&sig2)).unwrap();
    let q2_rec_id = env2.record_id().unwrap();
    let q2_json = env2.to_json().unwrap();
    put_bytes(&f.host, QUOTE_HISTORY, &q2_rec_id, q2_json.as_bytes()).await.unwrap();
    disclosures::store(&f.host, &q2_rec_id, RECORD_QUOTE, &q1_disc, now + 10).await.unwrap();

    let ptr2 = RecordPointerRow {
        envelope: q2_json,
        record_id: q2_rec_id.clone(),
        id: quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 2,
        issuer: cp_did.clone(),
        mine: false,
        updated_at_secs: now + 10,
        version_count: 2,
        issued_at_secs: now + 10,
        request_record_id: None,
        counterparty_did: Some(f.master_did.clone()),
        declined_at_secs: None,
        decline_note: None,
    };
    put_row(&f.host, QUOTES, &quote_id, &ptr2).await.unwrap();

    // v1 was accepted by counterparty, but writer refused it with quote-superseded
    // snapshot
    let agr_v1 = AgreementRow {
        quote_record_id: q1_rec_id.clone(),
        quote_id: quote_id.clone(),
        conversation: "conv_1".to_string(),
        counterparty_did: f.master_did.clone(),
        quote_author_did: cp_did.clone(),
        terms: q1_terms.clone(),
        agreed_at_secs: None,
        counterparty: Some(ReceiptHalf {
            record_id: "rec_cp_half_v1".to_string(),
            envelope: "{}".to_string(),
            issuer: f.master_did.clone(),
            issued_at_secs: now,
        }),
        quote_author: None,
        state: Some(AgreementState::NotFormed),
        state_reason: Some("quote-superseded".to_string()),
        cancel_reason: None,
        seq: Some(1),
        commitments: vec![],
        updated_at_secs: now + 5,
    };
    put_row(&f.host, AGREEMENTS, &q1_rec_id, &agr_v1).await.unwrap();

    // has_unrefused_counterparty_half must answer false
    assert!(!decline_ops::has_unrefused_counterparty_half(&f.host, &quote_id).await.unwrap());

    // Counterparty can decline v2
    let out = decline_ops::decline_at(
        &f.host,
        DealDeclineInput { declines: q2_rec_id, note: Some("decline v2".to_string()), lang: None },
        now + 20,
    )
    .await
    .unwrap();
    assert!(out.declined);
}
