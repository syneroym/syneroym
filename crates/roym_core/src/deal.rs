//! Pure deal core types, records, and validation.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    extension::{self, ExtensionCheck, Registry},
    lang::{LangTag, TranslationError, check_text, check_translation_langs},
    listing, money,
    transaction::{DealRole, QuoteLocation, TimeWindow},
};

pub mod sd;
pub use sd::{SignedCommitment, SignedTerms};
pub mod transition;

#[cfg(test)]
mod tests;

pub const MAX_COMMITMENTS: usize = 16;
pub const MAX_DECLARED_VALUE_MINOR: i64 = 1_000_000_000_000_000; // 10^15
pub const MAX_COMMITMENT_DESCRIPTION_LEN: usize = 2048;
pub const MAX_AFTER: usize = 15;
pub const MAX_EXTENSION_ID_LEN: usize = 32;
pub const MAX_PAYEE_LEN: usize = 256;
pub const MAX_PAYMENT_METHODS: usize = 16;
pub const MAX_PAYMENT_METHOD_LEN: usize = 64;
pub const MAX_SCOPE_LEN: usize = 4096;
pub const MAX_TERMS_TEXT_LEN: usize = 2048;
pub const MAX_DETAILS_EXTRA: usize = 16;
pub const MAX_DETAILS_EXTRA_LABEL_LEN: usize = 64;
pub const MAX_DETAILS_EXTRA_VALUE_LEN: usize = 512;

/// A localized text string with its original language and author translations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalText {
    pub lang: LangTag,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub translations: Vec<TextTranslation>,
}

impl LocalText {
    #[must_use]
    pub fn plain(lang: LangTag, text: impl Into<String>) -> Self {
        Self { lang, text: text.into(), translations: Vec::new() }
    }

    /// Returns the text in the requested language if available, falling back to
    /// original.
    pub fn text_in(&self, lang: &LangTag) -> &str {
        if &self.lang == lang {
            return &self.text;
        }
        for t in &self.translations {
            if &t.lang == lang {
                return &t.text;
            }
        }
        &self.text
    }

    /// Validates the text and its translations.
    pub fn validate(&self, field: &'static str, max_len: usize) -> Result<(), TranslationError> {
        if self.text.trim().is_empty() {
            return Err(TranslationError::Empty { lang: self.lang.to_string(), field });
        }
        check_text(&self.lang, field, &self.text, max_len, true)?;
        check_translation_langs(&self.lang, self.translations.iter().map(|t| &t.lang))?;
        for t in &self.translations {
            check_text(&t.lang, field, &t.text, max_len, true)?;
        }
        Ok(())
    }
}

/// A translation of a text into another language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextTranslation {
    pub lang: LangTag,
    pub text: String,
}

/// The kind of resource exchanged in a commitment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceKind {
    ServiceInSlot,
    GoodsOrService,
    Currency,
}

/// Settlement behavior for a commitment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommitmentKind {
    Fulfilment,
    Currency,
}

/// Role of a party in a settlement record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SettlementRole {
    Giver,
    Receiver,
}

impl SettlementRole {
    /// The role's name on the wire, the same text serde writes. A record
    /// key built from a role uses this, so it cannot drift from the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Giver => "giver",
            Self::Receiver => "receiver",
        }
    }
}

/// Reference to a Roym extension and version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionRef {
    pub id: String,
    pub version: u32,
}

/// Extension-specific data attached to a commitment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Details {
    pub extension: String,
    pub schema: String,
    pub data: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<DetailsExtra>,
}

/// Label-value pair in a details block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetailsExtra {
    pub label: String,
    pub value: String,
}

/// One party giving one resource to the other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commitment {
    pub commitment_id: String,
    pub giver: DealRole,
    pub resource_kind: ResourceKind,
    pub kind: CommitmentKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension: Option<ExtensionRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
    pub description: LocalText,
    pub declared_value_minor: i64,
    #[serde(default)]
    pub details: Option<Details>,
    #[serde(default)]
    pub window: Option<TimeWindow>,
    #[serde(default)]
    pub location: Option<QuoteLocation>,
    #[serde(default)]
    pub payee: Option<String>,
    #[serde(default)]
    pub payment_methods: Option<Vec<String>>,
    #[serde(default)]
    pub tax_minor: Option<i64>,
    #[serde(default)]
    pub fees_minor: Option<i64>,
    #[serde(default)]
    pub eep: Option<serde_json::Value>,
}

