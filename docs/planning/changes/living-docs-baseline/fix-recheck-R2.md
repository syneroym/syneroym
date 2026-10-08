# Fix recheck, part R2

Reader: the maintainer of `docs/system-architecture.md`.

Scope: Task A checks the 60 new rows of `fix-new-claims.md` with labels S5 to S9. Task B checks the findings of `fix-verification-S5.md` to `S9.md`. All work was static reading. Line numbers of the architecture doc are from the branch `docs/architecture-fix` as it is now.

## 1. Summary

Task A (60 rows): CONFIRMED 55, CITE-OFF 2, PARTLY 3, WRONG 0, UNVERIFIABLE 0.

| Label | Rows | CONFIRMED | CITE-OFF | PARTLY |
| --- | --- | --- | --- | --- |
| S5 | 12 | 12 | 0 | 0 |
| S6 | 12 | 11 | 0 | 1 (S6.4) |
| S7 | 7 | 7 | 0 | 0 |
| S8 | 11 | 10 | 1 (S8.1) | 0 |
| S9 | 18 | 15 | 1 (S9.2) | 2 (S9.10, S9.15) |

Task B (75 items): FIXED 70, STILL-WRONG 3, CODE-GAP-ONLY 2.

| Report | Items | FIXED | STILL-WRONG | CODE-GAP-ONLY |
| --- | --- | --- | --- | --- |
| S5 | 16 | 13 | 3 | 0 |
| S6 | 11 | 11 | 0 | 0 |
| S7 | 13 | 13 | 0 | 0 |
| S8 | 15 | 15 | 0 | 0 |
| S9 | 20 | 18 | 0 | 2 |

The 3 STILL-WRONG items: line 820 of the doc (old 24-hour anchor claim), `config.sample.toml` and `config.dev.toml` ("Metrics (Prometheus)"), and `AGENTS.md` ("Prometheus `/metrics`"). The last two are outside the architecture doc.

## 2. Task A table

