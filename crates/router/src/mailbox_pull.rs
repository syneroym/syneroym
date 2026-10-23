//! The receiver side of a service mailbox (ADR-0026).
//!
//! A hosted service whose signed record names a mailbox has a queue there.
//! One worker for the node registers the queue, claims waiting messages,
//! opens each one with the service's mailbox key, checks the sender, and
//! hands the call to the service through the same fence a call from the
//! wire meets. It then acknowledges the message.

mod message;
mod notes;
mod service;

use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, PoisonError},
    time::Duration,
};

use serde_json::json;
use syneroym_control_plane::{ControlPlaneService, MailboxPullControl};
use syneroym_core::{
    dht_registry::SignedEndpointInfo,
    local_registry::EndpointRegistry,
    mailbox::{METHOD_UNREGISTER, PullSummary, codes},
};
use syneroym_mailbox_format::{MailboxSecretKey, MessageContent};
use syneroym_rpc::{CallerContext, ProxyError};
use tokio::{
    sync::{Mutex, Notify},
    time,
};
use tokio_util::sync::CancellationToken;
use tracing::debug;

use self::{
    notes::Notes,
    service::{ServicePass, ServiceState},
};
use crate::{
    proxy::{AnchorCache, ProxyRouter},
    route_handler::{RouteHandler, deferred::DeferredOutcome},
};

/// How often a pass runs when nothing wakes the worker.
const PULL_INTERVAL: Duration = Duration::from_secs(60);
/// How long a registration lasts before it is sent again. A `claim` keeps the
/// queue alive, so this only makes sure a lost queue comes back.
const REREGISTER_INTERVAL: Duration = Duration::from_secs(3600);
const SERVED_RECORD_CHECK_INTERVAL: Duration = Duration::from_secs(3600);
const SERVED_RECORD_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);
/// The most claim rounds one service gets in one pass.
const MAX_CLAIM_ROUNDS: usize = 8;

/// What the puller needs from the node's control plane.
#[async_trait::async_trait]
pub trait ReceiverVault: Send + Sync {
    /// The service's mailbox secret key, or `None` when it has none.
    async fn mailbox_key(&self, service_id: &str) -> anyhow::Result<Option<MailboxSecretKey>>;
    /// The record the deployer signed, as stored at deploy.
    async fn deploy_record(&self, service_id: &str) -> anyhow::Result<Option<SignedEndpointInfo>>;
}

/// How a call from the mailbox reaches a service on this node.
#[async_trait::async_trait]
pub trait DeferredDispatch: Send + Sync {
    async fn dispatch_deferred(
        &self,
        service_id: &str,
        interface: &str,
        content: &MessageContent,
        caller: &CallerContext,
    ) -> DeferredOutcome;
}

