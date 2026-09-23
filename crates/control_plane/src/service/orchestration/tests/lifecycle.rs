use std::{fs, sync::Arc};

use syneroym_core::test_constants::{GREETER_INTERFACE_NAME, greeter_wasm_path};
use syneroym_rpc::ProxyError;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::ServiceConfig;

use super::{super::*, helpers::*};

/// A `tcp` service's process runs outside this substrate -- restart
/// must refuse it and say why, rather than silently succeeding, which
/// a supervisor's remediation budget would count as a real attempt.
#[tokio::test]
async fn restart_refuses_a_tcp_service_naming_why() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let owner = node_wide_caller("owner");

    service
        .deploy("tcp-restart-svc".to_string(), inline_manifest(None, None, None), &owner)
        .await
        .unwrap();

    let err = service.restart("tcp-restart-svc".to_string(), 0, &owner).await.unwrap_err();
    assert!(err.contains("tcp") && err.contains("outside this substrate"), "{err}");
}

/// `restart` is a lifecycle action and must be generation-gated
/// exactly like `deploy`/`write-bindings` -- a superseded supervisor
/// must not be able to restart a service it no longer manages.
#[tokio::test]
async fn restart_is_rejected_at_a_lower_generation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(AppContext { generation: 5, ..app_context("app-1", "frontend", vec![]) }),
            &caller,
        )
        .await
        .unwrap();

    let err = service.restart("frontend-svc".to_string(), 3, &caller).await.unwrap_err();
    assert!(err.contains("at generation 5"), "{err}");
}

/// `restart` was the one lifecycle write with no
/// service-owner check -- a scoped grantee for `service_id` could
/// restart a service a *different* caller owns, which `deploy`/
/// `undeploy`/`write-bindings` all already refuse as a takeover.
#[tokio::test]
async fn restart_is_refused_for_a_service_owned_by_another_caller() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let alice = node_wide_caller("did:key:zAlice");

    service
        .deploy("owned-svc".to_string(), inline_manifest(None, None, None), &alice)
        .await
        .unwrap();
    assert_eq!(service.registry.owner_of("owned-svc"), Some("did:key:zAlice".to_string()));

    let bob = scoped_deploy_caller("did:key:zBob", "owned-svc");
    let err = service.restart("owned-svc".to_string(), 0, &bob).await.unwrap_err();
    assert!(err.contains("owned-svc") && err.contains("owned by"), "{err}");
}

/// The boundary of the check above: a node-wide `orchestrator/deploy`
/// grantee -- the shape a supervisor holds -- restarts a
/// service it does not own without being blocked by the new check.
#[tokio::test]
async fn restart_by_a_node_wide_deploy_grantee_ignores_the_service_owner() {
    let wasm_bytes = fs::read(greeter_wasm_path())
        .expect("greeter fixture must be built (see test-components/greeter's own build step)");
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let alice = node_wide_caller("did:key:zAlice");

    let manifest = DeployManifest {
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
            visibility: None,
        },
        service_type: WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(wasm_bytes),
            hash: None,
            interfaces: vec![GREETER_INTERFACE_NAME.to_string()],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    service.deploy("owned-wasm-svc".to_string(), manifest, &alice).await.unwrap();
    assert_eq!(service.registry.owner_of("owned-wasm-svc"), Some("did:key:zAlice".to_string()));

    let bob = node_wide_caller("did:key:zBob");
    service.restart("owned-wasm-svc".to_string(), 0, &bob).await.unwrap();
}

/// A schedule naming a service this node does not host must be refused
/// outright, never handed to the proxy: `invoke_inner` would read the
/// empty local lookup as "remote", resolve the name through the
/// community registry, and dispatch under this node's own key -- with
/// the owner and generation checks both skipped, since neither can see
/// a service the node knows nothing about.
#[tokio::test]
async fn run_scheduled_refuses_a_target_with_no_local_endpoint() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");
    let proxy = Arc::new(RecordingProxy::default());
    wire_service_proxy(&service, &proxy);

    let err = service
        .run_scheduled(
            "elsewhere-svc".to_string(),
            0,
            "scheduled-driver".to_string(),
            "tick".to_string(),
            None,
            &caller,
        )
        .await
        .unwrap_err();
    assert!(err.contains("no local endpoint"), "{err}");
    assert!(
        proxy.last_request.lock().unwrap().is_none(),
        "the proxy must not be reached for a target this node does not host"
    );
}

