//! Slice B4-fdae: the after-step seam for ADR-0017 §7's stage-4 WASM ABAC.
//! Sibling to `fdae_fetch.rs` and for the same reason -- `crates/fdae` stays
//! engine-free, `crates/data_db` has no WASM dependency, and `syneroym-rpc`
//! is the one crate both read ingresses already depend on. `syneroym-rpc`
//! does not depend on `syneroym-wit-interfaces`, so the DTOs below are
//! rpc-local; each ingress converts its own WIT-generated row type into a
//! [`CandidateRow`] and back.

use std::{
    fmt,
    sync::{Arc, Weak},
    time::Duration,
};

use syneroym_fdae::{AbacTrace, CompiledSieve};
use syneroym_ucan::SessionContext;
use tokio::time;

/// Wall-clock ceiling one after-step invocation may not exceed, enforced by
/// the caller in addition to the engine's own epoch deadline (same
/// belt-and-braces reasoning as `FDAE_FETCH_TIMEOUT` vs. the proxy's own
/// advisory timeout).
pub const FDAE_ABAC_TIMEOUT: Duration = Duration::from_secs(3);

/// Hard cap on one batch, matching `data_db`'s `MAX_QUERY_PAGE_SIZE` -- one
/// page is one batch, so this can never be the binding constraint on a
/// legitimate read, only on a malformed one.
pub const MAX_ABAC_ROWS: usize = 1000;

/// Hard cap on one batch's total row-payload bytes (review finding B4-02).
/// `MAX_ABAC_ROWS` bounds row *count*, not bytes: the engine lowers every
/// payload byte into its own `wasmtime::component::Val` (the dynamic `Val`
/// API has no raw-bytes fast path), and `Val`'s largest variant is ~40
/// bytes, so an unbounded per-record payload turns into unbounded transient
/// host memory at up to ~40x the wire size, per concurrent read. 16 MiB
/// keeps a full `MAX_ABAC_ROWS`-row batch of realistically-sized records
/// well under the memory a single after-step call should ever transiently
/// hold, while staying far above any legitimate single-page payload.
pub const MAX_ABAC_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

/// Everything the guest-exported `authorize-rows` after-step needs to know
/// about the call it is being asked to judge, mirroring the WIT
/// `auth-context` fields of the same name (`wit/data-layer/authorizer.wit`):
/// without them a guest cannot tell which collection or which rule it is
/// being asked about, since one export serves every opted-in permission on
/// every collection (ADR-0017 §7).
#[derive(Debug, Clone)]
pub struct AbacAuthContext {
    pub collection: String,
    pub permissions: Vec<String>,
    pub subject_did: String,
    pub anchor_did: Option<String>,
    /// `with::can` strings, the same shape `DecisionTrace.held` uses --
    /// never the caveats, which can carry row-level data.
    pub capabilities: Vec<String>,
    /// `session.claims`, serialized. A JSON string rather than
    /// `list<tuple<string,string>>` because claims are typed scalars
    /// (`serde_json::Map<String, Value>`) that a string-pair list would
    /// silently flatten.
    pub claims_json: String,
}

impl AbacAuthContext {
    #[must_use]
    pub fn build(session: &SessionContext, collection: &str, permissions: &[String]) -> Self {
        Self {
            collection: collection.to_string(),
            permissions: permissions.to_vec(),
            subject_did: session.subject_did.clone(),
            anchor_did: session.anchor_did.clone(),
            capabilities: session
                .capabilities
                .iter()
                .map(|cap| format!("{}::{}", cap.with.0, cap.can.0))
                .collect(),
            claims_json: serde_json::Value::Object(session.claims.clone()).to_string(),
        }
    }
}

/// One sieve-admitted row, handed to the after-step as a candidate.
#[derive(Debug, Clone)]
pub struct CandidateRow {
    pub id: String,
    pub payload: Vec<u8>,
    pub creator_id: String,
    pub created_at: u64,
    pub updated_at: u64,
}

/// One after-step decision, positionally aligned to the [`CandidateRow`] it
/// judges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowDecision {
    Allow,
    Deny,
    /// Top-level payload keys to strip. Dotted paths are rejected (H3): a
    /// dotted entry would silently mask nothing, so it denies the row.
    Redact(Vec<String>),
}

