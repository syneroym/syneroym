# Slice B2 Phase 2 — `data_db` Integration: Implementation Plan

> Planning artifact for M04B Slice B2 **Phase 2** (data_db integration).
> Phase 1 (the standalone `syneroym-fdae` crate — policy model, JSON schema,
> ReBAC→SQL compiler) is merged to `main` (PR #86). This phase wires the
> **already-shipped** `syneroym_fdae::compile_read` into `data_db`'s
> read/delete paths.
>
> Grounded on `main` @ `f96860f`. The shipped `crates/fdae` is treated as
> **ground truth** over the parent plan doc's (`slice-b2-implementation-plan.md`)
> pseudocode. Nothing in `crates/fdae` changes in Phase 2.
>
> Cross-refs: `slice-b2-implementation-plan.md` (§4 reused code, §5 data_db
> integration, §3.5 watchdog, §6/§6.1 RLS/CLS + CLS→B4 ordering, §10 strict/
> default-deny/**decision trace**, §11 tests), ADR-0017 (§4 Mode A/B, §8 safety
> rails, §9 decision trace, "Resolved at acceptance", 2026-07-20
> `principal_column` amendment).

---

## 0. Branch status — blocker

Work started on `main` (clean tree). Per AGENTS.md, no staging/commits on
`main`. **Cut a feature branch before any code** (e.g.
`feat/m04b-slice-b2-data-db`). Phase 1 used `feat/m04b-slice-b2-fdae-sieve`.

---

## 1. Scope + the resolved decisions

Phase 2 = threading + SQL merge + watchdog + tests. The compiler is done.

**Decisions the parent plan / task.md left open — now RESOLVED (accepted
2026-07-20); the rest of this plan is written against them:**

| # | Decision | Parent plan said | **Resolved** | Rationale |
|---|---|---|---|---|
| **D1** | How `masked_fields` leaves the store (§6.1 says "get it out", but `QueryResult` is a **WIT-generated** type — can't carry a host-only field) | "extend `QueryResult` or expose alongside" (ambiguous) | ✅ `query`/`get` return a `data_db`-local wrapper `ReadOutcome<T> { value, masked_fields }` | Single source of truth (store compiles the sieve once); clean B4 hand-off. Alt = host re-runs `compile_read` just for `masked_fields` → double-compile, two sources of truth. Rejected. |
| **D2** | `operation` passed to `compile_read` for **`delete_many`** | §5.2 lumps `delete_many` with the read paths (implies `data-layer/read`) | ✅ `data-layer/write` | Deleting is a write. With `read`, a read-only permission's `paths` would make its rows *deletable* — an escalation. |
| **D3** | `check_access` no-policy semantics | unspecified | ✅ existence-check (`Ok(true)` iff row exists) | consistent with Mode B "no policy ⇒ all rows visible". Alt = unconditional `Ok(true)`. Rejected. |

---

## 2. Crate wiring — `crates/data_db/Cargo.toml`

Add (both already in root `[workspace.dependencies]`, verified lines 57–58):

```toml
syneroym-fdae.workspace = true
syneroym-ucan.workspace = true   # QueryAuth borrows &SessionContext; Ability::DATA_LAYER_* for the op
```

No cycle: `data_db → fdae → ucan → identity`, and `data_db → ucan` directly;
none depend back on `data_db`.

---

## 3. New types — new file `crates/data_db/src/auth.rs`

`lib.rs`: add `pub mod auth;` and `pub use auth::{QueryAuth, ReadOutcome};`.

```rust
use syneroym_fdae::Policy;
use syneroym_ucan::SessionContext;

/// Per-request policy + caller context threaded into the read/delete paths.
/// `None` at a call site preserves today's unfiltered behavior (policy-absent
/// services, native dispatch, benches, tests).
pub struct QueryAuth<'a> {
    pub policy: &'a Policy,
    pub session: &'a SessionContext,
    pub service_id: &'a str,
}

/// A read result plus the CLS field-mask the host must apply as its final
/// projection (host-side, Phase 3 -- this crate never strips fields itself,
/// per the stage-4 ordering contract). `masked_fields` is always empty on the
/// policy-absent path.
pub struct ReadOutcome<T> {
    pub value: T,
    pub masked_fields: Vec<String>,
}
```

---

## 4. `ServiceStore` trait signature changes — `crates/data_db/src/traits.rs`

```rust
async fn get(&self, collection: &str, id: &str, auth: Option<&QueryAuth<'_>>)
    -> Result<ReadOutcome<Option<host_store::RecordReadValue>>, host_store::DataLayerError>;

async fn query(&self, collection: &str, opts: &host_store::QueryOptions,
               auth: Option<&QueryAuth<'_>>)
    -> Result<ReadOutcome<host_store::QueryResult>, host_store::DataLayerError>;

async fn aggregate(&self, collection: &str, pipeline: &str,
                   auth: Option<&QueryAuth<'_>>)
    -> Result<host_store::RawQueryResult, host_store::DataLayerError>;   // no ReadOutcome: CLS→deny

async fn delete_many(&self, collection: &str, filter: Option<&str>,
                     auth: Option<&QueryAuth<'_>>)
    -> Result<u64, host_store::DataLayerError>;

/// Mode A point-in-time check (ADR-0017 §4). Fail-closed: policy/compile/exec
/// errors and watchdog timeouts return `Ok(false)`, never an error read as allow.
async fn check_access(&self, collection: &str, id: &str, operation: &str,
                      auth: Option<&QueryAuth<'_>>)
    -> Result<bool, host_store::DataLayerError>;
```

Unchanged: `put`/`patch`/`delete`/`batch_mutate`/`create_collection`/
`drop_collection`/`execute_ddl`/`query_raw`/`write_secret`/`reveal_secret`.
`get`'s `ReadOutcome` carries `masked_fields` for the single record too (host
strips it in Phase 3).

**Sequencing:** step 1 does the `auth` param + `ReadOutcome` wrapping +
`None`-threading for the **four existing** methods (compiles, zero behavior
change). `check_access` (trait method + `do_check_access`) is added in **step 2**
— it's new, has no existing callers, so it can't break step 1's gate.

---

## 5. Step 1 — signatures + `None` threading (must compile, zero behavior change)

### 5.1 Both trait impls (verified)
- `impl ServiceStore for SqliteServiceStore` — `sqlite.rs:1234` (real impl).
- `impl ServiceStore for Arc<SqliteServiceStore>` — `sqlite.rs:1421` (forwarder:
  `self.as_ref().query(collection, opts, auth).await`, etc.; add `auth` to each
  forwarded call + add the `check_access` forwarder).

In the real impl, `query`/`get` wrap results in `ReadOutcome`. In **step 1**
(sieve not yet wired) `masked_fields` is always `Vec::new()`.

### 5.2 Every existing call site (exhaustive — verified by workspace grep)

| File:line | Call | Change |
|---|---|---|
| `sandbox_wasm/src/host_capabilities.rs:429` | `store.get(&collection,&id)` | `…, auth.as_ref())` then `.value` |
| `…host_capabilities.rs:443` | `store.query(&collection,&opts)` | `…, auth.as_ref())` then `.value` |
| `…host_capabilities.rs:457` | `store.aggregate(&collection,&pipeline)` | `…, auth.as_ref())` |
| `…host_capabilities.rs:481` | `store.delete_many(&collection, Some(...))` | `…, auth.as_ref())` |
| `control_plane/src/synsvc_native.rs:284` | `store.get(...)` | `…, None)` then `.value` |
| `…synsvc_native.rs:295` | `store.query(...)` | `…, None)` then `.value` |
| `…synsvc_native.rs:319` | `store.delete_many(...)` | `…, None)` — **parent plan §5.4 said `:311`, stale** |
| `…synsvc_native.rs:432` | `store.aggregate(...)` | `…, None)` — **parent plan §5.4 said `:421`, stale** |
| `data_db/benches/security_config_bench.rs:147` | `store.get("bench", …)` | `…, None)` then `.value` |
| `sandbox_wasm/benches/data_layer_bench.rs:64` | `store.get("bench", …)` | `…, None)` then `.value` |
| `…data_layer_bench.rs:85` | `store.query("bench", …)` | `…, None)` then `.value` |
| `data_db/src/tests_crud.rs` | 11× `store.get(`, 9× `store.query(`, `store.delete_many` (`:220`), 5× `store.aggregate(` | append `None`; `get`/`query` results append `.value` |

In step 1 the host builds `auth = None` (real `QueryAuth` construction lands in
Phase 3 when `HostState.fdae_policy` exists). **Gate:** `cargo test -p
syneroym-data-db` green, no clippy warnings.

---

## 6. Step 2 — `do_query` merge, `do_get`, `check_access`

### 6.1 Shared merge helper (new, in `sqlite.rs`)
RLS **and** caveat `where` filters are intersective and must AND in. The shipped
`CompiledSieve` carries `where_caveats: Vec<serde_json::Value>` — dropping them
re-opens the Phase-1 "dropped-caveat" bug class (a `caveats.where={region:"EU"}`
caller would see all regions).

```rust
// Returns (clause, params) = RLS  AND  each compiled caveat `where`.
// Pure/DB-free; runs inside the interact closure on the owned sieve.
fn merge_sieve(sieve: &CompiledSieve)
    -> Result<(String, Vec<SqlValue>), host_store::DataLayerError> {
    let mut clauses = vec![format!("({})", sieve.where_clause)];
    let mut params: Vec<SqlValue> = sieve.params.clone();
    for caveat in &sieve.where_caveats {
        let raw = serde_json::to_string(caveat)...;
        if let Some(cf) = filter::compile_filter(Some(&raw))? {   // fail-closed on bad JSON
            clauses.push(format!("({})", cf.where_clause));
            params.extend(cf.params);
        }
    }
    Ok((clauses.join(" AND "), params))
}
```

### 6.2 `do_query` — binding order re-derived from the current body (`sqlite.rs:319–372`)

Current order is **filter → cursor → LIMIT** (params `[filter…, cursor, limit]`;
the `limit` param is pushed *after* the WHERE is built, at line 345). The sieve
block goes **first** in both clause and param lists, still before the `limit`
push:

```rust
fn do_query(conn, collection, opts, sieve: Option<&CompiledSieve>) -> …QueryResult… {
    validate_identifier(collection)?;
    let compiled = filter::compile_filter(opts.filter.as_deref())?;
    let limit = opts.limit.unwrap_or(MAX_QUERY_PAGE_SIZE).min(MAX_QUERY_PAGE_SIZE);

    let mut where_clauses = Vec::new();
    let mut bound_params: Vec<SqlValue> = Vec::new();

    // (1) SIEVE FIRST -- RLS ∧ caveats, ahead of filter/cursor/limit.
    let _watchdog = if let Some(s) = sieve {
        let (clause, params) = merge_sieve(s)?;
        where_clauses.push(clause);
        bound_params.extend(params);
        Some(install_watchdog(conn)?)          // §9; only on the policy path
    } else { None };

    // (2) filter  -- unchanged
    if let Some(cf) = &compiled { where_clauses.push(cf.where_clause.clone());
                                  bound_params.extend(cf.params.iter().cloned()); }
    // (3) cursor  -- unchanged
    if let Some(cursor) = &opts.cursor { where_clauses.push("id > ?".into());
                                         bound_params.push(SqlValue::Text(cursor.clone())); }
    let where_sql = if where_clauses.is_empty() { String::new() }
                    else { format!("WHERE {}", where_clauses.join(" AND ")) };
    // (4) limit push -- unchanged, stays LAST
    bound_params.push(SqlValue::Integer(i64::from(limit) + 1));
    // …prepare/query_map identical, but map step errors so a watchdog interrupt → QuotaExceeded
}
```

Final param order `[sieve…, filter…, cursor, limit]` matches clause text order.
The RLS `EXISTS(...)` correlates to `{collection}.payload`/`.creator_id` (the
compiler uses the base **table name** as the qualifier); the single
`FROM {collection}` makes both the RLS and the unqualified `payload` (from
filter/caveats) resolve. **`masked_fields` is not touched here** — cloned out in
the async wrapper (§6.5), returned in `ReadOutcome`; no field stripping in
`data_db`.

### 6.3 `do_get` — Mode A wrap (`sqlite.rs:284`)
`compile_read(Mode::PointInTime{id})` already appends `… AND {table}.id = ?`
(id bound) to the RLS, so a sieve'd `do_get` is a self-contained WHERE — **no
separate `id = ?1`** (that would double-bind):

```rust
match sieve {
  None    => // today's exact path: WHERE id = ?1
  Some(s) => {
     let _watchdog = install_watchdog(conn)?;
     let (clause, params) = merge_sieve(s)?;
     // SELECT payload, creator_id, created_at, updated_at FROM {collection} WHERE {clause}
     // query_row → QueryReturnedNoRows → Ok(None)  (unauthorized get = a miss, ADR-0007)
     // map OperationInterrupted → QuotaExceeded
  }
}
```

### 6.4 `check_access` + `do_check_access` (new, Mode A)
Async method compiles `Mode::PointInTime{id}` with `operation =
Ability(operation_arg)`; **fail-closed to `Ok(false)`** on any
`PolicyError`/exec-error/timeout:

```rust
let sieve = match auth {
    Some(a) => match compile_read(a.policy, collection, a.session, a.service_id,
                                  &Ability(operation.to_string()), Mode::PointInTime{id}) {
        Ok(s) => s,                 // Option<CompiledSieve>; deny_all() ⇒ Some("0=1")
        Err(_) => return Ok(false), // fail-closed
    },
    None => None,
};
// closure → do_check_access:
//   Some(sieve) → let (clause,params)=merge_sieve(&sieve)?;
//                 SELECT EXISTS(SELECT 1 FROM {collection} WHERE {clause})   -- clause already has id=?
//   None        → SELECT EXISTS(SELECT 1 FROM {collection} WHERE id = ?)     -- D3
// watchdog interrupt → Ok(false).  Present-AND-reachable ⇒ true.
```

**D3 (resolved):** no-policy case (`auth=None` or `compile_read → Ok(None)`) is
an existence check.

### 6.5 Async wrappers thread `masked_fields` out
Compile in the async method (borrows `auth.session`), clone `masked_fields`
**before** moving the owned sieve into the closure:

```rust
async fn query(&self, collection, opts, auth) -> Result<ReadOutcome<QueryResult>, _> {
    let sieve: Option<CompiledSieve> = match auth {
        Some(a) => compile_read(a.policy, collection, a.session, a.service_id,
                                &Ability(Ability::DATA_LAYER_READ.into()), Mode::Filter)
                   .map_err(|e| DataLayerError::Internal(e.to_string()))?,   // fail-closed, loud
        None => None,
    };
    let masked_fields = sieve.as_ref().map(|s| s.masked_fields.clone()).unwrap_or_default();
    let conn = self.reader_pool.get().await…?;
    let value = conn.interact(move |conn| do_query(conn, &collection, &opts, sieve.as_ref()))
                    .await…??;
    Ok(ReadOutcome { value, masked_fields })
}
```

`get` mirrors with `Mode::PointInTime{id}`. Compile-error policy: Mode B
(`query`/`get`) → `Err(Internal)` (a malformed policy is not "zero rows");
`deny_all()`'s `0=1` → legitimately empty (ADR-0007).

**Integration tests here** run real SQL against seeded rows (mirror
`crates/fdae`'s style): seed a `users` collection (`payload:{"did":…}`) +
`documents` (`payload:{"creator_uuid":…}`), a `document`→`user` `creator` policy
via `parse_and_validate`, a hand-built `SessionContext`, assert **row
visibility** (`ids == ["doc-1"]`), not SQL string shape.

### 6.6 Fail-closed on missing schema objects (`principal_column` / target table)

ADR-0017's 2026-07-20 amendment requires a `principal_column` that is absent at
query time to *"fail-closed with a trace entry if missing."* In `data_db`'s
fixed 5-column schema (`id, payload, creator_id, created_at, updated_at`) this
splits cleanly and is already fail-closed **by construction**:

- **Non-reserved** `principal_column`/`join_column`/key names compile to
  `json_extract(<qual>.payload, ?)` (the shipped `col()`), and `json_extract` of
  an absent JSON path returns `NULL` — the correlation/terminal comparison simply
  fails to match, so the row is pruned. No SQL error, no leak.
- **Reserved** names (`id`/`creator_id`/`created_at`/`updated_at`) always exist
  on every collection table, so they cannot be "missing."
- The residual error case is a missing **target table** (a policy-declared
  relation target whose collection the guest never created) or a reserved column
  absent from a guest's custom-DDL table: SQLite raises `no such table`/`no such
  column` at execution. That maps fail-closed — **Mode B (`query`/`get`/
  `aggregate`/`delete_many`) → `Err`/deny; Mode A (`check_access`) → `Ok(false)`**
  — via the same error handling as the watchdog interrupt. Add one integration
  test (policy references a not-yet-created target table → deny, not leak).

The *"trace entry"* half of the amendment is part of the decision trace, which is
deferred — see §12 item 8. Phase 2 guarantees the **fail-closed** half; the
observable "why" lands with the trace.

---

## 7. Step 3 — `delete_many` via the writer path (NOT `do_query`)

Runs `send_write_command` → `DbCommand::DeleteMany` (`sqlite.rs:798`) → writer
loop dispatch (`sqlite.rs:894`) → `do_delete_many` (`sqlite.rs:238`) on the
**single persistent writer `conn`**, not the reader pool.

- Add `sieve: Option<CompiledSieve>` to the `DeleteMany` variant (owned; all
  fields `Send+'static`).
- Async `delete_many` (`sqlite.rs:1369`) compiles with **`Mode::Filter`, op =
  `DATA_LAYER_WRITE`** (D2), moves the owned sieve into the command.
- Dispatch (`sqlite.rs:894`): `do_delete_many(&conn, &collection,
  filter.as_deref(), sieve.as_ref())`.
- `do_delete_many`: sieve clause+params **first**, then filter:
  ```
  where_clauses = [sieve_clause?, filter_clause?]   params = [sieve…, filter…]
  DELETE FROM {collection} WHERE {join " AND "}     (no WHERE if both absent)
  ```
- **Watchdog on the writer conn** (persistent!): install progress handler + a
  guard that **clears it after** so the next writer command isn't affected.
  Interrupt → `QuotaExceeded` (Mode B). Only on the policy path.

---

## 8. Step 4 — `aggregate` sieve-awareness (RLS-bypass fix)

`aggregate` is ungated (open like `query`); if `query` filters and `aggregate`
doesn't, RLS is trivially bypassed.

- **CLS → fail-closed:** in `do_aggregate`, if `sieve.masked_fields` is non-empty
  → `Err(DataLayerError::PermissionDenied)` (deny the whole aggregate; no CLS-safe
  aggregation attempt — the parent plan's explicit call).
- **RLS:** merge the sieve (`merge_sieve` → `(clause, params)`) and inject into
  the inner base query. `aggregate::compile` (`aggregate.rs:38`) gains `sieve:
  Option<(&str, &[Value])>`. Inner build (`aggregate.rs:75–88`) becomes
  `WHERE {sieve} AND {match}` (or just `{sieve}`), and the param order becomes
  **`group.params ++ sieve_params ++ match_params ++ having_params ++
  limit_params`** — sieve slots right after the `$group` select-expr params
  (first in `SELECT … FROM`) and before `$match`.
- `do_aggregate` (`sqlite.rs:386`) already installs the `QUERY_RAW_MAX_VM_OPS`
  progress handler + `QueryRawGuard`; keep it. Interrupt already → `QuotaExceeded`
  via `map_query_raw_step_error`.
- Update `aggregate.rs` test helpers `compile_ok`/`compile_err`
  (`aggregate.rs:445/449`) to pass `None`.

---

## 9. Step 5 — Watchdog matrix (copy the `do_aggregate` pattern)

`install_watchdog(conn)` = `conn.progress_handler(FDAE_MAX_VM_OPS, Some(|| true))`
returning a **new progress-only drop-guard** `ProgressGuard { conn }` whose
`Drop` calls `conn.progress_handler(0, None)`. (Resolves the §6.2/§6.3 forward
reference. A progress-only guard, *not* a reuse of `QueryRawGuard` — the sieve
paths install no authorizer, and the writer conn has none either, so clearing an
authorizer would be dead noise. `do_aggregate` keeps its existing `QueryRawGuard`
unchanged.) **Install only when a sieve is present** (policy-absent path stays
byte-for-byte unchanged). Reader-pool conns are reused across calls, so the guard
must clear on drop; the writer conn is persistent, so the same clear-on-drop is
load-bearing there too.

**Time budget is a hard-coded constant in Phase 2 — deferred configurability,
recorded (not silent).** ADR-0017 §8 requires the budget be *"configurable, not
hard-coded"*; parent plan §3.5 says *"thread from policy or substrate config."*
Neither exists yet: `crates/fdae/src/policy.rs` and `schema/fdae-v1.json` carry
**no** budget/timeout field, and there is no substrate-config wire-in. Phase 2
therefore reuses the existing `QUERY_RAW_MAX_VM_OPS` constant as the interim
default (rename/alias to `FDAE_MAX_VM_OPS` for intent; same value). Real
configurability is deferred — a policy-level `budget` field is an `fdae`
schema/model change (Phase 1 territory, to re-open), and a substrate-level
default belongs with the Phase 4 deploy/config plumbing. **Do not close this by
baking in the constant silently — it is the exact anti-pattern §8 called out.**
See §12 item 9.

| Path | Conn | Timeout mapping (ADR-0017 §8) |
|---|---|---|
| `do_query` | reader pool | `Err(QuotaExceeded)` |
| `do_get` | reader pool | `Err(QuotaExceeded)` |
| `do_check_access` | reader pool | **`Ok(false)`** (Mode A) |
| `do_delete_many` | **writer (persistent)** | `Err(QuotaExceeded)` — guard must clear |
| `do_aggregate` | reader pool | `Err(QuotaExceeded)` (already) |

`do_get`/`do_check_access` currently map via `map_rusqlite_error`, which does
**not** special-case `OperationInterrupted` — the sieve path needs the
interrupt→`QuotaExceeded`/`Ok(false)` mapping added.

---

## 10. Step 6 — CLS threading (no stripping in `data_db`)

Per §6.1's recorded architecture decision, the **field strip is a host-side final
projection landing in Phase 3** (above B4's stage-4 hook, below the WIT
response). Phase 2's only CLS job is to surface `masked_fields` out of the store
via `ReadOutcome.masked_fields` (§6.5). **No `serde_json` payload rewriting in
`data_db`.** A Phase-2 test asserts the store *exposes* `masked_fields ==
["ssn"]` for a CLS policy but that query rows are otherwise unmasked (stripping
is Phase 3).

> **Interim-state caveat (do not misread a green Phase-2 run).** task.md's
> Failure/Security row *"CLS: caller lacks column permission → column masked/
> projected out; value never returned"* is **not** satisfied by Phase 2 — masked
> values are still returned by the store; only the metadata is exposed. That
> task.md row stays **open until Phase 3** lands the host-side projection. A
> passing Phase-2 CLS test proves the plumbing, not the requirement. (Aggregate
> is the exception: with CLS active it fails closed *now* — §8 — so no masked
> value leaks through an aggregate in Phase 2.)

---

## 11. Step 7 — tests + gates

- **`data_db` integration (`tests_fdae.rs`, real SQL/seeded rows):** Mode B
  excludes unreachable rows (empty, not error); Mode A `check_access` deny; `get`
  of an unreachable row → `None`; `aggregate` row-filtered identically to `query`
  **and** denied when CLS active; `delete_many` row-filtered (write op);
  **binding order** — sieve ∧ filter ∧ cursor pagination returns the right rows
  with a caveat-`where` present; watchdog timeout → deny (pathological recursion
  vs. a tiny op-budget); `masked_fields` exposed but rows unmasked.
- **Fail-closed matrix:** invalid policy → `Err`/`Ok(false)`; adversarial
  `subject_did`/caveat bound not interpolated (covered in `fdae`; add a data_db
  end-to-end row); **missing target table** (policy references an uncreated
  collection) → Mode B `Err`/deny, Mode A `Ok(false)` (§6.6), not a leak.
- **Final gates:** `cargo +nightly fmt --all`, `cargo clippy --workspace
  --all-targets --all-features` (clean), `cargo test --workspace`.
  `wasm32-wasip2` untouched (no WIT change in Phase 2 — `check-access` WIT is
  Phase 3). Import-hygiene pass over every edited file; no planning-doc IDs in
  comments.
- **Review pass before "done"** — Phase 1 caught an auth-bypass, a
  dropped-caveat, and an injection surface; highest-risk Phase-2 spots are the
  four binding-order splices and `merge_sieve` (caveat inclusion).

---

## 12. Stale / ambiguous / deferred items surfaced

Items 1–7 are call-outs resolved within this plan; **items 8–10 are scope
decisions carried by Phase 2** — recorded here as explicit deferrals (a decision,
not an oversight), each with where it actually lands.

1. **`synsvc_native.rs` line drift** — parent plan §5.4 cites `delete_many :311`
   / `aggregate :421`; actual `:319` / `:432`. (get `:284`, query `:295` still
   accurate.)
2. **D1 — `masked_fields` egress** — `QueryResult` is WIT-generated; "extend
   `QueryResult`" isn't literally doable without leaking mask metadata to the
   guest. ✅ Resolved via `ReadOutcome` wrapper (host-recompute rejected).
3. **D2 — `delete_many` operation** — ✅ Resolved: `data-layer/write`, not
   `read`.
4. **D3 — `check_access` no-policy semantics** — ✅ Resolved: existence-check.
5. **`check_access` has no non-test caller in Phase 2** — the WIT `check-access`
   fn + host `store::Host::check_access` are Phase 3. Phase 2 adds only the Rust
   trait method + `do_check_access`, exercised by tests. Intentional.
6. **`where_caveats` merge is in-scope and required** though the numbered steps
   don't call it out — the shipped `CompiledSieve.where_caveats` exists precisely
   so `data_db` ANDs them (§6). Dropping = security regression. Folded into
   `merge_sieve`.
7. **task.md "ServiceManifest Extension" / `[services.my-svc.fdae]`** references a
   `ServiceManifest` struct that doesn't exist (real type `ServiceConfig`) —
   already flagged in parent plan §12.6, and it's **Phase 4**, out of Phase 2
   scope. No action now.
8. **Decision trace — deferred to Phase 5, recorded (both reviewers flagged).**
   ADR-0017 §9 / parent plan §10 call the structured trace *"not optional"* and
   say it *"ships with the first slice"* (all of B2). Phase 1 (PR #86) shipped
   without it — `grep` of `crates/fdae/src/compile.rs` finds no `Trace`/`tracing`.
   Phase 2 adds the richest deny paths (`do_check_access`, `deny_all`, `get`→
   `None`, missing-schema fail-closed), so it is the natural emit site — **but a
   *rich* trace (`held`, `operation_admitted`, `rows_reached`, `path_failed`)
   cannot be built in `data_db`**: `compile_read` returns only the compiled SQL
   (`0=1` or a `WHERE EXISTS` string) and discards the decision structure. So the
   trace is **not a pure `data_db` add** — it needs an `fdae` API change to
   surface the decision (a `DecisionTrace` alongside `CompiledSieve`), *plus* an
   emit point at each Phase-2 deny seam. Parent plan §13 sequences it in **Phase
   5**; this plan keeps it there. **Phase 2 action:** none beyond leaving the
   deny return points (`Ok(false)`, `Ok(None)`, `deny_all`, `Err`) as the
   identified seams. If the next session wants to honor *"ships with the first
   slice"* literally, this is the item to pull forward — but it reopens `fdae`
   (Phase 1), so it is a deliberate call, not a silent drop. (The
   `principal_column`-missing *"trace entry"* of the ADR amendment, §6.6, is the
   same deferral; Phase 2 guarantees only its fail-closed half.)
9. **Watchdog time budget hard-coded — configurability deferred (§9).** ADR-0017
   §8 requires it *"configurable, not hard-coded"*; no budget field exists in
   `policy.rs`/`schema/fdae-v1.json` or substrate config today. Phase 2 uses the
   `QUERY_RAW_MAX_VM_OPS` constant (aliased `FDAE_MAX_VM_OPS`) as the interim
   default and says so; policy-level config = an `fdae`-schema change, substrate
   default = Phase 4. Recorded so the fixed constant is not mistaken for done.
10. **Write-side Tier 3 is a separate deliverable — now scheduled as Slice
    B5-fdae, not a vague follow-up (Reviewer 2).** FDAE splits into two halves:
    *read-side Tier 3 (confidentiality)* — RLS/CLS on `query`/`get`/`aggregate`,
    delivered by B2/B3 — and *write-side Tier 3 (integrity)* — Mode-A
    authorization of single-row mutations, **not** in B2. Today `put`/`patch`/
    `delete`/`batch_mutate` stay `auth`-free (§4) and run under *service*
    authority (`creator_id = component_id`; they never consult `caller.session`);
    they carry no capability gate either. So a caller who cannot *see* a row via
    RLS can still `delete(id)`/`patch(id)` it — an **integrity** gap (real:
    record ids are guest-chosen, often knowable), distinct from the
    confidentiality B2 protects. This is **pre-existing** (the state before B2, so
    B2 regresses nothing) and parent plan §5.2 deferred it; the **asymmetry** is
    that B2 *does* filter `delete_many` (D2, write op), yet the same row is still
    removable one-at-a-time via `delete(id)`.

    **Why it is not bolted onto B2:** `patch`/`delete` of an *existing* row map
    cleanly to Mode A, but **`put`-create is unmodeled** — for a new id there is
    no row to walk `[creator, caller]` against, and row-reachability ReBAC cannot
    express *"who may create a row in this collection"* (that is a
    collection-scoped permission the policy model lacks). Enforcing only
    `patch`/`delete` would close half the hole and imply "writes are protected"
    — worse than a documented gap. So write-side Tier 3 needs a design decision
    (new sub-decision **D-04-02-f**, creation authorization) *plus* code, and gets
    its own slice.

    **Recorded in task.md:** Slice **B5-fdae** (write-path Mode-A enforcement +
    `batch_mutate` per-mutation) and sub-decision **D-04-02-f** (creation authz).
    **Known limitation until B5-fdae lands:** post-B2, FDAE protects read
    confidentiality and bulk-delete, but single-row write/delete integrity is
    unenforced — deployments relying on FDAE for write integrity must wait. Phase
    2 does **not** expand to cover it.

---

## 13. Execution order + gates

1. **Step 1** — signatures + `ReadOutcome` + `None` threading (all 4 existing
   methods, both impls, all call sites). Gate: `cargo test -p syneroym-data-db`.
2. **Step 2** — `merge_sieve`; `do_query` splice; `do_get` Mode-A wrap;
   `check_access`/`do_check_access`; async wrappers thread `masked_fields`.
   Integration tests on seeded rows.
3. **Step 3** — `delete_many` writer path (`DbCommand::DeleteMany` sieve field,
   dispatch, `do_delete_many` merge, writer watchdog).
4. **Step 4** — `aggregate` RLS inject + CLS deny; `aggregate::compile` sieve
   param.
5. **Step 5** — watchdog matrix (install-on-sieve, timeout mappings, progress-only
   guard); budget stays the interim `FDAE_MAX_VM_OPS` constant (§9, §12 item 9).
6. **Step 6** — confirm `masked_fields` egress; no data_db stripping (task.md CLS
   row stays open until Phase 3, §10).
7. **Step 7** — full test pass; fmt/clippy/`cargo test --workspace`; import
   hygiene; review pass.

Stop for review after each step.

**Explicitly out of Phase 2** (see §12 items 8–10): the decision trace (Phase 5;
reopens `fdae`), policy/substrate-configurable watchdog budget (fdae-schema /
Phase 4), and write-side Tier-3 enforcement (**Slice B5-fdae** + sub-decision
**D-04-02-f** creation authz, now scheduled in task.md). Each is recorded as a
deferral, not silently dropped.
