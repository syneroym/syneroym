//! Embedded in-process MQTT broker (`rumqttd`) backing `syneroym:messaging`
//! (ADR-0010, M3B Slice 6A).
//!
//! `rumqttd::Broker::new` spawns the router's event loop on its own native
//! OS thread as a side effect of construction; `Broker::start` is never
//! called here since it only starts network listeners (v4/v5/ws) and
//! errors out if none are configured. Leaving `v4`/`v5`/`ws`/etc. unset in
//! [`MqttBroker::new`] means no TCP/MQTT listener is ever bound (ADR-0010
//! Finding A5): the only way in or out of the broker is `Broker::link`.
//!
//! `rumqttd` 0.20.0 exposes no way to stop that router thread once
//! started (confirmed by reading its source: the `Router` struct holds a
//! permanent internal clone of its own event-channel sender, so dropping
//! every external link never closes the channel, and no `Shutdown` event
//! variant exists anywhere in the crate). [`MqttBroker::drop`] therefore
//! only guarantees that no *new* subscriptions/links are created and that
//! every live forwarding task (the Tokio tasks this crate itself spawns
//! per `subscribe`) stops promptly; the underlying router OS thread is an
//! accepted, harmless leak (parked on a blocking channel `recv`, no CPU
//! use) until process exit.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

#[cfg(test)]
mod tests;

use std::{fmt, sync::Arc};

#[cfg(test)]
use bytes::Bytes;
#[cfg(test)]
use rumqttd::protocol::{Packet, Publish};
use rumqttd::{
    Broker, Config as RumqttdConfig, Notification, RouterConfig,
    local::{LinkRx, LinkTx},
};
use thiserror::Error;
use tokio::{
    sync::{Mutex, mpsc, mpsc::error::TrySendError},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use tracing::warn;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum MessagingError {
    #[error("internal: {0}")]
    Internal(String),
}

// Mirrors `syneroym_core::config::MessagingConfig` (same `channel_capacity`
// field, `usize` here vs. `u64` there) -- `core` can't depend on this
// crate, so this is intentional duplication, not accidental drift.
#[derive(Debug, Clone)]
pub struct MqttBrokerConfig {
    pub channel_capacity: usize,
}

impl Default for MqttBrokerConfig {
    fn default() -> Self {
        Self { channel_capacity: 1024 }
    }
}

/// Prefixes `topic` into the calling service's namespace unless it is
/// already a fully-qualified `svc/<other_service>/...` topic, in which
/// case it is used literally (explicit cross-service opt-in per
/// ADR-0010's Topic Namespace Isolation section). **Subscribe-side only**
/// — see [`namespace_topic_for_publish`] for why publish cannot reuse this.
pub fn namespace_topic(service_id: &str, topic: &str) -> String {
    if topic.starts_with("svc/") { topic.to_string() } else { format!("svc/{service_id}/{topic}") }
}

/// Prefixes `topic` into the calling service's namespace unconditionally,
/// even if `topic` already looks like a fully-qualified `svc/<other>/...`
/// topic. Publish has no cross-service opt-in (only subscribe does, via
/// [`namespace_topic`]): letting a caller-supplied `svc/` prefix through
/// literally on publish would let any caller spoof any other service's
/// topic namespace.
pub fn namespace_topic_for_publish(service_id: &str, topic: &str) -> String {
    format!("svc/{service_id}/{topic}")
}

pub struct MqttBroker {
    broker: Broker,
    /// Shared substrate-wide: every WASM guest publish and every native
    /// `dispatch_messaging` publish across the process serializes through
    /// this one link. `try_publish` itself is fast/non-blocking, so this
    /// is a contention point rather than a correctness bug -- a deliberate
    /// accepted tradeoff for Slice 6A, capping publish throughput on
    /// total substrate-wide volume rather than per-service/per-topic
    /// volume. Worth moving to a per-caller link if publish volume
    /// becomes a real bottleneck.
    host_link: Mutex<(LinkTx, LinkRx)>,
    cancellation: CancellationToken,
    channel_capacity: usize,
}

impl fmt::Debug for MqttBroker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MqttBroker").field("channel_capacity", &self.channel_capacity).finish()
    }
}

impl MqttBroker {
    pub fn new(config: MqttBrokerConfig) -> Result<Self, MessagingError> {
        if config.channel_capacity == 0 {
            // `mpsc::channel(0)` panics (`assert!(buffer > 0)`) the first
            // time a subscribe tries to create its forwarding channel;
            // caught here, at construction, so a bad `[mqtt]
            // channel_capacity = 0` in the substrate TOML surfaces as a
            // clean config error instead of a panic well downstream.
            return Err(MessagingError::Internal(
                "MqttBrokerConfig::channel_capacity must be greater than 0".to_string(),
            ));
        }

        let rumqttd_config = RumqttdConfig {
            id: 0,
            router: RouterConfig {
                max_connections: 10_000,
                max_outgoing_packet_count: 10_000,
                max_segment_size: 1024 * 1024,
                max_segment_count: 10,
                ..Default::default()
            },
            v4: None,
            v5: None,
            ws: None,
            cluster: None,
            console: None,
            bridge: None,
            prometheus: None,
            metrics: None,
        };
        let broker = Broker::new(rumqttd_config);
        let host_link = broker.link("host").map_err(|e| MessagingError::Internal(e.to_string()))?;
        Ok(Self {
            broker,
            host_link: Mutex::new(host_link),
            cancellation: CancellationToken::new(),
            channel_capacity: config.channel_capacity,
        })
    }

