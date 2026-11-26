use serde_json::json;
use syneroym_text_lang::LangTag;

use crate::{
    deal::{
        Commitment, CommitmentKind, DealTerms, LocalText, ResourceKind, transition::CommitmentState,
    },
    eventlog::{Direction, EventLogEntry, LogEvent},
    statement::{
        AgreementSource, EepPosition, EepRole, EepSection, MAX_SAFE_INTEGER, Statement,
        StatementError, StatementOpenRow, StatementRow, StatementTotal, build,
    },
    transaction::DealRole,
};

fn lang_en() -> LangTag {
    LangTag::parse("en").unwrap()
}

fn text(s: &str) -> LocalText {
    LocalText::plain(lang_en(), s)
}

fn make_commitment(
    id: &str,
    giver: DealRole,
    resource_kind: ResourceKind,
    kind: CommitmentKind,
    desc: &str,
    value: i64,
) -> Commitment {
    Commitment {
        commitment_id: id.to_string(),
        giver,
        resource_kind,
        kind,
        extension: None,
        after: Vec::new(),
        description: text(desc),
        declared_value_minor: value,
        details: None,
        window: None,
        location: None,
        payee: None,
        payment_methods: None,
        tax_minor: None,
        fees_minor: None,
        eep: None,
    }
}

fn test_terms() -> DealTerms {
    DealTerms {
        currency: "INR".to_string(),
        quote_expires_at_secs: 3600,
        home_exchange: None,
        commitments: vec![
            make_commitment(
                "c1",
                DealRole::QuoteAuthor,
                ResourceKind::GoodsOrService,
                CommitmentKind::Fulfilment,
                "lesson",
                500,
            ),
            make_commitment(
                "c2",
                DealRole::Counterparty,
                ResourceKind::Currency,
                CommitmentKind::Currency,
                "fee",
                500,
            ),
        ],
        listing_id: None,
        scope: text("tutoring"),
        cancellation_terms: text("cancel"),
        refund_terms: text("refund"),
        dispute_path: text("dispute"),
    }
}

fn formed_entry(agreement: &str, occurred_at_secs: u64) -> EventLogEntry {
    EventLogEntry {
        id: format!("{agreement}:-:agreement-formed"),
        occurred_at_secs,
        recorded_at_secs: 0,
        event: LogEvent::AgreementFormed,
        agreement: agreement.to_string(),
        conversation: "conv_1".to_string(),
        counterparty_did: "did:key:alice".to_string(),
        commitment_id: None,
        direction: None,
        resource_kind: None,
        commitment_kind: None,
        final_state: None,
        currency: "INR".to_string(),
        declared_value_minor: None,
        settlement_note: None,
        records: vec!["h1".to_string()],
    }
}

#[test]
fn bad_period_when_from_gte_to() {
    let terms = test_terms();
    let agr = AgreementSource {
        agreement: "rec_agr",
        counterparty_did: "did:key:alice",
        owner_role: DealRole::QuoteAuthor,
        terms: &terms,
    };
    let err_equal =
        build("did:key:bob", 1000, 1000, 2000, &[], std::slice::from_ref(&agr)).unwrap_err();
    assert_eq!(err_equal, StatementError::BadPeriod);

    let err_greater = build("did:key:bob", 1001, 1000, 2000, &[], &[agr]).unwrap_err();
    assert_eq!(err_greater, StatementError::BadPeriod);
}

