//! Step records: one signed work step of an extension, inside a formed
//! agreement.
//!
//! A step never changes a deal state. It says that a party did a piece of
//! work that the extension's manifest declares, for one commitment. The
//! step's `data` is hidden like a commitment's `details`: the signed record
//! holds one digest, and the card carries the disclosure.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{
    deal::{ExtensionRef, MAX_EXTENSION_ID_LEN, SettlementRole},
    disclosure::check_sd_shape,
    extension::{MAX_STEP_TYPE_LEN, Registry, StepBy},
    listing, person,
    record::RECORD_STEP,
    verdict::{self, PartyRecordPayload, RecordVerdict},
};

pub const STEP_VERSION: u32 = 1;

/// Longest serialized `data` of one step. It is the same as the limit of a
/// commitment's `details` disclosure. For a step of an extension that this
/// node does not know, it is the only bound on `data`.
pub const MAX_STEP_DATA_BYTES: usize = 8 * 1024;

/// Most step rows one agreement may hold, of both parties together. Without
/// it, one party could file steps of an unknown extension until the
/// conversation's card cap is used up.
pub const MAX_STEPS_PER_AGREEMENT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StepError {
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
    #[error("commitment_id is empty")]
    CommitmentIdEmpty,
    #[error("extension '{0}' is not a valid extension reference")]
    InvalidExtension(String),
    #[error(
        "step_type '{0}' must be '<extension id>/<name>' and at most {MAX_STEP_TYPE_LEN} bytes"
    )]
    InvalidStepType(String),
    #[error("selective disclosure shape invalid: {0}")]
    SdShape(String),
    #[error("step data must be a JSON object")]
    DataNotObject,
    #[error("step data is {bytes} bytes, over the {MAX_STEP_DATA_BYTES}-byte limit")]
    DataTooLarge { bytes: usize },
    #[error("step type '{0}' is not declared by the extension")]
    StepTypeUnknown(String),
    #[error("step type '{0}' is not recorded by this party of the commitment")]
    NotYourStep(String),
    #[error("step data: {0}")]
    DataInvalid(String),
    #[error("an agreement holds at most {MAX_STEPS_PER_AGREEMENT} steps")]
    TooManySteps,
}

impl StepError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidAgreement(_) => "invalid-agreement",
            Self::InvalidCounterpartyDid(_) => "invalid-counterparty-did",
            Self::InvalidQuoteAuthorDid(_) => "invalid-quote-author-did",
            Self::SamePartyDid => "same-party-did",
            Self::ConversationEmpty => "conversation-empty",
            Self::CommitmentIdEmpty => "commitment-id-empty",
            Self::InvalidExtension(_) => "invalid-extension",
            Self::InvalidStepType(_) => "invalid-step-type",
            Self::SdShape(_) => "sd-shape",
            Self::DataNotObject => "details-invalid",
            Self::DataTooLarge { .. } => "step-data-too-large",
            Self::StepTypeUnknown(_) => "step-type-unknown",
            Self::NotYourStep(_) => "not-your-step",
            Self::DataInvalid(_) => "details-invalid",
            Self::TooManySteps => "too-many-steps",
        }
    }
}

/// One party's record of a work step of one commitment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepPayload {
    /// The agreed quote version, `rec_…`.
    pub agreement: String,
    pub conversation: String,
    pub commitment_id: String,
    pub counterparty_did: String,
    pub quote_author_did: String,
    /// The signer's role in this commitment.
    pub role: SettlementRole,
    pub extension: ExtensionRef,
    /// `<extension id>/<name>`.
    pub step_type: String,
    #[serde(rename = "_sd")]
    pub sd: Vec<String>,
}

