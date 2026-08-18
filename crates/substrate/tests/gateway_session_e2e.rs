#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]
//! Client Gateway Person Identity & Session End-to-End Tests.
//!
//! Covers:
//! - Anonymous request through gateway -> `caller.auth == "self-asserted"`,
//!   `caller.did == node_did`.
//! - Logged-in request via Cookie -> `caller.auth == "delegated"`, `caller.did
//!   == person_did`.
//! - Logged-in request via Authorization Bearer -> `caller.auth ==
//!   "delegated"`, `caller.did == person_did`.
//! - Gateway session token is stripped from proxied headers (Cookie and
//!   Bearer).
//! - Bearer takes priority over Cookie when both are present.
//! - Expired gateway session falls back to self-asserted node DID.
//! - Substrate restart clears all in-memory sessions.
//! - Reserved path lifecycle (`/_syneroym/session/*`: challenge, login, whoami,
//!   logout).
//! - Reserved path is never proxied to guest handlers (404 on unknown
//!   endpoint).
//! - `roymctl session` CLI commands lifecycle (`login`, `status`, `token`,
//!   `logout`).

use std::{
    fs,
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
use tokio::time;

mod common;
use common::SubstrateTestContext;

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

/// Test 18: Logged-in request via Bearer sees delegated person DID.
#[tokio::test]
async fn test_18_logged_in_request_via_bearer_sees_delegated_person_did() {
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
    ctx.teardown().await;
}

/// Test 19: Gateway session token is stripped from proxied headers.
#[tokio::test]
async fn test_19_gateway_session_token_is_stripped_from_proxied_headers() {
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

    ctx.teardown().await;
}

/// Test 20: Bearer takes priority over Cookie when both are present.
#[tokio::test]
async fn test_20_bearer_takes_priority_over_cookie() {
    let (ctx, gateway_port, reg_url, _svc_did, host) = setup_gateway_test_node(None).await;
    let gateway_url = format!("http://127.0.0.1:{gateway_port}");
    let client = test_client();
    poll_until_guest_ready(&client, &gateway_url, &host).await;

    let person_a = Identity::generate().unwrap();
    let (token_a, _person_a_did) = login_to_gateway(&gateway_url, &person_a, &reg_url, 24).await;

    let person_b = Identity::generate().unwrap();
    let (token_b, person_b_did) = login_to_gateway(&gateway_url, &person_b, &reg_url, 24).await;

    let resp = client
        .get(format!("{gateway_url}/echo"))
        .header("Host", &host)
        .header("Cookie", format!("{SESSION_COOKIE_NAME}={token_a}"))
        .header("Authorization", format!("Bearer {token_b}"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();

    assert_eq!(body["caller"]["auth"], "delegated");
    assert_eq!(body["caller"]["did"], person_b_did);
    ctx.teardown().await;
}

/// Test 21: Expired gateway session falls back to self-asserted node DID.
#[tokio::test]
async fn test_21_expired_gateway_session_falls_back_to_self_asserted_node_did() {
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
    ctx.teardown().await;
}

/// Test 22: Substrate restart clears all in-memory sessions.
#[tokio::test]
async fn test_22_restart_clears_all_sessions() {
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

/// Test 23: Reserved path challenge, login, whoami, logout lifecycle.
#[tokio::test]
async fn test_23_reserved_path_challenge_login_whoami_logout_lifecycle() {
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

/// Test 24: Reserved path is never proxied to guest handlers.
#[tokio::test]
async fn test_24_reserved_path_is_never_proxied_to_guest() {
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

/// Test 25: roymctl session CLI commands lifecycle.
#[tokio::test]
async fn test_25_roymctl_session_cli_lifecycle() {
    let (ctx, gateway_port, reg_url, _svc_did, _host) = setup_gateway_test_node(None).await;
    let gateway_url = format!("http://127.0.0.1:{gateway_port}");

    let temp_dir = tempfile::tempdir().unwrap();
    let identities_dir = temp_dir.path().join("identities");
    fs::create_dir_all(&identities_dir).unwrap();

    let person = Identity::generate().unwrap();
    let _person_did = substrate::derive_did_key(&person.public_key());
    person.save_to_path(identities_dir.join("alice.key")).unwrap();

    // 1. Login
    let login_cmd = roymctl::commands::session::SessionCommands::Login {
        gateway_url: gateway_url.clone(),
        registry_url: Some(reg_url),
        expires_hours: 24,
    };
    roymctl::commands::session::handle(&login_cmd, temp_dir.path(), Some("alice")).await.unwrap();

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

    // 5. Status after logout -> fails
    let err =
        roymctl::commands::session::handle(&status_cmd, temp_dir.path(), None).await.unwrap_err();
    assert!(err.to_string().contains("no active session"), "{err}");

    ctx.teardown().await;
}
