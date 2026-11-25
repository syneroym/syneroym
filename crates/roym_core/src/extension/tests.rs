use std::{collections::HashMap, fs, path::PathBuf};

use serde_json::{Value, json};

use super::{
    ExtensionRef, FieldType, MANIFESTS, ManifestError, ManifestKind, Registry,
    SCHEDULING_EXTENSION, SCHEDULING_SLOT_SCHEMA, ids, registry,
};
use crate::{deal::ResourceKind, services};

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
    assert_eq!(loaded.manifests().len(), 3);
    for (id, version) in [(SCHEDULING_EXTENSION, 1), ("one-shot", 1)] {
        let from_parse = loaded.workflow(&ext(id, version)).unwrap();
        let from_static = registry().workflow(&ext(id, version)).unwrap();
        assert_eq!(from_parse, from_static);
    }
    let poll_parse = loaded.widget_type(&ext("poll", 1), "poll/single").unwrap();
    let poll_static = registry().widget_type(&ext("poll", 1), "poll/single").unwrap();
    assert_eq!(poll_parse, poll_static);
}

#[test]
fn shipped_manifests_say_what_they_allow() {
    let scheduling = registry().workflow(&ext("scheduling", 1)).unwrap();
    assert!(scheduling.allows(ResourceKind::ServiceInSlot));
    assert!(!scheduling.allows(ResourceKind::GoodsOrService));
    assert!(scheduling.has_reserve_hook());
    assert!(scheduling.has_prepare_hook());
    let one_shot = registry().workflow(&ext("one-shot", 1)).unwrap();
    assert!(one_shot.allows(ResourceKind::GoodsOrService));
    assert!(!one_shot.has_reserve_hook());
    assert!(!one_shot.has_prepare_hook());
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
    let cases = [
        ("form@1", "must start with 'demo/'"),
        ("other/form@1", "must start with 'demo/'"),
        ("demo/form", "must end with @<version>"),
        ("demo/form@0", "must end with @<version>"),
        ("demo/form@x", "must end with @<version>"),
    ];
    for (name, rule) in cases {
        let mut m = base();
        m["details"][0]["schema"] = json!(name);
        let err = refused(&m);
        assert!(err.contains(rule), "schema {name:?}: {err}");
    }
}

#[test]
fn step_types_need_the_id_prefix_and_a_limit() {
    let cases = [
        ("demo", "done".to_string(), "must start with 'demo/'"),
        ("demo", "other/done".to_string(), "must start with 'demo/'"),
        ("demo", format!("demo/{}", "a".repeat(33)), "must be [a-z0-9-]"),
        ("demo", format!("demo/{}", "a".repeat(60)), "too long"),
        // 32 + 1 + 32 bytes: each part is allowed, only the whole is too long.
        ("a".repeat(32).leak(), format!("{}/{}", "a".repeat(32), "b".repeat(32)), "too long"),
    ];
    for (id, step, rule) in cases {
        let mut m = base();
        m["id"] = json!(id);
        m["details"][0]["schema"] = json!(format!("{id}/form@1"));
        m["steps"][0]["type"] = json!(step);
        let err = refused(&m);
        assert!(err.contains(rule), "step {step:?}: {err}");
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
        json!(["prepare", "reserve", "confirm"]),
    ] {
        let mut m = base();
        m["hooks"] = hooks;
        assert!(refused(&m).contains("hooks"));
    }
    let mut m = base();
    m["hooks"] = json!(["reserve", "confirm", "release"]);
    m["service"] = json!("scheduling");
    parse_one(&m).unwrap();
}

#[test]
fn prepare_is_only_allowed_with_the_three_hooks() {
    let mut m = base();
    m["service"] = json!("scheduling");
    m["hooks"] = json!(["prepare"]);
    assert!(refused(&m).contains("prepare needs"));
    m["hooks"] = json!(["prepare", "reserve", "confirm", "release"]);
    let loaded = parse_one(&m).unwrap();
    assert!(loaded.manifests()[0].has_prepare_hook());
    m["hooks"] = json!(["reserve", "confirm", "release"]);
    assert!(!parse_one(&m).unwrap().manifests()[0].has_prepare_hook());
}

#[test]
fn hooks_need_a_service_and_a_service_needs_hooks() {
    let mut m = base();
    m["hooks"] = json!(["reserve", "confirm", "release"]);
    assert!(refused(&m).contains("need a service"));
    m["service"] = json!("scheduling");
    parse_one(&m).unwrap();
    let mut m = base();
    m["service"] = json!("scheduling");
    assert!(refused(&m).contains("only for an extension with hooks"));
}

