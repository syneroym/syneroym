use super::*;

fn no_saga_store_error() -> ProxyError {
    ProxyError::Internal(
        "this node keeps no per-service storage, so it has no durable saga log".to_string(),
    )
}

fn parse_rpc_saga_state(state: &str) -> RpcSagaState {
    match state {
        "open" => RpcSagaState::Open,
        "compensating" => RpcSagaState::Compensating,
        "compensated" => RpcSagaState::Compensated,
        // A stored state this router does not recognise is a schema-level
        // bug, not a value an operator should ever act on differently from
        // an ordinary failure -- `SagaLog` itself already validates every
        // state it reads back, so this arm is unreachable in practice.
        _ => RpcSagaState::Failed,
    }
}

pub(super) fn rpc_saga_info_from(info: QueueSagaInfo) -> SagaInfo {
    SagaInfo {
        saga_id: info.saga_id,
        name: info.name,
        state: parse_rpc_saga_state(&info.state),
        steps: info.steps,
        compensated_steps: info.compensated_steps,
        created_at: info.created_at,
        deadline_at: info.deadline_at,
        last_error: info.last_error,
    }
}

/// Builds the live cross-service call one saga step's own forward call
/// sends -- byte-identical in shape to `request_from`'s reasoning for a
/// queued call: the identity has to match what `proxy::Host::call` would
/// build for the same call, or authorization at the receiver would
/// silently diverge between a live step and one replayed later.
fn request_from_step(
    req: &SagaStepRequest,
    target_service: String,
    timeout_ms: u64,
) -> ProxyRequest {
    ProxyRequest {
        target_service,
        interface: req.interface.clone(),
        method: req.method.clone(),
        params: req.params.clone(),
        caller: CallerContext::service_system(&req.caller_service_id),
        origin: CallOrigin::Guest { service_id: req.caller_service_id.clone() },
        protocol: ProxyProtocol::parse(req.protocol.as_deref()).unwrap_or(ProxyProtocol::JsonRpcV1),
        idempotent: req.idempotency_key.is_some(),
        idempotency_key: req.idempotency_key.clone(),
        timeout: Some(Duration::from_millis(timeout_ms)),
    }
}

impl ProxyRouter {
    /// Opens a saga and returns its host-minted id (never guest-chosen).
    /// Refuses up front on the same two grounds `enqueue` does, for the
    /// identical reason: every undo this saga may later send travels under
    /// the caller's own identity, so a caller with no unexpired instance
    /// certificate would have every one of them refused as anonymous.
    pub(super) async fn saga_begin_impl(&self, req: SagaBegin) -> Result<String, ProxyError> {
        let Some(store) = &self.sagas else { return Err(no_saga_store_error()) };

        let cert = self
            .registry
            .instance_cert(&req.caller_service_id)
            .filter(|c| !c.is_expired())
            .ok_or_else(|| {
                ProxyError::PermissionDenied(format!(
                    "service '{}' holds no unexpired instance certificate, so every undo this \
                     saga may later send would be refused as anonymous",
                    req.caller_service_id
                ))
            })?;

        let config = store.config().clone();
        let deadline_ms = match req.deadline_secs {
            None => config.default_deadline_ms,
            Some(secs) => {
                let ms = i64::try_from(secs.saturating_mul(1000)).unwrap_or(i64::MAX);
                if ms > config.max_deadline_ms {
                    return Err(ProxyError::PermissionDenied(format!(
                        "requested saga deadline {secs}s exceeds this node's ceiling of {}s",
                        config.max_deadline_ms / 1000
                    )));
                }
                ms
            }
        };

        let now = proxy_outbox::now_ms();
        // `deadline_ms` above is a duration; `sagas.deadline_at` is the
        // absolute instant `SagaLog::abandoned` compares against `now`.
        let deadline_at = now.saturating_add(deadline_ms);

        // Not a refusal: a *managed* instance's certificate is renewed on
        // every supervisor pass, so its own current expiry cannot decide
        // whether a long deadline is sound. An unmanaged instance's can,
        // and this is the only signal the host has for it.
        let cert_expires_ms =
            i64::try_from(cert.expires_at_secs.saturating_mul(1000)).unwrap_or(i64::MAX);
        if deadline_at > cert_expires_ms {
            warn!(
                caller = %req.caller_service_id,
                deadline_at,
                cert_expires_at = cert_expires_ms,
                "saga deadline outlives the caller's current instance certificate; a service \
                 whose certificate is not renewed before then cannot compensate past its expiry"
            );
        }

        let saga_id = uuid::Uuid::new_v4().to_string();
        let log = store.log_for(&req.caller_service_id).await?;
        let id = saga_id.clone();
        let name = req.name.clone();
        let app_instance_id = req.app_instance_id.clone();
        task::spawn_blocking(move || {
            log.begin(&id, &name, app_instance_id.as_deref(), deadline_at, now)
        })
        .await
        .map_err(|e| ProxyError::Internal(format!("saga begin task failed: {e}")))?
        .map_err(|e| ProxyError::Internal(e.to_string()))?;
        metrics::counter!("substrate.proxy.saga.opened").increment(1);
        Ok(saga_id)
    }

