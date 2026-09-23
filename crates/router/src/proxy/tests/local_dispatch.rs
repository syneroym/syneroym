use super::helpers::*;

// -- local native dispatch -------------------------------------------

#[tokio::test]
async fn invoke_local_native_reaches_registered_service_with_caller_identity() {
    let registry = empty_registry();
    registry
        .register(
            "svc-a".to_string(),
            "data-layer".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "svc-a".to_string() },
        )
        .await
        .unwrap();

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let service = Arc::new(RecordingNativeService::default());
    native_dispatch.insert("svc-a".to_string(), service.clone() as Arc<dyn NativeService>);

    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    let mut req = base_request("svc-a", "data-layer");
    req.caller = test_caller("did:key:zCallerOne");
    let result = router.invoke(req).await.unwrap();
    assert_eq!(result, Value::String("ok".to_string()));
    assert_eq!(service.invoked.load(Ordering::SeqCst), 1);
    assert_eq!(service.last_caller_did.lock().unwrap().as_deref(), Some("did:key:zCallerOne"));
}

// -- unknown service ---------------------------------------------------

#[tokio::test]
async fn unknown_service_is_service_not_found_and_hop_never_called() {
    let hop = Arc::new(MockHop::default());
    let router = test_router(hop.clone(), empty_registry());

    let result = router.invoke(base_request("no-such-service", "greet")).await;
    assert!(matches!(result, Err(ProxyError::ServiceNotFound(_))));
    assert_eq!(hop.call_count(), 0);
}

// -- native capability gate --------------------------------------------

#[tokio::test]
async fn guest_cross_service_native_capability_is_denied_and_never_dispatched() {
    let registry = empty_registry();
    registry
        .register(
            "svc-b".to_string(),
            "data-layer".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "svc-b".to_string() },
        )
        .await
        .unwrap();
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let service = Arc::new(RecordingNativeService::default());
    native_dispatch.insert("svc-b".to_string(), service.clone() as Arc<dyn NativeService>);

    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    let mut req = base_request("svc-b", "data-layer");
    req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
    let result = router.invoke(req).await;
    assert!(matches!(result, Err(ProxyError::PermissionDenied(_))));
    assert_eq!(service.invoked.load(Ordering::SeqCst), 0);
}

/// A guest that requests the interface by its `short_hash` (what
/// `EndpointRegistry::lookup` also accepts and canonicalizes back to the
/// literal name) must be denied exactly like the literal-name request
/// above -- `short_hash` is an unsalted SHA-256 prefix, so it's
/// guest-computable and must not bypass the gate.
#[tokio::test]
async fn guest_cross_service_native_capability_is_denied_via_short_hash_too() {
    let registry = empty_registry();
    registry
        .register(
            "svc-b".to_string(),
            "data-layer".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "svc-b".to_string() },
        )
        .await
        .unwrap();
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let service = Arc::new(RecordingNativeService::default());
    native_dispatch.insert("svc-b".to_string(), service.clone() as Arc<dyn NativeService>);

    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    let mut req = base_request("svc-b", &util::short_hash("data-layer"));
    req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
    let result = router.invoke(req).await;
    assert!(matches!(result, Err(ProxyError::PermissionDenied(_))));
    assert_eq!(service.invoked.load(Ordering::SeqCst), 0);
}

/// A0-01: `orchestrator`/`security` are node-level (registered under the
/// node's own DID, not any deployed service's), so unlike the
/// `NATIVE_CAPABILITY_INTERFACES` gate above there is no same-service
/// exemption -- a guest whose own service is the node's own DID (which
/// cannot legitimately happen, but a guest freely chooses
/// `target_service`) must still be denied. Guards against a guest whose
/// service holds an installed instance certificate (ADR-0020 §1) walking
/// its now-verified identity into `orchestrator` (gated since the
/// deploy-grant admission gate) or `security` (also gated on
/// `substrate/admin` now) -- neither of which a guest could reach at all
/// before that certificate mechanism existed, since a guest-origin call
/// always presented anonymous.
#[tokio::test]
async fn guest_cannot_reach_node_level_orchestrator_or_security_through_the_proxy() {
    let registry = empty_registry();
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let service = Arc::new(RecordingNativeService::default());
    native_dispatch.insert("node-did".to_string(), service.clone() as Arc<dyn NativeService>);

    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    for interface in ["orchestrator", "security"] {
        let mut req = base_request("node-did", interface);
        req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
        let result = router.invoke(req).await;
        assert!(
            matches!(result, Err(ProxyError::PermissionDenied(_))),
            "interface '{interface}' must be denied, got {result:?}"
        );
    }
    assert_eq!(service.invoked.load(Ordering::SeqCst), 0);
}