impl StepPayload {
    pub fn validate(&self) -> Result<(), StepError> {
        if !self.agreement.starts_with("rec_") {
            return Err(StepError::InvalidAgreement(self.agreement.clone()));
        }
        if !person::is_did_key(&self.counterparty_did) {
            return Err(StepError::InvalidCounterpartyDid(self.counterparty_did.clone()));
        }
        if !person::is_did_key(&self.quote_author_did) {
            return Err(StepError::InvalidQuoteAuthorDid(self.quote_author_did.clone()));
        }
        if self.counterparty_did == self.quote_author_did {
            return Err(StepError::SamePartyDid);
        }
        if self.conversation.is_empty() {
            return Err(StepError::ConversationEmpty);
        }
        if self.commitment_id.is_empty() {
            return Err(StepError::CommitmentIdEmpty);
        }
        if !listing::valid_token(&self.extension.id, MAX_EXTENSION_ID_LEN)
            || self.extension.version == 0
        {
            return Err(StepError::InvalidExtension(self.extension.id.clone()));
        }
        let prefix_ok = self
            .step_type
            .strip_prefix(&self.extension.id)
            .and_then(|rest| rest.strip_prefix('/'))
            .is_some_and(|name| !name.is_empty());
        if !prefix_ok || self.step_type.len() > MAX_STEP_TYPE_LEN {
            return Err(StepError::InvalidStepType(self.step_type.clone()));
        }
        check_sd_shape(&self.sd, 1, "step").map_err(StepError::SdShape)?;
        Ok(())
    }
}

/// The hidden field of a step record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepHidden {
    pub data: Value,
}

impl StepHidden {
    /// The shape and size of `data`. The extension's field list is checked
    /// by the caller, because only a node that knows the extension has it.
    pub fn validate(&self) -> Result<(), StepError> {
        check_data_size(&self.data)
    }
}

/// `data` must be a JSON object of at most `MAX_STEP_DATA_BYTES`.
pub fn check_data_size(data: &Value) -> Result<(), StepError> {
    if !data.is_object() {
        return Err(StepError::DataNotObject);
    }
    let bytes = serde_json::to_vec(data).map(|v| v.len()).unwrap_or(usize::MAX);
    if bytes > MAX_STEP_DATA_BYTES {
        return Err(StepError::DataTooLarge { bytes });
    }
    Ok(())
}

/// What the registry could say about a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepCheck {
    /// Known extension: the step type, who records it and its fields pass.
    Checked,
    /// The extension is not a workflow extension in this registry. Only the
    /// size of `data` was checked.
    NotChecked,
}

/// The one rule set for a step: `step.add`, filing a received card and
/// importing a backup all call it. `role` is the signer's role in the
/// commitment.
pub fn check_step(
    registry: &Registry,
    ext: &ExtensionRef,
    step_type: &str,
    role: SettlementRole,
    data: &Value,
) -> Result<StepCheck, StepError> {
    check_data_size(data)?;
    if registry.workflow(ext).is_none() {
        return Ok(StepCheck::NotChecked);
    }
    let declared = registry
        .step_type(ext, step_type)
        .ok_or_else(|| StepError::StepTypeUnknown(step_type.to_string()))?;
    let by_ok = match declared.by {
        StepBy::Giver => role == SettlementRole::Giver,
        StepBy::Receiver => role == SettlementRole::Receiver,
        StepBy::Either => true,
    };
    if !by_ok {
        return Err(StepError::NotYourStep(step_type.to_string()));
    }
    declared.check(data).map_err(|e| StepError::DataInvalid(e.0))?;
    Ok(StepCheck::Checked)
}

impl PartyRecordPayload for StepPayload {
    type Error = StepError;

    fn validate(&self) -> Result<(), Self::Error> {
        self.validate()
    }

    fn agreement(&self) -> &str {
        &self.agreement
    }

    fn quote_author_did(&self) -> &str {
        &self.quote_author_did
    }

    fn counterparty_did(&self) -> &str {
        &self.counterparty_did
    }
}

pub fn verify_step(envelope: &str, now_secs: u64) -> RecordVerdict<StepPayload> {
    verdict::verify_party_record(envelope, now_secs, RECORD_STEP, STEP_VERSION, "step", "step")
}

#[cfg(test)]
mod tests;
