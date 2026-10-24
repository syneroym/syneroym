#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The life of a deposited call, end to end across real substrates
//! (ADR-0026): a sender that goes away, a receiver with no address, a
//! receiver that moves, and a queue that is full.
//!
//! The receiver that has no address stands for a receiver that was asleep
//! longer than the registry keeps its node record (2 hours). A test cannot
//! wait that long, so the record names a node that has no record.

use std::time::Duration;

use common::{
    mailbox_world::{DEPOSIT_WAIT, Options, World, world},
    outbox::{dead_letter_keys, wait_until, wasm_instantiations},
};
use tokio::time;

mod common;

/// A deploy instantiates the guest once to check it.
const DEPLOY_INSTANTIATIONS: u64 = 1;

/// Since `before`, which the test took just ahead of the receiver's deploy,
/// the guest ran `calls` times and no more, after a short wait. A deploy
/// wakes the node's mailbox receiver, so the pull can run a message before
/// the test asks for its own pass. That is why the count starts ahead of the
/// deploy.
async fn assert_ran(before: u64, calls: u64) {
    time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        wasm_instantiations() - before,
        DEPLOY_INSTANTIATIONS + calls,
        "the receiver runs each method once"
    );
}

async fn assert_no_dead_letters(world: &World) {
    assert!(dead_letter_keys(world.sender(), &world.guest_did).await.is_empty());
}

#[tokio::test]
async fn a_sender_that_was_offline_completes_its_row_when_it_returns() {
    let mut world = world(Options { registry_on_mailbox: true, ..Options::standard() }).await;
    world.stop_receiver().await;
    world.enqueue("msg-1").await;
    world.wait_deposited("msg-1").await;

    // The sender is away while the receiver comes back, pulls and signs a
    // delivery.
    world.stop_sender().await;
    let before = wasm_instantiations();
    world.restart_receiver().await;
    world.receiver_pulls().await;
    world.start_sender().await;

    world.wait_outbox_empty().await;
    assert_no_dead_letters(&world).await;
    assert_ran(before, 1).await;
    world.teardown().await;
}

#[tokio::test]
async fn a_message_waits_for_a_receiver_that_has_no_address() {
    let world = world(Options { deploy_receiver: false, ..Options::standard() }).await;
    world.register_receiver_without_address().await;

    world.enqueue("msg-1").await;

    // There is no address to try, so the call goes to the mailbox without
    // a direct attempt.
    world.wait_deposited("msg-1").await;
    assert_eq!(world.attempts_of("msg-1").await, 0, "no direct attempt was used");

    // The receiver wakes on its node: its newer certificate takes the queue.
    let before = wasm_instantiations();
    world.deploy_receiver().await;
    world.receiver_pulls().await;

    world.wait_outbox_empty().await;
    assert_no_dead_letters(&world).await;
    assert_ran(before, 1).await;
    world.teardown().await;
}

#[tokio::test]
async fn a_moved_receiver_opens_messages_sent_before_the_move() {
    let mut world = world(Options::standard()).await;
    world.stop_receiver().await;
    world.enqueue("msg-1").await;
    world.wait_deposited("msg-1").await;

    // The same master on a new node with a new directory derives the same
    // mailbox key, so it opens what was sealed for the old node.
    let before = wasm_instantiations();
    world.relocate_receiver().await;
    world.receiver_pulls().await;

    world.wait_outbox_empty().await;
    assert_no_dead_letters(&world).await;
    assert_ran(before, 1).await;
    world.teardown().await;
}

#[tokio::test]
async fn a_full_queue_keeps_the_second_call_on_direct_attempts_and_both_land_once() {
    let mut world = world(Options { max_queue_messages: Some(1), ..Options::standard() }).await;
    world.stop_receiver().await;
    world.enqueue("msg-1").await;
    world.enqueue("msg-2").await;

    // The queue holds one message. The sender is the one that holds the most
    // bytes, so the second deposit is refused and that call stays on direct
    // attempts.
    let one_deposited = |rows: Vec<(String, bool)>| rows.iter().filter(|(_, d)| *d).count() == 1;
    assert!(
        wait_until(DEPOSIT_WAIT, || async { one_deposited(world.outbox().await) }).await,
        "exactly one call must be deposited; outbox {:?}",
        world.outbox().await
    );
    let rows = world.outbox().await;
    let waiting = rows.iter().find(|(_, deposited)| !deposited).expect("one call is not deposited");
    assert!(
        wait_until(DEPOSIT_WAIT, || async { world.attempts_of(&waiting.0).await >= 2 }).await,
        "the refused call keeps being tried directly"
    );
    assert!(one_deposited(world.outbox().await), "the queue still holds one call only");

    // The refused call keeps trying directly while the receiver boots. A node
    // that has no key yet answers that it is locked, which is not final, so
    // the call waits and lands once the node is ready.
    let before = wasm_instantiations();
    world.restart_receiver().await;
    world.receiver_pulls().await;

    world.wait_outbox_empty().await;
    assert_no_dead_letters(&world).await;
    assert_ran(before, 2).await;
    world.teardown().await;
}
