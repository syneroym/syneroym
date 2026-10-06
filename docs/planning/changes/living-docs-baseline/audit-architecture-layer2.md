# Audit: architecture doc, "Layer 2 — Substrate Runtime"

Reader: a developer who will split and fix [system-architecture.md](../../../system-architecture.md).

This is a pilot audit. It checks lines 233-373 of the architecture doc against
the code. It changes no code and does not edit the doc. Part of the
[living docs baseline](change.md).

**How to read the verdicts.** Only code and tests count as proof. Paths are
from the repository root. Every MATCHES row has a `file:line` citation.

- **MATCHES**: the code does this.
- **DIVERGES**: the code does something different. The code is right, so the doc needs a fix.
- **NOT BUILT**: nothing in the code does this. "Presented as" says whether the doc writes it as working or as planned.
- **STALE**: the name no longer exists. The current name is given.
- **UNCLEAR**: I could not decide. The row says what I looked at.

All checks are static reading. I did not run the substrate or the tests.

## 1. Summary

| Verdict | Claims |
| --- | --- |
| MATCHES | 20 |
| DIVERGES | 15 |
| NOT BUILT | 18 |
| STALE | 3 |
| UNCLEAR | 5 |
| **Total** | **61** |

Main findings:

1. **Almost all NOT BUILT claims are presented as working.** The section has no "planned" marker. Only the warning at doc lines 5-6 mentions wRPC.
2. **The Layer 2 diagram shows three "Shared Utilities"** (matching, reputation, payment). None exists as a substrate component.
3. **Replication and backup are the largest gap.** Litestream, the backup pool, hot standby and "replica until an operator promotes it" do not exist. The doc also contradicts itself: the later `[PLT-RED]` design says it will *not* use Litestream.
4. **The write-arbitration table describes a design, not the code.** Two rows match (messages, booking slot). Order state and catalog items are different in the code. The reputation row has no code.
5. **Placement is explicit, not scheduled.** The operator names the target substrate. Nothing schedules by `cpu`, `memory`, `gpu` or locality.
6. **Real parts missing from the diagram:** client gateway, community registry, coordinator, auth service, app supervisor, observability, embedded MQTT broker and the conversation host.

## 2. Claims

Doc line is the line in `docs/system-architecture.md`.

### 2.1 Substrate internal architecture (diagram, lines 237-289)

