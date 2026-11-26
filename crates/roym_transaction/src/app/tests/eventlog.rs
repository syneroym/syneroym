use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    clock,
    deal::{Commitment, DealTerms, LocalText, transition::CommitmentState},
    eventlog::{Direction, EventLogEntry, LogEvent},
    lang::LangTag,
    progress::{
        AgreementProgressPayload, AgreementState, CancelReason, CommitmentProgress, FormedAs,
    },
    transaction::{DealRole, ReceiptHalf},
};

use super::{
    fixture, fixture_with_master,
    settle::{make_currency_commitment, make_service_commitment, sample_note, setup_agreed_deal},
};
use crate::app::{
    AGREEMENT_PROGRESS, AGREEMENTS, AgreementRow, EVENT_LOG, EVENT_LOG_STATE,
    decide::SnapshotRow,
    eventlog_ops::{self, EventLogListInput},
    fulfilment_ops::{self, FulfilmentSignInput},
    get_row,
    payment_ops::{self, PaymentAckInput},
    put_row,
    sync::sync_direct,
};

async fn store_progress_snapshot(
    fix: &super::TestFixture,
    agreement: &str,
    seq: u32,
    state: AgreementState,
    commitments: Vec<CommitmentProgress>,
    issued_at_secs: u64,
) {
    let payload = AgreementProgressPayload {
        agreement: agreement.to_string(),
        conversation: "conv_1".to_string(),
        counterparty_did: "did:key:zCp".to_string(),
        quote_author_did: fix.master_did.clone(),
        seq,
        state,
        formed_as: FormedAs::Quote,
        state_reason: None,
        cancel_reason: None,
        commitments,
    };
    let (env, record_id) =
        crate::app::progress::sign(&fix.host, &payload).await.expect("sign snapshot");
    let snap_key = crate::app::decide::snapshot_id(agreement, seq);
    let snap_row = SnapshotRow {
        seq,
        agreement: agreement.to_string(),
        conversation: "conv_1".to_string(),
        snapshot: payload,
        envelope: env,
        record_id,
        message_id: None,
        stored_at_secs: issued_at_secs,
    };
    put_row(&fix.host, AGREEMENT_PROGRESS, &snap_key, &snap_row).await.expect("put snapshot row");
}

fn make_prog(id: &str, state: CommitmentState, halves: Vec<String>) -> CommitmentProgress {
    CommitmentProgress { commitment_id: id.to_string(), state, reason: None, claimed: None, halves }
}

#[tokio::test]
async fn writer_derives_agreement_formed() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();
    let (agreement, _) =
        setup_agreed_deal(&fix, &cp, vec![make_service_commitment("c1")], now).await;

    eventlog_ops::derive_conversation(&fix.host, "conv_1", now).await.expect("derive");

    let list = eventlog_ops::list(&fix.host, EventLogListInput::default()).await.expect("list");
    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].event, LogEvent::AgreementFormed);
    assert_eq!(list.entries[0].occurred_at_secs, now);
    assert_eq!(list.entries[0].agreement, agreement);
}

#[tokio::test]
async fn writer_derives_commitment_settled_for_service() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();
    let mut comm = make_service_commitment("c1");
    comm.giver = DealRole::Counterparty;
    let (agreement, _) = setup_agreed_deal(&fix, &cp, vec![comm], now).await;

    fix.host.set_now(now + 15);
    fulfilment_ops::sign_at(
        &fix.host,
        FulfilmentSignInput {
            agreement: agreement.clone(),
            commitment_id: "c1".to_string(),
            settlement_note: Some(sample_note("All done")),
        },
        now + 15,
    )
    .await
    .expect("sign fulfilment");

    eventlog_ops::derive_conversation(&fix.host, "conv_1", now + 20).await.expect("derive");

    let list = eventlog_ops::list(&fix.host, EventLogListInput::default()).await.expect("list");
    assert_eq!(list.entries.len(), 3);
    let settled = list.entries.iter().find(|e| e.event == LogEvent::CommitmentSettled).unwrap();
    assert_eq!(settled.direction, Some(Direction::Received));
    assert_eq!(settled.occurred_at_secs, now + 15);
    assert_eq!(settled.commitment_id.as_deref(), Some("c1"));
    assert_eq!(settled.settlement_note.as_ref().unwrap().text, "All done");
    assert!(list.entries.iter().any(|e| e.event == LogEvent::AgreementClosed));
}

