use syneroym_identity::DelegationCertificate;
use syneroym_rpc::SessionContext;
use syneroym_ucan::CapabilityToken;

use super::*;

#[test]
fn blob_hash_from_path_extracts_a_bare_hash() {
    assert_eq!(blob_hash_from_path("/blobs/deadbeef"), Some("deadbeef"));
}

#[test]
fn blob_hash_from_path_rejects_non_blob_paths_and_nested_segments() {
    assert_eq!(blob_hash_from_path("/orders/abc"), None);
    assert_eq!(blob_hash_from_path("/blobs/"), None);
    assert_eq!(blob_hash_from_path("/blobs/a/b"), None);
}

#[test]
fn if_none_match_hits_a_wildcard() {
    let value = HeaderValue::from_static("*");
    assert!(if_none_match_hits(Some(&value), "\"abc\""));
}

#[test]
fn if_none_match_hits_an_exact_strong_etag() {
    let value = HeaderValue::from_static("\"abc\"");
    assert!(if_none_match_hits(Some(&value), "\"abc\""));
}

#[test]
fn if_none_match_hits_one_entry_in_a_comma_separated_list() {
    let value = HeaderValue::from_static("\"nope\", \"abc\", \"also-nope\"");
    assert!(if_none_match_hits(Some(&value), "\"abc\""));
}

#[test]
fn if_none_match_hits_a_weak_validator_by_stripping_the_prefix() {
    let value = HeaderValue::from_static("W/\"abc\"");
    assert!(if_none_match_hits(Some(&value), "\"abc\""));
}

#[test]
fn if_none_match_misses_a_different_etag_or_a_missing_header() {
    let value = HeaderValue::from_static("\"different\"");
    assert!(!if_none_match_hits(Some(&value), "\"abc\""));
    assert!(!if_none_match_hits(None, "\"abc\""));
}

