use std::{
    collections::HashMap,
    fmt,
    fs::DirBuilder,
    path::{Path, PathBuf},
    str,
    sync::{Arc, LazyLock, Mutex},
};

use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce, aead::Aead};
use async_trait::async_trait;
use chrono::Utc;
use deadpool_sqlite::{Config as PoolConfig, Hook, HookError, Pool, Runtime};
use rand::RngCore;
use regex::Regex;
use rusqlite::{Connection, Error as SqliteError, params, types::Value as SqlValue};
use serde_json::{Map, Value};
use syneroym_data_keystore::{KeyStore, KeyStoreError};
use tokio::{
    sync::{mpsc, oneshot},
    task,
};
use zeroize::Zeroizing;

use crate::{
    aggregate,
    errors::map_rusqlite_error,
    filter, host_store,
    traits::{ServiceStore, StorageProvider},
};

// Real service ids are DIDs (e.g. `did:key:h7wy...`), which contain colons;
// `:` is not a path separator on any Rust-supported OS, so allowing it here
// does not weaken the path-traversal guard below (which still relies on
// rejecting `.`/`/`/`\` and on the `starts_with` descendant check).
#[allow(clippy::unwrap_used)]
static SERVICE_ID_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_:\-]{1,128}$").unwrap());

#[allow(clippy::unwrap_used)]
static IDENTIFIER_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_]{0,63}$").unwrap());

const SUBSTRATE_SCHEMA_VERSION: &str = "m3b";

/// Hard upper bound on records returned per `query` page, enforced
/// regardless of what the guest requests via `query-options.limit`.
pub const MAX_QUERY_PAGE_SIZE: u32 = 1000;

/// Hard upper bound on the number of mutations accepted by a single
/// `batch-mutate` call, to bound how long the single-writer actor is
/// occupied by one request.
pub const MAX_BATCH_SIZE: usize = 200;

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

/// Applies an RFC 7396 JSON merge-patch: `patch` values overwrite `target`,
/// `null` values remove the key, and nested objects merge recursively.
fn apply_merge_patch(target: &mut Value, patch: &Value) {
    let Value::Object(patch_obj) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    #[allow(clippy::expect_used)]
    let target_obj = target.as_object_mut().expect("target was just coerced into an object");
    for (key, value) in patch_obj {
        if value.is_null() {
            target_obj.remove(key);
        } else {
            let entry = target_obj.entry(key.clone()).or_insert(Value::Null);
            apply_merge_patch(entry, value);
        }
    }
}

fn payload_to_text(payload: &[u8]) -> Result<String, host_store::DataLayerError> {
    let text = str::from_utf8(payload).map_err(|_| {
        host_store::DataLayerError::SchemaViolation("payload must be valid UTF-8".into())
    })?;
    serde_json::from_str::<Value>(text).map_err(|e| {
        host_store::DataLayerError::SchemaViolation(format!("payload is not valid JSON: {e}"))
    })?;
    Ok(text.to_string())
}