#[test]
fn period_boundary_from_in_to_out() {
    let terms = test_terms();
    let agr = AgreementSource {
        agreement: "rec_agr",
        counterparty_did: "did:key:alice",
        owner_role: DealRole::QuoteAuthor,
        terms: &terms,
    };
    let entries = vec![
        formed_entry("rec_agr", 100),
        EventLogEntry {
            id: "rec_agr:c1:commitment-settled".to_string(),
            occurred_at_secs: 1000,
            recorded_at_secs: 0,
            event: LogEvent::CommitmentSettled,
            agreement: "rec_agr".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: "did:key:alice".to_string(),
            commitment_id: Some("c1".to_string()),
            direction: Some(Direction::Given),
            resource_kind: Some(ResourceKind::GoodsOrService),
            commitment_kind: Some(CommitmentKind::Fulfilment),
            final_state: Some(CommitmentState::Fulfilled),
            currency: "INR".to_string(),
            declared_value_minor: Some(500),
            settlement_note: None,
            records: vec!["rec_f1".to_string()],
        },
        EventLogEntry {
            id: "rec_agr:c2:commitment-settled".to_string(),
            occurred_at_secs: 2000,
            recorded_at_secs: 0,
            event: LogEvent::CommitmentSettled,
            agreement: "rec_agr".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: "did:key:alice".to_string(),
            commitment_id: Some("c2".to_string()),
            direction: Some(Direction::Received),
            resource_kind: Some(ResourceKind::Currency),
            commitment_kind: Some(CommitmentKind::Currency),
            final_state: Some(CommitmentState::Acknowledged),
            currency: "INR".to_string(),
            declared_value_minor: Some(500),
            settlement_note: None,
            records: vec!["rec_p1".to_string()],
        },
    ];

    let st = build("did:key:bob", 1000, 2000, 3000, &entries, &[agr]).unwrap();
    assert_eq!(st.rows.len(), 1);
    assert_eq!(st.rows[0].commitment_id, "c1");
    assert_eq!(st.rows[0].settled_at_secs, 1000);
}

#[test]
fn given_and_received_rows_with_totals() {
    let terms = test_terms();
    let agr = AgreementSource {
        agreement: "rec_agr",
        counterparty_did: "did:key:alice",
        owner_role: DealRole::QuoteAuthor,
        terms: &terms,
    };
    let entries = vec![
        formed_entry("rec_agr", 100),
        EventLogEntry {
            id: "rec_agr:c1:commitment-settled".to_string(),
            occurred_at_secs: 1500,
            recorded_at_secs: 0,
            event: LogEvent::CommitmentSettled,
            agreement: "rec_agr".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: "did:key:alice".to_string(),
            commitment_id: Some("c1".to_string()),
            direction: Some(Direction::Given),
            resource_kind: Some(ResourceKind::GoodsOrService),
            commitment_kind: Some(CommitmentKind::Fulfilment),
            final_state: Some(CommitmentState::Fulfilled),
            currency: "INR".to_string(),
            declared_value_minor: Some(500),
            settlement_note: None,
            records: vec!["rec_f1".to_string()],
        },
        EventLogEntry {
            id: "rec_agr:c2:commitment-settled".to_string(),
            occurred_at_secs: 1600,
            recorded_at_secs: 0,
            event: LogEvent::CommitmentSettled,
            agreement: "rec_agr".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: "did:key:alice".to_string(),
            commitment_id: Some("c2".to_string()),
            direction: Some(Direction::Received),
            resource_kind: Some(ResourceKind::Currency),
            commitment_kind: Some(CommitmentKind::Currency),
            final_state: Some(CommitmentState::Acknowledged),
            currency: "INR".to_string(),
            declared_value_minor: Some(500),
            settlement_note: None,
            records: vec!["rec_p1".to_string()],
        },
    ];

    let st = build("did:key:bob", 1000, 2000, 3000, &entries, &[agr]).unwrap();
    assert_eq!(st.rows.len(), 2);
    assert_eq!(st.totals.len(), 2);

    let total_given = st.totals.iter().find(|t| t.direction == Direction::Given).unwrap();
    assert_eq!(total_given.currency, "INR");
    assert_eq!(total_given.resource_kind, ResourceKind::GoodsOrService);
    assert_eq!(total_given.sum_minor, 500);

    let total_received = st.totals.iter().find(|t| t.direction == Direction::Received).unwrap();
    assert_eq!(total_received.currency, "INR");
    assert_eq!(total_received.resource_kind, ResourceKind::Currency);
    assert_eq!(total_received.sum_minor, 500);
}

