use super::*;

impl ControlPlaneService {
    /// The `proxy-*` verbs read and replay one service's queued work, so
    /// they are gated exactly as their per-service neighbours on this
    /// interface are -- `orchestrator/status`, node-wide or scoped to that
    /// one service. No new resource namespace: a second way to name the
    /// same authority is how an operator ends up holding a grant that does
    /// not mean what they think.
    pub(super) fn authorize_proxy_queue_access(
        &self,
        service_id: &str,
        caller: &CallerContext,
    ) -> Result<(), String> {
        self.authorize_proxy_queue(service_id, caller, Ability::ORCHESTRATOR_STATUS)
    }

    /// `proxy-replay` re-enqueues a call the worker then *sends*, so it is
    /// a lifecycle write and takes the write gate -- the same one
    /// `restart` uses, for the same reason. Listing the queues is a read
    /// and keeps the read gate; a holder of read-only status must not be
    /// able to make a service emit calls.
    pub(super) fn authorize_proxy_queue_write(
        &self,
        service_id: &str,
        caller: &CallerContext,
    ) -> Result<(), String> {
        self.authorize_proxy_queue(service_id, caller, Ability::ORCHESTRATOR_DEPLOY)
    }

    pub(super) fn authorize_proxy_queue(
        &self,
        service_id: &str,
        caller: &CallerContext,
        ability: &'static str,
    ) -> Result<(), String> {
        if service_id.is_empty() {
            return Err("a service id is required".to_string());
        }
        if self.has_node_wide_ability(caller, ability) {
            return Ok(());
        }
        let resource = ResourceUri(format!("substrate:{}/app/{service_id}", self.node_did));
        if caller.has_capability(&resource, &Ability(ability.to_string())) {
            return Ok(());
        }
        Err(format!("caller {} holds no {ability} grant for '{service_id}'", caller.caller_did))
    }

    /// The router's queue view, once it has been wired in. Absent means
    /// this node has no durable proxy path at all (coordinator mode, or a
    /// test harness with no router), which is a different answer from "the
    /// queue is empty" and is reported as such.
    pub(super) fn proxy_queue_inspector(&self) -> Result<Arc<dyn ProxyQueueInspector>, String> {
        self.proxy_queues
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| "this node keeps no durable proxy queues".to_string())
    }
}
