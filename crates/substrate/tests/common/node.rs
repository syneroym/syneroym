//! One full `syneroym-substrate` instance for the `crates/substrate/tests/*.rs`
//! integration suites, plus a builder for the ways a test needs to vary it.
//!
//! Every `*_e2e.rs` file used to carry its own near-verbatim `struct Node` /
//! `async fn boot` -- about 110 lines each, 25 copies. They had all drifted
//! from one original: different loopback spelling (`0.0.0.0` vs `127.0.0.1`),
//! some injecting a KEK at boot and some not, some able to reboot under a
//! saved on-disk identity and some not, all hand-picking port numbers from a
//! comment-tracked ledger that nothing enforced. This is the single copy.
//!
//! ```ignore
//! // one owned node on its own registry
//! let node = common::SubstrateNode::builder().boot().await;
//!
//! // a second node that publishes into and resolves through the first
//! let managed = common::SubstrateNode::builder()
//!     .owner(&managed_owner)
//!     .shared_registry(node.registry_url())
//!     .shared_relay(node.relay_url())
//!     .boot()
//!     .await;
//!
//! // reboot the same node under its own on-disk identity after an outage
//! let node = node.restart().await;
//! ```
//!
//! Serialising node lifetimes within one test binary is the caller's job:
//! hold [`crate::common::serial_guard`] for the whole test body. (The older
//! [`crate::common::SubstrateTestContext`] holds that same lock internally
//! for its lifetime, which is why a test that needs two live nodes cannot
//! use it and reaches for this builder instead.)

#![allow(dead_code)]

use std::{path::PathBuf, sync::Arc, time::Duration};

use syneroym_core::config::{
    ClientGatewayRole, CoordinatorIrohConfig, CoordinatorRole, IrohParentConfig, LogTarget,
    ServiceRegistryRole, SubstrateConfig, SupervisorRole,
};
use syneroym_identity::{Identity, substrate};
use syneroym_sdk::SyneroymClient;
use syneroym_substrate::identity;
use tempfile::TempDir;
use tokio::{
    sync::{mpsc, mpsc::Sender},
    task::JoinHandle,
    time,
};

use crate::common::alloc_ports;

/// The four listeners one node needs, all probed free below the OS ephemeral
/// range by [`alloc_ports`]. The QUIC port is separate because the iroh
/// coordinator's QUIC listener otherwise defaults to a fixed `0.0.0.0:7965`,
/// which collides the moment two node processes run at once.
#[derive(Clone, Copy, Debug)]
pub struct NodePorts {
    pub iroh: u16,
    pub registry: u16,
    pub gateway: u16,
    pub quic: u16,
}

impl NodePorts {
    fn alloc() -> Self {
        let [iroh, registry, gateway, quic] = alloc_ports::<4>();
        Self { iroh, registry, gateway, quic }
    }
}

type ConfigHook = Arc<dyn Fn(&mut SubstrateConfig) + Send + Sync>;
type IdentityHook = Arc<dyn Fn(&substrate::SubstrateIdentityState) + Send + Sync>;

/// Builder for [`SubstrateNode`]. Defaults: a freshly generated owner
/// identity (its DID becomes `[iam].admin_ucan_root`, so the node boots
/// owned -- an unowned substrate fails closed), that owner's own
/// `SyneroymClient`, this node's own registry and relay, no supervisor
/// role, no KEK injected, a temp directory cleaned on drop.
#[derive(Clone)]
pub struct NodeBuilder {
    ports: NodePorts,
    base_path: Option<PathBuf>,
    owner: Option<Arc<Identity>>,
    own_node: bool,
    make_owner_client: bool,
    shared_registry_url: Option<String>,
    shared_relay_url: Option<String>,
    supervisor: Option<SupervisorRole>,
    inject_kek: Option<[u8; 32]>,
    configure: ConfigHook,
    inspect_identity: Option<IdentityHook>,
}

impl NodeBuilder {
    fn new() -> Self {
        Self {
            ports: NodePorts::alloc(),
            base_path: None,
            owner: None,
            own_node: true,
            make_owner_client: true,
            shared_registry_url: None,
            shared_relay_url: None,
            supervisor: None,
            inject_kek: None,
            configure: Arc::new(|_| {}),
            inspect_identity: None,
        }
    }

