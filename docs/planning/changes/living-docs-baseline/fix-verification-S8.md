# Fix verification, batch S8

Reader: the author who applies the stage 2 fixes to `docs/system-architecture.md`.

Batch S8 covers fix commit 20 (Layer 2: node ownership, client gateway and auth service, failure and shutdown, upgrade and versioning, limits, deployment profiles; 50 rows). All work was static reading. Doc line numbers are those of `docs/system-architecture.md`. Row ids are the n-th row with first cell `20` in `fix-new-claims.md`.

## Summary

| Verdict | Count |
| --- | --- |
| CONFIRMED | 45 |
| CITE-OFF | 1 |
| PARTLY | 4 |
| WRONG | 0 |
| UNVERIFIABLE | 0 |
| Rows | 50 |

Negative claims repeated with other search terms, all held:

- No SIGTERM handler: searched `SIGTERM`, `SIGINT`, `SignalKind`, `signal_hook`, `ctrlc`, `libc::SIG`, `tokio::signal`, `unix::signal` over `crates/`, `apps/`, `xtask/`. Only `ctrl_c` (`crates/substrate/src/runtime.rs:48`) and `SIGUSR1` (`crates/core/src/tls.rs:31-32`) exist.
- `profiles` table read by nothing: searched `profiles`, `.profile`, `ProfileConfig`, `config.profile`, and `[profiles.` in TOML/docs. `ProfileConfig` is only declared (`crates/core/src/config/base.rs:210`) and held as a field (`config.rs:72`). `config.profile` is only logged (`runtime.rs:126`, `services.rs:277`) or set from `--profile` (`main.rs:99-101`).
- No manifest field selects `Sharded`: searched `Sharded`, `TopologyMode::`, `sharding_strategy`, `topology_mode`. Only tests set `Sharded` (`topology_document.rs:322`, `app_supervisor/src/topology.rs:413`). The compiler picks `Redundant` or the default (`compiler.rs:196-200`).
- No caller proof over the preamble: searched `challenge`, `sign(`, `verify(`, `remote_id` in `crates/router/src`. `connection.remote_id()` is only logged (`route_handler.rs:525`). The SDK binds its Iroh endpoint with no secret key (`crates/sdk/src/client.rs:374`).
- No WAL on service databases: searched `journal_mode`, `wal`, `pragma_update`, `execute_batch` in `crates/data_db/src`. Other node-owned files do set WAL (see "Doc text not covered").
- No peer-address check in the gateway: searched `peer_addr`, `remote_addr`, `is_loopback`, `allowlist` in `crates/client_gateway/src`. `peer_addr` is only logged (`gateway.rs:202-207`).

## Rows

