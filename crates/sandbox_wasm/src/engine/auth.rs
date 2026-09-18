use super::*;

impl AppSandboxEngine {
    /// Core in-memory subscribe logic shared by a live guest `subscribe()`
    /// call and substrate-startup replay (the latter has no `HostState` to
    /// call through, since it runs before any request is served). Spawns a
    /// forwarding task that calls `deliver_message` per broker message and
    /// exits when the broker's receiver closes (including when this
    /// engine itself is dropped, via `MqttBroker`'s `CancellationToken`).
    pub async fn register_internal_subscription(
        &self,
        service_id: &str,
        namespaced_topic: &str,
    ) -> Result<()> {
        let key = (service_id.to_string(), namespaced_topic.to_string());
        if self.subscriptions.contains_key(&key) {
            // Already live (e.g. a guest retrying `subscribe` after a
            // transient error it couldn't distinguish from "already
            // subscribed") -- opening a second broker link here would
            // double-deliver every message on this topic until the first
            // link's handle is eventually dropped.
            return Ok(());
        }

        let (handle, mut receiver) = self
            .messaging_broker
            .subscribe(key.1.clone())
            .await
            .map_err(|e| anyhow::anyhow!("broker subscribe failed: {e}"))?;

        let engine_weak = self.self_weak.get().cloned().unwrap_or_default();
        let service_id_owned = service_id.to_string();
        tokio::spawn(async move {
            while let Some((topic, payload)) = receiver.recv().await {
                let Some(engine) = engine_weak.upgrade() else { break };
                engine.deliver_message(&service_id_owned, &topic, payload).await;
            }
        });

        self.subscriptions.insert(key, handle);
        Ok(())
    }

    /// Drops every live guest-delivery subscription for `service_id`
    /// (called from `ControlPlaneService::undeploy`'s cleanup).
    pub fn unsubscribe_all(&self, service_id: &str) {
        self.subscriptions.retain(|(sid, _topic), _handle| sid != service_id);
    }

    /// Invokes the deployed component's exported `guest-api::handle-message`
    /// with a freshly-instantiated `Store` (same reasoning as any other
    /// invocation -- see `build_store_and_instantiate`), if it declares
    /// that export. If not, the message is silently discarded (per
    /// ADR-0010): this makes it safe to call for every subscription
    /// regardless of whether the target component implements messaging.
    ///
    /// Retries a bounded number of times on host-level transient failures --
    /// instantiation failing (e.g. the pooling allocator's engine-wide
    /// instance cap is momentarily saturated by concurrent short-lived calls,
    /// see `build_wasm_engine`) or the call itself trapping (e.g. an epoch
    /// deadline hit while the runtime was starved of CPU). Neither of these
    /// is a judgment about the message itself, so silently dropping the
    /// message on the first occurrence -- as this used to do -- turns an
    /// ordinary, momentary resource hiccup into permanent message loss with
    /// no redelivery. A missing export or a guest-returned application error
    /// is not retried: retrying can't change either outcome.
    async fn deliver_message(&self, service_id: &str, topic: &str, payload: Vec<u8>) {
        const GUEST_API_INTERFACE: &str = "syneroym:messaging/guest-api@0.1.0";
        const MAX_ATTEMPTS: u32 = 4;
        const RETRY_BACKOFF: Duration = Duration::from_millis(50);

        for attempt in 1..=MAX_ATTEMPTS {
            let last_attempt = attempt == MAX_ATTEMPTS;

            // `service_system`, never `local_elevated`: this is the inbound
            // broker-delivery hot path -- an accidentally elevated caller
            // here would let every delivered message pass the `execute-ddl`
            // Admin gate. The component receiving a message acts as itself.
            let (mut store, instance, _max_instructions) = match self
                .build_store_and_instantiate(
                    service_id,
                    CallerContext::service_system(service_id),
                    self.dispatch_epoch_ticks,
                    InstanceOptions::default(),
                )
                .await
            {
                Ok(triple) => triple,
                Err(e) if !last_attempt => {
                    debug!(
                        service_id,
                        attempt,
                        error = %e,
                        "messaging: failed to instantiate component for delivery, retrying"
                    );
                    time::sleep(RETRY_BACKOFF).await;
                    continue;
                }
                Err(e) => {
                    warn!(
                        service_id,
                        attempts = MAX_ATTEMPTS,
                        error = %e,
                        "messaging: failed to instantiate component for delivery, giving up"
                    );
                    return;
                }
            };

            let (func, results_len, _item) = match Self::get_wasm_func(
                &mut store,
                &instance,
                Some(GUEST_API_INTERFACE),
                "handle-message",
            ) {
                Ok(found) => found,
                Err(_) => {
                    debug!(
                        service_id,
                        "messaging: component does not export guest-api::handle-message, \
                         discarding"
                    );
                    return;
                }
            };

            let args = [
                Val::String(topic.to_string()),
                Val::List(payload.clone().into_iter().map(Val::U8).collect()),
            ];
            let mut results = vec![Val::Bool(false); results_len];
            match func.call_async(&mut store, &args, &mut results).await {
                Ok(()) => {
                    if let Some(msg) = Self::wasm_result_err(&results) {
                        warn!(service_id, error = %msg, "messaging: handle-message returned an error");
                    }
                    return;
                }
                Err(e) if !last_attempt => {
                    debug!(
                        service_id,
                        attempt,
                        error = %e,
                        "messaging: handle-message invocation trapped, retrying"
                    );
                    time::sleep(RETRY_BACKOFF).await;
                }
                Err(e) => {
                    warn!(
                        service_id,
                        attempts = MAX_ATTEMPTS,
                        error = %e,
                        "messaging: handle-message invocation trapped, giving up"
                    );
                    return;
                }
            }
        }
    }

