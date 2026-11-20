//! Inbound filing for step cards.

use serde_json::Value;
use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    deal::SettlementRole,
    disclosure::{STEP_SET, reveal_hidden_strict},
    extension::{self, Registry},
    record::RECORD_STEP,
    step::{self, MAX_STEPS_PER_AGREEMENT, StepError, StepHidden, StepPayload},
    transaction::{DealRole, PairState, ReceiptHalf, pair_state},
};

use super::{
    super::{
        AGREEMENTS, AgreementRow, CardRow, SendState, disclosures, get_row, parties, put_card,
        step_ops::{self, StepRow},
    },
    FileCardResult,
    receipts::defer_or_refuse,
    refuse_card,
};

/// Why a card is not filed. `defer` means the card may become fileable once
/// this node has the agreement.
struct Refusal {
    reason: String,
    defer: bool,
}

impl Refusal {
    fn now(reason: impl Into<String>) -> Self {
        Self { reason: reason.into(), defer: false }
    }

    fn later(reason: impl Into<String>) -> Self {
        Self { reason: reason.into(), defer: true }
    }
}

/// The agreement row agrees with the card: it is formed here, names the same
/// parties and conversation, and the issuer is the party that `role` says
/// for that commitment, under the extension the commitment names.
fn check_against_agreement(
    agr: Option<&AgreementRow>,
    p: &StepPayload,
    conversation: &str,
    issuer: Option<&str>,
) -> Result<(), Refusal> {
    let agr = agr.ok_or_else(|| Refusal::later("names an agreement this node does not hold"))?;
    let pair = pair_state(agr.counterparty.as_ref(), agr.quote_author.as_ref());
    if pair != PairState::Complete {
        return Err(Refusal::later("agreement not complete on this node"));
    }
    if p.quote_author_did != agr.quote_author_did || p.counterparty_did != agr.counterparty_did {
        return Err(Refusal::now("names the wrong parties"));
    }
    if p.conversation != conversation || p.conversation != agr.conversation {
        return Err(Refusal::now("card names another conversation"));
    }
    let commitment = agr
        .terms
        .commitments
        .iter()
        .find(|c| c.commitment_id == p.commitment_id)
        .ok_or_else(|| Refusal::now("no-such-commitment"))?;
    if commitment.extension.as_ref() != Some(&p.extension) {
        return Err(Refusal::now("the commitment names another extension"));
    }
    let issuer_role = match issuer {
        Some(did) if did == agr.quote_author_did => DealRole::QuoteAuthor,
        Some(did) if did == agr.counterparty_did => DealRole::Counterparty,
        _ => return Err(Refusal::now("not-a-party")),
    };
    if p.role != parties::settlement_role(issuer_role, commitment.giver) {
        return Err(Refusal::now("role-mismatch"));
    }
    Ok(())
}

/// The registry's verdict on the step. A step of an extension that this node
/// does not know is filed, and shown as not checked.
fn check_step(registry: &Registry, p: &StepPayload, hidden: &StepHidden) -> Result<(), Refusal> {
    step::check_step(registry, &p.extension, &p.step_type, p.role, &hidden.data)
        .map(|_| ())
        .map_err(|e: StepError| Refusal::now(format!("step: {}", e.code())))
}

pub(crate) async fn file_step_card<H: AppHost>(
    host: &H,
    row: CardRow,
    envelope: &str,
    disclosures: &[String],
    conversation: &str,
    now: u64,
) -> Result<FileCardResult, String> {
    file_step_card_in(extension::registry(), host, row, envelope, disclosures, conversation, now)
        .await
}

/// `file_step_card` against a chosen registry, so a test can use a manifest
/// that does not ship.
pub(crate) async fn file_step_card_in<H: AppHost>(
    registry: &Registry,
    host: &H,
    mut row: CardRow,
    envelope: &str,
    disclosures: &[String],
    conversation: &str,
    now: u64,
) -> Result<FileCardResult, String> {
    let v = step::verify_step(envelope, now);
    let Some(p) = v.payload.as_ref().filter(|_| v.verified) else {
        let reason = v.reason.clone().unwrap_or_else(|| "step does not verify".to_string());
        return refuse_card(host, row, reason).await;
    };
    let hidden: StepHidden = match reveal_hidden_strict(&p.sd, &STEP_SET, disclosures) {
        Ok(h) => h,
        Err(e) => return refuse_card(host, row, format!("step disclosures: {}", e.code())).await,
    };
    let agr: Option<AgreementRow> = get_row(host, AGREEMENTS, &p.agreement).await?;
    let checked = check_against_agreement(agr.as_ref(), p, conversation, v.issuer.as_deref())
        .and_then(|()| check_step(registry, p, &hidden));
    if let Err(refusal) = checked {
        return if refusal.defer {
            defer_or_refuse(host, row, envelope, now, &refusal.reason).await
        } else {
            refuse_card(host, row, refusal.reason).await
        };
    }

    let rec_id = v.record_id.clone().unwrap_or_default();
    let stored = StepRow {
        agreement: p.agreement.clone(),
        commitment_id: p.commitment_id.clone(),
        conversation: conversation.to_string(),
        step_type: p.step_type.clone(),
        role: p.role,
        half: ReceiptHalf {
            record_id: rec_id.clone(),
            envelope: envelope.to_string(),
            issuer: v.issuer.clone().unwrap_or_default(),
            issued_at_secs: v.issued_at_secs.unwrap_or(now),
        },
        state: SendState::Delivered,
    };
    match store_step(host, stored, p.role, disclosures, now).await? {
        Stored::New => {}
        Stored::Repeat => row.repeat = true,
        Stored::Refused(reason) => return refuse_card(host, row, reason).await,
    }

    let p_val = serde_json::to_value(p).unwrap_or(Value::Null);
    row.known = true;
    row.verified = true;
    row.record_id = Some(rec_id);
    row.data = Some(disclosures::plain_view(&p_val, disclosures));
    put_card(host, &row).await?;
    Ok(FileCardResult {
        filed: true,
        refused: false,
        unknown: false,
        countersigned: false,
        deferred: false,
    })
}

enum Stored {
    New,
    Repeat,
    Refused(&'static str),
}

/// Writes the step row, after its disclosure. A second record for the same
/// key is refused and leaves nothing behind.
async fn store_step<H: AppHost>(
    host: &H,
    stored: StepRow,
    role: SettlementRole,
    disclosures: &[String],
    now: u64,
) -> Result<Stored, String> {
    let key = step_ops::step_id(&stored.agreement, &stored.commitment_id, &stored.step_type, role);
    let existing = step_ops::load_step_row(host, &key).await?;
    if let Some(ex) = existing {
        return Ok(if ex.half.record_id == stored.half.record_id {
            Stored::Repeat
        } else {
            Stored::Refused("a second step of this type by this role")
        });
    }
    let held = step_ops::count_steps(host, &stored.agreement).await?;
    if held >= MAX_STEPS_PER_AGREEMENT {
        return Ok(Stored::Refused("too-many-steps"));
    }
    disclosures::store(host, &stored.half.record_id, RECORD_STEP, disclosures, now).await?;
    match step_ops::claim_step_row(host, &key, &stored).await? {
        None => Ok(Stored::New),
        Some(winner) if winner.half.record_id == stored.half.record_id => Ok(Stored::Repeat),
        Some(_) => Ok(Stored::Refused("a second step of this type by this role")),
    }
}
