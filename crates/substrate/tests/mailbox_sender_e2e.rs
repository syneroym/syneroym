#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The sender side of a receiver's mailbox, end to end across real
//! substrates (ADR-0026): a guest queues a call to a receiver that cannot be
//! reached, the call is sealed and deposited at the mailbox its signed
//! record names, and a receipt the receiver signed ends it.
//!
//! The receiver is a greeter deployed with a mailbox on its own node, which
//! the tests then stop. A test double plays the receiver at the mailbox,
//! because the substrate side of receiving is not part of this slice.
//!
//! The sender asks about a deposit once a minute (the receiver pulls about
//! that often), so a test that needs the answer waits about that long.

use std::time::Duration;

use common::{
    SubstrateNode,
    mailbox_receiver::MailboxReceiver,
    outbox::{
        DELIVERY_ATTEMPT_BUDGET, artifacts, dead_letter_keys, deploy_guest, guest_enqueue,
        outbox_keys, publish_endpoint, wait_until,
    },
};
use rustls::crypto::ring;
use syneroym_core::{
    config::{MailboxRole, SubstrateConfig},
    test_constants,
};
use syneroym_identity::{Identity, substrate};
use syneroym_sdk::{Publication, SyneroymClient};
use tempfile::TempDir;
use tokio::time;

mod common;

const KEK: [u8; 32] = [0xcd; 32];
/// Long enough for the sender's worker to deposit, which needs the 5 second
/// direct attempt first.
const DEPOSIT_WAIT: Duration = Duration::from_secs(60);
/// The sender asks about a deposit once a minute.
const ANSWER_WAIT: Duration = Duration::from_secs(150);

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

fn apply_all(config: &mut SubstrateConfig, role: Option<&MailboxRole>) {
    config.roles.app_sandbox = Some(common::outbox::fast_queue_role(DELIVERY_ATTEMPT_BUDGET));
    config.retry.max_attempts = 1;
    if let Some(role) = role {
        config.roles.mailbox = Some(role.clone());
    }
}

struct World {
    /// Hosts the registry and the sending guest. Also the mailbox when
    /// `mailbox` is `None`.
    sender: SubstrateNode,
    mailbox: Option<SubstrateNode>,
    mailbox_builder: Option<common::NodeBuilder>,
    receiver_node: Option<SubstrateNode>,
    receiver_builder: common::NodeBuilder,
    receiver: MailboxReceiver,
    guest_did: String,
    greeter_wasm: Vec<u8>,
    _dirs: Vec<TempDir>,
}

struct Options {
    max_ttl_secs: u64,
    /// The sending guest lives on the mailbox node.
    same_node: bool,
}

impl Options {
    fn standard() -> Self {
        Self { max_ttl_secs: 7 * 24 * 3600, same_node: false }
    }
}

async fn world(options: Options) -> World {
    let _ = ring::default_provider().install_default();
    let (proxy_wasm, greeter_wasm) =
        artifacts().expect("proxy-test/greeter wasm artifacts not built");
    let owner = Identity::generate().unwrap();
    let role = mailbox_role(options.max_ttl_secs);
    let mut dirs = Vec::new();

    let sender_role = options.same_node.then(|| role.clone());
    let mut sender = SubstrateNode::builder()
        .owner(&owner)
        .configure(move |c| apply_all(c, sender_role.as_ref()))
        .inject_kek_bytes(KEK)
        .boot()
        .await;
    let registry_url = sender.registry_url().to_string();

    let (mailbox, mailbox_builder) = if options.same_node {
        (None, None)
    } else {
        let dir = tempfile::tempdir().unwrap();
        let mailbox_role = role.clone();
        let builder = SubstrateNode::builder()
            .owner(&owner)
            .base_path(dir.path())
            .shared_registry(&registry_url)
            .configure(move |c| c.roles.mailbox = Some(mailbox_role.clone()))
            .inject_kek_bytes(KEK);
        dirs.push(dir);
        (Some(builder.clone().boot().await), Some(builder))
    };

    let receiver_dir = tempfile::tempdir().unwrap();
    let receiver_builder = SubstrateNode::builder()
        .owner(&owner)
        .base_path(receiver_dir.path())
        .shared_registry(&registry_url)
        .inject_kek_bytes(KEK);
    dirs.push(receiver_dir);
    let receiver_node = receiver_builder.clone().boot().await;

    let caller_master = Identity::generate().unwrap();
    let guest_did = deploy_guest(&mut sender, &caller_master, proxy_wasm).await;

    let world = World {
        sender,
        mailbox,
        mailbox_builder,
        receiver_node: Some(receiver_node),
        receiver_builder,
        receiver: MailboxReceiver::new(),
        guest_did,
        greeter_wasm,
        _dirs: dirs,
    };
    world.deploy_receiver().await;
    world.register_receiver().await;
    world
}

