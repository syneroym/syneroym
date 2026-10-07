# Fix verification, batch S2 (fix commits 6 and 7)

Reader: the author of the architecture fix, who must correct the doc text.

Scope: 49 rows of `fix-new-claims.md` (commit 6: 31, commit 7: 18). Checked against the code on branch `docs/architecture-fix`. Only static reading. Row id = `<commit>.<n>`, the n-th row of that commit in file order. The current file was read (including the follow-up that changed a row of commit 6).

Doc lines below are lines of `docs/system-architecture.md`.

## Summary

| Verdict | Count |
| --- | --- |
| CONFIRMED | 35 |
| CITE-OFF | 0 |
| PARTLY | 13 |
| WRONG | 1 |
| UNVERIFIABLE | 0 |
| Total | 49 |

## Rows

| Row | Verdict | Note | Checked at |
| --- | --- | --- | --- |
| 6.1 | CONFIRMED | Three topology modes. No replica, WAL-ship, failover or quarantine code in `crates/`, `apps/`. | `crates/app_orchestration/src/models/service.rs:26-31`; grep `replica`, `litestream`, `quarantin`, `denylist`, `failover` |
| 6.2 | PARTLY | `replicas > 1` refused only if the config JSON `schema` is set. That field is not a database schema. | `crates/app_orchestration/src/models/manifest.rs:92`; `crates/app_orchestration/src/models/service.rs:167`; `docs/developer-guide.md:1149-1153` |
| 6.3 | CONFIRMED | Bindings live in the service's own store. Endpoints resolve via the registry. The supervisor is an optional role. | `crates/data_db/src/registry_store.rs:157`; `crates/router/src/proxy/router.rs:298-303`; `crates/substrate/Cargo.toml:60-66` |
| 6.4 | PARTLY | `state.db` sets no journal mode. But `async.db`, a per-service file, is opened in WAL mode. | `crates/data_db/src/sqlite/provider.rs:269-289`; `crates/async_queue/src/queue.rs:606-619` |
| 6.5 | CONFIRMED | Only `Transport` and `Timeout` are retried, only when `idempotent` or keyed, with jittered exponential backoff. | `crates/router/src/proxy/router.rs:417-437`; `crates/core/src/retry.rs:12` |
| 6.6 | CONFIRMED | Defaults 3 / 100 ms / x2 / 30 s and 30 s call timeout match. Outbox has its own separate budget. | `crates/core/src/config/base.rs:237-267`; `crates/rpc/src/proxy.rs:148`; `crates/router/src/route_handler.rs:292-294` |
| 6.7 | PARTLY | Dead letter is written for any target-produced failure of a keyed guest call, not only after retries run out. | `crates/router/src/proxy.rs:58-83`; `crates/router/src/proxy/router.rs:16-55,589-593` |
| 6.8 | PARTLY | Timeout, not-found and internal errors also queue the call. The 256 KiB limit is on the whole stored record. | `crates/router/src/proxy/router.rs:490-575`; `crates/router/src/proxy_outbox.rs:102-123,262-273` |
| 6.9 | PARTLY | Name is stored only when the guest names a dependency. A DID target is stored as the DID. | `crates/router/src/proxy_outbox.rs:219-248`; `crates/sandbox_wasm/src/host_capabilities/capabilities_proxy.rs:246-260` |
| 6.10 | CONFIRMED | `record-write-value` carries the caller's `id`. `create` refuses when any id exists. | `crates/wit_interfaces/wit/data-layer/data-layer.wit:20-23,164-175` |
| 6.11 | PARTLY | Codes and storage right. "Does not run twice" is not absolute. The row cap is per caller, not per service. | `crates/async_queue/src/dedup.rs:15-25,80,93-100,383-430` |
| 6.12 | CONFIRMED | Both entry points call the guard. No store means refusal. Wire path answers -32603 when no guard exists. | `crates/router/src/call_dedup.rs:1-24,300-335`; `crates/router/src/route_handler/dispatch.rs:122-158` |
| 6.13 | PARTLY | Parser also accepts a seconds field and a year field. All other numbers and limits match. | `crates/app_orchestration/src/schedule.rs:12-45,150-157`; `croner-3.0.1/src/parser.rs:55-70` |
| 6.14 | CONFIRMED | First-sight watermark, round-robin over healthy members, skip when none, awaited under the instance lock. | `crates/app_supervisor/src/service/schedules.rs:15-80,154-200`; `.../resident_loop/pass.rs:19-23` |
| 6.15 | CONFIRMED | Five saga verbs, intent logged first, reverse walk, `saga-undo-` call with platform key, `forward-result` merged. | `crates/wit_interfaces/wit/proxy/proxy.wit:112-190`; `crates/router/src/proxy/saga_dispatch.rs:138-215,540-575,700-719` |
| 6.16 | CONFIRMED | SDK sets `idempotency_key: None`. No `restartable` or task registry anywhere. See section 4 on "only a service has an outbox". | `crates/sdk/src/client.rs:490,578`; grep `restartable` |
| 6.17 | CONFIRMED | No DataFusion, Substrait, Arrow, Parquet in any manifest or source. No `data/stream` or `data/transform` WIT. | grep over `crates/`, `apps/`, `Cargo.toml`, `Cargo.lock`; `crates/wit_interfaces/wit/` listing |
| 6.18 | CONFIRMED | `call-target` is a DID or a dependency name. Resolved per call. Native capabilities of another service refused. | `crates/sandbox_wasm/src/host_capabilities/capabilities_proxy.rs:77-110`; `crates/router/src/proxy/router.rs:138-142` |
| 6.19 | CONFIRMED | One in-process `rumqttd`; `v4`/`v5`/`ws`/`bridge` are `None`. Retain exists only as a test helper. | `crates/mqtt_broker/src/lib.rs:112-132,158-165`; `crates/wit_interfaces/wit/messaging/messaging.wit:9-13` |
| 6.20 | CONFIRMED | Subscribe keeps a `svc/` topic as written. Publish always prefixes. Both used by the WASM host. | `crates/mqtt_broker/src/lib.rs:65-83`; `crates/sandbox_wasm/src/host_capabilities/capabilities_services.rs:154,166` |
| 6.21 | CONFIRMED | Own crate. Router thread spawned by `Broker::new`. Forwarding uses `tokio::spawn`. | `crates/mqtt_broker/src/lib.rs:1-17,195`; `crates/mqtt_broker/Cargo.toml:2` |
| 6.22 | CONFIRMED | Table in `substrate.db`, replayed at start. Missing `handle-message` is discarded with `debug!`. | `crates/data_db/src/sqlite/provider.rs:138-146`; `crates/substrate/src/runtime/router.rs:581-607`; `crates/sandbox_wasm/src/engine/auth.rs:114-125` |
| 6.23 | CONFIRMED | `websocket` is a route target next to the other four. It upgrades and hands frames to the guest. | `crates/router/src/route_handler/http/dispatch.rs:291-303`; `.../http/websocket.rs:105-125` |
| 6.24 | WRONG | Row is right. Doc text says `messaging` always needs a caller, and "only `public: true`" reaches guest code. Both false. | `crates/router/src/route_handler/http/streaming.rs:9-30,93-130`; `.../http/dispatch.rs:37`; `deferred-backlog.md:52` |
| 6.25 | CONFIRMED | Features `syneroym-oltp`/`syneroym-olap` are empty. No `cfg(feature)` or other crate uses them. | `crates/data_db/Cargo.toml:30-32`; grep `oltp`, `olap` |
| 6.26 | PARTLY | Writer task, `mpsc`, reader pool, `batch-mutate` right. Handler caps VM instructions, and only on some queries. | `crates/data_db/src/sqlite/provider.rs:299-330`; `.../sieve.rs:66-95`; `.../query.rs:40,125,198,296` |
| 6.27 | PARTLY | Index `type` is accepted but never used. Every index is `json_extract(payload, '$.field')`. | `crates/data_db/src/sqlite/schema.rs:29-56`; `.../mutation.rs:40-47`; grep `IndexType` |
| 6.28 | CONFIRMED | All 14 functions exist in `store`. `create` is insert-only-if-no-id. | `crates/wit_interfaces/wit/data-layer/data-layer.wit:82-197` |
| 6.29 | CONFIRMED | Seven stage keys. `GROUP BY` and `HAVING`. Views deferred, WIT says so. | `crates/data_db/src/aggregate.rs:1-24,96-99`; `data-layer.wit:132-142` |
| 6.30 | CONFIRMED | `AppScope::Foreign` is keyed by app DID. Alias is `<nickname>-<8-char hash>`. | `crates/app_orchestration/src/resolver/types.rs:293-310`; `crates/core/src/util.rs:85-103`; `crates/community_registry/src/registry.rs:196-208` |
| 6.31 | PARTLY | Blob quota is one node-wide config (`storage.blob_store`), not set per service. Backends and `aws` right. | `crates/core/src/config/base.rs:63-100`; `crates/data_blob/src/object_store_impl.rs:77-90`; `crates/substrate/src/runtime/router.rs:623-637` |
| 7.1 | CONFIRMED | Inject and rotate go through a control-plane call gated on `substrate/admin`. Rotation re-wraps `dek_store` rows only. | `crates/control_plane/src/service/dispatch.rs:54-80`; `crates/data_keystore/src/key_store.rs:79,197-260` |
| 7.2 | CONFIRMED | HKDF-SHA256 per `service_id`, DEK in `dek_store` under AES-256-GCM, KEK only in RAM, open fails without it. | `crates/data_keystore/src/key_store.rs:37-47,119-150`; `crates/data_db/src/sqlite/provider.rs:117,164-170,242` |
| 7.3 | PARTLY | `_vault` in `state.db`, AES-256-GCM right. Native `vault/reveal` also returns the secret to the service's owner. | `crates/data_db/src/sqlite/service_store.rs:174-215`; `crates/control_plane/src/synsvc_native/vault_config.rs:11-33`; `.../signing.rs:42-67` |
| 7.4 | CONFIRMED | `mlock` on Unix, `madvise` on Linux only, warning on failure. Called for identity keys and the KEK. | `crates/identity/src/keys.rs:26-62,114,126`; `crates/data_keystore/src/key_store.rs:92,279` |
| 7.5 | CONFIRMED | No attestation code. Podman has no secret or `tmpfs` path. KEK is one `Zeroizing<[u8;32]>`, no per-instance inject. | `crates/sandbox_podman/src/engine.rs:339-372`; `crates/data_keystore/src/key_store.rs:25-30,75-78`; grep `attest`, `tpm`, `tmpfs` |
| 7.6 | PARTLY | Flatten, schema check, generation table, per-invocation pin all right. An identical redeploy is a no-op with no new generation. | `crates/control_plane/src/service/orchestration/deploy/manifest.rs:36-73`; `.../deploy.rs:42-50`; `.../deploy/admission.rs:182-195` |
| 7.7 | CONFIRMED | `get` and `get-section` (prefix). Podman `-e`, manifest `env` wins, `:ro` when volume has manifest files. | `crates/wit_interfaces/wit/app-config/app-config.wit:8-14`; `crates/sandbox_podman/src/engine.rs:75-90,339-372` |
| 7.8 | CONFIRMED | `reveal` returns bytes to the calling guest. `WasiCtx` is empty, so no env or file path carries it. | `crates/sandbox_wasm/src/host_capabilities/capabilities_services.rs:21-47`; `crates/wit_interfaces/wit/host/deps/vault/vault.wit:10` |
| 7.9 | CONFIRMED | `WasiCtx::builder().build()` is the only WASI context. No `restartable` or task registry. | `crates/sandbox_wasm/src/host_capabilities.rs:429` |
| 7.10 | CONFIRMED | `write_attribution` as stated. Capability resource format as stated. Write value has only `id` and `payload`. | `crates/rpc/src/native.rs:157-176`; `crates/sandbox_wasm/src/host_capabilities/capabilities_store.rs:158,470,495`; `crates/ucan/src/capability.rs:5-15` |
| 7.11 | CONFIRMED | Every data-layer function calls `open_store(component_id, ...)`. | `crates/sandbox_wasm/src/host_capabilities/capabilities_store.rs:12-20,108-550` |
| 7.12 | CONFIRMED | Certificate checks, key match, revocation check, fallback to own key. No signature field in the preamble. | `crates/router/src/handshake.rs:25-83`; `crates/identity/src/delegation.rs:13-30,203-293`; `crates/router/src/preamble.rs:171-196` |
| 7.13 | CONFIRMED | `fdae/v1` check, embedded schema, 32-hop and depth-64 limits, relation fields, operators, conditions all match. | `crates/fdae/src/policy.rs:19-50`; `crates/fdae/src/policy/types.rs:14,29-170`; `crates/fdae/src/compile/types.rs:14` |
| 7.14 | CONFIRMED | Deny list, caveat deny and `where` ANDed, parse-time refusals all match. | `crates/fdae/src/policy.rs:180-210`; `crates/fdae/src/compile/plan.rs:606-640`; `crates/data_db/src/sqlite/sieve.rs:97-112` |
| 7.15 | CONFIRMED | Restrict-only; missing export, trap, malformed, over-large batch all fail closed to no rows. | `crates/rpc/src/fdae_abac.rs:215-300`; `crates/fdae/src/policy/types.rs:122-145` |
| 7.16 | PARTLY | Correlated `EXISTS` and fused recursive block right. A zero-hop path is `col = ?`, a remote last hop is `IN (...)`. | `crates/fdae/src/compile/emit.rs:128-146,299-345,526` |
| 7.17 | CONFIRMED | `plan_read` pending query, `resolve-relation`, 15 s, asserter check, failure gives permission-denied. | `crates/rpc/src/fdae_fetch.rs:31,100-135`; `crates/sandbox_wasm/src/host_capabilities.rs:585-600` |
| 7.18 | CONFIRMED | 50,000,000 VM ops constant, installed on sieved reads and writes, interrupt gives `QuotaExceeded` or `Internal`. | `crates/data_db/src/sqlite/sieve.rs:66-95`; `crates/data_db/src/sqlite/query_raw.rs:83-98`; `crates/data_db/src/errors.rs:12-17` |

