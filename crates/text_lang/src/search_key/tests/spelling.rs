use std::{collections::HashMap, fs};

use crate::{
    fts,
    search_key::{query_terms, search_key},
};

const SPELLING_DATA: &str = include_str!("spelling-pairs.tsv");
const GOLDEN_RESULTS: &str = include_str!("spelling-results.tsv");

const ACCEPTED_MERGES: &[(&str, &str, &str)] =
    &[("hi", "jharu", "jaru"), ("hi", "saree", "sadi"), ("ta", "ennai", "enna")];

fn test_matches(query: &str, target: &str) -> bool {
    let terms = query_terms(query);
    if terms.long.is_empty() && terms.short.is_empty() {
        return false;
    }

    let target_keys = fts::index_keys(target);
    let target_words: Vec<&str> = target_keys.words.split_whitespace().collect();

    for t in &terms.long {
        if !target_keys.joined.contains(t) {
            return false;
        }
    }

    for t in &terms.short {
        if !target_words.iter().any(|w| w.starts_with(t)) {
            return false;
        }
    }

    true
}

#[test]
fn required_spelling_rows_meet() {
    let dev_plumber = "नल वाला";
    assert!(test_matches("nal wala", dev_plumber));
    assert!(test_matches(dev_plumber, "nal wala"));

    assert!(test_matches("naal waala", dev_plumber));
    assert!(test_matches(dev_plumber, "naal waala"));

    assert!(test_matches("nalwala", dev_plumber));
    assert!(test_matches(dev_plumber, "nalwala"));

    let dev_ram = "राम";
    assert!(test_matches("ram", dev_ram));
    assert!(test_matches(dev_ram, "ram"));

    assert!(test_matches("Ram", dev_ram));
    assert!(test_matches(dev_ram, "Ram"));
}

struct Row<'a> {
    lang: &'a str,
    group: &'a str,
    kind: &'a str,
    native: &'a str,
    latin: &'a str,
}

#[test]
fn spelling_rows_match_the_owner_decisions() {
    let mut native_by_group: HashMap<(&str, &str), &str> = HashMap::new();
    let mut rows = Vec::new();

    for line in SPELLING_DATA.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = trimmed.split('\t').collect();
        if cols.len() < 5 {
            continue;
        }
        let lang = cols[0];
        let group = cols[1];
        let kind = cols[2];
        let native = cols[3];
        let latin = cols[4];

        if kind == "pair" && !native.is_empty() {
            native_by_group.insert((lang, group), native);
        }

        rows.push(Row { lang, group, kind, native, latin });
    }

    let mut hi_pairs_fwd = 0;
    let mut hi_pairs_rev = 0;
    let mut ta_pairs_fwd = 0;
    let mut ta_pairs_rev = 0;

    let mut actual_results = String::new();
    actual_results.push_str("# Generated with UPDATE_SPELLING_RESULTS=1. Do not edit manually.\n");

    for r in &rows {
        let native = if r.native.is_empty() {
            native_by_group.get(&(r.lang, r.group)).copied().unwrap_or("")
        } else {
            r.native
        };

        let latin_key = search_key(r.latin);
        let native_key = search_key(native);
        let meets = test_matches(r.latin, native);

        if r.kind == "distinct" {
            let is_accepted = ACCEPTED_MERGES
                .iter()
                .any(|&(l, g, lat)| l == r.lang && g == r.group && lat == r.latin);
            if !is_accepted {
                assert!(
                    !meets,
                    "Distinct row met unexpectedly: lang={}, group={}, latin={}, native={}",
                    r.lang, r.group, r.latin, native
                );
            }
        } else {
            actual_results.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                r.lang, r.group, r.kind, r.latin, latin_key, native_key, meets
            ));

            if r.kind == "pair" {
                let rev_meets = test_matches(native, r.latin);
                if r.lang == "hi" {
                    if meets {
                        hi_pairs_fwd += 1;
                    }
                    if rev_meets {
                        hi_pairs_rev += 1;
                    }
                } else if r.lang == "ta" {
                    if meets {
                        ta_pairs_fwd += 1;
                    }
                    if rev_meets {
                        ta_pairs_rev += 1;
                    }
                }
            }
        }
    }

    assert!(
        hi_pairs_fwd >= 160 && hi_pairs_rev >= 160,
        "Hindi pairs floor failed: fwd={hi_pairs_fwd}, rev={hi_pairs_rev} (min 160)"
    );
    assert!(
        ta_pairs_fwd >= 140 && ta_pairs_rev >= 140,
        "Tamil pairs floor failed: fwd={ta_pairs_fwd}, rev={ta_pairs_rev} (min 140)"
    );

    if std::env::var("UPDATE_SPELLING_RESULTS").as_deref() == Ok("1") {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let golden_path = format!("{manifest_dir}/src/search_key/tests/spelling-results.tsv");
        fs::write(&golden_path, &actual_results)
            .expect("Failed to write golden spelling-results.tsv");
    } else {
        assert_eq!(actual_results, GOLDEN_RESULTS, "spelling-results.tsv differences detected");
    }
}
