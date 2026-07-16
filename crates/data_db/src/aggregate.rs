//! MongoDB-style JSON aggregation document -> parameterized SQLite
//! `SELECT ... GROUP BY ... HAVING ...` compiler (ADR-0007, Slice B4).
//!
//! Pure and DB-free. Field paths and all literal values are bound as `?`
//! placeholders; the only caller-derived text ever interpolated into SQL is
//! output *aliases* and *field names*, each first passed through
//! `validate_identifier` (SQL-identifier charset only) -- never a value, never
//! an operator.

use std::collections::BTreeSet;

use rusqlite::types::Value;
use serde_json::{Map, Value as Json};

use crate::{
    filter,
    host_store::DataLayerError,
    sqlite::{MAX_QUERY_PAGE_SIZE, validate_identifier},
};

const MAX_HAVING_DEPTH: u32 = 10;

const ALLOWED_STAGE_KEYS: [&str; 7] =
    ["$match", "$group", "$having", "$project", "$sort", "$limit", "$skip"];

/// A compiled aggregation: the full SQL statement plus its bound parameters
/// in binding (left-to-right textual) order.
#[derive(Debug, Clone)]
pub struct CompiledAggregation {
    pub sql: String,
    pub params: Vec<Value>,
}

/// Compiles a MongoDB-style JSON aggregation document into a parameterized
/// SQLite `SELECT ... GROUP BY ... HAVING ...` statement targeting
/// `collection`. `collection` is validated here too (belt-and-suspenders --
/// `do_aggregate` also validates it before calling this function).
pub fn compile(
    collection: &str,
    pipeline_json: &str,
) -> Result<CompiledAggregation, DataLayerError> {
    validate_identifier(collection)?;
    let doc = parse_object(pipeline_json)?;

    for key in doc.keys() {
        if !ALLOWED_STAGE_KEYS.contains(&key.as_str()) {
            return Err(schema(format!("unsupported aggregation stage: {key}")));
        }
    }

    let group_val = doc.get("$group").ok_or_else(|| schema("aggregate requires a $group stage"))?;
    let group = compile_group(group_val)?;

    let (where_sql, match_params) = match doc.get("$match") {
        Some(match_val) => {
            let raw = serde_json::to_string(match_val)
                .map_err(|e| schema(format!("invalid $match document: {e}")))?;
            match filter::compile_filter(Some(&raw))? {
                Some(compiled) => (Some(compiled.where_clause), compiled.params),
                None => (None, Vec::new()),
            }
        }
        None => (None, Vec::new()),
    };

    let (having_sql, having_params) = match doc.get("$having") {
        Some(having_val) => {
            let mut params = Vec::new();
            let clause = compile_having(having_val, &group.aliases, 0, &mut params)?;
            (Some(clause), params)
        }
        None => (None, Vec::new()),
    };

    let mut inner = format!("SELECT {} FROM {collection}", group.select_exprs.join(", "));
    if let Some(where_clause) = &where_sql {
        inner.push_str(&format!(" WHERE {where_clause}"));
    }
    if group.group_by_id {
        inner.push_str(" GROUP BY _id");
    }
    if let Some(having_clause) = &having_sql {
        inner.push_str(&format!(" HAVING {having_clause}"));
    }

    let mut params = group.params;
    params.extend(match_params);
    params.extend(having_params);

    let project = doc.get("$project");
    let sort = doc.get("$sort");
    let limit = doc.get("$limit");
    let skip = doc.get("$skip");

    let mut sql = if project.is_some() || sort.is_some() || limit.is_some() || skip.is_some() {
        let cols = match project {
            Some(p) => compile_project(p, &group.aliases)?,
            None => "*".to_string(),
        };
        format!("SELECT {cols} FROM ({inner})")
    } else {
        inner
    };

    match sort {
        Some(sort_val) => {
            let order = compile_sort(sort_val, &group.aliases)?;
            sql.push_str(&format!(" ORDER BY {order}"));
        }
        // F5: determinism nicety -- absent an explicit `$sort`, a grouped
        // result gets a stable default order rather than depending on
        // SQLite's unspecified `GROUP BY` row order.
        None if group.group_by_id => sql.push_str(" ORDER BY _id ASC"),
        None => {}
    }

    let (limit_sql, limit_params) = compile_limit_skip(limit, skip)?;
    if let Some(limit_clause) = limit_sql {
        sql.push_str(&format!(" {limit_clause}"));
        params.extend(limit_params);
    }

    Ok(CompiledAggregation { sql, params })
}

