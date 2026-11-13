use crate::search_key::{
    devanagari::{CONSONANTS, INDEPENDENT_VOWELS, NUKTA_CONSONANTS, SIGNS, VOWEL_SIGNS, to_latin},
    search_key,
};

#[test]
fn letter_and_sign_table_values_are_ascii_lowercase() {
    let check = |table: &[(char, &str)], name: &str| {
        for (c, val) in table {
            assert!(
                val.chars().all(|ch| ch.is_ascii_lowercase()),
                "Table {name} entry for {c} has non-ascii-lowercase: {val}"
            );
        }
    };
    check(INDEPENDENT_VOWELS, "INDEPENDENT_VOWELS");
    check(VOWEL_SIGNS, "VOWEL_SIGNS");
    check(CONSONANTS, "CONSONANTS");
    check(NUKTA_CONSONANTS, "NUKTA_CONSONANTS");
    check(SIGNS, "SIGNS");
}

#[test]
fn all_devanagari_table_characters_mapped() {
    for &(c, exp) in INDEPENDENT_VOWELS {
        let s = c.to_string();
        assert_eq!(to_latin(&s), exp, "Failed for independent vowel {c}");
    }
    for &(c, exp) in VOWEL_SIGNS {
        let s = format!("क{c}");
        assert_eq!(to_latin(&s), format!("k{exp}"), "Failed for vowel sign {c}");
    }
    for &(c, exp) in CONSONANTS {
        let s = format!("{c}्");
        assert_eq!(to_latin(&s), exp, "Failed for consonant {c}");
    }
    for &(c, exp) in NUKTA_CONSONANTS {
        let s = format!("{c}़्");
        assert_eq!(to_latin(&s), exp, "Failed for nukta consonant {c}");
    }
    for &(c, exp) in SIGNS {
        let s = c.to_string();
        assert_eq!(to_latin(&s), exp, "Failed for sign {c}");
    }
}

#[test]
fn devanagari_schwa_deletion() {
    assert_eq!(search_key("नल"), "nal");
    assert_eq!(search_key("न"), "na");
    assert_eq!(search_key("कमरा"), "kamra");
    assert_eq!(search_key("बिजली"), "bijli");
    assert_eq!(search_key("राज मिस्त्री"), "raj mistri");
}

#[test]
fn devanagari_special_and_unmapped_letters() {
    assert_eq!(to_latin("ऩ्"), "n");
    assert_eq!(to_latin("ऱ्"), "r");
    assert_eq!(to_latin("ऴ्"), "l");
    assert_eq!(to_latin("ऽ"), "");
    assert_eq!(to_latin("ॐ"), "om");
    assert_eq!(to_latin("ॠ"), "ॠ");
    assert_eq!(to_latin("ॄ"), "ॄ");
}

#[test]
fn devanagari_nukta_composition_equivalence() {
    let qila_single = "\u{0958}\u{093F}\u{0932}\u{093E}";
    let qila_decomposed = "\u{0915}\u{093C}\u{093F}\u{0932}\u{093E}";
    assert_eq!(search_key(qila_single), search_key(qila_decomposed));
    assert_eq!(search_key(qila_single), "kila");
}

#[test]
fn devanagari_zwnj_zwj_and_danda() {
    let with_zwnj = "स्\u{200C}कूल";
    let without_zwnj = "स्कूल";
    assert_eq!(search_key(with_zwnj), search_key(without_zwnj));

    let with_zwj = "क्\u{200D}त";
    let without_zwj = "क्त";
    assert_eq!(search_key(with_zwj), search_key(without_zwj));

    assert_eq!(search_key("नल।वाला"), "nal vala");
    assert_eq!(search_key("नल॥वाला"), "nal vala");
}

#[test]
fn devanagari_digits() {
    assert_eq!(search_key("०१२३४५६७८९"), "0123456789");
    assert_eq!(search_key("नल०१"), "nal01");
}
