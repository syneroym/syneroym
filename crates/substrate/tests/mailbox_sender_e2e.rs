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

use std::{path::PathBuf, time::Duration};

use common::{
    SubstrateNode, mailbox_client,
    mailbox_receiver::MailboxReceiver,
    outbox::{
        DELIVERY_ATTEMPT_BUDGET, artifacts, dead_letter_keys, deploy_guest, guest_enqueue,
        outbox_keys, publish_endpoint, wait_until,
    },
};
use rustls::crypto::ring;
use serde_json::json;
use syneroym_core::{
    config::{MailboxRole, SubstrateConfig},
    mailbox::codes,
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
    receiver_node: Option<SubstrateNode>,
    /// What a node needs to come back: the same identity and directory. Each
    /// boot gets a new builder, so it gets new ports. A reused builder would
    /// dial the old ports again, which another test may hold by then.
    restart: Restart,
    receiver: MailboxReceiver,
    guest_did: String,
    greeter_wasm: Vec<u8>,
    opted_in: bool,
    _dirs: Vec<TempDir>,
}

struct Restart {
    owner: Identity,
    registry_url: String,
    mailbox_role: MailboxRole,
    mailbox_dir: Option<PathBuf>,
    receiver_dir: PathBuf,
}

impl Restart {
    fn mailbox_builder(&self) -> common::NodeBuilder {
        let role = self.mailbox_role.clone();
        SubstrateNode::builder()
            .owner(&self.owner)
            .base_path(self.mailbox_dir.as_ref().expect("a separate mailbox node"))
            .shared_registry(&self.registry_url)
            .configure(move |c| c.roles.mailbox = Some(role.clone()))
            .inject_kek_bytes(KEK)
    }

    fn receiver_builder(&self) -> common::NodeBuilder {
        SubstrateNode::builder()
            .owner(&self.owner)
            .base_path(&self.receiver_dir)
            .shared_registry(&self.registry_url)
            .inject_kek_bytes(KEK)
    }
}

struct Options {
    max_ttl_secs: u64,
    /// The sending guest lives on the mailbox node.
    same_node: bool,
    /// The greeter lists its method for delivery from the mailbox.
    opted_in: bool,
}

impl Options {
    fn standard() -> Self {
        Self { max_ttl_secs: 7 * 24 * 3600, same_node: false, opted_in: true }
    }
}

/// The greeter method the sending guest calls, as a service lists it for
/// delivery from the mailbox.
fn greet_entry() -> String {
    format!("{}#greet", test_constants::GREETER_INTERFACE_NAME)
}

async fn world(options: Options) -> World {
    let _ = ring::default_provider().install_default();
    let (proxy_wasm, greeter_wasm) =
        artifacts().expect("proxy-test/greeter wasm artifacts not built");
    let owner = Identity::generate().unwrap();
    let role = mailbox_role(options.max_ttl_secs);

    let sender_role = options.same_node.then(|| role.clone());
    let mut sender = SubstrateNode::builder()
        .owner(&owner)
        .configure(move |c| apply_all(c, sender_role.as_ref()))
        .inject_kek_bytes(KEK)
        .boot()
        .await;
    let registry_url = sender.registry_url().to_string();

    let mailbox_dir = (!options.same_node).then(|| tempfile::tempdir().unwrap());
    let receiver_dir = tempfile::tempdir().unwrap();
    let restart = Restart {
        owner,
        registry_url,
        mailbox_role: role,
        mailbox_dir: mailbox_dir.as_ref().map(|dir| dir.path().to_path_buf()),
        receiver_dir: receiver_dir.path().to_path_buf(),
    };
    let mailbox = match options.same_node {
        true => None,
        false => Some(restart.mailbox_builder().boot().await),
    };
    let receiver_node = restart.receiver_builder().boot().await;
    let dirs: Vec<TempDir> = mailbox_dir.into_iter().chain([receiver_dir]).collect();

    let caller_master = Identity::generate().unwrap();
    let guest_did = deploy_guest(&mut sender, &caller_master, proxy_wasm).await;

    let world = World {
        sender,
        mailbox,
        receiver_node: Some(receiver_node),
        restart,
        receiver: MailboxReceiver::new(),
        guest_did,
        greeter_wasm,
        opted_in: options.opted_in,
        _dirs: dirs,
    };
    world.deploy_receiver().await;
    // The receiver's own pass makes the queue the sender deposits into.
    world.receiver_pulls().await;
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

    /// The receiver's greeter, public, with a mailbox in its record. It lists
    /// `greet` for delivery from the mailbox unless the test says not.
    async fn deploy_receiver(&self) {
        let entry = greet_entry();
        let methods: Vec<&str> = if self.opted_in { vec![entry.as_str()] } else { Vec::new() };
        self.receiver
            .deploy_with_methods(
                self.receiver_node(),
                self.mailbox_node().did(),
                test_constants::GREETER_INTERFACE_NAME,
                self.greeter_wasm.clone(),
                &methods,
            )
            .await;
    }

    /// One pass of the receiver node's mailbox receiver, waited for. The
    /// receiver's queue is registered at the end of it.
    async fn receiver_pulls(&self) {
        let summary = self
            .receiver_node()
            .substrate_client
            .mailbox_pull_now()
            .await
            .expect("security.mailbox-pull-now must answer");
        assert_eq!(summary.blocked, 0, "{summary:?}");
        assert_eq!(summary.registered, 1, "{summary:?}");
    }

    async fn mailbox_client(&self) -> SyneroymClient {
        self.receiver.client(self.mailbox_node()).await
    }

    /// The double registers the queue itself, for a receiver whose node no
    /// longer hosts the service.
    async fn register_receiver_double(&self) {
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
        self.receiver_node = Some(self.restart.receiver_builder().boot().await);
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

    /// How many attempts the outbox row for `key` has used.
    async fn attempts_of(&self, key: &str) -> u32 {
        self.sender
            .substrate_client
            .proxy_outbox(self.guest_did.clone())
            .await
            .expect("proxy-outbox failed")
            .into_iter()
            .find(|row| row.idempotency_key == key)
            .map_or(0, |row| row.attempts)
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
async fn a_queued_call_to_a_method_that_did_not_opt_in_is_dead_lettered() {
    let mut world = world(Options { opted_in: false, ..Options::standard() }).await;
    world.stop_receiver().await;

    world.enqueue("msg-1").await;
    world.wait_deposited("msg-1").await;
    world.restart_receiver().await;
    world.receiver_pulls().await;

    assert!(
        wait_until(ANSWER_WAIT, || async {
            dead_letter_keys(&world.sender, &world.guest_did).await == vec!["msg-1".to_string()]
        })
        .await,
        "the signed rejection must dead-letter the call; outbox {:?}",
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
    assert!(dead_letter_keys(&world.sender, &world.guest_did).await.is_empty());

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
