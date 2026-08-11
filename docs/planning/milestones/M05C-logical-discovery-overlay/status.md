# M05C Logical Service Discovery Overlay — Status

**Milestone:** [task.md](task.md) · **Design of record:**
[ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md)
(Accepted 2026-08-02) · **Plan:**
[implementation-plan.md](implementation-plan.md)

**Overall:** 🚧 **In progress. S1 and S2 complete 2026-08-08; S3 substantially
complete 2026-08-10, post-review pass 2026-08-11 (Playwright tests 103-104
still not implemented, see S3's evidence below).** Promoted from the
*Committed Work: Logical Service
Discovery Overlay* section of
[meta-implementation-plan.md](../../meta-implementation-plan.md) into a
milestone directory, so the largest committed-but-unplanned work in the tree
carries the same discipline as everything else.

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
| S3 | Gateway hostname scheme + routing-key header; coordinator relay | ⚠️ **Substantially complete (2026-08-10); post-review pass 2026-08-11** — [plan](slice-s3-implementation-plan.md); evidence below. Playwright tests 103-104 blocked by a genuine iroh self-dial deadlock, not implemented | S2 **cleared** |
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

### S3 — Verification evidence (2026-08-10)

**What shipped**, by phase (see
[slice-s3-implementation-plan.md](slice-s3-implementation-plan.md) §2 for the
phase plan):

- **Phase 1 (one builder, one parser, in `syneroym-core`):** `TargetHost`
  (`Service`/`App` variants) and a rewritten `parse_target_host` in
  [protocol_utils.rs](../../../../crates/core/src/protocol_utils.rs), reading
  the first DNS label and popping `-i`/`-s`/`-a` right to left with the
  fixed-width guard §0.12 found necessary; everything past that first label
  is ignored, same as before S3. `generate_alias` reshaped to drop its `-p`
  letter (D-S3-14), and two new builders,
  `generate_service_host`/`generate_app_host`, in
  [util.rs](../../../../crates/core/src/util.rs), both refusing a label over
  the 63-character DNS limit. **Directed out during this slice's own
  implementation (2026-08-10)**: the plan's own D-S3-13 shipped a trailing
  `-roym1` format-version marker, in two revisions — first as the label's own
  last dash-segment, then moved to its own `.roym1.` subdomain label to free
  6 characters back into the nickname budget — before being dropped
  entirely, since the parser was already domain-agnostic before S3 and a
  future grammar change is a problem for whenever it actually happens, not
  one worth a marker today. `client_gateway`'s own duplicate parser is
  deleted; every hand-formatted host site (`roymctl alias`,
  `basic_lifecycle.rs`, `tcp_proxy_latency.rs`, the four hand-built
  `community_registry` test aliases) now goes through these four functions —
  what makes exit criterion 5 true.
- **Phase 2 (the destination resolves what the caller could not):**
  `resolve_service_name` in
  [topology.rs](../../../../crates/app_supervisor/src/topology.rs) — an exact
  name match wins, a hash matching two declared names is refused
  (`AmbiguousHash`) — wired into `handle_resolve` so the signed document
  always carries the real name, never the hash a caller sent. `EndpointRegistry`
  in [local_registry.rs](../../../../crates/core/src/local_registry.rs) gained
  `resolve_interface`, folding the exact/hash/empty cases into one function;
  an empty interface resolves to "the one app-declared interface" only at the
  hop that terminates the route (`route_handler/io.rs`) — a relay hop still
  forwards it untouched, pinned by test 81.
- **Phase 3 (the client gateway's app-scoped path):** `[iam]
  .grant_resolve_to_node_did` and `[roles.client_gateway] resolve_ucan` (new
  `SubstrateConfig`/`ClientGatewayRole` fields), a same-node bare
  `substrate:<node_did>` grant with ability `supervisor/resolve` alongside the
  existing `admin_ucan_root` branch in
  [route_handler/io.rs](../../../../crates/router/src/route_handler/io.rs).
  `GatewayState` gained its own `LogicalResolver`, `RegistryTopologyFetcher`,
  and two caches (`app_dids`, `service_names`); `resolve_app_host` does the
  Tier-1-then-Tier-2 walk with both D-S3-5 binding checks
  (`short_hash(record.info.service_id) == a_hash`,
  `short_hash(document.service_name) == s_hash`), refusing rather than
  resolving on either mismatch. `RegistryTopologyFetcher` gained
  `fetch_via`, skipping the redundant second Tier-1 lookup the plan's own
  first draft had (D-S3-17).
- **Phase 4 (the coordinator resolves an app-scoped host):** the WebRTC
  coordinator gained an identity (the node's own key, so the same
  `grant_resolve_to_node_did` gate covers it), a `topology_fetcher` and
  `resolver` on `BootstrapState`, and the same `resolve_app_host` logic
  reused from phase 3. `peer-proxy.html`/`peer-proxy.js` gained an
  interpolated `TARGET_INTERFACE`, and both of the page's own hostname
  parsers (raw-tunnel and service-worker paths) are **deleted** — the net
  effect of phase 4 on `templates/` is a deletion, not an addition (D-S3-16).
- **Phase 5 (the operator surface, the e2e proof):** `roymctl alias` gained
  `--service`/`--domain`; a clap-level check refuses `--service` without
  `--nickname`. New
  [gateway_hostname_e2e.rs](../../../../crates/substrate/tests/gateway_hostname_e2e.rs)
  covering tests 99-102 against two real substrates and a real registry —
  the milestone's cross-app half, from an ordinary HTTP client, including the
  routing-key header over the wire and both credential shapes D-S3-6
  describes (a same-node grant with no credential file, and a cross-node
  `resolve_ucan` token).

**Playwright tests 103-104 are not implemented — a genuine blocker, not an
oversight.** Both need a real app instance (adopted, with an app master DID)
reachable through the WebRTC bootstrap page, and the Playwright fixture
(`crates/substrate/tests/e2e/`) runs exactly one substrate process, so the
only way to build that instance is a supervisor deploying onto **its own**
node — a shape nothing else in the tree exercises (every Rust e2e's managed
node, including this slice's own `gateway_hostname_e2e.rs`, is a genuinely
separate process/DID). Standing this up (`[roles.supervisor]`, `[storage]
encryption = false`, a node-wide `substrate/admin` self-grant, a `substrates.toml`
inventory naming the node's own DID, `supervisor submit`/`adopt` against a
two-interface `backend` manifest) worked right up to `adopt`, which calls
`build_clients` → `connected_client` → `SyneroymClient::connect` to read the
instance's currently-held generation from every substrate it is placed on —
here, itself. That call **hung indefinitely**: reproduced twice, in the
identical call chain both times (confirmed with macOS `sample(1)`, since
`lldb -p` attach is blocked in this environment — parked inside
`Endpoint::bind()`/`endpoint.connect()`, not spinning), killed after 40+
minutes with zero forward progress, even though `wait_for_ready`'s own
`time::timeout` should have bounded the attempt to `MANAGED_SUBSTRATE_CONNECT_TIMEOUT`
(10 seconds). Not root-caused past that point — a same-node self-dial through
the coordinator's own relay is a case iroh's own architecture may never
exercise. Filed as its own backlog row
([deferred-backlog.md](../../deferred-backlog.md) §1) with the full repro
path and exact file:line citations, rather than forced. The fixture changes
attempted for this were reverted in full (`global-setup.ts`,
`webrtc.spec.ts` are unchanged from phase 5's commit) so the rest of the
Playwright suite is not put at risk by code that hangs.

**A bug found and fixed along the way, unrelated to S3's own design but
blocking even the already-committed fixture code**: `roymctl svc deploy
--tcp` refused more than one `--interfaces` value
(`apps/roymctl/src/commands/svc.rs`), even though `deploy_svc_tcp` and the
underlying WIT `network-endpoint` record already take a list — the guard was
an artificial CLI-level restriction with nothing behind it. This blocked
`global-setup.ts`'s own pre-existing two-interface TCP deploy (`--interfaces
http,admin --tcp ...`, added for test 104's fixture) from ever running.
Fixed by building one `NetworkEndpoint` per declared interface, all naming
the same `(host, port)` — a TCP passthrough has nothing to dispatch on, so
every declared interface is just another registered name for the identical
backend.

**Test coverage**: 43 new test functions across the phases above — 8 in
[protocol_utils.rs](../../../../crates/core/src/protocol_utils.rs), 5 in
[util.rs](../../../../crates/core/src/util.rs), 3 in
[local_registry.rs](../../../../crates/core/src/local_registry.rs), 5 in
[app_supervisor/topology.rs](../../../../crates/app_supervisor/src/topology.rs),
1 in [service.rs](../../../../crates/app_supervisor/src/service.rs), 2 in
[client_gateway/gateway.rs](../../../../crates/client_gateway/src/gateway.rs),
4 in [bootstrap.rs](../../../../crates/coordinator_webrtc/src/bootstrap.rs),
2 in [route_handler/io.rs](../../../../crates/router/src/route_handler/io.rs),
7 in [sdk/topology.rs](../../../../crates/sdk/src/topology.rs), 2 in
`roymctl`'s [cli_args.rs](../../../../apps/roymctl/tests/cli_args.rs), and 4
e2e in `gateway_hostname_e2e.rs` (tests 99-102).

**Matrix and budget coverage**: no new matrix row (S1 closed 1-3/11, S2
closed 4-10); rows 6, 7, and 10 gain a second named test at the hostname
layer (test 87 for expiry, test 101 for a clean denial, test 78 for the
epoch surviving a hashed request) per the slice plan §4. Budget 1
re-measured at the gateway (test 85, a fetch-count assertion); budget 2 —
one Tier-1 lookup per cold app-scoped resolve, not two — measured for the
first time in this milestone (test 86, a registry-call-count assertion).

**A live requester correction, worked out during this slice's own
implementation (2026-08-10), not part of the original plan**: D-S3-13's
`-roym1` format-version marker is gone entirely, in two rounds the same
day — first moved from the label's own trailing dash-segment to a `.roym1.`
subdomain label (freeing 6 characters into the nickname budget, since DNS
label limits are per-label), then dropped outright, since the parser was
already domain-agnostic before S3 (everything past the first label was, and
still is, ignored) and a version marker was paying for a problem — a future
grammar change — that has no consumer yet. The corrected grammar carries no
marker at all: `<nickname>-a<app-did-hash>-s<service-name-hash>
[-i<interface-hash>].<domain>`, freeing the nickname budget to 33 characters
on an app-scoped host (43 with no `-i`), up from the plan's original
27/37. See [ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md)
§7's amendment and this plan's D-S3-13 for the full record. Every builder,
parser, and test this touched — `crates/core/src/protocol_utils.rs`,
`util.rs`, `apps/roymctl/tests/cli_args.rs`, `crates/client_gateway/src/
gateway.rs`, `crates/coordinator_webrtc/src/bootstrap.rs` — was re-verified
after the change (below); the 43 new-test count above is unaffected, since no
test functions were added or removed, only their bodies.

**Verification**:
- `cargo +nightly fmt --all`: clean.
- `cargo clippy --workspace --all-targets --all-features`: clean.
- `cargo build --workspace --all-targets`: clean (confirms nothing else in
  the tree referenced the now-deleted `HOST_FORMAT_MARKER`).
- `cargo test --workspace` (sandboxed): everything not needing a real socket
  bind passes clean, including every one of the 43 new tests above. The same
  fixed set S1/S2's evidence documents needs a separate sandbox-disabled
  re-run for a real socket bind (`Operation not permitted` under the
  sandbox) — `syneroym-community-registry`, `syneroym-control-plane`,
  `syneroym-mqtt-broker`, `syneroym-coordinator-iroh`'s integration
  binaries, `syneroym-sdk`'s `connect_timeout` — **plus one new addition
  this slice**: `syneroym-coordinator-webrtc`'s own lib tests, since phase 4
  added `#[tokio::test]`s in `bootstrap.rs` that bind real sockets too. Every
  one of those, re-run independently with the sandbox disabled, passes in
  full: `syneroym-community-registry` 16/16, `syneroym-control-plane`
  176/176, `syneroym-coordinator-iroh` 9/9 (2 lib + connection_limit 1/1 +
  multi_hop_relay 5/5 + tls_rotation 1/1), `syneroym-coordinator-webrtc`
  6/6, `syneroym-mqtt-broker` 12/12, `syneroym-sdk`'s `connect_timeout` 1/1.
  One pre-existing, already-documented flaky failure hit during a
  `--no-fail-fast` full run and unrelated to this slice (`syneroym-router`'s
  `native_dispatch_identity`, a `mainline::dht` "actor thread unexpectedly
  shutdown: SendError" panic under parallel load — see
  [deferred-backlog.md](../../deferred-backlog.md) §1): confirmed by
  re-running the file alone with `--test-threads=1`, 39/39.
- `crates/substrate`'s `tests/*_e2e.rs` integration binaries (sandbox
  disabled, `--test-threads=4`, ~29 binaries): this slice's own new file,
  `gateway_hostname_e2e.rs`, passes in full — all 4 tests (99-102), ~130s,
  re-verified again after the marker-removal redesign. Every other file
  passed too, except `durable_outbox_e2e`, which hit `Address already in
  use` on a port a concurrently-running parallel session (a different
  worktree, fixing an unrelated e2e issue on this same machine) was using at
  the time — not a regression from this slice, which touches no code in the
  outbox/queue path.
- `mise run test:e2e` (Playwright WebRTC suite): **12 passed**, re-run twice
  (once before, once after the marker-removal redesign) — the pre-existing 8
  in `webrtc.spec.ts` and 4 in `multi-hop.spec.ts`, unaffected by this
  slice's own gateway/coordinator changes (both still address a service by
  its unscoped `-s...` host, now with no `-roym1` marker at all). Tests
  103-104 are not present, per the blocker above.

