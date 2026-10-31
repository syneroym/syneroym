use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{IndexType, RecordWriteValue},
};
use syneroym_roym_core::{
    envelope::Response,
    method::{NoParams, PublicationLimitsInput, Reply},
    paging,
    safety::PublicationLimits,
};

use super::{PUBLICATION_LIMITS_KEY, PUBLICATIONS, SETTINGS, ensure_coll, idx};

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

pub(crate) async fn limits<H: AppHost>(host: &H, _: NoParams) -> Reply<PublicationLimits> {
    load_publication_limits(host).await.map_err(Response::internal_error)
}

pub(crate) async fn set_limits<H: AppHost>(
    host: &H,
    input: PublicationLimitsInput,
) -> Reply<PublicationLimits> {
    let limits =
        PublicationLimits { window_secs: input.window_secs, max_per_window: input.max_per_window };
    if let Err(e) = limits.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }
    if let Err(e) = ensure_coll(host, SETTINGS, &[]).await {
        return Err(Response::internal_error(e));
    }
    let payload =
        serde_json::to_vec(&limits).map_err(|e| Response::internal_error(e.to_string()))?;
    if let Err(e) = AppDataLayer::put(
        host,
        SETTINGS.to_string(),
        RecordWriteValue { id: PUBLICATION_LIMITS_KEY.to_string(), payload },
    )
    .await
    {
        return Err(Response::internal_error(e.to_string()));
    }
    Ok(limits)
}
