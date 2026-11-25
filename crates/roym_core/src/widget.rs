//! Chat widget and widget answer formats and checks.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    deal::{ExtensionRef, MAX_EXTENSION_ID_LEN},
    extension::{FieldType, WidgetTypes, is_extension_id, schema},
    listing,
};

#[cfg(test)]
mod tests;

pub const WIDGET_CONTENT_TYPE: &str = "application/vnd.roym.widget+json";
pub const WIDGET_ANSWER_CONTENT_TYPE: &str = "application/vnd.roym.widget-answer+json";
pub const WIDGET_BODY_VERSION: u32 = 1;
pub const ANSWER_BODY_VERSION: u32 = 1;

/// The whole message body, so the limit is what travels.
pub const MAX_WIDGET_BODY_BYTES: usize = 8 * 1024;
pub const MAX_ANSWER_BODY_BYTES: usize = 8 * 1024;
pub const MAX_WIDGET_FIELDS: usize = 16;
pub const MAX_WIDGET_CHOICES: usize = 32;
pub const MAX_WIDGET_TYPE_LEN: usize = 64;
pub const MAX_SEMANTIC_LEN: usize = 32;
pub const MAX_PROMPT_LEN: usize = schema::MAX_TEXT_LEN;
pub const MAX_WIDGET_LABEL_LEN: usize = 128;
pub const MAX_CHOICE_TEXT_LEN: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WidgetError {
    #[error("widget body is too large: {bytes} bytes")]
    TooLarge { bytes: usize },
    #[error("unknown widget version: {0}")]
    UnknownVersion(u32),
    #[error("invalid widget json: {0}")]
    Json(String),
    #[error("invalid widget shape: {0}")]
    Shape(String),
    #[error("unknown extension: {0}")]
    TypeUnknown(String),
    #[error("widget type rules broken: {0}")]
    TypeRules(String),
    #[error("answer body is too large: {bytes} bytes")]
    AnswerTooLarge { bytes: usize },
    #[error("invalid answer data: {0}")]
    Answer(String),
    #[error("widget not found")]
    NotFound,
    #[error("target message is not a widget")]
    NotAWidget,
    #[error("widget was deleted")]
    Deleted,
    #[error("widget refused: {0}")]
    Refused(String),
}