### S3 — Post-review pass (2026-08-11)

An independent review against `task.md` and this slice's own plan found 17
findings in the shipped commit (`baf5f22`): 4 correctness, 5
security/robustness, 7 test-coverage, 1 clarity. 15 were fixed; 1
(A2) was investigated and declined as a code change in this pass, recorded
instead as a sharper backlog row; 1 (B5) was a "note, not a defect" the
review itself flagged, recorded as a backlog row rather than a fix. All
fixes re-verified (below).

**Fixed:**

1. **The unscoped host builder skipped §0.12's ambiguous-nickname refusal**
   (A1, critical). `refuse_ambiguous_nickname_tail` was wired into
   `generate_app_host` only; `generate_service_host` minted a host whose
   nickname's own final segment could misread as a real `-a<hash>` segment
   on parse. Called from both builders now; test 70 extended to cover both
   (also closes C4, the matching test-coverage finding).
   `crates/core/src/util.rs`.
2. **Every `resolve` error was treated as a cache miss** (A3, major),
   so a *permanent* selection failure (`Sharded` with no routing key, an
   empty member set) refetched Tier 2 on every single request, breaking
   task.md budget 1 for any caller stuck in one of those states.
   `LogicalResolver` gained a typed, downcastable
   `RetryableResolveError`/`is_retryable_resolve_error` distinguishing
   "not registered" and "expired" (genuinely retryable) from everything
   `select_member` returns (a permanent property of the document, not a
   cache state) — `resolve_app_host`'s warm path now falls through to a
   refetch only on the former, and returns the latter straight to the
   caller. `crates/app_orchestration/src/resolver.rs`,
   `crates/sdk/src/topology.rs`.