#[tokio::test]
async fn writer_derives_commitment_settled_for_payment() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();
    let (agreement, _) = setup_agreed_deal(
        &fix,
        &cp,
        vec![make_currency_commitment("c1", &fix.master_did), make_service_commitment("c2")],
        now,
    )
    .await;

    fix.host.set_now(now + 25);
    payment_ops::acknowledge_at(
        &fix.host,
        PaymentAckInput {
            agreement: agreement.clone(),
            commitment_id: "c1".to_string(),
            observed_at_secs: None,
            method: None,
            reference: None,
            settlement_note: Some(sample_note("Thanks for payment")),
            supersedes: None,
        },
        now + 25,
    )
    .await
    .expect("ack payment");

    eventlog_ops::derive_conversation(&fix.host, "conv_1", now + 30).await.expect("derive");

    let list = eventlog_ops::list(&fix.host, EventLogListInput::default()).await.expect("list");
    let settled = list.entries.iter().find(|e| e.event == LogEvent::CommitmentSettled).unwrap();
    assert_eq!(settled.direction, Some(Direction::Received));
    assert_eq!(settled.occurred_at_secs, now + 25);
    assert_eq!(settled.commitment_id.as_deref(), Some("c1"));
}

#[tokio::test]
async fn writer_derives_agreement_closed() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();
    let (agreement, _) =
        setup_agreed_deal(&fix, &cp, vec![make_service_commitment("c1")], now).await;

    store_progress_snapshot(
        &fix,
        &agreement,
        2,
        AgreementState::Closed,
        vec![make_prog("c1", CommitmentState::Fulfilled, vec!["h1".into()])],
        now + 50,
    )
    .await;

    let mut row: AgreementRow = get_row(&fix.host, AGREEMENTS, &agreement).await.unwrap().unwrap();
    row.state = Some(AgreementState::Closed);
    row.seq = Some(2);
    put_row(&fix.host, AGREEMENTS, &agreement, &row).await.unwrap();

    eventlog_ops::derive_conversation(&fix.host, "conv_1", now + 50).await.expect("derive");

    let list = eventlog_ops::list(&fix.host, EventLogListInput::default()).await.expect("list");
    assert!(list.entries.iter().any(|e| e.event == LogEvent::AgreementClosed));
}

#[tokio::test]
async fn writer_derives_agreement_cancelled() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();
    let (agreement, _) =
        setup_agreed_deal(&fix, &cp, vec![make_service_commitment("c1")], now).await;

    store_progress_snapshot(
        &fix,
        &agreement,
        2,
        AgreementState::Cancelled,
        vec![make_prog("c1", CommitmentState::Cancelled, vec![])],
        now + 40,
    )
    .await;

    let mut row: AgreementRow = get_row(&fix.host, AGREEMENTS, &agreement).await.unwrap().unwrap();
    row.state = Some(AgreementState::Cancelled);
    row.seq = Some(2);
    put_row(&fix.host, AGREEMENTS, &agreement, &row).await.unwrap();

    eventlog_ops::derive_conversation(&fix.host, "conv_1", now + 40).await.expect("derive");

    let list = eventlog_ops::list(&fix.host, EventLogListInput::default()).await.expect("list");
    assert!(list.entries.iter().any(|e| e.event == LogEvent::AgreementCancelled));
}

