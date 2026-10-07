# Fix verification, batch S3 (fix commits 8, 9 and 10)

Reader: the author of the architecture fix, who must correct the doc text.

Scope: 59 rows of `fix-new-claims.md` (commit 8: 17, commit 9: 0, commit 10: 42). Checked against the code on branch `docs/architecture-fix`. Only static reading. Row id = `<commit>.<n>`, the n-th row of that commit in file order. Doc lines below are lines of `docs/system-architecture.md`.

## Summary

| Verdict | Count |
| --- | --- |
| CONFIRMED | 47 |
| CITE-OFF | 2 |
| PARTLY | 9 |
| WRONG | 1 |
| UNVERIFIABLE | 0 |
| Total | 59 |

Commit 8: 14 CONFIRMED, 3 PARTLY. Commit 9: no rows. Commit 10: 33 CONFIRMED, 2 CITE-OFF, 6 PARTLY, 1 WRONG.

## Rows

| Row | Verdict | Note | Checked at |
| --- | --- | --- | --- |
| 8.1 | CONFIRMED | `roymctl` calls `compile`; `deploy-plan` goes to the orchestrator; supervisor receives a ready plan. | `crates/app_orchestration/src/compiler.rs:25`; `apps/roymctl/src/commands/app/deploy.rs:266`; `crates/sdk/src/client/orchestrator.rs:188` |
| 8.2 | CONFIRMED | `ServiceId` and `AppDid` must start `did:key:`; `AppScope` and `TopologyKey` as stated. | `crates/app_orchestration/src/models/identifiers.rs:153-170`; `crates/app_orchestration/src/resolver/types.rs:305-345` |
| 8.3 | CONFIRMED | Three modes; unkeyed `Redundant` round-robin, keyed uses rendezvous. | `crates/app_orchestration/src/resolver/select.rs:80-146` |
| 8.4 | CONFIRMED | Four u64 big-endian length-prefixed fields; highest digest wins; tie goes to highest ServiceId bytes; range sharding excluded. | `crates/app_orchestration/src/resolver/select.rs:23-61,99-146`; `resolver/types.rs:325-330` |
| 8.5 | CONFIRMED | Cache keyed by `TopologyKey`; TTL, `not_after`, `register`, `invalidate`; no epoch check; no route cache in router. | `crates/app_orchestration/src/resolver.rs:123-180`; `resolver/registry.rs:106-133` |
| 8.6 | CONFIRMED | `Foreign` entries come from verified Tier-2 documents; SDK registers them this way. | `crates/app_orchestration/src/resolver/types.rs:293-304`; `crates/sdk/src/topology.rs:525-545` |
| 8.7 | CONFIRMED | Compiler emits `Redundant` or default only; `sharding_strategy` is copied, read by no mode choice. | `crates/app_orchestration/src/compiler.rs:196-199,242`; `models/service.rs:316-327` |
| 8.8 | CONFIRMED | Routing key is bytes; split at first zero byte; `range_select` validates; manifest refuses `range_sharding`. | `crates/app_orchestration/src/resolver/select.rs:64-65,126-131`; `models/manifest.rs:110-117` |
| 8.9 | CONFIRMED | Record fields as stated; `StaticInventory` is a map; replay registers each `service_bindings` row. | `crates/app_orchestration/src/resolver/types.rs:182-218`; `crates/substrate/src/runtime/router.rs:295-318`; `crates/data_db/src/registry_store.rs:34,157` |
| 8.10 | CONFIRMED | Six states exist; deploy writes Planned, Applying, then Active or Degraded. | `crates/app_orchestration/src/journal.rs:15-26`; `apps/roymctl/src/commands/app/deploy.rs:516-523,581,590` |
| 8.11 | CONFIRMED | `reconcile` prints only; `deploy` resumes Applying or Degraded; no write of the two rollback states. | `apps/roymctl/src/commands/app/reconcile.rs:24-70`; `crates/app_orchestration/src/reconcile.rs:55-75`; grep `RollingBack` |
| 8.12 | PARTLY | Refusals exist. But `schema` is the config JSON schema, not a database schema. | `crates/app_orchestration/src/models/manifest.rs:92`; `models/service.rs:167`; `crates/control_plane/src/service/orchestration/deploy/manifest.rs:51` |
| 8.13 | CONFIRMED | 5 s timeout, `revoked_keys` check, registry then DHT, signature plus 24 h bound on registry anchor. | `crates/router/src/handshake.rs:65-75`; `crates/core/src/dht_registry/client.rs:290-330`; `master_anchor.rs:152-165` |
| 8.14 | PARTLY | Only a person's own `sources` list exists. No SynOrg, directory or aggregator query code. | `crates/roym_directory/src/app/client_query.rs:117-142`; `crates/roym_directory/src/app/held.rs:1-3` |
| 8.15 | CONFIRMED | No connection cache in `coordinator_iroh`; new `connect` per hop call and relay; WebRTC cache exists (reason is a code comment). | `crates/router/src/net_iroh.rs:151-161`; `crates/coordinator_webrtc/src/bootstrap/tunnel.rs:170-232` |
| 8.16 | CONFIRMED | Defaults 3, 100 ms, x2, 30 s, +-10% jitter; users as listed; tunnel connects once. | `crates/core/src/config/base.rs:237-267`; `crates/core/src/retry.rs:12-50`; `crates/router/src/proxy/router.rs:405-437` |
| 8.17 | PARTLY | Node side true. But `SyneroymClient` keeps one Iroh connection and opens a stream per call. | `crates/sdk/src/client.rs:339,386-388,511` |
| 10.1 | CONFIRMED | SDK looks up registry then DHT, dials Iroh, sends preamble; router accepts Iroh and WebRTC. | `crates/sdk/src/client.rs:338-410`; `crates/router/src/connection_router.rs:114-170` |
| 10.2 | CONFIRMED | Local registry miss triggers `resolve_iroh_addr` then Iroh `connect` and byte pipe. | `crates/router/src/route_handler/io.rs:313-317,471-498` |
| 10.3 | CONFIRMED | Config types parsed; no reader. Only hit is `transport_bridge: None` in smoke tests. | `crates/core/src/config/base.rs:195-205`; `roles.rs:257,340-350`; grep `ble`, `lora`, `transport_bridge` |
| 10.4 | PARTLY | Encoding covers a 2-byte multicodec prefix `0xed 0x01` plus the key, not the bare key. | `crates/identity/src/substrate.rs:142-147,158-166` |
| 10.5 | CONFIRMED | Alias is `<nickname>-<hash>` or bare hash; registry resolves it; DHT path warns for non-DID. | `crates/core/src/util.rs:90-100`; `crates/community_registry/src/registry.rs:306-312`; `dht_registry/client.rs:256` |
| 10.6 | CONFIRMED | `substrate_id` is the hosting node; for a node record it equals `service_id`. | `crates/core/src/dht_registry/types.rs:67-70`; `crates/sdk/src/deploy/certify.rs:163-170` |
| 10.7 | CITE-OFF | True. Cite names `pkarr-5.0.3`; workspace uses `pkarr` 6.0.1 (same check at line 276). | `~/.cargo/registry/src/*/pkarr-6.0.1/src/signed_packet.rs:276`; `Cargo.toml:142` |
| 10.8 | CONFIRMED | Address pruned to node id; relay URL kept separately. Cite lines are about 6 off. | `crates/substrate/src/runtime/publish.rs:168-176` |
| 10.9 | CONFIRMED | One `EndpointInfo`; `EndpointType` has `Substrate` and `Service`, serialized snake case. | `crates/core/src/dht_registry/types.rs:47-93` |
| 10.10 | CONFIRMED | `verify` ties packet key to `service_id`; mainline target is SHA-1 of the key. | `crates/core/src/dht_registry/types.rs:131-145`; `mainline-6.2.0/src/common/mutable.rs:46-58` |
| 10.11 | CONFIRMED | All service records built with empty `mechanisms`; lookup copies node mechanisms. | `crates/sdk/src/deploy/certify.rs:163-170`; `apps/roymctl/src/commands/svc/deploy.rs:443-453`; `dht_registry/client.rs:272-277` |
| 10.12 | CONFIRMED | No `protocols` field; fixed `plan_pipeline` match on scheme and endpoint. | `crates/core/src/dht_registry/types.rs:67-93`; `crates/router/src/route_handler/dispatch.rs:314-357` |
| 10.13 | PARTLY | Types as stated. But no code ever publishes a `WebRtc` mechanism. | `crates/substrate/src/runtime/publish.rs:177-191`; grep `EndpointMechanism::WebRtc` |
| 10.14 | CONFIRMED | `WebRtc` arm is a no-op ("Not implemented"). | `crates/sdk/src/client.rs:403-405` |
| 10.15 | CONFIRMED | POST `/register`; DHT publish in background when enabled; registry refusal returns error. | `crates/core/src/dht_registry/client.rs:109-190`; `crates/core/src/config/base.rs:224,233` |
| 10.16 | CONFIRMED | Registry first; DHT if none or no registry; DHT hit written back to registry. | `crates/core/src/dht_registry/client.rs:195-269` |
| 10.17 | CONFIRMED | `verify` (signature, `not_after`); full-DID check; failure returns error, no DHT fallback. | `crates/core/src/dht_registry/client.rs:208-232`; `types.rs:173-178` |
| 10.18 | CONFIRMED | Registry forwards when `!is_private` to `parent_registry_url`. | `crates/community_registry/src/registry.rs:226-230,287-300` |
| 10.19 | CONFIRMED | One spawned task with `retry_with_backoff`, no loop. Also needs `community_registry_url`. | `crates/coordinator_iroh/src/coordinator.rs:149-166,349-389` |
| 10.20 | CONFIRMED | Hourly heartbeat; stored records replayed verbatim; `orchestrator.republish` forces a pass. | `crates/substrate/src/runtime/publish.rs:66-73,103-135`; `crates/control_plane/src/service/dispatch.rs:49,465-476` |
| 10.21 | CONFIRMED | `ttl` or 7200 s; sweep every 15 minutes. | `crates/community_registry/src/registry.rs:137-157`; `dht_registry/types.rs:16` |
| 10.22 | CONFIRMED | 30 day `not_after`; `verify` rejects expired; 7 day warning window. | `crates/core/src/dht_registry/types.rs:37,173-178`; `crates/core/src/endpoint_publisher.rs:26-33,79-110` |
| 10.23 | CONFIRMED | Timestamp compare, identical bytes refresh; mainline checks `seq`; `generation` not enforced. | `crates/community_registry/src/registry.rs:240-285`; `mainline-6.2.0/src/rpc/server.rs:345` |
| 10.24 | CONFIRMED | Default `Private`; `Internal` is `is_private` true; both gates skip parent and DHT. | `crates/app_orchestration/src/models/service.rs:217-225`; `crates/sdk/src/deploy/certify.rs:149-170`; `dht_registry/client.rs:143-152` |
| 10.25 | CONFIRMED | `--record-out` writes record; `new_with_record` verifies; override looks up `substrate_id`. | `apps/roymctl/src/commands/svc.rs:151`; `crates/sdk/src/client.rs:245-270,338-360` |
| 10.26 | CONFIRMED | `connect_with_mechanisms` binds its own `Endpoint::empty_builder()`. | `crates/sdk/src/client.rs:363-390` |
| 10.27 | CONFIRMED | `RegistryClient`, `EndpointRegistry`, `ConnectionRouter`, `AdaptationStage` all exist. | `crates/core/src/dht_registry/client.rs:23`; `crates/core/src/local_registry.rs:77`; `crates/router/src/routing.rs:41-54` |
| 10.28 | WRONG | Names exist. Doc says `request_raw` gives a raw byte stream; it sends and returns JSON-RPC. | `crates/sdk/src/client.rs:479,528,620-690` |
| 10.29 | PARTLY | True for WASM. A TCP or container service listens on its own port; doc says "never". | `crates/router/src/route_handler/io.rs:524-540`; `connection_router.rs:150-170` |
| 10.30 | CONFIRMED | Both stream types implement `AsyncRead` and `AsyncWrite`. | `crates/router/src/net_iroh.rs:57,67`; `net_webrtc.rs:114,124` |
| 10.31 | PARTLY | `raw://` to WASM is a stream protocol (needs `dir=`); `http://` to TCP is raw copy. | `crates/router/src/route_handler/io.rs:549-587`; `dispatch.rs:366-370` |
| 10.32 | CONFIRMED | Two transports; `TcpProxy` is a service stage; SDK dials `Iroh` only. | `crates/router/src/net_iroh.rs:28`; `routing.rs:72`; `crates/sdk/src/client.rs:363-410` |
| 10.33 | CONFIRMED | First `Iroh` mechanism, no strategies. Needs record `relay_url` for any relay (see list below). | `crates/sdk/src/client.rs:363-410` |
| 10.34 | CONFIRMED | `relay_to_next_hop` forwards over Iroh; no BLE or LoRa code exists. | `crates/router/src/route_handler/io.rs:471-498` |
| 10.35 | CONFIRMED | `lookup(.., true)`; second lookup only for `Service` records; router uses same call. | `crates/sdk/src/client.rs:338-360`; `dht_registry/client.rs:272-277`; `router/src/net_iroh.rs:129-134` |
| 10.36 | CONFIRMED | ALPN `syneroym/0.1`, 10 s default, WebRtc skipped, first error returned, final message exact. | `crates/sdk/src/client.rs:28-33,384-410`; `connection_router.rs:48` |
| 10.37 | CONFIRMED | Scheme table in `parse`; `wrpc` and unknown give typed error `-32091`. | `crates/router/src/preamble.rs:216-240`; `dispatch.rs:286-295,348-355`; `crates/rpc/src/proxy.rs:140` |
| 10.38 | CONFIRMED | Code comment says negotiation is deferred; grep finds no protocol-list exchange. | `crates/router/src/route_handler/dispatch.rs:348-352`; `crates/rpc/src/proxy.rs:31` |
| 10.39 | PARTLY | The planner never builds `JsonRpcToWrpc`. A `wrpc://` stream gets the unsupported-protocol error. | `crates/router/src/route_handler/dispatch.rs:282-285,348-355`; grep `JsonRpcToWrpc` |
| 10.40 | CONFIRMED | Preamble, identity check, `ecdh-p256` end, pipeline of four stages. | `crates/router/src/route_handler/io.rs:322-326,395-400`; `dispatch.rs:314-372`; `handshake.rs:40-85` |
| 10.41 | PARTLY | "No gateway role exists" clashes with the real `client_gateway` role. Scope the wording. | `crates/core/src/config/roles.rs:244-262`; `crates/sdk/src/client.rs:363-410` |
| 10.42 | CITE-OFF | True. WebRTC relay lives in the tunnel code, not in `bootstrap.rs:60-68` (the cache field). | `crates/coordinator_webrtc/src/bootstrap/tunnel.rs:170-232`; `crates/coordinator_iroh/src/config.rs:101-109` |

