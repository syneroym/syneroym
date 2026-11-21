use std::{any, future::Future};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use syneroym_app_host::{AppHost, types::proxy::CallTarget};

use crate::{
    clock,
    deal::sd::SignedTerms,
    disclosure::{
        COMMITMENT_SET, DisclosureIndex, FULFILMENT_SET, HiddenSet, PAYMENT_ACK_SET,
        PAYMENT_REQUEST_SET, STEP_SET, TERMS_SET, check_all_used, reveal_object,
    },
    envelope::{Request, Response, RpcError},
    fulfilment::{FulfilmentHidden, FulfilmentReceiptPayload},
    payment::{
        PaymentAckHidden, PaymentAcknowledgementPayload, PaymentRequestHidden,
        PaymentRequestPayload,
    },
    presentation::validate::validate_partial_terms_fields,
    services::Service,
    step::{StepPayload, check_data_size},
    verdict::RecordVerdict,
};

pub mod catalog;
pub mod check;
pub mod conversation;
pub mod directory;
pub mod profile;
pub mod scheduling;
#[cfg(test)]
mod tests;

pub type Reply<T> = Result<T, Response>;

/// Input of a method with no parameters.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NoParams {}

/// An empty input that ignores unknown fields, for wire-reachable methods
/// that take no parameters (such as `directory.info`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireNoParams {}

/// Publication rate limits input for catalog and directory services.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationLimitsInput {
    pub window_secs: u64,
    pub max_per_window: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingOutput {
    pub service: String,
}

/// Input for verify methods taking record envelope text.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyInput {
    pub envelope: String,
}

/// Runs one record verifier on the envelope text, at the current time.
pub fn verify_with<V>(input: VerifyInput, verify: fn(&str, u64) -> V) -> Reply<V> {
    Ok(verify(&input.envelope, clock::now_secs()))
}

/// Input for verify methods taking record envelope text and optional
/// disclosures.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DisclosedVerifyInput {
    pub envelope: String,
    #[serde(default)]
    pub disclosures: Vec<String>,
}

/// Output for verify methods with disclosed and hidden fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisclosedVerdict<P> {
    #[serde(flatten)]
    pub verdict: RecordVerdict<P>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown: Option<Value>,
    pub hidden: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disclosure_refused: Option<String>,
}

pub fn reveal_disclosed_terms(
    signed: &SignedTerms,
    disclosures: &[String],
) -> (Option<Value>, Vec<String>, Option<String>) {
    let all_hidden = all_terms_hidden_paths(signed);
    let max_disclosures = 5 + signed.commitments.len() * 10;
    let index = match DisclosureIndex::build(disclosures, max_disclosures) {
        Ok(idx) => idx,
        Err(e) => return (None, all_hidden, Some(e.code().to_string())),
    };
    let partial = match signed.reveal(&index) {
        Ok(p) => p,
        Err(e) => return (None, all_hidden, Some(e.code().to_string())),
    };
    if let Err(e) = check_all_used(&index, &partial.used) {
        return (None, all_hidden, Some(e.code().to_string()));
    }
    let mut not_checked = Vec::new();
    if let Err(e) = validate_partial_terms_fields(&partial, signed, &[], &mut not_checked) {
        return (None, all_hidden, Some(e.code().to_string()));
    }
    let hidden = partial.hidden.into_iter().map(|p| p.to_string()).collect();
    (Some(partial.value), hidden, None)
}

pub fn all_terms_hidden_paths(signed: &SignedTerms) -> Vec<String> {
    let mut hidden = Vec::with_capacity(
        TERMS_SET.hidden.len() + signed.commitments.len() * COMMITMENT_SET.hidden.len(),
    );
    for name in TERMS_SET.hidden {
        hidden.push(format!("terms.{name}"));
    }
    for c in &signed.commitments {
        let cid = &c.commitment_id;
        for name in COMMITMENT_SET.hidden {
            hidden.push(format!("terms.commitments[{cid}].{name}"));
        }
    }
    hidden
}

pub fn all_payment_request_hidden() -> Vec<String> {
    PAYMENT_REQUEST_SET.hidden.iter().map(|&s| s.to_string()).collect()
}

pub fn all_payment_ack_hidden() -> Vec<String> {
    PAYMENT_ACK_SET.hidden.iter().map(|&s| s.to_string()).collect()
}

pub fn all_fulfilment_hidden() -> Vec<String> {
    FULFILMENT_SET.hidden.iter().map(|&s| s.to_string()).collect()
}

pub fn reveal_settlement(
    set: &HiddenSet,
    sd: &[String],
    disclosures: &[String],
) -> Result<(Value, Vec<String>), String> {
    let index =
        DisclosureIndex::build(disclosures, set.hidden.len()).map_err(|e| e.code().to_string())?;
    let mut obj = Map::new();
    obj.insert("_sd".to_string(), Value::Array(sd.iter().cloned().map(Value::String).collect()));
    let revealed = reveal_object(&obj, set, &index).map_err(|e| e.code().to_string())?;
    check_all_used(&index, &revealed.used).map_err(|e| e.code().to_string())?;
    let hidden = revealed.hidden.into_iter().map(|s| s.to_string()).collect();
    Ok((Value::Object(revealed.fields), hidden))
}

