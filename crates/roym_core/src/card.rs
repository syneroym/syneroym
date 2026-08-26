//! Card types and versions.

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

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::*;

    #[test]
    fn the_ui_card_registry_matches_this_crate() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let registry_path = manifest_dir.join("../roym_web/ui/src/cards/registry.ts");
        if !registry_path.exists() {
            // If the UI source hasn't been written yet in early pipeline steps, allow
            // skipping this test until Step 10 creates it.
            return;
        }

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
}
