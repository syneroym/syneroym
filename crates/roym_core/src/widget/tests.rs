use std::iter;

use serde_json::json;

use super::*;
use crate::extension::widget_types;

fn poll() -> WidgetDefinition {
    poll_definition("Which day?", &["Saturday".to_string(), "Sunday".to_string()])
}

#[test]
fn a_poll_definition_passes_every_check() {
    let p = poll();
    check_shape(&p).unwrap();
    assert_eq!(check_type(widget_types(), &p).unwrap(), WidgetCheck::Checked);
    let serialized = widget_body(&p).unwrap();
    let parsed = parse_widget_body(serialized.as_bytes()).unwrap();
    assert_eq!(p, parsed);
}

#[test]
fn check_shape_rule_1_extension_id_and_version() {
    let mut p = poll();
    p.extension.id = "bad ID".to_string();
    let err = check_shape(&p).unwrap_err();
    assert_eq!(err.code(), "widget-shape");

    let mut p2 = poll();
    p2.extension.version = 0;
    let err2 = check_shape(&p2).unwrap_err();
    assert_eq!(err2.code(), "widget-shape");
}

#[test]
fn check_shape_rule_2_widget_type() {
    let mut p = poll();
    p.widget_type = "other/single".to_string();
    let err = check_shape(&p).unwrap_err();
    assert_eq!(err.code(), "widget-shape");

    let mut p2 = poll();
    p2.widget_type = format!("poll/{}", "a".repeat(MAX_WIDGET_TYPE_LEN));
    let err2 = check_shape(&p2).unwrap_err();
    assert_eq!(err2.code(), "widget-shape");

    let mut p3 = poll();
    p3.widget_type = "poll/BAD_TOKEN".to_string();
    let err3 = check_shape(&p3).unwrap_err();
    assert_eq!(err3.code(), "widget-shape");
}

#[test]
fn check_shape_rule_3_semantic() {
    let mut p = poll();
    p.semantic = "bad semantic!".to_string();
    let err = check_shape(&p).unwrap_err();
    assert_eq!(err.code(), "widget-shape");

    let mut p2 = poll();
    p2.semantic = "a".repeat(MAX_SEMANTIC_LEN + 1);
    let err2 = check_shape(&p2).unwrap_err();
    assert_eq!(err2.code(), "widget-shape");
}

#[test]
fn check_shape_rule_4_prompt() {
    let mut p = poll();
    p.prompt = "   ".to_string();
    let err = check_shape(&p).unwrap_err();
    assert_eq!(err.code(), "widget-shape");

    let mut p2 = poll();
    p2.prompt = "x".repeat(MAX_PROMPT_LEN + 1);
    let err2 = check_shape(&p2).unwrap_err();
    assert_eq!(err2.code(), "widget-shape");
}

#[test]
fn check_shape_rule_5_fields() {
    let mut p = poll();
    p.fields.clear();
    assert_eq!(check_shape(&p).unwrap_err().code(), "widget-shape");

    let mut p2 = poll();
    for i in 0..MAX_WIDGET_FIELDS {
        p2.fields.push(DefinedField {
            name: format!("f{i}"),
            field_type: FieldType::Text,
            required: false,
            max_len: Some(10),
            min: None,
            max: None,
            label: None,
            choices: None,
        });
    }
    assert_eq!(check_shape(&p2).unwrap_err().code(), "widget-shape");

    let mut p3 = poll();
    p3.fields.push(p3.fields[0].clone());
    assert_eq!(check_shape(&p3).unwrap_err().code(), "widget-shape");

    let mut p4 = poll();
    p4.fields[0].field_type = FieldType::Money;
    assert_eq!(check_shape(&p4).unwrap_err().code(), "widget-shape");

    let mut p5 = poll();
    p5.fields[0].label = Some("x".repeat(MAX_WIDGET_LABEL_LEN + 1));
    assert_eq!(check_shape(&p5).unwrap_err().code(), "widget-shape");

    let mut p6 = poll();
    p6.fields[0].choices = Some(vec![]);
    assert_eq!(check_shape(&p6).unwrap_err().code(), "widget-shape");

    let mut p7 = poll();
    p7.fields[0].choices = Some(
        (0..=MAX_WIDGET_CHOICES)
            .map(|i| DefinedChoice { code: format!("c{i}"), text: "opt".to_string() })
            .collect(),
    );
    assert_eq!(check_shape(&p7).unwrap_err().code(), "widget-shape");

    let mut p8 = poll();
    p8.fields[0].choices.as_mut().unwrap()[0].text = "  ".to_string();
    assert_eq!(check_shape(&p8).unwrap_err().code(), "widget-shape");
}

