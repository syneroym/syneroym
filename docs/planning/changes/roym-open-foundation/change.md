---
status: draft
living-docs-touched:
---
# Roym open foundation

*Reader: developers who plan and build Milestone 8, and the project owner.*

This change makes Roym a set of services that any client can use. The Hub (the Roym web UI) becomes one client among many. Another team can build a different app, a phone client or a bot on the same services, with no Hub code.

It is the first part of Milestone 8 (M08A). It comes first because it renames fields on the wire, and every later M8 change adds to those fields.

Terms used here:

- **Roym:** the SynApp for local commerce and chat. See [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md).
- **Hub:** the Roym web UI (`crates/roym_web/ui`).
- **Client:** any program that uses Roym's methods for a person: the Hub, `roymctl`, a phone app, a third-party app.
- **Participant:** a person or business with a Roym identity. It replaces "provider" and "consumer" (see the [Value Exchange](../roym-value-exchange/change.md) change).
- **Roym library:** a Rust crate that a Roym service, or any other Syneroym service, links in. It holds logic that more than one service needs. It builds for native and for `wasm32-wasip2`.
- **Event feed:** a stream from a participant's own node that says "something changed". Defined below.
- SynApp, SynSvc, SynOrg, substrate: see [TERMINOLOGY.md](../../../TERMINOLOGY.md).

## Problem

The integrated spec already says: "Every Roym capability the UI uses is a public JSON-RPC method. If the UI needs something the API cannot do, the API is wrong, not the UI." This is mostly true today. Five gaps remain:

1. **The client runs the directory search loop.** A search takes three calls: `directory.start-run`, then `directory.query-source` once for each directory, then `directory.merge`. The loop is in the Hub (`crates/roym_web/ui/src/directory/search.ts`, `runSearch`) and again in `roymctl`. A third client must write the loop a third time. An exchange that pulls from its friend exchanges (M8 federation) needs the same loop on the server side.
   This was done on purpose (M06C decision `D-C6-9`). A WASM guest call has a wall-clock limit (`dispatch_epoch_timeout_secs`, 5 s by default). The clock keeps running while the guest waits in a host call. So one guest call cannot wait on several slow directories.
2. **The words "provider" and "consumer" are on the wire.** `Role::Consumer` and `Role::Provider` (`crates/roym_core/src/transaction.rs:85`) and about 430 lines in 52 Roym files use them. M8 makes every participant able to give and receive. A published API with the old words would have to change later.
3. **A client cannot learn about changes without asking.** Messages between nodes are pushed. But a client on the person's own node is not told when a message, a card or a booking changes. The Hub fetches again only after the user clicks something. A new message is not shown while the screen is open. `conversation.changes` (a "changes since `after_seq`" call for one conversation) exists, but nothing tells the client when to call it.
4. **A client must run on the same machine as the node.** The Roym spec says the gateway binds `127.0.0.1`. The code binds `0.0.0.0` (`crates/client_gateway/src/gateway.rs:184`). The `local` login method has no proof step (`crates/auth/src/service.rs:343`). Both are backlog rows. The owner's decision: `0.0.0.0` is correct once a client must prove who it is.
5. **Method inputs are read field by field from loose JSON.** About 124 places in the Roym services read `req.params.get(...)`. Only 11 parse a typed struct. A machine-readable contract (a schema of every method) cannot be generated from loose JSON.

## Scope and non-goals

