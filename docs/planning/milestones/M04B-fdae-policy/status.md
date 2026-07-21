# M04B Status

## Slice B2 — Local FDAE (SQL Pushdown Sieve)

### Phase 1 — Policy model & ReBAC→SQL compiler ✅ (2026-07-20, PR #86)

Branch: `feat/m04b-slice-b2-fdae-sieve`. New `crates/fdae` (`syneroym-fdae`):
the typed `Policy` model (`policy.rs`, JSON-Schema-validated against the
embedded `fdae-v1.json`) and the `compile_read` ReBAC→SQL compiler
(`compile.rs`) producing a `CompiledSieve` (`where_clause` + bound `params` +
`masked_fields` + `where_caveats`) for Mode A (Point-In-Time) and Mode B
(Filter). Covers the `WHERE EXISTS`/fused `WITH RECURSIVE` path compilation,
the `visited_track`-equivalent cycle guard (`instr(...)` path-seen check +
`MAX_RECURSION_DEPTH` depth backstop), the grant∩policy intersection
(`applicable_permissions`/`close_over_includes`), default-deny (D-04-02-b),
and CLS mask-list derivation from `deny`-lists. `crates/data_db` is
untouched by Phase 1 — nothing in `data_db` calls `compile_read` yet.

Not part of Phase 1 (deferred, tracked below): threading the compiler into
`data_db`'s actual read/delete paths, the watchdog *installation* (the
compiler only documents where it belongs), and the decision trace.

## Phase 2 — `data_db` Integration ✅ (2026-07-20)

Branch: `feat/m04b-slice-b2-data-db`. Plan:
[slice-b2-phase2-data-db-plan.md](slice-b2-phase2-data-db-plan.md). `crates/fdae`
is unchanged in this phase (treated as ground truth per the plan).

### What was delivered

- **New `crates/data_db/src/auth.rs`** — `QueryAuth<'a> { policy, session,
  service_id }` (per-request policy + caller context; `None` at a call site
  preserves today's unfiltered behavior) and `ReadOutcome<T> { value,
  masked_fields }` (the CLS field-mask a read result carries out, since
  `QueryResult`/`RecordReadValue` are WIT-generated types that can't carry a
  host-only field).
- **`ServiceStore` trait (`traits.rs`)** — `get`/`query`/`aggregate`/
  `delete_many` gained an `auth: Option<&QueryAuth<'_>>` parameter; `get`/
  `query` now return `ReadOutcome<T>`; new `check_access` (Mode A
  point-in-time primitive, fail-closed to `Ok(false)`).
