//! Answering one as-listed enquiry on the owner's node.

use serde::{Deserialize, Serialize};
use serde_json::json;
use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    as_listed::{self, AsListed},
    clock,
    deal::DealTerms,
    envelope::Response,
    listing::{self, ListingPayload, ListingStatus},
    method::{
        Reply, SiblingError, call_sibling,
        catalog::{ListingGetOutput, ListingKeyInput},
    },
    paging,
    progress::FormedAs,
    record::Envelope,
    services,
    signing::{self, CertificateError},
    transaction::{self, EnquiryPayload, RECORD_QUOTE},
};

use super::{
    super::{
        ENQUIRIES, ENQUIRY_HISTORY, LEDGER, OutgoingCard, QUOTES, RecordPointerRow, count_mine,
        decide, decline_ops, disclosures, ensure_collections, get_bytes, get_row, hooks,
        is_deal_ended,
        ledger::LedgerKind,
        put_row,
        quote_ops::{IssueQuote, QuoteSetOutput, issue_quote},
        resolve_principal_and_owner, send_card_and_file,
    },
    WAITING_ANSWER_IN_PROGRESS, WAITING_CAP_REACHED, WAITING_CONFIRM_FIRST,
    WAITING_COUNTERPARTY_HOLDS_ONE, WAITING_SIGNING_NOT_READY, WAITING_SLOT_CHECK_FAILED,
    open_instant,
};

/// The owner's Hub calls this to answer a waiting tap by hand.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FromListingInput {
    pub enquiry_record_id: String,
}

/// Who asked for the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trigger {
    /// The node's own driver, inside `transaction.sync` or `transaction.drive`.
    Driver,
    /// The owner pressed "Confirm".
    Owner,
}

/// What one answer attempt did.
pub(crate) enum Outcome {
    Quoted(QuoteSetOutput),
    /// Nothing was written. The reason is one the owner's Hub shows.
    Waiting(&'static str),
    Declined,
    /// The enquiry is not an as-listed one.
    NotAsListed,
}

/// The ledger row that keeps one tap to at most one quote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AnswerRow {
    pub(crate) kind: LedgerKind,
    pub(crate) enquiry_record_id: String,
    pub(crate) conversation: String,
    pub(crate) listing_id: String,
    pub(crate) quote_id: String,
    /// The quote sequence the id was derived from, kept so a retry after a
    /// crash signs the same record.
    pub(crate) sequence: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) quote_record_id: Option<String>,
    pub(crate) formed_as: FormedAs,
    pub(crate) state: AnswerState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) code: Option<String>,
    pub(crate) created_at_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AnswerState {
    Answering,
    Answered,
    Declined,
}

pub(crate) fn answer_id(enquiry_record_id: &str) -> String {
    format!("answer:{enquiry_record_id}")
}

/// `quote.from-listing` for the owner's Hub.
pub(crate) async fn answer_from_listing<H: AppHost>(
    host: &H,
    input: FromListingInput,
) -> Reply<QuoteSetOutput> {
    answer_from_listing_at(host, input, clock::now_secs()).await
}

pub(crate) async fn answer_from_listing_at<H: AppHost>(
    host: &H,
    input: FromListingInput,
    now: u64,
) -> Reply<QuoteSetOutput> {
    // The Hub call must fail the way every signing verb fails, so resolve the
    // person first.
    let _ = resolve_principal_and_owner(host, now).await?;
    match answer_as_listed(host, &input.enquiry_record_id, Trigger::Owner, now).await? {
        Outcome::Quoted(out) => Ok(out),
        Outcome::Waiting(reason) => Err(Response::invalid_params(format!(
            "the offering cannot answer this tap yet: {reason}"
        ))
        .with_data(json!({ "code": "as-listed-waiting", "reason": reason }))),
        Outcome::Declined => Err(Response::invalid_params("the tap was answered with a decline")),
        Outcome::NotAsListed => {
            Err(Response::invalid_params("this enquiry does not agree to a listing as listed"))
        }
    }
}

