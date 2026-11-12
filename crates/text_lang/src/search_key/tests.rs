use super::{QueryTerms, SEARCH_KEY_VERSION, SHORT_TERM_MAX_CHARS, nfc, query_terms, search_key};

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
    assert_eq!(search_key("Nal WALA"), "nal wala");
    assert_eq!(search_key("E\u{301}"), "é");
    assert_eq!(search_key("É"), "é");
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
fn query_terms_classification_and_cleaning() {
    assert_eq!(
        query_terms("  Nal   WALA "),
        QueryTerms { long: vec!["nal".to_string(), "wala".to_string()], short: vec![] }
    );

    assert_eq!(
        query_terms("ek do teen"),
        QueryTerms {
            long: vec!["teen".to_string()],
            short: vec!["ek".to_string(), "do".to_string()],
        }
    );

    assert_eq!(
        query_terms("नल वाला"),
        QueryTerms { long: vec!["वाला".to_string()], short: vec!["नल".to_string()] }
    );

    assert_eq!(query_terms("--- !!"), QueryTerms { long: vec![], short: vec![] });

    assert_eq!(query_terms("nal nal"), QueryTerms { long: vec!["nal".to_string()], short: vec![] });

    assert_eq!(
        query_terms("plumber,"),
        QueryTerms { long: vec!["plumber".to_string()], short: vec![] }
    );

    assert_eq!(query_terms("(ek)"), QueryTerms { long: vec![], short: vec!["ek".to_string()] });

    assert_eq!(query_terms("\"nal\""), QueryTerms { long: vec!["nal".to_string()], short: vec![] });

    assert_eq!(
        query_terms("plumber's"),
        QueryTerms { long: vec!["plumber's".to_string()], short: vec![] }
    );

    assert_eq!(query_terms("a-b"), QueryTerms { long: vec!["a-b".to_string()], short: vec![] });

    assert_eq!(query_terms("நல்"), QueryTerms { long: vec!["நல்".to_string()], short: vec![] });

    assert_eq!(query_terms("प्"), QueryTerms { long: vec![], short: vec!["प्".to_string()] });

    assert_eq!(query_terms("नल।"), QueryTerms { long: vec![], short: vec!["नल".to_string()] });
}

#[test]
fn constants_match_specification() {
    assert_eq!(SHORT_TERM_MAX_CHARS, 2);
    assert_eq!(SEARCH_KEY_VERSION, 1);
}
