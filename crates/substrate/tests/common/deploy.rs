//! Shared deploy helpers for the `crates/substrate/tests/*.rs` suites.
//!
//! Every test that deploys a WASM or TCP service through the orchestrator's
//! `orchestrator/deploy` JSON-RPC method used a local copy of the same
//! two-line helper. Both variants are here:
//!
//! - [`deploy_app`]: panics on failure — the common case where deployment is a
//!   precondition, not what the test is asserting.
//! - [`try_deploy_app`]: returns `anyhow::Result<()>` — for tests whose test
//!   subject is the deploy call itself.
//!
//! [`one_service_manifest`] is the supervisor-family fixture: a single-service
//! `SynAppManifest` parameterised by blueprint id and dummy TCP source
//! address.  Every test that needs one calls this rather than maintaining its
//! own near-identical copy.

#![allow(dead_code)]

use std::collections::BTreeMap;

use semver::Version;
use serde_json::json;
use syneroym_app_orchestration::models::{
    AppBlueprintId, LogicalServiceName, PlacementSelector, ServiceConfig, ServiceSpec, ServiceType,
    SubstrateAlias, SynAppManifest,
};
use syneroym_sdk::{DeployManifest, SyneroymClient};

/// Deploy `manifest` for `service_id`, asserting that the orchestrator
/// reports `{"status": "deployed"}`.  Panics on any failure, including a
/// non-success JSON-RPC result — use this when deployment is a test
/// precondition, not the thing being tested.
pub async fn deploy_app(client: &SyneroymClient, service_id: &str, manifest: DeployManifest) {
    let params = serde_json::to_value((service_id.to_string(), manifest)).unwrap();
    let res =
        client.request("orchestrator", "deploy", params).await.expect("deploy request failed");
    assert_eq!(res.result, json!({"status": "deployed"}), "deploy did not succeed: {res:?}");
}

/// Like [`deploy_app`], but returns `anyhow::Result<()>` instead of panicking.
/// Use this when the test is asserting on the deploy outcome itself.
pub async fn try_deploy_app(
    client: &SyneroymClient,
    service_id: &str,
    manifest: DeployManifest,
) -> anyhow::Result<()> {
    let params = serde_json::to_value((service_id.to_string(), manifest))?;
    let res = client.request("orchestrator", "deploy", params).await?;
    if res.result == json!({"status": "deployed"}) {
        Ok(())
    } else {
        Err(anyhow::anyhow!("deploy did not report success: {:?}", res.result))
    }
}

/// A minimal single-service `SynAppManifest`: one `backend` service of type
/// `Tcp` placed on the `MANAGED_ALIAS` substrate alias.
///
/// - `id`: the `AppBlueprintId` string (must be unique per test to avoid
///   cross-test artifact collisions, e.g. `"syneroym:my-test-app"`).
/// - `source`: the dummy TCP address the service nominally listens on (never
///   actually dialed by tests that use this fixture).
pub fn one_service_manifest(id: &str, source: &str) -> SynAppManifest {
    let mut services = BTreeMap::new();
    services.insert(
        LogicalServiceName::new("backend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: source.to_string(),
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
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new("managed"))),
            replicas: 1,
            sharding_strategy: None,
            schedule: None,
            topology_visibility: Default::default(),
        },
    );
    SynAppManifest {
        id: AppBlueprintId::new(id),
        version: Version::new(0, 1, 0),
        description: None,
        placement: None,
        services,
        dependencies: BTreeMap::new(),
    }
}
