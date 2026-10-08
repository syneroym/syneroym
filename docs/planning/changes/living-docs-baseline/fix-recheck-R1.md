# Fix recheck, part R1 (labels S1 to S4)

Reader: the developer who closes out the architecture fix.

Scope: Task A checks the 64 new rows in `fix-new-claims.md` labelled S1 (15), S2 (21), S3 (16) and S4 (12). Task B checks that the findings in `fix-verification-S1.md` to `S4.md` are fixed in `docs/system-architecture.md`. Static reading only. Nothing was edited except this file.

## 1. Summary

Task A (64 rows):

| Verdict | Count | Rows |
| --- | --- | --- |
| CONFIRMED | 55 | all others |
| PARTLY | 6 | S1.8, S1.12, S1.13, S2.18, S2.19, S3.3 |
| WRONG | 1 | S3.4 |
| CITE-OFF | 2 | S3.11, S4.12 |
| UNVERIFIABLE | 0 | |

Task B (71 items: 38 findings and 33 uncovered-text items):

| Verdict | Count | Items |
| --- | --- | --- |
| FIXED | 65 | all others |
| STILL-WRONG | 3 | S2 uncovered 8, S3 8.17, S3 uncovered 2 |
| CODE-GAP-ONLY | 3 | S2 6.24, S2 6.2, S3 8.12 |

Per batch for Task B: S1 has 16 items (7 findings, 9 uncovered), S2 has 22 (14 and 8), S3 has 20 (12 and 8), S4 has 13 (5 and 8).

## 2. Task A table

