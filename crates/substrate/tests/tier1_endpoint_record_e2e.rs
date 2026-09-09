#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Tier 1 of the logical discovery overlay (ADR-0022 §2), proven across two
//! genuinely independent `syneroym-substrate` instances -- a caller outside
//! the app instance resolving "which supervisor holds this app" through the
//! same registry every other DID in the system already uses.
//!
//! `one_service_manifest` is local; the supervisor/managed pair and the
//! submit helpers come from `common`. `poll_interval_secs` is lowered so the
//! resident loop's own Tier-1 publish -- which nothing on the `supervisor`
//! RPC surface triggers synchronously (`force-reconcile` calls
//! `deploy_submission` directly, not the write-phase gate the resident loop
//! evaluates) -- lands inside this test's own poll budget.

use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use common::{compiled_plan_json, submission, supervisor_and_managed};
use semver::Version;
use serde_json::json;
use syneroym_app_orchestration::models::{
    AppBlueprintId, LogicalServiceName, PlacementSelector, ServiceConfig, ServiceSpec, ServiceType,
    SubstrateAlias, SynAppManifest,
};
use syneroym_core::dht_registry::{EndpointInfo, EndpointType, RegistryClient};
use syneroym_identity::{Identity, substrate};
use tokio::time;

mod common;

#[path = "common/retry.rs"]
mod retry;

const MANAGED_ALIAS: &str = "managed";

/// The resident loop's own tick is what this test waits on, so the poll
/// interval is short.
const POLL_INTERVAL_SECS: u64 = 2;

/// A single-service manifest, `backend` placed on `MANAGED_ALIAS`.
fn one_service_manifest() -> SynAppManifest {
    let mut services = BTreeMap::new();
    services.insert(
        LogicalServiceName::new("backend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41901".to_string(),
                hash: None,
                interfaces: vec![],
                env: BTreeMap::new(),
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: Default::default(),
                fdae: None,
                health_check: None,
                assets: None,
                visibility: Default::default(),
            },
            depends_on: vec![],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_ALIAS))),
            replicas: 1,
            sharding_strategy: None,
            schedule: None,
            topology_visibility: Default::default(),
        },
    );
    SynAppManifest {
        id: AppBlueprintId::new("syneroym:tier1-test-app"),
        version: Version::new(0, 1, 0),
        description: None,
        placement: None,
        services,
        dependencies: BTreeMap::new(),
    }
}

/// The reference scenario's steps 1-2: submit and adopt an app instance,
/// confirm the app master DID on `status`, then assert the Tier-1 record
/// resolves through the registry -- naming the supervisor and verifying
/// against the app DID with no other trust input.
#[tokio::test]
async fn an_app_did_resolves_to_its_supervising_node_through_the_registry() {
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) = supervisor_and_managed(
        &supervisor_owner,
        &managed_owner,
        POLL_INTERVAL_SECS,
        MANAGED_ALIAS,
    )
    .await;

    let manifest = one_service_manifest();
    let plan_json = compiled_plan_json(&manifest, "tier1-resolve-inst").await;
    // `supervisor_node`'s connection was dialed and proven live by its own
    // `wait_for_ready` during boot inside `supervisor_and_managed`, then sat
    // idle through the managed node's own full boot that followed -- long
    // enough under CI's scheduling pressure for the peer to abandon that
    // idle path ("no viable network path exists: last path abandoned by
    // peer"). Recover by explicit shutdown->reconnect before one retry.
    let submit_params = submission("tier1-resolve-inst", plan_json, inventory_json, 0);
    crate::call_with_reconnect!(
        supervisor_node.substrate_client,
        "supervisor",
        "submit",
        submit_params
    );
    let adopted = supervisor_node
        .substrate_client
        .request("supervisor", "adopt", json!(["tier1-resolve-inst"]))
        .await
        .expect("adopt failed");
    let app_did = adopted
        .result
        .get("app_master_did")
        .and_then(|v| v.as_str())
        .expect("adopt-result carries app_master_did")
        .to_string();
    assert!(app_did.starts_with("did:key:"), "{app_did}");

    // Confirmed on `status` too (D-A7-6, already proven at unit scale) --
    // the DID this test then resolves through the registry is the exact
    // one the operator would read off `status`.
    let status = supervisor_node
        .substrate_client
        .request("supervisor", "status", json!(["tier1-resolve-inst"]))
        .await
        .expect("status failed");
    assert_eq!(
        status.result.get("app_master_did").and_then(|v| v.as_str()),
        Some(app_did.as_str())
    );

    // The resident loop's own tick publishes the Tier-1 record; nothing on
    // the RPC surface triggers it synchronously (`force-reconcile` bypasses
    // the write-phase gate the loop itself evaluates), so this polls for
    // it rather than asserting immediately.
    let registry_client =
        RegistryClient::new(false, Some(supervisor_node.registry_url().to_string()));
    let deadline = Instant::now() + Duration::from_secs(60);
    let signed = loop {
        match registry_client.lookup(&app_did, false).await {
            Ok(signed) => break signed,
            Err(e) => {
                assert!(
                    Instant::now() < deadline,
                    "the Tier-1 record never resolved through the registry: {e}"
                );
                time::sleep(Duration::from_millis(300)).await;
            }
        }
    };

    // Looking up the app DID returns the supervising node, and the record
    // verifies against the app DID with no other trust input.
    assert_eq!(
        signed.info.substrate_id,
        supervisor_node.did(),
        "the record must name the substrate supervising this app, not the app DID itself"
    );
    assert_eq!(signed.info.service_id, app_did);
    assert_eq!(signed.info.endpoint_type, EndpointType::Substrate);
    assert!(signed.verify().is_ok(), "a freshly published Tier-1 record must verify");

    // `status`'s own expiry field (D-C-2) is populated once a publish has
    // actually landed.
    let status_after = supervisor_node
        .substrate_client
        .request("supervisor", "status", json!(["tier1-resolve-inst"]))
        .await
        .expect("status failed");
    assert!(
        status_after.result.get("app_record_expires_at").and_then(|v| v.as_u64()).is_some(),
        "status must report the published record's expiry: {:?}",
        status_after.result
    );

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

/// Failure-matrix row 1: a Tier-1 record claiming an app DID as its
/// `service_id` but signed by a key unrelated to it must be rejected at the
/// registry, in the shape `master_endpoint_record_e2e.rs`'s own
/// hand-forged-record case already uses.
#[tokio::test]
async fn a_forged_tier1_record_is_rejected_at_the_registry() {
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (supervisor_node, managed_node, _inventory_json) = supervisor_and_managed(
        &supervisor_owner,
        &managed_owner,
        POLL_INTERVAL_SECS,
        MANAGED_ALIAS,
    )
    .await;

    let claimed_app_master = Identity::generate().unwrap();
    let claimed_app_did = substrate::derive_did_key(&claimed_app_master.public_key());
    let uncertified = Identity::generate().unwrap();

    let forged = EndpointInfo {
        service_id: claimed_app_did,
        substrate_id: supervisor_node.did().to_string(),
        endpoint_type: EndpointType::Substrate,
        mechanisms: vec![],
        nickname: Some("forged".to_string()),
        is_private: false,
        ttl: None,
        not_after: u64::MAX / 2,
        generation: 0,
    }
    .sign(&uncertified)
    .expect("failed to sign forged record");

    let res = reqwest::Client::new()
        .post(format!("{}/register", supervisor_node.registry_url()))
        .json(&forged)
        .send()
        .await
        .expect("failed to POST forged record");
    assert_eq!(res.status(), reqwest::StatusCode::UNAUTHORIZED);

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}
