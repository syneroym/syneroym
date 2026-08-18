#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]
//! Client Gateway Person Identity & Session End-to-End Tests.
//!
//! Covers:
//! - Anonymous request through gateway -> `caller.auth == "self-asserted"`,
//!   `caller.did == node_did`.
//! - Logged-in request via Cookie -> `caller.auth == "delegated"`, `caller.did
//!   == person_did`.
//! - Two people logged in on one node: each `/whoami` and proxied request
//!   reports its own person DID.
//! - A second local process with no token while a session is live sees
//!   `self-asserted:<node_did>`.
//! - Forged login (attacker signs with own key, claims victim DID) -> 401.
//! - Gateway session token is stripped from proxied headers (Cookie and
//!   Bearer); unrelated Authorization headers are preserved.
//! - Cookie takes priority over Bearer when both are present.
//! - Login with unresolvable/unpublished master anchor is refused with 409.
//! - `Expect: 100-continue` handshake over raw TCP completes cleanly.
//! - Expired gateway session falls back to self-asserted node DID and strips
//!   credentials.
//! - Substrate restart clears all in-memory sessions.
//! - Reserved path lifecycle (`/_syneroym/session/*`: challenge, login, whoami,
//!   logout).
//! - Reserved path is never proxied to guest handlers (404 on unknown
//!   endpoint).
//! - `roymctl session` CLI commands lifecycle (`login`, `status`, `token`,
//!   `logout`), file verification, and permissions (0600 on Unix).
//! - `roymctl session` CLI error handling (missing `--as`, unresolvable
//!   anchor).

use std::{
    fs, str,
    time::{Duration, Instant},
};

use reqwest::Client;
use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_core::{
    config::ClientGatewayRole,
    dht_registry::{EndpointInfo, EndpointType, RegistryClient},
    protocol_utils::{SESSION_COOKIE_NAME, gateway_session_assertion},
    test_constants, util,
};
use syneroym_identity::{DelegationCertificate, Identity, delegation::SCOPE_ROUTING, substrate};
use syneroym_sdk::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, SyneroymClient, WasmManifest,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::Mutex,
    time,
};

mod common;
use common::SubstrateTestContext;

/// Serializes substrate boots within this test binary to avoid port, QUIC,
/// and relay server bind collisions under concurrent test runners.
static SUBSTRATE_TEST_LOCK: Mutex<()> = Mutex::const_new(());

fn guest_wasm_manifest(wasm_bytes: Vec<u8>, http_routes: Value) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: Some(http_routes.to_string()),
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: None,
        },
        service_type: ServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(wasm_bytes),
            hash: None,
            interfaces: vec![test_constants::HTTP_GUEST_TEST_DRIVER_INTERFACE.to_string()],
        }),
        registry_certificate: None,
        instance_certificate: None,
    }
}

async fn deploy(client: &SyneroymClient, service_id: &str, manifest: DeployManifest) {
    let params = serde_json::to_value((service_id.to_string(), manifest)).unwrap();
    let res =
        client.request("orchestrator", "deploy", params).await.expect("deploy request failed");
    assert_eq!(res.result, json!({"status": "deployed"}), "deploy did not succeed");
}

async fn setup_gateway_test_node(
    session_ttl_secs: Option<u64>,
) -> (SubstrateTestContext, u16, String, String, String) {
    let _ = ring::default_provider().install_default();
    let [iroh_port, registry_port, gateway_port] = common::alloc_ports::<3>();

    let ctx = SubstrateTestContext::setup_with(iroh_port, registry_port, gateway_port, |config| {
        config.storage.encryption = false;
        config.substrate.enable_bep0044_dht = false;
        if let Some(ttl) = session_ttl_secs {
            config.roles.client_gateway = Some(ClientGatewayRole {
                http_port: gateway_port,
                session_ttl_secs: ttl,
                ..Default::default()
            });
        }
    })
    .await;

    let registry_url = format!("http://localhost:{registry_port}");
    let service_identity = Identity::generate().unwrap();
    let service_did = substrate::derive_did_key(&service_identity.public_key());

    let wasm_bytes = fs::read(test_constants::http_guest_test_wasm_path()).expect("wasm artifact");
    let http_routes = json!({
        "http_routes": [
            {
                "method": "GET",
                "path": "/echo",
                "target": "guest",
                "operation": "handle-request",
                "public": true
            },
            {
                "method": "POST",
                "path": "/echo",
                "target": "guest",
                "operation": "handle-request",
                "public": true
            }
        ]
    });
    let manifest = guest_wasm_manifest(wasm_bytes, http_routes);
    deploy(&ctx.substrate_client, &service_did, manifest).await;

    // Register service endpoint in the registry so gateway can resolve alias/DID
    let info = EndpointInfo {
        service_id: service_did.clone(),
        substrate_id: ctx.substrate_client.service_id().to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: ctx.substrate_mechanisms.clone(),
        nickname: Some("test-svc".to_string()),
        is_private: false,
        ttl: None,
        not_after: u64::MAX / 2,
        generation: 0,
    };
    let signed = info.sign(&service_identity).expect("failed to sign endpoint info");
    let http = Client::new();
    let reg_res = http
        .post(format!("{registry_url}/register"))
        .json(&signed)
        .send()
        .await
        .expect("failed to register endpoint in HTTP registry");
    assert!(reg_res.status().is_success());

    let host = util::generate_service_host(
        Some("test-svc"),
        &service_did,
        Some("http-native"),
        "localhost",
    )
    .unwrap();

    (ctx, gateway_port, registry_url, service_did, host)
}

