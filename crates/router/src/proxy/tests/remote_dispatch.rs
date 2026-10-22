use syneroym_rpc::PreambleSignature;

use super::helpers::*;

// -- remote dispatch: retry -------------------------------------------

#[tokio::test]
async fn idempotent_call_retries_transport_failures_up_to_max_attempts() {
    let hop = Arc::new(MockHop::with_outcomes(vec![
        MockOutcome::Transport,
        MockOutcome::Transport,
        MockOutcome::Transport,
    ]));
    let router = test_router(hop.clone(), empty_registry());

    let mut req = base_request("remote-svc", "greet");
    req.idempotent = true;
    let result = router.invoke_remote_at(&synthetic_peer(), &req).await;
    assert!(matches!(result, Err(ProxyError::Transport(_))));
    assert_eq!(hop.call_count(), 3, "must retry up to max_attempts (3) for an idempotent call");
}

#[tokio::test]
async fn non_idempotent_call_never_retries_transport_failures() {
    let hop =
        Arc::new(MockHop::with_outcomes(vec![MockOutcome::Transport, MockOutcome::Transport]));
    let router = test_router(hop.clone(), empty_registry());

    let req = base_request("remote-svc", "greet"); // idempotent: false (default)
    let result = router.invoke_remote_at(&synthetic_peer(), &req).await;
    assert!(matches!(result, Err(ProxyError::Transport(_))));
    assert_eq!(hop.call_count(), 1, "a non-idempotent call must never be retried");
}

#[tokio::test]
async fn callee_error_is_never_retried_even_when_idempotent() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Callee {
        code: -32010,
        message: "denied".to_string(),
    }]));
    let router = test_router(hop.clone(), empty_registry());

    let mut req = base_request("remote-svc", "greet");
    req.idempotent = true;
    let result = router.invoke_remote_at(&synthetic_peer(), &req).await;
    assert!(matches!(result, Err(ProxyError::Callee { code: -32010, .. })));
    assert_eq!(hop.call_count(), 1, "a definitive callee error must never be retried");
}

#[tokio::test]
async fn idempotent_call_stops_retrying_once_it_succeeds() {
    let hop = Arc::new(MockHop::with_outcomes(vec![
        MockOutcome::Transport,
        MockOutcome::Success(Value::String("recovered".to_string())),
    ]));
    let router = test_router(hop.clone(), empty_registry());

    let mut req = base_request("remote-svc", "greet");
    req.idempotent = true;
    let result = router.invoke_remote_at(&synthetic_peer(), &req).await.unwrap();
    assert_eq!(result, Value::String("recovered".to_string()));
    assert_eq!(hop.call_count(), 2);
}

// -- remote dispatch: proof forwarding ---------------------------------

