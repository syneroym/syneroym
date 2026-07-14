# M04A Status

## Slice A0′ — Full WIT⇄JSON Value Conversion ✅ (2026-07-14)

Branch: `feat/m04a-a0-prime`. Requirement `[PLT-DAT]` (typed dispatch). No ADR
dependency. Plan: [plans/A0-prime.md](plans/A0-prime.md).

### What was delivered

Replaced the `crates/sandbox_wasm/src/conversions.rs` stub (which handled only
`String`/`U32`/`Bool` on input and string-or-`{:?}`-debug on output) with a full
bidirectional component-model ↔ JSON converter:

- **`val_to_json(&Val) -> Value`** and **`json_to_val(&Value, &Type) -> Val`** —
  two `Type`-directed primitives covering the entire WIT type system: bool, all
  integer widths, `f32`/`f64`, `char`, `string`, `list`, `tuple`, `record`,
  `variant`, `enum`, `option`, `result`, `flags`, `map`; resource/future/stream/
  error-context return a typed "unsupported" error.
- **`json_to_wasm_params`** now switches on the JSON-RPC params shape: a JSON
  **object** binds parameters **by name**, an **array** binds positionally, and a
  missing `option<_>` parameter becomes `none` (the `conversions.rs` TODO). It
  now takes `&Value` instead of `Vec<Value>`; the only callers are
  `engine.rs:508` and the crate benchmark, both updated.
- **`wasm_results_to_json_string`** reimplemented over `val_to_json` while
  **preserving the existing boundary contract** (a `string` result is returned
  raw, not JSON-quoted; a WIT `result::err` becomes a transport error). Fully
  typing the JSON-RPC `result` field is deliberately left to Slice A1, which owns
  `route_handler/dispatch.rs` — A0′ must not break `dispatch.rs` or the
  integration tests that parse the raw string.
- The lossy-edge JSON encoding conventions (the A.5 "design note") live as the
  module doc-comment at the top of `conversions.rs`.

### Lossy edges — pinned and tested (no silent corruption)

- **`u64`/`s64` > 2^53**: emitted as native JSON numbers; `serde_json::Value`
  stores them losslessly, so in-process round-trips are exact for the full 64-bit
  range. The gap is interop-only (IEEE-754/JS consumers above 2^53). Tested to
  `u64::MAX` / `i64::MIN`.
- **`char` vs `string`**: `char` ⇄ one-scalar JSON string; indistinguishable from
  a length-1 `string` at the JSON layer, disambiguated by the WIT `Type` on
  decode.
- **nested `option<option<T>>`**: `null` deterministically collapses outer `none`
  and `some(none)` → both encode to `null`, `null` decodes to outer `none`. Tested.
- **non-finite floats**: encoding `NaN`/`±Inf` is a hard error (never `null`);
  decoding a finite-but-out-of-`f32`-range number (which would cast to `±inf`) is
  likewise an error. Tested.

### Tests

`cargo test -p syneroym-sandbox-wasm --lib conversions` → 18 passing. Strategy:

1. **`val_to_json` (encode)** — exhaustive hand-built `Val` for every variant +
   every lossy edge. No component needed.
2. **`json_to_val` (decode) round-trip** — `Type`s harvested from real
   components:
   - scalars + `char`/`bool`/`tuple`/`option`/`result`/nested-`option`/`flags`: a
     memory-free, hand-written component-model WAT fixture (these types are flat
     in the canonical ABI, so no linear memory/realloc is needed). `flags` needed
     one extra trick (below) since it's a *nominal* type.
   - `record`/`variant`/`enum`/`list`/`string` heap composites: the prebuilt
     `data-layer-test` component (`record-write-value`, `query-options`,
     `data-layer-error` variant, `index-type` enum), skip-if-artifact-missing.
3. **Named/positional param binding** and the **`wasm_results_to_json_string`
   boundary contract** (raw string for string results, `err` → transport error,
   non-string → JSON, multi-result `err` also propagates) have dedicated tests.

