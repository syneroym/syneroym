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

`cargo test -p syneroym-sandbox-wasm --lib conversions` → 16 passing. Strategy:

1. **`val_to_json` (encode)** — exhaustive hand-built `Val` for every variant +
   every lossy edge. No component needed.
2. **`json_to_val` (decode) round-trip** — `Type`s harvested from real
   components:
   - scalars + `char`/`bool`/`tuple`/`option`/`result`/nested-`option`: a
     memory-free, hand-written component-model WAT fixture (these types are flat
     in the canonical ABI, so no linear memory/realloc is needed).
   - `record`/`variant`/`enum`/`list`/`string` heap composites: the prebuilt
     `data-layer-test` component (`record-write-value`, `query-options`,
     `data-layer-error` variant, `index-type` enum), skip-if-artifact-missing.
3. **Named/positional param binding** and the **`wasm_results_to_json_string`
   boundary contract** (raw string for string results, `err` → transport error,
   non-string → JSON) have dedicated tests.

**Known test-coverage limitation (honest note):** `flags` and `map` **decode**
(`json_to_val`) is *not* round-tripped against a real WIT `Type`. Neither type is
used by any live WIT interface in the repo (`grep` confirms no `flags`/`map` in
`crates/wit_interfaces/wit` or the test components), and the component model
requires nominal types (`record`/`variant`/`enum`/`flags`) referenced by an
exported function to be *named* — which is not expressible in a simple top-level
hand-written WAT (verified empirically: `record`/`variant`/`flags` fail
`Component::new` with "func not valid to be used as export", while structural
`tuple`/`option`/`result` succeed). `record`/`variant`/`enum` are therefore
covered via the real data-layer-test component instead; `flags`/`map` **encode**
is fully covered by `val_to_json` tests, and their decode logic is simple and
structurally identical to the tested `enum`/`list` paths. Also, the
resource/future/stream/error-context "unsupported → error" arms are covered by
`match` exhaustiveness (not a fabricated `Val`/`Type`, since `ResourceAny` has no
public constructor).

### Performance (criterion, `--bench wasm_engine`)

| Bench | Measured |
|---|---|
| `json_to_wasm_params` (bind one `string` param) | ~76 ns |
| `wit_json_roundtrip` (`val_to_json` of a `record-read-value`-shaped record with a 256-byte `list<u8>`) | ~2.69 µs |

Budget was "must not dominate same-node call latency" (same-node Universal Proxy
budget is < 5 ms p99). Record encode at ~2.7 µs is ~0.05% of that — negligible.

### Gate

- `cargo +nightly fmt --all` — clean.
- `cargo clippy --workspace --all-targets --all-features` — zero warnings.
- `cargo test --workspace` — **356 passed, 0 failed**.
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