## Findings

Each finding gives the doc words, what the code does, and the proposed new wording.

### 6.2 (PARTLY) replicas refusal says "database schema"

Doc (line 2734, `[PLT-RED]` intro): "A manifest can ask for several `replicas` of a service that has no database ... A manifest that asks for more than one replica of a service with a database schema is refused."

Code: the check is `spec.replicas > 1 && spec.config.schema.is_some()` (`crates/app_orchestration/src/models/manifest.rs:92`). `config.schema` is the JSON Schema that validates `custom_config` (`crates/app_orchestration/src/models/service.rs:167`; `crates/control_plane/src/service/orchestration/deploy/manifest.rs:51-59`). It is not a database schema. A service that uses the data layer without a `schema` is not caught (`docs/developer-guide.md:1149-1153`).

Proposed text: "A manifest can ask for several `replicas` of a stateless service. The compiler then gives the service the `Redundant` topology mode, and the caller picks a member itself (see `[TOP-ADR]`). The manifest check refuses more than one replica when the service declares a config `schema`, because that field marks a service that holds state. The check cannot see a service that uses the data layer without a `schema`."

### 6.4 (PARTLY) "service databases do not use WAL mode"

Doc (line 2741): "WAL shipping needs service databases in WAL mode, which they do not use today." Same idea at line 2624: "Running service databases in WAL mode".

