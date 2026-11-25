//! Agreement progress snapshots signed by the quote author's transaction
//! service. Pure: no host calls.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    deal::{
        CommitmentKind, DealTerms, SettlementRole,
        transition::{CommitmentEvent, CommitmentState, Step, is_final, step, track_end},
    },
    listing, person,
    record::{self, VerifyOptions},
    verdict::RecordVerdict,
};

#[cfg(test)]
mod tests;

pub const AGREEMENT_PROGRESS_VERSION: u32 = 1;
pub const MAX_DEFER_SECS: u64 = 7 * 24 * 3600;
pub use crate::record::RECORD_AGREEMENT_PROGRESS;

pub const PAYMENT_NOTICE: &str = "This records what each side says about the payment. Roym does \
                                  not see the money move and cannot confirm that it did.";
pub const PROGRESS_NOTICE: &str = "This status comes from the system of the person who wrote the \
                                   quote. It is not a signed statement by either person.";
pub const PAYMENT_CLAIMED: &str = "The payer says they paid.";
pub const PAYMENT_ACKNOWLEDGED: &str = "The payee confirms they received the payment.";
pub const FULFILMENT_CLAIMED: &str = "The person doing the work says it is done.";
pub const FULFILMENT_ACKNOWLEDGED: &str = "The person receiving the work confirms it is done.";
pub const TRACK_UNCONFIRMED: &str = "No confirmation was recorded before the window closed.";
pub const ONE_PAYMENT_NOTICE: &str =
    "Each payment in this quote is paid once. A deposit is its own payment.";

/// High-level lifecycle states of an agreement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgreementState {
    Agreed,
    Closed,
    Cancelled,
    NotFormed,
}

impl AgreementState {
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Closed | Self::Cancelled | Self::NotFormed)
    }
}

/// Standard cancellation reason codes for an agreement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CancelReason {
    CannotDeliver,
    ScheduleConflict,
    CounterpartyAsked,
    Other,
}

/// Per-commitment progress within an agreement snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitmentProgress {
    pub commitment_id: String,
    pub state: CommitmentState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub halves: Vec<String>,
}

/// How the writer answered the enquiry that led to this agreement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormedAs {
    Quote,
    AsListedInstant,
    AsListedConfirmed,
}

