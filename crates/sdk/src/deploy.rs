//! Applying a compiled deployment plan across one or more substrates
//! (M05A Slice A3), and the per-substrate member-identity minting that A3's
//! multi-substrate placement requires (M05A §0.1).
//!
//! `PlanApplier` is ADR-0021 §5's narrow "apply this action to that
//! substrate" boundary, introduced here rather than at A5 for two reasons:
//! partial-failure behavior is otherwise not testable without killing a live
//! substrate mid-test, and A5's durable outbox implementation then replaces
//! this trait's body instead of restructuring its callers.

use std::{
    collections::BTreeMap,
    fmt,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use ed25519_dalek::VerifyingKey;
use syneroym_app_orchestration::{
    ActionRecord, ActionState, DeploymentJournal,
    models::{DeploymentPlan, LogicalServiceRef, PlannedService, ServiceId, SubstrateAlias},
};
use syneroym_core::dht_registry::{DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointType};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};

use crate::{
    BindingWrite, BindingWriteOutcome, DeploymentPlan as WitDeploymentPlan, SyneroymClient,
    mapper::map_deployment_plan_to_wit,
};

/// The attended posture's default certificate lifetime for a plan-deploy-time
/// certification.
pub const DEFAULT_INSTANCE_CERT_EXPIRES_HOURS: u64 = 24;

/// ADR-0021 §5's narrow "apply this action to that substrate" boundary.
/// Three actions, not one: A3 introduced this trait (as `PlanApplier`) when
/// applying a plan was the only action, and A5 adds the two the
/// supervisor's own loop issues. A6 replaces the implementation with an
/// outbox/DLQ-backed one and nothing above this trait changes -- which only
/// holds if every action it must make durable is *on* it.
#[async_trait::async_trait]
pub trait SubstrateActor: fmt::Debug + Send + Sync {
    async fn apply_plan(&self, plan: WitDeploymentPlan) -> Result<(), String>;
    async fn write_bindings(&self, write: BindingWrite)
    -> Result<Vec<BindingWriteOutcome>, String>;
    /// Included for the same reason `stop` sits beside `start` on an
    /// engine: a fake in a test must be able to answer every action the
    /// supervisor takes. A6 may keep this synchronous -- a restart queued
    /// for later delivery is usually wrong -- and that is a decision for
    /// A6's implementation, not a reason to leave it outside the trait.
    async fn restart(&self, service_id: String, generation: u64) -> Result<(), String>;
    /// This substrate's held generation for an app instance, if it has
    /// ever recorded one (M05A A5c §19.7/§19.12) -- the supervisor's own
    /// resident loop uses this to detect supersession (ADR-0021 §4).
    /// Behind this trait, not called directly against `SyneroymClient`,
    /// so the loop's superseded/skip decisions are testable against a
    /// fake substrate with no live connection.
    async fn held_generation(&self, app_instance_id: &str) -> Result<Option<u64>, String>;
}

#[async_trait::async_trait]
impl SubstrateActor for SyneroymClient {
    async fn apply_plan(&self, plan: WitDeploymentPlan) -> Result<(), String> {
        self.deploy_plan(plan).await.map_err(|e| e.to_string())
    }

    // `write_bindings`/`restart` deliberately do not delegate to the
    // same-named inherent `SyneroymClient` methods below: inherent
    // methods shadow trait methods of the same name for every dot-call
    // site on this type, including from inside this very impl block, so
    // `self.write_bindings(write)` here would silently call the inherent
    // one anyway -- correct, but confusing to read as "does this
    // recurse?" at a glance. Calling `request` directly keeps this impl
    // legible on its own.
    async fn write_bindings(
        &self,
        write: BindingWrite,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        let params = serde_json::to_value((write,)).map_err(|e| e.to_string())?;
        let res = self
            .request("orchestrator", "write-bindings", params)
            .await
            .map_err(|e| e.to_string())?;
        serde_json::from_value(res.result).map_err(|e| e.to_string())
    }

    async fn restart(&self, service_id: String, generation: u64) -> Result<(), String> {
        let params = serde_json::to_value((service_id, generation)).map_err(|e| e.to_string())?;
        let res =
            self.request("orchestrator", "restart", params).await.map_err(|e| e.to_string())?;
        if res.result == serde_json::json!({"status": "restarted"}) {
            Ok(())
        } else {
            Err(format!("Restart failed: {:?}", res.result))
        }
    }