/// Complete terms of a deal quote or agreement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DealTerms {
    pub currency: String,
    pub quote_expires_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_exchange: Option<String>,
    pub commitments: Vec<Commitment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listing_id: Option<String>,
    pub scope: LocalText,
    pub cancellation_terms: LocalText,
    pub refund_terms: LocalText,
    pub dispute_path: LocalText,
}

/// What enquiry or want intent a quote answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Answers {
    Enquiry { record_id: String },
    Want { intent_id: String, version: u32 },
}

/// Errors returned when validating deal terms.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DealError {
    #[error("commitment count must be between 1 and 16, got {0}")]
    CommitmentCount(usize),
    #[error("commitment at position {0} must have id {1}, got {2}")]
    CommitmentId(usize, String, String),
    #[error("commitment kind not supported: {0}")]
    CommitmentKindNotSupported(String),
    #[error("kind mismatch: resource {0:?} does not match commitment kind {1:?}")]
    KindMismatch(ResourceKind, CommitmentKind),
    #[error("declared value out of range: {0}")]
    DeclaredValueRange(i64),
    #[error("currency commitment value must be at least 1, got {0}")]
    CurrencyValueZero(i64),
    #[error("currency-only field present on non-currency commitment: {0}")]
    CurrencyOnlyField(&'static str),
    #[error("eep field must be null")]
    EepField,
    #[error("payee must not be empty")]
    PayeeEmpty,
    #[error("tax + fees ({0} + {1}) exceeds declared value {2}")]
    BreakdownExceedsValue(i64, i64, i64),
    #[error("after names unknown commitment id: {0}")]
    AfterUnknown(String),
    #[error("cycle detected in after: {0}")]
    AfterCycle(String),
    #[error("extension shape invalid: {0}")]
    ExtensionShape(String),
    #[error("details does not match extension")]
    DetailsMismatch,
    #[error("slot details invalid: {0}")]
    SlotDetails(String),
    #[error("extension not known to this node: {0}")]
    ExtensionUnknown(String),
    #[error("details do not fit their schema: {0}")]
    DetailsInvalid(String),
    #[error("home exchange not supported")]
    HomeExchangeNotSupported,
    #[error(transparent)]
    Text(#[from] TranslationError),
    #[error("all texts must share the same language: expected {expected}, got {actual}")]
    TextLangMismatch { expected: LangTag, actual: LangTag },
    #[error("all texts must share the same translation languages")]
    TranslationSetMismatch,
    #[error("unknown currency: {0}")]
    CurrencyUnknown(String),
    #[error("window invalid: {0}")]
    Window(String),
    #[error("location invalid: {0}")]
    Location(String),
    #[error("selective disclosure shape invalid: {0}")]
    SdShape(String),
}

impl DealError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::CommitmentCount(_) => "commitment-count",
            Self::CommitmentId(..) => "commitment-id",
            Self::CommitmentKindNotSupported(_) => "commitment-kind-not-supported",
            Self::KindMismatch(..) => "kind-mismatch",
            Self::DeclaredValueRange(_) => "declared-value-range",
            Self::CurrencyValueZero(_) => "currency-value-zero",
            Self::CurrencyOnlyField(_) => "currency-only-field",
            Self::EepField => "eep-field",
            Self::PayeeEmpty => "payee-empty",
            Self::BreakdownExceedsValue(..) => "breakdown-exceeds-value",
            Self::AfterUnknown(_) => "after-unknown",
            Self::AfterCycle(_) => "after-cycle",
            Self::ExtensionShape(_) => "extension-shape",
            Self::DetailsMismatch => "details-mismatch",
            Self::SlotDetails(_) => "slot-details",
            Self::ExtensionUnknown(_) => "extension-unknown",
            Self::DetailsInvalid(_) => "details-invalid",
            Self::HomeExchangeNotSupported => "home-exchange-not-supported",
            Self::Text(_) => "text",
            Self::TextLangMismatch { .. } => "text-lang-mismatch",
            Self::TranslationSetMismatch => "translation-set-mismatch",
            Self::CurrencyUnknown(_) => "currency-unknown",
            Self::Window(_) => "window",
            Self::Location(_) => "location",
            Self::SdShape(_) => "sd-shape",
        }
    }
}

