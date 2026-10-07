# Fix verification, batch S1 (fix commits 1 to 5)

Reader: the author of the architecture fix, who must correct the doc text.

Scope: 62 rows of `fix-new-claims.md` (commit 1: 9, commit 2: 15, commit 3: 8, commit 4: 12, commit 5: 18). Checked against the code on branch `docs/architecture-fix`. Only static reading. Row id = `<commit>.<n>` in file order.

## Summary

| Verdict | Count |
| --- | --- |
| CONFIRMED | 55 |
| CITE-OFF | 2 |
| PARTLY | 5 |
| WRONG | 0 |
| UNVERIFIABLE | 0 |
| Total | 62 |

## Rows

| Row | Verdict | Note | Checked at |
| --- | --- | --- | --- |
| 1.1 | CONFIRMED | `wrpc` parses to `Wrpc`; the planner sends it to the unsupported-protocol error (-32091). | `crates/router/src/preamble.rs:227`; `crates/router/src/route_handler/dispatch.rs:287-294,352-354`; `crates/rpc/src/proxy.rs:140` |
| 1.2 | CONFIRMED | `authorize-rows` is the stage-4 guest export; ADR-0017 section 7 is the stage-4 heading. | `crates/wit_interfaces/wit/data-layer/authorizer.wit:47`; `crates/fdae/src/trace.rs:153` |
| 1.3 | CONFIRMED | WIT says `put` replaces the whole payload; listing saved with `put`. | `crates/wit_interfaces/wit/data-layer/data-layer.wit:96-104`; `crates/roym_catalog/src/app/listing_ops.rs:232` |
| 1.4 | CONFIRMED | `SynAppManifest` holds a set of services; `roym.toml` declares six. | `crates/app_orchestration/src/models/manifest.rs:14-27`; `crates/roym_core/app/roym.toml:9` |
| 1.5 | CONFIRMED | `pkarr` client publishes to Mainline DHT. No libp2p in any `Cargo.toml` or `Cargo.lock`. | `Cargo.toml:142`; `crates/core/src/dht_registry/client.rs:32-35` |
| 1.6 | CONFIRMED | `importKey(..., false, ...)` makes the key non-extractable; kept in IndexedDB; `session delegate` exists. | `crates/roym_web/ui/src/session/login.ts:128,135`; `apps/roymctl/src/commands/session.rs:29,220` |
| 1.7 | CONFIRMED | Credential, revocation and moderation-decision records exist as Roym record types; no W3C VC code. | `crates/roym_core/src/membership.rs:1-20`; `crates/roym_core/src/record.rs:20-30` |
| 1.8 | CONFIRMED | Publish loop sleeps `HEARTBEAT_INTERVAL_SECS` (3600) between passes. | `crates/core/src/dht_registry/types.rs:20`; `crates/substrate/src/runtime/publish.rs:117` |
| 1.9 | CONFIRMED | Hub is the TypeScript web UI. No tauri or capacitor in any manifest outside lock files. | `crates/roym_web/ui/package.json`; repo-wide grep |
| 2.1 | CONFIRMED | `roym.toml` is the only manifest with `[services.*]`; it has six. | `crates/roym_core/app/roym.toml:9,37,47,60,73,87` |
| 2.2 | CONFIRMED | Vite build; `web` serves bundle and public `POST /rpc`; Bearer header in `rpc.ts` and `login.ts`. | `crates/roym_core/app/roym.toml:19-31`; `crates/roym_web/ui/src/rpc.ts:24-33`; `.../session/login.ts:53-59` |
| 2.3 | CONFIRMED | No tauri, capacitor, electron, rig-core or sqlite-vec in code manifests. | repo-wide grep (only lock-file hits in `test-components`) |
| 2.4 | CONFIRMED | `interface saga` exists and is linked for guests; no saga use in `roym_*`. Trigger is guest `compensate` or deadline. | `crates/wit_interfaces/wit/proxy/proxy.wit:122-178`; `crates/sandbox_wasm/src/host_capabilities/capabilities_proxy.rs:292` |
| 2.5 | CONFIRMED | Seven card types; unlisted type or version renders "Unknown Card"; button calls `agreement.accept`. | `crates/roym_core/src/card.rs:9-17`; `crates/roym_web/ui/src/cards/render.ts:19-24`; `.../screens/messages.ts:548` |
| 2.6 | PARTLY | Roym does compare payment records with the agreed terms and refuses a mismatch. See Findings. | `crates/roym_core/src/payment.rs:150`; `crates/roym_transaction/src/app/payment_ops.rs:503` |
| 2.7 | CONFIRMED | No `PaymentIntent` or gateway code. The `ledger` table is a booking-step ledger, not a credit ledger. | repo-wide grep; `crates/roym_transaction/src/app/ledger.rs:1-25` |
| 2.8 | PARTLY | `delegation` is optional on the envelope. Doc says a record carries it. See Findings. | `crates/signed_record/src/envelope.rs:140-142`; `crates/signed_record/src/verify.rs:187-205` |
| 2.9 | CONFIRMED | No JSON-LD, IPFS or VC code in `crates/`, `apps/`. | repo-wide grep (only `cid` = conversation id) |
| 2.10 | CONFIRMED | Default 3 per 24 h; recipient sets limits and blocks; conversation service calls the check. | `crates/roym_core/src/safety.rs:19-28,129-136`; `crates/roym_profile/src/app.rs:75-82`; `crates/roym_conversation/src/app/inbox.rs:185` |
| 2.11 | CONFIRMED | No stamp, lock, slash or stake code (only unrelated words such as "trailing slash"). | repo-wide grep |
| 2.12 | CONFIRMED | No escrow or dispute workflow; `dispute_path` is a `String`. | `crates/roym_core/src/transaction.rs:137`; repo-wide grep for `escrow`, `dispute`, `arbitrat` |
| 2.13 | CITE-OFF | Claim true. Cited `admit.rs:107` is a test table. Right place is `roym_directory/src/app.rs:74-79`. | `crates/roym_directory/src/app.rs:74-79`; `crates/roym_directory/src/app/publication_ops.rs:194-197,317-334,545-555` |
| 2.14 | CONFIRMED | No keyring, keychain, StrongBox, TPM, APNs, FCM, mobile target or `SecureStorage` WIT. | repo-wide grep |
| 2.15 | CONFIRMED | Durable outbox with attempts and backoff exists; `signing.wit` returns no key material. | `crates/async_queue/src/queue.rs:71-96`; `crates/wit_interfaces/wit/signing/signing.wit:3-7` |
| 3.1 | CONFIRMED | `directory.search` is `WireRule::Open`; `directory.publish` is `VerifiedOnly`; manifest visibility `public`. | `crates/roym_directory/src/app.rs:74-79`; `crates/roym_core/src/directory.rs:169-182`; `crates/roym_core/app/roym.toml:92` |
| 3.2 | CONFIRMED | Constants 8, 3, 2000 ms; `add_source` refuses a ninth; Hub honors `max_concurrency`; timeout is the constant. | `crates/roym_core/src/directory.rs:46,57,70`; `crates/roym_directory/src/app/client_sources.rs:89-90`; `crates/roym_web/ui/src/directory/search.ts:68-76` |
| 3.3 | CONFIRMED | Round-robin merge; `verify_envelope` per hit; membership evaluated locally; hit carries no verdict. | `crates/roym_directory/src/app/client_merge.rs:30,141`; `.../client_query.rs:162,320-345` |
| 3.4 | CONFIRMED | No `ttl`/`hop`/hierarchical tag. Queries carry flat `categories`, not tags. No outbound forward in server half. | `crates/roym_core/src/directory.rs:169-182`; `crates/roym_directory/src/app/client_query.rs:132` |
| 3.5 | CONFIRMED | No `reputation`, `rating`, `score`, `ema` in `roym_*` source or UI. | repo-wide grep over `crates/roym_*` |
| 3.6 | CONFIRMED | Two receipt types, per-party halves. Agreement completeness is `pair_state`; fulfilment completeness is derived in `fulfilment_ops.rs:234`. | `crates/roym_core/src/transaction.rs:9-11,205-237`; `crates/roym_transaction/src/app.rs:186-193`; `.../fulfilment_ops.rs:234` |
| 3.7 | CONFIRMED | `record_id` = `rec_` + z-base-32 SHA-256 of the canonical envelope. | `crates/signed_record/src/envelope.rs:199-229` |
| 3.8 | CONFIRMED | Each half is one issuer, one signature, no condition; complete when both exist. | `crates/roym_core/src/transaction.rs:9-11,230-237` |
| 4.1 | CONFIRMED | Router and WASM engine call `metrics::`; `client_gateway` and `auth` have no `metrics` dependency or call. | `crates/router/src/route_handler/dispatch.rs:163,188`; `crates/sandbox_wasm/src/engine/lifecycle.rs:175`; `crates/client_gateway/Cargo.toml` |
| 4.2 | CONFIRMED | `MemoryRecorder` holds `DashMap<_, Arc<Mutex<_>>>` for counters, gauges, histograms. | `crates/observability/src/recorder.rs:23-27` |
| 4.3 | CONFIRMED | Metrics route serves the recorder snapshot as JSON when `metrics.enabled`. | `crates/substrate/src/runtime/services.rs:390-410` |
| 4.4 | CONFIRMED | `init` sets logging, installs recorder, starts 1 s `SystemSampler`. | `crates/observability/src/engine.rs:79-90` |
| 4.5 | CITE-OFF | Claim true. Evidence says "no `mpsc` in crates/ and apps/", which is false (other crates use it). `observability` has none. | `crates/router/src/route_handler/http/websocket.rs:231`; `crates/observability/Cargo.toml` |
| 4.6 | CONFIRMED | No ollama, candle, rig, concierge, sqlite-vec, inference or embedding code. | repo-wide grep |
| 4.7 | CONFIRMED | `ProxyRouter` implements the Universal Proxy. | `crates/router/src/proxy.rs:1-6` |
| 4.8 | CONFIRMED | `mise.toml` builds guests with `cargo component build`. The only `build.rs` is `coordinator_webrtc` (not a guest). | `mise.toml:10,56-58,76`; `crates/coordinator_webrtc/build.rs:1` |
| 4.9 | CONFIRMED | `AppCommands::Deploy` takes a manifest or `.wasm`. | `apps/roymctl/src/commands/app.rs:127-135` |
| 4.10 | CONFIRMED | `roym` feature pulls `syneroym-app-host-native` and all six `syneroym-roym-*` crates; trait/impl crate headers match. | `crates/substrate/Cargo.toml:37-46,95-104`; `crates/app_host/src/lib.rs:1-5`; `crates/app_host_native/src/lib.rs:1-4` |
| 4.11 | CONFIRMED | `saga` WIT says steps are walked backwards, `saga-undo-<method>`. Failure is reported by the guest (`compensate`) or by deadline. | `crates/wit_interfaces/wit/proxy/proxy.wit:111-125,144,171-178` |
| 4.12 | CONFIRMED | No `syneroym-dev-sdk` or `synapp-template` outside docs; `test-components/` holds guests. | repo-wide grep; `test-components/` listing |
| 5.1 | CONFIRMED | `connect_with_mechanisms` dials Iroh; `WebRtc` arm is empty. Scope is control-plane calls (other `roymctl roym ...` commands use the HTTP gateway). | `crates/sdk/src/client.rs:364-407` |
| 5.2 | CONFIRMED | `resolve` is the one outward verb; only `roymctl`, client gateway and WebRTC coordinator call it; no guest does. | `crates/wit_interfaces/wit/supervisor/supervisor.wit:315-326`; `crates/app_supervisor/src/service/resolve.rs:1-25`; `crates/sdk/src/topology.rs:123` |
| 5.3 | CONFIRMED | 17 `func` verbs in the interface; doc comments match each description. | `crates/wit_interfaces/wit/supervisor/supervisor.wit:162,182,185,193,197-198,210,220,241,243-257,325` |
| 5.4 | CONFIRMED | `check_generation` refuses `Less` and same-generation other writer. Called for bindings, undeploy, restart, scheduled run, cert renewal (and deploy). | `crates/control_plane/src/service/orchestration/backends.rs:101-140`; `.../lifecycle.rs:133,214-225,256,529`; `.../cert.rs:129` |
| 5.5 | CONFIRMED | Vault in supervisor; `submit`/`adopt` return DIDs only; `master_backup_dir` is operator config. | `crates/app_supervisor/src/keys.rs:1-25,454`; `supervisor.wit:19-34,199-220`; `crates/core/src/config/roles.rs:155` |
| 5.6 | CONFIRMED | Supervisor starts only if `[roles.supervisor]` is set (and Cargo feature `supervisor`, default-on). | `crates/core/src/config/roles.rs:19`; `crates/substrate/src/runtime/supervisor.rs:39-43`; `crates/substrate/Cargo.toml:55-66` |
| 5.7 | CONFIRMED | `Reconcile`, `Health` with `--watch`, alert store (`--no-record` to skip), `Alerts`. | `apps/roymctl/src/commands/app.rs:161-231` |
| 5.8 | PARTLY | The tables listed exist. The file also holds the deployment journal, alerts and the outbox. See Findings. | `crates/app_supervisor/src/store.rs:75-83,102-140`; `.../resident_loop/pass.rs:76-80` |
| 5.9 | PARTLY | Member keys are minted at `submit`, which runs before `adopt`. "Before adopt" is too late for member keys. See Findings. | `crates/app_supervisor/src/service/verbs/submit.rs:236`; `.../verbs/lifecycle.rs:49-58` |
| 5.10 | CONFIRMED | `classify_binding_write` gives the four outcomes; called per dependent on write. "Same membership" also compares mode and sharding strategy. | `crates/app_orchestration/src/resolver/types.rs:127-175`; `crates/control_plane/src/service/orchestration/lifecycle.rs:178` |
| 5.11 | CONFIRMED | App master minted by `adopt`, delegates nothing, signs Tier-1 `EndpointInfo` (needs a configured registry). | `crates/app_supervisor/src/keys.rs:5-10`; `crates/app_supervisor/src/tier1.rs:1-9` |
| 5.12 | CONFIRMED | Document fields, unknown/unauthorized same refusal, paused answers, retired refused. | `supervisor.wit:263-326`; `crates/app_supervisor/src/service/resolve.rs:30-77` |
| 5.13 | CONFIRMED | `AppScope::{Local,Foreign}`; `roymctl app resolve` fetches, calls `verify(&app_did)`, prints members. Implementation is in `app/resolve.rs`. | `crates/app_orchestration/src/resolver/types.rs:305-308`; `apps/roymctl/src/commands/app/resolve.rs:14-48` |
| 5.14 | PARTLY | Hook choice and error handling are right. A byte-identical redeploy of a running service skips deploy, so no hook runs. See Findings. | `crates/sandbox_wasm/src/engine/lifecycle.rs:114-129,140-160`; `crates/control_plane/src/service/orchestration/deploy.rs:41-48` |
| 5.15 | CONFIRMED | `local_elevated` grants `data-layer/admin` on own resource; `execute-ddl` checks it. | `crates/rpc/src/native.rs:89-108`; `crates/sandbox_wasm/src/host_capabilities/capabilities_store.rs:512-522` |
| 5.16 | CONFIRMED | No pause, snapshot, restore or replication epoch; code comment states no rollback. | `crates/sandbox_wasm/src/engine/lifecycle.rs:119-126` |
| 5.17 | CONFIRMED | ALPN `syneroym/0.1`; `Wrpc`/`Other` give typed unsupported-protocol error; no profile exchange. | `crates/router/src/connection_router.rs:48`; `crates/router/src/route_handler/dispatch.rs:287-294,348-354` |
| 5.18 | CONFIRMED | Envelope and identity backup refuse unknown versions. Master Anchor refuses non-`master_anchor_v1` through `verify_signature` (generic error). | `crates/signed_record/src/verify.rs:159-161`; `crates/identity/src/backup.rs:215-216`; `crates/core/src/dht_registry/master_anchor.rs:100-139` |

