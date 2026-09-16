use super::*;

/// Ceiling on a guest-supplied idempotency key. It travels on every
/// attempt and becomes part of a primary key on the receiving node, so it
/// is bounded like any other guest-controlled string that leaves the
/// sandbox. Generous next to any real key (a UUID is 36 bytes).
pub(crate) const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;

/// Maps the proxy's transport-agnostic `syneroym_rpc::ProxyError` onto the
/// guest-facing `syneroym:proxy/proxy::proxy-error` WIT variant.
fn map_proxy_error(e: RpcProxyError) -> proxy::ProxyError {
    match e {
        RpcProxyError::ServiceNotFound(s) => proxy::ProxyError::ServiceNotFound(s),
        RpcProxyError::UnsupportedProtocol(s) => proxy::ProxyError::UnsupportedProtocol(s),
        RpcProxyError::UnsupportedTarget(s) => proxy::ProxyError::UnsupportedTarget(s),
        RpcProxyError::PermissionDenied(s) => proxy::ProxyError::PermissionDenied(s),
        RpcProxyError::Transport(s) => proxy::ProxyError::Transport(s),
        RpcProxyError::Timeout(_) => proxy::ProxyError::TimedOut,
        RpcProxyError::Callee { code, message, data } => proxy::ProxyError::Callee(CalleeError {
            code,
            message,
            data: data.map(|v| v.to_string()),
        }),
        RpcProxyError::Internal(s) => proxy::ProxyError::Internal(s),
    }
}

