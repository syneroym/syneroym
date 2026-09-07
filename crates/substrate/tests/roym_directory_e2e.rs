#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]
//! The Roym product's Directory service -- the search half -- end to end
//! across three genuinely independent `syneroym-substrate` instances, each
//! running the full Roym SynApp (the `wasm32-wasip2` build) under its own
//! owner identity, over real transports and one shared registry.
//!
//! Three nodes:
//!   * **Z** runs the SynOrg (the directory a provider publishes to and a
//!     consumer queries).
//!   * **Y** is the provider: it signs a listing and publishes it to Z.
//!   * **X** is the consumer: it adds Z as a source, runs the client fan-out
//!     loop (`start-run` -> `query-source` -> `merge`), verifies every returned
//!     envelope on its own node, and engages the provider from the search
//!     result's own conversation address.
//!
//! It proves the non-visual half of the directory acceptance test: a
//! directory is a query target and never a required hub (the
//! find-and-engage path runs with no directory in
//! it, both before any publication exists and again at the end after one
//! has); results carry source and freshness, and the freshness a person
//! sees is computed on their own clock; missing evidence renders as
//! `unknown`, never as a positive default; the directory verifies nothing
//! on the consumer's behalf; a publication past the SynOrg's limit is
//! refused visibly to the provider; a stranger dialling in from a
//! self-minted identity reaches `directory.search` and is refused
//! `member.list`, and -- because a generated key is still a verified
//! connection -- is admitted to `VerifiedOnly` `directory.publish` (the
//! truly key-less anonymous arm lives in the parity suite); a stale or
//! absent certificate on
//! the provider's own node is indistinguishable, at the provider, from
//! "this directory does not want you"; two directories disagreeing about a
//! version surface the disagreement rather than resolve it silently; and
//! `directory.unpublish` removes a listing from future search without
//! touching a copy a consumer already holds.
//!
//! `Node::boot` / `deploy` / `teardown` and the serial lock are copied in
//! shape from `roym_conversation_e2e.rs` (which itself copied them from
//! `conversation_e2e.rs` / `roym_identity_e2e.rs`); the repo tolerates this
//! e2e-harness duplication rather than a shared module. Only the first node
//! hosts the shared community registry -- three registry servers plus three
//! iroh relays in one process starve the first node's own registry out of
//! its 30-attempt (15 s) registration window on a loaded machine, and the
//! next heartbeat is an hour away.
//!
//! No step here restarts a substrate, so there is no redeploy-after-restart
//! path to get wrong; the certificate-dependency sub-step uses a fresh
//! fourth node instead.
//!
//! Skips when the Roym wasm artifacts or the UI bundle are absent
//! (`mise run build:roym` / `mise run build:roym-ui`).

use std::{
    collections::BTreeMap,
    fs,
    future::Future,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use reqwest::Client;
use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_app_orchestration::{
    AppInstanceId, DeploymentJournal, DeploymentPlan, DeploymentState, LocalFilesystemCatalog,
    compile,
    models::{ServiceId, SubstrateAlias, SynAppManifest, Visibility},
};
use syneroym_core::{
    config::{
        AppSandboxRole, AuthRole, ClientGatewayRole, CoordinatorIrohConfig, CoordinatorRole,
        IdentityMode, IrohParentConfig, LogTarget, ServiceRegistryRole, SubstrateConfig,
    },
    dht_registry::{DEFAULT_ENDPOINT_NOT_AFTER_SECS, RegistryClient},
    util::short_hash,
};
use syneroym_identity::{DelegationCertificate, Identity, substrate};
use syneroym_sdk::{
    SyneroymClient,
    deploy::{
        self, ApplyRequest, DeployTarget, SubstrateActor, apply_plan, certify_instance,
        member_registry_record,
    },
};
use syneroym_signed_record::SCOPE_RECORD_SIGNING;
use syneroym_substrate::identity;
use tokio::{
    sync::{Mutex, mpsc, mpsc::Sender},
    task::JoinHandle,
    time,
};

const SESSION_COOKIE_NAME: &str = "syneroym_session";
const DIRECTORY_INTERFACE: &str = "syneroym-roym:directory/api@0.1.0";

/// Not sharing a port block with any other e2e file here --
/// `conversation_e2e.rs` claims 14_000-14_102, `roym_conversation_e2e.rs`
/// claims 14_200-14_302.
const PORTS_X: (u16, u16, u16) = (14_400, 14_401, 14_402);
const PORTS_Y: (u16, u16, u16) = (14_500, 14_501, 14_502);
const PORTS_Z: (u16, u16, u16) = (14_600, 14_601, 14_602);
/// A transient fourth node, alive only for the certificate-dependency
/// sub-step, after X and Y have been torn down.
const PORTS_W: (u16, u16, u16) = (14_700, 14_701, 14_702);

/// Three full substrate instances plus wasmtime starve a CI runner badly
/// enough to time iroh's QUIC path validation out if two files' groups run
/// at once -- same fix as every other multi-node e2e file here.
static SUBSTRATE_TEST_LOCK: Mutex<()> = Mutex::const_new(());

/// The Roym services that sign a record and so need a record-signing
/// certificate. `directory` signs nothing and is deliberately
/// absent. Mirrors `roymctl`'s own list.
const SIGNING_SERVICES: &[&str] = &["profile", "catalog", "conversation"];

fn far_future_not_after() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS)
}