Code: `state.db` sets no journal mode (`crates/data_db/src/sqlite/provider.rs:269-289`). The per-service `async.db` (outbox, dead letters, dedup, saga log) is opened with `PRAGMA journal_mode=WAL` (`crates/async_queue/src/queue.rs:606-619`, used by `dedup.rs:181`, `saga.rs:49`). Line 2620 is already exact (`state.db`).

Proposed text (2741): "WAL shipping needs the `state.db` of a service in WAL mode. `state.db` does not use WAL mode today." (2624): "Running the `state.db` of a service in WAL mode, with ...".

### 6.7 (PARTLY) dead letter only "when the retries are used up"

Doc (line 2713): "When the retries are used up, the call fails to the caller directly. If the call carried an idempotency key, the proxy also writes a dead letter ..."

Code: `ServiceProxy::invoke` calls `record_failed_call` after any failure of the call (`crates/router/src/proxy/router.rs:589-593`). `record_failed_call` returns early unless the call has a key, the node has an outbox, and the origin is `CallOrigin::Guest` (`router.rs:16-22`). It then writes a row when `target_produced` is true (`crates/router/src/proxy.rs:58-83`). That is true for a transport failure, a timeout, a not-found target and any callee error except -32094 and -32095. A callee error is never retried, but it still gets a dead letter. Refusals (`PermissionDenied`, `UnsupportedTarget`, `UnsupportedProtocol`, `Internal`) get none.

