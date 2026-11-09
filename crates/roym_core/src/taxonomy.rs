//! The base taxonomy that the Roym project keeps.
//!
//! A record may carry any well-formed code. This module checks the structure
//! and valid format of the project's base categories, units, payment methods,
//! and attributes.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use crate::listing::{self, MAX_CATEGORY_LEN, MAX_PAYMENT_METHOD_LEN, MAX_UNIT_LEN};

const MAX_ATTRIBUTE_LEN: usize = 64;
const MAX_CHOICE_LEN: usize = 64;

/// The base taxonomy that the Roym project keeps. A record may also
/// carry a code that is not here; `ListingPayload::validate` checks only
/// the shape of a code.
pub const BASE_JSON: &str = include_str!("../taxonomy/base.json");

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Taxonomy {
    pub categories: Vec<CategoryDef>,
    pub units: Vec<String>,
    pub payment_methods: Vec<String>,
    pub attributes: Vec<AttributeDef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryDef {
    pub code: String,
    #[serde(default)]
    pub parent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttributeDef {
    pub code: String,
    pub categories: Vec<String>,
    pub value: AttributeValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AttributeValue {
    Choice {
        choices: Vec<String>,
    },
    Number {
        #[serde(default)]
        unit: Option<String>,
    },
    YesNo,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TaxonomyError {
    #[error("the taxonomy file does not parse: {0}")]
    Parse(String),
    #[error("{kind} code '{code}' is not a code of [a-z0-9-] that fits its field")]
    CodeShape { kind: &'static str, code: String },
    #[error("{kind} code '{code}' appears twice")]
    Duplicate { kind: &'static str, code: String },
    #[error("category '{code}' names a parent '{parent}' that is not a group")]
    BadParent { code: String, parent: String },
    #[error("category '{code}' appears before its group '{parent}'")]
    ParentAfterChild { code: String, parent: String },
    #[error("group '{0}' has no children")]
    GroupWithoutChildren(String),
    #[error("attribute '{attribute}' names an unknown category '{category}'")]
    UnknownCategory { attribute: String, category: String },
    #[error("attribute '{attribute}' names an unknown unit '{unit}'")]
    UnknownUnit { attribute: String, unit: String },
    #[error("attribute '{0}' has no categories, or a choice attribute has no choices")]
    Empty(String),
}

impl Taxonomy {
    pub fn parse(json: &str) -> Result<Self, TaxonomyError> {
        let taxonomy: Self =
            serde_json::from_str(json).map_err(|err| TaxonomyError::Parse(err.to_string()))?;
        taxonomy.validate()?;
        Ok(taxonomy)
    }

    pub fn validate(&self) -> Result<(), TaxonomyError> {
        validate_categories(&self.categories)?;
        validate_units(&self.units)?;
        validate_payment_methods(&self.payment_methods)?;
        validate_attributes(&self.attributes, &self.categories, &self.units)?;
        Ok(())
    }
}

fn validate_categories(categories: &[CategoryDef]) -> Result<(), TaxonomyError> {
    let mut indices = HashMap::new();
    let mut child_counts = HashMap::new();

    for (idx, cat) in categories.iter().enumerate() {
        if !listing::valid_token(&cat.code, MAX_CATEGORY_LEN) {
            return Err(TaxonomyError::CodeShape { kind: "category", code: cat.code.clone() });
        }
        if indices.insert(cat.code.as_str(), (idx, cat.parent.as_deref())).is_some() {
            return Err(TaxonomyError::Duplicate { kind: "category", code: cat.code.clone() });
        }
        if cat.parent.is_none() {
            child_counts.insert(cat.code.as_str(), 0usize);
        }
    }

    for (idx, cat) in categories.iter().enumerate() {
        if let Some(ref parent) = cat.parent {
            let parent_info = indices.get(parent.as_str()).copied();
            match parent_info {
                Some((_, Some(_))) | None => {
                    return Err(TaxonomyError::BadParent {
                        code: cat.code.clone(),
                        parent: parent.clone(),
                    });
                }
                Some((parent_idx, None)) => {
                    if parent_idx >= idx {
                        return Err(TaxonomyError::ParentAfterChild {
                            code: cat.code.clone(),
                            parent: parent.clone(),
                        });
                    }
                    if let Some(count) = child_counts.get_mut(parent.as_str()) {
                        *count += 1;
                    }
                }
            }
        }
    }

    for (group_code, count) in child_counts {
        if count == 0 {
            return Err(TaxonomyError::GroupWithoutChildren(group_code.to_string()));
        }
    }

    Ok(())
}

fn check_codes(kind: &'static str, codes: &[String], max: usize) -> Result<(), TaxonomyError> {
    let mut seen = HashSet::new();
    for code in codes {
        if !listing::valid_token(code, max) {
            return Err(TaxonomyError::CodeShape { kind, code: code.clone() });
        }
        if !seen.insert(code.as_str()) {
            return Err(TaxonomyError::Duplicate { kind, code: code.clone() });
        }
    }
    Ok(())
}

fn validate_units(units: &[String]) -> Result<(), TaxonomyError> {
    check_codes("unit", units, MAX_UNIT_LEN)
}

fn validate_payment_methods(payment_methods: &[String]) -> Result<(), TaxonomyError> {
    check_codes("payment method", payment_methods, MAX_PAYMENT_METHOD_LEN)
}

fn validate_attributes(
    attributes: &[AttributeDef],
    categories: &[CategoryDef],
    units: &[String],
) -> Result<(), TaxonomyError> {
    let mut seen_attrs = HashSet::new();
    let category_set: HashSet<&str> = categories.iter().map(|c| c.code.as_str()).collect();
    let unit_set: HashSet<&str> = units.iter().map(String::as_str).collect();

    for attr in attributes {
        if !listing::valid_token(&attr.code, MAX_ATTRIBUTE_LEN) {
            return Err(TaxonomyError::CodeShape { kind: "attribute", code: attr.code.clone() });
        }
        if !seen_attrs.insert(attr.code.as_str()) {
            return Err(TaxonomyError::Duplicate { kind: "attribute", code: attr.code.clone() });
        }
        if attr.categories.is_empty() {
            return Err(TaxonomyError::Empty(attr.code.clone()));
        }
        for cat in &attr.categories {
            if !category_set.contains(cat.as_str()) {
                return Err(TaxonomyError::UnknownCategory {
                    attribute: attr.code.clone(),
                    category: cat.clone(),
                });
            }
        }
        validate_attribute_value(&attr.code, &attr.value, &unit_set)?;
    }
    Ok(())
}

fn validate_attribute_value(
    attr_code: &str,
    value: &AttributeValue,
    unit_set: &HashSet<&str>,
) -> Result<(), TaxonomyError> {
    match value {
        AttributeValue::Choice { choices } => {
            if choices.is_empty() {
                return Err(TaxonomyError::Empty(attr_code.to_string()));
            }
            check_codes("choice", choices, MAX_CHOICE_LEN)?;
        }
        AttributeValue::Number { unit } => {
            if let Some(u) = unit
                && !unit_set.contains(u.as_str())
            {
                return Err(TaxonomyError::UnknownUnit {
                    attribute: attr_code.to_string(),
                    unit: u.clone(),
                });
            }
        }
        AttributeValue::YesNo | AttributeValue::Text => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests;
