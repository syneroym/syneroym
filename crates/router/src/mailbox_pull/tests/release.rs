use std::{sync::Arc, time::Duration};

use syneroym_control_plane::MailboxPullControl;
use syneroym_core::mailbox::codes;
use syneroym_mailbox_format::Receipt;

use super::helpers::*;

#[tokio::test]
async fn release_removes_the_queue_at_a_local_mailbox() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    let sender = SenderMember::new();
    let sent = node.deposit(&sender, &greet()).await;

    node.puller.release(&node.service_id, &node.mailbox_did).await;

    assert_eq!(node.mailbox.count("unregister"), 1);
    assert!(matches!(node.receipt(&sender, &sent).await, Receipt::Expired { .. }));
}

#[tokio::test]
async fn release_reaches_a_remote_mailbox_as_the_instance() {
    let node = pull_node_with(Setup { remote_mailbox: true, ..Setup::default() }).await;
    let certificate = node.registry.instance_cert(&node.service_id).unwrap();
    let instance = node.node_identity.derive_service_identity(OWNER, &node.service_id);

    node.puller.release(&node.service_id, &node.mailbox_did).await;

    assert_eq!(*node.hop.methods.lock().unwrap(), ["unregister"]);
    let preambles = node.hop.preambles.lock().unwrap();
    let preamble = preambles.last().unwrap();
    assert_eq!(
        preamble.pubkey.as_deref(),
        Some(hex::encode(instance.public_key().to_bytes()).as_str()),
        "the service's instance key, not the node key"
    );
    assert_eq!(preamble.delegation.as_ref(), Some(&certificate));
}

#[tokio::test]
async fn release_without_a_certificate_sends_nothing() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    node.registry.remove_instance_cert(&node.service_id).await.unwrap();
    node.registry.remove_owner(&node.service_id).await.unwrap();

    node.puller.release(&node.service_id, &node.mailbox_did).await;

    assert_eq!(node.mailbox.count("unregister"), 0);
}

#[tokio::test]
async fn release_ignores_a_newer_instance() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    node.mailbox.fail_with("unregister", codes::STALE_INSTANCE);

    node.puller.release(&node.service_id, &node.mailbox_did).await;

    assert_eq!(node.mailbox.count("unregister"), 1);
    // The queue is still there: a sender can deposit.
    node.deposit(&SenderMember::new(), &greet()).await;
}

#[tokio::test]
async fn release_does_not_wait_for_a_running_pass() {
    let node = Arc::new(pull_node(&["greeter#greet"]).await);
    node.pass().await;
    node.deposit(&SenderMember::new(), &greet()).await;
    let (started, proceed) =
        (Arc::new(tokio::sync::Notify::new()), Arc::new(tokio::sync::Notify::new()));
    {
        let (started, proceed) = (started.clone(), proceed.clone());
        *node.dispatch.on_first.lock().unwrap() = Some(Box::new(move || {
            Box::pin(async move {
                started.notify_one();
                proceed.notified().await;
            })
        }));
    }
    let running = {
        let node = node.clone();
        tokio::spawn(async move { node.pass().await })
    };
    started.notified().await;

    let released = tokio::time::timeout(
        Duration::from_secs(5),
        node.puller.release(&node.service_id, &node.mailbox_did),
    )
    .await;

    assert!(released.is_ok(), "the release did not wait for the pass");
    proceed.notify_one();
    running.await.unwrap();
}
