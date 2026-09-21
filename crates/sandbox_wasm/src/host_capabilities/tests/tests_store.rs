#![allow(clippy::too_many_lines, clippy::cognitive_complexity)]

use std::sync::Mutex;

use serde_json::json;
use syneroym_data_db::SqliteStorageProvider;
use syneroym_fdae::parse_and_validate;
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::{Capability, RelationshipProof, SessionContext};

use super::*;

// -- FDAE host wiring --------------------------------------------------
//
// Real `QueryAuth` construction from `HostState.fdae_policy`/`caller`,
// `check-access`, and host-side CLS field-stripping, exercised through
// `store::Host` on a `HostState` built with a hand-injected `Policy`
// (`fdae_policy` is `None` for a service with no stored policy).

const FDAE_SERVICE_ID: &str = "svc-fdae-host-test";

fn fdae_resource(collection: &str) -> ResourceUri {
    ResourceUri(format!(
        "{}/collection/{collection}",
        ResourceUri::service(FDAE_SERVICE_ID, FDAE_SERVICE_ID).0
    ))
}

fn fdae_read_cap(collection: &str) -> Capability {
    Capability {
        with: fdae_resource(collection),
        can: Ability(Ability::DATA_LAYER_READ.to_string()),
        caveats: None,
    }
}

fn fdae_caller(subject_did: &str, capabilities: Vec<Capability>) -> CallerContext {
    CallerContext {
        caller_did: subject_did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: subject_did.to_string(),
            capabilities,
            ..Default::default()
        },
        auth: AuthLevel::Ucan,
        proof: None,
    }
}

/// `document` --creator--> `user`, `view` permission reachable only via
/// the creator relation. Mirrors `data_db::tests_fdae::single_hop_policy`.
fn fdae_single_hop_policy() -> Policy {
    parse_and_validate(
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
    .unwrap()
}

/// Same shape as `fdae_single_hop_policy`, plus a CLS `fields.deny:
/// ["ssn"]`.
fn fdae_cls_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "paths": [["creator", "caller"]],
                            "fields": {"deny": ["ssn"]}
                        }
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

