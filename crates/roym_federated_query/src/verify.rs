//! Verification of returned envelopes and building result rows.

use std::collections::BTreeMap;

use serde::Serialize;
use syneroym_roym_core::{
    directory::{MAX_REFUSED_RESULTS, MAX_STORED_PER_SOURCE, SearchHit},
    listing::{self, ListingStatus, ListingVerdict},
    membership::{MembershipEvidence, MembershipVerdict},
};

use crate::{
    SourceReport,
    gather::{Source, SourceOutcome},
    merge::Row,
};

#[derive(Debug, Clone, PartialEq)]
pub struct HeldEvidence {
    pub source: String,
    pub member_did: String,
    pub pinned_issuer: Option<String>,
    pub evidence: MembershipEvidence,
}

#[derive(Debug)]
pub struct VerifiedOutcomes {
    pub verified_rows: Vec<Row>,
    pub refused_rows: Vec<Row>,
    pub reports: Vec<SourceReport>,
    pub envelopes_by_record_id: BTreeMap<String, String>,
    pub evidence: Vec<HeldEvidence>,
}

struct VerifyAccumulators<'a> {
    verified_rows: &'a mut Vec<Row>,
    refused_rows: &'a mut Vec<Row>,
    envelopes_by_record_id: &'a mut BTreeMap<String, String>,
    evidence: &'a mut Vec<HeldEvidence>,
}

pub fn verify_outcomes(outcomes: Vec<SourceOutcome>, now_secs: u64) -> VerifiedOutcomes {
    let mut verified_rows = Vec::new();
    let mut refused_rows = Vec::new();
    let mut reports = Vec::new();
    let mut envelopes_by_record_id = BTreeMap::new();
    let mut evidence = Vec::new();

    let mut acc = VerifyAccumulators {
        verified_rows: &mut verified_rows,
        refused_rows: &mut refused_rows,
        envelopes_by_record_id: &mut envelopes_by_record_id,
        evidence: &mut evidence,
    };

    for outcome in outcomes {
        match outcome {
            SourceOutcome::Error { source, error } => {
                reports.push(SourceReport {
                    source: source.did,
                    verified: 0,
                    refused: 0,
                    truncated: false,
                    error: Some(error),
                });
            }
            SourceOutcome::Success { source, hits, truncated } => {
                let report = verify_single_source(source, hits, truncated, now_secs, &mut acc);
                reports.push(report);
            }
        }
    }

    VerifiedOutcomes { verified_rows, refused_rows, reports, envelopes_by_record_id, evidence }
}

fn verify_single_source(
    source: Source,
    hits: Vec<SearchHit>,
    truncated: bool,
    now_secs: u64,
    acc: &mut VerifyAccumulators<'_>,
) -> SourceReport {
    let mut verified_count = 0u32;
    let mut refused_count = 0u32;

    for hit in hits {
        let verdict = listing::verify_envelope(&hit.envelope, now_secs);
        if verdict.verified {
            if verified_count >= MAX_STORED_PER_SOURCE {
                continue;
            }
            if let Some(row) = build_verified_row(
                &source,
                &hit,
                verdict,
                now_secs,
                acc.envelopes_by_record_id,
                acc.evidence,
            ) {
                acc.verified_rows.push(row);
                verified_count += 1;
            }
        } else {
            if refused_count >= MAX_REFUSED_RESULTS {
                continue;
            }
            acc.refused_rows.push(build_refused_row(&source, hit, verdict));
            refused_count += 1;
        }
    }

    SourceReport {
        source: source.did,
        verified: verified_count,
        refused: refused_count,
        truncated,
        error: None,
    }
}

fn build_verified_row(
    source: &Source,
    hit: &SearchHit,
    verdict: ListingVerdict,
    _now_secs: u64,
    envelopes_by_record_id: &mut BTreeMap<String, String>,
    evidence: &mut Vec<HeldEvidence>,
) -> Option<Row> {
    let payload = verdict.payload?;
    let issuer = verdict.issuer.unwrap_or_default();
    let listing_id = verdict.listing_id.unwrap_or_default();
    let areas = payload.location.as_ref().map(|l| l.service_area.clone()).unwrap_or_default();
    let record_id = verdict.record_id.unwrap_or_default();
    envelopes_by_record_id.insert(record_id.clone(), hit.envelope.clone());
    evidence.push(HeldEvidence {
        source: source.did.clone(),
        member_did: issuer.clone(),
        pinned_issuer: source.pinned_issuer.clone(),
        evidence: hit.membership.clone(),
    });

    Some(Row {
        listing_id,
        record_id,
        source: source.did.clone(),
        issuer,
        title: payload.title,
        summary: payload.summary,
        categories: payload.categories,
        conversation_address: verdict.conversation_address.unwrap_or_default(),
        status: serde_str(&verdict.status.unwrap_or(ListingStatus::Active)),
        reason: None,
        revocation_status: verdict.revocation_status.unwrap_or_else(|| "unknown".to_string()),
        membership: if source.pinned_issuer.is_none() {
            Some(MembershipVerdict::Unknown { reason: "issuer-not-pinned".to_string() })
        } else {
            None
        },
        issued_at_secs: verdict.issued_at_secs.unwrap_or(hit.issued_at_secs),
        received_at_secs: hit.received_at_secs,
        raw_evidence: Some(hit.membership.clone()),
        pinned_issuer: source.pinned_issuer.clone(),
        areas,
    })
}

fn build_refused_row(source: &Source, hit: SearchHit, verdict: ListingVerdict) -> Row {
    Row {
        listing_id: hit.listing_id,
        record_id: hit.record_id,
        source: source.did.clone(),
        reason: verdict.reason,
        revocation_status: "unknown".to_string(),
        issued_at_secs: hit.issued_at_secs,
        received_at_secs: hit.received_at_secs,
        ..Default::default()
    }
}

fn serde_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|j| j.as_str().map(str::to_string)).unwrap_or_default()
}