fn do_create_collection(
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

fn do_drop_collection(conn: &Connection, name: &str) -> Result<(), host_store::DataLayerError> {
    validate_identifier(name)?;
    conn.execute(&format!("DROP TABLE IF EXISTS {name}"), []).map_err(map_rusqlite_error)?;
    Ok(())
}

fn do_execute_ddl(conn: &Connection, sql: &str) -> Result<(), host_store::DataLayerError> {
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

fn do_put(
    conn: &Connection,
    collection: &str,
    value: &host_store::RecordWriteValue,
    creator_id: &str,
) -> Result<(), host_store::DataLayerError> {
    validate_identifier(collection)?;
    let payload_text = payload_to_text(&value.payload)?;
    let now = Utc::now().timestamp_millis();

    let existing_created_at: Option<i64> = conn
        .query_row(
            &format!("SELECT created_at FROM {collection} WHERE id = ?1"),
            params![value.id],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            SqliteError::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(map_rusqlite_error)?;
    let created_at = existing_created_at.unwrap_or(now);

    conn.execute(
        &format!(
            "INSERT INTO {collection} (id, payload, creator_id, created_at, updated_at) VALUES \
             (?1, ?2, ?3, ?4, ?5) ON CONFLICT(id) DO UPDATE SET payload = excluded.payload, \
             creator_id = excluded.creator_id, updated_at = excluded.updated_at"
        ),
        params![value.id, payload_text, creator_id, created_at, now],
    )
    .map_err(map_rusqlite_error)?;
    Ok(())
}

fn do_patch(
    conn: &Connection,
    collection: &str,
    id: &str,
    patch_json: &[u8],
) -> Result<(), host_store::DataLayerError> {
    validate_identifier(collection)?;
    let existing_payload: String = conn
        .query_row(&format!("SELECT payload FROM {collection} WHERE id = ?1"), params![id], |row| {
            row.get(0)
        })
        .map_err(|e| match e {
            SqliteError::QueryReturnedNoRows => host_store::DataLayerError::SchemaViolation(
                format!("record not found for patch: {id}"),
            ),
            other => map_rusqlite_error(other),
        })?;

    let mut target: Value = serde_json::from_str(&existing_payload).map_err(|e| {
        host_store::DataLayerError::Internal(format!("stored payload is not valid JSON: {e}"))
    })?;
    let patch_text = str::from_utf8(patch_json).map_err(|_| {
        host_store::DataLayerError::SchemaViolation("patch-json must be valid UTF-8".into())
    })?;
    let patch_doc: Value = serde_json::from_str(patch_text).map_err(|e| {
        host_store::DataLayerError::SchemaViolation(format!("patch-json is not valid JSON: {e}"))
    })?;
    apply_merge_patch(&mut target, &patch_doc);
    let merged_text = serde_json::to_string(&target)
        .map_err(|e| host_store::DataLayerError::Internal(e.to_string()))?;

    let now = Utc::now().timestamp_millis();
    conn.execute(
        &format!("UPDATE {collection} SET payload = ?1, updated_at = ?2 WHERE id = ?3"),
        params![merged_text, now, id],
    )
    .map_err(map_rusqlite_error)?;
    Ok(())
}

fn do_delete(
    conn: &Connection,
    collection: &str,
    id: &str,
) -> Result<(), host_store::DataLayerError> {
    validate_identifier(collection)?;
    // Idempotent: deleting a non-existent id is not an error, only a
    // non-existent collection is (surfaced via map_rusqlite_error below).
    conn.execute(&format!("DELETE FROM {collection} WHERE id = ?1"), params![id])
        .map_err(map_rusqlite_error)?;
    Ok(())
}

fn do_delete_many(
    conn: &Connection,
    collection: &str,
    filter_json: Option<&str>,
) -> Result<u64, host_store::DataLayerError> {
    validate_identifier(collection)?;
    let compiled = filter::compile_filter(filter_json)?;
    let (where_sql, bound_params) = match &compiled {
        Some(cf) => (format!("WHERE {}", cf.where_clause), cf.params.clone()),
        None => (String::new(), Vec::new()),
    };
    let affected = conn
        .execute(
            &format!("DELETE FROM {collection} {where_sql}"),
            rusqlite::params_from_iter(bound_params.iter()),
        )
        .map_err(map_rusqlite_error)?;
    Ok(affected as u64)
}

fn do_batch_mutate(
    conn: &mut Connection,
    collection: &str,
    mutations: &[host_store::Mutation],
    creator_id: &str,
) -> Result<(), host_store::DataLayerError> {
    validate_identifier(collection)?;
    if mutations.len() > MAX_BATCH_SIZE {
        return Err(host_store::DataLayerError::SchemaViolation(format!(
            "batch exceeds MAX_BATCH_SIZE ({MAX_BATCH_SIZE})"
        )));
    }
    let tx = conn.transaction().map_err(map_rusqlite_error)?;
    for mutation in mutations {
        match mutation {
            host_store::Mutation::Put(value) => do_put(&tx, collection, value, creator_id)?,
            host_store::Mutation::Patch(patch_mutation) => {
                do_patch(&tx, collection, &patch_mutation.id, &patch_mutation.patch_json)?
            }
            host_store::Mutation::Delete(id) => do_delete(&tx, collection, id)?,
        }
    }
    tx.commit().map_err(map_rusqlite_error)?;
    Ok(())
}

fn do_get(
    conn: &Connection,
    collection: &str,
    id: &str,
) -> Result<Option<host_store::RecordReadValue>, host_store::DataLayerError> {
    validate_identifier(collection)?;
    let result = conn.query_row(
        &format!(
            "SELECT payload, creator_id, created_at, updated_at FROM {collection} WHERE id = ?1"
        ),
        params![id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        },
    );
    match result {
        Ok((payload, creator_id, created_at, updated_at)) => {
            Ok(Some(host_store::RecordReadValue {
                id: id.to_string(),
                payload: payload.into_bytes(),
                creator_id,
                created_at: created_at as u64,
                updated_at: updated_at as u64,
            }))
        }
        Err(SqliteError::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(map_rusqlite_error(e)),
    }
}

fn do_query(
    conn: &Connection,
    collection: &str,
    opts: &host_store::QueryOptions,
) -> Result<host_store::QueryResult, host_store::DataLayerError> {
    validate_identifier(collection)?;
    let compiled = filter::compile_filter(opts.filter.as_deref())?;
    let limit = opts.limit.unwrap_or(MAX_QUERY_PAGE_SIZE).min(MAX_QUERY_PAGE_SIZE);

    let mut where_clauses = Vec::new();
    let mut bound_params: Vec<SqlValue> = Vec::new();
    if let Some(cf) = &compiled {
        where_clauses.push(cf.where_clause.clone());
        bound_params.extend(cf.params.iter().cloned());
    }
    if let Some(cursor) = &opts.cursor {
        where_clauses.push("id > ?".to_string());
        bound_params.push(SqlValue::Text(cursor.clone()));
    }
    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };
    // Fetch one extra row past the page limit to determine whether a
    // next-cursor should be returned.
    bound_params.push(SqlValue::Integer(i64::from(limit) + 1));

    let sql = format!(
        "SELECT id, payload, creator_id, created_at, updated_at FROM {collection} {where_sql} \
         ORDER BY id ASC LIMIT ?"
    );
    let mut stmt = conn.prepare(&sql).map_err(map_rusqlite_error)?;
    let mut records = stmt
        .query_map(rusqlite::params_from_iter(bound_params.iter()), |row| {
            Ok(host_store::RecordReadValue {
                id: row.get::<_, String>(0)?,
                payload: row.get::<_, String>(1)?.into_bytes(),
                creator_id: row.get(2)?,
                created_at: row.get::<_, i64>(3)? as u64,
                updated_at: row.get::<_, i64>(4)? as u64,
            })
        })
        .map_err(map_rusqlite_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_rusqlite_error)?;

    let next_cursor = if records.len() as u32 > limit {
        records.truncate(limit as usize);
        records.last().map(|r| r.id.clone())
    } else {
        None
    };
    Ok(host_store::QueryResult { records, next_cursor })
}

/// Runs an `aggregate` call (ADR-0007, Slice B4) on the reader pool. The
/// compiled SQL is entirely host-generated (bound params + validated
/// identifiers only), so it is `readonly()` by construction and needs
/// neither `do_query_raw`'s authorizer nor its progress handler -- those
/// defend against *arbitrary caller SQL*, which `aggregate` never accepts.
fn do_aggregate(
    conn: &Connection,
    collection: &str,
    pipeline_json: &str,
) -> Result<host_store::RawQueryResult, host_store::DataLayerError> {
    validate_identifier(collection)?;
    let compiled = aggregate::compile(collection, pipeline_json)?;
    run_query_raw(conn, &compiled.sql, &compiled.params)
}

fn wit_to_rusqlite_value(v: &host_store::SqlValue) -> SqlValue {
    match v {
        host_store::SqlValue::Text(s) => SqlValue::Text(s.clone()),
        host_store::SqlValue::Integer(i) => SqlValue::Integer(*i),
        host_store::SqlValue::Real(f) => SqlValue::Real(*f),
        host_store::SqlValue::Boolean(b) => SqlValue::Integer(i64::from(*b)),
        host_store::SqlValue::Null => SqlValue::Null,
    }
}

fn rusqlite_to_wit_value(
    v: rusqlite::types::ValueRef<'_>,
) -> Result<host_store::SqlValue, host_store::DataLayerError> {
    use rusqlite::types::ValueRef;
    Ok(match v {
        ValueRef::Null => host_store::SqlValue::Null,
        ValueRef::Integer(i) => host_store::SqlValue::Integer(i),
        ValueRef::Real(f) => host_store::SqlValue::Real(f),
        ValueRef::Text(bytes) => {
            host_store::SqlValue::Text(String::from_utf8(bytes.to_vec()).map_err(|_| {
                host_store::DataLayerError::SchemaViolation(
                    "query-raw returned non-UTF-8 text".to_string(),
                )
            })?)
        }
        // WIT `sql-value` has no blob arm (ADR-0011): surface, don't corrupt.
        ValueRef::Blob(_) => {
            return Err(host_store::DataLayerError::SchemaViolation(
                "query-raw: BLOB columns are not representable in sql-value; project them via \
                 hex()/base64 instead"
                    .to_string(),
            ));
        }
    })
}

/// Denies connection-configuration/state changes that `Statement::readonly()`
/// does not classify as a write to the database's *content* (SQLite's own
/// docs for `sqlite3_stmt_readonly()` note this gap): `ATTACH`/`DETACH`
/// change which files this connection can read/write -- confirmed
/// empirically, a bare `ATTACH DATABASE '<host path>' AS x` reports
/// `readonly() == true` and creates `<host path>` on the host filesystem as a
/// side effect, which would otherwise let an admin caller escape per-service
/// DB isolation (read another service's file, or write an arbitrary host
/// path). `BEGIN`/a value-setting `PRAGMA` would mutate connection state that
/// leaks onto whichever caller borrows this pooled connection next. None of
/// these has a legitimate use in `query-raw`, a read-only escape hatch
/// scoped to this service's own database (ADR-0011).
fn deny_query_raw_escapes(ctx: rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization {
    use rusqlite::hooks::{AuthAction, Authorization};
    match ctx.action {
        AuthAction::Attach { .. } | AuthAction::Detach { .. } | AuthAction::Transaction { .. } => {
            Authorization::Deny
        }
        AuthAction::Pragma { pragma_value: Some(_), .. } => Authorization::Deny,
        _ => Authorization::Allow,
    }
}

fn map_query_raw_prepare_error(e: rusqlite::Error) -> host_store::DataLayerError {
    if let rusqlite::Error::SqliteFailure(ffi_err, _) = &e
        && ffi_err.code == rusqlite::ErrorCode::AuthorizationForStatementDenied
    {
        return host_store::DataLayerError::PermissionDenied;
    }
    host_store::DataLayerError::SchemaViolation(format!("query-raw prepare failed: {e}"))
}

fn map_query_raw_step_error(e: rusqlite::Error) -> host_store::DataLayerError {
    if let rusqlite::Error::SqliteFailure(ffi_err, _) = &e
        && ffi_err.code == rusqlite::ErrorCode::OperationInterrupted
    {
        return host_store::DataLayerError::QuotaExceeded;
    }
    map_rusqlite_error(e)
}

/// Coarse compute bound (Flag S2, B5 post-commit review): the page cap
/// (`MAX_QUERY_PAGE_SIZE`) only bounds *emitted rows* -- a recursive CTE or
/// an unconstrained cross join can do effectively unbounded work while
/// producing few or no output rows, pinning a reader-pool connection
/// indefinitely. `Connection::progress_handler` interrupts execution after
/// this many virtual-machine instructions regardless of row count. The
/// budget is intentionally generous (legitimate small-per-service-DB
/// queries should never approach it) -- this is a backstop against
/// pathological/runaway statements, not a query-cost optimizer.
const QUERY_RAW_MAX_VM_OPS: i32 = 50_000_000;

/// Clears the authorizer and progress handler on drop -- including on
/// unwind, if `run_query_raw` panics mid-statement -- so this pooled
/// connection's next borrower (`get`/`query`/a future `query-raw` call)
/// never inherits this call's callbacks. `deadpool_sqlite` already discards
/// a connection whose `interact` closure panics rather than returning it to
/// the pool, so the panic path is not reachable in practice today; this
/// guard makes the cleanup correct regardless of that pool behavior, not
/// dependent on it.
struct QueryRawGuard<'c> {
    conn: &'c Connection,
}

impl Drop for QueryRawGuard<'_> {
    fn drop(&mut self) {
        let _ = self
            .conn
            .authorizer::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>(
                None,
            );
        let _ = self.conn.progress_handler(0, None::<fn() -> bool>);
    }
}

/// Executes a privileged read-only raw-SQL query (ADR-0011) on the reader
/// pool. Read-only enforcement (D2 of B5.md) is two-layered: `Statement::
/// readonly()` rejects statements that write the database's content
/// (INSERT/UPDATE/DELETE/DDL/PRAGMA-write), and the authorizer installed
/// below (`deny_query_raw_escapes`) rejects the connection-configuration
/// escapes `readonly()` alone does not cover. Together they ensure the
/// read-write-capable reader connection can never mutate the database or
/// step outside this service's own file. A progress handler
/// (`QUERY_RAW_MAX_VM_OPS`) additionally bounds total compute, independent
/// of the row-count page cap.
fn do_query_raw(
    conn: &Connection,
    sql: &str,
    params: &[host_store::SqlValue],
) -> Result<host_store::RawQueryResult, host_store::DataLayerError> {
    let bound: Vec<SqlValue> = params.iter().map(wit_to_rusqlite_value).collect();

    conn.authorizer(Some(deny_query_raw_escapes)).map_err(map_rusqlite_error)?;
    conn.progress_handler(QUERY_RAW_MAX_VM_OPS, Some(|| true)).map_err(map_rusqlite_error)?;
    let _guard = QueryRawGuard { conn };
    run_query_raw(conn, sql, &bound)
}

fn run_query_raw(
    conn: &Connection,
    sql: &str,
    bound: &[SqlValue],
) -> Result<host_store::RawQueryResult, host_store::DataLayerError> {
    let mut stmt = conn.prepare(sql).map_err(map_query_raw_prepare_error)?;

    if !stmt.readonly() {
        return Err(host_store::DataLayerError::PermissionDenied);
    }

    let column_count = stmt.column_count();
    let columns: Vec<String> = stmt.column_names().into_iter().map(str::to_string).collect();

    // Unlike `query`, there is no cursor for arbitrary raw SQL, so a result
    // exceeding the page cap fails loudly rather than being silently
    // truncated -- the caller must add its own `LIMIT`.
    let mut rows_out: Vec<Vec<host_store::SqlValue>> = Vec::new();
    let mut rows =
        stmt.query(rusqlite::params_from_iter(bound.iter())).map_err(map_query_raw_step_error)?;
    while let Some(row) = rows.next().map_err(map_query_raw_step_error)? {
        if rows_out.len() as u32 >= MAX_QUERY_PAGE_SIZE {
            return Err(host_store::DataLayerError::QuotaExceeded);
        }
        let mut cells = Vec::with_capacity(column_count);
        for i in 0..column_count {
            cells.push(rusqlite_to_wit_value(row.get_ref(i).map_err(map_rusqlite_error)?)?);
        }
        rows_out.push(cells);
    }

    Ok(host_store::RawQueryResult { columns, rows: rows_out })
}

/// SqliteStorageProvider manages the substrate.db (metadata) and per-service
/// encrypted databases.
pub struct SqliteStorageProvider {
    db_dir: PathBuf,
    substrate_conn: Arc<Mutex<Connection>>,
    service_stores: Arc<Mutex<HashMap<String, Arc<SqliteServiceStore>>>>,
    encryption_enabled: bool,
}

impl fmt::Debug for SqliteStorageProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqliteStorageProvider")
            .field("db_dir", &self.db_dir)
            .field("encryption_enabled", &self.encryption_enabled)
            .finish()
    }
}

