# Fix verification, batch S4 (fix commits 11 and 12)

Reader: the maintainer of `docs/system-architecture.md`.
Scope: 63 rows of `fix-new-claims.md` (31 for commit 11, 32 for commit 12).
Method: static reading only. Every cite was opened. Negative claims were searched again with other terms.

## Summary

| Verdict | Count |
| --- | --- |
| CONFIRMED | 58 |
| CITE-OFF | 0 |
| PARTLY | 5 |
| WRONG | 0 |
| UNVERIFIABLE | 0 |
| Total | 63 |

Commit 11: 29 CONFIRMED, 2 PARTLY (11.21, 11.24). Commit 12: 29 CONFIRMED, 3 PARTLY (12.6, 12.27, 12.31).

## Rows

| Row | Verdict | Note | Checked at |
| --- | --- | --- | --- |
| 11.1 | CONFIRMED | Edition 2024 in workspace package; channel `stable`. | `Cargo.toml:50`; `rust-toolchain.toml:2` |
| 11.2 | CONFIRMED | iroh, iroh-base, iroh-relay all 0.97.0 in lock; zero `iroh-net` packages. | `Cargo.toml:192-194`; `Cargo.lock:3379,3432,3485` |
| 11.3 | CONFIRMED | `webrtc = "0.17"` (lock 0.17.2); used by the router data channel code. | `Cargo.toml:110`; `crates/router/Cargo.toml:43`; `crates/router/src/net_webrtc.rs:15` |
| 11.4 | CONFIRMED | wasmtime and wasmtime-wasi 46.0.2 (lock 46.0.3), component-model feature, `wasmtime_wasi::p2`. | `Cargo.toml:197-198`; `crates/sandbox_wasm/src/engine.rs:72` |
| 11.5 | CONFIRMED | Engine runs `Command::new(podman_path)`; no version or rootless check; dev guide says rootless is advice. | `crates/sandbox_podman/src/engine.rs:28,266-275`; `docs/developer-guide.md:561` |
| 11.6 | CONFIRMED | WIT dir has host/guest worlds; preamble grammar and Roym `Request`/`Response` envelope exist. | `crates/wit_interfaces/wit/`; `crates/router/src/preamble.rs:1-38`; `crates/roym_core/src/envelope.rs:1-25` |
| 11.7 | CONFIRMED | `wit_bindgen::generate!` with serde derives; dispatch is a hand-written `match` on method name. | `crates/wit_interfaces/src/control_plane.rs:5-9`; `crates/control_plane/src/service/dispatch.rs:6-25` |
| 11.8 | CONFIRMED | HTTP registry tried first, DHT only if no result; pkarr 6.0 (lock 6.0.1). | `crates/core/src/dht_registry/client.rs:195-262`; `Cargo.toml:142` |
| 11.9 | CONFIRMED | `metrics` 0.23 (lock 0.23.1), `MemoryRecorder`, JSON snapshot on configured endpoint. | `Cargo.toml:128-131`; `crates/observability/src/recorder.rs:26`; `crates/substrate/src/runtime/services.rs:385-411` |
| 11.10 | CONFIRMED | `OtlpConfig` is config only; engine header has TODO; no `opentelemetry` Rust crate in lock. | `crates/core/src/config/roles.rs:477-505`; `crates/observability/src/engine.rs:1-8` |
| 11.11 | CONFIRMED | `toml::from_str` into typed config, bail checks in code; `jsonschema` only in manifest and fdae. | `crates/substrate/src/main.rs:90-92,127-133`; `Cargo.toml:119` |
| 11.12 | CONFIRMED | Roym build task runs `cargo component build --target wasm32-wasip2`; wit-bindgen 0.57 (lock 0.57.1). | `mise.toml:76`; `Cargo.toml:199` |
| 11.13 | CONFIRMED | `run_args.push(container_manifest.image)`; no base-image rule. | `crates/sandbox_podman/src/engine.rs:260` |
| 11.14 | CONFIRMED | vodozemac 0.10 Olm account and session in conversation crypto. | `Cargo.toml:162`; `crates/conversation/src/crypto.rs:5-28` |
| 11.15 | CONFIRMED | `Aes256Gcm` seal/open with epoch key; owner-only rekey with fresh random key; no MLS crate in lock. | `crates/conversation/src/dag.rs:260-290`; `crates/conversation/src/group.rs:264,294,585` |
| 11.16 | CONFIRMED | Directory signs credentials as an `Envelope` via `sign_as_synorg`. | `crates/roym_directory/src/app/credential_ops.rs:11-60,173`; `crates/signed_record/src/envelope.rs:146-182` |
| 11.17 | CONFIRMED | Payment request and acknowledgement payloads exist; no payment processing code or gateway crate. | `crates/roym_core/src/payment.rs:1-2,49-58,94-121` |
| 11.18 | CONFIRMED | No `ssi*`, `shaka*` or `stripe*` package in lock or manifests; searched `Cargo.toml`, `package.json` files. | `Cargo.lock`; `crates/roym_web/ui/package.json:1-25` |
| 11.19 | CONFIRMED | Vite/TS UI; manifest names `bundle.tar.gz` archive; mise task packs `dist`. | `crates/roym_web/ui/package.json:6-18`; `crates/roym_core/app/roym.toml:29-30`; `mise.toml:62-68` |
| 11.20 | CONFIRMED | `apps/` has only `roymctl`; no tauri, swift, kotlin or webview file or manifest entry. | `apps/` listing; repo-wide search |
| 11.21 | PARTLY | WebCrypto code is real, but it runs only on the WebSocket tunnel fallback, not on the WebRTC data channel. | `crates/coordinator_webrtc/templates/peer-proxy.js:303-350,376-377,428-440` |
| 11.22 | CONFIRMED | mise installs `stable` and `nightly-2026-04-06`; verify gate runs fmt on that nightly. | `mise.toml:8`; `xtask/src/verify.rs:32` |
| 11.23 | CONFIRMED | `cargo:cargo-component = "0.21"`; used in build tasks. | `mise.toml:10,76` |
| 11.24 | PARTLY | No wit-bindgen CLI in mise; crate is 0.57. But `dual-build-fixture` in `test-components` uses workspace 0.57, not 0.55.0. | `mise.toml:8-26`; `test-components/greeter/Cargo.toml:9`; `test-components/dual-build-fixture/Cargo.toml:20` |
| 11.25 | CONFIRMED | `cargo:wasm-tools = "latest"`. | `mise.toml:9` |
| 11.26 | CONFIRMED | Compose runs `ghcr.io/syneroym/syneroym-substrate`, command `run --config`; ports for registry, Iroh. | `deploy/docker-compose.community.yml:3-14,40` |
| 11.27 | CONFIRMED | All six cargo tools and `node = "20"` listed; dupes pinned 0.2.1. | `mise.toml:10-26` |
| 11.28 | CONFIRMED | Playwright 1.60.0, TypeScript 5.9.3 in e2e; Vite and Vitest in UI package. | `crates/substrate/tests/e2e/package.json:9-13`; `crates/roym_web/ui/package.json:6-17` |
| 11.29 | CONFIRMED | Gates match the doc list. Doc omits the Python planning-refs gate and the docs-only skip (incomplete, not false). | `mise.toml:134-142`; `xtask/src/verify.rs:30-110` |
| 11.30 | CONFIRMED | All 12 groups present; `substrate` has `alias = "node"`. | `apps/roymctl/src/commands.rs:42-125` |
| 11.31 | CONFIRMED | No `otelcol`, collector or litestream outside `docs/`. | repo-wide grep (excluding `docs`, `target`, `node_modules`) |
| 12.1 | CONFIRMED | `relay_to_next_hop` at io.rs:471; `new_coordinator` uses mock registry, no sandbox, no proxy. | `crates/router/src/route_handler/io.rs:471-515`; `crates/router/src/route_handler.rs:440-486` |
| 12.2 | CONFIRMED | Local miss falls to relay; needs an Iroh endpoint (else error), which a substrate has only with `parent_coordinator.iroh`. | `crates/router/src/route_handler/io.rs:310-317,480-486`; `crates/router/src/connection_router.rs:77-92` |
| 12.3 | CONFIRMED | Relay server spawned if `enable_relay`; `RouteHandler::new_coordinator` accepts the ALPN. | `crates/coordinator_iroh/src/coordinator.rs:120-121,180,225-243` |
| 12.4 | CONFIRMED | DHT client only if flag on; DHT publish skipped for private records and only after HTTP registry success. | `crates/core/src/dht_registry/client.rs:100-160`; `crates/core/src/config/base.rs:224,232` |
| 12.5 | CONFIRMED | `register_endpoint` calls `propagate_registration` to parent for non-private records. | `crates/community_registry/src/registry.rs:226-229,287-304` |
| 12.6 | PARTLY | Parent relay is used and the 30 s wait exists. But the wait always runs, with or without a parent, and a timeout only warns. | `crates/coordinator_iroh/src/coordinator.rs:63-80,196-217` |
| 12.7 | CONFIRMED | `parent_registry_url` read at init, used only in propagate step. | `crates/community_registry/src/registry.rs:90-99,226-229` |
| 12.8 | CONFIRMED | Registry client built from `substrate.registry_url`; endpoint built from `parent_coordinator.iroh.url`. | `crates/substrate/src/runtime/router.rs:191-198,220`; `crates/router/src/connection_router.rs:77-92` |
| 12.9 | CONFIRMED | `coordinator_discovery_url` only at declaration and default; searched all non-doc files. | `crates/core/src/config/base.rs:223,232`; `crates/core/src/dht_registry/types.rs:48-51` |
| 12.10 | CONFIRMED | `Publication::Public(record)` splits into serialized `registry_certificate` in the manifest; record signed by master, empty mechanisms. | `crates/sdk/src/client/orchestrator.rs:43-50`; `crates/sdk/src/types.rs:34-69`; `crates/sdk/src/deploy/certify.rs:151-170` |
| 12.11 | CONFIRMED | Deploy calls `publish_service`; heartbeat (3600 s) replays all stored records, only when the node has an Iroh address. | `crates/control_plane/src/service/orchestration/deploy.rs:160-166`; `crates/substrate/src/runtime/publish.rs:29-67,77-130` |
| 12.12 | CONFIRMED | Self-register with 30 retries, then `publish_all_services`; `Ok(false)` when no record. | `crates/substrate/src/runtime/publish.rs:29-67`; `crates/core/src/endpoint_publisher.rs:50-60` |
| 12.13 | CONFIRMED | Registration needs `share_in_registry` and `community_registry_url`; spawned once with `retry_with_backoff`. | `crates/coordinator_iroh/src/coordinator.rs:149-165,349-379` |
| 12.14 | CONFIRMED | One `POST /register` to the single parent per accepted non-private record; no retry. | `crates/community_registry/src/registry.rs:226-229,287-304` |
| 12.15 | CONFIRMED | Service record has `substrate_id`, no mechanisms; lookup with `resolve` copies substrate mechanisms; SDK dials relay. | `crates/sdk/src/deploy/certify.rs:158-165`; `crates/core/src/dht_registry/client.rs:272-276`; `crates/sdk/src/client.rs:346-390` |
| 12.16 | CONFIRMED | Nickname `coordinator-{first 8}`; `EndpointInfo` has no entry-point or topology field. | `crates/coordinator_iroh/src/coordinator.rs:487-497`; `crates/core/src/dht_registry/types.rs:48-62` |
| 12.17 | CONFIRMED | `invoke_remote` resolves and dials via `IrohHop`; SDK `connect` for outside callers. | `crates/router/src/proxy/router.rs:298-304`; `crates/router/src/proxy/hop.rs:48-80`; `crates/sdk/src/client.rs:338-361` |
| 12.18 | CONFIRMED | SDK `lookup(target, true)`; resolve follows `substrate_id` on same client. | `crates/sdk/src/client.rs:346-352`; `crates/core/src/dht_registry/client.rs:272-276` |
| 12.19 | CONFIRMED | Preamble struct has service_id, pubkey, delegation, ucan; SDK sets pubkey and ucan. | `crates/router/src/preamble.rs:171-190`; `crates/sdk/src/client.rs:517-519` |
| 12.20 | CONFIRMED | Handshake stage is applied after lookup and before dispatch, only for `Some("ecdh-p256")`. | `crates/router/src/route_handler/io.rs:306-330`; `crates/router/src/route_handler/dispatch.rs:322` |
| 12.21 | CONFIRMED | Empty mock registry misses, lookup, `connect_with_retry`, preamble forwarded, `copy_bidirectional`; test drives it. | `crates/router/src/route_handler/io.rs:310-317,471-515`; `crates/coordinator_iroh/tests/multi_hop_relay.rs:565-607` |
| 12.22 | CONFIRMED | Test hands SDK the coordinator address; `EndpointInfo` has no entry-point field. | `crates/coordinator_iroh/tests/multi_hop_relay.rs:586-594`; `crates/core/src/dht_registry/types.rs:48-62` |
| 12.23 | CONFIRMED | `lookup_endpoint` returns `NOT_FOUND`; parent URL used only at lines 226,229,290. | `crates/community_registry/src/registry.rs:306-314` |
| 12.24 | CONFIRMED | `invoke_remote` dials resolved address with own endpoint; any resolve error maps to `ServiceNotFound`; `_parent_relay_url` never read. | `crates/router/src/proxy/router.rs:298-304`; `crates/router/src/route_handler.rs:415,458` |
| 12.25 | CONFIRMED | Each relayed stream does `connect_with_retry`; no pool. Outbound test via Cp. | `crates/router/src/route_handler/io.rs:471-515`; `crates/coordinator_iroh/tests/multi_hop_relay.rs:610-650` |
| 12.26 | CONFIRMED | Same evidence as 12.23 and 12.24. | `crates/router/src/route_handler.rs:415,458`; `crates/community_registry/src/registry.rs:306-314` |
| 12.27 | PARTLY | `enc` is never set in Rust and the browser does set it, but over a WebRTC data channel it strips the query before sending. | `crates/coordinator_webrtc/templates/peer-proxy.js:376-377,587,944`; `crates/router/src/preamble.rs:314,346` |
| 12.28 | CONFIRMED | Server decodes `pubkey`, signs server key + client key (130 bytes) with identity, AES-256-GCM; browser verifies. | `crates/router/src/route_handler/encryption.rs:278-328`; `crates/coordinator_webrtc/templates/peer-proxy.js:303-361` |
| 12.29 | CONFIRMED | `SYNEROYM_ALPN = b"syneroym/0.1"`; coordinator reads preamble before any decrypt. | `crates/router/src/connection_router.rs:48,154-155`; `crates/router/src/route_handler/io.rs:302-317` |
| 12.30 | CONFIRMED | `relay_to_next_hop` returns before the encryption stage; `copy_bidirectional` on raw bytes. | `crates/router/src/route_handler/io.rs:317,502-512` |
| 12.31 | PARTLY | JSON-RPC 2.0 is the only implemented wire protocol; but `raw://` streams and WebSocket frames also pass this path and are not JSON-RPC. | `crates/router/src/route_handler/dispatch.rs:283,291`; `crates/router/src/preamble.rs:12-17` |
| 12.32 | CONFIRMED | Server signs only; client key is an input to the signed payload, not signed itself. | `crates/router/src/route_handler/encryption.rs:312-328` |

