use rusqlite::{Connection, types::Value as SqlValue};

use super::MAX_QUERY_PAGE_SIZE;
use crate::{errors::map_rusqlite_error, host_store};

pub(crate) fn wit_to_rusqlite_value(v: &host_store::SqlValue) -> SqlValue {
    match v {
        host_store::SqlValue::Text(s) => SqlValue::Text(s.clone()),
        host_store::SqlValue::Integer(i) => SqlValue::Integer(*i),
        host_store::SqlValue::Real(f) => SqlValue::Real(*f),
        host_store::SqlValue::Boolean(b) => SqlValue::Integer(i64::from(*b)),
        host_store::SqlValue::Null => SqlValue::Null,
    }
}

pub(crate) fn rusqlite_to_wit_value(
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
pub(crate) fn deny_query_raw_escapes(
    ctx: rusqlite::hooks::AuthContext<'_>,
) -> rusqlite::hooks::Authorization {
    use rusqlite::hooks::{AuthAction, Authorization};
    match ctx.action {
        AuthAction::Attach { .. } | AuthAction::Detach { .. } | AuthAction::Transaction { .. } => {
            Authorization::Deny
        }
        AuthAction::Pragma { pragma_value: Some(_), .. } => Authorization::Deny,
        _ => Authorization::Allow,
    }
}

/// `op` names the caller-facing operation (`"query-raw"` or `"aggregate"`)
/// so a prepare failure -- e.g. "no such table" for an `aggregate` over a
/// missing collection -- doesn't misattribute itself to the other, shared
/// `run_query_raw` caller.
pub(crate) fn map_sql_prepare_error(op: &str, e: rusqlite::Error) -> host_store::DataLayerError {
    if let rusqlite::Error::SqliteFailure(ffi_err, _) = &e
        && ffi_err.code == rusqlite::ErrorCode::AuthorizationForStatementDenied
    {
        return host_store::DataLayerError::PermissionDenied;
    }
    host_store::DataLayerError::SchemaViolation(format!("{op} prepare failed: {e}"))
}

pub(crate) fn is_operation_interrupted(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(ffi_err, _) if ffi_err.code == rusqlite::ErrorCode::OperationInterrupted)
}

pub(crate) fn map_query_raw_step_error(e: rusqlite::Error) -> host_store::DataLayerError {
    if is_operation_interrupted(&e) {
        return host_store::DataLayerError::QuotaExceeded;
    }
    map_rusqlite_error(e)
}

/// Coarse compute bound: the page cap
/// (`MAX_QUERY_PAGE_SIZE`) only bounds *emitted rows* -- a recursive CTE or
/// an unconstrained cross join can do effectively unbounded work while
/// producing few or no output rows, pinning a reader-pool connection
/// indefinitely. `Connection::progress_handler` interrupts execution after
/// this many virtual-machine instructions regardless of row count. The
/// budget is intentionally generous (legitimate small-per-service-DB
/// queries should never approach it) -- this is a backstop against
/// pathological/runaway statements, not a query-cost optimizer.
pub(crate) const QUERY_RAW_MAX_VM_OPS: i32 = 50_000_000;

/// Clears the authorizer and progress handler on drop -- including on
/// unwind, if `run_query_raw` panics mid-statement -- so this pooled
/// connection's next borrower (`get`/`query`/a future `query-raw` call)
/// never inherits this call's callbacks. `deadpool_sqlite` already discards
/// a connection whose `interact` closure panics rather than returning it to
/// the pool, so the panic path is not reachable in practice today; this
/// guard makes the cleanup correct regardless of that pool behavior, not
/// dependent on it.
pub(crate) struct QueryRawGuard<'c> {
    pub(crate) conn: &'c Connection,
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
/// pool. Read-only enforcement is two-layered: `Statement::
/// readonly()` rejects statements that write the database's content
/// (INSERT/UPDATE/DELETE/DDL/PRAGMA-write), and the authorizer installed
/// below (`deny_query_raw_escapes`) rejects the connection-configuration
/// escapes `readonly()` alone does not cover. Together they ensure the
/// read-write-capable reader connection can never mutate the database or
/// step outside this service's own file. A progress handler
/// (`QUERY_RAW_MAX_VM_OPS`) additionally bounds total compute, independent
/// of the row-count page cap.
pub(crate) fn do_query_raw(
    conn: &Connection,
    sql: &str,
    params: &[host_store::SqlValue],
) -> Result<host_store::RawQueryResult, host_store::DataLayerError> {
    let bound: Vec<SqlValue> = params.iter().map(wit_to_rusqlite_value).collect();

    conn.authorizer(Some(deny_query_raw_escapes)).map_err(map_rusqlite_error)?;
    conn.progress_handler(QUERY_RAW_MAX_VM_OPS, Some(|| true)).map_err(map_rusqlite_error)?;
    let _guard = QueryRawGuard { conn };
    run_query_raw(conn, "query-raw", sql, &bound)
}

pub(crate) fn run_query_raw(
    conn: &Connection,
    op: &str,
    sql: &str,
    bound: &[SqlValue],
) -> Result<host_store::RawQueryResult, host_store::DataLayerError> {
    let mut stmt = conn.prepare(sql).map_err(|e| map_sql_prepare_error(op, e))?;

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
