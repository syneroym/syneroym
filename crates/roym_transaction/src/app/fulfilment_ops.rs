//! Fulfilment receipt signing, retrieval, and verification operations.

use serde::{Deserialize, Serialize};
use syneroym_app_host::{
    AppDataLayer, AppHost, AppSigning,
    types::{
        data_layer::RecordWriteValue,
        signing::{Principal, RecordDraft},
    },
};
use syneroym_roym_core::{
    clock,
    deal::{CommitmentKind, LocalText, SettlementRole, transition::CommitmentState},
    disclosure::{FULFILMENT_SET, conceal_hidden},
    envelope::Response,
    fulfilment::{self, FulfilmentHidden, FulfilmentReceiptPayload},
    method::{self, DisclosedVerdict, DisclosedVerifyInput, Reply},
    progress::AgreementEvent,
    record::{Envelope, RECORD_FULFILMENT_RECEIPT},
    transaction::{DealRole, ReceiptHalf},
};

use super::{
    AGREEMENTS, AgreementRow, FULFILMENTS, FenceOutcome, FulfilmentsRow, HalfRecordedOutput,
    LEDGER, OutgoingCard, SendState, disclosures, get_row,
    ledger::{LedgerKind, LedgerRow},
    parties, progress_ops, put_row, resolve_principal_and_owner, send_card_and_file,
};

