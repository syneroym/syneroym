use std::fs;

use syneroym_core::test_constants::{
    GREETER_INTERFACE_NAME, STREAM_TEST_DRIVER_INTERFACE, greeter_wasm_path, stream_test_wasm_path,
};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    HttpProbe as WitHttpProbe, RpcProbe as WitRpcProbe, ServiceConfig, TcpProbe as WitTcpProbe,
};

use super::{super::*, helpers::*};

/// A4-13: nothing before this pinned that `run_probe`'s `HttpGet` arm
/// ever actually reaches a listener -- the only other `HttpGet` tests in
/// this file check deploy-time rejection.
#[tokio::test]
async fn an_http_probe_passes_on_the_expected_status() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let port =
        serve_http_responses("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await;

    let manifest = tcp_manifest_with(
        port,
        Some(WitHealthCheck::HttpGet(WitHttpProbe {
            interface_name: "main".to_string(),
            path: "/healthz".to_string(),
            expect_status: 200,
            timeout_ms: 2000,
        })),
    );
    service.deploy("http-ok-svc".to_string(), manifest, &node_wide_caller("owner")).await.unwrap();
    let status =
        service.status(vec!["http-ok-svc".to_string()], &node_wide_caller("owner")).await.unwrap();
    assert!(
        matches!(status.services[0].probe, ProbeStatus::Passing),
        "{:?}",
        status.services[0].probe
    );
}

#[tokio::test]
async fn an_http_probe_fails_on_an_unexpected_status() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let port = serve_http_responses(
        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    )
    .await;

    let manifest = tcp_manifest_with(
        port,
        Some(WitHealthCheck::HttpGet(WitHttpProbe {
            interface_name: "main".to_string(),
            path: "/healthz".to_string(),
            expect_status: 200,
            timeout_ms: 2000,
        })),
    );
    service.deploy("http-503-svc".to_string(), manifest, &node_wide_caller("owner")).await.unwrap();
    let status =
        service.status(vec!["http-503-svc".to_string()], &node_wide_caller("owner")).await.unwrap();
    assert!(
        matches!(&status.services[0].probe, ProbeStatus::Failing(d) if d.contains("got 503")),
        "{:?}",
        status.services[0].probe
    );
}

/// A4-12: a hostile or compromised container answering a probe with a
/// redirect must not make the substrate follow it -- a readiness check
/// has no reason to, and `reqwest`'s default policy (up to ten hops)
/// would otherwise let the probed service steer this substrate's own
/// requests (SSRF).
#[tokio::test]
async fn an_http_probe_does_not_follow_a_redirect() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let port = serve_http_responses(
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/elsewhere\r\nContent-Length: \
         0\r\nConnection: close\r\n\r\n",
    )
    .await;

    let manifest = tcp_manifest_with(
        port,
        Some(WitHealthCheck::HttpGet(WitHttpProbe {
            interface_name: "main".to_string(),
            path: "/healthz".to_string(),
            expect_status: 200,
            timeout_ms: 2000,
        })),
    );
    service
        .deploy("http-redirect-svc".to_string(), manifest, &node_wide_caller("owner"))
        .await
        .unwrap();
    let status = service
        .status(vec!["http-redirect-svc".to_string()], &node_wide_caller("owner"))
        .await
        .unwrap();
    assert!(
        matches!(&status.services[0].probe, ProbeStatus::Failing(d) if d.contains("got 302")),
        "the probe must report the redirect itself, not follow it: {:?}",
        status.services[0].probe
    );
}

/// A4-14: `run_probe`'s four error branches, each worded distinctly and
/// read straight off `app health` by an operator -- none were reachable
/// from the suite before this.
#[tokio::test]
async fn run_probe_reports_an_unreadable_stored_health_check() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    service
        .registry
        .set_deploy_facts(
            "bad-check-svc".to_string(),
            "tcp".to_string(),
            Some("not valid json".to_string()),
            None,
            None,
        )
        .await
        .unwrap();
    service
        .registry
        .register(
            "bad-check-svc".to_string(),
            "main".to_string(),
            SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port: 9 },
        )
        .await
        .unwrap();

    let status = service
        .status(vec!["bad-check-svc".to_string()], &status_capable_caller("owner"))
        .await
        .unwrap();
    assert!(
        matches!(&status.services[0].probe, ProbeStatus::Failing(d) if d.contains("stored health check is unreadable")),
        "{:?}",
        status.services[0].probe
    );
}

