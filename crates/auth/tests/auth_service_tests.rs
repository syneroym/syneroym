#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::json;
use syneroym_app_host::types::http::HttpRequest;
use syneroym_auth::{
    AUTH_METHOD_DELEGATED_KEY, AUTH_METHOD_LOCAL, AuthService, DelegatedKeyLoginParams,
    SessionToken,
};
use syneroym_core::{
    dht_registry::{MasterAnchorPayload, MasterAnchorResolver},
    protocol_utils::gateway_session_assertion,
};
use syneroym_identity::{
    DelegationCertificate, Identity,
    delegation::{SCOPE_ROUTING, SCOPE_SERVICE_INSTANCE, SCOPE_SESSION_AUTH},
    substrate,
};
use syneroym_rpc::NativeHttpService;
use tempfile::tempdir;

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Debug)]
struct MockAnchorResolver {
    resolvable: bool,
    revoked: Vec<String>,
}

#[async_trait::async_trait]
impl MasterAnchorResolver for MockAnchorResolver {
    async fn resolve_master_anchor(&self, master_id: &str) -> anyhow::Result<MasterAnchorPayload> {
        if !self.resolvable {
            return Err(anyhow::anyhow!("DHT anchor lookup failed for {master_id}"));
        }
        Ok(MasterAnchorPayload {
            schema: "master_anchor_v1".to_string(),
            revoked_keys: self.revoked.clone(),
            revoke_list_registry: None,
            timestamp: now_secs(),
        })
    }
}