/// The same refusal for the everyday mistake: the service is deployed
/// here, but the schedule names an interface it does not export.
#[tokio::test]
async fn run_scheduled_refuses_an_interface_the_local_service_does_not_export() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");
    let proxy = Arc::new(RecordingProxy::default());
    wire_service_proxy(&service, &proxy);
    register_local_endpoint(&service, "worker-svc", "some-other-interface").await;

    let err = service
        .run_scheduled(
            "worker-svc".to_string(),
            0,
            "scheduled-driver".to_string(),
            "tick".to_string(),
            None,
            &caller,
        )
        .await
        .unwrap_err();
    assert!(err.contains("scheduled-driver"), "{err}");
    assert!(proxy.last_request.lock().unwrap().is_none());
}

/// `run-scheduled` takes exactly `restart`'s gate -- a caller
/// with no `orchestrator/deploy` grant is refused before the proxy is
/// ever touched.
#[tokio::test]
async fn run_scheduled_is_refused_without_an_orchestrator_deploy_grant() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let no_grant = CallerContext::service_system("no-grant-caller");

    let err = service
        .run_scheduled(
            "unscheduled-svc".to_string(),
            0,
            "scheduled-driver".to_string(),
            "tick".to_string(),
            None,
            &no_grant,
        )
        .await
        .unwrap_err();
    assert!(err.contains("orchestrator/deploy"), "{err}");
}

/// The owner check `restart_impl` carries, applied identically here:
/// a scoped grantee for `service_id`
/// must not run a scheduled task on a service a *different* caller
/// owns.
#[tokio::test]
async fn run_scheduled_is_refused_for_a_service_another_caller_owns() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let alice = node_wide_caller("did:key:zAlice");

    service
        .deploy("owned-svc".to_string(), inline_manifest(None, None, None), &alice)
        .await
        .unwrap();

    let bob = scoped_deploy_caller("did:key:zBob", "owned-svc");
    let err = service
        .run_scheduled(
            "owned-svc".to_string(),
            0,
            "scheduled-driver".to_string(),
            "tick".to_string(),
            None,
            &bob,
        )
        .await
        .unwrap_err();
    assert!(err.contains("owned-svc") && err.contains("owned by"), "{err}");
}

/// `generation` follows `restart`'s rule: gated only where an
/// app instance exists, so a superseded supervisor cannot keep firing
/// ticks at an instance another one now manages.
#[tokio::test]
async fn run_scheduled_is_refused_at_a_stale_generation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(AppContext { generation: 5, ..app_context("app-1", "frontend", vec![]) }),
            &caller,
        )
        .await
        .unwrap();

    let err = service
        .run_scheduled(
            "frontend-svc".to_string(),
            3,
            "scheduled-driver".to_string(),
            "tick".to_string(),
            None,
            &caller,
        )
        .await
        .unwrap_err();
    assert!(err.contains("at generation 5"), "{err}");
}

/// The whole authorization argument for scheduled ticks: the target
/// observes `CallerContext::service_system(service_id)` -- the service
/// acting as itself -- not the supervisor's own identity, and the call
/// travels as `CallOrigin::Native` with the dispatching service named.
#[tokio::test]
async fn run_scheduled_dispatches_the_named_method_as_the_service_itself() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");
    let proxy = Arc::new(RecordingProxy::default());
    wire_service_proxy(&service, &proxy);
    register_local_endpoint(&service, "worker-svc", "scheduled-driver").await;
    *proxy.response.lock().unwrap() = Some(Ok(Value::Null));

    service
        .run_scheduled(
            "worker-svc".to_string(),
            0,
            "scheduled-driver".to_string(),
            "tick".to_string(),
            Some(r#"["arg"]"#.to_string()),
            &caller,
        )
        .await
        .unwrap();

    let req = proxy.last_request.lock().unwrap().take().expect("proxy was not invoked");
    assert_eq!(req.target_service, "worker-svc");
    assert_eq!(req.interface, "scheduled-driver");
    assert_eq!(req.method, "tick");
    assert_eq!(req.params, serde_json::json!(["arg"]));
    assert_eq!(req.caller.caller_did, "system:worker-svc");
    assert_eq!(req.origin, CallOrigin::Native { service_id: Some("worker-svc".to_string()) });
    assert!(!req.idempotent);
    assert_eq!(req.idempotency_key, None);
}

