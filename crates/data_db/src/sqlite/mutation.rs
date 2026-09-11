use std::str;

use chrono::Utc;
use rusqlite::{Connection, Error as SqliteError, params, types::Value as SqlValue};
use serde_json::{Map, Value};
use syneroym_fdae::CompiledSieve;

use super::{
    MAX_BATCH_SIZE,
    query_raw::map_query_raw_step_error,
    schema::validate_identifier,
    sieve::{
        ModeAOutcome, emit_mode_a_execution_trace, emit_mode_b_trace, install_watchdog, merge_sieve,
    },
};
use crate::{errors::map_rusqlite_error, filter, host_store};

/// Applies an RFC 7396 JSON merge-patch: `patch` values overwrite `target`,
/// `null` values remove the key, and nested objects merge recursively.
pub(crate) fn apply_merge_patch(target: &mut Value, patch: &Value) {
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

pub(crate) fn payload_to_text(payload: &[u8]) -> Result<String, host_store::DataLayerError> {
    let text = str::from_utf8(payload).map_err(|_| {
        host_store::DataLayerError::SchemaViolation("payload must be valid UTF-8".into())
    })?;
    serde_json::from_str::<Value>(text).map_err(|e| {
        host_store::DataLayerError::SchemaViolation(format!("payload is not valid JSON: {e}"))
    })?;
    Ok(text.to_string())
}

pub(crate) fn do_put(
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

    // `creator_id` is create-time-only, like `created_at` above -- an
    // upsert must not let a later writer reassign a row's identity anchor
    // to themselves, which would silently steal ownership out from under a
    // `principal_column: "creator_id"` policy.
    conn.execute(
        &format!(
            "INSERT INTO {collection} (id, payload, creator_id, created_at, updated_at) VALUES \
             (?1, ?2, ?3, ?4, ?5) ON CONFLICT(id) DO UPDATE SET payload = excluded.payload, \
             updated_at = excluded.updated_at"
        ),
        params![value.id, payload_text, creator_id, created_at, now],
    )
    .map_err(map_rusqlite_error)?;
    Ok(())
}

pub(crate) fn do_patch(
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

pub(crate) fn do_delete(
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

pub(crate) fn do_delete_many(
    conn: &Connection,
    collection: &str,
    filter_json: Option<&str>,
    sieve: Option<&CompiledSieve>,
) -> Result<u64, host_store::DataLayerError> {
    validate_identifier(collection)?;
    // A stage-4-active sieve (ADR-0017 §7) needs the deleted rows
    // materialized and run through the guest-exported after-step before
    // they may be removed -- deletion happens inside SQL, so there is
    // nothing to hand the after-step. Deny closed, same category as the CLS
    // denial below: a permission opting into `authorize_rows` never widens
    // what `delete_many` may remove, so this cannot be worked around by
    // narrowing the caller's request.
    if sieve.is_some_and(|s| !s.abac_permissions.is_empty()) {
        return Err(host_store::DataLayerError::PermissionDenied);
    }
    emit_mode_b_trace(sieve);
    let compiled = filter::compile_filter(filter_json)?;

    let mut where_clauses = Vec::new();
    let mut bound_params: Vec<SqlValue> = Vec::new();
    let _watchdog = if let Some(s) = sieve {
        let (clause, params) = merge_sieve(s)?;
        where_clauses.push(clause);
        bound_params.extend(params);
        Some(install_watchdog(conn)?)
    } else {
        None
    };
    if let Some(cf) = &compiled {
        where_clauses.push(cf.where_clause.clone());
        bound_params.extend(cf.params.iter().cloned());
    }
    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    let affected = conn
        .execute(
            &format!("DELETE FROM {collection} {where_sql}"),
            rusqlite::params_from_iter(bound_params.iter()),
        )
        .map_err(map_query_raw_step_error)?;
    Ok(affected as u64)
}

pub(crate) fn do_batch_mutate(
    conn: &mut Connection,
    collection: &str,
    mutations: &[host_store::Mutation],
    creator_id: &str,
    sieve: Option<&CompiledSieve>,
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
            host_store::Mutation::Put(value) => {
                // Only probed under a sieve (decides the create-vs-update
                // branch of the `USING`/`WITH CHECK` split below) --
                // `authorize_and_mutate` ignores `require_pre_image` entirely
                // on the policy-absent path, so there is nothing to gain
                // from paying this query when `sieve` is `None`.
                let existed =
                    if sieve.is_some() { row_exists(&tx, collection, &value.id)? } else { false };
                authorize_and_mutate(
                    &tx,
                    collection,
                    &value.id,
                    sieve,
                    existed,
                    true,
                    false,
                    |c| do_put(c, collection, value, creator_id),
                )?;
            }
            host_store::Mutation::Patch(patch_mutation) => {
                authorize_and_mutate(
                    &tx,
                    collection,
                    &patch_mutation.id,
                    sieve,
                    true,
                    true,
                    false,
                    |c| do_patch(c, collection, &patch_mutation.id, &patch_mutation.patch_json),
                )?;
            }
            host_store::Mutation::Delete(id) => {
                authorize_and_mutate(&tx, collection, id, sieve, true, false, false, |c| {
                    do_delete(c, collection, id)
                })?;
            }
        }
    }
    tx.commit().map_err(map_rusqlite_error)?;
    Ok(())
}

/// The unsieved create-vs-update probe: decides *which* rule applies
/// (pre-image required, or not), not whether the write is allowed --
/// both branches still end in `PermissionDenied` on failure, so this leaks
/// nothing beyond "id `X` was free", inherent to any create-by-id API.
pub(crate) fn row_exists(
    conn: &Connection,
    collection: &str,
    id: &str,
) -> Result<bool, host_store::DataLayerError> {
    conn.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {collection} WHERE id = ?1)"),
        params![id],
        |row| row.get::<_, bool>(0),
    )
    .map_err(map_rusqlite_error)
}

