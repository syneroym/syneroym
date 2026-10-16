//! Integration tests for substrate home relay passport registration,
//! renewal, denial, and bootstrapping.

#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::{path::PathBuf, time::Duration};

use common::{
    SubstrateNode,
    passport::{PassportRelay, owner_dir, reaches},
};
use iroh::RelayUrl;
use roymctl::commands::substrate::home_relay::{SetArgs, build_set_request};
use rustls::crypto::ring;
use syneroym_identity::{Identity, substrate};
use syneroym_passport::{
    CapabilityToken, PassportId,
    wire::{HomeRelaySetRequest, HomeRelayStatus},
};
use syneroym_rpc::{JsonRpcError, PERMISSION_DENIED_CODE};
use tempfile::TempDir;
use tokio::time;

fn write_passport_file(passport: &CapabilityToken) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir for passport");
    let path = dir.path().join("passport.json");
    let json = serde_json::to_string(passport).expect("serialize passport");
    std::fs::write(&path, json).expect("write passport");
    (dir, path)
}

fn write_cert_file(cert_pem: &str) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir for cert");
    let path = dir.path().join("ca.pem");
    std::fs::write(&path, cert_pem).expect("write cert");
    (dir, path)
}

async fn boot_managed_node(owner: &Identity, relay: &PassportRelay) -> SubstrateNode {
    SubstrateNode::builder()
        .owner(owner)
        .shared_registry(relay.open.registry_url())
        .shared_relay(relay.relay_url())
        .configure(|c| c.roles.coordinator = None)
        .skip_ready_wait()
        .boot()
        .await
}

async fn admit_node(
    relay: &PassportRelay,
    node: &SubstrateNode,
    owner: &Identity,
    passport: &CapabilityToken,
) -> HomeRelayStatus {
    let (_p_dir, p_path) = write_passport_file(passport);
    let (_c_dir, c_path) = write_cert_file(&relay.cert_pem);
    let o_dir = owner_dir(owner);

    let set_args = SetArgs {
        info_url: relay.info_url.clone(),
        passport: p_path,
        relay_url: None,
        ca_cert: Some(c_path),
    };
    let request = build_set_request(&set_args, node.did(), o_dir.path(), Some("owner"))
        .expect("build set request");

    let client = node.direct_client_as(Identity::from_bytes(&owner.to_bytes())).await;
    client.home_relay_set(&request).await.expect("home_relay_set")
}

async fn assert_leases_contains(relay: &PassportRelay, node_did: &str) {
    let mut controller_client =
        relay.relay.client_as(Identity::from_bytes(&relay.controller.to_bytes()));
    controller_client.connect().await.expect("connect controller");
    let leases = controller_client.relay_passport_leases().await.expect("leases");
    assert!(leases.iter().any(|l| l.subject == node_did), "leases did not contain {node_did}");
    controller_client.shutdown().await.ok();
}

async fn wait_for_status(
    client: &syneroym_sdk::SyneroymClient,
    predicate: impl Fn(&HomeRelayStatus) -> bool,
    timeout: Duration,
) -> HomeRelayStatus {
    let deadline = time::Instant::now() + timeout;
    while time::Instant::now() < deadline {
        if let Ok(Some(status)) = client.home_relay_status().await
            && predicate(&status)
        {
            return status;
        }
        time::sleep(Duration::from_millis(100)).await;
    }
    client.home_relay_status().await.expect("query status").expect("status present")
}

#[tokio::test]
async fn a_node_with_a_passport_is_admitted_without_restart() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let relay = PassportRelay::boot().await;
    let owner = Identity::generate().expect("owner");
    let owner_did = substrate::derive_did_key(&owner.public_key());
    let passport = relay.issue(&owner_did, 86_400);

    let b = boot_managed_node(&owner, &relay).await;
    let peer = relay.leased_peer(&passport, &owner).await;
    let relay_url: RelayUrl = relay.relay_url().parse().expect("parse relay url");

    assert!(!reaches(&peer, &b, relay_url.clone(), Duration::from_secs(5)).await);

    let status = admit_node(&relay, &b, &owner, &passport).await;
    assert!(status.active);
    assert!(status.lease_ends_at.is_some());
    assert!(status.last_error.is_none());

    assert_leases_contains(&relay, b.did()).await;
    assert!(reaches(&peer, &b, relay_url, Duration::from_secs(30)).await);

    let client = b.direct_client_as(Identity::from_bytes(&owner.to_bytes())).await;
    let status_now = client.home_relay_status().await.expect("status").expect("some");
    assert_eq!(status_now.lease_ends_at, status.lease_ends_at);

    b.teardown().await;
    relay.teardown().await;
}