## Findings

### 2.6 (PARTLY) Phase 6 > 3 Payments

Doc words: "Roym does not process payments, hold money or check a payment. It records what each side says."

Code: Roym never checks that money moved. But it does check each payment record against the agreed terms. `matches_terms` compares currency, amount and method (`crates/roym_core/src/payment.rs:150-165`). `acknowledge` refuses a method not in the terms with `method-not-in-terms` (`crates/roym_transaction/src/app/payment_ops.rs:503-510`). The sync path refuses a received record with `amount-mismatch` (`crates/roym_transaction/src/app/sync/receipts.rs:185-186,460-461`). "Check a payment" can be read as either.

Proposed text: "Roym does not process payments or hold money. It does not check that money moved. It records what each side says. It only checks that a payment record matches the agreed amount, currency and method."

### 2.8 (PARTLY) Phase 6 > 4 Portable Data

Doc words: "A signed Roym record carries its issuer, its signature and its delegation certificate."

Code: `delegation` is `Option<String>` (`crates/signed_record/src/envelope.rs:140-141`). When it is absent, the issuer key signed the record itself (`crates/signed_record/src/verify.rs:187-205`, the `None => e.issuer.clone()` arm).

Proposed text: "A signed Roym record carries its issuer and its signature. If a delegated key signed it, the record also carries the delegation certificate. Any node verifies it with the same code, without contacting the issuer."