    /// `msg` as the JSON shape `on-message`'s WIT `message` record expects
    /// -- kebab-case keys, matching the wasm component ABI's own field
    /// names (verified by `wasmtime::component::Val::Record`'s field
    /// names, which are the WIT identifiers verbatim, not Rust's
    /// snake_case).
    fn conversation_message_json(msg: &syneroym_rpc::ConversationMessage) -> serde_json::Value {
        serde_json::json!({
            "id": msg.id,
            "conversation": msg.conversation,
            "author": msg.author,
            "sender-timestamp": msg.sender_timestamp,
            "received-at": msg.received_at,
            "content-type": msg.content_type,
            "body": msg.body,
            "state": Self::conversation_state_str(msg.state),
            "verified": msg.verified,
            "last-error": msg.last_error,
        })
    }

    fn conversation_state_str(s: syneroym_rpc::ConversationDeliveryState) -> &'static str {
        match s {
            ConversationDeliveryState::Pending => "pending",
            ConversationDeliveryState::Delivered => "delivered",
            ConversationDeliveryState::Failed => "failed",
        }
    }

    /// Invokes the deployed component's optional
    /// `syneroym:conversation/guest-api::on-message` export, mirroring
    /// [`Self::deliver_message`]'s shape exactly (retry budget,
    /// `service_system` caller, silent discard on a missing export).
    pub(crate) async fn notify_guest_message(
        &self,
        service_id: &str,
        msg: syneroym_rpc::ConversationMessage,
    ) {
        const GUEST_API_INTERFACE: &str = "syneroym:conversation/guest-api@0.1.0";
        const MAX_ATTEMPTS: u32 = 4;
        const RETRY_BACKOFF: Duration = Duration::from_millis(50);
        let payload = Self::conversation_message_json(&msg);

        for attempt in 1..=MAX_ATTEMPTS {
            let last_attempt = attempt == MAX_ATTEMPTS;
            let (mut store, instance, _max_instructions) = match self
                .build_store_and_instantiate(
                    service_id,
                    CallerContext::service_system(service_id),
                    self.dispatch_epoch_ticks,
                    InstanceOptions::default(),
                )
                .await
            {
                Ok(triple) => triple,
                Err(e) if !last_attempt => {
                    debug!(service_id, attempt, error = %e, "conversation: failed to instantiate for on-message, retrying");
                    time::sleep(RETRY_BACKOFF).await;
                    continue;
                }
                Err(e) => {
                    warn!(service_id, attempts = MAX_ATTEMPTS, error = %e, "conversation: failed to instantiate for on-message, giving up");
                    return;
                }
            };

            let (func, results_len, item) = match Self::get_wasm_func(
                &mut store,
                &instance,
                Some(GUEST_API_INTERFACE),
                "on-message",
            ) {
                Ok(found) => found,
                Err(_) => {
                    debug!(
                        service_id,
                        "conversation: component does not export guest-api::on-message, discarding"
                    );
                    return;
                }
            };
            let params_iter = match &item {
                ComponentItem::ComponentFunc(f) => f.params(),
                _ => return,
            };
            let wasm_params = match conversions::json_to_wasm_params(
                params_iter,
                &Value::Array(vec![payload.clone()]),
            ) {
                Ok(p) => p,
                Err(e) => {
                    warn!(service_id, error = %e, "conversation: could not encode on-message params");
                    return;
                }
            };
            let mut results = vec![Val::Bool(false); results_len];
            match func.call_async(&mut store, &wasm_params, &mut results).await {
                Ok(()) => {
                    if let Some(msg) = Self::wasm_result_err(&results) {
                        warn!(service_id, error = %msg, "conversation: on-message returned an error");
                    }
                    return;
                }
                Err(e) if !last_attempt => {
                    debug!(service_id, attempt, error = %e, "conversation: on-message invocation trapped, retrying");
                    time::sleep(RETRY_BACKOFF).await;
                }
                Err(e) => {
                    warn!(service_id, attempts = MAX_ATTEMPTS, error = %e, "conversation: on-message invocation trapped, giving up");
                    return;
                }
            }
        }
    }

    /// Invokes the deployed component's optional
    /// `syneroym:conversation/guest-api::on-delivery-state` export --
    /// mirrors [`Self::notify_guest_message`] exactly.
    pub(crate) async fn notify_guest_state(
        &self,
        service_id: &str,
        message_id: String,
        state: syneroym_rpc::ConversationDeliveryState,
    ) {
        const GUEST_API_INTERFACE: &str = "syneroym:conversation/guest-api@0.1.0";
        const MAX_ATTEMPTS: u32 = 4;
        const RETRY_BACKOFF: Duration = Duration::from_millis(50);
        let params_json = Value::Array(vec![
            Value::String(message_id.clone()),
            Value::String(Self::conversation_state_str(state).to_string()),
        ]);

        for attempt in 1..=MAX_ATTEMPTS {
            let last_attempt = attempt == MAX_ATTEMPTS;
            let (mut store, instance, _max_instructions) = match self
                .build_store_and_instantiate(
                    service_id,
                    CallerContext::service_system(service_id),
                    self.dispatch_epoch_ticks,
                    InstanceOptions::default(),
                )
                .await
            {
                Ok(triple) => triple,
                Err(e) if !last_attempt => {
                    debug!(service_id, attempt, error = %e, "conversation: failed to instantiate for on-delivery-state, retrying");
                    time::sleep(RETRY_BACKOFF).await;
                    continue;
                }
                Err(e) => {
                    warn!(service_id, attempts = MAX_ATTEMPTS, error = %e, "conversation: failed to instantiate for on-delivery-state, giving up");
                    return;
                }
            };

            let (func, results_len, item) = match Self::get_wasm_func(
                &mut store,
                &instance,
                Some(GUEST_API_INTERFACE),
                "on-delivery-state",
            ) {
                Ok(found) => found,
                Err(_) => {
                    debug!(
                        service_id,
                        "conversation: component does not export guest-api::on-delivery-state, \
                         discarding"
                    );
                    return;
                }
            };
            let params_iter = match &item {
                ComponentItem::ComponentFunc(f) => f.params(),
                _ => return,
            };
            let wasm_params = match conversions::json_to_wasm_params(params_iter, &params_json) {
                Ok(p) => p,
                Err(e) => {
                    warn!(service_id, error = %e, "conversation: could not encode on-delivery-state params");
                    return;
                }
            };
            let mut results = vec![Val::Bool(false); results_len];
            match func.call_async(&mut store, &wasm_params, &mut results).await {
                Ok(()) => {
                    if let Some(msg) = Self::wasm_result_err(&results) {
                        warn!(service_id, error = %msg, "conversation: on-delivery-state returned an error");
                    }
                    return;
                }
                Err(e) if !last_attempt => {
                    debug!(service_id, attempt, error = %e, "conversation: on-delivery-state invocation trapped, retrying");
                    time::sleep(RETRY_BACKOFF).await;
                }
                Err(e) => {
                    warn!(service_id, attempts = MAX_ATTEMPTS, error = %e, "conversation: on-delivery-state invocation trapped, giving up");
                    return;
                }
            }
        }
    }
}

