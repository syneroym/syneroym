#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]
//! The Roym product's transaction vertical: requests, quotes, and agreement
//! receipts across two independent `syneroym-substrate` instances, each
//! running the full Roym SynApp (the `wasm32-wasip2` build) under its own owner
//! identity.
//!
//! Proves an offer is agreed across two installations:
//! consumer finds provider by signed listing without a directory, sends a
//! signed request, receives a signed quote, and accepts it. Provider
//! countersigns and both parties reach a completed agreement pair with
//! identical terms. Also proves tampered cards are refused and never verified.

use std::{
    collections::BTreeMap,
    fs,
    future::Future,
    mem,
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
use syneroym_roym_core::{
    card::{CARD_CONTENT_TYPE, card_body},
    record::RECORD_REQUEST,
    transaction::{DEFAULT_DATA_USE_NOTICE, REQUEST_VERSION},
};
use syneroym_sdk::{
    SyneroymClient,
    deploy::{
        self, ApplyRequest, DeployTarget, SubstrateActor, apply_plan, certify_instance,
        member_registry_record,
    },
};
use syneroym_signed_record::{Envelope, SCOPE_RECORD_SIGNING};
use syneroym_substrate::identity;
use tokio::{
    sync::{Mutex, mpsc, mpsc::Sender},
    task::JoinHandle,
    time,
};

mod common;
use common::alloc_ports;

const SESSION_COOKIE_NAME: &str = "syneroym_session";

static SUBSTRATE_TEST_LOCK: Mutex<()> = Mutex::const_new(());

const SIGNING_SERVICES: &[&str] = &["profile", "catalog", "conversation", "transaction"];

fn far_future_not_after() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS)
}

