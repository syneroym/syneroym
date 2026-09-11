use syneroym_data_db::{auth::QueryAuth, traits::ServiceStore};
use syneroym_fdae::{MAX_FETCH_IDS, Mode};
use syneroym_identity::{DelegationCertificate, Identity};
use syneroym_rpc::{
    Ability, NativeInvocation, NativeResponse, RelationshipProof, ResourceUri, RpcResult,
};
use syneroym_wit_interfaces::host::syneroym::data_layer::store::{
    DataLayerError, QueryOptions, RawQueryResult, SqlValue,
};

use super::{data::data_layer_error, *};

/// Signs `ids` as a `syneroym_rpc::RelationshipProof` asserted by
/// `identity`, under `certificate`'s master when one is installed. Thin
/// wrapper so this file's call sites don't need to know the proof lives in
/// `syneroym-rpc` (shared with the requesting side,
/// `syneroym_rpc::fdae_fetch::resolve_fetches`, which verifies one).
fn sign_relationship_proof(
    identity: &Identity,
    certificate: Option<&DelegationCertificate>,
    relation: &str,
    principal: &str,
    ids: Vec<String>,
) -> RpcResult<RelationshipProof> {
    RelationshipProof::sign(identity, certificate, relation, principal, ids)
        .map_err(|e| internal(format!("failed to sign relationship proof: {e}")))
}

/// Extracts the single `id` column from a `SELECT id FROM ... WHERE ...`
/// [`RawQueryResult`] (the A2 structural-resolution query). Fails closed on
/// a shape this query never produces (a non-text id, or more/fewer than one
/// column) rather than silently coercing or dropping rows.
fn extract_id_column(result: RawQueryResult) -> RpcResult<Vec<String>> {
    result
        .rows
        .into_iter()
        .map(|row| match row.as_slice() {
            [SqlValue::Text(id)] => Ok(id.clone()),
            _ => {
                Err(internal("resolve-relation: structural query returned an unexpected row shape"))
            }
        })
        .collect()
}

