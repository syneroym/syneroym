#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The full operator-facing ownership path over a real
//! substrate -- `ControllerAgreement::issue` (the mechanism `roymctl
//! substrate claim` wraps), written to `app_data_dir/agreement.json` (the
//! implicit-discovery default) *before* the substrate ever starts, then a
//! single boot that must come up owned with no `[identity].agreement` config
//! line at all. This is the only test that exercises discovery, the
//! handshake, and both gates (`orchestrator/deploy` and `security`) together
//! -- every other related test either drives `SubstrateIdentityState::init`
//! directly (`crates/identity/src/substrate.rs`) or `build_caller` directly
//! (`crates/router/src/route_handler/io.rs`), never both through a real boot.
//!
//! Proves, live: the controller deploys a service and injects a KEK
//! (the positive half of denying a non-controller both); an unrelated
//! identity, verified but never delegated anything, is denied both
//! (the negative half).

use std::{fs, time::Duration};

use rustls::crypto::ring;
use syneroym_core::config::{
    ClientGatewayRole, CoordinatorIrohConfig, CoordinatorRole, DEFAULT_CONTROLLER_AGREEMENT_FILE,
    DEFAULT_SUBSTRATE_KEY_FILE, IrohParentConfig, LogTarget, ServiceRegistryRole, SubstrateConfig,
};
use syneroym_identity::{
    Identity,
    substrate::{ControllerAgreement, SubstrateIdentityStatus},
};
use syneroym_rpc::{JsonRpcError, PERMISSION_DENIED_CODE};
use syneroym_sdk::{NetworkEndpoint, SyneroymClient};
use syneroym_substrate::identity;
use tempfile::TempDir;
use tokio::{
    sync::{mpsc, mpsc::Sender},
    task::JoinHandle,
};

const IROH_PORT: u16 = 8600;
const REGISTRY_PORT: u16 = 8601;
const GATEWAY_PORT: u16 = 8602;

struct Node {
    registry_url: String,
    substrate_client: SyneroymClient,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
    _temp_dir: TempDir,
}

