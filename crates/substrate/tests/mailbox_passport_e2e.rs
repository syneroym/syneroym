#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! A mailbox passport raises the queue limits of the services of one owner,
//! end to end across real substrates (ADR-0015 A10): the mailbox admits the
//! owner's child token at `register`, keeps a lease, and falls back to its
//! open limits when the passport is denied.
//!
//! The mailbox queues two messages for a stranger. The passports below raise
//! that to five. The mailbox host is claimed by the world's owner, who is its
//! controller and so the root of its passports.

use std::time::Duration;

use clap::Parser;
use common::{
    mailbox_passport::PassportDesk,
    mailbox_receiver::MailboxReceiver,
    mailbox_world::{DEPOSIT_WAIT, Options, World, world},
    outbox::{dead_letter_keys, wait_until, wasm_instantiations},
};
use roymctl::commands::mailbox::{self, MailboxCommands};
use syneroym_passport::{PassportId, facts};
use tokio::time;

mod common;

const OPEN_MESSAGES: u32 = 2;

#[derive(Parser)]
struct MailboxCli {
    #[command(subcommand)]
    command: MailboxCommands,
}

fn options() -> Options {
    Options {
        max_queue_messages: Some(OPEN_MESSAGES),
        mailbox_controller: true,
        deploy_receiver: false,
        ..Options::standard()
    }
}

/// A world whose receiver is deployed with a child token of a passport that
/// allows `max_messages` messages in a queue.
async fn world_with_passport(max_messages: u64) -> (World, PassportDesk, PassportId) {
    let mut world = world(options()).await;
    let desk = PassportDesk::new(&world.restart.owner, world.mailbox_node().did());
    let passport = desk.passport(&[(facts::MAX_MESSAGES, max_messages)]);
    let id = PassportId::of(&passport).unwrap();
    world.set_child_token(Some(desk.child_token(&passport, &world.receiver_id())));
    world.deploy_receiver().await;
    world.receiver_pulls().await;
    (world, desk, id)
}

async fn deposited(world: &World, keys: &[&str]) -> bool {
    let rows = world.outbox().await;
    keys.iter().all(|key| rows.contains(&((*key).to_string(), true)))
}

async fn wait_deposited_all(world: &World, keys: &[&str]) {
    assert!(
        wait_until(DEPOSIT_WAIT * 2, || async { deposited(world, keys).await }).await,
        "every call must wait at the mailbox; outbox {:?}",
        world.outbox().await
    );
}

async fn assert_no_dead_letters(world: &World) {
    assert!(dead_letter_keys(world.sender(), &world.guest_did).await.is_empty());
}

#[tokio::test]
async fn a_passport_raises_the_queue_limit() {
    let (mut world, _desk, _id) = world_with_passport(5).await;
    world.stop_receiver().await;

    let keys = ["msg-1", "msg-2", "msg-3", "msg-4"];
    for key in keys {
        world.enqueue(key).await;
    }

    // Four calls wait at a mailbox that holds two for a stranger.
    wait_deposited_all(&world, &keys).await;

    // A round stops before the visibility time of the mailbox ends, so four
    // calls may need more than one pass.
    let before = wasm_instantiations();
    world.restart_receiver().await;
    for _ in 0..4 {
        world.receiver_pulls().await;
        time::sleep(Duration::from_secs(3)).await;
    }
    world.wait_outbox_empty().await;
    assert_no_dead_letters(&world).await;
    time::sleep(Duration::from_secs(3)).await;
    // One instantiation for the deploy, and one for each call.
    assert_eq!(wasm_instantiations() - before, 1 + keys.len() as u64);
    world.teardown().await;
}

