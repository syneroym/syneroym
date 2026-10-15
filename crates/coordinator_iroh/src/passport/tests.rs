use std::{net::Ipv4Addr, sync::Arc};

use axum::http::Request;
use serde_json::Map;
use syneroym_core::dht_registry::MasterAnchorPayload;
use syneroym_identity::{Identity, substrate::SubstrateIdentityStatus};
use syneroym_passport::{
    Challenge, child_token, issue_passport, sign_challenge,
    wire::{ErrorBody, RegisterRequest},
};
use tower::ServiceExt;

use super::*;

#[derive(Debug)]
struct Revokes(Option<(String, String)>);

#[async_trait::async_trait]
impl MasterAnchorResolver for Revokes {
    async fn resolve_master_anchor(&self, master_id: &str) -> Result<MasterAnchorPayload> {
        let mut anchor = MasterAnchorPayload::default();
        if let Some((issuer, revoked_audience)) = &self.0
            && master_id == issuer
        {
            anchor.revoked_keys = vec![revoked_audience.clone()];
        }
        Ok(anchor)
    }
}

fn make_identities() -> (Identity, String, Identity, String, Identity, String) {
    let operator = Identity::generate().unwrap();
    let op_did = substrate::derive_did_key(&operator.public_key());
    let owner = Identity::generate().unwrap();
    let ow_did = substrate::derive_did_key(&owner.public_key());
    let node = Identity::generate().unwrap();
    let n_did = substrate::derive_did_key(&node.public_key());
    (operator, op_did, owner, ow_did, node, n_did)
}

#[tokio::test]
async fn register_with_valid_body_grants_lease() {
    let temp = tempfile::tempdir().unwrap();
    let (operator, op_did, owner, ow_did, node, n_did) = make_identities();
    let host = HostIdentity::unowned(op_did.clone());
    let resolver = Arc::new(Revokes(None));
    let passports = RelayPassports::open(temp.path(), &host, resolver).await.unwrap();

    let now = syneroym_passport::now_secs();
    let peer = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
    let challenge_resp = passports.challenge(peer, now).unwrap();
    let challenge = Challenge::from_hex(&challenge_resp.challenge).unwrap();

    let passport =
        issue_passport(&operator, &op_did, &RELAY_HOME, &ow_did, Map::new(), 3600, now).unwrap();
    let child = child_token(&owner, &passport, &n_did, 3600, now).unwrap();
    let signature = sign_challenge(&node, &op_did, &RELAY_HOME, &challenge, &child).unwrap();

    let req = RegisterRequest {
        token: child,
        challenge: challenge.to_hex(),
        signer_did: n_did.clone(),
        signature,
    };
    let body = Ok(Bytes::from(serde_json::to_vec(&req).unwrap()));
    let lease = passports.register(peer, body, now).await.unwrap();

    assert_eq!(lease.subject, n_did);
    assert!(passports.leases().is_admitted(&n_did, now));
}

#[tokio::test]
async fn revoked_edges_prevent_registration() {
    let temp = tempfile::tempdir().unwrap();
    let (operator, op_did, owner, ow_did, node, n_did) = make_identities();
    let host = HostIdentity::unowned(op_did.clone());

    // Test revoking owner -> subject edge
    let resolver = Arc::new(Revokes(Some((ow_did.clone(), n_did.clone()))));
    let passports = RelayPassports::open(temp.path(), &host, resolver).await.unwrap();
    let now = syneroym_passport::now_secs();
    let peer = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));
    let challenge_resp = passports.challenge(peer, now).unwrap();
    let challenge = Challenge::from_hex(&challenge_resp.challenge).unwrap();

    let passport =
        issue_passport(&operator, &op_did, &RELAY_HOME, &ow_did, Map::new(), 3600, now).unwrap();
    let child = child_token(&owner, &passport, &n_did, 3600, now).unwrap();
    let signature = sign_challenge(&node, &op_did, &RELAY_HOME, &challenge, &child).unwrap();
    let req = RegisterRequest {
        token: child,
        challenge: challenge.to_hex(),
        signer_did: n_did.clone(),
        signature,
    };
    let body = Ok(Bytes::from(serde_json::to_vec(&req).unwrap()));
    let err = passports.register(peer, body, now).await.unwrap_err();
    assert_eq!(err, RegisterError::Revoked);
    assert!(!passports.leases().is_admitted(&n_did, now));

    // Test revoking operator -> owner edge
    let resolver2 = Arc::new(Revokes(Some((op_did.clone(), ow_did.clone()))));
    let passports2 = RelayPassports::open(temp.path(), &host, resolver2).await.unwrap();
    let challenge_resp2 = passports2.challenge(peer, now).unwrap();
    let challenge2 = Challenge::from_hex(&challenge_resp2.challenge).unwrap();
    let child2 = child_token(&owner, &passport, &n_did, 3600, now).unwrap();
    let signature2 = sign_challenge(&node, &op_did, &RELAY_HOME, &challenge2, &child2).unwrap();
    let req2 = RegisterRequest {
        token: child2,
        challenge: challenge2.to_hex(),
        signer_did: n_did.clone(),
        signature: signature2,
    };
    let body2 = Ok(Bytes::from(serde_json::to_vec(&req2).unwrap()));
    let err2 = passports2.register(peer, body2, now).await.unwrap_err();
    assert_eq!(err2, RegisterError::Revoked);
}