Proposed text: "When the call fails, it fails to the caller directly. If a guest service made the call and it carried an idempotency key, the proxy also writes a dead letter to a local SQLite-backed DLQ. This happens when the target was reached or should have been: a transport failure after the retries, a timeout, a target not found, or an error answer from the callee. A refusal made before the call (permission, unsupported target or protocol, internal error) and the dedup codes -32094 and -32095 write none. A call with no key is never dead-lettered, because a replay of it could run the target twice. A dead letter is a row in the `dead_letters` table of the calling service's own `async.db` file. An outbox item that fails for good, or uses up its attempt budget, also becomes a dead letter."

### 6.8 (PARTLY) outbox entry condition and size limit

Doc (line 2715): "The call is tried at once. Only a transport failure puts it in the outbox of the calling service." and "a queued call carries at most 256 KiB of parameters."

Code: `enqueue_call` makes a probe call limited to 2 s (`crates/router/src/proxy.rs:94`; `router.rs:546-572`). It queues the call when the probe times out, or when `disposition_of` says `Retry`. `Retry` covers `Transport`, `Timeout`, `Internal`, `ServiceNotFound`, and callee codes -32094 and the not-found code (`crates/router/src/proxy_outbox.rs:102-123`). Other errors go back to the guest and nothing is queued. `Delivered` (-32095) counts as success. The 256 KiB limit is checked on the whole JSON-encoded `QueuedCall` (target, interface, method, params, key), only when the call is queued (`proxy_outbox.rs:262-273`). A second limit, `max_pending_rows`, refuses new items when a service has too many waiting (`proxy_outbox.rs:274-286`).

