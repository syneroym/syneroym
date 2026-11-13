pub const INDEPENDENT_VOWELS: &[(char, &str)] = &[
    ('\u{0B85}', "a"),
    ('\u{0B86}', "a"),
    ('\u{0B87}', "i"),
    ('\u{0B88}', "i"),
    ('\u{0B89}', "u"),
    ('\u{0B8A}', "u"),
    ('\u{0B8E}', "e"),
    ('\u{0B8F}', "e"),
    ('\u{0B90}', "ai"),
    ('\u{0B92}', "o"),
    ('\u{0B93}', "o"),
    ('\u{0B94}', "au"),
];

pub const VOWEL_SIGNS: &[(char, &str)] = &[
    ('\u{0BBE}', "a"),
    ('\u{0BBF}', "i"),
    ('\u{0BC0}', "i"),
    ('\u{0BC1}', "u"),
    ('\u{0BC2}', "u"),
    ('\u{0BC6}', "e"),
    ('\u{0BC7}', "e"),
    ('\u{0BC8}', "ai"),
    ('\u{0BCA}', "o"),
    ('\u{0BCB}', "o"),
    ('\u{0BCC}', "au"),
];

pub const NON_STOPS: &[(char, &str)] = &[
    ('\u{0B99}', "n"),
    ('\u{0B9E}', "n"),
    ('\u{0BA3}', "n"),
    ('\u{0BA8}', "n"),
    ('\u{0BAE}', "m"),
    ('\u{0BAF}', "y"),
    ('\u{0BB0}', "r"),
    ('\u{0BB2}', "l"),
    ('\u{0BB5}', "v"),
    ('\u{0BB4}', "zh"),
    ('\u{0BB3}', "l"),
    ('\u{0BB1}', "r"),
    ('\u{0BA9}', "n"),
];

pub const GRANTHA: &[(char, &str)] = &[
    ('\u{0B9C}', "j"),
    ('\u{0BB7}', "sh"),
    ('\u{0BB8}', "s"),
    ('\u{0BB9}', "h"),
    ('\u{0BB6}', "sh"),
];

pub const STOPS: &[char] = &['\u{0B95}', '\u{0B9A}', '\u{0B9F}', '\u{0BA4}', '\u{0BAA}'];

const PULLI: char = '\u{0BCD}';

