//! Backup verification for the `steps` section.
//!
//! Every row is checked as if it had just arrived as a card: the record must
//! verify, name this agreement and commitment, come from the party that its
//! role says, and carry a disclosure that reveals strictly. The row's key
//! must be the key that the payload makes.

use std::collections::HashMap;

use serde_json::Value;
use syneroym_roym_core::{
    deal::{Commitment, SettlementRole},
    disclosure::{STEP_SET, reveal_hidden_strict},
    envelope::Response,
    extension,
    step::{self, MAX_STEPS_PER_AGREEMENT, StepHidden, StepPayload},
    transaction::{DealRole, PairState, pair_state},
};

use super::{
    super::{
        AgreementRow,
        step_ops::{StepRow, step_id},
    },
    sections::HistoryEntry,
};

pub(crate) type ValidatedSteps = (Vec<(String, StepRow)>, Vec<HistoryEntry>);

fn refuse(id: &str, what: &str) -> Response {
    Response::invalid_params(format!("steps '{id}': {what}"))
}

/// The row's record matches the row and the agreement.
fn check_payload(
    id: &str,
    row: &StepRow,
    p: &StepPayload,
    issuer: Option<&str>,
    agr: &AgreementRow,
    commitment: &Commitment,
) -> Result<(), Response> {
    if p.agreement != row.agreement
        || p.commitment_id != row.commitment_id
        || p.step_type != row.step_type
        || p.role != row.role
    {
        return Err(refuse(id, "the record does not match its row"));
    }
    if p.conversation != agr.conversation
        || p.quote_author_did != agr.quote_author_did
        || p.counterparty_did != agr.counterparty_did
    {
        return Err(refuse(id, "names another conversation or other parties"));
    }
    if commitment.extension.as_ref() != Some(&p.extension) {
        return Err(refuse(id, "the commitment names another extension"));
    }
    let giver_did = match commitment.giver {
        DealRole::QuoteAuthor => &agr.quote_author_did,
        DealRole::Counterparty => &agr.counterparty_did,
    };
    let other_did = if giver_did == &agr.quote_author_did {
        &agr.counterparty_did
    } else {
        &agr.quote_author_did
    };
    let expected_issuer = match p.role {
        SettlementRole::Giver => giver_did,
        SettlementRole::Receiver => other_did,
    };
    if issuer != Some(expected_issuer.as_str()) || row.half.issuer != *expected_issuer {
        return Err(refuse(id, "issuer is not the party its role names"));
    }
    if id != step_id(&row.agreement, &row.commitment_id, &row.step_type, row.role) {
        return Err(refuse(id, "the key is not made from the record"));
    }
    Ok(())
}

/// The disclosure is in the bundle, reveals strictly and passes the step's
/// rules.
fn check_disclosure(
    id: &str,
    row: &StepRow,
    p: &StepPayload,
    disclosures: &HashMap<String, Vec<String>>,
) -> Result<(), Response> {
    let Some(disc) = disclosures.get(&row.half.record_id) else {
        return Err(refuse(id, "its disclosure is not in the bundle"));
    };
    let hidden: StepHidden = reveal_hidden_strict(&p.sd, &STEP_SET, disc)
        .map_err(|e| refuse(id, &format!("its disclosure does not reveal: {e}")))?;
    step::check_step(extension::registry(), &p.extension, &p.step_type, p.role, &hidden.data)
        .map_err(|e| refuse(id, &format!("its data fails: {}", e.code())))?;
    Ok(())
}

pub(crate) fn validate_steps(
    step_rows: Vec<Value>,
    agreements: &HashMap<String, AgreementRow>,
    disclosures: &HashMap<String, Vec<String>>,
    now: u64,
) -> Result<ValidatedSteps, Response> {
    let mut validated = Vec::with_capacity(step_rows.len());
    let mut history = Vec::with_capacity(step_rows.len());
    let mut per_agreement: HashMap<String, usize> = HashMap::new();
    for r in step_rows {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let payload = r.get("payload").cloned().unwrap_or(Value::Null);
        let row: StepRow = serde_json::from_value(payload)
            .map_err(|e| Response::invalid_params(format!("steps '{id}': invalid row: {e}")))?;

        let agr = agreements
            .get(&row.agreement)
            .ok_or_else(|| refuse(id, "references an agreement that is not in the bundle"))?;
        if pair_state(agr.counterparty.as_ref(), agr.quote_author.as_ref()) != PairState::Complete {
            return Err(refuse(id, "references an agreement that is not formed"));
        }
        let commitment = agr
            .terms
            .commitments
            .iter()
            .find(|c| c.commitment_id == row.commitment_id)
            .ok_or_else(|| refuse(id, "references a commitment that is not in the agreement"))?;

        let verdict = step::verify_step(&row.half.envelope, now);
        let Some(p) = verdict.payload.as_ref().filter(|_| verdict.verified) else {
            return Err(refuse(id, "the record does not verify"));
        };
        if verdict.record_id.as_deref() != Some(row.half.record_id.as_str()) {
            return Err(refuse(id, "the record id does not match its envelope"));
        }
        check_payload(id, &row, p, verdict.issuer.as_deref(), agr, commitment)?;
        check_disclosure(id, &row, p, disclosures)?;

        let count = per_agreement.entry(row.agreement.clone()).or_default();
        *count += 1;
        if *count > MAX_STEPS_PER_AGREEMENT {
            return Err(refuse(id, "the agreement holds too many steps"));
        }
        history.push((row.half.record_id.clone(), "step".to_string(), row.half.envelope.clone()));
        validated.push((id.to_string(), row));
    }
    Ok((validated, history))
}
