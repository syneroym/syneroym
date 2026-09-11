use super::*;

/// The `data-layer`/`blob-store`/JSON-RPC error -> HTTP status mapping
/// table, defined once and reused by every bridged route. See
/// `data_layer_error`/`blob_error` in
/// `crates/control_plane/src/synsvc_native.rs` for the code assignments.
pub(super) fn status_for_rpc_error_code(code: i32) -> StatusCode {
    match code {
        -32001 => StatusCode::NOT_FOUND,         // blob not found
        -32002 => StatusCode::TOO_MANY_REQUESTS, // blob quota exceeded
        -32010 => StatusCode::FORBIDDEN,         // data-layer permission denied
        -32011 => StatusCode::NOT_FOUND,         // data-layer collection not found
        -32012 => StatusCode::BAD_REQUEST,       // data-layer schema violation
        -32013 => StatusCode::TOO_MANY_REQUESTS, // data-layer quota exceeded
        UNAUTHENTICATED_RPC_CODE => StatusCode::UNAUTHORIZED,
        -32602 => StatusCode::BAD_REQUEST, // JSON-RPC invalid params
        UNSUPPORTED_PROTOCOL_RPC_CODE | UNSUPPORTED_TARGET_RPC_CODE => StatusCode::NOT_IMPLEMENTED,
        PROXY_TRANSPORT_RPC_CODE => StatusCode::BAD_GATEWAY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
/// **Cache-Control is chosen by content type, not by path.**
/// `text/html`'s name is stable while its content changes every
/// deploy, so caching it immutably would pin a browser to a stale bundle
/// indefinitely. Everything else gets long-lived immutable caching, correct
/// for the bundler-hashed filenames a real asset pipeline produces.
pub(super) fn cache_control_for(content_type: &str) -> &'static str {
    if content_type.starts_with("text/html") {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    }
}

/// Whether an `If-None-Match` header value matches `etag` (always a strong
/// validator here -- the manifest's own content hash). Per RFC 9110
/// section 13.1.2: a bare `*` matches unconditionally (the entry was already
/// resolved by the caller, so a representation does currently exist), and
/// the header may otherwise carry a comma-separated list, each member
/// optionally weak (`W/"..."`) -- a weak comparison ignores that prefix,
/// same as a strong one, since this function is only ever asked "does the
/// client already have exactly this content", not "byte-for-byte
/// identical". A browser always echoes the token verbatim, so this is a
/// pure widening: a proxy or `fetch` caller sending a list or a weak
/// validator now gets a 304 instead of silently re-downloading the whole
/// body.
pub(super) fn if_none_match_hits(header: Option<&HeaderValue>, etag: &str) -> bool {
    let Some(value) = header.and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let value = value.trim();
    value == "*"
        || value.split(',').any(|candidate| {
            let candidate = candidate.trim();
            candidate.strip_prefix("W/").unwrap_or(candidate) == etag
        })
}

/// Parses an HTTP query string (`k=v&k2=v2`) and percent-decodes keys and
/// values.
pub(super) fn format_sse_frame(topic: &str, payload: &[u8]) -> String {
    let safe_topic: String =
        topic.chars().map(|c| if c == '\r' || c == '\n' { ' ' } else { c }).collect();
    let text = String::from_utf8_lossy(payload);
    let mut frame = format!("event: {safe_topic}\n");
    if text.is_empty() {
        frame.push_str("data: \n");
    } else {
        for line in text.lines() {
            frame.push_str("data: ");
            frame.push_str(line);
            frame.push('\n');
        }
    }
    frame.push('\n');
    frame
}

pub(super) fn full_body(bytes: Bytes) -> HttpBody {
    Full::new(bytes).boxed_unsync()
}

pub(super) fn json_response(status: StatusCode, value: &Value) -> Response<HttpBody> {
    let bytes = serde_json::to_vec(value).unwrap_or_else(|_| b"null".to_vec());
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(full_body(Bytes::from(bytes)))
        .unwrap_or_else(|_| Response::default())
}

pub(super) fn structured_rpc_error(
    status: StatusCode,
    code: i32,
    message: String,
) -> Response<HttpBody> {
    let body = JsonRpcErrorResponse {
        jsonrpc: "2.0".to_string(),
        error: JsonRpcError { code, message, data: None },
        id: None,
    };
    json_response(status, &serde_json::to_value(&body).unwrap_or(Value::Null))
}

/// Request headers a guest sees: lowercased, with
/// every `HOST_OWNED_HEADERS` entry removed (the host owns framing, not the
/// guest) and any non-UTF-8 value silently dropped rather than failing the
/// request. A free function so the filtering rule is unit-testable without
/// a live `HttpHandler`, same as `blob_hash_from_path`/`if_none_match_hits`.
/// Error is `(status, message)`, not a built `Response`, so this stays a
/// small `Result` -- the caller builds the response with `http_error`.
/// Turns a guest's answer into an HTTP response, or into a 500 when the
/// guest's answer is malformed. `Content-Length` is always the
/// host's computed one, never the guest's -- a mismatch would be a
/// connection desync -- and an invalid header **fails the whole response**
/// rather than being silently dropped: a guest that thought it set
/// `Content-Type: application/json` must not silently serve
/// `application/octet-stream`.
pub(super) fn build_guest_response(response: HttpResponse) -> Response<HttpBody> {
    if response.body.len() > MAX_GUEST_RESPONSE_BODY_BYTES {
        return http_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("guest response body exceeds {MAX_GUEST_RESPONSE_BODY_BYTES} byte limit"),
        );
    }
    if response.headers.len() > MAX_GUEST_RESPONSE_HEADERS {
        return http_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("guest response declares more than {MAX_GUEST_RESPONSE_HEADERS} headers"),
        );
    }
    // 200-599 only: the WIT doc caps this range (1xx is informational, not
    // a final response), narrower than `StatusCode::from_u16`'s own
    // 100..=999 acceptance.
    if !(200..600).contains(&response.status) {
        return http_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("guest returned an out-of-range status: {}", response.status),
        );
    }
    let Ok(status) = StatusCode::from_u16(response.status) else {
        return http_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("guest returned an out-of-range status: {}", response.status),
        );
    };

    let mut builder = Response::builder().status(status);
    let mut saw_content_type = false;
    let mut saw_nosniff = false;
    for (name, value) in response.headers {
        let lower = name.to_ascii_lowercase();
        if HOST_OWNED_HEADERS.contains(&lower.as_str()) {
            debug!(header = %lower, "guest response header stripped -- host owns framing (D-A2-5)");
            continue;
        }
        let Ok(header_name) = HeaderName::from_bytes(lower.as_bytes()) else {
            return http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("guest returned an invalid header name: {name:?}"),
            );
        };
        let Ok(header_value) = HeaderValue::from_str(&value) else {
            return http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("guest returned an invalid value for header {lower}"),
            );
        };
        saw_content_type |= lower == CONTENT_TYPE.as_str();
        saw_nosniff |= lower == X_CONTENT_TYPE_OPTIONS.as_str();
        builder = builder.header(header_name, header_value);
    }
    if !saw_content_type {
        builder = builder.header(CONTENT_TYPE, "application/octet-stream");
    }
    if !saw_nosniff {
        builder = builder.header(X_CONTENT_TYPE_OPTIONS, "nosniff");
    }
    builder = builder.header(CONTENT_LENGTH, response.body.len().to_string());

    match builder.body(full_body(Bytes::from(response.body))) {
        Ok(resp) => resp,
        Err(e) => http_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to build guest response: {e}"),
        ),
    }
}

