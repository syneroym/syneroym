use syneroym_app_host::{AppBlobStore, AppBlobWriter};

use super::helpers::*;

/// Resource lifetime: a fresh `HostState` (and therefore a fresh
/// `ResourceTable`) is built per invocation on the native build, exactly
/// as the sandbox builds a fresh `Store` per guest call. The
/// WASM side cannot even express this (the fixture has one verb, and a
/// resource never crosses a `run` call); asserted natively, where
/// `NativeHostFactory::host_for` is separately reachable per call. Two
/// independent invocations opening an upload each land at table index 0
/// (`rep` 0) in their own fresh table -- if invocations shared one
/// table, the second `open_upload` would land at index 1.
#[tokio::test]
async fn each_native_invocation_gets_a_fresh_resource_table() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Arc::new(StubProxy);
    let node_id = Arc::new(Identity::generate().unwrap());
    let master = Identity::generate().unwrap();
    let clock = RecordClock::Fixed(1_800_000_000);
    let (_, factory, _, _, _, _) =
        build_native_stack(dir.path(), &stub, node_id, &master, clock).await;

    let host_a = factory.host_for(caller());
    let writer_a = host_a.open_upload().await.unwrap();
    assert_eq!(
        writer_a.rep(),
        0,
        "invocation a's writer should be the first entry in its own fresh table"
    );
    let hash_a = {
        let mut w = writer_a;
        w.write(b"invocation a".to_vec()).await.unwrap();
        w.finish().await.unwrap()
    };

    let host_b = factory.host_for(caller());
    let writer_b = host_b.open_upload().await.unwrap();
    assert_eq!(
        writer_b.rep(),
        0,
        "invocation b's writer should also be index 0 -- a shared table would put it at 1"
    );
    let hash_b = {
        let mut w = writer_b;
        w.write(b"invocation b".to_vec()).await.unwrap();
        w.finish().await.unwrap()
    };

    // Belt and suspenders: both blobs are also separately retrievable
    // afterward, proving neither invocation's table state leaked into
    // or clobbered the other's.
    assert_ne!(hash_a, hash_b);
    assert_eq!(host_a.get_blob(hash_a).await.unwrap(), b"invocation a");
    assert_eq!(host_b.get_blob(hash_b).await.unwrap(), b"invocation b");
}

