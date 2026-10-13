use std::fs;

use syneroym_core::curated_list::{CoordinatorEntry, CoordinatorTransport, CuratedList};

use super::{CoordinatorCache, load, save};

fn entry(url: &str) -> CoordinatorEntry {
    CoordinatorEntry {
        transport: CoordinatorTransport::IrohRelay,
        url: url.into(),
        info_url: "http://info.example".into(),
        operator: "Example".into(),
    }
}

fn cache(fetched_at_secs: u64, max_age_secs: u64) -> CoordinatorCache {
    CoordinatorCache {
        registry_url: "http://reg.example".into(),
        fetched_at_secs,
        list: CuratedList { max_age_secs, entries: vec![entry("http://relay.example")] },
        chosen: Some(entry("http://relay.example")),
    }
}

#[test]
fn save_then_load_gives_the_same_cache() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db").join("coordinators.json");
    let want = cache(100, 10);

    save(&path, &want).unwrap();

    assert_eq!(load(&path), Some(want));
}

#[test]
fn missing_file_is_none() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(load(&dir.path().join("coordinators.json")), None);
}

#[test]
fn corrupt_file_is_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("coordinators.json");
    fs::write(&path, "{").unwrap();

    assert_eq!(load(&path), None);
}

#[test]
fn save_leaves_no_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("coordinators.json");

    save(&path, &cache(100, 10)).unwrap();

    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["coordinators.json"]);
}

#[test]
fn is_fresh_ends_at_max_age() {
    let c = cache(100, 10);
    assert!(c.is_fresh(109));
    assert!(!c.is_fresh(110));
}

#[test]
fn refresh_chosen_takes_the_entry_of_the_list() {
    let mut c = cache(100, 10);
    let mut moved = entry("HTTP://Relay.Example:80/");
    moved.info_url = "http://moved.example".into();
    moved.operator = "New operator".into();
    c.list.entries = vec![entry("http://other.example"), moved];

    c.refresh_chosen();

    let chosen = c.chosen.unwrap();
    assert_eq!(chosen.url, "http://relay.example");
    assert_eq!(chosen.info_url, "http://moved.example");
    assert_eq!(chosen.operator, "New operator");
}

#[test]
fn refresh_chosen_keeps_the_entry_when_the_list_dropped_it() {
    let mut c = cache(100, 10);
    c.list.entries = vec![entry("http://other.example")];

    c.refresh_chosen();

    assert_eq!(c.chosen, Some(entry("http://relay.example")));
}

#[test]
fn lists_compares_normalised_urls() {
    let mut c = cache(100, 10);
    c.list.entries = vec![entry("HTTP://R.Example:80/")];

    assert!(c.lists("http://r.example"));
    assert!(!c.lists("http://other.example"));
}
