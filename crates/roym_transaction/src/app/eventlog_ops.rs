//! Local agreement event log operations and derivation.

pub(crate) mod gather;

use serde::{Deserialize, Serialize};
use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    clock, envelope::Response, eventlog::EventLogEntry, paging, progress::AgreementState, signing,
};

use crate::app::{
    AGREEMENTS, AgreementRow, EVENT_LOG, EVENT_LOG_STATE, ensure_collections, get_row, put_row,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventLogStateRow {
    pub agreement: String,
    pub seq: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<AgreementState>,
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventLogCursor {
    pub recorded_at_secs: u64,
    pub id: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EventLogListInput {
    pub after: Option<EventLogCursor>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventLogListOutput {
    pub entries: Vec<EventLogEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<EventLogCursor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventLogRebuildOutput {
    pub made: u32,
}

pub(crate) async fn derive_agreement<H: AppHost>(
    host: &H,
    owner_did: &str,
    row: &AgreementRow,
    now: u64,
    ignore_marker: bool,
) -> Result<u32, String> {
    if !ignore_marker
        && let Some(marker) =
            get_row::<EventLogStateRow, _>(host, EVENT_LOG_STATE, &row.quote_record_id).await?
        && marker.complete
        && marker.seq == row.seq.unwrap_or(0)
        && marker.state == row.state
    {
        return Ok(0);
    }

    let derived = gather::gather_and_derive(host, owner_did, row).await?;
    let mut made = 0;

    for mut entry in derived.entries {
        if entry.recorded_at_secs == 0 {
            entry.recorded_at_secs = now;
        }
        let bytes = serde_json::to_vec(&entry).map_err(|e| e.to_string())?;
        let write = RecordWriteValue { id: entry.id.clone(), payload: bytes };
        match AppDataLayer::create(host, EVENT_LOG.to_string(), vec![write]).await {
            Ok(None) => made += 1,
            Ok(Some(_collision)) => {}
            Err(e) => return Err(e.to_string()),
        }
    }

    let marker = EventLogStateRow {
        agreement: row.quote_record_id.clone(),
        seq: row.seq.unwrap_or(0),
        state: row.state,
        complete: derived.complete,
    };
    put_row(host, EVENT_LOG_STATE, &row.quote_record_id, &marker).await?;

    Ok(made)
}

pub async fn derive_conversation<H: AppHost>(
    host: &H,
    conversation: &str,
    now: u64,
) -> Result<(), String> {
    ensure_collections(host).await?;
    let filter = serde_json::json!({ "conversation": conversation }).to_string();
    let rows: Vec<AgreementRow> = paging::query_all(host, AGREEMENTS, Some(filter)).await?;
    let owner_did = match signing::owner_did(host).await {
        Ok(did) => did,
        Err(e) => return Err(e.to_string()),
    };
    for row in &rows {
        let _ = derive_agreement(host, &owner_did, row, now, false).await;
    }
    Ok(())
}

pub async fn derive_all<H: AppHost>(host: &H, now: u64) -> Result<(), String> {
    ensure_collections(host).await?;
    let rows: Vec<AgreementRow> = paging::query_all(host, AGREEMENTS, None).await?;
    let owner_did = match signing::owner_did(host).await {
        Ok(did) => did,
        Err(e) => return Err(e.to_string()),
    };
    for row in &rows {
        let _ = derive_agreement(host, &owner_did, row, now, false).await;
    }
    Ok(())
}

pub async fn list<H: AppHost>(
    host: &H,
    input: EventLogListInput,
) -> Result<EventLogListOutput, Response> {
    list_at(host, input, clock::now_secs()).await
}

pub async fn list_at<H: AppHost>(
    host: &H,
    input: EventLogListInput,
    now: u64,
) -> Result<EventLogListOutput, Response> {
    ensure_collections(host).await.map_err(Response::internal_error)?;
    derive_all(host, now).await.map_err(Response::internal_error)?;

    let limit = input.limit.unwrap_or(100).clamp(1, 500) as usize;

    let filter = input.after.as_ref().map(|after| {
        serde_json::json!({
            "recorded_at_secs": { "$gte": after.recorded_at_secs }
        })
        .to_string()
    });

    let mut entries: Vec<EventLogEntry> =
        paging::query_all(host, EVENT_LOG, filter).await.map_err(Response::internal_error)?;

    entries
        .sort_by(|a, b| a.recorded_at_secs.cmp(&b.recorded_at_secs).then_with(|| a.id.cmp(&b.id)));

    if let Some(ref after) = input.after {
        drop_before_cursor(&mut entries, after);
    }

    let next = compute_next(&entries, limit);
    entries.truncate(limit);

    Ok(EventLogListOutput { entries, next })
}

fn drop_before_cursor(entries: &mut Vec<EventLogEntry>, cursor: &EventLogCursor) {
    entries.retain(|e| {
        e.recorded_at_secs > cursor.recorded_at_secs
            || (e.recorded_at_secs == cursor.recorded_at_secs && e.id.as_str() > cursor.id.as_str())
    });
}

fn compute_next(entries: &[EventLogEntry], limit: usize) -> Option<EventLogCursor> {
    if entries.len() > limit {
        entries
            .get(limit - 1)
            .map(|e| EventLogCursor { recorded_at_secs: e.recorded_at_secs, id: e.id.clone() })
    } else {
        None
    }
}

pub async fn rebuild<H: AppHost>(host: &H) -> Result<EventLogRebuildOutput, Response> {
    rebuild_at(host, clock::now_secs()).await
}

pub async fn rebuild_at<H: AppHost>(host: &H, now: u64) -> Result<EventLogRebuildOutput, Response> {
    ensure_collections(host).await.map_err(Response::internal_error)?;
    let rows: Vec<AgreementRow> =
        paging::query_all(host, AGREEMENTS, None).await.map_err(Response::internal_error)?;
    let owner_did =
        signing::owner_did(host).await.map_err(|e| Response::internal_error(e.to_string()))?;
    let mut total_made = 0;
    for row in &rows {
        let made = derive_agreement(host, &owner_did, row, now, true)
            .await
            .map_err(Response::internal_error)?;
        total_made += made;
    }
    Ok(EventLogRebuildOutput { made: total_made })
}
