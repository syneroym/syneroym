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

## Slice B0 — Native-Dispatch Authentication Gap Closure ✅ (2026-07-14)

Branch: `feat/m04a-b0`. Requirement `[FND-IAM]` foundation; closes M3→M4 gate
items **#1** (native-dispatch/HTTP-bridge auth gap) and **#4**
(`is_init_context` → Admin UCAN). Blocked on ADRs
[D-04-01](../../../decisions/0015-ucan-capability-model.md) and
[D-04-05](../../../decisions/0016-native-dispatch-identity-threading.md)
(both Accepted). Plan: [plans/B0.md](plans/B0.md).

### What was delivered

1. **New crate `syneroym-ucan`** (`crates/ucan`) — `ResourceUri`, `Ability`
   (with a `data-layer` `admin ⊇ write ⊇ read` tier and `substrate/admin`
   entailing everything, fail-closed for every other ability/pair),
   `Capability`, `SessionContext`. `CapabilityToken`/`issue`/`verify_chain`
   are deferred to B1 per the plan.
2. **`syneroym-rpc`** gains `CallerContext { caller_did, app_instance,
   session, auth }`, `AuthLevel { Delegated, Ucan, LocalElevated }`, and a
   `caller: CallerContext` field on `NativeInvocation`.
   `CallerContext::local_elevated`/`service_system` construct the two
   substrate-injected identities (lifecycle-admin vs. component-acting-as-
   itself). `JsonRpcConverter::json_to_native` takes the caller explicitly.
3. **Router (`crates/router`)**: `HandshakeVerifier::verify_preamble` is now
   *always* attempted in `handle_stream` (`io.rs`) — the old
   `if preamble.delegation.is_some()` gate is gone. A verified handshake (or
   an unverifiable/no-pubkey connection) yields `Option<CallerContext>` via
   the new `build_caller`, threaded through `handle_binary_stream`/
   `handle_http_stream`. `dispatch_json_rpc_once`'s Native-service arm
   rejects `caller: None` *before* looking up/invoking the service; the
   `messaging/subscribe` long-lived-stream arm (which bypasses
   `dispatch_json_rpc_once` entirely) has its own matching `None` guard.
   `RouteHandlerInner` gained `admin_ucan_root: Option<String>` from
   `[iam].admin_ucan_root`; a caller whose DID matches it is granted
   `substrate/admin`.
