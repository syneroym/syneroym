#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration tests verifying registry-listed curated coordinator endpoints
//! and configuration validation through full substrate boot.

use reqwest::Client;
use syneroym_core::curated_list::{
    CoordinatorEntry, CoordinatorFilter, CoordinatorTransport, MailboxEntry, MailboxFilter, fetch,
};

use crate::common::SubstrateNode;

mod common;

#[tokio::test]
async fn booted_substrate_serves_curated_coordinators_list() {
    let entry = CoordinatorEntry {
        transport: CoordinatorTransport::IrohRelay,
        url: "https://Relay.Example.ORG:443/".to_string(),
        info_url: "https://Relay.Example.ORG:8443/".to_string(),
        operator: "Example Org".to_string(),
    };

    let node = SubstrateNode::builder()
        .configure({
            let entry = entry.clone();
            move |c| {
                if let Some(reg) = c.roles.community_registry.as_mut() {
                    reg.coordinators = vec![entry.clone()];
                }
            }
        })
        .boot()
        .await;

    let client = Client::new();
    let filter = CoordinatorFilter { transport: CoordinatorTransport::IrohRelay };
    let list = fetch::<CoordinatorEntry>(&client, node.registry_url(), &filter, 0)
        .await
        .expect("fetching curated list from booted substrate");

    assert_eq!(list.max_age_secs, 86_400);
    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].url, "https://relay.example.org");
    assert_eq!(list.entries[0].info_url, "https://relay.example.org:8443");
    assert_eq!(list.entries[0].operator, "Example Org");

    node.teardown().await;
}

#[tokio::test]
async fn duplicate_coordinator_urls_stop_substrate_init() {
    let entry1 = CoordinatorEntry {
        transport: CoordinatorTransport::IrohRelay,
        url: "https://relay.example.org".to_string(),
        info_url: "https://relay.example.org:7974".to_string(),
        operator: "First".to_string(),
    };
    let entry2 = CoordinatorEntry {
        transport: CoordinatorTransport::IrohRelay,
        url: "HTTPS://relay.example.ORG:443/".to_string(),
        info_url: "https://relay.example.org:7974/".to_string(),
        operator: "Duplicate".to_string(),
    };

    let err = SubstrateNode::builder()
        .configure(move |c| {
            if let Some(reg) = c.roles.community_registry.as_mut() {
                reg.coordinators = vec![entry1.clone(), entry2.clone()];
            }
        })
        .init_error()
        .await;

    let err_msg = format!("{err:#}");
    assert!(
        err_msg.contains("roles.community_registry.coordinators[1]"),
        "error chain {err_msg:?} must name entry [1]"
    );
}

#[tokio::test]
async fn booted_substrate_serves_curated_mailboxes_list() {
    let did = "did:key:h7wy1f7qpqh89eo5km8ugxk4rptd9m4h5mnqo6zt1mb59ooj4t9hx4sy";
    let entry = MailboxEntry { service_id: did.to_string(), operator: "Example Org".to_string() };

    let node = SubstrateNode::builder()
        .configure({
            let entry = entry.clone();
            move |c| {
                if let Some(reg) = c.roles.community_registry.as_mut() {
                    reg.mailboxes = vec![entry.clone()];
                }
            }
        })
        .boot()
        .await;

    let client = Client::new();
    let list = fetch::<MailboxEntry>(&client, node.registry_url(), &MailboxFilter {}, 0)
        .await
        .expect("fetching the mailbox list from a booted substrate");

    assert_eq!(list.entries, vec![entry]);
    node.teardown().await;
}

#[tokio::test]
async fn duplicate_mailbox_ids_stop_substrate_init() {
    let did = "did:key:h7wy1f7qpqh89eo5km8ugxk4rptd9m4h5mnqo6zt1mb59ooj4t9hx4sy";
    let entry = MailboxEntry { service_id: did.to_string(), operator: "First".to_string() };

    let err = SubstrateNode::builder()
        .configure(move |c| {
            if let Some(reg) = c.roles.community_registry.as_mut() {
                reg.mailboxes = vec![entry.clone(), entry.clone()];
            }
        })
        .init_error()
        .await;

    let err_msg = format!("{err:#}");
    assert!(
        err_msg.contains("roles.community_registry.mailboxes[1]"),
        "error chain {err_msg:?} must name entry [1]"
    );
}
