//! SubstrateActor trait and client/durable implementations.

use std::{fmt, sync::Arc};

use anyhow::Result;
use syneroym_rpc::JsonRpcError;

use crate::{
    BindingWrite, BindingWriteOutcome, DeploymentPlan as WitDeploymentPlan, InstanceIdentity,
    SyneroymClient,
};
/// ADR-0021 §5's narrow "apply this action to that substrate" boundary.
/// Three actions, not one: this trait (as `PlanApplier`) began when
/// applying a plan was the only action, and later gained the two the
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
    /// reinstall -- the unattended-renewal path. On the trait for the same
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
    /// ever recorded one -- the supervisor's own resident loop uses this
    /// to detect supersession (ADR-0021 §4). Behind this trait, not
    /// called directly against `SyneroymClient`, so the loop's
    /// superseded/skip decisions are testable against a fake substrate
    /// with no live connection.
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
/// expressions. `roymctl` and the SDK's own e2e fixtures
/// call this deliberately: a one-shot process exits when its
/// command finishes, so a durable queue behind it would be written and never
/// drained. [`build_durable_actor`] is the other call this function's
/// callers choose between, for the one action-owner that keeps a worker
/// alive to drain what it enqueues.
#[must_use]
pub fn build_actor<T: SubstrateActor + 'static>(actor: Arc<T>) -> Arc<dyn SubstrateActor> {
    actor
}

/// What a durable actor's `write_bindings` enqueues onto when it cannot
/// reach its target (try synchronously first, enqueue only on
/// transport failure). A trait here, rather than this crate depending on
/// `syneroym-async-queue` directly, so the queue's storage, its worker, and
/// the `instance_lock` it coordinates with stay owned by the crate that
/// also owns those (`syneroym-app-supervisor`) -- this crate only needs to
/// know that *something* durable exists to hand a failed write to.
#[async_trait::async_trait]
pub trait WriteBindingsOutbox: fmt::Debug + Send + Sync {
    /// `queue_key` is opaque to this crate -- the caller's own grouping key
    /// (the supervisor's is `(instance, logical_ref, substrate)`),
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
/// that already reported it.
#[must_use]
pub fn is_callee_error(err: &anyhow::Error) -> bool {
    err.downcast_ref::<JsonRpcError>().is_some()
}

/// `true` when `err` is specifically the substrate answering that this
/// write's own target no longer exists -- narrower
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
/// starting) that a later retry would have cleared.
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
/// (`restart`, `apply_plan`, and `renew_cert` are never
/// queued, the last two because they embed a certificate that expires in
/// hours, not because queueing them is hard). `write_bindings` attempts
/// synchronously first and, only on a transport failure, also enqueues onto
/// `outbox` before returning the same error a bare client would have
/// returned -- so a caller reading the return value sees no
/// difference from today, and `push_bindings`'s existing alert/`Degraded`
/// handling needs no change at all.
#[derive(Debug)]
struct DurableActor<T> {
    inner: Arc<T>,
    substrate_did: String,
    /// The caller's own grouping key, bound at construction --
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
/// past this process's own lifetime. `substrate_did` and
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
