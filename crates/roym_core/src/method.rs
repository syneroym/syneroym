use std::future::Future;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::envelope::{Request, Response};

pub mod check;
#[cfg(test)]
mod tests;

pub type Reply<T> = Result<T, Response>;

/// Input of a method with no parameters.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NoParams {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingOutput {
    pub service: String,
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
    let value =
        if params.is_null() { Value::Object(serde_json::Map::new()) } else { params.clone() };
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
            input: std::any::type_name::<I>(),
            output: std::any::type_name::<O>(),
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