| Row | Verdict | Note | Checked at |
| --- | --- | --- | --- |
| S5.1 | CONFIRMED | vodozemac docs say 3DH. Doc has no "X3DH" left. | `vodozemac-0.10.0/src/olm/mod.rs:16-33`; `Cargo.toml:162`; `crates/conversation/src/crypto.rs:319,386` |
| S5.2 | CONFIRMED | Iroh verifier lists only TLS 1.3. | `iroh-0.97.0/src/tls/verifier.rs:19-20` |
| S5.3 | CONFIRMED | session-auth only at login; router takes only routing and service-instance. | `apps/roymctl/src/commands/session.rs:237`; `crates/auth/src/service.rs:262`; `identity.rs:240-262`; `crates/identity/src/delegation.rs:30` |
| S5.4 | CONFIRMED | Router checks `revoked_keys`; only `revoke_instance_key` adds; publish-anchor sends empty list. | `crates/router/src/handshake.rs:71`; `identity.rs:272`; `client.rs:540-554`; `anchors.rs:74`; `session.rs:31-32` |
| S5.5 | CONFIRMED | Zero DEK is used when encryption is off. Default is on. | `crates/data_db/src/sqlite/provider.rs:196-198,277-285`; `service_store.rs:174-182`; `base.rs:36,48`; `runtime/router.rs:338` |
| S5.6 | CONFIRMED | Names use `#`. | `crates/app_supervisor/src/keys.rs:350,387`; `member_identity.rs:45` |
| S5.7 | CONFIRMED | `--recovery-key-out` exists on both commands. | `identity.rs:365-367`; `roym/backup.rs:184-186` |
| S5.8 | CONFIRMED | `verify` checks 24 hours. Only the HTTP branch calls it. | `master_anchor.rs:151-165`; `client.rs:309,329-355` |
| S5.9 | CONFIRMED | Container gets native endpoints; no db path in `podman run`. | `deploy.rs:79-91`; `synsvc_native/data.rs:171`; `engine.rs:249-263`; `local_registry.rs:42-47` |
| S5.10 | CONFIRMED | Publish adds `svc/supervisor/`; subscribe rule matches. Default topic matches. | `service.rs:660-663`; `mqtt_broker/src/lib.rs:70,80`; `dispatch.rs:524-529`; `roles.rs:73` |
| S5.11 | CONFIRMED | Counter fires when attempts are a multiple of 20. | `crates/conversation/src/outbox.rs:409-424` |
| S5.12 | CONFIRMED | Test shows scheduled and conflict with slot-taken. | `roym_web/tests/dual_build_parity/booking.rs:321,340` |
| S6.1 | CONFIRMED | Node returns 3; Hub applies it. Node limit is a separate permit count of 4. | `roym_core/src/directory.rs:46-57`; `client_query.rs:117`; `search.ts:189` |
| S6.2 | CONFIRMED | Busy node gives 503; Hub retries once. | `search.ts:88,120,194-196`; `router/src/route_handler/http/guest.rs:156-161` |
| S6.3 | CONFIRMED | `start_run` prunes rows older than 3600 s. | `directory.rs:76`; `client_query.rs:79-100` |
| S6.4 | PARTLY | Two verifiers read signed records without a type check. See finding F1. | `moderation_ops.rs:131`; `roym_profile/src/app/backup.rs:25` |
| S6.5 | CONFIRMED | Twelve entries; no reader; booking-progress outside. | `roym_core/src/record.rs:18-31,44-53`; repo grep |
| S6.6 | CONFIRMED | Only `IrohHop` implements `RemoteHop`; SDK WebRTC is a stub. | `proxy/hop.rs:14-20,48`; `route_handler.rs:292`; `sdk/src/client.rs:404-406` |
| S6.7 | CONFIRMED | `PUBLIC_METHODS` has only `profile.policy`; all routes are Owner. | `roym_web/src/app.rs:130-134`; `roym_core/src/router.rs:11-67` |
| S6.8 | CONFIRMED | All six `status` exports are open; `invoke` gates. | `roym_transaction/src/app.rs:212,221`; `roym_web/src/app.rs:20,134,239`; `roym_directory/src/app.rs:81` |
| S6.9 | CONFIRMED | Wire table has four methods; client half calls each. | `roym_directory/src/app.rs:75-78,184`; `client_query.rs:131-135`; `client_sources.rs:39-41,245`; `held.rs:89-93` |
| S6.10 | CONFIRMED | `maybe_countersign` is called only from `sync.rs`. | `roym_transaction/src/app/sync.rs:567`; `agreement_ops.rs:405-440`; `messages.ts:329`; `roymctl/.../transaction.rs:453` |
| S6.11 | CONFIRMED | Consumer fulfilment half is against interest and acknowledges at once. | `fulfilment_ops.rs:249-270`; `roym_core/src/booking.rs:259-276`; `sync/receipts.rs:419-428` |
| S6.12 | CONFIRMED | `call_peer` is used only for `prekey-bundle` and `deliver`. | `conversation/src/transport.rs:136,225`; `roym_transaction/src/app.rs:560-585` |
| S7.1 | CONFIRMED | `verify_envelope` uses `VerifyOptions::new`; revocation source is empty. | `listing.rs:505-534`; `signed_record/src/verify.rs:19-27,85-91,101,564-634` |
| S7.2 | CONFIRMED | `with_revocations` only in tests; crate has no sign function. | `verify.rs:101,266`; `signed_record/src/lib.rs:4-9` |
| S7.3 | CONFIRMED | DHT read and write for both record kinds. | `dht_registry/client.rs:240-246,333-358,402-413` |
| S7.4 | CONFIRMED | Three end states to `failed`; 30 days default. | `outbox.rs:195-205,229-245`; `transport.rs:50-58,75-86`; `sandbox.rs:210-214,334` |
| S7.5 | CONFIRMED | DHT branch needs `resolve_did_key`; else warning. | `client.rs:240-259` |
| S7.6 | CONFIRMED | Only supervisor revokes; roymctl has no other revoke path. | `client.rs:540-554`; `anchors.rs:73-75`; `supervisor.rs:97,402`; `identity.rs:272` |
| S7.7 | CONFIRMED | No writer without registry; vault check; 12 h; registry is `DashMap`. | `anchors.rs:55-65`; `renewal.rs:422-452`; `roles.rs:79-84`; `community_registry/src/registry.rs:59,70,98` |
| S8.1 | CITE-OFF | Doc is right. Other-node agreement gives status `None`, not `Unverified`. Cite 270-278 too. | `identity/src/substrate.rs:270-278,309-360`; `runtime/router.rs:53-92,332-345` |
| S8.2 | CONFIRMED | `substrate/admin` entails all; `security` needs it. | `ucan/src/capability.rs:142-145`; `control_plane/src/service/dispatch.rs:64-72` |
| S8.3 | CONFIRMED | Preamble has pubkey, no signature; only `passthrough_with_conn` used. | `gateway.rs:517-532`; `sdk/src/client.rs:77-84,662`; `preamble.rs:171-195` |
| S8.4 | CONFIRMED | `assertion` is built only from `master_did`. | `auth/src/service.rs:213,234-238`; `login.ts:196-199` |
| S8.5 | CONFIRMED | `local` reads the key file; only `Origin` gives 403. | `service.rs:343-415,622-630` |
| S8.6 | CONFIRMED | In-memory map; returns when full. | `service.rs:29,118,142,194-196` |
| S8.7 | CONFIRMED | Gateway-origin test is node DID and no delegation. | `http/auth.rs:84-91`; `handshake.rs:76-81` |
| S8.8 | CONFIRMED | `instance_cert_expiry_sweep_loop` returns `!` and is in `select!`. | `services.rs:250,289`; `publish.rs:130` |
| S8.9 | CONFIRMED | Hook skipped without export; failure returns error. | `lifecycle.rs:127-129,150-153` |
| S8.10 | CONFIRMED | `timeout_ms: u32`, default 10 s, max 30 s. | `schedule.rs:25,34,66`; `manifest.rs:149` |
| S8.11 | CONFIRMED | Both features list `coordinator`. | `crates/substrate/Cargo.toml:67-69` |
| S9.1 | CONFIRMED | Raw to WASM has no adaptation; TCP gets `TcpProxy`. | `preamble.rs:20`; `dispatch.rs:332-347` |
| S9.2 | CITE-OFF | True. "while none exists" is at `iroh-0.97.0/src/lib.rs:101-104`, not 58-66. | `iroh-0.97.0/src/lib.rs:58-66,101-104`; `coordinator.rs:120-121` |
| S9.3 | CONFIRMED | Test names coordinator; tunnel is a blind pipe to the substrate. | `multi_hop_relay.rs:586-594`; `bootstrap/tunnel.rs:21-43,157-250` |
| S9.4 | CONFIRMED | `KeyStore` has KEK, DEK generate and load; vault export exists. | `data_keystore/src/key_store.rs:29-31,119,152`; `keys.rs:300` |
| S9.5 | CONFIRMED | Three backup commands; no other data backup command found. | `roym/backup.rs:46-60`; `identity.rs:133-145`; `supervisor.rs:77-85` |
| S9.6 | CONFIRMED | TCP restart error and NativeHost mapper error match. | `lifecycle.rs:544-547`; `sdk/src/mapper.rs:320-322` |
| S9.7 | CONFIRMED | `custom_config.image` overrides `source`. | `mapper.rs:264,270-272` |
| S9.8 | CONFIRMED | Lib docs: first via home relay, then hole punching. | `iroh-0.97.0/src/lib.rs:58-66` |
| S9.9 | CONFIRMED | Relay only if `enable_relay`; endpoint and info server whenever `role.iroh`. | `coordinator.rs:120-146` |
| S9.10 | PARTLY | Config matches relay lib. But one test does set `role.tls` (path test). See F2. | `coordinator_iroh/src/config.rs:63,103`; `iroh-relay-0.97.0/src/server.rs:186,371,428`; `core/src/config/tests.rs:20` |
| S9.11 | CONFIRMED | Endpoint uses parent relay; SDK dials record relay; no block test exists. | `connection_router.rs:77-92`; `sdk/src/client.rs:366-384`; `multi_hop_relay.rs:586-594` |
| S9.12 | CONFIRMED | Own record registered; no entry-point field. | `coordinator.rs:148-166`; `dht_registry/types.rs:53-91` |
| S9.13 | CONFIRMED | Field `_parent_relay_url` is never read; per-target connect. | `coordinator.rs:69-76,201-207,245`; `route_handler.rs:106,415,458`; `io.rs:494` |
| S9.14 | CONFIRMED | Preamble fields exist. | `crates/router/src/preamble.rs:183,186,190` |
| S9.15 | PARTLY | Tunnel and coordinator do not call `relay_to_next_hop`. The data channel ends in a handler that can. See F3. | `bootstrap.rs:39` (row says 36); `coordinator.rs:72`; `connection_router.rs:122-127,426`; `route_handler/io.rs:317,471` |
| S9.16 | CONFIRMED | Other strings map to Everyone. | `coordinator_iroh/src/config.rs:83,98`; `core/src/config/roles.rs:50-53` |
| S9.17 | CONFIRMED | Dev config matches. | `crates/substrate/src/main.rs:95,190-232` |
| S9.18 | CONFIRMED | Three DHT rules hold. | `dht_registry/client.rs:150-151,212-215,242-260` |