#[derive(Debug, thiserror::Error)]
pub enum AbacError {
    /// No `RowAuthorizer` was available to run the after-step at all --
    /// either the service isn't WASM-backed/wired (the original meaning of
    /// this variant), or the throw-away instance's own instantiation failed
    /// (e.g. the wasmtime pooling allocator's instance budget was exhausted,
    /// review finding B4-01). Both are resource-availability failures, not
    /// an authorization decision -- callers map this to a distinguishable
    /// error, never to "zero rows" (B4-04).
    #[error(
        "no row authorizer is available for service '{0}' (unwired service, or the after-step \
         instance could not be started)"
    )]
    Unavailable(String),
    #[error("service '{0}' does not export syneroym:data-layer/authorizer#authorize-rows")]
    MissingExport(String),
    #[error("stage-4 after-step for '{service}' exceeded its budget: {detail}")]
    BudgetExceeded { service: String, detail: String },
    #[error("stage-4 after-step for '{service}' trapped: {detail}")]
    Trap { service: String, detail: String },
    #[error("stage-4 after-step returned {got} decisions for {expected} rows")]
    ArityMismatch { expected: usize, got: usize },
    #[error("stage-4 after-step returned a malformed decision: {0}")]
    Malformed(String),
    #[error("batch of {0} rows exceeds the {MAX_ABAC_ROWS}-row cap")]
    BatchTooLarge(usize),
    #[error("batch payload of {bytes} bytes exceeds the {MAX_ABAC_PAYLOAD_BYTES}-byte cap")]
    PayloadTooLarge { bytes: usize },
}

/// Invokes a service's guest-exported stage-4 after-step (ADR-0017 §7). The
/// sole implementation is `AppSandboxEngine` (`crates/sandbox_wasm`); this
/// trait lives here, not there, so `crates/control_plane`'s
/// `synsvc_native.rs` -- which never names `AppSandboxEngine` -- can call
/// through it without gaining a WASM dependency.
#[async_trait::async_trait]
pub trait RowAuthorizer: Send + Sync + fmt::Debug {
    /// Invokes `service_id`'s guest-exported `authorize-rows` over one
    /// batch. Returns exactly `rows.len()` decisions, positionally aligned;
    /// any other outcome is an `AbacError` the caller must treat as a
    /// whole-batch deny.
    async fn authorize_rows(
        &self,
        service_id: &str,
        ctx: &AbacAuthContext,
        rows: &[CandidateRow],
    ) -> Result<Vec<RowDecision>, AbacError>;
}

/// An always-empty `Weak<dyn RowAuthorizer>` (`.upgrade()` always returns
/// `None`) -- used for contexts with no WASM engine (coordinator mode,
/// tests). Same unsized-coercion trick as
/// `host_capabilities::empty_service_proxy`: the inherent `Weak::new()` only
/// exists for `T: Sized`, so an unsized `Weak<dyn RowAuthorizer>` has to be
/// produced via Rust's unsized coercion from a concrete, never-instantiated
/// marker type instead.
#[must_use]
pub fn empty_row_authorizer() -> Weak<dyn RowAuthorizer> {
    #[derive(Debug)]
    struct NeverConstructed;
    #[async_trait::async_trait]
    impl RowAuthorizer for NeverConstructed {
        async fn authorize_rows(
            &self,
            _service_id: &str,
            _ctx: &AbacAuthContext,
            _rows: &[CandidateRow],
        ) -> Result<Vec<RowDecision>, AbacError> {
            unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
        }
    }
    Weak::<NeverConstructed>::new()
}

