//! Substrate node-local service wake worker.
//!
//! Periodic and event-triggered local driver for services declaring a
//! `[services.<name>.wake]` block (ADR-0031).

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use syneroym_app_orchestration::WakeSpec;
use syneroym_core::{
    event_bus::{Cursor, EventBus, ReadOutcome},
    local_registry::EndpointRegistry,
};
use syneroym_rpc::{CallOrigin, CallerContext, ProxyProtocol, ProxyRequest, ServiceProxy};
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::warn;

#[cfg(test)]
mod tests;

/// Provides wake targets for the worker.
pub(super) trait WakeTargets: Send + Sync {
    fn wake_targets(&self) -> Vec<(String, WakeSpec)>;
}

/// Dynamic wake targets read from the local endpoint registry's deploy facts.
pub(super) struct RegistryWakeTargets {
    registry: EndpointRegistry,
}

impl RegistryWakeTargets {
    pub(super) fn new(registry: EndpointRegistry) -> Self {
        Self { registry }
    }
}

impl WakeTargets for RegistryWakeTargets {
    fn wake_targets(&self) -> Vec<(String, WakeSpec)> {
        self.registry
            .wake_targets()
            .into_iter()
            .filter_map(|(service_id, json)| match serde_json::from_str::<WakeSpec>(&json) {
                Ok(spec) => Some((service_id, spec)),
                Err(e) => {
                    warn!(service_id = %service_id, "failed to parse wake spec: {e}");
                    None
                }
            })
            .collect()
    }
}

struct RunningGuard(Arc<AtomicBool>);

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

struct TargetState {
    bus_cursor: Option<Cursor>,
    last_run_at: u64,
    running: Arc<AtomicBool>,
    has_pending: Arc<AtomicBool>,
    warned_no_app_context: bool,
    warned_missing_endpoint: bool,
}

/// Background worker that wakes local services on events and intervals.
pub(super) struct WakeWorker {
    targets: Arc<dyn WakeTargets>,
    bus: Arc<EventBus>,
    proxy: Arc<dyn ServiceProxy>,
    registry: EndpointRegistry,
    states: HashMap<String, TargetState>,
}

impl WakeWorker {
    pub(super) fn new(
        targets: Arc<dyn WakeTargets>,
        bus: Arc<EventBus>,
        proxy: Arc<dyn ServiceProxy>,
        registry: EndpointRegistry,
    ) -> Self {
        Self { targets, bus, proxy, registry, states: HashMap::new() }
    }

    pub(super) async fn run(mut self, cancel: CancellationToken) {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = interval.tick() => {
                    self.tick().await;
                }
            }
        }
    }

    async fn tick(&mut self) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        self.tick_at(now).await;
    }

    pub(super) async fn tick_at(&mut self, now: u64) {
        let targets = self.targets.wake_targets();
        let target_ids: HashSet<String> = targets.iter().map(|(id, _)| id.clone()).collect();
        self.states.retain(|id, _| target_ids.contains(id));

        for (service_id, spec) in targets {
            self.tick_target(&service_id, &spec, now);
        }
    }

    fn tick_target(&mut self, service_id: &str, spec: &WakeSpec, now: u64) {
        let state = self.states.entry(service_id.to_string()).or_insert_with(|| TargetState {
            bus_cursor: None,
            last_run_at: 0,
            running: Arc::new(AtomicBool::new(false)),
            has_pending: Arc::new(AtomicBool::new(false)),
            warned_no_app_context: false,
            warned_missing_endpoint: false,
        });

        if state.running.load(Ordering::SeqCst) {
            return;
        }

        // Strictly node-local; never resolve via remote/community registry.
        if self.registry.lookup(service_id, spec.interface.as_str()).is_none() {
            if !state.warned_missing_endpoint {
                warn!(
                    service_id = %service_id,
                    interface = %spec.interface,
                    "wake target has no local endpoint registered; skipping"
                );
                state.warned_missing_endpoint = true;
            }
            return;
        }

        state.warned_missing_endpoint = false;

        let mut triggered = state.has_pending.swap(false, Ordering::SeqCst);
        if !spec.on_events.is_empty() && check_events(&self.bus, service_id, spec, state) {
            triggered = true;
        }

        let due = now.saturating_sub(state.last_run_at) >= u64::from(spec.every_secs);
        if !triggered && !due {
            return;
        }

        state.running.store(true, Ordering::SeqCst);
        state.last_run_at = now;

        spawn_wake_call(
            self.proxy.clone(),
            service_id.to_string(),
            spec.clone(),
            state.running.clone(),
            state.has_pending.clone(),
        );
    }
}

fn check_events(
    bus: &EventBus,
    service_id: &str,
    spec: &WakeSpec,
    state: &mut TargetState,
) -> bool {
    let Some(app_instance) = bus.app_instance_of(service_id) else {
        if !state.warned_no_app_context {
            warn!(
                service_id = %service_id,
                "wake target declared on_events but has no app context; events are app-scoped and will be ignored"
            );
            state.warned_no_app_context = true;
        }
        return false;
    };

    let now_ms =
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
    match bus.read(&app_instance, state.bus_cursor.as_ref(), 100, now_ms) {
        ReadOutcome::Events { events, last } => {
            state.bus_cursor = Some(last);
            events.iter().any(|e| spec.on_events.iter().any(|kind| kind == &e.kind))
        }
        ReadOutcome::Reload { now } => {
            state.bus_cursor = Some(now);
            true
        }
    }
}

fn wake_reply(val: Value) -> Result<Value, String> {
    let obj = match val {
        Value::String(s) => serde_json::from_str::<Value>(&s)
            .map_err(|e| format!("failed to parse reply string as JSON: {e}"))?,
        other => other,
    };

    if let Some(err_val) = obj.get("error")
        && !err_val.is_null()
    {
        return Err(err_val.to_string());
    }

    if let Some(result) = obj.get("result") { Ok(result.clone()) } else { Ok(obj) }
}

/// Warning texts of a wake reply, in order. Entries that are not
/// strings are skipped; a missing or non-array `warnings` is none.
fn reply_warnings(val: &Value) -> Vec<&str> {
    val.get("warnings")
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn spawn_wake_call(
    proxy: Arc<dyn ServiceProxy>,
    service_id: String,
    spec: WakeSpec,
    running: Arc<AtomicBool>,
    has_pending: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let _guard = RunningGuard(running);
        let params: Value = spec
            .params
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_else(|| Value::Array(vec![]));

        let req = ProxyRequest {
            target_service: service_id.clone(),
            interface: spec.interface.to_string(),
            method: spec.method,
            params,
            caller: CallerContext::service_system(&service_id),
            origin: CallOrigin::Native { service_id: Some(service_id.clone()) },
            protocol: ProxyProtocol::JsonRpcV1,
            idempotent: false,
            idempotency_key: None,
            timeout: Some(Duration::from_millis(u64::from(spec.timeout_ms))),
        };

        match proxy.invoke(req).await {
            Ok(raw_val) => match wake_reply(raw_val) {
                Ok(val) => {
                    for msg in reply_warnings(&val) {
                        warn!(service_id = %service_id, "{msg}");
                    }
                    if val.get("again").and_then(Value::as_bool).unwrap_or(false) {
                        has_pending.store(true, Ordering::SeqCst);
                    }
                }
                Err(err) => {
                    warn!(
                        service_id = %service_id,
                        error = %err,
                        "wake call failed"
                    );
                }
            },
            Err(err) => {
                warn!(
                    service_id = %service_id,
                    error = %err,
                    "wake call failed"
                );
            }
        }
    });
}