#[tokio::test]
async fn max_services_admits_one_master() {
    let (world, desk, _first_id) = world_with_passport(5).await;
    // A second master under a passport that admits one service.
    let one = desk.passport(&[(facts::MAX_SERVICES, 1), (facts::MAX_MESSAGES, 5)]);
    let admitted = MailboxReceiver::new();
    let refused = MailboxReceiver::new();
    world.deploy_other_receiver(&admitted, desk.child_token(&one, &admitted.service_id())).await;
    world.deploy_other_receiver(&refused, desk.child_token(&one, &refused.service_id())).await;

    // The refused token does not stop the queue: it registers with the
    // open limits. Three queues: the first receiver, and the two others.
    let summary = world.receiver_node().substrate_client.mailbox_pull_now().await.unwrap();
    assert_eq!(summary.registered, 3, "{summary:?}");
    assert_eq!(summary.blocked, 0, "{summary:?}");

    let leases = world.mailbox_node().substrate_client.mailbox_passport_leases().await.unwrap();
    let subjects: Vec<&str> = leases.iter().map(|l| l.subject.as_str()).collect();
    assert!(subjects.contains(&admitted.service_id().as_str()), "{subjects:?}");
    assert!(!subjects.contains(&refused.service_id().as_str()), "{subjects:?}");

    let mut world = world;
    world.stop_receiver().await;
    for i in 1..=3 {
        world.enqueue_for(&admitted.service_id(), &format!("a-{i}")).await;
        world.enqueue_for(&refused.service_id(), &format!("r-{i}")).await;
    }

    // The admitted master holds all three. The other holds two, and its
    // third call keeps trying directly.
    wait_deposited_all(&world, &["a-1", "a-2", "a-3", "r-1", "r-2"]).await;
    assert!(
        wait_until(DEPOSIT_WAIT, || async { world.attempts_of("r-3").await >= 2 }).await,
        "the call refused as full keeps being tried directly"
    );
    assert!(!world.is_deposited("r-3").await, "the open limit holds two messages");
    world.teardown().await;
}

#[tokio::test]
async fn a_denied_passport_falls_back_to_the_open_limits() {
    let (mut world, _desk, id) = world_with_passport(5).await;
    let mailbox = world.mailbox_node();
    let leases = mailbox.substrate_client.mailbox_passport_leases().await.unwrap();
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0].passport_id, id);
    assert_eq!(leases[0].subject, world.receiver_id());

    world.stop_receiver().await;
    for key in ["msg-1", "msg-2", "msg-3"] {
        world.enqueue(key).await;
    }
    wait_deposited_all(&world, &["msg-1", "msg-2", "msg-3"]).await;

    // The operator denies the passport through `roymctl`, as the owner.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("identities")).unwrap();
    world.restart.owner.save_to_path(dir.path().join("identities").join("owner.key")).unwrap();
    let mailbox = world.mailbox_node();
    let argv = ["mailbox", "passport", "deny", id.as_str()];
    let cli = MailboxCli::try_parse_from(argv).unwrap();
    mailbox::handle(
        &cli.command,
        mailbox.registry_url(),
        Some(mailbox.did().to_string()),
        dir.path(),
        Some("owner"),
        None,
    )
    .await
    .expect("the owner may deny a passport");
    assert!(mailbox.substrate_client.mailbox_passport_leases().await.unwrap().is_empty());

    // The queue holds three and the open limit is two: the next call is
    // refused as full and stays on direct attempts.
    world.enqueue("msg-4").await;
    assert!(
        wait_until(DEPOSIT_WAIT, || async { world.attempts_of("msg-4").await >= 2 }).await,
        "the call refused as full keeps being tried directly"
    );
    assert!(!world.is_deposited("msg-4").await);
    world.teardown().await;
}

#[tokio::test]
async fn a_lease_survives_a_mailbox_restart() {
    let (mut world, _desk, _id) = world_with_passport(5).await;
    world.stop_receiver().await;

    // The receiver is down, so nothing registers again: the raised limit
    // comes from the lease file alone.
    world.restart_mailbox().await;

    let keys = ["msg-1", "msg-2", "msg-3"];
    for key in keys {
        world.enqueue(key).await;
    }
    wait_deposited_all(&world, &keys).await;
    world.teardown().await;
}
