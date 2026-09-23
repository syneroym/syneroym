use syneroym_app_orchestration::models::{AppBlueprintId, TopologyMode};
use syneroym_core::util;

use super::{super::*, helpers::*};

/// The document names every member in member-index order.
#[tokio::test]
async fn resolve_returns_a_document_naming_every_member_master_did_in_index_order() {
    let s = service();
    let plan_json = plan_json_n_member_service("inst-1", "backend", "redundant", 3);
    let app_did = adopted_instance(&s, "inst-1", &plan_json).await;

    let resp = dispatch(
        &s,
        resolve_grant("did:key:zOutsideCaller", &app_did),
        "resolve",
        serde_json::json!([app_did, "backend"]),
    )
    .await
    .unwrap();
    let signed = decode_signed_document(resp.payload);

    assert_eq!(
        signed.document.members,
        vec![
            ServiceId::new("did:key:hMember0"),
            ServiceId::new("did:key:hMember1"),
            ServiceId::new("did:key:hMember2"),
        ]
    );
    assert_eq!(signed.document.mode, TopologyMode::Redundant);
    assert!(signed.verify(&AppDid::new(app_did)).is_ok());
}

/// The property the gateway's own check depends on -- the signed
/// document never echoes the hash back, always the real name, even
/// when the caller supplied the hash. Also pins that the epoch is
/// still carried and preserved on a hashed request.
#[tokio::test]
async fn resolve_answers_a_hashed_service_name_with_a_document_naming_the_real_name() {
    let s = service();
    let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
    let app_did = adopted_instance(&s, "inst-1", &plan_json).await;
    let name_hash = util::short_hash("backend");

    let by_name = dispatch(
        &s,
        resolve_grant("did:key:zOutsideCaller", &app_did),
        "resolve",
        serde_json::json!([app_did, "backend"]),
    )
    .await
    .unwrap();
    let by_hash = dispatch(
        &s,
        resolve_grant("did:key:zOutsideCaller", &app_did),
        "resolve",
        serde_json::json!([app_did, name_hash]),
    )
    .await
    .unwrap();

    let signed_by_hash = decode_signed_document(by_hash.payload);
    let signed_by_name = decode_signed_document(by_name.payload);
    assert_eq!(signed_by_hash.document.service_name.as_str(), "backend");
    assert_eq!(signed_by_hash.document.epoch, signed_by_name.document.epoch);
    assert!(signed_by_hash.verify(&AppDid::new(app_did)).is_ok());
}

/// Matrix row 7: a caller holding no grant for this app is refused.
#[tokio::test]
async fn resolve_refuses_a_caller_holding_no_grant_for_this_app() {
    let s = service();
    let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
    let app_did = adopted_instance(&s, "inst-1", &plan_json).await;

    let err = dispatch(
        &s,
        caller_with_no_capabilities("did:key:zOutsideCaller"),
        "resolve",
        serde_json::json!([app_did, "backend"]),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), PERMISSION_DENIED_CODE);
}

/// An ungranted caller resolving an `open` service receives the signed
/// document.
#[tokio::test]
async fn resolve_open_service_answers_ungranted_caller() {
    let s = service();
    let plan_json =
        plan_json_n_member_service_with_vis("inst-1", "backend", "singleton", 1, "public", "open");
    let app_did = adopted_instance(&s, "inst-1", &plan_json).await;

    let resp = dispatch(
        &s,
        caller_with_no_capabilities("did:key:zOutsideCaller"),
        "resolve",
        serde_json::json!([app_did, "backend"]),
    )
    .await
    .unwrap();
    let signed = decode_signed_document(resp.payload);
    assert_eq!(signed.document.members, vec![ServiceId::new("did:key:hMember0")]);
    assert_eq!(signed.document.mode, TopologyMode::Singleton);
    assert!(signed.verify(&AppDid::new(app_did)).is_ok());
}

/// An ungranted caller naming a non-existent service gets the same
/// refusal as an unauthorized call.
#[tokio::test]
async fn resolve_open_service_nonexistent_service_refuses_identically_for_ungranted_caller() {
    let s = service();
    let plan_json =
        plan_json_n_member_service_with_vis("inst-1", "backend", "singleton", 1, "public", "open");
    let app_did = adopted_instance(&s, "inst-1", &plan_json).await;

    let err = dispatch(
        &s,
        caller_with_no_capabilities("did:key:zOutsideCaller"),
        "resolve",
        serde_json::json!([app_did, "nonexistent"]),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), PERMISSION_DENIED_CODE);
}