/// Returns the commitment id for a 0-based position (`c1`..`c16`).
pub fn commitment_id(position: usize) -> String {
    format!("c{}", position + 1)
}

/// Verifies that all `LocalText` instances share the same original language
/// and translation language set.
pub fn check_record_text<'a>(
    mut texts: impl Iterator<Item = &'a LocalText>,
) -> Result<(), DealError> {
    let first = match texts.next() {
        Some(t) => t,
        None => return Ok(()),
    };
    check_translation_langs(&first.lang, first.translations.iter().map(|t| &t.lang))?;
    let mut expected_trans: Vec<&LangTag> = first.translations.iter().map(|t| &t.lang).collect();
    expected_trans.sort();

    for t in texts {
        if t.lang != first.lang {
            return Err(DealError::TextLangMismatch {
                expected: first.lang.clone(),
                actual: t.lang.clone(),
            });
        }
        check_translation_langs(&t.lang, t.translations.iter().map(|tr| &tr.lang))?;
        let mut trans: Vec<&LangTag> = t.translations.iter().map(|tr| &tr.lang).collect();
        trans.sort();
        if trans != expected_trans {
            return Err(DealError::TranslationSetMismatch);
        }
    }
    Ok(())
}

impl DealTerms {
    /// How the registry judges each commitment, in order. For the callers
    /// that must know which commitments are `NotChecked`.
    pub fn check_extensions(&self, registry: &Registry) -> Result<Vec<ExtensionCheck>, DealError> {
        self.commitments
            .iter()
            .map(|c| extension::check_commitment_extension(registry, c))
            .collect()
    }

    pub fn validate(&self) -> Result<(), DealError> {
        if self.commitments.is_empty() || self.commitments.len() > MAX_COMMITMENTS {
            return Err(DealError::CommitmentCount(self.commitments.len()));
        }
        if money::currency_minor_exponent(&self.currency).is_none() {
            return Err(DealError::CurrencyUnknown(self.currency.clone()));
        }
        if self.home_exchange.is_some() {
            return Err(DealError::HomeExchangeNotSupported);
        }

        for (i, c) in self.commitments.iter().enumerate() {
            let expected_id = commitment_id(i);
            if c.commitment_id != expected_id {
                return Err(DealError::CommitmentId(i, expected_id, c.commitment_id.clone()));
            }
            validate_commitment(c)?;
        }

        validate_after_graph(&self.commitments)?;
        validate_terms_texts(self)?;
        Ok(())
    }
}

fn validate_commitment(c: &Commitment) -> Result<(), DealError> {
    extension::check_commitment_extension(extension::registry(), c)?;
    validate_commitment_values(c)?;
    if c.eep.is_some() {
        return Err(DealError::EepField);
    }
    if let Some(ref w) = c.window {
        w.validate().map_err(|e| DealError::Window(e.to_string()))?;
    }
    if let Some(ref loc) = c.location {
        loc.validate().map_err(|e| DealError::Location(e.to_string()))?;
    }
    validate_local_text(&c.description, "description", MAX_COMMITMENT_DESCRIPTION_LEN)?;
    Ok(())
}