/// Ceiling on how much of a guest-controlled string (a returned `err`
/// payload, or the debug rendering of an unrecognized `Val`) is kept once it
/// becomes an `AbacError` detail. Every `AbacError`
/// eventually reaches `AbacTrace::emit`'s `info!` line unbounded, so without
/// this a guest returning a multi-megabyte error string -- or a decision
/// list carrying row-derived data in a malformed shape -- writes it to the
/// log in full on every read that hits it.
const ABAC_ERROR_DETAIL_MAX_LEN: usize = 500;

pub(crate) fn truncate_detail(s: String) -> String {
    if s.len() <= ABAC_ERROR_DETAIL_MAX_LEN {
        return s;
    }
    let mut cut = ABAC_ERROR_DETAIL_MAX_LEN;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}... ({} bytes total, truncated)", &s[..cut], s.len())
}

#[async_trait::async_trait]
impl RowAuthorizer for AppSandboxEngine {
    /// Invokes `service_id`'s guest-exported `authorize-rows` (ADR-0017 §7)
    /// in a freshly instantiated, throw-away instance of the same
    /// component -- the same "instantiate, call, discard" shape
    /// `deliver_message`/`invoke_lifecycle_hook` already use, since a host
    /// function (this trait's only caller) cannot re-enter the live
    /// instance it's already running inside. No retry loop, unlike
    /// `deliver_message`: a retried after-step would double the worst-case
    /// latency of a hot-path read, and every failure mode below is already
    /// deny-closed.
    ///
    /// Records `substrate.fdae.abac_ms` on *every* exit path, labelled by
    /// outcome: the un-refactored version only
    /// recorded it after a successful `func.call_async`, so instantiation
    /// failure (the pool-exhaustion symptom) and a missing export both
    /// skipped it entirely, undercounting exactly the two failure modes an
    /// operator most needs visibility into.
    async fn authorize_rows(
        &self,
        service_id: &str,
        ctx: &AbacAuthContext,
        rows: &[CandidateRow],
    ) -> Result<Vec<RowDecision>, AbacError> {
        let exec_start = Instant::now();
        let result = self.authorize_rows_inner(service_id, ctx, rows).await;
        let outcome = if result.is_ok() { "ok" } else { "error" };
        metrics::histogram!("substrate.fdae.abac_ms", "outcome" => outcome)
            .record(exec_start.elapsed().as_secs_f64() * 1000.0);
        result
    }
}

