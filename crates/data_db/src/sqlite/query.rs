use rusqlite::{Connection, Error as SqliteError, params, types::Value as SqlValue};
use syneroym_fdae::CompiledSieve;

use super::{
    MAX_QUERY_PAGE_SIZE,
    query_raw::{QUERY_RAW_MAX_VM_OPS, QueryRawGuard, map_query_raw_step_error, run_query_raw},
    schema::{RECORD_COLUMNS, RECORD_COLUMNS_WITHOUT_ID, VAULT_TABLE, validate_identifier},
    sieve::{
        ModeAOutcome, emit_mode_a_execution_trace, emit_mode_b_trace, install_watchdog, merge_sieve,
    },
};
use crate::{aggregate, errors::map_rusqlite_error, filter, host_store};

/// `sieve` is `Some` only for `Mode::PointInTime{id}` (`compile_read` already
/// appends `... AND {table}.id = ?` to the RLS), so a sieve'd fetch is a
/// self-contained `WHERE` -- no separate `id = ?1` alongside it, which would
/// double-bind the id.
pub(super) fn do_get(
    conn: &Connection,
    collection: &str,
    id: &str,
    sieve: Option<&CompiledSieve>,
) -> Result<Option<host_store::RecordReadValue>, host_store::DataLayerError> {
    validate_identifier(collection)?;

    // Wrapped in the same "capture every error, decide after" shape
    // `do_check_access` uses, so a watchdog interrupt or a malformed caveat
    // can be told apart from a genuine `QueryReturnedNoRows` for the
    // decision trace below -- both used to `?`-propagate identically,
    // leaving no evaluation-aborted case for the trace to fill in at all.
    let outcome: Result<rusqlite::Result<(String, String, i64, i64)>, host_store::DataLayerError> =
        (|| {
            Ok(match sieve {
                None => conn.query_row(
                    &format!("SELECT {RECORD_COLUMNS_WITHOUT_ID} FROM {collection} WHERE id = ?1"),
                    params![id],
                    read_record_row,
                ),
                Some(s) => {
                    let _watchdog = install_watchdog(conn)?;
                    let (clause, sieve_params) = merge_sieve(s)?;
                    conn.query_row(
                        &format!(
                            "SELECT {RECORD_COLUMNS_WITHOUT_ID} FROM {collection} WHERE {clause}"
                        ),
                        rusqlite::params_from_iter(sieve_params.iter()),
                        read_record_row,
                    )
                }
            })
        })();

    emit_mode_a_execution_trace(
        sieve.map(|s| &s.trace),
        match &outcome {
            Ok(Ok(_)) => ModeAOutcome::Matched,
            Ok(Err(SqliteError::QueryReturnedNoRows)) => ModeAOutcome::NotMatched,
            Ok(Err(e)) => ModeAOutcome::Aborted(format!("{e}")),
            Err(e) => ModeAOutcome::Aborted(format!("{e:?}")),
        },
    );

    match outcome {
        Ok(Ok((payload, creator_id, created_at, updated_at))) => {
            Ok(Some(host_store::RecordReadValue {
                id: id.to_string(),
                payload: payload.into_bytes(),
                creator_id,
                created_at: created_at as u64,
                updated_at: updated_at as u64,
            }))
        }
        // Unauthorized-but-existing and genuinely-missing are
        // indistinguishable here by design (ADR-0007 "no result is a valid
        // outcome").
        Ok(Err(SqliteError::QueryReturnedNoRows)) => Ok(None),
        Ok(Err(e)) => Err(map_query_raw_step_error(e)),
        Err(e) => Err(e),
    }
}

fn read_record_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, String, i64, i64)> {
    Ok((
        row.get::<_, String>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, i64>(2)?,
        row.get::<_, i64>(3)?,
    ))
}

