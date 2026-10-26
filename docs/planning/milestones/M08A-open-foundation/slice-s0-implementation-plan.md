# M08A slice S0 — Spike and decisions: implementation plan

*Reader: the developer (or AI) who implements slice S0. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: ready, 2026-10-28. The owner accepted the recommendations for
Q1 to Q8 (section 2.1) on 2026-10-27. Re-checked on 2026-10-28 after
`main` upgraded Wasmtime to 48.0.7 and Iroh to 1.4: the design holds, and
only line numbers changed.**

**Size and PR.** M. One PR, three commits on `feat/m08a-s0-spike`: this
plan, the spike test, the close-out (decisions written into the docs).

**Base.** `feat/m08a-s0-spike`, rebased on `main` at `1a92250e` (after
the Wasmtime 48.0.7 and Iroh 1.4 upgrades and the demo stories), plus the
AGENTS.md end-user-section rule. Every file path and line number below was
checked against that base. Wasmtime API facts were checked in the cached
crate source `wasmtime-48.0.7` (the version in `Cargo.toml` and
`Cargo.lock`). Iroh is not in `syneroym-sandbox-wasm`'s dependency tree,
so the Iroh upgrade does not touch S0.

**Parallel work.** A1 (the participant rename) may be built at the same
time. S0 touches only `crates/sandbox_wasm/tests/` and planning docs. The
only file both may touch is the change doc's `## Deviations` section.
Rebase before the close-out commit if A1 merged first.

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most: `mise run verify` before
  you finish (background job, about 20 minutes, no `--skip`), the import
  clean-up pass, no planning references in code (no "M08A", "S0", "OF-4",
  "D-OF-1" in the test file, its comments or its test names; ADR ids are
  fine), simple English in comments and docs, 800-line files, 100-line
  functions, the deferred backlog and the living-docs close-out.
- [task.md](task.md) section "S0 — Spike and decisions", sections 1, 2, 4.
- [roym-open-foundation change doc](../../changes/roym-open-foundation/change.md):
  OF-3 to OF-11 and OF-17, "Batched outbound call", "Event feed", "Open
  questions".

### Terms used in this plan

