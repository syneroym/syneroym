//! Shared Roym test harness for substrate integration tests.

use core::future::Future;
use std::{
    collections::BTreeMap,
    fs, mem,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use reqwest::Client;
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

pub const SIGNING_SERVICES: &[&str] = &["profile", "catalog", "conversation", "transaction"];
pub const SESSION_COOKIE_NAME: &str = "syneroym_session";

pub fn far_future_not_after() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS)
}

#[derive(Default, Clone, Debug)]
pub struct CertOverrides {
    pub skip_instance_cert: Vec<String>,
}

pub fn fast_conversation_role(max_pending_age_secs: u64) -> AppSandboxRole {
    AppSandboxRole {
        conversation_tick_secs: 1,
        conversation_max_pending_age_secs: max_pending_age_secs,
        ..AppSandboxRole::default()
    }
}

pub fn roym_artifacts_present() -> bool {
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

pub fn service_visibility(name: &str) -> Visibility {
    match name {
        "web" => Visibility::Internal,
        "profile" => Visibility::Private,
        _ => Visibility::Public,
    }
}

pub fn mint_masters(plan: &DeploymentPlan) -> BTreeMap<String, Identity> {
    plan.services
        .iter()
        .map(|s| (s.logical_ref.service_name.as_str().to_string(), Identity::generate().unwrap()))
        .collect()
}

pub fn substitute_plan(
    plan: &DeploymentPlan,
    masters: &BTreeMap<String, Identity>,
) -> DeploymentPlan {
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

pub fn masters_by_id(
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

pub async fn certify_and_publish(
    plan: &DeploymentPlan,
    masters: &BTreeMap<ServiceId, Identity>,
    client: &Arc<SyneroymClient>,
    overrides: &CertOverrides,
) -> (BTreeMap<ServiceId, String>, BTreeMap<ServiceId, String>) {
    let mut certs = BTreeMap::new();
    let mut records = BTreeMap::new();
    for svc in &plan.services {
        let master = &masters[&svc.service_id];
        let name = svc.logical_ref.service_name.as_str();
        if !overrides.skip_instance_cert.iter().any(|s| s == name) {
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

pub struct RoymNode {
    pub label: &'static str,
    pub base_path: PathBuf,
    pub shared_registry_url: Option<String>,
    pub owner: Identity,
    pub masters: BTreeMap<String, Identity>,
    pub role: AppSandboxRole,
    pub cert_overrides: CertOverrides,

    pub builder: super::NodeBuilder,
    pub node: Option<super::SubstrateNode>,
    pub registry_url: String,
    pub gateway_url: String,
    pub substrate_did: String,
    pub dids: BTreeMap<String, String>,
    pub session_token: Option<String>,
}

impl RoymNode {
    pub async fn boot(
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

    pub async fn boot_default(
        label: &'static str,
        base_path: PathBuf,
        shared_registry_url: Option<String>,
        owner: Identity,
    ) -> Self {
        Self::boot(label, base_path, shared_registry_url, owner, AppSandboxRole::default()).await
    }

    pub fn with_cert_overrides(mut self, overrides: CertOverrides) -> Self {
        self.cert_overrides = overrides;
        self
    }

    pub fn make_builder(
        base_path: &Path,
        shared_registry_url: Option<&str>,
        owner: &Identity,
        role: &AppSandboxRole,
    ) -> super::NodeBuilder {
        let ids_dir = base_path.join("identities");
        let role = role.clone();
        let mut builder = super::SubstrateNode::builder()
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

    pub async fn spawn_substrate(
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
            cert_overrides: CertOverrides::default(),
            registry_url: node.registry_url().to_string(),
            gateway_url: node.gateway_url(),
            substrate_did: node.did().to_string(),
            builder,
            node: Some(node),
            dids: BTreeMap::new(),
            session_token: None,
        }
    }

    pub fn substrate_did(&self) -> String {
        self.substrate_did.clone()
    }

    pub async fn deploy(&mut self, redeploy: bool) {
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
            certify_and_publish(&new_plan, &masters, &client, &self.cert_overrides).await;

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
        for master in self.masters.values() {
            let did = substrate::derive_did_key(&master.public_key());
            registry
                .publish_master_anchor(&did, vec![], None, master, true)
                .await
                .expect("failed to publish a service master anchor");
        }
    }

    pub fn web_host_header(&self) -> String {
        format!("s{}.localhost", short_hash(&self.dids["web"]))
    }

    pub async fn login(&mut self) {
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

    pub async fn rpc(&self, method: &str, params: Value) -> Value {
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

    pub async fn rpc_ok(&self, method: &str, params: Value) -> Value {
        let v = self.rpc(method, params).await;
        assert!(v.get("error").is_none(), "{} {method} errored: {v}", self.label);
        v["result"].clone()
    }

    pub async fn rpc_err(&self, method: &str, params: Value) -> Value {
        let v = self.rpc(method, params).await;
        assert!(v.get("error").is_some(), "{} {method} unexpectedly succeeded: {v}", self.label);
        v["error"].clone()
    }

    pub async fn wait_for_proxy(&self) {
        self.wait_for_service_proxy("conversation").await;
    }

    pub async fn wait_for_service_proxy(&self, service: &str) {
        let deadline = Instant::now() + Duration::from_secs(300);
        let ping_method = format!("{service}.ping");
        while Instant::now() < deadline {
            let v = self.rpc(&ping_method, json!({})).await;
            if v.get("result").and_then(|r| r.get("service")).is_some() {
                return;
            }
            time::sleep(Duration::from_millis(1500)).await;
        }
        panic!("{} web never proxied {service}.ping after deploy", self.label);
    }

    pub async fn enrol_signing(&self) {
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

    pub async fn full_bring_up(&mut self) {
        self.deploy(false).await;
        self.login().await;
        self.wait_for_proxy().await;
        self.enrol_signing().await;
    }

    pub async fn stop(&mut self, wipe_service_state: Option<&str>) {
        if let Some(node) = self.node.take() {
            node.teardown().await;
        }
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

    pub async fn resume(&mut self, new_role: Option<AppSandboxRole>) {
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

    pub async fn restart(
        &mut self,
        new_role: Option<AppSandboxRole>,
        wipe_service_state: Option<&str>,
    ) {
        self.stop(wipe_service_state).await;
        self.resume(new_role).await;
    }

    pub async fn republish_registry(&self) {
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
            let master_did = substrate::derive_did_key(&master.public_key());
            let _ = registry.publish_master_anchor(&master_did, vec![], None, master, true).await;
        }
    }

    pub async fn teardown(mut self) {
        if let Some(node) = self.node.take() {
            node.teardown().await;
        }
    }
}

pub fn history_messages(result: &Value) -> Vec<Value> {
    result["messages"].as_array().cloned().unwrap_or_default()
}

pub async fn wait_until<F, Fut>(budget: Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if check().await {
            return true;
        }
        time::sleep(Duration::from_millis(500)).await;
    }
    false
}

pub async fn wait_delivered(node: &RoymNode, message_id: &str) -> bool {
    let _ = node.rpc("conversation.retry", json!({ "message_id": message_id })).await;
    wait_until(Duration::from_secs(90), || async {
        let _ = node.rpc("conversation.retry", json!({ "message_id": message_id })).await;
        let s =
            node.rpc_ok("conversation.delivery-status", json!({ "message_id": message_id })).await;
        s["state"] == "delivered"
    })
    .await
}
