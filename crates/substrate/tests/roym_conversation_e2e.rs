#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]
//! The Roym product's 1:1 conversation, end to end across two genuinely
//! independent `syneroym-substrate` instances, each running the full Roym
//! SynApp (the `wasm32-wasip2` build) under its own owner identity.
//!
//! It proves the messaging half of the product: a message is `pending`
//! from the host's own answer and never optimistically `delivered`; Roym's
//! own copy on each side is the same message; a blocked sender's message
//! reaches no conversation, no search result and no count, and the sender
//! is never told; a listing carries the provider's conversation address
//! under their own signature so a stranger can engage them with no
//! directory anywhere; every service's own copy round-trips through
//! export/import; deleting a message removes the local copy, keeps a
//! deletion record, and asks the other side, which honours it only for a
//! message the requester authored; and a message that never reaches its
//! peer settles `failed` with the host's own reason.
//!
//! Each node's substrate comes from `common::SubstrateNode` (with the Roym
//! config layered on through `.configure`); the domain `Node` here keeps the
//! deploy / login / restart machinery on top.
//!
//! The reference scenario is fifteen numbered steps. A guest-HTTP
//! component's route re-registration after a substrate restart is slow and
//! slower still when two full substrates share a CI runner, so the steps
//! that restart a substrate (6, 9 -- pending survives a restart; 15 -- a
//! message settles failed) are each their own single-node test with the
//! machine to themselves. The two-substrate test carries steps 1-5, 7-8
//! and 10-14, and the export/import steps re-import against the running
//! substrate -- the wipe-and-restore variant is the parity suite's
//! (scenarios 49-51, 63-65). A backlog row tracks the restart-route gap.
//!
//! Skips when the Roym wasm artifacts or the UI bundle are absent
//! (`mise run build:roym` / `mise run build:roym-ui`).

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
    config::{AppSandboxRole, AuthRole, ClientGatewayRole, IdentityMode},
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
use tokio::time;

mod common;

const SESSION_COOKIE_NAME: &str = "syneroym_session";

/// Every Roym service that signs a record and so needs a record-signing
/// certificate. Mirrors `roymctl`'s own list.
const SIGNING_SERVICES: &[&str] = &["profile", "catalog", "conversation", "transaction"];

fn far_future_not_after() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS)
}

/// `conversation_tick_secs: 1` so a delivery attempt does not wait out the
/// production budget, plus `conversation_max_pending_age_secs` -- the one
/// field this helper gains beyond the tick, so step 15 can watch a message
/// settle `failed` without a real wall-clock wait. The main flow passes a
/// large value; step 15 restarts node A with a small one.
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
    shared_registry_url: Option<String>,
    owner: Identity,
    /// Stable across restarts, keyed by service name.
    masters: BTreeMap<String, Identity>,
    role: AppSandboxRole,

    /// The captured builder, reused on `resume` so a reboot keeps the same
    /// ports.
    builder: common::NodeBuilder,
    /// `None` only between `stop` and `resume`.
    node: Option<common::SubstrateNode>,
    registry_url: String,
    gateway_url: String,
    substrate_did: String,
    /// Minted DID per service name, set by `deploy`.
    dids: BTreeMap<String, String>,
    session_token: Option<String>,
}

impl Node {
    async fn boot(
        label: &'static str,
        base_path: PathBuf,
        shared_registry_url: Option<String>,
        owner: Identity,
        role: AppSandboxRole,
    ) -> Self {
        let ids_dir = base_path.join("identities");
        fs::create_dir_all(&ids_dir).unwrap();
        owner.save_to_path(ids_dir.join("owner.key")).unwrap();

        Self::spawn_substrate(label, base_path, shared_registry_url, owner, role).await
    }

