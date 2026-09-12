use std::fs;

use axum::http::StatusCode;
use syneroym_core::{
    config::{AccessControl, ServiceRegistryRole, SubstrateConfig},
    dht_registry::{
        EndpointInfo, EndpointMechanism, EndpointType, MASTER_ANCHOR_SCHEMA_V1,
        MasterAnchorPayload, RegistryClient,
    },
    endpoint_publisher::EndpointPublisher,
    util,
};
use syneroym_identity::{Identity, substrate};

use super::*;

fn create_signed_info(identity: &Identity, info: EndpointInfo) -> SignedEndpointInfo {
    info.sign(identity).unwrap()
}

fn far_future() -> u64 {
    u64::MAX / 2
}

fn sample_service_info_for(service_id: &str) -> EndpointInfo {
    EndpointInfo {
        service_id: service_id.to_string(),
        substrate_id: "did:key:zSubstrate".to_string(),
        endpoint_type: EndpointType::Service,
        nickname: None,
        mechanisms: vec![],
        is_private: false,
        ttl: None,
        not_after: far_future(),
        generation: 0,
    }
}

async fn spawn_registry() -> (EcosystemRegistry, String) {
    spawn_registry_with_parent(None).await
}

async fn spawn_registry_with_parent(
    parent_registry_url: Option<String>,
) -> (EcosystemRegistry, String) {
    let config = SubstrateConfig {
        roles: syneroym_core::config::RolesConfig {
            community_registry: Some(ServiceRegistryRole {
                access: AccessControl::String("everyone".to_string()),
                http_bind_address: "127.0.0.1:0".to_string(),
                parent_registry_url,
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut registry = EcosystemRegistry::init(&config).await.unwrap();
    let url = registry.bind().await.unwrap();
    registry.spawn().await.unwrap();
    (registry, url)
}

#[tokio::test]
async fn test_master_anchor_register_and_lookup() {
    let state = Arc::new(RegistryState::default());
    let identity = Identity::generate().unwrap();
    let master_id = substrate::derive_did_key(&identity.public_key());

    let _temp_identity = Identity::generate().unwrap();

    let payload = MasterAnchorPayload {
        revoked_keys: vec!["did:key:revoked".to_string()],
        timestamp: 1690000000,
        ..Default::default()
    };

    let signed_anchor = payload.sign(&identity).unwrap();

    // Register
    let reg_res = register_master_endpoint(State(state.clone()), Json(signed_anchor.clone())).await;
    assert!(reg_res.is_ok());

    // Lookup
    let lookup_res = lookup_master_endpoint(Path(master_id.clone()), State(state)).await;
    assert!(lookup_res.is_ok());
    let Json(retrieved) = lookup_res.unwrap();
    assert_eq!(retrieved.master_id, master_id);
    assert_eq!(retrieved.payload.schema, MASTER_ANCHOR_SCHEMA_V1);
    assert_eq!(retrieved.payload.revoked_keys.len(), 1);
}

#[tokio::test]
async fn test_register_and_lookup_success() {
    let state = Arc::new(RegistryState::default());
    let identity = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&identity.public_key());

    let info = EndpointInfo {
        service_id: did.clone(),
        substrate_id: did.clone(),
        endpoint_type: EndpointType::Substrate,
        nickname: Some("alice".to_string()),
        mechanisms: vec![EndpointMechanism::Iroh {
            endpoint_addr_bytes: vec![1, 2, 3],
            relay_url: Some("http://relay.example.com".to_string()),
        }],
        is_private: false,
        ttl: None,
        not_after: far_future(),
        generation: 0,
    };

    let signed_info = create_signed_info(&identity, info);

    // Register
    let res = register_endpoint(State(state.clone()), Json(signed_info.clone())).await;
    assert_eq!(res.unwrap(), StatusCode::OK);

    // Lookup by alias
    let alias = util::generate_alias(Some("alice"), &did);
    let lookup_res = lookup_endpoint(Path(alias), State(state)).await;

    let Json(retrieved) = lookup_res.unwrap();
    assert_eq!(retrieved.info.service_id, signed_info.info.service_id);
}

#[tokio::test]
async fn test_register_invalid_signature() {
    let state = Arc::new(RegistryState::default());
    let identity = Identity::generate().unwrap();
    let other_identity = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&identity.public_key());

    let info = EndpointInfo {
        service_id: did.clone(),
        substrate_id: did.clone(),
        endpoint_type: EndpointType::Substrate,
        nickname: None,
        mechanisms: vec![],
        is_private: false,
        ttl: None,
        not_after: far_future(),
        generation: 0,
    };

    // Sign with OTHER identity
    let signed_info = create_signed_info(&other_identity, info);

    let res = register_endpoint(State(state), Json(signed_info)).await;
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_register_invalid_did() {
    let state = Arc::new(RegistryState::default());
    let identity = Identity::generate().unwrap();

    let info = EndpointInfo {
        service_id: "invalid-did".to_string(),
        substrate_id: "invalid-did".to_string(),
        endpoint_type: EndpointType::Substrate,
        nickname: None,
        mechanisms: vec![],
        is_private: false,
        ttl: None,
        not_after: far_future(),
        generation: 0,
    };

    let signed_info = create_signed_info(&identity, info);

    let res = register_endpoint(State(state), Json(signed_info)).await;
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_indirect_lookup() {
    let state = Arc::new(RegistryState::default());
    let substrate_id = "did:key:hsubstrate";
    let service_id = "did:key:hservice";

    // Mock a substrate record
    let substrate_info = SignedEndpointInfo {
        info: EndpointInfo {
            service_id: substrate_id.to_string(),
            substrate_id: substrate_id.to_string(),
            endpoint_type: EndpointType::Substrate,
            nickname: None,
            mechanisms: vec![EndpointMechanism::Iroh {
                endpoint_addr_bytes: vec![42],
                relay_url: None,
            }],
            is_private: false,
            ttl: None,
            not_after: far_future(),
            generation: 0,
        },
        pkarr_packet_hex: "mock-hex".to_string(),
    };
    state.endpoints.insert(substrate_id.to_string(), (substrate_info.clone(), Instant::now(), 0));

    // Mock a service record pointing to that substrate
    let service_info = SignedEndpointInfo {
        info: EndpointInfo {
            service_id: service_id.to_string(),
            substrate_id: substrate_id.to_string(),
            endpoint_type: EndpointType::Service,
            nickname: None,
            mechanisms: vec![],
            is_private: false,
            ttl: None,
            not_after: far_future(),
            generation: 0,
        },
        pkarr_packet_hex: "mock-hex".to_string(),
    };
    state.endpoints.insert(service_id.to_string(), (service_info, Instant::now(), 0));

    // Lookup service
    let lookup_res = lookup_endpoint(Path(service_id.to_string()), State(state.clone())).await;

    let Json(retrieved) = lookup_res.unwrap();
    assert_eq!(retrieved.info.service_id, service_id);
    // Ensure mechanisms are NOT populated since we removed server-side resolution
    assert!(retrieved.info.mechanisms.is_empty());
}

#[tokio::test]
async fn test_lookup_by_shorthash_no_nickname() {
    let state = Arc::new(RegistryState::default());
    let identity = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&identity.public_key());

    let info = EndpointInfo {
        service_id: did.clone(),
        substrate_id: did.clone(),
        endpoint_type: EndpointType::Substrate,
        nickname: None, // No nickname
        mechanisms: vec![],
        is_private: false,
        ttl: None,
        not_after: far_future(),
        generation: 0,
    };

    let signed_info = create_signed_info(&identity, info);
    register_endpoint(State(state.clone()), Json(signed_info)).await.unwrap();

    // Lookup by bare shorthash should work
    let alias = util::generate_alias(None, &did);
    let lookup_res = lookup_endpoint(Path(alias), State(state)).await;

    assert!(lookup_res.is_ok());
    let Json(retrieved) = lookup_res.unwrap();
    assert_eq!(retrieved.info.service_id, did);
}

#[tokio::test]
async fn test_lookup_by_shorthash_fails_if_nickname_present() {
    let state = Arc::new(RegistryState::default());
    let identity = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&identity.public_key());

    let info = EndpointInfo {
        service_id: did.clone(),
        substrate_id: did.clone(),
        endpoint_type: EndpointType::Substrate,
        nickname: Some("alice".to_string()),
        mechanisms: vec![],
        is_private: false,
        ttl: None,
        not_after: far_future(),
        generation: 0,
    };

    let signed_info = create_signed_info(&identity, info);
    register_endpoint(State(state.clone()), Json(signed_info)).await.unwrap();

    // Lookup by bare shorthash should FAIL because a nickname was provided
    let alias = util::generate_alias(None, &did);
    let lookup_res = lookup_endpoint(Path(alias), State(state)).await;

    assert!(lookup_res.is_err());
    assert_eq!(lookup_res.unwrap_err(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_lookup_not_found() {
    let state = Arc::new(RegistryState::default());
    let res = lookup_endpoint(Path("non-existent".to_string()), State(state)).await;

    assert!(res.is_err());
    assert_eq!(res.unwrap_err(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_master_signed_endpoint_record_registers_and_looks_up_under_its_own_did() {
    // The one keying shape this design has: a member's endpoint record
    // is signed by the deployer's own member master key,
    // self-consistently, exactly like any other self-signed record --
    // there is no longer a separate "signed by a delegated instance
    // key, keyed by a different master DID" shape to exercise.
    let state = Arc::new(RegistryState::default());
    let master = Identity::generate().unwrap();
    let master_did = substrate::derive_did_key(&master.public_key());

    let signed = sample_service_info_for(&master_did).sign(&master).unwrap();

    let res = register_endpoint(State(state.clone()), Json(signed.clone())).await;
    assert_eq!(res.unwrap(), StatusCode::OK);

    let lookup_res = lookup_endpoint(Path(master_did.clone()), State(state)).await;
    let Json(retrieved) = lookup_res.unwrap();
    assert_eq!(retrieved.info.service_id, master_did);
}

#[tokio::test]
async fn a_masters_records_alias_is_derived_from_its_own_did() {
    let state = Arc::new(RegistryState::default());
    let master = Identity::generate().unwrap();
    let master_did = substrate::derive_did_key(&master.public_key());

    let mut info = sample_service_info_for(&master_did);
    info.nickname = Some("member-one".to_string());
    let signed = info.sign(&master).unwrap();

    register_endpoint(State(state.clone()), Json(signed)).await.unwrap();

    let alias = util::generate_alias(Some("member-one"), &master_did);
    let lookup_res = lookup_endpoint(Path(alias), State(state)).await;
    let Json(retrieved) = lookup_res.unwrap();
    assert_eq!(retrieved.info.service_id, master_did);
}

#[tokio::test]
async fn refreshing_a_master_anchor_keeps_its_revocations_and_its_revoke_list_registry() {
    let (_registry, url) = spawn_registry().await;
    let client = RegistryClient::new(false, Some(url));
    let master = Identity::generate().unwrap();
    let master_did = substrate::derive_did_key(&master.public_key());

    client
        .publish_master_anchor(
            &master_did,
            vec!["did:key:zRevoked".to_string()],
            Some("https://revocations.example/list".to_string()),
            &master,
            true,
        )
        .await
        .unwrap();

    client.refresh_master_anchor(&master).await.unwrap();

    let refreshed = client.resolve_master_anchor(&master_did, None).await.unwrap();
    assert_eq!(refreshed.revoked_keys, vec!["did:key:zRevoked".to_string()]);
    assert_eq!(
        refreshed.revoke_list_registry,
        Some("https://revocations.example/list".to_string())
    );
}

#[tokio::test]
async fn refreshing_a_stale_master_anchor_keeps_its_revocations() {
    let (registry, url) = spawn_registry().await;
    let client = RegistryClient::new(false, Some(url));
    let master = Identity::generate().unwrap();
    let master_did = substrate::derive_did_key(&master.public_key());

    // Seed the registry directly with a backdated, but genuinely signed,
    // anchor -- the common case a late operator hits.
    let stale_payload = MasterAnchorPayload {
        revoked_keys: vec!["did:key:zRevoked".to_string()],
        ..Default::default()
    };
    let stale_signed = sign_backdated(stale_payload, &master, 25);
    registry.state.master_anchors.insert(master_did.clone(), (stale_signed, Instant::now()));

    client.refresh_master_anchor(&master).await.unwrap();

    let refreshed = client.resolve_master_anchor(&master_did, None).await.unwrap();
    assert_eq!(refreshed.revoked_keys, vec!["did:key:zRevoked".to_string()]);
}

#[tokio::test]
async fn refreshing_refuses_to_overwrite_an_anchor_it_cannot_read() {
    let (registry, url) = spawn_registry().await;
    let client = RegistryClient::new(false, Some(url));
    let master = Identity::generate().unwrap();
    let master_did = substrate::derive_did_key(&master.public_key());

    // A corrupted anchor: signed by a different master than the one it
    // claims (`master_id` mismatches the signing key), so it can never
    // pass `verify_signature`.
    let other = Identity::generate().unwrap();
    let mut corrupt = MasterAnchorPayload::default().sign(&other).unwrap();
    corrupt.master_id = master_did.clone();
    registry.state.master_anchors.insert(master_did.clone(), (corrupt.clone(), Instant::now()));

    let err = client.refresh_master_anchor(&master).await;
    assert!(err.is_err());

    let stored = registry.state.master_anchors.get(&master_did).unwrap();
    assert_eq!(stored.0.pkarr_packet_hex, corrupt.pkarr_packet_hex);
}

/// The `master_id` equality check, against `fetch_own_master_anchor`
/// (reached through `refresh_master_anchor`).
/// Unlike `refreshing_refuses_to_overwrite_an_anchor_it_cannot_read`'s
/// corrupted anchor, this one has a perfectly valid signature -- it is
/// honestly signed by `other`, and only the identity it is served under
/// is wrong, the shape a compromised or buggy registry would produce by
/// answering a lookup for one master with another's anchor.
#[tokio::test]
async fn refresh_refuses_an_anchor_served_under_the_wrong_master() {
    let (registry, url) = spawn_registry().await;
    let client = RegistryClient::new(false, Some(url));
    let requested_master = Identity::generate().unwrap();
    let requested_master_did = substrate::derive_did_key(&requested_master.public_key());

    let other_master = Identity::generate().unwrap();
    let other_anchor = MasterAnchorPayload::default().sign(&other_master).unwrap();
    registry
        .state
        .master_anchors
        .insert(requested_master_did.clone(), (other_anchor, Instant::now()));

    let err = client.refresh_master_anchor(&requested_master).await;
    assert!(err.is_err(), "a validly-signed anchor for a different master must be refused");
}

/// The same `master_id` equality check, against `resolve_master_anchor`
/// -- the consumer-facing read path, not the refresh path above.
#[tokio::test]
async fn resolve_master_anchor_refuses_an_anchor_served_under_the_wrong_master() {
    let (registry, url) = spawn_registry().await;
    let client = RegistryClient::new(false, Some(url));
    let requested_master = Identity::generate().unwrap();
    let requested_master_did = substrate::derive_did_key(&requested_master.public_key());

    let other_master = Identity::generate().unwrap();
    let other_anchor = MasterAnchorPayload::default().sign(&other_master).unwrap();
    registry
        .state
        .master_anchors
        .insert(requested_master_did.clone(), (other_anchor, Instant::now()));

    let err = client.resolve_master_anchor(&requested_master_did, None).await;
    assert!(err.is_err(), "a validly-signed anchor for a different master must be refused");
}

/// The DHT gate must skip only the *DHT* channel for a private-flagged
/// record, not the HTTP
/// registry channel -- which is the whole of what `internal` promises
/// (registered with this substrate's registry only, still reachable
/// through it). `crates/core::dht_registry`'s own gate tests cannot
/// cover this half: `syneroym-core` cannot depend on
/// `syneroym-community-registry` (the dependency runs the other way),
/// so there is no live HTTP registry to register against from there.
#[tokio::test]
async fn an_internal_record_registers_to_a_live_registry_with_the_dht_enabled_and_is_retrievable() {
    let (_registry, url) = spawn_registry().await;
    let client = RegistryClient::new(true, Some(url));

    let identity = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&identity.public_key());
    let mut info = sample_service_info_for(&did);
    info.is_private = true;
    let signed = create_signed_info(&identity, info);

    // `sync_dht: false` backgrounds the real DHT publish (the gate
    // itself, and its skip for `is_private`, is `dht_registry`'s own
    // `test_dht_publication_skipped_for_private_record`) -- this test
    // needs no live DHT network, only that the HTTP registration this
    // gate must NOT block actually succeeds.
    client.register(&signed, false).await.unwrap();

    let looked_up = client.lookup(&did, false).await.unwrap();
    assert_eq!(looked_up.info.service_id, did);
    assert!(looked_up.info.is_private);
}

/// Pairs with the `multi_substrate_placement_e2e.rs` case:
/// the two published tiers stay distinguishable end to end -- `public`
/// resolves *and* its record reaches a parent registry, where `internal`
/// resolves and stops. `admit_endpoint`'s `!payload.info.is_private`
/// parent-relay gate (above) is the whole mechanism; this proves both of
/// its visible outcomes together, since a test only pinning one could
/// pass by accident if the gate's condition were ever inverted.
#[tokio::test]
async fn a_public_record_propagates_to_the_parent_registry_while_an_internal_one_does_not() {
    let (_parent, parent_url) = spawn_registry().await;
    let (_child, child_url) = spawn_registry_with_parent(Some(parent_url.clone())).await;
    let child_client = RegistryClient::new(false, Some(child_url));
    let parent_client = RegistryClient::new(false, Some(parent_url));

    let public_identity = Identity::generate().unwrap();
    let public_did = substrate::derive_did_key(&public_identity.public_key());
    let mut public_info = sample_service_info_for(&public_did);
    public_info.is_private = false;
    child_client.register(&create_signed_info(&public_identity, public_info), false).await.unwrap();

    let internal_identity = Identity::generate().unwrap();
    let internal_did = substrate::derive_did_key(&internal_identity.public_key());
    let mut internal_info = sample_service_info_for(&internal_did);
    internal_info.is_private = true;
    child_client
        .register(&create_signed_info(&internal_identity, internal_info), false)
        .await
        .unwrap();

    // Both records are retrievable through the child registry that
    // admitted them, regardless of visibility -- `internal` still means
    // "registered here", not "registered nowhere".
    assert!(child_client.lookup(&public_did, false).await.is_ok());
    assert!(child_client.lookup(&internal_did, false).await.is_ok());

    // Propagation to the parent is fire-and-forget (`tokio::spawn` in
    // `propagate_registration`), so poll rather than assert immediately.
    let mut public_propagated = false;
    for _ in 0..40 {
        if parent_client.lookup(&public_did, false).await.is_ok() {
            public_propagated = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(public_propagated, "a public record must propagate to the parent registry");

    // The internal record had the same window to propagate as the
    // public one did -- it must never have.
    assert!(
        parent_client.lookup(&internal_did, false).await.is_err(),
        "an internal record must never reach the parent registry"
    );
}

/// The sweep's recovery path, against a live registry -- `build_record`'s
/// own tests (`crates/core/src/endpoint_publisher.rs`) exercise the
/// decision table but never call `publish_all_services` itself, since
/// `register` against no registry is a silent no-op rather than the
/// failure this test needs.
///
/// The failure exercised here is the registry's compare-and-swap: a stored
/// record whose `service_id` a strictly newer record already occupies
/// at the live registry (as if a relocation already published one
/// elsewhere) is a genuine `Err` from `publish_service`, not the benign
/// `Ok(false)` a verification failure would produce. The sweep must
/// survive it and still publish the other stored record.
///
/// **The rejected record's filename is load-bearing and must keep
/// sorting first.** The sweep walks a `BTreeSet`, so ids run in
/// ascending byte order; naming it to sort *last* would let every
/// assertion below hold even under an implementation that aborted on
/// the first error. `aaa-` (0x61) precedes `did:key:` (0x64), so the
/// rejection happens before the other record is reached. The filename
/// is independent of the record's own `info.service_id` --
/// `build_record` looks the file up by the sweep's id, not by what is
/// inside it -- so naming the file for sort order does not change what
/// gets registered.
#[tokio::test]
async fn publish_all_services_survives_a_record_rejected_by_admission() {
    let (_registry, url) = spawn_registry().await;
    let hosted_apps_dir = tempfile::tempdir().unwrap();
    let client = RegistryClient::new(false, Some(url.clone()));

    let identity = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&identity.public_key());

    let stale = sample_service_info_for(&did).sign(&identity).unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;
    let fresh = sample_service_info_for(&did).sign(&identity).unwrap();

    // The fresher record lands first, as if a relocation had already
    // published it elsewhere.
    client.register(&fresh, false).await.unwrap();

    fs::write(
        hosted_apps_dir.path().join("aaa-conflicting.json"),
        serde_json::to_string(&stale).unwrap(),
    )
    .unwrap();

    // A second, unrelated service with a valid stored record -- sorts
    // after the conflicting one, so reaching it proves the sweep
    // continued rather than aborting.
    let other_identity = Identity::generate().unwrap();
    let other_did = substrate::derive_did_key(&other_identity.public_key());
    let other_signed = sample_service_info_for(&other_did).sign(&other_identity).unwrap();
    fs::write(
        hosted_apps_dir.path().join(format!("{other_did}.json")),
        serde_json::to_string(&other_signed).unwrap(),
    )
    .unwrap();

    let publisher = EndpointPublisher::new(
        Arc::new(RegistryClient::new(false, Some(url.clone()))),
        hosted_apps_dir.path().to_path_buf(),
    );

    publisher.publish_all_services().await;

    let looked_up = client.lookup(&did, false).await.unwrap();
    assert_eq!(
        looked_up.pkarr_packet_hex, fresh.pkarr_packet_hex,
        "the stale record must not have overwritten the fresh one"
    );
    assert!(
        client.lookup(&other_did, false).await.is_ok(),
        "the other stored record must still have been published despite the conflict"
    );
}

/// Mirrors `MasterAnchorPayload::sign`, backdated -- kept in this test
/// module too, since
/// `refreshing_a_stale_master_anchor_keeps_its_revocations`
/// needs to seed a registry directly rather than go through a
/// `RegistryClient`.
fn sign_backdated(
    mut payload: MasterAnchorPayload,
    identity: &Identity,
    hours_ago: u64,
) -> SignedMasterAnchor {
    use std::time::{SystemTime, UNIX_EPOCH};

    use pkarr::{
        Keypair, SignedPacket, Timestamp,
        dns::{CLASS, Name, ResourceRecord, rdata::RData},
    };
    use syneroym_core::dht_registry::{PKARR_DNS_NAME, PKARR_TTL};

    let master_id = substrate::derive_did_key(&identity.public_key());
    let keypair = Keypair::from_secret_key(&identity.to_bytes());

    let now_micros = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros() as u64;
    let backdated_micros = now_micros - hours_ago * 60 * 60 * 1_000_000;
    let timestamp = Timestamp::from(backdated_micros);
    payload.timestamp = timestamp.as_u64();

    let json_str = serde_json::to_string(&payload).unwrap();
    let txt_rdata = pkarr::dns::rdata::TXT::try_from(json_str.as_str()).unwrap();
    let name = Name::new(PKARR_DNS_NAME).unwrap();
    let records = vec![ResourceRecord::new(name, CLASS::IN, PKARR_TTL, RData::TXT(txt_rdata))];
    let signed_packet = SignedPacket::new(&keypair, &records, timestamp).unwrap();
    let pkarr_packet_hex = hex::encode(signed_packet.to_relay_payload());
    SignedMasterAnchor { master_id, payload, pkarr_packet_hex }
}