## 3. Task B table

| Report row | Verdict | Note | Checked at |
| --- | --- | --- | --- |
| S5 14.28 vault name | FIXED | Doc has `#` form. | doc:1599 |
| S5 14.12 X3DH | FIXED | No X3DH in doc. Olm and 3DH text in six places. | doc:912,921,937,1557,1573,1857 |
| S5 14.16 who sets `enc` | FIXED | Tunnel sets it; data channel removes it. | doc:316,1580,1811 |
| S5 14.24 revoking a key | FIXED | Table now says roymctl has no command. | doc:818,1595 |
| S5 14.29 Never stored | FIXED | "Syneroym never keeps a copy". | doc:1600 |
| S5 14.31 24-hour anchor | STILL-WRONG | Line 820 states the 24-hour rule with no DHT exception. See F4. | doc:820 (fixed at 811, 1604, 2506) |
| S5 14.8 container database | FIXED | Diagram and Podman bullet fixed. | doc:1641,1680 |
| S5 15.4 metric list | FIXED | `admission.stuck` added. | doc:1420 |
| S5 15.19 no scenario | FIXED | "No such simulation scenario" plus the test. | doc:1538 |
| S5 U1 T1 TLS 1.3 | FIXED | Row S5.2 now covers it. | doc:1551 |
| S5 U2 delegate scopes | FIXED | Table names each command and scope. | doc:1595 |
| S5 U3 DEK zero key | FIXED | Sentence added. | doc:1598 |
| S5 U4 R1 box | FIXED | "when storage.encryption is on". | doc:1563 |
| S5 U5 alert prefix | FIXED | Full topic named. | doc:1431 |
| S5 U6 sample config "Prometheus" | STILL-WRONG | Outside the doc. Endpoint answers JSON. | `config.sample.toml:253`; `config.dev.toml:185`; `services.rs:400` |
| S5 U7 AGENTS.md "Prometheus" | STILL-WRONG | Outside the doc. | `AGENTS.md:250` |
| S6 16.4 3 in flight | FIXED | Node reports 3; Hub keeps it; node does not enforce. | doc:1298 |
| S6 16.14 fixed table | FIXED | New text; see A.S6.4 for one small gap. | doc:1342 |
| S6 16.17 cite only | FIXED | Doc text true. Nothing to change. | doc:1353 |
| S6 16.21 Iroh or WebRTC | FIXED | "Iroh stream; not WebRTC today". Diagram says Iroh. | doc:1357,1382 |
| S6 16.24 every Hub method | FIXED | Wording has "that it forwards" and whoami. | doc:1089,1392 |
| S6 17.7 -32013 | FIXED | `status` open stated. | doc:509,1096 |
| S6 U1 fulfilment | FIXED | Matches code. | doc:1260 |
| S6 U2 transaction.sync | FIXED | "when transaction.sync runs". | doc:1163 |
| S6 U3 only through cards | FIXED | "through the conversation, which carries the cards". | doc:1211 |
| S6 U4 four directory methods | FIXED | All four named. | doc:1096 |
| S6 U5 one hour runs | FIXED | "deletes runs older than one hour when next search starts". | doc:1266,1298 |
| S7 18.3 revocation checks | FIXED | Status is always `unknown`; Envisioned marker. | doc:865,867 |
| S7 18.6 X3DH | FIXED | Same as S5 14.12. | doc:912,937,1857 |
| S7 18.8 pending to failed | FIXED | Three causes listed. | doc:939 |
| S7 19.19 alias at DHT | FIXED | Alias only at the registry. | doc:809,837,1969 |
| S7 19.21 revoke stolen key | FIXED | Supervisor only; Envisioned for a person's key. | doc:813-818,847 |
| S7 19.22 12-hour republish | FIXED | Conditions added. | doc:820 |
| S7 U1 no revocation source | FIXED | Sentence added. | doc:789 |
| S7 U2 DHT carries anchors | FIXED | "endpoint records and master anchors". | doc:861 |
| S7 U3 pubkey P-256 vs Ed25519 | FIXED | "One field, two uses" matches code. | doc:1583; `handshake.rs:29-37,416-424`; `encryption.rs:292-303` |
| S7 U4 registry memory-only | FIXED | Stated. | doc:820 |
| S7 U5 delegate lifetimes | FIXED | Optional. Doc is correct. | doc:726 |
| S7 U6 guest cannot sign | FIXED | "key that the node holds". | doc:789 |
| S7 U7 aggregator queries | FIXED | Phrase is gone. | doc:865 |
| S8 20.13 admin_ucan_root | FIXED | "no verified agreement". | doc:512 |
| S8 20.21 gateway signs | FIXED | "names ... does not sign". | doc:651 |
| S8 20.27 challenge text | FIXED | Client names master DID. | doc:663 |
| S8 20.38 archive cite | FIXED | Doc text true. Nothing to change. | doc:571 |
| S8 20.45 timeout and cron | FIXED | `timeout_ms`; five fields plus optional. | doc:703,706,2730 |
| S8 U1 "local" gateway | FIXED | Word is gone from the doc. | doc:490,651 |
| S8 U2 logout list | FIXED | In memory, 10,000. | doc:668 |
| S8 U3 `local` login warning | FIXED | "no proof step". | doc:668 |
| S8 U4 same-node gateway | FIXED | Stated. | doc:670 |
| S8 U5 expiry loop in select | FIXED | Listed. | doc:674 |
| S8 U6 Docker PID 1 | FIXED | Doc states "no handler for SIGTERM". The Docker effect was only an inference. | doc:676 |
| S8 U7 init if exported | FIXED | Stated. | doc:687 |
| S8 U8 substrate/admin list | FIXED | "every node-wide ability". | doc:512 |
| S8 U9 feature `coordinator` | FIXED | Stated. | doc:710 |
| S8 U10 WAL pragma | FIXED | "no WAL pragma on state.db". | doc:609,2629 |
| S9 21.5 separate parts | FIXED | Endpoint always runs; relay only with flag. | doc:284 |
| S9 21.7 relay TLS port | CODE-GAP-ONLY | Doc says what code does; row exists in backlog. | doc:280; `deferred-backlog.md:273` |
| S9 21.12 inbound-blocked | FIXED | "No test blocks inbound traffic". | doc:310 |
| S9 21.13 entry point | FIXED | SDK call or coordinator's own record. | doc:312 |
| S9 21.14 outbound-only | FIXED | Parent relay; accepts inbound. | doc:312 |
| S9 22.1 only as a fallback | FIXED | Rewritten. | doc:129 |
| S9 22.6 key stores label | FIXED | New label. | doc:194 |
| S9 22.8 backup | FIXED | Keys have own backups. | doc:208 |
| S9 23.1 JSON-RPC everywhere | FIXED | "only RPC wire protocol"; raw and TCP named. | doc:8 |
| S9 21.8 cite | FIXED | Doc text true. Nothing to change. | doc:129 |
| S9 22.15 cite | FIXED | Doc text true. Nothing to change. | doc:1089 |
| S9 U1 Iroh direct first | FIXED | "first contact through the relay". | doc:256 |
| S9 U2 share no code | FIXED | Old phrase gone. New phrase has a small gap, see F3. | doc:322 |
| S9 U3 access string | FIXED | Added. | doc:365 |
| S9 U4 dev defaults | FIXED | Added. | doc:368 |
| S9 U5 preamble fields | FIXED | Added. | doc:316 |
| S9 U6 DHT rules | FIXED | Added. | doc:380 |
| S9 U7 TCP and native-host | FIXED | Added. | doc:240 |
| S9 U8 image override | FIXED | Added. | doc:240 |
| S9 U9 relay TLS | CODE-GAP-ONLY | Same as 21.7. | doc:280; `deferred-backlog.md:273` |

