# M08A slice A3 — Batched outbound call (`call-all`): implementation plan

*Reader: the developer (or AI) who implements slice A3. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan reviewed 2026-11-01. Owner answered Q1 (a), Q2 (a), Q3 (a)
on 2026-11-01, and Q4 (a) to Q9 (a) on 2026-11-01. Sections 5 and 6 follow
these answers. Plan review round 1: findings P1–P15 were folded in; the
fixes check reopened six and added P16–P19 (see slice-a3-review.md).**

**Size and PR.** M. One PR on `feat/m08a-a3-call-all`, with the commits of
section 6 in that order. The branch starts from `main` at `54993066` (A2 is
complete: A2a, A2b and A2c are merged). A3 touches `sandbox_wasm`, the WIT,
the dual-build shim and test fixtures. It touches no Roym service. A4 needs
A3 (task.md section 4).

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`).
  - Functions of at most 100 lines. Match arms of at most 10 lines. Files
    of at most 800 lines. `proxy::Host::call` is 126 lines today
    ([capabilities_proxy.rs:41-167](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_proxy.rs#L41)),
    so Step 4 splits it. Do not copy it.
  - WASM and WIT rules: keep the `.wit` boundary clean, and remember the
    cross-compile to `wasm32-wasip2`. After a WIT change, rebuild the WASM
    artifacts (`mise run build:test-components`, `mise run build:roym`)
    before any test that loads them.
  - No planning ids in code: no "M08A", "A3", "OF-4", "D-OF-1", "D-A3-3" in
    code, comments or test names. ADR ids are fine. A comment says why, not
    which plan.
  - Comments never cite size rules or line counts.
  - Keep every "why" comment beside code that stays (A2b R8).
  - The import clean-up pass, the deferred backlog, the living-docs
    close-out, the ratchets (`check-file-lengths`, `check-duplication`,
    `check-lint-suppressions`).
  - Tests never hardcode a port.
- [task.md](task.md): section 1 (X3) and "A3 — Batched outbound call".
- [roym-open-foundation change doc](../../changes/roym-open-foundation/change.md):
  "Batched outbound call", OF-3, OF-4, OF-5, OF-17, decisions D-OF-1,
  D-OF-2, D-OF-5, D-OF-6, Deviations.
- [slice-s0-implementation-plan.md](slice-s0-implementation-plan.md) and the
  spike [epoch_wait_credit.rs](../../../../crates/sandbox_wasm/tests/epoch_wait_credit.rs).
  A3 turns the spike into production code. The spike test stays as it is. It
  pins the Wasmtime behaviour that A3 relies on.
- [slice-a2a-review.md](slice-a2a-review.md),
  [slice-a2b-review.md](slice-a2b-review.md) and
  [slice-a2c-review.md](slice-a2c-review.md): section 7.5 lists how A3 avoids
  each kind of finding.
- [deferred-backlog.md](../../deferred-backlog.md) section 9 (the row "A
  real epoch-deadline trap is classified as `Other`", line 488) and section
  11 (lines 535 to 539).

### Terms used in this plan

- **Epoch, tick, epoch deadline, wait credit, own time:** as in the S0 plan,
  section 0. One tick is 100 ms (`EPOCH_TICK_MS`,
  [engine.rs:317](../../../../crates/sandbox_wasm/src/engine.rs#L317)). The
  epoch deadline is the time at which Wasmtime stops a running guest. The
  default is 5 s. **Wait credit** is the time a guest spent waiting inside
  `call-all`. The host does not count it against the guest. **Own time** is
  the wall-clock time of the guest call minus the credit.
- **Batch:** one `call-all` host call. It holds a list of calls.
- **Waiter:** a guest instance that sits inside a `call-all` and waits. It
  holds one slot of Wasmtime's instance pool while it waits.
- **Instance pool:** Wasmtime's pooling allocator. It holds
  `max_concurrent_instances` slots (10 by default,
  [sandbox.rs:39](../../../../crates/core/src/config/sandbox.rs#L39)). A
  guest that cannot get a slot fails at once. It does not wait.
- **Fixture:** `test-components/dual-build-fixture`. One Rust source tree,
  built as a WASM component and linked in natively. It is a member of the
  workspace (it is built with `cargo component build -p
  syneroym-test-dual-build-fixture`, `mise.toml` line 41).
- **Parity harness (app host):**
  `crates/app_host_native/tests/dual_build_parity.rs`. It runs the fixture
  on both builds and compares the results.

## 1. What A3 builds, in one paragraph

A3 adds one function, `call-all`, to the `syneroym:proxy/proxy` WIT
interface, next to `call`. A guest gives it a list of calls. The host runs
them at the same time, at most 8 at once for one guest instance. It gives
each call its own time limit. It gives the whole batch a total limit. It
returns one result per call, in the input order. The host sets every limit.
The guest cannot raise any of them. The time the guest waits in `call-all`
does not count against its epoch deadline, up to a cap. To make this true,
the store now uses Wasmtime's epoch-deadline callback in place of the plain
deadline trap (D-OF-1, proved by the S0 spike). Each call in a batch is
built by the same function as a single `proxy.call` and sent through the
same `ServiceProxy::invoke`, so it passes the same checks. The native build
uses the same `HostState` code, so both builds give the same results and
the same limits (OF-5). A node-wide limit on waiters keeps slow batches
from filling the instance pool. No Roym service calls `call-all` in A3. A4
does. The proof that it works is a test fixture, a scripted `ServiceProxy`,
and tests in four places (section 7). A correct existing guest sees no
change (section 4).

## How this is shown to an end user

An end user sees nothing new. Every screen works as before. The visible
result comes in A4: the Hub runs a directory search in one call.

- A developer can run `cargo nextest run -p syneroym-app-host-native --test
  dual_build_parity call_all`. It shows that the same batch gives the same
  list of results, in the same order, in the WASM build and in the native
  build. It shows a good call, a failed call, a slow call and a refused
  batch.
- A developer can run `cargo nextest run -p syneroym-app-host-native --test
  dual_build_parity call_all_deadline`. It shows a guest that waits 6 s in a
  batch and is not stopped, and a guest that only waits 6 s in a plain call
  and is stopped.
- A developer can run `cargo nextest run -p syneroym-sandbox-wasm
  host_capabilities::tests::tests_call_all`. It shows each limit (target
  count, concurrency, per-call time, total time, waiters).

**Demo:** none. A3 adds no command, no screen and no working path that a
viewer can see. The change doc's `demo:` list keeps its current line. A5
builds the demo (task.md, "Demo").

## 2. Points for the owner

Each question has the options, a recommendation and the reason. Section 5
and section 6 follow the recommendations. The owner's answer goes under
each question.

Three choices are not questions, because earlier decisions already settle
them: `call-all` is in `syneroym:proxy` beside `call` and reuses its target,
options and error types (D-OF-6); params are JSON text (D-OF-6); and the
extension uses `epoch_deadline_callback` with the credit capped at one
total limit per guest call (D-OF-1).

### Q1. What does a guest get back from `call-all`?

The change doc sketch has a record `call-outcome { target-index, result }`.
Results come back in input order, so `target-index` repeats what the
position already says.

- (a) `result<list<result<string, proxy-error>>, proxy-error>`. The outer
  `result` is the answer for a refused batch. The list has one entry per
  call, in input order. No new record.
- (b) A list of `call-outcome` records with `target-index`, as in the
  sketch.

*Recommendation:* (a). It is shorter. A guest zips the input list with the
output list and needs no index. An index only helps when the order can
differ, and the order never does. (b) adds a record and a field that can
disagree with the position.

Owner answer: (a), 2026-11-01.

### Q2. How does a guest see a refused batch?

A batch can be refused before any call runs: too many calls, the node has
too many waiters, the guest used its wait allowance (Q4), the guest is a
read-only after-step instance, or the proxy is gone.

- (a) Reuse `proxy-error`. Too many calls is `unsupported-target` with the
  limit in the text. Every other refusal is `internal` with a fixed text
  that starts with `call-all busy:` or is the same text `call` uses. No
  change to any existing WIT type.
- (b) Add a variant to `proxy-error`, for example `batch-refused`.

*Recommendation:* (a). A new variant changes `proxy-error`, which every
guest already imports. `unverified`: whether Wasmtime still instantiates a
guest built against the older variant. Wasmtime compares variant types
exactly in other cases. (a) cannot break an existing guest. A guest that
needs to tell "busy" from "too many" can read the text, and the host counts
each reason in a metric (Q8).

Owner answer: (a), 2026-11-01.

### Q3. Where do the limits live, and what are the first values?

- (a) Config fields in `[roles.app_sandbox]` (`AppSandboxRole`), with the
  defaults below. An operator can change them. The sample config shows them.
- (b) Constants in the code. No config.

First values: at most **16** calls in a batch (the change doc's value), at
most **8** running at once per guest instance (the change doc's value), a
default per-call time of **5 s**, a per-call cap of **10 s**, a total time
(and credit cap) of **15 s**, and at most **4** waiters on the node (Q5).

*Recommendation:* (a), because every other time and size limit of the
sandbox is a config field (`dispatch_epoch_timeout_secs`,
`max_concurrent_guest_http_per_service`). The times follow from the rest:
16 calls at 8 at once is two waves, and two waves of the 5 s default is 10
s, inside 15 s. One guest call then lasts at most 5 s of own time plus 15 s
of credit, which is 20 s. That is less than `DEFAULT_PROXY_CALL_TIMEOUT`
(30 s,
[proxy.rs:173](../../../../crates/rpc/src/proxy.rs#L173)). So a caller
that waits for a guest which runs a batch times out later than the guest.
The numbers are first values, as the change doc says. A real measure in A4
can change them.

Owner answer: (a), 2026-11-01.

### Q4. What happens when one guest call runs more than one batch?

The credit is capped at one total limit per guest call (D-OF-1). Take a
guest that runs two batches of 15 s each in one call.

- (a) A batch may only wait as long as the guest still has credit left. Its
  total limit is the smaller of the host's total limit and the credit left.
  When no credit is left, the batch is refused (`internal`, Q2). The guest
  never waits in a batch for time that the host will not credit.
- (b) A batch always gets the full total limit. Wait time past the cap
  counts as own time, so the guest is stopped when it returns to its code.

*Recommendation:* (a). With (b) the host does the work, then throws away
the result by stopping the guest. With (a) the guest gets an answer it can
act on, and the wall time of one guest call stays at 20 s with the first
values. The cost is a few lines in the check order (section 5, D-A3-9).

Owner answer: (a), with `MIN_BATCH_ALLOWANCE` (review P8), 2026-11-01.

### Q5. What limits how many guests can wait in batches at once?

The backlog row "Slow `call-all` waits can hold every guest instance slot"
([deferred-backlog.md:539](../../deferred-backlog.md)) names this: A3 needs
a limit on waiters, and a test that other calls still run when the limit is
reached. A waiter keeps its pool slot for up to 15 s plus its own time.

- (a) One node-wide limit, a `Semaphore` with `call_all_max_waiting_guests`
  permits (first value 4, clamped below `max_concurrent_instances`). A batch
  takes a permit with `try_acquire`. If there is none, the batch is refused
  at once (Q2).
- (b) The same, but the batch waits in a queue for a permit.
- (c) A limit for each service, below the node limit.

*Recommendation:* (a). The permit bounds the number of waiters on the node
(and thus bounds pool slots held by waiters to at most 4 of 10 when targets
are remote). Note that slots held by one batch in the real case equal the
waiter + its local WASM targets in flight + any caller that waits for it
with a plain `call` (e.g. `web` calling `directory`). The permit does not
prevent pool exhaustion from concurrent local WASM targets (that is handled
by Q9 (a) and tested). (b) is worse: a guest in the
queue holds its own pool slot while it waits, which is the problem the limit
solves. (c) is a second table to keep for a case that no code reaches in
A3. No Roym service calls `call-all` yet, and only A4's client search and
M08F's exchange will. A row in the backlog records (c).

Owner answer: (a), keeping the default of 4 waiters. The arithmetic is the real one (D-A3-8), and the local-target case is tested (review P4), 2026-11-01.

### Q6. Does A3 fix the deadline classification?

Today `classify_call_failure` finds an epoch deadline only by the words
"epoch" or "deadline" in the error text
([engine.rs:367-382](../../../../crates/sandbox_wasm/src/engine.rs#L367)).
The S0 spike showed that a real deadline stop is `Trap::Interrupt`, whose
text is "wasm trap: interrupt". So a real deadline is classified as `Other`.
The backlog row (line 488) says A3 fixes it. The effect: an ABAC after-step
that runs out of time is today `AbacError::Trap`, shown to the caller as
`DataLayerError::Internal`. After the fix it is `AbacError::BudgetExceeded`,
shown as `QuotaExceeded`, which is what its own comment says it should be.
A guest HTTP request that runs out of time becomes `BudgetExceeded` instead
of `Trap`. Both end in a 500 (only `Declined` is treated differently,
[guest.rs:172](../../../../crates/router/src/route_handler/http/guest.rs#L172)),
so only the log text changes.

- (a) Fix it in A3, in Step 2, with a type check (`Trap::Interrupt`) before
  the text check.
- (b) Leave it, keep the backlog row.

*Recommendation:* (a). A3 rewrites how the deadline fires, so the
classification is the same code path. A test that proves a credited guest is
stopped later must also prove the stop is classified as a deadline. The
changed ABAC error is the intended meaning, and the audit (section 4) lists
it.

Owner answer: (a), 2026-11-01.

### Q7. What if the guest's service is stopped while a batch runs?

`stop_wasm` removes the component and aborts its streams
([lifecycle.rs:166-177](../../../../crates/sandbox_wasm/src/engine/lifecycle.rs#L166)).
It has no hook for a running ordinary call. A batch that is in flight keeps
running until it ends or reaches the total limit (15 s).

- (a) Leave it. The total limit bounds the wait. Record a backlog row.
- (b) Give every batch a cancellation token that `stop_wasm` sets.

*Recommendation:* (a). The same gap exists for every ordinary call today
(backlog line 43). (b) is a new mechanism for all calls. It is not a
`call-all` problem.

Owner answer: (a), 2026-11-01.

### Q8. Metrics and logs for refused and timed-out calls now, or later?

- (a) Now. Five counters, one histogram and one gauge (`.waiting`, permits in
  use) under `substrate.proxy.call_all.` and one `warn!` for a refusal because
  of the waiter limit. About 25 lines.
- (b) A backlog row only.

*Recommendation:* (a). The `metrics` crate is already a dependency of
`sandbox_wasm`, and `substrate.proxy.calls` and `substrate.proxy.errors`
use the same style. An operator who sees the waiter limit reached needs a
signal to raise it, and the gauge lets them see active permits before
refusals occur.

Owner answer: (a), with the `.waiting` gauge (review P9), 2026-11-01.

### Q9. Local WASM targets can run out of instance-pool slots in a batch

A call to a service on the same node runs that service in a new instance
([router.rs:270-290](../../../../crates/router/src/proxy/router.rs#L270)).
A batch of 8 local WASM targets holds 1 slot for the waiter and 8 for the
callees. With a pool of 10 that fits once. Two waiters do not fit. A guest
that cannot get a slot fails at instantiation, and the call ends as a
`callee` error with code -32603. It is never a hang.

- (a) Accept. The target gets a failed outcome. Record a backlog row. A4's
  sources are mostly directories on other nodes, which use no local slot.
- (b) Count local targets against the waiter permit (each local WASM target
  holds a permit while it runs).

*Recommendation:* (a). The same limit applies today to any guest that sends
several calls at the same time. (b) couples two limits. Test it (section 7)
so the failure is known and bounded.

Owner answer: (a), with a test of the local-target pool failure (review P4), 2026-11-01.

## 3. Facts checked in the code

Every line below was read on `main` at `54993066`.

### 3.1 The WIT today

- `syneroym:proxy/proxy` has `call` and `enqueue`, and the types
  `callee-error`, `call-target`, `proxy-error`, `call-options`
  ([proxy.wit:11-103](../../../../crates/wit_interfaces/wit/proxy/proxy.wit#L11)).
  `call` is at [:77](../../../../crates/wit_interfaces/wit/proxy/proxy.wit#L77)
  and takes `target, %interface, method, params, options` and returns
  `result<string, proxy-error>`.
- `call-options.timeout-ms` is the per-call deadline. It "defaults to the
  host's 30s budget" ([:52-53](../../../../crates/wit_interfaces/wit/proxy/proxy.wit#L52)).
- The change doc sketched `call-request { target, method, params }` and
  `call-all(calls, per-call-timeout-ms)`. It has no `interface`, no
  `options`. `call` needs an interface, so the sketch cannot be used as it
  is. The slice plan sets the exact form (D-OF-6).
- Two WIT files are not symlinks to this one:
  `test-components/saga-test/wit/deps/proxy/proxy.wit` is a plain file. The
  other copies (six Roym crates, `dual-build-fixture`, `proxy-test`) are
  symlinks, so they follow the change. The `saga-test` copy has the same
  `proxy` and `saga` interfaces as `main` today (the only difference is the
  `proxy-import` world at the end), and A3 leaves it as it is, on purpose
  (section 7.2: it is the proof that a guest built against the older WIT
  still instantiates).

### 3.2 The host function `call`

- `impl proxy::Host for HostState`
  ([capabilities_proxy.rs:35](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_proxy.rs#L35)).
  `call` does, in this order:
  1. refuse if `self.read_only` (`internal`, "stage-4 after-step instances
     may not originate proxy calls", [:53](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_proxy.rs#L53));
  2. upgrade the `Weak<dyn ServiceProxy>` (`internal`, "proxy unavailable");
  3. parse `params` as JSON (an empty text is `null`; a bad text is
     `internal`);
  4. read the options and parse the protocol tag (`unsupported-protocol`,
     [:81](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_proxy.rs#L81));
  5. resolve a `dependency` target against the component's own app instance
     (`dependency-not-bound` when it does not resolve);
  6. choose the caller: the real `self.caller` if the target is the
     component itself, otherwise `CallerContext::service_system`
     ([:145](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_proxy.rs#L145));
  7. build a `ProxyRequest` with `origin: CallOrigin::Guest` and
     `timeout: timeout_ms` ([:150](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_proxy.rs#L150));
  8. `service_proxy.invoke(req)` ([:163](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_proxy.rs#L163));
  9. return a string result raw and any other JSON as text.
- `ProxyError` to WIT: `map_proxy_error`
  ([:14-38](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_proxy.rs#L14)).
  `Timeout` becomes `timed-out`.
- Steps 3 to 7 need only `&self`. Only step 8 awaits. This lets a batch
  build all requests first and run them without holding `&mut self`.

### 3.3 What `invoke` checks (so a batch can reuse it)

- `ProxyRouter::invoke` calls `invoke_inner`
  ([router.rs:634-642](../../../../crates/router/src/proxy/router.rs#L634)).
  `invoke_inner` ([:500](../../../../crates/router/src/proxy/router.rs#L500))
  runs the guest native-capability gate (`check_native_capability_gate`,
  [:511](../../../../crates/router/src/proxy/router.rs#L511); it refuses node
  interfaces and another service's native capabilities), looks the target
  up in the endpoint registry ([:524](../../../../crates/router/src/proxy/router.rs#L524)),
  then runs the local path (`invoke_local_guarded`, with the idempotency
  fence) or the remote path (`invoke_remote`, with identity, the end-to-end
  handshake and encryption).
- Every check that makes a call safe lives in step 5 to 7 of `call` or in
  `invoke`. A batch that builds each request with the same function and
  sends it through the same `invoke` has the same checks. Nothing else
  needs to be copied.
- A keyed failure is recorded as a dead letter inside `invoke`
  ([router.rs:18](../../../../crates/router/src/proxy/router.rs#L18), `record_failed_call`).
  In a batch, the per-call time bounds the whole call, retries included. A
  call that the batch's timer cuts before `record_failed_call` runs is not
  dead-lettered; its claim stays in flight until its window ends. A call with
  a 250 ms grace on the outer task timer (`CALL_CANCEL_GRACE`) does reach
  `record_failed_call` if its single attempt times out on its own (D-A3-5,
  D-A3-6). A `call` through the single-call path still dead-letters as
  before.
- The local path applies `req.timeout` (30 s if `None`) with
  `time::timeout` ([router.rs:225-286](../../../../crates/router/src/proxy/router.rs#L225)).
  The remote path applies it **for each attempt**
  ([:449-470](../../../../crates/router/src/proxy/router.rs#L449)); an
  idempotent or keyed call can retry, so one call can last longer than its
  timeout. The `ServiceProxy` trait itself promises no deadline (the fdae
  fetch comment says `request.timeout` is advisory,
  [fdae_fetch.rs:113](../../../../crates/rpc/src/fdae_fetch.rs#L113)).
  So a batch also wraps each call in its own `tokio::time::timeout`.
- A same-node WASM target runs in a **new** store and instance
  (`execute_wasm_json`, via `build_store_and_instantiate`,
  [engine.rs:469](../../../../crates/sandbox_wasm/src/engine.rs#L469)). It
  never runs in the caller's instance. A target that is the calling service
  is no different.
- A keyed call that is already running here gets the fence's "already
  running" answer ([router.rs:71-112](../../../../crates/router/src/proxy/router.rs#L71) and [call_dedup.rs:479](../../../../crates/router/src/call_dedup.rs#L479)).
  So the same key twice at the same time is safe, and the second call
  fails.

### 3.4 The epoch deadline: every place

The store is built in exactly one place in production, and re-armed in one
other.

| Place | What it does | Line |
| --- | --- | --- |
| `build_store_and_instantiate` | `Store::new`, `epoch_deadline_trap()`, `set_epoch_deadline(ticks)`, then instantiate | [engine.rs:566-570](../../../../crates/sandbox_wasm/src/engine.rs#L566) |
| `call_resource_method` | `set_epoch_deadline(ticks)` before each stream chunk call on a long-lived store | [stream.rs:188](../../../../crates/sandbox_wasm/src/stream.rs#L188) |

`build_store_and_instantiate` has these callers. Each passes the ticks of
its call kind:

| Caller | Ticks | Line |
| --- | --- | --- |
| `prepare_wasm_execution` (RPC and proxy dispatch) | `dispatch_epoch_ticks` | [engine.rs:632-636](../../../../crates/sandbox_wasm/src/engine.rs#L632) |
| guest HTTP | `dispatch_epoch_ticks` | [guest_http.rs:104](../../../../crates/sandbox_wasm/src/engine/guest_http.rs#L104) |
| `deliver_message`, conversation on-message, on-delivery-state | `dispatch_epoch_ticks` | [auth.rs:84](../../../../crates/sandbox_wasm/src/engine/auth.rs#L84), [:264](../../../../crates/sandbox_wasm/src/engine/auth.rs#L264), [:361](../../../../crates/sandbox_wasm/src/engine/auth.rs#L361) |
| stage-4 ABAC after-step | `abac_epoch_ticks` | [auth.rs:519](../../../../crates/sandbox_wasm/src/engine/auth.rs#L519) |
| open a stream instance | `dispatch_epoch_ticks` | [guest_stream.rs:96](../../../../crates/sandbox_wasm/src/engine/guest_stream.rs#L96) |
| websocket open, message, close | `dispatch_epoch_ticks` | [guest_stream.rs:294](../../../../crates/sandbox_wasm/src/engine/guest_stream.rs#L294), [:338](../../../../crates/sandbox_wasm/src/engine/guest_stream.rs#L338), [:386](../../../../crates/sandbox_wasm/src/engine/guest_stream.rs#L386) |
| lifecycle hook (`init`, `migrate`) | `lifecycle_hook_epoch_ticks` | [lifecycle.rs:142](../../../../crates/sandbox_wasm/src/engine/lifecycle.rs#L142) |

- A store that lives longer than one call is only a stream instance. Its
  chunk calls are re-armed in `call_resource_method` (called at
  [stream.rs:332](../../../../crates/sandbox_wasm/src/stream.rs#L332),
  [:397](../../../../crates/sandbox_wasm/src/stream.rs#L397),
  [:412](../../../../crates/sandbox_wasm/src/stream.rs#L412)). The two calls
  that open a stream (`call_handle_stream_request`,
  `call_accept_stream_upload`) use the deadline set at store build.
- `Store::new` also appears in two benchmarks and two unit tests
  ([benches/wasm_engine.rs:113-145](../../../../crates/sandbox_wasm/benches/wasm_engine.rs#L113),
  [engine/tests.rs:454](../../../../crates/sandbox_wasm/src/engine/tests.rs#L454),
  `:665`). They are not production paths. They keep
  `epoch_deadline_trap()`.
- The ticker runs every 100 ms with `MissedTickBehavior::Delay`
  ([epoch_ticker.rs:19](../../../../crates/sandbox_wasm/src/engine/epoch_ticker.rs#L19)),
  so a missed tick is not made up. The engine has
  `epoch_interruption(true)`
  ([init.rs:293](../../../../crates/sandbox_wasm/src/engine/init.rs#L293)).
- The callback runs only when guest code reaches a passed deadline. Wasmtime
  checks at function entry and loop back-edges. A guest that has no code to
  run after `call-all` returns is never stopped. A Roym guest always runs
  code after it (merge, JSON), as the S0 plan says.
- The spike
  ([epoch_wait_credit.rs](../../../../crates/sandbox_wasm/tests/epoch_wait_credit.rs))
  calls `store.epoch_deadline_callback(|ctx| Ok(on_deadline(ctx.data_mut())))`
  and returns `Continue(ticks)` with the budget minus own time rounded up. It
  has the pure function `on_deadline_at` with a unit test.
- **One more fact, found while planning.** The spike's callback, with no
  credit, can end the guest up to one tick **later** than the trap does
  today. The callback runs at the epoch event, when elapsed wall time is
  between `budget - 1 tick` and `budget`. Own time is then a little less
  than the budget, so the spike code returns `Continue(1)` and the guest
  runs one more tick. Production code must not change a guest that never
  uses `call-all`. So the production callback interrupts at once when the
  credit is zero (D-A3-4).

### 3.5 Limits and config

- `AppSandboxRole` ([sandbox.rs:99](../../../../crates/core/src/config/sandbox.rs#L99))
  holds `max_concurrent_instances` (10 by default,
  [:39](../../../../crates/core/src/config/sandbox.rs#L39)),
  `dispatch_epoch_timeout_secs` (5), `max_concurrent_guest_http_per_service`
  and other limits. `resolve_sandbox_tuning`
  ([init.rs:41-91](../../../../crates/sandbox_wasm/src/engine/init.rs#L41))
  copies them, and clamps a zero (`max_concurrent_guest_http_per_service`,
  [:82](../../../../crates/sandbox_wasm/src/engine/init.rs#L82)) with a
  warning. The sample config is at
  [config.sample.toml:148](../../../../crates/substrate/config.sample.toml#L148).
- The pool reserves `STREAM_INSTANCE_POOL_HEADROOM` = 2 slots for ordinary
  calls ([engine.rs:306](../../../../crates/sandbox_wasm/src/engine.rs#L306),
  [init.rs:108-126](../../../../crates/sandbox_wasm/src/engine/init.rs#L108)).
- The proxy default call time is 30 s
  ([proxy.rs:173](../../../../crates/rpc/src/proxy.rs#L173)).
- `HostState` holds `service_proxy: Weak<dyn ServiceProxy>`, `read_only`,
  `app_instance_id`, `caller` and `component_id`
  ([host_capabilities.rs:337-398](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L337)).
  `HostState::new` has 15 arguments
  ([:413](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L413)).
  Optional state is added with a builder (`with_conversation`,
  `with_record_signer`), so no call site changes. `HostState::new` is called
  from the engine, the native factory
  ([factory.rs:286](../../../../crates/app_host_native/src/factory.rs#L286))
  and test helpers.
- `futures` and `futures-util` are workspace dependencies
  ([Cargo.toml:207-208](../../../../Cargo.toml#L207)).
  `syneroym-sandbox-wasm` depends on neither.

### 3.6 The native build

- `NativeAppHost` is a wrapper of the **same** `HostState`. Its `AppProxy`
  impl calls `HostProxy::call(&mut *state, ...)`, the WIT host trait
  implemented in `sandbox_wasm`
  ([host.rs:590-609](../../../../crates/app_host_native/src/host.rs#L590)).
  A fresh `HostState` is made for each invocation
  ([factory.rs:203](../../../../crates/app_host_native/src/factory.rs#L203),
  and `permitted_differences.rs`).
- The conversion helpers are in
  [convert/proxy.rs:5,12,28](../../../../crates/app_host_native/src/convert/proxy.rs#L5).
- So one `call_all` in `HostState` serves both builds. The native build
  gives the same limits, order and results by construction (OF-5). It has no
  epoch deadline, so the credit does nothing there. That is a permitted
  difference (D-A3-12).
- `AppProxy` is implemented in four places: the trait
  ([app_host/src/lib.rs:73](../../../../crates/app_host/src/lib.rs#L73)),
  the WASM guest
  ([guest.rs:372](../../../../crates/app_host/src/guest.rs#L372)), the native
  host, and a test host in
  [roym_core/src/signing.rs](../../../../crates/roym_core/src/signing.rs).
  All four need `call_all`.
- `bindings.rs` of the Roym crates is generated and ignored by git
  (`.gitignore:60`). Nothing is checked in to regenerate. Their guests reach
  the proxy types through the `app_host` bindings
  ([roym_directory/src/guest.rs:22](../../../../crates/roym_directory/src/guest.rs#L22)).

### 3.7 Tests and fixtures that exist

- The fixture imports the proxy and has the verbs `ProxyCallSelf`,
  `ProxyCallDependency`, `ProxyCallCrossServiceNative` and the enqueue verbs
  ([dispatch.rs:345-385](../../../../test-components/dual-build-fixture/src/app/dispatch.rs#L345),
  [app.rs:180-215](../../../../test-components/dual-build-fixture/src/app.rs#L180)).
  It has no verb that loops in guest code.
- The parity harness runs the fixture on both builds with `StubProxy`, a
  unit struct that answers only `greeter.greet`
  ([helpers.rs:254-270](../../../../crates/app_host_native/tests/dual_build_parity/helpers.rs#L254)).
  `build_wasm_stack` makes a real `AppSandboxEngine` (with its own epoch
  ticker) with the default config
  ([helpers.rs:579-686](../../../../crates/app_host_native/tests/dual_build_parity/helpers.rs#L579)).
- `helpers.rs` is on the oversized list at 869 lines
  ([oversized-test-files.txt:1](../../../../xtask/oversized-test-files.txt#L1)).
  It cannot grow. `engine/tests.rs` is at 852
  ([:25](../../../../xtask/oversized-test-files.txt#L25)). It cannot grow.
  `crates/router/tests/proxy_dispatch.rs` is at 1171
  ([:19](../../../../xtask/oversized-test-files.txt#L19)). It cannot grow.
  `host_capabilities/tests/tests_proxy.rs` has 761 lines and is not listed,
  so it may not pass 800.
- `crates/router/tests/proxy_dispatch.rs` drives the guest WIT boundary
  through a real `RouteHandler` and `ProxyRouter` with the `proxy-test` and
  `greeter` components
  ([:128](../../../../crates/router/tests/proxy_dispatch.rs#L128),
  [:363](../../../../crates/router/tests/proxy_dispatch.rs#L363)). The
  `proxy-test` driver has `call-peer` and `enqueue-peer`
  ([proxy-test/wit/world.wit](../../../../test-components/proxy-test/wit/world.wit),
  [src/lib.rs:36](../../../../test-components/proxy-test/src/lib.rs#L36)).
- `build:test-components` builds the fixture and `proxy-test`
  ([mise.toml:31-45](../../../../mise.toml#L31)). A test that loads a WASM
  artifact needs it rebuilt after a WIT change
  (`dual_build_parity` loads the pre-built file).

### 3.8 What no code does yet

- No Roym service calls `call-all`. A4 does. A3 proves the call with the
  fixture, a scripted `ServiceProxy` and the router test (section 7).

## 4. Behaviour-change audit

**Existing guests (should see nothing).**

| What | Change | Why it is safe |
| --- | --- | --- |
| `syneroym:proxy/proxy` gains `call-all` and `call-request` | Added only. `call`, `enqueue`, and all existing types are unchanged. | A guest that does not import `call-all` is not affected. Extra exports in host instances are allowed by Wasmtime (`matching.rs:154-175`). The `saga-test` fixture has an older copy of the WIT and shows that the older type set still links (section 7.2). |
| `proxy-error`, `call-options`, `call-target` | Unchanged (Q2 (a)) | — |
| `HostState::new` | Signature unchanged. New fields get defaults. | No call site changes. |
| Config | Six new optional fields in `[roles.app_sandbox]` with defaults | A config file that does not name them behaves as before. |
| Metrics | New names under `substrate.proxy.call_all.` | Added only. |

**The epoch deadline of calls that do not use `call-all`.**

| What | Change | Why it is safe |
| --- | --- | --- |
| The store uses `epoch_deadline_callback` in place of `epoch_deadline_trap` | With zero credit the callback returns `UpdateDeadline::Interrupt` at the same epoch event where the trap fires. | `Trap::Interrupt`, the same error (S0 T3). The stop time does not change (D-A3-4). A test (section 7.1) proves the zero-credit result without a clock. The existing e2e tests of a spinning guest (`guest_http_e2e.rs:467`, `:575`) and the S0 spike stay green. |
| `started` and the budget are reset where the deadline is set | Same two places as `set_epoch_deadline`. | A chunk of a stream gets a fresh budget and a fresh credit, as it gets a fresh deadline today. |
| A guest with zero credit pays one extra call of a small closure at the deadline | Only when the guest is stopped. | No cost on the hot path. |
| `proxy.call`, `enqueue`, `saga.step` give no credit | Unchanged (D-OF-1) | A single `proxy.call` can still wait 30 s with no credit. The backlog row stays. |
| `classify_call_failure` finds `Trap::Interrupt` (Q6 (a)) | A real deadline is now `Deadline`, not `Other`. | Effect by caller: the ABAC after-step error becomes `BudgetExceeded` (`QuotaExceeded` to the caller) in place of `Trap` (`Internal`); a guest HTTP request that runs out of time is `BudgetExceeded` in place of `Trap`, both 500; `execute_wasm_vals` treats `Deadline` and `Other` the same. The existing test of this function uses made-up text. A real-trap case is added. |
| The epoch ticker runs on a dedicated OS thread (`std::thread::spawn` with `sleep(EPOCH_TICK_MS)`) instead of `tokio::spawn` | Ticker advances every 100 ms regardless of worker thread CPU saturation. Sleep for one tick after each tick; missed ticks are not made up. A guest that waits for a worker thread now uses its budget during the wait, matching true wall-clock time. | Prevents concurrent CPU-bound local WASM targets from starving the runtime clock and delaying deadline expiration (P17, P22). Deadlines fire on time even when all tokio workers are busy. |

**Native build.** `AppProxy` gets `call_all`. A native app that does not call
it is not affected. No epoch deadline exists natively, so no deadline
behaviour changes.

**New behaviour of `call-all` itself (not a change, but listed so review can
check it).** Section 5, D-A3-5 to D-A3-11.

## 5. Decisions this plan makes

These follow the recommendations of section 2. Section 9 copies them into the
change doc with the same numbers.

**D-A3-1. The WIT (Q1 (a), Q2 (a)).** In `proxy.wit`, inside `interface
proxy`, right after `call`:

```wit
    /// One call of a `call-all` batch. The five arguments of `call`.
    record call-request {
        target: call-target,
        %interface: string,
        method: string,
        params: string,
        options: option<call-options>,
    }

    /// Runs the calls at the same time and returns one result per call,
    /// in the order of `calls`.
    ///
    /// Each call is checked and sent exactly as `call` does. An empty list
    /// returns `Ok([])` with no call started.
    ///
    /// Limits (all set by the host; the guest cannot raise them):
    /// - The host refuses a list longer than its maximum with
    ///   `Err(unsupported-target("call-all takes at most N calls, got M"))`.
    ///   No call runs.
    /// - The host runs at most a fixed number of calls at once per guest
    ///   instance and queues the rest.
    /// - Each call has a time limit. `options.timeout-ms` sets it, but the
    ///   host caps it. The default when `options` is absent or
    ///   `timeout-ms` is absent is the host's configured default (not
    ///   `call`'s 30 s). The time limits the whole call including retries;
    ///   for `call`, it limits each attempt.
    /// - The whole batch has a total time limit set by the host.
    ///
    /// A call that cannot finish in time has the result `timed-out`.
    ///
    /// A refused batch returns an `Err`. A refusal because the node has
    /// too many waiting guests, or because this guest call has no wait time
    /// left, is `internal` with a text that starts with `call-all busy:`.
    /// A read-only instance and a missing proxy give the same `internal` text
    /// as `call`.
    call-all: func(calls: list<call-request>)
        -> result<list<result<string, proxy-error>>, proxy-error>;