async fn login_to_gateway(
    gateway_url: &str,
    person: &Identity,
    registry_url: &str,
    expires_hours: u64,
) -> (String, String) {
    let client = Client::new();
    let person_did = substrate::derive_did_key(&person.public_key());

    // 1. Challenge
    let ch_resp: Value = client
        .post(format!("{gateway_url}/_syneroym/session/challenge"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let nonce = ch_resp["nonce"].as_str().unwrap();
    let node_did = ch_resp["node_did"].as_str().unwrap();

    // 2. Delegation + Challenge Signature
    let node_pubkey = substrate::resolve_did_key(node_did).unwrap();
    let cert = DelegationCertificate::issue(
        person,
        node_pubkey,
        expires_hours * 3600,
        SCOPE_ROUTING.to_string(),
    )
    .unwrap();
    let assertion = gateway_session_assertion(node_did, nonce, &person_did);
    let sig = person.sign_json(&assertion).unwrap();

    // 3. Publish Master Anchor
    let reg_client = RegistryClient::new(false, Some(registry_url.to_string()));
    reg_client.refresh_master_anchor(person).await.unwrap();

    // 4. Login
    let login_body = json!({
        "person_did": person_did,
        "nonce": nonce,
        "signature": sig,
        "delegation": cert,
    });
    let login_resp = client
        .post(format!("{gateway_url}/_syneroym/session/login"))
        .json(&login_body)
        .send()
        .await
        .unwrap();
    assert_eq!(login_resp.status(), 200);
    let resp_val: Value = login_resp.json().await.unwrap();
    let token = resp_val["token"].as_str().unwrap().to_string();
    (token, person_did)
}

fn test_client() -> Client {
    Client::builder().pool_max_idle_per_host(0).build().expect("failed to build test client")
}

async fn poll_until_guest_ready(client: &Client, gateway_url: &str, host: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match client
            .get(format!("{gateway_url}/echo"))
            .header("Host", host)
            .header("Connection", "close")
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => return,
            _ if Instant::now() < deadline => time::sleep(Duration::from_millis(200)).await,
            Ok(r) => panic!("gateway poll failed with status {}", r.status()),
            Err(e) => panic!("gateway poll failed: {e}"),
        }
    }
}

/// Test 16: Anonymous request through gateway sees self-asserted node DID.
#[tokio::test]
async fn test_16_anonymous_request_sees_self_asserted_node_did() {
    let _test_lock = SUBSTRATE_TEST_LOCK.lock().await;
    let (ctx, gateway_port, _reg_url, _svc_did, host) = setup_gateway_test_node(None).await;
    let gateway_url = format!("http://127.0.0.1:{gateway_port}");
    let client = test_client();
    poll_until_guest_ready(&client, &gateway_url, &host).await;

    let resp = client
        .get(format!("{gateway_url}/echo"))
        .header("Host", &host)
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();

    assert_eq!(body["caller"]["auth"], "self-asserted");
    assert!(
        body["caller"]["did"].as_str().unwrap().starts_with("did:key:"),
        "caller did must be a valid DID: {:?}",
        body["caller"]
    );
    ctx.teardown().await;
}

