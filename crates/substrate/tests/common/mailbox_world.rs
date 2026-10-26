//! The world of the mailbox end-to-end tests: a sender node with a guest that
//! queues calls, a mailbox node, and a receiver node whose greeter names that
//! mailbox in its record (ADR-0026).
//!
//! The tests stop and start the receiver. When it starts, its own mailbox
//! receiver pulls the message (the tests ask for the pull instead of waiting
//! for the timer).

#![allow(clippy::cognitive_complexity, dead_code)]

use std::{path::PathBuf, time::Duration};

use rustls::crypto::ring;
use syneroym_core::{
    config::{MailboxRole, SubstrateConfig},
    dht_registry::RegistryClient,
    test_constants,
};
use syneroym_identity::Identity;
use syneroym_sdk::SyneroymClient;
use tempfile::TempDir;
use tokio::{sync::MutexGuard, time};

use super::{
    NodeBuilder, StoppedNode, SubstrateNode,
    mailbox_client::did_of,
    mailbox_receiver::MailboxReceiver,
    outbox::{
        DELIVERY_ATTEMPT_BUDGET, artifacts, deploy_guest, fast_queue_role, guest_enqueue,
        outbox_keys, wait_until,
    },
    write_claim,
};

pub const KEK: [u8; 32] = [0xcd; 32];
/// Long enough for the sender's worker to deposit, which needs the 5 second
/// direct attempt first.
pub const DEPOSIT_WAIT: Duration = Duration::from_secs(60);
/// The sender asks about a deposit once a minute.
pub const ANSWER_WAIT: Duration = Duration::from_secs(150);

/// Short sweeps so an expired deposit is noticed quickly.
fn mailbox_role(options: &Options) -> MailboxRole {
    let mut role = MailboxRole {
        max_ttl_secs: options.max_ttl_secs,
        visibility_secs: 2,
        sweep_interval_secs: 1,
        republish_interval_secs: 2,
        ..Default::default()
    };
    if let Some(messages) = options.max_queue_messages {
        role.max_queue_messages = messages;
    }
    role
}

pub fn apply_all(config: &mut SubstrateConfig, role: Option<&MailboxRole>) {
    config.roles.app_sandbox = Some(fast_queue_role(DELIVERY_ATTEMPT_BUDGET));
    config.retry.max_attempts = 1;
    if let Some(role) = role {
        config.roles.mailbox = Some(role.clone());
    }
}

pub struct World {
    /// Hosts the registry and the sending guest, unless the mailbox node
    /// hosts the registry. Also the mailbox when `Options::same_node` is set.
    sender_node: Option<SubstrateNode>,
    stopped_sender: Option<StoppedNode>,
    pub mailbox: Option<SubstrateNode>,
    receiver_node: Option<SubstrateNode>,
    /// What a node needs to come back: the same identity and directory. Each
    /// boot gets a new builder, so it gets new ports. A reused builder would
    /// dial the old ports again, which another test may hold by then.
    pub restart: Restart,
    pub receiver: MailboxReceiver,
    pub guest_did: String,
    greeter_wasm: Vec<u8>,
    opted_in: bool,
    dirs: Vec<TempDir>,
    /// The owner's child token for the receiver's mailbox passport, shown
    /// at every deploy of the receiver.
    child_token: Option<String>,
    _serial_guard: MutexGuard<'static, ()>,
}

pub struct Restart {
    pub owner: Identity,
    pub registry_url: String,
    /// The relay of the node that hosts the registry. The other nodes use it
    /// as their home relay.
    pub relay_url: String,
    pub mailbox_role: MailboxRole,
    pub mailbox_dir: Option<PathBuf>,
    pub receiver_dir: PathBuf,
}

impl Restart {
    pub fn mailbox_builder(&self) -> NodeBuilder {
        let role = self.mailbox_role.clone();
        SubstrateNode::builder()
            .owner(&self.owner)
            .base_path(self.mailbox_dir.as_ref().expect("a separate mailbox node"))
            .shared_registry(&self.registry_url)
            .shared_relay(&self.relay_url)
            .configure(move |c| c.roles.mailbox = Some(role.clone()))
            .inject_kek_bytes(KEK)
    }

