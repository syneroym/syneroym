//! After an import, this node takes again the reservations of its restored
//! agreements.
//!
//! An agreement and its reservation are in two bundles, and a person may
//! import them in either order. A restored agreement whose seat looks free
//! would let a second booking take it. So the import asks the extension to
//! reserve again each commitment that this node gives and that is still open.
//! A reserve is idempotent: when the extension's own bundle came first, it
//! finds the restored reservation and changes nothing.

use serde::Serialize;
use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    deal::{DealTerms, transition::CommitmentState},
    transaction::DealRole,
};

use crate::app::{
    AgreementRow,
    hooks::{self, ConfirmOutcome, ExtensionHooks, Reservations, ReserveOutcome},
};

/// One reservation that an import could not take again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReservationFailure {
    pub agreement: String,
    pub commitment_id: String,
    pub reason: String,
}

#[derive(Debug, Default)]
pub(crate) struct Restored {
    pub(crate) restored: u32,
    pub(crate) failed: Vec<ReservationFailure>,
}

fn own_role(row: &AgreementRow, owner: &str) -> Option<DealRole> {
    if owner == row.quote_author_did {
        Some(DealRole::QuoteAuthor)
    } else if owner == row.counterparty_did {
        Some(DealRole::Counterparty)
    } else {
        None
    }
}

/// Whether this node holds reservations for the agreement. The quote author
/// holds them once the agreement is formed, and the counterparty holds them
/// from its acceptance until the agreement ends.
fn holds_reservations(row: &AgreementRow, role: DealRole, now: u64) -> bool {
    match (row.state, role) {
        (Some(state), _) if state.is_terminal() => false,
        (Some(_), _) => true,
        (None, DealRole::QuoteAuthor) => false,
        (None, DealRole::Counterparty) => {
            row.counterparty.is_some() && now < row.terms.quote_expires_at_secs
        }
    }
}

/// The commitment ids this node gives, needs reserved, and that are still
/// open: no settlement half, and not past `promised`.
fn open_commitments<'a>(
    row: &'a AgreementRow,
    terms: &'a DealTerms,
    role: DealRole,
) -> Vec<&'a str> {
    hooks::reserved_by(terms, role)
        .filter(|c| {
            row.commitments
                .iter()
                .find(|p| p.commitment_id == c.commitment_id)
                .is_none_or(|p| p.state == CommitmentState::Promised && p.halves.is_empty())
        })
        .map(|c| c.commitment_id.as_str())
        .collect()
}

/// Reserves again every open reservation of the agreements this node holds.
pub(crate) async fn reserve_again<H: AppHost>(
    host: &H,
    agreements: &[(String, AgreementRow)],
    owner: &str,
    now: u64,
) -> Restored {
    let hooks = ExtensionHooks;
    let mut out = Restored::default();
    for (_, row) in agreements {
        let Some(role) = own_role(row, owner) else { continue };
        if !holds_reservations(row, role, now) {
            continue;
        }
        let open = open_commitments(row, &row.terms, role);
        for c in hooks::reserved_by(&row.terms, role)
            .filter(|c| open.contains(&c.commitment_id.as_str()))
        {
            let failed = |reason: String| ReservationFailure {
                agreement: row.quote_record_id.clone(),
                commitment_id: c.commitment_id.clone(),
                reason,
            };
            let reserved = match hooks.reserve(host, &row.quote_record_id, c).await {
                Ok(ReserveOutcome::Held) => true,
                Ok(ReserveOutcome::Refused { reason }) | Err(reason) => {
                    out.failed.push(failed(reason));
                    false
                }
            };
            if !reserved {
                continue;
            }
            if row.quote_author.is_some() {
                match hooks.confirm(host, &row.quote_record_id, c).await {
                    Ok(ConfirmOutcome::Confirmed) => {}
                    Ok(ConfirmOutcome::Refused { reason }) | Err(reason) => {
                        out.failed.push(failed(reason));
                        continue;
                    }
                }
            }
            out.restored += 1;
        }
    }
    out
}
