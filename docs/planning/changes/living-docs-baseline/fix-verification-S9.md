# Fix verification, batch S9

Reader: the developer who applies the stage 2 corrections to `docs/system-architecture.md` and `docs/TERMINOLOGY.md`.

Scope: fix commits 21 (Layer 1), 22 (overview, entity model, Executive Summary), 23 (top-matter warning) and 24 (TERMINOLOGY.md, App Supervisor). 57 rows. Static reading only. Iroh crate source was read from `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/iroh-0.97.0` and `iroh-relay-0.97.0`.

## 1. Summary

| Verdict | Count |
|---|---|
| CONFIRMED | 46 |
| CITE-OFF | 2 |
| PARTLY | 9 |
| WRONG | 0 |
| UNVERIFIABLE | 0 |
| **Total** | **57** |

By commit: 21 = 32 CONFIRMED, 1 CITE-OFF, 5 PARTLY (38). 22 = 11 CONFIRMED, 1 CITE-OFF, 3 PARTLY (15). 23 = 1 CONFIRMED, 1 PARTLY (2). 24 = 2 CONFIRMED (2).

## 2. Rows

Short names: `cr` = `crates/router/src`, `ci` = `crates/coordinator_iroh/src`, `cw` = `crates/coordinator_webrtc`, `core` = `crates/core/src`.

### Commit 21 (Layer 1)

