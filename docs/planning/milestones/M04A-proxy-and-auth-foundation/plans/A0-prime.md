# Slice A0′ Implementation Plan — Full WIT⇄JSON Value Conversion

> **Scope.** Slice A0′ only, from
> [M04A task.md](../task.md) (§"Slice A0′: Full WIT⇄JSON Value Conversion").
> No ADR blocks it. Requirement `[PLT-DAT]` (typed dispatch). Branch:
> `feat/m04a-a0-prime`. This plan resolves every open judgment call so
> implementation is mechanical.

## 1. Objective

Replace the stub at `crates/sandbox_wasm/src/conversions.rs` (today handles only
`String`/`U32`/`Bool` on input, string-or-`{:?}`-debug on output) with a
**complete, bidirectional** component-model ↔ JSON converter covering the entire
WIT type system, plus:

- switch positional → **named** parameter binding (the file's `TODO`), keeping
  positional as a fallback;
- pin the **lossy-edge encodings** (`u64 > 2^53`, `char` vs `string`, nested
  `option<option<T>>`, non-finite floats) as documented conventions, not hacks;
- add round-trip unit tests across the full type set and a `criterion`
  conversion benchmark.

Non-goals for A0′ (belong to A1/B0, do **not** touch): rewiring the JSON-RPC
`result` field to a fully-typed value in `dispatch.rs`, the `AdaptationStage`
proxy seam, identity threading, `NativeInvocation` shape.

## 2. Target API (module `conversions.rs`)

Two **pure, Type-directed** primitives do all the work; the two existing public
functions become thin adapters over them (call sites keep working).

```rust
/// WIT component value  ->  JSON. Self-describing; no Type needed.
pub fn val_to_json(val: &Val) -> Result<Value>;

/// JSON  ->  WIT component value, directed by the target WIT `Type`.
pub fn json_to_val(json: &Value, ty: &Type) -> Result<Val>;

/// Bind a JSON-RPC params payload (object = named, array/scalar = positional)
/// to a function's typed parameter list. Now takes `&Value`, not `Vec<Value>`.
pub fn json_to_wasm_params<'a>(
    params_iter: impl Iterator<Item = (&'a str, Type)>,
    json_params: &Value,
) -> Result<Vec<Val>>;

/// Function results -> string for the current JSON-RPC boundary.
/// Reimplemented on `val_to_json`, but preserves today's boundary contract.
pub fn wasm_results_to_json_string(wasm_results: &[Val]) -> Result<String>;
```

Rationale: `val_to_json` needs no `Type` (a `Val` is self-describing) so it is
exhaustively unit-testable with hand-built values; `json_to_val` is
`Type`-directed because JSON is lossy (a `null` could be `option::none`, a
one-char string could be `char` or `string`, an object could be a record or a
map — only the WIT `Type` disambiguates).

## 3. Full type mapping

`Type` variant (wasmtime 46) → `Val` → JSON. Both directions are exact inverses
except at the documented lossy edges (§4).

| WIT / `Type` | `Val` | JSON encoding | Decode (`json_to_val`) rule |
|---|---|---|---|
| `bool` | `Bool` | `Bool` | `as_bool()` else error |
| `s8 s16 s32` | `S8 S16 S32` | `Number` (i64) | `as_i64()` range-checked into width; else error |
| `u8 u16 u32` | `U8 U16 U32` | `Number` (u64) | `as_u64()` range-checked into width; else error |
| `s64` | `S64` | `Number` (i64) | `as_i64()`; else error |
| `u64` | `U64` | `Number` (u64) | `as_u64()`; else error |
| `f32` | `Float32` | `Number` (f64) or **error** if non-finite | `as_f64()` → `as f32`, then re-check `is_finite()` (a finite f64 out of f32 range casts to ±inf) → error if not finite |
| `f64` | `Float64` | `Number` (f64) or **error** if non-finite | `as_f64()`; else error |
| `char` | `Char` | `String` of exactly 1 scalar | one-char string → that char; else error |
| `string` | `String` | `String` | `as_str()` else error |
| `list<T>` | `List` | `Array` | each elem `json_to_val(_, elem_ty)` |
| `tuple<A,B,…>` | `Tuple` | `Array` (positional, fixed arity) | length must equal arity; per-index type |
| `record{…}` | `Record` | `Object` (WIT field names, kebab-case) | each field by name; missing non-`option` field = error; missing `option` field = `none` |
| `variant{…}` | `Variant` | `{"tag": name[, "val": payload]}` | look up case by `tag`; `val` required iff case has payload |
| `enum{…}` | `Enum` | `String` (case name) | must match a case name; else error |
| `option<T>` | `Option` | `null` \| `json(T)` | `null` → `none`; else `some(json_to_val(_,T))` |
| `result<T,E>` | `Result` | `{"ok": json(T)?}` \| `{"err": json(E)?}` | exactly one key; payload optional per `ok`/`err` presence |
| `flags{…}` | `Flags` | `Array` of enabled flag names | array of strings; each must be a declared flag |
| `map<K,V>` | `Map` | `Object` if `K=string`, else `Array` of `[k,v]` | inverse of encode (see §4.5) |
| `own`/`borrow` (resource) | `Resource` | — | **unsupported** → typed error both directions |
| `future`/`stream`/`error-context` | `Future`/`Stream`/`ErrorContext` | — | **unsupported** → typed error both directions |

Encoding notes:

- **Records** map to JSON objects keyed by the WIT field name verbatim (WIT
  names are kebab-case, e.g. `creator-id`, `next-cursor`) — no rename to
  snake/camel. Decode is by exact name.
- **Variants** use an explicit tag object. `{"tag":"put","val":{…}}`;
  payload-less cases (e.g. `permission-denied`) encode as `{"tag":"permission-denied"}`
  (no `val`). This handles `data-layer-error` (mixed payload/no-payload) and
  `mutation` cleanly. A record can never collide because decode is `Type`-directed.
- **result** uses `{"ok":…}` / `{"err":…}`. `result<_, E>` (no ok payload):
  success is `{"ok":null}`; `result<T>` (no err payload): failure is
  `{"err":null}`. Note this is the *value-level* encoding used by `val_to_json`
  / `json_to_val`; it is distinct from the *transport-level* err→JSON-RPC-error
  behavior kept in `wasm_results_to_json_string` (§5), which stays as-is for
  backward compatibility.
- **flags** → JSON array of the set flag names (order = declaration order on
  encode; any order accepted on decode).

**Decode leniency/strictness pinned (determinism at edges):**
- *record*: extra JSON keys not named by the WIT record are ignored (forward-compat).
- *variant*: `tag` required; extra keys beyond `tag`/`val` are ignored; a
  spurious `val` on a payload-less case is ignored (tag alone decides); `val`
  required and non-omittable iff the resolved case has a payload type.
- *flags*: each array entry must be a declared flag name (unknown → error);
  duplicates collapse (set semantics); non-string entries → error.
- *enum*: string must match a declared case (unknown → error).
- *result*: object must have exactly one of `ok`/`err` (both or neither → error).

## 4. Lossy edges — pinned conventions (documented, not worked around)

These are recorded authoritatively as a module doc-comment at the top of
`conversions.rs` (lives with the code) and summarized in `status.md`. Each has a
dedicated round-trip test asserting the *documented deterministic* behavior — so
"no silent corruption": the behavior is defined and stable, even where fidelity
is imperfect.

### 4.1 `u64` / `s64` > 2^53
`serde_json::Value::Number` stores `u64`/`i64` **losslessly**, so an in-process
Rust round-trip (`json_to_val ∘ val_to_json`) is exact for the full 64-bit
range. The gap is **interop-only**: a consumer that parses the serialized JSON
with IEEE-754 doubles (e.g. JavaScript `JSON.parse`) loses precision above
`2^53`. Convention: emit native JSON numbers; do **not** stringify big integers.
Documented as a known interop limitation. Test: round-trips `u64::MAX`,
`i64::MIN`, `2^53+1` exactly through `Value`.

### 4.2 `char` vs `string`
A WIT `char` and a length-1 WIT `string` both encode to a one-character JSON
string; JSON cannot distinguish them. This is unambiguous in **typed** decode
(the WIT `Type` says which). Convention: `char` ⇄ one-scalar string; decode
errors if the string is empty or has >1 `char`. Documented: at the JSON layer
alone (no type), the two are indistinguishable. Test: `Val::Char('λ')` ⇄
`"λ"`, and multi-char string decoded as `char` errors.

### 4.3 nested `option<option<T>>`
JSON `null` collapses the two "empty" states of a nested option: outer `none`
and `some(none)` both serialize to `null`. Convention (deterministic):
- encode: `none → null`; `some(none) → null`; `some(some(v)) → json(v)`.
- decode at `option<option<T>>`: `null → none` (outer); non-null →
  `some(some(decode(v)))`.

So `some(none)` round-trips to `none` — a **documented, deterministic collapse**,
not corruption. Test asserts exactly this (encodes `some(none)`, decodes back to
`none`) and that `some(some(5))` is lossless. (Single-level `option<T>` is fully
lossless.)

### 4.4 non-finite floats
`serde_json` cannot represent `NaN`/`±Infinity` as a JSON number
(`Number::from_f64` returns `None`). Convention: encoding a non-finite `f32`/`f64`
is a **hard typed error** ("non-finite float cannot be represented in JSON") —
honest failure over silent `null`-corruption. Finite floats round-trip via
`f64`. **The decode side must also enforce this invariant symmetrically**: for
`f32`, a *finite* JSON number outside `f32` range casts to `±inf` via `as f32`,
so re-check `is_finite()` after the narrowing cast and error — never silently
produce a non-finite `Val::Float32`. Test: `Val::Float64(f64::NAN)` →
`val_to_json` errors; a JSON `1e40` decoded at `f32` errors; `1.5f32`/`f64`
round-trip.

### 4.5 `map` key typing
`map<string,V>` → JSON object; `map<K,V>` with non-string `K` → JSON array of
`[k,v]` two-element arrays (objects require string keys). Decode inverts using
the `Map` key `Type`. `map` is not in A0′'s enumerated required set (WIT has no
first-class `map`; this is wasmtime's `Val::Map`) — implemented for completeness
so no live type hits the catch-all error, lightly tested.

## 5. `wasm_results_to_json_string` — preserve the boundary contract

**Critical compatibility finding.** Today the function returns a **raw** string
(not JSON-quoted): `dispatch.rs` wraps it as `result: Value::String(…)`, and
tests do `result.parse::<u32>()` / compare raw strings
(`crates/sandbox_wasm/tests/data_layer_integration.rs:82-96`, `messaging_integration.rs`,
`stream_integration.rs`, `control_plane/src/service.rs:315`). Fully typing the
`result` field is **A1's** job (it owns `dispatch.rs`). A0′ must **not** break
this. New behavior, backward-compatible, built on `val_to_json`:

- empty results → `""` (unchanged).
- single `result<T,E>`: `Err(_)` → transport `Err` (unchanged: WIT err becomes
  a JSON-RPC error); `Ok(None)` → `""`; `Ok(Some(v))` → `stringify(v)`.
- single other `v` → `stringify(v)`.
- multiple results → JSON array string of `val_to_json` for each (previously the
  extras were dropped; no live caller has >1 result — safe improvement).
- `stringify(v)`: if `val_to_json(v)` is `Value::String(s)` → return **raw** `s`
  (keeps greeter/data-layer tests green); else `serde_json::to_string(&json)`
  (replaces the old `{:?}` debug fallback with real JSON). No current test
  depends on the old debug format (all driver funcs return `string` /
  `result<string,_>`), verified by call-site read.

This keeps A0′ self-contained and non-breaking while still routing every result
through the new full converter.

## 6. Named parameters (positional → named switch)

`json_to_wasm_params` takes `&Value` (was `Vec<Value>`) and inspects the shape:

- `Value::Object(map)`: **named** — for each `(name, ty)` from `params_iter`,
  bind `map.get(name)`; if absent and `ty` is `option<_>` → `none`, else error
  (`missing required parameter '<name>'`). Extra keys ignored.
- `Value::Array(arr)`: **positional** — index-align (today's behavior); missing
  trailing positions where `ty` is `option` → `none`, else error.
- `Value::Null`: treated as zero params (error if any non-`option` param).
- other scalar `v`: single-positional (matches engine's current
  `other => vec![other]` normalization) — valid only when there is exactly one
  param; else error.

Because names come straight from `ComponentFunc::params()` (`(&str, Type)`),
this needs no new metadata. This subsumes the old
`Value::Array => clone / other => wrap` normalization that lived in `engine.rs`.

## 7. Call-site changes (all in-scope for A0′)

1. `crates/sandbox_wasm/src/engine.rs:503-508` — delete the `json_params`
   `Array`-or-wrap normalization; call
   `json_to_wasm_params(params_iter, &request.params)?`.
2. `crates/sandbox_wasm/benches/wasm_engine.rs:130-141` — pass a `&Value`
   (`Value::Array(vec![...])`) to the updated signature; add the new round-trip
   bench (§9).
3. No change to `dispatch.rs` (A1), `native.rs` (B0), or any WIT file — A0′ adds
   **no** new WIT types; it only converts existing ones more completely, so
   `wasm32-wasip2` is untouched.

## 8. Tests (`crates/sandbox_wasm/src/conversions.rs` `#[cfg(test)]` + integration)

**8a. `val_to_json` — exhaustive, hand-built `Val`, no component needed.**
One assertion per `Val` variant to exact JSON, plus every §4 lossy edge:
bool; all int widths incl. `u64::MAX`, `i64::MIN`; finite float; **non-finite
float errors**; `char` incl. non-ASCII; string; `list`; `tuple`; `record`
(kebab field names); `variant` with and without payload; `enum`; `option`
some/none; `option<option<_>>` collapse; `result` ok/err with and without
payload; `flags`; `map` (string-key → object, int-key → array). This is the
backbone and pins all encodings deterministically.

*Resource/future/stream/error-context caveat:* these cannot be hand-constructed
(`ResourceAny` etc. have no public constructor outside a live instantiation, and
no repo fixture exports resources), so the "unsupported → error" arms are **not**
unit-tested via a constructed `Val`/`Type`. They are covered by the exhaustive
`match` returning a typed error (compiler-enforced exhaustiveness) and noted in
`status.md`. Do not spend effort trying to fabricate a `Val::Resource`.

**8b. `json_to_val` round-trip — real `Type`s from the prebuilt
`data-layer-test` component** (has record/variant/enum/option/result/list/string/
u32/u64). Gated skip-if-artifact-missing (repo convention,
`data_layer_test_wasm_path()`). Harvest each function's param `Type` via
`get_wasm_func` + `ComponentItem::ComponentFunc::params()`, then assert
`json_to_val(val_to_json(v)?, &ty)? == v` for representative composite values.

**8c. `json_to_val` leaf/width coverage — inline component WAT fixture.**
The scalar/flat types not exercised by data-layer (`s8 u8 s16 u16 s32 s64`,
`float32 float64`, `char`, scalar `tuple`, small `flags`) are **flat** in the
canonical ABI (no linear memory / realloc), so a minimal component-model `.wat`
exporting one function per type — instantiated with the dev-dep `wat` feature —
yields their `Type`s cheaply. Round-trip each. **Validate this WAT approach
first** in implementation; if it proves unexpectedly fragile, fall back to a
`test-components/conversion-test` WIT component (repo convention) or, last
resort, cover those leaves via `val_to_json` only and record the gap in
`status.md` rather than thrash.

**8d. Named-params test** — object payload binds by name; array binds
positionally; missing `option` param → `none`; missing required → error; extra
keys ignored. Uses a harvested `Type` (data-layer or greeter).

**8e. `wasm_results_to_json_string` regression** — string result → raw string;
`result<string,err>::ok` → raw inner; `::err` → `Err`; a non-string result →
proper JSON (guards the removed `{:?}` fallback).

## 9. Benchmark (`criterion`, Performance Budget line "WIT⇄JSON conversion")

In `crates/sandbox_wasm/benches/wasm_engine.rs`:

- fix the existing `json_to_wasm_params` bench to the `&Value` signature;
- add `wit_json_roundtrip`: a representative record `Val` shaped like
  `record-read-value` (`id: string`, `payload: list<u8>` ~256 B, `creator-id:
  string`, `created-at/updated-at: u64`); bench `val_to_json` (no `Type` needed);
  if the `data-layer-test` artifact is present, also bench `json_to_val` with the
  harvested `Type` for the full round-trip.

Record measured numbers in `status.md`; the budget is "must not dominate
same-node call latency" (no hard p99), so we report, not gate.

## 10. Design-note deliverable (Decision Register A.5)

The §3 mapping table + §4 lossy conventions are the "short design note that pins
the lossy-edge JSON encoding conventions." Authoritative copy lives as the
module doc-comment atop `conversions.rs`; this plan and `status.md` reference it.

## 11. Implementation order

1. Empirically validate the inline component WAT harvesting (§8c) with a throwaway
   test — decide fixture strategy before writing conversion code.
2. Write `val_to_json` + exhaustive tests (8a). Pin all encodings first.
3. Write `json_to_val` (Type-directed) + tests (8b, 8c).
4. Rewrite `json_to_wasm_params` (named/positional) over `json_to_val`; update
   `engine.rs` + bench call sites; named-params tests (8d).
5. Rewrite `wasm_results_to_json_string` over `val_to_json`; regression tests (8e).
6. Add the `wit_json_roundtrip` bench (§9).
7. Module doc-comment (§10). Mandatory import-cleanup pass (AGENTS.md rules) on
   every touched file.
8. Gate: `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets
   --all-features` (zero warnings), `cargo test --workspace` (green),
   `cargo build --target wasm32-wasip2 -p …` still builds. Record bench output +
   any deviations in `status.md`.

## 11a. Independent review — findings folded

A fresh-context subagent reviewed this plan. Dispositions:

- **ACCEPTED (GAP): f32 decode finiteness.** `as_f64() → as f32` can silently
  yield `±inf` for a finite-but-out-of-range JSON number, violating §4.4. Fixed:
  §3 table + §4.4 now require a post-cast `is_finite()` re-check on `f32` decode,
  with a dedicated test (`1e40` at `f32` errors).
- **ACCEPTED (RISK): resource/future/stream unsupported-error test is
  impractical.** `ResourceAny` has no public constructor; narrowed §8a — those
  arms are covered by exhaustive `match` + a `status.md` note, not a fabricated
  value.
- **ACCEPTED (minor): variant/flags/enum/result decode edge determinism.** §3 now
  pins extra-key/duplicate/unknown-name handling explicitly.
- **NOTED (minor): `Value::Null → zero params`** is a behavior change but
  unreachable (callers send `{}`, never `null`); kept as defined, harmless.
- **CONFIRMED PASS:** full 26-variant type coverage; `option<option<T>>` collapse
  falls out of the generic recursive rule (not a special-case hack); serde_json
  `Number` stores `u64`/`i64` losslessly; only `engine.rs:508` + the bench call
  `json_to_wasm_params`; boundary-contract preservation verified against
  `dispatch.rs`, all four integration tests, and `control_plane/service.rs`;
  flat-scalar WAT rationale + `wat` dev-dep present; "adds no WIT types" wasm
  claim holds.

Nothing was rejected.

## 12. Risk register

- **Component WAT fragility (§8c)** → validated first (step 1); documented
  fallbacks. Highest-uncertainty item.
- **Hidden dependence on `{:?}` result debug format** → ruled out by call-site
  read; regression test 8e locks it.
- **Named-params breaking a caller** → only `engine.rs` (uses `request.params`,
  handled) and the bench call `json_to_wasm_params`; `execute_wasm` signature is
  unchanged, so its 8 callers are unaffected.
- **`wasm32-wasip2` breakage** → A0′ adds no WIT types; risk ≈ nil, still gated.
