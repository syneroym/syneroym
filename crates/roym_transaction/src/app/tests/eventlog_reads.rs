use syneroym_identity::Identity;
use syneroym_roym_core::clock;

use super::{
    fixture,
    settle::{make_currency_commitment, make_service_commitment, sample_note, setup_agreed_deal},
};
use crate::app::{
    eventlog_ops::{self, EventLogListInput},
    fulfilment_ops::{self, FulfilmentSignInput},
    payment_ops::{self, PaymentAckInput},
};

#[tokio::test]
async fn full_deal_path_never_reads_event_log_until_list_called() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();

    let comms =
        vec![make_service_commitment("c1"), make_currency_commitment("c2", &fix.master_did)];
    let (agreement, _) = setup_agreed_deal(&fix, &cp, comms, now).await;

    fulfilment_ops::sign_at(
        &fix.host,
        FulfilmentSignInput {
            agreement: agreement.clone(),
            commitment_id: "c1".to_string(),
            settlement_note: Some(sample_note("Lesson done")),
        },
        now + 10,
    )
    .await
    .expect("sign fulfilment");

    fix.host.set_now(now + 20);
    payment_ops::acknowledge_at(
        &fix.host,
        PaymentAckInput {
            agreement: agreement.clone(),
            commitment_id: "c2".to_string(),
            observed_at_secs: None,
            method: None,
            reference: None,
            settlement_note: Some(sample_note("Payment acked")),
            supersedes: None,
        },
        now + 20,
    )
    .await
    .expect("ack payment");

    eventlog_ops::derive_conversation(&fix.host, "conv_1", now + 25)
        .await
        .expect("derive conversation");

    let reads_during_deal = fix.host.reads();
    assert!(
        reads_during_deal.iter().all(|(coll, _)| coll != "event_log"),
        "found read of event_log during deal flow: {reads_during_deal:?}"
    );

    let list_out =
        eventlog_ops::list(&fix.host, EventLogListInput::default()).await.expect("list eventlog");
    assert!(!list_out.entries.is_empty());

    let reads_after_list = fix.host.reads();
    assert!(
        reads_after_list.iter().any(|(coll, _)| coll == "event_log"),
        "expected read of event_log when listing, but none found"
    );
}
