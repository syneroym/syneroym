//! ISO-4217 minor units. The one place this product decides how many
//! minor units a currency has, so a signed amount means the same thing to
//! whoever renders it. A signed payload may hold no non-integer number,
//! so an amount is always minor units plus a code, and the code has to be
//! one this build knows.

/// Currencies with no minor unit at all.
pub const EXPONENT_0: &[&str] = &[
    "BIF", "CLP", "DJF", "GNF", "ISK", "JPY", "KMF", "KRW", "PYG", "RWF", "UGX", "UYI", "VND",
    "VUV", "XAF", "XOF", "XPF",
];

/// Currencies with three minor digits.
pub const EXPONENT_3: &[&str] = &["BHD", "IQD", "JOD", "KWD", "LYD", "OMR", "TND"];

/// Every ISO-4217 alphabetic code this build accepts, sorted, so a
/// lookup is a binary search and a reviewer can see the whole set. A code
/// outside this list is refused, not assumed to have two minor digits:
/// assuming two signs a Kuwaiti dinar a thousand times low, and assuming
/// a currency exists at all lets a signed amount name nothing.
pub const CURRENCY_CODES: &[&str] = &[
    "AED", "AFN", "ALL", "AMD", "ANG", "AOA", "ARS", "AUD", "AWG", "AZN", "BAM", "BBD", "BDT",
    "BGN", "BHD", "BIF", "BMD", "BND", "BOB", "BOV", "BRL", "BSD", "BTN", "BWP", "BYN", "BZD",
    "CAD", "CDF", "CHE", "CHF", "CHW", "CLP", "CNY", "COP", "COU", "CRC", "CUP", "CVE", "CZK",
    "DJF", "DKK", "DOP", "DZD", "EGP", "ERN", "ETB", "EUR", "FJD", "FKP", "GBP", "GEL", "GHS",
    "GIP", "GMD", "GNF", "GTQ", "GYD", "HKD", "HNL", "HTG", "HUF", "IDR", "ILS", "INR", "IQD",
    "IRR", "ISK", "JMD", "JOD", "JPY", "KES", "KGS", "KHR", "KMF", "KPW", "KRW", "KWD", "KYD",
    "KZT", "LAK", "LBP", "LKR", "LRD", "LSL", "LYD", "MAD", "MDL", "MGA", "MKD", "MMK", "MNT",
    "MOP", "MRU", "MUR", "MVR", "MWK", "MXN", "MXV", "MYR", "MZN", "NAD", "NGN", "NIO", "NOK",
    "NPR", "NZD", "OMR", "PAB", "PEN", "PGK", "PHP", "PKR", "PLN", "PYG", "QAR", "RON", "RSD",
    "RUB", "RWF", "SAR", "SBD", "SCR", "SDG", "SEK", "SGD", "SHP", "SLE", "SOS", "SRD", "SSP",
    "STN", "SVC", "SYP", "SZL", "THB", "TJS", "TMT", "TND", "TOP", "TRY", "TTD", "TWD", "TZS",
    "UAH", "UGX", "USD", "USN", "UYI", "UYU", "UZS", "VED", "VES", "VND", "VUV", "WST", "XAF",
    "XCD", "XOF", "XPF", "YER", "ZAR", "ZMW",
];

/// Three uppercase ASCII letters. Shape only; `currency_minor_exponent`
/// decides whether the code is one this build knows.
#[must_use]
pub fn is_currency_shape(code: &str) -> bool {
    code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase())
}