#[tokio::test]
async fn delegated_key_login_flow_and_refusals() {
    let auth_id = Identity::generate().unwrap();
    let auth_did = substrate::derive_did_key(&auth_id.public_key());
    let node_id = Identity::generate().unwrap();
    let node_did = substrate::derive_did_key(&node_id.public_key());

    let master_id = Identity::generate().unwrap();
    let master_did = substrate::derive_did_key(&master_id.public_key());

    let temp_id = Identity::generate().unwrap();
    let temp_did = substrate::derive_did_key(&temp_id.public_key());

    let resolver = Arc::new(MockAnchorResolver { resolvable: true, revoked: vec![] });

    let service = AuthService::new(auth_id, node_did.clone(), 3600, 60, None, resolver.clone());

    // 1. Issue challenge
    let challenge = service.issue_challenge(None);
    assert_eq!(challenge.node_did, node_did);

    // 2. Mint delegation certificate with session-auth scope
    let delegation = DelegationCertificate::issue(
        &master_id,
        temp_id.public_key(),
        7200,
        SCOPE_SESSION_AUTH.to_string(),
    )
    .unwrap();

    // 3. Sign challenge
    let assertion = gateway_session_assertion(&node_did, &challenge.nonce, &master_did);
    let signature = temp_id.sign_json(&assertion).unwrap();

    // 4. Successful login
    let params = DelegatedKeyLoginParams {
        temp_did: temp_did.clone(),
        delegation: delegation.clone(),
        nonce: challenge.nonce.clone(),
        signature: signature.clone(),
    };
    let res = service.login_delegated_key(&params).await.unwrap();
    assert_eq!(res.person_did, master_did);

    // Verify minted token binds to master DID, not temp DID
    let claims = SessionToken::verify(&res.token, &auth_did).unwrap();
    assert_eq!(claims.person_did, master_did);
    assert_eq!(claims.auth_method(), Some(AUTH_METHOD_DELEGATED_KEY));

    // 5. Replaying used nonce fails
    let err = service.login_delegated_key(&params).await.unwrap_err();
    assert_eq!(err.0, 401);
    assert_eq!(err.1, "unknown or already used nonce");

    // 6. Routing scope refused (Finding 3)
    let ch2a = service.issue_challenge(None);
    let routing_scope_delegation = DelegationCertificate::issue(
        &master_id,
        temp_id.public_key(),
        7200,
        SCOPE_ROUTING.to_string(),
    )
    .unwrap();
    let sig2a =
        temp_id.sign_json(&gateway_session_assertion(&node_did, &ch2a.nonce, &master_did)).unwrap();
    let err = service
        .login_delegated_key(&DelegatedKeyLoginParams {
            temp_did: temp_did.clone(),
            delegation: routing_scope_delegation,
            nonce: ch2a.nonce,
            signature: sig2a,
        })
        .await
        .unwrap_err();
    assert_eq!(err.0, 401);
    assert_eq!(err.1, "invalid delegation certificate");

    // 6b. Service instance scope refused
    let ch2 = service.issue_challenge(None);
    let wrong_scope_delegation = DelegationCertificate::issue(
        &master_id,
        temp_id.public_key(),
        7200,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    let sig2 =
        temp_id.sign_json(&gateway_session_assertion(&node_did, &ch2.nonce, &master_did)).unwrap();
    let err = service
        .login_delegated_key(&DelegatedKeyLoginParams {
            temp_did: temp_did.clone(),
            delegation: wrong_scope_delegation,
            nonce: ch2.nonce,
            signature: sig2,
        })
        .await
        .unwrap_err();
    assert_eq!(err.0, 401);
    assert_eq!(err.1, "invalid delegation certificate");

    // 7. Revoked key refused
    let rev_resolver =
        Arc::new(MockAnchorResolver { resolvable: true, revoked: vec![temp_did.clone()] });
    let auth_id_rev = Identity::generate().unwrap();
    let rev_service = AuthService::new(auth_id_rev, node_did.clone(), 3600, 60, None, rev_resolver);
    let ch3 = rev_service.issue_challenge(None);
    let sig3 =
        temp_id.sign_json(&gateway_session_assertion(&node_did, &ch3.nonce, &master_did)).unwrap();
    let err = rev_service
        .login_delegated_key(&DelegatedKeyLoginParams {
            temp_did: temp_did.clone(),
            delegation: delegation.clone(),
            nonce: ch3.nonce,
            signature: sig3,
        })
        .await
        .unwrap_err();
    assert_eq!(err.0, 401);
    assert_eq!(err.1, "delegated key is in master revoked_keys list");

    // 8. Expired delegation refused
    let ch4 = service.issue_challenge(None);
    let mut expired_delegation = DelegationCertificate::issue(
        &master_id,
        temp_id.public_key(),
        100,
        SCOPE_SESSION_AUTH.to_string(),
    )
    .unwrap();
    expired_delegation.expires_at_secs = now_secs() - 10;
    let sig4 =
        temp_id.sign_json(&gateway_session_assertion(&node_did, &ch4.nonce, &master_did)).unwrap();
    let err = service
        .login_delegated_key(&DelegatedKeyLoginParams {
            temp_did: temp_did.clone(),
            delegation: expired_delegation,
            nonce: ch4.nonce,
            signature: sig4,
        })
        .await
        .unwrap_err();
    assert_eq!(err.0, 401);

    // 9. Bad signature refused
    let ch5 = service.issue_challenge(None);
    let other_temp = Identity::generate().unwrap();
    let bad_sig = other_temp
        .sign_json(&gateway_session_assertion(&node_did, &ch5.nonce, &master_did))
        .unwrap();
    let err = service
        .login_delegated_key(&DelegatedKeyLoginParams {
            temp_did,
            delegation,
            nonce: ch5.nonce,
            signature: bad_sig,
        })
        .await
        .unwrap_err();
    assert_eq!(err.0, 401);
    assert_eq!(err.1, "invalid signature");
}

#[tokio::test]
async fn local_login_and_refusals() {
    let auth_id_bytes = Identity::generate().unwrap().to_bytes();
    let node_id = Identity::generate().unwrap();
    let node_did = substrate::derive_did_key(&node_id.public_key());

    let resolver = Arc::new(MockAnchorResolver { resolvable: true, revoked: vec![] });

    // 1. Without key directory, local login is disabled
    let service_no_local = AuthService::new(
        Identity::from_bytes(&auth_id_bytes),
        node_did.clone(),
        3600,
        60,
        None,
        resolver.clone(),
    );
    let methods = service_no_local.methods();
    assert!(!methods.methods.contains(&AUTH_METHOD_LOCAL.to_string()));
    assert_eq!(service_no_local.login_local("alice").unwrap_err().0, 400);

    // 2. With key directory, local login works
    let temp = tempdir().unwrap();
    let alice_id = Identity::generate().unwrap();
    let alice_did = substrate::derive_did_key(&alice_id.public_key());
    fs::write(temp.path().join("alice.key"), alice_id.to_bytes()).unwrap();

    let service_with_local = AuthService::new(
        Identity::from_bytes(&auth_id_bytes),
        node_did,
        3600,
        60,
        Some(temp.path().to_path_buf()),
        resolver,
    );
    let methods = service_with_local.methods();
    assert!(methods.methods.contains(&AUTH_METHOD_LOCAL.to_string()));

    let grant = service_with_local.login_local("alice").unwrap();
    assert_eq!(grant.person_did, alice_did);

    // 3. Unknown identity fails
    let err = service_with_local.login_local("bob").unwrap_err();
    assert_eq!(err.0, 401);

    // 4. Path traversal attempt refused with 400
    assert_eq!(service_with_local.login_local("../alice").unwrap_err().0, 400);
    assert_eq!(service_with_local.login_local("subdir/alice").unwrap_err().0, 400);
    assert_eq!(service_with_local.login_local("").unwrap_err().0, 400);
}

#[tokio::test]
async fn http_interface_endpoints_and_unknown_method() {
    let auth_id = Identity::generate().unwrap();
    let node_id = Identity::generate().unwrap();
    let node_did = substrate::derive_did_key(&node_id.public_key());

    let resolver = Arc::new(MockAnchorResolver { resolvable: true, revoked: vec![] });

    let temp = tempdir().unwrap();
    let alice_id = Identity::generate().unwrap();
    let alice_did = substrate::derive_did_key(&alice_id.public_key());
    fs::write(temp.path().join("alice.key"), alice_id.to_bytes()).unwrap();

    let service = Arc::new(AuthService::new(
        auth_id,
        node_did,
        3600,
        60,
        Some(temp.path().to_path_buf()),
        resolver,
    ));

    // Methods endpoint
    let req = HttpRequest {
        method: "GET".to_string(),
        path: "/_syneroym/session/methods".to_string(),
        query: "".to_string(),
        route: "/_syneroym/session/methods".to_string(),
        path_params: vec![],
        headers: vec![],
        body: vec![],
        caller: None,
    };
    let resp = service.handle_request(req, None).await.unwrap();
    assert_eq!(resp.status, 200);

    // Unknown method -> 400 (Seam 1)
    let bad_login_req = HttpRequest {
        method: "POST".to_string(),
        path: "/_syneroym/session/login".to_string(),
        query: "".to_string(),
        route: "/_syneroym/session/login".to_string(),
        path_params: vec![],
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body: serde_json::to_vec(&json!({"method": "oauth-unknown"})).unwrap(),
        caller: None,
    };
    let bad_resp = service.handle_request(bad_login_req, None).await.unwrap();
    assert_eq!(bad_resp.status, 400);

    // Missing method -> 400 (Finding 26)
    let missing_method_req = HttpRequest {
        method: "POST".to_string(),
        path: "/_syneroym/session/login".to_string(),
        query: "".to_string(),
        route: "/_syneroym/session/login".to_string(),
        path_params: vec![],
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body: serde_json::to_vec(&json!({"identity": "alice"})).unwrap(),
        caller: None,
    };
    let missing_resp = service.handle_request(missing_method_req, None).await.unwrap();
    assert_eq!(missing_resp.status, 400);

    // Local login via HTTP
    let local_login_req = HttpRequest {
        method: "POST".to_string(),
        path: "/_syneroym/session/login".to_string(),
        query: "".to_string(),
        route: "/_syneroym/session/login".to_string(),
        path_params: vec![],
        headers: vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("origin".to_string(), "http://web.localhost:7960".to_string()),
        ],
        body: serde_json::to_vec(&json!({"method": "local", "identity": "alice"})).unwrap(),
        caller: None,
    };
    let login_resp = service.handle_request(local_login_req, None).await.unwrap();
    assert_eq!(login_resp.status, 200);

    // CORS origin echo (Finding 12)
    let allow_origin =
        login_resp.headers.iter().find(|(k, _)| k == "access-control-allow-origin").unwrap();
    assert_eq!(allow_origin.1, "http://web.localhost:7960");
    let allow_cred =
        login_resp.headers.iter().find(|(k, _)| k == "access-control-allow-credentials").unwrap();
    assert_eq!(allow_cred.1, "true");

    let cookie_header = login_resp.headers.iter().find(|(k, _)| k == "set-cookie").unwrap();
    assert!(cookie_header.1.contains("syneroym_session="));

    let grant: syneroym_auth::LoginResponse = serde_json::from_slice(&login_resp.body).unwrap();
    assert_eq!(grant.person_did, alice_did);

    // Whoami endpoint with cookie reports auth = local (Finding 22)
    let whoami_req = HttpRequest {
        method: "GET".to_string(),
        path: "/_syneroym/session/whoami".to_string(),
        query: "".to_string(),
        route: "/_syneroym/session/whoami".to_string(),
        path_params: vec![],
        headers: vec![("cookie".to_string(), format!("syneroym_session={}", grant.token))],
        body: vec![],
        caller: None,
    };
    let whoami_resp = service.handle_request(whoami_req, None).await.unwrap();
    assert_eq!(whoami_resp.status, 200);
    let whoami_data: syneroym_auth::WhoamiResponse =
        serde_json::from_slice(&whoami_resp.body).unwrap();
    assert_eq!(whoami_data.person_did, alice_did);
    assert_eq!(whoami_data.auth, "local");

    // Whoami endpoint with Authorization Bearer
    let whoami_bearer_req = HttpRequest {
        method: "GET".to_string(),
        path: "/_syneroym/session/whoami".to_string(),
        query: "".to_string(),
        route: "/_syneroym/session/whoami".to_string(),
        path_params: vec![],
        headers: vec![("authorization".to_string(), format!("Bearer {}", grant.token))],
        body: vec![],
        caller: None,
    };
    let whoami_bearer_resp = service.handle_request(whoami_bearer_req, None).await.unwrap();
    assert_eq!(whoami_bearer_resp.status, 200);

    // Refresh endpoint
    let refresh_req = HttpRequest {
        method: "POST".to_string(),
        path: "/_syneroym/session/refresh".to_string(),
        query: "".to_string(),
        route: "/_syneroym/session/refresh".to_string(),
        path_params: vec![],
        headers: vec![("cookie".to_string(), format!("syneroym_session={}", grant.token))],
        body: vec![],
        caller: None,
    };
    let refresh_resp = service.handle_request(refresh_req, None).await.unwrap();
    assert_eq!(refresh_resp.status, 200);

    // Logout endpoint
    let logout_req = HttpRequest {
        method: "POST".to_string(),
        path: "/_syneroym/session/logout".to_string(),
        query: "".to_string(),
        route: "/_syneroym/session/logout".to_string(),
        path_params: vec![],
        headers: vec![("cookie".to_string(), format!("syneroym_session={}", grant.token))],
        body: vec![],
        caller: None,
    };
    let logout_resp = service.handle_request(logout_req, None).await.unwrap();
    assert_eq!(logout_resp.status, 200);
    let logout_cookie = logout_resp.headers.iter().find(|(k, _)| k == "set-cookie").unwrap();
    assert!(logout_cookie.1.contains("Max-Age=0"));

    // After logout, token fails whoami (Finding 1)
    let whoami_after_logout = HttpRequest {
        method: "GET".to_string(),
        path: "/_syneroym/session/whoami".to_string(),
        query: "".to_string(),
        route: "/_syneroym/session/whoami".to_string(),
        path_params: vec![],
        headers: vec![("cookie".to_string(), format!("syneroym_session={}", grant.token))],
        body: vec![],
        caller: None,
    };
    let after_resp = service.handle_request(whoami_after_logout, None).await.unwrap();
    assert_eq!(after_resp.status, 401);
}