| Row | Verdict | Note | Checked at |
| --- | --- | --- | --- |
| S1.1 | CONFIRMED | Payment record checked against terms; no money check. | `roym_core/src/payment.rs:150-165`; `roym_transaction/src/app/payment_ops.rs:503-510`; `sync/receipts.rs:185,460` |
| S1.2 | CONFIRMED | Same statement in the Payments section (doc line 1045). | same as S1.1 |
| S1.3 | CONFIRMED | `delegation` is optional; `None` arm means issuer signed. | `signed_record/src/envelope.rs:145`; `verify.rs:187-212` |
| S1.4 | CONFIRMED | Store holds journal, alerts, queue; alerts stored, health not stored. | `app_supervisor/src/store.rs:75-83`; `sdk/src/health.rs:560-600` |
| S1.5 | CONFIRMED | Rebuild is manual; `adopt` reads only held generation; needs prior `submit`. | `app_supervisor/src/service/verbs/lifecycle.rs:10-58` |
| S1.6 | CONFIRMED | `submit` mints missing member keys; `adopt` calls `app_master`. | `verbs/submit.rs:236`; `keys.rs:234-260,394-404` |
| S1.7 | CONFIRMED | Gateway and WebRTC coordinator use only the SDK `resolve` fetcher, with cache. | `sdk/src/topology.rs:100-135,306-335`; `client_gateway/src/gateway.rs:159-165` |
| S1.8 | PARTLY | `roym address` and `session delegate` do not call the gateway; address uses Iroh via `list_svcs`. | `apps/roymctl/src/commands/roym/address.rs:10-22`; `session.rs:26-39` |
| S1.9 | CONFIRMED | `check_generation` gates all listed operations. | `control_plane/.../deploy/admission.rs:154`; `lifecycle.rs:133,221`; `app_instance.rs:59,99`; `cert.rs:129` |
| S1.10 | CONFIRMED | Needs `[roles.supervisor]` and the default-on `supervisor` feature. | `substrate/Cargo.toml:55-66`; `runtime/supervisor.rs:21-43` |
| S1.11 | CONFIRMED | Same denial for unknown, ungranted, retired; `open` needs no grant. | `app_supervisor/src/service/resolve.rs:36-77`; `roym_core/app/roym.toml:93` |
| S1.12 | PARTLY | No-op needs same process run, same caller, running instance. Doc says only "identical". | `control_plane/.../deploy/admission.rs:182-195`; `deploy.rs:171` |
| S1.13 | PARTLY | Same omitted conditions as S1.12 (doc lines 687, 2811). | `deploy/admission.rs:182-195` |
| S1.14 | CONFIRMED | Backward walk on `compensate` or deadline; platform never decides failure. | `wit/proxy/proxy.wit:111-125,143-146`; `async_queue/src/saga.rs:279-287` |
| S1.15 | CONFIRMED | Only the person's own sources are queried; server half makes no outbound search. | `roym_directory/src/app/client_sources.rs:41,89`; `client_query.rs:132-135` |
| S2.1 | CONFIRMED | Check is `replicas > 1 && schema.is_some()`; blind spot stated. | `app_orchestration/src/models/manifest.rs:92`; `service.rs:167` |
| S2.2 | CONFIRMED | `state.db` writer sets no journal pragma; `async.db` sets WAL. | `data_db/src/sqlite/provider.rs:281-289`; `async_queue/src/queue.rs:619` |
| S2.3 | CONFIRMED | Dead letter needs key, outbox, guest origin, target-produced error. | `router/src/proxy/router.rs:16-32,588-593`; `proxy.rs:58-84` |
| S2.4 | CONFIRMED | 2 s probe; queue on Retry; -32095 delivered; others return. | `router/src/proxy/router.rs:547-578`; `proxy_outbox.rs:109-124` |
| S2.5 | CONFIRMED | 256 KiB value is at `proxy_outbox.rs:50`, used at :269; 10,000 cap right. | `router/src/proxy_outbox.rs:50,269-286`; `async_queue/src/queue/types.rs:71,92` |
| S2.6 | CONFIRMED | Claim window is 2x budget; prune is per caller, 10,000. | `async_queue/src/dedup.rs:80,95-96,341-350,380-410` |
| S2.7 | CONFIRMED | croner default accepts optional seconds and year. | `croner-3.0.1/src/parser.rs:55-70,143-152`; `app_orchestration/src/schedule.rs:74,150` |
| S2.8 | CONFIRMED | 401 in `dispatch_native`; SSE and stream have no caller check. | `router/src/route_handler/http/dispatch.rs:37-42,425-426`; `streaming.rs:9-95`; `guest.rs:41`; `websocket.rs:110` |
| S2.9 | CONFIRMED | 50,000,000 handler installed on the listed paths (and `row_reachable`). | `data_db/src/sqlite/query_raw.rs:98`; `sieve.rs:91-95`; `query.rs:40,125,198,296`; `mutation.rs:169` |
| S2.10 | CONFIRMED | Index SQL is `json_extract`; type never read. | `data_db/src/sqlite/schema.rs:49-55` |
| S2.11 | CONFIRMED | Masked fields or ABAC permissions give PermissionDenied. | `data_db/src/sqlite/query.rs:285-291` |
| S2.12 | CONFIRMED | Admin capability gate; lifecycle context carries it. | `sandbox_wasm/.../capabilities_store.rs:512-523`; `rpc/src/native.rs:92-108` |
| S2.13 | CONFIRMED | Both limits from node config; default 100 MiB; total optional. | `core/src/config/base.rs:55-89`; `substrate/src/runtime/router.rs:625-642` |
| S2.14 | CONFIRMED | Native `reveal` admits own system ids and owner; host fn returns bytes. | `control_plane/src/synsvc_native/signing.rs:42-67`; `vault_config.rs:16`; `capabilities_services.rs:38-47` |
| S2.15 | CONFIRMED | Schema only with `custom_config`; no-op conditions listed correctly. | `deploy/manifest.rs:44-51`; `deploy/admission.rs:182-195` |
| S2.16 | CONFIRMED | Zero-relation `=`, remote `IN (...)`, local `EXISTS`. | `fdae/src/compile/emit.rs:297-325,345` |
| S2.17 | CONFIRMED | `seen` path string; depth 64 second bound. | `fdae/src/compile/emit.rs:526,554-556`; `types.rs:14` |
| S2.18 | PARTLY | Guest presents cert only if unexpired cert and owner exist; else anonymous. Doc says "presents". | `router/src/proxy/router.rs:376-390` |
| S2.19 | PARTLY | Outbox budget right. "retry policy covers the proxy only" is false: relay forward and registration use it. | `router/src/route_handler/io.rs:494`; `coordinator_iroh/src/coordinator.rs:164`; `core/src/config/base.rs:238-249` |
| S2.20 | CONFIRMED | Supervisor outbox behind `WriteBindingsOutbox`; SDK client sets no key. | `app_supervisor/src/outbox.rs:98`; `sdk/src/client.rs:490,578` |
| S2.21 | CONFIRMED | WASM subscriptions inserted; native subscribe does not persist. | `data_db/src/sqlite/provider.rs:475-485`; `app_host_native/src/factory.rs:329-336` |
| S3.1 | CONFIRMED | Same as S2.1; Crash Consistency wording matches. | `app_orchestration/src/models/manifest.rs:92,102-117` |
| S3.2 | CONFIRMED | Max 8 sources; only client half queries; no code lets a directory choose. | `roym_core/src/directory.rs:46`; `roym_directory/src/app/client_sources.rs:89`; `client_query.rs:132` |
| S3.3 | PARTLY | SDK part right. "WebRTC bootstrap is the only node-side connection cache" is false: gateway keeps a client map. | `sdk/src/client.rs:341-343,387`; `client_gateway/src/gateway.rs:54,489-495` |
| S3.4 | WRONG | "No other component keeps a connection cache": gateway reuses one `SyneroymClient` per service, never evicts. | `client_gateway/src/gateway.rs:54,489-506`; `sdk/src/client.rs:341-343` |
| S3.5 | CONFIRMED | `0xed 0x01` plus key, z-base-32; other prefix or length rejected. | `identity/src/substrate.rs:142-147,163-167` |
| S3.6 | CONFIRMED | Only Iroh mechanisms built; record holds node id only. | `substrate/src/runtime/publish.rs:168-186`; `coordinator_iroh/src/coordinator.rs:492` |
| S3.7 | CONFIRMED | `request`, `request_raw`, `passthrough` as described. | `sdk/src/client.rs:479-540,620-675` |
| S3.8 | CONFIRMED | No listen in WIT; `TcpProxy` connects out to the TCP service. | `router/src/route_handler/io.rs:524-540`; `dispatch.rs:345-347` |
| S3.9 | CONFIRMED | `raw://` and `http://` handling as stated. | `router/src/route_handler/io.rs:524-540,575-598`; `http.rs:310-330`; `dispatch.rs:366-370` |
| S3.10 | CONFIRMED | `plan_pipeline` never builds `JsonRpcToWrpc`; wrpc gets -32091. | `router/src/route_handler/dispatch.rs:287-294,351-355`; `rpc/src/proxy.rs:140` |
| S3.11 | CITE-OFF | True. BLE/LoRa types are unread: use `core/src/config/base.rs:197-198` and `roles.rs:257`. | `core/src/config/base.rs:197-198`; `roles.rs:257` |
| S3.12 | CONFIRMED | Only JSON-RPC 2.0 as RPC; others carry bytes or HTTP. | `router/src/route_handler/http.rs:310-330`; `io.rs:524-540` |
| S3.13 | CONFIRMED | `register` errors when neither registry nor DHT. Both are used if both exist. | `core/src/dht_registry/client.rs:109-185` |
| S3.14 | CONFIRMED | Second lookup only for `Service`; `connect` fails with no URL or mechanisms. | `core/src/dht_registry/client.rs:271-277`; `sdk/src/client.rs:353-357` |
| S3.15 | CONFIRMED | `empty_builder` has no lookup and no relay; relay from record only. | `sdk/src/client.rs:374-380`; `iroh-0.97.0/src/endpoint.rs:171` |
| S3.16 | CONFIRMED | Both `share_in_registry` and URL are needed. | `coordinator_iroh/src/coordinator.rs:149-151` |
| S4.1 | CONFIRMED | Data channel strips query; ECDH only in WebSocket fallback. | `coordinator_webrtc/templates/peer-proxy.js:376-377,428-440` |
| S4.2 | CONFIRMED | 11 guests pin 0.55.0; fixture uses workspace. | `test-components/*/Cargo.toml:9`; `dual-build-fixture/Cargo.toml:20` |
| S4.3 | CONFIRMED | Only `mise.toml:9` and one test comment name `wasm-tools`. | `mise.toml:9`; `router/tests/proxy_dispatch.rs:71` |
| S4.4 | CONFIRMED | Gate list and docs-only skips match. | `xtask/src/verify.rs:28-110,242-258` |
| S4.5 | CONFIRMED | The seven `roym` subcommands exist. | `apps/roymctl/src/commands/roym.rs:36-93` |
| S4.6 | CONFIRMED | No parent call; parent relay only; 30 s wait always, warn on timeout. | `coordinator_iroh/src/coordinator.rs:194-217` |
| S4.7 | CONFIRMED | Registers once; `ttl: None`; sweep at 7200 s every 15 min. | `coordinator_iroh/src/coordinator.rs:349-379,494`; `community_registry/src/registry.rs:143-165` |
| S4.8 | CONFIRMED | DHT fallback when registry gave no record; DHT on by default. | `core/src/dht_registry/client.rs:235-264`; `config/base.rs:233` |
| S4.9 | CONFIRMED | HTTP failure returns before DHT publish. | `core/src/dht_registry/client.rs:136-153` |
| S4.10 | CONFIRMED | Endpoint only with `iroh` and `parent_coordinator.iroh`; else error text matches. | `router/src/connection_router.rs:81-98`; `route_handler/io.rs:487-491` |
| S4.11 | CONFIRMED | Page sets `enc`; only the WebSocket path runs the handshake. | `peer-proxy.js:376-377,428-440,587,944` |
| S4.12 | CITE-OFF | True, but the wrpc error is -32091 "unsupported protocol": cite `dispatch.rs:287-294,353-354`. | `router/src/route_handler/dispatch.rs:281-294,353-354` |

