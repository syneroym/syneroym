use std::time::Duration;

use syneroym_core::{config::MailboxRole, mailbox::AckOutcome};

use super::helpers::*;

fn short_visibility(secs: u64) -> Setup {
    Setup {
        methods: vec!["greeter#greet".to_string()],
        mailbox_role: MailboxRole { visibility_secs: secs, ..MailboxRole::default() },
        ..Setup::default()
    }
}

async fn deposit_many(node: &PullNode, count: usize) {
    let sender = SenderMember::new();
    for i in 0..count {
        node.deposit(&sender, &with_key(&format!("key-{i}"))).await;
    }
}

#[tokio::test]
async fn acks_are_sent_every_eight_messages() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    deposit_many(&node, 20).await;

    let summary = node.pass().await;

    assert_eq!(summary.delivered, 20);
    assert_eq!(node.mailbox.count("ack"), 3, "8 + 8 + 4");
}

#[tokio::test]
async fn a_full_batch_is_followed_by_another_claim() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    deposit_many(&node, 40).await;

    let summary = node.pass().await;

    assert_eq!(summary.delivered, 40);
}

#[tokio::test]
async fn the_first_message_of_a_round_is_always_handled() {
    // The budget is one second, shorter than one delivery timeout, so only
    // the first message of the round is handled.
    let node = pull_node_with(short_visibility(10)).await;
    node.pass().await;
    deposit_many(&node, 3).await;

    let summary = node.pass().await;

    assert_eq!(summary.delivered, 1);
    assert_eq!(summary.not_acked, 2, "the rest come back after the visibility time");
}

#[tokio::test(start_paused = true)]
async fn a_round_stops_before_its_visibility_time_ends() {
    // Visibility 120 s leaves a budget of 90 s. Each delivery takes 25 s on
    // the paused clock, so after three the next one could end too late.
    let node = pull_node_with(short_visibility(120)).await;
    node.pass().await;
    deposit_many(&node, 6).await;
    *node.dispatch.delay.lock().unwrap() = Duration::from_secs(25);

    let summary = node.pass().await;

    assert_eq!(summary.delivered, 3);
    assert_eq!(summary.not_acked, 3);
    assert_eq!(node.mailbox.count("ack"), 1, "what was handled is acked before the stop");
}

#[tokio::test]
async fn a_mailbox_clock_far_ahead_or_behind_does_not_change_the_round() {
    for shift in [86_400, -86_400] {
        let node = pull_node(&["greeter#greet"]).await;
        node.pass().await;
        node.mailbox.shift_claim_clock(shift);
        deposit_many(&node, 5).await;

        let summary = node.pass().await;

        assert_eq!(summary.delivered, 5, "shift {shift}");
        assert_eq!(summary.not_acked, 0, "shift {shift}");
    }
}

#[tokio::test]
async fn every_ack_outcome_is_read() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    deposit_many(&node, 3).await;
    node.mailbox.answer_acks(vec![AckOutcome::Stored, AckOutcome::NotFound, AckOutcome::Refused]);

    let summary = node.pass().await;

    assert_eq!(summary.delivered, 1, "only a stored receipt counts");
    assert_eq!(summary.not_acked, 1, "a refused receipt leaves the message for later");
    assert_eq!(node.mailbox.count("ack"), 1);
}

#[tokio::test]
async fn a_message_is_not_counted_when_its_ack_fails() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    deposit_many(&node, 2).await;
    node.mailbox.fail_with("ack", -32603);

    let summary = node.pass().await;

    assert_eq!((summary.delivered, summary.rejected), (0, 0), "{summary:?}");
    assert_eq!(summary.not_acked, 2, "{summary:?}");
    assert_eq!(node.dispatch.call_count(), 2);
}
