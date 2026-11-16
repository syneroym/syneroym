//! Input types for quote term submission.

use serde::{Deserialize, Serialize};
use syneroym_roym_core::{
    deal::{
        Commitment, CommitmentKind, DealTerms, Details, ExtensionRef, LocalText, ResourceKind,
        commitment_id,
    },
    transaction::{QuoteLocation, TimeWindow},
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DealTermsInput {
    pub currency: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_exchange: Option<String>,
    pub commitments: Vec<CommitmentInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listing_id: Option<String>,
    pub scope: LocalText,
    pub cancellation_terms: LocalText,
    pub refund_terms: LocalText,
    pub dispute_path: LocalText,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommitmentInput {
    pub giver: syneroym_roym_core::transaction::DealRole,
    pub resource_kind: ResourceKind,
    pub kind: CommitmentKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension: Option<ExtensionRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
    pub description: LocalText,
    pub declared_value_minor: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Details>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<TimeWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<QuoteLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payee: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_methods: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tax_minor: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fees_minor: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eep: Option<serde_json::Value>,
}

impl DealTermsInput {
    #[must_use]
    pub fn into_deal_terms(self, quote_expires_at_secs: u64) -> DealTerms {
        let commitments = self
            .commitments
            .into_iter()
            .enumerate()
            .map(|(i, c)| Commitment {
                commitment_id: commitment_id(i),
                giver: c.giver,
                resource_kind: c.resource_kind,
                kind: c.kind,
                extension: c.extension,
                after: c.after,
                description: c.description,
                declared_value_minor: c.declared_value_minor,
                details: c.details,
                window: c.window,
                location: c.location,
                payee: c.payee,
                payment_methods: c.payment_methods,
                tax_minor: c.tax_minor,
                fees_minor: c.fees_minor,
                eep: c.eep,
            })
            .collect();

        DealTerms {
            currency: self.currency,
            quote_expires_at_secs,
            home_exchange: self.home_exchange,
            commitments,
            listing_id: self.listing_id,
            scope: self.scope,
            cancellation_terms: self.cancellation_terms,
            refund_terms: self.refund_terms,
            dispute_path: self.dispute_path,
        }
    }
}