## 3. Task B table

Report row names: `S1 2.6` means finding 2.6 in `fix-verification-S1.md`. `U<n>` is item n in "Doc text not covered by any row". Doc line numbers are in `docs/system-architecture.md` now.

| Report row | Verdict | Note | Checked at |
| --- | --- | --- | --- |
| S1 2.6 | FIXED | Payment wording matches the proposed text; no old phrase left. | doc 1045, 2981 |
| S1 2.8 | FIXED | Delegation certificate is now conditional. | doc 2993 |
| S1 2.13 | FIXED | Cite issue only; doc statement still correct (limit 20 per 24 h). | doc 3025; `roym_directory/src/app.rs:74-79`; `roym_core/src/safety.rs:63` |
| S1 4.5 | FIXED | Evidence issue only; doc unchanged and correct. | doc 1439 |
| S1 5.8 | FIXED | Ledger now lists journal, alerts, outbox; health not stored. | doc 2785 |
| S1 5.9 | FIXED | `import-master` before first `submit` for members; app key before `adopt`. | doc 1599, 2785 |
| S1 5.14 | FIXED | Hook text matches. Missing "same process" condition is Task A S1.12. | doc 687, 2811 |
| S1 U1 | FIXED | Gateway and WebRTC coordinator `resolve` now stated; "hot path" gone. | doc 2771 |
| S1 U2 | FIXED | `open` needs no grant; directory is open. | doc 2801 |
| S1 U3 | FIXED | "Each client chooses"; no "SynOrg, directory or aggregator" phrase left. | doc 2917 |
| S1 U4 | FIXED | Cargo feature `supervisor` stated; `minimal` leaves it out. | doc 2780 |
| S1 U5 | FIXED | "for its deployment commands". `roym address` still goes over Iroh (Task A S1.8). | doc 2770 |
| S1 U6 | FIXED | No "sweep" claim; manual rebuild; sweep marked Envisioned. | doc 2785, 2787 |
| S1 U7 | FIXED | "when the guest gives up or the deadline passes". | doc 2733-2734, 2904, 2965 |
| S1 U8 | FIXED | Hook text now says "WASM service". | doc 687, 2811 |
| S1 U9 | FIXED | Generation list now complete. | doc 2778 |
| S2 6.2 | CODE-GAP-ONLY | Doc says `schema` marker and the blind spot. Backlog row 354. | doc 628, 2743; `deferred-backlog.md:354` |
| S2 6.4 | FIXED | Now says `state.db` has no WAL; only `async.db` has WAL. | doc 609, 2629, 2633 |
| S2 6.7 | FIXED | DLQ text lists the cases and the -32094/-32095 exclusion. | doc 2722 |
| S2 6.8 | FIXED | Outbox entry cases, 2 s probe, 256 KiB, 10,000 stated. | doc 2724 |
| S2 6.9 | FIXED | Dependency name versus DID stated. | doc 2724 |
| S2 6.11 | FIXED | At-least-once and per-caller row cap stated. | doc 2727 |
| S2 6.13 | FIXED | Seconds and year fields stated. | doc 706, 2730 |
| S2 6.24 | CODE-GAP-ONLY | SSE and `stream` ungated is stated. Backlog rows 46 and 52 (row 52 wrongly says messaging admits anonymous). | doc 2681; `deferred-backlog.md:46,52` |
| S2 6.26 | FIXED | "50,000,000 instructions" on listed paths; "cap how long" gone. | doc 2608 |
| S2 6.27 | FIXED | Type not used; all indexes `json_extract`. | doc 2615 |
| S2 6.31 | FIXED | One node-wide blob quota. | doc 2626 |
| S2 7.3 | FIXED | Native `reveal` for own service and owner. | doc 2534 |
| S2 7.6 | FIXED | "deploy that changes something"; no-op stated with condition. | doc 2546 |
| S2 7.16 | FIXED | `EXISTS`, direct column, `IN (...)` per case. | doc 2576 |
| S2 U1 | FIXED | `visited_track` gone; `seen` used. | doc 2583 |
| S2 U2 | FIXED | Now `invoke_remote_at` in `proxy/router.rs`. | doc 2688 |
| S2 U3 | FIXED | Native origin forwards proof; guest never does. | doc 2688 |
| S2 U4 | FIXED | Supervisor outbox mentioned. | doc 2738 |
| S2 U5 | FIXED | Gate is the `data-layer/admin` capability. | doc 2619 |
| S2 U6 | FIXED | `aggregate` refusal is in the code; doc covers views only. Wording not required. | `data_db/src/sqlite/query.rs:285-291`; doc 2617 |
| S2 U7 | FIXED | WASM persisted; native not persisted. | doc 2677 |
| S2 U8 | STILL-WRONG | "This `retry` policy covers the retries of the proxy only" is false. Doc line 2515 contradicts it. | doc 2721; `route_handler/io.rs:494`; `coordinator_iroh/src/coordinator.rs:164` |
| S3 8.12 | CODE-GAP-ONLY | Doc says `schema` marker and blind spot. Backlog row 354. | doc 2504; `deferred-backlog.md:354` |
| S3 8.14 | FIXED | "Each client chooses"; old phrase gone. | doc 2917 |
| S3 8.17 | STILL-WRONG | "Only the WebRTC bootstrap reuses connections between nodes" is false: gateway reuses one client per service. | doc 2520; `client_gateway/src/gateway.rs:489-495` |
| S3 10.4 | FIXED | DID encoding with `0xed 0x01` stated. | doc 1931 |
| S3 10.13 | FIXED | "No node publishes a `WebRtc` mechanism today." | doc 2055 |
| S3 10.28 | FIXED | `request`, `request_raw`, `passthrough` described right. | doc 2144 |
| S3 10.29 | FIXED | WASM has no accept; TCP service has own listener. | doc 2146, 2163 |
| S3 10.31 | FIXED | `raw://` and `http://` cases match proposed text. | doc 2150 |
| S3 10.39 | FIXED | `JsonRpcToWrpc` never picked; -32091. | doc 2326 |
| S3 10.41 | FIXED | "No node role routes inside a BLE or LoRa network." | doc 2414 |
| S3 10.7 | FIXED | Evidence cite only; doc says pkarr 6.0. | doc 1841 |
| S3 10.42 | FIXED | Evidence cite only; no doc change needed. | `coordinator_webrtc/src/bootstrap/tunnel.rs:170-232` |
| S3 U1 | FIXED | "only RPC wire protocol" wording used. | doc 8, 2348 |
| S3 U2 | STILL-WRONG | "The only connection cache is in the WebRTC bootstrap ... No other component keeps connection references" is false. | doc 2517; `client_gateway/src/gateway.rs:54,489-495` |
| S3 U3 | FIXED | Record visibility text now conditional. | doc 2122 |
| S3 U4 | FIXED | Second lookup only for `Service` records. | doc 2275 |
| S3 U5 | FIXED | Registry URL or mechanisms needed; `connect` fails at once. | doc 2259 |
| S3 U6 | FIXED | SDK has no relay unless record has `relay_url`. | doc 2176 |
| S3 U7 | FIXED | Both settings needed. | doc 362, 2098 |
| S3 U8 | FIXED | Proxy allows one connect attempt per try; stated. | doc 2515 |
| S4 11.21 | FIXED | WebSocket tunnel path only. | doc 1874 |
| S4 11.24 | FIXED | "Most guests pin 0.55.0". | doc 1884 |
| S4 12.6 | FIXED | 30 s wait always; parent relay only. | doc 1734 |
| S4 12.27 | FIXED | Browser sets `enc` on WebSocket path only. | doc 1811 |
| S4 12.31 | FIXED | Payload text conditional on JSON-RPC route. | doc 1815 |
| S4 U1 | FIXED | Cp record gone after about 2 hours. | doc 1757 |
| S4 U2 | FIXED | "in the registry or in the DHT". | doc 1798 |
| S4 U3 | FIXED | DHT publish only after registry accepts. | doc 1720, 2086 |
| S4 U4 | FIXED | `roymctl` row lists the `roym` subcommands. | doc 1896 |
| S4 U5 | FIXED | Python gate and docs-only skip stated. | doc 1895 |
| S4 U6 | FIXED | "No task calls it." | doc 1885 |
| S4 U7 | FIXED | "makes no call to its parent coordinator". | doc 1734 |
| S4 U8 | FIXED | Iroh endpoint condition stated. | doc 1742 |

