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

### Per-service signing identity (2026-07-24)

Raised by the user, not the artifact review: `sign_relationship_proof`
signed every `RelationshipProof` under the single node-wide
`node_identity`, so every service co-hosted on one substrate node shared
one `asserter_did` -- contradicting ADR-0017 §6/§7's "`hr-svc` asserts..."
model, which treats each service's assertions as attributable to *that
service*, not to "some service hosted on this node." Multi-tenant hosting
(several unrelated services on one node) is the normal case, not an edge
case, so this was a real gap.

Fixed with the same "Model A: derived, not separately provisioned"
pattern (ADR-0006) `syneroym-data-keystore`'s `derive_instance_kek`
already establishes for per-service DEKs, applied to signing identity
instead of encryption:

- `Identity::derive_service_identity(owner_did, service_id)`
  (`crates/identity/src/keys.rs`) -- HKDF-SHA256 over the node's own
  secret key, domain-separated by `"syneroym:identity:v1:<len>:<owner_did>:<service_id>"`.
  `owner_did`'s byte length is prefixed so the two variable-length fields
  can't be reassigned across their boundary (DIDs contain `:` themselves,
  so a bare `{owner_did}:{service_id}` join would let e.g.
  `owner_did="did:key:zA:x", service_id="y"` collide with
  `owner_did="did:key:zA", service_id="x:y"`).
- `owner_did` is folded into the derivation, not just `service_id`,
  because `service_id` is a reusable string: `ControlPlaneService`'s
  `undeploy` already frees it (`registry.remove_owner`), and a
  *different* owner can later redeploy under the same name
  (`registry.owner_of`/`set_owner` bookkeeping, M04A Slice B7a). Deriving
  from `service_id` alone would hand that new, unrelated owner's service
  the exact signing key the old owner's service had -- letting a stale
  `RelationshipProof` from the old tenancy still verify under the new
  tenant's asserter DID. Keying on both makes an ownership change yield a
  distinct identity even when the name is recycled, while still
  distinguishing co-hosted services under the same owner from each other.
