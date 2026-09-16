//! Transaction thread and card pagination.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Value, json};
use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    clock,
    envelope::{Request, Response},
};

use super::{
    CARDS, CardRow, QUOTES, REQUESTS, RecordPointerRow, collect_typed, ensure_collections,
};

#[derive(Debug, Deserialize)]
struct ThreadParams {
    conversation: String,
    #[serde(default = "default_thread_limit")]
    limit: usize,
    #[serde(default)]
    cursor: usize,
}

fn default_thread_limit() -> usize {
    200
}

pub(crate) async fn thread<H: AppHost>(host: &H, req: &Request) -> Response {
    let params: ThreadParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let now = clock::now_secs();
    let filter = json!({ "conversation": params.conversation }).to_string();

    let quote_pointers: Vec<RecordPointerRow> =
        match collect_typed(host, QUOTES, Some(filter.clone())).await {
            Ok(v) => v,
            Err(e) => return e,
        };
    let quote_map: HashMap<String, RecordPointerRow> =
        quote_pointers.into_iter().map(|p| (p.id.clone(), p)).collect();

    let request_pointers: Vec<RecordPointerRow> =
        match collect_typed(host, REQUESTS, Some(filter.clone())).await {
            Ok(v) => v,
            Err(e) => return e,
        };
    let request_map: HashMap<String, RecordPointerRow> =
        request_pointers.into_iter().map(|p| (p.id.clone(), p)).collect();

    let mut rows: Vec<CardRow> = match collect_typed(host, CARDS, Some(filter.clone())).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    for row in &mut rows {
        if row.card_type == "quote" && row.verified {
            if let Some(qid) =
                row.data.as_ref().and_then(|q| q.get("quote_id")).and_then(Value::as_str)
                && let Some(pointer) = quote_map.get(qid)
            {
                if pointer.declined_at_secs.is_some() {
                    row.declined = Some(true);
                }
                if row.version_count.is_none() {
                    row.version_count = Some(pointer.version_count);
                }
            }
            if let Some(exp) = row
                .data
                .as_ref()
                .and_then(|q| q.get("terms"))
                .and_then(|t| t.get("quote_expires_at_secs"))
                .and_then(Value::as_u64)
            {
                row.expired = now >= exp;
            }
        } else if row.card_type == "request"
            && row.verified
            && let Some(rid) =
                row.data.as_ref().and_then(|r| r.get("request_id")).and_then(Value::as_str)
            && let Some(pointer) = request_map.get(rid)
            && row.version_count.is_none()
        {
            row.version_count = Some(pointer.version_count);
        }
    }

    // Sort by (sender_timestamp_ms, message_id). The author component is
    // absent because a card row's author is its issuer (a person DID), not the
    // conversation author, so mixing them would order transcripts differently.
    rows.sort_by(|a, b| {
        a.sender_timestamp_ms
            .cmp(&b.sender_timestamp_ms)
            .then_with(|| a.message_id.cmp(&b.message_id))
    });

    let page: Vec<CardRow> = rows.into_iter().skip(params.cursor).take(params.limit).collect();
    Response::ok(json!({ "cards": page }))
}
