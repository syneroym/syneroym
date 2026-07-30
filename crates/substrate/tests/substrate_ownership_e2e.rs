#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! M05A Slice P0: the full operator-facing ownership path over a real
//! substrate -- `ControllerAgreement::issue` (the mechanism `roymctl
//! substrate claim` wraps), written to `app_data_dir/agreement.json` (D-P0-5's
//! implicit-discovery default) *before* the substrate ever starts, then a
//! single boot that must come up owned with no `[identity].agreement` config
//! line at all. This is the only test that exercises discovery, the
//! handshake, and both gates (`orchestrator/deploy` and `security`) together
//! -- every other P0 test either drives `SubstrateIdentityState::init`
//! directly (`crates/identity/src/substrate.rs`) or `build_caller` directly
//! (`crates/router/src/route_handler/io.rs`), never both through a real boot.
//!
//! Proves, live: the controller deploys a service and injects a KEK
//! (**matrix rows 16/17**'s positive half); an unrelated identity, verified
//! but never delegated anything, is denied both (**matrix rows 16/17**'s
//! negative half).

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
    /// Mints the node's own key and a `ControllerAgreement` binding it to
    /// `controller` *before* the substrate ever starts, writes the
    /// agreement to `app_data_dir/agreement.json`, and boots with no
    /// `[identity].agreement` config line -- the discovery path D-P0-5
    /// adds, exercised for real rather than at the `setup_substrate_identity`
    /// unit-test level.
    async fn boot_claimed(
        iroh_port: u16,
        registry_port: u16,
        gateway_port: u16,
        controller: &Identity,
    ) -> Self {
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

        // The node's own key, generated and saved exactly as
        // `setup_substrate_identity` would on a first boot -- generated
        // here, rather than left to that call, because `issue` needs the
        // `Identity` to mint the agreement before the substrate ever reads
        // the key file.
        let node = Identity::generate().expect("node identity");
        fs::create_dir_all(&config.app_data_dir).expect("create app_data_dir");
        let key_path = config
            .identity
            .key
            .clone()
            .unwrap_or_else(|| config.app_data_dir.join(DEFAULT_SUBSTRATE_KEY_FILE));
        node.save_to_path(&key_path).expect("save node key");

        let agreement =
            ControllerAgreement::issue(&node, controller, None).expect("issue agreement");
        let agreement_path = config.app_data_dir.join(DEFAULT_CONTROLLER_AGREEMENT_FILE);
        fs::write(&agreement_path, serde_json::to_string(&agreement).unwrap())
            .expect("write agreement.json");

        let substrate_identity_state =
            identity::setup_substrate_identity(&config.identity, &config.app_data_dir)
                .expect("failed to setup identity");
        let substrate_service_id = substrate_identity_state.did.clone();
        assert_eq!(
            substrate_identity_state.status,
            SubstrateIdentityStatus::Verified,
            "the discovered agreement must verify before the substrate ever starts routing \
             connections"
        );

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
    // by the controller and not the node's own key. Both matrix row
    // 16 (security) and row 17 (orchestrator) must deny it. ---
    let stranger = Identity::generate().unwrap();
    let mut stranger_client = orchestrator_client(&node, stranger);
    stranger_client.connect().await.expect("stranger failed to connect");

    let deploy_result = stranger_client
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
        .await;
    assert!(
        deploy_result.is_err(),
        "matrix row 17: an unrelated identity must not be able to deploy on a claimed substrate \
         it does not control"
    );

    let kek_result = stranger_client.inject_kek("bb".repeat(32)).await;
    assert!(
        kek_result.is_err(),
        "matrix row 16: an unrelated identity must not be able to reach the security interface on \
         a claimed substrate it does not control"
    );

    node.teardown().await;
}
