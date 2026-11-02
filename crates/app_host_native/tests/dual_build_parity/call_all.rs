use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use syneroym_core::config::{AppSandboxRole, SubstrateConfig};

use super::helpers::*;

#[tokio::test]
async fn call_all_empty_list_is_the_same_in_both_builds() {
    let h = harness().await;
    let req = json!({
        "op": "proxy-call-all",
        "calls": []
    })
    .to_string();

    let wasm_res = h.wasm.run(&req).await.unwrap();
    let native_res = h.native.run(&req).await.unwrap();
    assert_eq!(wasm_res, native_res);
    let v: Value = serde_json::from_str(&wasm_res).unwrap();
    assert_eq!(v["ok"]["outcomes"], json!([]));
}

#[tokio::test]
async fn call_all_results_in_input_order_are_the_same() {
    let h = harness().await;
    let req = json!({
        "op": "proxy-call-all",
        "calls": [
            {
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "sleep",
                "params": r#"{"ms":200}"#
            },
            {
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "sleep",
                "params": r#"{"ms":100}"#
            },
            {
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "echo",
                "params": r#"{"msg":"fast"}"#
            }
        ]
    })
    .to_string();

    let wasm_res = h.wasm.run(&req).await.unwrap();
    let native_res = h.native.run(&req).await.unwrap();
    assert_eq!(wasm_res, native_res);
    let v: Value = serde_json::from_str(&wasm_res).unwrap();
    assert_eq!(v["ok"]["outcomes"][0]["ok"], "slept");
    assert_eq!(v["ok"]["outcomes"][1]["ok"], "slept");
    assert_eq!(v["ok"]["outcomes"][2]["ok"], r#"{"msg":"fast"}"#);
}

#[tokio::test]
async fn call_all_good_failed_and_timed_out_calls_give_the_same_outcomes() {
    let h = harness().await;
    let req = json!({
        "op": "proxy-call-all",
        "calls": [
            {
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "echo",
                "params": r#"{"hello":"world"}"#
            },
            {
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "fail",
                "params": "{}"
            },
            {
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "sleep",
                "params": r#"{"ms":1000}"#,
                "timeout_ms": 100
            },
            {
                "kind": "dependency",
                "name": "unbound-dep",
                "interface": "scripted",
                "method": "echo",
                "params": "{}"
            },
            {
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "echo",
                "params": "not valid json"
            }
        ]
    })
    .to_string();

    let wasm_res = h.wasm.run(&req).await.unwrap();
    let native_res = h.native.run(&req).await.unwrap();
    assert_eq!(wasm_res, native_res);
    let v: Value = serde_json::from_str(&wasm_res).unwrap();
    let outcomes = v["ok"]["outcomes"].as_array().unwrap();
    assert_eq!(outcomes.len(), 5);

    assert_eq!(outcomes[0]["ok"], r#"{"hello":"world"}"#);
    assert!(outcomes[1]["err"].as_str().unwrap().contains("Callee"));
    assert!(outcomes[2]["err"].as_str().unwrap().contains("TimedOut"));
    assert!(outcomes[3]["err"].as_str().unwrap().contains("DependencyNotBound"));
    assert!(outcomes[4]["err"].as_str().unwrap().contains("Internal"));
}

#[tokio::test]
async fn call_all_over_the_maximum_is_refused_the_same() {
    let h = harness().await;
    let calls: Vec<_> = (0..17)
        .map(|i| {
            json!({
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "echo",
                "params": format!(r#"{{"i":{i}}}"#)
            })
        })
        .collect();
    let req = json!({
        "op": "proxy-call-all",
        "calls": calls
    })
    .to_string();

    let wasm_res = h.wasm.run(&req).await.unwrap();
    let native_res = h.native.run(&req).await.unwrap();
    assert_eq!(wasm_res, native_res);
    let v: Value = serde_json::from_str(&wasm_res).unwrap();
    let err_str = v["ok"]["refused"].as_str().unwrap();
    assert!(err_str.contains("call-all takes at most 16 calls, got 17"));
}

#[tokio::test]
async fn call_all_runs_at_most_eight_at_once_in_both_builds() {
    let h = harness().await;
    let calls: Vec<_> = (0..16)
        .map(|_| {
            json!({
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "sleep",
                "params": r#"{"ms":100}"#
            })
        })
        .collect();
    let req = json!({
        "op": "proxy-call-all",
        "calls": calls
    })
    .to_string();

    let wasm_res = h.wasm.run(&req).await.unwrap();
    let wasm_max = h._stub_proxy.max_in_flight.load(Ordering::SeqCst);
    h._stub_proxy.reset();

    let native_res = h.native.run(&req).await.unwrap();
    let native_max = h._stub_proxy.max_in_flight.load(Ordering::SeqCst);

    assert_eq!(wasm_res, native_res);
    assert_eq!(wasm_max, 8);
    assert_eq!(native_max, 8);
}

#[tokio::test]
async fn call_all_the_same_target_twice_gives_two_entries() {
    let h = harness().await;
    let req = json!({
        "op": "proxy-call-all",
        "calls": [
            {
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "echo",
                "params": r#"{"call":1}"#
            },
            {
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "echo",
                "params": r#"{"call":1}"#
            }
        ]
    })
    .to_string();

    let wasm_res = h.wasm.run(&req).await.unwrap();
    let native_res = h.native.run(&req).await.unwrap();
    assert_eq!(wasm_res, native_res);
    let v: Value = serde_json::from_str(&wasm_res).unwrap();
    let outcomes = v["ok"]["outcomes"].as_array().unwrap();
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0]["ok"], r#"{"call":1}"#);
    assert_eq!(outcomes[1]["ok"], r#"{"call":1}"#);
}

#[tokio::test]
async fn call_all_non_default_config_is_identical_on_both_builds() {
    let mut config = SubstrateConfig::default();
    config.roles.app_sandbox =
        Some(AppSandboxRole { call_all_max_calls: 4, ..AppSandboxRole::default() });
    let h = harness_with_config(Some(config)).await;

    let calls: Vec<_> = (0..5)
        .map(|i| {
            json!({
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "echo",
                "params": format!(r#"{{"i":{i}}}"#)
            })
        })
        .collect();
    let req = json!({
        "op": "proxy-call-all",
        "calls": calls
    })
    .to_string();

    let wasm_res = h.wasm.run(&req).await.unwrap();
    let native_res = h.native.run(&req).await.unwrap();
    assert_eq!(wasm_res, native_res);
    let v: Value = serde_json::from_str(&wasm_res).unwrap();
    let err_str = v["ok"]["refused"].as_str().unwrap();
    assert!(err_str.contains("call-all takes at most 4 calls, got 5"));
}

struct BatchMutant<'a, D>(&'a D);

impl<D: Driver> Driver for BatchMutant<'_, D> {
    async fn run(&self, request: &str) -> Result<String, String> {
        self.0.run(request).await.map(|s| s.replace("normal", "mutant"))
    }
}

#[tokio::test]
async fn a_mutant_batch_answer_is_detected() {
    let h = harness().await;
    let req = json!({
        "op": "proxy-call-all",
        "calls": [
            {
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "echo",
                "params": r#"{"text":"normal"}"#
            }
        ]
    })
    .to_string();

    let wasm_res = h.wasm.run(&req).await.unwrap();
    let mutant_res = BatchMutant(&h.native).run(&req).await.unwrap();
    assert_ne!(wasm_res, mutant_res);
}
