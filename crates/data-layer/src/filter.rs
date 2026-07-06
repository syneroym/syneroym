//! MongoDB-style JSON filter document -> parameterized SQLite WHERE-clause
//! compiler.
//!
//! Pure and DB-free: produces a WHERE-clause fragment plus a list of bound
//! parameters, in binding order. Guest-supplied values -- including JSON
//! field paths -- are always bound as `?` placeholders and passed as the
//! second argument to `json_extract`; nothing from the filter document is
//! ever interpolated into the SQL text itself.

use rusqlite::types::Value;
use serde_json::Value as Json;

use crate::host_store::DataLayerError;

const MAX_FILTER_DEPTH: u32 = 10;

/// A compiled MongoDB-style filter: a SQL boolean expression plus its bound
/// parameters, in binding order.
#[derive(Debug, Clone)]
pub struct CompiledFilter {
    pub where_clause: String,
    pub params: Vec<Value>,
}

/// Compiles an optional MongoDB-style JSON filter document into a
/// parameterized SQL WHERE-clause fragment. `None`, an empty string, or an
/// empty JSON object all compile to `Ok(None)` (no WHERE clause needed).
pub fn compile_filter(filter_json: Option<&str>) -> Result<Option<CompiledFilter>, DataLayerError> {
    let Some(raw) = filter_json else { return Ok(None) };
    if raw.trim().is_empty() {
        return Ok(None);
    }

    let parsed: Json = serde_json::from_str(raw)
        .map_err(|e| DataLayerError::SchemaViolation(format!("invalid filter JSON: {e}")))?;

    let Json::Object(doc) = parsed else {
        return Err(DataLayerError::SchemaViolation(
            "filter document must be a JSON object".to_string(),
        ));
    };

    if doc.is_empty() {
        return Ok(None);
    }

    let mut params = Vec::new();
    let where_clause = compile_document(&doc, 0, &mut params)?;
    Ok(Some(CompiledFilter { where_clause, params }))
}

fn compile_document(
    doc: &serde_json::Map<String, Json>,
    depth: u32,
    params: &mut Vec<Value>,
) -> Result<String, DataLayerError> {
    if depth > MAX_FILTER_DEPTH {
        return Err(DataLayerError::SchemaViolation(
            "query document too deeply nested".to_string(),
        ));
    }
    if doc.is_empty() {
        return Ok("1=1".to_string());
    }

    let mut clauses = Vec::with_capacity(doc.len());
    for (key, value) in doc {
        clauses.push(compile_entry(key, value, depth, params)?);
    }
    Ok(clauses.join(" AND "))
}

fn compile_entry(
    key: &str,
    value: &Json,
    depth: u32,
    params: &mut Vec<Value>,
) -> Result<String, DataLayerError> {
    match key {
        "$and" => compile_logical_array(value, depth, params, " AND "),
        "$or" => compile_logical_array(value, depth, params, " OR "),
        "$not" => {
            let Json::Object(sub) = value else {
                return Err(DataLayerError::SchemaViolation("$not requires an object".to_string()));
            };
            let inner = compile_document(sub, depth + 1, params)?;
            Ok(format!("NOT ({inner})"))
        }
        _ if key.starts_with('$') => {
            Err(DataLayerError::SchemaViolation(format!("unsupported operator: {key}")))
        }
        _ => compile_field(key, value, depth, params),
    }
}

fn compile_logical_array(
    value: &Json,
    depth: u32,
    params: &mut Vec<Value>,
    joiner: &str,
) -> Result<String, DataLayerError> {
    let Json::Array(items) = value else {
        return Err(DataLayerError::SchemaViolation(
            "$and/$or requires an array of filter documents".to_string(),
        ));
    };
    if items.is_empty() {
        return Err(DataLayerError::SchemaViolation(
            "$and/$or requires at least one sub-filter".to_string(),
        ));
    }
    let mut parts = Vec::with_capacity(items.len());
    for item in items {
        let Json::Object(sub) = item else {
            return Err(DataLayerError::SchemaViolation(
                "$and/$or array elements must be filter documents".to_string(),
            ));
        };
        parts.push(compile_document(sub, depth + 1, params)?);
    }
    Ok(format!("({})", parts.join(joiner)))
}