    /// `app-instance-management-of` returns `option<app-instance-
    /// management>`, which serializes as `null` or the record's fields
    /// directly (ordinary serde `Option`).
    async fn held_generation(&self, app_instance_id: &str) -> Result<Option<u64>, String> {
        let params =
            serde_json::to_value((app_instance_id.to_string(),)).map_err(|e| e.to_string())?;
        let res = self
            .request("orchestrator", "app-instance-management-of", params)
            .await
            .map_err(|e| e.to_string())?;
        Ok(res.result.get("generation").and_then(serde_json::Value::as_u64))
    }
}

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
    /// The generation this apply writes at (ADR-0021 §4, M05A A5a). `0`
    /// for every caller through A5a -- the supervisor that presents a
    /// real, `adopt`-minted generation does not exist yet.
    pub generation: u64,
    /// The binding epoch to stamp on each dependent service's own
    /// bindings, keyed by the dependent's `LogicalServiceRef` (M05A A5c
    /// §19.3/D-A5c-4) -- not a scalar, since one apply can deploy several
    /// dependent services whose counters have each advanced independently.
    /// A service absent from this map maps its bindings at epoch `0`,
    /// meaning "no supervisor has written here", which is what every
    /// caller through A5b still means by it.
    pub binding_epochs: &'a BTreeMap<LogicalServiceRef, u64>,
}

#[derive(Debug, Clone)]
pub struct ServiceFailure {
    pub logical_ref: LogicalServiceRef,
    pub alias: Option<SubstrateAlias>,
    pub substrate_did: String,
    pub error: String,
}

#[derive(Debug, Default)]
pub struct ApplyReport {
    pub deployed: Vec<LogicalServiceRef>,
    pub skipped: Vec<LogicalServiceRef>,
    pub failures: Vec<ServiceFailure>,
}

impl ApplyReport {
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Pairs every service with the target it is placed on, in the plan's own
/// topological order. Fails closed on any alias the caller did not build a
/// target for, before a single deploy call is made -- an unknown alias must
/// never produce a half-applied app.
pub fn resolve_targets<'a>(
    plan: &'a DeploymentPlan,
    targets: &'a BTreeMap<SubstrateAlias, DeployTarget>,
    fallback: Option<&'a DeployTarget>,
) -> Result<Vec<(&'a PlannedService, &'a DeployTarget)>> {
    let mut missing = Vec::new();
    let mut out = Vec::new();
    for svc in &plan.services {
        match (&svc.substrate, fallback) {
            (None, Some(f)) => out.push((svc, f)),
            (None, None) => anyhow::bail!(
                "service '{}' has no placement and no default substrate was supplied",
                svc.logical_ref
            ),
            (Some(alias), _) => match targets.get(alias) {
                Some(t) => out.push((svc, t)),
                None => missing.push((svc.logical_ref.clone(), alias.clone())),
            },
        }
    }
    if missing.is_empty() {
        Ok(out)
    } else {
        let list = missing
            .iter()
            .map(|(logical_ref, alias)| format!("{logical_ref} -> '{alias}'"))
            .collect::<Vec<_>>()
            .join(", ");
        Err(anyhow::anyhow!("no deploy target built for: {list}"))
    }
}

/// The substrate a logical ref is currently placed on, per the **most
/// recent** row naming it, or `None` if it has never landed or was most
/// recently `REMOVE`d. `landed` must be ordered oldest-first, exactly as
/// `DeploymentJournal::get_completed_actions`/`get_completed_actions_for_
/// instance` return it.
///
/// Shared by `apply_plan`'s resume-skip and `roymctl`'s placement-change
/// refusal (`check_no_placement_change`) so the two cannot read the journal
/// two different ways again: post-review, the refusal was fixed to this
/// most-recent-row-wins reading (a `REMOVE` from `app forget` clears it) but
/// the resume skip was not, so a service `forget`-ten and redeployed under
/// an *unchanged* manifest was wrongly reported "already applied" while
/// running nowhere -- the stale `ADD` row was still present, `.any()` does
/// not care that a `REMOVE` sits after it.
pub fn current_placement<'a>(
    landed: &'a [ActionRecord],
    logical_ref: &str,
) -> Option<&'a ActionRecord> {
    match landed.iter().rev().find(|r| r.logical_ref == logical_ref) {
        Some(r) if r.action_type == "ADD" => Some(r),
        _ => None,
    }
}

