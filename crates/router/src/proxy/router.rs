use super::*;

impl ProxyRouter {
    /// operator may choose to repeat, and the resolution already happened
    /// before the request existed.
    pub(super) async fn record_failed_call(&self, req: &ProxyRequest, error: &ProxyError) {
        let (Some(outbox), Some(key)) = (&self.outbox, req.idempotency_key.as_deref()) else {
            return;
        };
        let CallOrigin::Guest { service_id } = &req.origin else { return };

        // Only a failure the *target* produced earns a row. The fence's
        // own answers are not delivery failures at all:
        //
        // - "already running here" means the call is succeeding on another task right
        //   now, so a dead letter for it would be indistinguishable from a genuine
        //   exhausted delivery and would never be cleared when the real call finished;
        // - "already ran, result too large to retain" is a delivery;
        // - a fail-closed refusal (no store, anonymous caller, node-level target) means
        //   nothing was attempted, so there is nothing to replay -- and replaying would
        //   hit the same refusal.
        if !target_produced(error) {
            return;
        }

        // A queued item that failed is dead-lettered by the worker, which
        // owns its own retry history; this path is only for the caller
        // that is still holding the error.
        let call = QueuedCall {
            app_instance_id: None,
            caller_service_id: service_id.clone(),
            target: QueuedTarget::Service(req.target_service.clone()),
            routing_key: None,
            interface: req.interface.clone(),
            method: req.method.clone(),
            params: req.params.clone(),
            idempotency_key: key.to_string(),
            protocol: None,
            timeout_ms: req.timeout.map(|t| t.as_millis() as u64),
        };
        if let Err(e) = outbox.record_dead_letter(&call, &error.to_string()).await {
            warn!(error = %e, "could not record a dead letter for a failed keyed call");
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
    /// is every call on the hot path today, and it is what keeps the
    /// fence off the existing call budget.
    pub(super) async fn invoke_local_guarded(
        &self,
        req: &ProxyRequest,
        endpoint: SubstrateEndpoint,
        canonical_iface: String,
    ) -> Result<Value, ProxyError> {
        let Some(guard) = &self.dedup_guard else {
            if req.idempotency_key.is_some() {
                return Err(ProxyError::Internal(
                    "this node keeps no per-service storage, so it cannot honour an idempotency \
                     key"
                    .to_string(),
                ));
            }
            return self.invoke_local(req, endpoint, canonical_iface).await;
        };
        // Keyed on the *resolved* endpoint's service id, not the id the
        // caller addressed. The wire entry point keys on the resolved one
        // too, and for a native channel registered under a different id
        // than it is addressed by the two disagree -- which would make
        // "one guard, both entry points" one guard reading two different
        // keys.
        let store_owner = match &endpoint {
            SubstrateEndpoint::NativeHostChannel { service_id }
            | SubstrateEndpoint::WasmChannel { service_id } => service_id.clone(),
            _ => req.target_service.clone(),
        };
        match guard
            .begin(
                &store_owner,
                &req.interface,
                Some(&req.caller.caller_did),
                req.idempotency_key.as_deref(),
            )
            .await
        {
            GuardOutcome::Refuse(e) => Err(e),
            GuardOutcome::Answer(outcome) => call_dedup::replay_as_result(outcome),
            GuardOutcome::Execute(claim) => {
                let outcome = self.invoke_local(req, endpoint, canonical_iface).await;
                if let Some(claim) = claim {
                    claim.settle(&outcome).await;
                }
                outcome
            }
        }
    }

    /// `data-layer`, `vault`, `app-config`, `blob-store`, `messaging`,
    /// `http-native` -- the reserved names every deployed service
    /// auto-registers (`syneroym_core::local_registry::
    /// NATIVE_CAPABILITY_INTERFACES`).
    ///
    /// TODO(FDAE): this is an interim, coarse, fail-closed gate -- "a
    /// guest may only reach its **own** service's native capabilities
    /// through the proxy." A full FDAE policy replaces it with real
    /// per-caller/per-row policy evaluated against `caller.session` at the
    /// data-owning node, at which point a guest-originated cross-service
    /// `data-layer` read becomes expressible (and filtered), not refused
    /// outright. Do not widen this gate before that policy exists.
    ///
    /// Applies to `CallOrigin::Guest` only. A substrate-internal
    /// (`CallOrigin::Native`) call to another service's `data-layer` is
    /// exactly what the relationship-proof fetch is, so gating
    /// it here would foreclose the scenario the proxy is explicitly
    /// supposed to co-design for. Native-origin calls are authorized at the
    /// **data-owning node** -- the destination re-verifies the forwarded
    /// proof (`invoke_remote`) and, once the FDAE policy lands, runs it
    /// inside the callee's own `data-layer` dispatch.
    pub(super) fn check_native_capability_gate(
        &self,
        req: &ProxyRequest,
    ) -> Result<(), ProxyError> {
        let CallOrigin::Guest { service_id } = &req.origin else { return Ok(()) };

        // An empty interface is a convenience for a caller that
        // cannot know a remote service's interface names -- a gateway or
        // coordinator resolving an external hostname. A WASM guest is
        // never in that position: it always names the interface it wants.
        // Refused here, before `registry.lookup` gets a chance to resolve
        // it to "the one app-declared interface" of whatever
        // `target_service` names -- `matches_interface`
        // below can never match `""` against a real interface name, so
        // nothing past this point would otherwise have stopped it.
        if req.interface.is_empty() {
            return Err(ProxyError::PermissionDenied(format!(
                "component '{service_id}' must name an interface; the proxy does not resolve an \
                 empty interface for a guest call"
            )));
        }

        // `req.interface` may be the literal name or `EndpointRegistry`'s
        // short-hash of it (`local_registry::short_hash` is an unsalted
        // SHA-256 prefix -- guest-computable, and `lookup` canonicalizes it
        // right back to the literal name for dispatch). Matching only the
        // literal string here let a guest bypass this gate entirely by
        // passing the hash instead of the name.
        let matches_interface =
            |name: &&str| *name == req.interface || util::short_hash(name) == req.interface;

        // `orchestrator`/`security` are node-level, not service-scoped, so
        // the same-service exemption below (which only makes sense for an
        // interface a service can itself hold) does not apply -- denied
        // outright, for any target. Since ADR-0020 §1, a guest whose own
        // service holds an installed instance certificate presents a
        // *verified* identity on outbound calls (see `invoke_remote_at`);
        // without this, that verified identity would reach these two
        // node-owned interfaces exactly like a legitimate native caller --
        // a WASM guest was never able to present a verified identity to a
        // native interface at all before that certificate mechanism
        // existed. Both interfaces are gated now
        // (`orchestrator` since the deploy-grant admission gate, `security` on
        // `substrate/admin`, `control_plane::service`), but this
        // outright denial stays: a deployed service's own instance identity
        // should never reach node-owner-only interfaces at all, gated or
        // not.
        if NODE_NATIVE_INTERFACES.iter().any(matches_interface) {
            return Err(ProxyError::PermissionDenied(format!(
                "component '{service_id}' may not reach node-level interface '{}' through the \
                 proxy",
                req.interface
            )));
        }

        let is_native_capability = NATIVE_CAPABILITY_INTERFACES.iter().any(matches_interface);
        if !is_native_capability {
            return Ok(());
        }

        // Compare the guest's **raw** component_id against the target. NOT
        // `caller.caller_did`: `CallerContext::service_system` sets that to
        // `"system:<service_id>"`, which can never equal a plain service
        // id -- using it would reject a component's calls to its own
        // service too.
        if service_id == &req.target_service {
            return Ok(());
        }

        Err(ProxyError::PermissionDenied(format!(
            "component '{service_id}' may not reach native capability '{}' on service '{}' \
             through the proxy (cross-service native-capability policy is FDAE/M04B)",
            req.interface, req.target_service
        )))
    }

    /// Local-node dispatch: the endpoint registry is authoritative for
    /// services hosted on this node -- also the `<5ms p99` same-node path
    /// (in-process dispatch, no wire round trip).
    pub(super) async fn invoke_local(
        &self,
        req: &ProxyRequest,
        endpoint: SubstrateEndpoint,
        canonical_iface: String,
    ) -> Result<Value, ProxyError> {
        let call_timeout = req.timeout.unwrap_or(DEFAULT_PROXY_CALL_TIMEOUT);
        match endpoint {
            SubstrateEndpoint::NativeHostChannel { service_id } => {
                let table = self.native_dispatch.upgrade().ok_or_else(|| {
                    ProxyError::Internal("native dispatch registry gone".to_string())
                })?;
                let svc = table
                    .get(&service_id)
                    .as_deref()
                    .cloned()
                    .ok_or_else(|| ProxyError::ServiceNotFound(service_id.clone()))?;
                let invocation = NativeInvocation {
                    interface: canonical_iface,
                    method: req.method.clone(),
                    params: req.params.clone(),
                    caller: req.caller.clone(),
                };
                time::timeout(call_timeout, svc.dispatch(invocation))
                    .await
                    .map_err(|_| ProxyError::Timeout(call_timeout))?
                    .map(|r| r.payload)
                    .map_err(|e: RpcError| ProxyError::Callee {
                        code: e.code(),
                        message: e.to_string(),
                        data: e.data(),
                    })
            }
            // Identity threading through a proxied WASM call is "the callee
            // acts as itself": `caller: None` below always synthesizes
            // `service_system` (see `execute_wasm_json`'s own doc comment).
            // This is a *different* question from the two guest-originated-
            // read ingresses that thread a router-verified caller's own
            // identity into its own guest's reads (direct or via
            // self-proxy) -- this is one guest delegating to a *different*
            // service's guest-exported interface through the proxy, which
            // would need real caller-delegation (UCAN, not yet built) to
            // forward safely. Not an oversight.
            //
            // Known limitation, same boundary: any error from
            // `execute_wasm_json` -- including a callee's own typed
            // `result::err` -- collapses to `Callee{ code: -32603 }` below.
            // The structured `E` doesn't survive the WIT<->JSON boundary
            // here, so a caller can't distinguish a business rejection from
            // a host crash. Acceptable for now; a component-to-component
            // error channel that can carry typed errors is a follow-up.
            SubstrateEndpoint::WasmChannel { service_id } => {
                let engine = self.app_sandbox_engine.upgrade().ok_or_else(|| {
                    ProxyError::Internal("sandbox engine unavailable".to_string())
                })?;
                let request = JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    method: req.method.clone(),
                    params: req.params.clone(),
                    id: Some(Value::from(1)),
                    idempotency_key: req.idempotency_key.clone(),
                };
                time::timeout(
                    call_timeout,
                    engine.execute_wasm_json(&service_id, &canonical_iface, &request, None),
                )
                .await
                .map_err(|_| ProxyError::Timeout(call_timeout))?
                .map_err(|e| ProxyError::Callee {
                    code: -32603,
                    message: e.to_string(),
                    data: None,
                })
            }
            other @ (SubstrateEndpoint::TcpHostPort { .. }
            | SubstrateEndpoint::PodmanSocket { .. }) => {
                Err(ProxyError::UnsupportedTarget(format!("{other:?}")))
            }
        }
    }

    /// Resolves `req.target_service`'s Iroh address via the community
    /// registry / DHT and dispatches the call over [`RemoteHop`].
    pub(super) async fn invoke_remote(&self, req: &ProxyRequest) -> Result<Value, ProxyError> {
        let addr = net_iroh::resolve_iroh_addr(&self.registry_client, &req.target_service)
            .await
            .map_err(|_| ProxyError::ServiceNotFound(req.target_service.clone()))?;
        self.invoke_remote_at(&addr, req).await
    }

    /// The retry loop and preamble construction, split out from
    /// [`Self::invoke_remote`] so unit tests can drive it against a
    /// pre-resolved (synthetic) address without a live registry/DHT.
    pub(super) async fn invoke_remote_at(
        &self,
        addr: &EndpointAddr,
        req: &ProxyRequest,
    ) -> Result<Value, ProxyError> {
        // Identity: forward the caller's *signed proof* verbatim when it has
        // one and the call is genuinely substrate-internal (ADR-0016 §6 --
        // the destination re-verifies with `verify_preamble` and builds a
        // fresh `CallerContext`); otherwise present this node's own identity
        // -- again only for `CallOrigin::Native`. A guest is never allowed to
        // present the *caller's* proof or the *node's* key remotely, even one
        // it legitimately carries today (the self-proxy path forwards a
        // self-proxy caller's real `CallerContext`, proof included, whenever
        // the target is the guest's own service -- `check_native_capability_
        // gate`'s same-service check only restricts *native-capability*
        // interfaces, so an ordinary interface the local registry doesn't
        // happen to have registered still falls through to this remote path
        // with that proof attached). Presenting either of those on a guest's
        // behalf would let the guest steer its own freely-chosen `(interface,
        // method, params)` onto the wire under a real, potentially
        // privileged identity -- exactly the laundering this function's
        // `CallOrigin::Guest` branch exists to prevent.
        //
        // What a guest *may* present is its own service's certified instance
        // key (ADR-0020 §1): that grants no privilege the guest didn't
        // already have as itself, since the guest still chooses the call --
        // only the identity it travels under changes, from anonymous to the
        // member master this substrate derived and was certified for. `None`
        // for either the certificate or the recorded owner (a service
        // deployed before an owner/certificate existed) falls back to
        // presenting nothing, unchanged from before: the destination treats
        // it as anonymous, which the native-dispatch arm already rejects and
        // non-native paths already tolerate. Capabilities never cross either
        // way.
        let mut preamble = RoutePreamble::binary_json_rpc(&req.target_service, &req.interface);
        match (&req.caller.proof, &req.origin) {
            (Some(proof), CallOrigin::Native { .. }) => {
                preamble.pubkey = Some(proof.pubkey_hex.clone());
                preamble.delegation = proof
                    .delegation_json
                    .as_deref()
                    .and_then(|json| DelegationCertificate::from_json(json).ok());
            }
            // Built for the guest-origin arm; the same reasoning
            // applies to a substrate-internal call made on a service's
            // behalf. Only the no-proof case: the arm above forwards the
            // original caller's chain verbatim, which is what lets the
            // destination re-derive `subject_did`/`anchor_did` and authorize
            // the real caller. Presenting the service's identity
            // here instead would silently change who the destination thinks
            // is asking.
            (None, CallOrigin::Native { service_id: Some(sid) }) => {
                if let Some(cert) = self.registry.instance_cert(sid)
                    && !cert.is_expired()
                    && let Some(owner) = self.registry.owner_of(sid)
                {
                    let instance = self.node_identity.derive_service_identity(&owner, sid);
                    preamble.pubkey = Some(hex::encode(instance.public_key().to_bytes()));
                    preamble.delegation = Some(cert);
                } else {
                    preamble.pubkey = Some(hex::encode(self.node_identity.public_key().to_bytes()));
                }
            }
            (None, CallOrigin::Native { service_id: None }) => {
                preamble.pubkey = Some(hex::encode(self.node_identity.public_key().to_bytes()));
            }
            (_, CallOrigin::Guest { service_id }) => {
                // An expired certificate is worse than none: the
                // destination hard-rejects any connection whose delegation
                // fails to verify (`route_handler/io.rs`), where a `None`
                // pubkey instead falls back to anonymous -- which non-
                // native-dispatch destinations already tolerate. Presenting
                // it anyway would turn a missed renewal into an outage for
                // passthrough/relay calls that never cared about identity
                // before this certificate mechanism existed.
                if let Some(cert) = self.registry.instance_cert(service_id)
                    && !cert.is_expired()
                    && let Some(owner) = self.registry.owner_of(service_id)
                {
                    let instance = self.node_identity.derive_service_identity(&owner, service_id);
                    preamble.pubkey = Some(hex::encode(instance.public_key().to_bytes()));
                    preamble.delegation = Some(cert);
                }
            }
        }

        let json_rpc_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: req.method.clone(),
            params: req.params.clone(),
            id: Some(Value::from(1)),
            idempotency_key: req.idempotency_key.clone(),
        };
        let call_timeout = req.timeout.unwrap_or(DEFAULT_PROXY_CALL_TIMEOUT);

        // Retry loop. Only *transport* failures are retried; a
        // callee-returned error is a definitive answer and is never
        // retried. Retry-eligible means the caller declared the call
        // idempotent, or supplied an idempotency key -- a key is a
        // strictly stronger fence than the caller's own assertion, since
        // the receiver enforces it.
        //
        // Exhausting the budget always fails the caller directly. It
        // additionally writes a dead letter *only* when the call carried a
        // key (`record_failed_call`): a dead letter exists to be replayed,
        // and replaying a call with no fence would be a second delivery of
        // something nothing can deduplicate.
        let retry_eligible = req.idempotent || req.idempotency_key.is_some();
        let attempts: u8 = if retry_eligible { self.retry_policy.max_attempts.max(1) } else { 1 };
        let mut backoff = self.retry_policy.initial_backoff_ms;
        let mut attempt: u8 = 1;
        loop {
            match self.hop.call(addr, &preamble, &json_rpc_request, call_timeout).await {
                Ok(v) => return Ok(v),
                Err(e)
                    if attempt >= attempts
                        || !matches!(e, ProxyError::Transport(_) | ProxyError::Timeout(_)) =>
                {
                    return Err(e);
                }
                Err(e) => {
                    warn!(attempt, max = attempts, error = %e, "proxy call failed; retrying");
                    metrics::counter!("substrate.proxy.retries").increment(1);
                    time::sleep(Duration::from_millis(retry::calculate_jittered_backoff(backoff)))
                        .await;
                    backoff = ((backoff as f64 * self.retry_policy.backoff_multiplier) as u64)
                        .min(self.retry_policy.max_backoff_ms);
                    attempt += 1;
                }
            }
        }
    }

