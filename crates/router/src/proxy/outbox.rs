use super::*;

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

    /// Writes a dead letter for a failed call that carried a fence.
    ///
    /// An **unkeyed** call writes nothing, and that is the rule rather
    /// than an omission: its caller is alive and holding the error, so
    /// this is not silent loss, and there would be nothing safe to replay
    /// -- a replayable dead letter for a call with no fence *is* a second
    /// delivery of an unfenced call.
    ///
    /// The recorded target is the DID this attempt actually resolved to,
    /// not the dependency name: this row describes one specific attempt an
    /// operator may choose to repeat, and the resolution already happened
    /// before the request existed.
    /// Re-resolved on every attempt and never stored, so a binding
    /// re-pushed while the item waited takes effect (ADR-0021 §2).
    pub(super) fn resolve_queued_target(&self, call: &QueuedCall) -> Result<String, ProxyError> {
        self.outbox
            .as_ref()
            .ok_or_else(|| ProxyError::Internal("no durable outbox on this node".to_string()))?
            .resolve_target(call)
    }

    pub(super) async fn deliver_queued(
        &self,
        call: &QueuedCall,
        target: String,
    ) -> Result<Value, ProxyError> {
        self.invoke_inner(&self.request_from(call, target)).await
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

        let mut settled = 0;
        for item in claimed {
            let queue = queue.clone();
            if !still_deployed {
                let _ = tokio::task::spawn_blocking(move || queue.complete(item.id)).await;
                settled += 1;
                continue;
            }
            // A claim that never resolves through `fail`/`complete` -- a
            // panic, a crash, a shutdown caught mid-delivery -- is bounded
            // by this rather than by the attempt count, which only `fail`
            // advances.
            if item.claim_count > u32::from(queue.max_attempts()) {
                let _ = tokio::task::spawn_blocking(move || {
                    queue.fail(item.id, now, "claimed repeatedly without ever completing", true)
                })
                .await;
                settled += 1;
                continue;
            }
            let Ok(call) = serde_json::from_slice::<QueuedCall>(&item.payload) else {
                let _ = tokio::task::spawn_blocking(move || {
                    queue.fail(item.id, now, "queued payload is unreadable", true)
                })
                .await;
                settled += 1;
                continue;
            };
            settled += 1;
            // A name bound to nobody is terminal on its own terms: this is
            // a failure to have a target at all, not a failed delivery, so
            // it does not go through the retry classification.
            let target = match self.resolve_queued_target(&call) {
                Ok(target) => target,
                Err(e) => {
                    proxy_outbox::log_delivery_failure(&call.idempotency_key, &e);
                    let message = e.to_string();
                    let outcome = tokio::task::spawn_blocking(move || {
                        queue.fail(item.id, now, &message, true)
                    })
                    .await;
                    if let Ok(Ok(FailOutcome::DeadLettered { .. })) = outcome {
                        metrics::counter!("substrate.proxy.outbox.dead_lettered").increment(1);
                    }
                    continue;
                }
            };
            match self.deliver_queued(&call, target).await {
                Ok(_) => {
                    metrics::counter!("substrate.proxy.outbox.delivered").increment(1);
                    let _ = tokio::task::spawn_blocking(move || queue.complete(item.id)).await;
                }
                Err(e) => match proxy_outbox::disposition_of(&e) {
                    // The receiver already ran this item and could not
                    // hand back its result. That is a delivery reported
                    // through the error channel, so the item is done.
                    Disposition::Delivered => {
                        metrics::counter!("substrate.proxy.outbox.delivered").increment(1);
                        let _ = tokio::task::spawn_blocking(move || queue.complete(item.id)).await;
                    }
                    disposition => {
                        proxy_outbox::log_delivery_failure(&call.idempotency_key, &e);
                        let terminal = disposition == Disposition::Terminal;
                        let message = e.to_string();
                        let outcome = tokio::task::spawn_blocking(move || {
                            queue.fail(item.id, now, &message, terminal)
                        })
                        .await;
                        if let Ok(Ok(FailOutcome::DeadLettered { .. })) = outcome {
                            metrics::counter!("substrate.proxy.outbox.dead_lettered").increment(1);
                        }
                    }
                },
            }
        }
        settled
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
