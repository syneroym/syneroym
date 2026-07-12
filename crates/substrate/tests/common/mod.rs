//! Shared full-substrate-instance bootstrap for `crates/substrate/tests/*.rs`
//! integration suites. Each consuming file (`http_passthrough_e2e.rs`,
//! `stream_client_e2e.rs`, `messaging_client_e2e.rs`) pulls this in via
//! `mod common;` -- since every `tests/*.rs` file is compiled as its own
//! independent test binary, this module (and `SUBSTRATE_TEST_LOCK` below)
//! is duplicated once per consuming binary, not shared process-wide.
//!
//! Extracted from three near-verbatim ~90-line copies that had already
//! started drifting (`http_passthrough_e2e.rs`'s copy added the DHT-port
//! lock guard below; the other two didn't have it).

use std::time::Duration;

use syneroym_core::{
    config::{ClientGatewayRole, IrohParentConfig, LogTarget, SubstrateConfig},
    dht_registry::EndpointMechanism,
};
use syneroym_sdk::SyneroymClient;
use syneroym_substrate::identity;
use tempfile::TempDir;
use tokio::{
    sync::{Mutex, MutexGuard, mpsc, mpsc::Sender},
    task::JoinHandle,
    time,
};

/// Every consuming test file spins up a full substrate instance, and each
/// one includes a `mainline` DHT component that (independent of the
/// caller's own per-test `iroh_port`/`registry_port`/`gateway_port`
/// arguments) always tries the standard BitTorrent DHT port `6881` first.
/// With `cargo test`'s default in-binary parallelism, two tests in the same
/// binary starting at once can reliably lose that race with an `Address
/// already in use` startup failure. Serializing full-substrate-instance
/// setup within one binary (not a fix to the DHT component itself, which is
/// out of scope here) avoids it; cross-binary parallelism (separate
/// `cargo test --test` processes, each with its own copy of this static) is
/// unaffected.
static SUBSTRATE_TEST_LOCK: Mutex<()> = Mutex::const_new(());

pub struct SubstrateTestContext {
    #[allow(dead_code)]
    config: SubstrateConfig,
    pub substrate_client: SyneroymClient,
    registry_url: String,
    pub substrate_mechanisms: Vec<EndpointMechanism>,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
    temp_dir: TempDir,
    _lock_guard: MutexGuard<'static, ()>,
}

impl SubstrateTestContext {
    pub async fn setup(iroh_port: u16, registry_port: u16, gateway_port: u16) -> Self {
        use syneroym_core::config::{CoordinatorIrohConfig, CoordinatorRole, ServiceRegistryRole};

        let lock_guard = SUBSTRATE_TEST_LOCK.lock().await;

        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
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

        let substrate_identity_state =
            identity::setup_substrate_identity(&config.identity, &config.app_data_dir)
                .expect("Failed to setup identity");
        let substrate_service_id = substrate_identity_state.did.clone();

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        let runtime =
            syneroym_substrate::init(config.clone()).await.expect("Failed to initialize runtime");

        let config_clone = config.clone();
        let substrate_handle = tokio::spawn(async move {
            syneroym_substrate::run_with_signal(config_clone, runtime, async {
                let _ = shutdown_rx.recv().await;
            })
            .await
            .expect("Substrate failed to run");
        });

        let mut substrate_client =
            SyneroymClient::new(substrate_service_id.clone(), registry_url.clone());
        substrate_client
            .wait_for_ready(Duration::from_secs(30))
            .await
            .expect("Substrate did not become available in time");

        let substrate_info =
            substrate_client.lookup().await.expect("Failed to lookup substrate info from registry");
        let substrate_mechanisms = substrate_info.info.mechanisms;

        Self {
            config,
            substrate_client,
            registry_url,
            substrate_mechanisms,
            shutdown_tx,
            substrate_handle,
            temp_dir,
            _lock_guard: lock_guard,
        }
    }

    pub async fn teardown(mut self) {
        eprintln!("[teardown] shutting down substrate_client...");
        let _ = self.substrate_client.shutdown().await;
        eprintln!("[teardown] sending shutdown signal...");
        let _ = self.shutdown_tx.send(()).await;
        eprintln!("[teardown] awaiting substrate_handle...");
        let _ = time::timeout(Duration::from_secs(20), self.substrate_handle)
            .await
            .map_err(|_| eprintln!("[teardown] substrate_handle join TIMED OUT"));
        eprintln!("[teardown] done");
    }
}