fn parse_object(pipeline_json: &str) -> Result<Map<String, Json>, DataLayerError> {
    let parsed: Json = serde_json::from_str(pipeline_json)
        .map_err(|e| schema(format!("invalid aggregation JSON: {e}")))?;
    match parsed {
        Json::Object(obj) => Ok(obj),
        _ => Err(schema("aggregation pipeline must be a JSON object")),
    }
}

struct GroupCompilation {
    select_exprs: Vec<String>,
    params: Vec<Value>,
    aliases: BTreeSet<String>,
    group_by_id: bool,
}

/// Compiles the `$group` stage's `_id` key and accumulators into the
/// `SELECT`-list. Iterates `group_obj` (a `BTreeMap`-backed
/// `serde_json::Map` -- this workspace does not enable `preserve_order`) so
/// accumulator output order is alphabetical and deterministic; `_id` is
/// always emitted first regardless, since it is handled before the loop.
fn compile_group(group_val: &Json) -> Result<GroupCompilation, DataLayerError> {
    let Json::Object(group_obj) = group_val else {
        return Err(schema("$group must be a JSON object"));
    };

    let mut select_exprs = Vec::new();
    let mut params = Vec::new();
    let mut aliases = BTreeSet::new();
    let mut group_by_id = false;

    match group_obj.get("_id") {
        None | Some(Json::Null) => {}
        Some(Json::String(path)) => {
            select_exprs.push("json_extract(payload, ?) AS _id".to_string());
            params.push(payload_path_param(path));
            aliases.insert("_id".to_string());
            group_by_id = true;
        }
        Some(Json::Object(_)) => return Err(schema("composite $group._id is not supported")),
        Some(_) => return Err(schema("$group._id must be a string field path or null")),
    }

    for (alias, spec) in group_obj {
        if alias == "_id" {
            continue;
        }
        validate_identifier(alias)?;
        let Json::Object(acc_obj) = spec else {
            return Err(schema(format!("accumulator '{alias}' must be an object")));
        };
        if acc_obj.len() != 1 {
            return Err(schema(format!("accumulator '{alias}' must have exactly one operator")));
        }
        #[allow(clippy::expect_used)]
        let (op, arg) = acc_obj.iter().next().expect("length checked to be exactly 1 above");
        let expr = match op.as_str() {
            "$sum" => match arg {
                Json::Number(n) if n.as_i64() == Some(1) => format!("COUNT(*) AS {alias}"),
                Json::Number(_) => {
                    return Err(schema("$sum literal must be 1 (use a field path to sum a field)"));
                }
                Json::String(field) => {
                    params.push(payload_path_param(field));
                    format!("SUM(json_extract(payload, ?)) AS {alias}")
                }
                _ => return Err(schema(format!("unsupported $sum argument for '{alias}'"))),
            },
            "$avg" | "$min" | "$max" => {
                let Json::String(field) = arg else {
                    return Err(schema(format!("'{op}' requires a field path for '{alias}'")));
                };
                let sql_fn = match op.as_str() {
                    "$avg" => "AVG",
                    "$min" => "MIN",
                    _ => "MAX",
                };
                params.push(payload_path_param(field));
                format!("{sql_fn}(json_extract(payload, ?)) AS {alias}")
            }
            other => return Err(schema(format!("unsupported accumulator: {other}"))),
        };
        select_exprs.push(expr);
        aliases.insert(alias.clone());
    }

    if select_exprs.is_empty() {
        return Err(schema("$group must define _id or at least one accumulator"));
    }

    Ok(GroupCompilation { select_exprs, params, aliases, group_by_id })
}

