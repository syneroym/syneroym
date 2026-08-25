//! The live WebSocket connection table, shared by every build.
//!
//! Lived inside `AppSandboxEngine` until C1. Moved out because a natively
//! linked app must push frames onto the same connections the router
//! registered, and reaching them through `Weak<AppSandboxEngine>` gave a
//! native app a table it could never see into -- `send` failed as
//! "Unknown connection ID", which is the wrong answer for the wrong
//! reason.

use std::sync::Arc;

use dashmap::DashMap;
use syneroym_app_host::types::http::FrameKind;
use tokio::sync::mpsc::{self, Receiver, Sender};

pub type WebSocketSender = Sender<(Vec<u8>, FrameKind)>;
pub type WebSocketReceiver = Receiver<(Vec<u8>, FrameKind)>;

/// `service_id -> conn_id -> sender`.
#[derive(Debug, Default)]
pub struct WebSocketSenders {
    inner: DashMap<String, Arc<DashMap<String, WebSocketSender>>>,
}

impl WebSocketSenders {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Registers a unicast sender channel for an active WebSocket connection,
    /// returning the receiver to drain in the router's connection loop.
    pub fn register(&self, service_id: &str, conn_id: &str) -> WebSocketReceiver {
        let (tx, rx) = mpsc::channel(32);
        let service_map = self
            .inner
            .entry(service_id.to_string())
            .or_insert_with(|| Arc::new(DashMap::new()))
            .clone();
        service_map.insert(conn_id.to_string(), tx);
        rx
    }

    /// Removes a unicast sender channel for a closed WebSocket connection.
    pub fn deregister(&self, service_id: &str, conn_id: &str) {
        if let Some(service_map) = self.inner.get(service_id) {
            service_map.remove(conn_id);
        }
    }

    /// Drops all WebSocket senders for a service.
    /// Called from undeploy/stop. Dropping the senders unblocks any active
    /// rx.recv() loops in the router, terminating the WebSockets cleanly.
    pub fn forget_service(&self, service_id: &str) {
        self.inner.remove(service_id);
    }

    /// `Err` distinguishes the two real failures the WIT's `result<_, string>`
    /// collapses: a `conn_id` this node never had, and one whose peer went
    /// away. Both become the same string at the WIT boundary; keeping them
    /// apart here is what lets a native app log the difference.
    pub async fn send(
        &self,
        service_id: &str,
        conn: &str,
        frame: Vec<u8>,
        kind: FrameKind,
    ) -> Result<(), String> {
        if let Some(service_map) = self.inner.get(service_id)
            && let Some(sender) = service_map.get(conn)
        {
            match sender.send((frame, kind)).await {
                Ok(_) => return Ok(()),
                Err(_) => return Err("Connection closed".to_string()),
            }
        }
        Err("Unknown connection ID".to_string())
    }
}
