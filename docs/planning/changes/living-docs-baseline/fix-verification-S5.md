# Stage 2 verification, batch S5

Reader: the person who fixes `docs/system-architecture.md` after stage 2.

Scope: fix commits 13 (Resolved Architecture TBD Items), 14 (Security Architecture) and 15 (Observability Architecture). 63 rows: 11 + 32 + 20, counted from `fix-new-claims.md`. Row ids are `<commit>.<n>`.

Method: static reading only. For each row I opened the code myself, then read the matching doc text in `docs/system-architecture.md`. For vodozemac I read the crate source in the cargo registry.

## Summary

| Verdict | Commit 13 | Commit 14 | Commit 15 | Total |
| --- | --- | --- | --- | --- |
| CONFIRMED | 11 | 25 | 18 | 54 |
| CITE-OFF | 0 | 0 | 0 | 0 |
| PARTLY | 0 | 6 | 2 | 8 |
| WRONG | 0 | 1 | 0 | 1 |
| UNVERIFIABLE | 0 | 0 | 0 | 0 |
| Rows | 11 | 32 | 20 | 63 |

Rows that need a doc fix: 14.8, 14.12, 14.16, 14.24, 14.28 (WRONG), 14.29, 14.31, 15.4, 15.19.

## Per-row verdicts

