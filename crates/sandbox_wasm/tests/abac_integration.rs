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
use syneroym_data_db::{
    SqliteStorageProvider, StorageProvider,
    host_store::{CollectionSchema, RecordWriteValue},
};
use syneroym_data_keystore::KeyStore;
use syneroym_fdae::{Policy, parse_and_validate};
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::{
    Ability, AuthLevel, CallerContext, Capability, ResourceUri, RowAuthorizer, SessionContext,
};
use syneroym_sandbox_wasm::{
    AppSandboxEngine, HostState, MessagingContext, StreamContext, empty_service_proxy,
};
use syneroym_wit_interfaces::{
    control_plane::exports::syneroym::control_plane::orchestrator::{
        ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
    },
    host::syneroym::data_layer::store::{DataLayerError, Host as DataLayerHost, QueryOptions},
};

const SERVICE_ID: &str = "abac-test-svc";

/// A single permission opted into the stage-4 after-step (ADR-0017 §7),
/// reachable via `data-layer/read` and terminating in a bare `caller`
/// match against `profiles.creator_uuid` (no join needed) -- mirrors
/// `data_layer_integration.rs`'s `principal_column`-direct shape.
///
/// `lookup_targets` is also stage-4-gated, via an unconditionally public
/// permission (`paths: []`, no `principal_column` needed):
/// `stage4_nested_read_does_not_re_enter_the_after_step` (review finding
/// B4-03) needs a nested read that reliably returns rows *today*, under the
/// intact `LocalReadOnly` exemption, so a regression in that exemption
/// flips the fixture's decision and fails the test's outer assertion.
/// `paths: []` only makes the row-level predicate unconditional; it does
/// **not** bypass `applicable_permissions`' capability gate (review residual
/// R1, `compile.rs::applicable_permissions`), which requires a *held
/// capability* before any permission -- public or not -- becomes
/// applicable. `CallerContext::service_abac` is deliberately capability-less
/// (D-B4-2), so if the exemption were ever narrowed, this nested read would
/// compile `deny_all()` (zero rows, deny-closed), not re-enter
/// `authorize_rows` -- the capability gate independently blocks true
/// recursion regardless of this exemption. See the test's own doc comment
/// for what the test does and doesn't prove.
///
/// `lookup_targets` also carries a `fields.deny` (CLS), unlike `profiles`,
/// so `stage4_cls_mask_unions_with_the_after_step_redact_set` (review
/// finding B4-08) has a permission combining both to assert against, which
/// no existing `profiles` permission does (adding `fields.deny` there would
/// change what `stage4_redact_removes_the_named_field_before_the_guest_sees_it`
/// asserts survives the redact).
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
            },
            "lookup_targets": {
                "table": "lookup_targets",
                "permissions": {
                    "view": {
                        "allows": ["data-layer/read"],
                        "paths": [],
                        "fields": { "deny": ["classification"] },
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
        syneroym_app_orchestration::empty_resolver(),
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
            health_check: None,
            assets: None,
        },
        service_type: ServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(bytes),
            hash: None,
            interfaces: vec![],
        }),
        registry_certificate: None,
        instance_certificate: None,
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
        empty_service_proxy(),
        Some(policy),
        false,
        row_authorizer,
        None,
        syneroym_app_orchestration::empty_resolver(),
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
    // Mirrors the composition root (`runtime.rs`): without this, the
    // after-step's own throw-away instance gets `empty_row_authorizer()`
    // (`build_store_and_instantiate` falls back to it when `self_weak` is
    // unset), so any nested read from inside `authorize_rows` could never
    // actually re-enter it -- silently defeating any test of D-B4-4's
    // recursion bound (review finding B4-03).
    engine.self_weak.set(Arc::downgrade(&engine)).unwrap();

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
            None,
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
            None,
        )
        .await
        .unwrap();
    drop(store);

    let policy = Arc::new(parse_and_validate(&abac_policy()).unwrap());
    Some(Deployed { engine, storage_provider, key_store, blob_provider, policy, config_generation })
}

