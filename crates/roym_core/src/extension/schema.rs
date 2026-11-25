//! Typed field lists and the check of a JSON object against one.
//!
//! The same list is used for a commitment's `details.data` and for a step's
//! `data`. Every number is an integer, as the deal records have no floats.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::listing;

#[cfg(test)]
mod tests;

pub const MAX_FIELD_NAME_LEN: usize = 32;
pub const MAX_TEXT_LEN: usize = 512;
pub const MAX_LONG_TEXT_LEN: usize = 2048;
pub const MAX_CODE_LEN: usize = 128;
pub const MAX_CHOICE_LEN: usize = 32;

/// The value type of a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FieldType {
    /// Free text in the record's language.
    Text,
    /// Free text with line breaks.
    LongText,
    /// An identifier with no language, such as a listing id.
    Code,
    Number,
    /// Integer minor units of the deal's currency, at least 0.
    Money,
    /// Integer seconds since 1970.
    DateTime,
    /// Integer seconds, at least 0.
    Duration,
    Choice,
}

impl FieldType {
    /// The largest `max_len` a manifest may give this type, if it takes one.
    pub(super) fn len_ceiling(self) -> Option<usize> {
        match self {
            Self::Text => Some(MAX_TEXT_LEN),
            Self::LongText => Some(MAX_LONG_TEXT_LEN),
            Self::Code => Some(MAX_CODE_LEN),
            _ => None,
        }
    }
}

/// One named, typed field.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,
    #[serde(default)]
    pub required: bool,
    /// Bytes. Required for `text`, `long-text` and `code`.
    #[serde(default)]
    pub max_len: Option<usize>,
    /// `number` only.
    #[serde(default)]
    pub min: Option<i64>,
    /// `number` only.
    #[serde(default)]
    pub max: Option<i64>,
    /// `choice` only: the allowed codes.
    #[serde(default)]
    pub choices: Option<Vec<String>>,
}

/// A check that a field list cannot say. Each one is built into this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Check {
    /// `slot_id` is derived from `listing_id` and the commitment's window.
    SlotMatchesWindow,
}

/// A named, versioned field list for a commitment's `details.data`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schema {
    /// `<extension id>/<name>@<version>`.
    pub schema: String,
    pub fields: Vec<Field>,
    #[serde(default)]
    pub checks: Vec<Check>,
}

impl Schema {
    /// Checks `data` against the field list. A `money` value is minor units
    /// of the deal's currency, so the check needs no currency.
    pub fn check(&self, data: &Value) -> Result<(), SchemaError> {
        check_fields(&self.fields, data)
    }
}

/// Why a value does not fit its field list.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SchemaError(pub String);

/// Checks that `data` is an object with exactly the listed fields.
pub fn check_fields(fields: &[Field], data: &Value) -> Result<(), SchemaError> {
    let obj = data.as_object().ok_or_else(|| SchemaError("data must be an object".to_string()))?;
    if let Some(key) = obj.keys().find(|k| !fields.iter().any(|f| &f.name == *k)) {
        return Err(SchemaError(format!("unknown field '{key}'")));
    }
    for field in fields {
        match obj.get(&field.name) {
            Some(value) => check_value(field, value)?,
            None if field.required => {
                return Err(SchemaError(format!("field '{}' is required", field.name)));
            }
            None => {}
        }
    }
    Ok(())
}