- **Epoch, tick:** Wasmtime counts time in "epochs". The node's ticker adds
  one every `EPOCH_TICK_MS` = 100 ms
  ([engine.rs:317](../../../../crates/sandbox_wasm/src/engine.rs#L317),
  [epoch_ticker.rs:19](../../../../crates/sandbox_wasm/src/engine/epoch_ticker.rs#L19)).
  One added epoch is one tick.
- **Epoch deadline:** the epoch at which a running guest is stopped. Set
  once per store with `Store::set_epoch_deadline`
  ([engine.rs:570](../../../../crates/sandbox_wasm/src/engine.rs#L570)), and
  again before each stream chunk call
  ([stream.rs:188](../../../../crates/sandbox_wasm/src/stream.rs#L188)).
  The ordinary budget is `dispatch_epoch_timeout_secs`, 5 s by default
  ([init.rs:70](../../../../crates/sandbox_wasm/src/engine/init.rs#L70)).
- **Epoch-deadline callback:** a function the host gives to the store with
  `Store::epoch_deadline_callback`. Wasmtime calls it when guest code
  reaches a passed deadline. It returns "stop" or "continue for N more
  ticks".
- **Wait credit:** the time a guest spent waiting inside a host call that
  the host agrees not to count against the guest's budget. This plan's
  name, not Wasmtime's.
- **Own time:** wall-clock time since the guest call started, minus the
  wait credit. The time the guest spent running its own code, or waiting
  in host calls that give no credit.

## 1. What S0 builds, in one paragraph

S0 answers one technical question with a test, and makes four design
decisions that A3, A4 and A5 build on. The question: can a WASM guest wait
inside one host call for longer than its epoch deadline, and still not be
stopped, while a guest that loops in its own code is still stopped on
time? The test is a new integration test in `crates/sandbox_wasm/tests/`.
It uses the node's real engine configuration and the same host-binding
style as production, with a small test-only host interface. Production
code does not change in S0. The four decisions: what happens if the test
fails (the two-step fallback), where the event feed lives, the v1 event
kinds and catch-up window, and whether search keeps a live-progress form.

## How this is shown to an end user

- Nothing new. An end user sees no change from S0. It is a test and a set
  of written decisions.
- The visible result comes later. M08A's demo (task.md, "Demo") extends
  the Roym chat story
  `crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts`: Bob's reply and the
  `delivered` state on Alice's message appear with no refresh. A5 builds
  that. S0 adds no demo step. A4 lets the Hub run a search with one call.

How a developer can see it work:

- `cargo nextest run -p syneroym-sandbox-wasm --test epoch_wait_credit`
  prints five passing tests (section 4, Step 2).
- The decisions are in the change doc's `## Decisions` section, and
  [status.md](status.md) links the test and the PR.

## 2. Points for the owner

### 2.1 Questions (all recommendations accepted by the owner, 2026-10-27)

**Q1. Where does `call-all` go?** task.md (X3) and the change doc say
"`call-all` in `syneroym:invocation`", and the change doc says "Today a
guest calls one target at a time through `syneroym:invocation`". That is
not the code. `syneroym:invocation` has one function, `caller`
([invocation.wit:32](../../../../crates/wit_interfaces/wit/invocation/invocation.wit#L32)).
Outbound calls are `syneroym:proxy`'s `call`
([proxy.wit:77](../../../../crates/wit_interfaces/wit/proxy/proxy.wit#L77)),
with `call-target`, `call-options` and `proxy-error` already defined there.
The change doc's sketch also uses `params: list<u8>`, but `call` takes JSON
text (`params: string`).
*Recommendation:* put `call-all` in `syneroym:proxy`, beside `call`, and
reuse its target, options and error types. Params stay JSON text, as in
`call`. A3's plan sets the exact signature. S0 corrects task.md and the
change doc.

**Q2. What form does the spike code take?** Options: (a) one integration
test, `crates/sandbox_wasm/tests/epoch_wait_credit.rs`, with its own small
store state and a test-only host interface; production code unchanged.
(b) Add the wait-credit state to `HostState` and the callback to
`build_store_and_instantiate` now, used by nothing yet.
*Recommendation:* (a). (b) adds unused production code (a `dead_code`
warning, or a suppression) and changes every guest call's store before
anything needs it. A3 moves the logic into production, and A3's own tests
cover the real path. The S0 test stays after A3: it pins the Wasmtime
behaviour the design depends on, so a Wasmtime upgrade that changes it
fails here first. Note that `engine/tests.rs` cannot take new tests: it is
on the oversized list at its cap of 852 lines
(`xtask/oversized-test-files.txt:25`).

**Q3. The deadline trap may be classified wrong today (found while
reading).** A real epoch-deadline stop is `Trap::Interrupt`, whose text is
`"interrupt"` (`wasmtime-environ-48.0.7/src/trap_encoding.rs:141`).
`classify_call_failure` checks for `Trap::OutOfFuel` by type, but finds a
deadline only by the words `"epoch"` or `"deadline"` in the error text
([engine.rs:367-381](../../../../crates/sandbox_wasm/src/engine.rs#L367)).
Its test uses a made-up error string, not a real trap
([engine/tests.rs:847](../../../../crates/sandbox_wasm/src/engine/tests.rs#L847)).
If the real text has neither word, a guest HTTP request that runs out of
time is reported as `Trap`, not `BudgetExceeded`
([guest_http.rs:137](../../../../crates/sandbox_wasm/src/engine/guest_http.rs#L137)),
and the same for ABAC ([auth.rs:616](../../../../crates/sandbox_wasm/src/engine/auth.rs#L616)).
`unverified`: the exact text of a real trap's root cause. Step 2 checks it.
*Recommendation:* S0 checks it and records the result. If it is wrong, S0
adds a backlog row and A3 fixes it (a `Trap::Interrupt` type check, beside
the `OutOfFuel` one), because A3 changes this code path anyway. Not fixed
in S0: S0 changes no production code (Q2).

**Q4. Does search keep a live-progress form?** Today the Hub shows each
directory's answer as it arrives. The change doc recommends "one call,
plus `search.progress` events". That does not fit the primary design: the
guest is inside one `call-all` host call until every target has answered
or timed out, so the guest cannot write a progress event in the middle.
Only the host could, and then the host writes events on the guest's
behalf. Options: (a) no progress in v1: one call, one merged page with one
outcome per source (`ok`, `timed-out`, `refused`, `unreadable`); (b) keep
the three-call form beside the one call; (c) the host writes a progress
event per answered target.
*Recommendation:* (a). The wait is bounded by `call-all`'s total limit
(a few seconds), and the page names a slow source. (b) keeps a loop in
every client, which is what OF-7 removes. (c) is real extra design for a
spinner. A backlog row records (c) as a later option.

**Q5. Where does the event feed live?** *Recommendation:* the host-level
event bus, as the change doc already recommends. Section 3, D-OF-3 gives
the comparison with the facts from the code. One fact in the change doc's
table is wrong and S0 corrects it: it says `web`'s "Instances are reused
and keep no state today". In fact no instance is reused. Each ordinary
call and each WebSocket event gets a new store and instance
(`build_store_and_instantiate`,
[engine.rs:469](../../../../crates/sandbox_wasm/src/engine.rs#L469), used by
`handle_websocket_on_open` at
[guest_stream.rs:294](../../../../crates/sandbox_wasm/src/engine/guest_stream.rs#L294)).
Only a raw stream keeps one instance for its life
([guest_stream.rs:83](../../../../crates/sandbox_wasm/src/engine/guest_stream.rs#L83)).
The table also cites `app.rs:293`; the comment is at
[app.rs:295](../../../../crates/roym_web/src/app.rs#L295).

**Q6. The catch-up window.** *Recommendation:* in memory, one window per
app instance, the newest 10,000 events or 24 hours, whichever is smaller.
A cursor names the node's run (a random id made at start-up) and a
sequence number. After a node restart, or for a cursor older than the
window, the answer is "reload". Nothing is written to disk. Reason: an
event only says "something changed", and a client that reloads fetches
the current state with the normal methods anyway. A durable window would
add a disk write per event for no new information. One window per app
instance means a busy app cannot push another app's events out.

**Q7. The v1 event kinds.** *Recommendation:* four kinds, all written by
Roym services. The host does not read the kind; it is a string the writing
app owns.

| Kind | Written by | Raised when |
| --- | --- | --- |
| `conversation.message` | `conversation` | A message is stored, sent or received, in a 1:1 conversation or a group. |
| `conversation.delivery` | `conversation` | The delivery state of an outgoing message changes. |
| `transaction.card` | `transaction` | A card (request, quote, agreement, receipt, payment, fulfilment) is created or changes state. |
| `transaction.booking` | `transaction` | A booking changes state. |

These cover OF-8 ("a message, a card, a booking or a delivery state").
The ids each kind carries are set in A5 with A2's typed records. `match`
comes in M08F. With the fallback (section 5), A3 adds `search.done`.
M08C may rename `transaction.booking` when bookings become commitments.

**Q8. A cap on the wait credit.** Without a cap, a guest could call
`call-all` again and again in one guest call, each time inside its total
limit, and hold an instance slot without end.
*Recommendation:* the host gives at most one `call-all` total limit of
credit per guest call. Wait time past the cap counts as own time, so the
guest is stopped by its normal budget. A3 sets the value with the total
limit.

### 2.2 Stale or unclear points in task.md and the change doc

- task.md calls S0 "docs only". S0 also commits one test file. No
  decision needed; the close-out fixes the word.
- The two backlog rows the S0 brief names are both in section 11 of
  [deferred-backlog.md](../../deferred-backlog.md) (lines 525 and
  526), not in sections 10 and 11. A3 and A4 close them, not S0.
- The change doc says the callback "can extend the deadline". Precisely:
  `UpdateDeadline::Continue(n)` sets the new deadline to the **current**
  epoch plus `n` (`wasmtime-48.0.7/src/runtime/store.rs:2231-2240`, called
  from `vm/libcalls.rs:738`). It does not add `n` to the old deadline. The
  callback runs when the guest next checks the epoch after the wait, and
  by then the old deadline has passed. So the callback must return "the
  budget minus own time", not "the wait". Returning the wait would give the
  guest almost one extra wait of time (Step 2, test T4 catches it).
- A plain `proxy.call` defaults to a 30 s timeout
  ([proxy.wit:55](../../../../crates/wit_interfaces/wit/proxy/proxy.wit#L55)),
  longer than the 5 s budget. So today a single slow `call` can also stop a
  guest after it returns. This design gives no credit to `call`, on
  purpose: credit for every call would remove the bound on one guest
  call's wall time. No change; S0 states it in the decision.

## 3. Decisions this plan makes

D-OF-1 depends on the test result. Where each one is written down
(Step 4 does it):

| Decision | Change doc | task.md | status.md | Backlog |
| --- | --- | --- | --- | --- |
| D-OF-1 extension (or D-OF-2 fallback) | `## Decisions`; "Batched outbound call" text; `## Deviations` if the fallback | S0 result line; X3; section 4 order if the fallback | S0 row evidence | §11 row note; §9 row if Q3 shows the gap |
| D-OF-3 bus | `## Decisions`; "Event feed" text and table | S0 result line | — | — |
| D-OF-4 kinds and window | `## Decisions`; "Event feed"; "Open questions" entry removed | S0 result line | — | §8 row (window lost on restart) |
| D-OF-5 no progress | `## Decisions`; "Open questions" entry removed | S0 result line | — | §11 row |
| D-OF-6 `syneroym:proxy` | `## Decisions`; `## Deviations` (doc fact corrected) | X3 | — | — |

**D-OF-1. The epoch-deadline extension (primary approach, if Step 2
passes).**

- The store's data holds the wait-credit state: when the guest call
  started, its budget, and the credit so far.
- It is reset wherever `set_epoch_deadline` is called today:
  [engine.rs:570](../../../../crates/sandbox_wasm/src/engine.rs#L570) for
  each store, [stream.rs:188](../../../../crates/sandbox_wasm/src/stream.rs#L188)
  for each stream chunk. The budget is that call's own budget (dispatch,
  lifecycle hook or ABAC).
- `call-all` adds the time it waited to the credit, also when it ends with
  an error or a timeout, up to the cap (Q8). No other host call adds
  credit.
- The store uses `epoch_deadline_callback` in place of
  `epoch_deadline_trap`. The callback computes own time. If own time is at
  or past the budget, it returns `UpdateDeadline::Interrupt`. This gives
  the same `Trap::Interrupt` as `epoch_deadline_trap` gives today, so a
  guest with no credit behaves exactly as before. Otherwise it returns
  `Continue(ticks)`, the budget left, rounded up to whole ticks, at least
  1.
- Own time uses the wall clock (`Instant`), because the current epoch is
  not public in Wasmtime (`Engine::current_epoch` is `pub(crate)`,
  `wasmtime-48.0.7/src/engine.rs:827`). The ticker never runs ahead of the
  wall clock (missed ticks are not made up), so the callback is never
  called before the epoch budget has passed.
- No `Config` change. `async_support` is a no-op in Wasmtime 48
  (`config.rs:426-431`). Wasmtime's `Yield` variants are not used.
- `UpdateDeadline` is `#[non_exhaustive]` in Wasmtime 48
  (`store.rs:387`). The callback only builds values of it and never
  matches on it, so this has no effect.

**D-OF-2. The fallback, if Step 2 fails.** See section 5. If Step 2
passes, the change doc keeps the fallback text as the rejected
alternative, with the reason.

**D-OF-3. The event feed is a host-level event bus.** Compared on the
change doc's three points, with the code facts:

| | Host-level bus | `web` service |
| --- | --- | --- |
| Write cost per event | One in-process host call: add to a bounded list, wake the subscribers. No new guest instance, no JSON-RPC hop. | One `proxy.call` to `web` inside the writer's own guest call. Each call creates a new `web` store and instance ([engine.rs:469](../../../../crates/sandbox_wasm/src/engine.rs#L469)). Then `web` reads its subscriber list from storage and sends to each connection. The time counts against the writer's budget, and a write fails while `web` is down. |
| Where connection state lives | In the host, for the life of the node, next to the login session check. | The sockets are already held by the host (`WebSocketSenders`, [guest_stream.rs:56-80](../../../../crates/sandbox_wasm/src/engine/guest_stream.rs#L56)). But which session and cursor belong to which connection must be stored by `web` in its data layer: a write on every open and close, a read on every event. |
| Guest instance reuse | Not needed. | None exists. Every call and every WebSocket event runs in a new instance (Q5), so `web` cannot keep a list in memory. In the native build it could, which would make the two builds behave differently. |

The host tags each event with the writer's app instance from its own
records (`app_context_of`,
[engine.rs:535](../../../../crates/sandbox_wasm/src/engine.rs#L535)),
never from the event body. A5 decides the subscription path for services
(OF-10) and what a service outside any app instance may do (recommended:
it cannot write events in v1).

**D-OF-4. Event kinds and catch-up window:** as Q6 and Q7.

**D-OF-5. Search has no live-progress form in v1:** as Q4.

**D-OF-6. `call-all` is in `syneroym:proxy`:** as Q1.

## 4. Step-by-step changes

### Step 1 — Commit this plan

- Commit only this file:
  `docs(planning): plan M08A S0 spike and decisions`.
- Then wait for the owner's answers to section 2.1. Update section 3 with
  them before Step 2.

### Step 2 — The spike test

**File:** `crates/sandbox_wasm/tests/epoch_wait_credit.rs` (new). It is
its own test binary, so it follows the "`tests/<name>.rs`" rule and needs
no `mod` line anywhere. Expected size: about 250 lines. No new
dependency: `wasmtime` (with `component-model` from the workspace and
`wat` in dev-dependencies) and `tokio` are already there.

**What it builds:**

1. **The engine:** `AppSandboxEngine::build_wasm_engine` (public,
   [init.rs:283](../../../../crates/sandbox_wasm/src/engine/init.rs#L283)),
   with the pooling allocator on, as production uses it. So the test runs
   with the node's real `Config`: component model, fuel, epoch
   interruption.
2. **The clock:** a tokio task that calls `engine.increment_epoch()` every
   100 ms with `MissedTickBehavior::Delay`, the same as the node's ticker
   (`epoch_ticker::spawn` is private to the engine module, so the test has
   its own copy of these few lines). The test runtime is
   `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]`: a
   spinning guest blocks its worker thread, and the clock must run on
   another one. Production uses a multi-thread runtime too
   ([main.rs:177](../../../../crates/substrate/src/main.rs#L177)).
3. **A test-only host interface**, in the production binding style:
   `wasmtime::component::bindgen!` with inline WIT and
   `imports: { default: async }`, the trait implemented on the test's
   store state, and `add_to_linker::<_, HasSelf<State>>(&mut linker, |s| s)`
   (the same form as
   [init.rs:382](../../../../crates/sandbox_wasm/src/engine/init.rs#L382)).
   Two functions:
   - `wait(ms: u32)`: sleeps; gives no credit (like every host call
     today).
   - `wait-credited(ms: u32)`: sleeps; adds the time it waited to the
     state's credit (as `call-all` will).
4. **The store state:** `started: Instant`, `budget: Duration`,
   `credited: Duration`. Nothing else: the test component imports only
   the test interface, so the linker needs no WASI.
5. **The callback:** a function `on_deadline(state) -> UpdateDeadline`,
   exactly as in D-OF-1, installed with `store.epoch_deadline_callback`.
   It is small and pure, so the test also has a unit test of its
   arithmetic (budget left, rounding up, at least 1 tick, stop at zero).
6. **The guest:** a WAT component (compiled with `Component::new`, as
   [conversions/tests.rs:203](../../../../crates/sandbox_wasm/src/conversions/tests.rs#L203)
   does). It imports the test interface and exports four functions:
   - `wait-then-work(ms)`: calls `wait`, then runs a short loop (about
     1,000 turns).
   - `credited-wait-then-work(ms)`: the same with `wait-credited`.
   - `spin()`: loops forever.
   - `credited-wait-then-spin(ms)`: `wait-credited`, then loops forever.

   The short loop matters: Wasmtime checks the epoch only in guest code
   (at function entry and loop back-edges), so a guest that returned with
   no code after the wait would never be checked. A Roym guest always
   merges results after the wait, so the loop is the honest case.
7. **The call:** `Func::call_async`, the same entry point production
   uses ([execute.rs:129](../../../../crates/sandbox_wasm/src/engine/execute.rs#L129)).
   The store gets a large fuel budget (`set_fuel`), so that the epoch, not
   fuel, stops a spinning guest.

**Numbers:** budget B = 5 ticks (500 ms). Wait W = 2,000 ms. Margin
M = 750 ms. These keep the whole file under about 8 s, and they separate
a correct extension (stop at W + B = 2.5 s) from the "returned the wait"
mistake (stop at about 2W = 4 s).

**The tests and the exact pass/fail criterion.** The spike **passes** when
all five pass, with no change to `build_wasm_engine`, no `Yield`, and no
host binding style other than the one above:

| # | Test | Store setup | Guest export | Pass when |
| --- | --- | --- | --- | --- |
| T1 | `a_long_host_wait_stops_the_guest_today` | `epoch_deadline_trap` (today) | `wait-then-work(W)` | Err, and the error is `Trap::Interrupt`. The control: shows the problem is real. |
| T2 | `a_credited_wait_does_not_stop_the_guest` | callback | `credited-wait-then-work(W)` | Ok. |
| T3 | `a_guest_loop_is_still_stopped_at_its_budget` | callback | `spin()` | `Trap::Interrupt`, after between B − 1 tick and B + M. |
| T4 | `credit_buys_back_the_wait_and_no_more` | callback | `credited-wait-then-spin(W)` | `Trap::Interrupt`, after between W + B − 1 tick and W + B + M. |
| T5 | `a_wait_without_credit_still_counts` | callback | `wait-then-work(W)` | `Trap::Interrupt`. Shows that the callback changes nothing for other host calls. |

Plus the unit test of `on_deadline` (no clock, no guest).

The spike **fails** if T2 cannot pass (the callback is not called, or
cannot move the deadline, under `call_async` with an async import), or if
T3 or T4 cannot pass with any correct arithmetic (the extension cannot be
bounded). Then section 5 applies. If it passes only with a change outside
the criterion above (a `Config` change, `Yield`, a different binding
style), stop and ask the owner before choosing.

**Flakiness check:** run the file 20 times in a row and record the result
in the PR text:

```bash
for i in $(seq 20); do cargo nextest run -p syneroym-sandbox-wasm --test epoch_wait_credit > target/spike-$i.log 2>&1 || echo "run $i failed"; done
```

Zero failures is required. If timing noise fails a run, widen M, never
the B or W ratio that tells T4's two cases apart.

**Q3 check (not a test that stays):** in T3, print once
`e.root_cause().to_string()` and whether it contains `"epoch"` or
`"deadline"`. Record the text in the decision. Remove the print before
committing.

**Comments in the file:** say what each test proves and why the short
loop and the multi-thread runtime are needed. No "spike", "S0", "M08A" or
"OF-4" in the file. The module doc says the file pins the Wasmtime
epoch-callback behaviour that the batched outbound call relies on.

**Commit:** `test(sandbox-wasm): pin epoch deadline credit`.

### Step 3 — If Step 2 fails

Do not write more spike code. Write section 5's changes into the docs in
Step 4, and record the failure (which test, the error) in the change
doc's `## Deviations`. Delete the test file, or keep only T1 if the owner
wants the control kept. Ask the owner which.

### Step 4 — The close-out (docs)

**Change doc** ([change.md](../../changes/roym-open-foundation/change.md)):

- Add a `## Decisions` section, before `## Deviations`, with D-OF-1 to
  D-OF-6, each with its reason and (D-OF-1) the test file and result.
- "Batched outbound call": `syneroym:invocation` becomes `syneroym:proxy`
  in the prose and the sketch; `params` becomes `string`. "Primary
  approach (to be proved in the M08A spike)" becomes the proved approach;
  remove its `unverified` line; add the "budget minus own time" point and
  the credit cap. If the spike failed: the fallback becomes the design and
  the primary approach becomes the rejected one.
- "Event feed": replace "Where the feed lives is decided in the M08A
  spike" with the decision; fix the table's "Instances are reused" cell
  and the `app.rs:293` line; add the window and the kinds.
- "Open questions": remove "Live progress for search" and "Event window
  size" (now decisions). "Typed-method check" stays for A2.
- `## Deviations`: one entry for each correction of the doc's own facts
  (the interface name, `params` type, the instance-reuse cell), and, if
  the spike failed, the fallback choice.
- `## Tasks`: tick "M08A spike". `status` stays `draft` (the change is not
  done).
- OF-4 and OF-6: unchanged if the spike passes. With the fallback, OF-4
  is replaced and OF-6 uses its three-step text, as the change doc says.

**task.md:** X3 says `syneroym:proxy`. Section 2's first bullet adds the
result in one line. S0's bullets get a "Result:" line each. "docs only"
becomes "docs and one test". With the fallback, section 4's order gets
"A5 before A4".

**status.md:** S0 row: Done, implemented and reviewed by (tool names),
evidence: the test file, the 20-run result, the PR link.

**M08-overview.md:** no change if the spike passes. With the fallback, the
"epoch-deadline spike" bullet says the fallback was chosen.

**Deferred backlog** ([deferred-backlog.md](../../deferred-backlog.md)):

- Section 11, the two rows at lines 525-526: keep them open (A3 and A4
  close them). Add to the first row's target: "mechanism proved by
  `crates/sandbox_wasm/tests/epoch_wait_credit.rs`".
- New row, section 11: "Search shows no per-source progress" (Q4), target
  TBD, with option (c) as the later path.
- New row, section 8 (node lifecycle): "The event-feed catch-up window is
  lost on restart" (Q6), target TBD.
- New row, section 9 (observability), only if Q3's check shows the gap:
  "A real epoch-deadline trap is classified as `Other`", target M08A A3.
- New row, section 11: "A single `proxy.call` can wait past the guest's
  budget" (section 2.2), target TBD, reason: credit for every call would
  remove the bound on one guest call.

**Living docs:** none change. S0 changes no behaviour a reader of the
requirements, architecture or developer guide can see. The architecture
doc changes in A3 and A5, when the code does.

**Commit:** `docs(planning): record M08A S0 decisions`.

### Step 5 — Finish

- Import pass over the test file.
- `cargo +nightly-2026-04-06 fmt --all`.
- `mise run verify` as a background job, no `--skip`, every gate green.
  `cargo xtask check-change-docs` checks the change doc's front matter.
- Open one PR from `feat/m08a-s0-spike` with the CLA line and the
  attribution line. The PR text gives the test result (20 runs), the
  decisions in one line each, and the Q3 finding.

## 5. The fallback, if the spike fails

From the change doc's fallback design: the guest starts a job that
returns at once; the host runs the fan-out natively, outside any guest
call, under the same limits; it stores the results and raises an event; a
second short guest call reads the results by job id. What changes:

- **A3.** No long `call-all`. In its place, two `syneroym:proxy`
  functions: one starts a batched call and returns a job id at once, one
  reads a job's results. A small in-memory job table in the host, bounded
  in size and age, scoped to the app instance. The host raises a "done"
  event on the event bus (the kind is set by A3's plan; for search it is
  `search.done`). The native shim does the same. No wait credit, no
  callback; OF-4 is replaced.
- **A4.** A search is three steps for every client: `search.start`, the
  `search.done` event, `search.results`. `web` cannot chain them inside
  one call, because it would wait inside its own guest call. The Hub
  needs the event feed for search. `roymctl` can instead call
  `search.results` until it stops answering "pending", so it needs no
  WebSocket client.
- **A5.** It must merge before A4, and it adds the `search.done` kind.
  The order in task.md section 4 changes to A5 before A4.
- **M08F.** A forwarding exchange cannot wait for its friends inside one
  wire call. `exchange.search` returns the exchange's own results at once,
  plus a job id for the friends' results. The participant's node gets them
  with a second wire-reachable method (`exchange.search-results`) or the
  exchange sends them to the participant's `inbox` service. One more
  method, a job table on the exchange node, and a second round trip. M08F's
  plan chooses between the two.

## 6. Exit criteria

S0 is done when all of these are true:

1. The owner has answered Q1 to Q8, and section 3 matches the answers.
2. Either the five tests of Step 2 pass, and pass 20 runs in a row, or the
   failure is recorded and the fallback is chosen (Step 3).
3. The change doc has a `## Decisions` section with D-OF-1 to D-OF-6, its
   design text matches them, the two answered open questions are gone,
   its `## Deviations` lists every corrected doc fact, and its first task
   box is ticked.
4. task.md, status.md and the deferred backlog are updated as in Step 4.
   (M08-overview.md only with the fallback.)
5. No production code changed. The test file follows AGENTS.md: clippy
   clean, no planning ids in it, under 800 lines, functions under 100
   lines.
6. `mise run verify` passes with no `--skip`.
7. One PR with the three commits, after the code review rounds and a
   final quick review.

## 7. What S0 does not do

- No production code change: no `call-all`, no `HostState` field, no
  callback in `build_store_and_instantiate`, no event bus. A3 and A5 build
  them.
- No fix to `classify_call_failure` (Q3).
- No WIT change.
- No change to the backlog rows A3 and A4 will close, beyond the note in
  Step 4.