/// A `manage` permission covering `data-layer/write`, reachable via the
/// same creator relation -- used to exercise `delete_many`'s write-mode
/// sieve. Mirrors `data_db::tests_fdae::write_policy`.
fn fdae_write_policy() -> Policy {
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

fn fdae_write_cap(collection: &str) -> Capability {
    Capability {
        with: fdae_resource(collection),
        can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
        caveats: None,
    }
}

fn fdae_host_state(
    storage_provider: Arc<dyn StorageProvider>,
    caller: CallerContext,
    fdae_policy: Option<Arc<Policy>>,
) -> HostState {
    HostState::new(
        FDAE_SERVICE_ID.to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage_provider,
        test_blob_provider(),
        caller,
        0,
        test_messaging_context(),
        test_streaming_context(),
        test_service_proxy(),
        fdae_policy,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    )
}

/// Seeds `users`/`documents` collections: `doc-1` created by alice,
/// `doc-2` created by bob, both carrying an `ssn` field for the CLS
/// tests. Uses a policy-absent `HostState` (`put`/`create_collection`
/// carry no FDAE gate).
async fn fdae_seed_documents(storage_provider: Arc<dyn StorageProvider>) {
    let mut seeder =
        fdae_host_state(storage_provider, CallerContext::service_system(FDAE_SERVICE_ID), None);
    store::Host::create_collection(
        &mut seeder,
        CollectionSchema { name: "users".to_string(), indexes: vec![] },
    )
    .await
    .unwrap();
    store::Host::create_collection(
        &mut seeder,
        CollectionSchema { name: "documents".to_string(), indexes: vec![] },
    )
    .await
    .unwrap();
    store::Host::put(
        &mut seeder,
        "users".to_string(),
        RecordWriteValue {
            id: "u-alice".to_string(),
            payload: json!({"did": "did:key:alice"}).to_string().into_bytes(),
        },
    )
    .await
    .unwrap();
    store::Host::put(
        &mut seeder,
        "users".to_string(),
        RecordWriteValue {
            id: "u-bob".to_string(),
            payload: json!({"did": "did:key:bob"}).to_string().into_bytes(),
        },
    )
    .await
    .unwrap();
    store::Host::put(
        &mut seeder,
        "documents".to_string(),
        RecordWriteValue {
            id: "doc-1".to_string(),
            payload: json!({"creator_uuid": "u-alice", "ssn": "111-11-1111"})
                .to_string()
                .into_bytes(),
        },
    )
    .await
    .unwrap();
    store::Host::put(
        &mut seeder,
        "documents".to_string(),
        RecordWriteValue {
            id: "doc-2".to_string(),
            payload: json!({"creator_uuid": "u-bob", "ssn": "222-22-2222"})
                .to_string()
                .into_bytes(),
        },
    )
    .await
    .unwrap();
}

fn payload_json(record: &RecordReadValue) -> Value {
    serde_json::from_slice(&record.payload).unwrap()
}

/// RLS: `get`/`query` return only alice's own reachable row, and
/// `check_access` matches (reachable -> `true`, unreachable -> `false`).
#[tokio::test]
async fn fdae_rls_filters_get_query_and_check_access() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;

    let policy = Arc::new(fdae_single_hop_policy());
    let alice = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
    let mut host = fdae_host_state(storage_provider, alice, Some(policy));

    let own =
        store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string()).await.unwrap();
    assert!(own.is_some(), "alice's own document must be reachable");
    let other =
        store::Host::get(&mut host, "documents".to_string(), "doc-2".to_string()).await.unwrap();
    assert!(other.is_none(), "bob's document is unreachable, not an error (ADR-0007)");

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let result = store::Host::query(&mut host, "documents".to_string(), opts).await.unwrap();
    let ids: Vec<_> = result.records.iter().map(|r| r.id.clone()).collect();
    assert_eq!(ids, vec!["doc-1"], "bob's document must be excluded from query results");

    assert!(
        store::Host::check_access(
            &mut host,
            "documents".to_string(),
            "doc-1".to_string(),
            Ability::DATA_LAYER_READ.to_string(),
        )
        .await
        .unwrap(),
        "check_access must allow alice's own reachable row"
    );
    assert!(
        !store::Host::check_access(
            &mut host,
            "documents".to_string(),
            "doc-2".to_string(),
            Ability::DATA_LAYER_READ.to_string(),
        )
        .await
        .unwrap(),
        "check_access must deny bob's unreachable row"
    );
}

/// CLS: a policy with `fields.deny: ["ssn"]` strips `ssn` from the
/// payload returned by both `get` and `query` -- host-side projection
/// means a masked value is never returned to the caller.
#[tokio::test]
async fn fdae_cls_strips_masked_field_from_get_and_query() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;

    let policy = Arc::new(fdae_cls_policy());
    let alice = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
    let mut host = fdae_host_state(storage_provider, alice, Some(policy));

    let own = store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
        .await
        .unwrap()
        .unwrap();
    let payload = payload_json(&own);
    assert!(payload.get("ssn").is_none(), "ssn must be stripped from get's payload");
    assert_eq!(payload.get("creator_uuid").and_then(Value::as_str), Some("u-alice"));

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let result = store::Host::query(&mut host, "documents".to_string(), opts).await.unwrap();
    assert_eq!(result.records.len(), 1);
    let payload = payload_json(&result.records[0]);
    assert!(payload.get("ssn").is_none(), "ssn must be stripped from query's payload");
}

/// Pass-through: `fdae_policy: None` leaves rows and payloads unchanged
/// -- zero behavior change on the unconfigured (today's production)
/// path.
#[tokio::test]
async fn fdae_policy_absent_is_unfiltered_pass_through() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;

    let caller = CallerContext::service_system(FDAE_SERVICE_ID);
    let mut host = fdae_host_state(storage_provider, caller, None);

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let result = store::Host::query(&mut host, "documents".to_string(), opts).await.unwrap();
    assert_eq!(result.records.len(), 2, "no policy means both rows are visible");
    for record in &result.records {
        assert!(
            payload_json(record).get("ssn").is_some(),
            "no policy means no CLS strip -- ssn must survive untouched"
        );
    }
}

