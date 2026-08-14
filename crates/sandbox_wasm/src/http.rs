//! Dynamic `Val` marshalling for the guest HTTP handler (M06A A2,
//! `syneroym:http/incoming-handler@0.1.0`). Reuses `stream.rs`'s
//! `bytes_to_val_list`/`val_list_to_bytes` -- every guest call in this crate
//! is already dynamic (`AppSandboxEngine::get_wasm_func` +
//! `Func::call_async`), so a first typed (`bindgen!`) export path would be
//! new machinery for one interface with a request/response body capped at
//! 1 MiB.

use syneroym_core::guest_http::{
    GuestCallerAuth, GuestCallerIdentity, GuestHttpRequest, GuestHttpResponse,
};
use wasmtime::component::Val;

use crate::{
    engine::GuestHttpFailure,
    stream::{bytes_to_val_list, val_list_to_bytes},
};

/// WIT-package-qualified name of the guest HTTP handler interface (M06A
/// A2) -- the short name alone does not resolve, same as
/// `STREAM_TYPES_INTERFACE` and `AUTHORIZER_INTERFACE`.
pub(crate) const HTTP_HANDLER_INTERFACE: &str = "syneroym:http/incoming-handler@0.1.0";

fn string_pairs_to_val(pairs: &[(String, String)]) -> Val {
    Val::List(
        pairs
            .iter()
            .map(|(k, v)| Val::Tuple(vec![Val::String(k.clone()), Val::String(v.clone())]))
            .collect(),
    )
}

fn caller_auth_case(auth: GuestCallerAuth) -> &'static str {
    match auth {
        GuestCallerAuth::Delegated => "delegated",
        GuestCallerAuth::Ucan => "ucan",
        GuestCallerAuth::SelfAsserted => "self-asserted",
    }
}

fn caller_identity_to_val(caller: &GuestCallerIdentity) -> Val {
    Val::Record(vec![
        ("did".to_string(), Val::String(caller.did.clone())),
        ("auth".to_string(), Val::Enum(caller_auth_case(caller.auth).to_string())),
        (
            "app-instance".to_string(),
            Val::Option(caller.app_instance.clone().map(|s| Box::new(Val::String(s)))),
        ),
    ])
}

/// Builds the `Val::Record` argument for `handle-request`. Field order is
/// the WIT declaration order and must stay in sync with
/// `crates/wit_interfaces/wit/http/http.wit`.
pub(crate) fn request_to_val(request: &GuestHttpRequest) -> Val {
    Val::Record(vec![
        ("method".to_string(), Val::String(request.method.clone())),
        ("path".to_string(), Val::String(request.path.clone())),
        ("query".to_string(), Val::String(request.query.clone())),
        ("route".to_string(), Val::String(request.route.clone())),
        ("path-params".to_string(), string_pairs_to_val(&request.path_params)),
        ("headers".to_string(), string_pairs_to_val(&request.headers)),
        ("body".to_string(), bytes_to_val_list(request.body.clone())),
        (
            "caller".to_string(),
            Val::Option(request.caller.as_ref().map(|c| Box::new(caller_identity_to_val(c)))),
        ),
    ])
}

fn decode_string_pairs(val: &Val) -> Result<Vec<(String, String)>, String> {
    let Val::List(items) = val else {
        return Err(format!("expected list<tuple<string, string>>, got {val:?}"));
    };
    items
        .iter()
        .map(|item| match item {
            Val::Tuple(pair) => match pair.as_slice() {
                [Val::String(k), Val::String(v)] => Ok((k.clone(), v.clone())),
                other => Err(format!("expected tuple<string, string>, got {other:?}")),
            },
            other => Err(format!("expected a tuple, got {other:?}")),
        })
        .collect()
}

fn find_field<'a>(fields: &'a [(String, Val)], name: &str) -> Option<&'a Val> {
    fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
}

