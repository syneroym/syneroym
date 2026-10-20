use std::sync::{Arc, Weak};

use dashmap::DashMap;
use serde_json::Value;
use syneroym_core::config::RetryPolicy;
use syneroym_identity::{DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE};
use syneroym_rpc::{
    CallOrigin, CallerProof, NativeDispatchRegistry, OnBehalfProof, PreambleSignature,
};

use crate::{
    OutboundIdentity, RoutePreamble,
    proxy::{
        ProxyRouter,
        tests::helpers::{
            MockHop, MockOutcome, base_request, empty_registry, empty_registry_client,
            synthetic_addr, test_router,
        },
    },
};

fn sample_p_proof() -> CallerProof {
    CallerProof {
        pubkey_hex: "deadbeef0011".to_string(),
        delegation_json: None,
        signed: Some(PreambleSignature {
            service_id: "client-svc".to_string(),
            interface: "client-iface".to_string(),
            ts: 1_700_000_000,
            sig_hex: "00".repeat(64),
        }),
        on_behalf: None,
    }
}

#[tokio::test]
async fn native_call_with_proof_and_no_service_id_presents_node_key_and_passes_on_behalf() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let node_identity = Arc::new(Identity::generate().unwrap());
    let expected_pubkey = hex::encode(node_identity.public_key().to_bytes());
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        empty_registry(),
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        node_identity,
        RetryPolicy::default(),
    );

    let mut req = base_request("remote-svc", "data-layer");
    req.caller.proof = Some(sample_p_proof());
    req.origin = CallOrigin::Native { service_id: None };
    router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(preamble.pubkey.as_deref(), Some(expected_pubkey.as_str()));
    assert!(preamble.sig.is_some());
    let on_behalf = preamble.on_behalf.expect("on_behalf must be present");
    assert_eq!(on_behalf.pubkey, "deadbeef0011");
    assert_eq!(on_behalf.service_id, "client-svc");
}

#[tokio::test]
async fn native_call_with_service_id_and_cert_presents_instance_and_passes_on_behalf() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let node_identity = Arc::new(Identity::generate().unwrap());
    let registry = empty_registry();

    let owner_did = "did:key:zMemberOwner".to_string();
    let service_id = "hr-svc".to_string();
    registry.set_owner(service_id.clone(), owner_did.clone()).await.unwrap();
    let member_master = Identity::generate().unwrap();
    let instance = node_identity.derive_service_identity(&owner_did, &service_id);
    let instance_pubkey = hex::encode(instance.public_key().to_bytes());
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
    req.caller.proof = Some(sample_p_proof());
    req.origin = CallOrigin::Native { service_id: Some(service_id) };
    router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(preamble.pubkey.as_deref(), Some(instance_pubkey.as_str()));
    assert!(preamble.delegation.is_some());
    let on_behalf = preamble.on_behalf.expect("on_behalf must be present");
    assert_eq!(on_behalf.pubkey, "deadbeef0011");
}

#[tokio::test]
async fn caller_proof_carrying_on_behalf_passes_carried_on_behalf_unchanged() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let router = test_router(hop.clone(), empty_registry());

    let q_proof = OnBehalfProof {
        pubkey: "q_pubkey".to_string(),
        delegation: None,
        service_id: "q-svc".to_string(),
        interface: "q-iface".to_string(),
        ts: 1_699_999_000,
        sig: "11".repeat(64),
    };

    let mut p_proof = sample_p_proof();
    p_proof.on_behalf = Some(q_proof);

    let mut req = base_request("remote-svc", "greet");
    req.caller.proof = Some(p_proof);
    req.origin = CallOrigin::Native { service_id: None };
    router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    let on_behalf = preamble.on_behalf.expect("on_behalf must be present");
    assert_eq!(on_behalf.pubkey, "q_pubkey");
    assert_eq!(on_behalf.service_id, "q-svc");
}

#[tokio::test]
async fn native_call_with_unsigned_local_proof_sends_no_on_behalf() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let router = test_router(hop.clone(), empty_registry());

    let unsigned = CallerProof {
        pubkey_hex: "local_key".to_string(),
        delegation_json: None,
        signed: None,
        on_behalf: None,
    };

    let mut req = base_request("remote-svc", "greet");
    req.caller.proof = Some(unsigned);
    req.origin = CallOrigin::Native { service_id: None };
    router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(preamble.on_behalf, None);
}

#[tokio::test]
async fn guest_call_with_proof_sends_no_on_behalf() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let router = test_router(hop.clone(), empty_registry());

    let mut req = base_request("remote-svc", "greet");
    req.caller.proof = Some(sample_p_proof());
    req.origin = CallOrigin::Guest { service_id: "guest-component".to_string() };
    router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(preamble.on_behalf, None);
}

#[tokio::test]
async fn retried_idempotent_call_attempts_carry_distinct_timestamps() {
    let hop = Arc::new(MockHop::with_outcomes(vec![
        MockOutcome::Transport,
        MockOutcome::Success(Value::String("done".to_string())),
    ]));
    let router = test_router(hop.clone(), empty_registry());

    let mut req = base_request("remote-svc", "greet");
    req.idempotent = true;
    let res = router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();
    assert_eq!(res, Value::String("done".to_string()));

    let preambles = hop.all_preambles.lock().unwrap().clone();
    assert_eq!(preambles.len(), 2);
    let ts0 = preambles[0].ts.expect("ts0 must be set");
    let ts1 = preambles[1].ts.expect("ts1 must be set");
    assert_ne!(ts0, ts1);
    let sig0 = preambles[0].sig.as_ref().expect("sig0 must be set");
    let sig1 = preambles[1].sig.as_ref().expect("sig1 must be set");
    assert_ne!(sig0, sig1, "signatures must differ across attempts with different timestamps");

    // Also prove OutboundIdentity re-computes distinct signatures on each call:
    let out_id = OutboundIdentity::key_only(Arc::new(Identity::generate().unwrap()));
    let route = RoutePreamble::binary_json_rpc("remote-svc", "greet");
    let p0 = out_id.apply(&route, ts0);
    let p1 = out_id.apply(&route, ts1);
    assert_ne!(p0.sig, p1.sig, "fresh signatures must be generated for each timestamp");
}