## 4. Findings (rows not CONFIRMED / FIXED / CODE-GAP-ONLY)

### F1. S3.3, S3.4, S3 8.17 (STILL-WRONG), S3 U2 (STILL-WRONG): the client gateway keeps a connection cache

Doc words:
- `### Reactive Eviction`, line 2517: "The only connection cache is in the WebRTC bootstrap. ... No other component keeps connection references to evict."
- Envisioned note, line 2520: "Only the WebRTC bootstrap reuses connections between nodes."

Code: `GatewayState.clients` is `DashMap<String, Arc<Mutex<SyneroymClient>>>` (`crates/client_gateway/src/gateway.rs:54`). Each proxied request takes the client for its service from this map, or makes one, then calls `connect()` (`gateway.rs:489-506`). `SyneroymClient::connect` returns at once when a connection is already stored (`crates/sdk/src/client.rs:341-343`). So the gateway reuses one Iroh connection per service. It never checks that the connection is still open and never evicts it.

Proposed wording for line 2517: "Connections are not proactively monitored. Two components keep connections between requests. The WebRTC bootstrap keeps a `connection_cache`. It drops a cached connection that has a close reason before it reuses it, and removes the entry when `open_bi()` fails. The client gateway keeps one SDK client for each service and reuses its connection. It does not check or evict that connection."

