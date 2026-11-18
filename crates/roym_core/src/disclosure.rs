//! Selective disclosure rules, hidden field sets, and indices for Roym.

use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use syneroym_signed_record::{
    DIGEST_CHARS, DecodedDisclosure, DisclosureError, Salt, decode_base64url, decode_disclosure,
    disclosure_digest, encode_disclosure,
};
use thiserror::Error;

#[cfg(test)]
mod tests;

pub const MAX_RECORD_DISCLOSURE_BYTES: usize = 96 * 1024;
pub const MAX_DETAILS_DISCLOSURE_BYTES: usize = 8 * 1024;

/// Specification of visible and hidden fields for a record or sub-object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HiddenSet {
    pub hidden: &'static [&'static str],
    pub visible: &'static [&'static str],
}

pub const TERMS_SET: HiddenSet = HiddenSet {
    hidden: &["listing_id", "scope", "cancellation_terms", "refund_terms", "dispute_path"],
    visible: &["currency", "quote_expires_at_secs", "home_exchange", "commitments"],
};

pub const COMMITMENT_SET: HiddenSet = HiddenSet {
    hidden: &[
        "description",
        "declared_value_minor",
        "details",
        "window",
        "location",
        "payee",
        "payment_methods",
        "tax_minor",
        "fees_minor",
        "eep",
    ],
    visible: &["commitment_id", "giver", "resource_kind", "kind", "extension", "after"],
};

pub const PAYMENT_REQUEST_SET: HiddenSet = HiddenSet {
    hidden: &["note"],
    visible: &[
        "agreement",
        "conversation",
        "commitment_id",
        "quote_author_did",
        "counterparty_did",
    ],
};

pub const PAYMENT_ACK_SET: HiddenSet = HiddenSet {
    hidden: &["method", "reference", "settlement_note"],
    visible: &[
        "agreement",
        "conversation",
        "commitment_id",
        "quote_author_did",
        "counterparty_did",
        "role",
        "observed_at_secs",
        "supersedes",
    ],
};

pub const FULFILMENT_SET: HiddenSet = HiddenSet {
    hidden: &["settlement_note"],
    visible: &[
        "agreement",
        "conversation",
        "commitment_id",
        "quote_author_did",
        "counterparty_did",
        "role",
    ],
};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SdError {
    #[error("sd shape invalid: {0}")]
    SdShape(String),
    #[error("disclosure unknown digest: {0}")]
    DisclosureUnknownDigest(String),
    #[error("digest repeated: {0}")]
    DigestRepeated(String),
    #[error("disclosure name invalid: {0}")]
    DisclosureName(String),
    #[error("disclosure name matches visible field: {0}")]
    DisclosureVisibleName(String),
    #[error("disclosure value invalid: {0}")]
    DisclosureValue(String),
    #[error("salt length must be 16 bytes, found {0}")]
    SaltLength(usize),
    #[error("salt repeated: {0}")]
    SaltRepeated(String),
    #[error("quote disclosures ({bytes} bytes) exceed maximum ({limit} bytes)")]
    QuoteTooLarge { bytes: usize, limit: usize },
    #[error(
        "commitment {commitment_id} details disclosure ({bytes} bytes) exceeds maximum ({limit} \
         bytes)"
    )]
    DetailsTooLarge { commitment_id: String, bytes: usize, limit: usize },
    #[error("disclosure missing: {0}")]
    DisclosureMissing(String),
    #[error("disclosure count {count} exceeds maximum {limit}")]
    DisclosureCountTooHigh { count: usize, limit: usize },
    #[error("unclassified field '{0}' is neither visible nor hidden")]
    UnclassifiedField(String),
}

impl SdError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::SdShape(_) => "sd-shape",
            Self::DisclosureUnknownDigest(_) => "disclosure-unknown-digest",
            Self::DigestRepeated(_) => "digest-repeated",
            Self::DisclosureName(_) => "disclosure-name",
            Self::DisclosureVisibleName(_) => "disclosure-visible-name",
            Self::DisclosureValue(_) => "disclosure-value",
            Self::SaltLength(_) => "salt-length",
            Self::SaltRepeated(_) => "salt-repeated",
            Self::QuoteTooLarge { .. } => "quote-too-large",
            Self::DetailsTooLarge { .. } => "details-too-large",
            Self::DisclosureMissing(_) => "disclosure-missing",
            Self::DisclosureCountTooHigh { .. } => "disclosure-count-too-high",
            Self::UnclassifiedField(_) => "unclassified-field",
        }
    }
}

