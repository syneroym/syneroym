#![allow(clippy::too_many_lines, clippy::cognitive_complexity)]

use syneroym_data_db::SqliteStorageProvider;
use syneroym_rpc::SessionContext;

use super::*;

fn origin_host_state(
    storage: Arc<dyn StorageProvider>,
    origin: InvocationOrigin,
    auth: AuthLevel,
) -> HostState {
    let caller = CallerContext {
        caller_did: "did:key:zWireCaller".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:zWireCaller".to_string(),
            ..Default::default()
        },
        auth,
        proof: None,
    };
    HostState::new(
        "origin-test".to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage,
        test_blob_provider(),
        caller,
        0,
        test_messaging_context(),
        test_streaming_context(),
        test_service_proxy(),
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    )
    .with_invocation_origin(origin)
}

/// The origin rule: a local dispatch is `internal` whatever identity it
/// carries; a wire dispatch reads the caller's auth, and a
/// substrate-injected level is never `verified`.
#[tokio::test]
async fn invocation_caller_origin_mapping() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());

    for auth in [
        AuthLevel::Delegated,
        AuthLevel::Ucan,
        AuthLevel::System,
        AuthLevel::LocalElevated,
        AuthLevel::LocalReadOnly,
    ] {
        let mut local = origin_host_state(storage.clone(), InvocationOrigin::Local, auth);
        assert_eq!(
            invocation::Host::caller(&mut local).await,
            WitCallerOrigin::Internal,
            "local call must be internal for {auth:?}"
        );
    }

    for auth in [AuthLevel::Delegated, AuthLevel::Ucan] {
        let mut wire = origin_host_state(storage.clone(), InvocationOrigin::Wire, auth);
        assert_eq!(
            invocation::Host::caller(&mut wire).await,
            WitCallerOrigin::Verified("did:key:zWireCaller".to_string()),
        );
    }
    for auth in [AuthLevel::System, AuthLevel::LocalElevated, AuthLevel::LocalReadOnly] {
        let mut wire = origin_host_state(storage.clone(), InvocationOrigin::Wire, auth);
        assert_eq!(
            invocation::Host::caller(&mut wire).await,
            WitCallerOrigin::Anonymous,
            "a substrate-injected level must not read as verified on the wire ({auth:?})"
        );
    }
}

#[tokio::test]
async fn test_config_get_and_get_section() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());

    let config_json =
        r#"{"db_host": "localhost", "db_port": "5432", "db.password": "secret", "db": "mydb"}"#;
    let generation = storage.save_config_generation("test_svc", config_json).await.unwrap();

    let mut host = HostState::new(
        "test_svc".to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage,
        test_blob_provider(),
        CallerContext::service_system("test-caller"),
        generation,
        test_messaging_context(),
        test_streaming_context(),
        test_service_proxy(),
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    );

    use app_config::Host as ConfigHost;

    // 1. Existing key returns Ok(Some(value))
    let val = ConfigHost::get(&mut host, "db_host".to_string()).await.unwrap().unwrap();
    assert_eq!(val, "localhost");

    // 2. Missing key returns Ok(None)
    let missing = ConfigHost::get(&mut host, "db_user".to_string()).await.unwrap();
    assert!(missing.is_none());

    // get_section returns prefixed values with exact matching boundaries
    let section = ConfigHost::get_section(&mut host, "db".to_string()).await.unwrap();
    let mut section_keys: Vec<String> = section.into_iter().map(|(k, _)| k).collect();
    section_keys.sort();
    // "db" and "db.password" match. "db_host" and "db_port" DO NOT.
    assert_eq!(section_keys, vec!["db", "db.password"]);
}

#[tokio::test]
async fn test_config_isolation_and_generation_pinning() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());

    // Service A Gen 1
    let gen1_a = storage.save_config_generation("svc_a", r#"{"mode": "v1"}"#).await.unwrap();
    // Service A Gen 2
    let gen2_a = storage.save_config_generation("svc_a", r#"{"mode": "v2"}"#).await.unwrap();

    // Service B Gen 1
    let gen1_b = storage.save_config_generation("svc_b", r#"{"mode": "b_mode"}"#).await.unwrap();

    use app_config::Host as ConfigHost;

    // Two WASM components with different configs get isolated values
    let mut host_a_gen2 = HostState::new(
        "svc_a".to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage.clone(),
        test_blob_provider(),
        CallerContext::service_system("test-caller"),
        gen2_a,
        test_messaging_context(),
        test_streaming_context(),
        test_service_proxy(),
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    );
    let mut host_b = HostState::new(
        "svc_b".to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage.clone(),
        test_blob_provider(),
        CallerContext::service_system("test-caller"),
        gen1_b,
        test_messaging_context(),
        test_streaming_context(),
        test_service_proxy(),
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    );

    let val_a = ConfigHost::get(&mut host_a_gen2, "mode".to_string()).await.unwrap().unwrap();
    let val_b = ConfigHost::get(&mut host_b, "mode".to_string()).await.unwrap().unwrap();
    assert_eq!(val_a, "v2");
    assert_eq!(val_b, "b_mode");

    // Re-deploy bumps generation; in-flight invocations retain prior generation
    let mut host_a_gen1 = HostState::new(
        "svc_a".to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage.clone(),
        test_blob_provider(),
        CallerContext::service_system("test-caller"),
        gen1_a,
        test_messaging_context(),
        test_streaming_context(),
        test_service_proxy(),
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    );
    let val_a_old = ConfigHost::get(&mut host_a_gen1, "mode".to_string()).await.unwrap().unwrap();
    assert_eq!(val_a_old, "v1");
}

/// M3A failure/security test: `vault/reveal` on a non-existent key
/// returns `vault-error::not-found` at the WIT host-function boundary
/// (not just `Ok(None)` one layer down at `ServiceStore::reveal_secret`,
/// which `syneroym-data-db`'s own tests already cover).
#[tokio::test]
async fn test_vault_reveal_not_found_at_host_boundary() {
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([3u8; 32]).unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), true).unwrap());
    let mut host_state = HostState::new(
        "vault-not-found-svc".to_string(),
        None,
        key_store,
        storage_provider,
        test_blob_provider(),
        CallerContext::service_system("test-caller"),
        0,
        test_messaging_context(),
        test_streaming_context(),
        test_service_proxy(),
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    );

    let result = vault::Host::reveal(&mut host_state, "does-not-exist".to_string()).await;
    assert!(matches!(result, Err(VaultError::NotFound)));
}