#[async_trait::async_trait]
impl syneroym_rpc::ConversationNotifier for AppSandboxEngine {
    async fn notify_message(&self, service_id: &str, msg: syneroym_rpc::ConversationMessage) {
        self.notify_guest_message(service_id, msg).await;
    }

    async fn notify_delivery_state(
        &self,
        service_id: &str,
        message_id: String,
        state: syneroym_rpc::ConversationDeliveryState,
    ) {
        self.notify_guest_state(service_id, message_id, state).await;
    }
}

impl AppSandboxEngine {
    async fn authorize_rows_inner(
        &self,
        service_id: &str,
        ctx: &AbacAuthContext,
        rows: &[CandidateRow],
    ) -> Result<Vec<RowDecision>, AbacError> {
        // Bounds concurrent after-step instantiation -- see
        // `abac_instance_permits`'s doc comment. Acquired
        // before instantiating, inside `apply_stage4`'s own
        // `FDAE_ABAC_TIMEOUT` wrapper, so a long queue wait surfaces as the
        // same `AbacError::BudgetExceeded` a fuel/epoch overrun would,
        // rather than hanging indefinitely or racing wasmtime's pool
        // directly.
        let _permit = self
            .abac_instance_permits
            .acquire()
            .await
            .map_err(|_| AbacError::Unavailable(service_id.to_string()))?;

        let (mut store, instance, _max_instructions) = self
            .build_store_and_instantiate(
                service_id,
                CallerContext::service_abac(service_id),
                self.abac_epoch_ticks,
                InstanceOptions {
                    fuel_override: Some(self.abac_max_instructions),
                    read_only: true,
                    ..InstanceOptions::default()
                },
            )
            .await
            .map_err(|_| AbacError::Unavailable(service_id.to_string()))?;

        let (func, results_len, _item) = Self::get_wasm_func(
            &mut store,
            &instance,
            Some(Self::AUTHORIZER_INTERFACE),
            "authorize-rows",
        )
        .map_err(|_| AbacError::MissingExport(service_id.to_string()))?;

        let ctx_val = Self::abac_ctx_to_val(ctx);
        let rows_val = Self::candidate_rows_to_val(rows);

        let mut results = vec![Val::Bool(false); results_len];
        let call_result = func.call_async(&mut store, &[ctx_val, rows_val], &mut results).await;
        Self::map_abac_call_error(service_id, call_result)?;

        let decisions = Self::decode_row_decisions(service_id, &results)?;
        let denied = decisions.iter().filter(|d| matches!(d, RowDecision::Deny)).count() as u64;
        if denied > 0 {
            metrics::counter!("substrate.fdae.abac_rows_denied").increment(denied);
        }
        Ok(decisions)
    }