/// An ungranted caller resolving an `open` service on a retired
/// instance is refused.
#[tokio::test]
async fn resolve_open_service_on_retired_instance_is_refused() {
    let s = service();
    let plan_json =
        plan_json_n_member_service_with_vis("inst-1", "backend", "singleton", 1, "public", "open");
    let app_did = adopted_instance(&s, "inst-1", &plan_json).await;
    s.store.retire("inst-1").unwrap();

    let err = dispatch(
        &s,
        caller_with_no_capabilities("did:key:zOutsideCaller"),
        "resolve",
        serde_json::json!([app_did, "backend"]),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), PERMISSION_DENIED_CODE);
}

/// A granted caller naming a non-existent service gets InvalidParams,
/// not denied.
#[tokio::test]
async fn resolve_granted_caller_naming_nonexistent_service_returns_invalid_params() {
    let s = service();
    let plan_json =
        plan_json_n_member_service_with_vis("inst-1", "backend", "singleton", 1, "public", "open");
    let app_did = adopted_instance(&s, "inst-1", &plan_json).await;

    let err = dispatch(
        &s,
        resolve_grant("did:key:zOutsideCaller", &app_did),
        "resolve",
        serde_json::json!([app_did, "nonexistent"]),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, RpcError::InvalidParams(_)), "expected InvalidParams, got {err:?}");
}

/// The document served to an ungranted `open` caller is byte-identical
/// to the one served to a granted caller.
#[tokio::test]
async fn resolve_open_service_served_to_ungranted_caller_is_identical_to_granted_caller() {
    let s = service();
    let plan_json =
        plan_json_n_member_service_with_vis("inst-1", "backend", "singleton", 1, "public", "open");
    let app_did = adopted_instance(&s, "inst-1", &plan_json).await;

    let granted_resp = dispatch(
        &s,
        resolve_grant("did:key:zGrantedCaller", &app_did),
        "resolve",
        serde_json::json!([app_did, "backend"]),
    )
    .await
    .unwrap();
    let ungranted_resp = dispatch(
        &s,
        caller_with_no_capabilities("did:key:zUngrantedCaller"),
        "resolve",
        serde_json::json!([app_did, "backend"]),
    )
    .await
    .unwrap();

    let granted_signed = decode_signed_document(granted_resp.payload);
    let ungranted_signed = decode_signed_document(ungranted_resp.payload);

    assert_eq!(granted_signed.document, ungranted_signed.document);
    assert_eq!(granted_signed.signature, ungranted_signed.signature);
}

/// `topology_visibility = open` and `visibility = private` are not
/// redundant, and neither alone proves the other is unnecessary. The
/// compiler refuses this combination at both of its entry points
/// (`compile()` and `handle_submit`) precisely because, left to reach
/// a supervisor, `open` alone is sufficient for `handle_resolve` to
/// hand out the document -- the visibility read at
/// [`SupervisorService::handle_resolve`]'s top does not consult
/// `config.visibility` at all, only `topology_visibility`. This test
/// goes around both refusal points the way `adopted_instance` always
/// does (`s.store.submit` directly, not the RPC `submit` that calls
/// `handle_submit`) to prove that half of the claim as an actual
/// runtime behaviour, not just an absence of a compile-time refusal.
///
/// The other half -- that the member named in this document is then
/// unreachable -- is proven separately, structurally:
/// `member_registry_record_mints_nothing_for_a_private_member`
/// (`crates/sdk/src/deploy.rs`) shows a `private` member gets no
/// registry record at all, so nothing a caller could look up ever
/// exists to dial. Reproducing that failure here as well would need a
/// live registry and a live gateway dial, which is
/// `gateway_hostname_e2e.rs`'s job for the `(open, internal)` pair.
/// The two halves come from two different mechanisms.
#[tokio::test]
async fn resolve_open_service_over_a_private_member_still_serves_the_document() {
    let s = service();
    let plan_json =
        plan_json_n_member_service_with_vis("inst-1", "backend", "singleton", 1, "private", "open");
    let app_did = adopted_instance(&s, "inst-1", &plan_json).await;

    let resp = dispatch(
        &s,
        caller_with_no_capabilities("did:key:zOutsideCaller"),
        "resolve",
        serde_json::json!([app_did, "backend"]),
    )
    .await
    .unwrap();
    let signed = decode_signed_document(resp.payload);
    assert_eq!(signed.document.members, vec![ServiceId::new("did:key:hMember0")]);
    assert!(signed.verify(&AppDid::new(app_did)).is_ok());
}