/// Reads a record's raw JSON payload directly, bypassing the sieve -- only
/// ever called from `authorize_and_mutate` after the relevant reachability
/// check has already run (or for a create, where there is no pre-image).
pub(crate) fn read_payload(
    conn: &Connection,
    collection: &str,
    id: &str,
) -> Result<Vec<u8>, host_store::DataLayerError> {
    let payload: String = conn
        .query_row(&format!("SELECT payload FROM {collection} WHERE id = ?1"), params![id], |row| {
            row.get(0)
        })
        .map_err(map_rusqlite_error)?;
    Ok(payload.into_bytes())
}

/// Whether every masked field's value is unchanged between a write's
/// pre- and post-image (CLS extended to the write path). `pre: None` means
/// a create -- any masked key present
/// in `post` is a rejection, since a caller who cannot read a field cannot
/// author it either. Fail-closed on a payload that won't parse as a JSON
/// object while a non-empty mask applies, mirroring
/// `auth::strip_masked_fields`'s own rule.
pub(crate) fn masked_fields_unchanged(
    pre: Option<&[u8]>,
    post: &[u8],
    masked: &[String],
) -> Result<bool, host_store::DataLayerError> {
    // Fail-closed as `PermissionDenied`, not `SchemaViolation` -- this is
    // the same envelope `row_reachable` folds every abort into, and a
    // caller must not be able to tell "CLS couldn't evaluate" apart from
    // "CLS said no" (a distinguishable error is exactly the existence
    // oracle CLS-masking already refuses to provide elsewhere in this
    // file). A row stored as a non-object payload (a lifecycle write, or
    // written before the policy existed) becomes unwritable under a
    // CLS-active policy either way; the point is that it fails the same
    // way a denial does.
    fn as_object(payload: &[u8]) -> Result<Map<String, Value>, host_store::DataLayerError> {
        match serde_json::from_slice(payload) {
            Ok(Value::Object(map)) => Ok(map),
            _ => Err(host_store::DataLayerError::PermissionDenied),
        }
    }

    let post_map = as_object(post)?;
    let pre_map = pre.map(as_object).transpose()?;
    for field in masked {
        let post_val = post_map.get(field);
        let unchanged = match &pre_map {
            None => post_val.is_none(),
            Some(pre_map) => pre_map.get(field) == post_val,
        };
        if !unchanged {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Evaluates a `data-layer/write` sieve against exactly one row id on `conn`.
/// The sieve is compiled in `Mode::Filter` (once per call, not per row), so
/// the id predicate is appended here -- equivalent to `Mode::PointInTime`,
/// without recompiling for every mutation in a batch.
///
/// Fail-closed: a watchdog interrupt, a malformed caveat, or a missing table
/// is `Ok(false)`, never a silent pass. Same contract as `do_check_access`.
pub(crate) fn row_reachable(
    conn: &Connection,
    collection: &str,
    id: &str,
    sieve: &CompiledSieve,
    phase: &'static str,
    trace_allows: bool,
) -> Result<bool, host_store::DataLayerError> {
    let outcome: Result<rusqlite::Result<bool>, host_store::DataLayerError> = (|| {
        let _watchdog = install_watchdog(conn)?;
        let (clause, mut params) = merge_sieve(sieve)?;
        params.push(SqlValue::Text(id.to_string()));
        Ok(conn.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM {collection} WHERE ({clause}) AND {collection}.id = \
                 ?)"
            ),
            rusqlite::params_from_iter(params.iter()),
            |row| row.get::<_, bool>(0),
        ))
    })();

    let allowed = matches!(outcome, Ok(Ok(true)));
    if !allowed || trace_allows {
        // Clone only the trace, not the whole compiled sieve (SQL text,
        // bound params, caveats) -- `emit_mode_a_execution_trace` never
        // reads anything else.
        let mut trace = sieve.trace.clone();
        trace.row_id = Some(id.to_string());
        trace.write_phase = Some(phase.to_string());
        emit_mode_a_execution_trace(
            Some(&trace),
            match &outcome {
                Ok(Ok(true)) => ModeAOutcome::Matched,
                Ok(Ok(false)) => ModeAOutcome::NotMatched,
                Ok(Err(e)) => ModeAOutcome::Aborted(format!("{e}")),
                Err(e) => ModeAOutcome::Aborted(format!("{e:?}")),
            },
        );
    }
    Ok(allowed)
}

