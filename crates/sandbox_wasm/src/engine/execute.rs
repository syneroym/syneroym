use super::*;

impl AppSandboxEngine {
    /// Execute a WASM component for a given service, returning the guest's
    /// results as the string-shaped boundary contract every existing caller
    /// relies on (see [`crate::conversions::wasm_results_to_json_string`]).
    /// Test/dev-harness entry point only (smoke tests, the messaging test
    /// driver, `invoke_test_context`) -- always dispatches as
    /// `service_system` (`caller: None` below). A real caller reaching a
    /// guest belongs on [`Self::execute_wasm_json`].
    pub async fn execute_wasm(
        &self,
        service_id: &str,
        interface_name: &str,
        request: &JsonRpcRequest,
    ) -> Result<String> {
        let wasm_results = self
            .execute_wasm_vals(service_id, interface_name, request, None, InvocationOrigin::Local)
            .await?;
        conversions::wasm_results_to_json_string(&wasm_results)
    }

    /// Typed entry point: the guest's results as a real JSON
    /// [`Value`], with no string special-case. Used by the Universal Proxy
    /// (`ProxyRouter::invoke_local`) and the inbound `JsonRpcToWasm` route.
    ///
    /// `caller`, when `Some`, becomes the invoked guest's `HostState.caller`
    /// instead of the synthesized `service_system`
    /// [`prepare_wasm_execution`] falls back to on `None` -- so the guest's
    /// own host-function reads see who is actually asking. `dispatch.rs`'s
    /// `JsonRpcToWasm` branch passes the router-verified caller (or `None`
    /// for an unauthenticated connection, which WASM guests admit);
    /// `ProxyRouter::invoke_local`'s `WasmChannel` arm deliberately passes
    /// `None` -- a proxied guest-to-guest call is a different, not-yet-built
    /// delegation question (see that call site's own comment).
    pub async fn execute_wasm_json(
        &self,
        service_id: &str,
        interface_name: &str,
        request: &JsonRpcRequest,
        caller: Option<CallerContext>,
    ) -> Result<Value> {
        let wasm_results = self
            .execute_wasm_vals(service_id, interface_name, request, caller, InvocationOrigin::Local)
            .await?;
        conversions::wasm_results_to_json(&wasm_results)
    }

    /// A call that arrived over the network. The only caller is the
    /// router's own JSON-RPC dispatch (`dispatch.rs`); every other path
    /// into a component originates on this node and uses
    /// [`Self::execute_wasm_json`]. The guest sees the difference through
    /// `syneroym:invocation/invocation.caller`: this entry point makes it
    /// `verified(did)` or `anonymous`, the local one makes it `internal`.
    pub async fn execute_wasm_json_from_wire(
        &self,
        service_id: &str,
        interface_name: &str,
        request: &JsonRpcRequest,
        caller: Option<CallerContext>,
    ) -> Result<Value> {
        let wasm_results = self
            .execute_wasm_vals(service_id, interface_name, request, caller, InvocationOrigin::Wire)
            .await?;
        conversions::wasm_results_to_json(&wasm_results)
    }

    /// The `rpc` health-probe entry point:
    /// identical to [`Self::execute_wasm_json`] with `caller: None` (a
    /// substrate-originated probe, the same choice `ProxyRouter::
    /// invoke_local` makes for a guest-to-guest call), except bounded by
    /// `probe_instance_permits` -- see that field's doc comment for why a
    /// health sweep's own concurrent fan-out needs its own cap rather than
    /// relying on wasmtime's pool to reject the excess.
    pub async fn execute_probe_json(
        &self,
        service_id: &str,
        interface_name: &str,
        request: &JsonRpcRequest,
    ) -> Result<Value> {
        let _permit = self
            .probe_instance_permits
            .acquire()
            .await
            .map_err(|_| anyhow!("probe_instance_permits semaphore is closed"))?;
        self.execute_wasm_json(service_id, interface_name, request, None).await
    }

    /// Everything shared by [`Self::execute_wasm`]/[`Self::execute_wasm_json`]:
    /// resolves and instantiates the target component, binds JSON-RPC params
    /// to its typed signature, calls it, and maps quota/memory traps -- up to
    /// but not including result serialization, which the two typed/string
    /// entry points above handle differently.
    async fn execute_wasm_vals(
        &self,
        service_id: &str,
        interface_name: &str,
        request: &JsonRpcRequest,
        caller: Option<CallerContext>,
        origin: InvocationOrigin,
    ) -> Result<Vec<Val>> {
        Self::validate_service_id(service_id)?;
        let _guard = ActiveInstanceGuard::new();
        debug!("starting to execute wasm");

        // TODO: Later optimize this by caching things like function parameter details
        // on first execution, so we don't have to do the same lookups every time.
        let (mut store, func, results_len, item) = self
            .prepare_wasm_execution(service_id, interface_name, &request.method, caller, origin)
            .await?;

        // Parse parameters based on ComponentFunc signature
        let params_iter = match &item {
            ComponentItem::ComponentFunc(f) => f.params(),
            _ => return Err(anyhow::anyhow!("Expected a function item")),
        };

        debug!("extracted the function and parameter iter");

        // Bind JSON-RPC params to the typed signature (named or positional).
        let wasm_params = conversions::json_to_wasm_params(params_iter, &request.params)?;

        debug!("created input types");

        let mut wasm_results = vec![Val::Bool(false); results_len];
        debug!("created result types");

        let exec_start = Instant::now();
        let res = func.call_async(&mut store, &wasm_params, &mut wasm_results).await;
        metrics::histogram!("substrate.wasm.execution_ms")
            .record(exec_start.elapsed().as_secs_f64() * 1000.0);

        debug!("called wasm function, processing results");

        if let Err(e) = res {
            match classify_call_failure(&e) {
                CallFailure::OutOfFuel => {
                    warn!("Wasm execution exceeded fuel limit for service: {}", service_id);
                    return Err(anyhow::anyhow!(
                        "QuotaExceeded: Wasm execution exceeded fuel limit"
                    ));
                }
                CallFailure::MemoryFault => {
                    return Err(anyhow::anyhow!(
                        "MemoryFault: Wasm execution exceeded memory limit"
                    ));
                }
                // Deadline and Other are indistinguishable here, matching
                // pre-refactor behaviour: neither was classified before, so
                // both fell through to the raw error.
                CallFailure::Deadline | CallFailure::Other => return Err(e.into()),
            }
        }

        Ok(wasm_results)
    }

