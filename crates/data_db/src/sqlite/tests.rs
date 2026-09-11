use syneroym_fdae::{DecisionTrace, parse_and_validate};
use tempfile::tempdir;

use super::*;

#[test]
fn test_startup_migrates_previous_substrate_schema_to_m3a() {
    let dir = tempdir().unwrap();
    let substrate_path = dir.path().join("substrate.db");
    {
        let conn = Connection::open(&substrate_path).unwrap();
        conn.execute(
            "CREATE TABLE schema_version (
                    id INTEGER PRIMARY KEY CHECK (id = 1),
                    version TEXT NOT NULL
                )",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO schema_version (id, version) VALUES (1, 'm2')", []).unwrap();
    }

    let _provider = SqliteStorageProvider::new(dir.path(), false).unwrap();

    let conn = Connection::open(&substrate_path).unwrap();
    let version: String =
        conn.query_row("SELECT version FROM schema_version LIMIT 1", [], |row| row.get(0)).unwrap();
    assert_eq!(version, SUBSTRATE_SCHEMA_VERSION);

    let dek_store_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'dek_store'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(dek_store_count, 1);
}

#[test]
fn test_startup_uses_documented_schema_version_shape() {
    let dir = tempdir().unwrap();
    let substrate_path = dir.path().join("substrate.db");

    let _provider = SqliteStorageProvider::new(dir.path(), false).unwrap();

    let conn = Connection::open(&substrate_path).unwrap();
    let columns: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(schema_version)").unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };

    assert_eq!(columns, vec!["version"]);
}

#[tokio::test]
async fn test_service_id_validation_and_path_traversal() {
    let dir = tempdir().unwrap();
    let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
    let key_store = Arc::new(KeyStore::new());

    // Valid ID
    assert!(provider.open_service_db("svc-1", &key_store).await.is_ok());
    assert!(provider.open_service_db("my_service-2", &key_store).await.is_ok());
    // Real service ids are DIDs and contain colons.
    assert!(
        provider
            .open_service_db(
                "did:key:h7wy4ppo5gystkfs71hf19qhmbaqc3yx7gpcbtg4s9h6ojozbgx61nco",
                &key_store
            )
            .await
            .is_ok()
    );

    // Invalid IDs
    assert!(provider.open_service_db("svc/../../traversal", &key_store).await.is_err());
    assert!(provider.open_service_db("did:key:../../traversal", &key_store).await.is_err());
    assert!(provider.open_service_db("svc_with_spaces ", &key_store).await.is_err());
    assert!(provider.open_service_db("svc!", &key_store).await.is_err());
    assert!(provider.open_service_db("", &key_store).await.is_err());
}

#[tokio::test]
async fn test_encryption_key_required() {
    let dir = tempdir().unwrap();
    // Encryption enabled
    let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
    let key_store = Arc::new(KeyStore::new());

    // Fails with EncryptionKeyRequired because KEK is not injected
    let res = provider.open_service_db("my-service", &key_store).await;
    assert!(res.is_err());
    assert_eq!(res.err().unwrap().to_string(), "EncryptionKeyRequired");

    // Inject KEK
    key_store.inject_kek([9u8; 32]).unwrap();

    // Now succeeds
    assert!(provider.open_service_db("my-service", &key_store).await.is_ok());
}

#[tokio::test]
async fn test_load_service_dek_none_when_encryption_disabled() {
    let dir = tempdir().unwrap();
    let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
    let key_store = Arc::new(KeyStore::new());
    let dek = provider.load_service_dek("svc-a", &key_store).await.unwrap();
    assert_eq!(dek, None);
}

#[tokio::test]
async fn test_load_service_dek_requires_kek_when_encryption_enabled() {
    let dir = tempdir().unwrap();
    let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
    let key_store = Arc::new(KeyStore::new());
    let res = provider.load_service_dek("svc-a", &key_store).await;
    assert!(res.is_err());
}