pub struct RawWord {
    pub text: String,
    pub ends_with_pulli_stop: Option<char>,
    pub starts_with_stop: Option<char>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UnitKind {
    Vowel,
    ConsonantWithVowel,
    PulliStop,
    PulliNasal,
    PulliOther,
    Other,
}

struct Unit {
    latin: String,
    kind: UnitKind,
    char_base: char,
}

fn lookup(table: &[(char, &'static str)], c: char) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == c).map(|(_, v)| *v)
}

fn unvoiced_stop(c: char) -> &'static str {
    match c {
        '\u{0B95}' => "k",
        '\u{0B9A}' => "ch",
        '\u{0B9F}' => "t",
        '\u{0BA4}' => "t",
        '\u{0BAA}' => "p",
        _ => "",
    }
}

fn stop_by_position(
    c: char,
    is_first: bool,
    prev_kind: Option<UnitKind>,
    prev_char: Option<char>,
) -> &'static str {
    if is_first {
        match c {
            '\u{0B95}' => "k",
            '\u{0B9A}' => "s",
            '\u{0B9F}' => "d",
            '\u{0BA4}' => "t",
            '\u{0BAA}' => "p",
            _ => "",
        }
    } else if matches!(prev_kind, Some(UnitKind::Vowel | UnitKind::ConsonantWithVowel)) {
        match c {
            '\u{0B95}' => "g",
            '\u{0B9A}' => "s",
            '\u{0B9F}' => "d",
            '\u{0BA4}' => "dh",
            '\u{0BAA}' => "b",
            _ => "",
        }
    } else if prev_kind == Some(UnitKind::PulliNasal) {
        match c {
            '\u{0B95}' => "g",
            '\u{0B9A}' => "j",
            '\u{0B9F}' => "d",
            '\u{0BA4}' => "d",
            '\u{0BAA}' => "b",
            _ => "",
        }
    } else if prev_kind == Some(UnitKind::PulliStop) && prev_char == Some(c) {
        match c {
            '\u{0B95}' => "k",
            '\u{0B9A}' => "ch",
            '\u{0B9F}' => "t",
            '\u{0BA4}' => "t",
            '\u{0BAA}' => "p",
            _ => "",
        }
    } else {
        match c {
            '\u{0B95}' => "k",
            '\u{0B9A}' => "s",
            '\u{0B9F}' => "t",
            '\u{0BA4}' => "t",
            '\u{0BAA}' => "p",
            _ => "",
        }
    }
}

fn is_nasal(c: char) -> bool {
    matches!(c, '\u{0B99}' | '\u{0B9E}' | '\u{0BA3}' | '\u{0BA8}' | '\u{0BAE}' | '\u{0BA9}')
}

pub fn to_latin(run: &str) -> RawWord {
    let chars: Vec<char> = run.chars().collect();
    if chars.is_empty() {
        return RawWord { text: String::new(), ends_with_pulli_stop: None, starts_with_stop: None };
    }

    let units = parse_units(&chars);
    let mut out = String::new();
    for u in &units {
        out.push_str(&u.latin);
    }

    if out.ends_with("ey") {
        out.pop();
        out.pop();
        out.push_str("ai");
    }

    let ends_with_pulli_stop = units.last().and_then(|last| {
        if last.kind == UnitKind::PulliStop { Some(last.char_base) } else { None }
    });

    let starts_with_stop = if STOPS.contains(&chars[0]) { Some(chars[0]) } else { None };

    RawWord { text: out, ends_with_pulli_stop, starts_with_stop }
}

fn parse_special_pair(chars: &[char], i: &mut usize) -> Option<Unit> {
    let n = chars.len();
    if *i + 2 < n && chars[*i + 1] == PULLI && chars[*i + 2] == '\u{0BB1}' {
        let base_prefix = match chars[*i] {
            '\u{0BB1}' => "tr",
            '\u{0BA9}' => "ndr",
            _ => return None,
        };
        *i += 3;
        let mut vs = "a";
        let mut has_pulli = false;
        if *i < n && lookup(VOWEL_SIGNS, chars[*i]).is_some() {
            vs = lookup(VOWEL_SIGNS, chars[*i]).unwrap_or("");
            *i += 1;
        } else if *i < n && chars[*i] == PULLI {
            vs = "";
            has_pulli = true;
            *i += 1;
        }
        let kind = if has_pulli { UnitKind::PulliOther } else { UnitKind::ConsonantWithVowel };
        return Some(Unit { latin: format!("{base_prefix}{vs}"), kind, char_base: '\u{0BB1}' });
    }
    None
}

fn parse_stop_unit(
    chars: &[char],
    i: &mut usize,
    c: char,
    unit_idx: usize,
    loan_drop_i: bool,
    units: &[Unit],
) -> Unit {
    let n = chars.len();
    *i += 1;
    let mut has_pulli = false;
    let mut vs = "a";
    if *i < n && chars[*i] == PULLI {
        has_pulli = true;
        *i += 1;
    } else if *i < n && lookup(VOWEL_SIGNS, chars[*i]).is_some() {
        vs = lookup(VOWEL_SIGNS, chars[*i]).unwrap_or("");
        *i += 1;
    }

    if unit_idx == 0 && loan_drop_i && vs == "i" {
        vs = "";
    }

    if has_pulli {
        let unvoiced = unvoiced_stop(c);
        Unit { latin: unvoiced.to_string(), kind: UnitKind::PulliStop, char_base: c }
    } else {
        let prev_kind = units.last().map(|u| u.kind);
        let prev_char = units.last().map(|u| u.char_base);
        let val = stop_by_position(c, unit_idx == 0, prev_kind, prev_char);
        let kind = if vs.is_empty() { UnitKind::PulliStop } else { UnitKind::ConsonantWithVowel };
        Unit { latin: format!("{val}{vs}"), kind, char_base: c }
    }
}

/// `ஃப` is one sound, `f`: the stop after the aytham is read as `f` with the
/// vowel sign or pulli that follows it. `i` starts on the aytham.
fn parse_aytham_pa(chars: &[char], i: &mut usize, unit_idx: usize, units: &[Unit]) -> Unit {
    *i += 1;
    let stop = parse_stop_unit(chars, i, '\u{0BAA}', unit_idx + 1, false, units);
    let rest = stop.latin.chars().skip(1).collect::<String>();
    Unit { latin: format!("f{rest}"), kind: stop.kind, char_base: stop.char_base }
}

fn parse_units(chars: &[char]) -> Vec<Unit> {
    let loan_drop_i = chars.len() >= 3
        && matches!(chars[0], '\u{0BAA}' | '\u{0B9F}' | '\u{0B95}')
        && chars[1] == '\u{0BBF}'
        && matches!(chars[2], '\u{0BB0}' | '\u{0BB3}' | '\u{0BB2}');

    let mut units = Vec::new();
    let mut i = 0;
    let n = chars.len();
    let mut unit_idx = 0;

    while i < n {
        let c = chars[i];
        if ('\u{0BE6}'..='\u{0BEF}').contains(&c) {
            let digit = char::from_u32('0' as u32 + (c as u32 - 0x0BE6)).unwrap_or(c);
            units.push(Unit { latin: digit.to_string(), kind: UnitKind::Other, char_base: c });
            i += 1;
            unit_idx += 1;
            continue;
        }
        if c == '\u{0B83}' {
            if i + 1 < n && chars[i + 1] == '\u{0BAA}' {
                units.push(parse_aytham_pa(chars, &mut i, unit_idx, &units));
            } else {
                units.push(Unit { latin: "h".to_string(), kind: UnitKind::Other, char_base: c });
                i += 1;
            }
            unit_idx += 1;
            continue;
        }
        if let Some(v) = lookup(INDEPENDENT_VOWELS, c) {
            units.push(Unit { latin: v.to_string(), kind: UnitKind::Vowel, char_base: c });
            i += 1;
            unit_idx += 1;
            continue;
        }
        if let Some(pair_unit) = parse_special_pair(chars, &mut i) {
            units.push(pair_unit);
            unit_idx += 1;
            continue;
        }
        if STOPS.contains(&c) {
            let unit = parse_stop_unit(chars, &mut i, c, unit_idx, loan_drop_i, &units);
            units.push(unit);
            unit_idx += 1;
            continue;
        }
        let non_stop_val = lookup(NON_STOPS, c).or_else(|| lookup(GRANTHA, c));
        if let Some(val) = non_stop_val {
            i += 1;
            let mut has_pulli = false;
            let mut vs = "a";
            if i < n && chars[i] == PULLI {
                has_pulli = true;
                i += 1;
            } else if i < n && lookup(VOWEL_SIGNS, chars[i]).is_some() {
                vs = lookup(VOWEL_SIGNS, chars[i]).unwrap_or("");
                i += 1;
            }

            let kind = if has_pulli {
                if is_nasal(c) { UnitKind::PulliNasal } else { UnitKind::PulliOther }
            } else {
                UnitKind::ConsonantWithVowel
            };
            units.push(Unit {
                latin: if has_pulli { val.to_string() } else { format!("{val}{vs}") },
                kind,
                char_base: c,
            });
            unit_idx += 1;
            continue;
        }
        units.push(Unit { latin: c.to_string(), kind: UnitKind::Other, char_base: c });
        i += 1;
        unit_idx += 1;
    }
    units
}

pub fn sandhi(words: Vec<RawWord>) -> Vec<String> {
    let mut out = Vec::with_capacity(words.len());
    let len = words.len();

    for i in 0..len {
        let mut text = words[i].text.clone();
        let drops_stop = words[i]
            .ends_with_pulli_stop
            .filter(|&c| i + 1 < len && words[i + 1].starts_with_stop == Some(c));
        if let Some(stop_char) = drops_stop {
            let unvoiced = unvoiced_stop(stop_char);
            if text.ends_with(unvoiced) {
                text.truncate(text.len() - unvoiced.len());
            }
        }
        out.push(text);
    }

    out
}