| ID | Doc line | Claim | Verdict | Evidence | Proposed fix |
| --- | --- | --- | --- | --- | --- |
| L2-01 | 239 | "SYN-SUBSTRATE (Rust / Tokio)" | MATCHES | `crates/substrate/src/main.rs:178` builds a multi-thread Tokio runtime and runs the substrate on it. | None. |
| L2-02 | 243 | Ingress: "JSON-RPC over WebSocket for edge of WASM, say Browsers, CLI" | DIVERGES | Edge JSON-RPC arrives as HTTP/1.1 (`crates/router/src/route_handler/http.rs:175`; the client gateway forwards the raw bytes, `crates/client_gateway/src/gateway.rs:477-535`) or as `json-rpc://` framed streams (`crates/router/src/preamble.rs:15-20`). WebSocket exists only as an opt-in route target a guest declares (`target = "websocket"`), and the guest receives raw frames: `crates/router/src/route_handler/http/websocket.rs:105-145`, `crates/wit_interfaces/wit/http/http.wit:127-139`. | Write: JSON-RPC 2.0 over HTTP and over framed Iroh/WebRTC streams. Add: WebSocket is an optional per-app route that carries app-defined frames. |
| L2-03 | 244 | Ingress: "wRPC — WASM component-component local or network calls" | NOT BUILT | `crates/router/src/lib.rs:4` ("not yet implemented"); `crates/router/src/preamble.rs:19,28-29`; `crates/router/src/route_handler/dispatch.rs:282-285` (answers "not implemented yet"); `dispatch.rs:353` (typed unsupported-protocol answer). Presented as: working (diagram node, no label). Today components call each other through the Universal Proxy with JSON-RPC (`crates/router/src/proxy.rs:1-6`). | Remove the node, or label it `planned`. Show the Universal Proxy instead. |
| L2-04 | 245 | Ingress: "Iroh QUIC Endpoint" | MATCHES | `crates/router/src/connection_router.rs:48,154-155` (ALPN `syneroym/0.1`, Iroh router accepts it). | None. |
| L2-05 | 246 | Ingress: "WebRTC Data Channel" | MATCHES | `crates/router/src/net_webrtc.rs:26` (`WebRTCStream` wraps a data channel); `crates/router/src/connection_router.rs:180-215` (signaling client, streams go to the route handler). | None. |
| L2-06 | 250 | Core: "Key Manager Ed25519 + Delegation" | MATCHES | Ed25519 identity: `crates/identity/src/keys.rs:16,91`. Delegation: `crates/identity/src/delegation.rs:62-74`. Data keys: `crates/data_keystore/src/key_store.rs:29,119,152,197`. Per-app master keys: `crates/app_supervisor/src/keys.rs:300`. | Say there is no single "Key Manager". Three pieces do the work: identity, key store (KEK/DEK), supervisor key vault. |
| L2-07 | 251, 275-276 | Core: "Access Control Engine"; ingress goes through it before the core | DIVERGES | There is no single gate. Identity is checked when the stream opens (`crates/router/src/route_handler/io.rs:393`, `build_caller` at `io.rs:158`). Native services admit callers one method at a time (`crates/roym_core/src/admit.rs:20,56`). Row-level policy (FDAE) is compiled into the data layer (`crates/fdae/src/lib.rs:1-3`, `crates/data_db/src/auth.rs:1-2`). `io.rs:150-158` names the tiers. | Describe three layers: stream identity, per-service admission, per-row policy. |
| L2-08 | 252 | Core: "Message Router" | STALE | The component is `ConnectionRouter` in `syneroym-router` (`crates/router/src/connection_router.rs:54`, `crates/router/src/lib.rs:1-5`). The name "message router" is easy to confuse with the embedded MQTT broker (`crates/mqtt_broker/src/lib.rs:1`). | Rename to "connection router". |
| L2-09 | 253 | Core: "Service Orchestrator Deploy / Lifecycle" | MATCHES | `ControlPlaneService` deploys and undeploys on the node (`crates/control_plane/src/service.rs:70-74`, `crates/control_plane/src/service/orchestration/lifecycle.rs:249`). The supervisor role reconciles desired state (`crates/app_supervisor/src/lib.rs:1-8`). The plan is compiled on the client side (`crates/app_orchestration/src/lib.rs:1-3`). | Describe the three parts: control plane service, supervisor, client-side compiler. |
| L2-10 | 257 | Sandbox: "Wasmtime WASM Component Runtime" | MATCHES | `Cargo.toml:197` (wasmtime, component-model); limits for memory, fuel and epoch time: `crates/sandbox_wasm/src/engine.rs:84,131,232`. | None. |
| L2-11 | 258 | Sandbox: "Podman Rootless OCI Container Runtime" | UNCLEAR | The engine runs the host `podman` command: `podman run -d --name … --network bridge` (`crates/sandbox_podman/src/engine.rs:238-275`). It sets no rootless option and does not check the mode. The developer guide says "rootless by default" (`docs/developer-guide.md:561`). I found no code that enforces it. | Decide: say "uses the host's Podman; rootless when the substrate user is not root", or add a check. See question Q7. |
| L2-12 | 262 | Storage: "SQLite (encrypted) Store" | MATCHES | `Cargo.toml:144` (`bundled-sqlcipher`); key set with `PRAGMA key`: `crates/data_db/src/sqlite/provider.rs:281-285`. Encryption can be turned off by config; default is on: `crates/core/src/config/base.rs:36,48`. | Add: encryption is a config switch. |
| L2-13 | 262 | Diagram node id `CRSQL` | STALE | No cr-sqlite in the code (search for `crsql`/`cr-sqlite` finds nothing in `crates/` or `Cargo.toml`). | Rename the node id. |
| L2-14 | 263 | Storage: "Offline Outbox Queue SQLite + Tokio channel" | DIVERGES | The queue is SQLite (`crates/async_queue/src/queue.rs:71-94`). A worker polls it on a timer, not a channel (`crates/router/src/proxy/outbox_forwarding.rs:190`). It is a node-side queue for service-to-service calls and belongs to one process (`crates/async_queue/src/lib.rs:1-14`, `crates/router/src/proxy_outbox.rs:1-10`). | Say "durable SQLite outbox, polled by a worker". Do not call it an offline client queue. |
| L2-15 | 264 | Storage: "Content-addressed Blob Store" | MATCHES | `crates/data_blob/src/lib.rs:1`; hash of the plaintext and dedup: `crates/data_blob/src/object_store_impl/upload_session.rs:80-83`; encrypted: `crates/data_blob/src/crypto.rs:2-4`. | None. |
| L2-16 | 265, 280 | Storage: "Litestream WAL Replication" | NOT BUILT | No Litestream anywhere in code, config or `mise.toml`. Presented as: working. The doc contradicts itself: `[PLT-RED]` (line 2040) says it will not use Litestream. | Remove from the diagram. Move to a `planned` file with the `[PLT-RED]` design. See Q1. |
| L2-17 | 281 | "Backup Store S3-compatible / peer" for streamed WAL | NOT BUILT | No WAL backup target. S3 exists only as an optional blob backend (`crates/substrate/Cargo.toml` feature `aws`, `crates/data_blob/src/object_store_impl.rs`). Presented as: working. | Mention only the S3 blob backend. |
| L2-18 | 269 | Utilities: "Matching Fabric Client" | NOT BUILT | No such component. The nearest thing is the Roym `directory` service (`crates/roym_core/app/roym.toml`, service `directory`). Presented as: working. | Remove from Layer 2. Describe discovery under the Roym app. See Q2. |
| L2-19 | 270 | Utilities: "Reputation Engine" | NOT BUILT | No engine. The signed record types contain no reputation record (`crates/roym_core/src/record.rs:18-31`). Presented as: working. | Remove from Layer 2, or mark `planned`. |
| L2-20 | 271 | Utilities: "Payment Adapter" | NOT BUILT | No adapter to a payment provider. Roym only records what each side says: `crates/roym_core/src/payment.rs:1-2`, and states it "does not see the money move" (`crates/roym_core/src/booking.rs:26`). Presented as: working. | Remove from Layer 2. Describe payment records under the Roym app. |
| L2-21 | 275-279 | Flow: ingress → access control → core → sandbox / storage / utilities | DIVERGES | The router plans a pipeline per stream (`crates/router/src/route_handler/dispatch.rs:314-353`) and calls a native service, a WASM guest or a TCP proxy. Guests reach storage through host capabilities (`crates/sandbox_wasm/src/host_capabilities.rs`), not through a "core" box. | Redraw as: stream → identity → pipeline → service; guest → host capability → storage. |
| L2-22 | 238-289 | The diagram shows the substrate's parts | DIVERGES | These parts exist but are missing: client gateway (`crates/client_gateway/src/gateway.rs:73`), community registry, coordinator, auth, supervisor, observability, roym (`crates/core/src/config/roles.rs:9-21`, `crates/substrate/src/runtime/services.rs:29-36`), embedded MQTT broker (`crates/mqtt_broker/src/lib.rs:1`), conversation host (`crates/conversation/src/lib.rs:1-8`). | Add a roles table. Each role is toggled by `SubstrateConfig.roles` and Cargo features. |