#[tokio::test]
async fn caller_with_proof_presents_node_identity_and_passes_proof_on_outbound_preamble() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let identity = Arc::new(Identity::generate().unwrap());
    let expected_pubkey = hex::encode(identity.public_key().to_bytes());
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        empty_registry(),
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        identity,
        RetryPolicy::default(),
    );

    let mut req = base_request("remote-svc", "greet");
    req.caller.proof = Some(CallerProof {
        pubkey_hex: "deadbeef".to_string(),
        delegation_json: None,
        signed: Some(PreambleSignature {
            service_id: "orig-svc".to_string(),
            interface: "orig-iface".to_string(),
            ts: 1_700_000_000,
            sig_hex: "00".repeat(64),
        }),
        on_behalf: None,
    });
    router.invoke_remote_at(&synthetic_peer(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(preamble.pubkey.as_deref(), Some(expected_pubkey.as_str()));
    assert_eq!(preamble.on_behalf.as_ref().map(|o| o.pubkey.as_str()), Some("deadbeef"));
}

#[tokio::test]
async fn caller_without_proof_presents_the_nodes_own_identity() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let identity = Arc::new(Identity::generate().unwrap());
    let expected_pubkey = hex::encode(identity.public_key().to_bytes());
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        empty_registry(),
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        identity,
        RetryPolicy::default(),
    );

    let req = base_request("remote-svc", "greet"); // caller.proof: None
    router.invoke_remote_at(&synthetic_peer(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(preamble.pubkey.as_deref(), Some(expected_pubkey.as_str()));
}

/// A guest never carries a proof (`CallerContext::service_system`), so
/// unlike the `CallOrigin::Native` case above, a cross-node guest call
/// must not launder itself as the node's own identity -- that would let
/// the destination attribute the call to a real, potentially privileged
/// DID (e.g. its `admin_ucan_root`) with no marker that a guest
/// originated it.
#[tokio::test]
async fn guest_without_proof_forwards_as_anonymous_not_node_identity() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let identity = Arc::new(Identity::generate().unwrap());
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        empty_registry(),
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        identity,
        RetryPolicy::default(),
    );

    let mut req = base_request("remote-svc", "greet"); // caller.proof: None
    req.origin = CallOrigin::Guest { service_id: "guest-component".to_string() };
    router.invoke_remote_at(&synthetic_peer(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(preamble.pubkey, None);
}

/// The self-proxy branch (`host_capabilities.rs`) forwards
/// a `CallOrigin::Guest` request that legitimately carries the real
/// caller's proof when the target is the guest's own service. If that
/// request ever falls through `invoke`'s local-registry lookup (an
/// interface the local `EndpointRegistry` hasn't got, even for the
/// guest's raw own `component_id` -- `check_native_capability_gate`
/// only restricts *native-capability* interfaces cross-service, not
/// this fallback), it must not present that proof, or this node's own
/// identity, to a remote destination the guest fully chose the
/// `(interface, method, params)` for. Same invariant as
/// `guest_without_proof_forwards_as_anonymous_not_node_identity`, now
/// pinned for a guest caller that *does* carry a proof.
#[tokio::test]
async fn guest_with_proof_still_forwards_as_anonymous_not_the_real_proof() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let identity = Arc::new(Identity::generate().unwrap());
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        empty_registry(),
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        identity,
        RetryPolicy::default(),
    );

    let mut req = base_request("guest-component", "some-unregistered-iface");
    req.caller.proof = Some(CallerProof {
        pubkey_hex: "deadbeef".to_string(),
        delegation_json: None,
        ..Default::default()
    });
    req.origin = CallOrigin::Guest { service_id: "guest-component".to_string() };
    router.invoke_remote_at(&synthetic_peer(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(
        preamble.pubkey, None,
        "a guest-origin call must never present a proof (its own or the node's) to a remote \
         destination, even when `req.caller.proof` is `Some` -- otherwise a guest could launder a \
         real caller's identity onto the wire by steering a self-proxy call onto an interface the \
         local registry misses"
    );
}

/// The slice's core claim: a service holding an installed instance
/// certificate makes a guest-origin remote call under its own member
/// master, not anonymous and not the node's identity -- and the
/// destination's handshake (fed the exact preamble this router
/// constructs) resolves that master, matching `HandshakeVerifier`'s
/// contract end to end.
#[tokio::test]
async fn a_guest_call_travels_under_its_services_member_master_not_the_node_identity() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let node_identity = Arc::new(Identity::generate().unwrap());
    let registry = empty_registry();

    let owner_did = "did:key:zMemberOwner".to_string();
    let service_id = "guest-with-cert".to_string();
    registry.set_owner(service_id.clone(), owner_did.clone()).await.unwrap();

    let member_master = Identity::generate().unwrap();
    let member_master_did = substrate::derive_did_key(&member_master.public_key());
    let instance = node_identity.derive_service_identity(&owner_did, &service_id);
    let cert = DelegationCertificate::issue(
        &member_master,
        instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    registry.set_instance_cert(service_id.clone(), cert).await.unwrap();

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        node_identity,
        RetryPolicy::default(),
    );

    let mut req = base_request("remote-svc", "greet");
    req.origin = CallOrigin::Guest { service_id: service_id.clone() };
    router.invoke_remote_at(&synthetic_peer(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(
        preamble.pubkey.as_deref(),
        Some(hex::encode(instance.public_key().to_bytes()).as_str())
    );
    assert!(preamble.delegation.is_some());

    let verified = HandshakeVerifier::verify_preamble(&preamble, &EmptyAnchorResolver)
        .await
        .expect("the destination's handshake must admit a service-instance certificate");
    assert_eq!(verified.master_did, member_master_did);
}

/// The unchanged path (the migration guarantee): a service with no
/// installed certificate presents nothing, exactly like before this arm
/// existed.
#[tokio::test]
async fn a_guest_call_from_a_service_without_a_certificate_is_still_anonymous() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let registry = empty_registry();
    registry.set_owner("no-cert-svc".to_string(), "did:key:zOwner".to_string()).await.unwrap();

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    let mut req = base_request("remote-svc", "greet");
    req.origin = CallOrigin::Guest { service_id: "no-cert-svc".to_string() };
    router.invoke_remote_at(&synthetic_peer(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(preamble.pubkey, None);
    assert!(preamble.delegation.is_none());
}

/// A0-07: an already-expired installed certificate must fall back to
/// anonymous exactly like no certificate at all -- not get attached and
/// then hard-rejected at the destination (`route_handler/io.rs` rejects
/// any connection whose delegation fails to verify), which would turn a
/// missed renewal into an outage for passthrough/relay calls that
/// tolerated anonymous before this certificate mechanism existed.
#[tokio::test]
async fn a_guest_call_from_a_service_with_an_expired_certificate_is_anonymous_not_rejected() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let node_identity = Arc::new(Identity::generate().unwrap());
    let registry = empty_registry();

    let owner_did = "did:key:zMemberOwner".to_string();
    let service_id = "expired-cert-svc".to_string();
    registry.set_owner(service_id.clone(), owner_did.clone()).await.unwrap();

    let member_master = Identity::generate().unwrap();
    let instance = node_identity.derive_service_identity(&owner_did, &service_id);
    let cert = DelegationCertificate::issue(
        &member_master,
        instance.public_key(),
        0,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    assert!(cert.is_expired());
    registry.set_instance_cert(service_id.clone(), cert).await.unwrap();

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        node_identity,
        RetryPolicy::default(),
    );

    let mut req = base_request("remote-svc", "greet");
    req.origin = CallOrigin::Guest { service_id: service_id.clone() };
    router.invoke_remote_at(&synthetic_peer(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(preamble.pubkey, None);
    assert!(preamble.delegation.is_none());
}

/// The transport half: a substrate-internal call made on a deployed
/// service's behalf (the FDAE relationship-proof fetch) presents that
/// service's certified instance key, not the node's -- the same
/// reasoning applied to the guest-origin arm, now at the
/// `(None, Native { service_id: Some(_) })` site.
#[tokio::test]
async fn a_native_origin_call_on_a_services_behalf_presents_that_services_instance_key() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let node_identity = Arc::new(Identity::generate().unwrap());
    let registry = empty_registry();

    let owner_did = "did:key:zMemberOwner".to_string();
    let service_id = "hr-svc".to_string();
    registry.set_owner(service_id.clone(), owner_did.clone()).await.unwrap();

    let member_master = Identity::generate().unwrap();
    let member_master_did = substrate::derive_did_key(&member_master.public_key());
    let instance = node_identity.derive_service_identity(&owner_did, &service_id);
    let cert = DelegationCertificate::issue(
        &member_master,
        instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    registry.set_instance_cert(service_id.clone(), cert).await.unwrap();

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        node_identity,
        RetryPolicy::default(),
    );

    let mut req = base_request("remote-svc", "data-layer");
    req.caller.proof = None;
    req.origin = CallOrigin::Native { service_id: Some(service_id.clone()) };
    router.invoke_remote_at(&synthetic_peer(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(
        preamble.pubkey.as_deref(),
        Some(hex::encode(instance.public_key().to_bytes()).as_str())
    );
    assert!(preamble.delegation.is_some());

    let verified = HandshakeVerifier::verify_preamble(&preamble, &EmptyAnchorResolver)
        .await
        .expect("the destination's handshake must admit a service-instance certificate");
    assert_eq!(verified.master_did, member_master_did);
}

/// When the caller already carries a signed proof, a `Native` call made on a
/// service's behalf presents the service instance identity and passes the
/// caller's proof on in `on_behalf`.
#[tokio::test]
async fn a_native_origin_call_with_a_caller_proof_presents_service_instance_and_passes_proof() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let node_identity = Arc::new(Identity::generate().unwrap());
    let registry = empty_registry();

    let owner_did = "did:key:zMemberOwner".to_string();
    let service_id = "hr-svc".to_string();
    registry.set_owner(service_id.clone(), owner_did.clone()).await.unwrap();
    let member_master = Identity::generate().unwrap();
    let instance = node_identity.derive_service_identity(&owner_did, &service_id);
    let cert = DelegationCertificate::issue(
        &member_master,
        instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    registry.set_instance_cert(service_id.clone(), cert).await.unwrap();

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        node_identity,
        RetryPolicy::default(),
    );

    let forwarded_pubkey_hex = "aabbccdd".to_string();
    let mut req = base_request("remote-svc", "data-layer");
    req.caller.proof = Some(CallerProof {
        pubkey_hex: forwarded_pubkey_hex.clone(),
        delegation_json: None,
        signed: Some(PreambleSignature {
            service_id: "orig-svc".to_string(),
            interface: "orig-iface".to_string(),
            ts: 1_700_000_000,
            sig_hex: "00".repeat(64),
        }),
        on_behalf: None,
    });
    req.origin = CallOrigin::Native { service_id: Some(service_id.clone()) };
    router.invoke_remote_at(&synthetic_peer(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(
        preamble.pubkey.as_deref(),
        Some(hex::encode(instance.public_key().to_bytes()).as_str()),
    );
    assert_eq!(
        preamble.on_behalf.as_ref().map(|o| o.pubkey.as_str()),
        Some(forwarded_pubkey_hex.as_str()),
    );
}

/// A native-origin call made on nobody's behalf (`service_id: None`,
/// substrate-internal tooling, tests) keeps presenting the node's own
/// identity, exactly as before this arm existed.
#[tokio::test]
async fn a_native_origin_call_with_no_service_id_still_presents_the_node_identity() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let node_identity = Arc::new(Identity::generate().unwrap());
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        empty_registry(),
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        node_identity.clone(),
        RetryPolicy::default(),
    );

    let mut req = base_request("remote-svc", "data-layer");
    req.caller.proof = None;
    req.origin = CallOrigin::Native { service_id: None };
    router.invoke_remote_at(&synthetic_peer(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(
        preamble.pubkey.as_deref(),
        Some(hex::encode(node_identity.public_key().to_bytes()).as_str()),
        "a node-level native call (no service_id) must still present the node's own identity"
    );
    assert!(preamble.delegation.is_none());
}

/// The same fallback A0 gave the guest arm: an installed-but-expired
/// certificate must fall back to the node identity, not anonymous -- a
/// substrate-internal call has always presented *something*, and
/// dropping to anonymous would break native-dispatch destinations that
/// reject an anonymous caller outright.
#[tokio::test]
async fn a_native_origin_call_with_an_expired_certificate_falls_back_to_the_node_identity() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let node_identity = Arc::new(Identity::generate().unwrap());
    let registry = empty_registry();

    let owner_did = "did:key:zMemberOwner".to_string();
    let service_id = "expired-cert-native-svc".to_string();
    registry.set_owner(service_id.clone(), owner_did.clone()).await.unwrap();

    let member_master = Identity::generate().unwrap();
    let instance = node_identity.derive_service_identity(&owner_did, &service_id);
    let cert = DelegationCertificate::issue(
        &member_master,
        instance.public_key(),
        0,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    assert!(cert.is_expired());
    registry.set_instance_cert(service_id.clone(), cert).await.unwrap();

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        node_identity.clone(),
        RetryPolicy::default(),
    );

    let mut req = base_request("remote-svc", "data-layer");
    req.caller.proof = None;
    req.origin = CallOrigin::Native { service_id: Some(service_id.clone()) };
    router.invoke_remote_at(&synthetic_peer(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(
        preamble.pubkey.as_deref(),
        Some(hex::encode(node_identity.public_key().to_bytes()).as_str()),
        "an expired certificate must fall back to the node identity, not anonymous -- a \
         native-origin call has always presented something"
    );
    assert!(preamble.delegation.is_none());
}

// -- remote dispatch: peer records ---------------------------------------

#[tokio::test]
async fn a_remote_call_keeps_the_record_of_a_peer_with_a_mailbox() {
    use crate::{
        peer_records::PeerRecordStore,
        test_support::{Substrate, registry_serving_only, sleeping_peer_record},
    };

    let record = sleeping_peer_record();
    let url = registry_serving_only(&record, Substrate::Missing).await;
    let store = Arc::new(PeerRecordStore::open_in_memory().unwrap());
    let hop = Arc::new(MockHop::with_outcomes(vec![]));
    let identity = Arc::new(Identity::generate().unwrap());
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        empty_registry(),
        Arc::new(RegistryClient::new(false, Some(url))),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        identity,
        RetryPolicy::default(),
    )
    .with_peer_records(store.clone());

    let req = base_request(&record.info.service_id, "greet");
    let result = router.invoke_remote(&req).await;

    // The peer has a mailbox but no address, so it cannot be called directly.
    assert!(matches!(result, Err(ProxyError::ServiceNotFound(_))));
    assert_eq!(hop.call_count(), 0);
    let kept = store.recall(&record.info.service_id).await.unwrap();
    assert_eq!(kept.pkarr_packet_hex, record.pkarr_packet_hex);
}
