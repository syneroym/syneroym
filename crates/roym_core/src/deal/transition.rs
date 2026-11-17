//! Pure commitment state transition functions and track window calculations.

use serde::{Deserialize, Serialize};

use crate::{deal::CommitmentKind, transaction::TimeWindow};

#[cfg(test)]
mod tests;

pub const TRACK_WINDOW_SECS: u64 = 30 * 24 * 60 * 60; // 30 days

/// States a commitment can take in the deal core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommitmentState {
    Promised,
    PaymentRequested,
    Claimed,
    Fulfilled,
    Acknowledged,
    EndedUnconfirmed,
    Failed,
    Cancelled,
}

/// Events that drive commitment transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitmentEvent {
    GiverHalf,
    ReceiverHalf,
    PaymentRequest,
    Tick,
    Cancel,
    Failure,
    OtherFailed,
}

/// The result of applying an event to a commitment state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    To { state: CommitmentState, reason: Option<&'static str>, claimed: Option<bool> },
    NoChange,
    Refused,
}

/// Returns whether `state` is a final state that cannot transition further.
pub fn is_final(state: CommitmentState) -> bool {
    matches!(
        state,
        CommitmentState::Fulfilled
            | CommitmentState::Acknowledged
            | CommitmentState::EndedUnconfirmed
            | CommitmentState::Failed
            | CommitmentState::Cancelled
    )
}

/// Computes the track end timestamp for a commitment.
pub fn track_end(window: Option<&TimeWindow>, agreed_at_secs: u64) -> u64 {
    window
        .map_or(agreed_at_secs, |w| w.latest_secs.max(agreed_at_secs))
        .saturating_add(TRACK_WINDOW_SECS)
}

/// Applies an event to a commitment's state according to the kind's transition
/// table.
pub fn step(kind: CommitmentKind, state: CommitmentState, event: CommitmentEvent) -> Step {
    match kind {
        CommitmentKind::Fulfilment => step_fulfilment(state, event),
        CommitmentKind::Currency => step_currency(state, event),
    }
}

fn step_fulfilment(state: CommitmentState, event: CommitmentEvent) -> Step {
    if is_final(state) {
        return step_fulfilment_final(event);
    }
    match state {
        CommitmentState::Promised => step_fulfilment_promised(event),
        CommitmentState::Claimed => step_fulfilment_claimed(event),
        _ => Step::Refused,
    }
}

fn step_fulfilment_final(event: CommitmentEvent) -> Step {
    match event {
        CommitmentEvent::GiverHalf
        | CommitmentEvent::ReceiverHalf
        | CommitmentEvent::Tick
        | CommitmentEvent::OtherFailed => Step::NoChange,
        CommitmentEvent::Cancel | CommitmentEvent::Failure | CommitmentEvent::PaymentRequest => {
            Step::Refused
        }
    }
}

fn step_fulfilment_promised(event: CommitmentEvent) -> Step {
    match event {
        CommitmentEvent::GiverHalf => {
            Step::To { state: CommitmentState::Claimed, reason: None, claimed: None }
        }
        CommitmentEvent::ReceiverHalf => {
            Step::To { state: CommitmentState::Fulfilled, reason: None, claimed: None }
        }
        CommitmentEvent::Tick => Step::To {
            state: CommitmentState::EndedUnconfirmed,
            reason: None,
            claimed: Some(false),
        },
        CommitmentEvent::Cancel => {
            Step::To { state: CommitmentState::Cancelled, reason: None, claimed: None }
        }
        CommitmentEvent::Failure => Step::To {
            state: CommitmentState::Failed,
            reason: Some("reservation-lost"),
            claimed: None,
        },
        CommitmentEvent::OtherFailed => Step::To {
            state: CommitmentState::Cancelled,
            reason: Some("other-commitment-failed"),
            claimed: None,
        },
        CommitmentEvent::PaymentRequest => Step::Refused,
    }
}