#[tokio::test]
async fn eleventh_call_in_same_second_is_rate_limited() {
    let temp = tempfile::tempdir().unwrap();
    let (_operator, op_did, _owner, _ow_did, _node, _n_did) = make_identities();
    let host = HostIdentity::unowned(op_did);
    let passports =
        RelayPassports::open(temp.path(), &host, Arc::new(Revokes(None))).await.unwrap();

    let now = syneroym_passport::now_secs();
    let peer = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 3));
    for _ in 0..10 {
        assert!(passports.challenge(peer, now).is_ok());
    }
    let err = passports.challenge(peer, now).unwrap_err();
    assert_eq!(err, RegisterError::RateLimited);
}

#[tokio::test]
async fn body_error_maps_length_limit_rejection() {
    let temp = tempfile::tempdir().unwrap();
    let (_operator, op_did, _owner, _ow_did, _node, _n_did) = make_identities();
    let host = HostIdentity::unowned(op_did);
    let passports =
        RelayPassports::open(temp.path(), &host, Arc::new(Revokes(None))).await.unwrap();

    let app = routes(Some(Arc::new(passports)));
    let req = Request::builder()
        .method("POST")
        .uri(wire::REGISTER_PATH)
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 4], 4000))))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(vec![b' '; 17 * 1024]))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let err_body: ErrorBody = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(err_body.code, "request_too_large");
}

#[test]
fn status_of_maps_all_variants_correctly() {
    let cases = [
        (RegisterError::RequestTooLarge { max: 10 }, StatusCode::PAYLOAD_TOO_LARGE),
        (RegisterError::Malformed("m".into()), StatusCode::BAD_REQUEST),
        (RegisterError::UnknownChallenge, StatusCode::BAD_REQUEST),
        (RegisterError::ChallengeExpired, StatusCode::BAD_REQUEST),
        (RegisterError::RateLimited, StatusCode::TOO_MANY_REQUESTS),
        (RegisterError::Store("s".into()), StatusCode::INTERNAL_SERVER_ERROR),
        (RegisterError::BadSignature, StatusCode::FORBIDDEN),
        (RegisterError::NotBound, StatusCode::FORBIDDEN),
        (RegisterError::UntrustedRoot, StatusCode::FORBIDDEN),
        (RegisterError::BrokenChain, StatusCode::FORBIDDEN),
        (
            RegisterError::WrongCapability { ability: "a".into(), resource: "r".into() },
            StatusCode::FORBIDDEN,
        ),
        (RegisterError::Expired, StatusCode::FORBIDDEN),
        (RegisterError::InvalidChain("i".into()), StatusCode::FORBIDDEN),
        (RegisterError::Denied, StatusCode::FORBIDDEN),
        (RegisterError::Revoked, StatusCode::FORBIDDEN),
        (RegisterError::LimitReached { limit: 1 }, StatusCode::FORBIDDEN),
        (RegisterError::NotAccepted, StatusCode::FORBIDDEN),
    ];
    for (err, expected_status) in cases {
        assert_eq!(status_of(&err), expected_status, "failed for {err:?}");
    }
}

#[test]
fn endpoint_did_round_trips_with_endpoint_of_did() {
    let id = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&id.public_key());
    let ep = endpoint_of_did(&did).unwrap();
    assert_eq!(endpoint_did(&ep).as_deref(), Some(did.as_str()));
}