Proposed text: "The call is tried at once. If the target cannot be reached for now, the call goes into the outbox of the calling service. That means a transport failure, a timeout (the first try has a 2 s limit), a target that is not found yet, or an internal error. Any other error from the target goes back to the caller and nothing is queued. ... `enqueue` requires an `idempotency-key`. A queued call (the whole stored record, parameters included) is at most 256 KiB, and a service has a cap on how many calls may wait. Success means ..."

### 6.9 (PARTLY) outbox "stores the dependency name"

Doc (line 2715): "The outbox stores the dependency name, not a resolved address, and resolves it again on every attempt."

Code: `QueuedTarget::Dependency(name)` is stored and resolved again at each attempt. `QueuedTarget::Service(did)` is stored when the guest passed a DID (`crates/sandbox_wasm/src/host_capabilities/capabilities_proxy.rs:246-260`; `crates/router/src/proxy_outbox.rs:219-248`).

Proposed text: "When the guest names a declared dependency, the outbox stores that name, not a resolved address, and resolves it again on every attempt. When the guest names a DID, the outbox stores the DID."

### 6.11 (PARTLY) dedup: "does not run twice" and "each service has a row cap"

Doc (line 2718): "A duplicate is answered from the stored outcome, and the target does not run twice. ... each service has a row cap."

