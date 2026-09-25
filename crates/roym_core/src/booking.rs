//! The booking a provider's transaction service writes, and the two
//! tracks that finish it. Written on the provider's node only; every
//! other node reads the writer's signed snapshots. Pure: no host calls.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    listing, person,
    record::{self, RECORD_BOOKING_PROGRESS, VerifyOptions},
    transaction::{PaymentTiming, Role, TimeWindow},
    verdict::RecordVerdict,
};

pub const BOOKING_PROGRESS_VERSION: u32 = 1;
/// How long a track stays open after the work's window, before it ends
/// at its named terminal holding whatever claim exists.
pub const TRACK_WINDOW_SECS: u64 = 30 * 24 * 3600;
/// Seats one slot can hand out. Bounds the claim loop.
pub const MAX_SLOT_CAPACITY: u32 = 64;
pub const MAX_CANCEL_REASON_LEN: usize = 512;
/// How long a card may wait for the card it depends on before it is
/// refused rather than deferred.
pub const MAX_DEFER_SECS: u64 = 7 * 24 * 3600;

pub const PAYMENT_NOTICE: &str = "This records what each side says about the payment. Roym does \
                                  not see the money move and cannot confirm that it did.";
pub const PROGRESS_NOTICE: &str =
    "This status comes from the provider's system. It is not a signed statement by either person.";
pub const PAYMENT_CLAIMED: &str = "The customer says they paid.";
pub const PAYMENT_ACKNOWLEDGED: &str = "The provider confirms they received the payment.";
pub const FULFILMENT_CLAIMED: &str = "The provider says the work is done.";
pub const FULFILMENT_ACKNOWLEDGED: &str = "The customer confirms the work is done.";
pub const TRACK_UNCONFIRMED: &str = "No confirmation was recorded before the window closed.";
pub const ONE_PAYMENT_NOTICE: &str =
    "This quote is paid in one payment. Deposits and part payments are not supported.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BookingState {
    Scheduled,
    InProgress,
    Completed,
    Cancelled,
    Conflict,
    EndedUnconfirmed,
}