/// Lifecycle-hook reads (`init`/`migrate`, which run as
/// `CallerContext::local_elevated`) must stay unfiltered even under a
/// deployed policy. Without `query_auth`'s `LocalElevated` exemption,
/// `local_elevated`'s `data-layer/admin` capability entails
/// `data-layer/read` and covers every collection, so `compile_read`
/// compiles a *real* sieve here -- bound to
/// `"system:local-elevated:<service_id>"`, a DID no principal row can
/// ever hold -- and both documents would silently vanish. A migration
/// that reads its own data to decide how to rewrite it would act on
/// that emptiness instead of erroring.
#[tokio::test]
async fn fdae_local_elevated_lifecycle_reads_stay_unfiltered_under_a_policy() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;

    let policy = Arc::new(fdae_single_hop_policy());
    let caller = CallerContext::local_elevated(FDAE_SERVICE_ID);
    let mut host = fdae_host_state(storage_provider, caller, Some(policy));

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let result = store::Host::query(&mut host, "documents".to_string(), opts).await.unwrap();
    assert_eq!(
        result.records.len(),
        2,
        "a lifecycle hook must see every row regardless of the deployed policy"
    );

    let doc =
        store::Host::get(&mut host, "documents".to_string(), "doc-2".to_string()).await.unwrap();
    assert!(
        doc.is_some(),
        "get during init/migrate must not be sieved against the synthesized local-elevated \
         identity"
    );
}

/// `aggregate` is row-filtered through the host layer identically to
/// `get`/`query` -- covers the `store::Host::aggregate` wiring seam this
/// phase adds, which no host test previously exercised with a real
/// `Some(policy)` (a dropped or `None`-replaced `query_auth()` call here
/// would have passed every prior test).
#[tokio::test]
async fn fdae_aggregate_is_row_filtered_through_host() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;

    let policy = Arc::new(fdae_single_hop_policy());
    let alice = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
    let mut host = fdae_host_state(storage_provider, alice, Some(policy));

    let result = store::Host::aggregate(
        &mut host,
        "documents".to_string(),
        r#"{"$group":{"_id":null,"n":{"$sum":1}}}"#.to_string(),
    )
    .await
    .unwrap();
    // `SqlValue` doesn't derive `PartialEq` -- compare via its
    // already-derived `Serialize` impl.
    assert_eq!(
        serde_json::to_value(&result.rows).unwrap(),
        serde_json::to_value(vec![vec![SqlValue::Integer(1)]]).unwrap(),
        "only alice's own doc-1 is counted"
    );
}

/// `delete_many` is filtered as a write operation through the host layer
/// -- same wiring-seam coverage gap as `aggregate` above.
#[tokio::test]
async fn fdae_delete_many_is_write_filtered_through_host() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;
    let policy = Arc::new(fdae_write_policy());

    // A read-only capability must not satisfy the write-mode sieve.
    let alice_read_only = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
    let mut host_ro =
        fdae_host_state(storage_provider.clone(), alice_read_only, Some(policy.clone()));
    let deleted = store::Host::delete_many(&mut host_ro, "documents".to_string(), String::new())
        .await
        .unwrap();
    assert_eq!(deleted, 0, "a read-only capability must not delete anything");

    // A write capability deletes only alice's own row.
    let alice_write = fdae_caller("did:key:alice", vec![fdae_write_cap("documents")]);
    let mut host_rw = fdae_host_state(storage_provider, alice_write, Some(policy));
    let deleted = store::Host::delete_many(&mut host_rw, "documents".to_string(), String::new())
        .await
        .unwrap();
    assert_eq!(deleted, 1, "only alice's own document is deletable");
}