    /// Publishes `payload` on `topic` (already fully-namespaced by the
    /// caller). Backpressure on the host<->router bridge surfaces as
    /// [`MessagingError::Internal`] rather than blocking.
    pub async fn publish(&self, topic: String, payload: Vec<u8>) -> Result<(), MessagingError> {
        let mut host_link = self.host_link.lock().await;
        host_link
            .0
            .try_publish(topic, payload)
            .map_err(|e| MessagingError::Internal(format!("broker publish failed: {e}")))?;
        Ok(())
    }

    /// Publishes a retained message via the raw-packet escape hatch
    /// (`LinkTx::publish`/`try_publish` have no retain parameter). Not
    /// part of the guest-facing WIT surface for Slice 6A (ADR-0010
    /// Finding A4) — used only by this crate's own retained-message test.
    #[cfg(test)]
    pub(crate) async fn publish_retained_for_test(
        &self,
        topic: &str,
        payload: Vec<u8>,
    ) -> Result<(), MessagingError> {
        let mut host_link = self.host_link.lock().await;
        let publish = Publish::new(Bytes::from(topic.to_string()), Bytes::from(payload), true);
        host_link
            .0
            .send(Packet::Publish(publish, None))
            .await
            .map_err(|e| MessagingError::Internal(e.to_string()))?;
        Ok(())
    }

    /// Subscribes to `topic_filter` (already fully-namespaced by the
    /// caller), returning a handle whose `Drop` unsubscribes and a
    /// bounded receiver fed by an internal forwarding task.
    pub async fn subscribe(
        &self,
        topic_filter: String,
    ) -> Result<(SubscriptionHandle, mpsc::Receiver<(String, Vec<u8>)>), MessagingError> {
        let client_id = format!("sub-{}", Uuid::new_v4());
        let (mut link_tx, mut link_rx) =
            self.broker.link(&client_id).map_err(|e| MessagingError::Internal(e.to_string()))?;
        link_tx
            .subscribe(topic_filter.clone())
            .map_err(|e| MessagingError::Internal(e.to_string()))?;

        let (sender, receiver) = mpsc::channel(self.channel_capacity);
        let child_token = self.cancellation.child_token();
        let forward_token = child_token.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = forward_token.cancelled() => break,
                    notification = link_rx.next() => {
                        match notification {
                            Ok(Some(Notification::Forward(forward))) => {
                                let topic = String::from_utf8_lossy(&forward.publish.topic).into_owned();
                                let payload = forward.publish.payload.to_vec();
                                // Best-effort delivery: a subscriber that
                                // can't keep up gets messages dropped past
                                // the bound rather than blocking this
                                // task's draining of `link_rx`, which
                                // would otherwise back-pressure into
                                // rumqttd's own internal per-link buffer.
                                match sender.try_send((topic, payload)) {
                                    Ok(()) => {}
                                    Err(TrySendError::Full(_)) => {
                                        warn!("messaging: dropping message, subscriber channel full");
                                    }
                                    Err(TrySendError::Closed(_)) => break,
                                }
                            }
                            Ok(Some(_)) => continue,
                            Ok(None) | Err(_) => break,
                        }
                    }
                }
            }
        });

        Ok((
            SubscriptionHandle {
                cancellation: child_token,
                link_tx: Some(link_tx),
                topic_filter,
                task: Some(task),
            },
            receiver,
        ))
    }
}

impl Drop for MqttBroker {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

/// Owns one broker subscription. Dropping it unsubscribes from the
/// broker and stops the associated forwarding task.
pub struct SubscriptionHandle {
    cancellation: CancellationToken,
    link_tx: Option<LinkTx>,
    topic_filter: String,
    task: Option<JoinHandle<()>>,
}

impl fmt::Debug for SubscriptionHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SubscriptionHandle").field("topic_filter", &self.topic_filter).finish()
    }
}

impl Drop for SubscriptionHandle {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
        if let Some(mut link_tx) = self.link_tx.take() {
            // Non-blocking: `unsubscribe` is a synchronous, blocking call
            // into rumqttd's internal event channel, which risks stalling
            // whatever Tokio worker thread happens to run this `Drop` if
            // that channel is momentarily full (mirrors the publish/
            // try_publish split already used elsewhere in this file).
            let _ = link_tx.try_unsubscribe(self.topic_filter.clone());
        }
    }
}

/// Type alias used by callers that thread the broker through `Arc`-shared
/// long-lived state (e.g. `AppSandboxEngine`, `ControlPlaneService`).
pub type SharedMqttBroker = Arc<MqttBroker>;