/// The probing guard: an unknown app and an unauthorized caller are
/// reported identically, asserted on the exact error string.
/// `record_adopt` writes the row's `app_master_did`
/// directly (bypassing the vault) so both branches can share one
/// literal DID and one literal caller DID, making the two error
/// strings byte-comparable.
#[tokio::test]
async fn resolve_reports_an_unknown_app_and_an_unauthorized_caller_identically() {
    const SHARED_DID: &str = "did:key:zSharedForComparison";
    const SHARED_CALLER: &str = "did:key:zOutsideCaller";

    // Branch 1: no instance anywhere claims this DID.
    let unknown_app = service();
    let err_unknown = dispatch(
        &unknown_app,
        resolve_grant(SHARED_CALLER, SHARED_DID),
        "resolve",
        serde_json::json!([SHARED_DID, "backend"]),
    )
    .await
    .unwrap_err();

    // Branch 2: the app exists, under the same DID, but the caller
    // holds no grant for it.
    let unauthorized = service();
    let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
    unauthorized.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
    unauthorized.store.record_adopt("inst-1", 1, SHARED_DID).unwrap();
    let err_unauthorized = dispatch(
        &unauthorized,
        caller_with_no_capabilities(SHARED_CALLER),
        "resolve",
        serde_json::json!([SHARED_DID, "backend"]),
    )
    .await
    .unwrap_err();

    assert_eq!(err_unknown.to_string(), err_unauthorized.to_string());
    assert!(err_unknown.to_string().contains(SHARED_DID));
    assert!(err_unknown.to_string().contains(SHARED_CALLER));
}

/// A refusal carries no member DIDs at all -- the document is built
/// whole or not at all, and the authorization check runs before it is
/// built.
#[tokio::test]
async fn a_refused_resolve_carries_no_member_dids_at_all() {
    let s = service();
    let plan_json = serde_json::json!({
        "app_instance_id": "inst-1",
        "blueprint_id": "syneroym:test",
        "version": "1.0.0",
        "services": [{
            "service_id": "did:key:hVerySecretMember",
            "logical_ref": "inst-1/backend",
            "substrate": "edge-1",
            "service_type": "tcp", "source": "127.0.0.1:9000",
            "rotation_policy": "none",
            "resolved_dependencies": {},
            "topology_mode": "singleton",
        }],
    })
    .to_string();
    let app_did = adopted_instance(&s, "inst-1", &plan_json).await;

    let err = dispatch(
        &s,
        caller_with_no_capabilities("did:key:zOutsideCaller"),
        "resolve",
        serde_json::json!([app_did, "backend"]),
    )
    .await
    .unwrap_err();
    assert!(!err.to_string().contains("hVerySecretMember"), "{err}");
}

/// A locked vault fails loudly and names `inject-kek`, never a silent
/// empty answer.
#[tokio::test]
async fn resolve_on_a_locked_vault_fails_loudly_and_names_inject_kek() {
    let s = service_with_locked_vault();
    let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
    s.store.record_adopt("inst-1", 1, "did:key:zPlaceholderAppMaster").unwrap();

    let err = dispatch(
        &s,
        resolve_grant("did:key:zOutsideCaller", "did:key:zPlaceholderAppMaster"),
        "resolve",
        serde_json::json!(["did:key:zPlaceholderAppMaster", "backend"]),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("inject-kek"), "{err}");
    let active = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
    assert!(active.iter().any(|a| a.kind == AlertKind::VaultLocked));
}

/// The supervisor signs once per `(service, epoch)` and serves the
/// cached document afterwards -- asserted on the signature bytes being
/// identical across two calls.
#[tokio::test]
async fn resolve_signs_once_per_epoch_and_serves_the_cached_document_afterwards() {
    let s = service();
    let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
    let app_did = adopted_instance(&s, "inst-1", &plan_json).await;
    let caller = resolve_grant("did:key:zOutsideCaller", &app_did);

    let first = decode_signed_document(
        dispatch(&s, caller.clone(), "resolve", serde_json::json!([app_did, "backend"]))
            .await
            .unwrap()
            .payload,
    );
    let second = decode_signed_document(
        dispatch(&s, caller, "resolve", serde_json::json!([app_did, "backend"]))
            .await
            .unwrap()
            .payload,
    );
    assert_eq!(first.signature, second.signature, "a cache hit must not re-sign");
    assert_eq!(first.document.issued_at, second.document.issued_at);
}