pub(crate) fn fulfilment_id(agreement: &str, commitment_id: &str) -> String {
    format!("{agreement}:{commitment_id}")
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FulfilmentSignInput {
    pub agreement: String,
    pub commitment_id: String,
    #[serde(default)]
    pub settlement_note: Option<LocalText>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FulfilmentKeyInput {
    pub agreement: String,
    pub commitment_id: String,
}

#[derive(Debug, Serialize)]
pub struct FulfilmentGetOutput {
    pub counterparty: Option<ReceiptHalf>,
    pub quote_author: Option<ReceiptHalf>,
    pub track: CommitmentState,
}

async fn load_fulfilments_row<H: AppHost>(
    host: &H,
    f_id: &str,
    agreement: &str,
    commitment_id: &str,
    conversation: &str,
    now: u64,
) -> Result<FulfilmentsRow, Response> {
    match get_row(host, FULFILMENTS, f_id).await {
        Ok(Some(f)) => Ok(f),
        Ok(None) => Ok(FulfilmentsRow {
            agreement: agreement.to_string(),
            commitment_id: commitment_id.to_string(),
            conversation: conversation.to_string(),
            counterparty: None,
            quote_author: None,
            updated_at_secs: now,
        }),
        Err(e) => Err(Response::internal_error(e)),
    }
}

pub(crate) async fn sign<H: AppHost>(
    host: &H,
    input: FulfilmentSignInput,
) -> Reply<HalfRecordedOutput> {
    sign_at(host, input, clock::now_secs()).await
}

pub(crate) async fn sign_at<H: AppHost>(
    host: &H,
    input: FulfilmentSignInput,
    now: u64,
) -> Reply<HalfRecordedOutput> {
    let (principal, owner) = resolve_principal_and_owner(host, now).await?;

    let agr: AgreementRow = match get_row(host, AGREEMENTS, &input.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(Response::invalid_params("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let (deal_role, settlement_role) =
        check_fulfilment_preconditions(&agr, &input.commitment_id, &owner)?;

    let f_id = fulfilment_id(&input.agreement, &input.commitment_id);
    let fulfilments = load_fulfilments_row(
        host,
        &f_id,
        &input.agreement,
        &input.commitment_id,
        &agr.conversation,
        now,
    )
    .await?;

    let existing = match deal_role {
        DealRole::Counterparty => &fulfilments.counterparty,
        DealRole::QuoteAuthor => &fulfilments.quote_author,
    };
    if let Some(half) = existing {
        let record_id = half.record_id.clone();
        let author_ctx = FulfilmentAuthorContext {
            agreement: &input.agreement,
            commitment_id: &input.commitment_id,
            role: settlement_role,
            record_id: &record_id,
            owner: &owner,
            quote_author_did: &agr.quote_author_did,
            now,
        };
        return already_recorded_fulfilment_response(host, deal_role, author_ctx).await;
    }

    let (payload, disc) = prepare_fulfilment_payload(host, &input, &agr, settlement_role).await?;

    let (env_json, env, record_id) =
        sign_fulfilment_receipt(host, &input.agreement, &payload, principal).await?;

    let half = ReceiptHalf {
        record_id: record_id.clone(),
        envelope: env_json,
        issuer: env.issuer,
        issued_at_secs: env.issued_at_secs,
    };

    disclosures::store(host, &record_id, RECORD_FULFILMENT_RECEIPT, &disc, now)
        .await
        .map_err(Response::internal_error)?;

    let fence_claim = FulfilmentFenceClaim {
        agreement: &input.agreement,
        commitment_id: &input.commitment_id,
        conversation: &agr.conversation,
        deal_role,
        settlement_role,
        owner: &owner,
        quote_author_did: &agr.quote_author_did,
    };
    match claim_fulfilment_fence(host, fence_claim, &half, now).await? {
        FenceOutcome::Claimed => {}
        FenceOutcome::Answered(out) => return Ok(out),
    }

    let author_ctx = FulfilmentAuthorContext {
        agreement: &input.agreement,
        commitment_id: &input.commitment_id,
        role: settlement_role,
        record_id: &record_id,
        owner: &owner,
        quote_author_did: &agr.quote_author_did,
        now,
    };
    let send_ctx = FulfilmentSendContext { conversation: &agr.conversation, disclosures: &disc };
    record_and_send_fulfilment(host, &f_id, fulfilments, deal_role, half, send_ctx, author_ctx)
        .await
}

struct FulfilmentSendContext<'a> {
    conversation: &'a str,
    disclosures: &'a [String],
}

struct FulfilmentAuthorContext<'a> {
    agreement: &'a str,
    commitment_id: &'a str,
    role: SettlementRole,
    record_id: &'a str,
    owner: &'a str,
    quote_author_did: &'a str,
    now: u64,
}

async fn record_and_send_fulfilment<H: AppHost>(
    host: &H,
    f_id: &str,
    mut fulfilments: FulfilmentsRow,
    deal_role: DealRole,
    half: ReceiptHalf,
    send_ctx: FulfilmentSendContext<'_>,
    author_ctx: FulfilmentAuthorContext<'_>,
) -> Reply<HalfRecordedOutput> {
    let now = author_ctx.now;
    match deal_role {
        DealRole::Counterparty => fulfilments.counterparty = Some(half.clone()),
        DealRole::QuoteAuthor => fulfilments.quote_author = Some(half.clone()),
    }
    fulfilments.updated_at_secs = now;

    if let Err(e) = put_row(host, FULFILMENTS, f_id, &fulfilments).await {
        return Err(Response::internal_error(e));
    }

    let card = OutgoingCard {
        card_type: "fulfilment-receipt",
        version: 1,
        envelope: &half.envelope,
        send: send_ctx.disclosures,
        view: send_ctx.disclosures,
    };

    let (message_id, send_state, _) =
        send_card_and_file(host, send_ctx.conversation, &card, now, None).await;

    maybe_transition_author_fulfilment(host, &author_ctx).await;

    Ok(HalfRecordedOutput {
        record_id: half.record_id,
        role: deal_role,
        message_id: Some(message_id),
        state: send_state,
    })
}

async fn prepare_fulfilment_payload<H: AppHost>(
    host: &H,
    input: &FulfilmentSignInput,
    agr: &AgreementRow,
    settlement_role: SettlementRole,
) -> Result<(FulfilmentReceiptPayload, Vec<String>), Response> {
    let hidden = FulfilmentHidden { settlement_note: input.settlement_note.clone() };
    if let Err(e) = hidden.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }
    let mut salts = disclosures::fresh_salts(host, 1).await?.into_iter();
    let (sd, disc) = conceal_hidden(&hidden, &FULFILMENT_SET, &mut salts)
        .map_err(|e| Response::invalid_params(e.to_string()))?;

    let payload = FulfilmentReceiptPayload {
        agreement: input.agreement.clone(),
        conversation: agr.conversation.clone(),
        commitment_id: input.commitment_id.clone(),
        counterparty_did: agr.counterparty_did.clone(),
        quote_author_did: agr.quote_author_did.clone(),
        role: settlement_role,
        sd,
    };
    if let Err(e) = payload.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }
    Ok((payload, disc))
}

async fn maybe_transition_author_fulfilment<H: AppHost>(
    host: &H,
    ctx: &FulfilmentAuthorContext<'_>,
) {
    if ctx.owner == ctx.quote_author_did {
        let _ = progress_ops::transition_at(
            host,
            ctx.agreement,
            AgreementEvent::Half {
                commitment_id: ctx.commitment_id.to_string(),
                role: ctx.role,
                record_id: ctx.record_id.to_string(),
            },
            ctx.now,
        )
        .await;
    }
}

async fn already_recorded_fulfilment_response<H: AppHost>(
    host: &H,
    deal_role: DealRole,
    author_ctx: FulfilmentAuthorContext<'_>,
) -> Reply<HalfRecordedOutput> {
    maybe_transition_author_fulfilment(host, &author_ctx).await;
    Ok(HalfRecordedOutput {
        record_id: author_ctx.record_id.to_string(),
        role: deal_role,
        message_id: None,
        state: SendState::AlreadyRecorded,
    })
}

pub(crate) async fn get<H: AppHost>(
    host: &H,
    input: FulfilmentKeyInput,
) -> Reply<FulfilmentGetOutput> {
    let agr: AgreementRow = match get_row(host, AGREEMENTS, &input.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(Response::invalid_params("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let commitment =
        match agr.terms.commitments.iter().find(|c| c.commitment_id == input.commitment_id) {
            Some(c) => c,
            None => return Err(Response::invalid_params("no-such-commitment")),
        };
    if commitment.kind != CommitmentKind::Fulfilment {
        return Err(Response::invalid_params("not-fulfilment-commitment"));
    }

    let f_id = fulfilment_id(&input.agreement, &input.commitment_id);
    let fulfilments: FulfilmentsRow = match get_row(host, FULFILMENTS, &f_id).await {
        Ok(Some(f)) => f,
        Ok(None) => FulfilmentsRow {
            agreement: input.agreement.clone(),
            commitment_id: input.commitment_id.clone(),
            conversation: agr.conversation.clone(),
            counterparty: None,
            quote_author: None,
            updated_at_secs: agr.updated_at_secs,
        },
        Err(e) => return Err(Response::internal_error(e)),
    };

    let track = agr
        .commitments
        .iter()
        .find(|c| c.commitment_id == input.commitment_id)
        .map(|c| c.state)
        .unwrap_or(CommitmentState::Promised);

    Ok(FulfilmentGetOutput {
        counterparty: fulfilments.counterparty,
        quote_author: fulfilments.quote_author,
        track,
    })
}

fn check_fulfilment_preconditions(
    agr: &AgreementRow,
    commitment_id: &str,
    owner: &str,
) -> Result<(DealRole, SettlementRole), Response> {
    parties::require_open_agreement(agr)?;

    let c = agr
        .terms
        .commitments
        .iter()
        .find(|c| c.commitment_id == commitment_id)
        .ok_or_else(|| Response::invalid_params("no-such-commitment"))?;
    if c.kind != CommitmentKind::Fulfilment {
        return Err(Response::invalid_params("not-fulfilment-commitment"));
    }

    let deal_role = parties::agreement_party(agr, owner)?;
    Ok((deal_role, parties::settlement_role(deal_role, c.giver)))
}

async fn sign_fulfilment_receipt<H: AppHost>(
    host: &H,
    agreement: &str,
    payload: &FulfilmentReceiptPayload,
    principal: Principal,
) -> Result<(String, Envelope, String), Response> {
    let payload_str = match serde_json::to_string(payload) {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let draft = RecordDraft {
        version: fulfilment::FULFILMENT_RECEIPT_VERSION,
        record_type: RECORD_FULFILMENT_RECEIPT.to_string(),
        subject: agreement.to_string(),
        payload: payload_str,
        expires_at_secs: None,
        supersedes: None,
    };
    let env_json = match AppSigning::sign_record(host, draft, principal).await {
        Ok(j) => j,
        Err(e) => return Err(Response::invalid_params(e.to_string())),
    };
    let env = match Envelope::from_json(&env_json) {
        Ok(e) => e,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let record_id = match env.record_id() {
        Ok(id) => id,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    Ok((env_json, env, record_id))
}

pub(crate) struct FulfilmentFenceClaim<'a> {
    pub(crate) agreement: &'a str,
    pub(crate) commitment_id: &'a str,
    pub(crate) conversation: &'a str,
    pub(crate) deal_role: DealRole,
    pub(crate) settlement_role: SettlementRole,
    pub(crate) owner: &'a str,
    pub(crate) quote_author_did: &'a str,
}

pub(crate) async fn claim_fulfilment_fence<H: AppHost>(
    host: &H,
    claim: FulfilmentFenceClaim<'_>,
    half: &ReceiptHalf,
    now: u64,
) -> Result<FenceOutcome<HalfRecordedOutput>, Response> {
    let FulfilmentFenceClaim {
        agreement,
        commitment_id,
        conversation,
        deal_role,
        settlement_role,
        owner,
        quote_author_did,
    } = claim;
    let role_str = deal_role.as_str();
    let fence_key = format!("fulfil:{agreement}:{commitment_id}:{role_str}");
    let fence_row = LedgerRow {
        kind: LedgerKind::Fence,
        agreement: agreement.to_string(),
        state: None,
        quote_id: None,
        commitment_id: Some(commitment_id.to_string()),
        reason: None,
        half: Some(half.clone()),
        created_at_secs: now,
    };
    let fence_write = match serde_json::to_vec(&fence_row) {
        Ok(b) => RecordWriteValue { id: fence_key.clone(), payload: b },
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    match AppDataLayer::create(host, LEDGER.to_string(), vec![fence_write]).await {
        Ok(None) => Ok(FenceOutcome::Claimed),
        Ok(Some(_)) => {
            if let Ok(Some(fence)) = get_row::<LedgerRow, _>(host, LEDGER, &fence_key).await
                && let Some(existing) = &fence.half
            {
                let ctx = FulfilmentRecordCtx {
                    agreement,
                    commitment_id,
                    conversation,
                    deal_role,
                    settlement_role,
                    owner,
                    quote_author_did,
                };
                return ensure_fulfilment_recorded(host, ctx, existing, now)
                    .await
                    .map(FenceOutcome::Answered);
            }
            Err(Response::invalid_params("fulfilment-sign-in-flight"))
        }
        Err(e) => Err(Response::internal_error(e.to_string())),
    }
}

pub(crate) struct FulfilmentRecordCtx<'a> {
    pub(crate) agreement: &'a str,
    pub(crate) commitment_id: &'a str,
    pub(crate) conversation: &'a str,
    pub(crate) deal_role: DealRole,
    pub(crate) settlement_role: SettlementRole,
    pub(crate) owner: &'a str,
    pub(crate) quote_author_did: &'a str,
}

pub(crate) async fn ensure_fulfilment_recorded<H: AppHost>(
    host: &H,
    ctx: FulfilmentRecordCtx<'_>,
    half: &ReceiptHalf,
    now: u64,
) -> Reply<HalfRecordedOutput> {
    let FulfilmentRecordCtx {
        agreement,
        commitment_id,
        conversation,
        deal_role,
        settlement_role,
        owner,
        quote_author_did,
    } = ctx;
    let f_id = fulfilment_id(agreement, commitment_id);
    let mut f: FulfilmentsRow = match get_row(host, FULFILMENTS, &f_id).await {
        Ok(Some(row)) => row,
        Ok(None) => FulfilmentsRow {
            agreement: agreement.to_string(),
            commitment_id: commitment_id.to_string(),
            conversation: conversation.to_string(),
            counterparty: None,
            quote_author: None,
            updated_at_secs: now,
        },
        Err(e) => return Err(Response::internal_error(e)),
    };
    let slot = match deal_role {
        DealRole::Counterparty => &mut f.counterparty,
        DealRole::QuoteAuthor => &mut f.quote_author,
    };
    let already_present = slot.as_ref().is_some_and(|h| h.record_id == half.record_id);
    if !already_present {
        *slot = Some(half.clone());
        f.updated_at_secs = now;
        if let Err(e) = put_row(host, FULFILMENTS, &f_id, &f).await {
            return Err(Response::internal_error(e));
        }
    }

    let author_ctx = FulfilmentAuthorContext {
        agreement,
        commitment_id,
        role: settlement_role,
        record_id: &half.record_id,
        owner,
        quote_author_did,
        now,
    };
    maybe_transition_author_fulfilment(host, &author_ctx).await;

    if already_present {
        return Ok(HalfRecordedOutput {
            record_id: half.record_id.clone(),
            role: deal_role,
            message_id: None,
            state: SendState::AlreadyRecorded,
        });
    }
    let stored_disc = disclosures::load(host, &half.record_id)
        .await
        .map_err(Response::internal_error)?
        .ok_or_else(|| {
            Response::internal_error(format!("disclosures missing for {}", half.record_id))
        })?;
    let card = OutgoingCard {
        card_type: "fulfilment-receipt",
        version: 1,
        envelope: &half.envelope,
        send: &stored_disc,
        view: &stored_disc,
    };
    let (message_id, send_state, _) =
        send_card_and_file(host, conversation, &card, now, None).await;
    Ok(HalfRecordedOutput {
        record_id: half.record_id.clone(),
        role: deal_role,
        message_id: Some(message_id),
        state: send_state,
    })
}

pub(crate) async fn verify<H: AppHost>(
    _host: &H,
    input: DisclosedVerifyInput,
) -> Reply<DisclosedVerdict<FulfilmentReceiptPayload>> {
    let now = clock::now_secs();
    let verdict = fulfilment::verify_fulfilment_receipt(&input.envelope, now);
    Ok(method::verify_disclosed(verdict, &input.disclosures, method::reveal_disclosed_fulfilment))
}