#[tokio::test]
async fn a_restarted_node_registers_before_it_connects() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let relay = PassportRelay::boot().await;
    let owner = Identity::generate().expect("owner");
    let owner_did = substrate::derive_did_key(&owner.public_key());
    let passport = relay.issue(&owner_did, 86_400);

    let b = boot_managed_node(&owner, &relay).await;
    let peer = relay.leased_peer(&passport, &owner).await;
    let relay_url: RelayUrl = relay.relay_url().parse().expect("parse relay url");

    let status1 = admit_node(&relay, &b, &owner, &passport).await;
    assert!(reaches(&peer, &b, relay_url.clone(), Duration::from_secs(30)).await);

    time::sleep(Duration::from_secs(2)).await;
    let restart_secs = syneroym_passport::now_secs();
    let b = b.restart().await;

    let client = b.direct_client_as(Identity::from_bytes(&owner.to_bytes())).await;
    let status2 = wait_for_status(
        &client,
        |s| s.active && s.last_attempt_at.is_some_and(|t| t >= restart_secs),
        Duration::from_secs(10),
    )
    .await;
    assert!(status2.active);
    assert!(status2.last_attempt_at.unwrap_or(0) >= restart_secs);
    assert!(status2.lease_ends_at.unwrap_or(0) >= status1.lease_ends_at.unwrap_or(0));

    assert!(reaches(&peer, &b, relay_url, Duration::from_secs(30)).await);

    b.teardown().await;
    relay.teardown().await;
}

#[tokio::test]
async fn a_restarted_relay_keeps_the_lease() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let mut relay = PassportRelay::boot().await;
    let owner = Identity::generate().expect("owner");
    let owner_did = substrate::derive_did_key(&owner.public_key());
    let passport = relay.issue(&owner_did, 86_400);

    let b = boot_managed_node(&owner, &relay).await;
    let _status = admit_node(&relay, &b, &owner, &passport).await;
    let relay_url: RelayUrl = relay.relay_url().parse().expect("parse relay url");

    relay.restart_relay().await;
    let peer = relay.leased_peer(&passport, &owner).await;

    assert!(reaches(&peer, &b, relay_url, Duration::from_secs(30)).await);

    b.teardown().await;
    relay.teardown().await;
}

#[tokio::test]
async fn a_denied_passport_is_refused_on_the_next_connect() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let mut relay = PassportRelay::boot().await;
    let owner = Identity::generate().expect("owner");
    let owner_did = substrate::derive_did_key(&owner.public_key());
    let passport = relay.issue(&owner_did, 86_400);

    let b = boot_managed_node(&owner, &relay).await;
    let _status = admit_node(&relay, &b, &owner, &passport).await;
    let relay_url: RelayUrl = relay.relay_url().parse().expect("parse relay url");

    // Positive control: peer2 with a valid passport can reach b before the deny
    let owner2 = Identity::generate().expect("owner2");
    let owner2_did = substrate::derive_did_key(&owner2.public_key());
    let passport2 = relay.issue(&owner2_did, 86_400);
    let peer2 = relay.leased_peer(&passport2, &owner2).await;
    assert!(reaches(&peer2, &b, relay_url.clone(), Duration::from_secs(30)).await);
    let _ = peer2.close().await;

    let passport_id = PassportId::of(&passport).expect("passport id");
    let mut controller_client =
        relay.relay.client_as(Identity::from_bytes(&relay.controller.to_bytes()));
    controller_client.connect().await.expect("connect controller");
    controller_client.relay_passport_deny(&passport_id).await.expect("deny passport");
    controller_client.shutdown().await.ok();

    relay.restart_relay().await;

    // Fresh leased peer after restart
    let peer2_after = relay.leased_peer(&passport2, &owner2).await;
    assert!(!reaches(&peer2_after, &b, relay_url, Duration::from_secs(10)).await);
    let _ = peer2_after.close().await;

    let (_p_dir, p_path) = write_passport_file(&passport);
    let (_c_dir, c_path) = write_cert_file(&relay.cert_pem);
    let o_dir = owner_dir(&owner);
    let set_args = SetArgs {
        info_url: relay.info_url.clone(),
        passport: p_path,
        relay_url: None,
        ca_cert: Some(c_path),
    };
    let request = build_set_request(&set_args, b.did(), o_dir.path(), Some("owner"))
        .expect("build set request");
    let client = b.direct_client_as(Identity::from_bytes(&owner.to_bytes())).await;
    let set_status = client.home_relay_set(&request).await.expect("home_relay_set");
    assert_eq!(set_status.last_error.as_ref().map(|e| e.code.as_str()), Some("denied"));
    assert!(set_status.lease_ends_at.is_none());

    b.teardown().await;
    relay.teardown().await;
}