    /// Takes one forward step: records its intent, dispatches the call, and
    /// records the outcome. Refuses the same two node/self targets
    /// `enqueue` refuses, and for the same reason: a node-level target's
    /// undo can never be fenced, and a self-target's undo cannot be
    /// rebuilt under the right caller identity.
    pub(super) async fn saga_step_impl(&self, req: SagaStepRequest) -> Result<Value, ProxyError> {
        let started = Instant::now();
        let Some(store) = &self.sagas else { return Err(no_saga_store_error()) };

        if call_dedup::is_node_level_interface(&req.interface) {
            return Err(ProxyError::PermissionDenied(format!(
                "node-level interface '{}' cannot take part in a saga: its compensation could \
                 never be fenced, so the first undo would fail the whole saga",
                req.interface
            )));
        }

        let log = store.log_for(&req.caller_service_id).await?;
        let target = store.resolve_step_target(
            req.app_instance_id.as_deref(),
            &req.target,
            req.routing_key.as_deref(),
        )?;

        if target == req.caller_service_id {
            return Err(ProxyError::UnsupportedTarget(format!(
                "service '{}' cannot make a saga step against itself: its own compensation would \
                 run under a different caller identity than the step did",
                req.caller_service_id
            )));
        }

        let target_json = serde_json::to_string(&req.target)
            .map_err(|e| ProxyError::Internal(format!("saga step target unencodable: {e}")))?;
        let params_bytes = serde_json::to_vec(&req.params)
            .map_err(|e| ProxyError::Internal(format!("saga step params unencodable: {e}")))?;
        if params_bytes.len() > MAX_SAGA_PAYLOAD_BYTES {
            return Err(ProxyError::Internal(format!(
                "saga step params exceed the {MAX_SAGA_PAYLOAD_BYTES} byte limit"
            )));
        }
        let intent = StepIntent {
            target: target_json,
            routing_key: req.routing_key.clone(),
            interface: req.interface.clone(),
            method: req.method.clone(),
            params: params_bytes,
        };

        let now = proxy_outbox::now_ms();
        let saga_id = req.saga_id.clone();
        let log_for_write = log.clone();
        let idx =
            task::spawn_blocking(move || log_for_write.record_step_intent(&saga_id, &intent, now))
                .await
                .map_err(|e| ProxyError::Internal(format!("saga step task failed: {e}")))?
                .map_err(|e| ProxyError::Internal(e.to_string()))?;

        // Budget: the guest's own `timeout_ms` when it named one, otherwise
        // what is *left* of this node's step budget after this function's
        // own bookkeeping so far (the log open plus the intent write) --
        // one second inside the guest's epoch, not the proxy's 30s default,
        // and minus the bookkeeping rather than ignoring it.
        let budget_ms = req.timeout_ms.unwrap_or_else(|| {
            step_call_budget_ms(store.config().step_timeout_ms, started.elapsed())
        });

        // `invoke`, not `invoke_inner`: a keyed step still earns the
        // guest proxy outbox's own dead letter on failure, an operator
        // surface this reuses rather than duplicates.
        let outcome = self.invoke(request_from_step(&req, target, budget_ms)).await;

        let saga_id = req.saga_id.clone();
        let now = proxy_outbox::now_ms();
        match &outcome {
            Ok(value) => {
                let result_bytes = serde_json::to_vec(value).ok();
                let _ = task::spawn_blocking(move || {
                    log.record_step_outcome(&saga_id, idx, result_bytes.as_deref(), None, now)
                })
                .await;
            }
            Err(e) => {
                let error_text = e.to_string();
                let _ = task::spawn_blocking(move || {
                    log.record_step_outcome(&saga_id, idx, None, Some(&error_text), now)
                })
                .await;
            }
        }
        outcome
    }