impl SqliteStorageProvider {
    /// Creates a new `SqliteStorageProvider`. Runs schema migrations on
    /// `substrate.db`.
    pub fn new<P: AsRef<Path>>(db_dir: P, encryption_enabled: bool) -> anyhow::Result<Self> {
        let db_dir = db_dir.as_ref().to_path_buf();

        // Ensure db_dir exists
        if !db_dir.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = DirBuilder::new();
                builder.recursive(true).mode(0o700);
                builder.create(&db_dir)?;
            }
            #[cfg(not(unix))]
            {
                std::fs::create_dir_all(&db_dir)?;
            }
        }

        let substrate_path = db_dir.join("substrate.db");
        let mut conn = Connection::open(&substrate_path)?;

        conn.execute("CREATE TABLE IF NOT EXISTS schema_version (version TEXT NOT NULL)", [])?;

        let stored_version: Option<String> = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| row.get(0))
            .map(Some)
            .or_else(|e| match e {
                SqliteError::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;

        let tx = conn.transaction()?;
        if let Some(version) = stored_version.as_deref()
            && version != SUBSTRATE_SCHEMA_VERSION
        {
            tracing::info!(
                from_version = version,
                to_version = SUBSTRATE_SCHEMA_VERSION,
                "Migrating substrate database schema"
            );
        }
        Self::run_m3a_migration(&tx)?;
        Self::run_m3b_migration(&tx)?;
        let updated =
            tx.execute("UPDATE schema_version SET version = ?1", [SUBSTRATE_SCHEMA_VERSION])?;
        if updated == 0 {
            tx.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [SUBSTRATE_SCHEMA_VERSION],
            )?;
        }
        tx.commit()?;

        Ok(Self {
            db_dir,
            substrate_conn: Arc::new(Mutex::new(conn)),
            service_stores: Arc::new(Mutex::new(HashMap::new())),
            encryption_enabled,
        })
    }

    fn run_m3a_migration(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS dek_store (
                service_id    TEXT PRIMARY KEY,
                encrypted_dek BLOB NOT NULL,
                nonce         BLOB NOT NULL,
                created_at    INTEGER NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS config_generations (
                service_id TEXT NOT NULL,
                generation INTEGER NOT NULL,
                config_blob TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (service_id, generation)
            )",
            [],
        )?;
        Ok(())
    }

    fn run_m3b_migration(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS messaging_subscriptions (
                service_id TEXT NOT NULL,
                topic      TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (service_id, topic)
            )",
            [],
        )?;
        Ok(())
    }

    /// Internal helper to obtain KEK status and print warnings.
    fn verify_encryption_mode(&self, key_store: &Arc<KeyStore>) -> anyhow::Result<()> {
        if self.encryption_enabled {
            if !key_store.kek_is_loaded() {
                tracing::error!("Production encryption is enabled, but no KEK has been injected!");
                return Err(anyhow::anyhow!("EncryptionKeyRequired"));
            }
        } else {
            // Log security warning
            tracing::warn!(
                "INSECURE: storage encryption is disabled. Only use in development profiles."
            );
        }
        Ok(())
    }

    /// Resolves (generating on first use) the DEK for `service_id`.
    /// `Ok(None)` means encryption is disabled -- callers must not treat
    /// this as an error, it's a deliberate per-deployment mode. Does not
    /// call `verify_encryption_mode`; callers that need the
    /// `EncryptionKeyRequired` / insecure-mode-warning checks (e.g.
    /// `open_service_db`) must call it themselves first.
    fn resolve_dek(
        &self,
        service_id: &str,
        key_store: &Arc<KeyStore>,
    ) -> anyhow::Result<Option<[u8; 32]>> {
        if !self.encryption_enabled {
            return Ok(None);
        }
        let conn =
            self.substrate_conn.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
        let dek = match key_store.load_dek(service_id, &conn) {
            Ok(bytes) => bytes,
            Err(KeyStoreError::Database(SqliteError::QueryReturnedNoRows)) => {
                key_store.generate_dek(service_id, &conn)?
            }
            Err(e) => return Err(e.into()),
        };
        Ok(Some(dek))
    }

    /// Validates `service_id` and resolves its expected per-service database
    /// directory, guarding against path traversal. Does not touch the
    /// filesystem.
    fn resolve_service_db_dir(&self, service_id: &str) -> anyhow::Result<PathBuf> {
        if !SERVICE_ID_REGEX.is_match(service_id) {
            return Err(anyhow::anyhow!("Invalid service ID format: {}", service_id));
        }
        let services_dir = self.db_dir.join("services");
        let service_db_dir = services_dir.join(service_id);
        if !service_db_dir.starts_with(&services_dir) {
            return Err(anyhow::anyhow!(
                "Path traversal attempt rejected for service ID: {}",
                service_id
            ));
        }
        Ok(service_db_dir)
    }
}