- `SynSvcNativeService` (`crates/control_plane/src/synsvc_native.rs`)
  gained an `owner_did: &str` constructor parameter and derives its
  private `service_identity: Identity` field internally
  (`node_identity.derive_service_identity(owner_did, &service_id)`) --
  callers pass the same `node_identity` and the deploying caller's DID
  they already have in scope; no caller needs to know service-scoping
  happens at all. The production call site
  (`crates/control_plane/src/service/orchestration.rs`'s `deploy`) passes
  `&caller.caller_did`, already in scope there and consistent with what
  gets recorded as the owner a few lines later.
- `sign_relationship_proof` now signs with `self.service_identity`
  instead of `self.node_identity`/a shared `Arc<Identity>`.

**Tests added:**
- `crates/identity/src/keys.rs`: `derive_service_identity_differs_per_owner`
  (same node/service_id, different owner_did -> distinct identity -- the
  redeploy-under-a-new-owner scenario) and
  `derive_service_identity_is_not_ambiguous_across_the_owner_service_boundary`
  (pins the length-prefix fix against the concatenation-collision case
  above), alongside the four pre-existing derivation tests updated for the
  new two-argument signature.
- `crates/router/tests/native_dispatch_identity.rs`:
  `resolve_relation_co_hosted_services_sign_with_distinct_asserter_dids`
  (two services, same node identity, same owner, different `service_id`s
  -> distinct `asserter_did`s, and each proof verifies only against its
  own) and `resolve_relation_service_id_reused_by_a_different_owner_signs_distinctly`
  (same node identity, same `service_id`, different `owner_did` ->
  distinct `asserter_did`s -- the ownership-change scenario). Both go
  through real `dispatch_json_rpc_once` calls against real
  `SynSvcNativeService` instances, not unit-level identity derivation
  alone.

**Verification:** `cargo test -p syneroym-identity` -- **28 passed**, 0
failed (22 baseline + 6 derivation tests, 2 new). `cargo +nightly fmt
--all` clean. `cargo clippy --workspace --all-targets --all-features` --
zero warnings (`SynSvcNativeService::new` picked up an 8th constructor
argument, `#[allow(clippy::too_many_arguments)]` added, matching the same
bare-attribute precedent already used on `ControlPlaneService`'s own
multi-argument constructors). `cargo test -p syneroym-router --test
native_dispatch_identity` -- **31 passed**, 0 failed (29 baseline + 2
new), run twice in isolation to confirm. `cargo test --workspace
--no-fail-fast` -- the same ten pre-existing, sandbox-environmental
targets fail (the nine already documented above, plus this run's parallel
execution additionally tripped the shared `mainline` DHT-actor flake
(`actor thread unexpectedly shutdown: "SendError(..)"`) inside
`native_dispatch_identity` itself rather than `query_raw`'s test this
time -- same panic signature as previously observed on
`admin_caller_admitted_query_raw`, confirmed not a regression by rerunning
the full binary standalone: 31/31 pass). `mise run test:e2e` -- not run;
this change touches only the native `resolve-relation` RPC's internal
signing identity, no WIT/`wasm32-wasip2` change, no reference-scenario-
visible behavior.

**Carried into Phase 4 (recorded as D-B3-8/D-B3-9 in
`slice-b3-implementation-plan.md` §7, not yet resolved):** distinct
per-service asserter DIDs are a guarantee only once the *verifying* side
checks them -- today every verification is self-referential (a proof
verifies against the `asserter_did` embedded in that same proof, so any
signer can self-declare its own DID and pass). Phase 4's `resolve_fetches`
must independently derive the expected asserter for the service/owner it
queried and reject a mismatch (D-B3-8, load-bearing). Separately,
`resolve_relation`'s A1 branch gates on the connection's capabilities
while row-filtering against the anchor -- a no-op today since every test
caller *is* the effective principal, but Phase 4's real service-to-service
proxy will make the connection's identity (the proxying service) diverge
from the anchor's, so which principal's capability must gate A1 needs
settling before the fetch orchestration ships (D-B3-9).

### Phase 4 — Orchestration seam (`resolve_fetches`) ✅ (2026-07-24)

Branch: `feat/m04b-slice-b3-phase4` (new; Phases 1-3 + hardening already
merged to `main` via PR #89/#100). Plan:
[slice-b3-implementation-plan.md](slice-b3-implementation-plan.md) §4, §8
item 4. This is the phase that makes the cross-service fetch *real*: both
read ingresses (WASM host path, native dispatch) now call `plan_read`
themselves, resolve any remote fetch over the real Universal Proxy, and
`finalize` before ever reaching `data_db` -- replacing every prior phase's
hand-wired stand-in.

#### Decisions corrected this session, before implementing

Three of the plan's own recommendations were checked against `main` while
starting this phase and found not to hold as written. All three are argued
in full in `slice-b3-implementation-plan.md` §7 (D-B3-4, D-B3-8, D-B3-10) and
the corresponding ADR/task.md entries; summarized here for the status record.

- **No app-context registry exists (D-B3-10).** ADR-0017 §1 and task.md both
  assert a remote relation's `service:` name "resolves through the
  app-context registry that already exists." It doesn't.
  `crates/app_orchestration`'s `AppRegistry`/`LogicalResolver` is the right
  shape but has only a `StaticInventory` implementation with zero production
  callers. **Not deferred-backlog material** (per the user's explicit
  correction: this is committed platform work, not droppable debt) --
  reserved as its own interstitial between Milestone 4 and Milestone 5
  (`meta-implementation-plan.md`), owned by `app_orchestration`
  (`[PLT-DAP-01]`), not FDAE. Phase 4's interim mechanism: `RemoteFetch.
  service` resolves exactly like `ProxyRequest.target_service` already does
  for every other proxied call -- directly, through the existing
  `EndpointRegistry`/community-registry DID lookup.
- **D-B3-8's literal recipe is cross-node-impossible.** The plan said the
  verifier should independently *derive* the expected `asserter_did` via
  `Identity::derive_service_identity(owner_did, service_id)`. Checked: that
  derivation is keyed on `self.to_bytes()` -- the calling node's *own*
  secret key. A different node can never reproduce it. Fixed by making the
  trust anchor an explicit, policy-declared fact instead of a derivation: a
  remote `Relation` now requires `expected_asserter_did: String` alongside
  `service`, the same category of explicit trust knob as the already-shipped
  `resolvable_without_capability`.
- **D-B3-9 resolves for free.** Forwarding the local invocation's own
  already-verified `CallerContext` (proof intact) as `ProxyRequest.caller`
  for the fetch needs no new mechanism -- `invoke_remote_at` already forwards
  `caller.proof` verbatim for any `CallOrigin::Native` call, so the remote's
  own `verify_chain` naturally re-derives `subject_did`/`anchor_did` and A1
  gates on whatever was legitimately delegated through the real chain.
- **D-B3-4's ingress-(ii) closure doesn't work either (correction, not a
  resolution).** The plan recommended closing D-04-02-h ingress (ii) (guest
  self-proxy) in B3, forwarding `HostState.caller` instead of re-synthesizing
  `service_system` in `proxy::Host::call`'s self-proxy branch. Checked:
  `router/src/route_handler/dispatch.rs`'s `JsonRpcToWasm` branch calls
  `AppSandboxEngine::execute_wasm_json` with **no `caller` argument at all**
  -- the verified caller `dispatch_json_rpc_once` already holds is dropped
  before `prepare_wasm_execution` ever runs. So `HostState.caller` is
  `service_system` for *both* ingresses identically; forwarding it in
  ingress (ii) would forward that same synthesized identity, closing
  nothing. Confirmed with the user: Phase 4 does not attempt the larger
  cross-cut (threading the real caller through `dispatch.rs` →
  `execute_wasm_json`/`execute_wasm_vals` → `prepare_wasm_execution` →
  `HostState`) needed to close *either* ingress for real. Both pinned
  regression tests (`guest_self_proxy_data_layer_returns_empty_when_policy_
  present`, `test_deployed_policy_yields_empty_guest_originated_query_
  d04_02_h`) are unchanged, confirmed still passing.

#### What was delivered

- **`crates/fdae`** -- `Relation.expected_asserter_did: Option<String>`
  (required whenever `service` is set, enforced in `validate_relation_shape`)
  threaded onto `RemoteFetch`; `FetchCtx::register` fails closed if two hops
  reaching the same `(service, relation)` declare conflicting values.
  `trace::RemoteFetchTrace` (service, relation, principal, asserter, TTL) and
  `DecisionTrace.remote_fetches: Vec<RemoteFetchTrace>`; `FetchResult` gains
  a `trace: RemoteFetchTrace` field (the verified provenance the fetching
  side already computed), and `finalize` folds each *distinct slot's* trace
  into the sieve's own `DecisionTrace` (deduped, matching the existing
  fetch-dedup key -- a remote relation reached by two OR'd local paths gets
  one provenance record, not two).
- **`crates/rpc`** (new modules) -- `relationship_proof::RelationshipProof`
  (moved out of `control_plane`, now the shared wire type both the signing
  side and the verifying side use) with `sign`/`verify`; `verify` checks the
  policy-declared `expected_asserter_did` (never the proof's own
  self-declared field) and the TTL, not just the signature. `fdae_fetch::
  resolve_fetches(fetches, caller, proxy)` -- the orchestration seam itself:
  issues each `RemoteFetch` as a real `ProxyRequest{origin: Native, caller:
  caller.clone(), interface: "data-layer", method: "resolve-relation",
  timeout: Some(FDAE_FETCH_TIMEOUT)}`, verifies the returned proof, and
  cross-checks `proof.relation`/`proof.principal` against what was actually
  asked (catches a receiving-side bug or a replayed proof answering a
  different question, which a signature check alone can't). Sequential, not
  parallel, across distinct fetches in one plan -- a scope-recorded perf
  follow-up (`deferred-backlog.md`), not a correctness gap.
- **`crates/data_db`** -- `QueryAuth.resolved_sieve: Option<CompiledSieve>`.
  `None` (every existing call site) preserves the exact Phase 2 behavior:
  `data_db` compiles the sieve itself via `compile_read`. `Some(sieve)` is
  used verbatim, `compile_read` never consulted -- the path a caller who
  already ran `plan_read`+`resolve_fetches`+`finalize` takes.
- **`crates/sandbox_wasm`** -- `HostState::query_auth` replaced by an async
  `resolve_query_auth(collection, operation, mode)`: runs `plan_read`
  itself, and when fetches are needed, resolves them via `resolve_fetches`
  through `self.service_proxy` before building `QueryAuth`. Fails closed to
  `DataLayerError::PermissionDenied` on any fetch error for `get`/`query`/
  `aggregate`/`delete_many`; `check_access` maps that further to `Ok(false)`
  (Mode A's existing convention). **Had to change the receiver from `&self`
  to `&mut self`**: `HostState` holds non-`Sync` WASI internals, so an async
  method taking `&self` that awaits and then uses `self` again afterward
  (to build the final `QueryAuth`, which borrows `&self.caller.session`)
  forces the whole `&HostState` into the generator's state across the
  `.await`, which the WIT-generated `Host` trait's `Send`-future requirement
  rejects -- confirmed by `sample`-ing a hung test process (see below) before
  landing the fix, not just reasoning about it.
- **`crates/control_plane`** -- `synsvc_native.rs`'s `RelationshipProof`/
  `sign_relationship_proof` now delegate to the shared `syneroym_rpc` type.
  `SynSvcNativeService` gains a `service_proxy: Weak<dyn ServiceProxy>`
  field (new trailing constructor param) and the same `resolve_query_auth`
  treatment as `HostState`, wired into `get`/`query`/`delete-many`/
  `aggregate`. `ControlPlaneService` gains a `pub service_proxy:
  OnceLock<Weak<dyn ServiceProxy>>` (mirrors `AppSandboxEngine.service_proxy`
  exactly, for the identical two-phase-construction reason: `ProxyRouter`
  doesn't exist yet when either service is built at substrate startup).
- **`crates/router`** -- `RouteHandlerDeps` gains `control_plane:
  Option<Arc<ControlPlaneService>>` (concrete, alongside the existing
  type-erased `control_plane_service: Arc<dyn NativeService>` used for
  dispatch registration -- kept separate so every existing test double
  substituting a fake `control_plane_service` needed only one mechanical
  `control_plane: None` addition, not a rewrite). `RouteHandler::init` calls
  `.set(...)` on it right where it already does for `AppSandboxEngine`'s.
  `syneroym-control-plane` moved from `router`'s dev-dependencies to real
  dependencies (needed to name the concrete type in production code; no
  cycle -- `control_plane` doesn't depend on `router`).
- **`crates/substrate`** -- `runtime.rs`'s `build_route_handler_deps` passes
  the same `Arc<ControlPlaneService>` for both `RouteHandlerDeps` fields.

#### Tests

- **`crates/fdae`** (3 new): `plan_read_carries_the_policys_expected_
  asserter_did_onto_the_fetch`, `plan_read_fails_closed_when_two_hops_
  disagree_on_expected_asserter_did`, `finalize_records_one_trace_entry_
  per_deduped_slot_not_per_occurrence`. `finalize_binds_the_fetched_id_set_
  and_runs_correctly` extended to assert `sieve.trace.remote_fetches`
  directly, not just SQL row visibility.
- **`crates/rpc`** (new `relationship_proof`/`fdae_fetch` modules, 8 tests):
  sign/verify round trip; rejects a mismatched `expected_asserter_did`;
  rejects a tampered field (signature check); rejects an expired proof
  (independent of signature validity); `resolve_fetches` forwards the
  caller's own context and `CallOrigin::Native` (via a stub `ServiceProxy`);
  denies on a proxy timeout/error, an asserter mismatch, and a mismatched
  relation/principal answer.
- **`crates/data_db`** (2 new): `resolved_sieve_preempts_compile_read_and_
  is_used_verbatim` and its Mode-A sibling -- a stranger session
  `compile_read` would deny outright still reaches every row when
  `resolved_sieve` is supplied, proving it's used as-is, not silently
  re-derived.
- **`crates/sandbox_wasm`** (2 new): `fdae_remote_relation_fetch_succeeds_
  through_host_state` (a stub `ServiceProxy` returning a real signed proof,
  through the real `store::Host::get`) and `..._fetch_failure_denies_closed`
  (a stub erroring -> `DataLayerError::PermissionDenied`, not `Ok(None)`
  masquerading as "not found").
- **`crates/router`** (`native_dispatch_identity.rs`, 3 new, replacing the
  Phase 2/3 hand-wired stand-in): `plan_read_resolve_fetches_finalize_join_
  end_to_end_through_a_real_proxy` -- a real `ProxyRouter` (hr-svc registered
  as a `NativeHostChannel`, no `RouteHandler` needed since `resolve_fetches`
  calls `ProxyRouter::invoke` directly), the full `plan_read` ->
  `resolve_fetches` -> `finalize` -> real SQL join, and asserts the
  successful fetch's `DecisionTrace` provenance (asserter DID,
  `valid_until_secs > 0`) -- not just the deny path. `resolve_fetches_denies_
  when_the_real_proxys_asserter_does_not_match_the_policy` -- a real,
  correctly-signed proof from the *wrong* asserter, rejected through the
  real proxy path (not just `rpc`'s own unit test of `RelationshipProof::
  verify` in isolation). `native_dispatch_denies_closed_on_a_cross_service_
  fetch_failure` -- the native-dispatch-path analogue of the `sandbox_wasm`
  fail-closed tests, for ingress parity.

**A genuine hang, found and fixed before landing.** The two new `router`
tests using a real `ProxyRouter` initially hung indefinitely (confirmed via
`sample` on the stuck process, not just a slow-test guess -- the earlier
`lldb`-blocked-in-sandbox constraint meant `sample(1)` was the right tool).
Root cause: the helper building hr-svc's `SynSvcNativeService` `Box::leak`ed
its `NativeDispatchRegistry`/`TempDir` to satisfy `ProxyRouter`'s lifetime
requirements -- but `SqliteStorageProvider` spawns a `spawn_blocking`
writer-loop task on the *ambient* `#[tokio::test]` runtime, which only exits
once its channel `Sender` (owned transitively through the leaked chain) is
dropped. Leaking it meant the writer thread ran forever, and
`#[tokio::test]`'s own runtime teardown (`BlockingPool::shutdown`) blocks
until every spawned blocking task finishes -- a deadlock. Fixed by returning
owned handles from the helper for the test function to hold until it
returns (normal drop order) instead of leaking.

#### Explicitly out of Phase 4 scope (recorded, not silently dropped)

- **D-04-02-h (both ingresses)** -- stays open, jointly, per the corrected
  D-B3-4 above. Recorded in `deferred-backlog.md` (not silently dropped, and
  the user was explicit this is committed-but-deferred, tracked work, not
  something to quietly skip).
- **Reference scenario steps 22-23, the < 50 ms p99 federated-hop perf
  budget, Failure/Security matrix row 6's ✅ flip, `traceability-matrix.md`'s
  update** -- all Phase 5, per the plan's own phase split. Step 22's
  "…never reaches the WASM guest" half stays open in full (D-04-02-h,
  above).
- **`resolve_fetches` fetch parallelism** -- sequential across distinct
  fetches in one plan; recorded in `deferred-backlog.md` as a perf
  follow-up, not a correctness dependency (mirrors D-B3-6's own
  "correctness over scale" precedent for the fetch-result cache).
- **D-B3-5/D-B3-6/D-B3-7** -- unaffected by this phase, already resolved in
  Phases 1-2.

#### Verification evidence

- `cargo +nightly fmt --all` -- clean.
- `cargo clippy --workspace --all-targets --all-features` -- zero warnings.
- `cargo test -p syneroym-fdae` -- **93 passed**, 0 failed (89 prior + 4 new,
  net after also extending one existing test in place).
- `cargo test -p syneroym-rpc --lib` -- **22 passed**, 0 failed (14 prior +
  8 new: 4 `relationship_proof` + 4 `fdae_fetch`).
- `cargo test -p syneroym-data-db --lib` -- **140 passed**, 0 failed (138
  prior + 2 new).
- `cargo test -p syneroym-sandbox-wasm --lib --tests` -- all green across
  the lib and every integration binary (44 lib, up from 42; all
  pre-existing integration suites, including the D-04-02-h pin in
  `data_layer_integration.rs`, unchanged and still passing).
- `cargo test -p syneroym-control-plane --lib` -- **45 passed**, 0 failed
  (unchanged -- this phase's `control_plane` changes are exercised by
  `router`'s integration tests, per that crate's own established
  convention for native-dispatch behavior).
- `cargo test -p syneroym-router --lib --tests` -- **33 passed** in
  `native_dispatch_identity` (29 prior + 4 new: the two-fetch join test
  replacing the old hand-wired one is a net +1 over the test it replaced,
  plus 2 wholly new), all green in isolation (`--test-threads=1`); under
  default parallel execution, the same pre-existing `mainline` DHT-actor
  flake every prior phase documented (`actor thread unexpectedly shutdown:
  "SendError(..)"`) hit 2-3 unrelated tests, confirmed not a regression by
  the isolated rerun. `proxy_dispatch`/`ucan_context`/`unsupported_
  protocol`/`service_ownership`/`deploy_grant` all green, unchanged.
- `cargo test --workspace --no-fail-fast` -- the same 9 pre-existing,
  sandbox-environmental targets fail (`coordinator-iroh`'s
  `connection_limit`/`multi_hop_relay`/`tls_rotation`, `mqtt-broker`'s lib
  tests, `sdk`'s `connect_timeout`, `substrate`'s `basic_lifecycle`/
  `http_passthrough_e2e`/`messaging_client_e2e`/`stream_client_e2e`), all
  `"Operation not permitted (os error 1)"` binding a real port under this
  CLI's default network sandbox -- identical set, identical error class, to
  every prior phase's own documented list; none of these crates' test files
  were touched this phase.
- `mise run test:e2e` -- run (with the sandbox disabled, needed for real
  port binds), **12/12 green** (8 `webrtc.spec.ts` + 4 `multi-hop.spec.ts`),
  matching the established baseline. Run despite no WIT/guest-visible
  change, since this phase touches `crates/substrate/src/runtime.rs`'s
  startup wiring directly (the new `ControlPlaneService.service_proxy`
  `OnceLock` set call) -- worth confirming the substrate itself still comes
  up cleanly, not just that FDAE behavior is unchanged for these fixtures.

### Phase 5 -- Two-real-substrate e2e, federated-hop perf, matrix/traceability sign-off ✅ (2026-07-25)

Branch: `feat/m04b-slice-b3-phase5`. Plan:
[slice-b3-implementation-plan.md](slice-b3-implementation-plan.md) §8 item 5,
§9. Closes out Slice B3: reference scenario step 23, the timeout/mismatch
deny-closed case, and the federated-hop perf budget, all proven across two
genuinely independent `syneroym-substrate` instances -- not the in-process
`ProxyRouter` Phase 4's own tests already cover.

#### What was delivered

- **`crates/substrate/tests/federated_fdae_e2e.rs`** (new, 672 lines) --
  `federated_fdae_fetch_across_two_real_substrates`, a real two-node
  integration test in the established `crates/substrate/tests/*.rs` idiom
  (each file boots one or more full substrate instances in-process via
  `syneroym_substrate::init`/`run_with_signal`, per `http_passthrough_e2e.rs`/
  `basic_lifecycle.rs`'s own precedent -- see "Decisions" below for why this,
  not a new Playwright scenario, is the right vehicle). A local `Node` helper
  (adapted from `basic_lifecycle.rs`'s own `SubstrateTestContext`, since only
  three of five `crates/substrate/tests/*.rs` files share the one in
  `tests/common/mod.rs`, and this test needs two extra capabilities neither
  copy exposes: an optional shared registry URL and a way to read the node's
  own identity back off disk for computing `expected_asserter_did`) boots:
  - **Node A** (ports 8000/8001/8002) -- self-referential coordinator +
    registry, **owned** (`iam.admin_ucan_root` set to the deploying owner's
    DID -- see "Decisions" below for why this is required, not incidental).
    Hosts an `hr-svc`-equivalent native service (deployed via the real
    `orchestrator/deploy` JSON-RPC call, a `ServiceConfig.fdae_policy`
    `DocumentSource::Inline`) with an `employee` definition declaring
    `resolvable_without_capability: true` (A2).
  - **Node B** (ports 8100/8101/8102) -- its own coordinator role stays up
    but unused; `substrate.registry_url` and `parent_coordinator.iroh` both
    point at Node A's, so cross-node discovery/dialing goes through one
    shared registry and one shared relay (also see "Decisions"). Hosts a
    `documents` service whose policy's `owner` relation names Node A's
    service as remote, trusting the real, independently-computed
    `expected_asserter_did`.
  - Alice (a fresh `Identity`) deploys and owns Node B's service herself,
    self-issuing a root `CapabilityToken` (no delegation chain, ADR-0015 A6:
    an owner-rooted capability needs no node-wide admin) granting herself
    `data-layer/read` on her own `documents` collection, then queries it for
    real over the wire.
- Publishes each deployed app's own `EndpointInfo` into the shared registry
  via a direct `POST {registry_url}/register`, self-signed by the app's own
  identity -- the same pattern `basic_lifecycle.rs`'s own
  `register_app_in_registry` already established (independently confirmed,
  not invented for this phase), needed because `ProxyRouter::invoke_remote`
  resolves a target service by its own registry entry, not by the hosting
  node's.
- A second scenario in the same test proves Failure/Security matrix row 6's
  cross-substrate case: a second Node B app whose policy declares the
  *wrong* `expected_asserter_did` gets a real, correctly-signed proof from
  Node A rejected over the real network hop, surfacing as a hard deny (see
  "Decisions" on why this is an error, not an empty result).

#### Decisions made this session

- **A real two-substrate Rust integration test, not a new Playwright
  scenario.** `crates/substrate/tests/*.rs` already treats one
  in-process-booted full substrate as "a real substrate instance" (see
  `http_passthrough_e2e.rs`'s own doc comment); running two of them in one
  test, each with an independent identity/ports/storage and a real Iroh QUIC
  hop between them, satisfies "≥2 substrates" without inventing a new
  harness. The existing Playwright multi-hop suite tests a different thing
  (WebRTC coordinator relay hops for a browser client), not FDAE, and
  extending it would have meant standing up a WASM miniapp with an FDAE
  policy plus a second coordinator topology from scratch for no proof-value
  this approach doesn't already deliver more directly.
- **Node A must be owned, not left in the default unowned-bootstrap
  posture.** Discovered by a real failing run, not anticipated: on an
  unowned substrate, `build_caller` issues every verified caller a bare
  `substrate:<node_did>` capability for the free `orchestrator/*` abilities
  (M04A B7a F4). `resolve_relation`'s A1/A2 fork (B3-07) treats *any*
  substrate-scoped capability as "holds a capability scoped to this
  resource" regardless of ability, so that free capability always routes to
  A1 -- which then correctly denies (Alice really holds nothing on Node A),
  but for the wrong reason, permanently defeating A2's
  `resolvable_without_capability` path. Fixed in the test by giving Node A
  an explicit `iam.admin_ucan_root`; recorded as its own `deferred-backlog.md`
  entry (existing Phase 3 behavior, not something this phase's own code
  introduced, so not fixed here).
- **Both nodes share one relay, not each its own.** Tried each node with
  its own self-referential relay first; two independently-relayed loopback
  Iroh endpoints cost seconds per connection attempt (see the perf finding
  below) -- switching Node B's `parent_coordinator.iroh` to Node A's relay
  made no measurable difference, ruling out "different relay homes" as the
  cause (see the next decision) but is still the more realistic shape for
  two nodes actually meant to reach each other, so kept.
  `coordinator_iroh`'s "/v1/info" server always binds
  `http_bind_address`'s port **+ 10** (`spawn_http_info_server`), which
  collided across nodes at the first port choice -- the hundred-port gap
  between Node A's and Node B's port blocks avoids it.
- **The federated-hop perf budget is measured and documented, not
  hard-asserted at 50 ms.** Every iteration succeeded on its first
  connection attempt (zero "proxy call failed; retrying" log lines), so the
  ~4.4-4.9 s p50/p99 measured here is genuine fresh-QUIC-connection
  establishment cost on this sandboxed test environment, not a retry storm
  or a correctness bug -- confirmed by trying both a per-node relay and a
  shared relay with no change. `IrohHop` (`crates/router/src/proxy.rs`)
  opens a brand-new connection per `resolve_fetches` call with no reuse; the
  test's own `FETCH_LATENCY_SANITY_CEILING` (30 s) is a hang/regression
  backstop, not the task.md budget, which is recorded here instead:
  **not met** on this environment, for a transport-layer reason recorded as
  its own `deferred-backlog.md` entry (`IrohHop` connection reuse), separate
  from and more fundamental than the already-recorded fetch-parallelism
  item.
- **A cross-service fetch failure (asserter mismatch, timeout) asserts as a
  transport-level `Err`, not an empty `result`.** Confirmed against
  `SynSvcNativeService::resolve_query_auth`'s documented behavior and
  `native_dispatch_identity.rs`'s own
  `native_dispatch_denies_closed_on_a_cross_service_fetch_failure`: this
  failure mode is `DataLayerError::PermissionDenied`, a real JSON-RPC error
  (code -32010) -- deliberately different from an ordinary "row not
  reachable" deny (ADR-0007's "no result is a valid outcome"), since it is
  an infrastructure/trust failure, not a legitimate absence.
  `SyneroymClient::request` deserializes strictly into
  `JsonRpcResponse{result: Value, ..}` (no `error` field), so the wire's
  error envelope surfaces as a deserialize-level `Err` here rather than a
  structured error code -- the test asserts `Err` itself, not its content.
- **Doc-hygiene, per the B3 plan §10 checklist:** all three
  "audience of the first non-root token" supersessions were already fixed
  in Phase 1 (checked this session, not re-done); the `route_handler/
  dispatch.rs` inventory correction was already applied in Phase 4's own
  status entry. Nothing left outstanding from that checklist.

#### Tests

`crates/substrate/tests/federated_fdae_e2e.rs` (new, 1 test exercising both
the success and the timeout/mismatch-deny scenarios in one real two-node
setup, since standing up two full substrates per scenario would roughly
double an already ~50 s test for no independent proof-value):
`federated_fdae_fetch_across_two_real_substrates` -- deploys real services
on two independent substrates via the real `orchestrator/deploy` JSON-RPC
call, seeds data over the wire, queries `documents` on Node B as a real
wire caller (a self-issued root `CapabilityToken`, no hand-built
`CallerContext`), and asserts: (1) only alice's own document comes back,
proving the real cross-substrate `plan_read` -> `resolve_fetches` (real
Iroh QUIC hop) -> `finalize` join; (2) the federated-hop latency, measured
and printed, against a generous hang-backstop rather than the task.md
budget (see "Decisions"); (3) a second Node B app whose policy declares the
wrong `expected_asserter_did` gets a hard deny, not an empty-but-successful
result or a leaked row, proving Failure/Security matrix row 6's timeout/
mismatch case cross-substrate.

#### Explicitly out of Phase 5 scope (recorded, not silently dropped)

- **The federated-hop perf budget itself (< 50 ms p99)** -- not met on this
  environment; the gap is a transport-layer connection-reuse gap
  (`deferred-backlog.md`), not an FDAE correctness issue, and not something
  this phase's own scope (proving the mechanism works, and measuring it
  honestly) extends to fixing.
- **`resolve_relation`'s A1/A2 fork on an unowned substrate** -- discovered,
  worked around in the test (Node A is owned), recorded in
  `deferred-backlog.md`; not fixed, since it is existing Phase 3 behavior
  this phase's own code did not introduce.
- **D-04-02-h (both ingresses)** -- unaffected by this phase; stays open per
  Phase 4's own disposition. Step 22's "…never reaches the WASM guest" half
  of the reference scenario stays open for the same reason.
- **Automated cross-node `expected_asserter_did` discovery/publication** --
  this phase's test reads the DID directly off the node it itself
  constructed (the same access a real deploying operator would have, not a
  bypass); a real lookup/publication *mechanism* so a policy author never
  needs an out-of-band step at all is still not built (`deferred-backlog.md`,
  updated this phase to correct its own now-stale "not yet operable
  end-to-end" claim).

#### Verification evidence

- `cargo +nightly fmt --all` -- clean.
- `cargo clippy --workspace --all-targets --all-features` -- zero warnings.
- `cargo test --test federated_fdae_e2e -p syneroym-substrate` (sandbox
  disabled, real port binds) -- **1 passed**, run twice back to back to
  confirm non-flakiness (48.41 s, 50.28 s); federated fetch latency both
  runs: p50 ≈ 4.46-4.50 s, p99 ≈ 4.48-4.52 s (5 iterations each; see
  "Decisions" for why this doesn't meet the < 50 ms budget and why that's
  not treated as a Phase 5 regression).
- `cargo test --workspace` -- unchanged from Phase 4's baseline aside from
  the new test binary; the same nine pre-existing sandbox-environmental
  targets fail under this CLI's default network sandbox, identical set and
  error class to every prior phase.
- `mise run test:e2e` -- run (sandbox disabled), **12/12 green**, matching
  the established baseline; this phase adds no WIT/guest-visible change and
  touches no substrate startup wiring, so this reconfirms no regression
  rather than exercising new behavior.
- `traceability-matrix.md`'s `[FND-IAM]` (M4B) row flipped `In Progress
  (Slice B2 complete)` → `In Progress (Slices B2, B3 complete)`, with B3's
  delivered evidence and known gaps recorded.
- Failure/Security matrix row 6 and reference-scenario step 23 in `task.md`
  updated with this phase's e2e evidence; Slice B3's own header line marked
  complete.
- `wasm32-wasip2` -- unbroken; no WIT change this phase.

## Slice B3.5-fdae — Guest-Originated Read Identity Threading (D-04-02-h) ✅ (2026-07-25)

Branch: `feat/m04b-slice-b3.5-fdae` (based on `feat/m04b-slice-b3-phase5`, since
main does not yet carry Slice B3 Phase 5). Closes the one gap left open
across three consecutive prior phases (B2 Phase 4, B3 Phase 4, B3 Phase 5):
D-04-02-h, "a guest-originated `data-layer` read carries no real external
principal into `HostState`/native dispatch." `crates/fdae`'s
`plan_read`/`compile_read`, `crates/data_db`'s `QueryAuth`/`check_access`, and
`SynSvcNativeService::resolve_query_auth`'s deliberate no-`AuthLevel`-carve-out
design are all unchanged ground truth for this slice — the fix is entirely
about *what identity reaches* those already-correct call sites.

### Root cause (confirmed against `main`, matching the B3 Phase 4 analysis)

`router/src/route_handler/dispatch.rs`'s `JsonRpcToWasm` branch called
`AppSandboxEngine::execute_wasm_json(service_id, interface, request)` with no
`caller` argument at all, even though `dispatch_json_rpc_once` already held
the router-verified `caller: Option<&CallerContext>` one arm over (the
Native-service branch uses it). `execute_wasm_json` → `execute_wasm_vals` →
`prepare_wasm_execution` unconditionally built
`CallerContext::service_system(service_id)`, so `HostState.caller` was always
the synthesized system identity regardless of who actually invoked the guest.
Both D-04-02-h ingresses trace back to this one gap:

- **Ingress (i)** — the WASM host-function path (`store::Host for
  HostState`'s `get`/`query`/etc.) reads `self.caller` directly.
- **Ingress (ii)** — a guest's `syneroym:proxy::call` into its own service's
  native `data-layer` (`proxy::Host::call`'s self-proxy branch) always
  constructed a *fresh* `CallerContext::service_system(&self.component_id)`
  for the `ProxyRequest`, independent of ingress (i)'s fix — so closing (i)
  alone would not have closed (ii).

### What was delivered

- **`crates/sandbox_wasm/src/engine.rs`** — `execute_wasm_json`,
  `execute_wasm_vals`, and `prepare_wasm_execution` all gained a trailing
  `caller: Option<CallerContext>` parameter. `prepare_wasm_execution` now
  does `caller.unwrap_or_else(|| CallerContext::service_system(service_id))`
  in place of the unconditional synthesis — `None` (an unauthenticated
  connection, which a WASM guest still admits per design §6.1.2) reproduces
  the exact prior behavior. `execute_wasm` (the string-typed entry point used
  only by test/dev harnesses — `smoke-tests`, the messaging test driver in
  `control_plane/src/service.rs`, `invoke_test_context`) keeps its old
  signature unchanged and always passes `caller: None` internally, so none of
  those call sites' behavior changed and none needed editing.
- **`crates/router/src/route_handler/dispatch.rs`** — the `JsonRpcToWasm` arm
  now passes `caller.cloned()` into `execute_wasm_json`, closing ingress (i)
  for any router-verified caller reaching a WASM guest's own exported
  interface.
- **`crates/sandbox_wasm/src/host_capabilities.rs`** — `proxy::Host::call`
  (the guest-facing `syneroym:proxy/proxy::call` implementation) now forwards
  `self.caller.clone()` as the `ProxyRequest.caller` whenever the proxy
  target is the guest's **own** service (`service == self.component_id`) —
  the same-service self-proxy case the proxy gate's existing exception
  already restricts to a guest's own data, so this cannot escalate to another
  service's rights. A genuine cross-service proxy call (a different target
  service) still synthesizes `service_system`, unchanged: real
  cross-service caller-delegation is a separate, not-yet-built B1/UCAN
  mechanism, explicitly out of this slice's scope. This closes ingress (ii)
  with **no change needed** in `SynSvcNativeService::resolve_query_auth`
  (`control_plane/src/synsvc_native.rs`) — its documented "no `AuthLevel`
  carve-out" design (kept specifically so the self-proxy route can never be
  *more* permissive than the direct route) was already correct for whatever
  `NativeInvocation.caller` turned out to be; it had only ever received a
  synthesized identity because nothing upstream forwarded a real one.
- **`crates/router/src/proxy.rs`** — `ProxyRouter::invoke_local`'s
  `WasmChannel` arm (a proxied call into a *different* WASM service's own
  guest-exported interface, not a native capability) now passes
  `caller: None` **explicitly** to `execute_wasm_json`, with an updated
  comment distinguishing this from the two D-04-02-h ingresses closed above:
  forwarding a proxy's own caller across a *genuine* cross-service call is
  the separate, deferred caller-delegation question, not this slice's scope.
  No behavior change on this path.
- Doc-hygiene: `dispatch_json_rpc_once`'s own doc comment (which said `caller`
  was "unused" outside the Native arm) updated to describe the WASM arm's new
  use; the D-04-02-h Decision Register entry and Slice B3.5-fdae's own
  `task.md` entry marked resolved with a full account of what closed and how;
  reference scenario step 22's "…never reaches the WASM guest" half marked
  done; `traceability-matrix.md`'s `[FND-IAM]` (M4B) row updated (still `In
  Progress` — B4-fdae/B5-fdae remain); `deferred-backlog.md`'s D-04-02-h row
  moved to "Recently resolved" (no in-code `TODO`/`FIXME` marker existed for
  it to remove).

### Tests

Both pinned D-04-02-h regression tests were **flipped, not deleted** — each
now proves closure for a *real* caller while a sibling test keeps pinning the
one case that is still legitimately empty (`caller: None`, no verified
identity at all reaches either ingress):

- **`crates/sandbox_wasm/tests/data_layer_integration.rs`** —
  `test_deployed_policy_filters_guest_originated_query_for_a_real_caller_d04_02_h_closed`
  (new): deploys the same `data-layer-test` WASM fixture under a
  `principal_column`-direct policy (`profiles.creator_uuid`, a payload field
  via `json_extract` — chosen over the original `creator`/`user`-join shape
  because the write path's host-stamped `creator_id` is always the
  *service's* own `component_id`, never a real caller's DID, so a
  caller-owned row has to be seeded directly through `ServiceStore` rather
  than through the guest's own `put`). Seeds one row owned (per the policy)
  by a real, capability-bearing `CallerContext`, then drives the guest's
  `run-crud-scenario(1)` through `execute_wasm_json`'s new `caller` param:
  the guest's own fresh write (unrelated, no `creator_uuid`) stays correctly
  excluded, and the query reaches exactly the one seeded row — `1`, not the
  prior unconditional `0`.
  `test_deployed_policy_yields_empty_guest_originated_query_d04_02_h`
  (existing, re-scoped): unchanged in behavior and assertion (still `0`) —
  its doc comment now explains this is the anonymous-connection case
  (`run_crud_scenario`/`execute_wasm` always pass `caller: None`), not a
  general gap, and points at the new test for the closed case.
  `make_engine_with_storage` gained a third return value (`Arc<KeyStore>`,
  needed to seed a row directly), threaded through its one prior call site.
- **`crates/router/tests/proxy_dispatch.rs`** —
  `guest_self_proxy_data_layer_filters_for_a_real_caller_d04_02_h_closed`
  (new): builds the same `proxy-caller`/`SynSvcNativeService`
  self-proxy harness as the existing pin, under a `principal_column`-direct
  `items.creator_id` policy (the physical column, so no `json_extract`
  needed at all here). Seeds two rows directly (bypassing the guest's
  self-proxy `put`, for exact control over which principal owns which row)
  — one owned by a real caller, one by a different principal — then drives
  `self_proxy_call`'s `get` for each with `dispatch_json_rpc_once`'s
  `caller: Some(&real_caller)`: the real caller reaches their own row and is
  denied the other principal's row.
  `guest_self_proxy_data_layer_returns_empty_when_policy_present` (existing,
  re-scoped): unchanged in behavior and assertion (still empty) — its doc
  comment now explains this is the `caller: None` case; points at the new
  test for the closed case.
  `test_route_handler_with_self_native_data_layer` now also returns
  `(Arc<dyn StorageProvider>, Arc<KeyStore>)` (needed for direct seeding),
  threaded through both its existing call sites; `self_proxy_call` gained a
  `caller: Option<&CallerContext>` parameter, threaded through its four
  existing call sites as `None` (preserving their exact behavior).
- Both new tests independently prove **per-row** filtering, not just "the
  policy now passes everything through": each leaves at least one row in the
  same collection that must stay excluded (the guest's own unrelated write
  in the sandbox_wasm test; the other principal's row in the router test).

### Import cleanup

One inline fully-qualified path introduced incidentally by the `router` test
file's expanded return type touched the surrounding line —
`syneroym_identity::Identity::generate()` in
`test_route_handler_with_self_native_data_layer` — fixed by importing
`syneroym_identity::Identity` and calling `Identity::generate()`, per
AGENTS.md's import-qualification rule. No other inline `::`-qualified paths
were introduced by this slice's diff (checked by scanning the diff of every
touched file, not just the new code).

### Verification evidence

- `cargo +nightly fmt --all` — clean.
- `cargo clippy --workspace --all-targets --all-features` — zero warnings.
- `cargo test -p syneroym-sandbox-wasm --lib --tests` — **74 passed**, 0
  failed (44 lib + 5 blob + 3 data-layer [2 prior + 1 new] + 6 lifecycle + 3
  messaging + 13 stream).
- `cargo test -p syneroym-router --lib --tests` — **134 passed**, 0 failed
  (72 lib + 9 deploy_grant + 34 native_dispatch_identity + 5 proxy_dispatch
  [4 prior + 1 new] + 10 service_ownership + 2 ucan_context + 2
  unsupported_protocol).
- `cargo test -p syneroym-router --test proxy_dispatch -- --nocapture` and
  `cargo test -p syneroym-sandbox-wasm --test data_layer_integration --
  d04_02_h --nocapture` — both new + both re-scoped tests independently
  re-run and confirmed passing before the full-crate runs above.
- `cargo test --workspace --no-fail-fast` — **10 pre-existing,
  sandbox-environmental targets fail** (`coordinator-iroh`'s
  `connection_limit`/`multi_hop_relay`/`tls_rotation`, `mqtt-broker`'s lib
  tests, `sdk`'s `connect_timeout`, `substrate`'s `basic_lifecycle`/
  `http_passthrough_e2e`/`messaging_client_e2e`/`stream_client_e2e`/
  `federated_fdae_e2e`), all `"Operation not permitted (os error 1)"`
  binding a real port under this CLI's default network sandbox — the same
  set and error class every prior phase has documented, plus
  `federated_fdae_e2e` (added in Slice B3 Phase 5, which already documented
  it needs the sandbox disabled for its real two-substrate QUIC hop; not
  previously exercised by a *default-sandbox* full-workspace run in this
  slice's diff history). None of these ten targets' source files were
  touched by this slice. Re-ran `federated_fdae_e2e` in isolation with the
  sandbox disabled to confirm it is unaffected by this slice's changes: **1
  passed** (52.20s) — matching Phase 5's own documented result.
- `mise run test:e2e` — run (sandbox disabled, needed for real port binds),
  **12/12 green** (8 `webrtc.spec.ts` + 4 `multi-hop.spec.ts`), matching the
  established baseline. Run deliberately, despite no WIT change: this slice
  changes real production behavior on the `JsonRpcToWasm` dispatch path
  every router-originated WASM call takes (a router-verified caller now
  reaches `HostState.caller` instead of always `service_system`), so
  reconfirming the existing e2e fixtures (which deploy no FDAE policy) still
  come up and behave identically is worth the run, not assumed from "no WIT
  change" alone.
- `wasm32-wasip2` — unbroken; no `.wit` file touched this slice, and no
  `test-components` rebuild was needed (the existing `data-layer-test`,
  `proxy-test`, and `greeter` artifacts exercised correctly through both new
  tests without modification).

### Explicitly out of scope (recorded, not silently dropped)

- **Cross-service guest caller-delegation** (a proxy call to a *different*
  service inheriting the calling guest's own identity rather than acting as
  itself) — unaffected by this slice; still `service_system` in both
  `proxy::Host::call` (cross-service branch) and `ProxyRouter::invoke_local`'s
  `WasmChannel` arm. Real delegation needs B1/UCAN chain-of-custody design
  work this slice does not attempt.
- **`AppSandboxEngine::execute_wasm`'s test/dev-harness call sites** (smoke
  tests, the messaging test driver, `invoke_test_context`) — deliberately
  left on the unchanged, `service_system`-only signature; none of them
  represent a router-verified or self-proxy ingress.
- **B4-fdae (stage-4 WASM ABAC) and B5-fdae (write-side Mode-A
  authorization)** — untouched, as before; this slice closes D-04-02-h only.

## Slice B3.5-fdae — Post-implementation review response (2026-07-25)

An independent review of commit `ac02c89` (re-running tests/clippy/e2e
rather than trusting this document) surfaced eight findings, ranked by the
review as High/High/Medium/Medium/Medium/Low/Low/Low. All eight were
verified against the code before acting on any of them; six were confirmed
and fixed, one was confirmed but correctly belongs to B5-fdae, one was a
documentation-only observation. Full rationale for each is in `task.md`'s
Slice B3.5-fdae entry; this section is the verification/evidence trail.

| # | Finding | Verdict | Disposition |
|---|---|---|---|
| F1 | Admin-rooted caller can now reach guest `execute-ddl`/`query-raw` from the wire | Confirmed, real | **Disagree it's a defect to fix here** — `lifecycle_hooks.rs::test_execute_ddl_allowed_for_admin_ucan_root_caller` already pins the same admission (ADR-0015/0016, B0.md §11.2) against a hand-built `HostState`; this slice makes an already-accepted design reachable from the wire for the first time, not a new capability. Pinned through the real dispatch wiring instead: `engine.rs::prepare_wasm_execution_forwards_a_wire_admin_caller_that_reaches_guest_execute_ddl` |
| F2 | Ingress-(i) test couldn't distinguish "filtered" from "unfiltered" (limit truncation masked it) | Confirmed | **Fixed** — strengthened `data_layer_integration.rs`'s test: 5 unrelated guest writes + a second, differently-owned seeded row (7 total), `limit: 5`; `observed == 1` now only holds under genuine filtering |
| F3 | Self-proxy `put`/`batch-mutate` now attributes `creator_id` to the real caller, diverging from the direct-WIT path | Confirmed, real | **Out of this slice's scope, B5-fdae's (D-04-02-f)** — pinned explicitly (`proxy_dispatch.rs::guest_self_proxy_put_attributes_creator_id_to_the_real_caller_not_the_service`) and recorded in `deferred-backlog.md` rather than guessed at here |
| F4 | A guest's real proof can launder onto a remote hop if the self-proxy target falls through to `invoke_remote` | Confirmed, real | **Fixed** — `invoke_remote_at` now keys the identity-presentation decision off `req.origin`, not just proof presence; a `CallOrigin::Guest` request never presents a proof remotely. New test: `proxy.rs::guest_with_proof_still_forwards_as_anonymous_not_the_real_proof` |
| F5 | The cross-service `else` branch and the now-functional guest-ingress federated fetch are untested | Partially confirmed | **Fixed the else-branch gap** (`host_capabilities.rs::self_proxy_forwarding_does_not_extend_to_a_different_target_service`); **accepted the federated-fetch-success gap as residual** — the two dimensions it depends on (real-caller wire-reachability; the fetch mechanism itself) are each already independently covered, narrowing the actual gap to their untested intersection |
| F6 | The "no forwarded `LocalElevated`" invariant was prose-only | Confirmed | **Fixed** — `debug_assert!` added at `prepare_wasm_execution`'s `caller.unwrap_or_else` |
| F7 | Guest-path `DecisionTrace` now logs real end-user DIDs | Confirmed, expected | **Documentation only** — consistent with the native path's existing behavior; noted as a conscious sign-off, not a new exposure class |
| F8 | Two intermittent test failures in parallel mode | Confirmed, pre-existing | **No action** — reproduced, then confirmed clean with `--test-threads=1`; same `mainline` DHT actor-thread flake already known, unrelated to this slice |

### Verification after the response

- `cargo +nightly fmt --all` — clean.
- `cargo clippy --workspace --all-targets --all-features` — clean, 0
  warnings.
- `cargo test -p syneroym-router --lib --tests` and
  `cargo test -p syneroym-sandbox-wasm --lib --tests`, both
  `--test-threads=1` — all green, including the five new/strengthened tests
  above.
- `cargo test --workspace --no-fail-fast` — same baseline as this slice's
  own original run: sandbox-port-bind failures only, on the same
  already-documented targets, none touched by this response.
