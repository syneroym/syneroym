use std::collections::HashSet;

use rusqlite::{Connection, params, types::Value as SqlValue};
use serde_json::Value;
use syneroym_text_lang::SEARCH_KEY_VERSION;

use crate::{errors::map_rusqlite_error, host_store::DataLayerError};

pub(crate) const TRI_TOKENIZE: &str = "tokenize = 'trigram case_sensitive 1'";
pub(crate) const WORD_TOKENIZE: &str =
    "tokenize = \"unicode61 remove_diacritics 0 categories 'L* N* Co M*'\", prefix = '1 2'";

fn map_text_index_error(err: rusqlite::Error, collection: &str) -> DataLayerError {
    let msg = err.to_string();
    if msg.contains("no such table") {
        DataLayerError::Internal(format!("text index of '{collection}' is missing"))
    } else {
        map_rusqlite_error(err)
    }
}

fn set_equal(a: &[String], b: &[String]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let set_a: HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    b.iter().all(|s| set_a.contains(s.as_str()))
}

/// The text fields of `collection`, in declared order; empty when it
/// has no text index. Reads `SELECT name FROM pragma_table_info(?1)` of
/// `_text_<c>_word` and strips `f_`. A missing table gives no rows, not
/// an error, so raw test connections need no host table.
pub(super) fn fields(conn: &Connection, collection: &str) -> Result<Vec<String>, DataLayerError> {
    let word_table = format!("_text_{collection}_word");
    let mut stmt = conn
        .prepare_cached("SELECT name FROM pragma_table_info(?1)")
        .map_err(map_rusqlite_error)?;
    let rows = stmt
        .query_map(params![word_table], |row| row.get::<_, String>(0))
        .map_err(map_rusqlite_error)?;
    let mut result = Vec::new();
    for r in rows {
        let name = r.map_err(map_rusqlite_error)?;
        if let Some(field) = name.strip_prefix("f_") {
            result.push(field.to_string());
        }
    }
    Ok(result)
}

/// Check that every text field of `payload` is a string, null or absent.
pub(super) fn check_values(fields: &[String], payload: &Value) -> Result<(), DataLayerError> {
    if fields.is_empty() {
        return Ok(());
    }
    if let Some(obj) = payload.as_object() {
        for f in fields {
            if let Some(v) = obj.get(f)
                && !v.is_string()
                && !v.is_null()
            {
                return Err(DataLayerError::SchemaViolation(format!(
                    "text field '{f}' must be a string"
                )));
            }
        }
    }
    Ok(())
}

/// Create, replace or keep the text index of `collection`.
pub(super) fn declare(
    conn: &Connection,
    collection: &str,
    wanted: &[String],
) -> Result<(), DataLayerError> {
    if wanted.is_empty() {
        return Ok(());
    }
    let have = fields(conn, collection)?;
    let c_lower = collection.to_ascii_lowercase();

    let tri_table = format!("_text_{collection}_tri");
    let word_table = format!("_text_{collection}_word");

    let tri_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 COLLATE \
             NOCASE)",
            params![tri_table],
            |row| row.get(0),
        )
        .map_err(map_rusqlite_error)?;

    let version_matches = conn
        .query_row(
            "SELECT key_version FROM _text_index WHERE collection = ?1",
            params![c_lower],
            |row| row.get::<_, u32>(0),
        )
        .map(|v| v == SEARCH_KEY_VERSION)
        .unwrap_or(false);

    if set_equal(&have, wanted) && tri_exists && version_matches {
        return Ok(());
    }

    conn.execute(
        "CREATE TABLE IF NOT EXISTS _text_index (
            collection  TEXT PRIMARY KEY,
            key_version INTEGER NOT NULL
        )",
        [],
    )
    .map_err(map_rusqlite_error)?;

    conn.execute(&format!("DROP TABLE IF EXISTS {tri_table}"), []).map_err(map_rusqlite_error)?;
    conn.execute(&format!("DROP TABLE IF EXISTS {word_table}"), []).map_err(map_rusqlite_error)?;

    let cols = wanted.iter().map(|f| format!("f_{f}")).collect::<Vec<_>>().join(", ");
    conn.execute(
        &format!("CREATE VIRTUAL TABLE {tri_table} USING fts5({cols}, {TRI_TOKENIZE})"),
        [],
    )
    .map_err(map_rusqlite_error)?;
    conn.execute(
        &format!("CREATE VIRTUAL TABLE {word_table} USING fts5({cols}, {WORD_TOKENIZE})"),
        [],
    )
    .map_err(map_rusqlite_error)?;

    let records: Vec<(i64, String)> = {
        let mut stmt = conn
            .prepare(&format!("SELECT rowid, payload FROM {collection}"))
            .map_err(map_rusqlite_error)?;
        let rows =
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?))).map_err(map_rusqlite_error)?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(map_rusqlite_error)?);
        }
        list
    };

    let mut skipped = 0usize;
    for (rowid, payload_str) in records {
        let Ok(val) = serde_json::from_str::<Value>(&payload_str) else {
            continue;
        };
        if let Some(obj) = val.as_object() {
            for f in wanted {
                if let Some(v) = obj.get(f)
                    && !v.is_string()
                    && !v.is_null()
                {
                    skipped += 1;
                }
            }
        }
        sync_row(conn, collection, wanted, rowid, &val)?;
    }

    conn.execute(
        "INSERT INTO _text_index (collection, key_version) VALUES (?1, ?2) ON \
         CONFLICT(collection) DO UPDATE SET key_version = excluded.key_version",
        params![c_lower, SEARCH_KEY_VERSION],
    )
    .map_err(map_rusqlite_error)?;

    if skipped > 0 {
        tracing::warn!(
            collection,
            skipped,
            "text index rebuild skipped values that are not strings"
        );
    }

    Ok(())
}

