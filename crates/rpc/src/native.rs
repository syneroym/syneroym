//! Native service execution bridge
//!
//! Provides the core abstraction and types for in-process, native Rust
//! services, permitting local request dispatching within the substrate.

use std::fmt::Debug;

use serde_json::Value;
use syneroym_ucan::{Ability, ResourceUri, SessionContext};

use crate::RpcResult;

/// A verified caller identity, threaded through every native dispatch
/// (ADR-0016 §2). `CallerContext` is always locally constructed from a
/// verified handshake or a substrate-injected lifecycle context — it is
/// never serialized into, nor deserialized out of, the wire. A cross-node
/// proxy hop (M04A Slice A1) carries the caller's DID and its signed proofs
/// in the request envelope metadata instead, and the data-owning
/// (destination) node re-verifies those proofs and constructs a fresh
/// `CallerContext` locally before dispatch (ADR-0016 §6).
#[derive(Clone, Debug)]
pub struct CallerContext {
    /// Verified `did:key` of the immediate caller.
    pub caller_did: String,
    /// App-instance the caller acts as (`creator_id`). `None` on the raw B0
    /// path. Names the per-app identity, but does **not** drive per-app KEK
    /// selection: M04A Slice B6 derives each service's KEK from the bound
    /// `service_id` (== this identity today) at the storage layer, not by
    /// reading this field — see `syneroym_data_keystore::key_store`.
    pub app_instance: Option<String>,
    /// Verified capabilities/claims. Empty unless the interim admin-root
    /// path (B0) or a real UCAN chain (B1) populated it.
    pub session: SessionContext,
    pub auth: AuthLevel,
    /// Signed, forwardable proof of this caller's identity (M04A Slice A1,
    /// ADR-0016 §6) -- verbatim what the inbound preamble carried. A
    /// cross-node proxy hop (`syneroym-router`'s `ProxyRouter`) re-presents
    /// it on the outbound preamble; the destination re-verifies it with
    /// `HandshakeVerifier::verify_preamble` and builds a **fresh**
    /// `CallerContext` -- capabilities themselves never cross the wire, only
    /// this proof does. `None` for substrate-injected callers
    /// (`local_elevated`/`service_system`), which therefore cannot be
    /// impersonated across a hop.
    pub proof: Option<CallerProof>,
}

/// See [`CallerContext::proof`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallerProof {
    /// Hex-encoded ed25519 public key (the preamble's `pubkey=`).
    pub pubkey_hex: String,
    /// JSON `DelegationCertificate` (the preamble's `delegation=`, pre-hex).
    pub delegation_json: Option<String>,
}

/// How a `CallerContext` was established.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthLevel {
    /// Verified `DelegationCertificate` only (pre-UCAN / transport identity).
    Delegated,
    /// Full verified UCAN capability chain (B1).
    Ucan,
    /// Substrate-injected lifecycle context (init/migrate), carrying
    /// `data-layer/admin` on the service's own resource.
    LocalElevated,
    /// Substrate-injected stage-4 ABAC context (ADR-0017 §7): the service
    /// acting as itself for the after-step. Carries **no** capabilities and
    /// is exempt from the FDAE sieve, per §7's "the escape hatches run under
    /// the service's own identity". Read-only-ness comes from
    /// `HostState.read_only`, not from a capability -- host write paths
    /// carry no capability gate of their own (D-04-02-f), so a narrower
    /// capability would enforce nothing. Distinct from capability-less
    /// `System` only because the sieve exemption keys on this level, and
    /// exempting `System` would re-open D-04-02-h's ingress-(ii) bypass.
    LocalReadOnly,
    /// Substrate-injected system context (a service acting as itself, or an
    /// already-authorized internal dispatch), not derived from a wire
    /// handshake. Carries no elevated capabilities, unlike `LocalElevated`.
    System,
}

impl CallerContext {
    /// Admin gate helper (ADR-0016 §5).
    #[must_use]
    pub fn has_capability(&self, resource: &ResourceUri, ability: &Ability) -> bool {
        self.session.has_capability(resource, ability)
    }

    /// Substrate-injected lifecycle identity (init/migrate) bearing
    /// `data-layer/admin` on `service_id`'s own resource.
    #[must_use]
    pub fn local_elevated(service_id: &str) -> Self {
        use syneroym_ucan::Capability;

        let resource = ResourceUri::service(service_id, service_id);
        Self {
            caller_did: format!("system:local-elevated:{service_id}"),
            app_instance: None,
            session: SessionContext {
                subject_did: format!("system:local-elevated:{service_id}"),
                capabilities: vec![Capability {
                    with: resource,
                    can: Ability(Ability::DATA_LAYER_ADMIN.to_string()),
                    caveats: None,
                }],
                ..Default::default()
            },
            auth: AuthLevel::LocalElevated,
            proof: None,
        }
    }