/// Test 17: Logged-in request via Cookie sees delegated person DID.
#[tokio::test]
async fn test_17_logged_in_request_via_cookie_sees_delegated_person_did() {
    let _test_lock = SUBSTRATE_TEST_LOCK.lock().await;
    let (ctx, gateway_port, reg_url, _svc_did, host) = setup_gateway_test_node(None).await;
    let gateway_url = format!("http://127.0.0.1:{gateway_port}");
    let client = test_client();
    poll_until_guest_ready(&client, &gateway_url, &host).await;

    let person = Identity::generate().unwrap();
    let (token, person_did) = login_to_gateway(&gateway_url, &person, &reg_url, 24).await;

    let resp = client
        .get(format!("{gateway_url}/echo"))
        .header("Host", &host)
        .header("Cookie", format!("{SESSION_COOKIE_NAME}={token}"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();

    assert_eq!(body["caller"]["auth"], "delegated");
    assert_eq!(body["caller"]["did"], person_did);
    ctx.teardown().await;
}

/// Test 18: Two people on one node -> two logins, two tokens;
/// each /whoami and /echo returns its own DID and never the other's, never the
/// node's.
#[tokio::test]
async fn test_18_two_people_logged_in_each_whoami_and_echo_returns_own_did() {
    let _test_lock = SUBSTRATE_TEST_LOCK.lock().await;
    let (ctx, gateway_port, reg_url, _svc_did, host) = setup_gateway_test_node(None).await;
    let gateway_url = format!("http://127.0.0.1:{gateway_port}");
    let client = test_client();
    poll_until_guest_ready(&client, &gateway_url, &host).await;

    let person_a = Identity::generate().unwrap();
    let (token_a, person_a_did) = login_to_gateway(&gateway_url, &person_a, &reg_url, 24).await;

    let person_b = Identity::generate().unwrap();
    let (token_b, person_b_did) = login_to_gateway(&gateway_url, &person_b, &reg_url, 24).await;
    assert_ne!(person_a_did, person_b_did);

    // Whoami for Alice
    let resp_whoami_a = client
        .get(format!("{gateway_url}/_syneroym/session/whoami"))
        .header("Authorization", format!("Bearer {token_a}"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp_whoami_a.status(), 200);
    let val_whoami_a: Value = resp_whoami_a.json().await.unwrap();
    assert_eq!(val_whoami_a["person_did"], person_a_did);
    assert_eq!(val_whoami_a["auth"], "delegated");

    // Whoami for Bob
    let resp_whoami_b = client
        .get(format!("{gateway_url}/_syneroym/session/whoami"))
        .header("Authorization", format!("Bearer {token_b}"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp_whoami_b.status(), 200);
    let val_whoami_b: Value = resp_whoami_b.json().await.unwrap();
    assert_eq!(val_whoami_b["person_did"], person_b_did);
    assert_eq!(val_whoami_b["auth"], "delegated");

    // Re-verify Alice whoami still returns Alice
    let resp_whoami_a_re = client
        .get(format!("{gateway_url}/_syneroym/session/whoami"))
        .header("Authorization", format!("Bearer {token_a}"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp_whoami_a_re.status(), 200);
    let val_whoami_a_re: Value = resp_whoami_a_re.json().await.unwrap();
    assert_eq!(val_whoami_a_re["person_did"], person_a_did);

    // Echo for Alice
    let resp_echo_a = client
        .get(format!("{gateway_url}/echo"))
        .header("Host", &host)
        .header("Authorization", format!("Bearer {token_a}"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp_echo_a.status(), 200);
    let body_echo_a: Value = resp_echo_a.json().await.unwrap();
    assert_eq!(body_echo_a["caller"]["did"], person_a_did);

    // Echo for Bob
    let resp_echo_b = client
        .get(format!("{gateway_url}/echo"))
        .header("Host", &host)
        .header("Authorization", format!("Bearer {token_b}"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp_echo_b.status(), 200);
    let body_echo_b: Value = resp_echo_b.json().await.unwrap();
    assert_eq!(body_echo_b["caller"]["did"], person_b_did);

    ctx.teardown().await;
}

/// Test 19: A second local process with no token while a session
/// is live sees `self-asserted:<node_did>`.
#[tokio::test]
async fn test_19_second_local_process_without_token_while_session_live_sees_self_asserted() {
    let _test_lock = SUBSTRATE_TEST_LOCK.lock().await;
    let (ctx, gateway_port, reg_url, _svc_did, host) = setup_gateway_test_node(None).await;
    let gateway_url = format!("http://127.0.0.1:{gateway_port}");
    let client = test_client();
    poll_until_guest_ready(&client, &gateway_url, &host).await;

    let person = Identity::generate().unwrap();
    let (token, person_did) = login_to_gateway(&gateway_url, &person, &reg_url, 24).await;

    // Process A: with token
    let resp_a = client
        .get(format!("{gateway_url}/echo"))
        .header("Host", &host)
        .header("Cookie", format!("{SESSION_COOKIE_NAME}={token}"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp_a.status(), 200);
    let body_a: Value = resp_a.json().await.unwrap();
    assert_eq!(body_a["caller"]["auth"], "delegated");
    assert_eq!(body_a["caller"]["did"], person_did);

    // Process B: concurrent request with no token to the same node
    let resp_b = client
        .get(format!("{gateway_url}/echo"))
        .header("Host", &host)
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp_b.status(), 200);
    let body_b: Value = resp_b.json().await.unwrap();
    assert_eq!(body_b["caller"]["auth"], "self-asserted");
    assert_ne!(body_b["caller"]["did"], person_did);

    ctx.teardown().await;
}

/// Test 20: A forged login (attacker signs with own key, claims
/// Alice's DID) is rejected with 401 at the gateway.
#[tokio::test]
async fn test_20_forged_login_is_rejected_with_401() {
    let _test_lock = SUBSTRATE_TEST_LOCK.lock().await;
    let (ctx, gateway_port, reg_url, _svc_did, host) = setup_gateway_test_node(None).await;
    let gateway_url = format!("http://127.0.0.1:{gateway_port}");
    let client = test_client();
    poll_until_guest_ready(&client, &gateway_url, &host).await;

    let alice = Identity::generate().unwrap();
    let alice_did = substrate::derive_did_key(&alice.public_key());
    let eve = Identity::generate().unwrap();

    // Publish Alice's master anchor
    let reg_client = RegistryClient::new(false, Some(reg_url));
    reg_client.refresh_master_anchor(&alice).await.unwrap();

    // 1. Challenge
    let ch_resp: Value = client
        .post(format!("{gateway_url}/_syneroym/session/challenge"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let nonce = ch_resp["nonce"].as_str().unwrap();
    let node_did = ch_resp["node_did"].as_str().unwrap();

    // 2. Eve signs the assertion claiming Alice's DID
    let node_pubkey = substrate::resolve_did_key(node_did).unwrap();
    let cert =
        DelegationCertificate::issue(&alice, node_pubkey, 24 * 3600, SCOPE_ROUTING.to_string())
            .unwrap();
    let assertion = gateway_session_assertion(node_did, nonce, &alice_did);
    let forged_sig = eve.sign_json(&assertion).unwrap();

    // 3. Forged login attempt -> 401
    let login_body = json!({
        "person_did": alice_did,
        "nonce": nonce,
        "signature": forged_sig,
        "delegation": cert,
    });
    let login_resp = client
        .post(format!("{gateway_url}/_syneroym/session/login"))
        .json(&login_body)
        .send()
        .await
        .unwrap();
    assert_eq!(login_resp.status(), 401);
    let err_val: Value = login_resp.json().await.unwrap();
    assert_eq!(err_val["error"], "invalid signature");

    // Ensure requests without a valid session still report self-asserted
    let resp = client
        .get(format!("{gateway_url}/echo"))
        .header("Host", &host)
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["caller"]["auth"], "self-asserted");

    ctx.teardown().await;
}

/// Test 21: Gateway session token is stripped from proxied headers.
#[tokio::test]
async fn test_21_gateway_session_token_is_stripped_from_proxied_headers() {
    let _test_lock = SUBSTRATE_TEST_LOCK.lock().await;
    let (ctx, gateway_port, reg_url, _svc_did, host) = setup_gateway_test_node(None).await;
    let gateway_url = format!("http://127.0.0.1:{gateway_port}");
    let client = test_client();
    poll_until_guest_ready(&client, &gateway_url, &host).await;

    let person = Identity::generate().unwrap();
    let (token, _person_did) = login_to_gateway(&gateway_url, &person, &reg_url, 24).await;

    // A: Cookie with multiple entries
    let resp = client
        .get(format!("{gateway_url}/echo"))
        .header("Host", &host)
        .header("Cookie", format!("other=1; {SESSION_COOKIE_NAME}={token}; third=2"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let headers: Vec<(String, String)> = serde_json::from_value(body["headers"].clone()).unwrap();
    let cookie_val = headers.iter().find(|(name, _)| name == "cookie").map(|(_, v)| v.as_str());
    assert_eq!(cookie_val, Some("other=1; third=2"));

    // B: Authorization Bearer (session token)
    let resp = client
        .get(format!("{gateway_url}/echo"))
        .header("Host", &host)
        .header("Authorization", format!("Bearer {token}"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let headers: Vec<(String, String)> = serde_json::from_value(body["headers"].clone()).unwrap();
    assert!(
        !headers.iter().any(|(name, _)| name == "authorization"),
        "Authorization header with session token must be stripped"
    );

    // C: Authorization Basic (non-session auth preserved)
    let resp = client
        .get(format!("{gateway_url}/echo"))
        .header("Host", &host)
        .header("Authorization", "Basic dXNlcjpwYXNz")
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let headers: Vec<(String, String)> = serde_json::from_value(body["headers"].clone()).unwrap();
    let auth_val =
        headers.iter().find(|(name, _)| name == "authorization").map(|(_, v)| v.as_str());
    assert_eq!(auth_val, Some("Basic dXNlcjpwYXNz"));

    // D: Both Cookie (session) and Authorization (app bearer) -> Cookie stripped,
    // Authorization preserved
    let resp = client
        .get(format!("{gateway_url}/echo"))
        .header("Host", &host)
        .header("Cookie", format!("{SESSION_COOKIE_NAME}={token}"))
        .header("Authorization", "Bearer app_secret_token_123")
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let headers: Vec<(String, String)> = serde_json::from_value(body["headers"].clone()).unwrap();
    assert!(!headers.iter().any(|(name, _)| name == "cookie"), "Session cookie must be stripped");
    let auth_val =
        headers.iter().find(|(name, _)| name == "authorization").map(|(_, v)| v.as_str());
    assert_eq!(auth_val, Some("Bearer app_secret_token_123"));

    ctx.teardown().await;
}

/// Test 22: Cookie takes priority over Bearer when both are present.
/// The session cookie determines the caller identity and is stripped;
/// the unrelated application Authorization Bearer header is forwarded
/// untouched.
#[tokio::test]
async fn test_22_cookie_takes_priority_over_bearer() {
    let _test_lock = SUBSTRATE_TEST_LOCK.lock().await;
    let (ctx, gateway_port, reg_url, _svc_did, host) = setup_gateway_test_node(None).await;
    let gateway_url = format!("http://127.0.0.1:{gateway_port}");
    let client = test_client();
    poll_until_guest_ready(&client, &gateway_url, &host).await;

    let person_a = Identity::generate().unwrap();
    let (token_a, person_a_did) = login_to_gateway(&gateway_url, &person_a, &reg_url, 24).await;

    let app_token = "app_bearer_secret_xyz123";

    let resp = client
        .get(format!("{gateway_url}/echo"))
        .header("Host", &host)
        .header("Cookie", format!("{SESSION_COOKIE_NAME}={token_a}"))
        .header("Authorization", format!("Bearer {app_token}"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();

    // Cookie took priority, so person A is the caller DID
    assert_eq!(body["caller"]["auth"], "delegated");
    assert_eq!(body["caller"]["did"], person_a_did);

    // Check that session cookie was stripped and app Bearer was forwarded untouched
    let headers: Vec<(String, String)> = serde_json::from_value(body["headers"].clone()).unwrap();
    assert!(
        !headers.iter().any(|(name, _)| name == "cookie"),
        "syneroym_session cookie must be stripped"
    );
    let auth_header =
        headers.iter().find(|(name, _)| name == "authorization").map(|(_, v)| v.as_str());
    assert_eq!(auth_header, Some(format!("Bearer {app_token}").as_str()));

    ctx.teardown().await;
}

/// Test 23: Login with no published master anchor is refused
/// with 409 naming the anchor.
#[tokio::test]
async fn test_23_login_with_no_published_anchor_is_refused_with_409() {
    let _test_lock = SUBSTRATE_TEST_LOCK.lock().await;
    let (ctx, gateway_port, _reg_url, _svc_did, host) = setup_gateway_test_node(None).await;
    let gateway_url = format!("http://127.0.0.1:{gateway_port}");
    let client = test_client();
    poll_until_guest_ready(&client, &gateway_url, &host).await;

    let person = Identity::generate().unwrap();
    let person_did = substrate::derive_did_key(&person.public_key());

    // 1. Challenge
    let ch_resp: Value = client
        .post(format!("{gateway_url}/_syneroym/session/challenge"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let nonce = ch_resp["nonce"].as_str().unwrap();
    let node_did = ch_resp["node_did"].as_str().unwrap();

    // 2. Sign delegation & challenge, but DO NOT publish master anchor
    let node_pubkey = substrate::resolve_did_key(node_did).unwrap();
    let cert =
        DelegationCertificate::issue(&person, node_pubkey, 24 * 3600, SCOPE_ROUTING.to_string())
            .unwrap();
    let assertion = gateway_session_assertion(node_did, nonce, &person_did);
    let sig = person.sign_json(&assertion).unwrap();

    // 3. Login attempt -> 409
    let login_body = json!({
        "person_did": person_did,
        "nonce": nonce,
        "signature": sig,
        "delegation": cert,
    });
    let login_resp = client
        .post(format!("{gateway_url}/_syneroym/session/login"))
        .json(&login_body)
        .send()
        .await
        .unwrap();
    assert_eq!(login_resp.status(), 409);
    let err_val: Value = login_resp.json().await.unwrap();
    assert!(
        err_val["error"].as_str().unwrap().contains("master anchor is not resolvable"),
        "error message must name master anchor resolution failure: {:?}",
        err_val
    );

    // Requests through gateway still report self-asserted
    let resp = client
        .get(format!("{gateway_url}/echo"))
        .header("Host", &host)
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["caller"]["auth"], "self-asserted");

    ctx.teardown().await;
}

/// Test 24: Expect: 100-continue completes over raw TCP stream.
#[tokio::test]
async fn test_24_expect_100_continue_handshake() {
    let _test_lock = SUBSTRATE_TEST_LOCK.lock().await;
    let (ctx, gateway_port, reg_url, _svc_did, _host) = setup_gateway_test_node(None).await;
    let gateway_url = format!("http://127.0.0.1:{gateway_port}");
    let client = test_client();

    let person = Identity::generate().unwrap();
    let person_did = substrate::derive_did_key(&person.public_key());

    // 1. Challenge
    let ch_resp: Value = client
        .post(format!("{gateway_url}/_syneroym/session/challenge"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let nonce = ch_resp["nonce"].as_str().unwrap();
    let node_did = ch_resp["node_did"].as_str().unwrap();

    let node_pubkey = substrate::resolve_did_key(node_did).unwrap();
    let cert =
        DelegationCertificate::issue(&person, node_pubkey, 24 * 3600, SCOPE_ROUTING.to_string())
            .unwrap();
    let assertion = gateway_session_assertion(node_did, nonce, &person_did);
    let sig = person.sign_json(&assertion).unwrap();

    let reg_client = RegistryClient::new(false, Some(reg_url));
    reg_client.refresh_master_anchor(&person).await.unwrap();

    let login_body = serde_json::to_vec(&json!({
        "person_did": person_did,
        "nonce": nonce,
        "signature": sig,
        "delegation": cert,
    }))
    .unwrap();

    // Connect raw TCP stream
    let mut tcp = TcpStream::connect(format!("127.0.0.1:{gateway_port}")).await.unwrap();
    let req_headers = format!(
        "POST /_syneroym/session/login HTTP/1.1\r\nHost: localhost:{gateway_port}\r\nExpect: \
         100-continue\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: \
         close\r\n\r\n",
        login_body.len()
    );
    tcp.write_all(req_headers.as_bytes()).await.unwrap();

    // Read the 100 Continue
    let mut buf = [0u8; 1024];
    let n = tcp.read(&mut buf).await.unwrap();
    let resp1 = str::from_utf8(&buf[..n]).unwrap();
    assert!(resp1.starts_with("HTTP/1.1 100 Continue"), "expected 100 Continue, got {resp1}");

    // Send the body
    tcp.write_all(&login_body).await.unwrap();

    // Read the final response
    let mut resp_bytes = Vec::new();
    loop {
        let n = tcp.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        resp_bytes.extend_from_slice(&buf[..n]);
    }
    let final_resp = str::from_utf8(&resp_bytes).unwrap();
    assert!(final_resp.starts_with("HTTP/1.1 200"), "expected HTTP/1.1 200, got {final_resp}");
    assert!(final_resp.contains("syneroym_session="));

    ctx.teardown().await;
}

/// Test 25: Expired gateway session falls back to self-asserted node DID.
#[tokio::test]
async fn test_25_expired_gateway_session_falls_back_to_self_asserted_node_did() {
    let _test_lock = SUBSTRATE_TEST_LOCK.lock().await;
    let (ctx, gateway_port, reg_url, _svc_did, host) = setup_gateway_test_node(Some(1)).await;
    let gateway_url = format!("http://127.0.0.1:{gateway_port}");
    let client = test_client();
    poll_until_guest_ready(&client, &gateway_url, &host).await;

    let person = Identity::generate().unwrap();
    let (token, _person_did) = login_to_gateway(&gateway_url, &person, &reg_url, 24).await;

    // Wait for the 1-second session to expire
    time::sleep(Duration::from_secs(2)).await;

    let resp = client
        .get(format!("{gateway_url}/echo"))
        .header("Host", &host)
        .header("Authorization", format!("Bearer {token}"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();

    assert_eq!(body["caller"]["auth"], "self-asserted");

    // Verify the expired token header is stripped even though lookup returned None
    // (HIGH-3)
    let headers: Vec<(String, String)> = serde_json::from_value(body["headers"].clone()).unwrap();
    assert!(
        !headers.iter().any(|(name, _)| name == "authorization"),
        "Expired bearer token must be stripped from proxied request headers"
    );

    ctx.teardown().await;
}

/// Test 26: Substrate restart clears all in-memory sessions.
#[tokio::test]
async fn test_26_restart_clears_all_sessions() {
    let _test_lock = SUBSTRATE_TEST_LOCK.lock().await;
    let (ctx, gateway_port, reg_url, _svc_did, host) = setup_gateway_test_node(None).await;
    let gateway_url = format!("http://127.0.0.1:{gateway_port}");
    let client = test_client();
    poll_until_guest_ready(&client, &gateway_url, &host).await;

    let person = Identity::generate().unwrap();
    let (token, person_did) = login_to_gateway(&gateway_url, &person, &reg_url, 24).await;

    let resp = client
        .get(format!("{gateway_url}/echo"))
        .header("Host", &host)
        .header("Authorization", format!("Bearer {token}"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["caller"]["auth"], "delegated");
    assert_eq!(body["caller"]["did"], person_did);

    // Teardown ctx gracefully
    ctx.teardown().await;

    // Boot a new substrate context
    let (ctx2, gateway_port2, _reg_url2, _svc_did2, host2) = setup_gateway_test_node(None).await;
    let gateway_url2 = format!("http://127.0.0.1:{gateway_port2}");
    poll_until_guest_ready(&client, &gateway_url2, &host2).await;

    // Use token from previous run
    let resp2 = client
        .get(format!("{gateway_url2}/echo"))
        .header("Host", &host2)
        .header("Authorization", format!("Bearer {token}"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), 200);
    let body2: Value = resp2.json().await.unwrap();
    assert_eq!(
        body2["caller"]["auth"], "self-asserted",
        "restarted gateway must not remember previously issued session tokens"
    );

    ctx2.teardown().await;
}

/// Test 27: Reserved path challenge, login, whoami, logout lifecycle.
#[tokio::test]
async fn test_27_reserved_path_challenge_login_whoami_logout_lifecycle() {
    let _test_lock = SUBSTRATE_TEST_LOCK.lock().await;
    let (ctx, gateway_port, reg_url, _svc_did, _host) = setup_gateway_test_node(None).await;
    let gateway_url = format!("http://127.0.0.1:{gateway_port}");
    let client = test_client();

    let person = Identity::generate().unwrap();
    let person_did = substrate::derive_did_key(&person.public_key());

    // 1. Challenge
    let ch_resp = client
        .post(format!("{gateway_url}/_syneroym/session/challenge"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(ch_resp.status(), 200);
    let ch: Value = ch_resp.json().await.unwrap();
    let nonce = ch["nonce"].as_str().unwrap();
    let node_did = ch["node_did"].as_str().unwrap();

    // 2. Sign delegation & assertion
    let node_pubkey = substrate::resolve_did_key(node_did).unwrap();
    let cert =
        DelegationCertificate::issue(&person, node_pubkey, 24 * 3600, SCOPE_ROUTING.to_string())
            .unwrap();
    let assertion = gateway_session_assertion(node_did, nonce, &person_did);
    let sig = person.sign_json(&assertion).unwrap();

    // 3. Publish Master Anchor
    let reg_client = RegistryClient::new(false, Some(reg_url));
    reg_client.refresh_master_anchor(&person).await.unwrap();

    // 4. Login
    let login_body = json!({
        "person_did": person_did,
        "nonce": nonce,
        "signature": sig,
        "delegation": cert,
    });
    let login_resp = client
        .post(format!("{gateway_url}/_syneroym/session/login"))
        .json(&login_body)
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(login_resp.status(), 200);
    assert!(login_resp.headers().contains_key("set-cookie"));
    let login_val: Value = login_resp.json().await.unwrap();
    let token = login_val["token"].as_str().unwrap();
    assert_eq!(login_val["person_did"], person_did);

    // 5. Whoami
    let whoami_resp = client
        .get(format!("{gateway_url}/_syneroym/session/whoami"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(whoami_resp.status(), 200);
    let whoami_val: Value = whoami_resp.json().await.unwrap();
    assert_eq!(whoami_val["person_did"], person_did);
    assert_eq!(whoami_val["auth"], "delegated");

    // 6. Logout
    let logout_resp = client
        .post(format!("{gateway_url}/_syneroym/session/logout"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(logout_resp.status(), 200);
    let cookie_header = logout_resp.headers().get("set-cookie").unwrap().to_str().unwrap();
    assert!(cookie_header.contains("Max-Age=0"));

    // 7. Whoami after logout
    let whoami_after = client
        .get(format!("{gateway_url}/_syneroym/session/whoami"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(whoami_after.status(), 401);

    ctx.teardown().await;
}

/// Test 28: Reserved path is never proxied to guest handlers.
#[tokio::test]
async fn test_28_reserved_path_is_never_proxied_to_guest() {
    let _test_lock = SUBSTRATE_TEST_LOCK.lock().await;
    let (ctx, gateway_port, _reg_url, _svc_did, host) = setup_gateway_test_node(None).await;
    let gateway_url = format!("http://127.0.0.1:{gateway_port}");
    let client = test_client();
    poll_until_guest_ready(&client, &gateway_url, &host).await;

    // Unknown endpoint under reserved prefix
    let resp = client
        .get(format!("{gateway_url}/_syneroym/unknown"))
        .header("Host", &host)
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let val: Value = resp.json().await.unwrap();
    assert_eq!(val["error"], "unknown gateway endpoint");

    let resp2 = client
        .post(format!("{gateway_url}/_syneroym/session/custom"))
        .header("Host", &host)
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), 404);

    ctx.teardown().await;
}

/// Test 29: roymctl session CLI commands lifecycle.
#[tokio::test]
async fn test_29_roymctl_session_cli_lifecycle() {
    let _test_lock = SUBSTRATE_TEST_LOCK.lock().await;
    let (ctx, gateway_port, reg_url, _svc_did, _host) = setup_gateway_test_node(None).await;
    let gateway_url = format!("http://127.0.0.1:{gateway_port}");

    let temp_dir = tempfile::tempdir().unwrap();
    let identities_dir = temp_dir.path().join("identities");
    fs::create_dir_all(&identities_dir).unwrap();

    let person = Identity::generate().unwrap();
    let person_did = substrate::derive_did_key(&person.public_key());
    person.save_to_path(identities_dir.join("alice.key")).unwrap();

    // 1. Login
    let login_cmd = roymctl::commands::session::SessionCommands::Login {
        gateway_url: gateway_url.clone(),
        registry_url: Some(reg_url),
        expires_hours: 24,
    };
    roymctl::commands::session::handle(&login_cmd, temp_dir.path(), Some("alice")).await.unwrap();

    // Verify session file exists and contents
    let sanitized_url: String =
        gateway_url.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let session_file = temp_dir.path().join("sessions").join(format!("{sanitized_url}.json"));
    assert!(session_file.exists(), "session file must exist at {}", session_file.display());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::metadata(&session_file).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600, "session file must be mode 0600 on unix");
    }

    let file_content = fs::read_to_string(&session_file).unwrap();
    let file_json: Value = serde_json::from_str(&file_content).unwrap();
    assert_eq!(file_json["person_did"], person_did);
    assert_eq!(file_json["gateway_url"], gateway_url);
    assert!(file_json["token"].as_str().unwrap().len() >= 32);

    // 2. Status
    let status_cmd =
        roymctl::commands::session::SessionCommands::Status { gateway_url: gateway_url.clone() };
    roymctl::commands::session::handle(&status_cmd, temp_dir.path(), None).await.unwrap();

    // 3. Token
    let token_cmd =
        roymctl::commands::session::SessionCommands::Token { gateway_url: gateway_url.clone() };
    roymctl::commands::session::handle(&token_cmd, temp_dir.path(), None).await.unwrap();

    // 4. Logout
    let logout_cmd =
        roymctl::commands::session::SessionCommands::Logout { gateway_url: gateway_url.clone() };
    roymctl::commands::session::handle(&logout_cmd, temp_dir.path(), None).await.unwrap();

    // Verify session file was deleted
    assert!(!session_file.exists(), "session file must be deleted after logout");

    // 5. Status after logout -> fails
    let err =
        roymctl::commands::session::handle(&status_cmd, temp_dir.path(), None).await.unwrap_err();
    assert!(err.to_string().contains("no active session"), "{err}");

    ctx.teardown().await;
}

/// Test 30: roymctl session CLI error handling.
#[tokio::test]
async fn test_30_roymctl_session_cli_error_handling() {
    let _test_lock = SUBSTRATE_TEST_LOCK.lock().await;
    let temp_dir = tempfile::tempdir().unwrap();

    // 1. Missing --as flag
    let login_cmd = roymctl::commands::session::SessionCommands::Login {
        gateway_url: "http://localhost:7960".to_string(),
        registry_url: None,
        expires_hours: 24,
    };
    let err =
        roymctl::commands::session::handle(&login_cmd, temp_dir.path(), None).await.unwrap_err();
    assert!(err.to_string().contains("--as"), "error must name missing --as flag: {err}");

    // 2. Identity file does not exist
    let err_no_key =
        roymctl::commands::session::handle(&login_cmd, temp_dir.path(), Some("nonexistent"))
            .await
            .unwrap_err();
    assert!(
        err_no_key.to_string().contains("not found"),
        "error must indicate identity not found: {err_no_key}"
    );

    // 3. Unresolvable anchor surfaces 409 error
    let (ctx, gateway_port, _reg_url, _svc_did, _host) = setup_gateway_test_node(None).await;
    let gateway_url = format!("http://127.0.0.1:{gateway_port}");

    let identities_dir = temp_dir.path().join("identities");
    fs::create_dir_all(&identities_dir).unwrap();
    let person = Identity::generate().unwrap();
    person.save_to_path(identities_dir.join("bob.key")).unwrap();

    let login_cmd_no_reg = roymctl::commands::session::SessionCommands::Login {
        gateway_url,
        registry_url: None,
        expires_hours: 24,
    };
    let err_409 =
        roymctl::commands::session::handle(&login_cmd_no_reg, temp_dir.path(), Some("bob"))
            .await
            .unwrap_err();
    assert!(
        err_409.to_string().contains("409") || err_409.to_string().contains("master anchor"),
        "error must surface 409 unresolvable master anchor: {err_409}"
    );

    ctx.teardown().await;
}
