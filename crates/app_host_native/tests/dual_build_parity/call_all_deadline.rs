use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use syneroym_core::config::{AppSandboxRole, SubstrateConfig};
use tokio::time;

use super::helpers::*;

async fn deadline_harness() -> Harness {
    let mut config = SubstrateConfig::default();
    config.roles.app_sandbox = Some(AppSandboxRole {
        default_max_instructions: Some(500_000_000_000),
        ..AppSandboxRole::default()
    });
    harness_with_config(Some(config)).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plain_call_that_waits_past_the_budget_stops_the_guest() {
    let h = harness().await;
    let req = json!({
        "op": "proxy-call-then-work",
        "call": {
            "kind": "service",
            "name": "sibling",
            "interface": "scripted",
            "method": "sleep",
            "params": r#"{"ms":6000}"#,
            "timeout_ms": 10000
        }
    })
    .to_string();

    let start = Instant::now();
    let res = h.wasm.run(&req).await;
    let elapsed = start.elapsed();

    let err = res.expect_err("plain call waiting past budget should stop the guest");
    assert!(
        err.contains("interrupt") || err.contains("deadline"),
        "expected interrupt trap, got {err}"
    );
    assert!(elapsed >= Duration::from_secs(6), "elapsed {elapsed:?} must be at least 6 s");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_batch_that_waits_past_the_budget_does_not_stop_the_guest() {
    let h = harness().await;
    let req = json!({
        "op": "proxy-call-all",
        "calls": [
            {
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "sleep",
                "params": r#"{"ms":6000}"#,
                "timeout_ms": 10000
            }
        ]
    })
    .to_string();

    let start = Instant::now();
    let res = h.wasm.run(&req).await.unwrap();
    let elapsed = start.elapsed();

    assert!(elapsed >= Duration::from_secs(6), "elapsed {elapsed:?} must be at least 6 s");
    let v: Value = serde_json::from_str(&res).unwrap();
    assert_eq!(v["ok"]["outcomes"][0]["ok"], "slept");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn credit_buys_back_the_wait_and_no_more() {
    let h = deadline_harness().await;
    let req = json!({
        "op": "proxy-call-all-then-spin",
        "calls": [
            {
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "sleep",
                "params": r#"{"ms":6000}"#,
                "timeout_ms": 10000
            }
        ]
    })
    .to_string();

    let start = Instant::now();
    let res = h.wasm.run(&req).await;
    let elapsed = start.elapsed();

    let err = res.expect_err("spinning guest after batch should be stopped");
    assert!(
        err.contains("interrupt") || err.contains("deadline"),
        "expected interrupt trap, got {err}"
    );

    let min_time = Duration::from_millis(10_900); // 6s wait + 5s budget - 100ms tick
    let max_time = Duration::from_millis(12_500); // 6s wait + 5s budget + 1.5s margin
    assert!(
        elapsed >= min_time && elapsed <= max_time,
        "elapsed {elapsed:?} outside expected range [{min_time:?}, {max_time:?}]"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_guest_that_spins_is_stopped_on_time_when_it_never_used_call_all() {
    let h = deadline_harness().await;
    let req = json!({
        "op": "proxy-call-all-then-spin",
        "calls": []
    })
    .to_string();

    let start = Instant::now();
    let res = h.wasm.run(&req).await;
    let elapsed = start.elapsed();

    let err = res.expect_err("spinning guest with no batch calls should be stopped at budget");
    assert!(
        err.contains("interrupt") || err.contains("deadline"),
        "expected interrupt trap, got {err}"
    );

    let min_time = Duration::from_millis(4_900); // 5s budget - 100ms tick
    let max_time = Duration::from_millis(6_500); // 5s budget + 1.5s margin
    assert!(
        elapsed >= min_time && elapsed <= max_time,
        "elapsed {elapsed:?} outside expected range [{min_time:?}, {max_time:?}]"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_batches_in_one_wasm_guest_call_accumulate_credit() {
    let h = harness().await;
    let req = json!({
        "op": "proxy-call-all-seq",
        "batches": [
            [
                {
                    "kind": "service",
                    "name": "sibling",
                    "interface": "scripted",
                    "method": "sleep",
                    "params": r#"{"ms":7000}"#,
                    "timeout_ms": 10000
                }
            ],
            [
                {
                    "kind": "service",
                    "name": "sibling",
                    "interface": "scripted",
                    "method": "sleep",
                    "params": r#"{"ms":7500}"#,
                    "timeout_ms": 10000
                }
            ],
            [
                {
                    "kind": "service",
                    "name": "sibling",
                    "interface": "scripted",
                    "method": "echo",
                    "params": r#"{"msg":"third"}"#
                }
            ]
        ]
    })
    .to_string();

    let start = Instant::now();
    let res = h.wasm.run(&req).await.unwrap();
    let elapsed = start.elapsed();

    assert!(
        elapsed >= Duration::from_millis(14_500),
        "elapsed {elapsed:?} must be at least 14.5 s"
    );

    let v: Value = serde_json::from_str(&res).unwrap();
    let batches = v["ok"]["batches"].as_array().unwrap();
    assert_eq!(batches.len(), 3);
    assert_eq!(batches[0]["outcomes"][0]["ok"], "slept");
    assert_eq!(batches[1]["outcomes"][0]["ok"], "slept");
    let third_refused = batches[2]["refused"].as_str().unwrap();
    assert!(
        third_refused.contains("no wait time left"),
        "third batch should be refused for allowance: {third_refused}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_batches_do_not_fill_the_instance_pool() {
    let h = Arc::new(harness().await);

    let batch_req = json!({
        "op": "proxy-call-all",
        "calls": [
            {
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "sleep",
                "params": r#"{"ms":3000}"#,
                "timeout_ms": 5000
            }
        ]
    })
    .to_string();

    // Start 4 concurrent batches (the default waiter permit limit).
    let mut handles = Vec::new();
    for _ in 0..4 {
        let h_clone = Arc::clone(&h);
        let req_clone = batch_req.clone();
        handles.push(tokio::spawn(async move { h_clone.wasm.run(&req_clone).await.unwrap() }));
    }

    // Wait a short time for all 4 batches to acquire waiter permits and begin
    // waiting.
    time::sleep(Duration::from_millis(300)).await;

    // While 4 batches are waiting, a plain run (read-config) answers Ok within 1 s.
    let echo_req = json!({
        "op": "read-config",
        "key": "greeting"
    })
    .to_string();

    let echo_start = Instant::now();
    let echo_res = h.wasm.run(&echo_req).await.unwrap();
    let echo_elapsed = echo_start.elapsed();
    assert!(echo_elapsed < Duration::from_secs(1), "echo call took too long: {echo_elapsed:?}");
    let echo_val: Value = serde_json::from_str(&echo_res).unwrap();
    assert_eq!(echo_val["ok"]["value"], "hello");

    // A 5th batch is refused with call-all busy because all 4 permits are taken.
    let fifth_res = h.wasm.run(&batch_req).await.unwrap();
    let fifth_val: Value = serde_json::from_str(&fifth_res).unwrap();
    let fifth_refused = fifth_val["ok"]["refused"].as_str().unwrap();
    assert!(
        fifth_refused.contains("call-all busy: too many guests are waiting"),
        "expected busy refusal, got {fifth_refused}"
    );

    // Wait for the initial 4 batches to finish.
    for handle in handles {
        let out = handle.await.unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["ok"]["outcomes"][0]["ok"], "slept");
    }

    // After the first 4 end, permits are released; a 6th batch runs and succeeds.
    let sixth_req = json!({
        "op": "proxy-call-all",
        "calls": [
            {
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "echo",
                "params": r#"{"msg":"sixth"}"#
            }
        ]
    })
    .to_string();
    let sixth_res = h.wasm.run(&sixth_req).await.unwrap();
    let sixth_val: Value = serde_json::from_str(&sixth_res).unwrap();
    assert_eq!(sixth_val["ok"]["outcomes"][0]["ok"], r#"{"msg":"sixth"}"#);
}

#[tokio::test]
async fn the_native_build_has_no_deadline_for_a_slow_batch() {
    let h = harness().await;

    // A 6 s batch returns Ok natively.
    let batch_req = json!({
        "op": "proxy-call-all",
        "calls": [
            {
                "kind": "service",
                "name": "sibling",
                "interface": "scripted",
                "method": "sleep",
                "params": r#"{"ms":6000}"#,
                "timeout_ms": 10000
            }
        ]
    })
    .to_string();

    let native_batch_res = h.native.run(&batch_req).await.unwrap();
    let v: Value = serde_json::from_str(&native_batch_res).unwrap();
    assert_eq!(v["ok"]["outcomes"][0]["ok"], "slept");

    // The plain call ProxyCallThenWork also returns Ok natively (no epoch
    // interrupt).
    let plain_req = json!({
        "op": "proxy-call-then-work",
        "call": {
            "kind": "service",
            "name": "sibling",
            "interface": "scripted",
            "method": "sleep",
            "params": r#"{"ms":6000}"#,
            "timeout_ms": 10000
        }
    })
    .to_string();

    let native_plain_res = h.native.run(&plain_req).await.unwrap();
    let v_plain: Value = serde_json::from_str(&native_plain_res).unwrap();
    assert_eq!(v_plain["ok"]["result"], "slept");
}
