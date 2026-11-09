use std::str::FromStr;

use super::*;

#[test]
fn tag_table_parse_canonical_syntax_forms() {
    let cases = [
        ("hi", "hi"),
        ("HI", "hi"),
        ("hi-latn", "hi-Latn"),
        ("hi-Latn-IN", "hi-Latn-IN"),
        ("ta-IN", "ta-IN"),
        ("und", "und"),
        ("hi-Deva", "hi-Deva"),
        ("zh-hant-tw", "zh-Hant-TW"),
        ("en-u-nu-deva", "en-u-nu-deva"),
        ("en-us", "en-US"),
        ("iw", "iw"),
    ];

    for (input, expected) in cases {
        assert_eq!(
            LangTag::parse(input).as_ref().map(LangTag::as_str),
            Ok(expected),
            "parse failed for {input}"
        );
    }
}

#[test]
fn tag_table_malformed_inputs() {
    let malformed = ["hi_IN", "x-private", "toolongsubtag", ""];
    for input in malformed {
        assert_eq!(LangTag::parse(input), Err(LangTagError::Malformed(input.to_string())),);
    }
}

#[test]
fn tag_exceeding_max_len_is_too_long() {
    let input_36 = "a".repeat(36);
    assert_eq!(LangTag::parse(&input_36), Err(LangTagError::TooLong));
}

#[test]
fn parse_canonical_enforces_exact_form() {
    assert_eq!(
        LangTag::parse_canonical("hi-latn"),
        Err(LangTagError::NotCanonical {
            given: "hi-latn".to_string(),
            canonical: "hi-Latn".to_string(),
        })
    );
    assert_eq!(LangTag::parse_canonical("hi-Latn").as_ref().map(LangTag::as_str), Ok("hi-Latn"));
}

#[test]
fn serde_round_trip_and_canonical_enforcement() {
    let canonical = "\"hi-Latn\"";
    let deserialized: Result<LangTag, _> = serde_json::from_str(canonical);
    assert!(deserialized.is_ok());

    let tag = deserialized.ok();
    assert_eq!(tag.as_ref().map(LangTag::as_str), Some("hi-Latn"));
    assert_eq!(
        tag.as_ref().map(|t| serde_json::to_string(t).unwrap_or_default()),
        Some(canonical.to_string())
    );

    let non_canonical = "\"hi-latn\"";
    let res: Result<LangTag, _> = serde_json::from_str(non_canonical);
    assert!(
        matches!(res, Err(ref e) if e.to_string().contains("is not in canonical form; send 'hi-Latn'")),
        "expected not canonical error"
    );
}

#[test]
fn display_and_as_ref_and_from_str() {
    let tag = <LangTag as FromStr>::from_str("hi-latn");
    assert_eq!(tag.as_ref().map(LangTag::to_string), Ok("hi-Latn".to_string()));
    assert_eq!(tag.as_ref().map(|t| t.as_ref()), Ok("hi-Latn"));
}