Proposed wording for line 2520: "Today a node opens a new QUIC connection for each proxied call and each forwarded stream. The WebRTC bootstrap and the client gateway reuse connections. The SDK client keeps one connection and opens a new stream for each call."

Also review whether the gateway should be added as a backlog row (no eviction of a closed cached client). Check `docs/planning/deferred-backlog.md` before adding.

### F2. S2.19 and S2 U8 (STILL-WRONG): the `retry` policy is not "proxy only"

Doc words: `[PLT-ASY]` Configuration, line 2721: "This `retry` policy covers the retries of the proxy only."

Code: the same node-wide `retry` also sets the connect retry when the router forwards a stream (`crates/router/src/route_handler/io.rs:494`, `self.inner.retry_policy` from `route_handler.rs:416`) and the retry of the coordinator registration (`crates/coordinator_iroh/src/coordinator.rs:164`, `retry_with_backoff` at :359). Doc line 2515 already says this.

Proposed wording: "This `retry` policy is not used by the outbox. The outbox has its own attempt budget in `roles.app_sandbox`: `queue_max_attempts` (default 54) and a backoff ceiling `queue_max_backoff_secs` (default 900 s)."

### F3. S1.8: not every `roym` and `session` command calls the gateway

Doc words: `CLI Standalone`, line 2770: "Its `roym` and `session` command groups call the client gateway over HTTP, and `registry` calls the community registry over HTTP."