/// Compiles a single `"field": <value-or-operator-doc>` entry. `path` is the
/// dot-notation JSON path built up so far (without the leading `$.`).
fn compile_field(
    path: &str,
    value: &Json,
    depth: u32,
    params: &mut Vec<Value>,
) -> Result<String, DataLayerError> {
    if depth > MAX_FILTER_DEPTH {
        return Err(DataLayerError::SchemaViolation(
            "query document too deeply nested".to_string(),
        ));
    }
    match value {
        Json::Object(obj) if obj.keys().any(|k| k.starts_with('$')) => {
            compile_operator_map(path, obj, params)
        }
        Json::Object(obj) => {
            // Nested document without operator keys: recurse with
            // dot-notation, e.g. {"address": {"city": "London"}} behaves the
            // same as {"address.city": "London"}.
            let mut clauses = Vec::with_capacity(obj.len());
            for (sub_key, sub_value) in obj {
                let nested_path = format!("{path}.{sub_key}");
                clauses.push(compile_field(&nested_path, sub_value, depth + 1, params)?);
            }
            if clauses.is_empty() {
                Ok("1=1".to_string())
            } else {
                Ok(format!("({})", clauses.join(" AND ")))
            }
        }
        Json::Array(_) => Err(DataLayerError::SchemaViolation(format!(
            "unsupported filter value type for field '{path}'"
        ))),
        scalar => compile_equality(path, scalar, params),
    }
}

fn compile_operator_map(
    path: &str,
    obj: &serde_json::Map<String, Json>,
    params: &mut Vec<Value>,
) -> Result<String, DataLayerError> {
    let mut clauses = Vec::with_capacity(obj.len());
    for (op, opval) in obj {
        let sql_op = match op.as_str() {
            "$gt" => ">",
            "$gte" => ">=",
            "$lt" => "<",
            "$lte" => "<=",
            "$ne" => "!=",
            "$in" => {
                clauses.push(compile_in(path, opval, params, false)?);
                continue;
            }
            "$nin" => {
                clauses.push(compile_in(path, opval, params, true)?);
                continue;
            }
            "$regex" => {
                clauses.push(compile_regex(path, opval, params)?);
                continue;
            }
            other => {
                return Err(DataLayerError::SchemaViolation(format!(
                    "unsupported operator: {other}"
                )));
            }
        };
        let bound = json_scalar_to_value(opval)?;
        params.push(json_path_param(path));
        params.push(bound);
        clauses.push(format!("json_extract(payload, ?) {sql_op} ?"));
    }
    Ok(format!("({})", clauses.join(" AND ")))
}

fn compile_in(
    path: &str,
    value: &Json,
    params: &mut Vec<Value>,
    negate: bool,
) -> Result<String, DataLayerError> {
    let Json::Array(items) = value else {
        return Err(DataLayerError::SchemaViolation(format!(
            "$in/$nin requires an array for field '{path}'"
        )));
    };
    if items.is_empty() {
        return Err(DataLayerError::SchemaViolation(format!(
            "$in/$nin requires a non-empty array for field '{path}'"
        )));
    }
    let placeholders = vec!["?"; items.len()].join(", ");
    params.push(json_path_param(path));
    for item in items {
        params.push(json_scalar_to_value(item)?);
    }
    let op = if negate { "NOT IN" } else { "IN" };
    Ok(format!("json_extract(payload, ?) {op} ({placeholders})"))
}

fn compile_regex(
    path: &str,
    value: &Json,
    params: &mut Vec<Value>,
) -> Result<String, DataLayerError> {
    let Json::String(pattern) = value else {
        return Err(DataLayerError::SchemaViolation(format!(
            "$regex requires a string pattern for field '{path}'"
        )));
    };
    params.push(json_path_param(path));
    params.push(Value::Text(format!("%{pattern}%")));
    Ok("json_extract(payload, ?) LIKE ?".to_string())
}

fn compile_equality(
    path: &str,
    value: &Json,
    params: &mut Vec<Value>,
) -> Result<String, DataLayerError> {
    if value.is_null() {
        params.push(json_path_param(path));
        return Ok("json_extract(payload, ?) IS NULL".to_string());
    }
    params.push(json_path_param(path));
    params.push(json_scalar_to_value(value)?);
    Ok("json_extract(payload, ?) = ?".to_string())
}

fn json_path_param(path: &str) -> Value {
    Value::Text(format!("$.{path}"))
}