| Row | Verdict | Note | Checked at |
| --- | --- | --- | --- |
| 13.1 | CONFIRMED | No `TBD` text in the requirements spec; intro no longer claims an index of such markers. | `docs/system-requirements-spec.md` (grep, 0 hits); `docs/system-architecture.md:1679` |
| 13.2 | CONFIRMED | `roymctl roym backup` seals archive under a recovery key; bundle manifest signed by the person. | `apps/roymctl/src/commands/roym/backup.rs:49-75,167-175`; `crates/roym_core/src/backup.rs:75-79,192` |
| 13.3 | CONFIRMED | One writer loop per service DB; write rules exist for all five record types. | `crates/data_db/src/sqlite/provider.rs:301-304`; `service_store.rs:100`; `ledger.rs:58-62,150`; `listing_ops.rs:231-245`; `dag_store.rs:308`; `traits.rs:112-118` |
| 13.4 | CONFIRMED | Re-searched `vouch`, `Reputation`, `bayes`, `tfidf`, `IOU`, `coin`, `escrow`: only unrelated words. | `crates/ucan/src/session_token.rs:4`; `crates/conversation/src/crypto.rs:366`; `search.ts:5` |
| 13.5 | CONFIRMED | `membership-credential` and `revocation` record types, signed by SynOrg; no `ssi` or `didkit` in any manifest. | `crates/roym_core/src/record.rs:21-23`; `credential_ops.rs:1-15`; `crates/roym_directory/src/app.rs:214-217` |
| 13.6 | CONFIRMED | `member.suspend` and `member.lift` verbs sign decisions; DHT `EndpointType` has only two variants. | `crates/roym_directory/src/app.rs:218-219`; `moderation_ops.rs:1-2,20,104`; `types.rs:46-52` |
| 13.7 | CONFIRMED | Defaults 3 per 24 h (contact), 20 per 24 h (publication); both are enforced in callers. | `crates/roym_core/src/safety.rs:25-28,58-61,126-146`; `contacts.rs:313`; `publication_ops.rs:328` |
| 13.8 | CONFIRMED | `payment-request` and `payment-acknowledgement` records exist; no UPI, escrow or gateway word in code. | `crates/roym_core/src/payment.rs:1-2`; `record.rs:26,29,41` |
| 13.9 | CONFIRMED | Client queries chosen sources, verifies each hit, round-robin merge; rendezvous hashing only picks a service member. | `client_query.rs:120-165`; `client_merge.rs:130-142`; `select.rs:13-23` |
| 13.10 | CONFIRMED | Hits sorted by `issued_at_secs` descending, tie by `listing_id`; no weights. | `crates/roym_directory/src/app/search_ops.rs:212-214` |
| 13.11 | CONFIRMED | pkarr packet published to DHT after registry; lookup tries registry first; no relay-mirror or governance code. | `crates/core/src/dht_registry/client.rs:116-165,202-203`; `base.rs:224` |
| 14.1 | CONFIRMED | Pooling allocator and store memory cap exist; both depend on config values, which default to set (256 MiB). | `init.rs:309-313,333,379`; `host_capabilities.rs:431-432`; `core/src/config/sandbox.rs:408` |
| 14.2 | CONFIRMED | Epoch deadline always set; fuel set only when a quota or default exists (default 10 billion). | `engine.rs:567-574`; `init.rs:310-311`; `sandbox.rs:407` |
| 14.3 | CONFIRMED | `WasiCtx::builder().build()` with no preopens, env or sockets; linker has WASI plus Syneroym interfaces only. | `host_capabilities.rs:429`; `init.rs:386-401` |
| 14.4 | CONFIRMED | Guest call goes via `service_proxy.invoke`; proxy gate passes non-native interfaces; callee admits (Roym `admit`). | `capabilities_proxy.rs:156`; `proxy/router.rs:195-198`; `roym_core/src/admit.rs:20,56` |
| 14.5 | CONFIRMED | Exactly these 8 names; same-service call allowed, other service gets `PermissionDenied`. | `local_registry.rs:42-51`; `proxy/router.rs:196-215` |
| 14.6 | CONFIRMED | `orchestrator` and `security` denied for any target. | `local_registry.rs:59`; `proxy/router.rs:185-192` |
| 14.7 | CONFIRMED | `services/<service_id>/state.db`, one store per service id. | `provider.rs:216-228,243-250,269` |
| 14.8 | PARTLY | Podman args confirmed. Diagram implies no substrate DB for a container, but every service gets a `data-layer` endpoint. | `sandbox_podman/src/engine.rs:238-262`; `control_plane/.../deploy.rs:79-91`; `deploy/commit.rs:153` |
| 14.9 | CONFIRMED | Three layers exist: stream identity, per-service admit, FDAE row policy; both router types exist. | `route_handler/io.rs:389-424`; `admit.rs:20,56`; `fdae/src/lib.rs:1-3`; `connection_router.rs:54`; `proxy.rs:129` |
| 14.10 | CONFIRMED | `kek inject` is manual; `attest` appears only in signed Roym record text; no TPM or quote code. | `roymctl/src/commands/security.rs:11-15`; `key_store.rs:79`; `roym_core/src/transaction.rs:1-3` |
| 14.11 | CONFIRMED | "attestation" in `roym_core` means a party's signed acceptance of terms. | `crates/roym_core/src/transaction.rs:1-12` |
| 14.12 | PARTLY | `vodozemac` and separate Ed25519 key confirmed. vodozemac implements Olm (3DH plus Double Ratchet), not X3DH. | `conversation/src/crypto.rs:1-13`; vodozemac-0.10.0 `src/olm/mod.rs` (docs: "triple Diffie-Hellman (3DH)") |
| 14.13 | CONFIRMED | Owner makes epoch keys at create and rekey; scheduled rekey only where owner is this service. | `conversation/src/group.rs:157-162,286-294,560-585`; `dag.rs:255-270` |
| 14.14 | CONFIRMED | `DeliveryPayload` signed with Ed25519 key, then ratchet session encrypts it. | `conversation/src/envelope.rs:1-12`; `crypto.rs:152,421` |
| 14.15 | CONFIRMED | No `libsignal` or `openmls` in `Cargo.lock`; only `vodozemac` at `Cargo.toml:162`. | `Cargo.toml:162`; `Cargo.lock` (grep) |
| 14.16 | PARTLY | `enc=ecdh-p256` set only on the WebSocket tunnel fallback. On the WebRTC data channel the page drops the query. | `peer-proxy.js:376-378,429,587`; `dispatch.rs:321-323`; `sdk/src/client.rs:661` |
| 14.17 | CONFIRMED | Node signs `server_pub || client_pub`; client key itself is not signed. | `route_handler/encryption.rs:312-328`; `io.rs:328-334` |
| 14.18 | CONFIRMED | Shared secret bytes copied into the AES-256-GCM key; no KDF. | `route_handler/encryption.rs:303-308` |
| 14.19 | CONFIRMED | P-256 point read for ECDH; same field fails Ed25519 length check; delegation present gives `Unauthorized`, absent gives no identity. | `encryption.rs:287-298`; `handshake.rs:25-39`; `io.rs:411-424` |
| 14.20 | CONFIRMED | Random 32-byte recovery key, HKDF-SHA256, AES-256-GCM; no replication code. | `identity/src/backup.rs:1-6,115-157`; `roymctl/.../backup.rs:167-175` |
| 14.21 | CONFIRMED | Data channel wrapper and `/ws` signaling exist; DTLS comes from the `webrtc` crate; SQLCipher and vault AES-GCM confirmed. | `net_webrtc.rs:15-35`; `signalling.rs:34`; `Cargo.toml:144`; `service_store.rs:173-190` |
| 14.22 | CONFIRMED | `substrate.key` default; load if present, else generate and save. | `core/src/config.rs:18`; `substrate/src/identity.rs:22-34` |
| 14.23 | CONFIRMED | `identities/<name>.key`; `delegate` and `publish-anchor` sign with it; `import` restores. | `roymctl/src/commands/identity.rs:160-161,242-262,265-273,375` |
| 14.24 | PARTLY | Default 24 h and non-extractable key confirmed. No command adds a key to `revoked_keys`; `publish-anchor` sends an empty list. | `session.rs:26-39,212-255`; `login.ts:1-10,128`; `identity.rs:272`; `handshake.rs:62-66` |
| 14.25 | CONFIRMED | KEK in a `Mutex<Option<..>>`; `rotate_kek` re-wraps all DEKs; wrong master gives `Crypto` error. | `key_store.rs:62-113,197-260,539-560`; `roymctl/.../security.rs:11-20` |
| 14.26 | CONFIRMED | HKDF-SHA256 with info `syneroym:kek:v1:<scope>`; not stored. | `crates/data_keystore/src/key_store.rs:37-44` |
| 14.27 | CONFIRMED | `dek_store` in `substrate.db`; SQLCipher key; `_vault`; blob subkeys by HKDF-SHA256. | `provider.rs:71,117,271-283`; `service_store.rs:173-190`; `data_blob/src/crypto.rs:1-6` |
| 14.28 | WRONG | Real name is `member-<app_instance_id>#<service_name>-<index>`; doc and row omit the `#`. | `crates/app_supervisor/src/keys.rs:324-351`; `roymctl/src/commands/member_identity.rs:45` |
| 14.29 | PARTLY | 32 random bytes, HKDF-SHA256, AES-GCM confirmed. `--recovery-key-out` writes the key to a file, so "never stored" is too strong. | `identity/src/backup.rs:115-157`; `roymctl/.../backup.rs:183-196`; `identity.rs:365-368` |
| 14.30 | CONFIRMED | Four scope constants; `TRANSPORT_SCOPES` holds `routing` and `service-instance` only. | `crates/identity/src/delegation.rs:11-30`; `handshake.rs:53` |
| 14.31 | PARTLY | 12 h default, publish command, reject-without-anchor confirmed. The 24 h age check runs on the registry path only, not the DHT fallback. | `master_anchor.rs:152-165`; `client.rs:283-362`; `roles.rs:76-81,155-162` |
| 14.32 | CONFIRMED | Router checks cert, DID match, revocation; no proof of key; only auth login checks a nonce signature. | `handshake.rs:25-70`; `sdk/src/client.rs:77-85`; `auth/src/service.rs:296-313` |
| 15.1 | CONFIRMED | Logs, `OK` health, JSON metrics, health polling with alerts exist; no narrator, status page or `/admin`. | `observability/src/engine.rs:30-95`; `services.rs:364-420`; `app_orchestration/src/alerts.rs:1-45` |
| 15.2 | CONFIRMED | JSON log format; JSON snapshot endpoint; no `opentelemetry` or Prometheus dependency. | `core/src/config/base.rs:132-146`; `services.rs:390-410`; `observability/Cargo.toml` |
| 15.3 | CONFIRMED | 492 event macros; no `#[instrument]`, `*_span!`, `.instrument(`, `.in_scope(`. | grep over `crates/`, `apps/`, `test-components/` |
| 15.4 | PARTLY | All listed families exist. Doc list omits `substrate.conversation.admission.stuck`. | `conversation/src/outbox.rs:336,347,421`; `router/src/route_handler/dispatch.rs`; `observability/src/metrics.rs:39-50` |
| 15.5 | CONFIRMED | Sampler interval is 1 s; sets the four gauges. | `observability/src/engine.rs:86`; `metrics.rs:27-50` |
| 15.6 | CONFIRMED | Counter and gauge are single values; histogram is a `Vec<f64>` with `push`; snapshot has 7 fields. | `observability/src/recorder.rs:15-27,62-70,103-141` |
| 15.7 | CONFIRMED | Defaults pretty and stdout; daily rolling file `syneroym.log`. | `base.rs:113-146`; `engine.rs:58-72` |
| 15.8 | CONFIRMED | `EndpointConfig` fields; own listeners; `OK`; JSON snapshot. | `roles.rs:455-467`; `services.rs:364-420` |
| 15.9 | CONFIRMED | `dev_mode_config()` used when no config path; ports 7966 and 7967. | `crates/substrate/src/main.rs:89-95,196-225` |
| 15.10 | CONFIRMED | `TracingConfig` and `OtlpConfig` have no reader outside `roles.rs`. | `roles.rs:474-515`; grep `.tracing`, `OtlpConfig`, `.sampling` |
| 15.11 | CONFIRMED | Flags `--watch`, `--no-record`, `--strict`, `--all`; fault bails non-zero. | `roymctl/src/commands/app.rs:194-231`; `app/health.rs:162-235` |
| 15.12 | CONFIRMED | Table `alerts` with partial unique index; `alerts.db` beside journal. | `app_orchestration/src/alerts.rs:224-249`; `app.rs:115-125` |
| 15.13 | CONFIRMED | Same store in `supervisor.db`; loop polls; `alerts` verb; publish of opened alerts. Broker adds a `svc/<id>/` prefix. | `app_supervisor/src/store/schema.rs:51`; `service.rs:650-690,748`; `resident_loop/pass.rs:180,334` |
| 15.14 | CONFIRMED | Enum has exactly these 16 variants in this order. | `crates/app_orchestration/src/alerts.rs:26-116` |
| 15.15 | CONFIRMED | Searched `health-narrator`, `HealthState`, `ring buffer`, `diagnostic bundle`, `observability enable`: no hit. | `roymctl/src/commands.rs`; `services.rs:364-420` |
| 15.16 | CONFIRMED | No `metrics.db` anywhere; recorder is in memory. | `observability/src/recorder.rs:15-27` |
| 15.17 | CONFIRMED | Two separate listeners; no `/admin` route in any router. | `services.rs:364-420`; `roles.rs:455-467` |
| 15.18 | CONFIRMED | No hardware or tier logic in the observability crate. | `observability/src/engine.rs:30-95` |
| 15.19 | PARTLY | No proptest, turmoil or madsim. But a concurrent booking test already checks the `slot-taken` write rule. | `dual_build_parity/booking.rs:320-341`; `Cargo.toml` (grep) |
| 15.20 | CONFIRMED | `substrate.db` and per-service `state.db` are the files named. | `crates/data_db/src/sqlite/provider.rs:71,269` |

