//! Supervisor-family fixtures for the `crates/substrate/tests/*_e2e.rs` suites
//! that boot a supervisor node plus a managed node. Every one of those files
//! carried a verbatim copy of each helper below.
//!
//! Gated on the `supervisor` Cargo feature (in the default set) because the
//! inventory type comes from the optional `syneroym-app-supervisor` crate.

#![allow(dead_code)]

use std::{collections::BTreeMap, path::PathBuf};

use rustls::crypto::ring;
use serde_json::{Map, Value, json};
use syneroym_app_orchestration::{
    LocalFilesystemCatalog, compile,
    models::{AppInstanceId, SynAppManifest},
};
use syneroym_app_supervisor::inventory::SupervisorInventoryEntry;
use syneroym_core::config::SupervisorRole;
use syneroym_identity::Identity;
use syneroym_rpc::{Ability, Capability, CapabilityToken, ResourceUri};

use crate::common::SubstrateNode;

/// A `SupervisorRole` with the test-fast knobs every suite uses.
/// `poll_interval_secs` is the one value they actually vary -- lowered from
/// the 30s default when a test waits on the resident loop's own tick.
pub fn supervisor_role(poll_interval_secs: u64) -> SupervisorRole {
    SupervisorRole {
        poll_interval_secs,
        db_name: "supervisor.db".to_string(),
        max_restart_attempts: 3,
        restart_backoff_secs: 30,
        alert_topic: "supervisor/alerts".to_string(),
        master_backup_dir: "master-backups".to_string(),
        ..SupervisorRole::default()
    }
}

/// Node-wide `orchestrator/deploy` **and** `orchestrator/status` for
/// `grantee_did` on `node_did` -- what a supervisor needs on every substrate
/// it manages.
pub fn node_wide_supervisor_grant(
    node_owner: &Identity,
    grantee_did: &str,
    node_did: &str,
) -> CapabilityToken {
    let resource = ResourceUri::substrate(node_did);
    CapabilityToken::issue(
        node_owner,
        grantee_did,
        [Ability::ORCHESTRATOR_DEPLOY, Ability::ORCHESTRATOR_STATUS]
            .into_iter()
            .map(|a| Capability {
                with: resource.clone(),
                can: Ability(a.to_string()),
                caveats: None,
            })
            .collect(),
        Map::new(),
        3600,
        vec![],
    )
    .expect("issue node-wide supervisor grant")
}

/// The one-entry inventory JSON a `supervisor.submit` takes: `alias` mapped
/// to `managed`'s DID and registry, carrying `grant`.
pub fn inventory_json(alias: &str, managed: &SubstrateNode, grant: CapabilityToken) -> String {
    serde_json::to_string(&BTreeMap::from([(
        alias.to_string(),
        SupervisorInventoryEntry {
            did: managed.did().to_string(),
            api_url: Some(managed.registry_url().to_string()),
            ucan: Some(grant),
        },
    )]))
    .expect("serialize supervisor inventory")
}

/// Compile `manifest` for `instance_id` and return the last plan as JSON.
pub async fn compiled_plan_json(manifest: &SynAppManifest, instance_id: &str) -> String {
    let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
    let compiled = compile(AppInstanceId::new(instance_id), manifest, &catalog).await.unwrap();
    compiled.plans.last().unwrap().to_json().unwrap()
}

/// The `supervisor.submit` params array.
pub fn submission(
    instance_id: &str,
    plan_json: String,
    inventory_json: String,
    generation: u64,
) -> Value {
    json!([{
        "app_instance_id": instance_id,
        "plan_json": plan_json,
        "inventory_json": inventory_json,
        "generation": generation,
    }])
}

/// Boot a supervisor node (hosting the registry) and a managed node that
/// joins it through a shared registry and relay, granting the supervisor
/// node-wide `orchestrator/deploy` + `/status` on the managed node. Returns
/// `(supervisor, managed, inventory_json)` keyed by `alias`.
pub async fn supervisor_and_managed(
    supervisor_owner: &Identity,
    managed_owner: &Identity,
    poll_interval_secs: u64,
    alias: &str,
) -> (SubstrateNode, SubstrateNode, String) {
    let _ = ring::default_provider().install_default();

    let supervisor = SubstrateNode::builder()
        .owner(supervisor_owner)
        .supervisor(supervisor_role(poll_interval_secs))
        .inject_kek()
        .boot()
        .await;
    let managed = SubstrateNode::builder()
        .owner(managed_owner)
        .shared_registry(supervisor.registry_url())
        .shared_relay(supervisor.relay_url())
        .inject_kek()
        .boot()
        .await;
    let grant = node_wide_supervisor_grant(managed_owner, supervisor.did(), managed.did());
    let inv = inventory_json(alias, &managed, grant);
    (supervisor, managed, inv)
}
