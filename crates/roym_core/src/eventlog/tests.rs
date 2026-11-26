use syneroym_text_lang::LangTag;

use crate::{
    deal::{
        Commitment, CommitmentKind, DealTerms, LocalText, ResourceKind, transition::CommitmentState,
    },
    eventlog::{DeriveInput, Direction, FormedFact, HeldSnapshot, LogEvent, SettlingHalf, derive},
    progress::{AgreementState, CommitmentProgress},
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

#[test]
fn event_kind_agreement_formed() {
    let terms = test_terms();
    let r_halves = ["rec_h1".to_string(), "rec_h2".to_string()];
    let input = DeriveInput {
        owner_role: DealRole::QuoteAuthor,
        agreement: "rec_agr1",
        conversation: "conv_1",
        counterparty_did: "did:key:alice",
        terms: &terms,
        state: Some(AgreementState::Agreed),
        commitments: &[],
        agreed: Some(FormedFact {
            quote_author_issued_at_secs: Some(1000),
            receipt_halves: &r_halves,
        }),
        snapshots: &[],
        settling: &[],
    };
    let d = derive(&input);
    assert!(d.complete);
    assert_eq!(d.entries.len(), 1);
    assert_eq!(d.entries[0].event, LogEvent::AgreementFormed);
    assert_eq!(d.entries[0].occurred_at_secs, 1000);
    assert_eq!(d.entries[0].records, vec!["rec_h1", "rec_h2"]);
    assert_eq!(d.entries[0].id, "rec_agr1:-:agreement-formed");
}

#[test]
fn event_kind_agreement_closed() {
    let terms = test_terms();
    let input = DeriveInput {
        owner_role: DealRole::QuoteAuthor,
        agreement: "rec_agr1",
        conversation: "conv_1",
        counterparty_did: "did:key:alice",
        terms: &terms,
        state: Some(AgreementState::Closed),
        commitments: &[],
        agreed: None,
        snapshots: &[HeldSnapshot {
            seq: 2,
            issued_at_secs: 1200,
            record_id: "rec_snap_closed",
            state: AgreementState::Closed,
            commitments: &[],
        }],
        settling: &[],
    };
    let d = derive(&input);
    assert!(d.complete);
    assert_eq!(d.entries.len(), 1);
    assert_eq!(d.entries[0].event, LogEvent::AgreementClosed);
    assert_eq!(d.entries[0].occurred_at_secs, 1200);
    assert_eq!(d.entries[0].records, vec!["rec_snap_closed"]);
    assert_eq!(d.entries[0].id, "rec_agr1:-:agreement-closed");
}

#[test]
fn event_kind_agreement_cancelled() {
    let terms = test_terms();
    let input = DeriveInput {
        owner_role: DealRole::QuoteAuthor,
        agreement: "rec_agr1",
        conversation: "conv_1",
        counterparty_did: "did:key:alice",
        terms: &terms,
        state: Some(AgreementState::Cancelled),
        commitments: &[],
        agreed: None,
        snapshots: &[HeldSnapshot {
            seq: 1,
            issued_at_secs: 1300,
            record_id: "rec_snap_cancel",
            state: AgreementState::Cancelled,
            commitments: &[],
        }],
        settling: &[],
    };
    let d = derive(&input);
    assert!(d.complete);
    assert_eq!(d.entries[0].event, LogEvent::AgreementCancelled);
    assert_eq!(d.entries[0].occurred_at_secs, 1300);
    assert_eq!(d.entries[0].records, vec!["rec_snap_cancel"]);
    assert_eq!(d.entries[0].id, "rec_agr1:-:agreement-cancelled");
}

#[test]
fn event_kind_agreement_not_formed() {
    let terms = test_terms();
    let input = DeriveInput {
        owner_role: DealRole::QuoteAuthor,
        agreement: "rec_agr1",
        conversation: "conv_1",
        counterparty_did: "did:key:alice",
        terms: &terms,
        state: Some(AgreementState::NotFormed),
        commitments: &[],
        agreed: None,
        snapshots: &[HeldSnapshot {
            seq: 1,
            issued_at_secs: 1400,
            record_id: "rec_snap_not_formed",
            state: AgreementState::NotFormed,
            commitments: &[],
        }],
        settling: &[],
    };
    let d = derive(&input);
    assert!(d.complete);
    assert_eq!(d.entries[0].event, LogEvent::AgreementNotFormed);
    assert_eq!(d.entries[0].occurred_at_secs, 1400);
    assert_eq!(d.entries[0].records, vec!["rec_snap_not_formed"]);
    assert_eq!(d.entries[0].id, "rec_agr1:-:agreement-not-formed");
}

#[test]
fn event_kind_commitment_settled() {
    let terms = test_terms();
    let note = text("handed over in person");
    let settling_recs = ["rec_f1".to_string(), "rec_f2".to_string()];
    let input = DeriveInput {
        owner_role: DealRole::QuoteAuthor,
        agreement: "rec_agr1",
        conversation: "conv_1",
        counterparty_did: "did:key:alice",
        terms: &terms,
        state: Some(AgreementState::Agreed),
        commitments: &[CommitmentProgress {
            commitment_id: "c1".to_string(),
            state: CommitmentState::Fulfilled,
            reason: None,
            claimed: Some(true),
            halves: vec!["rec_f1".to_string()],
        }],
        agreed: None,
        snapshots: &[],
        settling: &[SettlingHalf {
            commitment_id: "c1",
            records: &settling_recs,
            settling_issued_at_secs: Some(1500),
            settlement_note: Some(&note),
        }],
    };
    let d = derive(&input);
    assert!(d.complete);
    assert_eq!(d.entries.len(), 1);
    assert_eq!(d.entries[0].event, LogEvent::CommitmentSettled);
    assert_eq!(d.entries[0].occurred_at_secs, 1500);
    assert_eq!(d.entries[0].records, vec!["rec_f1", "rec_f2"]);
    assert_eq!(d.entries[0].settlement_note, Some(note));
    assert_eq!(d.entries[0].id, "rec_agr1:c1:commitment-settled");
}

#[test]
fn event_kind_commitment_failed() {
    let terms = test_terms();
    let input = DeriveInput {
        owner_role: DealRole::QuoteAuthor,
        agreement: "rec_agr1",
        conversation: "conv_1",
        counterparty_did: "did:key:alice",
        terms: &terms,
        state: Some(AgreementState::Agreed),
        commitments: &[CommitmentProgress {
            commitment_id: "c1".to_string(),
            state: CommitmentState::Failed,
            reason: Some("dispute".to_string()),
            claimed: None,
            halves: vec![],
        }],
        agreed: None,
        snapshots: &[HeldSnapshot {
            seq: 3,
            issued_at_secs: 1600,
            record_id: "rec_snap_fail",
            state: AgreementState::Agreed,
            commitments: &[CommitmentProgress {
                commitment_id: "c1".to_string(),
                state: CommitmentState::Failed,
                reason: Some("dispute".to_string()),
                claimed: None,
                halves: vec![],
            }],
        }],
        settling: &[],
    };
    let d = derive(&input);
    assert!(d.complete);
    assert_eq!(d.entries[0].event, LogEvent::CommitmentFailed);
    assert_eq!(d.entries[0].occurred_at_secs, 1600);
    assert_eq!(d.entries[0].records, vec!["rec_snap_fail"]);
    assert_eq!(d.entries[0].id, "rec_agr1:c1:commitment-failed");
}

#[test]
fn event_kind_commitment_ended_unconfirmed() {
    let terms = test_terms();
    let input = DeriveInput {
        owner_role: DealRole::QuoteAuthor,
        agreement: "rec_agr1",
        conversation: "conv_1",
        counterparty_did: "did:key:alice",
        terms: &terms,
        state: Some(AgreementState::Agreed),
        commitments: &[CommitmentProgress {
            commitment_id: "c1".to_string(),
            state: CommitmentState::EndedUnconfirmed,
            reason: None,
            claimed: Some(true),
            halves: vec![],
        }],
        agreed: None,
        snapshots: &[HeldSnapshot {
            seq: 2,
            issued_at_secs: 1700,
            record_id: "rec_snap_unconf",
            state: AgreementState::Agreed,
            commitments: &[CommitmentProgress {
                commitment_id: "c1".to_string(),
                state: CommitmentState::EndedUnconfirmed,
                reason: None,
                claimed: Some(true),
                halves: vec![],
            }],
        }],
        settling: &[],
    };
    let d = derive(&input);
    assert!(d.complete);
    assert_eq!(d.entries[0].event, LogEvent::CommitmentEndedUnconfirmed);
    assert_eq!(d.entries[0].occurred_at_secs, 1700);
    assert_eq!(d.entries[0].records, vec!["rec_snap_unconf"]);
    assert_eq!(d.entries[0].id, "rec_agr1:c1:commitment-ended-unconfirmed");
}

#[test]
fn event_kind_commitment_cancelled() {
    let terms = test_terms();
    let input = DeriveInput {
        owner_role: DealRole::QuoteAuthor,
        agreement: "rec_agr1",
        conversation: "conv_1",
        counterparty_did: "did:key:alice",
        terms: &terms,
        state: Some(AgreementState::Cancelled),
        commitments: &[CommitmentProgress {
            commitment_id: "c1".to_string(),
            state: CommitmentState::Cancelled,
            reason: None,
            claimed: None,
            halves: vec![],
        }],
        agreed: None,
        snapshots: &[HeldSnapshot {
            seq: 2,
            issued_at_secs: 1800,
            record_id: "rec_snap_cancel",
            state: AgreementState::Cancelled,
            commitments: &[CommitmentProgress {
                commitment_id: "c1".to_string(),
                state: CommitmentState::Cancelled,
                reason: None,
                claimed: None,
                halves: vec![],
            }],
        }],
        settling: &[],
    };
    let d = derive(&input);
    assert!(d.complete);
    assert_eq!(d.entries[0].event, LogEvent::CommitmentCancelled);
    assert_eq!(d.entries[0].occurred_at_secs, 1800);
    assert_eq!(d.entries[0].records, vec!["rec_snap_cancel"]);
    assert_eq!(d.entries[0].id, "rec_agr1:c1:commitment-cancelled");
}

#[test]
fn direction_from_each_side() {
    let terms = test_terms();
    let commitments = vec![
        CommitmentProgress {
            commitment_id: "c1".to_string(),
            state: CommitmentState::Fulfilled,
            reason: None,
            claimed: None,
            halves: vec![],
        },
        CommitmentProgress {
            commitment_id: "c2".to_string(),
            state: CommitmentState::Acknowledged,
            reason: None,
            claimed: None,
            halves: vec![],
        },
    ];
    let empty_records: [String; 0] = [];
    let settling = vec![
        SettlingHalf {
            commitment_id: "c1",
            records: &empty_records,
            settling_issued_at_secs: Some(100),
            settlement_note: None,
        },
        SettlingHalf {
            commitment_id: "c2",
            records: &empty_records,
            settling_issued_at_secs: Some(101),
            settlement_note: None,
        },
    ];

    // As QuoteAuthor: c1 is given, c2 is received
    let author_input = DeriveInput {
        owner_role: DealRole::QuoteAuthor,
        agreement: "rec_agr",
        conversation: "conv_1",
        counterparty_did: "did:key:alice",
        terms: &terms,
        state: Some(AgreementState::Agreed),
        commitments: &commitments,
        agreed: None,
        snapshots: &[],
        settling: &settling,
    };
    let d_author = derive(&author_input);
    assert_eq!(d_author.entries.len(), 2);
    assert_eq!(d_author.entries[0].direction, Some(Direction::Given));
    assert_eq!(d_author.entries[1].direction, Some(Direction::Received));

    // As Counterparty: c1 is received, c2 is given
    let counterparty_input = DeriveInput {
        owner_role: DealRole::Counterparty,
        agreement: "rec_agr",
        conversation: "conv_1",
        counterparty_did: "did:key:bob",
        terms: &terms,
        state: Some(AgreementState::Agreed),
        commitments: &commitments,
        agreed: None,
        snapshots: &[],
        settling: &settling,
    };
    let d_counterparty = derive(&counterparty_input);
    assert_eq!(d_counterparty.entries.len(), 2);
    assert_eq!(d_counterparty.entries[0].direction, Some(Direction::Received));
    assert_eq!(d_counterparty.entries[1].direction, Some(Direction::Given));
}

#[test]
fn settled_commitment_without_settling_half_defers_entry() {
    let terms = test_terms();
    let commitments = vec![CommitmentProgress {
        commitment_id: "c1".to_string(),
        state: CommitmentState::Fulfilled,
        reason: None,
        claimed: None,
        halves: vec![],
    }];

    // Settling half missing
    let input_missing = DeriveInput {
        owner_role: DealRole::QuoteAuthor,
        agreement: "rec_agr",
        conversation: "conv_1",
        counterparty_did: "did:key:alice",
        terms: &terms,
        state: Some(AgreementState::Agreed),
        commitments: &commitments,
        agreed: None,
        snapshots: &[],
        settling: &[],
    };
    let d_missing = derive(&input_missing);
    assert!(!d_missing.complete);
    assert!(d_missing.entries.is_empty());

    // Settling half added
    let r_half = ["rec_half".to_string()];
    let settling = vec![SettlingHalf {
        commitment_id: "c1",
        records: &r_half,
        settling_issued_at_secs: Some(500),
        settlement_note: None,
    }];
    let input_complete = DeriveInput {
        owner_role: DealRole::QuoteAuthor,
        agreement: "rec_agr",
        conversation: "conv_1",
        counterparty_did: "did:key:alice",
        terms: &terms,
        state: Some(AgreementState::Agreed),
        commitments: &commitments,
        agreed: None,
        snapshots: &[],
        settling: &settling,
    };
    let d_complete = derive(&input_complete);
    assert!(d_complete.complete);
    assert_eq!(d_complete.entries.len(), 1);
    assert_eq!(d_complete.entries[0].event, LogEvent::CommitmentSettled);
}

#[test]
fn corrected_acknowledgement_does_not_move_occurred_at() {
    let terms = test_terms();
    let commitments = vec![CommitmentProgress {
        commitment_id: "c2".to_string(),
        state: CommitmentState::Acknowledged,
        reason: None,
        claimed: None,
        halves: vec!["rec_ack_1".to_string(), "rec_ack_2".to_string()],
    }];
    let r_ack = ["rec_ack_1".to_string(), "rec_ack_2".to_string()];
    let settling = vec![SettlingHalf {
        commitment_id: "c2",
        records: &r_ack,
        settling_issued_at_secs: Some(1000),
        settlement_note: None,
    }];
    let input = DeriveInput {
        owner_role: DealRole::QuoteAuthor,
        agreement: "rec_agr",
        conversation: "conv_1",
        counterparty_did: "did:key:alice",
        terms: &terms,
        state: Some(AgreementState::Agreed),
        commitments: &commitments,
        agreed: None,
        snapshots: &[],
        settling: &settling,
    };
    let d = derive(&input);
    assert_eq!(d.entries[0].occurred_at_secs, 1000);
    assert_eq!(d.entries[0].records, vec!["rec_ack_1", "rec_ack_2"]);
}

#[test]
fn agreed_at_secs_uses_countersigned_half_time() {
    let terms = test_terms();
    let r_halves = ["rec_h1".to_string(), "rec_h2".to_string()];
    let input = DeriveInput {
        owner_role: DealRole::Counterparty,
        agreement: "rec_agr",
        conversation: "conv_1",
        counterparty_did: "did:key:bob",
        terms: &terms,
        state: Some(AgreementState::Agreed),
        commitments: &[],
        agreed: Some(FormedFact {
            quote_author_issued_at_secs: Some(950),
            receipt_halves: &r_halves,
        }),
        snapshots: &[],
        settling: &[],
    };
    let d = derive(&input);
    assert_eq!(d.entries[0].occurred_at_secs, 950);
}

#[test]
fn lowest_seq_snapshot_wins_and_second_used_if_first_missing() {
    let terms = test_terms();
    let snapshots_both = vec![
        HeldSnapshot {
            seq: 2,
            issued_at_secs: 200,
            record_id: "rec_snap_2",
            state: AgreementState::Closed,
            commitments: &[],
        },
        HeldSnapshot {
            seq: 3,
            issued_at_secs: 300,
            record_id: "rec_snap_3",
            state: AgreementState::Closed,
            commitments: &[],
        },
    ];

    let input_both = DeriveInput {
        owner_role: DealRole::QuoteAuthor,
        agreement: "rec_agr",
        conversation: "conv_1",
        counterparty_did: "did:key:alice",
        terms: &terms,
        state: Some(AgreementState::Closed),
        commitments: &[],
        agreed: None,
        snapshots: &snapshots_both,
        settling: &[],
    };
    let d_both = derive(&input_both);
    assert_eq!(d_both.entries[0].occurred_at_secs, 200);
    assert_eq!(d_both.entries[0].records, vec!["rec_snap_2"]);

    // With seq 2 missing, seq 3 is used
    let input_second = DeriveInput { snapshots: &snapshots_both[1..], ..input_both };
    let d_second = derive(&input_second);
    assert_eq!(d_second.entries[0].occurred_at_secs, 300);
    assert_eq!(d_second.entries[0].records, vec!["rec_snap_3"]);
}

#[test]
fn derive_run_twice_gives_equal_entries() {
    let terms = test_terms();
    let h1 = ["h1".to_string()];
    let h2 = ["h2".to_string()];
    let input = DeriveInput {
        owner_role: DealRole::QuoteAuthor,
        agreement: "rec_agr",
        conversation: "conv_1",
        counterparty_did: "did:key:alice",
        terms: &terms,
        state: Some(AgreementState::Closed),
        commitments: &[CommitmentProgress {
            commitment_id: "c1".to_string(),
            state: CommitmentState::Fulfilled,
            reason: None,
            claimed: None,
            halves: vec![],
        }],
        agreed: Some(FormedFact { quote_author_issued_at_secs: Some(100), receipt_halves: &h1 }),
        snapshots: &[HeldSnapshot {
            seq: 1,
            issued_at_secs: 300,
            record_id: "snap_1",
            state: AgreementState::Closed,
            commitments: &[],
        }],
        settling: &[SettlingHalf {
            commitment_id: "c1",
            records: &h2,
            settling_issued_at_secs: Some(200),
            settlement_note: None,
        }],
    };

    let d1 = derive(&input);
    let d2 = derive(&input);
    assert_eq!(d1, d2);
}

#[test]
fn deal_with_no_currency_commitment_gives_same_entry_kinds() {
    let terms = DealTerms {
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
                ResourceKind::GoodsOrService,
                CommitmentKind::Fulfilment,
                "wheat",
                500,
            ),
        ],
        listing_id: None,
        scope: text("barter-scope"),
        cancellation_terms: text("cancel"),
        refund_terms: text("refund"),
        dispute_path: text("dispute"),
    };

    let commitments = vec![
        CommitmentProgress {
            commitment_id: "c1".to_string(),
            state: CommitmentState::Fulfilled,
            reason: None,
            claimed: None,
            halves: vec![],
        },
        CommitmentProgress {
            commitment_id: "c2".to_string(),
            state: CommitmentState::Fulfilled,
            reason: None,
            claimed: None,
            halves: vec![],
        },
    ];

    let h1 = ["rec_h1".to_string()];
    let h2 = ["rec_h2".to_string()];
    let settling = vec![
        SettlingHalf {
            commitment_id: "c1",
            records: &h1,
            settling_issued_at_secs: Some(100),
            settlement_note: None,
        },
        SettlingHalf {
            commitment_id: "c2",
            records: &h2,
            settling_issued_at_secs: Some(150),
            settlement_note: None,
        },
    ];

    let agree_h = ["rec_agree_1".to_string()];
    let input = DeriveInput {
        owner_role: DealRole::QuoteAuthor,
        agreement: "rec_agr",
        conversation: "conv_1",
        counterparty_did: "did:key:alice",
        terms: &terms,
        state: Some(AgreementState::Closed),
        commitments: &commitments,
        agreed: Some(FormedFact {
            quote_author_issued_at_secs: Some(50),
            receipt_halves: &agree_h,
        }),
        snapshots: &[HeldSnapshot {
            seq: 2,
            issued_at_secs: 200,
            record_id: "rec_snap_closed",
            state: AgreementState::Closed,
            commitments: &commitments,
        }],
        settling: &settling,
    };

    let d = derive(&input);
    assert!(d.complete);
    assert_eq!(d.entries.len(), 4);
    assert_eq!(d.entries[0].event, LogEvent::AgreementFormed);
    assert_eq!(d.entries[1].event, LogEvent::CommitmentSettled);
    assert_eq!(d.entries[1].commitment_kind, Some(CommitmentKind::Fulfilment));
    assert_eq!(d.entries[2].event, LogEvent::CommitmentSettled);
    assert_eq!(d.entries[2].commitment_kind, Some(CommitmentKind::Fulfilment));
    assert_eq!(d.entries[3].event, LogEvent::AgreementClosed);
}