## Findings

### 14.28 (WRONG): vault entry name

Doc, Security > Keys table, "Master keys of managed app instances" (`system-architecture.md:1592`): "The entries are named `member-<instance>-<service>-<index>` and `app-<app_instance_id>`."

Code: `member_master_name` builds `member-{app_instance_id}#{service_name}-{index}` (`crates/app_supervisor/src/keys.rs:350`). The `#` is on purpose: it marks the border between the two ids. `roymctl` builds the same name (`apps/roymctl/src/commands/member_identity.rs:45`). The module header comment (`keys.rs:3`) has the same error, and the row copied it.

Fix: "The entries are named `member-<app_instance_id>#<service_name>-<index>` and `app-<app_instance_id>`."

### 14.12 (PARTLY): "X3DH"

Doc, Security > Messaging (`:1550` diagram box M1 and `:1566`): "1-to-1 chat uses X3DH key agreement and a Double Ratchet. The `vodozemac` crate implements both."

Code: `vodozemac` implements the Olm protocol. Its own docs say the keys "participate in a triple Diffie-Hellman key exchange (3DH)" (`vodozemac-0.10.0/src/olm/mod.rs`, lines 15-35). 3DH is close to X3DH but is a different, simpler exchange. The repo comment `crates/conversation/src/crypto.rs:5` says "X3DH", but the library does not claim that name.