fn validate_commitment_values(c: &Commitment) -> Result<(), DealError> {
    if c.declared_value_minor < 0 || c.declared_value_minor > MAX_DECLARED_VALUE_MINOR {
        return Err(DealError::DeclaredValueRange(c.declared_value_minor));
    }
    if c.kind == CommitmentKind::Currency {
        if c.declared_value_minor < 1 {
            return Err(DealError::CurrencyValueZero(c.declared_value_minor));
        }
        let payee = c.payee.as_deref().ok_or(DealError::PayeeEmpty)?;
        if payee.trim().is_empty() || payee.len() > MAX_PAYEE_LEN {
            return Err(DealError::PayeeEmpty);
        }
        if let Some(ref methods) = c.payment_methods {
            if methods.len() > MAX_PAYMENT_METHODS {
                return Err(DealError::CurrencyOnlyField("too many payment methods"));
            }
            for m in methods {
                if !listing::valid_token(m, MAX_PAYMENT_METHOD_LEN) {
                    return Err(DealError::CurrencyOnlyField("invalid payment method token"));
                }
            }
        }
        let tax = c.tax_minor.unwrap_or(0);
        let fees = c.fees_minor.unwrap_or(0);
        if tax < 0 || fees < 0 || tax.saturating_add(fees) > c.declared_value_minor {
            return Err(DealError::BreakdownExceedsValue(tax, fees, c.declared_value_minor));
        }
    } else {
        if c.payee.is_some() {
            return Err(DealError::CurrencyOnlyField("payee"));
        }
        if c.payment_methods.is_some() {
            return Err(DealError::CurrencyOnlyField("payment_methods"));
        }
        if c.tax_minor.is_some() {
            return Err(DealError::CurrencyOnlyField("tax_minor"));
        }
        if c.fees_minor.is_some() {
            return Err(DealError::CurrencyOnlyField("fees_minor"));
        }
    }
    Ok(())
}

fn validate_after_graph(commitments: &[Commitment]) -> Result<(), DealError> {
    let entries: Vec<(&str, &[String])> =
        commitments.iter().map(|c| (c.commitment_id.as_str(), c.after.as_slice())).collect();
    validate_after_graph_entries(&entries)
}

pub fn validate_after_graph_entries(entries: &[(&str, &[String])]) -> Result<(), DealError> {
    let id_to_index: HashMap<&str, usize> =
        entries.iter().enumerate().map(|(i, (id, _))| (*id, i)).collect();

    for (c_id, after) in entries {
        if after.len() > MAX_AFTER {
            return Err(DealError::AfterUnknown(format!(
                "commitment {c_id} has more than {MAX_AFTER} after dependencies"
            )));
        }
        let mut seen = HashSet::with_capacity(after.len());
        for target in *after {
            if target == *c_id {
                return Err(DealError::AfterCycle(target.clone()));
            }
            if !id_to_index.contains_key(target.as_str()) {
                return Err(DealError::AfterUnknown(target.clone()));
            }
            if !seen.insert(target.as_str()) {
                return Err(DealError::AfterCycle(target.clone()));
            }
        }
    }

    // Cycle detection via DFS with 3 states: 0 = unvisited, 1 = visiting, 2 =
    // visited.
    let n = entries.len();
    let mut state = vec![0u8; n];
    for start in 0..n {
        if state[start] == 0 {
            detect_cycle_entries(start, entries, &id_to_index, &mut state)?;
        }
    }
    Ok(())
}

fn detect_cycle_entries(
    u: usize,
    entries: &[(&str, &[String])],
    id_to_index: &HashMap<&str, usize>,
    state: &mut [u8],
) -> Result<(), DealError> {
    state[u] = 1; // Visiting
    for target_id in entries[u].1 {
        let v = id_to_index[target_id.as_str()];
        if state[v] == 1 {
            return Err(DealError::AfterCycle(target_id.clone()));
        }
        if state[v] == 0 {
            detect_cycle_entries(v, entries, id_to_index, state)?;
        }
    }
    state[u] = 2; // Visited
    Ok(())
}

fn validate_local_text(
    lt: &LocalText,
    field: &'static str,
    max_len: usize,
) -> Result<(), DealError> {
    lt.validate(field, max_len).map_err(DealError::Text)
}

fn validate_terms_texts(terms: &DealTerms) -> Result<(), DealError> {
    validate_local_text(&terms.scope, "scope", MAX_SCOPE_LEN)?;
    validate_local_text(&terms.cancellation_terms, "cancellation_terms", MAX_TERMS_TEXT_LEN)?;
    validate_local_text(&terms.refund_terms, "refund_terms", MAX_TERMS_TEXT_LEN)?;
    validate_local_text(&terms.dispute_path, "dispute_path", MAX_TERMS_TEXT_LEN)?;

    let mut all_texts =
        vec![&terms.scope, &terms.cancellation_terms, &terms.refund_terms, &terms.dispute_path];
    for c in &terms.commitments {
        all_texts.push(&c.description);
    }
    check_record_text(all_texts.into_iter())
}
