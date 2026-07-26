#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Slice B4-fdae end-to-end integration tests: a real `AppSandboxEngine`
//! with the `abac-test` fixture component deployed (ADR-0017 §7), driving
//! `HostState::store::Host` directly (ingress i) so the stage-4 after-step
//! actually executes a real WASM export, not a hand-injected stub. Mirrors
//! `lifecycle_hooks.rs`'s "construct a `HostState` by hand" pattern, with
//! `row_authorizer` wired to the real deployed engine instead of
//! `empty_row_authorizer()`.

use std::{
    fs,
    path::Path,
    sync::{Arc, Weak},
};

use serde_json::json;
use syneroym_core::{
    config::SubstrateConfig, local_registry::EndpointRegistry, storage::MockStorage, test_constants,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, StorageProvider, host_store::RecordWriteValue};
use syneroym_data_keystore::KeyStore;
use syneroym_fdae::{Policy, parse_and_validate};
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::{
    Ability, AuthLevel, CallerContext, Capability, ResourceUri, RowAuthorizer, SessionContext,
};
use syneroym_sandbox_wasm::{AppSandboxEngine, HostState, MessagingContext, StreamContext};
use syneroym_wit_interfaces::{
    control_plane::exports::syneroym::control_plane::orchestrator::{
        ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
    },
    host::syneroym::data_layer::store::{Host as DataLayerHost, QueryOptions},
};

const SERVICE_ID: &str = "abac-test-svc";

/// A single permission opted into the stage-4 after-step (ADR-0017 §7),
/// reachable via `data-layer/read` and terminating in a bare `caller`
/// match against `profiles.creator_uuid` (no join needed) -- mirrors
/// `data_layer_integration.rs`'s `principal_column`-direct shape.
fn abac_policy() -> String {
    r#"{
        "version": "fdae/v1",
        "definitions": {
            "profiles": {
                "table": "profiles",
                "principal_column": "creator_uuid",
                "permissions": {
                    "view": {
                        "allows": ["data-layer/read"],
                        "paths": [["caller"]],
                        "authorize_rows": true
                    }
                }
            }
        }
    }"#
    .to_string()
}

fn test_messaging_context() -> MessagingContext {
    MessagingContext {
        broker: Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
        engine: Weak::new(),
    }
}

fn test_streaming_context() -> StreamContext {
    StreamContext {
        registry: EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
        engine: Weak::new(),
    }
}

async fn make_engine_with_storage(
    dir: &Path,
) -> (Arc<AppSandboxEngine>, Arc<dyn StorageProvider>, Arc<KeyStore>, Arc<dyn BlobProvider>) {
    let mut config = SubstrateConfig {
        app_local_data_dir: dir.join("data"),
        app_data_dir: dir.join("user_data"),
        app_cache_dir: dir.join("cache"),
        app_log_dir: dir.join("logs"),
        profile: "full".to_string(),
        ..SubstrateConfig::default()
    };
    config.resolve_paths();

    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(&config.storage.db_dir, false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));

    let engine = AppSandboxEngine::init(
        &config,
        vec![],
        key_store.clone(),
        storage_provider.clone(),
        blob_provider.clone(),
        Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
        EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
    )
    .await
    .unwrap();
    (Arc::new(engine), storage_provider, key_store, blob_provider)
}

fn wasm_deploy_manifest(bytes: Vec<u8>) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
        },
        service_type: ServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(bytes),
            hash: None,
            interfaces: vec![],
        }),
        registry_certificate: None,
    }
}