## Findings

### 8.12 (PARTLY) `schema` is not a database schema

Doc (line 2495, `[TOP-DSC]` Crash Consistency): "The manifest check refuses `replicas > 1` on a service that has a `schema` (each member has its own database, so the data would split) ..."

Code: the check is `spec.replicas > 1 && spec.config.schema.is_some()` (`crates/app_orchestration/src/models/manifest.rs:92`). `config.schema` is the JSON Schema that validates `custom_config` (`crates/control_plane/src/service/orchestration/deploy/manifest.rs:51-59`). A service that uses the data layer without that field is not refused. The compiler's own message makes the same assumption, so the intent is clear, but the rule does not see a database.

Proposed text: "The manifest check refuses `replicas > 1` on a service that declares a config `schema`. It treats that field as a sign of a service that holds state, because each member has its own database and the data would split. It cannot see a service that uses the data layer without a `schema`."

### 8.14 (PARTLY) Who chooses what to query

Doc (line 2499, `[TOP-DSC]` Finding Providers): "Clients, SynOrgs, directories and aggregators each choose what they query."

Code: the only querying code is the "client half" of the `directory` service. It reads `this person's own sources` (`crates/roym_directory/src/app/held.rs:1-3`, `client_sources.rs:89-90`, at most 8) and calls each with `directory.search` (`client_query.rs:117-142`). No code makes a SynOrg, a directory or an aggregator pick sources. `aggregator` appears in no `.rs` file except a comment in `crates/observability/src/recorder.rs:24`.

