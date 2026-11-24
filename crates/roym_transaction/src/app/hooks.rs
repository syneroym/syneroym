//! The deal core's calls to an extension's reserve hooks.
//!
//! The deal core names no extension. It asks the extension registry whether
//! a commitment's extension has the reserve hooks, and calls the service that
//! the extension's manifest names. A commitment whose extension has no
//! reserve hook is never reserved.

use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    deal::{Commitment, DealTerms, Details, ExtensionRef, transition::CommitmentState},
    envelope::{Response, RpcError},
    extension::{self, Manifest},
    method::{
        SiblingError, call_sibling,
        scheduling::{
            PrepareInput, PrepareOutput, ReleaseOutput, ReservationKey, ReservationView,
            ReserveInput,
        },
    },
    progress::{AgreementState, CommitmentProgress},
    services::Service,
    transaction::{DealRole, TimeWindow},
};

use super::{
    AgreementRow, LEDGER, get_row,
    ledger::{LedgerKind, LedgerRow},
    put_row,
    quote_ops::CommitmentInput,
};

/// The longest reason code an extension may answer. A code is signed into a
/// snapshot, so it must have the form of every Roym reason code.
const MAX_REASON_LEN: usize = 64;

fn manifest_of(extension: Option<&ExtensionRef>) -> Option<&'static Manifest> {
    extension.and_then(|e| extension::registry().workflow(e))
}

/// Whether the deal core must reserve this commitment's resource.
pub(crate) fn needs_reservation(c: &Commitment) -> bool {
    manifest_of(c.extension.as_ref()).is_some_and(Manifest::has_reserve_hook)
}

/// The commitments of `terms` that `role` gives and that must be reserved.
pub(crate) fn reserved_by(terms: &DealTerms, role: DealRole) -> impl Iterator<Item = &Commitment> {
    terms.commitments.iter().filter(move |c| c.giver == role && needs_reservation(c))
}

