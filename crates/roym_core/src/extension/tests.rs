use std::{collections::HashMap, fs, path::PathBuf};

use serde_json::{Value, json};

use super::{
    ExtensionRef, MANIFESTS, ManifestError, ManifestKind, Registry, SCHEDULING_EXTENSION,
    SCHEDULING_SLOT_SCHEMA, ids, registry,
};
use crate::deal::ResourceKind;

fn ext(id: &str, version: u32) -> ExtensionRef {
    ExtensionRef { id: id.to_string(), version }
}

fn parse_one(manifest: &Value) -> Result<Registry, ManifestError> {
    Registry::parse(&[&manifest.to_string()])
}

fn refused(manifest: &Value) -> String {
    match parse_one(manifest) {
        Ok(_) => panic!("manifest should not load: {manifest}"),
        Err(e) => e.to_string(),
    }
}

/// A small workflow manifest that loads, for the tests to break one rule at a
/// time.
fn base() -> Value {
    json!({
        "id": "demo",
        "version": 1,
        "kind": "workflow",
        "resource_kinds": ["goods-or-service"],
        "hooks": [],
        "details": [{
            "schema": "demo/form@1",
            "fields": [{ "name": "note", "type": "text", "max_len": 100 }]
        }],
        "steps": [{ "type": "demo/done", "by": "either", "fields": [] }]
    })
}

fn with_field(field: Value) -> Value {
    let mut m = base();
    m["details"][0]["fields"] = json!([field]);
    m
}

#[test]
fn every_shipped_manifest_loads() {
    let loaded = Registry::parse(MANIFESTS).unwrap();
    assert_eq!(loaded.manifests().len(), 2);
    for (id, version) in [(SCHEDULING_EXTENSION, 1), ("one-shot", 1)] {
        let from_parse = loaded.workflow(&ext(id, version)).unwrap();
        let from_static = registry().workflow(&ext(id, version)).unwrap();
        assert_eq!(from_parse, from_static);
    }
}

#[test]
fn shipped_manifests_say_what_they_allow() {
    let scheduling = registry().workflow(&ext("scheduling", 1)).unwrap();
    assert!(scheduling.allows(ResourceKind::ServiceInSlot));
    assert!(!scheduling.allows(ResourceKind::GoodsOrService));
    assert!(scheduling.has_reserve_hook());
    let one_shot = registry().workflow(&ext("one-shot", 1)).unwrap();
    assert!(one_shot.allows(ResourceKind::GoodsOrService));
    assert!(!one_shot.has_reserve_hook());
    assert!(registry().workflow(&ext("scheduling", 2)).is_none());
    assert!(registry().workflow(&ext("future", 1)).is_none());
    assert!(registry().details_schema(&ext("scheduling", 1), SCHEDULING_SLOT_SCHEMA).is_some());
    assert!(registry().details_schema(&ext("scheduling", 1), "scheduling/other@1").is_none());
    assert!(registry().step_type(&ext("scheduling", 1), "scheduling/started").is_some());
}

#[test]
fn a_good_base_manifest_loads() {
    parse_one(&base()).unwrap();
}

#[test]
fn a_bad_id_is_refused() {
    for id in ["", "Demo", "has space", "a_b", &"a".repeat(33)] {
        let mut m = base();
        m["id"] = json!(id);
        assert!(refused(&m).contains("id must be"), "id {id:?}");
    }
}

#[test]
fn a_repeated_version_is_refused() {
    let one = base().to_string();
    let err = Registry::parse(&[&one, &one]).unwrap_err().to_string();
    assert!(err.contains("listed twice"), "{err}");
}

#[test]
fn version_zero_is_refused() {
    let mut m = base();
    m["version"] = json!(0);
    assert!(refused(&m).contains("version"));
}

#[test]
fn schema_names_need_the_id_prefix_and_a_version() {
    for name in ["form@1", "other/form@1", "demo/form", "demo/form@0", "demo/form@x"] {
        let mut m = base();
        m["details"][0]["schema"] = json!(name);
        assert!(!refused(&m).is_empty(), "schema {name:?}");
    }
}

