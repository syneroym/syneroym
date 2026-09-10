#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The app-instance master identity end to end, across two
//! genuinely independent `syneroym-substrate` instances -- the operator's
//! own sequence: `submit`, `adopt`, `status`, `export-master`, a second
//! `adopt`. The supervisor/managed pair and the submit helpers come from
//! `common`; `one_service_manifest` is local. The test reads the supervisor
//! node's `app_data_dir` to confirm `export-master` wrote a real file under
//! its own `master_backup_dir`, not just that the RPC returned a path string.

use std::{collections::BTreeMap, path::PathBuf};

use common::{compiled_plan_json, submission, supervisor_and_managed};
use semver::Version;
use serde_json::json;
use syneroym_app_orchestration::models::{
    AppBlueprintId, LogicalServiceName, PlacementSelector, ServiceConfig, ServiceSpec, ServiceType,
    SubstrateAlias, SynAppManifest,
};
use syneroym_identity::Identity;

mod common;

const MANAGED_ALIAS: &str = "managed";

/// Nothing in this file waits on the resident loop, so the poll interval is
/// left near the production default.
const POLL_INTERVAL_SECS: u64 = 30;

/// A single-service manifest, `backend` placed on `MANAGED_ALIAS`.
fn one_service_manifest() -> SynAppManifest {
    let mut services = BTreeMap::new();
    services.insert(
        LogicalServiceName::new("backend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41601".to_string(),
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
        id: AppBlueprintId::new("syneroym:a7-test-app"),
        version: Version::new(0, 1, 0),
        description: None,
        placement: None,
        services,
        dependencies: BTreeMap::new(),
    }
}

/// The operator's own sequence over a real supervisor and a real
/// managed substrate -- `submit`, `adopt`, then (a) `adopt`'s result
/// carries a `did:key:` app master and a vault name, (b) `status` reports
/// the same DID, (c) `export-master` with that name writes a file under
/// the node's `master_backup_dir`, and (d) a second `adopt` reports the
/// identical DID at a higher generation. The individual properties are
/// already proven at unit scale (tests 90-97, 99-101); the claim here is
/// the sequence, in the operator's own order.
#[tokio::test]
async fn an_adopted_app_instance_carries_an_exportable_master_did() {
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
    let plan_json = compiled_plan_json(&manifest, "a7-adopt-inst").await;
    let submit_params = submission("a7-adopt-inst", plan_json, inventory_json, 0);
    // `supervisor_node`'s connection was dialed and proven live by its own
    // `wait_for_ready` during `supervisor_and_managed`, then sat idle for the
    // entire `managed_node` boot that followed (a second full substrate start
    // -- wasmtime/DHT/relay init) before this, the test's first real call,
    // reuses it -- long enough under CI's scheduling pressure for the peer
    // to abandon that idle path ("no viable network path exists: last path
    // abandoned by peer"). `SyneroymClient::connect` no-ops on an
    // already-`Some` connection, so recovering means an explicit
    // `shutdown`-then-`connect` (redial) before one retry, not just
    // retrying the same request on the same dead connection.
    if supervisor_node
        .substrate_client
        .request("supervisor", "submit", submit_params.clone())
        .await
        .is_err()
    {
        supervisor_node
            .substrate_client
            .shutdown()
            .await
            .expect("failed to reset supervisor_node's stale connection");
        supervisor_node
            .substrate_client
            .connect()
            .await
            .expect("failed to reconnect supervisor_node");
    }
    supervisor_node
        .substrate_client
        .request("supervisor", "submit", submit_params)
        .await
        .expect("submit failed");

    // (a) `adopt`'s result carries a `did:key:` app master and a vault
    // name.
    let adopted = supervisor_node
        .substrate_client
        .request("supervisor", "adopt", json!(["a7-adopt-inst"]))
        .await
        .expect("adopt failed");
    assert_eq!(adopted.result.get("generation").and_then(serde_json::Value::as_u64), Some(1));
    let app_master_did = adopted
        .result
        .get("app_master_did")
        .and_then(|v| v.as_str())
        .expect("adopt-result carries app_master_did")
        .to_string();
    assert!(app_master_did.starts_with("did:key:"), "{app_master_did}");
    let vault_name = adopted
        .result
        .get("vault_name")
        .and_then(|v| v.as_str())
        .expect("adopt-result carries vault_name")
        .to_string();
    assert_eq!(vault_name, "app-a7-adopt-inst");

    // (b) `status` reports the same DID.
    let status = supervisor_node
        .substrate_client
        .request("supervisor", "status", json!(["a7-adopt-inst"]))
        .await
        .expect("status failed");
    assert_eq!(
        status.result.get("app_master_did").and_then(|v| v.as_str()),
        Some(app_master_did.as_str())
    );

    // (c) `export-master` with that name writes a real file under this
    // node's own `master_backup_dir` -- the master is movable through
    // `export-master`/`import-master`.
    let exported = supervisor_node
        .substrate_client
        .request("supervisor", "export-master", json!([vault_name.clone()]))
        .await
        .expect("export-master failed");
    let exported_path =
        PathBuf::from(exported.result.as_str().expect("export-master returns a path string"));
    assert_eq!(
        exported_path,
        supervisor_node.app_data_dir().join("master-backups").join(format!("{vault_name}.key"))
    );
    assert!(
        tokio::fs::metadata(&exported_path).await.is_ok(),
        "export-master must have written a real file at {}",
        exported_path.display()
    );

    // (d) a second `adopt` reports the identical DID at a higher
    // generation.
    let adopted_again = supervisor_node
        .substrate_client
        .request("supervisor", "adopt", json!(["a7-adopt-inst"]))
        .await
        .expect("second adopt failed");
    assert_eq!(adopted_again.result.get("generation").and_then(serde_json::Value::as_u64), Some(2));
    assert_eq!(
        adopted_again.result.get("app_master_did").and_then(|v| v.as_str()),
        Some(app_master_did.as_str())
    );

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}