### 2.2 SynApp packaging and API pipeline (lines 291-321)

| ID | Doc line | Claim | Verdict | Evidence | Proposed fix |
| --- | --- | --- | --- | --- | --- |
| L2-23 | 297-300, 305-306 | WIT → `wit-bindgen` → Rust source | MATCHES | `Cargo.toml:199` (wit-bindgen); WIT under `crates/wit_interfaces/wit/` and e.g. `crates/roym_web/wit/world.wit`; generated `crates/roym_web/src/bindings.rs`. | None. |
| L2-24 | 307 | Build with `cargo component build` | MATCHES | `mise.toml:10` (tool), `mise.toml:58,76` (`cargo component build --release --target wasm32-wasip2`). | Add the target `wasm32-wasip2`. |
| L2-25 | 301, 308 | App Spec `.toml` manifest | MATCHES | `crates/app_orchestration/src/models/manifest.rs:14-16` (`SynAppManifest`); example `crates/roym_core/app/roym.toml`. | None. |
| L2-26 | 302, 309 | Manifest is deployed to the orchestrator | MATCHES | `roymctl app deploy` (`apps/roymctl/src/commands/app.rs:129-131`); plan applied to each substrate by `crates/sdk/src/deploy/actor.rs` (tested in `crates/sdk/src/deploy/tests.rs:171`). | None. |
| L2-27 | 303, 310 | JSON-RPC 2.0 is derived automatically from WIT; wRPC planned | MATCHES | Type-directed JSON ⇄ WIT conversion at the component boundary: `crates/sandbox_wasm/src/conversions.rs:1-30`; used by the JSON-RPC-to-WASM stage (`crates/router/src/route_handler/dispatch.rs:227,331-334`). wRPC is not built (see L2-03), so "planned" is right. | None. |
| L2-28 | 317 | `syneroym export --app <app-id>` | NOT BUILT | There is no `export` command for apps. `roymctl app` has Deploy, Reconcile, Forget, Health, Alerts, Resolve (`apps/roymctl/src/commands/app.rs:129-239`). There is no binary named `syneroym`; the binaries are `syneroym-substrate` and `roymctl`. Nearest: `roymctl roym backup create` (`apps/roymctl/src/commands/roym/backup.rs:47-60`) and `roymctl identity export` (`apps/roymctl/src/commands/identity.rs:136-146`). Presented as: working. | Replace with the Roym backup commands. Put a generic app export under `planned`. See Q6. |
| L2-29 | 317 | Archive holds "SQLite snapshot + blob store + identity keypair (optional) + App Spec" | DIVERGES | The Roym archive holds a master identity and per-service documents read through the app (sections: `crates/roym_core/src/backup.rs:17-25`). It holds no SQLite file, no blob store and no App Spec. The identity part is a separate encrypted file (`crates/identity/src/backup.rs:1-10`). | Describe what the Roym archive holds. |
| L2-30 | 317 | Archive is "signed" | UNCLEAR | The archive is encrypted with AES-GCM under a recovery key (`apps/roymctl/src/commands/roym/backup.rs:87,173`; `crates/identity/src/backup.rs:1-10`). A signed `bundle-manifest` record exists (`crates/roym_core/src/record.rs:30,44`). I did not trace whether the whole archive is signed or only the manifest. | Check in the fix step. Say "encrypted, with a signed manifest" if confirmed. |
| L2-31 | 318 | Archive is portable to any substrate with a compatible version | UNCLEAR | Archives carry a version number (`crates/roym_core/src/backup.rs:15`, `ARCHIVE_VERSION` at `apps/roymctl/src/commands/roym/backup.rs:25`). I did not find a rule about which substrate versions can restore it. | Check in the fix step. |
| L2-32 | 319 | Import "validates the archive signature and replays into a fresh SQLite instance" | DIVERGES | `restore-data` sends the data through the gateway into the running app services (`apps/roymctl/src/commands/roym/backup.rs:72-84`). It does not create a SQLite instance. | Rewrite to match `restore-identity` and `restore-data`. |
| L2-33 | 320 | Litestream keeps a live replica on a secondary node ("Torrent-Style Backup Pool") | NOT BUILT | No replication code (see L2-16). Presented as: working ("can keep"). | Move to `planned`. |
| L2-34 | 320 | Mutual pool: nodes host each other's symmetrically encrypted backups | NOT BUILT | No code, no config. Presented as: working ("formalised into"). | Move to `planned`. |
| L2-35 | 321 | Active failover: a Backup Substrate answers for a downed peer | NOT BUILT | No code. Also conflicts with the `[PLT-RED]` rule "no automatic failover" (line 2051). Presented as: working ("can act"). | Remove, or `planned`. Keep one rule: promotion is manual. See Q1. |