#[tokio::test]
async fn an_expired_token_ends_access_and_a_new_passport_restores_it() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let mut relay = PassportRelay::boot().await;
    let owner = Identity::generate().expect("owner");
    let owner_did = substrate::derive_did_key(&owner.public_key());
    let passport = relay.issue(&owner_did, 86_400);

    let b = boot_managed_node(&owner, &relay).await;
    let now = syneroym_passport::now_secs();
    // 30 s child token life to avoid timing races on slower CI hosts
    let short_child = syneroym_passport::child_token(&owner, &passport, b.did(), 30, now)
        .expect("short child token");

    let set_req = HomeRelaySetRequest {
        token: short_child,
        info_url: relay.info_url.clone(),
        relay_url: None,
        ca_cert_pem: Some(relay.cert_pem.clone()),
    };
    let client = b.direct_client_as(Identity::from_bytes(&owner.to_bytes())).await;
    let status = client.home_relay_set(&set_req).await.expect("home_relay_set");
    assert!(status.active);
    assert!(status.lease_ends_at.is_some());
    // Lease extends to the access end (30s), so next_attempt_at is the wake-up at
    // access_ends_at
    assert_eq!(status.next_attempt_at, Some(now + 30));

    let peer = relay.leased_peer(&passport, &owner).await;
    let relay_url: RelayUrl = relay.relay_url().parse().expect("parse relay url");
    assert!(reaches(&peer, &b, relay_url.clone(), Duration::from_secs(20)).await);
    let _ = peer.close().await;

    // Wait until token expires (now + 30 s)
    while syneroym_passport::now_secs() <= now + 30 {
        time::sleep(Duration::from_millis(500)).await;
    }
    relay.restart_relay().await;

    // Fresh leased peer after relay restart
    let peer_fresh = relay.leased_peer(&passport, &owner).await;
    assert!(!reaches(&peer_fresh, &b, relay_url.clone(), Duration::from_secs(10)).await);
    let _ = peer_fresh.close().await;

    let status_expired = client.home_relay_status().await.expect("status").expect("some");
    assert!(status_expired.lease_ends_at.unwrap_or(0) <= syneroym_passport::now_secs());
    assert_eq!(status_expired.last_error.as_ref().map(|e| e.code.as_str()), Some("expired"));
    let summary = roymctl::commands::substrate::home_relay::summary_line(
        Some(&status_expired),
        syneroym_passport::now_secs(),
    );
    assert!(summary.starts_with("not registered: expired:"), "summary was {summary}");

    let passport_new = relay.issue(&owner_did, 86_400);
    let status_restored = admit_node(&relay, &b, &owner, &passport_new).await;
    assert!(status_restored.active);
    let peer_new = relay.leased_peer(&passport_new, &owner).await;
    let ok = reaches(&peer_new, &b, relay_url, Duration::from_secs(35)).await;
    let _ = peer_new.close().await;
    assert!(ok);

    b.teardown().await;
    relay.teardown().await;
}

#[tokio::test]
async fn a_node_without_a_fixed_relay_refuses_set() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let relay = PassportRelay::boot().await;
    let owner = Identity::generate().expect("owner");
    let owner_did = substrate::derive_did_key(&owner.public_key());
    let passport = relay.issue(&owner_did, 86_400);

    let c = SubstrateNode::builder()
        .owner(&owner)
        .shared_registry(relay.open.registry_url())
        .configure(|cfg| {
            cfg.roles.coordinator = None;
            cfg.parent_coordinator.iroh.url = None;
        })
        .skip_ready_wait()
        .boot()
        .await;

    let (_p_dir, p_path) = write_passport_file(&passport);
    let (_c_dir, c_path) = write_cert_file(&relay.cert_pem);
    let o_dir = owner_dir(&owner);

    let set_args = SetArgs {
        info_url: relay.info_url.clone(),
        passport: p_path,
        relay_url: None,
        ca_cert: Some(c_path),
    };
    let request = build_set_request(&set_args, c.did(), o_dir.path(), Some("owner"))
        .expect("build set request");

    let client = c.direct_client_as(Identity::from_bytes(&owner.to_bytes())).await;
    let err = client.home_relay_set(&request).await.unwrap_err();
    let rpc_err = err.downcast_ref::<JsonRpcError>().expect("JsonRpcError");
    assert_eq!(rpc_err.code, -32601);
    assert!(rpc_err.message.contains("[parent_coordinator.iroh] url"));

    c.teardown().await;
    relay.teardown().await;
}

