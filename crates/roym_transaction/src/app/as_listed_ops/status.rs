//! The owner's Hub view of one offering's as-listed state.

use serde::{Deserialize, Serialize};
use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    as_listed::AsListed, clock, envelope::Response, listing::Acceptance, method::Reply, paging,
    signing, transaction,
};

use super::{
    super::{
        ENQUIRIES, ENQUIRY_HISTORY, LEDGER, RecordPointerRow, ensure_collections, get_bytes,
        get_row,
    },
    WAITING_ANSWER_IN_PROGRESS,
    answer::{
        AnswerRow, AnswerState, CurrentListing, Trigger, Verdict, answer_id, evaluate,
        in_flight_instant, load_current_listing,
    },
    open_instant,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsListedStatusInput {
    pub listing_id: String,
}

#[derive(Debug, Serialize)]
pub struct AsListedStatusOutput {
    pub acceptance: Acceptance,
    pub instant_cap: u16,
    pub open_instant: Vec<String>,
    pub in_flight_instant: u32,
    pub signing_ready: bool,
    pub waiting: Vec<WaitingEnquiry>,
}

/// One tap the owner's answer has not resolved.
#[derive(Debug, Serialize)]
pub struct WaitingEnquiry {
    pub enquiry_record_id: String,
    pub conversation: String,
    pub reason: String,
}

pub(crate) async fn as_listed_status<H: AppHost>(
    host: &H,
    input: AsListedStatusInput,
) -> Reply<AsListedStatusOutput> {
    as_listed_status_at(host, input, clock::now_secs()).await
}

pub(crate) async fn as_listed_status_at<H: AppHost>(
    host: &H,
    input: AsListedStatusInput,
    now: u64,
) -> Reply<AsListedStatusOutput> {
    ensure_collections(host).await.map_err(Response::internal_error)?;
    let listing_id = input.listing_id;
    let Some(current) = load_current_listing(host, &listing_id, now).await? else {
        return Err(Response::invalid_params("no such listing"));
    };
    let signing_ready = signing::person_principal(host, now).await.is_ok();
    let open: Vec<String> = open_instant(host, &listing_id, now)
        .await
        .map_err(Response::internal_error)?
        .into_iter()
        .map(|row| row.quote_record_id)
        .collect();
    let in_flight =
        in_flight_instant(host, &listing_id, now).await.map_err(Response::internal_error)?;

    let waiting = waiting_enquiries(host, &listing_id, &current, now).await?;

    Ok(AsListedStatusOutput {
        acceptance: current.payload.acceptance,
        instant_cap: current.payload.instant_cap,
        open_instant: open,
        in_flight_instant: in_flight,
        signing_ready,
        waiting,
    })
}

async fn waiting_enquiries<H: AppHost>(
    host: &H,
    listing_id: &str,
    current: &CurrentListing,
    now: u64,
) -> Result<Vec<WaitingEnquiry>, Response> {
    let rows: Vec<RecordPointerRow> =
        paging::query_all(host, ENQUIRIES, None).await.map_err(Response::internal_error)?;
    let mut waiting = Vec::new();
    for row in rows {
        if row.mine {
            continue;
        }
        let Some((payload, issuer)) = held_enquiry(host, &row.record_id, now).await? else {
            continue;
        };
        let (Some(consent), Some(enquiry_listing)) = (&payload.as_listed, &payload.listing_id)
        else {
            continue;
        };
        if enquiry_listing != listing_id || now > consent.valid_until_secs {
            continue;
        }
        match answer_state(host, &row.record_id).await? {
            Some(AnswerState::Answered | AnswerState::Declined) => continue,
            Some(AnswerState::Answering) => {
                waiting.push(WaitingEnquiry {
                    enquiry_record_id: row.record_id,
                    conversation: row.conversation,
                    reason: WAITING_ANSWER_IN_PROGRESS.to_string(),
                });
                continue;
            }
            None => {}
        }
        let consent = AsListed {
            record_id: current.record_id.clone(),
            slot: consent.slot.clone(),
            variant: None,
            valid_until_secs: consent.valid_until_secs,
        };
        let verdict = evaluate(
            host,
            &current.payload,
            &consent,
            &current.owner,
            &issuer,
            Trigger::Driver,
            now,
        )
        .await?;
        match verdict {
            Verdict::Wait(reason) => waiting.push(WaitingEnquiry {
                enquiry_record_id: row.record_id,
                conversation: row.conversation,
                reason: reason.to_string(),
            }),
            Verdict::Answer { .. } | Verdict::Decline(_) => {}
        }
    }
    waiting.sort_by(|a, b| a.enquiry_record_id.cmp(&b.enquiry_record_id));
    Ok(waiting)
}

async fn answer_state<H: AppHost>(
    host: &H,
    enquiry_record_id: &str,
) -> Result<Option<AnswerState>, Response> {
    Ok(get_row::<AnswerRow, _>(host, LEDGER, &answer_id(enquiry_record_id))
        .await
        .map_err(Response::internal_error)?
        .map(|row| row.state))
}

async fn held_enquiry<H: AppHost>(
    host: &H,
    record_id: &str,
    now: u64,
) -> Result<Option<(transaction::EnquiryPayload, String)>, Response> {
    let Some(bytes) =
        get_bytes(host, ENQUIRY_HISTORY, record_id).await.map_err(Response::internal_error)?
    else {
        return Ok(None);
    };
    let Ok(envelope) = String::from_utf8(bytes) else { return Ok(None) };
    let verdict = transaction::verify_enquiry(&envelope, now);
    let (Some(payload), Some(issuer)) = (verdict.payload, verdict.issuer) else {
        return Ok(None);
    };
    Ok(Some((payload, issuer)))
}
