#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration tests for coordinator relay passport admission, registration,
//! enforcement, and lifecycle.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::passport::{
    Fixture, accepting_node, echo_through_relay, goes_online, node_endpoint, passport_coordinator,
    passport_coordinator_with_relay, register, wait_for_info,
};
use iroh::SecretKey;
use serde_json::Map;
use syneroym_core::{
    config::CoordinatorAccess,
    coordinator_info::{CoordinatorInfo, INFO_PATH},
    host_identity::HostIdentity,
};
use syneroym_identity::{
    Identity,
    substrate::{self, SubstrateIdentityStatus},
};
use syneroym_passport::{
    RELAY_HOME,
    wire::{CHALLENGE_PATH, ChallengeResponse, ErrorBody, REGISTER_PATH, RegisterRequest},
};

#[tokio::test]
async fn registered_node_is_admitted_without_restart() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let fixture = Fixture::new();
    let host_did = substrate::derive_did_key(&fixture.host_key.public_key());
    let host = HostIdentity::unowned(host_did.clone());

    let (mut coord, client) =
        passport_coordinator(temp_dir.path(), CoordinatorAccess::Passport, &host).await?;
    common::wait_for_relay(&coord, Duration::from_secs(20)).await?;

    let relay_url = common::relay_url(&coord).expect("relay url");
    let host_id = SecretKey::from_bytes(&fixture.host_key.to_bytes()).public();
    let host_router = accepting_node(&fixture.host_key, relay_url.clone()).await?;

    let node_a = Identity::generate()?;

    // Before registering, node A cannot connect through the relay.
    let ep = node_endpoint(&node_a, relay_url.clone()).await?;
    assert!(!echo_through_relay(&ep, host_id, relay_url.clone(), Duration::from_secs(3)).await);
    let _ = ep.close().await;

    // Register node A.
    let passport = fixture.passport(Map::new(), 86_400);
    let child = fixture.child(&passport, &node_a, 86_400);
    let reg_res = register(&client, coord.info_addr().unwrap(), &host_did, &node_a, &child)
        .await?
        .expect("registration should succeed");

    let now = syneroym_passport::now_secs();
    assert!(reg_res.lease_ends_at <= now + 86_400);
    assert!(reg_res.lease_ends_at <= child.expires_at_secs);

    // After registration, a new endpoint connects through the relay immediately.
    let ep2 = node_endpoint(&node_a, relay_url.clone()).await?;
    assert!(echo_through_relay(&ep2, host_id, relay_url, Duration::from_secs(5)).await);
    let _ = ep2.close().await;

    drop(host_router);
    coord.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn max_nodes_limits_distinct_nodes() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let fixture = Fixture::new();
    let host_did = substrate::derive_did_key(&fixture.host_key.public_key());
    let host = HostIdentity::unowned(host_did.clone());

    let (mut coord, client) =
        passport_coordinator(temp_dir.path(), CoordinatorAccess::Passport, &host).await?;
    common::wait_for_relay(&coord, Duration::from_secs(20)).await?;

    let mut limits = Map::new();
    limits.insert("max_nodes".to_string(), 1.into());
    let passport = fixture.passport(limits, 86_400);

    let node_a = Identity::generate()?;
    let child_a = fixture.child(&passport, &node_a, 86_400);
    let reg_a = register(&client, coord.info_addr().unwrap(), &host_did, &node_a, &child_a).await?;
    assert!(reg_a.is_ok(), "node A should register successfully");

    // Second distinct node with max_nodes = 1 is refused.
    let node_b = Identity::generate()?;
    let child_b = fixture.child(&passport, &node_b, 86_400);
    let reg_b = register(&client, coord.info_addr().unwrap(), &host_did, &node_b, &child_b).await?;
    let (status, body) = reg_b.expect_err("node B should be refused");
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body.code, "limit_reached");

    // Re-registering node A (renewal) succeeds.
    let child_a_renew = fixture.child(&passport, &node_a, 86_400);
    let reg_a_renew =
        register(&client, coord.info_addr().unwrap(), &host_did, &node_a, &child_a_renew).await?;
    assert!(reg_a_renew.is_ok(), "node A renewal should succeed");

    coord.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn open_relay_refuses_registration() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let fixture = Fixture::new();
    let host_did = substrate::derive_did_key(&fixture.host_key.public_key());
    let host = HostIdentity::unowned(host_did);

    // 1. Everyone access
    let (mut coord_everyone, client_everyone) =
        passport_coordinator(temp_dir.path(), CoordinatorAccess::Everyone, &host).await?;
    let ch_url = format!("https://{}{CHALLENGE_PATH}", coord_everyone.info_addr().unwrap());
    let res = client_everyone.post(&ch_url).send().await?;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let body: ErrorBody = res.json().await?;
    assert_eq!(body.code, "not_accepted");
    coord_everyone.shutdown().await?;

    // 2. List access
    let temp_dir2 = tempfile::tempdir()?;
    let allowed_id = SecretKey::generate(&mut rand::rng()).public().to_string();
    let (mut coord_list, client_list) =
        passport_coordinator(temp_dir2.path(), CoordinatorAccess::List(vec![allowed_id]), &host)
            .await?;
    let ch_url2 = format!("https://{}{CHALLENGE_PATH}", coord_list.info_addr().unwrap());
    let res2 = client_list.post(&ch_url2).send().await?;
    assert_eq!(res2.status(), StatusCode::FORBIDDEN);
    let body2: ErrorBody = res2.json().await?;
    assert_eq!(body2.code, "not_accepted");
    coord_list.shutdown().await?;

    Ok(())
}