### 2.3 Storage and write arbitration (lines 323-338)

| ID | Doc line | Claim | Verdict | Evidence | Proposed fix |
| --- | --- | --- | --- | --- | --- |
| L2-36 | 325 | One encrypted SQLite database per service (`rusqlite` + `sqlcipher`) | MATCHES | One `state.db` per `service_id`: `crates/data_db/src/sqlite/provider.rs:250-285`; `Cargo.toml:144`. | Add: encryption is a config switch (L2-12). |
| L2-37 | 325 | Single writer, multiple readers | MATCHES | One writer task over an `mpsc` channel (`crates/data_db/src/sqlite/service_store.rs:100,237`, `provider.rs:300-304`) and a pool of reader connections (`provider.rs:308-330`). | None. |
| L2-38 | 325 | "A replica stays read-only until an operator promotes it" | NOT BUILT | No replica role, no read-only mode, no promote command in `crates/` or `apps/`. `replicas = N` compiles to N independent services, each with its own database (`crates/app_orchestration/src/compiler.rs:196-201`, `crates/app_orchestration/src/models/service.rs:305-315`, one file per `service_id` in `provider.rs:250-285`). Presented as: working. | Say what `replicas` does today. Move replica and promotion to `planned`. See Q1. |
| L2-39 | 327 | The writer applies "a business-level arbitration rule per entity" | DIVERGES | The data layer only serialises writes (L2-37). The rules live in app code (the Roym services), not in the substrate. | State that arbitration is an app rule. Move the table to the Roym docs. See Q3. |
| L2-40 | 331 | Order state: "Provider action beats a same-instant consumer action; otherwise first request wins" | DIVERGES | There is no order entity in Roym. The nearest rule is the agreement decision. One decision row is created per agreement with the data-layer create fence, and a later attempt gets `AlreadyDecided` (`crates/roym_transaction/src/app/ledger.rs:48,103-126`). The record is written on the provider's node only (`crates/roym_core/src/booking.rs:1-3`). No code makes a provider action win a tie. | Rewrite as "agreement decision: first claim wins; written only on the provider's node". |
| L2-41 | 332 | Catalog item: "Last write wins per field" | DIVERGES | A listing is saved with `put`, which replaces the whole payload (`crates/roym_catalog/src/app/listing_ops.rs:232-245`; `crates/wit_interfaces/wit/data-layer/data-layer.wit:85-100`). Each version is also kept in a history collection. Per-field merge is `patch`, which the catalog does not use. | Write "last write wins per listing; every version is kept". |
| L2-42 | 333 | Message: append-only log | MATCHES | `INSERT OR IGNORE INTO dag_entries` (`crates/conversation/src/store/dag_store.rs:308`). | None. |
| L2-43 | 334 | Booking slot: first confirmed reservation wins; later requests are rejected | MATCHES | Seats are claimed with the create fence in order; when all are taken the answer is `SlotTaken` (`crates/roym_transaction/src/app/ledger.rs:117-156`). | None. |
| L2-44 | 335 | Reputation record: append-only, signed by the issuer | NOT BUILT | No reputation record type (`crates/roym_core/src/record.rs:18-31`). Signed receipts exist (payment acknowledgement, fulfilment receipt). Presented as: working. | Remove the row, or `planned`. |
| L2-45 | 336 | Access control policy: "Provider's write wins; infrastructure provider cannot override" | UNCLEAR | A policy is saved with last-write-wins and replaces the old one (`crates/data_db/src/traits.rs:112-116`). Only the app owner may deploy (`crates/control_plane/src/service/orchestration/app_instance.rs:30`). Whether the node operator "cannot override" is not decided by the code I read: the operator holds the node's keys and files. | Check in the fix step. Do not claim operator-proof without a test. See Q8. |
| L2-46 | 338 | A disconnected client queues requests locally and replays them on reconnect, with idempotency keys | DIVERGES | The outbox and the idempotency fence are on the node, for service-to-service calls (`crates/router/src/proxy_outbox.rs:1-10`, `crates/router/src/call_dedup.rs:1-14`). The SDK client sends no idempotency key (`crates/sdk/src/client.rs:490,578`). No client-device queue exists. | Describe the node-side outbox as built. Move device queues to `planned`. |

