//! Data types for deployment plans and execution reports.

use std::{collections::BTreeMap, sync::Arc};

use syneroym_app_orchestration::models::{DeploymentPlan, MemberRef, ServiceId, SubstrateAlias};

use super::SubstrateActor;
/// A connected deploy target: the alias it was named by (`None` for the
/// invocation's own default substrate), the substrate's DID, and the
/// actor.
#[derive(Debug, Clone)]
pub struct DeployTarget {
    pub alias: Option<SubstrateAlias>,
    pub substrate_did: String,
    pub actor: Arc<dyn SubstrateActor>,
}

#[derive(Debug)]
pub struct ApplyRequest<'a> {
    pub plan: &'a DeploymentPlan,
    pub targets: &'a BTreeMap<SubstrateAlias, DeployTarget>,
    /// The invocation's own default substrate, for services with no
    /// placement. `None` when every service is placed by alias -- a
    /// fully-placed app must not require a default substrate it never
    /// touches.
    pub fallback: Option<&'a DeployTarget>,
    pub instance_certificates: &'a BTreeMap<ServiceId, String>,
    pub registry_certificates: &'a BTreeMap<ServiceId, String>,
    pub emit_bindings: bool,
    /// The generation this apply writes at (ADR-0021 §4). `0` for an
    /// unmanaged deploy; a managing supervisor stamps its adopted
    /// generation here.
    pub generation: u64,
    /// The binding epoch to stamp on each dependent member's own bindings,
    /// keyed by the dependent's `MemberRef` (each member holds its own
    /// `service_bindings` row on the substrate) -- not a scalar, since one
    /// apply can deploy several dependent members whose counters have each
    /// advanced independently. A member absent from this map maps its
    /// bindings at epoch `0`, meaning "no supervisor has written here".
    pub binding_epochs: &'a BTreeMap<MemberRef, u64>,
}

#[derive(Debug, Clone)]
pub struct ServiceFailure {
    pub member_ref: MemberRef,
    pub alias: Option<SubstrateAlias>,
    pub substrate_did: String,
    pub error: String,
}

#[derive(Debug, Default)]
pub struct ApplyReport {
    pub deployed: Vec<MemberRef>,
    pub skipped: Vec<MemberRef>,
    pub failures: Vec<ServiceFailure>,
}

impl ApplyReport {
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.failures.is_empty()
    }
}