/// A cached document is re-signed once less than half its validity
/// remains, so a served copy always outlives a caller's own
/// cache TTL rather than being served right up to the moment it
/// expires.
#[tokio::test]
async fn a_nearly_expired_cached_document_is_re_signed_rather_than_served() {
    let s = Fixture { topology_document_not_after_secs: Some(2), ..Fixture::default() }.build();
    let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
    let app_did = adopted_instance(&s, "inst-1", &plan_json).await;
    let caller = resolve_grant("did:key:zOutsideCaller", &app_did);

    let first = decode_signed_document(
        dispatch(&s, caller.clone(), "resolve", serde_json::json!([app_did, "backend"]))
            .await
            .unwrap()
            .payload,
    );

    // Past half of the 2s validity, with no membership change.
    tokio::time::sleep(Duration::from_millis(1_100)).await;

    let second = decode_signed_document(
        dispatch(&s, caller, "resolve", serde_json::json!([app_did, "backend"]))
            .await
            .unwrap()
            .payload,
    );
    assert_ne!(first.signature, second.signature, "a nearly-expired document must re-sign");
    assert!(second.document.issued_at >= first.document.issued_at);
    assert!(second.document.not_after > first.document.not_after);
    assert_eq!(first.document.epoch, second.document.epoch, "membership did not change");
}

/// A membership change re-signs the document at the new epoch. The
/// store's own epoch/fingerprint update (which
/// `handle_submit` performs after a real resubmit) is driven directly
/// here, since a real resubmit needs a live substrate connection this
/// unit test has no reason to stand up.
#[tokio::test]
async fn a_membership_change_re_signs_the_document_at_the_new_epoch() {
    let s = service();
    let one_member = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
    let app_did = adopted_instance(&s, "inst-1", &one_member).await;
    let caller = resolve_grant("did:key:zOutsideCaller", &app_did);

    let first = decode_signed_document(
        dispatch(&s, caller.clone(), "resolve", serde_json::json!([app_did, "backend"]))
            .await
            .unwrap()
            .payload,
    );
    assert_eq!(first.document.epoch, TopologyEpoch(1));

    // A scale-out: the stored plan now names two members, and the
    // fingerprint is advanced the way `handle_submit` would after a
    // real resubmit.
    let two_members = plan_json_n_member_service("inst-1", "backend", "redundant", 2);
    s.store.submit("inst-1", &two_members, "{}", "did:key:owner", 1).unwrap();
    let plan = DeploymentPlan::from_json(&two_members).unwrap();
    let topo = topology::service_topology(&plan, &LogicalServiceName::new("backend")).unwrap();
    let fp = topology_fingerprint(topo.mode, &topo.members, topo.sharding_strategy.as_ref());
    s.store.record_topology_fingerprint("inst-1", "backend", &fp).unwrap();

    let second = decode_signed_document(
        dispatch(&s, caller, "resolve", serde_json::json!([app_did, "backend"]))
            .await
            .unwrap()
            .payload,
    );
    assert_eq!(second.document.epoch, TopologyEpoch(2));
    assert_eq!(second.document.members.len(), 2);
    assert_ne!(first.signature, second.signature);
}