fn json_scalar_to_value(value: &Json) -> Result<Value, DataLayerError> {
    match value {
        Json::String(s) => Ok(Value::Text(s.clone())),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Real(f))
            } else {
                Err(DataLayerError::SchemaViolation("unsupported numeric filter value".to_string()))
            }
        }
        Json::Bool(b) => Ok(Value::Integer(i64::from(*b))),
        Json::Null => Ok(Value::Null),
        other => {
            Err(DataLayerError::SchemaViolation(format!("unsupported filter value type: {other}")))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn compile(json: &str) -> CompiledFilter {
        compile_filter(Some(json)).unwrap().unwrap()
    }

    #[test]
    fn test_none_and_empty_filter_compile_to_none() {
        assert!(compile_filter(None).unwrap().is_none());
        assert!(compile_filter(Some("")).unwrap().is_none());
        assert!(compile_filter(Some("{}")).unwrap().is_none());
    }

    #[test]
    fn test_equality() {
        let f = compile(r#"{"name": "alice"}"#);
        assert_eq!(f.where_clause, "json_extract(payload, ?) = ?");
        assert_eq!(f.params, vec![Value::Text("$.name".into()), Value::Text("alice".into())]);
    }

    #[test]
    fn test_gt_operator() {
        let f = compile(r#"{"age": {"$gt": 18}}"#);
        assert_eq!(f.where_clause, "(json_extract(payload, ?) > ?)");
        assert_eq!(f.params, vec![Value::Text("$.age".into()), Value::Integer(18)]);
    }

    #[test]
    fn test_in_operator() {
        let f = compile(r#"{"status": {"$in": ["a", "b"]}}"#);
        assert_eq!(f.where_clause, "(json_extract(payload, ?) IN (?, ?))");
        assert_eq!(
            f.params,
            vec![Value::Text("$.status".into()), Value::Text("a".into()), Value::Text("b".into())]
        );
    }

    #[test]
    fn test_regex_operator() {
        let f = compile(r#"{"name": {"$regex": "ali"}}"#);
        assert_eq!(f.where_clause, "(json_extract(payload, ?) LIKE ?)");
        assert_eq!(f.params, vec![Value::Text("$.name".into()), Value::Text("%ali%".into())]);
    }

    #[test]
    fn test_and_operator() {
        let f = compile(r#"{"$and": [{"age": {"$gt": 18}}, {"name": "alice"}]}"#);
        assert_eq!(
            f.where_clause,
            "((json_extract(payload, ?) > ?) AND json_extract(payload, ?) = ?)"
        );
    }

    #[test]
    fn test_dot_notation() {
        let f = compile(r#"{"address.city": "London"}"#);
        assert_eq!(f.params[0], Value::Text("$.address.city".into()));

        let f2 = compile(r#"{"address": {"city": "London"}}"#);
        assert_eq!(f2.params[0], Value::Text("$.address.city".into()));
    }

    #[test]
    fn test_unsupported_operator_rejected() {
        let err = compile_filter(Some(r#"{"name": {"$lookup": 1}}"#)).unwrap_err();
        match err {
            DataLayerError::SchemaViolation(msg) => assert!(msg.contains("unsupported operator")),
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn test_nested_over_10_levels_rejected() {
        // Build a filter nested 12 levels deep.
        let mut json = "1".to_string();
        for i in 0..12 {
            json = format!(r#"{{"f{i}": {json}}}"#);
        }
        let err = compile_filter(Some(&json)).unwrap_err();
        match err {
            DataLayerError::SchemaViolation(msg) => assert!(msg.contains("too deeply nested")),
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn test_sql_injection_attempt_is_bound_not_interpolated() {
        let f = compile(r#"{"name": "'; DROP TABLE profiles; --"}"#);
        assert_eq!(f.where_clause, "json_extract(payload, ?) = ?");
        assert_eq!(f.params[1], Value::Text("'; DROP TABLE profiles; --".into()));
        assert!(!f.where_clause.contains("DROP TABLE"));
    }

    #[test]
    fn test_empty_in_array_rejected() {
        let err = compile_filter(Some(r#"{"status": {"$in": []}}"#)).unwrap_err();
        assert!(matches!(err, DataLayerError::SchemaViolation(_)));
    }

    #[test]
    fn test_invalid_json_rejected() {
        let err = compile_filter(Some("not json")).unwrap_err();
        assert!(matches!(err, DataLayerError::SchemaViolation(_)));
    }
}