/// The empty-interface convenience ("the destination resolves the
/// caller's one app-declared interface") is for an
/// external caller that cannot know a service's interface names -- the
/// gateway or coordinator resolving a hostname. A WASM guest always
/// names the interface it wants, so an empty one must be denied before
/// `registry.lookup` gets a chance to resolve it -- the target
/// registers exactly one app-declared interface here, so a resolve
/// would otherwise have succeeded.
#[tokio::test]
async fn guest_with_an_empty_interface_is_denied_before_resolution() {
    let registry = empty_registry();
    registry
        .register(
            "svc-b".to_string(),
            "default".to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "svc-b".to_string() },
        )
        .await
        .unwrap();
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let service = Arc::new(RecordingNativeService::default());
    native_dispatch.insert("svc-b".to_string(), service.clone() as Arc<dyn NativeService>);

    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    let mut req = base_request("svc-b", "");
    req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
    let result = router.invoke(req).await;
    assert!(matches!(result, Err(ProxyError::PermissionDenied(_))), "{result:?}");
    assert_eq!(service.invoked.load(Ordering::SeqCst), 0);
}

/// The case that would fail against a `caller_did`-based comparison
/// instead of the guest's raw `component_id` -- `service_system` puts
/// `"system:svc-a"` in `caller_did`, which would never equal a plain
/// service id.
#[tokio::test]
async fn guest_reaching_its_own_native_capability_is_allowed() {
    let registry = empty_registry();
    registry
        .register(
            "svc-a".to_string(),
            "data-layer".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "svc-a".to_string() },
        )
        .await
        .unwrap();
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let service = Arc::new(RecordingNativeService::default());
    native_dispatch.insert("svc-a".to_string(), service.clone() as Arc<dyn NativeService>);

    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    let mut req = base_request("svc-a", "data-layer");
    req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
    req.caller = CallerContext::service_system("svc-a");
    let result = router.invoke(req).await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(service.invoked.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn guest_reaching_a_non_native_interface_on_another_service_is_allowed() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let router = test_router(hop.clone(), empty_registry());

    let mut req = base_request("svc-b", "some-app-interface");
    req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
    let result = router.invoke_remote_at(&synthetic_addr(), &req).await;
    assert!(result.is_ok());
}

/// The relationship-proof-fetch shape -- a guard against a future
/// tightening of the gate silently re-breaking that fetch.
#[tokio::test]
async fn native_origin_cross_service_data_layer_call_is_allowed_by_the_gate() {
    let registry = empty_registry();
    registry
        .register(
            "svc-b".to_string(),
            "data-layer".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "svc-b".to_string() },
        )
        .await
        .unwrap();
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let service = Arc::new(RecordingNativeService::default());
    native_dispatch.insert("svc-b".to_string(), service.clone() as Arc<dyn NativeService>);

    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    let mut req = base_request("svc-b", "data-layer");
    req.origin = CallOrigin::Native { service_id: None };
    let result = router.invoke(req).await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(service.invoked.load(Ordering::SeqCst), 1);
}

#[test]
fn check_native_capability_gate_refuses_cross_service_signing_call() {
    let registry = EndpointRegistry::new_mock(Arc::new(syneroym_core::storage::MockStorage::new()));
    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Weak::new(),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    let mut self_req = base_request("svc1", "signing");
    self_req.origin = CallOrigin::Guest { service_id: "svc1".to_string() };
    self_req.method = "sign-record".to_string();
    assert!(router.check_native_capability_gate(&self_req).is_ok());

    let mut cross_req = base_request("svc2", "signing");
    cross_req.origin = CallOrigin::Guest { service_id: "svc1".to_string() };
    cross_req.method = "sign-record".to_string();
    let err = router.check_native_capability_gate(&cross_req).unwrap_err();
    assert!(matches!(err, ProxyError::PermissionDenied(msg) if msg.contains("signing")));
}