impl proxy::Host for HostState {
    /// Originates a cross-service call through the Universal Proxy.
    /// Always constructs `CallOrigin::Guest` -- this is the only
    /// construction site a component can reach, so the proxy's guest
    /// native-capability gate (`ProxyRouter::check_native_capability_gate`)
    /// cannot be bypassed from guest code.
    async fn call(
        &mut self,
        target: CallTarget,
        interface: String,
        method: String,
        params: String,
        options: Option<CallOptions>,
    ) -> Result<String, proxy::ProxyError> {
        // ADR-0017 §7 is *local* read-only lookups; a cross-service call
        // mid-query is exactly the N+1-over-the-network cost the ADR
        // deliberately contained by keeping stage 4 out of the query-planner
        // business.
        if self.read_only {
            return Err(proxy::ProxyError::Internal(
                "stage-4 after-step instances may not originate proxy calls".to_string(),
            ));
        }
        let service_proxy = self
            .service_proxy
            .upgrade()
            .ok_or_else(|| proxy::ProxyError::Internal("proxy unavailable".to_string()))?;

        let params: Value = if params.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&params)
                .map_err(|e| proxy::ProxyError::Internal(format!("params must be JSON: {e}")))?
        };

        let (protocol_tag, idempotent, timeout_ms, routing_key, idempotency_key) = match &options {
            Some(o) => (
                o.protocol.as_deref(),
                o.idempotent,
                o.timeout_ms,
                o.routing_key.clone(),
                o.idempotency_key.clone(),
            ),
            None => (None, false, None, None, None),
        };
        let protocol =
            ProxyProtocol::parse(protocol_tag).map_err(proxy::ProxyError::UnsupportedProtocol)?;

        // ADR-0021 §2: the host supplies `app_instance_id`, the guest
        // supplies only the declared name. Resolution happens here, before
        // the `ProxyRequest` exists, so a guest never holds the resolved DID
        // and cannot snapshot it past a re-push.
        let target_service = match target {
            CallTarget::Service(service) => service,
            CallTarget::Dependency(name) => {
                let app_instance_id = self.app_instance_id.as_deref().ok_or_else(|| {
                    proxy::ProxyError::DependencyNotBound(format!(
                        "component '{}' was not deployed as part of an app instance, so it has no \
                         declared dependency '{name}'",
                        self.component_id
                    ))
                })?;
                let topology_key = TopologyKey::local(
                    // This string came out of a `service_app_context` row,
                    // not out of the guest. A corrupted row is a
                    // substrate-side fault, so it maps to `Internal`, not to
                    // the guest-facing "you are not bound".
                    AppInstanceId::try_new(app_instance_id).map_err(|e| {
                        proxy::ProxyError::Internal(format!(
                            "stored app context for '{}' is unreadable: {e}",
                            self.component_id
                        ))
                    })?,
                    LogicalServiceName::try_new(&name).map_err(|e| {
                        proxy::ProxyError::DependencyNotBound(format!(
                            "invalid dependency name: {e}"
                        ))
                    })?,
                );
                self.logical_resolver
                    .resolve(&topology_key, routing_key.as_deref().map(str::as_bytes))
                    .map_err(|e| {
                        proxy::ProxyError::DependencyNotBound(format!(
                            "dependency '{name}' of '{}' is not bound: {e}",
                            self.component_id
                        ))
                    })?
                    .to_string()
            }
        };

        // A guest proxying into its **own** service's
        // native `data-layer` forwards this invocation's real `HostState.
        // caller` (router-verified, or `service_system` if none reached
        // this guest -- see `prepare_wasm_execution`), so the receiving
        // `SynSvcNativeService::resolve_query_auth` sees who is actually
        // asking instead of always synthesizing `service_system` -- the
        // same-service exception (`ProxyRouter::check_native_capability_
        // gate`) already restricts this to the guest's own data, so this
        // cannot escalate to another service's rights.
        //
        // A genuine cross-service call still acts as itself: it does NOT
        // inherit the identity of whoever invoked *this* guest (no U->X
        // delegation exists in the current model), so a proxied call to a
        // *different* service cannot be used to escalate to the original
        // caller's rights. Real cross-service caller-delegation via UCAN is
        // not yet built. The self-proxy caller-forwarding rule is evaluated
        // against the *resolved* target: a component that reaches its own
        // service through a declared dependency name is still the same
        // service, so it still forwards its real caller.
        let caller = if target_service == self.component_id {
            self.caller.clone()
        } else {
            CallerContext::service_system(&self.component_id)
        };
        let req = ProxyRequest {
            target_service,
            interface,
            method,
            params,
            caller,
            origin: CallOrigin::Guest { service_id: self.component_id.clone() },
            protocol,
            idempotent,
            idempotency_key,
            timeout: timeout_ms.map(|ms| Duration::from_millis(ms.into())),
        };

        let value = service_proxy.invoke(req).await.map_err(map_proxy_error)?;
        // Mirrors the dispatch-boundary convention (a string result comes back raw,
        // not JSON-quoted) so guest code doesn't have to strip quotes.
        Ok(match value {
            Value::String(s) => s,
            other => other.to_string(),
        })
    }

    /// Hands a call to this service's durable outbox.
    ///
    /// Unlike `call`, a dependency name is **not** resolved here: the queued
    /// item stores the name, and resolution happens again on every delivery
    /// attempt, so a binding re-pushed while the item waits takes effect
    /// (ADR-0021 §2). Resolving once and storing the answer would snapshot
    /// the resolved DID for hours -- the exact thing that rule forbids, and
    /// for far longer than a guest could manage on its own.
    async fn enqueue(
        &mut self,
        target: CallTarget,
        interface: String,
        method: String,
        params: String,
        options: Option<CallOptions>,
    ) -> Result<(), proxy::ProxyError> {
        if self.read_only {
            return Err(proxy::ProxyError::Internal(
                "stage-4 after-step instances may not originate proxy calls".to_string(),
            ));
        }
        let service_proxy = self
            .service_proxy
            .upgrade()
            .ok_or_else(|| proxy::ProxyError::Internal("proxy unavailable".to_string()))?;

        let params: Value = if params.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&params)
                .map_err(|e| proxy::ProxyError::Internal(format!("params must be JSON: {e}")))?
        };

        let options = options.unwrap_or(CallOptions {
            protocol: None,
            idempotent: false,
            timeout_ms: None,
            routing_key: None,
            idempotency_key: None,
        });

        // Refused before anything is resolved, written, or attempted. A
        // queued call is delivered at least once, so one with no fence
        // would run the target twice on the first retry -- there is no
        // safe way to accept this, and naming the missing field is what
        // makes the refusal actionable.
        let idempotency_key = options.idempotency_key.clone().ok_or_else(|| {
            proxy::ProxyError::Internal(
                "enqueue requires call-options.idempotency-key: a queued call is delivered at \
                 least once, so it must carry a key the receiver can deduplicate on"
                    .to_string(),
            )
        })?;
        // Present is not the same as usable. An empty key is the sharp
        // case: it becomes this service's queue key, so the *first*
        // enqueue takes it and every later one is silently treated as a
        // duplicate of that one and dropped -- a guest would see nothing
        // but success while nothing was ever queued. The length bound is
        // the ordinary one for a guest-controlled string that becomes a
        // primary-key component on the receiving node.
        if idempotency_key.trim().is_empty() {
            return Err(proxy::ProxyError::Internal(
                "enqueue requires a non-empty call-options.idempotency-key: an empty key would be \
                 shared by every call this service queues"
                    .to_string(),
            ));
        }
        if idempotency_key.len() > MAX_IDEMPOTENCY_KEY_BYTES {
            return Err(proxy::ProxyError::Internal(format!(
                "call-options.idempotency-key is {} bytes; the limit is \
                 {MAX_IDEMPOTENCY_KEY_BYTES}",
                idempotency_key.len()
            )));
        }

        // Validated here so an unsupported tag fails at the call rather
        // than hours later in a dead letter.
        ProxyProtocol::parse(options.protocol.as_deref())
            .map_err(proxy::ProxyError::UnsupportedProtocol)?;

        let target = match target {
            CallTarget::Service(service) => QueuedTarget::Service(service),
            CallTarget::Dependency(name) => {
                // Validated now, resolved later: an unusable name should
                // fail at the call, but the resolution itself must happen
                // at delivery.
                LogicalServiceName::try_new(&name).map_err(|e| {
                    proxy::ProxyError::DependencyNotBound(format!("invalid dependency name: {e}"))
                })?;
                if self.app_instance_id.is_none() {
                    return Err(proxy::ProxyError::DependencyNotBound(format!(
                        "component '{}' was not deployed as part of an app instance, so it has no \
                         declared dependency '{name}'",
                        self.component_id
                    )));
                }
                QueuedTarget::Dependency(name)
            }
        };

        service_proxy
            .enqueue(QueuedCall {
                app_instance_id: self.app_instance_id.clone(),
                caller_service_id: self.component_id.clone(),
                target,
                routing_key: options.routing_key,
                interface,
                method,
                params,
                idempotency_key,
                protocol: options.protocol,
                timeout_ms: options.timeout_ms.map(u64::from),
            })
            .await
            .map_err(map_proxy_error)
    }
}

