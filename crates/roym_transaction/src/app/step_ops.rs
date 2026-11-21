//! Step record signing, listing and verification.
//!
//! A step is one work step of an extension, for one commitment of a formed
//! agreement. It changes no deal state: it is a signed card that both
//! parties keep.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost, AppSigning,
    types::{
        data_layer::RecordWriteValue,
        signing::{Principal, RecordDraft},
    },
};
use syneroym_roym_core::{
    clock,
    deal::{DealError, ExtensionRef, SettlementRole, transition::is_final},
    disclosure::{STEP_SET, conceal_hidden, reveal_hidden_strict},
    envelope::Response,
    extension::{self, Registry},
    method::{self, DisclosedVerdict, DisclosedVerifyInput, Reply},
    paging,
    record::{Envelope, RECORD_STEP},
    step::{
        self, MAX_STEPS_PER_AGREEMENT, STEP_VERSION, StepCheck, StepError, StepHidden, StepPayload,
    },
    transaction::{DealRole, ReceiptHalf},
};

use super::{
    AGREEMENTS, AgreementRow, HalfRecordedOutput, OutgoingCard, STEPS, SendState, disclosures,
    get_row, parties, put_row, resolve_principal_and_owner, send_card_and_file,
};

/// One stored step. The key is `step_id`, so a party records one step of a
/// type once for a commitment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StepRow {
    pub(crate) agreement: String,
    pub(crate) commitment_id: String,
    pub(crate) conversation: String,
    pub(crate) step_type: String,
    pub(crate) role: SettlementRole,
    pub(crate) half: ReceiptHalf,
    pub(crate) state: SendState,
}

pub(crate) fn step_id(
    agreement: &str,
    commitment_id: &str,
    step_type: &str,
    role: SettlementRole,
) -> String {
    format!("step:{agreement}:{commitment_id}:{step_type}:{}", role.as_str())
}