/// F1: the document cache is keyed only by `(app_instance_id,
/// service_name)`, so a handover that reassigns `app_master_did`
/// (`import-master` + `adopt`, simulated here at the store level --
/// the same shortcut the test above uses for a resubmit) must not let
/// a caller asking for the *new* DID be served a document cached
/// under the *old* one, which would carry the wrong `app_did` and
/// fail every caller's `verify`.
#[tokio::test]
async fn a_handover_to_a_different_app_did_does_not_serve_the_previous_masters_cached_document() {
    let s = service();
    let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
    let old_app_did = adopted_instance(&s, "inst-1", &plan_json).await;

    let first = decode_signed_document(
        dispatch(
            &s,
            resolve_grant("did:key:zOutsideCaller", &old_app_did),
            "resolve",
            serde_json::json!([old_app_did, "backend"]),
        )
        .await
        .unwrap()
        .payload,
    );
    assert_eq!(first.document.app_did, AppDid::new(old_app_did.clone()));

    // The row-level effect of `import-master` + `adopt`: the same
    // instance, a different recorded app master DID, no vault key
    // rotation (a real handover also imports a new vault key -- the
    // resulting mismatch, and its alert, are `resolve_on_a_locked_
    // vault_fails_loudly_and_names_inject_kek`'s sibling test, not
    // this one's concern). Generation is held at `1`, unchanged from
    // `adopted_instance`'s own call, so this isolates the app DID as
    // the only thing that moved -- `record_adopt` is a plain `UPDATE`
    // with no monotonic guard on generation, so re-recording the same
    // one is accepted. Bumping it here as well would let the
    // `generation` clause alone force the cache miss this test means
    // to pin on `app_did`.
    let new_app_did = "did:key:zHandedOverMaster";
    s.store.record_adopt("inst-1", 1, new_app_did).unwrap();

    let err = dispatch(
        &s,
        resolve_grant("did:key:zOutsideCaller", new_app_did),
        "resolve",
        serde_json::json!([new_app_did, "backend"]),
    )
    .await
    .unwrap_err();
    // Proves the stale document was not served: a cache hit would
    // have returned `Ok` with `first`'s bytes. Instead the fresh-sign
    // path ran and correctly refused, since the vault's real key
    // still derives `old_app_did`, not `new_app_did`.
    assert!(err.to_string().contains("does not match its recorded app master DID"), "{err}");
}

/// F4: the cache hit condition did not compare `generation`, so a
/// second `adopt` of the same app master -- no membership change, no
/// `AppDid` change -- could serve a document carrying the *previous*
/// generation, the field ADR-0022 §2 gives a reader to tell two
/// supervisors' documents apart.
#[tokio::test]
async fn a_generation_bump_with_no_membership_change_is_not_served_from_a_stale_cache() {
    let s = service();
    let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
    let app_did = adopted_instance(&s, "inst-1", &plan_json).await;
    let caller = resolve_grant("did:key:zOutsideCaller", &app_did);

    let first = decode_signed_document(
        dispatch(&s, caller.clone(), "resolve", serde_json::json!([app_did, "backend"]))
            .await
            .unwrap()
            .payload,
    );
    assert_eq!(first.document.generation, 1);

    // Same app master, same plan -- only the generation moves, the
    // way a second `adopt` of an already-adopted instance would.
    s.store.record_adopt("inst-1", 2, &app_did).unwrap();

    let second = decode_signed_document(
        dispatch(&s, caller, "resolve", serde_json::json!([app_did, "backend"]))
            .await
            .unwrap()
            .payload,
    );
    assert_eq!(second.document.generation, 2);
    assert_ne!(first.signature, second.signature, "a stale generation must not be served");
    assert_eq!(first.document.epoch, second.document.epoch, "membership did not change");
}

/// F2 (and F6, the same fix): `initialise_topology_epoch`'s
/// insert-only form can never correct an existing row's fingerprint,
/// so a row left holding one that disagrees with the real plan --
/// whether from an earlier `submit`'s fingerprint write landing
/// wrong, or a genuine concurrent `submit` -- used to exhaust both
/// lock-free attempts and fail permanently. The locked repair path
/// settles it instead.
#[tokio::test]
async fn resolve_repairs_a_topology_epoch_row_stuck_on_the_wrong_fingerprint() {
    let s = service();
    let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
    let app_did = adopted_instance(&s, "inst-1", &plan_json).await;

    // A row that disagrees with what `service_topology` actually
    // computes for the stored plan -- the state a lock-free
    // `resolve` can never fix on its own.
    s.store.record_topology_fingerprint("inst-1", "backend", "garbage-fingerprint").unwrap();

    let doc = decode_signed_document(
        dispatch(
            &s,
            resolve_grant("did:key:zOutsideCaller", &app_did),
            "resolve",
            serde_json::json!([app_did, "backend"]),
        )
        .await
        .unwrap()
        .payload,
    );
    // The repair path's advancing write bumps the epoch past the
    // garbage row's `1`, since the stored fingerprint did not match.
    assert_eq!(doc.document.epoch, TopologyEpoch(2));
}

