use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use syneroym_control_plane::MailboxPullControl;
use syneroym_core::{
    config::MailboxRole,
    dht_registry::SignedEndpointInfo,
    mailbox::{HostedMailbox, codes},
};
use syneroym_mailbox_format::{MailboxSecretKey, OPEN_MAX_TTL_SECS, Receipt};
use syneroym_rpc::RpcError;
use tokio::time;

use super::helpers::*;
use crate::handshake::now_secs;

fn fresh_record(node: &PullNode) -> SignedEndpointInfo {
    service_record(&node.svc_master, &node.service_id, &node.mailbox_did, &node.vault.key)
}

/// A deposit for the service now meets no queue.
async fn queue_is_gone(node: &PullNode) -> bool {
    let sender = SenderMember::new();
    let key = node.vault.key.public_key();
    let recipient =
        syneroym_mailbox_format::Recipient { service_id: &node.service_id, mailbox_key: &key };
    let sealed = syneroym_mailbox_format::seal(
        &greet(),
        recipient,
        syneroym_mailbox_format::Sender { instance: &sender.instance, certificate: &sender.cert },
        WEEK_SECS,
        now_secs(),
    )
    .unwrap();
    let params = serde_json::to_value(&sealed.deposit).unwrap();
    matches!(
        node.call_mailbox_as(&sender, "deposit", params).await,
        Err(RpcError::Custom(code, ..)) if code == codes::UNKNOWN_DESTINATION
    )
}

#[tokio::test]
async fn the_first_pass_registers_and_later_passes_only_claim() {
    let node = pull_node(&["greeter#greet"]).await;

    for _ in 0..3 {
        node.pass().await;
    }

    // The register is made as the service's own instance: the queue exists,
    // so a sender can deposit.
    assert_eq!(node.mailbox.count("register"), 1);
    assert_eq!(node.mailbox.count("info"), 1);
    assert_eq!(node.mailbox.count("claim"), 3);
    assert!(!queue_is_gone(&node).await);
}

#[tokio::test]
async fn a_new_stored_record_registers_again() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    *node.vault.record.lock().unwrap() = Some(fresh_record(&node));

    node.pass().await;

    assert_eq!(node.mailbox.count("register"), 2);
}

#[tokio::test]
async fn a_removed_queue_is_registered_again_in_the_same_pass() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    node.puller.release(&node.service_id, &node.mailbox_did).await;
    assert!(queue_is_gone(&node).await);

    node.pass().await;

    assert_eq!(node.mailbox.count("register"), 2);
    assert!(!queue_is_gone(&node).await);
}

#[tokio::test]
async fn an_expired_record_keeps_claiming_with_the_known_time_to_live() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    let expired = {
        let mut info = fresh_record(&node).info;
        info.not_after = now_secs() - 10;
        info.sign(&node.svc_master).unwrap()
    };
    *node.vault.record.lock().unwrap() = Some(expired);
    let sender = SenderMember::new();
    let sent = node.deposit(&sender, &greet()).await;

    let summary = node.pass().await;

    assert_eq!(summary.delivered, 1);
    assert!(matches!(node.receipt(&sender, &sent).await, Receipt::Delivered { .. }));
}

#[tokio::test]
async fn the_learned_time_to_live_is_written_to_the_fact() {
    let role = MailboxRole { max_ttl_secs: 3600, ..MailboxRole::default() };
    let node = pull_node_with(Setup { mailbox_role: role, ..Setup::default() }).await;
    assert_eq!(node.registry.mailbox_of(&node.service_id).unwrap().max_ttl_secs, WEEK_SECS);

    node.pass().await;

    assert_eq!(node.registry.mailbox_of(&node.service_id).unwrap().max_ttl_secs, 3600);
}

#[tokio::test]
async fn a_fact_changed_by_a_deploy_is_not_overwritten() {
    let role = MailboxRole { max_ttl_secs: 3600, ..MailboxRole::default() };
    let node = pull_node_with(Setup { mailbox_role: role, ..Setup::default() }).await;
    // A deploy moved the fact to another mailbox while the stored record
    // still names this one.
    let other = HostedMailbox {
        mailbox: "did:key:zOtherMailbox".to_string(),
        max_ttl_secs: WEEK_SECS,
        methods: Vec::new(),
    };
    node.registry.set_mailbox(node.service_id.clone(), other.clone()).await.unwrap();

    node.pass().await;

    assert_eq!(node.registry.mailbox_of(&node.service_id), Some(other));
}

#[tokio::test]
async fn a_fact_rewritten_with_the_same_mailbox_still_registers() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    let mut fact = node.registry.mailbox_of(&node.service_id).unwrap();
    fact.methods = vec!["greeter#other".to_string()];
    node.registry.set_mailbox(node.service_id.clone(), fact).await.unwrap();
    node.puller.release(&node.service_id, &node.mailbox_did).await;

    node.pass().await;

    assert_eq!(node.mailbox.count("register"), 2);
    assert!(!queue_is_gone(&node).await);
}