Code: delivery is at least once. A claim past its window is retaken and the call runs again (`crates/async_queue/src/dedup.rs:15-25,268-300`). A late attempt records nothing (`dedup.rs:335-345`). A record that expires or is pruned no longer answers (`dedup.rs:378-382`). A claim is dropped, not kept, when the call never reached the target (`dedup.rs:360-372`). The row cap is counted per caller, not per service: default 10,000 rows (`dedup.rs:80`, `prune` at `:383-430`).

Proposed text: "A target node remembers every call that carries an idempotency key, per caller and key, in the target service's own `async.db` file. While a record lives, a duplicate is answered from the stored outcome and the target does not run again. ... A record expires after a time derived from the sender's retry window. Each caller has a row cap (10,000 records) in the store of each service. A call that is still running when its claim window (twice the call budget) ends, or whose record has expired or been pruned, can run again."

### 6.13 (PARTLY) "five-field cron"

Doc (line 2721): "a five-field cron expression evaluated in UTC".

Code: the manifest parses `cron` with `croner::Cron::from_str` (`crates/app_orchestration/src/schedule.rs:72-77`). In croner 3.0.1 the default parser has seconds `Optional` and year `Optional` (`croner-3.0.1/src/parser.rs:55-70`). The repo test reads a six-field expression with seconds first (`schedule.rs:150-157`). The manifest check does not restrict the field count (`manifest.rs:122-133`).

Proposed text: "a cron expression evaluated in UTC (five fields; the parser also accepts a leading seconds field and a trailing year field, but the supervisor looks at the schedule once per reconcile pass)".

### 6.24 (WRONG) caller identity on the HTTP bridge

Doc (line 2672): "The `data-layer` and `messaging` targets require a verified caller identity before dispatch: ... an anonymous caller is rejected before the native service is invoked (bridged routes return HTTP 401) ... The `guest` target is authenticated by default. A route reaches guest code with no verified caller only when it explicitly sets `public: true`."

Code: `dispatch_native` returns 401 for no caller (`crates/router/src/route_handler/http/dispatch.rs:37-42`). The `data-layer` operations and `messaging` `publish` go through it. The `messaging` operation `subscribe-sse` does not: `handle_messaging_sse` has no caller check (`crates/router/src/route_handler/http/streaming.rs:9-95`). The `stream` target also reaches guest code with no check (`streaming.rs:93-130`), as the doc itself says two sentences earlier. `guest` and `websocket` both gate on `!route.public` (`guest.rs:41`, `websocket.rs:110`). The deferred backlog lists the gap for both targets (`docs/planning/deferred-backlog.md:52`). The row 6.24 itself (stream has no gate; guest needs identity unless `public`) is correct.

Proposed text: "The `data-layer` targets and `messaging` `publish` require a verified caller identity before dispatch: ... The `messaging` `subscribe-sse` operation and the `stream` target do not go through that check. They have no caller gate at all (a known gap, tracked in `deferred-backlog.md`). The `guest` and `websocket` targets are authenticated by default. They reach guest code with no verified caller only when the route explicitly sets `public: true`."