#[test]
fn step_types_need_the_id_prefix_and_a_limit() {
    for step in ["done", "other/done", &format!("demo/{}", "a".repeat(60))] {
        let mut m = base();
        m["steps"][0]["type"] = json!(step);
        assert!(!refused(&m).is_empty(), "step {step:?}");
    }
}

#[test]
fn a_repeated_field_name_is_refused() {
    let mut m = base();
    m["details"][0]["fields"] = json!([
        { "name": "a", "type": "text", "max_len": 10 },
        { "name": "a", "type": "text", "max_len": 10 }
    ]);
    assert!(refused(&m).contains("listed twice"));
}

#[test]
fn field_limits_are_load_rules() {
    let cases = [
        json!({ "name": "t", "type": "text" }),
        json!({ "name": "t", "type": "text", "max_len": 513 }),
        json!({ "name": "t", "type": "long-text", "max_len": 2049 }),
        json!({ "name": "c", "type": "code", "max_len": 129 }),
        json!({ "name": "c", "type": "code" }),
        json!({ "name": "n", "type": "number", "min": 5, "max": 1 }),
        json!({ "name": "n", "type": "number", "max_len": 5 }),
        json!({ "name": "t", "type": "text", "max_len": 5, "min": 1 }),
        json!({ "name": "k", "type": "choice" }),
        json!({ "name": "k", "type": "choice", "choices": [] }),
        json!({ "name": "k", "type": "choice", "choices": ["a", "a"] }),
        json!({ "name": "k", "type": "choice", "choices": ["Not A Code"] }),
        json!({ "name": "t", "type": "text", "max_len": 5, "choices": ["a"] }),
        json!({ "name": "Bad", "type": "text", "max_len": 5 }),
        json!({ "name": "", "type": "text", "max_len": 5 }),
    ];
    for field in cases {
        let message = refused(&with_field(field.clone()));
        assert!(message.contains("field"), "{field}: {message}");
    }
}

#[test]
fn field_limits_at_the_ceiling_load() {
    for field in [
        json!({ "name": "t", "type": "text", "max_len": 512 }),
        json!({ "name": "t", "type": "long-text", "max_len": 2048 }),
        json!({ "name": "c", "type": "code", "max_len": 128 }),
        json!({ "name": "n", "type": "number", "min": -5, "max": 5 }),
        json!({ "name": "k", "type": "choice", "choices": ["a", "b-c"] }),
        json!({ "name": "m", "type": "money" }),
        json!({ "name": "d", "type": "date-time" }),
        json!({ "name": "u", "type": "duration" }),
    ] {
        parse_one(&with_field(field)).unwrap();
    }
}

#[test]
fn hooks_are_all_three_or_none() {
    for hooks in [
        json!(["reserve"]),
        json!(["reserve", "confirm"]),
        json!(["reserve", "reserve", "confirm"]),
    ] {
        let mut m = base();
        m["hooks"] = hooks;
        assert!(refused(&m).contains("hooks"));
    }
    let mut m = base();
    m["hooks"] = json!(["reserve", "confirm", "release"]);
    parse_one(&m).unwrap();
}

#[test]
fn service_in_slot_needs_the_hooks() {
    let mut m = base();
    m["resource_kinds"] = json!(["service-in-slot"]);
    assert!(refused(&m).contains("service-in-slot"));
}

#[test]
fn an_unknown_check_field_type_or_key_does_not_load() {
    let mut m = base();
    m["details"][0]["checks"] = json!(["no-such-check"]);
    assert!(matches!(parse_one(&m), Err(ManifestError::Format(_))));
    // `location` is not built.
    let m = with_field(json!({ "name": "where", "type": "location" }));
    assert!(matches!(parse_one(&m), Err(ManifestError::Format(_))));
    let mut m = base();
    m["widgets"] = json!([]);
    assert!(matches!(parse_one(&m), Err(ManifestError::Format(_))));
}