fn step_fulfilment_claimed(event: CommitmentEvent) -> Step {
    match event {
        CommitmentEvent::GiverHalf | CommitmentEvent::OtherFailed => Step::NoChange,
        CommitmentEvent::ReceiverHalf => {
            Step::To { state: CommitmentState::Fulfilled, reason: None, claimed: None }
        }
        CommitmentEvent::Tick => {
            Step::To { state: CommitmentState::EndedUnconfirmed, reason: None, claimed: Some(true) }
        }
        CommitmentEvent::Cancel | CommitmentEvent::Failure | CommitmentEvent::PaymentRequest => {
            Step::Refused
        }
    }
}

fn step_currency(state: CommitmentState, event: CommitmentEvent) -> Step {
    if is_final(state) {
        return step_currency_final(event);
    }
    match state {
        CommitmentState::Promised => step_currency_promised(event),
        CommitmentState::PaymentRequested => step_currency_payment_requested(event),
        CommitmentState::Claimed => step_currency_claimed(event),
        _ => Step::Refused,
    }
}

fn step_currency_final(event: CommitmentEvent) -> Step {
    match event {
        CommitmentEvent::PaymentRequest
        | CommitmentEvent::GiverHalf
        | CommitmentEvent::ReceiverHalf
        | CommitmentEvent::Tick
        | CommitmentEvent::OtherFailed => Step::NoChange,
        CommitmentEvent::Cancel | CommitmentEvent::Failure => Step::Refused,
    }
}

fn step_currency_promised(event: CommitmentEvent) -> Step {
    match event {
        CommitmentEvent::PaymentRequest => {
            Step::To { state: CommitmentState::PaymentRequested, reason: None, claimed: None }
        }
        CommitmentEvent::GiverHalf => {
            Step::To { state: CommitmentState::Claimed, reason: None, claimed: None }
        }
        CommitmentEvent::ReceiverHalf => {
            Step::To { state: CommitmentState::Acknowledged, reason: None, claimed: None }
        }
        CommitmentEvent::Tick => Step::To {
            state: CommitmentState::EndedUnconfirmed,
            reason: None,
            claimed: Some(false),
        },
        CommitmentEvent::Cancel => {
            Step::To { state: CommitmentState::Cancelled, reason: None, claimed: None }
        }
        CommitmentEvent::OtherFailed => Step::To {
            state: CommitmentState::Cancelled,
            reason: Some("other-commitment-failed"),
            claimed: None,
        },
        CommitmentEvent::Failure => Step::Refused,
    }
}

fn step_currency_payment_requested(event: CommitmentEvent) -> Step {
    match event {
        CommitmentEvent::PaymentRequest => Step::NoChange,
        CommitmentEvent::GiverHalf => {
            Step::To { state: CommitmentState::Claimed, reason: None, claimed: None }
        }
        CommitmentEvent::ReceiverHalf => {
            Step::To { state: CommitmentState::Acknowledged, reason: None, claimed: None }
        }
        CommitmentEvent::Tick => Step::To {
            state: CommitmentState::EndedUnconfirmed,
            reason: None,
            claimed: Some(false),
        },
        CommitmentEvent::Cancel => {
            Step::To { state: CommitmentState::Cancelled, reason: None, claimed: None }
        }
        CommitmentEvent::OtherFailed => Step::To {
            state: CommitmentState::Cancelled,
            reason: Some("other-commitment-failed"),
            claimed: None,
        },
        CommitmentEvent::Failure => Step::Refused,
    }
}

fn step_currency_claimed(event: CommitmentEvent) -> Step {
    match event {
        CommitmentEvent::PaymentRequest
        | CommitmentEvent::GiverHalf
        | CommitmentEvent::OtherFailed => Step::NoChange,
        CommitmentEvent::ReceiverHalf => {
            Step::To { state: CommitmentState::Acknowledged, reason: None, claimed: None }
        }
        CommitmentEvent::Tick => {
            Step::To { state: CommitmentState::EndedUnconfirmed, reason: None, claimed: Some(true) }
        }
        CommitmentEvent::Cancel | CommitmentEvent::Failure => Step::Refused,
    }
}