    /// `AbacAuthContext` as the WIT `abac-auth-context` record `Val` --
    /// field names are the WIT identifiers verbatim (kebab-case), matching
    /// the wasm component ABI, not Rust's `snake_case`.
    fn abac_ctx_to_val(ctx: &AbacAuthContext) -> Val {
        Val::Record(vec![
            ("collection".to_string(), Val::String(ctx.collection.clone())),
            (
                "permissions".to_string(),
                Val::List(ctx.permissions.iter().cloned().map(Val::String).collect()),
            ),
            ("subject-did".to_string(), Val::String(ctx.subject_did.clone())),
            (
                "anchor-did".to_string(),
                Val::Option(ctx.anchor_did.clone().map(|d| Box::new(Val::String(d)))),
            ),
            (
                "capabilities".to_string(),
                Val::List(ctx.capabilities.iter().cloned().map(Val::String).collect()),
            ),
            ("claims-json".to_string(), Val::String(ctx.claims_json.clone())),
        ])
    }

    /// `rows` as the WIT `list<candidate-row>` `Val` -- same field-naming
    /// note as [`Self::abac_ctx_to_val`].
    fn candidate_rows_to_val(rows: &[CandidateRow]) -> Val {
        Val::List(
            rows.iter()
                .map(|r| {
                    Val::Record(vec![
                        ("id".to_string(), Val::String(r.id.clone())),
                        (
                            "payload".to_string(),
                            Val::List(r.payload.iter().copied().map(Val::U8).collect()),
                        ),
                        ("creator-id".to_string(), Val::String(r.creator_id.clone())),
                        ("created-at".to_string(), Val::U64(r.created_at)),
                        ("updated-at".to_string(), Val::U64(r.updated_at)),
                    ])
                })
                .collect(),
        )
    }