#[test]
fn failed_cancelled_and_ended_unconfirmed_are_not_rows_and_not_open_at_end() {
    let terms = test_terms();
    let agr = AgreementSource {
        agreement: "rec_agr",
        counterparty_did: "did:key:alice",
        owner_role: DealRole::QuoteAuthor,
        terms: &terms,
    };
    let entries = vec![
        EventLogEntry {
            id: "rec_agr:c1:commitment-failed".to_string(),
            occurred_at_secs: 1500,
            recorded_at_secs: 0,
            event: LogEvent::CommitmentFailed,
            agreement: "rec_agr".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: "did:key:alice".to_string(),
            commitment_id: Some("c1".to_string()),
            direction: Some(Direction::Given),
            resource_kind: Some(ResourceKind::GoodsOrService),
            commitment_kind: Some(CommitmentKind::Fulfilment),
            final_state: Some(CommitmentState::Failed),
            currency: "INR".to_string(),
            declared_value_minor: Some(500),
            settlement_note: None,
            records: vec!["snap_fail".to_string()],
        },
        EventLogEntry {
            id: "rec_agr:c2:commitment-cancelled".to_string(),
            occurred_at_secs: 1600,
            recorded_at_secs: 0,
            event: LogEvent::CommitmentCancelled,
            agreement: "rec_agr".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: "did:key:alice".to_string(),
            commitment_id: Some("c2".to_string()),
            direction: Some(Direction::Received),
            resource_kind: Some(ResourceKind::Currency),
            commitment_kind: Some(CommitmentKind::Currency),
            final_state: Some(CommitmentState::Cancelled),
            currency: "INR".to_string(),
            declared_value_minor: Some(500),
            settlement_note: None,
            records: vec!["snap_cancel".to_string()],
        },
    ];

    let st = build("did:key:bob", 1000, 2000, 3000, &entries, &[agr]).unwrap();
    assert!(st.rows.is_empty());
    assert!(st.open_at_end.is_empty());
}

#[test]
fn open_at_end_includes_agreed_commitment_without_final_entry() {
    let terms = test_terms();
    let agr = AgreementSource {
        agreement: "rec_agr",
        counterparty_did: "did:key:alice",
        owner_role: DealRole::QuoteAuthor,
        terms: &terms,
    };
    let entries = vec![EventLogEntry {
        id: "rec_agr:-:agreement-formed".to_string(),
        occurred_at_secs: 100,
        recorded_at_secs: 0,
        event: LogEvent::AgreementFormed,
        agreement: "rec_agr".to_string(),
        conversation: "conv_1".to_string(),
        counterparty_did: "did:key:alice".to_string(),
        commitment_id: None,
        direction: None,
        resource_kind: None,
        commitment_kind: None,
        final_state: None,
        currency: "INR".to_string(),
        declared_value_minor: None,
        settlement_note: None,
        records: vec!["h1".to_string(), "h2".to_string()],
    }];

    let st = build("did:key:bob", 1000, 2000, 3000, &entries, &[agr]).unwrap();
    assert_eq!(st.open_at_end.len(), 2);
    assert_eq!(st.open_at_end[0].commitment_id, "c1");
    assert_eq!(st.open_at_end[1].commitment_id, "c2");
}

#[test]
fn open_at_end_excludes_commitments_with_any_final_entry_whatever_its_time() {
    let terms = test_terms();
    let agr = AgreementSource {
        agreement: "rec_agr",
        counterparty_did: "did:key:alice",
        owner_role: DealRole::QuoteAuthor,
        terms: &terms,
    };
    // Final entry occurs at t=2500 (after to_secs=2000)
    let entries = vec![
        EventLogEntry {
            id: "rec_agr:-:agreement-formed".to_string(),
            occurred_at_secs: 100,
            recorded_at_secs: 0,
            event: LogEvent::AgreementFormed,
            agreement: "rec_agr".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: "did:key:alice".to_string(),
            commitment_id: None,
            direction: None,
            resource_kind: None,
            commitment_kind: None,
            final_state: None,
            currency: "INR".to_string(),
            declared_value_minor: None,
            settlement_note: None,
            records: vec![],
        },
        EventLogEntry {
            id: "rec_agr:c1:commitment-failed".to_string(),
            occurred_at_secs: 2500,
            recorded_at_secs: 0,
            event: LogEvent::CommitmentFailed,
            agreement: "rec_agr".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: "did:key:alice".to_string(),
            commitment_id: Some("c1".to_string()),
            direction: Some(Direction::Given),
            resource_kind: Some(ResourceKind::GoodsOrService),
            commitment_kind: Some(CommitmentKind::Fulfilment),
            final_state: Some(CommitmentState::Failed),
            currency: "INR".to_string(),
            declared_value_minor: Some(500),
            settlement_note: None,
            records: vec![],
        },
    ];

    let st = build("did:key:bob", 1000, 2000, 3000, &entries, &[agr]).unwrap();
    // c1 is excluded because it has a final entry; c2 is open
    assert_eq!(st.open_at_end.len(), 1);
    assert_eq!(st.open_at_end[0].commitment_id, "c2");
}

