//! Who this node's owner is in an agreement, and whether the agreement still
//! takes new records. Fulfilment receipts and step records share these rules.

use syneroym_roym_core::{
    deal::SettlementRole,
    envelope::Response,
    transaction::{DealRole, PairState, pair_state},
};

use super::AgreementRow;

/// The agreement is formed on this node (both halves held) and not ended.
pub(crate) fn require_open_agreement(agr: &AgreementRow) -> Result<(), Response> {
    let pair = pair_state(agr.counterparty.as_ref(), agr.quote_author.as_ref());
    if pair != PairState::Complete {
        return Err(Response::invalid_params("agreement-incomplete"));
    }
    if let Some(state) = agr.state
        && state.is_terminal()
    {
        return Err(Response::invalid_params("agreement-terminal"));
    }
    Ok(())
}

/// Which party of the agreement `owner` is.
pub(crate) fn agreement_party(agr: &AgreementRow, owner: &str) -> Result<DealRole, Response> {
    if owner == agr.quote_author_did {
        Ok(DealRole::QuoteAuthor)
    } else if owner == agr.counterparty_did {
        Ok(DealRole::Counterparty)
    } else {
        Err(Response::invalid_params("not-a-party"))
    }
}

/// The party's role in one commitment, from who gives that commitment.
pub(crate) fn settlement_role(party: DealRole, giver: DealRole) -> SettlementRole {
    if party == giver { SettlementRole::Giver } else { SettlementRole::Receiver }
}