| Row | Verdict | Note | Checked at |
| --- | --- | --- | --- |
| 20.1 | CONFIRMED | Iroh accept and WebRTC both call `handle_stream`, which first reads and verifies the preamble. No MQTT listener is bound. | `crates/router/src/connection_router.rs:48,154-155`, `route_handler.rs:525-554`, `route_handler/io.rs:294,358-420` |
| 20.2 | CONFIRMED | `verify` checks signature, window, scope in `TRANSPORT_SCOPES`; then key match and master revocation. | `crates/router/src/handshake.rs:26-80`, `crates/identity/src/delegation.rs:30,203-280` |
| 20.3 | CONFIRMED | No cert: preamble key becomes master; no pubkey gives anonymous. No proof of the private key anywhere. | `crates/router/src/handshake.rs:76-81`, `route_handler/io.rs:393-427`, `route_handler/http/auth.rs:11-17` |
| 20.4 | CONFIRMED | Five services call `require_internal` in `invoke`; `directory` calls `admit`. `-32013` and the three origins match. | `crates/roym_core/src/admit.rs:15-30`, `roym_*/src/app.rs` (`invoke` lines), `sandbox_wasm/.../capabilities_services.rs:80-96` |
| 20.5 | CONFIRMED | WIT `caller` returns `internal`, `verified` or `anonymous`; the host decides by dispatch path. | `crates/wit_interfaces/wit/invocation/invocation.wit:11-34`, `app_host_native/src/host.rs:692-698` |
| 20.6 | CONFIRMED | No `status` calls an admission check. `roym.toml` health checks call `status`. | `crates/roym_directory/src/app.rs:81-86`, `crates/roym_core/app/roym.toml:33-35` |
| 20.7 | CONFIRMED | `WIRE_REACHABLE` has four entries; absent methods are refused. The `credential.*` and other verbs are local-only. | `crates/roym_directory/src/app.rs:70-78,184-219`, `roym_core/src/admit.rs:46-81` |
| 20.8 | CONFIRMED | Struct has `controlled` and `controller`; `issue` makes two proofs. | `crates/identity/src/substrate.rs:46-120` |
| 20.9 | CONFIRMED | `claim` reads the node key from disk; `issue` refuses equal DIDs. | `apps/roymctl/src/commands/substrate.rs:26-55,151-170`, `crates/identity/src/substrate.rs:81-86` |
| 20.10 | CONFIRMED | Default `agreement.json` in `app_data_dir`; read once at start; expiry checked in `init` only. | `crates/core/src/config.rs:22`, `crates/substrate/src/identity.rs:41-47,82`, `crates/identity/src/substrate.rs:313-325` |
| 20.11 | CONFIRMED | Only `Verified` status yields a controller; it becomes `admin_ucan_root`; router pushes `substrate/admin`. | `crates/substrate/src/runtime/router.rs:55-112`, `crates/router/src/route_handler/io.rs:186-193` |
| 20.12 | CONFIRMED | Security interface needs `substrate/admin`; that ability entails every ability, so deploy, undeploy, status too. | `crates/control_plane/src/service/dispatch.rs:64-72`, `crates/ucan/src/capability.rs:142-145` |
| 20.13 | PARTLY | Fallback is real and fail-closed is real. But the fallback applies whenever no agreement is *verified*, not only when none exists. | `crates/substrate/src/runtime/router.rs:55-93`, `crates/identity/src/substrate.rs:313-360`, `crates/core/src/config/base.rs:315-326` |
| 20.14 | CONFIRMED | Default off (derive `Default`); grants only `supervisor/resolve` when `master_did == node_did`. | `crates/router/src/route_handler/io.rs:195-210`, `crates/core/src/config/base.rs:328-345` |
| 20.15 | CONFIRMED | 17 verbs counted in `supervisor.wit` and in the dispatch match; `export-master` doc says backup is mandatory. | `crates/wit_interfaces/wit/supervisor/supervisor.wit:162-325`, `crates/app_supervisor/src/service.rs:727-751` |
| 20.16 | CONFIRMED | `state.db` writer sets only the `key` pragma. WAL is set only on other files. | `crates/data_db/src/sqlite/provider.rs:281-285` |
| 20.17 | CONFIRMED | `AlreadyDecided` returns the stored booking. | `crates/roym_transaction/src/app/ledger.rs:62-64,124-126`, `booking_ops.rs:76-78` |
| 20.18 | CONFIRMED | Seats claimed with the create fence in order; none left gives `NoSeat(SlotTaken)`, opened as `conflict`; kebab-case on the wire. | `crates/roym_transaction/src/app/ledger.rs:150-175`, `roym_core/src/booking.rs:63-69,199-222` |
| 20.19 | CONFIRMED | `replicas > 1` with a schema is refused; one `service_id` per member. | `crates/app_orchestration/src/models/manifest.rs:78-101`, `compiler.rs:226-229` |
| 20.20 | CONFIRMED | Field copied at `compiler.rs:242`; mode is `Redundant` or default; only tests set `Sharded`. | `crates/app_orchestration/src/compiler.rs:196-200,242`, `models/manifest.rs:102-117` |
| 20.21 | PARTLY | Delegation only in `fixed` mode is right. "Signs every stream" is wrong: the preamble carries the public key only. | `crates/client_gateway/src/gateway.rs:517-532`, `crates/sdk/src/client.rs:640-667,374` |
| 20.22 | CONFIRMED | `IdentityMode` default `Open`; field in `ClientGatewayRole` under `roles.client_gateway`. | `crates/core/src/config/roles.rs:366-401`, `roles.rs:14` |
| 20.23 | CONFIRMED | 401 only in `Login` with the gate on and not the auth host. | `crates/client_gateway/src/gateway.rs:351-394,441-449` |
| 20.24 | CONFIRMED | `Fixed` mode answers whoami with `fixed_identity_did`. | `crates/client_gateway/src/gateway.rs:330-347,432-440` |
| 20.25 | CONFIRMED | Binds `0.0.0.0:<port>`; `peer_addr` only logged. | `crates/client_gateway/src/gateway.rs:184-207` |
| 20.26 | CONFIRMED | Six paths, with and without the `/_syneroym/session` prefix. | `crates/auth/src/service.rs:736-741` |
| 20.27 | PARTLY | Checks match. But the text to sign is returned only if the client names its master DID in the challenge request. | `crates/auth/src/service.rs:213-240,244-340`, `crates/core/src/config/roles.rs:360-362` |
| 20.28 | CONFIRMED | Lifetime is `min(remaining cert, session_ttl_secs)`; default 8 h. | `crates/auth/src/service.rs:310-325`, `crates/core/src/config/roles.rs:420-423` |
| 20.29 | CONFIRMED | Cookie `HttpOnly; SameSite=Lax`, `Secure` when set. | `crates/auth/src/service.rs:551-575`, `crates/core/src/protocol_utils.rs:82` |
| 20.30 | CONFIRMED | `local` is disabled without the dir; logout stores the token in an in-memory map. | `crates/auth/src/service.rs:180-198,343-352,418-424,622-638,672-690` |
| 20.31 | CONFIRMED | Hub keeps a non-extractable key in IndexedDB and runs challenge then login. | `crates/roym_web/ui/src/session/login.ts:1-20,190-237` |
| 20.32 | CONFIRMED | Replaces the caller only for the target node's own key and no cert; refuses logged-out tokens; no auth service gives `None`. | `crates/router/src/route_handler/http/auth.rs:78-120`, `crates/substrate/src/runtime/router.rs:461` |
| 20.33 | CONFIRMED | `select!` has all listed arms; `log_component_exit` handles `Ok` and `Err`; absent parts are pending. | `crates/substrate/src/runtime/services.rs:200-291,431-442,521-528` |
| 20.34 | CONFIRMED | Bind error is logged, then `pending_component().await`. | `crates/substrate/src/runtime/services.rs:364-440` |
| 20.35 | CONFIRMED | Only `signal::ctrl_c`; no other signal code outside `tls.rs` (SIGUSR1). | `crates/substrate/src/runtime.rs:43-50`, `crates/core/src/tls.rs:31-32` |
| 20.36 | CONFIRMED | Order matches: supervisor awaited; three workers cancelled and handles dropped; gateway, coordinator, registry; then observability and router. | `crates/substrate/src/runtime/services.rs:294-350`, `runtime.rs:98-119` |
| 20.37 | CONFIRMED | `claim_due` sets `visible_at = now + timeout`; an unacked item is claimable again. All three outboxes use this queue. | `crates/async_queue/src/queue.rs:268-325`, `conversation/src/outbox.rs:150` |
| 20.38 | CITE-OFF | All true. `ARCHIVE_VERSION` is in roymctl, not `roym_core/src/backup.rs:15`. | `apps/roymctl/src/commands/roym/backup.rs:25,229,258`; `lifecycle.rs:114-129`; `verify.rs:159-160`; `backup.rs:22,215-216`; `master_anchor.rs:17,113` |
| 20.39 | CONFIRMED | Default 8 in `StreamingConfig`; used by the WASM engine. | `crates/core/src/config/base.rs:297-313`, `crates/sandbox_wasm/src/engine/init.rs:223-227` |
| 20.40 | CONFIRMED | 5 s, 256 KiB, and 5 s are fixed constants and are applied. | `crates/router/src/route_handler/io.rs:31,41,272,374`, `handshake.rs:67` |
| 20.41 | CONFIRMED | 30 s default; `req.timeout.unwrap_or(...)` in local and remote paths. | `crates/rpc/src/proxy.rs:148`, `crates/router/src/proxy/router.rs:223,403` |
| 20.42 | CONFIRMED | `MAX_QUEUED_PAYLOAD_BYTES = 256 * 1024`, checked in `store`. | `crates/router/src/proxy_outbox.rs:50,269-271` |
| 20.43 | CONFIRMED | Defaults 5 and 30; fields under `AppSandboxRole`, wired to epoch ticks. | `crates/core/src/config/sandbox.rs:68-73,123,131,409-410`, `sandbox_wasm/src/engine.rs:231-235` |
| 20.44 | CONFIRMED | `MAX_REPLICAS = 16`, `MAX_SCHEDULED_SERVICES = 16`; both checked in `validate`. | `crates/app_orchestration/src/models/service.rs:287`, `schedule.rs:12-16`, `models/manifest.rs:78-89,163-166` |
| 20.45 | PARTLY | 10 s, 30 s, UTC are right. The parser also takes 6 and 7 fields, and the field is `timeout_ms`, not `timeout`. | `crates/app_orchestration/src/schedule.rs:18-45,138-160`, croner 3.0.1 `src/parser.rs:55-60,125-129` |
| 20.46 | CONFIRMED | `DEFAULT_CONNECT_TIMEOUT` 10 s; `with_connect_timeout` overrides. | `crates/sdk/src/client.rs:27-33,301-307` |
| 20.47 | CONFIRMED | Feature lists and non-optional dependencies match. | `crates/substrate/Cargo.toml:11-103`, `services.rs:211-241` |
| 20.48 | CONFIRMED | `profile` is logged only; `profiles` is read by nothing. Negative search repeated. | `crates/substrate/src/main.rs:32-34,99-101`, `runtime.rs:126`, `core/src/config.rs:63,72` |
| 20.49 | CONFIRMED | Base, binaries, entry point, command and ports match. | `Dockerfile:20-38` |
| 20.50 | CONFIRMED | SIGUSR1 handler reloads cert and key; non-Unix only warns; only the Iroh info server uses it. | `crates/coordinator_iroh/src/coordinator.rs:321-331`, `crates/core/src/tls.rs:26-62` |