#[test]
fn commitment_settled_after_to_is_in_open_at_end() {
    let terms = test_terms();
    let agr = AgreementSource {
        agreement: "rec_agr",
        counterparty_did: "did:key:alice",
        owner_role: DealRole::QuoteAuthor,
        terms: &terms,
    };
    // Settled at t=2500 (after to_secs=2000)
    let entries = vec![
        EventLogEntry {
            id: "rec_agr:-:agreement-formed".to_string(),
            occurred_at_secs: 100,
            recorded_at_secs: 0,
            event: LogEvent::AgreementFormed,
            agreement: "rec_agr".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: "did:key:alice".to_string(),
            commitment_id: None,
            direction: None,
            resource_kind: None,
            commitment_kind: None,
            final_state: None,
            currency: "INR".to_string(),
            declared_value_minor: None,
            settlement_note: None,
            records: vec![],
        },
        EventLogEntry {
            id: "rec_agr:c1:commitment-settled".to_string(),
            occurred_at_secs: 2500,
            recorded_at_secs: 0,
            event: LogEvent::CommitmentSettled,
            agreement: "rec_agr".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: "did:key:alice".to_string(),
            commitment_id: Some("c1".to_string()),
            direction: Some(Direction::Given),
            resource_kind: Some(ResourceKind::GoodsOrService),
            commitment_kind: Some(CommitmentKind::Fulfilment),
            final_state: Some(CommitmentState::Fulfilled),
            currency: "INR".to_string(),
            declared_value_minor: Some(500),
            settlement_note: None,
            records: vec![],
        },
    ];

    let st = build("did:key:bob", 1000, 2000, 3000, &entries, &[agr]).unwrap();
    // c1 settled after to_secs, so it was open at to_secs!
    assert!(st.rows.is_empty());
    assert_eq!(st.open_at_end.len(), 2);
    assert_eq!(st.open_at_end[0].commitment_id, "c1");
    assert_eq!(st.open_at_end[1].commitment_id, "c2");
}

#[test]
fn settlement_note_is_present_on_row() {
    let terms = test_terms();
    let agr = AgreementSource {
        agreement: "rec_agr",
        counterparty_did: "did:key:alice",
        owner_role: DealRole::QuoteAuthor,
        terms: &terms,
    };
    let note = text("passed envelope");
    let entries = vec![
        formed_entry("rec_agr", 100),
        EventLogEntry {
            id: "rec_agr:c1:commitment-settled".to_string(),
            occurred_at_secs: 1500,
            recorded_at_secs: 0,
            event: LogEvent::CommitmentSettled,
            agreement: "rec_agr".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: "did:key:alice".to_string(),
            commitment_id: Some("c1".to_string()),
            direction: Some(Direction::Given),
            resource_kind: Some(ResourceKind::GoodsOrService),
            commitment_kind: Some(CommitmentKind::Fulfilment),
            final_state: Some(CommitmentState::Fulfilled),
            currency: "INR".to_string(),
            declared_value_minor: Some(500),
            settlement_note: Some(note.clone()),
            records: vec![],
        },
    ];

    let st = build("did:key:bob", 1000, 2000, 3000, &entries, &[agr]).unwrap();
    assert_eq!(st.rows.len(), 1);
    assert_eq!(st.rows[0].settlement_note, Some(note));
}

#[test]
fn eep_section_serializes_to_agreements_doc_json() {
    let not_avail = EepSection::NotAvailable;
    assert_eq!(serde_json::to_value(&not_avail).unwrap(), json!({"status": "not-available"}));

    let unavail = EepSection::Unavailable { reason: "transport timeout".to_string() };
    assert_eq!(
        serde_json::to_value(&unavail).unwrap(),
        json!({"status": "unavailable", "reason": "transport timeout"})
    );

    let avail = EepSection::Available {
        outstanding_at_start: vec![EepPosition {
            eep_id: 1,
            version: 1,
            role: EepRole::Issuer,
            issuer_did: "did:key:bob".to_string(),
            holder_did: "did:key:alice".to_string(),
            currency: "INR".to_string(),
            face_value_minor: 1000,
            expires_at_secs: 5000,
            record_id: "rec_eep_1".to_string(),
        }],
        outstanding_at_end: vec![],
        events: vec![],
    };
    assert_eq!(
        serde_json::to_value(&avail).unwrap(),
        json!({
            "status": "available",
            "outstanding_at_start": [{
                "eep_id": 1,
                "version": 1,
                "role": "issuer",
                "issuer_did": "did:key:bob",
                "holder_did": "did:key:alice",
                "currency": "INR",
                "face_value_minor": 1000,
                "expires_at_secs": 5000,
                "record_id": "rec_eep_1"
            }],
            "outstanding_at_end": [],
            "events": []
        })
    );
}