### 2.4 Multi-device sync and sharded deployment (lines 340-362)

| ID | Doc line | Claim | Verdict | Evidence | Proposed fix |
| --- | --- | --- | --- | --- | --- |
| L2-47 | 346 | A secondary provider device is a client, not a second writer | UNCLEAR | True only because no multi-device feature exists. I found no code for a secondary device. | Keep as a design rule under `planned`. See Q4. |
| L2-48 | 347 | Offline requests queue in the device's local outbox with an idempotency key | NOT BUILT | See L2-46. Presented as: working. | `planned`. |
| L2-49 | 348 | On reconnection the queue replays against the single writer | NOT BUILT | See L2-46. Presented as: working. | `planned`. |
| L2-50 | 349 | Ownership is deterministic because there is one writer, not a merge step | MATCHES | Single writer task per service: `crates/data_db/src/sqlite/service_store.rs:100`. There is no merge code. | None. |
| L2-51 | 353 | "App Spec supports per-component placement constraints" | DIVERGES | Placement is per service, not per component. A manifest has a default and each service may override it. The only selector is the name of a substrate in the operator's inventory (`crates/app_orchestration/src/models/service.rs:306`, `crates/app_orchestration/src/models/manifest.rs:24`, `crates/app_orchestration/src/models/identifiers.rs:214-224`, `crates/app_orchestration/src/compiler.rs:146,233-235`). | Write: "a service can name the substrate it runs on". |
| L2-52 | 354 | The orchestrator schedules by resource class (`cpu`, `memory`, `gpu`, locality tags) | NOT BUILT | No scheduler. The operator keeps an inventory of substrate aliases, and "capabilities" there means service types only (`crates/app_orchestration/src/substrate_inventory.rs:1-20,50`). `cpu_limit` in `crates/core/src/config/sandbox.rs:33,102` limits one sandbox. It is not a placement rule. Presented as: working. | Move to `planned`. See Q5. |
| L2-53 | 355 | Inter-shard calls use "substrate-authenticated service identities over QUIC/WebSocket" | DIVERGES | Remote calls use Iroh QUIC with JSON-RPC (`crates/router/src/proxy.rs:1-6`, `crates/router/src/proxy/hop.rs`). The caller is verified at the receiving stream (`crates/router/src/route_handler/io.rs:393`). No WebSocket path. The compiler never emits `Sharded` mode (`crates/app_orchestration/src/compiler.rs:196-201`, `crates/app_orchestration/src/resolver/types.rs:317`). | Write "Iroh QUIC with JSON-RPC". Say sharded mode is not compiled yet. |
| L2-54 | 356 | A failed shard does not stop the others; dependents move to queued/retry mode | DIVERGES | Calls retry with backoff, and a guest may queue a call in the durable outbox (`crates/router/src/proxy_outbox.rs:1-10`). A synchronous call to a failed target fails; nothing moves it to "queued" by itself. | Say exactly which calls are queued. |
| L2-55 | 358-362 | Example placement: `catalog-browser`, `space-manager`, `order-engine`, `payment-adapter`, `drm-content-server` | STALE | These components do not exist. The Roym services are `web`, `profile`, `conversation`, `catalog`, `transaction`, `directory` (`crates/roym_core/app/roym.toml`). | Use the real service names or drop the example. |

