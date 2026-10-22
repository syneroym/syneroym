use super::{
    outbox_parked::{complete_row, delivered_row, fail_row, park_row},
    *,
};

impl ProxyRouter {
    /// same cross-service call exactly -- `service_system(caller)` plus a
    /// guest origin -- or authorization at the receiver would silently
    /// differ between the immediate attempt and every later one.
    pub(super) fn request_from(&self, call: &QueuedCall, target_service: String) -> ProxyRequest {
        ProxyRequest {
            target_service,
            interface: call.interface.clone(),
            method: call.method.clone(),
            params: call.params.clone(),
            caller: CallerContext::service_system(&call.caller_service_id),
            origin: CallOrigin::Guest { service_id: call.caller_service_id.clone() },
            protocol: ProxyProtocol::parse(call.protocol.as_deref())
                .unwrap_or(ProxyProtocol::JsonRpcV1),
            idempotent: true,
            idempotency_key: Some(call.idempotency_key.clone()),
            timeout: call.timeout_ms.map(Duration::from_millis),
        }
    }

    /// Delivers one queued item: re-resolve, then invoke. Split out so the
    /// worker and the immediate try-then-queue attempt cannot drift apart.
    /// Returns `Err` only when there is nothing to deliver *to* -- the
    /// stored dependency name no longer resolves to any member.
    ///
    /// Kept separate from the delivery itself because the two failures are
    /// not the same kind. "This name is bound to nobody" is settled: no
    /// number of retries invents a member, so it is terminal. "I could not
    /// reach the member it is bound to" is not settled at all, and is
    /// handled by the ordinary retry classification.
    ///
    /// Re-resolved on every attempt and never stored, so a binding
    /// re-pushed while the item waited takes effect (ADR-0021 §2).
    pub(super) fn resolve_queued_target(&self, call: &QueuedCall) -> Result<String, ProxyError> {
        self.outbox
            .as_ref()
            .ok_or_else(|| ProxyError::Internal("no durable outbox on this node".to_string()))?
            .resolve_target(call)
    }

    /// Delivers one queued item directly, or deposits it at the receiver's
    /// mailbox when the receiver cannot be reached.
    pub(super) async fn deliver_queued(
        &self,
        call: &QueuedCall,
        target: String,
        max_wait_secs: u64,
    ) -> Result<Delivery, ProxyError> {
        self.deliver_or_deposit_inner(&self.request_from(call, target), max_wait_secs).await
    }

    /// [`Self::invoke_local`] under the receiver-side fence.
    ///
    /// A call with no key runs straight through, touching no store: that
    /// is every call on the hot path today, and it is what keeps the
    /// fence off the existing call budget.
    /// One pass over every outbox this node currently has open, plus every
    /// deployed service whose queue file already exists on disk -- the
    /// second half is what lets a restart pick up items written before it.
    ///
    /// Returns how many items it settled, for tests and metrics.
    pub async fn drain_outboxes_once(&self) -> usize {
        let Some(outbox) = &self.outbox else { return 0 };
        let deployed: BTreeSet<String> = self
            .registry
            .get_all_endpoints()
            .into_iter()
            .map(|(service_id, _, _)| service_id)
            .collect();

        let mut services: BTreeSet<String> = outbox.open_services().into_iter().collect();
        for service_id in &deployed {
            if outbox.queue_file_exists(service_id) {
                services.insert(service_id.clone());
            }
        }

        let mut settled = 0;
        for service_id in services {
            let Ok(queue) = outbox.queue_for(&service_id).await else { continue };
            // Nothing removes a service's data directory on undeploy, so
            // an outbox can outlive the service that wrote it. Its items
            // are completed silently: delivering would resurrect intent an
            // operator withdrew, and dead-lettering would raise noise
            // about a service nobody is going to act on.
            let still_deployed = deployed.contains(&service_id);
            settled += self.drain_one_outbox(&queue, still_deployed).await;
            settled += self.drain_parked_outbox(&queue, &service_id, still_deployed).await;
        }
        settled
    }

