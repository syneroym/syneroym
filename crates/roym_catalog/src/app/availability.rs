use super::*;

const SLOT_ID_PREFIX: &str = "slot_";

#[derive(Debug, Deserialize)]
struct SlotInput {
    start_secs: u64,
    end_secs: u64,
    capacity: u32,
}

fn slot_id(listing_id: &str, start_secs: u64, end_secs: u64) -> Result<String, String> {
    content_digest(
        SLOT_ID_PREFIX,
        &json!({ "listing_id": listing_id, "start_secs": start_secs, "end_secs": end_secs }),
    )
    .map_err(|e| e.to_string())
}

pub(super) async fn availability_set<H: AppHost>(host: &H, req: &Request) -> Response {
    let listing_id = match req.params.get("listing_id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => return Response::invalid_params("listing_id is required"),
    };
    let slots: Vec<SlotInput> = match req.params.get("slots").cloned() {
        Some(v) => match serde_json::from_value(v) {
            Ok(s) => s,
            Err(e) => return Response::invalid_params(format!("invalid slots: {e}")),
        },
        None => return Response::invalid_params("slots is required"),
    };
    if let Err(e) = ensure_availability(host).await {
        return Response::internal_error(e);
    }
    let mut ids = Vec::new();
    for s in slots {
        if s.end_secs <= s.start_secs {
            return Response::invalid_params("slot end_secs must be after start_secs");
        }
        let id = match slot_id(&listing_id, s.start_secs, s.end_secs) {
            Ok(id) => id,
            Err(e) => return Response::internal_error(e),
        };
        let row = json!({
            "slot_id": id,
            "listing_id": listing_id,
            "start_secs": s.start_secs,
            "end_secs": s.end_secs,
            "capacity": s.capacity,
        });
        if let Err(e) = AppDataLayer::put(
            host,
            AVAILABILITY.to_string(),
            RecordWriteValue {
                id: id.clone(),
                payload: serde_json::to_vec(&row).unwrap_or_default(),
            },
        )
        .await
        {
            return Response::internal_error(e.to_string());
        }
        ids.push(id);
    }
    Response::ok(json!({ "listing_id": listing_id, "slot_ids": ids }))
}

pub(super) async fn availability_list<H: AppHost>(host: &H, req: &Request) -> Response {
    let listing_id = match req.params.get("listing_id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => return Response::invalid_params("listing_id is required"),
    };
    let from = req.params.get("from_secs").and_then(Value::as_u64);
    let to = req.params.get("to_secs").and_then(Value::as_u64);
    if let Err(e) = ensure_availability(host).await {
        return Response::internal_error(e);
    }
    let rows = match backup::collect(host, AVAILABILITY).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let mut slots: Vec<Value> = rows
        .into_iter()
        .filter_map(|r| r.get("payload").cloned())
        .filter(|p| p.get("listing_id").and_then(Value::as_str) == Some(listing_id.as_str()))
        .filter(|p| {
            let start = p.get("start_secs").and_then(Value::as_u64).unwrap_or(0);
            from.map(|f| start >= f).unwrap_or(true) && to.map(|t| start <= t).unwrap_or(true)
        })
        .collect();
    slots.sort_by_key(|p| p.get("start_secs").and_then(Value::as_u64).unwrap_or(0));
    Response::ok(json!({ "slots": slots }))
}

pub(super) async fn availability_remove<H: AppHost>(host: &H, req: &Request) -> Response {
    let slot_id = match req.params.get("slot_id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => return Response::invalid_params("slot_id is required"),
    };
    if let Err(e) = ensure_availability(host).await {
        return Response::internal_error(e);
    }
    let existed = AppDataLayer::get(host, AVAILABILITY.to_string(), slot_id.clone())
        .await
        .map(|o| o.is_some())
        .unwrap_or(false);
    if existed && let Err(e) = AppDataLayer::delete(host, AVAILABILITY.to_string(), slot_id).await {
        return Response::internal_error(e.to_string());
    }
    Response::ok(json!({ "removed": existed }))
}
