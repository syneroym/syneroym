#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use syneroym_core::test_constants;

mod proxy_harness;
use proxy_harness::*;

#[tokio::test]
async fn batch_of_mixed_calls_through_proxy_matches_single_call_answers() {
    let route_handler =
        test_route_handler_with_proxy_components().await.expect("wasm artifacts not built");

    let nested_params = json!({
        "service": "proxy-callee",
        "interface": test_constants::GREETER_INTERFACE_NAME,
        "method": "greet",
        "params": "[\"World\"]",
        "target-kind": "service",
    })
    .to_string();

    let calls = json!([
        ["service", "proxy-callee", test_constants::GREETER_INTERFACE_NAME, "greet", "[\"World\"]"],
        [
            "service",
            "nonexistent-svc",
            test_constants::GREETER_INTERFACE_NAME,
            "greet",
            "[\"World\"]"
        ],
        ["service", "some-other-svc", "data-layer", "get", "{}"],
        [
            "dependency",
            "unbound-dep",
            test_constants::GREETER_INTERFACE_NAME,
            "greet",
            "[\"World\"]"
        ],
        [
            "service",
            "proxy-caller",
            test_constants::PROXY_TEST_DRIVER_INTERFACE,
            "call-peer",
            nested_params
        ],
        ["service", "proxy-callee", test_constants::GREETER_INTERFACE_NAME, "greet", "[\"World\"]"]
    ]);

    let body = json_rpc_body("call-peers", json!({ "calls": calls }));
    let response_bytes = route_handler
        .dispatch_json_rpc_once(&call_peer_pipeline(), &call_peer_preamble(), None, &body)
        .await
        .unwrap();
    let response: Value = serde_json::from_slice(&response_bytes).unwrap();
    assert!(response.get("error").is_none(), "batch failed: {response:?}");

    let results =
        response.get("result").and_then(Value::as_array).expect("expected array of results");
    assert_eq!(results.len(), 6);

    assert!(results[0]["ok"].as_str().unwrap().contains("Hello, World!"));
    assert!(results[1]["err"].as_str().unwrap().contains("ServiceNotFound"));
    assert!(results[2]["err"].as_str().unwrap().contains("PermissionDenied"));
    assert!(results[3]["err"].as_str().unwrap().contains("DependencyNotBound"));
    assert!(results[4]["ok"].as_str().unwrap().contains("Hello, World!"));
    assert!(results[5]["ok"].as_str().unwrap().contains("Hello, World!"));

    let single_body = json_rpc_body(
        "call-peer",
        json!({
            "service": "proxy-callee",
            "interface": test_constants::GREETER_INTERFACE_NAME,
            "method": "greet",
            "params": "[\"World\"]",
            "target-kind": "service",
        }),
    );
    let single_res: Value = serde_json::from_slice(
        &route_handler
            .dispatch_json_rpc_once(
                &call_peer_pipeline(),
                &call_peer_preamble(),
                None,
                &single_body,
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(results[0]["ok"], single_res["result"]);

    let single_body = json_rpc_body(
        "call-peer",
        json!({
            "service": "nonexistent-svc",
            "interface": test_constants::GREETER_INTERFACE_NAME,
            "method": "greet",
            "params": "[\"World\"]",
            "target-kind": "service",
        }),
    );
    let single_res: Value = serde_json::from_slice(
        &route_handler
            .dispatch_json_rpc_once(
                &call_peer_pipeline(),
                &call_peer_preamble(),
                None,
                &single_body,
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(single_res["error"]["message"].as_str().unwrap().contains("ServiceNotFound"));

    let single_body = json_rpc_body(
        "call-peer",
        json!({
            "service": "some-other-svc",
            "interface": "data-layer",
            "method": "get",
            "params": "{}",
            "target-kind": "service",
        }),
    );
    let single_res: Value = serde_json::from_slice(
        &route_handler
            .dispatch_json_rpc_once(
                &call_peer_pipeline(),
                &call_peer_preamble(),
                None,
                &single_body,
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(single_res["error"]["message"].as_str().unwrap().contains("PermissionDenied"));
}

#[tokio::test]
async fn batch_over_the_maximum_is_refused() {
    let route_handler =
        test_route_handler_with_proxy_components().await.expect("wasm artifacts not built");

    let single_call = json!([
        "service",
        "proxy-callee",
        test_constants::GREETER_INTERFACE_NAME,
        "greet",
        "[\"World\"]"
    ]);
    let calls_17 = vec![single_call; 17];

    let body = json_rpc_body("call-peers", json!({ "calls": calls_17 }));
    let response_bytes = route_handler
        .dispatch_json_rpc_once(&call_peer_pipeline(), &call_peer_preamble(), None, &body)
        .await
        .unwrap();
    let response: Value = serde_json::from_slice(&response_bytes).unwrap();
    let err_msg = response["error"]["message"].as_str().unwrap_or_default();
    assert!(
        err_msg.contains("call-all takes at most 16 calls, got 17"),
        "expected refusal error message, got: {response:?}"
    );
}

#[tokio::test]
async fn keyed_call_in_batch_timing_out_within_grace_records_dead_letter() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = test_substrate_config(temp_dir.path());
    let (route_handler, outbox) = test_route_handler_with_outbox(config, temp_dir.path())
        .await
        .expect("wasm artifacts not built");

    let call = json!({
        "kind": "service",
        "service": "sleeper-service",
        "interface": "sleeper",
        "method": "sleep",
        "params": "{}",
        "timeout-ms": 200,
        "idempotency-key": "dead-letter-test-batch-key"
    });

    let body = json_rpc_body("call-peers-full", json!({ "calls": [call] }));
    let response_bytes = route_handler
        .dispatch_json_rpc_once(&call_peer_pipeline(), &call_peer_preamble(), None, &body)
        .await
        .unwrap();
    let response: Value = serde_json::from_slice(&response_bytes).unwrap();
    assert!(response.get("error").is_none(), "expected batch response, got: {response:?}");

    let results = response["result"].as_array().expect("expected array of results");
    assert_eq!(results.len(), 1);
    assert!(
        results[0]["err"].as_str().unwrap_or_default().contains("TimedOut"),
        "expected call to time out, got: {:?}",
        results[0]
    );

    let dead = outbox.dead_letters("proxy-caller").await.expect("read dead letters");
    assert_eq!(dead.len(), 1, "expected 1 dead letter, got: {dead:?}");
    assert_eq!(dead[0].idempotency_key, "dead-letter-test-batch-key");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_wasm_callees_execute_in_parallel_via_joinset() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = test_substrate_config(temp_dir.path());
    if let Some(role) = config.roles.app_sandbox.as_mut() {
        role.max_concurrent_instances = 8;
    }
    let route_handler = test_route_handler_with_config(config, temp_dir.path())
        .await
        .expect("wasm artifacts not built");

    let calls = json!([
        [
            "service",
            "proxy-spinner",
            test_constants::PROXY_TEST_DRIVER_INTERFACE,
            "spin-for",
            "[2000]"
        ],
        [
            "service",
            "proxy-caller",
            test_constants::PROXY_TEST_DRIVER_INTERFACE,
            "spin-for",
            "[2000]"
        ]
    ]);

    let body = json_rpc_body("call-peers", json!({ "calls": calls }));
    let start = std::time::Instant::now();
    let response_bytes = route_handler
        .dispatch_json_rpc_once(&call_peer_pipeline(), &call_peer_preamble(), None, &body)
        .await
        .unwrap();
    let elapsed = start.elapsed();

    let response: Value = serde_json::from_slice(&response_bytes).unwrap();
    assert!(response.get("error").is_none(), "batch failed: {response:?}");
    let results = response["result"].as_array().expect("results array");
    assert_eq!(results.len(), 2);
    assert!(results[0]["ok"].is_string());
    assert!(results[1]["ok"].is_string());

    assert!(
        elapsed < std::time::Duration::from_millis(3600),
        "expected parallel execution in ~2s, but took {:?}",
        elapsed
    );
    assert!(
        elapsed >= std::time::Duration::from_millis(1800),
        "expected execution to take at least 1.8s, took {:?}",
        elapsed
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 10)]
async fn local_wasm_callees_pool_exhaustion_is_bounded() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = test_substrate_config(temp_dir.path());
    if let Some(role) = config.roles.app_sandbox.as_mut() {
        role.max_concurrent_instances = 6;
    }
    let route_handler = test_route_handler_with_config(config, temp_dir.path())
        .await
        .expect("wasm artifacts not built");

    let mut calls = Vec::new();
    for _ in 0..10 {
        calls.push(json!([
            "service",
            "proxy-spinner",
            test_constants::PROXY_TEST_DRIVER_INTERFACE,
            "spin-for",
            "[1000]"
        ]));
    }

    let body = json_rpc_body("call-peers", json!({ "calls": calls }));
    let response_bytes = route_handler
        .dispatch_json_rpc_once(&call_peer_pipeline(), &call_peer_preamble(), None, &body)
        .await
        .unwrap();
    let response: Value = serde_json::from_slice(&response_bytes).unwrap();
    assert!(response.get("error").is_none(), "batch failed: {response:?}");
    let results = response["result"].as_array().expect("results array");
    assert_eq!(results.len(), 10);

    let has_callee_err = results.iter().any(|r| {
        r["err"].as_str().is_some_and(|msg| msg.contains("-32603") || msg.contains("Callee"))
    });
    assert!(
        has_callee_err,
        "expected some calls to fail with callee error (-32603) due to pool exhaustion, got: \
         {results:?}"
    );

    let single_body = json_rpc_body(
        "call-peer",
        json!({
            "service": "proxy-callee",
            "interface": test_constants::GREETER_INTERFACE_NAME,
            "method": "greet",
            "params": "[\"World\"]",
            "target-kind": "service",
        }),
    );
    let single_res: Value = serde_json::from_slice(
        &route_handler
            .dispatch_json_rpc_once(
                &call_peer_pipeline(),
                &call_peer_preamble(),
                None,
                &single_body,
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(single_res["result"].as_str().unwrap().contains("Hello, World!"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn epoch_ticker_on_dedicated_thread_prevents_clock_starvation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = test_substrate_config(temp_dir.path());
    if let Some(role) = config.roles.app_sandbox.as_mut() {
        role.max_concurrent_instances = 6;
        role.dispatch_epoch_timeout_secs = 1;
        role.default_max_instructions = Some(500_000_000_000);
    }
    let route_handler = test_route_handler_with_config(config, temp_dir.path())
        .await
        .expect("wasm artifacts not built");

    let calls = json!([
        [
            "service",
            "proxy-spinner",
            test_constants::PROXY_TEST_DRIVER_INTERFACE,
            "spin-for",
            "[4294967295]"
        ],
        [
            "service",
            "proxy-caller",
            test_constants::PROXY_TEST_DRIVER_INTERFACE,
            "spin-for",
            "[4294967295]"
        ]
    ]);

    let body = json_rpc_body("call-peers", json!({ "calls": calls }));
    let start = std::time::Instant::now();
    let response_bytes = route_handler
        .dispatch_json_rpc_once(&call_peer_pipeline(), &call_peer_preamble(), None, &body)
        .await
        .unwrap();
    let elapsed = start.elapsed();

    let response: Value = serde_json::from_slice(&response_bytes).unwrap();
    assert!(
        elapsed < std::time::Duration::from_millis(3500),
        "expected epoch interrupt within 3.5s, took {:?}",
        elapsed
    );
    assert!(
        elapsed >= std::time::Duration::from_millis(800),
        "expected at least 800ms, took {:?}",
        elapsed
    );

    let results = response["result"].as_array().expect("results array");
    assert_eq!(results.len(), 2);
    for r in results {
        let err_str = r["err"].as_str().unwrap_or_default();
        assert!(
            err_str.contains("TimedOut")
                || err_str.contains("Deadline")
                || err_str.contains("interrupt")
                || err_str.contains("Callee")
                || err_str.contains("-32603"),
            "expected timeout/deadline/interrupt/callee, got: {err_str}"
        );
        assert!(!err_str.contains("fuel"), "must not be fuel exhaustion, got: {err_str}");
    }
}
