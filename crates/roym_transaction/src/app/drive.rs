//! Node-local transaction driver.
//!
//! Periodic and event-triggered background processing for conversations
//! needing synchronization or timed transition evaluation (ADR-0031).

use serde::{Deserialize, Serialize};
use serde_json::json;
use syneroym_app_host::{AppHost, types::events::ReadResult};
use syneroym_roym_core::{
    clock,
    envelope::Response,
    method::{Reply, call_sibling},
    paging, services,
};

use super::{
    AGREEMENTS, AgreementRow, DRIVE_PENDING, DRIVE_STATE, as_listed_ops, decide, delete_row,
    ensure_collections, get_row, put_row,
    sync::{SyncError, sync_direct},
};

pub const MAX_DRIVE_CONVERSATIONS: usize = 8;
pub const MAX_DRIVE_FAILURES: u32 = 5;
pub const DRIVE_EVENT_READ: u32 = 500;
pub const DRIVE_RESCAN_PAGE: u64 = 16;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriveInput {}

#[derive(Debug, Serialize)]
pub struct DriveOutput {
    pub synced: u32,
    pub pending: u32,
    pub rescanning: bool,
    pub again: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PendingRow {
    pub(crate) since: u64,
    #[serde(default)]
    pub(crate) failures: u32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct DriveState {
    pub(crate) event_cursor: Option<String>,
    pub(crate) rescan_offset: Option<u64>,
    #[serde(default)]
    pub(crate) timed_cursor: usize,
}

#[derive(Debug, Deserialize)]
struct DriveConversationItem {
    id: String,
}

#[derive(Debug, Deserialize)]
struct ConversationListResult {
    conversations: Vec<DriveConversationItem>,
}

pub(crate) async fn drive<H: AppHost>(host: &H, input: DriveInput) -> Reply<DriveOutput> {
    drive_at(host, input, clock::now_secs()).await
}

pub(crate) async fn drive_at<H: AppHost>(
    host: &H,
    _input: DriveInput,
    now: u64,
) -> Reply<DriveOutput> {
    ensure_collections(host).await.map_err(Response::internal_error)?;

    let _ = decide::recover_deciding_rows(host, now).await;

    let mut state: DriveState = match get_row(host, DRIVE_STATE, "drive").await {
        Ok(Some(s)) => s,
        Ok(None) => DriveState::default(),
        Err(e) => return Err(Response::internal_error(e)),
    };
    let mut warnings = Vec::new();

    poll_events(host, &mut state, now, &mut warnings).await?;
    rescan_conversations(host, &mut state, now, &mut warnings).await?;

    let candidates = collect_candidates(host, &mut state, now).await?;
    let batch: Vec<(String, PendingRow)> =
        candidates.into_iter().take(MAX_DRIVE_CONVERSATIONS).collect();

    let synced_count = sync_batch(host, batch, now, &mut warnings).await?;
    as_listed_ops::answer_waiting(host, now, &mut warnings).await;
    as_listed_ops::accept_waiting(host, now, &mut warnings).await;

    let pending_count =
        paging::count(host, DRIVE_PENDING, None).await.map_err(Response::internal_error)? as u32;
    let again = synced_count == (MAX_DRIVE_CONVERSATIONS as u32) && pending_count > 0;
    let rescanning = state.rescan_offset.is_some();

    put_row(host, DRIVE_STATE, "drive", &state).await.map_err(Response::internal_error)?;

    Ok(DriveOutput { synced: synced_count, pending: pending_count, rescanning, again, warnings })
}

async fn poll_events<H: AppHost>(
    host: &H,
    state: &mut DriveState,
    now: u64,
    warnings: &mut Vec<String>,
) -> Result<(), Response> {
    if state.event_cursor.is_none() {
        match host.read_events(None, DRIVE_EVENT_READ).await {
            Ok(ReadResult::Events((_, head))) | Ok(ReadResult::Reload(head)) => {
                state.event_cursor = Some(head);
                state.rescan_offset = Some(0);
            }
            Err(e) => {
                warnings.push(format!("events read error: {e:?}"));
            }
        }
        return Ok(());
    }

    match host.read_events(state.event_cursor.clone(), DRIVE_EVENT_READ).await {
        Ok(ReadResult::Events((list, next))) => {
            for e in list {
                if e.kind == "conversation.message" {
                    put_row(
                        host,
                        DRIVE_PENDING,
                        &e.subject,
                        &PendingRow { since: now, failures: 0 },
                    )
                    .await
                    .map_err(Response::internal_error)?;
                }
            }
            state.event_cursor = Some(next);
        }
        Ok(ReadResult::Reload(head)) => {
            state.event_cursor = Some(head);
            state.rescan_offset = Some(0);
        }
        Err(e) => {
            warnings.push(format!("events read error: {e:?}"));
        }
    }
    Ok(())
}

async fn rescan_conversations<H: AppHost>(
    host: &H,
    state: &mut DriveState,
    now: u64,
    warnings: &mut Vec<String>,
) -> Result<(), Response> {
    let Some(off) = state.rescan_offset else {
        return Ok(());
    };

    let params = json!({
        "kind": "direct",
        "offset": off,
        "limit": DRIVE_RESCAN_PAGE,
    });
    match call_sibling::<_, serde_json::Value, ConversationListResult>(
        host,
        services::CONVERSATION,
        "conversation.list",
        &params,
    )
    .await
    {
        Ok(res) => {
            let page_len = res.conversations.len() as u64;
            for item in res.conversations {
                put_row(host, DRIVE_PENDING, &item.id, &PendingRow { since: now, failures: 0 })
                    .await
                    .map_err(Response::internal_error)?;
            }
            state.rescan_offset =
                if page_len < DRIVE_RESCAN_PAGE { None } else { Some(off + page_len) };
        }
        Err(e) => {
            warnings.push(format!("conversation.list rescan failed: {e}"));
        }
    }
    Ok(())
}

async fn collect_candidates<H: AppHost>(
    host: &H,
    state: &mut DriveState,
    now: u64,
) -> Result<Vec<(String, PendingRow)>, Response> {
    let mut candidates: Vec<(String, PendingRow)> =
        paging::query_all_with_ids(host, DRIVE_PENDING, None)
            .await
            .map_err(Response::internal_error)?;

    let agreements: Vec<AgreementRow> =
        paging::query_all(host, AGREEMENTS, None).await.unwrap_or_default();
    let mut unfinalized_convs: Vec<String> = agreements
        .into_iter()
        .filter(|a| a.state.as_ref().is_none_or(|s| !s.is_terminal()))
        .map(|a| a.conversation)
        .collect();
    unfinalized_convs.sort_unstable();
    unfinalized_convs.dedup();

    if !unfinalized_convs.is_empty() {
        if state.timed_cursor >= unfinalized_convs.len() {
            state.timed_cursor = 0;
        }
        let take_count = MAX_DRIVE_CONVERSATIONS.min(unfinalized_convs.len());
        for i in 0..take_count {
            let idx = (state.timed_cursor + i) % unfinalized_convs.len();
            let conv = &unfinalized_convs[idx];
            if !candidates.iter().any(|(id, _)| id == conv) {
                candidates.push((conv.clone(), PendingRow { since: now, failures: 0 }));
            }
        }
        state.timed_cursor = (state.timed_cursor + take_count) % unfinalized_convs.len();
    }

    candidates.sort_by_key(|(_, r)| r.since);
    Ok(candidates)
}

async fn sync_batch<H: AppHost>(
    host: &H,
    batch: Vec<(String, PendingRow)>,
    now: u64,
    warnings: &mut Vec<String>,
) -> Result<u32, Response> {
    let mut synced_count = 0;
    for (conv, cand) in batch {
        match sync_direct(host, &conv, false, now).await {
            Ok(out) => {
                synced_count += 1;
                if out.more {
                    put_row(host, DRIVE_PENDING, &conv, &PendingRow { since: now, failures: 0 })
                        .await
                        .map_err(Response::internal_error)?;
                } else {
                    delete_pending_if_same(host, &conv, cand.since).await?;
                }
            }
            Err(SyncError::NotDirect) => {
                delete_pending_if_same(host, &conv, cand.since).await?;
            }
            Err(SyncError::Internal(err)) => {
                if cand.failures + 1 >= MAX_DRIVE_FAILURES {
                    delete_pending_if_same(host, &conv, cand.since).await?;
                    warnings.push(format!("gave up on {conv}: {err}"));
                } else {
                    update_pending_failure_if_same(host, &conv, cand.since, now, cand.failures + 1)
                        .await?;
                }
            }
        }
    }
    Ok(synced_count)
}

async fn delete_pending_if_same<H: AppHost>(
    host: &H,
    conv: &str,
    expected_since: u64,
) -> Result<(), Response> {
    let curr = get_row::<PendingRow, _>(host, DRIVE_PENDING, conv)
        .await
        .map_err(Response::internal_error)?;
    if let Some(curr) = curr
        && curr.since == expected_since
    {
        delete_row(host, DRIVE_PENDING, conv).await.map_err(Response::internal_error)?;
    }
    Ok(())
}

async fn update_pending_failure_if_same<H: AppHost>(
    host: &H,
    conv: &str,
    expected_since: u64,
    now: u64,
    failures: u32,
) -> Result<(), Response> {
    let curr = get_row::<PendingRow, _>(host, DRIVE_PENDING, conv)
        .await
        .map_err(Response::internal_error)?;
    if let Some(curr) = curr
        && curr.since == expected_since
    {
        put_row(host, DRIVE_PENDING, conv, &PendingRow { since: now, failures })
            .await
            .map_err(Response::internal_error)?;
    }
    Ok(())
}