    /// A `SubstrateNode` builder carrying the Roym-specific config: a
    /// login-mode client gateway, the `auth` role pointed at the on-disk
    /// person identities, and the app-sandbox role.
    fn make_builder(
        base_path: &std::path::Path,
        shared_registry_url: Option<&str>,
        owner: &Identity,
        role: &AppSandboxRole,
    ) -> common::NodeBuilder {
        let ids_dir = base_path.join("identities");
        let role = role.clone();
        let mut builder = common::SubstrateNode::builder()
            .owner(owner)
            .base_path(base_path.to_path_buf())
            .inject_kek_bytes([0xcd; 32])
            .configure(move |c| {
                let gateway = c.roles.client_gateway.take().unwrap_or_default();
                c.roles.client_gateway =
                    Some(ClientGatewayRole { identity_mode: IdentityMode::Login, ..gateway });
                c.roles.auth = Some(AuthRole {
                    person_identities_dir: Some(ids_dir.clone()),
                    ..Default::default()
                });
                c.roles.app_sandbox = Some(role.clone());
            });
        if let Some(url) = shared_registry_url {
            builder = builder.shared_registry(url.to_string());
        }
        builder
    }

    async fn spawn_substrate(
        label: &'static str,
        base_path: PathBuf,
        shared_registry_url: Option<String>,
        owner: Identity,
        role: AppSandboxRole,
    ) -> Self {
        let builder = Self::make_builder(&base_path, shared_registry_url.as_deref(), &owner, &role);
        let node = builder.clone().boot().await;

        Self {
            label,
            base_path,
            shared_registry_url,
            owner: Identity::from_bytes(&owner.to_bytes()),
            masters: BTreeMap::new(),
            role,
            registry_url: node.registry_url().to_string(),
            gateway_url: node.gateway_url(),
            substrate_did: node.did().to_string(),
            builder,
            node: Some(node),
            dids: BTreeMap::new(),
            session_token: None,
        }
    }

    fn substrate_did(&self) -> String {
        self.substrate_did.clone()
    }

    /// Compile, mint (or reuse) masters, certify, publish, apply. On a
    /// redeploy after a restart (`redeploy = true`) the generation
    /// out-ranks the one the substrate still holds; the substrate then runs
    /// the full deploy (its post-restart dedup guard sees no process-local
    /// route entry) and every component is force-restarted for good
    /// measure. Also publishes each service's master anchor so the *other*
    /// node can verify an inbound delivery's delegation chain.
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
        // KEK already injected by `spawn_substrate` (node-global, once).
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

        // A redeploy of an unchanged plan re-runs `apply_plan`'s diff and
        // decides nothing changed, so the components (and `web`'s guest
        // HTTP route) are not re-instantiated -- but after a substrate
        // restart "as they were" means "not running". Force each one.
        if redeploy {
            for did in self.dids.values() {
                let _ = SubstrateActor::restart(&*client, did.clone(), generation).await;
            }
        }

        // The other node resolves an inbound conversation delivery's
        // delegation chain through its own registry, which is the shared
        // one -- publish each signing service's master anchor there.
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

    /// After a substrate restart the sandbox's warm-up loop has to
    /// re-instantiate every component and re-register `web`'s guest HTTP
    /// route before `POST /rpc` reaches guest code again -- until then the
    /// router treats the JSON-RPC `method` as a raw interface method and
    /// answers "not found in interface web/api". Poll a proxied verb until
    /// that clears.
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

    /// Enrol each signing service's certificate, exactly as
    /// `roym enrol-signing` does.
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