    pub fn receiver_builder(&self) -> NodeBuilder {
        SubstrateNode::builder()
            .owner(&self.owner)
            .base_path(&self.receiver_dir)
            .shared_registry(&self.registry_url)
            .shared_relay(&self.relay_url)
            .inject_kek_bytes(KEK)
    }
}

pub struct Options {
    pub max_ttl_secs: u64,
    /// The sending guest lives on the mailbox node.
    pub same_node: bool,
    /// The greeter lists its method for delivery from the mailbox.
    pub opted_in: bool,
    /// The mailbox node boots first and hosts the registry, so the sender
    /// can stop without taking the registry down with it.
    pub registry_on_mailbox: bool,
    /// Deploy the receiver's greeter at the start. A test that wants a
    /// receiver with no address leaves it out.
    pub deploy_receiver: bool,
    /// The most messages one queue holds at the mailbox.
    pub max_queue_messages: Option<u32>,
    /// The mailbox node is claimed by the world's owner, so the owner is its
    /// verified controller: the owner signs its passports, and the owner
    /// can deny them. Needs a separate mailbox node.
    pub mailbox_controller: bool,
}

impl Options {
    pub fn standard() -> Self {
        Self {
            max_ttl_secs: 7 * 24 * 3600,
            same_node: false,
            opted_in: true,
            registry_on_mailbox: false,
            deploy_receiver: true,
            max_queue_messages: None,
            mailbox_controller: false,
        }
    }
}

/// The greeter method the sending guest calls, as a service lists it for
/// delivery from the mailbox.
pub fn greet_entry() -> String {
    format!("{}#greet", test_constants::GREETER_INTERFACE_NAME)
}

async fn boot_nodes(
    options: &Options,
    owner: &Identity,
    role: &MailboxRole,
    mailbox_dir: Option<&TempDir>,
) -> (SubstrateNode, Option<SubstrateNode>) {
    let sender_role = options.same_node.then(|| role.clone());
    if options.registry_on_mailbox && !options.same_node {
        let mailbox_role = role.clone();
        let mailbox = SubstrateNode::builder()
            .owner(owner)
            .base_path(mailbox_dir.expect("a separate mailbox node").path())
            .configure(move |c| c.roles.mailbox = Some(mailbox_role.clone()))
            .inject_kek_bytes(KEK)
            .boot()
            .await;
        let sender = SubstrateNode::builder()
            .owner(owner)
            .shared_registry(mailbox.registry_url())
            .shared_relay(mailbox.relay_url())
            .configure(move |c| apply_all(c, None))
            .inject_kek_bytes(KEK)
            .boot()
            .await;
        return (sender, Some(mailbox));
    }
    let sender = SubstrateNode::builder()
        .owner(owner)
        .configure(move |c| apply_all(c, sender_role.as_ref()))
        .inject_kek_bytes(KEK)
        .boot()
        .await;
    (sender, None)
}

pub async fn world(options: Options) -> World {
    let serial_guard = super::serial_guard().await;
    let _ = ring::default_provider().install_default();
    let (proxy_wasm, greeter_wasm) =
        artifacts().expect("proxy-test/greeter wasm artifacts not built");
    let owner = Identity::generate().unwrap();
    let role = mailbox_role(&options);
    let mailbox_dir = (!options.same_node).then(|| tempfile::tempdir().unwrap());
    if options.mailbox_controller {
        let dir = mailbox_dir.as_ref().expect("a controlled mailbox is a separate node");
        write_claim(dir.path(), &owner);
    }
    let receiver_dir = tempfile::tempdir().unwrap();

    let (mut sender, hosting_mailbox) =
        boot_nodes(&options, &owner, &role, mailbox_dir.as_ref()).await;
    // The registry host also hosts the relay of the world, and no test stops
    // that node. A node that stops with its own relay leaves the relay
    // actors of its peers dialing a dead address. While such an actor waits
    // to reconnect, it does not read its send queue, and iroh then holds
    // back every other relay datagram of the dialing node: calls on
    // unrelated connections stall for up to a minute.
    let relay_url = hosting_mailbox
        .as_ref()
        .map_or_else(|| sender.relay_url(), SubstrateNode::relay_url)
        .to_string();
    let restart = Restart {
        owner,
        registry_url: sender.registry_url().to_string(),
        relay_url,
        mailbox_role: role,
        mailbox_dir: mailbox_dir.as_ref().map(|dir| dir.path().to_path_buf()),
        receiver_dir: receiver_dir.path().to_path_buf(),
    };
    let mailbox = match (hosting_mailbox, options.same_node) {
        (Some(mailbox), _) => Some(mailbox),
        (None, true) => None,
        (None, false) => Some(restart.mailbox_builder().boot().await),
    };
    let receiver_node = restart.receiver_builder().boot().await;
    let dirs: Vec<TempDir> = mailbox_dir.into_iter().chain([receiver_dir]).collect();

    let caller_master = Identity::generate().unwrap();
    let guest_did = deploy_guest(&mut sender, &caller_master, proxy_wasm).await;

    let world = World {
        sender_node: Some(sender),
        stopped_sender: None,
        mailbox,
        receiver_node: Some(receiver_node),
        restart,
        receiver: MailboxReceiver::new(),
        guest_did,
        greeter_wasm,
        opted_in: options.opted_in,
        dirs,
        child_token: None,
        _serial_guard: serial_guard,
    };
    if options.deploy_receiver {
        world.deploy_receiver().await;
        // The receiver's own pass makes the queue the sender deposits into.
        world.receiver_pulls().await;
    }
    world
}

