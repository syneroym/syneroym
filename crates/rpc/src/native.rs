//! Native service execution bridge
//!
//! Provides the core abstraction and types for in-process, native Rust
//! services, permitting local request dispatching within the substrate.

use std::fmt::{self, Debug};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use syneroym_ucan::{Ability, Capability, ResourceUri, SessionContext};

use crate::RpcResult;

/// A verified caller identity, threaded through every native dispatch
/// (ADR-0016 §2). `CallerContext` is always locally constructed from a
/// verified handshake or a substrate-injected lifecycle context — it is
/// never serialized into, nor deserialized out of, the wire. A cross-node
/// proxy hop carries the caller's DID and its signed proofs
/// in the request envelope metadata instead, and the data-owning
/// (destination) node re-verifies those proofs and constructs a fresh
/// `CallerContext` locally before dispatch (ADR-0016 §6).
#[derive(Clone, Debug)]
pub struct CallerContext {
    /// Verified `did:key` of the immediate caller.
    pub caller_did: String,
    /// App-instance the caller acts as (`creator_id`). `None` when no
    /// app-instance is bound. Names the per-app identity, but does **not**
    /// drive per-app KEK selection: the storage layer derives each service's
    /// KEK from the bound `service_id` (== this identity today), not by
    /// reading this field — see `syneroym_data_keystore::key_store`.
    pub app_instance: Option<String>,
    /// Verified capabilities/claims. Empty unless the interim admin-root
    /// path or a real UCAN chain populated it.
    pub session: SessionContext,
    pub auth: AuthLevel,
    /// Signed, forwardable proof of this caller's identity (ADR-0016 §6,
    /// Amendment 2). The destination re-verifies it; a node never presents
    /// it as its own identity. A native call made for this caller passes it
    /// on in `on_behalf` when it is signed.
    pub proof: Option<CallerProof>,
    /// The verified master DID of the person this caller presented an
    /// `on_behalf` proof for (ADR-0016 Amendment 3). Only `resolve-relation`
    /// reads it, to answer a question about that person. It is deliberately
    /// not `session.anchor_did`: the row policy and write attribution read
    /// the anchor, and a service the person called must not read or write
    /// the person's rows with this proof.
    pub on_behalf_did: Option<String>,
}

/// See [`CallerContext::proof`].
#[derive(Clone, Default, PartialEq, Eq)]
pub struct CallerProof {
    /// Hex Ed25519 public key (the preamble's `pubkey=`).
    pub pubkey_hex: String,
    /// JSON `DelegationCertificate` (the preamble's `delegation=`, pre-hex).
    pub delegation_json: Option<String>,
    /// What the caller signed on its stream (ADR-0002 A1). The router
    /// always sets it. `None` for a proof built on this node for a
    /// same-node call, which has no stream to sign.
    pub signed: Option<PreambleSignature>,
    /// The original caller's proof that this caller passed on
    /// (ADR-0016 Amendment 2). `None` when this caller is the original.
    pub on_behalf: Option<OnBehalfProof>,
}

/// The signed fields of a preamble, as the caller sent them.
#[derive(Clone, PartialEq, Eq)]
pub struct PreambleSignature {
    pub service_id: String,
    pub interface: String,
    pub ts: u64,
    /// Hex Ed25519 signature (the preamble's `sig=`).
    pub sig_hex: String,
}

/// The original caller's proof, as it travels in `on_behalf=`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnBehalfProof {
    pub pubkey: String,
    pub delegation: Option<String>,
    pub service_id: String,
    pub interface: String,
    pub ts: u64,
    pub sig: String,
}

impl CallerProof {
    /// The proof a native call made for this caller passes on: the one
    /// this caller carried, else this caller's own when it is signed.
    /// `None` for an unsigned local proof: a destination would refuse
    /// it, so it is not sent. Never wrapped.
    #[must_use]
    pub fn to_pass_on(&self) -> Option<OnBehalfProof> {
        if let Some(carried) = &self.on_behalf {
            return Some(carried.clone());
        }
        let signed = self.signed.as_ref()?;
        Some(OnBehalfProof {
            pubkey: self.pubkey_hex.clone(),
            delegation: self.delegation_json.clone(),
            service_id: signed.service_id.clone(),
            interface: signed.interface.clone(),
            ts: signed.ts,
            sig: signed.sig_hex.clone(),
        })
    }
}

impl fmt::Debug for PreambleSignature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreambleSignature")
            .field("service_id", &self.service_id)
            .field("interface", &self.interface)
            .field("ts", &self.ts)
            .field("sig", &"present")
            .finish()
    }
}

impl fmt::Debug for OnBehalfProof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let delegation = if self.delegation.is_some() { "present" } else { "absent" };
        f.debug_struct("OnBehalfProof")
            .field("pubkey", &self.pubkey)
            .field("delegation", &delegation)
            .field("service_id", &self.service_id)
            .field("interface", &self.interface)
            .field("ts", &self.ts)
            .field("sig", &"present")
            .finish()
    }
}

impl fmt::Debug for CallerProof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let delegation = if self.delegation_json.is_some() { "present" } else { "absent" };
        f.debug_struct("CallerProof")
            .field("pubkey_hex", &self.pubkey_hex)
            .field("delegation", &delegation)
            .field("signed", &self.signed)
            .field("on_behalf", &self.on_behalf)
            .finish()
    }
}

/// How a `CallerContext` was established.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthLevel {
    /// Verified `DelegationCertificate` only (pre-UCAN / transport identity).
    Delegated,
    /// Full verified UCAN capability chain.
    Ucan,
    /// Substrate-injected lifecycle context (init/migrate), carrying
    /// `data-layer/admin` on the service's own resource.
    LocalElevated,
    /// Substrate-injected stage-4 ABAC context (ADR-0017 §7): the service
    /// acting as itself for the after-step. Carries **no** capabilities and
    /// is exempt from the FDAE sieve, per ADR-0017 §7's "the escape hatches
    /// run under the service's own identity". Read-only-ness comes from
    /// `HostState.read_only`, not from a capability -- host write paths
    /// carry no capability gate of their own, so a narrower
    /// capability would enforce nothing. Distinct from capability-less
    /// `System` only because the sieve exemption keys on this level, and
    /// exempting `System` would re-open the ingress-(ii) bypass.
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
            on_behalf_did: None,
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
            on_behalf_did: None,
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
            on_behalf_did: None,
        }
    }

    /// A caller verified as `did`, as the router builds one for a direct
    /// caller: no app instance, no forwardable proof, no `on_behalf`
    /// person, and `capabilities` as its whole session grant.
    #[must_use]
    pub fn verified(did: &str, auth: AuthLevel, capabilities: Vec<Capability>) -> Self {
        Self {
            caller_did: did.to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: did.to_string(),
                capabilities,
                ..Default::default()
            },
            auth,
            proof: None,
            on_behalf_did: None,
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
            on_behalf_did: None,
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
