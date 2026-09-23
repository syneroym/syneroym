use super::helpers::*;

#[tokio::test]
async fn resolve_relation_a1_overflow_maps_to_quota_exceeded() {
    let service_id = "resolve-relation-a1-overflow-svc";
    let (route_handler, pipeline, preamble, _temp_dir, storage_provider, key_store) =
        resolve_relation_service_and_pipeline(service_id, Some(resolvable_employee_policy())).await;
    seed_many_employees(
        &storage_provider,
        &key_store,
        service_id,
        "did:key:alice",
        MAX_FETCH_IDS + 1,
    )
    .await;

    let alice = employee_reader_caller("did:key:alice", service_id);
    let body = resolve_relation_body("employee", "did:key:alice");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(
        resp["error"]["code"], -32013,
        "an A1 result exceeding MAX_FETCH_IDS must map to quota-exceeded: {resp:?}"
    );
}

/// The A2 mirror: `query_raw`'s explicit `LIMIT MAX_FETCH_IDS + 1` is what
/// makes the overflow observable at all (raw SQL has no automatic page cap
/// the way `query` does) -- confirms it's actually wired up, not merely
/// present in the SQL text.
#[tokio::test]
async fn resolve_relation_a2_overflow_maps_to_quota_exceeded() {
    let service_id = "resolve-relation-a2-overflow-svc";
    let (route_handler, pipeline, preamble, _temp_dir, storage_provider, key_store) =
        resolve_relation_service_and_pipeline(service_id, Some(resolvable_employee_policy())).await;
    seed_many_employees(
        &storage_provider,
        &key_store,
        service_id,
        "did:key:bob",
        MAX_FETCH_IDS + 1,
    )
    .await;

    let bob = zero_capability_caller("did:key:bob");
    let body = resolve_relation_body("employee", "did:key:bob");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&bob), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(
        resp["error"]["code"], -32013,
        "an A2 result exceeding MAX_FETCH_IDS must map to quota-exceeded: {resp:?}"
    );
}

/// A1: a caller holding a real capability entitling `employee`'s
/// `view_self` permission resolves to their own row via the existing
/// capability-gated sieve -- and the returned `RelationshipProof` verifies
/// against its own `asserter_did`.
#[tokio::test]
async fn resolve_relation_a1_resolves_via_the_capability_gated_sieve_and_verifies() {
    let service_id = "resolve-relation-a1-svc";
    let (route_handler, pipeline, preamble, _temp_dir, _storage_provider, _key_store) =
        resolve_relation_service_and_pipeline(service_id, Some(resolvable_employee_policy())).await;

    let alice = employee_reader_caller("did:key:alice", service_id);
    let body = resolve_relation_body("employee", "did:key:alice");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "resolve-relation must succeed: {resp:?}");
    let result = &resp["result"];
    assert_eq!(result["ids"], json!(["emp-alice"]), "A1 must resolve only alice's own row");
    assert_eq!(result["relation"], "employee");
    assert_eq!(result["principal"], "did:key:alice");

    let asserter_did = result["asserter_did"].as_str().unwrap();
    let signature = result["signature"].as_str().unwrap();
    let mut unsigned = result.clone();
    unsigned["signature"] = json!("");
    syneroym_identity::substrate::verify_json_signature(asserter_did, &unsigned, signature)
        .expect("the returned proof must verify against its own asserter_did");
}

