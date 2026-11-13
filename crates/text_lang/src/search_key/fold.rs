pub const RULES_2_3_4: &[(&str, &str)] = &[("x", "ks"), ("q", "k"), ("sch", "sk"), ("ea", "i")];

pub const RULES_6_8_10: &[(&str, &str)] = &[
    ("sh", "s"),
    ("zh", "l"),
    ("ph", "f"),
    ("ck", "k"),
    ("w", "v"),
    ("z", "j"),
    ("ee", "i"),
    ("oo", "u"),
];

const CH_MARKER: char = '\x01';
const C_LETTERS: &[char] = &['b', 'c', 'd', 'g', 'j', 'k', 'l', 'm', 'n', 'p', 'r', 's', 't', 'v'];

pub fn fold_word(w: &str) -> String {
    let mut current = w.to_string();
    loop {
        let next = fold_word_pass(&current);
        if next == current {
            return current;
        }
        current = next;
    }
}

pub fn fold_word_pass(w: &str) -> String {
    let s = apply_rules_2_to_5(w);
    let s = apply_rules_6_to_9(&s);
    let s = apply_rules_10_and_11(&s);
    apply_rules_12_and_13(&s)
}

pub fn apply_rules_2_to_5(w: &str) -> String {
    let mut s = w.to_string();
    for (pat, repl) in RULES_2_3_4 {
        s = s.replace(pat, repl);
    }
    s = s.replace("chh", "ch");
    s.replace("ch", &CH_MARKER.to_string())
}

pub fn apply_rules_6_to_9(w: &str) -> String {
    let mut s = w.to_string();
    for (pat, repl) in &RULES_6_8_10[..4] {
        s = s.replace(pat, repl);
    }

    let mut res = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        if chars[i] == 'c' {
            if i + 1 < n && matches!(chars[i + 1], 'e' | 'i' | 'y') {
                res.push('s');
            } else {
                res.push('k');
            }
        } else {
            res.push(chars[i]);
        }
        i += 1;
    }
    s = res;

    for (pat, repl) in &RULES_6_8_10[4..6] {
        s = s.replace(pat, repl);
    }

    let chars: Vec<char> = s.chars().collect();
    let mut res = String::with_capacity(chars.len());
    for (idx, &ch) in chars.iter().enumerate() {
        if ch == 'h' && idx > 0 && C_LETTERS.contains(&chars[idx - 1]) {
            continue;
        }
        res.push(ch);
    }

    res
}

pub fn apply_rules_10_and_11(w: &str) -> String {
    let mut s = w.to_string();
    for (pat, repl) in &RULES_6_8_10[6..] {
        s = s.replace(pat, repl);
    }

    let mut res = String::with_capacity(s.len());
    for ch in s.chars() {
        // Only Latin letters (and the `ch` marker) are spelling variants.
        // A doubled digit or a doubled letter of another script is data.
        let collapsible = ch.is_ascii_lowercase() || ch == CH_MARKER;
        if !(collapsible && res.ends_with(ch)) {
            res.push(ch);
        }
    }

    res.replace(CH_MARKER, "ch")
}

pub fn apply_rules_12_and_13(w: &str) -> String {
    let mut s = w.replace("yi", "i");

    let chars: Vec<char> = s.chars().collect();
    let mut res = String::with_capacity(chars.len());
    let n = chars.len();
    let mut i = 0;
    while i < n {
        if i + 1 < n
            && chars[i] == 'a'
            && chars[i + 1] == 'y'
            && (i + 2 == n || !matches!(chars[i + 2], 'a' | 'e' | 'i' | 'o' | 'u'))
        {
            res.push_str("ai");
            i += 2;
        } else {
            res.push(chars[i]);
            i += 1;
        }
    }
    s = res;

    if s.ends_with('y') {
        s.pop();
        s.push('i');
    }

    if s.ends_with("er") {
        s.pop();
        s.pop();
        s.push_str("ar");
    }
    if s.ends_with("ei") || s.ends_with("ey") {
        s.pop();
        s.pop();
        s.push_str("ai");
    }

    let chars: Vec<char> = s.chars().collect();
    let mut res = String::with_capacity(chars.len());
    let n = chars.len();
    let mut i = 0;
    while i < n {
        if i + 1 < n
            && chars[i] == 'i'
            && chars[i + 1] == 'y'
            && i + 2 < n
            && matches!(chars[i + 2], 'a' | 'e' | 'o' | 'u')
        {
            res.push('i');
            i += 2;
        } else {
            res.push(chars[i]);
            i += 1;
        }
    }

    res
}