    /// The workflow reached its goal: drops the log. Refuses on anything
    /// but `open`.
    pub(super) async fn saga_commit_impl(
        &self,
        service_id: &str,
        saga_id: &str,
    ) -> Result<(), ProxyError> {
        let Some(store) = &self.sagas else { return Err(no_saga_store_error()) };
        let log = store.log_for(service_id).await?;
        let id = saga_id.to_string();
        task::spawn_blocking(move || log.commit(&id))
            .await
            .map_err(|e| ProxyError::Internal(format!("saga commit task failed: {e}")))?
            .map_err(|e| ProxyError::Internal(e.to_string()))?;
        metrics::counter!("substrate.proxy.saga.committed").increment(1);
        Ok(())
    }

    /// Marks the saga for compensation. Returns immediately -- the walk
    /// runs on the async worker's next tick, never inline. Idempotent:
    /// asking an already-compensating saga to compensate again is a no-op,
    /// not an error.
    pub(super) async fn saga_compensate_impl(
        &self,
        service_id: &str,
        saga_id: &str,
    ) -> Result<(), ProxyError> {
        let Some(store) = &self.sagas else { return Err(no_saga_store_error()) };
        let log = store.log_for(service_id).await?;
        let now = proxy_outbox::now_ms();
        let id = saga_id.to_string();
        let log_for_mark = log.clone();
        let transitioned = task::spawn_blocking(move || log_for_mark.mark_compensating(&id, now))
            .await
            .map_err(|e| ProxyError::Internal(format!("saga compensate task failed: {e}")))?
            .map_err(|e| ProxyError::Internal(e.to_string()))?;
        if transitioned {
            return Ok(());
        }
        let id = saga_id.to_string();
        let info = task::spawn_blocking(move || log.status(&id))
            .await
            .map_err(|e| ProxyError::Internal(format!("saga status task failed: {e}")))?
            .map_err(|e| ProxyError::Internal(e.to_string()))?;
        match info {
            None => Err(ProxyError::Internal(format!("unknown saga {saga_id}"))),
            Some(info) if info.state == "compensating" => Ok(()),
            Some(info) => Err(ProxyError::Internal(format!(
                "saga {saga_id} is {}; only an open saga can be asked to compensate",
                info.state
            ))),
        }
    }

    pub(super) async fn saga_status_impl(
        &self,
        service_id: &str,
        saga_id: &str,
    ) -> Result<SagaInfo, ProxyError> {
        let Some(store) = &self.sagas else { return Err(no_saga_store_error()) };
        let Some(log) = store.existing_log_for(service_id).await? else {
            return Err(ProxyError::Internal(format!("unknown saga {saga_id}")));
        };
        let id = saga_id.to_string();
        let info = task::spawn_blocking(move || log.status(&id))
            .await
            .map_err(|e| ProxyError::Internal(format!("saga status task failed: {e}")))?
            .map_err(|e| ProxyError::Internal(e.to_string()))?
            .ok_or_else(|| ProxyError::Internal(format!("unknown saga {saga_id}")))?;
        Ok(rpc_saga_info_from(info))
    }
}