3. **An empty interface reached the guest proxy path** (A4, major),
   contradicting D-S3-15's own claim that it "simply fails" there.
   `check_native_capability_gate`'s `matches_interface` closure can never
   match `""` against a real interface name, so nothing stopped a guest
   call from reaching `registry.lookup(target, "")`, which now resolves to
   "the one app-declared interface" (D-S3-15). No live escalation --
   `NODE_NATIVE_INTERFACES` was not filtered by that empty-interface
   resolution, but every node registers both `orchestrator` and
   `security`, so the ambiguity check happened to refuse it -- but that
   safety was incidental, not designed. Fixed two ways: the gate now
   refuses an empty interface for `CallOrigin::Guest` outright (a WASM
   guest always names the interface it wants; the convenience is for a
   caller reading a hostname, not a guest), and `resolve_interface`'s
   empty branch now filters `NODE_NATIVE_INTERFACES` alongside
   `NATIVE_CAPABILITY_INTERFACES` as defence in depth.
   `crates/router/src/proxy.rs`, `crates/core/src/local_registry.rs`.
4. **`AppHostResolver`'s Tier-1 cache was keyed by `a_hash` alone**
   (B3, minor) while the registry lookup itself uses the full
   `app_lookup_alias`, so a warm entry could answer a *different*
   nickname over the same app hash without its own alias lookup --
   D-S3-5's binding check still bound the answer to the right app DID, so
   this never crossed an authority boundary, but it silently widened what
   the parser accepts. Keyed on the alias now.
   `crates/sdk/src/topology.rs`.