/// Skips (or, under `CI`, fails) a test whose fixture artifact wasn't
/// built. The silent-skip half is the house pattern shared by every
/// WASM-fixture test in this workspace (`data_layer_integration.rs` and
/// friends), kept here for local-dev convenience; the `CI` panic is
/// deliberately narrower than that pattern (review finding B4-10) --
/// these eight tests *are* Slice B4-fdae's security evidence
/// (Failure/Security matrix rows 7-9), and CI is known to build this
/// fixture (`.github/actions/ci-build-and-test/action.yml`), so a silent
/// skip there would mean a green run stopped proving what it claims to.
macro_rules! skip_if_missing_artifact {
    ($test_name:expr) => {
        panic!("{}: abac-test WASM artifact not found", $test_name);
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

/// `store::Host::get`'s stage-4 arm at ingress (i) (review finding B4-09):
/// every other test in this file drives `query` or `check_access`, leaving
/// `get`'s own fail-closed shape (`apply_stage4`'s `Err` mapped through
/// `map_abac_error`, per B4-04) untested end to end against a real guest
/// export. Same fixture, same seeded row, same `deny_by_field` mode as
/// `stage4_denies_rows_the_guest_rejects_for_a_real_caller` above -- only
/// the ingress method differs.
#[tokio::test]
async fn stage4_get_denies_rows_the_guest_rejects_for_a_real_caller() {
    let dir = tempfile::tempdir().unwrap();
    let Some(d) = deploy_with_mode(dir.path(), "deny_by_field").await else {
        skip_if_missing_artifact!("stage4_get_denies_rows_the_guest_rejects_for_a_real_caller");
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

    let result =
        DataLayerHost::get(&mut host, "profiles".to_string(), "owned-by-alice".to_string())
            .await
            .unwrap();
    assert!(
        result.is_none(),
        "the after-step must deny Alice's own 'secret' row on `get`, even though the sieve admits \
         it (the row exists and is hers): {result:?}"
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

/// The CLS-mask ∪ after-step-redact union, at both maskings' actual
/// sources (review finding B4-08): `lookup_targets`'s `view` permission
/// carries a policy `fields.deny: ["classification"]` (CLS) *and*
/// `authorize_rows: true`, and `redact` mode additionally redacts `ssn` at
/// runtime -- no existing test combined both on one permission, so the
/// `outcome.masked_fields.iter().cloned().chain(extra)` union at each
/// ingress site had nothing to exercise past `apply_stage4`'s own,
/// CLS-free unit tests.
#[tokio::test]
async fn stage4_cls_mask_unions_with_the_after_step_redact_set() {
    let dir = tempfile::tempdir().unwrap();
    let Some(d) = deploy_with_mode(dir.path(), "redact").await else {
        skip_if_missing_artifact!("stage4_cls_mask_unions_with_the_after_step_redact_set");
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
    let result = DataLayerHost::query(&mut host, "lookup_targets".to_string(), opts).await.unwrap();
    assert_eq!(result.records.len(), 2, "both seeded rows are redacted, not denied: {result:?}");
    for record in &result.records {
        let payload: serde_json::Value = serde_json::from_slice(&record.payload).unwrap();
        assert!(payload.get("classification").is_none(), "CLS-masked field leaked: {payload:?}");
        assert!(payload.get("ssn").is_none(), "after-step-redacted field leaked: {payload:?}");
        assert!(
            payload.get("note").is_some(),
            "a field named by neither mask must survive the union: {payload:?}"
        );
    }
}

/// Row 8: the `spin` mode's unbounded loop exceeds the after-step's
/// fuel/epoch budget -- `apply_stage4` maps that to a fail-closed `Err`,
/// which the ingress maps to a distinguishable `QuotaExceeded` error (B4-04):
/// resource pressure that stopped the after-step from running at all is not
/// the same claim as "it ran and denied every row", so it is not a silent
/// empty-but-successful page either.
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
    let err = DataLayerHost::query(&mut host, "profiles".to_string(), opts).await.unwrap_err();
    assert!(
        matches!(err, DataLayerError::QuotaExceeded),
        "a budget-exceeded after-step must deny closed with a distinguishable error: {err:?}"
    );
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
/// The recursion-bound half of this claim (review finding B4-03: the same
/// exemption is what *stops* this nested read from re-entering
/// `authorize_rows`) has its own dedicated test,
/// `stage4_nested_read_does_not_re_enter_the_after_step`, below -- this one
/// only needs a single-row `get` to exist, which `lookup_targets` having no
/// reachable sieve at all (pre-B4-03-fix) or a public one (post-fix) would
/// satisfy identically, so it can't by itself prove termination would fail
/// under a narrowed exemption.
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

/// D-B4-4's recursion bound (review finding B4-03) -- corrected per review
/// residual R1, which is right that the original version of this comment
/// named the wrong mechanism.
///
/// **What this test actually proves.** `resolve_query_auth`'s
/// `LocalReadOnly` exemption (`host_capabilities.rs`) makes the after-step's
/// own reads carry no `QueryAuth` at all, so the nested `query` inside
/// `nested_query_recurses_if_unexempted` mode reads `lookup_targets`
/// genuinely unfiltered -- seeing both seeded rows regardless of any policy
/// on that collection. If the exemption were ever narrowed, that nested
/// read would start going through `plan_read` like any other, and this test
/// fails: the fixture would see an empty result (not two rows) and deny
/// every candidate, dropping the outer `query`'s surviving row count from 1
/// to 0, breaking the assertion below.
///
/// **What it does *not* prove.** A narrowed exemption would *not* actually
/// make the nested read re-enter `authorize_rows`. `lookup_targets`' `view`
/// permission being unconditionally public (`paths: []`) only makes its
/// *row-level* predicate unconditional -- it does nothing to
/// `applicable_permissions`' *permission-level* gate
/// (`crates/fdae/src/compile.rs`), which requires the caller to hold some
/// capability granting the operation before *any* permission, public or
/// not, is even considered applicable. `CallerContext::service_abac` is
/// deliberately capability-less (D-B4-2), so under a narrowed exemption the
/// nested read would compile `deny_all()` -- zero rows, no `abac_permissions`
/// -- rather than reaching a second `authorize_rows` call. The recursion
/// bound is therefore doubly held today: the exemption (when intact) skips
/// policy evaluation for this identity entirely, and the capability gate
/// (independently, and regardless of the exemption) would deny it anyway.
/// Should a future change ever give `service_abac` a capability, the
/// capability gate stops being a backstop and this exemption becomes the
/// *only* thing preventing genuine re-entry -- that is the point at which
/// this test's original "prevents recursion" framing would become literally
/// true, and worth re-deriving from scratch rather than assuming still
/// holds.
///
/// Unlike `stage4_lookup_sees_its_own_service_data` above, an empty-rows
/// short-circuit inside `apply_stage4` can't silently make this pass for
/// the wrong reason: `profiles`' own `caller`-scoped permission would never
/// be applicable for the after-step's synthetic (capability-less) identity
/// either, so a nested read against *that* collection would already be
/// empty today -- exactly why this needs its own definition (`abac_policy`'s
/// doc comment) rather than reusing `profiles`.
///
/// A fast, correctly-decided completion is what today's behavior looks
/// like; a regression here surfaces as a broken assertion (this test
/// fails), not a hang -- and separately, even an actually-recursing
/// hypothetical would be bounded by `abac_instance_permits` +
/// `FDAE_ABAC_TIMEOUT` (review finding B4-01) rather than looping forever.
#[tokio::test]
async fn stage4_nested_read_does_not_re_enter_the_after_step() {
    let dir = tempfile::tempdir().unwrap();
    let Some(d) = deploy_with_mode(dir.path(), "nested_query_recurses_if_unexempted").await else {
        skip_if_missing_artifact!("stage4_nested_read_does_not_re_enter_the_after_step");
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
        "the after-step's nested `lookup_targets` query saw both seeded rows unfiltered and \
         allowed alice's own `profiles` row accordingly, without hanging or erroring: {result:?}"
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
    let err = DataLayerHost::query(&mut host, "profiles".to_string(), opts).await.unwrap_err();
    assert!(
        matches!(err, DataLayerError::Internal(_)),
        "an arity-mismatched decision list must deny closed with a distinguishable error: {err:?}"
    );
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

/// The runtime missing-export deny path (review finding B4-11). The
/// deploy-time gate (`orchestration.rs`'s `validate_stage4_export`) has its
/// own tests in `control_plane::service::orchestration`, but this file --
/// like every other low-level `sandbox_wasm` integration test -- deploys
/// straight through `AppSandboxEngine::deploy_wasm`, which performs no such
/// validation (deliberately: it's a `control_plane`-layer concern). That
/// left `AbacError::MissingExport` -- the last line of defence for a
/// persisted policy whose component was redeployed some other way, or
/// simply never re-validated -- untested at the engine level: every other
/// test in this file deploys `abac-test`, which always has the export. This
/// one deploys the `greeter` fixture (exports only `greet`, no
/// `authorize-rows`) under a stage-4-opted policy, seeding its data
/// directly at the storage level since `store::Host` is host-native and
/// doesn't care what the guest itself implements.
#[tokio::test]
async fn stage4_missing_export_under_an_opted_in_policy_denies_closed() {
    const GREETER_SERVICE_ID: &str = "greeter-abac-svc";
    let dir = tempfile::tempdir().unwrap();
    let Ok(greeter_bytes) = fs::read(test_constants::greeter_wasm_path()) else {
        skip_if_missing_artifact!("stage4_missing_export_under_an_opted_in_policy_denies_closed");
    };
    let (engine, storage_provider, key_store, blob_provider) =
        make_engine_with_storage(dir.path()).await;
    engine.self_weak.set(Arc::downgrade(&engine)).unwrap();

    let policy_json = r#"{
        "version": "fdae/v1",
        "definitions": {
            "widgets": {
                "table": "widgets",
                "permissions": {
                    "view": {
                        "allows": ["data-layer/read"],
                        "paths": [],
                        "authorize_rows": true
                    }
                }
            }
        }
    }"#;
    storage_provider.save_fdae_policy(GREETER_SERVICE_ID, policy_json).await.unwrap();
    let config_generation =
        storage_provider.save_config_generation(GREETER_SERVICE_ID, "{}").await.unwrap();

    let manifest = wasm_deploy_manifest(greeter_bytes);
    engine.deploy_wasm(GREETER_SERVICE_ID, &manifest).await.unwrap();

    let store = storage_provider.open_service_db(GREETER_SERVICE_ID, &key_store).await.unwrap();
    store
        .create_collection(&CollectionSchema { name: "widgets".to_string(), indexes: vec![] })
        .await
        .unwrap();
    store
        .put(
            "widgets",
            &RecordWriteValue { id: "w1".to_string(), payload: b"{}".to_vec() },
            GREETER_SERVICE_ID,
            None,
        )
        .await
        .unwrap();
    drop(store);

    let policy = Arc::new(parse_and_validate(policy_json).unwrap());
    let row_authorizer = row_authorizer_for(&engine);
    // Not `real_caller`: its capability is scoped to `SERVICE_ID`
    // ("abac-test-svc"), the constant every other test in this file
    // deploys against -- here the resource is `GREETER_SERVICE_ID`, so the
    // capability must name that instead, or `data-layer/read` is never
    // held on this resource and the permission is never applicable at all
    // (a `deny_all()` sieve with empty `abac_permissions`, which would
    // reach `Ok(empty)` without the after-step ever running -- silently
    // proving nothing about `MissingExport`).
    let caller = CallerContext {
        caller_did: "did:key:zAlice".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:zAlice".to_string(),
            capabilities: vec![Capability {
                with: ResourceUri::service(GREETER_SERVICE_ID, GREETER_SERVICE_ID),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Ucan,
        proof: None,
    };
    let mut host = HostState::new(
        GREETER_SERVICE_ID.to_string(),
        None,
        key_store,
        storage_provider,
        blob_provider,
        caller,
        config_generation,
        test_messaging_context(),
        test_streaming_context(),
        empty_service_proxy(),
        Some(policy),
        false,
        row_authorizer,
        None,
        syneroym_app_orchestration::empty_resolver(),
    );

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let err = DataLayerHost::query(&mut host, "widgets".to_string(), opts).await.unwrap_err();
    assert!(
        matches!(err, DataLayerError::Internal(_)),
        "a stage-4-opted policy against a component with no `authorize-rows` export must deny \
         closed with a distinguishable error, not silently pass rows through: {err:?}"
    );
}