| Row | Verdict | Note | Checked at |
|---|---|---|---|
| 21.1 | CONFIRMED | Only `iroh` and `webrtc` start; no BLE/LoRa reader or crate anywhere. | `cr/connection_router.rs:113-133`; `core/config/base.rs:197-198` |
| 21.2 | CONFIRMED | Relay URL comes from `parent_coordinator.iroh.url`; record carries `Iroh { endpoint_addr_bytes, relay_url }`. | `crates/substrate/src/runtime/router.rs:220`; `.../publish.rs:162-189` |
| 21.3 | CONFIRMED | SDK builds `RelayMode::Custom` from the record URL. | `crates/sdk/src/client.rs:366-384` |
| 21.4 | CONFIRMED | Parsing URL swaps `Endpoint::builder(N0)` for `empty_builder()`; `Builder::empty` has no address lookup. | `cr/net_iroh.rs:99-110`; `iroh-0.97.0/src/endpoint.rs:171` |
| 21.5 | PARTLY | `server` feature and `enable_relay` gate are right. Relay alone is impossible. | `ci/coordinator.rs:120-127`; `Cargo.toml:194` |
| 21.6 | CONFIRMED | `access` default `"everyone"`; a list becomes `Restricted` over `EndpointId`. | `core/config/roles.rs:55-59`; `ci/config.rs:82-99` |
| 21.7 | PARTLY | Cert, key and QUIC are configured. HTTPS and HTTP listeners get the same address. No test. | `ci/config.rs:42-80`; `iroh-relay-0.97.0/src/server.rs:371,428` |
| 21.8 | CITE-OFF | True (Iroh relay forwards encrypted datagrams by endpoint id). Cited text is a code comment and a doc. | `iroh-relay-0.97.0/src/lib.rs:2-5`; `.../protos/relay.rs:127` |
| 21.9 | CONFIRMED | No `syneroym.net` relay name, TURN, relay list, cache or bootstrap server in code. | `core/protocol_utils.rs:211,395`; searches below |
| 21.10 | CONFIRMED | `relay_to_next_hop` is in the router; `new_coordinator` has an empty registry, no sandbox, no proxy. | `cr/route_handler/io.rs:471-514`; `cr/route_handler.rs:440-480`; `ci/coordinator.rs:225-247` |
| 21.11 | CONFIRMED | Registry miss calls `relay_to_next_hop`; substrate passes its Iroh endpoint to `RouteHandler::init`. | `cr/route_handler/io.rs:313-317`; `cr/connection_router.rs:98-106` |
| 21.12 | PARTLY | Config, publish and SDK chain is right. Test cited uses a coordinator entry point and blocks nothing. | `cr/connection_router.rs:77-92`; `crates/coordinator_iroh/tests/multi_hop_relay.rs:564-607` |
| 21.13 | PARTLY | Forwarding is right. "when a record ... names it" contradicts "no entry-point field". | `cr/route_handler/io.rs:471-514`; `cr/net_iroh.rs:129-146`; `core/dht_registry/types.rs:53-91` |
| 21.14 | PARTLY | Parent URL only sets the home relay (`_parent_relay_url` is unused). "Outbound-only" is wrong. | `ci/coordinator.rs:74-76,225-247`; `cr/route_handler.rs:106,458` |
| 21.15 | CONFIRMED | `EndpointInfo` and `EndpointMechanism` have no entry-point field. | `core/dht_registry/types.rs:53-91` |
| 21.16 | CONFIRMED | `enc: None` in SDK; browser adds `enc=ecdh-p256`; coordinator parses the preamble first. Preamble also holds delegation and UCAN. | `cr/preamble.rs:314`; `cw/templates/peer-proxy.js:587,944`; `crates/sdk/src/client.rs:661` |
| 21.17 | CONFIRMED | Routes `/sw.js`, `/__syneroym/peer-proxy.js`, `/__syneroym/tunnel`, plus fallback page; default `0.0.0.0:7962`. | `cw/src/bootstrap.rs:113-120`; `core/config/roles.rs:312-314` |
| 21.18 | CONFIRMED | SW posts the request on a `MessageChannel` to a window; page calls `handleSWRequest`. | `cw/templates/sw.js:53-99`; `cw/templates/peer-proxy.js:761-765,933` |
| 21.19 | CONFIRMED | Page registers at `/ws`, sends offer with `target`; substrate registers when `webrtc` is an interface and section exists. | `cw/src/signalling.rs:49-105`; `cr/connection_router.rs:113-125,274-280`; `cw/templates/peer-proxy.js:828,909` |
| 21.20 | CONFIRMED | Default STUN in config and `peer-proxy.js`; `stun_servers` fed to `RTCConfiguration`; no TURN. | `core/config/base.rs:170-172`; `cr/connection_router.rs:199-205`; `cw/templates/peer-proxy.js:868` |
| 21.21 | CONFIRMED | One `createDataChannel` per request; query string (not only `enc`) is cut off. | `cw/templates/peer-proxy.js:365-378` |
| 21.22 | CONFIRMED | Falls back to WS `/__syneroym/tunnel` and sends the preamble. | `cw/templates/peer-proxy.js:413-453,772-777` |
| 21.23 | CONFIRMED | Registry check, DID-derived Iroh address, `connect_iroh_stream`, blind pipe. | `cw/src/bootstrap/tunnel.rs:21-43,157-250` |
| 21.24 | CONFIRMED | Client verifies the server signature against the target key; payload is encrypted to the substrate. | `cw/templates/peer-proxy.js:303-363,428-440,474-500` |
| 21.25 | CONFIRMED | `iroh: Option` None; section default `http://localhost:7964`; `--iroh-relay-url` sets it. | `core/config/base.rs:148-162,194-199`; `crates/substrate/src/main.rs:141-144` |
| 21.26 | CONFIRMED | All five defaults match (`enable_bep0044_dht = !cfg!(test)`). | `core/config/base.rs:164-189,214-234` |
| 21.27 | CONFIRMED | All seven defaults match; info port is `http` port + 10 unless port is 0. | `core/config/roles.rs:272-306`; `ci/coordinator.rs:257-265` |
| 21.28 | CONFIRMED | `0.0.0.0:7963`, `0.0.0.0:7962`, `0.0.0.0:7961`; `access` `"everyone"`; `tls` None. | `core/config/roles.rs:24-26,55-59,252-256,309-335` |
| 21.29 | CONFIRMED | Over the cap: writes `ServiceUnavailable`, closes with 503. | `cr/route_handler.rs:497-541` |
| 21.30 | CONFIRMED | One spawned registration task (with retries) when both fields set. | `ci/coordinator.rs:149-167,349-389` |
| 21.31 | CONFIRMED | All listed fields are in `CoordinatorInfo`; TLS days come from `[tls]` (`config.tls`). | `ci/info_endpoint.rs:92-128`; `ci/coordinator.rs:301,319` |
| 21.32 | CONFIRMED | N0 preset = n0 relays + n0 DNS publish/resolve; parse failure keeps N0 and warns. | `cr/net_iroh.rs:94-110`; `iroh-0.97.0/src/endpoint/presets.rs:23-60` |
| 21.33 | CONFIRMED | `resolve_relay_url` returns `None`; WebRTC coordinator passes the optional parent URL only. | `ci/coordinator.rs:69-87`; `cw/src/coordinator.rs:71-72` |
| 21.34 | CONFIRMED | `iroh_endpoint` is `None` without `parent_coordinator.iroh`. | `cr/connection_router.rs:77-107` |
| 21.35 | CONFIRMED | SDK uses `Endpoint::empty_builder()`; `Builder::empty` is `RelayMode::Disabled`. | `crates/sdk/src/client.rs:374-381`; `iroh-0.97.0/src/endpoint.rs:171-172` |
| 21.36 | CONFIRMED | Listed settings have no reader outside config and `main.rs`; webrtc coordinator starts on the section. | `crates/coordinator/src/coordinator.rs:33-38`; `cw/src/coordinator.rs:58-63` |
| 21.37 | CONFIRMED | One pkarr packet per record; registry first, DHT second, DHT hit written back; 3600 s heartbeat. | `core/dht_registry/client.rs:150-166,195-270`; `core/dht_registry/types.rs:20,44` |
| 21.38 | CONFIRMED | No `syneroym-relays`, `governance`, relay list or cache in `crates/` or `apps/`. | search below; `core/config/base.rs:223` |