#[tokio::test]
async fn writer_derives_agreement_not_formed() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();
    let (agreement, _) =
        setup_agreed_deal(&fix, &cp, vec![make_service_commitment("c1")], now).await;

    store_progress_snapshot(
        &fix,
        &agreement,
        2,
        AgreementState::NotFormed,
        vec![make_prog("c1", CommitmentState::Promised, vec![])],
        now + 35,
    )
    .await;

    let mut row: AgreementRow = get_row(&fix.host, AGREEMENTS, &agreement).await.unwrap().unwrap();
    row.state = Some(AgreementState::NotFormed);
    row.quote_author = None;
    row.seq = Some(2);
    put_row(&fix.host, AGREEMENTS, &agreement, &row).await.unwrap();

    eventlog_ops::derive_conversation(&fix.host, "conv_1", now + 35).await.expect("derive");

    let marker: crate::app::eventlog_ops::EventLogStateRow =
        get_row(&fix.host, EVENT_LOG_STATE, &agreement).await.unwrap().unwrap();
    assert!(marker.complete);
    assert_eq!(marker.state, Some(AgreementState::NotFormed));

    let list = eventlog_ops::list(&fix.host, EventLogListInput::default()).await.expect("list");
    assert!(list.entries.iter().any(|e| e.event == LogEvent::AgreementNotFormed));
}

#[tokio::test]
async fn writer_derives_commitment_terminal_states() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();
    let comms = vec![
        make_service_commitment("c1"),
        make_service_commitment("c2"),
        make_service_commitment("c3"),
    ];
    let (agreement, _) = setup_agreed_deal(&fix, &cp, comms, now).await;

    store_progress_snapshot(
        &fix,
        &agreement,
        2,
        AgreementState::Agreed,
        vec![
            make_prog("c1", CommitmentState::Failed, vec![]),
            make_prog("c2", CommitmentState::EndedUnconfirmed, vec![]),
            make_prog("c3", CommitmentState::Cancelled, vec![]),
        ],
        now + 60,
    )
    .await;

    let mut row: AgreementRow = get_row(&fix.host, AGREEMENTS, &agreement).await.unwrap().unwrap();
    row.seq = Some(2);
    row.commitments = vec![
        make_prog("c1", CommitmentState::Failed, vec![]),
        make_prog("c2", CommitmentState::EndedUnconfirmed, vec![]),
        make_prog("c3", CommitmentState::Cancelled, vec![]),
    ];
    put_row(&fix.host, AGREEMENTS, &agreement, &row).await.unwrap();

    eventlog_ops::derive_conversation(&fix.host, "conv_1", now + 60).await.expect("derive");

    let list = eventlog_ops::list(&fix.host, EventLogListInput::default()).await.expect("list");
    assert!(
        list.entries
            .iter()
            .any(|e| e.event == LogEvent::CommitmentFailed
                && e.commitment_id.as_deref() == Some("c1"))
    );
    assert!(list.entries.iter().any(|e| e.event == LogEvent::CommitmentEndedUnconfirmed
        && e.commitment_id.as_deref() == Some("c2")));
    assert!(
        list.entries.iter().any(|e| e.event == LogEvent::CommitmentCancelled
            && e.commitment_id.as_deref() == Some("c3"))
    );
}