### 2.5 Substrate API surfaces (lines 364-369)

| ID | Doc line | Claim | Verdict | Evidence | Proposed fix |
| --- | --- | --- | --- | --- | --- |
| L2-56 | 366 | "Two API surfaces, both derived from identical WIT definitions" | DIVERGES | One surface exists: JSON-RPC 2.0 (L2-03, L2-59). | Say one surface, with wRPC `planned`. |
| L2-57 | 368 | wRPC surface for WASM components, peer substrates over Iroh QUIC, and CLI | NOT BUILT | Peers use JSON-RPC over Iroh QUIC (`crates/router/src/proxy.rs:1-6`). The CLI uses the gateway or Iroh with JSON-RPC. wRPC is reserved in the preamble parser only (`crates/router/src/preamble.rs:19,28-29`). Presented as: working. | `planned`. |
| L2-58 | 368 | "WIT types are preserved end-to-end; zero serialization overhead" | NOT BUILT | Every call converts JSON ⇄ WIT values (`crates/sandbox_wasm/src/conversions.rs:1-30`). Presented as: working. | Delete, or attach to the planned wRPC file. |
| L2-59 | 369 | JSON-RPC 2.0 surface for browsers, third-party integrations and the status UI | MATCHES | `crates/router/src/route_handler/http.rs:175`; the Roym web service answers `POST /rpc` (`crates/roym_core/app/roym.toml`, `services.web` routes). | None. |
| L2-60 | 369 | The JSON-RPC surface is derived automatically from WIT | MATCHES | `crates/sandbox_wasm/src/conversions.rs:1-30`. | None. |
| L2-61 | 369 | "documented as an OpenRPC schema" | NOT BUILT | No OpenRPC file or generator in `crates/`, `apps/` or the developer guide. Presented as: working. | Remove, or `planned`. |

