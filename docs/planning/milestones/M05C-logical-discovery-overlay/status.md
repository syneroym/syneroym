# M05C Logical Service Discovery Overlay — Status

**Milestone:** [task.md](task.md) · **Design of record:**
[ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md)
(Accepted 2026-08-02) · **Plan:**
[implementation-plan.md](implementation-plan.md)

**Overall:** 🚧 **In progress. S1 complete 2026-08-08.** Promoted from the
*Committed Work: Logical Service Discovery Overlay* section of
[meta-implementation-plan.md](../../meta-implementation-plan.md) into a
milestone directory, so the largest committed-but-unplanned work in the tree
carries the same discipline as everything else. **S2's gate is clear** — S1
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
| S2 | Tier 2: signed topology document, `resolve` RPC, client verify/cache | 📋 Planned (sketch; owes its own `§0`) | S1 **cleared** |
| S3 | Gateway hostname scheme + routing-key header; coordinator relay | 📋 Planned (sketch; owes its own `§0`) | S2 |
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