fn response_from_record(fields: &[(String, Val)]) -> Result<GuestHttpResponse, GuestHttpFailure> {
    let status = match find_field(fields, "status") {
        Some(Val::U16(s)) => *s,
        other => {
            return Err(GuestHttpFailure::Malformed(format!(
                "expected http-response.status: u16, got {other:?}"
            )));
        }
    };
    let headers = match find_field(fields, "headers") {
        Some(val) => decode_string_pairs(val)
            .map_err(|e| GuestHttpFailure::Malformed(format!("http-response.headers: {e}")))?,
        None => return Err(GuestHttpFailure::Malformed("http-response missing headers".into())),
    };
    let body = match find_field(fields, "body") {
        Some(val) => val_list_to_bytes(val)
            .map_err(|e| GuestHttpFailure::Malformed(format!("http-response.body: {e}")))?,
        None => return Err(GuestHttpFailure::Malformed("http-response missing body".into())),
    };
    Ok(GuestHttpResponse { status, headers, body })
}

/// Decodes `handle-request`'s `result<http-response, string>` return.
///
/// Distinguishes a guest `Err(msg)` from a wrong return shape *before*
/// inspecting the `Ok` payload: both would otherwise collapse into the same
/// kind of error, and a deliberate guest rejection (`Declined`) must not be
/// reported the same way as a broken component (`Malformed`).
pub(crate) fn response_from_results(
    results: &[Val],
) -> Result<GuestHttpResponse, GuestHttpFailure> {
    let [val] = results else {
        return Err(GuestHttpFailure::Malformed(format!(
            "expected exactly 1 result<http-response, string> return value, got {}",
            results.len()
        )));
    };
    match val {
        Val::Result(Err(payload)) => {
            let msg = match payload.as_deref() {
                Some(Val::String(s)) => crate::engine::truncate_detail(s.clone()),
                Some(other) => crate::engine::truncate_detail(format!("{other:?}")),
                None => "guest declined the request".to_string(),
            };
            Err(GuestHttpFailure::Declined(msg))
        }
        Val::Result(Ok(Some(boxed))) => match boxed.as_ref() {
            Val::Record(fields) => response_from_record(fields),
            other => Err(GuestHttpFailure::Malformed(format!(
                "expected an http-response record, got {other:?}"
            ))),
        },
        other => Err(GuestHttpFailure::Malformed(format!(
            "expected result<http-response, string>, got {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> GuestHttpRequest {
        GuestHttpRequest {
            method: "GET".to_string(),
            path: "/items/42".to_string(),
            query: "a=b".to_string(),
            route: "/items/{id}".to_string(),
            path_params: vec![("id".to_string(), "42".to_string())],
            headers: vec![("x-test".to_string(), "1".to_string())],
            body: vec![1, 2, 3],
            caller: None,
        }
    }

    #[test]
    fn request_to_val_builds_fields_in_wit_declaration_order() {
        let Val::Record(fields) = request_to_val(&sample_request()) else {
            panic!("expected a record");
        };
        let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec!["method", "path", "query", "route", "path-params", "headers", "body", "caller"]
        );
    }

    #[test]
    fn request_to_val_path_params_is_a_list_of_tuples() {
        let Val::Record(fields) = request_to_val(&sample_request()) else {
            panic!("expected a record");
        };
        let path_params = find_field(fields.as_slice(), "path-params").unwrap();
        assert_eq!(
            *path_params,
            Val::List(vec![Val::Tuple(vec![
                Val::String("id".to_string()),
                Val::String("42".to_string())
            ])])
        );
    }

    #[test]
    fn request_to_val_caller_none_is_val_option_none() {
        let Val::Record(fields) = request_to_val(&sample_request()) else {
            panic!("expected a record");
        };
        assert_eq!(*find_field(fields.as_slice(), "caller").unwrap(), Val::Option(None));
    }

    #[test]
    fn request_to_val_caller_some_carries_did_and_auth() {
        let mut request = sample_request();
        request.caller = Some(GuestCallerIdentity {
            did: "did:key:z6Mk...".to_string(),
            auth: GuestCallerAuth::Ucan,
            app_instance: Some("instance-1".to_string()),
        });
        let Val::Record(fields) = request_to_val(&request) else {
            panic!("expected a record");
        };
        let Val::Option(Some(caller_val)) = find_field(fields.as_slice(), "caller").unwrap() else {
            panic!("expected Some(caller)");
        };
        let Val::Record(caller_fields) = caller_val.as_ref() else {
            panic!("expected a record");
        };
        assert_eq!(
            *find_field(caller_fields.as_slice(), "did").unwrap(),
            Val::String("did:key:z6Mk...".to_string())
        );
        assert_eq!(
            *find_field(caller_fields.as_slice(), "auth").unwrap(),
            Val::Enum("ucan".to_string())
        );
        assert_eq!(
            *find_field(caller_fields.as_slice(), "app-instance").unwrap(),
            Val::Option(Some(Box::new(Val::String("instance-1".to_string()))))
        );
    }

    fn ok_response_val(status: u16, headers: Vec<(&str, &str)>, body: Vec<u8>) -> Val {
        Val::Result(Ok(Some(Box::new(Val::Record(vec![
            ("status".to_string(), Val::U16(status)),
            (
                "headers".to_string(),
                Val::List(
                    headers
                        .into_iter()
                        .map(|(k, v)| {
                            Val::Tuple(vec![Val::String(k.to_string()), Val::String(v.to_string())])
                        })
                        .collect(),
                ),
            ),
            ("body".to_string(), bytes_to_val_list(body)),
        ])))))
    }

    #[test]
    fn response_from_results_decodes_a_valid_record() {
        let results = [ok_response_val(200, vec![("content-type", "text/plain")], vec![1, 2, 3])];
        let response = response_from_results(&results).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.headers, vec![("content-type".to_string(), "text/plain".to_string())]);
        assert_eq!(response.body, vec![1, 2, 3]);
    }

    #[test]
    fn response_from_results_guest_err_is_declined_not_malformed() {
        let results =
            [Val::Result(Err(Some(Box::new(Val::String("comment is empty".to_string())))))];
        assert_eq!(
            response_from_results(&results).unwrap_err(),
            GuestHttpFailure::Declined("comment is empty".to_string())
        );
    }

    #[test]
    fn response_from_results_wrong_arity_is_malformed() {
        let results = [ok_response_val(200, vec![], vec![]), Val::Bool(true)];
        assert!(matches!(
            response_from_results(&results).unwrap_err(),
            GuestHttpFailure::Malformed(_)
        ));
    }

    #[test]
    fn response_from_results_non_record_ok_is_malformed() {
        let results = [Val::Result(Ok(Some(Box::new(Val::String("oops".to_string())))))];
        assert!(matches!(
            response_from_results(&results).unwrap_err(),
            GuestHttpFailure::Malformed(_)
        ));
    }

    #[test]
    fn response_from_results_missing_field_is_malformed() {
        let results = [Val::Result(Ok(Some(Box::new(Val::Record(vec![(
            "status".to_string(),
            Val::U16(200),
        )])))))];
        assert!(matches!(
            response_from_results(&results).unwrap_err(),
            GuestHttpFailure::Malformed(_)
        ));
    }

    #[test]
    fn response_from_results_wrong_field_type_is_malformed() {
        let results = [Val::Result(Ok(Some(Box::new(Val::Record(vec![
            ("status".to_string(), Val::String("200".to_string())),
            ("headers".to_string(), Val::List(vec![])),
            ("body".to_string(), Val::List(vec![])),
        ])))))];
        assert!(matches!(
            response_from_results(&results).unwrap_err(),
            GuestHttpFailure::Malformed(_)
        ));
    }

    #[test]
    fn response_from_results_non_u8_body_element_is_malformed() {
        let results = [Val::Result(Ok(Some(Box::new(Val::Record(vec![
            ("status".to_string(), Val::U16(200)),
            ("headers".to_string(), Val::List(vec![])),
            ("body".to_string(), Val::List(vec![Val::String("not a byte".to_string())])),
        ])))))];
        assert!(matches!(
            response_from_results(&results).unwrap_err(),
            GuestHttpFailure::Malformed(_)
        ));
    }
}