#[tokio::test]
async fn test_load_service_dek_generates_then_reuses_same_dek() {
    let dir = tempdir().unwrap();
    let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([21u8; 32]).unwrap();

    let dek_a = provider.load_service_dek("svc-a", &key_store).await.unwrap();
    let dek_b = provider.load_service_dek("svc-a", &key_store).await.unwrap();
    assert!(dek_a.is_some());
    assert_eq!(dek_a, dek_b);
}

#[tokio::test]
async fn test_load_service_dek_matches_open_service_db_dek() {
    // Regression guard for the open_service_db refactor: both paths
    // must resolve to the identical DEK for the same service_id, since
    // open_service_db's SQLCipher pragma and load_service_dek's callers
    // (e.g. blob-store) must agree on the same key material.
    let dir = tempdir().unwrap();
    let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([33u8; 32]).unwrap();

    // Opening the service DB first generates the DEK as a side effect.
    let _ = provider.open_service_db("svc-shared", &key_store).await.unwrap();
    let via_load = provider.load_service_dek("svc-shared", &key_store).await.unwrap();
    assert!(via_load.is_some());

    // A second provider instance sharing the same substrate.db must
    // resolve the identical DEK (survives "restart").
    let provider2 = SqliteStorageProvider::new(dir.path(), true).unwrap();
    let via_load2 = provider2.load_service_dek("svc-shared", &key_store).await.unwrap();
    assert_eq!(via_load, via_load2);
}

/// Two distinct `service_id`s under one master
/// KEK produce two working, independently-keyed service DBs, exercising
/// the full `StorageProvider` path with per-instance derivation.
#[tokio::test]
async fn test_open_service_db_two_instances_independently_keyed() {
    let dir = tempdir().unwrap();
    let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([44u8; 32]).unwrap();

    let store_a = provider.open_service_db("kek-svc-a", &key_store).await.unwrap();
    let store_b = provider.open_service_db("kek-svc-b", &key_store).await.unwrap();

    store_a.write_secret("api_key", b"secret-for-a").await.unwrap();
    store_b.write_secret("api_key", b"secret-for-b").await.unwrap();

    assert_eq!(store_a.reveal_secret("api_key").await.unwrap(), Some(b"secret-for-a".to_vec()));
    assert_eq!(store_b.reveal_secret("api_key").await.unwrap(), Some(b"secret-for-b".to_vec()));

    // Each instance's DEK is distinct -- derived under its own scope.
    let dek_a = provider.load_service_dek("kek-svc-a", &key_store).await.unwrap();
    let dek_b = provider.load_service_dek("kek-svc-b", &key_store).await.unwrap();
    assert_ne!(dek_a, dek_b);
}

/// The negative case at the storage layer: a DEK that is genuinely
/// instance A's own (not a copy of B's)
/// does not open instance B's on-disk SQLCipher database. SQLCipher
/// accepts any `PRAGMA key`; a wrong key surfaces as a decrypt failure
/// on the first real read ("file is not a database"), asserted here via
/// a raw connection rather than through `StorageProvider`, since the
/// trait never exposes "open with an explicit foreign key".
#[tokio::test]
async fn test_cross_instance_dek_does_not_open_sibling_sqlcipher_db() {
    let dir = tempdir().unwrap();
    let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([55u8; 32]).unwrap();

    let store_b = provider.open_service_db("iso-svc-b", &key_store).await.unwrap();
    store_b.write_secret("marker", b"only-in-b").await.unwrap();
    drop(store_b);

    // svc-a's own (real, generated) DEK -- not svc-b's.
    let dek_a = provider.load_service_dek("iso-svc-a", &key_store).await.unwrap().unwrap();

    let db_b_path = dir.path().join("services").join("iso-svc-b").join("state.db");
    let raw = Connection::open(&db_b_path).unwrap();
    let pragma_val = format!("x'{}'", hex::encode(*dek_a));
    raw.pragma_update(None, "key", &pragma_val).unwrap();
    let result: rusqlite::Result<i64> =
        raw.query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| row.get(0));
    assert!(result.is_err(), "svc-a's DEK must not decrypt svc-b's SQLCipher database");
}

