use crate::search_key::fold::fold_word;

#[test]
fn fold_rules_table() {
    // Rule 2
    assert_eq!(fold_word("taxi"), "taksi");
    assert_eq!(fold_word("iraq"), "irak");

    // Rule 3
    assert_eq!(fold_word("schema"), "skema");

    // Rule 4
    assert_eq!(fold_word("tea"), "ti");

    // Rule 5
    assert_eq!(fold_word("chhat"), "chat");

    // Rule 6
    assert_eq!(fold_word("shankar"), "sankar");
    assert_eq!(fold_word("tamizh"), "tamil");
    assert_eq!(fold_word("phone"), "fone");
    assert_eq!(fold_word("back"), "bak");

    // Rule 7
    assert_eq!(fold_word("cement"), "sement");
    assert_eq!(fold_word("doctor"), "doktor");

    // Rule 8
    assert_eq!(fold_word("wala"), "vala");
    assert_eq!(fold_word("darzi"), "darji");

    // Rule 9
    assert_eq!(fold_word("thali"), "tali");
    assert_eq!(fold_word("thakkali"), "takali");

    // Rule 10
    assert_eq!(fold_word("meen"), "min");
    assert_eq!(fold_word("koorai"), "kurai");

    // Rule 11
    assert_eq!(fold_word("naal"), "nal");
    assert_eq!(fold_word("waala"), "vala");

    // Rule 12
    assert_eq!(fold_word("bayi"), "bai");
    assert_eq!(fold_word("badhayi"), "badai");
    assert_eq!(fold_word("trichy"), "trichi");
    assert_eq!(fold_word("day"), "dai");

    // Rule 13
    assert_eq!(fold_word("teacher"), "tichar");
    assert_eq!(fold_word("kadei"), "kadai");
    assert_eq!(fold_word("kadey"), "kadai");
    assert_eq!(fold_word("takiya"), "takia");

    // Negative cases: ch keeps its h in rule 9, c in ch is not rule 7
    assert_eq!(fold_word("chat"), "chat");
}

#[test]
fn fold_fixed_cases() {
    assert_eq!(fold_word("aeiy"), "ai");
    assert_eq!(fold_word("kaey"), fold_word("kai"));
}

#[test]
fn fold_reaches_a_fixed_point() {
    let alpha1 = ['a', 'e', 'i', 'y', 'h', 'c', 'k', 's', 'r'];
    let mut word = Vec::new();
    test_idempotence_recurse(&alpha1, &mut word, 5);

    let alpha2 = ['a', 'e', 'i', 'o', 'u', 'y', 'h', 'c', 'k', 's', 'r', 'w', 'z', 'x'];
    let mut word2 = Vec::new();
    test_idempotence_recurse(&alpha2, &mut word2, 4);
}

fn test_idempotence_recurse(alpha: &[char], word: &mut Vec<char>, max_len: usize) {
    if !word.is_empty() {
        let s: String = word.iter().collect();
        let f1 = fold_word(&s);
        let f2 = fold_word(&f1);
        assert_eq!(f1, f2, "Fold not idempotent for string: {s}");
    }
    if word.len() < max_len {
        for &ch in alpha {
            word.push(ch);
            test_idempotence_recurse(alpha, word, max_len);
            word.pop();
        }
    }
}
