//! Presentation format and verification for selective disclosure to third
//! parties.

pub(crate) mod settlement;
pub mod types;
pub(crate) mod validate;
pub mod verify;

#[cfg(test)]
mod tests;

pub use types::{
    CommitmentOutcome, DealHeader, MAX_PRESENTATION_RECORDS, PRESENTATION_VERSION, Presentation,
    PresentationError, PresentationView, RecordCheck, SettlementView, WriterCheck,
};
pub use verify::verify_presentation;
