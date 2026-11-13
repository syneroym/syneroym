use crate::search_key::{
    search_key,
    tamil::{GRANTHA, INDEPENDENT_VOWELS, NON_STOPS, VOWEL_SIGNS, to_latin},
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
    check(NON_STOPS, "NON_STOPS");
    check(GRANTHA, "GRANTHA");
}

#[test]
fn tamil_stops_table_positions() {
    // Row 1: first unit of the word (k, s, d, t, p)
    assert!(to_latin("கடை").text.starts_with('k'));
    assert!(to_latin("சட்டை").text.starts_with('s'));
    assert!(to_latin("டப்பா").text.starts_with('d'));
    assert!(to_latin("தலை").text.starts_with('t'));
    assert!(to_latin("பச்சை").text.starts_with('p'));

    // Row 2: after a vowel (g, s, d, dh, b)
    assert_eq!(to_latin("அகம்").text, "agam");
    assert_eq!(to_latin("அசுரன்").text, "asuran");
    assert_eq!(to_latin("படம்").text, "padam");
    assert_eq!(to_latin("பதம்").text, "padham");
    assert_eq!(to_latin("அபயம்").text, "abayam");

    // Row 3: after a nasal with pulli (g, j, d, d, b)
    assert_eq!(to_latin("அங்கம்").text, "angam");
    assert_eq!(to_latin("மஞ்சள்").text, "manjal");
    assert_eq!(to_latin("கண்டம்").text, "kandam");
    assert_eq!(to_latin("சந்தம்").text, "sandam");
    assert_eq!(to_latin("அன்பு").text, "anbu");

    // Row 4: after the same stop with pulli (doubled) (k, ch, t, t, p)
    assert_eq!(to_latin("அக்கா").text, "akka");
    assert_eq!(to_latin("பச்சை").text, "pachchai");
    assert_eq!(to_latin("பட்டம்").text, "pattam");
    assert_eq!(to_latin("பத்து").text, "pattu");
    assert_eq!(to_latin("அப்பம்").text, "appam");

    // Row 5: after any other pulli consonant (k, s, t, t, p)
    assert_eq!(to_latin("செய்க").text, "seyka");
    assert_eq!(to_latin("செய்ச").text, "seysa");
    assert_eq!(to_latin("செய்ட").text, "seyta");
    assert_eq!(to_latin("செய்த").text, "seyta");
    assert_eq!(to_latin("செய்ப").text, "seypa");
}

#[test]
fn tamil_special_pairs_and_endings() {
    assert_eq!(search_key("நன்றி"), "nandri");
    assert_eq!(search_key("வெற்றி"), "vetri");
    assert_eq!(to_latin("எண்ணெய்").text, "ennai");
    assert_eq!(search_key("எண்ணெய்"), "enai");
}

#[test]
fn tamil_loan_clusters() {
    assert_eq!(to_latin("பிரியா").text, "priya");
    assert_eq!(to_latin("டிரைவர்").text, "draivar");
    assert_eq!(search_key("திருச்சி"), "tiruchi");
}

#[test]
fn tamil_sandhi() {
    assert_eq!(search_key("கறிக் கடை"), "kari kadai");
}

#[test]
fn tamil_aytham() {
    assert_eq!(to_latin("அஃது").text, "ahtu");
    assert_eq!(to_latin("அஃப").text, "afa");
    assert_eq!(search_key("ஃபோன்"), "fon");
    // A search for `phone` meets it: the long term `fon` is inside `fone`.
    assert!(search_key("phone").contains(&search_key("ஃபோன்")));
}

#[test]
fn tamil_digits() {
    assert_eq!(search_key("௦௧௨௩௪௫௬௭௮௯"), "0123456789");
}

#[test]
fn tamil_composed_vowel_signs_equivalence() {
    let decomposed = "\u{0B95}\u{0BC6}\u{0BBE}";
    let composed = "\u{0B95}\u{0BCA}";
    assert_eq!(search_key(decomposed), search_key(composed));
}
