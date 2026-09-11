use tempfile::{TempDir, tempdir};

use super::*;

async fn make_store() -> (SqliteEndpointStorage, TempDir) {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");
    let store = SqliteEndpointStorage::new(path).await.unwrap();
    (store, dir)
}

/// Every other deploy-facts test writes through `MockStorage` or
/// a live single-process deploy -- nothing proves the round trip through
/// a real `SqliteEndpointStorage` file and back out, which is the fact
/// `instance_phase`'s recorded-type lookup and `readyz`'s repaired guess
/// rest on. If this path regressed, every service on
/// a rebooted node would silently fall to `Unknown("no service type
/// recorded")` and `readyz` would quietly stop inspecting containers --
/// a healthy-looking failure the suite would otherwise never catch.
#[tokio::test]
async fn deploy_facts_survive_a_real_reopen_of_the_same_database_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("registry.db");

    {
        let store = SqliteEndpointStorage::new(&path).await.unwrap();
        store
            .save_deploy_facts(
                "svc-1",
                "tcp",
                Some(r#"{"tcp-connect":{"interface-name":"main","timeout-ms":2000}}"#),
                Some("deadbeef"),
                Some("internal"),
            )
            .await
            .unwrap();
    }

    // A genuinely fresh connection to the same file, not the same
    // `SqliteEndpointStorage` instance.
    let reopened = SqliteEndpointStorage::new(&path).await.unwrap();
    let facts = reopened.load_all_deploy_facts().await.unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].0, "svc-1");
    assert_eq!(facts[0].1, "tcp");
    assert!(facts[0].2.as_deref().unwrap().contains("tcp-connect"));
    assert_eq!(facts[0].3.as_deref(), Some("deadbeef"));
    assert_eq!(facts[0].4.as_deref(), Some("internal"));
}

/// `service_deploy_facts` predates its `manifest_hash` column -- unlike
/// every other table in this file, `CREATE TABLE IF NOT EXISTS` is a
/// no-op against it, so the column needs its own idempotent `ALTER
/// TABLE`. Built with a raw `Connection` for the same reason as
/// `an_existing_database_gains_the_certificate_table_on_open`: it has to
/// reproduce the older, `manifest_hash`-less table directly, not go
/// through `SqliteEndpointStorage::new`, which already creates the
/// column unconditionally regardless of whether the `ALTER TABLE` still
/// runs.
#[tokio::test]
async fn an_existing_database_gains_the_manifest_hash_column_on_open() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");

    {
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "CREATE TABLE service_deploy_facts (
                    service_id        TEXT PRIMARY KEY,
                    service_type      TEXT NOT NULL,
                    health_check_json TEXT,
                    created_at        INTEGER NOT NULL
                );",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO service_deploy_facts (service_id, service_type, created_at) VALUES \
             ('svc-old', 'tcp', 0)",
            [],
        )
        .unwrap();
        conn.execute("PRAGMA user_version = 1", []).unwrap();
    }

    // Reopen through the real constructor -- the pre-existing row must
    // survive, and `manifest_hash` must both read (as `None` for the
    // untouched row) and write.
    let store = SqliteEndpointStorage::new(&path).await.unwrap();
    let facts = store.load_all_deploy_facts().await.unwrap();
    assert_eq!(facts, vec![("svc-old".to_string(), "tcp".to_string(), None, None, None)]);

    store
        .save_deploy_facts("svc-old", "tcp", None, Some("deadbeef"), Some("public"))
        .await
        .unwrap();
    let facts = store.load_all_deploy_facts().await.unwrap();
    assert_eq!(facts[0].3.as_deref(), Some("deadbeef"));
    assert_eq!(facts[0].4.as_deref(), Some("public"));
}

