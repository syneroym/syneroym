//! Tests for `create` on `ServiceStore`: atomic insertion of multiple rows
//! conditioned on none of their IDs existing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::cognitive_complexity)]

use std::sync::Arc;

use serde_json::json;
use syneroym_data_keystore::KeyStore;
use syneroym_fdae::{Policy, parse_and_validate};
use syneroym_ucan::{Ability, Capability, ResourceUri, SessionContext};
use tempfile::tempdir;

use crate::{
    QueryAuth, ServiceStore, SqliteStorageProvider, StorageProvider,
    host_store::{CollectionSchema, DataLayerError, RecordWriteValue},
};

const SERVICE_ID: &str = "svc-create-test";

async fn setup_store() -> Arc<dyn ServiceStore> {
    let dir = tempdir().unwrap();
    let dir = Box::leak(Box::new(dir));
    let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
    let key_store = Arc::new(KeyStore::new());
    Arc::from(provider.open_service_db(SERVICE_ID, &key_store).await.unwrap())
}

fn plain_schema(name: &str) -> CollectionSchema {
    CollectionSchema { name: name.to_string(), indexes: vec![] }
}

fn write_value(id: &str, payload_json: &str) -> RecordWriteValue {
    RecordWriteValue { id: id.to_string(), payload: payload_json.as_bytes().to_vec() }
}

fn resource(collection: &str) -> ResourceUri {
    ResourceUri(format!(
        "{}/collection/{collection}",
        ResourceUri::service(SERVICE_ID, SERVICE_ID).0
    ))
}

fn write_cap(collection: &str) -> Capability {
    Capability {
        with: resource(collection),
        can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
        caveats: None,
    }
}

fn session(subject_did: &str, capabilities: Vec<Capability>) -> SessionContext {
    SessionContext {
        subject_did: subject_did.to_string(),
        anchor_did: None,
        capabilities,
        claims: serde_json::Map::new(),
        verified_at_secs: 0,
    }
}

fn write_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "manage": {"allows": ["data-layer/write"], "paths": [["creator", "caller"]]}
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

#[tokio::test]
async fn create_inserts_every_row_when_none_exist() {
    let store = setup_store().await;
    store.create_collection(&plain_schema("items")).await.unwrap();

    let values = vec![
        write_value("r1", r#"{"n":1}"#),
        write_value("r2", r#"{"n":2}"#),
        write_value("r3", r#"{"n":3}"#),
    ];
    let res = store.create("items", &values, "creator", None).await.unwrap();
    assert_eq!(res, None);

    for (id, expected_n) in [("r1", 1), ("r2", 2), ("r3", 3)] {
        let got = store.get("items", id, None).await.unwrap().value.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&got.payload).unwrap();
        assert_eq!(payload["n"], expected_n);
    }
}

#[tokio::test]
async fn create_writes_nothing_and_names_the_first_existing_id() {
    let store = setup_store().await;
    store.create_collection(&plain_schema("items")).await.unwrap();

    store.put("items", &write_value("row-1", r#"{"n":100}"#), "creator", None).await.unwrap();

    let values = vec![
        write_value("row-0", r#"{"n":0}"#),
        write_value("row-1", r#"{"n":1}"#),
        write_value("row-2", r#"{"n":2}"#),
    ];
    let res = store.create("items", &values, "creator", None).await.unwrap();
    assert_eq!(res, Some("row-1".to_string()));

    assert!(store.get("items", "row-0", None).await.unwrap().value.is_none());
    assert!(store.get("items", "row-2", None).await.unwrap().value.is_none());
    let r1 = store.get("items", "row-1", None).await.unwrap().value.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&r1.payload).unwrap();
    assert_eq!(payload["n"], 100, "existing row must not be modified");
}

#[tokio::test]
async fn create_refuses_an_id_repeated_inside_one_call() {
    let store = setup_store().await;
    store.create_collection(&plain_schema("items")).await.unwrap();

    let values = vec![
        write_value("row-a", r#"{"n":1}"#),
        write_value("row-b", r#"{"n":2}"#),
        write_value("row-a", r#"{"n":3}"#),
    ];
    let res = store.create("items", &values, "creator", None).await.unwrap();
    assert_eq!(res, Some("row-a".to_string()));

    assert!(store.get("items", "row-a", None).await.unwrap().value.is_none());
    assert!(store.get("items", "row-b", None).await.unwrap().value.is_none());
}

#[tokio::test]
async fn create_of_an_empty_list_is_none() {
    let store = setup_store().await;
    store.create_collection(&plain_schema("items")).await.unwrap();

    let res = store.create("items", &[], "creator", None).await.unwrap();
    assert_eq!(res, None);
}

#[tokio::test]
async fn concurrent_creates_of_one_id_admit_exactly_one() {
    let store = setup_store().await;
    store.create_collection(&plain_schema("items")).await.unwrap();

    let mut handles = Vec::with_capacity(20);
    for i in 0..20 {
        let s = Arc::clone(&store);
        handles.push(tokio::spawn(async move {
            s.create(
                "items",
                &[write_value("unique-id", &format!(r#"{{"task":{i}}}"#))],
                &format!("creator-{i}"),
                None,
            )
            .await
            .unwrap()
        }));
    }

    let mut none_count = 0;
    let mut conflict_count = 0;
    for h in handles {
        match h.await.unwrap() {
            None => none_count += 1,
            Some(id) if id == "unique-id" => conflict_count += 1,
            other => panic!("unexpected create result: {other:?}"),
        }
    }
    assert_eq!(none_count, 1, "exactly one create must succeed");
    assert_eq!(conflict_count, 19, "all other creates must conflict");
}

#[tokio::test]
async fn create_is_denied_and_rolled_back_when_one_row_is_unauthorized() {
    let store = setup_store().await;
    store.create_collection(&plain_schema("users")).await.unwrap();
    store.create_collection(&plain_schema("documents")).await.unwrap();
    store
        .put(
            "users",
            &write_value("u-alice", &json!({"did": "did:key:alice"}).to_string()),
            "svc",
            None,
        )
        .await
        .unwrap();
    store
        .put(
            "users",
            &write_value("u-bob", &json!({"did": "did:key:bob"}).to_string()),
            "svc",
            None,
        )
        .await
        .unwrap();

    let policy = write_policy();
    let alice = session("did:key:alice", vec![write_cap("documents")]);
    let auth = QueryAuth {
        policy: &policy,
        session: &alice,
        service_id: SERVICE_ID,
        resolved_sieve: None,
    };

    let values = vec![
        write_value("doc-alice", &json!({"creator_uuid": "u-alice"}).to_string()),
        // Denied: the post-image is attributed to bob, unreachable to alice.
        write_value("doc-bob", &json!({"creator_uuid": "u-bob"}).to_string()),
    ];

    let err = store.create("documents", &values, "did:key:alice", Some(&auth)).await.unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));

    assert!(
        store.get("documents", "doc-alice", None).await.unwrap().value.is_none(),
        "the first row must have been rolled back"
    );
    assert!(
        store.get("documents", "doc-bob", None).await.unwrap().value.is_none(),
        "the denied row must not have persisted"
    );
}