    /// The dispatch itself, with no dead-letter side effect.
    ///
    /// Split from the trait method so the two callers that own their own
    /// failure record -- the outbox worker, which dead-letters through the
    /// queue's retry history, and `enqueue`'s immediate attempt, which
    /// either queues the item or hands the error back -- cannot also
    /// produce a second row through `record_failed_call`.
    pub(super) async fn invoke_inner(&self, req: &ProxyRequest) -> Result<Value, ProxyError> {
        // Protocol gate: the minimal `[LFC-VER]` behavior kept from the
        // deferred protocol-negotiation slice (A.7). `ProxyProtocol` has
        // exactly one variant today, so this is a no-op in practice; it
        // stays as the seam a future wRPC variant plugs into.
        if req.protocol != ProxyProtocol::JsonRpcV1 {
            return Err(ProxyError::UnsupportedProtocol(format!("{:?}", req.protocol)));
        }

        // Capability gate: a WASM guest must not reach another service's
        // native capabilities through the proxy.
        self.check_native_capability_gate(req)?;

        metrics::counter!("substrate.proxy.calls").increment(1);
        let started = Instant::now();

        // Local first: the endpoint registry is authoritative for services
        // hosted on this node (this is also the <5ms same-node path).
        let outcome = match self.registry.lookup(&req.target_service, &req.interface) {
            Some((endpoint, canonical_iface)) => {
                self.invoke_local_guarded(req, endpoint, canonical_iface).await
            }
            None => self.invoke_remote(req).await,
        };

        metrics::histogram!("substrate.proxy.duration_ms")
            .record(started.elapsed().as_secs_f64() * 1000.0);
        if outcome.is_err() {
            metrics::counter!("substrate.proxy.errors").increment(1);
        }
        outcome
    }

