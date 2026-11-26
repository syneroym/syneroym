use serde_json::json;
use syneroym_roym_core::envelope::Response;

#[allow(dead_code)]
pub(crate) fn refusal(code: &str) -> Response {
    Response::invalid_params(code).with_data(json!({ "code": code }))
}