#[tokio::test]
async fn only_the_owner_may_set_or_read() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let relay = PassportRelay::boot().await;
    let owner = Identity::generate().expect("owner");
    let owner_did = substrate::derive_did_key(&owner.public_key());
    let passport = relay.issue(&owner_did, 86_400);

    let b = boot_managed_node(&owner, &relay).await;
    let (_p_dir, p_path) = write_passport_file(&passport);
    let (_c_dir, c_path) = write_cert_file(&relay.cert_pem);
    let o_dir = owner_dir(&owner);

    let set_args = SetArgs {
        info_url: relay.info_url.clone(),
        passport: p_path,
        relay_url: None,
        ca_cert: Some(c_path),
    };
    let request = build_set_request(&set_args, b.did(), o_dir.path(), Some("owner"))
        .expect("build set request");

    let stranger = Identity::generate().expect("stranger");
    let stranger_client = b.direct_client_as(stranger).await;

    let err_set = stranger_client.home_relay_set(&request).await.unwrap_err();
    let rpc_err_set = err_set.downcast_ref::<JsonRpcError>().expect("JsonRpcError on set");
    assert_eq!(rpc_err_set.code, PERMISSION_DENIED_CODE);

    let err_status = stranger_client.home_relay_status().await.unwrap_err();
    let rpc_err_status = err_status.downcast_ref::<JsonRpcError>().expect("JsonRpcError on status");
    assert_eq!(rpc_err_status.code, PERMISSION_DENIED_CODE);

    b.teardown().await;
    relay.teardown().await;
}

#[tokio::test]
async fn the_bootstrap_order_moves_a_node_onto_a_passport_relay() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let relay = PassportRelay::boot().await;
    let owner = Identity::generate().expect("owner");
    let owner_did = substrate::derive_did_key(&owner.public_key());
    let passport = relay.issue(&owner_did, 86_400);

    let mut d = SubstrateNode::builder()
        .owner(&owner)
        .shared_registry(relay.open.registry_url())
        .shared_relay(relay.open.relay_url())
        .configure(|c| c.roles.coordinator = None)
        .boot()
        .await;

    let (_p_dir, p_path) = write_passport_file(&passport);
    let (_c_dir, c_path) = write_cert_file(&relay.cert_pem);
    let o_dir = owner_dir(&owner);

    let set_args = SetArgs {
        info_url: relay.info_url.clone(),
        passport: p_path,
        relay_url: Some(relay.relay_url().to_string()),
        ca_cert: Some(c_path),
    };

    let status = roymctl::commands::substrate::home_relay::set(
        &set_args,
        d.registry_url(),
        Some(d.did().to_string()),
        o_dir.path(),
        Some("owner"),
        None,
    )
    .await
    .expect("roymctl home_relay set");

    assert!(!status.active);
    assert!(status.lease_ends_at.is_none());

    let mut controller_client =
        relay.relay.client_as(Identity::from_bytes(&relay.controller.to_bytes()));
    controller_client.connect().await.expect("connect controller");
    let leases = controller_client.relay_passport_leases().await.expect("leases");
    assert!(!leases.iter().any(|l| l.subject == d.did()));
    controller_client.shutdown().await.ok();

    let status_check = roymctl::commands::substrate::home_relay::status(
        d.registry_url(),
        Some(d.did().to_string()),
        o_dir.path(),
        Some("owner"),
        None,
    )
    .await
    .expect("roymctl home_relay status")
    .expect("status returned");
    assert_eq!(status_check, status);

    let now = syneroym_passport::now_secs();
    let summary = roymctl::commands::substrate::home_relay::summary_line(Some(&status), now);
    assert!(summary.starts_with("stored, not used:"), "summary was: {summary}");

    let restart_secs = syneroym_passport::now_secs();
    d = d.restart_with(|b| b.shared_relay(relay.relay_url()).skip_ready_wait()).await;

    let client = d.direct_client_as(Identity::from_bytes(&owner.to_bytes())).await;
    let status_after = wait_for_status(
        &client,
        |s| s.active && s.last_attempt_at.is_some_and(|t| t >= restart_secs),
        Duration::from_secs(10),
    )
    .await;
    assert!(status_after.active);
    assert!(status_after.last_attempt_at.unwrap_or(0) >= restart_secs);
    assert!(status_after.lease_ends_at.is_some());

    assert_leases_contains(&relay, d.did()).await;

    let peer = relay.leased_peer(&passport, &owner).await;
    let relay_url: RelayUrl = relay.relay_url().parse().expect("parse relay url");
    assert!(reaches(&peer, &d, relay_url, Duration::from_secs(30)).await);

    d.teardown().await;
    relay.teardown().await;
}