/// Applies one deploy call per (service, substrate), recording a journal
/// action row for each and continuing past a failure rather than aborting
/// the whole app (task.md's partial-deploy non-goal / failure-matrix row
/// 12). A service whose most recent row is `COMPLETED ADD` on the same
/// substrate DID is skipped -- this is A3's "retry" mechanism: a re-run
/// resumes rather than redeploying everything.
pub async fn apply_plan(
    req: ApplyRequest<'_>,
    journal: &DeploymentJournal,
    deployment_id: i64,
) -> Result<ApplyReport> {
    let placed = resolve_targets(req.plan, req.targets, req.fallback)?;
    let completed = journal.get_completed_actions(deployment_id)?;
    let mut report = ApplyReport::default();

    for (svc, target) in placed {
        let l_ref = svc.logical_ref.to_string();

        // D-A3-11: keyed on the DID, so an alias re-pointed at a different
        // node correctly redeploys rather than being skipped as already
        // done. `current_placement` reads the most recent row, not just any
        // ADD -- a `REMOVE` from `app forget` must force a redeploy, not a
        // skip, even though an older ADD for the same (ref, DID) pair is
        // still sitting in this record's history.
        if current_placement(&completed, &l_ref)
            .is_some_and(|r| r.substrate_did == target.substrate_did)
        {
            report.skipped.push(svc.logical_ref.clone());
            continue;
        }

        let action_id = journal.append_action(
            deployment_id,
            "ADD",
            &l_ref,
            target.alias.as_ref().map(SubstrateAlias::as_str),
            &target.substrate_did,
            ActionState::InProgress,
        )?;

        // A mapping error (an unreadable WASM artifact, an oversized
        // document) is this one service's failure, not the whole app's: the
        // same no-rollback-keep-going rule a substrate-side deploy failure
        // gets.
        let outcome: Result<(), String> = match map_deployment_plan_to_wit(
            req.plan,
            &[svc],
            req.instance_certificates,
            req.registry_certificates,
            req.emit_bindings,
            req.generation,
            req.binding_epochs,
        ) {
            Err(e) => Err(e.to_string()),
            Ok(wit_plan) => target.actor.apply_plan(wit_plan).await,
        };

        match outcome {
            Ok(()) => {
                journal.update_action_state(action_id, ActionState::Completed)?;
                report.deployed.push(svc.logical_ref.clone());
            }
            Err(error) => {
                journal.update_action_state(action_id, ActionState::Failed)?;
                report.failures.push(ServiceFailure {
                    logical_ref: svc.logical_ref.clone(),
                    alias: target.alias.clone(),
                    substrate_did: target.substrate_did.clone(),
                    error,
                });
            }
        }
    }

    Ok(report)
}