- In scope:
  - The participant rename: wire fields, enums, method names, card text, Hub text, docs.
  - A **batched outbound call** in the substrate, so that a guest can wait on many targets in one call without hitting the guest time limit.
  - A **federated query library** (a Roym library): fan-out, time limits, merge, and verification of results from many sources. It is used by the participant's search and later by the exchange.
  - One method for a whole search. The three-call form stays only if a client still needs live progress (see [Open questions](#open-questions)).
  - The **event feed**: one per participant node. WebSocket for clients outside the node. An internal call for services on the node.
  - **Remote clients:** a proof step for the `local` login, and the gateway bind decision written down in the spec and the code.
  - **Typed method inputs and outputs:** every Roym method reads its input into a struct and writes its output from a struct.
  - A **client guide** for developers: how to log in, call methods, read the event feed, render cards.
- Not in scope:
  - Publishing the API contract (OpenRPC or JSON Schema files). Typed structs make it possible at any time. It waits until the owner asks for it. A backlog row tracks it.
  - A TypeScript or other client SDK package. The Hub's own `rpc.ts` stays as it is.
  - Cross-version compatibility of records (G5 in the integrated spec). It stays deferred.
  - Browser access for a visitor who has no node (a public shop page). See the [shop](../roym-shop/change.md) change.

## Acceptance criteria

| ID | Criterion | Test |
| --- | --- | --- |
| **Participant rename** | | |
| OF-1 | No Roym wire field, enum value, method name or card label says "provider" or "consumer". A role in one deal is named by what the party does in it (for example `giver` and `receiver` of a commitment, or `quote_author`). | |
| OF-2 | The Hub and the user docs say "participant", or a deal role, everywhere. | |
| **Batched call and federated query** | | |
| OF-3 | A guest can send one call to many targets in a single host call. The host runs the calls at the same time, each with its own time limit, and returns one result per target in the input order. | |
| OF-4 | The guest's wall-clock limit does not count the time the guest waits inside the batched call. The batched call has its own total limit, set by the host. | |
| OF-5 | The same scenario gives the same result in the native build and in the WASM build. | |
| OF-6 | A search over three directories, one of them too slow, returns the results of the two others and names the slow one, in one client call. | |
| OF-7 | The federated query library merges, removes duplicates, and verifies every signed record. It is the only code in the workspace that does this. The Hub and `roymctl` hold no merge or verify logic. | |
| **Event feed** | | |
| OF-8 | A client opens the event feed with its login session. When a message, a card, a booking or a delivery state changes on the node, the client gets an event within 1 s. The event names what changed and a cursor. It carries no message content. | |
| OF-9 | A client that reconnects with its last cursor gets every event after that cursor, or is told the cursor is too old and it must reload. | |
| OF-10 | A service on the node can subscribe to the same events through an internal call, without WebSocket. | |
| OF-11 | The Hub shows a new incoming message on an open screen with no user action. | |
| **Remote clients** | | |
| OF-12 | The `local` login needs proof that the client holds the person's key. A request with no valid proof gets no session. | |
| OF-13 | The gateway binds the address set in the config. The spec and the default config say the same thing. With the proof step in place, a client on another machine can log in and use every Roym method. | |
| **Typed methods** | | |
| OF-14 | Every Roym method reads its input into a typed struct and returns a typed struct. A wrong field type gets `-32602` with the field name. | |
| OF-15 | A test lists every Roym method and fails when one has no typed input or output. | |
| **Client guide** | | |
| OF-16 | The developer guide has a "Build a Roym client" section. A developer can log in, list conversations, send a message, run a search and follow the event feed with only that section. | |

## Design

### Participant rename

- `Role { Consumer, Provider }` goes away. A deal names its parties by what they do in it: `quote_author` and `counterparty` for a quote, `giver` and `receiver` for one commitment. The [Value Exchange](../roym-value-exchange/change.md) change uses these words.
- The product is not released. Records change in place, with no migration. Every changed record keeps `version` 1, as earlier unreleased changes did.
- The integrated spec, the requirements and `TERMINOLOGY.md` are updated at close-out.

### Batched outbound call

Today a guest calls one target at a time through `syneroym:invocation`. The new call, roughly:

```wit
record call-target { target: call-target-ref, method: string, params: list<u8> }
record call-outcome { target-index: u32, result: result<list<u8>, call-error> }

/// Runs every call at the same time. Each call has `per-call-timeout-ms`.
/// Returns when all have answered or timed out, or when the host's total
/// limit ends. The guest's own wall-clock limit is paused during the wait.
call-all: func(calls: list<call-target>, per-call-timeout-ms: u32) -> list<call-outcome>;
```

The exact names are set in the slice plan. The rules:

- The host owns concurrency and the total limit. The guest cannot ask for more.
- **The wait does not burn the guest's time limit.** The engine pauses the epoch deadline for this one host call. This is the part that `D-C6-9` said was missing.
- Results come back in input order, so the guest needs no matching logic.
- The native shim (`syneroym-app-host-native`) implements the same call with `join_all`, so both builds behave the same (OF-5).

Rejected: wait for async guests (WASI 0.3). Its date is not known, and M8 federation needs the loop now. Rejected: a native search service outside the Roym SynApp. It would make the WASM build of Roym incomplete, which the integrated spec forbids (D2).

### Federated query library

A Roym library, `crates/roym_federated_query` (name set in the plan). It takes a list of sources, a query and limits. It calls `call-all`, verifies each signed record, merges, removes duplicates, and returns a page with one outcome per source (`ok`, `timed-out`, `refused`, `unreadable`). It moves today's merge and verify code out of `roym_directory` (the `directory.merge` path) into one place.

Two users:

1. **The participant's own search.** One method runs the whole search. The client sends the query and gets the merged page.
2. **The exchange, later** (the [intent matching](../roym-intent-matching/change.md) change). An exchange forwards a search to its friend exchanges with the same library.

### Event feed

One feed per node, for the people and services on it.

- **Events are small.** Each one names a kind (`message`, `card`, `delivery-state`, `booking`, `match`, …), an id and a cursor. Content is fetched with the normal methods. So the feed leaks nothing new, and needs no new access rules beyond "the caller is the owner".
- **Clients outside the node** connect with WebSocket through the client gateway. The platform supports guest WebSocket today (M06A). `web` does not use it yet (`crates/roym_web/src/app.rs:295`).
- **Services on the node** (for example a Roym extension service, see [Roym extensions](../roym-extensions/change.md)) register for event kinds through an internal call.
- **Catch-up.** Each event has a cursor. A client that was away asks for events after its last cursor. The node keeps a bounded window of events. Past the window, it answers "reload".
- Which service holds the feed is set in the plan. `web` is the likely home, because it already faces the client. Each service writes its events through an internal call.

### Remote clients

- `local` login gets a proof step: the gateway sends a challenge, and the client signs it with the person's key. The design follows the existing proof forms in the auth service. The plan decides the exact form.
- The default bind address stays `127.0.0.1` until the proof step ships. Then the config default and the spec say `0.0.0.0`, and the two backlog rows close.

### Typed methods

Each method gets an input struct and an output struct in `roym_core` (or in the service crate when only it uses them), with `serde` and `deny_unknown_fields`. This makes a schema possible later, with a crate such as `schemars`, without hand-written files. The publish step itself is deferred.

## Open questions

- **Live progress for search.** The Hub shows each directory's answer as it arrives. With one call, the client waits for all of them. Options: one call that streams through the event feed, or keep the three-call form for clients that want progress. Recommendation: one call, plus `search.progress` events in the feed.
- **Event window size.** How many events, or how long, the node keeps for catch-up. A first value to tune: 24 hours or 10,000 events.
- **Typed-method check.** OF-15 needs a list of all methods. `roym_core::router` lists method prefixes, not methods. The plan decides whether each service exports its method table.

## Tasks

- [ ] Write the M08A slice plans.
- [ ] Participant rename across Roym crates, the Hub, `roymctl`, docs.
- [ ] `call-all` in `syneroym:invocation`, the engine's paused epoch for it, and the native shim.
- [ ] The federated query library. Move merge and verify into it. One search method.
- [ ] The event feed: event writer, WebSocket path in `web`, internal subscription, catch-up.
- [ ] `local` login proof step. Gateway bind default, spec and code agree.
- [ ] Typed input and output for every Roym method, and the check test.
- [ ] "Build a Roym client" in the developer guide.

## Deviations

## Close-out

Before merge, update the living docs (requirements, architecture, integrated spec, developer guide, terminology) so they describe the result. Then set `status: done` and fill in `living-docs-touched`. Record anything you postponed in [deferred-backlog.md](../../deferred-backlog.md).