Fix, `:1566`: "1-to-1 chat uses the Olm protocol: a Double Ratchet with a triple Diffie-Hellman (3DH) key exchange. The `vodozemac` crate implements it. A service has a `vodozemac` account for the ratchet and a separate Ed25519 key for signing." Fix, `:1550`: "M1[1-to-1 chat: Olm (3DH + Double Ratchet) via vodozemac]".

### 14.16 (PARTLY): who sets `enc=ecdh-p256`

Doc, Security > Optional end-to-end stream layer (`:1573`): "The browser bootstrap page (`peer-proxy.js`) sets it."

Code: on a WebRTC data channel the page removes the query part of the preamble ("WebRTC data channels are already DTLS encrypted, no ECDH is needed"): `crates/coordinator_webrtc/templates/peer-proxy.js:376-378`. It sets `enc=ecdh-p256` only on the WebSocket tunnel fallback (`:429`, `:587`, `:944`).

Fix: "The browser bootstrap page (`peer-proxy.js`) sets it when it falls back to the WebSocket tunnel. On a WebRTC data channel it does not set it, because DTLS already protects that channel. The Rust client (`SyneroymClient`) does not set it. Nothing else in the SDK or the gateway sets it."

### 14.24 (PARTLY): revoking a temporary key

Doc, Keys table, "Temporary key and delegation certificate", last cell (`:1588`): "The master revokes a stolen key by listing its DID in its master anchor."