    /// Try-then-queue. A reachable target costs one call and zero queue
    /// writes; only a retryable failure puts the item on disk.
    ///
    /// The three refusals all happen before anything is written, and all
    /// for the same reason: each names a condition every later delivery
    /// attempt would hit too, so failing now is the same answer given
    /// hours sooner, to a caller that is still alive to read it.
    pub(super) async fn enqueue_call(&self, call: QueuedCall) -> Result<(), ProxyError> {
        let Some(outbox) = &self.outbox else {
            return Err(ProxyError::Internal(
                "this node keeps no per-service storage, so it has no durable outbox".to_string(),
            ));
        };

        // A node-level interface keeps no record to fence a key with, so
        // it could never honour the guarantee a queued call depends on.
        if call_dedup::is_node_level_interface(&call.interface) {
            return Err(ProxyError::PermissionDenied(format!(
                "node-level interface '{}' cannot be reached through the durable outbox: it keeps \
                 no record to deduplicate a redelivery against",
                call.interface
            )));
        }

        // Without an unexpired instance certificate every delivery attempt
        // would present as anonymous, and the receiver refuses a keyed
        // call it cannot scope to a caller. Queuing it would mean ten
        // hours of retries ending in a dead letter, to say the same "no".
        let certified = self
            .registry
            .instance_cert(&call.caller_service_id)
            .is_some_and(|cert| !cert.is_expired());
        if !certified {
            return Err(ProxyError::PermissionDenied(format!(
                "service '{}' holds no unexpired instance certificate, so a queued call from it \
                 would be refused as anonymous at every delivery attempt",
                call.caller_service_id
            )));
        }

        let target = outbox.resolve_target(&call)?;

        // The one caller identity that cannot be rebuilt at delivery is a
        // guest's own forwarded caller, which is exactly what a self-call
        // uses. It is also local and by definition running, so durability
        // buys it nothing.
        if target == call.caller_service_id {
            return Err(ProxyError::UnsupportedTarget(format!(
                "service '{}' cannot enqueue a call to itself: it is local and already running, \
                 so there is nothing for a durable queue to survive",
                call.caller_service_id
            )));
        }

        // The immediate attempt is a *probe*, not a delivery, so it runs
        // under its own tight bound rather than the call's full budget.
        //
        // Without this the guest waits out a doomed connect and every
        // retry underneath it -- comfortably past the sandbox's own
        // `dispatch_epoch_timeout_secs` (5s), which interrupts the guest
        // mid-call and turns a successful "accepted for delivery" into a
        // trap. That is the opposite of what a fire-and-forget verb owes
        // its caller, and the outbox already *is* the retry mechanism, so
        // there is nothing to gain by waiting longer here.
        let probe = time::timeout(
            ENQUEUE_PROBE_BUDGET,
            self.invoke_inner(&self.request_from(&call, target)),
        )
        .await;
        match probe {
            Ok(Ok(_)) => Ok(()),
            // The probe ran out of its own budget: nothing is known about
            // the target, which is exactly the retryable case.
            Err(_) => {
                warn!("proxy enqueue probe timed out; queueing");
                outbox.store(&call).await
            }
            // Matched on the enum rather than compared against one
            // variant, so a future third disposition is a compile error
            // here instead of silently falling through to "terminal" --
            // which is exactly how `Delivered` slipped past this arm when
            // it was added for the worker.
            Ok(Err(e)) => match proxy_outbox::disposition_of(&e) {
                // The receiver already ran this call and could not hand
                // its result back. Reporting that to the guest as a
                // failure would be the same confusion the queued path was
                // fixed for, one layer up: the call succeeded.
                Disposition::Delivered => Ok(()),
                Disposition::Retry => {
                    warn!(error = %e, "proxy enqueue could not deliver now; queueing");
                    outbox.store(&call).await
                }
                // Terminal on the very first attempt: the caller is still
                // here to be told, which is a better answer than a dead
                // letter it cannot read.
                Disposition::Terminal => Err(e),
            },
        }
    }
}

