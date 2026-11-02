# M08A slice A4 — Federated query library and one search call: implementation plan

*Reader: the developer (or AI) who implements slice A4. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan reviewed 2026-11-02. The owner answered (a) to Q1–Q6 on 2026-11-02.**

**Size and PR.** M. One PR on `feat/m08a-a4-federated-query`, with the
commits of section 6 in that order. The branch starts from `main` at
`2f043276` (A3 is merged). A4 adds one crate, changes the `directory`
service, the Hub, `roymctl`, the tests that drive the old search loop and one
list in `xtask` (the allowed dependencies of Roym crates). No `src/` change outside
`roym_directory`, `roym_core`, the new crate, the Hub UI and `roymctl`. The
substrate test files and the sample-config comment change as listed in
Step 6 and §9 (P24).

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`).
  - Functions of at most 100 lines. Match arms of at most 10 lines. Files of
    at most 800 lines. `store_verified_hit` in
    [client_query.rs:333](../../../../crates/roym_directory/src/app/client_query.rs#L333)
    carries `#[allow(clippy::too_many_arguments)]`. The move in Step 3 must
    not carry that allow over.
  - **New crate naming:** directory `crates/roym_federated_query`, package
    `syneroym-roym-federated-query`.
  - WASM rules: the new crate is linked into the `directory` component, so
    it must build for `wasm32-wasip2`. After any change to `roym_directory`
    or the library, rebuild the WASM artifacts (`mise run build:roym`)
    before any test that loads them. The parity harness loads pre-built
    components, so a stale build runs old code.
  - No planning ids in code: no "M08A", "A4", "OF-6", "D-A4-3" in code,
    comments or test names. ADR ids are fine. A comment says why, not which
    plan.
  - Comments never cite size rules or line counts.
  - Keep every "why" comment beside code that stays (A2b R8). The merge code
    in [client_merge.rs](../../../../crates/roym_directory/src/app/client_merge.rs)
    has long why-comments (round-robin, forged `listing_id`). They move with
    the code.
  - The import clean-up pass, the deferred backlog, the living-docs
    close-out, the ratchets (`check-file-lengths`, `check-duplication`,
    `check-lint-suppressions`).
  - Tests never hardcode a port.
- [task.md](task.md): section 1 (X3) and "A4 — Federated query library and
  one search call".
- [roym-open-foundation change doc](../../changes/roym-open-foundation/change.md):
  "Federated query library", OF-6, OF-7, decisions D-OF-1, D-OF-5, D-OF-6,
  D-A3-1 to D-A3-15. No other change doc defines a criterion for A4.
  `task.md` names no other change doc for this slice.
- [slice-a3-implementation-plan.md](slice-a3-implementation-plan.md)
  **section 12** (what A4 must decide). Q2 below answers it.
- [slice-a2c-implementation-plan.md](slice-a2c-implementation-plan.md): the
  directory is typed; A4 keeps its conventions (`service_methods!`, `typed`,
  `Reply`, `assert_service_table`).
- The review files [slice-a2a-review.md](slice-a2a-review.md),
  [slice-a2b-review.md](slice-a2b-review.md),
  [slice-a2c-review.md](slice-a2c-review.md) and
  [slice-a3-review.md](slice-a3-review.md). Section 7.7 lists how A4 avoids
  each kind of finding.
- [deferred-backlog.md](../../deferred-backlog.md) section 9 and section 11,
  the rows named in section 8 of this plan.

### Terms used in this plan