/// One agreement progress snapshot signed as `agreement-progress`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgreementProgressPayload {
    pub agreement: String,
    pub conversation: String,
    pub counterparty_did: String,
    pub quote_author_did: String,
    pub seq: u32,
    pub state: AgreementState,
    /// How the deal formed. The same in every snapshot of one agreement.
    pub formed_as: FormedAs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_reason: Option<CancelReason>,
    pub commitments: Vec<CommitmentProgress>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransitionError {
    #[error("agreement is {0:?}")]
    Terminal(AgreementState),
    #[error("cannot cancel agreement once a commitment has moved")]
    CannotCancel,
    #[error("commitment '{0}' not found in deal terms")]
    CommitmentNotFound(String),
    #[error("transition refused for commitment '{0}'")]
    Refused(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProgressPayloadError {
    #[error("agreement '{0}' must start with 'rec_'")]
    InvalidAgreement(String),
    #[error("counterparty_did '{0}' is not a valid did:key")]
    InvalidCounterpartyDid(String),
    #[error("quote_author_did '{0}' is not a valid did:key")]
    InvalidQuoteAuthorDid(String),
    #[error("counterparty and quote author cannot be the same DID")]
    SamePartyDid,
    #[error("conversation is empty")]
    ConversationEmpty,
    #[error("seq must be >= 1, got {0}")]
    InvalidSeq(u32),
    #[error("commitments list cannot be empty")]
    EmptyCommitments,
    #[error("state_reason must be present if and only if state is not-formed")]
    NotFormedStateMismatch,
    #[error("cancel_reason must be present if and only if state is cancelled")]
    CancelledStateMismatch,
}

impl AgreementProgressPayload {
    pub fn validate(&self) -> Result<(), ProgressPayloadError> {
        if !self.agreement.starts_with("rec_") {
            return Err(ProgressPayloadError::InvalidAgreement(self.agreement.clone()));
        }
        if !person::is_did_key(&self.counterparty_did) {
            return Err(ProgressPayloadError::InvalidCounterpartyDid(
                self.counterparty_did.clone(),
            ));
        }
        if !person::is_did_key(&self.quote_author_did) {
            return Err(ProgressPayloadError::InvalidQuoteAuthorDid(self.quote_author_did.clone()));
        }
        if self.counterparty_did == self.quote_author_did {
            return Err(ProgressPayloadError::SamePartyDid);
        }
        if self.conversation.is_empty() {
            return Err(ProgressPayloadError::ConversationEmpty);
        }
        if self.seq < 1 {
            return Err(ProgressPayloadError::InvalidSeq(self.seq));
        }
        if self.commitments.is_empty() {
            return Err(ProgressPayloadError::EmptyCommitments);
        }
        let not_formed = self.state == AgreementState::NotFormed;
        if self.state_reason.is_some() != not_formed {
            return Err(ProgressPayloadError::NotFormedStateMismatch);
        }
        let cancelled = self.state == AgreementState::Cancelled;
        if self.cancel_reason.is_some() != cancelled {
            return Err(ProgressPayloadError::CancelledStateMismatch);
        }
        Ok(())
    }
}

/// What the writer is asked to apply to an agreement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgreementEvent {
    Cancel { reason: CancelReason },
    Half { commitment_id: String, role: SettlementRole, record_id: String },
    PaymentRequest { commitment_id: String },
    Tick { now_secs: u64 },
    Failure { commitment_id: String, reason: String },
}

/// The first snapshot, seq 1: `agreed`, or `not-formed` with its reason.
pub fn open(
    agreement: String,
    conversation: String,
    counterparty_did: String,
    quote_author_did: String,
    terms: &DealTerms,
    formed_as: FormedAs,
    not_formed_reason: Option<String>,
) -> AgreementProgressPayload {
    let state = if not_formed_reason.is_some() {
        AgreementState::NotFormed
    } else {
        AgreementState::Agreed
    };
    let commitments = terms
        .commitments
        .iter()
        .map(|c| CommitmentProgress {
            commitment_id: c.commitment_id.clone(),
            state: CommitmentState::Promised,
            reason: None,
            claimed: None,
            halves: Vec::new(),
        })
        .collect();
    AgreementProgressPayload {
        agreement,
        conversation,
        counterparty_did,
        quote_author_did,
        seq: 1,
        state,
        formed_as,
        state_reason: not_formed_reason,
        cancel_reason: None,
        commitments,
    }
}

/// Applies an event to the agreement progress snapshot.
///
/// Returns `Ok(None)` for idempotent repeated events that cause no change.
pub fn apply(
    s: &AgreementProgressPayload,
    terms: &DealTerms,
    agreed_at_secs: u64,
    e: &AgreementEvent,
) -> Result<Option<AgreementProgressPayload>, TransitionError> {
    if s.state.is_terminal() {
        return match e {
            AgreementEvent::Tick { .. } => Ok(None),
            AgreementEvent::Cancel { .. } if s.state == AgreementState::Cancelled => Ok(None),
            _ => Err(TransitionError::Terminal(s.state)),
        };
    }
    let mut n = s.clone();
    match e {
        AgreementEvent::Cancel { reason } => apply_cancel(&mut n, terms, *reason)?,
        AgreementEvent::PaymentRequest { commitment_id } => {
            apply_payment_request(&mut n, terms, commitment_id)?;
        }
        AgreementEvent::Half { commitment_id, role, record_id } => {
            apply_half(&mut n, terms, commitment_id, *role, record_id)?;
        }
        AgreementEvent::Failure { commitment_id, reason } => {
            apply_failure(&mut n, terms, commitment_id, reason)?;
        }
        AgreementEvent::Tick { now_secs } => {
            apply_tick(&mut n, terms, agreed_at_secs, *now_secs)?;
        }
    }
    if n.state == AgreementState::Agreed && n.commitments.iter().all(|c| is_final(c.state)) {
        n.state = AgreementState::Closed;
    }
    let modified = n != *s;
    Ok(modified.then_some(n))
}

fn apply_cancel(
    n: &mut AgreementProgressPayload,
    terms: &DealTerms,
    reason: CancelReason,
) -> Result<(), TransitionError> {
    let can_cancel = n
        .commitments
        .iter()
        .all(|c| matches!(c.state, CommitmentState::Promised | CommitmentState::PaymentRequested));
    if !can_cancel {
        return Err(TransitionError::CannotCancel);
    }
    for c in &mut n.commitments {
        let kind = find_kind(terms, &c.commitment_id)?;
        if let Step::To { state, reason: r, claimed } = step(kind, c.state, CommitmentEvent::Cancel)
        {
            c.state = state;
            c.reason = r.map(ToString::to_string);
            c.claimed = claimed;
        }
    }
    n.state = AgreementState::Cancelled;
    n.cancel_reason = Some(reason);
    Ok(())
}

fn apply_payment_request(
    n: &mut AgreementProgressPayload,
    terms: &DealTerms,
    commitment_id: &str,
) -> Result<(), TransitionError> {
    let c = find_progress_mut(&mut n.commitments, commitment_id)?;
    let kind = find_kind(terms, commitment_id)?;
    match step(kind, c.state, CommitmentEvent::PaymentRequest) {
        Step::To { state, reason, claimed } => {
            c.state = state;
            c.reason = reason.map(ToString::to_string);
            c.claimed = claimed;
        }
        Step::NoChange => {}
        Step::Refused => return Err(TransitionError::Refused(commitment_id.to_string())),
    }
    Ok(())
}

fn apply_half(
    n: &mut AgreementProgressPayload,
    terms: &DealTerms,
    commitment_id: &str,
    role: SettlementRole,
    record_id: &str,
) -> Result<(), TransitionError> {
    let c = find_progress_mut(&mut n.commitments, commitment_id)?;
    let kind = find_kind(terms, commitment_id)?;
    let half_ev = match role {
        SettlementRole::Giver => CommitmentEvent::GiverHalf,
        SettlementRole::Receiver => CommitmentEvent::ReceiverHalf,
    };
    match step(kind, c.state, half_ev) {
        Step::To { state, reason, claimed } => {
            c.state = state;
            c.reason = reason.map(ToString::to_string);
            c.claimed = claimed;
        }
        Step::NoChange => {}
        Step::Refused => return Err(TransitionError::Refused(commitment_id.to_string())),
    }
    if !c.halves.iter().any(|h| h == record_id) {
        c.halves.push(record_id.to_string());
    }
    Ok(())
}

fn apply_failure(
    n: &mut AgreementProgressPayload,
    terms: &DealTerms,
    commitment_id: &str,
    reason: &str,
) -> Result<(), TransitionError> {
    let c = find_progress_mut(&mut n.commitments, commitment_id)?;
    let kind = find_kind(terms, commitment_id)?;
    match step(kind, c.state, CommitmentEvent::Failure) {
        Step::To { state, claimed, .. } => {
            c.state = state;
            c.reason = Some(reason.to_string());
            c.claimed = claimed;
        }
        Step::NoChange => {}
        Step::Refused => return Err(TransitionError::Refused(commitment_id.to_string())),
    }
    for other in &mut n.commitments {
        if other.commitment_id == commitment_id {
            continue;
        }
        if matches!(other.state, CommitmentState::Promised | CommitmentState::PaymentRequested) {
            let other_kind = find_kind(terms, &other.commitment_id)?;
            if let Step::To { state, reason: r, claimed } =
                step(other_kind, other.state, CommitmentEvent::OtherFailed)
            {
                other.state = state;
                other.reason = r.map(ToString::to_string);
                other.claimed = claimed;
            }
        }
    }
    Ok(())
}

fn apply_tick(
    n: &mut AgreementProgressPayload,
    terms: &DealTerms,
    agreed_at_secs: u64,
    now_secs: u64,
) -> Result<(), TransitionError> {
    for c in &mut n.commitments {
        let term = terms
            .commitments
            .iter()
            .find(|t| t.commitment_id == c.commitment_id)
            .ok_or_else(|| TransitionError::CommitmentNotFound(c.commitment_id.clone()))?;
        let end = track_end(term.window.as_ref(), agreed_at_secs);
        if now_secs >= end {
            let kind = term.kind;
            if let Step::To { state, reason, claimed } = step(kind, c.state, CommitmentEvent::Tick)
            {
                c.state = state;
                c.reason = reason.map(ToString::to_string);
                c.claimed = claimed;
            }
        }
    }
    Ok(())
}

fn find_kind(terms: &DealTerms, commitment_id: &str) -> Result<CommitmentKind, TransitionError> {
    terms
        .commitments
        .iter()
        .find(|c| c.commitment_id == commitment_id)
        .map(|c| c.kind)
        .ok_or_else(|| TransitionError::CommitmentNotFound(commitment_id.to_string()))
}

fn find_progress_mut<'a>(
    commitments: &'a mut [CommitmentProgress],
    commitment_id: &str,
) -> Result<&'a mut CommitmentProgress, TransitionError> {
    commitments
        .iter_mut()
        .find(|c| c.commitment_id == commitment_id)
        .ok_or_else(|| TransitionError::CommitmentNotFound(commitment_id.to_string()))
}

/// Verifies an agreement-progress envelope as a record.
pub fn verify_agreement_progress(
    envelope: &str,
    now_secs: u64,
) -> RecordVerdict<AgreementProgressPayload> {
    let opts = VerifyOptions::new(now_secs).allowing_expired();
    let verified = match record::verify_json(envelope, &opts) {
        Ok(v) => v,
        Err(e) => return RecordVerdict::refused(e.to_string()),
    };
    if verified.record_type != RECORD_AGREEMENT_PROGRESS
        || verified.version != AGREEMENT_PROGRESS_VERSION
    {
        return RecordVerdict::refused("not an agreement-progress record this build understands");
    }
    let payload: AgreementProgressPayload = match serde_json::from_value(verified.payload.clone()) {
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
        return RecordVerdict::refused("an agreement progress record may not declare an expiry");
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
