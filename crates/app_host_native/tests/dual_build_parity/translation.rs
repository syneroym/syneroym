use super::helpers::*;

async fn drive<D: Driver>(driver: &D, request: &str) -> Value {
    let result = driver.run(request).await.unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    v["ok"].clone()
}

/// The three calls of `syneroym:translation` answer the same on both
/// builds: no target, and `unavailable` for detection and translation.
#[tokio::test]
async fn translation_answers_unavailable_on_both_builds() {
    let h = harness().await;
    let requests = [
        r#"{"op":"translation-targets"}"#,
        r#"{"op":"detect-language","text":"नल वाला"}"#,
        r#"{"op":"translate","text":"नल वाला","source_lang":"hi","target_lang":"en"}"#,
        r#"{"op":"translate","text":"plumber","source_lang":null,"target_lang":"hi"}"#,
    ];
    let expected = [
        json!({ "targets": [] }),
        json!({ "err": "Unavailable" }),
        json!({ "err": "Unavailable" }),
        json!({ "err": "Unavailable" }),
    ];
    for (request, expected) in requests.into_iter().zip(expected) {
        let wasm = drive(&h.wasm, request).await;
        let native = drive(&h.native, request).await;
        assert_eq!(wasm, native, "{request}: the builds must answer the same");
        assert_eq!(wasm, expected, "{request}");
    }
}