enum DbCommand {
    WriteSecret {
        key: String,
        secret_bytes: Vec<u8>,
        resp: oneshot::Sender<anyhow::Result<()>>,
    },
    RevealSecret {
        key: String,
        resp: oneshot::Sender<anyhow::Result<Option<Vec<u8>>>>,
    },
    CreateCollection {
        schema: host_store::CollectionSchema,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
    DropCollection {
        name: String,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
    ExecuteDdl {
        sql: String,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
    Put {
        collection: String,
        value: host_store::RecordWriteValue,
        creator_id: String,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
    Patch {
        collection: String,
        id: String,
        patch_json: Vec<u8>,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
    Delete {
        collection: String,
        id: String,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
    DeleteMany {
        collection: String,
        filter: Option<String>,
        resp: oneshot::Sender<Result<u64, host_store::DataLayerError>>,
    },
    BatchMutate {
        collection: String,
        mutations: Vec<host_store::Mutation>,
        creator_id: String,
        resp: oneshot::Sender<Result<(), host_store::DataLayerError>>,
    },
}

fn run_writer_loop(
    mut conn: Connection,
    mut rx: mpsc::Receiver<DbCommand>,
    dek: Zeroizing<[u8; 32]>,
) {
    while let Some(cmd) = rx.blocking_recv() {
        match cmd {
            DbCommand::WriteSecret { key, secret_bytes, resp } => {
                // Encrypt secret bytes with DEK (AES-256-GCM)
                let aes_key = Key::<Aes256Gcm>::from_slice(&*dek);
                let cipher = Aes256Gcm::new(aes_key);

                let mut nonce_bytes = [0u8; 12];
                rand::rng().fill_bytes(&mut nonce_bytes);
                let nonce = Nonce::from_slice(&nonce_bytes);

                let ciphertext_res = cipher
                    .encrypt(nonce, secret_bytes.as_slice())
                    .map_err(|e| anyhow::anyhow!("Encryption failure: {}", e));

                let res = match ciphertext_res {
                    Ok(ciphertext) => {
                        let now = Utc::now().timestamp_millis();
                        conn.execute(
                            "INSERT OR REPLACE INTO _vault (key, ciphertext, nonce, updated_at)
                             VALUES (?1, ?2, ?3, ?4)",
                            rusqlite::params![key, ciphertext, nonce_bytes.as_slice(), now],
                        )
                        .map(|_| ())
                        .map_err(|e| e.into())
                    }
                    Err(e) => Err(e),
                };
                let _ = resp.send(res);
            }
            DbCommand::RevealSecret { key, resp } => {
                let res = (|| -> anyhow::Result<Option<Vec<u8>>> {
                    let mut stmt =
                        conn.prepare("SELECT ciphertext, nonce FROM _vault WHERE key = ?1")?;
                    let mut rows = stmt.query(rusqlite::params![key])?;

                    if let Some(row) = rows.next()? {
                        let ciphertext: Vec<u8> = row.get(0)?;
                        let nonce_bytes: Vec<u8> = row.get(1)?;

                        if nonce_bytes.len() != 12 {
                            return Err(anyhow::anyhow!("Invalid stored nonce length"));
                        }

                        // Decrypt
                        let aes_key = Key::<Aes256Gcm>::from_slice(&*dek);
                        let cipher = Aes256Gcm::new(aes_key);
                        let nonce = Nonce::from_slice(&nonce_bytes);

                        let decrypted = cipher
                            .decrypt(nonce, ciphertext.as_slice())
                            .map_err(|e| anyhow::anyhow!("Decryption failure: {}", e))?;

                        Ok(Some(decrypted))
                    } else {
                        Ok(None)
                    }
                })();
                let _ = resp.send(res);
            }
            DbCommand::CreateCollection { schema, resp } => {
                let _ = resp.send(do_create_collection(&conn, &schema));
            }
            DbCommand::DropCollection { name, resp } => {
                let _ = resp.send(do_drop_collection(&conn, &name));
            }
            DbCommand::ExecuteDdl { sql, resp } => {
                let _ = resp.send(do_execute_ddl(&conn, &sql));
            }
            DbCommand::Put { collection, value, creator_id, resp } => {
                let _ = resp.send(do_put(&conn, &collection, &value, &creator_id));
            }
            DbCommand::Patch { collection, id, patch_json, resp } => {
                let _ = resp.send(do_patch(&conn, &collection, &id, &patch_json));
            }
            DbCommand::Delete { collection, id, resp } => {
                let _ = resp.send(do_delete(&conn, &collection, &id));
            }
            DbCommand::DeleteMany { collection, filter, resp } => {
                let _ = resp.send(do_delete_many(&conn, &collection, filter.as_deref()));
            }
            DbCommand::BatchMutate { collection, mutations, creator_id, resp } => {
                let _ = resp.send(do_batch_mutate(&mut conn, &collection, &mutations, &creator_id));
            }
        }
    }
}

#[async_trait]
impl StorageProvider for SqliteStorageProvider {
    async fn open_service_db(
        &self,
        service_id: &str,
        key_store: &Arc<KeyStore>,
    ) -> anyhow::Result<Box<dyn ServiceStore>> {
        let service_db_dir = self.resolve_service_db_dir(service_id)?;

        // Verify encryption requirements
        self.verify_encryption_mode(key_store)?;

        if let Some(store) = self
            .service_stores
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?
            .get(service_id)
            .cloned()
        {
            return Ok(Box::new(store));
        }

        // Ensure service_db_dir exists
        if !service_db_dir.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = DirBuilder::new();
                builder.recursive(true).mode(0o700);
                builder.create(&service_db_dir)?;
            }
            #[cfg(not(unix))]
            {
                std::fs::create_dir_all(&service_db_dir)?;
            }
        }

        let db_file_path = service_db_dir.join("state.db");

        // Resolve or generate DEK. `resolve_dek` returns `None` when
        // encryption is disabled; SQLCipher still needs *a* key to pragma,
        // so a dummy all-zero key is substituted here specifically (this
        // dummy-key behavior is local to the SQLite/SQLCipher path -- the
        // public `load_service_dek` trait method returns `None` as-is to
        // its callers instead of this substitution).
        let dek = Zeroizing::new(self.resolve_dek(service_id, key_store)?.unwrap_or([0u8; 32]));

        // Open service DB connection for the single writer actor
        let writer_conn = Connection::open(&db_file_path)?;
        if self.encryption_enabled {
            let pragma_val = Zeroizing::new(format!("x'{}'", hex::encode(*dek)));
            writer_conn.pragma_update(None, "key", &*pragma_val)?;
        }

        // Initialize vault table
        writer_conn.execute(
            "CREATE TABLE IF NOT EXISTS _vault (
                key        TEXT PRIMARY KEY,
                ciphertext BLOB NOT NULL,
                nonce      BLOB NOT NULL,
                updated_at INTEGER NOT NULL
            )",
            [],
        )?;

        // Start single-writer background loop
        let (writer_tx, writer_rx) = mpsc::channel(100);
        let dek_writer = dek.clone();
        task::spawn_blocking(move || {
            run_writer_loop(writer_conn, writer_rx, dek_writer);
        });

        // Initialize reader pool, used for all read-only operations (vault
        // reveal reads stay on the writer above for simplicity; CRUD reads
        // use this pool for concurrency).
        let cfg = PoolConfig::new(&db_file_path);
        let mut reader_pool_builder = cfg.builder(Runtime::Tokio1)?;
        if self.encryption_enabled {
            let reader_dek = dek.clone();
            reader_pool_builder =
                reader_pool_builder.post_create(Hook::async_fn(move |conn, _metrics| {
                    let reader_dek = reader_dek.clone();
                    Box::pin(async move {
                        let pragma_val = Zeroizing::new(format!("x'{}'", hex::encode(*reader_dek)));
                        conn.interact(move |conn| conn.pragma_update(None, "key", &*pragma_val))
                            .await
                            .map_err(|e| HookError::message(format!("Interact error: {}", e)))?
                            .map_err(HookError::Backend)?;
                        Ok(())
                    })
                }));
        }
        let reader_pool = reader_pool_builder.build()?;

        let new_store = Arc::new(SqliteServiceStore { reader_pool, writer_tx });
        let store = {
            let mut stores =
                self.service_stores.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
            stores.entry(service_id.to_string()).or_insert_with(|| new_store.clone()).clone()
        };

        Ok(Box::new(store))
    }

    async fn rotate_kek(&self, key_store: &Arc<KeyStore>, new_kek: [u8; 32]) -> anyhow::Result<()> {
        let conn_arc = self.substrate_conn.clone();
        let ks = key_store.clone();
        task::spawn_blocking(move || -> anyhow::Result<()> {
            let mut conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
            ks.rotate_kek(new_kek, &mut conn)?;
            Ok(())
        })
        .await?
    }

    async fn service_exists(&self, service_id: &str) -> anyhow::Result<bool> {
        let service_db_dir = self.resolve_service_db_dir(service_id)?;
        Ok(service_db_dir.join("state.db").exists())
    }

    async fn load_service_dek(
        &self,
        service_id: &str,
        key_store: &Arc<KeyStore>,
    ) -> anyhow::Result<Option<Zeroizing<[u8; 32]>>> {
        self.verify_encryption_mode(key_store)?;
        Ok(self.resolve_dek(service_id, key_store)?.map(Zeroizing::new))
    }

    async fn save_config_generation(
        &self,
        service_id: &str,
        config_blob: &str,
    ) -> anyhow::Result<u64> {
        let conn_arc = self.substrate_conn.clone();
        let s_id = service_id.to_string();
        let blob = config_blob.to_string();
        task::spawn_blocking(move || -> anyhow::Result<u64> {
            let mut conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
            let tx = conn.transaction()?;

            let current_gen: Option<i64> = tx
                .query_row(
                    "SELECT MAX(generation) FROM config_generations WHERE service_id = ?1",
                    params![s_id],
                    |row| row.get(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    SqliteError::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?
                .flatten();

            let next_gen = current_gen.unwrap_or(0) + 1;
            let now = Utc::now().timestamp_millis();

            tx.execute(
                "INSERT INTO config_generations (service_id, generation, config_blob, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![s_id, next_gen, blob, now],
            )?;

            tx.commit()?;
            Ok(next_gen as u64)
        })
        .await?
    }

    async fn delete_config_generation(
        &self,
        service_id: &str,
        generation: u64,
    ) -> anyhow::Result<()> {
        let conn_arc = self.substrate_conn.clone();
        let s_id = service_id.to_string();
        task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
            conn.execute(
                "DELETE FROM config_generations WHERE service_id = ?1 AND generation = ?2",
                params![s_id, generation as i64],
            )?;
            Ok(())
        })
        .await?
    }

    async fn get_config_generation(
        &self,
        service_id: &str,
        generation: u64,
    ) -> anyhow::Result<Option<String>> {
        let conn_arc = self.substrate_conn.clone();
        let s_id = service_id.to_string();
        task::spawn_blocking(move || -> anyhow::Result<Option<String>> {
            let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
            let mut stmt = conn.prepare(
                "SELECT config_blob FROM config_generations WHERE service_id = ?1 AND generation \
                 = ?2",
            )?;
            let mut rows = stmt.query(params![s_id, generation as i64])?;

            if let Some(row) = rows.next()? { Ok(Some(row.get(0)?)) } else { Ok(None) }
        })
        .await?
    }

    async fn get_latest_config_generation(
        &self,
        service_id: &str,
    ) -> anyhow::Result<Option<(u64, String)>> {
        let conn_arc = self.substrate_conn.clone();
        let s_id = service_id.to_string();
        task::spawn_blocking(move || -> anyhow::Result<Option<(u64, String)>> {
            let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
            let mut stmt = conn.prepare(
                "SELECT generation, config_blob FROM config_generations WHERE service_id = ?1 \
                 ORDER BY generation DESC LIMIT 1",
            )?;
            let mut rows = stmt.query(params![s_id])?;

            if let Some(row) = rows.next()? {
                Ok(Some((row.get::<_, i64>(0)? as u64, row.get(1)?)))
            } else {
                Ok(None)
            }
        })
        .await?
    }

    async fn save_messaging_subscription(
        &self,
        service_id: &str,
        topic: &str,
    ) -> anyhow::Result<()> {
        let conn_arc = self.substrate_conn.clone();
        let s_id = service_id.to_string();
        let topic = topic.to_string();
        task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
            let now = Utc::now().timestamp_millis();
            conn.execute(
                "INSERT INTO messaging_subscriptions (service_id, topic, created_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (service_id, topic) DO NOTHING",
                params![s_id, topic, now],
            )?;
            Ok(())
        })
        .await?
    }

    async fn delete_messaging_subscription(
        &self,
        service_id: &str,
        topic: &str,
    ) -> anyhow::Result<()> {
        let conn_arc = self.substrate_conn.clone();
        let s_id = service_id.to_string();
        let topic = topic.to_string();
        task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
            conn.execute(
                "DELETE FROM messaging_subscriptions WHERE service_id = ?1 AND topic = ?2",
                params![s_id, topic],
            )?;
            Ok(())
        })
        .await?
    }

    async fn delete_all_messaging_subscriptions_for_service(
        &self,
        service_id: &str,
    ) -> anyhow::Result<()> {
        let conn_arc = self.substrate_conn.clone();
        let s_id = service_id.to_string();
        task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
            conn.execute(
                "DELETE FROM messaging_subscriptions WHERE service_id = ?1",
                params![s_id],
            )?;
            Ok(())
        })
        .await?
    }

    async fn list_all_messaging_subscriptions(&self) -> anyhow::Result<Vec<(String, String)>> {
        let conn_arc = self.substrate_conn.clone();
        task::spawn_blocking(move || -> anyhow::Result<Vec<(String, String)>> {
            let conn = conn_arc.lock().map_err(|e| anyhow::anyhow!("Mutex poisoned: {}", e))?;
            let mut stmt = conn.prepare("SELECT service_id, topic FROM messaging_subscriptions")?;
            let rows = stmt
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?
    }
}

pub struct SqliteServiceStore {
    reader_pool: Pool,
    writer_tx: mpsc::Sender<DbCommand>,
}

impl fmt::Debug for SqliteServiceStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqliteServiceStore").field("dek", &"<redacted>").finish()
    }
}

