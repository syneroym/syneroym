---
status: draft
living-docs-touched:
  - docs/TERMINOLOGY.md
  - docs/system-requirements-spec.md
  - docs/system-architecture.md
  - docs/roym-integrated-experience-spec.md
  - docs/developer-guide.md
  - docs/planning/traceability-matrix.md
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
- **Wire-reachable method:** a method that another node can call. Every other Roym method answers only local calls (`roym_core::admit::require_internal`).
- **Epoch deadline:** the wall-clock limit on one WASM guest call. The engine sets it with `Store::set_epoch_deadline` when the call starts (`crates/sandbox_wasm/src/engine.rs:570`). It is `dispatch_epoch_timeout_secs`, 5 s by default.
- **Event feed:** a stream from a participant's own node that says "something changed". Defined below.
- SynApp, SynSvc, SynOrg, substrate: see [TERMINOLOGY.md](../../../TERMINOLOGY.md).

## Problem

The integrated spec already says: "Every Roym capability the UI uses is a public JSON-RPC method. If the UI needs something the API cannot do, the API is wrong, not the UI." This is mostly true today. Five gaps remain:

1. **The client runs the directory search loop.** A search takes three calls: `directory.start-run`, then `directory.query-source` once for each directory, then `directory.merge`. The loop is in the Hub (`crates/roym_web/ui/src/directory/search.ts`, `runSearch`) and again in `roymctl`. A third client must write the loop a third time. An exchange that pulls from its friend exchanges (M8 federation) needs the same loop on the server side.
   This was done on purpose (M06C decision `D-C6-9`). The epoch deadline is set once, when a guest call starts. The epoch advances on a wall-clock timer, so the time keeps running while the guest waits in a host call. One guest call therefore cannot wait on several slow directories.
2. **The words "provider" and "consumer" are on the wire.** `Role::Consumer` and `Role::Provider` (`crates/roym_core/src/transaction.rs:85`) and about 430 lines in 52 Roym files use them. M8 makes every participant able to give and receive. A published API with the old words would have to change later.
3. **A client cannot learn about changes without asking.** Messages between nodes are pushed. But a client on the person's own node is not told when a message, a card or a booking changes. The Hub fetches again only after the user clicks something. A new message is not shown while the screen is open. `conversation.changes` (a "changes since `after_seq`" call for one conversation) exists, but nothing tells the client when to call it.
4. **A client must run on the same machine as the node.** The Roym spec says the gateway binds `127.0.0.1`. The code binds `0.0.0.0` (`crates/client_gateway/src/gateway.rs:184`). The `local` login (`crates/auth/src/service.rs:343`) reads the person's key from `person_identities_dir` on the node and has no proof step, so it is safe only for a client on the same machine. A client on another machine has no key file. The `delegated-key` login (`login_delegated_key`, `service.rs:243`) already does a challenge and proof. All three are backlog rows (§7).
5. **Method inputs are read field by field from loose JSON.** About 124 places in the Roym services read `req.params.get(...)`. Only 11 files parse a typed struct. A machine-readable contract (a schema of every method) cannot be generated from loose JSON.

## Scope and non-goals

