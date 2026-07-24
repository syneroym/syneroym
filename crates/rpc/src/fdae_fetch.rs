//! Slice B3 Phase 4: the orchestration seam that actually performs a
//! `crates/fdae`-planned remote relationship-proof fetch (pipeline stage 2,
//! ADR-0017 §6). Deliberately outside `crates/fdae` (kept proxy-free, plan
//! §1.1) and outside `crates/data_db` (no `ServiceProxy` dependency there
//! either) -- this is the one place both read ingresses (the WASM host
//! path, native dispatch) share, per plan §4's "a small `fdae-runtime`-style
//! helper... not a new crate" recommendation.

use std::time::Duration;

use syneroym_fdae::{FetchResult, RemoteFetch, RemoteFetchTrace};
use tokio::time;

use crate::{
    CallOrigin, CallerContext, ProxyError, ProxyProtocol, ProxyRequest, ServiceProxy,
    relationship_proof::{RelationshipProof, RelationshipProofError},
};

/// A conservative ceiling for one cross-service relationship-proof fetch.
/// Well above the `< 50 ms p99` *floor* the perf budget targets for a
/// healthy hop (`task.md`), but still bounded -- a fetch that overruns this
/// denies the whole read closed rather than hang the caller indefinitely.
pub const FDAE_FETCH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("fetching relation '{relation}' on service '{service}' failed: {source}")]
    Proxy { service: String, relation: String, source: ProxyError },
    #[error(
        "relation '{relation}' on service '{service}' returned a malformed relationship proof: \
         {source}"
    )]
    MalformedProof { service: String, relation: String, source: serde_json::Error },
    #[error(
        "relationship proof for relation '{relation}' on service '{service}' failed verification: \
         {source}"
    )]
    ProofInvalid { service: String, relation: String, source: RelationshipProofError },
    /// The remote answered a *different* relation or principal than asked
    /// -- either a receiving-side bug, or a replay of an old, still
    /// signature-valid proof answering the wrong question. The signature
    /// alone can't catch this (it only proves *a* legitimate signer
    /// produced *some* self-consistent record), so it's checked separately.
    #[error(
        "relation '{relation}' on service '{service}' answered a different relation/principal \
         than requested"
    )]
    MismatchedAnswer { service: String, relation: String },
}

/// Issues every `RemoteFetch` a `crates/fdae` `plan_read` collected,
/// **sequentially** (Phase 4 scope; parallelizing distinct fetches is a
/// pure perf follow-up, not a correctness dependency -- see plan §5/D-B3-6's
/// own "correctness over scale" precedent), and returns the corresponding
/// `FetchResult`s ready for `syneroym_fdae::finalize`.
///
/// `caller` is forwarded **unmodified** (D-B3-9): `ProxyRequest.caller`
/// carries the local invocation's own already-verified `CallerContext`
/// (proof intact), not a synthesized identity. `invoke_remote_at` already
/// forwards `caller.proof` verbatim for any `CallOrigin::Native` call
/// (`router/src/proxy.rs`), so the remote's own `verify_chain` re-derives
/// `subject_did`/`anchor_did` from the real chain and A1 gates on whatever
/// was legitimately delegated through it -- no new proof-forwarding
/// mechanism needed.
///
/// **Fails closed on the first error**: a timeout, transport error, or a
/// proof that doesn't verify against its `RemoteFetch.expected_asserter_did`
/// (D-B3-8) aborts the whole batch. The caller (the WASM host path or
/// native dispatch) must treat any `Err` here as a deny -- Mode B empty,
/// Mode A `false` -- never as "no fetches needed."
pub async fn resolve_fetches(
    fetches: &[RemoteFetch],
    caller: &CallerContext,
    proxy: &dyn ServiceProxy,
) -> Result<Vec<FetchResult>, FetchError> {
    let mut results = Vec::with_capacity(fetches.len());
    for fetch in fetches {
        results.push(resolve_one_fetch(fetch, caller, proxy).await?);
    }
    Ok(results)
}