impl BookingState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Conflict | Self::EndedUnconfirmed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrackState {
    None,
    Claimed,
    Acknowledged,
    Unconfirmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictReason {
    /// Every seat of the slot the quote named is held by another booking.
    SlotTaken,
    /// The slot the quote named no longer exists in the provider's catalog.
    SlotUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Track {
    Payment,
    Fulfilment,
}

/// What the writer is asked to apply. `now_secs` rides on every event so
/// the time edge is checked on every write, not only on reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookingEvent {
    Start,
    Cancel { reason: String },
    Half { track: Track, role: Role },
    Tick,
}

/// One snapshot. Signed as the `booking-progress` payload, so every
/// field is an integer, a string, or an enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookingProgressPayload {
    pub agreement: String,
    pub conversation: String,
    pub consumer_did: String,
    pub provider_did: String,
    pub seq: u32,
    pub state: BookingState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<ConflictReason>,
    pub payment: TrackState,
    pub fulfilment: TrackState,
    pub track_window_ends_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled_by: Option<Role>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransitionError {
    #[error("booking is {0:?}")]
    Terminal(BookingState),
    #[error("cannot start booking in state {0:?}")]
    CannotStart(BookingState),
    #[error("cannot cancel booking once a track has moved")]
    CannotCancel,
    #[error("cancel reason is over {MAX_CANCEL_REASON_LEN} characters")]
    ReasonTooLong,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BookingPayloadError {
    #[error("agreement '{0}' must start with 'rec_'")]
    InvalidAgreement(String),
    #[error("consumer_did '{0}' is not a valid did:key")]
    InvalidConsumerDid(String),
    #[error("provider_did '{0}' is not a valid did:key")]
    InvalidProviderDid(String),
    #[error("consumer and provider cannot be the same DID")]
    SamePartyDid,
    #[error("conversation is empty")]
    ConversationEmpty,
    #[error("seq must be >= 1, got {0}")]
    InvalidSeq(u32),
    #[error("conflict must be present if and only if state is conflict")]
    ConflictStateMismatch,
    #[error("cancellation fields must be present if and only if state is cancelled")]
    CancelledStateMismatch,
    #[error("cancel_reason is over {MAX_CANCEL_REASON_LEN} characters")]
    ReasonTooLong,
    #[error("completed state requires both tracks to be acknowledged")]
    CompletedTracksNotAcknowledged,
}

impl BookingProgressPayload {
    pub fn validate(&self) -> Result<(), BookingPayloadError> {
        if !self.agreement.starts_with("rec_") {
            return Err(BookingPayloadError::InvalidAgreement(self.agreement.clone()));
        }
        if !person::is_did_key(&self.consumer_did) {
            return Err(BookingPayloadError::InvalidConsumerDid(self.consumer_did.clone()));
        }
        if !person::is_did_key(&self.provider_did) {
            return Err(BookingPayloadError::InvalidProviderDid(self.provider_did.clone()));
        }
        if self.consumer_did == self.provider_did {
            return Err(BookingPayloadError::SamePartyDid);
        }
        if self.conversation.is_empty() {
            return Err(BookingPayloadError::ConversationEmpty);
        }
        if self.seq < 1 {
            return Err(BookingPayloadError::InvalidSeq(self.seq));
        }
        if self.conflict.is_some() != (self.state == BookingState::Conflict) {
            return Err(BookingPayloadError::ConflictStateMismatch);
        }
        let cancelled = self.state == BookingState::Cancelled;
        if self.cancelled_by.is_some() != cancelled || self.cancel_reason.is_some() != cancelled {
            return Err(BookingPayloadError::CancelledStateMismatch);
        }
        if self.cancel_reason.as_ref().is_some_and(|r| r.len() > MAX_CANCEL_REASON_LEN) {
            return Err(BookingPayloadError::ReasonTooLong);
        }
        if self.state == BookingState::Completed
            && (self.payment != TrackState::Acknowledged
                || self.fulfilment != TrackState::Acknowledged)
        {
            return Err(BookingPayloadError::CompletedTracksNotAcknowledged);
        }
        Ok(())
    }
}

/// The window end for a booking scheduled at `scheduled_at_secs`.
#[must_use]
pub fn track_window_end(schedule: Option<&TimeWindow>, scheduled_at_secs: u64) -> u64 {
    schedule.map_or(scheduled_at_secs, |w| w.latest_secs).saturating_add(TRACK_WINDOW_SECS)
}

/// True when the state is terminal and will receive no further transitions.
#[must_use]
pub fn is_terminal(state: BookingState) -> bool {
    state.is_terminal()
}

/// The first snapshot, seq 1: `scheduled`, or `conflict` with its reason.
pub fn open(
    agreement: String,
    conversation: String,
    consumer_did: String,
    provider_did: String,
    conflict: Option<ConflictReason>,
    window_end: u64,
) -> BookingProgressPayload {
    let state = if conflict.is_some() { BookingState::Conflict } else { BookingState::Scheduled };
    BookingProgressPayload {
        agreement,
        conversation,
        consumer_did,
        provider_did,
        seq: 1,
        state,
        conflict,
        payment: TrackState::None,
        fulfilment: TrackState::None,
        track_window_ends_at_secs: window_end,
        cancelled_by: None,
        cancel_reason: None,
    }
}

fn close_expired_tracks(n: &mut BookingProgressPayload, now_secs: u64) -> bool {
    if now_secs < n.track_window_ends_at_secs
        || !matches!(n.state, BookingState::Scheduled | BookingState::InProgress)
    {
        return false;
    }
    let mut changed = false;
    if matches!(n.payment, TrackState::None | TrackState::Claimed) {
        n.payment = TrackState::Unconfirmed;
        changed = true;
    }
    if matches!(n.fulfilment, TrackState::None | TrackState::Claimed) {
        n.fulfilment = TrackState::Unconfirmed;
        changed = true;
    }
    if changed {
        finish_if_both_terminal(n);
    }
    changed
}

fn is_track_terminal(t: TrackState) -> bool {
    matches!(t, TrackState::Acknowledged | TrackState::Unconfirmed)
}

fn finish_if_both_terminal(n: &mut BookingProgressPayload) {
    if is_track_terminal(n.payment) && is_track_terminal(n.fulfilment) {
        if n.payment == TrackState::Acknowledged && n.fulfilment == TrackState::Acknowledged {
            n.state = BookingState::Completed;
        } else {
            n.state = BookingState::EndedUnconfirmed;
        }
    }
}

fn apply_half(n: &mut BookingProgressPayload, track: Track, role: Role) {
    let against_interest = matches!(
        (track, role),
        (Track::Payment, Role::Provider) | (Track::Fulfilment, Role::Consumer)
    );
    let t = match track {
        Track::Payment => &mut n.payment,
        Track::Fulfilment => &mut n.fulfilment,
    };
    *t = match (*t, against_interest) {
        (TrackState::Unconfirmed, _) => TrackState::Unconfirmed,
        (_, true) => TrackState::Acknowledged,
        (TrackState::None, false) => TrackState::Claimed,
        (other, false) => other,
    };
    if n.state == BookingState::Scheduled {
        n.state = BookingState::InProgress;
    }
    if n.payment == TrackState::Acknowledged && n.fulfilment == TrackState::Acknowledged {
        n.state = BookingState::Completed;
    }
}

/// `Ok(None)` means "no change" -- the idempotent answer to a repeated
/// event. The caller sets `seq` on the returned snapshot.
pub fn apply(
    s: &BookingProgressPayload,
    e: &BookingEvent,
    now_secs: u64,
) -> Result<Option<BookingProgressPayload>, TransitionError> {
    let mut n = s.clone();
    let changed = close_expired_tracks(&mut n, now_secs);
    if is_terminal(n.state) {
        return match e {
            BookingEvent::Tick => Ok(changed.then_some(n)),
            BookingEvent::Cancel { .. } if n.state == BookingState::Cancelled => Ok(None),
            _ if changed => Ok(Some(n)),
            _ => Err(TransitionError::Terminal(n.state)),
        };
    }
    match e {
        BookingEvent::Start => match n.state {
            BookingState::Scheduled => n.state = BookingState::InProgress,
            BookingState::InProgress => return Ok(changed.then_some(n)),
            other => return Err(TransitionError::CannotStart(other)),
        },
        BookingEvent::Cancel { reason } => {
            if reason.len() > MAX_CANCEL_REASON_LEN {
                return Err(TransitionError::ReasonTooLong);
            }
            if n.payment != TrackState::None || n.fulfilment != TrackState::None {
                return Err(TransitionError::CannotCancel);
            }
            n.state = BookingState::Cancelled;
            n.cancelled_by = Some(Role::Provider);
            n.cancel_reason = Some(reason.clone());
        }
        BookingEvent::Half { track, role } => {
            apply_half(&mut n, *track, *role);
        }
        BookingEvent::Tick => {}
    }
    finish_if_both_terminal(&mut n);
    let modified = n != *s;
    Ok(modified.then_some(n))
}

/// Verifies a booking-progress envelope as a record. Binding it to the
/// writer is the caller's job: it must hold the quote and compare
/// `issuer` with that quote's verdict `signer_did`.
pub fn verify_booking_progress(
    envelope: &str,
    now_secs: u64,
) -> RecordVerdict<BookingProgressPayload> {
    let opts = VerifyOptions::new(now_secs).allowing_expired();
    let verified = match record::verify_json(envelope, &opts) {
        Ok(v) => v,
        Err(e) => return RecordVerdict::refused(e.to_string()),
    };
    if verified.record_type != RECORD_BOOKING_PROGRESS
        || verified.version != BOOKING_PROGRESS_VERSION
    {
        return RecordVerdict::refused("not a booking-progress record this build understands");
    }
    let payload: BookingProgressPayload = match serde_json::from_value(verified.payload.clone()) {
        Ok(p) => p,
        Err(e) => return RecordVerdict::refused(format!("payload: {e}")),
    };
    if let Err(e) = payload.validate() {
        return RecordVerdict::refused(e.to_string());
    }
    if verified.subject != payload.agreement {
        return RecordVerdict::refused("envelope subject does not match agreement");
    }
    if verified.expires_at_secs.is_some() {
        return RecordVerdict::refused("a booking progress record may not declare an expiry");
    }
    RecordVerdict {
        verified: true,
        expired: false,
        reason: None,
        revocation_status: Some(listing::revocation_status_word(verified.revocation_status)),
        record_id: Some(verified.record_id),
        issuer: Some(verified.issuer),
        signer_did: Some(verified.signer_did),
        issued_at_secs: Some(verified.issued_at_secs),
        expires_at_secs: None,
        supersedes: verified.supersedes,
        payload: Some(payload),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NextStep {
    WaitForProvider,
    RequestPayment,
    PayOutsideRoym,
    ConfirmPaymentReceived,
    MarkWorkComplete,
    ConfirmWorkComplete,
    Nothing,
}

#[must_use]
pub fn next_step(s: &BookingProgressPayload, timing: PaymentTiming, me: Role) -> NextStep {
    if is_terminal(s.state) {
        return NextStep::Nothing;
    }
    match me {
        Role::Provider => {
            if s.payment == TrackState::None && timing == PaymentTiming::BeforeWork {
                NextStep::RequestPayment
            } else if s.payment == TrackState::Claimed {
                NextStep::ConfirmPaymentReceived
            } else if s.fulfilment == TrackState::None {
                NextStep::MarkWorkComplete
            } else {
                NextStep::Nothing
            }
        }
        Role::Consumer => {
            if s.payment == TrackState::None {
                if timing == PaymentTiming::BeforeWork || s.fulfilment != TrackState::None {
                    NextStep::PayOutsideRoym
                } else {
                    NextStep::WaitForProvider
                }
            } else if s.fulfilment == TrackState::Claimed {
                NextStep::ConfirmWorkComplete
            } else {
                NextStep::WaitForProvider
            }
        }
    }
}

#[cfg(test)]
mod tests;
