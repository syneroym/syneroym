use serde::{Deserialize, Serialize};
use serde_json::Value;
use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    envelope::Response,
    listing,
    method::{
        Reply,
        catalog::{SlotKeyInput, SlotRow},
    },
    paging,
};

use super::{AVAILABILITY, ensure_availability};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotInput {
    pub start_secs: u64,
    pub end_secs: u64,
    pub capacity: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvailabilitySetInput {
    pub listing_id: String,
    pub slots: Vec<SlotInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailabilitySetOutput {
    pub listing_id: String,
    pub slot_ids: Vec<String>,
}

pub(crate) async fn set<H: AppHost>(
    host: &H,
    input: AvailabilitySetInput,
) -> Reply<AvailabilitySetOutput> {
    if let Err(e) = ensure_availability(host).await {
        return Err(Response::internal_error(e));
    }
    let mut ids = Vec::new();
    for s in input.slots {
        if s.end_secs <= s.start_secs {
            return Err(Response::invalid_params("slot end_secs must be after start_secs"));
        }
        let id = match listing::derive_slot_id(&input.listing_id, s.start_secs, s.end_secs) {
            Ok(id) => id,
            Err(e) => return Err(Response::internal_error(e.to_string())),
        };
        let row = SlotRow {
            slot_id: id.clone(),
            listing_id: input.listing_id.clone(),
            start_secs: s.start_secs,
            end_secs: s.end_secs,
            capacity: s.capacity,
        };
        let payload =
            serde_json::to_vec(&row).map_err(|e| Response::internal_error(e.to_string()))?;
        if let Err(e) = AppDataLayer::put(
            host,
            AVAILABILITY.to_string(),
            RecordWriteValue { id: id.clone(), payload },
        )
        .await
        {
            return Err(Response::internal_error(e.to_string()));
        }
        ids.push(id);
    }
    Ok(AvailabilitySetOutput { listing_id: input.listing_id, slot_ids: ids })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvailabilityListInput {
    pub listing_id: String,
    #[serde(default)]
    pub from_secs: Option<u64>,
    #[serde(default)]
    pub to_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailabilityListOutput {
    pub slots: Vec<SlotRow>,
}

pub(crate) async fn list<H: AppHost>(
    host: &H,
    input: AvailabilityListInput,
) -> Reply<AvailabilityListOutput> {
    if let Err(e) = ensure_availability(host).await {
        return Err(Response::internal_error(e));
    }
    let rows: Vec<Value> = match paging::query_all(host, AVAILABILITY, None).await {
        Ok(v) => v,
        Err(e) => return Err(Response::internal_error(e)),
    };
    let mut slots: Vec<SlotRow> = rows
        .into_iter()
        .filter(|p| p.get("listing_id").and_then(Value::as_str) == Some(input.listing_id.as_str()))
        .filter_map(|p| serde_json::from_value::<SlotRow>(p).ok())
        .filter(|r| {
            input.from_secs.is_none_or(|f| r.start_secs >= f)
                && input.to_secs.is_none_or(|t| r.start_secs <= t)
        })
        .collect();
    slots.sort_by_key(|r| r.start_secs);
    Ok(AvailabilityListOutput { slots })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailabilityRemoveOutput {
    pub removed: bool,
}

pub(crate) async fn remove<H: AppHost>(
    host: &H,
    input: SlotKeyInput,
) -> Reply<AvailabilityRemoveOutput> {
    if let Err(e) = ensure_availability(host).await {
        return Err(Response::internal_error(e));
    }
    let existed = AppDataLayer::get(host, AVAILABILITY.to_string(), input.slot_id.clone())
        .await
        .map(|o| o.is_some())
        .unwrap_or(false);
    if existed
        && let Err(e) = AppDataLayer::delete(host, AVAILABILITY.to_string(), input.slot_id).await
    {
        return Err(Response::internal_error(e.to_string()));
    }
    Ok(AvailabilityRemoveOutput { removed: existed })
}

pub(crate) async fn get<H: AppHost>(host: &H, input: SlotKeyInput) -> Reply<Option<SlotRow>> {
    if let Err(e) = ensure_availability(host).await {
        return Err(Response::internal_error(e));
    }
    match AppDataLayer::get(host, AVAILABILITY.to_string(), input.slot_id).await {
        Ok(Some(row)) => {
            let val: SlotRow = serde_json::from_slice(&row.payload)
                .map_err(|e| Response::internal_error(e.to_string()))?;
            Ok(Some(val))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(Response::internal_error(e.to_string())),
    }
}
