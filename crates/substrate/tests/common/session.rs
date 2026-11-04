//! Delegated-key login through a node's client gateway, as a remote client
//! does it.

use reqwest::Client;
use serde_json::{Value, json};
use syneroym_core::{dht_registry::RegistryClient, protocol_utils::gateway_session_assertion};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SESSION_AUTH, substrate,
};

/// The `Host` header that routes a request to the node's auth service.
pub const AUTH_HOST: &str = "auth.localhost";

/// Logs `person` in with `delegated-key` and returns the session token and
/// the person's DID. Publishes the person's master anchor to `registry_url`
/// first, because the auth service resolves it.
pub async fn delegated_login(
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
        .header("Host", AUTH_HOST)
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
        SCOPE_SESSION_AUTH.to_string(),
    )
    .unwrap();
    let assertion = gateway_session_assertion(node_did, nonce, &person_did);
    let sig = person.sign_json(&assertion).unwrap();

    // 3. Publish Master Anchor
    let reg_client = RegistryClient::new(false, Some(registry_url.to_string()));
    reg_client.refresh_master_anchor(person).await.unwrap();

    // 4. Login
    let login_body = json!({
        "method": "delegated-key",
        "person_did": person_did,
        "nonce": nonce,
        "signature": sig,
        "delegation": cert,
    });
    let login_resp = client
        .post(format!("{gateway_url}/_syneroym/session/login"))
        .header("Host", AUTH_HOST)
        .json(&login_body)
        .send()
        .await
        .unwrap();
    assert_eq!(login_resp.status(), 200);
    let resp_val: Value = login_resp.json().await.unwrap();
    let token = resp_val["token"].as_str().unwrap().to_string();
    (token, person_did)
}