Proposed text: "Discovery is what the Roym `directory` service does today. A person's node queries the directories that the person chose. See [P2P-DSC](#p2p-dsc-tag-routed-discovery-routing-mechanics) for what is built and what is not."

### 8.17 (PARTLY) "Only the WebRTC bootstrap reuses connections"

Doc (line 2511, `[TOP-ROB]` Envisioned): "Today a proxied call and a forwarded stream each open a new QUIC connection, and only the WebRTC bootstrap reuses connections."

Code: `SyneroymClient` stores one Iroh `Connection` after `connect()` (`crates/sdk/src/client.rs:339,386-388`). Each `request`, `request_raw` and `passthrough` opens a new stream on it with `conn.open_bi()` (`client.rs:511,651`). So the SDK client reuses its connection. Node-side code does not.

Proposed text: "Today a node opens a new QUIC connection for each proxied call and each forwarded stream. Only the WebRTC bootstrap reuses connections between nodes. The SDK client keeps one connection and opens a new stream for each call."

### 10.4 (PARTLY) Encoding of the DID

Doc (line 1924, Identity Model): "`did:key:h` followed by the z-base-32 encoding of an Ed25519 public key."

Code: `derive_did_key` encodes the 2 bytes `0xed 0x01` followed by the 32 key bytes, then z-base-32 (`crates/identity/src/substrate.rs:142-147`). `resolve_did_key` rejects any other prefix or a length other than 34 (`substrate.rs:158-166`). A developer who encodes only the 32 key bytes builds a DID that does not resolve.

