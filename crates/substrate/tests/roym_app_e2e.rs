#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]
//! Substrate-level deployment integration test for the Roym SynApp.
//!
//! Deploys the six Roym services on a live substrate instance with a gateway
//! and registry, and tests:
//! 1. Static UI serving (`GET /`) from the asset bundle with CSP meta header.
//! 2. `POST /rpc` `session.whoami` returns caller DID and `auth: "delegated"`
//!    under a session cookie.
//! 3. `POST /rpc` `profile.ping` proxy call reaches the sibling service.
//! 4. Open topology on `directory` vs restricted/private on `profile`.
//! 5. `Authorization: Bearer <token>` works without browser cookie.
//! 6. Unauthenticated request reports `self-asserted:<node-did>`.

use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use reqwest::Client;
use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_app_orchestration::{
    AppInstanceId, DeploymentJournal, DeploymentPlan, DeploymentState, LocalFilesystemCatalog,
    compile,
    models::{ServiceId, SubstrateAlias, SynAppManifest},
};
use syneroym_core::{
    dht_registry::{DEFAULT_ENDPOINT_NOT_AFTER_SECS, RegistryClient},
    util::short_hash,
};
use syneroym_identity::{Identity, substrate};
use syneroym_sdk::{
    SyneroymClient,
    deploy::{
        self, ApplyRequest, DeployTarget, apply_plan, certify_instance, member_registry_record,
    },
};

mod common;
use common::{SubstrateTestContext, alloc_ports};

const SESSION_COOKIE_NAME: &str = "syneroym_session";

fn far_future_not_after() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS)
}

fn mint_and_substitute_masters(
    plan: &DeploymentPlan,
) -> (DeploymentPlan, BTreeMap<ServiceId, Identity>) {
    let mut substitution: BTreeMap<ServiceId, ServiceId> = BTreeMap::new();
    let mut masters: BTreeMap<ServiceId, Identity> = BTreeMap::new();
    for svc in &plan.services {
        let master = Identity::generate().unwrap();
        let master_did = ServiceId::new(substrate::derive_did_key(&master.public_key()));
        substitution.insert(svc.service_id.clone(), master_did.clone());
        masters.insert(master_did, master);
    }

    let mut new_plan = plan.clone();
    for svc in &mut new_plan.services {
        let old_id = svc.service_id.clone();
        svc.service_id = substitution[&old_id].clone();
        svc.resolved_dependencies = svc
            .resolved_dependencies
            .iter()
            .map(|(name, members)| {
                (name.clone(), members.iter().map(|m| substitution[m].clone()).collect())
            })
            .collect();
    }
    (new_plan, masters)
}