#[tokio::test]
async fn counterparty_derives_events_after_snapshot_is_filed() {
    let cp_identity = std::sync::Arc::new(Identity::generate().expect("cp"));
    let cp_fix = fixture_with_master(cp_identity);
    let author = Identity::generate().expect("author");
    let author_did = substrate::derive_did_key(&author.public_key());
    let now = clock::now_secs();

    let comm = make_service_commitment("c1");
    let terms = sample_terms(vec![comm], now);

    crate::app::ensure_collections(&cp_fix.host).await.unwrap();
    let agr_row = AgreementRow {
        quote_record_id: "rec_deal_1".to_string(),
        formed_as: None,
        quote_id: "q_1".to_string(),
        conversation: "conv_1".to_string(),
        counterparty_did: cp_fix.master_did.clone(),
        quote_author_did: author_did.clone(),
        terms: terms.clone(),
        agreed_at_secs: Some(now),
        counterparty: None,
        quote_author: Some(ReceiptHalf {
            record_id: "h_qa".to_string(),
            envelope: "{}".to_string(),
            issuer: author_did.clone(),
            issued_at_secs: now,
        }),
        state: Some(AgreementState::Agreed),
        state_reason: None,
        cancel_reason: None,
        seq: Some(1),
        commitments: vec![make_prog("c1", CommitmentState::Fulfilled, vec!["h1".into()])],
        updated_at_secs: now,
    };
    put_row(&cp_fix.host, AGREEMENTS, "rec_deal_1", &agr_row).await.unwrap();

    let f_row = crate::app::FulfilmentsRow {
        agreement: "rec_deal_1".to_string(),
        commitment_id: "c1".to_string(),
        conversation: "conv_1".to_string(),
        counterparty: Some(ReceiptHalf {
            record_id: "h1".to_string(),
            envelope: "{}".to_string(),
            issuer: cp_fix.master_did.clone(),
            issued_at_secs: now + 5,
        }),
        quote_author: None,
        updated_at_secs: now + 5,
    };
    put_row(
        &cp_fix.host,
        crate::app::FULFILMENTS,
        &crate::app::fulfilment_ops::fulfilment_id("rec_deal_1", "c1"),
        &f_row,
    )
    .await
    .unwrap();

    store_progress_snapshot(
        &cp_fix,
        "rec_deal_1",
        1,
        AgreementState::Agreed,
        vec![make_prog("c1", CommitmentState::Fulfilled, vec!["h1".into()])],
        now + 10,
    )
    .await;

    eventlog_ops::derive_conversation(&cp_fix.host, "conv_1", now + 15).await.expect("derive");

    let list = eventlog_ops::list(&cp_fix.host, EventLogListInput::default()).await.expect("list");
    assert!(list.entries.iter().any(
        |e| e.event == LogEvent::CommitmentSettled && e.direction == Some(Direction::Received)
    ));
}

#[tokio::test]
async fn crash_on_write_event_log_state_recovers_on_next_sync() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();
    let (agreement, _) =
        setup_agreed_deal(&fix, &cp, vec![make_service_commitment("c1")], now).await;

    fix.host.crash_on_write(EVENT_LOG_STATE, &agreement);

    let res = eventlog_ops::derive_agreement(
        &fix.host,
        &fix.master_did,
        &get_row::<AgreementRow, _>(&fix.host, AGREEMENTS, &agreement).await.unwrap().unwrap(),
        now,
        false,
    )
    .await;
    assert!(res.is_err());

    fix.host.restart();

    let res2 = eventlog_ops::derive_agreement(
        &fix.host,
        &fix.master_did,
        &get_row::<AgreementRow, _>(&fix.host, AGREEMENTS, &agreement).await.unwrap().unwrap(),
        now,
        false,
    )
    .await;
    assert!(res2.is_ok());

    let list = eventlog_ops::list(&fix.host, EventLogListInput::default()).await.expect("list");
    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].event, LogEvent::AgreementFormed);
}

#[tokio::test]
async fn fail_write_event_log_does_not_fail_sync_and_recovers_next_sync() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();
    let (agreement, _) =
        setup_agreed_deal(&fix, &cp, vec![make_service_commitment("c1")], now).await;

    fix.host.fail_write(EVENT_LOG, "");

    let sync_res = sync_direct(&fix.host, "conv_1", false, now).await;
    assert!(sync_res.is_ok());

    fix.host.restart();

    let sync_res2 = sync_direct(&fix.host, "conv_1", false, now).await;
    assert!(sync_res2.is_ok());

    let entry_id = format!("{}:-:{}", agreement, LogEvent::AgreementFormed.as_str());
    let entry: Option<EventLogEntry> = get_row(&fix.host, EVENT_LOG, &entry_id).await.unwrap();
    assert!(entry.is_some(), "expected AgreementFormed written directly to event_log table");
}

