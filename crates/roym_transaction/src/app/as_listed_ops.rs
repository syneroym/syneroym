//! The owner's side of "agree as listed": answering a tap, the status the
//! owner's Hub reads, and the counts both need.

pub(crate) mod accept;
pub(crate) mod answer;
pub(crate) mod status;

pub(crate) use accept::{AutoAcceptRow, accept_waiting, auto_accept};
pub(crate) use answer::{
    AnswerRow, AnswerState, FromListingInput, Trigger, answer_as_listed, answer_from_listing,
    answer_waiting,
};
pub(crate) use status::{AsListedStatusInput, AsListedStatusOutput, as_listed_status};
use syneroym_roym_core::{paging, progress::FormedAs};

use super::{AGREEMENTS, AgreementRow};

/// How long an instant agreement counts as open for the cap. A deal nobody
/// touched for a week no longer holds a place.
pub(crate) const OPEN_INSTANT_SECS: u64 = 7 * 86_400;

/// Why a tap waits, as the owner's Hub shows it.
pub(crate) const WAITING_ANSWER_IN_PROGRESS: &str = "answer-in-progress";
pub(crate) const WAITING_CONFIRM_FIRST: &str = "confirm-first";
pub(crate) const WAITING_CAP_REACHED: &str = "cap-reached";
pub(crate) const WAITING_COUNTERPARTY_HOLDS_ONE: &str = "counterparty-holds-one";
pub(crate) const WAITING_SIGNING_NOT_READY: &str = "signing-not-ready";
pub(crate) const WAITING_SLOT_CHECK_FAILED: &str = "slot-check-failed";

/// Whether an agreement is one of the listing's open instant agreements:
/// formed as instant, still open, no half filed, and younger than a week.
#[must_use]
pub(crate) fn is_open_instant(row: &AgreementRow, listing_id: &str, now: u64) -> bool {
    if row.formed_as() != FormedAs::AsListedInstant {
        return false;
    }
    if row.terms.listing_id.as_deref() != Some(listing_id) {
        return false;
    }
    if row.state.is_some_and(|state| state.is_terminal()) {
        return false;
    }
    if row.commitments.iter().any(|c| !c.halves.is_empty()) {
        return false;
    }
    row.agreed_at_secs.is_some_and(|at| now.saturating_sub(at) < OPEN_INSTANT_SECS)
}

/// Every open instant agreement of `listing_id`, oldest first.
pub(crate) async fn open_instant<H: syneroym_app_host::AppHost>(
    host: &H,
    listing_id: &str,
    now: u64,
) -> Result<Vec<AgreementRow>, String> {
    let rows: Vec<AgreementRow> = paging::query_all(host, AGREEMENTS, None).await?;
    let mut open: Vec<AgreementRow> =
        rows.into_iter().filter(|row| is_open_instant(row, listing_id, now)).collect();
    open.sort_by_key(|row| (row.agreed_at_secs.unwrap_or(0), row.quote_record_id.clone()));
    Ok(open)
}