/// Drop both tables and the `_text_index` row (if `_text_index` exists).
pub(super) fn drop_index(conn: &Connection, collection: &str) -> Result<(), DataLayerError> {
    let tri_table = format!("_text_{collection}_tri");
    let word_table = format!("_text_{collection}_word");
    conn.execute(&format!("DROP TABLE IF EXISTS {tri_table}"), []).map_err(map_rusqlite_error)?;
    conn.execute(&format!("DROP TABLE IF EXISTS {word_table}"), []).map_err(map_rusqlite_error)?;

    let has_index_table: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = \
             '_text_index')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);
    if has_index_table {
        let c_lower = collection.to_ascii_lowercase();
        conn.execute("DELETE FROM _text_index WHERE collection = ?1", params![c_lower])
            .map_err(map_rusqlite_error)?;
    }
    Ok(())
}

/// Replace the index rows of `rowid` with the keys of `payload`.
pub(super) fn sync_row(
    conn: &Connection,
    collection: &str,
    fields: &[String],
    rowid: i64,
    payload: &Value,
) -> Result<(), DataLayerError> {
    if fields.is_empty() {
        return Ok(());
    }
    let tri_table = format!("_text_{collection}_tri");
    let word_table = format!("_text_{collection}_word");

    let del_tri_sql = format!("DELETE FROM {tri_table} WHERE rowid = ?1");
    let del_word_sql = format!("DELETE FROM {word_table} WHERE rowid = ?1");

    conn.execute(&del_tri_sql, params![rowid]).map_err(|e| map_text_index_error(e, collection))?;
    conn.execute(&del_word_sql, params![rowid]).map_err(|e| map_text_index_error(e, collection))?;

    let obj = payload.as_object();
    let mut keys = Vec::with_capacity(fields.len());
    for f in fields {
        let val_opt = obj.and_then(|m| m.get(f)).and_then(|v| v.as_str());
        let key_opt = val_opt.map(syneroym_text_lang::search_key);
        keys.push(key_opt);
    }

    let col_names = fields.iter().map(|f| format!("f_{f}")).collect::<Vec<_>>().join(", ");
    let placeholders =
        (2..=fields.len() + 1).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");

    let tri_ins_sql =
        format!("INSERT INTO {tri_table} (rowid, {col_names}) VALUES (?1, {placeholders})");
    let word_ins_sql =
        format!("INSERT INTO {word_table} (rowid, {col_names}) VALUES (?1, {placeholders})");

    let mut params_vec: Vec<SqlValue> = Vec::with_capacity(fields.len() + 1);
    params_vec.push(SqlValue::Integer(rowid));
    for k in keys {
        match k {
            Some(s) => params_vec.push(SqlValue::Text(s)),
            None => params_vec.push(SqlValue::Null),
        }
    }

    conn.execute(&tri_ins_sql, rusqlite::params_from_iter(params_vec.iter()))
        .map_err(|e| map_text_index_error(e, collection))?;
    conn.execute(&word_ins_sql, rusqlite::params_from_iter(params_vec.iter()))
        .map_err(|e| map_text_index_error(e, collection))?;

    Ok(())
}

/// Remove the index rows of these rowids.
pub(super) fn remove_rows(
    conn: &Connection,
    collection: &str,
    rowids: &[i64],
) -> Result<(), DataLayerError> {
    if rowids.is_empty() {
        return Ok(());
    }
    let tri_table = format!("_text_{collection}_tri");
    let word_table = format!("_text_{collection}_word");

    let del_tri_sql = format!("DELETE FROM {tri_table} WHERE rowid = ?1");
    let del_word_sql = format!("DELETE FROM {word_table} WHERE rowid = ?1");

    for &rid in rowids {
        conn.execute(&del_tri_sql, params![rid])
            .map_err(|e| map_text_index_error(e, collection))?;
        conn.execute(&del_word_sql, params![rid])
            .map_err(|e| map_text_index_error(e, collection))?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests;