pub(super) fn do_query(
    conn: &Connection,
    collection: &str,
    opts: &host_store::QueryOptions,
    sieve: Option<&CompiledSieve>,
) -> Result<host_store::QueryResult, host_store::DataLayerError> {
    validate_identifier(collection)?;
    emit_mode_b_trace(sieve);

    // A CLS-masked field must not be filterable either -- otherwise masking
    // only the projection turns the predicate into an oracle that recovers
    // the value via presence/absence (or, with `$regex`/comparison
    // operators, full extraction) even though it never appears in a
    // returned payload.
    if let Some(s) = sieve
        && !s.masked_fields.is_empty()
    {
        let referenced = filter::referenced_top_level_fields(opts.filter.as_deref())?;
        if s.masked_fields.iter().any(|f| referenced.contains(f)) {
            return Err(host_store::DataLayerError::PermissionDenied);
        }
    }

    let compiled = filter::compile_filter(opts.filter.as_deref())?;
    let limit = opts.limit.unwrap_or(MAX_QUERY_PAGE_SIZE).min(MAX_QUERY_PAGE_SIZE);

    let mut where_clauses = Vec::new();
    let mut bound_params: Vec<SqlValue> = Vec::new();

    // Sieve first (RLS ∧ caveats), ahead of the caller's filter/cursor/limit.
    let _watchdog = if let Some(s) = sieve {
        let (clause, sieve_params) = merge_sieve(s)?;
        where_clauses.push(clause);
        bound_params.extend(sieve_params);
        Some(install_watchdog(conn)?)
    } else {
        None
    };
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

    let sql =
        format!("SELECT {RECORD_COLUMNS} FROM {collection} {where_sql} ORDER BY id ASC LIMIT ?");
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
        .map_err(map_query_raw_step_error)?;

    let next_cursor = if records.len() as u32 > limit {
        records.truncate(limit as usize);
        records.last().map(|r| r.id.clone())
    } else {
        None
    };
    Ok(host_store::QueryResult { records, next_cursor })
}

/// Mode A `check-access`: `sieve` is `None` either because the caller passed
/// no `auth` or because the policy names no definition for `collection` (an
/// unfiltered read); either way that falls back to a plain existence
/// check. `Some(sieve)`
/// (possibly the `deny_all` `0=1`) is a self-contained `id`-bound predicate
/// (`Mode::PointInTime`), same shape as `do_get`'s sieve branch.
pub(super) fn do_check_access(
    conn: &Connection,
    collection: &str,
    id: &str,
    sieve: Option<&CompiledSieve>,
) -> Result<bool, host_store::DataLayerError> {
    validate_identifier(collection)?;

    // Fail-closed past this point: a malformed caveat, a watchdog-install
    // failure, a missing target table, or a watchdog interrupt must never be
    // mistaken for "allowed" -- only a real `true`/`false` existence answer
    // is trusted.
    let outcome: Result<rusqlite::Result<bool>, host_store::DataLayerError> = (|| {
        Ok(match sieve {
            None => conn.query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {collection} WHERE id = ?1)"),
                params![id],
                |row| row.get::<_, bool>(0),
            ),
            Some(s) => {
                let _watchdog = install_watchdog(conn)?;
                let (clause, sieve_params) = merge_sieve(s)?;
                conn.query_row(
                    &format!("SELECT EXISTS(SELECT 1 FROM {collection} WHERE {clause})"),
                    rusqlite::params_from_iter(sieve_params.iter()),
                    |row| row.get::<_, bool>(0),
                )
            }
        })
    })();

    emit_mode_a_execution_trace(
        sieve.map(|s| &s.trace),
        match &outcome {
            Ok(Ok(true)) => ModeAOutcome::Matched,
            Ok(Ok(false)) => ModeAOutcome::NotMatched,
            Ok(Err(e)) => ModeAOutcome::Aborted(format!("{e}")),
            Err(e) => ModeAOutcome::Aborted(format!("{e:?}")),
        },
    );

    Ok(match outcome {
        Ok(Ok(exists)) => exists,
        Ok(Err(_)) | Err(_) => false,
    })
}

