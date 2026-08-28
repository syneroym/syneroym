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

use std::{
    net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener},
    sync::atomic::{AtomicU16, Ordering},
    time::Duration,
};

use syneroym_core::{
    config::{ClientGatewayRole, IrohParentConfig, LogTarget, SubstrateConfig},
    dht_registry::EndpointMechanism,
};
use syneroym_identity::{Identity, substrate};
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
///
/// The guard this returns is held by `SubstrateTestContext` for its whole
/// lifetime (a struct field, dropped only at `teardown`/drop), not just
/// through `setup` -- so, incidentally, no two tests in one binary ever
/// have a substrate live at the same time either, not only never mid-setup.
/// At least one consuming file (`static_assets_e2e.rs`'s `counter_value`)
/// now depends on that full-lifetime property, not just the port race, to
/// read a process-global metrics counter safely -- narrowing this lock back
/// to just the setup race would silently reopen that.
static SUBSTRATE_TEST_LOCK: Mutex<()> = Mutex::const_new(());

/// Ports below 32_768 sit outside the OS's ephemeral range on Linux and
/// macOS (`net.ipv4.ip_local_port_range` defaults to roughly 32768-60999 on
/// Linux, the default on GitHub Actions runners). The kernel never hands a
/// port down here to an unrelated outbound socket on its own -- the only
/// way something else holds one is another explicit bind, i.e. another test
/// binary (within one binary, `SUBSTRATE_TEST_LOCK` above already
/// serializes tests, so there's no same-binary contention to race against).
/// A probe bind -- try it, keep it if it succeeds, move on if it doesn't --
/// is therefore a reliable free-port check in this range. It would not be
/// inside the ephemeral one, where a bind can lose a race to a transient
/// outbound connection that releases the port moments later; that's what
/// made a hardcoded port in that range (`gateway_hostname_e2e.rs`'s old
/// `42_600`) an intermittent CI failure.
const PORT_POOL_START: u16 = 12_000;
const PORT_POOL_END: u16 = 32_000;

static NEXT_PORT_HINT: AtomicU16 = AtomicU16::new(0);

fn probe_bind(port: u16) -> Option<StdTcpListener> {
    StdTcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).ok()
}

/// Reserve `N` distinct free ports below the OS ephemeral range, each
/// verified by an actual bind (immediately released) rather than trusting a
/// hand-tracked constant. Self-correcting against both leftover local
/// sockets and other test binaries racing for the same numbers: a losing
/// probe just moves the search on to the next candidate block instead of
/// failing the caller. There's a small window between the probe release
/// here and the caller's own real bind; outside the ephemeral range the
/// only realistic contender is another test binary doing the same probe at
/// the same instant, which is rare enough in practice that this hasn't
/// needed a stronger guarantee (e.g. the OS reporting its own resolved
/// port back to the caller instead of pre-allocating one).
pub fn alloc_ports<const N: usize>() -> [u16; N] {
    let span = PORT_POOL_END - PORT_POOL_START;
    loop {
        let offset = NEXT_PORT_HINT.fetch_add(N as u16, Ordering::Relaxed) % span;
        let start = PORT_POOL_START + offset;
        if start as u32 + N as u32 > PORT_POOL_END as u32 {
            continue; // wrapped mid-block; the next fetch_add tries elsewhere
        }

        let mut listeners = Vec::with_capacity(N);
        let mut all_free = true;
        for port in start..start + N as u16 {
            match probe_bind(port) {
                Some(listener) => listeners.push(listener),
                None => {
                    all_free = false;
                    break;
                }
            }
        }
        if !all_free {
            continue;
        }

        let ports: Vec<u16> =
            listeners.iter().map(|l| l.local_addr().expect("bound listener").port()).collect();
        drop(listeners);
        return ports.try_into().expect("exactly N ports collected");
    }
}

pub struct SubstrateTestContext {
    #[allow(dead_code)]
    config: SubstrateConfig,
    pub substrate_client: SyneroymClient,
    #[allow(dead_code)]
    registry_url: String,
    #[allow(dead_code)]
    pub substrate_mechanisms: Vec<EndpointMechanism>,
    /// The DID that owns this substrate (an unowned
    /// substrate now fails closed, so every harness must own its own
    /// node). Exposed for tests that build an extra client of their own.
    #[allow(dead_code)]
    pub owner_did: String,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
    #[allow(dead_code)]
    temp_dir: TempDir,
    _lock_guard: MutexGuard<'static, ()>,
}

impl SubstrateTestContext {
    pub async fn setup(iroh_port: u16, registry_port: u16, gateway_port: u16) -> Self {
        Self::setup_with(iroh_port, registry_port, gateway_port, |_| {}).await
    }

    /// [`Self::setup`], plus a hook to mutate the `SubstrateConfig` before
    /// the substrate boots (M06A A2) -- e.g. setting `roles.app_sandbox` to
    /// exercise a non-default `AppSandboxRole` knob (`D-A2-11`'s
    /// `max_concurrent_guest_http_per_service`). Additive: `setup` above is
    /// unchanged for its existing callers.
    pub async fn setup_with(
        iroh_port: u16,
        registry_port: u16,
        gateway_port: u16,
        configure: impl FnOnce(&mut SubstrateConfig),
    ) -> Self {
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
        config.substrate.enable_bep0044_dht = false;
        config.parent_coordinator.iroh =
            Some(IrohParentConfig { url: format!("http://localhost:{iroh_port}") });
        config.roles.client_gateway =
            Some(ClientGatewayRole { http_port: gateway_port, ..Default::default() });
        config.roles.auth = Some(syneroym_core::config::AuthRole::default());

        configure(&mut config);

        // An unowned substrate now fails closed, so this
        // harness must own its own node -- mint an owner identity and
        // configure it directly (`admin_ucan_root`), rather than going
        // through the `roymctl substrate claim` file-discovery path this
        // harness has no reason to also exercise.
        let owner = Identity::generate().expect("owner identity");
        let owner_did = substrate::derive_did_key(&owner.public_key());
        config.iam.admin_ucan_root = Some(owner_did.clone());

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

        let mut substrate_client = SyneroymClient::new_with_identity(
            substrate_service_id.clone(),
            registry_url.clone(),
            owner,
        )
        .with_registry_dht(false);
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
            owner_did,
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