/// Whether the extension of a commitment that the quote author gives has the
/// `prepare` hook.
pub(crate) fn has_prepare_hook(extension: Option<&ExtensionRef>) -> bool {
    manifest_of(extension).is_some_and(Manifest::has_prepare_hook)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReserveOutcome {
    Held,
    /// The extension refused, with its reason code.
    Refused {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmOutcome {
    Confirmed,
    /// The extension refused, with its reason code.
    Refused {
        reason: String,
    },
}

/// The one place where the deal core meets an extension that holds a
/// resource. An `Err` is a fault (a transport error, or an answer that is
/// not a refusal): the caller retries later and does not end the deal.
#[allow(async_fn_in_trait)]
pub trait Reservations {
    async fn reserve<H: AppHost>(
        &self,
        host: &H,
        agreement: &str,
        commitment: &Commitment,
    ) -> Result<ReserveOutcome, String>;

    async fn confirm<H: AppHost>(
        &self,
        host: &H,
        agreement: &str,
        commitment: &Commitment,
    ) -> Result<ConfirmOutcome, String>;

    async fn release<H: AppHost>(
        &self,
        host: &H,
        agreement: &str,
        commitment: &Commitment,
    ) -> Result<(), String>;
}

/// Calls `reservation.<hook>` on the service that the extension's manifest
/// names.
#[derive(Debug, Default, Clone, Copy)]
pub struct ExtensionHooks;

fn hook_service(c: &Commitment) -> Result<Service, String> {
    manifest_of(c.extension.as_ref())
        .and_then(Manifest::hook_service)
        .ok_or_else(|| format!("commitment '{}' has no reserve hook", c.commitment_id))
}

fn is_reason_code(code: &str) -> bool {
    (1..=MAX_REASON_LEN).contains(&code.len())
        && code.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The reason code of a refusal. A refusal with no code, or a code that is not
/// a reason code, is a fault in the extension and not a refusal: the reason is
/// signed into a snapshot, so it is checked before it is used.
fn refusal_reason(error: &RpcError) -> Result<String, String> {
    let code = error.data.as_ref().and_then(|d| d.get("code")).and_then(serde_json::Value::as_str);
    match code {
        Some(code) if is_reason_code(code) => Ok(code.to_string()),
        _ => Err(format!("a hook answered an error with no valid reason code: {}", error.message)),
    }
}

fn key_of(agreement: &str, c: &Commitment) -> ReservationKey {
    ReservationKey { agreement: agreement.to_string(), commitment_id: c.commitment_id.clone() }
}

impl Reservations for ExtensionHooks {
    async fn reserve<H: AppHost>(
        &self,
        host: &H,
        agreement: &str,
        commitment: &Commitment,
    ) -> Result<ReserveOutcome, String> {
        let service = hook_service(commitment)?;
        let details = commitment.details.clone().ok_or_else(|| {
            format!("commitment '{}' has no details to reserve", commitment.commitment_id)
        })?;
        let input = ReserveInput {
            agreement: agreement.to_string(),
            commitment_id: commitment.commitment_id.clone(),
            details,
        };
        match call_sibling::<_, _, ReservationView>(host, service, "reservation.reserve", &input)
            .await
        {
            Ok(_) => Ok(ReserveOutcome::Held),
            Err(SiblingError::Refused(e)) => {
                refusal_reason(&e).map(|reason| ReserveOutcome::Refused { reason })
            }
            Err(other) => Err(other.to_string()),
        }
    }

    async fn confirm<H: AppHost>(
        &self,
        host: &H,
        agreement: &str,
        commitment: &Commitment,
    ) -> Result<ConfirmOutcome, String> {
        let service = hook_service(commitment)?;
        let key = key_of(agreement, commitment);
        match call_sibling::<_, _, ReservationView>(host, service, "reservation.confirm", &key)
            .await
        {
            Ok(_) => Ok(ConfirmOutcome::Confirmed),
            Err(SiblingError::Refused(e)) => {
                refusal_reason(&e).map(|reason| ConfirmOutcome::Refused { reason })
            }
            Err(other) => Err(other.to_string()),
        }
    }

    async fn release<H: AppHost>(
        &self,
        host: &H,
        agreement: &str,
        commitment: &Commitment,
    ) -> Result<(), String> {
        let service = hook_service(commitment)?;
        let key = key_of(agreement, commitment);
        call_sibling::<_, _, ReleaseOutput>(host, service, "reservation.release", &key)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Why `reserve_each` stopped before the last commitment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Stop {
    /// The extension refused, with its reason code.
    Refused(String),
    /// A fault: the call could not be answered. The caller tries again later.
    Fault(String),
}

/// What `reserve_each` did: the commitments it holds, and why it stopped early
/// if it did. The caller decides what to release, also after a fault.
#[derive(Debug, Default)]
pub(crate) struct Reserved {
    pub(crate) held: Vec<Commitment>,
    pub(crate) stopped: Option<Stop>,
}

/// Reserves every commitment in `commitments`, in order. It stops at the
/// first refusal or fault.
pub(crate) async fn reserve_each<'a, H: AppHost, R: Reservations>(
    host: &H,
    hooks: &R,
    agreement: &str,
    commitments: impl IntoIterator<Item = &'a Commitment>,
) -> Reserved {
    let mut out = Reserved::default();
    for c in commitments {
        match hooks.reserve(host, agreement, c).await {
            Ok(ReserveOutcome::Held) => out.held.push(c.clone()),
            Ok(ReserveOutcome::Refused { reason }) => {
                out.stopped = Some(Stop::Refused(reason));
                break;
            }
            Err(fault) => {
                out.stopped = Some(Stop::Fault(fault));
                break;
            }
        }
    }
    out
}

/// Confirms every commitment in `held`. A refusal answers its reason.
pub(crate) async fn confirm_each<H: AppHost, R: Reservations>(
    host: &H,
    hooks: &R,
    agreement: &str,
    held: &[Commitment],
) -> Result<Option<String>, String> {
    for c in held {
        if let ConfirmOutcome::Refused { reason } = hooks.confirm(host, agreement, c).await? {
            return Ok(Some(reason));
        }
    }
    Ok(None)
}

/// Releases every commitment in `commitments`. Every one is tried; the answer
/// is whether all of them were released. A release is a cleanup: a failed one
/// is tried again by the next repeat of the step that calls it, and the
/// extension's own idempotency makes that safe.
pub(crate) async fn release_each<'a, H: AppHost, R: Reservations>(
    host: &H,
    hooks: &R,
    agreement: &str,
    commitments: impl IntoIterator<Item = &'a Commitment>,
) -> bool {
    let mut all = true;
    for c in commitments {
        all &= hooks.release(host, agreement, c).await.is_ok();
    }
    all
}

/// Releases what `role` holds in `terms`. See [`release_each`].
pub(crate) async fn release_own<H: AppHost, R: Reservations>(
    host: &H,
    hooks: &R,
    agreement: &str,
    terms: &DealTerms,
    role: DealRole,
) -> bool {
    release_each(host, hooks, agreement, reserved_by(terms, role)).await
}

/// What `quote.set` and `quote.measure` do for a commitment whose extension has
/// the `prepare` hook: it answers the window to sign.
pub(crate) async fn prepare<H: AppHost>(
    host: &H,
    commitment: &CommitmentInput,
    terms_listing_id: Option<&str>,
) -> Result<TimeWindow, Response> {
    let details = commitment
        .details
        .clone()
        .ok_or_else(|| Response::invalid_params("commitment requires details"))?;
    prepare_window(
        host,
        commitment.extension.as_ref(),
        details,
        commitment.window,
        terms_listing_id,
    )
    .await
}

/// The `prepare` hook on a commitment's own parts. The as-listed answer calls
/// this directly, because it has finished terms rather than an input.
pub(crate) async fn prepare_window<H: AppHost>(
    host: &H,
    extension: Option<&ExtensionRef>,
    details: Details,
    window: Option<TimeWindow>,
    terms_listing_id: Option<&str>,
) -> Result<TimeWindow, Response> {
    let service = manifest_of(extension)
        .and_then(Manifest::hook_service)
        .ok_or_else(|| Response::internal_error("the extension has no hook service"))?;
    let input = PrepareInput { details, window, listing_id: terms_listing_id.map(str::to_string) };
    match call_sibling::<_, _, PrepareOutput>(host, service, "reservation.prepare", &input).await {
        Ok(out) => Ok(out.window),
        Err(SiblingError::Refused(e)) => {
            let refusal = Response::invalid_params(e.message);
            Err(match e.data {
                Some(data) => refusal.with_data(data),
                None => refusal,
            })
        }
        Err(other) => Err(Response::internal_error(other.to_string())),
    }
}

/// The ledger row that says a counterparty has released every reservation it
/// took for an agreement whose quote expired with no countersigned half.
pub(crate) fn released_id(agreement: &str) -> String {
    format!("released:{agreement}")
}

/// Releases the reservations that this node took as the counterparty, once
/// the quote has expired with no countersigned half.
///
/// `agreement.get` and the sync tick both run this on every call after the
/// expiry. So after every release has worked, a marker row records it and the
/// later calls make no hook call. A lost marker costs one more release, which
/// the extension answers idempotently.
pub(crate) async fn release_after_quote_expiry<H: AppHost, R: Reservations>(
    host: &H,
    hooks: &R,
    agreement: &AgreementRow,
    now: u64,
) {
    if reserved_by(&agreement.terms, DealRole::Counterparty).next().is_none() {
        return;
    }
    let id = released_id(&agreement.quote_record_id);
    if matches!(get_row::<LedgerRow, _>(host, LEDGER, &id).await, Ok(Some(_))) {
        return;
    }
    let released = release_own(
        host,
        hooks,
        &agreement.quote_record_id,
        &agreement.terms,
        DealRole::Counterparty,
    )
    .await;
    if !released {
        return;
    }
    let marker = LedgerRow {
        kind: LedgerKind::Released,
        agreement: agreement.quote_record_id.clone(),
        state: None,
        quote_id: None,
        commitment_id: None,
        reason: None,
        half: None,
        created_at_secs: now,
    };
    // A marker that is not stored only means the next call releases again.
    let _ = put_row(host, LEDGER, &id, &marker).await;
}

/// The ledger row that says this node released its reservation of one
/// commitment that the agreement's snapshot shows ended.
pub(crate) fn released_commitment_id(agreement: &str, commitment_id: &str) -> String {
    format!("released:{agreement}:{commitment_id}")
}

/// What a snapshot shows about the commitments that may need a release.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ended<'a> {
    pub(crate) state: AgreementState,
    pub(crate) commitments: &'a [CommitmentProgress],
}

/// Releases the reservations of `role` whose commitments the snapshot shows
/// ended: `failed` or `cancelled`, or every one when the agreement is
/// `not-formed` or `cancelled`. A commitment with a settlement half is
/// skipped, so a resource that was claimed is never freed.
///
/// A release that worked is recorded in the ledger, so a later call makes no
/// hook call for it. A release that failed leaves no row, so the next sync
/// tries it again: nothing else would. Answers whether every one is released.
pub(crate) async fn release_ended<H: AppHost, R: Reservations>(
    host: &H,
    hooks: &R,
    agreement: &AgreementRow,
    role: DealRole,
    ended: Ended<'_>,
    now: u64,
) -> bool {
    let all = matches!(ended.state, AgreementState::NotFormed | AgreementState::Cancelled);
    let mut done = true;
    for c in reserved_by(&agreement.terms, role) {
        let progress = ended.commitments.iter().find(|p| p.commitment_id == c.commitment_id);
        if progress.is_some_and(|p| !p.halves.is_empty()) {
            continue;
        }
        let over = all
            || progress.is_some_and(|p| {
                matches!(p.state, CommitmentState::Failed | CommitmentState::Cancelled)
            });
        if over {
            done &= release_marked(host, hooks, agreement, c, now).await;
        }
    }
    done
}

async fn release_marked<H: AppHost, R: Reservations>(
    host: &H,
    hooks: &R,
    agreement: &AgreementRow,
    c: &Commitment,
    now: u64,
) -> bool {
    let id = released_commitment_id(&agreement.quote_record_id, &c.commitment_id);
    if matches!(get_row::<LedgerRow, _>(host, LEDGER, &id).await, Ok(Some(_))) {
        return true;
    }
    if hooks.release(host, &agreement.quote_record_id, c).await.is_err() {
        return false;
    }
    let marker = LedgerRow {
        kind: LedgerKind::Released,
        agreement: agreement.quote_record_id.clone(),
        state: None,
        quote_id: None,
        commitment_id: Some(c.commitment_id.clone()),
        reason: None,
        half: None,
        created_at_secs: now,
    };
    // A marker that is not stored only means the next sync releases again.
    put_row(host, LEDGER, &id, &marker).await.is_ok()
}

/// The retry of [`release_ended`] for an agreement row that this node holds:
/// it reads the snapshot that the row already shows.
pub(crate) async fn release_ended_of_row<H: AppHost, R: Reservations>(
    host: &H,
    hooks: &R,
    row: &AgreementRow,
    owner: &str,
    now: u64,
) {
    let role = if owner == row.quote_author_did {
        DealRole::QuoteAuthor
    } else if owner == row.counterparty_did {
        DealRole::Counterparty
    } else {
        return;
    };
    if let Some(state) = row.state {
        let ended = Ended { state, commitments: &row.commitments };
        release_ended(host, hooks, row, role, ended, now).await;
    }
}