/// One authorized single-row mutation (ADR-0017 §4 Mode A, write side).
/// `sieve == None` (policy-absent, or an exempt caller) is today's
/// unfiltered behavior, unchanged. `conn` is always inside a transaction the
/// caller owns, so an `Err` here rolls the mutation back.
#[allow(clippy::too_many_arguments)]
pub(crate) fn authorize_and_mutate(
    conn: &Connection,
    collection: &str,
    id: &str,
    sieve: Option<&CompiledSieve>,
    require_pre_image: bool,
    check_post_image: bool,
    trace_allows: bool,
    mutate: impl FnOnce(&Connection) -> Result<(), host_store::DataLayerError>,
) -> Result<(), host_store::DataLayerError> {
    validate_identifier(collection)?;
    let Some(sieve) = sieve else { return mutate(conn) };

    // No candidate-row batch exists mid-mutation, so the stage-4 after-step
    // cannot run -- deny closed, same rule as `do_delete_many`/`do_aggregate`.
    if !sieve.abac_permissions.is_empty() {
        return Err(host_store::DataLayerError::PermissionDenied);
    }

    // USING half: may the caller reach the row as it stands today? This
    // subsumes existence -- a `delete`/`patch` of a row that does not exist
    // under a policy therefore denies rather than reporting not-found, since
    // this check cannot distinguish "absent" from "present but unreachable"
    // without becoming the existence oracle CLS-masking already refuses to
    // provide.
    if require_pre_image && !row_reachable(conn, collection, id, sieve, "pre-image", trace_allows)?
    {
        return Err(host_store::DataLayerError::PermissionDenied);
    }

    // Capture the pre-image payload only when CLS is active *and* there's a
    // post-image check to compare it against -- `delete` (`require_pre_image
    // && !check_post_image`) never reads `pre_payload` below, so paying for
    // it there would be a wasted read on every delete under a CLS policy.
    let pre_payload = if !sieve.masked_fields.is_empty() && require_pre_image && check_post_image {
        Some(read_payload(conn, collection, id)?)
    } else {
        None
    };

    mutate(conn)?;

    // WITH CHECK half: may the caller reach the row they just wrote? Rejects
    // both "create a row you could never see" and "rewrite a row out of
    // your own reach". `Err` rolls the caller's transaction back.
    if check_post_image && !row_reachable(conn, collection, id, sieve, "post-image", trace_allows)?
    {
        return Err(host_store::DataLayerError::PermissionDenied);
    }

    // A field the caller cannot read is one they cannot write.
    if !sieve.masked_fields.is_empty() && check_post_image {
        let post = read_payload(conn, collection, id)?;
        if !masked_fields_unchanged(pre_payload.as_deref(), &post, &sieve.masked_fields)? {
            return Err(host_store::DataLayerError::PermissionDenied);
        }
    }
    Ok(())
}