/// The WIT record and the serde struct are two descriptions of one
/// wire format, and nothing else stops them drifting.
#[test]
fn the_resolve_payloads_json_keys_match_the_wit_records_field_names() {
    let (resolve, iface_id) = supervisor_interface();
    let iface = &resolve.interfaces[iface_id];
    let record_ty = *iface.types.get("topology-document").expect("no topology-document type");
    let wit_parser::TypeDefKind::Record(record) = &resolve.types[record_ty].kind else {
        panic!("topology-document is not a record");
    };
    let wit_fields: BTreeSet<String> =
        record.fields.iter().map(|f| f.name.replace('-', "_")).collect();

    // The fixture must declare a sharding_strategy: it is
    // skip_serializing_if = "Option::is_none", so a fixture without
    // one omits the key and the comparison would read a real name
    // match as a mismatch.
    let doc = TopologyDocument {
        app_instance_id: AppInstanceId::new("inst-1"),
        app_did: AppDid::new("did:key:zApp"),
        service_name: LogicalServiceName::new("backend"),
        mode: TopologyMode::Sharded,
        members: vec![ServiceId::new("did:key:zM0")],
        sharding_strategy: Some(ShardingStrategy::HashSharding),
        epoch: TopologyEpoch(1),
        generation: 0,
        issued_at: 0,
        not_after: 0,
        cache_ttl_ms: 0,
    };
    let value = serde_json::to_value(&doc).unwrap();
    let json_keys: BTreeSet<String> = value.as_object().unwrap().keys().cloned().collect();

    assert_eq!(wit_fields, json_keys);
}

/// An instance with no app master DID yet, the same skip
/// `refresh_due_app_tier1_record` makes.
#[tokio::test]
async fn resolve_is_refused_for_an_instance_that_has_no_app_master_did() {
    let s = service();
    let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
    // Submitted, never adopted -- `app_master_did` stays empty.
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();

    let err = dispatch(
        &s,
        resolve_grant("did:key:zOutsideCaller", "did:key:zNeverAssigned"),
        "resolve",
        serde_json::json!(["did:key:zNeverAssigned", "backend"]),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), PERMISSION_DENIED_CODE);
}

/// `resolve` answers for a paused instance and refuses for a retired
/// one, in one test because the decision is the contrast.
#[tokio::test]
async fn resolve_answers_for_a_paused_instance_and_refuses_for_a_retired_one() {
    let paused = service();
    let plan_json = plan_json_n_member_service("inst-1", "backend", "singleton", 1);
    let paused_did = adopted_instance(&paused, "inst-1", &plan_json).await;
    paused.store.pause("inst-1").unwrap();
    let resp = dispatch(
        &paused,
        resolve_grant("did:key:zOutsideCaller", &paused_did),
        "resolve",
        serde_json::json!([paused_did, "backend"]),
    )
    .await;
    assert!(resp.is_ok(), "pause stops the resident loop, not the members");

    let retired = service();
    let retired_did = adopted_instance(&retired, "inst-1", &plan_json).await;
    retired.store.retire("inst-1").unwrap();
    let err = dispatch(
        &retired,
        resolve_grant("did:key:zOutsideCaller", &retired_did),
        "resolve",
        serde_json::json!([retired_did, "backend"]),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), PERMISSION_DENIED_CODE);
}

/// Following `refuse_replicas_above_cap`'s existing refuse/allow pair
/// exactly.
#[test]
fn refuse_unshardable_plan_refuses_a_hand_authored_range_sharding_plan() {
    use syneroym_app_orchestration::resolver::{RangeChunk, RangeRoutingTable};

    let mut svc = dependent_service("backend", "unrelated");
    svc.topology_mode = TopologyMode::Sharded;
    svc.sharding_strategy = Some(ShardingStrategy::RangeSharding(RangeRoutingTable {
        chunks: vec![RangeChunk {
            start_key: None,
            end_key: None,
            target: ServiceId::new("did:key:hShard0"),
        }],
    }));
    let mut svc2 = svc.clone();
    svc2.member_index = 1;
    let plan = DeploymentPlan {
        app_instance_id: AppInstanceId::new("inst-1"),
        blueprint_id: AppBlueprintId::new("syneroym:test"),
        version: semver::Version::new(1, 0, 0),
        services: vec![svc, svc2],
    };
    let err = SupervisorService::refuse_unshardable_plan(&plan).unwrap_err();
    assert!(err.contains("range_sharding"), "{err}");
}
