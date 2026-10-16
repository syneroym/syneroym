use std::fs;

use serde_json::Map;
use syneroym_identity::{Identity, substrate};
use syneroym_passport::{
    RELAY_HOME, inspect_child_token, issue_passport,
    wire::{ErrorBody, HomeRelayStatus},
};
use tempfile::tempdir;

use super::{SetArgs, build_set_request, summary_line};

fn make_passport(
    signer: &Identity,
    host_did: &str,
    owner_did: &str,
    life_secs: u64,
    start_secs: u64,
) -> syneroym_passport::CapabilityToken {
    issue_passport(signer, host_did, &RELAY_HOME, owner_did, Map::new(), life_secs, start_secs)
        .unwrap()
}

fn sample_status() -> HomeRelayStatus {
    HomeRelayStatus {
        relay_url: "http://127.0.0.1:7964".into(),
        info_url: "https://relay.example:7974".into(),
        active: true,
        passport_id: "deadbeef".repeat(8).parse().unwrap(),
        host_node_did: "did:key:host".into(),
        owner_did: "did:key:owner".into(),
        access_ends_at: 1_700_100_000,
        lease_ends_at: None,
        last_attempt_at: None,
        last_error: None,
        next_attempt_at: None,
    }
}

#[test]
fn build_set_request_without_as_errors() {
    let dir = tempdir().unwrap();
    let args = SetArgs {
        info_url: "https://relay.example:7974".into(),
        passport: dir.path().join("passport.json"),
        relay_url: None,
        ca_cert: None,
    };
    let err = build_set_request(&args, "did:key:node", dir.path(), None).unwrap_err();
    assert!(err.to_string().contains("--as"));
}

#[test]
fn build_set_request_refuses_passport_for_another_owner() {
    let dir = tempdir().unwrap();
    let id_dir = dir.path().join("identities");
    fs::create_dir_all(&id_dir).unwrap();

    let owner = Identity::generate().unwrap();
    let other_owner = Identity::generate().unwrap();
    let signer = Identity::generate().unwrap();
    let host = Identity::generate().unwrap();

    owner.save_to_path(id_dir.join("alice.key")).unwrap();

    let host_did = substrate::derive_did_key(&host.public_key());
    let other_did = substrate::derive_did_key(&other_owner.public_key());
    let now = syneroym_passport::now_secs();
    let passport = make_passport(&signer, &host_did, &other_did, 86400, now);

    let passport_path = dir.path().join("passport.json");
    fs::write(&passport_path, serde_json::to_string(&passport).unwrap()).unwrap();

    let args = SetArgs {
        info_url: "https://relay.example:7974".into(),
        passport: passport_path,
        relay_url: None,
        ca_cert: None,
    };
    let err = build_set_request(&args, "did:key:node", dir.path(), Some("alice")).unwrap_err();
    assert!(err.to_string().contains("issued to"));
}

#[test]
fn build_set_request_refuses_expired_passport() {
    let dir = tempdir().unwrap();
    let id_dir = dir.path().join("identities");
    fs::create_dir_all(&id_dir).unwrap();

    let owner = Identity::generate().unwrap();
    let signer = Identity::generate().unwrap();
    let host = Identity::generate().unwrap();

    owner.save_to_path(id_dir.join("alice.key")).unwrap();

    let host_did = substrate::derive_did_key(&host.public_key());
    let owner_did = substrate::derive_did_key(&owner.public_key());
    // Passport expired 100 seconds ago
    let start = 1_000_000;
    let passport = make_passport(&signer, &host_did, &owner_did, 10, start);

    let passport_path = dir.path().join("passport.json");
    fs::write(&passport_path, serde_json::to_string(&passport).unwrap()).unwrap();

    let args = SetArgs {
        info_url: "https://relay.example:7974".into(),
        passport: passport_path,
        relay_url: None,
        ca_cert: None,
    };
    let err = build_set_request(&args, "did:key:node", dir.path(), Some("alice")).unwrap_err();
    assert!(err.to_string().contains("expired"));
}

#[test]
fn build_set_request_succeeds_and_carries_ca_cert() {
    let dir = tempdir().unwrap();
    let id_dir = dir.path().join("identities");
    fs::create_dir_all(&id_dir).unwrap();

    let owner = Identity::generate().unwrap();
    let signer = Identity::generate().unwrap();
    let host = Identity::generate().unwrap();
    let node = Identity::generate().unwrap();

    owner.save_to_path(id_dir.join("alice.key")).unwrap();

    let host_did = substrate::derive_did_key(&host.public_key());
    let owner_did = substrate::derive_did_key(&owner.public_key());
    let node_did = substrate::derive_did_key(&node.public_key());
    let now = syneroym_passport::now_secs();
    let passport = make_passport(&signer, &host_did, &owner_did, 86400, now);

    let passport_path = dir.path().join("passport.json");
    fs::write(&passport_path, serde_json::to_string(&passport).unwrap()).unwrap();

    let ca_path = dir.path().join("ca.pem");
    fs::write(&ca_path, "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----").unwrap();

    let args = SetArgs {
        info_url: "https://relay.example:7974".into(),
        passport: passport_path,
        relay_url: Some("http://127.0.0.1:7964".into()),
        ca_cert: Some(ca_path),
    };
    let req = build_set_request(&args, &node_did, dir.path(), Some("alice")).unwrap();
    assert_eq!(req.info_url, "https://relay.example:7974");
    assert_eq!(req.relay_url, Some("http://127.0.0.1:7964".into()));
    assert!(req.ca_cert_pem.as_deref().unwrap().contains("BEGIN CERTIFICATE"));

    let presented = inspect_child_token(&req.token, &RELAY_HOME, &node_did, now).unwrap();
    assert_eq!(presented.owner_did, owner_did);
    assert_eq!(presented.subject, node_did);
}

#[test]
fn summary_line_rules() {
    let now = 1000;

    // Rule 1: None
    assert_eq!(summary_line(None, now), "no home-relay passport is stored");

    // Rule 2: !active
    let mut s = sample_status();
    s.active = false;
    assert_eq!(
        summary_line(Some(&s), now),
        "stored, not used: parent_coordinator.iroh.url is not http://127.0.0.1:7964"
    );

    // Rule 3: lease running + last_error
    let mut s = sample_status();
    s.lease_ends_at = Some(2000);
    s.last_error =
        Some(ErrorBody { code: "rate_limited".into(), message: "too many requests".into() });
    assert_eq!(
        summary_line(Some(&s), now),
        "the lease runs until 2000, but the last registration failed: rate_limited: too many \
         requests"
    );

    // Rule 4: lease running
    let mut s = sample_status();
    s.lease_ends_at = Some(2000);
    s.last_error = None;
    assert_eq!(summary_line(Some(&s), now), "registered; the lease ends at 2000");

    // Rule 5: ended lease + last_error
    let mut s = sample_status();
    s.lease_ends_at = Some(900); // ended
    s.last_error = Some(ErrorBody { code: "denied".into(), message: "access denied".into() });
    assert_eq!(summary_line(Some(&s), now), "not registered: denied: access denied");

    // Rule 6a: not registered yet with next_attempt_at
    let mut s = sample_status();
    s.lease_ends_at = None;
    s.last_error = None;
    s.next_attempt_at = Some(1500);
    assert_eq!(summary_line(Some(&s), now), "not registered yet; next attempt at 1500");

    // Rule 6b: not registered yet without next_attempt_at
    let mut s = sample_status();
    s.lease_ends_at = None;
    s.last_error = None;
    s.next_attempt_at = None;
    assert_eq!(summary_line(Some(&s), now), "no attempt planned");
}