/// Two services co-hosted on the same node (same `node_identity`, same
/// `owner_did`) but with distinct `service_id`s must sign their
/// `RelationshipProof`s under distinct `asserter_did`s -- the multi-tenancy
/// concern ADR-0017 §6/§7's "`hr-svc` asserts..." model is meant to
/// address: a shared node-wide signing identity would make every co-hosted
/// service's assertions cryptographically indistinguishable.
#[tokio::test]
async fn resolve_relation_co_hosted_services_sign_with_distinct_asserter_dids() {
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let owner_did = "did:key:zSharedOwner";

    let (hr_handler, hr_pipeline, hr_preamble, _hr_dir, _hr_sp, _hr_ks) =
        resolve_relation_service_and_pipeline_with(
            "hr-svc",
            Some(resolvable_employee_policy()),
            node_identity.clone(),
            owner_did,
        )
        .await;
    let (
        finance_handler,
        finance_pipeline,
        finance_preamble,
        _finance_dir,
        _finance_sp,
        _finance_ks,
    ) = resolve_relation_service_and_pipeline_with(
        "finance-svc",
        Some(resolvable_employee_policy()),
        node_identity,
        owner_did,
    )
    .await;

    let bob = zero_capability_caller("did:key:bob");
    let body = resolve_relation_body("employee", "did:key:bob");

    let hr_resp = hr_handler
        .dispatch_json_rpc_once(&hr_pipeline, &hr_preamble, Some(&bob), &body)
        .await
        .unwrap();
    let hr_resp: Value = serde_json::from_slice(&hr_resp).unwrap();
    assert!(hr_resp.get("error").is_none(), "resolve-relation must succeed: {hr_resp:?}");
    let hr_asserter = hr_resp["result"]["asserter_did"].as_str().unwrap();

    let finance_resp = finance_handler
        .dispatch_json_rpc_once(&finance_pipeline, &finance_preamble, Some(&bob), &body)
        .await
        .unwrap();
    let finance_resp: Value = serde_json::from_slice(&finance_resp).unwrap();
    assert!(finance_resp.get("error").is_none(), "resolve-relation must succeed: {finance_resp:?}");
    let finance_asserter = finance_resp["result"]["asserter_did"].as_str().unwrap();

    assert_ne!(
        hr_asserter, finance_asserter,
        "two co-hosted services under the same owner must sign as distinct asserter_dids"
    );

    // Each proof must verify only against its own service's asserter_did.
    let mut hr_unsigned = hr_resp["result"].clone();
    hr_unsigned["signature"] = json!("");
    let hr_signature = hr_resp["result"]["signature"].as_str().unwrap();
    syneroym_identity::substrate::verify_json_signature(hr_asserter, &hr_unsigned, hr_signature)
        .expect("hr-svc's proof must verify against its own asserter_did");
    assert!(
        syneroym_identity::substrate::verify_json_signature(
            finance_asserter,
            &hr_unsigned,
            hr_signature
        )
        .is_err(),
        "hr-svc's proof must not verify under finance-svc's asserter_did"
    );
}

/// A `service_id` freed by undeploy and redeployed under a **different**
/// owner must not inherit the old owner's signing key: same node, same
/// `service_id`, different `owner_did` must still derive distinct
/// `asserter_did`s. Otherwise a stale `RelationshipProof` cached from the
/// old tenancy would still verify under the new tenant's identity.
#[tokio::test]
async fn resolve_relation_service_id_reused_by_a_different_owner_signs_distinctly() {
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let service_id = "reused-service-id-svc";

    let (old_handler, old_pipeline, old_preamble, _old_dir, _old_sp, _old_ks) =
        resolve_relation_service_and_pipeline_with(
            service_id,
            Some(resolvable_employee_policy()),
            node_identity.clone(),
            "did:key:zOldOwner",
        )
        .await;
    let (new_handler, new_pipeline, new_preamble, _new_dir, _new_sp, _new_ks) =
        resolve_relation_service_and_pipeline_with(
            service_id,
            Some(resolvable_employee_policy()),
            node_identity,
            "did:key:zNewOwner",
        )
        .await;

    let bob = zero_capability_caller("did:key:bob");
    let body = resolve_relation_body("employee", "did:key:bob");

    let old_resp = old_handler
        .dispatch_json_rpc_once(&old_pipeline, &old_preamble, Some(&bob), &body)
        .await
        .unwrap();
    let old_resp: Value = serde_json::from_slice(&old_resp).unwrap();
    let old_asserter = old_resp["result"]["asserter_did"].as_str().unwrap();

    let new_resp = new_handler
        .dispatch_json_rpc_once(&new_pipeline, &new_preamble, Some(&bob), &body)
        .await
        .unwrap();
    let new_resp: Value = serde_json::from_slice(&new_resp).unwrap();
    let new_asserter = new_resp["result"]["asserter_did"].as_str().unwrap();

    assert_ne!(
        old_asserter, new_asserter,
        "a service_id reused under a different owner must derive a distinct asserter_did"
    );
}

