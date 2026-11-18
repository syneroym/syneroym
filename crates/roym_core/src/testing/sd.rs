//! Shared test builders and helpers for selective disclosure.

use syneroym_signed_record::{Salt, seeded_salt};

use crate::card;

/// Salts 0, 1, 2, … from `seed`: `(0..).map(|n| seeded_salt(seed, n))`.
pub fn seeded_salts(seed: u64) -> impl Iterator<Item = Salt> {
    (0..).map(move |n| seeded_salt(seed, n))
}

/// Serializes a card body with disclosures, expecting success.
pub fn deal_card_body(card_type: &str, envelope: &str, disclosures: &[String]) -> String {
    card::card_body(card_type, 1, envelope, disclosures).expect("serialize deal card body")
}