#[tokio::test]
async fn a_move_to_another_mailbox_registers_there_without_unregistering() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    let other = MailboxSecretKey::generate().unwrap();
    let elsewhere = service_record(&node.svc_master, &node.service_id, "did:key:zNowhere", &other);
    *node.vault.record.lock().unwrap() = Some(elsewhere);

    node.pass().await;

    // The new mailbox cannot be reached here; the old queue is left to the
    // control plane, so the puller never sends `unregister` itself.
    assert_eq!(node.mailbox.count("register"), 1);
    assert_eq!(node.mailbox.count("unregister"), 0);
}

#[tokio::test]
async fn an_undeploy_during_a_pass_leaves_no_queue() {
    let node = Arc::new(pull_node(&["greeter#greet"]).await);
    node.pass().await;
    // Enough messages for a second claim round.
    let sender = SenderMember::new();
    for i in 0..32 {
        node.deposit(&sender, &with_key(&format!("key-{i}"))).await;
    }
    // The control plane removes the fact and then the queue, while the pass
    // is delivering.
    let undeploy = {
        let node = node.clone();
        move || -> Pin<Box<dyn Future<Output = ()> + Send>> {
            Box::pin(async move {
                node.registry.remove_mailbox(&node.service_id).await.unwrap();
                node.puller.release(&node.service_id, &node.mailbox_did).await;
            })
        }
    };
    *node.dispatch.on_first.lock().unwrap() = Some(Box::new(undeploy));

    node.pass().await;

    assert_eq!(node.mailbox.count("register"), 1, "the pass did not register again");
    assert!(queue_is_gone(&node).await);
}

#[tokio::test]
async fn a_release_waits_for_a_register_in_flight() {
    let node = Arc::new(pull_node(&["greeter#greet"]).await);
    let (entered, proceed) = node.mailbox.hold("register");
    let pass = {
        let node = node.clone();
        tokio::spawn(async move { node.pass().await })
    };
    entered.notified().await;
    // The control plane changes the fact first, then asks for the release.
    node.registry.remove_mailbox(&node.service_id).await.unwrap();
    let release = {
        let node = node.clone();
        tokio::spawn(async move {
            node.puller.release(&node.service_id, &node.mailbox_did).await;
        })
    };
    time::sleep(Duration::from_millis(200)).await;
    assert!(!release.is_finished(), "the release waits for the register in flight");

    proceed.notify_one();
    pass.await.unwrap();
    release.await.unwrap();

    assert!(queue_is_gone(&node).await);
}

fn expired_record(node: &PullNode) -> SignedEndpointInfo {
    let mut info = fresh_record(node).info;
    info.not_after = now_secs() - 10;
    info.sign(&node.svc_master).unwrap()
}

#[tokio::test]
async fn a_huge_queue_lifetime_from_the_mailbox_is_cut_to_the_open_limit() {
    let node = pull_node(&["greeter#greet"]).await;
    node.mailbox.answer_register_ttl(u64::MAX);

    let summary = node.pass().await;

    assert_eq!(summary.registered, 1);
    let fact = node.registry.mailbox_of(&node.service_id).unwrap();
    assert_eq!(fact.max_ttl_secs, OPEN_MAX_TTL_SECS);
    let sender = SenderMember::new();
    let sent = node.deposit(&sender, &greet()).await;
    assert_eq!(node.pass().await.delivered, 1);
    assert!(matches!(node.receipt(&sender, &sent).await, Receipt::Delivered { .. }));
}

#[tokio::test]
async fn an_expired_record_is_not_sent_again_on_every_pass() {
    let node = pull_node(&["greeter#greet"]).await;
    *node.vault.record.lock().unwrap() = Some(expired_record(&node));

    let summary = node.pass().await;
    node.pass().await;

    assert_eq!(summary.registered, 0);
    assert_eq!(node.mailbox.count("register"), 1, "one register until the record changes");
    assert_eq!(node.mailbox.count("info"), 1, "the visibility time is still read");
    assert_eq!(node.mailbox.count("claim"), 2, "one claim a pass, which finds no queue");

    *node.vault.record.lock().unwrap() = Some(fresh_record(&node));
    assert_eq!(node.pass().await.registered, 1);
}

#[tokio::test]
async fn an_undeploy_while_the_lifetime_is_learned_is_not_undone() {
    let role = MailboxRole { max_ttl_secs: 3600, ..MailboxRole::default() };
    let node = Arc::new(pull_node_with(Setup { mailbox_role: role, ..Setup::default() }).await);
    // `info` runs after the register and before the lifetime is written.
    let (entered, proceed) = node.mailbox.hold("info");
    let pass = {
        let node = node.clone();
        tokio::spawn(async move { node.pass().await })
    };
    entered.notified().await;
    node.registry.remove_mailbox(&node.service_id).await.unwrap();

    proceed.notify_one();
    pass.await.unwrap();

    assert_eq!(node.registry.mailbox_of(&node.service_id), None);
}
