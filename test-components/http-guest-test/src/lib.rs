#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Guest HTTP handler test component (M06A A2).
//!
//! Exercises `syneroym:http/incoming-handler#handle-request` end to end.
//! Behaviour switches on `request.path` -- see the match in `handle_request`
//! for what each one proves.

use bindings::{
    exports::{
        syneroym::http::incoming_handler::{
            CallerAuth, Guest as IncomingHandlerGuest, HttpRequest, HttpResponse,
        },
        syneroym_test::http_guest_test::test_driver::Guest as TestDriverGuest,
    },
    syneroym::data_layer::store,
};

mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "http-guest-test",
        with: {
            "syneroym:data-layer/store@0.1.0": generate,
            "syneroym:http/incoming-handler@0.1.0": generate,
        },
    });

    use super::HttpGuestTestComponent;
    export!(HttpGuestTestComponent);
}

const REQUESTS_COLLECTION: &str = "requests";
const LAST_REQUEST_ID: &str = "last";
/// Comfortably over the router's `MAX_GUEST_RESPONSE_BODY_BYTES` (1 MiB),
/// so `/huge` always trips the over-cap rejection regardless of that
/// constant's exact value.
const HUGE_BODY_LEN: usize = 2 * 1024 * 1024;

struct HttpGuestTestComponent;

fn caller_auth_str(auth: CallerAuth) -> &'static str {
    match auth {
        CallerAuth::Delegated => "delegated",
        CallerAuth::Ucan => "ucan",
        CallerAuth::SelfAsserted => "self-asserted",
    }
}

/// The shared JSON shape both `record_last_request` (persisted, read back by
/// `test-driver::last-request`) and `/echo` (returned directly in the HTTP
/// response) use to describe an inbound request.
fn describe_request(request: &HttpRequest) -> serde_json::Value {
    let caller = match &request.caller {
        Some(c) => serde_json::json!({
            "did": c.did,
            "auth": caller_auth_str(c.auth),
            "app_instance": c.app_instance,
        }),
        None => serde_json::Value::Null,
    };
    serde_json::json!({
        "method": request.method,
        "path": request.path,
        "query": request.query,
        "route": request.route,
        "path_params": request.path_params,
        "header_count": request.headers.len(),
        "body_len": request.body.len(),
        "caller": caller,
    })
}

/// Persists a JSON description of `request` so `test-driver::last-request`
/// can read it back from a later, entirely separate instantiation (a fresh
/// `Store`/`Instance` per `handle-request` call means nothing kept in guest
/// memory would survive).
fn record_last_request(request: &HttpRequest) {
    let Ok(payload) = serde_json::to_vec(&describe_request(request)) else { return };
    let record = store::RecordWriteValue { id: LAST_REQUEST_ID.to_string(), payload };
    let _ = store::put(REQUESTS_COLLECTION, &record);
}

fn ok(status: u16, body: Vec<u8>) -> Result<HttpResponse, String> {
    Ok(HttpResponse { status, headers: vec![], body })
}

fn ok_text(status: u16, body: impl Into<String>) -> Result<HttpResponse, String> {
    ok(status, body.into().into_bytes())
}

fn handle_echo(request: &HttpRequest) -> Result<HttpResponse, String> {
    ok_text(200, describe_request(request).to_string())
}

fn handle_items(request: &HttpRequest) -> Result<HttpResponse, String> {
    match request.path_params.first() {
        Some((_, id)) => ok_text(200, id.clone()),
        None => ok_text(400, "missing id path param"),
    }
}

fn handle_whoami(request: &HttpRequest) -> Result<HttpResponse, String> {
    let body = match &request.caller {
        Some(c) => format!("{}:{}", caller_auth_str(c.auth), c.did),
        None => "anonymous".to_string(),
    };
    ok_text(200, body)
}

fn handle_bad_header() -> Result<HttpResponse, String> {
    Ok(HttpResponse {
        status: 200,
        headers: vec![("x-bad".to_string(), "line1\r\nline2".to_string())],
        body: vec![],
    })
}

fn handle_framing() -> Result<HttpResponse, String> {
    Ok(HttpResponse {
        status: 200,
        headers: vec![
            ("content-length".to_string(), "999".to_string()),
            ("connection".to_string(), "close".to_string()),
        ],
        body: b"ok".to_vec(),
    })
}

/// Busy-waits `?ms=N` (default 1000) before answering 200. The query knob
/// lets a test tune how long one call holds its guest-HTTP admission permit
/// (M06A D-A2-11) without needing a second near-duplicate test path.
fn handle_slow(request: &HttpRequest) -> Result<HttpResponse, String> {
    let ms = request
        .query
        .split('&')
        .find_map(|kv| kv.strip_prefix("ms="))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1000);
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_millis(ms) {
        std::hint::spin_loop();
    }
    ok_text(200, "slow-ok")
}

impl IncomingHandlerGuest for HttpGuestTestComponent {
    fn handle_request(request: HttpRequest) -> Result<HttpResponse, String> {
        record_last_request(&request);
        match request.path.as_str() {
            "/echo" => handle_echo(&request),
            "/whoami" => handle_whoami(&request),
            "/reject" => ok_text(422, "comment is empty"),
            "/fail" => Err("handler blew up".to_string()),
            "/trap" => unreachable!("test sentinel: forced trap"),
            "/spin" => loop {
                std::hint::spin_loop();
            },
            "/huge" => ok(200, vec![b'x'; HUGE_BODY_LEN]),
            "/bad-header" => handle_bad_header(),
            "/framing" => handle_framing(),
            "/slow" => handle_slow(&request),
            path if path.starts_with("/items/") => handle_items(&request),
            other => ok_text(404, format!("no such test path: {other}")),
        }
    }
}

impl bindings::Guest for HttpGuestTestComponent {
    fn init() -> Result<(), String> {
        store::create_collection(&store::CollectionSchema {
            name: REQUESTS_COLLECTION.to_string(),
            indexes: vec![],
        })
        .map_err(|e| format!("{e:?}"))
    }

    fn migrate() -> Result<(), String> {
        Ok(())
    }
}

impl TestDriverGuest for HttpGuestTestComponent {
    fn last_request() -> Result<String, String> {
        match store::get(REQUESTS_COLLECTION, LAST_REQUEST_ID).map_err(|e| format!("{e:?}"))? {
            Some(record) => String::from_utf8(record.payload)
                .map_err(|e| format!("stored payload not valid UTF-8: {e}")),
            None => Ok(String::new()),
        }
    }
}