/// `conversation_tick_secs: 1` so a delivery attempt does not wait out the
/// production budget -- the find-and-engage steps (4, 8, 12) each deliver
/// one real message.
fn fast_conversation_role() -> AppSandboxRole {
    AppSandboxRole { conversation_tick_secs: 1, ..AppSandboxRole::default() }
}

fn roym_artifacts_present() -> bool {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = root.join("crates/roym_core/app/roym.toml");
    let Ok(toml_str) = fs::read_to_string(&manifest) else { return false };
    let Ok(m): Result<SynAppManifest, _> = toml::from_str(&toml_str) else { return false };
    for svc in m.services.values() {
        if !root.join(&svc.config.source).exists() {
            return false;
        }
        if let Some(assets) = &svc.config.assets
            && !root.join(&assets.archive).exists()
        {
            return false;
        }
    }
    true
}

/// The manifest's declared visibility per service (`roym.toml`).
fn service_visibility(name: &str) -> Visibility {
    match name {
        "web" => Visibility::Internal,
        "profile" => Visibility::Private,
        _ => Visibility::Public,
    }
}

fn mint_masters(plan: &DeploymentPlan) -> BTreeMap<String, Identity> {
    plan.services
        .iter()
        .map(|s| (s.logical_ref.service_name.as_str().to_string(), Identity::generate().unwrap()))
        .collect()
}

/// Substitutes each service's compiled id with the DID of the stable master
/// for that service *name*, so a redeploy after a restart keeps every
/// service id -- Roym's own storage is keyed by it.
fn substitute_plan(plan: &DeploymentPlan, masters: &BTreeMap<String, Identity>) -> DeploymentPlan {
    let by_old: BTreeMap<ServiceId, ServiceId> = plan
        .services
        .iter()
        .map(|s| {
            let name = s.logical_ref.service_name.as_str();
            let did = substrate::derive_did_key(&masters[name].public_key());
            (s.service_id.clone(), ServiceId::new(did))
        })
        .collect();
    let mut new_plan = plan.clone();
    for svc in &mut new_plan.services {
        let old = svc.service_id.clone();
        svc.service_id = by_old[&old].clone();
        svc.resolved_dependencies = svc
            .resolved_dependencies
            .iter()
            .map(|(name, members)| {
                (name.clone(), members.iter().map(|m| by_old[m].clone()).collect())
            })
            .collect();
    }
    new_plan
}

fn masters_by_id(
    plan: &DeploymentPlan,
    masters: &BTreeMap<String, Identity>,
) -> BTreeMap<ServiceId, Identity> {
    plan.services
        .iter()
        .map(|s| {
            let name = s.logical_ref.service_name.as_str();
            (s.service_id.clone(), Identity::from_bytes(&masters[name].to_bytes()))
        })
        .collect()
}

/// Per-service overrides for `certify_and_publish`. A name in
/// `skip_instance_cert` gets no `service-instance` certificate at all, so
/// every proxy call it makes over the wire arrives anonymous -- the
/// outbound mirror of the missing-instance-certificate limit, at e2e scale.
#[derive(Default, Clone)]
struct CertOverrides {
    skip_instance_cert: Vec<String>,
}

async fn certify_and_publish(
    plan: &DeploymentPlan,
    masters: &BTreeMap<ServiceId, Identity>,
    names: &BTreeMap<ServiceId, String>,
    client: &Arc<SyneroymClient>,
    overrides: &CertOverrides,
) -> (BTreeMap<ServiceId, String>, BTreeMap<ServiceId, String>) {
    let mut certs = BTreeMap::new();
    let mut records = BTreeMap::new();
    for svc in &plan.services {
        let master = &masters[&svc.service_id];
        let name = &names[&svc.service_id];
        if !overrides.skip_instance_cert.contains(name) {
            let cert = certify_instance(client, master, svc.service_id.as_str(), 24).await.unwrap();
            certs.insert(svc.service_id.clone(), cert.to_json().unwrap());
        }
        if let Some(record_json) = member_registry_record(
            svc.config.visibility,
            svc.service_id.as_str(),
            client.service_id(),
            master,
            far_future_not_after(),
        )
        .unwrap()
        {
            records.insert(svc.service_id.clone(), record_json);
        }
    }
    (certs, records)
}

struct Node {
    label: &'static str,
    base_path: PathBuf,
    ports: (u16, u16, u16),
    shared_registry_url: Option<String>,
    owner: Identity,
    kek_hex: String,
    /// Stable across restarts, keyed by service name.
    masters: BTreeMap<String, Identity>,
    role: AppSandboxRole,
    cert_overrides: CertOverrides,

    substrate_client: SyneroymClient,
    registry_url: String,
    gateway_url: String,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
    /// Minted DID per service name, set by `deploy`.
    dids: BTreeMap<String, String>,
    session_token: Option<String>,
}

impl Node {
    async fn boot(
        label: &'static str,
        base_path: PathBuf,
        ports: (u16, u16, u16),
        shared_registry_url: Option<String>,
        owner: Identity,
    ) -> Self {
        let ids_dir = base_path.join("identities");
        fs::create_dir_all(&ids_dir).unwrap();
        owner.save_to_path(ids_dir.join("owner.key")).unwrap();
        Self::spawn_substrate(label, base_path, ports, shared_registry_url, &owner).await
    }