#[async_trait::async_trait]
impl ServiceProxy for ProxyRouter {
    /// Dispatches a call, and -- when it fails and carried a fence --
    /// records it for an operator. See [`Self::record_failed_call`] for
    /// why an unkeyed failure writes nothing.
    async fn invoke(&self, req: ProxyRequest) -> Result<Value, ProxyError> {
        let outcome = self.invoke_inner(&req).await;
        if let Err(error) = &outcome {
            self.record_failed_call(&req, error).await;
        }
        outcome
    }

    async fn enqueue(&self, call: QueuedCall) -> Result<(), ProxyError> {
        ProxyRouter::enqueue_call(self, call).await
    }

    async fn saga_begin(&self, req: SagaBegin) -> Result<String, ProxyError> {
        ProxyRouter::saga_begin_impl(self, req).await
    }

    async fn saga_step(&self, req: SagaStepRequest) -> Result<Value, ProxyError> {
        ProxyRouter::saga_step_impl(self, req).await
    }

    async fn saga_commit(&self, service_id: &str, saga_id: &str) -> Result<(), ProxyError> {
        ProxyRouter::saga_commit_impl(self, service_id, saga_id).await
    }

    async fn saga_compensate(&self, service_id: &str, saga_id: &str) -> Result<(), ProxyError> {
        ProxyRouter::saga_compensate_impl(self, service_id, saga_id).await
    }

    async fn saga_status(&self, service_id: &str, saga_id: &str) -> Result<SagaInfo, ProxyError> {
        ProxyRouter::saga_status_impl(self, service_id, saga_id).await
    }
}