fn empty_object() -> Value {
    json!({})
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepAddInput {
    pub agreement: String,
    pub commitment_id: String,
    pub step_type: String,
    #[serde(default = "empty_object")]
    pub data: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepListInput {
    pub agreement: String,
}

#[derive(Debug, Serialize)]
pub struct StepView {
    pub record_id: String,
    /// The signed record, so a client can pass it to `step.verify`.
    pub envelope: String,
    pub commitment_id: String,
    pub step_type: String,
    pub extension: ExtensionRef,
    pub role: SettlementRole,
    pub issuer: String,
    pub issued_at_secs: u64,
    /// The plain `data`, or `None` when this node holds no valid disclosure.
    pub data: Option<Value>,
    pub verified: bool,
    /// False when this node does not know the extension, or the step does
    /// not pass its manifest.
    pub checked: bool,
}

#[derive(Debug, Serialize)]
pub struct StepListOutput {
    pub steps: Vec<StepView>,
}

fn refusal(error: &StepError) -> Response {
    Response::invalid_params(error.to_string()).with_data(json!({ "code": error.code() }))
}

async fn load_agreement<H: AppHost>(host: &H, agreement: &str) -> Result<AgreementRow, Response> {
    match get_row(host, AGREEMENTS, agreement).await {
        Ok(Some(a)) => Ok(a),
        Ok(None) => Err(Response::invalid_params("no-such-agreement")),
        Err(e) => Err(Response::internal_error(e)),
    }
}

pub(crate) async fn load_step_row<H: AppHost>(
    host: &H,
    key: &str,
) -> Result<Option<StepRow>, String> {
    get_row(host, STEPS, key).await
}

pub(crate) async fn count_steps<H: AppHost>(host: &H, agreement: &str) -> Result<usize, String> {
    let filter = json!({ "agreement": agreement }).to_string();
    paging::count(host, STEPS, Some(filter)).await
}

/// What the commitment says about a new step.
struct StepTarget {
    extension: ExtensionRef,
    role: SettlementRole,
}

/// The commitment exists, names an extension and is not final. The signer's
/// role in it follows from who gives it.
fn step_target(
    agr: &AgreementRow,
    commitment_id: &str,
    owner: &str,
) -> Result<StepTarget, Response> {
    let c = agr
        .terms
        .commitments
        .iter()
        .find(|c| c.commitment_id == commitment_id)
        .ok_or_else(|| Response::invalid_params("no-such-commitment"))?;
    let extension = c
        .extension
        .clone()
        .ok_or_else(|| Response::invalid_params("commitment-has-no-extension"))?;
    let is_ended = agr
        .commitments
        .iter()
        .find(|p| p.commitment_id == commitment_id)
        .is_some_and(|p| is_final(p.state));
    if is_ended {
        return Err(Response::invalid_params("commitment-final"));
    }
    let party = parties::agreement_party(agr, owner)?;
    Ok(StepTarget { extension, role: parties::settlement_role(party, c.giver) })
}

pub(crate) async fn add<H: AppHost>(host: &H, input: StepAddInput) -> Reply<HalfRecordedOutput> {
    add_at(host, input, clock::now_secs()).await
}

pub(crate) async fn add_at<H: AppHost>(
    host: &H,
    input: StepAddInput,
    now: u64,
) -> Reply<HalfRecordedOutput> {
    add_in(host, input, now, extension::registry()).await
}

/// `add_at` against a chosen registry, so a test can use a manifest that does
/// not ship.
pub(crate) async fn add_in<H: AppHost>(
    host: &H,
    input: StepAddInput,
    now: u64,
    registry: &Registry,
) -> Reply<HalfRecordedOutput> {
    let (principal, owner) = resolve_principal_and_owner(host, now).await?;
    let agr = load_agreement(host, &input.agreement).await?;
    parties::require_open_agreement(&agr)?;
    let deal_role = parties::agreement_party(&agr, &owner)?;
    let target = step_target(&agr, &input.commitment_id, &owner)?;

    let key = step_id(&input.agreement, &input.commitment_id, &input.step_type, target.role);
    if let Some(existing) = load_step_row(host, &key).await.map_err(Response::internal_error)? {
        return answer_existing(host, existing, deal_role, now).await;
    }

    match step::check_step(registry, &target.extension, &input.step_type, target.role, &input.data)
    {
        Ok(StepCheck::Checked) => {}
        Ok(StepCheck::NotChecked) => {
            let unknown = DealError::ExtensionUnknown(target.extension.id.clone());
            return Err(Response::invalid_params(unknown.to_string())
                .with_data(json!({ "code": unknown.code() })));
        }
        Err(e) => return Err(refusal(&e)),
    }
    let held = count_steps(host, &input.agreement).await.map_err(Response::internal_error)?;
    if held >= MAX_STEPS_PER_AGREEMENT {
        return Err(refusal(&StepError::TooManySteps));
    }

    let signed = sign_step(host, &agr, &input, &target, principal).await?;
    disclosures::store(host, &signed.record_id, RECORD_STEP, &signed.disclosures, now)
        .await
        .map_err(Response::internal_error)?;

    let row = StepRow {
        agreement: input.agreement.clone(),
        commitment_id: input.commitment_id.clone(),
        conversation: agr.conversation.clone(),
        step_type: input.step_type.clone(),
        role: target.role,
        half: ReceiptHalf {
            record_id: signed.record_id.clone(),
            envelope: signed.envelope_json,
            issuer: signed.issuer,
            issued_at_secs: signed.issued_at_secs,
        },
        state: SendState::Pending,
    };
    if let Some(winner) =
        claim_step_row(host, &key, &row).await.map_err(Response::internal_error)?
    {
        if winner.half.record_id != signed.record_id {
            let _ = disclosures::delete(host, &signed.record_id).await;
        }
        return Ok(HalfRecordedOutput {
            record_id: winner.half.record_id,
            role: deal_role,
            message_id: None,
            state: SendState::AlreadyRecorded,
        });
    }
    send_step(host, row, &signed.disclosures, deal_role, now).await
}

struct SignedStep {
    envelope_json: String,
    record_id: String,
    issuer: String,
    issued_at_secs: u64,
    disclosures: Vec<String>,
}

async fn sign_step<H: AppHost>(
    host: &H,
    agr: &AgreementRow,
    input: &StepAddInput,
    target: &StepTarget,
    principal: Principal,
) -> Result<SignedStep, Response> {
    let hidden = StepHidden { data: input.data.clone() };
    hidden.validate().map_err(|e| refusal(&e))?;
    let mut salts = disclosures::fresh_salts(host, 1).await?.into_iter();
    let (sd, disclosures) = conceal_hidden(&hidden, &STEP_SET, &mut salts)
        .map_err(|e| Response::invalid_params(e.to_string()))?;
    let payload = StepPayload {
        agreement: input.agreement.clone(),
        conversation: agr.conversation.clone(),
        commitment_id: input.commitment_id.clone(),
        counterparty_did: agr.counterparty_did.clone(),
        quote_author_did: agr.quote_author_did.clone(),
        role: target.role,
        extension: target.extension.clone(),
        step_type: input.step_type.clone(),
        sd,
    };
    payload.validate().map_err(|e| refusal(&e))?;

    let draft = RecordDraft {
        version: STEP_VERSION,
        record_type: RECORD_STEP.to_string(),
        subject: input.agreement.clone(),
        payload: serde_json::to_string(&payload)
            .map_err(|e| Response::internal_error(e.to_string()))?,
        expires_at_secs: None,
        supersedes: None,
    };
    let envelope_json = AppSigning::sign_record(host, draft, principal)
        .await
        .map_err(|e| Response::invalid_params(e.to_string()))?;
    let env =
        Envelope::from_json(&envelope_json).map_err(|e| Response::internal_error(e.to_string()))?;
    let record_id = env.record_id().map_err(|e| Response::internal_error(e.to_string()))?;
    Ok(SignedStep {
        envelope_json,
        record_id,
        issuer: env.issuer,
        issued_at_secs: env.issued_at_secs,
        disclosures,
    })
}

/// Writes the row. If another call already wrote one for this key, that row
/// is returned and nothing is written.
pub(crate) async fn claim_step_row<H: AppHost>(
    host: &H,
    key: &str,
    row: &StepRow,
) -> Result<Option<StepRow>, String> {
    let payload = serde_json::to_vec(row).map_err(|e| e.to_string())?;
    let write = RecordWriteValue { id: key.to_string(), payload };
    match AppDataLayer::create(host, STEPS.to_string(), vec![write]).await {
        Ok(None) => Ok(None),
        Ok(Some(_)) => match load_step_row(host, key).await? {
            Some(winner) => Ok(Some(winner)),
            None => Err(format!("step row '{key}' vanished")),
        },
        Err(e) => Err(e.to_string()),
    }
}

/// Sends the card, then records how the send went.
async fn send_step<H: AppHost>(
    host: &H,
    mut row: StepRow,
    disclosures: &[String],
    deal_role: DealRole,
    now: u64,
) -> Reply<HalfRecordedOutput> {
    let card = OutgoingCard {
        card_type: "step",
        version: STEP_VERSION,
        envelope: &row.half.envelope,
        send: disclosures,
        view: disclosures,
    };
    let (message_id, state, _) =
        send_card_and_file(host, &row.conversation, &card, now, None).await;
    if state != row.state {
        row.state = state;
        let key = step_id(&row.agreement, &row.commitment_id, &row.step_type, row.role);
        put_row(host, STEPS, &key, &row).await.map_err(Response::internal_error)?;
    }
    Ok(HalfRecordedOutput {
        record_id: row.half.record_id,
        role: deal_role,
        message_id: Some(message_id),
        state,
    })
}

/// A repeat of a step that is already stored. A step whose first send failed
/// is sent again; any other repeat only answers with the stored record.
async fn answer_existing<H: AppHost>(
    host: &H,
    row: StepRow,
    deal_role: DealRole,
    now: u64,
) -> Reply<HalfRecordedOutput> {
    if row.state != SendState::NotSent {
        return Ok(HalfRecordedOutput {
            record_id: row.half.record_id,
            role: deal_role,
            message_id: None,
            state: SendState::AlreadyRecorded,
        });
    }
    let stored = disclosures::load(host, &row.half.record_id)
        .await
        .map_err(Response::internal_error)?
        .ok_or_else(|| {
            Response::internal_error(format!("disclosures missing for {}", row.half.record_id))
        })?;
    send_step(host, row, &stored, deal_role, now).await
}

pub(crate) async fn list<H: AppHost>(host: &H, input: StepListInput) -> Reply<StepListOutput> {
    load_agreement(host, &input.agreement).await?;
    let filter = json!({ "agreement": input.agreement }).to_string();
    let mut rows: Vec<StepRow> =
        paging::query_all(host, STEPS, Some(filter)).await.map_err(Response::internal_error)?;
    rows.sort_by(|a, b| {
        (a.half.issued_at_secs, &a.half.record_id).cmp(&(b.half.issued_at_secs, &b.half.record_id))
    });
    let now = clock::now_secs();
    let mut steps = Vec::with_capacity(rows.len());
    for row in rows {
        steps.push(view_of(host, row, now).await?);
    }
    Ok(StepListOutput { steps })
}

async fn view_of<H: AppHost>(host: &H, row: StepRow, now: u64) -> Result<StepView, Response> {
    let verdict = step::verify_step(&row.half.envelope, now);
    let stored =
        disclosures::load(host, &row.half.record_id).await.map_err(Response::internal_error)?;
    let payload = verdict.payload.as_ref();
    let data = match (payload, stored) {
        (Some(p), Some(d)) => {
            reveal_hidden_strict::<StepHidden>(&p.sd, &STEP_SET, &d).ok().map(|h| h.data)
        }
        _ => None,
    };
    let extension = payload
        .map(|p| p.extension.clone())
        .unwrap_or(ExtensionRef { id: String::new(), version: 0 });
    let checked = data.as_ref().is_some_and(|d| {
        matches!(
            step::check_step(extension::registry(), &extension, &row.step_type, row.role, d),
            Ok(StepCheck::Checked)
        )
    });
    Ok(StepView {
        record_id: row.half.record_id,
        envelope: row.half.envelope,
        commitment_id: row.commitment_id,
        step_type: row.step_type,
        extension,
        role: row.role,
        issuer: row.half.issuer,
        issued_at_secs: row.half.issued_at_secs,
        data,
        verified: verdict.verified,
        checked,
    })
}

pub(crate) async fn verify<H: AppHost>(
    _host: &H,
    input: DisclosedVerifyInput,
) -> Reply<DisclosedVerdict<StepPayload>> {
    let verdict = step::verify_step(&input.envelope, clock::now_secs());
    Ok(method::verify_disclosed(verdict, &input.disclosures, method::reveal_disclosed_step))
}
