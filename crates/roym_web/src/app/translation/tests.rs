//! The wire shape of the translation refusals, and the loop that translates
//! many texts. A scripted engine stands in for the host: the v1 node has no
//! engine, so only `unavailable` can happen there.

use std::sync::Mutex;

use serde_json::json;
use syneroym_app_host::{
    AppTranslation,
    types::translation::{Detected, EngineInfo, Translated, TranslationError},
};

use super::*;

/// Answers by the text it is given: the words `pair`, `long` and `broken`
/// are refused, and any other text comes back with `!` added.
#[derive(Default)]
struct ScriptedEngine {
    seen: Mutex<Vec<String>>,
}

impl AppTranslation for ScriptedEngine {
    async fn supported_targets(&self) -> Vec<String> {
        vec!["en".to_string(), "hi".to_string()]
    }

    async fn detect_language(&self, _text: String) -> Result<Vec<Detected>, TranslationError> {
        Err(TranslationError::Unavailable)
    }

    async fn translate(
        &self,
        text: String,
        source_lang: Option<String>,
        target_lang: String,
    ) -> Result<Translated, TranslationError> {
        self.seen.lock().unwrap().push(text.clone());
        match text.as_str() {
            "pair" => Err(TranslationError::UnsupportedPair((
                source_lang.unwrap_or_default(),
                target_lang,
            ))),
            "long" => Err(TranslationError::TooLong(100)),
            "broken" => Err(TranslationError::EngineFailed("engine said no".to_string())),
            other => Ok(Translated {
                text: format!("{other}!"),
                source_lang: source_lang.unwrap_or_else(|| "hi".to_string()),
                engine: EngineInfo { name: "scripted".to_string(), remote: false },
            }),
        }
    }
}

fn error_of(resp: &Response) -> (i64, &str, &serde_json::Value) {
    let err = resp.error.as_ref().expect("an error answer");
    (err.code, err.message.as_str(), err.data.as_ref().expect("error data"))
}

#[test]
fn each_refusal_has_its_documented_code_message_and_data() {
    let cases = [
        (
            TranslationError::Unavailable,
            "translation unavailable",
            json!({ "kind": "unavailable" }),
        ),
        (
            TranslationError::UnsupportedPair(("hi".to_string(), "ta".to_string())),
            "translation pair unsupported",
            json!({ "kind": "unsupported-pair", "source": "hi", "target": "ta" }),
        ),
        (
            TranslationError::TooLong(4096),
            "text too long for the engine",
            json!({ "kind": "too-long", "limit": 4096 }),
        ),
        // The engine's own text is in the message only, never in `data`.
        (
            TranslationError::EngineFailed("boom".to_string()),
            "translation engine failed: boom",
            json!({ "kind": "engine-failed" }),
        ),
    ];
    for (refusal, message, data) in cases {
        let resp = translation_error(refusal);
        assert_eq!(error_of(&resp), (TRANSLATION_ERROR, message, &data));
    }
}

fn input(texts: &[&str]) -> serde_json::Value {
    json!({ "texts": texts, "source_lang": "hi", "target_lang": "en" })
}

#[tokio::test]
async fn texts_are_translated_one_call_each_in_order() {
    let engine = ScriptedEngine::default();
    let resp = translate(&engine, &input(&["a", "b", "c"])).await;

    let items = resp.result.expect("a result")["items"].clone();
    let texts: Vec<_> = items.as_array().unwrap().iter().map(|i| i["text"].clone()).collect();
    assert_eq!(texts, [json!("a!"), json!("b!"), json!("c!")]);
    assert_eq!(items[0]["engine"], json!({ "name": "scripted", "remote": false }));
    assert_eq!(*engine.seen.lock().unwrap(), ["a", "b", "c"]);
}

#[tokio::test]
async fn the_first_refusal_stops_the_loop_and_is_the_answer() {
    let engine = ScriptedEngine::default();
    let resp = translate(&engine, &input(&["a", "long", "c"])).await;

    assert_eq!(
        error_of(&resp),
        (
            TRANSLATION_ERROR,
            "text too long for the engine",
            &json!({ "kind": "too-long", "limit": 100 })
        )
    );
    // The third text was never sent.
    assert_eq!(*engine.seen.lock().unwrap(), ["a", "long"]);
}

#[tokio::test]
async fn an_unsupported_pair_names_the_source_and_the_target_in_that_order() {
    let engine = ScriptedEngine::default();
    let resp = translate(&engine, &input(&["pair"])).await;

    assert_eq!(
        error_of(&resp).2,
        &json!({ "kind": "unsupported-pair", "source": "hi", "target": "en" })
    );
}

#[tokio::test]
async fn an_engine_failure_keeps_its_text_out_of_data() {
    let engine = ScriptedEngine::default();
    let resp = translate(&engine, &input(&["broken"])).await;

    let (code, message, data) = error_of(&resp);
    assert_eq!(code, TRANSLATION_ERROR);
    assert!(message.contains("engine said no"));
    assert_eq!(data, &json!({ "kind": "engine-failed" }));
}

#[tokio::test]
async fn targets_come_from_the_host() {
    let resp = targets(&ScriptedEngine::default(), &json!({})).await;
    assert_eq!(resp.result.unwrap(), json!({ "targets": ["en", "hi"] }));
}
