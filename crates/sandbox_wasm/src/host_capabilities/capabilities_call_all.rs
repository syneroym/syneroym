use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use serde_json::Value;
use syneroym_rpc::{ProxyRequest, ServiceProxy};
use syneroym_wit_interfaces::host::syneroym::proxy::proxy::{self, CallRequest};
use tokio::{
    sync::OwnedSemaphorePermit,
    task::{Id as TaskId, JoinSet},
};
use tracing::Instrument;

use super::*;

pub(super) const CALL_CANCEL_GRACE: Duration = Duration::from_millis(250);

/// Proof token certifying that wait credit was measured and granted exclusively
/// by the batched call execution path.
#[derive(Clone, Copy, Debug)]
pub struct WaitCredit(Duration);

impl WaitCredit {
    #[must_use]
    fn new(d: Duration) -> Self {
        Self(d)
    }

    /// Returns the credited duration.
    #[must_use]
    pub(crate) fn duration(&self) -> Duration {
        self.0
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn for_test(d: Duration) -> Self {
        Self(d)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CallAllRefusal {
    ReadOnly,
    ProxyUnavailable,
    TooManyCalls { max: usize, got: usize },
    Busy,
    AllowanceUsed,
}

impl CallAllRefusal {
    pub(super) fn into_proxy_error(self) -> proxy::ProxyError {
        match self {
            Self::ReadOnly => proxy::ProxyError::Internal(
                "stage-4 after-step instances may not originate proxy calls".to_string(),
            ),
            Self::ProxyUnavailable => proxy::ProxyError::Internal("proxy unavailable".to_string()),
            Self::TooManyCalls { max, got } => proxy::ProxyError::UnsupportedTarget(format!(
                "call-all takes at most {max} calls, got {got}"
            )),
            Self::Busy => proxy::ProxyError::Internal(
                "call-all busy: too many guests are waiting in call-all; try again".to_string(),
            ),
            Self::AllowanceUsed => proxy::ProxyError::Internal(
                "call-all busy: this guest call has no wait time left in call-all".to_string(),
            ),
        }
    }

    pub(super) fn metric_label(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::ProxyUnavailable => "proxy_unavailable",
            Self::TooManyCalls { .. } => "too_many_calls",
            Self::Busy => "busy",
            Self::AllowanceUsed => "allowance_used",
        }
    }
}

pub(super) fn record_refusal(refusal: CallAllRefusal) -> proxy::ProxyError {
    metrics::counter!("substrate.proxy.call_all.refused", "reason" => refusal.metric_label())
        .increment(1);
    refusal.into_proxy_error()
}

struct WaiterGuard {
    _permit: OwnedSemaphorePermit,
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        metrics::gauge!("substrate.proxy.call_all.waiting").decrement(1.0);
    }
}

fn spawn_call(
    set: &mut JoinSet<(usize, Result<Value, proxy::ProxyError>)>,
    task_to_slot: &mut HashMap<TaskId, usize>,
    proxy: &Arc<dyn ServiceProxy>,
    (i, req, timeout): (usize, ProxyRequest, Duration),
) {
    let p = Arc::clone(proxy);
    let handle = set.spawn(
        async move {
            let r = tokio::time::timeout(timeout + CALL_CANCEL_GRACE, p.invoke(req)).await;
            let outcome = match r {
                Ok(Ok(val)) => Ok(val),
                Ok(Err(rpc_err)) => Err(capabilities_proxy::map_proxy_error(rpc_err)),
                Err(_elapsed) => Err(proxy::ProxyError::TimedOut),
            };
            (i, outcome)
        }
        .instrument(tracing::Span::current()),
    );
    task_to_slot.insert(handle.id(), i);
}

async fn drain_batch(
    set: &mut JoinSet<(usize, Result<Value, proxy::ProxyError>)>,
    task_to_slot: &mut HashMap<TaskId, usize>,
    pending: &mut impl Iterator<Item = (usize, ProxyRequest, Duration)>,
    proxy: &Arc<dyn ServiceProxy>,
    slots: &mut [Option<Result<String, proxy::ProxyError>>],
) {
    while let Some(res) = set.join_next_with_id().await {
        match res {
            Ok((_id, (i, r))) => {
                task_to_slot.remove(&_id);
                slots[i] = Some(r.map(capabilities_proxy::reply_text));
            }
            Err(join_err) => {
                let id = join_err.id();
                if let Some(idx) = task_to_slot.remove(&id) {
                    tracing::warn!(task_id = ?id, error = ?join_err, "call-all host task failed");
                    slots[idx] = Some(Err(proxy::ProxyError::Internal(
                        "call-all: call failed in the host".to_string(),
                    )));
                }
            }
        }
        if let Some(item) = pending.next() {
            spawn_call(set, task_to_slot, proxy, item);
        }
    }
}

impl HostState {
    pub(super) async fn execute_call_all(
        &mut self,
        calls: Vec<CallRequest>,
    ) -> Result<Vec<Result<String, proxy::ProxyError>>, proxy::ProxyError> {
        if self.read_only {
            return Err(record_refusal(CallAllRefusal::ReadOnly));
        }
        if calls.is_empty() {
            return Ok(vec![]);
        }
        let limits = self.call_all.limits;
        if calls.len() > limits.max_calls {
            return Err(record_refusal(CallAllRefusal::TooManyCalls {
                max: limits.max_calls,
                got: calls.len(),
            }));
        }
        let service_proxy = self
            .service_proxy
            .upgrade()
            .ok_or_else(|| record_refusal(CallAllRefusal::ProxyUnavailable))?;
        let allowance = self.epoch.allowance();
        if allowance < MIN_BATCH_ALLOWANCE {
            return Err(record_refusal(CallAllRefusal::AllowanceUsed));
        }
        let permit = self.call_all.waiters.clone().try_acquire_owned().map_err(|_| {
            tracing::warn!(
                service_id = %self.component_id,
                "call-all busy: too many guests are waiting in call-all; try again"
            );
            record_refusal(CallAllRefusal::Busy)
        })?;
        metrics::gauge!("substrate.proxy.call_all.waiting").increment(1.0);
        let _waiter_guard = WaiterGuard { _permit: permit };

        metrics::counter!("substrate.proxy.call_all.batches").increment(1);

        let total = limits.total_timeout.min(allowance);
        let mut slots: Vec<Option<Result<String, proxy::ProxyError>>> = vec![None; calls.len()];
        let mut runnable = Vec::new();
        for (i, call) in calls.into_iter().enumerate() {
            let timeout = limits.call_timeout(call.options.as_ref().and_then(|o| o.timeout_ms));
            match self.proxy_request(call, Some(timeout)) {
                Ok(req) => runnable.push((i, req, timeout)),
                Err(e) => slots[i] = Some(Err(e)),
            }
        }

        metrics::counter!("substrate.proxy.call_all.calls").increment(runnable.len() as u64);

        let started = Instant::now();
        let mut set = JoinSet::new();
        let mut task_to_slot = HashMap::new();
        let mut pending = runnable.into_iter();

        while set.len() < limits.max_concurrency {
            let Some(item) = pending.next() else { break };
            spawn_call(&mut set, &mut task_to_slot, &service_proxy, item);
        }

        let _ = tokio::time::timeout(
            total,
            drain_batch(&mut set, &mut task_to_slot, &mut pending, &service_proxy, &mut slots),
        )
        .await;

        let elapsed = started.elapsed();
        self.epoch.credit(WaitCredit::new(elapsed));
        metrics::histogram!("substrate.proxy.call_all.duration_ms")
            .record(elapsed.as_millis() as f64);

        let mut timed_out_count = 0u64;
        let results: Vec<Result<String, proxy::ProxyError>> = slots
            .into_iter()
            .map(|s| {
                let res = s.unwrap_or(Err(proxy::ProxyError::TimedOut));
                if matches!(res, Err(proxy::ProxyError::TimedOut)) {
                    timed_out_count += 1;
                }
                res
            })
            .collect();

        if timed_out_count > 0 {
            metrics::counter!("substrate.proxy.call_all.timed_out").increment(timed_out_count);
        }

        Ok(results)
    }
}