- **Source:** one directory this person added with `directory.add-source`.
  The node stores it in the `sources` collection
  ([client_sources.rs:25](../../../../crates/roym_directory/src/app/client_sources.rs#L25)).
  A person may add at most `MAX_SOURCES` = 8
  ([directory.rs:48](../../../../crates/roym_core/src/directory.rs#L48)).
- **Search** (server half): `directory.search`. A directory answers it from
  its own index. Other nodes may call it.
- **Find** (client half): the new `directory.find`. It asks every source with
  `directory.search`, checks every answer, merges and returns one page.
- **Run:** one find. It has a `run_id`. The node keeps the signed bytes of
  the page's records under that id for one hour, so `directory.run-envelope`
  can return them.
- **Outcome:** what one source contributed: how many verified hits, how many
  refused hits, whether the source said it had more, and an error if it
  failed.
- **Wave:** a group of calls that `call-all` runs at the same time. With 8
  sources and a concurrency limit of 8 there is one wave.
- **Library:** the new crate `syneroym-roym-federated-query`.
- Epoch, wait credit, own time, batch, waiter: as in
  [slice-a3-implementation-plan.md](slice-a3-implementation-plan.md) section
  0.

## 1. What A4 builds, in one paragraph

A4 adds one crate, `crates/roym_federated_query`. Given a list of sources and
a query, it sends one `call-all` batch (one `directory.search` call per
source, 2 s each). Then it classifies each answer, checks every returned
listing with the existing `roym_core` functions, and merges the result with
the existing rules (round-robin across sources, newest version wins, caps).
It returns one page with one outcome per source. The merge and
duplicate-removal code moves out of `roym_directory` into this crate
unchanged. The `directory` service gets one new local method, `directory.find`.
It reads the source list, calls the library, and then does the bookkeeping the
library must not do: it keeps the page's signed bytes for
`directory.run-envelope`, remembers membership evidence, and updates each
source's last-ok and last-error. The three methods `directory.start-run`,
`directory.query-source` and `directory.merge` are removed. The Hub and
`roymctl` make one call and print or draw the page. Their own loops, the
`max_concurrency` hint, the 503 retry and `window.RoymDirectory` are removed.
The proof is an equivalence test (old loop against `find`, same page, both
builds), a slow-source test (OF-6), unit tests of the library, and the
existing parity, contract and end-to-end suites moved onto `find`.

## How this is shown to an end user

A person who searches sees a small change. The results now appear together,
not one directory at a time.

- The person opens the **Directory** tab, types a word and presses
  **Search**.
- The page says "Searching..." for up to about two seconds.
- Then all the results appear at once. Each result still shows which
  directories listed it.
- A directory that did not answer in time, or refused, is named below the
  results in plain words. The results from the other directories are still
  shown.
- If this installation could not start the search, the page says so in
  one line and the person can press **Search** again.
- The `roymctl roym directory find` command prints the same lines as before.

A developer can see the work in these ways:

- `cargo nextest run -p syneroym-roym-federated-query` runs the library tests
  (merge rules, caps, error mapping, request bytes, size limits).
- `cargo nextest run -p syneroym-roym-web --test dual_build_parity find` shows
  that the WASM build and the native build return the same page, and that a
  search with one slow directory returns the other results and names the slow
  one.
- `roymctl roym directory find --category cycling` prints the result. A
  developer can run it against a test node and read the source notes.

**Demo:** none (Q6). Search has no demo story today; its Hub path is covered
by Hub cases 22 and 23, and the milestone demo is the chat story in A5.

## 2. Points for the owner

Each question has options, a recommendation and the reason. Sections 5 and 6
follow the recommendations. The owner's answer goes under each question.

Five choices are not questions, because earlier decisions settle them:

- The three-call form is removed. D-OF-5 says search has no live-progress
  form in v1.
- The merge code moves to the library. The change doc says so
  ("Today's `directory.merge` code moves into it").
- Each record is still verified with the `roym_core` functions used today (OF-7).
- The call goes through `call-all`, as A3 built it (D-OF-6).
- The Hub and `roymctl` keep raw JSON and do not import the structs (D-A2a-13).

### Q1. What is the one search method called?

`directory.search` already exists. Every directory answers it, and other
nodes call it. The new client-half method needs another name.

- (a) **`directory.find`.** It matches the `roymctl roym directory find`
  command, which already exists. Short. Easy to tell from `search`:
  "search" asks one directory, "find" asks all of mine.
- (b) `directory.search-sources`. Longer, but says what it does.
- (c) `directory.federated-search`. Names the library, not the action.

**Recommendation: (a).** The CLI word and the method word are the same, and a
third client learns one word.

Owner answer: (a), 2026-11-02

### Q2. How does `web` carry a search that takes about two seconds?

This is [A3 section 12](slice-a3-implementation-plan.md), points 1 to 3. The
path is Hub, `web`, `directory`, then the sources. `web` forwards with a plain
`call` ([app.rs:217](../../../../crates/roym_web/src/app.rs#L217)). A plain call
gives no wait credit. `web` runs a guest HTTP request with the same 5 s epoch
deadline ([guest_http.rs:44](../../../../crates/sandbox_wasm/src/engine/guest_http.rs#L44)).
So `web` is stopped at 5 s of wall time, whatever `directory` does.

- (a) **Plain forward, and make the search fit.** All sources go in one wave
  (8 sources, concurrency limit 8). Each call has 2 s
  ([directory.rs:72](../../../../crates/roym_core/src/directory.rs#L72)). The
  search then ends in about 2 s plus the time to check the answers. A startup
  test asserts the relationship against the real defaults: `waves ×
  DEFAULT_SOURCE_TIMEOUT_MS + DISPATCH_HEADROOM_MS < dispatch epoch`. This
  replaces today's test, which checks one call.
- (b) `web` forwards `directory.find` through a one-entry `call-all`. `web`
  then gets wait credit. Cost: a special case for one method name in the
  generic forwarder; every search holds two waiter permits (one for `web`, one
  for `directory`), so the node allows 2 searches at once at the default limit
  of 4; and `web` has to read a `call-all` reply.
- (c) Raise the dispatch epoch for `web`. Rejected. It moves a security
  limit for every guest.

**Recommendation: (a).** It adds no code to `web`, uses one waiter permit per
search, and the relationship is tested. The risk is an operator who sets
`call_all_max_concurrency` below 8. Then there are two waves, the search takes
about 4 s plus checking, and `web` may be stopped. Section 7.6 covers this:
the sample config gets a comment, and a backlog row records it.

Owner answer: (a), 2026-11-02. The sample-config comment guards this today;
a backlog row records adding a substrate startup warning when
`call_all_max_concurrency < MAX_SOURCES`.

### Q3. What does the node keep of a run?

Today every verified hit of every source is stored as a row with its full
signed envelope, up to 20 per source
([client_query.rs:333](../../../../crates/roym_directory/src/app/client_query.rs#L333)).
Refused hits are stored too. `directory.merge` reads these rows. After the
merge is in memory, only `directory.run-envelope` reads them.
`run-envelope` is used by one Hub button and by the end-to-end test that
checks the bytes are the ones the listing owner signed.

- (a) **One row per record on the returned page.** After the merge, store
  `{run_id, record_id, envelope, at_secs}` for every record named in the
  page's hits (all sources of each hit, deduplicated by `record_id`). Refused
  hits are not stored. `run-envelope` keeps its wire shape. Rows older than
  one hour are still pruned.
- (b) Keep every verified row as today. No change in what is kept. Up to 160
  writes of full envelopes in one dispatch.
- (c) Remove `run-envelope` and the Hub check.

**Recommendation: (a).** It writes only what a client can ask for, far less
than (b), and the wire method does not change. (c) removes a method the e2e
test uses to prove "the bytes are byte-identical". That is outside this slice.

Owner answer: (a), 2026-11-02. The library carries envelopes to the page step
via a `BTreeMap<record_id, envelope>` beside the rows (P4).

### Q4. What does `find` return when the node refuses the whole batch?

`call-all` can refuse the whole batch: the node has too many waiting guests,
or this guest call has no wait time left (both are `internal` with a text
that starts with `call-all busy:`), or a read-only caller (`internal`), or
too many calls (`unsupported-target`) (P3). Today the same situation appears
per source as `not-started` ("this installation was busy and did not start the
call"), because the node's own door refuses the HTTP call.

- (a) **Return the page normally.** Every source gets the outcome
  `not-started`. There are no hits. The Hub shows one neutral line
  "This installation could not start the search. Press Search to try again."
  when every outcome is `not-started` (P3). No source row is marked as failed
  (the directory is not at fault). The cause is observed host-side via
  `substrate.proxy.call_all.refused` (P3).
- (b) Answer with a JSON-RPC error and no page.

**Recommendation: (a).** `not-started` already exists, already means "the
node did this, not the directory", and the Hub and `roymctl` already have
words for it. (b) adds an error code that every client must learn.

Owner answer: (a), 2026-11-02. All outer refusals (`Internal` and
`UnsupportedTarget`, P3) map to `not-started`. The Hub shows a neutral start
notice; the cause is observed via the host metric `substrate.proxy.call_all.refused`
(P3).

### Q5. Does the library work for other wire methods now?

The exchange (M08F) and the shop browse (M08D) will also fan out to many
nodes. Their wire methods are different from `directory.search`.

- (a) **No.** The library runs `directory.search` and merges `SearchHit`s.
  The first time a second method needs it, that slice generalizes it, with a
  real second user to design against.
- (b) Make the call name, the hit type and the merge key parameters now.

**Recommendation: (a).** We have one user. A parameter list designed with no
second user usually has the wrong parameters. The library's seams are small
(the request builder and the row type), so a later change is local.

Owner answer: (a), 2026-11-02

### Q6. Does A4 add a demo story?

The Mandatory Demo Close-out asks whether the change adds something a viewer
can see. Search is visible, but there is no search story today. A story needs
a node with a SynOrg, a published listing and an added source. The Hub
end-to-end tests (cases 22 and 23) already do this with the Playwright suite.

- (a) **No. Write `- none: <reason>` in the `demo:` list.** The milestone demo
  is the chat story that A5 extends.
- (b) Extend the Roym chat story with a loopback search step in A4.

**Recommendation: (a).** The setup for (b) is large and belongs with a story
that is about discovery. The change doc's `demo:` line already says `none` for
the internal contracts; A4 adds to its reason.

Owner answer: (a), 2026-11-02. `demo: none` is approved as an owner exception
with the updated reason (recorded in change doc Deviations, P11).

All links are to `main` at `2f043276`.

### 3.1 The client half of `directory` today

- The method table has the four client verbs
  ([api.rs:75-78](../../../../crates/roym_directory/src/api.rs#L75)):
  `directory.start-run`, `directory.query-source`, `directory.merge`,
  `directory.run-envelope`. None is in `WIRE_REACHABLE`
  ([api.rs:34](../../../../crates/roym_directory/src/api.rs#L34)), which has the
  four server verbs only.
- `start_run` ([client_query.rs:94](../../../../crates/roym_directory/src/app/client_query.rs#L94))
  creates three collections, prunes `runs` and `search_runs` rows older than
  `RUN_RETENTION_SECS` (3,600), reads and sorts the source DIDs, mints
  `run_{secs}_{count}`, stores a `RunRow {at_secs, sources}`, and returns
  `max_concurrency` = `MAX_CLIENT_CONCURRENCY` (3).
- `query_source` ([client_query.rs:145](../../../../crates/roym_directory/src/app/client_query.rs#L145))
  checks the run and the source ([:229](../../../../crates/roym_directory/src/app/client_query.rs#L229)).
  It makes one `host.call(CallTarget::Service(source), "directory", "invoke",
  json!([{"method":"directory.search","params":query}]), idempotent, 2 000 ms)`.
  It then parses the reply ([:256](../../../../crates/roym_directory/src/app/client_query.rs#L256)):
  `Response` JSON, `result.hits`, `result.truncated`; it truncates the hit list
  to `MAX_STORED_PER_SOURCE + MAX_REFUSED_RESULTS` = 40 **before** verifying.
  It maps errors with `map_proxy_error` ([:429](../../../../crates/roym_directory/src/app/client_query.rs#L429)).
- For each hit it runs `listing::verify_envelope(&hit.envelope, now)`. A
  verified hit becomes a `SearchRunRow` after the per-source cap of 20, with a
  membership verdict from `membership::evaluate` using the source's pinned
  issuer ([:333](../../../../crates/roym_directory/src/app/client_query.rs#L333)),
  and `held::remember` stores the evidence
  ([held.rs:75](../../../../crates/roym_directory/src/app/held.rs#L75)). A
  refused hit becomes a refused row, capped at 20
  ([:396](../../../../crates/roym_directory/src/app/client_query.rs#L396)).
- After the loop it calls `mark_source_ok` ([:456](../../../../crates/roym_directory/src/app/client_query.rs#L456)).
  The early-failure path calls `record_source_error` through `refuse_source`
  ([:313](../../../../crates/roym_directory/src/app/client_query.rs#L313)).
- `merge` ([client_merge.rs:77](../../../../crates/roym_directory/src/app/client_merge.rs#L77))
  loads the run's rows, then runs: `dedupe_and_cap_verified` (newest version
  per listing per source, cap 10 per source, [:135](../../../../crates/roym_directory/src/app/client_merge.rs#L135)),
  `round_robin_merge` (sources in DID order, total cap 50 or 20,
  [:193](../../../../crates/roym_directory/src/app/client_merge.rs#L193)),
  `candidates_by_listing`, `build_hits`, `group_and_sort_refused`,
  `build_refused`. `run_envelope` ([:311](../../../../crates/roym_directory/src/app/client_merge.rs#L311))
  returns the envelope of a **verified** row with that `record_id`.
- Constants in [roym_core/src/directory.rs](../../../../crates/roym_core/src/directory.rs):
  `MAX_SOURCES` :48, `MAX_CLIENT_CONCURRENCY` :59, `MAX_STORED_PER_SOURCE` :64,
  `DEFAULT_SOURCE_TIMEOUT_MS` :72 (2 000), `DISPATCH_HEADROOM_MS` :75 (1 500),
  `RUN_RETENTION_SECS` :78, `SourceError` :220 (`NotFound`, `TimedOut`,
  `NotStarted`, `Refused`, `Unreadable`).
- Only `SOURCES`, not `RUNS` or `SEARCH_RUNS`, is in the export bundle
  ([backup.rs:40](../../../../crates/roym_directory/src/app/backup.rs#L40)). So a
  change to the run rows needs no schema version bump.
- Other single calls in the client half stay as they are:
  `held::check_standing` ([held.rs:100](../../../../crates/roym_directory/src/app/held.rs#L100)),
  `probe_info` and `publish_to_source` ([client_sources.rs:93](../../../../crates/roym_directory/src/app/client_sources.rs#L93),
  [:310](../../../../crates/roym_directory/src/app/client_sources.rs#L310)).
  They are single calls, not fan-out.

### 3.2 `call-all` (from A3)

- `AppProxy::call_all(Vec<CallRequest>) -> Result<Vec<Result<String,
  ProxyError>>, ProxyError>` ([app_host/src/lib.rs:83](../../../../crates/app_host/src/lib.rs#L83)).
  `AppHost` includes `AppProxy`, so any `H: AppHost` can call it. The trait
  needs three methods, so a test fake is small.
- Defaults ([sandbox.rs:97-119](../../../../crates/core/src/config/sandbox.rs#L97)):
  16 calls, 8 at once, per-call default 5 s, per-call cap 10 s, total 15 s, 4
  waiters node-wide.
- A refused batch is an outer `Err`. "Too many waiting guests" and "no wait
  time left" are `Internal("call-all busy: …")`. More calls than the maximum
  is `UnsupportedTarget` ([capabilities_call_all.rs:49-72](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_call_all.rs#L49)).
  Read-only and proxy-unavailable are `Internal`. All outer refusals map to
  `not-started` in the library (P3).
- A call that does not finish in its time has the entry `Err(TimedOut)`
  ([capabilities_call_all.rs:113](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_call_all.rs#L113)).
- Credit is given to the calling guest for its wait. Own time still counts.
  So `directory` has 5 s for its own work (parse, verify, store), and the
  wait for the sources is free (D-A3-4).
- The native shim reaches the same code (`HostState::call_all`).
  `NativeHostFactory::with_call_all` is set in
  [runtime/roym.rs:157](../../../../crates/substrate/src/runtime/roym.rs#L157)
  for the six native Roym services, and defaults to `CallAll::default()`
  otherwise ([factory.rs:312](../../../../crates/app_host_native/src/factory.rs#L312)).
  The `roym_web` parity harness uses the `call-all` defaults on both builds,
  but overrides the WASM dispatch epoch to 120 s (`dispatch_epoch_timeout_secs: 120`
  at [helpers.rs:1072](../../../../crates/roym_web/tests/dual_build_parity/helpers.rs#L1072))
  so CI host load does not trip the normal 5 s deadline (P19).
  (`AppSandboxEngine::init` at
  [helpers.rs:1109](../../../../crates/roym_web/tests/dual_build_parity/helpers.rs#L1109)
  for WASM; the native factories take the default).

### 3.3 `web`

- `web` forwards every method other than `session.whoami` with
  `host.call(CallTarget::Dependency(service), …, None)`
  ([app.rs:217-262](../../../../crates/roym_web/src/app.rs#L217)). It routes
  by prefix: `directory.` goes to the directory
  ([router.rs:47](../../../../crates/roym_core/src/router.rs#L47)), and the
  method must be `MethodAuth::Owner`. A removed method answers `-32601` from
  `directory`, not from `web`.
- A guest HTTP request runs under the dispatch epoch (5 s) and the per-service
  admission permit (`max_concurrent_guest_http_per_service`, default 4). Admission
  waits up to 2 s (`GUEST_HTTP_ADMISSION_TIMEOUT`, [engine.rs:314](../../../../crates/sandbox_wasm/src/engine.rs#L314))
  and then answers HTTP 503. Today `MAX_CLIENT_CONCURRENCY` = 3, and a search
  makes up to 3 simultaneous HTTP requests, each holding a permit for up to 2 s.
  After A4 it makes 1 request that holds 1 permit for about 2 s (P8).

### 3.4 The Hub

- [search.ts](../../../../crates/roym_web/ui/src/directory/search.ts) has
  `startRun` (:61), `queryOneSource` (:88), and `runSearch` (:157), the loop
  with a worker pool bounded by `max_concurrency`, one retry of `not-started`
  sources, and the `ignoreConcurrency` option (:154). It also has
  `runEnvelope` (:213). `SourceOutcomeKind` includes `not-started`.
- [screens/directory.ts:235-330](../../../../crates/roym_web/ui/src/screens/directory.ts#L235)
  calls `runSearch`, redraws a partial merge after every source
  (`refreshPartial`, which calls `directory.merge`), and writes the progress
  line "N of M directories answered". `runEnvelope` is used at :404.
- [main.ts:48-69](../../../../crates/roym_web/ui/src/main.ts#L48) exposes
  `window.RoymDirectory = { runSearch }` for one test (case 23b).
- [search.test.ts](../../../../crates/roym_web/ui/src/directory/search.test.ts)
  tests the loop with a stub `fetch`.

### 3.5 `roymctl`

[find.rs](../../../../apps/roymctl/src/commands/roym/find.rs) has its own copy
of the loop: `start_find_run` (:129), `query_source` (:163),
`run_source_queries` (:192, a semaphore bound by `max_concurrency` and one
serial retry of 503), and `print_find_results` (:263, which calls
`directory.merge`). It has two `#[allow(clippy::too_many_arguments)]`. It
prints one note per failed source, then the hits with one membership line
per source, then the refused hits. The unit test at
[tests.rs:43](../../../../apps/roymctl/src/commands/roym/tests.rs#L43) only
parses the arguments.

### 3.6 Tests and fixtures that drive the old loop

| Where | What | Fate |
| --- | --- | --- |
| [dual_build_parity/fixtures.rs:636](../../../../crates/roym_web/tests/dual_build_parity/fixtures.rs#L636) | `fan_out_one`, `fan_out`: 17 call sites in `directory.rs`, `trust.rs`, `trust_sources.rs` | Rewrite the two helpers to call `directory.find`. Call sites keep working. |
| `dual_build_parity/directory.rs` scenarios 97, 101, 102, 116, 119 (:448-552, :801, :1007), 120 (:1057) | Call `start-run`, `query-source` and `merge` directly | 97, 101, 119, 120: rewrite onto `find`. 102 and 116 test properties of the removed verbs (unregistered source, foreign run id; two `query-source` calls for one source): rewrite or replace (section 7). 106b (:573) lists the verbs: update the list. |
| `dual_build_parity/directory_compat.rs:20` | `start-run` then `query-source` with a newer node's answer | Rewrite onto `find`. |
| `dual_build_parity/directory.rs:1116` | `directory_source_timeout_and_concurrency_fit_the_real_sandbox_defaults` | Rewrite (Q2). |
| `method_contract/shapes_directory.rs:251-297` and `method_shapes/directory_{start_run,query_source_*,merge}.json` | Records output shapes of the three verbs | Replace by `directory_find_*.json`. |
| `method_contract/contract_directory.rs:86-110` | Invalid `open_to` in `query-source`; `merge` | Move onto `find`. |
| [substrate/tests/common/roym_flow.rs:56](../../../../crates/substrate/tests/common/roym_flow.rs#L56) | `run_client_loop`: 4 users in `roym_directory_e2e.rs`, 2 in `roym_trust_e2e.rs` | Rewrite the helper to call `directory.find`. Keep its return type. |
| `roym_directory_e2e.rs:18, 246, 412-445` | Doc comment, `run-envelope`, the MAX_SOURCES run with 6 unreachable sources | Update the comment. The 8-source run is now one batch of 8. |
| [roym-hub.spec.ts:763](../../../../crates/substrate/tests/e2e/tests/roym-hub.spec.ts#L763), :796, :830 | Cases 22 (progressive render), 23 (8 sources, no 503), 23b (`ignoreConcurrency`) | 22: reword to one call. 23: keep. 23b: delete (no hook, no 503 per source). |
| [search.test.ts](../../../../crates/roym_web/ui/src/directory/search.test.ts) | Loop tests | Replace by tests of `findListings`. |

The parity harness has a fake foreign source layer in
`trust_harness::canned_source_response` (a synchronous function called at the
top of both `ServiceProxy::invoke` implementations,
[helpers.rs:787 and :872](../../../../crates/roym_web/tests/dual_build_parity/helpers.rs#L787))
and a `foreign_wire_route` that runs a real second directory on each build.
A target whose name contains `timeout` returns `ProxyError::Timeout` at once
([helpers.rs:821](../../../../crates/roym_web/tests/dual_build_parity/helpers.rs#L821)).
Nothing in the harness is slow. Step 5 adds a slow target.

### 3.7 Size and ratchet facts

- `directory.rs` (parity) is on the oversized list at 1 135 lines, `helpers.rs`
  at 1 429 ([oversized-test-files.txt:21-22](../../../../xtask/oversized-test-files.txt#L21)).
  A listed file may not grow. Removing scenarios lowers `directory.rs`, and
  the list must be lowered in the same commit. `helpers.rs` must not grow:
  Step 5 changes it by zero lines.
- `fixtures.rs` is 787 lines, 13 below the cap. Rewriting `fan_out_one`
  shortens it.
- `client_query.rs` is 468 lines and `client_merge.rs` 341. Both shrink or
  disappear.

### 3.8 What no code does yet

- No code reads the `call-all` limits. A3's backlog row "A guest cannot read
  the `call-all` limits" has target A4. A4 does not need them (section 5,
  D-A4-6) and re-targets the row.
- No Roym service calls `call_all`.

## 4. Behaviour-change audit

Every change below is listed. Everything not listed stays the same.

### Wire changes

| # | Change | Who sees it |
| --- | --- | --- |
| W1 | `directory.start-run`, `directory.query-source` and `directory.merge` are removed. They answer `-32601` locally and `-32013` over the wire (P13). | Hub, `roymctl`, tests. |
| W2 | New local-only method `directory.find`. Input `{ "query": SearchQuery }` (`query` optional, default `{}`; top-level unknown fields refused). Output `{ run_id, outcomes: [{source, verified, refused, truncated, error}], hits, hits_truncated, refused, refused_truncated }`. | Hub, `roymctl`, tests. |
| W3 | An invalid query field (for example `open_to`) is refused by `directory.find` with `-32602` and `data.field` = `query.open_to`. Before, `query-source` refused it as `open_to`. | Clients, tests (B46). |
| W4 | `directory.run-envelope` has the same input and output. It finds only records that are on the page `find` returned (Q3 (a)). Before, it found any verified row of the run, including a hit that did not reach the page. | Hub button, e2e. |
| W5 | The `outcomes` list is in source order (sorted by DID). Before, a client got outcomes in the order the sources answered. | `roymctl` output order of source notes. |
| W6 | `max_concurrency` is gone from every reply. | Hub, `roymctl`. |
| W7 | `verified` and `refused` in an outcome count what passed checking and the per-source cap, not what was stored successfully. | Hub, `roymctl` (no practical change). |
| W8 | The `directory.search` request that goes to each source is byte-for-byte the same: `["{\"method\":\"directory.search\",\"params\":<query>}"]`, interface `directory`, method `invoke`, `idempotent`, 2 000 ms. | Source nodes (no change). |

### Behaviour changes that are not wire changes

| # | Change |
| --- | --- |
| B1 | All sources are asked at the same time (8 at once at the default limit). Before, a client asked 3 at a time. A search with 8 slow sources takes about 2 s, not about 6 s. |
| B2 | One request to `web` holds 1 guest-HTTP admission permit for about 2 s (down from up to 3 permits before). A fifth simultaneous request waits up to 2 s and gets 503 if no permit frees (P8). |
| B3 | One search holds one `call-all` waiter permit (node-wide limit 4 by default). A fifth simultaneous search gets `not-started` for every source (Q4). |
| B4 | The Hub draws the page once, when the call returns. It no longer shows partial results or "N of M directories answered". |
| B5 | The Hub does not retry a busy node. The person presses **Search** again. `roymctl` does not retry either. |
| B6 | The source rows (`last_ok_secs`, `last_error`) and the held membership evidence are written after the batch, not after each source. `not-started` never writes `last_error`. |
| B7 | Evidence is remembered for each counted verified hit under the per-source cap in order, exactly as today's `store_verified_hit` loop does (P4). Hits over the cap are checked and dropped with no evidence kept. |
| B8 | The run row has no `sources` list. `RUNS` exists only for the id and retention pruning (P9). Run ids are not unique under concurrent finds (backlog row). |
| B9 | Refused hits are not stored. Nothing reads them after the merge. |
| B10 | A search reply whose `truncated`, `directory` or `answered_at_secs` has the wrong type is `unreadable` (before: read as false/ignored), a wire-reading change of remote answers (D-A4-16, P16). |

### Stays the same

The server half. The request each source receives. The checking of every
record. The membership verdicts. The merge rules and the page order. All caps
(10 per source on the page, 50 on the page, 20 refused, 20 stored per source,
40 read per source). The outcome kinds and their words. `SourceError`. The
1-hour pruning of runs. `run-envelope`'s wire shape. The run id format. The
export bundle.

## 5. Decisions this plan makes

Decisions are numbered D-A4-n. They go into the change doc at close-out
(section 9). Q1 to Q6 are the owner's. The others are made here.

**D-A4-1. The method (Q1 (a)).** `directory.find` in the method table of
`roym_directory`. `FindInput { #[serde(default)] query: SearchQuery }` with
`#[serde(deny_unknown_fields)]`. `SearchQuery` itself stays tolerant of
unknown fields, because a directory on the wire reads it (D-A2c-3).
`FindOutput { run_id: String, #[serde(flatten)] page: Page }` is defined in
`roym_directory` (`client_find.rs`); `Page` comes from the library (D-A4-3).
`find` is not in `WIRE_REACHABLE`, so it is local-only. Check in Step 4 that
the shape recorder and `assert_service_table` accept a `flatten` field in an
output. (`flatten` only conflicts with `deny_unknown_fields`, which an output
never has, D-A2a-4.) If they do not, write the six fields out in `FindOutput`.

**D-A4-2. Removed methods.** `directory.start-run`, `directory.query-source`
and `directory.merge` leave the method table, with their input and output
types, `RunInput`, `StartRunOutput`, `QuerySourceInput`, `QuerySourceOutput`,
`MergeOutput`, `MAX_CLIENT_CONCURRENCY`, the `validate_run_and_source`
check, and their shapes. No alias, no deprecation (the product is unreleased).
`directory.run-envelope` stays.

**D-A4-3. The library.** `crates/roym_federated_query`, package
`syneroym-roym-federated-query`. Dependencies: `syneroym-app-host`,
`syneroym-roym-core`, `serde`, `serde_json`. Dev dependencies for tests only:
`tokio`, `syneroym-identity`, `syneroym-signed-record`, `z32` (to sign a test
listing, the same 20 lines as
[listing/tests.rs:10](../../../../crates/roym_core/src/listing/tests.rs#L10)).
It builds for native and for `wasm32-wasip2`. It is generic over
`P: AppProxy`, not `AppHost`. It does not read or write the data layer, does
not read a clock (the caller passes `now_secs`) and does no logging. This
keeps it testable with a three-method fake.

To prevent a hostile or large source from exhausting guest memory, the library
enforces two guards (P1):
1. `text.len()` is checked against `MAX_SEARCH_REPLY_BYTES = 8 * 1024 * 1024` (8 MiB)
   **before** JSON parsing. The cap is derived from the theoretical maximum reply:
   `MAX_HITS_PER_QUERY` (50) × (`MAX_PAYLOAD_BYTES` (64 KiB) + evidence bound from
   `MAX_EVIDENCE_CREDENTIALS` (4 credentials × ~3.5 KiB) + `MAX_EVIDENCE_DECISIONS`
   (8 decisions × ~3.5 KiB, with `MAX_REASON_LEN` 2 048 and `MAX_RULE_LEN` 256) + ~1 KiB
   envelope wrapper ≈ 107 KiB raw per hit) × 1.15 JSON escaping factor ≈ 6.0 MiB, plus
   a 2 MiB safety margin = 8 MiB (P1). An honest reply with 50 maximum-evidence hits
   fits under 8 MiB (tested in §7.1); a reply over 8 MiB becomes
   `error: Unreadable { reason: "reply too large" }` with no parse.
2. Replies are taken by value (`into_iter`) and each reply string is dropped as
   soon as its hits are read, avoiding keeping all 8 raw responses in memory.

```rust
pub struct Source {
    pub did: String,
    /// The issuer pinned for this source, if any (trust on first use).
    pub pinned_issuer: Option<String>,
}

/// One `call-all` batch to every source, then checking and merging.
/// `Err` only when `sources` is longer than `MAX_SOURCES`.
pub async fn find<P: AppProxy>(
    proxy: &P,
    sources: &[Source],
    query: &SearchQuery,
    now_secs: u64,
) -> Result<Gathered, FindError>;

pub struct Gathered {
    pub page: Page,
    /// The signed bytes of every record named on the page: `(record_id,
    /// envelope)`, deduplicated by `record_id`. The caller keeps them.
    pub page_envelopes: Vec<(String, String)>,
    /// Membership evidence of every counted verified hit under the per-source
    /// cap, in order. The caller remembers them.
    pub evidence: Vec<HeldEvidence>,
}

pub struct Page {            // Serialize + Deserialize, no deny_unknown_fields
    pub outcomes: Vec<SourceReport>,
    pub hits: Vec<MergedHit>,
    pub hits_truncated: bool,
    pub refused: Vec<RefusedHit>,
    pub refused_truncated: bool,
}

pub struct SourceReport {
    pub source: String,
    pub verified: u32,
    pub refused: u32,
    pub truncated: bool,
    pub error: Option<SourceError>,   // roym_core::directory::SourceError
}

pub struct HeldEvidence {
    pub source: String,
    pub member_did: String,
    pub pinned_issuer: Option<String>,
    pub evidence: MembershipEvidence,
}
```

`MergedHit`, `HitSource` and `RefusedHit` move from
`client_merge.rs` with the same fields and serde names. The library keeps a
`BTreeMap<String, String>` of `(record_id, envelope)` beside the rows during
checking; `page_envelopes` is read from it after the merge (P4). Files in the
crate: `src/lib.rs` (the entry and types; `src/types.rs` if types grow),
`src/gather.rs` (request building, error mapping, reply parsing), `src/verify.rs`
(verification and rows), `src/merge.rs` (the moved merge code), `src/tests.rs`
and `src/tests/{merge,find,verify}.rs` (section 7, P14). Each is well under the
file limit.

**D-A4-4. One batch, one wave, no chunking.** `find` sends one `call_all` with
one request per source, in input order. Each request:
`CallTarget::Service(did)`, interface `services::DIRECTORY.interface`, method
`"invoke"`, params `json!([{"method":"directory.search","params":query}
.to_string()])`, options `{ idempotent: true, timeout_ms:
Some(DEFAULT_SOURCE_TIMEOUT_MS) }` (the same bytes as
[client_query.rs:155-169](../../../../crates/roym_directory/src/app/client_query.rs#L155)).
No chunking, because `MAX_SOURCES` (8) is below the host maximum (16), and the
directory caps sources when they are added
([client_sources.rs:176](../../../../crates/roym_directory/src/app/client_sources.rs#L176)).
A list longer than `MAX_SOURCES` is a caller error: `FindError::TooManySources`,
and the directory answers `-32603`. An empty list makes no call and returns an
empty page (`call_all([])` is `Ok([])`, but `find` returns before calling).

**D-A4-5. Outcome mapping.** The mapping from the old `map_proxy_error` and
`parse_source_response`:

| Result of one call | Outcome |
| --- | --- |
| `Err(ServiceNotFound)` or `Err(DependencyNotBound)` | `error: NotFound` |
| `Err(TimedOut)` | `error: TimedOut` |
| `Err(Callee(c))` | `error: Refused { code: c.code, message }` |
| any other `Err` | `error: Refused { code: -32603, message: format!("{other:?}") }` |
| `Ok(text)` with `text.len() > MAX_SEARCH_REPLY_BYTES` | `error: Unreadable { reason: "reply too large" }` (P1) |
| `Ok(text)` that is not a `Response` | `error: Unreadable` |
| `Ok` with a JSON-RPC `error` and no `result` | `error: Refused { code, message }` |
| `Ok` with a `result` whose `hits` cannot be read | `error: Unreadable` |
| `Ok` with a good `result` | no error; `truncated` from `result.truncated` |
| The **outer** `Err` (`Internal` or `UnsupportedTarget`) | every source: `error: NotStarted` (P3, P17) |

**D-A4-6. Checking is unchanged.** For each source, in the order the source
returned: parse the reply into `SearchOutput` (D-A4-16, P12), truncate to 40
hits, then `listing::verify_envelope(&hit.envelope, now)` for each. A verified
hit counts against the cap of 20 and gets a membership verdict from
`membership::evaluate` with the source's pinned issuer, exactly as in
`store_verified_hit`. A refused hit counts against 20. A hit over its cap is
checked and dropped, as today (the check is the same cost; the order of checks
is the same). Evidence and page envelopes are kept only for counted verified
hits under the cap in order (P4). The library does not read the `call-all`
limits (the backlog row is re-targeted, section 8).

**D-A4-7. The merge moves unchanged.** The functions
`dedupe_and_cap_verified`, `group_and_sort_refused`, `round_robin_merge`,
`candidates_by_listing`, `build_hits`, `build_refused` move to `merge.rs`
with their comments. Their input type changes from `SearchRunRow` to a
smaller `Row` (the fields the merge reads; no `run_id`, `at_secs`, `envelope`
or `refused`). The merge works on rows in memory. It has no data layer. The
existing parity scenarios (98, 102c, 102d, 119) prove the result is the same
(D-A4-15).

**D-A4-8. What `directory.find` does (the service side).** In order:

1. `begin_run`: create the `runs` and `search_runs` collections, prune both
   by `at_secs` older than `RUN_RETENTION_SECS`, mint `run_{secs}_{count}`,
   store `RunRow { at_secs }`. `RUNS` exists only for the id and retention
   pruning; run ids are not unique under concurrent finds (P9).
2. Read the sources: `SourceRow`s, sorted by DID, as `federated_query::Source`
   (`did`, `issuer_did` as `pinned_issuer`).
3. `federated_query::find(host, &sources, &query, now_secs)`. Outer
   batch refusals are mapped to `not-started` without guest logging; the cause is
   observed host-side via `substrate.proxy.call_all.refused{reason}` (P3).
4. For each report: `mark_source_ok` when there is no error; `record_source_error`
   when there is an error **other than** `NotStarted`.
5. For each `HeldEvidence` in order: `held::remember` (P4, best effort, as today).
6. For each page envelope: store a `RunEnvelopeRow { run_id, record_id,
   envelope, at_secs }` under key `{run_id}#{record_id}` in `search_runs`
   (Q3 (a)), best effort.
7. Return `FindOutput { run_id, page }`.

The handler stays under 100 lines by moving steps 4 to 6 into one
`persist_find_results` function. New files in `roym_directory`:
`app/client_find.rs` (steps 1 to 3 and 7) and `app/client_runs.rs`
(`begin_run`, `persist_find_results`, `run_envelope`, `RunEnvelopeRow`).
`client_merge.rs` and `client_query.rs` are deleted. `held.rs` and
`client_sources.rs` import what they use from the new files.

**D-A4-9. `run-envelope` reads `RunEnvelopeRow`.** The input and output are
unchanged. The row struct has no `deny_unknown_fields`, so an older row from
a developer's database still parses. Older rows are pruned within the hour.

**D-A4-10. `NotStarted` stays, as a unit variant.** It is now produced by the
library when the batch is refused. The Hub shows the neutral line "This installation
could not start the search. Press Search to try again." when every outcome is
`not-started` (P3). The batch refusal cause is observed host-side via
`substrate.proxy.call_all.refused{reason}` (D-A3-13, P3); guest instances cannot
emit logs. `record_source_error` is never called for it: the choice is
one small pure function in `client_runs.rs`, `errors_to_record(outcomes)`,
which skips `NotStarted`, and a unit test fixes it (case 23b checks the same
rule in a browser today and is deleted).

**D-A4-11. The time model (Q2 (a)).** One wave of calls with 2 000 ms each
ends within about 2 s plus the 250 ms cancel grace
(`CALL_CANCEL_GRACE`, [capabilities_call_all.rs:19](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_call_all.rs#L19)).
Then `directory` checks up to 8 × 40 envelopes and stores the page's rows.
`DISPATCH_HEADROOM_MS` (1 500) is the allowance for that work. The existing
test is rewritten as `find_time_fits_the_real_sandbox_defaults`:

```text
waves = ceil(MAX_SOURCES / call_all_max_concurrency)
wave_time = DEFAULT_SOURCE_TIMEOUT_MS + 250 // CALL_CANCEL_GRACE
assert  MAX_SOURCES <= call_all_max_calls
assert  DEFAULT_SOURCE_TIMEOUT_MS <= call_all_max_call_timeout_ms
assert  waves * wave_time <= call_all_total_timeout_ms
assert  waves * wave_time + DISPATCH_HEADROOM_MS
            < dispatch_epoch_timeout_secs * 1000
```

It reads the real `AppSandboxRole::default()`. In the shared parity run, the
ceiling test (`find_ceiling_run_of_eight_sources_fits_the_dispatch_budget`)
checks correctness only (all 8 sources `ok`, 20 verified per source, the
merged page) and prints the measured wall time. To protect against CI
wall-clock flakiness under parallel load and the 120 s harness epoch (P19),
the headroom bound check is placed in an isolated measurement test
(`find_ceiling_measure_fits_dispatch_headroom`, marked `#[ignore]`). It
measures the wall time of the whole `directory.find` call on the WASM build
when all 8 sources answer at once without delay with 40 genuine signed
listings carrying full membership evidence (4 credentials, 4 revocations,
8 decisions), full evidence writes via `held::remember`, and realistic
maximum payload sizes, minus nothing (P18). Step 5 (4) and Step 9 run this
test isolated (`cargo nextest run -p syneroym-roym-web --test dual_build_parity -- --run-ignored only -j 1`),
assert wall time is below `DISPATCH_HEADROOM_MS`, run 20 times in a loop,
and record the worst value in the PR description (P10, P19).

**D-A4-12. Clients.** The Hub gets `findListings(query)` in `search.ts`:
one `call("directory.find", {query})` and a mapping from `outcome.error?.kind`
to the existing `SourceOutcomeKind`. A thrown `RpcError` with code 503 (the
`web` door is full) becomes a page-level notice: "This installation could not
start the search. Press Search to try again." (P3, P11). Any other thrown
error (such as `-32602` for an invalid query field, `-32603` for `TooManySources`
or data errors, or an HTTP 500 when `web` or `directory` traps) clears
`Searching...`, shows one page-level error line: "The search failed: <message>.
Press Search to try again.", and re-enables the Search button (P20).
Hub string contracts (P11, P20):
- While waiting: `Searching...`
- Final line: `N directories searched` (or `You have not added any directories to search.`)
- Refusal notice: `This installation could not start the search. Press Search to try again.`
- Failure notice: `The search failed: <message>. Press Search to try again.`
- Help text in `screens/directory.ts` removes "one directory at a time in turn"
- The per-DID line map (`sourceLines`) and retry line map are removed.

`runSearch`, `startRun`, `queryOneSource`, `RunSearchOptions`,
`ignoreConcurrency`, `SearchProgress`, `refreshPartial` and `window.RoymDirectory`
are removed. `roymctl` makes one `rpc_call("directory.find", {query})` and
prints the same lines. On error from `rpc_call`, `roymctl` prints the error
to stderr and exits with a non-zero code (P20). Its semaphore, `JoinSet`, retry
and the two `too_many_arguments` allows are removed. Neither client imports
the structs (D-A2a-13).

**D-A4-13. Tests move in a safe order (Steps 4 to 7).** First `find` is added
beside the old verbs, with an equivalence test that runs both paths on the
same harness and compares the page. Then the clients and helpers move to
`find`. Only then are the old verbs and their tests deleted. Every step
builds and passes.

**D-A4-14. The slow source (OF-6) and ceiling fakes.**
`trust_harness::canned_source_response` becomes `async` and gains
`did:key:hSlow`. `hSlow` inspects the method inside `params`: it sleeps 5 s
only when the method is `directory.search`; for `directory.info` it answers
immediately with a normal info response (P5). Both proxies call that function
at the top of `invoke`; adding `.await` does not change the line count of
`helpers.rs`. The host cuts the search call at 2 s, so the outcome is `timed-out`.
The test asserts the other sources' hits, the `timed-out` outcome for the slow
DID, and that the whole call ended before `hSlow`'s 5 s sleep (so the caller
did not wait for the slow source), without asserting a rigid 4 s wall-clock
bound (P19), on both builds.

For the ceiling scenario, add 8 canned sources `hCeiling1..8` in `trust_harness.rs`
(P5, P18). Each answers `directory.info` with an `issuer_did` (a test SynOrg key,
as `trust_harness` does for `hTrustValid`) so `add-source` pins the issuer (P18).
Each answers `directory.search` with 40 genuine signed listings built once, each
carrying maximum evidence signed by that issuer: 4 valid credentials, 4 revocations,
and 8 decisions, with maximum field lengths (P18). A shared builder helper is used
for this and for the 50-hit size test (§7.1) so both follow the real limits (P18).
For scenario 116 replacement (P6, P16), add canned source `hTwoVersions`: answers
`directory.info` with a name, and `directory.search` with two genuine signed
versions of one listing in one reply. For the oversized reply test (P1, P16), add
canned source `hOversized`: answers `directory.info` with a name, and
`directory.search` with a payload exceeding `MAX_SEARCH_REPLY_BYTES` (8 MiB).
Confirm `trust_harness.rs` stays under 800 lines (P5).

**D-A4-15. Equivalence before deletion.** Step 4's test
`find_gives_the_page_the_old_loop_gave` drives the old loop
(`start-run` → one `query-source` per source → `merge`) and `find` on each
build for the scenarios 97, 98, 102c, 102d, 119 and 120, and asserts the
`hits`, `hits_truncated`, `refused`, `refused_truncated` are equal (after
normalizing `age_secs`, which depends on the clock), and that each source's
`verified`, `refused`, `truncated` and `error` equal the old `query-source`
reply. The test is deleted in Step 7 with the old verbs. Its value is that
the merge move and the check move are proved before the old code goes.

**D-A4-16. Typed remote search reply (P12).** Remote `directory.search` answers
are parsed into `SearchOutput` from `roym_core::method::directory` (fulfilling
D-A2c-10 and D-A2c-13). A non-boolean `truncated` field fails typed parsing as
`Unreadable` (tested in library unit tests). Truncation to 40 hits applies
immediately after parsing, before envelope checking.

## 6. Step-by-step changes

Each step builds and its tests pass. The whole branch is one PR. Rebuild the
WASM components (`mise run build:roym`) after every step that changes
`roym_directory` or the library (Steps 2 to 4 and 7).

### Step 1 — Commit this plan

`docs(planning): plan M08A slice A4`. Status row A4: "Plan written".

### Step 2 — The crate and the moved merge

Behaviour does not change.

1. Add `crates/roym_federated_query` with `Cargo.toml` (lints from the
   workspace, `[lib]` default crate type), `src/lib.rs`, `src/merge.rs`,
   `src/tests.rs` and `src/tests/` (following the module-layout rule, P14). Add
   `syneroym-roym-federated-query = { path = "crates/roym_federated_query" }`
   next to the other Roym crates in the root
   [Cargo.toml:95-101](../../../../Cargo.toml#L95). Dependencies as D-A4-3.
   Versions are `major.minor`. Run `cargo xtask check-dep-versions`.
   `cargo xtask check-roym-deps` scans every `crates/roym_*` directory and
   refuses a dependency that is not on its allowed list
   ([xtask/src/main.rs:154](../../../../xtask/src/main.rs#L154), the list `target_independent`).
   `roym_directory` now depends on the library, so add
   `"syneroym-roym-federated-query"` to `target_independent` there. The
   library's own dependencies are all on the list already. Run the check.
2. Move the merge functions and the types `MergedHit`, `HitSource`,
   `RefusedHit` into `merge.rs` with their comments. Add `Row` and one
   public function `pub fn merge_rows(verified: Vec<Row>, refused: Vec<Row>,
   now_secs: u64) -> MergedRows` that returns `hits`, `hits_truncated`,
   `refused`, `refused_truncated`. Take the clock as an argument
   (`build_hits` already takes `now`).
3. In `client_merge.rs`, make `merge` map its `SearchRunRow`s to `Row` and
   call `merge_rows`. Delete the moved functions from `client_merge.rs`.
4. Unit tests in the library for the merge (section 7.1). They use `Row`
   values, no signing.

Check: all existing parity scenarios, `method_contract`, `roym_directory`
tests pass unchanged. `cargo shear` finds no unused dependency.

### Step 3 — The library's fan-out and checking

Behaviour does not change; nothing calls it yet.

1. `gather.rs`: build the `CallRequest` list (D-A4-4), check `text.len() <=
   MAX_SEARCH_REPLY_BYTES` before parsing (P1), parse `SearchOutput` (D-A4-16,
   P12), map results to per-source replies and map outer `Err` to `NotStarted`
   (D-A4-5, P3, P17). Drop each reply text as soon as parsed (`into_iter`, P1).
   `verify.rs`: the check and the caps (D-A4-6) producing `Row`s,
   `BTreeMap<String, String>` for page envelopes, and `HeldEvidence` for
   counted verified hits under the cap (P4). `lib.rs`: `find`, `Gathered`,
   `Page`, `SourceReport`, `HeldEvidence`, `FindError`.
2. Move `map_proxy_error` and `parse_source_response` logic here. Leave the
   old copies in `client_query.rs` until Step 7. (A short-lived copy is
   acceptable only because Step 7 deletes it; the duplication gate is
   checked at the end.)
3. Unit tests with a fake `AppProxy` (section 7.1): request bytes, outcome
   mapping, caps, truncation before checking, oversized replies and 50
   maximum-evidence hits (P1), typed `SearchOutput` parsing (P12), outer
   refusal for `Internal` and `UnsupportedTarget` (mapping every source to
   `NotStarted`, P3, P17), evidence collected only under cap (P4).

Check: library tests pass; `cargo clippy -p syneroym-roym-federated-query`
is clean; `cargo build --target wasm32-wasip2 -p
syneroym-roym-federated-query` builds (the library is part of the
`directory` component build).

### Step 4 — `directory.find`, the equivalence test and the shapes

1. `client_find.rs` and `client_runs.rs` (D-A4-8, D-A4-9). Add the table row
   `"directory.find": client_find::find [client_find::FindInput =>
   client_find::FindOutput]` to [api.rs](../../../../crates/roym_directory/src/api.rs).
   Keep the old rows. `begin_run` is `start_run` without the output; `RUNS`
   exists only for the id and retention pruning (P9). Outer batch refusal
   causes are observed host-side via `substrate.proxy.call_all.refused{reason}` (P3).
2. `run_envelope` reads `RunEnvelopeRow`. For this step the old verbs still
   write full `SearchRunRow`s: `RunEnvelopeRow` ignores their extra fields,
   and the `refused: true` rows that old `query-source` writes carry the
   hostile `record_id` problem. So in this step `run_envelope` keeps the
   `!row.refused` check by reading `#[serde(default)] refused: bool`. Step 7
   removes the field with the old verbs.
3. Parity: new file `dual_build_parity/directory_find.rs` (declared in
   [dual_build_parity.rs](../../../../crates/roym_web/tests/dual_build_parity.rs)).
   It has the equivalence test (D-A4-15) and the first `find` scenarios
   (section 7.2).
4. Method contract: `shapes_directory.rs` records `directory_find_*.json`
   with `UPDATE_METHOD_SHAPES=1`, then asserts they are made of the old
   shapes: each `outcomes` item equals a recorded `directory_query_source_*`
   shape, and `hits`/`refused` equal the `directory_merge` shape. Add `find`
   to the directory table test (`every_directory_method_has_typed_input_and_output`;
   37 + 1 rows).
5. Rebuild WASM. Run the directory parity and contract tests.

### Step 5 — The slow source, the headroom test and the measure

1. `trust_harness::canned_source_response` becomes `async fn`; add
   `did:key:hSlow` (D-A4-14, P5). Add `.await` at its two call sites. Confirm
   `helpers.rs` has the same line count (`cargo xtask check-file-lengths`).
   Add `hCeiling1..8` (with pinned issuer and full membership evidence, P18),
   `hTwoVersions` (P6, P16), and `hOversized` (P1, P16) canned sources to
   `trust_harness.rs`.
2. Scenarios in `directory_find.rs`: the slow source (OF-6, asserting finished
   before `hSlow`'s 5 s sleep, P19), the whole batch refused (needs a way to
   refuse: see section 7.2), the oversized reply with `hOversized` (P1), and the
   ceiling correctness scenario (8 sources, 40 genuine hits each with full
   evidence, clean of oversized replies, asserting correctness and printing time,
   P1, P2, P5, P18, P19). Add isolated measurement test
   `find_ceiling_measure_fits_dispatch_headroom` marked `#[ignore]` (P19).
3. Rewrite `directory_source_timeout_and_concurrency_fit_the_real_sandbox_defaults`
   as `find_time_fits_the_real_sandbox_defaults` (D-A4-11). Count a wave as
   `DEFAULT_SOURCE_TIMEOUT_MS + 250` (cancel grace, `CALL_CANCEL_GRACE`, P19).
   Remove `MAX_CLIENT_CONCURRENCY` from its imports; note that the old test's
   second assertion (`MAX_CLIENT_CONCURRENCY < admission limit`) is dropped on
   purpose because client concurrency is now 1 (P8).
4. Measure the wall time of the whole `directory.find` call in the ceiling
   scenario on the WASM build via the isolated measurement test
   (`cargo nextest run -p syneroym-roym-web --test dual_build_parity -- --run-ignored only -j 1`,
   P10, P19). Record the measured time in the PR description. If it is above
   60 % of `DISPATCH_HEADROOM_MS`, apply the fallback in section 7.6 before going on.

### Step 6 — Move the clients and test helpers to `find`

1. Hub: `search.ts` (D-A4-12, handles 503 and non-503 errors, P20),
   `screens/directory.ts` (one call, one redraw, exact string contracts from
   D-A4-12 including error notices, help text update, P11, P20), `main.ts`
   (remove `RoymDirectory`), `search.test.ts` (new tests including non-503
   failure handling, section 7.4, P20).
2. `roymctl`: `find.rs` (D-A4-12). Keep the output text; on RPC error, print
   error to stderr and exit non-zero (P20). Add a unit test for printing
   functions and error handling with a canned `find` reply (section 7.4, P20).
3. `fixtures.rs`: `fan_out_one` calls `directory.find` once, splits `run_id`
   off the reply, asserts `directory.sources` equals `sources`, and returns
   `(run_id, page)` keeping the `{"result": …}` wrapper expected by call sites
   (P7). `roym_flow.rs`: `run_client_loop` calls `directory.find` and returns
   `(run_id, page)` similarly (P7).
4. Rewrite parity scenarios 97, 101, 119, 120 and `directory_compat.rs` onto
   `find`. Move the `contract_directory.rs` cases (W3).
5. Hub e2e: rename case 22 ("the results view shows the loopback hit and names
   the unreachable sources", P21), case 23 checks that the new start notice text
   is absent from the whole page and keeps the 503 check (P21), case 23b deleted
   ([roym-hub.spec.ts:830](../../../../crates/substrate/tests/e2e/tests/roym-hub.spec.ts#L830)).
   The old verbs still exist, so a failure in this step shows only client or
   helper errors.

### Step 7 — Delete the old verbs

1. Remove the three rows from `api.rs`, `client_query.rs`, the `merge`
   function and its types, `client_merge.rs`, the rows' shape files, the
   old scenarios 102 and 116 (replaced, section 7.2), the equivalence test,
   and `MAX_CLIENT_CONCURRENCY`. Remove the `#[serde(default)] refused` field
   from `RunEnvelopeRow`. Update scenario 106b's verb list and
   `all_directory_verbs()` (37 methods, minus 3 removed, plus `find`: 35).
2. Remove the Hub's `refreshPartial` leftovers, and `NOT_STARTED` retry code
   if any remains. Remove `ignoreConcurrency`.
3. Ratchets: set `directory.rs` in `xtask/oversized-test-files.txt` to its new
   line count; run `cargo xtask check-file-lengths`, `check-duplication`
   (lower `MAX_EXACT_DUPLICATE_PERCENT` if it asks),
   `check-lint-suppressions`.
4. Run `rg -n "query-source|start-run|directory\.merge|max_concurrency|MAX_CLIENT_CONCURRENCY"`
   over `crates apps` and fix every hit that is not about `call_all_max_concurrency` (P13).
   Update dead constants and stale comments in `roym_core/directory.rs`
   (`SourceError::NotStarted`, `DEFAULT_SOURCE_TIMEOUT_MS`, `DISPATCH_HEADROOM_MS`,
   `MAX_SOURCES`), `app.rs:124`, `helpers.rs:264`, `trust_harness.rs:18, 43`,
   and Hub doc comments (P13).
5. Rebuild WASM. Run everything for the crates touched (section 7.5).

### Step 8 — Docs and close-out

Section 9. Backlog (section 8). Change doc: tick the task, fill the Test
column of OF-6 and OF-7, update counts in the OF-14, OF-15 and OF-18 Test
columns from the real files after Step 7 (count shape files with
`ls crates/roym_web/tests/method_shapes | wc -l`, P23), rewrite the Scope line
to state the three-call form is removed (P22), add D-A4-1 to D-A4-16 and the
owner's answers, add deviations. Update `AGENTS.md` (Architecture) sentence on
`CallTarget::Service` to name the federated query library (P22). Update
`system-architecture.md:1353` for one `find` call, all sources queried at once,
no Hub retry (P22). `task.md` result line. `status.md` row.

### Step 9 — Finish

- `cargo +nightly-2026-04-06 fmt --all`, clippy clean, the import clean-up
  pass over every edited file.
- `cargo dupes report` over the diff (the library keeps one copy of the merge
  and of the error mapping; the Step 3 copies in `client_query.rs` are gone).
- `mise run verify` as a background job, no `--skip`.
- Run the new parity file, Hub cases 22 and 23, and the isolated ceiling
  measure test (`find_ceiling_measure_fits_dispatch_headroom`, `--run-ignored only -j 1`)
  20 times. Assert the isolated measure fits within `DISPATCH_HEADROOM_MS`, and
  record worst ceiling measure in PR text (P10, P19).
- Open the PR (CLA line verbatim).

## 7. Tests

### 7.1 Library unit tests (`crates/roym_federated_query/src/tests.rs` and `src/tests/`, P14)

The fake `AppProxy` records the `CallRequest`s it receives and returns
scripted `Result<String, ProxyError>` entries (or an outer `Err`). A
`signed_listing()` helper signs a listing with a fresh key (the
`derive_listing_id` rule of `verify_envelope`).

`merge` (`src/tests/merge.rs`, rows only, no signing):
- `newer_version_wins_inside_one_source`
- `one_source_gets_at_most_its_share_of_the_page`
- `sources_take_turns_in_did_order_not_listing_id_order`
- `the_page_is_cut_at_the_total_cap_and_says_so`
- `the_same_listing_from_two_sources_is_one_hit_with_two_sources`
- `versions_differ_is_true_only_for_different_record_ids`
- `refused_hits_are_capped_apart_and_cannot_crowd_out_a_hit`
- `a_forged_listing_id_that_sorts_first_cannot_fill_the_refused_page`
- `an_empty_input_gives_an_empty_page`

`find` and `verify` (`src/tests/find.rs`, `src/tests/verify.rs`):
- `each_call_is_built_like_the_old_query_source` (target, interface, method,
  params bytes, options)
- `results_follow_the_source_order` (the fake answers in reverse)
- `a_missing_service_is_not_found` / `a_slow_call_is_timed_out` /
  `a_callee_error_is_refused_with_its_code` / `another_proxy_error_is_refused_32603`
- `a_reply_that_is_not_json_rpc_is_unreadable` / `a_json_rpc_error_is_refused`
  / `hits_that_do_not_parse_are_unreadable`
- `a_reply_over_the_size_cap_is_unreadable_and_does_not_stop_the_others` (P1)
- `fifty_hits_with_maximum_evidence_fit_under_size_cap_and_are_read` (builds a reply of
  50 hits with maximum payload and full evidence — 4 credentials, 4 revocations,
  8 decisions at maximum lengths — and asserts it fits under `MAX_SEARCH_REPLY_BYTES`
  and is read, P1, P18)
- `a_truncated_flag_that_is_not_a_boolean_is_unreadable` (P12)
- `the_result_truncated_flag_is_reported_per_source`
- `a_source_with_no_hits_still_reports_truncated`
- `at_most_forty_hits_are_read_before_checking`
- `a_tampered_envelope_is_refused_evidence_not_a_hit`
- `a_source_is_capped_at_twenty_verified_and_twenty_refused`
- `membership_uses_the_pinned_issuer_of_that_source`
- `evidence_is_kept_once_per_source_and_issuer_with_the_last_hit_winning`
- `evidence_of_a_hit_over_the_cap_is_not_kept` (P4)
- `page_envelopes_cover_every_source_of_every_hit_once`
- `a_refused_batch_marks_every_source_not_started` (tests both `Internal` and
  `UnsupportedTarget` refusals, P3)
- `no_sources_makes_no_call`
- `more_than_max_sources_is_an_error_and_no_call_runs`

### 7.2 Parity tests (`dual_build_parity/directory_find.rs`, both builds)

Every test runs both builds and compares the full answer. A test that reads
one build is the A2c R1 mistake.

- `find_gives_the_page_the_old_loop_gave` (D-A4-15; deleted in Step 7)
- `find_over_one_source_matches_the_native_and_wasm_builds` (replaces 97)
- `find_with_no_sources_is_an_empty_page_with_a_run_id` (replaces 101)
- `find_names_the_slow_source_and_keeps_the_other_hits` (OF-6, asserts call
  finishes before `hSlow`'s 5 s sleep and slow source is timed-out, P19)
- `find_marks_an_unreachable_source_not_found_and_keeps_the_others`
- `find_refuses_an_invalid_query_field_with_the_field_name` (W3)
- `find_ceiling_run_of_eight_sources_fits_the_dispatch_budget` (ceiling correctness
  scenario: 8 sources with 40 genuine signed listings each carrying full
  membership evidence and writes, clean of oversized replies, asserts correctness
  and prints wall time, P2, P5, P18, P19)
- `find_ceiling_measure_fits_dispatch_headroom` (isolated measurement test marked
  `#[ignore]`: runs ceiling scenario alone, asserts wall time fits under
  `DISPATCH_HEADROOM_MS`, P10, P18, P19)
- `find_marks_an_oversized_reply_unreadable_and_keeps_the_others` (with canned
  source `hOversized` returning a reply exceeding the 8 MiB cap, P1, P16)
- `run_envelope_returns_the_signed_bytes_of_a_page_record` (replaces the
  scenario 117/98 assertions at directory.rs :923)
- `run_envelope_does_not_find_a_record_that_is_not_on_the_page` (W4)
- `a_second_find_for_one_source_does_not_duplicate_a_listing` (replaces 116:
  one find with canned source `hTwoVersions` returning two genuine signed
  versions of one listing in one answer, P6)
- `a_find_after_the_retention_hour_prunes_old_run_rows`
- `an_unregistered_run_id_has_no_envelope` (replaces the run-id half of 102)

Refusing the whole batch needs the node-wide waiter permits to be full. The
parity harness shares them through a process-wide default, and filling them
would make the tests depend on each other. So the whole-batch refusal is
covered in two small pieces, not through the harness: the library test
`a_refused_batch_marks_every_source_not_started` (the mapping), and the
`roym_directory` unit test `errors_to_record_skips_not_started` (no
`last_error` is written). The full-stack version is under "not tested" (7.3).

Updated tests: scenarios 98, 102c, 102d, 119, 120, `directory_compat.rs`, and
the trust files (through `fan_out`). 106b gets the new verb list.

### 7.3 Not tested

- A real 8-node search (Hub e2e case 23 and `roym_directory_e2e` step 13b
  cover 8 sources, 6 of them unreachable and fast; no real slow remote
  directory exists in the e2e harness).
- The waiter-limit refusal through the full stack (above), if too costly.
- The `web` → `directory` chain under the real 5 s epoch with a source that
  waits its full 2 s (the parity harness overrides WASM dispatch epoch to
  120 s to prevent CI load timeouts, P19).
- A node with `call_all_max_concurrency` below 8 (documented, section 7.6).
- Verification cost on a slow machine (the measure is on the CI machine).

### 7.4 Client tests

- Hub, vitest `search.test.ts`: `findListings_maps_each_error_kind_to_an_outcome`,
  `findListings_turns_an_http_503_into_a_busy_notice`,
  `findListings_turns_any_other_failure_into_a_page_error` (P20),
  `findListings_with_no_sources_returns_no_outcomes`,
  `findListings_sends_one_request`.
- Hub e2e: case 22 ("the results view shows the loopback hit and names the
  unreachable sources", reworded from per-source progress, P21), case 23
  (8 sources, no 503, and checks absence of new start notice text from the
  page, P21).
- `roymctl`: unit tests for the printing of a canned `find` reply: a failed
  source note, a truncated note, the membership lines, the refused block, the
  all-`not-started` case, and error exit non-zero on failure (P20). The existing
  argument-parsing test stays.
- `roym_directory_e2e.rs` and `roym_trust_e2e.rs` go through `run_client_loop`
  and stay in place.

### 7.5 Commands to run after each step

```bash
cargo nextest run -p syneroym-roym-federated-query
cargo nextest run -p syneroym-roym-directory -p syneroym-roym-core
mise run build:roym
cargo nextest run -p syneroym-roym-web
cargo nextest run -p syneroym-substrate --test roym_directory_e2e --test roym_trust_e2e
(cd crates/roym_web/ui && npm test)
cargo xtask check-file-lengths && cargo xtask check-lint-suppressions
```

### 7.6 Size risks and the checking-time risk

- The library: `merge.rs` about 330 lines, `gather.rs` about 200,
  `verify.rs` about 200, `lib.rs` about 150. All under the limit.
- `client_find.rs` about 200 lines, `client_runs.rs` about 200.
  `client_query.rs` and `client_merge.rs` are deleted (-810 lines).
- `directory.rs` (parity) goes from 1 135 to about 900 (scenarios 97, 101,
  102, 116, 119, 120 and the headroom test move out). Lower its entry in the
  oversized list. `directory_find.rs` is new and stays under 600.
- `helpers.rs` must not grow (3.7). `fixtures.rs` shrinks.
- `roym-hub.spec.ts` shrinks (case 23b out).
- **Checking time (unverified).** 8 sources × 40 hits is 320 `verify_envelope`
  calls plus membership checks and evidence writes in one guest call with 5 s
  of own time. The worst case for checking time is 40 genuine hits per source
  (all verified, 20 kept and 20 dropped, P2), carrying maximum membership evidence
  with pinned issuers (P18). If the measure in Step 5 is above 60 % of
  `DISPATCH_HEADROOM_MS`, the fallbacks in order are: (1') verify refused-looking
  envelopes cheaply first (a forged envelope usually fails at its JSON parse, so
  cost is mostly genuine signature checks, P2); (1'') compute membership verdicts
  only for rows that survive `dedupe_and_cap_verified` (at most 10 per source,
  up to 80 total, saving up to 80 membership evaluations, P18); (2) lower
  `MAX_STORED_PER_SOURCE` + `MAX_REFUSED_RESULTS`; (3) raise
  `DISPATCH_HEADROOM_MS` and lower `DEFAULT_SOURCE_TIMEOUT_MS`. (The previous
  fallback (1) to stop after both caps are full saved nothing because the list
  is already cut at 40 before checking, P2).
- **Memory risk during lowering (unverified).** Host lowering writes all 8
  replies into guest memory before guest code runs. The library caps JSON
  parsing at `MAX_SEARCH_REPLY_BYTES` = 8 MiB (derived from 50 max hits ×
  (64 KiB payload + 42 KiB maximum credential/decision evidence + envelope
  overhead) × 1.15 JSON escape factor ≈ 6.0 MiB + 2 MiB margin, P1), but
  `call-all` has no host-side per-reply size cap yet. A backlog row tracks
  this for when `call-all` is next opened (P1).
- **Config risk.** If an operator sets `call_all_max_concurrency` below 8,
  the search takes two waves. Add a comment to `config.sample.toml` next to
  `call_all_max_concurrency`: keep it at 8 or more while a person may add 8
  directories. The developer guide says the same.
- **Pool slots.** A source hosted on the same node is a local WASM callee and
  holds an instance-pool slot while it answers (A3 Q9). A person who adds
  the node's own directory as a source (the Hub e2e does) uses one extra
  slot. Eight local sources plus `web` and `directory` would use 10 of the
  default pool of 10. This is the A3 backlog row; A4 does not change it.

### 7.7 How A4 avoids the kinds of findings A2a, A2b, A2c and A3 had

| Earlier finding | The risk in A4 | What this plan does |
| --- | --- | --- |
| A2c R1 (a test that reads one build) | parity tests of the new method | Every parity test runs both builds and compares the full answer. |
| A2a R1 (a check that does nothing) | the equivalence test could compare empty pages | The test asserts the pages are not empty first (hits and refused), and the Step 4 plan names the scenarios with known non-empty pages. |
| A2b R5 (behaviour change hidden in a rewrite) | the move of the merge and of the check | The functions move without edits. The equivalence test proves it before the old code is deleted (D-A4-15). A reviewer diffs `merge.rs` against `main`'s `client_merge.rs`. |
| A2b R8 (deleted why-comments) | long why-comments in `client_merge.rs` and `client_query.rs` | Section 0 and Step 2: the comments move with the code. |
| A2a R2, R3; A2b R1, R12; A3 R9 (change doc decisions and deviations) | 16 new decisions | Section 9 lists D-A4-1 to D-A4-16 (P12) with the same numbers and meaning; real deviations go in their own list. |
| A2b R2, A2c R3, A3 R3 (wrong counts) | test counts | Step 9 and section 10 say what to count. Record counts in `status.md`. |
| A2a R10, A2b R7, A2c R6 (numbered comments) | tests | Named test functions. No numbered comments. |
| A2b R9 (set-up written twice) | the old and the new helper | `fan_out_one` and `run_client_loop` keep their names and return types. One fake source layer. |
| A3 R10 (living-doc statements now false) | the search loop is described in four places | Section 9 lists each place with a grep to run. |
| A3 R7 (a silent config effect) | a small `call_all` limit silently changes the time | The assertion test covers the defaults; the sample config and the guide state the rule; a backlog row records it. |

## 8. Deferred backlog

All rows are in [deferred-backlog.md](../../deferred-backlog.md).

- **Move to "Recently resolved":**
  - "A guest dispatch cannot loop over more than one network call" (:534).
    The mechanism was built in A3. `directory.find` now uses it.
  - "The Hub and `roymctl` each own their own copy of the client-side search
    loop" (:535). Both are gone.
- **Update:**
  - "Search shows no per-source progress" (:536): the first sentence now says
    the Hub also lost its per-source drawing. The option (c) stays.
  - "A guest cannot read the `call-all` limits" (:542): change the target from
    A4 to "M08F (when the library fans out to more than 16 nodes)". Reason:
    A4 never has more than `MAX_SOURCES` = 8 calls.
  - "The Hub bundle ships three `window` test hooks, one with a resource cost"
    (:512): `RoymDirectory` and `ignoreConcurrency` are gone. The remaining
    hooks have no resource cost. Reword the title and the text; keep the
    pickup trigger.
  - "`search_runs` rows are pruned by age" (:552): now the rows hold only
    page envelopes; the abandoned-run gap shrinks. Reword; keep it open.
  - "`versions_differ` is surfaced by `directory.merge`" (:551): rename the
    method in the title to `directory.find`.
  - "A merged search page cannot be paged past `MAX_SEARCH_RESULTS`" (:545):
    the method name `directory.merge` becomes `directory.find`.
  - "Slow `call-all` waits can hold every guest instance slot" (:538): add
    that a find holds one waiter permit for about 2 s.
- **New rows, section 11:**
  - "**`find` takes two waves when `call_all_max_concurrency` is below 8**":
    the search then needs about 4 s plus checking, which can pass the 5 s
    guest-HTTP deadline of `web`. Reason: Q2 (a). Target: when a guest can
    read the limits, or `web` forwards with `call-all`.
  - "**The federated query library runs only `directory.search`**": the
    exchange and the shop browse need a second method and hit type. Reason:
    Q5 (a). Target: M08F.
  - "**`call-all` has no per-reply size cap**": host lowers remote replies into
    guest memory before guest runs. Target: when `call-all` is next opened (P1).
  - "**Substrate startup warning when `call_all_max_concurrency < MAX_SOURCES`**":
    sample-config comment is the only guard today. Target: substrate startup
    validation (Q2).
  - "**Run ids are not unique under concurrent finds in the same second**":
    two finds starting in the same second count existing runs and mint the
    same id. Target: when a second client starts finds or fast concurrent finds
    occur (P9).
  - "**The node has no guest logging interface**": guest instances cannot emit
    log lines to node logs or tracing. Batch refusal causes are visible host-side
    via `substrate.proxy.call_all.refused{reason}` (D-A3-13). Target: when
    host-to-guest logging is designed (P3).
  - "**A search has no partial result for a slow directory beyond its 2 s**"
    is not needed: D-OF-5 already covers it.
- No new open `TODO` or `FIXME` marker is added.

## 9. Living docs to update at close-out

Ask for each: is a statement now false or incomplete? Do not cite change docs
or milestone and slice ids in these files. Link ADRs only.

- [docs/developer-guide.md](../../../developer-guide.md):
  - The paragraph at :1735-1737 says the Hub and `roymctl` "each still owns
    its own fan-out loop". It is false after A4. Say that both call
    `directory.find`.
  - "Batched Outbound Calls (`call-all`)" (:588): the example says "querying
    several directories during federated search". Add one sentence that the
    `directory` service does this through the federated query library, and
    the rule that `call_all_max_concurrency` should stay at 8 or more while
    a person may add 8 directories.
  - The `roymctl` section: check the `find` description.
  - Grep for `start-run`, `query-source`, `directory.merge` and `max_concurrency`.
- [docs/system-architecture.md](../../../system-architecture.md): grep for
  `directory`, `fan-out`, `merge`. Update :1353 to describe one call to
  `directory.find` with the node asking all sources at once up to
  `call_all_max_concurrency` and no Hub retry (P22). Add the library to the
  crate list if the file has one. Update the search flow if it names the three verbs.
- [docs/system-requirements-spec.md](../../../system-requirements-spec.md)
  (:1300-1302, discovery): check that "clients query directories" still
  reads right.
- [docs/roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md):
  :102 (the Directory row says the client half is "that person's list of
  directories … the fan-out, and the merge"), :368-375 (step C6) and :773
  ("queries each directory it has been given, in parallel"). Update only what
  is false. The Hub paragraph about partial results, if any, changes.
- [docs/TERMINOLOGY.md](../../../TERMINOLOGY.md): add "federated query library"
  and "find" if the file defines terms of this kind. Check first.
- [docs/planning/traceability-matrix.md](../../traceability-matrix.md): add a
  row after `[ROY-CHK]` (:93) for the federated query and the one search
  call. Use the next free id. Evidence: the tests of section 7.
- [crates/substrate/config.sample.toml](../../../../crates/substrate/config.sample.toml):
  the comment next to `call_all_max_concurrency` (:157).
- `AGENTS.md` (Architecture): update sentence on `CallTarget::Service` to name
  the federated query library ("`directory`, through the federated query library
  it links, …", P22).
- Change doc [roym-open-foundation](../../changes/roym-open-foundation/change.md):
  - **Scope:** rewrite line :51 ("The three-call form stays only if a client
    still needs live progress...") to state the three-call form is removed (P22).
  - **Tasks:** tick "The federated query library. Move merging and duplicate
    removal into it. One search method." and the line that holds "Write the
    M08A slice plans" only when all slice plans exist (not A4's job).
  - **Acceptance criteria:** fill the Test column of OF-6 and OF-7 with the
    tests of section 7. Update counts in the OF-14, OF-15 and OF-18 Test columns
    from the real files after Step 7 (count shape files with
    `ls crates/roym_web/tests/method_shapes | wc -l`, 141 methods, 35 directory
    rows, 137 local-only rows, P23).
  - **Design, "Federated query library":** replace the paragraph with the
    real API (D-A4-3 to D-A4-6, D-A4-16): the outcome kinds are `error: null`,
    `not-found`, `timed-out`, `not-started`, `refused`, `unreadable` (the
    sketch listed four).
  - **Decisions:** add D-A4-1 to D-A4-16 (P12) with the same numbers and meaning,
    and one line with the owner's answers to Q1 to Q6.
  - **Deviations:** under "Different from the plan": the outcome kinds
    (`not-found` and `not-started` exist besides the four named); `run-envelope`
    finds only page records; the method is `directory.find`; the three-call
    form is removed (as D-OF-5 said); `SearchOutput` typing and non-boolean
    `truncated` edge case (D-A4-16, P12). Under "Planned wire changes": W1 to W8.
  - **`demo:`:** extend the `none:` reason (Q6, P11).
  - `status` stays `draft`.
- [task.md](task.md): add the A4 result line under "A4".
- [status.md](status.md): row A4. Later steps record the PR, the test counts
  (library tests, parity scenarios, contract rows, vitest, Hub cases) and the
  20-run result.

No ADR is added.

## 10. Exit criteria

A4 is done when all of these are true:

1. The owner has answered Q1 to Q6, and sections 5 to 9 match the answers.
2. `crates/roym_federated_query` exists, builds for native and
   `wasm32-wasip2`, and holds the only merge and duplicate-removal code in the
   workspace (OF-7). `grep -rn "round_robin_merge" crates apps` finds only the
   library.
3. `directory.find` returns the page and one outcome per source. A search over
   three directories, one too slow, returns the two others' hits and names the
   slow one, in one client call, on both builds (OF-6).
4. `directory.start-run`, `directory.query-source` and `directory.merge` are
   gone from the method table, the shapes, the Hub, `roymctl`, the tests and
   the docs.
5. The Hub and `roymctl` make one call to search. `ignoreConcurrency`,
   `window.RoymDirectory` and `MAX_CLIENT_CONCURRENCY` are gone.
6. Each record is checked by the same `roym_core` functions as before. The
   equivalence test passed before the old verbs were deleted.
7. `directory.run-envelope` returns the signed bytes of a page record,
   byte-identical (the e2e check passes).
8. `find_time_fits_the_real_sandbox_defaults` passes against the real
   defaults (counting wave time with 250 ms cancel grace, P19), the parity
   ceiling correctness test passes, and the isolated ceiling measure
   (`find_ceiling_measure_fits_dispatch_headroom`, measuring wall time on WASM
   with 8 sources answering 40 genuine hits each with full membership evidence
   and writes minus nothing) is below `DISPATCH_HEADROOM_MS` (P2, P10, P18, P19).
9. A whole-batch refusal gives `not-started` for every source for both `Internal`
   and `UnsupportedTarget` refusals (library test, P3), and `not-started` never
   writes `last_error` (directory unit test).
10. `a_reply_over_the_size_cap_is_unreadable_and_does_not_stop_the_others`,
    `fifty_hits_with_maximum_evidence_fit_under_size_cap_and_are_read`, and
    `find_marks_an_oversized_reply_unreadable_and_keeps_the_others` pass (P1, P18).
    `directory.rs` (parity) is lowered in the oversized list, the duplication
    baseline is correct, and `helpers.rs` did not grow.
11. The change doc, developer guide, spec, traceability matrix, sample config,
    `task.md`, `status.md` and the backlog are updated as in sections 8 and 9.
12. No `src/` change outside `roym_directory`, `roym_core`, the new crate,
    the Hub UI and `roymctl`. The substrate test files and the sample-config
    comment change as listed in Step 6 and §9 (P24).
13. The new parity file and Hub cases 22 and 23 ran 20 times in a row with no
    failure.
14. `mise run verify` passes with no `--skip`.
15. One PR.

## 11. What A4 does not do

- No generalization of the library to other wire methods (Q5). M08F and M08D
  do that.
- No live progress and no per-source events during a find (D-OF-5).
- No chunking above 16 sources and no reading of the `call-all` limits.
- No change to `web`'s forwarder, to `call-all`, to the epoch rules, to the
  waiter limit or to any limit in the sandbox.
- No change to the server half of `directory` (`directory.search`,
  `publish`, `info`, `standing`).
- No change to the merge rules, the caps, the check rules, the membership
  rules or the page order.
- No cursor or paging past 50 hits.
- No resolution of `versions_differ`.
- No change to `check-standing`, `probe-info` or `publish-to-source`. They are
  single calls.
- No event feed (A5), no remote-client work (A6), no client guide (A7).
- No new demo story (Q6).
- No SDK package and no published schema.