/// B3-07: a capability scoped to a completely unrelated resource must not
/// change the answer relative to holding zero capabilities -- it routes to
/// A2 (structural resolution), the same as `zero_capability_caller` would,
/// not to a real-but-irrelevant A1 grant check.
#[tokio::test]
async fn resolve_relation_an_unrelated_resource_capability_still_gets_a2() {
    let service_id = "resolve-relation-unrelated-resource-svc";
    let (route_handler, pipeline, preamble, _temp_dir, _storage_provider, _key_store) =
        resolve_relation_service_and_pipeline(service_id, Some(resolvable_employee_policy())).await;

    let caller = unrelated_resource_capability_caller("did:key:alice", service_id);
    let body = resolve_relation_body("employee", "did:key:alice");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "resolve-relation must succeed: {resp:?}");
    assert_eq!(
        resp["result"]["ids"],
        json!(["emp-alice"]),
        "an unrelated-resource capability must not block A2's structural resolution: {resp:?}"
    );
}

/// The real A1 deny (a capability scoped to `employees` but for an ability
/// `view_self` doesn't cover) is final -- it must **not** be rescued by
/// A2, even though the definition has opted into
/// `resolvable_without_capability`. Mutually exclusive per request, not a
/// fallback chain.
#[tokio::test]
async fn resolve_relation_a1_deny_is_not_rescued_by_a2() {
    let service_id = "resolve-relation-a1-deny-svc";
    let (route_handler, pipeline, preamble, _temp_dir, _storage_provider, _key_store) =
        resolve_relation_service_and_pipeline(service_id, Some(resolvable_employee_policy())).await;

    let mallory = wrong_ability_on_the_right_resource_caller("did:key:alice", service_id);
    let body = resolve_relation_body("employee", "did:key:alice");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&mallory), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "resolve-relation must still succeed with an empty set");
    assert_eq!(
        resp["result"]["ids"],
        json!([]),
        "an unrelated capability must not trigger A1 grant, nor fall through to A2: {resp:?}"
    );
}

/// A2: a caller holding **zero** capabilities structurally resolves via
/// the bare `principal_column` match, since `employee` opted into
/// `resolvable_without_capability`.
#[tokio::test]
async fn resolve_relation_a2_resolves_structurally_with_zero_capabilities() {
    let service_id = "resolve-relation-a2-svc";
    let (route_handler, pipeline, preamble, _temp_dir, _storage_provider, _key_store) =
        resolve_relation_service_and_pipeline(service_id, Some(resolvable_employee_policy())).await;

    let bob = zero_capability_caller("did:key:bob");
    let body = resolve_relation_body("employee", "did:key:bob");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&bob), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "resolve-relation must succeed: {resp:?}");
    assert_eq!(resp["result"]["ids"], json!(["emp-bob"]), "A2 must resolve bob's own row");
}

/// A caller with zero capabilities against a definition that has **not**
/// opted into `resolvable_without_capability` gets an empty result --
/// neither A1 (no capabilities to evaluate) nor A2 (not opted in) applies.
#[tokio::test]
async fn resolve_relation_denies_when_not_opted_in_and_no_capabilities() {
    let service_id = "resolve-relation-not-opted-in-svc";
    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "employee": {"table": "employees", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap();
    let (route_handler, pipeline, preamble, _temp_dir, _storage_provider, _key_store) =
        resolve_relation_service_and_pipeline(service_id, Some(policy)).await;

    let bob = zero_capability_caller("did:key:bob");
    let body = resolve_relation_body("employee", "did:key:bob");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&bob), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none());
    assert_eq!(resp["result"]["ids"], json!([]));
}