### Commit 22 (overview, entity model, Executive Summary)

| Row | Verdict | Note | Checked at |
|---|---|---|---|
| 22.1 | PARTLY | Registry half right. Iroh sends first via the relay; a coordinator can be the named entry point. Not "only fallback". | `iroh-0.97.0/src/lib.rs:58-67`; `crates/coordinator_iroh/tests/multi_hop_relay.rs:586-594`; `crates/community_registry/src/registry.rs:134-139` |
| 22.2 | CONFIRMED | No `reputation` or rating in code; trust records are credential and revocation. | `crates/roym_core/src/record.rs:18-31`; search below |
| 22.3 | CONFIRMED | One manifest only; six services (lines 9-98); SynOrg directory. | `crates/roym_core/app/roym.toml:5,9-98`; `crates/roym_core/src/directory.rs:1-5` |
| 22.4 | CONFIRMED | Matrix: x86_64 and aarch64 Linux, x86_64 Windows, macOS universal. | `.github/workflows/release.yml:15-31` |
| 22.5 | CONFIRMED | "android" only in an npm lock file of a test component. | search below |
| 22.6 | PARTLY | Wasmtime, Podman, router, SQLite, Iroh, WebRTC, registry, DHT, release targets all hold. "Key Stores (KEK, DEK, vault)" has two stores, not three names. | `crates/data_keystore/src/key_store.rs:29-31`; `crates/data_db/src/sqlite/provider.rs:281-285`; `crates/app_supervisor/src/keys.rs:300` |
| 22.7 | CONFIRMED | `directory` search, credential ops, transaction and payment records exist. | `crates/roym_directory/src/app/search_ops.rs:164,212`; `crates/roym_core/src/payment.rs:1-2` |
| 22.8 | PARTLY | Roym archive, relay URL, no WAL shipping, no bootstrap server hold. Other key backups exist. | `apps/roymctl/src/commands/roym/backup.rs:46-60`; `apps/roymctl/src/commands/identity.rs:133-145`; `apps/roymctl/src/commands/supervisor.rs:77-85` |
| 22.9 | CONFIRMED | Per-service `placement` overrides the manifest default; two-substrate e2e. | `crates/app_orchestration/src/models/service.rs:299-306`; `.../manifest.rs:20-24` |
| 22.10 | CONFIRMED | One `IrohParentConfig.url`; no Syneroym registration step. Iroh itself connects to it as home relay. | `core/config/base.rs:152-162`; `crates/substrate/src/runtime/publish.rs:181-186` |
| 22.11 | CONFIRMED | `ServiceType` has four kinds. Wasm and Container are sandboxed; TCP is external; NativeHost is in-process. | `crates/app_orchestration/src/models/service.rs:13-19`; `crates/control_plane/src/service/orchestration/lifecycle.rs:540-551` |
| 22.12 | CONFIRMED | `ServiceConfig.source` is the artifact; container `image` defaults to `source`. | `crates/app_orchestration/src/models/service.rs:149-153`; `crates/sdk/src/mapper.rs:264-272` |
| 22.13 | CONFIRMED | `Role { Consumer, Provider }` in the transaction receipt. | `crates/roym_core/src/transaction.rs:82-88` |
| 22.14 | CONFIRMED | Server `search` makes no outbound call. Outbound `CallTarget::Service` is only in the client half. | `crates/roym_directory/src/app/search_ops.rs:164-236`; `.../client_query.rs:135`; `.../held.rs:93` |
| 22.15 | CITE-OFF | True. Hub UI is served by `web` over HTTP routes; session subject is the owner DID. `keys.rs:16` is an import. | `crates/roym_core/app/roym.toml:9-27`; `crates/roym_web/src/app.rs:84-86` |

