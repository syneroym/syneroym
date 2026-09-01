#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]
//! Substrate-level end-to-end integration tests for Roym identity enrolment
//! and authorization.

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
    config::{AuthRole, IdentityMode},
    dht_registry::DEFAULT_ENDPOINT_NOT_AFTER_SECS,
};
use syneroym_identity::{DelegationCertificate, Identity, substrate};
use syneroym_sdk::{
    SyneroymClient,
    deploy::{
        self, ApplyRequest, DeployTarget, apply_plan, certify_instance, member_registry_record,
    },
};
use syneroym_signed_record::SCOPE_RECORD_SIGNING;

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

struct RoymDeployment {
    ctx: SubstrateTestContext,
    gateway_url: String,
    registry_url: String,
    web_did: String,
    profile_did: String,
    alice: Identity,
    alice_did: String,
    stranger: Identity,
    stranger_did: String,
    _person_identities_dir: tempfile::TempDir,
}

async fn deploy_roym_app() -> RoymDeployment {
    let _ = ring::default_provider().install_default();
    let [iroh_port, reg_port, gw_port] = alloc_ports::<3>();

    let person_identities_dir = tempfile::tempdir().unwrap();
    let ids_dir = person_identities_dir.path().join("identities");
    fs::create_dir_all(&ids_dir).unwrap();

    let ids_dir_for_auth = ids_dir.clone();
    let ctx = SubstrateTestContext::setup_with(iroh_port, reg_port, gw_port, move |cfg| {
        cfg.roles.auth =
            Some(AuthRole { person_identities_dir: Some(ids_dir_for_auth), ..Default::default() });
        if let Some(gw) = cfg.roles.client_gateway.as_mut() {
            gw.identity_mode = IdentityMode::Login;
        }
    })
    .await;

    let alice = Identity::from_bytes(&ctx.owner.to_bytes());
    let alice_did = ctx.owner_did.clone();
    alice.save_to_path(ids_dir.join("alice.key")).unwrap();

    let stranger = Identity::generate().unwrap();
    let stranger_did = substrate::derive_did_key(&stranger.public_key());
    stranger.save_to_path(ids_dir.join("stranger.key")).unwrap();

    let gateway_url = format!("http://127.0.0.1:{gw_port}");
    let registry_url = format!("http://127.0.0.1:{reg_port}");

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

    let web_svc =
        new_plan.services.iter().find(|s| s.logical_ref.service_name.as_str() == "web").unwrap();
    let web_did = web_svc.service_id.as_str().to_string();

    let profile_svc = new_plan
        .services
        .iter()
        .find(|s| s.logical_ref.service_name.as_str() == "profile")
        .unwrap();
    let profile_did = profile_svc.service_id.as_str().to_string();

    RoymDeployment {
        ctx,
        gateway_url,
        registry_url,
        web_did,
        profile_did,
        alice,
        alice_did,
        stranger,
        stranger_did,
        _person_identities_dir: person_identities_dir,
    }
}