#[tokio::test]
async fn eventlog_list_derives_without_sync_and_pages() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();
    let (agreement, _) = setup_agreed_deal(
        &fix,
        &cp,
        vec![make_currency_commitment("c1", &fix.master_did), make_service_commitment("c2")],
        now,
    )
    .await;

    fix.host.set_now(now + 25);
    payment_ops::acknowledge_at(
        &fix.host,
        PaymentAckInput {
            agreement: agreement.clone(),
            commitment_id: "c1".to_string(),
            observed_at_secs: None,
            method: None,
            reference: None,
            settlement_note: None,
            supersedes: None,
        },
        now + 10,
    )
    .await
    .expect("ack payment");

    let page1 = eventlog_ops::list(&fix.host, EventLogListInput { after: None, limit: Some(1) })
        .await
        .expect("list page 1");

    assert_eq!(page1.entries.len(), 1);
    assert!(page1.next.is_some());

    let cursor = page1.next.unwrap();
    let page2 =
        eventlog_ops::list(&fix.host, EventLogListInput { after: Some(cursor), limit: Some(1) })
            .await
            .expect("list page 2");

    assert_eq!(page2.entries.len(), 1);
    assert_ne!(page1.entries[0].id, page2.entries[0].id);
}

#[tokio::test]
async fn fail_read_disclosures_defers_derive_and_recovers_clean_sync() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();
    let mut comm = make_service_commitment("c1");
    comm.giver = DealRole::Counterparty;
    let (agreement, _) = setup_agreed_deal(&fix, &cp, vec![comm], now).await;

    fix.host.set_now(now + 15);
    fulfilment_ops::sign_at(
        &fix.host,
        FulfilmentSignInput {
            agreement: agreement.clone(),
            commitment_id: "c1".to_string(),
            settlement_note: Some(sample_note("All done")),
        },
        now + 15,
    )
    .await
    .expect("sign fulfilment");

    fix.host.fail_read(crate::app::disclosures::DISCLOSURES, "");

    let sync_res = sync_direct(&fix.host, "conv_1", false, now + 20).await;
    assert!(sync_res.is_ok());

    let settled_id = format!("{}:c1:{}", agreement, LogEvent::CommitmentSettled.as_str());
    let entry_before: Option<EventLogEntry> =
        get_row(&fix.host, EVENT_LOG, &settled_id).await.unwrap();
    assert!(entry_before.is_none(), "entry must not be created when disclosure read fails");

    fix.host.restart();

    let sync_res2 = sync_direct(&fix.host, "conv_1", false, now + 25).await;
    assert!(sync_res2.is_ok());

    let entry_after: EventLogEntry = get_row(&fix.host, EVENT_LOG, &settled_id)
        .await
        .unwrap()
        .expect("entry created after recovery");
    assert_eq!(entry_after.settlement_note.as_ref().unwrap().text, "All done");
}

#[tokio::test]
async fn fail_read_agreement_progress_defers_derive_and_recovers_clean_sync() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();
    let (agreement, _) =
        setup_agreed_deal(&fix, &cp, vec![make_service_commitment("c1")], now).await;

    fix.host.set_now(now + 50);
    store_progress_snapshot(
        &fix,
        &agreement,
        2,
        AgreementState::Closed,
        vec![make_prog("c1", CommitmentState::Fulfilled, vec!["h1".into()])],
        now + 50,
    )
    .await;

    let mut row: AgreementRow = get_row(&fix.host, AGREEMENTS, &agreement).await.unwrap().unwrap();
    row.state = Some(AgreementState::Closed);
    row.seq = Some(2);
    put_row(&fix.host, AGREEMENTS, &agreement, &row).await.unwrap();

    fix.host.fail_read(AGREEMENT_PROGRESS, "");

    let sync_res = sync_direct(&fix.host, "conv_1", false, now + 60).await;
    assert!(sync_res.is_ok());

    let closed_id = format!("{}:-:{}", agreement, LogEvent::AgreementClosed.as_str());
    let entry_before: Option<EventLogEntry> =
        get_row(&fix.host, EVENT_LOG, &closed_id).await.unwrap();
    assert!(entry_before.is_none(), "entry must not be created when progress read fails");

    fix.host.restart();

    let sync_res2 = sync_direct(&fix.host, "conv_1", false, now + 70).await;
    assert!(sync_res2.is_ok());

    let entry_after: EventLogEntry =
        get_row(&fix.host, EVENT_LOG, &closed_id).await.unwrap().expect("closed entry created");
    assert_eq!(entry_after.occurred_at_secs, now + 50);
}

