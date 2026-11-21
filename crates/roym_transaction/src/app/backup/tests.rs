mod disclosures;

use std::collections::{BTreeMap, HashMap};

use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    backup::{
        Bundle, ImportInput, SECTION_DISCLOSURES, SECTION_LEDGER, SECTION_QUOTE_HISTORY,
        SECTION_QUOTES,
    },
    deal::{Commitment, CommitmentKind, DealTerms, LocalText, ResourceKind},
    lang::LangTag,
    record::{Envelope, RECORD_DEAL_DECLINE, RecordDraft},
    signing,
    testing::sd::{self, PlainQuote, conceal_quote},
    transaction::{
        Answers, DEAL_DECLINE_VERSION, DealDeclinePayload, DealRole, QUOTE_VERSION, QuotePayload,
        ReceiptHalf, derive_quote_id,
    },
};

use crate::app::{
    AgreementRow, RecordPointerRow, SCHEMA_VERSION, backup,
    backup::{ImportedQuote, QuotesByRecordId, sections::validate_ledger},
    decide::{CountersignRow, DecisionRow, DecisionState, RefusedRow},
    decline_ops::DeclineRow,
    disclosures::DisclosureRow,
    ledger::{LedgerKind, LedgerRow},
    tests::fixture,
};

fn sample_deal_terms() -> DealTerms {
    DealTerms {
        currency: "EUR".to_string(),
        commitments: vec![
            Commitment {
                commitment_id: "c1".to_string(),
                giver: DealRole::QuoteAuthor,
                resource_kind: ResourceKind::GoodsOrService,
                kind: CommitmentKind::Fulfilment,
                extension: None,
                after: vec![],
                description: LocalText::plain(
                    LangTag::parse_canonical("en").expect("lang"),
                    "Service",
                ),
                declared_value_minor: 1000,
                details: None,
                window: None,
                location: None,
                payee: None,
                payment_methods: None,
                tax_minor: None,
                fees_minor: None,
                eep: None,
            },
            Commitment {
                commitment_id: "c2".to_string(),
                giver: DealRole::Counterparty,
                resource_kind: ResourceKind::Currency,
                kind: CommitmentKind::Currency,
                extension: None,
                after: vec!["c1".to_string()],
                description: LocalText::plain(
                    LangTag::parse_canonical("en").expect("lang"),
                    "Currency",
                ),
                declared_value_minor: 1000,
                details: None,
                window: None,
                location: None,
                payee: Some("X".to_string()),
                payment_methods: None,
                tax_minor: None,
                fees_minor: None,
                eep: None,
            },
        ],
        home_exchange: None,
        listing_id: None,
        scope: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Scope"),
        cancellation_terms: LocalText::plain(
            LangTag::parse_canonical("en").expect("lang"),
            "Terms",
        ),
        refund_terms: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Refund"),
        dispute_path: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Dispute"),
        quote_expires_at_secs: 10000,
    }
}

