#![allow(
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    dead_code
)]
//! End-to-end tests for remote clients of the client gateway: the `local`
//! login rules and `delegated-key` login on a node with remote access.

use std::{fs, net::Ipv4Addr, path::Path, str, time::Duration};

use reqwest::Client;
use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_core::{
    config::{AuthRole, ClientGatewayRole, IdentityMode, RemoteAccess},
    protocol_utils::AUTH_SERVICE_ALIAS,
};
use syneroym_identity::{Identity, substrate};
use syneroym_sdk::SyneroymClient;
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
    time,
};

mod common;
use common::{
    NodeBuilder, SubstrateNode,
    roym::{RoymNode, roym_artifacts_present},
    serial_guard,
    session::{AUTH_HOST, delegated_login},
};

/// A node in `login` mode whose `local` login reads keys from `ids_dir`.
fn node_builder(ids_dir: &Path, remote_access: bool) -> NodeBuilder {
    let ids_dir = ids_dir.to_path_buf();
    SubstrateNode::builder().configure(move |c| {
        let gateway = c.roles.client_gateway.take().unwrap_or_default();
        c.roles.client_gateway = Some(ClientGatewayRole {
            identity_mode: IdentityMode::Login,
            remote_access: remote_access
                .then(|| RemoteAccess { bind_address: Ipv4Addr::LOCALHOST.into() }),
            ..gateway
        });
        c.roles.auth =
            Some(AuthRole { person_identities_dir: Some(ids_dir.clone()), ..Default::default() });
    })
}

/// A directory holding `alice.key`. Returns Alice's DID too.
fn identities_dir() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let alice = Identity::generate().unwrap();
    fs::write(dir.path().join("alice.key"), alice.to_bytes()).unwrap();
    (dir, substrate::derive_did_key(&alice.public_key()))
}

async fn post_local_login(gateway_url: &str) -> (u16, String) {
    let resp = Client::new()
        .post(format!("{gateway_url}/_syneroym/session/login"))
        .json(&json!({ "method": "local", "identity": "alice" }))
        .send()
        .await
        .unwrap();
    (resp.status().as_u16(), resp.text().await.unwrap())
}

#[tokio::test]
async fn local_login_through_the_gateway_still_works_with_remote_access_off() {
    let _serial = serial_guard().await;
    let _ = ring::default_provider().install_default();
    let (dir, alice_did) = identities_dir();
    let node = node_builder(dir.path(), false).boot().await;

    let (status, body) = post_local_login(&node.gateway_url()).await;

    assert_eq!(status, 200, "body: {body}");
    let grant: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(grant["person_did"], alice_did);
    assert!(grant["token"].as_str().is_some_and(|t| !t.is_empty()));
    node.teardown().await;
}

/// Reads one HTTP response: the status and the body.
async fn read_http_response(stream: &mut TcpStream) -> (u16, String) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = time::timeout(Duration::from_secs(10), stream.read(&mut chunk))
            .await
            .expect("timeout waiting for the auth service answer")
            .unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(head_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = str::from_utf8(&buf[..head_end]).unwrap().to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .map_or(0, |v| v.trim().parse::<usize>().unwrap());
            if buf.len() >= head_end + 4 + length {
                break;
            }
        }
    }
    let text = String::from_utf8(buf).unwrap();
    let status = text.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    (status, body)
}

/// Sends `request` to this node's auth service over Iroh as `caller`, the way
/// the gateway does, and returns the HTTP answer.
async fn http_over_iroh_as(node: &SubstrateNode, caller: Identity, request: &str) -> (u16, String) {
    let client: SyneroymClient = {
        let mut c = node.client_as(Identity::from_bytes(&caller.to_bytes()));
        c.connect().await.unwrap();
        c
    };
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mut browser_side = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
    let (mut gateway_side, _) = listener.accept().await.unwrap();

    let conn = client.connection().unwrap();
    let request = request.to_string();
    let forward = tokio::spawn(async move {
        SyneroymClient::passthrough_with_conn(
            conn,
            AUTH_SERVICE_ALIAS,
            "",
            request.as_bytes(),
            &mut gateway_side,
            &caller,
            None,
        )
        .await
    });
    let answer = read_http_response(&mut browser_side).await;
    forward.abort();
    answer
}

#[tokio::test]
async fn local_login_over_iroh_from_another_identity_is_refused() {
    let _serial = serial_guard().await;
    let _ = ring::default_provider().install_default();
    let (dir, _) = identities_dir();
    let node = node_builder(dir.path(), false).boot().await;
    let body = r#"{"method":"local","identity":"alice"}"#;
    let request = format!(
        "POST /_syneroym/session/login HTTP/1.1\r\nHost: {AUTH_HOST}\r\nContent-Type: \
         application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );

    let (status, answer) = http_over_iroh_as(&node, Identity::generate().unwrap(), &request).await;

    assert_eq!(status, 403, "body: {answer}");
    assert!(answer.contains("delegated-key"), "body: {answer}");
    node.teardown().await;
}

#[tokio::test]
async fn remote_access_turns_local_login_off_and_delegated_key_still_logs_in() {
    let _serial = serial_guard().await;
    let _ = ring::default_provider().install_default();
    let (dir, _) = identities_dir();
    let node = node_builder(dir.path(), true).boot().await;
    let gateway_url = node.gateway_url();

    let (status, body) = post_local_login(&gateway_url).await;
    assert_eq!(status, 403, "body: {body}");
    assert!(body.contains("remote access") && body.contains("delegated-key"), "body: {body}");

    let methods: Value = Client::new()
        .get(format!("{gateway_url}/_syneroym/session/methods"))
        .header("Host", AUTH_HOST)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(methods["methods"], json!(["delegated-key"]));

    let person = Identity::generate().unwrap();
    let (token, person_did) = delegated_login(&gateway_url, &person, node.registry_url(), 1).await;
    let whoami: Value = Client::new()
        .get(format!("{gateway_url}/_syneroym/session/whoami"))
        .header("Host", AUTH_HOST)
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(whoami["person_did"], person_did);
    node.teardown().await;
}

#[tokio::test]
async fn a_roym_method_works_with_a_delegated_session_on_a_remote_access_node() {
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym artifacts are not built; run `mise run build:roym`");
        return;
    }
    let _serial = serial_guard().await;
    let _ = ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let owner = Identity::generate().unwrap();
    let mut node = RoymNode::boot_with_remote_access(
        "remote",
        dir.path().to_path_buf(),
        None,
        owner,
        Default::default(),
    )
    .await;
    node.full_bring_up().await;

    // `profile.get` answers the owner only; a stranger gets -32011. The
    // delegated session makes the owner the caller, like `local` login does.
    node.rpc_ok("profile.get", json!({})).await;
    node.teardown().await;
}

#[tokio::test]
async fn remote_access_in_fixed_mode_stops_startup() {
    let _serial = serial_guard().await;
    let _ = ring::default_provider().install_default();

    let error = SubstrateNode::builder()
        .configure(|c| {
            c.roles.client_gateway = Some(ClientGatewayRole {
                identity_mode: IdentityMode::Fixed,
                remote_access: Some(RemoteAccess { bind_address: Ipv4Addr::LOCALHOST.into() }),
                ..Default::default()
            });
        })
        .init_error()
        .await;

    let text = format!("{error:#}");
    assert!(text.contains("remote_access") && text.contains("fixed"), "error: {text}");
}