### 6.26 (PARTLY) "progress handlers ... cap how long a query may run"

Doc (line 2599): "progress handlers (they cap how long a query may run)".

Code: the handler counts SQLite virtual-machine instructions (50,000,000), not time (`crates/data_db/src/sqlite/sieve.rs:66-95`). It is installed only on reads and `delete-many` that run under a policy sieve, and on `query-raw` (`query.rs:40,125,198,296`; `mutation.rs:169`). A plain `get` or `query` with no policy has none.

Proposed text: "progress handlers (they stop a policy-checked or raw query that runs too many SQLite instructions)".

### 6.27 (PARTLY) "typed indexes"

Doc (lines 2603 and 2606): "a list of typed indexes" and "Declares indexed fields explicitly (`string`, `numeric` or `boolean`). Each index is a SQLite expression index over the JSON field."

Code: `do_create_collection` builds `CREATE INDEX ... ON t(json_extract(payload, '$.field'))` and never reads the index type (`crates/data_db/src/sqlite/schema.rs:29-56`). Nothing else in `crates/data_db/src` uses `IndexType`.

Proposed text: "The schema is the name of the collection plus a list of indexes. Each index names a JSON field and a declared type (`string`, `numeric` or `boolean`). The host does not use the type today. Every index is a SQLite expression index, `json_extract(payload, '$.field')`. A write is checked only to be valid UTF-8 JSON: ..."

### 6.31 (PARTLY) blob quota "each service has"

Doc (line 2617): "Each service has a blob quota (a size limit per blob and an optional total per service)".

Code: both limits come from one node setting, `storage.blob_store.max_blob_bytes` (default 100 MiB) and `max_service_total_bytes` (default none). The provider applies them to every service (`crates/core/src/config/base.rs:63-100`; `crates/data_blob/src/object_store_impl.rs:77-90`). The manifest quota has only `max_instructions` and `max_memory_bytes` (`crates/app_orchestration/src/models/service.rs:142-147`).

Proposed text: "The node sets one blob quota in its `storage.blob_store` configuration and applies it to each service: a size limit per blob (default 100 MiB) and an optional total per service."

### 7.3 (PARTLY) "`reveal` ... returns the value to the calling guest only"

Doc (line 2525): "The `reveal` host function returns the value to the calling guest only; the vault never writes secret values to files or environment variables."

Code: the WASM host function does return the bytes to the calling guest (`crates/sandbox_wasm/src/host_capabilities/capabilities_services.rs:21-47`). The native `vault` interface also answers `reveal` over JSON-RPC. It admits the service itself (`system:<id>` identities) and the service's recorded owner (`crates/control_plane/src/synsvc_native/vault_config.rs:11-33`; `signing.rs:42-67`). I found no code that writes a secret to a file or environment variable (grep `reveal_secret` callers: `app_supervisor/src/keys.rs:216`, `vault_config.rs:28`, the host function).

Proposed text: "The `reveal` host function returns the value to the calling guest. The native `vault` interface also answers `reveal`, but only for the service itself and for the owner recorded for that service. The vault never writes secret values to files or environment variables."

### 7.6 (PARTLY) "On each deploy ... saves it as a new configuration generation"

Doc (line 2537): "On each deploy, the orchestrator flattens the `custom_config` ... and saves it as a new configuration generation ..."

Code: a deploy whose content is identical to the installed, running service (same caller, same hash) returns success and does nothing (`crates/control_plane/src/service/orchestration/deploy.rs:42-50`; `.../deploy/admission.rs:182-195`). No new generation is saved then. The schema check also runs only when `custom_config` is present (`.../deploy/manifest.rs:42-56`).

Proposed text: "On each deploy that changes something, the orchestrator flattens ... A deploy that is identical to the running service is a no-op and saves no new generation."

### 7.16 (PARTLY) "each permission path compiles to a correlated `EXISTS`"