Proposed text: "Both are `did:key` identifiers: `did:key:h` followed by the z-base-32 encoding of the two bytes `0xed 0x01` and then the 32 bytes of an Ed25519 public key."

### 10.13 (PARTLY) `WebRtc` mechanism is never published

Doc (line 2046, Node Record table): "`WebRtc` | Reachable over a WebRTC peer (`peer_id`). The Rust SDK client does not dial this mechanism."

Code: `EndpointMechanism::WebRtc` is defined (`crates/core/src/dht_registry/types.rs:61`). The only other mention in the workspace is the skip in the SDK (`crates/sdk/src/client.rs:404`). The substrate and the Iroh coordinator publish only `Iroh` mechanisms (`crates/substrate/src/runtime/publish.rs:186`; `crates/coordinator_iroh/src/coordinator.rs:492`).

Proposed text: add after the table: "No node publishes a `WebRtc` mechanism today. The variant is only defined."

### 10.28 (WRONG) `request_raw` is not a raw byte stream

Doc (line 2135, Application Interface): "`request` sends a JSON-RPC call. `request_raw` and `passthrough` give a raw byte stream."

Code: `request` builds a `JsonRpcRequest` and calls `request_raw` (`crates/sdk/src/client.rs:479-492`). `request_raw` takes a ready `JsonRpcRequest`, writes one framed request and returns a `JsonRpcResponse` (`client.rs:528-566`). `passthrough` is the byte-copy call: it copies bytes both ways between a local `TcpStream` and a new Iroh stream, and its preamble uses HTTP transport with the JSON-RPC protocol, not `raw://` (`client.rs:620-690`).

