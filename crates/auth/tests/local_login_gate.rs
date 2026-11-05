#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use serde_json::{Value, json};
use syneroym_auth::{AUTH_METHOD_LOCAL, AuthService, LoginResponse};
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::{AuthLevel, CallerContext, NativeHttpService};
use tempfile::tempdir;

mod common;

use common::{MockAnchorResolver, login_request, node_caller, service_with_identities};

fn local_body() -> Value {
    json!({"method": "local", "identity": "alice"})
}

fn json_headers() -> Vec<(String, String)> {
    vec![("content-type".to_string(), "application/json".to_string())]
}

async fn login(
    service: &AuthService,
    headers: Vec<(String, String)>,
    caller: Option<CallerContext>,
) -> (u16, String) {
    let resp = service.handle_request(login_request(headers, &local_body()), caller).await.unwrap();
    (resp.status, String::from_utf8(resp.body).unwrap())
}

fn assert_refused_naming_delegated_key(answer: &(u16, String), status: u16) {
    assert_eq!(answer.0, status, "body: {}", answer.1);
    assert!(answer.1.contains("delegated-key"), "body: {}", answer.1);
}

#[tokio::test]
async fn local_login_on_an_own_gateway_stream_gets_a_token() {
    let dir = tempdir().unwrap();
    let (service, alice_did) = service_with_identities(dir.path());

    let resp = service
        .handle_request(login_request(json_headers(), &local_body()), Some(node_caller(&service)))
        .await
        .unwrap();

    assert_eq!(resp.status, 200);
    let grant: LoginResponse = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(grant.person_did, alice_did);
}

#[tokio::test]
async fn local_login_with_no_caller_is_refused_and_names_delegated_key() {
    let dir = tempdir().unwrap();
    let (service, _) = service_with_identities(dir.path());

    let answer = login(&service, json_headers(), None).await;

    assert_refused_naming_delegated_key(&answer, 403);
}

#[tokio::test]
async fn local_login_from_another_nodes_key_is_refused() {
    let dir = tempdir().unwrap();
    let (service, _) = service_with_identities(dir.path());
    // A `fixed`-mode gateway stream carries the fixed person as caller, so it
    // looks the same as a stream from another node: a DID that is not ours.
    let person_did = substrate::derive_did_key(&Identity::generate().unwrap().public_key());
    let other = CallerContext::verified(&person_did, AuthLevel::Delegated, vec![]);

    let answer = login(&service, json_headers(), Some(other)).await;

    assert_refused_naming_delegated_key(&answer, 403);
}

#[tokio::test]
async fn local_login_through_a_proxy_is_refused() {
    let dir = tempdir().unwrap();
    let (service, _) = service_with_identities(dir.path());

    for header in ["X-Forwarded-For", "forwarded", "X-Real-IP", "Via"] {
        let mut headers = json_headers();
        headers.push((header.to_string(), "203.0.113.9".to_string()));
        let answer = login(&service, headers, Some(node_caller(&service))).await;
        assert_refused_naming_delegated_key(&answer, 403);
    }
}

#[tokio::test]
async fn remote_access_turns_local_login_off_and_hides_it_from_methods() {
    let dir = tempdir().unwrap();
    let (service, _) = service_with_identities(dir.path());
    let service = service.with_remote_access(true);

    let answer = login(&service, json_headers(), Some(node_caller(&service))).await;

    assert_refused_naming_delegated_key(&answer, 403);
    assert!(answer.1.contains("remote access"), "body: {}", answer.1);
    assert!(!service.methods().methods.contains(&AUTH_METHOD_LOCAL.to_string()));
    assert_eq!(service.methods().methods, ["delegated-key"]);
}

#[tokio::test]
async fn a_node_with_no_identities_dir_still_answers_400() {
    let node_did = substrate::derive_did_key(&Identity::generate().unwrap().public_key());
    let resolver = Arc::new(MockAnchorResolver { resolvable: true, revoked: vec![] });
    let service =
        AuthService::new(Identity::generate().unwrap(), node_did, 3600, 60, None, resolver);

    let answer = login(&service, json_headers(), Some(node_caller(&service))).await;

    assert_refused_naming_delegated_key(&answer, 400);
}

#[tokio::test]
async fn the_origin_refusal_comes_before_the_caller_check() {
    let dir = tempdir().unwrap();
    let (service, _) = service_with_identities(dir.path());
    let mut headers = json_headers();
    headers.push(("origin".to_string(), "http://hub.localhost:7960".to_string()));

    let answer = login(&service, headers, None).await;

    assert_refused_naming_delegated_key(&answer, 403);
    assert!(answer.1.contains("browser origin"), "body: {}", answer.1);
}