- In scope:
  - The participant rename: wire fields, enums, method names, card text, Hub text, docs.
  - A **batched outbound call** in the substrate, so that a guest can wait on many targets in one call without hitting its epoch deadline. It has a cap on targets and on concurrency. A fallback design is part of this change, in case the deadline cannot be extended.
  - A **federated query library** (a Roym library): fan-out, time limits, and the merge and removal of duplicates over results from many sources. It is used by the participant's search and later by the exchange.
  - One method for a whole search. The three-call form stays only if a client still needs live progress (see [Open questions](#open-questions)).
  - The **event feed**: one per participant node. WebSocket for clients outside the node. An internal subscription for services on the node.
  - **Remote clients:** remote access is a setting that is off by default. When it is on, clients log in with `delegated-key`, and `local` login keeps answering only loopback requests.
  - **Typed method inputs and outputs:** every Roym method reads its input into a struct and writes its output from a struct.
  - A **client guide** for developers: how to log in, call methods, read the event feed, render cards.
- Not in scope:
  - Publishing the API contract (OpenRPC or JSON Schema files). Typed structs make it possible at any time. It waits until the owner asks for it. A backlog row tracks it.
  - A TypeScript or other client SDK package. The Hub's own `rpc.ts` stays as it is.
  - Cross-version compatibility of records and methods (G5 in the integrated spec). It stays deferred. See [Typed methods](#typed-methods) for the one rule this change adds.
  - Browser access for a visitor who has no node (a public shop page). See the [shop](../roym-shop/change.md) change.

## Acceptance criteria

| ID | Criterion | Test |
| --- | --- | --- |
| **Participant rename** | | |
| OF-1 | No Roym wire field, enum value, method name or card label says "provider" or "consumer". A role in one deal is named by what the party does in it (for example `giver` and `receiver` of a commitment, or `quote_author`). | |
| OF-2 | The Hub and the user docs say "participant", or a deal role, everywhere. | |
| **Batched call and federated query** | | |
| OF-3 | A guest can send one call to many targets in a single host call. The host runs the calls at the same time, each with its own time limit, and returns one result per target in the input order. | |
| OF-4 | The time the guest waits inside the batched call does not count against its epoch deadline. The batched call has its own total limit, set by the host. If the spike shows this cannot be done, the fallback in [Batched outbound call](#batched-outbound-call) replaces this criterion and changes OF-6, and the change doc records both as deviations. | |
| OF-5 | The same scenario gives the same result in the native build and in the WASM build. | |
| OF-6 | A search over three directories, one of them too slow, returns the results of the two others and names the slow one, in one client call. With the fallback: in one `search.start` call, one `search.done` event and one `search.results` call. | |
| OF-7 | Merging results from many sources and removing duplicates is done only by the federated query library. The Hub and `roymctl` hold no merge logic. Each service still verifies the records it receives, with the same `roym_core` verification functions as today. | |
| OF-17 | A batched call with more targets than the host maximum is refused before any call is made. The host runs at most a fixed number of outbound calls at once for one guest instance, and queues the rest. | |
| **Event feed** | | |
| OF-8 | A client opens the event feed with its login session. When a message, a card, a booking or a delivery state changes on the node, the client gets an event within 1 s. The event names what changed and a cursor. It carries no message content. | |
| OF-9 | A client that reconnects with its last cursor gets every event after that cursor, or is told the cursor is too old and it must reload. | |
| OF-10 | A service on the node can subscribe to the same events through an internal subscription, without WebSocket. A service receives only the events of its own app instance. A client receives only the events of the apps its login session may use. | |
| OF-11 | The Hub shows a new incoming message on an open screen with no user action. | |
| **Remote clients** | | |
| OF-12 | `local` login answers only a request whose TCP socket peer address is a loopback address. Headers such as `X-Forwarded-For` are never used for this check. When remote access is on, `local` login is turned off completely, because a proxy on the same machine (for example Caddy for TLS) makes every request arrive from loopback. A refused request gets an error that names `delegated-key` as the method to use. | |
| OF-13 | By default the gateway binds `127.0.0.1`, and the spec and the default config say so. A node owner turns on remote access with a config setting that names the bind address. With it on, a client on another machine logs in with `delegated-key` and can use every Roym method. The Hub works from that machine (its Content Security Policy no longer names only `auth.localhost`). The Docker deploy (`deploy/docker-compose.community.yml`, `Dockerfile`) still works after the default changes. | |
| **Typed methods** | | |
| OF-14 | Every Roym method reads its input into a typed struct and returns a typed struct. A wrong field type gets `-32602` with the field name. | |
| OF-15 | A test lists every Roym method and fails when one has no typed input or output. | |
| OF-18 | A local-only method refuses an unknown input field with `-32602`. A wire-reachable method ignores unknown input fields, so a node that has added a field can still call an older node. | |
| **Client guide** | | |
| OF-16 | The developer guide has a "Build a Roym client" section. A developer can log in, list conversations, send a message, run a search and follow the event feed with only that section. | |

## Design

### Participant rename

- `Role { Consumer, Provider }` goes away. A deal names its parties by what they do in it: `quote_author` and `counterparty` for a quote, `giver` and `receiver` for one commitment. The [Value Exchange](../roym-value-exchange/change.md) change uses these words.
- The product is not released. Records change in place, with no migration. Every changed record keeps `version` 1, as earlier unreleased changes did.
- The integrated spec, the requirements and `TERMINOLOGY.md` are updated at close-out.

### Batched outbound call

Today a guest calls one target at a time through `syneroym:proxy`. The new call, roughly:

```wit
record call-request { target: call-target, method: string, params: string }
record call-outcome { target-index: u32, result: result<string, proxy-error> }

/// Runs the calls at the same time, up to the host's concurrency cap.
/// Each call has `per-call-timeout-ms`. Returns when all have answered or
/// timed out, or when the host's total limit ends.
call-all: func(calls: list<call-request>, per-call-timeout-ms: u32) -> list<call-outcome>;
```

The exact names are set in the slice plan. The rules:

- **Limits.** The host refuses a list longer than a maximum (a first value: 16 targets). It runs at most a fixed number of calls at once for one guest instance (a first value: 8), and queues the rest. The per-call and total time limits are capped by the host. The guest cannot raise any of them.
- Results come back in input order, so the guest needs no matching logic.
- The native shim (`syneroym-app-host-native`) implements the same call with bounded concurrency, so both builds behave the same (OF-5).

**The epoch deadline is the main risk of this change.** The deadline is set once, at the start of the guest call (`engine.rs:570`). The epoch advances on a wall-clock timer, also while the guest waits in a host call. The deadline is checked only when guest code runs, so the guest traps at its first check after `call-all` returns.

- **Primary approach (proved in the M08A spike).** Proved by [crates/sandbox_wasm/tests/epoch_wait_credit.rs](../../../crates/sandbox_wasm/tests/epoch_wait_credit.rs) (D-OF-1). Wasmtime lets the host register an epoch-deadline callback on the store (`Store::epoch_deadline_callback`). The callback computes own time (wall-clock elapsed time minus credited wait time). If own time is at or past the budget, it returns `UpdateDeadline::Interrupt`. Otherwise it extends the deadline by returning `UpdateDeadline::Continue(ticks)` with the remaining budget rounded up to whole ticks (at least 1). Wait credit is capped at one `call-all` total limit per guest call. Credit and start time reset at every `set_epoch_deadline` (`engine.rs:570` for stores, `stream.rs:188` for stream chunks). Credit is also added when `call-all` ends with an error or a timeout. No other host call gives credit. Own time uses the wall clock (`Instant`) because `Engine::current_epoch` is `pub(crate)`. No `Config` change and no `Yield`. Returning budget minus own time (rather than the wait time) is required because `Continue(n)` in Wasmtime sets the new deadline to `current_epoch + n`, not `old_deadline + n`.
- **Fallback (rejected alternative): a two-step search with no long host call.** The guest calls `start-search(sources, query)`, which returns a job id at once. The host runs the fan-out natively, outside any guest call, under the same limits. When it ends, the host stores the results and raises a `search.done` event on the event feed. A second guest call reads the results by job id and merges them with the federated query library. Each guest call stays short, so the epoch deadline never applies to the wait. The library's merge code is the same with either approach.
- **What the fallback would change, beyond OF-4:**
  - **The client API (OF-6).** A search is no longer one client call. The client calls `search.start`, waits for the `search.done` event, then calls `search.results`. `web` cannot chain the two steps inside one call, because it would hit the same deadline while it waits. So every client needs the event feed for search, and A5 (event feed) must land before A4.
  - **The exchange protocol (M08F).** A friend that receives a forwarded search answers from its own index only and never forwards it on (VX-34), so its side stays one wire call. The problem is the **forwarding** exchange. A participant's node calls it over the wire, and it must fan out to its friends before it can answer. With the fallback it cannot wait for them inside that one wire call. So `exchange.search` returns the exchange's own results at once, plus a job id for its friends' results. The participant's node then fetches the friends' results by job id (a second wire-reachable method, `exchange.search-results`), or the exchange sends them back with a call to the participant's `inbox` service. This adds a method, a job table and a second round trip to the participant-to-exchange protocol in M08F. M08F's plan chooses between the two ways.
  - The cost: two guest calls instead of one, a small job table in the host, and the client and M08F changes above.
- The spike proved the primary approach, so the fallback was not needed.

Rejected: wait for async guests (WASI 0.3). Its date is not known, and M8 federation needs the loop now. Rejected: a native search service outside the Roym SynApp. It would make the WASM build of Roym incomplete, which the integrated spec forbids (D2).

### Federated query library

A Roym library in `crates/roym_federated_query`, Cargo package `syneroym-roym-federated-query`. It takes a list of sources, a query and limits. It calls `call-all` (or the fallback), verifies each record with the existing `roym_core` functions, merges, removes duplicates, and returns a page with one outcome per source (`ok`, `timed-out`, `refused`, `unreadable`).

**Who owns what:**

- The library owns fan-out, merging and the removal of duplicates. Today's `directory.merge` code moves into it.
- The client half of `directory` keeps the participant's list of sources and calls the library. It holds no merge code of its own.
- Every service still verifies the records it receives.

Two users:

1. **The participant's own search.** One method runs the whole search. The client sends the query and gets the merged page.
2. **The exchange, later** (the [intent matching](../roym-intent-matching/change.md) change). An exchange forwards a search to its friend exchanges with the same library.

### Event feed

One feed per node, for the people and services on it.

- **Events are small.** Each one names a kind (`conversation.message`, `conversation.delivery`, `transaction.card`, `transaction.booking`; `match` comes in M08F), an id and a cursor. Content is fetched with the normal methods. So the feed leaks nothing new, and needs no new access rules beyond "the caller is the owner".
- **Catch-up.** Each event has a cursor. A client that was away asks for events after its last cursor. The node keeps a bounded window of events. Past the window, it answers "reload".

Where the feed lives was decided in the M08A spike (D-OF-3): the host-level event bus.

| | A host-level event bus (decided) | The `web` service (a WASM guest) |
| --- | --- | --- |
| How a service writes an event | One host call to the bus. It returns at once. | A synchronous internal call to `web` on every write. |
| Who holds the open connections | The host, which lives as long as the node. | `web`'s guest instance. Each ordinary call and each WebSocket event creates a new store and instance (`crates/roym_web/src/app.rs:295`: "nothing reads connection state"), so `web` cannot keep connection state in memory and would need a new storage path. |
| How clients connect | WebSocket through the client gateway, checked against the login session. | WebSocket to `web`, as M06A allows. |
| How services subscribe | The host calls the service's export, or the service reads by cursor. | `web` calls each subscribed service. |
| Who sees which events | **Scoped by app instance.** The host tags each event with the app instance that wrote it, from the caller identity, never from the event body. A service subscribes only to its own app's events. A client session gets only the events of apps it may use. One app can never read another app's events. | Only Roym's own events exist, so there is no cross-app question. |
| Cost | A new host interface, shared by every SynApp. | Roym-only, but slower, and one more state problem in a guest. |

The host-level bus is simpler, faster, and does not need state in a guest (D-OF-3).

**Catch-up window and kinds (D-OF-4):**
- In-memory window per app instance: newest 10,000 events or 24 hours, whichever is smaller.
- Events carry a run id (node session) and sequence number. After a restart, or for cursors past the window, the node responds with "reload".
- Four v1 event kinds: `conversation.message`, `conversation.delivery`, `transaction.card`, `transaction.booking` (owned and written by Roym services).

### Remote clients

- **Remote access is a setting, off by default.** Binding a non-loopback address exposes every route behind the client gateway to the network, not only Roym: every app's hostnames, the auth service and the static assets. So it must be the node owner's explicit choice. The setting names the bind address.
- **The default stays `127.0.0.1`.** The code changes from `0.0.0.0` to match the spec. This closes the backlog row about the bind address.
- **`local` login is loopback only, by the socket.** The node reads the person's key from `person_identities_dir`, so `local` proves nothing about the client. The gateway checks the TCP socket's peer address, never a header such as `X-Forwarded-For`. When remote access is on, `local` login is off completely. Reason: an owner may put a TLS proxy (for example Caddy) on the same machine. Then every request, from anywhere, reaches the gateway from loopback, and a loopback check alone would open `local` login to the whole network.
- **Remote clients use `delegated-key`.** It already does a challenge and proof (`login_delegated_key`). The Hub uses it when it is not on the node's machine.
- **The Hub's Content Security Policy** hardcodes `auth.localhost` today (backlog §10). It must allow the node's configured auth host before the Hub can work from another machine.
- **Docker.** `deploy/docker-compose.community.yml` deploys a **public community node**: it publishes every port to the host (`7960:7960` for the gateway, and the registry and Iroh ports), and it mounts `/etc/letsencrypt` for native TLS. The client gateway serves app hostnames to outside HTTP clients, so on such a node it must stay reachable from outside. Inside a container, a gateway bound to `127.0.0.1` cannot be reached through any port mapping, because requests arrive on the container's network interface. So the compose file's config turns remote access on, with the bind address `0.0.0.0` inside the container, and the mapping stays `7960:7960`. Remote clients log in with `delegated-key`, and `local` login is off because remote access is on. An owner who runs the image as a private node can change the mapping to `127.0.0.1:7960:7960`, and the compose file says so in a comment. `unverified`: that every community node needs a public gateway; A6 confirms this with the owner before changing the file. The compose file, the `Dockerfile` comments and the developer guide's Docker section change in A6.

### Typed methods

Each method gets an input struct and an output struct in `roym_core` (or in the service crate when only it uses them), with `serde`. This makes a schema possible later, with a crate such as `schemars`, without hand-written files. The publish step itself is deferred.

**Unknown fields depend on who can call the method:**

- **Local-only methods** use `deny_unknown_fields`. The caller is the node's own client, built with the node, so an unknown field is a bug.
- **Wire-reachable methods** ignore unknown fields. Today these are `directory.search`, `directory.info`, `directory.publish` and `directory.standing`. Later, the shop browse methods, `notify` and the `obligation` methods join them. Other people's nodes call them, and those nodes may run a newer version. If unknown fields were refused, an older node would reject every request from a newer node with `-32602` as soon as M8 added a field.
- This keeps one direction working: a newer node can call an older one. A removed or renamed field still breaks the call. That is accepted until G5 (cross-version compatibility) is planned.

## Open questions

- **Typed-method check.** OF-15 needs a list of all methods. `roym_core::router` lists method prefixes, not methods. The plan decides whether each service exports its method table.

## Tasks

- [x] M08A spike: the epoch-deadline callback for `call-all`, and the event feed's home. Write both results here.
- [ ] Write the M08A slice plans.
- [x] Participant rename across Roym crates, the Hub, `roymctl`, docs.
- [ ] `call-all` with its limits, the deadline extension (or the fallback), and the native shim.
- [ ] The federated query library. Move merging and duplicate removal into it. One search method.
- [ ] The event feed: the bus (or `web`), scoped by app instance, the WebSocket path, internal subscription, catch-up.
- [ ] Remote access setting, default bind `127.0.0.1`, `local` login by socket peer address and off when remote access is on, `delegated-key` for remote clients, the Hub CSP, the Docker deploy.
- [ ] Typed input and output for every Roym method, the unknown-field rule, and the check test.
- [ ] "Build a Roym client" in the developer guide.

## Decisions

- **D-OF-1. Epoch deadline extension for `call-all`.** Proved by [crates/sandbox_wasm/tests/epoch_wait_credit.rs](../../../crates/sandbox_wasm/tests/epoch_wait_credit.rs) (PR [#329](https://github.com/syneroym/syneroym/pull/329), 20/20 runs passed).
  - The store's data holds the wait-credit state: when the guest call started, its budget, and the credited wait time so far.
  - Credit and start time are reset at every `set_epoch_deadline`: `engine.rs:570` for each store, and `stream.rs:188` for each stream chunk.
  - The budget is the budget of that call kind: dispatch, lifecycle hook or ABAC.
  - `call-all` adds the time it waited to the credit, also when it ends with an error or a timeout, up to the cap (at most one `call-all` total limit of credit per guest call).
  - **No other host call gives credit.** Giving credit only to `call-all` keeps one guest call's wall time bounded. A single `proxy.call` can still wait up to 30 s with no credit (backlog row).
  - The store uses `epoch_deadline_callback` in place of `epoch_deadline_trap`. The callback computes own time (wall-clock elapsed time minus credited wait time). If own time is at or past the budget, it returns `UpdateDeadline::Interrupt`. This produces `Trap::Interrupt`, identical to `epoch_deadline_trap` today. Otherwise it returns `UpdateDeadline::Continue(ticks)` with the remaining budget rounded up to whole ticks (at least 1).
  - Own time uses the wall clock (`Instant`), because `Engine::current_epoch` is `pub(crate)` in Wasmtime (`wasmtime-48.0.7/src/engine.rs:827`).
  - No `Config` change (`async_support` is a no-op in Wasmtime 48) and no `Yield`.
  - T3 confirmed that Wasmtime's deadline trap is `Trap::Interrupt` with root cause `"wasm trap: interrupt"` (no "epoch" or "deadline" substring).
- **D-OF-2. Rejected fallback (two-step search).** A two-step search (`search.start` + `search.done` event + `search.results`) was rejected because the primary approach (D-OF-1) passed the spike. The fallback would require client-visible changes (OF-6), mandatory event feed usage for search, and a two-round-trip exchange protocol in M08F.
- **D-OF-3. Event feed lives in a host-level event bus.** Simpler, faster, zero guest instances on write, and avoids forcing ephemeral connection state into database storage.
  - The host tags each event with the writer's app instance from `app_context_of` (`engine.rs:535`), never from the event body.
  - A service outside any app instance cannot write events in v1 (recommended).
  - A5 decides how services subscribe (OF-10).
- **D-OF-4. Event kinds and in-memory catch-up window.** In-memory ring buffer per app instance (newest 10,000 events or 24 hours). Lost on restart (answers "reload"). Four v1 event kinds: `conversation.message`, `conversation.delivery`, `transaction.card`, `transaction.booking`.
- **D-OF-5. Search has no live-progress form in v1.** Search is one client call returning a merged page. Bounded by `call-all` total limit. Option (c) (host-emitted progress events) recorded in deferred backlog as a future option.
- **D-OF-6. `call-all` in `syneroym:proxy`.** Placed beside `proxy.call`, reusing target, options, and error types, with JSON text params.
- **D-A1-1. One deal role everywhere.** `pub enum DealRole { QuoteAuthor, Counterparty }` in `roym_core::transaction`. On the wire the values are `"quote-author"` and `"counterparty"`. Every `consumer_did` is now `counterparty_did`, and every `provider_did` is now `quote_author_did`. The row halves `consumer` and `provider` are now `counterparty` and `quote_author`. The name `DealRole` keeps it apart from `AppSandboxRole`, `MailboxRole` and the substrate `[roles.*]`. There is no giver and receiver type in code yet.
- **D-A1-2. Rename in place, no alias.** Record versions stay 1. The storage `SCHEMA_VERSION` of `roym_transaction` goes from 3 to 4, so an older backup is refused with a clear message. A node on an older build and a node on the new build cannot exchange deal cards.
- **D-A1-3. Place values.** `ServiceLocation { AtGiver, AtReceiver, Remote }`, signed as `"at-giver"`, `"at-receiver"` and `"remote"`. The catalog `SCHEMA_VERSION` goes from 2 to 3 and `DIRECTORY_SCHEMA_VERSION` from 4 to 5, because listings with the old values no longer parse.
- **D-A1-4. Other senses are renamed too.** The listing owner (who publishes) and the searcher (who reads results) are named in Roym code, the Hub and the docs. "Customer" is part of the rename.
- **D-A1-5. Words people read.** Cards say "Quote to" and "Quote by", "The payer says they paid", "The person doing the work says it is done", and so on. Errors and `roymctl` help use the exact deal-role words. Places read "At my place", "At their place" and "Remote" in forms, and "At the quote author's place" and "At the counterparty's place" on cards.
- **D-A1-6. Guard tests.** `roym_core` has a test that fails when a signed record or a wire value carries "provider", "consumer" or "customer". It fills every optional field and runs every variant of the role, place, audience and next-step enums. The Hub has a test for the shared notices, the labels and the card text.
- **D-A1-7. Other names.** `NextStep::WaitForQuoteAuthor` (`"wait-for-quote-author"`). The booking helpers use `author` and `counterparty` in their names. `DealRole::as_str()` is the one source of the role name in fence ids and in the backup import message.
- **D-A1-8. Audience value.** `OpenTo::ExistingCustomers` is now `OpenTo::PastCounterparties`, signed as `"past-counterparties"`. The Hub label is "People I have worked with before".

## Deviations

- **Giver and receiver are in the docs only.** The Design section above lists `giver` and `receiver` for one commitment. A1 uses one role type for the parties of a deal, because the payment and fulfilment records name the deal parties on both tracks. The backlog tracks the move to giver and receiver with commitments.
- **Enum values are kebab-case.** Field names are `quote_author_did` and `counterparty_did`. The role value is `"quote-author"`, as every enum value on the wire is kebab-case.
- **"Customer" is part of the rename.** It also covers the audience value `existing-customers` (now `past-counterparties`) and the place value `at-customer` (now `at-receiver`).
- **More storage schema versions changed than planned.** The plan named the transaction service only. The catalog and the directory also store the renamed place and audience values, so their versions changed too.
- **One error text changed in the Rust-names-only step.** The backup import message "receipt has role" printed the Rust variant name. It prints the wire name now.
- **`call-all` interface:** task.md and change doc originally named `syneroym:invocation`. Corrected to `syneroym:proxy` beside `call` (Q1 / D-OF-6).
- **`call-all` params type:** sketch originally used `params: list<u8>`. Corrected to `params: string` (JSON text) matching `proxy.call` (Q1 / D-OF-6).
- **`web` instance reuse:** change doc table previously stated `web` instances are reused and keep no state. In fact each ordinary call and WebSocket event creates a new store and instance (`engine.rs:469`, `guest_stream.rs:294`, `app.rs:295`) (Q5 / D-OF-3).

## Close-out

Before merge, update the living docs (requirements, architecture, integrated spec, developer guide, terminology) so they describe the result. Then set `status: done` and fill in `living-docs-touched`. Record anything you postponed in [deferred-backlog.md](../../deferred-backlog.md).
