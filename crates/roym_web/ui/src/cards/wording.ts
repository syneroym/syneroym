/// Wording pinned character-for-character against the Rust constants of
/// the same name in crates/roym_core/src/booking.rs. This is the single
/// place each sentence lives on the TypeScript side; templates import
/// these constants instead of writing the sentences inline.
///
/// crates/roym_core/src/booking/tests.rs has a test,
/// `the_ui_wording_matches_this_crate`, that reads this file and checks
/// each Rust constant's value appears verbatim here. Keep both in sync.

export const PAYMENT_NOTICE =
  "This records what each side says about the payment. Roym does not see the money move and cannot confirm that it did.";

export const PROGRESS_NOTICE =
  "This status comes from the provider's system. It is not a signed statement by either person.";

export const PAYMENT_CLAIMED = "The customer says they paid.";

export const PAYMENT_ACKNOWLEDGED = "The provider confirms they received the payment.";

export const FULFILMENT_CLAIMED = "The provider says the work is done.";

export const FULFILMENT_ACKNOWLEDGED = "The customer confirms the work is done.";

export const TRACK_UNCONFIRMED = "No confirmation was recorded before the window closed.";

export const ONE_PAYMENT_NOTICE =
  "This quote is paid in one payment. Deposits and part payments are not supported.";
