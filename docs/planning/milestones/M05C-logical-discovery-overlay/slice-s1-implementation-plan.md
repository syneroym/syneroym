# Slice S1 Implementation Plan — Tier 1: the App-DID Registry Record

**Status:** 📋 Planned (2026-08-04). Not started. Milestone:
[task.md](task.md) slice **S1**; milestone-level plan:
[implementation-plan.md](implementation-plan.md). Design of record:
[ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md) §2.
Depends on **M05A slice A7 (= S0) — Complete**. Gates S2.

**The one-sentence summary.** S1 makes "which supervisor holds this app" an
answerable question, by having the supervisor publish one registry record under
the app master DID A7 minted — and it adds the manifest surface for
`ShardingStrategy` that S5 will eventually read.

**Read before writing the publisher:**
[A7's plan](../M05A-app-supervisor/slice-a7-implementation-plan.md) §0.5. A7
made `adopt` self-correcting about the `import-master`-before-`adopt` ordering
rule; S1 is where getting it wrong first has a consequence outside this node —
two app DIDs means two records, which no generation comparison can reconcile.

**Read the milestone plan's §0 and §1 first.** They carry the two contradictions
found inside ADR-0022 (§0.1, the missing `generation` field) and the
locked-vault decay (§0.2), both of which bind this slice.

---

## §0 — What ADR-0022 §2, the meta plan's slice table, and the shipped tree leave open, understate, or state wrongly

### 0.1 (Scope-changing) `EndpointPublisher` is the wrong vehicle, and the reason is that the key is in the wrong process

The obvious place to publish an endpoint record is
[`EndpointPublisher`](../../../../crates/core/src/endpoint_publisher.rs) —
`publish_service`, `publish_all_services`, and the heartbeat refresh already do
exactly this shape of work.

It cannot do this one. `EndpointPublisher` is **substrate-side**: it walks
`hosted_apps_dir` and signs each record with the key the substrate derives for
that service. The Tier-1 record is signed with the **app master**, which lives
in the *supervisor's* vault and, by ADR-0020 §3, must never be on the substrate.

So S1's publisher is supervisor-side, and its shape already exists: A5d's
`refresh_master_anchor` is a supervisor-driven, key-holding, registry-writing
call on the same `RegistryClient`. S1 adds a sibling to it, not a branch inside
`EndpointPublisher`.

This is a scope finding, not a detail: "reuse the existing publisher" is a
plausible half-day and the real work is a new call path with its own tests.

### 0.2 (Correctness) A paused app instance silently stops refreshing, and the app disappears 30 days later

The resident loop's work list is `all_active`, which **excludes a paused
instance entirely** — no health poll, no renewal, no anchor refresh. That is
already a known, documented cost
([deferred-backlog.md](../../deferred-backlog.md) §8, *a paused instance gets no
certificate renewal or anchor refresh*), and A5d's 4-hour certificate default
turned it into a real clock.

S1 hangs a third thing off that same tick, with a much longer and much quieter
fuse. `EndpointInfo.not_after` defaults to **30 days**
(`DEFAULT_ENDPOINT_NOT_AFTER_SECS`). So `pause` — documented as stopping
reconciliation "and nothing else" — now also means "and this app stops being
discoverable to the outside world in a month, with no event at the moment you
paused it".

`pause` exists so an operator can stop the supervisor fighting them while they
work on something. Making it silently expire the app's public identity is not
what it promises.

**Two candidate resolutions, and this slice must pick one rather than inherit
the existing behavior by default:**

- **Refresh Tier 1 even while paused.** Correct in intent, but A5c
  deliberately made a paused instance get *zero* processing and has two tests
  asserting exactly that. Reopening it means splitting the write phase into
  maintenance and reconcile halves — the change the backlog row above already
  considered and declined.
- **Keep the skip, and make the consequence visible.** `pause` warns at the
  moment it is called, naming the expiry date, and `status` shows it. No
  structural change, and the operator learns the cost when they choose it.

Recommended: the second (D-S1-4). It costs one warning and one field, it does
not reverse a tested design decision, and it converts a silent 30-day fuse into
an informed choice. Recorded, with the first option's cost, so a future slice
that *does* split the write phase knows this was considered.

### 0.3 (Ambiguous) `EndpointInfo` is shaped for "how to reach this service", and Tier 1 answers "who supervises this app"

Reusing the struct means three fields have no natural value:

- **`endpoint_type` / `mechanisms`** describe how to open a connection. Nobody
  connects to an app DID — the record is a pointer to the supervising node,
  whose own record carries the real mechanisms.
- **`nickname` / `is_private`** are endpoint-record metadata that
  [deferred-backlog.md](../../deferred-backlog.md) §8 already flags as
  travelling in a signed record that no longer needs them.

The alternative — a dedicated record type — is explicitly rejected by ADR-0022
§2's reasoning about admission rules ("a second admission rule in a component
that has exactly one"), and that reasoning is right.

So: reuse the struct, and **decide what the unnatural fields mean rather than
leaving each publisher to guess** (D-S1-3). The one thing that must not happen
is two publishers filling them differently, since the fields are inside the
signed payload and a reader has no way to know which convention it is looking
at.

### 0.4 (Understated) The refresh cadence is not free to choose — it is bounded on both sides

`not_after` is 30 days. A5d's anchor refresh interval is 12 hours. A refresh
that is too rare risks expiry after a run of failures; too frequent and it is a
signature plus a network write on a key-holding process, repeated per app
instance.

The ratio that matters is **how many consecutive refresh failures the record
survives**. At 12 hours against 30 days, sixty. That is the number to state and
test against, rather than the interval, and it is the number that makes the
locked-vault window in the milestone plan's §0.2 concrete: an operator has about
a month, not "some time".

Reuse `master_anchor_refresh_interval_secs`' cadence and its persisted-fact
pattern — the `master_anchor_refresh` table is `(master_did, last_refreshed_at)`
and needs no timer of its own, evaluated on the ordinary pass tick.

### 0.5 (Scope-changing) `ShardingStrategy` does not exist as a type, and S1 is defining a vocabulary S5 must live with

The absorbed backlog row asks for "a manifest surface for `ShardingStrategy`".
There is no such type in
[models.rs](../../../../crates/app_orchestration/src/models.rs) — `TopologyMode`
has a `Sharded` variant and nothing else.

So S1 is not adding a field to an existing type; it is naming the strategies. And
nothing will read them until S5, in M7 (milestone D-C-4), so the vocabulary will
be a year old before its first consumer exercises it.

**Consequence: keep it minimal.** One strategy that matches what the resolver
already does — rendezvous hashing over the member set, which
[resolver.rs](../../../../crates/app_orchestration/src/resolver.rs) implements
today for `Redundant`'s keyed calls — plus the enum's own extensibility. Naming
range-sharding, consistent-hashing variants, or a custom-function escape hatch
now would be designing against an imagined consumer.

### 0.6 (Correctness) The record must be published under the app DID, and A7 stored that DID in exactly one place

A7's D-A7-4 made the decision that matters here: the app master DID is
**stored** on the instance row, not derived on read, because nothing can
enumerate vault keys and the vault is locked after every restart. The row is the
only index and the only copy readable while the vault is shut.

So the publisher reads the DID from `desired_state.app_master_did` and opens the
vault only to sign. An instance adopted before A7 has an empty column
(D-A7-7: it gains its master at its next `adopt`, and nowhere else) — for S1
that means **no Tier-1 record and no error**, since there is no identity to
publish under. It must be a visible skip, not a silent one, and it must not
mint: minting here would create an app identity outside `adopt`, which is the
one place A7 put it.

---

## §1 — S1 decisions

| ID | Decision |
|---|---|
| **D-S1-1** | **The publisher is supervisor-side, a sibling of `refresh_master_anchor`, not a branch in `EndpointPublisher`** (§0.1). The app master must not reach the substrate (ADR-0020 §3), and `EndpointPublisher` runs there. |
| **D-S1-2** | **`EndpointInfo` gains `generation: u64`, `#[serde(default)]`** (milestone D-C-1). Every existing publisher passes `0`. Reader-compared, not admission-enforced — the meta plan's "generation-fenced" is corrected in the milestone plan §0.1. |
| **D-S1-3** | **One stated convention for the fields that do not fit** (§0.3): the record's `endpoint_type`/`mechanisms` describe the *supervising node*, `nickname` is the app instance's human name, `is_private` follows the app's own visibility declaration. Written once, in a doc comment on the constructor, because the fields are inside the signed payload and a second convention would be unreadable. |
| **D-S1-4** | **A paused instance keeps its current skip, and `pause` warns with the expiry date** (§0.2). `status` shows the Tier-1 record's expiry. The alternative — splitting the write phase into maintenance and reconcile halves — is recorded with its cost and not taken, because it reverses two A5c tests that assert a paused instance gets zero processing. |
| **D-S1-5** | **Refresh reuses `master_anchor_refresh`'s cadence and its persisted-fact pattern** (§0.4): a `(app_did, last_refreshed_at)` row evaluated on the ordinary pass tick, no second timer. The tested property is the **failure tolerance** — sixty consecutive failed refreshes before expiry — not the interval. |
| **D-S1-6** | **An instance with no app master DID on its row is skipped visibly and never mints** (§0.6). Minting outside `adopt` would create an app identity in a second place, which A7 deliberately has exactly one of. |
| **D-S1-7** | **`ShardingStrategy` names one strategy — rendezvous hashing over the member set** — matching what the resolver already implements (§0.5). No range sharding, no custom escape hatch, until S5 has a real consumer. |
| **D-S1-8** | **No HTTP registry configured is a warning, not a failure** (milestone D-C-5). Single-node deployments do not use cross-app discovery and must not break to enable it. |
| **D-S1-9** | **No anchor for the app master** (milestone D-C-3). It delegates nothing; the record is self-signed under the DID that *is* the key. A7's deferred question is answered, and its backlog row is reworded to the real limitation rather than closed. |

---

## §2 — Phase plan and merge order

**Gated on [M05B slice B1 phase 1](../M05B-async-primitives/slice-b1-implementation-plan.md)
landing first** — the milestone plan §2 explains why: it is a pure refactor of
the same 8,852-line file both streams edit, and it is one small merge either way
round.

Four phases. Nothing is observable until phase 3.

1. **The wire field and the vocabulary.** `EndpointInfo.generation` (D-S1-2)
   with `#[serde(default)]`, every existing construction site passing `0`; and
   `ShardingStrategy` in `models.rs` with its manifest surface (D-S1-7), read by
   nothing, carrying the comment that says what will read it and when (milestone
   D-C-4). Tests 1–4.
2. **The store side.** The `(app_did, last_refreshed_at)` fact, in
   `master_anchor_refresh`'s exact shape (D-S1-5). Tests 5–6.
3. **The publisher and the refresh.** The supervisor-side publish/refresh call
   (D-S1-1), the field convention (D-S1-3), the skip for an instance with no app
   master (D-S1-6), the no-registry warning (D-S1-8), and its evaluation on the
   ordinary pass tick. Tests 7–14.
4. **Operator visibility, and the e2e.** `pause`'s expiry warning and the
   `status` expiry field (D-S1-4); the `VaultLocked` alert's new consequence
   (milestone D-C-2); the two-substrate e2e; and the docs. Tests 15–18.

**What could move:**

- **Phases 1 and 2 can merge together** — a wire field nothing sets and a table
  nothing writes.
- **Phase 4 cannot be dropped.** Without it S1 ships the silent 30-day fuse that
  §0.2 exists to prevent, and every property in phases 1–3 would still pass its
  tests.
- **Phase 3's no-registry path needs a test more than it needs code.** One
  `if let Some(client)` is trivial; the regression it prevents — a supervisor
  that refuses to start on a single-node deployment — is not.

---

## §3 — S1 tests

**e2e cases are marked; everything else is a unit test.** Numbering is
per-milestone and restarts at 1.

**Phase 1:**

1. `an_endpoint_record_written_before_generation_existed_reads_as_zero` —
   D-S1-2's `#[serde(default)]`, against a stored JSON fixture, not a
   round trip
2. `a_generation_survives_the_signature_round_trip` — the field is inside the
   signed payload; a record whose generation is altered must fail `verify`
3. `a_sharding_strategy_round_trips_through_a_manifest`
4. `a_manifest_with_no_sharding_strategy_parses_as_it_does_today` — the
   absent-means-current-behavior property

**Phase 2:**

5. `a_refresh_fact_is_recorded_per_app_did`
6. `an_app_did_with_no_refresh_fact_is_due_immediately`

**Phase 3:**

7. `the_record_is_signed_with_the_app_master_and_verifies_against_the_app_did`
8. `the_record_names_the_supervising_node_in_substrate_id` — §0.3's convention,
   pinned so a second publisher cannot diverge
9. `an_instance_with_no_app_master_did_is_skipped_and_nothing_is_minted` —
   D-S1-6, both halves
10. `a_locked_vault_fails_the_refresh_without_touching_the_registry` — the
    failure is loud and local; the milestone plan's §0.2 hazard is that it is
    quiet, and this pins where it is not
11. `no_configured_registry_warns_and_the_supervisor_keeps_running` — D-S1-8
12. `sixty_consecutive_failed_refreshes_are_survivable_and_the_sixty_first_is_not`
    — D-S1-5's real property, driven against a fake clock, not a real one
13. `a_refresh_runs_on_the_ordinary_pass_tick_and_starts_no_second_timer`
14. `a_paused_instance_gets_no_refresh` — asserting the *chosen* behavior
    (D-S1-4), companion to A5c's two existing paused-instance tests

**Phase 4:**

15. `pause_warns_with_the_records_expiry_date` — D-S1-4
16. `status_reports_the_tier_one_record_expiry`
17. **(e2e)** `an_app_did_resolves_to_its_supervising_node_through_the_registry`
    — [task.md](task.md)'s reference scenario steps 1–2 against two real
    substrates and a real registry
18. **(e2e)** `a_forged_tier_one_record_is_rejected_at_the_registry` —
    failure-matrix row 1, in the shape
    `master_endpoint_record_e2e.rs`'s hand-forged-record case already uses

---

## §4 — What closing S1 closes

- **Tier 1 of ADR-0022**, and with it S2's gate.
- **A7's deferred anchor question** (D-S1-9, milestone D-C-3).
- The *`TopologyMode::Sharded` has no expressible sharding strategy* backlog
  row — **surface only**; the row is updated, not closed, because `Sharded` is
  still compiled by nothing until S5.

What it does **not** close: the topology document and the `resolve` RPC (S2);
the locked-vault KEK problem, which S1 makes visible and more costly without
fixing; or app-master rotation.
