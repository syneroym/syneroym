#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The sender side of a receiver's mailbox, end to end across real
//! substrates (ADR-0026): a guest queues a call to a receiver that cannot be
//! reached, the call is sealed and deposited at the mailbox its signed
//! record names, and a receipt the receiver signed ends it.
//!
//! The receiver is a greeter deployed with a mailbox on its own node, which
//! the tests stop and start again. When it starts, its own mailbox receiver
//! pulls the message (the tests ask for the pull instead of waiting for the
//! timer). One test uses a double at the mailbox, for a receiver whose old
//! node no longer hosts the service.
//!
//! The sender asks about a deposit once a minute, so a test that needs the
//! answer waits about that long.

use std::time::Duration;

use common::{
    SubstrateNode, mailbox_client,
    mailbox_world::{ANSWER_WAIT, DEPOSIT_WAIT, KEK, Options, apply_all, world},
    outbox::{
        artifacts, dead_letter_keys, deploy_guest, guest_enqueue, publish_endpoint, wait_until,
    },
};
use rustls::crypto::ring;
use serde_json::json;
use syneroym_core::{config::MailboxRole, mailbox::codes, test_constants};
use syneroym_identity::{Identity, substrate};
use syneroym_sdk::Publication;
use tokio::time;

mod common;

/// Long enough to see two failed direct attempts when the target has no
/// mailbox. Each attempt uses the full `DEFAULT_PROXY_CALL_TIMEOUT` (30 s),
/// so two attempts can take up to 60 s after the outbox starts, plus setup.
/// 120 s gives a 2× buffer over that worst case.
const NO_MAILBOX_ATTEMPT_WAIT: Duration = Duration::from_secs(120);

/// Short sweeps so an expired deposit is noticed quickly.
fn mailbox_role(max_ttl_secs: u64) -> MailboxRole {
    MailboxRole {
        max_ttl_secs,
        visibility_secs: 2,
        sweep_interval_secs: 1,
        republish_interval_secs: 2,
        ..Default::default()
    }
}

#[tokio::test]
async fn an_opted_in_queued_call_runs_once_and_completes_the_row() {
    let mut world = world(Options::standard()).await;
    world.stop_receiver().await;

    world.enqueue("msg-1").await;
    world.wait_deposited("msg-1").await;

    // The row waits and sends nothing more directly. The receiver node comes
    // back, pulls the message, runs the method once and signs a delivery.
    world.restart_receiver().await;
    let before = common::outbox::wasm_instantiations();
    world.receiver_pulls().await;

    world.wait_outbox_empty().await;
    assert!(dead_letter_keys(world.sender(), &world.guest_did).await.is_empty());
    time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        common::outbox::wasm_instantiations() - before,
        1,
        "the receiver runs the method once"
    );
    world.teardown().await;
}

#[tokio::test]
async fn a_queued_call_to_a_method_that_did_not_opt_in_is_dead_lettered() {
    let mut world = world(Options { opted_in: false, ..Options::standard() }).await;
    world.stop_receiver().await;

    world.enqueue("msg-1").await;
    world.wait_deposited("msg-1").await;
    world.restart_receiver().await;
    world.receiver_pulls().await;

    assert!(
        wait_until(ANSWER_WAIT, || async {
            dead_letter_keys(world.sender(), &world.guest_did).await == vec!["msg-1".to_string()]
        })
        .await,
        "the signed rejection must dead-letter the call; outbox {:?}",
        world.outbox().await
    );
    let letters =
        world.sender().substrate_client.proxy_dead_letters(world.guest_did.clone()).await.unwrap();
    assert!(letters[0].last_error.contains("not_eligible"), "{}", letters[0].last_error);
    world.teardown().await;
}

#[tokio::test]
async fn an_expired_deposit_goes_back_to_direct_and_lands_once_when_the_receiver_returns() {
    // A mailbox that keeps a message for 3 seconds: the sender's week is
    // refused and sealed again shorter.
    let mut world = world(Options { max_ttl_secs: 3, ..Options::standard() }).await;
    world.stop_receiver().await;

    world.enqueue("msg-1").await;
    world.wait_deposited("msg-1").await;

    // Nobody pulls the message, so it expires. The receiver returns before
    // the sender next asks.
    time::sleep(Duration::from_secs(5)).await;
    world.restart_receiver().await;
    let before = common::outbox::wasm_instantiations();

    world.wait_outbox_empty().await;
    assert!(dead_letter_keys(world.sender(), &world.guest_did).await.is_empty());
    time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        common::outbox::wasm_instantiations() - before,
        1,
        "the receiver runs the method once"
    );
    world.teardown().await;
}