```

The `call-options.timeout-ms` field comment is updated to say that in a
batch its default is the host's configured default, not 30 s. The Rust
names are `CallRequest` and `call_all`.

- `params` is JSON text, as in `call` (D-OF-6). An empty text is `null`.
- `options` is the `call-options` of `call`. Every field has the same
  meaning in a batch, except `timeout-ms` (D-A3-6).
- A guest sees these results:

| The guest sends | The guest gets |
| --- | --- |
| An empty list | `Ok([])`. No call runs, no permit is taken, no credit is given. |
| More calls than the host maximum | `Err(unsupported-target("call-all takes at most 16 calls, got 20"))`. No call runs. The number is the configured maximum. |
| A list where some calls work and some fail | `Ok(list)` with the same length. Each entry is `Ok(text)` or `Err(proxy-error)` for that call alone. One failure never changes another entry. |
| A call with a bad `params` text, a bad protocol tag or a dependency that does not resolve | The entry of that call is `Err(...)` with the same variant and text that `call` gives. The other calls still run. |
| Calls that do not finish before their per-call time | The entry of each is `Err(timed-out)`. |
| A batch that does not finish before the total time | Every call that has not finished has `Err(timed-out)`. A call that has not started has `Err(timed-out)` too. The finished entries keep their results. |
| A read-only after-step instance | `Err(internal("stage-4 after-step instances may not originate proxy calls"))`, the same text as `call`. |
| The proxy is gone | `Err(internal("proxy unavailable"))`, the same text as `call`. |
| The node has too many waiters (D-A3-8) | `Err(internal("call-all busy: ..."))`. No call runs. |
| No wait allowance left in this guest call (D-A3-9) | `Err(internal("call-all busy: ..."))`. No call runs. |

- `proxy-error` and every other existing type stay as they are. No WIT
  version change: the product is unreleased (no migrations before release).

**D-A3-2. One request builder, one `invoke` (reuse, not a copy).** Step 4
splits `proxy::Host::call` into functions on `HostState`:

```rust
/// Steps 3 to 7 of `call`: params, options, target, caller, request.
fn proxy_request(
    &self,
    call: CallRequest,
    timeout: Option<Duration>,   // overrides options.timeout-ms; call passes the guest's own value
) -> Result<ProxyRequest, proxy::ProxyError>;