Code: the router does reject a key listed in `revoked_keys` (`handshake.rs:62-66`). But no `roymctl` command lets a person add a key to that list. `roymctl identity publish-anchor` publishes an empty list: `publish_master_anchor(&master_id, vec![], ...)` (`apps/roymctl/src/commands/identity.rs:272`). Only `RegistryClient::revoke_instance_key` adds a key, and the App Supervisor calls it for instance keys it manages (`crates/app_supervisor/src/anchors.rs:74`).

Fix: "Make a new pair with the master key. The router rejects a key that the master lists in `revoked_keys` of its anchor. Today `roymctl` has no command that adds a person's key to that list. `roymctl identity publish-anchor` publishes an empty list. The App Supervisor can revoke the instance keys it manages. A stolen session key stops working when its certificate expires (24 hours by default)."

### 14.29 (PARTLY): "Never stored"

Doc, Keys table, "Recovery key" (`:1593`): "Shown to the person once. Never stored."

Code: `roymctl roym backup create` and `roymctl identity export` accept `--recovery-key-out <path>`, which writes the key to a file (`apps/roymctl/src/commands/roym/backup.rs:183-186`; `identity.rs:365-368`). Syneroym does not keep a copy by itself.

Fix: "Shown to the person once. Syneroym never keeps a copy. The option `--recovery-key-out` writes it to a file that the person chooses."

### 14.31 (PARTLY): 24-hour age of an anchor

Doc, Security > Keys, "The master anchor is a duty" (`:1597`): "An anchor stops verifying 24 hours after its signing time."

Code: `SignedMasterAnchor::verify` has the 24-hour check (`master_anchor.rs:152-165`), and `resolve_master_anchor` calls it only for an anchor from the HTTP registry (`client.rs:309`). The DHT fallback (`client.rs:330-355`) parses the payload and does not check its age. The router uses this resolver (`handshake.rs:55-60`).

Fix: "A registry refuses an anchor that is older than 24 hours after its signing time. The DHT fallback does not check the age of an anchor today."

### 14.8 (PARTLY): container and the substrate database

Row text: "The diagram shows no substrate database for a container (the run arguments carry no database path)." Doc: diagram node `APP2` (`:1633-1634`) has no database, while the WASM boxes have one.

Code: every deployed service, whatever its type, gets the native `data-layer`, `vault`, `app-config` and `blob-store` endpoints ("regardless of type": `crates/control_plane/src/service/orchestration/deploy.rs:79-91`, `deploy/commit.rs:153`). The substrate opens `state.db` for the service id when such a call or a secret write arrives (`synsvc_native/data.rs:171`). The `podman run` arguments really carry no DB path (`engine.rs:238-262`), so the Podman text on `:1673` is right.

