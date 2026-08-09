# M05C Logical Service Discovery Overlay — Status

**Milestone:** [task.md](task.md) · **Design of record:**
[ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md)
(Accepted 2026-08-02) · **Plan:**
[implementation-plan.md](implementation-plan.md)

**Overall:** 🚧 **In progress. S1 and S2 complete 2026-08-08.** Promoted from
the *Committed Work: Logical Service Discovery Overlay* section of
[meta-implementation-plan.md](../../meta-implementation-plan.md) into a
milestone directory, so the largest committed-but-unplanned work in the tree
carries the same discipline as everything else. **S3's gate is clear** — S2
landed 2026-08-08.

**Plan layout.** [implementation-plan.md](implementation-plan.md) is milestone
level. Each slice's own findings, decisions, phases, and tests live in that
slice's file — [slice-s1-implementation-plan.md](slice-s1-implementation-plan.md)
is the first; S2–S4 get theirs when picked up. The reasoning is in
[M05B's plan](../M05B-async-primitives/implementation-plan.md) §2.

## Slice status

| Slice | Scope | Status | Gate |
|---|---|---|---|
| S0 | App-instance master DID | **Complete (2026-08-04)** as [M05A slice A7](../M05A-app-supervisor/slice-a7-implementation-plan.md) | — |
| S1 | Tier 1: the app-DID registry record; `ShardingStrategy` manifest surface | **✅ Complete (2026-08-08)** — [plan](slice-s1-implementation-plan.md); evidence below | S0 **cleared** |
| S2 | Tier 2: signed topology document, `resolve` RPC, client verify/cache | **✅ Complete (2026-08-08)** — [plan](slice-s2-implementation-plan.md); evidence below | S1 **cleared** |
| S3 | Gateway hostname scheme + routing-key header; coordinator relay | 📋 **Planned (2026-08-09)** — [plan](slice-s3-implementation-plan.md), `§0` written | S2 **cleared** |
| S4 | Cross-app `Bind` | 📋 Planned (sketch; owes its own `§0`) | S2 **and a real consumer** (D-C-7) |
| S5 | Shard rebalancing, epoch enforcement | Out of this milestone | **M7** `[PLT-RED]` |

## Findings from the planning pass

**Two are contradictions inside ADR-0022 itself**, found by reading it against
the tree rather than on its own terms:

- **The Tier-1 record cannot be `EndpointInfo` "unchanged" *and* carry a
  generation** ([plan](implementation-plan.md) §0.1). The struct has eight
  fields and no generation, and it is inside the signed payload. Resolved in
  favour of the field. Separately, the meta plan's slice table calls S1
  "generation-fenced" — ADR-0022 §2 is careful and says the generation is
  compared **by a reader**, not enforced at admission. Corrected.
- **`LogicalResolver` is keyed by two human names**
  ([plan](implementation-plan.md) §0.4). S2 registers *foreign* apps' topology
  documents into that map, so two unrelated apps both called `chat` collide and
  one is silently re-pointed at the other's members. Latent today, reachable
  from the network at S2. S2's own `§0` must settle it.

**One is a failure mode that inverts** ([plan](implementation-plan.md) §0.2).
The supervisor's vault is locked after every restart until a human runs
`inject-kek`. For A7 that cost one loud refusal at `adopt`. Here the supervisor
keeps running and simply stops refreshing the Tier-1 record, which sits in the
registry until `not_after` — 30 days — and then the app stops being
discoverable to every outside caller at once, with no event when the cause
occurred. Silent decay over a month is worse than a refusal at a keystroke, so
D-C-2 makes the locked state standing and visible with a deadline.

**And one is the same shape, from `pause`** ([S1 plan](slice-s1-implementation-plan.md)
§0.2). A paused instance is excluded from the loop's work list entirely, so
`pause` — documented as stopping reconciliation "and nothing else" — would also
expire the app's public identity in 30 days. S1 keeps the skip and makes `pause`
warn with the date, rather than reversing two A5c tests that assert a paused
instance gets zero processing.

## Cross-stream sequencing with M05B

The meta plan says M05C and M05B have no dependency in either direction. True of
the designs, **false of the files** — both change `app_supervisor/src/service.rs`
(8,852 lines), `store.rs`, `supervisor.wit`, and `models.rs`
([plan](implementation-plan.md) §2 has the table).

**Decided 2026-08-04: M05B runs first, then M05C.** A sequence removes the
collision rather than managing it, and costs nothing — the meta plan already
puts M05B ahead on merit, since it unblocks A6 and M6 where this overlay
unblocks nothing yet. The concrete benefit for S1 is that M05B slice B1 phase 1
is a pure refactor of `service.rs`, so by the time S1 opens that file it has one
`SubstrateActor` construction point instead of six.

If the sequence is ever relaxed, the convention is that `supervisor.wit` and
`SupervisorStore` schema changes are announced in the other stream's status doc
— those are where a silent conflict merges cleanly and breaks later.

**Honored**: M05B's slices B1–B4 merged before S1 started (`main` at
`2b44dc3` when S1's branch was cut), so S1 opened `service.rs` with the single
`SubstrateActor` construction point B1 phase 1 promised, not six.

## Evidence

### S1 — Verification evidence (2026-08-08)

**What shipped**, by phase (see
[slice-s1-implementation-plan.md](slice-s1-implementation-plan.md) §2 for the
phase plan):

- **Phase 1 (the wire field and the vocabulary):**
  `EndpointInfo.generation: u64` (`#[serde(default)]`,
  [dht_registry.rs](../../../../crates/core/src/dht_registry.rs)) — every
  existing construction site across the workspace (~25, production and test)
  now passes `generation: 0`, found and fixed by letting the compiler enumerate
  them (`missing field` errors) rather than by grep. `ShardingStrategy` gained
  a manifest surface, `ServiceSpec.sharding_strategy: Option<ShardingStrategy>`
  ([models.rs](../../../../crates/app_orchestration/src/models.rs)) — reusing
  the resolver's own three-variant enum rather than defining a second,
  redundant one (the slice plan's "name one strategy" assumed no type existed
  yet; `resolver.rs` already had one, with a real consumer in
  `LogicalResolver::select_member`). `RangeSharding` is refused at
  `SynAppManifest::validate` (it names concrete `ServiceId`s a manifest is
  authored before any exist), and `sharding_strategy` alongside
  `replicas <= 1` is refused as not a selection.
- **Phase 2 (the store side):** `app_tier1_refresh(app_did, last_refreshed_at)`
  in [store.rs](../../../../crates/app_supervisor/src/store.rs), the same
  shape `master_anchor_refresh` uses, with `last_tier1_refresh`/
  `record_tier1_refresh`.
- **Phase 3 (the publisher and the refresh):** a new
  [tier1.rs](../../../../crates/app_supervisor/src/tier1.rs) — `Tier1Writer`
  trait and `RegistryTier1Writer` (a sibling of `anchors::AnchorWriter`, per
  D-S1-1: the app master gets no `MasterAnchorPayload`, ADR-0022 §3, so this
  only ever publishes an `EndpointInfo`), and `sign_tier1_record`, which reads
  the app master read-only (`keys::existing_app_master`, new) and fails before
  any writer is reached on a locked vault or a missing master — never minting
  (D-S1-6). `SupervisorService::refresh_due_app_tier1_record` is called from
  `apply_write_phase` alongside `refresh_due_master_anchors`, gated into the
  write phase by `self.tier1_writer.is_some()`, the same shape
  `anchor_writer` uses — so it runs on the ordinary per-instance pass tick,
  with no timer of its own. No registry configured means no writer
  (`RegistryTier1Writer::from_registry_url` returns `None`), and
  `runtime.rs` warns once at supervisor init if `substrate.registry_url` is
  unset (D-S1-8).
- **Phase 4 (operator visibility, and the e2e):** `pause`'s response carries
  `app_record_expires_at` (derived from the last successful refresh plus
  `DEFAULT_ENDPOINT_NOT_AFTER_SECS`) with a `tracing::warn!` naming the date
  (D-S1-4); `roymctl supervisor pause` prints it. `supervisor.wit`'s
  `instance-status` gained `app-record-expires-at: option<u64>`, read from the
  stored refresh fact, never the registry. Two new e2e tests in
  [tier1_endpoint_record_e2e.rs](../../../../crates/substrate/tests/tier1_endpoint_record_e2e.rs)
  (ports 15_000–15_002/15_100–15_102 and 15_200–15_202/15_300–15_302) against
  two real substrates and a real registry: `an_app_did_resolves_to_its_
  supervising_node_through_the_registry` (submit → adopt → poll the resident
  loop's own tick for the published record → verify it names the supervisor
  and self-verifies → `status` reports the expiry) and
  `a_forged_tier1_record_is_rejected_at_the_registry` (401, the same shape
  `master_endpoint_record_e2e.rs`'s own forged-record case uses).

**A design deviation from the slice plan, decided during implementation**:
D-S1-7 recommended defining a single-variant `ShardingStrategy` under the
premise that no such type existed. It does — `resolver.rs`'s `ShardingStrategy`
(`HashSharding`/`EntityTagSharding`/`RangeSharding`) already has a real
consumer in `LogicalResolver::select_member`. Reusing it and refusing
`RangeSharding` at validation gets the same ship-before-enforce property
(nothing reads a declared strategy until S5) without a second, name-colliding
type. Recorded here since it changes what the plan's own §1 decided.

**Test coverage**: all 18 numbered tests in the slice plan's §3 are present
(7 in [tier1.rs](../../../../crates/app_supervisor/src/tier1.rs), 4 in
[keys.rs](../../../../crates/app_supervisor/src/keys.rs)/
[dht_registry.rs](../../../../crates/core/src/dht_registry.rs), 5 in
[models.rs](../../../../crates/app_orchestration/src/models.rs)/
[config.rs](../../../../crates/core/src/config.rs), 6 in
[service.rs](../../../../crates/app_supervisor/src/service.rs), 2 e2e), plus a
handful of additional tests beyond the plan's minimum (the negative
`sharding_strategy` validation cases, the absent-registry-writer no-op case,
and the paired "reports none" cases for `status`/`pause` when nothing has been
published yet).

**Verification**:
- `cargo +nightly fmt --all`: clean.
- `cargo clippy --workspace --all-targets --all-features`: clean.
- `cargo test --workspace --all-features` (sandboxed): every unit test passes,
  including every new one this slice adds. A fixed, pre-existing set needs
  real socket binds the sandboxed environment refuses
  (`Operation not permitted`) — `syneroym-community-registry` (6),
  `syneroym-control-plane` (7), `syneroym-mqtt-broker` (1),
  `syneroym-coordinator-iroh`'s three integration-test binaries,
  `syneroym-sdk`'s `connect_timeout`, and every `crates/substrate/tests/
  *_e2e.rs` integration binary. Each of the non-substrate ones was
  independently re-run with the sandbox disabled and passes in full. One
  `syneroym-app-supervisor` test
  (`keys::tests::get_or_mint_warns_with_the_wording_matching_its_kind`, a
  pre-existing test this slice does not touch) failed once under full
  parallel load and passed cleanly on three immediate reruns — consistent
  with its own tracing-capture helper being thread-local against a
  background writer thread, not a regression.
- `crates/substrate`'s ~20 `tests/*_e2e.rs` integration binaries (sandbox
  disabled, each boots one or two real substrate nodes): this slice's own new
  file, [tier1_endpoint_record_e2e.rs](../../../../crates/substrate/tests/tier1_endpoint_record_e2e.rs),
  passes in full (both tests). An exhaustive run of every file in the crate
  was started and stopped after ~40 minutes with no sign of a failure caused
  by this change — sequential real-node e2e at that count is simply slow, not
  stuck. In its place, three representative files were run individually to
  completion, all passing: `master_endpoint_record_e2e` (touches
  `EndpointInfo` directly, the type this slice adds `generation` to),
  `app_instance_identity_e2e` (the file this slice's own e2e helpers are
  copied from), and `supervisor_loop_e2e` (exercises the resident loop and
  the `apply_write_phase` gate this slice adds a branch to, 211s). The other
  ~13 touched files in this crate received only a single mechanical,
  compiler-verified field addition each (`generation: 0,` on an existing
  `EndpointInfo` literal, or `sharding_strategy: None,` on an existing
  `ServiceSpec` literal) with no logic change, so the residual risk an
  exhaustive re-run would catch is low.
- `mise run test:e2e` (Playwright WebRTC suite): **4 passed**. Unaffected by
  this slice, as expected (no client-gateway or WebRTC surface touched).

### Post-review pass (2026-08-08), 13 findings, all incorporated

An independent review against `task.md`/`slice-s1-implementation-plan.md`
found three defects that would have stopped the slice delivering the
milestone goal in production, all confirmed against the shipped code before
fixing:

- **The record was absent from the registry for most of every refresh
  cycle.** `EndpointInfo.ttl: None` inherited the community registry's
  2-hour sweep default, while the refresh reused `master_anchor_refresh_
  interval_secs` (12h default) -- the record was evicted roughly two hours
  after each publish and stayed unresolvable until the next one. Master
  anchors are safe at that cadence only because the sweep never touches
  `master_anchors`, a fact this slice's own reasoning missed by analogy.
  Fixed: `sign_tier1_record` now takes the refresh interval and sets an
  explicit `ttl` at 3x it, pinned by
  `tier1::tests::the_records_ttl_leaves_multiple_refresh_cycles_of_margin`.
- **The refresh fact was keyed by `app_instance_id`, not the app DID**
  D-S1-5 specified. Self-consistent until a handover: a new app master DID
  (`import-master` under a different key, then `adopt`) would inherit the
  old DID's "recently refreshed" stamp under the shared instance-id key,
  leaving the new DID unpublished for up to a full interval with no
  indication. Fixed at all call sites (`refresh_due_app_tier1_record`,
  `handle_status`, `handle_pause`) to key on `state.app_master_did`.
- **The record was signed under whatever the vault held, never checked
  against `state.app_master_did`.** Plan §0.6 requires reading the DID from
  the row; the shipped code only checked it was non-empty, then re-derived
  the actual signing key independently from the vault. A mismatch (an
  `import-master` not yet followed by `adopt`, the exact hazard A7 §0.5 and
  `task.md`'s S1 note name) would have published under a DID nobody looks
  up. Fixed: `sign_tier1_record` now takes the expected DID and refuses
  (`Tier1SignError::IdentityMismatch`) rather than matching on error text,
  and the caller raises a new `AlertKind::AppIdentityMismatch`.

Two "should fix" findings and four test-coverage gaps were also
incorporated: a locked vault now raises (and clears) `AlertKind::VaultLocked`
from this path too, not only from certificate renewal, since an instance
with no member near cert expiry got no signal at all; a backlog row records
the hardcoded `is_private: false`; the interval gate is now tested across a
real success (not only failures, which cannot regress it); the published
record's `generation` is pinned in the pass-tick test; a service-level
locked-vault-with-a-configured-writer test asserts the writer is never
reached; and the no-registry test is renamed to what it actually asserts.
Four minor items: one `RegistryClient` (and one DHT socket) is now shared
between the anchor and Tier-1 writers rather than each building its own; a
backwards clock step is now treated as due immediately in both this
refresh and `refresh_due_master_anchors`; `retire`'s doc now states the
record decays rather than being withdrawn; and the no-registry warning
reads as node-wide rather than naming one app. New tests:
`a_vault_key_that_does_not_match_the_rows_recorded_did_is_refused`,
`the_records_ttl_leaves_multiple_refresh_cycles_of_margin`,
`the_interval_gate_holds_after_a_successful_publish`,
`a_locked_vault_never_reaches_a_configured_tier1_writer`,
`a_mismatched_vault_key_raises_an_alert_and_never_publishes`. Full
`cargo test --workspace` and the two `tier1_endpoint_record_e2e.rs` e2e
tests re-verified clean after every fix.

### Post-review pass, round 2 (2026-08-08), 4 residuals

A follow-up read of the same review found four items round 1 left open --
the fourth was flagged as a regression, not a residual, and undercounted
in this doc's first pass:

- **S1-1's DHT half was still open.** Fixing the record's `EndpointInfo.ttl`
  only closed the community registry's own in-memory sweep; the DHT copy's
  freshness is governed by `PKARR_TTL` (1h), baked into every signed
  record's pkarr packet by `EndpointInfo::sign` and shared system-wide --
  `sign_tier1_record`'s `ttl_secs` parameter never reaches it. On an
  `enable_bep0044_dht` deployment the record still lapses from the DHT
  between 12h-cadence refreshes. Not new to this record -- master anchors
  already have the identical property at their own slower-than-heartbeat
  cadence, accepted on D-A1-2's reasoning (HTTP registry is the resolution
  path; DHT is best-effort backup, checked second). Documented rather than
  fixed, since `PKARR_TTL` is shared by every signed record in the system
  and lowering it (or moving this record to DHT-heartbeat cadence) is a
  bigger decision than this one record: a doc note on `sign_tier1_record`
  and a deferred-backlog row.
- **`generation` was missing from `stable_registry_certificate_for_hash`'s
  dedup tuple** (`crates/control_plane/src/service/orchestration.rs`),
  which lists every other `EndpointInfo` field explicitly. Latent only --
  every member record today passes `0` -- but the day one publisher's
  generation is genuinely nonzero, two records differing only in
  generation would hash identically and silently defeat redeploy dedup.
  Added, with a new unit test,
  `stable_registry_certificate_for_hash_distinguishes_by_generation`,
  pinning that two records differing only in `generation` hash
  differently.
- **`pause_warns_with_the_records_expiry_date` still asserted the payload,
  not the log line its name promised**, the same gap round 1's rename
  fixed on the sibling no-registry test but missed on this one. Renamed to
  `pause_reports_the_records_expiry_date`, with a doc note explaining the
  log line is real (`handle_pause`) but outside what a dispatch-level RPC
  test can capture without `keys.rs`'s own `run_capturing_logs`-shaped
  machinery.
- **`refresh_due_app_tier1_record` regressed the vault-locked check from
  A7's shape to A5d's, and round 1's own `VaultLocked` fix is what
  introduced it.** The pre-check `!self.vault.kek_is_loaded()` reads the
  `KeyStore`, not whether the storage provider's own encryption is even
  on -- on a node with `storage.encryption = false` every vault read
  succeeds, but `kek_is_loaded()` still answers `false`, since no KEK is
  ever injected on such a node. That pre-check would have skipped this
  instance's Tier-1 publish forever, silently, on exactly that node, and
  raised a `VaultLocked` alert that was never true -- total, silent loss
  of the feature this slice exists for, on a real and unremarkable
  deployment shape. No test caught it: every fixture and the e2e both
  inject a KEK even when unencrypted, by design (`Fixture`'s own doc
  says so), so nothing exercised the unencrypted-and-no-KEK state.
  **Fixed by deleting the pre-check** and instead matching
  `Tier1SignError::Vault(keys::VaultError::Locked)` off the real
  `sign_tier1_record` attempt (A7's D-A7-1 shape) -- the same alert,
  the same clear-on-success path, and it additionally now catches the
  vault locked *between* an earlier check and the call, which the
  pre-check could not. `Fixture` gained `skip_kek_injection` (default
  `false`, so no existing test's behavior changes) to build exactly the
  unencrypted-no-KEK state, and a new test,
  `an_unencrypted_vault_with_no_kek_still_reaches_the_tier1_writer`,
  confirmed to fail against the pre-check version and pass against the
  fix before landing.

Re-verified: `cargo +nightly fmt --all` and
`cargo clippy --workspace --all-targets --all-features` clean;
`syneroym-app-supervisor` 245/245 and `syneroym-control-plane` 176/176
(both sandbox-disabled for the latter's real HTTP-probe tests, a
pre-existing, unrelated sandbox constraint) passing; both
`tier1_endpoint_record_e2e.rs` e2e tests re-verified against real
substrates and a real registry.

### S2 — Verification evidence (2026-08-08)

**What shipped**, by phase (see
[slice-s2-implementation-plan.md](slice-s2-implementation-plan.md) §2 for the
phase plan):

- **Phase 1 (the resolver key and expiry):** `AppScope` (`Local(AppInstanceId)`
  \| `Foreign(AppDid)`) and `TopologyKey` in
  [resolver.rs](../../../../crates/app_orchestration/src/resolver.rs),
  replacing every `(AppInstanceId, LogicalServiceName)` pair `AppRegistry`/
  `LogicalResolver` took — settling milestone plan §0.4/D-C-6 by making the
  two namespaces disjoint by type. `TopologyEntry` and `ResolvedTopology`
  gained `not_after: Option<u64>`; `LogicalResolver::get_topology` refuses an
  expired entry on both the cache-hit and the registry-read path, evicting a
  cache entry that ages past its own `not_after` without a registry
  re-read. Every production call site was updated (`runtime.rs`'s
  `replay_persisted_bindings`, `control_plane/service/orchestration.rs`'s
  `install_app_context`/`write_bindings`/`prepare_binding`,
  `sandbox_wasm/host_capabilities.rs`'s `CallTarget::Dependency`,
  `router/proxy_outbox.rs` and `router/saga.rs`'s `QueuedTarget::Dependency`
  resolution — the latter two are M05B B2/B4 code the slice plan's own file
  table flagged as missing from it), plus every test/bench call site,
  compiler-enumerated.
- **Phase 2 (the document type and the plan field):** new
  [topology_document.rs](../../../../crates/app_orchestration/src/topology_document.rs)
  — `TopologyDocument`/`SignedTopologyDocument` (`sign`/`verify`, RFC-8785
  canonical JSON + z-base-32 Ed25519 via `Identity::sign_json`/
  `substrate::verify_json_signature`, the same shape `DelegationCertificate`
  uses), `to_topology_entry`, `foreign_key`, `register_verified`, the
  `TopologyFetcher` trait, and `topology_fingerprint` (a BLAKE3 hash of
  `(mode, ordered members, sharding_strategy)`). `syneroym-identity` added as
  a dependency of `syneroym-app-orchestration` (a leaf, no cycle).
  `PlannedService` gained `sharding_strategy`, cloned from `ServiceSpec` by
  `compiler.rs` in `schedule`'s exact shape (D-S2-9) — so a Tier-2 document
  built from a stored plan can name a strategy at all, which S1's manifest-only
  surface could not do.
- **Phase 3 (the supervisor side):** a new `topology_epochs` table in
  [store.rs](../../../../crates/app_supervisor/src/store.rs) — `(app_instance_id,
  service_name) -> (epoch, fingerprint)`, rows never deleted — with two entry
  points and the safety property between them being the whole point (D-S2-4):
  `record_topology_fingerprint` (advancing, one `INSERT … ON CONFLICT DO
  UPDATE … WHERE fingerprint != excluded.fingerprint`, called only from
  `handle_submit` under the instance lock) and `initialise_topology_epoch`
  (insert-only, `ON CONFLICT DO NOTHING` then read back, called only from
  `handle_resolve`, which holds no lock — backfills a pre-S2 instance at
  epoch 1 and can never advance one). New
  [topology.rs](../../../../crates/app_supervisor/src/topology.rs) —
  `service_topology`, a pure function grouping a stored plan's members into
  one logical service's `(mode, members, sharding_strategy)` in member-index
  order, refusing an internally inconsistent plan. `SupervisorService` gained
  `topology_document_not_after_secs`/`topology_document_cache_ttl_secs`
  (new `SupervisorRole` config fields, defaults 3600/300) and
  `signed_documents: DashMap<(String, String), CachedDocument>` — one
  signature per `(service, epoch)`, re-signed once less than half its
  validity remains (D-S2-6). `refuse_unshardable_plan`, a third sibling of
  `refuse_replicas_above_cap`/`refuse_unrunnable_schedules`, re-checks S1's
  two `sharding_strategy` manifest rules against an already-compiled plan
  from both `handle_submit` and `handle_force_reconcile` (D-S2-15).
  `handle_submit` now computes every named service's topology fingerprint
  before `store.submit`'s durable write (so a `service_topology` defect
  refuses the submit with nothing stored) and records it after (evicting a
  service's cached document only when its epoch actually moved). New
  `handle_resolve`, dispatched on `"resolve"`: looks up the app instance by
  its master DID (`instance_by_app_master_did`, D-S2-5), authorizes on
  `supervisor/resolve` against `synapp:<app-did>` (new
  `Ability::SUPERVISOR_RESOLVE` in `syneroym-ucan`, D-S2-7) — an unknown app
  and an unauthorized caller refused identically, before any document is
  built (D-S2-8) — answers for a paused instance and refuses for a retired
  one (D-S2-14), retries once against a fresh read if a `submit` lands
  mid-call, and raises `AlertKind::VaultLocked`/`AppIdentityMismatch` the same
  way `refresh_due_app_tier1_record` does on a locked vault or a DID
  mismatch (D-S2-12). `supervisor.wit` gained `topology-document`/
  `signed-topology-document` records and the `resolve` function; the
  handler's response is the domain `SignedTopologyDocument` serialized
  directly (D-S2-6's "what a caller deserializes is what was signed"), not a
  separately-converted wit-bindgen type — pinned by a `wit_parser`-driven
  test that the two describe one wire format.
- **Phase 4 (the client, the CLI, the e2e):** new
  [sdk/src/topology.rs](../../../../crates/sdk/src/topology.rs) —
  `RegistryTopologyFetcher` (Tier 1 via `RegistryClient::lookup` + `verify()`,
  then Tier 3 to the supervisor via `SyneroymClient`, one connection per
  fetch) and `fetch_and_register`, storing `identity_bytes: Option<[u8; 32]>`
  rather than an `Identity` (a plan deviation, see below). New
  `roymctl app resolve <app-did> <service-name>`, which builds a
  `RegistryTopologyFetcher` from `--as`/`--ucan`, fetches, verifies, and
  prints the mode/epoch/members. New
  [topology_document_e2e.rs](../../../../crates/substrate/tests/topology_document_e2e.rs)
  (port block 15_400-16_502), covering the reference scenario's steps 3
  through 8 against two real substrates and a real registry.

**A design deviation from the slice plan, decided during implementation**:
the plan's `RegistryTopologyFetcher` sketch held `identity: Option<Identity>`
directly; `Identity` deliberately does not implement `Clone` (documented on
`SupervisorService::client_identity_bytes`), so a `&self` method cannot move
it out to hand `SyneroymClient::new_with_identity` an owned value. Resolved
the same way `LiveQueueConnector` already does: store `identity_bytes:
Option<[u8; 32]>` and reconstruct via `Identity::from_bytes` per fetch.
Recorded here since it changes what the plan's own §3 phase 4 sketch shows.

**Test coverage**: every numbered test in the slice plan's §4 (19 through
59) is present, plus a handful beyond the plan's minimum (an extra
`to_topology_entry` cache-TTL-override case, a local/foreign collision case
alongside the two-foreign-apps one, and a `roymctl` CLI-parsing smoke test
for the new `resolve` subcommand) — 53 new test functions in total: 5 in
[resolver.rs](../../../../crates/app_orchestration/src/resolver.rs) (40
total in that file now), 12 in
[topology_document.rs](../../../../crates/app_orchestration/src/topology_document.rs),
8 in [store.rs](../../../../crates/app_supervisor/src/store.rs) (35 total),
5 in [topology.rs](../../../../crates/app_supervisor/src/topology.rs), 15 in
[service.rs](../../../../crates/app_supervisor/src/service.rs), 1 in
[sdk/src/topology.rs](../../../../crates/sdk/src/topology.rs), 1 `roymctl`
CLI test, and 6 e2e.

**Matrix and budget coverage**: task.md's failure/security matrix rows 4
through 10 (S1 closed 1-3 and 11) each have a named test per the slice plan
§4's map; performance budgets 1 and 3 (this slice's own) are both covered by
`one_fetch_and_register_serves_every_later_resolve`'s two assertions.

**Verification**:
- `cargo +nightly fmt --all`: clean.
- `cargo clippy --workspace --all-targets --all-features`: clean.
- `cargo test --workspace` (sandboxed): every unit test passes, including
  every new one this slice adds --
  [syneroym-app-orchestration](../../../../crates/app_orchestration) 152/152,
  [syneroym-app-supervisor](../../../../crates/app_supervisor) 272/272,
  [syneroym-sdk](../../../../crates/sdk) lib 46/46, `roymctl` lib 65/65. The
  same fixed, pre-existing set of crates needs real socket binds the
  sandboxed environment refuses (`Operation not permitted`) as S1's own
  evidence section documents (`syneroym-community-registry`,
  `syneroym-control-plane`, `syneroym-mqtt-broker`,
  `syneroym-coordinator-iroh`'s four integration-test binaries,
  `syneroym-sdk`'s `connect_timeout`, and every `crates/substrate/tests/
  *_e2e.rs` integration binary) — each independently re-run with the sandbox
  disabled: `syneroym-community-registry` 16/16, `syneroym-control-plane`
  176/176, `syneroym-mqtt-broker` 12/12, `syneroym-coordinator-iroh` lib 2/2
  plus all 4 integration binaries, `syneroym-sdk`'s `connect_timeout` 1/1 —
  all pass in full.
- `crates/substrate`'s ~20 `tests/*_e2e.rs` integration binaries (sandbox
  disabled, each boots one or two real substrate nodes): this slice's own
  new file,
  [topology_document_e2e.rs](../../../../crates/substrate/tests/topology_document_e2e.rs),
  passes in full (all 6 tests, `--test-threads=6`, ~170s). An exhaustive
  run of every file in the crate (`--test-threads=4`) found exactly one
  failure in ~20 files/dozens of tests: `messaging_client_e2e`'s
  `test_native_subscriber_receives_push_delivery_and_close_unsubscribes`, a
  p99-delivery-latency budget assertion (17.99ms observed) unrelated to
  this slice's own code (no messaging/MQTT path touched) -- confirmed a
  load artifact, not a regression, by re-running that one file alone
  (`--test-threads=1`), where it passes cleanly. Every other file --
  `app_instance_identity_e2e`, `basic_lifecycle`, `binding_push_e2e`,
  `cert_renewal_e2e`, `durable_outbox_e2e`, `federated_fdae_e2e`,
  `health_monitoring_e2e`, `http_passthrough_e2e`, `instance_identity_e2e`,
  `master_endpoint_record_e2e`, `tier1_endpoint_record_e2e`, and this
  slice's own `topology_document_e2e` -- passed outright. Consistent with
  this slice's only production changes to shared e2e-exercised code being
  the mechanical `TopologyKey`/`not_after` call-site updates enumerated
  above.
- `mise run test:e2e` (Playwright WebRTC suite): **4 passed**. Unaffected by
  this slice, as expected (no client-gateway or WebRTC surface touched).

### S2 — Post-merge review fixes (2026-08-09)

An independent review against this plan and `task.md`'s exit criteria found
14 findings in the shipped commit. 13 were confirmed and fixed; one (a
`cache_ttl` refresh gap) was investigated and declined as a code change,
recorded instead as a sharper backlog row. See
[slice-s2-implementation-plan.md](slice-s2-implementation-plan.md) §0's
"Post-merge code review" addendum for the five defects that change a
decision this plan made; the summary here is the full list, fix vs.
pushback, and the re-verification.

**Fixed:**

1. **The signed-document cache ignored the app DID** (High, correctness).
   `handle_resolve`'s `signed_documents` cache hit condition now also
   requires `cached.signed.document.app_did == app_did`, closing a window
   where an in-place handover could serve a document no caller could
   verify. `crates/app_supervisor/src/service.rs` (`handle_resolve`).
2. **A failed fingerprint write could brick `resolve` permanently, and a
   genuine concurrent `submit` failed every `resolve` for its duration**
   (High + Medium, correctness + concurrency). `handle_resolve` now falls
   back to taking the per-instance async lock and using the *advancing*
   form of the fingerprint write once, after two lock-free attempts still
   disagree — repairing a stuck row and riding out a real race instead of
   erroring. Same file.
3. **A cached document could report a stale `generation`** (Medium,
   correctness). Added to the same cache hit condition as finding 1.
4. **The Tier-1 HTTP lookup was never bound to the DID it was asked for**
   (High, security). `RegistryClient::lookup`'s HTTP branch now checks the
   returned record's `service_id` against the requested id (when the id is
   a full DID), mirroring the check the DHT branch already had.
   `crates/core/src/dht_registry.rs`.
5. **D-S2-6's re-sign-at-half-validity rule had no test.** Added
   `a_nearly_expired_cached_document_is_re_signed_rather_than_served`.
6. **`resolve` was missing from
   `every_verb_is_refused_without_substrate_admin`.** Added.
7. **The scale-out e2e never exercised the caller's own `LogicalResolver`
   cache**, only the supervisor's. Extended to register both documents and
   assert `resolve_all` reflects the second.
   `crates/substrate/tests/topology_document_e2e.rs`.
8. **A 1s `not_after` margin in a resolver unit test raced the wall-clock
   second boundary.** Widened to 3s, matching commit 38ebbea's flakiness
   fixes. `crates/app_orchestration/src/resolver.rs`.
9. **`topology_document_not_after_secs`/`cache_ttl_secs` were unvalidated**
   (a `0` or a `cache_ttl` at/above half of `not_after` both broke a stated
   property). Clamped with a warning in `SupervisorService::new`, same
   shape as the existing `max_renewals_per_pass` guard.
10. **`AppDid` permitted `/` and `#`**, unlike its two siblings, despite
    being interpolated into a `synapp:<app-did>` `ResourceUri`. Forbidden
    now. `crates/app_orchestration/src/models.rs`.
11. **Duplicate `member_index` values were not refused** in
    `service_topology`, leaving output order plan-order-dependent.
    Refused as a third `InconsistentPlan` case.
    `crates/app_supervisor/src/topology.rs`.
12. **An unknown service name came back as `InternalError`** instead of
    `InvalidParams`. Fixed in `handle_resolve`.

**Declined as a code change:** `cache_ttl`'s "on expiry try to refresh"
(ADR-0022 §3) has no implementation anywhere yet — not a regression this
slice introduced, and building a scheduled refresher for S2 would be
building ahead of a consumer (S3/S4's own substrate-side fetcher, D-S2-13)
that does not exist yet. Recorded as a sharper backlog row instead of a
new one (the existing D-S2-13 row already named the closest cause).

**Not covered by a new test:** finding 4's `RegistryClient::lookup` fix has
no dedicated regression test — reproducing a malicious registry's mismatched
HTTP response needs a mock HTTP server this crate's test module does not
have today. The fix mirrors an already-tested sibling check
(`extract_verified_endpoint_from_packet`'s `service_id == id`), and every
existing `dht_registry`/`community_registry` test still passes.

**Also fixed in this pass, unrelated to the review**: a flaky
`supervisor_alerts_e2e` failure ("no viable network path exists: last path
abandoned by peer" on the first `submit` after `managed_node`'s full boot)
— same root cause and same reconnect-then-retry fix as commit 38ebbea's
`app_instance_identity_e2e` fix.
`crates/substrate/tests/supervisor_alerts_e2e.rs`. Investigating it also
surfaced one real gap behind the "Endpoint dropped without calling
`Endpoint::close`" log noise every e2e file emits on teardown:
`ConnectionRouter::shutdown` propagated `Router::shutdown`'s `Err` via `?`,
which skipped its own fallback `ep.close()` on that path. Fixed to match
`coordinator_iroh::Coordinator::shutdown`'s existing shape.
`crates/router/src/connection_router.rs`.

**Re-verification**: `cargo +nightly fmt --all` clean; `cargo clippy
--workspace --all-targets --all-features` clean; `cargo test --workspace`
— see numbers below.

### S2 — Second-round review fixes (2026-08-09)

A follow-up pass on the round above found four more gaps, all fixed:

1. **The repair path re-read `state` but never re-checked `retired`**
   (`handle_resolve`). A `resolve` that reaches the locked repair branch
   while a concurrent `retire` holds the same instance lock blocks, wakes
   once `retire` finishes, re-reads a now-`retired` state, and signed
   anyway — the lock hand-off makes this the *expected* ordering, not a
   narrow race, since the pre-lock `retired` check ran before `retire`
   landed. Fixed: the same `if state.retired { return Err(denied()) }`
   guard, re-run after the repair path's re-read.
2. **Findings F1, F2, F4, and F6 shipped with no regression test.** The
   cache hit condition is four `&&` clauses now, two of which are one
   refactor away from silently vanishing. Added three tests (F2 and F6
   share the identical fix, so one test covers both):
   `a_handover_to_a_different_app_did_does_not_serve_the_previous_masters_cached_document`
   (F1, via `record_adopt` with a different DID and no vault key
   rotation, asserting the fresh-sign path runs and correctly refuses
   rather than serving the stale document),
   `a_generation_bump_with_no_membership_change_is_not_served_from_a_stale_cache`
   (F4, via `record_adopt` at a new generation with the same DID and
   plan), and
   `resolve_repairs_a_topology_epoch_row_stuck_on_the_wrong_fingerprint`
   (F2/F6, via a directly-written `record_topology_fingerprint` row that
   disagrees with the real plan).
3. **The cache-TTL clamp (`not_after_secs / 4`) could itself produce
   `0`** for any `not_after` under 4 seconds, and a reader taking that
   `0` as its own cache TTL gets `Duration::ZERO`, which never registers
   a cache hit. Only reachable with a misconfigured (very short)
   `not_after` today, mostly relevant to tests. Fixed with `.max(1)`, and
   pinned by `the_cache_ttl_clamp_never_produces_zero`.
4. **`AppDid`'s new `/`/`#` refusal had no test**, unlike both sibling
   wrappers (`AppInstanceId`, `LogicalServiceName`). Added
   `an_app_did_containing_a_separator_is_refused`, matching their
   existing test shape.

**Re-verification**: `cargo +nightly fmt --all` clean; `cargo clippy
--workspace --all-targets --all-features` clean; `cargo test -p
syneroym-app-supervisor --lib` 280/280 (4 new), `cargo test -p
syneroym-app-orchestration --lib` 153/153 (1 new).

### S2 — Third-round review: a test that passed for the wrong reason

A third pass on the tests above found finding 2's F1 test
(`a_handover_to_a_different_app_did_does_not_serve_the_previous_masters_cached_document`)
was not discriminating: it called `record_adopt("inst-1", 2, new_app_did)`,
which bumps `generation` to `2` *and* changes the DID in one write
(`store.rs`'s `record_adopt` sets both columns together). Since the cache
hit condition already checks `generation`, that clause alone forced the
cache miss the test asserts on -- the `app_did` clause it meant to pin
was never consulted, and deleting it left the test green. Fixed by holding
`generation` at `1` (unchanged from `adopted_instance`'s own call) and
changing only the DID, isolating the dimension the test names. Verified
by temporarily removing the `app_did` clause from the cache hit condition
and confirming the test now fails (`Result::unwrap_err()` on an `Ok`
carrying the stale document), then restoring it. F4's mirror-image test
was already discriminating -- it holds the DID fixed and moves only the
generation -- and needed no change.

Also found in this pass, pre-existing and unrelated to S2 (that slice's
only change to `keys.rs` is one fixture field): a flaky
`keys::tests::get_or_mint_warns_with_the_wording_matching_its_kind`, whose
log-capture harness (`run_capturing_logs`) uses a thread-local
`tracing` subscriber that only sees a warning if it happens to fire on the
same thread that installed it. Recorded in
[deferred-backlog.md](../../deferred-backlog.md), not fixed here.