impl SynSvcNativeService {
    pub(super) async fn resolve_query_auth<'a>(
        &'a self,
        invocation: &'a NativeInvocation,
        collection: &str,
        operation: &Ability,
        mode: Mode,
    ) -> Result<Option<QueryAuth<'a>>, DataLayerError> {
        let Some(policy) = self.fdae_policy.as_ref() else { return Ok(None) };
        let session = &invocation.caller.session;
        let plan = syneroym_fdae::plan_read(
            policy,
            collection,
            session,
            &self.service_id,
            operation,
            mode,
        )
        .map_err(|e| DataLayerError::Internal(e.to_string()))?;
        let resolved_sieve = if plan.fetches.is_empty() {
            plan.local
        } else {
            let proxy = self.service_proxy.upgrade().ok_or_else(|| {
                DataLayerError::Internal(
                    "service proxy unavailable for a cross-service FDAE fetch".to_string(),
                )
            })?;
            let caller = invocation.caller.clone();
            let results = syneroym_rpc::resolve_fetches(
                &plan.fetches,
                &caller,
                proxy.as_ref(),
                &self.service_id,
            )
            .await
            .map_err(|e| {
                tracing::warn!(
                    error = %e,
                    collection,
                    "fdae: cross-service relationship fetch failed, denying closed"
                );
                DataLayerError::PermissionDenied
            })?;
            let pending = plan.pending.ok_or_else(|| {
                DataLayerError::Internal(
                    "internal: plan_read reported fetches but no pending sieve".to_string(),
                )
            })?;
            Some(
                syneroym_fdae::finalize(pending, &results)
                    .map_err(|e| DataLayerError::Internal(e.to_string()))?,
            )
        };
        Ok(Some(QueryAuth { policy, session, service_id: &self.service_id, resolved_sieve }))
    }

    /// The *receiving* (data-owning) side of relation resolution: "which
    /// rows does `principal` reach via `relation`," signed as a
    /// [`RelationshipProof`]. The *sending* side (issuing the fetch, timeout
    /// handling, `plan_read`/`finalize` wiring) lives elsewhere -- this
    /// method only answers the question, over whatever transport already got
    /// the request here (native dispatch, `dispatch_json_rpc_once`).
    ///
    /// **A1 vs. A2, mutually exclusive per request, not a fallback
    /// chain:** if the caller holds a capability scoped to *this resource*
    /// (not merely *any* capability -- an unrelated grant on some
    /// other collection must not change the answer), only **A1** (the
    /// existing capability-gated sieve, via `ServiceStore::query`) is
    /// attempted -- an empty result is a real, final deny, never silently
    /// widened by A2. **A2** (a bare `principal_column` match, gated only by
    /// the definition's own `resolvable_without_capability` opt-in) applies
    /// only when the caller holds no capability scoped here, so a
    /// real-but-denied A1 decision can never be second-guessed by the
    /// looser A2 model.
    pub(super) async fn resolve_relation(
        &self,
        invocation: &NativeInvocation,
        store: &dyn ServiceStore,
    ) -> RpcResult<NativeResponse> {
        #[derive(serde::Deserialize)]
        struct Req {
            relation: String,
            principal: String,
        }
        let req: Req = parse_params(invocation)?;

        // The wire caller must be re-verified as exactly the principal
        // being asked about -- either the direct verified identity or the
        // anchor it's proxying for (`anchor_did.unwrap_or(subject_did)`,
        // the same fallback `compile::terminal_value`/`emit_remote_terminal`
        // use). B3 exists precisely because `caller != anchor`: a forwarded
        // chain `alice -> svc-A` re-verifies here with `subject_did =
        // svc-A` (whoever actually authenticated this connection) and
        // `anchor_did = alice`, while `RemoteFetch.principal_did` is always
        // the anchor -- comparing against `subject_did` alone would reject
        // every genuinely cross-service ask. `principal` is still a
        // caller-declared label that must match one of these, never a free
        // parameter naming an arbitrary third party.
        let effective_principal = invocation
            .caller
            .session
            .anchor_did
            .as_deref()
            .unwrap_or(&invocation.caller.session.subject_did);
        if req.principal != effective_principal {
            return Err(data_layer_error(DataLayerError::PermissionDenied));
        }

        let Some(policy) = self.fdae_policy.as_ref() else {
            let proof = sign_relationship_proof(
                &self.service_identity,
                self.instance_cert.as_ref(),
                &req.relation,
                &req.principal,
                Vec::new(),
            )?;
            return to_payload(&proof);
        };

        // No definition matches `relation` at all: unlike an ordinary read,
        // where "no definition" correctly falls through to unfiltered
        // (grant-layer-admitted) access, a cross-service relationship ask
        // has no backing grant-layer admission -- deny outright rather than
        // let `ServiceStore::query`'s own no-definition pass-through leak
        // the whole collection. Also resolves the definition's *physical
        // table* -- `ServiceStore::query` addresses a collection literally
        // (unlike `compile_read`'s own permissive key-or-table matching),
        // so a `relation` naming a policy *key* (the fetch convention, e.g.
        // `RemoteFetch.relation`) must be translated before it reaches the
        // A1 `store.query` call, or a key that isn't also the table's own name
        // spuriously fails `collection-not-found`.
        let Some(table) = syneroym_fdae::definition_table(policy, &req.relation) else {
            let proof = sign_relationship_proof(
                &self.service_identity,
                self.instance_cert.as_ref(),
                &req.relation,
                &req.principal,
                Vec::new(),
            )?;
            return to_payload(&proof);
        };

        // A definition whose permissions opt into the stage-4
        // after-step (ADR-0017 §7) cannot be resolved structurally by
        // either A1 or A2 -- neither branch below has a compiled sieve in
        // hand (the A1 `store.query` call builds `QueryAuth { resolved_sieve:
        // None, .. }` and lets `data_db` compile internally; A2 has no
        // sieve at all), so there is nothing to read `abac_permissions`
        // from per-read. This is a coarser, definition-level check
        // (`definition_has_abac`) than a compiled sieve's own
        // `abac_permissions` -- deliberately so: the remote asking must not
        // be able to route around this node's after-step by resolving
        // structurally instead of through the direct, after-step-aware read
        // path.
        if syneroym_fdae::definition_has_abac(policy, &req.relation) {
            return Err(data_layer_error(DataLayerError::PermissionDenied));
        }

        // B3-07: the A1/A2 fork is keyed on whether the caller holds *any*
        // capability scoped to *this resource* -- not on whether they hold
        // capabilities at all, which would make the fork's outcome depend
        // on unrelated grants a chain happens to carry. Mirrors the same
        // resource-matching `Capability::grants` uses internally.
        let resource = ResourceUri(format!(
            "{}/collection/{table}",
            ResourceUri::service(&self.service_id, &self.service_id).0
        ));
        let has_scoped_capability = invocation
            .caller
            .session
            .capabilities
            .iter()
            .any(|cap| cap.with.is_substrate_scope() || cap.with.covers_resource(&resource));

        let ids = if !has_scoped_capability {
            match syneroym_fdae::resolve_structural(policy, &req.relation, &req.principal) {
                Ok(Some(resolved)) => {
                    let sql = format!(
                        "SELECT id FROM {} WHERE {} LIMIT {}",
                        resolved.table,
                        resolved.where_clause,
                        MAX_FETCH_IDS + 1
                    );
                    let params: Vec<SqlValue> =
                        resolved.params.into_iter().map(SqlValue::Text).collect();
                    // B3-06: `query_raw`'s own doc comment documents itself
                    // as privileged ("callers must have already verified
                    // `data-layer/admin`"), a contract this call
                    // deliberately does not satisfy -- the caller reaching
                    // this branch holds *no* relevant capability at all
                    // (that's the A2 fork condition above). Safe anyway,
                    // for reasons specific to this one call site, not a
                    // general precedent: (1) the SQL text and every bound
                    // identifier come from `resolve_structural`, whose
                    // `table`/`principal_column`/`join_column` are all
                    // constrained by the policy schema's
                    // `^[A-Za-z_][A-Za-z0-9_]*$` `sql_identifier` pattern,
                    // so there is no caller-controlled string reaching the
                    // query text; (2) the actual authorization gate is
                    // `Definition::resolvable_without_capability`, an
                    // explicit per-definition opt-in the *policy author*
                    // controls -- `query_raw`'s admin check would be
                    // redundant with, not a replacement for, that gate.
                    let raw = store.query_raw(&sql, &params).await.map_err(data_layer_error)?;
                    extract_id_column(raw)?
                }
                // Not opted into `resolvable_without_capability` -- deny,
                // never treated as "found nothing to structurally resolve
                // so fall back to something looser."
                Ok(None) => Vec::new(),
                Err(e) => return Err(internal(e.to_string())),
            }
        } else {
            // The evaluation session presents the *effective principal*
            // (already validated above) as `subject_did`, not necessarily
            // the immediate connection identity -- resolve-relation answers
            // "what can the principal reach," so a `caller`-terminal path in
            // the remote's *own* policy must bind to that principal, per
            // the same confused-deputy reasoning `emit_remote_terminal`
            // documents. The real capabilities present on the connection are
            // unchanged; only the identity they're evaluated against shifts.
            let mut session = invocation.caller.session.clone();
            session.subject_did = req.principal.clone();
            let auth = QueryAuth {
                policy,
                session: &session,
                service_id: &self.service_id,
                resolved_sieve: None,
            };
            let opts = QueryOptions {
                filter: None,
                limit: Some(u32::try_from(MAX_FETCH_IDS).unwrap_or(u32::MAX)),
                cursor: None,
            };
            let outcome = store.query(table, &opts, Some(&auth)).await.map_err(data_layer_error)?;
            if outcome.value.next_cursor.is_some() {
                return Err(data_layer_error(DataLayerError::QuotaExceeded));
            }
            outcome.value.records.into_iter().map(|r| r.id).collect()
        };

        if ids.len() > MAX_FETCH_IDS {
            return Err(data_layer_error(DataLayerError::QuotaExceeded));
        }
        let proof = sign_relationship_proof(
            &self.service_identity,
            self.instance_cert.as_ref(),
            &req.relation,
            &req.principal,
            ids,
        )?;
        to_payload(&proof)
    }
}