#[tokio::test]
async fn run_probe_reports_no_endpoint_for_the_declared_interface() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    service
        .registry
        .set_deploy_facts(
            "no-endpoint-svc".to_string(),
            "tcp".to_string(),
            Some(
                serde_json::to_string(&WitHealthCheck::TcpConnect(WitTcpProbe {
                    interface_name: "main".to_string(),
                    timeout_ms: 1000,
                }))
                .unwrap(),
            ),
            None,
            None,
        )
        .await
        .unwrap();
    // Registered under a different interface, so the service is visible
    // at all -- but deliberately no `registry.register(...)` call for
    // "main", the interface the health check actually names.
    service
        .registry
        .register(
            "no-endpoint-svc".to_string(),
            "other".to_string(),
            SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port: 9 },
        )
        .await
        .unwrap();

    let status = service
        .status(vec!["no-endpoint-svc".to_string()], &status_capable_caller("owner"))
        .await
        .unwrap();
    assert!(
        matches!(&status.services[0].probe, ProbeStatus::Failing(d) if d.contains("no endpoint registered for interface 'main'")),
        "{:?}",
        status.services[0].probe
    );
}

#[tokio::test]
async fn run_probe_reports_a_non_tcp_endpoint_under_tcp_connect() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    service
        .registry
        .set_deploy_facts(
            "non-tcp-endpoint-svc".to_string(),
            // "nativehost" so `instance_phase` reports `Unknown` (the
            // probe runs) without this test also having to fake a real
            // running instance -- "wasm" would report `NotRunning` and
            // skip the probe entirely, before ever reaching `run_probe`.
            "nativehost".to_string(),
            Some(
                serde_json::to_string(&WitHealthCheck::TcpConnect(WitTcpProbe {
                    interface_name: "main".to_string(),
                    timeout_ms: 1000,
                }))
                .unwrap(),
            ),
            None,
            None,
        )
        .await
        .unwrap();
    service
        .registry
        .register(
            "non-tcp-endpoint-svc".to_string(),
            "main".to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "non-tcp-endpoint-svc".to_string() },
        )
        .await
        .unwrap();

    let status = service
        .status(vec!["non-tcp-endpoint-svc".to_string()], &status_capable_caller("owner"))
        .await
        .unwrap();
    assert!(
        matches!(&status.services[0].probe, ProbeStatus::Failing(d) if d.contains("is not a TCP endpoint")),
        "{:?}",
        status.services[0].probe
    );
}

#[tokio::test]
async fn run_probe_reports_a_non_tcp_endpoint_under_http_get() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    service
        .registry
        .set_deploy_facts(
            "non-tcp-http-svc".to_string(),
            // See the identical note in the tcp-connect version of this
            // test just above.
            "nativehost".to_string(),
            Some(
                serde_json::to_string(&WitHealthCheck::HttpGet(WitHttpProbe {
                    interface_name: "main".to_string(),
                    path: "/healthz".to_string(),
                    expect_status: 200,
                    timeout_ms: 1000,
                }))
                .unwrap(),
            ),
            None,
            None,
        )
        .await
        .unwrap();
    service
        .registry
        .register(
            "non-tcp-http-svc".to_string(),
            "main".to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "non-tcp-http-svc".to_string() },
        )
        .await
        .unwrap();

    let status = service
        .status(vec!["non-tcp-http-svc".to_string()], &status_capable_caller("owner"))
        .await
        .unwrap();
    assert!(
        matches!(&status.services[0].probe, ProbeStatus::Failing(d) if d.contains("is not a TCP endpoint")),
        "{:?}",
        status.services[0].probe
    );
}