impl World {
    fn mailbox_node(&self) -> &SubstrateNode {
        self.mailbox.as_ref().unwrap_or(&self.sender)
    }

    fn receiver_node(&self) -> &SubstrateNode {
        self.receiver_node.as_ref().expect("the receiver node is running")
    }

    fn receiver_id(&self) -> String {
        self.receiver.service_id()
    }

    /// The receiver's greeter, public, with a mailbox in its record.
    async fn deploy_receiver(&self) {
        self.receiver
            .deploy(
                self.receiver_node(),
                self.mailbox_node().did(),
                test_constants::GREETER_INTERFACE_NAME,
                self.greeter_wasm.clone(),
            )
            .await;
    }

    async fn mailbox_client(&self) -> SyneroymClient {
        self.receiver.client(self.mailbox_node()).await
    }

    async fn register_receiver(&self) {
        let mut client = self.mailbox_client().await;
        self.receiver
            .register(&client, self.receiver_node().did(), self.mailbox_node().did())
            .await;
        client.shutdown().await.ok();
    }

    /// The receiver node goes down. Its record stays in the registry, so a
    /// sender resolves an address and then fails to connect.
    async fn stop_receiver(&mut self) {
        self.receiver_node.take().expect("the receiver node is running").teardown().await;
        time::sleep(Duration::from_secs(1)).await;
    }

    /// The receiver node comes back: same identity and directory, new
    /// ports, and its greeter deployed again.
    async fn restart_receiver(&mut self) {
        self.receiver_node = Some(self.receiver_builder.clone().boot().await);
        self.deploy_receiver().await;
    }

    async fn enqueue(&self, key: &str) {
        guest_enqueue(&self.sender, &self.guest_did, &self.receiver_id(), key)
            .await
            .expect("the enqueue must be accepted for delivery");
    }

    /// Whether the guest's outbox holds exactly `key`, and whether it waits
    /// at a mailbox.
    async fn outbox(&self) -> Vec<(String, bool)> {
        self.sender
            .substrate_client
            .proxy_outbox(self.guest_did.clone())
            .await
            .expect("proxy-outbox failed")
            .into_iter()
            .map(|row| (row.idempotency_key, row.deposited))
            .collect()
    }

    async fn is_deposited(&self, key: &str) -> bool {
        self.outbox().await.contains(&(key.to_string(), true))
    }

    async fn wait_deposited(&self, key: &str) {
        assert!(
            wait_until(DEPOSIT_WAIT, || async { self.is_deposited(key).await }).await,
            "the call must wait at the mailbox; outbox {:?}",
            self.outbox().await
        );
    }

    async fn wait_outbox_empty(&self) {
        assert!(
            wait_until(ANSWER_WAIT, || async {
                outbox_keys(&self.sender, &self.guest_did).await.is_empty()
            })
            .await,
            "the call must leave the outbox; it holds {:?}",
            self.outbox().await
        );
    }