4. **HTTP bridge (`route_handler/http.rs`)**: `HttpHandler` carries
   `caller: Option<CallerContext>`; the shared `dispatch_native` free
   function rejects `None` and maps it to HTTP 401 via a new reserved
   `-32090` JSON-RPC code in `status_for_rpc_error_code`. The signed-URL
   blob `GET` route (`handle_blob_get`, `blob_download_step`, and
   `BlobDownloadState`'s `Drop`) bypasses `self.dispatch()`/`self.caller`
   entirely and uses an explicit `CallerContext::service_system(..)` for its
   internal `open-download`/`read-chunk`/`close-download` calls, so the HMAC
   signature remains the real authorization for that path regardless of the
   connection's own delegation.
5. **`crates/control_plane`**: `SynSvcNativeService::dispatch_data_layer`'s
   `put`/`batch-mutate` arms now attribute `creator_id` to
   `invocation.caller.app_instance.unwrap_or(caller_did)`, not
   `self.service_id`. `execute-ddl`'s former unconditional deny is replaced
   by a `data-layer/admin` capability check on the caller (returns the same
   `-32010 permission-denied` shape on failure). `ControlPlaneService`'s
   `security` interface threads `invocation.caller` but is deliberately
   **not** gated at B0 (§8.1 of the plan — roymctl holds no admin key);
   `TODO(M04B/FDAE)` marks the deferred gate.
6. **`crates/sandbox_wasm`**: `HostState.is_init_context: bool` replaced by
   `caller: CallerContext`; the guest `execute_ddl` gate now checks
   `data-layer/admin` the same way the native path does. `engine.rs`'s four
   `build_store_and_instantiate` call sites: `prepare_wasm_execution`
   (`init`/`migrate` → `local_elevated`, everything else →
   `service_system`), `invoke_lifecycle_hook` (→ `local_elevated`), and —
   security-critical — `deliver_message`/`open_stream_instance` (→
   `service_system`, **never** elevated, so an inbound broker message or a
   raw-stream instantiation can never pass the Admin gate).
7. **Client-side identity (§0.5 of the plan, added scope)**: mandatory
   verify would otherwise reject every existing internal client (they send
   no pubkey). `SyneroymClient` (`crates/sdk`) gains an `identity:
   syneroym_identity::Identity` field — `new`/`new_with_mechanisms` generate
   an ephemeral one, `new_with_identity` accepts a stable one — and sets a
   self-asserted `pubkey` on every outbound preamble
   (`open_request_stream`, `passthrough`/`passthrough_with_conn`).
   `client_gateway` loads (or generates+persists, whichever component boots
   first) the node's own identity from `config.identity.key` (same path
   `syneroym_substrate::identity::setup_substrate_identity` uses) and
   presents it as every downstream `SyneroymClient`'s identity — the
   owner→node delegation needed to present the *substrate-owner* DID instead
   is deferred (`TODO(post-B0)` at the gateway's client-construction site,
   per the plan's §0.5.1). `roymctl` needed no changes: it already
   constructs plain `SyneroymClient::new(..)`, which now self-asserts.
8. **Cross-node proxy-hop seam (§9.5 of the plan, design-only)**:
   `CallerContext`'s doc comment states it is always locally constructed and
   never wire-serialized; a future cross-node hop (A1) carries the caller's
   DID and signed proofs in the envelope, re-verified at the destination.

### Tests

New `crates/router/tests/native_dispatch_identity.rs` (5 tests) — "the
single most important test in this milestone" (task.md Tests Summary):
- `anonymous_caller_rejected_before_native_dispatch_for_every_interface` —
  drives `dispatch_json_rpc_once` with `caller: None` against each of the 5
  native-capability interfaces (`data-layer`/`vault`/`app-config`/
  `blob-store`/`messaging`) and asserts both an `Err` *and* that a recording
  `NativeService` double was never invoked (rejection happens before
  dispatch, not just an error envelope after).
- `authenticated_caller_reaches_native_dispatch` — the positive control:
  the same double *is* invoked for a `Some(caller)` request.
- `authenticated_caller_identity_becomes_creator_id_not_service_id` — a real
  `SynSvcNativeService`, `create-collection` → `put` → `get`, asserts the
  stored `creator_id` equals the caller's DID, not the service's own id.
- `http_bridge_rejects_anonymous_caller_with_401` — a real `hyper` request
  over an in-memory `tokio::io::duplex` into `handle_http_stream` with
  `caller: None`, asserting the raw HTTP response starts `HTTP/1.1 401`.
- `messaging_subscribe_rejected_for_anonymous_caller` — the long-lived
  `handle_binary_stream` special-case gets its own gate check, verified via
  a framed JSON-RPC error response (not a `"subscribed"` ack).

Plus: 12 new unit tests in `syneroym-ucan` (entailment fail-closed in both
directions, `Capability::grants`, `SessionContext::has_capability`); a new
positive DDL test (`test_execute_ddl_allowed_for_local_elevated_lifecycle_context`,
`crates/sandbox_wasm/tests/lifecycle_hooks.rs`) alongside the existing
denial test (updated from the `is_init_context` bool to a `service_system`
caller). `test_security_dispatch_returns_sdk_statuses`
(`crates/control_plane/src/service.rs`) still passes unmodified with a
non-admin test caller, proving §8.1's "threaded but not gated" claim.

### Regression found and fixed

`crates/substrate/tests/http_passthrough_e2e.rs`'s `open_http_stream` helper
hand-built its own `RoutePreamble` with `pubkey: None` (bypassing the SDK's
`open_request_stream`, which the fix in item 7 above doesn't touch). Once
`verify_preamble` became mandatory, every bridged native route in that file
started 401ing. Fixed by generating a fresh ephemeral `Identity` per stream
and setting `pubkey` on the hand-built preamble, mirroring what the SDK now
does — found via the full `cargo test --workspace` regression pass, not
code inspection.

### Gate

- `cargo +nightly fmt --all` — clean.
- `cargo clippy --workspace --all-targets --all-features` — zero warnings.
- `cargo test --workspace` — **376 passed, 0 failed** across 71 test
  binaries (full run, sandbox disabled — see environment note below).
- `mise run test:e2e` — **12 passed, 0 failed** (8 + 4 across the two
  Playwright configs); confirms zero regression in the browser-driven
  WebRTC/blind-tunnel flows now that the gateway/SDK identity changes are in
  place.
- `wasm32-wasip2` — `test-components/data-layer-test` builds; B0 adds no WIT
  types, so the guest surface is unchanged (only `sandbox_wasm` host code
  and `syneroym-rpc`/`syneroym-ucan` changed, neither wasip2-compiled).
- `system-architecture.md:1892`'s interim-security-posture note updated to
  record the gap as closed.

**Environment notes:**
- Under the agent command sandbox, `syneroym-coordinator-iroh`'s
  `connection_limit` test fails to bind a loopback socket ("Operation not
  permitted") — the same pre-existing, unrelated limitation A0′ documented.
  The 376/0 figure above is from the full suite run with the sandbox
  disabled.
- One latency-budget test (`sandbox_wasm`'s
  `test_guest_delivery_latency_budget`, p99 < some ms) failed once under
  heavy parallel-build CPU contention (multiple `cargo` invocations
  overlapping) and passed cleanly in isolation immediately after — a system-
  load flake, not a regression; not caused by any B0 change (its own hot
  path, `deliver_message`, only gained a `CallerContext::service_system`
  construction, a couple of cheap `String` allocations).
- The host's disk filled to ~100% mid-session (`target/debug/incremental`
  alone was 82G, accumulated across the wider session's builds, not solely
  this task's); the user cleared space before the final gate run. Unrelated
  to this slice's code changes, noted here only because it interrupted the
  first `cargo build --workspace --all-targets --all-features` attempt.

### Scope discipline

Every change maps to a plan section (§0.5 client identity, §1–§9.5). No B1
(`CapabilityToken`/`issue`/`verify_chain`), A1 (Universal Proxy/cross-node
envelope), B4/B5/B6, or FDAE (M04B) work was started. No WIT file changed.
`query-raw`/full `AggregationPipeline` remain out of scope (B4/B5). The
owner→node delegation for presenting the substrate-owner DID at the gateway
is explicitly deferred (`TODO(post-B0)`), per the plan's §0.5.1.

### Post-commit addendum (2026-07-14) — `admin_ucan_root` unified with `ControllerAgreement`

Design discussion surfaced that B0's `[iam].admin_ucan_root` (a plain config
string) and the pre-existing, cryptographically two-way-signed
`ControllerAgreement`/`SubstrateIdentityState` mechanism
(`crates/identity/src/substrate.rs`, wired at boot in
`crates/substrate/src/identity.rs`) were two independent, disconnected
notions of "who owns this substrate" — the latter was computed at boot and
then discarded (only `.did` was kept; `.controller`/`.status` went unused).

Fixed in `crates/substrate/src/runtime.rs`
(`setup_identity_and_storage`/`setup_connection_router`): a verified
(`SubstrateIdentityStatus::Verified`, i.e. both the substrate and the
controller signed) `ControllerAgreement` controller now overrides
`admin_ucan_root` before it reaches `RouteHandler::init`. `Unverified`/`None`
never grant `substrate/admin`. The raw config value remains only as a
fallback for deployments with no agreement configured at all — doc comment
updated on `IamConfig` (`crates/core/src/config.rs`).

Verified: `cargo build`/`clippy` clean on `syneroym-substrate`/`syneroym-core`;
`native_dispatch_identity` (8/8), `lifecycle_hooks` (4/4), and
`basic_lifecycle` (3/3, sandbox disabled) all pass unchanged — the tests that
set `admin_ucan_root` directly bypass this boot path entirely, so the
fallback behavior they exercise is untouched.

**Explicitly out of scope for this addendum** (see new Slice B7 below):
service-level ownership (deploy/undeploy/status-check permission grants),
app-catalog owner attribution, and registry-publish-on-behalf-of-owner.

## Slice A1 — Universal Proxy Dispatch (JSON-RPC transport) ✅ (2026-07-15)

Branch: `feat/m04a-a1`. Requirement `[PLT-DAT]` (Universal Proxy) + the minimal
`[LFC-VER]` typed-unsupported-protocol error kept from the deferred A2.
Depends on A0′ (done) and B0's `NativeInvocation.caller`/`CallerContext`
(done). Plan: [plans/A1.md](plans/A1.md).

### What was delivered

1. **`syneroym-rpc` proxy contract** (`crates/rpc/src/proxy.rs`, new): the
   transport-agnostic `ServiceProxy` trait (`async fn invoke(ProxyRequest) ->
   Result<Value, ProxyError>`), `ProxyProtocol` (reserved single-variant enum,
   `JsonRpcV1`), `CallOrigin` (`Guest{service_id}` / `Native`), `ProxyRequest`,
   and `ProxyError` with reserved JSON-RPC codes (`-32091` unsupported
   protocol, `-32092` transport, `-32093` unsupported target). `CallerContext`
   gains `proof: Option<CallerProof>` (hex pubkey + optional delegation JSON)
   — the mechanism a cross-node hop uses to forward the caller's signed
   identity without ever putting capabilities on the wire (ADR-0016 §6).
2. **Typed WASM results** (`crates/sandbox_wasm/src/conversions.rs`,
   `engine.rs`): `wasm_results_to_json` (Slice A1's typed counterpart to
   A0′'s `wasm_results_to_json_string`), and `AppSandboxEngine::execute_wasm_vals`
   factored out so both `execute_wasm` (string, unchanged) and the new
   `execute_wasm_json` share the call/quota/trap-mapping logic. The inbound
   `(JsonRpcToWasm, WasmComponent)` route (`route_handler/dispatch.rs`) now
   returns real typed JSON instead of double-encoding non-string results —
   confirmed inert for every existing test component (all return plain
   strings) by the full test-suite pass below.
3. **Typed unsupported-protocol error** (`routing.rs`, `dispatch.rs`,
   `http.rs`): new `ServiceStage::UnsupportedProtocol`; `plan_pipeline`
   routes `RouteProtocol::Wrpc`/`Other(_)` there instead of into the
   ADR-0014 raw-stream path (which produced a confusing "missing dir="
   error — Flag F2); `dispatch_json_rpc_once` answers with `-32091` and the
   node's actual spoken protocol (`json-rpc/v1`); `http.rs` maps `-32091`/
   `-32093` to HTTP 501 and `-32092` to 502. Dead `(RouteProtocol::Wrpc,
   WasmChannel)` `plan_pipeline` arm and its matching transport-override
   block removed (F1/F2) — the `AdaptationStage::JsonRpcToWrpc` variant and
   its `dispatch_json_rpc_once` guard arm stay reserved for A.5.
4. **Outbound Iroh endpoint** (`connection_router.rs`, `route_handler.rs`,
   fixes Flag F7): `ConnectionRouter::init` now builds the Iroh `Endpoint`
   *before* `RouteHandler::init` (previously built inside `init_iroh`, after)
   so `RouteHandler::init` can hand it to the `ProxyRouter`'s `IrohHop`. Side
   effect (intended, per F7): the registry-miss relay-forwarding path in
   `io.rs` — which reads `self.inner.iroh_endpoint` — now has a real endpoint
   on a substrate node for the first time (`RouteHandlerInner.iroh_endpoint`
   was hardcoded `None` pre-A1). `net_iroh::resolve_iroh_addr` factors the
   registry/DHT address-resolution block out of `io.rs` so `ProxyRouter`'s
   remote hop shares the exact same lookup logic.
5. **`NATIVE_CAPABILITY_INTERFACES` consolidated** into
   `syneroym_core::local_registry` (was three independently-maintained
   copies — `control_plane`'s deploy-time registration list, `router`'s own
   test copy, and now needed by the new guest proxy gate too).
6. **`ProxyRouter`** (`crates/router/src/proxy.rs`, new) — the only
   `ServiceProxy` implementation: `invoke` gates on protocol (reserved, F8
   no-op today) then the guest native-capability gate, then dispatches
   local-first (`registry.lookup` hit → native `NativeService::dispatch` or
   WASM `execute_wasm_json`) or falls to `invoke_remote` (resolve via
   `net_iroh::resolve_iroh_addr` → `RemoteHop::call`, retrying only
   *transport* failures and only when `idempotent`, backoff via
   `syneroym_core::retry::calculate_jittered_backoff`, never retrying a
   definitive `Callee` error). `RemoteHop`/`IrohHop` is the transport-
   agnostic seam a future wRPC wire slots into (A.5) — `IrohHop::new` forces
   its internal `connect_with_retry` to a single attempt so the outer
   call-level retry loop is the only source of backoff (documented
   `max_attempts²` risk this avoids). The guest native-capability gate
   (`check_native_capability_gate`) is scoped to `CallOrigin::Guest` only —
   `CallOrigin::Native` (M04B's B3 relationship-proof fetch) is explicitly
   exempted, with a regression test pinning that shape as allowed.
   `RouteHandlerInner.identity`/`.registry_client` are now `Arc`-wrapped (a
   deviation the plan didn't call out explicitly) so the `ProxyRouter` can
   share the exact same `Identity`/`RegistryClient` instances rather than
   constructing second ones — re-constructing a second `RegistryClient`
   would spin up a second DHT client (background bootstrap tasks + sockets)
   when DHT is enabled.
7. **`syneroym:proxy@0.1.0` WIT package** (`crates/wit_interfaces/wit/proxy/`):
   `call(service, %interface, method, params, options) ->
   result<string, proxy-error>` (the WIT keyword `interface` needed the `%`
   escape). Wired into `host-environment`'s imports and
   `AppSandboxEngine::build_wasm_linker`.
8. **Guest host function** (`sandbox_wasm/src/host_capabilities.rs`):
   `impl proxy::Host for HostState` parses `params` as JSON, maps
   `call-options` to a `ProxyRequest` with `caller:
   CallerContext::service_system(component_id)` and **always**
   `origin: CallOrigin::Guest{..}` (the only construction site reachable from
   guest code, so the capability gate cannot be bypassed), and maps
   `syneroym_rpc::ProxyError` onto the WIT `proxy-error` variant.
   `AppSandboxEngine` gains `service_proxy: OnceLock<Weak<dyn ServiceProxy>>`
   (mirrors `self_weak`); `HostState` gains a `service_proxy: Weak<dyn
   ServiceProxy>` field threaded through all 16 `HostState::new` call sites
   (14 in `sandbox_wasm`'s own tests/benches, 2 more found in `tests/perf`
   that the plan's own call-site count had missed). `Weak<dyn ServiceProxy>`
   cannot use the inherent `Weak::new()` (that's `T: Sized`-only), so a
   small always-empty helper (`syneroym_sandbox_wasm::empty_service_proxy`,
   via unsized coercion from a never-instantiated marker type) replaces
   13 bare call-site constructions.
9. **Composition-root wiring** (`route_handler.rs`): `ProxyRouter` is built
   inside `RouteHandler::init`, after `iroh_endpoint` exists, using `Weak`
   downgrades of `deps.native_dispatch`/`deps.app_sandbox_engine` (still
   owned by `deps` at that point); its `Weak<dyn ServiceProxy>` is published
   into `AppSandboxEngine::service_proxy` before `deps.app_sandbox_engine` is
   moved into `RouteHandlerInner`. `RouteHandlerInner` gains `_proxy:
   Option<Arc<ProxyRouter>>` — the strong owner (underscore-prefixed per this
   struct's existing `_parent_relay_url` convention: not read anywhere yet,
   A1 only wires the *outbound* call surface, so the field's job is solely to
   keep the router alive). `None` in coordinator mode.

### Flags resolved (plan.md §1)

- **F1/F2** — the `dispatch.rs:122-123` "stub" anchor was a mis-anchor; the
  real dead arm was `plan_pipeline`'s `(Wrpc, WasmChannel)` combination,
  deleted along with its transport-override block. Fixed via item 3 above.
- **F3** — confirmed by code read: `HandshakeVerifier::verify_preamble` never
  compares the cert against `preamble.service_id`; the failure-tests row in
  `task.md` describing that is inaccurate. Not "fixed" (A1 doesn't add a
  callee-binding check — that's a B1/UCAN concern) but flagged in this
  status entry per the plan's recommendation; `task.md`'s row is corrected
  below.
- **F4** — `TcpHostPort`/`PodmanSocket` proxy targets return
  `ProxyError::UnsupportedTarget` (`-32093`) rather than being silently
  unreachable; `task.md`'s Goal wording is corrected below to note
  TCP/Podman JSON-RPC proxy targets are deferred.
- **F5** — `syneroym:proxy@0.1.0` added (item 7); `task.md`'s Migration
  Strategy WIT list is corrected below.
- **F6** — this slice delivers the *routing/identity/retry* substance of the
  Universal Proxy via an explicit `syneroym:proxy/proxy::call` import, not
  WIT-import interception/late binding (`system-architecture.md:1930`'s
  vision). Recorded explicitly there (doc update below); late binding is
  unstarted, not silently "done".
- **F7** — fixed via item 4 above.
- **F8** — interpreted as in-process local dispatch, per the plan's own
  recommendation; benchmarked as such (see Performance below).
- **F9** — stale anchors in `task.md`'s Current State Inventory refreshed as
  part of the exit-criteria edits below.
- **F10** — confirmed: `coordinator_iroh/tests/multi_hop_relay.rs` already
  runs two full substrate nodes in one process with
  `enable_bep0044_dht = false`; the cross-node proxy test
  (`test_cross_node_proxy_call`) was added there rather than via Playwright.
  It needed **no coordinator/relay infrastructure at all** — a discovery
  made while implementing it, one step simpler than the plan's own
  characterization: two direct-address-only Iroh endpoints (no relay) plus a
  lightweight HTTP `EcosystemRegistry` (no DHT) are sufficient for the
  `ProxyRouter`'s remote hop to resolve and connect. Two non-obvious fixes
  were needed along the way, recorded here since they're easy to
  rediscover-the-hard-way: (a) `Endpoint::online()` waits for *both* a relay
  connection *and* a local address — with no relay configured it never
  resolves, so the test polls `Endpoint::addr()` directly instead
  (`wait_for_local_addr`); (b) the existing `create_signed_info` helper in
  that file deliberately prunes an `EndpointAddr` down to a bare
  `EndpointId` (fine for its own tests, which reconnect via a relay URL
  alongside the pruned id) — a relay-less direct-connect test needs the real
  addresses preserved, so a second helper
  (`create_signed_info_with_full_addr`) was added rather than changing the
  first one's behavior for its existing callers.
- **F11** — the guest gate is scoped to `CallOrigin::Guest`; a
  `CallOrigin::Native` case is pinned as allowed by a dedicated regression
  test (`native_origin_cross_service_data_layer_call_is_allowed_by_the_gate`,
  `crates/router/src/proxy.rs`).

### Deviations from the plan (recorded, not silent)

- **`RouteHandlerInner.identity`/`.registry_client` became `Arc`-wrapped.**
  The plan's §10 pseudocode passed owned `Identity`/`RegistryClient` values
  into `ProxyRouter::new`, but neither type implements `Clone` (`Identity`
  wraps a zeroizing secret key — deliberately not `Clone`-derived), and
  `RouteHandlerInner` already owns exactly one of each. Re-constructing a
  second `RegistryClient` from the same config would spin up a second DHT
  `mainline::Client` (background bootstrap/routing-table tasks and sockets)
  when DHT is enabled — wasteful and not something either type's
  constructor should be called twice for. `Arc`-wrapping both fields lets
  `RouteHandlerInner` and `ProxyRouter` share the exact same instances;
  every existing by-reference call site (`&self.inner.identity`,
  `&self.inner.registry_client`) still compiles unchanged via deref
  coercion, with one exception (`HandshakeVerifier::verify_preamble`'s
  trait-object parameter) that needed an explicit `.as_ref()`.
- **B0 plan §9.5's "A1 does not modify `CallerContext`"** — `proof` is added
  anyway, per A1's own plan.md §3.1, which explicitly reconciles this: the
  §9.5 sentence's intent was "don't put capabilities on the wire," and
  `proof` is the mechanism that sentence itself mandates for forwarding
  identity across a hop.
- **Identity threading through a proxied WASM call is "the callee acts as
  itself,"** not the original caller's identity — `execute_wasm_json` /
  `prepare_wasm_execution` builds the callee's `CallerContext` internally
  (`service_system`/`local_elevated`), so a WASM callee never sees the
  proxy caller's identity. This is B0's existing shape, unchanged by A1, and
  explicitly not a caller-scoped identity gap to fix here — that's an
  FDAE/M04B concern. (Native callees *do* receive the exact forwarded
  `req.caller`, unchanged from before.)

### Tests

- **Unit** — `crates/rpc/src/proxy.rs` (3): `ProxyProtocol::parse`
  none/reserved-tag/unknown-tag, `ProxyError::code()` mapping table.
  `crates/sandbox_wasm/src/conversions.rs` (+1, 19 total in that module):
  `wasm_results_to_json_contract` (empty/`Result::Ok`/`Result::Err`/scalar/
  string/multi-value, contrasted against the unchanged `_to_json_string`
  raw-string boundary).
  `crates/router/src/proxy.rs` (12, new module): local-native dispatch with
  caller-identity threading; unknown-service → `ServiceNotFound` with the
  hop never called; the guest capability gate's four cases (cross-service
  denied + never dispatched, same-service allowed — the regression case a
  `caller_did`-based check would have wrongly rejected — non-native
  interface allowed, `CallOrigin::Native` allowed); idempotent-retries-up-
  to-max / non-idempotent-never-retries / callee-error-never-retries /
  retry-then-succeeds; proof-forwarded-verbatim / no-proof-uses-node-identity.
- **Integration** — `crates/router/tests/proxy_dispatch.rs` (2, new):
  guest-to-guest same-node proxy call returns the callee's typed result;
  guest reaching another service's `data-layer` through the proxy is denied
  as a WIT `proxy-error` (the A0′ `result::err` → transport-error boundary
  contract means this surfaces as a JSON-RPC `error.message`, not a
  `result` string — asserted accordingly).
  `crates/router/tests/unsupported_protocol.rs` (2, new): `wrpc://` and an
  arbitrary custom scheme both yield the reserved `-32091` code with a
  message naming `json-rpc/v1`.
- **E2E / cross-node** —
  `crates/coordinator_iroh/tests/multi_hop_relay.rs::test_cross_node_proxy_call`
  (new): two full substrate nodes, no coordinator/relay, a `proxy-test`
  guest component on Sx calls `greeter` deployed on Sz across a real Iroh
  QUIC connection resolved via a live HTTP community registry — asserts the
  correct typed greeting comes back. Exercises §6's endpoint fix, §5.5's
  `IrohHop`, and proof/identity forwarding together; the guest-originated
  call can only reach Sz's WASM component (not a native capability, by the
  gate's own design), so router-level caller-verification for a *native*-
  origin cross-node hop is not separately asserted here — B3 (M04B) will
  get dedicated coverage for that when it lands.
- **New test component** — `test-components/proxy-test/` (mirrors
  `test-components/stream-test/`): imports `syneroym:proxy/proxy@0.1.0`,
  exports a `test-driver::call-peer` that forwards to `proxy::call`. Builds
  clean for `wasm32-wasip2`.

### Performance (criterion, `--bench proxy`, `--quick`)

| Bench | Measured | Budget |
|---|---|---|
| `proxy_local_native` (`ProxyRouter::invoke` → in-memory `NativeService`) | ~619 ns | < 5 ms p99 (F8: same-node = in-process) |
| `proxy_local_wasm` (→ cached `greeter` component, full WIT⇄JSON both ways) | ~34.6 µs | < 5 ms p99 |

Both several orders of magnitude under budget. Remote-hop latency needs two
live nodes and is not benched (per plan.md §12) — the cross-node e2e test
above is the evidence that the remote path works, not a latency number.

### Gate

- `cargo +nightly fmt --all` — clean.
- `cargo clippy --workspace --all-targets --all-features` — zero warnings.
- `cargo test --workspace` — **402 passed, 0 failed** across 73 test binaries
  (full run, sandbox disabled — see environment note below). Includes all of
  this slice's new tests: `syneroym-rpc`'s `proxy` unit tests (3),
  `sandbox_wasm`'s `wasm_results_to_json_contract` (1),
  `syneroym-router`'s `proxy` module (12), `proxy_dispatch.rs` (2),
  `unsupported_protocol.rs` (2), and `coordinator_iroh`'s
  `test_cross_node_proxy_call`.
- `wasm32-wasip2` — `test-components/proxy-test` builds clean (validates the
  new `syneroym:proxy` WIT package end to end on the guest side);
  `test-components/data-layer-test`/`greeter` unaffected.
- `mise run test:e2e` — **12 passed, 0 failed** (8 + 4 across the two
  Playwright configs) — matches B0's own baseline exactly; zero regression
  from the typed-inbound-WASM-result switch (`execute_wasm_json`) or the new
  `syneroym:proxy` linker import.

**Environment notes:**
- Under the agent command sandbox, the same pre-existing network-binding
  limitations A0′/B0 documented recur here for new tests that bind real
  sockets (`test_cross_node_proxy_call`'s local `EcosystemRegistry` HTTP
  listener, `wasm32-wasip2` component builds writing to the shared cargo
  registry cache) — all runs reported in this section used the sandbox
  disabled, consistent with A0′/B0's own gate methodology.
- `Endpoint::online()` hanging without a configured relay (see F10) cost one
  full debugging cycle before the root cause was found via the iroh docs;
  recorded above so a future relay-less Iroh test doesn't rediscover it.

### Scope discipline

Only Slice A1 was touched, per plan.md's execution order (§14): `syneroym-rpc`
proxy contract, `sandbox_wasm` typed results + guest host function,
`router`'s `ProxyRouter`/endpoint plumbing/typed-protocol-error, the new
`syneroym:proxy` WIT package, and the new/extended test files listed above.
No M04B (FDAE) work, no B1 (`CapabilityToken`/UCAN chains — `CallerProof`
carries only the delegation half per the plan's own TODO), no B4/B5/B6. The
`AdaptationStage::JsonRpcToWrpc` variant and `wrpc://`/`RouteProtocol::Wrpc`
scheme stay reserved, unimplemented (A.5) — only the *unsupported-protocol
error path* for them was added, not a wire.