**`flags` decode via WAT — the named-export-alias trick.** The component model
requires nominal types (`record`/`variant`/`enum`/`flags`) referenced by an
exported function to themselves be *exported by name* — a bare inline
`(flags ...)` or a referenced-but-unexported `(type $t (flags ...))` both fail
`Component::new` with "func not valid to be used as export" (verified
empirically across several syntax attempts). The fix, found during the A0′
code-review pass: export the type through a **named alias** —
`(export $alias "name" (type $t))` — and reference `$alias` (not `$t`) in the
function signature. `(export "name" (type $t))` *without* binding `$alias` does
**not** work (it registers a distinct export-slot type id, not `$t`'s own id, so
the func's reference to `$t` is still "unnamed"). This closed the `flags` decode
gap; `record`/`variant`/`enum` didn't need it since data-layer-test already
supplies real named-and-exported instances of those.

**`map<K,V>` remains untested on the decode side, and is not reachable in
practice.** `map` requires wasmtime's unstable `wasm_component_model_map` engine
feature (confirmed: compiling a component with a `map` type fails with "Maps
require the component model map feature" unless that flag is set), and
`AppSandboxEngine::build_wasm_engine` (the only engine construction path used by
the real substrate) does not enable it. So `Type::Map`/`Val::Map` cannot occur
for any component this substrate can actually load today — spending effort to
fabricate a non-production test engine just to exercise it would be testing a
structurally unreachable path. `map` **encode** is still fully covered by
`val_to_json` tests. The resource/future/stream/error-context "unsupported →
error" arms are covered by `match` exhaustiveness (not a fabricated `Val`/`Type`,
since `ResourceAny` has no public constructor).

### Performance (criterion, `--bench wasm_engine`)

| Bench | Measured |
|---|---|
| `json_to_wasm_params` (bind one `string` param) | ~76 ns |
| `wit_json_roundtrip` (`val_to_json` of a `record-read-value`-shaped record with a 256-byte `list<u8>`) | ~2.69 µs |

Budget was "must not dominate same-node call latency" (same-node Universal Proxy
budget is < 5 ms p99). Record encode at ~2.7 µs is ~0.05% of that — negligible.

### Post-commit code review (2026-07-14) — findings incorporated

A follow-up code review of the committed diff found five items. Verified each
against the actual code/config before acting:

- **Fixed — f32 decode silently underflows to `0.0`.** The original guard only
  caught overflow (`!f.is_finite()`); a finite JSON number smaller than f32's
  minimum subnormal (e.g. `1e-50`) cast to exactly `0.0`, passing the guard and
  silently discarding the value. Now also rejects `f == 0.0 && original != 0.0`.
  Tested.
- **Fixed — `result` decode silently dropped a payload on a unit arm.**
  `decode_result_arm(_, None)` returned `Ok(None)` regardless of the JSON given,
  so `{"ok": 5}` against `result<_, E>` (no `ok` payload) silently accepted and
  discarded the `5` — inconsistent with every other decode path's strictness.
  Now requires the JSON to be `null` when the arm has no payload type, else
  errors. Tested.
- **Fixed — multi-result `Err` propagation inconsistency.** The single-result
  path turned a WIT `result::err` into a transport `Err`; the multi-result
  (`&[Val]` len ≥ 2) path did not, JSON-serializing an error as if it were
  success data. Fixed for consistency, though verified this arm is currently
  **unreachable**: WIT surface syntax cannot declare a function with more than
  one top-level result value (multi-value returns are expressed as a single
  tuple), confirmed by `grep`-ing every `.wit` file in the repo. Tested anyway.
- **Fixed — `invoke_test_context` (`engine.rs`) would now error on a call it
  used to silently no-op.** It sends `params: Value::String(request_ctx)` to a
  method hardcoded to `"run"`, and `host.wit`'s `app::run` is genuinely zero-arg
  — so `request_ctx` was *already* never reaching the guest before A0′ (the old
  converter's `for` loop over an empty param iterator silently ignored it).
  Confirmed via repo-wide `grep`: **zero callers** of this `pub fn` exist
  anywhere. A0′'s stricter binding turns this pre-existing silent no-op into a
  loud error. Fixed the call site to send `Value::Null` (matching the real
  0-arg signature) instead of chasing down what the function's intent might
  have been — it is untested, uncalled scaffolding.
- **Partially addressed — `flags`/`map` decode test coverage.** Found the
  correct WAT syntax to harvest a real `Type::Flags` (the named-export-alias
  trick, documented above) and added round-trip + rejection tests. `map` decode
  stays untested — pushed back on this one: it requires enabling an unstable
  wasmtime feature (`wasm_component_model_map`) that production's engine
  construction path never enables, so testing it would exercise code the real
  system cannot reach. Documented explicitly in the module doc-comment instead
  of built.

### Gate

- `cargo +nightly fmt --all` — clean.
- `cargo clippy --workspace --all-targets --all-features` — zero warnings.
- `cargo test --workspace` — **358 passed, 0 failed** (16 → 18 conversions
  tests after the review fixes).
- `wasm32-wasip2` — builds (verified via `test-components/data-layer-test`); A0′
  adds no WIT types, so the guest surface is unchanged.

**Environment note:** under the agent command sandbox, network-binding
integration tests (e.g. `syneroym-coordinator-iroh`'s `connection_limit`, which
spawns an iroh relay server) fail with "Operation not permitted (os error 1)" on
`bind`. These are unrelated to A0′ and pass with the sandbox disabled; the 356/0
figure above is from the full suite run without the command sandbox.

### Scope discipline

Only Slice A0′ was touched: `conversions.rs`, its two call sites
(`engine.rs`, `benches/wasm_engine.rs`), and this planning doc set. No other
slice (B0 identity threading, A1 proxy, etc.), no WIT file, and no reserved
`wrpc`/`AdaptationStage` seam were modified. `mise run test:e2e` (reference
scenario steps) belongs to A1/B0 and was not run for A0′.
