# M06A Slice A3 — Inbound WebSocket: Implementation Plan

> **Milestone:** [task.md](task.md) · **Slice:** A3 · **Status:** Planned, not started. **Revision 2**, after gap review.
>
> Decision ids are `D-A3-n`. Milestone-level decisions (`D-06A-n`) live in [task.md](task.md) and are inputs here. Slice A2's decisions (`D-A2-n`) live in [slice-a2-implementation-plan.md](slice-a2-implementation-plan.md); A2 is **shipped**, so its decisions are facts on the ground here, not proposals.
>
> **Scope, from task.md's slice table:** "Inbound WebSocket. `Upgrade` detection and the 101 handshake in the HTTP bridge; frame codec via the router's existing `tokio-tungstenite`; a `websocket-handler` interface extending `syneroym:http` with guest exports (`on-open`/`on-message`/`on-close`) plus the host side that owns connection lifetime. Unicast reply by host import; **broadcast reuses the pub-sub broker** rather than inventing a second fan-out (D-06A-2)."

---

## §0 Review response (revision 2)

All 15 findings from the gap review are accepted and folded into this revision.

| # | Finding | Disposition |
|---|---|---|
| **G1** | `AppSandboxInner` does not exist | **Accepted.** Replaced with `AppSandboxEngine` and direct field access. |
| **G2** | Constructor is `init(...)`, not `new(...)` | **Accepted.** §4 updated. |
| **G3** | No undeploy cleanup for `websocket_senders` | **Accepted.** Added `forget_websocket_senders`. Changed map signature to group by `service_id` so dropping the map cleanly aborts all active `rx.recv()` loops. `D-A3-7`. |
| **G4** | Deploy-time export check missing rollback details | **Accepted.** Explicitly specified `rollback_config_generation` and `rollback_fdae_policy` in §4. |
| **G5** | No `("websocket", other)` rejection arm | **Accepted.** Added to `validate_route` changes in §4. |
| **G6** | `topic` required-vs-optional not decided | **Accepted.** Topic is optional; broadcast is skipped if absent, allowing unicast-only routes. `D-A3-9`. |
| **G7** | No non-Wasm service refusal for `websocket` | **Accepted.** Added to `deploy_with_context` in §4. |
| **G8** | A4 depends on `public: true` | **Accepted.** Noted in §4. |
| **G9** | WebSocket handshake mechanism is wrong | **Accepted.** `accept_async` on an upgraded socket hangs because hyper already read the headers. The pseudo-code in §5.1 now uses `tungstenite::handshake::server::create_response_with_body` to generate the correct `Sec-WebSocket-Accept` header for hyper to return, followed by `from_raw_socket`. |
| **G10** | No failure/security matrix rows for WebSocket | **Accepted.** Added §6 Failure and Security Matrix. |
| **G11** | No verification/test plan | **Accepted.** Added §7 Verification Plan. |
| **G12** | No per-service WebSocket connection bound | **Accepted.** Added `guest_websocket_permits` semaphore, matching D-A2-11. `D-A3-8`. |
| **G13** | `bindgen!` direction is correct | **Acknowledged.** |
| **G14** | Doc comment cites milestone IDs | **Accepted.** Removed milestone citation from the code comment. |
| **G15** | `topic_rx.next()` vs. `.recv()` mismatch | **Accepted.** Corrected in pseudo-code. |

---

## §1 Findings from reading the tree

Every line reference was checked against the current working tree (`d63c9dd`, A1 and A2 merged).

### F1 — Component instances are transient per-frame