/// Compiles a `$having` document. A second, independent recursive compiler
/// from `filter::compile_filter` (deliberately -- `$having` operates on bare
/// output aliases, never `json_extract(payload, ...)`), with its own depth
/// guard (R1.3) since it does not inherit `filter.rs`'s.
fn compile_having(
    doc: &Json,
    aliases: &BTreeSet<String>,
    depth: u32,
    params: &mut Vec<Value>,
) -> Result<String, DataLayerError> {
    if depth > MAX_HAVING_DEPTH {
        return Err(schema("$having document too deeply nested"));
    }
    let Json::Object(obj) = doc else {
        return Err(schema("$having must be a JSON object"));
    };
    if obj.is_empty() {
        return Ok("1=1".to_string());
    }

    let mut clauses = Vec::with_capacity(obj.len());
    for (key, value) in obj {
        clauses.push(compile_having_entry(key, value, aliases, depth, params)?);
    }
    Ok(clauses.join(" AND "))
}

fn compile_having_entry(
    key: &str,
    value: &Json,
    aliases: &BTreeSet<String>,
    depth: u32,
    params: &mut Vec<Value>,
) -> Result<String, DataLayerError> {
    match key {
        "$and" => compile_having_logical(value, aliases, depth, params, " AND "),
        "$or" => compile_having_logical(value, aliases, depth, params, " OR "),
        _ if key.starts_with('$') => Err(schema(format!("unsupported $having operator: {key}"))),
        alias => {
            if !aliases.contains(alias) {
                return Err(schema(format!("$having references unknown field: {alias}")));
            }
            compile_having_value(alias, value, params)
        }
    }
}

fn compile_having_logical(
    value: &Json,
    aliases: &BTreeSet<String>,
    depth: u32,
    params: &mut Vec<Value>,
    joiner: &str,
) -> Result<String, DataLayerError> {
    let Json::Array(items) = value else {
        return Err(schema("$and/$or requires an array of $having documents"));
    };
    if items.is_empty() {
        return Err(schema("$and/$or requires at least one sub-document"));
    }
    let mut parts = Vec::with_capacity(items.len());
    for item in items {
        parts.push(compile_having(item, aliases, depth + 1, params)?);
    }
    Ok(format!("({})", parts.join(joiner)))
}

fn compile_having_value(
    alias: &str,
    value: &Json,
    params: &mut Vec<Value>,
) -> Result<String, DataLayerError> {
    match value {
        Json::Object(obj) if obj.keys().any(|k| k.starts_with('$')) => {
            let mut clauses = Vec::with_capacity(obj.len());
            for (op, opval) in obj {
                let sql_op = match op.as_str() {
                    "$gt" => ">",
                    "$gte" => ">=",
                    "$lt" => "<",
                    "$lte" => "<=",
                    "$ne" => "!=",
                    other => return Err(schema(format!("unsupported $having operator: {other}"))),
                };
                params.push(json_scalar_to_value(opval)?);
                clauses.push(format!("{alias} {sql_op} ?"));
            }
            if clauses.len() == 1 {
                #[allow(clippy::expect_used)]
                Ok(clauses.into_iter().next().expect("length checked to be exactly 1 above"))
            } else {
                Ok(format!("({})", clauses.join(" AND ")))
            }
        }
        Json::Object(_) | Json::Array(_) => {
            Err(schema(format!("unsupported $having value type for '{alias}'")))
        }
        scalar => {
            params.push(json_scalar_to_value(scalar)?);
            Ok(format!("{alias} = ?"))
        }
    }
}