## 4. Findings

### F1. S6.4 (PARTLY): "Each Roym verifier"

Doc, "Minimum Federation Contract", line 1342: "Each Roym verifier checks that a record has the type and version it expects, and refuses any other."

Code: two places call `verify_json` and do not check the type or version. `crates/roym_directory/src/app/moderation_ops.rs:131` re-reads a moderation decision that this node stored when it issued it. It then reads the payload as a moderation decision. `crates/roym_profile/src/app/backup.rs:25` (`reverify_profiles`) only counts how many profile envelopes verify. The verifiers listed in row S6.4 check both fields. The profile code in `roym_profile/src/app/contacts.rs:97-99` and `backup.rs:100-106` checks them too.

Proposed text: "Each Roym verifier that accepts a record from another party checks that the record has the type and version it expects, and refuses any other. Code that re-reads a record this node signed itself does not check the type again."

### F2. S9.10 (PARTLY): "no test sets `role.tls`"

Row text: "no test sets `role.tls`". The doc says "No test covers relay TLS." (line 280). The doc is right. The row is not: `crates/core/src/config/tests.rs:20` sets `role.tls`. That test only checks that relative paths are resolved. It starts no relay. No test starts a relay with TLS.

Proposed row text: "No test starts a relay with `role.tls`. One config test sets it to check path resolution (`core/src/config/tests.rs:20`)." No doc change needed. The backlog row (`deferred-backlog.md:273`) says "No test sets `role.tls` for the relay", which is true.