async fn certify_and_publish(
    plan: &DeploymentPlan,
    masters: &BTreeMap<ServiceId, Identity>,
    clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
) -> (BTreeMap<ServiceId, String>, BTreeMap<ServiceId, String>) {
    let mut certs = BTreeMap::new();
    let mut records = BTreeMap::new();
    for svc in &plan.services {
        let master = &masters[&svc.service_id];
        let client = svc
            .substrate
            .as_ref()
            .and_then(|a| clients.get(a))
            .or_else(|| clients.values().next())
            .expect("client exists");

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

fn deploy_targets(
    clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
) -> BTreeMap<SubstrateAlias, DeployTarget> {
    clients
        .iter()
        .map(|(alias, c)| {
            (
                alias.clone(),
                DeployTarget {
                    alias: Some(alias.clone()),
                    substrate_did: c.service_id().to_string(),
                    actor: deploy::build_actor(c.clone()),
                },
            )
        })
        .collect()
}

#[tokio::test]
async fn test_roym_app_e2e_lifecycle() {
    let _ = ring::default_provider().install_default();
    let [iroh_port, reg_port, gw_port] = alloc_ports::<3>();

    let person_identities_dir = tempfile::tempdir().unwrap();
    let ids_dir = person_identities_dir.path().join("identities");
    fs::create_dir_all(&ids_dir).unwrap();
    let alice = Identity::generate().unwrap();
    let alice_did = substrate::derive_did_key(&alice.public_key());
    let alice_key_path = ids_dir.join("alice.key");
    alice.save_to_path(&alice_key_path).unwrap();

    let p_dir = person_identities_dir.path().to_path_buf();
    let ctx = SubstrateTestContext::setup_with(iroh_port, reg_port, gw_port, move |cfg| {
        if let Some(gw) = cfg.roles.client_gateway.as_mut() {
            gw.person_identities_dir = Some(p_dir);
        }
    })
    .await;

    let gateway_url = format!("http://127.0.0.1:{gw_port}");
    let registry_url = format!("http://127.0.0.1:{reg_port}");
    let client = Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();

    // Prepare SynApp manifest
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest_path = root.join("crates/roym_core/app/roym.toml");
    let manifest_toml = fs::read_to_string(&manifest_path).unwrap();
    let manifest: SynAppManifest = toml::from_str(&manifest_toml).unwrap();
    let catalog = LocalFilesystemCatalog::new(root.clone());
    let compiled = compile(AppInstanceId::new("roym"), &manifest, &catalog).await.unwrap();
    let plan = compiled.plans.last().unwrap().clone();
    let (mut new_plan, masters) = mint_and_substitute_masters(&plan);
    for svc in &mut new_plan.services {
        svc.config.source = root.join(&svc.config.source).to_string_lossy().to_string();
        if let Some(assets) = svc.config.assets.as_mut() {
            assets.archive = root.join(&assets.archive).to_string_lossy().to_string();
        }
    }

    let alias = SubstrateAlias::new("node-0");
    let mut sdk_client = SyneroymClient::new_with_identity(
        ctx.substrate_client.service_id().to_string(),
        registry_url.clone(),
        Identity::from_bytes(&ctx.owner.to_bytes()),
    )
    .with_registry_dht(false);
    sdk_client.connect().await.unwrap();
    sdk_client.inject_kek("aa".repeat(32)).await.unwrap();
    let client_handle = Arc::new(sdk_client);
    let clients = BTreeMap::from([(alias.clone(), client_handle)]);

    let (instance_certs, registry_certs) = certify_and_publish(&new_plan, &masters, &clients).await;
    let targets = deploy_targets(&clients);

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
            generation: 0,
            binding_epochs: &BTreeMap::new(),
        },
        &journal,
        deployment_id,
    )
    .await
    .unwrap();
    assert!(report.is_complete(), "{:?}", report.failures);

    // Find the web service DID
    let web_svc =
        new_plan.services.iter().find(|s| s.logical_ref.service_name.as_str() == "web").unwrap();
    let web_did = web_svc.service_id.as_str();

    let dir_svc = new_plan
        .services
        .iter()
        .find(|s| s.logical_ref.service_name.as_str() == "directory")
        .unwrap();
    let dir_did = dir_svc.service_id.as_str();

    let profile_svc = new_plan
        .services
        .iter()
        .find(|s| s.logical_ref.service_name.as_str() == "profile")
        .unwrap();
    let profile_did = profile_svc.service_id.as_str();

    // Login local via gateway
    let login_res = client
        .post(format!("{gateway_url}/_syneroym/session/login-local"))
        .json(&json!({ "identity": "alice" }))
        .send()
        .await
        .unwrap();
    assert_eq!(login_res.status(), 200);

    // Extract cookie
    let cookie_header = login_res.headers().get("set-cookie").unwrap().to_str().unwrap();
    let cookie_val = cookie_header
        .split(';')
        .find(|part| part.trim().starts_with(SESSION_COOKIE_NAME))
        .unwrap()
        .trim();
    let token = cookie_val.strip_prefix(&format!("{SESSION_COOKIE_NAME}=")).unwrap();

    let s_hash = short_hash(web_did);
    let host_header = format!("s{s_hash}.localhost");

    // 1. GET / on web service's DID returns index.html with CSP meta tag
    let web_resp = client
        .get(format!("{gateway_url}/"))
        .header("Host", &host_header)
        .header("Cookie", format!("{SESSION_COOKIE_NAME}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(web_resp.status(), 200);
    let web_html = web_resp.text().await.unwrap();
    assert!(
        web_html.contains("Content-Security-Policy"),
        "index.html must include CSP meta header"
    );
    assert!(web_html.contains("Roym Hub"), "index.html must contain title");

    // 2. POST /rpc with session.whoami under cookie returns alice DID and auth:
    //    "delegated"
    let whoami_resp = client
        .post(format!("{gateway_url}/rpc"))
        .header("Host", &host_header)
        .header("Cookie", format!("{SESSION_COOKIE_NAME}={token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "session.whoami",
            "params": {}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(whoami_resp.status(), 200);
    let whoami_json: Value = whoami_resp.json().await.unwrap();
    assert_eq!(whoami_json["result"]["did"], alice_did);
    assert_eq!(whoami_json["result"]["auth"], "delegated");

    // 3. POST /rpc with profile.ping under the cookie reaches profile
    let ping_resp = client
        .post(format!("{gateway_url}/rpc"))
        .header("Host", &host_header)
        .header("Cookie", format!("{SESSION_COOKIE_NAME}={token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "profile.ping",
            "params": {}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ping_resp.status(), 200);
    let ping_json: Value = ping_resp.json().await.unwrap();
    assert_eq!(ping_json["result"]["service"], "profile");

    // 4. Open topology on directory vs private on profile
    let reg_client = RegistryClient::new(false, Some(registry_url.clone()));
    let dir_lookup = reg_client.lookup(dir_did, false).await;
    assert!(dir_lookup.is_ok(), "directory is public and must resolve in registry");
    let profile_lookup = reg_client.lookup(profile_did, false).await;
    assert!(profile_lookup.is_err(), "profile is private and must not be published");

    // 5. Plain HTTP client with Authorization: Bearer <token>
    let bearer_resp = client
        .post(format!("{gateway_url}/rpc"))
        .header("Host", &host_header)
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "session.whoami",
            "params": {}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bearer_resp.status(), 200);
    let bearer_json: Value = bearer_resp.json().await.unwrap();
    assert_eq!(bearer_json["result"]["did"], alice_did);
    assert_eq!(bearer_json["result"]["auth"], "delegated");

    // 6. Unauthenticated request reports self-asserted (simulating second local
    //    process)
    let anon_client = Client::new();
    let anon_resp = anon_client
        .post(format!("{gateway_url}/rpc"))
        .header("Host", &host_header)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "session.whoami",
            "params": {}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(anon_resp.status(), 200);
    let anon_json: Value = anon_resp.json().await.unwrap();
    assert_eq!(anon_json["result"]["auth"], "self-asserted");
    assert_ne!(anon_json["result"]["did"], alice_did);

    ctx.teardown().await;
}
