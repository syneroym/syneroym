//! Transaction thread and card pagination.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Value, json};
use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::QueryOptions};
use syneroym_roym_core::{
    clock,
    envelope::{Request, Response},
};

use super::{CARDS, CardRow, QUOTES, REQUESTS, RecordPointerRow, ensure_collections};

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

    let mut quote_map: HashMap<String, RecordPointerRow> = HashMap::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            QUOTES.to_string(),
            QueryOptions { filter: Some(filter.clone()), limit: Some(500), cursor: cursor.clone() },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            if let Ok(pointer) = serde_json::from_slice::<RecordPointerRow>(&r.payload) {
                quote_map.insert(pointer.id.clone(), pointer);
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }

    let mut request_map: HashMap<String, RecordPointerRow> = HashMap::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            REQUESTS.to_string(),
            QueryOptions { filter: Some(filter.clone()), limit: Some(500), cursor: cursor.clone() },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            if let Ok(pointer) = serde_json::from_slice::<RecordPointerRow>(&r.payload) {
                request_map.insert(pointer.id.clone(), pointer);
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }

    let mut rows = Vec::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            CARDS.to_string(),
            QueryOptions { filter: Some(filter.clone()), limit: Some(500), cursor: cursor.clone() },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            if let Ok(mut row) = serde_json::from_slice::<CardRow>(&r.payload) {
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
                rows.push(row);
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
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
