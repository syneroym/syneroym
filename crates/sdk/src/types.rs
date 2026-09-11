//! Data models for SDK client communication, service status, and deployment
//! options.

use anyhow::Result;
use syneroym_core::dht_registry::SignedEndpointInfo;
use syneroym_identity::DelegationCertificate;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    AssetBundle, Visibility,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeployedService {
    pub service_id: String,
    pub interfaces: Vec<String>,
    pub endpoint_type: String,
    /// Unix seconds when the installed instance certificate expires, if one
    /// is installed. `#[serde(default)]` so a substrate predating this field
    /// still deserializes.
    #[serde(default)]
    pub instance_certificate_expires_at: Option<u64>,
    /// Declared publication visibility (ADR-0018 §4). `#[serde(default)]` so a
    /// substrate predating this field still deserializes as `None`.
    #[serde(default)]
    pub visibility: Option<Visibility>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SigningIdentityInfo {
    pub signing_did: String,
    pub pubkey_hex: String,
    pub owner_did: Option<String>,
}

/// Publication policy for a service deployment (ADR-0018 §4). One type rather
/// than two loose fields, so the three legal pairings are the only ones a
/// caller can express -- the substrate still validates independently.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Publication {
    /// Never registered. The default.
    #[default]
    Private,
    /// Registered, and propagated to parent registries. The record's own
    /// `is_private` must be `false`.
    Public(SignedEndpointInfo),
    /// Registered with the local registry only. The record's own
    /// `is_private` must be `true`.
    Internal(SignedEndpointInfo),
}

impl Publication {
    /// `(visibility, serialized certificate)` for a `DeployManifest`.
    pub fn split(self) -> Result<(Visibility, Option<String>)> {
        match self {
            Self::Private => Ok((Visibility::Private, None)),
            Self::Public(record) => {
                let serialized = serde_json::to_string(&record).map_err(|e| {
                    anyhow::anyhow!("Failed to serialize registry certificate: {e}")
                })?;
                Ok((Visibility::Public, Some(serialized)))
            }
            Self::Internal(record) => {
                let serialized = serde_json::to_string(&record).map_err(|e| {
                    anyhow::anyhow!("Failed to serialize registry certificate: {e}")
                })?;
                Ok((Visibility::Internal, Some(serialized)))
            }
        }
    }
}

/// Whether the substrate believes a service's instance is running.
/// Deliberately not a bool: a supervisor's remediation differs per variant,
/// and `Unknown` must never be silently read as healthy.
///
/// **No `rename_all`**: this mirrors the wire type
/// (`syneroym_wit_interfaces::control_plane::exports::…::InstancePhase`),
/// which `wit_bindgen`'s `additional_derives` gives a plain, unrenamed
/// `Serialize`/`Deserialize` -- its JSON tags are the literal Rust variant
/// names (`"Running"`, not `"running"`). A `kebab-case` rename here would
/// silently stop parsing the server's actual response.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum InstancePhase {
    Running,
    NotRunning(String),
    Unknown(String),
    /// Also what a caller without a grant on an explicitly named id gets
    /// back -- identical to an id never deployed at all, deliberately, so
    /// a caller with no grant cannot use this to probe for an id's
    /// existence.
    NotFound,
}

/// **No `rename_all`** -- see [`InstancePhase`]'s doc comment.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ProbeStatus {
    NotDeclared,
    Passing,
    Failing(String),
}

/// Outcome of one epoch-guarded binding write (ADR-0021 §3).
/// **No `rename_all`** -- see [`InstancePhase`]'s doc comment.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BindingWriteOutcome {
    Applied,
    NoOp,
    Stale(u64),
    Conflict(u64),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceStatus {
    pub service_id: String,
    pub service_type: Option<String>,
    pub endpoint_type: String,
    pub app_instance_id: Option<String>,
    pub service_name: Option<String>,
    pub phase: InstancePhase,
    pub probe: ProbeStatus,
    pub instance_certificate_issued_at: Option<u64>,
    pub instance_certificate_expires_at: Option<u64>,
    pub probe_checked_at: Option<u64>,
    /// Per declared dependency of this service, the epoch this substrate
    /// currently serves it. `status`'s per-dependent binding convergence
    /// report.
    pub binding_epochs: Vec<(String, u64)>,
}

/// What this node is, as opposed to what is running on it. Present
/// only for a caller holding node-wide `orchestrator/status`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NodeFacts {
    pub node_did: String,
    pub service_types: Vec<String>,
    pub registry_url: Option<String>,
    pub dht_enabled: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubstrateStatus {
    pub node: Option<NodeFacts>,
    pub checked_at: u64,
    pub services: Vec<ServiceStatus>,
}

/// Everything optional about a [`SyneroymClient::deploy_svc_wasm_with_options`]
/// call. Replaces the growing positional tail `deploy_svc_wasm_with_assets`
/// had started: `assets` was the first optional addition, `custom_config`
/// the second, and a third would have meant a third method.
#[derive(Debug, Default)]
pub struct DeploySvcOptions {
    pub publication: Publication,
    pub instance_certificate: Option<DelegationCertificate>,
    pub assets: Option<AssetBundle>,
    /// Verbatim `ServiceConfig.custom_config`. The reserved `http_routes`
    /// key inside it is what declares HTTP routes.
    pub custom_config: Option<String>,
}
