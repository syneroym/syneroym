use std::time::Instant;

use super::{QueryTerms, SEARCH_KEY_VERSION, SHORT_TERM_MAX_CHARS, nfc, query_terms, search_key};

mod devanagari;
mod fold;
mod spelling;
mod tamil;

#[test]
fn nfc_normalization_s0_inputs() {
    assert_eq!(nfc("\u{0958}"), "\u{0915}\u{093C}");
    assert_eq!(nfc("\u{0915}\u{093C}"), "\u{0915}\u{093C}");
    assert_eq!(nfc("\u{0B95}\u{0BC6}\u{0BBE}"), "\u{0B95}\u{0BCA}");
    assert_eq!(nfc("\u{0065}\u{0301}"), "\u{00E9}");
    assert_eq!(nfc("\u{00E9}"), "\u{00E9}");
    assert_eq!(
        nfc("\u{0928}\u{0932} \u{0935}\u{093E}\u{0932}\u{093E}"),
        "\u{0928}\u{0932} \u{0935}\u{093E}\u{0932}\u{093E}"
    );
}

#[test]
fn search_key_case_and_idempotence() {
    assert_eq!(search_key("Nal WALA"), "nal vala");
    assert_eq!(search_key("E\u{301}"), "e");
    assert_eq!(search_key("É"), "e");
    assert_eq!(search_key("E\u{301}"), search_key("É"));

    let qila_composed = "\u{0958}\u{093F}\u{0932}\u{093E}";
    let qila_decomposed = "\u{0915}\u{093C}\u{093F}\u{0932}\u{093E}";
    assert_eq!(search_key(qila_composed), search_key(qila_decomposed));

    let samples = [
        "Chennai Plumber",
        "नल वाला",
        "சென்னை வாலா",
        "mixed भाषा and text 123",
        "E\u{301}léphant",
        "ÉLÉPHANT",
        "क़िला और किला",
        "ek do teen",
        "a-b and plumber's",
        "   leading and trailing spaces   ",
    ];
    for s in samples {
        let key1 = search_key(s);
        let key2 = search_key(&key1);
        assert_eq!(key1, key2, "search_key must be idempotent for sample: {s}");
    }
}

#[test]
fn search_key_is_idempotent_spelling_data() {
    let data = include_str!("tests/spelling-pairs.tsv");
    for line in data.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = trimmed.split('\t').collect();
        if cols.len() >= 5 {
            let native = cols[3];
            let latin = cols[4];
            if !native.is_empty() {
                let k1 = search_key(native);
                let k2 = search_key(&k1);
                assert_eq!(k1, k2, "Not idempotent for native: {native}");
            }
            if !latin.is_empty() {
                let k1 = search_key(latin);
                let k2 = search_key(&k1);
                assert_eq!(k1, k2, "Not idempotent for latin: {latin}");
            }
        }
    }
}

#[test]
fn search_key_breaks_words_at_punctuation() {
    assert_eq!(search_key("plumber's"), "plumbar s");
    assert_eq!(search_key("50%"), "50");
    assert_eq!(search_key("a-b"), "a b");
}

#[test]
fn search_key_script_runs_and_latin_base() {
    // é written as e + U+0301 is one Latin run and gives e
    let e_acute_decomposed = "e\u{0301}";
    assert_eq!(search_key(e_acute_decomposed), "e");

    // ø stays ø
    assert_eq!(search_key("ø"), "ø");

    // a digit after a Devanagari letter stays in its word
    assert_eq!(search_key("नल123"), "nal123");

    // mixed script words
    assert_eq!(search_key("नलwala"), "nalvala");

    // NUL inside text is a break
    assert_eq!(search_key("nal\0wala"), "nal vala");
}

#[test]
fn search_key_size_safety() {
    let dev_1kib = "नल वाला ".repeat(128);
    let tam_1kib = "சென்னை பிளம்பர் ".repeat(50);
    let lat_1kib = "plumber service ".repeat(64);

    assert!(!search_key(&dev_1kib).is_empty());
    assert!(!search_key(&tam_1kib).is_empty());
    assert!(!search_key(&lat_1kib).is_empty());

    let dev_64kib = "नल वाला ".repeat(8192);
    let tam_64kib = "சென்னை பிளம்பர் ".repeat(3200);
    let lat_64kib = "plumber service ".repeat(4096);

    assert!(!search_key(&dev_64kib).is_empty());
    assert!(!search_key(&tam_64kib).is_empty());
    assert!(!search_key(&lat_64kib).is_empty());
}

#[test]
#[ignore = "Performance measurement for pull request text"]
fn measure_search_key_64_kib() {
    let lat_64kib = "plumber service ".repeat(4096);
    let start = Instant::now();
    let key = search_key(&lat_64kib);
    let elapsed = start.elapsed();
    assert!(!key.is_empty());
    println!("search_key 64 KiB computed in: {elapsed:?}");
}

#[test]
fn query_terms_classification_and_cleaning() {
    assert_eq!(
        query_terms("  Nal   WALA "),
        QueryTerms { long: vec!["nal".to_string(), "vala".to_string()], short: vec![] }
    );

    assert_eq!(
        query_terms("ek do teen"),
        QueryTerms {
            long: vec!["tin".to_string()],
            short: vec!["ek".to_string(), "do".to_string()],
        }
    );

    // नल folds to nal (3 chars) -> long, वाला folds to vala (4 chars) -> long
    assert_eq!(
        query_terms("नल वाला"),
        QueryTerms { long: vec!["nal".to_string(), "vala".to_string()], short: vec![] }
    );

    // एक folds to ek (2 chars) -> short
    assert_eq!(
        query_terms("एक नल"),
        QueryTerms { long: vec!["nal".to_string()], short: vec!["ek".to_string()] }
    );

    assert_eq!(query_terms("--- !!"), QueryTerms { long: vec![], short: vec![] });

    assert_eq!(query_terms("nal nal"), QueryTerms { long: vec!["nal".to_string()], short: vec![] });

    assert_eq!(
        query_terms("plumber,"),
        QueryTerms { long: vec!["plumbar".to_string()], short: vec![] }
    );

    assert_eq!(query_terms("(ek)"), QueryTerms { long: vec![], short: vec!["ek".to_string()] });

    assert_eq!(query_terms("\"nal\""), QueryTerms { long: vec!["nal".to_string()], short: vec![] });

    assert_eq!(
        query_terms("plumber's"),
        QueryTerms { long: vec!["plumbar".to_string()], short: vec!["s".to_string()] }
    );

    assert_eq!(
        query_terms("a-b"),
        QueryTerms { long: vec![], short: vec!["a".to_string(), "b".to_string()] }
    );
}

#[test]
fn constants_match_specification() {
    assert_eq!(SHORT_TERM_MAX_CHARS, 2);
    assert_eq!(SEARCH_KEY_VERSION, 2);
}