/// Lists the service's collections (user tables) for the deploy-time
/// `strict:` author-time warning (ADR-0017 §1): excludes SQLite
/// internals (`sqlite_%`) and the host's own `_vault` table, since those are
/// never `definitions:` targets in a policy document.
pub(super) fn do_list_collections(
    conn: &mut Connection,
) -> Result<Vec<String>, host_store::DataLayerError> {
    // Excludes SQLite's own internal tables and the host's own `_vault` by
    // exact name -- not the whole `_%` namespace. `IDENTIFIER_REGEX` permits
    // a leading underscore (`^[a-zA-Z_]...`), so a guest-created collection
    // like `_audit` is a legal name that must still appear here for the
    // `strict:` warning to see it correctly in both directions.
    let mut stmt = conn
        .prepare(&format!(
            "SELECT name FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\'
               AND name != '{VAULT_TABLE}'"
        ))
        .map_err(|e| host_store::DataLayerError::Internal(format!("list_collections: {e}")))?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| host_store::DataLayerError::Internal(format!("list_collections: {e}")))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| host_store::DataLayerError::Internal(format!("list_collections: {e}")))?;
    Ok(names)
}

/// Runs an `aggregate` call (ADR-0007) on the reader pool. The
/// compiled SQL is entirely host-generated (bound params + validated
/// identifiers only), so it is `readonly()` by construction and needs none
/// of `do_query_raw`'s authorizer (`deny_query_raw_escapes` defends against
/// *arbitrary caller SQL* containing `ATTACH`/`DETACH`/pragma-set, which the
/// compiler can never emit). It **does** install the same progress-handler
/// compute backstop (`QUERY_RAW_MAX_VM_OPS`): `aggregate` carries no
/// capability gate (open to any caller, like `query`), and unlike `query`'s
/// `LIMIT`, a `GROUP BY`/`ORDER BY` does its scanning/hashing/sorting work
/// over the *whole* collection before `$limit`/`$skip` ever apply, so the
/// row-count page cap alone does not bound compute here either.
/// `sieve.masked_fields` non-empty (a CLS-active policy) fails the whole
/// aggregate closed rather than attempting a CLS-safe aggregation -- an
/// aggregate's `SUM`/`AVG`/etc. can leak a masked field's value through its
/// output even without projecting the raw column, and there is no general
/// way to tell which accumulators are "safe" over a masked field. RLS,
/// unlike CLS, injects cleanly into the inner query's `WHERE`. The same
/// reasoning extends to `sieve.abac_permissions` (ADR-0017 §7): the stage-4
/// after-step needs materialized candidate rows to judge, and an aggregate
/// never surfaces rows -- only accumulator output, which can leak a
/// would-be-denied row's contribution the same way a masked field can leak
/// through `SUM`/`AVG`.
pub(super) fn do_aggregate(
    conn: &Connection,
    collection: &str,
    pipeline_json: &str,
    sieve: Option<&CompiledSieve>,
) -> Result<host_store::RawQueryResult, host_store::DataLayerError> {
    validate_identifier(collection)?;
    emit_mode_b_trace(sieve);

    // Check CLS/stage-4 denial *before* compiling caveat filters: either
    // denies the whole call regardless of what its caveats say, so there is
    // no reason to pay a caveat-filter compile (or surface its error, if the
    // caveat is malformed) on a call that is about to be denied anyway.
    if let Some(s) = sieve
        && (!s.masked_fields.is_empty() || !s.abac_permissions.is_empty())
    {
        return Err(host_store::DataLayerError::PermissionDenied);
    }
    let merged = sieve.map(merge_sieve).transpose()?;
    let sieve_arg = merged.as_ref().map(|(clause, params)| (clause.as_str(), params.as_slice()));

    let compiled = aggregate::compile(collection, pipeline_json, sieve_arg)?;
    conn.progress_handler(QUERY_RAW_MAX_VM_OPS, Some(|| true)).map_err(map_rusqlite_error)?;
    let _guard = QueryRawGuard { conn };
    run_query_raw(conn, "aggregate", &compiled.sql, &compiled.params)
}