### Commit 23 (top matter)

| Row | Verdict | Note | Checked at |
|---|---|---|---|
| 23.1 | PARTLY | No wRPC wire exists. "JSON-RPC everywhere" ignores `raw://` streams, MQTT and typed WIT calls. | `crates/rpc/src/proxy.rs:17-19`; `cr/preamble.rs:24-25`; `cr/route_handler/dispatch.rs:341-344` |
| 23.2 | CONFIRMED | `-32091` for `wrpc://` on Wasm and native endpoints (test). A TCP endpoint is proxied instead. | `cr/route_handler/dispatch.rs:287-294,350-355`; `crates/router/tests/unsupported_protocol.rs:84-108` |

### Commit 24 (TERMINOLOGY.md, App Supervisor)

| Row | Verdict | Note | Checked at |
|---|---|---|---|
| 24.1 | CONFIRMED | Grant or `Open`; unknown app, ungranted caller and retired instance return one `resolve_denied` error. | `crates/app_supervisor/src/service.rs:753`; `.../service/resolve.rs:34-70,126-135`; `.../wit/supervisor/supervisor.wit:325` |
| 24.2 | CONFIRMED | `RegistryTopologyFetcher::fetch_via` sends `supervisor` / `resolve`. | `crates/sdk/src/topology.rs:119-128` |

Searches for negative rows (my own terms, `rg` over `crates/` and `apps/` unless noted):
- 21.1: `btleplug|bluer|lora|bluetooth|\bble\b`, `\.ble\b`, `\.lora\b`. Only config structs.
- 21.9, 21.38: `syneroym\.net`, `turn_server|turns?:|\bTURN\b|coturn`, `relay_list|relay_cache|syneroym-relays|governance|home_relay|relay_assign`, `bootstrap` (only WebRTC page, SDK and router comments about other things).
- 21.36: `enable_signalling|enable_relay|coordinator_discovery_url|bootstrap_url|transport_bridge`. Readers: only `enable_relay` of the Iroh coordinator.
- 22.2: `reputation`, `rating`, `score` in `crates/roym_*`. No hit.
- 22.5: `android|cargo-ndk` over the repo outside docs and lock files. Only an npm lock file.
- 22.8: `litestream|replication|replica|rqlite|raft|crsqlite`. Only `replicas` (supervisor members, no data copy).

## 3. Findings

### 21.5 (PARTLY)
Doc: "Relay Node Architecture", "The relay server and the Syneroym endpoint are separate: a coordinator can run either one or both."
Code: `enable_relay` is a field of `CoordinatorIrohConfig`. The relay spawns only if `role.iroh` exists (`crates/coordinator_iroh/src/coordinator.rs:120`). The Syneroym endpoint, router and `/v1/info` always start when `role.iroh` exists (`:124-146`). So the relay cannot run without the endpoint. The code comment at `:119` says "A, B, or both", but there is no path for the relay alone.
Fix: "The relay server and the Syneroym endpoint are separate parts. A coordinator with `[roles.coordinator.iroh]` always runs the Syneroym endpoint. It runs the relay server too only when `enable_relay = true`."

### 21.7 (PARTLY)
Doc: "Relay Node Architecture", "With `[roles.coordinator.tls]` set, the relay uses that certificate and key, and it also runs QUIC address discovery on `quic_bind_address`."
Code: `build_relay_config` loads the cert and key and builds `QuicConfig` (`crates/coordinator_iroh/src/config.rs:42-80`). It sets `TlsConfig.https_bind_addr` and `RelayConfig.http_bind_addr` to the same `http_bind_address` (`:63,103`). In iroh-relay, with TLS the HTTPS server binds `https_bind_addr` and a separate plain HTTP probe server binds `http_bind_addr` (`iroh-relay-0.97.0/src/server.rs:371,428`). Its doc says these "have to be on a different port" (`server.rs:187-190`). Reading suggests a fixed port fails with address in use. No test sets `role.tls` (`crates/coordinator_iroh/tests/tls_rotation.rs:84` leaves it unset). I could not run it.
Fix: keep the sentence, add: "No test covers relay TLS." Add a deferred-backlog row: relay TLS gives the HTTPS and HTTP probe listeners one address.

