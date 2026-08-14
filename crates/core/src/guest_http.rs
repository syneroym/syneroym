//! Host-side mirror of the `syneroym:http/incoming-handler@0.1.0` WIT
//! records (M06A A2, `crates/wit_interfaces/wit/http/http.wit`).
//!
//! Lives in `core` for the same reason `StreamDirection` does:
//! `syneroym-router` builds these, `syneroym-sandbox-wasm` consumes them,
//! and neither crate depends on the other.

/// How an inbound caller's identity was established, as a guest sees it.
/// Mirrors the WIT `caller-auth`. Deliberately **not** a projection of
/// `syneroym_rpc::AuthLevel`: that enum's `Delegated` covers an
/// unchallenged self-asserted pubkey too, which is what the client gateway
/// sends (M06A F5a, D-A2-12). Derived from the preamble instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestCallerAuth {
    Delegated,
    Ucan,
    SelfAsserted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestCallerIdentity {
    pub did: String,
    pub auth: GuestCallerAuth,
    pub app_instance: Option<String>,
}

/// One inbound HTTP request on its way to a guest's
/// `syneroym:http/incoming-handler#handle-request` (M06A A2). Mirrors the
/// WIT `http-request` field for field, **in the same order** -- the dynamic
/// `Val::Record` built from it must match the declared field order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuestHttpRequest {
    pub method: String,
    pub path: String,
    pub query: String,
    pub route: String,
    pub path_params: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub caller: Option<GuestCallerIdentity>,
}

/// A guest's answer, exactly as returned. Nothing here is validated yet --
/// status range, header well-formedness and body size are the router's
/// checks (M06A D-A2-5), since they are HTTP questions and neither `core`
/// nor `sandbox_wasm` holds `hyper` types.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuestHttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
