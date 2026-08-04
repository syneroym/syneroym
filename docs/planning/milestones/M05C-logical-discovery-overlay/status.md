# M05C Logical Service Discovery Overlay — Status

**Milestone:** [task.md](task.md) · **Design of record:**
[ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md)
(Accepted 2026-08-02) · **Plan:**
[implementation-plan.md](implementation-plan.md)

**Overall:** 📋 **Planned (2026-08-04). Not started.** Promoted from the
*Committed Work: Logical Service Discovery Overlay* section of
[meta-implementation-plan.md](../../meta-implementation-plan.md) into a
milestone directory, so the largest committed-but-unplanned work in the tree
carries the same discipline as everything else. **S1's gate is clear** — A7
(= S0) landed 2026-08-04.

**Plan layout.** [implementation-plan.md](implementation-plan.md) is milestone
level. Each slice's own findings, decisions, phases, and tests live in that
slice's file — [slice-s1-implementation-plan.md](slice-s1-implementation-plan.md)
is the first; S2–S4 get theirs when picked up. The reasoning is in
[M05B's plan](../M05B-async-primitives/implementation-plan.md) §2.

## Slice status

| Slice | Scope | Status | Gate |
|---|---|---|---|
| S0 | App-instance master DID | **Complete (2026-08-04)** as [M05A slice A7](../M05A-app-supervisor/slice-a7-implementation-plan.md) | — |
| S1 | Tier 1: the app-DID registry record; `ShardingStrategy` manifest surface | 📋 Planned — [plan](slice-s1-implementation-plan.md) | S0 **cleared**; sequenced after M05B B1 phase 1 (see below) |
| S2 | Tier 2: signed topology document, `resolve` RPC, client verify/cache | 📋 Planned (sketch; owes its own `§0`) | S1 |
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

## Evidence

_(Empty. Each slice's verification evidence lands here as it completes, in the
shape [M05A status.md](../M05A-app-supervisor/status.md) uses.)_