impl World {
    pub fn sender(&self) -> &SubstrateNode {
        self.sender_node.as_ref().expect("the sender node is running")
    }

    pub fn mailbox_node(&self) -> &SubstrateNode {
        self.mailbox.as_ref().unwrap_or_else(|| self.sender())
    }

    pub fn receiver_node(&self) -> &SubstrateNode {
        self.receiver_node.as_ref().expect("the receiver node is running")
    }

    pub fn receiver_id(&self) -> String {
        self.receiver.service_id()
    }

    /// The receiver's greeter is deployed with this child token from now on.
    pub fn set_child_token(&mut self, child_token: Option<String>) {
        self.child_token = child_token;
    }

    /// Another receiver member on the receiver node, with the same greeter
    /// and the same mailbox, deployed with `child_token`.
    pub async fn deploy_other_receiver(&self, other: &MailboxReceiver, child_token: String) {
        let entry = greet_entry();
        other
            .deploy_with_passport(
                self.receiver_node(),
                self.mailbox_node().did(),
                test_constants::GREETER_INTERFACE_NAME,
                self.greeter_wasm.clone(),
                &[entry.as_str()],
                Some(child_token),
            )
            .await;
    }

    /// The receiver's greeter, public, with a mailbox in its record. It lists
    /// `greet` for delivery from the mailbox unless the test says not.
    pub async fn deploy_receiver(&self) {
        let entry = greet_entry();
        let methods: Vec<&str> = if self.opted_in { vec![entry.as_str()] } else { Vec::new() };
        self.receiver
            .deploy_with_passport(
                self.receiver_node(),
                self.mailbox_node().did(),
                test_constants::GREETER_INTERFACE_NAME,
                self.greeter_wasm.clone(),
                &methods,
                self.child_token.clone(),
            )
            .await;
    }

    /// One pass of the receiver node's mailbox receiver, waited for. The
    /// receiver's queue is registered at the end of it.
    pub async fn receiver_pulls(&self) {
        let summary = self
            .receiver_node()
            .substrate_client
            .mailbox_pull_now()
            .await
            .expect("security.mailbox-pull-now must answer");
        assert_eq!(summary.blocked, 0, "{summary:?}");
        assert_eq!(summary.registered, 1, "{summary:?}");
    }

    pub async fn mailbox_client(&self) -> SyneroymClient {
        self.receiver.client(self.mailbox_node()).await
    }

    /// The double registers the queue itself, for a receiver whose node no
    /// longer hosts the service.
    pub async fn register_receiver_double(&self) {
        let mut client = self.mailbox_client().await;
        self.receiver
            .register(&client, self.receiver_node().did(), self.mailbox_node().did())
            .await;
        client.shutdown().await.ok();
    }

    /// A receiver that has no address: its record names a node that has no
    /// record anywhere, as a node that has been asleep too long does. The
    /// record is in the registry and the queue exists at the mailbox.
    pub async fn register_receiver_without_address(&self) {
        let asleep = did_of(&Identity::generate().unwrap());
        let registry = RegistryClient::new(false, Some(self.restart.registry_url.clone()));
        registry
            .publish_master_anchor(&self.receiver_id(), vec![], None, &self.receiver.master, true)
            .await
            .expect("failed to publish the receiver master's anchor");
        let record = self.receiver.record(&asleep, self.mailbox_node().did());
        registry.register(&record, false).await.expect("the registry must admit the record");
        let mut client = self.mailbox_client().await;
        self.receiver.register(&client, &asleep, self.mailbox_node().did()).await;
        client.shutdown().await.ok();
    }