/// The mailbox receiver of one node.
pub struct MailboxPuller {
    proxy: Arc<ProxyRouter>,
    dispatch: Arc<dyn DeferredDispatch>,
    vault: Arc<dyn ReceiverVault>,
    registry: EndpointRegistry,
    state: Mutex<PullState>,
    wake: Notify,
    /// One gate for each service, outside the pass lock: a `register` and a
    /// `release` of the same service never cross.
    register_gates: std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

#[derive(Default)]
struct PullState {
    services: HashMap<String, ServiceState>,
    notes: Notes,
}

impl MailboxPuller {
    #[must_use]
    pub fn new(
        proxy: Arc<ProxyRouter>,
        dispatch: Arc<dyn DeferredDispatch>,
        vault: Arc<dyn ReceiverVault>,
        registry: EndpointRegistry,
    ) -> Self {
        Self {
            proxy,
            dispatch,
            vault,
            registry,
            state: Mutex::new(PullState::default()),
            wake: Notify::new(),
            register_gates: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// The register gate of `service_id`, made on first use.
    fn register_gate(&self, service_id: &str) -> Arc<Mutex<()>> {
        let mut gates = self.register_gates.lock().unwrap_or_else(PoisonError::into_inner);
        gates.entry(service_id.to_string()).or_default().clone()
    }

    /// Pulls at start, then every minute, and when woken, until cancelled.
    pub async fn run(self: Arc<Self>, cancel: CancellationToken) {
        loop {
            self.pull_once().await;
            tokio::select! {
                () = cancel.cancelled() => return,
                () = self.wake.notified() => {}
                () = time::sleep(PULL_INTERVAL) => {}
            }
        }
    }

    /// One pass over every hosted service with a mailbox.
    pub async fn pull_once(&self) -> PullSummary {
        let mut state = self.state.lock().await;
        let PullState { services, notes } = &mut *state;
        let hosted = self.registry.hosted_mailboxes();
        // The control plane removed the queue of a service that is gone;
        // only its state is left to forget.
        services.retain(|id, _| hosted.iter().any(|(h, _)| h == id));
        self.drop_unused_gates(&hosted);
        let mut anchors = AnchorCache::new();
        let mut summary = PullSummary::default();
        for (service_id, fact) in &hosted {
            summary.services += 1;
            let entry = services.entry(service_id.clone()).or_default();
            let mut pass = ServicePass {
                service_id,
                fact,
                entry,
                notes,
                anchors: &mut anchors,
                summary: &mut summary,
            };
            self.pull_service(&mut pass).await;
        }
        summary
    }

    /// Drops the gate of a service that is gone, when no one holds it. A
    /// `release` in progress keeps its own. A poisoned lock only skips this
    /// clean-up.
    fn drop_unused_gates(&self, hosted: &[(String, syneroym_core::mailbox::HostedMailbox)]) {
        if let Ok(mut gates) = self.register_gates.lock() {
            gates.retain(|id, gate| {
                Arc::strong_count(gate) > 1 || hosted.iter().any(|(h, _)| h == id)
            });
        }
    }
}

#[async_trait::async_trait]
impl MailboxPullControl for MailboxPuller {
    async fn pull_now(&self) -> PullSummary {
        self.pull_once().await
    }

    fn wake(&self) {
        self.wake.notify_one();
    }

    /// Takes no pass lock: the control plane calls this inside an undeploy
    /// or deploy, which must not wait for a pass. It takes the service's
    /// register gate, so a `register` in flight lands first and this
    /// `unregister` removes it.
    async fn release(&self, service_id: &str, mailbox: &str) {
        let Some(instance) = self.proxy.service_instance(service_id) else {
            debug!(
                service_id,
                "no instance certificate; the old queue stays until its lifetime ends"
            );
            return;
        };
        let gate = self.register_gate(service_id);
        let _held = gate.lock().await;
        match self.proxy.call_mailbox(&instance, mailbox, METHOD_UNREGISTER, json!({})).await {
            Ok(_) => {}
            // A newer instance holds the queue: the expected answer after a
            // relocation.
            Err(ProxyError::Callee { code, .. }) if code == codes::STALE_INSTANCE => {}
            Err(e) => debug!(service_id, mailbox, error = %e, "could not remove the old queue"),
        }
    }
}

impl fmt::Debug for MailboxPuller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MailboxPuller").finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl ReceiverVault for ControlPlaneService {
    async fn mailbox_key(&self, service_id: &str) -> anyhow::Result<Option<MailboxSecretKey>> {
        ControlPlaneService::mailbox_key(self, service_id).await
    }

    async fn deploy_record(&self, service_id: &str) -> anyhow::Result<Option<SignedEndpointInfo>> {
        ControlPlaneService::deploy_record(self, service_id).await
    }
}

#[async_trait::async_trait]
impl DeferredDispatch for RouteHandler {
    async fn dispatch_deferred(
        &self,
        service_id: &str,
        interface: &str,
        content: &MessageContent,
        caller: &CallerContext,
    ) -> DeferredOutcome {
        RouteHandler::dispatch_deferred(self, service_id, interface, content, caller).await
    }
}

#[cfg(test)]
mod tests;
