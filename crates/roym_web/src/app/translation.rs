//! `translation.targets` and `translation.translate`: the two methods `web`
//! answers itself so the Hub can reach the node's translation host. Both
//! need the owner (see `router::WEB_ROUTES`) and run only on the HTTP path.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{AppHost, AppTranslation, types::translation::TranslationError};
use syneroym_roym_core::{
    envelope::Response,
    lang::LangTag,
    method::{self, NoParams, ParamError},
};

use super::json_rpc_response;

pub const TARGETS: &str = "translation.targets";
pub const TRANSLATE: &str = "translation.translate";

/// The error code of every refusal by the translation host.
pub const TRANSLATION_ERROR: i64 = -32020;

/// A call translates at most this many texts.
pub const MAX_TRANSLATE_TEXTS: usize = 8;
/// One text is at most this many bytes.
pub const MAX_TRANSLATE_TEXT_BYTES: usize = 8_192;
/// All texts of one call together are at most this many bytes. This covers
/// the largest Hub call: the four free-text terms of a quote.
pub const MAX_TRANSLATE_TOTAL_BYTES: usize = 32_768;

#[derive(Debug, Serialize, Deserialize)]
pub struct TargetsOutput {
    pub targets: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TranslateInput {
    pub texts: Vec<String>,
    #[serde(default)]
    pub source_lang: Option<String>,
    pub target_lang: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EngineView {
    pub name: String,
    pub remote: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TranslatedItem {
    pub text: String,
    pub source_lang: String,
    pub engine: EngineView,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TranslateOutput {
    pub items: Vec<TranslatedItem>,
}

/// Answers one `translation.*` method as a JSON-RPC response value.
pub async fn dispatch<H: AppHost>(
    host: &H,
    method: &str,
    params: Value,
    id: Option<Value>,
) -> Value {
    let resp = match method {
        TARGETS => targets(host, &params).await,
        TRANSLATE => translate(host, &params).await,
        other => Response::method_not_found(other),
    };
    match resp.error {
        Some(err) => json_rpc_response(id, None, serde_json::to_value(err).ok()),
        None => json_rpc_response(id, resp.result, None),
    }
}

async fn targets<H: AppHost>(host: &H, params: &Value) -> Response {
    if let Err(err) = method::parse_value::<NoParams>(params) {
        return err.into_response();
    }
    let targets = AppTranslation::supported_targets(host).await;
    Response::ok(json!(TargetsOutput { targets }))
}

async fn translate<H: AppHost>(host: &H, params: &Value) -> Response {
    let input = match method::parse_value::<TranslateInput>(params) {
        Ok(input) => input,
        Err(err) => return err.into_response(),
    };
    if let Err(err) = check_input(&input) {
        return err.into_response();
    }
    let mut items = Vec::with_capacity(input.texts.len());
    for text in input.texts {
        let done = AppTranslation::translate(
            host,
            text,
            input.source_lang.clone(),
            input.target_lang.clone(),
        )
        .await;
        match done {
            Ok(t) => items.push(TranslatedItem {
                text: t.text,
                source_lang: t.source_lang,
                engine: EngineView { name: t.engine.name, remote: t.engine.remote },
            }),
            Err(e) => return translation_error(e),
        }
    }
    Response::ok(json!(TranslateOutput { items }))
}

fn bad_input(field: &str, message: impl Into<String>) -> ParamError {
    ParamError { field: Some(field.to_string()), message: message.into() }
}

fn check_input(input: &TranslateInput) -> Result<(), ParamError> {
    if input.texts.is_empty() || input.texts.len() > MAX_TRANSLATE_TEXTS {
        return Err(bad_input(
            "texts",
            format!("invalid params: `texts` needs 1 to {MAX_TRANSLATE_TEXTS} entries"),
        ));
    }
    if input.texts.iter().any(|t| t.len() > MAX_TRANSLATE_TEXT_BYTES) {
        return Err(bad_input(
            "texts",
            format!("invalid params: a text is longer than {MAX_TRANSLATE_TEXT_BYTES} bytes"),
        ));
    }
    if input.texts.iter().map(String::len).sum::<usize>() > MAX_TRANSLATE_TOTAL_BYTES {
        return Err(bad_input(
            "texts",
            format!("invalid params: the texts are longer than {MAX_TRANSLATE_TOTAL_BYTES} bytes"),
        ));
    }
    check_tag("target_lang", &input.target_lang)?;
    if let Some(source) = input.source_lang.as_deref() {
        check_tag("source_lang", source)?;
    }
    Ok(())
}

fn check_tag(field: &str, tag: &str) -> Result<(), ParamError> {
    LangTag::parse_canonical(tag)
        .map(|_| ())
        .map_err(|e| bad_input(field, format!("invalid params: `{field}`: {e}")))
}

/// The engine's own text goes into the message, never into `data`.
fn translation_error(e: TranslationError) -> Response {
    let (message, data) = match e {
        TranslationError::Unavailable => {
            ("translation unavailable".to_string(), json!({ "kind": "unavailable" }))
        }
        TranslationError::UnsupportedPair((source, target)) => (
            "translation pair unsupported".to_string(),
            json!({ "kind": "unsupported-pair", "source": source, "target": target }),
        ),
        TranslationError::TooLong(limit) => (
            "text too long for the engine".to_string(),
            json!({ "kind": "too-long", "limit": limit }),
        ),
        TranslationError::EngineFailed(text) => {
            (format!("translation engine failed: {text}"), json!({ "kind": "engine-failed" }))
        }
    };
    Response::err(TRANSLATION_ERROR, message).with_data(data)
}
