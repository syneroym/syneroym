#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration test for key revocation and master anchor republishing.

mod common;

use std::fs;

use common::SubstrateNode;
use roymctl::commands::identity;
use rustls::crypto::ring;
use serde_json::Value;
use syneroym_core::dht_registry::RegistryClient;
use syneroym_identity::{DelegationCertificate, Identity, substrate};
use syneroym_rpc::{JsonRpcError, STREAM_UNAUTHENTICATED_RPC_CODE};
use syneroym_sdk::SyneroymClient;

#[tokio::test]
async fn revoke_key_e2e_refuses_caller_and_republish_anchor_keeps_revocations() -> anyhow::Result<()>
{
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    // 1. Boot one SubstrateNode (it serves the registry).
    let node = SubstrateNode::builder().boot().await;
    let registry_url = node.registry_url().to_string();

    // 2. Create master identity and save it under <tempdir>/identities/<name>.key
    let temp_dir = tempfile::tempdir()?;
    let identities_dir = temp_dir.path().join("identities");
    fs::create_dir_all(&identities_dir)?;
    let master = Identity::generate()?;
    let master_name = "test-master";
    let key_file = identities_dir.join(format!("{master_name}.key"));
    fs::write(&key_file, master.to_bytes())?;

    let master_did = substrate::derive_did_key(&master.public_key());

    // 3. Issue a routing-scoped certificate for a fresh temporary key; publish the
    //    anchor.
    let temp_id = Identity::generate()?;
    let temp_bytes = temp_id.to_bytes();
    let temp_did = substrate::derive_did_key(&temp_id.public_key());
    let cert =
        DelegationCertificate::issue(&master, temp_id.public_key(), 3600, "routing".to_string())?;

    let reg_client = RegistryClient::new(false, Some(registry_url.clone()));
    reg_client.publish_master_anchor(&master_did, vec![], None, &master, false).await?;

    // 4. A client with the temporary key and delegation calls orchestrator.readyz
    //    -> succeeds!
    let mut client1 = SyneroymClient::new_with_identity(
        node.did().to_string(),
        registry_url.clone(),
        Identity::from_bytes(&temp_bytes),
    )
    .with_registry_dht(false)
    .with_delegation(cert.clone());

    client1.connect().await?;
    let res1 = client1.request("orchestrator", "readyz", Value::Null).await;
    assert!(
        res1.is_ok(),
        "client with valid delegation must succeed before revocation: {:?}",
        res1.err()
    );
    let _ = client1.shutdown().await;

    // 5. Call roymctl revoke_key
    identity::revoke_key(&reg_client, temp_dir.path(), master_name, &temp_did, false).await?;

    // 6. A new client with the same key and certificate now fails with
    //    JsonRpcError.code == -32097!
    let mut client2 = SyneroymClient::new_with_identity(
        node.did().to_string(),
        registry_url.clone(),
        Identity::from_bytes(&temp_bytes),
    )
    .with_registry_dht(false)
    .with_delegation(cert.clone());

    client2.connect().await?;
    let res2 = client2.request("orchestrator", "readyz", Value::Null).await;
    assert!(res2.is_err(), "revoked key must be rejected");
    let err = res2.unwrap_err();
    let rpc_err = err.downcast_ref::<JsonRpcError>().expect("expected JsonRpcError on refusal");
    assert_eq!(
        rpc_err.code, STREAM_UNAUTHENTICATED_RPC_CODE,
        "revoked key must be refused with STREAM_UNAUTHENTICATED_RPC_CODE (-32097)"
    );
    let _ = client2.shutdown().await;

    // 7. Call republish_master_anchor and verify revocations were kept!
    reg_client.republish_master_anchor(&master, false).await?;
    let anchor = reg_client.resolve_master_anchor(&master_did, None).await?;
    assert!(
        anchor.revoked_keys.contains(&temp_did),
        "republish_master_anchor must preserve existing revoked keys"
    );

    // And verify the revoked key is STILL refused after republish!
    let mut client3 = SyneroymClient::new_with_identity(
        node.did().to_string(),
        registry_url.clone(),
        Identity::from_bytes(&temp_bytes),
    )
    .with_registry_dht(false)
    .with_delegation(cert.clone());

    client3.connect().await?;
    let res3 = client3.request("orchestrator", "readyz", Value::Null).await;
    assert!(res3.is_err());
    let rpc_err3 = res3.unwrap_err().downcast_ref::<JsonRpcError>().unwrap().code;
    assert_eq!(rpc_err3, STREAM_UNAUTHENTICATED_RPC_CODE);
    let _ = client3.shutdown().await;

    node.teardown().await;
    Ok(())
}