#[test]
fn status_for_rpc_error_code_maps_every_known_code() {
    assert_eq!(status_for_rpc_error_code(-32001), StatusCode::NOT_FOUND);
    assert_eq!(status_for_rpc_error_code(-32002), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(status_for_rpc_error_code(-32010), StatusCode::FORBIDDEN);
    assert_eq!(status_for_rpc_error_code(-32011), StatusCode::NOT_FOUND);
    assert_eq!(status_for_rpc_error_code(-32012), StatusCode::BAD_REQUEST);
    assert_eq!(status_for_rpc_error_code(-32013), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(status_for_rpc_error_code(-32602), StatusCode::BAD_REQUEST);
    assert_eq!(status_for_rpc_error_code(-32603), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        status_for_rpc_error_code(UNSUPPORTED_PROTOCOL_RPC_CODE),
        StatusCode::NOT_IMPLEMENTED
    );
    assert_eq!(status_for_rpc_error_code(PROXY_TRANSPORT_RPC_CODE), StatusCode::BAD_GATEWAY);
    assert_eq!(status_for_rpc_error_code(UNSUPPORTED_TARGET_RPC_CODE), StatusCode::NOT_IMPLEMENTED);
    assert_eq!(status_for_rpc_error_code(-1), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn parse_query_parses_ampersand_separated_pairs_and_percent_decodes() {
    let parsed = parse_query("svc=abc&exp=123&sig=deadbeef&name=hello%20world&tag=a%26b");
    assert_eq!(parsed.get("svc"), Some(&"abc".to_string()));
    assert_eq!(parsed.get("exp"), Some(&"123".to_string()));
    assert_eq!(parsed.get("sig"), Some(&"deadbeef".to_string()));
    assert_eq!(parsed.get("name"), Some(&"hello world".to_string()));
    assert_eq!(parsed.get("tag"), Some(&"a&b".to_string()));
}

#[test]
fn query_opts_from_query_string_maps_reserved_and_filter_keys() {
    let opts = query_opts_from_query_string("status=open&limit=5&cursor=abc").unwrap();
    assert_eq!(opts["limit"], serde_json::json!(5));
    assert_eq!(opts["cursor"], serde_json::json!("abc"));
    let filter: Value = serde_json::from_str(opts["filter"].as_str().unwrap()).unwrap();
    assert_eq!(filter, serde_json::json!({"status": "open"}));
}

#[test]
fn query_opts_from_query_string_empty_query_is_unfiltered() {
    let opts = query_opts_from_query_string("").unwrap();
    assert_eq!(opts, serde_json::json!({"filter": null, "limit": null, "cursor": null}));
}

#[test]
fn query_opts_from_query_string_rejects_non_numeric_limit() {
    assert!(query_opts_from_query_string("limit=notanumber").is_err());
}

#[test]
fn format_sse_frame_includes_event_and_data_lines() {
    let frame = format_sse_frame("orders/new", b"hello");
    assert!(frame.starts_with("event: orders/new\n"));
    assert!(frame.contains("data: hello\n"));
    assert!(frame.ends_with("\n\n"));
}

#[test]
fn format_sse_frame_strips_embedded_newlines_from_topic() {
    // A publisher-controlled topic containing CR/LF must not be able to
    // inject extra `data:`/`event:` lines into the frame -- a topic
    // string is exactly one MQTT topic, exactly one `event:` line.
    let malicious = "orders/new\ndata: {\"fake\":true}\n\nevent: spoofed";
    let frame = format_sse_frame(malicious, b"hello");
    let event_lines = frame.lines().filter(|l| l.starts_with("event:")).count();
    let data_lines = frame.lines().filter(|l| l.starts_with("data:")).count();
    assert_eq!(event_lines, 1, "exactly one event: line, frame was:\n{frame}");
    assert_eq!(data_lines, 1, "exactly one data: line, frame was:\n{frame}");
    assert!(!frame.contains('\r'), "no raw CR should survive into the frame");
}

#[test]
fn service_relative_topic_strips_this_services_namespace() {
    assert_eq!(service_relative_topic("svc-a", "svc/svc-a/comment-updates"), "comment-updates");
}

#[test]
fn service_relative_topic_leaves_a_foreign_or_unprefixed_topic_whole() {
    assert_eq!(service_relative_topic("svc-a", "svc/svc-b/x"), "svc/svc-b/x");
    assert_eq!(service_relative_topic("svc-a", "plain"), "plain");
}

// -- guest HTTP route target ------------------------------------------

fn caller_context(auth: AuthLevel) -> CallerContext {
    CallerContext {
        caller_did: "did:key:caller".to_string(),
        app_instance: None,
        session: SessionContext { subject_did: "did:key:caller".to_string(), ..Default::default() },
        auth,
        proof: None,
    }
}

fn preamble_with(delegation: Option<()>, ucan: Option<()>) -> RoutePreamble {
    let mut preamble = RoutePreamble::binary_json_rpc("svc", "http-native");
    if delegation.is_some() {
        preamble.delegation = Some(DelegationCertificate {
            master_did: "did:key:master".to_string(),
            temporary_did: "did:key:temp".to_string(),
            issued_at_secs: 0,
            expires_at_secs: u64::MAX,
            scope: "routing".to_string(),
            signature: "test-signature".to_string(),
        });
    }
    if ucan.is_some() {
        // Only `preamble.ucan.is_some()`-ness is exercised by these
        // tests (F5b: a rejected chain still leaves this set) -- the
        // token's own fields don't need to verify.
        preamble.ucan = Some(CapabilityToken {
            issuer_did: "did:key:issuer".to_string(),
            audience_did: "did:key:caller".to_string(),
            anchor_did: None,
            capabilities: vec![],
            facts: serde_json::Map::new(),
            not_before_secs: 0,
            expires_at_secs: u64::MAX,
            proofs: vec![],
            signature: "junk-signature".to_string(),
        });
    }
    preamble
}

#[test]
fn guest_request_headers_lowercases_and_drops_host_owned() {
    let mut headers = HeaderMap::new();
    headers.insert("X-Test", HeaderValue::from_static("1"));
    headers.insert(CONTENT_LENGTH, HeaderValue::from_static("100"));
    headers.insert("Connection", HeaderValue::from_static("keep-alive"));
    let result = guest_request_headers(&headers).unwrap();
    assert_eq!(result, vec![("x-test".to_string(), "1".to_string())]);
}

#[test]
fn guest_request_headers_drops_non_utf8_values() {
    let mut headers = HeaderMap::new();
    headers.insert("x-binary", HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap());
    headers.insert("x-ok", HeaderValue::from_static("fine"));
    let result = guest_request_headers(&headers).unwrap();
    assert_eq!(result, vec![("x-ok".to_string(), "fine".to_string())]);
}

#[test]
fn guest_request_headers_431s_past_the_count_cap() {
    let mut headers = HeaderMap::new();
    for i in 0..MAX_GUEST_REQUEST_HEADERS + 1 {
        headers.insert(
            HeaderName::from_bytes(format!("x-h{i}").as_bytes()).unwrap(),
            HeaderValue::from_static("v"),
        );
    }
    let (status, _) = guest_request_headers(&headers).unwrap_err();
    assert_eq!(status, StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE);
}

#[test]
fn guest_caller_identity_none_stays_none() {
    let preamble = RoutePreamble::binary_json_rpc("svc", "http-native");
    assert_eq!(guest_caller_identity(None, &preamble).unwrap(), None);
}

#[test]
fn guest_caller_identity_bare_pubkey_is_self_asserted_even_though_auth_says_delegated() {
    // F5a: `AuthLevel::Delegated` is assigned to every verified preamble,
    // including the client gateway's unchallenged pubkey -- so `auth`
    // must not be read straight off it.
    let caller = caller_context(AuthLevel::Delegated);
    let preamble = preamble_with(None, None);
    let identity = guest_caller_identity(Some(&caller), &preamble).unwrap().unwrap();
    assert_eq!(identity.auth, CallerAuth::SelfAsserted);
}

#[test]
fn guest_caller_identity_a_rejected_ucan_is_self_asserted_not_ucan() {
    // F5b: `build_caller` fails open on a bad UCAN chain, leaving
    // `preamble.ucan` set but `CallerContext.auth` at `Delegated` -- so
    // keying `ucan` off the preamble would let any caller self-label
    // the strongest value with a junk token.
    let caller = caller_context(AuthLevel::Delegated);
    let preamble = preamble_with(None, Some(()));
    let identity = guest_caller_identity(Some(&caller), &preamble).unwrap().unwrap();
    assert_eq!(identity.auth, CallerAuth::SelfAsserted);
}

#[test]
fn guest_caller_identity_verified_ucan_is_ucan() {
    let caller = caller_context(AuthLevel::Ucan);
    let preamble = preamble_with(None, Some(()));
    let identity = guest_caller_identity(Some(&caller), &preamble).unwrap().unwrap();
    assert_eq!(identity.auth, CallerAuth::Ucan);
}

#[test]
fn guest_caller_identity_delegation_present_is_delegated() {
    let caller = caller_context(AuthLevel::Delegated);
    let preamble = preamble_with(Some(()), None);
    let identity = guest_caller_identity(Some(&caller), &preamble).unwrap().unwrap();
    assert_eq!(identity.auth, CallerAuth::Delegated);
}

#[test]
fn guest_caller_identity_session_caller_is_delegated() {
    let mut caller = caller_context(AuthLevel::Delegated);
    caller.session.claims.insert(SESSION_CALLER_MARKER.to_string(), serde_json::Value::Bool(true));
    let preamble = preamble_with(None, None);
    let identity = guest_caller_identity(Some(&caller), &preamble).unwrap().unwrap();
    assert_eq!(identity.auth, CallerAuth::Delegated);
}

#[test]
fn guest_caller_identity_fails_closed_on_substrate_injected_levels() {
    let preamble = RoutePreamble::binary_json_rpc("svc", "http-native");
    for level in [AuthLevel::LocalElevated, AuthLevel::LocalReadOnly, AuthLevel::System] {
        let caller = caller_context(level);
        assert!(guest_caller_identity(Some(&caller), &preamble).is_err());
    }
}

#[test]
fn http_route_with_no_public_key_deserializes_to_public_false() {
    let route: HttpRoute = serde_json::from_value(serde_json::json!({
        "method": "GET",
        "path": "/echo",
        "target": "guest",
        "operation": "handle-request"
    }))
    .unwrap();
    assert!(!route.public);
}

fn sample_guest_response(status: u16, headers: Vec<(&str, &str)>, body: Vec<u8>) -> HttpResponse {
    HttpResponse {
        status,
        headers: headers.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        body,
    }
}

#[test]
fn build_guest_response_strips_host_owned_headers() {
    let response = sample_guest_response(
        200,
        vec![("content-length", "999"), ("connection", "close"), ("x-ok", "1")],
        b"hi".to_vec(),
    );
    let resp = build_guest_response(response);
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get(CONTENT_LENGTH).unwrap(), "2");
    assert_eq!(resp.headers().get("x-ok").unwrap(), "1");
}

#[test]
fn build_guest_response_rejects_invalid_header_value_with_500() {
    let response = sample_guest_response(200, vec![("x-bad", "line1\r\nline2")], b"hi".to_vec());
    let resp = build_guest_response(response);
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn build_guest_response_rejects_invalid_header_name_with_500() {
    let response = sample_guest_response(200, vec![("x bad", "1")], b"hi".to_vec());
    let resp = build_guest_response(response);
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn build_guest_response_rejects_out_of_range_status() {
    for status in [0u16, 99, 100, 600, 999] {
        let response = sample_guest_response(status, vec![], vec![]);
        let resp = build_guest_response(response);
        assert_eq!(
            resp.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "status {status} should have been rejected"
        );
    }
}

#[test]
fn build_guest_response_rejects_over_cap_body() {
    let response = sample_guest_response(200, vec![], vec![0u8; MAX_GUEST_RESPONSE_BODY_BYTES + 1]);
    let resp = build_guest_response(response);
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn build_guest_response_rejects_over_cap_header_count() {
    let headers =
        (0..MAX_GUEST_RESPONSE_HEADERS + 1).map(|i| (format!("x-h{i}"), "v".to_string())).collect();
    let response = HttpResponse { status: 200, headers, body: vec![] };
    let resp = build_guest_response(response);
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn build_guest_response_adds_nosniff_only_when_absent() {
    let response = sample_guest_response(200, vec![], vec![]);
    let resp = build_guest_response(response);
    assert_eq!(resp.headers().get(X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff");

    let response = sample_guest_response(200, vec![("x-content-type-options", "custom")], vec![]);
    let resp = build_guest_response(response);
    assert_eq!(resp.headers().get(X_CONTENT_TYPE_OPTIONS).unwrap(), "custom");
}

#[test]
fn build_guest_response_keeps_repeated_set_cookie_headers() {
    let response =
        sample_guest_response(200, vec![("set-cookie", "a=1"), ("set-cookie", "b=2")], vec![]);
    let resp = build_guest_response(response);
    let values: Vec<&str> =
        resp.headers().get_all("set-cookie").iter().map(|v| v.to_str().unwrap()).collect();
    assert_eq!(values, vec!["a=1", "b=2"]);
}

#[test]
fn websocket_upgrade_headers_validation_accepts_valid_and_rejects_invalid() {
    let mut headers = HeaderMap::new();
    headers.insert("Upgrade", HeaderValue::from_static("websocket"));
    headers.insert("Connection", HeaderValue::from_static("Upgrade"));
    headers.insert("Sec-WebSocket-Version", HeaderValue::from_static("13"));
    headers.insert("Sec-WebSocket-Key", HeaderValue::from_static("dGhlIHNhbXBsZSBub25jZQ=="));
    assert_eq!(
        HttpHandler::validate_websocket_upgrade_headers(&headers),
        Ok("dGhlIHNhbXBsZSBub25jZQ==")
    );

    // Missing Sec-WebSocket-Key
    let mut bad_headers = headers.clone();
    bad_headers.remove("Sec-WebSocket-Key");
    assert!(HttpHandler::validate_websocket_upgrade_headers(&bad_headers).is_err());

    // Empty Sec-WebSocket-Key
    let mut empty_key = headers.clone();
    empty_key.insert("Sec-WebSocket-Key", HeaderValue::from_static(""));
    assert!(HttpHandler::validate_websocket_upgrade_headers(&empty_key).is_err());

    // Wrong version
    let mut bad_ver = headers.clone();
    bad_ver.insert("Sec-WebSocket-Version", HeaderValue::from_static("12"));
    assert!(HttpHandler::validate_websocket_upgrade_headers(&bad_ver).is_err());

    // Wrong connection
    let mut bad_conn = headers.clone();
    bad_conn.insert("Connection", HeaderValue::from_static("keep-alive"));
    assert!(HttpHandler::validate_websocket_upgrade_headers(&bad_conn).is_err());

    // Wrong upgrade
    let mut bad_up = headers.clone();
    bad_up.insert("Upgrade", HeaderValue::from_static("http2"));
    assert!(HttpHandler::validate_websocket_upgrade_headers(&bad_up).is_err());
}
