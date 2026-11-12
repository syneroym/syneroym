use rusqlite::types::Value as SqlValue;
use serde_json::Value as Json;

use crate::host_store::DataLayerError;

/// The text index of one collection, as the filter compiler needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFields<'a> {
    pub collection: &'a str,
    pub fields: &'a [String],
}

pub const MAX_SEARCH_TERMS: usize = 16;
pub const MAX_SEARCH_BYTES: usize = 1024;

/// Compiles `{"<path>": {"$search": <value>}}` to a SQL condition on the
/// collection's rowid, and pushes its bound parameters.
pub(crate) fn compile_search(
    path: &str,
    value: &Json,
    text: Option<&TextFields<'_>>,
    params: &mut Vec<SqlValue>,
) -> Result<String, DataLayerError> {
    let Some(text) = text else {
        return Err(DataLayerError::SchemaViolation(
            "$search is not available in this filter".to_string(),
        ));
    };

    if !text.fields.iter().any(|f| f == path) {
        return Err(DataLayerError::SchemaViolation(format!(
            "'{path}' is not a text field of collection '{}'",
            text.collection
        )));
    }

    let Json::String(raw_text) = value else {
        return Err(DataLayerError::SchemaViolation(format!(
            "$search requires a string for field '{path}'"
        )));
    };

    if raw_text.contains('\0') {
        return Err(DataLayerError::SchemaViolation(
            "$search text must not contain NUL".to_string(),
        ));
    }

    if raw_text.len() > MAX_SEARCH_BYTES {
        return Err(DataLayerError::SchemaViolation(
            "$search text is longer than 1024 bytes".to_string(),
        ));
    }

    let terms = syneroym_text_lang::query_terms(raw_text);
    let total_terms = terms.long.len() + terms.short.len();
    if total_terms == 0 {
        return Err(DataLayerError::SchemaViolation("$search needs at least one word".to_string()));
    }
    if total_terms > MAX_SEARCH_TERMS {
        return Err(DataLayerError::SchemaViolation("$search has more than 16 words".to_string()));
    }

    let coll = text.collection;

    let tri_clause = if !terms.long.is_empty() {
        let expr = terms
            .long
            .iter()
            .map(|t| format!("f_{path} : \"{}\"", t.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ");
        params.push(SqlValue::Text(expr));
        Some(format!(
            "{coll}.rowid IN (SELECT rowid FROM _text_{coll}_tri WHERE _text_{coll}_tri MATCH ?)"
        ))
    } else {
        None
    };

    let word_clause = if !terms.short.is_empty() {
        let expr = terms
            .short
            .iter()
            .map(|t| format!("f_{path} : \"{}\"*", t.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ");
        params.push(SqlValue::Text(expr));
        Some(format!(
            "{coll}.rowid IN (SELECT rowid FROM _text_{coll}_word WHERE _text_{coll}_word MATCH ?)"
        ))
    } else {
        None
    };

    match (tri_clause, word_clause) {
        (Some(tri), Some(word)) => Ok(format!("({tri} AND {word})")),
        (Some(tri), None) => Ok(tri),
        (None, Some(word)) => Ok(word),
        (None, None) => unreachable!("checked total_terms > 0"),
    }
}