5. **`TARGET_INTERFACE` (and every other constant on the bootstrap
   page) was interpolated into a `<script>` block through askama's
   default HTML escaper** (B4, minor) -- the right escaper for an HTML
   *body*, not a JS string literal: it handles `"`/`<`/`&`, never `\`, so
   a value ending in an odd number of backslashes (reachable through the
   deliberately permissive `-i` segment, D-S3-12) could escape the
   closing quote. Not reachable from a real browser URL, but the wrong
   escaper for the context regardless. Every constant now renders through
   askama's `json` filter (new `serde-json` cargo feature) followed by
   `|safe`, producing a complete, self-quoting JS/JSON literal instead of
   a hand-quoted, HTML-escaped one. `Cargo.toml`,
   `crates/coordinator_webrtc/templates/peer-proxy.html`.
6. **The public, unauthenticated WebRTC bootstrap listener did
   uncached Tier-1/Tier-2 work per request, with no bound on concurrent
   in-flight resolves** (B1 + B2, both major). `AppHostResolver` gained
   two things behind its existing warm-path cache: a per-`(app_lookup_
   alias, s_hash)` `tokio::sync::Mutex`-based single-flight (B2), so
   concurrent callers for the same not-yet-cached host share one Tier-1-
   then-Tier-2 round trip instead of each starting an independent one;
   and a 5-second negative-result cache (B1), so a caller repeating the
   same unresolvable host does not repeat a full round trip for every
   repeat. Both apply uniformly to the client gateway and the coordinator,
   since they share this one resolver (D-S3-7/D-S3-11).
   `crates/sdk/src/topology.rs`.
7. **e2e test 100 could not fail regardless of correctness** (C1,
   critical): both `Redundant` replicas proxy to the *same* physical TCP
   backend by construction in this milestone (`service_manifest` clones
   `config.source` per `member_index`), so comparing response *content*
   across two keyed requests was true no matter which member the gateway
   actually dialed -- and the test reused one `Client` across every
   request, so finding A2's per-connection behavior would have masked a
   real bug too. Renamed and rewritten: the backend now echoes the exact
   `X-Syneroym-Routing-Key` bytes it received, each request runs over its
   own fresh connection, and the assertion is on that echoed value (three
   requests: none, `"alice"`, `"bob"`) -- proving the header travels the
   real wire unmodified per request, which is what this test's own doc
   comment always said its job was. Per-member selection consistency
   stays unit-tested, with real members to distinguish, at `syneroym-sdk`
   test 88. `crates/substrate/tests/gateway_hostname_e2e.rs`.
8. **Test 88 never asserted its own title's second half** (C2, major)
   -- "the same key twice returns the same member" was covered; "no
   header returns members in round-robin" was not. Added.
   `crates/sdk/src/topology.rs`.
9. **`global-setup.ts` computed and exported `APP_ALIAS_ADMIN` for
   Playwright test 104**, which no spec consumes and which cannot land
   until the iroh self-dial deadlock blocking 103-104 is resolved (see
   the backlog row below) -- left in place, it would have rotted into a
   false signal (C3, major). Dropped the unused alias computation/export
   and the second declared interface it existed to support; the TCP
   deploy is back to a single `http` interface.
   `crates/substrate/tests/e2e/global-setup.ts`.
10. **Test 84 passed an unformatted literal string and accepted either
    of two unrelated error messages** (C5, minor), so it would have
    passed on a Tier-1 regression as readily as on the Tier-2 binding
    check it exists to pin. Alias now built with `format!`; the
    assertion now pins the Tier-2 message specifically.
    `crates/sdk/src/topology.rs`.
11. **Test 82 ("the gateway's own regression pin") exercised no
    gateway code**, only `protocol_utils::parse_target_host` -- a copy of
    that module's own test 60 (C6, minor). The `TargetHost ->
    (service_id, interface)` decision `handle_connection` makes is now
    its own function, `resolve_target`, with its own two direct unit
    tests (the unscoped pass-through, and an app-scoped resolution
    failure surfacing as `resolve_target`'s own `Err`); test 82 itself is
    retitled to say what it actually checks.
    `crates/client_gateway/src/gateway.rs`,
    `crates/client_gateway/Cargo.toml` (new `async-trait` dev-dependency).
12. **The gateway's own no-registry-configured path had no test**
    (C7, minor) -- covered only through the coordinator's test 96. Added.
    `crates/sdk/src/topology.rs`.
13. **`CredentialWarning`'s doc comment opened with a truncated,
    mid-sentence paragraph copied from `AppHostResolver`'s own doc** (D1,
    minor merge artifact) -- the real `AppHostResolver` doc appeared
    again, complete, thirty lines later. Deleted the stray paragraph.
    `crates/sdk/src/topology.rs`.
14. Unrelated to the review, found while investigating a `cargo audit`
    warning the requester separately flagged: `smartstring` (transitively
    pulled in by `swc_ecma_parser`, itself pulled in by the `swc_core`
    build-dependency this crate uses to minify `sw.js`/`peer-proxy.js` in
    release builds) is unmaintained
    ([RUSTSEC-2026-0249](https://rustsec.org/advisories/RUSTSEC-2026-0249)).
    Bumped `swc_core` from `68.0.5` to the current `76.0.0`; confirmed via
    `cargo tree -i smartstring` that the newer version's dependency graph
    drops it entirely (`swc_atoms` moved to the `hstr` crate), rather than
    adding an `audit.toml` suppression for a warning a real upgrade
    removes. `build.rs`'s `swc_core` API usage needed no changes across
    the eight-minor-version jump; a release build was run to completion to
    confirm the minifier still runs, and `cargo audit` is clean.
    `Cargo.toml`, `crates/coordinator_webrtc/Cargo.toml`.

**Declined as a code change:** the routing-key header (A2, critical) is
read once, from the first HTTP request on a TCP connection, and the whole
connection is then handed to `passthrough_with_conn`'s raw bidirectional
byte copy for its lifetime -- every later request an HTTP keep-alive
reuses that connection for rides the member chosen for request one,
regardless of its own header. A real fix needs the gateway to parse HTTP
request boundaries *inside* an already-open raw byte tunnel and
potentially re-select a member mid-connection -- turning
`ServiceType::Tcp` passthrough from a byte-level proxy into an HTTP-aware
one, a substantially larger change than this pass's scope. Documented
instead: a caller-facing note in the developer guide's gateway-hostname
section, a code comment at the read site, and a backlog row.
`crates/client_gateway/src/gateway.rs` (`handle_connection`),
[developer-guide.md](../../../developer-guide.md).

**Backlog rows added** (both in [deferred-backlog.md](../../deferred-backlog.md)
§7): A2's per-connection routing-key limitation above, and B5's note that
the `-a<app-did-hash>`/`-s<service-name-hash>` binding rests on a 40-bit
(`short_hash`) collision space, closed against an *unrelated* record only
by D-S3-5's check and against a *colliding, same-nickname* record only by
the registry's alias-collision refusal at admission -- itself in-memory
and rebuilt on restart. Not fixed: S3 makes this hash the root of a
resolution chain that previously ended at a plain service lookup, and
closing it for real (persisting the alias map, or widening the hash) is
sized for the next gateway-hostname format break, not a review-pass fix.

**Test coverage**: 8 new regression tests, plus the C1 rewrite and the C2
addition to an existing one --
`a_permanent_selection_failure_is_not_treated_as_a_cache_miss`,
`a_different_nickname_over_the_same_app_hash_repeats_the_tier1_lookup`,
`an_app_scoped_host_is_refused_with_no_registry_configured`,
`a_recent_failure_is_served_from_the_negative_cache_without_a_repeat_lookup`,
`concurrent_cold_resolves_for_the_same_host_share_one_fetch` (all in
`sdk/src/topology.rs`); `guest_with_an_empty_interface_is_denied_before_
resolution` (`router/src/proxy.rs`);
`an_empty_interface_never_resolves_to_a_node_native_interface`
(`core/src/local_registry.rs`);
`resolve_target_passes_an_unscoped_host_through_unresolved` and
`resolve_target_routes_an_app_scoped_host_through_the_resolver_and_
surfaces_its_error` (`client_gateway/src/gateway.rs`);
`a_value_ending_in_a_backslash_cannot_escape_its_js_string_literal`
(`coordinator_webrtc/src/bootstrap.rs`).

**Verification**:
- `cargo +nightly fmt --all`: clean.
- `cargo clippy --workspace --all-targets --all-features`: clean.
- `cargo build --workspace --all-targets`: clean, including a `--release`
  build (exercises `coordinator_webrtc`'s `build.rs` minifier against the
  bumped `swc_core`).
- `cargo test --workspace --lib --bins` (every crate's unit tests, the fast
  and exhaustive half): **34/34 crates pass, 0 failures**, including every
  new test listed above. `cargo test --workspace` unqualified was tried
  first and abandoned -- it also serially runs all ~29 real-node
  `crates/substrate/tests/*_e2e.rs` binaries, which S1/S2/S3's own evidence
  above already puts at 40+ minutes for that directory alone, so the
  `--lib --bins` split plus a targeted e2e re-run below is this pass's
  verification shape, not a shortcut around it.
- Targeted `crates/substrate/tests/*_e2e.rs` re-runs (sandbox disabled),
  chosen for direct coverage of the changed code: `gateway_hostname_e2e`
  (this slice's own file, all 4 tests 99-102, ~124s), `topology_document_e2e`
  (exercises `LogicalResolver` directly, the file finding A3 changed, all 6
  tests, ~127s), `tier1_endpoint_record_e2e` (registry/Tier-1 path, both
  tests, ~123s), `basic_lifecycle` (calls `util::generate_service_host`
  directly, the function finding A1 changed, all 3 tests, ~11s) -- all
  pass.
- `mise run test:e2e` (Playwright WebRTC suite): **12 passed** (the same 8
  in `webrtc.spec.ts` + 4 in `multi-hop.spec.ts` as before), re-run twice
  after the `global-setup.ts` cleanup (finding C3) to confirm dropping the
  unused `APP_ALIAS_ADMIN` scaffolding changed nothing it shouldn't have.
- `cargo audit`: clean (see finding 14 above) -- previously one allowed
  warning (`smartstring`, `RUSTSEC-2026-0249`).

### S3 — Two residuals in the review-pass code itself (2026-08-11)

Found reading `ensure_populated` (`crates/sdk/src/topology.rs`) after the
pass above landed, both in the B1/B2 fix:

1. **`negative_cache` never swept.** An entry expired logically after
   `NEGATIVE_CACHE_TTL` but was only ever removed by a *later success* for
   that exact key -- a host that keeps failing keeps its entry forever.
   Both key parts (`app_lookup_alias`, `s_hash`) come straight off the
   `Host` header on the unauthenticated `0.0.0.0:7962` WebRTC bootstrap
   listener, so this was unbounded growth inside the fix meant to harden
   that exact path (slow -- one real Tier-1 round trip buys each new key
   -- but unbounded). Fixed: a failure now sweeps every expired entry
   (`retain`) before inserting its own, bounding the map to roughly one
   `NEGATIVE_CACHE_TTL` window's worth of distinct failures. Pinned by
   `a_new_failure_sweeps_every_expired_negative_cache_entry`.
2. **`inflight.remove` ran before the outcome was cached.** On the
   failure path, `self.inflight.remove(&coalesce_key)` ran, then the
   `negative_cache` write -- so a caller arriving in that exact window
   found neither the lock (already gone) nor the cached failure (not yet
   written), and started its own redundant fetch. One extra fetch under a
   real but narrow race, not a correctness bug (the success path was
   already safe, since `fetch_and_bind` writes `app_dids`/
   `service_names` before returning). Fixed by recording the outcome
   before dropping the in-flight lock, making the ordering deliberate
   rather than incidental. Not given its own regression test: reproducing
   the exact race window deterministically needs an injection point this
   pass judged not worth adding to production code for one timing test:
   `concurrent_cold_resolves_for_the_same_host_share_one_fetch` (finding
   B2) already covers the success-path single-flight property under real
   concurrency.

**Re-verification**: `cargo +nightly fmt --all -- --check` clean; `cargo
clippy --workspace --all-targets --all-features` clean; `cargo test -p
syneroym-sdk --lib topology::` 14/14 (1 new); `gateway_hostname_e2e.rs`
re-run in full, 4/4.

### S3 — A "what does an unnamed interface get called" pass (2026-08-11)

Raised in PR review, not by an independent tool: three different
"single interface, no name given" code paths had three different
answers, none of them a single source of truth.

- **`roymctl svc deploy`'s own `--interfaces` fallback was dead code.**
  `if ifaces.is_empty() { vec!["default"] } else { ifaces }`, applied
  *after* `interfaces.split(',')` -- but `"".split(',')` yields one
  empty-string element, never zero, so the length check could never see
  the empty case. `--interfaces ""` silently registered a service under
  the literal interface name `""` instead of falling back to anything.
- **The manifest-driven deploy path (`sdk::mapper`'s TCP mapping) had its
  own, different, silent default: `"main"`.** Same situation (`svc.config.
  interfaces.is_empty()`), different answer, and neither path knew about
  the other's choice.
- **WASM had no fallback at all** -- an empty `interfaces` list registers
  zero interfaces, which D-S3-15's own ambiguity rule then correctly
  refuses to resolve (zero is as ambiguous as two). Left alone: this one
  was never inconsistent, just a third shape.

**Fixed**, unifying on one shared name rather than just repairing the
broken check: `syneroym_sdk::mapper::DEFAULT_INTERFACE_NAME` (`"default"`,
chosen as the already-dominant convention -- used explicitly across ~18
files in `crates/substrate/tests/*_e2e.rs`, this slice's own D-S3-15 tests,
and the developer guide's own examples, versus `"main"`'s handful of
occurrences confined to `sdk::mapper` and one `control_plane::orchestration`
test file). `sdk::mapper`'s TCP-mapping fallback now uses the constant
instead of a hardcoded `"main"`. `roymctl`'s own parsing is now a real
function, `parse_interfaces`, with the check fixed to actually detect a
blank value (rather than an empty vector that could never occur) --
and a **new distinction this pass adds**: a *fully* blank `--interfaces`
falls back to the shared default, but a blank *segment* amid otherwise
real names (a stray comma, e.g. `"http,,admin"`) is refused outright
rather than silently coerced, since guessing which name the operator
actually meant there would be wrong as often as right. `apps/roymctl/
src/commands/svc.rs`, `crates/sdk/src/mapper.rs`.

**Doc-only, no code change**: the developer guide's deploy section gained
a note on "interface" meaning two different things depending on service
type -- a WIT-exported namespace for WASM, a named auxiliary port (closer
to "a metrics/readiness port alongside the main one") for TCP/container --
sharing the same `--interfaces` flag and hostname `-i` segment only
because `EndpointRegistry` doesn't need to know which sense applies.
Raised in the same review pass; not renamed, since the fix would ripple
through WIT files, the route preamble wire format, and every CLI flag for
a naming question, not a behavior one -- out of scope for this slice.
`docs/developer-guide.md`.

**Test coverage**: 3 new tests in `apps/roymctl/src/commands/svc.rs`
(`parse_interfaces_falls_back_to_the_shared_default_name_when_blank`,
`parse_interfaces_splits_and_trims_a_real_list`,
`parse_interfaces_rejects_a_blank_segment_in_an_otherwise_real_list`) plus
one composing `parse_interfaces` with the container-port validator
(`a_blank_interfaces_value_composes_with_a_default_named_container_port`);
2 new tests in `crates/sdk/src/mapper.rs`
(`a_tcp_service_with_no_declared_interfaces_gets_the_shared_default_name`,
`a_tcp_services_declared_interface_name_is_used_verbatim`).

**Re-verification**: `cargo +nightly fmt --all -- --check` clean; `cargo
clippy --workspace --all-targets --all-features` clean; `cargo test -p
syneroym-sdk --lib` 61/61 (2 new); `cargo test -p roymctl --lib` 69/69
(4 new). No e2e fixture passes a blank `--interfaces` (checked
`crates/substrate/tests/e2e/*.ts` and every Rust e2e file shelling out to
`roymctl svc deploy`), so none needed re-verification against the new
error path.