### 21.12 (PARTLY)
Doc: "Multi-Hop Relay", "A caller looks the record up in the registry and dials the substrate through that relay. This is the normal path. It is what lets a fully inbound-blocked substrate stay reachable."
Code: the chain config -> endpoint -> record -> SDK relay map is right (`cr/connection_router.rs:77-92`, `publish.rs:186`, `sdk/src/client.rs:343-384`). The cited test dials coordinator C with `new_with_mechanisms` (`multi_hop_relay.rs:586-594`), not a registry lookup. All nodes run on localhost, so no inbound block is tested. "Stays reachable" is an Iroh relay property, not shown by this repo.
Fix: "A caller that looks the record up in the registry dials the substrate through that relay. An Iroh relay is made so that a peer that accepts no inbound connection can still be reached. No test in this repository blocks inbound traffic."

### 21.13 (PARTLY)
Doc: "Multi-Hop Relay", "A coordinator is the entry point only when a record or an SDK call names it."
Code: only an SDK call can name it (`SyneroymClient::new_with_mechanisms`, test `multi_hop_relay.rs:586-594`). A record cannot name an entry point (`core/dht_registry/types.rs:53-91`), and the next paragraph of the doc says so. The one record that names a coordinator is the coordinator's own record (`ci/coordinator.rs:149-166`).
Fix: "A coordinator is the entry point only when an SDK call names it, or when a caller dials the record of the coordinator itself."

### 21.14 (PARTLY)
Doc: "Multi-Hop Relay", "A coordinator that has a parent is outbound-only. It keeps one connection to the relay of that parent, and it does not connect to the parent on demand."
Code: the parent URL becomes the relay of the coordinator's own Iroh endpoint (`ci/coordinator.rs:74-76,205-213`). `RouteHandler` stores it as `_parent_relay_url` and never reads it (`cr/route_handler.rs:106,458`), so nothing dials the parent on demand. Iroh keeps a home relay connection (`iroh-0.97.0/src/lib.rs:60-63`). But the coordinator is not "outbound-only". It accepts inbound Iroh streams (`ci/coordinator.rs:244-245`) and opens a new connection to each target substrate (`io.rs:493`).
Fix: "A coordinator that has a parent uses the relay of that parent as its own relay. No code dials the parent on demand. The coordinator still accepts inbound streams and opens a connection to each target substrate."

### 22.1 (PARTLY)
Doc: "Executive Summary", "Relays and coordinators carry traffic only as a fallback when no direct path exists".
Code: Iroh connects through the home relay first, then tries to go direct (`iroh-0.97.0/src/lib.rs:58-66`). A coordinator forwards when an SDK call names it as entry point, with no direct-path test (`multi_hop_relay.rs:586-594`). The doc's own Multi-Hop section calls the relay path "the normal path" for a private substrate.
Fix: "A direct connection between two participants needs no server in the data path. An Iroh relay helps two peers connect and carries their traffic when no direct path exists. A coordinator forwards traffic only when a caller names it as the entry point or a browser falls back to the tunnel. Registries store signed endpoint records and answer lookups."

### 22.6 (PARTLY)
Doc: "System Layers Overview", diagram node `K["Key Stores (KEK, DEK, vault)"]`.
Code: KEK and DEK are one store (`KeyStore` holds the KEK; DEKs live in `dek_store`, `crates/data_keystore/src/key_store.rs:29-31`). "vault" is two things: the supervisor key vault (`crates/app_supervisor/src/keys.rs:300`) and the `_vault` secret rows in each service database (`crates/data_db/src/sqlite/provider.rs:287-290`). The Internal Architecture diagram (line 445) lists "KEK/DEK key store, supervisor key vault".
Fix: `K["Key Stores (KEK and DEK, supervisor key vault)"]`. The remaining claims in the row hold.

### 22.8 (PARTLY)
Doc: "System Layers Overview (note)", "Today the built backup is the Roym archive".
Code: the Roym archive is `roymctl roym backup create` (`apps/roymctl/src/commands/roym/backup.rs:46-60`). Key backups also exist: `roymctl identity export` under a recovery key (`identity.rs:133-145`) and `roymctl supervisor export-master` (`supervisor.rs:77-85`). The doc says so at line 516 and 1592.
Fix: "Today the built data backup is the Roym archive. Keys have their own backups: `roymctl identity export` and `roymctl supervisor export-master`."