#[test]
fn a_body_of_exactly_the_limit_passes_and_one_byte_more_is_refused() {
    let mut p = poll();
    let base_json = serde_json::to_string(&WidgetBody {
        widget_version: WIDGET_BODY_VERSION,
        widget: p.clone(),
    })
    .unwrap();
    let overhead = base_json.len() - p.prompt.len();
    let max_prompt_fit = MAX_WIDGET_BODY_BYTES - overhead;
    let actual_prompt_len = max_prompt_fit.min(MAX_PROMPT_LEN);
    p.prompt = "a".repeat(actual_prompt_len);

    let json_bytes =
        serde_json::to_vec(&WidgetBody { widget_version: WIDGET_BODY_VERSION, widget: p.clone() })
            .unwrap();

    let mut exact_8192 = json_bytes;
    if exact_8192.len() < MAX_WIDGET_BODY_BYTES {
        let pad = MAX_WIDGET_BODY_BYTES - exact_8192.len();
        exact_8192.extend(iter::repeat_n(b' ', pad));
    }
    assert_eq!(exact_8192.len(), 8 * 1024);
    assert!(parse_widget_body(&exact_8192).is_ok());

    let mut over_8192 = exact_8192;
    over_8192.push(b' ');
    assert_eq!(over_8192.len(), 8 * 1024 + 1);
    let err = parse_widget_body(&over_8192).unwrap_err();
    assert_eq!(err.code(), "widget-too-large");
}

#[test]
fn an_unknown_body_version_is_refused() {
    let bad = json!({
        "widget_version": 99,
        "widget": poll()
    });
    let bytes = serde_json::to_vec(&bad).unwrap();
    let err = parse_widget_body(&bytes).unwrap_err();
    assert_eq!(err.code(), "widget-version");
}

#[test]
fn a_body_with_unknown_version_and_extra_keys_returns_widget_version() {
    let bad = json!({
        "widget_version": 2,
        "extra_key": "some value",
        "widget": poll()
    });
    let bytes = serde_json::to_vec(&bad).unwrap();
    let err = parse_widget_body(&bytes).unwrap_err();
    assert_eq!(err.code(), "widget-version");
}

#[test]
fn a_body_with_unknown_field_type_returns_widget_shape() {
    let bad = json!({
        "widget_version": 1,
        "widget": {
            "extension": { "id": "poll", "version": 1 },
            "type": "poll/single",
            "semantic": "poll",
            "prompt": "Where?",
            "fields": [
                { "name": "place", "type": "location" }
            ]
        }
    });
    let bytes = serde_json::to_vec(&bad).unwrap();
    let err = parse_widget_body(&bytes).unwrap_err();
    assert_eq!(err.code(), "widget-shape");
}

#[test]
fn a_poll_with_one_or_thirteen_options_breaks_its_type_rules() {
    let one = poll_definition("Which day?", &["Saturday".to_string()]);
    let err = check_type(widget_types(), &one).unwrap_err();
    assert_eq!(err.code(), "widget-rules");

    let thirteen_opts: Vec<String> = (1..=13).map(|i| format!("Day {i}")).collect();
    let thirteen = poll_definition("Which day?", &thirteen_opts);
    let err = check_type(widget_types(), &thirteen).unwrap_err();
    assert_eq!(err.code(), "widget-rules");
}