## Findings

### 11.21 (PARTLY)

Doc words: "Consumer Frontend" table, row "Client-side crypto": "Built: browser WebCrypto (P-256 ECDH, Ed25519 verify) for the WebRTC end-to-end handshake."

Code: `connectTunnel` sends the preamble over a WebRTC data channel with the query removed (`preamble.split('?')[0]`), with the comment "WebRTC data channels are already DTLS encrypted, no ECDH is needed" (`peer-proxy.js:376-377`). The ECDH handshake and `verifyAndDeriveSharedSecret` run only in the WebSocket tunnel fallback (`peer-proxy.js:428-440,482`). The section "Browser Path" already says this (architecture line 341).

Proposed fix: "Built: browser WebCrypto (P-256 ECDH, Ed25519 verify) for the end-to-end handshake on the WebSocket tunnel path. A WebRTC data channel does not run it."

### 11.24 (PARTLY)

Doc words: Developer Toolchain, `wit-bindgen` crate: "The `test-components` guests pin 0.55.0."

Code: 11 test components pin `wit-bindgen = "0.55.0"`, but `test-components/dual-build-fixture/Cargo.toml:20` uses `wit-bindgen.workspace = true` (0.57).

Proposed fix: "Most `test-components` guests pin 0.55.0. `dual-build-fixture` uses the workspace version."