    async fn drain_one_outbox(&self, queue: &Queue, still_deployed: bool) -> usize {
        let now = proxy_outbox::now_ms();
        let claimed = {
            let queue = queue.clone();
            match tokio::task::spawn_blocking(move || {
                queue.claim_due(now, proxy_outbox::CLAIM_LIMIT_PER_TICK)
            })
            .await
            {
                Ok(Ok(items)) => items,
                _ => return 0,
            }
        };

        let settled = claimed.len();
        for item in claimed {
            self.settle_pending(queue, item, still_deployed, now).await;
        }
        settled
    }

    /// One claimed pending item: deliver it, deposit it, or end it.
    async fn settle_pending(&self, queue: &Queue, item: QueueItem, still_deployed: bool, now: i64) {
        if !still_deployed {
            complete_row(queue, item.id).await;
            return;
        }
        // A claim that never resolves through `fail`/`complete` -- a
        // panic, a crash, a shutdown caught mid-delivery -- is bounded
        // by this rather than by the attempt count, which only `fail`
        // advances.
        if item.claim_count > u32::from(queue.max_attempts()) {
            fail_row(queue, item.id, now, "claimed repeatedly without ever completing", true).await;
            return;
        }
        // Only a call to a receiver with a mailbox can be this old: deposit
        // and direct attempts take turns without using the attempt budget.
        if now - item.created_at > proxy_outbox::MAX_ROW_AGE_MS {
            fail_row(queue, item.id, now, "the receiver never took this call", true).await;
            return;
        }
        let Ok(call) = serde_json::from_slice::<QueuedCall>(&item.payload) else {
            fail_row(queue, item.id, now, "queued payload is unreadable", true).await;
            return;
        };
        // A name bound to nobody is terminal on its own terms: this is
        // a failure to have a target at all, not a failed delivery, so
        // it does not go through the retry classification.
        let target = match self.resolve_queued_target(&call) {
            Ok(target) => target,
            Err(e) => {
                proxy_outbox::log_delivery_failure(&call.idempotency_key, &e);
                fail_row(queue, item.id, now, &e.to_string(), true).await;
                return;
            }
        };
        let max_wait_secs = proxy_outbox::max_wait_secs(item.created_at, now);
        match self.deliver_queued(&call, target, max_wait_secs).await {
            Ok(Delivery::Direct(_)) => delivered_row(queue, item.id).await,
            Ok(Delivery::Deposited(ticket)) => park_row(queue, &item, now, &ticket).await,
            Err(e) => self.settle_failed_delivery(queue, &item, &call, &e, now).await,
        }
    }

    async fn settle_failed_delivery(
        &self,
        queue: &Queue,
        item: &QueueItem,
        call: &QueuedCall,
        error: &ProxyError,
        now: i64,
    ) {
        match proxy_outbox::disposition_of(error) {
            // The receiver already ran this item and could not hand back
            // its result. That is a delivery reported through the error
            // channel, so the item is done.
            Disposition::Delivered => delivered_row(queue, item.id).await,
            disposition => {
                proxy_outbox::log_delivery_failure(&call.idempotency_key, error);
                let terminal = disposition == Disposition::Terminal;
                fail_row(queue, item.id, now, &error.to_string(), terminal).await;
            }
        }
    }

    /// The resident loop: drains outboxes and sweeps saga logs, then races
    /// cancellation into both -- see `run_async_worker`'s own doc for why
    /// it carries a name that says both, not just the first.
    pub async fn run_async_worker(self: Arc<Self>, tick: Duration, cancel: CancellationToken) {
        if self.outbox.is_none() && self.sagas.is_none() {
            return;
        }
        let mut ticker = time::interval(tick);
        ticker.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                _ = ticker.tick() => {}
            }
            tokio::select! {
                () = cancel.cancelled() => return,
                (outbox_settled, saga_settled) = async {
                    let a = self.drain_outboxes_once().await;
                    let b = self.sweep_sagas_once().await;
                    (a, b)
                } => {
                    if outbox_settled > 0 || saga_settled > 0 {
                        debug!(
                            outbox_settled,
                            saga_settled,
                            "async worker settled queued calls and saga undos"
                        );
                    }
                }
            }
        }
    }
}