fn fast_conversation_role(max_pending_age_secs: u64) -> AppSandboxRole {
    AppSandboxRole {
        conversation_tick_secs: 1,
        conversation_max_pending_age_secs: max_pending_age_secs,
        ..AppSandboxRole::default()
    }
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

async fn certify_and_publish(
    plan: &DeploymentPlan,
    masters: &BTreeMap<ServiceId, Identity>,
    client: &Arc<SyneroymClient>,
) -> (BTreeMap<ServiceId, String>, BTreeMap<ServiceId, String>) {
    let mut certs = BTreeMap::new();
    let mut records = BTreeMap::new();
    for svc in &plan.services {
        let master = &masters[&svc.service_id];
        let cert = certify_instance(client, master, svc.service_id.as_str(), 24).await.unwrap();
        certs.insert(svc.service_id.clone(), cert.to_json().unwrap());
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
    ports: (u16, u16, u16, u16),
    shared_registry_url: Option<String>,
    owner: Identity,
    kek_hex: String,
    masters: BTreeMap<String, Identity>,
    role: AppSandboxRole,

    substrate_client: SyneroymClient,
    registry_url: String,
    gateway_url: String,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
    dids: BTreeMap<String, String>,
    session_token: Option<String>,
}

impl Node {
    async fn boot(
        label: &'static str,
        base_path: PathBuf,
        ports: (u16, u16, u16, u16),
        shared_registry_url: Option<String>,
        owner: Identity,
        role: AppSandboxRole,
    ) -> Self {
        let ids_dir = base_path.join("identities");
        fs::create_dir_all(&ids_dir).unwrap();
        owner.save_to_path(ids_dir.join("owner.key")).unwrap();

        Self::spawn_substrate(label, base_path, ports, shared_registry_url, &owner, role).await
    }

    async fn spawn_substrate(
        label: &'static str,
        base_path: PathBuf,
        ports: (u16, u16, u16, u16),
        shared_registry_url: Option<String>,
        owner: &Identity,
        role: AppSandboxRole,
    ) -> Self {
        let (iroh_port, registry_port, gateway_port, quic_port) = ports;
        let kek_hex = hex::encode([0xcdu8; 32]);
        let ids_dir = base_path.join("identities");

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
                quic_bind_address: format!("127.0.0.1:{quic_port}"),
                ..Default::default()
            }),
            ..Default::default()
        });
        config.roles.community_registry = Some(ServiceRegistryRole {
            http_bind_address: format!("127.0.0.1:{registry_port}"),
            ..Default::default()
        });
        let own_registry_url = format!("http://127.0.0.1:{registry_port}");
        let effective_registry_url = shared_registry_url.clone().unwrap_or(own_registry_url);
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
        substrate_client
            .wait_for_ready(Duration::from_secs(30))
            .await
            .expect("substrate did not become available in time");
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
            certify_and_publish(&new_plan, &masters, &client).await;

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
        for name in SIGNING_SERVICES {
            let master = &self.masters[*name];
            let did = substrate::derive_did_key(&master.public_key());
            registry
                .publish_master_anchor(&did, vec![], None, master, true)
                .await
                .expect("failed to publish a service master anchor");
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

    async fn wait_for_proxy(&self) {
        let deadline = Instant::now() + Duration::from_secs(300);
        while Instant::now() < deadline {
            let v = self.rpc("conversation.ping", json!({})).await;
            if v.get("result").and_then(|r| r.get("service")).is_some() {
                return;
            }
            time::sleep(Duration::from_millis(1500)).await;
        }
        panic!("{} web never proxied conversation.ping after deploy", self.label);
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

    async fn stop(&mut self, wipe_service_state: Option<&str>) {
        let _ = self.substrate_client.shutdown().await;
        let _ = self.shutdown_tx.send(()).await;
        let handle = mem::replace(&mut self.substrate_handle, tokio::spawn(async {}));
        let _ = handle.await;
        time::sleep(Duration::from_secs(3)).await;

        if let Some(name) = wipe_service_state
            && let Some(did) = self.dids.get(name)
        {
            let dir = self.base_path.join("data/db/services").join(did);
            for f in ["state.db", "state.db-wal", "state.db-shm"] {
                let _ = fs::remove_file(dir.join(f));
            }
        }
        self.session_token = None;
    }

    async fn resume(&mut self, new_role: Option<AppSandboxRole>) {
        if let Some(role) = new_role {
            self.role = role;
        }
        let masters = mem::take(&mut self.masters);
        let owner = Identity::from_bytes(&self.owner.to_bytes());
        let fresh = Self::spawn_substrate(
            self.label,
            self.base_path.clone(),
            self.ports,
            self.shared_registry_url.clone(),
            &owner,
            self.role.clone(),
        )
        .await;
        self.substrate_client = fresh.substrate_client;
        self.registry_url = fresh.registry_url;
        self.gateway_url = fresh.gateway_url;
        self.shutdown_tx = fresh.shutdown_tx;
        self.substrate_handle = fresh.substrate_handle;
        self.masters = masters;
        self.session_token = None;

        self.deploy(true).await;
        self.republish_registry().await;
        self.login().await;
        self.wait_for_proxy().await;
    }

    async fn restart(
        &mut self,
        new_role: Option<AppSandboxRole>,
        wipe_service_state: Option<&str>,
    ) {
        self.stop(wipe_service_state).await;
        self.resume(new_role).await;
    }

    async fn republish_registry(&self) {
        let registry = RegistryClient::new(false, Some(self.registry_url.clone()));
        let http = Client::new();
        for (name, did) in &self.dids {
            let master = &self.masters[name];
            let visibility = service_visibility(name);
            if let Ok(Some(record)) = member_registry_record(
                visibility,
                did,
                &self.substrate_did(),
                master,
                far_future_not_after(),
            ) {
                let _ = http
                    .post(format!("{}/register", self.registry_url))
                    .body(record)
                    .header("content-type", "application/json")
                    .send()
                    .await;
            }
            let _ = registry.publish_master_anchor(did, vec![], None, master, true).await;
        }
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

fn history_messages(result: &Value) -> Vec<Value> {
    result["messages"].as_array().cloned().unwrap_or_default()
}

async fn wait_delivered(node: &Node, message_id: &str) -> bool {
    let _ = node.rpc("conversation.retry", json!({ "message_id": message_id })).await;
    wait_until(Duration::from_secs(90), || async {
        let _ = node.rpc("conversation.retry", json!({ "message_id": message_id })).await;
        let s =
            node.rpc_ok("conversation.delivery-status", json!({ "message_id": message_id })).await;
        s["state"] == "delivered"
    })
    .await
}

#[tokio::test]
async fn an_offer_is_agreed_across_two_installations() {
    let _guard = SUBSTRATE_TEST_LOCK.lock().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let dir_x = tempfile::tempdir().unwrap();
    let dir_y = tempfile::tempdir().unwrap();
    let owner_x = Identity::generate().unwrap();
    let owner_y = Identity::generate().unwrap();
    let owner_x_did = substrate::derive_did_key(&owner_x.public_key());
    let owner_y_did = substrate::derive_did_key(&owner_y.public_key());

    let [a_iroh, a_reg, a_gw, a_quic] = alloc_ports();
    let [b_iroh, b_reg, b_gw, b_quic] = alloc_ports();
    let ports_a = (a_iroh, a_reg, a_gw, a_quic);
    let ports_b = (b_iroh, b_reg, b_gw, b_quic);

    // Step 1: Boot X and Y; deploy Roym on both; enrol signing on all 4 services on
    // each.
    let mut node_x = Node::boot(
        "node-x",
        dir_x.path().to_path_buf(),
        ports_a,
        None,
        Identity::from_bytes(&owner_x.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    let shared_registry = node_x.registry_url.clone();
    node_x.full_bring_up().await;

    let mut node_y = Node::boot(
        "node-y",
        dir_y.path().to_path_buf(),
        ports_b,
        Some(shared_registry.clone()),
        Identity::from_bytes(&owner_y.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    node_y.full_bring_up().await;

    let x_conv_did = node_x.dids["conversation"].clone();
    let y_conv_did = node_y.dids["conversation"].clone();

    // Step 2: Y sets a profile and an active listing; X sets a profile.
    node_x
        .rpc_ok("profile.set", json!({ "display_name": "X", "conversation_address": x_conv_did }))
        .await;
    node_y
        .rpc_ok("profile.set", json!({ "display_name": "Y", "conversation_address": y_conv_did }))
        .await;

    let y_listing = node_y
        .rpc_ok(
            "listing.set",
            json!({
                "title": "Bike repair",
                "summary": "Same-day service at your door",
                "categories": ["cycling"],
                "payment": {
                    "currency": "EUR", "model": "fixed", "amount_minor": 4500,
                    "tax_included": true, "payee": "Y Repairs"
                }
            }),
        )
        .await;
    let y_listing_id = y_listing["listing_id"].as_str().unwrap().to_string();
    let y_listing_row = node_y.rpc_ok("listing.get", json!({ "listing_id": y_listing_id })).await;
    let y_listing_envelope = y_listing_row["envelope"].as_str().unwrap().to_string();

    let x_verify = node_x.rpc_ok("listing.verify", json!({ "envelope": y_listing_envelope })).await;
    assert_eq!(x_verify["verified"], true);
    let y_conv_address = x_verify["conversation_address"].as_str().unwrap().to_string();
    assert_eq!(y_conv_address, y_conv_did);

    // Step 3: X conversation.open to Y's conversation address from the signed
    // listing.
    let opened = node_x.rpc_ok("conversation.open", json!({ "address": y_conv_address })).await;
    let x_conv_id = opened["conversation_id"].as_str().unwrap().to_string();

    // Step 4: X request.set -> a card is sent. Wait for delivered.
    let req_res = node_x
        .rpc_ok(
            "request.set",
            json!({
                "conversation": x_conv_id,
                "description": "Fix my front brake cable",
                "categories": ["cycling"],
                "data_use_notice": DEFAULT_DATA_USE_NOTICE,
            }),
        )
        .await;
    let req_record_id = req_res["record_id"].as_str().unwrap().to_string();
    let req_msg_id = req_res["message_id"].as_str().unwrap().to_string();

    let delivered = wait_delivered(&node_x, &req_msg_id).await;
    assert!(delivered, "request card was delivered to Y");

    // Step 5: Y transaction.sync { conversation } -> filed: 1; transaction.thread
    // shows verified request.
    let y_has_conv = wait_until(Duration::from_secs(20), || async {
        let list = node_y.rpc_ok("conversation.list", json!({})).await;
        list["conversations"].as_array().map(|c| !c.is_empty()).unwrap_or(false)
    })
    .await;
    assert!(y_has_conv, "Y has received the conversation");
    let y_convs = node_y.rpc_ok("conversation.list", json!({})).await;
    let y_conv_id = y_convs["conversations"][0]["id"].as_str().unwrap().to_string();

    let y_sync = node_y.rpc_ok("transaction.sync", json!({ "conversation": y_conv_id })).await;
    assert_eq!(y_sync["filed"], 1);

    let y_thread = node_y.rpc_ok("transaction.thread", json!({ "conversation": y_conv_id })).await;
    let y_cards = y_thread["cards"].as_array().unwrap();
    assert_eq!(y_cards.len(), 1);
    assert_eq!(y_cards[0]["verified"], true);
    assert_eq!(y_cards[0]["card_type"], "request");
    assert_eq!(y_cards[0]["issuer"], owner_x_did);

    // Step 6: Y quote.set with every AgreedTerms field filled and expires_in_secs =
    // 3600 -> card sent.
    let quote_res = node_y
        .rpc_ok(
            "quote.set",
            json!({
                "request_record_id": req_record_id,
                "expires_in_secs": 3600,
                "terms": {
                    "scope": "Replace brake cable and adjust pads",
                    "currency": "EUR",
                    "amount_minor": 4500,
                    "tax_minor": 500,
                    "fees_minor": 0,
                    "payment_methods": ["cash", "sepa"],
                    "payee": "Y Repairs",
                    "payment_timing": "after-work",
                    "schedule": {
                        "earliest_secs": 1800000000,
                        "latest_secs": 1800003600
                    },
                    "location": {
                        "where": "at-customer",
                        "address": "123 High Street"
                    },
                    "cancellation_terms": "24 hours notice required for full refund",
                    "refund_terms": "Full refund if work not completed",
                    "dispute_path": "Small claims court or informal mediation"
                }
            }),
        )
        .await;
    let quote_record_id = quote_res["record_id"].as_str().unwrap().to_string();
    let quote_msg_id = quote_res["message_id"].as_str().unwrap().to_string();

    let quote_delivered = wait_delivered(&node_y, &quote_msg_id).await;
    assert!(quote_delivered, "quote card was delivered to X");

    // Step 7: X transaction.sync -> files the quote; thread shows it verified with
    // X as consumer_did.
    let x_sync = node_x.rpc_ok("transaction.sync", json!({ "conversation": x_conv_id })).await;
    assert_eq!(x_sync["filed"], 1);

    let x_thread = node_x.rpc_ok("transaction.thread", json!({ "conversation": x_conv_id })).await;
    let x_cards = x_thread["cards"].as_array().unwrap();
    let quote_card =
        x_cards.iter().find(|c| c["card_type"] == "quote").expect("quote card present");
    assert_eq!(quote_card["verified"], true);
    assert_eq!(quote_card["data"]["consumer_did"], owner_x_did);

    // Step 8: X agreement.accept { quote_record_id } -> role: consumer, pair: half.
    let accept_res =
        node_x.rpc_ok("agreement.accept", json!({ "quote_record_id": quote_record_id })).await;
    assert_eq!(accept_res["role"], "consumer");
    assert_eq!(accept_res["pair"]["state"], "half");
    assert_eq!(accept_res["pair"]["role"], "consumer");
    let accept_msg_id = accept_res["message_id"].as_str().unwrap().to_string();

    let accept_delivered = wait_delivered(&node_x, &accept_msg_id).await;
    assert!(accept_delivered, "accept card was delivered to Y");

    // Step 9: Y transaction.sync -> countersigned: 1; agreement.get reports pair:
    // complete.
    let y_sync2 = node_y.rpc_ok("transaction.sync", json!({ "conversation": y_conv_id })).await;
    assert_eq!(y_sync2["countersigned"], 1);

    let y_agr = node_y.rpc_ok("agreement.get", json!({ "quote_record_id": quote_record_id })).await;
    assert_eq!(y_agr["pair"]["state"], "complete");
    assert!(y_agr["consumer"].is_object());
    assert!(y_agr["provider"].is_object());

    // Wait until X receives the provider's countersigned card
    let y_thread2 = node_y.rpc_ok("transaction.thread", json!({ "conversation": y_conv_id })).await;
    let y_cards2 = y_thread2["cards"].as_array().unwrap();
    let y_prov_card = y_cards2
        .iter()
        .find(|c| c["card_type"] == "agreement-receipt" && c["issuer"] == owner_y_did)
        .expect("provider card in Y thread");
    let prov_msg_id = y_prov_card["message_id"].as_str().unwrap();
    let prov_delivered = wait_delivered(&node_y, prov_msg_id).await;
    assert!(prov_delivered, "countersigned receipt delivered to X");

    // Step 10: X transaction.sync -> X agreement.get reports pair: complete,
    // payloads differ only in role.
    let _x_sync2 = node_x.rpc_ok("transaction.sync", json!({ "conversation": x_conv_id })).await;
    let x_agr = node_x.rpc_ok("agreement.get", json!({ "quote_record_id": quote_record_id })).await;
    assert_eq!(x_agr["pair"]["state"], "complete");
    assert!(x_agr["consumer"].is_object());
    assert!(x_agr["provider"].is_object());

    let x_consumer_env =
        Envelope::from_json(x_agr["consumer"]["envelope"].as_str().unwrap()).unwrap();
    let x_provider_env =
        Envelope::from_json(x_agr["provider"]["envelope"].as_str().unwrap()).unwrap();
    let mut c_payload = x_consumer_env.payload.clone();
    let mut p_payload = x_provider_env.payload.clone();
    assert_eq!(c_payload["role"], "consumer");
    assert_eq!(p_payload["role"], "provider");
    c_payload["role"] = json!("same");
    p_payload["role"] = json!("same");
    assert_eq!(c_payload, p_payload);

    // Step 11: Every field the Records table names is present on both halves.
    for half_payload in [&x_consumer_env.payload, &x_provider_env.payload] {
        assert_eq!(half_payload["consumer_did"], owner_x_did);
        assert_eq!(half_payload["provider_did"], owner_y_did);
        assert_eq!(half_payload["quote_record_id"], quote_record_id);
        let terms = &half_payload["terms"];
        assert!(terms["payee"].is_string() && !terms["payee"].as_str().unwrap().is_empty());
        assert!(
            terms["quote_expires_at_secs"].is_u64()
                && terms["quote_expires_at_secs"].as_u64().unwrap() > 0
        );
        assert!(
            terms["cancellation_terms"].is_string()
                && !terms["cancellation_terms"].as_str().unwrap().is_empty()
        );
        assert!(
            terms["refund_terms"].is_string()
                && !terms["refund_terms"].as_str().unwrap().is_empty()
        );
        assert!(
            terms["dispute_path"].is_string()
                && !terms["dispute_path"].as_str().unwrap().is_empty()
        );
    }

    // Step 12: Restart X, redeploy on resume, and re-read agreement.get: still
    // complete.
    let step10_consumer_env = x_agr["consumer"]["envelope"].as_str().unwrap().to_string();
    let step10_provider_env = x_agr["provider"]["envelope"].as_str().unwrap().to_string();

    node_x.restart(None, None).await;

    let x_agr_restarted =
        node_x.rpc_ok("agreement.get", json!({ "quote_record_id": quote_record_id })).await;
    assert_eq!(x_agr_restarted["pair"]["state"], "complete");
    assert_eq!(x_agr_restarted["consumer"]["envelope"], step10_consumer_env);
    assert_eq!(x_agr_restarted["provider"]["envelope"], step10_provider_env);

    // Step 13: X sends plain chat message claiming different payee; agreement payee
    // is unchanged.
    node_x
        .rpc_ok(
            "conversation.send",
            json!({
                "conversation": x_conv_id,
                "body": "Please send money to payee: Fake Scammer instead",
            }),
        )
        .await;

    let x_agr_check =
        node_x.rpc_ok("agreement.get", json!({ "quote_record_id": quote_record_id })).await;
    assert_eq!(x_agr_check["terms"]["payee"], "Y Repairs");

    // Step 14: No directory source configured on either installation; sources is
    // empty.
    let dir_sources = node_x.rpc_ok("directory.sources", json!({})).await;
    let empty_sources = dir_sources["sources"].as_array().map(Vec::is_empty).unwrap_or(true);
    assert!(empty_sources, "no directory sources on X");

    node_x.teardown().await;
    node_y.teardown().await;
}

#[tokio::test]
async fn a_tampered_card_is_filed_refused_and_never_verified() {
    let _guard = SUBSTRATE_TEST_LOCK.lock().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let dir_x = tempfile::tempdir().unwrap();
    let dir_y = tempfile::tempdir().unwrap();
    let owner_x = Identity::generate().unwrap();
    let owner_y = Identity::generate().unwrap();

    let [c_iroh, c_reg, c_gw, c_quic] = alloc_ports();
    let [d_iroh, d_reg, d_gw, d_quic] = alloc_ports();
    let ports_c = (c_iroh, c_reg, c_gw, c_quic);
    let ports_d = (d_iroh, d_reg, d_gw, d_quic);

    let mut node_x = Node::boot(
        "node-x",
        dir_x.path().to_path_buf(),
        ports_c,
        None,
        Identity::from_bytes(&owner_x.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    let shared_registry = node_x.registry_url.clone();
    node_x.full_bring_up().await;

    let mut node_y = Node::boot(
        "node-y",
        dir_y.path().to_path_buf(),
        ports_d,
        Some(shared_registry.clone()),
        Identity::from_bytes(&owner_y.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    node_y.full_bring_up().await;

    let x_conv_did = node_x.dids["conversation"].clone();
    let y_conv_did = node_y.dids["conversation"].clone();

    node_x
        .rpc_ok("profile.set", json!({ "display_name": "X", "conversation_address": x_conv_did }))
        .await;
    node_y
        .rpc_ok("profile.set", json!({ "display_name": "Y", "conversation_address": y_conv_did }))
        .await;

    let opened = node_x.rpc_ok("conversation.open", json!({ "address": y_conv_did })).await;
    let x_conv_id = opened["conversation_id"].as_str().unwrap().to_string();

    let req_res = node_x
        .rpc_ok(
            "request.set",
            json!({
                "conversation": x_conv_id,
                "description": "Legitimate request description",
                "categories": ["cycling"],
                "data_use_notice": DEFAULT_DATA_USE_NOTICE,
            }),
        )
        .await;
    let req_id = req_res["request_id"].as_str().unwrap().to_string();
    let req_get = node_x.rpc_ok("request.get", json!({ "request_id": req_id })).await;
    let valid_envelope_str = req_get["envelope"].as_str().unwrap();

    // Tamper the payload of the envelope: change one field without updating
    // signature
    let mut env_val: Value = serde_json::from_str(valid_envelope_str).unwrap();
    if let Some(p) = env_val.get_mut("payload")
        && let Some(desc) = p.get_mut("description")
    {
        *desc = json!("Tampered description");
    }
    let tampered_env_str = serde_json::to_string(&env_val).unwrap();

    let tampered_card = card_body(RECORD_REQUEST, REQUEST_VERSION, &tampered_env_str).unwrap();

    let sent = node_x
        .rpc_ok(
            "conversation.send",
            json!({
                "conversation": x_conv_id,
                "body": tampered_card,
                "content_type": CARD_CONTENT_TYPE,
            }),
        )
        .await;
    let sent_msg_id = sent["message_id"].as_str().unwrap().to_string();

    let delivered = wait_delivered(&node_x, &sent_msg_id).await;
    assert!(delivered, "tampered card was delivered to Y");

    let y_has_conv = wait_until(Duration::from_secs(20), || async {
        let list = node_y.rpc_ok("conversation.list", json!({})).await;
        list["conversations"].as_array().map(|c| !c.is_empty()).unwrap_or(false)
    })
    .await;
    assert!(y_has_conv, "Y received conversation");
    let y_convs = node_y.rpc_ok("conversation.list", json!({})).await;
    let y_conv_id = y_convs["conversations"][0]["id"].as_str().unwrap().to_string();

    node_y.rpc_ok("transaction.sync", json!({ "conversation": y_conv_id })).await;

    let y_thread = node_y.rpc_ok("transaction.thread", json!({ "conversation": y_conv_id })).await;
    let cards = y_thread["cards"].as_array().unwrap();
    let tampered_card_row =
        cards.iter().find(|c| c["message_id"] == sent_msg_id).expect("tampered card in thread");
    assert_eq!(tampered_card_row["verified"], false);
    assert!(
        tampered_card_row["reason"].is_string()
            && !tampered_card_row["reason"].as_str().unwrap().is_empty()
    );
    assert!(tampered_card_row["data"].is_null());

    let y_req_list = node_y.rpc_ok("request.list", json!({ "conversation": y_conv_id })).await;
    let y_requests = y_req_list["requests"].as_array().unwrap();
    assert!(y_requests.iter().all(|r| r["envelope"] != tampered_env_str));

    node_x.teardown().await;
    node_y.teardown().await;
}