- **`sqlite.rs`**:
  - `merge_sieve` ANDs the compiled RLS `where_clause` with each entitling
    capability's caveat `where` (re-compiled via the existing
    `filter::compile_filter`) — dropping `where_caveats` would silently widen
    access beyond what a caveat restricts (the Phase-1 "dropped-caveat" bug
    class).
  - `do_query` — sieve spliced in **first**, ahead of the caller's own JSON
    filter/cursor; final bound-param order is `[sieve…, filter…, cursor,
    limit]`, matching the assembled clause text order.
  - `do_get` — Mode A wrap: `compile_read(..., Mode::PointInTime{id})` already
    ANDs `{table}.id = ?` onto the RLS, so a sieved `get` is one
    self-contained `WHERE` (no separate `id = ?1` alongside it, which would
    double-bind).
  - `check_access`/`do_check_access` — new Mode A primitive. Fail-closed
    (`Ok(false)`) on a `PolicyError`, a caveat-compile error, a watchdog-install
    failure, or a watchdog interrupt; `auth = None` falls back to a plain
    existence check (D3, resolved: no-policy ⇒ existence, not unconditional
    `true`).
  - `delete_many`/`do_delete_many` — sieve compiled as `Mode::Filter` with
    operation `data-layer/write` (D2, resolved: deleting is a write, so a
    read-only permission's `paths` must not become "these rows are
    deletable"), applied on the writer connection (`DbCommand::DeleteMany`
    gained an owned `sieve: Option<CompiledSieve>` field).
  - `do_aggregate`/`aggregate::compile` — the RLS sieve injects into the
    inner query's `WHERE` (param order: `group.params ++ sieve_params ++
    match_params ++ having_params ++ limit_params`); a CLS-active sieve
    (`masked_fields` non-empty) fails the **whole aggregate** closed
    (`PermissionDenied`) rather than attempting a CLS-safe aggregation — an
    aggregate accumulator can leak a masked field's value without ever
    projecting the raw column.
  - Watchdog matrix — `install_watchdog`/`ProgressGuard` (progress-only,
    clear-on-drop, unlike `QueryRawGuard` which also clears an authorizer the
    sieve paths never install), aliasing `FDAE_MAX_VM_OPS =
    QUERY_RAW_MAX_VM_OPS` as the interim hard-coded budget. Installed **only**
    when a sieve is present (the policy-absent path is byte-for-byte
    unchanged). An interrupt maps to `QuotaExceeded` for `do_query`/`do_get`/
    `do_delete_many` (Mode B) and to `Ok(false)` for `do_check_access` (Mode
    A).
- **All four existing call sites** thread `auth = None`, preserving today's
  behavior exactly: `sandbox_wasm/src/host_capabilities.rs` (WASM guest
  dispatch — real `QueryAuth` construction from `HostState`'s policy/session
  is Phase 3), `control_plane/src/synsvc_native.rs` (native dispatch),
  `data_db/benches/security_config_bench.rs` and
  `sandbox_wasm/benches/data_layer_bench.rs`, and `data_db/src/tests_crud.rs`
  (25 call sites across `get`/`query`/`aggregate`/`delete_many`).

### Tests

- **`crates/data_db/src/tests_fdae.rs`** (new, 11 integration tests, real SQL
  against seeded rows through the `ServiceStore` trait with a real compiled
  `Policy` + hand-built `SessionContext`s):
  - Mode B excludes an unreachable row (empty result, not an error).
  - Mode A `check_access` allows the reachable row, denies the unreachable
    one; a no-`auth` call is a plain existence check (D3).
  - `get` of an unreachable-but-existing row returns `None` (ADR-0007 "no
    result is a valid outcome"), not an error.
  - `aggregate` is row-filtered identically to `query`, and denied outright
    when CLS is active.
  - `masked_fields` is exposed on `ReadOutcome` for a CLS policy, but the row
    itself is still unmasked in Phase 2 (the strip is Phase 3 — pinned
    explicitly so a passing test isn't misread as satisfying the task.md CLS
    row).
  - `delete_many` is filtered as a **write** operation: a read-only capability
    deletes nothing; a write capability deletes only the caller's own row.
  - Binding order: sieve ∧ a capability's caveat `where` ∧ the caller's own
    JSON filter, together, return the correct row.
  - A policy-declared relation target whose physical table was never created
    fails closed — Mode B surfaces an error (not an empty-but-successful
    leak), Mode A returns `Ok(false)` (never `Ok(true)`).
  - `auth` present but the policy names no definition for the collection
    stays unfiltered (not strict) — the `compile_read` `Ok(None)` branch.
- **`crates/data_db/src/sqlite.rs`** (new, 4 unit tests in the existing
  private `tests` module): a hand-built pathological `CompiledSieve` (mirrors
  `tests_crud.rs`'s existing `test_query_raw_bounds_compute_independent_of_
  row_count` trick, since `crates/fdae`'s own `MAX_RECURSION_DEPTH` caps any
  *policy-compiled* recursive relation at 64 steps — far too cheap to ever
  approach `FDAE_MAX_VM_OPS`; what's under test here is `data_db`'s own
  watchdog wiring, not the compiler) confirms a watchdog interrupt maps to
  `QuotaExceeded` for `do_query`/`do_get`/`do_delete_many` and to `Ok(false)`
  for `do_check_access`, and that the connection remains fully usable
  afterward (the guard cleared on drop).
- `tests_crud.rs`/benches — call sites updated only, no new assertions; all
  102 pre-existing tests stay green on the `auth = None` path, proving zero
  behavior change.

### Verification evidence

- `cargo +nightly fmt --all` — clean.
- `cargo clippy --workspace --all-targets --all-features` — zero warnings.
- `cargo test -p syneroym-data-db` — **117 passed, 0 failed** (102
  pre-existing + 11 new FDAE integration + 4 new watchdog unit).
- `cargo test --workspace` — all crates green. (One test,
  `syneroym-coordinator-iroh`'s `connection_limit::
  accepts_up_to_cap_and_rejects_the_rest`, fails under this CLI's default
  network sandbox — "Operation not permitted" binding a UDP relay socket to
  `127.0.0.1:0` — pre-existing/environmental, unrelated to this change;
  confirmed by rerunning the full workspace suite with the sandbox disabled,
  which passed with zero failures.)
- `mise run test:e2e` — not run. Phase 2 has no WIT-boundary or
  guest-visible behavior change (`check-access`'s WIT export is Phase 3; the
  four wired call sites all pass `auth = None`, byte-for-byte identical to
  pre-Phase-2 behavior), so there is nothing new for the Playwright e2e
  suite to exercise. A deliberate skip, not an oversight — recorded per the
  plan's own scoping (§13).

### Explicitly out of Phase 2 scope (plan §12, items 8–10 — recorded, not silently dropped)

- **Decision trace** — deferred to Phase 5 (would require an `fdae` API
  change to surface a `DecisionTrace` alongside `CompiledSieve`, reopening
  Phase 1).
- **Policy/substrate-configurable watchdog budget** — `FDAE_MAX_VM_OPS` is
  the interim hard-coded constant; real configurability needs an `fdae`
  schema change (a `budget` field) plus Phase 4 substrate-config plumbing.
- **Write-side Tier-3 enforcement** (single-row `put`/`patch`/`delete`
  authorization) — scheduled as Slice B5-fdae, gated on sub-decision
  D-04-02-f (creation authorization). `delete_many` is filtered by this
  phase; single-row `delete`/`patch` are not.
- **CLS field-stripping** — Phase 3 lands the host-side final projection
  (above the stage-4 hook, below the WIT response). Phase 2's job is only to
  surface `masked_fields` out of the store via `ReadOutcome`; task.md's CLS
  Failure/Security row stays open until Phase 3.

### Post-commit review (2026-07-21) — two independent passes

Two reviews came back against commit `14d318a`. Both independently re-ran
`cargo test -p syneroym-data-db` (117/117 green) and clippy on the touched
crates before reviewing, rather than trusting this file's self-report; both
concluded no SQL-injection or auth-bypass (privilege-widening) defect exists
— every merge path they traced fails toward over-restriction, never a leak.

**Addressed, code changed (this session, still Phase 2 scope):**

- **`do_aggregate` compiled caveat filters before checking CLS denial**
  (low severity) — `merge_sieve` ran (and could itself fail/propagate) ahead
  of the `masked_fields.is_empty()` check that unconditionally denies a
  CLS-active aggregate. Fixed: the CLS check now runs first, so a CLS-active
  call is denied immediately without compiling its caveats at all, and a
  malformed caveat on a CLS-masked collection can no longer surface as a
  generic `Err` instead of `PermissionDenied`.
- **Plan §11's "adversarial `subject_did`/caveat bound not interpolated"
  data_db end-to-end row was missing** (medium severity) — added
  `tests_fdae.rs::adversarial_subject_did_and_caveat_value_are_bound_not_interpolated`:
  an attacker-controlled `subject_did` (`"attacker' OR '1'='1"`) and a
  caveat `where` value containing `DROP TABLE`/comment syntax, exercised
  through both `query` (Mode B) and `check_access` (Mode A, a real
  parameterized `PointInTime` sieve with bound `id`/`subject_did` params —
  this also directly answers Reviewer 2's ask for a `check_access` test with
  real bound parameters, not just the watchdog test's hand-built
  parameterless sieve). Asserts correct denial *and* that the table survives
  intact, proving binding rather than interpolation.

**Recorded as a known limitation, not fixed here (out of Phase 2 scope):**

- **An extra capability can narrow access below what a broader one alone
  grants** (medium severity, confirmed real) — `CompiledSieve.where_caveats`
  is a flat list spanning *every* entitling capability, ANDed together by
  `merge_sieve`; a caller holding both an unrestricted and a
  narrower-caveated capability on the same resource gets the
  **intersection**, not the union each should independently provide.
  Capabilities are meant to be additive; this is accidentally intersective.
  **Root cause is in `crates/fdae` (Phase 1, already merged via PR #86)** —
  `CompiledSieve` would need to carry each caveat alongside the specific
  OR-branch/permission it entitles, an ADR-0017-level contract change, not a
  `data_db`/Phase 2 fix. Both reviewers independently agreed this is
  Phase-1-scoped. Recorded as Decision Register **D-04-02-g** in task.md
  (open, not gating B2). Added
  `tests_fdae.rs::two_capabilities_with_conflicting_caveats_currently_narrow_to_zero_rows`,
  which pins today's (undesired) behavior explicitly with a comment
  directing whoever resolves D-04-02-g to flip the assertion — so the fix,
  when it lands, has a concrete regression to update rather than rediscovering
  the bug.

**Reviewed and no action needed:**

- **FDAE enforces nothing yet for any real caller (`auth = None`
  everywhere)** (informational) — correct and already documented at every
  call site, in this file, and in task.md; real `QueryAuth` construction is
  Phase 3.
- **Write-side integrity (`put`/`patch`) is unenforced** (Reviewer 2) —
  already correctly scoped to Slice B5-fdae behind sub-decision D-04-02-f;
  no new information, no action.

Verification after the two code changes above:
`cargo test -p syneroym-data-db` — **119 passed, 0 failed** (117 prior + 2
new); `cargo +nightly fmt --all` clean; `cargo clippy --workspace
--all-targets --all-features` zero warnings.

## Phase 3 — WIT `check-access` + Host QueryAuth Wiring + CLS Strip ✅ (2026-07-21)

Branch: `feat/m04b-slice-b2-data-db` (same branch/PR as Phases 1-2). Plan:
[slice-b2-phase3-plan.md](slice-b2-phase3-plan.md). `crates/fdae` and
`crates/data_db`'s `QueryAuth`/`ReadOutcome`/`check_access` are unchanged
ground truth for this phase.

### What was delivered

- **WIT** — additive `check-access: func(collection, id, operation) ->
  result<bool, data-layer-error>` added to
  `crates/wit_interfaces/wit/data-layer/data-layer.wit`'s `store` interface,
  after `query-raw`. `wit/host/deps/data-layer/data-layer.wit` and every
  `test-components/*/wit/deps/data-layer/data-layer.wit` are symlinks to this
  one file, so the host `bindgen!` and every guest `generate!` picked it up
  from the single edit — no manual mirror, no guest rebuild needed (additive;
  existing guests ignore it).
- **`crates/data_db/src/auth.rs`** — `pub fn strip_masked_fields(payload:
  Vec<u8>, masked: &[String]) -> Result<Vec<u8>, DataLayerError>`: removes
  each top-level key named in `masked` from a JSON-object payload. Fail-closed
  (a non-empty mask against a payload that won't parse as a JSON object is an
  `Err`, never a pass-through); an empty mask returns the payload untouched
  without parsing it. Exported alongside `QueryAuth`/`ReadOutcome`; 5 new unit
  tests.
- **`HostState.fdae_policy: Option<Arc<syneroym_fdae::Policy>>`**
  (`crates/sandbox_wasm/src/host_capabilities.rs`) — `None` = today's
  unfiltered behavior. New trailing `HostState::new` param, threaded through
  every call site (the one production site in `engine.rs` passes `None`; all
  ~17 test/bench sites pass `None` except the new Phase-3 host tests, which
  pass `Some(policy)`). A private `HostState::query_auth(&self)` helper
  builds `QueryAuth` from `fdae_policy` + `caller.session` +
  `component_id`, reused by every `store::Host` method below.
- **`store::Host for HostState`** — `get`/`query`/`aggregate`/`delete_many`
  now build a real `QueryAuth` via `query_auth()` instead of a hardcoded
  `None`. New `check_access` method: builds the same `QueryAuth`, delegates
  to `ServiceStore::check_access`, **no capability gate** (unlike
  `execute_ddl`/`query_raw`) — `check-access` *is* the authorization
  primitive, reveals only the caller's own access, and is fail-closed to
  `false` inside the store, so gating it would be circular. `get`/`query`
  capture the full `ReadOutcome` and run `strip_masked_fields` over each
  returned record's payload before returning; a fail-closed `Err` from the
  helper propagates as the method's `Err`. `aggregate` needs no strip — Phase
  2 already denies a CLS-active aggregate outright.
- **Native path** (`crates/control_plane/src/synsvc_native.rs`) — `get`/
  `query` arms gained the same `strip_masked_fields` call (capturing the full
  `ReadOutcome`) for symmetry. `auth` stays `None` here (no policy field on
  `SynSvcNativeService`; that's Phase 4), so `masked_fields` is always empty
  and the strip is a correct no-op today — Phase 4's native policy wiring
  needs zero further change to this path.

### Tests

- **`crates/data_db/src/auth.rs`** (5 new unit tests): strips a named
  top-level key; leaves sibling fields untouched; empty mask returns the
  payload untouched without parsing; a non-JSON payload with a non-empty
  mask fails closed; a mask naming an absent key is a no-op success.
- **`crates/sandbox_wasm/src/host_capabilities.rs::tests`** (4 new
  integration tests, a `HostState` built with a hand-injected `Policy` and a
  `caller.session` carrying real capabilities, seeded rows via the same
  `store::Host` trait the tests exercise):
  - `fdae_rls_filters_get_query_and_check_access` — `get`/`query` return only
    the caller-reachable row; `check_access` returns the right Mode-A bool
    for a reachable vs. unreachable row.
  - `fdae_cls_strips_masked_field_from_get_and_query` — a `fields.deny:
    ["ssn"]` policy strips `ssn` from both `get`'s and `query`'s returned
    payload while leaving sibling fields intact (the row itself is still
    correctly RLS-filtered).
  - `fdae_policy_absent_is_unfiltered_pass_through` — `fdae_policy: None`
    leaves both rows and payloads (including `ssn`) untouched, proving zero
    behavior change on the unconfigured (today's production) path.
  - `fdae_d04_02_g_extra_caveated_capability_narrows_cls_strip` — **required
    D-04-02-g CLS-narrowing pin**: a caller holding both an unrestricted
    `read` capability and a second `read` capability caveated `fields.deny:
    ["ssn"]` on the same resource gets `ssn` stripped even from the
    unrestricted grant's payload (today's over-restrictive union across
    capabilities — mirrors the RLS variant Phase 2 already pinned in
    `tests_fdae.rs`). Comment ties it to D-04-02-g and directs whoever fixes
    it to flip the assertion to "ssn is present".
- **No `wasm32-wasip2` guest rebuild, no through-the-guest E2E** — the WIT
  change is additive and the reference-scenario E2E step needs a deployed
  policy (Phase 4), both deliberately out of scope per the plan.

### Decisions carried into this phase

- **`HostState.fdae_policy` stays `None` in production.** Phase 3 proves
  itself entirely with a hand-injected `Policy` in the new host tests
  (per the phasing note in `slice-b2-implementation-plan.md` §9.3: "Phases
  1-3 are testable with a policy injected directly… land 1-3 first"). **FDAE
  still enforces nothing for a live deployed caller after this phase** — the
  same informational caveat as Phase 2, now also true at the WIT boundary.
  Loading a real policy at instantiation is Phase 4 (deploy/persist/manifest
  plumbing), explicitly out of scope here.
- **No capability gate on `check-access`.** Unlike `execute-ddl`/`query-raw`
  (gated on `data-layer/admin`), `check-access` is itself the authorization
  primitive a guest uses to ask "may I act on this row?" — it reveals only
  the caller's own access and fails closed to `false`, so adding a gate on
  top would be circular and would just turn every legitimate use into a
  denial.
- **CLS strip lives host-side, not in the store.** `strip_masked_fields` is a
  `data_db`-exported utility the host calls after reading a `ReadOutcome`,
  not something `ServiceStore` applies itself — this respects Phase 2's
  recorded "the store never strips fields itself" contract and is why the
  Phase 2 test `masked_fields_exposed_but_rows_unmasked_in_phase_2` stays
  unchanged and still correctly documents the `data_db`-level contract.
- **Native-path strip is a no-op today, by design.** Added for symmetry so
  Phase 4's native policy wiring is a construction-site change only, not a
  new call to wire in.

### Explicitly out of Phase 3 scope (plan §4 — recorded, not silently dropped)

- **Phase 4 — deploy/persist/manifest plumbing**: the `fdae`/`policy_path`
  field on both `ServiceConfig` types + the SDK WIT mapper, deploy-time
  read/validate + `strict:` author-time warning, the `fdae_policies` storage
  table with `save`/`load_fdae_policy`, and `engine.rs` load-at-instantiation.
- **Native-path real policy** — `synsvc_native.rs` gets the strip call but no
  policy source until Phase 4.
- **Decision trace** (ADR-0017 §9) — Phase 5.
- **`strict:` mode enforcement wiring** — the deploy-path author-time warning
  is Phase 4.
- **B3 `anchor` terminal, B4-fdae stage-4 ABAC, B5-fdae write-path gate,
  D-04-02-e native-admission TODO** — later slices, untouched.

### Verification evidence

- `cargo +nightly fmt --all` — clean.
- `cargo clippy --workspace --all-targets --all-features` — zero warnings.
- `cargo test -p syneroym-data-db` — **124 passed, 0 failed** (119 prior + 5
  new `strip_masked_fields` unit tests).
- `cargo test -p syneroym-sandbox-wasm` — **32 passed, 0 failed** (28 prior +
  4 new FDAE host-wiring tests), plus all pre-existing integration test
  binaries (`blob_store_integration`, `data_layer_integration`,
  `lifecycle_hooks`, `messaging_integration`, `stream_integration`) green.
- `cargo test -p syneroym-control-plane` — green (native-path strip is a
  no-op on the `auth = None` path; no behavior change).
- `cargo test --workspace` — all crates green. (`syneroym-coordinator-iroh`'s
  `connection_limit::accepts_up_to_cap_and_rejects_the_rest` fails under
  this CLI's default network sandbox — "Operation not permitted" binding a
  UDP relay socket — pre-existing/environmental, unrelated to this change,
  same as Phase 2.)
- `mise run test:e2e` — not run, same reasoning as Phase 2: the WIT change is
  additive and no call site's real behavior changes for a production caller
  (`fdae_policy` is `None` everywhere real deployment happens), so there is
  nothing new for the Playwright e2e suite to exercise. A deliberate skip,
  recorded per the plan's own scoping (§2).