Code: `roym address` builds an SDK client with `client_for` and calls `list_svcs()` over Iroh (`apps/roymctl/src/commands/roym/address.rs:10-22`). `session delegate` makes a key file and publishes the master anchor to the registry. It has no `gateway_url` option (`apps/roymctl/src/commands/session.rs:26-39`). The other `roym` and `session` commands take `gateway_url`.

Proposed wording: "Most of its `roym` and `session` commands call the client gateway over HTTP. `roym address` reads the service list from the node over Iroh. `session delegate` works on local files and the registry. `registry` calls the community registry over HTTP."

### F4. S1.12 and S1.13: "identical deploy does nothing" needs its conditions

Doc words: LFC-VER line 687 and line 2811: "A deploy that is identical to the running service does nothing, so no hook runs."

Code: the no-op needs all of: this process has finished a full deploy of the service before, the stored hash is equal, the caller is the owner (or there is no owner), and the instance is running (`crates/control_plane/src/service/orchestration/deploy/admission.rs:182-195`; `deploy.rs:171`). After a node restart the first identical redeploy is not a no-op. The service has a database, so it runs `migrate()` (`crates/sandbox_wasm/src/engine/lifecycle.rs:114-129`). Doc line 2546 already states the condition.

Proposed wording: "A deploy that is identical to the installed, running service (same caller) does nothing, so no hook runs. This holds only after the node has run a full deploy of that service since it started."

