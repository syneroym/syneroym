#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! End-to-end integration tests for record signing over a live substrate
//! instance.

mod common;
#[path = "common/retry.rs"]
mod retry;

use common::{SubstrateTestContext, alloc_ports};
use syneroym_core::dht_registry::{DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointType};
use syneroym_identity::{Identity, delegation::SCOPE_RECORD_SIGNING, substrate};
use syneroym_sdk::{SyneroymClient, deploy};
use syneroym_signed_record::{VerifyOptions, verify};

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

    // 4. Sign record over JSON-RPC via signing interface
    let draft = syneroym_signed_record::RecordDraft {
        version: 1,
        record_type: "listing".to_string(),
        subject: "e2e_item".to_string(),
        payload: serde_json::json!({"title":"test item"}),
        expires_at_secs: Some(now + 3600),
        supersedes: None,
    };
    let signed_val = call_with_reconnect!(
        client,
        client.request("signing", "sign-record", serde_json::json!([draft, "service"])).await
    );
    let envelope_json_str: String =
        serde_json::from_value(signed_val.result).expect("extract signed envelope string");
    let envelope: syneroym_signed_record::Envelope =
        serde_json::from_str(&envelope_json_str).expect("parse signed record envelope");
    assert_eq!(envelope.record_type, "listing");
    assert_eq!(envelope.subject, "e2e_item");

    let verified = verify(&envelope, &VerifyOptions::new(now))
        .expect("verify signed record envelope signature and validity window");
    assert_eq!(verified.issuer, id_info.signing_did);

    // 5. Stranger calls signing/sign-record over native dispatch -> refused by
    //    owner gate
    let stranger = Identity::generate().unwrap();
    let mut stranger_client = SyneroymClient::new_with_identity(
        service_id.to_string(),
        ctx.registry_url.clone(),
        stranger,
    )
    .with_registry_dht(false);
    stranger_client.connect().await.expect("connect stranger client");
    let stranger_err = stranger_client
        .request("signing", "sign-record", serde_json::json!([draft, "service"]))
        .await
        .unwrap_err();
    assert!(
        stranger_err.to_string().contains("neither the service itself nor its recorded owner"),
        "expected gate error, got: {stranger_err}"
    );
    let _ = stranger_client.shutdown().await;

    let _ = service_client.shutdown().await;
    ctx.teardown().await;
}