/// The `app_instance_management`-shaped sibling of the deploy-facts
/// round-trip test above -- every other table in this file has this
/// coverage; this table did not.
#[tokio::test]
async fn app_instance_management_saves_loads_and_removes() {
    let (store, _dir) = make_store().await;
    let management = AppInstanceManagement {
        owner_did: "did:key:owner".to_string(),
        supervisor_did: Some("did:key:supervisor".to_string()),
        generation: 3,
    };
    store.save_app_instance_management("app-1", &management).await.unwrap();

    let loaded = store.load_all_app_instance_management().await.unwrap();
    assert_eq!(loaded, vec![("app-1".to_string(), management)]);

    store.remove_app_instance_management("app-1").await.unwrap();
    assert!(store.load_all_app_instance_management().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_save_and_load_wasm() {
    let (store, _dir) = make_store().await;
    let ep = SubstrateEndpoint::WasmChannel { service_id: "app-123".to_string() };
    store.save("app-123", "greet", &ep).await.unwrap();

    let all = store.load_all().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0, "app-123");
    assert_eq!(all[0].1, "greet");
    assert!(
        matches!(&all[0].2, SubstrateEndpoint::WasmChannel { service_id } if service_id == "app-123")
    );
}

#[tokio::test]
async fn test_save_and_load_tcp() {
    let (store, _dir) = make_store().await;
    let ep = SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port: 8080 };
    store.save("app-tcp", "api", &ep).await.unwrap();

    let all = store.load_all().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0, "app-tcp");
    assert_eq!(all[0].1, "api");
    assert!(
        matches!(&all[0].2, SubstrateEndpoint::TcpHostPort { host, port } if host == "127.0.0.1" && *port == 8080)
    );
}

#[tokio::test]
async fn test_save_and_load_podman() {
    let (store, _dir) = make_store().await;
    let ep = SubstrateEndpoint::PodmanSocket { socket_path: "/var/run/podman.sock".to_string() };
    store.save("app-podman", "socket", &ep).await.unwrap();

    let all = store.load_all().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0, "app-podman");
    assert_eq!(all[0].1, "socket");
    assert!(
        matches!(&all[0].2, SubstrateEndpoint::PodmanSocket { socket_path } if socket_path == "/var/run/podman.sock")
    );
}

#[tokio::test]
async fn test_save_upserts() {
    let (store, _dir) = make_store().await;
    let ep1 = SubstrateEndpoint::WasmChannel { service_id: "v1".to_string() };
    let ep2 = SubstrateEndpoint::WasmChannel { service_id: "v2".to_string() };
    store.save("svc", "iface", &ep1).await.unwrap();
    store.save("svc", "iface", &ep2).await.unwrap();

    let all = store.load_all().await.unwrap();
    assert_eq!(all.len(), 1);
    assert!(
        matches!(&all[0].2, SubstrateEndpoint::WasmChannel { service_id } if service_id == "v2")
    );
}

