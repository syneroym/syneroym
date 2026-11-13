pub const INDEPENDENT_VOWELS: &[(char, &str)] = &[
    ('\u{0905}', "a"),
    ('\u{0906}', "a"),
    ('\u{0907}', "i"),
    ('\u{0908}', "i"),
    ('\u{0909}', "u"),
    ('\u{090A}', "u"),
    ('\u{090B}', "ri"),
    ('\u{090D}', "e"),
    ('\u{090F}', "e"),
    ('\u{0910}', "ai"),
    ('\u{0911}', "o"),
    ('\u{0913}', "o"),
    ('\u{0914}', "au"),
];

pub const VOWEL_SIGNS: &[(char, &str)] = &[
    ('\u{093E}', "a"),
    ('\u{093F}', "i"),
    ('\u{0940}', "i"),
    ('\u{0941}', "u"),
    ('\u{0942}', "u"),
    ('\u{0943}', "ri"),
    ('\u{0945}', "e"),
    ('\u{0947}', "e"),
    ('\u{0948}', "ai"),
    ('\u{0949}', "o"),
    ('\u{094B}', "o"),
    ('\u{094C}', "au"),
];

pub const CONSONANTS: &[(char, &str)] = &[
    ('\u{0915}', "k"),
    ('\u{0916}', "kh"),
    ('\u{0917}', "g"),
    ('\u{0918}', "gh"),
    ('\u{0919}', "n"),
    ('\u{091A}', "ch"),
    ('\u{091B}', "chh"),
    ('\u{091C}', "j"),
    ('\u{091D}', "jh"),
    ('\u{091E}', "n"),
    ('\u{091F}', "t"),
    ('\u{0920}', "th"),
    ('\u{0921}', "d"),
    ('\u{0922}', "dh"),
    ('\u{0923}', "n"),
    ('\u{0924}', "t"),
    ('\u{0925}', "th"),
    ('\u{0926}', "d"),
    ('\u{0927}', "dh"),
    ('\u{0928}', "n"),
    ('\u{0929}', "n"),
    ('\u{092A}', "p"),
    ('\u{092B}', "ph"),
    ('\u{092C}', "b"),
    ('\u{092D}', "bh"),
    ('\u{092E}', "m"),
    ('\u{092F}', "y"),
    ('\u{0930}', "r"),
    ('\u{0931}', "r"),
    ('\u{0932}', "l"),
    ('\u{0933}', "l"),
    ('\u{0934}', "l"),
    ('\u{0935}', "v"),
    ('\u{0936}', "sh"),
    ('\u{0937}', "sh"),
    ('\u{0938}', "s"),
    ('\u{0939}', "h"),
];

pub const NUKTA_CONSONANTS: &[(char, &str)] = &[
    ('\u{0915}', "q"),
    ('\u{0916}', "kh"),
    ('\u{0917}', "g"),
    ('\u{091C}', "z"),
    ('\u{0921}', "d"),
    ('\u{0922}', "dh"),
    ('\u{092B}', "f"),
    ('\u{092F}', "y"),
];

pub const SIGNS: &[(char, &str)] = &[('\u{0901}', "n"), ('\u{0902}', "n"), ('\u{0903}', "h")];