    async fn spawn_substrate(
        label: &'static str,
        base_path: PathBuf,
        ports: (u16, u16, u16),
        shared_registry_url: Option<String>,
        owner: &Identity,
    ) -> Self {
        let (iroh_port, registry_port, gateway_port) = ports;
        let kek_hex = hex::encode([0xcdu8; 32]);
        let ids_dir = base_path.join("identities");
        let role = fast_conversation_role();

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
                http_bind_address: format!("127.0.0.1:{iroh_port}"),
                ..Default::default()
            }),
            ..Default::default()
        });
        // Only the first node hosts the shared community registry; the
        // others point at it. Three registry servers plus three iroh relays
        // in one process is enough contention on a loaded machine to keep
        // the first node's own registry from binding inside its
        // registration-retry window.
        let own_registry_url = format!("http://127.0.0.1:{registry_port}");
        let effective_registry_url = match &shared_registry_url {
            Some(url) => url.clone(),
            None => {
                config.roles.community_registry = Some(ServiceRegistryRole {
                    http_bind_address: format!("127.0.0.1:{registry_port}"),
                    ..Default::default()
                });
                own_registry_url
            }
        };
        config.substrate.registry_url = Some(effective_registry_url.clone());
        config.substrate.enable_bep0044_dht = false;
        config.parent_coordinator.iroh =
            Some(IrohParentConfig { url: format!("http://127.0.0.1:{iroh_port}") });
        config.roles.client_gateway = Some(ClientGatewayRole {
            http_port: gateway_port,
            identity_mode: IdentityMode::Login,
            ..Default::default()
        });
        config.roles.auth =
            Some(AuthRole { person_identities_dir: Some(ids_dir.clone()), ..Default::default() });
        config.roles.app_sandbox = Some(role.clone());
        config.iam.admin_ucan_root = Some(substrate::derive_did_key(&owner.public_key()));

        let state = identity::setup_substrate_identity(&config.identity, &config.app_data_dir)
            .expect("failed to setup identity");
        let substrate_service_id = state.did.clone();

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

        let mut substrate_client = SyneroymClient::new_with_identity(
            substrate_service_id,
            effective_registry_url.clone(),
            Identity::from_bytes(&owner.to_bytes()),
        )
        .with_registry_dht(false);
        substrate_client.wait_for_ready(Duration::from_secs(90)).await.unwrap_or_else(|e| {
            panic!("{label} substrate not ready via {effective_registry_url}: {e}")
        });
        substrate_client.inject_kek(kek_hex.clone()).await.expect("inject_kek failed");

        Self {
            label,
            base_path,
            ports,
            shared_registry_url,
            owner: Identity::from_bytes(&owner.to_bytes()),
            kek_hex,
            masters: BTreeMap::new(),
            role,
            cert_overrides: CertOverrides::default(),
            substrate_client,
            registry_url: effective_registry_url.clone(),
            gateway_url: format!("http://127.0.0.1:{gateway_port}"),
            shutdown_tx,
            substrate_handle,
            dids: BTreeMap::new(),
            session_token: None,
        }
    }

    fn substrate_did(&self) -> String {
        self.substrate_client.service_id().to_string()
    }

    /// Compile, mint (or reuse) masters, certify, publish, apply. Publishes
    /// every service's master anchor (not only the signing ones) into the
    /// shared registry, so another node can verify an inbound directory
    /// call's `service-instance` delegation chain.
    async fn deploy(&mut self, redeploy: bool) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest_toml =
            fs::read_to_string(root.join("crates/roym_core/app/roym.toml")).unwrap();
        let manifest: SynAppManifest = toml::from_str(&manifest_toml).unwrap();
        let catalog = LocalFilesystemCatalog::new(root.clone());
        let compiled = compile(AppInstanceId::new("roym"), &manifest, &catalog).await.unwrap();
        let plan = compiled.plans.last().unwrap().clone();

        if self.masters.is_empty() {
            self.masters = mint_masters(&plan);
        }
        let mut new_plan = substitute_plan(&plan, &self.masters);
        for svc in &mut new_plan.services {
            svc.config.source = root.join(&svc.config.source).to_string_lossy().to_string();
            if let Some(assets) = svc.config.assets.as_mut() {
                assets.archive = root.join(&assets.archive).to_string_lossy().to_string();
            }
        }
        let masters = masters_by_id(&new_plan, &self.masters);
        let names: BTreeMap<ServiceId, String> = new_plan
            .services
            .iter()
            .map(|s| (s.service_id.clone(), s.logical_ref.service_name.as_str().to_string()))
            .collect();

        let mut connectable = SyneroymClient::new_with_identity(
            self.substrate_did(),
            self.registry_url.clone(),
            Identity::from_bytes(&self.owner.to_bytes()),
        )
        .with_registry_dht(false);
        connectable.connect().await.unwrap();
        let client = Arc::new(connectable);

        let alias = SubstrateAlias::new(self.label);
        let (instance_certs, registry_certs) =
            certify_and_publish(&new_plan, &masters, &names, &client, &self.cert_overrides).await;

        let targets = BTreeMap::from([(
            alias.clone(),
            DeployTarget {
                alias: Some(alias.clone()),
                substrate_did: self.substrate_did(),
                actor: deploy::build_actor(client.clone()),
            },
        )]);

        let generation = if redeploy {
            SubstrateActor::held_generation(&*client, "roym")
                .await
                .ok()
                .flatten()
                .map(|g| g + 1)
                .unwrap_or(1)
        } else {
            0
        };

        let journal = DeploymentJournal::open_in_memory().unwrap();
        let deployment_id = journal.append(&new_plan, DeploymentState::Applying).unwrap();
        let report = apply_plan(
            ApplyRequest {
                plan: &new_plan,
                targets: &targets,
                fallback: Some(&targets[&alias]),
                instance_certificates: &instance_certs,
                registry_certificates: &registry_certs,
                emit_bindings: true,
                generation,
                binding_epochs: &BTreeMap::new(),
            },
            &journal,
            deployment_id,
        )
        .await
        .unwrap();
        assert!(report.is_complete(), "{} deploy failed: {:?}", self.label, report.failures);

        self.dids = new_plan
            .services
            .iter()
            .map(|s| {
                (s.logical_ref.service_name.as_str().to_string(), s.service_id.as_str().to_string())
            })
            .collect();

        if redeploy {
            for did in self.dids.values() {
                let _ = SubstrateActor::restart(&*client, did.clone(), generation).await;
            }
        }

        let registry = RegistryClient::new(false, Some(self.registry_url.clone()));
        for (name, master) in &self.masters {
            let did = substrate::derive_did_key(&master.public_key());
            registry
                .publish_master_anchor(&did, vec![], None, master, true)
                .await
                .unwrap_or_else(|e| panic!("{}: publish {name} master anchor: {e}", self.label));
        }
    }

    fn web_host_header(&self) -> String {
        format!("s{}.localhost", short_hash(&self.dids["web"]))
    }

    async fn login(&mut self) {
        let http = Client::builder().pool_max_idle_per_host(0).build().unwrap();
        let resp = http
            .post(format!("{}/_syneroym/session/login", self.gateway_url))
            .json(&json!({ "method": "local", "identity": "owner" }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "{} login failed: {:?}", self.label, resp.text().await);
        let body: Value = resp.json().await.unwrap();
        self.session_token = Some(body["token"].as_str().unwrap().to_string());
    }

    async fn rpc(&self, method: &str, params: Value) -> Value {
        let http = Client::builder().pool_max_idle_per_host(0).build().unwrap();
        let token = self.session_token.as_deref().expect("login first");
        let resp = http
            .post(format!("{}/rpc", self.gateway_url))
            .header("Host", self.web_host_header())
            .header("Cookie", format!("{SESSION_COOKIE_NAME}={token}"))
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
            .send()
            .await
            .unwrap();
        resp.json().await.unwrap()
    }

    async fn rpc_ok(&self, method: &str, params: Value) -> Value {
        let v = self.rpc(method, params).await;
        assert!(v.get("error").is_none(), "{} {method} errored: {v}", self.label);
        v["result"].clone()
    }

    async fn rpc_err(&self, method: &str, params: Value) -> Value {
        let v = self.rpc(method, params).await;
        assert!(v.get("error").is_some(), "{} {method} unexpectedly succeeded: {v}", self.label);
        v["error"].clone()
    }

    async fn wait_for_proxy(&self) {
        let deadline = Instant::now() + Duration::from_secs(300);
        while Instant::now() < deadline {
            let v = self.rpc("directory.ping", json!({})).await;
            if v.get("result").and_then(|r| r.get("service")).is_some() {
                return;
            }
            time::sleep(Duration::from_millis(1500)).await;
        }
        panic!("{} web never proxied directory.ping after deploy", self.label);
    }

    async fn enrol_signing(&self) {
        for name in SIGNING_SERVICES {
            let status = self.rpc_ok(&format!("{name}.signing-status"), json!({})).await;
            let signing_did = status["signing_did"].as_str().unwrap().to_string();
            let pubkey = substrate::resolve_did_key(&signing_did).unwrap();
            let cert = DelegationCertificate::issue(
                &self.owner,
                pubkey,
                24 * 3600,
                SCOPE_RECORD_SIGNING.to_string(),
            )
            .unwrap();
            let install = self
                .rpc(
                    &format!("{name}.install-signing-certificate"),
                    json!({ "certificate": cert.to_json().unwrap() }),
                )
                .await;
            assert!(install.get("error").is_none(), "{} enrol {name}: {install}", self.label);
        }
    }

    async fn full_bring_up(&mut self) {
        self.deploy(false).await;
        self.login().await;
        self.wait_for_proxy().await;
        self.enrol_signing().await;
    }

    async fn teardown(mut self) {
        let _ = self.substrate_client.shutdown().await;
        let _ = self.shutdown_tx.send(()).await;
        let _ = self.substrate_handle.await;
    }
}