/// Through the `store::Host` guest boundary: a
/// write-capable caller who cannot reach a row via the compiled sieve
/// is denied `put`/`patch`/`delete` on it; the same caller against a
/// row they do reach succeeds.
#[tokio::test]
async fn fdae_put_patch_delete_deny_an_unreachable_row_and_allow_a_reachable_one() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;
    let policy = Arc::new(fdae_write_policy());
    let alice = fdae_caller("did:key:alice", vec![fdae_write_cap("documents")]);
    let mut host = fdae_host_state(storage_provider, alice, Some(policy));

    // doc-2 belongs to bob -- unreachable to alice under the write sieve.
    let err = store::Host::patch(
        &mut host,
        "documents".to_string(),
        "doc-2".to_string(),
        br#"{"x":1}"#.to_vec(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));

    let err = store::Host::delete(&mut host, "documents".to_string(), "doc-2".to_string())
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));

    let err = store::Host::put(
        &mut host,
        "documents".to_string(),
        RecordWriteValue {
            id: "doc-2".to_string(),
            payload: json!({"creator_uuid": "u-bob", "hijacked": true}).to_string().into_bytes(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));

    // doc-1 belongs to alice -- reachable.
    store::Host::patch(
        &mut host,
        "documents".to_string(),
        "doc-1".to_string(),
        br#"{"nickname":"al"}"#.to_vec(),
    )
    .await
    .unwrap();
    let record = store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(payload_json(&record)["nickname"], "al");

    store::Host::delete(&mut host, "documents".to_string(), "doc-1".to_string()).await.unwrap();
    assert!(
        store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
            .await
            .unwrap()
            .is_none()
    );
}

/// `drop_collection` bypasses any per-row policy on the collection
/// entirely, so it must not be reachable through an ordinary write
/// capability: a caller holding only `data-layer/write` on `documents`
/// (able to `put`/`patch`/`delete` rows it can individually reach) is
/// denied `drop_collection("documents")` outright; a caller holding
/// `data-layer/admin` on the service succeeds.
#[tokio::test]
async fn drop_collection_requires_admin_not_an_ordinary_write_capability() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;

    let writer = fdae_caller("did:key:alice", vec![fdae_write_cap("documents")]);
    let mut host = fdae_host_state(storage_provider.clone(), writer, None);
    let err = store::Host::drop_collection(&mut host, "documents".to_string()).await.unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
    assert!(
        store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
            .await
            .unwrap()
            .is_some(),
        "a denied drop_collection must leave the collection intact"
    );

    let admin = fdae_caller(
        "did:key:admin",
        vec![Capability {
            with: ResourceUri::service(FDAE_SERVICE_ID, FDAE_SERVICE_ID),
            can: Ability(Ability::DATA_LAYER_ADMIN.to_string()),
            caveats: None,
        }],
    );
    let mut host = fdae_host_state(storage_provider, admin, None);
    store::Host::drop_collection(&mut host, "documents".to_string()).await.unwrap();
}

/// **Extra-capability CLS-narrowing pin.** The same "an extra
/// capability shouldn't narrow" defect pinned for RLS (a caveated
/// second capability narrowing the result to zero rows) applies to
/// CLS `fields.deny` union across capabilities too. Alice holds both
/// an unrestricted `read` capability and a second `read` capability
/// caveated `fields.deny: ["ssn"]` on the same resource; today's
/// `compile_cls` unions every entitling capability's deny-list, so
/// even the unrestricted grant's payload comes back stripped. When
/// this defect is fixed, this assertion should flip to `ssn` being
/// **present** (the unrestricted capability's caveat-free access
/// should win).
#[tokio::test]
async fn fdae_d04_02_g_extra_caveated_capability_narrows_cls_strip() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;

    // `fdae_single_hop_policy` carries no policy-level `fields.deny` --
    // the mask below comes entirely from the second capability's caveat.
    let policy = Arc::new(fdae_single_hop_policy());
    let unrestricted_cap = fdae_read_cap("documents");
    let ssn_deny_cap = Capability {
        with: fdae_resource("documents"),
        can: Ability(Ability::DATA_LAYER_READ.to_string()),
        caveats: Some(json!({"fields": {"deny": ["ssn"]}})),
    };
    let alice = fdae_caller("did:key:alice", vec![unrestricted_cap, ssn_deny_cap]);
    let mut host = fdae_host_state(storage_provider, alice, Some(policy));

    let own = store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
        .await
        .unwrap()
        .unwrap();
    let payload = payload_json(&own);
    assert!(
        payload.get("ssn").is_none(),
        "D-04-02-g: today, the caveated capability's fields.deny narrows the unrestricted \
         capability's access too, so ssn is stripped even though the unrestricted grant alone \
         should expose it. If this assertion starts failing, D-04-02-g has been fixed -- update \
         this test to assert ssn IS present."
    );
}