/// Sends a write command over the single-writer channel and awaits its
/// response, flattening channel-disconnect failures into a `DataLayerError`.
async fn send_write_command<T>(
    writer_tx: &mpsc::Sender<DbCommand>,
    build: impl FnOnce(oneshot::Sender<Result<T, host_store::DataLayerError>>) -> DbCommand,
) -> Result<T, host_store::DataLayerError> {
    let (resp_tx, resp_rx) = oneshot::channel();
    writer_tx.send(build(resp_tx)).await.map_err(|_| {
        host_store::DataLayerError::Internal("writer task disconnected".to_string())
    })?;
    resp_rx
        .await
        .map_err(|_| host_store::DataLayerError::Internal("writer task disconnected".to_string()))?
}

#[async_trait]
impl ServiceStore for SqliteServiceStore {
    async fn write_secret(&self, key: &str, secret_bytes: &[u8]) -> anyhow::Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.writer_tx
            .send(DbCommand::WriteSecret {
                key: key.to_string(),
                secret_bytes: secret_bytes.to_vec(),
                resp: resp_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("Writer task disconnected"))?;
        resp_rx.await?
    }

    async fn reveal_secret(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.writer_tx
            .send(DbCommand::RevealSecret { key: key.to_string(), resp: resp_tx })
            .await
            .map_err(|_| anyhow::anyhow!("Writer task disconnected"))?;
        resp_rx.await?
    }