## Findings

### 20.13 (PARTLY): `admin_ucan_root` fallback is narrower in the doc than in the code

Doc words (Node ownership, line 510): "`[iam].admin_ucan_root` is only a fallback for a node with no agreement. A node with neither runs unowned".

Code: only a `Verified` agreement overrides the config value (`crates/substrate/src/runtime/router.rs:75-78,100-109`). `require_agreement` is a plain `bool` (`crates/core/src/config/base.rs:11`). With it off, an agreement that is expired, has bad signatures, or names another node still lets the node boot as `Unverified` or `None` (`crates/identity/src/substrate.rs:313-360`). Then `admin_ucan_root` is used. If it is not set, the node is unowned.

Proposed fix: "`[iam].admin_ucan_root` is only a fallback for a node with no verified agreement. A node with neither a verified agreement nor `admin_ucan_root` runs unowned".

### 20.21 (PARTLY): the gateway does not sign streams

Doc words (Client Gateway and Auth Service, line 649): "The gateway signs every stream with its own node key."

Code: the gateway puts its node public key in the preamble (`crates/client_gateway/src/gateway.rs:521-531`, `crates/sdk/src/client.rs:662`). There is no signature field in `RoutePreamble`. The SDK binds its Iroh endpoint with no secret key (`crates/sdk/src/client.rs:374`), so the QUIC identity is not the node key either. The SDK says "a self-asserted pubkey is an assertion, not proof-of-possession" (`crates/sdk/src/client.rs:77-84`). The doc itself says in the Access control item 1 (line 506) that the router does not check the private key. The two sentences disagree.

