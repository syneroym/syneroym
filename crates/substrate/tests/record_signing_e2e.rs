#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! End-to-end integration tests for record signing over a live substrate
//! instance (M06C Slice C3).

mod common;
#[path = "common/retry.rs"]
mod retry;

use common::{SubstrateTestContext, alloc_ports};
use syneroym_core::dht_registry::{DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointType};
use syneroym_identity::{Identity, delegation::SCOPE_RECORD_SIGNING, substrate};
use syneroym_sdk::{SyneroymClient, deploy};

#[tokio::test]
async fn record_signing_e2e_service_and_delegated_flow() {
    let [iroh_port, registry_port, gateway_port] = alloc_ports();
    let ctx = SubstrateTestContext::setup(iroh_port, registry_port, gateway_port).await;
    let wasm_bytes = std::fs::read(syneroym_core::test_constants::greeter_wasm_path())
        .expect("Failed to read compiled test WASM component");
    let service_identity = Identity::generate().unwrap();
    let service_id = substrate::derive_did_key(&service_identity.public_key());
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let record = EndpointInfo {
        service_id: service_id.clone(),
        substrate_id: ctx.substrate_client.service_id().to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: vec![],
        nickname: None,
        is_private: false,
        ttl: None,
        not_after: now + DEFAULT_ENDPOINT_NOT_AFTER_SECS,
        generation: 0,
    }
    .sign(&service_identity)
    .unwrap();

    ctx.substrate_client
        .deploy_svc_wasm(
            service_id.clone(),
            vec!["greeter".to_string()],
            wasm_bytes,
            syneroym_sdk::Publication::Public(record),
            None,
        )
        .await
        .expect("deploy test service");

    let mut service_client = SyneroymClient::new_with_identity(
        service_id.to_string(),
        ctx.registry_url.clone(),
        Identity::from_bytes(&ctx.owner.to_bytes()),
    )
    .with_registry_dht(false);
    service_client.connect().await.expect("failed to connect service client");
    let client = &mut service_client;

    // 1. Query signing identity over JSON-RPC via client
    let id_info = call_with_reconnect!(client, client.signing_identity(&service_id).await);
    assert!(id_info.signing_did.starts_with("did:key:"));
    assert!(!id_info.pubkey_hex.is_empty());
    assert_eq!(id_info.owner_did.as_deref(), Some(ctx.owner_did.as_str()));

    // 2. Certify record signing using client & master key
    let cert = call_with_reconnect!(
        client,
        deploy::certify_record_signing(client, &ctx.owner, &service_id, 24).await
    );
    assert_eq!(cert.master_did, ctx.owner_did);
    assert_eq!(cert.temporary_did, id_info.signing_did);
    assert_eq!(cert.scope, SCOPE_RECORD_SIGNING);

    // 3. Certifying for a non-owner master identity is rejected
    let other_master = Identity::generate().unwrap();
    let err =
        deploy::certify_record_signing(client, &other_master, &service_id, 24).await.unwrap_err();
    assert!(err.to_string().contains("does not match service owner"));
}