pub fn reveal_disclosed_payment_request(
    p: &PaymentRequestPayload,
    disclosures: &[String],
) -> (Option<Value>, Vec<String>, Option<String>) {
    let all_hidden = all_payment_request_hidden();
    let (val, hidden) = match reveal_settlement(&PAYMENT_REQUEST_SET, &p.sd, disclosures) {
        Ok(r) => r,
        Err(code) => return (None, all_hidden, Some(code)),
    };
    let hidden_struct: Result<PaymentRequestHidden, _> = serde_json::from_value(val.clone());
    match hidden_struct {
        Ok(h) => {
            if h.validate().is_err() {
                return (None, all_hidden, Some("field-invalid".to_string()));
            }
        }
        Err(_) => return (None, all_hidden, Some("field-invalid".to_string())),
    }
    (Some(val), hidden, None)
}

pub fn reveal_disclosed_payment_ack(
    p: &PaymentAcknowledgementPayload,
    disclosures: &[String],
) -> (Option<Value>, Vec<String>, Option<String>) {
    let all_hidden = all_payment_ack_hidden();
    let (val, hidden) = match reveal_settlement(&PAYMENT_ACK_SET, &p.sd, disclosures) {
        Ok(r) => r,
        Err(code) => return (None, all_hidden, Some(code)),
    };
    let hidden_struct: Result<PaymentAckHidden, _> = serde_json::from_value(val.clone());
    match hidden_struct {
        Ok(h) => {
            if h.validate().is_err() {
                return (None, all_hidden, Some("field-invalid".to_string()));
            }
        }
        Err(_) => return (None, all_hidden, Some("field-invalid".to_string())),
    }
    (Some(val), hidden, None)
}

pub fn reveal_disclosed_fulfilment(
    p: &FulfilmentReceiptPayload,
    disclosures: &[String],
) -> (Option<Value>, Vec<String>, Option<String>) {
    let all_hidden = all_fulfilment_hidden();
    let (val, hidden) = match reveal_settlement(&FULFILMENT_SET, &p.sd, disclosures) {
        Ok(r) => r,
        Err(code) => return (None, all_hidden, Some(code)),
    };
    let hidden_struct: Result<FulfilmentHidden, _> = serde_json::from_value(val.clone());
    match hidden_struct {
        Ok(h) => {
            if h.validate().is_err() {
                return (None, all_hidden, Some("field-invalid".to_string()));
            }
        }
        Err(_) => return (None, all_hidden, Some("field-invalid".to_string())),
    }
    (Some(val), hidden, None)
}

pub fn reveal_disclosed_step(
    p: &StepPayload,
    disclosures: &[String],
) -> (Option<Value>, Vec<String>, Option<String>) {
    let all_hidden: Vec<String> = STEP_SET.hidden.iter().map(|&s| s.to_string()).collect();
    let (val, hidden) = match reveal_settlement(&STEP_SET, &p.sd, disclosures) {
        Ok(r) => r,
        Err(code) => return (None, all_hidden, Some(code)),
    };
    // `data` is only there when its disclosure was given.
    if val.get("data").is_some_and(|data| check_data_size(data).is_err()) {
        return (None, all_hidden, Some("field-invalid".to_string()));
    }
    (Some(val), hidden, None)
}

pub fn verify_disclosed<P, F>(
    verdict: RecordVerdict<P>,
    disclosures: &[String],
    reveal_fn: F,
) -> DisclosedVerdict<P>
where
    F: FnOnce(&P, &[String]) -> (Option<Value>, Vec<String>, Option<String>),
{
    let (shown, hidden, disclosure_refused) = if verdict.verified {
        if let Some(ref p) = verdict.payload {
            reveal_fn(p, disclosures)
        } else {
            (None, Vec::new(), None)
        }
    } else {
        (None, Vec::new(), None)
    };
    DisclosedVerdict { verdict, shown, hidden, disclosure_refused }
}

/// Output for history methods returning an envelope chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeHistoryOutput {
    pub history: Vec<String>,
}

/// A bad input, before it is turned into a `-32602` response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamError {
    pub field: Option<String>,
    pub message: String,
}

impl ParamError {
    #[must_use]
    pub fn into_response(self) -> Response {
        let resp = Response::invalid_params(self.message);
        if let Some(field) = self.field {
            resp.with_data(serde_json::json!({ "field": field }))
        } else {
            resp
        }
    }
}

fn quoted_name(text: &str) -> Option<String> {
    let start = text.find('`')? + 1;
    let end = text[start..].find('`')? + start;
    Some(text[start..end].to_string())
}

fn has_named_segment(path: &str) -> bool {
    path.split('.').any(|seg| {
        let s = seg.trim_start_matches('[').trim_end_matches(']');
        !s.is_empty() && !s.chars().all(|c| c.is_ascii_digit()) && s != "?"
    })
}

fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        s.to_string()
    } else {
        let mut end = max_bytes;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        s[..end].to_string()
    }
}