Proposed fix: "The gateway names its own node key in the route preamble of every stream. It does not sign the stream, and the router does not check that the gateway holds the private key."

Same words in the `open` row of the table (line 655, "The node DID of the gateway") are correct and need no change.

### 20.27 (PARTLY): challenge text is returned only on request

Doc words (line 661): "The service returns a one-time nonce (lifetime `nonce_ttl_secs`, default 60 seconds) and a text that names the node, the nonce and the master DID."

Code: `assertion` is `Option`. It is built only when the challenge request carries `master_did` (`crates/auth/src/service.rs:234-238`). The Hub sends `master_did` (`crates/roym_web/ui/src/session/login.ts:196-199`). A client that omits it gets the nonce and the node DID only.

Proposed fix: "The client asks for a challenge and names its master DID. The service returns a one-time nonce (lifetime `nonce_ttl_secs`, default 60 seconds) and a text that names the node, the nonce and that master DID."

### 20.38 (CITE-OFF)

The claim and doc text are true. Only the cite for the Roym archive is wrong. `crates/roym_core/src/backup.rs:15` holds `BUNDLE_VERSION`, a different constant. `ARCHIVE_VERSION = 1` is at `apps/roymctl/src/commands/roym/backup.rs:25`. Restore refuses other versions at `:229` and `:258`. The doc text (line 685) needs no change.

### 20.45 (PARTLY): cron field count and the `timeout` name

Doc words (Limits and Budgets, lines 701 and 704): "The `timeout` of the schedule" and "A schedule is a five-field cron expression."

Code:

- The field is `timeout_ms`, in milliseconds (`crates/app_orchestration/src/schedule.rs:65-66`). The manifest check names `MAX_SCHEDULE_TIMEOUT_MS` (`models/manifest.rs:149-155`).
- The cron text is parsed with `str::parse::<croner::Cron>()` (`schedule.rs:74`). In croner 3.0.1 that parser has seconds and year optional (`src/parser.rs:55-60,125-129`). So five, six (leading seconds) and seven fields all parse. The repo's own test `a_six_field_expression_is_read_as_seconds_first` (`schedule.rs:149-157`) shows six fields are accepted. The doc says five.