fn resolve_target(&self, target: CallTarget, routing_key: Option<&str>)
    -> Result<String, proxy::ProxyError>;   // step 5

fn reply_text(value: Value) -> String;      // step 9
```

`call` becomes: refuse if read-only, upgrade, `proxy_request`, `invoke`,
`reply_text`. `call_all` calls the **same** `proxy_request` and
`reply_text` for every element, and the same `ServiceProxy::invoke`. Nothing
that checks the caller identity, the access rule, the target, the encryption
or the idempotency fence is written twice. The step that differs is only the
timeout: `call` passes the guest's `timeout-ms` unchanged (None means 30 s,
as today); a batch passes the clamped value of D-A3-6. The order of the
checks in `call` does not change.

**D-A3-3. Limits and where they are set (Q3 (a)).** Six fields in
`AppSandboxRole`, each with a `default_*` function like the others in
[sandbox.rs](../../../../crates/core/src/config/sandbox.rs):

| Field | Default | Meaning |
| --- | --- | --- |
| `call_all_max_calls` | 16 | Most calls in one batch. |
| `call_all_max_concurrency` | 8 | Most calls running at once for one guest instance. The rest wait their turn (in order). |
| `call_all_default_call_timeout_ms` | 5000 | Time of a call that gives no `timeout-ms`. |
| `call_all_max_call_timeout_ms` | 10000 | The most a guest can ask for one call. A larger value is cut to this. |
| `call_all_total_timeout_ms` | 15000 | Time of the whole batch, queue wait included. Also the cap of the wait credit of one guest call. |
| `call_all_max_waiting_guests` | 4 | Most guests that may wait in a batch on the node at once (D-A3-8). |

`resolve_sandbox_tuning` copies them into a plain struct
`CallAllLimits { max_calls, max_concurrency, default_call_timeout,
max_call_timeout, total_timeout }` and a waiter count. It clamps with a
warning, as it does for `max_concurrent_guest_http_per_service`: a zero
becomes 1; `default_call_timeout` is cut to `max_call_timeout`;
`max_call_timeout` is cut to `total_timeout`; `max_waiting_guests` is cut to
`max(1, max_concurrent_instances - 1)`. The guest has no way to read or set
any of them. The only guest input is `options.timeout-ms`, and the host cuts
it.

`CallAllLimits` has a `Default` with the first values. It is used by the
native factory and by tests that do not set it.

**D-A3-4. The epoch budget, in production.** A new file
`crates/sandbox_wasm/src/engine/epoch_budget.rs` (with
`epoch_budget/tests.rs`):

```rust
pub(crate) struct EpochBudget {
    started: Instant,
    budget: Duration,
    credited: Duration,
    credit_cap: Duration,
}