fn compile_project(
    project_val: &Json,
    aliases: &BTreeSet<String>,
) -> Result<String, DataLayerError> {
    let Json::Array(items) = project_val else {
        return Err(schema("$project must be an array of output column names"));
    };
    if items.is_empty() {
        return Err(schema("$project must name at least one output column"));
    }
    let mut cols = Vec::with_capacity(items.len());
    for item in items {
        let Json::String(name) = item else {
            return Err(schema("$project entries must be strings"));
        };
        if !aliases.contains(name) {
            return Err(schema(format!("$project references unknown field: {name}")));
        }
        cols.push(name.clone());
    }
    Ok(cols.join(", "))
}

fn compile_sort(sort_val: &Json, aliases: &BTreeSet<String>) -> Result<String, DataLayerError> {
    let Json::Object(obj) = sort_val else {
        return Err(schema("$sort must be an object of {field: 1 | -1}"));
    };
    if obj.is_empty() {
        return Err(schema("$sort must name at least one field"));
    }
    let mut parts = Vec::with_capacity(obj.len());
    for (name, dir) in obj {
        if !aliases.contains(name) {
            return Err(schema(format!("$sort references unknown field: {name}")));
        }
        let dir_sql = match dir.as_i64() {
            Some(1) => "ASC",
            Some(-1) => "DESC",
            _ => return Err(schema(format!("$sort direction for '{name}' must be 1 or -1"))),
        };
        parts.push(format!("{name} {dir_sql}"));
    }
    Ok(parts.join(", "))
}

/// `$limit`/`$skip` -> `LIMIT ?`/`LIMIT ? OFFSET ?`. SQLite requires a
/// `LIMIT` before an `OFFSET`, so `$skip` without `$limit` binds `LIMIT` to
/// `MAX_QUERY_PAGE_SIZE` -- this is what lets a caller page past
/// `run_query_raw`'s row cap (R2.2): page *k* is
/// `{"$skip": k*pagesize, "$limit": pagesize}`.
fn compile_limit_skip(
    limit: Option<&Json>,
    skip: Option<&Json>,
) -> Result<(Option<String>, Vec<Value>), DataLayerError> {
    let limit_val = match limit {
        Some(Json::Number(n)) => Some(
            n.as_i64()
                .filter(|v| *v > 0)
                .ok_or_else(|| schema("$limit must be a positive integer"))?
                .min(i64::from(MAX_QUERY_PAGE_SIZE)),
        ),
        Some(_) => return Err(schema("$limit must be a positive integer")),
        None => None,
    };
    let skip_val = match skip {
        Some(Json::Number(n)) => Some(
            n.as_i64()
                .filter(|v| *v >= 0)
                .ok_or_else(|| schema("$skip must be a non-negative integer"))?,
        ),
        Some(_) => return Err(schema("$skip must be a non-negative integer")),
        None => None,
    };

    match (limit_val, skip_val) {
        (None, None) => Ok((None, Vec::new())),
        (Some(l), None) => Ok((Some("LIMIT ?".to_string()), vec![Value::Integer(l)])),
        (l, Some(s)) => {
            let l = l.unwrap_or(i64::from(MAX_QUERY_PAGE_SIZE));
            Ok((Some("LIMIT ? OFFSET ?".to_string()), vec![Value::Integer(l), Value::Integer(s)]))
        }
    }
}

fn schema(msg: impl Into<String>) -> DataLayerError {
    DataLayerError::SchemaViolation(msg.into())
}

