use axum::http;
use iroh::RelayUrl;
use syneroym_identity::Identity;
use syneroym_passport::{
    MAILBOX_USE, PassportId, child_token, issue_passport,
    wire::{ErrorBody, HomeRelaySetRequest, RegisterResponse},
};

use super::{
    FIXED_RELAY, INFO_URL, START_SECS, StoredPassport,
    client::{Failure, parse_response},
    did_of, test_fixture,
};
use crate::runtime::home_relay::passport::store::{load, save};

#[test]
fn save_then_load_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("passport.json");
    let fixture = test_fixture(30 * 24 * 3600);
    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child,
        ca_cert_pem: Some("PEM CERT".into()),
    };
    save(&path, &stored).unwrap();
    let loaded = load(&path).expect("loaded");
    assert_eq!(loaded, stored);
}

#[cfg(unix)]
#[test]
fn saved_file_has_mode_0600() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("passport.json");
    let fixture = test_fixture(30 * 24 * 3600);
    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child,
        ca_cert_pem: None,
    };
    save(&path, &stored).unwrap();
    let meta = std::fs::metadata(&path).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
}

#[test]
fn from_request_refuses_invalid_inputs() {
    let fixture = test_fixture(30 * 24 * 3600);
    let fixed_url: RelayUrl = FIXED_RELAY.parse().unwrap();
    let node_did = fixture.node_did.clone();

    // 1. http:// info URL
    let req = HomeRelaySetRequest {
        token: fixture.child.clone(),
        info_url: "http://relay.example:7974".into(),
        relay_url: None,
        ca_cert_pem: None,
    };
    let err = StoredPassport::from_request(req, &node_did, &fixed_url, START_SECS).unwrap_err();
    assert!(err.to_string().contains("info_url"), "{err:?}");

    // 2. Query in info URL
    let req = HomeRelaySetRequest {
        token: fixture.child.clone(),
        info_url: "https://relay.example:7974?query=1".into(),
        relay_url: None,
        ca_cert_pem: None,
    };
    let err = StoredPassport::from_request(req, &node_did, &fixed_url, START_SECS).unwrap_err();
    assert!(err.to_string().contains("info_url"), "{err:?}");

    // 3. Bad relay URL
    let req = HomeRelaySetRequest {
        token: fixture.child.clone(),
        info_url: INFO_URL.into(),
        relay_url: Some("not a url".into()),
        ca_cert_pem: None,
    };
    let err = StoredPassport::from_request(req, &node_did, &fixed_url, START_SECS).unwrap_err();
    assert!(err.to_string().contains("relay_url"), "{err:?}");

    // 4. CA cert that is not PEM
    let req = HomeRelaySetRequest {
        token: fixture.child.clone(),
        info_url: INFO_URL.into(),
        relay_url: None,
        ca_cert_pem: Some("not pem content".into()),
    };
    let err = StoredPassport::from_request(req, &node_did, &fixed_url, START_SECS).unwrap_err();
    assert!(err.to_string().contains("ca_cert_pem"), "{err:?}");

    // 4b. CA cert with valid header but invalid body
    let req = HomeRelaySetRequest {
        token: fixture.child.clone(),
        info_url: INFO_URL.into(),
        relay_url: None,
        ca_cert_pem: Some(
            "-----BEGIN CERTIFICATE-----\nnot valid base64!!!\n-----END CERTIFICATE-----".into(),
        ),
    };
    let err = StoredPassport::from_request(req, &node_did, &fixed_url, START_SECS).unwrap_err();
    assert!(err.to_string().contains("ca_cert_pem"), "{err:?}");

    // 5. CA cert over 64 KiB
    let big_cert = "A".repeat(65 * 1024);
    let req = HomeRelaySetRequest {
        token: fixture.child.clone(),
        info_url: INFO_URL.into(),
        relay_url: None,
        ca_cert_pem: Some(big_cert),
    };
    let err = StoredPassport::from_request(req, &node_did, &fixed_url, START_SECS).unwrap_err();
    assert!(err.to_string().contains("ca_cert_pem"), "{err:?}");

    // 6. Token for another node
    let stranger = Identity::generate().unwrap();
    let stranger_did = did_of(&stranger);
    let req = HomeRelaySetRequest {
        token: fixture.child.clone(),
        info_url: INFO_URL.into(),
        relay_url: None,
        ca_cert_pem: None,
    };
    let err = StoredPassport::from_request(req, &stranger_did, &fixed_url, START_SECS).unwrap_err();
    assert!(err.to_string().contains("not_bound"), "{err:?}");

    // 7. Expired token
    let req = HomeRelaySetRequest {
        token: fixture.child.clone(),
        info_url: INFO_URL.into(),
        relay_url: None,
        ca_cert_pem: None,
    };
    let err = StoredPassport::from_request(req, &node_did, &fixed_url, START_SECS + 31 * 24 * 3600)
        .unwrap_err();
    assert!(err.to_string().contains("expired"), "{err:?}");

    // 8. Mailbox token instead of relay
    let mailbox_passport = issue_passport(
        &fixture.signer,
        &fixture.host_did,
        &MAILBOX_USE,
        &fixture.owner_did,
        serde_json::Map::new(),
        3600,
        START_SECS,
    )
    .unwrap();
    let mailbox_child =
        child_token(&fixture.owner, &mailbox_passport, &node_did, 3600, START_SECS).unwrap();
    let req = HomeRelaySetRequest {
        token: mailbox_child,
        info_url: INFO_URL.into(),
        relay_url: None,
        ca_cert_pem: None,
    };
    let err = StoredPassport::from_request(req, &node_did, &fixed_url, START_SECS).unwrap_err();
    assert!(err.to_string().contains("malformed"), "{err:?}");
}