/// Runs the stage-4 after-step over `rows` when `sieve` opts in, and returns
/// the surviving rows each paired with the **extra** field names to strip
/// (the after-step's `redact` set; the caller unions this with the sieve's
/// own `masked_fields` before projecting).
///
/// **Restrict-only by construction**: the guest is only ever shown rows the
/// sieve already admitted and can only answer per-position, so there is no
/// representation for "also allow row X". A decision list that doesn't align
/// 1:1 with `rows` is the one shape that could smuggle an extra row in, and
/// it denies the entire batch.
///
/// **Fail-closed on everything**: missing export, unavailable authorizer,
/// budget overrun, trap, malformed decision, over-large batch -- all return
/// `Err`, which every caller maps to "no rows" (Mode B) / `false` (Mode A),
/// never to "unfiltered".
pub async fn apply_stage4(
    sieve: &CompiledSieve,
    session: &SessionContext,
    service_id: &str,
    collection: &str,
    authorizer: Option<Arc<dyn RowAuthorizer>>,
    rows: Vec<CandidateRow>,
) -> Result<Vec<(CandidateRow, Vec<String>)>, AbacError> {
    if sieve.abac_permissions.is_empty() {
        return Ok(rows.into_iter().map(|r| (r, Vec::new())).collect());
    }

    let mut trace = AbacTrace {
        permissions: sieve.abac_permissions.clone(),
        rows_in: rows.len(),
        rows_denied: 0,
        rows_redacted: 0,
        failed_closed: None,
    };

    if rows.len() > MAX_ABAC_ROWS {
        trace.failed_closed = Some(format!("batch of {} rows exceeds the cap", rows.len()));
        trace.emit(collection, service_id, &session.subject_did);
        return Err(AbacError::BatchTooLarge(rows.len()));
    }

    let payload_bytes: usize = rows.iter().map(|r| r.payload.len()).sum();
    if payload_bytes > MAX_ABAC_PAYLOAD_BYTES {
        trace.failed_closed =
            Some(format!("batch payload of {payload_bytes} bytes exceeds the cap"));
        trace.emit(collection, service_id, &session.subject_did);
        return Err(AbacError::PayloadTooLarge { bytes: payload_bytes });
    }

    let Some(auth) = authorizer else {
        trace.failed_closed = Some("no row authorizer available".to_string());
        trace.emit(collection, service_id, &session.subject_did);
        return Err(AbacError::Unavailable(service_id.to_string()));
    };

    if rows.is_empty() {
        trace.emit(collection, service_id, &session.subject_did);
        return Ok(Vec::new());
    }

    let ctx = AbacAuthContext::build(session, collection, &sieve.abac_permissions);
    let decisions = match time::timeout(
        FDAE_ABAC_TIMEOUT,
        auth.authorize_rows(service_id, &ctx, &rows),
    )
    .await
    {
        Ok(Ok(decisions)) => decisions,
        Ok(Err(err)) => {
            trace.failed_closed = Some(err.to_string());
            trace.emit(collection, service_id, &session.subject_did);
            return Err(err);
        }
        Err(_) => {
            trace.failed_closed = Some(format!("exceeded {FDAE_ABAC_TIMEOUT:?}"));
            trace.emit(collection, service_id, &session.subject_did);
            return Err(AbacError::BudgetExceeded {
                service: service_id.to_string(),
                detail: format!("exceeded {FDAE_ABAC_TIMEOUT:?}"),
            });
        }
    };

    if decisions.len() != rows.len() {
        trace.failed_closed =
            Some(format!("expected {} decisions, got {}", rows.len(), decisions.len()));
        trace.emit(collection, service_id, &session.subject_did);
        return Err(AbacError::ArityMismatch { expected: rows.len(), got: decisions.len() });
    }

    let mut kept = Vec::with_capacity(rows.len());
    for (row, decision) in rows.into_iter().zip(decisions) {
        match decision {
            RowDecision::Allow => kept.push((row, Vec::new())),
            RowDecision::Deny => trace.rows_denied += 1,
            RowDecision::Redact(fields) => {
                if fields.iter().any(|f| f.contains('.')) {
                    trace.failed_closed = Some("redact named a dotted path".to_string());
                    trace.emit(collection, service_id, &session.subject_did);
                    return Err(AbacError::Malformed("redact named a dotted path".to_string()));
                }
                trace.rows_redacted += 1;
                kept.push((row, fields));
            }
        }
    }
    trace.emit(collection, service_id, &session.subject_did);
    Ok(kept)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use std::sync::Mutex;

    use syneroym_fdae::DecisionTrace;

    use super::*;

    fn sieve(abac_permissions: Vec<String>) -> CompiledSieve {
        CompiledSieve {
            where_clause: "1=1".to_string(),
            params: Vec::new(),
            masked_fields: Vec::new(),
            where_caveats: Vec::new(),
            trace: DecisionTrace::default(),
            abac_permissions,
        }
    }

    fn session() -> SessionContext {
        SessionContext { subject_did: "did:key:alice".to_string(), ..Default::default() }
    }

    fn row(id: &str) -> CandidateRow {
        CandidateRow {
            id: id.to_string(),
            payload: br#"{"a":1}"#.to_vec(),
            creator_id: "did:key:alice".to_string(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[derive(Debug)]
    struct StubAuthorizer {
        response: Mutex<Option<Result<Vec<RowDecision>, AbacError>>>,
    }

    impl StubAuthorizer {
        fn new(response: Result<Vec<RowDecision>, AbacError>) -> Arc<Self> {
            Arc::new(Self { response: Mutex::new(Some(response)) })
        }
    }

    #[async_trait::async_trait]
    impl RowAuthorizer for StubAuthorizer {
        async fn authorize_rows(
            &self,
            _service_id: &str,
            _ctx: &AbacAuthContext,
            _rows: &[CandidateRow],
        ) -> Result<Vec<RowDecision>, AbacError> {
            self.response.lock().unwrap().take().expect("authorize_rows invoked more than once")
        }
    }

    #[derive(Debug)]
    struct HangingAuthorizer;

    #[async_trait::async_trait]
    impl RowAuthorizer for HangingAuthorizer {
        async fn authorize_rows(
            &self,
            _service_id: &str,
            _ctx: &AbacAuthContext,
            _rows: &[CandidateRow],
        ) -> Result<Vec<RowDecision>, AbacError> {
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn apply_stage4_is_a_pass_through_when_no_permission_opts_in() {
        let s = sieve(Vec::new());
        let rows = vec![row("r1"), row("r2")];
        // No authorizer configured at all -- proves the empty-`abac_permissions`
        // path never even consults `authorizer`.
        let kept = apply_stage4(&s, &session(), "svc", "docs", None, rows.clone()).await.unwrap();
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].0.id, "r1");
        assert!(kept[0].1.is_empty());
    }

    #[tokio::test]
    async fn arity_mismatch_denies_the_whole_batch() {
        let s = sieve(vec!["view".to_string()]);
        let authorizer = StubAuthorizer::new(Ok(vec![RowDecision::Allow]));
        let rows = vec![row("r1"), row("r2")];
        let err =
            apply_stage4(&s, &session(), "svc", "docs", Some(authorizer), rows).await.unwrap_err();
        assert!(matches!(err, AbacError::ArityMismatch { expected: 2, got: 1 }));
    }

    #[tokio::test]
    async fn deny_drops_only_its_own_row() {
        let s = sieve(vec!["view".to_string()]);
        let authorizer = StubAuthorizer::new(Ok(vec![
            RowDecision::Allow,
            RowDecision::Deny,
            RowDecision::Allow,
        ]));
        let rows = vec![row("r1"), row("r2"), row("r3")];
        let kept =
            apply_stage4(&s, &session(), "svc", "docs", Some(authorizer), rows).await.unwrap();
        let ids: Vec<&str> = kept.iter().map(|(r, _)| r.id.as_str()).collect();
        assert_eq!(ids, vec!["r1", "r3"]);
    }

    #[tokio::test]
    async fn redact_returns_exactly_the_decided_fields_never_subtracting_them() {
        let s = sieve(vec!["view".to_string()]);
        let authorizer =
            StubAuthorizer::new(Ok(vec![RowDecision::Redact(vec!["ssn".to_string()])]));
        let kept = apply_stage4(&s, &session(), "svc", "docs", Some(authorizer), vec![row("r1")])
            .await
            .unwrap();
        assert_eq!(kept.len(), 1, "a redacted row is still kept, not dropped");
        assert_eq!(kept[0].1, vec!["ssn".to_string()]);
    }

    #[tokio::test]
    async fn a_dotted_redact_path_denies_the_row() {
        let s = sieve(vec!["view".to_string()]);
        let authorizer =
            StubAuthorizer::new(Ok(vec![RowDecision::Redact(vec!["profile.ssn".to_string()])]));
        let err = apply_stage4(&s, &session(), "svc", "docs", Some(authorizer), vec![row("r1")])
            .await
            .unwrap_err();
        assert!(matches!(err, AbacError::Malformed(_)));
    }

    #[tokio::test]
    async fn an_unavailable_authorizer_denies_closed() {
        let s = sieve(vec!["view".to_string()]);
        let err =
            apply_stage4(&s, &session(), "svc", "docs", None, vec![row("r1")]).await.unwrap_err();
        assert!(matches!(err, AbacError::Unavailable(_)));
    }

    #[tokio::test]
    async fn an_over_large_batch_denies_closed() {
        let s = sieve(vec!["view".to_string()]);
        let authorizer = StubAuthorizer::new(Ok(Vec::new()));
        let rows: Vec<CandidateRow> = (0..MAX_ABAC_ROWS + 1).map(|i| row(&i.to_string())).collect();
        let err =
            apply_stage4(&s, &session(), "svc", "docs", Some(authorizer), rows).await.unwrap_err();
        assert!(matches!(err, AbacError::BatchTooLarge(_)));
    }

    #[tokio::test]
    async fn an_over_large_payload_denies_closed() {
        let s = sieve(vec!["view".to_string()]);
        let authorizer = StubAuthorizer::new(Ok(vec![RowDecision::Allow]));
        let big_row = CandidateRow {
            id: "big".to_string(),
            payload: vec![0u8; MAX_ABAC_PAYLOAD_BYTES + 1],
            creator_id: "did:key:alice".to_string(),
            created_at: 0,
            updated_at: 0,
        };
        let err = apply_stage4(&s, &session(), "svc", "docs", Some(authorizer), vec![big_row])
            .await
            .unwrap_err();
        assert!(matches!(err, AbacError::PayloadTooLarge { .. }));
    }

    #[tokio::test(start_paused = true)]
    async fn an_elapsed_timeout_denies_closed() {
        let s = sieve(vec!["view".to_string()]);
        let authorizer = Arc::new(HangingAuthorizer);
        let err = apply_stage4(&s, &session(), "svc", "docs", Some(authorizer), vec![row("r1")])
            .await
            .unwrap_err();
        assert!(matches!(err, AbacError::BudgetExceeded { .. }));
    }
}