impl WidgetError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::TooLarge { .. } => "widget-too-large",
            Self::UnknownVersion(_) => "widget-version",
            Self::Json(_) => "widget-json",
            Self::Shape(_) => "widget-shape",
            Self::TypeUnknown(_) => "extension-unknown",
            Self::TypeRules(_) => "widget-rules",
            Self::AnswerTooLarge { .. } => "answer-too-large",
            Self::Answer(_) => "answer-invalid",
            Self::NotFound => "widget-not-found",
            Self::NotAWidget => "not-a-widget",
            Self::Deleted => "widget-deleted",
            Self::Refused(_) => "widget-refused",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetCheck {
    Checked,
    NotChecked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetDefinition {
    pub extension: ExtensionRef,
    /// `<extension id>/<name>`.
    #[serde(rename = "type")]
    pub widget_type: String,
    /// A code: `poll`, `form`, `checklist`, ...
    pub semantic: String,
    /// The author's question, in the message's first language.
    pub prompt: String,
    /// What an answer holds.
    pub fields: Vec<DefinedField>,
}

impl WidgetDefinition {
    pub fn answer_fields(&self) -> Vec<schema::Field> {
        self.fields
            .iter()
            .map(|f| schema::Field {
                name: f.name.clone(),
                field_type: f.field_type,
                required: f.required,
                max_len: f.max_len,
                min: f.min,
                max: f.max,
                choices: f
                    .choices
                    .as_ref()
                    .map(|choices| choices.iter().map(|c| c.code.clone()).collect()),
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefinedField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_len: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
    /// The author's label. A known type's field is labelled from the
    /// catalog instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// `choice` only. Each choice is a code and the author's text for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<DefinedChoice>>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefinedChoice {
    pub code: String,
    pub text: String,
}

/// A widget message body: `{"widget_version": 1, "widget": {...}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetBody {
    pub widget_version: u32,
    pub widget: WidgetDefinition,
}

/// An answer message body:
/// `{"answer_version": 1, "widget_id": "<message id>", "data": {...}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerBody {
    pub answer_version: u32,
    pub widget_id: String,
    pub data: Value,
}

/// Size first, then parse to `serde_json::Value`, check `widget_version`
/// (returns `UnknownVersion` / "widget-version"), and deserialize the
/// definition (mapping any serde failure to `Shape` / "widget-shape").
pub fn parse_widget_body(body: &[u8]) -> Result<WidgetDefinition, WidgetError> {
    if body.len() > MAX_WIDGET_BODY_BYTES {
        return Err(WidgetError::TooLarge { bytes: body.len() });
    }
    let val: Value = serde_json::from_slice(body).map_err(|e| WidgetError::Json(e.to_string()))?;
    let version = val
        .get("widget_version")
        .and_then(Value::as_u64)
        .ok_or_else(|| WidgetError::Json("missing widget_version".to_string()))?;
    if version != u64::from(WIDGET_BODY_VERSION) {
        return Err(WidgetError::UnknownVersion(version as u32));
    }
    let parsed: WidgetBody =
        serde_json::from_value(val).map_err(|e| WidgetError::Shape(e.to_string()))?;
    check_shape(&parsed.widget)?;
    Ok(parsed.widget)
}

/// Shape, then size of the serialized body.
pub fn widget_body(def: &WidgetDefinition) -> Result<String, WidgetError> {
    check_shape(def)?;
    let body = WidgetBody { widget_version: WIDGET_BODY_VERSION, widget: def.clone() };
    let s = serde_json::to_string(&body).map_err(|e| WidgetError::Json(e.to_string()))?;
    if s.len() > MAX_WIDGET_BODY_BYTES {
        return Err(WidgetError::TooLarge { bytes: s.len() });
    }
    Ok(s)
}

/// The rules for every widget, known or not.
pub fn check_shape(def: &WidgetDefinition) -> Result<(), WidgetError> {
    if !is_extension_id(&def.extension.id) || def.extension.version < 1 {
        return Err(WidgetError::Shape("invalid extension".to_string()));
    }
    if def.widget_type.len() > MAX_WIDGET_TYPE_LEN {
        return Err(WidgetError::Shape("widget_type too long".to_string()));
    }
    let name = def.widget_type.strip_prefix(&def.extension.id).and_then(|r| r.strip_prefix('/'));
    match name {
        Some(n) if listing::valid_token(n, MAX_EXTENSION_ID_LEN) => {}
        _ => return Err(WidgetError::Shape("widget_type must be <extension>/<code>".to_string())),
    }
    if !listing::valid_token(&def.semantic, MAX_SEMANTIC_LEN) {
        return Err(WidgetError::Shape("semantic must be a valid code".to_string()));
    }
    if def.prompt.trim().is_empty() || def.prompt.len() > MAX_PROMPT_LEN {
        return Err(WidgetError::Shape(
            "prompt must not be blank and at most 512 bytes".to_string(),
        ));
    }
    if def.fields.is_empty() || def.fields.len() > MAX_WIDGET_FIELDS {
        return Err(WidgetError::Shape(format!("fields count must be 1 to {MAX_WIDGET_FIELDS}")));
    }
    let mut seen = HashSet::new();
    for f in &def.fields {
        if !seen.insert(f.name.as_str()) {
            return Err(WidgetError::Shape(format!("field '{}' is listed twice", f.name)));
        }
        if f.field_type == FieldType::Money {
            return Err(WidgetError::Shape(format!(
                "field '{}': chat widgets cannot use money",
                f.name
            )));
        }
        if let Some(label) = &f.label
            && (label.trim().is_empty() || label.len() > MAX_WIDGET_LABEL_LEN)
        {
            return Err(WidgetError::Shape(format!(
                "field '{}': label must not be blank and at most {MAX_WIDGET_LABEL_LEN} bytes",
                f.name
            )));
        }
        let choices = if f.field_type == FieldType::Choice {
            let list = f.choices.as_ref().ok_or_else(|| {
                WidgetError::Shape(format!("field '{}': choice field needs choices", f.name))
            })?;
            if list.is_empty() || list.len() > MAX_WIDGET_CHOICES {
                return Err(WidgetError::Shape(format!(
                    "field '{}': choices count must be 1 to {MAX_WIDGET_CHOICES}",
                    f.name
                )));
            }
            let mut codes = Vec::with_capacity(list.len());
            for c in list {
                if c.text.trim().is_empty() || c.text.len() > MAX_CHOICE_TEXT_LEN {
                    return Err(WidgetError::Shape(format!(
                        "field '{}': choice text must not be blank and at most \
                         {MAX_CHOICE_TEXT_LEN} bytes",
                        f.name
                    )));
                }
                codes.push(c.code.clone());
            }
            Some(codes)
        } else {
            if f.choices.is_some() {
                return Err(WidgetError::Shape(format!(
                    "field '{}': only choice fields have choices",
                    f.name
                )));
            }
            None
        };
        let dummy = schema::Field {
            name: f.name.clone(),
            field_type: f.field_type,
            required: f.required,
            max_len: f.max_len,
            min: f.min,
            max: f.max,
            choices,
        };
        schema::check_definition(&dummy).map_err(WidgetError::Shape)?;
    }
    Ok(())
}

/// The type's rules, when this build knows the type.
pub fn check_type(types: &WidgetTypes, def: &WidgetDefinition) -> Result<WidgetCheck, WidgetError> {
    let rule_type = match types.get(&def.extension, &def.widget_type) {
        Some(t) => t,
        None => return Ok(WidgetCheck::NotChecked),
    };
    if def.semantic != rule_type.semantic {
        return Err(WidgetError::TypeRules(format!(
            "semantic '{}' does not match rule '{}'",
            def.semantic, rule_type.semantic
        )));
    }
    if def.fields.len() != rule_type.fields.len() {
        return Err(WidgetError::TypeRules(format!(
            "field count {} does not match rule count {}",
            def.fields.len(),
            rule_type.fields.len()
        )));
    }
    for (def_field, rule_field) in def.fields.iter().zip(&rule_type.fields) {
        if def_field.name != rule_field.name {
            return Err(WidgetError::TypeRules(format!(
                "field name '{}' does not match rule '{}'",
                def_field.name, rule_field.name
            )));
        }
        if def_field.field_type != rule_field.field_type {
            return Err(WidgetError::TypeRules(format!(
                "field '{}' type does not match rule",
                def_field.name
            )));
        }
        if def_field.required != rule_field.required {
            return Err(WidgetError::TypeRules(format!(
                "field '{}' required flag does not match rule",
                def_field.name
            )));
        }
        if def_field.max_len != rule_field.max_len {
            return Err(WidgetError::TypeRules(format!(
                "field '{}' max_len does not match rule",
                def_field.name
            )));
        }
        if def_field.min != rule_field.min || def_field.max != rule_field.max {
            return Err(WidgetError::TypeRules(format!(
                "field '{}' number limits do not match rule",
                def_field.name
            )));
        }
        if def_field.label.is_some() {
            return Err(WidgetError::TypeRules(format!(
                "field '{}' of a known type must have no label",
                def_field.name
            )));
        }
        if def_field.field_type == FieldType::Choice {
            let count = def_field.choices.as_ref().map_or(0, Vec::len);
            let min = rule_field.min_choices.unwrap_or(0);
            let max = rule_field.max_choices.unwrap_or(MAX_WIDGET_CHOICES);
            if count < min || count > max {
                return Err(WidgetError::TypeRules(format!(
                    "field '{}' choices count {} is outside {}..={}",
                    def_field.name, count, min, max
                )));
            }
        }
    }
    Ok(WidgetCheck::Checked)
}

/// `data` against the definition's own fields.
pub fn check_answer(def: &WidgetDefinition, data: &Value) -> Result<(), WidgetError> {
    schema::check_fields(&def.answer_fields(), data).map_err(|e| WidgetError::Answer(e.to_string()))
}

pub fn parse_answer_body(body: &[u8]) -> Result<AnswerBody, WidgetError> {
    if body.len() > MAX_ANSWER_BODY_BYTES {
        return Err(WidgetError::AnswerTooLarge { bytes: body.len() });
    }
    let val: Value = serde_json::from_slice(body).map_err(|e| WidgetError::Json(e.to_string()))?;
    let version = val
        .get("answer_version")
        .and_then(Value::as_u64)
        .ok_or_else(|| WidgetError::Json("missing answer_version".to_string()))?;
    if version != u64::from(ANSWER_BODY_VERSION) {
        return Err(WidgetError::UnknownVersion(version as u32));
    }
    let answer: AnswerBody =
        serde_json::from_value(val).map_err(|e| WidgetError::Answer(e.to_string()))?;
    Ok(answer)
}

pub fn answer_body(widget_id: &str, data: &Value) -> Result<String, WidgetError> {
    let body = AnswerBody {
        answer_version: ANSWER_BODY_VERSION,
        widget_id: widget_id.to_string(),
        data: data.clone(),
    };
    let s = serde_json::to_string(&body).map_err(|e| WidgetError::Json(e.to_string()))?;
    if s.len() > MAX_ANSWER_BODY_BYTES {
        return Err(WidgetError::AnswerTooLarge { bytes: s.len() });
    }
    Ok(s)
}

/// The one poll type, with the codes `o1`, `o2`, ... in order.
pub fn poll_definition(prompt: &str, options: &[String]) -> WidgetDefinition {
    let choices = options
        .iter()
        .enumerate()
        .map(|(i, opt)| DefinedChoice { code: format!("o{}", i + 1), text: opt.clone() })
        .collect();
    WidgetDefinition {
        extension: ExtensionRef { id: "poll".to_string(), version: 1 },
        widget_type: "poll/single".to_string(),
        semantic: "poll".to_string(),
        prompt: prompt.to_string(),
        fields: vec![DefinedField {
            name: "option".to_string(),
            field_type: FieldType::Choice,
            required: true,
            max_len: None,
            min: None,
            max: None,
            label: None,
            choices: Some(choices),
        }],
    }
}