### 2.13 (CITE-OFF) Phase 6 > 7 Aggregator

The claim and the doc text are right. The cite `crates/roym_core/src/admit.rs:107` points to a unit-test table. Use `crates/roym_directory/src/app.rs:74-79` (`WIRE_REACHABLE`). The rate limit is keyed on `published_by`, one publisher per connection (`publication_ops.rs:185-199`). The limit is settable through `set_limits` (`publication_ops.rs:545-555`). No doc change needed.

### 4.5 (CITE-OFF) [ADV-OBS] Envisioned

The claim is true. The evidence text "mpsc in crates/ and apps/ (no hit)" is false: `mpsc` appears in `crates/router/src/route_handler/http/websocket.rs:231` and `crates/substrate/src/runtime/publish.rs`. None is a metrics pipeline. Correct the evidence to: "no `mpsc` in `crates/observability`; no metrics channel anywhere". No doc change needed.

### 5.8 (PARTLY) [LFC-MGT] 2 > Authoritative Ledger

Doc words: "The supervisor's SQLite database stores the Desired State (the compiled plan and the substrate inventory of each managed instance), the binding epoch last written to each dependent, and the restart counters for bounded remediation. Actual State (health) is read from the substrates on each pass ... and is not stored."

Code: the same file also holds the deployment journal, the alerts and the durable outbox queue (`crates/app_supervisor/src/store.rs:75-83`). It also has tables for anchor refresh, Tier-1 refresh, topology epochs, revoked placements, pending rotation restarts and schedule runs (`store.rs:86-100`). Alerts come from the health sweep and are stored (`resident_loop/pass.rs:76-80`, `record_pass_health`). Current per-service health is not stored.