### 23.1 (PARTLY)
Doc: "Implementation Note", "JSON-RPC 2.0 is the wire protocol everywhere today, between components and on the external API surface."
Code: there is no wRPC wire (`crates/rpc/src/proxy.rs:17-19`). But `raw://` streams carry raw bytes with no JSON-RPC (`cr/preamble.rs:24-25`, `dispatch.rs:341-344`). A TCP endpoint is a byte proxy (`dispatch.rs:345-347`). The substrate also runs an MQTT broker (`cr/route_handler.rs:462-464`). A guest calls its host through typed WIT imports, not JSON-RPC (`crates/wit_interfaces/wit/data-layer/data-layer.wit`).
Fix: "JSON-RPC 2.0 is the only RPC wire protocol today. Raw byte streams (`raw://`), TCP proxies and MQTT messaging exist beside it. A guest calls its host through typed WIT imports."

### 8 and 15: CITE-OFF
- 21.8: use `iroh-relay-0.97.0/src/lib.rs:2-5` and `.../protos/relay.rs:127` (datagram has `dst_endpoint_id`). The cited comment `coordinator.rs:105-111` is the authors' note.
- 22.15: use `crates/roym_core/app/roym.toml:9-27` and `crates/roym_web/src/app.rs:84-86`.

## 4. Doc text not covered by any row

1. `P2P Networking: Iroh`: "Iroh tries a direct path first and uses a relay when it needs one." Iroh first connects through the home relay, then moves to a direct path (`iroh-0.97.0/src/lib.rs:58-66`). Say "uses the relay first and moves to a direct path when it finds one".
2. `Browser Path`: "The two paths share no code with the Iroh forwarding above." The tunnel uses `syneroym_router::net_iroh::IrohStream` (`crates/coordinator_webrtc/src/bootstrap.rs:36`) and `net_iroh::build_iroh_endpoint` (`.../coordinator.rs:72`). The data channel path ends in the substrate `RouteHandler` (`cr/connection_router.rs:123-125`). Only `relay_to_next_hop` is not shared.
3. `Relay and Registry Configuration`, table row `access`: a string other than `"everyone"` silently means everyone (`ci/config.rs:98`). Say so.
4. Same table, "Default" column: the values are the config-type defaults. With no `--config`, `run` uses the dev setup: `parent_coordinator.iroh` is present and `registry_url` is `http://localhost:7961` (`crates/substrate/src/main.rs:226-232`). The table says "absent" and "None".
5. `Multi-Hop Relay`: "It sees the target service id and the caller's public key." The preamble can also carry the delegation certificate and the capability token (`cr/preamble.rs:176-190`; `crates/sdk/src/client.rs:661-662`). The `pubkey` field is only filled by some callers.
6. `Bootstrap Server & DHT Fallback`, Today paragraph: a record marked `is_private` is not published to the DHT (`core/dht_registry/client.rs:150-151`). A bad record from the registry stops the lookup with no DHT fallback (`:212-215`). DHT lookup needs a full DID (`:242-260`).
7. `Conceptual Entity Model`: "A TCP service ... run without [a sandbox]". The TCP process runs outside the substrate (`lifecycle.rs:544-547`). A native-host service cannot be put in a deployment plan (`crates/sdk/src/mapper.rs:320-322`).
8. `Conceptual Entity Model`: "named in the `source` field". For a container, `custom_config.image` overrides `source` (`crates/sdk/src/mapper.rs:264-272`).
9. `Relay Node Architecture`: relay TLS finding for 21.7 also affects the sentence "With `[roles.coordinator.tls]` set ..." in the config table. See Finding 21.7.

## 5. Cost notes

- 57 rows. About 3 hours of reading, so roughly 19 rows per hour. Commit 21 took most of it (38 rows, many dense config rows).
- Fast rows: the config defaults (21.25 to 21.28), and commits 22 and 24. Each is one struct or one function.
- Hard rows: 21.7 (needed iroh-relay source to see the double bind), 21.12 to 21.14 (the test does not match the claim, and "outbound-only" needed the router code), 22.6 (a diagram label needed three crates), 23.1 (a negative claim about "everywhere": needed a search for raw, MQTT and WIT paths).
- Method that helped: read the iroh and iroh-relay crate source for every claim that says "Iroh does X".
