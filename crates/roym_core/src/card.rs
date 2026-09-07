//! Card types and versions.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The seven card types of the first release, and the version each one is
/// rendered at today. Fixed: a card of an unlisted type, or a listed type
/// at an unlisted version, renders as the neutral unknown block.
pub const CARD_TYPES: &[(&str, u32)] = &[
    ("request", 1),
    ("quote", 1),
    ("agreement-receipt", 1),
    ("booking-progress", 1),
    ("payment-request", 1),
    ("payment-acknowledgement", 1),
    ("fulfilment-receipt", 1),
];

pub fn is_known_card(card_type: &str, version: u32) -> bool {
    CARD_TYPES.iter().any(|&(t, v)| t == card_type && v == version)
}

/// The reserved content type a card message carries. A client that does
/// not understand it sees an ordinary message of an unknown type, which
/// is the honest failure mode.
pub const CARD_CONTENT_TYPE: &str = "application/vnd.roym.card+json";

/// The wrapper's own version, distinct from the card type's version. It
/// says how to read the three fields below; the type's version says which
/// template renders them.
pub const CARD_WRAPPER_VERSION: u32 = 1;

/// A card as it travels. It carries the signed envelope and nothing
/// derived from it: the receiving node verifies and projects, so a sender
/// cannot supply a rendering that disagrees with what it signed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub card_version: u32,
    #[serde(rename = "type")]
    pub card_type: String,
    pub version: u32,
    /// The signed envelope, exactly as the host returned it.
    pub envelope: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CardError {
    #[error("card body is not valid JSON: {0}")]
    Json(String),
    #[error("card wrapper version {0} is not understood by this build")]
    UnknownWrapperVersion(u32),
    #[error("card body is over the {MAX_CARD_BODY_BYTES}-byte maximum")]
    TooLarge,
}

/// The envelope's own 64 KiB payload ceiling (`MAX_PAYLOAD_BYTES`) plus
/// room for what wraps it: the envelope's other fields, a delegation
/// certificate, and the JSON string escaping that carries the whole
/// envelope inside this body's `envelope` field -- escaping alone can
/// nearly double a payload made of quotes and backslashes. Not a round
/// multiple of the payload ceiling, and deliberately not described as
/// one. A body over this is refused before it is parsed.
pub const MAX_CARD_BODY_BYTES: usize = 160 * 1024;

/// Parses a card body. Refuses an unknown wrapper version rather than
/// guessing; an unknown `(type, version)` is **not** refused here, because
/// the neutral unknown block is a rendering decision, not a parse failure.
pub fn parse_card(body: &str) -> Result<Card, CardError> {
    if body.len() > MAX_CARD_BODY_BYTES {
        return Err(CardError::TooLarge);
    }
    let card: Card = serde_json::from_str(body).map_err(|e| CardError::Json(e.to_string()))?;
    if card.card_version != CARD_WRAPPER_VERSION {
        return Err(CardError::UnknownWrapperVersion(card.card_version));
    }
    Ok(card)
}

/// The body a card message carries for `envelope`.
pub fn card_body(card_type: &str, version: u32, envelope: &str) -> Result<String, CardError> {
    let card = Card {
        card_version: CARD_WRAPPER_VERSION,
        card_type: card_type.to_string(),
        version,
        envelope: envelope.to_string(),
    };
    let s = serde_json::to_string(&card).map_err(|e| CardError::Json(e.to_string()))?;
    if s.len() > MAX_CARD_BODY_BYTES {
        return Err(CardError::TooLarge);
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::*;

    #[test]
    fn the_ui_card_registry_matches_this_crate() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let registry_path = manifest_dir.join("../roym_web/ui/src/cards/registry.ts");
        assert!(registry_path.exists(), "missing ../roym_web/ui/src/cards/registry.ts");

        let content = fs::read_to_string(&registry_path)
            .expect("Failed to read ../roym_web/ui/src/cards/registry.ts");

        // Parse pairs like `["request", 1]`
        let mut parsed_pairs = Vec::new();
        let start_idx = content
            .find("CARD_TYPES")
            .and_then(|idx| content[idx..].find('['))
            .map(|offset| content.find("CARD_TYPES").unwrap() + offset)
            .expect("CARD_TYPES array opening bracket not found");

        let slice = &content[start_idx..];
        let end_idx = slice.find(';').unwrap_or(slice.len());
        let array_str = &slice[..end_idx];

        for item in array_str.split('[') {
            if let Some(close) = item.find(']') {
                let inner = &item[..close].trim();
                if inner.contains(',') {
                    let parts: Vec<&str> = inner.split(',').collect();
                    if parts.len() == 2 {
                        let name = parts[0].trim().trim_matches(|c| c == '"' || c == '\'');
                        if let Ok(ver) = parts[1].trim().parse::<u32>() {
                            parsed_pairs.push((name.to_string(), ver));
                        }
                    }
                }
            }
        }

        let expected_pairs: Vec<(String, u32)> =
            CARD_TYPES.iter().map(|(s, v)| (s.to_string(), *v)).collect();
        assert_eq!(parsed_pairs, expected_pairs);
    }

    #[test]
    fn card_round_trip() {
        let envelope = "{\"test\": \"env\"}";
        let body = card_body("request", 1, envelope).expect("serialize card");
        let parsed = parse_card(&body).expect("parse card");
        assert_eq!(parsed.card_version, CARD_WRAPPER_VERSION);
        assert_eq!(parsed.card_type, "request");
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.envelope, envelope);
    }

    #[test]
    fn unknown_wrapper_version_is_refused() {
        let json = r#"{"card_version": 99, "type": "request", "version": 1, "envelope": "{}"}"#;
        assert_eq!(parse_card(json), Err(CardError::UnknownWrapperVersion(99)));
    }

    #[test]
    fn unknown_type_and_version_parses() {
        let json = r#"{"card_version": 1, "type": "future-card", "version": 5, "envelope": "{}"}"#;
        let parsed = parse_card(json).expect("should parse unknown card type/version");
        assert_eq!(parsed.card_type, "future-card");
        assert_eq!(parsed.version, 5);
    }

    #[test]
    fn oversize_body_is_refused() {
        let large_body = "x".repeat(MAX_CARD_BODY_BYTES + 1);
        assert_eq!(parse_card(&large_body), Err(CardError::TooLarge));

        let large_env = "x".repeat(MAX_CARD_BODY_BYTES);
        assert_eq!(card_body("request", 1, &large_env), Err(CardError::TooLarge));
    }

    #[test]
    fn extra_keys_parse_but_missing_envelope_fails() {
        let with_extra = r#"{"card_version": 1, "type": "request", "version": 1, "envelope": "{}", "unexpected": 123}"#;
        let parsed = parse_card(with_extra).expect("serde should ignore extra keys");
        assert_eq!(parsed.card_type, "request");

        let missing_env = r#"{"card_version": 1, "type": "request", "version": 1}"#;
        assert!(matches!(parse_card(missing_env), Err(CardError::Json(_))));
    }
}
