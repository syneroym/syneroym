//! What `quote.set` and `agreement.accept` do with an extension that this
//! node's registry does not know.
//!
//! The writer of a quote must know what it signs. The other party may sign a
//! part it cannot check, unless it is the giver of that part: the giver's node
//! must run the extension's hooks and steps.

use syneroym_roym_core::{
    deal::{Commitment, DealError, DealTerms},
    envelope::Response,
    extension::{self, ExtensionCheck},
    transaction::DealRole,
};

fn refusal(error: &DealError) -> Response {
    Response::invalid_params(error.to_string())
        .with_data(serde_json::json!({ "code": error.code() }))
}

fn extension_id(c: &Commitment) -> String {
    c.extension.as_ref().map(|e| e.id.clone()).unwrap_or_default()
}

/// The commitments that the registry cannot check.
fn unchecked(terms: &DealTerms) -> Result<Vec<&Commitment>, Response> {
    let checks = terms.check_extensions(extension::registry()).map_err(|e| refusal(&e))?;
    Ok(terms
        .commitments
        .iter()
        .zip(checks)
        .filter(|(_, check)| *check == ExtensionCheck::NotChecked)
        .map(|(c, _)| c)
        .collect())
}

/// `quote.set`: every commitment must have no extension or a known one.
pub(crate) fn require_known_extensions(terms: &DealTerms) -> Result<(), Response> {
    match unchecked(terms)?.first() {
        Some(c) => Err(refusal(&DealError::ExtensionUnknown(extension_id(c)))),
        None => Ok(()),
    }
}

/// `agreement.accept`: this node may not give a part it cannot check.
pub(crate) fn require_known_extensions_given_by(
    terms: &DealTerms,
    role: DealRole,
) -> Result<(), Response> {
    match unchecked(terms)?.into_iter().find(|c| c.giver == role) {
        Some(c) => Err(refusal(&DealError::ExtensionUnknown(extension_id(c)))),
        None => Ok(()),
    }
}
