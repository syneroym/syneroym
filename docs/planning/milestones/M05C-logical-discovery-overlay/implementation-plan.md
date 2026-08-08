# M05C Implementation Plan — Logical Service Discovery Overlay (milestone level)

**Status:** 📋 Planned (2026-08-04). Not started. Milestone:
[task.md](task.md). Design of record:
[ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md)
(**Accepted** 2026-08-02). Depends on **M05A slice A7 (= S0) — Complete**.

**This document is milestone level only** — the split, cross-cutting findings
and decisions, the slice sequence, the cross-stream merge order, and the
docs/backlog impact. **Each slice's own findings, decisions, phases, and tests
live in that slice's file** —
[slice-s1-implementation-plan.md](slice-s1-implementation-plan.md) is the first.
The rule and its evidence are in
[M05B's plan](../M05B-async-primitives/implementation-plan.md) §2.

**The one-sentence summary.** A caller outside an app instance has no way to
find that app's services; this milestone gives it a two-tier answer it can
verify against the app's own DID, cache, and use while the supervisor is down.

---

## §0 — Cross-cutting findings

Findings that hold across the milestone, or that set a slice boundary. Findings
about one slice's own implementation live in that slice's file. **Two of these
are contradictions inside ADR-0022 itself**, found by reading it against the
tree rather than on its own terms.

### 0.1 (Correctness) ADR-0022 §2 says the registry record is reused "unchanged" and also that it "carries `generation`" — and `EndpointInfo` has no such field

The two sentences are three paragraphs apart:

> "This reuses `EndpointInfo` ... **unchanged**: `service_id` is the app DID,
> `substrate_id` is the supervising node."
>
> "**The record carries `generation`.**"

`EndpointInfo`
([dht_registry.rs:76](../../../../crates/core/src/dht_registry.rs#L76)) has
eight fields — `service_id`, `substrate_id`, `endpoint_type`, `mechanisms`,
`nickname`, `is_private`, `ttl`, `not_after` — and none is a generation. The
struct is inside the signed payload, so this is a wire change, not a local one.

**Resolved in favour of the second sentence**, because the generation is
load-bearing and "unchanged" is a claim about convenience: `EndpointInfo` gains
`generation: u64` with `#[serde(default)]`, so a record written before it reads
as `0` — which is also the correct reading, "no generation claimed". Every
existing publisher passes `0` and nothing else changes.

**And the meta plan's own slice table overstates what this buys.** It calls S1
"generation-fenced". ADR-0022 §2 is careful and the table is not: the generation
is compared **by a reader**, to tell which of two answers is current. Admission
stays last-writer-wins. The ADR says so directly — "This record does not create
the hazard, but it makes it visible in a new place". Prevention would need
registry compare-and-set, which
[ADR-0023 §6](../../../decisions/0023-durable-async-primitives.md) declines to
build for reasons that apply here too. So: **reader-visible, not enforced**, and
S1's plan says that where an implementer will see it.

### 0.2 (Correctness, and it is the milestone's sharpest finding) The supervisor's vault is locked after every restart, and here that decays discovery silently instead of failing loudly

Signing the Tier-1 record (S1) and every topology document (S2) needs the app
master key. That key lives in the supervisor's vault, and **the vault is locked
until a human runs `inject-kek` after every restart** — `KeyStore` is
memory-only and no config field or environment variable supplies a KEK
([deferred-backlog.md](../../deferred-backlog.md) §8, *The supervisor's vault is
locked until an operator injects the KEK*).

For A7 this cost one loud failure: `adopt` refuses with `VaultError::Locked`,
naming `inject-kek`, and the operator re-runs it. **Here the failure mode
inverts.** A rebooted, un-injected supervisor keeps serving `status`, keeps its
resident loop running, and simply stops refreshing the Tier-1 record. Nothing
fails. The record sits in the registry until `not_after` — default 30 days — and
then the app stops being discoverable to every outside caller at once, with no
event at the moment the cause occurred.

Silent decay over days is a worse failure than a refusal at a keystroke.

**Consequence, binding on S1 and S2:** the locked-vault condition must be
surfaced as a standing, visible state for as long as it holds, not as a
one-shot startup warning. A5d's `VaultLocked` alert already exists and is the
right vehicle; what S1 adds is that the alert now means something an operator
can act on with a deadline, and that the Tier-1 record's expiry is readable on
`status` so the deadline is visible. Decided in D-C-2.

### 0.3 (Ambiguous, and A7 deferred it here by name) Whether the app master needs an anchor is S1's decision, and the answer is no

A7's D-A7-10 shipped the app master with no anchor, no registry record, and no
signing, and said: "Whether it ever needs an anchor is S1's decision, recorded
as a backlog row." The row exists —
[deferred-backlog.md](../../deferred-backlog.md) §8, *The app master has no
anchor and no revocation story*.

**The answer is no, and the reason is structural.** A master anchor exists to
let a reader check whether a *delegated* key has been revoked. The app master
delegates nothing: the Tier-1 record is self-signed under the app DID, and the
app DID *is* that key. There is no third party whose authority could be
withdrawn. Revoking an app master is not a revocation operation at all — it is
minting a different app, which produces a different DID and therefore a
different record.

That is a real limitation and it stays open as a backlog row, reworded rather
than closed: an app master that leaks cannot be rotated without changing the
app's identity. It is the same property `did:key` has everywhere else in this
system.

### 0.4 (Correctness, sets S2's boundary) `LogicalResolver` is keyed by a human name, and S2 puts foreign apps into it

`AppRegistry::register` takes `(AppInstanceId, LogicalServiceName)`, and
`LogicalServiceRef` is exactly that pair
([models.rs:214](../../../../crates/app_orchestration/src/models.rs#L214)) —
two human names, no DID anywhere in the key.

Today every entry arrives from the substrate's own deploys through
`install_app_context`, so the names are ones the node's operator chose. S2
changes the source: a verified topology document for a *foreign* app is
registered into the same map, and the foreign app's `app_instance_id` is a
human name chosen by someone else. Two unrelated apps both called `chat` now
collide, and the loser is silently re-pointed at the winner's members.

The collision is latent today and becomes reachable-from-the-network at S2,
which is the escalation that matters. S2's `§0` settles the fix — the natural
one is keying foreign entries by app DID, since ADR-0022 §1 already establishes
that the DID is the identity and `app_instance_id` is only the human name — but
it must be settled *in* S2 and not discovered in review.

### 0.5 (Understated) S1 ships a manifest field that nothing reads, and that is correct but must be said

S1 absorbs the backlog row *`TopologyMode::Sharded` has no expressible sharding
strategy in a manifest*. But `Sharded` is compiled by nothing today — A5e's
`replicas` sets `TopologyMode::Redundant` unconditionally above 1
([deferred-backlog.md](../../deferred-backlog.md) §8) — and rebalancing is S5,
in M7.

So S1 adds a declaration with no consumer, deliberately, on the same reasoning
ADR-0022 §6 gives for shipping the `epoch` field before enforcing it: a manifest
field is free to add before anything depends on the format and expensive
afterwards. The backlog row is **updated, not closed** — the surface exists, the
behavior does not.

### 0.6 (Ambiguous) A supervisor with no HTTP registry configured cannot publish Tier 1, and must not fail for it

A1's D-A1-2 established that master-DID resolution requires a configured HTTP
registry; the DHT copy is best-effort backup. A node running a supervisor with
no registry configured is a valid deployment today — every single-node
deployment is one.

Publishing Tier 1 there is impossible and must be a **warning, not a failure**:
the app still works, its intra-app push path is unaffected, and only cross-app
discovery is unavailable. Failing the supervisor would break working
single-node deployments to enable a feature they do not use. Matrix row 11.

---

## §1 — Cross-cutting decisions

| ID | Decision |
|---|---|
| **D-C-1** | **`EndpointInfo` gains `generation: u64`, `#[serde(default)]`** (§0.1). ADR-0022 §2's "unchanged" is corrected; its "carries `generation`" stands. The generation is **compared by readers, not enforced at admission** — no registry compare-and-set, consistent with [ADR-0023](../../../decisions/0023-durable-async-primitives.md) §6. |
| **D-C-2** | **A locked vault is a standing visible state with a deadline, not a startup warning** (§0.2). The existing `VaultLocked` alert carries it, and the Tier-1 record's expiry is readable on `status` so an operator can see how long they have. The silent-decay-to-`not_after` path must not exist. |
| **D-C-3** | **No anchor for the app master, and the reason is that it delegates nothing** (§0.3). A7's deferred decision is answered here. The backlog row is reworded to state the real limitation — an app master cannot be rotated without changing the app's identity — rather than closed. |
| **D-C-4** | **Ship-before-enforce, twice**: S1's `ShardingStrategy` and S2's `epoch` are both declarations with no consumer until S5 (§0.5, ADR-0022 §6). Each carries a comment saying what will read it and when, so neither reads as dead code. |
| **D-C-5** | **No registry publish is a warning, never a supervisor failure** (§0.6). |
| **D-C-6** | **Foreign topology documents must not collide with local entries in `LogicalResolver`** (§0.4). S2 owns the fix and must decide it in its own `§0`; this decision fixes only that the collision is not acceptable. |
| **D-C-7** | **S4 is not started on schedule, only on a consumer.** Its gate is "S2 Complete **and** a first real cross-app dependency exists". Building a cross-app `Bind` with nothing to bind is how a surface gets designed against an imagined caller. |
| **D-C-8** | **S5 is executed in M7, not here**, and this milestone closes without it. Recorded so a reader does not treat M05C as incomplete for lacking rebalancing. |

---

## §2 — Slice sequence and cross-stream merge order

**Within this milestone:** S1 → S2 → S3, strictly sequential — each consumes
what the previous publishes. S4 branches off S2 and additionally waits on a real
consumer (D-C-7).

```
A7 (= S0, Complete) → S1 → S2 → S3
                            └→ S4
```

### Cross-stream merge order with M05B — the part neither doc had

The meta plan says M05C and M05B "have no dependency in either direction". That
is true of the **designs** and false of the **files**. Both streams change the
same four places:

| File | M05C | M05B |
|---|---|---|
| `app_supervisor/src/service.rs` (8,852 lines) | S1 publish/refresh; S2 `resolve` dispatch | B1 actor construction + worker spawn; B3 tick evaluation |
| `app_supervisor/src/store.rs` | S1 last-refreshed fact | B1 queue tables; B3 overlap row |
| `wit/supervisor/supervisor.wit` | S2 `resolve` verb | B1 `dead-letters` / `replay` verbs |
| `app_orchestration/src/models.rs` | S1 `ShardingStrategy`; S4 cross-app `Bind` | B3 schedule surface |

Run concurrently with no stated order, that is repeated rebasing on the largest
file in the crate, and nobody owns it.

**Decided with the requester, 2026-08-04: M05B runs first, then M05C.** Not "in
parallel with a merge convention" — a sequence removes the collision entirely
rather than managing it, and it costs nothing, because the meta plan already
puts M05B ahead on merit ("it unblocks A6 and M6, where the overlay unblocks
nothing yet").

Two things follow, and they are worth keeping even under a sequence:

- **M05B slice B1 phase 1 is the specific thing S1 builds on.** It is a pure
  refactor with no behavior change — routing six `as Arc<dyn SubstrateActor>`
  upcasts through one constructor — so by the time S1 opens `service.rs`, that
  file has one construction point instead of six. This is the concrete benefit
  of the ordering, not a general "less rebasing".
- **If the sequence is ever relaxed** — someone picks up S1 while a later M05B
  slice is still in flight — the convention is that a change to
  `supervisor.wit` or to `SupervisorStore`'s schema is announced in the other
  stream's status doc. Those two are where a silent conflict merges cleanly and
  breaks later.

**Slice plans:**

| Slice | Plan | State |
|---|---|---|
| S1 | [slice-s1-implementation-plan.md](slice-s1-implementation-plan.md) | ✅ Complete (2026-08-08) |
| S2 | [slice-s2-implementation-plan.md](slice-s2-implementation-plan.md) | ✅ Complete (2026-08-08) — settled §0.4 as D-S2-1 |
| S3 | *(not written)* | Sketch in [task.md](task.md); owes its own `§0` |
| S4 | *(not written)* | Sketch in [task.md](task.md); owes its own `§0`. Not started without a consumer (D-C-7) |

---

## §3 — Docs and backlog impact

**Created by promoting this work to a milestone (documentation, no code):**

- This directory: `task.md`, `implementation-plan.md`,
  `slice-s1-implementation-plan.md`, `status.md`.
- A pointer from `meta-implementation-plan.md`'s *Committed Work: Logical
  Service Discovery Overlay* section to this directory, in the shape it already
  uses to point at M05A. **The section keeps its narrative** — the build-order
  reasoning, the stream argument, and the M6 hostname coupling are all still the
  right things to say at meta-plan level.
- The cross-stream merge-order note (§2) added to both that section and
  [M05B task.md](../M05B-async-primitives/task.md).

**Backlog rows this milestone resolves** (at closeout):

| Row | How |
|---|---|
| §8 *The app master has no anchor and no revocation story* | **Half.** D-C-3 answers the anchor question (no, and why). The rotation limitation is reworded and stays open |
| §5/§8 *`TopologyMode::Sharded` has no expressible sharding strategy in a manifest* | **Surface only**, S1. Updated, not closed (§0.5) — `Sharded` is still compiled by nothing until S5 |
| §8 *Cross-app `Bind` dependency naming has no manifest surface* | S4, on its own gate |

**Backlog rows this milestone deliberately does not resolve:**

| Row | Why |
|---|---|
| §8 *The supervisor's vault is locked ... and the KEK does not survive a restart* | D-C-2 makes the consequence **visible**; it does not fix it. A restart-surviving KEK is a key-management design with its own threat model, inherited from M04A. The row gains one sentence noting that M05C raises its cost |
| §8 *Two supervisors can mint two different app masters for one instance* | S1 makes it externally consequential (§0.2's sibling hazard) and does not prevent it. A7's `import-master`-before-`adopt` ordering rule stays the mitigation |
| §8 *`replicas` places every member of a service on one substrate* | Placement, not discovery |

**New backlog rows this milestone's own choices create** (added at closeout):

- **An app master cannot be rotated without changing the app's identity**
  (D-C-3's reworded row).
- **`generation` on `EndpointInfo` is reader-compared, not admission-enforced**
  (D-C-1) — the prevention half needs registry compare-and-set, which
  ADR-0023 §6 declines.
- **Whatever S2 does not choose** for the foreign-entry collision (§0.4).

---

## §4 — What closing this closes

Against the meta plan's overlay section and ADR-0022:

| Overlay slice | Closed by | Note |
|---|---|---|
| S0 | M05A A7 | Already Complete |
| S1 | This milestone | Tier 1 plus the sharding-strategy surface |
| S2 | This milestone | Tier 2, the `resolve` RPC, the client cache path |
| S3 | This milestone | Gateway hostname and routing-key header |
| S4 | This milestone, **on a consumer** | Not on a schedule (D-C-7) |
| S5 | **M7** | Rebalancing and epoch enforcement |

And beyond the slice list:

- **`[PLT-DAP-01]`'s cross-app half.** M05A closed the intra-app half by push;
  a caller outside the app instance could not resolve anything until now.
- **ADR-0021 §6's own boundary is confirmed from the other side.** ADR-0022 §11
  argues this is not the live directory §6 rejected, and shipping it is where
  that argument is tested rather than asserted.
- **M7's `[PLT-RED]` is unblocked.** Replicating a service across three members
  is pointless while callers cannot discover the current member set — the reason
  this stream is not deferred with the rest of M5 item 2.

What it does **not** close: S5; the federated-query orchestrator (M5 item 2's
third half, still deferred); app-master rotation; and anything M05B owns.

---

## §5 — Questions for the requester

1. ~~**Does S1 wait for M05B's B1 phase 1, or start now?**~~ **Answered
   2026-08-04: M05B runs to completion first, then M05C** (§2). The file
   collision is removed rather than managed.
2. **How loud should a locked vault be?** D-C-2 makes it a standing alert with a
   visible expiry deadline. The stronger option — refuse to start the supervisor
   at all without a KEK once an app has a published Tier-1 record — would make
   the failure impossible to miss but would also turn a reboot into an outage
   for the intra-app path, which does not need the vault. I recommend the alert;
   worth confirming, since it is the difference between "degraded and visible"
   and "stopped".
