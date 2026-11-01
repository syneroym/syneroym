#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Universal Proxy guest test component.
//!
//! Exercises `syneroym:proxy/proxy::call` from guest code end to end -- the
//! only way a real component can originate a cross-service call, as opposed
//! to a Rust-level `ProxyRouter::invoke` unit test. Also exports the
//! stage-4 ABAC after-step (ADR-0017 §7) so this same component can serve
//! as the self-proxy target for the router-side ingress-(ii)
//! integration test.

use std::time::{Duration, Instant};

use bindings::{
    exports::{
        syneroym::data_layer::authorizer::{
            AuthContext, CandidateRow, Guest as AuthorizerGuest, RowDecision,
        },
        syneroym_test::proxy_test::test_driver::{
            Guest as TestDriverGuest, PeerCall,
        },
    },
    syneroym::proxy::proxy,
};

mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "proxy-test",
        with: {
            "syneroym:proxy/proxy@0.1.0": generate,
            "syneroym:data-layer/authorizer@0.1.0": generate,
        },
    });

    use super::ProxyTestComponent;
    export!(ProxyTestComponent);
}

struct ProxyTestComponent;

impl TestDriverGuest for ProxyTestComponent {
    fn call_peer(
        service: String,
        interface: String,
        method: String,
        params: String,
        target_kind: String,
    ) -> Result<String, String> {
        let target = match target_kind.as_str() {
            "dependency" => proxy::CallTarget::Dependency(service),
            _ => proxy::CallTarget::Service(service),
        };
        proxy::call(&target, &interface, &method, &params, None).map_err(|e| format!("{e:?}"))
    }

    fn enqueue_peer(
        service: String,
        interface: String,
        method: String,
        params: String,
        target_kind: String,
        idempotency_key: String,
    ) -> Result<(), String> {
        let target = match target_kind.as_str() {
            "dependency" => proxy::CallTarget::Dependency(service),
            _ => proxy::CallTarget::Service(service),
        };
        let options = proxy::CallOptions {
            protocol: None,
            idempotent: false,
            timeout_ms: None,
            routing_key: None,
            // Empty means absent, so the host's own no-key refusal can be
            // driven from real guest code.
            idempotency_key: if idempotency_key.is_empty() {
                None
            } else {
                Some(idempotency_key)
            },
        };
        proxy::enqueue(&target, &interface, &method, &params, Some(&options))
            .map_err(|e| format!("{e:?}"))
    }

    fn call_peers(
        calls: Vec<(String, String, String, String, String)>,
    ) -> Result<Vec<Result<String, String>>, String> {
        let full_calls = calls
            .into_iter()
            .map(|(kind, service, interface, method, params)| PeerCall {
                kind,
                service,
                interface,
                method,
                params,
                timeout_ms: None,
                idempotency_key: None,
            })
            .collect();
        Self::call_peers_full(full_calls)
    }

    fn call_peers_full(
        calls: Vec<PeerCall>,
    ) -> Result<Vec<Result<String, String>>, String> {
        let requests: Vec<proxy::CallRequest> = calls
            .into_iter()
            .map(|call| {
                let target = match call.kind.as_str() {
                    "dependency" => proxy::CallTarget::Dependency(call.service),
                    _ => proxy::CallTarget::Service(call.service),
                };
                let options = if call.timeout_ms.is_some() || call.idempotency_key.is_some() {
                    Some(proxy::CallOptions {
                        protocol: None,
                        idempotent: false,
                        timeout_ms: call.timeout_ms,
                        routing_key: None,
                        idempotency_key: call.idempotency_key,
                    })
                } else {
                    None
                };
                proxy::CallRequest {
                    target,
                    interface: call.interface,
                    method: call.method,
                    params: call.params,
                    options,
                }
            })
            .collect();

        match proxy::call_all(&requests) {
            Ok(results) => Ok(results
                .into_iter()
                .map(|r| r.map_err(|e| format!("{e:?}")))
                .collect()),
            Err(e) => Err(format!("{e:?}")),
        }
    }

    fn spin_for(ms: u32) {
        let start = Instant::now();
        let target = Duration::from_millis(ms as u64);
        while start.elapsed() < target {
            core::hint::black_box(());
        }
    }
}

/// Deterministic, fixed behavior (no `app-config`-driven mode switching like
/// `abac-test`'s fixture -- this component only needs to prove ingress (ii)
/// actually invokes the after-step, not re-test every authorization case):
/// denies the row seeded with id `"secret"`, allows everything else.
impl AuthorizerGuest for ProxyTestComponent {
    fn authorize_rows(
        _ctx: AuthContext,
        rows: Vec<CandidateRow>,
    ) -> Result<Vec<RowDecision>, String> {
        Ok(rows
            .into_iter()
            .map(|row| if row.id == "secret" { RowDecision::Deny } else { RowDecision::Allow })
            .collect())
    }
}