/// Answers one tap. A `Waiting` outcome writes nothing, so the next drive
/// tries again.
pub(crate) async fn answer_as_listed<H: AppHost>(
    host: &H,
    enquiry_record_id: &str,
    trigger: Trigger,
    now: u64,
) -> Result<Outcome, Response> {
    ensure_collections(host).await.map_err(Response::internal_error)?;
    let node_owner = signing::owner_did(host).await.map_err(certificate_error)?;
    let (enquiry, conversation, counterparty) =
        load_held_enquiry(host, enquiry_record_id, now).await?;
    let (Some(consent), Some(listing_id)) = (&enquiry.as_listed, &enquiry.listing_id) else {
        return Ok(Outcome::NotAsListed);
    };
    if counterparty == node_owner {
        return Ok(Outcome::NotAsListed);
    }
    let start =
        start(host, enquiry_record_id, listing_id, &conversation, consent, &node_owner, now)
            .await?;
    let (current, consent) = match start {
        Start::Done(outcome) => return Ok(outcome),
        Start::Declined => return Ok(Outcome::Declined),
        Start::Resume { answer, current, consent } => {
            return resume(host, &answer, &counterparty, current, consent, now).await;
        }
        Start::Fresh { current, consent } => (current, consent),
    };

    let verdict =
        evaluate(host, &current.payload, &consent, &current.owner, &counterparty, trigger, now)
            .await?;
    let (terms, formed_as) = match verdict {
        Verdict::Answer { terms, formed_as } => (*terms, formed_as),
        Verdict::Wait(reason) => return Ok(Outcome::Waiting(reason)),
        Verdict::Decline(code) => {
            return decline(host, enquiry_record_id, listing_id, &conversation, code, now).await;
        }
    };

    let sequence = count_mine(host, QUOTES, &conversation, &current.owner)
        .await
        .map_err(Response::internal_error)?
        + 1;
    let quote_id = transaction::derive_quote_id(&conversation, &current.owner, sequence)
        .map_err(|e| Response::internal_error(e.to_string()))?;
    let answer = AnswerRow {
        kind: LedgerKind::Answer,
        enquiry_record_id: enquiry_record_id.to_string(),
        conversation: conversation.clone(),
        listing_id: listing_id.clone(),
        quote_id: quote_id.clone(),
        sequence,
        quote_record_id: None,
        formed_as,
        state: AnswerState::Answering,
        code: None,
        created_at_secs: now,
    };
    if !claim_answer(host, &answer).await? {
        if let Some(row) = get_row::<AnswerRow, _>(host, LEDGER, &answer_id(enquiry_record_id))
            .await
            .map_err(Response::internal_error)?
        {
            return match row.state {
                AnswerState::Answered => resend(host, &row, now).await,
                AnswerState::Declined => Ok(Outcome::Declined),
                AnswerState::Answering => Ok(Outcome::Waiting(WAITING_ANSWER_IN_PROGRESS)),
            };
        }
        return Ok(Outcome::Waiting(WAITING_ANSWER_IN_PROGRESS));
    }

    let principal = signing::person_principal(host, now).await.map_err(certificate_error)?.0;
    let expires_at_secs = terms.quote_expires_at_secs;
    let out = issue_quote(
        host,
        IssueQuote {
            principal,
            owner: &current.owner,
            enquiry_record_id,
            quote_id: &quote_id,
            conversation: &conversation,
            counterparty_did: &counterparty,
            terms,
            sequence,
            supersedes: None,
            next_count: 1,
            formed_as: Some(formed_as),
            expires_at_secs,
            now,
        },
    )
    .await?;

    mark_answered(host, &answer, &out.record_id).await?;
    Ok(Outcome::Quoted(out))
}

/// Finishes an answer that reached its fence but not its quote. The quote id
/// and the formed-as value are the stored ones, so a retry cannot turn the
/// same tap into a second answer or change how it was formed.
async fn resume<H: AppHost>(
    host: &H,
    answer: &AnswerRow,
    counterparty: &str,
    current: Box<CurrentListing>,
    consent: AsListed,
    now: u64,
) -> Result<Outcome, Response> {
    let terms = match as_listed::terms_from_listing(&current.payload, &current.owner, &consent, now)
    {
        Ok(terms) => terms,
        Err(reason) => return Ok(Outcome::Waiting(reason.code())),
    };
    let principal = signing::person_principal(host, now).await.map_err(certificate_error)?.0;
    let expires_at_secs = terms.quote_expires_at_secs;
    let out = issue_quote(
        host,
        IssueQuote {
            principal,
            owner: &current.owner,
            enquiry_record_id: &answer.enquiry_record_id,
            quote_id: &answer.quote_id,
            conversation: &answer.conversation,
            counterparty_did: counterparty,
            terms,
            sequence: answer.sequence,
            supersedes: None,
            next_count: 1,
            formed_as: Some(answer.formed_as),
            expires_at_secs,
            now,
        },
    )
    .await?;
    mark_answered(host, answer, &out.record_id).await?;
    Ok(Outcome::Quoted(out))
}

