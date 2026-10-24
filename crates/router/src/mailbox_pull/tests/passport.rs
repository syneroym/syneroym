//! The child token of the owner's mailbox passport: how the puller sends it,
//! and what it does when the mailbox refuses it.

use syneroym_core::config::MailboxRole;
use syneroym_identity::Identity;
use syneroym_mailbox_format::{DEDICATED_MAX_TTL_SECS, OPEN_MAX_TTL_SECS};
use syneroym_passport::facts;
use tokio::time::{self, Duration};

use super::helpers::*;

fn ttl_of_fact(node: &PullNode) -> u64 {
    node.registry.mailbox_of(&node.service_id).unwrap().max_ttl_secs
}

#[tokio::test]
async fn the_child_token_is_sent_and_raises_the_queue_limit() {
    let role = MailboxRole { max_queue_messages: 1, ..MailboxRole::default() };
    let methods = vec!["greeter#greet".to_string()];
    let node = pull_node_with(Setup { mailbox_role: role, methods, ..Setup::default() }).await;
    node.store_child_token(Some(node.child_token(&[(facts::MAX_MESSAGES, 3)])));
    node.pass().await;

    let sender = SenderMember::new();
    for i in 0..3 {
        node.deposit(&sender, &with_key(&format!("key-{i}"))).await;
    }

    assert_eq!(node.mailbox.register_tokens(), [true]);
    let summary = node.pass().await;
    assert_eq!(summary.delivered, 3, "{summary:?}");
}

#[tokio::test]
async fn a_service_without_a_token_registers_without_one() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    assert_eq!(node.mailbox.register_tokens(), [false]);
}

#[tokio::test]
async fn a_refused_token_registers_again_without_it_in_the_same_pass() {
    let node = pull_node(&["greeter#greet"]).await;
    let stranger = Identity::generate().unwrap();
    node.store_child_token(Some(node.child_token_from(&stranger, &[])));

    let summary = node.pass().await;

    assert_eq!(node.mailbox.register_tokens(), [true, false]);
    assert_eq!(summary.registered, 1, "the queue lives with the open limits");
    let sender = SenderMember::new();
    node.deposit(&sender, &greet()).await;
    assert_eq!(node.pass().await.delivered, 1);
}

#[tokio::test]
async fn a_refused_token_is_not_sent_again_until_it_changes() {
    let node = pull_node(&["greeter#greet"]).await;
    let stranger = Identity::generate().unwrap();
    node.store_child_token(Some(node.child_token_from(&stranger, &[])));
    node.pass().await;
    node.pass().await;
    assert_eq!(node.mailbox.count("register"), 2, "no register on the second pass");

    node.store_child_token(Some(node.child_token(&[(facts::MAX_MESSAGES, 9)])));
    node.pass().await;

    assert_eq!(node.mailbox.register_tokens(), [true, false, true]);
}

#[tokio::test(start_paused = true)]
async fn a_refused_token_is_tried_again_after_an_hour() {
    let node = pull_node(&["greeter#greet"]).await;
    let stranger = Identity::generate().unwrap();
    node.store_child_token(Some(node.child_token_from(&stranger, &[])));
    node.pass().await;

    time::advance(Duration::from_secs(3601)).await;
    node.pass().await;

    assert_eq!(node.mailbox.register_tokens(), [true, false, true, false]);
}

#[tokio::test]
async fn removing_the_token_registers_again_without_it() {
    let node = pull_node(&["greeter#greet"]).await;
    node.store_child_token(Some(node.child_token(&[])));
    node.pass().await;

    node.store_child_token(None);
    node.pass().await;

    assert_eq!(node.mailbox.register_tokens(), [true, false]);
}

#[tokio::test]
async fn a_locked_vault_stops_the_pass_before_a_register() {
    let node = pull_node(&["greeter#greet"]).await;
    *node.vault.mode.lock().unwrap() = VaultMode::Locked;

    let summary = node.pass().await;

    assert_eq!(summary.blocked, 1);
    assert_eq!(node.mailbox.count("register"), 0);
}

#[tokio::test]
async fn a_learned_lifetime_is_cut_to_the_passport_cap_with_a_token() {
    let node = pull_node(&["greeter#greet"]).await;
    node.store_child_token(Some(
        node.child_token(&[(facts::MAX_TTL_SECS, DEDICATED_MAX_TTL_SECS)]),
    ));
    node.mailbox.answer_register_ttl(u64::MAX);

    node.pass().await;

    assert_eq!(ttl_of_fact(&node), DEDICATED_MAX_TTL_SECS);
}

#[tokio::test]
async fn a_learned_lifetime_is_cut_to_the_open_limit_without_a_token() {
    let node = pull_node(&["greeter#greet"]).await;
    node.mailbox.answer_register_ttl(DEDICATED_MAX_TTL_SECS);

    node.pass().await;

    assert_eq!(ttl_of_fact(&node), OPEN_MAX_TTL_SECS);
}
