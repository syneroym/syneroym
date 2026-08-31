#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! The signed record envelope: one stable byte encoding, one verifier, and
//! the rules the host applies before it signs anything.
//!
//! Compiles for the host and for `wasm32-wasip2`. Deliberately exposes no
//! function taking a `syneroym_identity::Identity`: producing a signature
//! needs a private key, and the only private key involved lives on the far
//! side of the `syneroym:signing` WIT boundary. A component linking this
//! crate can build a draft and verify an envelope; it cannot sign one.

pub mod envelope;
pub mod verify;

pub use envelope::{
    DraftError, ENVELOPE_VERSION, Envelope, EnvelopeError, MAX_PAYLOAD_BYTES, MAX_PAYLOAD_DEPTH,
    MAX_RECORD_TYPE_LEN, MAX_SUBJECT_LEN, RECORD_ID_PREFIX, RecordDraft,
};
pub use syneroym_identity::delegation::SCOPE_RECORD_SIGNING;
pub use verify::{
    DEFAULT_ACCEPTED_SCOPES, EmptyRevocations, RevocationCheck, RevocationSet, RevocationSource,
    RevocationStatus, VerifiedRecord, VerifyError, VerifyOptions, verify, verify_json,
};