/// What is already known about this tap, before any term is built.
enum Start {
    Done(Outcome),
    Declined,
    /// The answer reached its fence; the stored row's id, sequence and
    /// formed-as must be reused.
    Resume {
        answer: AnswerRow,
        current: Box<CurrentListing>,
        consent: AsListed,
    },
    Fresh {
        current: Box<CurrentListing>,
        consent: AsListed,
    },
}

async fn start<H: AppHost>(
    host: &H,
    enquiry_record_id: &str,
    listing_id: &str,
    conversation: &str,
    consent: &AsListed,
    node_owner: &str,
    now: u64,
) -> Result<Start, Response> {
    let existing: Option<AnswerRow> = get_row(host, LEDGER, &answer_id(enquiry_record_id))
        .await
        .map_err(Response::internal_error)?;
    let resume = match &existing {
        None => None,
        Some(row) => match row.state {
            AnswerState::Answered => return Ok(Start::Done(resend(host, row, now).await?)),
            AnswerState::Declined => return Ok(Start::Done(Outcome::Declined)),
            AnswerState::Answering => {
                if let Some(ptr) = get_row::<RecordPointerRow, _>(host, QUOTES, &row.quote_id)
                    .await
                    .map_err(Response::internal_error)?
                {
                    mark_answered(host, row, &ptr.record_id).await?;
                    let row = answered(row, &ptr.record_id);
                    return Ok(Start::Done(resend(host, &row, now).await?));
                }
                Some(row.clone())
            }
        },
    };

    if now > consent.valid_until_secs {
        if let Some(row) = &resume {
            end_answer(host, row, "consent-expired").await?;
            return Ok(Start::Done(Outcome::Declined));
        }
        decline(host, enquiry_record_id, listing_id, conversation, "consent-expired", now).await?;
        return Ok(Start::Declined);
    }

    let Some(current) = load_current_listing(host, listing_id, now).await? else {
        if resume.is_some() {
            return Ok(Start::Done(Outcome::Waiting("offering-withdrawn")));
        }
        decline(host, enquiry_record_id, listing_id, conversation, "offering-withdrawn", now)
            .await?;
        return Ok(Start::Declined);
    };
    if current.owner != node_owner {
        return Ok(Start::Done(Outcome::NotAsListed));
    }
    if current.payload.status != ListingStatus::Active {
        if resume.is_some() {
            return Ok(Start::Done(Outcome::Waiting("offering-withdrawn")));
        }
        decline(host, enquiry_record_id, listing_id, conversation, "offering-withdrawn", now)
            .await?;
        return Ok(Start::Declined);
    }

    let consent = AsListed {
        record_id: current.record_id.clone(),
        slot: consent.slot.clone(),
        variant: None,
        valid_until_secs: consent.valid_until_secs,
    };
    let current = Box::new(current);
    match resume {
        Some(answer) => Ok(Start::Resume { answer, current, consent }),
        None => Ok(Start::Fresh { current, consent }),
    }
}

