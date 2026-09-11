use super::*;

pub(super) fn blob_hash_from_path(path: &str) -> Option<&str> {
    let hash = path.strip_prefix("/blobs/")?;
    if hash.is_empty() || hash.contains('/') { None } else { Some(hash) }
}

/// Parses an HTTP query string (`k=v&k2=v2`) and percent-decodes keys and
/// values.
pub(super) fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter_map(|part| part.split_once('='))
        .map(|(k, v)| {
            let key = percent_encoding::percent_decode_str(k).decode_utf8_lossy().to_string();
            let val = percent_encoding::percent_decode_str(v).decode_utf8_lossy().to_string();
            (key, val)
        })
        .collect()
}

/// Maps a `GET`-with-query-string request onto `data-layer::query`'s
/// `query-options` (`filter`/`limit`/`cursor`). `limit` and `cursor` are
/// reserved keys mapped directly onto those fields; every other key becomes
/// an equality clause in the MongoDB-style filter document (`?status=open`
/// -> `{"status": "open"}`), matching `compile_filter`'s own `{field:
/// value}` equality shorthand (`crates/data_db/src/filter.rs`) -- string
/// values only, no operators (`$gt`, `$in`, ...) or type coercion. That
/// covers the common case this bridge is for; a route needing richer
/// filtering than plain-equality-AND can still be reached directly via the
/// JSON-RPC bridge, which takes a filter document verbatim. An absent or
/// empty query string maps to an unfiltered query (`filter: null`),
/// unchanged from before this mapping existed. A non-numeric `limit`
/// produces a `400`-worthy error message rather than being silently dropped.
pub(super) fn query_opts_from_query_string(query: &str) -> result::Result<Value, String> {
    let mut params = parse_query(query);
    let limit = match params.remove("limit") {
        Some(raw) => {
            let n = raw
                .parse::<u32>()
                .map_err(|_| format!("invalid `limit` query parameter: {raw:?}"))?;
            Value::Number(n.into())
        }
        None => Value::Null,
    };
    let cursor = params.remove("cursor").map_or(Value::Null, Value::String);
    let filter = if params.is_empty() {
        Value::Null
    } else {
        let filter_doc: serde_json::Map<String, Value> =
            params.into_iter().map(|(k, v)| (k, Value::String(v))).collect();
        Value::String(serde_json::to_string(&filter_doc).map_err(|e| e.to_string())?)
    };
    Ok(serde_json::json!({"filter": filter, "limit": limit, "cursor": cursor}))
}

/// The `svc/<service-id>/` prefix `namespace_topic` adds is a substrate
/// implementation detail. An SSE subscriber names topics the way the route
/// table does, so the wire carries the service-relative name -- a browser
/// cannot subscribe by a name that embeds the deployment's own DID.
/// A topic that is not in this service's namespace (a cross-service
/// subscription) is passed through whole.
pub(super) fn service_relative_topic<'a>(service_id: &str, topic: &'a str) -> &'a str {
    topic
        .strip_prefix("svc/")
        .and_then(|rest| rest.strip_prefix(service_id))
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(topic)
}

/// Formats one broker-delivered `(topic, payload)` message as an SSE frame.
///
/// Payload is treated as UTF-8 text (lossy) -- every fixture in this repo
/// only ever publishes UTF-8 text payloads, and SSE's `data:` framing is
/// line-oriented, so a payload
/// containing newlines emits multiple `data:` lines.
///
/// Topic replaces `\r` and `\n` with spaces: publisher-supplied topics from
/// `MqttBroker` do not validate characters, and unescaped CR/LF in a
/// single-line `event:` field would allow a publisher to inject fabricated
/// `data:` or `event:` lines into another subscriber's stream.
pub(super) fn guest_request_headers(
    headers: &HeaderMap,
) -> result::Result<Vec<(String, String)>, (StatusCode, String)> {
    let mut out = Vec::new();
    for (name, value) in headers {
        let lower = name.as_str().to_ascii_lowercase();
        if HOST_OWNED_HEADERS.contains(&lower.as_str()) {
            continue;
        }
        let Ok(text) = value.to_str() else { continue };
        if out.len() == MAX_GUEST_REQUEST_HEADERS {
            return Err((
                StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                format!("request has more than {MAX_GUEST_REQUEST_HEADERS} headers"),
            ));
        }
        out.push((lower, text.to_string()));
    }
    Ok(out)
}
