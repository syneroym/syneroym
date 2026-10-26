# M08A: Open foundation

*Reader: developers who build this sub-milestone.*

**Status: planning (2026-10-26).** Progress is in [status.md](status.md). Milestone overview and order: [M08-overview.md](../M08-overview.md).

This sub-milestone makes Roym usable by any client, not only the Hub. It builds one change doc. Read it first: [roym-open-foundation](../../changes/roym-open-foundation/change.md) (criteria `OF-*`).

The goal of the order: rename and type every method once, before the rest of Milestone 8 adds fields to them. Then build the parts that later sub-milestones depend on: the batched call (for federation) and the event feed (for extensions and match notifications).

## 1. Shared parts

| # | Shared part | Used by | Built in |
| --- | --- | --- | --- |
| X1 | **Participant vocabulary** in `roym_core`: deal roles named by action (`quote_author`, `counterparty`, `giver`, `receiver`). `Role { Consumer, Provider }` is removed. | Every Roym service, the Hub, `roymctl`, all of M8 | A1 |
| X2 | **Typed method inputs and outputs**, and the method table each service exports for the check test. | OF-14, OF-15; every later method | A2 |
| X3 | **`call-all`** in `syneroym:invocation`, with the paused epoch and the native shim. | The federated query library (A4); the exchange's friend forwarding (M08F) | A3 |
| X4 | **Event writer API**: one call each service makes to record an event. | The event feed (A5); extension services (M08C); match events (M08F) | A5 |

## 2. Facts checked in the code for this plan

- The search loop lives in the client on purpose: M06C decision `D-C6-9`. A guest call traps after `dispatch_epoch_timeout_secs` (5 s default). The epoch advances on a wall-clock interval, so the time burns while the guest waits in a host call. Backlog rows "A guest dispatch cannot loop over more than one network call" and "The Hub and `roymctl` each own their own copy of the client-side search loop" (§11) track this. A3 and A4 close both.
- `Role { Consumer, Provider }` is at `crates/roym_core/src/transaction.rs:85`. "provider" appears in about 234 lines and "consumer" in about 200 lines across Roym `.rs` and `.ts` files.
- About 124 places in Roym services read `req.params.get(...)`. 11 files parse a typed struct. The backlog row "Roym's sibling boundary is untyped JSON" (§10) is related. A2 narrows it. It does not add a WIT contract between services.
- `conversation.changes` (changes after `after_seq`, for one conversation) exists. The Hub does not call it.
- `web` answers WebSocket upgrades with no-op handlers (`crates/roym_web/src/app.rs:295`).
- The gateway binds `0.0.0.0` (`crates/client_gateway/src/gateway.rs:184`). The `local` login has no proof step (`crates/auth/src/service.rs:343`). Both are backlog rows (§7).

## 3. Slices

### S0 — Decisions and spike (docs only, S)

- Spike: can the engine pause the epoch deadline for one host call, in Wasmtime's current version, without affecting other calls on the same store? Write the answer and the chosen mechanism into the change doc.
- Decide: where the event feed lives (`web` is the recommendation), the event kinds of v1, the catch-up window (open questions in the change doc).
- Decide: one search call with progress events, or keep the three-call form for progress.

### A1 — Participant rename (M)

- X1. Wire fields, enums, method names, card labels, Hub text, `roymctl` text, tests, fixtures.
- No behaviour change. The parity harness (`crates/roym_web/tests/dual_build_parity.rs`) must pass unchanged in shape.
- Criteria: OF-1, OF-2.

### A2 — Typed methods (L, may split per service)

- X2. One input struct and one output struct per method, with `deny_unknown_fields`. A wrong field gets `-32602` with the field name.
- Each service exports its method table. A test lists every method and fails on one with no typed input or output.
- Split if large: A2a profile and conversation, A2b catalog and transaction, A2c directory and web.
- Criteria: OF-14, OF-15.

### A3 — Batched outbound call (M)

- X3. `call-all` in the WIT, the host implementation with concurrent calls and a total limit, the paused epoch, the native shim with `join_all`.
- Parity test: the same scenario in both builds.
- Criteria: OF-3, OF-4, OF-5.

### A4 — Federated query library and one search call (M)

- New Roym library crate. Move merge, dedupe and verification out of `roym_directory`.
- One search method. The Hub and `roymctl` stop running their own loops. Remove `window.RoymDirectory`'s `ignoreConcurrency` hook if nothing needs it.
- Criteria: OF-6, OF-7.

### A5 — Event feed (M)

- X4, the event store with a bounded window, the WebSocket path in `web`, catch-up by cursor, internal subscription for services.
- The Hub shows new messages, cards and delivery states live.
- Criteria: OF-8 to OF-11.

### A6 — Remote clients (S)

- Proof step for `local` login. Default bind and spec agree. Close the two backlog rows.
- Criteria: OF-12, OF-13.

### A7 — Client guide and close-out (S)

- "Build a Roym client" in the developer guide, tested by following it with `curl` and `websocat` (or a small script) against a test node.
- Living docs updated. Change doc `status: done`.
- Criteria: OF-16.

## 4. Order and parallel work

```text
S0 ─▶ A1 ─▶ A2 ─▶ A4 ─▶ A7
       │     └──▶ A5 ──┘
       ├──▶ A3 ──▶ A4
       └──▶ A6 ─────────▶ A7
```

- A1 first: it touches almost every Roym file. Anything built beside it conflicts.
- A3 (substrate and engine) and A6 (auth and gateway) touch few Roym files. They can run beside A2.
- A4 needs A3. A5 needs A2's types for event payloads.

## 5. Not in this plan

- Publishing the API contract (OpenRPC or JSON Schema). A2 makes it possible. A backlog row tracks it.
- A client SDK package.
- Cross-version compatibility of records (G5).