Doc (line 2567): "The engine compiles each permission path into a correlated `EXISTS` subquery, nested for a chain of local relations."

Code: a path with no relation compiles to `column = ?`. A path whose last relation is remote compiles to an `IN (...)` check against the fetched ids. Only paths with local relations compile to `EXISTS` (`crates/fdae/src/compile/emit.rs:299-345`, doc comment of `compile_path`).

Proposed text: "The engine compiles each permission path with local relations into a correlated `EXISTS` subquery, nested for a chain of local relations. A path with no relation becomes a direct column comparison. When the last relation of a path is remote, the path becomes an `IN (...)` check against the fetched ids. When the last relation is recursive, ..."

## Doc text not covered by any row

1. Line 2574: "a path concatenation tracker (`visited_track`)". The code has no `visited_track`. The recursive block uses a column named `seen` (`crates/fdae/src/compile/emit.rs:526-556`). The name appears only in a comment (`crates/fdae/src/compile/types.rs:12`). Fix: say "a path string (`seen`)".
2. Line 2679: "`ProxyRouter::invoke_remote`, `crates/router/src/proxy.rs`". The function is in `crates/router/src/proxy/router.rs:297`, and the retry loop is `invoke_remote_at` in the same file. Connection retries are in `crates/router/src/net_iroh.rs:153-156`.
3. Line 2679: "A caller's signed identity proof forwards across a cross-node hop and is re-verified at the destination." True only when the call origin is native. A guest call never forwards the caller's proof. It presents the guest's own instance certificate (`crates/router/src/proxy/router.rs:346-380`).
4. Lines 2715 and 2729: "Today only a service on a node has an outbox." The App Supervisor also keeps a durable outbox of binding writes (`crates/app_supervisor/src/service/queue_worker.rs:4-10`) behind the SDK trait `WriteBindingsOutbox` (`crates/sdk/src/deploy/actor.rs:190`). The supervisor is a native node service, so the sentence may be true. Make it exact: "only a service or the supervisor on a node".
5. Line 2610: execute-ddl "is gated to the elevated lifecycle context and rejected from normal invocations". The gate is the `data-layer/admin` capability (`crates/sandbox_wasm/src/host_capabilities/capabilities_store.rs:512-524`). The lifecycle context carries it. Any other caller with that capability passes too.
6. Line 2608: `aggregate` is described without a limit. Under a policy with column masks or stage-4 ABAC, `aggregate` is refused (`crates/data_db/src/sqlite/query.rs:263-290`).
7. Line 2668: "Subscriptions are persisted by the host ... replayed on startup." A natively linked app's subscriptions are not persisted (`crates/app_host_native/src/factory.rs:325-337`, tracked in the backlog). Add "for WASM services".
8. Line 2712: "The retry limits are one node-wide `retry` policy". The outbox has its own attempt budget (54 attempts, 15 min ceiling) from the supervisor role (`crates/core/src/config/roles.rs:95-108`). Say that the `retry` policy covers the proxy's own retries only.

## Cost notes

- Effort: one long session. About 49 rows with roughly 60 code files opened. Estimated 20 to 25 rows per hour of effort.
- Easy rows (about 30): wit/config constants, enum lists, negative searches. One read each.
- Hard rows: 6.7, 6.8, 6.11 (needed the full dead-letter, outbox and dedup paths, not only the cited lines), 6.13 (needed the third-party croner source), 6.15 (large saga code), 6.24 (needed every HTTP route target, plus the backlog), 7.3 (found a second `reveal` path), 7.12 and 7.13 (many small facts per row).
- The cited evidence was usually right. The row text was often right. Most findings are in the doc sentences, which say more than the row ("only", "each", "does not run twice", "typed").
- Method that found most findings: read the full function next to each cite, then grep for a second caller or a second path (`record_failed_call`, `reveal_secret`, `journal_mode`, `IndexType`).