pub fn parse_value<T: DeserializeOwned>(params: &Value) -> Result<T, ParamError> {
    if !params.is_object() && !params.is_null() {
        return Err(ParamError {
            field: None,
            message: "invalid params: expected an object".to_string(),
        });
    }
    let empty = Value::Object(Map::new());
    let value = if params.is_null() { &empty } else { params };
    match serde_path_to_error::deserialize(value) {
        Ok(t) => Ok(t),
        Err(e) => {
            let path = e.path().to_string();
            let text = e.into_inner().to_string();
            let name = quoted_name(&text);
            let is_missing_or_dup =
                text.starts_with("missing field") || text.starts_with("duplicate field");
            let field = if is_missing_or_dup {
                if path == "." { name } else { name.map(|n| format!("{path}.{n}")) }
            } else if path != "." && has_named_segment(&path) {
                Some(path.clone())
            } else {
                None
            };
            let field = field.map(|f| truncate_utf8(&f, 256));
            let msg = if path == "." || path.starts_with('?') {
                format!("invalid params: {text}")
            } else {
                format!("invalid params: {path}: {text}")
            };
            let message = truncate_utf8(&msg, 256);
            Err(ParamError { field, message })
        }
    }
}

pub async fn typed<'a, H, I, O, F, Fut>(host: &'a H, req: &Request, handler: F) -> Response
where
    I: DeserializeOwned,
    O: Serialize,
    F: FnOnce(&'a H, I) -> Fut,
    Fut: Future<Output = Reply<O>>,
{
    let input = match parse_value::<I>(&req.params) {
        Ok(inp) => inp,
        Err(err) => return err.into_response(),
    };
    let output = match handler(host, input).await {
        Ok(out) => out,
        Err(resp) => return resp,
    };
    match serde_json::to_value(output) {
        Ok(v) => Response::ok(v),
        Err(e) => Response::internal_error(e.to_string()),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SiblingError {
    #[error("{0}")]
    Transport(String),
    #[error("{} (code {})", .0.message, .0.code)]
    Refused(RpcError),
    #[error("{0}")]
    BadReply(String),
}

pub async fn call_sibling<H: AppHost, I: Serialize, O: DeserializeOwned>(
    host: &H,
    service: Service,
    method: &str,
    input: &I,
) -> Result<O, SiblingError> {
    let req = serde_json::to_string(&serde_json::json!({
        "method": method,
        "params": input,
    }))
    .map_err(|e| SiblingError::Transport(e.to_string()))?;

    let raw = host
        .call(
            CallTarget::Dependency(service.name.to_string()),
            service.interface.to_string(),
            "invoke".to_string(),
            serde_json::to_string(&serde_json::json!([req]))
                .map_err(|e| SiblingError::Transport(e.to_string()))?,
            None,
        )
        .await
        .map_err(|e| SiblingError::Transport(format!("{method}: {e:?}")))?;

    read_reply(&raw)
}

/// Reads one `invoke` reply: an `error` is `Refused`, a `result` is read as
/// `O`.
pub fn read_reply<O: DeserializeOwned>(raw: &str) -> Result<O, SiblingError> {
    let raw_val: Value = serde_json::from_str(raw)
        .map_err(|e| SiblingError::BadReply(format!("invalid JSON-RPC response: {e}")))?;

    if let Some(err_val) = raw_val.get("error")
        && !err_val.is_null()
    {
        let err: RpcError = serde_json::from_value(err_val.clone())
            .map_err(|e| SiblingError::BadReply(format!("invalid RPC error payload: {e}")))?;
        return Err(SiblingError::Refused(err));
    }

    let Some(result) = raw_val.get("result") else {
        return Err(SiblingError::BadReply("missing result in response".to_string()));
    };

    serde_json::from_value(result.clone())
        .map_err(|e| SiblingError::BadReply(format!("invalid output: {e}")))
}

#[derive(Debug, Clone)]
pub struct MethodInfo {
    pub name: String,
    pub input: &'static str,
    pub output: &'static str,
    pub probe: fn(&Value) -> Result<(), ParamError>,
}

impl MethodInfo {
    pub fn of<I: DeserializeOwned, O: Serialize>(name: &str) -> Self {
        Self {
            name: name.to_string(),
            input: any::type_name::<I>(),
            output: any::type_name::<O>(),
            probe: |v| parse_value::<I>(v).map(|_| ()),
        }
    }
}

#[macro_export]
macro_rules! service_methods {
    ($( $name:literal : $handler:path [ $input:ty => $output:ty ] ),+ $(,)?) => {
        pub fn method_rows() -> Vec<$crate::method::MethodInfo> {
            vec![ $( $crate::method::MethodInfo::of::<$input, $output>($name) ),+ ]
        }
        pub(crate) async fn dispatch<H: ::syneroym_app_host::AppHost>(
            host: &H, req: &$crate::envelope::Request,
        ) -> Option<$crate::envelope::Response> {
            match req.method.as_str() {
                $( $name => Some($crate::method::typed::<H, $input, $output, _, _>(
                    host, req, $handler).await), )+
                _ => None,
            }
        }
    };
}