impl ProxyRouter {
    /// pending a second absent tick, so an inconsistent view after a panic
    /// costs at most one extra tick of delay before a real undeploy is
    /// confirmed, never a lost or duplicated drop.
    fn saga_undeploy_candidates_lock(&self) -> std::sync::MutexGuard<'_, BTreeSet<String>> {
        self.saga_undeploy_candidates.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// One pass over every saga log this node has open, plus every deployed
    /// service whose log file already exists -- the second half is what
    /// lets a restart pick up a saga written before it. Mirrors
    /// `drain_outboxes_once`'s shape, but not its undeployed-service rule
    /// verbatim: the outbox only completes not-yet-delivered intent on one
    /// absent tick, which is recoverable; dropping a saga log is not, so
    /// this sweep requires the service to be absent on two consecutive
    /// ticks before it drops anything (`saga_undeploy_candidates`).
    ///
    /// Returns how many sagas it settled (a step undone, or a saga finished
    /// or failed), for tests and metrics.
    pub async fn sweep_sagas_once(&self) -> usize {
        let Some(store) = &self.sagas else { return 0 };
        let deployed: BTreeSet<String> = self
            .registry
            .get_all_endpoints()
            .into_iter()
            .map(|(service_id, _, _)| service_id)
            .collect();

        let mut services: BTreeSet<String> = store.open_services().into_iter().collect();
        for service_id in &deployed {
            if store.log_file_exists(service_id) {
                services.insert(service_id.clone());
            }
        }

        let now = proxy_outbox::now_ms();
        let mut settled = 0;
        for service_id in services {
            // Not `else { continue }` silently: a locked vault
            // makes every open fail with `KekRequired`, which is the state
            // of every substrate after a restart until an operator injects
            // the KEK -- silence there means a node that compensates
            // nothing and says nothing about it.
            let log = match store.log_for(&service_id).await {
                Ok(log) => log,
                Err(e) => {
                    warn!(service_id = %service_id, error = %e, "cannot open saga log this sweep");
                    continue;
                }
            };

            if !deployed.contains(&service_id) {
                // A single absent tick is not enough: undeploy removes a
                // service's endpoints one interface at a time, so a
                // redeploy can leave a service transiently missing from
                // `get_all_endpoints()` for exactly one tick. Only a
                // *second* consecutive absence drops the log.
                let confirmed = !self.saga_undeploy_candidates_lock().insert(service_id.clone());
                if !confirmed {
                    debug!(
                        service_id = %service_id,
                        "service absent from the registry this tick; sagas kept pending a second consecutive absence"
                    );
                    continue;
                }

                // Nothing removes a service's data directory on undeploy.
                // Its sagas are dropped rather than compensated: the
                // operator withdrew the whole service, and sending undos on
                // behalf of something that no longer exists is the mirror
                // of the outbox's own "delivering would resurrect intent an
                // operator withdrew".
                let dropped = {
                    let log = log.clone();
                    task::spawn_blocking(move || log.drop_all_for_undeployed()).await
                };
                self.saga_undeploy_candidates_lock().remove(&service_id);
                match dropped {
                    Ok(Ok(())) => {
                        info!(service_id = %service_id, "dropped sagas for an undeployed service");
                    }
                    Ok(Err(e)) => {
                        warn!(service_id = %service_id, error = %e, "could not drop sagas for an undeployed service");
                    }
                    Err(e) => {
                        warn!(service_id = %service_id, error = %e, "saga drop task failed");
                    }
                }
                continue;
            }

            // Deployed again (or still): clear any stale absence marker so
            // a later real undeploy needs its own two consecutive ticks
            // rather than firing on the first one because of a mark left
            // over from an earlier redeploy.
            self.saga_undeploy_candidates_lock().remove(&service_id);

            // The crash case: an open saga past its deadline starts
            // walking back. Nothing else can notice, because a guest does
            // not exist between calls.
            let abandoned = {
                let log = log.clone();
                task::spawn_blocking(move || log.abandoned(now, SAGA_SWEEP_LIMIT)).await
            };
            if let Ok(Ok(heads)) = abandoned {
                for head in heads {
                    let saga_id = head.saga_id.clone();
                    let started = {
                        let log = log.clone();
                        task::spawn_blocking(move || log.mark_compensating(&saga_id, now)).await
                    };
                    if matches!(started, Ok(Ok(true))) {
                        warn!(
                            saga = %head.saga_id,
                            service_id = %service_id,
                            "saga passed its deadline; compensating"
                        );
                    }
                }
            }

            let due = {
                let log = log.clone();
                task::spawn_blocking(move || log.due_compensations(now, SAGA_SWEEP_LIMIT)).await
            };
            if let Ok(Ok(heads)) = due {
                for head in heads {
                    settled += self.compensate_next_step(&service_id, &log, &head).await;
                }
            }
        }
        settled
    }