### F5. S2.18: a guest call does not always present a certificate

Doc words: `3 Universal Proxy`, line 2688: "A call made by a guest never forwards the proof of its caller. It presents the own instance certificate of the guest service instead."

Code: the guest arm presents the instance key and certificate only when the service has an unexpired certificate and a recorded owner. Otherwise it presents nothing and the target sees an anonymous caller (`crates/router/src/proxy/router.rs:376-390`).

Proposed wording: "A call made by a guest never forwards the proof of its caller. It presents the instance certificate of the guest service when the service holds a valid one. Otherwise the target sees an anonymous caller."

### F6. S3.11 and S4.12 (CITE-OFF)

- S3.11: no doc change. Use `crates/core/src/config/base.rs:197-198` (`parent_coordinator.ble`, `.lora`) and `crates/core/src/config/roles.rs:257` (`transport_bridge`) as evidence. No other file reads them.
- S4.12: no doc change (doc line 8 already says "unsupported-protocol error"). The row text "answers a not-implemented error" is wrong. The router answers `-32091` unsupported protocol (`crates/router/src/route_handler/dispatch.rs:287-294,353-354`). Line 283 is a guard that `plan_pipeline` never reaches.

## 5. Doc statements that look wrong or unproven (not in any row)

1. `docs/system-architecture.md:3030`: "An aggregator is a SynOrg `directory` service, and can federate with other aggregators and proxy queries to them." The paragraph is the design under an Envisioned note (line 3027). It reads as a built fact. No code does this (`client_query.rs:132`, `client_sources.rs:41,248` are the only `CallTarget::Service` calls). Add "In the design," or "Envisioned:".
2. `docs/system-architecture.md:1739`: "**Sx** connects outbound to Coordinator **C** and Registry **R**." Code only builds an Iroh endpoint that uses the parent relay URL (`crates/router/src/connection_router.rs:81-98`). It makes no connection to a coordinator. Unproven for **C**.
3. `docs/planning/deferred-backlog.md:52` (a planning doc, not the architecture doc): "`stream` and `messaging` targets admit anonymous requests". `messaging` `publish` returns 401 for no caller (`crates/router/src/route_handler/http/dispatch.rs:37-42,435-448`). Only `subscribe-sse` is open.
4. `docs/system-architecture.md:2122` says a node "publishes its own record to its registry, or to the DHT when the DHT is on". With both configured, the code does both (`crates/core/src/dht_registry/client.rs:109-153`); doc line 2086 says so. Replace "or" with "and, when the DHT is on, also".
