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

#[test]
fn check_tag_list_enforces_count_and_uniqueness() {
    let empty: [LangTag; 0] = [];
    assert_eq!(check_tag_list(&empty, MAX_MESSAGE_LANGUAGES), Ok(()));

    let one = [LangTag::parse("hi").expect("valid tag")];
    assert_eq!(check_tag_list(&one, MAX_MESSAGE_LANGUAGES), Ok(()));

    let three = [
        LangTag::parse("hi").expect("valid tag"),
        LangTag::parse("en").expect("valid tag"),
        LangTag::parse("ta").expect("valid tag"),
    ];
    assert_eq!(check_tag_list(&three, MAX_MESSAGE_LANGUAGES), Ok(()));

    let four = [
        LangTag::parse("hi").expect("valid tag"),
        LangTag::parse("en").expect("valid tag"),
        LangTag::parse("ta").expect("valid tag"),
        LangTag::parse("mr").expect("valid tag"),
    ];
    assert_eq!(check_tag_list(&four, MAX_MESSAGE_LANGUAGES), Err(TagListError::TooMany(3)));

    let dup = [LangTag::parse("hi").expect("valid tag"), LangTag::parse("hi").expect("valid tag")];
    assert_eq!(
        check_tag_list(&dup, MAX_MESSAGE_LANGUAGES),
        Err(TagListError::Duplicate("hi".into()))
    );

    let distinct =
        [LangTag::parse("hi").expect("valid tag"), LangTag::parse("hi-IN").expect("valid tag")];
    assert_eq!(check_tag_list(&distinct, MAX_MESSAGE_LANGUAGES), Ok(()));
}

#[test]
fn parse_message_languages_validates_tags_and_list() {
    let two = vec!["hi".to_string(), "en".to_string()];
    let res = parse_message_languages(&two).expect("valid tags");
    assert_eq!(res.len(), 2);
    assert_eq!(res[0].as_str(), "hi");
    assert_eq!(res[1].as_str(), "en");

    let non_canon = vec!["HI".to_string()];
    assert_eq!(
        parse_message_languages(&non_canon),
        Err(MessageLanguagesError::Tag(LangTagError::NotCanonical {
            given: "HI".into(),
            canonical: "hi".into(),
        }))
    );

    let malformed = vec!["hi_IN".to_string()];
    assert_eq!(
        parse_message_languages(&malformed),
        Err(MessageLanguagesError::Tag(LangTagError::Malformed("hi_IN".into())))
    );

    let four = vec!["hi".into(), "en".into(), "ta".into(), "mr".into()];
    assert_eq!(
        parse_message_languages(&four),
        Err(MessageLanguagesError::List(TagListError::TooMany(3)))
    );
}