async fn resolve_one_fetch(
    fetch: &RemoteFetch,
    caller: &CallerContext,
    proxy: &dyn ServiceProxy,
) -> Result<FetchResult, FetchError> {
    let request = ProxyRequest {
        target_service: fetch.service.clone(),
        interface: "data-layer".to_string(),
        method: "resolve-relation".to_string(),
        params: serde_json::json!({ "relation": fetch.relation, "principal": fetch.principal_did }),
        caller: caller.clone(),
        origin: CallOrigin::Native,
        protocol: ProxyProtocol::default(),
        // Read-only, idempotent by construction -- safe for the proxy's own
        // transport-failure retry loop.
        idempotent: true,
        timeout: Some(FDAE_FETCH_TIMEOUT),
    };
    // `request.timeout` above is *advisory* -- it's honored by the
    // production `ProxyRouter`, but `proxy` here is any `dyn ServiceProxy`,
    // and nothing stops a different implementation from ignoring the field.
    // This crate's own fail-closed contract must not depend on that: wrap
    // the call in a deadline it enforces itself, so a `ServiceProxy` that
    // never returns still denies closed after `FDAE_FETCH_TIMEOUT` rather
    // than hanging the read indefinitely.
    let response = time::timeout(FDAE_FETCH_TIMEOUT, proxy.invoke(request))
        .await
        .map_err(|_| FetchError::Proxy {
            service: fetch.service.clone(),
            relation: fetch.relation.clone(),
            source: ProxyError::Timeout(FDAE_FETCH_TIMEOUT),
        })?
        .map_err(|source| FetchError::Proxy {
            service: fetch.service.clone(),
            relation: fetch.relation.clone(),
            source,
        })?;
    let proof: RelationshipProof =
        serde_json::from_value(response).map_err(|source| FetchError::MalformedProof {
            service: fetch.service.clone(),
            relation: fetch.relation.clone(),
            source,
        })?;
    proof.verify(&fetch.expected_asserter_did).map_err(|source| FetchError::ProofInvalid {
        service: fetch.service.clone(),
        relation: fetch.relation.clone(),
        source,
    })?;
    if proof.relation != fetch.relation || proof.principal != fetch.principal_did {
        return Err(FetchError::MismatchedAnswer {
            service: fetch.service.clone(),
            relation: fetch.relation.clone(),
        });
    }
    Ok(FetchResult {
        slot: fetch.slot,
        ids: proof.ids.clone(),
        trace: RemoteFetchTrace {
            service: fetch.service.clone(),
            relation: fetch.relation.clone(),
            principal_did: fetch.principal_did.clone(),
            asserter_did: proof.asserter_did,
            valid_until_secs: proof.valid_until_secs,
        },
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use std::{
        future,
        sync::{Arc, Mutex},
    };

    use syneroym_fdae::Mode;
    use syneroym_identity::{Identity, substrate};
    use syneroym_ucan::{Ability, SessionContext};

    use super::*;
    use crate::AuthLevel;

    fn test_caller() -> CallerContext {
        CallerContext {
            caller_did: "did:key:svc-a".to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: "did:key:svc-a".to_string(),
                anchor_did: Some("did:key:alice".to_string()),
                ..Default::default()
            },
            auth: AuthLevel::Ucan,
            proof: None,
        }
    }

    /// `FetchSlot`'s inner field is private to `crates/fdae` (correctly --
    /// it's an opaque correlation key), so a real `RemoteFetch` for this
    /// test is obtained the only way an external crate can: through a real
    /// `plan_read` call against a minimal remote-relation policy, exactly
    /// as the production `resolve_fetches` caller would.
    fn fetch(expected_asserter_did: &str) -> RemoteFetch {
        let policy = syneroym_fdae::parse_and_validate(&format!(
            r#"{{
                "version": "fdae/v1",
                "definitions": {{
                    "document": {{
                        "table": "documents",
                        "relations": {{"owner": {{
                            "target": "employee", "service": "hr-svc",
                            "join_column": "owner_uuid",
                            "expected_asserter_did": "{expected_asserter_did}"
                        }}}},
                        "permissions": {{
                            "view": {{"allows": ["data-layer/read"], "paths": [["owner", "anchor"]]}}
                        }}
                    }}
                }}
            }}"#
        ))
        .unwrap();
        let session = SessionContext {
            subject_did: "did:key:svc-a".to_string(),
            anchor_did: Some("did:key:alice".to_string()),
            capabilities: vec![syneroym_ucan::Capability {
                with: syneroym_ucan::ResourceUri(
                    "synapp:local-svc:svc:local-svc/collection/document".to_string(),
                ),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        };
        let plan = syneroym_fdae::plan_read(
            &policy,
            "document",
            &session,
            "local-svc",
            &Ability(Ability::DATA_LAYER_READ.to_string()),
            Mode::Filter,
        )
        .unwrap();
        plan.fetches[0].clone()
    }

    #[derive(Debug)]
    struct StubProxy {
        response: Mutex<Option<Result<serde_json::Value, String>>>,
        received: Mutex<Option<ProxyRequest>>,
    }

    #[async_trait::async_trait]
    impl ServiceProxy for StubProxy {
        async fn invoke(&self, request: ProxyRequest) -> Result<serde_json::Value, ProxyError> {
            *self.received.lock().unwrap() = Some(request);
            match self.response.lock().unwrap().take() {
                Some(Ok(v)) => Ok(v),
                Some(Err(msg)) => {
                    Err(ProxyError::Callee { code: -32000, message: msg, data: None })
                }
                None => panic!("StubProxy invoked with no response configured"),
            }
        }
    }

    #[tokio::test]
    async fn resolve_fetches_forwards_the_callers_own_context_and_origin_native() {
        let identity = Identity::generate().unwrap();
        let asserter_did = substrate::derive_did_key(&identity.public_key());
        let proof = RelationshipProof::sign(
            &identity,
            "employee",
            "did:key:alice",
            vec!["emp-1".to_string()],
        )
        .unwrap();
        let stub = Arc::new(StubProxy {
            response: Mutex::new(Some(Ok(serde_json::to_value(&proof).unwrap()))),
            received: Mutex::new(None),
        });
        let caller = test_caller();
        let results =
            resolve_fetches(&[fetch(&asserter_did)], &caller, stub.as_ref()).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].ids, vec!["emp-1".to_string()]);
        assert_eq!(results[0].trace.asserter_did, asserter_did);

        let received = stub.received.lock().unwrap().take().unwrap();
        assert!(matches!(received.origin, CallOrigin::Native));
        assert_eq!(received.caller.caller_did, caller.caller_did);
        assert_eq!(received.caller.session.anchor_did, caller.session.anchor_did);
    }

    #[tokio::test]
    async fn resolve_fetches_denies_on_a_proxy_error() {
        let stub = Arc::new(StubProxy {
            response: Mutex::new(Some(Err("boom".to_string()))),
            received: Mutex::new(None),
        });
        let caller = test_caller();
        let err = resolve_fetches(&[fetch("did:key:zAnything")], &caller, stub.as_ref())
            .await
            .unwrap_err();
        assert!(matches!(err, FetchError::Proxy { .. }));
    }

    /// A `ServiceProxy` that never resolves -- `request.timeout` is only
    /// advisory (honored by the production `ProxyRouter`, not enforced by
    /// this crate), so `resolve_one_fetch` must enforce `FDAE_FETCH_TIMEOUT`
    /// itself. Distinct from `resolve_fetches_denies_on_a_proxy_error`
    /// above, which tests an immediate error, not an elapsed deadline.
    #[derive(Debug)]
    struct StallingProxy;

    #[async_trait::async_trait]
    impl ServiceProxy for StallingProxy {
        async fn invoke(&self, _request: ProxyRequest) -> Result<serde_json::Value, ProxyError> {
            future::pending().await
        }
    }

    // Paused virtual time: tokio auto-advances the clock to the next timer
    // once the test task is blocked on nothing but that timer, so this
    // resolves instantly in wall-clock terms despite waiting out the full
    // `FDAE_FETCH_TIMEOUT` in virtual time.
    #[tokio::test(start_paused = true)]
    async fn resolve_fetches_denies_on_an_actually_elapsed_timeout() {
        let caller = test_caller();
        let err = resolve_fetches(&[fetch("did:key:zAnything")], &caller, &StallingProxy)
            .await
            .unwrap_err();
        match err {
            FetchError::Proxy { source: ProxyError::Timeout(d), .. } => {
                assert_eq!(d, FDAE_FETCH_TIMEOUT);
            }
            other => panic!(
                "a proxy that never resolves must be denied by this crate's own deadline, not \
                 hang: {other:?}"
            ),
        }
    }

    #[tokio::test]
    async fn resolve_fetches_denies_on_an_asserter_mismatch() {
        let identity = Identity::generate().unwrap();
        let proof = RelationshipProof::sign(
            &identity,
            "employee",
            "did:key:alice",
            vec!["emp-1".to_string()],
        )
        .unwrap();
        let stub = Arc::new(StubProxy {
            response: Mutex::new(Some(Ok(serde_json::to_value(&proof).unwrap()))),
            received: Mutex::new(None),
        });
        let caller = test_caller();
        let err = resolve_fetches(&[fetch("did:key:zSomeoneElse")], &caller, stub.as_ref())
            .await
            .unwrap_err();
        assert!(matches!(err, FetchError::ProofInvalid { .. }));
    }

    #[tokio::test]
    async fn resolve_fetches_denies_on_a_mismatched_relation_answer() {
        let identity = Identity::generate().unwrap();
        let asserter_did = substrate::derive_did_key(&identity.public_key());
        // A validly-signed proof, but for a *different* relation than asked.
        let proof =
            RelationshipProof::sign(&identity, "team", "did:key:alice", vec!["team-1".to_string()])
                .unwrap();
        let stub = Arc::new(StubProxy {
            response: Mutex::new(Some(Ok(serde_json::to_value(&proof).unwrap()))),
            received: Mutex::new(None),
        });
        let caller = test_caller();
        let err =
            resolve_fetches(&[fetch(&asserter_did)], &caller, stub.as_ref()).await.unwrap_err();
        assert!(matches!(err, FetchError::MismatchedAnswer { .. }));
    }
}