#[test]
fn message_ids_are_derived_as_written() {
    assert_eq!(
        ids::field_label_id("scheduling", SCHEDULING_SLOT_SCHEMA, "listing_id"),
        "ext-scheduling-slot-listing-id"
    );
    assert_eq!(
        ids::step_name_id("scheduling", "scheduling/started"),
        "ext-scheduling-step-started"
    );
    assert_eq!(
        ids::choice_label_id("a-b", "a-b/some_form@2", "kind_of", "big-one"),
        "ext-a-b-some-form-kind-of-big-one"
    );
    let scheduling = registry().workflow(&ext("scheduling", 1)).unwrap();
    assert_eq!(
        ids::manifest_message_ids(scheduling),
        [
            "ext-scheduling-slot-listing-id",
            "ext-scheduling-slot-slot-id",
            "ext-scheduling-step-started"
        ]
    );
}

#[test]
fn two_manifests_that_make_one_message_id_are_refused() {
    // `a-b` with schema `c` and field `d`, and `a` with schema `b-c` and
    // field `d`, both give `ext-a-b-c-d`.
    let first = json!({
        "id": "a-b", "version": 1, "kind": "workflow",
        "details": [{ "schema": "a-b/c@1", "fields": [{ "name": "d", "type": "text", "max_len": 5 }] }]
    });
    let second = json!({
        "id": "a", "version": 1, "kind": "workflow",
        "details": [{ "schema": "a/b-c@1", "fields": [{ "name": "d", "type": "text", "max_len": 5 }] }]
    });
    let err = Registry::parse(&[&first.to_string(), &second.to_string()]).unwrap_err().to_string();
    assert!(err.contains("ext-a-b-c-d"), "{err}");
}

fn insight() -> Value {
    json!({ "id": "digest", "version": 1, "kind": "insight", "reads": ["event-log"] })
}

#[test]
fn insight_manifest_loads_and_cannot_be_named() {
    let loaded = parse_one(&insight()).unwrap();
    let reference = ext("digest", 1);
    let manifest = loaded.get(&reference).unwrap();
    assert_eq!(manifest.kind, ManifestKind::Insight);
    assert!(loaded.workflow(&reference).is_none());
    assert!(loaded.details_schema(&reference, "digest/x@1").is_none());
    assert!(loaded.step_type(&reference, "digest/x").is_none());
}

#[test]
fn insight_and_workflow_keys_do_not_mix() {
    let mut m = insight();
    m["steps"] = json!([{ "type": "digest/x", "by": "giver", "fields": [] }]);
    assert!(refused(&m).contains("insight"));
    let mut m = insight();
    m["reads"] = json!([]);
    assert!(refused(&m).contains("reads"));
    let mut m = base();
    m["reads"] = json!(["event-log"]);
    assert!(refused(&m).contains("reads"));
}

/// The Hub imports the same JSON files, in the same order, as `MANIFESTS`.
#[test]
fn the_hub_registry_lists_the_manifests_that_this_crate_compiles_in() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../roym_web/ui/src/extensions/registry.ts");
    let ts = fs::read_to_string(&path).expect("read the Hub's extension registry");

    let mut files_by_name: HashMap<&str, &str> = HashMap::new();
    for line in ts.lines().filter(|l| l.starts_with("import ")) {
        let name = line.split_whitespace().nth(1).expect("import name");
        let file = line.split("/roym_core/extensions/").nth(1).expect("import path");
        files_by_name.insert(name, file.split('"').next().expect("file name"));
    }
    let array = ts
        .lines()
        .find(|l| l.contains("MANIFESTS: readonly Manifest[]"))
        .expect("the MANIFESTS array");
    let inside = array.split('[').nth(2).and_then(|rest| rest.split(']').next()).expect("array");
    let hub_files: Vec<&str> = inside.split(',').map(|n| files_by_name[n.trim()]).collect();

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("extensions");
    let hub_sources: Vec<String> = hub_files
        .iter()
        .map(|file| fs::read_to_string(dir.join(file)).expect("read a manifest file"))
        .collect();
    assert_eq!(hub_sources, MANIFESTS);
}