    async fn teardown(mut self) {
        if let Some(node) = self.receiver_node.take() {
            node.teardown().await;
        }
        if let Some(node) = self.mailbox.take() {
            node.teardown().await;
        }
        self.sender.teardown().await;
    }
}

#[tokio::test]
async fn a_queued_call_to_a_stopped_receiver_waits_at_its_mailbox_until_a_signed_delivery() {
    let mut world = world(Options::standard()).await;
    world.stop_receiver().await;

    world.enqueue("msg-1").await;
    world.wait_deposited("msg-1").await;

    // The row waits and sends nothing more directly; the receiver finds the
    // message at its mailbox and says it delivered it.
    let mut client = world.mailbox_client().await;
    assert_eq!(world.receiver.deliver_all(&client).await, 1);
    client.shutdown().await.ok();

    world.wait_outbox_empty().await;
    assert!(dead_letter_keys(&world.sender, &world.guest_did).await.is_empty());
    world.teardown().await;
}

#[tokio::test]
async fn a_signed_not_eligible_rejection_dead_letters_the_queued_call() {
    let mut world = world(Options::standard()).await;
    world.stop_receiver().await;

    world.enqueue("msg-1").await;
    world.wait_deposited("msg-1").await;
    let mut client = world.mailbox_client().await;
    assert_eq!(world.receiver.reject_all_as_not_eligible(&client).await, 1);
    client.shutdown().await.ok();

    assert!(
        wait_until(ANSWER_WAIT, || async {
            dead_letter_keys(&world.sender, &world.guest_did).await == vec!["msg-1".to_string()]
        })
        .await,
        "the rejection must dead-letter the call; outbox {:?}",
        world.outbox().await
    );
    let letters =
        world.sender.substrate_client.proxy_dead_letters(world.guest_did.clone()).await.unwrap();
    assert!(letters[0].last_error.contains("not_eligible"), "{}", letters[0].last_error);
    world.teardown().await;
}

#[tokio::test]
async fn an_expired_deposit_goes_back_to_direct_and_lands_once_when_the_receiver_returns() {
    // A mailbox that keeps a message for 3 seconds: the sender's week is
    // refused and sealed again shorter.
    let mut world = world(Options { max_ttl_secs: 3, same_node: false }).await;
    world.stop_receiver().await;

    world.enqueue("msg-1").await;
    world.wait_deposited("msg-1").await;

    // Nobody pulls the message, so it expires. The receiver returns before
    // the sender next asks.
    time::sleep(Duration::from_secs(5)).await;
    world.restart_receiver().await;
    let before = common::outbox::wasm_instantiations();

    world.wait_outbox_empty().await;
    assert!(dead_letter_keys(&world.sender, &world.guest_did).await.is_empty());
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
    time::sleep(Duration::from_secs(15)).await;

    let rows = sender.substrate_client.proxy_outbox(guest_did.clone()).await.unwrap();
    assert_eq!(rows.len(), 1, "the call keeps waiting for direct delivery");
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
    time::sleep(Duration::from_secs(20)).await;
    assert_eq!(world.outbox().await, vec![("msg-1".to_string(), false)]);
    assert!(dead_letter_keys(&world.sender, &world.guest_did).await.is_empty());

    // The mailbox returns, and the deposit is made on a later attempt.
    world.mailbox = Some(world.mailbox_builder.clone().unwrap().boot().await);
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
    let mut client = world.mailbox_client().await;
    assert_eq!(world.receiver.deliver_all(&client).await, 1);
    client.shutdown().await.ok();
    world.teardown().await;
}

#[tokio::test]
async fn a_receiver_whose_old_host_answers_not_hosted_here_gets_a_deposit() {
    let world = world(Options::standard()).await;
    // The receiver's node keeps running but no longer hosts the service. The
    // registry still names that node, which refuses the call before any
    // dispatch.
    world.receiver_node().substrate_client.undeploy(world.receiver_id(), 0).await.unwrap();

    world.enqueue("msg-1").await;
    world.wait_deposited("msg-1").await;
    world.teardown().await;
}
