use super::helpers::*;

/// The cross-service join relationship proof called out as missing during
/// review, now through the real `syneroym_rpc::resolve_fetches` orchestration
/// instead of a hand-wired stand-in: `plan_read` (`crates/fdae`) ->
/// `resolve_fetches` (a real `ProxyRouter` call, `CallOrigin::Native`, to
/// hr-svc's native `resolve-relation`) -> `finalize` -> real SQL execution.
/// Also proves a *successful* fetch leaves `DecisionTrace` provenance
/// (ADR-0017 §6 reason 2), not just the deny path.
#[tokio::test]
#[expect(clippy::too_many_lines, reason = "linear cross-service fetch scenario")]
async fn plan_read_resolve_fetches_finalize_join_end_to_end_through_a_real_proxy() {
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let owner_did = "did:key:zHrSvcOwner";
    // `_native_dispatch`/`_temp_dir` must stay alive for the whole test --
    // `proxy_router` only holds a `Weak` to the dispatch table, and dropping
    // the storage's backing directory would tear down its writer thread out
    // from under a still-in-flight query. See `build_hr_svc_proxy_router`'s
    // own doc comment for why these must never be leaked instead.
    let (proxy_router, expected_asserter_did, _native_dispatch, _temp_dir) =
        build_hr_svc_proxy_router(node_identity, owner_did).await;

    // -- the local (requesting) service: app-svc, whose own policy names a
    // remote relation pointing at hr-svc, trusting the *real* derived
    // asserter DID -- not a placeholder.
    let local_service_id = "app-svc-join-test";
    let local_policy = parse_and_validate(&format!(
        r#"{{
            "version": "fdae/v1",
            "definitions": {{
                "document": {{
                    "table": "documents",
                    "relations": {{"owner": {{
                        "target": "employee", "service": "hr-svc", "join_column": "owner_uuid",
                        "expected_asserter_did": "{expected_asserter_did}"
                    }}}},
                    "permissions": {{
                        "view": {{"allows": ["data-layer/read"], "paths": [["owner", "anchor"]]}}
                    }}
                }}
            }}
        }}"#
    ))
    .unwrap();

    // A caller presenting as `svc-A` (whoever actually authenticated this
    // connection to app-svc) proxying for anchor `alice`. `proof: None`
    // here (no real UCAN chain in this test), so `resolve_fetches` forwards
    // an unauthenticated caller and hr-svc's own re-verification is a no-op
    // that trusts `caller_did` as given -- sufficient to exercise the real
    // proxy/wire path end to end without standing up a full B1 chain.
    let proxying_caller = CallerContext {
        caller_did: "did:key:svc-A".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:svc-A".to_string(),
            anchor_did: Some("did:key:alice".to_string()),
            capabilities: vec![Capability {
                with: native_fdae_resource(local_service_id, "document"),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Ucan,
        proof: None,
    };

    // Step 1: `plan_read` on the *local* policy -- must produce a
    // `RemoteFetch`, asking about the anchor, never the proxying caller.
    let mut plan = syneroym_fdae::plan_read(
        &local_policy,
        "document",
        &proxying_caller.session,
        local_service_id,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();
    assert!(
        plan.local.is_none(),
        "a policy needing a remote fetch must not resolve to a local sieve"
    );
    assert_eq!(plan.fetches.len(), 1);
    let fetch = plan.fetches[0].clone();
    assert_eq!(fetch.service, "hr-svc");
    assert_eq!(fetch.relation, "employee", "the wire relation is the remote object type");
    assert_eq!(fetch.principal_did, "did:key:alice", "the fetch asks about the anchor");

    // Step 2: the real orchestration seam -- `resolve_fetches` issues the
    // fetch as `CallOrigin::Native` through the real `ProxyRouter`, which
    // dispatches it to hr-svc's native `resolve-relation`, verifies the
    // returned `RelationshipProof` against `fetch.expected_asserter_did`,
    // and returns a `FetchResult` carrying real provenance.
    let results = syneroym_rpc::resolve_fetches(
        &plan.fetches,
        &proxying_caller,
        proxy_router.as_ref(),
        local_service_id,
    )
    .await
    .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].ids, vec!["emp-alice".to_string()]);
    assert_eq!(results[0].trace.asserter_did, expected_asserter_did);
    assert_eq!(results[0].trace.relation, "employee");
    assert_eq!(results[0].trace.principal_did, "did:key:alice");

    // Step 3: `finalize` the plan with the real fetched id-set, and run
    // the resulting sieve against a locally-seeded `documents` table --
    // the same raw-SQL verification `crates/fdae`'s own `finalize` tests
    // use, proving the compiled predicate is not just well-typed but
    // actually correct.
    let pending = plan.pending.take().unwrap();
    let sieve = syneroym_fdae::finalize(pending, &results).unwrap();

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE documents (
            id TEXT PRIMARY KEY, creator_id TEXT, created_at INTEGER, updated_at INTEGER,
            payload TEXT NOT NULL DEFAULT '{}'
        );",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO documents (id, payload) VALUES ('doc-1', ?1)",
        [json!({"owner_uuid": "emp-alice"}).to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO documents (id, payload) VALUES ('doc-2', ?1)",
        [json!({"owner_uuid": "emp-bob"}).to_string()],
    )
    .unwrap();

    let sql = format!("SELECT id FROM documents WHERE {} ORDER BY id", sieve.where_clause);
    let mut stmt = conn.prepare(&sql).unwrap();
    let visible: Vec<String> = stmt
        .query_map(rusqlite::params_from_iter(sieve.params.iter()), |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(
        visible,
        vec!["doc-1"],
        "only alice's own document is reachable through the full plan -> fetch -> finalize join, \
         through a real ProxyRouter"
    );

    // The successful fetch must also leave provenance in the sieve's own
    // `DecisionTrace` (ADR-0017 §6 reason 2, sibling requirement for
    // the allow path) -- not just the deny path already covered.
    assert_eq!(sieve.trace.remote_fetches.len(), 1);
    assert_eq!(sieve.trace.remote_fetches[0].asserter_did, expected_asserter_did);
    assert!(sieve.trace.remote_fetches[0].valid_until_secs > 0);
}

/// The test above drives `plan_read` -> `resolve_fetches` -> `finalize` ->
/// raw SQL by hand, bypassing `resolve_query_auth`. This is the same join,
/// but through `SynSvcNativeService::dispatch`'s real `"query"` handler --
/// the production method the fetch-failure deny test above exercises for its
/// (deny) branch -- so the assembled success branch (`resolve_query_auth`
/// building a `QueryAuth` whose `resolved_sieve` actually returns
/// correctly-filtered rows via `store.query`) has coverage through the
/// dispatch method too, not just through its individually-tested pieces.
#[tokio::test]
async fn native_dispatch_query_resolves_a_cross_service_fetch_end_to_end() {
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (proxy_router, expected_asserter_did, _native_dispatch, _hr_temp_dir) =
        build_hr_svc_proxy_router(node_identity.clone(), "did:key:zHrSvcOwner3").await;

    let local_service_id = "app-svc-query-through-dispatch";
    let local_policy = parse_and_validate(&format!(
        r#"{{
            "version": "fdae/v1",
            "definitions": {{
                "document": {{
                    "table": "documents",
                    "relations": {{"owner": {{
                        "target": "employee", "service": "hr-svc", "join_column": "owner_uuid",
                        "expected_asserter_did": "{expected_asserter_did}"
                    }}}},
                    "permissions": {{
                        "view": {{"allows": ["data-layer/read"], "paths": [["owner", "anchor"]]}}
                    }}
                }}
            }}
        }}"#
    ))
    .unwrap();

    let key_store = Arc::new(KeyStore::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let service_proxy: Arc<dyn ServiceProxy> = proxy_router;
    let app_service = SynSvcNativeService::new(
        local_service_id.to_string(),
        key_store.clone(),
        storage_provider.clone(),
        blob_provider,
        messaging_broker,
        Some(Arc::new(local_policy)),
        node_identity,
        "did:key:zAppSvcOwner",
        Arc::downgrade(&service_proxy),
        syneroym_rpc::empty_row_authorizer(),
        None,
    );

    // Seeded directly against the store, `auth: None` -- `local_policy`
    // declares only a `view` (read) permission, so seeding through the
    // gated native `"put"` dispatch would deny closed regardless of caller.
    for (id, owner_uuid) in [("doc-1", "emp-alice"), ("doc-2", "emp-bob")] {
        seed_via_store(
            &storage_provider,
            &key_store,
            local_service_id,
            "documents",
            id,
            &json!({"owner_uuid": owner_uuid}),
        )
        .await;
    }

    // Alice's proxying caller, same shape as the hand-wired join test above.
    let proxying_caller = CallerContext {
        caller_did: "did:key:svc-A".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:svc-A".to_string(),
            anchor_did: Some("did:key:alice".to_string()),
            capabilities: vec![Capability {
                with: native_fdae_resource(local_service_id, "documents"),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Ucan,
        proof: None,
    };

    let resp = app_service
        .dispatch(NativeInvocation {
            interface: "data-layer".to_string(),
            method: "query".to_string(),
            params: json!({"collection": "documents", "opts": {}}),
            caller: proxying_caller,
        })
        .await
        .unwrap();
    let records = resp.payload["records"].as_array().expect("query must return records");
    assert_eq!(
        records.len(),
        1,
        "only alice's own document must be reachable through a real dispatch(\"query\") call: {:?}",
        resp.payload
    );
    assert_eq!(records[0]["id"], "doc-1");
}

/// A `RelationshipProof` signed by an identity the policy does *not* name in
/// `expected_asserter_did` (e.g. an impersonator standing up its own service
/// at the same logical name) must be rejected, not silently trusted off its
/// own self-declared `asserter_did` field -- exercised through the
/// real `ProxyRouter`/`resolve_fetches` path, not just `rpc`'s own unit test
/// of `RelationshipProof::verify` in isolation.
#[tokio::test]
async fn resolve_fetches_denies_when_the_real_proxys_asserter_does_not_match_the_policy() {
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (proxy_router, _real_asserter_did, _native_dispatch, _temp_dir) =
        build_hr_svc_proxy_router(node_identity, "did:key:zHrSvcOwner").await;

    let local_policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"owner": {
                        "target": "employee", "service": "hr-svc", "join_column": "owner_uuid",
                        "expected_asserter_did": "did:key:zNotTheRealHrSvc"
                    }},
                    "permissions": {
                        "view": {"allows": ["data-layer/read"], "paths": [["owner", "anchor"]]}
                    }
                }
            }
        }"#,
    )
    .unwrap();
    let proxying_caller = CallerContext {
        caller_did: "did:key:svc-A".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:svc-A".to_string(),
            anchor_did: Some("did:key:alice".to_string()),
            capabilities: vec![Capability {
                with: native_fdae_resource("app-svc-mismatch-test", "document"),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Ucan,
        proof: None,
    };
    let plan = syneroym_fdae::plan_read(
        &local_policy,
        "document",
        &proxying_caller.session,
        "app-svc-mismatch-test",
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();

    let err = syneroym_rpc::resolve_fetches(
        &plan.fetches,
        &proxying_caller,
        proxy_router.as_ref(),
        "app-svc-mismatch-test",
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, FetchError::ProofInvalid { .. }),
        "a real, correctly-signed proof from the wrong asserter must still be rejected: {err:?}"
    );
}

/// Native dispatch's own fail-closed behavior for a cross-service fetch
/// failure (mirrors `sandbox_wasm::host_capabilities`'s equivalent WASM-path
/// tests): `SynSvcNativeService`'s `get`/`query` must deny the whole read,
/// not fall back to unfiltered or silently-empty, when the configured
/// `ServiceProxy` errors on the fetch.
#[tokio::test]
async fn native_dispatch_denies_closed_on_a_cross_service_fetch_failure() {
    #[derive(Debug)]
    struct AlwaysErrorsProxy;
    #[async_trait::async_trait]
    impl ServiceProxy for AlwaysErrorsProxy {
        async fn invoke(&self, _request: ProxyRequest) -> Result<Value, ProxyError> {
            Err(ProxyError::Timeout(Duration::from_secs(5)))
        }
    }

    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"owner": {
                        "target": "employee", "service": "hr-svc", "join_column": "owner_uuid",
                        "expected_asserter_did": "did:key:zHrSvc"
                    }},
                    "permissions": {
                        "view": {"allows": ["data-layer/read"], "paths": [["owner", "anchor"]]}
                    }
                }
            }
        }"#,
    )
    .unwrap();
    let service_id = "app-svc-fetch-failure-test";
    let key_store = Arc::new(KeyStore::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let stub_proxy: Arc<dyn ServiceProxy> = Arc::new(AlwaysErrorsProxy);
    let app_service = SynSvcNativeService::new(
        service_id.to_string(),
        key_store,
        storage_provider,
        blob_provider,
        messaging_broker,
        Some(Arc::new(policy)),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
        Arc::downgrade(&stub_proxy),
        syneroym_rpc::empty_row_authorizer(),
        None,
    );

    let caller = CallerContext {
        caller_did: "did:key:svc-A".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:svc-A".to_string(),
            anchor_did: Some("did:key:alice".to_string()),
            capabilities: vec![Capability {
                // Scoped to "documents" (the literal `collection` param the
                // "query" call below passes), matching how `plan_read`
                // builds its resource string from whatever collection
                // argument it's called with -- not "document" (the policy's
                // own definition key), which would leave zero entitling
                // capabilities and mask the fetch-failure path this test
                // means to exercise behind an unrelated "collection not
                // found" (the query never having reached `resolve_fetches`
                // at all).
                with: native_fdae_resource(service_id, "documents"),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Ucan,
        proof: None,
    };
    let resp = app_service
        .dispatch(NativeInvocation {
            interface: "data-layer".to_string(),
            method: "query".to_string(),
            params: json!({"collection": "documents", "opts": {}}),
            caller,
        })
        .await;
    let err = resp.unwrap_err();
    assert_eq!(err.code(), -32010, "a cross-service fetch failure must deny closed: {err:?}");
}