/// Transaction wrapper for an authorized `put`. Opens a transaction only
/// when a sieve is present, so the policy-absent hot path pays nothing new.
pub(crate) fn do_authorized_put(
    conn: &mut Connection,
    collection: &str,
    value: &host_store::RecordWriteValue,
    creator_id: &str,
    sieve: Option<&CompiledSieve>,
) -> Result<(), host_store::DataLayerError> {
    let Some(sieve) = sieve else { return do_put(conn, collection, value, creator_id) };
    validate_identifier(collection)?;
    let tx = conn.transaction().map_err(map_rusqlite_error)?;
    let existed = row_exists(&tx, collection, &value.id)?;
    authorize_and_mutate(&tx, collection, &value.id, Some(sieve), existed, true, true, |c| {
        do_put(c, collection, value, creator_id)
    })?;
    tx.commit().map_err(map_rusqlite_error)
}

/// Transaction wrapper for an authorized `patch`. See `do_authorized_put`.
pub(crate) fn do_authorized_patch(
    conn: &mut Connection,
    collection: &str,
    id: &str,
    patch_json: &[u8],
    sieve: Option<&CompiledSieve>,
) -> Result<(), host_store::DataLayerError> {
    let Some(sieve) = sieve else { return do_patch(conn, collection, id, patch_json) };
    validate_identifier(collection)?;
    let tx = conn.transaction().map_err(map_rusqlite_error)?;
    authorize_and_mutate(&tx, collection, id, Some(sieve), true, true, true, |c| {
        do_patch(c, collection, id, patch_json)
    })?;
    tx.commit().map_err(map_rusqlite_error)
}

/// Transaction wrapper for an authorized `delete`. See `do_authorized_put`.
/// No `WITH CHECK` half -- a deleted row has no post-image to evaluate.
pub(crate) fn do_authorized_delete(
    conn: &mut Connection,
    collection: &str,
    id: &str,
    sieve: Option<&CompiledSieve>,
) -> Result<(), host_store::DataLayerError> {
    let Some(sieve) = sieve else { return do_delete(conn, collection, id) };
    validate_identifier(collection)?;
    let tx = conn.transaction().map_err(map_rusqlite_error)?;
    authorize_and_mutate(&tx, collection, id, Some(sieve), true, false, true, |c| {
        do_delete(c, collection, id)
    })?;
    tx.commit().map_err(map_rusqlite_error)
}