    /// A service-scoped system caller for already-authorized internal
    /// dispatches (e.g. the HMAC-signed blob GET path, or a component
    /// acting as itself). Carries no elevated capabilities.
    #[must_use]
    pub fn service_system(service_id: &str) -> Self {
        Self {
            caller_did: format!("system:{service_id}"),
            app_instance: None,
            session: SessionContext {
                subject_did: format!("system:{service_id}"),
                ..Default::default()
            },
            auth: AuthLevel::System,
            proof: None,
        }
    }

    /// Substrate-injected stage-4 ABAC identity. Never constructible from
    /// guest or wire input: `AppSandboxEngine::authorize_rows` is the sole
    /// producer, exactly as `invoke_lifecycle_hook` is for `local_elevated`.
    /// Deliberately capability-less -- see `AuthLevel::LocalReadOnly`.
    #[must_use]
    pub fn service_abac(service_id: &str) -> Self {
        Self {
            caller_did: format!("system:abac:{service_id}"),
            app_instance: None,
            session: SessionContext {
                subject_did: format!("system:abac:{service_id}"),
                ..Default::default()
            },
            auth: AuthLevel::LocalReadOnly,
            proof: None,
        }
    }

    /// The DID a row this caller writes should be attributed to. Synthesized
    /// substrate contexts have no external principal, so the service owns
    /// the row; everyone else is attributed to the principal they act for --
    /// `anchor_did` before `subject_did`, the same precedence
    /// `syneroym_fdae::compile`'s path-terminal resolution and
    /// `RemoteFetch.principal_did` use, so a row created through a proxying
    /// service is attributed to the principal it acts for, not the proxy.
    ///
    /// Deliberately does **not** consult `app_instance`: the compiled sieve's
    /// `WITH CHECK` half re-derives the same row's owning principal from
    /// `anchor_did`/`subject_did` alone (`terminal_value` has no
    /// `app_instance` terminal at all), so stamping `app_instance` here
    /// first would attribute a row to a DID the sieve can never match --
    /// failing a policy's own `WITH CHECK` on the very write that just
    /// passed its `USING` half. If `app_instance` ever needs to participate
    /// in attribution, the policy language needs an `app_instance` terminal
    /// first, so the two stay in lockstep by construction rather than by
    /// convention.
    #[must_use]
    pub fn write_attribution(&self, service_id: &str) -> String {
        match self.auth {
            AuthLevel::System | AuthLevel::LocalElevated | AuthLevel::LocalReadOnly => {
                service_id.to_string()
            }
            AuthLevel::Delegated | AuthLevel::Ucan => {
                self.session.anchor_did.clone().unwrap_or_else(|| self.session.subject_did.clone())
            }
        }
    }
}

/// Represents a parsed and validated request ready for dispatch to a native
/// service.
#[derive(Debug)]
pub struct NativeInvocation {
    pub interface: String,
    pub method: String,
    pub params: Value,
    pub caller: CallerContext,
}

/// Represents a response from a native service.
#[derive(Debug)]
pub struct NativeResponse {
    pub payload: Value,
}

#[async_trait::async_trait]
pub trait NativeService: Send + Sync + Debug {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caller(auth: AuthLevel, subject_did: &str, anchor_did: Option<&str>) -> CallerContext {
        CallerContext {
            caller_did: subject_did.to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: subject_did.to_string(),
                anchor_did: anchor_did.map(str::to_string),
                ..Default::default()
            },
            auth,
            proof: None,
        }
    }

    #[test]
    fn synthesized_substrate_contexts_attribute_to_the_service() {
        for auth in [AuthLevel::System, AuthLevel::LocalElevated, AuthLevel::LocalReadOnly] {
            let ctx = caller(auth, "system:whatever", None);
            assert_eq!(ctx.write_attribution("svc-a"), "svc-a");
        }
    }

    #[test]
    fn a_direct_caller_attributes_to_its_own_subject_did() {
        let ctx = caller(AuthLevel::Ucan, "did:key:zDirectCaller", None);
        assert_eq!(ctx.write_attribution("svc-a"), "did:key:zDirectCaller");
    }

    #[test]
    fn a_proxied_caller_attributes_to_the_anchor_not_the_immediate_subject() {
        let ctx = caller(
            AuthLevel::Delegated,
            "did:key:zProxyingService",
            Some("did:key:zRealPrincipal"),
        );
        assert_eq!(ctx.write_attribution("svc-a"), "did:key:zRealPrincipal");
    }
}
