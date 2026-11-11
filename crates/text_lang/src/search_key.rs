use std::collections::HashSet;

use icu_normalizer::ComposingNormalizerBorrowed;
use icu_properties::{
    CodePointMapData, CodePointMapDataBorrowed,
    props::{GeneralCategory, GeneralCategoryGroup},
};

#[cfg(test)]
mod tests;

/// Changes whenever `search_key` gives a different output for some input.
/// A stored text index records the version it was built with; a
/// different version makes the next `create-collection` rebuild it.
pub const SEARCH_KEY_VERSION: u32 = 1;

/// A query term of at most this many `char`s is a short term.
pub const SHORT_TERM_MAX_CHARS: usize = 2;

/// Unicode NFC (canonical composition).
pub fn nfc(text: &str) -> String {
    ComposingNormalizerBorrowed::new_nfc().normalize(text).into_owned()
}

/// The form of a text that search compares: lower case, then NFC.
/// Idempotent: `search_key(&search_key(s)) == search_key(s)`.
pub fn search_key(text: &str) -> String {
    nfc(&text.to_lowercase())
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryTerms {
    /// Terms of 3 or more `char`s, in query order, no duplicates.
    pub long: Vec<String>,
    /// Terms of 1 or 2 `char`s, in query order, no duplicates.
    pub short: Vec<String>,
}

fn is_retained_boundary_char(
    c: char,
    gc_map: &CodePointMapDataBorrowed<'_, GeneralCategory>,
) -> bool {
    if c.is_alphanumeric() {
        return true;
    }
    let gc = gc_map.get(c);
    GeneralCategoryGroup::Mark.contains(gc)
}

fn trim_term<'a>(
    token: &'a str,
    gc_map: &CodePointMapDataBorrowed<'_, GeneralCategory>,
) -> &'a str {
    let Some(start_idx) = token.find(|c| is_retained_boundary_char(c, gc_map)) else {
        return "";
    };
    let Some(last_char_idx) = token.rfind(|c| is_retained_boundary_char(c, gc_map)) else {
        return "";
    };
    let Some(c) = token[last_char_idx..].chars().next() else {
        return "";
    };
    &token[start_idx..last_char_idx + c.len_utf8()]
}

/// `search_key(query)`, split on Unicode white space. Terms are trimmed
/// of leading and trailing punctuation and symbol `char`s (stopping at
/// alphanumeric chars or Unicode marks, such as virama, pulli, or nukta; P16),
/// classified into long/short on trimmed length, and empty terms are dropped
/// (P4). Interior punctuation (`plumber's`, `a-b`) is kept. Empty when nothing
/// is left.
pub fn query_terms(query: &str) -> QueryTerms {
    let key = search_key(query);
    let gc_map = CodePointMapData::<GeneralCategory>::new();

    let mut terms = QueryTerms::default();
    let mut seen = HashSet::new();

    for token in key.split_whitespace() {
        let trimmed = trim_term(token, &gc_map);
        if trimmed.is_empty() {
            continue;
        }
        if !seen.insert(trimmed.to_string()) {
            continue;
        }
        let char_count = trimmed.chars().count();
        if char_count <= SHORT_TERM_MAX_CHARS {
            terms.short.push(trimmed.to_string());
        } else {
            terms.long.push(trimmed.to_string());
        }
    }

    terms
}