fn rpc_saga_state_to_wit(state: RpcSagaState) -> WitSagaState {
    match state {
        RpcSagaState::Open => WitSagaState::Open,
        RpcSagaState::Compensating => WitSagaState::Compensating,
        RpcSagaState::Compensated => WitSagaState::Compensated,
        RpcSagaState::Failed => WitSagaState::Failed,
    }
}

impl saga::Host for HostState {
    /// Opens a saga (ADR-0023 §7, as amended). Each of this interface's
    /// five functions follows `proxy::Host::call`'s own shape:
    /// the `read_only` refusal first, then `service_proxy.upgrade()`, then
    /// params parsing, then the call.
    async fn begin(
        &mut self,
        name: String,
        deadline_secs: Option<u64>,
    ) -> Result<String, proxy::ProxyError> {
        if self.read_only {
            return Err(proxy::ProxyError::Internal(
                "stage-4 after-step instances may not originate proxy calls".to_string(),
            ));
        }
        let service_proxy = self
            .service_proxy
            .upgrade()
            .ok_or_else(|| proxy::ProxyError::Internal("proxy unavailable".to_string()))?;
        service_proxy
            .saga_begin(SagaBegin {
                caller_service_id: self.component_id.clone(),
                app_instance_id: self.app_instance_id.clone(),
                name,
                deadline_secs,
            })
            .await
            .map_err(map_proxy_error)
    }

