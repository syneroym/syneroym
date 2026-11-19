//! Types and errors for presentation bundles and verification views.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{
    deal::{
        CommitmentKind, ExtensionRef, ResourceKind, SettlementRole, transition::CommitmentState,
    },
    disclosure::SdError,
    transaction::DealRole,
};

pub const PRESENTATION_VERSION: u32 = 1;
pub const MAX_PRESENTATION_RECORDS: usize = 64;

/// A bundle of signed deal records and selected disclosures shown to a third
/// party.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presentation {
    pub presentation_version: u32,
    pub records: Vec<String>,
    pub disclosures: Vec<String>,
}

/// Identifiers and fixed properties of the presented deal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DealHeader {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_id: Option<String>,
    pub agreement: String,
    pub quote_author_did: String,
    pub counterparty_did: String,
    pub currency: String,
    pub quote_expires_at_secs: u64,
    #[serde(default)]
    pub quote_checked: bool,
}

/// Verification verdict for a single record within the presentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordCheck {
    pub record_type: String,
    pub record_id: String,
    pub issuer: String,
    pub issued_at_secs: u64,
    pub verified: bool,
}

/// Visible outcome and state of a single commitment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitmentOutcome {
    pub id: String,
    pub giver: DealRole,
    pub resource_kind: ResourceKind,
    pub kind: CommitmentKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension: Option<ExtensionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<CommitmentState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed: Option<bool>,
}

/// Disclosed and hidden fields of a settlement record in the presentation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettlementView {
    pub record_id: String,
    pub commitment_id: String,
    pub role: SettlementRole,
    pub shown: Value,
    pub hidden: Vec<String>,
}

/// Status of the snapshot writer verification against deal signing keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WriterCheck {
    Checked,
    NotChecked,
    NoSnapshot,
}

/// The verified outcome of a presentation shown to a third party.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresentationView {
    pub deal: DealHeader,
    pub records: Vec<RecordCheck>,
    pub terms: Value,
    pub hidden: Vec<String>,
    pub commitments: Vec<CommitmentOutcome>,
    pub settlement: Vec<SettlementView>,
    pub writer: WriterCheck,
    pub not_checked: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PresentationError {
    #[error("presentation version must be 1, found {0}")]
    UnsupportedVersion(u32),
    #[error("presentation must contain between 1 and 64 records, found {0}")]
    RecordCount(usize),
    #[error("record failed verification: {0}")]
    RecordRefused(String),
    #[error("unsupported record type in presentation: {0}")]
    UnsupportedRecordType(String),
    #[error("records do not belong to the same deal: {0}")]
    MultipleDeals(String),
    #[error("multiple records of the same type: {0}")]
    DuplicateRecord(String),
    #[error("agreement receipt halves disagree")]
    HalvesDisagree,
    #[error("quote and receipt half terms do not match")]
    TermsMismatch,
    #[error("presentation parties do not match quote or deal")]
    PartyMismatch,
    #[error("commitment '{0}' does not match deal terms")]
    CommitmentMismatch(String),
    #[error("presentation must include a quote or an agreement receipt")]
    MissingTerms,
    #[error("selective disclosure error: {0}")]
    Sd(#[from] SdError),
    #[error("field '{0}' invalid: {1}")]
    FieldInvalid(String, String),
}

impl PresentationError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedVersion(_) => "unsupported-version",
            Self::RecordCount(_) => "record-count",
            Self::RecordRefused(_) => "record-refused",
            Self::UnsupportedRecordType(_) => "unsupported-record-type",
            Self::MultipleDeals(_) => "multiple-deals",
            Self::DuplicateRecord(_) => "duplicate-record",
            Self::HalvesDisagree => "halves-disagree",
            Self::TermsMismatch => "terms-mismatch",
            Self::PartyMismatch => "party-mismatch",
            Self::CommitmentMismatch(_) => "commitment-mismatch",
            Self::MissingTerms => "missing-terms",
            Self::Sd(e) => e.code(),
            Self::FieldInvalid(_, _) => "field-invalid",
        }
    }
}
