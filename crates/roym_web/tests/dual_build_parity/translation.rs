//! `translation.targets` and `translation.translate` on both builds. The v1
//! node has no engine, so the answers are an empty list and `unavailable`.

use serde_json::{Value, json};

use super::{fixtures::both_rpc, helpers::*};

/// One method on both builds, which must answer alike.
async fn same_on_both(h: &Harness, method: &str, params: Value) -> Value {
    let (wasm, native) = both_rpc(h, method, params).await;
    assert_eq!(wasm, native, "{method}: the builds must answer the same");
    wasm
}

fn error_of(answer: &Value) -> (i64, &Value) {
    (answer["error"]["code"].as_i64().unwrap(), &answer["error"]["data"])
}

#[tokio::test]
async fn scenario_236_translation_methods_parity() {
    let h = harness().await;

    let targets = same_on_both(&h, "translation.targets", json!({})).await;
    assert_eq!(targets["result"], json!({ "targets": [] }));

    let answer = same_on_both(
        &h,
        "translation.translate",
        json!({ "texts": ["नल वाला"], "target_lang": "en" }),
    )
    .await;
    assert_eq!(error_of(&answer), (-32020, &json!({ "kind": "unavailable" })));

    let refused = [
        (json!({ "texts": ["a"], "target_lang": "EN" }), "target_lang"),
        (json!({ "texts": ["a"], "source_lang": "hi_IN", "target_lang": "en" }), "source_lang"),
        (json!({ "texts": vec!["a"; 9], "target_lang": "en" }), "texts"),
        (json!({ "texts": [], "target_lang": "en" }), "texts"),
        (json!({ "texts": ["a".repeat(8_193)], "target_lang": "en" }), "texts"),
        (json!({ "texts": vec!["a".repeat(8_192); 5], "target_lang": "en" }), "texts"),
        (json!({ "texts": ["a"], "target_lang": "en", "extra": 1 }), "extra"),
    ];
    for (params, field) in refused {
        let answer = same_on_both(&h, "translation.translate", params).await;
        assert_eq!(error_of(&answer), (-32602, &json!({ "field": field })));
    }

    let unknown = same_on_both(&h, "translation.nothing", json!({})).await;
    assert_eq!(unknown["error"]["code"], -32601);
}

#[tokio::test]
async fn translation_methods_need_a_signed_in_owner() {
    let h = harness().await;
    for method in ["translation.targets", "translation.translate"] {
        let req = json!({ "method": method, "params": {} }).to_string().into_bytes();
        let wasm = h.wasm_http.post("/rpc", req.clone(), None).await;
        let native = h.native_http.post("/rpc", req, None).await;
        assert_eq!(wasm.body, native.body, "{method}");
        let answer: Value = serde_json::from_slice(&wasm.body).unwrap();
        assert_eq!(answer["error"]["code"], -32010, "{method}");
    }
}