async fn wait_until<F, Fut>(budget: Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if check().await {
            return true;
        }
        time::sleep(Duration::from_millis(400)).await;
    }
    false
}

/// One JSON-RPC `invoke` frame delivered to `target_did`'s directory
/// interface over a real QUIC stream, from a freshly generated identity
/// with no delegation. The connection key is still verified by the
/// handshake, so the router reads `CallerOrigin::Verified(<generated
/// did>)` -- a stranger, not an anonymous caller. A truly key-less
/// `Anonymous` wire caller cannot be expressed over iroh; the parity
/// suite covers that arm with `AuthLevel::System`.
/// Returns the inner `envelope::Response`-shaped value the guest produced.
async fn stranger_wire_invoke(
    registry_url: &str,
    target_did: &str,
    method: &str,
    params: Value,
) -> Value {
    let frame = json!({ "method": method, "params": params }).to_string();
    let mut client = SyneroymClient::new_with_identity(
        target_did.to_string(),
        registry_url.to_string(),
        Identity::generate().unwrap(),
    )
    .with_registry_dht(false);
    client.connect().await.expect("anonymous caller failed to connect to the directory");
    let resp = client
        .request(DIRECTORY_INTERFACE, "invoke", json!([frame]))
        .await
        .expect("anonymous invoke returned a wire error");
    let _ = client.shutdown().await;
    let payload = resp.result.as_str().expect("guest returns a JSON string").to_string();
    serde_json::from_str(&payload).expect("guest payload is JSON")
}

