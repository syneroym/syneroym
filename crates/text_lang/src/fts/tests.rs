use crate::{
    fts::{IndexKeys, index_keys, match_exprs},
    search_key::QueryTerms,
};

#[test]
fn index_keys_formation() {
    let keys = index_keys("nal wala");
    assert_eq!(keys, IndexKeys { words: "nal vala".to_string(), joined: "nalvala".to_string() });
}

#[test]
fn match_exprs_variations() {
    let terms_long_only =
        QueryTerms { long: vec!["plumber".to_string(), "shop".to_string()], short: vec![] };
    let (long, short) = match_exprs("body", &terms_long_only);
    assert_eq!(long, Some("body : \"plumber\" AND body : \"shop\"".to_string()));
    assert_eq!(short, None);

    let terms_short_only =
        QueryTerms { long: vec![], short: vec!["ek".to_string(), "do".to_string()] };
    let (long, short) = match_exprs("body", &terms_short_only);
    assert_eq!(long, None);
    assert_eq!(short, Some("body : \"ek\"* AND body : \"do\"*".to_string()));

    let terms_both =
        QueryTerms { long: vec!["plumber".to_string()], short: vec!["ek".to_string()] };
    let (long, short) = match_exprs("body", &terms_both);
    assert_eq!(long, Some("body : \"plumber\"".to_string()));
    assert_eq!(short, Some("body : \"ek\"*".to_string()));

    let terms_with_quotes =
        QueryTerms { long: vec!["quo\"te".to_string()], short: vec!["q\"".to_string()] };
    let (long, short) = match_exprs("body", &terms_with_quotes);
    assert_eq!(long, Some("body : \"quo\"\"te\"".to_string()));
    assert_eq!(short, Some("body : \"q\"\"\"*".to_string()));
}
