use super::*;

/// How a `handle-request` call ended. `Err` from the enclosing
/// `Result` is reserved for host-side failure; everything a *guest* can do
/// lands in here, mirroring `StreamRequestOutcome`'s split.
#[derive(Debug)]
pub enum GuestHttpOutcome {
    Response(HttpResponse),
    Failed(GuestHttpFailure),
}

/// Why a guest HTTP call produced no usable response. Every variant maps to
/// 500 **except `Unavailable`, which maps to 503** -- resource exhaustion is
/// "try again", not "the guest broke".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestHttpFailure {
    /// The component does not export `handle-request`. Unreachable through
    /// a normal deploy (deploy-time validation refuses it); kept because the
    /// engine cannot assume its caller checked.
    NoHandler,
    /// The guest returned `Err(msg)` -- the handler failed. A guest
    /// *rejecting* a request returns `Ok` with a 4xx status instead.
    Declined(String),
    /// Fuel exhausted or the epoch deadline reached.
    BudgetExceeded(String),
    Trap(String),
    /// The return value was not `result<http-response, string>`.
    Malformed(String),
    /// No instance could be obtained: the per-service admission permit
    /// timed out, or wasmtime's pool refused
    /// (`PoolConcurrencyLimitError`).
    Unavailable(String),
}

impl AppSandboxEngine {
    /// Drops `service_id`'s guest HTTP admission semaphore.
    /// Called from the same undeploy path as `unsubscribe_all`; in-flight
    /// requests keep their own `OwnedSemaphorePermit` and finish, they just
    /// stop sharing a budget with a service that no longer exists.
    pub fn forget_guest_http_permits(&self, service_id: &str) {
        self.guest_http_permits.remove(service_id);
    }

    /// Runs one inbound HTTP request through the guest's `handle-request`
    /// export on a fresh per-call instance, bounded by
    /// `dispatch_epoch_ticks` (the 5s
    /// `dispatch_epoch_timeout_secs`), the service's fuel/memory quota, and
    /// this service's own guest-HTTP admission permit.
    ///
    /// `caller` is forwarded into `HostState.caller` exactly as
    /// `execute_wasm_json` does. `None` reaches here only for a route the
    /// deploy declared `public` -- the router answers 401 otherwise, before
    /// this function is called.
    pub async fn handle_guest_http_request(
        &self,
        service_id: &str,
        request: &HttpRequest,
        caller: Option<CallerContext>,
    ) -> Result<GuestHttpOutcome> {
        Self::validate_service_id(service_id)?;
        debug_assert!(
            !matches!(
                &caller,
                Some(c) if matches!(c.auth, AuthLevel::LocalElevated | AuthLevel::LocalReadOnly)
            ),
            "handle_guest_http_request must never receive a forwarded LocalElevated or \
             LocalReadOnly caller -- those contexts are reserved for invoke_lifecycle_hook and \
             authorize_rows respectively, neither of which calls this function"
        );

        // Bounded queuing instead of the pool's hard refusal. Per
        // service, so one service's traffic degrades that service.
        //
        // MUST NOT be written as `entry(..).or_insert_with(..)` followed by
        // an `.await` on the result: `entry` returns a `RefMut` that holds
        // the DashMap shard's write lock for as long as it lives, so
        // awaiting the permit would block every other task touching that
        // shard for up to `GUEST_HTTP_ADMISSION_TIMEOUT`. Clone the `Arc`
        // out and drop the guard in its own scope, BEFORE the await. Do not
        // "simplify" this back.
        let permits: Arc<Semaphore> = {
            let entry =
                self.guest_http_permits.entry(service_id.to_string()).or_insert_with(|| {
                    Arc::new(Semaphore::new(self.max_concurrent_guest_http_per_service as usize))
                });
            entry.value().clone()
        };
        let Ok(Ok(_permit)) =
            time::timeout(GUEST_HTTP_ADMISSION_TIMEOUT, permits.acquire_owned()).await
        else {
            return Ok(GuestHttpOutcome::Failed(GuestHttpFailure::Unavailable(
                "guest HTTP admission timed out".to_string(),
            )));
        };

        let caller = caller.unwrap_or_else(|| CallerContext::service_system(service_id));

        let _active = ActiveInstanceGuard::new();
        // `from_wire`: a guest HTTP request is always router ingress, never
        // a local dispatch path -- `invocation.caller()` must not report
        // `internal`. `syneroym:http`'s `caller-identity` stays the
        // authority for who the HTTP client is.
        let (mut store, instance, _quota) = match self
            .build_store_and_instantiate(
                service_id,
                caller,
                self.dispatch_epoch_ticks,
                InstanceOptions::from_wire(),
            )
            .await
        {
            Ok(v) => v,
            Err(e) if e.downcast_ref::<wasmtime::PoolConcurrencyLimitError>().is_some() => {
                return Ok(GuestHttpOutcome::Failed(GuestHttpFailure::Unavailable(e.to_string())));
            }
            Err(e) => return Err(e),
        };

        let Ok((func, results_len, _item)) = Self::get_wasm_func(
            &mut store,
            &instance,
            Some(http::HTTP_HANDLER_INTERFACE),
            "handle-request",
        ) else {
            return Ok(GuestHttpOutcome::Failed(GuestHttpFailure::NoHandler));
        };

        let args = [http::request_to_val(request)];
        let mut results = vec![Val::Bool(false); results_len];
        let exec_start = Instant::now();
        let call = func.call_async(&mut store, &args, &mut results).await;
        metrics::histogram!("substrate.wasm.execution_ms")
            .record(exec_start.elapsed().as_secs_f64() * 1000.0);
        if let Err(e) = call {
            let detail = truncate_detail(e.root_cause().to_string());
            return Ok(GuestHttpOutcome::Failed(match classify_call_failure(&e) {
                CallFailure::OutOfFuel | CallFailure::Deadline => {
                    GuestHttpFailure::BudgetExceeded(detail)
                }
                CallFailure::MemoryFault | CallFailure::Other => GuestHttpFailure::Trap(detail),
            }));
        }

        Ok(match http::response_from_results(&results) {
            Ok(response) => GuestHttpOutcome::Response(response),
            Err(failure) => GuestHttpOutcome::Failed(failure),
        })
    }
}