### F3. S9.15 (PARTLY): "Neither path calls `relay_to_next_hop`"

Doc, "Browser Path", line 322: "The data channel path ends in the router of the substrate. Neither path calls `relay_to_next_hop`, the forwarding function above."

Code: the coordinator code of neither path calls it (grep in `crates/coordinator_webrtc/src`: no hit). The data channel is handled by `RouteHandler::handle_stream` (`crates/router/src/connection_router.rs:405-426`). That function calls `relay_to_next_hop` when the substrate does not host the service (`crates/router/src/route_handler/io.rs:317`). So a data channel that names a service the substrate does not host would be forwarded. The row cite `bootstrap.rs:36` is also wrong: the import is at `bootstrap.rs:39`.

Proposed text: "The tunnel dials the substrate with the Iroh stream and endpoint code of the router. The data channel path ends in the router of the substrate. The code of the WebRTC coordinator never calls `relay_to_next_hop`. A substrate forwards only a stream for a service it does not host."

### F4. S5 14.31 (STILL-WRONG): line 820

Doc, "Identity Resolution & Revocation", line 820: "An anchor stops verifying 24 hours after it was signed. A master must republish it before then. ... A master with no valid anchor cannot use its certificates on a stream: the router refuses them."

Code: `SignedMasterAnchor::verify` has the 24-hour check (`master_anchor.rs:152-165`). `resolve_master_anchor` calls it only for an anchor from the HTTP registry (`client.rs:309`). The DHT fallback parses the payload and checks only schema and the cached timestamp (`client.rs:333-355`). So an old anchor from the DHT is accepted when the registry gives no answer. The same rule is written correctly at lines 811, 1604 and 2506, so line 820 disagrees with them.

