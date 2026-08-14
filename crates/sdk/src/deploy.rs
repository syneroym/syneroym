//! Applying a compiled deployment plan across one or more substrates
//! (M05A Slice A3), and the per-substrate member-identity minting that A3's
//! multi-substrate placement requires (M05A §0.1).
//!
//! `PlanApplier` is ADR-0021 §5's narrow "apply this action to that
//! substrate" boundary, introduced here rather than at A5 for two reasons:
//! partial-failure behavior is otherwise not testable without killing a live
//! substrate mid-test, and a durable, queue-backed implementation
//! ([`build_durable_actor`]) replaces this trait's body instead of
//! restructuring its callers.

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
    models::{
        DeploymentPlan, LogicalServiceRef, MemberRef, PlannedService, ServiceId, SubstrateAlias,
    },
};
use syneroym_core::dht_registry::{DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointType};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_rpc::JsonRpcError;

use crate::{
    BindingWrite, BindingWriteOutcome, DeploymentPlan as WitDeploymentPlan, InstanceIdentity,
    SyneroymClient, mapper::map_deployment_plan_to_wit,
};

/// The attended posture's default certificate lifetime for a plan-deploy-time
/// certification.
pub const DEFAULT_INSTANCE_CERT_EXPIRES_HOURS: u64 = 24;

/// ADR-0021 §5's narrow "apply this action to that substrate" boundary.
/// Three actions, not one: A3 introduced this trait (as `PlanApplier`) when
/// applying a plan was the only action, and A5 adds the two the
/// supervisor's own loop issues. [`build_durable_actor`] wraps this trait
/// with an outbox/DLQ-backed implementation, and nothing above it changed --
/// which holds only because every action that must be made durable is *on*
/// this trait, not bolted on beside it.
#[async_trait::async_trait]
pub trait SubstrateActor: fmt::Debug + Send + Sync {
    async fn apply_plan(&self, plan: WitDeploymentPlan) -> Result<(), String>;
    async fn write_bindings(&self, write: BindingWrite)
    -> Result<Vec<BindingWriteOutcome>, String>;
    /// Included for the same reason `stop` sits beside `start` on an
    /// engine: a fake in a test must be able to answer every action the
    /// supervisor takes. Never queued: a restart is remediation for a
    /// condition observed *now*, and delivering it later restarts a
    /// service that may already have recovered on its own -- the
    /// supervisor's own bounded remediation policy decides what a failed
    /// restart means, and a queue behind it would be a second policy
    /// disagreeing with the first.
    async fn restart(&self, service_id: String, generation: u64) -> Result<(), String>;
    /// Run one scheduled tick on a deployed member. Never
    /// queued: the intent expires (ADR-0023 §3), and the schedule's next
    /// tick is a better retry than a delivery hours later.
    ///
    /// Defaulted because most implementations of this trait are fakes for
    /// control flow that has nothing to do with scheduling; an
    /// implementation that means to run ticks overrides it.
    async fn run_scheduled(
        &self,
        _service_id: String,
        _generation: u64,
        _interface: String,
        _method: String,
        _params_json: Option<String>,
    ) -> Result<(), String> {
        Err("this actor does not run scheduled tasks".to_string())
    }
    /// Install a freshly-issued instance certificate in place, without a
    /// reinstall (M05A A5's unattended renewal). On the trait for the same
    /// reason `restart` is: the supervisor's renewal work-list is a control
    /// flow worth testing against a fake substrate rather than a live one.
    async fn renew_cert(
        &self,
        service_id: String,
        generation: u64,
        instance_certificate: String,
    ) -> Result<(), String>;
    /// The instance signing key this substrate derives for `service_id`
    /// under the connecting caller -- the read half of minting a
    /// certificate. On the trait so a renewal's whole mint -> install ->
    /// rotate sequence is exercisable without a live substrate.
    async fn instance_identity(&self, service_id: &str) -> Result<InstanceIdentity, String>;
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