#[tokio::test]
async fn a_probe_is_not_run_for_an_instance_that_is_not_running() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    // A `wasm` service with a recorded type but nothing loaded --
    // `instance_phase` reports `NotRunning`, and the probe must not run
    // for a fault the substrate already knows about.
    service
        .registry
        .set_deploy_facts(
            "not-running-svc".to_string(),
            "wasm".to_string(),
            Some(
                serde_json::to_string(&WitHealthCheck::Rpc(WitRpcProbe {
                    interface_name: "main".to_string(),
                    method: "ping".to_string(),
                    timeout_ms: 1000,
                }))
                .unwrap(),
            ),
            None,
            None,
        )
        .await
        .unwrap();
    service
        .registry
        .register(
            "not-running-svc".to_string(),
            "main".to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "not-running-svc".to_string() },
        )
        .await
        .unwrap();

    let status = service
        .status(vec!["not-running-svc".to_string()], &status_capable_caller("owner"))
        .await
        .unwrap();
    assert_eq!(status.services.len(), 1);
    assert!(matches!(status.services[0].phase, InstancePhase::NotRunning(_)));
    assert!(matches!(status.services[0].probe, ProbeStatus::NotDeclared));
}

/// A4-13: nothing before this pinned that `run_probe`'s `Rpc` arm ever
/// actually reaches a running guest -- every other `Rpc` test in this
/// file deploys `Binary(vec![])` (deliberately fake, for deploy-time
/// rejection only) or skips deploy entirely via `set_deploy_facts`. This
/// one deploys the real `greeter` fixture and lets the probe genuinely
/// invoke it.
///
/// Post-review (N-1): `run_probe`'s `Rpc` arm always sends
/// `params: Value::Array(vec![])`, so a probe method that takes a
/// required argument -- `greeter`'s own `greet(name: string)` -- can
/// never pass; `json_to_wasm_params`'s `default_for_missing`
/// (`sandbox_wasm/src/conversions.rs`) errors for any non-`Option`
/// parameter the array doesn't supply. That is a real, permanent
/// `ProbeFailing` an operator would read as a live outage, not a
/// declaration mistake -- recorded in `deferred-backlog.md` rather than
/// fixed here, since the real fix (a `params` field on `rpc-probe`, or
/// deploy-time introspection of the guest's exported signature) is a
/// WIT/schema decision of its own, not a drive-by change. Pinned
/// explicitly here rather than left as an accidentally-green test.
#[tokio::test]
async fn an_rpc_probe_permanently_fails_for_a_method_that_takes_a_required_argument() {
    let wasm_bytes = fs::read(greeter_wasm_path())
        .expect("greeter fixture must be built (see test-components/greeter's own build step)");
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let owner = node_wide_caller("owner");

    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: Some(WitHealthCheck::Rpc(WitRpcProbe {
                interface_name: GREETER_INTERFACE_NAME.to_string(),
                method: "greet".to_string(),
                timeout_ms: 2000,
            })),
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
    service.deploy("greeter-svc".to_string(), manifest, &owner).await.unwrap();

    let status = service.status(vec!["greeter-svc".to_string()], &owner).await.unwrap();
    assert_eq!(status.services.len(), 1);
    assert!(
        matches!(&status.services[0].probe, ProbeStatus::Failing(d) if d.contains("missing required parameter")),
        "{:?}",
        status.services[0].probe
    );
}

/// The genuine happy path N-1 found missing: a real `rpc` probe against
/// a method that takes no arguments must report `Passing`.
/// `stream-test`'s `get-uploaded-content` (its own `test-driver`
/// interface) takes none and is side-effect-free against an empty
/// store.
#[tokio::test]
async fn an_rpc_probe_passes_for_a_method_that_takes_no_arguments() {
    let wasm_bytes = fs::read(stream_test_wasm_path()).expect(
        "stream-test fixture must be built (see test-components/stream-test's own build step)",
    );
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let owner = node_wide_caller("owner");

    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: Some(WitHealthCheck::Rpc(WitRpcProbe {
                interface_name: STREAM_TEST_DRIVER_INTERFACE.to_string(),
                method: "get-uploaded-content".to_string(),
                timeout_ms: 2000,
            })),
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(wasm_bytes),
            hash: None,
            interfaces: vec![STREAM_TEST_DRIVER_INTERFACE.to_string()],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    service.deploy("stream-test-svc".to_string(), manifest, &owner).await.unwrap();

    let status = service.status(vec!["stream-test-svc".to_string()], &owner).await.unwrap();
    assert_eq!(status.services.len(), 1);
    assert!(
        matches!(status.services[0].probe, ProbeStatus::Passing),
        "{:?}",
        status.services[0].probe
    );
}