    /// Own this node with `owner` instead of a fresh random identity. The
    /// caller keeps `owner`; the builder takes its own copy.
    pub fn owner(mut self, owner: &Identity) -> Self {
        self.owner = Some(Arc::new(Identity::from_bytes(&owner.to_bytes())));
        self
    }

    /// Boot with no `[iam].admin_ucan_root` at all -- an ordinary,
    /// never-claimed substrate. Also drops the owner `SyneroymClient`, since
    /// there is no owner identity to build it from; use
    /// [`SubstrateNode::client_as`] for a caller of your choice.
    pub fn unowned(mut self) -> Self {
        self.own_node = false;
        self.make_owner_client = false;
        self
    }

    /// Keep the node owned but skip building the owner's `SyneroymClient`
    /// (the test builds its own client, e.g. with a non-owner caller).
    pub fn no_owner_client(mut self) -> Self {
        self.make_owner_client = false;
        self
    }

    /// Publish into and resolve through another node's registry instead of
    /// hosting one. This node still runs its own `community_registry` role
    /// (harmless, and a few tests read it), but its `substrate.registry_url`
    /// and its owner client both point at `url`.
    pub fn shared_registry(mut self, url: impl Into<String>) -> Self {
        self.shared_registry_url = Some(url.into());
        self
    }

    /// Relay through another node's iroh coordinator instead of self-relaying.
    /// Two localhost peers on separate relays otherwise pay real
    /// direct-path-negotiation latency for nothing.
    pub fn shared_relay(mut self, url: impl Into<String>) -> Self {
        self.shared_relay_url = Some(url.into());
        self
    }

    /// Run the supervisor role with this configuration.
    pub fn supervisor(mut self, role: SupervisorRole) -> Self {
        self.supervisor = Some(role);
        self
    }

    /// Inject a KEK once the node is ready. Default storage encryption needs
    /// one before any deployed service's native-capability endpoints can be
    /// set up. The value is arbitrary; use [`Self::inject_kek_bytes`] to pin
    /// it.
    pub fn inject_kek(mut self) -> Self {
        self.inject_kek = Some([0xab; 32]);
        self
    }

    /// [`Self::inject_kek`] with a caller-chosen 32-byte key.
    pub fn inject_kek_bytes(mut self, kek: [u8; 32]) -> Self {
        self.inject_kek = Some(kek);
        self
    }