    async fn renew_cert(
        &self,
        service_id: String,
        generation: u64,
        instance_certificate: String,
    ) -> Result<(), String> {
        let params = serde_json::to_value((service_id, generation, instance_certificate))
            .map_err(|e| e.to_string())?;
        let res =
            self.request("orchestrator", "renew-cert", params).await.map_err(|e| e.to_string())?;
        if res.result == serde_json::json!({"status": "cert_renewed"}) {
            Ok(())
        } else {
            Err(format!("Certificate renewal failed: {:?}", res.result))
        }
    }

    async fn run_scheduled(
        &self,
        service_id: String,
        generation: u64,
        interface: String,
        method: String,
        params_json: Option<String>,
    ) -> Result<(), String> {
        let params = serde_json::to_value((service_id, generation, interface, method, params_json))
            .map_err(|e| e.to_string())?;
        let res = self
            .request("orchestrator", "run-scheduled", params)
            .await
            .map_err(|e| e.to_string())?;
        if res.result == serde_json::json!({"status": "ran"}) {
            Ok(())
        } else {
            Err(format!("Scheduled run failed: {:?}", res.result))
        }
    }

    async fn instance_identity(&self, service_id: &str) -> Result<InstanceIdentity, String> {
        SyneroymClient::instance_identity(self, service_id).await.map_err(|e| e.to_string())
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

/// The single point every call site with no durable queue behind it upcasts
/// a connected client (or, in a test, a fake) into the trait-object shape
/// [`DeployTarget::actor`] and [`ApplyRequest`]'s targets consume --
/// replacing ten near-identical `client.clone() as Arc<dyn SubstrateActor>`
/// expressions (M05B B1, D-B1-4). `roymctl` and the SDK's own e2e fixtures
/// call this deliberately (D-B1-11): a one-shot process exits when its
/// command finishes, so a durable queue behind it would be written and never
/// drained. [`build_durable_actor`] is the other call this function's
/// callers choose between, for the one action-owner that keeps a worker
/// alive to drain what it enqueues.
#[must_use]
pub fn build_actor<T: SubstrateActor + 'static>(actor: Arc<T>) -> Arc<dyn SubstrateActor> {
    actor
}

/// What a durable actor's `write_bindings` enqueues onto when it cannot
/// reach its target (D-B1-1: try synchronously first, enqueue only on
/// transport failure). A trait here, rather than this crate depending on
/// `syneroym-async-queue` directly, so the queue's storage, its worker, and
/// the `instance_lock` it coordinates with stay owned by the crate that
/// also owns those (`syneroym-app-supervisor`) -- this crate only needs to
/// know that *something* durable exists to hand a failed write to.
#[async_trait::async_trait]
pub trait WriteBindingsOutbox: fmt::Debug + Send + Sync {
    /// `queue_key` is opaque to this crate -- the caller's own grouping key
    /// (the supervisor's is `(instance, logical_ref, substrate)`, D-B1-6),
    /// bound in at [`build_durable_actor`] rather than derived here, since
    /// `BindingWrite` alone does not carry a logical ref. Infallible from
    /// this trait's own perspective: an enqueue that cannot itself be
    /// written is a queue-storage problem the implementation logs and
    /// swallows, not a second failure `write_bindings` should report on top
    /// of the transport error it is already returning.
    async fn enqueue(&self, queue_key: &str, substrate_did: &str, write: &BindingWrite);
}

/// `true` when `err` is the substrate answering and refusing the call
/// (a `JsonRpcError` wire response, e.g. `PermissionDenied`) rather than a
/// failure to reach it at all. `SyneroymClient::request_raw` is the only
/// place a `JsonRpcError` enters the chain -- an error response frame
/// deserializes into one; every other failure along the connect/write/read
/// path (a dropped connection, a malformed response, a timeout) surfaces as
/// some other error type. A callee error is never worth retrying: the
/// substrate was reached and it said no, and retrying against the same
/// answer forever would be a second, silent policy competing with the one
/// that already reported it (test 16, D-B1-1).
#[must_use]
pub fn is_callee_error(err: &anyhow::Error) -> bool {
    err.downcast_ref::<JsonRpcError>().is_some()
}

/// `true` when `err` is specifically the substrate answering that this
/// write's own target no longer exists (failure-matrix row 9) -- narrower
/// than [`is_callee_error`], which is `true` for *every* reached-and-
/// answered failure. `control_plane`'s `write-bindings` dispatch maps every
/// server-side refusal (a stale generation, an authorization gap, a
/// genuinely gone service) to the same wire-level `InternalError` code, so
/// the message text is the only signal this crate can read without a wire
/// protocol change; matched against the one message `write_bindings_impl`
/// emits for "gone" specifically, not against `InternalError` generally.
///
/// Used only by the queue worker's terminal decision for an *already-
/// durable* item ([`DurableActor::write_bindings`]'s own enqueue decision
/// stays on the broader [`is_callee_error`], since declining to enqueue is
/// cheap to get wrong in either direction -- the write is not yet
/// durable). Dead-lettering a queued item on any callee error, as
/// `is_callee_error` alone would, would also give up on a transient,
/// reached-and-answered failure (a locked database, a service still
/// starting) that a later retry would have cleared (M05B B1 review finding
/// 10).
#[must_use]
pub fn is_target_gone_error(err: &anyhow::Error) -> bool {
    err.downcast_ref::<JsonRpcError>()
        .is_some_and(|e| e.message.contains("has no app context on this substrate"))
}

/// What `DurableActor` calls to attempt a delivery -- narrower than
/// `SubstrateActor` itself so a test can fake exactly the transport-vs-
/// callee distinction `write_bindings`'s durability decision rests on
/// (`is_callee_error`), without faking a live client. `SyneroymClient`'s own
/// inherent `write_bindings` (not the trait method a few lines below, which
/// stringifies the error before this decision can be made) is the only
/// production implementation.
#[async_trait::async_trait]
pub trait WriteBindingsAttempt: fmt::Debug + Send + Sync {
    async fn attempt_write_bindings(&self, write: BindingWrite)
    -> Result<Vec<BindingWriteOutcome>>;
}

#[async_trait::async_trait]
impl WriteBindingsAttempt for SyneroymClient {
    async fn attempt_write_bindings(
        &self,
        write: BindingWrite,
    ) -> Result<Vec<BindingWriteOutcome>> {
        self.write_bindings(write).await
    }
}

/// The durable `SubstrateActor`: every action but `write_bindings` stays
/// exactly the synchronous, undurable call the trait already made
/// (D-B1-3/D-B1-12 -- `restart`, `apply_plan`, and `renew_cert` are never
/// queued, the last two because they embed a certificate that expires in
/// hours, not because queueing them is hard). `write_bindings` attempts
/// synchronously first and, only on a transport failure, also enqueues onto
/// `outbox` before returning the same error a bare client would have
/// returned (D-B1-1) -- so a caller reading the return value sees no
/// difference from today, and `push_bindings`'s existing alert/`Degraded`
/// handling needs no change at all.
#[derive(Debug)]
struct DurableActor<T> {
    inner: Arc<T>,
    substrate_did: String,
    /// The caller's own grouping key (D-B1-6), bound at construction --
    /// `BindingWrite` alone carries no logical ref for this to derive.
    queue_key: String,
    outbox: Arc<dyn WriteBindingsOutbox>,
}

#[async_trait::async_trait]
impl<T: SubstrateActor + WriteBindingsAttempt + 'static> SubstrateActor for DurableActor<T> {
    async fn apply_plan(&self, plan: WitDeploymentPlan) -> Result<(), String> {
        <T as SubstrateActor>::apply_plan(&self.inner, plan).await
    }

