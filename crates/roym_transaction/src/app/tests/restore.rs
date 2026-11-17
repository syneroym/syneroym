//! Tests for restore rules: continuing sequence from cards and reusing sent
//! countersigns.

use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    card::{self, CARD_CONTENT_TYPE},
    deal::{
        Commitment, CommitmentKind, DealTerms, Details, ExtensionRef, LocalText, ResourceKind,
        transition::CommitmentState,
    },
    lang::LangTag,
    listing::derive_slot_id,
    progress::{
        AGREEMENT_PROGRESS_VERSION, AgreementProgressPayload, AgreementState, CommitmentProgress,
    },
    record::{Envelope, RECORD_AGREEMENT_PROGRESS, RECORD_AGREEMENT_RECEIPT, RecordDraft},
    transaction::{
        AGREEMENT_RECEIPT_VERSION, AgreementReceiptPayload, Answers, DealRole, QUOTE_VERSION,
        QuotePayload, ReceiptHalf, TimeWindow, derive_quote_id,
    },
};

use crate::app::{
    AGREEMENT_PROGRESS, AGREEMENTS, AgreementRow, LEDGER, QUOTE_HISTORY, QUOTES, RecordPointerRow,
    decide::{
        CountersignRow, DecisionRow, DecisionState, SnapshotRow, countersign_id, decision_id,
        recover_deciding_rows, snapshot_id,
    },
    get_row, put_bytes, put_row,
    sync::{self, SyncInput},
    tests::fixture,
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

fn build_test_quote(author: &Identity, cp_did: &str, now: u64) -> (QuotePayload, String, String) {
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
    let payload = QuotePayload {
        quote_id,
        conversation: "conv_1".to_string(),
        sequence: 1,
        counterparty_did: cp_did.to_string(),
        terms,
        answers: Answers::Enquiry { record_id: "rec_enq_1".to_string() },
    };
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
    let q_json = env.to_json().expect("json");
    (payload, rec_id, q_json)
}

fn make_countersign_envelope(
    author: &Identity,
    quote_record_id: &str,
    cp_did: &str,
    terms: &DealTerms,
    now: u64,
) -> (String, String) {
    let author_did = substrate::derive_did_key(&author.public_key());
    let payload = AgreementReceiptPayload {
        quote_record_id: quote_record_id.to_string(),
        counterparty_did: cp_did.to_string(),
        quote_author_did: author_did.clone(),
        role: DealRole::QuoteAuthor,
        terms: terms.clone(),
        viewed_lang: LangTag::parse_canonical("en").expect("lang"),
        viewed_via_mt: false,
    };
    let draft = RecordDraft {
        version: AGREEMENT_RECEIPT_VERSION,
        record_type: RECORD_AGREEMENT_RECEIPT.to_string(),
        subject: quote_record_id.to_string(),
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, author_did, None, now).expect("env");
    let sig = author.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("sig");
    let rec_id = env.record_id().expect("rec id");
    (env.to_json().expect("json"), rec_id)
}

fn initial_commitments(commitments: &[Commitment]) -> Vec<CommitmentProgress> {
    commitments
        .iter()
        .map(|c| CommitmentProgress {
            commitment_id: c.commitment_id.clone(),
            state: CommitmentState::Promised,
            reason: None,
            claimed: None,
            halves: vec![],
        })
        .collect()
}

fn make_progress_envelope(
    author: &Identity,
    quote_record_id: &str,
    cp_did: &str,
    seq: u32,
    state: AgreementState,
    commitments: Vec<CommitmentProgress>,
    now: u64,
) -> (String, String, AgreementProgressPayload) {
    let author_did = substrate::derive_did_key(&author.public_key());
    let payload = AgreementProgressPayload {
        agreement: quote_record_id.to_string(),
        conversation: "conv_1".to_string(),
        seq,
        counterparty_did: cp_did.to_string(),
        quote_author_did: author_did.clone(),
        state,
        state_reason: None,
        cancel_reason: None,
        commitments,
    };
    let draft = RecordDraft {
        version: AGREEMENT_PROGRESS_VERSION,
        record_type: RECORD_AGREEMENT_PROGRESS.to_string(),
        subject: quote_record_id.to_string(),
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

struct InitialAgreedState<'a> {
    q_rec_id: &'a str,
    quote: &'a QuotePayload,
    q_json: &'a str,
    cp_did: &'a str,
    cs: (&'a str, &'a str),
    p1: (&'a str, String, AgreementProgressPayload),
    comm_prog: Vec<CommitmentProgress>,
    now: u64,
}

async fn seed_initial_agreed_state(f: &super::TestFixture, s: InitialAgreedState<'_>) {
    let (cs_json, cs_rec_id) = s.cs;
    let (p1_json, p1_rec_id, p1_payload) = s.p1;
    let initial_row = AgreementRow {
        quote_record_id: s.q_rec_id.to_string(),
        quote_id: s.quote.quote_id.clone(),
        conversation: "conv_1".to_string(),
        counterparty_did: s.cp_did.to_string(),
        quote_author_did: f.master_did.clone(),
        terms: s.quote.terms.clone(),
        agreed_at_secs: Some(s.now),
        counterparty: None,
        quote_author: Some(ReceiptHalf {
            envelope: cs_json.to_string(),
            record_id: cs_rec_id.to_string(),
            issuer: f.master_did.clone(),
            issued_at_secs: s.now,
        }),
        state: Some(AgreementState::Agreed),
        state_reason: None,
        cancel_reason: None,
        seq: Some(1),
        commitments: s.comm_prog,
        updated_at_secs: s.now,
    };
    put_row(&f.host, AGREEMENTS, s.q_rec_id, &initial_row).await.unwrap();

    let snap1_row = SnapshotRow {
        seq: 1,
        agreement: s.q_rec_id.to_string(),
        conversation: "conv_1".to_string(),
        snapshot: p1_payload,
        envelope: p1_json.to_string(),
        record_id: p1_rec_id,
        message_id: Some("msg_p1".to_string()),
        stored_at_secs: s.now,
    };
    put_row(&f.host, AGREEMENT_PROGRESS, &snapshot_id(s.q_rec_id, 1), &snap1_row).await.unwrap();

    put_bytes(&f.host, QUOTE_HISTORY, s.q_rec_id, s.q_json.as_bytes()).await.unwrap();
    let ptr = RecordPointerRow {
        envelope: "{}".to_string(),
        record_id: s.q_rec_id.to_string(),
        id: s.quote.quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        issuer: f.master_did.clone(),
        mine: true,
        updated_at_secs: s.now,
        version_count: 1,
        issued_at_secs: s.now,
        request_record_id: None,
        counterparty_did: Some(s.cp_did.to_string()),
        declined_at_secs: None,
        decline_note: None,
    };
    put_row(&f.host, QUOTES, &s.quote.quote_id, &ptr).await.unwrap();
}

#[tokio::test]
async fn restored_writer_continues_seq_from_its_cards() {
    let f = fixture();
    let now = f.host.now();
    let cp = Identity::generate().expect("cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());

    let (quote, q_rec_id, q_json) = build_test_quote(&f.master, &cp_did, now);
    let (cs_json, cs_rec_id) =
        make_countersign_envelope(&f.master, &q_rec_id, &cp_did, &quote.terms, now);

    let comm_prog = initial_commitments(&quote.terms.commitments);
    let (p1_json, p1_rec_id, p1_payload) = make_progress_envelope(
        &f.master,
        &q_rec_id,
        &cp_did,
        1,
        AgreementState::Agreed,
        comm_prog.clone(),
        now,
    );
    let (p2_json, p2_rec_id, _p2_payload) = make_progress_envelope(
        &f.master,
        &q_rec_id,
        &cp_did,
        2,
        AgreementState::Agreed,
        comm_prog.clone(),
        now + 10,
    );

    seed_initial_agreed_state(
        &f,
        InitialAgreedState {
            q_rec_id: &q_rec_id,
            quote: &quote,
            q_json: &q_json,
            cp_did: &cp_did,
            cs: (&cs_json, &cs_rec_id),
            p1: (&p1_json, p1_rec_id, p1_payload),
            comm_prog,
            now,
        },
    )
    .await;

    // Now sync discovers outgoing card seq 2 in conversation history
    let p2_card_body =
        card::card_body(RECORD_AGREEMENT_PROGRESS, AGREEMENT_PROGRESS_VERSION, &p2_json)
            .expect("card body");

    f.host.on_call("conversation", "conversation.changes", move |_| {
        Ok(json!({
            "result": {
                "messages": [
                    {
                        "id": "msg_p2",
                        "conversation": "conv_1",
                        "author": "did:key:author",
                        "direction": "outgoing",
                        "sender_timestamp_ms": (now + 10) as i64 * 1000,
                        "content_type": CARD_CONTENT_TYPE,
                        "languages": ["en"],
                        "body_encoding": "utf8",
                        "restored": false,
                        "state": "delivered",
                        "stored_at_secs": (now + 10) as i64,
                        "visible_seq": 2,
                        "verified": true,
                        "body": p2_card_body,
                    }
                ],
                "last_seq": 2,
                "kind": "direct"
            }
        })
        .to_string())
    });

    sync::sync(&f.host, SyncInput { conversation: "conv_1".to_string(), full: false })
        .await
        .unwrap();

    // Writer tracked highest seq: row.seq is now 2
    let updated_agr: AgreementRow = get_row(&f.host, AGREEMENTS, &q_rec_id).await.unwrap().unwrap();
    assert_eq!(updated_agr.seq, Some(2));

    // And snapshot 2 is stored
    let snap2: Option<SnapshotRow> =
        get_row(&f.host, AGREEMENT_PROGRESS, &snapshot_id(&q_rec_id, 2)).await.unwrap();
    assert!(snap2.is_some());
    assert_eq!(snap2.unwrap().record_id, p2_rec_id);
}

#[tokio::test]
async fn restored_writer_reuses_its_sent_countersign() {
    let f = fixture();
    let now = f.host.now();
    let cp = Identity::generate().expect("cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());

    let (quote, q_rec_id, _) = build_test_quote(&f.master, &cp_did, now);
    let (cs_json, cs_rec_id) =
        make_countersign_envelope(&f.master, &q_rec_id, &cp_did, &quote.terms, now);

    // Initial state: decision row is deciding, but countersign row in LEDGER is
    // missing!
    let dec_fence = DecisionRow {
        kind: "decision".to_string(),
        state: DecisionState::Deciding.as_str().to_string(),
        agreement: q_rec_id.clone(),
        quote_id: quote.quote_id.clone(),
        reason: None,
        created_at_secs: now,
    };
    put_row(&f.host, LEDGER, &decision_id(&quote.quote_id), &dec_fence).await.unwrap();

    let initial_row = AgreementRow {
        quote_record_id: q_rec_id.clone(),
        quote_id: quote.quote_id.clone(),
        conversation: "conv_1".to_string(),
        counterparty_did: cp_did.clone(),
        quote_author_did: f.master_did.clone(),
        terms: quote.terms.clone(),
        agreed_at_secs: None,
        counterparty: None,
        quote_author: None,
        state: None,
        state_reason: None,
        cancel_reason: None,
        seq: None,
        commitments: vec![],
        updated_at_secs: now,
    };
    put_row(&f.host, AGREEMENTS, &q_rec_id, &initial_row).await.unwrap();

    // Conversation history from 0 holds the sent countersign card
    let cs_card_body =
        card::card_body(RECORD_AGREEMENT_RECEIPT, AGREEMENT_RECEIPT_VERSION, &cs_json)
            .expect("card body");

    f.host.on_call("conversation", "conversation.changes", move |_| {
        Ok(json!({
            "result": {
                "messages": [
                    {
                        "id": "msg_sent_cs",
                        "conversation": "conv_1",
                        "author": "did:key:author",
                        "direction": "outgoing",
                        "sender_timestamp_ms": (now * 1000) as i64,
                        "content_type": CARD_CONTENT_TYPE,
                        "languages": ["en"],
                        "body_encoding": "utf8",
                        "restored": false,
                        "state": "delivered",
                        "stored_at_secs": now as i64,
                        "visible_seq": 1,
                        "verified": true,
                        "body": cs_card_body,
                    }
                ],
                "last_seq": 1,
                "kind": "direct"
            }
        })
        .to_string())
    });

    recover_deciding_rows(&f.host, now).await.unwrap();

    // The countersign row is restored and reuses msg_sent_cs
    let cs_row: CountersignRow =
        get_row(&f.host, LEDGER, &countersign_id(&q_rec_id)).await.unwrap().unwrap();
    assert_eq!(cs_row.half.record_id, cs_rec_id);
    assert_eq!(cs_row.message_id.as_deref(), Some("msg_sent_cs"));

    // Agreement row is marked agreed with the restored author receipt half
    let agr_row: AgreementRow = get_row(&f.host, AGREEMENTS, &q_rec_id).await.unwrap().unwrap();
    assert_eq!(agr_row.quote_author.unwrap().record_id, cs_rec_id);

    // Decision row ends agreed
    let dec_row: DecisionRow =
        get_row(&f.host, LEDGER, &decision_id(&quote.quote_id)).await.unwrap().unwrap();
    assert_eq!(dec_row.state, DecisionState::Agreed.as_str());
}