    /// One undo. Deliberately one per saga per tick: the walk is ordered,
    /// so a step that fails must not be overtaken by the step below it,
    /// and a saga with a slow provider must not hold the tick against
    /// every other saga.
    async fn compensate_next_step(
        &self,
        service_id: &str,
        log: &SagaLog,
        head: &SagaHead,
    ) -> usize {
        let now = proxy_outbox::now_ms();
        let saga_id = head.saga_id.clone();
        let step = {
            let log = log.clone();
            task::spawn_blocking(move || log.next_uncompensated_step(&saga_id)).await
        };
        let Ok(Ok(step)) = step else { return 0 };
        let Some(step) = step else {
            // Nothing left to compensate: done.
            let saga_id = head.saga_id.clone();
            let log = log.clone();
            let _ = task::spawn_blocking(move || log.finish_compensation(&saga_id, now)).await;
            metrics::counter!("substrate.proxy.saga.compensated").increment(1);
            return 1;
        };

        // Before dispatch, not after: a crash inside the call must cost an
        // attempt, or a poison step is retried forever. Safe only because
        // the idempotency key below lets the receiver answer a duplicate
        // from its own record.
        let idx = step.idx;
        let saga_id = head.saga_id.clone();
        let attempts = {
            let log = log.clone();
            task::spawn_blocking(move || log.begin_undo_attempt(&saga_id, idx, now)).await
        };
        let Ok(Ok(attempts)) = attempts else { return 0 };
        if attempts > u32::from(log.max_attempts()) {
            let saga_id = head.saga_id.clone();
            let log = log.clone();
            let _ = task::spawn_blocking(move || {
                log.fail_compensation(
                    &saga_id,
                    idx,
                    now,
                    "undo attempted repeatedly without ever completing",
                    true,
                )
            })
            .await;
            metrics::counter!("substrate.proxy.saga.failed").increment(1);
            return 1;
        }

        let Some(sagas) = &self.sagas else { return 0 };
        let target_value: QueuedTarget = match serde_json::from_str(&step.target) {
            Ok(t) => t,
            Err(e) => {
                self.fail_terminal_saga_step(
                    log,
                    &head.saga_id,
                    idx,
                    now,
                    &format!("stored saga step target is unreadable: {e}"),
                )
                .await;
                return 1;
            }
        };
        // A dependency name bound to nobody is not a failed delivery, it is
        // having nothing to deliver to -- terminal on its own terms, the
        // same split `resolve_queued_target` makes for the outbox.
        let target = match sagas.resolve_step_target(
            head.app_instance_id.as_deref(),
            &target_value,
            step.routing_key.as_deref(),
        ) {
            Ok(t) => t,
            Err(e) => {
                self.fail_terminal_saga_step(log, &head.saga_id, idx, now, &e.to_string()).await;
                return 1;
            }
        };

        let params_value: Value = serde_json::from_slice(&step.params).unwrap_or(Value::Null);
        let result_value: Option<Value> =
            step.result.as_deref().and_then(|bytes| serde_json::from_slice(bytes).ok());
        let params = merge_forward_result(&params_value, result_value.as_ref());

        let req = ProxyRequest {
            target_service: target,
            interface: step.interface.clone(),
            method: saga_undo_name(&step.method),
            params,
            caller: CallerContext::service_system(service_id),
            origin: CallOrigin::Guest { service_id: service_id.to_string() },
            protocol: ProxyProtocol::JsonRpcV1,
            idempotent: true,
            idempotency_key: Some(format!("saga:{}:{}", head.saga_id, idx)),
            timeout: Some(SAGA_UNDO_CALL_BUDGET),
        };

        // `invoke_inner`, not `invoke`: an undo has no live caller holding
        // the error, so writing a second, un-repliable proxy dead letter
        // for it would only add noise -- the saga's own `fail_compensation`
        // is this failure's operator surface.
        match self.invoke_inner(&req).await {
            Ok(_) => {
                metrics::counter!("substrate.proxy.saga.undo_delivered").increment(1);
                let saga_id = head.saga_id.clone();
                let log = log.clone();
                let _ = task::spawn_blocking(move || log.mark_step_compensated(&saga_id, idx, now))
                    .await;
            }
            Err(e) => match proxy_outbox::disposition_of(&e) {
                // The receiver already ran this undo and could not hand
                // back a result. That is a delivery, not a failure.
                Disposition::Delivered => {
                    let saga_id = head.saga_id.clone();
                    let log = log.clone();
                    let _ =
                        task::spawn_blocking(move || log.mark_step_compensated(&saga_id, idx, now))
                            .await;
                }
                Disposition::Retry => {
                    self.fail_saga_step(
                        log,
                        &head.saga_id,
                        idx,
                        now,
                        &explain_undo_error(&e, &step),
                    )
                    .await;
                }
                Disposition::Terminal => {
                    self.fail_terminal_saga_step(
                        log,
                        &head.saga_id,
                        idx,
                        now,
                        &explain_undo_error(&e, &step),
                    )
                    .await;
                }
            },
        }
        1
    }

