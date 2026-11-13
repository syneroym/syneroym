use icu_properties::{
    CodePointMapDataBorrowed,
    props::{GeneralCategory, GeneralCategoryGroup},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Devanagari,
    Tamil,
    Latin,
    Other,
}

fn is_latin_letter(c: char) -> bool {
    matches!(
        c,
        'a'..='z'
            | 'A'..='Z'
            | '\u{00C0}'..='\u{024F}'
            | '\u{1E00}'..='\u{1EFF}'
    )
}

pub fn script_runs<'a>(
    word: &'a str,
    gc_map: &CodePointMapDataBorrowed<'_, GeneralCategory>,
) -> Vec<(Script, &'a str)> {
    let mut runs = Vec::new();
    let mut cur_script: Option<Script> = None;
    let mut run_start = 0;
    let mut prev_end = 0;

    for (idx, c) in word.char_indices() {
        let char_len = c.len_utf8();
        let char_end = idx + char_len;

        let script = if ('\u{0900}'..='\u{097F}').contains(&c) {
            Script::Devanagari
        } else if ('\u{0B80}'..='\u{0BFF}').contains(&c) {
            Script::Tamil
        } else if is_latin_letter(c) {
            Script::Latin
        } else if GeneralCategoryGroup::Mark.contains(gc_map.get(c))
            || c.is_ascii_digit()
            || c.is_numeric()
        {
            cur_script.unwrap_or(Script::Other)
        } else {
            Script::Other
        };

        match cur_script {
            Some(s) if s == script => {
                prev_end = char_end;
            }
            Some(s) => {
                runs.push((s, &word[run_start..prev_end]));
                cur_script = Some(script);
                run_start = idx;
                prev_end = char_end;
            }
            None => {
                cur_script = Some(script);
                run_start = idx;
                prev_end = char_end;
            }
        }
    }

    if let Some(s) = cur_script {
        runs.push((s, &word[run_start..prev_end]));
    }

    runs
}