#[tokio::test]
async fn test_remove() {
    let (store, _dir) = make_store().await;
    let ep = SubstrateEndpoint::NativeHostChannel { service_id: "sub-1".to_string() };
    store.save("sub-1", "orchestrator", &ep).await.unwrap();
    store.remove("sub-1", "orchestrator").await.unwrap();
    assert!(store.load_all().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_load_invalid_endpoints() {
    let (store, _dir) = make_store().await;

    // Save a valid one first
    let ep = SubstrateEndpoint::WasmChannel { service_id: "valid-1".to_string() };
    store.save("valid-1", "iface", &ep).await.unwrap();

    // Directly insert invalid rows to mock corrupted DB data
    let conn_arc = store.conn.clone();
    task::spawn_blocking(move || {
        let conn = conn_arc.lock().unwrap();

        // Invalid endpoint type key
        conn.execute(
            "INSERT INTO local_endpoints (service_id, interface_name, endpoint_type, \
             endpoint_data)
                 VALUES ('invalid-type', 'iface', 'unknown_type', 'some_data')",
            [],
        )
        .unwrap();

        // Invalid TCP data format (no colon)
        conn.execute(
            "INSERT INTO local_endpoints (service_id, interface_name, endpoint_type, \
             endpoint_data)
                 VALUES ('invalid-tcp-no-colon', 'iface', 'tcp', '127.0.0.1')",
            [],
        )
        .unwrap();

        // Invalid TCP data format (non-integer port)
        conn.execute(
            "INSERT INTO local_endpoints (service_id, interface_name, endpoint_type, \
             endpoint_data)
                 VALUES ('invalid-tcp-bad-port', 'iface', 'tcp', '127.0.0.1:abc')",
            [],
        )
        .unwrap();
    })
    .await
    .unwrap();

    // load_all should skip the invalid ones, warning about them, and still return
    // the valid one
    let all = store.load_all().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0, "valid-1");
    assert!(
        matches!(&all[0].2, SubstrateEndpoint::WasmChannel { service_id } if service_id == "valid-1")
    );
}

/// A freshly created `endpoints.db` gets both tables in the same
/// `version == 0` migration block -- `service_owners` is usable
/// immediately, no separate migration step.
#[tokio::test]
async fn test_fresh_db_gets_service_owners_table() {
    let (store, _dir) = make_store().await;
    store.save_owner("svc-1", "did:key:zOwner").await.unwrap();
    let owners = store.load_all_owners().await.unwrap();
    assert_eq!(owners, vec![("svc-1".to_string(), "did:key:zOwner".to_string())]);
}

#[tokio::test]
async fn test_save_owner_upserts() {
    let (store, _dir) = make_store().await;
    store.save_owner("svc-1", "did:key:zAlice").await.unwrap();
    store.save_owner("svc-1", "did:key:zBob").await.unwrap();

    let owners = store.load_all_owners().await.unwrap();
    assert_eq!(owners, vec![("svc-1".to_string(), "did:key:zBob".to_string())]);
}

#[tokio::test]
async fn test_remove_owner() {
    let (store, _dir) = make_store().await;
    store.save_owner("svc-1", "did:key:zOwner").await.unwrap();
    store.remove_owner("svc-1").await.unwrap();
    assert!(store.load_all_owners().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_remove_owner_is_idempotent() {
    let (store, _dir) = make_store().await;
    store.remove_owner("never-owned").await.unwrap();
}

#[tokio::test]
async fn a_fresh_db_gets_the_certificate_table() {
    let (store, _dir) = make_store().await;
    store.save_cert("svc-1", r#"{"fake":"cert"}"#).await.unwrap();
    let certs = store.load_all_certs().await.unwrap();
    assert_eq!(certs, vec![("svc-1".to_string(), r#"{"fake":"cert"}"#.to_string())]);
}

#[tokio::test]
async fn saving_a_certificate_upserts() {
    let (store, _dir) = make_store().await;
    store.save_cert("svc-1", r#"{"v":1}"#).await.unwrap();
    store.save_cert("svc-1", r#"{"v":2}"#).await.unwrap();

    let certs = store.load_all_certs().await.unwrap();
    assert_eq!(certs, vec![("svc-1".to_string(), r#"{"v":2}"#.to_string())]);
}

#[tokio::test]
async fn removing_a_service_removes_its_certificate() {
    let (store, _dir) = make_store().await;
    store.save_cert("svc-1", r#"{"fake":"cert"}"#).await.unwrap();
    store.remove_cert("svc-1").await.unwrap();
    assert!(store.load_all_certs().await.unwrap().is_empty());
}

/// An existing database, already at `PRAGMA user_version == 1` from
/// before the certificate table existed, must still gain it on the next
/// open -- this is the regression a `version < 2` migration gate would
/// have reintroduced. Built with a raw `Connection` rather than
/// `SqliteEndpointStorage::new`: that constructor already creates the
/// certificate table unconditionally, so opening through it here would
/// make this test pass identically whether or not the gate it exists to
/// catch was reintroduced -- it has to reproduce the pre-existing,
/// version-1, certificate-table-less file directly.
#[tokio::test]
async fn an_existing_database_gains_the_certificate_table_on_open() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");

    {
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "CREATE TABLE local_endpoints (
                    service_id TEXT NOT NULL,
                    interface_name TEXT NOT NULL,
                    endpoint_type TEXT NOT NULL,
                    endpoint_data TEXT NOT NULL,
                    PRIMARY KEY (service_id, interface_name)
                );",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE service_owners (
                    service_id TEXT PRIMARY KEY,
                    owner_did  TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );",
            [],
        )
        .unwrap();
        conn.execute("PRAGMA user_version = 1", []).unwrap();
    }

    // Reopen through the real constructor -- schema creation must run
    // unconditionally and add the certificate table to this
    // pre-existing, version-1 file.
    let store = SqliteEndpointStorage::new(&path).await.unwrap();
    store.save_cert("svc-1", r#"{"fake":"cert"}"#).await.unwrap();
    let certs = store.load_all_certs().await.unwrap();
    assert_eq!(certs, vec![("svc-1".to_string(), r#"{"fake":"cert"}"#.to_string())]);
}

/// Another version of the same regression: a database that predates
/// `service_app_context`/`service_bindings` must still gain both tables
/// on the next open, for the identical reason `an_existing_database_
/// gains_the_certificate_table_on_open` exists one table over.
#[tokio::test]
async fn an_existing_database_gains_the_app_context_and_binding_tables_on_open() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");

    {
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "CREATE TABLE local_endpoints (
                    service_id TEXT NOT NULL,
                    interface_name TEXT NOT NULL,
                    endpoint_type TEXT NOT NULL,
                    endpoint_data TEXT NOT NULL,
                    PRIMARY KEY (service_id, interface_name)
                );",
            [],
        )
        .unwrap();
        conn.execute("PRAGMA user_version = 1", []).unwrap();
    }

    let store = SqliteEndpointStorage::new(&path).await.unwrap();
    store.save_app_context("svc-1", "app-1", "backend").await.unwrap();
    store.save_binding("svc-1", "app-1", "backend", r#"{"fake":"entry"}"#).await.unwrap();
    assert_eq!(
        store.load_all_app_contexts().await.unwrap(),
        vec![("svc-1".to_string(), "app-1".to_string(), "backend".to_string())]
    );
    assert_eq!(
        store.load_all_bindings().await.unwrap(),
        vec![(
            "svc-1".to_string(),
            "app-1".to_string(),
            "backend".to_string(),
            r#"{"fake":"entry"}"#.to_string()
        )]
    );
}

#[tokio::test]
async fn saving_an_app_context_upserts() {
    let (store, _dir) = make_store().await;
    store.save_app_context("svc-1", "app-1", "backend").await.unwrap();
    store.save_app_context("svc-1", "app-2", "backend-v2").await.unwrap();

    let contexts = store.load_all_app_contexts().await.unwrap();
    assert_eq!(
        contexts,
        vec![("svc-1".to_string(), "app-2".to_string(), "backend-v2".to_string())]
    );
}

#[tokio::test]
async fn saving_a_binding_upserts_in_place() {
    let (store, _dir) = make_store().await;
    store.save_binding("svc-1", "app-1", "backend", r#"{"v":1}"#).await.unwrap();
    store.save_binding("svc-1", "app-1", "backend", r#"{"v":2}"#).await.unwrap();

    let bindings = store.load_all_bindings().await.unwrap();
    assert_eq!(
        bindings,
        vec![(
            "svc-1".to_string(),
            "app-1".to_string(),
            "backend".to_string(),
            r#"{"v":2}"#.to_string()
        )]
    );
}

/// `load_all_bindings`'s replay consumer
/// (`substrate::runtime::replay_persisted_bindings`) discards
/// `service_id` and keys purely on `(app_instance_id, dependency_name)`
/// -- last-write-wins means a conflict between two services'
/// rows for the same instance/name depends entirely on iteration order.
/// Inserted deliberately out of both id order and insertion order, so a
/// `SELECT` with no `ORDER BY` (SQLite's row order is otherwise
/// unspecified) would have a real chance of returning them unsorted.
#[tokio::test]
async fn loading_all_bindings_is_ordered_by_service_id_then_dependency_name() {
    let (store, _dir) = make_store().await;
    store.save_binding("svc-b", "app-1", "y-dep", r#"{"v":1}"#).await.unwrap();
    store.save_binding("svc-a", "app-1", "z-dep", r#"{"v":1}"#).await.unwrap();
    store.save_binding("svc-a", "app-1", "x-dep", r#"{"v":1}"#).await.unwrap();
    store.save_binding("svc-b", "app-1", "a-dep", r#"{"v":1}"#).await.unwrap();

    let bindings = store.load_all_bindings().await.unwrap();
    let ids: Vec<(&str, &str)> =
        bindings.iter().map(|(sid, _instance, dep, _entry)| (sid.as_str(), dep.as_str())).collect();
    assert_eq!(
        ids,
        vec![("svc-a", "x-dep"), ("svc-a", "z-dep"), ("svc-b", "a-dep"), ("svc-b", "y-dep")],
        "replay must be reproducible across restarts regardless of insertion order or SQLite's \
         own unspecified row order"
    );
}

#[tokio::test]
async fn removing_an_app_context_removes_its_binding_rows_too() {
    let (store, _dir) = make_store().await;
    store.save_app_context("svc-1", "app-1", "backend").await.unwrap();
    store.save_binding("svc-1", "app-1", "backend-dep", r#"{"fake":"entry"}"#).await.unwrap();

    store.remove_app_context("svc-1").await.unwrap();

    assert!(store.load_all_app_contexts().await.unwrap().is_empty());
    assert!(store.load_all_bindings().await.unwrap().is_empty());
}

#[tokio::test]
async fn removing_an_app_context_is_idempotent() {
    let (store, _dir) = make_store().await;
    store.remove_app_context("never-deployed").await.unwrap();
}