Proposed fix, table row: "One scheduled run | 10 seconds. At most 30 seconds. | The `timeout_ms` of the schedule (milliseconds)".

Proposed fix, text under the table: "A schedule is a cron expression. Write it with five fields (minute, hour, day of month, month, day of week). The parser also accepts a leading seconds field and a trailing year field. The App Supervisor evaluates it in UTC."

## Doc text not covered by any row

1. Lines 488 and 649 say the gateway is a "local HTTP proxy" and "the local HTTP entry". The listener binds `0.0.0.0` (`crates/client_gateway/src/gateway.rs:185`). Row 20.25 says the gateway sets no rule about which machine may connect, which is right, but "local" suggests loopback. Proposed: "A local HTTP proxy. It listens on all interfaces of the host."
2. Line 666: "`logout` puts the token on a list that the auth service keeps." The list is in memory only and holds at most 10,000 tokens (`crates/auth/src/service.rs:29,118,194-198`). A restart empties it. A full list silently stops recording. The doc does not say so. Proposed: add "The list is in memory. A restart clears it, and it holds at most 10,000 tokens."
3. Line 666: the `local` login needs no secret. It mints a token for any key file name in the directory. It refuses only a request that carries an `Origin` header (`crates/auth/src/service.rs:622-630`). The gateway listens on all interfaces (item 1). The doc does not warn. Proposed: say that `local` is for a trusted host and has no proof step.
4. Line 668: "...that is, from the gateway." The test is `caller_did == node_did` of the target router (`crates/router/src/route_handler/http/auth.rs:84-87`). Only a gateway on the same node as the target qualifies. A gateway on another node gets no session caller. Proposed: "that is, from the gateway on the same node."
5. Line 672: the `select!` also has the instance certificate expiry loop (`crates/substrate/src/runtime/services.rs:250,289`). It never finishes (`runtime/publish.rs:130`, return type `!`). The list of components omits it. Low impact.
6. Line 674: the Docker image uses the exec form `ENTRYPOINT ["syneroym-substrate"]` (`Dockerfile:37`), so the substrate is PID 1. With no SIGTERM handler, `docker stop` probably cannot start the graceful shutdown. This is my inference from the Linux PID 1 rule, not read in code. The doc could say that a container is stopped by SIGKILL after the stop timeout.
7. Line 685: "calls the guest's `init()`..." The hook runs only if the component exports it (`crates/sandbox_wasm/src/engine/lifecycle.rs:150-153`). A failed hook fails the deploy (`:128-130`). Proposed: "calls the guest's `init()`, if the guest exports it".
8. Line 510: "This covers deploy, undeploy and status, and the `security` interface." `substrate/admin` entails every ability (`crates/ucan/src/capability.rs:142-145`). The list is true but reads as complete. Proposed: "This covers every node-wide ability, including deploy, undeploy, status and the `security` interface."
9. Line 708: "Other features are `coordinator_iroh`, `coordinator_webrtc`, `aws`, `roym`..." The feature `coordinator` also exists (`crates/substrate/Cargo.toml:67`). It is an internal switch the other two use. Low impact.
10. Line 607: "the substrate sets no WAL pragma on them today." True for `state.db`. The outbox files, the conversation store and the supervisor store do set WAL (`crates/async_queue/src/queue.rs:619`, `crates/conversation/src/store/schema.rs:253`, `crates/app_supervisor/src/store/schema.rs:32`). Say "no WAL pragma on the service database (`state.db`)" to avoid doubt.

## Cost notes

- 50 rows took roughly 2.5 hours of reading, about 20 rows per hour. Rows that cite one constant are quick (about 1 minute). Rows on identity and auth need a full read of two or three files.
- Hard rows:
  - 20.45: the cite shows 10 s and 30 s, but "five-field" needed the croner 3.0.1 source in `~/.cargo/registry`. The repo's own six-field test settled it.
  - 20.21: needed a read of the SDK connect path to see that no signature and no node secret key are used.
  - 20.32: the phrase "node's own key" means the target node's key. Took a read of `route_handler/http/auth.rs` and the gateway to be sure.
  - 20.13: needed `SubstrateIdentityState::init` end to end to see the `Unverified` branches.
  - 20.38: one bad cite among six constants, found only by opening each file.
- Row ids are by file order; all 50 matched the order of the doc sections, so mapping was easy.