#[test]
fn total_overflow_above_max_safe_integer_refuses() {
    let terms = test_terms();
    let agr = AgreementSource {
        agreement: "rec_agr",
        counterparty_did: "did:key:alice",
        owner_role: DealRole::QuoteAuthor,
        terms: &terms,
    };
    let entries = vec![
        formed_entry("rec_agr", 100),
        EventLogEntry {
            id: "rec_agr:c1:commitment-settled".to_string(),
            occurred_at_secs: 1500,
            recorded_at_secs: 0,
            event: LogEvent::CommitmentSettled,
            agreement: "rec_agr".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: "did:key:alice".to_string(),
            commitment_id: Some("c1".to_string()),
            direction: Some(Direction::Given),
            resource_kind: Some(ResourceKind::GoodsOrService),
            commitment_kind: Some(CommitmentKind::Fulfilment),
            final_state: Some(CommitmentState::Fulfilled),
            currency: "INR".to_string(),
            declared_value_minor: Some(MAX_SAFE_INTEGER),
            settlement_note: None,
            records: vec![],
        },
        EventLogEntry {
            id: "rec_agr:c1:commitment-settled-2".to_string(),
            occurred_at_secs: 1600,
            recorded_at_secs: 0,
            event: LogEvent::CommitmentSettled,
            agreement: "rec_agr".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: "did:key:alice".to_string(),
            commitment_id: Some("c1".to_string()),
            direction: Some(Direction::Given),
            resource_kind: Some(ResourceKind::GoodsOrService),
            commitment_kind: Some(CommitmentKind::Fulfilment),
            final_state: Some(CommitmentState::Fulfilled),
            currency: "INR".to_string(),
            declared_value_minor: Some(1),
            settlement_note: None,
            records: vec![],
        },
    ];

    let err = build("did:key:bob", 1000, 2000, 3000, &entries, &[agr]).unwrap_err();
    assert_eq!(err, StatementError::TotalOverflow);
}

#[test]
fn agreement_without_formed_entry_is_not_in_open_at_end() {
    let terms = test_terms();
    let agr = AgreementSource {
        agreement: "rec_agr",
        counterparty_did: "did:key:alice",
        owner_role: DealRole::QuoteAuthor,
        terms: &terms,
    };
    // No AgreementFormed entry in entries
    let st = build("did:key:bob", 1000, 2000, 3000, &[], &[agr]).unwrap();
    assert!(st.open_at_end.is_empty());
}

#[test]
fn entry_with_unknown_agreement_or_commitment_is_skipped() {
    let terms = test_terms();
    let agr = AgreementSource {
        agreement: "rec_agr",
        counterparty_did: "did:key:alice",
        owner_role: DealRole::QuoteAuthor,
        terms: &terms,
    };
    let entries = vec![
        formed_entry("rec_agr", 100),
        EventLogEntry {
            id: "unknown_agr:c1:commitment-settled".to_string(),
            occurred_at_secs: 1500,
            recorded_at_secs: 0,
            event: LogEvent::CommitmentSettled,
            agreement: "unknown_agr".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: "did:key:alice".to_string(),
            commitment_id: Some("c1".to_string()),
            direction: Some(Direction::Given),
            resource_kind: Some(ResourceKind::GoodsOrService),
            commitment_kind: Some(CommitmentKind::Fulfilment),
            final_state: Some(CommitmentState::Fulfilled),
            currency: "INR".to_string(),
            declared_value_minor: Some(500),
            settlement_note: None,
            records: vec![],
        },
        EventLogEntry {
            id: "rec_agr:unknown_comm:commitment-settled".to_string(),
            occurred_at_secs: 1500,
            recorded_at_secs: 0,
            event: LogEvent::CommitmentSettled,
            agreement: "rec_agr".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: "did:key:alice".to_string(),
            commitment_id: Some("unknown_comm".to_string()),
            direction: Some(Direction::Given),
            resource_kind: Some(ResourceKind::GoodsOrService),
            commitment_kind: Some(CommitmentKind::Fulfilment),
            final_state: Some(CommitmentState::Fulfilled),
            currency: "INR".to_string(),
            declared_value_minor: Some(500),
            settlement_note: None,
            records: vec![],
        },
    ];
    let st = build("did:key:bob", 1000, 2000, 3000, &entries, &[agr]).unwrap();
    assert!(st.rows.is_empty());
}