#[tokio::test]
async fn corrected_acknowledgement_uses_snapshot_settling_half_time() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();
    let (agreement, _) = setup_agreed_deal(
        &fix,
        &cp,
        vec![make_currency_commitment("c1", &fix.master_did), make_service_commitment("c2")],
        now,
    )
    .await;

    fix.host.set_now(now + 10);
    let ack1 = payment_ops::acknowledge_at(
        &fix.host,
        PaymentAckInput {
            agreement: agreement.clone(),
            commitment_id: "c1".to_string(),
            observed_at_secs: None,
            method: None,
            reference: None,
            settlement_note: Some(sample_note("First receipt")),
            supersedes: None,
        },
        now + 10,
    )
    .await
    .expect("ack 1");

    fix.host.set_now(now + 20);
    let _ack2 = payment_ops::acknowledge_at(
        &fix.host,
        PaymentAckInput {
            agreement: agreement.clone(),
            commitment_id: "c1".to_string(),
            observed_at_secs: None,
            method: None,
            reference: None,
            settlement_note: Some(sample_note("Corrected receipt")),
            supersedes: Some(ack1.record_id),
        },
        now + 20,
    )
    .await
    .expect("ack 2");

    eventlog_ops::derive_conversation(&fix.host, "conv_1", now + 30).await.expect("derive");

    let settled_id = format!("{}:c1:{}", agreement, LogEvent::CommitmentSettled.as_str());
    let entry: EventLogEntry = get_row(&fix.host, EVENT_LOG, &settled_id).await.unwrap().unwrap();
    assert_eq!(entry.occurred_at_secs, now + 10);
    assert_eq!(entry.settlement_note.as_ref().unwrap().text, "First receipt");
}

fn sample_terms(comms: Vec<Commitment>, now: u64) -> DealTerms {
    let tag = LangTag::parse_canonical("en").expect("parse");
    DealTerms {
        currency: "EUR".to_string(),
        commitments: comms,
        home_exchange: None,
        listing_id: None,
        scope: LocalText::plain(tag.clone(), "Lesson"),
        cancellation_terms: LocalText::plain(tag.clone(), "none"),
        refund_terms: LocalText::plain(tag.clone(), "none"),
        dispute_path: LocalText::plain(tag, "none"),
        quote_expires_at_secs: now + 3600,
    }
}

struct CpSeed<'a> {
    fix: &'a super::TestFixture,
    terms: &'a DealTerms,
    author_did: &'a str,
    now: u64,
}

struct CpDeal {
    agr_id: String,
    quote_id: String,
    state: AgreementState,
    cancel_reason: Option<CancelReason>,
    state_reason: Option<String>,
    commitments: Vec<CommitmentProgress>,
    has_author_half: bool,
}

async fn seed_cp_deal(seed: &CpSeed<'_>, deal: CpDeal) {
    let CpDeal {
        agr_id,
        quote_id,
        state,
        cancel_reason,
        state_reason,
        commitments,
        has_author_half,
    } = deal;
    let fix = seed.fix;
    let now = seed.now;
    let quote_author = has_author_half.then(|| ReceiptHalf {
        record_id: format!("h_qa_{agr_id}"),
        envelope: "{}".to_string(),
        issuer: seed.author_did.to_string(),
        issued_at_secs: now,
    });
    let counterparty = (!has_author_half).then(|| ReceiptHalf {
        record_id: format!("h_cp_{agr_id}"),
        envelope: "{}".to_string(),
        issuer: fix.master_did.clone(),
        issued_at_secs: now + 30,
    });
    let seq = if state == AgreementState::NotFormed { 1 } else { 2 };
    let agr_row = AgreementRow {
        quote_record_id: agr_id.clone(),
        formed_as: None,
        quote_id,
        conversation: "conv_1".to_string(),
        counterparty_did: fix.master_did.clone(),
        quote_author_did: seed.author_did.to_string(),
        terms: seed.terms.clone(),
        agreed_at_secs: has_author_half.then_some(now),
        counterparty,
        quote_author,
        state: Some(state),
        state_reason,
        cancel_reason,
        seq: Some(seq),
        commitments: commitments.clone(),
        updated_at_secs: now + 40,
    };
    put_row(&fix.host, AGREEMENTS, &agr_id, &agr_row).await.unwrap();
    store_progress_snapshot(fix, &agr_id, seq, state, commitments, now + 40).await;
}

