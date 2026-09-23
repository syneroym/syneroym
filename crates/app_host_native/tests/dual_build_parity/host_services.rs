use super::helpers::*;

#[tokio::test]
async fn a_dependency_resolves_to_the_same_target_on_both_builds() {
    let h = harness().await;
    let wasm_res = h
        .wasm
        .run(r#"{"op":"proxy-call-dependency","name":"sibling","interface":"greeter","method":"greet","params":"{}"}"#)
        .await
        .unwrap();
    let native_res = h
        .native
        .run(r#"{"op":"proxy-call-dependency","name":"sibling","interface":"greeter","method":"greet","params":"{}"}"#)
        .await
        .unwrap();
    assert_eq!(wasm_res, native_res);
}

#[tokio::test]
async fn an_enqueue_without_an_idempotency_key_is_refused_identically() {
    let h = harness().await;
    let wasm_res = h.wasm.run(r#"{"op":"proxy-enqueue-no-key","name":"sibling"}"#).await.unwrap();
    let native_res =
        h.native.run(r#"{"op":"proxy-enqueue-no-key","name":"sibling"}"#).await.unwrap();
    assert_eq!(wasm_res, native_res);
}

#[tokio::test]
async fn both_builds_read_the_same_config_generation() {
    let h = harness().await;
    let wasm_res1 = h.wasm.run(r#"{"op":"read-config","key":"greeting"}"#).await.unwrap();
    let native_res1 = h.native.run(r#"{"op":"read-config","key":"greeting"}"#).await.unwrap();
    assert_eq!(wasm_res1, native_res1);

    h.wasm_engine
        .storage_provider
        .save_config_generation(SERVICE_ID, r#"{"greeting":"hello generation 2"}"#)
        .await
        .unwrap();
    h.native_storage_provider
        .save_config_generation(SERVICE_ID, r#"{"greeting":"hello generation 2"}"#)
        .await
        .unwrap();

    let wasm_res2 = h.wasm.run(r#"{"op":"read-config","key":"greeting"}"#).await.unwrap();
    let native_res2 = h.native.run(r#"{"op":"read-config","key":"greeting"}"#).await.unwrap();
    assert_eq!(wasm_res2, native_res2);
    let v: Value = serde_json::from_str(&wasm_res2).unwrap();
    assert_eq!(v["ok"]["value"], "hello generation 2");
}

#[tokio::test]
async fn a_cached_fdae_policy_can_be_invalidated_and_reloaded() {
    let dir = tempfile::tempdir().unwrap();
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([0x42; 32]).expect("inject kek");
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(dir.path().join("data"), true).unwrap());
    {
        let service_store = storage_provider.open_service_db(SERVICE_ID, &key_store).await.unwrap();
        service_store
            .create_collection(&DbCollectionSchema {
                name: "profiles".to_string(),
                indexes: vec![],
            })
            .await
            .unwrap();
    }
    // Initially no policy -> write succeeds
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
        storage_provider.clone(),
        blob_provider,
        broker,
        endpoint_registry,
        resolver,
        conversation,
        WebSocketSenders::new(),
    );
    factory.set_service_proxy(Arc::downgrade(&stub) as Weak<dyn ServiceProxy>);

    let host1 = factory.host_for(caller());
    let res1 = host1
        .put(
            "profiles".to_string(),
            RecordWriteValue { id: "p1".to_string(), payload: b"{}".to_vec() },
        )
        .await;
    assert!(res1.is_ok());

    // Now save an ABAC policy into storage. Because policy is cached as None,
    // it would still be None without invalidation.
    storage_provider.save_fdae_policy(SERVICE_ID, &abac_policy()).await.unwrap();

    // Invalidate FDAE policy cache and bump generation
    factory.invalidate_fdae_policy().await;

    // Fresh host should now reload policy from storage and deny write under ABAC
    let host2 = factory.host_for(caller());
    let res2 = host2
        .put(
            "profiles".to_string(),
            RecordWriteValue { id: "p2".to_string(), payload: b"{}".to_vec() },
        )
        .await;
    assert!(res2.is_err());
}