Proposed text: "An anchor from the HTTP registry stops verifying 24 hours after it was signed. A master must republish it before then. The DHT fallback does not check the age of an anchor today." Then keep the supervisor and registry sentences. Replace the last two sentences with: "If the router cannot get a valid anchor from the registry, or any anchor from the DHT, it refuses a stream that carries a certificate of that master."

### F5. S5 U6 and U7 (STILL-WRONG, outside the doc)

- `crates/substrate/config.sample.toml:253` and `config.dev.toml:185`: "Metrics (Prometheus) configuration". The endpoint answers a JSON snapshot (`crates/substrate/src/runtime/services.rs:400`). Proposed: "Metrics (JSON snapshot) configuration".
- `AGENTS.md:250`: "metrics (Prometheus `/metrics`)". Proposed: "metrics (JSON snapshot at `/metrics`)".

### F6. S8.1 and S9.2 (CITE-OFF): fix the rows only

- S8.1: say "an agreement for another node gives status `None` (`identity/src/substrate.rs:270-278`); an expired or bad-signature agreement gives `Unverified` (`:309-360`)". Both leave the node with no owner. The doc text is right.
- S9.2: add `iroh-0.97.0/src/lib.rs:101-104` ("keep flowing over the relay server as a fallback") for the words "or for as long as none exists".