#[tokio::test]
async fn counterparty_derives_agreement_ends_and_failure_ends() {
    let cp_identity = std::sync::Arc::new(Identity::generate().expect("cp"));
    let cp_fix = fixture_with_master(cp_identity);
    let author = Identity::generate().expect("author");
    let author_did = substrate::derive_did_key(&author.public_key());
    let now = clock::now_secs();

    let comms = vec![
        make_service_commitment("c1"),
        make_service_commitment("c2"),
        make_service_commitment("c3"),
    ];
    let terms = sample_terms(comms, now);

    crate::app::ensure_collections(&cp_fix.host).await.unwrap();

    let seed = CpSeed { fix: &cp_fix, terms: &terms, author_did: &author_did, now };

    let agr1 = "rec_deal_ends";
    let failure_comms = vec![
        make_prog("c1", CommitmentState::Failed, vec![]),
        make_prog("c2", CommitmentState::EndedUnconfirmed, vec![]),
        make_prog("c3", CommitmentState::Cancelled, vec![]),
    ];
    seed_cp_deal(
        &seed,
        CpDeal {
            agr_id: agr1.to_string(),
            quote_id: "q_1".to_string(),
            state: AgreementState::Closed,
            cancel_reason: None,
            state_reason: None,
            commitments: failure_comms,
            has_author_half: true,
        },
    )
    .await;

    let agr2 = "rec_deal_cancelled";
    seed_cp_deal(
        &seed,
        CpDeal {
            agr_id: agr2.to_string(),
            quote_id: "q_2".to_string(),
            state: AgreementState::Cancelled,
            cancel_reason: Some(CancelReason::Other),
            state_reason: None,
            commitments: vec![],
            has_author_half: true,
        },
    )
    .await;

    let agr3 = "rec_deal_not_formed";
    seed_cp_deal(
        &seed,
        CpDeal {
            agr_id: agr3.to_string(),
            quote_id: "q_3".to_string(),
            state: AgreementState::NotFormed,
            cancel_reason: None,
            state_reason: Some("quote-superseded".to_string()),
            commitments: vec![],
            has_author_half: false,
        },
    )
    .await;

    eventlog_ops::derive_conversation(&cp_fix.host, "conv_1", now + 60).await.expect("derive");

    let list = eventlog_ops::list(&cp_fix.host, EventLogListInput::default()).await.expect("list");
    assert!(
        list.entries.iter().any(|e| e.event == LogEvent::AgreementClosed && e.agreement == agr1)
    );
    assert!(list.entries.iter().any(|e| e.event == LogEvent::CommitmentFailed
        && e.agreement == agr1
        && e.commitment_id.as_deref() == Some("c1")));
    assert!(list.entries.iter().any(|e| e.event == LogEvent::CommitmentEndedUnconfirmed
        && e.agreement == agr1
        && e.commitment_id.as_deref() == Some("c2")));
    assert!(list.entries.iter().any(|e| e.event == LogEvent::CommitmentCancelled
        && e.agreement == agr1
        && e.commitment_id.as_deref() == Some("c3")));
    assert!(
        list.entries.iter().any(|e| e.event == LogEvent::AgreementCancelled && e.agreement == agr2)
    );
    assert!(
        list.entries.iter().any(|e| e.event == LogEvent::AgreementNotFormed && e.agreement == agr3)
    );
}
