use std::sync::atomic::Ordering;

use reqwest::Client;
use syneroym_core::{
    config::{RolesConfig, SubstrateConfig, roles::ServiceRegistryRole},
    curated_list::{
        CoordinatorEntry, CoordinatorFilter, CoordinatorTransport, MailboxEntry, fetch,
    },
};

use super::{StubMode, spawn_list_registry, spawn_role_registry, spawn_stub_parent};
use crate::EcosystemRegistry;

const DID_A: &str = "did:key:h7wy1f7qpqh89eo5km8ugxk4rptd9m4h5mnqo6zt1mb59ooj4t9hx4sy";
const DID_B: &str = "did:key:h7wy5y3pq4jr89hu6s1j39so7emhp6wn7wdeqf9t3kiuonw6kdmdyeaa";

fn mailbox(service_id: &str) -> MailboxEntry {
    MailboxEntry { service_id: service_id.to_string(), operator: "op".to_string() }
}

fn role(mailboxes: Vec<MailboxEntry>, parent: Option<String>) -> ServiceRegistryRole {
    ServiceRegistryRole {
        http_bind_address: "127.0.0.1:0".to_string(),
        mailboxes,
        parent_registry_url: parent,
        ..Default::default()
    }
}

#[tokio::test]
async fn registry_serves_its_configured_mailbox_list() {
    let (_reg, url) =
        spawn_role_registry(role(vec![mailbox(DID_A), mailbox(DID_B)], None)).await.unwrap();
    let client = Client::new();

    let plain = client.get(format!("{url}/mailboxes")).send().await.unwrap();
    assert!(plain.status().is_success());
    let with_hops = client.get(format!("{url}/mailboxes?hops=0")).send().await.unwrap();
    assert!(with_hops.status().is_success(), "the hops query must not be read as a filter");

    let list = fetch::<MailboxEntry>(&client, &url, &Default::default(), 0).await.unwrap();
    let ids: Vec<_> = list.entries.iter().map(|e| e.service_id.as_str()).collect();
    assert_eq!(ids, [DID_A, DID_B]);
    assert_eq!(list.max_age_secs, 86_400);
}

#[tokio::test]
async fn an_empty_mailbox_list_is_not_forwarded_to_the_parent() {
    let (parent, parent_url) = spawn_role_registry(role(vec![mailbox(DID_A)], None)).await.unwrap();
    let (_child, child_url) = spawn_role_registry(role(vec![], Some(parent_url))).await.unwrap();
    let (stub_url, counter) = spawn_stub_parent(StubMode::Status500).await;
    let (_child2, child2_url) = spawn_role_registry(role(vec![], Some(stub_url))).await.unwrap();
    let client = Client::new();

    let list = fetch::<MailboxEntry>(&client, &child_url, &Default::default(), 0).await.unwrap();
    let list2 = fetch::<MailboxEntry>(&client, &child2_url, &Default::default(), 0).await.unwrap();

    assert!(list.entries.is_empty());
    assert!(list2.entries.is_empty(), "a parent that fails is not asked");
    assert_eq!(counter.load(Ordering::SeqCst), 0);
    drop(parent);
}

#[tokio::test]
async fn the_coordinator_list_still_forwards_when_the_mailbox_list_does_not() {
    let coordinator = CoordinatorEntry {
        transport: CoordinatorTransport::IrohRelay,
        url: "https://parent-relay.example.org".to_string(),
        info_url: "https://parent-relay.example.org:7974".to_string(),
        operator: "Parent".to_string(),
    };
    let (_parent, parent_url) = spawn_list_registry(vec![coordinator], None, None).await.unwrap();
    let (_child, child_url) = spawn_role_registry(role(vec![], Some(parent_url))).await.unwrap();
    let client = Client::new();

    let relays = fetch::<CoordinatorEntry>(
        &client,
        &child_url,
        &CoordinatorFilter { transport: CoordinatorTransport::IrohRelay },
        0,
    )
    .await
    .unwrap();
    let mailboxes =
        fetch::<MailboxEntry>(&client, &child_url, &Default::default(), 0).await.unwrap();

    assert_eq!(relays.entries.len(), 1);
    assert!(mailboxes.entries.is_empty());
}

#[tokio::test]
async fn a_bad_or_duplicate_mailbox_entry_stops_initialization() {
    let init = |mailboxes| {
        let config = SubstrateConfig {
            roles: RolesConfig {
                community_registry: Some(role(mailboxes, None)),
                ..Default::default()
            },
            ..Default::default()
        };
        async move { EcosystemRegistry::init(&config).await.err().map(|e| format!("{e:#}")) }
    };

    let bad = init(vec![mailbox("did:web:example.com")]).await.unwrap();
    assert!(bad.contains("roles.community_registry.mailboxes[0]"), "{bad}");

    let duplicate = init(vec![mailbox(DID_A), mailbox(DID_A)]).await.unwrap();
    assert!(
        duplicate.contains("roles.community_registry.mailboxes[1] is a duplicate"),
        "{duplicate}"
    );
}