Proposed text: "The supervisor's SQLite database stores the Desired State (the compiled plan and the substrate inventory of each managed instance), the binding epoch last written to each dependent, the restart counters for bounded remediation, the deployment journal, the alerts and the outbox of binding writes. The current health of each service is read from the substrates on each pass and is not stored. Alerts raised from it are stored."

### 5.9 (PARTLY) [LFC-MGT] 2 > Authoritative Ledger

Doc words: "a new supervisor runs `import-master` for each key before `adopt`, and without the backups it mints new master keys."

Code: member master keys are minted by `submit` (`crates/app_supervisor/src/service/verbs/submit.rs:236`, `keys::mint_and_substitute` -> `get_or_mint`, `keys.rs:234-260`). `adopt` needs a prior `submit` ("run `supervisor submit` first", `verbs/lifecycle.rs:49-58`). So member keys must be imported before `submit`. Only the app instance key is minted by `adopt` (`supervisor.wit:176-181,211-219`).

Proposed text: "A rebuild needs the master-key backups made with `export-master`. A new supervisor runs `import-master` for each member key before the first `submit`, because `submit` mints any member key it does not find. It runs `import-master` for the app instance key before `adopt`. Without the backups, the supervisor mints new master keys."

### 5.14 (PARTLY) [LFC-VER] 1 > Built today