## 3. Structural problems in the whole file

I did not audit these in depth. They come from reading headings and searching the file.

1. **Two documents in one file.** There is a second `#` title at line 1738 ("Syneroym: Substrate Feature Implementation Design"). Line 3 says the addendum holds the "canonical Layer 1-4 definitions", but Layers 1-4 are still above it. Two sources of truth disagree (example: Litestream at lines 265 and 320 against `[PLT-RED]` at line 2040).
2. **Duplicate heading.** "Layer 3 — Shared Substrate Utilities" appears at lines 373 and 480.
3. **Broken table of contents.** Line 25 links to "MVP Phase 1 Scope & Acceptance Criteria". That heading does not exist. Line 26 links to `#connectivity-substrate-in-heterogeneous-networks`, but the heading at line 1269 is spelled "Heteregenous". The "Open Questions" link does not match the heading "Open Questions & Recommendations" (line 2249). The table omits the Appendix (line 1116) and the whole addendum (line 1737). These anchors are by GitHub's slug rules; I did not open a renderer.
4. **Plans written as working systems.** Most of this section (see section 1). Only one warning, at lines 5-6, and it covers only wRPC and multi-hop. The same pattern is likely in other Layers.
5. **Milestone and slice IDs in a living doc.** Examples: lines 513, 515, 1744, 1808, 1875, 1886, 1892, 1926-1929. This breaks rule 1 in [docs/README.md](../../../README.md). Line 1744 is a dated "Implementation Status" note (2026-07-12) that is now stale.
6. **A commit hash in the text.** Lines 3 and 1737 use "dd864a1" as a time marker. It means nothing to a new reader.
7. **Other places repeat Layer 2 claims.** Line 1003 says every arbitration rule has a simulation scenario. Lines 1098-1099 repeat the arbitration decision. When the table is fixed, these lines must change too.
8. **Mixed readers.** The file mixes substrate internals, the Roym product spec (Layer 4, Consumer Experience) and operations (Observability). One reader cannot use it all.
9. **Stale names in diagrams.** `CRSQL` (L2-13). Other diagrams were not checked.

## 4. Questions for the user

I made a default choice for each, so work can go on. Please correct me if wrong.