    async fn write_bindings(
        &self,
        write: BindingWrite,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        match self.inner.attempt_write_bindings(write.clone()).await {
            Ok(outcomes) => Ok(outcomes),
            Err(err) => {
                if !is_callee_error(&err) {
                    self.outbox.enqueue(&self.queue_key, &self.substrate_did, &write).await;
                }
                Err(err.to_string())
            }
        }
    }

    async fn restart(&self, service_id: String, generation: u64) -> Result<(), String> {
        <T as SubstrateActor>::restart(&self.inner, service_id, generation).await
    }

    // Forwarded with no outbox involvement, for the same reason `restart`
    // isn't queued: the intent expires (ADR-0023 §3), and a scheduled run
    // delivered late is not a delivery worth making at all.
    async fn run_scheduled(
        &self,
        service_id: String,
        generation: u64,
        interface: String,
        method: String,
        params_json: Option<String>,
    ) -> Result<(), String> {
        <T as SubstrateActor>::run_scheduled(
            &self.inner,
            service_id,
            generation,
            interface,
            method,
            params_json,
        )
        .await
    }

    async fn renew_cert(
        &self,
        service_id: String,
        generation: u64,
        instance_certificate: String,
    ) -> Result<(), String> {
        <T as SubstrateActor>::renew_cert(&self.inner, service_id, generation, instance_certificate)
            .await
    }

