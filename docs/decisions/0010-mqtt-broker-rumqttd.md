# ADR-0010: Embedded MQTT Broker via `rumqttd` (D-03-05)

## Status

Accepted (Amendment 1, 2026-07-08: `syneroym:pubsub` renamed to
`syneroym:messaging`, folded into a broader messaging boundary that also
covers bidirectional streaming, both guest-as-source and guest-as-sink.
Amendment 2, 2026-07-08: decentralized QUIC overlay rescheduled from M5 to
M7, alongside DB and blob replication as a shared primitive. See both
amendments below.)

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

## Amendment 1 (2026-07-08): Renamed to `syneroym:messaging`, Split Across M3B/M3C

### Context

Planning for M3B Slice 6 identified that generic bidirectional streaming
(point-to-point, IoC-style: guest registers a stream protocol, host pushes a
peer-initiated request in, guest hands back a stateful iterator resource for
the host to pull from) is a closely related but separate capability from MQTT
pub/sub, and shares the same underlying design principle — guest stays
synchronous, host owns all async orchestration. Rather than a second,
unrelated WIT package, the two are unified under one `syneroym:messaging`
package so the WIT boundary doesn't need a breaking rename later. See
[system-architecture.md §2](../system-architecture.md) and
[system-requirements-spec.md `[PLT-DAP-06]`](../system-requirements-spec.md).

### Decision

- The WIT package `syneroym:pubsub@0.1.0` described above is renamed
  `syneroym:messaging@0.1.0`, split into three interfaces:
  `host-api` (guest-triggered: `publish`, `subscribe`,
  `register-stream-protocol`), `stream-types` (the `stream-cursor` resource
  for guest-as-source streaming, and the `stream-sink` resource for
  guest-as-sink streaming), and `guest-api` (host-triggered:
  `handle-message`, `handle-stream-request`, `accept-stream-upload`).
- The guest-exported push function for pub/sub delivery is renamed
  `on-message` → `guest-api::handle-message`. Behavior (optional export;
  silently discarded if absent) is unchanged.
- **Sequencing:** all decisions and consequences recorded above for the
  pub/sub half remain in force and are implemented in **M3B (Slice 6A)**
  unchanged in substance. The `stream-types`/`register-stream-protocol`/
  `handle-stream-request` surface is declared in the same WIT file (for
  interface stability) but is implemented separately in **M3C (Slice 6B)**,
  which requires new host-side QUIC stream acceptance/routing infrastructure
  not covered by this ADR — see the M3C dependency gate in
  `docs/planning/milestones/M03-sss/task.md`, which requires its own
  design note or ADR before Slice 6B implementation begins.
- This amendment is a rename and scope-boundary clarification only; it does
  not change any decision content above (broker deployment, delivery model,
  topic namespacing, backpressure, or cancellation).

## Amendment 2 (2026-07-08): Decentralized Overlay Rescheduled from M5 to M7

### Context

The decentralized QUIC log-replication overlay for pub/sub (mentioned above
and in the original M3 planning as "pending full QUIC P2P overlay in M5")
was re-examined alongside `[PLT-RED]` Service Redundancy planning. It was
recognized as the same underlying problem as SQLite WAL replication — a
peer node pulling and applying an ordered, checksummed log over Iroh QUIC —
just with a different payload (MQTT topic-log entries instead of WAL
frames). Blob replication for non-S3 deployments was identified as a third
instance of the same pattern (simplified by content-addressing).

### Decision

- The decentralized pub/sub overlay is **rescheduled from Milestone 5 to
  Milestone 7**, to be implemented alongside SQLite WAL replication and
  peer-to-peer blob replication under one `[PLT-RED]` effort, sharing the
  same Iroh multiplexed-stream transport and ordered/checksummed
  frame-shipping primitive. See
  [meta-implementation-plan.md, Milestone 7](../planning/meta-implementation-plan.md).
- **Correction (this amendment, not a later one):** the overlay is purely a
  redundancy/failover feature for the broker's own topic-log state — making
  it survive the loss of the node that hosts it — exactly parallel to WAL
  replication for a service's DB. It is **not** a prerequisite for
  cross-node pub/sub to function. A `publish`/`subscribe` call from a
  different physical node than the target service's broker is routed there
  via the same RPC/native-dispatch path used for any cross-node
  host-function call (JSON-RPC bridge today, per Slice 5's native dispatch
  registry; wRPC once the Universal Proxy ships in M4) — identical to how a
  cross-node `data-layer` call already reaches whichever node holds the
  target service's SQLite file. Moving the log-replication overlay to M7
  therefore has no bearing on basic cross-node pub/sub delivery, which does
  not wait on it.
- `[PLT-RED]`'s "Blob Storage Redundancy" scope is correspondingly widened:
  previously blob redundancy was delegated entirely to the S3-compatible
  backend; it now also covers Syneroym-built peer-to-peer blob replication
  for deployments with no S3-compatible backend configured. See
  [system-requirements-spec.md `[PLT-RED]`](../system-requirements-spec.md).
- No change to the M3B/M3C WIT surface, delivery model, or any other
  decision content in this ADR — this amendment only moves *when* the
  decentralized overlay ships and *why* it now ships alongside DB/blob
  replication instead of in M5.