### 12.6 (PARTLY)

Doc words: Appendix step 1.2: "Cp connects outbound to its parent coordinator. When it has one (`parent_coordinator.iroh.url`), its Iroh endpoint uses the parent's relay as its home relay and, at startup, waits up to 30 seconds for the endpoint to come online."

Code: `build_iroh_endpoint` always waits `timeout(30s, endpoint.online())`, with or without a parent (`coordinator.rs:214`). On timeout it logs a warning and startup goes on (`:216`). Only the home relay choice depends on the parent (`resolve_relay_url`, `:63-80`).

Proposed fix: "With a parent (`parent_coordinator.iroh.url`), its Iroh endpoint uses the parent's relay as home relay. At startup it always waits up to 30 seconds for the endpoint to come online. If the wait ends, it logs a warning and continues."

### 12.27 (PARTLY)

Doc words: Data Transfer, step 1: "The handshake runs only when the caller asks for it with `enc=ecdh-p256` in the preamble. The Rust client (`SyneroymClient`) never sets it. The browser does."

Code: The page builds the preamble with `enc=ecdh-p256` (`peer-proxy.js:587,944`). On a WebRTC data channel it sends `preamble.split('?')[0]`, so `enc` is dropped (`:376-377`). Only the WebSocket tunnel keeps `enc` and replaces `pubkey` (`:428-440`).