/// A short, safe-to-return sentence per `GuestHttpFailure` variant. The
/// guest's own string (in `Declined`/`BudgetExceeded`/`Trap`/`Malformed`) is
/// already truncated by the engine's `truncate_detail` before it reaches
/// here, so it is safe to include verbatim.
pub(super) fn describe_guest_http_failure(failure: &GuestHttpFailure) -> String {
    match failure {
        GuestHttpFailure::NoHandler => {
            "deployed component does not export the guest HTTP handler".to_string()
        }
        GuestHttpFailure::Declined(detail) => format!("guest HTTP handler failed: {detail}"),
        GuestHttpFailure::BudgetExceeded(detail) => {
            format!("guest HTTP handler exceeded its budget: {detail}")
        }
        GuestHttpFailure::Trap(detail) => format!("guest HTTP handler trapped: {detail}"),
        GuestHttpFailure::Malformed(detail) => {
            format!("guest HTTP handler returned a malformed response: {detail}")
        }
        // Handled by its own 503 branch at the call site; kept here so the
        // match stays exhaustive if a new caller reuses this function.
        GuestHttpFailure::Unavailable(detail) => {
            format!("guest HTTP handler unavailable: {detail}")
        }
    }
}
/// Formats a JSON-RPC error response within an HTTP response, using the
/// generic `-32603` internal-error code -- callers with a real mapped RPC
/// error code use `structured_rpc_error` instead, to preserve it.
pub fn http_error(status: StatusCode, message: String) -> Response<HttpBody> {
    let body = JsonRpcErrorResponse {
        jsonrpc: "2.0".to_string(),
        error: JsonRpcError { code: -32603, message, data: None },
        id: None,
    };
    json_response(status, &serde_json::to_value(&body).unwrap_or(Value::Null))
}