impl Node {
    /// Builds the shared config skeleton both `boot_claimed` and
    /// `boot_unowned` start from, and generates+saves the node's own key --
    /// the one thing every boot path needs regardless of whether an
    /// agreement gets minted on top of it.
    fn base_config(
        iroh_port: u16,
        registry_port: u16,
        gateway_port: u16,
    ) -> (SubstrateConfig, TempDir, Identity, String) {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let base_path = temp_dir.path();
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
                http_bind_address: format!("0.0.0.0:{iroh_port}"),
                ..Default::default()
            }),
            ..Default::default()
        });
        config.roles.community_registry = Some(ServiceRegistryRole {
            http_bind_address: format!("0.0.0.0:{registry_port}"),
            ..Default::default()
        });
        let registry_url = format!("http://localhost:{registry_port}");
        config.substrate.registry_url = Some(registry_url.clone());
        config.parent_coordinator.iroh =
            Some(IrohParentConfig { url: format!("http://localhost:{iroh_port}") });
        config.roles.client_gateway = Some(ClientGatewayRole { http_port: gateway_port });

        let node = Identity::generate().expect("node identity");
        fs::create_dir_all(&config.app_data_dir).expect("create app_data_dir");
        let key_path = config
            .identity
            .key
            .clone()
            .unwrap_or_else(|| config.app_data_dir.join(DEFAULT_SUBSTRATE_KEY_FILE));
        node.save_to_path(&key_path).expect("save node key");

        (config, temp_dir, node, registry_url)
    }

    async fn start(config: SubstrateConfig, registry_url: String, temp_dir: TempDir) -> Self {
        let substrate_identity_state =
            identity::setup_substrate_identity(&config.identity, &config.app_data_dir)
                .expect("failed to setup identity");
        let substrate_service_id = substrate_identity_state.did.clone();

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        let runtime =
            syneroym_substrate::init(config.clone()).await.expect("failed to initialize runtime");

        let config_clone = config.clone();
        let substrate_handle = tokio::spawn(async move {
            syneroym_substrate::run_with_signal(config_clone, runtime, async {
                let _ = shutdown_rx.recv().await;
            })
            .await
            .expect("substrate failed to run");
        });

        let mut substrate_client =
            SyneroymClient::new(substrate_service_id.clone(), registry_url.clone());
        substrate_client
            .wait_for_ready(Duration::from_secs(30))
            .await
            .expect("substrate did not become available in time");

        Self { registry_url, substrate_client, shutdown_tx, substrate_handle, _temp_dir: temp_dir }
    }

    /// Mints the node's own key and a `ControllerAgreement` binding it to
    /// `controller` *before* the substrate ever starts, writes the
    /// agreement to `app_data_dir/agreement.json`, and boots with no
    /// `[identity].agreement` config line -- the implicit-discovery path,
    /// exercised for real rather than at the `setup_substrate_identity`
    /// unit-test level.
    async fn boot_claimed(
        iroh_port: u16,
        registry_port: u16,
        gateway_port: u16,
        controller: &Identity,
    ) -> Self {
        let (config, temp_dir, node, registry_url) =
            Self::base_config(iroh_port, registry_port, gateway_port);

        let agreement =
            ControllerAgreement::issue(&node, controller, None).expect("issue agreement");
        let agreement_path = config.app_data_dir.join(DEFAULT_CONTROLLER_AGREEMENT_FILE);
        fs::write(&agreement_path, serde_json::to_string(&agreement).unwrap())
            .expect("write agreement.json");

        let state = identity::setup_substrate_identity(&config.identity, &config.app_data_dir)
            .expect("failed to setup identity");
        assert_eq!(
            state.status,
            SubstrateIdentityStatus::Verified,
            "the discovered agreement must verify before the substrate ever starts routing \
             connections"
        );

        Self::start(config, registry_url, temp_dir).await
    }

    /// Boots with no agreement at all -- an ordinary, never-claimed
    /// substrate. Proves an unowned substrate denies a deploy over the wire:
    /// every other related test drives `build_caller` or
    /// `SubstrateIdentityState::init` directly, never a real unowned
    /// substrate denying a real deploy.
    async fn boot_unowned(iroh_port: u16, registry_port: u16, gateway_port: u16) -> Self {
        let (config, temp_dir, _node, registry_url) =
            Self::base_config(iroh_port, registry_port, gateway_port);

        let state = identity::setup_substrate_identity(&config.identity, &config.app_data_dir)
            .expect("failed to setup identity");
        assert_eq!(
            state.status,
            SubstrateIdentityStatus::None,
            "a substrate with no agreement.json must boot unowned"
        );

        Self::start(config, registry_url, temp_dir).await
    }

    fn did(&self) -> &str {
        self.substrate_client.service_id()
    }

    async fn teardown(mut self) {
        let _ = self.substrate_client.shutdown().await;
        let _ = self.shutdown_tx.send(()).await;
        let _ = self.substrate_handle.await;
    }
}

fn orchestrator_client(node: &Node, caller: Identity) -> SyneroymClient {
    SyneroymClient::new_with_identity(node.did().to_string(), node.registry_url.clone(), caller)
}

