//! Selective disclosure wire types and reveal operations for deal terms.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use syneroym_signed_record::{Salt, encode_disclosure};

use crate::{
    deal::{
        CommitmentKind, DealError, DealTerms, ExtensionRef, MAX_COMMITMENTS, ResourceKind,
        commitment_id, validate_after_graph_entries,
    },
    disclosure::{
        COMMITMENT_SET, DisclosureIndex, FieldPath, MAX_DETAILS_DISCLOSURE_BYTES,
        MAX_RECORD_DISCLOSURE_BYTES, SdError, TERMS_SET, check_all_used, check_sd_shape,
        conceal_object, reveal_object,
    },
    extension, money,
    transaction::DealRole,
};

#[cfg(test)]
mod tests;

/// The wire representation of deal terms signed by quote author and both
/// halves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedTerms {
    pub currency: String,
    pub quote_expires_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_exchange: Option<String>,
    pub commitments: Vec<SignedCommitment>,
    #[serde(rename = "_sd")]
    pub sd: Vec<String>,
}

/// The wire representation of a single commitment inside `SignedTerms`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedCommitment {
    pub commitment_id: String,
    pub giver: DealRole,
    pub resource_kind: ResourceKind,
    pub kind: CommitmentKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension: Option<ExtensionRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
    #[serde(rename = "_sd")]
    pub sd: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConcealedTerms {
    pub signed: SignedTerms,
    pub disclosures: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisclosureSize {
    pub total_bytes: usize,
    pub details: Vec<(String, usize)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PartialTerms {
    pub value: Value,
    pub hidden: Vec<FieldPath>,
    pub used: Vec<String>,
}

impl DealTerms {
    /// Returns the exact number of fresh salts needed to conceal these terms:
    /// 5 for terms-level fields plus 10 per commitment.
    pub fn hidden_field_count(&self) -> usize {
        5 + self.commitments.len() * 10
    }

    /// Measures the total disclosure size using zero-valued 16-byte salts.
    ///
    /// Refuses any `details` disclosure over `MAX_DETAILS_DISCLOSURE_BYTES`
    /// or total disclosures over `MAX_RECORD_DISCLOSURE_BYTES`.
    pub fn disclosure_size(&self) -> Result<DisclosureSize, SdError> {
        let zero_salt = Salt::from_bytes([0u8; 16]);
        let mut total_bytes = 0;

        // Terms fields: listing_id, scope, cancellation_terms, refund_terms,
        // dispute_path
        let listing_val =
            serde_json::to_value(&self.listing_id).map_err(|e| SdError::SdShape(e.to_string()))?;
        let scope_val =
            serde_json::to_value(&self.scope).map_err(|e| SdError::SdShape(e.to_string()))?;
        let cancel_val = serde_json::to_value(&self.cancellation_terms)
            .map_err(|e| SdError::SdShape(e.to_string()))?;
        let refund_val = serde_json::to_value(&self.refund_terms)
            .map_err(|e| SdError::SdShape(e.to_string()))?;
        let dispute_val = serde_json::to_value(&self.dispute_path)
            .map_err(|e| SdError::SdShape(e.to_string()))?;

        for (name, val) in [
            ("listing_id", &listing_val),
            ("scope", &scope_val),
            ("cancellation_terms", &cancel_val),
            ("refund_terms", &refund_val),
            ("dispute_path", &dispute_val),
        ] {
            let disc = encode_disclosure(&zero_salt, name, val)
                .map_err(|e| SdError::DisclosureValue(e.to_string()))?;
            total_bytes += disc.len();
        }

        let mut details_sizes = Vec::new();

        for c in &self.commitments {
            let desc_val = serde_json::to_value(&c.description)
                .map_err(|e| SdError::SdShape(e.to_string()))?;
            let decl_val = serde_json::to_value(c.declared_value_minor)
                .map_err(|e| SdError::SdShape(e.to_string()))?;
            let details_val =
                serde_json::to_value(&c.details).map_err(|e| SdError::SdShape(e.to_string()))?;
            let window_val =
                serde_json::to_value(c.window).map_err(|e| SdError::SdShape(e.to_string()))?;
            let loc_val =
                serde_json::to_value(&c.location).map_err(|e| SdError::SdShape(e.to_string()))?;
            let payee_val =
                serde_json::to_value(&c.payee).map_err(|e| SdError::SdShape(e.to_string()))?;
            let methods_val = serde_json::to_value(&c.payment_methods)
                .map_err(|e| SdError::SdShape(e.to_string()))?;
            let tax_val =
                serde_json::to_value(c.tax_minor).map_err(|e| SdError::SdShape(e.to_string()))?;
            let fees_val =
                serde_json::to_value(c.fees_minor).map_err(|e| SdError::SdShape(e.to_string()))?;
            let eep_val =
                serde_json::to_value(&c.eep).map_err(|e| SdError::SdShape(e.to_string()))?;

            for (name, val) in [
                ("description", &desc_val),
                ("declared_value_minor", &decl_val),
                ("details", &details_val),
                ("window", &window_val),
                ("location", &loc_val),
                ("payee", &payee_val),
                ("payment_methods", &methods_val),
                ("tax_minor", &tax_val),
                ("fees_minor", &fees_val),
                ("eep", &eep_val),
            ] {
                let disc = encode_disclosure(&zero_salt, name, val)
                    .map_err(|e| SdError::DisclosureValue(e.to_string()))?;
                if name == "details" {
                    let d_len = disc.len();
                    details_sizes.push((c.commitment_id.clone(), d_len));
                }
                total_bytes += disc.len();
            }
        }

        Ok(DisclosureSize { total_bytes, details: details_sizes })
    }

    /// Conceals these terms into `SignedTerms` and disclosures using fresh
    /// salts.
    ///
    /// Validates both size limits upfront before consuming any salts.
    pub fn conceal(
        &self,
        salts: &mut impl Iterator<Item = Salt>,
    ) -> Result<ConcealedTerms, SdError> {
        let size = self.disclosure_size()?;
        if let Some((cid, bytes)) =
            size.details.iter().find(|(_, b)| *b > MAX_DETAILS_DISCLOSURE_BYTES)
        {
            return Err(SdError::DetailsTooLarge {
                commitment_id: cid.clone(),
                bytes: *bytes,
                limit: MAX_DETAILS_DISCLOSURE_BYTES,
            });
        }
        if size.total_bytes > MAX_RECORD_DISCLOSURE_BYTES {
            return Err(SdError::QuoteTooLarge {
                bytes: size.total_bytes,
                limit: MAX_RECORD_DISCLOSURE_BYTES,
            });
        }
        self.conceal_unbounded(salts)
    }

    /// Conceals these terms into `SignedTerms` and disclosures without applying
    /// size limits (used for sizing and upper-bound measurements).
    pub fn conceal_unbounded(
        &self,
        salts: &mut impl Iterator<Item = Salt>,
    ) -> Result<ConcealedTerms, SdError> {
        let mut all_disclosures = Vec::with_capacity(self.hidden_field_count());

        let mut terms_val =
            serde_json::to_value(self).map_err(|e| SdError::SdShape(e.to_string()))?;
        let terms_map = terms_val
            .as_object_mut()
            .ok_or_else(|| SdError::SdShape("expected terms object".to_string()))?;

        terms_map.remove("commitments");
        let terms_disclosures = conceal_object(terms_map, &TERMS_SET, salts)?;
        all_disclosures.extend(terms_disclosures);

        let mut signed_commitments = Vec::with_capacity(self.commitments.len());
        for c in &self.commitments {
            let mut comm_val =
                serde_json::to_value(c).map_err(|e| SdError::SdShape(e.to_string()))?;
            let comm_map = comm_val
                .as_object_mut()
                .ok_or_else(|| SdError::SdShape("expected commitment object".to_string()))?;
            let comm_disclosures = conceal_object(comm_map, &COMMITMENT_SET, salts)?;
            all_disclosures.extend(comm_disclosures);

            let signed_c: SignedCommitment =
                serde_json::from_value(Value::Object(comm_map.clone()))
                    .map_err(|e| SdError::SdShape(e.to_string()))?;
            signed_commitments.push(signed_c);
        }

        terms_map.insert(
            "commitments".to_string(),
            serde_json::to_value(&signed_commitments)
                .map_err(|e| SdError::SdShape(e.to_string()))?,
        );

        let signed: SignedTerms = serde_json::from_value(Value::Object(terms_map.clone()))
            .map_err(|e| SdError::SdShape(e.to_string()))?;

        Ok(ConcealedTerms { signed, disclosures: all_disclosures })
    }
}

impl SignedTerms {
    /// Validates the visible structure of `SignedTerms` without needing any
    /// disclosures.
    pub fn validate_visible(&self) -> Result<(), DealError> {
        if self.commitments.is_empty() || self.commitments.len() > MAX_COMMITMENTS {
            return Err(DealError::CommitmentCount(self.commitments.len()));
        }
        if money::currency_minor_exponent(&self.currency).is_none() {
            return Err(DealError::CurrencyUnknown(self.currency.clone()));
        }
        if self.home_exchange.is_some() {
            return Err(DealError::HomeExchangeNotSupported);
        }

        validate_sd_list(&self.sd, 5, "terms")?;

        let mut seen_digests = HashSet::new();
        for d in &self.sd {
            seen_digests.insert(d.as_str());
        }

        for (i, c) in self.commitments.iter().enumerate() {
            let expected_id = commitment_id(i);
            if c.commitment_id != expected_id {
                return Err(DealError::CommitmentId(i, expected_id, c.commitment_id.clone()));
            }
            extension::check_extension_kind(
                extension::registry(),
                c.resource_kind,
                c.kind,
                c.extension.as_ref(),
            )?;
            validate_sd_list(&c.sd, 10, &c.commitment_id)?;
            for d in &c.sd {
                if !seen_digests.insert(d.as_str()) {
                    return Err(DealError::SdShape(format!(
                        "digest repeated across deal terms: {d}"
                    )));
                }
            }
        }

        let entries: Vec<(&str, &[String])> = self
            .commitments
            .iter()
            .map(|c| (c.commitment_id.as_str(), c.after.as_slice()))
            .collect();
        validate_after_graph_entries(&entries)?;

        Ok(())
    }

    /// Reveals disclosed fields against an index, returning partial terms.
    pub fn reveal(&self, index: &DisclosureIndex) -> Result<PartialTerms, SdError> {
        self.validate_visible().map_err(|e| SdError::SdShape(e.to_string()))?;

        let mut terms_raw =
            serde_json::to_value(self).map_err(|e| SdError::SdShape(e.to_string()))?;
        let terms_map = terms_raw
            .as_object_mut()
            .ok_or_else(|| SdError::SdShape("expected object".to_string()))?;

        terms_map.remove("commitments");

        let revealed_terms = reveal_object(terms_map, &TERMS_SET, index)?;
        let mut hidden_paths = Vec::new();
        for h in revealed_terms.hidden {
            hidden_paths.push(FieldPath::Terms(h.to_string()));
        }
        let mut used_digests = revealed_terms.used;

        let mut revealed_commitments = Vec::with_capacity(self.commitments.len());
        for c in &self.commitments {
            let mut comm_raw =
                serde_json::to_value(c).map_err(|e| SdError::SdShape(e.to_string()))?;
            let comm_map = comm_raw
                .as_object_mut()
                .ok_or_else(|| SdError::SdShape("expected object".to_string()))?;

            let revealed_comm = reveal_object(comm_map, &COMMITMENT_SET, index)?;

            for h in revealed_comm.hidden {
                hidden_paths.push(FieldPath::Commitment(c.commitment_id.clone(), h.to_string()));
            }
            for u in revealed_comm.used {
                used_digests.push(u);
            }

            for digest in &c.sd {
                if let Some((decoded, len)) = index.get(digest)
                    && decoded.name == "details"
                    && *len > MAX_DETAILS_DISCLOSURE_BYTES
                {
                    return Err(SdError::DetailsTooLarge {
                        commitment_id: c.commitment_id.clone(),
                        bytes: *len,
                        limit: MAX_DETAILS_DISCLOSURE_BYTES,
                    });
                }
            }

            revealed_commitments.push(Value::Object(revealed_comm.fields));
        }

        let mut final_fields = revealed_terms.fields;
        final_fields.insert("commitments".to_string(), Value::Array(revealed_commitments));

        Ok(PartialTerms {
            value: Value::Object(final_fields),
            hidden: hidden_paths,
            used: used_digests,
        })
    }
}

fn validate_sd_list(sd: &[String], expected_len: usize, context: &str) -> Result<(), DealError> {
    check_sd_shape(sd, expected_len, context).map_err(DealError::SdShape)
}

/// Strict reveal for filing: builds index, checks salts, checks limits,
/// reveals terms, checks all disclosures used, ensures nothing remains hidden,
/// and validates the restored `DealTerms`.
pub fn reveal_terms_strict(
    signed: &SignedTerms,
    disclosures: &[String],
) -> Result<DealTerms, SdError> {
    let max_count = signed.commitments.len() * 10 + 5;
    let index = DisclosureIndex::build(disclosures, max_count)?;
    index.check_salts()?;

    if index.total_bytes() > MAX_RECORD_DISCLOSURE_BYTES {
        return Err(SdError::QuoteTooLarge {
            bytes: index.total_bytes(),
            limit: MAX_RECORD_DISCLOSURE_BYTES,
        });
    }

    let partial = signed.reveal(&index)?;
    check_all_used(&index, &partial.used)?;

    if let Some(first_missing) = partial.hidden.first() {
        return Err(SdError::DisclosureMissing(first_missing.to_string()));
    }

    let terms: DealTerms =
        serde_json::from_value(partial.value).map_err(|e| SdError::SdShape(e.to_string()))?;
    terms.validate().map_err(|e| SdError::SdShape(e.to_string()))?;

    Ok(terms)
}
