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

### Post-commit review (2026-07-21)

Reviewed against commit `fc8d7d5`. Independently re-ran the new
`sandbox_wasm` FDAE host tests, the `strip_masked_fields` unit tests, and
clippy on the three touched crates before reviewing.

**Addressed, code changed (this session, still Phase 3 scope):**

- **CLS masks output only — the masked column stayed filterable, so its
  value was recoverable via predicate probing** (medium severity, confirmed)
  — the Phase-3 strip removes a masked field from the *returned payload*,
  but `do_query`'s caller-supplied filter compiles directly against the raw
  `payload` JSON (`filter::compile_filter`, unaware of `masked_fields`), and
  supports `$regex`/comparison operators. A caller could filter on a masked
  field (e.g. `{"ssn": {"$regex": "^111"}}`) and read the value back out via
  row presence/absence, or extract it character-by-character — even though
  the field never appears in the output. This meant `task.md`'s "CLS: value
  never returned" row read as satisfied when the requirement's actual intent
  (the caller cannot *learn* the masked value) wasn't met. Fixed: new
  `filter::referenced_top_level_fields` extracts the top-level field names a
  filter document touches (recursing through `$and`/`$or`/`$not`); `do_query`
  now rejects (`PermissionDenied`) a filter that references any
  `masked_fields` key, before compiling or running it — masked fields are
  always flat top-level keys (`compile_cls` copies `fields.deny` verbatim),
  so no path-parsing complexity. `aggregate` needed no equivalent fix
  (Phase 2 already denies a CLS-active aggregate outright) and `get` takes
  no filter. New tests:
  `tests_fdae.rs::query_filter_referencing_a_cls_masked_field_is_denied`
  (bare and `$and`/dotted-path forms) and
  `::query_filter_on_non_masked_field_still_works_when_cls_active` (proves
  the deny doesn't over-trigger), plus 3 new `filter.rs` unit tests for the
  extraction helper itself.
- **`aggregate`/`delete_many` host-path wiring was untested with a real
  policy** (low severity) — the new Phase-3 host tests covered `get`/
  `query`/`check_access` with an injected policy, but nothing exercised
  `aggregate`/`delete_many` through `store::Host` with `Some(policy)`, so a
  dropped or `None`-replaced `query_auth()` call at either site would have
  passed every existing test. Added
  `host_capabilities.rs::tests::fdae_aggregate_is_row_filtered_through_host`
  and `::fdae_delete_many_is_write_filtered_through_host`.
- **Inline `std::mem::take` violated the repo's own import convention** (low
  severity) — `host_capabilities.rs`'s and `synsvc_native.rs`'s `query` strip
  loops called `std::mem::take(...)` as an inline fully-qualified path.
  AGENTS.md's import-cleanup rule asks for functions qualified by parent
  module; fixed by importing `std::mem` and calling `mem::take(...)` in both
  files.

**Recorded as a known, by-design boundary (out of Phase 3 scope):**

- **`query_raw` is not sieve-aware** (informational) — the privileged
  raw-SQL escape hatch threads no `QueryAuth` and applies neither RLS nor
  CLS. This is guarded by the `data-layer/admin` capability (a higher trust
  tier than ordinary read access) and predates this slice, not a Phase-3
  regression — but now that CLS is "live," it's worth recording explicitly
  that the row/column guarantees have a deliberate gap at `query_raw`: an
  admin-capable caller can read any masked column directly. No code change;
  flagged here so it's a documented limit of the CLS guarantee, not an
  assumed-closed one.

**Reviewed and no action needed:**

- **FDAE enforces nothing yet for any live deployed caller
  (`fdae_policy: None` everywhere in production)** (informational) — already
  correctly documented in this file, task.md, and the plan; unchanged by
  this review.

Verification after the code changes above: `cargo test -p syneroym-data-db`
— **129 passed, 0 failed** (124 prior + 5 new: 2 filter-probe integration
tests + 3 `filter.rs` unit tests); `cargo test -p syneroym-sandbox-wasm` —
**34 passed, 0 failed** (32 prior + 2 new aggregate/delete_many host tests);
`cargo +nightly fmt --all` clean; `cargo clippy --workspace --all-targets
--all-features` zero warnings; `cargo test --workspace` green (same
pre-existing/environmental `coordinator-iroh` sandbox failure, confirmed
unrelated by rerunning with the sandbox disabled).

## Phase 4 — Manifest + Deploy + Persistence Plumbing ✅ (2026-07-21)

Branch: `feat/m04b-slice-b2-data-db` (same branch/PR as Phases 1-3). Plan:
[slice-b2-phase4-deploy-persist-plan.md](slice-b2-phase4-deploy-persist-plan.md).
`crates/fdae`'s `Policy`/`parse_and_validate`/`compile_read`, `crates/data_db`'s
`QueryAuth`/`ReadOutcome`/`check_access`/`strip_masked_fields`,
`HostState.fdae_policy`/`query_auth()`, and the WIT `check-access` function are
unchanged ground truth for this phase.

### What was delivered

- **Manifest** — `ServiceConfig.fdae: Option<FdaeManifest>`
  (`app_orchestration/src/models.rs`) and the mirrored WIT `service-config.
  fdae-policy-path: option<string>` (`control-plane.wit`), copied by
  `sdk::mapper::map_deployment_plan_to_wit`. All ~32 existing struct-literal
  sites (6 Rust `ServiceConfig`, 26 WIT `WitServiceConfig`) updated
  mechanically to `fdae: None` / `fdae_policy_path: None` — zero behavior
  change confirmed by the full pre-existing suite staying green.