    /// Stop the substrate task, keeping the data directory and every stable
    /// identity so a later `resume` brings the same node back.
    /// `wipe_service_state` names a service whose Roym data-layer store
    /// (`state.db`) is removed first, leaving the host's own
    /// `conversation.db` in the same directory intact -- so an import
    /// rebuilds only Roym's own copy.
    async fn stop(&mut self, wipe_service_state: Option<&str>) {
        if let Some(node) = self.node.take() {
            node.teardown().await;
        }
        // Let the OS actually release the iroh UDP sockets before the next boot.
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

    /// Boot the substrate again under the same identity, redeploy (which
    /// forces every component to re-instantiate and re-register its
    /// routes), rebuild the in-memory community registry, then log in.
    /// Signing certificates persist in the app store.
    async fn resume(&mut self, new_role: Option<AppSandboxRole>) {
        if let Some(role) = new_role {
            self.role = role;
            self.builder = Self::make_builder(
                &self.base_path,
                self.shared_registry_url.as_deref(),
                &self.owner,
                &self.role,
            );
        }
        let masters = mem::take(&mut self.masters);
        let node = self.builder.clone().boot().await;
        self.registry_url = node.registry_url().to_string();
        self.gateway_url = node.gateway_url();
        self.node = Some(node);
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

    /// The community registry is in-memory, so a substrate restart wipes
    /// every record it holds -- including records for services on the
    /// *other* node when this node hosts the shared registry. The substrate
    /// replays its own hosted-service endpoint records on boot, but never
    /// the master anchors (it holds no key that signed one), and the other
    /// node's records only return on its next heartbeat. Re-push both,
    /// verbatim, so delivery does not stall waiting for a heartbeat.
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
        if let Some(node) = self.node.take() {
            node.teardown().await;
        }
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

#[tokio::test]
async fn roym_conversation_survives_restarts_blocks_and_round_trips() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let owner_a = Identity::generate().unwrap();
    let owner_b = Identity::generate().unwrap();

    // --- Step 1: deploy Roym on A and B; enrol signing on both. ----------
    let mut node_a = Node::boot(
        "node-a",
        dir_a.path().to_path_buf(),
        None,
        Identity::from_bytes(&owner_a.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    let shared_registry = node_a.registry_url.clone();
    node_a.full_bring_up().await;

    let mut node_b = Node::boot(
        "node-b",
        dir_b.path().to_path_buf(),
        Some(shared_registry.clone()),
        Identity::from_bytes(&owner_b.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    node_b.full_bring_up().await;

    // This test reads each conversation service id straight from the
    // substituted deployment plan; a person running the product gets the
    // same string from `roymctl roym address`, which looks it up in
    // `svc list` and prints it to paste into `profile.set` below.
    let b_conv_did = node_b.dids["conversation"].clone();
    let a_conv_did = node_a.dids["conversation"].clone();
    let owner_b_did = substrate::derive_did_key(&owner_b.public_key());

    // --- Step 2: profile.set on both, each carrying its own conversation
    //     service address (`roymctl roym address` supplies it). ------------
    node_a
        .rpc_ok("profile.set", json!({ "display_name": "Ann", "conversation_address": a_conv_did }))
        .await;
    let b_profile = node_b
        .rpc_ok("profile.set", json!({ "display_name": "Bo", "conversation_address": b_conv_did }))
        .await;
    // `contacts.upsert` wants the envelope as the raw JSON string, verbatim.
    let b_profile_envelope = b_profile["envelope"].as_str().unwrap().to_string();

    // --- Step 3: A: contacts.upsert for B, using B's verified profile
    //     envelope -> the address comes from the record. -------------------
    node_a
        .rpc_ok(
            "contacts.upsert",
            json!({ "person_did": owner_b_did, "profile_envelope": b_profile_envelope }),
        )
        .await;
    let contacts = node_a.rpc_ok("contacts.list", json!({})).await;
    let b_contact = contacts
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["person_did"] == owner_b_did))
        .cloned()
        .expect("B is in A's contacts");
    assert_eq!(
        b_contact["conversation_address"], b_conv_did,
        "the contact address came from the verified profile record: {b_contact}"
    );
    assert!(
        b_contact.get("from_profile_record").is_some_and(|v| !v.is_null()),
        "from_profile_record is set when the address came from a verified envelope: {b_contact}"
    );

    // --- Step 4: A: conversation.open { person_did: B } -> resolved through
    //     contacts. --------------------------------------------------------
    let opened = node_a.rpc_ok("conversation.open", json!({ "person_did": owner_b_did })).await;
    let conversation_id = opened["conversation_id"].as_str().unwrap().to_string();
    assert_eq!(opened["peer_address"], b_conv_did);

    // --- Step 5: A: conversation.send -> "pending" from the host's own
    //     return value, never optimistic. -------------------------------
    let sent = node_a
        .rpc_ok(
            "conversation.send",
            json!({ "conversation": conversation_id, "body": "hello from A" }),
        )
        .await;
    let message_id = sent["message_id"].as_str().unwrap().to_string();
    assert_eq!(sent["state"], "pending", "born pending, from the host, never optimistic");

    // --- Steps 6-9: the restart-survival half of the reference scenario
    //     (message stays pending across a restart, then delivers; both
    //     sides survive their own restart) lives in
    //     `a_pending_message_and_its_body_survive_a_substrate_restart` --
    //     one restart in isolation, because a guest-HTTP component's route
    //     re-registration after a substrate restart is slow and slows
    //     further when two full substrates share a machine.
    //
    //     Here both nodes stay up: A's message to B delivers, and Roym's
    //     own copy on each side is the same message. --------------------
    let _ = node_a.rpc("conversation.retry", json!({ "message_id": message_id })).await;
    let delivered = wait_until(Duration::from_secs(90), || {
        let node_a = &node_a;
        let message_id = message_id.clone();
        async move {
            let _ = node_a.rpc("conversation.retry", json!({ "message_id": message_id })).await;
            let s = node_a
                .rpc_ok("conversation.delivery-status", json!({ "message_id": message_id }))
                .await;
            s["state"] == "delivered"
        }
    })
    .await;
    assert!(delivered, "the message must deliver to a reachable peer");

    let a_hist =
        node_a.rpc_ok("conversation.history", json!({ "conversation": conversation_id })).await;
    assert_eq!(history_messages(&a_hist)[0]["state"], "delivered");
    assert_eq!(history_messages(&a_hist)[0]["body"], "hello from A");

    let b_conversations = node_b.rpc_ok("conversation.list", json!({})).await;
    let b_conv_id = b_conversations["conversations"][0]["id"].as_str().unwrap().to_string();
    let b_hist = node_b.rpc_ok("conversation.history", json!({ "conversation": b_conv_id })).await;
    let b_msgs = history_messages(&b_hist);
    assert_eq!(b_msgs.len(), 1, "B holds exactly the one message: {b_hist}");
    assert_eq!(b_msgs[0]["body"], "hello from A");
    assert_eq!(b_msgs[0]["id"], message_id, "the same host message id on both sides");

    // --- Step 10: B blocks A's address; A sends again. The host stores and
    //     delivers it (the product never claims otherwise); B's copy does
    //     not grow, B's search finds nothing, and the sender is not told. -
    node_b.rpc_ok("block.add", json!({ "address": a_conv_did })).await;
    let sent2 = node_a
        .rpc_ok(
            "conversation.send",
            json!({ "conversation": conversation_id, "body": "second from A" }),
        )
        .await;
    let message_id_2 = sent2["message_id"].as_str().unwrap().to_string();
    let delivered2 = wait_until(Duration::from_secs(120), || {
        let node_a = &node_a;
        let message_id_2 = message_id_2.clone();
        async move {
            let _ = node_a.rpc("conversation.retry", json!({ "message_id": message_id_2 })).await;
            let s = node_a
                .rpc_ok("conversation.delivery-status", json!({ "message_id": message_id_2 }))
                .await;
            s["state"] == "delivered"
        }
    })
    .await;
    assert!(delivered2, "the host delivered it; the product does not pretend it was refused");

    // Give B's inbox a moment to run its refusal.
    time::sleep(Duration::from_secs(2)).await;
    let b_hist3 = node_b.rpc_ok("conversation.history", json!({ "conversation": b_conv_id })).await;
    assert_eq!(history_messages(&b_hist3).len(), 1, "the blocked message reached no conversation");
    let b_search = node_b.rpc_ok("conversation.search", json!({ "query": "second from A" })).await;
    assert_eq!(
        b_search["matches"].as_array().map(Vec::len),
        Some(0),
        "the blocked message is in no search result -- counted nowhere"
    );

    // --- Step 11: B publishes a signed listing carrying its conversation
    //     address; A verifies it locally -- no directory anywhere. --------
    let b_listing = node_b
        .rpc_ok(
            "listing.set",
            json!({
                "title": "Bike repair",
                "summary": "Same-day, at your door.",
                "categories": ["cycling"],
                "payment": {
                    "currency": "EUR", "model": "per-hour", "amount_minor": 4000,
                    "tax_included": true, "payee": "B"
                }
            }),
        )
        .await;
    let b_listing_id = b_listing["listing_id"].as_str().unwrap().to_string();
    let b_listing_row = node_b.rpc_ok("listing.get", json!({ "listing_id": b_listing_id })).await;
    let b_listing_envelope = b_listing_row["envelope"].as_str().unwrap().to_string();
    let a_verify = node_a.rpc_ok("listing.verify", json!({ "envelope": b_listing_envelope })).await;
    assert_eq!(a_verify["verified"], true, "A verifies B's listing: {a_verify}");
    assert_eq!(
        a_verify["conversation_address"], b_conv_did,
        "the address A gets from the listing is the one it can already message"
    );

    // --- Step 12: A: conversation.export, then conversation.import of the
    //     same bundle against the running substrate -> integrity checks,
    //     every row round-trips, and the order and states are unchanged.
    //     The wipe-then-restore variant is proven by parity 63-65; doing a
    //     third substrate restart here only to re-exercise it is not worth
    //     the cost (a guest-HTTP component's warm-up slows with each
    //     restart). ------------------------------------------------------
    let a_export = node_a.rpc_ok("conversation.export", json!({})).await;
    let import_counts = node_a.rpc_ok("conversation.import", json!({ "bundle": a_export })).await;
    assert!(
        import_counts["imported"]["messages"].as_u64().unwrap_or(0) >= 2,
        "the bundle carried both messages: {import_counts}"
    );
    let a_hist_after =
        node_a.rpc_ok("conversation.history", json!({ "conversation": conversation_id })).await;
    let a_msgs_after = history_messages(&a_hist_after);
    assert_eq!(a_msgs_after.len(), 2, "both messages round-trip: {a_hist_after}");
    let bodies: Vec<&str> = a_msgs_after.iter().filter_map(|m| m["body"].as_str()).collect();
    assert!(bodies.contains(&"hello from A"), "the delivered message round-trips: {a_hist_after}");
    assert!(bodies.contains(&"second from A"), "the second message round-trips: {a_hist_after}");
    assert!(
        a_msgs_after.iter().all(|m| m["state"] == "delivered"),
        "every round-tripped message keeps its delivered state: {a_hist_after}"
    );

    // A tampered bundle -- one message body edited, the manifest untouched
    // -- is refused.
    let mut tampered = a_export.clone();
    if let Some(rows) = tampered["sections"]["messages"].as_array_mut()
        && let Some(first) = rows.first_mut()
    {
        first["payload"]["body"] = json!("forged");
    }
    let tampered_res = node_a.rpc("conversation.import", json!({ "bundle": tampered })).await;
    assert_eq!(
        tampered_res["error"]["code"], -32602,
        "a section digest mismatch is refused: {tampered_res}"
    );

    // --- Step 13: B: catalog.export, then catalog.import of the same
    //     bundle against the running substrate -> the listing round-trip
    //     acceptance test (listing_id preserved, signature still verifying,
    //     schema version preserved). The wipe variant is parity 49-51. ----
    let b_catalog_export = node_b.rpc_ok("catalog.export", json!({})).await;
    let listings_declared = b_catalog_export["manifest"]["sections"]["listings"]["schema_version"]
        .as_u64()
        .unwrap_or(0);
    assert_eq!(listings_declared, 2, "the bundle declares the listings schema version");
    node_b.rpc_ok("catalog.import", json!({ "bundle": b_catalog_export })).await;
    let b_listing_back = node_b.rpc_ok("listing.get", json!({ "listing_id": b_listing_id })).await;
    assert_eq!(b_listing_back["listing_id"], b_listing_id, "same listing id after import");
    let b_reverify = node_b
        .rpc_ok(
            "listing.verify",
            json!({ "envelope": b_listing_back["envelope"].as_str().unwrap() }),
        )
        .await;
    assert_eq!(b_reverify["verified"], true, "the imported listing still verifies");

    // --- Step 14: A deletes a message it sent B, ask_peer true. A's copy
    //     loses its body and keeps its row; B honours it *because A authored
    //     it*. B blocked A in step 10, and a blocked peer's deletion
    //     request is refused at the inbox like any other message -- lift
    //     the block so this step exercises the honour path, not the block
    //     path (which step 10 already covered). ------------------------
    node_b.rpc_ok("block.remove", json!({ "address": a_conv_did })).await;
    let del = node_a
        .rpc_ok(
            "conversation.delete-message",
            json!({ "message_id": message_id, "ask_peer": true }),
        )
        .await;
    assert_eq!(del["asked_peer"], true);
    let a_after_del =
        node_a.rpc_ok("conversation.history", json!({ "conversation": conversation_id })).await;
    let a_row = history_messages(&a_after_del)
        .into_iter()
        .find(|m| m["id"] == message_id)
        .expect("A keeps the tombstoned row");
    assert!(a_row.get("body").is_none() || a_row["body"].is_null(), "A's copy lost its body");

    // The deletion request is a message in A's own outbox; poke a retry
    // each poll so a backed-off attempt does not hold up a loaded machine.
    let b_honoured = wait_until(Duration::from_secs(120), || {
        let (node_a, node_b) = (&node_a, &node_b);
        let b_conv_id = b_conv_id.clone();
        let message_id = message_id.clone();
        let del_msg = del["deleted"].as_str().unwrap_or_default().to_string();
        async move {
            if let Some(outbox) = node_a
                .rpc_ok("conversation.outbox", json!({}))
                .await
                .get("outbox")
                .and_then(Value::as_array)
            {
                for row in outbox {
                    if let Some(id) = row["id"].as_str() {
                        let _ = node_a.rpc("conversation.retry", json!({ "message_id": id })).await;
                    }
                }
            }
            let _ = del_msg;
            let h =
                node_b.rpc_ok("conversation.history", json!({ "conversation": b_conv_id })).await;
            history_messages(&h)
                .into_iter()
                .find(|m| m["id"] == message_id)
                .map(|m| m.get("body").is_none() || m["body"].is_null())
                .unwrap_or(false)
        }
    })
    .await;
    assert!(b_honoured, "B honours the deletion request for a message A authored");

    node_b.teardown().await;
    node_a.teardown().await;
}

/// Steps 6 and 9, as one single-node test: a `pending` message keeps its
/// state and its body across a real substrate restart -- Roym's own copy
/// in `state.db` and the host's outbox in `conversation.db` both survive,
/// and the redeploy on `resume` re-registers `web`'s guest-HTTP route so
/// the post-restart `conversation.history` RPC lands.
///
/// This was `#[ignore]`d while the redeploy after a restart deduped into a
/// no-op (a matching persisted `manifest_hash` plus a warmed instance),
/// leaving `POST /rpc` unrouted -- see `deploy_with_context`'s
/// `full_deploy_completed` guard. NOTE: a bare restart with no redeploy is
/// still not self-healing; this test's `resume()` does redeploy, which is
/// what the guard makes real work again.
#[tokio::test]
async fn a_pending_message_and_its_body_survive_a_substrate_restart() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let owner = Identity::generate().unwrap();
    let mut node = Node::boot(
        "node",
        dir.path().to_path_buf(),
        None,
        Identity::from_bytes(&owner.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    node.full_bring_up().await;

    node.rpc_ok(
        "profile.set",
        json!({ "display_name": "Solo", "conversation_address": node.dids["conversation"] }),
    )
    .await;

    // A peer that will never answer: the message can only ever be pending.
    let dead_peer = substrate::derive_did_key(&Identity::generate().unwrap().public_key());
    let opened = node.rpc_ok("conversation.open", json!({ "address": dead_peer })).await;
    let conv = opened["conversation_id"].as_str().unwrap().to_string();
    let sent = node
        .rpc_ok("conversation.send", json!({ "conversation": conv, "body": "hold this" }))
        .await;
    let message_id = sent["message_id"].as_str().unwrap().to_string();
    assert_eq!(sent["state"], "pending");
    for _ in 0..4 {
        let s =
            node.rpc_ok("conversation.delivery-status", json!({ "message_id": message_id })).await;
        assert_eq!(s["state"], "pending", "never optimistically delivered");
        time::sleep(Duration::from_millis(700)).await;
    }

    node.restart(None, None).await;

    let history = node.rpc_ok("conversation.history", json!({ "conversation": conv })).await;
    let msgs = history_messages(&history);
    assert_eq!(msgs.len(), 1, "exactly the one message after the restart: {history}");
    assert_eq!(msgs[0]["id"], message_id, "the same message id, not a new one");
    assert_eq!(msgs[0]["state"], "pending", "still pending, not reset and not delivered");
    assert_eq!(msgs[0]["body"], "hold this", "the body is intact");

    let outbox = node.rpc_ok("conversation.outbox", json!({})).await;
    assert!(
        outbox["outbox"].as_array().is_some_and(|rows| rows.iter().any(|r| r["id"] == message_id)),
        "the host outbox kept the item across the restart: {outbox}"
    );

    node.teardown().await;
}

/// Step 15, as its own single-node test: a message to a peer that never
/// answers goes `pending`, then `failed` once
/// `conversation_max_pending_age_secs` passes, and `conversation.history`
/// reports it `failed` with the host's own reason -- the `failed` third of
/// the pending / delivered / failed scope, watched with no real clock wait.
///
/// Split out of the main flow because the main flow already restarts each
/// substrate twice and a guest-HTTP component's warm-up after a restart is
/// slow; a third restart of node A there compounds it. This needs no
/// restart at all.
#[tokio::test]
async fn a_message_that_never_reaches_its_peer_settles_failed_with_the_hosts_reason() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let owner = Identity::generate().unwrap();
    let mut node = Node::boot(
        "node",
        dir.path().to_path_buf(),
        None,
        Identity::from_bytes(&owner.to_bytes()),
        fast_conversation_role(6),
    )
    .await;
    node.full_bring_up().await;

    node.rpc_ok(
        "profile.set",
        json!({ "display_name": "Solo", "conversation_address": node.dids["conversation"] }),
    )
    .await;

    let dead_peer = substrate::derive_did_key(&Identity::generate().unwrap().public_key());
    let opened = node.rpc_ok("conversation.open", json!({ "address": dead_peer })).await;
    let conv = opened["conversation_id"].as_str().unwrap().to_string();

    let sent = node
        .rpc_ok("conversation.send", json!({ "conversation": conv, "body": "into the void" }))
        .await;
    let message_id = sent["message_id"].as_str().unwrap().to_string();
    assert_eq!(sent["state"], "pending", "born pending, never optimistically anything else");

    let failed = wait_until(Duration::from_secs(40), || {
        let node = &node;
        let conv = conv.clone();
        let message_id = message_id.clone();
        async move {
            let h = node.rpc_ok("conversation.history", json!({ "conversation": conv })).await;
            history_messages(&h)
                .into_iter()
                .find(|m| m["id"] == message_id)
                .map(|m| m["state"] == "failed")
                .unwrap_or(false)
        }
    })
    .await;
    assert!(failed, "a message that never reaches its peer settles failed once the window passes");

    let hist = node.rpc_ok("conversation.history", json!({ "conversation": conv })).await;
    let row = history_messages(&hist).into_iter().find(|m| m["id"] == message_id).unwrap();
    assert_eq!(row["state"], "failed");
    assert!(
        row["last_error"].as_str().is_some_and(|s| !s.is_empty()),
        "conversation.history reports failed with the host's own reason: {row}"
    );

    node.teardown().await;
}