Proposed text: "`request` sends a JSON-RPC call built from a method name and parameters. `request_raw` sends a JSON-RPC request that the caller built and returns the JSON-RPC response. `passthrough` copies bytes both ways between a local TCP stream and a stream to the service."

### 10.29 (PARTLY) "A service never accepts connections itself"

Doc (line 2154, Transport Layer): "A service never accepts connections itself." (also line 2137: "A service does not accept connections.")

Code: this holds for a WASM service: the node accepts and calls it. A TCP or container service runs its own listener, and the `TcpProxy` stage opens a TCP connection to it (`crates/router/src/route_handler/io.rs:524-540`). Line 2152 of the same doc says this.

Proposed text: "A WASM service has no listen or accept call. The node accepts the stream and calls the service. A TCP or container service runs its own TCP listener, and the `TcpProxy` stage connects to it."

### 10.31 (PARTLY) `raw://` and `http://` handling

Doc (line 2141, Application Interface): "On a `raw://` stream, the substrate does not interpret or modify the data. On a `json-rpc://` or `http://` route, the router parses the JSON-RPC itself and applies the adaptation stage for the target."

Code: `raw://` to a TCP service is a plain byte copy (`io.rs:524-540`). `raw://` to a WASM component is a stream-protocol request: the router requires `dir=upload` or `dir=download`, reads one framed first message, and then gives the stream to the guest (`io.rs:549-587`). `raw://` to a native service is refused (`io.rs:552-555`). An `http://` stream to a TCP service is forced to raw transport and is not parsed (`crates/router/src/route_handler/dispatch.rs:366-370`). An `http://` stream to a WASM or native service goes through the HTTP handler: it serves blobs, assets and declared routes, and only then falls back to the JSON-RPC bridge (`crates/router/src/route_handler/http.rs:270-330`).