impl EpochBudget {
    /// A budget with the given credit cap. `on_deadline` interrupts until
    /// `arm` is called. The cap is set at construction so that a
    /// `HostState` that is never armed still allows a full batch.
    pub(crate) fn new(credit_cap: Duration) -> Self;
    /// Starts a new guest call: resets the start time and the credit.
    pub(crate) fn arm(&mut self, now: Instant, ticks: u64);
    /// How much more credit this guest call may still receive.
    pub(crate) fn allowance(&self) -> Duration;      // credit_cap - credited
    /// Adds the time a batch waited, up to the cap.
    pub(crate) fn credit(&mut self, waited: Duration);
    /// What Wasmtime should do when guest code reaches the deadline.
    pub(crate) fn on_deadline(&self, now: Instant) -> UpdateDeadline {
        if self.credited.is_zero() { return UpdateDeadline::Interrupt; }
        let own = now.saturating_duration_since(self.started).saturating_sub(self.credited);
        if own >= self.budget { return UpdateDeadline::Interrupt; }
        let ticks = (self.budget - own).as_nanos().div_ceil(TICK.as_nanos()).max(1);
        UpdateDeadline::Continue(ticks as u64)
    }
}

/// The only way production code sets an epoch deadline.
pub(crate) fn arm_epoch_deadline(store: &mut Store<HostState>, ticks: u64) {
    store.data_mut().epoch.arm(Instant::now(), ticks);
    store.set_epoch_deadline(ticks);
}
```

- `TICK` is `Duration::from_millis(EPOCH_TICK_MS)`
  ([engine.rs:317](../../../../crates/sandbox_wasm/src/engine.rs#L317)).
  The budget is `ticks * TICK`.
- **The cap is set at construction**, not at `arm` time. `HostState::new` calls
  `EpochBudget::new(CallAllLimits::default().total_timeout)`. `with_call_all`
  rebuilds it from the real limits, so the cap and the limits stay in sync.
  This means a `HostState` that is never armed (the native build, and unit
  tests that do not call `arm_epoch_deadline`) still allows a batch, and the
  cap reflects the actual configured limit (P1).
- **Zero credit interrupts at once.** This is the line that makes the
  deadline of every guest that does not use `call-all` exactly what it is
  today (section 3.4, last fact). Credit is also the only case where the
  callback must compute anything.
- **Own time.** It is the wall clock, because `Engine::current_epoch` is not
  public (D-OF-1). `Continue(n)` sets the new deadline to the **current**
  epoch plus `n`, so the callback returns the budget left, not the wait.
  Own time can pass the budget by less than one tick (rounding up). That is
  accepted.
- **Wiring.** `build_store_and_instantiate` replaces
  `store.epoch_deadline_trap(); store.set_epoch_deadline(ticks)`
  ([engine.rs:569-570](../../../../crates/sandbox_wasm/src/engine.rs#L569))
  with:

  ```rust
  store.epoch_deadline_callback(|ctx| Ok(ctx.data().epoch.on_deadline(Instant::now())));
  arm_epoch_deadline(&mut store, epoch_deadline_ticks);
  ```

  `call_resource_method` replaces `store.set_epoch_deadline(ticks)`
  ([stream.rs:188](../../../../crates/sandbox_wasm/src/stream.rs#L188)) with
  `arm_epoch_deadline(store, ticks)`. After that no production code calls
  `set_epoch_deadline` itself. A unit test verifies this (P6 fix: see below).
- `HostState` gets `pub(crate) epoch: EpochBudget`, set to
  `EpochBudget::new(CallAllLimits::default().total_timeout)` in `HostState::new`.
- **Credit on every exit.** `call_all` calls `credit` once, after the batch
  ends, with the time from the first started call to the end, whatever the
  end was (all done, some timed out, total limit). A refusal before any
  wait gives no credit. **No other host function calls `credit`.** To make
  this a type rule, not a text rule: `credit` takes a `pub(super) struct
  WaitCredit(Duration)` that only `capabilities_call_all.rs` can construct.
  One unit test verifies that both production construction sites call
  `with_call_all` (P2 fix: see D-A3-8 below).
- **Dedicated OS thread for the epoch ticker (P17, P22).** `epoch_ticker::spawn`
  runs on its own OS thread (`std::thread::Builder::new().name("wasm-epoch-ticker".into()).spawn(...)`
  with a `std::thread::sleep(Duration::from_millis(EPOCH_TICK_MS))` loop).
  After each tick, it calls `shutdown_rx.try_recv()` and stops on any result
  other than `Err(TryRecvError::Empty)`. A dropped sender (when `AppSandboxEngine`
  is dropped) stops the thread, preventing thread or engine leaks in tests and
  production (P22 fix). The ticker is immune to tokio worker starvation when up to
  `max_concurrency` local WASM callees execute CPU-bound work concurrently
  in parallel tasks. Sleep is executed after each tick without catching up
  on missed ticks. A guest waiting for a worker thread now uses its budget
  during that wait, matching true wall-clock time (P22).

**D-A3-5. The run (pseudo-code).** In
`crates/sandbox_wasm/src/host_capabilities/capabilities_call_all.rs` (new, a
sibling of `capabilities_proxy.rs`, declared in `host_capabilities.rs`):

```rust
const MIN_BATCH_ALLOWANCE: Duration = Duration::from_secs(1);
const CALL_CANCEL_GRACE: Duration = Duration::from_millis(250);