    async fn create_collection(
        &self,
        schema: &host_store::CollectionSchema,
    ) -> Result<(), host_store::DataLayerError> {
        let schema = schema.clone();
        send_write_command(&self.writer_tx, |resp| DbCommand::CreateCollection { schema, resp })
            .await
    }

    async fn drop_collection(&self, name: &str) -> Result<(), host_store::DataLayerError> {
        let name = name.to_string();
        send_write_command(&self.writer_tx, |resp| DbCommand::DropCollection { name, resp }).await
    }

    async fn execute_ddl(&self, sql: &str) -> Result<(), host_store::DataLayerError> {
        let sql = sql.to_string();
        send_write_command(&self.writer_tx, |resp| DbCommand::ExecuteDdl { sql, resp }).await
    }

    async fn put(
        &self,
        collection: &str,
        value: &host_store::RecordWriteValue,
        creator_id: &str,
    ) -> Result<(), host_store::DataLayerError> {
        let collection = collection.to_string();
        let value = value.clone();
        let creator_id = creator_id.to_string();
        send_write_command(&self.writer_tx, |resp| DbCommand::Put {
            collection,
            value,
            creator_id,
            resp,
        })
        .await
    }

    async fn patch(
        &self,
        collection: &str,
        id: &str,
        patch_json: &[u8],
    ) -> Result<(), host_store::DataLayerError> {
        let collection = collection.to_string();
        let id = id.to_string();
        let patch_json = patch_json.to_vec();
        send_write_command(&self.writer_tx, |resp| DbCommand::Patch {
            collection,
            id,
            patch_json,
            resp,
        })
        .await
    }

    async fn get(
        &self,
        collection: &str,
        id: &str,
    ) -> Result<Option<host_store::RecordReadValue>, host_store::DataLayerError> {
        let collection = collection.to_string();
        let id = id.to_string();
        let conn = self
            .reader_pool
            .get()
            .await
            .map_err(|e| host_store::DataLayerError::Internal(format!("reader pool: {e}")))?;
        conn.interact(move |conn| do_get(conn, &collection, &id)).await.map_err(|e| {
            host_store::DataLayerError::Internal(format!("reader pool interact: {e}"))
        })?
    }

    async fn query(
        &self,
        collection: &str,
        opts: &host_store::QueryOptions,
    ) -> Result<host_store::QueryResult, host_store::DataLayerError> {
        let collection = collection.to_string();
        let opts = opts.clone();
        let conn = self
            .reader_pool
            .get()
            .await
            .map_err(|e| host_store::DataLayerError::Internal(format!("reader pool: {e}")))?;
        conn.interact(move |conn| do_query(conn, &collection, &opts)).await.map_err(|e| {
            host_store::DataLayerError::Internal(format!("reader pool interact: {e}"))
        })?
    }

    async fn aggregate(
        &self,
        collection: &str,
        pipeline: &str,
    ) -> Result<host_store::RawQueryResult, host_store::DataLayerError> {
        let collection = collection.to_string();
        let pipeline = pipeline.to_string();
        let conn = self
            .reader_pool
            .get()
            .await
            .map_err(|e| host_store::DataLayerError::Internal(format!("reader pool: {e}")))?;
        conn.interact(move |conn| do_aggregate(conn, &collection, &pipeline)).await.map_err(
            |e| host_store::DataLayerError::Internal(format!("reader pool interact: {e}")),
        )?
    }

    async fn delete(&self, collection: &str, id: &str) -> Result<(), host_store::DataLayerError> {
        let collection = collection.to_string();
        let id = id.to_string();
        send_write_command(&self.writer_tx, |resp| DbCommand::Delete { collection, id, resp }).await
    }

    async fn delete_many(
        &self,
        collection: &str,
        filter: Option<&str>,
    ) -> Result<u64, host_store::DataLayerError> {
        let collection = collection.to_string();
        let filter = filter.map(str::to_string);
        send_write_command(&self.writer_tx, |resp| DbCommand::DeleteMany {
            collection,
            filter,
            resp,
        })
        .await
    }