/// Moves each hidden name out of `obj` (a missing key becomes `null`),
/// adds `_sd` (sorted ascending). Refuses an unclassified field.
pub fn conceal_object(
    obj: &mut Map<String, Value>,
    set: &HiddenSet,
    salts: &mut impl Iterator<Item = Salt>,
) -> Result<Vec<String>, SdError> {
    for key in obj.keys() {
        if !set.visible.contains(&key.as_str()) && !set.hidden.contains(&key.as_str()) {
            return Err(SdError::UnclassifiedField(key.clone()));
        }
    }

    let mut disclosures = Vec::with_capacity(set.hidden.len());
    let mut digests = Vec::with_capacity(set.hidden.len());

    for &hidden_name in set.hidden {
        let val = obj.remove(hidden_name).unwrap_or(Value::Null);
        let salt = salts.next().ok_or_else(|| SdError::SdShape("not enough salts".to_string()))?;
        let disc = encode_disclosure(&salt, hidden_name, &val)
            .map_err(|e| SdError::DisclosureValue(e.to_string()))?;
        let digest = disclosure_digest(&disc);
        disclosures.push(disc);
        digests.push(digest);
    }

    digests.sort();
    obj.insert("_sd".to_string(), Value::Array(digests.into_iter().map(Value::String).collect()));

    Ok(disclosures)
}

/// An index of decoded disclosures keyed by their digest.
#[derive(Debug, Clone)]
pub struct DisclosureIndex {
    entries: HashMap<String, (DecodedDisclosure, usize)>,
    total_bytes: usize,
}

impl DisclosureIndex {
    /// Builds an index from encoded disclosure strings.
    ///
    /// Refuses more than `max_count` disclosures, or one longer than
    /// `MAX_RECORD_DISCLOSURE_BYTES`, before any decoding.
    pub fn build(disclosures: &[String], max_count: usize) -> Result<Self, SdError> {
        if disclosures.len() > max_count {
            return Err(SdError::DisclosureCountTooHigh {
                count: disclosures.len(),
                limit: max_count,
            });
        }

        let mut total_bytes = 0;
        for d in disclosures {
            if d.len() > MAX_RECORD_DISCLOSURE_BYTES {
                return Err(SdError::QuoteTooLarge {
                    bytes: d.len(),
                    limit: MAX_RECORD_DISCLOSURE_BYTES,
                });
            }
            total_bytes += d.len();
        }

        let mut entries = HashMap::with_capacity(disclosures.len());
        for d in disclosures {
            let decoded = decode_disclosure(d).map_err(|e| match e {
                DisclosureError::NonIntegerNumber => SdError::DisclosureValue(e.to_string()),
                _ => SdError::SdShape(e.to_string()),
            })?;

            let digest = decoded.digest.clone();
            if entries.contains_key(&digest) {
                return Err(SdError::DigestRepeated(digest));
            }
            entries.insert(digest, (decoded, d.len()));
        }

        Ok(Self { entries, total_bytes })
    }

    /// Verifies Roym rules: every salt decodes to exactly 16 bytes and no salt
    /// is repeated in this index.
    pub fn check_salts(&self) -> Result<(), SdError> {
        let mut seen_salts = HashSet::with_capacity(self.entries.len());
        for (decoded, _) in self.entries.values() {
            let salt_bytes = decode_base64url(&decoded.salt).map_err(|_| SdError::SaltLength(0))?;
            if salt_bytes.len() != 16 {
                return Err(SdError::SaltLength(salt_bytes.len()));
            }
            if !seen_salts.insert(decoded.salt.clone()) {
                return Err(SdError::SaltRepeated(decoded.salt.clone()));
            }
        }
        Ok(())
    }

    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    pub fn get(&self, digest: &str) -> Option<&(DecodedDisclosure, usize)> {
        self.entries.get(digest)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn digests(&self) -> impl Iterator<Item = &String> {
        self.entries.keys()
    }
}

/// The result of revealing an object against a disclosure index.
#[derive(Debug, Clone, PartialEq)]
pub struct RevealedObject {
    pub fields: Map<String, Value>,
    pub hidden: Vec<&'static str>,
    pub used: Vec<String>,
}

/// Reveals fields from an object whose hidden fields are represented in `_sd`.
pub fn reveal_object(
    obj: &Map<String, Value>,
    set: &HiddenSet,
    index: &DisclosureIndex,
) -> Result<RevealedObject, SdError> {
    let sd_val = obj.get("_sd").ok_or_else(|| SdError::SdShape("missing _sd field".to_string()))?;
    let sd_arr =
        sd_val.as_array().ok_or_else(|| SdError::SdShape("_sd is not an array".to_string()))?;

    if sd_arr.len() != set.hidden.len() {
        return Err(SdError::SdShape(format!(
            "_sd length {} does not match expected {}",
            sd_arr.len(),
            set.hidden.len()
        )));
    }

    let mut digests = Vec::with_capacity(sd_arr.len());
    for v in sd_arr {
        let d = v.as_str().ok_or_else(|| SdError::SdShape("digest is not a string".to_string()))?;
        if d.len() != DIGEST_CHARS {
            return Err(SdError::SdShape(format!(
                "digest length {} does not match {DIGEST_CHARS}",
                d.len()
            )));
        }
        digests.push(d);
    }

    for window in digests.windows(2) {
        if window[0] >= window[1] {
            return Err(SdError::SdShape("_sd digests not strictly ascending".to_string()));
        }
    }

    let mut fields = Map::new();
    for (k, v) in obj {
        if k == "_sd" {
            continue;
        }
        if set.hidden.contains(&k.as_str()) {
            return Err(SdError::SdShape(format!("hidden field '{k}' present in plain form")));
        }
        fields.insert(k.clone(), v.clone());
    }

    let mut used = Vec::new();
    let mut disclosed_names = HashSet::new();

    for &digest in &digests {
        if let Some((decoded, _)) = index.get(digest) {
            if set.visible.contains(&decoded.name.as_str()) {
                return Err(SdError::DisclosureVisibleName(decoded.name.clone()));
            }
            if !set.hidden.contains(&decoded.name.as_str()) {
                return Err(SdError::DisclosureName(decoded.name.clone()));
            }
            if !disclosed_names.insert(decoded.name.clone()) {
                return Err(SdError::SdShape(format!(
                    "field '{}' disclosed twice in same object",
                    decoded.name
                )));
            }
            fields.insert(decoded.name.clone(), decoded.value.clone());
            used.push(digest.to_string());
        }
    }

    let mut hidden = Vec::new();
    for &h in set.hidden {
        if !disclosed_names.contains(h) {
            hidden.push(h);
        }
    }

    Ok(RevealedObject { fields, hidden, used })
}

/// Refuses any disclosure in `index` that was not used by any revealed object.
pub fn check_all_used(index: &DisclosureIndex, used: &[String]) -> Result<(), SdError> {
    let used_set: HashSet<&str> = used.iter().map(String::as_str).collect();
    for digest in index.digests() {
        if !used_set.contains(digest.as_str()) {
            return Err(SdError::DisclosureUnknownDigest(digest.clone()));
        }
    }
    Ok(())
}

/// Structured path designating a hidden field in a deal or settlement record.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FieldPath {
    Terms(String),
    Commitment(String, String),
    Record(String, String),
}