fn lookup(table: &[(char, &'static str)], c: char) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == c).map(|(_, v)| *v)
}

struct Unit {
    latin: String,
    has_vowel: bool,
    is_inherent: bool,
    is_consonant: bool,
}

pub fn to_latin(run: &str) -> String {
    let chars: Vec<char> = run.chars().collect();
    let mut units = parse_units(&chars);
    apply_schwa_deletion(&mut units);

    let mut out = String::new();
    for u in units {
        out.push_str(&u.latin);
    }
    out
}

fn parse_units(chars: &[char]) -> Vec<Unit> {
    let mut units = Vec::new();
    let mut i = 0;
    let n = chars.len();

    while i < n {
        let c = chars[i];
        if c == '\u{093D}' {
            i += 1;
            continue;
        }
        if c == '\u{0950}' {
            units.push(Unit {
                latin: "om".to_string(),
                has_vowel: true,
                is_inherent: false,
                is_consonant: false,
            });
            i += 1;
            continue;
        }
        if ('\u{0966}'..='\u{096F}').contains(&c) {
            let digit = char::from_u32('0' as u32 + (c as u32 - 0x0966)).unwrap_or(c);
            units.push(Unit {
                latin: digit.to_string(),
                has_vowel: false,
                is_inherent: false,
                is_consonant: false,
            });
            i += 1;
            continue;
        }
        if let Some(v) = lookup(INDEPENDENT_VOWELS, c) {
            units.push(Unit {
                latin: v.to_string(),
                has_vowel: true,
                is_inherent: false,
                is_consonant: false,
            });
            i += 1;
            continue;
        }
        if let Some(s) = lookup(SIGNS, c) {
            units.push(Unit {
                latin: s.to_string(),
                has_vowel: false,
                is_inherent: false,
                is_consonant: false,
            });
            i += 1;
            continue;
        }
        if let Some(base_val) = lookup(CONSONANTS, c) {
            let mut val = base_val;
            i += 1;
            if i < n && chars[i] == '\u{093C}' {
                if let Some(nukta_val) = lookup(NUKTA_CONSONANTS, c) {
                    val = nukta_val;
                }
                i += 1;
            }
            if i < n && lookup(VOWEL_SIGNS, chars[i]).is_some() {
                let vs = lookup(VOWEL_SIGNS, chars[i]).unwrap_or("");
                units.push(Unit {
                    latin: format!("{val}{vs}"),
                    has_vowel: true,
                    is_inherent: false,
                    is_consonant: true,
                });
                i += 1;
            } else if i < n && chars[i] == '\u{094D}' {
                units.push(Unit {
                    latin: val.to_string(),
                    has_vowel: false,
                    is_inherent: false,
                    is_consonant: true,
                });
                i += 1;
            } else {
                units.push(Unit {
                    latin: format!("{val}a"),
                    has_vowel: true,
                    is_inherent: true,
                    is_consonant: true,
                });
            }
            continue;
        }
        if c == '\u{093C}' || c == '\u{094D}' {
            i += 1;
            continue;
        }
        units.push(Unit {
            latin: c.to_string(),
            has_vowel: false,
            is_inherent: false,
            is_consonant: false,
        });
        i += 1;
    }

    units
}

fn apply_schwa_deletion(units: &mut [Unit]) {
    let vowel_count = units.iter().filter(|u| u.has_vowel).count();
    if vowel_count < 2 {
        return;
    }

    let mut last_vowel_idx = units.len();
    for (idx, u) in units.iter().enumerate().rev() {
        if u.has_vowel {
            last_vowel_idx = idx;
            break;
        }
    }
    if last_vowel_idx < units.len()
        && units[last_vowel_idx].is_inherent
        && units[last_vowel_idx].latin.ends_with('a')
    {
        units[last_vowel_idx].latin.pop();
        units[last_vowel_idx].has_vowel = false;
        units[last_vowel_idx].is_inherent = false;
    }

    if units.len() >= 3 {
        for idx in (1..=units.len() - 2).rev() {
            if units[idx].is_inherent
                && before_and_after_vowel(units, idx)
                && units[idx].latin.ends_with('a')
            {
                units[idx].latin.pop();
                units[idx].has_vowel = false;
                units[idx].is_inherent = false;
            }
        }
    }
}

fn before_and_after_vowel(units: &[Unit], idx: usize) -> bool {
    let before_has_vowel = units[idx - 1].has_vowel;
    let after_is_consonant_with_vowel = units[idx + 1].is_consonant && units[idx + 1].has_vowel;
    before_has_vowel && after_is_consonant_with_vowel
}