#[tokio::test]
async fn request_checks_come_before_signatures() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let fixture = Fixture::new();
    let host_did = substrate::derive_did_key(&fixture.host_key.public_key());
    let host = HostIdentity::unowned(host_did.clone());

    let (mut coord, client) =
        passport_coordinator(temp_dir.path(), CoordinatorAccess::Passport, &host).await?;
    let reg_url = format!("https://{}{REGISTER_PATH}", coord.info_addr().unwrap());
    let ch_url = format!("https://{}{CHALLENGE_PATH}", coord.info_addr().unwrap());

    // 1. 17 KiB body is 413 request_too_large
    let large_body = vec![b'a'; 17 * 1024];
    let res = client.post(&reg_url).body(large_body).send().await?;
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body: ErrorBody = res.json().await?;
    assert_eq!(body.code, "request_too_large");

    // 2. Empty JSON object is 400 malformed
    let res = client.post(&reg_url).body(b"{}".to_vec()).send().await?;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body: ErrorBody = res.json().await?;
    assert_eq!(body.code, "malformed");

    // 3. Unknown challenge is 400 unknown_challenge
    let node = Identity::generate()?;
    let passport = fixture.passport(Map::new(), 86_400);
    let child = fixture.child(&passport, &node, 86_400);
    let fake_challenge = [42u8; 32];
    let fake_challenge_hex = hex::encode(fake_challenge);
    let req = RegisterRequest {
        token: child.clone(),
        challenge: fake_challenge_hex,
        signer_did: substrate::derive_did_key(&node.public_key()),
        signature: hex::encode([0u8; 64]),
    };
    let res = client.post(&reg_url).body(serde_json::to_vec(&req)?).send().await?;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body: ErrorBody = res.json().await?;
    assert_eq!(body.code, "unknown_challenge");

    // 4. Real challenge with wrong signature is 403 bad_signature
    let ch_res: ChallengeResponse = client.post(&ch_url).send().await?.json().await?;
    let wrong_sig_req = RegisterRequest {
        token: child,
        challenge: ch_res.challenge,
        signer_did: substrate::derive_did_key(&node.public_key()),
        signature: hex::encode([1u8; 64]),
    };
    let res = client.post(&reg_url).body(serde_json::to_vec(&wrong_sig_req)?).send().await?;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let body: ErrorBody = res.json().await?;
    assert_eq!(body.code, "bad_signature");

    coord.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn eleventh_request_in_a_minute_is_refused() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let fixture = Fixture::new();
    let host_did = substrate::derive_did_key(&fixture.host_key.public_key());
    let host = HostIdentity::unowned(host_did);

    let (mut coord, client) =
        passport_coordinator(temp_dir.path(), CoordinatorAccess::Passport, &host).await?;
    let ch_url = format!("https://{}{CHALLENGE_PATH}", coord.info_addr().unwrap());

    // 10 challenge requests succeed.
    for i in 0..10 {
        let res = client.post(&ch_url).send().await?;
        assert_eq!(res.status(), StatusCode::OK, "request {i} should succeed");
    }

    // 11th request is 429 rate_limited.
    let res = client.post(&ch_url).send().await?;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    let body: ErrorBody = res.json().await?;
    assert_eq!(body.code, "rate_limited");

    coord.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn leases_survive_a_restart() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let fixture = Fixture::new();
    let host_did = substrate::derive_did_key(&fixture.host_key.public_key());
    let host = HostIdentity::unowned(host_did.clone());

    let (mut coord, client) =
        passport_coordinator(temp_dir.path(), CoordinatorAccess::Passport, &host).await?;
    common::wait_for_relay(&coord, Duration::from_secs(20)).await?;

    let node = Identity::generate()?;
    let passport = fixture.passport(Map::new(), 86_400);
    let child = fixture.child(&passport, &node, 86_400);
    let reg_res = register(&client, coord.info_addr().unwrap(), &host_did, &node, &child).await?;
    assert!(reg_res.is_ok(), "registration before restart should succeed");

    coord.shutdown().await?;
    drop(coord);

    // Restart coordinator on the same directory and host.
    let (mut coord2, _client2) =
        passport_coordinator(temp_dir.path(), CoordinatorAccess::Passport, &host).await?;
    common::wait_for_relay(&coord2, Duration::from_secs(20)).await?;

    let relay_url = common::relay_url(&coord2).expect("relay url");
    let ep = node_endpoint(&node, relay_url).await?;
    assert!(
        goes_online(&ep, Duration::from_secs(10)).await,
        "node endpoint should go online after coordinator restart without new registration"
    );
    let _ = ep.close().await;

    coord2.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn denied_passport_is_refused_at_next_connect() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let fixture = Fixture::new();
    let host_did = substrate::derive_did_key(&fixture.host_key.public_key());
    let host = HostIdentity::unowned(host_did.clone());

    let (mut coord, client) =
        passport_coordinator(temp_dir.path(), CoordinatorAccess::Passport, &host).await?;
    common::wait_for_relay(&coord, Duration::from_secs(20)).await?;

    let relay_url = common::relay_url(&coord).expect("relay url");
    let passport = fixture.passport(Map::new(), 86_400);
    let passport_id = syneroym_passport::PassportId::of(&passport)?;

    let node_a = Identity::generate()?;
    let child_a = fixture.child(&passport, &node_a, 86_400);
    register(&client, coord.info_addr().unwrap(), &host_did, &node_a, &child_a).await?.unwrap();

    let node_b = Identity::generate()?;
    let child_b = fixture.child(&passport, &node_b, 86_400);
    register(&client, coord.info_addr().unwrap(), &host_did, &node_b, &child_b).await?.unwrap();

    // Node A comes online and keeps its router open.
    let router_a = accepting_node(&node_a, relay_url.clone()).await?;
    let node_a_id = SecretKey::from_bytes(&node_a.to_bytes()).public();

    // Host node endpoint connects through relay to A.
    let host_ep = node_endpoint(&fixture.host_key, relay_url.clone()).await?;
    assert!(goes_online(&host_ep, Duration::from_secs(10)).await);
    assert!(
        echo_through_relay(&host_ep, node_a_id, relay_url.clone(), Duration::from_secs(5)).await
    );

    // Record open relay client count before deny (coordinator, node A, host
    // endpoint). Poll until the count reaches 3.
    let info_before = wait_for_info(&client, &coord, Duration::from_secs(10), |info| {
        info.relay.as_ref().and_then(|r| r.clients) == Some(3)
    })
    .await?;
    let clients_before = info_before.relay.and_then(|r| r.clients).unwrap();

    // Deny the passport in the lease store.
    let leases = coord.relay_leases().expect("lease store must be present");
    leases.deny(&passport_id, syneroym_passport::now_secs())?;

    // Node B cannot establish a new connection through the relay.
    let ep_b = node_endpoint(&node_b, relay_url.clone()).await?;
    assert!(!echo_through_relay(&ep_b, node_a_id, relay_url.clone(), Duration::from_secs(3)).await);
    let _ = ep_b.close().await;

    // Re-registration is refused with 403 denied.
    let child_b_new = fixture.child(&passport, &node_b, 86_400);
    let reg_denied =
        register(&client, coord.info_addr().unwrap(), &host_did, &node_b, &child_b_new).await?;
    let (status, body) = reg_denied.expect_err("registration should be denied");
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body.code, "denied");

    // Node A's open connection is not cut: echo still succeeds.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(echo_through_relay(&host_ep, node_a_id, relay_url, Duration::from_secs(5)).await);

    // After 2 s, /v1/info relay clients still counts the open connection.
    let _ = wait_for_info(&client, &coord, Duration::from_secs(10), |info| {
        info.relay.as_ref().and_then(|r| r.clients) == Some(clients_before)
    })
    .await?;

    let _ = host_ep.close().await;
    drop(router_a);
    coord.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn ended_lease_is_refused_at_next_connect() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let fixture = Fixture::new();
    let host_did = substrate::derive_did_key(&fixture.host_key.public_key());
    let host = HostIdentity::unowned(host_did.clone());

    let (mut coord, client) =
        passport_coordinator(temp_dir.path(), CoordinatorAccess::Passport, &host).await?;
    common::wait_for_relay(&coord, Duration::from_secs(20)).await?;

    let relay_url = common::relay_url(&coord).expect("relay url");
    let passport = fixture.passport(Map::new(), 86_400);

    // Node A child token lives 86_400 seconds so its lease does not expire during
    // test.
    let node_a = Identity::generate()?;
    let child_a = fixture.child(&passport, &node_a, 86_400);
    register(&client, coord.info_addr().unwrap(), &host_did, &node_a, &child_a).await?.unwrap();

    // Node B child token living 5 seconds to test lease expiry.
    let node_b = Identity::generate()?;
    let child_b = fixture.child(&passport, &node_b, 5);
    let resp_b =
        register(&client, coord.info_addr().unwrap(), &host_did, &node_b, &child_b).await?.unwrap();

    // Node A comes online and keeps its connection open.
    let router_a = accepting_node(&node_a, relay_url.clone()).await?;
    assert!(goes_online(router_a.endpoint(), Duration::from_secs(10)).await);
    let node_a_id = SecretKey::from_bytes(&node_a.to_bytes()).public();

    let host_ep = node_endpoint(&fixture.host_key, relay_url.clone()).await?;
    assert!(goes_online(&host_ep, Duration::from_secs(10)).await);
    assert!(
        echo_through_relay(&host_ep, node_a_id, relay_url.clone(), Duration::from_secs(5)).await
    );

    // Wait until B's lease expires: sleep until lease_ends_at + 1 on the wall
    // clock.
    let now = syneroym_passport::now_secs();
    if resp_b.lease_ends_at >= now {
        let wait_secs = resp_b.lease_ends_at - now + 1;
        tokio::time::sleep(Duration::from_secs(wait_secs)).await;
    }

    // Node B cannot establish a new connection through the relay.
    let ep_b = node_endpoint(&node_b, relay_url.clone()).await?;
    assert!(!echo_through_relay(&ep_b, node_a_id, relay_url.clone(), Duration::from_secs(3)).await);
    let _ = ep_b.close().await;

    // Node A's open connection still carries traffic.
    assert!(echo_through_relay(&host_ep, node_a_id, relay_url, Duration::from_secs(5)).await);

    let _ = host_ep.close().await;
    drop(router_a);
    coord.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn own_endpoint_and_host_node_are_admitted() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let fixture = Fixture::new();
    let host_did = substrate::derive_did_key(&fixture.host_key.public_key());
    let host = HostIdentity::unowned(host_did);

    let (mut coord, _client) =
        passport_coordinator(temp_dir.path(), CoordinatorAccess::Passport, &host).await?;

    // Own endpoint connects to relay without error.
    common::wait_for_relay(&coord, Duration::from_secs(20)).await?;

    let relay_url = common::relay_url(&coord).expect("relay url");
    // Host node endpoint goes online without any lease.
    let host_ep = node_endpoint(&fixture.host_key, relay_url).await?;
    assert!(goes_online(&host_ep, Duration::from_secs(10)).await);
    let _ = host_ep.close().await;

    coord.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn info_reports_the_host_identity() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let fixture = Fixture::new();
    let host_did = substrate::derive_did_key(&fixture.host_key.public_key());
    let host = HostIdentity::unowned(host_did.clone());

    let (mut coord, client) =
        passport_coordinator(temp_dir.path(), CoordinatorAccess::Passport, &host).await?;

    let info_url = format!("https://{}{INFO_PATH}", coord.info_addr().unwrap());
    let info: CoordinatorInfo = client.get(&info_url).send().await?.json().await?;

    let identity = info.identity.expect("identity must be reported");
    assert_eq!(identity.did, host_did);
    assert_eq!(identity.controller_status, "none");

    coord.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn only_the_passport_root_may_issue() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let host_node_key = Identity::generate()?;
    let host_did = substrate::derive_did_key(&host_node_key.public_key());

    let controller = Identity::generate()?;
    let controller_did = substrate::derive_did_key(&controller.public_key());

    let host = HostIdentity {
        node_did: host_did.clone(),
        controller_status: SubstrateIdentityStatus::Verified,
        verified_controller: Some(controller_did.clone()),
    };

    let (mut coord, client) =
        passport_coordinator(temp_dir.path(), CoordinatorAccess::Passport, &host).await?;

    let owner = Identity::generate()?;
    let owner_did = substrate::derive_did_key(&owner.public_key());
    let node = Identity::generate()?;
    let node_did = substrate::derive_did_key(&node.public_key());

    // 1. Passport issued by the verified controller -> Accepted.
    let now = syneroym_passport::now_secs();
    let valid_passport = syneroym_passport::issue_passport(
        &controller,
        &host_did,
        &RELAY_HOME,
        &owner_did,
        Map::new(),
        86_400,
        now,
    )?;
    let child_valid =
        syneroym_passport::child_token(&owner, &valid_passport, &node_did, 86_400, now)?;
    let reg_ok =
        register(&client, coord.info_addr().unwrap(), &host_did, &node, &child_valid).await?;
    assert!(reg_ok.is_ok(), "passport from verified controller must be accepted");

    // 2. Passport issued by the host node key -> Refused with untrusted_root.
    let node2 = Identity::generate()?;
    let node2_did = substrate::derive_did_key(&node2.public_key());
    let bad_root_passport = syneroym_passport::issue_passport(
        &host_node_key,
        &host_did,
        &RELAY_HOME,
        &owner_did,
        Map::new(),
        86_400,
        now,
    )?;
    let child_bad =
        syneroym_passport::child_token(&owner, &bad_root_passport, &node2_did, 86_400, now)?;
    let reg_bad =
        register(&client, coord.info_addr().unwrap(), &host_did, &node2, &child_bad).await?;
    let (status, body) = reg_bad.expect_err("passport from node key must be rejected");
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body.code, "untrusted_root");

    // 3. Passport issued by an unrelated identity -> Refused with untrusted_root.
    let third_party = Identity::generate()?;
    let node3 = Identity::generate()?;
    let node3_did = substrate::derive_did_key(&node3.public_key());
    let third_party_passport = syneroym_passport::issue_passport(
        &third_party,
        &host_did,
        &RELAY_HOME,
        &owner_did,
        Map::new(),
        86_400,
        now,
    )?;
    let child_third =
        syneroym_passport::child_token(&owner, &third_party_passport, &node3_did, 86_400, now)?;
    let reg_third =
        register(&client, coord.info_addr().unwrap(), &host_did, &node3, &child_third).await?;
    let (status, body) = reg_third.expect_err("passport from stranger must be rejected");
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body.code, "untrusted_root");

    coord.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn visitor_without_a_lease_is_refused() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let fixture = Fixture::new();
    let host_did = substrate::derive_did_key(&fixture.host_key.public_key());
    let host = HostIdentity::unowned(host_did.clone());

    let (mut coord, client) =
        passport_coordinator(temp_dir.path(), CoordinatorAccess::Passport, &host).await?;
    common::wait_for_relay(&coord, Duration::from_secs(20)).await?;

    let relay_url = common::relay_url(&coord).expect("relay url");
    let passport = fixture.passport(Map::new(), 86_400);

    let node_a = Identity::generate()?;
    let child_a = fixture.child(&passport, &node_a, 86_400);
    register(&client, coord.info_addr().unwrap(), &host_did, &node_a, &child_a).await?.unwrap();

    let router_a = accepting_node(&node_a, relay_url.clone()).await?;
    assert!(
        goes_online(router_a.endpoint(), Duration::from_secs(10)).await,
        "node A must go online"
    );
    let node_a_id = SecretKey::from_bytes(&node_a.to_bytes()).public();

    // Positive control: host node endpoint (always admitted) echoes node A through
    // the relay.
    let host_ep = node_endpoint(&fixture.host_key, relay_url.clone()).await?;
    assert!(goes_online(&host_ep, Duration::from_secs(10)).await);
    assert!(
        echo_through_relay(&host_ep, node_a_id, relay_url.clone(), Duration::from_secs(5)).await,
        "admitted host endpoint must reach node A through relay"
    );
    let _ = host_ep.close().await;

    // Visitor without lease cannot connect to node A through the relay.
    let visitor = Identity::generate()?;
    let visitor_ep = node_endpoint(&visitor, relay_url.clone()).await?;
    assert!(!echo_through_relay(&visitor_ep, node_a_id, relay_url, Duration::from_secs(3)).await);

    let _ = visitor_ep.close().await;
    drop(router_a);
    coord.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn disabled_relay_refuses_registration() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let fixture = Fixture::new();
    let host_did = substrate::derive_did_key(&fixture.host_key.public_key());
    let host = HostIdentity::unowned(host_did);

    let (mut coord, client) =
        passport_coordinator_with_relay(temp_dir.path(), CoordinatorAccess::Passport, &host, false)
            .await?;

    let ch_url = format!("https://{}{CHALLENGE_PATH}", coord.info_addr().unwrap());
    let reg_url = format!("https://{}{REGISTER_PATH}", coord.info_addr().unwrap());

    let ch_res = client.post(&ch_url).send().await?;
    assert_eq!(ch_res.status(), StatusCode::FORBIDDEN);
    let ch_body: ErrorBody = ch_res.json().await?;
    assert_eq!(ch_body.code, "not_accepted");

    let reg_res = client.post(&reg_url).send().await?;
    assert_eq!(reg_res.status(), StatusCode::FORBIDDEN);
    let reg_body: ErrorBody = reg_res.json().await?;
    assert_eq!(reg_body.code, "not_accepted");

    coord.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn combined_form_admits_both_listed_and_leased_nodes() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let fixture = Fixture::new();
    let host_did = substrate::derive_did_key(&fixture.host_key.public_key());
    let host = HostIdentity::unowned(host_did.clone());

    let listed_node = Identity::generate()?;
    let listed_ep_id = SecretKey::from_bytes(&listed_node.to_bytes()).public();

    let (mut coord, client) = passport_coordinator(
        temp_dir.path(),
        CoordinatorAccess::ListOrPassport(vec![listed_ep_id.to_string()]),
        &host,
    )
    .await?;
    common::wait_for_relay(&coord, Duration::from_secs(20)).await?;
    let relay_url = common::relay_url(&coord).expect("relay url");

    // 1. Listed endpoint goes online without registration.
    let listed_ep = node_endpoint(&listed_node, relay_url.clone()).await?;
    assert!(
        goes_online(&listed_ep, Duration::from_secs(10)).await,
        "listed endpoint must go online"
    );

    // 2. Leased endpoint registers and goes online.
    let leased_node = Identity::generate()?;
    let passport = fixture.passport(Map::new(), 86_400);
    let child = fixture.child(&passport, &leased_node, 86_400);
    register(&client, coord.info_addr().unwrap(), &host_did, &leased_node, &child).await?.unwrap();

    let router_leased = accepting_node(&leased_node, relay_url.clone()).await?;
    assert!(
        goes_online(router_leased.endpoint(), Duration::from_secs(10)).await,
        "leased endpoint must go online"
    );
    let leased_ep_id = SecretKey::from_bytes(&leased_node.to_bytes()).public();

    // 3. Listed endpoint echoes leased endpoint through the relay.
    assert!(
        echo_through_relay(&listed_ep, leased_ep_id, relay_url, Duration::from_secs(5)).await,
        "listed endpoint must reach leased endpoint through relay"
    );

    let _ = listed_ep.close().await;
    drop(router_leased);
    coord.shutdown().await?;
    Ok(())
}