    async fn batch_mutate(
        &self,
        collection: &str,
        mutations: &[host_store::Mutation],
        creator_id: &str,
    ) -> Result<(), host_store::DataLayerError> {
        let collection = collection.to_string();
        let mutations = mutations.to_vec();
        let creator_id = creator_id.to_string();
        send_write_command(&self.writer_tx, |resp| DbCommand::BatchMutate {
            collection,
            mutations,
            creator_id,
            resp,
        })
        .await
    }

    async fn query_raw(
        &self,
        sql: &str,
        params: &[host_store::SqlValue],
    ) -> Result<host_store::RawQueryResult, host_store::DataLayerError> {
        let sql = sql.to_string();
        let params = params.to_vec();
        let conn = self
            .reader_pool
            .get()
            .await
            .map_err(|e| host_store::DataLayerError::Internal(format!("reader pool: {e}")))?;
        conn.interact(move |conn| do_query_raw(conn, &sql, &params)).await.map_err(|e| {
            host_store::DataLayerError::Internal(format!("reader pool interact: {e}"))
        })?
    }
}

#[async_trait]
impl ServiceStore for Arc<SqliteServiceStore> {
    async fn write_secret(&self, key: &str, secret_bytes: &[u8]) -> anyhow::Result<()> {
        self.as_ref().write_secret(key, secret_bytes).await
    }

    async fn reveal_secret(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        self.as_ref().reveal_secret(key).await
    }

    async fn create_collection(
        &self,
        schema: &host_store::CollectionSchema,
    ) -> Result<(), host_store::DataLayerError> {
        self.as_ref().create_collection(schema).await
    }

    async fn drop_collection(&self, name: &str) -> Result<(), host_store::DataLayerError> {
        self.as_ref().drop_collection(name).await
    }

    async fn execute_ddl(&self, sql: &str) -> Result<(), host_store::DataLayerError> {
        self.as_ref().execute_ddl(sql).await
    }

    async fn put(
        &self,
        collection: &str,
        value: &host_store::RecordWriteValue,
        creator_id: &str,
    ) -> Result<(), host_store::DataLayerError> {
        self.as_ref().put(collection, value, creator_id).await
    }

    async fn patch(
        &self,
        collection: &str,
        id: &str,
        patch_json: &[u8],
    ) -> Result<(), host_store::DataLayerError> {
        self.as_ref().patch(collection, id, patch_json).await
    }

    async fn get(
        &self,
        collection: &str,
        id: &str,
    ) -> Result<Option<host_store::RecordReadValue>, host_store::DataLayerError> {
        self.as_ref().get(collection, id).await
    }

    async fn query(
        &self,
        collection: &str,
        opts: &host_store::QueryOptions,
    ) -> Result<host_store::QueryResult, host_store::DataLayerError> {
        self.as_ref().query(collection, opts).await
    }

    async fn aggregate(
        &self,
        collection: &str,
        pipeline: &str,
    ) -> Result<host_store::RawQueryResult, host_store::DataLayerError> {
        self.as_ref().aggregate(collection, pipeline).await
    }

    async fn delete(&self, collection: &str, id: &str) -> Result<(), host_store::DataLayerError> {
        self.as_ref().delete(collection, id).await
    }

    async fn delete_many(
        &self,
        collection: &str,
        filter: Option<&str>,
    ) -> Result<u64, host_store::DataLayerError> {
        self.as_ref().delete_many(collection, filter).await
    }

    async fn batch_mutate(
        &self,
        collection: &str,
        mutations: &[host_store::Mutation],
        creator_id: &str,
    ) -> Result<(), host_store::DataLayerError> {
        self.as_ref().batch_mutate(collection, mutations, creator_id).await
    }

    async fn query_raw(
        &self,
        sql: &str,
        params: &[host_store::SqlValue],
    ) -> Result<host_store::RawQueryResult, host_store::DataLayerError> {
        self.as_ref().query_raw(sql, params).await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn test_startup_migrates_previous_substrate_schema_to_m3a() {
        let dir = tempdir().unwrap();
        let substrate_path = dir.path().join("substrate.db");
        {
            let conn = Connection::open(&substrate_path).unwrap();
            conn.execute(
                "CREATE TABLE schema_version (
                    id INTEGER PRIMARY KEY CHECK (id = 1),
                    version TEXT NOT NULL
                )",
                [],
            )
            .unwrap();
            conn.execute("INSERT INTO schema_version (id, version) VALUES (1, 'm2')", []).unwrap();
        }

        let _provider = SqliteStorageProvider::new(dir.path(), false).unwrap();

        let conn = Connection::open(&substrate_path).unwrap();
        let version: String = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SUBSTRATE_SCHEMA_VERSION);

