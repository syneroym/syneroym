# M08A: Open foundation

*Reader: developers who build this sub-milestone.*

**Status: planning (2026-10-26).** Progress is in [status.md](status.md). Milestone overview and order: [M08-overview.md](../M08-overview.md).

This sub-milestone makes Roym usable by any client, not only the Hub. It builds one change doc. Read it first: [roym-open-foundation](../../changes/roym-open-foundation/change.md) (criteria `OF-*`).

The goal of the order: rename and type every method once, before the rest of Milestone 8 adds fields to them. Then build the parts that later sub-milestones depend on: the batched call (for federation) and the event feed (for extensions and match notifications).

## 1. Shared parts

| # | Shared part | Used by | Built in |
| --- | --- | --- | --- |
| X1 | **Participant vocabulary** in `roym_core`: deal roles named by action (`quote_author`, `counterparty`, `giver`, `receiver`). `Role { Consumer, Provider }` is removed. | Every Roym service, the Hub, `roymctl`, all of M8 | A1 |
| X2 | **Typed method inputs and outputs**, the unknown-field rule (refused on local-only methods, ignored on wire-reachable ones), and the method table each service exports for the check test. | OF-14, OF-15, OF-18; every later method | A2 |
| X3 | **`call-all`** in `syneroym:invocation`, with its target and concurrency limits, the epoch-deadline extension (or the two-step fallback), and the native shim. | The federated query library (A4); the exchange's friend forwarding (M08F) | A3 |
| X4 | **Event writer API**: one call each service makes to record an event. | The event feed (A5); extension services (M08C); match events (M08F) | A5 |

## 2. Facts checked in the code for this plan

- The search loop lives in the client on purpose: M06C decision `D-C6-9`. The epoch deadline is set once per guest call (`Store::set_epoch_deadline`, `crates/sandbox_wasm/src/engine.rs:570`), 5 s by default. The epoch advances on a wall-clock timer, so the time burns while the guest waits in a host call. Backlog rows "A guest dispatch cannot loop over more than one network call" and "The Hub and `roymctl` each own their own copy of the client-side search loop" (§11) track this. A3 and A4 close both.
- `Role { Consumer, Provider }` is at `crates/roym_core/src/transaction.rs:85`. "provider" appears in about 234 lines and "consumer" in about 200 lines across Roym `.rs` and `.ts` files.
- About 124 places in Roym services read `req.params.get(...)`. 11 files parse a typed struct. The backlog row "Roym's sibling boundary is untyped JSON" (§10) is related. A2 narrows it. It does not add a WIT contract between services.
- `conversation.changes` (changes after `after_seq`, for one conversation) exists. The Hub does not call it.
- `web` answers WebSocket upgrades with no-op handlers, and its guest instances keep no connection state (`crates/roym_web/src/app.rs:293`).
- The gateway binds `0.0.0.0` (`crates/client_gateway/src/gateway.rs:184`). `local` login reads the key from `person_identities_dir` on the node and has no proof step (`crates/auth/src/service.rs:343`). `delegated-key` login already does a challenge and proof (`service.rs:243`). The Hub CSP hardcodes `auth.localhost`. Backlog rows: §7 (bind address, `local` login) and §10 (CSP).

## 3. Slices

### S0 — Spike and decisions (docs only, M)

This slice decides the design that A3, A4 and A5 build. It is not optional.

- **Spike: the epoch-deadline callback.** Prove that the host can record the time a guest waits in `call-all` and extend the deadline by that much in Wasmtime's epoch-deadline callback, with the async engine and the host-binding style used today. Write a small test in `sandbox_wasm` that shows a guest waiting longer than its deadline inside one host call and not trapping, while a guest that loops in its own code still traps.
- **If the spike fails, choose the fallback** described in the change doc (two-step search: `start-search` returns a job id, the host runs the fan-out, a `search.done` event, a second short guest call merges). Record it as a deviation. A3 and A4 then build the fallback. It changes more than A3: OF-6 becomes three steps (`search.start`, the `search.done` event, `search.results`), so A5 (event feed) must land before A4. It also adds a job id and a second round trip to the participant-to-exchange search in M08F. The change doc lists these.
- **Decide the event feed's home:** a host-level event bus (recommended) or `web`. Compare them on the change doc's table: write cost per event, where connection state lives, and guest instance reuse.
- Decide the event kinds of v1, the catch-up window, and whether search keeps a progress form.