    /// Boot from a directory the caller owns, rather than a fresh temp dir.
    /// The directory is not cleaned on drop. Needed when the same on-disk
    /// identity must outlive one node process -- see
    /// [`SubstrateNode::restart`].
    pub fn base_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.base_path = Some(path.into());
        self
    }

    /// Mutate the `SubstrateConfig` just before the node boots -- the escape
    /// hatch for any knob without a dedicated method (`roles.app_sandbox`,
    /// `roles.auth`, `client_gateway.identity_mode`, ...). Runs after the
    /// owner and `admin_ucan_root` are set and before the substrate identity
    /// is materialised, so it can also write an `agreement.json` into
    /// `config.app_data_dir` or override `admin_ucan_root`.
    pub fn configure(
        mut self,
        hook: impl Fn(&mut SubstrateConfig) + Send + Sync + 'static,
    ) -> Self {
        self.configure = Arc::new(hook);
        self
    }

    /// Inspect the substrate identity state the moment it is materialised,
    /// before the runtime starts routing. For a test that needs to assert on
    /// the discovered ownership status (`Verified` / `None` / ...) at exactly
    /// that point -- the assertion a hand-rolled harness would put between its
    /// own `setup_substrate_identity` call and its `run` spawn.
    pub fn inspect_identity(
        mut self,
        f: impl Fn(&substrate::SubstrateIdentityState) + Send + Sync + 'static,
    ) -> Self {
        self.inspect_identity = Some(Arc::new(f));
        self
    }

    /// The ports this node will bind. Read them before [`Self::boot`] when a
    /// test needs a port value up front.
    pub fn ports(&self) -> NodePorts {
        self.ports
    }

    fn build_config(&self, base_path: &std::path::Path) -> (SubstrateConfig, String, String) {
        let mut config = SubstrateConfig {
            app_local_data_dir: base_path.join("data"),
            app_data_dir: base_path.join("user_data"),
            app_cache_dir: base_path.join("cache"),
            app_log_dir: base_path.join("logs"),
            profile: "full".to_string(),
            ..SubstrateConfig::default()
        };
        config.resolve_paths();
        config.logging.target = LogTarget::Stdout;

        config.roles.coordinator = Some(CoordinatorRole {
            iroh: Some(CoordinatorIrohConfig {
                enable_relay: true,
                http_bind_address: format!("127.0.0.1:{}", self.ports.iroh),
                quic_bind_address: format!("127.0.0.1:{}", self.ports.quic),
                ..Default::default()
            }),
            ..Default::default()
        });
        config.roles.community_registry = Some(ServiceRegistryRole {
            http_bind_address: format!("127.0.0.1:{}", self.ports.registry),
            ..Default::default()
        });

        let own_registry_url = format!("http://127.0.0.1:{}", self.ports.registry);
        let registry_url = self.shared_registry_url.clone().unwrap_or(own_registry_url);
        config.substrate.registry_url = Some(registry_url.clone());
        config.substrate.enable_bep0044_dht = false;

        let own_relay_url = format!("http://127.0.0.1:{}", self.ports.iroh);
        let relay_url = self.shared_relay_url.clone().unwrap_or(own_relay_url.clone());
        config.parent_coordinator.iroh = Some(IrohParentConfig { url: relay_url });

        config.roles.client_gateway =
            Some(ClientGatewayRole { http_port: self.ports.gateway, ..Default::default() });
        config.roles.supervisor = self.supervisor.clone();

        (config, registry_url, own_relay_url)
    }

    /// Boot the node and wait until its registry reports it ready.
    pub async fn boot(self) -> SubstrateNode {
        let (base_path, temp_dir) = match &self.base_path {
            Some(path) => (path.clone(), None),
            None => {
                let dir = tempfile::tempdir().expect("failed to create temp dir");
                let path = dir.path().to_path_buf();
                (path, Some(dir))
            }
        };
        self.boot_at(base_path, temp_dir).await
    }

    async fn boot_at(self, base_path: PathBuf, temp_dir: Option<TempDir>) -> SubstrateNode {
        let (mut config, registry_url, relay_url) = self.build_config(&base_path);

        let owner: Option<Arc<Identity>> = if self.own_node || self.make_owner_client {
            Some(
                self.owner
                    .clone()
                    .unwrap_or_else(|| Arc::new(Identity::generate().expect("owner identity"))),
            )
        } else {
            self.owner.clone()
        };
        if self.own_node
            && let Some(owner) = &owner
        {
            config.iam.admin_ucan_root = Some(substrate::derive_did_key(&owner.public_key()));
        }

        (self.configure)(&mut config);

        let identity_state =
            identity::setup_substrate_identity(&config.identity, &config.app_data_dir)
                .expect("failed to setup substrate identity");
        if let Some(inspect) = &self.inspect_identity {
            inspect(&identity_state);
        }
        let service_id = identity_state.did.clone();

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        let runtime =
            syneroym_substrate::init(config.clone()).await.expect("failed to initialize runtime");
        let run_config = config.clone();
        let substrate_handle = tokio::spawn(async move {
            syneroym_substrate::run_with_signal(run_config, runtime, async {
                let _ = shutdown_rx.recv().await;
            })
            .await
            .expect("substrate failed to run");
        });

        let mut substrate_client = match (&owner, self.make_owner_client) {
            (Some(owner), true) => SyneroymClient::new_with_identity(
                service_id.clone(),
                registry_url.clone(),
                Identity::from_bytes(&owner.to_bytes()),
            ),
            _ => SyneroymClient::new(service_id.clone(), registry_url.clone()),
        }
        .with_registry_dht(false);
        substrate_client
            .wait_for_ready(Duration::from_secs(30))
            .await
            .expect("substrate did not become available in time");

        if let Some(kek) = self.inject_kek {
            substrate_client.inject_kek(hex::encode(kek)).await.expect("inject_kek failed");
        }

        SubstrateNode {
            builder: self,
            base_path,
            app_data_dir: config.app_data_dir,
            temp_dir,
            registry_url,
            relay_url,
            owner,
            substrate_client,
            shutdown_tx,
            substrate_handle,
        }
    }
}