// -- Cross-service relationship-proof fetch, wired
// through `HostState::resolve_query_auth` --------------------------

fn fdae_remote_relation_policy(expected_asserter_did: &str) -> Policy {
    parse_and_validate(&format!(
        r#"{{
            "version": "fdae/v1",
            "definitions": {{
                "document": {{
                    "table": "documents",
                    "relations": {{"owner": {{
                        "target": "employee", "service": "hr-svc",
                        "join_column": "owner_uuid",
                        "expected_asserter_did": "{expected_asserter_did}"
                    }}}},
                    "permissions": {{
                        "view": {{"allows": ["data-layer/read"], "paths": [["owner", "anchor"]]}}
                    }}
                }}
            }}
        }}"#
    ))
    .unwrap()
}

#[derive(Debug)]
struct StubProxy(Mutex<Option<Result<Value, RpcProxyError>>>);

#[async_trait::async_trait]
impl ServiceProxy for StubProxy {
    async fn invoke(&self, _request: ProxyRequest) -> Result<Value, RpcProxyError> {
        self.0.lock().unwrap().take().expect("StubProxy invoked with no response configured")
    }
}

async fn seed_one_remote_owned_document(storage_provider: Arc<dyn StorageProvider>) {
    let mut seeder =
        fdae_host_state(storage_provider, CallerContext::service_system(FDAE_SERVICE_ID), None);
    store::Host::create_collection(
        &mut seeder,
        CollectionSchema { name: "documents".to_string(), indexes: vec![] },
    )
    .await
    .unwrap();
    store::Host::put(
        &mut seeder,
        "documents".to_string(),
        RecordWriteValue {
            id: "doc-1".to_string(),
            payload: json!({"owner_uuid": "emp-alice"}).to_string().into_bytes(),
        },
    )
    .await
    .unwrap();
}

/// A policy naming a remote relation resolves through `resolve_query_auth`:
/// `get` reaches `HostState.service_proxy`, verifies the returned
/// `RelationshipProof` against the policy's `expected_asserter_did`, and
/// the finalized sieve correctly admits alice's own document.
#[tokio::test]
async fn fdae_remote_relation_fetch_succeeds_through_host_state() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    seed_one_remote_owned_document(storage_provider.clone()).await;

    let identity = Identity::generate().unwrap();
    let asserter_did = substrate::derive_did_key(&identity.public_key());
    let proof = RelationshipProof::sign(
        &identity,
        None,
        "employee",
        "did:key:alice",
        vec!["emp-alice".to_string()],
    )
    .unwrap();
    let stub: Arc<dyn ServiceProxy> =
        Arc::new(StubProxy(Mutex::new(Some(Ok(serde_json::to_value(&proof).unwrap())))));

    let policy = Arc::new(fdae_remote_relation_policy(&asserter_did));
    let alice = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
    let mut host = HostState::new(
        FDAE_SERVICE_ID.to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage_provider,
        test_blob_provider(),
        alice,
        0,
        test_messaging_context(),
        test_streaming_context(),
        Arc::downgrade(&stub),
        Some(policy),
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    );

    let own =
        store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string()).await.unwrap();
    assert!(own.is_some(), "alice's document must resolve through the real cross-service fetch");
}

/// A fetch failure (the remote proxy call errors) denies the whole read
/// closed rather than falling back to unfiltered or silently empty --
/// `get` must surface an `Err`, not `Ok(None)` masquerading as "not
/// found."
#[tokio::test]
async fn fdae_remote_relation_fetch_failure_denies_closed() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    seed_one_remote_owned_document(storage_provider.clone()).await;

    let stub: Arc<dyn ServiceProxy> =
        Arc::new(StubProxy(Mutex::new(Some(Err(RpcProxyError::Timeout(Duration::from_secs(5)))))));

    let policy = Arc::new(fdae_remote_relation_policy("did:key:zSomeAsserter"));
    let alice = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
    let mut host = HostState::new(
        FDAE_SERVICE_ID.to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage_provider,
        test_blob_provider(),
        alice,
        0,
        test_messaging_context(),
        test_streaming_context(),
        Arc::downgrade(&stub),
        Some(policy),
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    );

    let err = store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
}