    /// Maps a failed `authorize-rows` call into the matching `AbacError`.
    /// `classify_call_failure` replaces this site's own hand-rolled copy of
    /// the trap taxonomy: this site has (and had) no memory-fault arm, so a
    /// memory fault still becomes `Trap`, not a budget error. The
    /// classifier does not distinguish a downcast `Trap::OutOfFuel` from a
    /// string-matched fuel error, so both now carry the real Wasmtime
    /// message (`err_str`) instead of the pre-refactor downcast arm's fixed
    /// `"exceeded its fuel budget"` -- a deliberate simplification, not a
    /// behaviour this site's callers depend on.
    fn map_abac_call_error(
        service_id: &str,
        call_result: wasmtime::Result<()>,
    ) -> Result<(), AbacError> {
        let Err(e) = call_result else { return Ok(()) };
        let service = service_id.to_string();
        let err_str = truncate_detail(e.root_cause().to_string());
        Err(match classify_call_failure(&e) {
            CallFailure::OutOfFuel | CallFailure::Deadline => {
                AbacError::BudgetExceeded { service, detail: err_str }
            }
            CallFailure::MemoryFault | CallFailure::Other => {
                AbacError::Trap { service, detail: err_str }
            }
        })
    }

    /// Turns the wasm call's raw `results` (`result<list<row-decision>,
    /// string>`) into row decisions, or the matching `AbacError` for any
    /// shape wasmtime's dynamic `Val` does not already guarantee.
    fn decode_row_decisions(
        service_id: &str,
        results: &[Val],
    ) -> Result<Vec<RowDecision>, AbacError> {
        let [result_val] = results else {
            return Err(AbacError::Malformed(format!(
                "expected exactly 1 result<_, string> return value, got {}",
                results.len()
            )));
        };
        let decisions_val = match result_val {
            Val::Result(Ok(Some(boxed))) => boxed.as_ref(),
            Val::Result(Err(payload)) => {
                // Guest-controlled (an explicit `Err(string)`, or the debug
                // rendering of an unrecognized `Val`) -- truncated before it
                // becomes an `AbacError` so it can't carry an unbounded or
                // row-derived string into `AbacTrace`'s `info!` line.
                let msg = match payload.as_deref() {
                    Some(Val::String(s)) => truncate_detail(s.clone()),
                    Some(other) => truncate_detail(format!("{other:?}")),
                    None => "guest declined the request".to_string(),
                };
                return Err(AbacError::Trap { service: service_id.to_string(), detail: msg });
            }
            other => {
                return Err(AbacError::Malformed(truncate_detail(format!(
                    "expected result<list<row-decision>, string>, got {other:?}"
                ))));
            }
        };
        let Val::List(items) = decisions_val else {
            return Err(AbacError::Malformed(truncate_detail(format!(
                "expected list<row-decision>, got {decisions_val:?}"
            ))));
        };

        let mut decisions = Vec::with_capacity(items.len());
        for item in items {
            let decision = match item {
                Val::Variant(tag, None) if tag == "allow" => RowDecision::Allow,
                Val::Variant(tag, None) if tag == "deny" => RowDecision::Deny,
                Val::Variant(tag, Some(boxed)) if tag == "redact" => {
                    let Val::List(fields) = boxed.as_ref() else {
                        return Err(AbacError::Malformed(truncate_detail(format!(
                            "redact payload must be list<string>, got {boxed:?}"
                        ))));
                    };
                    let mut names = Vec::with_capacity(fields.len());
                    for field in fields {
                        let Val::String(s) = field else {
                            return Err(AbacError::Malformed(truncate_detail(format!(
                                "redact field must be string, got {field:?}"
                            ))));
                        };
                        names.push(s.clone());
                    }
                    RowDecision::Redact(names)
                }
                other => {
                    return Err(AbacError::Malformed(truncate_detail(format!(
                        "unrecognized row-decision: {other:?}"
                    ))));
                }
            };
            decisions.push(decision);
        }
        Ok(decisions)
    }
}