/// A `relation` naming no definition at all must deny outright, never fall
/// through to `ServiceStore::query`'s ordinary no-definition-means-
/// unfiltered pass-through -- there is no grant-layer admission backing a
/// cross-service relationship ask the way there is for an ordinary read.
#[tokio::test]
async fn resolve_relation_denies_for_an_undeclared_relation_not_unfiltered() {
    let service_id = "resolve-relation-undeclared-svc";
    let (route_handler, pipeline, preamble, _temp_dir, _storage_provider, _key_store) =
        resolve_relation_service_and_pipeline(service_id, Some(resolvable_employee_policy())).await;

    let bob = zero_capability_caller("did:key:bob");
    let body = resolve_relation_body("nonexistent_relation", "did:key:bob");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&bob), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none());
    assert_eq!(
        resp["result"]["ids"],
        json!([]),
        "an unrecognized relation name must never leak an unfiltered dump: {resp:?}"
    );
}

/// `principal` is a caller-declared label that must match the wire
/// caller's own re-verified identity -- a caller cannot ask about a
/// different principal's relationships.
#[tokio::test]
async fn resolve_relation_denies_when_principal_does_not_match_the_caller() {
    let service_id = "resolve-relation-mismatch-svc";
    let (route_handler, pipeline, preamble, _temp_dir, _storage_provider, _key_store) =
        resolve_relation_service_and_pipeline(service_id, Some(resolvable_employee_policy())).await;

    let alice = employee_reader_caller("did:key:alice", service_id);
    let body = resolve_relation_body("employee", "did:key:bob");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(
        resp["error"]["code"], -32010,
        "asking about a principal other than the verified caller must be denied: {resp:?}"
    );
}

/// No policy deployed: nothing to resolve against, an empty (not error)
/// result -- the same "no definition" treatment an unpoliced service gives
/// every other FDAE-aware method.
#[tokio::test]
async fn resolve_relation_is_empty_when_no_policy_is_deployed() {
    let service_id = "resolve-relation-no-policy-svc";
    let (route_handler, pipeline, preamble, _temp_dir, _storage_provider, _key_store) =
        resolve_relation_service_and_pipeline(service_id, None).await;

    let bob = zero_capability_caller("did:key:bob");
    let body = resolve_relation_body("employee", "did:key:bob");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&bob), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none());
    assert_eq!(resp["result"]["ids"], json!([]));
}

/// (same caller as
/// `resolve_relation_a1_resolves_via_the_capability_gated_sieve_and_verifies`)
/// is denied outright once `view_self` opts into stage 4 -- the remote must
/// not be able to route around this node's after-step by resolving
/// structurally instead of through the direct, after-step-aware read path.
#[tokio::test]
async fn resolve_relation_a1_denies_closed_under_a_stage4_definition() {
    let service_id = "resolve-relation-a1-stage4-svc";
    let (route_handler, pipeline, preamble, _temp_dir, _storage_provider, _key_store) =
        resolve_relation_service_and_pipeline(
            service_id,
            Some(resolvable_employee_policy_with_stage4()),
        )
        .await;

    let alice = employee_reader_caller("did:key:alice", service_id);
    let body = resolve_relation_body("employee", "did:key:alice");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(
        resp["error"]["code"], -32010,
        "an A1 resolution under a stage-4 definition must deny closed, not return an \
         after-step-unfiltered id-set: {resp:?}"
    );
}

/// A2: the bare `principal_column` match (`resolvable_without_capability`)
/// bypasses the sieve entirely, so it would otherwise be the wider hole of
/// the two -- same deny under the same stage-4 definition.
#[tokio::test]
async fn resolve_relation_a2_denies_closed_under_a_stage4_definition() {
    let service_id = "resolve-relation-a2-stage4-svc";
    let (route_handler, pipeline, preamble, _temp_dir, _storage_provider, _key_store) =
        resolve_relation_service_and_pipeline(
            service_id,
            Some(resolvable_employee_policy_with_stage4()),
        )
        .await;

    let bob = zero_capability_caller("did:key:bob");
    let body = resolve_relation_body("employee", "did:key:bob");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&bob), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(
        resp["error"]["code"], -32010,
        "an A2 resolution under a stage-4 definition must deny closed, not return an \
         after-step-unfiltered id-set: {resp:?}"
    );
}