`AppSandboxEngine` executes guest HTTP handlers by grabbing a fresh instance from the `wasmtime` pool, invoking the export, and dropping the instance immediately ([engine.rs:1010](../../../../crates/sandbox_wasm/src/engine.rs#L1010)). The guest cannot hold state in memory between calls. For WebSockets, this means `on-open`, `on-message`, and `on-close` will each execute in a **new, ephemeral instance**.

### F2 — WebSockets don't consume `wasmtime` pool slots while idle

Because of F1, long-lived WebSockets will not exhaust the global `wasmtime` concurrency pool or the service's `guest_http_permits` semaphore. A permit is only acquired for the milliseconds it takes to process a single frame. The host connection loop in `syneroym-router` holds the TCP socket open asynchronously without occupying wasmtime resources.

### F3 — The guest needs a host import for unicast replies

Since the guest executes transiently, a guest `on-message` handler cannot natively write back to a socket object. It must call a host import (`send(conn, frame)`) with a connection ID to ask the host to send the frame to the active WebSocket. 

### F4 — Host imports require `bindgen!` for maintainability

A2's F6 explicitly avoided adding the `syneroym:http` interface to the `host-environment` world because A2 only needed an *export* (guest function), and `bindgen!` generates dead code for exports, whereas dynamic `Val` marshalling is straightforward. However, A3 introduces a host *import* (`websocket::send`). Implementing dynamic `Val` unwrapping for host imports via `linker.func_wrap` is extremely tedious and error-prone. The existing pattern for host imports is to use `bindgen!` ([host.rs:6](../../../../crates/wit_interfaces/src/host.rs#L6)).

### F5 — Broker subscriptions are already pull-based via SSE

Task.md specifies that WebSocket broadcasts should reuse the pub-sub broker. `handle_messaging_sse` ([http.rs:1144](../../../../crates/router/src/route_handler/http.rs#L1144)) is the exact precedent: it subscribes to the broker using `route.topic` and streams messages to the client. A3 can reuse this logic identically for WebSockets, multiplexing broker messages into the WebSocket stream alongside unicast replies.

---

## §2 Decisions

| # | Decision | Rationale |
|---|---|---|
| **D-A3-1** | **WIT layout: `websocket-handler` and `websocket` interfaces.** Added to `crates/wit_interfaces/wit/http/http.wit`. `websocket-handler` defines guest exports (`on-open`, `on-message`, `on-close`). `websocket` defines the host import (`send`). | Task.md requires this exact structure. The split interface cleanly separates what the guest implements from what the host provides. |
| **D-A3-2** | **Host import via `bindgen!` in a new `http-host` world.** Create `crates/wit_interfaces/src/http.rs` using `bindgen!` on a dummy `world http-host` that only exports `syneroym:http/websocket`. | F4: while A2 avoided `bindgen!` for an export, writing `linker.func_wrap` manually for an import is brittle. A dedicated `bindgen!` invocation generates the `syneroym::http::websocket::Host` trait cleanly, matching the `vault` and `blob-store` patterns. |
| **D-A3-3** | **`WebSocketRegistry` grouped by service.** A global `DashMap<String, Arc<DashMap<String, mpsc::Sender<Vec<u8>>>>>` mapping `service_id` -> `conn_id` -> `Sender`. | F3: The host import `send` runs inside `AppSandboxEngine`'s `HostState`, which needs a way to route the frame back to the active WebSocket. Grouping by `service_id` makes undeploy cleanup trivial (D-A3-7). |
| **D-A3-4** | **Connection IDs are standard UUID v4 strings.** Generated by the router upon successful HTTP upgrade. | Ensures global uniqueness across all concurrent WebSocket connections, preventing cross-talk in the `WebSocketRegistry`. |
| **D-A3-5** | **Broadcast reuses `HttpRoute.topic` natively.** If `route.topic` is provided, the router's connection loop subscribes to the `messaging_broker` and automatically forwards messages to the WebSocket. | F5: This avoids needing a separate `subscribe(conn, topic)` host import, keeping the guest blind to pub-sub mechanics and perfectly mirroring the existing SSE behaviour. |
| **D-A3-6** | **Guest invocations use dynamic `Val` marshalling.** `on-open`, `on-message`, and `on-close` are invoked via `get_wasm_func` and `call_async` exactly like A2's `handle-request`. | Follows D-A2-2's precedent. No `bindgen!` typed accessor is generated for the guest exports. |
| **D-A3-7** | **Undeploy cleanup drops sender maps.** `forget_websocket_senders` removes the `service_id` key from the registry. | Dropping the map drops the `Sender`s for that service. This safely causes the `rx.recv()` in the router's connection loops to return `None`, unblocking them to perform a clean teardown of the WebSocket without hanging tasks. |
| **D-A3-8** | **Concurrency bounded per service.** Added `guest_websocket_permits: Arc<DashMap<String, Arc<Semaphore>>>` to `AppSandboxEngine`, sized by `max_concurrent_websockets_per_service` (default 50) on `AppSandboxRole`. | Protects the node from connection exhaustion (FD limits/memory) caused by a single service, mirroring D-A2-11. |
| **D-A3-9** | **Topic is optional.** If `route.topic` is missing, the connection operates as unicast-only. | Allows flexibility for routes that don't need pub-sub fan-out. |
| **D-A3-10** | **Explicit `FrameKind` typing (`text`, `binary`).** Added `websocket-types` interface in WIT with `enum frame-kind { text, binary }`. Guest and host handlers specify `kind: frame-kind` to preserve WebSocket opcode fidelity (`0x81` vs `0x82`). | Browser and standard WebSocket clients distinguish text frames from binary frames; preserving opcode fidelity avoids client deserialization failures. |
| **D-A3-11** | **Caller identity forwarding.** Forward `CallerContext` through `handle_websocket_on_open/message/close`. | Allows guest handlers to inspect authenticated session identity. |
| **D-A3-12** | **Sequential lifecycle dispatch.** `on-open` is awaited before entering the frame reader loop; `on-close` is awaited after the stream terminates. | Prevents `on-message` frames from racing ahead of initialization logic in `on-open`. |

---

## §3 Exact type and signature changes

### 3.1 WIT Interfaces (`crates/wit_interfaces/wit/http/http.wit`)

Append the following to the existing file:

```wit
interface websocket-types {
    enum frame-kind {
        text,
        binary,
    }
}

/// Optional WebSocket lifecycle handler (M06A A3). A component only exports
/// this when it declares an `http_routes` entry with `target = "websocket"`.
interface websocket-handler {
    use websocket-types.{frame-kind};

    /// Invoked when a WebSocket connection is successfully upgraded.
    on-open: func(conn: string);
    
    /// Invoked for every binary or text frame received from the client.
    on-message: func(conn: string, frame: list<u8>, kind: frame-kind);
    
    /// Invoked when the connection drops, cleanly or otherwise.
    on-close: func(conn: string);
}

/// Host-provided WebSocket capabilities.
interface websocket {
    use websocket-types.{frame-kind};

    /// Unicast a frame to a specific active connection.
    send: func(conn: string, frame: list<u8>, kind: frame-kind) -> result<_, string>;
}

/// Dummy world for generating the host import bindings.
world http-host {
    export websocket;
}
```

### 3.2 Host Bindings (`crates/wit_interfaces/src/http.rs`)

New module added to `crates/wit_interfaces/src/lib.rs`:

```rust
#[cfg(not(target_arch = "wasm32"))]
pub mod http;
```

```rust
// crates/wit_interfaces/src/http.rs
wasmtime::component::bindgen!({
    path: "wit/http",
    world: "http-host",
    additional_derives: [serde::Serialize, serde::Deserialize],
    imports: { default: async },
    exports: { default: async },
});
```

### 3.3 Core Types (`crates/core/src/http_routes.rs`)

Update the doc comment on `HttpRoute.public`:
```rust
    /// Whether a caller with no verified identity may reach this route.
    /// Meaningful for `target = "guest"` and `target = "websocket"`, 
    /// where `false` -- the default -- answers an anonymous request with 401 
    /// before the component is instantiated. Refused at deploy on any other target.
    #[serde(default)]
    pub public: bool,
```

### 3.4 Sandbox Engine (`crates/sandbox_wasm/src/engine.rs`)

Add fields to `AppSandboxEngine`:
```rust
    /// Registry for routing guest unicast sends back to active WebSocket connections (M06A A3).
    /// Grouped by service_id to allow clean bulk teardown on undeploy.
    websocket_senders: Arc<DashMap<String, Arc<DashMap<String, tokio::sync::mpsc::Sender<Vec<u8>>>>>>,
    
    /// Pool slots this node will let active WebSocket connections hold concurrently.
    guest_websocket_permits: Arc<DashMap<String, Arc<tokio::sync::Semaphore>>>,
    max_concurrent_websockets_per_service: u32,
```

Implement the host import trait for `HostState`:
```rust
#[async_trait::async_trait]
impl syneroym_wit_interfaces::http::syneroym::http::websocket::Host for HostState {
    async fn send(&mut self, conn: String, frame: Vec<u8>) -> wasmtime::Result<Result<(), String>> {
        if let Some(service_map) = self.engine.websocket_senders.get(&self.service_id) {
            if let Some(sender) = service_map.get(&conn) {
                match sender.send(frame).await {
                    Ok(_) => return Ok(Ok(())),
                    Err(_) => return Ok(Err("Connection closed".to_string())),
                }
            }
        }
        Ok(Err("Unknown connection ID".to_string()))
    }
}
```

Add teardown method:
```rust
    /// Drops all WebSocket senders for a service (M06A A3).
    /// Called from undeploy. Dropping the senders unblocks any active rx.recv() 
    /// loops in the router, terminating the WebSockets cleanly.
    pub fn forget_websocket_senders(&self, service_id: &str) {
        self.websocket_senders.remove(service_id);
        self.guest_websocket_permits.remove(service_id);
    }
```

---

## §4 Call sites that need updating

| File | Line Target | Change |
|---|---|---|
| `crates/sandbox_wasm/src/engine.rs` | `fn build_wasm_linker` | Add `syneroym_wit_interfaces::http::syneroym::http::websocket::add_to_linker(&mut linker, \|state\| state)?;` |
| `crates/sandbox_wasm/src/engine.rs` | `pub async fn init(config: &SubstrateConfig, ...)` | Initialize `websocket_senders`, `guest_websocket_permits`, and `max_concurrent_websockets_per_service`. |
| `crates/sandbox_wasm/src/engine.rs` | Beside `exports_http_handler` | Add `pub fn exports_websocket_handler(&self, service_id: &str) -> bool` looking up `"syneroym:http/websocket-handler@0.1.0"` |
| `crates/control_plane/src/http_routes.rs` | `fn validate_route` (~L82) | Allow `("websocket", "handle-upgrade")`. Add explicit rejection: `("websocket", other) => Err("...unsupported operation...")`. Update L94 to `if route.public && (route.target != "guest" && route.target != "websocket")`. Note: A4 depends on this `public: true` allowance. |
| `crates/control_plane/src/service/orchestration.rs` | `fn deploy_wasm_service` (~L809) | Mirror the `exports_http_handler` check: if any route has `target == "websocket"`, ensure `exports_websocket_handler` is true. If false, run `rollback_config_generation` and `rollback_fdae_policy` and return Err. |
| `crates/control_plane/src/service/orchestration.rs` | `fn deploy_with_context` (~L1653) | Refuse `websocket` target for `Tcp` and `Container` services, mirroring the existing check for `guest`. |
| `crates/control_plane/src/service/orchestration.rs` | `fn stop_wasm` / `undeploy_local` (~L2554) | Call `forget_websocket_senders(service_id)` and `forget_guest_http_permits(service_id)`. |
| `crates/router/src/route_handler/http.rs` | `fn dispatch_route` (~L979) | Add `"websocket" => self.handle_websocket_route(route, req).await` to the match arm. Server loop uses `AutoBuilder::new(TokioExecutor::new()).serve_connection_with_upgrades` to drive upgrades. |
| `crates/core/src/config.rs` | `AppSandboxRole` | Add `max_concurrent_websockets_per_service: u32` (default 50). |
| `test-components/websocket-guest-test` | New fixture | Test component exporting `websocket-handler` and importing `websocket` for echo tests. |

---

## §5 Pseudo-code for non-trivial logic

### 5.1 Router connection loop (`handle_websocket_route`)

```rust
async fn handle_websocket_route(...) -> Result<Response<HttpBody>> {
    // 1. Authentication (Mirror D-A2-7)
    if self.caller.is_none() && !route.public {
        return Ok(unauthenticated_rpc_response());
    }

    // 2. Upgrade check & Handshake
    if !hyper_upgrade_requested(req) {
        return Ok(http_error(400, "Upgrade required"));
    }

    let service_id = self.preamble.service_id.clone();
    let engine = self.route_handler.inner.app_sandbox_engine.clone();

    // 3. Admission Control (D-A3-8)
    let permit = engine.guest_websocket_permits
        .entry(service_id.clone())
        .or_insert_with(|| Arc::new(Semaphore::new(engine.max_concurrent_websockets_per_service)))
        .clone()
        .acquire_owned()
        .await;
    
    if permit.is_err() {
        return Ok(http_error(503, "Too many WebSocket connections"));
    }

    // 4. Setup messaging broker subscription (Mirror handle_messaging_sse)
    let mut topic_rx = if let Some(topic) = &route.topic {
        let namespaced = namespace_topic(&service_id, topic);
        Some(self.route_handler.inner.messaging_broker.subscribe(namespaced).await?)
    } else {
        None
    };

    // 5. Generate 101 Response
    let handshake_res = match tungstenite::handshake::server::create_response_with_body(&req, || HttpBody::empty()) {
        Ok(res) => res,
        Err(e) => return Ok(http_error(400, format!("WebSocket handshake failed: {e}"))),
    };

    let (mut response, upgrade_fut) = hyper::upgrade::on(req);
    // (Copy status and headers from handshake_res to response)
    
    tokio::spawn(async move {
        // Hold permit for the lifetime of the connection
        let _permit = permit;
        
        let upgraded = upgrade_fut.await.unwrap();
        let mut ws_stream = tokio_tungstenite::WebSocketStream::from_raw_socket(
            upgraded, 
            tokio_tungstenite::tungstenite::protocol::Role::Server, 
            None
        ).await;
        
        let conn_id = uuid::Uuid::new_v4().to_string();
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(32);
        
        // Register for unicast sends
        engine.websocket_senders
            .entry(service_id.clone())
            .or_default()
            .insert(conn_id.clone(), tx);
        
        // Notify guest
        engine.handle_websocket_on_open(&service_id, &conn_id).await;
        
        loop {
            tokio::select! {
                // Outbound: from Guest (Unicast)
                // If the service is undeployed, `forget_websocket_senders` drops the Sender
                // causing this to return `None`, safely exiting the loop.
                msg = rx.recv() => {
                    match msg {
                        Some(frame) => {
                            let _ = ws_stream.send(tungstenite::Message::Binary(frame)).await;
                        }
                        None => break, // Service undeployed
                    }
                },
                // Outbound: from Broker (Broadcast)
                Some(broker_msg) = async {
                    if let Some(rx) = &mut topic_rx { rx.recv().await } else { futures::future::pending().await }
                } => {
                    let _ = ws_stream.send(tungstenite::Message::Binary(broker_msg.payload)).await;
                },
                // Inbound: from Client
                msg = ws_stream.next() => {
                    match msg {
                        Some(Ok(msg)) => {
                            let payload = msg.into_data();
                            engine.handle_websocket_on_message(&service_id, &conn_id, payload).await;
                        },
                        _ => break, // Disconnect or error
                    }
                }
            }
        }
        
        // Cleanup
        if let Some(service_map) = engine.websocket_senders.get(&service_id) {
            service_map.remove(&conn_id);
        }
        engine.handle_websocket_on_close(&service_id, &conn_id).await;
    });

    Ok(response)
}
```

### 5.2 Dynamic `Val` marshalling for guest invocations

In `crates/sandbox_wasm/src/engine.rs`:

```rust
pub async fn handle_websocket_on_message(&self, service_id: &str, conn_id: &str, frame: Vec<u8>) {
    // Boilerplate setup mimicking handle_guest_http_request
    let (mut store, instance) = self.build_store_and_instantiate(...).await?;
    let func = self.get_wasm_func(&mut store, instance, "syneroym:http/websocket-handler@0.1.0", "on-message")?;
    
    // Marshalling
    let conn_val = Val::String(conn_id.to_string());
    let frame_val = crate::stream::bytes_to_val_list(&frame);
    
    // Execute (we ignore traps here since WebSockets shouldn't disconnect on guest panic, see §6)
    let _ = func.call_async(&mut store, &[conn_val, frame_val], &mut []).await;
}
```

---

## §6 Failure and Security Matrix

| Scenario | Behaviour | Mechanism |
|---|---|---|
| **Guest `on-message` trap / timeout** | Error logged, WebSocket stays open. | F1: Because each invocation uses a transient `wasmtime` instance, a guest trap (e.g., out of bounds) or an epoch timeout (`dispatch_epoch_ticks`) only aborts that specific handler execution. The WebSocket connection loop in the router continues unaffected. |
| **Oversized frame from client** | Connection aborted by host. | Enforced by `tokio_tungstenite`'s default frame size caps (`max_message_size` = 64 MiB, `max_frame_size` = 16 MiB). This prevents unbounded memory allocation before the guest even sees the frame. |
| **Concurrent connection exhaustion** | 503 Service Unavailable | `guest_websocket_permits` semaphore (D-A3-8) limits active connections per service. New upgrade requests failing to acquire a permit get a fast 503 instead of draining host resources. |
| **Half-open connections** | Dropped by TCP timeout / broker disconnect | `tokio_tungstenite` stream reads will fail or return `None` when the underlying TCP connection times out or the client disconnects, triggering loop exit and clean up. |
| **Service undeployed** | All active WebSockets cleanly aborted. | `forget_websocket_senders` drops the maps, causing `rx.recv()` in all connection loops to return `None`, cleanly tearing down the WebSockets. |

---

## §7 Verification Plan

### 7.1 Unit Tests
- **Sandbox Engine:** Add unit tests in `crates/sandbox_wasm` verifying `websocket-handler` dynamic `Val` marshalling (`on-open`, `on-message`, `on-close`), and verifying the `syneroym::http::websocket::Host` implementation accurately routes `send` frames to the `websocket_senders` map.
- **Control Plane:** Add tests in `crates/control_plane/src/http_routes.rs` asserting `validate_route` allows `websocket` targets, permits `public: true`, and correctly rejects unsupported operations like `upgrade`. Add tests in `orchestration.rs` asserting deploy fails if the `websocket-handler` export is missing.

### 7.2 Integration Tests
- **Echo (Unicast):** Add a native router test simulating an `Upgrade` request and asserting that frames sent by the client are echoed back by a fixture component via the `send` host import.
- **Broadcast (Pub-Sub):** Add an integration test simulating a WebSocket connection with a defined `topic`, triggering a `messaging.publish`, and asserting the frame arrives on the WebSocket stream.

### 7.3 End-to-End Tests
- **WebRTC Proxy:** M06A A5 (Playwright suite) will execute an E2E test verifying that a browser can establish a WebSocket over the WebRTC `peer-proxy.js` shim, validating that the exact `Sec-WebSocket-Accept` headers generated by the plan satisfy the browser's upgrade constraints.