async fn login_local(gateway_url: &str, identity: &str) -> String {
    let http = Client::builder().pool_max_idle_per_host(0).build().unwrap();
    let login_resp = http
        .post(format!("{gateway_url}/_syneroym/session/login"))
        .json(&json!({ "method": "local", "identity": identity }))
        .send()
        .await
        .unwrap();
    if !login_resp.status().is_success() {
        let status = login_resp.status();
        let body = login_resp.text().await.unwrap();
        panic!("login_local failed with status {status}: {body}");
    }
    let login_body: Value = login_resp.json().await.unwrap();
    login_body["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn test_roym_identity_e2e() {
    let RoymDeployment {
        ctx,
        gateway_url,
        registry_url: _,
        web_did,
        profile_did: _,
        alice,
        alice_did,
        stranger,
        stranger_did: _,
        _person_identities_dir,
    } = deploy_roym_app().await;

    let http = Client::builder().pool_max_idle_per_host(0).build().unwrap();
    let s_hash = syneroym_core::util::short_hash(&web_did);
    let host_header = format!("s{s_hash}.localhost");

    // 1. Owner/person DID check is established by harness: alice_did ==
    //    ctx.owner_did.
    assert_eq!(alice_did, ctx.owner_did);

    // 2. session login for alice
    let alice_token = login_local(&gateway_url, "alice").await;

    // 3. profile.signing-status
    let rpc_status = http
        .post(format!("{gateway_url}/rpc"))
        .header("Host", &host_header)
        .header("Authorization", format!("Bearer {alice_token}"))
        .header("Cookie", format!("{SESSION_COOKIE_NAME}={alice_token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "profile.signing-status",
            "params": {}
        }))
        .send()
        .await
        .unwrap();
    let status_val: Value = rpc_status.json().await.unwrap();
    let res = &status_val["result"];
    assert_eq!(res["certificate"]["state"], "missing", "full status_val: {status_val}");
    assert_eq!(res["owner_did"], alice_did);
    let signing_did = res["signing_did"].as_str().unwrap().to_string();
    assert!(signing_did.starts_with("did:key:"));

    // 4. profile.set before enrolment -> signing-not-enrolled
    let rpc_set_before = http
        .post(format!("{gateway_url}/rpc"))
        .header("Host", &host_header)
        .header("Authorization", format!("Bearer {alice_token}"))
        .header("Cookie", format!("{SESSION_COOKIE_NAME}={alice_token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "profile.set",
            "params": { "display_name": "Alice" }
        }))
        .send()
        .await
        .unwrap();
    let set_before_val: Value = rpc_set_before.json().await.unwrap();
    assert_eq!(set_before_val["error"]["code"], -32602);
    assert!(set_before_val["error"]["message"].as_str().unwrap().contains("signing-not-enrolled"));

    // 5. Mint cert and install-signing-certificate (what `roym enrol-signing`
    //    performs)
    let signing_pubkey = substrate::resolve_did_key(&signing_did).unwrap();
    let cert = DelegationCertificate::issue(
        &alice,
        signing_pubkey,
        720 * 3600,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();

    let rpc_install = http
        .post(format!("{gateway_url}/rpc"))
        .header("Host", &host_header)
        .header("Authorization", format!("Bearer {alice_token}"))
        .header("Cookie", format!("{SESSION_COOKIE_NAME}={alice_token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "profile.install-signing-certificate",
            "params": { "certificate": cert.to_json().unwrap() }
        }))
        .send()
        .await
        .unwrap();
    let install_val: Value = rpc_install.json().await.unwrap();
    let res = &install_val["result"];
    assert_eq!(res["master_did"], alice_did, "full install_val: {install_val}");
    assert!(res["expires_at_secs"].as_u64().is_some());

    // 6. profile.set { display_name, conversation_address } after enrolment
    let rpc_set_after = http
        .post(format!("{gateway_url}/rpc"))
        .header("Host", &host_header)
        .header("Authorization", format!("Bearer {alice_token}"))
        .header("Cookie", format!("{SESSION_COOKIE_NAME}={alice_token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "profile.set",
            "params": {
                "display_name": "Alice",
                "conversation_address": "syneroym://conv.addr/1"
            }
        }))
        .send()
        .await
        .unwrap();
    let set_after_val: Value = rpc_set_after.json().await.unwrap();
    let envelope_str = set_after_val["result"]["envelope"].as_str().unwrap();
    let envelope: syneroym_signed_record::Envelope = serde_json::from_str(envelope_str).unwrap();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let verified =
        syneroym_signed_record::verify(&envelope, &syneroym_signed_record::VerifyOptions::new(now))
            .unwrap();
    assert_eq!(verified.issuer, alice_did);
    assert_eq!(envelope.payload["display_name"], "Alice");
    assert_eq!(envelope.payload["conversation_address"], "syneroym://conv.addr/1");

    // 7. Stranger logs in and calls profile.get -> -32011 (NotOwner)
    let stranger_token = login_local(&gateway_url, "stranger").await;
    let rpc_stranger_get = http
        .post(format!("{gateway_url}/rpc"))
        .header("Host", &host_header)
        .header("Authorization", format!("Bearer {stranger_token}"))
        .header("Cookie", format!("{SESSION_COOKIE_NAME}={stranger_token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "profile.get",
            "params": {}
        }))
        .send()
        .await
        .unwrap();
    let stranger_get_val: Value = rpc_stranger_get.json().await.unwrap();
    assert_eq!(stranger_get_val["error"]["code"], -32011);

    // 8. Stranger mints a certificate naming stranger as master over profile's
    //    signing key and attempts install -> refused
    let stranger_cert = DelegationCertificate::issue(
        &stranger,
        signing_pubkey,
        720 * 3600,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();
    let rpc_stranger_install = http
        .post(format!("{gateway_url}/rpc"))
        .header("Host", &host_header)
        .header("Authorization", format!("Bearer {stranger_token}"))
        .header("Cookie", format!("{SESSION_COOKIE_NAME}={stranger_token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "profile.install-signing-certificate",
            "params": { "certificate": stranger_cert.to_json().unwrap() }
        }))
        .send()
        .await
        .unwrap();
    let stranger_install_val: Value = rpc_stranger_install.json().await.unwrap();
    assert_eq!(stranger_install_val["error"]["code"], -32011);

    // 9. Stranger calls signing method over HTTP RPC -> refused
    let rpc_stranger_sign = http
        .post(format!("{gateway_url}/rpc"))
        .header("Host", &host_header)
        .header("Authorization", format!("Bearer {stranger_token}"))
        .header("Cookie", format!("{SESSION_COOKIE_NAME}={stranger_token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "signing.sign-record",
            "params": {}
        }))
        .send()
        .await
        .unwrap();
    let stranger_sign_val: Value = rpc_stranger_sign.json().await.unwrap();
    assert!(stranger_sign_val.get("error").is_some());

    ctx.teardown().await;
}