    /// Resolves a service's FDAE policy for `build_store_and_instantiate`,
    /// via `fdae_policies` next to the component cache. On a cache miss,
    /// loads from `substrate.db` (durable across a substrate restart --
    /// `load_cached_wasm` recompiles from disk and the next instantiation
    /// re-resolves from here, not from any in-memory deploy result) and
    /// parses once. A parse failure here is fail-closed-**absent**: log and
    /// cache `None` rather than deny every read for the service. The deploy
    /// path (`control_plane`'s `orchestration.rs`) is what rejects a bad
    /// policy before it's ever persisted -- a row that fails to parse *here*
    /// means the DB was tampered with or the crate's schema moved since
    /// deploy, and the alternative (denying every read) would take a
    /// previously-working service down on a substrate upgrade rather than on
    /// the bad edit that actually caused it. A storage *read* failure is a
    /// different case and is **not** cached at all (see the `Err` arm
    /// below): unlike a genuinely absent or malformed row, it says nothing
    /// about whether a policy exists, so treating it as "no policy" and
    /// remembering that would silently disable FDAE for the service until
    /// the next redeploy over what may be a one-off transient error.
    pub(crate) async fn resolve_fdae_policy(&self, service_id: &str) -> Option<Arc<Policy>> {
        if let Some(cached) = self.fdae_policies.get(service_id) {
            return cached.clone();
        }
        // Captured *before* the cross-await storage read, so a concurrent
        // eviction (redeploy) that races this load can be detected below --
        // see `fdae_policy_generation`'s doc comment. `.get()` immutably
        // borrows a shard just long enough to copy the `u64` out; the shard
        // is not held across the `.await`.
        let generation_before =
            self.fdae_policy_generation.get(service_id).map(|g| *g).unwrap_or(0);
        let resolved = match self.storage_provider.load_fdae_policy(service_id).await {
            Ok(Some(doc)) => match syneroym_fdae::parse_and_validate(&doc) {
                Ok(policy) => Some(Arc::new(policy)),
                Err(e) => {
                    error!(
                        "FDAE policy for service {} failed to parse from storage (treating as \
                         policy-absent): {}",
                        service_id, e
                    );
                    None
                }
            },
            Ok(None) => None,
            Err(e) => {
                // A transient storage failure (a busy connection under load,
                // say) is not "this service has no policy" -- caching it as
                // such would silently disable FDAE for the service for the
                // rest of the process's uptime on one blip. Return uncached
                // instead, the same "don't trust an uncertain read" treatment
                // the generation-race branch below gives a load that lost to
                // a concurrent eviction, so the next call retries against
                // storage rather than serving this one's answer forever.
                error!("Failed to load FDAE policy for service {}: {}", service_id, e);
                return None;
            }
        };
        let generation_after = self.fdae_policy_generation.get(service_id).map(|g| *g).unwrap_or(0);
        if generation_before == generation_after {
            self.fdae_policies.insert(service_id.to_string(), resolved.clone());
        } else {
            // An eviction (redeploy) landed while this load was in flight --
            // this result may already be stale. Return it for *this* call
            // (it was the correct answer at some point during the read, and
            // returning it beats blocking or erroring), but do not cache it:
            // the next call re-resolves fresh rather than serving a policy a
            // redeploy already superseded.
            debug!(
                service_id,
                "FDAE policy resolution raced a redeploy; serving uncached this time"
            );
        }
        resolved
    }

    /// Simple test function to invoke test context. `run` (`wit/host/host.wit`
    /// `app::run`) is zero-arg, so `request_ctx` is not threaded through as a
    /// JSON-RPC param (it never was: an earlier converter also dropped it,
    /// silently, for any zero-arg target).
    pub async fn invoke_test_context(
        &self,
        service_id: &str,
        component_id: &str,
        _request_ctx: &str,
    ) -> Result<String> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "run".to_string(), // Default method for test
            params: Value::Null,
            id: None,
            idempotency_key: None,
        };
        self.execute_wasm(service_id, component_id, &request).await
    }

    /// Bumps `fdae_policy_generation` for `service_id`, marking any
    /// `resolve_fdae_policy` load currently in flight for it as stale --
    /// called alongside every `fdae_policies` eviction. See
    /// `fdae_policy_generation`'s doc comment.
    pub(crate) fn bump_fdae_policy_generation(&self, service_id: &str) {
        *self.fdae_policy_generation.entry(service_id.to_string()).or_insert(0) += 1;
    }
}