    async fn instance_identity(&self, service_id: &str) -> Result<InstanceIdentity, String> {
        <T as SubstrateActor>::instance_identity(&self.inner, service_id).await
    }

    async fn held_generation(&self, app_instance_id: &str) -> Result<Option<u64>, String> {
        <T as SubstrateActor>::held_generation(&self.inner, app_instance_id).await
    }
}

/// The other constructor [`build_actor`]'s callers choose between: wraps a
/// connected client so its `write_bindings` survives a transport failure
/// past this process's own lifetime (D-B1-4). `substrate_did` and
/// `queue_key` are carried alongside the client because
/// `WriteBindingsOutbox::enqueue` needs both and `SubstrateActor` itself
/// never learns which substrate -- or which of its caller's own logical
/// groupings -- it is bound to.
#[must_use]
pub fn build_durable_actor<T: SubstrateActor + WriteBindingsAttempt + 'static>(
    client: Arc<T>,
    substrate_did: String,
    queue_key: String,
    outbox: Arc<dyn WriteBindingsOutbox>,
) -> Arc<dyn SubstrateActor> {
    Arc::new(DurableActor { inner: client, substrate_did, queue_key, outbox })
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
    /// The binding epoch to stamp on each dependent member's own bindings,
    /// keyed by the dependent's `MemberRef` (M05A A5c §19.3/D-A5c-4;
    /// re-keyed from `LogicalServiceRef` in M05A A5e, D-A5e-2, since each
    /// member holds its own `service_bindings` row on the substrate) -- not
    /// a scalar, since one apply can deploy several dependent members whose
    /// counters have each advanced independently. A member absent from this
    /// map maps its bindings at epoch `0`, meaning "no supervisor has
    /// written here", which is what every caller through A5b still means by
    /// it.
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
                None => missing.push((svc.member_ref(), alias.clone())),
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

/// The substrate a member is currently placed on, per the **most recent**
/// row naming it, or `None` if it has never landed or was most recently
/// `REMOVE`d. `landed` must be ordered oldest-first, exactly as
/// `DeploymentJournal::get_completed_actions`/`get_completed_actions_for_
/// instance` return it.
///
/// `member_ref` is a `MemberRef`'s display string (M05A A5e, D-A5e-2): the
/// journal's action rows are keyed per managed member, not per logical
/// service, since two members of one logical service land as two separate
/// placements.
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
    member_ref: &str,
) -> Option<&'a ActionRecord> {
    match landed.iter().rev().find(|r| r.logical_ref == member_ref) {
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
        let l_ref = svc.member_ref().to_string();

        // D-A3-11: keyed on the DID, so an alias re-pointed at a different
        // node correctly redeploys rather than being skipped as already
        // done. `current_placement` reads the most recent row, not just any
        // ADD -- a `REMOVE` from `app forget` must force a redeploy, not a
        // skip, even though an older ADD for the same (ref, DID) pair is
        // still sitting in this record's history.
        if current_placement(&completed, &l_ref)
            .is_some_and(|r| r.substrate_did == target.substrate_did)
        {
            report.skipped.push(svc.member_ref());
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
                report.deployed.push(svc.member_ref());
            }
            Err(error) => {
                journal.update_action_state(action_id, ActionState::Failed)?;
                report.failures.push(ServiceFailure {
                    member_ref: svc.member_ref(),
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
    let identity = client
        .instance_identity(service_id)
        .await
        .context("failed to query the substrate for its derived instance identity")?;
    certificate_over_instance_identity(master, service_id, &identity, expires_hours)
}

/// The same round trip as [`certify_instance`], through the
/// [`SubstrateActor`] boundary -- what an unattended renewal takes, since
/// the supervisor's loop holds actors rather than raw clients and its
/// renewal control flow has to be testable against a fake substrate.
pub async fn certify_instance_via_actor(
    actor: &Arc<dyn SubstrateActor>,
    master: &Identity,
    service_id: &str,
    expires_hours: u64,
) -> Result<DelegationCertificate> {
    let identity = actor.instance_identity(service_id).await.map_err(|e| {
        anyhow::anyhow!("failed to query the substrate for its derived instance identity: {e}")
    })?;
    certificate_over_instance_identity(master, service_id, &identity, expires_hours)
}

/// The shared half of both `certify_instance` forms: everything after the
/// substrate has answered with the key it derives. One definition, so a
/// renewal and a deploy-time mint cannot drift apart on what they check
/// before signing.
///
/// **Bound to the querying client, not just this master.** The substrate
/// derives the key from its own node identity *and* the calling DID, so a
/// certificate minted through one client is rejected at install by any
/// other substrate, and by the same substrate reached as a different caller.
fn certificate_over_instance_identity(
    master: &Identity,
    service_id: &str,
    identity: &InstanceIdentity,
    expires_hours: u64,
) -> Result<DelegationCertificate> {
    let master_did = substrate::derive_did_key(&master.public_key());
    if master_did != service_id {
        anyhow::bail!(
            "master identity resolves to {master_did}, which does not match service_id \
             {service_id} -- a certificate for this pair would be rejected at install time"
        );
    }

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
            generation: 0,
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

        async fn renew_cert(
            &self,
            _service_id: String,
            _generation: u64,
            _instance_certificate: String,
        ) -> Result<(), String> {
            unimplemented!("not exercised by apply_plan's own tests")
        }

        async fn instance_identity(&self, _service_id: &str) -> Result<InstanceIdentity, String> {
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
            assets: None,
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
            member_index: 0,
            schedule: None,
            sharding_strategy: None,
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
                &p.services[0].member_ref().to_string(),
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
                &p.services[0].member_ref().to_string(),
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
        let l_ref = p.services[0].member_ref().to_string();
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

    // ── M05B B1: the durable actor's try-then-queue decision ────────────

    /// A fake standing in for `SyneroymClient`: `attempt_write_bindings`
    /// returns whatever `outcome` says, and `write_bindings`/every other
    /// `SubstrateActor` method is exercised only where a test names it, so
    /// each one's own `should_fail` flag drives it directly rather than
    /// sharing one flag across five unrelated actions.
    #[derive(Debug, Default)]
    struct DurableTestActor {
        write_bindings_outcome: Mutex<Option<Result<Vec<BindingWriteOutcome>>>>,
        apply_plan_should_fail: bool,
        restart_should_fail: bool,
        renew_cert_should_fail: bool,
        run_scheduled_should_fail: bool,
    }

    #[async_trait::async_trait]
    impl WriteBindingsAttempt for DurableTestActor {
        async fn attempt_write_bindings(
            &self,
            _write: BindingWrite,
        ) -> Result<Vec<BindingWriteOutcome>> {
            self.write_bindings_outcome.lock().unwrap().take().expect("outcome not set for test")
        }
    }

    #[async_trait::async_trait]
    impl SubstrateActor for DurableTestActor {
        async fn apply_plan(&self, _plan: WitDeploymentPlan) -> Result<(), String> {
            if self.apply_plan_should_fail { Err("apply_plan failed".to_string()) } else { Ok(()) }
        }

        async fn write_bindings(
            &self,
            _write: BindingWrite,
        ) -> Result<Vec<BindingWriteOutcome>, String> {
            unimplemented!("DurableActor<T> never calls T::write_bindings directly")
        }

        async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
            if self.restart_should_fail { Err("restart failed".to_string()) } else { Ok(()) }
        }

        async fn renew_cert(
            &self,
            _service_id: String,
            _generation: u64,
            _instance_certificate: String,
        ) -> Result<(), String> {
            if self.renew_cert_should_fail { Err("renew_cert failed".to_string()) } else { Ok(()) }
        }

        async fn run_scheduled(
            &self,
            _service_id: String,
            _generation: u64,
            _interface: String,
            _method: String,
            _params_json: Option<String>,
        ) -> Result<(), String> {
            if self.run_scheduled_should_fail {
                Err("run_scheduled failed".to_string())
            } else {
                Ok(())
            }
        }

        async fn instance_identity(&self, _service_id: &str) -> Result<InstanceIdentity, String> {
            unimplemented!("not exercised by the durable-actor tests")
        }

        async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
            unimplemented!("not exercised by the durable-actor tests")
        }
    }

    #[derive(Debug, Default)]
    struct RecordingOutbox {
        enqueued: Mutex<Vec<(String, String, BindingWrite)>>,
    }

    #[async_trait::async_trait]
    impl WriteBindingsOutbox for RecordingOutbox {
        async fn enqueue(&self, queue_key: &str, substrate_did: &str, write: &BindingWrite) {
            self.enqueued.lock().unwrap().push((
                queue_key.to_string(),
                substrate_did.to_string(),
                write.clone(),
            ));
        }
    }

    fn binding_write() -> BindingWrite {
        BindingWrite {
            service_id: "did:key:zSvc".to_string(),
            app_instance_id: "inst-1".to_string(),
            bindings: vec![],
            generation: 0,
        }
    }

    /// Test 14: D-B1-1 -- a successful call returns exactly what the trait
    /// returns today, with no queue involvement.
    #[tokio::test]
    async fn a_successful_write_bindings_returns_the_same_outcomes_it_returns_today() {
        let inner = Arc::new(DurableTestActor {
            write_bindings_outcome: Mutex::new(Some(Ok(vec![BindingWriteOutcome::Applied]))),
            ..Default::default()
        });
        let outbox = Arc::new(RecordingOutbox::default());
        let actor = build_durable_actor(
            inner,
            "did:key:zB".to_string(),
            "inst-1/backend@did:key:zB".to_string(),
            outbox.clone(),
        );

        let outcomes = actor.write_bindings(binding_write()).await.unwrap();
        assert_eq!(outcomes, vec![BindingWriteOutcome::Applied]);
        assert!(outbox.enqueued.lock().unwrap().is_empty());
    }

    /// Test 15 (the enqueue half; `Degraded` reporting is
    /// `push_bindings`'s own concern in `app_supervisor`, unchanged by this
    /// wrap per D-B1-1): a transport failure enqueues before returning the
    /// same error a bare client would have.
    #[tokio::test]
    async fn a_transport_failure_enqueues_and_returns_the_same_error_a_bare_client_would() {
        let inner = Arc::new(DurableTestActor {
            write_bindings_outcome: Mutex::new(Some(Err(anyhow::anyhow!("connection refused")))),
            ..Default::default()
        });
        let outbox = Arc::new(RecordingOutbox::default());
        let actor = build_durable_actor(
            inner,
            "did:key:zB".to_string(),
            "inst-1/backend@did:key:zB".to_string(),
            outbox.clone(),
        );

        let err = actor.write_bindings(binding_write()).await.unwrap_err();
        assert!(err.contains("connection refused"), "{err}");
        let enqueued = outbox.enqueued.lock().unwrap();
        assert_eq!(enqueued.len(), 1);
        assert_eq!(enqueued[0].0, "inst-1/backend@did:key:zB");
        assert_eq!(enqueued[0].1, "did:key:zB");
    }

    /// Test 16: a callee error (the substrate reached and refused the
    /// call) is never enqueued -- retrying the same refusal forever would
    /// be a second policy competing with the one that already answered.
    #[tokio::test]
    async fn a_callee_error_is_not_enqueued() {
        let callee_err =
            JsonRpcError { code: -32010, message: "not authorized".to_string(), data: None };
        let inner = Arc::new(DurableTestActor {
            write_bindings_outcome: Mutex::new(Some(Err(callee_err.into()))),
            ..Default::default()
        });
        let outbox = Arc::new(RecordingOutbox::default());
        let actor = build_durable_actor(
            inner,
            "did:key:zB".to_string(),
            "inst-1/backend@did:key:zB".to_string(),
            outbox.clone(),
        );

        let err = actor.write_bindings(binding_write()).await.unwrap_err();
        assert!(err.contains("not authorized"), "{err}");
        assert!(outbox.enqueued.lock().unwrap().is_empty(), "a callee error must not be queued");
    }

    /// Test 17: D-B1-3, failure-matrix row 3 -- `restart` never touches
    /// the outbox at all, whatever it returns.
    #[tokio::test]
    async fn a_failed_restart_is_never_enqueued() {
        let inner = Arc::new(DurableTestActor { restart_should_fail: true, ..Default::default() });
        let outbox = Arc::new(RecordingOutbox::default());
        let actor = build_durable_actor(
            inner,
            "did:key:zB".to_string(),
            "inst-1/backend@did:key:zB".to_string(),
            outbox.clone(),
        );

        let err = actor.restart("svc".to_string(), 0).await.unwrap_err();
        assert_eq!(err, "restart failed");
        assert!(outbox.enqueued.lock().unwrap().is_empty());
    }

    /// A scheduled run is never queued, the same
    /// shape `restart` already proved: `DurableActor` forwards the call
    /// straight through and touches the outbox not at all, whatever the
    /// inner actor returns.
    #[tokio::test]
    async fn a_durable_actor_runs_a_scheduled_tick_without_touching_the_queue() {
        let inner =
            Arc::new(DurableTestActor { run_scheduled_should_fail: true, ..Default::default() });
        let outbox = Arc::new(RecordingOutbox::default());
        let actor = build_durable_actor(
            inner,
            "did:key:zB".to_string(),
            "inst-1/backend@did:key:zB".to_string(),
            outbox.clone(),
        );

        let err = actor
            .run_scheduled(
                "svc".to_string(),
                0,
                "scheduled-driver".to_string(),
                "tick".to_string(),
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(err, "run_scheduled failed");
        assert!(outbox.enqueued.lock().unwrap().is_empty());
    }

    /// Test 18: D-B1-12 -- `apply_plan` and `renew_cert` embed a
    /// certificate that expires in hours, so neither is ever queued
    /// either, whatever it returns.
    #[tokio::test]
    async fn a_failed_apply_plan_and_renew_cert_are_never_enqueued() {
        let inner = Arc::new(DurableTestActor {
            apply_plan_should_fail: true,
            renew_cert_should_fail: true,
            ..Default::default()
        });
        let outbox = Arc::new(RecordingOutbox::default());
        let actor = build_durable_actor(
            inner,
            "did:key:zB".to_string(),
            "inst-1/backend@did:key:zB".to_string(),
            outbox.clone(),
        );

        assert!(
            actor
                .apply_plan(WitDeploymentPlan {
                    app_instance_id: "inst-1".to_string(),
                    blueprint_id: "syneroym:test".to_string(),
                    version: "1.0.0".to_string(),
                    services: vec![],
                })
                .await
                .is_err()
        );
        assert!(actor.renew_cert("svc".to_string(), 0, "cert".to_string()).await.is_err());
        assert!(outbox.enqueued.lock().unwrap().is_empty());
    }
}
