//! Web entrypoint service application logic, target-independent.

use serde::Deserialize;
use serde_json::{Value, json};
use syneroym_app_host::{
    AppHost,
    types::{
        http::{CallerAuth, FrameKind, HttpRequest, HttpResponse},
        proxy::{CallTarget, ProxyError},
    },
};
use syneroym_roym_core::{
    envelope::{self, Request, Response},
    router, services,
};

pub const SCHEMA_VERSION: u32 = 1;

pub async fn status<H: AppHost>(_host: &H) -> Result<String, String> {
    Ok(json!({
        "service": services::WEB.name,
        "schema_version": SCHEMA_VERSION,
    })
    .to_string())
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: Option<String>,
    #[serde(default)]
    params: Value,
}

fn json_rpc_response(id: Option<Value>, result: Option<Value>, error: Option<Value>) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("jsonrpc".to_string(), Value::String("2.0".to_string()));
    obj.insert("id".to_string(), id.unwrap_or(Value::Null));
    if let Some(err) = error {
        obj.insert("error".to_string(), err);
    } else {
        obj.insert("result".to_string(), result.unwrap_or(Value::Null));
    }
    Value::Object(obj)
}

fn json_rpc_error(id: Option<Value>, code: i64, message: impl Into<String>) -> Value {
    json_rpc_response(
        id,
        None,
        Some(json!({
            "code": code,
            "message": message.into(),
        })),
    )
}

pub async fn handle_http<H: AppHost>(
    host: &H,
    request: HttpRequest,
) -> Result<HttpResponse, String> {
    match (request.method.as_str(), request.route.as_str()) {
        ("POST", "/rpc") => rpc(host, request).await,
        ("GET", "/health") => Ok(HttpResponse {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body: json!({ "status": "ok" }).to_string().into_bytes(),
        }),
        _ => Ok(HttpResponse {
            status: 404,
            headers: vec![("content-type".into(), "application/json".into())],
            body: json!({ "error": "not found" }).to_string().into_bytes(),
        }),
    }
}

pub async fn rpc<H: AppHost>(host: &H, request: HttpRequest) -> Result<HttpResponse, String> {
    let rpc_req: JsonRpcRequest = match serde_json::from_slice(&request.body) {
        Ok(r) => r,
        Err(e) => {
            let err_val = json_rpc_error(None, -32700, format!("Parse error: {e}"));
            return Ok(HttpResponse {
                status: 200,
                headers: vec![("content-type".into(), "application/json".into())],
                body: serde_json::to_vec(&err_val).unwrap_or_default(),
            });
        }
    };

    let id = rpc_req.id;
    let method = match rpc_req.method {
        Some(m) if !m.is_empty() => m,
        _ => {
            let err_val = json_rpc_error(id, -32600, "Invalid Request: missing method");
            return Ok(HttpResponse {
                status: 200,
                headers: vec![("content-type".into(), "application/json".into())],
                body: serde_json::to_vec(&err_val).unwrap_or_default(),
            });
        }
    };

    if method == "session.whoami" {
        let result = if let Some(caller) = &request.caller {
            let auth_str = match caller.auth {
                CallerAuth::Delegated => "delegated",
                CallerAuth::Ucan => "ucan",
                CallerAuth::SelfAsserted => "self-asserted",
            };
            json!({
                "did": caller.did,
                "auth": auth_str,
                "app_instance": caller.app_instance,
            })
        } else {
            json!({
                "did": Value::Null,
                "auth": "anonymous",
                "app_instance": Value::Null,
            })
        };
        let resp_val = json_rpc_response(id, Some(result), None);
        return Ok(HttpResponse {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body: serde_json::to_vec(&resp_val).unwrap_or_default(),
        });
    }

    let service = match router::route(&method) {
        Some(s) => s,
        None => {
            let safe_method = envelope::truncate_method(&method);
            let err_val = json_rpc_error(id, -32601, format!("Method '{safe_method}' not found"));
            return Ok(HttpResponse {
                status: 200,
                headers: vec![("content-type".into(), "application/json".into())],
                body: serde_json::to_vec(&err_val).unwrap_or_default(),
            });
        }
    };

    let payload = Request { method, params: rpc_req.params };
    let payload_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(e) => {
            let err_val = json_rpc_error(id, -32603, format!("Internal error: {e}"));
            return Ok(HttpResponse {
                status: 200,
                headers: vec![("content-type".into(), "application/json".into())],
                body: serde_json::to_vec(&err_val).unwrap_or_default(),
            });
        }
    };

    let call_res = host
        .call(
            CallTarget::Dependency(service.name.to_string()),
            service.interface.to_string(),
            "invoke".to_string(),
            json!([payload_str]).to_string(),
            None,
        )
        .await;

    let resp_val = match call_res {
        Ok(response_json) => match serde_json::from_str::<Response>(&response_json) {
            Ok(resp) => {
                if let Some(err) = resp.error {
                    json_rpc_response(id, None, Some(serde_json::to_value(err).unwrap_or_default()))
                } else {
                    json_rpc_response(id, resp.result, None)
                }
            }
            Err(_) => json_rpc_error(id, -32603, "Internal error: invalid callee response"),
        },
        Err(proxy_error) => {
            let (code, msg) = match proxy_error {
                ProxyError::DependencyNotBound { .. } => (-32001, "service not available"),
                ProxyError::TimedOut => (-32002, "service did not answer in time"),
                _ => (-32603, "Internal error"),
            };
            json_rpc_error(id, code, msg)
        }
    };

    Ok(HttpResponse {
        status: 200,
        headers: vec![("content-type".into(), "application/json".into())],
        body: serde_json::to_vec(&resp_val).unwrap_or_default(),
    })
}

pub async fn invoke<H: AppHost>(host: &H, req: Request) -> Response {
    if req.method == "session.whoami" {
        return Response::ok(json!({
            "did": Value::Null,
            "auth": "anonymous",
            "app_instance": Value::Null,
        }));
    }

    let service = match router::route(&req.method) {
        Some(s) => s,
        None => return Response::method_not_found(&req.method),
    };

    let payload_str = match serde_json::to_string(&req) {
        Ok(s) => s,
        Err(e) => return Response::internal_error(format!("Internal error: {e}")),
    };

    let call_res = host
        .call(
            CallTarget::Dependency(service.name.to_string()),
            service.interface.to_string(),
            "invoke".to_string(),
            json!([payload_str]).to_string(),
            None,
        )
        .await;

    match call_res {
        Ok(response_json) => match serde_json::from_str::<Response>(&response_json) {
            Ok(resp) => resp,
            Err(e) => Response::internal_error(format!("Invalid callee response: {e}")),
        },
        Err(proxy_error) => match proxy_error {
            ProxyError::DependencyNotBound { .. } => Response::err(-32001, "service not available"),
            ProxyError::TimedOut => Response::err(-32002, "service did not answer in time"),
            _ => Response::err(-32603, "Internal error"),
        },
    }
}

// No websocket product behavior exists yet, and nothing reads connection
// state, so these stay no-ops rather than carry state nothing consumes.
pub async fn on_ws_open<H: AppHost>(_host: &H, _conn: String) {}

pub async fn on_ws_message<H: AppHost>(
    _host: &H,
    _conn: String,
    _frame: Vec<u8>,
    _kind: FrameKind,
) {
}

pub async fn on_ws_close<H: AppHost>(_host: &H, _conn: String) {}