#[test]
fn the_service_of_hooks_is_a_roym_service_that_is_not_web() {
    for name in ["no-such-service", "web", ""] {
        let mut m = base();
        m["hooks"] = json!(["reserve", "confirm", "release"]);
        m["service"] = json!(name);
        assert!(refused(&m).contains("not a Roym service"), "{name}");
    }
}

#[test]
fn the_hooks_of_scheduling_go_to_the_scheduling_service() {
    let scheduling = registry().workflow(&ext("scheduling", 1)).unwrap();
    assert_eq!(scheduling.hook_service(), Some(services::SCHEDULING));
    let one_shot = registry().workflow(&ext("one-shot", 1)).unwrap();
    assert_eq!(one_shot.hook_service(), None);
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
    m["extra_key"] = json!([]);
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

#[test]
fn two_versions_of_one_schema_may_share_a_field_name() {
    let mut m = base();
    m["details"] = json!([
        { "schema": "demo/form@1", "fields": [{ "name": "note", "type": "text", "max_len": 100 }] },
        { "schema": "demo/form@2", "fields": [{ "name": "note", "type": "text", "max_len": 200 }] }
    ]);
    parse_one(&m).unwrap();
}

#[test]
fn two_versions_of_one_extension_may_share_ids() {
    let first = base();
    let mut second = base();
    second["version"] = json!(2);
    let loaded = Registry::parse(&[&first.to_string(), &second.to_string()]).unwrap();
    assert_eq!(loaded.manifests().len(), 2);
}

#[test]
fn a_header_check_ignores_an_insight() {
    let sources = [insight().to_string(), base().to_string()];
    let sources: Vec<&str> = sources.iter().map(String::as_str).collect();
    assert!(super::known_workflow_in(&sources, &ext("demo", 1)));
    assert!(!super::known_workflow_in(&sources, &ext("digest", 1)));
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

#[test]
fn the_header_check_agrees_with_the_registry() {
    let names = ["scheduling", "one-shot", "future", "digest", ""];
    for id in names {
        for version in 0..4 {
            let ext = ext(id, version);
            assert_eq!(
                super::is_known_workflow(&ext),
                registry().workflow(&ext).is_some(),
                "{id} v{version}"
            );
        }
    }
    assert!(super::is_known_workflow(&ext("scheduling", 1)));
    assert!(!super::is_known_workflow(&ext("scheduling", 2)));
}

fn chat_manifest() -> Value {
    json!({
        "id": "poll",
        "version": 1,
        "kind": "chat",
        "widgets": [
            {
                "type": "poll/single",
                "semantic": "poll",
                "fields": [
                    { "name": "option", "type": "choice", "required": true, "min_choices": 2, "max_choices": 12 }
                ]
            }
        ]
    })
}

#[test]
fn a_chat_manifest_loads_and_its_widget_type_is_found() {
    let loaded = parse_one(&chat_manifest()).unwrap();
    let reference = ext("poll", 1);
    let manifest = loaded.get(&reference).unwrap();
    assert_eq!(manifest.kind, ManifestKind::Chat);
    assert!(loaded.workflow(&reference).is_none());
    let widget = loaded.widget_type(&reference, "poll/single").unwrap();
    assert_eq!(widget.semantic, "poll");
    assert_eq!(widget.fields.len(), 1);
    assert_eq!(widget.fields[0].name, "option");
    assert_eq!(widget.fields[0].field_type, FieldType::Choice);
    assert!(widget.fields[0].required);
    assert_eq!(widget.fields[0].min_choices, Some(2));
    assert_eq!(widget.fields[0].max_choices, Some(12));
}

#[test]
fn a_chat_manifest_with_a_deal_key_is_refused() {
    for (key, val) in [
        ("resource_kinds", json!(["goods-or-service"])),
        ("hooks", json!(["reserve", "confirm", "release"])),
        ("details", json!([{ "schema": "poll/slot@1", "fields": [] }])),
        ("steps", json!([{ "type": "poll/started", "by": "giver", "fields": [] }])),
        ("reads", json!(["event-log"])),
        ("service", json!("poll")),
    ] {
        let mut m = chat_manifest();
        m[key] = val;
        assert!(refused(&m).contains("chat manifest has only widgets"), "key {key}");
    }
    let mut empty_widgets = chat_manifest();
    empty_widgets["widgets"] = json!([]);
    assert!(refused(&empty_widgets).contains("at least one widget"));
}

#[test]
fn a_workflow_or_insight_manifest_with_widgets_is_refused() {
    let mut workflow = base();
    workflow["widgets"] = json!([{
        "type": "demo/widget",
        "semantic": "poll",
        "fields": [{ "name": "opt", "type": "choice", "min_choices": 2, "max_choices": 4 }]
    }]);
    assert!(refused(&workflow).contains("only a chat manifest has widgets"));

    let mut insight_manifest = insight();
    insight_manifest["widgets"] = json!([{
        "type": "digest/widget",
        "semantic": "poll",
        "fields": [{ "name": "opt", "type": "choice", "min_choices": 2, "max_choices": 4 }]
    }]);
    assert!(refused(&insight_manifest).contains("insight has only reads"));
}

#[test]
fn a_widget_type_must_start_with_its_extension_id() {
    let mut m = chat_manifest();
    m["widgets"][0]["type"] = json!("other/single");
    assert!(refused(&m).contains("must start with 'poll/'"));

    let mut m2 = chat_manifest();
    m2["widgets"][0]["type"] = json!("poll/BAD_NAME");
    assert!(refused(&m2).contains("must be [a-z0-9-]"));
}

#[test]
fn a_widget_choice_rule_needs_min_and_max_choices() {
    let mut m = chat_manifest();
    m["widgets"][0]["fields"][0]["min_choices"] = json!(null);
    assert!(refused(&m).contains("needs min_choices and max_choices"));

    let mut m = chat_manifest();
    m["widgets"][0]["fields"][0]["min_choices"] = json!(1);
    assert!(refused(&m).contains("min_choices must be at least 2"));

    let mut m = chat_manifest();
    m["widgets"][0]["fields"][0]["min_choices"] = json!(10);
    m["widgets"][0]["fields"][0]["max_choices"] = json!(5);
    assert!(refused(&m).contains("min_choices is above max_choices"));

    let mut m = chat_manifest();
    m["widgets"][0]["fields"][0]["max_choices"] = json!(33);
    assert!(refused(&m).contains("max_choices must be at most 32"));

    let mut text_field = chat_manifest();
    text_field["widgets"][0]["fields"] = json!([{
        "name": "note",
        "type": "text",
        "max_len": 50,
        "min_choices": 2
    }]);
    assert!(refused(&text_field).contains("only a choice field takes min_choices or max_choices"));
}

#[test]
fn a_widget_field_of_type_money_is_refused() {
    let mut m = chat_manifest();
    m["widgets"][0]["fields"] = json!([{
        "name": "price",
        "type": "money"
    }]);
    assert!(refused(&m).contains("cannot use money"));
}

#[test]
fn a_listing_cannot_name_a_chat_extension() {
    let reference = ext("poll", 1);
    assert!(!super::is_known_workflow(&reference));
    assert!(registry().workflow(&reference).is_none());
}

#[test]
fn widget_types_agree_with_the_registry() {
    let reference = ext("poll", 1);
    let from_reader = super::widget_types().get(&reference, "poll/single").unwrap();
    let from_registry = registry().widget_type(&reference, "poll/single").unwrap();
    assert_eq!(from_reader, from_registry);
    assert!(super::widget_types().get(&reference, "poll/nonexistent").is_none());
    assert!(super::widget_types().get(&ext("scheduling", 1), "scheduling/slot").is_none());
}

#[test]
fn widget_ids_are_derived_and_a_clash_is_refused() {
    let m = Registry::parse(MANIFESTS).unwrap();
    let sources = ids::manifest_message_sources(m.get(&ext("poll", 1)).unwrap());
    assert_eq!(
        sources,
        vec![
            ("ext-poll-widget-single".to_string(), "poll/widget/single".to_string()),
            ("ext-poll-widget-single-option".to_string(), "poll/widget/single/option".to_string()),
        ]
    );

    let mut clashing = chat_manifest();
    clashing["widgets"].as_array_mut().unwrap().push(json!({
        "type": "poll/single-option",
        "semantic": "poll",
        "fields": [{ "name": "val", "type": "text", "max_len": 50 }]
    }));
    let err = Registry::parse(&[&clashing.to_string()]).unwrap_err();
    assert!(err.to_string().contains("is made twice"));
}