/// Queries `client`'s substrate for the instance key it would derive for
/// `service_id` under the connecting caller's identity, and issues a
/// `service-instance`-scoped certificate over it from `master`. This is the
/// full round trip: one read-only RPC, one local signature, no install call
/// -- installation happens at deploy.
///
/// **Bound to this client, not just this master.** The substrate derives the
/// key from its own node identity *and* the calling DID, so a certificate
/// minted through one client is rejected at deploy by any other substrate,
/// and by the same substrate reached as a different caller.
pub async fn certify_instance(
    client: &SyneroymClient,
    master: &Identity,
    service_id: &str,
    expires_hours: u64,
) -> Result<DelegationCertificate> {
    let master_did = substrate::derive_did_key(&master.public_key());
    if master_did != service_id {
        anyhow::bail!(
            "master identity resolves to {master_did}, which does not match service_id \
             {service_id} -- a certificate for this pair would be rejected at install time"
        );
    }

    let identity = client
        .instance_identity(service_id)
        .await
        .context("failed to query the substrate for its derived instance identity")?;
    let pubkey_bytes = hex::decode(&identity.pubkey_hex)
        .context("substrate returned an invalid hex-encoded instance pubkey")?;
    let pubkey_array: [u8; 32] = pubkey_bytes.try_into().map_err(|_| {
        anyhow::anyhow!("substrate returned an instance pubkey of the wrong length")
    })?;
    let pubkey = VerifyingKey::from_bytes(&pubkey_array)
        .context("substrate returned an invalid ed25519 instance pubkey")?;

    DelegationCertificate::issue(
        master,
        pubkey,
        expires_hours * 3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
}

/// Mints, per placed member, the instance certificate its hosting substrate
/// will accept and the endpoint record that points at that substrate.
///
/// Returns `(instance_certificates, registry_certificates)`, both keyed by
/// the member master `ServiceId`, ready for `ApplyRequest`.
pub async fn certify_placed_members(
    plan: &DeploymentPlan,
    masters: &BTreeMap<ServiceId, Identity>,
    clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
    // `None` when every service is placed by alias.
    fallback: Option<&Arc<SyneroymClient>>,
    expires_hours: u64,
) -> Result<(BTreeMap<ServiceId, String>, BTreeMap<ServiceId, String>)> {
    // Two services sharing one master would need two endpoint records under
    // one service_id pointing at different substrates -- a permanent
    // compare-and-swap fight at the registry. Impossible from today's
    // compiler (one master per PlannedService), so this is an assertion, not
    // a supported case.
    let mut seen: BTreeMap<&ServiceId, &LogicalServiceRef> = BTreeMap::new();
    for svc in &plan.services {
        if let Some(prev) = seen.insert(&svc.service_id, &svc.logical_ref) {
            anyhow::bail!(
                "member master {} is placed twice ({}, {})",
                svc.service_id,
                prev,
                svc.logical_ref
            );
        }
    }

    let mut certs = BTreeMap::new();
    let mut records = BTreeMap::new();
    for svc in &plan.services {
        let master = masters.get(&svc.service_id).ok_or_else(|| {
            anyhow::anyhow!("no member master resolved for service {}", svc.service_id)
        })?;
        let client = match (&svc.substrate, fallback) {
            (Some(alias), _) => clients
                .get(alias)
                .ok_or_else(|| anyhow::anyhow!("no client built for substrate alias '{alias}'"))?,
            (None, Some(f)) => f,
            (None, None) => anyhow::bail!(
                "service '{}' has no placement and no default substrate was supplied",
                svc.logical_ref
            ),
        };

        let cert = certify_instance(client, master, svc.service_id.as_str(), expires_hours).await?;
        certs.insert(svc.service_id.clone(), cert.to_json()?);

        // The endpoint record: the substrate holds no key that could ever
        // sign this (ADR-0020 §3), so unlike the instance certificate above
        // this is the *only* place one gets produced for an app-deployed
        // member -- without it, the master DID never resolves to an address
        // at all.
        let not_after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS);
        let record = EndpointInfo {
            service_id: svc.service_id.to_string(),
            substrate_id: client.service_id().to_string(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: None,
            is_private: false,
            ttl: None,
            not_after,
        }
        .sign(master)?;
        records.insert(svc.service_id.clone(), serde_json::to_string(&record)?);
    }

    Ok((certs, records))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use semver::Version;
    use syneroym_app_orchestration::{
        DeploymentState,
        models::{
            AppBlueprintId, AppInstanceId, LogicalServiceName, ServiceConfig, ServiceType,
            TopologyMode,
        },
    };

    use super::*;

    #[derive(Debug, Default)]
    struct FailingApplier {
        should_fail: bool,
        calls: Mutex<Vec<WitDeploymentPlan>>,
    }

    #[async_trait::async_trait]
    impl SubstrateActor for FailingApplier {
        async fn apply_plan(&self, plan: WitDeploymentPlan) -> Result<(), String> {
            self.calls.lock().unwrap().push(plan);
            if self.should_fail { Err("simulated failure".to_string()) } else { Ok(()) }
        }

        async fn write_bindings(
            &self,
            _write: BindingWrite,
        ) -> Result<Vec<BindingWriteOutcome>, String> {
            unimplemented!("not exercised by apply_plan's own tests")
        }

        async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
            unimplemented!("not exercised by apply_plan's own tests")
        }

        async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
            unimplemented!("not exercised by apply_plan's own tests")
        }
    }

    fn dummy_config() -> ServiceConfig {
        ServiceConfig {
            service_type: ServiceType::Tcp,
            source: "127.0.0.1:9000".to_string(),
            hash: None,
            interfaces: vec![],
            env: BTreeMap::new(),
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: Default::default(),
            fdae: None,
            health_check: None,
        }
    }

    fn service(name: &str, substrate: Option<&str>) -> PlannedService {
        PlannedService {
            service_id: ServiceId::new(format!("did:key:h{name}")),
            logical_ref: LogicalServiceRef {
                app_instance_id: AppInstanceId::new("inst-1"),
                service_name: LogicalServiceName::new(name),
            },
            substrate: substrate.map(SubstrateAlias::new),
            config: dummy_config(),
            resolved_dependencies: BTreeMap::new(),
            topology_mode: TopologyMode::Singleton,
        }
    }

    fn plan(services: Vec<PlannedService>) -> DeploymentPlan {
        DeploymentPlan {
            app_instance_id: AppInstanceId::new("inst-1"),
            blueprint_id: AppBlueprintId::new("syneroym:test"),
            version: Version::parse("1.0.0").unwrap(),
            services,
        }
    }

    fn target(did: &str, alias: Option<&str>, actor: Arc<dyn SubstrateActor>) -> DeployTarget {
        DeployTarget {
            alias: alias.map(SubstrateAlias::new),
            substrate_did: did.to_string(),
            actor,
        }
    }

    #[test]
    fn resolve_targets_fails_closed_naming_every_unknown_alias() {
        let p = plan(vec![service("a", Some("edge-1")), service("b", Some("edge-2"))]);
        let targets = BTreeMap::new();
        let err = resolve_targets(&p, &targets, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("edge-1"));
        assert!(msg.contains("edge-2"));
    }

    #[tokio::test]
    async fn apply_plan_deploys_each_service_to_its_own_target() {
        let journal = DeploymentJournal::open_in_memory().unwrap();
        let p = plan(vec![service("a", Some("edge-1")), service("b", Some("edge-2"))]);
        let deployment_id = journal.append(&p, DeploymentState::Applying).unwrap();

        let applier_a = Arc::new(FailingApplier::default());
        let applier_b = Arc::new(FailingApplier::default());
        let targets = BTreeMap::from([
            (
                SubstrateAlias::new("edge-1"),
                target("did:key:zA", Some("edge-1"), applier_a.clone()),
            ),
            (
                SubstrateAlias::new("edge-2"),
                target("did:key:zB", Some("edge-2"), applier_b.clone()),
            ),
        ]);

        let report = apply_plan(
            ApplyRequest {
                plan: &p,
                targets: &targets,
                fallback: None,
                instance_certificates: &BTreeMap::new(),
                registry_certificates: &BTreeMap::new(),
                emit_bindings: false,
                generation: 0,
                binding_epochs: &BTreeMap::new(),
            },
            &journal,
            deployment_id,
        )
        .await
        .unwrap();

        assert_eq!(report.deployed.len(), 2);
        assert!(report.failures.is_empty());
        assert_eq!(applier_a.calls.lock().unwrap().len(), 1);
        assert_eq!(applier_b.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn apply_plan_records_one_action_row_per_service_and_substrate() {
        let journal = DeploymentJournal::open_in_memory().unwrap();
        let p = plan(vec![service("a", Some("edge-1"))]);
        let deployment_id = journal.append(&p, DeploymentState::Applying).unwrap();

        let applier = Arc::new(FailingApplier::default());
        let targets = BTreeMap::from([(
            SubstrateAlias::new("edge-1"),
            target("did:key:zA", Some("edge-1"), applier),
        )]);

        apply_plan(
            ApplyRequest {
                plan: &p,
                targets: &targets,
                fallback: None,
                instance_certificates: &BTreeMap::new(),
                registry_certificates: &BTreeMap::new(),
                emit_bindings: false,
                generation: 0,
                binding_epochs: &BTreeMap::new(),
            },
            &journal,
            deployment_id,
        )
        .await
        .unwrap();

        let completed = journal.get_completed_actions(deployment_id).unwrap();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].substrate_alias.as_deref(), Some("edge-1"));
        assert_eq!(completed[0].substrate_did, "did:key:zA");
    }

    #[tokio::test]
    async fn apply_plan_continues_past_a_failure_and_reports_it() {
        let journal = DeploymentJournal::open_in_memory().unwrap();
        let p = plan(vec![service("a", Some("edge-1")), service("b", Some("edge-2"))]);
        let deployment_id = journal.append(&p, DeploymentState::Applying).unwrap();

        let applier_a = Arc::new(FailingApplier { should_fail: true, ..Default::default() });
        let applier_b = Arc::new(FailingApplier::default());
        let targets = BTreeMap::from([
            (SubstrateAlias::new("edge-1"), target("did:key:zA", Some("edge-1"), applier_a)),
            (SubstrateAlias::new("edge-2"), target("did:key:zB", Some("edge-2"), applier_b)),
        ]);

        let report = apply_plan(
            ApplyRequest {
                plan: &p,
                targets: &targets,
                fallback: None,
                instance_certificates: &BTreeMap::new(),
                registry_certificates: &BTreeMap::new(),
                emit_bindings: false,
                generation: 0,
                binding_epochs: &BTreeMap::new(),
            },
            &journal,
            deployment_id,
        )
        .await
        .unwrap();

        assert_eq!(report.deployed.len(), 1);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].error, "simulated failure");
        assert!(!report.is_complete());
    }

    #[tokio::test]
    async fn apply_plan_skips_a_service_already_completed_on_the_same_substrate() {
        let journal = DeploymentJournal::open_in_memory().unwrap();
        let p = plan(vec![service("a", Some("edge-1"))]);
        let deployment_id = journal.append(&p, DeploymentState::Applying).unwrap();
        journal
            .append_action(
                deployment_id,
                "ADD",
                &p.services[0].logical_ref.to_string(),
                Some("edge-1"),
                "did:key:zA",
                ActionState::Completed,
            )
            .unwrap();

        let applier = Arc::new(FailingApplier::default());
        let targets = BTreeMap::from([(
            SubstrateAlias::new("edge-1"),
            target("did:key:zA", Some("edge-1"), applier.clone()),
        )]);

        let report = apply_plan(
            ApplyRequest {
                plan: &p,
                targets: &targets,
                fallback: None,
                instance_certificates: &BTreeMap::new(),
                registry_certificates: &BTreeMap::new(),
                emit_bindings: false,
                generation: 0,
                binding_epochs: &BTreeMap::new(),
            },
            &journal,
            deployment_id,
        )
        .await
        .unwrap();

        assert_eq!(report.skipped.len(), 1);
        assert!(report.deployed.is_empty());
        assert!(
            applier.calls.lock().unwrap().is_empty(),
            "a skipped service must not be re-applied"
        );
    }

    #[tokio::test]
    async fn apply_plan_does_not_skip_when_the_alias_now_resolves_to_a_different_did() {
        let journal = DeploymentJournal::open_in_memory().unwrap();
        let p = plan(vec![service("a", Some("edge-1"))]);
        let deployment_id = journal.append(&p, DeploymentState::Applying).unwrap();
        journal
            .append_action(
                deployment_id,
                "ADD",
                &p.services[0].logical_ref.to_string(),
                Some("edge-1"),
                "did:key:zOldNode",
                ActionState::Completed,
            )
            .unwrap();

        // The alias now resolves to a different node than the completed row names.
        let applier = Arc::new(FailingApplier::default());
        let targets = BTreeMap::from([(
            SubstrateAlias::new("edge-1"),
            target("did:key:zNewNode", Some("edge-1"), applier.clone()),
        )]);

        let report = apply_plan(
            ApplyRequest {
                plan: &p,
                targets: &targets,
                fallback: None,
                instance_certificates: &BTreeMap::new(),
                registry_certificates: &BTreeMap::new(),
                emit_bindings: false,
                generation: 0,
                binding_epochs: &BTreeMap::new(),
            },
            &journal,
            deployment_id,
        )
        .await
        .unwrap();

        assert_eq!(report.deployed.len(), 1);
        assert!(report.skipped.is_empty());
        assert_eq!(applier.calls.lock().unwrap().len(), 1);
    }

    /// Post-review finding A: `app forget` appends a `REMOVE` row for the
    /// same (logical ref, DID) an earlier `ADD` in this very record already
    /// completed. A skip check scoped to "does any completed ADD match"
    /// would still find that ADD and skip -- reporting a service "already
    /// applied" while nothing is running there, since it was forgotten. The
    /// most-recent-row-wins reading must see the `REMOVE` and redeploy.
    #[tokio::test]
    async fn apply_plan_redeploys_a_service_whose_most_recent_row_is_remove() {
        let journal = DeploymentJournal::open_in_memory().unwrap();
        let p = plan(vec![service("a", Some("edge-1"))]);
        let deployment_id = journal.append(&p, DeploymentState::Degraded).unwrap();
        let l_ref = p.services[0].logical_ref.to_string();
        journal
            .append_action(
                deployment_id,
                "ADD",
                &l_ref,
                Some("edge-1"),
                "did:key:zA",
                ActionState::Completed,
            )
            .unwrap();
        // `app forget`'s own write: a REMOVE row for the same (ref, DID),
        // appended to the same record.
        journal
            .append_action(
                deployment_id,
                "REMOVE",
                &l_ref,
                Some("edge-1"),
                "did:key:zA",
                ActionState::Completed,
            )
            .unwrap();

        let applier = Arc::new(FailingApplier::default());
        let targets = BTreeMap::from([(
            SubstrateAlias::new("edge-1"),
            target("did:key:zA", Some("edge-1"), applier.clone()),
        )]);

        let report = apply_plan(
            ApplyRequest {
                plan: &p,
                targets: &targets,
                fallback: None,
                instance_certificates: &BTreeMap::new(),
                registry_certificates: &BTreeMap::new(),
                emit_bindings: false,
                generation: 0,
                binding_epochs: &BTreeMap::new(),
            },
            &journal,
            deployment_id,
        )
        .await
        .unwrap();

        assert_eq!(report.deployed.len(), 1, "{report:?}");
        assert!(report.skipped.is_empty(), "{report:?}");
        assert_eq!(applier.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_mapping_error_fails_only_its_own_service() {
        let journal = DeploymentJournal::open_in_memory().unwrap();
        let mut bad = service("a", Some("edge-1"));
        bad.config.service_type = ServiceType::Wasm;
        bad.config.source = "does-not-exist.wasm".to_string();
        let p = plan(vec![bad, service("b", Some("edge-2"))]);
        let deployment_id = journal.append(&p, DeploymentState::Applying).unwrap();

        let applier_a = Arc::new(FailingApplier::default());
        let applier_b = Arc::new(FailingApplier::default());
        let targets = BTreeMap::from([
            (
                SubstrateAlias::new("edge-1"),
                target("did:key:zA", Some("edge-1"), applier_a.clone()),
            ),
            (
                SubstrateAlias::new("edge-2"),
                target("did:key:zB", Some("edge-2"), applier_b.clone()),
            ),
        ]);

        let report = apply_plan(
            ApplyRequest {
                plan: &p,
                targets: &targets,
                fallback: None,
                instance_certificates: &BTreeMap::new(),
                registry_certificates: &BTreeMap::new(),
                emit_bindings: false,
                generation: 0,
                binding_epochs: &BTreeMap::new(),
            },
            &journal,
            deployment_id,
        )
        .await
        .unwrap();

        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.deployed.len(), 1);
        assert!(
            applier_a.calls.lock().unwrap().is_empty(),
            "the failing mapper must not be applied"
        );
        assert_eq!(applier_b.calls.lock().unwrap().len(), 1);
    }

    /// D-A5-8: `apply_plan` holds `&DeploymentJournal` across every `.await`,
    /// so its future is `Send` only because the journal itself now is
    /// (`Arc<Mutex<Connection>>`, not a bare `Connection`) -- required for
    /// a supervisor's reconcile loop to `tokio::spawn` a call to this
    /// function.
    #[test]
    fn apply_plan_returns_a_send_future() {
        fn assert_send<T: Send>(_: T) {}

        let journal = DeploymentJournal::open_in_memory().unwrap();
        let p = plan(vec![service("a", Some("edge-1"))]);
        let deployment_id = journal.append(&p, DeploymentState::Applying).unwrap();
        let applier = Arc::new(FailingApplier::default());
        let targets = BTreeMap::from([(
            SubstrateAlias::new("edge-1"),
            target("did:key:zA", Some("edge-1"), applier),
        )]);

        let no_certs = BTreeMap::new();
        let no_epochs = BTreeMap::new();
        let fut = apply_plan(
            ApplyRequest {
                plan: &p,
                targets: &targets,
                fallback: None,
                instance_certificates: &no_certs,
                registry_certificates: &no_certs,
                emit_bindings: false,
                generation: 0,
                binding_epochs: &no_epochs,
            },
            &journal,
            deployment_id,
        );
        assert_send(fut);
    }
}