        let dek_store_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'dek_store'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(dek_store_count, 1);
    }

    #[test]
    fn test_startup_uses_documented_schema_version_shape() {
        let dir = tempdir().unwrap();
        let substrate_path = dir.path().join("substrate.db");

        let _provider = SqliteStorageProvider::new(dir.path(), false).unwrap();

        let conn = Connection::open(&substrate_path).unwrap();
        let columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(schema_version)").unwrap();
            stmt.query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };

        assert_eq!(columns, vec!["version"]);
    }

    #[tokio::test]
    async fn test_service_id_validation_and_path_traversal() {
        let dir = tempdir().unwrap();
        let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
        let key_store = Arc::new(KeyStore::new());

        // Valid ID
        assert!(provider.open_service_db("svc-1", &key_store).await.is_ok());
        assert!(provider.open_service_db("my_service-2", &key_store).await.is_ok());
        // Real service ids are DIDs and contain colons.
        assert!(
            provider
                .open_service_db(
                    "did:key:h7wy4ppo5gystkfs71hf19qhmbaqc3yx7gpcbtg4s9h6ojozbgx61nco",
                    &key_store
                )
                .await
                .is_ok()
        );

        // Invalid IDs
        assert!(provider.open_service_db("svc/../../traversal", &key_store).await.is_err());
        assert!(provider.open_service_db("did:key:../../traversal", &key_store).await.is_err());
        assert!(provider.open_service_db("svc_with_spaces ", &key_store).await.is_err());
        assert!(provider.open_service_db("svc!", &key_store).await.is_err());
        assert!(provider.open_service_db("", &key_store).await.is_err());
    }

    #[tokio::test]
    async fn test_encryption_key_required() {
        let dir = tempdir().unwrap();
        // Encryption enabled
        let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
        let key_store = Arc::new(KeyStore::new());

        // Fails with EncryptionKeyRequired because KEK is not injected
        let res = provider.open_service_db("my-service", &key_store).await;
        assert!(res.is_err());
        assert_eq!(res.err().unwrap().to_string(), "EncryptionKeyRequired");

        // Inject KEK
        key_store.inject_kek([9u8; 32], None).unwrap();

        // Now succeeds
        assert!(provider.open_service_db("my-service", &key_store).await.is_ok());
    }

    #[tokio::test]
    async fn test_load_service_dek_none_when_encryption_disabled() {
        let dir = tempdir().unwrap();
        let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
        let key_store = Arc::new(KeyStore::new());
        let dek = provider.load_service_dek("svc-a", &key_store).await.unwrap();
        assert_eq!(dek, None);
    }

    #[tokio::test]
    async fn test_load_service_dek_requires_kek_when_encryption_enabled() {
        let dir = tempdir().unwrap();
        let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
        let key_store = Arc::new(KeyStore::new());
        let res = provider.load_service_dek("svc-a", &key_store).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_load_service_dek_generates_then_reuses_same_dek() {
        let dir = tempdir().unwrap();
        let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
        let key_store = Arc::new(KeyStore::new());
        key_store.inject_kek([21u8; 32], None).unwrap();

        let dek_a = provider.load_service_dek("svc-a", &key_store).await.unwrap();
        let dek_b = provider.load_service_dek("svc-a", &key_store).await.unwrap();
        assert!(dek_a.is_some());
        assert_eq!(dek_a, dek_b);
    }

    #[tokio::test]
    async fn test_load_service_dek_matches_open_service_db_dek() {
        // Regression guard for the open_service_db refactor: both paths
        // must resolve to the identical DEK for the same service_id, since
        // open_service_db's SQLCipher pragma and load_service_dek's callers
        // (e.g. blob-store) must agree on the same key material.
        let dir = tempdir().unwrap();
        let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
        let key_store = Arc::new(KeyStore::new());
        key_store.inject_kek([33u8; 32], None).unwrap();

        // Opening the service DB first generates the DEK as a side effect.
        let _ = provider.open_service_db("svc-shared", &key_store).await.unwrap();
        let via_load = provider.load_service_dek("svc-shared", &key_store).await.unwrap();
        assert!(via_load.is_some());

        // A second provider instance sharing the same substrate.db must
        // resolve the identical DEK (survives "restart").
        let provider2 = SqliteStorageProvider::new(dir.path(), true).unwrap();
        let via_load2 = provider2.load_service_dek("svc-shared", &key_store).await.unwrap();
        assert_eq!(via_load, via_load2);
    }

    #[tokio::test]
    async fn test_vault_write_and_reveal() {
        let dir = tempdir().unwrap();
        let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
        let key_store = Arc::new(KeyStore::new());
        key_store.inject_kek([15u8; 32], None).unwrap();

        let store = provider.open_service_db("vault-test", &key_store).await.unwrap();

        // Reveal missing key returns None
        assert_eq!(store.reveal_secret("api_key").await.unwrap(), None);

        // Write secret
        let secret = b"super-secret-token-123";
        store.write_secret("api_key", secret).await.unwrap();

        // Reveal secret
        let revealed = store.reveal_secret("api_key").await.unwrap();
        assert_eq!(revealed, Some(secret.to_vec()));
    }

    #[tokio::test]
    async fn test_open_service_db_reuses_cached_store_actor() {
        let dir = tempdir().unwrap();
        let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
        let key_store = Arc::new(KeyStore::new());
        key_store.inject_kek([17u8; 32], None).unwrap();

        let first = provider.open_service_db("cached-vault", &key_store).await.unwrap();
        let second = provider.open_service_db("cached-vault", &key_store).await.unwrap();

        first.write_secret("api_key", b"cached-store-secret").await.unwrap();
        let revealed = second.reveal_secret("api_key").await.unwrap();

        assert_eq!(revealed, Some(b"cached-store-secret".to_vec()));
        assert_eq!(provider.service_stores.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_path_traversal_etc_passwd() {
        let dir = tempdir().unwrap();
        let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
        let key_store = Arc::new(KeyStore::new());

        // Explicitly assert path traversal reject
        assert!(provider.open_service_db("../../etc/passwd", &key_store).await.is_err());
    }

    #[tokio::test]
    async fn test_restart_survival() {
        let dir = tempdir().unwrap();
        let key_store = Arc::new(KeyStore::new());
        key_store.inject_kek([42u8; 32], None).unwrap();

        // Write data on first boot
        {
            let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
            let store = provider.open_service_db("restart-test", &key_store).await.unwrap();
            store.write_secret("secret_key", b"survival-data-100").await.unwrap();
        }

        // Read data after "restart" (re-instantiating SqliteStorageProvider)
        {
            let provider = SqliteStorageProvider::new(dir.path(), true).unwrap();
            let store = provider.open_service_db("restart-test", &key_store).await.unwrap();
            let revealed = store.reveal_secret("secret_key").await.unwrap();
            assert_eq!(revealed, Some(b"survival-data-100".to_vec()));
        }
    }

    #[tokio::test]
    async fn test_messaging_subscriptions_roundtrip_and_restart_survival() {
        let dir = tempdir().unwrap();

        {
            let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
            provider.save_messaging_subscription("svc-a", "svc/svc-a/orders/new").await.unwrap();
            provider.save_messaging_subscription("svc-a", "sensors/+/temp").await.unwrap();
            provider.save_messaging_subscription("svc-b", "svc/svc-b/status").await.unwrap();
            // Re-subscribing to the same topic is idempotent, not an error.
            provider.save_messaging_subscription("svc-a", "sensors/+/temp").await.unwrap();

            let mut all = provider.list_all_messaging_subscriptions().await.unwrap();
            all.sort();
            assert_eq!(
                all,
                vec![
                    ("svc-a".to_string(), "sensors/+/temp".to_string()),
                    ("svc-a".to_string(), "svc/svc-a/orders/new".to_string()),
                    ("svc-b".to_string(), "svc/svc-b/status".to_string()),
                ]
            );

            provider.delete_messaging_subscription("svc-a", "sensors/+/temp").await.unwrap();
            let mut after_delete = provider.list_all_messaging_subscriptions().await.unwrap();
            after_delete.sort();
            assert_eq!(
                after_delete,
                vec![
                    ("svc-a".to_string(), "svc/svc-a/orders/new".to_string()),
                    ("svc-b".to_string(), "svc/svc-b/status".to_string()),
                ]
            );

            provider.delete_all_messaging_subscriptions_for_service("svc-a").await.unwrap();
            let after_undeploy = provider.list_all_messaging_subscriptions().await.unwrap();
            assert_eq!(after_undeploy, vec![("svc-b".to_string(), "svc/svc-b/status".to_string())]);
        }

        // Surviving row is still there after "restart" (re-opening the same db_dir).
        {
            let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
            let all = provider.list_all_messaging_subscriptions().await.unwrap();
            assert_eq!(all, vec![("svc-b".to_string(), "svc/svc-b/status".to_string())]);
        }
    }

    #[test]
    fn test_insecure_mode_warning() {
        use std::io;

        use tracing_subscriber::prelude::*;

        let logs = Arc::new(Mutex::new(Vec::new()));
        let logs_clone = logs.clone();

        struct MockWriter {
            logs: Arc<Mutex<Vec<u8>>>,
        }
        impl io::Write for MockWriter {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.logs.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let make_writer = move || MockWriter { logs: logs_clone.clone() };
        let layer = tracing_subscriber::fmt::layer().with_writer(make_writer);

        let dir = tempdir().unwrap();
        let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
        let key_store = Arc::new(KeyStore::new());
        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            provider.verify_encryption_mode(&key_store).unwrap();
        });

        let logs_content = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
        assert!(logs_content.contains("INSECURE: storage encryption is disabled"));
    }

    #[tokio::test]
    async fn test_service_exists_reflects_persistent_db_state() {
        let dir = tempdir().unwrap();
        let provider = SqliteStorageProvider::new(dir.path(), false).unwrap();
        let key_store = Arc::new(KeyStore::new());

        assert!(!provider.service_exists("svc-a").await.unwrap());
        let _ = provider.open_service_db("svc-a", &key_store).await.unwrap();
        assert!(provider.service_exists("svc-a").await.unwrap());
    }
}