- **Deploy-time read, validate, persist** (`control_plane/src/service/
  orchestration.rs`'s `deploy`) — `fdae_policy_path` is read relative to the
  substrate's working directory with the same traversal guard as
  `schema_path`, parsed via `syneroym_fdae::parse_and_validate` (a hard
  deploy failure on any error), and persisted via the new
  `StorageProvider::save_fdae_policy` **before** the service is actually
  instantiated (so `init`/`migrate`'s first read already sees the row).
  Deliberately **not** nested inside the `custom_config` block the way
  `schema_path` is — a policy is independent of `custom_config` — regression-
  tested explicitly.
- **`strict:` author-time warning** (D-04-02-c) — a new
  `ServiceStore::list_collections` (excludes `sqlite_%` and `_%` tables) is
  called after the service's own DB exists (post first-deploy `init()`), and
  `warn_on_policy_collection_mismatch` warns in both directions: a table with
  no matching `definitions:` entry (would be denied under `strict: true`),
  and a `definitions:` entry whose table doesn't exist yet (expected for a
  lazily-initialized TCP/container service). Both are `tracing::warn!`, never
  a deploy failure.
- **Persistence** — new `fdae_policies` table in `substrate.db`
  (`service_id TEXT PRIMARY KEY, policy_json TEXT NOT NULL, updated_at
  INTEGER NOT NULL`), created by a new `run_fdae_migration` alongside the
  existing M3A/M3B migrations (not named after this milestone, per AGENTS.md).
  `save_fdae_policy`/`load_fdae_policy` on `StorageProvider`, last-write-wins
  (`INSERT … ON CONFLICT (service_id) DO UPDATE`) — a policy has no
  generation ladder, unlike config generations: ADR-0017's grant-layer design
  means a deployed policy must bind late, so tightening it must take effect
  immediately, not behind a version pin.
- **Native dispatch enforcement** (`control_plane/src/synsvc_native.rs`) —
  `SynSvcNativeService` gains `fdae_policy: Option<Arc<Policy>>` (set once at
  construction from the `Arc<Policy>` `deploy` already parsed; no load, no
  cache, no parse on this hot path) and a private `query_auth()` helper
  mirroring `HostState::query_auth`, wired into all four read/delete sites
  (`get`/`query`/`delete_many`/`aggregate`) in place of the former hardcoded
  `None`. **Deliberately no `AuthLevel` carve-out** — branching to `auth =
  None` for a synthesized/system caller would make the guest self-proxy
  ingress *more* permissive than the direct WIT path under the same policy,
  i.e. a bypass. `strip_record`'s doc comment (stale since Phase 3, "no
  policy source until Phase 4") rewritten to describe live CLS. The one
  production construction site (`orchestration.rs`'s `deploy`) now threads
  the just-parsed `Arc<Policy>`; the 11 test construction sites (`router`
  crate) pass `None`, preserving their existing behavior exactly.
- **WASM instantiation** (`sandbox_wasm/src/engine.rs`) — new
  `fdae_policies: DashMap<String, Option<Arc<Policy>>>` cache next to the
  component cache (the `Option` is itself cached, so "resolved: no policy" —
  the common case — doesn't re-query `substrate.db` per invocation).
  `build_store_and_instantiate`'s new `resolve_fdae_policy` helper looks up,
  and on a miss loads + `parse_and_validate`s + inserts; a parse failure at
  this point is fail-closed-**absent** (log and cache `None`, not deny every
  read for the service — the deploy path is what rejects a bad policy before
  it's ever persisted, so a row that fails to parse here means the DB was
  tampered with or the crate's schema moved since deploy). Evicted on
  `stop_wasm` and `compile_and_cache_wasm` (a re-deploy's recompile) so a
  redeploy re-resolves rather than serving the previous policy. Because the
  load is from `fdae_policies` (not from any in-memory deploy result), this
  is correct across a substrate restart: `load_cached_wasm` recompiles from
  disk and the next instantiation re-resolves the policy from the DB.

### What Phase 4 does and does not make live (§2 of the plan — stated per-ingress, not native-vs-WASM)

**Enforced** — an external, router-verified caller reaching native dispatch
through `dispatch_json_rpc_once` (`dispatch.rs:99-105` threads the verified
`CallerContext` into `NativeInvocation.caller`). This is the phase's headline
proof: `router/tests/native_dispatch_identity.rs`'s
`native_fdae_policy_row_filters_and_masks_for_two_distinct_verified_callers`
seeds two documents owned by two different verified callers and asserts each
sees only their own row, with a CLS-masked field absent from the payload.

**Not enforced (empty), by ingress, both pre-existing behavior changes on
paths that previously read unfiltered, both fail toward over-restriction:**

- **Guest → WIT host functions** (`prepare_wasm_execution` synthesizes
  `CallerContext::service_system(service_id)` — "the callee acts as itself",
  settled in M04A). A guest's own `query`/`get` under a deployed policy sees
  none of the rows it wrote via the (ungated) write path, since
  `service_system`'s empty capabilities can never be entitled to any
  permission and `compile_read` falls to `deny_all()`. Pinned:
  `sandbox_wasm/tests/data_layer_integration.rs::test_deployed_policy_yields_empty_guest_originated_query_d04_02_h`.
- **Guest self-proxy → native dispatch** — a guest's `syneroym:proxy` call
  into its **own** service's native `data-layer` also carries a synthesized
  `service_system` identity (`host_capabilities.rs`'s `proxy::Host::call`),
  and the proxy gate's same-service exception (`proxy.rs:224-231`)
  deliberately permits the call to reach `SynSvcNativeService` — the exact
  code the native-enforcement wiring above made policy-aware. **This is a
  behavior change**: before Phase 4 this ingress read unfiltered (`auth =
  None` everywhere); after Phase 4, for a policy-carrying service, it reads
  empty. Pinned in both directions, since this path had zero coverage
  before this phase: `router/tests/proxy_dispatch.rs`'s
  `guest_self_proxy_data_layer_reads_normally_when_policy_absent` (baseline,
  pins the same-service exception itself as intended behavior) and
  `guest_self_proxy_data_layer_returns_empty_when_policy_present` (the
  D-04-02-h pin).

Both gaps are recorded as **D-04-02-h** in `task.md`'s Decision Register,
expected to resolve alongside Slice B3's `anchor_did` work (the same
original-principal question), not as a slice of its own.

### Tests

- **`app_orchestration`** (`models.rs`) — `test_manifest_parsing_toml_with_fdae_policy`:
  a `[services.x.fdae] policy_path = "…"` TOML block parses into
  `Some(FdaeManifest)` and survives a `to_toml`/`from_toml` round trip; the
  existing `test_manifest_parsing_toml` gained an assertion that a manifest
  without the block parses with `fdae: None`.
- **`sdk`** (`mapper.rs`, new `#[cfg(test)] mod tests`) —
  `map_deployment_plan_to_wit_copies_fdae_policy_path` and
  `..._maps_absent_fdae_to_none`: the mapper's `fdae.policy_path` copy into
  `fdae_policy_path`, both directions (the §9.1 "unreachable code" guard --
  without this the field is silently dropped at the WIT boundary).
- **`data_db`** (`sqlite.rs`'s existing private `tests` module) —
  `test_fdae_policy_save_load_roundtrip_and_replace` (round trip; a second
  save for the same `service_id` replaces, one row; an unknown `service_id`
  is `Ok(None)`) and
  `test_list_collections_returns_created_tables_excludes_vault_and_sqlite_internals`.
- **`control_plane`** (`orchestration.rs`'s `#[cfg(test)] mod tests`) — four
  new deploy tests modeled on `test_deploy_config_schema_rejection`:
  `test_deploy_fdae_policy_validates_persists_and_is_loadable` (also the
  regression test for the FDAE block's placement outside `custom_config`),
  `test_deploy_fdae_policy_schema_invalid_rejected_and_not_persisted`,
  `test_deploy_fdae_policy_path_traversal_and_absolute_rejected`, and a
  direct unit test of the extracted `warn_on_policy_collection_mismatch`
  helper, `test_warn_on_policy_collection_mismatch_fires_in_both_directions`
  (a `tracing` capture, asserting both warning directions fire and a
  correctly-defined collection does not warn).
- **Native end-to-end — the phase's headline test** — see above.
- **Guest self-proxy ingress** — see above.
- **`sandbox_wasm`** — four new internal `engine::tests` unit tests
  (`fdae_policy_absent_resolves_none_and_caches`,
  `fdae_policy_present_resolves_some_and_cache_hit_skips_storage`,
  `fdae_policy_cache_evicted_on_stop_wasm_and_recompile`,
  `fdae_policy_unparseable_in_storage_resolves_none_not_error`) exercising
  the engine's cache directly (private-field access from the same module),
  plus the D-04-02-h pin in `data_layer_integration.rs` above.
- **Unchanged and stays green**: the D-04-02-g pins, every Phase 2/3 test,
  and all pre-existing deploy/mapper/manifest tests — the ~32 mechanical
  `None` literal sites change no behavior, confirmed by the full pre-existing
  suites passing unmodified.

### Decisions carried into this phase

- **Policy documents are JSON, not YAML** — `parse_and_validate` is
  `serde_json::from_str`; ADR-0017's examples are YAML for readability only.
  Noted in `task.md`'s Migration Strategy and belongs in the developer guide.
- **No generation ladder for policies** — last-write-wins via
  `ON CONFLICT (service_id) DO UPDATE`, because a grant that names a policy
  binds late by design (a deployed policy must take effect immediately on
  tightening, unlike a config generation that a grant can pin a version of).
- **The `strict:` warning is warn-only, in both directions, never a deploy
  failure** — D-04-02-c's resolution; direction 2 (a definition whose table
  doesn't exist yet) legitimately fires for a TCP/container service whose
  collections are created lazily on first use, so it must read as an
  expected case, not an error.
- **Engine-side policy cache, and why** — `parse_and_validate` re-compiles
  the embedded JSON Schema on every call; `build_store_and_instantiate` runs
  on *every* guest invocation, so caching (keyed by `service_id`, `Option`-
  valued so the no-policy case is cached too) is what keeps schema
  compilation off the hot path. Evicted on `stop_wasm`/recompile, not on a
  TTL, since a policy only changes on a re-deploy.
- **No `fdae_policies` rollback on a later deploy-failure path** — unlike
  `rollback_config_generation`, a deploy failure after the policy row is
  persisted (but before native-capability registration or owner attribution
  succeeds) leaves the row in place. No code path reads a policy for a
  service_id whose deploy never completed, and any future successful
  (re-)deploy of the same `service_id` overwrites the row unconditionally via
  `ON CONFLICT DO UPDATE`, so the row is inert, not a leak. Simpler than
  inventing a `delete_fdae_policy` method the plan's own trait list (§1.6)
  did not specify.

### Explicitly out of Phase 4 scope (plan §5 — recorded, not silently dropped)

- **Threading real caller identity into guest-originated reads** (D-04-02-h,
  both ingresses) — expected alongside B3's `anchor_did`. Not worked around
  by an `AuthLevel::System` sieve exemption (would make the self-proxy
  ingress a bypass of the direct-caller ingress's enforcement).
- **Reference-scenario step 22's "…never reaches the WASM guest" half** —
  blocked on the above; the filtering half is closed by this phase's native
  end-to-end test. No Playwright spec added or modified.
- **Decision trace** (ADR-0017 §9) — held at Phase 5, per the plan's own
  reasoning (pulling it forward would reopen `crates/fdae`'s Phase 1
  contract mid-flight, and Phase 5 follows immediately on the same
  branch/PR). Until Phase 5, a deny is diagnosable only from `RUST_LOG`
  tracing and the policy document itself.
- **Benchmarks** (`criterion` FDAE pushdown bench, the < 25 ms p99 budget
  row) — Phase 5.
- **Failure/Security matrix sign-off** — Phase 5.
- **Native `check-access` JSON-RPC method** — Mode A is not exposed on the
  native dispatch surface; adding it would be new API, not plumbing.
- **Policy-configurable watchdog budget** — still the interim
  `FDAE_MAX_VM_OPS` constant.
- **B3 `anchor` terminal, B4-fdae stage-4 ABAC, B5-fdae write-path gate,
  D-04-02-e native-admission TODO, `router/src/proxy.rs`'s interim gate** —
  later slices, untouched; the proxy gate was not widened while touching
  adjacent code.
- **`query_raw` sieve-awareness** — the documented Phase 3 CLS gap stands,
  guarded by `data-layer/admin`, unchanged here.

### Verification evidence

- `cargo +nightly fmt --all` — clean.
- `cargo clippy --workspace --all-targets --all-features` — zero warnings.
- `cargo test -p syneroym-app-orchestration` — 53 passed, 0 failed (52 prior
  + 1 new manifest test).
- `cargo test -p syneroym-sdk` — 2 passed, 0 failed (both new mapper tests;
  the crate had no prior test module).
- `cargo test -p syneroym-data-db` — 131 passed, 0 failed (129 prior + 2 new
  storage tests).
- `cargo test -p syneroym-control-plane --lib` — 30 passed, 0 failed (26
  prior + 4 new deploy/strict-warning tests).
- `cargo test -p syneroym-sandbox-wasm --lib --tests` — 67 passed, 0 failed
  across the lib and all integration test binaries (62 prior + 5 new: 4
  engine cache unit tests + the D-04-02-h pin in `data_layer_integration.rs`).
- `cargo test -p syneroym-router --lib --tests` — 114 passed, 0 failed
  across the lib (71) and all six test binaries (`deploy_grant` 9,
  `native_dispatch_identity` 16 -- including the new
  `native_fdae_policy_row_filters_and_masks_for_two_distinct_verified_callers`
  headline test, `proxy_dispatch` 4 -- including the two new self-proxy
  pins, `service_ownership` 10, `ucan_context` 2, `unsupported_protocol` 2).
  One run under heavy parallel background test load hit a one-off panic in
  `authenticated_caller_reaches_native_dispatch` (`mainline` DHT actor:
  `"actor thread unexpectedly shutdown: SendError(..)"`) -- unrelated to
  this test's own assertions; reran clean both in isolation and as part of
  the full 16-test binary immediately after, confirming a resource-
  contention flake, not a regression.
- `cargo test --workspace --no-fail-fast` — under this CLI's default network
  sandbox, 9 test targets fail on socket/UDP binding (`coordinator-iroh`'s
  `connection_limit`/`multi_hop_relay`/`tls_rotation`, `mqtt-broker`'s lib
  tests, `sdk`'s `connect_timeout`, and `substrate`'s `basic_lifecycle`/
  `http_passthrough_e2e`/`messaging_client_e2e`/`stream_client_e2e`) — all
  pre-existing/environmental (none of these crates' test files were touched
  this phase), same class as the `coordinator-iroh` failure Phases 2/3
  documented. Rerunning the full workspace suite with the sandbox disabled
  passed with **zero failures** (confirmed twice, including a rerun after
  the final import-hygiene pass).
- `mise run test:e2e` — not run. The reference-scenario E2E fixtures
  (`crates/substrate/tests/e2e/tests/`) are `webrtc.spec.ts` and
  `multi-hop.spec.ts` against `miniapp-demo1-web`, a Rust HTTP backend with
  no data-layer use and anonymous browser visitors by design — there is
  nothing in that suite for a deployed FDAE policy to touch. Closing
  step 22's filtering half with a Rust integration test rather than
  Playwright is the established convention (M04A closed steps 20/21/24/25
  the same way). A deliberate skip, recorded per the plan's own scoping
  (§2), same reasoning as Phases 2 and 3.
- `wasm32-wasip2` — unbroken. The `control-plane.wit` change is additive and
  touches no guest-imported interface (`data-layer.wit` is untouched this
  phase), so no `test-components` rebuild was required; confirmed via the
  `data-layer-test` fixture's existing compiled artifact still exercising
  correctly through `data_layer_integration.rs` (including the new
  D-04-02-h test).

### Post-commit review (2026-07-21)

Independent review against commit `7c0270a`. Re-ran every gate from a clean
tree (no code modified before reviewing) and confirmed F1 and F3's disclosure
behavior by direct execution, not inspection alone. Ten findings; two high,
three medium, five low. All ten were independently re-verified against the
code before being addressed below — two (F1, F4) by temporarily reverting the
fix and confirming the new regression test actually fails without it.

**Addressed, code changed (this session):**

- **F1 (High) — a `migrate()`/`init()` hook under a deployed policy silently
  read zero rows.** `invoke_lifecycle_hook` builds `CallerContext::
  local_elevated`, whose `data-layer/admin` capability entails
  `data-layer/read` and covers every collection -- so instead of the
  synthesized-identity `deny_all()` D-04-02-h describes, `compile_read`
  compiled a *real* sieve bound to `"system:local-elevated:<service_id>"`, a
  DID no principal row can ever hold. A migration reading its own data to
  decide how to rewrite it would see nothing and could act on that
  emptiness -- confirmed by reverting the fix and watching the new
  regression test fail with `left: 0, right: 2`. Fixed: `HostState::
  query_auth` now returns `None` for `AuthLevel::LocalElevated`, distinct
  from the `AuthLevel::System` carve-out that stays refused (`LocalElevated`
  is exclusively host-synthesized for `init`/`migrate`, never guest-
  reachable, so exempting it cannot become a self-proxy bypass the way
  exempting `System` would). New test:
  `host_capabilities.rs::fdae_local_elevated_lifecycle_reads_stay_unfiltered_under_a_policy`.
- **F2 (High) — a policy could never be removed, and the WASM engine cache
  resurrected it.** No `delete_fdae_policy` existed anywhere, and `undeploy`
  never touched `fdae_policies`; a re-deploy dropping the `[services.x.fdae]`
  block (with or without an intervening undeploy) left the row in place, so
  `AppSandboxEngine::resolve_fdae_policy` kept serving the stale policy to
  the WASM ingress while native dispatch had correctly gone unfiltered --
  two ingresses of the same service enforcing different policies with no way
  to un-declare one. Fixed: new `StorageProvider::delete_fdae_policy`,
  called from `undeploy` and from `deploy` whenever the manifest no longer
  declares `fdae_policy_path`. New tests: `test_undeploy_removes_fdae_policy`,
  `test_redeploy_without_fdae_block_clears_previous_policy`,
  `sqlite::tests::test_fdae_policy_delete_is_idempotent_and_removes_the_row`.
- **F3 (Medium) — the deploy error echoed policy-file content back to the
  caller.** `PolicyError::Schema`'s `to_string()` wraps `jsonschema::
  ValidationError::Display`, which embeds the offending JSON *instance* --
  for a top-level type mismatch on `fdae_policy_path`, that instance is the
  whole file, unlike `schema_path` (whose instance is always the caller's
  own `custom_config`). Confirmed by reading the `jsonschema` 0.46 source
  directly (`ValidationErrorKind::Type`'s `Display` arm) and by a test that
  writes a `"SUPER_SECRET_API_KEY_abc123"` policy file and asserts it does
  not appear in the returned error. A caller holding `orchestrator/deploy`
  -- which, on an unowned substrate (the runtime's default until a
  `ControllerAgreement` exists), is *every* verified caller -- could aim
  `fdae_policy_path` at any JSON file below the substrate's working
  directory and read fragments back through failed deploys. Fixed: the
  underlying error is logged in full via `tracing::warn!`; the caller gets
  a fixed generic message. New test:
  `test_deploy_fdae_policy_error_does_not_echo_file_contents`.
- **F4 (Medium) — lost cache invalidation in `resolve_fdae_policy`.**
  Check-cache → `await` storage load → insert, with no lock held across the
  await, so a redeploy's eviction landing mid-load (against a key not yet
  cached) could be immediately undone by the racing load's own insert once
  it finally completed -- silently serving a stale policy until the next
  `stop_wasm`/redeploy, contradicting ADR-0017's "tightening must take
  effect immediately." Confirmed by reverting the fix and watching the new
  race test fail. Fixed: a per-service generation counter
  (`fdae_policy_generation`), bumped by both eviction sites, captured before
  and compared after the storage read; a mismatch means an eviction raced
  the load, so the result is returned for that call but not cached. New
  test: `engine::tests::fdae_policy_resolution_racing_an_eviction_is_not_cached`,
  which reproduces the race deterministically via a `RacingStorageProvider`
  test double that pauses `load_fdae_policy` on a `Notify` -- not a flaky
  sleep-based timing test. The lower-severity thundering-herd cost the same
  finding raised (concurrent cold-cache misses each independently hit
  storage) is unaddressed -- deduplicating concurrent loads needs a per-key
  async lock, which is a proportionate fix for a perf optimization, not the
  correctness bug this session prioritized.
- **F5 (Medium) — a failed deploy left its policy in force, contradicting
  the code comment.** The in-comment justification ("nothing reads a policy
  for a service_id whose deploy never completed") was wrong for a *re*-
  deploy: `save_fdae_policy` runs before `deploy_wasm_service`, whose own
  first-branch failure only rolled back the config generation, so a
  still-running previous version's engine cache (evicted by
  `compile_and_cache_wasm` before the failure) would resolve the failed
  deploy's policy on its next miss. Fixed with more care than a blind
  delete: `fdae_policies` is last-write-wins with no generation ladder
  (unlike `config_generations`), so unconditionally deleting on rollback
  would have struck a still-valid *previous* policy on a re-deploy. `deploy`
  now captures the previous value via `load_fdae_policy` before overwriting,
  and `rollback_fdae_policy` (mirroring `rollback_config_generation`,
  called at the same four sites) restores it -- or deletes, only when there
  was no previous policy. New test:
  `test_deploy_failure_restores_previous_fdae_policy_not_the_new_one`,
  which deploys policy P1 successfully, then fails a re-deploy carrying
  policy P2, and asserts P1 (not P2, not an empty row) survives.
- **F6 (Low) — `list_collections` hid every collection whose name starts
  with `_`.** `IDENTIFIER_REGEX` (`^[a-zA-Z_]...`) permits a leading
  underscore, so a guest-created collection like `_audit` is a legal name
  that the `_%`-wide exclusion (written to drop the host's `_vault`) also
  swallowed -- direction 1 of the `strict:` warning would never fire for it,
  and direction 2 would false-positive claiming it doesn't exist. Fixed:
  excludes `_vault` by exact name. Test extended with a `_audit` collection
  asserted present in the result.
- **F7 (Low) — `delete_many`/`aggregate`'s native `QueryAuth` wiring was
  untested.** The headline test only drove `get`/`query`. New tests:
  `native_delete_many_is_row_filtered_as_a_write_operation` (a write-capable
  caller's `delete-many` removes only their own reachable row; verified via
  `query-raw` as an admin caller, independent of the RLS under test) and
  `native_aggregate_is_row_filtered_through_native_dispatch` (RLS-filtered
  count; a CLS-active policy was deliberately *not* used here, since
  `aggregate` already fails a CLS-active sieve closed outright -- confirmed
  correct and unchanged).
- **F9 (Low, partial) — two comments misattributed plan-only content to
  ADR-0017 section numbers, and one misattributed a `task.md` Decision
  Register entry to ADR-0017 itself.** `synsvc_native.rs`'s `query_auth` doc
  comment said "see ADR-0017's D-04-02-h in `task.md`" -- D-04-02-h is a
  `task.md` Decision Register entry; ADR-0017 does not contain it. Two
  other comments (`synsvc_native.rs`'s `strip_record`,
  `native_dispatch_identity.rs`'s section header) cited "(ADR-0017 §2.1)"
  for the ingress-enforcement distinction, which is actually the Phase 4
  plan's own §2 numbering -- ADR-0017's real §2.1 is "Defaults, per layer"
  (default-absent semantics), unrelated content. Fixed: all three corrected
  to drop the wrong citation rather than repeat it.

**Reviewed, not code-changed (context recorded here):**

- **F3's symlink/canonicalization gap.** The traversal guard rejects
  `ParentDir` components and absolute paths but never canonicalizes, so a
  symlink under the working directory could still walk outside it. This is
  not new: it is the exact guard `schema_path` already uses, deliberately
  mirrored per this phase's own plan ("Same guard as schema_path"). Fixing
  it only for `fdae_policy_path` would diverge from `schema_path`'s
  identical, already-shipped behavior; fixing both is a real but separate,
  self-contained hardening task, not a Phase 4 regression. Flagged as a
  follow-up rather than fixed asymmetrically here.
- **F8 — the D-04-02-h pins silently pass (`eprintln!` + early `return`)
  when the `proxy-test`/`greeter`/`data-layer-test` WASM fixtures aren't
  built**, so a job that skips the `wasm32-wasip2` build step would never
  exercise the two tests that are the only guard on a deliberate behavior
  change to an already-reachable production path. Checked against
  `.github/actions/ci-build-and-test/action.yml`: CI builds every
  `test-components/*` fixture unconditionally before `cargo test
  --workspace`, so in the environment that actually gates merges this
  finding's risk does not materialize today. The silent-skip pattern itself
  predates this phase and is used by every WASM-fixture-dependent test in
  both files (`test_deploy_init_crud_creator_id_and_migrate`,
  `guest_to_guest_same_node_proxy_call_returns_typed_result`, etc.) --
  changing it for only the two new tests would be an isolated inconsistency
  within files that otherwise agree; changing it file-wide is a real but
  separate convention decision (e.g. failing loud instead of skipping),
  out of scope for a targeted fix pass.
- **F10 — a node-wide admin's reads go empty with no diagnostic.** Confirmed
  correct, not a bug: `Capability::grants` short-circuits for
  substrate-scoped capabilities, so a node-wide admin is entitled to the
  permission and then row-filtered by the ReBAC path against their own DID
  -- typically to nothing, which is what default-deny asks for
  (`query_raw`/`execute_ddl` remain the admin escape hatch). The
  operability gap is real: until Phase 5's decision trace lands, an
  unexpectedly empty result is diagnosable only from `RUST_LOG` and the
  policy document, and ADR-0007's "no result is a valid outcome" means it
  does not even look like a denial. Already tracked as a named Phase 4
  limitation (this file, "The decision trace" under Explicitly out of Phase
  4 scope) -- no new action, but worth restating plainly here since the
  review specifically asked for it to be visible wherever Phase 4 is
  announced as enforcing.

### Verification evidence (post-review)

- `cargo +nightly fmt --all` — clean.
- `cargo clippy --workspace --all-targets --all-features` — zero warnings.
- `cargo test -p syneroym-sandbox-wasm --lib --tests` — 89 passed, 0 failed
  (lib 40, up from 38: the F1 and F4 regression tests, both independently
  confirmed to fail without their fix; the six integration binaries
  unchanged at 5+2+6+3+13+... see per-binary counts above -- all green).
- `cargo test -p syneroym-control-plane --lib` — 34 passed, 0 failed (30
  prior + F2's `test_undeploy_removes_fdae_policy`/
  `test_redeploy_without_fdae_block_clears_previous_policy`, F3's
  `test_deploy_fdae_policy_error_does_not_echo_file_contents`, and F5's
  `test_deploy_failure_restores_previous_fdae_policy_not_the_new_one`).
- `cargo test -p syneroym-data-db --lib` — 132 passed, 0 failed (131 prior
  + F2's `test_fdae_policy_delete_is_idempotent_and_removes_the_row`; F6's
  fix is an assertion change on an existing test, not a new one).
- `cargo test -p syneroym-router --test native_dispatch_identity` — 18
  passed, 0 failed (16 prior + F7's `native_delete_many_is_row_filtered_as_a_write_operation`
  and `native_aggregate_is_row_filtered_through_native_dispatch`).
- `cargo test --workspace --no-fail-fast` (sandbox disabled) — zero
  failures, full clean run (no `error: N targets failed` summary; every
  `test result:` line green through to the doctests, which run last).

### Post-commit review, second pass (2026-07-21)

A follow-up review ran against `e5fbc3a` plus the working tree (which by
then also held the separately-committed `2955ee5`, closing F3's recorded
symlink-canonicalization gap for both `schema_path` and `fdae_policy_path`).
Re-ran every gate from the working tree before reviewing. Disposition of
the ten F1-F10 findings: seven fixed and confirmed (F2, F3, F4 with a noted
residual, F6, F7), one regressed (F1 -- see N1 below), one partial (F5 -- a
second failure branch it named was still unrolled-back), two accepted with
no new information (F8, F10) and one partial-but-accepted (F9, citation
convention). Three new findings (N1-N3) from the fixes themselves. Each was
independently re-verified against current code before being addressed --
N1 and N2 by reverting the fix and confirming the new regression test fails
without it, matching this file's established practice from the first
post-commit review.

**Addressed, code changed (this session):**

- **N1 (High) — the `LocalElevated` exemption F1 added was reachable from
  the wire, turning a silent zero-rows bug into a total FDAE bypass.**
  F1 fixed `HostState::query_auth` to exempt `AuthLevel::LocalElevated`
  from the sieve, reasoning that `engine.rs`'s `invoke_lifecycle_hook` is
  its sole producer and no guest input can request it. That's true of
  `invoke_lifecycle_hook` itself, but `prepare_wasm_execution` -- the
  ordinary dispatch path reached from wire-originated JSON-RPC
  (`dispatch.rs`) and guest-to-guest proxy calls, both of which let an
  untrusted caller pick `method_name` freely -- independently synthesized
  the same `local_elevated` context whenever `method_name` was `"init"` or
  `"migrate"`, a check that predates FDAE (M3A) and was never guarded by
  any capability. Sending `{"method":"init"}` to a policy-carrying WASM
  service therefore ran every `get`/`query` in that invocation completely
  unfiltered -- no RLS, no CLS -- with no capability required, since the
  WASM ingress admits anonymous callers by design. Confirmed by tracing
  the call chain end to end (`dispatch.rs` → `execute_wasm_json` →
  `prepare_wasm_execution`) and by checking `invoke_lifecycle_hook`'s only
  call site (`deploy_wasm`, host-internal, never reached through
  `prepare_wasm_execution`) -- the method-name branch in
  `prepare_wasm_execution` had no legitimate caller at all. Fixed by
  removing the branch entirely: `prepare_wasm_execution` now always builds
  `CallerContext::service_system` at the ordinary dispatch epoch budget,
  regardless of `method_name`. This also closes the pre-existing,
  FDAE-independent hazard the same inference created (a wire caller
  self-elevating to `data-layer/admin`, gating `execute-ddl`/`query-raw`) as
  a side effect, with no functional loss: `local_elevated` is now
  producible only from `invoke_lifecycle_hook`, exactly as the exempting
  comment already claimed. New test:
  `engine::tests::prepare_wasm_execution_grants_no_elevation_for_init_or_migrate_method_names`,
  confirmed to fail (`left: LocalElevated, right: System`) against the
  pre-fix code by reverting and rerunning.
- **N2 (Medium) — dropping a policy on re-deploy was never restored,
  failing open.** `deploy`'s `fdae_policy_rollback` capture only ran
  `load_fdae_policy` (to remember the previous document for rollback) in
  the branch where the new manifest *declares* a policy; the branch where
  the manifest drops the `fdae` block called `delete_fdae_policy`
  unconditionally and recorded `None` ("nothing to roll back"). A later
  deploy-step failure on a re-deploy that dropped the block therefore left
  the row deleted rather than restoring whatever policy the previous,
  still-running version depended on -- the same "an already-running
  previous version loses its policy to an unrelated failed re-deploy"
  scenario F5's own fix comment already named as the reason to restore
  rather than delete, just reached from the other branch, and failing
  *open* instead of closed. Fixed by capturing `previous_fdae_policy` via
  `load_fdae_policy` unconditionally, before either the save or the delete,
  and rolling back to that captured value symmetrically in both
  directions; `rollback_fdae_policy` and `Option<Option<String>>`
  collapsed to `Option<String>` since a rollback target now always exists.
  New test: `test_deploy_failure_restores_a_policy_the_new_manifest_dropped`
  (deploys a policy, re-deploys dropping the `fdae` block with a WASM
  source that then fails, asserts the original policy is restored, not left
  deleted), confirmed to fail against the pre-fix code by reverting.
- **F5 residual (Medium) — the failure branch the finding actually named
  was still unrolled-back.** The first post-commit review's F5 fix added
  rollback to `deploy_wasm`'s own failure branch inside
  `deploy_wasm_service`, but `register_wasm_endpoints`'s failure --
  reached *after* `deploy_wasm` already succeeded (compiled/cached the
  component and run its lifecycle hook) -- returned its error via a bare
  `?`/`map_err` with no rollback call at all, leaving both the new config
  generation and the new FDAE policy in force despite the deploy failing.
  `deploy_container_service`'s endpoint-registration loop had the
  identical shape (a failure there also skipped rollback) -- fixed both,
  since leaving the sibling function with the same unrolled-back gap right
  next to this fix would be an obvious, easily-rediscovered inconsistency.
  New test:
  `test_deploy_failure_after_successful_wasm_compile_rolls_back_gen_and_policy`
  (a `FailingEndpointStorage` test double fails `EndpointRegistry::register`
  for one specific interface name, deterministically forcing the failure
  into `register_wasm_endpoints` after a real minimal WASM component has
  already compiled and a new policy has already persisted; asserts both the
  config generation and the FDAE policy roll back to their pre-deploy
  values), confirmed to fail against the pre-fix code by reverting. The
  container-path fix has no equivalent test -- deploying a container
  service successfully needs a real Podman socket, which nothing in this
  test suite provides (no existing test in this file deploys a container
  service at all); the fix is the same one-line-shape change reviewed by
  inspection, not exercised end-to-end.

**Reviewed and disagreed, no code changed:**

- **N3 (Low) — a narrower residual race, and unbounded `fdae_policy_
  generation` growth.** The review's own suggested response was "a comment
  acknowledging it... rather than a redesign," and that's the judgment
  applied here: (1) the generation comparison and the `fdae_policies`
  insert in `resolve_fdae_policy` are still two separate `DashMap`
  operations with no `await` between them, so an eviction landing in that
  now-much-narrower gap is still silently undone -- correctness-equivalent
  to the wide race F4 already closed, just far less likely, and closing it
  fully would mean merging two `DashMap`s behind one lock, a real redesign
  for a race this narrow; (2) `fdae_policy_generation` entries are only
  ever inserted or bumped, never removed on `stop_wasm`, so the map grows
  by one entry per distinct `service_id` the process has ever seen -- real,
  but bounded by service churn (redeploys/undeploys over the node's
  lifetime), not request volume, and not a request-driven leak. Documented
  both directly on the `fdae_policy_generation` field's doc comment rather
  than fixed, matching this file's own established pattern for a genuine,
  low-severity, by-design gap (F8/F10 above).

**Reviewed, already correct, no action needed:**

- **F1, F2, F3, F4, F6, F7 (as fixed in the prior session)** — re-verified
  against current code; still correct.
- **F8, F9, F10** — re-confirmed as already-recorded, accepted conventions;
  no new information this pass.

### Verification evidence (post second-pass review)

- `cargo +nightly fmt --all` — clean.
- `cargo clippy --workspace --all-targets --all-features` — zero warnings.
- `cargo test -p syneroym-sandbox-wasm --lib` — 41 passed, 0 failed (40
  prior + N1's regression test).
- `cargo test -p syneroym-control-plane --lib` — 38 passed, 0 failed (36
  prior, including the separately-committed symlink-hardening tests + N2's
  and F5's regression tests).
- `cargo test --workspace --no-fail-fast` — two isolated failures across
  two separate full runs (`syneroym-router --test native_dispatch_identity`
  once, `syneroym-sandbox-wasm --test messaging_integration`'s
  `test_guest_delivery_latency_budget` once), neither touched by this
  session's diff (engine.rs/orchestration.rs only); both passed cleanly
  when rerun in isolation immediately after, and a third full run completed
  with zero failures -- resource-contention flakes under parallel load,
  the same class already documented for Phase 4's own verification.
  With this CLI's default network sandbox left enabled, the same
  pre-existing/environmental socket-bind failures as every prior phase
  (`coordinator-iroh`, `mqtt-broker`, `sdk`'s `connect_timeout`,
  `substrate`'s HTTP/messaging/stream e2e binaries) reproduce identically;
  confirmed unrelated by the sandbox-disabled runs above.

### Post-commit review, third pass (2026-07-22)

A full-slice inline review of Phases 1-4 together (not a single commit's
diff), independently re-verified finding-by-finding against current code
before addressing. One Critical, eight High, and a page of Medium/Low
findings; disposition below. `C1`/`H1`-`H8` naming is the reviewing
session's own, kept here for continuity with that review rather than
renumbered into this file's `F`/`N` sequence.

**Addressed, code changed (commit 1 — `crates/fdae`, `crates/data_db`):**

- **C1 (Critical) — a case-variant collection name disabled RLS/CLS
  entirely.** `find_definition` matched a query's `collection` string
  against a definition's key/table case-*sensitively*, but SQLite resolves
  unquoted table names case-*insensitively* -- `query("DOCUMENTS")` against
  a policy defining `"documents"` found no definition, took the
  policy-absent "unfiltered" branch, and still hit the real, governed
  table. Fixed by matching case-insensitively (`eq_ignore_ascii_case`),
  with `validate_no_collection_ambiguity` updated to the same
  case-insensitive rule so two definitions can't collide under it. New
  tests: `compile::tests::collection_lookup_is_case_insensitive_like_sqlite`,
  `policy::tests::rejects_a_definitions_table_colliding_case_insensitively`,
  and an end-to-end `data_db` regression
  (`differently_cased_collection_name_does_not_bypass_the_sieve`) proving
  it against real SQLite with a zero-capability caller.
- **H1 (High) — `default:` escalation past its own `allows`.** The
  `default` permission fallback checked only that some held capability
  granted the requested operation, never that `default`'s own `allows`
  covered it -- a caller holding an unrelated (e.g. write) capability could
  ride a read-only default permission's paths through a write-mode check.
  Fixed by gating the fallback on `default_perm.allows` entailing
  `operation`, the same grant∩policy contract every other permission route
  already obeys. New test:
  `compile::tests::default_permission_not_covering_operation_is_denied`.
- **H3 (High) — a dotted `fields.deny` entry silently masked nothing.**
  `strip_masked_fields` only ever removes a flat top-level JSON key;
  `"profile.ssn"` passed schema validation, `compile_cls` copied it
  verbatim, and the anti-oracle filter guard independently collapsed a
  matching filter key to `"profile"` (`referenced_top_level_fields` splits
  on `.`) -- so neither the mask nor the oracle guard ever matched.
  Fixed at both layers: a policy `fields.deny` entry containing `.` is now
  a parse-time `PolicyError::Semantic` (same "loud error, not a silent
  no-op" treatment `fields.allow` already gets); a capability *caveat*'s
  `fields.deny` (a runtime value, not parse-time checkable) gets the same
  rejection inside `compile_cls`, failing the compile closed instead. New
  tests: `policy::tests::rejects_fields_deny_with_a_dotted_nested_path`,
  `compile::tests::caveat_fields_deny_with_a_dotted_path_fails_closed`.

**Addressed, code changed (commit 2 — `crates/control_plane`,
`crates/sandbox_wasm`):**

- **H4 (High) — unbounded path-hop recursion could abort the process.**
  `compile::emit_chain` recurses once per relation hop in a path with no
  depth guard of its own, and neither the schema nor `policy.rs` capped
  hop count -- a policy author (accidentally or otherwise) could drive a
  path deep enough to blow the Rust stack, a `SIGABRT` that takes down
  every service on the substrate, not just the misconfigured one. Fixed
  with a `MAX_PATH_HOPS = 32` cap in `policy::validate_path` (rejected at
  parse time, before any query ever compiles against the policy) and a
  matching `maxItems: 33` on the schema's `paths` item, kept as two
  independent gates since `Policy`'s public fields let a caller construct
  one bypassing `parse_and_validate` entirely (see Medium items, not
  itself closed this pass). New tests:
  `rejects_a_path_exceeding_the_max_hop_count_via_schema` and
  `..._at_the_semantic_layer_too` (the latter calls `validate_semantics`
  directly on a hand-built `Policy`, proving the semantic gate holds
  independently of the schema one).
- **H6 (High) — the TCP deploy arm had no FDAE rollback.**
  `deploy_wasm_service`/`deploy_container_service` both take
  `previous_fdae_policy`/`new_gen` and roll back on failure;
  `deploy_tcp_service` took neither and let `registry.register`'s error
  propagate bare -- a failed TCP redeploy left the new policy persisted
  and the config generation bumped, same shape H1/H2's rollback gaps had
  already closed for the other two arms. Fixed by giving it the identical
  parameters and rollback calls. New test:
  `test_deploy_tcp_endpoint_registration_failure_rolls_back_gen_and_policy`
  (reuses the existing `FailingEndpointStorage` fixture to force the
  failure deterministically).
- **H7 (High) — rollback restored the DB row but never invalidated the
  WASM engine's policy cache.** `rollback_fdae_policy` only touched
  `storage_provider`; a failed `deploy_wasm_service` attempt can reach it
  *after* `compile_and_cache_wasm`/`resolve_fdae_policy` already cached
  the new (about-to-be-rolled-back) policy, leaving the engine serving it
  for the rest of the process's uptime while storage says otherwise. Fixed
  by having `rollback_fdae_policy` also call `app_sandbox_engine.
  stop_wasm(service_id)` -- its cache-eviction side effect, safe to call
  unconditionally since it no-ops for a `service_id` the engine never
  cached anything for (the TCP/container rollback paths). Not covered by a
  new automated assertion: `AppSandboxEngine`'s resolved-policy cache is a
  private field of a different crate, so nothing outside `sandbox_wasm`
  can observe eviction directly without a real data-layer-touching WASM
  fixture exercising the difference end to end, which is a materially
  larger undertaking than this fix; the underlying `stop_wasm` eviction
  mechanism itself is independently covered by
  `engine::tests::fdae_policy_cache_evicted_on_stop_wasm_and_recompile`,
  and the full workspace suite (including that test) stayed green with
  this change in place.
- **H8 (High) — a transient storage error was cached as "no policy."**
  `resolve_fdae_policy`'s `Err` branch (a storage read failure, e.g. one
  `SQLITE_BUSY`) collapsed to `None` and was cached exactly like a
  genuine absence, silently disabling FDAE for the service until the next
  redeploy over what may be a one-off blip -- in contrast to the adjacent
  generation-race branch, which already declines to cache an uncertain
  read. Fixed by returning uncached (an early `return None`, skipping the
  `fdae_policies.insert`) on a storage error specifically, leaving the
  malformed-policy-in-storage case (a different, genuinely
  fail-closed-absent scenario, per that branch's own doc comment)
  unchanged. New test:
  `engine::tests::fdae_policy_transient_storage_error_is_not_cached`
  (a `FlakyStorageProvider` fixture fails `load_fdae_policy` exactly
  once, then succeeds; asserts the first call resolves `None` uncached
  and a retry resolves and caches the real policy).

**Reviewed and disagreed on remediation shape, code changed differently
than proposed (H2):**

- **H2 (High, review's framing) — "platform-ability grants select every
  covering branch; `default:` is never consulted."** The underlying
  mechanism is real and reviewed as such: a capability scoped to a
  platform ability (not a named `app/<type>.<permission>` grant) is
  admitted through *every* permission whose `allows` covers that ability
  (`applicable_permissions` ORs them together), so an unconditionally
  public sibling permission (`paths: []`) silently widens a
  path-restricted one sharing the same ability. But this is ADR-0017's own
  resolved, tested design (the direct route for a platform-ability
  capability), not a compiler bug -- fixing it in `applicable_permissions`
  would abandon the grant∩policy intersection contract entirely and break
  the documented entailment case (a write-capable grant also satisfying a
  read check, `write_capable_permission_also_covers_a_read_check`). The
  review's own framing ("`default:` is never consulted") is also
  imprecise: `default` is a separate fallback, reachable only when *no*
  permission's `allows` covers the operation at all, and this finding
  doesn't route through it. Addressed two ways instead of a compiler
  change: (1) an additive, warn-only author-time lint,
  `warn_on_ambiguous_public_permission`, alongside `strict:`'s own
  deploy-time check, flagging exactly this shape (public + restricted
  permissions sharing a covering ability with no `includes` link) so an
  author can link them or scope capability issuance to the named
  permission instead; (2) ADR-0017's default-permission bullet tightened
  to state explicitly when it's consulted and that it never overrides
  what other permissions grant, plus a new bullet recording this trade
  as a deliberate decision rather than an oversight. New test:
  `service::orchestration::tests::test_warn_on_ambiguous_public_permission`
  (fires on an unlinked public/restricted pair sharing an ability, silent
  when `includes`-linked, silent when abilities are disjoint).

**Reviewed and confirmed, not yet addressed (open):**

- **H5 (High) — the recursive CTE's `UNION ALL` plus a non-unique
  `from_key`/`to_key` join column lets row count blow up combinatorially
  (branching factor `b`, depth-64 bound → up to `b^64` rows) instead of
  being deduplicated, since `MAX_RECURSION_DEPTH` bounds path *length*,
  not row count.** Confirmed structurally (the CTE's `UNION ALL` and the
  guest-writable, non-unique join columns are both real), but the review's
  own suggested fix -- swap to plain `UNION` -- almost certainly does not
  work as stated: the CTE's rows carry `depth` and `seen` (the full
  visited-path string), so two branches reaching the same node rarely
  produce byte-identical tuples for `UNION`'s dedup to collapse. A real
  fix needs the recursion restructured to dedupe on visited `id` (or
  `id`+shortest-`depth`) independent of path, not a one-keyword swap --
  logged here as open rather than attempted as part of this pass. The
  `FDAE_MAX_VM_OPS` progress-handler watchdog (`install_watchdog`, wraps
  every sieved query including the recursive-CTE ones) does bound
  worst-case compute per query today, so the practical impact is
  reader-pool resource exhaustion under concurrent abuse of a
  guest-writable relation, not a true unbounded hang -- lower urgency than
  it would be without that backstop, but still open. Track as a follow-up
  before this compiler shape is relied on for a policy with guest-writable
  recursive relations at any real scale.
- **Medium/Low findings from the same review, not yet addressed:** `check_
  access`'s no-sieve path ignores `operation` (`do_check_access`,
  `sqlite.rs`); `delete_many` lacks the CLS anti-oracle predicate guard
  `do_query` has (`do_delete_many`, `sqlite.rs`); `drop-collection`/
  `create-collection` carry no `data-layer/admin` capability gate while
  `execute-ddl`/`query-raw` do (`synsvc_native.rs`); the path-guard TOCTOU
  in `reject_path_escape` (computes `resolved`, then reads the original
  relative path); a non-object payload fails an entire query page instead
  of just that record (`host_capabilities.rs`); no size bound on a policy
  document before it's read/persisted/re-parsed per cache miss
  (`orchestration.rs`); `Policy`'s public fields/public `Deserialize` let
  a caller bypass `parse_and_validate`'s schema+semantic gates entirely
  (the residual H4's defense-in-depth fix above is deliberately guarding
  against); `ResourceUri::service(service_id, service_id)` in `compile.rs`
  diverges from the workspace's `app_instance.unwrap_or(service_id)`
  convention used elsewhere. Recorded here rather than silently dropped;
  none attempted this pass.

### Verification evidence (post third-pass review)

- `cargo +nightly fmt --check --all` — clean.
- `cargo clippy --workspace --all-targets --all-features` — zero warnings.
- `cargo test -p syneroym-fdae` — 50 passed, 0 failed.
- `cargo test -p syneroym-data-db` — 133 passed, 0 failed.
- `cargo test -p syneroym-control-plane` — 41 passed, 0 failed.
- `cargo test -p syneroym-sandbox-wasm` — 7 (FDAE-cache-specific) + full
  crate suite passed, 0 failed.
- `cargo test --workspace` — clean except the same pre-existing,
  environmental `coordinator-iroh::connection_limit` socket-bind failure
  (`Failed to bind server socket to 127.0.0.1:0: Operation not permitted`)
  every prior phase's verification has already recorded as sandbox-caused,
  not code-caused.

## Phase 5 — Decision Trace, Bench, Failure/Security Matrix, Gate ✅ (2026-07-22)

Branch: `feat/m04b-slice-b2-data-db` (same branch/PR as Phases 1-4). Plan:
[slice-b2-implementation-plan.md](slice-b2-implementation-plan.md) §10, §11,
§13 item 5. `crates/fdae`'s `Policy`/`compile_read`/`CompiledSieve` and
`crates/data_db`'s `QueryAuth`/`do_check_access` are unchanged ground truth
going in, extended (not restructured) by this phase.

### What was delivered

- **Decision trace** (ADR-0017 §9) — a new `fdae::DecisionTrace` struct
  (`crates/fdae/src/trace.rs`): `tier` (always 3), `held` (the evaluated
  grants, `<resource>::<ability>`), `operation_admitted`,
  `applicable_permissions`, `compiled_predicate`, `rows_reached` (Mode A
  only — `None` at compile time, since `compile_read` never executes SQL),
  `path_failed`, `caveats_applied`. `CompiledSieve` gained a `trace: 
  DecisionTrace` field so every caller already holding a compiled sieve can
  see the same trace `compile_read` logged. `compile_read` builds and
  `tracing::info!`/`debug!`s one at every return point (`info` on a deny,
  `debug` on an allow) — the strict-unknown-collection early return, the
  no-applicable-permission-and-no-default early return, and the main body
  (detecting a claim-absent deny by the literal `"0=1"` string
  `compile_permission` returns only from that one path). `do_check_access`
  (`data_db/src/sqlite.rs`) clones `sieve.trace` after actually running the
  Mode A predicate, fills in `rows_reached`, and — the one deny reason
  `compile_read` cannot know at compile time — sets `path_failed` when an
  admitted operation's predicate still matched no row, then emits a second,
  execution-aware trace.
- **Criterion bench** (`crates/data_db/benches/fdae_bench.rs`, wired into
  `Cargo.toml`) — the task.md perf-budget row: FDAE pushdown `query` (Mode
  B), single-hop ReBAC, 100 seeded records (50 visible/50 excluded, so the
  bench does real row-pruning work), end to end through the real
  `ServiceStore` against real SQLite. Measured **~80 µs** mean, far under
  the 25 ms p99 budget — no sign of H5's recursive-CTE blowup (this shape is
  non-recursive, single-hop, so H5 doesn't apply here; H5 stays open,
  tracked separately).
- **Failure/Security matrix** (`task.md`) — the table gained a 4th
  "Outcome" column with evidence (test names) for B2's five rows (Mode B
  exclusion, Mode A deny, CLS, cyclic ReBAC, watchdog timeout); the B3 row
  and the three stage-4 rows are marked explicitly deferred (not yet
  implemented) rather than left silently blank. A new "Security review
  findings" table documents the `C1`/`H1`-`H8` third-pass review findings
  (614756f/3df969f) with their fix/status and evidence, including `H5` and
  `H2` as open/differently-addressed rather than silently marked done.
- **`mise run test:e2e`** — run for the first time since Phase 1 (per this
  phase's own scope, since it hadn't been run for Phases 2-4). All five
  `wasm32-wasip2` `test-components` (`greeter`, `data-layer-test`,
  `messaging-pubsub-test`, `stream-test`, `proxy-test`) rebuilt cleanly via
  `cargo component build --target wasm32-wasip2` first, confirming the
  additive Phase 3 WIT change (`check-access`) left the guest-imported
  surface unbroken. Both Playwright configs green: 8/8 (main), 4/4
  (multi-hop) — 12/12 total. This is a regression/compat gate on Phase 5's
  own changes, not step-22 evidence: the harness (`global-setup.ts`) deploys
  only a TCP passthrough service (`miniapp-demo1-web`, `svc deploy --tcp`)
  with no WASM component and no FDAE policy, so it exercises zero FDAE
  code, transitively or otherwise. Step 22's filtering half is proven by
  `native_dispatch_identity.rs`'s
  `native_fdae_policy_row_filters_and_masks_for_two_distinct_verified_callers`
  (Phase 4) instead — same scoping this section already noted for Phases 2
  and 3.
- **`traceability-matrix.md`** — the `[FND-IAM]` (M4B: FDAE) row flipped
  from `Planned` to `In Progress (Slice B2 complete)`, with evidence links
  covering the compiler, store integration, host wiring, deploy plumbing,
  decision trace, and bench, plus explicit call-outs for the two known gaps
  (D-04-02-h, H5) and the three slices (B3/B4-fdae/B5-fdae) still needed
  before this row can flip to `Complete`.

### Tests

- **`crates/fdae`** (`compile.rs`, new `#[test]`s) — one regression test
  per decision-trace deny reason that `compile_read` can determine at
  compile time: `decision_trace_records_operation_not_admitted` (caller
  holds no capability granting the operation at all),
  `decision_trace_records_strict_unknown_collection` (`strict: true`, no
  matching definition), `decision_trace_records_claim_absent` (a
  `conditions` entry whose claim is absent from `session.claims`) — plus
  `decision_trace_records_allow_with_no_path_failed` pinning the non-deny
  shape (`path_failed: None`, `compiled_predicate` equal to the sieve's own
  `where_clause`). All four assert on `sieve.trace` fields directly (the
  same `DecisionTrace` `compile_read` already logged), not on captured
  `tracing` output — `CompiledSieve::trace` makes that the simpler, more
  direct test.
- **`data_db`** (`sqlite.rs`'s existing private `tests` module, new
  `#[test]`) — the fourth deny reason, "rows not reached", is only
  knowable after Mode A actually executes: `decision_trace_records_rows_
  not_reached_after_check_access_executes` builds a real single-hop policy
  and a real `EXISTS(...)` predicate (Bob holding a read capability but not
  being the seeded row's creator), calls `do_check_access` directly under a
  captured `tracing_subscriber` layer (the `test_insecure_mode_warning`
  pattern, `.with_ansi(false)` so the field text is greppable), and asserts
  the emitted line carries `rows_reached=Some(false)` and the "no row
  satisfied the compiled predicate" reason.
- **Unchanged and stays green** — every Phase 1-4 test, plus the two new
  Phase 5 files (`trace.rs`, `fdae_bench.rs`) and the one `CompiledSieve`
  literal-construction test helper (`sqlite.rs`'s `pathological_sieve`)
  updated for the new `trace` field.

### Decisions carried into this phase

- **`CompiledSieve` gained a `trace` field instead of `compile_read`
  gaining a second return value** — every existing call site already holds
  a `CompiledSieve` (or `Option<CompiledSieve>`); a sibling `DecisionTrace`
  return would have meant threading a second value through `data_db`,
  `host_capabilities.rs`, and every test/bench call site for information
  only `do_check_access` (Mode A, post-execution) actually needs beyond
  what `compile_read` already logs. Attaching it to the sieve keeps the
  signature `compile_read` shipped in Phase 1 stable.
- **Claim-absent detection by string match on `"0=1"`, not a new enum
  variant** — `compile_permission` returns that literal in exactly one
  place (an absent `conditions` claim); every other branch builds `"1=1"`
  or an `EXISTS(...)` predicate. Adding a typed reason would have meant
  threading a new return shape through `compile_permission`'s call sites
  for a distinction only the decision trace needs; the string match is
  documented in place and pinned by
  `compile::tests::decision_trace_records_claim_absent`.
- **The bench measures `ServiceStore::query`, not `compile_read` alone** —
  the task.md budget row is explicitly end-to-end ("`criterion` integration
  bench"), so `crates/data_db/benches` (not `crates/fdae/benches`) matches
  both the plan's own suggestion and this workspace's existing bench
  convention (`security_config_bench.rs` benches the store, not the
  crypto primitives in isolation).
- **`traceability-matrix.md` status is `In Progress`, not `Complete`** —
  B2 is done, but B3 (cross-service fetch), B4-fdae (stage-4 ABAC), and
  B5-fdae (write-side Mode A) are unstarted; flipping the milestone-level
  `[FND-IAM]` (M4B) row to `Complete` before those land would misstate the
  requirement's actual coverage.

### Explicitly out of Phase 5 scope (recorded, not silently dropped)

- **B3 cross-service fetch, B4-fdae stage-4 ABAC, B5-fdae write-side Mode
  A** — later slices; the Failure/Security matrix rows naming them are
  marked deferred, not fabricated as passing.
- **H5 (recursive-CTE row-count blowup)** — confirmed open in the
  third-pass review (2026-07-22), explicitly out of this phase's scope per
  the task brief; not attempted. The new bench's single-hop shape doesn't
  exercise the recursive path, so it provides no new evidence either way.
- **A queryable decision-trace API** — ADR-0017 §9 scopes B2 to `tracing`
  emission only ("a queryable trace API is later"); not built.
- **Policy-configurable watchdog budget** — still the interim
  `FDAE_MAX_VM_OPS` constant, unchanged this phase.

### Verification evidence

- `cargo +nightly fmt --all` — clean.
- `cargo clippy --workspace --all-targets --all-features` — zero warnings.
- `cargo test -p syneroym-fdae` — 54 passed, 0 failed (50 prior + 4 new
  decision-trace tests).
- `cargo test -p syneroym-data-db` — 134 passed, 0 failed (133 prior + 1
  new decision-trace test).
- `cargo test -p syneroym-control-plane --lib` — 40 passed, 0 failed
  (unchanged from Phase 4/third-pass; untouched this phase).
- `cargo test -p syneroym-sandbox-wasm --lib --tests` — 71 passed, 0
  failed across the lib (42) and all five integration test binaries (5 +
  2 + 6 + 3 + 13); unchanged this phase.
- `cargo test -p syneroym-router --lib --tests` — 116 passed, 0 failed
  across the lib (71) and all six test binaries, unchanged this phase.
  One run hit the same class of one-off resource-contention flake Phase 4
  documented (`proxy_dispatch`'s
  `guest_self_proxy_data_layer_returns_empty_when_policy_present` failed
  with a WASM-execution error under full-workspace parallel load); reran
  clean twice, both in isolation and as the full `proxy_dispatch` binary
  and the full six-binary `router` suite -- a flake, not a regression (no
  code this phase touches that test's path).
- `cargo test --workspace` — under this CLI's default network sandbox, the
  same class of pre-existing socket-bind failures Phase 4 documented
  recur (`coordinator-iroh`, `mqtt-broker`, `sdk::connect_timeout`,
  `substrate`'s e2e-adjacent integration tests) — none of these crates'
  files were touched this phase. Rerunning with the sandbox disabled
  passed with zero failures (confirmed twice, including the router
  flake's isolated rerun above).
- `mise run test:e2e` — green: 8/8 (`playwright.config.ts`) + 4/4
  (`playwright-multihop.config.ts`), 12/12 total. Required the sandbox
  disabled (the substrate binary binds real ports); this is the CLI
  environment's network restriction, not a code issue -- documented the
  same way prior phases documented the `coordinator-iroh` socket-bind
  class.
- `wasm32-wasip2` — unbroken. All five `test-components` crates
  (`greeter`, `data-layer-test`, `messaging-pubsub-test`, `stream-test`,
  `proxy-test`) rebuilt cleanly via `cargo component build --target
  wasm32-wasip2` before running `test:e2e`; no WIT files changed this
  phase (Phase 5 touched only `crates/fdae`, `crates/data_db`, and docs).

## Slice B3 — Federated FDAE (Cross-Service Parameter Fetch)

### Phase 1 — Anchor stamp (ADR-0015 A5, amended) ✅ (2026-07-23)

Branch: `feat/m04b-slice-b3-anchor`. Plan:
[slice-b3-implementation-plan.md](slice-b3-implementation-plan.md) §2, §7
(D-B3-1/-2/-7), §8 item 1. Self-contained, no-network half of Slice B3 —
`crates/fdae` gains no async/proxy dependency; the cross-service fetch
(pipeline stage 2) is a later phase.

### What was delivered

- **Signed `anchor_did` on `CapabilityToken`** (`crates/ucan/src/token.rs`)
  — `Option<String>`, included in `signing_value()` so a middle service
  cannot rewrite it without invalidating its own signature.
  `CapabilityToken::issue` is unchanged (still issues `anchor_did: None`);
  a new `CapabilityToken::issue_with_anchor` takes the field explicitly for
  the two legitimate shapes: self-declaration (`anchor_did = Some(own
  DID)`) at origination, and unmodified propagation through onward
  delegation.
- **Propagation invariant in `verify_chain`** — enforced inline in
  `granted_capabilities`, per admitted capability: a `Some(a)` anchor must
  be either self-declared (`a == token.issuer_did`) or substantiated by the
  *same* continuity-respecting proof that backs that specific capability
  (`p.audience_did == token.issuer_did && p.anchor_did == Some(a) &&
  p`'s grants cover the capability). Any other value — including an anchor
  inherited from an unrelated sibling proof that never actually authorized
  this capability — is a hard `Err`, aborting the whole chain verification
  (D-B3-7: the anchor is a single chain-wide provenance assertion, not one
  authority claim among many that could be dropped in isolation). A rooted
  capability has no delegation lineage at all, so it can never substantiate
  a non-self-declared anchor. *(Tightened post-review — see below; the
  original cut bound the anchor to "any sibling proof addressed to this
  issuer," which admitted a confused-deputy gap of its own.)*
- **`SessionContext.anchor_did`** (`crates/ucan/src/session.rs`) —
  populated directly as `leaf.anchor_did.clone()` in `from_verified_chain`;
  no derivation walk. `None` for a direct call. Threaded into the real
  request path: `router/src/route_handler/io.rs`'s `build_caller` — the
  only place an inbound UCAN chain becomes a production `SessionContext` —
  now copies `verified.anchor_did` across alongside `capabilities`/`claims`
  (see below; missing initially).
- **`fdae::DecisionTrace.anchor_did`** (`crates/fdae/src/trace.rs`) —
  surfaced unconditionally alongside `subject_did` and included in both the
  `info!`/`debug!` `tracing` emission, so an operator reading a deny/allow
  line can tell whether a decision was made for the caller or for a
  different principal it was proxying for.
- **`anchor` path terminal** (`crates/fdae/src/compile.rs`) — replaces the
  B2-era compile-time stub. Resolves to
  `session.anchor_did.unwrap_or(session.subject_did)` (D-B3-1: a direct
  caller with no distinct anchor *is* the anchor, not a denial).
- **ADR-0015 A5 amendment** (`docs/decisions/0015-ucan-capability-model.md`)
  — dated 2026-07-23 block recording the D-B3-2 decision: the anchor is an
  explicit signed stamp (OAuth Token Exchange / Kerberos S4U pattern), not
  a structural derivation from "audience of the first non-root token" (the
  original A5 wording, which was ambiguous across owner-rooted vs.
  admin-rooted chain shapes). Supersedes that wording in place, following
  the ADR's own prior-amendment convention.
- **Doc-hygiene** — the same stale "audience of the first non-root token"
  wording corrected in `task.md`'s Slice B3 paragraph and
  `access-control-design.md`'s §11 milestone-mapping table (both flagged
  by the plan as needing this update); `task.md`'s Current State Inventory
  corrected to stop listing `route_handler/dispatch.rs`'s `TODO` as an
  FDAE seam (the code already reworded it to `TODO(B7b / post-B7)` and
  disclaims itself as not an FDAE question).

### Post-review hardening (2026-07-23)

An independent implementation review against commit `a8462d4` re-ran fmt/
clippy/test/e2e rather than trusting this file's self-report (all
independently confirmed green, matching the counts below), then reproduced
several issues directly against the shipped code.

**Addressed, code changed (this session, still Phase 1 scope):**

- **The verified anchor never reached a production session** (critical) —
  `build_caller` built `session` from `Default::default()` and merged only
  `capabilities`/`claims` from the verified chain; `verified.anchor_did` was
  never copied. Consequence: `session.anchor_did` was `None` on every real
  request, so the `anchor` terminal always took its `subject_did` fallback
  — a policy written with `anchor` was byte-for-byte identical to one
  written with `caller`, silently, with no test observing it. Fixed with
  one assignment (`session.anchor_did = verified.anchor_did;`); pinned by
  `build_caller_threads_the_verified_anchor_did_into_the_session`, a
  two-hop anchored chain presented end to end through `build_caller`.
- **Anchor inheritance was not bound to the capability it travels with**
  (high) — `validate_anchor` accepted an inherited anchor from *any*
  sibling proof addressed to the issuer, not specifically the proof backing
  the capability being exercised. A service could combine a capability
  obtained from one root/lineage with an anchor obtained from an entirely
  unrelated one. Reproduced: a service holding an admin-root-granted
  `medical` capability and a separately-obtained `user_a`-anchored
  `calendar` capability could self-issue a leaf asserting `medical` under
  `anchor = user_a`, and it verified. Fixed by folding the anchor check
  into the per-capability admission walk (see above); pinned by
  `anchor_inherited_from_an_unrelated_capabilitys_proof_is_rejected`.
- **The proof set sits outside the token signature, so an unsubstantiated
  anchor could be "rescued" post-issuance** (high) — `signing_value()`
  deliberately excludes `proofs` (documented performance tradeoff), so
  stapling an unrelated-but-genuine anchored proof onto an already-signed
  token could flip an anchor claim from rejected to accepted without any
  re-signing. The capability-binding fix above closes this for the
  reproduced shape too: a stapled proof must actually back the specific
  capability being asserted, not merely carry a matching anchor value.
- **The negative anchor-terminal test could not fail for the right
  reason** (medium) — `anchor_terminal_denies_when_the_anchor_is_a_stranger`
  used a `subject_did` that was itself a stranger to the only seeded row,
  so the empty result proved nothing about which field the sieve actually
  bound. Fixed: `subject_did` is now the row's real owner (`alice`), so a
  wrongly-`caller`-bound sieve would leak and the test would catch it.
- **Two load-bearing halves of the propagation invariant were unpinned**
  (medium) — mid-chain enforcement (the check runs on every node via
  `granted_capabilities`' recursion, not only the presented leaf) and the
  continuity clause (a proof addressed to a third party cannot substantiate
  *this* issuer's anchor, regardless of the value it carries) were both
  correct but untested. Added
  `mid_chain_anchor_rewrite_aborts_the_whole_chain_not_just_the_leaf` and
  `continuity_broken_proof_cannot_substantiate_an_inherited_anchor`.
- **The decision trace could not distinguish an anchor decision from a
  caller decision** (medium) — see `DecisionTrace.anchor_did` above; pinned
  by `decision_trace_records_the_anchor_did`.
- **Anchor coverage stopped at Mode B, single-hop, unit level** (medium) —
  added `anchor_terminal_holds_in_point_in_time_mode` (Mode A/
  `check_access`, a boolean allow/deny rather than a missing row),
  `anchor_terminal_holds_across_a_multi_hop_chain` (`emit_chain`'s terminal
  resolution, a separate code path from the single-hop case),
  `anchor_terminal_holds_on_a_recursive_relation` (`emit_fused_recursive`,
  reusing the cyclic eve/frank/mallory manager graph), and
  `crates/data_db/src/tests_fdae.rs`'s
  `mode_b_query_filters_by_anchor_not_by_the_proxying_caller` (the anchor
  reaching real SQL execution through `ServiceStore`, not just the compiled
  predicate string — `tests_fdae.rs` previously never constructed a session
  with a non-`None` anchor at all).
- **Planning-doc identifiers in code, including one in a user-visible
  error** (low) — `policy.rs`'s `accepts_anchor_terminal_at_parse_time`
  comment and `compile.rs`'s `resolve_hops` doc comment dropped their
  slice-ID references (AGENTS.md); `compile.rs`'s remote-relation error
  string ("cross-service relations require B3", surfaced to a policy
  author) reworded to "are not yet supported."

**Recorded, not code changes:**

- **The token wire-format break plan §2.2 asked to be called out was
  unrecorded** (low) — `anchor_did` joining `signing_value()` changes the
  signed payload (`canonicalize_json_value` preserves null-valued keys, so
  `"anchor_did": null` is now part of every signed body), so **no token
  issued before this phase verifies against the code in this branch**. No
  fixtures in the tree are affected, and this is acceptable pre-release
  (no migrations policy), but it is a real break for any externally-saved
  token (e.g. from `roymctl identity issue-grant`) and is called out here
  per the plan's explicit request, not silently absorbed into "the anchor
  field is new."
- **No operator-facing way to mint an anchored token** (low) —
  `apps/roymctl/src/commands/identity.rs`'s `issue-grant` calls
  `CapabilityToken::issue` with no anchor argument or flag; nothing outside
  the unit/integration tests in this repo can produce or consume an
  anchored chain today. Out of this phase's scope (the plan asked only for
  the library API), but a **Phase 4 prerequisite**: the e2e reference
  scenario for steps 22-23 will need a way to issue an anchored grant from
  the CLI.

Verification after the code changes above:
`cargo test -p syneroym-ucan` (56/56), `-p syneroym-fdae` (64/64), `-p
syneroym-data-db` (138/138) — see the updated counts below;
`cargo +nightly fmt --all` clean; `cargo clippy --workspace --all-targets
--all-features` zero warnings.

### Explicitly out of Phase 1 scope (recorded, not silently dropped)

- **The cross-service fetch itself** (pipeline stage 2: `plan_read`/
  `finalize`, the `resolve-relation` WIT export, the `ServiceProxy`
  orchestration seam, timeout→deny, decision-trace provenance) — Phases
  2-4 of the plan.
- **D-04-02-h ingress closure** — `router/tests/proxy_dispatch.rs`'s
  `guest_self_proxy_data_layer_returns_empty_when_policy_present` and
  `sandbox_wasm/tests/data_layer_integration.rs`'s
  `test_deployed_policy_yields_empty_guest_originated_query_d04_02_h`
  still assert today's over-restrictive empty result; flipping them
  requires the orchestration seam (Phase 4, D-B3-4) to actually thread an
  anchor through a real request, not just the token/session mechanism this
  phase adds. Both still pass unchanged.
- **Reference scenario steps 22-23, the federated-fetch perf budget, and
  the Failure/Security matrix row 6 flip** — depend on the cross-service
  fetch (Phase 4/5), not the anchor stamp alone.
- **`traceability-matrix.md`** — left at B2's "In Progress (Slice B2
  complete)"; not updated this phase, since Slice B3 as a whole isn't done
  (Phase 1 of 5).

### Tests

- **`crates/ucan`** (`token.rs`, 10 new `#[test]`s) — chain-shape table:
  `owner_rooted_anchor_propagates_through_two_service_hops`,
  `admin_rooted_anchor_self_stamps_at_first_service_delegation`,
  `three_hop_pass_through_anchor_survives_every_hop`,
  `direct_grant_with_no_anchor_leaves_session_anchor_did_none`. Attack
  cases: `middle_service_rewriting_anchor_to_an_undelegated_principal_is_rejected`
  (hard `Err`), `self_declared_downgrade_to_acting_as_self_is_accepted`,
  `anchor_did_tamper_after_signing_fails_signature_verification` (signature
  covers `anchor_did`),
  `anchor_inherited_from_an_unrelated_capabilitys_proof_is_rejected` (the
  post-review capability-binding fix),
  `mid_chain_anchor_rewrite_aborts_the_whole_chain_not_just_the_leaf`,
  `continuity_broken_proof_cannot_substantiate_an_inherited_anchor`.
- **`crates/router`** (`route_handler/io.rs`, 1 new `#[test]`) —
  `build_caller_threads_the_verified_anchor_did_into_the_session`: a real
  two-hop anchored chain presented end to end through `build_caller`,
  asserting `CallerContext.session.anchor_did`.
- **`crates/fdae`** (`compile.rs`, 7 new `#[test]`s) —
  `anchor_terminal_filters_by_the_original_principal_not_the_caller`
  (a proxying caller's `subject_did` differs from its `anchor_did`; the
  sieve filters by the anchor), `anchor_terminal_falls_back_to_subject_did_when_anchor_is_absent`
  (D-B3-1), `anchor_terminal_denies_when_the_anchor_is_a_stranger`
  (discriminating: `subject_did` is the row's real owner, so a
  wrongly-`caller`-bound sieve would leak), `decision_trace_records_the_anchor_did`,
  `anchor_terminal_holds_in_point_in_time_mode`,
  `anchor_terminal_holds_across_a_multi_hop_chain`,
  `anchor_terminal_holds_on_a_recursive_relation`.
- **`crates/data_db`** (`tests_fdae.rs`, 1 new `#[test]`) —
  `mode_b_query_filters_by_anchor_not_by_the_proxying_caller`: the anchor
  terminal reaching real SQL execution through `ServiceStore`, not just the
  compiled predicate string.
- Every `SessionContext` struct literal enumerating all fields explicitly
  (rather than using `..Default::default()`) needed `anchor_did` added to
  compile: `crates/fdae/src/compile.rs` (2 sites), `crates/data_db/src/tests_fdae.rs`,
  `crates/data_db/benches/fdae_bench.rs`, `crates/data_db/src/sqlite.rs`
  (2 sites) — no behavior change, all were already covered by existing
  tests that continue to pass.

### Verification evidence

Final, post-review-hardening numbers (superseding the pre-review figures the
first draft of this entry cited):

- `cargo +nightly fmt --all` — clean.
- `cargo clippy --workspace --all-targets --all-features` — zero warnings.
  Two warnings surfaced and were fixed during this phase: a
  `doc_lazy_continuation` in `issue_with_anchor`'s doc comment (missing
  blank line before its trailing paragraph), and a `collapsible_if` in the
  post-review anchor-substantiation check (folded into a single `if let …
  && …` chain).
- `cargo test -p syneroym-ucan` — **56 passed**, 0 failed (46 original +
  7 Phase 1 anchor-stamp tests + 3 post-review: the capability-binding
  attack case, mid-chain enforcement, continuity-clause enforcement).
- `cargo test -p syneroym-fdae` — **64 passed**, 0 failed (57 original + 3
  Phase 1 anchor-terminal tests + 4 post-review: decision-trace anchor,
  Mode A, multi-hop, recursive).
- `cargo test -p syneroym-data-db` — **138 passed**, 0 failed (137 original
  `SessionContext`-literal-only change + 1 post-review: the anchor reaching
  real SQL execution through `ServiceStore`).
- `cargo test -p syneroym-router --lib` — **72 passed**, 0 failed (71
  original + 1 post-review: `build_caller` threading `anchor_did`).
- `cargo test --workspace --no-fail-fast` — every crate this phase touched
  passes 100%: `syneroym-ucan` (56), `syneroym-fdae` (64),
  `syneroym-data-db` (138), `syneroym-control-plane` (45),
  `syneroym-router` (72 lib + 9 + 18 + 4 + 10 + 2 + 2 across its six
  integration binaries, including `guest_self_proxy_data_layer_returns_empty_when_policy_present`
  confirmed still passing unchanged — D-04-02-h ingress (ii) is not closed
  by this phase), `syneroym-sandbox-wasm` (42 lib + 5 + 2 + 6 + 3 + 13
  across its five integration binaries, including
  `test_deployed_policy_yields_empty_guest_originated_query_d04_02_h`
  likewise confirmed unchanged). The only failures are the same
  pre-existing sandbox socket-bind class Phase 4/5 documented —
  `Operation not permitted` / `PermissionDenied` binding a real port under
  this CLI's default network sandbox: `coordinator-iroh`
  (`connection_limit`, `multi_hop_relay`, `tls_rotation`), `mqtt-broker`
  (`no_network_listener_is_bound`), `sdk` (`connect_timeout`), and
  `substrate`'s e2e-adjacent binaries (`basic_lifecycle`,
  `http_passthrough_e2e`, `messaging_client_e2e`, `stream_client_e2e`) —
  none of these crates were touched this phase.
- `mise run test:e2e` — not run this phase. Phase 1 has no e2e-visible
  behavior: no WIT change, no orchestration wiring, no `wasm32-wasip2`
  rebuild needed (`crates/fdae` and `crates/ucan` are host-only, per
  ADR-0015's own implementation notes). The plan's e2e reference-scenario
  steps (22-23) are Phase 4/5 deliverables, gated on the cross-service
  fetch existing.

### Phase 2 — Two-phase compile (`plan_read`/`finalize`) ✅ (2026-07-23)

Branch: `feat/m04b-slice-b3-fetch`. Plan:
[slice-b3-implementation-plan.md](slice-b3-implementation-plan.md) §1.1, §8
item 2. Pure `crates/fdae` work — no async, no `ServiceProxy` dependency
(plan §1.1's "keep `crates/fdae` async-free and proxy-free" decision),
exactly the split the plan calls for: `plan_read` produces either a
finished local sieve (the B2 case) or a `PendingSieve` plus the
`RemoteFetch`es it needs; `finalize` binds fetched id-sets back in.

#### What was delivered

- **`ReadPlan`/`RemoteFetch`/`FetchResult`/`PendingSieve`/`FetchSlot`**
  (`crates/fdae/src/compile.rs`) — the plan's own sketch, implemented
  close to the letter: `RemoteFetch{service, relation, principal_did,
  slot}` (`principal_did` is always the **anchor**
  `session.anchor_did.unwrap_or(subject_did)`, never the path's own
  declared terminal word -- the confused-deputy defense holds regardless
  of whether a remote hop's path says `caller` or `anchor`); `FetchResult{
  slot, ids}`; `PendingSieve` opaque outside the module (only `finalize`
  reads it).
- **`plan_read`** — `compile_read`'s exact body, refactored to thread a new
  `FetchCtx` (fetches collected so far, deduped per distinct `(service,
  relation)` pair per plan §5, plus the SQL-text markers standing in for
  each occurrence's eventual `IN (...)` list) through `compile_permission`/
  `compile_path`/`emit_chain` alongside the existing `params: &mut
  Vec<Value>`. Zero behavior change for a fully-local policy: when
  `fetch_ctx.fetches` stays empty, `plan_read` returns exactly the
  `CompiledSieve` `compile_read` always built, byte for byte -- confirmed
  by every pre-existing `compile::tests` test passing unmodified.
- **`compile_read` becomes a thin wrapper** — calls `plan_read`, and errors
  (`PolicyError::Semantic`) if any fetches are needed, since `compile_read`
  is the synchronous/local-only entry point B2 shipped and has no way to
  perform a fetch itself. A caller needing to resolve a policy with remote
  relations must call `plan_read`/`finalize` directly. This preserves
  `remote_relation_fails_closed_at_compile_time`'s pinned `Err` outcome
  (renamed `compile_read_fails_closed_when_a_remote_fetch_is_needed` for
  accuracy: the *reason* changed from "remote relations unsupported at
  all" to "compile_read specifically can't resolve one", not the
  fail-closed behavior itself).
- **`emit_remote_terminal`** (new, `compile.rs`) — the terminal-hop
  compilation for a remote relation: `{col_expr} IN (<fetch marker>)`
  instead of local `EXISTS (SELECT ... FROM target_table)`, since there is
  no local table to join through. Registers the fetch (or reuses an
  already-registered one for the same `(service, relation)`) via
  `FetchCtx::register`, which returns a unique per-occurrence text token
  (`@@FDAE_FETCH_<slot>_<occurrence>@@`) -- unique per occurrence, not per
  slot, since the *same* remote relation can be reached by multiple OR'd
  permission paths at different text positions, each needing its own
  `replacen` target.
- **`finalize`** — walks `PendingSieve`'s markers in ascending
  `params_index` order (the position in the flat `params: Vec<Value>`
  sequence each marker's id-set belongs, captured at plan time), replacing
  each marker's token with a bound `?, ?, ...` list (or the literal `NULL`
  for an empty id-set -- `IN ()` is invalid SQL, `IN (NULL)` is valid and
  always false) and splicing the corresponding `Value::Text` entries into
  `params` at the right offset, tracking a cumulative `shift` so a later
  marker's insertion point accounts for every earlier one's effect on the
  vector. Fails closed (`PolicyError::Semantic`) on a missing `FetchResult`
  for a slot the plan actually needs, and on an id-set exceeding the new
  `pub const MAX_FETCH_IDS: usize = 1000` cap (plan §5 fan-out
  containment; matches `data_db`'s existing `MAX_QUERY_PAGE_SIZE`) --
  never silently truncates.
- **Schema: a remote relation may now also declare `join_column`**
  (`crates/fdae/src/policy.rs`) -- required by design, not optional: every
  other hop shape in this compiler needs to know which *local* column
  correlates to the target, and a bare `{target, service}` remote relation
  (accepted, but always fail-closed, since B2) had no way to say that. A
  join-based relation (`join_column` set, optionally paired with `service`
  for a remote target) and a recursive self-join (`from_key`+`to_key`)
  remain the only two shapes, mutually exclusive; `validate_relation_shape`
  changed from "exactly one of {local, recursive, remote}" to "exactly one
  of {join-based, recursive}", with `service` an orthogonal tag on the
  join-based shape rather than a third exclusive category. **This changes
  previously-passing test semantics**: `rejects_relation_with_two_shapes`
  (asserted `join_column`+`service` together was an error) is replaced by
  `accepts_remote_relation_with_join_column` (now valid) and
  `rejects_relation_with_join_and_recursive_shapes`/
  `rejects_recursive_relation_that_is_also_remote` (the shapes that *are*
  still mutually exclusive); `accepts_remote_relation_target_unresolved_locally`
  gained a `join_column`, with a new sibling
  `rejects_remote_relation_missing_join_column` pinning the now-required
  field. Confirmed with the user before implementing (this session) --
  a deliberate, narrow, pre-release schema tightening of behavior that had
  never actually worked (every remote-relation path failed closed until
  this phase).
- **`resolve_hops`/`policy::validate_path`** -- a remote relation must be
  the *last* hop before the path terminal (enforced at both the
  parse-time semantic-validation layer and defensively again at
  compile-time in `resolve_hops`): a remote hop's fetched id-set answers
  "which of *my* rows are reachable", which is inherently terminal --
  there is no local row on the far side to keep joining through. `Hop`'s
  `target_def` field became `Option<&Definition>` (`None` only for a
  remote hop, which -- by the last-hop invariant -- is only ever the sole
  remaining element `emit_chain` recurses down to); every other read
  fails closed with an "internal: ..." `PolicyError::Semantic` rather than
  panicking, matching this module's existing defensive style
  (`emit_fused_recursive`'s own `target_def` resolution got the same
  treatment, since a recursive relation is now also structurally
  guaranteed non-remote).
- **D-B3-5 (remote-inside-recursive) guard** -- confirmed already
  structurally impossible: `validate_relation_shape`'s "join-based XOR
  recursive" rule means a single `Relation` can never carry both
  `service` and `recursive: true`. Pinned by
  `policy::rejects_recursive_relation_that_is_also_remote` (schema layer)
  and `compile::remote_and_recursive_on_the_same_relation_cannot_reach_plan_read`
  (confirms no `Policy` value bypassing `parse_and_validate` could reach
  `plan_read` with the combination either).

#### Tests

`crates/fdae/src/compile.rs` (new): `compile_read_fails_closed_when_a_remote_fetch_is_needed`
(renamed/re-asserted, see above), `plan_read_collects_a_remote_fetch_instead_of_failing_closed`
(the fetch's `principal_did` is the anchor, not the proxying `subject_did`),
`plan_read_of_a_fully_local_policy_has_no_fetches` (B2 shape preserved),
`finalize_binds_the_fetched_id_set_and_runs_correctly` (real SQL execution
against seeded rows, not just string assertion),
`finalize_binds_an_empty_id_set_as_in_null_not_invalid_sql`,
`finalize_rejects_an_oversized_id_set`, `finalize_fails_closed_on_a_missing_fetch_result`,
`remote_and_recursive_on_the_same_relation_cannot_reach_plan_read`,
`plan_read_dedupes_repeated_fetches_to_the_same_remote_relation`.
`crates/fdae/src/policy.rs` (new): `accepts_remote_relation_with_join_column`,
`rejects_relation_with_join_and_recursive_shapes`,
`rejects_recursive_relation_that_is_also_remote`,
`rejects_remote_relation_missing_join_column`.

#### Explicitly out of Phase 2 scope (recorded, not silently dropped)

- **The actual fetch** -- resolving a `RemoteFetch.service` to a DID,
  issuing the `ProxyRequest`, timeout→deny, `DecisionTrace` provenance for
  a successful fetch. Phase 4's orchestration seam.
- **The WIT `resolve-relation` wire method + host impl** -- Phase 3
  (below).
- **`anchor`/`caller` terminal resolution downstream of a remote hop** --
  not applicable: a remote hop is always the path's terminal-adjacent
  step (the new last-hop invariant), so there is no "downstream" local
  hop after it to resolve a terminal against in this phase.

### Phase 3 — `resolve-relation` native method (D-B3-3, the receiving side) ✅ (2026-07-23)

Branch: `feat/m04b-slice-b3-fetch` (same branch/PR as Phase 2). Plan:
[slice-b3-implementation-plan.md](slice-b3-implementation-plan.md) §3.2, §8
item 3, with D-B3-3 resolved via a session confirmation (this conversation,
2026-07-23) rather than picked unilaterally -- see "Decisions" below.

**Scope note, a deliberate narrowing from the plan's literal phase-list
wording ("WIT `resolve-relation`"):** confirmed with the user before
implementing. `dispatch_json_rpc_once`'s routing (`crates/router/src/route_handler/dispatch.rs`)
only ever reaches a WASM-hosted service's *guest-exported* functions for an
external caller (`data-layer-guest` exports only `init`/`migrate`; `store`
is guest-*imported*, callable only from inside the guest's own execution,
never from outside) -- so a cross-service fetch's receiving end can only
ever land on a **native** `data-layer` service (`SynSvcNativeService`).
Adding `resolve-relation` to the WIT `store` interface would add a
guest-introspection surface nothing in B3 consumes, since no external
caller can reach it that way. Native-only, no WIT/`wasm32-wasip2` change
this phase.

#### What was delivered

- **`Definition.resolvable_without_capability: bool`** (`crates/fdae/src/policy.rs`
  + `schema/fdae-v1.json`, `#[serde(default)]`) -- D-B3-3's authorization
  fork, resolved this session: **A1** (reuse the existing capability-gated
  sieve via `ServiceStore::query`, requiring the anchor to hold a real
  capability on *this* service -- zero new authorization surface) is the
  default; **A2** (a bare `principal_column` match, gated only by the
  requesting identity's re-verification, no capability needed) is an
  explicit **per-definition** opt-in, matching every other FDAE
  trust-boundary declaration's own granularity (`principal_column`,
  `fields.deny`, `strict` are all per-`Definition`/`Permission`, never a
  substrate-wide flag). **Mutually exclusive per request, not a fallback
  chain**: A2 applies only when the caller holds *zero* capabilities at
  all (A1 was never attempted), so a real A1 deny (a capability that
  grants nothing on this specific resource) can never be second-guessed
  by the looser A2 model.
- **`resolve_structural`/`StructuralQuery`** (`crates/fdae/src/compile.rs`)
  -- the A2 primitive: a raw `<principal_column> = ?` predicate, reusing
  the same reserved-column-vs-JSON-payload addressing every other
  predicate in this compiler uses (a `principal_column` of `"creator_id"`
  resolves to the physical column, not `json_extract(payload,
  '$.creator_id')`). Pure, no capability check of its own (it has no
  `SessionContext` to check one against) -- the caller gates its use on
  `resolvable_without_capability` and zero capabilities.
- **`definition_table`** (`compile.rs`) -- resolves a `relation` string
  (policy definition key *or* table, case-insensitively, matching
  `find_definition`) to the definition's physical table. Needed because
  `ServiceStore::query`/`query_raw` address a collection **literally**
  (unlike `compile_read`'s own permissive key-or-table matching) -- passing
  a definition key that isn't also the table's own name would otherwise
  spuriously fail `collection-not-found` (caught by this session's own
  test failures before shipping, see "Post-implementation fixes" below).
  Doubles as the **hard pre-check** that `relation` names a real
  definition at all: unlike an ordinary read, where "no definition" means
  "the grant layer already admitted this, run unfiltered"
  (`compile_read`'s `Ok(None)`), a cross-service relationship ask has no
  such backing admission, so an unrecognized `relation` must deny, never
  fall through to `ServiceStore::query`'s ordinary unfiltered pass-through.
- **`RelationshipProof` + `sign_relationship_proof`**
  (`crates/control_plane/src/synsvc_native.rs`) -- the signed, TTL'd
  record ADR-0017 §6 specifies: `{asserter_did, relation, principal, ids,
  valid_until_secs, signature}`, signed via `Identity::sign_json` (RFC
  8785 canonicalization, already existed in `crates/identity`) with the
  `signature` field itself zeroed for the signing pass. TTL is a fixed
  `RELATIONSHIP_PROOF_TTL_SECS = 60` (the ADR's own worked example),
  policy-configurable budgeting deferred (no consumer yet -- Phase 4/5's
  cache, D-B3-6, is the first).
- **`SynSvcNativeService.node_identity: Arc<Identity>`** (new field/
  constructor param) -- signing requires the node's own key material,
  which neither `SynSvcNativeService` nor `AppSandboxEngine` held before
  this phase (only `router::proxy::ProxyRouter` did). Threaded from
  `crates/substrate/src/runtime.rs`'s `setup_connection_router` (which
  already holds `secret_key: [u8; 32]` from `setup_identity_and_storage`)
  through a new `secret_key` parameter on `build_route_handler_deps`,
  constructing `Arc::new(Identity::from_bytes(&secret_key))` once and
  passing it into `ControlPlaneService::init` (new trailing param, new
  `node_identity` field) and on into every `SynSvcNativeService::new` call
  at deploy time. New `syneroym-identity` dependency added to
  `control_plane`'s `Cargo.toml`. All ~29 test construction sites (`
  ControlPlaneService::init` in `service.rs`/`orchestration.rs` and its
  three router/coordinator-iroh integration-test call sites;
  `SynSvcNativeService::new` in `router`'s test binaries) updated
  mechanically to pass a fresh `Arc::new(Identity::generate().unwrap())`
  -- zero behavior change to any pre-existing assertion, confirmed by
  every pre-existing test passing unmodified.
- **`resolve_relation` dispatch method**
  (`SynSvcNativeService::dispatch_data_layer`, `"resolve-relation"` /
  `"resolve_relation"`) -- the full receiving-side flow: (1) `principal`
  must equal `invocation.caller.session.subject_did` -- the router has
  already re-verified whatever proof this request carried into
  `invocation.caller` (identical to every other native-dispatch method),
  so that identity is the only trustworthy source of "who is asking";
  `principal` is a caller-declared label that must match it, never a free
  parameter letting a verified caller ask about an arbitrary third
  party's relationships; (2) no policy deployed, or `relation` names no
  definition (`definition_table` returns `None`) -> an empty, signed
  proof, never an error and never `ServiceStore::query`'s unfiltered
  pass-through; (3) the caller holds capabilities -> **A1**: `ServiceStore::query`
  against the resolved table, `auth = Some(QueryAuth{policy, session,
  service_id})`, `limit = MAX_FETCH_IDS`, and `next_cursor.is_some()`
  (more rows than the cap) -> `QuotaExceeded`; (4) zero capabilities ->
  **A2**: `resolve_structural`, and if the definition hasn't opted in,
  `Ok(None)` -> empty (deny), never a fallback; a structural match runs
  via `store.query_raw` with an explicit `LIMIT {MAX_FETCH_IDS + 1}` (raw
  SQL has no automatic page cap the way `query` does) so an overflow is
  actually observable, not silently truncated.

#### Decisions confirmed this session (D-B3-3)

Two architectural forks, both confirmed with the user before implementing
(not decided unilaterally, since both diverge from a first-glance reading
of the plan doc):

1. **A1-default / A2-opt-in-per-definition**, mutually exclusive per
   request -- see "What was delivered" above. Rejected alternatives:
   a substrate-wide config flag (wrong granularity -- every other FDAE
   trust knob is per-`Definition`/`Permission`) and a fallback chain
   (A1-then-A2, which risks a real A1 deny being silently widened by the
   looser A2 model).
2. **Native-only, no WIT addition** -- see "Scope note" above. Rejected:
   adding `resolve-relation` to the WIT `store` interface for symmetry
   with `check-access`'s Phase-3-B2 precedent, since (unlike
   `check-access`, which a *guest* calls about *itself*) `resolve-relation`
   is answered *to* a remote caller, and no WASM-hosted service is
   externally reachable that way regardless.

#### Post-implementation fixes (found by this session's own tests, before landing)

Both caught by the new `native_dispatch_identity.rs` integration tests
failing on first run, not by inspection -- recorded so the fixes read as
verified, not asserted:

- **A1's `store.query` call used the wire `relation` string directly as
  the collection**, which fails `collection-not-found` whenever a
  policy's definition key differs from its table name (the common case,
  e.g. `"employee"` vs. table `"employees"`) -- `ServiceStore::query`
  addresses a collection literally, unlike `compile_read`'s own permissive
  key-or-table resolution. Fixed by resolving through the new
  `definition_table` (see above) before calling `store.query`.
- **The test double `test_caller()`** (pre-existing, `crates/router/tests/native_dispatch_identity.rs`)
  builds a `CallerContext` with `session: SessionContext::default()` --
  `subject_did` stays empty, unlike `build_caller`'s real production
  behavior (`crates/router/src/route_handler/io.rs`), which always
  populates it. `resolve_relation`'s principal-match check correctly
  rejected every test using it, surfacing the double's incompleteness for
  this new use rather than a bug in the check itself. Added
  `zero_capability_caller` (mirrors `build_caller`'s real shape: a
  populated `subject_did`, no capabilities) instead of reusing
  `test_caller` for `resolve-relation`'s zero-capability test cases.

#### Tests

`crates/fdae` (`compile.rs`, `policy.rs`): `resolve_structural_runs_correctly_against_a_json_payload_principal_column`,
`resolve_structural_addresses_a_reserved_column_directly`,
`resolve_structural_is_none_when_not_opted_in`,
`resolve_structural_is_none_for_an_unknown_relation`,
`definition_table_resolves_by_key_or_table_case_insensitively`,
`parses_resolvable_without_capability_when_declared`.

`crates/router/tests/native_dispatch_identity.rs` (new, driven through real
`RouteHandler::dispatch_json_rpc_once` -- not a hand-called method,
matching this file's own established convention): `resolve_relation_a1_resolves_via_the_capability_gated_sieve_and_verifies`
(also verifies the returned `RelationshipProof`'s signature against its
own `asserter_did` via `syneroym_identity::substrate::verify_json_signature`,
not just that a signature string is present), `resolve_relation_a1_deny_is_not_rescued_by_a2`
(an unrelated capability -- non-empty, but grants nothing on the resource
-- must not trigger the A2 fallback), `resolve_relation_a2_resolves_structurally_with_zero_capabilities`,
`resolve_relation_denies_when_not_opted_in_and_no_capabilities` (zero
capabilities *and* no opt-in -- neither model applies), `resolve_relation_denies_for_an_undeclared_relation_not_unfiltered`
(pins the `definition_table` pre-check specifically: must never leak an
unfiltered dump), `resolve_relation_denies_when_principal_does_not_match_the_caller`,
`resolve_relation_is_empty_when_no_policy_is_deployed`.

#### Explicitly out of Phase 3 scope (recorded, not silently dropped)

- **The calling side of the fetch** -- `resolve_fetches`/orchestration,
  resolving a logical `service` name to a DID via the app-context
  registry, issuing the `ProxyRequest` with `origin: Native`, timeout→deny,
  wiring `plan_read`→fetch→`finalize` into the WASM and native read
  ingresses, `DecisionTrace` provenance for a successful fetch. Phase 4.
- **D-04-02-h ingress closure** -- both pinned empty-result regression
  tests (`proxy_dispatch.rs`'s `guest_self_proxy_data_layer_returns_empty_when_policy_present`,
  `data_layer_integration.rs`'s
  `test_deployed_policy_yields_empty_guest_originated_query_d04_02_h`)
  still pass unchanged; closing (ii) per D-B3-4 needs Phase 4's real
  anchor-threading, not just the receiving-side primitive this phase adds.
- **Reference scenario steps 22-23, the federated-fetch perf budget
  (< 50 ms p99), the Failure/Security matrix row 6 flip, and
  `traceability-matrix.md`'s update** -- all depend on Phase 4's real
  cross-node fetch existing, not the receiving side alone.
- **D-B3-6 (fetch result caching)** -- the signed, TTL'd `RelationshipProof`
  shape lands now specifically so a future cache is a pure additive follow-
  up with no wire-format churn (plan §3.2's own reasoning); no cache
  itself this phase.

#### Verification evidence

- `cargo +nightly fmt --all` -- clean.
- `cargo clippy --workspace --all-targets --all-features` -- zero warnings.
- `cargo test -p syneroym-fdae` -- **81 passed**, 0 failed (64 Phase-1
  baseline + 15 Phase 2 + 6 Phase 3 new: `resolve_structural`/
  `definition_table`/`resolvable_without_capability` coverage above -- 6
  is the net after also accounting for the Phase 2 rename of one existing
  test).
- `cargo test -p syneroym-control-plane --lib` -- **45 passed**, 0 failed
  (unchanged from Phase-1 baseline -- this phase only added a constructor
  parameter and a new dispatch arm, no new `control_plane`-crate unit
  tests; the integration coverage lives in `router`'s
  `native_dispatch_identity.rs`, per that file's own established
  convention for native-dispatch behavior).
- `cargo test -p syneroym-router --lib --tests` -- **124 passed**, 0
  failed across the lib (72, unchanged) and all six integration binaries
  (`deploy_grant` 9, `native_dispatch_identity` 25 -- 18 baseline + 7 new
  `resolve_relation_*` tests, `proxy_dispatch` 4, `service_ownership` 10,
  `ucan_context` 2, `unsupported_protocol` 2 -- all unchanged from the
  Phase-1 baseline).
- `cargo test -p syneroym-ucan` / `-p syneroym-data-db` / `-p syneroym-sandbox-wasm`
  -- unchanged from Phase-1 baseline (56 / 138 / 42 lib respectively;
  neither crate's source was touched this phase), confirming zero
  regression from the identity-threading plumbing that passed through
  `crates/substrate`/`crates/control_plane`/`crates/router` alone.
- `cargo test --workspace --no-fail-fast` -- the only failures are the
  same pre-existing sandbox socket-bind class every prior phase
  documented, in the identical nine targets: `coordinator-iroh`
  (`connection_limit`, `multi_hop_relay`, `tls_rotation`), `mqtt-broker`
  (`no_network_listener_is_bound`), `sdk` (`connect_timeout`), and
  `substrate`'s e2e-adjacent binaries (`basic_lifecycle`,
  `http_passthrough_e2e`, `messaging_client_e2e`, `stream_client_e2e`) --
  `Operation not permitted`/`PermissionDenied` binding a real port under
  this CLI's default network sandbox, none of these crates touched by
  Phase 2 or 3.
- `mise run test:e2e` -- not run. Phase 2 is pure `crates/fdae` logic;
  Phase 3 adds a native JSON-RPC method with no WIT/`wasm32-wasip2`
  change and no reference-scenario-visible behavior yet (the actual
  cross-service fetch a Playwright spec could observe is Phase 4). Same
  reasoning and precedent as every prior phase's own skip.

### Post-review hardening (2026-07-24)

Independent review against commit `279d284`, delivered as a rendered
findings artifact rather than inline comments. Re-ran fmt/clippy/`cargo
test --workspace`/`mise run test:e2e` independently rather than trusting
this file's self-report before reviewing, and verified several claims by
hand against SQLite directly (`sqlite3` CLI), not just by reading the code.
Nine findings, two blocking, five correctness, two hygiene. All nine were
independently re-verified against the code in this session before being
addressed -- none were pushed back on; the review's reasoning held up in
every case, including two places where my own earlier design summary
(stated to the user before implementing) didn't match what the code
actually did (B3-01, B3-07).

**B3-01 (Blocking) -- `resolve_relation` rejected exactly the request the
planning side builds.** The principal check compared `req.principal`
against `session.subject_did` alone, but `RemoteFetch.principal_did` is
unconditionally the anchor -- B3 exists precisely because `caller !=
anchor`. A forwarded chain `alice -> svc-A` re-verifies on the receiving
node with `subject_did = svc-A` (whoever authenticated the connection) and
`anchor_did = alice`; comparing only against `subject_did` denied every
genuinely cross-service ask, unconditionally. Fixed: compare against
`anchor_did.unwrap_or(subject_did)`, the same fallback
`terminal_value`/`emit_remote_terminal` already use. A1's `QueryAuth`
session also hands `invocation.caller.session` straight through, which
would bind a remote policy's own `caller`-terminal paths to the relaying
connection's identity rather than the principal being asked about --
fixed by evaluating A1 under a session whose `subject_did` is the already-
validated effective principal, leaving the real capabilities on the
connection untouched.

**B3-02 (Blocking) -- sender and receiver disagreed on what `relation`
names.** The sender put `hop.name` (the *local* relation edge name, a key
in the requesting policy's own `Definition.relations`) on the wire; the
receiver resolved that same string through `definition_table`/
`resolve_structural`, which match the *remote's own* object-type keys and
table names -- two different namespaces, silently. An ordinary
`document.owner -> {service: "hr-svc", target: "employee"}` sent
`"owner"`; hr-svc has no definition called `owner`, so it returned an
empty id-set indistinguishable from a legitimate deny. Fixed: the sender
now registers `hop.relation.target` (the remote object type) instead of
`hop.name` -- matches the plan's own "the remote maps logical->physical
with its own `definitions:`" framing, and is the value a remote operator
can actually be told to declare a matching definition for.

**B3-03 (Correctness) -- `IN (NULL)` is `NULL`, not `false`, and inverts
under `NOT`.** `finalize`'s empty-id-set substitution was `{col} IN
(NULL)`, which the doc comment claimed was "always false" -- under
SQLite's three-valued logic it's `NULL`, indistinguishable from `false` in
a bare `WHERE` but **not** inverting under `NOT` (`NOT NULL` is `NULL`,
never `true`). An `exclusion`-operator permission with a remote hop that
legitimately resolves to nobody would deny every row instead of excluding
none. Verified against `sqlite3` directly (`SELECT typeof('x' IN
(NULL))` -- `'null'`). Fixed: the marker now stands for the whole `{expr}
IN (...)` predicate, and an empty id-set substitutes `IN (SELECT 1 WHERE
0)` -- an empty-subquery membership test, unambiguously `false` (also
verified directly: `SELECT 'x' IN (SELECT 1 WHERE 0), NOT (...)` -- `0,
1`). Fails toward over-restriction, never a leak, but a real
wrong-answer path with no prior test coverage.

**B3-04 (Correctness) -- a `caller` terminal on a remote path silently
meant `anchor`.** `emit_remote_terminal` never received the path's
declared terminal word at all, and unconditionally bound the anchor --
correct for *security* (a remote fetch must always resolve against the
original principal), but it meant a policy author writing `["owner",
"caller"]` got `anchor` semantics (the *broader* principal in any proxied
chain) with no error and no warning. The pre-fix dedupe test actually
depended on this silent substitution to produce its result. Fixed:
`policy::validate_path` now rejects `caller` as the terminal of a
remote-relation-terminated path at parse time -- the confused-deputy
argument is exactly the argument for a loud error, not an invisible
rewrite.

**B3-05 (Correctness) -- fetch dedupe dropped the remote object type.**
`FetchCtx::register` deduped on `(service, relation)` where `relation` was
the local edge name; `document.owner -> hr-svc:employee` and
`folder.owner -> hr-svc:employee` (different local names, same remote
type) needed to collapse, but `document.owner -> hr-svc:employee` and
`document.department -> hr-svc:team` (different remote types) did not --
and the old key couldn't tell the two apart. Resolved as a direct
consequence of B3-02's fix: `relation` is now the remote type, so the
existing dedupe key is automatically correct. Pinned by two new tests
(same-type-different-local-name collapses to one fetch;
different-remote-types stay distinct).

**B3-06 (Correctness) -- A2 calls `query_raw` without the capability it
documents itself as requiring.** `ServiceStore::query_raw`'s own doc
comment: "callers must have already verified the `data-layer/admin`
capability." The A2 structural branch reaches it for a caller holding no
relevant capability at all. Not exploitable as written -- the SQL and
every interpolated identifier come from `resolve_structural`, constrained
by the policy schema's `sql_identifier` pattern -- but an undocumented
exception to a stated security contract is exactly the kind of thing that
rots under later edits. Fixed: documented at the call site (why this one
exception is safe) and in `query_raw`'s own trait doc comment (so the
contract stays honest for the next reader, not silently narrower than it
states).

**B3-07 (Correctness) -- A1 vs. A2 was selected by "holds any capability
at all."** `invocation.caller.session.capabilities.is_empty()` meant a
capability for a completely unrelated collection on an unrelated service
routed a caller to A1 (a real-but-irrelevant grant check, empty result)
instead of A2 (which might resolve). The identical principal asking the
identical question with zero capabilities got a different answer than
with an unrelated one -- and this actually diverged from what I'd told the
user the design would do ("zero capabilities *scoped to this remote
service*") before implementing it. Fixed: the fork now checks whether any
capability's `with` covers the resolved resource (or is substrate-scoped),
mirroring `Capability::grants`'s own resource-matching predicate, not
merely whether the list is empty.

**B3-08 (Hygiene) -- a fail-closed `compile_read` left only an allow-shaped
trace.** `plan_read` emits its `DecisionTrace` unconditionally, before its
caller decides what to do with a non-empty `fetches` -- so when
`compile_read` immediately turns that into a hard deny, the only log
record was `operation_admitted: true`, `path_failed: None`, and a
`compiled_predicate` full of raw `@@FDAE_FETCH_...@@` markers. Fixed:
`compile_read` now emits a second, correctly-shaped deny trace naming the
unresolved fetch when it rejects a plan.

**B3-09 (Hygiene) -- a clock failure minted a proof stamped 1970 + 60s.**
`now_secs()` swallowed a `SystemTime` error with `unwrap_or(0)`; any real
clock fault would silently sign a `RelationshipProof` with a `valid_until_secs`
decades in the past instead of surfacing the fault. Safe direction (any
TTL-checking consumer treats it as expired), but a signed artifact
attesting to a claim the node never intended to make is a different class
of problem than an internal bookkeeping field -- fixed by propagating the
error instead of signing a known-bogus timestamp.

**Test coverage added, beyond fixing the nine findings:**
- `plan_read_resolve_relation_finalize_join_end_to_end`
  (`crates/router/tests/native_dispatch_identity.rs`) -- the join the
  review named as the single biggest gap: `plan_read` (real `crates/fdae`
  call) -> `resolve-relation` (real native dispatch against a *second*,
  distinct registered service) -> `finalize` -> real SQLite execution,
  wired by hand since there is no `ServiceProxy` orchestration yet (Phase
  4). This is the test that would have caught B3-01 and B3-02
  immediately; it now pins both fixes at the seam where they were found.
- `finalize_binds_two_distinct_remote_fetches_at_the_correct_offsets` /
  `..._regardless_of_result_order` -- the `params_index + shift` insertion
  arithmetic under two genuinely distinct remote fetches (different
  target types, an `intersection` permission), previously verified only
  by hand-tracing, not a test.
- `finalize_holds_in_point_in_time_mode_over_a_remote_relation` -- plan
  §1.2's "one fetch shape serves both modes" claim, actually run for Mode
  A, not just asserted.
- `finalize_exclusion_operator_with_an_empty_remote_fetch_excludes_nobody`
  -- the B3-03 regression.
- `resolve_relation_a1_overflow_maps_to_quota_exceeded` /
  `..._a2_overflow_maps_to_quota_exceeded` -- 1001-row fan-out over both
  the A1 (`next_cursor`) and A2 (explicit `LIMIT MAX_FETCH_IDS + 1`)
  paths, seeded via `batch-mutate` rather than 1001 individual calls.
- `resolve_relation_an_unrelated_resource_capability_still_gets_a2` /
  `resolve_relation_a1_deny_is_not_rescued_by_a2` (rewritten) -- B3-07's
  fixed fork predicate, both directions: an unrelated-resource capability
  now correctly gets A2 (previously it incorrectly got a real-but-empty
  A1); a same-resource-but-non-covering-ability capability (`blob/read`
  scoped to `employees` -- `data-layer/write` was tried first and
  rejected as a test case, since the `data-layer` namespace's tiered
  hierarchy means `write` actually entails `read`) still correctly routes
  to A1 and is denied there, not rescued by A2.
- `rejects_a_caller_terminal_on_a_remote_relation_path` /
  `accepts_an_anchor_terminal_on_a_remote_relation_path` -- B3-04's parse-
  time rejection.
- `plan_read_does_not_dedupe_fetches_to_different_remote_target_types` --
  B3-05, alongside a strengthened
  `plan_read_dedupes_repeated_fetches_to_the_same_remote_relation` (now
  exercises dedup via two distinct local relation names converging on the
  same remote type, instead of relying on the caller/anchor substitution
  B3-04 removed).
- `compile_read_emits_its_own_deny_trace_when_a_remote_fetch_is_needed` --
  B3-08, using the same `tracing`-capture pattern
  `compile_read_emits_a_deny_via_tracing` already established.
- `definition_table_resolves_by_key_or_table_case_insensitively` -- the
  new helper B3-01/B3-07's fixes both depend on.

Also renamed for accuracy (no behavior change): `compile_read`'s pinned
remote-relation test (`remote_relation_fails_closed_at_compile_time` ->
`compile_read_fails_closed_when_a_remote_fetch_is_needed`, since the
*reason* changed from "unsupported" to "compile_read specifically can't
resolve it") and `finalize`'s empty-id-set test (`..._as_in_null_...` ->
`..._as_a_false_empty_subquery_...`, matching B3-03's fix).

**`crates/router/Cargo.toml`** gained a `rusqlite` dev-dependency (already
deep in the workspace via `data_db`/`fdae`, not previously exposed to
`router`'s own test binaries) -- needed for the join test's final
real-SQL verification step.

Verification after all nine fixes and the new tests above: `cargo
+nightly fmt --all` clean; `cargo clippy --workspace --all-targets
--all-features` zero warnings; `cargo test -p syneroym-fdae` -- **89
passed**, 0 failed; `cargo test -p syneroym-control-plane --lib` -- **45
passed**, 0 failed (unchanged -- this pass touched no new
`control_plane`-crate unit tests, only integration coverage in `router`);
`cargo test -p syneroym-router --lib --tests` -- **158 passed**, 0 failed
across the lib (72, unchanged) and all six integration binaries
(`deploy_grant` 9, `native_dispatch_identity` 29 -- 25 baseline + 4 new:
the join test, the unrelated-resource-capability pin, and the two
overflow tests, `proxy_dispatch` 4, `service_ownership` 10, `ucan_context`
2, `unsupported_protocol` 2 -- all unchanged); `cargo test -p
syneroym-ucan` / `-p syneroym-data-db` / `-p syneroym-sandbox-wasm` --
unchanged from the pre-review baseline (56 / 138 / 42 lib respectively).
`cargo test --workspace --no-fail-fast` -- the same nine pre-existing,
sandbox-environmental targets fail (`coordinator-iroh`'s
`connection_limit`/`multi_hop_relay`/`tls_rotation`, `mqtt-broker`'s lib
tests, `sdk`'s `connect_timeout`, `substrate`'s
`basic_lifecycle`/`http_passthrough_e2e`/`messaging_client_e2e`/`stream_client_e2e`),
identical to every prior phase's own documented list -- nothing new,
nothing in a crate this review's fixes touched. `mise run test:e2e` --
not run, same reasoning as the phase's own entry above: still no
WIT/`wasm32-wasip2` change, still no reference-scenario-visible behavior.