### A1 — Participant rename (M, merged alone)

- X1. Wire fields, enums, method names, card labels, Hub text, `roymctl` text, tests, fixtures.
- No behaviour change. The parity harness (`crates/roym_web/tests/dual_build_parity.rs`) must pass unchanged in shape.
- **A1 is merged before any other M08A slice opens a PR.** It touches almost every Roym file. Work that starts beside it will hit constant conflicts. S0 can run during A1, because it is docs and a spike in `sandbox_wasm`.
- Criteria: OF-1, OF-2.

### A2 — Typed methods (L, split per service)

- X2. One input struct and one output struct per method. A wrong field gets `-32602` with the field name. Local-only methods refuse unknown fields. Wire-reachable methods ignore them.
- Each service exports its method table. A test lists every method and fails on one with no typed input or output.
- Split: A2a profile and conversation, A2b catalog and transaction, A2c directory and web. Each is its own PR.
- Criteria: OF-14, OF-15, OF-18.

### A3 — Batched outbound call (M)

- X3. `call-all` in the WIT, the host implementation with a maximum target count, a concurrency cap per guest instance, per-call and total limits, the deadline extension proved in S0 (or the fallback), and the native shim.
- Parity test: the same scenario in both builds.
- Criteria: OF-3, OF-4, OF-5, OF-17.

### A4 — Federated query library and one search call (M)

- New crate `crates/roym_federated_query`, package `syneroym-roym-federated-query`. Move merging and duplicate removal out of `roym_directory`. The directory's client half keeps the source list and calls the library.
- One search method. The Hub and `roymctl` stop running their own loops. Remove `window.RoymDirectory`'s `ignoreConcurrency` hook if nothing needs it.
- Criteria: OF-6, OF-7.

### A5 — Event feed (M)

- X4, the event store with a bounded window, scoped by app instance (one app never sees another app's events), the WebSocket path, catch-up by cursor, internal subscription for services, in the home S0 chose.
- The Hub shows new messages, cards and delivery states live.
- Criteria: OF-8 to OF-11.

### A6 — Remote clients (S)

- Default bind `127.0.0.1`. A remote-access setting that names the bind address. `local` login checks the TCP socket peer address (never a forwarding header), refuses non-loopback peers with an error that names `delegated-key`, and is off completely when remote access is on. The Hub logs in with `delegated-key` when it is not on the node's machine. The Hub CSP allows the configured auth host.
- Docker: `deploy/docker-compose.community.yml` maps `7960:7960`, which cannot reach a gateway bound to `127.0.0.1` inside the container. Its config turns remote access on with `0.0.0.0` inside the container, and the host mapping becomes `127.0.0.1:7960:7960`. Update the `Dockerfile` comments and the developer guide's Docker section to match.
- Close the backlog rows for the bind address, the `local` login and the CSP.
- Criteria: OF-12, OF-13.

### A7 — Client guide and close-out (S)

- "Build a Roym client" in the developer guide, tested by following it with `curl` and `websocat` (or a small script) against a test node.
- Living docs updated. Change doc `status: done`.
- Criteria: OF-16.

## 4. Order and parallel work

```text
A1 (merged alone) ─▶ A2a ─▶ A2b ─▶ A2c ─▶ A4 ─▶ A7
S0 (during A1) ───────▶ A3 ──────────────┘     ▲
                        A5 (after A2a) ────────┤
                        A6 ────────────────────┘
```

- A1 first, and merged alone. No other M08A PR opens until it is on `main`.
- S0 runs during A1. Its result decides A3, A4 and A5.
- A3 (substrate and engine) and A6 (auth, gateway and the Hub's login) touch few Roym service files. They can run beside A2.
- A4 needs A3 and A2c (the directory's typed methods). A5 needs A2a's types for event payloads. If S0 chooses the fallback, A4 also needs A5.

## 5. Not in this plan

- Publishing the API contract (OpenRPC or JSON Schema). A2 makes it possible. A backlog row tracks it.
- A client SDK package.
- Cross-version compatibility of records and methods (G5), beyond the unknown-field rule above.