/// Absent `params-json` sends an empty positional array,
/// not `Value::Null` -- the shape the one existing in-tree caller of a
/// no-argument guest method (the `rpc` readiness probe) sends.
#[tokio::test]
async fn run_scheduled_passes_absent_params_as_an_empty_positional_array() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");
    let proxy = Arc::new(RecordingProxy::default());
    wire_service_proxy(&service, &proxy);
    register_local_endpoint(&service, "worker-svc", "scheduled-driver").await;
    *proxy.response.lock().unwrap() = Some(Ok(Value::Null));

    service
        .run_scheduled(
            "worker-svc".to_string(),
            0,
            "scheduled-driver".to_string(),
            "tick".to_string(),
            None,
            &caller,
        )
        .await
        .unwrap();

    let req = proxy.last_request.lock().unwrap().take().expect("proxy was not invoked");
    assert_eq!(req.params, Value::Array(vec![]));
}

/// A hand-edited or malformed `params-json` is refused before ever
/// reaching the proxy, with a message naming the field.
#[tokio::test]
async fn run_scheduled_refuses_params_json_that_is_not_json() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");
    let proxy = Arc::new(RecordingProxy::default());
    wire_service_proxy(&service, &proxy);

    let err = service
        .run_scheduled(
            "worker-svc".to_string(),
            0,
            "scheduled-driver".to_string(),
            "tick".to_string(),
            Some("not json".to_string()),
            &caller,
        )
        .await
        .unwrap_err();
    assert!(err.contains("params-json is not JSON"), "{err}");
    assert!(proxy.last_request.lock().unwrap().is_none(), "the proxy must not be reached");
}

/// The callee's own error surfaces to the caller rather than being
/// swallowed -- the direct statement that a scheduled run's failure is
/// visible, which the alert this slice raises depends on.
#[tokio::test]
async fn run_scheduled_reports_a_callee_error_rather_than_swallowing_it() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");
    let proxy = Arc::new(RecordingProxy::default());
    wire_service_proxy(&service, &proxy);
    register_local_endpoint(&service, "worker-svc", "scheduled-driver").await;
    *proxy.response.lock().unwrap() = Some(Err(ProxyError::Callee {
        code: -32010,
        message: "guest refused".to_string(),
        data: None,
    }));

    let err = service
        .run_scheduled(
            "worker-svc".to_string(),
            0,
            "scheduled-driver".to_string(),
            "tick".to_string(),
            None,
            &caller,
        )
        .await
        .unwrap_err();
    assert!(err.contains("guest refused"), "{err}");
}

/// The blast-radius half at the substrate level: a
/// superseded supervisor must not be able to undeploy -- the most
/// destructive lifecycle action there is -- a service it no longer
/// manages.
#[tokio::test]
async fn undeploy_is_rejected_at_a_lower_generation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(AppContext { generation: 5, ..app_context("app-1", "frontend", vec![]) }),
            &caller,
        )
        .await
        .unwrap();

    let err = service.undeploy("frontend-svc".to_string(), 3, &caller).await.unwrap_err();
    assert!(err.contains("at generation 5"), "{err}");
}

/// Finding 04 (post-review fix): the app-context/binding write is
/// deferred until every fallible step earlier in the deploy has
/// succeeded (`install_app_context`, called near owner attribution),
/// so a redeploy whose `app_context.service_name` fails validation must
/// leave the previous deploy's bindings completely untouched -- not
/// removed-then-never-replaced, which is what happened when the same
/// removal ran before validation.
#[tokio::test]
async fn a_redeploy_with_an_invalid_service_name_preserves_the_previous_bindings() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("test-caller");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context(
                "app-1",
                "frontend",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &caller,
        )
        .await
        .unwrap();

    // `LogicalServiceName::try_new` rejects a name containing '/'.
    let err = service
        .deploy_with_context(
            "frontend-svc".to_string(),
            inline_manifest(None, None, None),
            Some(app_context("app-1", "bad/name", vec![])),
            &caller,
        )
        .await
        .unwrap_err();
    assert!(err.contains("invalid service name"), "{err}");

    assert_eq!(
        service.registry.app_context_of("frontend-svc"),
        Some(("app-1".to_string(), "frontend".to_string())),
        "the failed redeploy must not have removed the previous app context"
    );
    assert_eq!(
        service.registry.all_bindings().await.unwrap().len(),
        1,
        "the failed redeploy must not have removed the previous binding row"
    );
}