    /// Takes one forward step. Follows `enqueue`'s own target handling
    /// exactly: a dependency name is validated, never resolved here --
    /// resolution happens host-side, at the moment of dispatch.
    async fn step(
        &mut self,
        saga_id: String,
        target: CallTarget,
        interface: String,
        method: String,
        params: String,
        options: Option<CallOptions>,
    ) -> Result<String, proxy::ProxyError> {
        if self.read_only {
            return Err(proxy::ProxyError::Internal(
                "stage-4 after-step instances may not originate proxy calls".to_string(),
            ));
        }
        let service_proxy = self
            .service_proxy
            .upgrade()
            .ok_or_else(|| proxy::ProxyError::Internal("proxy unavailable".to_string()))?;

        let params: Value = if params.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&params)
                .map_err(|e| proxy::ProxyError::Internal(format!("params must be JSON: {e}")))?
        };

        let options = options.unwrap_or(CallOptions {
            protocol: None,
            idempotent: false,
            timeout_ms: None,
            routing_key: None,
            idempotency_key: None,
        });
        ProxyProtocol::parse(options.protocol.as_deref())
            .map_err(proxy::ProxyError::UnsupportedProtocol)?;

        let target = match target {
            CallTarget::Service(service) => QueuedTarget::Service(service),
            CallTarget::Dependency(name) => {
                LogicalServiceName::try_new(&name).map_err(|e| {
                    proxy::ProxyError::DependencyNotBound(format!("invalid dependency name: {e}"))
                })?;
                if self.app_instance_id.is_none() {
                    return Err(proxy::ProxyError::DependencyNotBound(format!(
                        "component '{}' was not deployed as part of an app instance, so it has no \
                         declared dependency '{name}'",
                        self.component_id
                    )));
                }
                QueuedTarget::Dependency(name)
            }
        };

        let value = service_proxy
            .saga_step(SagaStepRequest {
                caller_service_id: self.component_id.clone(),
                app_instance_id: self.app_instance_id.clone(),
                saga_id,
                target,
                routing_key: options.routing_key,
                interface,
                method,
                params,
                idempotency_key: options.idempotency_key,
                protocol: options.protocol,
                timeout_ms: options.timeout_ms.map(u64::from),
            })
            .await
            .map_err(map_proxy_error)?;
        Ok(match value {
            Value::String(s) => s,
            other => other.to_string(),
        })
    }

    async fn commit(&mut self, saga_id: String) -> Result<(), proxy::ProxyError> {
        if self.read_only {
            return Err(proxy::ProxyError::Internal(
                "stage-4 after-step instances may not originate proxy calls".to_string(),
            ));
        }
        let service_proxy = self
            .service_proxy
            .upgrade()
            .ok_or_else(|| proxy::ProxyError::Internal("proxy unavailable".to_string()))?;
        service_proxy.saga_commit(&self.component_id, &saga_id).await.map_err(map_proxy_error)
    }

    async fn compensate(&mut self, saga_id: String) -> Result<(), proxy::ProxyError> {
        if self.read_only {
            return Err(proxy::ProxyError::Internal(
                "stage-4 after-step instances may not originate proxy calls".to_string(),
            ));
        }
        let service_proxy = self
            .service_proxy
            .upgrade()
            .ok_or_else(|| proxy::ProxyError::Internal("proxy unavailable".to_string()))?;
        service_proxy.saga_compensate(&self.component_id, &saga_id).await.map_err(map_proxy_error)
    }

    async fn status(&mut self, saga_id: String) -> Result<SagaStatus, proxy::ProxyError> {
        if self.read_only {
            return Err(proxy::ProxyError::Internal(
                "stage-4 after-step instances may not originate proxy calls".to_string(),
            ));
        }
        let service_proxy = self
            .service_proxy
            .upgrade()
            .ok_or_else(|| proxy::ProxyError::Internal("proxy unavailable".to_string()))?;
        let info = service_proxy
            .saga_status(&self.component_id, &saga_id)
            .await
            .map_err(map_proxy_error)?;
        Ok(SagaStatus {
            saga_id: info.saga_id,
            name: info.name,
            state: rpc_saga_state_to_wit(info.state),
            steps: info.steps,
            compensated_steps: info.compensated_steps,
            created_at: info.created_at,
            deadline_at: info.deadline_at,
            last_error: info.last_error,
        })
    }
}
