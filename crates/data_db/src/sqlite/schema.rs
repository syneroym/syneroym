use std::sync::LazyLock;

use regex::Regex;
use rusqlite::Connection;

use crate::{errors::map_rusqlite_error, host_store};

pub(super) const SUBSTRATE_SCHEMA_VERSION: &str = "m3b";
pub(super) const VAULT_TABLE: &str = "_vault";
pub(super) const RECORD_COLUMNS: &str = "id, payload, creator_id, created_at, updated_at";
pub(super) const RECORD_COLUMNS_WITHOUT_ID: &str = "payload, creator_id, created_at, updated_at";

#[allow(clippy::unwrap_used)]
static IDENTIFIER_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_]{0,63}$").unwrap());

/// Validates a guest-supplied identifier (collection/table name, index
/// field name) before it is formatted into SQL text. Table and column names
/// cannot be bound as SQL parameters, so this allow-list is what stands in
/// for parameterization at the DDL boundary.
pub(crate) fn validate_identifier(name: &str) -> Result<(), host_store::DataLayerError> {
    if IDENTIFIER_REGEX.is_match(name) {
        Ok(())
    } else {
        Err(host_store::DataLayerError::SchemaViolation(format!("invalid identifier: {name}")))
    }
}

pub(super) fn do_create_collection(
    conn: &Connection,
    schema: &host_store::CollectionSchema,
) -> Result<(), host_store::DataLayerError> {
    validate_identifier(&schema.name)?;
    for idx in &schema.indexes {
        validate_identifier(&idx.field_name)?;
    }
    conn.execute(
        &format!(
            "CREATE TABLE IF NOT EXISTS {} (id TEXT PRIMARY KEY, payload JSON NOT NULL, \
             creator_id TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
            schema.name
        ),
        [],
    )
    .map_err(map_rusqlite_error)?;
    for idx in &schema.indexes {
        let index_name = format!("idx_{}_{}", schema.name, idx.field_name);
        conn.execute(
            &format!(
                "CREATE INDEX IF NOT EXISTS {index_name} ON {}(json_extract(payload, '$.{}'))",
                schema.name, idx.field_name
            ),
            [],
        )
        .map_err(map_rusqlite_error)?;
    }
    Ok(())
}

pub(super) fn do_drop_collection(
    conn: &Connection,
    name: &str,
) -> Result<(), host_store::DataLayerError> {
    validate_identifier(name)?;
    conn.execute(&format!("DROP TABLE IF EXISTS {name}"), []).map_err(map_rusqlite_error)?;
    Ok(())
}

pub(super) fn do_execute_ddl(
    conn: &Connection,
    sql: &str,
) -> Result<(), host_store::DataLayerError> {
    // Syntax-check first via a plain `prepare` (compiles without stepping the
    // statement, so nothing is mutated), then run the real statement(s).
    // NOTE: only the leading statement of a multi-statement `sql` is checked
    // this way; `execute_batch` below still validates the full batch.
    conn.prepare(&format!("EXPLAIN {sql}")).map_err(|e| {
        host_store::DataLayerError::Internal(format!("DDL syntax check failed: {e}"))
    })?;
    conn.execute_batch(sql)
        .map_err(|e| host_store::DataLayerError::Internal(format!("DDL execution failed: {e}")))?;
    Ok(())
}