Proposed fix: "The browser sets it on the WebSocket tunnel path. On a WebRTC data channel the page removes it, because DTLS already protects that channel."

### 12.31 (PARTLY)

Doc words: Data Transfer, step 2: "the application payload (JSON-RPC 2.0 frames) is encrypted at the caller and decrypted only at Sz".

Code: wRPC is not implemented (`dispatch.rs:283-291`, `lib.rs:3-4`). But the preamble also allows `raw://` byte streams, and the browser sends HTTP and WebSocket bytes through the same tunnel with `enc`. Those are not JSON-RPC frames (architecture line 500 says WebSocket frames are not JSON-RPC).

Proposed fix: "the application payload is encrypted at the caller and decrypted only at Sz. For a JSON-RPC route the payload is JSON-RPC 2.0 frames. wRPC is not implemented."

## Doc text not covered by any row

1. Appendix step 2.3, "Cp Registration": "It does this once at startup, with retries". True (`coordinator.rs:349-379`), but the registry deletes any entry not refreshed within 2 hours (`registry.rs:143-160`, `DEFAULT_REGISTRY_TTL_SECS = 7200` at `types.rs:16`; Cp's record has `ttl: None`, `coordinator.rs:487-497`). So Cp's record disappears from the registry about 2 hours after start. The doc does not say so.
2. Appendix step 4.3: "If the lookup finds no record, the call fails with a service-not-found error." `RegistryClient::lookup` also falls back to the DHT when `enable_bep0044_dht` is on (`client.rs:236-262`), which is on by default (`base.rs:232`). The sentence should say "finds no record in the registry or the DHT".
3. Appendix step 2.1 and Entities: "A node also publishes its public records to the BEP 0044 DHT". DHT publish runs only after the HTTP registry accepted the record, when one is configured (`client.rs:107-110`). If the registry is down, no DHT publish happens.
4. Consolidated Technology Stack, Developer Toolchain, `roymctl` row: ends with "and Roym backups". The `roym` group also has `enrol-signing`, `signing-status`, `address`, `directory`, `transaction` and `group` (`apps/roymctl/src/commands/roym.rs:40-110`). The row understates it.
5. Developer Toolchain, `mise run verify` row: lists the gates but omits the Python `planning-refs` gate and that nextest, doctests and e2e are skipped when only docs change (`xtask/src/verify.rs:76-86,151-156`). "The xtask checks" is not exact for the Python gate.
6. Developer Toolchain, `wasm-tools`: "Optional, for inspecting components". The only use in the repo is a comment (`crates/router/tests/proxy_dispatch.rs:71`). The purpose is not shown by any task. Harmless, but unproven.
7. Appendix step 1.2: "Cp connects outbound to its parent coordinator". The code only points Cp's Iroh endpoint at the parent's relay URL (`coordinator.rs:196-217`). There is no coordinator-to-coordinator call. Say "uses the parent's relay".
8. Appendix step 3 (Sz side): a substrate can forward only when it has an Iroh endpoint. That needs `"iroh"` in `communication_interfaces` and `parent_coordinator.iroh` set (`connection_router.rs:77-92`). Without them, `relay_to_next_hop` returns "No Iroh endpoint configured for relay forwarding" (`io.rs:489-492`). The intro sentence (row 12.2) omits this condition.

## Cost notes

- About 63 rows in one pass. Rate was roughly 40 rows per hour of effort.
- Easy rows: all version and tool rows in commit 11 (one grep of `Cargo.toml`, `Cargo.lock` or `mise.toml` each).
- Hard rows: 11.21 and 12.27 (needed the whole `peer-proxy.js` flow to find that the data-channel path drops `enc`), 12.6 (needed the full `build_iroh_endpoint`), 12.31 (scope of "payload frames").
- Cross-checks that paid off: reading `connectTunnel` in `peer-proxy.js` (found two findings), reading registry TTL sweep (found the Cp expiry gap), reading `RegistryClient::register` (found the DHT-after-HTTP order).
- Cites were good. No CITE-OFF row. A few were off by 5-15 lines (12.3, 12.11, 12.14).