impl FieldPath {
    pub fn parse(s: &str) -> Result<Self, SdError> {
        if let Some(rest) = s.strip_prefix("terms.commitments[") {
            let (id, field_part) = rest
                .split_once("].")
                .ok_or_else(|| SdError::SdShape(format!("invalid commitment field path: {s}")))?;
            if id.is_empty() || field_part.is_empty() {
                return Err(SdError::SdShape(format!("empty commitment id or field in: {s}")));
            }
            Ok(Self::Commitment(id.to_string(), field_part.to_string()))
        } else if let Some(field) = s.strip_prefix("terms.") {
            if field.is_empty() {
                return Err(SdError::SdShape(format!("empty terms field in: {s}")));
            }
            Ok(Self::Terms(field.to_string()))
        } else if let Some((rec, field)) = s.split_once('.') {
            if rec.is_empty() || field.is_empty() {
                return Err(SdError::SdShape(format!("empty record or field in: {s}")));
            }
            Ok(Self::Record(rec.to_string(), field.to_string()))
        } else {
            Err(SdError::SdShape(format!("unrecognized field path format: {s}")))
        }
    }
}

impl fmt::Display for FieldPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Terms(field) => write!(f, "terms.{field}"),
            Self::Commitment(id, field) => write!(f, "terms.commitments[{id}].{field}"),
            Self::Record(rec, field) => write!(f, "{rec}.{field}"),
        }
    }
}

/// Helper to conceal a dedicated hidden struct into its sorted digests and
/// disclosures.
pub fn conceal_hidden<T: Serialize>(
    hidden: &T,
    set: &HiddenSet,
    salts: &mut impl Iterator<Item = Salt>,
) -> Result<(Vec<String>, Vec<String>), SdError> {
    let mut val = serde_json::to_value(hidden).map_err(|e| SdError::SdShape(e.to_string()))?;
    let obj = val.as_object_mut().ok_or_else(|| {
        SdError::SdShape("hidden payload must serialize to a JSON object".to_string())
    })?;
    let disclosures = conceal_object(obj, set, salts)?;
    let sd = match obj.remove("_sd") {
        Some(Value::Array(arr)) => arr
            .into_iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Ok((sd, disclosures))
}

/// Strict reveal helper for a single hidden struct from its `_sd` and
/// disclosures.
pub fn reveal_hidden_strict<T: DeserializeOwned>(
    sd: &[String],
    set: &HiddenSet,
    disclosures: &[String],
) -> Result<T, SdError> {
    let index = DisclosureIndex::build(disclosures, set.hidden.len())?;
    index.check_salts()?;
    let mut obj = Map::new();
    obj.insert("_sd".to_string(), Value::Array(sd.iter().cloned().map(Value::String).collect()));
    let revealed = reveal_object(&obj, set, &index)?;
    check_all_used(&index, &revealed.used)?;
    if let Some(first_missing) = revealed.hidden.first() {
        return Err(SdError::DisclosureMissing((*first_missing).to_string()));
    }
    serde_json::from_value(Value::Object(revealed.fields))
        .map_err(|e| SdError::SdShape(e.to_string()))
}