#[tokio::test]
async fn open_with_invalid_controller_did_fails() {
    let temp = tempfile::tempdir().unwrap();
    let host = HostIdentity {
        node_did: "did:key:zNodeValid".to_string(),
        controller_status: SubstrateIdentityStatus::Verified,
        verified_controller: Some("not_a_did".to_string()),
    };
    let res = RelayPassports::open(temp.path(), &host, Arc::new(Revokes(None))).await;
    assert!(res.is_err());
    let err_msg = format!("{:#}", res.unwrap_err());
    assert!(
        err_msg.contains("[roles.coordinator] access"),
        "expected {err_msg} to name [roles.coordinator] access"
    );
}

#[tokio::test]
async fn root_rule_enforcement() {
    let temp = tempfile::tempdir().unwrap();
    let (controller, ctrl_did, owner, ow_did, node, n_did) = make_identities();
    let (node_key, host_node_did, _, _, _, _) = make_identities();

    // 1. Host with verified controller accepts passport from controller, rejects
    //    node key
    let host_verified = HostIdentity {
        node_did: host_node_did.clone(),
        controller_status: SubstrateIdentityStatus::Verified,
        verified_controller: Some(ctrl_did.clone()),
    };
    let passports =
        RelayPassports::open(temp.path(), &host_verified, Arc::new(Revokes(None))).await.unwrap();
    let now = syneroym_passport::now_secs();
    let peer = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 5));

    // Passport issued by controller succeeds
    let p_ctrl =
        issue_passport(&controller, &host_node_did, &RELAY_HOME, &ow_did, Map::new(), 3600, now)
            .unwrap();
    let c_ctrl = child_token(&owner, &p_ctrl, &n_did, 3600, now).unwrap();
    let ch_resp = passports.challenge(peer, now).unwrap();
    let ch = Challenge::from_hex(&ch_resp.challenge).unwrap();
    let sig = sign_challenge(&node, &host_node_did, &RELAY_HOME, &ch, &c_ctrl).unwrap();
    let req = RegisterRequest {
        token: c_ctrl,
        challenge: ch.to_hex(),
        signer_did: n_did.clone(),
        signature: sig,
    };
    assert!(
        passports
            .register(peer, Ok(Bytes::from(serde_json::to_vec(&req).unwrap())), now)
            .await
            .is_ok()
    );

    // Passport issued by node key fails with UntrustedRoot
    let p_node =
        issue_passport(&node_key, &host_node_did, &RELAY_HOME, &ow_did, Map::new(), 3600, now)
            .unwrap();
    let c_node = child_token(&owner, &p_node, &n_did, 3600, now).unwrap();
    let ch_resp2 = passports.challenge(peer, now).unwrap();
    let ch2 = Challenge::from_hex(&ch_resp2.challenge).unwrap();
    let sig2 = sign_challenge(&node, &host_node_did, &RELAY_HOME, &ch2, &c_node).unwrap();
    let req2 = RegisterRequest {
        token: c_node,
        challenge: ch2.to_hex(),
        signer_did: n_did.clone(),
        signature: sig2,
    };
    let err = passports
        .register(peer, Ok(Bytes::from(serde_json::to_vec(&req2).unwrap())), now)
        .await
        .unwrap_err();
    assert_eq!(err, RegisterError::UntrustedRoot);

    // 2. Host with no controller accepts passport from node key
    let temp2 = tempfile::tempdir().unwrap();
    let host_unowned = HostIdentity::unowned(host_node_did.clone());
    let passports2 =
        RelayPassports::open(temp2.path(), &host_unowned, Arc::new(Revokes(None))).await.unwrap();
    let ch_resp3 = passports2.challenge(peer, now).unwrap();
    let ch3 = Challenge::from_hex(&ch_resp3.challenge).unwrap();
    let c_node2 = child_token(&owner, &p_node, &n_did, 3600, now).unwrap();
    let sig3 = sign_challenge(&node, &host_node_did, &RELAY_HOME, &ch3, &c_node2).unwrap();
    let req3 = RegisterRequest {
        token: c_node2,
        challenge: ch3.to_hex(),
        signer_did: n_did.clone(),
        signature: sig3,
    };
    assert!(
        passports2
            .register(peer, Ok(Bytes::from(serde_json::to_vec(&req3).unwrap())), now)
            .await
            .is_ok()
    );
}