## 5. Doc statements that look wrong or unproven and are in no row

1. Doc line 820 (see F4). Its second part, "A master with no valid anchor cannot use its certificates", is false when the DHT holds an old anchor.
2. Doc line 322 (see F3).
3. `crates/conversation/src/crypto.rs:5` still says "Real X3DH + Double Ratchet". The type is still called `X3dhDoubleRatchetCrypto` (`:233`). The doc is fixed, but a reader who opens the code sees the old name. Not a doc fault.
4. `crates/app_supervisor/src/keys.rs:3` still says `member-<instance>-<service>-<index>`. The format string at `:350` uses `#`. The doc is right.
5. `crates/sandbox_wasm/src/engine/lifecycle.rs:116-117` has `TODO(M5)` and "M3A". This breaks the "no planning ids in code" rule in `AGENTS.md`. Not a doc fault.
6. Doc line 1298 says "The node reports a limit of 3 ... The node does not enforce the limit". True for 3. The node does enforce its own permit count (default 4, `sandbox_wasm/src/engine/guest_http.rs:80-96`), and a full count gives the 503 that the next sentence describes. The two sentences are right but a reader may think the node has no limit. Optional: add "The node has its own limit of 4 guest HTTP calls in flight for each service."
7. `AGENTS.md` still calls the client gateway a "local HTTP proxy". The listener binds `0.0.0.0` (`crates/client_gateway/src/gateway.rs:185`). Not a doc fault.