/// A live `syneroym-substrate` node: a real iroh QUIC socket, a self-hosted
/// relay (unless [`NodeBuilder::shared_relay`]), a registry, the WASM
/// sandbox. Tear it down with [`Self::teardown`].
pub struct SubstrateNode {
    builder: NodeBuilder,
    base_path: PathBuf,
    app_data_dir: PathBuf,
    temp_dir: Option<TempDir>,
    registry_url: String,
    relay_url: String,
    owner: Option<Arc<Identity>>,
    /// The owner's client, dialed and proven live during boot. Absent only
    /// when the builder was told to skip it ([`NodeBuilder::unowned`] /
    /// [`NodeBuilder::no_owner_client`]) -- calling this then panics, which
    /// is a test-wiring mistake, not a runtime condition.
    pub substrate_client: SyneroymClient,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
}

impl SubstrateNode {
    pub fn builder() -> NodeBuilder {
        NodeBuilder::new()
    }

    /// This node's DID.
    pub fn did(&self) -> &str {
        self.substrate_client.service_id()
    }

    /// The registry URL this node publishes into and resolves through --
    /// its own, or the shared one from [`NodeBuilder::shared_registry`].
    pub fn registry_url(&self) -> &str {
        &self.registry_url
    }

    /// This node's own relay URL, for another node's
    /// [`NodeBuilder::shared_relay`].
    pub fn relay_url(&self) -> &str {
        &self.relay_url
    }

    /// This node's client-gateway base URL (`http://127.0.0.1:<gateway port>`).
    /// For tests that drive the gateway's HTTP surface directly rather than
    /// through a `SyneroymClient`.
    pub fn gateway_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.builder.ports().gateway)
    }

    /// The directory this node boots from -- its `base_path`, holding `data/`,
    /// `user_data/`, `cache/`, and `logs/`. For a test that reaches into the
    /// on-disk service state between a teardown and a reboot.
    pub fn base_path(&self) -> &std::path::Path {
        &self.base_path
    }

    /// This node's resolved `app_data_dir` -- where its identity key, vault,
    /// and supervisor master backups live on disk.
    pub fn app_data_dir(&self) -> &std::path::Path {
        &self.app_data_dir
    }

    /// The owner identity, when the node is owned.
    pub fn owner(&self) -> Option<&Identity> {
        self.owner.as_deref()
    }

    /// The owner's DID, when the node is owned.
    pub fn owner_did(&self) -> Option<String> {
        self.owner.as_deref().map(|o| substrate::derive_did_key(&o.public_key()))
    }

    /// The ports this node bound.
    pub fn ports(&self) -> NodePorts {
        self.builder.ports
    }

    /// A `SyneroymClient` addressed to this node, acting as `caller`,
    /// resolving through this node's registry.
    pub fn client_as(&self, caller: Identity) -> SyneroymClient {
        SyneroymClient::new_with_identity(self.did().to_string(), self.registry_url.clone(), caller)
            .with_registry_dht(false)
    }

    /// Stop this node and reboot it under the same on-disk identity, ports,
    /// and configuration. The directory and the owning temp dir are carried
    /// across, so the rebooted node keeps its DID and its keys.
    pub async fn restart(mut self) -> SubstrateNode {
        let _ = self.substrate_client.shutdown().await;
        let _ = self.shutdown_tx.send(()).await;
        let _ = time::timeout(Duration::from_secs(20), self.substrate_handle).await;

        let temp_dir = self.temp_dir.take();
        let base_path = self.base_path.clone();
        let mut rebooted = self.builder.clone().boot_at(base_path, None).await;
        rebooted.temp_dir = temp_dir;
        rebooted
    }

    pub async fn teardown(mut self) {
        let _ = self.substrate_client.shutdown().await;
        let _ = self.shutdown_tx.send(()).await;
        let _ = time::timeout(Duration::from_secs(20), self.substrate_handle)
            .await
            .map_err(|_| eprintln!("[teardown] substrate handle join timed out"));
    }
}
