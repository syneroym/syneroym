use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use axum::{
    Json, Router,
    extract::OriginalUri,
    http::{StatusCode, header},
    routing::get,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use syneroym_core::{
    config::{RolesConfig, SubstrateConfig, roles::ServiceRegistryRole},
    curated_list::{
        CoordinatorEntry, CoordinatorFilter, CoordinatorTransport, CuratedList, ListEntry, fetch,
    },
};
use tokio::{net::TcpListener, sync::oneshot};

use super::list_route;
use crate::EcosystemRegistry;

async fn spawn_list_registry(
    coordinators: Vec<CoordinatorEntry>,
    parent_registry_url: Option<String>,
    list_max_age_secs: Option<u64>,
) -> anyhow::Result<(EcosystemRegistry, String)> {
    let mut role = ServiceRegistryRole {
        http_bind_address: "127.0.0.1:0".to_string(),
        coordinators,
        parent_registry_url,
        ..Default::default()
    };
    if let Some(max_age) = list_max_age_secs {
        role.list_max_age_secs = max_age;
    }
    spawn_role_registry(role).await
}

async fn spawn_role_registry(
    role: ServiceRegistryRole,
) -> anyhow::Result<(EcosystemRegistry, String)> {
    let config = SubstrateConfig {
        roles: RolesConfig { community_registry: Some(role), ..Default::default() },
        ..Default::default()
    };
    let mut registry = EcosystemRegistry::init(&config).await?;
    let url = registry.bind().await?;
    registry.spawn().await?;
    Ok((registry, url))
}

enum StubMode {
    Status500,
    Body(&'static str),
    HangUp,
}

async fn spawn_stub_parent(mode: StubMode) -> (String, Arc<AtomicUsize>) {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    match mode {
        StubMode::HangUp => {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(async move {
                while let Ok((socket, _)) = listener.accept().await {
                    counter_clone.fetch_add(1, Ordering::SeqCst);
                    drop(socket);
                }
            });
            (format!("http://127.0.0.1:{port}"), counter)
        }
        StubMode::Status500 => {
            let app = Router::new().fallback(move || {
                let cnt = counter_clone.clone();
                async move {
                    cnt.fetch_add(1, Ordering::SeqCst);
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            });
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(async move {
                let _ = axum::serve(listener, app).await;
            });
            (format!("http://127.0.0.1:{port}"), counter)
        }
        StubMode::Body(body) => {
            let app = Router::new().fallback(move || {
                let cnt = counter_clone.clone();
                async move {
                    cnt.fetch_add(1, Ordering::SeqCst);
                    ([(header::CONTENT_TYPE, "application/json")], body)
                }
            });
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(async move {
                let _ = axum::serve(listener, app).await;
            });
            (format!("http://127.0.0.1:{port}"), counter)
        }
    }
}

#[tokio::test]
async fn registry_serves_its_configured_curated_list() {
    let entries = vec![
        CoordinatorEntry {
            transport: CoordinatorTransport::IrohRelay,
            url: "HTTPS://relay-1.example.org:443/".to_string(),
            info_url: "https://relay-1.example.org:7974/".to_string(),
            operator: "First".to_string(),
        },
        CoordinatorEntry {
            transport: CoordinatorTransport::IrohRelay,
            url: "https://relay-2.example.org/".to_string(),
            info_url: "https://relay-2.example.org:7974".to_string(),
            operator: "Second".to_string(),
        },
    ];

    let (_reg, url) = spawn_list_registry(entries, None, None).await.unwrap();
    let client = Client::new();
    let filter = CoordinatorFilter { transport: CoordinatorTransport::IrohRelay };
    let list = fetch::<CoordinatorEntry>(&client, &url, &filter, 0).await.unwrap();

    assert_eq!(list.max_age_secs, 86_400);
    assert_eq!(list.entries.len(), 2);
    assert_eq!(list.entries[0].url, "https://relay-1.example.org");
    assert_eq!(list.entries[0].info_url, "https://relay-1.example.org:7974");
    assert_eq!(list.entries[1].url, "https://relay-2.example.org");

    let (_reg2, url2) = spawn_list_registry(vec![], None, Some(60)).await.unwrap();
    let list2 = fetch::<CoordinatorEntry>(&client, &url2, &filter, 0).await.unwrap();
    assert_eq!(list2.max_age_secs, 60);
    assert!(list2.entries.is_empty());
}

#[tokio::test]
async fn query_with_bad_or_missing_transport_returns_bad_request() {
    let (_reg, url) = spawn_list_registry(vec![], None, None).await.unwrap();
    let client = Client::new();

    let res_no_transport = client.get(format!("{url}/coordinators")).send().await.unwrap();
    assert_eq!(res_no_transport.status(), StatusCode::BAD_REQUEST);

    let res_bad_transport =
        client.get(format!("{url}/coordinators?transport=webrtc")).send().await.unwrap();
    assert_eq!(res_bad_transport.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn bad_config_or_duplicate_urls_stop_initialization() {
    let make_config = |coordinators| SubstrateConfig {
        roles: RolesConfig {
            community_registry: Some(ServiceRegistryRole {
                http_bind_address: "127.0.0.1:0".to_string(),
                coordinators,
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    let bad_url_config = make_config(vec![CoordinatorEntry {
        transport: CoordinatorTransport::IrohRelay,
        url: "bad-url".to_string(),
        info_url: "https://relay.example.org:7974".to_string(),
        operator: "Bad".to_string(),
    }]);
    let err_bad_url = EcosystemRegistry::init(&bad_url_config).await.unwrap_err();
    let msg_bad_url = format!("{err_bad_url:#}");
    assert!(msg_bad_url.contains("roles.community_registry.coordinators[0]"));

    let duplicate_config = make_config(vec![
        CoordinatorEntry {
            transport: CoordinatorTransport::IrohRelay,
            url: "https://relay.example.org".to_string(),
            info_url: "https://relay.example.org:7974".to_string(),
            operator: "First".to_string(),
        },
        CoordinatorEntry {
            transport: CoordinatorTransport::IrohRelay,
            url: "HTTPS://relay.example.org:443/".to_string(),
            info_url: "https://relay.example.org:7974".to_string(),
            operator: "Duplicate".to_string(),
        },
    ]);
    let err_duplicate = EcosystemRegistry::init(&duplicate_config).await.unwrap_err();
    let msg_duplicate = format!("{err_duplicate:#}");
    assert!(msg_duplicate.contains("roles.community_registry.coordinators[1] is a duplicate"));

    let bad_parent_config = SubstrateConfig {
        roles: RolesConfig {
            community_registry: Some(ServiceRegistryRole {
                http_bind_address: "127.0.0.1:0".to_string(),
                parent_registry_url: Some("bad-url".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let err_bad_parent = EcosystemRegistry::init(&bad_parent_config).await.unwrap_err();
    let msg_bad_parent = format!("{err_bad_parent:#}");
    assert!(msg_bad_parent.contains("parent_registry_url"));

    let query_parent_config = SubstrateConfig {
        roles: RolesConfig {
            community_registry: Some(ServiceRegistryRole {
                http_bind_address: "127.0.0.1:0".to_string(),
                parent_registry_url: Some("https://parent.example.org/?q=1".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let err_query_parent = EcosystemRegistry::init(&query_parent_config).await.unwrap_err();
    let msg_query_parent = format!("{err_query_parent:#}");
    assert!(msg_query_parent.contains("parent_registry_url"));
}

#[tokio::test]
async fn empty_list_forwards_to_parent_registry() {
    let parent_entry = CoordinatorEntry {
        transport: CoordinatorTransport::IrohRelay,
        url: "https://parent-relay.example.org".to_string(),
        info_url: "https://parent-relay.example.org:7974".to_string(),
        operator: "Parent".to_string(),
    };
    let (_parent, p_url) =
        spawn_list_registry(vec![parent_entry.clone()], None, Some(3600)).await.unwrap();

    let (_child, c_url) = spawn_list_registry(vec![], Some(p_url), Some(86400)).await.unwrap();

    let client = Client::new();
    let filter = CoordinatorFilter { transport: CoordinatorTransport::IrohRelay };
    let list = fetch::<CoordinatorEntry>(&client, &c_url, &filter, 0).await.unwrap();

    assert_eq!(list.max_age_secs, 3600);
    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].url, "https://parent-relay.example.org");
}

#[tokio::test]
async fn own_entries_take_precedence_without_asking_parent() {
    let (p_url, counter) = spawn_stub_parent(StubMode::Status500).await;

    let child_entry = CoordinatorEntry {
        transport: CoordinatorTransport::IrohRelay,
        url: "https://child-relay.example.org".to_string(),
        info_url: "https://child-relay.example.org:7974".to_string(),
        operator: "Child".to_string(),
    };
    let (_child, c_url) = spawn_list_registry(vec![child_entry], Some(p_url), None).await.unwrap();

    let client = Client::new();
    let filter = CoordinatorFilter { transport: CoordinatorTransport::IrohRelay };
    let list = fetch::<CoordinatorEntry>(&client, &c_url, &filter, 0).await.unwrap();

    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].url, "https://child-relay.example.org");
    assert_eq!(counter.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn hop_limit_stops_forwarding_chain() {
    let entry = CoordinatorEntry {
        transport: CoordinatorTransport::IrohRelay,
        url: "https://deep-relay.example.org".to_string(),
        info_url: "https://deep-relay.example.org:7974".to_string(),
        operator: "Deep".to_string(),
    };

    // Chain: R0 -> R1 -> R2 -> R3 -> R4
    // Spawn from end so URLs are available
    let (_r4, r4_url) = spawn_list_registry(vec![entry.clone()], None, None).await.unwrap();
    let (_r3, r3_url) = spawn_list_registry(vec![], Some(r4_url), None).await.unwrap();
    let (_r2, r2_url) = spawn_list_registry(vec![], Some(r3_url), None).await.unwrap();
    let (_r1, r1_url) = spawn_list_registry(vec![], Some(r2_url), None).await.unwrap();
    let (_r0, r0_url) = spawn_list_registry(vec![], Some(r1_url), None).await.unwrap();

    let client = Client::new();
    let filter = CoordinatorFilter { transport: CoordinatorTransport::IrohRelay };

    // At R0, hops=0 -> R1 hops=1 -> R2 hops=2 -> R3 hops=3 (hop limit reached,
    // returns empty)
    let list_stopped = fetch::<CoordinatorEntry>(&client, &r0_url, &filter, 0).await.unwrap();
    assert!(list_stopped.entries.is_empty());

    // Chain where R3 has the entry: R0 -> R1 -> R2 -> R3
    let (_r3_with_entry, r3_url2) = spawn_list_registry(vec![entry], None, None).await.unwrap();
    let (_r2_2, r2_url2) = spawn_list_registry(vec![], Some(r3_url2), None).await.unwrap();
    let (_r1_2, r1_url2) = spawn_list_registry(vec![], Some(r2_url2), None).await.unwrap();
    let (_r0_2, r0_url2) = spawn_list_registry(vec![], Some(r1_url2), None).await.unwrap();

    let list_reached = fetch::<CoordinatorEntry>(&client, &r0_url2, &filter, 0).await.unwrap();
    assert_eq!(list_reached.entries.len(), 1);
    assert_eq!(list_reached.entries[0].url, "https://deep-relay.example.org");
}

#[tokio::test]
async fn loop_prevention_at_hop_limit_does_not_call_parent() {
    let (p_url, counter) = spawn_stub_parent(StubMode::Status500).await;
    let (_r0, r0_url) = spawn_list_registry(vec![], Some(p_url), None).await.unwrap();

    let client = Client::new();
    let res = client
        .get(format!("{r0_url}/coordinators?transport=iroh-relay&hops=3"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let list = res.json::<CuratedList<CoordinatorEntry>>().await.unwrap();
    assert!(list.entries.is_empty());
    assert_eq!(counter.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn large_hop_count_stops_forwarding_or_returns_bad_request() {
    let (p_url, counter) = spawn_stub_parent(StubMode::Status500).await;
    let (_r0, r0_url) = spawn_list_registry(vec![], Some(p_url), None).await.unwrap();

    let client = Client::new();
    let res_200 = client
        .get(format!("{r0_url}/coordinators?transport=iroh-relay&hops=200"))
        .send()
        .await
        .unwrap();
    assert_eq!(res_200.status(), StatusCode::OK);
    let list = res_200.json::<CuratedList<CoordinatorEntry>>().await.unwrap();
    assert!(list.entries.is_empty());
    assert_eq!(counter.load(Ordering::SeqCst), 0);

    let res_256 = client
        .get(format!("{r0_url}/coordinators?transport=iroh-relay&hops=256"))
        .send()
        .await
        .unwrap();
    assert_eq!(res_256.status(), StatusCode::BAD_REQUEST);
    assert_eq!(counter.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn failing_parent_returns_bad_gateway() {
    for mode in [StubMode::Status500, StubMode::Body("not valid json"), StubMode::HangUp] {
        let (p_url, counter) = spawn_stub_parent(mode).await;
        let (_child, c_url) = spawn_list_registry(vec![], Some(p_url), None).await.unwrap();

        let client = Client::new();
        let res =
            client.get(format!("{c_url}/coordinators?transport=iroh-relay")).send().await.unwrap();

        assert_eq!(res.status(), StatusCode::BAD_GATEWAY);
        let body = res.text().await.unwrap();
        assert_eq!(body, "parent registry did not answer");
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn broken_parent_entry_fails_whole_forward_with_bad_gateway() {
    let (p_url, counter) = spawn_stub_parent(StubMode::Body(
        r#"{"max_age_secs":60,"coordinators":[{"url":"https://x"}]}"#,
    ))
    .await;
    let (_child, c_url) = spawn_list_registry(vec![], Some(p_url), None).await.unwrap();

    let client = Client::new();
    let res =
        client.get(format!("{c_url}/coordinators?transport=iroh-relay")).send().await.unwrap();

    assert_eq!(res.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn no_parent_and_empty_list_returns_empty_list_ok() {
    let (_child, c_url) = spawn_list_registry(vec![], None, None).await.unwrap();
    let client = Client::new();
    let filter = CoordinatorFilter { transport: CoordinatorTransport::IrohRelay };
    let list = fetch::<CoordinatorEntry>(&client, &c_url, &filter, 0).await.unwrap();

    assert_eq!(list.max_age_secs, 86_400);
    assert!(list.entries.is_empty());
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TestEntry {
    id: String,
}

#[derive(Debug, Deserialize)]
struct NoFilter {}

impl ListEntry for TestEntry {
    const LIST_KEY: &'static str = "test-entries";
    type Filter = NoFilter;

    fn matches(&self, _: &NoFilter) -> bool {
        true
    }

    fn filter_pairs(_: &NoFilter) -> Vec<(&'static str, String)> {
        Vec::new()
    }

    fn check(&mut self) -> anyhow::Result<String> {
        anyhow::ensure!(!self.id.is_empty(), "empty id");
        Ok(self.id.clone())
    }
}

#[tokio::test]
async fn generic_curated_list_route_supports_second_entry_type() {
    let role = ServiceRegistryRole { list_max_age_secs: 1800, ..Default::default() };
    let client = Client::new();
    let entries = vec![TestEntry { id: "item-1".to_string() }];

    let route = list_route::<TestEntry>(entries, &role, client.clone()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, route).await;
    });

    let res = client.get(format!("http://127.0.0.1:{port}/test-entries")).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.json::<CuratedList<TestEntry>>().await.unwrap();
    assert_eq!(body.max_age_secs, 1800);
    assert_eq!(body.entries.len(), 1);
    assert_eq!(body.entries[0].id, "item-1");

    let dup_entries =
        vec![TestEntry { id: "dup".to_string() }, TestEntry { id: "dup".to_string() }];
    assert!(list_route::<TestEntry>(dup_entries, &role, client).is_err());
}

#[tokio::test]
async fn forward_with_no_filter_carries_only_hops_query_param() {
    let (tx, rx) = oneshot::channel();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let parent_app = Router::new().route(
        "/test-entries",
        get(move |OriginalUri(uri): OriginalUri| {
            let tx = tx.clone();
            async move {
                if let Some(sender) = tx.lock().unwrap().take() {
                    let _ = sender.send(uri.to_string());
                }
                Json(CuratedList {
                    max_age_secs: 1800,
                    entries: vec![TestEntry { id: "parent-item".to_string() }],
                })
            }
        }),
    );
    let parent_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let parent_port = parent_listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(parent_listener, parent_app).await;
    });

    let client = Client::new();
    let child_role = ServiceRegistryRole {
        http_bind_address: "127.0.0.1:0".to_string(),
        parent_registry_url: Some(format!("http://127.0.0.1:{parent_port}")),
        list_max_age_secs: 1800,
        ..Default::default()
    };
    let child_entries: Vec<TestEntry> = Vec::new();
    let child_route = list_route::<TestEntry>(child_entries, &child_role, client.clone()).unwrap();
    let child_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let child_port = child_listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(child_listener, child_route).await;
    });

    let child_url = format!("http://127.0.0.1:{child_port}");
    let list = fetch::<TestEntry>(&client, &child_url, &NoFilter {}, 0).await.unwrap();
    assert_eq!(list.max_age_secs, 1800);
    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].id, "parent-item");

    let received_uri = rx.await.unwrap();
    assert_eq!(received_uri, "/test-entries?hops=1");
}

mod mailbox;
