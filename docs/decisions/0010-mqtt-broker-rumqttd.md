# ADR-0010: Embedded MQTT Broker via `rumqttd` (D-03-05)

## Status

Accepted

## Context

`[PLT-DAT]` requires a pub/sub event service for asynchronous coordination
between `SynSvcs`. The architecture specifies an embedded MQTT broker running
inside the substrate process. WASM guest components interact with it via a WIT
host function boundary.

Design questions:
1. Deployment model: in-process Tokio task vs. sidecar subprocess.
2. Subscription delivery: async stream returned to guest vs. push-model host
   invocation of a guest-exported handler.
3. Feature scope: wildcard topics, retained messages, cross-service pub/sub.
4. Backpressure between the Wasmtime host and `rumqttd`.

## Decision

### Broker Deployment

**In-process Tokio task** (Option A).

`rumqttd` is started as a background Tokio task within the substrate binary.
A channel bridge connects the Wasmtime host functions to `rumqttd`'s internal
router. This avoids subprocess lifecycle management, process supervision, and
IPC overhead.

### Subscription Delivery Model

**Push model.** The host does not return an async stream to the WASM guest
(WASI async streams are not yet stable across all runtimes). Instead:

1. When a guest calls `subscribe(topic)`, the host registers a subscription in
   the broker and associates it with the `service_id`.
2. When `rumqttd` delivers a matching message, the host invokes the component's
   exported `on-message(topic: string, payload: list<u8>)` function through the
   normal Wasmtime invocation path.
3. If the component does not export `on-message`, the subscription is registered
   but messages are silently discarded (the subscribe call still succeeds).

### Feature Scope (M3B)

The following are **in scope** for M3B:
- MQTT `+` (single-level) and `#` (multi-level) wildcard subscriptions.
- Retained messages: delivered to new subscribers joining after a retained
  publish.
- Cross-service pub/sub: a service can subscribe to topics published by another
  service, subject to topic namespace isolation (see below).

### Topic Namespace Isolation

Each service's topics are namespaced by the substrate under
`svc/<service_id>/<user-topic>`. A guest that calls `publish("orders/new", ...)`
actually publishes to `svc/<service_id>/orders/new`. A guest that calls
`subscribe("orders/new")` subscribes to `svc/<service_id>/orders/new`.

Cross-service subscriptions require the subscribing service to explicitly use
the fully qualified topic: `subscribe("svc/<other_service_id>/orders/new")`.
The host allows this (cross-service pub/sub is in scope) but the subscribing
guest must opt in by naming the target service ID explicitly.

This prevents accidental cross-service message leakage while permitting
intentional cross-service event flows.

### WIT Interface

```wit
interface pubsub {
    variant pubsub-error {
        permission-denied,
        internal(string),
    }

    publish:     func(topic: string, payload: list<u8>) -> result<_, pubsub-error>;
    subscribe:   func(topic: string) -> result<_, pubsub-error>;
    unsubscribe: func(topic: string) -> result<_, pubsub-error>;
}
```

Guest export (optional; host invokes on message delivery):
```wit
// Guest component exports this if it wants push-model delivery:
// on-message: func(topic: string, payload: list<u8>);
```

### Backpressure and Channel Bounds

The channel between the Wasmtime host and the `rumqttd` internal router must be
**bounded**. The default channel capacity is **1024 messages** per service.
When the channel is full, `publish` returns
`pubsub-error::internal("broker channel full: backpressure")` rather than
blocking the Wasmtime execution thread.

Services that need to publish at high frequency must implement their own local
buffering or reduce publish rate. The bound is configurable per-substrate:

```toml
[mqtt]
bind_addr         = "127.0.0.1:1883"
channel_capacity  = 1024   # messages in flight between host and rumqttd
```

### Cancellation and Lifecycle

The `MqttBroker` struct holds a `CancellationToken` (from `tokio-util`). When
the substrate shuts down, the token is cancelled and the background `rumqttd`
Tokio task terminates cleanly. This applies the lesson from the M2 epoch timer
audit finding: all background tasks must have explicit cancellation paths and
must not outlive the owning struct.

## Consequences

- `rumqttd` (latest stable version at M3B implementation time) and `tokio-util`
  (already likely in the dependency graph via other crates; confirm) are added
  to `Cargo.toml` workspace dependencies.
- `crates/mqtt-broker/` is a new crate.
- The substrate's management lifecycle must call `MqttBroker::shutdown()` (or
  drop the struct) as part of the graceful shutdown sequence.
- The `on-message` push invocation happens on a Tokio task separate from the
  `rumqttd` delivery task to avoid blocking the broker's internal loop.
- Tests must verify that the `CancellationToken` successfully terminates the
  broker task within 1 second of cancellation (no task leak).
- `rumqttd` version must be pinned in `Cargo.toml` at M3B implementation time
  and the pinning rationale documented as a comment, following the pattern
  established for `iroh` in the workspace.
