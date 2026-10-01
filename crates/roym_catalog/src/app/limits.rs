use super::*;

pub(crate) async fn load_publication_limits<H: AppHost>(
    host: &H,
) -> Result<PublicationLimits, String> {
    ensure_coll(host, SETTINGS, &[]).await?;
    let row = AppDataLayer::get(host, SETTINGS.to_string(), PUBLICATION_LIMITS_KEY.to_string())
        .await
        .map_err(|e| e.to_string())?;
    match row {
        Some(r) => serde_json::from_slice(&r.payload).map_err(|e| e.to_string()),
        None => Ok(PublicationLimits::default()),
    }
}

pub(crate) async fn publication_secs_in_window<H: AppHost>(
    host: &H,
    limits: &PublicationLimits,
    now: u64,
) -> Result<Vec<u64>, String> {
    ensure_coll(host, PUBLICATIONS, &[idx("at_secs", IndexType::Numeric)]).await?;
    let floor = now.saturating_sub(limits.window_secs);
    // Filtered at the host rather than scanned and dropped in the guest.
    // Still not an indexed scan -- the filter compiler binds the JSON
    // path as a parameter, which SQLite will not match against an
    // expression index -- but far fewer rows cross the host boundary.
    let filter = json!({ "at_secs": { "$gt": floor } });
    paging::filter_map(host, PUBLICATIONS, Some(filter.to_string()), |row| {
        let v: Value = serde_json::from_slice(&row.payload).ok()?;
        v.get("at_secs").and_then(Value::as_u64)
    })
    .await
}

pub(crate) async fn set_limits<H: AppHost>(host: &H, req: &Request) -> Response {
    let window_secs = match req.params.get("window_secs").and_then(Value::as_u64) {
        Some(w) => w,
        None => return Response::invalid_params("window_secs is required"),
    };
    let max_per_window = match req.params.get("max_per_window").and_then(Value::as_u64) {
        Some(m) => m as u32,
        None => return Response::invalid_params("max_per_window is required"),
    };
    let limits = PublicationLimits { window_secs, max_per_window };
    if let Err(e) = limits.validate() {
        return Response::invalid_params(e.to_string());
    }
    if let Err(e) = ensure_coll(host, SETTINGS, &[]).await {
        return Response::internal_error(e);
    }
    let payload = match serde_json::to_vec(&limits) {
        Ok(b) => b,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    if let Err(e) = AppDataLayer::put(
        host,
        SETTINGS.to_string(),
        RecordWriteValue { id: PUBLICATION_LIMITS_KEY.to_string(), payload },
    )
    .await
    {
        return Response::internal_error(e.to_string());
    }
    Response::ok(json!(limits))
}
