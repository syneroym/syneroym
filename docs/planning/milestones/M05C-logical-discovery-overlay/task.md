# Milestone 5C: Logical Service Discovery Overlay (M05C-logical-discovery-overlay)

> **Provenance.** The build-out of the *Committed Work: Logical Service
> Discovery Overlay* section in
> [meta-implementation-plan.md](../../meta-implementation-plan.md) (2026-08-02),
> promoted to a milestone directory on 2026-08-04. It is **Milestone 5 item 2**
> work by the same lineage as [M05A](../M05A-app-supervisor/task.md): item 2
> carried the "Design TBD" flag that ADR-0022 discharges. Item 2 therefore has
> three halves — the supervisor (M05A), this overlay (M05C), and the
> federated-query orchestrator (deferred to the final phase, untouched here).
>
> **Design of record:**
> [ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md).
>
> **What this milestone is.** Today a caller *inside* an app instance reaches a
> logical service because the App Supervisor pushed a binding into its config
> (ADR-0021). A caller *outside* the app has no way to find it at all — there is
> no `name → member set` answer anywhere in the tree. This milestone adds the
> two-tier path: Tier 1 maps an app DID to its supervisor through the existing
> registry, Tier 2 is a signed topology document naming member master DIDs, and
> Tier 3 (member DID → address) is the lookup that already exists.
>
> **What it is deliberately not.** Not the live directory ADR-0021 §6 rejected —
> what is fetched is a signed, cacheable document, not a hot-path query, and
> ADR-0022 §11 argues the distinction. Not a name registry: there is no
> `name → DID` record type, deliberately, because a name is not a key and
> admitting one would need a name-allocation surface. Not shard rebalancing —
> that is S5, gated on M7.

## Goal

By the end of M05C, a caller outside an app instance resolves that app's logical
service to its current member set, verifies the answer against the app's own
DID without trusting whoever relayed it, caches it, and routes to a member —
with the supervisor off the availability path for every resolution after the
first fetch.

---

## Requirement IDs (Traceability)

| Requirement ID | Sub-scope in M05C | Current matrix status |
|---|---|---|
| `[PLT-DAP-01]` (app-context registry / physical-sharding transparency) | Logical-name → member-set resolution for callers **outside** the app instance; sharding strategy declared in the manifest | Extends the row M05A carries. M05A closed the intra-app half by push; this is the cross-app half |
| `[TOP-DSC]` Discovery Mechanisms | The Tier 1 → Tier 2 → Tier 3 chain as one resolution path | Existing row, **Complete** for Tier 3 only. Needs re-scoping, not reopening |
| `[TOP-ADR]` Service Addressing | The gateway hostname scheme (`-a…-s…-i…`) and the routing-key header | Existing row, **Complete** at the M1 logical-ref level; S3 changes the external form |

---

## Explicit non-goals

- **No `name → DID` registry record.** ADR-0022 §2. A name is not a key, so such
  a record could not be self-signed, and admitting one would mean both a second
  admission rule in a component that has exactly one and a name-allocation
  surface.
- **No resolved physical address from Tier 2.** ADR-0022 §4/§9: the document
  names member master DIDs. Answering with a location would pull the supervisor
  into the failover path and make it the load balancer for the whole app.
- **No filtered member list.** ADR-0022 §5: a caller receives the full member set
  and mode, or a clean denial. Handing back 3 of 8 shard members does not
  restrict a caller, it corrupts it — rendezvous hashing over a partial set
  returns a confident wrong answer with no error.
- **No shard rebalancing and no epoch enforcement on the data path.** That is
  **S5**, gated on M7's `[PLT-RED]`. S2 ships the `epoch` field unenforced,
  because adding a field to a wire format is free before anything depends on it.
- **No app-master revocation story.** Deliberately unresolved; a self-signed
  Tier-1 record makes the DID and the key the same thing. See
  [implementation-plan.md](implementation-plan.md) §0.3.
- **No change to the intra-app push path.** ADR-0021 stays the mechanism inside
  an app instance. This overlay serves callers outside it.

---

## Dependency gates