fn payload_path_param(path: &str) -> Value {
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
                Err(schema("unsupported numeric $having value"))
            }
        }
        Json::Bool(b) => Ok(Value::Integer(i64::from(*b))),
        Json::Null => Ok(Value::Null),
        other => Err(schema(format!("unsupported $having value type: {other}"))),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn compile_ok(pipeline: &str) -> CompiledAggregation {
        compile("people", pipeline).unwrap()
    }

    fn compile_err(pipeline: &str) -> DataLayerError {
        compile("people", pipeline).unwrap_err()
    }

    fn expect_schema_violation(err: &DataLayerError, contains: &str) {
        match err {
            DataLayerError::SchemaViolation(msg) => {
                assert!(
                    msg.contains(contains),
                    "expected message to contain {contains:?}, got {msg:?}"
                );
            }
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn test_group_count_by_field() {
        let c = compile_ok(r#"{"$group":{"_id":"category","n":{"$sum":1}}}"#);
        assert!(c.sql.contains("json_extract(payload, ?) AS _id"));
        assert!(c.sql.contains("COUNT(*) AS n"));
        assert!(c.sql.contains("GROUP BY _id"));
        assert_eq!(c.params, vec![Value::Text("$.category".into())]);
    }

    #[test]
    fn test_group_sum_avg_min_max() {
        let c = compile_ok(
            r#"{"$group":{"_id":"category","total":{"$sum":"amount"},
                "avg_amount":{"$avg":"amount"},"min_amount":{"$min":"amount"},
                "max_amount":{"$max":"amount"}}}"#,
        );
        assert!(c.sql.contains("SUM(json_extract(payload, ?)) AS total"));
        assert!(c.sql.contains("AVG(json_extract(payload, ?)) AS avg_amount"));
        assert!(c.sql.contains("MIN(json_extract(payload, ?)) AS min_amount"));
        assert!(c.sql.contains("MAX(json_extract(payload, ?)) AS max_amount"));
        // alphabetical accumulator order (BTreeMap, no preserve_order):
        // avg_amount, max_amount, min_amount, total
        let avg_pos = c.sql.find("avg_amount").unwrap();
        let max_pos = c.sql.find("max_amount").unwrap();
        let min_pos = c.sql.find("min_amount").unwrap();
        let total_pos = c.sql.find(" AS total").unwrap();
        assert!(avg_pos < max_pos && max_pos < min_pos && min_pos < total_pos);
        assert_eq!(c.params.len(), 5);
        assert_eq!(c.params[0], Value::Text("$.category".into()));
    }

    #[test]
    fn test_group_id_null_no_group_by() {
        let c = compile_ok(r#"{"$group":{"_id":null,"n":{"$sum":1}}}"#);
        assert!(!c.sql.contains("GROUP BY"));
        assert!(!c.sql.contains("_id"));
        assert!(c.sql.contains("COUNT(*) AS n"));
    }

    #[test]
    fn test_match_compiles_to_where() {
        let c =
            compile_ok(r#"{"$match":{"active":true},"$group":{"_id":"category","n":{"$sum":1}}}"#);
        assert!(c.sql.contains("WHERE json_extract(payload, ?) = ?"));
        assert_eq!(
            c.params,
            vec![
                Value::Text("$.category".into()),
                Value::Text("$.active".into()),
                Value::Integer(1)
            ]
        );
    }

    #[test]
    fn test_having_on_alias() {
        let c = compile_ok(r#"{"$group":{"_id":"cat","n":{"$sum":1}},"$having":{"n":{"$gt":5}}}"#);
        assert!(c.sql.contains("HAVING n > ?"));
        assert_eq!(c.params.last(), Some(&Value::Integer(5)));
    }

    #[test]
    fn test_having_unknown_alias_rejected() {
        let err = compile_err(
            r#"{"$group":{"_id":"cat","n":{"$sum":1}},"$having":{"missing":{"$gt":5}}}"#,
        );
        expect_schema_violation(&err, "unknown field");
    }

    #[test]
    fn test_project_reorders_columns() {
        let c = compile_ok(r#"{"$group":{"_id":"cat","n":{"$sum":1}},"$project":["n","_id"]}"#);
        assert!(c.sql.contains("SELECT n, _id FROM ("));
    }

    #[test]
    fn test_sort_and_limit() {
        let c =
            compile_ok(r#"{"$group":{"_id":"cat","n":{"$sum":1}},"$sort":{"n":-1},"$limit":10}"#);
        assert!(c.sql.contains("ORDER BY n DESC"));
        assert!(c.sql.trim_end().ends_with("LIMIT ?"));
        assert_eq!(c.params.last(), Some(&Value::Integer(10)));
    }

    #[test]
    fn test_skip_maps_to_offset() {
        let c = compile_ok(r#"{"$group":{"_id":"cat","n":{"$sum":1}},"$skip":20,"$limit":10}"#);
        assert!(c.sql.contains("LIMIT ? OFFSET ?"));
        let tail = &c.params[c.params.len() - 2..];
        assert_eq!(tail, [Value::Integer(10), Value::Integer(20)]);

        let c2 = compile_ok(r#"{"$group":{"_id":"cat","n":{"$sum":1}},"$skip":20}"#);
        assert!(c2.sql.contains("LIMIT ? OFFSET ?"));
        let tail2 = &c2.params[c2.params.len() - 2..];
        assert_eq!(tail2, [Value::Integer(i64::from(MAX_QUERY_PAGE_SIZE)), Value::Integer(20)]);
    }

    #[test]
    fn test_empty_group_rejected() {
        let err = compile_err(r#"{"$group":{"_id":null}}"#);
        expect_schema_violation(&err, "_id or at least one accumulator");
    }

    #[test]
    fn test_having_nested_over_depth_rejected() {
        let mut having = r#"{"n":{"$gt":0}}"#.to_string();
        for _ in 0..12 {
            having = format!(r#"{{"$and":[{having}]}}"#);
        }
        let pipeline =
            format!(r#"{{"$group":{{"_id":"cat","n":{{"$sum":1}}}},"$having":{having}}}"#);
        let err = compile_err(&pipeline);
        expect_schema_violation(&err, "too deeply nested");
    }

    #[test]
    fn test_injection_in_field_path_is_bound() {
        let c = compile_ok(r#"{"$group":{"_id":"x'; DROP TABLE t; --","n":{"$sum":1}}}"#);
        assert!(!c.sql.contains("DROP TABLE"));
        assert!(c.params.contains(&Value::Text("$.x'; DROP TABLE t; --".into())));
    }

    #[test]
    fn test_injection_in_alias_rejected() {
        let err = compile_err(r#"{"$group":{"_id":"cat","n; DROP TABLE t":{"$sum":1}}}"#);
        assert!(matches!(err, DataLayerError::SchemaViolation(_)));
    }

    #[test]
    fn test_composite_id_rejected() {
        let err = compile_err(r#"{"$group":{"_id":{"c":"category","r":"region"},"n":{"$sum":1}}}"#);
        expect_schema_violation(&err, "composite");
    }

    #[test]
    fn test_unsupported_accumulator_rejected() {
        let err = compile_err(r#"{"$group":{"_id":"cat","n":{"$lookup":1}}}"#);
        expect_schema_violation(&err, "unsupported accumulator");
    }

    #[test]
    fn test_unsupported_stage_key_rejected() {
        let err = compile_err(r#"{"$group":{"_id":"cat","n":{"$sum":1}},"$lookup":{}}"#);
        expect_schema_violation(&err, "unsupported aggregation stage");
    }

    #[test]
    fn test_missing_group_rejected() {
        let err = compile_err(r#"{"$match":{"active":true}}"#);
        expect_schema_violation(&err, "requires a $group");
    }

    #[test]
    fn test_invalid_json_rejected() {
        let err = compile_err("not json");
        assert!(matches!(err, DataLayerError::SchemaViolation(_)));
    }

    #[test]
    fn test_sum_non_one_literal_rejected() {
        let err = compile_err(r#"{"$group":{"_id":"cat","n":{"$sum":2}}}"#);
        expect_schema_violation(&err, "$sum literal");
    }
}