Doc words: "On every deploy the substrate calls a hook that the guest exports."

Code: a deploy that is identical to what is installed and running returns early as a no-op (`crates/control_plane/src/service/orchestration/deploy.rs:41-48`, `deploy_is_redundant_noop`). It never reaches `deploy_wasm`, so no hook runs. The hook also runs only for WASM services (`sandbox_wasm/src/engine/lifecycle.rs:74`).

Proposed text: "When a WASM service is deployed, the substrate calls a hook that the guest exports. It calls `init()` when the service has no database yet (a first deploy) and `migrate()` when the service already has one (a re-deploy). A deploy that is identical to the running service does nothing, so no hook runs. A component that does not export the hook is skipped. If the hook returns an error, the deploy fails with that error."

## Doc text not covered by any row

1. `docs/system-architecture.md:2762`: "It exposes one interface, `supervisor`, consumed by `roymctl`" and "nothing queries it on the hot path". The client gateway and the WebRTC coordinator also call `supervisor.resolve` (Tier 2) for app-scoped hostnames, with a cache (`crates/sdk/src/topology.rs:100-135`; `crates/client_gateway/src/gateway.rs:159-165`; `crates/coordinator_webrtc/src/bootstrap.rs:40`). Suggest: "consumed by `roymctl`; the client gateway and the WebRTC coordinator call only `resolve`".
2. `docs/system-architecture.md:2790`: "an unauthorized caller get the same refusal". The code lets any caller resolve a service that the app declares with `topology_visibility = "open"` (`crates/app_supervisor/src/service/resolve.rs:43-62`; `crates/roym_core/app/roym.toml:93`). The doc does not say that an `open` service needs no grant. The Roym `directory` is `open`.
3. `docs/system-architecture.md:2906`: "Each client, SynOrg, directory or aggregator chooses which directories it queries." Only the per-installation source list exists (`crates/roym_directory/src/app/client_sources.rs:41,89`). The directory server half makes no outbound `directory.search` call. Only the client half does (`client_query.rs:132`; `held.rs:93`). Unproven for "directory or aggregator".
4. `docs/system-architecture.md:2771`: "The supervisor is enabled by configuration". It also needs the Cargo feature `supervisor` (default-on; the `minimal` feature set leaves it out): `crates/substrate/Cargo.toml:55-66`, `crates/substrate/src/runtime/supervisor.rs:21,26`.
5. `docs/system-architecture.md:2761`: "directly initiates connections to target substrates over Iroh". True for deploy and control-plane calls. Other `roymctl` command groups (`roym`, `session`, `registry`) call the client gateway over HTTP (`apps/roymctl/src/commands/roym/backup.rs:21,56`). Say "for deployment commands".
6. `docs/system-architecture.md:2776`: "designed to be rebuildable ... from the manifests the operator holds plus a sweep of the target substrates". No sweep code exists. `adopt` reads only the held generation of each substrate (`crates/app_supervisor/src/service/verbs/lifecycle.rs:10-36`). Rebuild is manual: `submit` again, then `adopt`.
7. `docs/system-architecture.md:2954` and `:2893`: "the substrate walks the compensations backwards if the workflow fails". The substrate never decides that a workflow failed. The guest calls `compensate`, or the saga deadline passes and the substrate compensates (`crates/wit_interfaces/wit/proxy/proxy.wit:112-114,143-146,171-178`). Suggest: "when the guest gives up, or the saga deadline passes".
8. `docs/system-architecture.md:2800`: the hook description covers WASM only. Native-build (`roym` feature) and container services run no `init()`/`migrate()` through this path (`crates/sandbox_wasm/src/engine/lifecycle.rs:74`). Not stated.
9. `docs/system-architecture.md:2769`: the list "binding writes, undeploy, restart and certificate renewal" leaves out `deploy`, `claim-app-instance` and scheduled runs, which are also gated by `check_generation` (`deploy/admission.rs:154`; `app_instance.rs:59,99`; `lifecycle.rs:596`). The list is true but reads as complete.

## Cost notes

- About 62 rows in one long pass. Most rows took 1 to 3 tool calls.
- Rate: roughly 15 to 20 rows per hour for the hard rows, 40 or more per hour for the confirmed negative searches and constant checks.
- Hard rows: 2.6 (need to trace `matches_terms` to its callers), 5.4 (list which handlers call `check_generation`), 5.8 and 5.9 (needed the store schema and the `submit`/`adopt` order), 5.14 (needed the deploy dedup path), 3.6 (the "complete" rule differs for the two receipt types).
- Negative rows (2.3, 2.7, 2.9, 2.11, 2.14, 4.6, 4.12) were cheap. A grep with several synonyms and a word-boundary flag was enough. Plain substring greps give false hits (`upi` in `input`, `stamp` in "management stamp").
- The rows' own evidence was mostly right. Wrong or loose cites: 2.13, 4.5. The risk was in the doc sentences that go beyond the row (2.8, 5.8, 5.9, 5.14).
