use super::helpers::*;

#[tokio::test]
async fn both_builds_produce_identical_results() {
    let h = harness().await;
    let wasm_results = scenarios(&h.wasm).await;
    let native_results = scenarios(&h.native).await;
    assert!(!wasm_results.is_empty(), "the scenario table must not be empty");
    assert_eq!(wasm_results, native_results);
}

/// A passing `both_builds_produce_identical_results` is not evidence of
/// anything unless the comparison is known to detect a real divergence.
#[tokio::test]
async fn the_parity_comparison_detects_a_divergence() {
    let h = harness().await;
    let wasm_results = scenarios(&h.wasm).await;
    let mutant_results = scenarios(&Mutant(&h.native)).await;
    assert!(!wasm_results.is_empty());
    assert_ne!(wasm_results, mutant_results);
}

/// Named per-build positive assertions: the `assert_eq!` above tells you
/// *that* the builds differ, not which is wrong. A failure here names a
/// build.
#[tokio::test]
async fn wasm_build_store_and_read_round_trip() {
    let h = harness().await;
    let result = h.wasm.run(r#"{"op":"store-messages","count":5}"#).await.unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["ok"]["written"], 5);
    assert_eq!(v["ok"]["read"], 5);
}

#[tokio::test]
async fn native_build_store_and_read_round_trip() {
    let h = harness().await;
    let result = h.native.run(r#"{"op":"store-messages","count":5}"#).await.unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["ok"]["written"], 5);
    assert_eq!(v["ok"]["read"], 5);
}

#[tokio::test]
async fn wasm_build_stream_blob_round_trips_the_body() {
    let h = harness().await;
    let result = h
        .wasm
        .run(r#"{"op":"stream-blob","chunks":["ab","cd","ef"],"read_chunk":2}"#)
        .await
        .unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["ok"]["body"], "abcdef");
}

#[tokio::test]
async fn native_build_stream_blob_round_trips_the_body() {
    let h = harness().await;
    let result = h
        .native
        .run(r#"{"op":"stream-blob","chunks":["ab","cd","ef"],"read_chunk":2}"#)
        .await
        .unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["ok"]["body"], "abcdef");
}

#[tokio::test]
async fn wasm_build_admin_ddl_is_denied() {
    let h = harness().await;
    let result = h.wasm.run(r#"{"op":"admin-ddl","sql":"DROP TABLE messages"}"#).await.unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert!(v.get("err").is_some(), "expected admin-ddl to be denied, got {v}");
}

#[tokio::test]
async fn native_build_admin_ddl_is_denied() {
    let h = harness().await;
    let result = h.native.run(r#"{"op":"admin-ddl","sql":"DROP TABLE messages"}"#).await.unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert!(v.get("err").is_some(), "expected admin-ddl to be denied, got {v}");
}

/// `app::run`'s `serde_json::from_str` failure is the fixture's only WIT
/// `Err` path (as opposed to a WIT-level `Ok` carrying a JSON `"err"`
/// field, like the two tests above). Both builds surface it as an error at
/// this layer: `NativeFixture::dispatch`'s own comment notes its
/// `RpcError::InternalError` "mirrors the WASM `Err` arm's -32603" -- that
/// numeric code is a `syneroym-router` JSON-RPC-framing property
/// (`RpcError::code`, `crates/rpc/src/lib.rs`) neither driver here goes
/// through, so it is out of this suite's reach to assert directly.
#[tokio::test]
async fn malformed_request_json_errors_on_both_builds() {
    let h = harness().await;
    assert!(h.wasm.run(r#"{"op":"#).await.is_err());
    assert!(h.native.run(r#"{"op":"#).await.is_err());
}

/// `extract_request_param`'s `InvalidParams` arm needs a malformed *frame*
/// (no `request` field to find), which `Driver::run` can never produce --
/// it always builds a well-shaped `params: [<json>]`. Pinned here directly
/// against the native fixture, bypassing `Driver`. No WASM equivalent:
/// `WasmDriver` doesn't go through `NativeService::dispatch` either, so
/// there is nothing to compare against.
#[tokio::test]
async fn malformed_params_frame_is_invalid_params_not_internal_error() {
    use syneroym_rpc::{NativeService, RpcError};

    let h = harness().await;
    let inv = NativeInvocation {
        interface: "test-driver".to_string(),
        method: "run".to_string(),
        params: json!({}), // no "request" key
        caller: caller(),
    };
    let err = h.native.fixture.dispatch(inv).await.unwrap_err();
    assert!(matches!(err, RpcError::InvalidParams(_)), "got {err:?}");
}

#[tokio::test]
async fn both_builds_create_fence_round_trip() {
    let h = harness().await;
    let wasm_res = h.wasm.run(r#"{"op":"create-fence","id":"cf1"}"#).await.unwrap();
    let native_res = h.native.run(r#"{"op":"create-fence","id":"cf1"}"#).await.unwrap();
    let expected = serde_json::json!([null, "cf1"]);
    let wasm_val: Value = serde_json::from_str(&wasm_res).unwrap();
    let native_val: Value = serde_json::from_str(&native_res).unwrap();
    assert_eq!(wasm_val["ok"], expected);
    assert_eq!(native_val["ok"], expected);
}