    /// The receiver node goes down. Its record stays in the registry, so a
    /// sender resolves an address and then fails to connect.
    pub async fn stop_receiver(&mut self) {
        self.receiver_node.take().expect("the receiver node is running").teardown().await;
        time::sleep(Duration::from_secs(1)).await;
    }

    /// The receiver node comes back: same identity and directory, new
    /// ports, and its greeter deployed again.
    pub async fn restart_receiver(&mut self) {
        self.receiver_node = Some(self.restart.receiver_builder().boot().await);
        self.deploy_receiver().await;
    }

    /// The receiver service moves to a new node with a new directory. It
    /// keeps its master, so it derives the same mailbox key.
    pub async fn relocate_receiver(&mut self) {
        let dir = tempfile::tempdir().unwrap();
        self.restart.receiver_dir = dir.path().to_path_buf();
        self.dirs.push(dir);
        self.receiver_node = Some(self.restart.receiver_builder().boot().await);
        self.deploy_receiver().await;
    }

    /// The sender node goes down. Its directory and identity are kept.
    pub async fn stop_sender(&mut self) {
        let node = self.sender_node.take().expect("the sender node is running");
        self.stopped_sender = Some(node.stop().await);
        time::sleep(Duration::from_secs(1)).await;
    }

    /// The sender node comes back with the same directory, identity and
    /// ports.
    pub async fn start_sender(&mut self) {
        let stopped = self.stopped_sender.take().expect("the sender node is stopped");
        self.sender_node = Some(stopped.boot().await);
    }

    pub async fn enqueue(&self, key: &str) {
        self.enqueue_for(&self.receiver_id(), key).await;
    }

    /// A queued call from the sending guest to `receiver_id`.
    pub async fn enqueue_for(&self, receiver_id: &str, key: &str) {
        guest_enqueue(self.sender(), &self.guest_did, receiver_id, key)
            .await
            .expect("the enqueue must be accepted for delivery");
    }

    /// The mailbox node restarts with its directory, identity and ports.
    pub async fn restart_mailbox(&mut self) {
        let node = self.mailbox.as_mut().expect("a separate mailbox node");
        node.restart_in_place().await;
        time::sleep(Duration::from_secs(1)).await;
    }

    /// The keys in the guest's outbox, and whether each waits at a mailbox.
    pub async fn outbox(&self) -> Vec<(String, bool)> {
        self.sender()
            .substrate_client
            .proxy_outbox(self.guest_did.clone())
            .await
            .expect("proxy-outbox failed")
            .into_iter()
            .map(|row| (row.idempotency_key, row.deposited))
            .collect()
    }

    /// How many attempts the outbox row for `key` has used.
    pub async fn attempts_of(&self, key: &str) -> u32 {
        self.sender()
            .substrate_client
            .proxy_outbox(self.guest_did.clone())
            .await
            .expect("proxy-outbox failed")
            .into_iter()
            .find(|row| row.idempotency_key == key)
            .map_or(0, |row| row.attempts)
    }

    pub async fn is_deposited(&self, key: &str) -> bool {
        self.outbox().await.contains(&(key.to_string(), true))
    }

    pub async fn wait_deposited(&self, key: &str) {
        assert!(
            wait_until(DEPOSIT_WAIT, || async { self.is_deposited(key).await }).await,
            "the call must wait at the mailbox; outbox {:?}",
            self.outbox().await
        );
    }

    pub async fn wait_outbox_empty(&self) {
        assert!(
            wait_until(ANSWER_WAIT, || async {
                outbox_keys(self.sender(), &self.guest_did).await.is_empty()
            })
            .await,
            "the call must leave the outbox; it holds {:?}",
            self.outbox().await
        );
    }

    pub async fn teardown(mut self) {
        if let Some(node) = self.receiver_node.take() {
            node.teardown().await;
        }
        if let Some(node) = self.mailbox.take() {
            node.teardown().await;
        }
        if let Some(node) = self.sender_node.take() {
            node.teardown().await;
        }
    }
}