/// The health-poll-cost budget, measured
/// **before** the resident loop exists so `poll_interval_secs`'s
/// default is chosen from this number rather than defended after it.
/// "One in-process node" (one real `ControlPlaneService`, dispatched
/// directly -- no client, no network) with 20 real `rpc`-probed wasm
/// services, all cache-missing on this, their first sweep --
/// `probe_cached`'s 5s minimum interval means every sweep at the
/// default 30s `poll_interval_secs` pays this cost. Every
/// target's probe already runs concurrently, so this also pins that
/// the batching holds at 20 rather than degrading linearly.
///
/// Budget, set a priori: **under 2s** for the whole pass. Two related
/// numbers (**at most 2 RPCs per substrate**, and
/// **under 5% of one core**) are not asserted here: the RPC count is
/// this test's own shape by construction -- one `status` call for all
/// 20 ids, exactly what a supervisor's sweep issues, with the second
/// RPC (`app-instance-management-of`) being an O(1) generation read
/// unrelated to service count. That second RPC's own "exactly one
/// call per substrate, not per service" half is a separate,
/// dedicated regression test at the call site
/// (`max_held_generation_from_clients_calls_held_generation_once_per_alias`,
/// `crates/app_supervisor/src/service.rs`, drives
/// `SupervisorService::max_held_generation_from_clients` against a
/// counting fake and pins the call count directly). CPU-percent is
/// not portably measurable from a `#[tokio::test]` -- a wall-clock
/// budget comfortably under 2s on a shared thread pool is the proxy
/// for it here, with `mise run bench:poll-cost` re-running this same
/// test with its duration printed for a repeatable number outside
/// the pass/fail assertion.
#[tokio::test]
async fn a_steady_state_sweep_of_twenty_services_stays_within_the_poll_budget() {
    let wasm_bytes = fs::read(stream_test_wasm_path()).expect(
        "stream-test fixture must be built (see test-components/stream-test's own build step)",
    );
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let owner = node_wide_caller("owner");

    let mut service_ids = Vec::new();
    for i in 0..20 {
        let id = format!("budget-svc-{i}");
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: Some(WitHealthCheck::Rpc(WitRpcProbe {
                    interface_name: STREAM_TEST_DRIVER_INTERFACE.to_string(),
                    method: "get-uploaded-content".to_string(),
                    timeout_ms: 2000,
                })),
                assets: None,
                visibility: None,
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(wasm_bytes.clone()),
                hash: None,
                interfaces: vec![STREAM_TEST_DRIVER_INTERFACE.to_string()],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };
        service.deploy(id.clone(), manifest, &owner).await.unwrap();
        service_ids.push(id);
    }

    let start = std::time::Instant::now();
    let status = service.status(service_ids, &owner).await.unwrap();
    let elapsed = start.elapsed();

    assert_eq!(status.services.len(), 20);
    for s in &status.services {
        assert!(matches!(s.probe, ProbeStatus::Passing), "{:?}", s.probe);
    }
    eprintln!("D-A5c-12 poll-cost budget: 20-service sweep took {elapsed:?} (budget 2s)");
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "a 20-service sweep took {elapsed:?}, over the 2s budget (D-A5c-12)"
    );
}

#[tokio::test]
async fn a_probe_result_is_cached_within_the_minimum_interval() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    // See the comment in `a_probe_runs_for_a_tcp_service_whose_phase_is_unknown`:
    // no accept loop needed, and a blocking `accept()` inside
    // `tokio::spawn` on this single-threaded test runtime would starve
    // the runtime instead of servicing connections.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    service
        .registry
        .register(
            "cached-probe-svc".to_string(),
            "main".to_string(),
            SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port },
        )
        .await
        .unwrap();
    service
        .registry
        .set_deploy_facts(
            "cached-probe-svc".to_string(),
            "tcp".to_string(),
            Some(
                serde_json::to_string(&WitHealthCheck::TcpConnect(WitTcpProbe {
                    interface_name: "main".to_string(),
                    timeout_ms: 2000,
                }))
                .unwrap(),
            ),
            None,
            None,
        )
        .await
        .unwrap();

    let now = 1_000_000;
    let (first, first_at) = service.probe_cached("cached-probe-svc", now).await;
    assert!(matches!(first, ProbeStatus::Passing));
    // Within the interval: served from cache, same `checked_at`.
    let (second, second_at) = service.probe_cached("cached-probe-svc", now + 1).await;
    assert!(matches!(second, ProbeStatus::Passing));
    assert_eq!(first_at, second_at);
}
