# M08A slice A3 — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-a3-implementation-plan.md](slice-a3-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-01

Checked against the code on `feat/m08a-a3-call-all` at `5f34e93c` (main
checkout, not a worktree). Line numbers below are at that commit. The plan was
written by Claude Sonnet 5.5, the same model as this review. So there is no
model independence here, as in the A2b plan review. A review by another tool
is still useful.

**Summary:** 0 blocker, 5 major, 9 minor.

Limits of this review: I read code and did not run it. The Wasmtime facts come
from the `wasmtime-48.0.8` source in the cargo cache (the version in
`Cargo.lock`). The timing claims (tick phase, wave arithmetic) come from
reading the spike test and the Wasmtime code, not from a run.

### What was checked and found correct

- **Every place that sets an epoch deadline.** Production code has exactly two:
  [engine.rs:569-570](../../../../crates/sandbox_wasm/src/engine.rs#L569)
  (`build_store_and_instantiate`) and
  [stream.rs:188](../../../../crates/sandbox_wasm/src/stream.rs#L188)
  (`call_resource_method`). `Store::new` also appears in two benchmarks
  ([wasm_engine.rs:113-145](../../../../crates/sandbox_wasm/benches/wasm_engine.rs#L113))
  and two unit tests
  ([engine/tests.rs:454](../../../../crates/sandbox_wasm/src/engine/tests.rs#L454),
  `:665`). I found no other store or deadline site in `crates/`, `apps/`,
  `xtask/` or `test-components/`. The caller table in section 3.4 matches.
- **`proxy::Host::call` today.** The nine steps of section 3.2 match
  [capabilities_proxy.rs:41-170](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_proxy.rs#L41),
  in that order. Steps 3 to 7 need only `&self`. `HostState` is the only
  implementer of the generated `proxy::Host` trait. `AppProxy` has the three
  implementers the plan lists (guest, native host, `roym_core` test host).
- **Wasmtime epoch behaviour.** In `wasmtime-48.0.8`
  (`src/runtime/vm/libcalls.rs`, `new_epoch`) `UpdateDeadline::Interrupt`
  returns `Trap::Interrupt`, and `Continue(delta)` re-arms the deadline. So
  the zero-credit shortcut of D-A3-4 returns the same error as
  `epoch_deadline_trap`. The plan's "one more fact" (the spike callback can end
  a guest one tick later than the trap does) is right. The deadline passes
  between `budget - 1 tick` and `budget` after `set_epoch_deadline`, because the
  ticker is free-running. So `own < budget` at the first event, and the spike's
  arithmetic returns `Continue(1)`. The zero-credit shortcut is needed.
- **Credit is added before the result is lowered.** The host returns the list
  by calling the guest's `cabi_realloc`. That is guest code, so the epoch
  callback can run there. The plan credits before it answers (D-A3-4, "Credit
  on every exit"), so this is correct. Keep this order and say why in a
  comment.
- **Extra exports are allowed (the plan's `unverified` item).**
  `wasmtime-48.0.8/src/runtime/component/matching.rs:154-175` says "It's ok,
  though, to have extra exports in the actual" instance. So a guest built
  against the older `proxy` interface still instantiates when the host
  instance also exports `call-all` and `call-request`. The "stop and ask the
  owner" branch in section 4 is not needed. I did not check what Wasmtime does
  when a variant gets a new case (see Q2).
- **What `invoke` checks.** The router matches section 3.3: the
  native-capability gate
  ([router.rs:140](../../../../crates/router/src/proxy/router.rs#L140)), the
  registry lookup, the local path with the idempotency fence, and the remote
  path. The local path applies `req.timeout` once
  ([:225](../../../../crates/router/src/proxy/router.rs#L225)). The remote path
  applies it for each attempt
  ([:441-480](../../../../crates/router/src/proxy/router.rs#L441)). A local WASM
  target runs in a new store
  ([:270-292](../../../../crates/router/src/proxy/router.rs#L270)).
- **Pool exhaustion is not a hang.** The pool is Wasmtime's
  `total_component_instances`
  ([init.rs:316](../../../../crates/sandbox_wasm/src/engine/init.rs#L316)).
  `instantiate_async` fails at once when no slot is free. Nothing waits for a
  slot. Q9's description is right.
- **Q6 consumers.** `execute_wasm_vals` treats `Deadline` and `Other` the same
  ([execute.rs:136-157](../../../../crates/sandbox_wasm/src/engine/execute.rs#L136)).
  The guest HTTP path maps `Deadline` to `BudgetExceeded` and `Other` to `Trap`
  ([guest_http.rs:136-140](../../../../crates/sandbox_wasm/src/engine/guest_http.rs#L136)),
  and both give a 500 that differs only in text
  ([format.rs:200](../../../../crates/router/src/route_handler/http/format.rs#L200)).
  The ABAC after-step maps `Deadline` to `BudgetExceeded`, which both ingresses
  turn into `QuotaExceeded`
  ([capabilities_store.rs:73](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_store.rs#L73),
  [synsvc_native/data.rs:53](../../../../crates/control_plane/src/synsvc_native/data.rs#L53)).
  The behaviour-change row in section 4 is accurate. No test or UI text reads
  "wasm trap: interrupt" or "handler trapped".
- **Size and layout facts.** `tests_proxy.rs` 761, `host_capabilities.rs` 624,
  `engine.rs` 665, `init.rs` 423, `capabilities_proxy.rs` 458, parity
  `helpers.rs` 869, `proxy_dispatch.rs` 1171, `engine/tests.rs` 852. The
  oversized entries for the three test files are in
  [oversized-test-files.txt](../../../../xtask/oversized-test-files.txt). The
  `proxy-test` WIT is a symlink to the shared `proxy.wit`; the `saga-test` copy
  is a plain file.
- **Plain-call behaviour stays.** A plain `proxy.call` gets no credit. The
  backlog row "A single `proxy.call` can wait past the guest's budget" stays
  true.
- **Writing for readers.** The "How this is shown to an end user" section
  follows the rule: one plain sentence for the end user, then three developer
  bullets. The demo line (`none`) is right: A3 adds nothing a viewer sees.

### Owner questions — reviewer view

I do not answer for the owner. The plan recommends one option for each
question. This is my view of those recommendations.

- **Q1 (return shape, option a). Agree.** An index that repeats the position
  can only disagree with it. OF-3 says "one result per target in the input
  order", and (a) meets it. **Yes, (a) is a deviation from the change doc's
  sketch and it must be recorded.** The sketch has `call-outcome {
  target-index, result }`, a batch-level `per-call-timeout-ms`, and no
  `interface` or `options`. The plan's form drops the index, moves the time to
  each call's `options`, adds `interface`, and adds an outer `result`. The
  change doc says the exact names are set in the slice plan, and its
  Deviations section already lists two earlier `call-all` corrections, so the
  place is clear. Plan section 9 already lists the deviation; keep it, and
  name each of the four differences. Nothing outside the change doc and the
  plan mentions `target-index`, `call-outcome` or `per-call-timeout-ms`
  (I searched `docs/`). One test to add: every exit path returns exactly one
  entry per input call. The type cannot say it (P14).
- **Q2 (refused batch, option a). Agree, with a condition.** (a) adds no
  `proxy-error` case, so I do not need to know how Wasmtime matches a variant
  with a new case. The condition is that the refusal texts become part of the
  contract: put the exact prefix `call-all busy:` in the WIT doc comment, make
  every refusal text one constant, and test it (P7). A4 has to tell "try
  again later" from a real `internal` error, and the prefix is its only
  signal.
- **Q3 (limits in config, option a). Agree.** Every other sandbox limit is a
  config field. The time arithmetic holds: two waves of 5 s fit in 15 s, and
  one guest call lasts at most 5 s own time plus 15 s credit. Two conditions.
  The native build must read the same config (P2). A4's chain must keep the
  inner total below the outer per-call cap (P5): 15 s inside, 10 s outside.
- **Q4 (wait allowance, option a). Agree.** (b) does the work and then stops
  the guest, which is worse. Add a floor so a tiny allowance does not start
  calls that cannot finish (P8).
- **Q5 (one node-wide permit, option a). Agree with (a) over (b) and (c), but
  the plan's reason is not enough.** "At least 6 of 10 slots stay free" is true
  only when a waiter holds one slot. It does not (P4). Keep (a). Lower the
  default, and add the test that shows the real local case.
- **Q6 (fix the classification, option a). Agree.** I checked every consumer
  (see above). The ABAC error change from `Internal` to `QuotaExceeded` is the
  one effect that a caller can see, and it is the meaning the code comments
  already claim.
- **Q7 (no cancel on `stop_wasm`, option a). Agree.** The total limit bounds
  the wait, and the same gap holds for every ordinary call.
- **Q8 (metrics now, option a). Agree.** Fix the count mismatch and add a
  gauge for permits in use, so an operator can see the limit before it
  refuses (P9).
- **Q9 (accept pool failures, option a). Agree that it is not a hang and that
  (b) couples two limits. Disagree that it is only a backlog row.** A4's
  loopback source is a local WASM target (the directory calls its own
  `directory.search`), so this is the normal case, not a corner. See P4.

### Findings

- **P1 — major.** *Where:* D-A3-4 (`EpochBudget::unarmed`, `arm`,
  `allowance`), D-A3-5 (the `allowance.is_zero()` refusal), D-A3-14 (native
  shim), section 7.1.
  [factory.rs:273-298](../../../../crates/app_host_native/src/factory.rs#L273),
  [host_capabilities.rs:413-464](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L413).
  *Problem:* the credit cap reaches `EpochBudget` only through `arm`. `unarmed()`
  takes no cap, and `HostState::new` calls it. `NativeHostFactory::build_host_state`
  calls `HostState::new` and never calls `arm_epoch_deadline`, because the native
  build has no epoch. So on the native build the cap is whatever `unarmed()`
  sets. If it is zero, `allowance()` is zero and `call_all` refuses every batch
  with "no wait time left". The plan's unit tests (`HostState::with_call_all`
  with small limits) have the same problem: they build a `HostState` and never
  arm it. The plan does not say what `unarmed()` returns.
  *Failure case:* Step 6 runs the first parity scenario. The WASM side
  answers, the native side answers `refused: call-all busy`. The scenario
  fails, and the cause is a missing default, not the feature.
  *Fix:* give `EpochBudget` its cap at construction: `EpochBudget::new(credit_cap)`.
  Build it in `HostState::new` with `CallAllLimits::default().total_timeout`,
  and rebuild it in `with_call_all` from the real limits, so the cap and the
  limits cannot disagree. `arm` then resets only the start time and the credit.
  Remove the `credit_cap` argument of `arm` and the read of `call_all.limits` in
  `arm_epoch_deadline`. Add two unit tests: `a_host_state_that_was_never_armed_allows_a_batch`
  and `with_call_all_sets_the_credit_cap`. Say in D-A3-12 that the native build
  still counts credit and enforces the allowance (it is the same code), and only
  skips the epoch callback.
  *Status:* Open.

- **P2 — major.** *Where:* D-A3-8 (last sentence), D-A3-14, Step 5 file list,
  OF-5, D-A3-12 (permitted differences).
  [roym.rs:145](../../../../crates/substrate/src/runtime/roym.rs#L145)
  (also `:215`, `:259`, `:304`, `:345`, `:386`),
  [dual_build_fixture.rs:56](../../../../crates/substrate/src/runtime/dual_build_fixture.rs#L56),
  [factory.rs:88-113](../../../../crates/app_host_native/src/factory.rs#L88).
  *Problem:* the plan says the native factory "makes its own [semaphore] with
  the default (4)" and uses `CallAllLimits::default()`. The node builds seven
  `NativeHostFactory` values (six Roym services and the fixture). So on a node
  that links Roym natively (the `roym` feature):
  1. an operator who sets `call_all_max_calls = 4` in `[roles.app_sandbox]`
     changes the WASM build and not the native build. This breaks "the native
     build has the same limits" (OF-5) outside the tests;
  2. the waiter limit is seven separate semaphores of 4, so up to 28 waiters
     natively and 4 in WASM. D-A3-12 lists no such difference.
  Step 5 does not list the substrate runtime files, so an implementer would
  reach for the default and the setter "for tests".
  *Failure case:* an operator lowers `call_all_total_timeout_ms` to 5000 for a
  small device. The WASM guests obey it. The native Roym services keep 15 s.
  The parity tests (default config in both builds) still pass.
  *Fix:* build one `CallAll` (limits and the waiter `Arc<Semaphore>`) from the
  config in one function, `CallAll::from_config(&SubstrateConfig)`, public in
  `syneroym-sandbox-wasm`. The engine takes it in `init`. The substrate runtime
  builds it once and passes the same value to every `NativeHostFactory` through
  `with_call_all` (seven call sites; add them to Step 5). Then the native build
  and the WASM build share both the limits and the waiter pool. Add a parity
  scenario with a non-default config on both builds (`call_all_max_calls = 4`,
  a 5th call refused). Keep "no epoch deadline natively" as the only permitted
  difference.
  *Status:* Open.

- **P3 — major.** *Where:* D-A3-5 (per-call `tokio::time::timeout` around
  `invoke`), D-A3-6, section 3.3 ("A keyed failure is recorded as a dead letter
  inside `invoke`. That also holds for a call in a batch."), section 7.1.
  [router.rs:225-286](../../../../crates/router/src/proxy/router.rs#L225),
  [router.rs:109-115](../../../../crates/router/src/proxy/router.rs#L109),
  [router.rs:638-644](../../../../crates/router/src/proxy/router.rs#L638),
  [call_dedup.rs:91-137](../../../../crates/router/src/call_dedup.rs#L91).
  *Problem:* the plan puts the same time in two places: `ProxyRequest.timeout`
  (the router applies it inside `invoke_local`) and an outer
  `tokio::time::timeout(timeout, invoke(req))`. The outer timer is created first
  (when the async block starts). The inner timer is created later, after the
  dedup probe (a `spawn_blocking` SQLite call) and the registry lookup. So the
  outer timer always has the earlier deadline and always wins when a call uses
  its whole time. The outer timeout drops the `invoke` future before it can do
  its two clean-up steps: `claim.settle(&outcome)` and `record_failed_call`. A
  keyed call that times out in a batch then leaves no dead letter, but the same
  call through `call` leaves one. The same happens to every unfinished keyed
  call when the total time ends the batch. The plan's claim in section 3.3 is
  false for these two cases. (A dropped claim stays in flight, which is also
  what `settle` does for a timeout, so the fence itself stays safe.)
  *Failure case:* a guest sends a batch with `idempotency-key` set and a 2 s
  per-call time to a node that is down. `call` would write a dead letter after
  its attempts. The batch writes none, and an operator sees no failed call.
  *Fix:* wrap each call in `timeout + CALL_CANCEL_GRACE` (a fixed constant, for
  example 250 ms), so the router's own timeout fires first and `invoke` returns
  `Err(Timeout)` and records the dead letter. The outer timer then only guards a
  `ServiceProxy` that ignores `req.timeout` (the case the plan names). Say in
  D-A3-5 that a call cancelled by the batch total is not dead-lettered and its
  claim stays in flight, and correct the sentence in section 3.3. Add a router
  test (Step 8, with the real `ProxyRouter`): a keyed batch entry that times out
  leaves one dead letter, as `call` does. Use the helpers of
  [dead_letter.rs](../../../../crates/router/src/proxy/tests/dead_letter.rs).
  *Status:* Open.

- **P4 — major.** *Where:* Q5 reasoning ("at least 6 slots stay free"), Q9,
  D-A3-8, D-A3-11, section 7.3 (`slow_batches_do_not_fill_the_instance_pool`),
  section 8.
  [init.rs:108-126](../../../../crates/sandbox_wasm/src/engine/init.rs#L108),
  [roym_web app.rs:233-241](../../../../crates/roym_web/src/app.rs#L233),
  [guest_http.rs:89](../../../../crates/sandbox_wasm/src/engine/guest_http.rs#L89),
  [directory.rs:48-72](../../../../crates/roym_core/src/directory.rs#L48).
  *Problem:* the waiter permit counts one slot for each waiting batch. A real
  search holds more slots than that:
  1. the guest that calls `web` also holds a slot while it waits. `web` forwards
     with a plain `call`, which takes no permit;
  2. every local WASM target of the batch holds a slot while it runs (section
     3.3 says so). A4's loopback source is the directory calling its own
     `directory.search`, which is a local WASM target;
  3. one waiter plus 8 local callees is 9 of the 10 slots.
  The ABAC doc in `init.rs` already counts it this way ("each concurrent
  after-step call holds two pool slots"). The planned test (4 waiters, a scripted
  proxy that uses no slot, an echo call that works) cannot see any of this.
  So the claim in the backlog row ("a test that other calls still run while the
  limit is reached") is not shown for the real case, and plan section 8 closes
  that row.
  *Failure case:* four Hub searches arrive. Four `web` instances (the guest HTTP
  limit per service is 4) and four directory waiters hold 8 of 10 slots. Each
  directory also runs its loopback source: the pool is full. A conversation
  delivery or a data-layer write that needs an ABAC after-step instance now
  fails at instantiation, with a `QuotaExceeded`, for up to 15 s.
  *Fix:* three parts. (1) Write the arithmetic in section 2 (Q5) for the real
  case: slots held by one search = the caller chain + 1 waiter + the local
  targets. (2) Lower the default `call_all_max_waiting_guests` to 2, and clamp
  it so that `waiters * 2 <= max_concurrent_instances - STREAM_INSTANCE_POOL_HEADROOM`.
  This is the owner's call; I recommend 2 because it keeps at least 4 slots for
  everything else when each search holds 3. (3) Replace the test with one that
  uses real local WASM callees: a small pool (`max_concurrent_instances` of 6),
  the limit of waiters reached with batches of local `greeter` calls, and an
  unrelated plain call that must still run, with the exact result (it either
  works or fails with a named error; say which). Keep the cheap scripted test
  for the permit itself. Do not close the backlog row; narrow it (P13).
  *Status:* Open.

- **P5 — major.** *Where:* section 1 ("A4 does"), Q3 reasoning, D-A3-6, D-A3-10,
  section 11, task.md A4. Question asked of this review: is the A3 API enough
  for A4 without a later WIT change?
  [roym_web app.rs:217-241](../../../../crates/roym_web/src/app.rs#L217),
  [client_query.rs:145-170](../../../../crates/roym_directory/src/app/client_query.rs#L145),
  [directory.rs:48-72](../../../../crates/roym_core/src/directory.rs#L48).
  *Problem:* the WIT shape is enough for A4 (see the table below). The
  surrounding design is not decided, and the plan hides it. A4's one search
  call runs: Hub, then `web`, then `directory`, then `call-all`. `web` forwards
  every method with a plain `call` and `options: None`
  (30 s, no credit). The `web` guest is itself an HTTP request with the 5 s
  epoch budget, and it waits for the whole search. So the credit in `directory`
  buys nothing end to end: `web` is stopped at its first guest code after
  5 s. Today's numbers hide this: 8 sources, 2 s each (`DEFAULT_SOURCE_TIMEOUT_MS`),
  and 8 in flight need one wave of about 2 s, which fits in 5 s with no credit.
  If A4 forwards through a one-entry `call-all` instead, three things bite.
  The inner batch total (15 s) is larger than the outer per-call cap (10 s),
  so the outer cuts the search first. Each search takes two waiter permits (one
  for `web`, one for `directory`; D-A3-10), so only two searches run at once at
  the default of 4. And `web`'s credit cap is shared with other batches in the
  same guest call. None of this is written down, and A4 starts from "A3 built
  the mechanism".
  *Failure case:* A4 follows the plan literally, sets a 5 s per-source time,
  and leaves `web` on a plain `call`. A slow source makes the search take 5.1 s.
  The `web` guest is stopped with `Trap::Interrupt` and the Hub shows an error
  instead of the partial page.
  *Fix:* add a short section "What A4 must decide" to the plan (and one line in
  task.md A4). It states: (1) `web` forwards `directory.search` through a
  one-entry `call-all` or A4 keeps the whole search below `web`'s own budget,
  and A4 picks one and tests the slow-source case end to end; (2) if it picks
  the first, the inner total must be below the outer per-call cap, so either
  `call_all_max_call_timeout_ms` is raised to the total, or A4 passes an inner
  total below 10 s; and the search takes two permits; (3) the existing
  host-side check `DEFAULT_SOURCE_TIMEOUT_MS + DISPATCH_HEADROOM_MS < epoch`
  ([directory.rs:65-72](../../../../crates/roym_core/src/directory.rs#L65))
  stays true or is replaced, not dropped silently. Do not change the WIT.
  *Status:* Open.

  What A4 needs from the API, checked:

  | A4 need | In A3? |
  | --- | --- |
  | A remote source is a `CallTarget::Service(did)` with a time and `idempotent` | Yes: `call-request` has `target` and `options`. |
  | Up to 8 sources (`MAX_SOURCES`) in one batch | Yes: limit 16. No split needed. |
  | One entry per source, in order, with a timeout entry for the slow one | Yes: `Err(timed-out)` at the source's index. |
  | Tell "busy, try later" from a real fault | Only by the text prefix (P7). |
  | Read the batch maximum | No. A4 does not need it with 8 sources. M08F might. A new function is an additive WIT change and is safe (extra exports are allowed). |
  | Retries inside `idempotent: true` | Cut by the per-call time (P7). |

- **P6 — minor.** *Where:* D-A3-4 (the grep test `no_production_code_sets_an_epoch_deadline_itself`),
  D-A3-4 last bullet (the grep for `epoch.credit(`), Step 2 tests, section
  7.1 `only_call_all_gives_credit`.
  *Problem:* three weaknesses. (1) The test file `epoch_budget/tests.rs` is
  under `src/` and contains the strings it searches for, so the scan finds
  itself. (2) `epoch.credit(` is a text pattern. `budget.credit(` in the unit
  tests of `EpochBudget`, or `let e = &mut self.epoch; e.credit(..)`, escapes it
  or breaks the count. (3) The scan proves no `set_epoch_deadline` outside the
  budget file but not that `stream.rs` calls `arm_epoch_deadline`. A forgotten
  re-arm would then keep the build-time start and credit for every chunk, and
  the plan deliberately adds no stream test.
  *Failure case:* a later change calls `credit` from a second host function
  through a helper variable. The grep still counts one match, and OF-4 ("no other
  host call gives credit") is broken without any test failing.
  *Fix:* make the rule a type rule, not a text rule. Give `credit` a token only
  `capabilities_call_all.rs` can make (for example `pub(super) struct WaitCredit(Duration)`
  built there, and `credit(&mut self, WaitCredit)`), or keep `credit` private to
  a function in `host_capabilities` that only `call_all` uses. If a scan stays,
  exclude `tests` files and `#[cfg(test)]`, require the match count to be
  exactly the expected one, and add the check that `arm_epoch_deadline(` appears
  in both `engine.rs` and `stream.rs`.
  *Status:* Open.

- **P7 — minor.** *Where:* D-A3-1 (the WIT text, doc comment given as "..."),
  Q2, D-A3-6, section 4 (the `saga-test` row), section 7.2.
  [proxy.wit:52-53](../../../../crates/wit_interfaces/wit/proxy/proxy.wit#L52),
  [saga-test world.wit:25-27](../../../../test-components/saga-test/wit/world.wit#L25).
  *Problem:* three gaps in the contract text. (1) The doc comment of `call-all`
  is left as a placeholder. The WIT is the public contract for every guest and
  for the client guide, so its words belong in the plan. It must say: `call-all
  busy:` is the fixed prefix of every refusal that is not `unsupported-target`;
  the host cuts `timeout-ms` to its maximum; the default is not `call`'s 30 s;
  and the time of a call in a batch covers the whole call, retries included. For
  `call`, the time covers each attempt. A4 passes `idempotent: true` with a
  2 s time, so on the remote path a failed attempt will no longer be retried
  when the time is spent. This is a small behaviour difference between the two
  functions and the plan does not state it. (2) The `call-options` field comment
  ("Per-call deadline; defaults to the host's 30s budget") becomes wrong for a
  batch. Say there that `call-all` has its own default. (3) The `saga-test`
  component imports only the types of `syneroym:proxy/proxy` (its `saga`
  interface uses `call-target`, `call-options`, `proxy-error`). It does not
  import the function `call`. So it proves less than section 4 says. The
  Wasmtime source proves the rule (see "found correct"), so the risk is nil. Say
  what `saga_e2e` shows: the older type set still links.
  *Failure case:* A4's author reads the WIT comment, sees only "...", and writes
  a retry loop that depends on `idempotent` inside a batch.
  *Fix:* write the full doc comment into D-A3-1. Make each refusal text a
  `const` in `capabilities_call_all.rs` and test the text of each, so a change to
  the prefix fails a test. Reword the `saga-test` sentences in sections 4 and
  7.2 to what the test shows.
  *Status:* Open.

- **P8 — minor.** *Where:* D-A3-9, D-A3-5 (`let total = limits.total_timeout.min(allowance)`).
  *Problem:* only a zero allowance refuses. An allowance of 3 ms starts every
  call and cancels them 3 ms later. Remote requests are already sent. A local
  callee is dropped halfway. The guest gets 16 `timed-out` entries and the
  targets may have done the work.
  *Failure case:* a guest runs two batches. The first uses 14.99 s of the 15 s.
  The second one starts 16 calls with a 10 ms total.
  *Fix:* refuse when `allowance < MIN_BATCH_ALLOWANCE` (a constant, for example
  one second, or the default per-call time). Same refusal text and metric label
  as "allowance used". Add the case to
  `the_wait_allowance_shrinks_the_total_time_and_runs_out`.
  *Status:* Open.

- **P9 — minor.** *Where:* Q8 ("Five counters and one histogram"), D-A3-13 (four
  counters and one histogram).
  *Problem:* the two sentences disagree. Also, Q8's reason is that an operator
  needs a signal to raise the waiter limit, but a `refused{reason="busy"}`
  counter only tells them after users are refused. There is no view of permits
  in use.
  *Fix:* fix the count in Q8. Add a gauge `substrate.proxy.call_all.waiting`
  (permits in use, updated when a permit is taken and when it is released).
  *Status:* Open.

- **P10 — minor.** *Where:* section 1 ("The guest cannot raise any limit"),
  D-A3-3, section 7.5.
  [framing.rs:20](../../../../crates/rpc/src/framing.rs#L20).
  *Problem:* two bounds are missing. (1) The number of calls in one guest call is
  bounded only by its own time. Batches that fail at once (a bad target) cost no
  allowance. A guest can loop batches of 16 for its whole 5 s and send many more
  concurrent requests than a plain `call` loop can. (2) The size of the answer
  is bounded per call (a 16 MiB frame), not per batch. 16 answers of 16 MiB
  would have to fit in the guest's memory in one list. A memory limit
  overrun then traps the whole guest call and loses the other 15 answers.
  Section 7.5 says only that the list length is "below any ABI limit".
  *Failure case:* a hostile source answers with a 16 MiB page. Eight of them
  fill the guest memory limit, the guest traps, and the person sees no result at
  all instead of eight refused sources.
  *Fix:* both are small. Count calls started in one guest call in `EpochBudget`
  and refuse past a cap (for example 4 times `call_all_max_calls`). Add an
  aggregate answer cap (`call_all_max_response_bytes`): an entry that would pass
  it becomes `Err(internal("call-all response too large"))`. If the owner does
  not want either now, record both as backlog rows with a trigger, and say so in
  section 11.
  *Status:* Open.

- **P11 — minor.** *Where:* Step 2 (the `engine/tests.rs` bullet), Step 3, section
  7.6, exit criterion 12.
  [engine/tests.rs:539-576](../../../../crates/sandbox_wasm/src/engine/tests.rs#L539)
  (and `:690-725`).
  *Problem:* `engine/tests.rs` builds `AppSandboxEngine { .. }` with a struct
  literal in two places. The plan adds a field to `AppSandboxEngine`
  (`call_all_waiters`, and the limits), so both literals need new lines. The
  plan says "do not add any other line to `tests.rs`" and counts only the lines
  that move out. The count in the oversized list is still lower than 852, so
  nothing fails, but the instruction as written cannot be followed. The same
  holds for any helper that builds a `SandboxTuning`.
  *Fix:* say in Step 2 and Step 3 that the two literals get the new field, and
  that the final line count of `engine/tests.rs` is recorded after both edits.
  *Status:* Open.

- **P12 — minor.** *Where:* section 9 (living docs).
  [system-architecture.md:744](../../../system-architecture.md),
  [developer-guide.md:568](../../../developer-guide.md),
  [developer-guide.md:1271](../../../developer-guide.md),
  [developer-guide.md:1329](../../../developer-guide.md).
  *Problem:* the plan adds batch rows to the limits table but leaves the row
  "One guest call, wall-clock time | 5 seconds" as it is. After A3 that row is
  incomplete: a guest that waits in `call-all` can take up to 20 s of wall
  clock. The developer guide says "the guest's own execution limit (5s)" for a
  scheduled run (`:1329`) and "inside ADR-0021 §6's 5-second budget" (`:1271`).
  Both are about own time and stay true if reworded. The plan's line for the
  "Universal Proxy" heading is 566; it is 568.
  *Fix:* in section 9, reword the architecture row to "One guest call, own time
  (time waiting in `call-all` is not counted, up to a cap)" and check the two
  developer-guide sentences. Refer to the heading, not the line number.
  *Status:* Open.

- **P13 — minor.** *Where:* section 8 (deferred backlog).
  *Problem:* the plan moves "Slow `call-all` waits can hold every guest instance
  slot" to "Recently resolved". The row asks for a limit "per caller or per
  service" and a test that other calls still run. The plan builds a node-wide
  limit (the plan's own new row says a busy service can take all permits) and,
  per P4, does not show the real case. So the row is narrowed, not closed.
  The new rows also miss the two gaps of P10.
  *Fix:* keep the row, rewrite it to the part that is open (per-service or
  per-caller share; slots held by callers and local callees), and add the P10
  rows if they are not built.
  *Status:* Open.

- **P14 — minor.** *Where:* section 7 (tests), checked against P1 to P5.
  *Problem:* the test plan lacks these cases, which the findings above
  need. Each is cheap.
  1. Every exit path returns one entry per input call, in the right order (the
     type cannot say it): all-ok, all-fail, total time ended, mixed with bad
     params.
  2. Two batches in one real guest call on the WASM engine: the second is
     given the rest of the allowance, the credit adds up, and a third is
     refused. Today only a unit test with a scripted clock covers this.
  3. The never-armed `HostState` (P1) on the native build and in the unit tests.
  4. A non-default config on both builds (P2).
  5. A keyed batch entry that times out leaves one dead letter (P3).
  6. Real local WASM callees at a small pool (P4).
  7. `call-all` on a `Dependency` target with a `routing-key` in the router test
     (section 7.4 has no dependency target that resolves).
  *Fix:* add them to sections 7.1 to 7.4, or say which one is left out and why.
  *Status:* Open.