Fix, node label in diagram: `P1[OCI Container run by the host's Podman. No database of its own is mounted. The substrate can still keep a `state.db` for its service id.]`. Or add after `:1672`: "A container service still has the native data endpoints. The substrate keeps its `state.db` outside the container."

### 15.4 (PARTLY): metric list is not complete

Doc, Observability > Instrumentation Layer > Metrics (`:1407`): "The substrate emits these metric families:" followed by the list.

Code: every listed family exists. One more counter exists: `substrate.conversation.admission.stuck`, with label `service`, raised every 20th failed re-ask of an undecided message (`crates/conversation/src/outbox.rs:413-424`).

Fix: add to the list after the `dead_lettered` line: "`substrate.conversation.admission.stuck` for a message that is still undecided after many re-asks." Or change the lead-in to "These are the main metric families:".

### 15.19 (PARTLY): "No such scenario exists today"

Doc, Simulation Testing and Replay Validation (`:1531`): "Each write rule ... gets a scenario that checks the outcome. No such scenario exists today."

Code: a test already checks one write rule under a race. Two conversations sync at once, one booking ends `scheduled`, the other `conflict` with `slot-taken` (`crates/roym_web/tests/dual_build_parity/booking.rs:320-341`). What does not exist is a simulation scenario with a fake network.

Fix: "No such simulation scenario exists today. Ordinary tests check some write rules, for example the booking slot conflict in `dual_build_parity/booking.rs`."

## Doc text not covered by any row

1. `system-architecture.md:1544` (diagram T1) "Node-to-node: QUIC TLS 1.3 via Iroh". No row. It rests on the `iroh` crate, which I did not read. Add a row, or cite the Iroh docs.
2. `:1588` "Lets a device or a session act under the master's identity". `roymctl session delegate` mints a `session-auth` certificate (`apps/roymctl/src/commands/session.rs:237`). The router refuses that scope on a stream (`TRANSPORT_SCOPES`, `delegation.rs:30`). A `routing` certificate comes from `roymctl identity delegate` (`identity.rs:242-262`). The cell can lead a reader to use the wrong certificate on a stream. Say which command makes which scope.
3. `:1591` DEK row: "Encrypts the `_vault` rows of the service." With `storage.encryption` off, `provider.rs:271-278` uses an all-zero key and does the same AES-GCM, so secrets are not protected. The doc does not say this. Add one sentence.
4. `:1556` diagram R1 "SQLCipher under a per-service key". Same case: with encryption off, no SQLCipher key pragma is set (`provider.rs:281-283`). Add "when `storage.encryption` is on".
5. `:1424` "to the topic `<alert_topic>/<app_instance_id>` ... of its messaging broker". The broker adds the prefix `svc/<supervisor service id>/` (`mqtt_broker/src/lib.rs:80-82`; `service.rs:660-663`). A subscriber must know that. Add the prefix or a pointer.
6. `crates/substrate/config.sample.toml:253` and `config.dev.toml:185` label the metrics block "Metrics (Prometheus)". The doc says no Prometheus export exists (`:1400`). The sample comments are wrong. Not a doc text issue, but a reader will see both.
7. `AGENTS.md` (architecture list, `syneroym-observability`) says "metrics (Prometheus `/metrics`)". It contradicts `:1400` and `:1439`. Not in scope for the architecture doc, but it should be fixed with it.

## Cost notes

- Effort: 63 rows in one session. A rough guess is 55 to 65 rows per hour; I did not time it.
- Easy rows (about 40): config values, enum lists, negative greps with a short, clear term (13.x, 15.x).
- Slow rows:
  - 14.12: the code comment says "X3DH", so the claim looked right. I had to read the vodozemac source in the cargo registry to find that it is 3DH.
  - 14.16 and 14.19: the claim touches three files (`peer-proxy.js`, `encryption.rs`, `handshake.rs`). The `peer-proxy.js` data-channel branch changed the verdict.
  - 14.24 and 14.31: the row cites the check, not the writer. I had to trace who can put a key in `revoked_keys` and which path calls `verify()`.
  - 14.8: needed the deploy path to see that containers get native endpoints.
  - 14.28: the row copied the module header comment. Only the `format!` call showed the real name.
- Pattern: when the evidence column cites a module header comment (14.28, 14.12), check the code that builds the value. Comments are older than the code.