/// What one attempt on a verified offering decides.
pub(super) enum Verdict {
    Answer { terms: Box<DealTerms>, formed_as: FormedAs },
    Wait(&'static str),
    Decline(&'static str),
}

pub(super) async fn evaluate<H: AppHost>(
    host: &H,
    listing: &ListingPayload,
    consent: &AsListed,
    owner: &str,
    counterparty: &str,
    trigger: Trigger,
    now: u64,
) -> Result<Verdict, Response> {
    let terms = match as_listed::terms_from_listing(listing, owner, consent, now) {
        Ok(terms) => terms,
        Err(reason) => return Ok(Verdict::Wait(reason.code())),
    };
    match check_author_slots(host, &terms).await? {
        SlotCheck::Ok => {}
        SlotCheck::Refused(code) => return Ok(Verdict::Decline(code)),
        SlotCheck::Unavailable => return Ok(Verdict::Wait(WAITING_SLOT_CHECK_FAILED)),
    }

    if trigger == Trigger::Owner {
        return Ok(Verdict::Answer {
            terms: Box::new(terms),
            formed_as: FormedAs::AsListedConfirmed,
        });
    }
    if listing.acceptance == listing::Acceptance::ConfirmFirst {
        return Ok(Verdict::Wait(WAITING_CONFIRM_FIRST));
    }
    let listing_id = listing.listing_id.as_str();
    let open =
        open_instant(host, listing_id, now).await.map_err(Response::internal_error)?.len() as u32;
    let in_flight =
        in_flight_instant(host, listing_id, now).await.map_err(Response::internal_error)?;
    if open + in_flight >= u32::from(listing.instant_cap) {
        return Ok(Verdict::Wait(WAITING_CAP_REACHED));
    }
    if holds_one(host, listing_id, counterparty, now).await.map_err(Response::internal_error)? {
        return Ok(Verdict::Wait(WAITING_COUNTERPARTY_HOLDS_ONE));
    }
    if signing::person_principal(host, now).await.is_err() {
        return Ok(Verdict::Wait(WAITING_SIGNING_NOT_READY));
    }
    Ok(Verdict::Answer { terms: Box::new(terms), formed_as: FormedAs::AsListedInstant })
}

/// The instant quotes that still hold a place for `listing_id`: answered
/// rows whose quote has no decision fence and has not ended. A quote that
/// formed an agreement is counted by the agreement, not here.
async fn in_flight_rows<H: AppHost>(
    host: &H,
    listing_id: &str,
    now: u64,
) -> Result<Vec<AnswerRow>, String> {
    let filter = json!({ "kind": "answer" }).to_string();
    let rows: Vec<AnswerRow> = paging::query_all(host, LEDGER, Some(filter)).await?;
    let mut in_flight = Vec::new();
    for row in rows {
        if row.listing_id != listing_id
            || row.formed_as != FormedAs::AsListedInstant
            || row.state != AnswerState::Answered
        {
            continue;
        }
        if get_row::<decide::DecisionRow, _>(host, LEDGER, &decide::decision_id(&row.quote_id))
            .await?
            .is_some()
        {
            continue;
        }
        if is_deal_ended(host, &row.quote_id, None, now).await? {
            continue;
        }
        in_flight.push(row);
    }
    Ok(in_flight)
}

/// Counts quotes already issued for a listing that wait for their answer.
pub(crate) async fn in_flight_instant<H: AppHost>(
    host: &H,
    listing_id: &str,
    now: u64,
) -> Result<u32, String> {
    Ok(in_flight_rows(host, listing_id, now).await?.len() as u32)
}

async fn holds_one<H: AppHost>(
    host: &H,
    listing_id: &str,
    counterparty: &str,
    now: u64,
) -> Result<bool, String> {
    if open_instant(host, listing_id, now)
        .await?
        .iter()
        .any(|row| row.counterparty_did == counterparty)
    {
        return Ok(true);
    }
    for row in in_flight_rows(host, listing_id, now).await? {
        if let Some(ptr) = get_row::<RecordPointerRow, _>(host, QUOTES, &row.quote_id).await?
            && ptr.counterparty_did.as_deref() == Some(counterparty)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The owner's own listing, as the catalog holds it, with the verified
/// payload and the consent time the quote may use.
pub(super) struct CurrentListing {
    pub(super) payload: ListingPayload,
    pub(super) record_id: String,
    pub(super) owner: String,
}

pub(super) async fn load_current_listing<H: AppHost>(
    host: &H,
    listing_id: &str,
    now: u64,
) -> Result<Option<CurrentListing>, Response> {
    let input = ListingKeyInput { listing_id: listing_id.to_string() };
    let found: Option<ListingGetOutput> =
        match call_sibling(host, services::CATALOG, "listing.get", &input).await {
            Ok(found) => found,
            Err(SiblingError::Refused(_)) => None,
            Err(e) => return Err(Response::internal_error(e.to_string())),
        };
    let Some(found) = found else { return Ok(None) };
    let verdict = listing::verify_envelope(&found.envelope, now);
    let (Some(payload), Some(owner)) = (verdict.payload, verdict.issuer) else {
        return Err(Response::internal_error(
            "the catalog returned an offering that does not verify",
        ));
    };
    Ok(Some(CurrentListing { payload, record_id: found.record_id, owner }))
}

async fn load_held_enquiry<H: AppHost>(
    host: &H,
    enquiry_record_id: &str,
    now: u64,
) -> Result<(EnquiryPayload, String, String), Response> {
    let bytes = get_bytes(host, ENQUIRY_HISTORY, enquiry_record_id)
        .await
        .map_err(Response::internal_error)?
        .ok_or_else(|| Response::invalid_params("no such enquiry"))?;
    let envelope = String::from_utf8(bytes).map_err(|e| Response::internal_error(e.to_string()))?;
    let verdict = transaction::verify_enquiry(&envelope, now);
    let (Some(payload), Some(issuer)) = (verdict.payload, verdict.issuer) else {
        return Err(Response::invalid_params("the enquiry does not verify"));
    };
    Ok((payload.clone(), payload.conversation, issuer))
}

enum SlotCheck {
    Ok,
    Refused(&'static str),
    Unavailable,
}

/// Runs the extension's `prepare` hook for each slot the owner gives, and
/// compares the window it answers with the one the tap carries.
async fn check_author_slots<H: AppHost>(
    host: &H,
    terms: &DealTerms,
) -> Result<SlotCheck, Response> {
    for commitment in &terms.commitments {
        if commitment.giver != transaction::DealRole::QuoteAuthor
            || !hooks::has_prepare_hook(commitment.extension.as_ref())
        {
            continue;
        }
        let Some(details) = commitment.details.clone() else {
            return Ok(SlotCheck::Refused("slot-not-in-listing"));
        };
        let answer = hooks::prepare_window(
            host,
            commitment.extension.as_ref(),
            details,
            commitment.window,
            terms.listing_id.as_deref(),
        )
        .await;
        match answer {
            Ok(window) if Some(window) == commitment.window => {}
            Ok(_) => return Ok(SlotCheck::Refused("slot-not-in-listing")),
            Err(response) => return Ok(slot_refusal(response)),
        }
    }
    Ok(SlotCheck::Ok)
}

fn slot_refusal(response: Response) -> SlotCheck {
    let Some(error) = response.error else { return SlotCheck::Unavailable };
    if error.code != -32602 {
        return SlotCheck::Unavailable;
    }
    let code = error.data.as_ref().and_then(|data| data.get("code")).and_then(|code| code.as_str());
    match code {
        Some("no-such-slot") => SlotCheck::Refused("no-such-slot"),
        Some("slot-not-in-listing") => SlotCheck::Refused("slot-not-in-listing"),
        _ => SlotCheck::Refused("slot-not-in-listing"),
    }
}

async fn claim_answer<H: AppHost>(host: &H, row: &AnswerRow) -> Result<bool, Response> {
    let write = RecordWriteValue {
        id: answer_id(&row.enquiry_record_id),
        payload: serde_json::to_vec(row).map_err(|e| Response::internal_error(e.to_string()))?,
    };
    let outcome = AppDataLayer::create(host, LEDGER.to_string(), vec![write])
        .await
        .map_err(|e| Response::internal_error(e.to_string()))?;
    Ok(outcome.is_none())
}

async fn mark_answered<H: AppHost>(
    host: &H,
    row: &AnswerRow,
    quote_record_id: &str,
) -> Result<(), Response> {
    let mut row = row.clone();
    row.state = AnswerState::Answered;
    row.quote_record_id = Some(quote_record_id.to_string());
    put_row(host, LEDGER, &answer_id(&row.enquiry_record_id), &row)
        .await
        .map_err(Response::internal_error)
}

/// Ends a stored answer that can no longer be signed. It writes no card: the
/// counterparty's own check refuses a quote issued after its consent.
async fn end_answer<H: AppHost>(host: &H, row: &AnswerRow, code: &str) -> Result<(), Response> {
    let mut ended = row.clone();
    ended.state = AnswerState::Declined;
    ended.code = Some(code.to_string());
    put_row(host, LEDGER, &answer_id(&ended.enquiry_record_id), &ended)
        .await
        .map_err(Response::internal_error)
}

fn answered(row: &AnswerRow, quote_record_id: &str) -> AnswerRow {
    let mut answered = row.clone();
    answered.state = AnswerState::Answered;
    answered.quote_record_id = Some(quote_record_id.to_string());
    answered
}

/// Sends the quote card again if its send never worked, and returns the
/// stored answer.
async fn resend<H: AppHost>(host: &H, row: &AnswerRow, now: u64) -> Result<Outcome, Response> {
    let Some(ptr) = get_row::<RecordPointerRow, _>(host, QUOTES, &row.quote_id)
        .await
        .map_err(Response::internal_error)?
    else {
        return Err(Response::internal_error("the answer names a quote this node does not hold"));
    };
    let disclosures = disclosures::load(host, &ptr.record_id)
        .await
        .map_err(Response::internal_error)?
        .unwrap_or_default();
    let card = OutgoingCard {
        card_type: RECORD_QUOTE,
        version: transaction::QUOTE_VERSION,
        envelope: &ptr.envelope,
        send: &disclosures,
        view: &disclosures,
    };
    let (message_id, state, send_error) =
        send_card_and_file(host, &row.conversation, &card, now, Some(ptr.version_count)).await;
    let expires_at_secs = expiry_of(&ptr).unwrap_or(now);
    Ok(Outcome::Quoted(QuoteSetOutput {
        quote_id: row.quote_id.clone(),
        record_id: ptr.record_id,
        version_count: ptr.version_count,
        expires_at_secs,
        message_id,
        state,
        send_error: send_error.map(|e| e.to_string()),
    }))
}

fn expiry_of(ptr: &RecordPointerRow) -> Option<u64> {
    let envelope = Envelope::from_json(&ptr.envelope).ok()?;
    envelope.payload.get("terms")?.get("quote_expires_at_secs")?.as_u64()
}

async fn decline<H: AppHost>(
    host: &H,
    enquiry_record_id: &str,
    listing_id: &str,
    conversation: &str,
    code: &str,
    now: u64,
) -> Result<Outcome, Response> {
    decline_ops::decline_with_code(host, enquiry_record_id, code, now).await?;
    let row = AnswerRow {
        kind: LedgerKind::Answer,
        enquiry_record_id: enquiry_record_id.to_string(),
        conversation: conversation.to_string(),
        listing_id: listing_id.to_string(),
        quote_id: String::new(),
        sequence: 0,
        quote_record_id: None,
        formed_as: FormedAs::AsListedInstant,
        state: AnswerState::Declined,
        code: Some(code.to_string()),
        created_at_secs: now,
    };
    put_row(host, LEDGER, &answer_id(enquiry_record_id), &row)
        .await
        .map_err(Response::internal_error)?;
    Ok(Outcome::Declined)
}

/// Answers every held as-listed enquiry that has no answer row. A failure is
/// a warning, so one bad conversation cannot stop the rest.
pub(crate) async fn answer_waiting<H: AppHost>(host: &H, now: u64, warnings: &mut Vec<String>) {
    let rows: Vec<RecordPointerRow> = match paging::query_all(host, ENQUIRIES, None).await {
        Ok(rows) => rows,
        Err(e) => {
            warnings.push(format!("as-listed scan failed: {e}"));
            return;
        }
    };
    for row in rows {
        if row.mine || row.declined_at_secs.is_some() {
            continue;
        }
        match get_row::<AnswerRow, _>(host, LEDGER, &answer_id(&row.record_id)).await {
            Ok(Some(answered)) if answered.state != AnswerState::Answering => continue,
            Ok(_) => {}
            Err(_) => continue,
        }
        let Some(bytes) = get_bytes(host, ENQUIRY_HISTORY, &row.record_id).await.ok().flatten()
        else {
            continue;
        };
        let Ok(envelope) = String::from_utf8(bytes) else { continue };
        if !holds_as_listed_tap(&envelope) {
            continue;
        }
        let verdict = transaction::verify_enquiry(&envelope, now);
        if verdict.payload.is_none() {
            continue;
        }
        if let Err(e) = answer_as_listed(host, &row.record_id, Trigger::Driver, now).await {
            let message = e.error.map(|error| error.message).unwrap_or_default();
            warnings.push(format!("as-listed answer failed for '{}': {message}", row.record_id));
        }
    }
}

/// A cheap look at the stored enquiry before its signature is verified: only
/// an enquiry that carries an `as_listed` block can be an answer's input.
fn holds_as_listed_tap(envelope: &str) -> bool {
    Envelope::from_json(envelope)
        .ok()
        .is_some_and(|env| env.payload.get("as_listed").is_some_and(|v| !v.is_null()))
}

pub(crate) fn certificate_error(e: CertificateError) -> Response {
    match e {
        CertificateError::NotEnrolled => Response::invalid_params("signing-not-enrolled")
            .with_data(json!({"code": "signing-not-enrolled"})),
        CertificateError::Expired(t) => {
            Response::invalid_params(format!("signing-certificate-expired at {t}"))
                .with_data(json!({"code": "signing-certificate-expired"}))
        }
        CertificateError::Stale { installed_for, current } => Response::invalid_params(format!(
            "signing-certificate-stale: {installed_for} vs {current}"
        ))
        .with_data(json!({"code": "signing-certificate-stale"})),
        CertificateError::NoOwner => {
            Response::invalid_params("this installation has no recorded owner")
                .with_data(json!({"code": "signing-no-owner"}))
        }
        other => Response::internal_error(other.to_string()),
    }
}