#[tokio::test]
async fn test_vault_write_and_reveal() {
    let dir = tempdir().unwrap();
    let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([15u8; 32]).unwrap();

    let store = provider.open_service_db("vault-test", &key_store).await.unwrap();

    // Reveal missing key returns None
    assert_eq!(store.reveal_secret("api_key").await.unwrap(), None);

    // Write secret
    let secret = b"super-secret-token-123";
    store.write_secret("api_key", secret).await.unwrap();

    // Reveal secret
    let revealed = store.reveal_secret("api_key").await.unwrap();
    assert_eq!(revealed, Some(secret.to_vec()));
}

#[tokio::test]
async fn test_open_service_db_reuses_cached_store_actor() {
    let dir = tempdir().unwrap();
    let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([17u8; 32]).unwrap();

    let first = provider.open_service_db("cached-vault", &key_store).await.unwrap();
    let second = provider.open_service_db("cached-vault", &key_store).await.unwrap();

    first.write_secret("api_key", b"cached-store-secret").await.unwrap();
    let revealed = second.reveal_secret("api_key").await.unwrap();

    assert_eq!(revealed, Some(b"cached-store-secret".to_vec()));
    assert_eq!(provider.service_stores.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_path_traversal_etc_passwd() {
    let dir = tempdir().unwrap();
    let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
    let key_store = Arc::new(KeyStore::new());

    // Explicitly assert path traversal reject
    assert!(provider.open_service_db("../../etc/passwd", &key_store).await.is_err());
}

#[tokio::test]
async fn test_restart_survival() {
    let dir = tempdir().unwrap();
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([42u8; 32]).unwrap();

    // Write data on first boot
    {
        let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
        let store = provider.open_service_db("restart-test", &key_store).await.unwrap();
        store.write_secret("secret_key", b"survival-data-100").await.unwrap();
    }

    // Read data after "restart" (re-instantiating SqliteStorageProvider)
    {
        let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
        let store = provider.open_service_db("restart-test", &key_store).await.unwrap();
        let revealed = store.reveal_secret("secret_key").await.unwrap();
        assert_eq!(revealed, Some(b"survival-data-100".to_vec()));
    }
}

#[tokio::test]
async fn test_messaging_subscriptions_roundtrip_and_restart_survival() {
    let dir = tempdir().unwrap();

    {
        let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
        provider.save_messaging_subscription("svc-a", "svc/svc-a/orders/new").await.unwrap();
        provider.save_messaging_subscription("svc-a", "sensors/+/temp").await.unwrap();
        provider.save_messaging_subscription("svc-b", "svc/svc-b/status").await.unwrap();
        // Re-subscribing to the same topic is idempotent, not an error.
        provider.save_messaging_subscription("svc-a", "sensors/+/temp").await.unwrap();

        let mut all = provider.list_all_messaging_subscriptions().await.unwrap();
        all.sort();
        assert_eq!(
            all,
            vec![
                ("svc-a".to_string(), "sensors/+/temp".to_string()),
                ("svc-a".to_string(), "svc/svc-a/orders/new".to_string()),
                ("svc-b".to_string(), "svc/svc-b/status".to_string()),
            ]
        );

        provider.delete_messaging_subscription("svc-a", "sensors/+/temp").await.unwrap();
        let mut after_delete = provider.list_all_messaging_subscriptions().await.unwrap();
        after_delete.sort();
        assert_eq!(
            after_delete,
            vec![
                ("svc-a".to_string(), "svc/svc-a/orders/new".to_string()),
                ("svc-b".to_string(), "svc/svc-b/status".to_string()),
            ]
        );

        provider.delete_all_messaging_subscriptions_for_service("svc-a").await.unwrap();
        let after_undeploy = provider.list_all_messaging_subscriptions().await.unwrap();
        assert_eq!(after_undeploy, vec![("svc-b".to_string(), "svc/svc-b/status".to_string())]);
    }

    // Surviving row is still there after "restart" (re-opening the same db_dir).
    {
        let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
        let all = provider.list_all_messaging_subscriptions().await.unwrap();
        assert_eq!(all, vec![("svc-b".to_string(), "svc/svc-b/status".to_string())]);
    }
}

#[tokio::test]
async fn test_fdae_policy_save_load_roundtrip_and_replace() {
    let dir = tempdir().unwrap();
    let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();

    assert_eq!(provider.load_fdae_policy("svc-a").await.unwrap(), None);

    provider.save_fdae_policy("svc-a", r#"{"version":1}"#).await.unwrap();
    assert_eq!(
        provider.load_fdae_policy("svc-a").await.unwrap(),
        Some(r#"{"version":1}"#.to_string())
    );

    // A second save for the same service_id replaces (last-write-wins, one row).
    provider.save_fdae_policy("svc-a", r#"{"version":2}"#).await.unwrap();
    assert_eq!(
        provider.load_fdae_policy("svc-a").await.unwrap(),
        Some(r#"{"version":2}"#.to_string())
    );

    assert_eq!(provider.load_fdae_policy("svc-unknown").await.unwrap(), None);
}

#[tokio::test]
async fn test_fdae_policy_delete_is_idempotent_and_removes_the_row() {
    let dir = tempdir().unwrap();
    let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();

    // Deleting a row that was never saved is not an error.
    provider.delete_fdae_policy("svc-never-saved").await.unwrap();

    provider.save_fdae_policy("svc-a", r#"{"version":1}"#).await.unwrap();
    provider.delete_fdae_policy("svc-a").await.unwrap();
    assert_eq!(provider.load_fdae_policy("svc-a").await.unwrap(), None);

    // Deleting the same row again is still not an error.
    provider.delete_fdae_policy("svc-a").await.unwrap();
}

#[tokio::test]
async fn test_list_collections_returns_created_tables_excludes_vault_and_sqlite_internals() {
    let dir = tempdir().unwrap();
    let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([21u8; 32]).unwrap();

    let store = provider.open_service_db("list-collections-svc", &key_store).await.unwrap();

    store
        .create_collection(&host_store::CollectionSchema {
            name: "widgets".to_string(),
            indexes: vec![],
        })
        .await
        .unwrap();
    store
        .create_collection(&host_store::CollectionSchema {
            name: "gadgets".to_string(),
            indexes: vec![],
        })
        .await
        .unwrap();
    // A leading underscore is a legal collection identifier
    // (`IDENTIFIER_REGEX` is `^[a-zA-Z_]...`) and must not be swept up
    // by an overly broad `_%` exclusion meant only for `_vault`.
    store
        .create_collection(&host_store::CollectionSchema {
            name: "_audit".to_string(),
            indexes: vec![],
        })
        .await
        .unwrap();
    // A vault write forces `_vault` to exist alongside the collections.
    store.write_secret("k", b"v").await.unwrap();

    let mut collections = store.list_collections().await.unwrap();
    collections.sort();
    assert_eq!(
        collections,
        vec!["_audit".to_string(), "gadgets".to_string(), "widgets".to_string()]
    );
}

#[test]
fn test_insecure_mode_warning() {
    use std::io;

    use tracing_subscriber::prelude::*;

    let logs = Arc::new(Mutex::new(Vec::new()));
    let logs_clone = logs.clone();

    struct MockWriter {
        logs: Arc<Mutex<Vec<u8>>>,
    }
    impl io::Write for MockWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.logs.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let make_writer = move || MockWriter { logs: logs_clone.clone() };
    let layer = tracing_subscriber::fmt::layer().with_writer(make_writer);

    let dir = tempdir().unwrap();
    let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
    let key_store = Arc::new(KeyStore::new());
    let subscriber = tracing_subscriber::registry().with(layer);

    tracing::subscriber::with_default(subscriber, || {
        provider.verify_encryption_mode(&key_store).unwrap();
    });

    let logs_content = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(logs_content.contains("INSECURE: storage encryption is disabled"));
}

#[tokio::test]
async fn test_service_exists_reflects_persistent_db_state() {
    let dir = tempdir().unwrap();
    let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
    let key_store = Arc::new(KeyStore::new());

    assert!(!provider.service_exists("svc-a").await.unwrap());
    let _ = provider.open_service_db("svc-a", &key_store).await.unwrap();
    assert!(provider.service_exists("svc-a").await.unwrap());
}

// -- FDAE watchdog matrix (ADR-0017 §8) --------------------------------
//
// Hand-builds a `CompiledSieve` whose `where_clause` is a pathological
// scalar subquery (mirroring `test_query_raw_bounds_compute_independent_
// of_row_count`'s trick), rather than going through `compile_read` --
// `MAX_RECURSION_DEPTH` bounds any policy-compiled recursive relation to
// 64 steps, far too cheap to ever approach `FDAE_MAX_VM_OPS`. What's
// under test here is `data_db`'s own watchdog *wiring* around the
// sieve, not the compiler.

fn pathological_sieve() -> CompiledSieve {
    CompiledSieve {
        where_clause: "(SELECT 1 FROM (WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x+1 \
                       FROM cnt WHERE x < 2000000000) SELECT x FROM cnt WHERE x >= 2000000000)) \
                       IS NOT NULL"
            .to_string(),
        params: Vec::new(),
        masked_fields: Vec::new(),
        where_caveats: Vec::new(),
        trace: DecisionTrace::default(),
        abac_permissions: Vec::new(),
    }
}

fn seed_one_row_documents(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE documents (id TEXT PRIMARY KEY, payload JSON NOT NULL, creator_id TEXT NOT \
         NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             INSERT INTO documents (id, payload, creator_id, created_at, updated_at) VALUES \
         ('doc-1', '{}', 'c', 0, 0);",
    )
    .unwrap();
}

#[test]
fn fdae_watchdog_interrupts_do_query_as_quota_exceeded() {
    let conn = Connection::open_in_memory().unwrap();
    seed_one_row_documents(&conn);
    let sieve = pathological_sieve();
    let opts = host_store::QueryOptions { filter: None, limit: None, cursor: None };
    let err = do_query(&conn, "documents", &opts, Some(&sieve)).unwrap_err();
    assert!(matches!(err, host_store::DataLayerError::QuotaExceeded));

    // The guard cleared the progress handler on drop -- the connection
    // remains fully usable for an ordinary (unsieved) query afterwards.
    let result = do_query(&conn, "documents", &opts, None).unwrap();
    assert_eq!(result.records.len(), 1);
}

#[test]
fn fdae_watchdog_interrupts_do_get_as_quota_exceeded() {
    let conn = Connection::open_in_memory().unwrap();
    seed_one_row_documents(&conn);
    let sieve = pathological_sieve();
    let err = do_get(&conn, "documents", "doc-1", Some(&sieve)).unwrap_err();
    assert!(matches!(err, host_store::DataLayerError::QuotaExceeded));
    assert!(do_get(&conn, "documents", "doc-1", None).unwrap().is_some());
}

#[test]
fn fdae_watchdog_interrupt_denies_do_check_access() {
    let conn = Connection::open_in_memory().unwrap();
    seed_one_row_documents(&conn);
    let sieve = pathological_sieve();
    // Fail-closed: a watchdog interrupt must surface as `Ok(false)`,
    // never as an `Err` a caller could misread as "allowed".
    assert!(!do_check_access(&conn, "documents", "doc-1", Some(&sieve)).unwrap());
    assert!(do_check_access(&conn, "documents", "doc-1", None).unwrap());
}

/// ADR-0017 §9 decision trace, "rows not reached": `compile_read`
/// cannot know whether a row actually satisfies its compiled
/// predicate -- it only produces SQL. Only
/// `do_check_access`, after running that predicate against a real row,
/// can know. This is the one deny reason that isn't knowable at compile
/// time, so it's tested here (post-execution) rather than in
/// `syneroym-fdae`'s own compile-time decision-trace tests.
#[test]
fn decision_trace_records_rows_not_reached_after_check_access_executes() {
    use std::io;

    use syneroym_ucan::{Capability, ResourceUri, SessionContext};
    use tracing_subscriber::prelude::*;

    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE users (id TEXT PRIMARY KEY, payload TEXT NOT NULL DEFAULT '{}');
             CREATE TABLE documents (id TEXT PRIMARY KEY, payload TEXT NOT NULL DEFAULT '{}');
             INSERT INTO users (id, payload) VALUES ('u-alice', '{\"did\":\"did:key:alice\"}');
             INSERT INTO documents (id, payload) VALUES ('doc-1', \
         '{\"creator_uuid\":\"u-alice\"}');",
    )
    .unwrap();

    let policy = parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "view": {"allows": ["data-layer/read"], "paths": [["creator", "caller"]]}
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#,
        )
        .unwrap();

    let resource =
        ResourceUri(format!("{}/collection/document", ResourceUri::service("svc-a", "svc-a").0));
    // Bob holds a read capability (the operation is admitted) but is
    // not doc-1's creator -- the compiled predicate is a real
    // `EXISTS(...)`, not a compile-time "0=1", so only execution can
    // tell the two apart.
    let bob = SessionContext {
        subject_did: "did:key:bob".to_string(),
        anchor_did: None,
        capabilities: vec![Capability {
            with: resource,
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: None,
        }],
        claims: serde_json::Map::new(),
        verified_at_secs: 0,
    };
    let sieve = compile_read(
        &policy,
        "document",
        &bob,
        "svc-a",
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::PointInTime { id: "doc-1".to_string() },
    )
    .unwrap()
    .unwrap();
    assert!(sieve.trace.operation_admitted);
    assert!(sieve.trace.path_failed.is_none(), "compile time cannot know this row is unreachable");

    let logs = Arc::new(Mutex::new(Vec::new()));
    let logs_clone = logs.clone();
    struct MockWriter {
        logs: Arc<Mutex<Vec<u8>>>,
    }
    impl io::Write for MockWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.logs.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let make_writer = move || MockWriter { logs: logs_clone.clone() };
    let layer = tracing_subscriber::fmt::layer().with_ansi(false).with_writer(make_writer);
    let subscriber = tracing_subscriber::registry().with(layer);

    let allowed = tracing::subscriber::with_default(subscriber, || {
        do_check_access(&conn, "documents", "doc-1", Some(&sieve)).unwrap()
    });
    assert!(!allowed, "bob is not doc-1's creator");

    let logs_content = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(logs_content.contains("fdae decision: deny"), "logs were: {logs_content}");
    assert!(logs_content.contains("rows_reached=Some(false)"), "logs were: {logs_content}");
    assert!(
        logs_content.contains("no row satisfied the compiled predicate"),
        "logs were: {logs_content}"
    );
}

/// Regression: a watchdog interrupt used to be folded into the same
/// `rows_reached: Some(false)` / "no row satisfied the compiled
/// predicate" trace as a genuine empty result, so a compute-budget
/// abort was indistinguishable in the logs from an ordinary deny. The
/// aborted case must stay `rows_reached: None` and get its own
/// `path_failed` reason.
#[test]
fn decision_trace_distinguishes_an_aborted_evaluation_from_a_real_no_row() {
    use std::io;

    use tracing_subscriber::prelude::*;

    let conn = Connection::open_in_memory().unwrap();
    seed_one_row_documents(&conn);
    let sieve = pathological_sieve();

    let logs = Arc::new(Mutex::new(Vec::new()));
    let logs_clone = logs.clone();
    struct MockWriter {
        logs: Arc<Mutex<Vec<u8>>>,
    }
    impl io::Write for MockWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.logs.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let make_writer = move || MockWriter { logs: logs_clone.clone() };
    let layer = tracing_subscriber::fmt::layer().with_ansi(false).with_writer(make_writer);
    let subscriber = tracing_subscriber::registry().with(layer);

    let allowed = tracing::subscriber::with_default(subscriber, || {
        do_check_access(&conn, "documents", "doc-1", Some(&sieve)).unwrap()
    });
    assert!(!allowed, "fail-closed on a watchdog interrupt");

    let logs_content = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(logs_content.contains("fdae decision: deny"), "logs were: {logs_content}");
    assert!(logs_content.contains("policy evaluation aborted"), "logs were: {logs_content}");
    assert!(
        !logs_content.contains("no row satisfied the compiled predicate"),
        "an aborted evaluation must not claim a fact about the data it never reached: logs were: \
         {logs_content}"
    );
    assert!(
        logs_content.contains("rows_reached=None"),
        "an aborted evaluation never learned whether a row matched: logs were: {logs_content}"
    );
}

/// ADR-0017 §9 decision trace: `do_get` is the other Mode A
/// (`PointInTime`) execution path besides `check_access`, and must emit
/// its own execution-aware trace on an unreachable row -- previously
/// only `do_check_access` did, so a denied `get` logged as an
/// undiagnosable "allow".
#[test]
fn decision_trace_records_rows_not_reached_after_do_get_executes() {
    use std::io;

    use syneroym_ucan::{Capability, ResourceUri, SessionContext};
    use tracing_subscriber::prelude::*;

    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE users (id TEXT PRIMARY KEY, payload TEXT NOT NULL DEFAULT '{}');
             CREATE TABLE documents (id TEXT PRIMARY KEY, payload TEXT NOT NULL DEFAULT '{}', \
         creator_id TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             INSERT INTO users (id, payload) VALUES ('u-alice', '{\"did\":\"did:key:alice\"}');
             INSERT INTO documents (id, payload, creator_id, created_at, updated_at) VALUES \
         ('doc-1', '{\"creator_uuid\":\"u-alice\"}', 'svc', 0, 0);",
    )
    .unwrap();

    let policy = parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "view": {"allows": ["data-layer/read"], "paths": [["creator", "caller"]]}
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#,
        )
        .unwrap();

    let resource =
        ResourceUri(format!("{}/collection/document", ResourceUri::service("svc-a", "svc-a").0));
    let bob = SessionContext {
        subject_did: "did:key:bob".to_string(),
        anchor_did: None,
        capabilities: vec![Capability {
            with: resource,
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: None,
        }],
        claims: serde_json::Map::new(),
        verified_at_secs: 0,
    };
    let sieve = compile_read(
        &policy,
        "document",
        &bob,
        "svc-a",
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::PointInTime { id: "doc-1".to_string() },
    )
    .unwrap()
    .unwrap();

    let logs = Arc::new(Mutex::new(Vec::new()));
    let logs_clone = logs.clone();
    struct MockWriter {
        logs: Arc<Mutex<Vec<u8>>>,
    }
    impl io::Write for MockWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.logs.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let make_writer = move || MockWriter { logs: logs_clone.clone() };
    let layer = tracing_subscriber::fmt::layer().with_ansi(false).with_writer(make_writer);
    let subscriber = tracing_subscriber::registry().with(layer);

    let record = tracing::subscriber::with_default(subscriber, || {
        do_get(&conn, "documents", "doc-1", Some(&sieve)).unwrap()
    });
    assert!(record.is_none(), "bob is not doc-1's creator");

    let logs_content = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(logs_content.contains("fdae decision: deny"), "logs were: {logs_content}");
    assert!(logs_content.contains("rows_reached=Some(false)"), "logs were: {logs_content}");
}

/// A successful cross-service fetch's provenance (asserter DID, TTL) is
/// only known once `finalize` folds the real `RemoteFetchTrace` into
/// `sieve.trace` -- after `plan_read`'s own compile-time `trace.emit()`
/// already ran with `remote_fetches: []`. Mode B (`query`) has no other
/// execution-time hook the way Mode A's `check_access`/`get` do, so
/// `emit_mode_b_trace` re-emitting the already-finalized trace is the
/// only place this becomes observable in the logs.
#[test]
fn decision_trace_emits_remote_fetch_provenance_for_a_mode_b_query() {
    use std::io;

    use syneroym_fdae::RemoteFetchTrace;
    use tracing_subscriber::prelude::*;

    let conn = Connection::open_in_memory().unwrap();
    seed_one_row_documents(&conn);

    let sieve = CompiledSieve {
        where_clause: "1=1".to_string(),
        params: Vec::new(),
        masked_fields: Vec::new(),
        where_caveats: Vec::new(),
        trace: DecisionTrace {
            remote_fetches: vec![RemoteFetchTrace {
                service: "hr-svc".to_string(),
                relation: "owner".to_string(),
                principal_did: "did:key:alice".to_string(),
                asserter_did: "did:key:zHrSvc".to_string(),
                valid_until_secs: 42,
            }],
            ..DecisionTrace::default()
        },
        abac_permissions: Vec::new(),
    };

    let logs = Arc::new(Mutex::new(Vec::new()));
    let logs_clone = logs.clone();
    struct MockWriter {
        logs: Arc<Mutex<Vec<u8>>>,
    }
    impl io::Write for MockWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.logs.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let make_writer = move || MockWriter { logs: logs_clone.clone() };
    let layer = tracing_subscriber::fmt::layer().with_ansi(false).with_writer(make_writer);
    let subscriber = tracing_subscriber::registry().with(layer);

    let opts = host_store::QueryOptions { filter: None, limit: None, cursor: None };
    tracing::subscriber::with_default(subscriber, || {
        do_query(&conn, "documents", &opts, Some(&sieve)).unwrap()
    });

    let logs_content = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(logs_content.contains("did:key:zHrSvc"), "logs were: {logs_content}");
    assert!(logs_content.contains("hr-svc"), "logs were: {logs_content}");
}

#[test]
fn fdae_watchdog_interrupts_do_delete_many_on_the_writer_conn() {
    let conn = Connection::open_in_memory().unwrap();
    seed_one_row_documents(&conn);
    let sieve = pathological_sieve();
    let err = do_delete_many(&conn, "documents", None, Some(&sieve)).unwrap_err();
    assert!(matches!(err, host_store::DataLayerError::QuotaExceeded));

    // The writer connection is persistent (never returned to a pool) --
    // the guard clearing the progress handler on drop is what keeps the
    // *next* command on this same connection unaffected.
    let deleted = do_delete_many(&conn, "documents", None, None).unwrap();
    assert_eq!(deleted, 1);
}

/// On the write path, a watchdog interrupt during `row_reachable`'s
/// pre-image check is `Ok(false)`
/// (fail-closed, same contract as `do_check_access`), which
/// `authorize_and_mutate` turns into an ordinary `PermissionDenied` --
/// unlike the read paths above, which surface `QuotaExceeded`
/// distinctly. The mutation itself never runs, and the connection stays
/// usable afterward.
#[test]
fn fdae_watchdog_interrupt_denies_a_write_and_rolls_back() {
    let mut conn = Connection::open_in_memory().unwrap();
    seed_one_row_documents(&conn);
    let sieve = pathological_sieve();
    let err = do_authorized_patch(&mut conn, "documents", "doc-1", br#"{"x":1}"#, Some(&sieve))
        .unwrap_err();
    assert!(matches!(err, host_store::DataLayerError::PermissionDenied));

    let record = do_get(&conn, "documents", "doc-1", None).unwrap().unwrap();
    let payload: Value = serde_json::from_slice(&record.payload).unwrap();
    assert!(payload.get("x").is_none(), "a watchdog-denied write must not have applied");

    // The connection remains fully usable afterward.
    do_authorized_patch(&mut conn, "documents", "doc-1", br#"{"x":1}"#, None).unwrap();
    let record = do_get(&conn, "documents", "doc-1", None).unwrap().unwrap();
    let payload: Value = serde_json::from_slice(&record.payload).unwrap();
    assert_eq!(payload["x"], 1);
}
