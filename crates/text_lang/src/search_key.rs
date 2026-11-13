use std::collections::HashSet;

use icu_normalizer::{ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed};
use icu_properties::{
    CodePointMapData, CodePointMapDataBorrowed,
    props::{GeneralCategory, GeneralCategoryGroup},
};

mod devanagari;
mod fold;
mod script;
mod tamil;

use script::Script;

#[cfg(test)]
mod tests;

/// Changes whenever `search_key` gives a different output for some input.
/// A stored text index records the version it was built with; a
/// different version makes the next `create-collection` rebuild it.
pub const SEARCH_KEY_VERSION: u32 = 2;

/// A query term of at most this many `char`s is a short term.
pub const SHORT_TERM_MAX_CHARS: usize = 2;

/// Unicode NFC (canonical composition).
pub fn nfc(text: &str) -> String {
    ComposingNormalizerBorrowed::new_nfc().normalize(text).into_owned()
}

/// Normalizes Latin letters by decomposing, removing non-spacing combining
/// marks (Mn), and composing back to NFC. Characters without decomposition
/// remain unchanged.
pub fn latin_base(text: &str) -> String {
    let nfd = DecomposingNormalizerBorrowed::new_nfd().normalize(text).into_owned();
    let gc_map = CodePointMapData::<GeneralCategory>::new();

    let stripped: String =
        nfd.chars().filter(|&c| gc_map.get(c) != GeneralCategory::NonspacingMark).collect();

    nfc(&stripped)
}

fn is_retained_char(c: char, gc_map: &CodePointMapDataBorrowed<'_, GeneralCategory>) -> bool {
    c.is_alphanumeric() || GeneralCategoryGroup::Mark.contains(gc_map.get(c))
}

fn split_words<'a>(
    text: &'a str,
    gc_map: &CodePointMapDataBorrowed<'_, GeneralCategory>,
) -> Vec<&'a str> {
    let mut words = Vec::new();
    let mut word_start = None;
    let mut prev_end = 0;

    for (idx, c) in text.char_indices() {
        let char_len = c.len_utf8();
        let char_end = idx + char_len;

        if is_retained_char(c, gc_map) {
            if word_start.is_none() {
                word_start = Some(idx);
            }
            prev_end = char_end;
        } else if let Some(start) = word_start.take() {
            words.push(&text[start..prev_end]);
        }
    }

    if let Some(start) = word_start {
        words.push(&text[start..prev_end]);
    }

    words
}

fn process_word(
    word: &str,
    gc_map: &CodePointMapDataBorrowed<'_, GeneralCategory>,
) -> tamil::RawWord {
    let runs = script::script_runs(word, gc_map);
    let mut word_text = String::new();
    let mut ends_with_pulli_stop = None;
    let mut starts_with_stop = None;

    for (sc, rtext) in runs {
        match sc {
            Script::Devanagari => {
                word_text.push_str(&devanagari::to_latin(rtext));
            }
            Script::Tamil => {
                let tw = tamil::to_latin(rtext);
                word_text.push_str(&tw.text);
                if starts_with_stop.is_none() {
                    starts_with_stop = tw.starts_with_stop;
                }
                ends_with_pulli_stop = tw.ends_with_pulli_stop;
            }
            Script::Latin => {
                word_text.push_str(&latin_base(rtext));
            }
            Script::Other => {
                word_text.push_str(rtext);
            }
        }
    }

    tamil::RawWord { text: word_text, ends_with_pulli_stop, starts_with_stop }
}

/// The form of a text that search compares: lower case, NFC, transliteration of
/// Devanagari and Tamil scripts, and phonetic Latin folding.
pub fn search_key(text: &str) -> String {
    let lowered = text.to_lowercase();
    let normalized = nfc(&lowered);
    let cleaned: String =
        normalized.chars().filter(|&c| c != '\u{200D}' && c != '\u{200C}').collect();

    let gc_map = CodePointMapData::<GeneralCategory>::new();

    let raw_tokens = split_words(&cleaned, &gc_map);
    let raw_words: Vec<tamil::RawWord> =
        raw_tokens.into_iter().map(|tok| process_word(tok, &gc_map)).collect();

    let sandhi_words = tamil::sandhi(raw_words);
    let folded_words: Vec<String> =
        sandhi_words.into_iter().map(|w| fold::fold_word(&w)).filter(|w| !w.is_empty()).collect();

    folded_words.join(" ")
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryTerms {
    /// Terms of 3 or more `char`s, in query order, no duplicates.
    pub long: Vec<String>,
    /// Terms of 1 or 2 `char`s, in query order, no duplicates.
    pub short: Vec<String>,
}

/// `search_key(query)`, split on space into non-empty, deduplicated terms.
/// Terms of 2 or fewer characters are classified as short, and longer terms as
/// long.
pub fn query_terms(query: &str) -> QueryTerms {
    let key = search_key(query);
    let mut terms = QueryTerms::default();
    let mut seen = HashSet::new();

    for token in key.split_whitespace() {
        if !seen.insert(token.to_string()) {
            continue;
        }
        let char_count = token.chars().count();
        if char_count <= SHORT_TERM_MAX_CHARS {
            terms.short.push(token.to_string());
        } else {
            terms.long.push(token.to_string());
        }
    }

    terms
}
