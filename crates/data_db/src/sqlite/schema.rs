use std::sync::LazyLock;

use regex::Regex;
use rusqlite::{
    Connection,
    hooks::{AuthAction, AuthContext, Authorization},
};

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
///
/// The vault table's name is refused: a guest must not create, drop or fill
/// a collection with that name.
pub(crate) fn validate_identifier(name: &str) -> Result<(), host_store::DataLayerError> {
    if IDENTIFIER_REGEX.is_match(name) && !name.eq_ignore_ascii_case(VAULT_TABLE) {
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

/// Whether `action` reads or changes the vault table. Guest SQL is not
/// allowed to touch it: the vault holds the secrets of the service and the
/// ones the substrate keeps for itself (`vault_names`).
pub(super) fn touches_vault(action: &AuthAction<'_>) -> bool {
    use AuthAction::{
        AlterTable, Analyze, CreateIndex, CreateTrigger, CreateVtable, Delete, DropIndex,
        DropTable, DropTrigger, DropVtable, Insert, Read, Update,
    };
    let table = match action {
        Read { table_name, .. }
        | Update { table_name, .. }
        | Insert { table_name }
        | Delete { table_name }
        | DropTable { table_name }
        | AlterTable { table_name, .. }
        | CreateIndex { table_name, .. }
        | DropIndex { table_name, .. }
        | CreateTrigger { table_name, .. }
        | DropTrigger { table_name, .. }
        | CreateVtable { table_name, .. }
        | DropVtable { table_name, .. }
        | Analyze { table_name } => table_name,
        _ => return false,
    };
    table.eq_ignore_ascii_case(VAULT_TABLE)
}

fn deny_vault_access(ctx: AuthContext<'_>) -> Authorization {
    if touches_vault(&ctx.action) { Authorization::Deny } else { Authorization::Allow }
}

/// Clears the authorizer when dropped, so the writer connection's own
/// statements (vault writes included) are never checked by it.
struct DdlAuthorizer<'c>(&'c Connection);

impl Drop for DdlAuthorizer<'_> {
    fn drop(&mut self) {
        let _ = self.0.authorizer::<fn(AuthContext<'_>) -> Authorization>(None);
    }
}

pub(super) fn do_execute_ddl(
    conn: &Connection,
    sql: &str,
) -> Result<(), host_store::DataLayerError> {
    conn.authorizer(Some(deny_vault_access)).map_err(map_rusqlite_error)?;
    let _authorizer = DdlAuthorizer(conn);
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