    /// Records a retryable undo failure. `Disposition::Retry` does not mean
    /// the saga stays `compensating`: this attempt may have been the one
    /// that exhausted the budget, and that case must count toward
    /// `substrate.proxy.saga.failed` exactly as a terminal disposition does
    /// -- an operator watching that counter cannot see the difference.
    async fn fail_saga_step(&self, log: &SagaLog, saga_id: &str, idx: u32, now: i64, error: &str) {
        let log = log.clone();
        let saga_id = saga_id.to_string();
        let error = error.to_string();
        let outcome =
            task::spawn_blocking(move || log.fail_compensation(&saga_id, idx, now, &error, false))
                .await;
        if matches!(outcome, Ok(Ok(CompensationOutcome::Failed))) {
            metrics::counter!("substrate.proxy.saga.failed").increment(1);
        }
    }

    /// Records an undo failure that can never succeed. `terminal = true`
    /// always exhausts the saga regardless of attempts remaining, so this
    /// always counts toward `substrate.proxy.saga.failed`.
    async fn fail_terminal_saga_step(
        &self,
        log: &SagaLog,
        saga_id: &str,
        idx: u32,
        now: i64,
        error: &str,
    ) {
        let log = log.clone();
        let saga_id = saga_id.to_string();
        let error = error.to_string();
        let _ =
            task::spawn_blocking(move || log.fail_compensation(&saga_id, idx, now, &error, true))
                .await;
        metrics::counter!("substrate.proxy.saga.failed").increment(1);
    }
}

/// How many sagas one sweep tick may act on for one service -- mirrors
/// `proxy_outbox::CLAIM_LIMIT_PER_TICK`'s reasoning: one service with many
/// due sagas must not spend the whole node's tick budget on its own
/// backlog.
const SAGA_SWEEP_LIMIT: u32 = 16;

/// The step budget's margin rule, as a pure function so the subtraction
/// itself is unit-testable without a real clock: a step's own call budget
/// is what is
/// *left* of `step_timeout_ms` after `bookkeeping` (the log open plus the
/// intent write) was spent, never `step_timeout_ms` plus that time. A slow
/// cold open shortens the call the guest's epoch has room for; it does not
/// borrow against the epoch.
pub(super) fn step_call_budget_ms(step_timeout_ms: u64, bookkeeping: Duration) -> u64 {
    step_timeout_ms.saturating_sub(bookkeeping.as_millis() as u64).max(MIN_STEP_CALL_BUDGET_MS)
}

/// The three-case rule for merging a forward call's own result into its
/// undo's parameters: an object gains a `forward-result` member, an array
/// gains a trailing element, and `null`/absent (or any other scalar) is
/// wrapped into `{"forward-result": ...}`. A forward call that produced no
/// result (`result` is `None`) sends no `forward-result` at all, which
/// binds to `none` for an `option<string>` parameter -- so an undo written
/// as "ensure this is not in effect" works even for a step whose own
/// result was never recorded.
pub(super) fn merge_forward_result(params: &Value, result: Option<&Value>) -> Value {
    let Some(result) = result else { return params.clone() };
    match params {
        Value::Object(map) => {
            let mut map = map.clone();
            map.insert("forward-result".to_string(), result.clone());
            Value::Object(map)
        }
        Value::Array(items) => {
            let mut items = items.clone();
            items.push(result.clone());
            Value::Array(items)
        }
        _ => {
            let mut map = serde_json::Map::new();
            map.insert("forward-result".to_string(), result.clone());
            Value::Object(map)
        }
    }
}

/// What the deploy gate deliberately does not catch surfaces here
/// instead: a callee error carrying the shared "not found" wire code
/// (`SERVICE_NOT_FOUND_RPC_CODE`, reused for both "no such service" and "no
/// such method on a service that exists") is rewritten to name the
/// convention explicitly, so an operator reading `sagas`' `last-error`
/// learns what to fix rather than staring at a bare JSON-RPC code.
fn explain_undo_error(error: &ProxyError, step: &StepRow) -> String {
    let base = error.to_string();
    let looks_like_a_missing_export =
        matches!(error, ProxyError::Callee { code, .. } if *code == SERVICE_NOT_FOUND_RPC_CODE);
    if looks_like_a_missing_export {
        format!(
            "{base}; target does not export '{}' on '{}': a saga participant must export \
             saga-undo-<method> for every operation a step calls",
            saga_undo_name(&step.method),
            step.interface
        )
    } else {
        base
    }
}
