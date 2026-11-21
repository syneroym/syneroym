//! The `prepare` hook: the checks `quote.set` needs before it signs a slot
//! commitment, and the window it fills in.

use serde_json::Value;
use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    envelope::Response,
    method::{
        Reply,
        scheduling::{PrepareInput, PrepareOutput, SlotKeyInput, code},
    },
    transaction::TimeWindow,
};

use super::{availability, ensure_availability, refuse};

/// Checks that the slot a commitment names exists, belongs to the quote's
/// listing and has the window the author gave, and answers the slot's window.
pub(crate) async fn prepare<H: AppHost>(host: &H, input: PrepareInput) -> Reply<PrepareOutput> {
    let data = &input.details.data;
    let slot_id = data
        .get("slot_id")
        .and_then(Value::as_str)
        .ok_or_else(|| Response::invalid_params("details.data requires slot_id"))?;
    let listing_id =
        match (input.listing_id.as_deref(), data.get("listing_id").and_then(Value::as_str)) {
            (Some(terms), Some(detail)) if terms != detail => {
                return Err(refuse(code::SLOT_NOT_IN_LISTING));
            }
            (Some(listing), _) | (None, Some(listing)) => listing,
            (None, None) => return Err(Response::invalid_params("slot requires listing_id")),
        };
    ensure_availability(host).await.map_err(Response::internal_error)?;
    let Some(slot) = availability::get(host, SlotKeyInput { slot_id: slot_id.to_string() }).await?
    else {
        return Err(refuse(code::NO_SUCH_SLOT));
    };
    if slot.listing_id != listing_id {
        return Err(refuse(code::SLOT_NOT_IN_LISTING));
    }
    if let Some(given) = input.window
        && (given.earliest_secs != slot.start_secs || given.latest_secs != slot.end_secs)
    {
        return Err(refuse(code::SCHEDULE_DIFFERS_FROM_SLOT));
    }
    Ok(PrepareOutput {
        window: TimeWindow { earliest_secs: slot.start_secs, latest_secs: slot.end_secs },
    })
}