#[tokio::test]
async fn a_claimed_substrate_admits_its_controller_and_denies_everyone_else() {
    let _ = ring::default_provider().install_default();

    let controller = Identity::generate().unwrap();
    let controller_for_client = Identity::from_bytes(&controller.to_bytes());
    let node = Node::boot_claimed(IROH_PORT, REGISTRY_PORT, GATEWAY_PORT, &controller).await;

    // --- The controller: deploys and injects a KEK, both of which an
    // unowned substrate would have denied entirely as of this slice. ---
    let mut controller_client = orchestrator_client(&node, controller_for_client);
    controller_client.connect().await.expect("controller failed to connect");

    controller_client
        .deploy_svc_tcp(
            "did:key:zP0OwnershipTestService".to_string(),
            vec![NetworkEndpoint {
                interface_name: "default".to_string(),
                host: "127.0.0.1".to_string(),
                port: 30099,
            }],
            None,
            None,
        )
        .await
        .expect("the controller must be able to deploy on a substrate it claimed");

    controller_client
        .inject_kek("aa".repeat(32))
        .await
        .expect("the controller must be able to inject a KEK on a substrate it claimed");

    // --- A stranger: verified over the wire, but never delegated anything
    // by the controller and not the node's own key. Both the `security`
    // interface and `orchestrator/deploy` must deny it. ---
    let stranger = Identity::generate().unwrap();
    let mut stranger_client = orchestrator_client(&node, stranger);
    stranger_client.connect().await.expect("stranger failed to connect");

    let deploy_err = stranger_client
        .deploy_svc_tcp(
            "did:key:zP0OwnershipStrangerService".to_string(),
            vec![NetworkEndpoint {
                interface_name: "default".to_string(),
                host: "127.0.0.1".to_string(),
                port: 30098,
            }],
            None,
            None,
        )
        .await
        .expect_err(
            "an unrelated identity must not be able to deploy on a claimed substrate it does not \
             control",
        );
    // Unlike `security`, the orchestrator's Tier-1 admission check
    // (`ControlPlaneService::deploy`) has no distinct denial code -- every
    // cause maps through `.map_err(RpcError::InternalError)`
    // (`crates/control_plane/src/service.rs`'s `"deploy"` arm), so the
    // message is the only way to confirm this failed for lack of a grant
    // and not some other reason.
    let deploy_err_string = deploy_err.to_string();
    assert!(
        deploy_err_string.contains("holds no orchestrator/deploy grant"),
        "must be denied for lack of a grant, not fail for some other reason: {deploy_err_string}"
    );

    // The controller already injected a KEK above, so a *second* injection
    // by anyone -- controller or stranger -- would also fail with
    // `KekAlreadyInjected` (-32603). Asserting only `is_err()` here cannot
    // tell that apart from the security gate actually denying the caller,
    // so the code must be checked specifically.
    let kek_err = stranger_client.inject_kek("bb".repeat(32)).await.expect_err(
        "an unrelated identity must not be able to reach the security interface on a claimed \
         substrate it does not control",
    );
    assert_eq!(
        kek_err.downcast_ref::<JsonRpcError>().map(|e| e.code),
        Some(PERMISSION_DENIED_CODE),
        "must be denied specifically, not fail on KekAlreadyInjected: {kek_err:?}"
    );

    node.teardown().await;
}

// +100 from the claimed-node block above, matching the spacing every other
// multi-node harness in this crate uses (8000/8100, 8200/8300, 8400/8500)
// -- not +10, which collides with the iroh coordinator's secondary
// `/v1/info` listener (`http_bind_address.port() + 10`,
// `crates/coordinator_iroh/src/coordinator.rs`): 8600 + 10 == 8610.
const UNOWNED_IROH_PORT: u16 = 8700;
const UNOWNED_REGISTRY_PORT: u16 = 8701;
const UNOWNED_GATEWAY_PORT: u16 = 8702;

/// The over-the-wire proof that an unowned substrate denies a deploy: a
/// substrate with no agreement at all denies a real deploy from a real,
/// verified caller. The claimed-node test above proves the same property
/// for a caller who is a *stranger to the controller*; this proves it for a
/// substrate that has no controller in the first place, which is the
/// failure mode the fail-closed flip exists to fix.
#[tokio::test]
async fn an_unowned_substrate_rejects_a_deploy() {
    let _ = ring::default_provider().install_default();

    let node =
        Node::boot_unowned(UNOWNED_IROH_PORT, UNOWNED_REGISTRY_PORT, UNOWNED_GATEWAY_PORT).await;

    let caller = Identity::generate().unwrap();
    let mut client = orchestrator_client(&node, caller);
    client.connect().await.expect("caller failed to connect");

    let deploy_err = client
        .deploy_svc_tcp(
            "did:key:zP0UnownedDeployService".to_string(),
            vec![NetworkEndpoint {
                interface_name: "default".to_string(),
                host: "127.0.0.1".to_string(),
                port: 30097,
            }],
            None,
            None,
        )
        .await
        .expect_err("an unowned substrate must deny every deploy");
    // As above: `deploy`'s Tier-1 admission denial has no distinct code, so
    // the message is what confirms this is a lack-of-grant denial.
    let deploy_err_string = deploy_err.to_string();
    assert!(
        deploy_err_string.contains("holds no orchestrator/deploy grant"),
        "must be denied for lack of a grant, not fail for some other reason: {deploy_err_string}"
    );

    node.teardown().await;
}