| Gate | State |
|---|---|
| **M05A slice A7 (= S0) Complete** | ✅ 2026-08-04. The app-instance master DID exists, lives in the supervisor vault, and moves through `export-master`/`import-master`. **S1's only real gate — cleared** |
| ADR-0022 accepted | ✅ 2026-08-02 |
| M05B (async primitives) | **Not a gate in either direction.** The two streams are design-independent. They are *not* file-independent — see [implementation-plan.md](implementation-plan.md) §2's cross-stream merge order |
| M6 | Not a gate. **But S3 changes the gateway hostname format that M6's shell builds against** — the coupling the meta plan already flags |
| M7 `[PLT-RED]` | Gates **S5 only**, which is executed in M7, not here |

---

## Slices

S0 landed as M05A slice A7. S5 is M7 work and is listed for completeness only.

| # | Scope | Gate |
|---|---|---|
| **S0** | App-instance master DID: minted at `adopt`, held in the supervisor vault, surfaced on `status`, exportable for handover | **Landed as M05A slice A7, Complete 2026-08-04** |
| **S1** | Tier 1: the app-DID registry record, published and refreshed by the supervisor, carrying a generation a reader can compare. Manifest surface for `ShardingStrategy` | **Complete 2026-08-08** |
| **S2** | Tier 2: the signed topology document, the supervisor `resolve` RPC, and the client-side verify/cache path feeding `LogicalResolver::register`. Ships `epoch` unenforced | **Complete 2026-08-08** |
| **S3** | Gateway hostname scheme (`-a…-s…-i…`) plus the routing-key request header; coordinator relay of the document in the WebRTC bootstrap page | S2 |
| **S4** | Cross-app `Bind`: manifest surface, UCAN-scoped per-service exposure declared in the submitted plan, and replacing `prepare_binding`'s intra-app refusal with an authorization check | S2 **and** a first real cross-app dependency exists |
| **S5** | Shard rebalancing, and enforcing the epoch fence on the data path | **M7** `[PLT-RED]`. Executed there, not here |

