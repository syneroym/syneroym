//! Closing commitments of an agreement on the node.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use syneroym_roym_core::{
    clock,
    deal::{CommitmentKind, transition::is_final},
    envelope::Response,
    method::Reply,
    progress::AgreementState,
    transaction::{PairState, pair_state},
};

use crate::app::{
    AGREEMENTS, AgreementRow, AppHost, SendState,
    fulfilment_ops::{self, FulfilmentSignInput},
    get_row, parties,
    payment_ops::{self, PaymentAckInput},
    refusal::refusal,
    resolve_principal_and_owner,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CloseAs {
    Giver,
    Receiver,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgreementCloseInput {
    pub agreement: String,
    #[serde(rename = "as")]
    pub close_as: CloseAs,
    pub commitments: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseRecord {
    pub commitment_id: String,
    pub record_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    pub state: SendState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseSkipped {
    pub commitment_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgreementCloseOutput {
    pub records: Vec<CloseRecord>,
    pub skipped: Vec<CloseSkipped>,
}

enum SignOutcome {
    Record(CloseRecord),
    Skipped(CloseSkipped),
}

fn validate_commitment_list(commitments: &[String]) -> Result<(), Response> {
    if commitments.is_empty() {
        return Err(refusal("bad-commitment-list"));
    }
    let mut seen = HashSet::with_capacity(commitments.len());
    for c in commitments {
        if !seen.insert(c.as_str()) {
            return Err(refusal("bad-commitment-list"));
        }
    }
    Ok(())
}

fn check_agreement_open(agr: &AgreementRow) -> Result<(), Response> {
    if agr.state == Some(AgreementState::Cancelled)
        || agr.state == Some(AgreementState::NotFormed)
        || agr.state.is_none()
        || pair_state(agr.counterparty.as_ref(), agr.quote_author.as_ref()) != PairState::Complete
    {
        return Err(refusal("agreement-not-open"));
    }
    Ok(())
}

fn validate_commitments_side(
    agr: &AgreementRow,
    commitments: &[String],
    close_as: CloseAs,
    owner: &str,
) -> Result<(), Response> {
    let deal_role = parties::agreement_party(agr, owner)?;
    for cid in commitments {
        let Some(comm) = agr.terms.commitments.iter().find(|c| c.commitment_id == *cid) else {
            return Err(refusal("no-such-commitment"));
        };
        let is_giver = comm.giver == deal_role;
        match close_as {
            CloseAs::Giver => {
                if !is_giver {
                    return Err(refusal("not-your-side"));
                }
            }
            CloseAs::Receiver => {
                if is_giver {
                    return Err(refusal("not-your-side"));
                }
            }
        }
    }
    Ok(())
}

async fn sign_one_commitment<H: AppHost>(
    host: &H,
    agreement: &str,
    cid: &str,
    now: u64,
) -> Result<SignOutcome, Response> {
    let curr_agr: AgreementRow = match get_row(host, AGREEMENTS, agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(refusal("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };
    if curr_agr.state == Some(AgreementState::Closed) {
        return Ok(SignOutcome::Skipped(CloseSkipped {
            commitment_id: cid.to_string(),
            reason: "final".to_string(),
        }));
    }
    if let Some(cp) = curr_agr.commitments.iter().find(|c| c.commitment_id == cid)
        && is_final(cp.state)
    {
        return Ok(SignOutcome::Skipped(CloseSkipped {
            commitment_id: cid.to_string(),
            reason: "final".to_string(),
        }));
    }
    let comm = curr_agr
        .terms
        .commitments
        .iter()
        .find(|c| c.commitment_id == cid)
        .ok_or_else(|| refusal("no-such-commitment"))?;

    match comm.kind {
        CommitmentKind::Fulfilment => {
            let res = fulfilment_ops::sign_at(
                host,
                FulfilmentSignInput {
                    agreement: agreement.to_string(),
                    commitment_id: cid.to_string(),
                    settlement_note: None,
                },
                now,
            )
            .await?;
            Ok(SignOutcome::Record(CloseRecord {
                commitment_id: cid.to_string(),
                record_id: res.record_id,
                message_id: res.message_id,
                state: res.state,
            }))
        }
        CommitmentKind::Currency => {
            let res = payment_ops::acknowledge_at(
                host,
                PaymentAckInput {
                    agreement: agreement.to_string(),
                    commitment_id: cid.to_string(),
                    observed_at_secs: None,
                    method: None,
                    reference: None,
                    settlement_note: None,
                    supersedes: None,
                },
                now,
            )
            .await?;
            Ok(SignOutcome::Record(CloseRecord {
                commitment_id: cid.to_string(),
                record_id: res.record_id,
                message_id: res.message_id,
                state: res.state,
            }))
        }
    }
}

pub async fn close<H: AppHost>(
    host: &H,
    input: AgreementCloseInput,
) -> Reply<AgreementCloseOutput> {
    close_at(host, input, clock::now_secs()).await
}

pub async fn close_at<H: AppHost>(
    host: &H,
    input: AgreementCloseInput,
    now: u64,
) -> Reply<AgreementCloseOutput> {
    validate_commitment_list(&input.commitments)?;

    let agr: AgreementRow = match get_row(host, AGREEMENTS, &input.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(refusal("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    check_agreement_open(&agr)?;

    let (_principal, owner) = resolve_principal_and_owner(host, now).await?;
    validate_commitments_side(&agr, &input.commitments, input.close_as, &owner)?;

    if agr.state == Some(AgreementState::Closed) {
        let skipped = input
            .commitments
            .into_iter()
            .map(|cid| CloseSkipped { commitment_id: cid, reason: "final".to_string() })
            .collect();
        return Ok(AgreementCloseOutput { records: Vec::new(), skipped });
    }

    let mut records = Vec::with_capacity(input.commitments.len());
    let mut skipped = Vec::new();

    for cid in &input.commitments {
        match sign_one_commitment(host, &input.agreement, cid, now).await? {
            SignOutcome::Record(r) => records.push(r),
            SignOutcome::Skipped(s) => skipped.push(s),
        }
    }

    Ok(AgreementCloseOutput { records, skipped })
}
