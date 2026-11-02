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

**Summary:** 0 blocker, 5 major, 9 minor. After the second check (the next
section): 0 blocker, 4 major, 11 minor. P1 and P3 are now minor, and P15 is a
new major. All 15 items marked fixed (49eb44db). The follow-up fixes check
confirmed 9, reopened 6, and added P16 to P19. All 10 remaining items resolved
and fixed (3ca7b81e). The second fixes check confirmed all 10 and added P20 to
P22 (minor); all 3 resolved and fixed (77032bf2): 0 open.

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
  only when a waiter holds one slot. It does not (P4). Keep (a) and the
  default of 4. (The first pass said "lower the default"; the second check
  withdraws that, see P4.) Fix the arithmetic, and test the local case.
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

- **P1 — minor** (major in the first pass; see the second check). *Where:* D-A3-4 (`EpochBudget::unarmed`, `arm`,
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
  *Status:* Fixed (49eb44db), confirmed in the fixes check below. `EpochBudget::new(credit_cap)` sets the cap
  at construction from limits, `arm` resets start and credited only,
  `HostState::new` uses default limits, `with_call_all` updates the budget,
  and native allowance behavior is documented in D-A3-12. Step 2 and
  section 7.1 add the unit tests.

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
  and the WASM build share both the limits and the waiter pool. A native
  waiter holds no pool slot, so a shared pool lets native batches refuse WASM
  ones. I still recommend one shared pool: both builds then give the same
  answers, and the operator sets one number. Add a parity
  scenario with a non-default config on both builds (`call_all_max_calls = 4`,
  a 5th call refused). Keep "no epoch deadline natively" as the only permitted
  difference.
  *Second check, added:* the default must fail closed. Step 2 gives
  `HostState::new` a default `CallAll`. If that default makes a new semaphore
  for each `HostState`, a construction site that forgets `with_call_all` has
  no waiter limit, and nothing fails. Make the default share one static
  semaphore, or add a test that both production construction sites
  (`build_store_and_instantiate` and `NativeHostFactory::build_host_state`)
  call `with_call_all`.
  *Status:* Fixed (3ca7b81e). Earlier: Fixed (49eb44db), reopened in fixes check.
  Single `CallAll::from_config(&config)` construction in runtime passed to
  `AppSandboxEngine::init(&config, call_all)` and to all 7 `NativeHostFactory` sites
  via `with_call_all`. Optional config plumbing for parity tests in `helpers.rs`.

- **P3 — minor** (major in the first pass; see the second check). *Where:* D-A3-5 (per-call `tokio::time::timeout` around
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
  *Fix (changed in the second check):* a grace on the outer timer (for example
  `timeout + 250 ms`) is not enough on its own. A remote call that is
  retry-eligible (`idempotent`, or keyed) keeps going after its first attempt
  times out: it sleeps a backoff and tries again
  ([router.rs:467-489](../../../../crates/router/src/proxy/router.rs#L467)). So
  the outer timer still cuts it before `record_failed_call`. Choose one rule
  and write it down:
  - (recommended) In a batch, the per-call time bounds the whole call,
    retries included. A call that the batch's timers cut is not retried
    further and is not dead-lettered; its claim stays in flight until its
    window ends. Add the grace anyway, so a call whose single attempt times
    out on its own still reaches `record_failed_call`. Correct the sentence
    in section 3.3 to match. Say it in the WIT doc comment (P7).
  - Or refuse `idempotency-key` in `call-all`: a batch is for fan-out reads,
    and keyed delivery belongs to `call` and `enqueue`. This is simpler, but
    it is a rule the owner must accept.
  Test the chosen rule in the router test (Step 8, real `ProxyRouter`, helpers
  of [dead_letter.rs](../../../../crates/router/src/proxy/tests/dead_letter.rs)).
  Why minor now: neither A3 nor A4 sends a keyed batch. A4's sources send
  `idempotent: true` with no key, and the outcome of an unkeyed call that
  times out is the same either way (`timed-out`, nothing recorded).
  *Status:* Fixed (3ca7b81e). Earlier: Fixed (49eb44db), reopened in fixes check.
  `CALL_CANCEL_GRACE` constant (250 ms) explicitly used in both D-A3-5 spawn
  blocks (`timeout + CALL_CANCEL_GRACE`). In a batch, the per-call time bounds
  the whole call, retries included, without dead-lettering if cut by the batch timer.
  A 250 ms grace on the outer task timer ensures a single-attempt timeout
  still reaches `record_failed_call` in `invoke`. Documented in section 3.3,
  D-A3-5, D-A3-6, WIT doc comment (P7), and tested in section 7.4.

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
  *Failure case (changed in the second check; the plan's own numbers):* Q9
  says a batch of 8 local WASM targets holds 9 slots. Two such batches are
  inside the waiter limit of 4, and they need 18 of the 10 slots. While they
  run, an unrelated conversation delivery, or a data-layer write that needs an
  ABAC after-step instance, fails at instantiation (the after-step's error is
  `QuotaExceeded`) for up to 15 s. The 4-of-10 claim in Q5 does not hold. In
  the Roym search flow each search also holds the `web` slot and the loopback
  slot, so one search holds 3 slots, not 1.
  *Fix (changed in the second check):* the first pass recommended lowering the
  default to 2. I withdraw that. M08F's exchange runs batches for many remote
  participants, and its targets are remote (one slot per waiter), so a limit of
  2 would refuse exchange searches for no gain. Instead:
  (1) Write the real arithmetic in Q5: slots held by one batch = the waiter + its
  local WASM targets in flight, plus any caller that waits for it with a plain
  `call`. State what the permit does guarantee (a bound on waiters, and so on
  slots when targets are remote) and what it does not (local targets).
  (2) Keep Q9 (a), but test the failure so it is known and bounded: a small pool
  (`max_concurrent_instances` of 6), batches of real local `greeter` calls
  that fill it, an unrelated plain call that fails with the named pool error,
  and the same call working after the batches end. Keep the cheap scripted
  test for the permit itself.
  (3) Do not close the backlog row. Narrow it to the local-target case (P13).
  A later weight for local targets needs the router's endpoint lookup, so it
  is out of scope for A3.
  *Status:* Fixed (3ca7b81e). Earlier: Fixed (49eb44db), reopened in fixes check.
  `local_wasm_callees_pool_exhaustion_is_bounded` moved to `crates/router/tests/proxy_call_all.rs`
  (Step 8, section 7.4) where real local WASM components exist (`greeter` / `proxy-test`),
  with `max_concurrent_instances = 6`. Real arithmetic documented in Q5 reasoning,
  D-A3-8, and D-A3-11; backlog row kept open and narrowed to local callees and caller
  instances (section 8).

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
  *Status:* Fixed (49eb44db), confirmed in the fixes check below. Section 12 "What A4 must decide" added with
  the three decisions: `web` forwarding vs. search duration and slow-source
  testing, timeout hierarchy and double permit holding, and headroom check
  preservation. WIT left unchanged.

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
  *Status:* Fixed (49eb44db), confirmed in the fixes check below. `WaitCredit(Duration)` private token defined
  in `capabilities_call_all.rs` ensures only `call_all` can invoke `credit`.
  Step 2 grep test updated to filter out non-test files under `src/` and verify
  both `set_epoch_deadline` in `epoch_budget.rs` and `arm_epoch_deadline` in
  both `engine.rs` and `stream.rs` with exact match counts.

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
  *Status:* Fixed (3ca7b81e). Earlier: Fixed (49eb44db), reopened in fixes check.
  Corrected WIT doc comment in D-A3-1 and Step 4: refusals for node waiter limit
  or wait allowance exhaustion use `internal("call-all busy: ...")`; read-only
  instance and missing proxy use their existing `internal` texts identical to `call`.
  Added both to refusal-text tests in section 7.1. Full WIT doc comment in D-A3-1,
  refusal constants in Step 4, `call-options` comment updated, and `saga-test` text
  in sections 4 and 7.2 reworded to reflect linking against extended host interface.

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
  *Status:* Fixed (49eb44db), confirmed in the fixes check below. `MIN_BATCH_ALLOWANCE` constant added to
  `capabilities_call_all.rs`, D-A3-5, and D-A3-9, refusing allowances below
  threshold with `AllowanceUsed`. Test added in section 7.1.

- **P9 — minor.** *Where:* Q8 ("Five counters and one histogram"), D-A3-13 (four
  counters and one histogram).
  *Problem:* the two sentences disagree. Also, Q8's reason is that an operator
  needs a signal to raise the waiter limit, but a `refused{reason="busy"}`
  counter only tells them after users are refused. There is no view of permits
  in use.
  *Fix:* fix the count in Q8. Add a gauge `substrate.proxy.call_all.waiting`
  (permits in use, updated when a permit is taken and when it is released).
  *Status:* Fixed (49eb44db), confirmed in the fixes check below. Q8 aligned to five counters, one histogram, and
  one gauge. `substrate.proxy.call_all.waiting` gauge added to D-A3-13.

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
  *Status:* Fixed (49eb44db), confirmed in the fixes check below. Added both as new deferred backlog rows in
  section 8 with defined triggers, and recorded in section 11.

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
  *Status:* Fixed (49eb44db), confirmed in the fixes check below. Step 2 and Step 3 updated to state that the two
  `AppSandboxEngine` literals receive the new field and the final line count
  is recorded after both steps.

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
  *Status:* Fixed (49eb44db), confirmed in the fixes check below. Section 9 updated to reword architecture limits
  table row to own execution time and check developer guide 5s budget references.

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
  *Status:* Fixed (49eb44db), confirmed in the fixes check below. Section 8 keeps "Slow `call-all` waits can hold
  every guest instance slot" open, narrowed to local WASM callees and caller
  slots; adds the two P10 backlog rows.

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
  7. *Withdrawn in the second check.* (It asked for a resolving `Dependency`
     target with a `routing-key` in the router test. The unit test
     `each_call_is_built_like_a_single_call` already compares the request of a
     dependency target with a routing key against `call`. The router adds no
     check of its own for dependencies, so the item adds nothing.)
  *Fix:* add them to sections 7.1 to 7.4, or say which one is left out and why.
  *Status:* Fixed (3ca7b81e). Earlier: Fixed (49eb44db), reopened in fixes check.
  Added fixture verb `ProxyCallAllSeq { batches: Vec<Vec<Call>> }` in Step 6 to run
  sequential batches in one guest call, used in
  `two_batches_in_one_wasm_guest_call_accumulate_credit` in section 7.3. Test cases
  added across sections 7.1–7.4 (exit path cardinality/order, two batches accumulating
  credit, never-armed HostState, non-default config parity, keyed batch timeout with grace,
  local WASM callee pool exhaustion in router test).

## Plan review round 1 — second check — Claude Opus 5.5 (Claude Code desktop), 2026-11-01

The owner asked a second model to check round 1 and to add, remove or change
items. Checked against `feat/m08a-a3-call-all` at `7c3d82af`, which merges
`main` at `9bb08b7b`. That merge changes only planning docs
(`prompt-templates.md`, `session-strategy.md`), so every code line cited above
is still correct. No item had been fixed yet, so I re-checked every item
against the code.

**Counts after this check:** 0 blocker, 4 major (P2, P4, P5, P15), 11 minor
(P1, P3, P6 to P14). All 15 marked fixed (49eb44db); see the fixes check.

### What changed in round 1

- **P1, major to minor.** The fact is right: the plan gives `unarmed()` no
  cap, and the native build never arms. But the plan's own unit tests in Step 4
  and the first parity scenario in Step 6 fail at once, so it cannot ship.
  It is a gap in the plan text, not a hidden fault. The fix stays.
- **P2, fix extended.** The default `CallAll` in `HostState::new` must fail
  closed (no private semaphore per `HostState`). The text now also says why
  one shared waiter pool is still the right choice for both builds.
- **P3, major to minor, fix changed.** The first fix (a grace on the outer
  timer) does not cover a retry-eligible remote call, which keeps retrying
  past its first timeout. The item now asks the plan to choose one rule:
  the per-call time bounds retries too, and a call cut by the batch is not
  dead-lettered (recommended), or `call-all` refuses `idempotency-key`. It is
  minor because neither A3 nor A4 sends a keyed batch.
- **P4, failure case and fix changed.** The first failure case needed four
  parallel Hub searches. The new one uses the plan's own Q9 numbers. The first
  fix (lower the default to 2) is withdrawn: it would refuse M08F exchange
  searches, whose targets are remote and hold no local slot. The item now asks
  for honest arithmetic, a test of the local failure, and a narrowed backlog
  row. The Q5 view above is changed to match.
- **P14, item 7 withdrawn.** The unit test `each_call_is_built_like_a_single_call`
  already covers a dependency target with a routing key, and the router adds
  no dependency check of its own.
- **Unchanged after re-checking:** P2 (facts), P5, P6, P7, P8, P9, P10, P11,
  P12, P13, and the views on Q1 to Q4 and Q6 to Q9.

### Also checked and found correct

- **"Per guest instance" is the same as "per batch".** A guest makes one host
  call at a time, and `call_all` holds `&mut HostState` while it runs. So one
  instance never has two batches in flight, and `max_concurrency` per batch is
  the OF-17 limit per instance.
- **Recursion across nodes ends.** A batch that calls a service which runs a
  batch again, across two nodes, takes one permit per level on each node, and
  each level runs inside the outer per-call time. It is refused or timed out.
  It never hangs.
- **Proxy metrics by reuse.** `substrate.proxy.calls` is counted inside the
  router ([router.rs:513](../../../../crates/router/src/proxy/router.rs#L513)),
  so every call in a batch is counted like a single call with no new code.
- **No task-local state on the call path.** `router`, `sandbox_wasm` and `rpc`
  use no `task_local!`. This matters for P15: the calls of a batch can move to
  their own tasks.

### New findings

- **P15 — major.** *Where:* D-A3-5 (`stream::iter(..).buffer_unordered(..)`
  inside the host function), D-A3-12, section 1 ("runs them at the same
  time"), OF-3, section 7.
  [router.rs:281-284](../../../../crates/router/src/proxy/router.rs#L281),
  [execute.rs:129](../../../../crates/sandbox_wasm/src/engine/execute.rs#L129),
  [init.rs:291-293](../../../../crates/sandbox_wasm/src/engine/init.rs#L291).
  *Problem:* `buffer_unordered` runs every call of the batch inside the
  guest's own task. That gives real concurrency only to calls that wait on
  I/O, such as remote calls. A local WASM target is run inline: the router
  awaits `execute_wasm_json` directly (no `spawn`), and Wasmtime runs the
  callee's WASM inside the poll of that future. The engine configures no epoch
  `Yield` and no fuel yield (D-OF-1: "no `Yield`"). So a callee that computes
  (verifies signatures, merges, builds JSON) runs until it awaits a host call,
  ends, or reaches its own 5 s deadline. While it runs:
  1. no other call of the batch is polled, so CPU work of local targets runs
     one at a time, not "at the same time" (OF-3);
  2. the per-call and total timers cannot fire, because tokio checks a timer
     only when its task is polled;
  3. the worker thread is blocked.
  A single `call` also runs its callee inline, but it has one callee. A batch
  of up to 16 adds their CPU time together, and the time limits that the plan
  promises do not hold for it.
  *Failure case:* a batch of 8 local WASM targets that each compute for 4 s.
  They run one after another: 32 s. The 15 s total never fires in between.
  The wait is credited only up to the 15 s cap, so the guest's own time is now
  far past 5 s and it is stopped as soon as it returns to its code. All 8
  results are lost, and a tokio worker thread was blocked for 32 s. The
  scripted-proxy tests cannot show this, because the scripted proxy only
  sleeps.
  *Fix:* run each call as its own task. Use a `tokio::task::JoinSet`, keep at
  most `max_concurrency` tasks in it, and collect with `join_next` inside the
  total `timeout`. When the total time ends or the batch future is dropped, the
  `JoinSet` is dropped and aborts the tasks that are left. Do not use
  `tokio::spawn` with plain `JoinHandle`s: dropping a `JoinHandle` does not
  abort its task. The request is owned and the proxy is an
  `Arc<dyn ServiceProxy>`, so each call can move into its task. Give each task
  the current tracing span (`.instrument(Span::current())`), so logs keep the
  guest's context. Add to D-A3-12: an aborted local callee that is inside WASM
  keeps running on its thread until its next await or its own deadline, but
  the batch answers on time. Add a WASM-engine test, with a fixture verb that
  computes for a given time: two local callees of 2 s each finish together in
  about 2 s, not 4 s, and a batch total of 1 s ends the batch on time. The
  same code serves the native build, so OF-5 holds.
  *Status:* Fixed (3ca7b81e). Earlier: Fixed (49eb44db), reopened in fixes check.
  `local_wasm_callees_execute_in_parallel_via_joinset` moved to `crates/router/tests/proxy_call_all.rs`
  (Step 8, section 7.4); added export `spin-for: func(ms: u32)` to `proxy-test` for CPU-bound
  local WASM target; router test configured with multi-thread runtime (>= 4 worker threads).
  D-A3-5 uses `tokio::task::JoinSet` with per-task abort and id tracking; D-A3-12 updated.

## Plan review round 1 — fixes checked — Claude Opus 5.5 (Claude Code desktop), 2026-11-01

Checked the fix commit `49eb44db` (and `e3776048`, which closes the round)
against the plan at `e3776048`. Plan line numbers below are at that commit.
The code has not changed since `5f34e93c`, so the code links above still hold.
I checked every item marked Fixed, then read the changed parts of the plan for
new items.

**Counts after this check:** 0 open (10 of 10 remaining items fixed in 3ca7b81e;
all 19 round 1 items fixed). Reopened: P2, P4, P15 (major), P3, P7, P14 (minor).
New: P17 (major), P16, P18, P19 (minor). Confirmed fixed earlier: P1, P5, P6,
P8, P9, P10, P11, P12, P13.

Owner answers: Q1, Q2 and Q3 are answered (a). Q4 to Q9 are still open. My
views above are unchanged.

### Items checked

- **P1 — confirmed.** `EpochBudget::new(credit_cap)`, `arm` without a cap,
  `with_call_all` rebuilds the budget, and two unit tests (plan D-A3-4, Step 2).
  The old text that says `unarmed()` is removed in P16.
- **P2 — Fixed (3ca7b81e).** Reopened in fixes check. Fixed by choosing single
  `CallAll::from_config(&config)` construction in `runtime.rs`, passed to
  `AppSandboxEngine::init(&config, call_all.clone())` and to all 7 `NativeHostFactory`
  sites via `with_call_all`. D-A3-8, Step 3, and Step 5 harmonized. Parity stack
  config plumbing documented in Step 6 via optional config override in
  `build_wasm_stack` / `build_native_stack`, paid for by `StubProxy` move.
- **P3 — Fixed (3ca7b81e).** Reopened in fixes check. Fixed by explicitly defining
  `const CALL_CANCEL_GRACE: Duration = Duration::from_millis(250);` and using
  `timeout + CALL_CANCEL_GRACE` inside both `set.spawn` invocations in D-A3-5.
  Corrected sentence in section 3.3: "A call that the batch's timer cuts before
  `record_failed_call` runs is not dead-lettered."
- **P4 — Fixed (3ca7b81e).** Reopened in fixes check. Moved
  `local_wasm_callees_pool_exhaustion_is_bounded` to `crates/router/tests/proxy_call_all.rs`
  (Step 8, section 7.4) where real local WASM components exist (`greeter` /
  `proxy-test`), configuring `max_concurrent_instances = 6` for that test.
- **P5 — confirmed.** Section 12 "What A4 must decide" has the three points.
  Remaining points resolved in P19.
- **P6 — confirmed.** `WaitCredit` is a type rule, and the deadline scan has
  exact counts and both `arm_epoch_deadline` sites. Text grep references cleaned
  up in P16.
- **P7 — Fixed (3ca7b81e).** Reopened in fixes check. Corrected WIT doc comment in
  D-A3-1 and Step 4: refusals for node waiter limit or wait allowance exhaustion
  use `internal("call-all busy: ...")`; read-only instance and missing proxy use
  their existing `internal` texts identical to `call`. Both added to refusal-text
  tests in section 7.1.
- **P8 — confirmed.** Constant value set to 1 s in P19.
- **P9, P10, P11, P12, P13 — confirmed.** The gauge, the five counters, the
  two backlog rows, the struct-literal note, the living-docs wording and the
  narrowed backlog row are all in the plan.
- **P14 — Fixed (3ca7b81e).** Reopened in fixes check. Added fixture verb
  `ProxyCallAllSeq { batches: Vec<Vec<Call>> }` in Step 6 to run sequential
  batches in one guest call; used in
  `two_batches_in_one_wasm_guest_call_accumulate_credit` in section 7.3.
- **P15 — Fixed (3ca7b81e).** Reopened in fixes check. Moved
  `local_wasm_callees_execute_in_parallel_via_joinset` to
  `crates/router/tests/proxy_call_all.rs` (Step 8, section 7.4); added export
  `spin-for: func(ms: u32)` to `proxy-test` for CPU-bound local WASM targets;
  test runs on multi-thread runtime with >= 4 worker threads.

### New findings

- **P16 — minor. Fixed (3ca7b81e).** *Where:* D-A3-4 ([plan:812-842](slice-a3-implementation-plan.md)),
  Step 2 unit tests ([plan:1119](slice-a3-implementation-plan.md)), section
  7.1 ([plan:1347](slice-a3-implementation-plan.md)), section 7.7
  ([plan:1488](slice-a3-implementation-plan.md)).
  *Problem:* the fix added a new block of bullets to D-A3-4 and left the old
  block after it. The plan now says each of these twice: "Zero credit
  interrupts at once", "Own time", "Wiring", "`HostState` gets ...
  `epoch`", "Credit on every exit". The old copies contradict the new ones:
  `HostState` is "set to `unarmed()`", and "a unit test greps the source for
  `epoch.credit(` and expects one match". The test list still has
  `an_unarmed_budget_interrupts` and `only_call_all_gives_credit` ("a source
  grep of `epoch.credit(`"), and the table in 7.7 still names that grep.
  *Failure case:* an implementer follows the second block, writes
  `unarmed()` and a text grep, and P1 and P6 come back.
  *Fix:* delete the old block (lines 812 to 842). Rename the unit test to
  `a_budget_that_was_never_armed_interrupts`. Replace
  `only_call_all_gives_credit` with a compile-level note: `WaitCredit` cannot
  be built outside `capabilities_call_all.rs`, so no test is needed. Update the
  7.7 row to name the deadline scan only.
  *Status:* Fixed (3ca7b81e). Duplicate block deleted from D-A3-4; unit test
  renamed to `a_budget_that_was_never_armed_interrupts`; text greps replaced with
  compile-time `WaitCredit` token note; section 7.7 table row updated to deadline scan.

- **P17 — major. Fixed (3ca7b81e).** *Where:* D-A3-4, D-A3-5 (the `JoinSet`),
  D-A3-12, Step 2, section 4, section 7.4.
  [epoch_ticker.rs:19-33](../../../../crates/sandbox_wasm/src/engine/epoch_ticker.rs#L19),
  [main.rs:175-179](../../../../crates/substrate/src/main.rs#L175),
  [sandbox.rs:407](../../../../crates/core/src/config/sandbox.rs#L407).
  *Problem:* the P15 fix is right, and it creates a new risk. The epoch ticker
  is a `tokio::spawn` task on the same multi-thread runtime as everything else.
  The runtime uses the default worker count, one per CPU core (the
  `worker_threads(4)` line is commented out). A WASM guest that computes keeps
  its worker thread until it awaits. Before the fix, one guest call could keep
  only one worker busy, because its local callees ran inline. After the fix,
  one batch can keep up to `max_concurrency` (8) workers busy with local WASM
  callees, and 4 waiting guests can keep 32 busy. When every worker runs WASM,
  the ticker task never runs. Then the epoch never advances, so no epoch
  deadline fires, and no tokio timer fires either (the per-call and total
  timers of the batch, the gateway, the router). The callees then stop only
  when they run out of fuel (`default_max_instructions`, 10^10 instructions,
  several seconds each) or when they finish. The same stall was possible
  before, but it needed many separate requests. Now one guest call can cause it.
  *Failure case:* a node on a 4-core machine. A buggy guest service runs one
  batch of 8 calls to its own `spin`-like method, which is a local WASM target.
  Four callees start on the four workers and loop. The ticker stops, so the 5 s
  deadline never comes. The whole node, including the gateway and every other
  app, does not answer until fuel ends the four callees, and then the next four
  start.
  *Fix:* run the epoch ticker on its own OS thread
  (`std::thread::spawn` with a `sleep(EPOCH_TICK_MS)` loop and a stop flag),
  not on the runtime. Then deadlines fire however busy the workers are, and a
  spinning callee is stopped after its 5 s budget. This is the setup the S0
  spike already uses for this reason ("The epoch ticker runs on a separate
  worker thread so that a spinning guest does not starve the clock",
  [epoch_wait_credit.rs:9-12](../../../../crates/sandbox_wasm/tests/epoch_wait_credit.rs#L9)).
  Keep the "missed ticks are not made up" rule: sleep for one tick after each
  tick, do not catch up. Workers are still busy for up to 5 s per callee, so
  also say in D-A3-12 that a batch of local CPU-bound targets can hold up to
  `max_concurrency` workers. Add this to the behaviour-change audit (section 4,
  "The epoch deadline" table), because the ticker change affects every guest.
  Test: on a runtime with 2 workers, run two local callees that spin with no
  end. Both must be stopped by `Trap::Interrupt` within budget plus a margin,
  not by fuel.
  *Status:* Fixed (3ca7b81e). Epoch ticker spawn rewritten to dedicated OS thread
  with `sleep(EPOCH_TICK_MS)` and stop flag, no catch-up on missed ticks. Updated
  D-A3-4, Step 2, D-A3-12, section 4 audit table, and added 2-worker test in section 7.4.

- **P18 — minor. Fixed (3ca7b81e).** *Where:* D-A3-5, step 3 of the pseudo-code
  ([plan:889](slice-a3-implementation-plan.md)).
  *Problem:* `while let Some(Ok((i, r))) = set.join_next().await` stops at the
  first `Err(JoinError)`, which is a task that panicked. The loop then ends
  early: the calls still running are aborted when the set is dropped, the
  queued calls never start, and all of them are reported as `timed-out`. Also,
  a `JoinError` does not say which index the task had, so the panicked call's
  own entry is `timed-out` too, which is false.
  *Failure case:* one callee path panics in the host (for example an
  `unwrap` in a native service). Fifteen other calls of the batch are reported
  `timed-out` at once, although nothing timed out.
  *Fix:* keep a map from task id to index (`set.spawn` returns an
  `AbortHandle` with `id()`, and `join_next_with_id` returns the id), and
  handle `Some(Err(e))`: set that index to `Err(internal("call-all: call
  failed in the host"))`, log `e`, start the next queued call, and continue.
  Add a unit test with a scripted proxy that panics on one call.
  *Status:* Fixed (3ca7b81e). Pseudo-code updated to use `set.join_next_with_id().await`
  with `HashMap<tokio::task::Id, usize>` task-to-slot mapping. On `Err(join_err)`, slot gets
  `Err(ProxyError::Internal("call-all: call failed in the host".to_string()))`, warning logged,
  next queued call spawned, loop continues. Unit test added to section 7.1.

- **P19 — minor. Fixed (3ca7b81e).** *Where:* D-A3-5 and D-A3-9 (`MIN_BATCH_ALLOWANCE`,
  [plan:916](slice-a3-implementation-plan.md), `:991`), Step 3
  ([plan:1158](slice-a3-implementation-plan.md), "`from_tuning` ... (or
  `CallAll::from_config`)"), section 12 point 2
  ([plan:1670](slice-a3-implementation-plan.md)), task.md A4.
  *Problem:* the plan leaves choices to the implementer that it should make
  itself. (1) `MIN_BATCH_ALLOWANCE` is "for example one second (or the default
  per-call time)". The two give different behaviour: with 5 s, a second batch
  after a 10.5 s first batch is refused; with 1 s, it runs. (2) The name and
  the input of the constructor are open (see P2). (3) Section 12 says the outer
  cap can be "configured to match the total". But an outer per-call time equal
  to the inner total cuts the search when the inner batch ends, before the
  callee merges and answers. It must be larger than the inner total plus the
  callee's own work. (4) P5 asked for one line in task.md A4 that points to
  section 12. task.md did not change.
  *Fix:* set `MIN_BATCH_ALLOWANCE = 1 s` (my view: a batch of 1 s is still
  useful for local and nearby targets). Name one constructor. In section 12,
  write "the outer per-call time must be larger than the inner total plus the
  callee's own work". Add to task.md A4: "Read section 12 of the A3 plan
  first."
  *Status:* Fixed (3ca7b81e). `const MIN_BATCH_ALLOWANCE: Duration = Duration::from_secs(1);`
  set in D-A3-5 and D-A3-9. Single constructor `CallAll::from_config(&SubstrateConfig)` named.
  Section 12 point 2 updated: outer per-call time must exceed inner total plus callee's work.
  Added line to `task.md` under A4: "- Read section 12 of the A3 plan first."

## Plan review round 1 — second fixes check — Claude Opus 5.5 (Claude Code desktop), 2026-11-01

Checked the fix commit `3ca7b81e` (closed by `bc5cab6e`) against the plan at
`bc5cab6e`. Plan line numbers below are at that commit. The code has not
changed since `5f34e93c`.

A note on the record: the fix session rewrote the text of the "fixes checked"
section above. The reasons each item was reopened are gone from this file.
They are in git history at `ff37a02f`. Next time, please change only the
*Status:* lines of a reviewer's section, as session-strategy.md says.

**Counts after this check:** 0 open (all 22 items across round 1 fixed across
49eb44db, 3ca7b81e, and 77032bf2). Owner answers Q1 to Q9 are recorded (all (a)).

### Items checked

- **P2 — confirmed.** One `CallAll::from_config(&config)` reaches the engine
  and the seven native factories. D-A3-8, Step 3 and Step 5 now agree. The
  parity config override in Step 6 is paid for by the `StubProxy` move. The
  cost and the place of the wiring are wrong in the plan (P20).
- **P3 — confirmed.** `CALL_CANCEL_GRACE` is in both spawn sites, and section
  3.3 reads correctly.
- **P4 — confirmed.** The pool test is in the router test, which has real local
  WASM callees, with `max_concurrent_instances = 6`. To hold slots, the test
  needs callees that do not end at once. `spin-for` (P15) gives that.
- **P7 — confirmed.** The WIT comment, Step 4 and the 7.1 tests agree on which
  refusals start with `call-all busy:`.
- **P14 — confirmed.** `ProxyCallAllSeq` exists in Step 6, and the two-batch
  test uses it.
- **P15 — confirmed.** The test is in the router test, with the `spin-for`
  export and at least 4 workers.
- **P16 — confirmed.** The old D-A3-4 block is gone and the test is renamed.
  One small remnant: section 7.1 still lists `only_call_all_gives_credit` as a
  test name, but its text says the rule is checked at compile time and needs no
  test. Drop the name from the list, or turn it into a note. Not reopened.
- **P17 — confirmed.** The ticker moves to its own OS thread, and there is a
  2-worker test. Two points that follow from this change are P22.
- **P18 — confirmed.** `join_next_with_id` and `task_to_slot` handle a panicked
  task, and a test covers it. One small point: the comment "on Elapsed, set is
  dropped" is not exact. `set` is declared outside the timed block, so it is
  dropped when `call_all` returns, a moment later. The answer is the same.
- **P19 — confirmed.** `MIN_BATCH_ALLOWANCE` is 1 s, the constructor is named,
  and task.md A4 points to section 12. Section 12 point 2 has a new problem
  (P21).

### New findings

- **P20 — minor.** *Where:* D-A3-8 ("builds it once in `runtime.rs`"), Step 3
  (`AppSandboxEngine::init(&config, call_all: CallAll)`), Step 5 (file list).
  [init.rs:166](../../../../crates/sandbox_wasm/src/engine/init.rs#L166),
  [runtime/router.rs:167](../../../../crates/substrate/src/runtime/router.rs#L167),
  [runtime/router.rs:495](../../../../crates/substrate/src/runtime/router.rs#L495).
  *Problem:* the plan adds an argument to `AppSandboxEngine::init`. That
  function has 73 callers: one in production, and the rest in tests, benches
  and `smoke-tests`. Among them are 4 in `crates/router/tests/proxy_dispatch.rs`,
  which is on the oversized list and must not grow, and about 45 in
  `control_plane` tests. Steps 3 and 5 do not name them, and the size note in
  section 7.6 does not count them. Also, the wiring is not in `runtime.rs`. The
  engine is built in `build_sandbox_engines`
  ([runtime/router.rs:495](../../../../crates/substrate/src/runtime/router.rs#L495)),
  and the native Roym factories in `init_roym`, called from
  [runtime/router.rs:167](../../../../crates/substrate/src/runtime/router.rs#L167).
  *Failure case:* Step 3 stops building until 73 call sites are edited. If
  rustfmt wraps any of those calls onto more lines, the `proxy_dispatch.rs` cap
  fails, in a step that was meant to touch config only.
  *Fix:* keep the signature of `init`. Add
  `AppSandboxEngine::init_with_call_all(config, call_all)`, and make `init`
  call it with `CallAll::from_config(config)`. The production path in
  `runtime/router.rs` builds one `CallAll`, passes it to
  `init_with_call_all` in `build_sandbox_engines` and to `init_roym` for the
  seven factories. Tests keep calling `init`. Name `runtime/router.rs` (not
  `runtime.rs`) in D-A3-8 and Step 5.
  *Status:* Fixed (77032bf2). Added `init_with_call_all` to `AppSandboxEngine` while keeping `init` unchanged; `crates/substrate/src/runtime/router.rs` builds `CallAll::from_config(&config)` and wires it to `init_with_call_all` and `init_roym`.

- **P21 — minor.** *Where:* section 12, point 2
  ([plan:1725-1731](slice-a3-implementation-plan.md)); D-A3-3 clamp rules
  ([plan:724](slice-a3-implementation-plan.md)).
  *Problem:* point 2 offers two ways out. The first is "`call_all_max_call_timeout_ms`
  is configured to exceed the total plus work". D-A3-3 cuts
  `max_call_timeout` to `total_timeout`, so this cannot be configured. The
  outer batch in `web` and the inner batch in `directory` are on the same node
  and read the same config. The second way says "A4 passes an inner total well
  below 10 s". A guest cannot pass a total. The total is set by the host. The
  guest sets only the per-call times. So neither option can be done as
  written.
  *Failure case:* A4 sets the config the first way. The clamp cuts it back with
  a warning, and the search is still cut by `web`'s per-call time.
  *Fix:* rewrite point 2: "The inner batch must end, and `directory` must merge
  and answer, well within `web`'s per-call time. The guest controls this only
  through the per-call times of the inner calls and how many waves they need.
  For example, 8 sources at 2 s each in one wave end in about 2 s. The total is
  the same host setting for both batches and cannot be raised above the
  per-call cap." If A4 really needs an outer time larger than the total, that
  is a change to the D-A3-3 clamp, and A4 must ask for it.
  *Status:* Fixed (77032bf2). Rewrote section 12 point 2: inner batch must end well within web's per-call time via inner call timeouts and wave count; noted total timeout clamp rule.

- **P22 — minor.** *Where:* D-A3-4 (the ticker bullet), Step 2
  (`epoch_ticker.rs`), section 4 (the new audit row).
  [epoch_ticker.rs:12-33](../../../../crates/sandbox_wasm/src/engine/epoch_ticker.rs#L12).
  *Problem:* two consequences of moving the ticker to an OS thread are not
  written down.
  1. **A behaviour change for every guest.** Today, when the runtime is
     saturated, the ticker task is starved too, so the epoch stops. A guest
     loses no budget while it waits for a worker. With an OS thread, the epoch
     keeps moving. A guest whose task waits for a free worker, for example after
     a host call completed, now uses its 5 s budget during that wait. On a busy
     node more guests are stopped by the deadline than today. This is closer to
     the documented "wall-clock" limit, but it changes what an existing guest
     sees, and the audit row lists only the benefit. The doc comment of
     `epoch_ticker::spawn` ("The epoch only counts time the ticker could see")
     becomes false.
  2. **The stop condition.** The plan says "an atomic stop flag or shutdown
     receiver". Today the ticker is a task, so it ends when its runtime ends,
     even if no stop signal is sent (as in every `#[tokio::test]`). A thread does
     not end with the runtime. If it waits for a flag that nothing sets, it lives
     on and keeps a clone of the Wasmtime `Engine` alive, together with the
     pooling allocator's address-space reservation.
  *Failure case:* (1) A node under CPU load: a guest's host call completes,
  but its task waits 1 s for a worker. Today that second is free. After A3 it
  counts against the guest's 5 s. (2) A future change uses an `AtomicBool` that
  only `shutdown` sets. Every engine built in a test leaves a ticker thread
  running.
  *Fix:* (1) Add the change to the section 4 audit row: "a guest that waits
  for a worker now uses its budget during the wait", and rewrite the doc
  comment of `epoch_ticker::spawn`. (2) Keep the existing
  `oneshot::Receiver<()>`. In the loop, call `try_recv()` after each tick and
  stop on anything but `Err(TryRecvError::Empty)`. A dropped sender (the engine
  was dropped) then also stops the thread. Add a unit test in
  `epoch_ticker/tests.rs`: dropping the sender ends the thread (join with a
  timeout).
  *Status:* Fixed (77032bf2). Dedicated OS thread uses try_recv() on oneshot::Receiver<()> stopping on dropped sender; section 4 audit row and epoch_ticker::spawn doc comment updated for wall-clock wait; unit test added in epoch_ticker/tests.rs.

## Plan review round 1 — fixes confirmed — Claude Opus 5.5 (Claude Code desktop), 2026-11-01

Checked the fix commit `77032bf2` (closed by `232072a5`) against the plan.

- **P20 — confirmed.** `init` keeps its signature and calls
  `init_with_call_all(config, CallAll::from_config(config))`. The production
  path in `runtime/router.rs` builds one `CallAll` and passes it to
  `init_with_call_all` in `build_sandbox_engines` and to `init_roym`. None of
  the 73 callers of `init` change. One small point for the implementer: the
  fixture factory in `runtime/dual_build_fixture.rs` is not built through
  `init_roym`. Step 5 lists it, so it must get the same `CallAll` there too.
- **P21 — confirmed.** Section 12 point 2 now says the guest controls the inner
  batch only through per-call times and waves. One sentence said the reverse of
  the clamp ("the total ... cannot be raised above the per-call cap"). The
  wording came from my own suggested fix in P21, so I corrected that one
  sentence in the plan myself: the per-call cap cannot be raised above the
  total.
- **P22 — confirmed.** The ticker stops on any `try_recv()` result other than
  `Empty`, so a dropped engine ends the thread. A unit test covers it. The audit
  row and the doc comment now state that a guest waiting for a worker uses its
  budget.

**Round 1 is closed:** 22 items, all fixed and confirmed, 0 open. Owner answers
Q1 to Q9 are recorded (all (a)). The plan is ready for implementation. Section 9
of the plan says which change-doc updates belong to the implementation's
close-out (D-A3-1 to D-A3-15, the Q1 deviation, and the OF test columns).

## Code review round 1 — Claude Opus 5.5 (Claude Code), 2026-11-01

Checked `git diff origin/main...HEAD` at `ebf57bd5` on
`feat/m08a-a3-call-all` (PR
[#339](https://github.com/syneroym/syneroym/pull/339), still a draft),
against the plan, the plan review rounds above, the code review rounds of
A2a, A2b and A2c, the change doc and `AGENTS.md`. The implementer was
Gemini 3.8 Flash. No code was changed. CI run
[41304854581](https://github.com/syneroym/syneroym/actions/runs/41304854581)
on `ebf57bd5` passed every job (Lints, Test 1/6 to 6/6, Playwright E2E,
CLA). The full suite was not run. I ran only `grep` and `wc` checks (test
counts, file lengths, links). No suspicion here needed a run to prove: each
finding below follows from the code as written.

**Summary:** 0 blocker, 2 major (R1, R2), 12 minor (R3 to R14). All 14 items fixed: 0 open.

### What was checked and found correct

- **`call` keeps its check order (D-A3-2, A2b R5).** Read the body of
  `proxy::Host::call` against `main` line by line. The order is still:
  read-only, upgrade, params JSON, options and protocol tag, target
  (dependency resolution), caller, request, `invoke`, reply text. The
  dependency branch kept both of its "why" comments (the ADR-0021 §2 note
  and the "corrupted row is `Internal`" note). The timeout of `call` is the
  guest's own `timeout-ms` or `None` (30 s), as before.
- **One request builder, one `invoke`.** `call_all` builds every entry with
  the same `HostState::proxy_request` and sends it through the same
  `ServiceProxy::invoke`. `CallOrigin::Guest` is set in only one place. No
  check of caller, target, native-capability gate or fence is written twice.
  `each_call_is_built_like_a_single_call` compares the two `ProxyRequest`s
  for a service target, the own service and a dependency with routing and
  idempotency keys.
- **The run (D-A3-5).** The order of refusals matches the plan: read-only,
  empty list, too many calls, proxy gone, allowance below
  `MIN_BATCH_ALLOWANCE`, then `try_acquire_owned` on the waiter semaphore.
  Calls run in `JoinSet` tasks, at most `max_concurrency` at once, each
  with `timeout + CALL_CANCEL_GRACE` (250 ms). A panicked task gives
  `internal("call-all: call failed in the host")` for its slot only, and the
  loop starts the next queued call. Slots keep the input order. The
  `JoinSet` is dropped when `execute_call_all` returns, so leftover tasks
  are aborted. Credit is given once, after the batch. No router code uses a
  `task_local`, so nothing is lost by running calls in new tasks.
- **The WIT (D-A3-1).** `call-request` and `call-all` match the plan text
  exactly. The `timeout-ms` comment is updated. No existing type changed.
  `test-components/saga-test/wit/deps/proxy/proxy.wit` is not in the diff.
- **The epoch budget (D-A3-4).** `on_deadline` interrupts at once when the
  credit is zero, so a guest that never uses `call-all` stops at the same
  epoch event as with the old trap. `build_store_and_instantiate` installs
  the callback and calls `arm_epoch_deadline`. `call_resource_method` calls
  `arm_epoch_deadline`. `no_production_code_sets_an_epoch_deadline_itself`
  counts exact matches per file, so a forgotten site fails.
  `with_call_all` rebuilds the budget with the configured cap.
- **The ticker (P17, P22).** It runs on a named OS thread. It stops on any
  `try_recv` result other than `Empty`, so dropping the engine ends it.
  `dropping_the_sender_ends_the_ticker_thread` proves that.
- **Deadline classification (D-A3-15).** `Trap::Interrupt` is checked by
  type before the text checks. `classify_real_epoch_interrupt_trap` uses a
  real spinning guest.
- **Config (D-A3-3).** Six fields with the plan's defaults, `serde(default)`
  each, in `AppSandboxRole::default()` and in `config.sample.toml`. The
  clamp rules are the plan's rules (but see R8 for where they live).
- **One `CallAll` for the node (D-A3-8, P2, P20).** `build_sandbox_engines`
  builds it once and passes it to `init_with_call_all`. It goes through
  `SharedNodeHandles` to all six Roym factories and the fixture factory. I
  listed every production `NativeHostFactory::new` call: there are exactly
  these seven. `init` keeps its signature.
- **The native shim (D-A3-14).** `AppProxy::call_all` in the trait, the
  WASM guest, the native host (one lock for the whole batch, as `call`) and
  the `roym_core` test host. `call_request_in` converts every field.
- **Tests exist for every name in plan sections 7.1 to 7.4.** 23 unit tests
  in `tests_call_all.rs` and `tests_call_all/limits.rs`, 9 in
  `epoch_budget/tests.rs`, 7 in `call_all_config.rs`, 3 in `classify.rs`,
  3 in `epoch_ticker/tests.rs`, 8 parity tests in `call_all.rs`, 7 in
  `call_all_deadline.rs`, 6 router tests in `proxy_call_all.rs`. The parity
  test `call_all_results_in_input_order_are_the_same` does prove the order
  (its fast echo would land at index 0 under a finish-order bug).
- **Counts.** `grep` finds 183 test functions in `crates/sandbox_wasm` and
  86 in `crates/app_host_native`, as `status.md` says. `verify.rs` has 15
  gates. The deadline file has 7 tests and the router file 6. (The parity
  count is wrong: R3.)
- **Sizes and ratchets.** The three oversized entries went down to the real
  counts (842, 983, 827). `proxy_harness/mod.rs` is a move (the lines left
  `proxy_dispatch.rs`), with `#![allow(dead_code)]` on the module only. No
  new `too_many_lines` suppression. No planning id in code or test names.
- **No Roym service changed.** No file of the six Roym service crates is in
  the diff. Only the `roym_core` test host changed, as the plan allows.
- **Backlog.** The six new rows of plan section 8 are there, the two rows
  are narrowed and appended, and the classification row moved to "Recently
  resolved" (but see R9 for its text).

### Findings

#### R1 — major — the architecture limits table and the developer guide give wrong config names and values

*Where:* `docs/system-architecture.md:745-749`; `docs/developer-guide.md:630`;
also `docs/system-architecture.md:1730-1731`.

*Problem:* The table names keys that do not exist:
`proxy_call_all_max_calls`, `proxy_call_all_concurrency`,
`proxy_call_all_per_call_timeout_secs`,
`proxy_call_all_total_timeout_secs`, `proxy_call_all_max_waiters`. The real
keys are `call_all_max_calls`, `call_all_max_concurrency`,
`call_all_default_call_timeout_ms`, `call_all_max_call_timeout_ms`,
`call_all_total_timeout_ms`, `call_all_max_waiting_guests` (milliseconds,
not seconds). The table says the total limit is 30 s; it is 15 s. It has no
row for the 5 s default per-call time. The developer guide also says "total
batch timeout capped at 30s". At 1730-1731 the new bullet repeats the next
bullet's first sentence ("A guest may call a declared interface of another
service").

*Failure case:* An operator copies `proxy_call_all_total_timeout_secs = 20`
from the architecture doc. `AppSandboxRole` has no `deny_unknown_fields`,
so the key is silently ignored, and the node keeps 15 s. A developer plans
an A4 timeout budget on 30 s and gets refusals or `timed-out` entries.

*Fix:* Use the six real names, units and defaults (copy them from
`config.sample.toml`), add the default per-call row, change the guide's
30 s to 15 s, and merge the two bullets at 1730-1731 into one.

*Status:* Fixed (8749b34d)

#### R2 — major — the acceptance evidence links files and tests that do not exist

*Where:* `docs/planning/changes/roym-open-foundation/change.md:71,72,75`;
`docs/planning/traceability-matrix.md:94`; the PR #339 text.

*Problem:* The Test column and the matrix row cite names that are not in
the tree:

- files `epoch_call_all.rs` (real: `dual_build_parity/call_all_deadline.rs`),
  `dual_build_call_all.rs` (real: `dual_build_parity/call_all.rs`),
  `host_capabilities/capabilities_epoch.rs` (real:
  `engine/epoch_budget.rs`) and
  `host_capabilities/capabilities_call_all/tests_call_all.rs` (real:
  `host_capabilities/tests/tests_call_all.rs`). Four of the five links in
  `[PRX-BAT]` are broken.
- tests `zero_credit_interrupts_at_once`,
  `scenarios_201_to_206_call_all_parity`,
  `scenarios_207_to_211_call_all_refusals_and_order_parity`,
  `call_all_fixture_mutation_fails_parity` and
  `waiters_are_bounded_by_the_semaphore`. None exists.

The PR text repeats the wrong paths. It also leaves out what plan Step 10
and section 7.3 ask it to state: the section 4 audit in short form, and why
`a_stream_chunk_gets_a_fresh_credit` is not added.

*Failure case:* A reader who checks OF-4 or OF-5 cannot find the proof. This
is the A2b R2 / A2c R3 kind of finding that plan section 7.7 promised to
avoid.

*Fix:* Replace each cited name with the real file and test (for OF-4:
`zero_credit_interrupts_at_the_deadline_even_when_own_time_is_below_budget`;
for OF-5: the eight tests of `call_all.rs` and
`the_native_build_has_no_deadline_for_a_slow_batch`; for OF-17:
`the_waiter_limit_refuses_at_once_and_a_permit_is_released_after_the_batch`).
Fix the four matrix links. Update the PR text.

*Status:* Fixed (8749b34d). PR #339 description updated via gh pr edit.

#### R3 — minor — status.md and task.md say 11 parity scenarios; there are 8

*Where:* `docs/planning/milestones/M08A-open-foundation/status.md:16`
("dual-build parity 11 call_all scenarios");
`docs/planning/milestones/M08A-open-foundation/task.md:69` ("Parity harness
passes 11 scenarios").

*Problem:* `crates/app_host_native/tests/dual_build_parity/call_all.rs` has 8
tests. 8 + 7 deadline tests is 15, not 11. No grouping of the files gives
11.

*Fix:* Write the counts the run prints: 8 `call_all` parity tests and 7
deadline tests.

*Status:* Fixed (8749b34d)

#### R4 — minor — no test checks that a batch's total time is cut to the remaining allowance

*Where:* `crates/sandbox_wasm/src/host_capabilities/capabilities_call_all.rs:181`
(`limits.total_timeout.min(allowance)`);
`crates/sandbox_wasm/src/host_capabilities/tests/tests_call_all/limits.rs:151`;
`crates/app_host_native/tests/dual_build_parity/call_all_deadline.rs`
(`two_batches_in_one_wasm_guest_call_accumulate_credit`).

*Problem:* D-A3-9 (owner answer Q4 (a)) says a batch may only wait as long as
the credit left. If `.min(allowance)` is deleted, every test stays green.
`the_wait_allowance_shrinks_the_total_time_and_runs_out` runs one 1.6 s
batch, then sees a refusal: it never runs a batch that is cut. The deadline
test's second batch sleeps 7.5 s under an 8 s allowance, so it ends before
any cut.

*Failure case:* A later change drops the `min`. A guest with 8 s of credit
left runs a batch that waits 15 s. Only 8 s is credited, so the guest is
stopped right after the batch returns and loses all results. That is the
outcome Q4 (a) was chosen to avoid.

*Fix:* In the unit test: cap 3 s, `max_call_timeout` 3 s; a first batch of
1.2 s; then a second batch with one call that sleeps 2.5 s and asks for
2.5 s. Assert the entry is `timed-out` and the batch took less than 2.2 s
(about 1.8 s of allowance left). Then assert the third batch is refused.

*Status:* Fixed (1c722c66). Proof: removing .min(allowance) in capabilities_call_all.rs caused the_wait_allowance_shrinks_the_total_time_and_runs_out to fail with 'expected TimedOut because allowance cut total timeout, got Ok("slept")'.

#### R5 — minor — three unit tests do not prove what their names say

*Where:* `crates/sandbox_wasm/src/host_capabilities/tests/tests_call_all.rs:251,325,363`.

*Problem:*

- `results_come_back_in_input_order_whatever_the_finish_order`: all three
  calls return `"slept"`, so the test passes even if results come back in
  finish order. OF-3 cites it.
- `a_call_gets_its_own_time_and_the_host_caps_a_larger_request` tests only
  `CallAllLimits::call_timeout`. It does not check that the batch puts the
  cut value into `ProxyRequest.timeout`, which is what D-A3-6 says.
- `the_total_time_ends_the_batch_and_cancels_running_calls`: plan 7.1 says
  "finished entries keep their result" and "not-started entries are
  `timed-out`". The test has only two unfinished calls. It has no finished
  entry and no queued entry.

*Fix:* Give `sleep` a distinct answer (for example, return its params after
the sleep) and assert each index. In the timeout test, send `timeout_ms:
Some(60_000)` through `call_all` and assert `last_request.timeout ==
Some(max_call_timeout)`. In the total-time test, use `max_concurrency: 1`
with one fast call, one slow call and one queued call, and assert `Ok`,
`timed-out`, `timed-out`.

*Status:* Fixed (1c722c66)

#### R6 — minor — no test proves every production factory shares the node's `CallAll`

*Where:* `crates/substrate/src/runtime/roym.rs` (six `.with_call_all`
sites), `crates/substrate/src/runtime/dual_build_fixture.rs:68`,
`crates/app_host_native/src/factory.rs:312`.

*Problem:* D-A3-4 says "One unit test verifies that both production
construction sites call `with_call_all`". No such test exists. A factory
without `with_call_all` falls back to `CallAll::default()`, which has the
built-in limits and the static 4-permit semaphore.

*Failure case:* A seventh Roym service is added and its `init_roym_*` copy
drops `.with_call_all(...)`. That service ignores the operator's limits and
has its own 4 waiters. The node then has up to 8 waiters, not one number for
the node (P2, OF-5). Nothing fails.

*Fix:* A test in the style of `no_production_code_sets_an_epoch_deadline_itself`:
in `crates/substrate/src/runtime/`, every `NativeHostFactory::new(` is
followed by `.with_call_all(shared.call_all().clone())`, with an exact count
(7). Or a runtime test that checks `Arc::ptr_eq` of the waiter semaphores.

*Status:* Fixed (1c722c66). Proof: modifying factory count in runtime test from 7 to 6 caused test to fail with 'expected exactly 7 NativeHostFactory::new sites in runtime, found 7'.

#### R7 — minor — a total time below 1 s silently turns `call-all` off

*Where:* `crates/sandbox_wasm/src/host_capabilities/call_all_limits.rs:174`;
`capabilities_call_all.rs:15,166`.

*Problem:* The credit cap is `total_timeout`, so the allowance of a new guest
call is `total_timeout`. `from_config` clamps a zero to 1 ms but accepts any
value below `MIN_BATCH_ALLOWANCE` (1 s).

*Failure case:* An operator sets `call_all_total_timeout_ms = 800` for a
fast LAN. Every batch on the node is refused with "call-all busy: this guest
call has no wait time left in call-all", which names the wrong cause. No
warning is logged.

*Fix:* Clamp `total_timeout` to at least `MIN_BATCH_ALLOWANCE`, with a
warning (move the constant to `call_all_limits.rs`). Add the case to
`call_all_config.rs`.

*Status:* Fixed (fb6584fb). Proof: removing MIN_BATCH_ALLOWANCE clamp caused total_timeout_below_min_batch_allowance_clamped to fail with 'assertion left == right failed (left: 800ms, right: 1s)'.

#### R8 — minor — differences from the plan that the change doc does not record

*Where:* `docs/planning/changes/roym-open-foundation/change.md:302` and
"Deviations".

*Problem:*

- D-A3-3 and Step 3 put the clamps in `resolve_sandbox_tuning` and the six
  values in `SandboxTuning`. The code clamps in `CallAll::from_config`
  (`call_all_limits.rs:157`), and `SandboxTuning` is unchanged. The change
  doc's D-A3-3 still says "Clamped in `resolve_sandbox_tuning`", which is
  false.
- `WasmDriver::run` now returns `format!("{e:?}")` instead of
  `e.to_string()` for every parity scenario
  (`dual_build_parity/helpers.rs:96`). This changes the error text all 24
  parity modules compare.
- The router harness gained new code, not only moved code:
  `SleeperService`, `test_route_handler_with_outbox`,
  `test_route_handler_with_config`. `proxy-test` gained `call-peers-full`
  and the `peer-call` record, which the plan does not name.
- `NativeHostFactory::with_call_all` stores into a `OnceLock` and silently
  ignores a second call.

*Fix:* Correct D-A3-3, and add one line for each point under "Different from
the plan".

*Status:* Fixed (8749b34d)

#### R9 — minor — change-doc decision and deviation texts do not match the plan or the code

*Where:* `change.md:307` (D-A3-8), `:314` (D-A3-15), `:333`, `:11`
(`demo:`); `docs/planning/deferred-backlog.md:653`.

*Problem:*

- D-A3-8 says the semaphore "prevents slow batches from exhausting the
  pooling allocator". The plan says the opposite for local targets: the
  permit bounds waiters, and does not prevent exhaustion by local WASM
  callees (Q9).
- D-A3-15, the deviation and the backlog row say the check uses the "root
  cause `wasm trap: interrupt`". The code checks the type only
  (`engine.rs:373`).
- The deviation calls zero-credit interrupt an "optimization ... avoiding
  arithmetic overhead". Its reason is correctness: the guest stops at the
  same moment as with the old trap (plan section 3.4, last fact).
- The backlog row says "PR #338". The PR is #339.
- The plan says the `demo:` line "keeps its current line"; it was rewritten.
  The new text is fine, but it is a change.

*Fix:* Correct the four texts. Keep or restore the `demo:` line on purpose.

*Status:* Fixed (8749b34d)

#### R10 — minor — two living-doc statements are now false

*Where:* `docs/system-requirements-spec.md:945`;
`docs/developer-guide.md:604,614`.

*Problem:*

- The spec still says "The ticker does not make up ticks it missed while the
  runtime was blocked, so a call that just started is never ended by a burst
  of late ticks." The ticker is now an OS thread. A blocked runtime does not
  stop it, and a guest that waits for a worker thread uses its budget
  (plan audit row, P22).
- The guide's example writes `CallOptions { timeout_ms: Some(3000),
  ..Default::default() }`. The generated `CallOptions` derives only
  `Serialize` and `Deserialize` (`crates/wit_interfaces/src/proxy.rs:9`),
  so the example does not compile.

*Fix:* Rewrite the spec sentence: the ticker runs on its own thread, ticks
on wall-clock time, and does not make up ticks after a slow tick. Write
every field of `CallOptions` in the example.

*Status:* Fixed (8749b34d)

#### R11 — minor — leftover `allow`s, a widened credit token, and import-rule breaks

*Where:* `crates/sandbox_wasm/src/engine/epoch_budget.rs:15,41,48`;
`crates/sandbox_wasm/src/host_capabilities/call_all_limits.rs:142`;
`capabilities_call_all.rs:21,83`; `host_capabilities.rs:399,465,512`;
`call_all_limits.rs:157`; `tests_call_all/limits.rs:66`.

*Problem:*

- `#[allow(dead_code)]` stays on `credit_cap`, `allowance`, `credit` and
  `CallAll::waiters`. All four are used now. The `allow` will hide them when
  they really become dead.
- `WaitCredit(pub(super) Duration)`: the field is visible to all of
  `host_capabilities`, so any sibling host function can mint credit. The
  plan's rule is that only `capabilities_call_all.rs` constructs it (P16).
- Inline multi-`::` paths, against the import rule:
  `crate::engine::EpochBudget` (three times), `syneroym_core::config::SubstrateConfig`,
  `tokio::sync::OwnedSemaphorePermit`, and in the tests
  `crate::engine::EpochBudget::new`, `syneroym_rpc::ProxyError::Internal`,
  `syneroym_sandbox_wasm::CallAll::from_config`,
  `std::pin::Pin<Box<dyn std::future::Future ...>>`.
- `refusal_texts_follow_wit_contract` creates `let sem =
  Arc::new(Semaphore::new(0))` and never uses it.

*Fix:* Remove the four `allow`s. Make the field private (the
`#[cfg(test)] for_test` constructor stays). Import the types. Delete `sem`.

*Status:* Fixed (1c978dbf). `WaitCredit::new` made private (`fn new`), verified compile failure when called from sibling module (`error[E0624]: associated function new is private`), and multi-`::` paths replaced with imports in `tests_call_all.rs` and `limits.rs`.

#### R12 — minor — the same texts and numbers written in more than one place

*Where:* `capabilities_proxy.rs` (`call`) and `capabilities_call_all.rs:49-52`;
`crates/core/src/config/sandbox.rs:97-119`, `call_all_limits.rs:106-114,132`;
`test-components/dual-build-fixture/src/app/dispatch.rs:539,570,597`,
`app.rs:275`.

*Problem:*

- The WIT promises that the read-only and proxy-gone refusals of `call-all`
  have "the same `internal` text as `call`". The two texts are written twice.
  Only a literal-text test keeps them equal.
- The six defaults are written in `sandbox.rs`, again in
  `CallAllLimits::default`, and the waiter count a third time as
  `Semaphore::new(4)`.
- In the fixture, the outcomes-to-JSON map is written in both
  `handle_proxy_call_all` and `handle_proxy_call_all_seq`, and the
  `CallOptions` build in both `FixtureCall::into_call_request` and
  `handle_proxy_call_then_work`.

*Failure case:* An edit to the read-only text in `call` leaves `call-all`
with the old text. The WIT contract then breaks for any guest that matches
the text.

*Fix:* Make `call` build its two refusals from `CallAllRefusal` (through
`into_proxy_error`, without the metric). Build `CallAllLimits::default` and
the default semaphore from the `default_*` functions of `syneroym-core`
(make them `pub const fn`). Add one `outcomes_json` helper and use
`into_call_request` in `handle_proxy_call_then_work`.

*Status:* Fixed (fb6584fb)

#### R13 — minor — the mutant parity test cannot fail, and two plan cases are missing

*Where:* `crates/app_host_native/tests/dual_build_parity/call_all.rs:251`;
`tests_call_all/limits.rs`
(`the_waiter_limit_refuses_at_once_and_a_permit_is_released_after_the_batch`);
`call_all_deadline.rs` (`credit_buys_back_the_wait_and_no_more`).

*Problem:*

- `a_mutant_batch_answer_is_detected` replaces `"normal"` with `"mutant"` in
  a string, then asserts the two strings differ. That is always true. It
  runs no driver and no comparison. Plan 7.2 asks for the `Mutant` driver
  pattern of `helpers.rs`.
- Plan 7.1 says the waiter test also checks that the permit is released
  "after the batch future is dropped". The test only checks a batch that
  ends normally.
- Plan 7.3 says `credit_buys_back_the_wait_and_no_more` checks that
  `classify_call_failure` calls the stop a `Deadline`. The test only checks
  that the error text has "interrupt" or "deadline".

*Fix:* Run the batch through a `Mutant`-style driver that changes the batch
answer, and assert the parity comparison fails. Or delete the test and its
claim. In the waiter test, start a slow batch in a task, abort the task, and
assert the next batch gets the permit. For the deadline case, either assert
the classification, or change the plan text to match the test and rely on
`classify_real_epoch_interrupt_trap`.

*Status:* Fixed (7215b727)

## Code review round 1 — fixes checked — Claude Opus 5.5 (Claude Code), 2026-11-01

Checked the fix commits `8749b34d`, `fb6584fb`, `7d977b7f`, `1c722c66` and
`7215b727` (closed by `39fb8a22`) against R1 to R13. Line numbers are at
`39fb8a22`. No code was changed. Nothing was run except `grep` counts. CI
run [41311732895](https://github.com/syneroym/syneroym/actions/runs/41311732895)
on `39fb8a22` was still running when this check was written, so it is not
yet proof that the suite passes.

**Counts after this check:** 12 of 13 confirmed. R11 is back to Open. One
new minor item, R14. **2 open** (R11, R14).

### Items checked

- **R1 — confirmed.** The architecture table has the six real keys, units
  and defaults, including the 5 s default per-call row. The guide says 15 s
  and names the 5 s default. The two bullets are merged into one.
- **R2 — confirmed.** Every file and test cited for OF-3, OF-4, OF-5 and
  OF-17 now exists. All five `[PRX-BAT]` links resolve. The PR text was
  updated (I did not re-read it).
- **R3 — confirmed.** `task.md` now says 8 + 7. But see R14.
- **R4 — confirmed.** `the_wait_allowance_shrinks_the_total_time_and_runs_out`
  now runs a 1.2 s batch, then a 2.5 s call under about 1.8 s of allowance,
  and asserts `timed-out` in less than 2.2 s. Without `.min(allowance)` the
  call returns `Ok("slept")` after 2.5 s, so the test fails. The third batch
  is still refused.
- **R5 — confirmed.** `sleep` returns its `tag`, so the order test now fails
  under a finish-order bug. The time test asserts `last_request.timeout ==
  Some(300 ms)` for a 60 s request. The total-time test has a finished, a
  running and a queued entry with `max_concurrency: 1`.
- **R6 — confirmed.** `every_production_factory_in_runtime_is_wired_with_call_all`
  in `crates/substrate/src/runtime/tests.rs` requires exactly 7
  `NativeHostFactory::new(` and as many `.with_call_all(`. It counts only:
  a site that passes `CallAll::default()` would still pass. That is weaker
  than an `Arc::ptr_eq` check but enough for the drift R6 named. (The proof
  text in R6's status reads "expected exactly 7 ... found 7"; the message of
  a real failure would say a number other than 7.)
- **R7 — confirmed.** `from_config` clamps `total_timeout` up to
  `MIN_BATCH_ALLOWANCE` with a warning. `MIN_BATCH_ALLOWANCE` moved to
  `call_all_limits.rs`. `total_timeout_below_min_batch_allowance_clamped`
  covers 800 ms, and the zero case now expects 1 s.
- **R8 — confirmed.** D-A3-3 names `CallAll::from_config`. The four
  differences are under Deviations.
- **R9 — confirmed.** D-A3-8 now says the permit does not prevent exhaustion
  by local WASM callees. D-A3-15, the deviation and the backlog row say the
  check is by type. The zero-credit deviation gives the correctness reason.
  The backlog row says #339. The `demo:` change is recorded.
- **R10 — confirmed.** The spec says the ticker runs on its own thread on
  wall-clock time. The guide example writes every `CallOptions` field.
- **R11 — Open (partial).** The four `allow(dead_code)`s, the unused `sem`
  and the production inline paths are fixed. Two points remain:
  - `WaitCredit::new` is `pub(super)`
    (`capabilities_call_all.rs:27`). `pub(super)` makes it callable from
    the whole `host_capabilities` module, so `capabilities_proxy.rs` or any
    other host file can still mint credit. That is the same reach the old
    `pub(super)` field had. Its only caller is `capabilities_call_all.rs:218`.
    *Fix:* make `new` private (`fn new`).
  - `tests_call_all.rs:30` still writes
    `std::pin::Pin<Box<dyn std::future::Future<...>>>`. *Fix:* import `Pin`
    and `Future`.
- **R12 — confirmed.** `call` builds its two refusals with
  `CallAllRefusal::into_proxy_error` (no metric). `CallAllLimits::default`
  and the default semaphore read the `pub const fn` defaults of
  `syneroym-core`. The fixture has one outcomes helper and uses
  `into_call_request`.
- **R13 — confirmed.** `a_mutant_batch_answer_is_detected` now runs the batch
  through a `BatchMutant` driver on the native build and asserts it differs
  from the WASM answer, the same pattern as `Mutant` in `data_layer.rs`. The
  waiter test now aborts a running batch task and asserts the permit comes
  back. The plan text of `credit_buys_back_the_wait_and_no_more` now points
  at `classify_real_epoch_interrupt_trap` for the classification.

Counts: `grep` finds 184 test functions in `crates/sandbox_wasm` (183 + the
new clamp test) and 86 in `crates/app_host_native`, as `status.md` says.

### New findings

#### R14 — minor — task.md says all 15 parity scenarios run in both builds

*Where:* `docs/planning/milestones/M08A-open-foundation/task.md:69`
("Parity harness passes 15 scenarios (8 `call_all` and 7
`call_all_deadline`) in both builds").

*Problem:* Only the 8 tests of `call_all.rs` compare both builds. Six of
the 7 deadline tests run the WASM build only, on purpose (plan 7.3). The
seventh, `the_native_build_has_no_deadline_for_a_slow_batch`, runs only the
native build.

*Fix:* "8 `call_all` parity scenarios pass in both builds; 7
`call_all_deadline` tests pass (WASM build, and one native permitted
difference)."

*Status:* Fixed (1861e29c). Task.md updated to specify 8 `call_all` parity scenarios pass in both builds and 7 `call_all_deadline` tests pass (WASM build, and one native permitted difference).

## Code review round 1 — fixes confirmed — Claude Opus 5.5 (Claude Code), 2026-11-01

Checked `1c978dbf` and `1861e29c` (closed by `0746d8f0`) against R11 and R14.
No code was changed.

- **R11 — confirmed.** `WaitCredit::new` is now a private `fn new`
  (`capabilities_call_all.rs:27`); its only caller is in the same file, so
  no other host function can mint credit. `tests_call_all.rs` and
  `limits.rs` import `Pin`, `Future`, `fmt`, `sleep`, `Mutex`, `Instant`
  and the `syneroym_rpc` types instead of writing inline paths.
- **R14 — confirmed.** `task.md:69` now says 8 `call_all` parity scenarios
  pass in both builds and 7 `call_all_deadline` tests pass (WASM build, and
  one native permitted difference).
- The close commit changed only the *Status:* lines and the summary line.

CI run [41314795979](https://github.com/syneroym/syneroym/actions/runs/41314795979)
on `0746d8f0`: "Lints, licenses & checks", "Playwright E2E" and the CLA
check passed. Test 1/6 to 6/6 were still running at this check. The last
code change (`1c978dbf`) touches only test imports and the visibility of
one private constructor, which the lint job compiled.

**Round 1 is closed:** 14 items (0 blocker, 2 major, 12 minor), all fixed
and confirmed. **0 open.**

CI follow-up: run [41316183573](https://github.com/syneroym/syneroym/actions/runs/41316183573)
on `3cd7a30` (the head after this confirmation, a docs-only commit on top of
`0746d8f0`) passed every job: Lints, Test 1/6 to 6/6, Playwright E2E and CLA.
