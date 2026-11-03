---
status: draft
living-docs-touched:
  - docs/TERMINOLOGY.md
  - docs/system-requirements-spec.md
  - docs/system-architecture.md
  - docs/roym-integrated-experience-spec.md
  - docs/developer-guide.md
  - docs/planning/traceability-matrix.md
  - docs/planning/deferred-backlog.md
demo:
  - none: internal typed method contracts, proxy call-all batching, dual-build shim capability, and internal federated query fan-out; search has no demo story because its Hub path is covered by Hub cases 22 and 23, and the milestone demo is the chat story in A5
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
  - One method for a whole search (`directory.find`). The three-call form (`directory.start-run`, `directory.query-source`, `directory.merge`) is removed.
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
| OF-3 | A guest can send one call to many targets in a single host call. The host runs the calls at the same time, each with its own time limit, and returns one result per target in the input order. | `host_capabilities/tests/tests_call_all.rs`: `results_come_back_in_input_order_whatever_the_finish_order`, `a_call_gets_its_own_time_and_the_host_caps_a_larger_request`, `a_failed_call_does_not_change_the_other_entries`. `proxy_call_all.rs`: `batch_of_mixed_calls_through_proxy_matches_single_call_answers`, `local_wasm_callees_execute_in_parallel_via_joinset`. |
| OF-4 | The time the guest waits inside the batched call does not count against its epoch deadline. The batched call has its own total limit, set by the host. If the spike shows this cannot be done, the fallback in [Batched outbound call](#batched-outbound-call) replaces this criterion and changes OF-6, and the change doc records both as deviations. | `dual_build_parity/call_all_deadline.rs`: `a_batch_that_waits_past_the_budget_does_not_stop_the_guest`, `credit_buys_back_the_wait_and_no_more`, `a_guest_that_spins_is_stopped_on_time_when_it_never_used_call_all`, `two_batches_in_one_wasm_guest_call_accumulate_credit`. `engine/epoch_budget/tests.rs`: `zero_credit_interrupts_at_the_deadline_even_when_own_time_is_below_budget`, `arm_resets_credit_and_start`. |
| OF-5 | The same scenario gives the same result in the native build and in the WASM build. | `dual_build_parity/call_all.rs` (eight parity tests) and `the_native_build_has_no_deadline_for_a_slow_batch` in `dual_build_parity/call_all_deadline.rs`. |
| OF-6 | A search over three directories, one of them too slow, returns the results of the two others and names the slow one, in one client call. With the fallback: in one `search.start` call, one `search.done` event and one `search.results` call. | `dual_build_parity/directory_find.rs`: `find_names_the_slow_source_and_keeps_the_other_hits`, `find_marks_an_unreachable_source_not_found_and_keeps_the_others`, `find_over_one_source_matches_the_native_and_wasm_builds`, `find_marks_an_oversized_reply_unreadable_and_keeps_the_others`, `find_ceiling_run_of_eight_sources_fits_the_dispatch_budget`. Hub `search.test.ts`: `findListings_maps_each_error_kind_to_an_outcome`. Hub e2e: cases 22 and 23. `roymctl roym directory find` canned reply tests. |
| OF-7 | Merging results from many sources and removing duplicates is done only by the federated query library. The Hub and `roymctl` hold no merge logic. Each service still verifies the records it receives, with the same `roym_core` verification functions as today. | `crates/roym_federated_query` unit tests: 35 tests in `merge.rs`, `find.rs`, `verify.rs` covering parallel fan-out, sorting, round-robin, size caps, and hit deduplication. `dual_build_parity/directory_find.rs`: `a_second_find_for_one_source_does_not_duplicate_a_listing`. `dual_build_parity/directory.rs`: `scenario_97`, `scenario_98`, `scenario_102c`, `scenario_102d`, `scenario_119`, `scenario_120`. |
| OF-17 | A batched call with more targets than the host maximum is refused before any call is made. The host runs at most a fixed number of outbound calls at once for one guest instance, and queues the rest. | `host_capabilities/tests/tests_call_all.rs`: `a_batch_over_the_maximum_is_refused_before_any_call`, `at_most_the_concurrency_limit_run_at_once_and_the_rest_wait`. `host_capabilities/tests/tests_call_all/limits.rs`: `the_waiter_limit_refuses_at_once_and_a_permit_is_released_after_the_batch`. `dual_build_parity/call_all_deadline.rs`: `slow_batches_do_not_fill_the_instance_pool`. `proxy_call_all.rs`: `batch_over_the_maximum_is_refused`, `local_wasm_callees_pool_exhaustion_is_bounded`. |
| **Event feed** | | |
| OF-8 | A client opens the event feed with its login session. When a message, a card, a booking or a delivery state changes on the node, the client gets an event within 1 s. The event names what changed and a cursor. It carries no message content. | |
| OF-9 | A client that reconnects with its last cursor gets every event after that cursor, or is told the cursor is too old and it must reload. | |
| OF-10 | A service on the node can subscribe to the same events through an internal subscription, without WebSocket. A service receives only the events of its own app instance. A client receives only the events of the apps its login session may use. | |
| OF-11 | The Hub shows a new incoming message on an open screen with no user action. | |
| **Remote clients** | | |
| OF-12 | `local` login answers only a request whose TCP socket peer address is a loopback address. Headers such as `X-Forwarded-For` are never used for this check. When remote access is on, `local` login is turned off completely, because a proxy on the same machine (for example Caddy for TLS) makes every request arrive from loopback. A refused request gets an error that names `delegated-key` as the method to use. | |
| OF-13 | By default the gateway binds `127.0.0.1`, and the spec and the default config say so. A node owner turns on remote access with a config setting that names the bind address. With it on, a client on another machine logs in with `delegated-key` and can use every Roym method. The Hub works from that machine (its Content Security Policy no longer names only `auth.localhost`). The Docker deploy (`deploy/docker-compose.community.yml`, `Dockerfile`) still works after the default changes. | |
| **Typed methods** | | |
| OF-14 | Every Roym method reads its input into a typed struct and returns a typed struct. A wrong field type gets `-32602` with the field name. | Met for all 141 methods of the five services with method tables, and for `web`'s own `session.whoami`. `method_contract`: `contract.rs` (`every_*_method_has_typed_input_and_output`, `web_has_one_typed_method_of_its_own`), `contract_directory.rs` (`directory_parameter_errors`, `whoami_rpc_rejects_unknown_fields`); the 214 shape files in `method_shapes/` fix every output. Both builds give the same error. Not covered: `web`'s forwarder (it passes `params` unchanged to the owning service), the `status` exports, and two opaque peer-card fields that stay `Value` (`CardRow.data`, `PeerStep.snapshot`); the Hub and `roymctl` still send raw JSON. |
| OF-15 | A test lists every Roym method and fails when one has no typed input or output. | `method_contract` `contract.rs`: one table test per service (24 + 24 + 17 + 41 + 35 rows) runs `assert_service_table` (`roym_core/src/method/check.rs`: unique names, routing, no `Value` as a top-level type, hand-written match arms and unlisted method names in the source); `every_listed_method_is_dispatched` runs each row in both builds. `web` has its own test, because its forwarder reads `req.method`. Not covered: a `Value` nested inside a struct. |
| OF-18 | A local-only method refuses an unknown input field with `-32602`. A wire-reachable method ignores unknown input fields, so a node that has added a field can still call an older node. | Refusal: `assert_service_table` probes every row (137 local-only rows refuse, the 4 wire rows ignore); `contract.rs` `invalid_field_types_and_unknown_fields_are_rejected` and `contract_directory.rs` `directory_parameter_errors` check the error over `/rpc`. Ignoring: `directory_wire_unknown_fields_and_parameters` (search, standing, info, publish over the wire), `every_method_answers_off_the_node_as_its_wire_rule_says`, `dual_build_parity/directory_compat.rs` (a newer node's answer is read), `roym_core` `method/tests/directory.rs`. Not covered: a nested record type inside an input drops an unknown key and does not refuse it; a renamed or removed field still breaks the call (G5). |
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
record call-request {
    target: call-target,
    %interface: string,
    method: string,
    params: string,
    options: option<call-options>,
}

/// Runs the calls at the same time and returns one result per call,
/// in the order of `calls`.
call-all: func(calls: list<call-request>)
    -> result<list<result<string, proxy-error>>, proxy-error>;
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

A Roym library in `crates/roym_federated_query`, Cargo package `syneroym-roym-federated-query`. It takes a list of sources (`Source { did, pinned_issuer }`), a query (`SearchQuery`), and `now_secs`. It calls `proxy::call_all`, checks every raw reply against `MAX_SEARCH_REPLY_BYTES` (2 MiB), parses JSON-RPC responses, verifies each record with `listing::verify_envelope` and `membership::evaluate` from `roym_core`, checks membership evidence against pinned issuers, caps results at 10 verified hits and 10 refused hits per source after checking (and 10 per source on the page), and merges deterministically with round-robin ordering across sources into a `Page` (up to 50 hits, 20 refused). The outcome kinds per source are `error: null`, `not-found`, `timed-out`, `not-started`, `refused`, and `unreadable`.

**Who owns what:**

- The library owns fan-out, checking, membership evaluation, merging and duplicate removal. The old `directory.merge` code moved into it.
- The client half of `directory` (`client_find.rs`, `client_runs.rs`) keeps the participant's list of sources and calls the library. It holds no merge code of its own. It persists page envelopes in working state under `search_runs` for retrieval via `directory.run-envelope`.
- Every service still verifies the records it receives.

Two users:

1. **The participant's own search.** One method (`directory.find`) runs the whole search. The client sends the query and gets the merged page.
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

- **Typed-method check.** Resolved by D-A2a-2 and D-A2a-17.

## Tasks

- [x] M08A spike: the epoch-deadline callback for `call-all`, and the event feed's home. Write both results here.
- [ ] Write the M08A slice plans.
- [x] Participant rename across Roym crates, the Hub, `roymctl`, docs.
- [x] `call-all` with its limits, the deadline extension (or the fallback), and the native shim.
- [x] The federated query library. Move merging and duplicate removal into it. One search method.
- [ ] The event feed: the bus (or `web`), scoped by app instance, the WebSocket path, internal subscription, catch-up.
- [ ] Remote access setting, default bind `127.0.0.1`, `local` login by socket peer address and off when remote access is on, `delegated-key` for remote clients, the Hub CSP, the Docker deploy.
- [x] Typed input and output for every Roym method, the unknown-field rule, and the check test.
  - [x] profile and conversation
  - [x] catalog and transaction
  - [x] directory and web
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
- **Q1–Q9: option (a), 2026-10-30.**
- **D-A2a-1. One wrapper for every handler.** A handler is `async fn(host: &H, input: I) -> Reply<O>` with `type Reply<T> = Result<T, Response>`. The `Err` side is an error `Response`. A function `typed` reads `req.params` into `I`, runs the handler, and writes `O` with `serde_json::to_value`. The parse runs once, before the handler. The one effect on the order of checks is B11.
- **D-A2a-2. One list per service writes the dispatch and the table** (`roym_core::service_methods!`, Q1 (a)). `invoke` keeps `require_internal` and then calls `dispatch`. It has no `match` on method names and no `handle_certificate_verb` call in profile and conversation. The certificate verbs are normal rows in the macro list, handled by typed functions in `roym_core::signing::verbs`. `certificate_rows` is deleted.
- **D-A2a-3. Where types live** (Q2 (a)). `roym_core::method` for X2. `roym_core::signing::verbs` for shared certificate types and verb handlers. `roym_core::method::profile` and `::conversation` for the structs that a sibling uses. `src/api.rs` for the `service_methods!` list in each service, and each service-only struct in `src/app/<area>.rs` beside the handler that reads it. No `src/api/` subdirectory is added.
- **D-A2a-4. Every A2a input refuses unknown fields.** Each input struct has `#[serde(deny_unknown_fields)]`. `NoParams` too (Q6 (a)). Two input structs are never joined with `#[serde(flatten)]`: `flatten` and `deny_unknown_fields` do not work together, so `Page`-like fields are repeated in each struct. Structs read from storage never use `deny_unknown_fields`.
- **D-A2a-5. The wire half of the rule is built as a check only** (Q3 (a)). `check::assert_service_table` takes a list of wire-reachable method names. A2a passes an empty list. A2c passes the directory's four.
- **D-A2a-6. The error** (Q4 (a)). `-32602`, message `invalid params: <path>: <serde text>` (no `<path>:` when the error is at the top), `data` `{"field": "<path>"}` when a field can be named. If the text starts with `missing field` or `duplicate field`, `field` is the path joined with the quoted name (only the name when the path is `.`). Otherwise, `field` is the path when it has at least one named segment. Otherwise there is no field. Error messages are truncated to 256 bytes on a char boundary. `serde_path_to_error` `0.1` is added to the workspace and to `roym_core`.
- **D-A2a-7. A missing or `null` `params` is `{}`.** Any non-object / non-null JSON value (such as an array) is rejected with `-32602 "invalid params: expected an object"` with no `data`.
- **D-A2a-8. The `group` key stays.** `ConversationRef`, `GroupRenameInput` and `GroupMemberInput` name the field `conversation` with `#[serde(alias = "group")]`. Both keys at once is a duplicate-field `-32602` with `data.field = "conversation"`. No caller sends both.
- **D-A2a-9. "One of two fields" rules stay in the handler**, with their old words: `conversation.open` and `group.add-member`/`remove-member` (`"person_did or address is required"`; `address` wins when both are sent), `block.add`/`remove`/`check` (`"at least one of person_did or address is required"`), `conversation.search` (an empty `query`: `"query is required"`).
- **D-A2a-10. Closed sets are enums** (Q7 (a)): `KindFilter { Direct, Group }`, `ReportSubject { Person, Listing, Message }`, `ReportCategory { Impersonation, Fraud, Harassment, UnsafeService, IllegalContent }`, all kebab-case. Stored rows keep their `String` fields.
- **D-A2a-11. Backup.** `*.export` returns `Bundle`. `*.import` takes `ImportInput { bundle: Bundle }`. The "whole `params` is the bundle" fallback is removed (B9).
- **D-A2a-12. Siblings use the structs** (Q8 (a)). `roym_core::method::call_sibling` takes the typed input and returns the typed output. It replaces `profile_call` in `conversation`. Five call sites in other crates move to it. Each site preserves its existing error handling (B17-B20).
- **D-A2a-13. Tests and clients keep sending raw JSON.** `roymctl`, the parity harness, the e2e tests and the Hub do not import the structs. A contract test that reuses the server's struct cannot find a wrong name.
- **D-A2a-14. The output shape is proved by shape files**, recorded from the unchanged code in Step 2 and kept green by every later step. An array records the sorted, de-duplicated list of its item shapes. A string records its exact value (not `"string"`) when its key is a closed word set (`kind`, `state`, `direction`, `body_encoding`, `admission`, `status`, `content_type`, `subject_kind`, `category`, `note`, or any key under `notices`).
- **D-A2a-15. Output rules.** An output struct reproduces today's JSON exactly. Where the old code wrote `null` (a `json!` with an `Option`), the field has no `skip_serializing_if`. Where the old code left a key out, the field has `skip_serializing_if = "Option::is_none"`. These places have the second kind: `body` and `deleted_at_secs` of `MessageView`, and `retry_after_secs`. `SigningStatusOutput.owner_did` is `Option<String>` without skip. `BlockCheckOutput` uses `struct BlockCheckOutput { blocked: bool, #[serde(flatten)] since: Option<BlockedSince> }` where `BlockedSince { reason: Option<String>, since_secs: Option<u64> }` has no skip on its options, preserving `null` when blocked without a reason. The three `get` methods write `null` for a missing row (an `Option` result, not a key). Stored `admission` remains kebab-case.
- **D-A2a-16. Numbers keep the width the handler already cast to**: `limit` of the conversation methods is `u32` (they cast `as u32`), the profile lists use `u64` (they cast `as usize`).
- **D-A2a-17. The shape tests and check tests live in one binary, `crates/roym_web/tests/method_contract.rs`.** It has modules `shapes` and `contract` in `method_contract/shapes.rs` and `method_contract/contract.rs`, declared with explicit `#[path = "method_contract/..."]` attributes, including the four parity harness helper modules with `#[allow(dead_code)]`.
- **D-A2a-18. Dead types are replaced.** Step 4 replaces `MessageRow` with `MessageView`. Step 6 replaces `ConversationRow` by `ConversationListItem` (`opened_at_secs: i64`, `last_activity_ms: i64`) and reuses `ConversationRowKind` for the `kind` output. `GroupMeta` and `apply_group_profile` remain as domain code.
- **Q1–Q6: option (a), 2026-10-30.**
- **D-A2b-1. No new machinery.** Both services use `service_methods!`, `typed`, `parse_value`, `call_sibling` and `assert_service_table` as A2a built them. The only additions to `roym_core` are structs (D-A2b-2).
- **D-A2b-2. Where types live.** Shared types in `roym_core::method` (`VerifyInput`, `EnvelopeHistoryOutput`); catalog shared types in `roym_core::method::catalog`; service-specific types in `crates/roym_catalog/src/app` and `crates/roym_transaction/src/app`.
- **D-A2b-3. Every input refuses unknown fields at its top level** (`deny_unknown_fields` on every input struct and on `SlotInput`). Record types inside inputs are not changed (Q1 (a)).
- **D-A2b-4. Existing field attributes stay.** A field that has `#[serde(default)]` or `#[serde(default = "..")]` today keeps it, so an absent field behaves as today. An explicit `null` for a non-`Option` field stays an error, as today.
- **D-A2b-5. Closed sets.** Inputs: `listing.list` `status: Option<ListingStatus>`, `booking.list` `state: Option<BookingState>` (D-A2a-10). Outputs: `SendState` and `BookingWriter` (Q5 (a)). The words do not change. `booking.list` compares `BookingState` values, not serialized strings.
- **D-A2b-6. Wide numbers.** `limit`, `offset` and `cursor` are `u64` in input structs and converted with `usize::try_from(v).unwrap_or(usize::MAX)` to avoid 32/64-bit differences.
- **D-A2b-7. Verify methods take `VerifyInput { envelope: String }`.** Sent as a string envelope.
- **D-A2b-8. Output shapes match existing JSON.** `agreement.accept` uses `booking: Option<Option<BookingView>>` with skip so missing is omitted and null is preserved. Counterparty `BookingHistoryStep` keeps raw `Value` snapshot.
- **D-A2b-9. Fences return typed answers and no `Response::ok` in handler code.** Fences F1 to F6 return `FenceOutcome<O>` (`Claimed` or `Answered(O)`). `ensure_*_recorded` returns `Reply<O>`. Crash recovery answers keep their `-32603` on error.
- **D-A2b-10. `booking_view` returns `BookingView`.** Returns typed struct directly.
- **D-A2b-11. Sibling calls keep error mapping.** `call_sibling` mappings for `Ok(None)`, `Refused`, `Transport`, `BadReply`. `sibling_call` and `catalog_call` deleted.
- **D-A2b-12. Exports use `backup::export_signed_bundle`.** Returns `Reply<Bundle>`.
- **D-A2b-13. Closed-word keys recorded in shapes.** Exact string values checked for closed enums.
- **D-A2b-14. Shared deal set-up in tests.** Moved to `deal_fixtures.rs`.
- **D-A2b-15. Transaction conversion in two steps.** Deal records first, then booking, payment, and fulfilment.
- **D-A2b-16. `quote.set` takes typed terms.** `QuoteTermsInput` with `into_agreed_terms`.
- **D-A2b-17. `roymctl` loses `booking list --agreement`.** Option removed from CLI.
- **Q1–Q7: option (a), 2026-10-31.**
- **D-A2c-1. No new machinery beyond three small parts.** `directory` uses `service_methods!`, `typed`, `parse_value`, `call_sibling` and `assert_service_table`. Additions: `WireNoParams`, `admit::caller`, and one allowed `admit::admit` call in the check scan.
- **D-A2c-2. Where types live.** `roym_core::method::directory` for wire structs, `roym_core::method` for `WireNoParams` and `PublicationLimitsInput`, `roym_directory` for local structs, `roym_web` for `WhoamiOutput` and `SessionAuth`.
- **D-A2c-3. Unknown fields.** Local-only input structs deny unknown fields; 4 wire inputs accept them. Nested types keep existing attributes.
- **D-A2c-4. Existing attributes and null stay field by field.** Optional fields use `Option`; required fields stay non-Option.
- **D-A2c-5. Empty string validation stays in handler.** Checks for empty strings remain in handlers with existing error messages.
- **D-A2c-6. Closed sets.** Inputs: `SearchQuery.open_to: Option<listing::OpenTo>`, `SearchQuery.booking_mode: Option<listing::BookingMode>`. Output: `WhoamiOutput.auth: SessionAuth`.
- **D-A2c-7. `directory.publish` reads its caller itself.** Uses `admit::caller` and passes `Caller` to `resolve_published_by`, which resolves the caller DID.
- **D-A2c-8. `directory.set-settings` takes `SynOrgSettingsInput`.** Completely destructured into `SynOrgSettings`.
- **D-A2c-9. Pass-through methods are typed.** Uses `method::read_reply` to preserve `null` results and pass errors through with `data`; unparseable results return `-32603`, and `add_source` reads the probe reply as `Value`.
- **D-A2c-10. Other client-half reads of remote answers do not change.** Left as-is until federated query library work in A4.
- **D-A2c-11. Multi-shape outputs.** Structured `Option` fields with skip serializing for absent keys: `PublishOutput`, `CheckStandingOutput`, `QuerySourceOutput`.
- **D-A2c-12. Stored rows parsed into structs.** Lists skip unparseable rows; settings returns -32603.
- **D-A2c-13. Wire output structs readable by older/newer nodes.** Derive Serialize and Deserialize without deny_unknown_fields; defaults for fields with natural empty values while `name`, `retention_secs`, `member_count`, and `PublishOutput.listing_id` stay required.
- **D-A2c-14. Certificate verbs, export, and deletions.** Certificate verbs in table; export uses `export_signed_bundle`; `handle_certificate_verb` and `export_signed` deleted.
- **D-A2c-15. `web` has one typed method and keeps its forwarder.** `session.whoami` typed with `NoParams` and `WhoamiOutput`.
- **D-A2c-16. One wire list and check scan.** `WIRE_REACHABLE` exported from `api.rs` and read by `admit::admit` and contract tests; the check scan allows only the exact call `admit::admit(host, api::WIRE_REACHABLE, &req.method)` in `app.rs` with one `req.method` on the line.
- **D-A2c-17. Parity lists.** Driven by `all_directory_verbs()` and the hand-written `WIRE_REACHABLE_DIRECTORY_VERBS`, which the contract test asserts equals `WIRE_REACHABLE`.
- **D-A2c-18. Shapes.** `auth` added to closed-word keys.
- **D-A2c-19. Directory conversion in two steps.** Server half then client half.
- **Q1–Q9: option (a), 2026-11-01.**
- **D-A3-1. The WIT.** `proxy.wit` adds `record call-request { target: call-target, %interface: string, method: string, params: string, options: option<call-options> }` and `call-all: func(calls: list<call-request>) -> result<list<result<string, proxy-error>>, proxy-error>`. Results are in input order. An empty batch returns `Ok([])` with no call started.
- **D-A3-2. One request builder, one `invoke`.** `proxy::Host::call` split into `proxy_request`, `resolve_target`, and `reply_text`. `call_all` reuses the same request building, validation, and `ServiceProxy::invoke` execution.
- **D-A3-3. Limits and where they are set.** Six fields in `AppSandboxRole`: `call_all_max_calls` (16), `call_all_max_concurrency` (8), `call_all_default_call_timeout_ms` (5000), `call_all_max_call_timeout_ms` (10000), `call_all_total_timeout_ms` (15000), `call_all_max_waiting_guests` (4). Clamped in `CallAll::from_config` into `CallAllLimits`.
- **D-A3-4. The epoch budget in production.** `EpochBudget` tracks start time, budget, credited wait time, and cap. Zero credit interrupts immediately on deadline. The dedicated OS thread for the epoch ticker ensures deadlines advance regardless of worker thread saturation.
- **D-A3-5. The run.** Calls spawned in `tokio::task::JoinSet` tasks up to `max_concurrency`. Tasks wrapped in `timeout + CALL_CANCEL_GRACE` (250 ms) so timeouts can record dead letters before task abort.
- **D-A3-6. Time of one call.** Clamped to `[1 ms, max_call_timeout]`. Bounds the whole call including retries.
- **D-A3-7. Order and duplicates.** Results in input order; duplicate targets run as independent calls; duplicate idempotency keys hit the dedup fence.
- **D-A3-8. Waiters.** Shared node-wide `Arc<Semaphore>` bounded below pool size bounds waiters and prevents pool exhaustion when targets are remote, but does not prevent exhaustion from local WASM callees.
- **D-A3-9. The wait allowance.** Batch total time bounded by remaining allowance; refused if allowance < 1 s (`MIN_BATCH_ALLOWANCE`).
- **D-A3-10. Nested calls.** Nested calls run in separate instances and separate stores with their own credit and concurrency limits; recursion depth bounded by the waiter permit limit.
- **D-A3-11. Target calling own instance or service.** Callee runs in a fresh instance; caller's concurrency limit counts only outbound calls; fails gracefully on instance pool exhaustion.
- **D-A3-12. Guest cancellation and stop.** Dropping batch future aborts `JoinSet` tasks and releases waiter permit; native build counts credit and checks allowance identically but has no epoch deadline interruption.
- **D-A3-13. Metrics and logs.** Emits `substrate.proxy.call_all.batches`, `.calls`, `.refused`, `.timed_out`, `.duration_ms`, and `.waiting`.
- **D-A3-14. The native shim.** `AppProxy::call_all` added to dual-build shim; native host runs the same `HostState::call_all` logic with shared limits.
- **D-A3-15. Deadline classification.** `classify_call_failure` checks `Trap::Interrupt` and classifies as `Deadline`.
- **Q1–Q6: option (a), 2026-11-02.**
- **D-A4-1. The method.** `directory.find` in the method table of `roym_directory`. `FindInput { #[serde(default)] query: SearchQuery }` with `#[serde(deny_unknown_fields)]`. `SearchQuery` stays tolerant of unknown fields. `FindOutput { run_id: String, #[serde(flatten)] page: Page }` defined in `roym_directory` (`client_find.rs`); `Page` comes from the library. Local-only.
- **D-A4-2. Removed methods.** `directory.start-run`, `directory.query-source`, `directory.merge`, `MAX_CLIENT_CONCURRENCY`, `validate_run_and_source`, and their shapes are removed without deprecation. `directory.run-envelope` stays.
- **D-A4-3. The library.** `crates/roym_federated_query` (`syneroym-roym-federated-query`). Depends on `syneroym-app-host`, `syneroym-roym-core`, `serde`, `serde_json`, and `thiserror`. Generic over `P: AppProxy`. Pre-parse 2 MiB size cap (`MAX_SEARCH_REPLY_BYTES`), reply strings dropped as soon as hits are parsed.
- **D-A4-4. One batch, one wave, no chunking.** `find` sends one `call_all` with one request per source, in input order, with `DEFAULT_SOURCE_TIMEOUT_MS`. A list longer than `MAX_SOURCES` (8) is `FindError::TooManySources` (`-32603`). An empty list returns an empty page without calling `call_all`.
- **D-A4-5. Outcome mapping.** `ServiceNotFound` / `DependencyNotBound` -> `NotFound`; `TimedOut` -> `TimedOut`; `Callee(c)` -> `Refused { code: c.code, message }`; other proxy error -> `Refused { -32603 }`; raw reply > 2 MiB -> `Unreadable { "reply too large" }`; malformed JSON-RPC or hits -> `Unreadable`; batch refusal (`Internal` / `UnsupportedTarget`) -> `NotStarted`.
- **D-A4-6. Checking is unchanged.** For each source in returned order: parse into `SearchOutput`, truncate to 10 hits (matching `MAX_STORED_PER_SOURCE` and `MAX_HITS_PER_SOURCE`), verify envelope with `listing::verify_envelope`. Cap at 10 verified and 10 refused per source. Membership evaluated with source's pinned issuer.
- **D-A4-7. The merge moves.** Merging and deduplication functions move to `merge.rs` in `syneroym-roym-federated-query` operating on in-memory `Row` structs without data-layer access. Membership evaluation is performed in `build_hits` via `membership_of` for candidates reaching the page.
- **D-A4-8. What `directory.find` does.** In order: `begin_run` (prune old runs past 1h, mint `run_{now}_{count}`), read sources, `federated_query::find`, update source stats, remember held evidence, persist page envelopes to `search_runs`, return `FindOutput`.
- **D-A4-9. `run-envelope` reads `RunEnvelopeRow`.** Serves signed envelope bytes for records that appeared on the page. Refused field removed.
- **D-A4-10. `NotStarted` stays as a unit variant.** Produced when batch is refused. Hub shows neutral notice: "This installation could not start the search. Press Search to try again." Skipped by `record_source_error`.
- **D-A4-11. The time model (Q2 (a)).** One wave of calls with 2 000 ms each ends within about 2 s plus the 250 ms cancel grace (`CALL_CANCEL_GRACE`). Then `directory` checks up to 8 × 10 envelopes (capped at `MAX_STORED_PER_SOURCE` to fit the WASM sandbox instruction fuel budget) and stores the page's rows. `DISPATCH_HEADROOM_MS` (3 000 ms) is the allowance for that work. Asserted against real `AppSandboxRole::default()` in `find_time_fits_the_real_sandbox_defaults`.
- **D-A4-12. Clients.** Hub uses `findListings(query)` in `search.ts`: one `directory.find` call, 503 -> busy notice, other error -> failure notice, help text updated, old per-source progress maps removed. `roymctl` makes one `directory.find` call, prints canned format, exits non-zero on error.
- **D-A4-13. Safe migration ordering.** `find` added beside old verbs with equivalence test before clients move, then old verbs and shapes deleted.
- **D-A4-14. Slow source and ceiling fakes.** `trust_harness::canned_source_response` made async, adds `hSlow` (sleeps 5s on search only), `hCeiling1..8` (10 valid listings each with maximum 64 KiB payloads and full evidence), `hFiftyHits` (50 maximum-size listings exceeding 2 MiB), `hTwoVersions` (two versions of one listing in one reply), `hOversized` (> 2 MiB payload).
- **D-A4-15. Equivalence before deletion.** Step 4 tested equivalence between the old 3-call loop and `find` on one combined scenario (4 sources: `hForeignWire`, `hForeignWire2`, `hForge1`, `hTrunc`) before Step 7 deletion.
- **D-A4-16. Typed remote search reply.** Remote `directory.search` parsed into `SearchOutput`; non-boolean `truncated` treated as `Unreadable`. Truncation to 10 hits (matching `MAX_CHECKED_PER_SOURCE`) applies before envelope checking.

## Deviations

- **Giver and receiver are in the docs only.** The Design section above lists `giver` and `receiver` for one commitment. A1 uses one role type for the parties of a deal, because the payment and fulfilment records name the deal parties on both tracks. The backlog tracks the move to giver and receiver with commitments.
- **Enum values are kebab-case.** Field names are `quote_author_did` and `counterparty_did`. The role value is `"quote-author"`, as every enum value on the wire is kebab-case.
- **"Customer" is part of the rename.** It also covers the audience value `existing-customers` (now `past-counterparties`) and the place value `at-customer` (now `at-receiver`).
- **More storage schema versions changed than planned.** The plan named the transaction service only. The catalog and the directory also store the renamed place and audience values, so their versions changed too.
- **One error text changed in the Rust-names-only step.** The backup import message "receipt has role" printed the Rust variant name. It prints the wire name now.
- **`call-all` interface:** task.md and change doc originally named `syneroym:invocation`. Corrected to `syneroym:proxy` beside `call` (Q1 / D-OF-6).
- **`call-all` params type:** sketch originally used `params: list<u8>`. Corrected to `params: string` (JSON text) matching `proxy.call` (Q1 / D-OF-6).
- **`web` instance reuse:** change doc table previously stated `web` instances are reused and keep no state. In fact each ordinary call and WebSocket event creates a new store and instance (`engine.rs:469`, `guest_stream.rs:294`, `app.rs:295`) (Q5 / D-OF-3).

### Different from the plan

- **Dual-build parity test harness instance certificate:** The `roym_web` parity harness did not install a service-instance certificate, unlike a real deploy (`deploy/commit.rs`). The shape test needs a successful `group.add-member`, whose host-side prekey fetch runs `check_outbound_identity` and refuses without one. The harness now installs one per service on both builds. This also lets outbox delivery in every parity scenario pass that check; no scenario asserted the old refusal. Production code is unchanged.
- **Substrate directory E2E test `listing.set` parameter:** In `crates/substrate/tests/roym_directory_e2e.rs:392`, the test previously passed `"listing_id": y_listing_id` to `listing.set`. The old untyped handler silently ignored unknown fields, but typed `SetListingInput` enforces `#[serde(deny_unknown_fields)]`. The redundant parameter was removed to match the method contract.
- **`call-all` WIT differs from sketch:** WIT uses five-field `call-request` (`target`, `%interface`, `method`, `params`, `options`), outer `result<..., proxy-error>`, results in input order without `target-index`, and no batch-level `per-call-timeout-ms` (uses per-call `options.timeout_ms` capped by host).
- **Waiter limit and wait allowance:** Node-wide waiter permit limit via `Arc<Semaphore>` bounded below pool size; wait allowance bounds total credited wait time in one guest call across all batches.
- **Zero-credit callback correctness:** Epoch deadline callback interrupts immediately if credited wait time is zero, ensuring guests that never use `call-all` stop at the exact same moment as with the old trap.
- **Deadline trap classification fix:** `classify_call_failure` checks for `Trap::Interrupt` to correctly identify epoch timeouts.
- **Dedicated OS thread for epoch ticker:** Epoch ticker runs on a dedicated OS thread rather than a cooperative Tokio async task, ensuring epoch advances even when Tokio worker threads are saturated by CPU-bound WASM guest execution.
- **`WasmDriver::run` error formatting:** Returns `format!("{e:?}")` instead of `e.to_string()` across all parity scenarios.
- **Router harness and test component additions:** `SleeperService`, `test_route_handler_with_outbox`, `test_route_handler_with_config` added to router test harness; `call-peers-full` and `peer-call` added to `proxy-test`.
- **`NativeHostFactory::with_call_all`:** Stores into `OnceLock` and silently ignores a duplicate call.
- **Outcome kinds:** `not-found` and `not-started` exist besides the four originally sketched in the change doc (`timed-out`, `refused`, `unreadable`, `error: null`).
- **`run-envelope` scope:** Finds only records that appeared on the merged page of a run. Refused envelopes are not persisted or served by `run-envelope`.
- **Search method name:** The method is `directory.find` (matching `roymctl roym directory find`).
- **Removal of three-call form:** `directory.start-run`, `directory.query-source`, and `directory.merge` are removed completely from the method table, as D-OF-5 decided.
- **`SearchOutput` typing and non-boolean `truncated`:** Typed deserialization into `SearchOutput` treats non-boolean `truncated` as `Unreadable` before envelope verification.
- **Deferred membership checks in ceiling scenario (fallback 1''):** In WASM execution, genuine Ed25519 verification and canonicalization of listings with full evidence is CPU and instruction heavy; membership checks were deferred to `build_hits` (at most 50 hits on the merged page) to safely fit headroom and instruction fuel limits.
- **Separate ceiling evidence builders:** Unit tests in `syneroym-roym-federated-query` and dual-build parity tests in `syneroym-roym-web` use separate evidence builders because `roym_core` is a WASM-target guest library that cannot export test helpers requiring `Identity` signing without introducing a host-only crypto dependency into its target-independent production graph.
- **Adjusted dispatch headroom, fan-out source timeout, and per-source stored hit cap (fallbacks 2 and 3):** To fit the ceiling verification budget in WASM execution, `DISPATCH_HEADROOM_MS` was set to 3 000 ms, while `FIND_SOURCE_TIMEOUT_MS` was created at 1 700 ms specifically for `find` requests (`waves * (timeout + 250) + headroom = 1 950 + 3 000 = 4 950 ms < 5 000 ms`), leaving `DEFAULT_SOURCE_TIMEOUT_MS` at 2 000 ms for single-source operations (`probe_info`, `publish_to_source`, `check_standing`). Furthermore, to prevent instruction fuel exhaustion on WASM against the default 10-billion instruction limit (`default_max_instructions = 10_000_000_000`), fallback (2) was applied: `MAX_STORED_PER_SOURCE` was formally lowered from 20 to 10 (matching `MAX_HITS_PER_SOURCE = 10`), capping checked hits per source to 10 and bounding ceiling instruction consumption to ~7.3B instructions (under 892 ms wall time with fixture pre-warming). In the honest ceiling run with 8 sources returning 10 maximum-size hits each (80 total verified listings with maximum 64 KiB payloads and full evidence), measured WASM fuel consumption is 7 306 615 468 instructions (~73% of the default 10 000 000 000 limit, leaving ~27% margin or ~2.69B instructions headroom). The owner confirmed keeping 10 checked listings per source with this margin rather than lowering the cap further. Because `MAX_STORED_PER_SOURCE` was originally 2× the page cap to allow round-robin merging when earlier listings duplicate across sources, lowering it to 10 means a source whose first listings duplicate another directory may contribute fewer than 10 hits to the merged page.
- **Pre-parse reply size cap and bounded query limit:** `MAX_SEARCH_REPLY_BYTES` was lowered from the planned 8 MiB to 2 MiB, derived from 10 maximum-size hits (64 KiB payload + 42 KiB evidence + envelope overhead) with ~2× JSON string escaping factor (~1.66 MiB) plus ~340 KiB margin. In `build_requests`, `query.limit` sent to each source is bounded to `min(limit.unwrap_or(10), 10)`, ensuring honest sources return at most 10 hits and preventing oversized or hostile responses from being parsed into guest JSON-RPC structures.
- **`thiserror` dependency in `syneroym-roym-federated-query` (D-A4-3):** The plan listed `serde` and `serde_json` only; `thiserror` was added to derive error variants for `FindError`.
- **Equivalence testing scope before deletion (D-A4-15):** Equivalence between the old 3-call loop and `find` was verified on one combined scenario with four distinct sources (`hForeignWire`, `hForeignWire2`, `hForge1`, `hTrunc`), rather than running the six separate planned scenarios (97, 98, 102c, 102d, 119, 120) through the old loop.
- **Demo story owner exception (Q6):** Search has no dedicated demo story; its Hub path is covered by existing test cases, and the milestone demo is the chat story in A5.
- **Method shapes and method table count:** Shapes for `start-run`, `query-source`, and `merge` deleted; `directory_find_empty.json` and `directory_find_with_hits.json` added. Method table has 35 methods (38 minus 3).
- **Demo close-out:** Front matter `demo:` combines the earlier slices' reasons (typed contracts, call-all batching, dual-build shim) with the search fan-out exception.

### Planned wire changes (plan section 4)

- **Hub profile_envelope format (B1):** The Hub's object parameter was a bug and was updated to send trimmed string text.
- **NoParams refusal changes parity scenario 2 (B2):** `profile.policy` now rejects unexpected extra fields (`auth`), returning `-32602` with `data.field = "auth"`.
- **Error text changes for missing and wrong fields (B4, B5, B10):** Standardized on `-32602` with `data.field` across profile and conversation. When no field can be named, for example when `params` is not a JSON object, `data` is absent.
- **Enums for closed word sets (B7, B8):** `conversation.list` now rejects invalid `kind` values with `-32602` instead of ignoring them.
- **Import parameter format (B9):** Removed support for passing the bundle directly as the whole params object; callers must send `{ "bundle": ... }`.
- **Certificate verbs refuse unknown fields (B21):** The certificate verbs of `catalog`, `transaction` and `directory` now refuse unknown fields, and a missing `certificate` gives `missing field` text instead of `certificate is required`. This also holds for `<svc>.signing-status`, which now takes `NoParams`.
- **Stored profile without record_id (B22):** `profile.get` answers `-32603` for a stored row with no `record_id`.
- **Check ordering in `request.set`, `quote.set`, `agreement.accept`, `quote.decline` (B23):** Input parsing runs before certificate, owner, range, and existence checks. An invalid parameter returns `-32602` before any business logic check.
- **Error text and field pointer on invalid params (B24):** Standardized on `-32602 invalid params: <path>: <text>` with `data.field` across catalog and transaction. Scenario 40 updated.
- **Refusal of unknown top-level keys across catalog and transaction (B26):** Local-only methods reject unknown top-level keys with `-32602` (`data.field = <key>`). Removed `--agreement` from `roymctl roym transaction booking list`.
- **Verify methods require string envelope (B28):** Verify methods reject JSON object envelopes with `-32602` (`data.field = "envelope"`). The Hub was updated to send string text.
- **Import parameter format for catalog and transaction (B29):** Passing bundle as whole params is rejected; callers must pass `{ "bundle": ... }`.
- **Listing limits overflow rejected (B30):** `max_per_window` above `u32::MAX` is rejected with `-32602`.
- **Invalid enum variants rejected (B31):** Unknown listing status or booking state rejected with `-32602 unknown variant` with `data.field`.
- **Malformed slot row handling (B32):** `availability.get` returns `-32603` on unparseable stored row; `availability.list` skips malformed rows.
- **Sibling error mapping standardization (B33):** Sibling calls use `call_sibling`: same as before, except that an unreadable or malformed `availability.get` reply is now `no-such-slot` (`quote.set`) or `slot-unavailable` (`booking`), not `-32603`.
- **Null or missing params accepted when all fields optional (B34):** Methods where all fields are optional accept `null` params or `{}`.
- **Large limit/offset supported across targets (B35):** `u64` values accepted uniformly on WASM and native.
- **Quote set rejects `quote_expires_at_secs` in terms (B36):** Overwriting terms expiry disallowed with `-32602` (`data.field = "terms.quote_expires_at_secs"`).
- **Credential issue note type (B41):** Non-string note rejected before signing with `-32602` (`data.field = "note"`).
- **Directory missing/wrong parameter errors (B42, B43, B45):** Standardized on `-32602 invalid params: <path>: <text>` with `data.field` across directory and wire methods. Empty strings keep their existing handler error words.
- **Refusal of unknown top-level keys across directory and whoami (B44, B52):** 33 local-only directory methods and `session.whoami` reject unknown keys with `-32602` (`data.field = <key>`).
- **Enums for closed word sets in directory search (B46):** `open_to` and `booking_mode` validated against enums before fan-out.
- **Directory set-limits overflow (B47):** `max_per_window` above `u32::MAX` rejected with `-32602`.
- **Directory import parameter format (B48):** Passing bundle as whole params rejected; callers must pass `{ "bundle": ... }`.
- **Malformed stored rows in directory (B49):** Directory list methods skip unparseable rows; `directory.settings` returns `-32603`.
- **Directory info parameter format (B50):** Non-object params rejected with `-32602 invalid params: expected an object`.
- **Pass-through remote reply typing (B51):** `probe-info` and `publish-to-source` drop unknown keys from remote responses and return `-32603` on unparseable results; remote errors pass through with `data`.
- **Publish caller attribution (B53):** Wire publish recorded and rate-limited under verified caller DID, verified by contract tests (no visible change on wire).
- **Directory certificate verbs in table (B54):** Handled via table rows (no visible change on wire).
- **Wire search with absent/null params (B55):** `directory.search` over the wire accepts absent or `null` params, returning newest listings as `{}` does.
- **Serde error ordering (B56):** Request with multiple errors reports the first one parsed by serde.
- **Null SynOrg result preservation (B57):** `probe-info` preserves `{"result": null}` when remote has no SynOrg; `add-source` records the probe note. Tested in `directory_compat.rs`.
- **Removed search verbs (W1):** `directory.start-run`, `directory.query-source`, and `directory.merge` are removed. They answer `-32601` locally and `-32013` over the wire (P13).
- **`directory.find` added (W2):** New local-only method `directory.find`. Input `{ "query": SearchQuery }` (`query` optional, default `{}`; top-level unknown fields refused). Output `{ run_id, outcomes: [{source, verified, refused, truncated, error}], hits, hits_truncated, refused, refused_truncated }`.
- **`SearchQuery` error path under `find` (W3):** An invalid query field (for example `open_to`) is refused by `directory.find` with `-32602` and `data.field` = `query.open_to`. Before, `query-source` refused it as `open_to`.
- **`directory.run-envelope` restricted to page records (W4):** `directory.run-envelope` has the same input and output. It finds only records that are on the page `find` returned (Q3 (a)). Before, it found any verified row of the run, including a hit that did not reach the page.
- **Outcomes order (W5):** The `outcomes` list is in source order (sorted by DID). Before, a client got outcomes in the order the sources answered.
- **`max_concurrency` removed (W6):** `max_concurrency` is gone from every reply.
- **Outcome verification counts (W7):** `verified` and `refused` in an outcome count what passed checking and the per-source cap, not what was stored successfully.
- **Wire search request format and timeout (W8):** The `directory.search` request that goes to each source sends `["{\"method\":\"directory.search\",\"params\":<query>}"]` with `query.limit` bounded to at most `MAX_CHECKED_PER_SOURCE` (10, or caller's smaller limit), interface `directory`, method `invoke`, `idempotent`, 1 700 ms (was 2 000 ms before this change).

## Close-out

Before merge, update the living docs (requirements, architecture, integrated spec, developer guide, terminology) so they describe the result. Then set `status: done` and fill in `living-docs-touched`. Record anything you postponed in [deferred-backlog.md](../../deferred-backlog.md).