Proposed text: "On a `raw://` stream to a TCP service, the node copies bytes both ways. On a `raw://` stream to a WASM component, the node reads one framed first message and the `dir` parameter, then hands the stream to the guest. A `json-rpc://` route is parsed as JSON-RPC and gets the adaptation stage of its target. An `http://` route to a WASM or native service is parsed as HTTP: the node serves blobs, assets and declared routes, and treats the rest as JSON-RPC. An `http://` route to a TCP service is copied as bytes."

### 10.39 (PARTLY) `JsonRpcToWrpc` answers "not implemented yet"

Doc (line 2317, Protocol Adaptation): "`JsonRpcToWrpc`: reserved for wRPC. It answers "not implemented yet"."

Code: only a `match` arm uses the variant (`crates/router/src/route_handler/dispatch.rs:282-285`). `plan_pipeline` never builds it, and the code comment above the arm says so. A `wrpc://` stream gets `ServiceStage::UnsupportedProtocol` and the error "unsupported protocol ... this node speaks json-rpc/v1", code `-32091` (`dispatch.rs:286-295,348-355`).

Proposed text: "`JsonRpcToWrpc`: reserved for wRPC. The router never picks it today. A `wrpc://` stream gets the unsupported-protocol error (`-32091`)."

### 10.41 (PARTLY) "No gateway role exists"

Doc (line 2405, Routing Model, Envisioned): "No gateway role exists."

Code: `[roles.client_gateway]` exists (a local HTTP proxy; see the Architecture notes in `AGENTS.md` and `crates/core/src/config/roles.rs`). The Envisioned item is a gateway that routes inside a BLE or LoRa network, and none exists (`config.sample.toml:106-112` types are unread). The short sentence reads as if the node has no gateway role at all.

Proposed text: "No node role routes inside a BLE or LoRa network. (The `client_gateway` role is a different thing: a local HTTP proxy.)" Use the same wording in line 2212 if it is kept.