#[test]
fn a_known_type_with_another_semantic_or_field_breaks_its_rules() {
    let mut p = poll();
    p.semantic = "form".to_string();
    let err = check_type(widget_types(), &p).unwrap_err();
    assert_eq!(err.code(), "widget-rules");

    let mut p2 = poll();
    p2.fields[0].name = "choice".to_string();
    let err2 = check_type(widget_types(), &p2).unwrap_err();
    assert_eq!(err2.code(), "widget-rules");

    let mut p3 = poll();
    p3.fields[0].label = Some("Your vote".to_string());
    let err3 = check_type(widget_types(), &p3).unwrap_err();
    assert_eq!(err3.code(), "widget-rules");
}

#[test]
fn an_unknown_type_is_not_checked_but_its_shape_is() {
    let custom = WidgetDefinition {
        extension: ExtensionRef { id: "survey".to_string(), version: 1 },
        widget_type: "survey/form".to_string(),
        semantic: "form".to_string(),
        prompt: "Please answer".to_string(),
        fields: vec![DefinedField {
            name: "comment".to_string(),
            field_type: FieldType::Text,
            required: true,
            max_len: Some(100),
            min: None,
            max: None,
            label: Some("Comments".to_string()),
            choices: None,
        }],
    };
    check_shape(&custom).unwrap();
    assert_eq!(check_type(widget_types(), &custom).unwrap(), WidgetCheck::NotChecked);
}

#[test]
fn an_answer_with_an_unknown_choice_an_extra_key_or_no_required_field_is_refused() {
    let p = poll();
    let good_answer = json!({ "option": "o1" });
    check_answer(&p, &good_answer).unwrap();

    let unknown_choice = json!({ "option": "o99" });
    assert_eq!(check_answer(&p, &unknown_choice).unwrap_err().code(), "answer-invalid");

    let extra_key = json!({ "option": "o1", "extra": "data" });
    assert_eq!(check_answer(&p, &extra_key).unwrap_err().code(), "answer-invalid");

    let empty = json!({});
    assert_eq!(check_answer(&p, &empty).unwrap_err().code(), "answer-invalid");
}

#[test]
fn an_answer_to_an_unknown_type_is_checked_against_its_own_fields() {
    let custom = WidgetDefinition {
        extension: ExtensionRef { id: "feedback".to_string(), version: 1 },
        widget_type: "feedback/short".to_string(),
        semantic: "form".to_string(),
        prompt: "Feedback".to_string(),
        fields: vec![DefinedField {
            name: "rating".to_string(),
            field_type: FieldType::Number,
            required: true,
            max_len: None,
            min: Some(1),
            max: Some(5),
            label: Some("Rating".to_string()),
            choices: None,
        }],
    };
    let valid = json!({ "rating": 5 });
    check_answer(&custom, &valid).unwrap();

    let invalid = json!({ "rating": 6 });
    assert_eq!(check_answer(&custom, &invalid).unwrap_err().code(), "answer-invalid");
}

#[test]
fn poll_definition_has_the_hub_shape() {
    let p = poll_definition(
        "Which day shall we meet?",
        &["Saturday".to_string(), "Sunday".to_string()],
    );
    let value = serde_json::to_value(&p).unwrap();
    assert_eq!(value["extension"]["id"], "poll");
    assert_eq!(value["extension"]["version"], 1);
    assert_eq!(value["type"], "poll/single");
    assert_eq!(value["semantic"], "poll");
    assert_eq!(value["prompt"], "Which day shall we meet?");
    assert_eq!(value["fields"][0]["name"], "option");
    assert_eq!(value["fields"][0]["type"], "choice");
    assert_eq!(value["fields"][0]["required"], true);
    assert_eq!(value["fields"][0]["choices"][0]["code"], "o1");
    assert_eq!(value["fields"][0]["choices"][0]["text"], "Saturday");
    assert_eq!(value["fields"][0]["choices"][1]["code"], "o2");
    assert_eq!(value["fields"][0]["choices"][1]["text"], "Sunday");
}