#[tokio::test]
async fn a_receiver_without_a_mailbox_is_never_deposited() {
    let _ = ring::default_provider().install_default();
    let (proxy_wasm, greeter_wasm) = artifacts().expect("wasm artifacts not built");
    let owner = Identity::generate().unwrap();
    let mut sender = SubstrateNode::builder()
        .owner(&owner)
        .configure(|c| apply_all(c, None))
        .inject_kek_bytes(KEK)
        .boot()
        .await;
    let mailbox = SubstrateNode::builder()
        .owner(&owner)
        .shared_registry(sender.registry_url())
        .configure(|c| c.roles.mailbox = Some(mailbox_role(7 * 24 * 3600)))
        .inject_kek_bytes(KEK)
        .boot()
        .await;
    let target = SubstrateNode::builder()
        .owner(&owner)
        .shared_registry(sender.registry_url())
        .inject_kek_bytes(KEK)
        .boot()
        .await;
    let target_master = Identity::generate().unwrap();
    let target_did = substrate::derive_did_key(&target_master.public_key());
    target
        .substrate_client
        .deploy_svc_wasm(
            target_did.clone(),
            vec![test_constants::GREETER_INTERFACE_NAME.to_string()],
            greeter_wasm,
            Publication::Private,
            None,
        )
        .await
        .unwrap();
    let mechanisms = target.substrate_client.lookup().await.unwrap().info.mechanisms;
    let target_node_did = target.did().to_string();
    publish_endpoint(
        &target_did,
        &target_node_did,
        mechanisms.clone(),
        &target_master,
        sender.registry_url(),
    )
    .await;
    let guest_did = deploy_guest(&mut sender, &Identity::generate().unwrap(), proxy_wasm).await;
    target.teardown().await;
    time::sleep(Duration::from_secs(1)).await;
    publish_endpoint(
        &target_did,
        &target_node_did,
        mechanisms,
        &target_master,
        sender.registry_url(),
    )
    .await;

    guest_enqueue(&sender, &guest_did, &target_did, "msg-1").await.unwrap();

    // Today's behaviour: every failed direct attempt spends an attempt.
    // Each attempt waits up to DEFAULT_PROXY_CALL_TIMEOUT (30 s) for the
    // iroh connect, so two attempts need up to 60 s just for delivery;
    // NO_MAILBOX_ATTEMPT_WAIT gives headroom for that plus CI overhead.
    let spent = wait_until(NO_MAILBOX_ATTEMPT_WAIT, || async {
        let rows = sender.substrate_client.proxy_outbox(guest_did.clone()).await.unwrap();
        rows.first().is_some_and(|row| row.attempts >= 2)
    })
    .await;
    let rows = sender.substrate_client.proxy_outbox(guest_did.clone()).await.unwrap();
    assert!(spent, "the call keeps being tried directly; outbox {rows:?}");
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].deposited, "a receiver with no mailbox is never deposited");
    mailbox.teardown().await;
    sender.teardown().await;
}

#[tokio::test]
async fn a_stopped_mailbox_keeps_the_call_on_direct_attempts() {
    let mut world = world(Options::standard()).await;
    world.stop_receiver().await;
    world.mailbox.take().expect("a separate mailbox node").teardown().await;
    time::sleep(Duration::from_secs(1)).await;

    world.enqueue("msg-1").await;
    assert!(
        wait_until(DEPOSIT_WAIT, || async { world.attempts_of("msg-1").await >= 2 }).await,
        "the call keeps being tried directly"
    );
    assert_eq!(world.outbox().await, vec![("msg-1".to_string(), false)]);
    assert!(dead_letter_keys(world.sender(), &world.guest_did).await.is_empty());

    // The mailbox returns, and the deposit is made on a later attempt.
    world.mailbox = Some(world.restart.mailbox_builder().boot().await);
    world.wait_deposited("msg-1").await;
    world.teardown().await;
}

#[tokio::test]
async fn a_sender_on_the_mailbox_host_deposits_through_the_local_path() {
    let mut world = world(Options { same_node: true, ..Options::standard() }).await;
    world.stop_receiver().await;

    world.enqueue("msg-1").await;
    world.wait_deposited("msg-1").await;

    // The message is really at the mailbox: the receiver finds it.
    world.restart_receiver().await;
    world.receiver_pulls().await;
    world.wait_outbox_empty().await;
    world.teardown().await;
}

#[tokio::test]
async fn a_receiver_whose_old_host_answers_not_hosted_here_gets_a_deposit() {
    let world = world(Options::standard()).await;
    // The receiver's node keeps running but no longer hosts the service. The
    // registry still names that node, which refuses the call before any
    // dispatch. The queue is removed with the service, so the double makes
    // one, as the node the service moved to would.
    world.receiver_node().substrate_client.undeploy(world.receiver_id(), 0).await.unwrap();
    world.register_receiver_double().await;

    world.enqueue("msg-1").await;
    world.wait_deposited("msg-1").await;
    world.teardown().await;
}

#[tokio::test]
async fn an_undeployed_service_removes_its_queue_at_a_remote_mailbox() {
    let world = world(Options::standard()).await;
    // While the service holds the queue, another instance of the member is
    // told a newer instance holds it.
    let mut client = world.mailbox_client().await;
    let (code, _) =
        mailbox_client::call(&client, "claim", json!({})).await.expect_err("the queue exists");
    assert_eq!(code, codes::STALE_INSTANCE);

    world.receiver_node().substrate_client.undeploy(world.receiver_id(), 0).await.unwrap();

    // No wait for a pass: the undeploy itself removed the queue.
    let (code, _) =
        mailbox_client::call(&client, "claim", json!({})).await.expect_err("the queue is gone");
    assert_eq!(code, codes::UNKNOWN_DESTINATION);
    client.shutdown().await.ok();
    world.teardown().await;
}

#[tokio::test]
async fn pull_now_is_for_the_node_owner_only() {
    let world = world(Options::standard()).await;
    let mut stranger = world.receiver_node().client_as(Identity::generate().unwrap());
    stranger.connect().await.expect("the stranger connects");

    let refused = stranger.mailbox_pull_now().await;

    assert!(refused.is_err(), "a stranger must not run a pass: {refused:?}");
    stranger.shutdown().await.ok();
    world.teardown().await;
}