### 10.7 and 10.42 (CITE-OFF)

10.7: the 1000-byte check is in `pkarr` 6.0.1 (`signed_packet.rs:276`), the version the workspace uses (`Cargo.toml:142`). The row names 5.0.3, which only enters through another dependency in `Cargo.lock`. The check is the same in both.
10.42: for the "WebRTC relay" use `crates/coordinator_webrtc/src/bootstrap/tunnel.rs:170-232` (a browser tunnel to Iroh) and `crates/coordinator_webrtc/src/coordinator.rs:54-130`, not `bootstrap.rs:60-68` (the connection cache field). No doc change.

## Doc text not covered by any row

1. Line 2339 (Protocol Adaptation, Envisioned): "JSON-RPC is the wire protocol everywhere." HTTP routes, SSE and WebSocket bridges (`crates/router/src/route_handler/http.rs:270-330`), raw stream protocols (`io.rs:549-587`) and `TcpProxy` (`io.rs:524-540`) are not JSON-RPC. Suggest "JSON-RPC is the only RPC wire protocol".
2. Line 2508 (`[TOP-ROB]` Reactive Eviction): "the system traps the error, reactively evicts any localized references". Only the WebRTC bootstrap has a connection cache to evict (`crates/coordinator_webrtc/src/bootstrap/tunnel.rs:179-232`). Nothing else keeps connection references. Unproven as a general statement.
3. Line 2113 (Record visibility): "the node always publishes its own record". `register` returns an error when no registry URL is set and the DHT is off (`crates/core/src/dht_registry/client.rs:170-185`). "Always" holds only with a registry or the DHT enabled.
4. Lines 2255-2266 (Connection Establishment): the pseudo-code shows an unconditional second lookup. The second lookup runs only when the first record has `endpoint_type` `Service` (`client.rs:272-277`). A call to a node DID makes one lookup.
5. Lines 2255 and 1902: the SDK client cannot use the DHT alone. With an empty `registry_url` and no given mechanisms, `connect` fails at once (`crates/sdk/src/client.rs:357`). The DHT is only a second step after a configured registry.
6. Line 2167 (Path Construction): "Iroh itself chooses between a direct path and the relay." The SDK builds `Endpoint::empty_builder()` (relay disabled, no address lookup; iroh 0.97 `endpoint.rs:171`). It gets a relay only from the record's `relay_url` (`crates/sdk/src/client.rs:374-380`). The record holds only the node id (`publish.rs:168-176`). Without a `relay_url` the SDK has no address to dial.
7. Line 2089: a coordinator registers only when `community_registry_url` is also set (`crates/coordinator_iroh/src/coordinator.rs:149-151`).
8. Line 2506: "The router uses this loop when it forwards a stream" is right, but `IrohHop` (the proxy) is forced to one connect attempt (`crates/router/src/proxy/hop.rs:23-45`). The doc says this a few words later; the order may confuse a reader. Low priority.

## Cost notes

- Effort: one session, about 59 rows. A rough rate of 25 to 30 rows per hour, because most rows needed 2 to 4 files each.
- Quick rows (about 40): constants, enum names, config defaults, and negative greps (BLE, LoRa, `RollingBack`, `connection_cache`, `JsonRpcToWrpc`).
- Hard rows: 8.14 (a design sentence with no code behind "SynOrgs, directories and aggregators"), 8.17 and 10.33 (needed a search of the whole SDK for connection reuse and the iroh `empty_builder` default), 10.28 to 10.31 (the doc text said more than the row: the checks needed the real SDK and router paths), 10.7 (two `pkarr` versions in `Cargo.lock`).
- Audit notes in `fix-progress.md` for commit 8 and 10 were right: the code differs from audit CN-40 (second lookup only for `Service` records) and CN-18 (address pruned).
- Rows that rest on a code comment only: 8.15 (WebRTC cache reason), 10.23 (mainline rule, checked in `mainline-6.2.0`), 10.38 (negotiation deferred).