#[test]
fn relay_url_defaults_to_the_fixed_url() {
    let fixture = test_fixture(30 * 24 * 3600);
    let fixed_url: RelayUrl = FIXED_RELAY.parse().unwrap();
    let req = HomeRelaySetRequest {
        token: fixture.child,
        info_url: INFO_URL.into(),
        relay_url: None,
        ca_cert_pem: None,
    };
    let (stored, _) =
        StoredPassport::from_request(req, &fixture.node_did, &fixed_url, START_SECS).unwrap();
    assert_eq!(stored.relay_url, "http://127.0.0.1:7964");
}

#[test]
fn is_for_ignores_a_trailing_slash() {
    let fixture = test_fixture(30 * 24 * 3600);
    let stored = StoredPassport {
        relay_url: "http://127.0.0.1:7964".into(),
        info_url: INFO_URL.into(),
        token: fixture.child,
        ca_cert_pem: None,
    };
    let with_slash: RelayUrl = "http://127.0.0.1:7964/".parse().unwrap();
    assert!(stored.is_for(&with_slash));
}

#[tokio::test]
async fn response_mapping_handles_all_kinds() {
    // 200 + valid JSON -> Ok
    let pid = PassportId::of(&test_fixture(100).passport).unwrap();
    let resp_body =
        serde_json::to_vec(&RegisterResponse { passport_id: pid.clone(), lease_ends_at: 12345 })
            .unwrap();
    let http_resp =
        http::Response::builder().status(200).body(reqwest::Body::from(resp_body)).unwrap();
    let parsed: RegisterResponse =
        parse_response(reqwest::Response::from(http_resp)).await.unwrap();
    assert_eq!(parsed.lease_ends_at, 12345);

    // 403 + ErrorBody -> Refused
    let err_body =
        serde_json::to_vec(&ErrorBody { code: "denied".into(), message: "passport denied".into() })
            .unwrap();
    let http_resp =
        http::Response::builder().status(403).body(reqwest::Body::from(err_body)).unwrap();
    let err =
        parse_response::<RegisterResponse>(reqwest::Response::from(http_resp)).await.unwrap_err();
    assert_eq!(
        err,
        Failure::Refused(ErrorBody { code: "denied".into(), message: "passport denied".into() })
    );

    // 502 + HTML -> Unreachable
    let http_resp = http::Response::builder()
        .status(502)
        .body(reqwest::Body::from("<html>Bad Gateway</html>"))
        .unwrap();
    let err =
        parse_response::<RegisterResponse>(reqwest::Response::from(http_resp)).await.unwrap_err();
    assert_eq!(err, Failure::Unreachable("HTTP 502 Bad Gateway".into()));

    // Body over 64 KiB -> Unreachable
    let big = vec![0u8; 65 * 1024];
    let http_resp = http::Response::builder().status(200).body(reqwest::Body::from(big)).unwrap();
    let err =
        parse_response::<RegisterResponse>(reqwest::Response::from(http_resp)).await.unwrap_err();
    assert!(matches!(err, Failure::Unreachable(_)));
}