- **Q1. Replication target.** The doc names Litestream (lines 265, 320) and also says `[PLT-RED]` uses Iroh WAL shipping with manual promotion (lines 2040-2051). Neither is built. *Default:* keep the `[PLT-RED]` design as the one `planned` target. Drop Litestream, the hot standby and the backup pool unless you still want them.
- **Q2. Shared utilities.** Are matching, reputation and payments planned as substrate components, or are they app features of Roym? *Default:* app features. Remove them from the substrate diagram.
- **Q3. Arbitration table.** Should the living doc describe Roym's real rules (agreement decision, seat claim, listing replace, message log) and move them to the Roym docs? *Default:* yes. Drop the "order" and "reputation" rows.
- **Q4. Multi-device sync.** Is "a secondary device is a client of the primary" still the intended design? *Default:* yes, as `planned`.
- **Q5. Resource-class scheduling.** Is scheduling by `cpu`/`memory`/`gpu`/locality still wanted, or is operator-named placement the intended end state? *Default:* describe operator-named placement as built. Keep scheduling as `planned`.
- **Q6. App export.** Should a generic app export/import exist, or is the Roym backup (recovery key, signed manifest) the intended path? *Default:* Roym backup as built. Generic export as `planned`.
- **Q7. Rootless Podman.** Is "rootless" a requirement the substrate must enforce, or advice to operators? *Default:* advice. Write "uses the host's Podman".
- **Q8. Operator cannot override policy.** Is this a real security goal? *Default:* do not claim it until a test shows it.

## 5. Proposed split of this section

One file per capability. Each file has a reader at the top and a status per statement (`built` or `planned`).

| File | Purpose |
| --- | --- |
| `substrate/README.md` | Index. What the substrate is, and the table of roles and Cargo features. |
| `substrate/ingress-and-transports.md` | How streams arrive: Iroh QUIC, WebRTC, HTTP, optional WebSocket route, the route preamble. |
| `substrate/routing-and-access.md` | The pipeline per stream, caller identity, per-service admission and row-level policy. |
| `substrate/sandboxes.md` | Wasmtime and Podman: what runs where and the limits. |
| `substrate/storage.md` | One encrypted SQLite per service, the single writer, KEK/DEK, blob store. |
| `substrate/durable-calls.md` | The durable outbox, the idempotency fence and what is retried. |
| `substrate/app-packaging-and-deploy.md` | WIT, build, the manifest, plan compile, deploy and the supervisor. |
| `substrate/placement-and-topology.md` | Operator-named placement, `replicas`, Singleton/Redundant/Sharded and the resolver. |
| `substrate/api-surfaces.md` | JSON-RPC 2.0 and how WIT maps to JSON. Marks wRPC and OpenRPC as `planned`. |
| `substrate/backup-and-restore.md` | The Roym backup and identity export as built. |
| `substrate/planned-replication-and-failover.md` | `planned`: WAL shipping, manual promotion, backup pool, multi-device queues, resource-class scheduling. |
| `roym/write-rules.md` | Not substrate. The app rules for agreement decision, seat claim, listing and message log. |

## 6. Cost notes

- **Claims:** 61, from about 140 doc lines. Diagram nodes and table rows each count as one claim.
- **Time:** about 65 tool calls, all by me. I did not use subagents. I did not time the wall-clock, so I give no minutes.
- **Easy parts:** claims that are "not built" (L2-16, L2-18, L2-61 and similar). One search with zero hits settles them.
- **Hard parts:**
  - Finding where the "orchestrator" really lives. It is split in three crates.
  - Telling substrate rules from Roym app rules (the arbitration table).
  - Telling a node-side outbox from the "device outbox" the doc describes.
  - UNCLEAR rows (L2-11, L2-30, L2-31, L2-45, L2-47). A search cannot settle them; they need a test or a decision.
- **Effort level:** high effort was useful for the DIVERGES and UNCLEAR rows (about a third of the claims). Medium would have been enough for MATCHES and NOT BUILT rows, which are search-and-cite work. For the other sections, I suggest two passes: a medium pass that does the searches and cites, then a high pass only on the rows that are not plain matches.
- **Risk to the next step:** this audit is static. A MATCHES row means the code path exists, not that a test proves it end to end. The fix step should check each MATCHES row against a test before the new doc states it as tested.
