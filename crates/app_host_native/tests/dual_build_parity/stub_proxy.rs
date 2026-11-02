use std::{
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};

use serde_json::{Value, json};
use syneroym_rpc::{ProxyError, ProxyRequest, ServiceProxy};
use tokio::time;

#[derive(Debug, Default)]
pub(crate) struct StubProxy {
    pub(crate) in_flight: AtomicUsize,
    pub(crate) max_in_flight: AtomicUsize,
    pub(crate) dropped: AtomicBool,
}

impl StubProxy {
    pub(crate) fn reset(&self) {
        self.in_flight.store(0, Ordering::SeqCst);
        self.max_in_flight.store(0, Ordering::SeqCst);
        self.dropped.store(false, Ordering::SeqCst);
    }
}

struct DropGuard<'a> {
    proxy: &'a StubProxy,
    completed: bool,
}

impl Drop for DropGuard<'_> {
    fn drop(&mut self) {
        self.proxy.in_flight.fetch_sub(1, Ordering::SeqCst);
        if !self.completed {
            self.proxy.dropped.store(true, Ordering::SeqCst);
        }
    }
}

#[async_trait::async_trait]
impl ServiceProxy for StubProxy {
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError> {
        let prev = self.in_flight.fetch_add(1, Ordering::SeqCst);
        let current = prev + 1;
        self.max_in_flight.fetch_max(current, Ordering::SeqCst);

        let mut guard = DropGuard { proxy: self, completed: false };

        let res = match (request.interface.as_str(), request.method.as_str()) {
            ("greeter", "greet") => Ok(json!({"greeting": "hello from stub"})),
            ("scripted", "echo") => Ok(request.params),
            ("scripted", "sleep") => {
                let ms = request.params.get("ms").and_then(Value::as_u64).unwrap_or(0);
                time::sleep(Duration::from_millis(ms)).await;
                Ok(json!("slept"))
            }
            ("scripted", "fail") => Err(ProxyError::Callee {
                code: -32000,
                message: "scripted failure".to_string(),
                data: None,
            }),
            _ => Err(ProxyError::UnsupportedTarget(format!(
                "StubProxy has no handler for {}.{}",
                request.interface, request.method
            ))),
        };

        guard.completed = true;
        res
    }
}
