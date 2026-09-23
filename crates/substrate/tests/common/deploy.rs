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

#![allow(dead_code)]

use serde_json::json;
use syneroym_sdk::{DeployManifest, SyneroymClient};

/// Deploy `manifest` for `service_id`, asserting that the orchestrator
/// reports `{"status": "deployed"}`. Panics on any failure, including a
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
