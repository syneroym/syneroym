#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Conversation messages to a peer that cannot be reached, end to end
//! across real substrates (ADR-0026): the message shows `deposited` while a
//! sealed copy waits at the peer's mailbox, and `delivered` once the peer
//! signs a receipt. A test double plays the peer at the mailbox.
//!
//! The first exchange between two peers is made while both are up, so a
//! session exists: first contact with an unreachable peer stays live.
//! The sender asks about a deposit once a minute, so a test that needs the
//! answer waits about that long.

use std::time::Duration;

use common::{
    SubstrateNode,
    conversation_fixture::{
        FIXTURE_INTERFACE, fixture_run, fixture_wasm, history_messages, open_conversation,
        publish_master_anchor, send_message, wait_for_state,
    },
    conversation_peer::Peer,
    mailbox_receiver::MailboxReceiver,
    roym::{fast_conversation_role, wait_until},
};
use rustls::crypto::ring;
use serde_json::json;
use syneroym_core::config::{AppSandboxRole, MailboxRole};
use syneroym_identity::Identity;
use tokio::time;

mod common;

const KEK: [u8; 32] = [0xcd; 32];
const DELIVER_BUDGET: Duration = Duration::from_secs(30);
const DEPOSIT_BUDGET: Duration = Duration::from_secs(60);
/// The sender asks about a deposit once a minute.
const ANSWER_BUDGET: Duration = Duration::from_secs(150);

fn role() -> AppSandboxRole {
    fast_conversation_role(AppSandboxRole::default().conversation_max_pending_age_secs)
}

struct World {
    /// Hosts the registry.
    a: Peer,
    mailbox: SubstrateNode,
    /// The peer with a mailbox, until it is stopped.
    b: Option<Peer>,
    b_receiver: MailboxReceiver,
    wasm: Vec<u8>,
}

async fn world() -> World {
    let _ = ring::default_provider().install_default();
    let wasm = fixture_wasm().expect("dual-build-fixture wasm artifact not built");
    let a = Peer::boot(None, role(), &wasm).await;
    let mailbox = SubstrateNode::builder()
        .owner(&Identity::generate().unwrap())
        .shared_registry(a.registry())
        .configure(|c| {
            c.roles.mailbox = Some(MailboxRole {
                visibility_secs: 2,
                sweep_interval_secs: 1,
                republish_interval_secs: 2,
                ..Default::default()
            });
        })
        .inject_kek_bytes(KEK)
        .boot()
        .await;

    // B: the fixture deployed public, with the mailbox in its record, and
    // registered at the mailbox.
    let b_receiver = MailboxReceiver::new();
    let mut b_node = SubstrateNode::builder()
        .owner(&Identity::generate().unwrap())
        .shared_registry(a.registry())
        .inject_kek_bytes(KEK)
        .configure(|c| c.roles.app_sandbox = Some(role()))
        .boot()
        .await;
    let did = b_receiver.deploy(&b_node, mailbox.did(), FIXTURE_INTERFACE, wasm.clone()).await;
    let mut client = b_receiver.client(&mailbox).await;
    b_receiver.register(&client, b_node.did(), mailbox.did()).await;
    client.shutdown().await.ok();
    // The deploy publishes B's record; B's replies need its anchor in A's
    // registry too, which `deploy` did.
    let master = Identity::from_bytes(&b_receiver.master.to_bytes());
    publish_master_anchor(&did, &master, a.registry()).await;
    let _ = &mut b_node;
    let b = Peer { node: b_node, master, did };
    World { a, mailbox, b: Some(b), b_receiver, wasm }
}

impl World {
    fn b(&self) -> &Peer {
        self.b.as_ref().expect("B is running")
    }

    async fn stop_b(&mut self) {
        self.b.take().expect("B is running").teardown().await;
        time::sleep(Duration::from_secs(1)).await;
    }

    async fn teardown(mut self) {
        if let Some(b) = self.b.take() {
            b.teardown().await;
        }
        self.mailbox.teardown().await;
        self.a.teardown().await;
    }
}

#[tokio::test]
async fn a_message_to_a_stopped_peer_shows_deposited_then_delivered() {
    let mut world = world().await;
    let conv = open_conversation(&world.a.node, &world.a.did, &world.b().did).await;
    let first = send_message(&world.a.node, &world.a.did, &conv, "first, while both are up").await;
    assert!(wait_for_state(&world.a.node, &world.a.did, &first, "delivered", DELIVER_BUDGET).await);

    world.stop_b().await;
    let message = send_message(&world.a.node, &world.a.did, &conv, "while you are away").await;
    assert!(
        wait_for_state(&world.a.node, &world.a.did, &message, "deposited", DEPOSIT_BUDGET).await,
        "the message must wait at the mailbox"
    );

    // The peer finds the message at its mailbox and says it delivered it.
    let mut client = world.b_receiver.client(&world.mailbox).await;
    assert_eq!(world.b_receiver.deliver_all(&client).await, 1);
    client.shutdown().await.ok();

    assert!(
        wait_for_state(&world.a.node, &world.a.did, &message, "delivered", ANSWER_BUDGET).await,
        "a receipt the peer signed must end it"
    );
    world.teardown().await;
}

#[tokio::test]
async fn a_group_push_to_one_stopped_member_is_deposited_and_the_others_are_delivered() {
    let mut world = world().await;
    let c = Peer::boot(Some(world.a.registry()), role(), &world.wasm).await;
    let (b_did, c_did) = (world.b().did.clone(), c.did.clone());

    let created = fixture_run(&world.a.node, &world.a.did, &json!({"op": "create-group"})).await;
    let group = created["ok"]["conversation"].as_str().unwrap().to_string();
    for member in [&b_did, &c_did] {
        let added = fixture_run(
            &world.a.node,
            &world.a.did,
            &json!({"op": "add-member", "conversation": group, "member_address": member}),
        )
        .await;
        assert_eq!(added["ok"]["added"], true, "{added}");
    }
    // The group key reaches both members while they are up.
    time::sleep(Duration::from_secs(4)).await;

    world.stop_b().await;
    let message = send_message(&world.a.node, &world.a.did, &group, "to the whole group").await;

    // The member that is up gets it, and the message stays pending while the
    // other member's copy waits at its mailbox.
    assert!(
        wait_until(DELIVER_BUDGET, || async {
            history_messages(&c.node, &c_did, &group).await.iter().any(|m| m["id"] == message)
        })
        .await,
        "the member that is up must receive the push"
    );
    let mut client = world.b_receiver.client(&world.mailbox).await;
    assert!(
        wait_until(DEPOSIT_BUDGET, || async { world.b_receiver.deliver_all(&client).await >= 1 })
            .await,
        "the push to the stopped member must wait at its mailbox"
    );
    client.shutdown().await.ok();

    assert!(
        wait_for_state(&world.a.node, &world.a.did, &message, "delivered", ANSWER_BUDGET).await,
        "every member settled, so the message is delivered"
    );
    c.teardown().await;
    world.teardown().await;
}
