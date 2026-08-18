# Elastic WASM Memory — Design and Implementation Plan

**Scope.** Replace the Wasmtime **pooling** instance allocator in
[`AppSandboxEngine::build_wasm_engine`](../../crates/sandbox_wasm/src/engine.rs#L675)
with **on-demand allocation plus a shared, engine-wide memory budget**
enforced by a `ResourceLimiterAsync`. Memory is charged when a guest actually
grows, and returned when the `Store` drops.

**Milestone placement.** Interstitial, not an M06B slice. It touches
`syneroym-sandbox-wasm` and `syneroym-core::config` only, and is independent
of M06B B2-B5 in both directions. Best executed at a quiescent point: B1 is
complete (2026-08-18), B2 has not started.

**Status.** Plan only. Nothing implemented. Written against the tree at
2026-08-18 (`feat/m06b-slice-b1`, `c1787b4`), wasmtime `46.0.2`.

**Read §1 first if you are reviewing.** Three of the nine findings contradict
what the code's own comments and the backlog currently say, and one of them
(F3) is a live latent bug that this change happens to fix.

---

## §1 Findings from reading the tree

### F1 — the pooling reservation is ~4 TiB per engine, not the ~1.28 GiB the backlog assumes

[deferred-backlog.md:38](deferred-backlog.md) states the theory as "Wasmtime's
pooling allocator reserves `instances * max_memory_size` address space per
`Engine`", giving ~1.28 GiB. That formula is wrong on both factors. From
wasmtime 46.0.2's `MemoryPool::new` → `SlabConstraints::new` → `calculate`:

```
slot_bytes       = max(tunables.memory_reservation, max_memory_size) + memory_guard_size
total_slab_bytes = total_memories * slot_bytes        (plus a pre-slab guard)
```

- `total_memories` is **not** `max_concurrent_instances`. It is set at
  [engine.rs:717](../../crates/sandbox_wasm/src/engine.rs#L717) to
  `max_concurrent_instances * max_memories_per_component` = **10 × 100 = 1000**.
- `slot_bytes` is **not** `max_memory_size`. `memory_reservation` is never
  configured, so it is wasmtime's 64-bit default of **4 GiB**, which dominates
  the configured 1 GiB. `memory_guard_size` defaults to 32 MiB.

So a default-configured substrate performs one `Mmap::accessible_reserved` of
`1000 × (4 GiB + 32 MiB)` ≈ **4.03 TiB** of `PROT_NONE` address space, per
`Engine`, at startup. Every guest-fixture test file builds its own engine, and
`cargo test` runs test binaries as separate processes — but several substrates
inside one binary each add another 4 TiB reservation to the same address space.
This is the concrete mechanism behind the row-38 flakiness, and it is ~3000×
larger than that row's estimate.

It is address space, not resident memory, so it usually succeeds. It fails as
one all-or-nothing `mmap` under a `ulimit -v`, a low `vm.max_map_count`, strict
overcommit, or a constrained CI container — and when it fails the whole engine
fails to build.

### F2 — `max_memory_size` is currently inert

Because `slot_bytes` takes `max(memory_reservation, max_memory_size)` and
`memory_reservation` is 4 GiB, setting `max_memory_size` to 1 GiB (or 128 MiB,
or 16 MiB as `proxy_dispatch.rs` does) changes nothing about the reservation.
The comment at [engine.rs:695-706](../../crates/sandbox_wasm/src/engine.rs#L695)
says the reservation "stays governed by Wasmtime's own defaults (1000 slots)".
That is half right: the slot *count* is ours (1000, computed at :717), the slot
*size* is wasmtime's (4 GiB). `max_memory_size`'s only live effect today is the
`bail!` if it ever exceeds `memory_reservation`.

### F3 — `StoreLimitsBuilder::instances(1).memories(1).tables(1)` is dead configuration

[host_capabilities.rs:178-183](../../crates/sandbox_wasm/src/host_capabilities.rs#L178)
builds a `StoreLimits` with `.instances(1).memories(1).tables(1)`. Wasmtime
reads those three counts **from the limiter object passed to
`Store::limiter`** — and that object is `HostState`, not `StoreLimits`
([engine.rs:1300](../../crates/sandbox_wasm/src/engine.rs#L1300),
`store.limiter(|state| state)`). `HostState`'s `ResourceLimiter` impl
([host_capabilities.rs:1661](../../crates/sandbox_wasm/src/host_capabilities.rs#L1661))
implements only `memory_growing` and `table_growing`; it never overrides
`instances()`, `memories()`, or `tables()`, so those fall through to the trait
defaults of **10000 each**.

The `StoreLimits` value is therefore consulted for exactly one thing:
`memory_size`, via the delegated `memory_growing`. The three count limits have
never had any effect.

This is not academic — it is why nothing is broken today. Real fixtures
instantiate **three** core modules (`wasm-tools print` on
`syneroym_test_greeter.wasm`: `$main`, `$wit-component-shim-module`,
`$wit-component-fixup`). Had `instances(1)` ever been live, every single
dispatch would have failed with `resource limit exceeded: instance count too
high at 2`.

### F4 — instantiation is on the hot path

`build_store_and_instantiate`
([engine.rs:1210](../../crates/sandbox_wasm/src/engine.rs#L1210)) builds a fresh
`Store` and calls `instance_pre.instantiate_async` on **every** dispatch — 9
call sites covering RPC, proxy, message delivery, guest HTTP, streams,
lifecycle hooks, probes, and the stage-4 ABAC after-step. Only the compiled
`InstancePre` is cached ([engine.rs:146](../../crates/sandbox_wasm/src/engine.rs#L146)).

Fast instantiate/drop is the main thing pooling buys, so this is the one real
cost of the change and the one thing P0 must measure rather than assume.

### F5 — four semaphores exist only because pool exhaustion is a hard error

`stream_instance_permits`, `abac_instance_permits`, `probe_instance_permits`,
and the per-service `guest_http_permits`
([engine.rs:218-311](../../crates/sandbox_wasm/src/engine.rs#L218)) all have the
same doc-comment justification: exhausting the pool is "a hard
`PoolConcurrencyLimitError` at instantiation, not a wait", so each path must
gate itself in front of the pool. Their sizes are derived from
`max_concurrent_instances` via `STREAM_INSTANCE_POOL_HEADROOM`
([engine.rs:339](../../crates/sandbox_wasm/src/engine.rs#L339)), with the
invariant `stream_instance_budget + abac_instance_budget * 2 == max_instances`.

On-demand allocation has no such cliff. The semaphores stay (they are still
useful CPU/memory back-pressure) but their *justification* changes from "pool
slots" to "concurrency", and the arithmetic invariant stops being load-bearing.
Their doc comments become false and must be rewritten.

### F6 — nothing blocks `limiter_async`

- `Config::async_support` is `#[deprecated(note = "no longer has any effect")]`
  in wasmtime 46 — async is always on. Nothing to enable.
- `Store::limiter_async` forbids the synchronous `Func::call`, `Memory::grow`,
  and `Table::grow`. A grep over `crates/sandbox_wasm/src` finds **zero** uses
  of any of them; every guest invocation already goes through `call_async`.
- `ResourceLimiterAsync::memory_growing` is an `async fn` with the same
  `(current, desired, maximum)` signature, so the delegation to `StoreLimits`
  inside it is a plain synchronous call and keeps working unchanged.

### F7 — this closes two open backlog rows and reshapes a third

- Row 38 (router tests flaky under parallel execution): F1 is its leading
  theory, with the arithmetic corrected.
- Row 260 (wasmtime 47.x+ investigated for the probe-concurrency fix): the
  question was whether a newer pooling allocator removes the need for the
  probe semaphore. Removing the pool answers it without the upgrade.
- Row 48 (D-A2-11's per-service gate queues in front of an already-saturated
  pool) stops being a correctness question and becomes a pure tuning question.

### F8 — `HostState::new` has 25 call sites, so it must not grow a parameter

`HostState::new` is called from 25 places, 24 of them tests and benches
(`lifecycle_hooks.rs`, `abac_integration.rs`, `blob_store_integration.rs`,
`benches/wasm_engine.rs`, and the two in-module test suites). Only
[engine.rs:1277](../../crates/sandbox_wasm/src/engine.rs#L1277) is production.
Adding a 15th positional parameter would touch all 25 for no benefit — the
budget is attached by a chained builder method instead (D-WMEM-6).

### F9 — two engine struct literals in tests need the new field

[engine.rs:3028](../../crates/sandbox_wasm/src/engine.rs#L3028) and
[engine.rs:3171](../../crates/sandbox_wasm/src/engine.rs#L3171) construct
`AppSandboxEngine` field-by-field with no `..Default::default()`.

---

## §2 Decisions

| # | Decision | Why |
|---|---|---|
| **D-WMEM-1** | **On-demand allocation only.** The pooling branch is deleted, not made optional | Two allocators means two behaviours to test and two sets of doc comments to keep true. Pre-release, so no compatibility ladder |
| **D-WMEM-2** | **A shared budget bounds the total; the existing per-store cap bounds one service.** Both checks run, per-store first | A global budget alone loses pooling's isolation: one greedy service could starve every other. `default_max_memory_bytes` (256 MiB) already exists and already works — it becomes the blast-radius bound |
| **D-WMEM-3** | **`memory_limit` is repurposed as the global budget**, rather than adding a new field | "Memory limit for the sandbox" is exactly what a shared budget is. It currently feeds `max_memory_size`, which F2 shows is inert, so nothing real changes meaning |
| **D-WMEM-4** | **A short bounded wait, then reject.** `memory_grow_wait_ms`, default 250, `0` = fail fast | Unbounded waiting is hold-and-wait: every store holds memory and waits for memory only another waiter can release, and nothing frees until a store drops. A wait that cannot end must not exist. 250 ms smooths bursts without approaching `dispatch_epoch_timeout_secs` (5 s) |
| **D-WMEM-5** | **`memory_reservation` defaults to 64 MiB, not 0** | 0 makes every growth past the initial size a remap-and-copy. 64 MiB means a component that never exceeds 64 MiB never relocates, while still cutting the startup reservation from 4 TiB to ~96 MiB of lazy VA *per live memory* |
| **D-WMEM-6** | **The budget is attached with a chained `with_memory_budget`**, not a `new` parameter | F8: 25 call sites, 1 of them production. Tests keep compiling unchanged and default to unbudgeted |
| **D-WMEM-7** | **The charge is held as a `tokio::sync::OwnedSemaphorePermit` inside `HostState`** | Release becomes `Drop`, which is already exactly the `Store`'s lifetime. No explicit release path means no leak path and no double-release path |
| **D-WMEM-8** | **One permit = one 64 KiB wasm page** | `Semaphore` permits are `usize`-counted and `acquire_many` takes `u32`. A 1 GiB budget is 16384 permits; a 64 GiB budget is 1048576. Byte-granular permits would overflow `u32` at 4 GiB |
| **D-WMEM-9** | **The four instance semaphores keep their current sizes**, and only their doc comments change | Retuning them is a separate question (backlog row 48) with its own test asserting today's arithmetic. Changing the allocator and the concurrency budgets in one step would make a regression impossible to attribute |
| **D-WMEM-10** | **F3's dead count limits are fixed, not deleted**: `HostState` will override `instances()`/`memories()`/`tables()` from the `StoreLimits` it holds | Leaving them dead after this change would be knowingly shipping inert config. The values must be raised from `1` to something real first — see D-WMEM-11 |
| **D-WMEM-11** | **The per-store counts become `max_core_instances_per_store` = 8, `memories`/`tables` = 8** | Measured: real fixtures use 3 core instances, each with its own memory and table. 8 leaves headroom for a larger component without being unbounded. This is the *only* surviving piece of the deleted per-component knobs, and it now sits where it is actually enforced |

---

## §3 Exact type and signature changes

### 3.1 New module — `crates/sandbox_wasm/src/memory_budget.rs`

```rust
/// Engine-wide ceiling on live guest linear memory, shared by every `Store`.
pub struct WasmMemoryBudget { /* Arc<Semaphore>, total_pages, wait: Duration */ }

pub struct MemoryCharge { /* Option<OwnedSemaphorePermit>, budget: Arc<...> */ }

impl WasmMemoryBudget {
    pub fn new(total_bytes: u64, wait: Duration) -> Arc<Self>;
    /// Reserves `pages` more; `Ok(())` on success. Never blocks past `wait`.
    async fn charge(self: &Arc<Self>, charge: &mut MemoryCharge, pages: u32) -> Result<(), BudgetError>;
    pub fn used_bytes(&self) -> u64;   // for tests and the gauge
    pub fn total_bytes(&self) -> u64;
}
```

`MemoryCharge` holds the merged permit and returns it on `Drop`. No public
release method exists.

`BudgetError` is an enum of `Exhausted { requested, total }` — one variant
today, an enum so a future `Poisoned` does not become a string match.

Exported from [lib.rs](../../crates/sandbox_wasm/src/lib.rs) as
`pub use memory_budget::WasmMemoryBudget;`.

No new dependency: `tokio`, `metrics`, and `anyhow` are already in
[Cargo.toml](../../crates/sandbox_wasm/Cargo.toml).

### 3.2 `crates/sandbox_wasm/src/host_capabilities.rs`

- `HostState` gains two fields: `memory_budget: Option<Arc<WasmMemoryBudget>>`
  and `memory_charge: MemoryCharge`.
- `HostState::new` — signature **unchanged** (D-WMEM-6). Its `StoreLimitsBuilder`
  block changes `.instances(1).memories(1).tables(1)` to the D-WMEM-11 values.
- New: `pub fn with_memory_budget(mut self, budget: Option<Arc<WasmMemoryBudget>>) -> Self`.
- `impl ResourceLimiter for HostState` → **`impl ResourceLimiterAsync for HostState`**,
  with `instances()`, `memories()`, `tables()` overridden to read from
  `self.memory_limits` (D-WMEM-10), and `memory_grow_failed` implemented so an
  allocation failure surfaces as a clean trap rather than a silently ignored
  `false`.

### 3.3 `crates/sandbox_wasm/src/engine.rs`

- `build_wasm_engine(max_instances, max_memory, u32, u32, u32)` →
  **`build_wasm_engine(tuning: WasmMemoryTuning)`**, where `WasmMemoryTuning`
  is a small `Copy` struct of `{ reservation_bytes, reservation_for_growth_bytes }`
  with a `Default` matching D-WMEM-5. All five old parameters are gone: three
  configured the deleted pool, and the other two are now engine-independent.
- `AppSandboxEngine` gains `memory_budget: Arc<WasmMemoryBudget>`.
- `init` builds the budget from `memory_limit_bytes()` and passes the tuning
  struct through.
- [engine.rs:1300](../../crates/sandbox_wasm/src/engine.rs#L1300):
  `store.limiter(|state| state)` → `store.limiter_async(|state| state)`.
- [engine.rs:1277](../../crates/sandbox_wasm/src/engine.rs#L1277): the
  `HostState::new(...)` call gains `.with_memory_budget(Some(self.memory_budget.clone()))`.
- `classify_call_failure` ([engine.rs:398](../../crates/sandbox_wasm/src/engine.rs#L398))
  gains one more `contains` arm for the budget-rejection message so it maps to
  `CallFailure::MemoryFault` like every other memory failure.
- Doc comments on the four semaphore fields and on
  `STREAM_INSTANCE_POOL_HEADROOM` rewritten per F5 — they currently describe a
  pool that will not exist.

### 3.4 `crates/core/src/config.rs` — `AppSandboxRole`

**Removed** (they exist only to size the pool):

```
max_core_instances_per_component
max_memories_per_component
max_tables_per_component
```

**Added**:

```rust
/// Address space reserved per live guest linear memory. A memory that stays
/// under this never has to be moved and copied as it grows.
pub memory_reservation_bytes: u64,          // default 64 MiB
/// How much slack is added past the reservation when a memory does have to
/// move, so a guest growing in small steps does not copy on every step.
pub memory_reservation_for_growth_bytes: u64, // default 4 MiB
/// How long a guest waits for the shared budget before its growth is
/// refused. 0 refuses immediately.
pub memory_grow_wait_ms: u64,               // default 250
/// Per-store ceilings on core instances, linear memories, and tables.
pub max_core_instances_per_store: u32,      // default 8
pub max_memories_per_store: u32,            // default 8
pub max_tables_per_store: u32,              // default 8
```

`memory_limit` / `memory_limit_bytes()` keep their names and gain a doc comment
saying they are now the shared budget across all stores (D-WMEM-3).
`max_concurrent_instances` keeps its name; its doc comment changes from a pool
size to a concurrency bound.

### 3.5 Metrics

Two new, following the existing `metrics::` macro style at
[engine.rs:369](../../crates/sandbox_wasm/src/engine.rs#L369):

- `substrate.wasm.memory_budget_used_bytes` (gauge, updated in `charge` and in
  `MemoryCharge::drop`)
- `substrate.wasm.memory_budget_rejections_total` (counter)

---

## §4 Call sites

| # | File | Change |
|---|---|---|
| 1 | [engine.rs:484](../../crates/sandbox_wasm/src/engine.rs#L484) | `init` — build budget, pass tuning |
| 2 | [engine.rs:464-481](../../crates/sandbox_wasm/src/engine.rs#L464) | the 5-tuple config read collapses to reading the new fields |
| 3 | [engine.rs:1277](../../crates/sandbox_wasm/src/engine.rs#L1277) | `.with_memory_budget(...)` |
| 4 | [engine.rs:1300](../../crates/sandbox_wasm/src/engine.rs#L1300) | `limiter_async` |
| 5 | [engine.rs:2890](../../crates/sandbox_wasm/src/engine.rs#L2890), [:3142](../../crates/sandbox_wasm/src/engine.rs#L3142) | `build_wasm_engine(None, None, 0, 0, 0)` → `build_wasm_engine(Default::default())` |
| 6 | [engine.rs:2997](../../crates/sandbox_wasm/src/engine.rs#L2997) | same, drops the pooling arguments |
| 7 | [engine.rs:3028](../../crates/sandbox_wasm/src/engine.rs#L3028), [:3171](../../crates/sandbox_wasm/src/engine.rs#L3171) | struct literals gain `memory_budget` (F9) |
| 8 | [engine.rs:3079-3140](../../crates/sandbox_wasm/src/engine.rs#L3079) | `a_component_at_the_configured_per_component_resource_max_still_instantiates` — rewritten, see §7 |
| 9 | [benches/wasm_engine.rs:65](../../crates/sandbox_wasm/benches/wasm_engine.rs#L65) | new `build_wasm_engine` signature |
| 10 | [host_capabilities.rs:178](../../crates/sandbox_wasm/src/host_capabilities.rs#L178) | `StoreLimitsBuilder` counts |
| 11 | [host_capabilities.rs:1661](../../crates/sandbox_wasm/src/host_capabilities.rs#L1661) | `ResourceLimiterAsync` |
| 12 | [config.rs:485-497](../../crates/core/src/config.rs#L485), [:643](../../crates/core/src/config.rs#L643) | field add/remove plus `Default` |
| 13 | [config.rs:420-448](../../crates/core/src/config.rs#L420) | the three `default_max_*_per_component` fns are deleted; six new default fns |
| 14 | [proxy_dispatch.rs:85-90](../../crates/router/tests/proxy_dispatch.rs#L85) | drops the three removed fields; keeps `memory_limit`/`max_concurrent_instances` |
| 15 | [config.sample.toml:134](../../crates/substrate/config.sample.toml#L134) | documents the new knobs |
| 16 | [smoke-tests/src/main.rs:353](../../crates/smoke-tests/src/main.rs#L353) | assertion string, see §7 |

Everything else is untouched. In particular: no WIT change, no router change,
no control-plane change, no change to any e2e harness.

---

## §5 Pseudo-code

### 5.1 `WasmMemoryBudget::charge`

```
pages_needed = requested_pages
if pages_needed == 0: return Ok

# Fast path: capacity free right now.
if let Ok(p) = semaphore.clone().try_acquire_many_owned(pages_needed):
    charge.absorb(p); gauge.increment(); return Ok

if wait.is_zero():
    counter!("...rejections_total").increment(1)
    return Err(Exhausted)

# Bounded wait. This is the ONLY await, and it can never outlive `wait`
# (D-WMEM-4): every holder is a live Store, and a Store only frees on drop,
# so a wait with no deadline could never be woken by anything but luck.
match timeout(wait, semaphore.clone().acquire_many_owned(pages_needed)).await:
    Ok(Ok(p)) => { charge.absorb(p); gauge.increment(); Ok }
    _         => { counter.increment(1); Err(Exhausted) }
```

`absorb` is `OwnedSemaphorePermit::merge`, so one `MemoryCharge` accumulates
every growth of that store into a single permit released once on drop.

### 5.2 `HostState::memory_growing` (async)

```
# 1. Per-service ceiling FIRST (D-WMEM-2). A service over its own quota is
#    refused whether or not the node has spare capacity -- otherwise a
#    quota would silently become "whatever is left over".
if !self.memory_limits.memory_growing(current, desired, maximum)?:
    return Err("MemoryFault: Wasm execution exceeded memory limit")

# 2. Node-wide budget, charged on the DELTA only. `current` is what this
#    store already paid for; charging `desired` would double-charge every
#    growth after the first.
if let Some(budget) = &self.memory_budget:
    delta_pages = ceil_to_pages(desired - current)
    budget.charge(&mut self.memory_charge, delta_pages).await
        .map_err(|_| Error::msg("MemoryFault: node wasm memory budget exhausted"))?

Ok(true)
```

Two notes for the implementer:

- The epoch deadline does not fire while this awaits — epoch interruption is
  checked at wasm instruction boundaries, and this is host code. It fires on
  the instruction after the wait ends. That is correct behaviour: a store that
  waited 250 ms for memory still gets killed if it then overruns its 5 s.
- `memory_growing` returning `Ok(false)` and returning `Err` differ: `false`
  lets the guest's own allocator see a failed `memory.grow` (which for Rust
  guests aborts via `unreachable`), `Err` traps immediately with our message.
  The current code returns `Err`; keep that, so `classify_call_failure` sees a
  message it can classify.

### 5.3 `build_wasm_engine`

```
config.wasm_component_model(true)
config.consume_fuel(true)
config.epoch_interruption(true)
config.memory_init_cow(true)            # was inside the pooling branch only
config.memory_may_move(true)            # explicit: growth past the reservation relocates
config.memory_reservation(tuning.reservation_bytes)
config.memory_reservation_for_growth(tuning.reservation_for_growth_bytes)
# no allocation_strategy call -- on-demand is the default
Engine::new(&config)
```

---

## §6 Phases

| Phase | Content | Gate |
|---|---|---|
| **P0** | **Measure first.** Parameterize `benches/wasm_engine.rs`'s `wasm_cached_instantiation` over both allocators and record both numbers. Note that this bench builds its engine with `(None, None, 0, 0, 0)` today — it has only ever measured the *on-demand* path, never production's pooled one, so there is currently no pooling baseline anywhere | Two numbers written into the plan. If on-demand instantiation is worse by more than ~2× *and* that difference is a measurable share of `substrate.wasm.execution_ms`, stop and reconsider D-WMEM-1 |
| **P1** | `memory_budget.rs` + unit tests, standalone. No engine wiring | `cargo test -p syneroym-sandbox-wasm` |
| **P2** | Config fields added and removed (§3.4), `config.sample.toml` updated, `proxy_dispatch.rs` fixed | `cargo build --workspace` |
| **P3** | `build_wasm_engine` rewritten, `HostState` switched to `ResourceLimiterAsync`, budget wired through, F3's count limits made live | `cargo test -p syneroym-sandbox-wasm -p syneroym-router` |
| **P4** | Doc comments on the four semaphores and `STREAM_INSTANCE_POOL_HEADROOM` rewritten (F5); metrics added | review only |
| **P5** | Full completion pass: `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets --all-features`, `cargo test --workspace`, `mise run test:e2e`, backlog rows (§9) | all green |

P1 and P2 are independent and can be done in either order. P3 depends on both.

---

## §7 Tests

**New, in `memory_budget.rs`:**

1. A charge under the budget succeeds; `used_bytes` reflects it.
2. Dropping the `MemoryCharge` returns the pages — `used_bytes` back to 0.
3. Repeated growth of one charge accumulates into one permit and releases once
   (guards the `merge` in `absorb`).
4. A charge over the remaining budget is refused with `Exhausted`, and
   `used_bytes` is unchanged (no partial charge left behind).
5. With `wait = 0`, refusal is immediate — asserted as elapsed time under a
   generous ceiling, not as an exact value.
6. With `wait > 0`, a charge that cannot be met returns `Exhausted` after
   approximately `wait` and not before. This is the D-WMEM-4 deadlock guard:
   it fails loudly if someone later removes the timeout.
7. A charge released by a dropped store unblocks a waiter within `wait`.

**New, in `engine.rs` tests:**

8. Two engines built back to back both succeed, and the process's mapped
   address space does not grow by terabytes. Practical form: assert the
   configured reservation is what we set, and that N engines build — the
   4 TiB reservation is what makes "build 8 engines in one process" a real
   risk, so build 8.
9. Per-store cap bites before the global budget: a store with a 1 MiB
   `default_max_memory_bytes` inside a 1 GiB budget still fails at 1 MiB.
10. The global budget bites across stores: two concurrent stores, each within
    its own per-store cap, together exceeding a deliberately small budget —
    the second is refused, and refused with a message
    `classify_call_failure` maps to `MemoryFault`.

**Rewritten:**

11. `a_component_at_the_configured_per_component_resource_max_still_instantiates`
    ([engine.rs:3079](../../crates/sandbox_wasm/src/engine.rs#L3079)) currently
    proves a pooling-allocator contract that will not exist. Its intent —
    *a component at the declared ceiling instantiates, one over it fails
    clearly* — survives, retargeted at D-WMEM-11's per-store counts. Its
    `n_module_component_wat` helper is reusable as is, but the store must now
    carry a `HostState` limiter (the current version uses `Store::new(&engine, ())`,
    which is exactly why it could not have caught F3).

**Adjusted assertions:**

12. [engine.rs:3069](../../crates/sandbox_wasm/src/engine.rs#L3069) and
    [smoke-tests/src/main.rs:353](../../crates/smoke-tests/src/main.rs#L353)
    both accept `MemoryFault` or `failed to grow memory`. The per-store path
    still produces `MemoryFault`, so both keep passing unchanged — but confirm
    it rather than assume it, since the guest-side abort path changes shape
    when a memory can move.

**Unchanged and must stay green** (they pin behaviour this change could break):

13. `test_stream_instances_across_services_bounded_by_shared_pool_budget`
    ([stream_integration.rs:684](../../crates/sandbox_wasm/tests/stream_integration.rs#L684))
    — asserts the stream semaphore's arithmetic. D-WMEM-9 keeps the sizes, so
    it should pass untouched. Its name and doc comment say "instance pool",
    which becomes inaccurate; rename in P4.
14. The whole `guest_http_e2e.rs` suite, including the two tests that set
    `max_concurrent_guest_http_per_service` to 1 and 2.
15. `mise run test:e2e` — the Playwright run is the realistic concurrent-load
    check, and is the closest thing to a direct test of the problem being fixed.

---

## §8 Documents this change edits

| Document | Edit |
|---|---|
| [config.sample.toml](../../crates/substrate/config.sample.toml#L134) | new knobs documented, removed ones deleted |
| [deferred-backlog.md](deferred-backlog.md) | rows 38, 48, 260 — see §9 |
| [developer-guide.md](../developer-guide.md) | only if a sandbox-tuning section is added; today it documents ports, not sandbox memory |

No ADR. This changes an implementation strategy inside one crate, not an
interface or a cross-cutting decision. If review disagrees, the natural home
would be a short ADR on "guest memory is budgeted, not partitioned".

---

## §9 Backlog rows owed

**Resolved** (move to *Recently resolved* with the evidence):

- Row 38 — root cause confirmed and removed, with the corrected arithmetic
  from F1. Keep the row's second sighting (the mainline-DHT flake) open as its
  own row; it is unrelated and this change does not touch it.
- Row 260 — answered without the wasmtime upgrade.

**Reshaped, stays open:**

- Row 48 — rewrite to say the pool it referred to no longer exists, so the
  question is now "are the four semaphore budgets the right *concurrency*
  numbers", with A3/A4's Playwright run still the measurement to tune from.

**New rows owed:**

- The four instance semaphores were kept at pool-derived sizes and not
  retuned for a world without a pool (D-WMEM-9). Target: with row 48.
- `memory_grow_wait_ms` is a first guess, not a measurement (D-WMEM-4).
- F3's dead count limits shipped inert for the life of the pooling
  implementation; nothing in the test suite could have caught it, because the
  one test aimed at per-component limits used a store with no limiter at all.
  Worth a row asking whether other `ResourceLimiter`-style delegations have
  the same shape.

Per the repo's *No Planning-Doc References in Code* rule, none of the
`D-WMEM-*` identifiers above may appear in code comments. Comments explain the
invariant (deadlock, double-charge, per-service-before-global); this document
and the commit message carry the rationale.

---

## §10 Risks

| Risk | Handling |
|---|---|
| On-demand instantiation is slower on a per-dispatch hot path (F4) | P0 measures before any code is written, and is an explicit stop gate |
| Bounds-check elision is lost below a 4 GiB reservation | Accepted. Affects memory-heavy guests by a few percent; the 64 MiB reservation (D-WMEM-5) keeps the mapping cheap without pretending to restore elision. Measurable in `bench:latency` |
| A shared budget lets one service crowd out others | Bounded by the per-store cap (D-WMEM-2), which is unchanged and already enforced |
| The async wait deadlocks the sandbox | Structurally prevented by the timeout (D-WMEM-4) and pinned by test 6 |
| Growth relocation copies large memories | `memory_reservation_for_growth` (default 4 MiB) amortizes it; both values are config, changeable without a rebuild |
| The change is wrong and must be reverted | It is one crate plus config fields, no WIT and no wire format. Revert is a straight `git revert` with no data or protocol consequences |
