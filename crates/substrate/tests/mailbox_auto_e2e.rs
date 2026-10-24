#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `mailbox = "auto"` through the supervisor, end to end (ADR-0026
//! Decision 8): the registry lists an open mailbox, `submit` pins it in the
//! stored plan, the member's signed record names it, and `status` shows it.

use std::time::{Duration, Instant};

use reqwest::Client;
use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_app_orchestration::{LogicalServiceName, MailboxChoice, Visibility};
use syneroym_core::{
    config::MailboxRole,
    curated_list::{MailboxEntry, MailboxFilter, fetch},
    dht_registry::RegistryClient,
};
use syneroym_identity::Identity;
use tokio::time;

use crate::common::SubstrateNode;

mod common;

const INSTANCE_ID: &str = "c7-auto";

#[tokio::test]
async fn a_submit_with_auto_pins_a_listed_mailbox_and_its_record_names_it() {
    let _ = ring::default_provider().install_default();
    let owner = Identity::generate().unwrap();
    let node = SubstrateNode::builder()
        .owner(&owner)
        .supervisor(common::supervisor_role(2))
        .inject_kek()
        .boot()
        .await;

    // The node lists itself as an open mailbox. Its DID is known only after
    // the first boot; the restart keeps its ports and identity, so the
    // registry address does not change.
    let did = node.did().to_string();
    let listed = did.clone();
    let node = node
        .restart_with(move |builder| {
            builder.configure(move |c| {
                c.roles.mailbox = Some(MailboxRole::default());
                c.roles.community_registry.as_mut().expect("a registry role").mailboxes =
                    vec![MailboxEntry { service_id: listed.clone(), operator: "test".into() }];
            })
        })
        .await;
    let list = fetch::<MailboxEntry>(&Client::new(), node.registry_url(), &MailboxFilter {}, 0)
        .await
        .expect("the registry serves the mailbox list");
    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].service_id, did);

    // One public service with `mailbox = "auto"`, placed on the node itself.
    let mut manifest = common::one_service_manifest("syneroym:c7-auto-app");
    let backend = manifest.services.get_mut(&LogicalServiceName::new("backend")).unwrap();
    backend.config.visibility = Visibility::Public;
    backend.mailbox = Some(MailboxChoice::Auto);
    let plan_json = common::compiled_plan_json(&manifest, INSTANCE_ID).await;
    let grant = common::node_wide_supervisor_grant(&owner, node.did(), node.did());
    let inventory_json = common::inventory_json(common::MANAGED_ALIAS, &node, grant);

    let submitted = node
        .substrate_client
        .request(
            "supervisor",
            "submit",
            common::submission(INSTANCE_ID, plan_json, inventory_json, 0),
        )
        .await
        .expect("submit failed");

    // The member's signed record names the pinned mailbox.
    let master_did = submitted.result["masters"][0]["master_did"]
        .as_str()
        .expect("submit names the minted master")
        .to_string();
    let registry = RegistryClient::new(false, Some(node.registry_url().to_string()));
    let deadline = Instant::now() + Duration::from_secs(60);
    let record = loop {
        match registry.lookup(&master_did, false).await {
            Ok(record) => break record,
            Err(_) if Instant::now() < deadline => time::sleep(Duration::from_millis(300)).await,
            Err(e) => panic!("the record of {master_did} never resolved: {e}"),
        }
    };
    let mailbox = record.info.mailbox.expect("the record names a mailbox");
    assert_eq!(mailbox.service_id, did);

    // `status` shows the choice the supervisor made.
    let status = node
        .substrate_client
        .request("supervisor", "status", json!([INSTANCE_ID]))
        .await
        .expect("status failed")
        .result;
    let services = status.get("services").and_then(Value::as_array).expect("status lists services");
    assert_eq!(services[0].get("mailbox").and_then(Value::as_str), Some(did.as_str()));

    node.teardown().await;
}