fn hits(result: &Value) -> Vec<Value> {
    result["hits"].as_array().cloned().unwrap_or_default()
}

/// Drive the consumer client loop the way `roymctl roym directory find` and
/// the Hub do: `start-run`, one `query-source` per source (respecting
/// `max_concurrency`), then `merge`. Returns `(run_id, merge_result)`.
async fn run_client_loop(node: &Node, query: Value) -> (String, Value) {
    let start = node.rpc_ok("directory.start-run", json!({})).await;
    let run_id = start["run_id"].as_str().unwrap().to_string();
    let sources: Vec<String> = start["sources"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let max_concurrency = start["max_concurrency"].as_u64().unwrap_or(1).max(1) as usize;
    for chunk in sources.chunks(max_concurrency) {
        for source in chunk {
            let _ = node
                .rpc(
                    "directory.query-source",
                    json!({ "run_id": run_id, "source": source, "query": query }),
                )
                .await;
        }
    }
    let merged = node.rpc_ok("directory.merge", json!({ "run_id": run_id })).await;
    (run_id, merged)
}

async fn listing_envelope(node: &Node, listing_id: &str) -> String {
    let row = node.rpc_ok("listing.get", json!({ "listing_id": listing_id })).await;
    row["envelope"].as_str().unwrap().to_string()
}

fn listing_params(title: &str, summary: &str) -> Value {
    json!({
        "title": title,
        "summary": summary,
        "categories": ["cycling"],
        "payment": {
            "currency": "EUR", "model": "per-hour", "amount_minor": 4000,
            "tax_included": true, "payee": "provider"
        }
    })
}

/// `conversation.open` on a raw address with no contact entry, one message
/// sent and driven to `delivered` with retries.
async fn deliver_one_message(from: &Node, to_label: &str, address: &str, body: &str) {
    let opened = from.rpc_ok("conversation.open", json!({ "address": address })).await;
    let conv = opened["conversation_id"].as_str().unwrap().to_string();
    let sent =
        from.rpc_ok("conversation.send", json!({ "conversation": conv, "body": body })).await;
    let message_id = sent["message_id"].as_str().unwrap().to_string();
    assert_eq!(sent["state"], "pending", "born pending, from the host");
    let delivered = wait_until(Duration::from_secs(150), || {
        let (from, message_id) = (from, message_id.clone());
        async move {
            let _ = from.rpc("conversation.retry", json!({ "message_id": message_id })).await;
            let s = from
                .rpc_ok("conversation.delivery-status", json!({ "message_id": message_id }))
                .await;
            s["state"] == "delivered"
        }
    })
    .await;
    assert!(delivered, "{} -> {to_label} message must deliver: {body}", from.label);
}

#[tokio::test]
async fn roym_directory_search_half_across_three_substrates() {
    let _guard = SUBSTRATE_TEST_LOCK.lock().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let dir_x = tempfile::tempdir().unwrap();
    let dir_y = tempfile::tempdir().unwrap();
    let dir_z = tempfile::tempdir().unwrap();
    let owner_x = Identity::generate().unwrap();
    let owner_y = Identity::generate().unwrap();
    let owner_z = Identity::generate().unwrap();

    // --- Step 1: three nodes boot, deploy Roym, enrol signing. -----------
    let mut node_x = Node::boot(
        "node-x",
        dir_x.path().to_path_buf(),
        PORTS_X,
        None,
        Identity::from_bytes(&owner_x.to_bytes()),
    )
    .await;
    let shared_registry = node_x.registry_url.clone();
    node_x.full_bring_up().await;

    let mut node_y = Node::boot(
        "node-y",
        dir_y.path().to_path_buf(),
        PORTS_Y,
        Some(shared_registry.clone()),
        Identity::from_bytes(&owner_y.to_bytes()),
    )
    .await;
    node_y.full_bring_up().await;

    let mut node_z = Node::boot(
        "node-z",
        dir_z.path().to_path_buf(),
        PORTS_Z,
        Some(shared_registry.clone()),
        Identity::from_bytes(&owner_z.to_bytes()),
    )
    .await;
    node_z.full_bring_up().await;

    let z_dir_did = node_z.dids["directory"].clone();
    let y_conv_did = node_y.dids["conversation"].clone();
    let owner_y_did = substrate::derive_did_key(&owner_y.public_key());

    // --- Step 2: Z creates the SynOrg; X reads it back over a real
    //     transport with directory.probe-info. ---------------------------
    let settings = json!({
        "name": "South Bengaluru Trades",
        "rules": "Be honest. Show up. Fix what you break.",
        "area": [],
        "categories": ["cycling", "plumbing"],
        "support_contact": "help@example.org",
        "dispute_path": "Email support; unresolved after 14 days goes to arbitration.",
        "retention_secs": 30 * 24 * 3600,
        "publication_limits": { "window_secs": 24 * 3600, "max_per_window": 20 },
    });
    node_z.rpc_ok("directory.set-settings", settings).await;

    let z_info = node_x.rpc_ok("directory.probe-info", json!({ "did": z_dir_did })).await;
    assert_eq!(
        z_info["name"], "South Bengaluru Trades",
        "X reads Z's SynOrg over the wire: {z_info}"
    );
    assert_eq!(z_info["retention_secs"], 30 * 24 * 3600);
    assert!(z_info.get("members").is_none(), "directory.info carries no roster: {z_info}");
    assert_eq!(z_info["member_count"], 0);

    // --- Step 3: Y creates a profile and a signed listing, in no
    //     directory. -----------------------------------------------------
    node_y
        .rpc_ok(
            "profile.set",
            json!({ "display_name": "Yara", "conversation_address": y_conv_did }),
        )
        .await;
    let y_listing = node_y
        .rpc_ok("listing.set", listing_params("Bike repair", "Same-day, at your door."))
        .await;
    let y_listing_id = y_listing["listing_id"].as_str().unwrap().to_string();
    let y_envelope = listing_envelope(&node_y, &y_listing_id).await;

    // --- Step 4: X reaches Y by direct link, no directory in the path.
    //     Runs BEFORE any publication exists anywhere. ------------------
    let x_verify = node_x.rpc_ok("listing.verify", json!({ "envelope": y_envelope })).await;
    assert_eq!(x_verify["verified"], true, "X verifies Y's listing with no directory: {x_verify}");
    assert_eq!(x_verify["conversation_address"], y_conv_did);
    deliver_one_message(&node_x, "node-y", &y_conv_did, "hello via direct link").await;

    // --- Step 5: Z adds Y to the roster. No credential is issued
    //     (a later cross-installation-trust concern). ------------------
    let member = node_z
        .rpc_ok("member.add", json!({ "did": owner_y_did, "note": "verified provider" }))
        .await;
    assert_eq!(member["did"], owner_y_did);
    let z_info_after = node_x.rpc_ok("directory.probe-info", json!({ "did": z_dir_did })).await;
    assert_eq!(z_info_after["member_count"], 1, "info reports the roster size, not the roster");

    // --- Step 6: Y publishes to Z through directory.publish-to-source,
    //     over the wire, verified. --------------------------------------
    node_y
        .rpc_ok(
            "directory.add-source",
            json!({ "did": z_dir_did, "label": "South Bengaluru Trades" }),
        )
        .await;
    let published = node_y
        .rpc_ok(
            "directory.publish-to-source",
            json!({ "source": z_dir_did, "listing_id": y_listing_id }),
        )
        .await;
    assert_eq!(published["listing_id"], y_listing_id, "Y publishes to Z: {published}");
    let z_pubs = node_z.rpc_ok("directory.publications", json!({})).await;
    assert_eq!(
        z_pubs["publications"].as_array().map(Vec::len),
        Some(1),
        "Z holds exactly Y's one publication: {z_pubs}"
    );

    // --- Step 7: X adds Z as a source and runs the client loop. --------
    let x_add = node_x.rpc_ok("directory.add-source", json!({ "did": z_dir_did })).await;
    assert!(x_add["source"]["last_error"].is_null(), "X's probe of Z succeeded: {x_add}");

    let (run_id, merged) = run_client_loop(&node_x, json!({ "categories": ["cycling"] })).await;
    let merged_hits = hits(&merged);
    assert_eq!(merged_hits.len(), 1, "one hit from Z: {merged}");
    let hit = merged_hits[0].clone();
    assert_eq!(hit["listing_id"], y_listing_id);
    assert_eq!(hit["issuer"], owner_y_did);
    assert_eq!(hit["revocation_status"], "unknown", "revocation renders unknown, never positive");
    assert_eq!(hit["credential"], "unknown", "membership renders unknown, never positive");
    assert!(hit["verified"].as_bool().unwrap_or(false), "X's own verdict, not Z's: {hit}");
    assert!(
        hit["age_secs"].as_u64().unwrap() < 3600,
        "age is computed on X's own clock, not Z's received_at: {hit}"
    );
    let hit_sources = hit["sources"].as_array().cloned().unwrap_or_default();
    assert_eq!(hit_sources.len(), 1);
    assert_eq!(hit_sources[0]["directory"], z_dir_did, "the source is Z: {hit}");
    assert!(merged["refused"].as_array().is_none_or(|r| r.is_empty()), "nothing refused: {merged}");

    let record_id = hit["record_id"].as_str().unwrap().to_string();
    let run_env = node_x
        .rpc_ok("directory.run-envelope", json!({ "run_id": run_id, "record_id": record_id }))
        .await;
    assert_eq!(
        run_env["envelope"].as_str(),
        Some(y_envelope.as_str()),
        "run-envelope returns bytes byte-identical to what Y signed"
    );

    // --- Step 8: X starts a conversation from the search result's own
    //     conversation_address, with no prior contact entry. -----------
    let result_address = hit["conversation_address"].as_str().unwrap().to_string();
    assert_eq!(result_address, y_conv_did);
    deliver_one_message(&node_x, "node-y", &result_address, "found you through the directory")
        .await;

    // --- Step 9: a stranger dialling in over the wire from a self-minted
    //     identity reaches directory.search, is refused member.list, and
    //     -- because a generated key is still a *verified* connection --
    //     is admitted to the VerifiedOnly directory.publish. -------------
    let stranger_search = stranger_wire_invoke(
        &shared_registry,
        &z_dir_did,
        "directory.search",
        json!({ "categories": ["cycling"] }),
    )
    .await;
    assert!(
        stranger_search["result"]["hits"].as_array().is_some(),
        "a stranger may read a directory: {stranger_search}"
    );
    let stranger_members =
        stranger_wire_invoke(&shared_registry, &z_dir_did, "member.list", json!({})).await;
    assert_eq!(
        stranger_members["error"]["code"], -32013,
        "member.list is never reachable off the node: {stranger_members}"
    );
    // `member.list` being refused proves only that the method is
    // unlisted, not anything about the caller's identity. `search`
    // (Open) and `publish` (VerifiedOnly) together do: this stranger is
    // admitted to both, which is only possible for a `Verified` caller.
    // A key-less, truly `Anonymous` wire caller -- refused `publish`
    // with -32013 -- is not expressible over iroh (every connection is
    // keyed); that arm is covered by parity scenario 80.
    let stranger_publish = stranger_wire_invoke(
        &shared_registry,
        &z_dir_did,
        "directory.publish",
        json!({ "envelope": y_envelope }),
    )
    .await;
    assert_ne!(
        stranger_publish["error"]["code"].as_i64(),
        Some(-32013),
        "a self-minted identity is a verified connection, so VerifiedOnly admits it: \
         {stranger_publish}"
    );

    // --- Step 10: Y publishes past the limit and is refused with a
    //     retry_after_secs, visible to Y. -------------------------------
    node_z
        .rpc_ok("directory.set-limits", json!({ "window_secs": 24 * 3600, "max_per_window": 1 }))
        .await;
    let y_listing_2 = node_y
        .rpc_ok("listing.set", listing_params("Wheel truing", "Bring the wheel, wait ten minutes."))
        .await;
    let y_listing_2_id = y_listing_2["listing_id"].as_str().unwrap().to_string();
    let refused = node_y
        .rpc_err(
            "directory.publish-to-source",
            json!({ "source": z_dir_did, "listing_id": y_listing_2_id }),
        )
        .await;
    assert_eq!(refused["code"], -32602, "over the limit is refused visibly: {refused}");
    assert!(
        refused["data"]["retry_after_secs"].as_u64().unwrap_or(0) > 0,
        "the refusal carries a retry_after_secs Y can act on: {refused}"
    );
    node_z
        .rpc_ok("directory.set-limits", json!({ "window_secs": 24 * 3600, "max_per_window": 20 }))
        .await;

    // --- Step 11: Z unpublishes Y's listing; X's next search no longer
    //     returns it, and X's already-held copy is untouched. ----------
    node_z.rpc_ok("directory.unpublish", json!({ "listing_id": y_listing_id })).await;
    let (_, after_unpublish) = run_client_loop(&node_x, json!({ "categories": ["cycling"] })).await;
    assert!(
        hits(&after_unpublish).is_empty(),
        "the unpublished listing is gone from Z's search: {after_unpublish}"
    );
    let held_still_valid = node_x.rpc_ok("listing.verify", json!({ "envelope": y_envelope })).await;
    assert_eq!(
        held_still_valid["verified"], true,
        "unpublish never touches a copy the consumer already holds"
    );

    // --- Step 12: the no-directory regression. X removes Z as a source
    //     and step 4's whole path is re-run and passes -- at the end,
    //     after a directory has existed. -----------------------------
    node_x.rpc_ok("directory.remove-source", json!({ "did": z_dir_did })).await;
    let (_, empty_run) = run_client_loop(&node_x, json!({})).await;
    assert!(hits(&empty_run).is_empty(), "no sources -> zero hits, no error: {empty_run}");
    let reverify = node_x.rpc_ok("listing.verify", json!({ "envelope": y_envelope })).await;
    assert_eq!(reverify["verified"], true, "the direct-link path still works with no directory");
    deliver_one_message(&node_x, "node-y", &y_conv_did, "still reachable without a directory")
        .await;

    // --- Step 13: two directories disagree about a version. Y's own node
    //     becomes a second SynOrg holding the older version of Y's
    //     listing; a search over both merges to one hit with
    //     versions_differ. ---------------------------------------------
    node_y
        .rpc_ok(
            "directory.set-settings",
            json!({
                "name": "Yara's picks",
                "rules": "personal recommendations",
                "area": [],
                "categories": ["cycling"],
                "support_contact": "yara@example.org",
                "dispute_path": "n/a",
                "retention_secs": 30 * 24 * 3600,
                "publication_limits": { "window_secs": 24 * 3600, "max_per_window": 20 },
            }),
        )
        .await;
    // The older version to Y's own directory (local publish, Internal
    // caller); a fresh, current version to Z.
    node_y.rpc_ok("directory.publish", json!({ "envelope": y_envelope })).await;
    node_y
        .rpc_ok(
            "listing.set",
            json!({
                "listing_id": y_listing_id,
                "title": "Bike repair",
                "summary": "Same-day, at your door. Now with loan bikes.",
                "categories": ["cycling"],
                "payment": {
                    "currency": "EUR", "model": "per-hour", "amount_minor": 4500,
                    "tax_included": true, "payee": "provider"
                }
            }),
        )
        .await;
    node_y
        .rpc_ok(
            "directory.publish-to-source",
            json!({ "source": z_dir_did, "listing_id": y_listing_id }),
        )
        .await;

    let y_dir_did = node_y.dids["directory"].clone();
    node_x.rpc_ok("directory.add-source", json!({ "did": z_dir_did })).await;
    node_x.rpc_ok("directory.add-source", json!({ "did": y_dir_did })).await;
    let (_, two_dir) = run_client_loop(&node_x, json!({ "categories": ["cycling"] })).await;
    let two_hits = hits(&two_dir);
    assert_eq!(two_hits.len(), 1, "the two directories merge to one hit: {two_dir}");
    assert_eq!(two_hits[0]["versions_differ"], true, "the disagreement is surfaced: {two_dir}");
    assert_eq!(
        two_hits[0]["sources"].as_array().map(Vec::len),
        Some(2),
        "both directories are named in sources[]: {two_dir}"
    );

    // --- Step 13b: the loop at MAX_SOURCES, most sources unreachable. --
    for i in 0..6 {
        let bogus = substrate::derive_did_key(&Identity::generate().unwrap().public_key());
        node_x
            .rpc_ok("directory.add-source", json!({ "did": bogus, "label": format!("bogus-{i}") }))
            .await;
    }
    let (_, ceiling_run) = run_client_loop(&node_x, json!({ "categories": ["cycling"] })).await;
    assert_eq!(
        hits(&ceiling_run).len(),
        1,
        "the two live directories still contribute: {ceiling_run}"
    );
    let x_sources = node_x.rpc_ok("directory.sources", json!({})).await;
    let source_rows = x_sources["sources"].as_array().cloned().unwrap_or_default();
    assert_eq!(source_rows.len(), 8, "X holds MAX_SOURCES sources: {x_sources}");
    let errored = source_rows.iter().filter(|s| !s["last_error"].is_null()).count();
    assert!(errored >= 6, "the unreachable sources each carry an error: {x_sources}");
    for s in &source_rows {
        assert_ne!(
            s["last_error"]["kind"], "not-started",
            "no source is blamed for this node's own admission limit: {s}"
        );
    }

    // --- Step 14: export / import of Z's directory bundles, then
    //     reindex, then an identical search. ---------------------------
    let z_before = node_z.rpc_ok("directory.search", json!({ "categories": ["cycling"] })).await;
    let z_bundle = node_z.rpc_ok("directory.export", json!({})).await;
    node_z.rpc_ok("directory.import", json!({ "bundle": z_bundle })).await;
    node_z.rpc_ok("directory.reindex", json!({})).await;
    let z_after = node_z.rpc_ok("directory.search", json!({ "categories": ["cycling"] })).await;
    assert_eq!(
        hits(&z_before).len(),
        hits(&z_after).len(),
        "search returns the same after export/import/reindex: {z_after}"
    );

    // --- Step 7b: the certificate dependency, over a real transport.
    //     A fresh provider whose directory service holds no instance
    //     certificate at all: publish-to-source fails with -32013,
    //     indistinguishable from "not yours to call". X stays up so the
    //     shared registry keeps resolving Z's directory for the newcomer.
    let dir_w = tempfile::tempdir().unwrap();
    let owner_w = Identity::generate().unwrap();
    let mut node_w = Node::boot(
        "node-w",
        dir_w.path().to_path_buf(),
        PORTS_W,
        Some(shared_registry.clone()),
        Identity::from_bytes(&owner_w.to_bytes()),
    )
    .await;
    node_w.cert_overrides.skip_instance_cert = vec!["directory".to_string()];
    node_w.full_bring_up().await;
    node_w
        .rpc_ok(
            "profile.set",
            json!({ "display_name": "Wes", "conversation_address": node_w.dids["conversation"] }),
        )
        .await;
    let w_listing =
        node_w.rpc_ok("listing.set", listing_params("Frame welding", "Steel and titanium.")).await;
    let w_listing_id = w_listing["listing_id"].as_str().unwrap().to_string();
    node_w.rpc_ok("directory.add-source", json!({ "did": z_dir_did })).await;
    let w_refused = node_w
        .rpc_err(
            "directory.publish-to-source",
            json!({ "source": z_dir_did, "listing_id": w_listing_id }),
        )
        .await;
    let w_text = w_refused.to_string();
    assert!(
        w_text.contains("32013"),
        "a stale/absent instance cert reads as 'not yours to call': {w_refused}"
    );

    node_w.teardown().await;
    node_z.teardown().await;
    node_y.teardown().await;
    node_x.teardown().await;
}
