//! Inbound filing for commitment-failure cards.
//!
//! Only the writer of an agreement acts on one. It marks the commitment
//! `failed`, and the progress transition cancels the commitments that are
//! still open. The giver's own node files its card back as a sent card and
//! changes no state.

use serde_json::Value;
use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    commitment_failure::{CommitmentFailurePayload, verify_commitment_failure},
    deal::Commitment,
    progress::{AgreementEvent, AgreementState},
    transaction::DealRole,
};

use super::{
    super::{
        AGREEMENTS, AgreementRow, CardRow, LEDGER, get_row,
        hooks::needs_reservation,
        ledger::{FailureRow, LedgerKind, failure_id},
        progress_ops, put_card,
    },
    FileCardResult, refuse_card,
};

/// The commitment a failure names, when this node may act on the card.
/// `issuer` is the person that signed it.
fn check_against_agreement<'a>(
    agr: &'a AgreementRow,
    p: &CommitmentFailurePayload,
    conversation: &str,
    owner: &str,
    issuer: &str,
) -> Result<&'a Commitment, &'static str> {
    if p.conversation != conversation || p.conversation != agr.conversation {
        return Err("card names another conversation");
    }
    if owner != agr.quote_author_did {
        return Err("only the writer of the agreement files a commitment failure");
    }
    if agr.state != Some(AgreementState::Agreed) {
        return Err("the agreement is not agreed");
    }
    let commitment = agr
        .terms
        .commitments
        .iter()
        .find(|c| c.commitment_id == p.commitment_id)
        .ok_or("no-such-commitment")?;
    if commitment.giver != DealRole::Counterparty || issuer != agr.counterparty_did {
        return Err("only the giver of the commitment may fail it");
    }
    if !needs_reservation(commitment) {
        return Err("the commitment has no reservation to lose");
    }
    let settled =
        agr.commitments.iter().any(|c| c.commitment_id == p.commitment_id && !c.halves.is_empty());
    if settled {
        return Err("a settlement half exists for the commitment");
    }
    Ok(commitment)
}

pub(crate) async fn file_commitment_failure_card<H: AppHost>(
    host: &H,
    mut row: CardRow,
    envelope: &str,
    disclosures: &[String],
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    if !disclosures.is_empty() {
        return refuse_card(host, row, "disclosures not permitted on commitment failure card")
            .await;
    }
    let v = verify_commitment_failure(envelope, now);
    let Some(p) = v.payload.as_ref().filter(|_| v.verified) else {
        let reason = v.reason.clone().unwrap_or_else(|| "failure does not verify".to_string());
        return refuse_card(host, row, reason).await;
    };
    let rec_id = v.record_id.clone().unwrap_or_default();
    let issuer = v.issuer.clone().unwrap_or_default();
    let Some(agr) = get_row::<AgreementRow, _>(host, AGREEMENTS, &p.agreement).await? else {
        return refuse_card(host, row, "names an agreement this node does not hold").await;
    };

    if issuer != owner {
        let key = failure_id(&p.agreement, &p.commitment_id);
        match get_row::<FailureRow, _>(host, LEDGER, &key).await? {
            Some(stored) if stored.record_id == rec_id => row.repeat = true,
            Some(_) => {
                return refuse_card(host, row, "a second failure for this commitment").await;
            }
            None => {
                if let Err(reason) = check_against_agreement(&agr, p, conversation, owner, &issuer)
                {
                    return refuse_card(host, row, reason).await;
                }
                if let Err(reason) = apply_failure(host, p, now).await {
                    return refuse_card(host, row, reason).await;
                }
                store_failure(host, &key, p, envelope, &rec_id, now).await?;
            }
        }
    }

    row.known = true;
    row.verified = true;
    row.issuer = Some(issuer);
    row.record_id = Some(rec_id);
    row.data = Some(serde_json::to_value(p).unwrap_or(Value::Null));
    put_card(host, &row).await?;
    Ok(FileCardResult {
        filed: true,
        refused: false,
        unknown: false,
        countersigned: false,
        deferred: false,
    })
}

/// Marks the commitment failed in the next snapshot. A refusal of the
/// transition (the commitment cannot fail from its state) refuses the card.
async fn apply_failure<H: AppHost>(
    host: &H,
    p: &CommitmentFailurePayload,
    now: u64,
) -> Result<(), String> {
    let event = AgreementEvent::Failure {
        commitment_id: p.commitment_id.clone(),
        reason: p.reason.as_str().to_string(),
    };
    progress_ops::transition_at(host, &p.agreement, event, now)
        .await
        .map(|_| ())
        .map_err(|e| e.error.map_or_else(|| "the transition failed".to_string(), |x| x.message))
}

/// Stores the filed record. A repeat of the card, or a second sync that files
/// it at the same time, finds the row already there.
async fn store_failure<H: AppHost>(
    host: &H,
    key: &str,
    p: &CommitmentFailurePayload,
    envelope: &str,
    record_id: &str,
    now: u64,
) -> Result<(), String> {
    let row = FailureRow {
        kind: LedgerKind::Failure,
        agreement: p.agreement.clone(),
        commitment_id: p.commitment_id.clone(),
        conversation: p.conversation.clone(),
        record_id: record_id.to_string(),
        envelope: envelope.to_string(),
        created_at_secs: now,
        message_id: None,
    };
    let payload = serde_json::to_vec(&row).map_err(|e| e.to_string())?;
    let write = RecordWriteValue { id: key.to_string(), payload };
    AppDataLayer::create(host, LEDGER.to_string(), vec![write])
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}