async fn call_all(&mut self, calls: Vec<CallRequest>)
    -> Result<Vec<Result<String, proxy::ProxyError>>, proxy::ProxyError>
{
    if self.read_only { return refuse(ReadOnly) }          // same text as call
    if calls.is_empty() { return Ok(vec![]) }
    let limits = self.call_all.limits;                     // Copy
    if calls.len() > limits.max_calls { return refuse(TooMany) }
    let service_proxy = upgrade_or_refuse()?;              // same text as call
    let allowance = self.epoch.allowance();
    if allowance < MIN_BATCH_ALLOWANCE { return refuse(AllowanceUsed) }
    let _permit = self.call_all.waiters.try_acquire() else refuse(Busy);

    let total = limits.total_timeout.min(allowance);
    // 1. Build every request first, with `&self` only. A failure is that
    //    call's result. It never takes a concurrency slot.
    let mut slots: Vec<Option<Result<String, ProxyError>>> = vec![None; calls.len()];
    let mut runnable = Vec::new();
    for (i, call) in calls.into_iter().enumerate() {
        let timeout = limits.call_timeout(call.options.timeout_ms);
        match self.proxy_request(call, Some(timeout)) {
            Ok(req)  => runnable.push((i, req, timeout)),
            Err(e)   => slots[i] = Some(Err(e)),
        }
    }
    // 2. Run them, at most `max_concurrency` at once, each in its own task.
    //    `JoinSet` aborts leftover tasks when dropped (total time ends).
    let started = Instant::now();
    let mut set = tokio::task::JoinSet::new();
    let mut task_to_slot = std::collections::HashMap::new();
    let mut pending = runnable.into_iter().peekable();
    // Helper to spawn one call into the set and track its task id.
    let mut spawn_next = |set: &mut tokio::task::JoinSet<(usize, Result<Value, ProxyError>)>,
                          task_to_slot: &mut std::collections::HashMap<tokio::task::Id, usize>,
                          proxy: &Arc<dyn ServiceProxy>,
                          (i, req, timeout): (usize, ProxyRequest, Duration)| {
        let p = Arc::clone(proxy);
        let handle = set.spawn(async move {
            let r = tokio::time::timeout(timeout + CALL_CANCEL_GRACE, p.invoke(req)).await;
            (i, flatten(r))    // Elapsed -> Err(TimedOut); Ok(Err(e)) -> map_proxy_error
        }.instrument(Span::current()));
        task_to_slot.insert(handle.id(), i);
    };
    // Seed with the first wave.
    while set.len() < limits.max_concurrency {
        let Some(item) = pending.next() else { break };
        spawn_next(&mut set, &mut task_to_slot, &service_proxy, item);
    }
    // 3. Fill the slots as calls finish, until the total time ends.
    let _ = tokio::time::timeout(total, async {
        while let Some(res) = set.join_next_with_id().await {
            match res {
                Ok((_id, (i, r))) => {
                    task_to_slot.remove(&_id);
                    slots[i] = Some(r);
                }
                Err(join_err) => {
                    let id = join_err.id();
                    if let Some(idx) = task_to_slot.remove(&id) {
                        tracing::warn!(task_id = ?id, error = ?join_err, "call-all host task failed");
                        slots[idx] = Some(Err(ProxyError::Internal("call-all: call failed in the host".to_string())));
                    }
                }
            }
            // Start the next queued call.
            if let Some(item) = pending.next() {
                spawn_next(&mut set, &mut task_to_slot, &service_proxy, item);
            }
        }
    }).await;                  // on Elapsed, set is dropped: tasks are aborted
    // 4. Credit, then answer. A slot still empty is a call that did not finish.
    self.epoch.credit(WaitCredit(started.elapsed()));
    Ok(slots.into_iter().map(|s| s.unwrap_or(Err(ProxyError::TimedOut)).map(reply_text)).collect())
}
```

- Each call runs in its own `tokio::task::JoinSet` task. This gives real
  concurrency to local WASM targets: Wasmtime runs each callee on whichever
  worker thread picks up that task, so CPU work runs in parallel, not one
  after another (P15 fix). Dropping the `JoinSet` aborts all tasks; dropping
  a plain `JoinHandle` does not.
- The slot vector keeps the input order. The result of each task is placed at
  its original index.
- Each task gets the current tracing span (`.instrument(Span::current())`),
  so logs keep the guest's context.
- `MIN_BATCH_ALLOWANCE` is a constant set to 1 second (`const
  MIN_BATCH_ALLOWANCE: Duration = Duration::from_secs(1);`, P19 fix). An
  allowance below this refuses with `AllowanceUsed` instead of starting calls
  that cancel almost immediately (P8 fix). The refusal text and metric label
  are the same as the zero-allowance case.
- The per-call timeout bounds the whole call, retries included. A call that
  the batch's timers cut is not retried further and is not dead-lettered; its
  claim stays in flight until its window ends. The constant `CALL_CANCEL_GRACE`
  (`Duration::from_millis(250)`) is added to the per-call outer timer (the task)
  in both spawn sites (`timeout + CALL_CANCEL_GRACE`), so that a single-attempt
  call whose one attempt times out on its own still reaches `record_failed_call`
  inside `invoke` before the task is collected (P3 fix).
- Host panics and cancellations are tracked via `set.join_next_with_id()` and
  `task_to_slot`. If a task panics in the host (`Err(join_err)`), the slot for
  that call receives `Err(internal("call-all: call failed in the host"))`,
  a warning is logged, and the loop starts the next queued call and continues.
  It does not abort the batch or report running/queued calls as `timed-out`
  (P18 fix).
- The per-call timeout wraps `invoke` because the remote path applies the
  timeout for each attempt, and the trait promises no deadline (section 3.3).
- `reply_text` is the same function `call` uses (step 9).
- The function stays under 100 lines. If it does not, extract the seed loop
  and the drain loop to helpers. Match arms stay under 10 lines.
- `tokio` is already a dependency of `syneroym-sandbox-wasm`. `tracing` is
  too. No new crate is added.

**D-A3-6. Time of one call.** `limits.call_timeout(requested)` returns
`requested.unwrap_or(default).clamp(1 ms, max_call_timeout)`. The call's
`ProxyRequest.timeout` is that value. The outer task also wraps `invoke` in
the same time plus a 250 ms grace (D-A3-5). In a batch, the per-call time
bounds the whole call including retries. A guest cannot ask for more than the
cap. A `call`'s timeout still covers only one attempt on the remote path,
as today. This difference is stated in the WIT doc comment (D-A3-1, P7 fix).

**D-A3-7. Order and duplicates.** The results are in input order. The same
target twice is two calls, two `invoke`s and two entries. The host does not
merge or remove duplicates. Two calls with the same `idempotency-key` run
at the same time: the fence lets one run and answers the other "already
running here" ([router.rs:71-112](../../../../crates/router/src/proxy/router.rs#L71) and [call_dedup.rs:479](../../../../crates/router/src/call_dedup.rs#L479)),
which is a `callee` error entry. That is the fence working, not a batch
bug.

**D-A3-8. Waiters (Q5 (a)).** `HostState` gets `pub(crate) call_all:
CallAll`, where

```rust
pub struct CallAll { pub limits: CallAllLimits, waiters: Arc<Semaphore> }
```

The waiter semaphore and the limits are **shared across both builds**.
`CallAll::from_config(&SubstrateConfig)` (a public function in
`syneroym-sandbox-wasm`) builds one `CallAll` value from the configured
limits and one shared `Arc<Semaphore>`. The substrate runtime builds it once
in `crates/substrate/src/runtime/router.rs` (`let call_all = CallAll::from_config(&config);`),
passes it to `AppSandboxEngine::init_with_call_all(&config, call_all.clone())`
in `build_sandbox_engines`, and passes it to `init_roym` for every `NativeHostFactory`
through `with_call_all` (P2, P20 fix). `AppSandboxEngine::init(config)` keeps its
existing signature unchanged for tests, delegating to `init_with_call_all(config, CallAll::from_config(config))`.
Every `HostState` on the node therefore uses the same limits and the same permit
pool, so an operator's config change affects both builds equally and the
waiter count is one number for the whole node (P2 fix; OF-5). A native waiter
holds no instance pool slot, so a shared pool slightly over-constrains
native batches — that is accepted, because both builds obey one number.

The default `CallAll` in `HostState::new` uses a static shared semaphore
(a `once_cell` or `OnceLock`) with the default number of permits. This means
a construction site that forgets `with_call_all` is not silently unlimited
(P2 fix, second-check addition). `EpochBudget::new` is called with the same
default cap.

The permit is taken with `try_acquire` after the cheap checks, and dropped
when `call_all` returns, also when the future is dropped. A refusal is
`internal("call-all busy: too many guests are waiting in call-all; try again")`,
counted (D-A3-13) and logged with `warn!`.

**The real arithmetic for Q5.** The permit counts one slot per waiting batch.
A plain `call` takes no permit. The pool slots held by one search in the
Roym flow are: 1 for the waiter, plus one per local WASM callee in flight.
A batch of 8 local WASM targets holds 1 + 8 = 9 of 10 slots while the first
wave runs. Two such batches exceed the pool. The permit guarantees a bound
on waiters, and (when all targets are remote) a bound on pool slots. It does
not prevent pool exhaustion from local targets. This is the known case of Q9
(a), and a test shows it (section 7.4, P4 fix).

**D-A3-9. The wait allowance (Q4 (a)).** The total time of a batch is
`min(total_timeout, epoch.allowance())`. If the allowance is below
`MIN_BATCH_ALLOWANCE` (`const MIN_BATCH_ALLOWANCE: Duration = Duration::from_secs(1);`,
P19), the batch is refused: `internal("call-all busy: this guest
call has no wait time left in call-all")`. A zero allowance and a tiny
allowance both use the same text and metric label. So in one guest call,
the waits in all batches add up to at most `total_timeout`, and every one of
those waits is credited in full. A batch never starts calls that will cancel
almost immediately.

**D-A3-10. Nested calls.** A target that itself runs a `call-all` is a
different guest in a different store with its own deadline, its own credit
and its own concurrency limit. The outer batch applies its per-call time to
the whole callee call, including the callee's own batch. When the outer time
ends, the outer future is dropped, and the callee's guest call, which is
inside it, is dropped too (section 3.3: the local path awaits
`execute_wasm_json` inside `time::timeout`). Depth is bounded by the waiter
limit: each level holds a permit, so at the fifth level the batch is refused
at once (D-A3-8). A refusal at depth does not hang. A test shows it
(section 7.1).

**D-A3-11. A target that is the calling instance or its service.** The
callee runs in a new instance (section 3.3). The caller's concurrency limit
counts only the caller's outgoing calls. No lock is held on the caller's
instance. So there is no deadlock with the concurrency limit, and none with
the waiter limit, which is a `try_acquire` and never waits. The only limit
the callee can hit is the instance pool (Q9). It fails at once.

**D-A3-12. The guest is stopped or ends while calls run.**

- A guest cannot be stopped while a batch runs. Wasmtime checks the deadline
  only in guest code. The batch is bounded by the total time.
- If the future that runs the guest call is dropped (the caller's own
  timeout, a closed connection), the batch `JoinSet` is dropped with it.
  The tasks it holds are aborted. An aborted local WASM callee keeps running
  on its worker thread until its next `await` point or its own deadline, but
  the batch answers on time. A batch of local CPU-bound targets can hold up to
  `max_concurrency` worker threads for up to 5 s per callee; running the epoch
  ticker on a dedicated OS thread ensures that deadlines continue advancing
  regardless of worker thread saturation (P17). A remote call stops waiting
  after its abort, but the request that was already sent is not recalled, so
  the target may still do its work. The waiter permit is released when the batch
  future is dropped.
- If the service is stopped while a batch runs, the batch is not cancelled
  and ends by its total time at the latest (Q7 (a)).
- The native build has no epoch deadline. A guest that waits in a batch
  there is never stopped by the deadline. The credit is still counted and the
  allowance is still checked, because the same `HostState` code runs on both
  builds. This is the one permitted difference (no deadline natively). A test
  states it (section 7.2). It also means a slow batch passes natively and
  fails in a WASM build that did not credit it, which is why the credit tests
  run on the WASM build.

**D-A3-13. Metrics and logs (Q8 (a)).** In `capabilities_call_all.rs`:
`substrate.proxy.call_all.batches` (counter), `.calls` (counter, number of
calls run), `.refused` (counter with label `reason`: `too_many_calls`,
`busy`, `allowance_used`, `read_only`, `proxy_unavailable`), `.timed_out`
(counter, calls that ended `timed-out`), `.duration_ms` (histogram) and
`.waiting` (gauge, permits in use, incremented when a permit is taken and
decremented when it is released). That is five counters, one histogram and
one gauge. A refusal for `busy` also logs `warn!` with the service id.

**D-A3-14. The native shim.** `AppProxy` gets:

```rust
fn call_all(&self, calls: Vec<CallRequest>)
    -> impl Future<Output = Result<Vec<Result<String, ProxyError>>, ProxyError>> + Send;
```

- `app_host::types::proxy` re-exports `CallRequest`.
- `GuestHost::call_all` calls the generated `px::call_all(&calls)`.
- `NativeAppHost::call_all` converts each `CallRequest` with a new
  `call_request_in` (beside `call_target_in` and `call_options_in`), calls
  `HostProxy::call_all(&mut *state, requests)`, and converts the answer with
  `proxy_error_out`. It holds the state lock for the whole batch, as `call`
  does for one call.
- `NativeHostFactory` receives the shared `CallAll` value from the substrate
  runtime through `with_call_all` (one of the seven call sites added in
  Step 5). It does not build its own semaphore or limits (P2 fix; see D-A3-8).
- The test host in `roym_core/src/signing.rs` gets a `call_all` that returns
  an error. Nothing in it uses it.
- The native build counts credit and enforces the allowance (it runs the same
  `HostState::call_all` code), but has no epoch callback so the credit never
  extends a deadline. This is the one permitted difference stated in D-A3-12.

**D-A3-15. The deadline classification (Q6 (a)).** `classify_call_failure`
checks `e.downcast_ref::<Trap>()` for `Trap::Interrupt` and returns
`CallFailure::Deadline` before the text checks. The text check stays for
older Wasmtime text.

## 6. Step-by-step changes

The commits are in this order. Each commit builds and keeps all existing
tests green.

### Step 1 — Commit this plan

- Commit only this file and the `status.md` row:
  `docs(planning): plan M08A slice A3`.
- Wait for the owner's answers to section 2. Update sections 5 to 9 with
  them before Step 2.
- The reviewer of this plan must be a different tool from the author
  (session-strategy section 1).

### Step 2 — The epoch budget (no `call-all` yet)

**Files:**

- `crates/sandbox_wasm/src/engine/epoch_budget.rs` (new) and
  `epoch_budget/tests.rs` (new): D-A3-4. `engine.rs` declares `mod
  epoch_budget;` and `pub(crate) use` of `EpochBudget` and
  `arm_epoch_deadline`.
- `crates/sandbox_wasm/src/host_capabilities.rs`: add `pub(crate) epoch:
  EpochBudget` to `HostState` and `EpochBudget::new(CallAllLimits::default().total_timeout)` in `new`.
  `with_call_all` rebuilds `self.epoch` from the new limits so the cap stays
  in sync. Add `pub(crate) call_all: CallAll` with its default for now
  (Step 3 fills it). Put the `CallAll` struct and `CallAllLimits` with
  their `Default` in Step 2 (a new file
  `host_capabilities/call_all_limits.rs`), and the config in Step 3.
- `crates/sandbox_wasm/src/engine.rs`: replace lines 569-570 as in D-A3-4.
  Fix `classify_call_failure` (D-A3-15).
- `crates/sandbox_wasm/src/engine/epoch_ticker.rs`: rewrite ticker spawn to
  run on its own OS thread (`std::thread::Builder::new().name("wasm-epoch-ticker".into()).spawn(...)`)
  with a `sleep(EPOCH_TICK_MS)` loop. After each tick, it calls `shutdown_rx.try_recv()`
  and stops on any result other than `Err(TryRecvError::Empty)`. Dropping the sender
  stops the thread, preventing thread or engine leaks in tests and production (P22 fix).
  Update doc comment: the epoch advances on wall-clock time independently of worker thread
  load; guests waiting for worker threads use budget during the wait. Unit test in
  `epoch_ticker/tests.rs`: dropping the sender ends the ticker thread within a timeout (P22 fix).
- `crates/sandbox_wasm/src/stream.rs`: replace line 188 as in D-A3-4.
- `crates/sandbox_wasm/src/engine/tests.rs`: the test of
  `classify_call_failure` (lines 824-852) moves to a new
  `engine/tests/classify.rs`, with two new cases (a real `Trap::Interrupt`
  from a spinning guest, and a real `Trap::OutOfFuel`). `tests.rs` gets one
  `mod classify;` line and loses about 29 lines, so its entry in
  `xtask/oversized-test-files.txt` goes down to the real count. The two
  `AppSandboxEngine { .. }` struct literals in `tests.rs` get the new
  `call_all` field; record the final line count after both changes and lower
  the oversized entry to match (P11). Do not add any other test to `tests.rs`.

**Unit tests** (`epoch_budget/tests.rs`, pure, no clock, no guest):

- `a_budget_that_was_never_armed_interrupts` (P16).
- `zero_credit_interrupts_at_the_deadline_even_when_own_time_is_below_budget`
  (own time 450 ms of 500 ms, credit zero: `Interrupt`). This is the test of
  section 3.4's last fact.
- `credit_extends_by_the_budget_left_rounded_up` (the cases of the spike's
  `callback_arithmetic_calculates_deadline_credit`, with credit: 1,200 ms
  elapsed, 1,000 ms credit gives `Continue(3)`; 1,500 ms gives `Interrupt`).
- `credit_is_capped` (credit past the cap is cut) and
  `allowance_is_the_cap_minus_the_credit`.
- `arm_resets_credit_and_start`.
- Note on credit callers: `credit` takes a private token `WaitCredit(Duration)`
  constructible only in `capabilities_call_all.rs`. Enforced at compile time
  by Rust type privacy; no source grep test is needed (P16).
- `no_production_code_sets_an_epoch_deadline_itself`: reads only non-test
  files under `src/` (exclude files under any `tests/` directory and any
  `#[cfg(test)]` module path). Checks that `set_epoch_deadline` appears only
  in `epoch_budget.rs`, and that `arm_epoch_deadline(` appears in both
  `engine.rs` and `stream.rs`. Require the exact expected count for each
  pattern so a forgotten call is a failure (P6 fix). Use unique file names
  under a `tempfile` directory if it writes any (A2a R11).
- `a_host_state_that_was_never_armed_allows_a_full_batch`: build a `HostState`
  with `HostState::new` (no `arm_epoch_deadline`); check that `allowance()`
  equals `CallAllLimits::default().total_timeout` (P1 fix).
- `with_call_all_sets_the_credit_cap`: call `with_call_all` with small limits;
  check that `allowance()` equals those limits' `total_timeout` (P1 fix).

**Engine test** (new file `crates/sandbox_wasm/tests/epoch_budget_engine.rs`
is **not** added: the spike file already pins the Wasmtime behaviour, and
Step 7 runs the production path with a real guest).

**Gate:** `cargo nextest run -p syneroym-sandbox-wasm`, the spike
(`--test epoch_wait_credit`), and `cargo nextest run -p syneroym-substrate
--test guest_http_e2e` pass. Commit:
`feat(sandbox-wasm): epoch deadline callback with wait credit`.

### Step 3 — Limits and config

- `crates/core/src/config/sandbox.rs`: six fields and their `default_*`
  functions (D-A3-3), with doc comments in the style of the neighbours.
- `crates/sandbox_wasm/src/engine/init.rs`: `SandboxTuning` gets the six
  values. `resolve_sandbox_tuning` copies them (and the `else` branch uses
  `CallAllLimits::default()`), and clamps with warnings (D-A3-3).
  Add `pub fn from_config(config: &SubstrateConfig) -> CallAll` that builds one
  `Arc<Semaphore>` with the configured permit count and the resolved limits
  (P2, P19). Add `AppSandboxEngine::init_with_call_all(&config, call_all: CallAll)`,
  storing `call_all` on the engine beside `abac_instance_permits`. Keep `init(&config)`
  unchanged (it calls `init_with_call_all(config, CallAll::from_config(config))`),
  so its 73 test and benchmark callers need no edits and `proxy_dispatch.rs` line count
  is untouched (P20 fix).
  `build_store_and_instantiate` calls
  `.with_call_all(self.call_all.clone())`
  next to `with_conversation`
  ([engine.rs:558](../../../../crates/sandbox_wasm/src/engine.rs#L558)).
- `crates/substrate/config.sample.toml`: the six fields, with short comments,
  after `dispatch_epoch_timeout_secs`.
- **Tests** (in `engine/tests/`, a new file `call_all_config.rs`, and the
  existing config tests of `core` if there are any): defaults match D-A3-3;
  a zero is clamped to 1; the three clamp rules; a config file that names
  none of the fields loads. The two `AppSandboxEngine { .. }` struct literals
  in `tests.rs` get the new `call_all` field; record the final line count
  after all Step 2 and Step 3 edits and lower the oversized entry to match
  (P11 fix). Do not add other lines to `engine/tests.rs` beyond the field
  additions and one `mod` line.

Commit: `feat(sandbox-wasm): configure call-all limits`.

### Step 4 — The WIT and the host function

- `crates/wit_interfaces/wit/proxy/proxy.wit`: `call-request` and `call-all`
  (D-A3-1). Do **not** touch
  `test-components/saga-test/wit/deps/proxy/proxy.wit`.
- `capabilities_proxy.rs`: split `call` into `resolve_target`,
  `proxy_request` and `reply_text` (D-A3-2). Keep every comment about why a
  step is there (the dependency note, the caller rule, the string rule).
  `call` keeps the order of its checks exactly. Read the diff of `call`
  against `main` line by line before you commit (A2b R5).
- `capabilities_call_all.rs` (new): `call_all` (D-A3-5, D-A3-6, D-A3-8,
  D-A3-9, D-A3-13), the small `CallAllRefusal` enum with one function
  that gives its text and its metric label. A refusal for too many waiters or
  exhausted allowance starts with `call-all busy:`; read-only instance and
  missing proxy use the same `internal` texts as `call` (P7 fix).
  `const MIN_BATCH_ALLOWANCE: Duration = Duration::from_secs(1);` (P19 fix)
  and `const CALL_CANCEL_GRACE: Duration = Duration::from_millis(250);` (P3 fix).
  The `pub(super) struct WaitCredit(Duration)` token that only this file can
  construct enforces credit caller rules at compile time (P16 fix).
- `host_capabilities.rs`: `mod capabilities_call_all;`.
- No `futures-util` addition: the run loop uses `tokio::task::JoinSet` from
  `tokio` (already a dependency).
- **Unit tests:** new `host_capabilities/tests/tests_call_all.rs` (section
  7.1). `tests_proxy.rs` gets no lines (761 now; the cap is 800).

Gate: `cargo nextest run -p syneroym-sandbox-wasm` and clippy on the crate.
Commit: `feat(sandbox-wasm): add proxy call-all host function`.

### Step 5 — The shim

- `crates/app_host/src/lib.rs`: `AppProxy::call_all` (D-A3-14).
- `crates/app_host/src/types.rs`: re-export `CallRequest`.
- `crates/app_host/src/guest.rs`: `GuestHost::call_all`.
- `crates/app_host_native/src/convert/proxy.rs`: `call_request_in`.
- `crates/app_host_native/src/host.rs`: `NativeAppHost::call_all`.
- `crates/app_host_native/src/factory.rs`: `with_call_all`. The factory
  receives the shared `CallAll` from the caller; it does not build its own.
- `crates/roym_core/src/signing.rs`: the test host's `call_all`.
- `crates/substrate/src/runtime/router.rs` (`build_sandbox_engines` and `init_roym`),
  `crates/substrate/src/runtime/roym.rs` (six sites) and
  `crates/substrate/src/runtime/dual_build_fixture.rs` (one site): the
  substrate runtime builds one `let call_all = CallAll::from_config(&config);`
  and passes it to `AppSandboxEngine::init_with_call_all(&config, call_all.clone())` in
  `build_sandbox_engines` and to every `NativeHostFactory` via `init_roym` through `with_call_all`.
  These sites ensure the single shared limits and semaphore reach both WASM and native builds
  from one construction without touching the 73 callers of `init` (P2, P20 fix; D-A3-8).
- Build every WASM guest: `mise run build:test-components` and
  `mise run build:roym`. All Roym services must compile as components. This
  is the proof that the one source tree still builds both ways.

Commit: `feat(app-host): add call-all to the dual-build shim`.

### Step 6 — Fixture and parity test

- `test-components/dual-build-fixture/src/app.rs` and `app/dispatch.rs`: one
  verb, `ProxyCallAll { calls }`. Each call has `kind` (`service` or
  `dependency`), `name`, `interface`, `method`, `params` and an optional
  `timeout_ms`. The answer is `{"outcomes": [{"ok": text} | {"err": text}]}`
  (each `err` is `fmt_err` of the error) or `{"refused": text}`. A second
  verb `ProxyCallAllThenSpin { calls }` runs the batch, then loops in guest
  code until it is stopped. A third, `ProxyCallThenWork { call }`, makes one
  plain `proxy.call` and then runs a short loop (the control, section 7.3).
  A fourth verb `ProxyCallAllSeq { batches: Vec<Vec<Call>> }` runs multiple
  batches in order within a single guest call, returning an array of outcomes
  per batch (P14 fix; needed by `two_batches_in_one_wasm_guest_call_accumulate_credit`).
  The dispatch table already has `#[expect(clippy::too_many_lines)]`; it
  does not change the suppression count.
- `crates/app_host_native/tests/dual_build_parity/stub_proxy.rs` (new): moves
  `StubProxy` out of `helpers.rs` and gives it a second interface,
  `scripted`, with these methods (params are JSON): `echo` (returns its
  params), `sleep {ms}` (waits, then returns `"slept"`), `fail` (a `callee`
  error, code -32000). It counts calls in flight and keeps the largest count
  (`max_in_flight`), and sets a flag when a call is dropped before it ends
  (`dropped`). `helpers.rs` re-exports it (`pub(crate) use`), so every
  caller of `StubProxy` keeps its path. `helpers.rs` shrinks by about 14
  lines, so lower its entry in `xtask/oversized-test-files.txt`.
- Non-default config plumbing (P2 fix): `call_all_non_default_config_is_identical_on_both_builds`
  needs non-default limits to reach both WASM and native parity stacks.
  `build_wasm_stack` and `build_native_stack` in `helpers.rs` take an optional
  config override (`config: Option<SubstrateConfig>`), paid for by the lines
  freed by the `StubProxy` move, keeping `helpers.rs` below its recorded line
  cap.
- `crates/app_host_native/tests/dual_build_parity/call_all.rs` (new) and the
  `#[path]` line in `dual_build_parity.rs`: the parity scenarios (section
  7.2).

Commit: `test(app-host-native): call-all parity`.

### Step 7 — Deadline and waiter tests on the real engine

- `crates/app_host_native/tests/dual_build_parity/call_all_deadline.rs` (new)
  with its `#[path]` line: section 7.3. WASM only, except the native half of
  the permitted difference.

Commit: `test(app-host-native): call-all epoch credit and waiters`.

### Step 8 — The router test (the same checks)

- `crates/router/tests/proxy_harness/mod.rs` (new): the harness functions of
  `proxy_dispatch.rs` that a second test needs
  (`test_substrate_config`, `wasm_deploy_manifest`,
  `test_route_handler_with_proxy_components`, `call_peer_pipeline`,
  `call_peer_preamble`, `json_rpc_body`; lines 82 to 360). `tests/<name>/mod.rs`
  is the one place `mod.rs` is allowed. Move them, do not copy them.
  `proxy_dispatch.rs` gets `mod proxy_harness;` and shrinks by the lines
  moved, so lower its entry in `xtask/oversized-test-files.txt`. A helper
  that one binary does not use triggers `dead_code`: use the pattern of
  `method_contract` (D-A2a-17), an `#[allow(dead_code)]` on the module,
  and nothing broader.
- `test-components/proxy-test/wit/world.wit` and `src/lib.rs`: a driver
  function `call-peers` that takes a list of `(kind, service, interface,
  method, params)` and returns each result as text (the `{e:?}` of an error,
  as `call-peer` does). Also an export `spin-for: func(ms: u32)` that loops in
  guest WASM code for a given number of milliseconds, used as a CPU-bound local
  callee target (P15 fix, P17 fix). The component is built by `build:test-components`.
- `crates/router/tests/proxy_call_all.rs` (new): section 7.4. Runs tests on the
  real router with local WASM components, including parallel local execution on
  a multi-thread runtime with >= 4 worker threads (P15), bounded pool exhaustion
  with `max_concurrent_instances = 6` (P4), and epoch ticker starvation immunity
  on a 2-worker runtime (P17).

Commit: `test(router): call-all through the real proxy router`.

### Step 9 — Docs and close-out

Sections 8 and 9. One commit:
`docs: describe call-all and the epoch credit`.

### Step 10 — Finish

- The import pass over every edited Rust file.
- `cargo +nightly-2026-04-06 fmt --all`.
- `cargo xtask check-duplication`. The new `call_all` and the split `call`
  share one request builder, so no copy is expected. If the percent goes
  down, lower `MAX_EXACT_DUPLICATE_PERCENT` as the gate message says. Also
  `cargo xtask check-file-lengths` (three oversized entries go down) and
  `cargo xtask check-lint-suppressions` (no new `too_many_lines`).
- Rebuild the WASM artifacts, then `mise run verify` as a background job, no
  `--skip`, every gate green. Check `target/gates/e2e.log` for node errors
  (A2a R1).
- One PR with the CLA line and the attribution line. The PR text gives the
  test results and the audit of section 4 in short form.

## 7. Tests

### 7.1 Unit tests of the host function (`tests_call_all.rs`)

They use a `ScriptedProxy` (a `ServiceProxy` with the scripted methods of
Step 6, in this file, because the fixture's stub lives in another crate) and
`HostState::with_call_all` with small limits and a small time. Each test has
one reason to fail. Names:

- `an_empty_batch_returns_an_empty_list_and_runs_nothing` (no `invoke`, no
  permit, no credit).
- `a_batch_over_the_maximum_is_refused_before_any_call` (OF-17: `invoke`
  count is 0; the error is `unsupported-target` with the maximum in the
  text).
- `results_come_back_in_input_order_whatever_the_finish_order` (three calls
  that finish in reverse order).
- `a_failed_call_does_not_change_the_other_entries` (ok, callee error, bad
  params text, bad protocol tag, unbound dependency).
- `at_most_the_concurrency_limit_run_at_once_and_the_rest_wait` (16 calls,
  limit 8: `max_in_flight` is 8; all 16 are answered).
- `a_call_gets_its_own_time_and_the_host_caps_a_larger_request` (a call that
  asks for 60 s is cut to the cap; a call with no value gets the default).
- `a_slow_call_times_out_and_the_others_keep_their_results`.
- `the_total_time_ends_the_batch_and_cancels_running_calls` (the scripted
  proxy sees its future dropped; unfinished and not-started entries are
  `timed-out`; finished entries keep their result).
- `a_finished_call_is_not_reported_as_timed_out_behind_a_slow_first_call`
  (the reason for `buffer_unordered`).
- `the_same_target_twice_is_two_calls` (D-A3-7).
- `each_call_is_built_like_a_single_call` (table-driven: for each of a
  `service` target to another service, a `service` target to the calling
  service, a `dependency` target, a routing key and an idempotency key, the
  `ProxyRequest` that `call` sends and the one a batch of one sends are
  equal, except `timeout`. This is the behavioural half of D-A3-2.)
- `a_read_only_instance_cannot_run_a_batch` (same text as `call`:
  `"stage-4 after-step instances may not originate proxy calls"`; P7 fix).
- `a_missing_proxy_is_refused_with_the_text_of_call` (`"proxy unavailable"`; P7 fix).
- `refusal_texts_follow_wit_contract` (P7 fix: waiter limit and allowance exhaustion
  refusals start with `"call-all busy:"`; read-only instance and missing proxy use
  their existing `internal` texts identical to `call`).
- `host_call_panic_is_recorded_as_internal_error_and_does_not_abort_batch` (P18 fix:
  scripted proxy panics on one call; that call's slot gets `Err(Internal("call-all: call failed in the host"))`,
  the remaining calls finish and keep their results, no calls are spuriously marked `timed-out`).
- `the_waiter_limit_refuses_at_once_and_a_permit_is_released_after_the_batch`
  (also after the batch future is dropped).
- `a_nested_batch_at_the_waiter_limit_is_refused_and_does_not_hang` (the
  scripted proxy answers one call by running another batch on a second
  `HostState` with the same waiter semaphore).
- `the_wait_allowance_shrinks_the_total_time_and_runs_out` (two batches in
  one guest call; the second gets the rest; the third is refused).
- `a_refused_batch_gives_no_credit_and_a_timed_out_batch_does_give_it`
  (D-A3-4: credit on error and timeout).
- Note on credit callers: `WaitCredit` is a private token constructible only in
  `capabilities_call_all.rs`, guaranteeing at compile time that no other host function
  calls `credit` (no runtime test needed; P16 fix).
- `a_callee_that_cannot_get_a_pool_slot_is_one_failed_entry` (Q9: scripted
  error `callee` -32603 for one entry; the others answer).
- `every_exit_path_preserves_input_order_and_cardinality` (P14: all-ok,
  all-fail, total time ended, mixed with bad params; returns exactly one
  entry per input call in order).
- `allowance_below_min_batch_allowance_refuses` (P8: allowance <
  `MIN_BATCH_ALLOWANCE` refuses with `AllowanceUsed`).
- `never_armed_host_state_allows_batch` (P1, P14: `HostState::new` without
  `arm_epoch_deadline` allows a batch up to default cap).

### 7.2 Parity test (`dual_build_parity/call_all.rs`)

Each scenario runs once on the WASM build and once on the native build, with
the same fixture verb, and compares the JSON answer. Names:

- `call_all_empty_list_is_the_same_in_both_builds`.
- `call_all_results_in_input_order_are_the_same` (calls that finish in the
  opposite order).
- `call_all_good_failed_and_timed_out_calls_give_the_same_outcomes` (a good
  call, a `callee` error, a call of 100 ms with a script that sleeps
  1,000 ms, an unbound dependency, a bad params text).
- `call_all_over_the_maximum_is_refused_the_same` (17 calls, same error text).
- `call_all_runs_at_most_eight_at_once_in_both_builds` (16 calls of 100 ms:
  `max_in_flight` is 8 on each build; the stub's counters are read and reset
  between the two runs, which run one after the other).
- `call_all_the_same_target_twice_gives_two_entries`.
- `call_all_non_default_config_is_identical_on_both_builds` (P2, P14: non-default
  limits `call_all_max_calls = 4` configured on both builds; 5th call refused
  identically).
- `a_mutant_batch_answer_is_detected` (as `Mutant` in `helpers.rs`: change one
  character of the WASM answer and prove the comparison fails, so the test
  compares something).

A guest built against the older WIT must still link (section 4).
This is not a parity test. `saga_e2e` already runs the `saga-test` component,
whose WIT copy A3 leaves untouched, showing that the older type set still links
against the extended host interface. If the normal gates do not run `saga_e2e`,
add one check that instantiates `saga-test`.

The fixture verbs of Step 6 are the way A3 proves `call-all` works with no
Roym service. The same verbs are what A4's tests can reuse.

### 7.3 Deadline and waiters (`dual_build_parity/call_all_deadline.rs`)

WASM build, default config (budget 5 s, total limit 15 s). The scripted
target sleeps 6 s, which is longer than the budget and shorter than the
total limit. Time is measured with `Instant`, with the margins of the spike.

- `a_plain_call_that_waits_past_the_budget_stops_the_guest` (the control:
  `ProxyCallThenWork` with a 6 s call. The call returns after 6 s. The guest
  is then stopped. The error is `Trap::Interrupt` ("wasm trap: interrupt").
  This is the problem A3 solves.)
- `a_batch_that_waits_past_the_budget_does_not_stop_the_guest` (a batch with
  the same 6 s call: `Ok`, and the elapsed time is at least 6 s).
- `credit_buys_back_the_wait_and_no_more` (`ProxyCallAllThenSpin` with a
  6 s call: the guest is stopped by `Trap::Interrupt` after between
  `6 s + 5 s - 1 tick` and `6 s + 5 s + 1 s`, and `classify_call_failure`
  calls it a `Deadline`).
- `a_guest_that_spins_is_stopped_on_time_when_it_never_used_call_all` (the
  zero-credit path of section 3.4, on the production callback).
- `two_batches_in_one_wasm_guest_call_accumulate_credit` (P14: real guest call
  uses `ProxyCallAllSeq` to run two consecutive batches; second gets remaining
  allowance, credited time accumulates, third is refused).
- `a_stream_chunk_gets_a_fresh_credit` is **not** added. The chunk path is
  the same `arm_epoch_deadline`. The unit test `arm_resets_credit_and_start`
  and the grep test cover it. State this in the PR.
- `slow_batches_do_not_fill_the_instance_pool`: start 4 batches (the
  waiter limit) that each wait 3 s. While they wait, a plain `run` (echo)
  answers `Ok` within 1 s. A fifth batch is refused with `call-all busy`.
  After the first four end, a sixth batch runs. This is the test the backlog
  row asks for.
- `the_native_build_has_no_deadline_for_a_slow_batch` (permitted difference,
  also listed in `permitted_differences.rs`' style: the same 6 s batch
  returns `Ok` natively. The plain call `ProxyCallThenWork` also returns
  `Ok` natively, which is the reason the credit tests run on the WASM build
  only).

Expected run time: the three long tests take about 6 s, 6 s and 11 s. They
run in separate processes under nextest. Flakiness check: run the file 20
times in a row (as the S0 spike did) and record the result in the PR. If
timing noise fails a run, widen the margin. Never change the 6 s to 5 s
ratio that separates the credited and uncredited cases.

### 7.4 The same checks, through the real router (`proxy_call_all.rs`)

A real `RouteHandler` and `ProxyRouter`, with `proxy-test` as the caller and
`greeter` as a local WASM target, as `proxy_dispatch.rs` does. One batch of
`call-peers` with these entries, and the exact answers:

- `service` target `proxy-callee` (greeter) `greet`: `Ok`, with the
  greeting.
- `service` target that does not exist: `service-not-found`.
- another service's native `data-layer`: `permission-denied` (the gate of
  `check_native_capability_gate`, the same answer as
  `guest_cross_service_native_capability_through_proxy_is_permission_denied`).
- a `dependency` name that is not declared: `dependency-not-bound`.
- the caller's own service (`proxy-test` calls its own `call-peer`): `Ok`,
  a nested call in a new instance. This is the test of D-A3-11 on the real
  engine.
- one call twice: two `Ok` entries.
- a batch of 17: refused.
- a keyed call in a batch timing out within grace: records a dead letter via
  `record_failed_call` (P3, P14).
- `local_wasm_callees_execute_in_parallel_via_joinset` (P15 fix): runs on a
  multi-thread runtime with >= 4 worker threads; two local WASM calls to
  `spin-for(2000)` execute concurrently in parallel `JoinSet` tasks, finishing
  in ~2 s instead of ~4 s; total timeout aborts tasks on time.
- `local_wasm_callees_pool_exhaustion_is_bounded` (P4 fix): router harness
  configured with `max_concurrent_instances = 6`; local WASM targets exhaust
  the instance pool returning `callee` -32603 for excess calls, while an
  unrelated call succeeds once the batch completes and releases pool slots.
- `epoch_ticker_on_dedicated_thread_prevents_clock_starvation` (P17 fix): on a
  runtime configured with 2 workers, two local callees spin endlessly in WASM;
  both are stopped by `Trap::Interrupt` within budget plus margin, not by fuel
  exhaustion, proving that the dedicated ticker OS thread advances epochs even
  when all tokio worker threads are saturated.

The test also checks that the entries equal what a single `call-peer` gives
for each, so a difference between `call` and `call-all` fails here.

### 7.5 Not tested

- Remote targets (another node) in a batch. The remote path is the same
  `invoke`. The e2e tests of `proxy.call` over two nodes
  (`crates/substrate/tests/`) cover it. A4's search e2e will run a batch over
  real directories.
- A recall of a request that was already sent when the batch is cancelled.
  It does not exist (D-A3-12).
- Sizes of the lists above 16 in the WASM canonical ABI. A list of 16 small
  records is far below any ABI limit.

### 7.6 Size risks

- `capabilities_proxy.rs` goes from 458 to about 480 lines (the split adds
  signatures). `capabilities_call_all.rs` is about 220 lines.
- `engine.rs` is 665 lines and gets about 10 more. `init.rs` is 423 lines and
  gets about 40.
- `host_capabilities.rs` is 624 lines and gets about 20. It stays under 800.
- `tests_call_all.rs` is about 600 lines. If it passes 800, split it in two
  (`tests_call_all_limits.rs`, `tests_call_all_time.rs`).
- The parity files are new and small. `dual_build_parity.rs` gets two lines.

### 7.7 How A3 avoids the kinds of findings A2a, A2b and A2c had

| Earlier finding | The risk in A3 | What this plan does |
| --- | --- | --- |
| A2c R1 (an error test that reads only one build) | the parity tests compare one build | Every parity scenario runs both builds and compares the full answer. The mutant test proves the comparison compares something. |
| A2a R1 (a check that silently does nothing) | the deadline scan (`set_epoch_deadline`); a stale WASM | The scan reads the tree from `CARGO_MANIFEST_DIR` and counts matches (a count of zero fails). `WaitCredit` is a private token constructed only in `capabilities_call_all.rs`, enforced at compile time (P16). Section 0: rebuild WASM before every run; check the e2e log. |
| A2b R5 (a behaviour change hidden in a rewrite) | the split of `call` | D-A3-2: `call` keeps its check order. The reviewer reads the diff of `call` against `main`. The test `each_call_is_built_like_a_single_call` fixes the request. |
| A2b R8 (deleted why-comments) | `call` has long comments | Step 4: keep every why-comment beside code that stays. |
| A2a R2, R3; A2b R1, R12 (change doc decisions and deviations) | 15 new decisions | Section 9: D-A3-1 to D-A3-15 with the same numbers and meaning; real deviations in their own list. |
| A2b R2, A2c R3 (wrong counts) | test counts | Step 10 and section 10 say what to count. Record the counts in `status.md`. |
| A2a R10, A2b R7, A2c R6 (numbered comments) | tests | Named test functions. No numbered comments. |
| A2b R9 (set-up written twice) | the parity and engine tests need a scripted proxy | One `StubProxy` (moved, not copied). One router harness (moved, not copied). The unit tests use their own `ScriptedProxy` because they live in another crate. |
| A2a R6 (a rule checked on the outer layer only) | per-call limits | The per-call time is checked on each call in the unit tests and in the parity test. |
| A2b P3 (a typed read that drops data) | `reply_text` | The same function as `call`, so a string is raw and other JSON is text. |
| S0 Q3 (a classification that may be wrong) | `classify_call_failure` | Step 2 tests a real trap, not a made-up string. |

## 8. Deferred backlog

All rows are in [deferred-backlog.md](../../deferred-backlog.md).

- **Move to "Recently resolved":** "A real epoch-deadline trap is classified
  as `Other`" (line 488).
- **Narrow and keep open:** "Slow `call-all` waits can hold every guest
  instance slot" (line 539): "A node-wide waiter permit limit (first value 4)
  is built in A3 (D-A3-8), bounding waiters when targets are remote. Kept
  open for: per-service or per-caller fair share, and accounting for pool
  slots held by callers and local WASM callees (P4, P13)."
- **Keep open and append:** "A guest dispatch cannot loop over more than one
  network call" (line 535): "The mechanism is built (`call-all`, the epoch
  credit). The row closes in A4, when a Roym service uses it."
- **New rows, §11:**
  - "**Local WASM targets in a batch can exhaust the instance pool**": a batch
    of local WASM targets holds one pool slot for each running callee, and a
    guest that cannot get a slot fails at once, as one failed entry. Reason:
    Q9 (a). Target: TBD. Source: `capabilities_call_all.rs`.
  - "**`call-all` is not cancelled when its service is stopped**": a batch ends
    by its total time at the latest. Reason: Q7 (a). The same gap holds for
    every ordinary call. Target: TBD.
  - "**The waiter limit is one for the whole node**": a busy service can use
    all the permits. Reason: Q5 (a). Target: when a second service uses
    `call-all` (M08F).
  - "**A guest cannot read the `call-all` limits**": it learns the maximum
    only from the refusal text. Target: when A4's library needs to split a
    list.
  - "**No bound on total `call-all` calls across fast-failing batches in one guest call**":
    a guest looping batches of 16 that fail at once can send many calls within
    its 5 s own time. Reason: P10. Target: TBD.
  - "**No aggregate response size cap across a `call-all` batch**": responses
    are bounded per call (16 MiB frame), not per batch. 16 large responses could
    exceed guest memory limit. Reason: P10. Target: TBD.
- **Unchanged:** "A single `proxy.call` can wait past the guest's budget"
  (D-OF-1) and "Search shows no per-source progress" (D-OF-5).
- No new open `TODO` or `FIXME` marker is added.

## 9. Living docs to update at close-out

Ask for each: is a statement now false or incomplete? Do not cite change docs
or milestone and slice ids in these files. Link ADRs only.

- [docs/system-architecture.md](../../../system-architecture.md): the
  Universal Proxy paragraph: add `call-all` in two or three
  sentences (what it does, who sets the limits, that each call is checked
  like `call`). The limits table: reword "One guest call, wall-clock time | 5 seconds"
  to "One guest call, own time (time waiting in `call-all` is not counted,
  up to a cap) | 5 seconds", and add the batch rows (calls,
  concurrency, per-call time, total time, waiters) with their config names (P12).
  In the section "A guest runs under an epoch deadline": add that the wait in
  `call-all` is credited up to a cap.
- [docs/system-requirements-spec.md](../../../system-requirements-spec.md)
  "Epoch-Based Interruption": add the credit, and say that no other
  host call gives it.
- [docs/developer-guide.md](../../../developer-guide.md) "Calling Another
  Service from a WASM Component (Universal Proxy)": add `call-all`
  with a short example, the result shape, the limits, the refusals, and the
  rule that the waiting time of a batch is not counted against the guest's
  5 s. Say that a plain `call` still is. Also check that sentences referring
  to the 5s budget (e.g. for scheduled runs or ADR-0021 §6) refer to own
  execution time (P12). Add the six config fields where the
  guide lists sandbox settings, if it does.
- [docs/TERMINOLOGY.md](../../../TERMINOLOGY.md): "call-all" and "wait
  credit", if the file defines terms of this kind. Check first.
- [docs/planning/traceability-matrix.md](../../traceability-matrix.md): add a
  row for the batched call after `[ROY-CHK]` (line 93), with the tests of
  section 7 as evidence. Use the next free id.
- [docs/roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md):
  grep for `call-all`, `fan-out` and `epoch`. Update only if a statement is
  false.
- [crates/substrate/config.sample.toml](../../../../crates/substrate/config.sample.toml):
  done in Step 3.
- Change doc [roym-open-foundation](../../changes/roym-open-foundation/change.md):
  - **Tasks:** tick "`call-all` with its limits, the deadline extension (or
    the fallback), and the native shim."
  - **Acceptance criteria:** fill the Test column of OF-3, OF-4, OF-5 and
    OF-17 with the tests of section 7, as A2 did for OF-14, OF-15 and OF-18.
  - **Design, "Batched outbound call":** replace the WIT sketch with the real
    one (D-A3-1) and correct the sentence "Wait credit is capped at one
    `call-all` total limit per guest call" if D-A3-9 changes it.
  - **Decisions:** add D-A3-1 to D-A3-15 with the same numbers and meaning,
    and one line with the owner's answers to Q1 to Q9.
  - **Deviations:** under "Different from the plan", add: the WIT differs from
    the sketch (five-field `call-request` with `interface` and `options`, no
    `target-index`, no batch-level `per-call-timeout-ms`, an outer `result`);
    the waiter limit and the wait allowance are new; the callback interrupts at
    once when the credit is zero (the spike's arithmetic did not); `classify_call_failure`
    is fixed. Add anything the implementation really did differently.
  - **`living-docs-touched`:** add any file above not yet listed.
  - `status` stays `draft`.
- [task.md](task.md): add the A3 result line under "A3".
- [status.md](status.md): row A3. Later steps record the PR, the test counts
  (unit tests, parity scenarios, deadline tests, router tests) and the 20-run
  result.

No ADR is added. The decisions live in the change doc, as D-OF-1 to D-OF-6
do. Do not rewrite ADR-0014's text about stream instance budgets.

## 10. Exit criteria

A3 is done when all of these are true:

1. The owner has answered Q1 to Q9, and sections 5 to 9 match the answers.
2. `proxy.wit` has `call-request` and `call-all`, and no existing type
   changed. `saga_e2e` passes with the `saga-test` WIT copy untouched.
3. `call-all` runs the calls at the same time, at most the configured number
   at once per guest instance, returns one entry per call in input order, and
   gives each call its own time and the batch its total time (OF-3).
4. A guest that waits in a batch for longer than its 5 s is not stopped. A
   guest that waits as long in a plain call is stopped. A guest that loops
   after a batch is stopped when its budget ends, counted from the end of the
   credited wait. The deadline is armed in one function, and only
   `call_all` gives credit (OF-4).
5. A guest that never uses `call-all` is stopped at the same moment as before
   (the zero-credit test and the existing e2e tests pass).
6. The same scenarios give the same answers in both builds, and the same
   maximum in flight (OF-5).
7. A batch over the maximum is refused before any call runs. At most the
   concurrency limit run at once and the rest wait. The waiter limit keeps
   other calls running (OF-17, and the backlog row).
8. Each call in a batch is built by `proxy_request` and sent by `invoke`. The
   router test shows the same answers as single calls.
9. `classify_call_failure` classifies a real deadline as `Deadline`, tested
   with a real trap.
10. Metrics for refused and timed-out calls exist (if Q8 (a)).
11. No Roym service changed. No `req.params` or Roym file in the diff.
12. The oversized list has the three lowered entries, and the duplication
    baseline is correct.
13. The change doc, the architecture doc, the requirements spec, the
    developer guide, the traceability matrix, task.md, status.md, the sample
    config and the backlog are updated as in sections 8 and 9.
14. The deadline test file ran 20 times in a row with no failure.
15. `mise run verify` passes with no `--skip`.
16. One PR.

## 11. What A3 does not do

- No Roym service calls `call-all`. A4 does that, and builds the federated
  query library and the one search method.
- No change to `proxy.call`, `enqueue` or the saga functions. They give no
  credit, and `call`'s 30 s default stays (backlog "A single `proxy.call` can
  wait past the guest's budget").
- No new `proxy-error` variant and no WIT version change.
- No per-call progress or events during a batch (D-OF-5).
- No cancellation token for `stop_wasm` (Q7).
- No per-service waiter limit (Q5).
- No guest-readable limits function.
- No batch form of `enqueue` or `saga.step`.
- No aggregate response size cap or batch count cap per guest call in code
  (tracked in deferred backlog, P10).
- No change to the epoch tick, the default budget, the pool size or the
  stream instance budget.
- No change to the S0 spike test. It stays as a pin of the Wasmtime
  behaviour.
- No Hub or `roymctl` change. No new demo story (the `demo:` list stays).

## 12. What A4 must decide

The WIT shape of `call-all` is complete for A4, but A4 must resolve these design decisions (P5):

1. **`web` forwarding vs. search duration:** A search runs Hub -> `web` ->
   `directory` -> `call-all`. `web` forwards methods with a plain `call`
   (30 s timeout, zero credit). Because `web` is a guest HTTP request with a
   5 s budget, wait credit in `directory` does not buy extra wall-clock time
   for `web`: `web` will be stopped at 5 s if it uses a plain `call`. A4 must
   either forward `directory.search` from `web` via a one-entry `call-all`
   (which gets wait credit), or ensure the whole search stays below `web`'s
   own 5 s budget. A4 must test the slow-source case end to end.
2. **Timeout hierarchy and permits:** If `web` forwards via `call-all`, the
   inner batch must end, and `directory` must merge and answer, well within
   `web`'s per-call time. The guest controls this only through the per-call
   times of the inner calls and how many waves they need. For example, 8 sources
   at 2 s each in one wave end in about 2 s. The total is the same host setting
   for both batches, and the per-call cap cannot be raised above it (D-A3-3 clamp
   cuts `max_call_timeout` to `total_timeout`; if A4 needs an outer time larger
   than the total, it must request a change to the clamp). Also, each search
   will hold two waiter permits on the node (one for `web`, one for `directory`),
   reducing concurrent searches to 2 at the default limit of 4 (P21 fix).
3. **Headroom check:** The existing host-side check
   `DEFAULT_SOURCE_TIMEOUT_MS + DISPATCH_HEADROOM_MS < epoch`
   ([directory.rs:65-72](../../../../crates/roym_core/src/directory.rs#L65))
   must stay true or be explicitly replaced with the batched call timeout
   model, not dropped silently.