/// Subscription persistence: the WASM build's subscription is written
/// to `messaging_subscriptions` and replayed at boot; the native build
/// deliberately writes nothing (see `NativeHostFactory::subscribe`'s own
/// doc comment for why). Asserted on `StorageProvider` state directly,
/// not via a restart simulation --
/// `replay_persisted_subscriptions` is private to `syneroym-substrate`.
/// Tracked in the deferred backlog as the native build's known restart
/// gap.
#[tokio::test]
async fn only_the_wasm_stacks_subscription_is_persisted() {
    let h = harness().await;
    h.wasm.run(r#"{"op":"subscribe-topic","topic":"persisted"}"#).await.unwrap();
    h.native.run(r#"{"op":"subscribe-topic","topic":"persisted"}"#).await.unwrap();

    let wasm_rows =
        h.wasm_engine.storage_provider.list_all_messaging_subscriptions().await.unwrap();
    assert!(wasm_rows.iter().any(|(sid, _)| sid == SERVICE_ID));

    let native_rows = h.native_storage_provider.list_all_messaging_subscriptions().await.unwrap();
    assert!(native_rows.is_empty());
}

#[tokio::test]
async fn a_policy_with_abac_permissions_fails_closed_on_the_native_build() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Arc::new(StubProxy);
    let node_id = Arc::new(Identity::generate().unwrap());
    let master = Identity::generate().unwrap();
    let clock = RecordClock::Fixed(1_800_000_000);
    let (_, factory, storage_provider, _, _, _) =
        build_native_stack(dir.path(), &stub, node_id, &master, clock).await;
    storage_provider.save_fdae_policy(SERVICE_ID, &abac_policy()).await.unwrap();

    let host = factory.host_for(caller());
    use syneroym_app_host::AppDataLayer;
    let res = host.get("profiles".to_string(), "owned-by-alice".to_string()).await;
    assert!(res.is_err(), "native build should fail closed on ABAC policies");
}

struct TransientFdaeStorage {
    inner: Arc<dyn StorageProvider>,
    failed_once: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl StorageProvider for TransientFdaeStorage {
    async fn open_service_db(
        &self,
        service_id: &str,
        key_store: &Arc<KeyStore>,
    ) -> anyhow::Result<Box<dyn syneroym_data_db::traits::ServiceStore>> {
        self.inner.open_service_db(service_id, key_store).await
    }

    async fn rotate_kek(&self, key_store: &Arc<KeyStore>, new_kek: [u8; 32]) -> anyhow::Result<()> {
        self.inner.rotate_kek(key_store, new_kek).await
    }

    async fn load_service_dek(
        &self,
        service_id: &str,
        key_store: &Arc<KeyStore>,
    ) -> anyhow::Result<Option<zeroize::Zeroizing<[u8; 32]>>> {
        self.inner.load_service_dek(service_id, key_store).await
    }

    async fn service_exists(&self, service_id: &str) -> anyhow::Result<bool> {
        self.inner.service_exists(service_id).await
    }

    async fn save_config_generation(
        &self,
        service_id: &str,
        config_blob: &str,
    ) -> anyhow::Result<u64> {
        self.inner.save_config_generation(service_id, config_blob).await
    }

    async fn delete_config_generation(
        &self,
        service_id: &str,
        generation: u64,
    ) -> anyhow::Result<()> {
        self.inner.delete_config_generation(service_id, generation).await
    }

    async fn get_config_generation(
        &self,
        service_id: &str,
        generation: u64,
    ) -> anyhow::Result<Option<String>> {
        self.inner.get_config_generation(service_id, generation).await
    }

    async fn get_latest_config_generation(
        &self,
        service_id: &str,
    ) -> anyhow::Result<Option<(u64, String)>> {
        self.inner.get_latest_config_generation(service_id).await
    }

    async fn save_messaging_subscription(
        &self,
        service_id: &str,
        topic: &str,
    ) -> anyhow::Result<()> {
        self.inner.save_messaging_subscription(service_id, topic).await
    }

    async fn delete_messaging_subscription(
        &self,
        service_id: &str,
        topic: &str,
    ) -> anyhow::Result<()> {
        self.inner.delete_messaging_subscription(service_id, topic).await
    }

    async fn delete_all_messaging_subscriptions_for_service(
        &self,
        service_id: &str,
    ) -> anyhow::Result<()> {
        self.inner.delete_all_messaging_subscriptions_for_service(service_id).await
    }

    async fn list_all_messaging_subscriptions(&self) -> anyhow::Result<Vec<(String, String)>> {
        self.inner.list_all_messaging_subscriptions().await
    }

    async fn save_fdae_policy(&self, service_id: &str, policy_json: &str) -> anyhow::Result<()> {
        self.inner.save_fdae_policy(service_id, policy_json).await
    }

    async fn load_fdae_policy(&self, service_id: &str) -> anyhow::Result<Option<String>> {
        if self.failed_once.fetch_and(false, Ordering::SeqCst) {
            anyhow::bail!("transient storage error");
        }
        self.inner.load_fdae_policy(service_id).await
    }

    async fn delete_fdae_policy(&self, service_id: &str) -> anyhow::Result<()> {
        self.inner.delete_fdae_policy(service_id).await
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn a_transient_fdae_policy_load_failure_is_not_memoized() {
    let dir = tempfile::tempdir().unwrap();
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([0x42; 32]).expect("inject kek");
    let raw_storage = Arc::new(SqliteStorageProvider::new(dir.path().join("data"), true).unwrap());
    {
        let service_store = raw_storage.open_service_db(SERVICE_ID, &key_store).await.unwrap();
        service_store
            .create_collection(&DbCollectionSchema {
                name: "profiles".to_string(),
                indexes: vec![],
            })
            .await
            .unwrap();
        service_store
            .put(
                "profiles",
                &DbRecordWriteValue {
                    id: "owned-by-alice".to_string(),
                    payload: br#"{"creator_uuid":"did:key:zParityTestCaller"}"#.to_vec(),
                },
                &caller().caller_did,
                None,
            )
            .await
            .unwrap();
    }
    raw_storage.save_fdae_policy(SERVICE_ID, &abac_policy()).await.unwrap();

    let storage_provider: Arc<dyn StorageProvider> = Arc::new(TransientFdaeStorage {
        inner: raw_storage,
        failed_once: std::sync::atomic::AtomicBool::new(true),
    });

    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let endpoint_registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let master = Identity::generate().unwrap();
    install_outbound_identity(&endpoint_registry, SERVICE_ID, &master).await;

    let app_instance = AppInstanceId::new("test-app");
    let sibling_name = LogicalServiceName::new("sibling");
    let inventory = Arc::new(StaticInventory::new());
    inventory.register(
        TopologyKey::local(app_instance.clone(), sibling_name),
        TopologyEntry {
            mode: TopologyMode::Singleton,
            members: vec![ServiceId::new("did:key:zSiblingMember")],
            sharding_strategy: None,
            epoch: TopologyEpoch(1),
            cache_ttl: Duration::from_secs(60),
            not_after: None,
        },
    );
    let resolver = Arc::new(LogicalResolver::new(inventory));
    endpoint_registry
        .set_app_context(SERVICE_ID.to_string(), app_instance.to_string(), "self".to_string())
        .await
        .unwrap();

    let conversation = test_conversation_service(
        storage_provider.clone(),
        key_store.clone(),
        endpoint_registry.clone(),
    );
    let stub = Arc::new(StubProxy);
    let factory = NativeHostFactory::new(
        SERVICE_ID.to_string(),
        key_store,
        storage_provider,
        blob_provider,
        broker,
        endpoint_registry,
        resolver,
        conversation,
        WebSocketSenders::new(),
    );
    factory.set_service_proxy(Arc::downgrade(&stub) as Weak<dyn ServiceProxy>);

    let host1 = factory.host_for(caller());
    // First load attempts to read FDAE policy, but TransientFdaeStorage returns an
    // Err. The error is not memoized, so host1 sees absent policy and put
    // succeeds.
    let res1 = host1
        .put(
            "profiles".to_string(),
            RecordWriteValue { id: "p1".to_string(), payload: b"{}".to_vec() },
        )
        .await;
    assert!(res1.is_ok());

    let host2 = factory.host_for(caller());
    // Second load attempts to read FDAE policy and succeeds. The ABAC policy is
    // memoized and denies write permissions to caller.
    let res2 = host2
        .put(
            "profiles".to_string(),
            RecordWriteValue { id: "p2".to_string(), payload: b"{}".to_vec() },
        )
        .await;
    assert!(res2.is_err());

    let host3 = factory.host_for(caller());
    // Third load uses the memoized policy and also denies write permissions.
    let res3 = host3
        .put(
            "profiles".to_string(),
            RecordWriteValue { id: "p3".to_string(), payload: b"{}".to_vec() },
        )
        .await;
    assert!(res3.is_err());
}
