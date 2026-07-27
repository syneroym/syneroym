# M05A App Supervisor — Status

**Milestone:** [task.md](task.md) · **Design of record:**
[ADR-0020](../../../decisions/0020-stable-logical-service-identity.md),
[ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md)

**Overall:** Not started. Design accepted 2026-07-27; no code written.

## Slice status

| Slice | Scope | Status | Gate |
|---|---|---|---|
| P0 | `ControllerAgreement` creation tool — **pulled forward from M5 item 5** | Not started | None; gates A3 |
| A0 | Stable member identity (master DID per member + delegated instance keys + ingress `scope` enforcement) | Not started | None — independently mergeable |
| A1 | Endpoint records published under the member master DID | Not started | A0 |
| A2 | Host-side dependency resolution; bindings carry `expected_asserter_did` | Not started | A1 |
| A3 | Multi-substrate placement + substrate inventory | Not started | `ControllerAgreement` tool (see below) |
| A4 | Health declaration + read-only monitoring | Not started | A3 |
| A5 | Supervisor loop, best-effort delivery, operator read surface | Not started | A0–A4 |
| A6 | Durable delivery via outbox/DLQ | **Deferred, post-M5** | M5 item 1 Complete |

**A1 was added after design review** (2026-07-27) on finding that the
`master DID → endpoint` mapping ADR-0021 leaned on does not exist: the registry
verifies a record against the key resolved from the `service_id` it is keyed
under, so an instance key cannot publish under its master, and
`MasterAnchorPayload` is a revocation list with no forward index. Without A1,
relocation silently stops resolving — the exact failure A0 exists to prevent.

## Dependencies pulled in

1. **`ControllerAgreement` creation tool + the two items B7 pairs with it**, all
   three as **Slice P0** rather than left in M5. Decided 2026-07-27; see P0 in
   `task.md` for the reasoning and for why the three cannot be separated.

   Why it moved: M5 item 5 had no scheduled position at all — the 2026-07-16
   resequencing front-loads item 1 and defers items 2-4, never mentioning item 5
   — so A3 would have been gated on something with no date. And the exposure is
   concrete rather than theoretical: an unowned substrate issues
   `orchestrator/deploy` to every verified caller, and this milestone is what
   turns that contained bootstrap posture into a fleet of unattended, networked
   deploy targets.

## Decisions carried in from design (2026-07-27)

- Push, not pull: no service-facing directory; the trigger for revisiting is a
  measured convergence budget, recorded in ADR-0021 §6 and in this milestone's
  exit criteria. An operator-facing read surface does exist and is required.
- One master DID per **member**, not per logical service — otherwise a redundant
  service's member list collapses to a repeated DID and round-robin and sharding
  have nothing distinct to select over.
- The generation stamp is minted by an operator `adopt` action, never
  self-incremented; it is a tiebreaker among authorized writers, not an
  authorization mechanism.
- Substrate role, not a WASM `SynApp` — deviates from the pre-2026-07-27 text in
  `system-architecture.md` §LFC-MGT, which has been corrected.
- Master keys are per member, and an operator picks one of two postures
  (ADR-0020 §3), because certificate *renewal* needs the same key relocation
  does — attended mode reschedules the online key rather than avoiding it:
  **online-key** (supervisor holds member masters, short-lived certificates,
  issues and renews unattended) or **attended** (long-lived certificates where
  revocation is the control, operator issues on a cadence, and a missed renewal
  is an outage rather than a degradation).
- Remediation is restart-in-place only until M7 replication lands.
- The registry-trust-model ADR that M04A B7 recorded as owed (§6.2 / F9 option
  2) is **discharged** by ADR-0020 §6 plus slice A1, not scheduled separately —
  it is the same change to `verify()`'s contract, reached from the opposite
  direction, and B7's "needs a real consumer" gate is met by A1.

## Superseded work

The *Interstitial: Live App-Context Registry* placeholder
([meta-implementation-plan.md](../../meta-implementation-plan.md), reserved
2026-07-24) is superseded by this milestone. Both of its goals are carried
forward in A2: logical-name resolution backed by real deployment state, and
`expected_asserter_did` publication (M04B Slice B3's D-B3-8 residual).