fn check_value(field: &Field, value: &Value) -> Result<(), SchemaError> {
    let name = &field.name;
    match field.field_type {
        FieldType::Text | FieldType::LongText => check_string(field, value, false),
        FieldType::Code => check_string(field, value, true),
        FieldType::Number => {
            let n = value.as_i64().ok_or_else(|| wrong(name, "an integer"))?;
            if field.min.is_some_and(|min| n < min) || field.max.is_some_and(|max| n > max) {
                return Err(SchemaError(format!("field '{name}' is out of range")));
            }
            Ok(())
        }
        FieldType::Money | FieldType::Duration => value
            .as_u64()
            .filter(|n| i64::try_from(*n).is_ok())
            .map(|_| ())
            .ok_or_else(|| wrong(name, "an integer that is not negative")),
        FieldType::DateTime => {
            value.as_u64().map(|_| ()).ok_or_else(|| wrong(name, "an integer number of seconds"))
        }
        FieldType::Choice => {
            let s = value.as_str().ok_or_else(|| wrong(name, "a string"))?;
            let allowed = field.choices.as_deref().unwrap_or_default();
            if allowed.iter().any(|c| c == s) {
                Ok(())
            } else {
                Err(SchemaError(format!("field '{name}' is not one of its choices")))
            }
        }
    }
}

fn check_string(field: &Field, value: &Value, code: bool) -> Result<(), SchemaError> {
    let name = &field.name;
    let s = value.as_str().ok_or_else(|| wrong(name, "a string"))?;
    if field.max_len.is_some_and(|max| s.len() > max) {
        return Err(SchemaError(format!("field '{name}' is too long")));
    }
    if code {
        if s.is_empty() || !s.chars().all(is_code_char) {
            return Err(SchemaError(format!("field '{name}' is not a code")));
        }
    } else if field.required && s.trim().is_empty() {
        return Err(SchemaError(format!("field '{name}' must not be empty")));
    }
    Ok(())
}

fn is_code_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-')
}

fn wrong(name: &str, expected: &str) -> SchemaError {
    SchemaError(format!("field '{name}' must be {expected}"))
}

/// The load-time rules for one field definition.
pub(crate) fn check_definition(field: &Field) -> Result<(), String> {
    check_field_limits(field)?;
    check_choices(field)
}

pub(crate) fn check_field_limits(field: &Field) -> Result<(), String> {
    let name = &field.name;
    let name_ok = !name.is_empty()
        && name.len() <= MAX_FIELD_NAME_LEN
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !name_ok {
        return Err(format!("field name '{name}' is not [a-z0-9_]{{1,{MAX_FIELD_NAME_LEN}}}"));
    }
    check_len_limit(field)?;
    check_number_limits(field)
}

fn check_len_limit(field: &Field) -> Result<(), String> {
    let name = &field.name;
    match (field.field_type.len_ceiling(), field.max_len) {
        (Some(ceiling), Some(max)) if (1..=ceiling).contains(&max) => Ok(()),
        (Some(ceiling), Some(_)) => Err(format!("field '{name}': max_len must be 1 to {ceiling}")),
        (Some(_), None) => Err(format!("field '{name}': max_len is required")),
        (None, Some(_)) => Err(format!("field '{name}': this type takes no max_len")),
        (None, None) => Ok(()),
    }
}

fn check_number_limits(field: &Field) -> Result<(), String> {
    let name = &field.name;
    if field.field_type != FieldType::Number {
        if field.min.is_some() || field.max.is_some() {
            return Err(format!("field '{name}': only a number takes min or max"));
        }
        return Ok(());
    }
    match (field.min, field.max) {
        (Some(min), Some(max)) if min > max => Err(format!("field '{name}': min is above max")),
        _ => Ok(()),
    }
}

fn check_choices(field: &Field) -> Result<(), String> {
    let name = &field.name;
    match (field.field_type, &field.choices) {
        (FieldType::Choice, Some(choices)) if !choices.is_empty() => {
            let mut seen = Vec::with_capacity(choices.len());
            for choice in choices {
                if !listing::valid_token(choice, MAX_CHOICE_LEN) {
                    return Err(format!("field '{name}': choice '{choice}' is not a code"));
                }
                if seen.contains(&choice) {
                    return Err(format!("field '{name}': choice '{choice}' is repeated"));
                }
                seen.push(choice);
            }
            Ok(())
        }
        (FieldType::Choice, _) => Err(format!("field '{name}': a choice needs a choices list")),
        (_, Some(_)) => Err(format!("field '{name}': only a choice takes choices")),
        (_, None) => Ok(()),
    }
}