#[test]
fn struct_exhaustiveness_for_statement() {
    let terms = test_terms();
    let agr = AgreementSource {
        agreement: "rec_agr",
        counterparty_did: "did:key:alice",
        owner_role: DealRole::QuoteAuthor,
        terms: &terms,
    };
    let st = build("did:key:bob", 1000, 2000, 3000, &[], &[agr]).unwrap();

    // Compile-time exhaustiveness check: fails if a field is added to Statement
    // without updating this pattern
    let Statement {
        statement_version: _,
        owner_did: _,
        from_secs: _,
        to_secs: _,
        generated_at_secs: _,
        rows: _,
        open_at_end: _,
        totals: _,
        eeps: _,
    } = st;

    let StatementRow {
        agreement: _,
        commitment_id: _,
        agreed_at_secs: _,
        settled_at_secs: _,
        direction: _,
        counterparty_did: _,
        counterparty_name: _,
        resource_kind: _,
        commitment_kind: _,
        description: _,
        currency: _,
        declared_value_minor: _,
        settlement_note: _,
        records: _,
    } = StatementRow {
        agreement: String::new(),
        commitment_id: String::new(),
        agreed_at_secs: 0,
        settled_at_secs: 0,
        direction: Direction::Given,
        counterparty_did: String::new(),
        counterparty_name: None,
        resource_kind: ResourceKind::GoodsOrService,
        commitment_kind: CommitmentKind::Fulfilment,
        description: text(""),
        currency: String::new(),
        declared_value_minor: 0,
        settlement_note: None,
        records: vec![],
    };

    let StatementOpenRow {
        agreement: _,
        commitment_id: _,
        agreed_at_secs: _,
        direction: _,
        counterparty_did: _,
        counterparty_name: _,
        resource_kind: _,
        commitment_kind: _,
        description: _,
        currency: _,
        declared_value_minor: _,
        settlement_note: _,
        records: _,
    } = StatementOpenRow {
        agreement: String::new(),
        commitment_id: String::new(),
        agreed_at_secs: 0,
        direction: Direction::Given,
        counterparty_did: String::new(),
        counterparty_name: None,
        resource_kind: ResourceKind::GoodsOrService,
        commitment_kind: CommitmentKind::Fulfilment,
        description: text(""),
        currency: String::new(),
        declared_value_minor: 0,
        settlement_note: None,
        records: vec![],
    };

    let StatementTotal { currency: _, direction: _, resource_kind: _, sum_minor: _ } =
        StatementTotal {
            currency: String::new(),
            direction: Direction::Given,
            resource_kind: ResourceKind::GoodsOrService,
            sum_minor: 0,
        };
}

#[test]
fn struct_exhaustiveness_for_event_log_entry() {
    let entry = EventLogEntry {
        id: "rec_agr:c1:commitment-settled".to_string(),
        occurred_at_secs: 1500,
        recorded_at_secs: 0,
        event: LogEvent::CommitmentSettled,
        agreement: "rec_agr".to_string(),
        conversation: "conv_1".to_string(),
        counterparty_did: "did:key:alice".to_string(),
        commitment_id: Some("c1".to_string()),
        direction: Some(Direction::Given),
        resource_kind: Some(ResourceKind::GoodsOrService),
        commitment_kind: Some(CommitmentKind::Fulfilment),
        final_state: Some(CommitmentState::Fulfilled),
        currency: "INR".to_string(),
        declared_value_minor: Some(500),
        settlement_note: None,
        records: vec![],
    };

    // Compile-time exhaustiveness check: fails if a field is added to EventLogEntry
    // without updating this pattern
    let EventLogEntry {
        id: _,
        occurred_at_secs: _,
        recorded_at_secs: _,
        event: _,
        agreement: _,
        conversation: _,
        counterparty_did: _,
        commitment_id: _,
        direction: _,
        resource_kind: _,
        commitment_kind: _,
        final_state: _,
        currency: _,
        declared_value_minor: _,
        settlement_note: _,
        records: _,
    } = entry;
}