/// `None` for a code outside `CURRENCY_CODES`, including a well-shaped
/// but unassigned one such as `"XYZ"`. This is the whole point of the
/// function: a caller refuses rather than assumes.
#[must_use]
pub fn currency_minor_exponent(code: &str) -> Option<u32> {
    if !is_currency_shape(code) {
        return None;
    }
    if CURRENCY_CODES.binary_search(&code).is_err() {
        return None;
    }
    if EXPONENT_0.binary_search(&code).is_ok() {
        Some(0)
    } else if EXPONENT_3.binary_search(&code).is_ok() {
        Some(3)
    } else {
        Some(2)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::*;

    #[test]
    fn exponent_0_and_3_lists_are_sorted_and_disjoint() {
        assert!(EXPONENT_0.windows(2).all(|w| w[0] < w[1]));
        assert!(EXPONENT_3.windows(2).all(|w| w[0] < w[1]));
        for code in EXPONENT_0 {
            assert!(
                !EXPONENT_3.contains(code),
                "code {code} must not be in both EXPONENT_0 and EXPONENT_3"
            );
            assert!(
                CURRENCY_CODES.contains(code),
                "code {code} from EXPONENT_0 must be in CURRENCY_CODES"
            );
        }
        for code in EXPONENT_3 {
            assert!(
                CURRENCY_CODES.contains(code),
                "code {code} from EXPONENT_3 must be in CURRENCY_CODES"
            );
        }
    }

    #[test]
    fn currency_codes_is_sorted_and_has_no_duplicates() {
        assert!(CURRENCY_CODES.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn an_unknown_code_has_no_exponent() {
        assert_eq!(currency_minor_exponent("XYZ"), None);
        assert_eq!(currency_minor_exponent("us"), None);
        assert_eq!(currency_minor_exponent("USDX"), None);
        assert_eq!(currency_minor_exponent("usd"), None);
        assert_eq!(currency_minor_exponent(""), None);

        // Non-currencies, testing codes, and precious metals are refused
        for code in [
            "XAU", "XAG", "XPD", "XPT", "XBA", "XBB", "XBC", "XBD", "XDR", "XSU", "XTS", "XUA",
            "XXX",
        ] {
            assert_eq!(
                currency_minor_exponent(code),
                None,
                "{code} must be refused as a non-currency"
            );
        }

        // Withdrawn currencies and 4-decimal codes are refused
        for code in ["CUC", "HRK", "ZWL", "CLF", "UYW"] {
            assert_eq!(
                currency_minor_exponent(code),
                None,
                "{code} must be refused as an unaccepted/withdrawn code"
            );
        }

        // Check a few known codes while here
        assert_eq!(currency_minor_exponent("USD"), Some(2));
        assert_eq!(currency_minor_exponent("EUR"), Some(2));
        assert_eq!(currency_minor_exponent("JPY"), Some(0));
        assert_eq!(currency_minor_exponent("KWD"), Some(3));
    }

    #[test]
    fn the_ui_currency_table_matches_this_crate() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let money_path = manifest_dir.join("../roym_web/ui/src/money.ts");
        assert!(money_path.exists(), "missing ../roym_web/ui/src/money.ts");

        let content =
            fs::read_to_string(&money_path).expect("Failed to read ../roym_web/ui/src/money.ts");

        fn parse_set(content: &str, set_name: &str) -> Vec<String> {
            let start_idx = content
                .find(set_name)
                .and_then(|idx| content[idx..].find('['))
                .map(|offset| content.find(set_name).unwrap() + offset)
                .unwrap_or_else(|| panic!("{set_name} array opening bracket not found"));
            let slice = &content[start_idx..];
            let end_idx = slice.find(']').unwrap_or(slice.len());
            let array_str = &slice[1..end_idx];
            array_str
                .split(',')
                .map(|item| item.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }

        let ui_exp0 = parse_set(&content, "EXPONENT_0");
        let expected_exp0: Vec<String> = EXPONENT_0.iter().map(|s| s.to_string()).collect();
        assert_eq!(ui_exp0, expected_exp0, "UI EXPONENT_0 must match Rust EXPONENT_0");

        let ui_exp3 = parse_set(&content, "EXPONENT_3");
        let expected_exp3: Vec<String> = EXPONENT_3.iter().map(|s| s.to_string()).collect();
        assert_eq!(ui_exp3, expected_exp3, "UI EXPONENT_3 must match Rust EXPONENT_3");
    }
}