fn sample_quote_context(quote_rec_id: &str) -> QuotesByRecordId {
    let mut quotes = HashMap::new();
    let terms = sample_deal_terms();
    let plain = PlainQuote {
        quote_id: "quote_1".to_string(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        counterparty_did: "did:key:cp".to_string(),
        terms,
        answers: Answers::Enquiry { record_id: "enq_1".to_string() },
    };
    let (payload, _disclosures) = conceal_quote(plain, 1);

    quotes.insert(
        quote_rec_id.to_string(),
        ImportedQuote { quote_author_did: "did:key:author".to_string(), payload },
    );
    quotes
}

fn released_row(agreement: &str) -> serde_json::Value {
    json!({
        "kind": "released",
        "agreement": agreement,
        "created_at_secs": 100,
    })
}

#[test]
fn released_row_is_accepted_for_a_held_agreement() {
    let quotes = sample_quote_context("quote_held");
    let (rows, _) = validate_ledger(
        vec![json!({ "id": "released:quote_held", "payload": released_row("quote_held") })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    )
    .expect("a released marker of a held agreement imports");
    assert_eq!(rows.len(), 1);
}

#[test]
fn released_row_refused_on_unheld_agreement_or_a_row_that_does_not_match_its_id() {
    let quotes = sample_quote_context("quote_held");

    let unheld = validate_ledger(
        vec![json!({ "id": "released:unheld_quote", "payload": released_row("unheld_quote") })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(unheld.unwrap_err().error.unwrap().message.contains("names unheld agreement"));

    let mismatch = validate_ledger(
        vec![json!({ "id": "released:quote_held", "payload": released_row("another") })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(mismatch.unwrap_err().error.unwrap().message.contains("does not match its id"));

    let wrong_kind = validate_ledger(
        vec![json!({
            "id": "released:quote_held",
            "payload": { "kind": "fence", "agreement": "quote_held", "created_at_secs": 100 },
        })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(wrong_kind.unwrap_err().error.unwrap().message.contains("does not match its id"));
}

#[test]
fn decision_row_refused_on_unheld_agreement_or_unknown_state() {
    let quotes = sample_quote_context("quote_held");

    let bad_agr_row = DecisionRow {
        kind: "decision".to_string(),
        state: DecisionState::Agreed,
        agreement: "unheld_quote".to_string(),
        quote_id: "q1".to_string(),
        reason: None,
        created_at_secs: 100,
    };
    let res = validate_ledger(
        vec![json!({ "id": "decision:q1", "payload": bad_agr_row })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(res.is_err());
    assert!(res.unwrap_err().error.unwrap().message.contains("names unheld agreement"));

    let bad_state_json = json!({
        "kind": "decision",
        "state": "invalid_state",
        "agreement": "quote_held",
        "quote_id": "q1",
        "created_at_secs": 100,
    });
    let res2 = validate_ledger(
        vec![json!({ "id": "decision:q1", "payload": bad_state_json })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(res2.is_err());
    assert!(res2.unwrap_err().error.unwrap().message.contains("invalid decision row"));
}

#[test]
fn countersign_row_refused_on_unheld_quote_or_bad_signature() {
    let quotes = sample_quote_context("quote_held");

    let bad_cs_row = CountersignRow {
        kind: "countersign".to_string(),
        agreement: "unheld_quote".to_string(),
        half: ReceiptHalf {
            envelope: "bad_envelope".to_string(),
            record_id: "r1".to_string(),
            issuer: "did:key:author".to_string(),
            issued_at_secs: 100,
        },
        message_id: None,
        created_at_secs: 100,
    };
    let res = validate_ledger(
        vec![json!({ "id": "countersign:unheld_quote", "payload": bad_cs_row })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(res.is_err());
    assert!(res.unwrap_err().error.unwrap().message.contains("countersign does not verify"));
}

#[test]
fn payreq_row_refused_on_non_currency_commitment() {
    let quotes = sample_quote_context("quote_held");

    let bad_payreq = LedgerRow {
        kind: LedgerKind::Fence,
        agreement: "quote_held".to_string(),
        state: None,
        quote_id: None,
        commitment_id: Some("c1".to_string()),
        reason: None,
        half: None,
        created_at_secs: 100,
    };
    let res = validate_ledger(
        vec![json!({ "id": "payreq:quote_held:c1", "payload": bad_payreq })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(res.is_err());
    assert!(
        res.unwrap_err().error.unwrap().message.contains("payreq commitment 'c1' is not currency")
    );
}

#[test]
fn fulfil_row_refused_on_currency_commitment() {
    let quotes = sample_quote_context("quote_held");

    let bad_fulfil = LedgerRow {
        kind: LedgerKind::Fence,
        agreement: "quote_held".to_string(),
        state: None,
        quote_id: None,
        commitment_id: Some("c2".to_string()),
        reason: None,
        half: None,
        created_at_secs: 100,
    };
    let res = validate_ledger(
        vec![json!({ "id": "fulfil:quote_held:c2:giver", "payload": bad_fulfil })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(res.is_err());
    assert!(
        res.unwrap_err()
            .error
            .unwrap()
            .message
            .contains("fulfil commitment 'c2' is not goods-or-service")
    );
}

#[test]
fn hold_and_seat_rows_of_an_older_version_are_an_unknown_ledger_kind() {
    let quotes = sample_quote_context("quote_held");
    for id in ["hold:quote_held:c1", "seat:slot_1:1"] {
        let res = validate_ledger(
            vec![json!({ "id": id, "payload": { "kind": "hold", "agreement": "quote_held" } })],
            &quotes,
            &HashMap::new(),
            &HashMap::new(),
            100,
        );
        assert!(res.unwrap_err().error.unwrap().message.contains("unknown ledger kind"), "{id}");
    }
}

#[test]
fn unknown_ledger_row_kind_is_refused() {
    let quotes = sample_quote_context("quote_held");

    let res = validate_ledger(
        vec![json!({ "id": "unknown_kind:123", "payload": { "foo": "bar" } })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(res.is_err());
    assert!(res.unwrap_err().error.unwrap().message.contains("unknown ledger kind"));
}

fn make_test_decline_envelope(
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

#[test]
fn decline_row_refused_on_payload_mismatch_or_unheld_target() {
    let quotes = sample_quote_context("rec_quote_1");
    let key = Identity::generate().expect("key");
    let (env_json, rec_id) =
        make_test_decline_envelope(&key, "rec_quote_1", "quote_1", "conv_1", 100);

    let valid_row = DeclineRow {
        kind: "decline".to_string(),
        deal: "quote_1".to_string(),
        declines: "rec_quote_1".to_string(),
        envelope: env_json.clone(),
        record_id: rec_id.clone(),
        message_id: None,
        created_at_secs: 100,
    };

    let ok = validate_ledger(
        vec![json!({ "id": "decline:quote_1", "payload": valid_row })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(ok.is_ok());

    let bad_deal_row = DeclineRow {
        kind: "decline".to_string(),
        deal: "quote_1".to_string(),
        declines: "rec_quote_1".to_string(),
        envelope: env_json.clone(),
        record_id: rec_id.clone(),
        message_id: None,
        created_at_secs: 100,
    };
    let err_deal = validate_ledger(
        vec![json!({ "id": "decline:quote_other", "payload": bad_deal_row })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(err_deal.unwrap_err().error.unwrap().message.contains("deal id mismatch"));

    let bad_rec_row = DeclineRow {
        kind: "decline".to_string(),
        deal: "quote_1".to_string(),
        declines: "rec_quote_1".to_string(),
        envelope: env_json.clone(),
        record_id: "rec_other".to_string(),
        message_id: None,
        created_at_secs: 100,
    };
    let err_rec = validate_ledger(
        vec![json!({ "id": "decline:quote_1", "payload": bad_rec_row })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(err_rec.unwrap_err().error.unwrap().message.contains("record id mismatch"));

    let (unheld_env, unheld_rec) =
        make_test_decline_envelope(&key, "rec_unheld", "quote_1", "conv_1", 100);
    let unheld_row = DeclineRow {
        kind: "decline".to_string(),
        deal: "quote_1".to_string(),
        declines: "rec_unheld".to_string(),
        envelope: unheld_env,
        record_id: unheld_rec,
        message_id: None,
        created_at_secs: 100,
    };
    let err_unheld = validate_ledger(
        vec![json!({ "id": "decline:quote_1", "payload": unheld_row })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(err_unheld.unwrap_err().error.unwrap().message.contains("names unheld target"));

    let enqs = HashMap::from([("rec_enq_1".to_string(), "enq_deal_1".to_string())]);
    let (enq_env, enq_rec) =
        make_test_decline_envelope(&key, "rec_enq_1", "enq_deal_1", "conv_1", 100);
    let enq_row = DeclineRow {
        kind: "decline".to_string(),
        deal: "enq_deal_1".to_string(),
        declines: "rec_enq_1".to_string(),
        envelope: enq_env,
        record_id: enq_rec,
        message_id: None,
        created_at_secs: 100,
    };
    let ok_enq = validate_ledger(
        vec![json!({ "id": "decline:enq_deal_1", "payload": enq_row })],
        &HashMap::new(),
        &HashMap::new(),
        &enqs,
        100,
    );
    assert!(ok_enq.is_ok());
}

#[test]
fn refused_row_refused_on_unheld_quote_or_counterparty_half() {
    let quotes = sample_quote_context("rec_quote_1");

    let empty_quote = RefusedRow {
        kind: "refused".to_string(),
        quote_id: String::new(),
        code: "invalid-record".to_string(),
        created_at_secs: 100,
    };
    let err1 = validate_ledger(
        vec![json!({ "id": "refused:rec_half_1", "payload": empty_quote })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(err1.unwrap_err().error.unwrap().message.contains("empty quote_id"));

    let unheld_quote = RefusedRow {
        kind: "refused".to_string(),
        quote_id: "unknown_quote".to_string(),
        code: "invalid-record".to_string(),
        created_at_secs: 100,
    };
    let err2 = validate_ledger(
        vec![json!({ "id": "refused:rec_half_1", "payload": unheld_quote })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(err2.unwrap_err().error.unwrap().message.contains("not in backup"));

    let unheld_half = RefusedRow {
        kind: "refused".to_string(),
        quote_id: "quote_1".to_string(),
        code: "invalid-record".to_string(),
        created_at_secs: 100,
    };
    let err3 = validate_ledger(
        vec![json!({ "id": "refused:rec_half_unheld", "payload": unheld_half.clone() })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(err3.unwrap_err().error.unwrap().message.contains("names unheld counterparty half"));

    let _sample_quote = quotes.get("rec_quote_1").expect("quote");
    let held_agreement = AgreementRow {
        quote_record_id: "rec_quote_1".to_string(),
        quote_id: "quote_1".to_string(),
        conversation: "conv_1".to_string(),
        counterparty_did: "did:key:cp".to_string(),
        quote_author_did: "did:key:author".to_string(),
        terms: sample_deal_terms(),
        state: None,
        state_reason: None,
        cancel_reason: None,
        seq: None,
        commitments: Vec::new(),
        agreed_at_secs: None,
        counterparty: Some(ReceiptHalf {
            envelope: "env".to_string(),
            record_id: "rec_half_1".to_string(),
            issuer: "did:key:cp".to_string(),
            issued_at_secs: 100,
        }),
        quote_author: None,
        updated_at_secs: 100,
    };
    let agreements = HashMap::from([("agr_1".to_string(), held_agreement)]);

    let ok = validate_ledger(
        vec![json!({ "id": "refused:rec_half_1", "payload": unheld_half })],
        &quotes,
        &agreements,
        &HashMap::new(),
        100,
    );
    assert!(ok.is_ok());
}

fn make_test_quote_envelope(
    author: &Identity,
    cp_did: &str,
    supersedes: Option<String>,
    now: u64,
) -> (String, String, QuotePayload, Vec<String>) {
    let author_did = substrate::derive_did_key(&author.public_key());
    let quote_id = derive_quote_id("conv_1", &author_did, 1).expect("quote id");
    let terms = DealTerms {
        currency: "EUR".to_string(),
        commitments: vec![Commitment {
            commitment_id: "c1".to_string(),
            giver: DealRole::QuoteAuthor,
            resource_kind: ResourceKind::GoodsOrService,
            kind: CommitmentKind::Fulfilment,
            extension: None,
            after: vec![],
            description: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Service"),
            declared_value_minor: 1000,
            details: None,
            window: None,
            location: None,
            payee: None,
            payment_methods: None,
            tax_minor: None,
            fees_minor: None,
            eep: None,
        }],
        home_exchange: None,
        listing_id: None,
        scope: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Scope"),
        cancellation_terms: LocalText::plain(
            LangTag::parse_canonical("en").expect("lang"),
            "Terms",
        ),
        refund_terms: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Refund"),
        dispute_path: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Dispute"),
        quote_expires_at_secs: now + 3600,
    };
    let plain = PlainQuote {
        quote_id,
        conversation: "conv_1".to_string(),
        sequence: 1,
        answers: Answers::Enquiry { record_id: "rec_enq_1".to_string() },
        counterparty_did: cp_did.to_string(),
        terms,
    };
    let (payload, disclosures) = sd::conceal_quote(plain, 1);
    let draft = RecordDraft {
        version: QUOTE_VERSION,
        record_type: "quote".to_string(),
        subject: "conv_1".to_string(),
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: Some(now + 3600),
        supersedes,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, author_did, None, now).expect("env");
    let sig = author.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("sig");
    let rec_id = env.record_id().expect("rec id");
    (env.to_json().expect("json"), rec_id, payload, disclosures)
}

#[tokio::test]
async fn decline_of_older_quote_version_in_history_imports_successfully() {
    let f = fixture();
    let now = f.host.now();
    let cp = Identity::generate().expect("cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());

    // Quote v1
    let (q1_json, q1_rec_id, q1_payload, q1_disc) =
        make_test_quote_envelope(&f.master, &cp_did, None, now);

    // Quote v2 supersedes v1
    let (q2_json, q2_rec_id, q2_payload, q2_disc) =
        make_test_quote_envelope(&f.master, &cp_did, Some(q1_rec_id.clone()), now + 10);

    // Decline of v1 by counterparty
    let (dec_env, dec_rec_id) =
        make_test_decline_envelope(&cp, &q1_rec_id, &q1_payload.quote_id, "conv_1", now + 20);

    let q2_ptr = RecordPointerRow {
        envelope: q2_json.clone(),
        record_id: q2_rec_id.clone(),
        id: q2_payload.quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 2,
        issuer: f.master_did.clone(),
        mine: true,
        updated_at_secs: now + 10,
        version_count: 2,
        issued_at_secs: now + 10,
        request_record_id: None,
        counterparty_did: Some(cp_did.clone()),
        declined_at_secs: None,
        decline_note: None,
    };

    let decline_row = DeclineRow {
        kind: "decline".to_string(),
        deal: q1_payload.quote_id.clone(),
        declines: q1_rec_id.clone(),
        envelope: dec_env,
        record_id: dec_rec_id,
        message_id: None,
        created_at_secs: now + 20,
    };

    let mut sections = BTreeMap::new();
    sections.insert(
        SECTION_QUOTES.to_string(),
        vec![json!({ "id": q2_payload.quote_id, "payload": q2_ptr })],
    );
    sections.insert(
        SECTION_QUOTE_HISTORY.to_string(),
        vec![
            json!({ "id": q1_rec_id, "payload": { "envelope": q1_json } }),
            json!({ "id": q2_rec_id, "payload": { "envelope": q2_json } }),
        ],
    );
    let disc1 = DisclosureRow {
        record_id: q1_rec_id.clone(),
        record_type: "quote".to_string(),
        disclosures: q1_disc,
        stored_at_secs: now,
    };
    let disc2 = DisclosureRow {
        record_id: q2_rec_id.clone(),
        record_type: "quote".to_string(),
        disclosures: q2_disc,
        stored_at_secs: now + 10,
    };
    sections.insert(
        SECTION_DISCLOSURES.to_string(),
        vec![
            json!({ "id": q1_rec_id, "payload": disc1 }),
            json!({ "id": q2_rec_id, "payload": disc2 }),
        ],
    );
    sections.insert(
        SECTION_LEDGER.to_string(),
        vec![json!({ "id": format!("decline:{}", q1_payload.quote_id), "payload": decline_row })],
    );

    let mut bundle = Bundle::new(f.master_did.clone(), SCHEMA_VERSION, sections).unwrap();
    signing::sign_bundle(&f.host, &mut bundle, now).await.unwrap();

    let res = backup::import(&f.host, ImportInput { bundle }).await;
    assert!(res.is_ok(), "import should succeed: {:?}", res.err());
    assert!(res.unwrap().imported);
}