**S1 inherits an ordering constraint from A7, not just a key.** `import-master`
must run before `adopt` on a handover, or `adopt` mints a *second* app identity
that the generation comparison cannot catch — two DIDs, not one record with two
writers. A7 documents the order and makes `adopt` self-correcting. S1 is where
publishing under the wrong identity first has an external consequence, so
whoever picks it up reads
[A7's plan](../M05A-app-supervisor/slice-a7-implementation-plan.md) §0.5 before
writing the publisher.

---

## Migration impact

- **One new field on a signed wire record.** S1 adds `generation` to
  `EndpointInfo`, which is inside the signed payload. `#[serde(default)]` so
  records written before it deserialize as `0`, which is also the correct
  reading ("no generation claimed"). See
  [implementation-plan.md](implementation-plan.md) §0.1 — ADR-0022 says the
  record is reused "unchanged", and it cannot be.
- **A manifest addition.** S1's `ShardingStrategy` surface; S4's cross-app
  `Bind` naming. Both absent-means-today's-behavior.
- **A new supervisor-side fact.** The Tier-1 record's last-refreshed timestamp,
  in the shape A5d's `master_anchor_refresh` table already uses.
- **An external format change, S3.** The client gateway hostname gains app and
  service segments. Centralised in `core::util` (build) and
  `core::protocol_utils` (parse), so a client going through those helpers sees
  one change in one place. Anything formatting host strings by hand breaks.
- **Nothing is dropped or renamed**, and no existing resolution path changes:
  Tier 3 is untouched, and the intra-app push path is untouched.

---

## Reference scenario (runnable)

Two real substrates, one community registry, one supervisor — the shape
`master_endpoint_record_e2e.rs` and `app_instance_identity_e2e.rs` already
establish. Written against S1 + S2, the milestone's central proof:

1. Submit and adopt an app instance with a `replicas > 1` logical service placed
   across both substrates. Confirm the app master DID on `status`.
2. Assert the **Tier-1 record** resolves: looking up the app DID in the registry
   returns the supervising node, and the record verifies against the app DID
   with no other trust input.
3. Fetch the **Tier-2 document** through the supervisor's `resolve` RPC from a
   caller that is *not* part of the app instance. Assert it names the member
   master DIDs, the mode, the epoch, and `not_after`, and that it verifies
   against the app DID from step 2.
4. **Verify a relayed copy.** Hand the same document bytes to a second party
   that never contacted the supervisor. It must verify identically — this is
   what makes the document a document rather than an RPC answer.
5. Route: resolve one member DID to an address through Tier 3 and call it.
6. **Scale out.** The member set changes and the epoch increments. Assert the
   cached document is superseded and a re-resolve returns the new set.
7. **Take the supervisor down.** Assert an already-cached document still routes
   until `not_after` — the availability property ADR-0022 §3 chose the document
   form for.
8. Forge a document under a different key. Assert it is rejected.

---

## Failure and security matrix

| # | Case | Required behavior |
|---|---|---|
| 1 | A Tier-1 record is forged by a party without the app master key | Rejected at the registry, the same way a forged endpoint record already is |
| 2 | Two supervisors publish a Tier-1 record for one app DID | Last write wins in the registry, and a reader can tell which is current from `generation`. **Visibility, not prevention** — ADR-0022 §2 says so explicitly, and the meta plan's "generation-fenced" overstates it (see plan §0.1) |
| 3 | The supervisor's vault is locked after a restart | It cannot sign a refresh. The failure must be **loud and early**, not a silent decay to `not_after`. Plan §0.2 — this is S1's sharpest finding |
| 4 | The supervisor is down when a caller resolves | A caller with a cached document routes until `not_after`; a caller with none fails cleanly. Scenario step 7 |
| 5 | A document is relayed by an untrusted party | Verifies or does not, on its signature alone. Scenario step 4 |
| 6 | A document is past `not_after` | Fails. Not "stale but usable" — the one rule ADR-0022 §3 gives, with no error taxonomy |
| 7 | A caller not authorized for a logical service fetches its document | Clean denial. **Never** a filtered member list (§5, non-goals) |
| 8 | Two different apps share a human `app_instance_id` | Foreign documents must not collide in `LogicalResolver`. Plan §0.4 — the key is a human name today |
| 9 | A relocated member | Absorbed by Tier 3. The document does **not** change and no cache is invalidated (§4) |
| 10 | A caller resolves at epoch N while a rebalance moves to N+1 | S5's enforcement. S1–S4 ship the field and do not enforce it; a test pins that the field is carried and preserved |
| 11 | No HTTP registry is configured on the supervisor's node | Tier 1 cannot publish. Warn, do not fail the supervisor — plan §0.6 |

---

## Performance budgets

| Budget | Target | Why |
|---|---|---|
| Resolution after the first fetch | No network call | The document is cached in `LogicalResolver`, which already has the TTL and epoch machinery. If a resolve hits the network every time, the design has become the live directory ADR-0021 §6 rejected |
| Tier-1 lookup | Within the existing registry-lookup budget | It is the same lookup shape as any other DID; no new cost is acceptable |
| Document verification | Once per fetch, not once per resolve | Verify on register, not on read |
| Tier-1 refresh cost on the supervisor | One signature and one publish per interval, on the existing pass tick | No second timer — the shape A5d's anchor refresh already uses |

---

## Measurable exit criteria

1. The reference scenario passes end to end against two real substrates and a
   real registry, including steps 4 (relayed verify) and 7 (supervisor down).
2. Every failure/security matrix row has a named test.
3. Every performance budget has a measurement, the cache one asserted as
   "no network call", not as a timing.
4. `[PLT-DAP-01]`'s cross-app half is recorded Complete with S1–S3 evidence; S4
   is recorded separately, since its own gate is a real consumer appearing.
5. S3's hostname change goes through `core::util` / `core::protocol_utils` only,
   demonstrated by the diff.
6. `cargo +nightly fmt --all` clean.
7. `cargo clippy --workspace --all-targets --all-features` clean.
8. `cargo test --workspace` passes.
9. `mise run test:e2e` passes.
10. `wasm32-wasip2` test components rebuild against any changed WIT.
