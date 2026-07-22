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