fn real_caller(did: &str) -> CallerContext {
    CallerContext {
        caller_did: did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: did.to_string(),
            capabilities: vec![Capability {
                with: ResourceUri::service(SERVICE_ID, SERVICE_ID),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Ucan,
        proof: None,
    }
}

/// Coerces the concrete engine to `Arc<dyn RowAuthorizer>` at this typed
/// `let` (same unsized-coercion pattern `build_store_and_instantiate`
/// itself uses for `self_weak`), then downgrades -- the resulting `Weak`
/// shares the same underlying allocation as `engine`, so it upgrades
/// successfully as long as `engine` (or any clone) is still alive.
fn row_authorizer_for(engine: &Arc<AppSandboxEngine>) -> Weak<dyn RowAuthorizer> {
    let trait_object: Arc<dyn RowAuthorizer> = engine.clone();
    Arc::downgrade(&trait_object)
}

#[allow(clippy::too_many_arguments)]
fn build_host_state(
    key_store: Arc<KeyStore>,
    storage_provider: Arc<dyn StorageProvider>,
    blob_provider: Arc<dyn BlobProvider>,
    caller: CallerContext,
    config_generation: u64,
    policy: Arc<Policy>,
    row_authorizer: Weak<dyn RowAuthorizer>,
) -> HostState {
    HostState::new(
        SERVICE_ID.to_string(),
        None,
        key_store,
        storage_provider,
        blob_provider,
        caller,
        config_generation,
        test_messaging_context(),
        test_streaming_context(),
        syneroym_sandbox_wasm::empty_service_proxy(),
        Some(policy),
        false,
        row_authorizer,
    )
}

struct Deployed {
    engine: Arc<AppSandboxEngine>,
    storage_provider: Arc<dyn StorageProvider>,
    key_store: Arc<KeyStore>,
    blob_provider: Arc<dyn BlobProvider>,
    policy: Arc<Policy>,
    config_generation: u64,
}

/// Deploys the fixture under `mode`, seeds `profiles` with two rows (one
/// owned by `did:key:zAlice`, one by a different principal), and returns
/// everything a test needs to drive `HostState::store::Host` as Alice.
/// `None` when the WASM artifact hasn't been built.
async fn deploy_with_mode(dir: &Path, mode: &str) -> Option<Deployed> {
    let wasm_bytes = fs::read(test_constants::abac_test_wasm_path()).ok()?;
    let (engine, storage_provider, key_store, blob_provider) = make_engine_with_storage(dir).await;

    storage_provider.save_fdae_policy(SERVICE_ID, &abac_policy()).await.unwrap();
    let config_generation = storage_provider
        .save_config_generation(SERVICE_ID, &json!({ "mode": mode }).to_string())
        .await
        .unwrap();

    let manifest = wasm_deploy_manifest(wasm_bytes);
    engine.deploy_wasm(SERVICE_ID, &manifest).await.unwrap();

    let store = storage_provider.open_service_db(SERVICE_ID, &key_store).await.unwrap();
    store
        .put(
            "profiles",
            &RecordWriteValue {
                id: "owned-by-alice".to_string(),
                payload: json!({
                    "creator_uuid": "did:key:zAlice",
                    "classification": "secret",
                    "ssn": "111-22-3333"
                })
                .to_string()
                .into_bytes(),
            },
            SERVICE_ID,
        )
        .await
        .unwrap();
    store
        .put(
            "profiles",
            &RecordWriteValue {
                id: "owned-by-someone-else".to_string(),
                payload: json!({"creator_uuid": "did:key:zBob"}).to_string().into_bytes(),
            },
            SERVICE_ID,
        )
        .await
        .unwrap();
    drop(store);

    let policy = Arc::new(parse_and_validate(&abac_policy()).unwrap());
    Some(Deployed { engine, storage_provider, key_store, blob_provider, policy, config_generation })
}

macro_rules! skip_if_missing_artifact {
    ($test_name:expr) => {
        eprintln!(
            "Skipping {}: abac-test WASM artifact not found (run `cargo component build --release \
             --target wasm32-wasip2` in test-components/abac-test)",
            $test_name
        );
        return;
    };
}

/// Row 7 (end to end, real export): the fixture's `deny_by_field` mode
/// denies the row marked `classification: "secret"` -- proving the
/// after-step's decision actually reaches a real `authorize-rows` export,
/// not just the sieve's own filtering (both seeded rows pass the sieve;
/// only the after-step can distinguish them by payload content).
#[tokio::test]
async fn stage4_denies_rows_the_guest_rejects_for_a_real_caller() {
    let dir = tempfile::tempdir().unwrap();
    let Some(d) = deploy_with_mode(dir.path(), "deny_by_field").await else {
        skip_if_missing_artifact!("stage4_denies_rows_the_guest_rejects_for_a_real_caller");
    };

    // Both of Alice's own rows aren't seeded here -- only one row is hers
    // (the "secret" one); this test only needs to prove that row gets
    // denied by the after-step despite passing the sieve.
    let row_authorizer = row_authorizer_for(&d.engine);
    let mut host = build_host_state(
        d.key_store,
        d.storage_provider,
        d.blob_provider,
        real_caller("did:key:zAlice"),
        d.config_generation,
        d.policy,
        row_authorizer,
    );

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let result = DataLayerHost::query(&mut host, "profiles".to_string(), opts).await.unwrap();
    assert!(
        result.records.is_empty(),
        "the after-step must deny Alice's own 'secret' row even though the sieve admits it: \
         {result:?}"
    );
}

/// Row 7's structural guard, end to end: even an `allow`-everything
/// after-step cannot surface a row the sieve already excluded (Bob's row).
/// The fixture's default mode (no `mode` key configured) is `allow_all`.
#[tokio::test]
async fn stage4_cannot_admit_a_row_the_sieve_excluded() {
    let dir = tempfile::tempdir().unwrap();
    let Some(d) = deploy_with_mode(dir.path(), "allow_all").await else {
        skip_if_missing_artifact!("stage4_cannot_admit_a_row_the_sieve_excluded");
    };

    let row_authorizer = row_authorizer_for(&d.engine);
    let mut host = build_host_state(
        d.key_store,
        d.storage_provider,
        d.blob_provider,
        real_caller("did:key:zAlice"),
        d.config_generation,
        d.policy,
        row_authorizer,
    );

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let result = DataLayerHost::query(&mut host, "profiles".to_string(), opts).await.unwrap();
    let ids: Vec<&str> = result.records.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["owned-by-alice"],
        "an allow-everything after-step must still never see Bob's row -- the sieve excluded it \
         before the after-step ever ran: {result:?}"
    );
}

/// Row 9: `redact` mode strips `ssn` from the surviving row before it
/// reaches the caller.
#[tokio::test]
async fn stage4_redact_removes_the_named_field_before_the_guest_sees_it() {
    let dir = tempfile::tempdir().unwrap();
    let Some(d) = deploy_with_mode(dir.path(), "redact").await else {
        skip_if_missing_artifact!("stage4_redact_removes_the_named_field_before_the_guest_sees_it");
    };

    let row_authorizer = row_authorizer_for(&d.engine);
    let mut host = build_host_state(
        d.key_store,
        d.storage_provider,
        d.blob_provider,
        real_caller("did:key:zAlice"),
        d.config_generation,
        d.policy,
        row_authorizer,
    );

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let result = DataLayerHost::query(&mut host, "profiles".to_string(), opts).await.unwrap();
    assert_eq!(result.records.len(), 1, "the row itself is still reachable, only redacted");
    let payload: serde_json::Value = serde_json::from_slice(&result.records[0].payload).unwrap();
    assert!(payload.get("ssn").is_none(), "ssn must be stripped: {payload:?}");
    assert!(
        payload.get("classification").is_some(),
        "sibling fields must survive the redact: {payload:?}"
    );
}

/// Row 8: the `spin` mode's unbounded loop exceeds the after-step's
/// fuel/epoch budget -- `apply_stage4` maps that to a fail-closed `Err`,
/// which the ingress maps to an empty page, never a partial or unfiltered
/// one.
#[tokio::test]
async fn stage4_fuel_exhaustion_denies_the_whole_batch() {
    let dir = tempfile::tempdir().unwrap();
    let Some(d) = deploy_with_mode(dir.path(), "spin").await else {
        skip_if_missing_artifact!("stage4_fuel_exhaustion_denies_the_whole_batch");
    };

    let row_authorizer = row_authorizer_for(&d.engine);
    let mut host = build_host_state(
        d.key_store,
        d.storage_provider,
        d.blob_provider,
        real_caller("did:key:zAlice"),
        d.config_generation,
        d.policy,
        row_authorizer,
    );

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let result = DataLayerHost::query(&mut host, "profiles".to_string(), opts).await.unwrap();
    assert!(result.records.is_empty(), "a budget-exceeded after-step must deny closed: {result:?}");
}

/// Read-only enforcement (D-B4-2): the after-step instance's own write
/// attempt is hard-denied by `HostState.read_only`, regardless of what the
/// guest decides to return -- asserted by checking storage directly, not
/// by trusting the guest's own report of what happened.
#[tokio::test]
async fn stage4_instance_cannot_write() {
    let dir = tempfile::tempdir().unwrap();
    let Some(d) = deploy_with_mode(dir.path(), "write_attempt").await else {
        skip_if_missing_artifact!("stage4_instance_cannot_write");
    };

    let row_authorizer = row_authorizer_for(&d.engine);
    let mut host = build_host_state(
        d.key_store.clone(),
        d.storage_provider.clone(),
        d.blob_provider,
        real_caller("did:key:zAlice"),
        d.config_generation,
        d.policy,
        row_authorizer,
    );

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let _ = DataLayerHost::query(&mut host, "profiles".to_string(), opts).await.unwrap();

    let store = d.storage_provider.open_service_db(SERVICE_ID, &d.key_store).await.unwrap();
    let written = store.get("profiles", "guest-write-attempt", None).await.unwrap();
    assert!(
        written.value.is_none(),
        "the after-step's write attempt must never reach storage: {written:?}"
    );
}

/// ADR-0017 §7's read-only lookup escape hatch: the after-step reads this
/// service's own `lookup_targets` collection (seeded by `init()`)
/// unfiltered -- no `QueryAuth` reaches a `LocalReadOnly` caller
/// (`resolve_query_auth`'s exemption) -- to decide every row in the batch.
/// A successful, non-hanging completion also demonstrates D-B4-4's
/// structural recursion bound: if the exemption didn't hold, this nested
/// read would itself carry a sieve and re-enter `authorize_rows`.
#[tokio::test]
async fn stage4_lookup_sees_its_own_service_data() {
    let dir = tempfile::tempdir().unwrap();
    let Some(d) = deploy_with_mode(dir.path(), "lookup").await else {
        skip_if_missing_artifact!("stage4_lookup_sees_its_own_service_data");
    };

    let row_authorizer = row_authorizer_for(&d.engine);
    let mut host = build_host_state(
        d.key_store,
        d.storage_provider,
        d.blob_provider,
        real_caller("did:key:zAlice"),
        d.config_generation,
        d.policy,
        row_authorizer,
    );

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let result = DataLayerHost::query(&mut host, "profiles".to_string(), opts).await.unwrap();
    assert_eq!(
        result.records.len(),
        1,
        "the lookup mode's read-only nested lookup found `lookup_targets/target-1` (seeded by \
         init()) and allowed accordingly: {result:?}"
    );
}

/// Row 7's structural guard: `bad_arity` mode returns one fewer decision
/// than rows, which `apply_stage4` must treat as a whole-batch deny.
#[tokio::test]
async fn stage4_bad_arity_denies_the_whole_batch() {
    let dir = tempfile::tempdir().unwrap();
    let Some(d) = deploy_with_mode(dir.path(), "bad_arity").await else {
        skip_if_missing_artifact!("stage4_bad_arity_denies_the_whole_batch");
    };

    let row_authorizer = row_authorizer_for(&d.engine);
    let mut host = build_host_state(
        d.key_store,
        d.storage_provider,
        d.blob_provider,
        real_caller("did:key:zAlice"),
        d.config_generation,
        d.policy,
        row_authorizer,
    );

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let result = DataLayerHost::query(&mut host, "profiles".to_string(), opts).await.unwrap();
    assert!(result.records.is_empty(), "an arity-mismatched decision list must deny closed");
}

/// `check_access` (Mode A) takes the same stage-4 substitution D-B4-3(b)
/// specifies: it runs `get` under the point-in-time sieve and asks the
/// after-step the same question, rather than the plain existence check.
#[tokio::test]
async fn stage4_check_access_runs_the_after_step_too() {
    let dir = tempfile::tempdir().unwrap();
    let Some(d) = deploy_with_mode(dir.path(), "deny_by_field").await else {
        skip_if_missing_artifact!("stage4_check_access_runs_the_after_step_too");
    };

    let row_authorizer = row_authorizer_for(&d.engine);
    let mut host = build_host_state(
        d.key_store,
        d.storage_provider,
        d.blob_provider,
        real_caller("did:key:zAlice"),
        d.config_generation,
        d.policy,
        row_authorizer,
    );

    let allowed = DataLayerHost::check_access(
        &mut host,
        "profiles".to_string(),
        "owned-by-alice".to_string(),
        Ability::DATA_LAYER_READ.to_string(),
    )
    .await
    .unwrap();
    assert!(
        !allowed,
        "check_access must run the after-step too, denying Alice's own 'secret' row exactly like \
         `query` does"
    );
}
