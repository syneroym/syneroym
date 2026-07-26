# Slice B5-fdae Implementation Plan — Write-Side Tier 3 (Mode-A Write Authorization)

**Status:** ready to implement, revision 2 (2026-07-26, post independent
review). Five decisions answered — D-B5-2 (`USING` + `WITH CHECK`), D-B5-3
(deny closed, no exemption), D-B5-5 (unify `creator_id`), plus two the review
surfaced: D-B5-6 (`creator_id` immutable on update) and D-B5-7 (CLS on the
write side).

**All line anchors are against `568c432`.** Revision 1 was written against
`0b0e63b`; the branch advanced mid-plan and `host_capabilities.rs` /
`synsvc_native.rs` shifted. `crates/data_db` is untouched by that commit.

**Source of record:** `task.md` "Slice B5-fdae", decision `D-04-02-f`.
**ADRs:** [ADR-0017](../../../decisions/0017-fdae-policy-schema-and-compilation.md)
§2.1/§4/§7/§8/§9, [ADR-0015](../../../decisions/0015-ucan-capability-model.md)
A2/A3/A5.
**Requirement:** `[FND-IAM]`.

---

## 0. What this slice is, in one paragraph

B2/B3 delivered read-side Tier 3 — confidentiality. Rows a caller cannot
reach never come back from `query`/`get`/`aggregate`, and `delete_many` is
already sieved. The **integrity** half is missing: `put`/`patch`/`delete`/
`batch_mutate` run under service authority, never consult `caller.session`,
and carry no capability gate, so a caller who cannot *see* row `X` can still
`delete("X")` or `patch("X")` it. B5 closes that: every single-row mutation is
authorized against the caller's compiled FDAE policy before it commits, rows
**and** columns.

**Where the enforcement lives is the load-bearing choice.** It goes inside
`data_db`'s writer actor, in the same transaction as the mutation — not at the
two ingresses. See D-B5-1.

---

## 1. Design decisions

### D-B5-1 — Enforcement point: `data_db` writer actor, in-transaction ✅

**The check runs inside `data_db`, in the same SQLite transaction as the
mutation.** The ingress (`sandbox_wasm::host_capabilities`,
`control_plane::synsvc_native`) keeps doing what it already does for
`delete_many` — build a `QueryAuth` for `data-layer/write` via its own
`resolve_query_auth`, which is where the cross-service stage-2 fetch can
happen, since `data_db` has no proxy — and hands it to the store. The store
compiles the sieve and evaluates it per target row on the **writer**
connection, inside the transaction.

Rejected: authorize at the ingress via `store.check_access(...)` then
`store.put(...)`. Three reasons, in order of weight:

1. **TOCTOU.** The check would run on the reader pool, the write on the
   writer actor. Between them, another writer can change the row's
   reachability. In-transaction, there is no window.
2. **`batch_mutate` loses its own guarantee.** Its contract is all-or-nothing
   inside one transaction; checks performed outside that transaction observe a
   different database than the mutations do.
3. **Two ingresses, one rule.** A third ingress that forgets the call is a
   silent bypass. Inside `data_db` the rule cannot be skipped — the same
   argument that put the read sieve there.

### D-B5-2 — What "authorized" means for a write (this *is* D-04-02-f) ✅

**Resolved: Postgres `USING` + `WITH CHECK`. No policy-schema change.**

| Operation | Pre-image reachable (`USING`) | Post-image reachable (`WITH CHECK`) |
|---|---|---|
| `delete(id)` | ✅ | n/a (row is gone) |
| `patch(id)` | ✅ | ✅ |
| `put(id)`, row exists (upsert-update) | ✅ | ✅ |
| `put(id)`, row absent (create) | n/a | ✅ |
| `batch_mutate` | per mutation, by the rows above | per mutation |

"Who may create a row in this collection" is answered by "may you reach the
row you are about to create" — which the existing `paths:` already expresses,
evaluated against the row's post-image instead of its pre-image. Why:

- **ADR-0017 §2.1 already commits to this model** — "This is Postgres's
  model: `GRANT` is the capability, `CREATE POLICY` (RLS) is the
  `permissions:` block." It maps `USING` and stops. `WITH CHECK` is precisely
  the half D-04-02-f is missing.
- **It blocks write-side confused deputy on creates.** A collection-level
  "may create" flag says nothing about *what* may be created: a caller
  allowed to create could create a row attributed to someone else. The
  post-image check forbids that. *(For the **update** branch this argument
  does not carry on its own — see D-B5-6.)*
- **The append-only case still works.** `allows: [data-layer/write]` with
  `paths: []` (public) passes the post-image check for anything and, because
  `allows` omits read, grants no read.
- **Zero new policy surface.** No `fdae-v1.json` change, no new `Permission`
  field, no deploy-time validation, no migration.

Alternative considered and rejected: `Permission.create: bool` + a schema
amendment (what `task.md` and D-04-02-f pre-commit to). Strictly more schema,
and it leaves the create-for-someone-else hole open.

### D-B5-3 — `AuthLevel::System` writes become impossible under a policy ✅

**Resolved: deny closed, no exemption.**

`CallerContext::service_system` (`crates/rpc/src/native.rs:117`) carries no
capabilities, so `plan_read` finds no applicable permission, falls to
`deny_all()`, and every write it gates is denied. Reads already behave this way
(D-04-02-h's documented "returns empty"); an empty read is survivable where a
denied write is a hard failure.

**Blast radius is guest-mediated writes only.** Both native arms already
refuse an anonymous caller outright — `http.rs:170` (`dispatch_native`) and
`dispatch.rs:101` — so an `http_routes` → native `data-layer` `put`/`patch`
always carries a router-verified caller. The paused local demo is therefore
unaffected. `System` reaches a write only through:

| Site | Shape |
|---|---|
| `sandbox_wasm/src/engine.rs:957` | `prepare_wasm_execution`'s `caller.unwrap_or_else(service_system)` |
| `router/src/route_handler/dispatch.rs:133` | `caller.cloned()` is `None` for an unauthenticated connection reaching a **WASM guest** (the native arm of the same dispatch refuses it) |
| `router/src/proxy.rs:306` | cross-service proxied WASM call, `caller: None` by design |
| `sandbox_wasm/src/engine.rs:1093` | `deliver_message` — broker-delivered message into a guest |
| `sandbox_wasm/src/engine.rs:1299` | `open_stream_instance` |

Rejected: exempting `System` from the write sieve (reopens the ingress-(ii)
bypass shape `host_capabilities.rs:204-235` argues against, and asymmetrically
— policy would apply to an anonymous visitor's reads but not their writes).
Deferred as the follow-up: thread a real principal into the
anonymous-connection and proxied-WASM ingresses, the direction B3.5 set.

Mitigation available today: policy is opt-in per object type (ADR-0017 §2.1),
so a collection a guest writes to as itself simply gets no `definitions:`
entry — unless `strict: true`.

### D-B5-4 — Stage-4 (`authorize_rows: true`) on the write path ✅

`data_db` has no WASM engine, and a mutation in flight has no candidate-row
batch to hand the after-step. **Deny closed**, exactly as `do_delete_many`
(`sqlite.rs:255`) and `do_aggregate` (`sqlite.rs:651`) already do:
`sieve.abac_permissions` non-empty ⇒ `PermissionDenied` before any mutation
runs. Extends an already-recorded deferral rather than creating a category.

### D-B5-5 — `creator_id` attribution ✅

Two write paths disagree today: the guest's direct WIT `put` stamps
`component_id` (the service DID); the self-proxy path
(`synsvc_native.rs:710`'s arm) stamps `caller.app_instance ?? caller.caller_did`.
Under D-B5-2 that decides whether the post-image check passes, so a policy
declaring `principal_column: "creator_id"` — a real pattern
(`control_plane/src/service/orchestration.rs:1627`,
`router/tests/proxy_dispatch.rs:724`) — gets opposite answers on the two paths.

**Resolved: `creator_id` is uniformly the effective principal**, derived once:

```
creator_id = match caller.auth {
    System | LocalElevated | LocalReadOnly => component_id,   // no external principal
    _ => caller.app_instance
             .or(caller.session.anchor_did)
             .unwrap_or(caller.session.subject_did),
}
```

`anchor_did` before `subject_did` mirrors `compile::terminal_value` and
`RemoteFetch.principal_did` — a row created through a proxying service is
attributed to the principal it acts for, not the proxy.

**Impact is at the two ingresses only.** *(Revision 1 wrongly listed
`tests_crud.rs:318`'s `test_creator_id_is_always_host_supplied` as needing an
update: it passes `"the-deploying-service-id"` as an explicit argument and
asserts the store persists what the host handed it — still true, since D-B5-5
changes only how the ingresses compute that argument. It needs the mechanical
`None` addition and nothing else.)* The genuinely affected test is
`router/tests/proxy_dispatch.rs::guest_self_proxy_put_attributes_creator_id_to_the_real_caller_not_the_service`,
which already asserts the chosen behavior — it stops being a pinned
inconsistency and becomes the spec.

### D-B5-6 — `creator_id` becomes immutable on update ✅ (new, from review)

D-B5-2 and D-B5-5 together open a one-call ownership steal that `WITH CHECK`
does **not** catch. `do_put`'s upsert is
`ON CONFLICT(id) DO UPDATE SET … creator_id = excluded.creator_id`
(`sqlite.rs:179`). On a policy whose `paths` reach a row by something *other*
than `creator_id` — a shared team/org path — caller B legitimately passes the
pre-image check on caller A's row, the upsert silently rewrites `creator_id`
to B, and the post-image check passes because the row is now B's. If any
definition uses `principal_column: "creator_id"`, that also strips A's access.

This is not pre-existing: before D-B5-5, guest-direct writes always stamped
the service's own id, so the refresh was a no-op in practice. D-B5-5 turns it
into a meaningful transfer.

**Resolved: drop `creator_id = excluded.creator_id` from `do_put`'s
`ON CONFLICT` clause.** `creator_id` then means what its name says — who
created the row — and becomes a stable identity anchor, which it must be to
serve as one of the four `RESERVED_COLUMNS` a policy may name as
`principal_column`. One-line SQL change; `created_at` is already preserved
across an upsert by the same reasoning (`sqlite.rs:162-174`), so this makes
the two consistent.

Rejected: an explicit pre-image-vs-post-image `creator_id` equality assertion
in `authorize_and_mutate`. Same effect, but it leaves the column mutable and
enforces the invariant in the authorization layer rather than in the write
itself — so any future write path that skips authorization reopens it.

### D-B5-7 — CLS applies to writes ✅ (new, from review)

Tier 3 is "RLS + CLS" throughout `task.md` (`:332-339`, Failure/Security row
3), and the read path applies `masked_fields` via `strip_masked_fields`.
Revision 1 authorized rows only, so a caller carrying
`fields: {deny: ["ssn"]}` could not read `ssn` but could `put`/`patch` it
freely — the post-image check asks only "is the row reachable". That is the
same asymmetry B5 exists to close, one level down.

**Resolved: enforce it.** The rule, when `sieve.masked_fields` is non-empty:

- **create** — reject if the new payload contains any masked key. A caller who
  cannot see a field cannot author it.
- **update (`put`/`patch`)** — reject if any masked key's value differs
  between pre-image and post-image payloads, including added or removed.

`data_db` already extends CLS past pure projection for exactly this reason:
`do_query` (`sqlite.rs:470-477`) refuses a filter referencing a masked field,
because "masking only the projection turns the predicate into an oracle."
Letting a masked field be *written* is that finding's integrity twin.

Cost: the pre-image probe must return the payload, not just `EXISTS`, when
`masked_fields` is non-empty. `do_patch` already reads the existing payload
(`sqlite.rs:195`), so only `put` pays anything new, and only under a
CLS-active policy.

If this is dropped to contain scope, it needs a §7 backlog row and a §6 line —
silence is the one unacceptable outcome.

---

## 2. Phase plan

| Phase | Content | Gate |
|---|---|---|
| 1 | `crates/fdae`: `DecisionTrace` gains `operation`/`row_id`/`write_phase` (§3.1) | `cargo test -p syneroym-fdae` green |
| 2 | `data_db`: `row_reachable`, `row_exists`, `masked_fields_unchanged`, `authorize_and_mutate`, `do_authorized_*`, `ServiceStore` signatures, `DbCommand`, writer loop, D-B5-6's one-line upsert change. All existing call sites pass `None`. | `cargo test -p syneroym-data-db` green, no behavior change on the `None` path |
| 3 | `data_db` unit tests (§5.1) | matrix rows 10-12 evidence exists |
| 4 | Ingress wiring + `internal`→`data_layer_error` fixes + `creator_id` unification (§3.4, §3.5) **and the test-fixture remediation in §3.7 — these must land together, or the fixtures break** | `cargo test --workspace` green |
| 5 | Integration tests (§5.3), bench row (§5.4) | `mise run test:all` green |
| 6 | Docs: ADR-0017 amendment, matrix rows, backlog, traceability | §8 |

---

## 3. Exact changes

### 3.1 `crates/fdae` — decision-trace fields

Revision 1 declared this crate unchanged. That forecloses D-B5-3's
diagnosability argument: `DecisionTrace` (`trace.rs:34-89`) has **no operation
and no row-id field**, so a write deny is indistinguishable from a read deny
on the same collection, and a pre-image deny from a post-image deny.

Add to `DecisionTrace`:

```rust
/// The ability this decision was compiled for (`data-layer/read`,
/// `data-layer/write`, or an app-permission ref). Without it a write deny
/// and a read deny on the same collection are indistinguishable in the log.
pub operation: String,
/// Mode A only: the row the compiled predicate was executed against.
pub row_id: Option<String>,
/// Write path only: which half of the ADR-0017 §4 check this record is --
/// `"pre-image"` (may the caller reach the row as it stands) or
/// `"post-image"` (may they reach what they just wrote). `None` for reads.
pub write_phase: Option<String>,
```

`plan_read` sets `operation` from its own `operation.0` argument on every
`DecisionTrace` it builds (there are four: the strict-mode deny, the
no-applicable-permission deny, the main path, and `compile_read`'s
fetch-required deny). `emit()` adds all three to both the `info!` and `debug!`
lines. All three are `Default`-able, so the many `..DecisionTrace::default()`
sites need no edit.

**Batch noise.** `emit_mode_a_execution_trace` fires per `row_reachable` call,
so an allowed 200-mutation batch would emit ~400 `debug!` lines. Denies always
emit; **allow** emits are suppressed for batch members via a
`trace_allows: bool` on `authorize_and_mutate` (`true` for single-row ops,
`false` inside `do_batch_mutate`), with one summary `debug!` per batch.

No policy-schema, compiler, or `CompiledSieve` change.

### 3.2 `crates/data_db/src/sqlite.rs` — the enforcement core

**`row_exists`** — the unsieved create-vs-update probe:

```rust
fn row_exists(conn: &Connection, collection: &str, id: &str)
    -> Result<bool, host_store::DataLayerError>
{
    conn.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {collection} WHERE id = ?1)"),
        params![id],
        |row| row.get::<_, bool>(0),
    )
    .map_err(map_rusqlite_error)
}
```

> Deliberately unsieved: it decides *which* rules apply, not whether the write
> is allowed. Both branches still end in `PermissionDenied` on failure, so it
> leaks nothing beyond "id `X` was free", which is inherent to any
> create-by-id API.

**`row_reachable`** — evaluate the write sieve against one row id. Placed next
to `do_check_access` and deliberately **not** reusing it: `do_check_access`
falls back to a bare existence probe when `sieve` is `None`
(`traits.rs:255`'s documented D3 behavior), which is the wrong answer for a
create.

```rust
/// Evaluates a `data-layer/write` sieve against exactly one row id on `conn`.
/// The sieve is compiled in `Mode::Filter` (once per call, not per row), so
/// the id predicate is appended here -- equivalent to `Mode::PointInTime`,
/// without recompiling for every mutation in a batch.
///
/// Fail-closed: a watchdog interrupt, a malformed caveat, or a missing table
/// is `Ok(false)`, never a silent pass. Same contract as `do_check_access`.
fn row_reachable(
    conn: &Connection,
    collection: &str,   // already `validate_identifier`-checked by the caller
    id: &str,
    sieve: &CompiledSieve,
    phase: &'static str,     // "pre-image" | "post-image"
    trace_allows: bool,
) -> Result<bool, host_store::DataLayerError> {
    let outcome: Result<rusqlite::Result<bool>, host_store::DataLayerError> = (|| {
        let _watchdog = install_watchdog(conn)?;   // dropped before the caller commits
        let (clause, mut params) = merge_sieve(sieve)?;
        params.push(SqlValue::Text(id.to_string()));
        Ok(conn.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM {collection} \
                 WHERE ({clause}) AND {collection}.id = ?)"
            ),
            rusqlite::params_from_iter(params.iter()),
            |row| row.get::<_, bool>(0),
        ))
    })();

    let allowed = matches!(outcome, Ok(Ok(true)));
    if !allowed || trace_allows {
        let mut sieve = sieve.clone();
        sieve.trace.row_id = Some(id.to_string());
        sieve.trace.write_phase = Some(phase.to_string());
        emit_mode_a_execution_trace(Some(&sieve), match &outcome {
            Ok(Ok(true))  => ModeAOutcome::Matched,
            Ok(Ok(false)) => ModeAOutcome::NotMatched,
            Ok(Err(e))    => ModeAOutcome::Aborted(format!("{e}")),
            Err(e)        => ModeAOutcome::Aborted(format!("{e:?}")),
        });
    }
    Ok(allowed)
}
```

> **Watchdog lifetime matters.** `install_watchdog` sets a
> `progress_handler(FDAE_MAX_VM_OPS, || true)` that interrupts *any* statement
> past the budget — including `COMMIT`. `ProgressGuard`'s `Drop` clears it and
> the guard is function-scoped, so it is always gone before the caller commits.
> Do not hoist it to the transaction's scope.

**`masked_fields_unchanged`** (D-B5-7) — compares the masked keys of two
payloads; `pre: None` means a create, where any masked key present is a
rejection:

```rust
fn masked_fields_unchanged(
    pre: Option<&[u8]>,
    post: &[u8],
    masked: &[String],
) -> Result<bool, host_store::DataLayerError>
```

Fail-closed on a payload that will not parse as a JSON object while a
non-empty mask applies, matching `auth::strip_masked_fields`'s own rule.

**`authorize_and_mutate`** — the pre/mutate/post envelope. `conn` is always
inside a transaction the *caller* owns, so an `Err` rolls the mutation back.

```rust
/// One authorized single-row mutation (ADR-0017 §4 Mode A, write side).
/// `sieve == None` (policy-absent, or an exempt caller) is today's
/// unfiltered behavior, unchanged.
#[allow(clippy::too_many_arguments)]
fn authorize_and_mutate(
    conn: &Connection,
    collection: &str,
    id: &str,
    sieve: Option<&CompiledSieve>,
    require_pre_image: bool,
    check_post_image: bool,
    trace_allows: bool,
    mutate: impl FnOnce(&Connection) -> Result<(), host_store::DataLayerError>,
) -> Result<(), host_store::DataLayerError> {
    validate_identifier(collection)?;
    let Some(sieve) = sieve else { return mutate(conn) };

    // D-B5-4: no candidate-row batch exists mid-mutation, so the after-step
    // cannot run -- deny closed, same rule as `do_delete_many`/`do_aggregate`.
    if !sieve.abac_permissions.is_empty() {
        return Err(host_store::DataLayerError::PermissionDenied);
    }

    // USING half: may the caller reach the row as it stands today?
    // Note this subsumes existence -- see the idempotency change in §4.
    if require_pre_image
        && !row_reachable(conn, collection, id, sieve, "pre-image", trace_allows)?
    {
        return Err(host_store::DataLayerError::PermissionDenied);
    }

    // D-B5-7: capture the pre-image payload only when CLS is active.
    let pre_payload = (!sieve.masked_fields.is_empty() && require_pre_image)
        .then(|| read_payload(conn, collection, id))
        .transpose()?;

    mutate(conn)?;

    // WITH CHECK half (D-B5-2): may the caller reach the row they just wrote?
    // Rejects both "create a row you could never see" and "rewrite a row out
    // of your own reach". `Err` rolls the caller's transaction back.
    if check_post_image
        && !row_reachable(conn, collection, id, sieve, "post-image", trace_allows)?
    {
        return Err(host_store::DataLayerError::PermissionDenied);
    }

    // D-B5-7: a field the caller cannot read is one they cannot write.
    if !sieve.masked_fields.is_empty() && check_post_image {
        let post = read_payload(conn, collection, id)?;
        if !masked_fields_unchanged(pre_payload.as_deref(), &post, &sieve.masked_fields)? {
            return Err(host_store::DataLayerError::PermissionDenied);
        }
    }
    Ok(())
}
```

**Per-operation wiring:**

| Op | `require_pre_image` | `check_post_image` | `mutate` |
|---|---|---|---|
| `put` | `row_exists(conn, collection, &value.id)?` | `true` | `do_put` |
| `patch` | `true` | `true` | `do_patch` |
| `delete` | `true` | `false` | `do_delete` |

**Transaction wrappers.** `do_put`/`do_patch`/`do_delete` stay as they are
(raw mutation on `&Connection`, still callable from `do_batch_mutate`'s
existing transaction). Three new wrappers open a transaction only when a sieve
is present, so the policy-absent hot path pays nothing new:

```rust
fn do_authorized_put(
    conn: &mut Connection,
    collection: &str,
    value: &host_store::RecordWriteValue,
    creator_id: &str,
    sieve: Option<&CompiledSieve>,
) -> Result<(), host_store::DataLayerError> {
    let Some(sieve) = sieve else { return do_put(conn, collection, value, creator_id) };
    validate_identifier(collection)?;
    let tx = conn.transaction().map_err(map_rusqlite_error)?;
    let existed = row_exists(&tx, collection, &value.id)?;
    authorize_and_mutate(
        &tx, collection, &value.id, Some(sieve),
        existed, true, true,
        |c| do_put(c, collection, value, creator_id),
    )?;
    tx.commit().map_err(map_rusqlite_error)
}
// `do_authorized_patch` (true, true) / `do_authorized_delete` (true, false)
// are the same shape, minus the `row_exists` probe.
```

*(Revision 1's samples were broken — `do_authorized_put` used an uncomputed
`existed`, and `do_batch_mutate` called a `row_exists` that was only described
in prose. Both are defined above.)*

**`do_batch_mutate`** gains `sieve: Option<&CompiledSieve>`. The existing `tx`
already gives all-or-nothing rollback, so the first denial `?`-propagates and
drops the whole batch — D-04-02-f's "how does `batch_mutate` authorize
per-mutation": **per mutation, inside the one transaction, first denial rolls
back everything.** `trace_allows: false` throughout (§3.1's noise rule):

```rust
for mutation in mutations {
    match mutation {
        Mutation::Put(v) => {
            let existed = row_exists(&tx, collection, &v.id)?;
            authorize_and_mutate(&tx, collection, &v.id, sieve, existed, true, false,
                |c| do_put(c, collection, v, creator_id))?
        }
        Mutation::Patch(p) => authorize_and_mutate(&tx, collection, &p.id, sieve, true, true, false,
            |c| do_patch(c, collection, &p.id, &p.patch_json))?,
        Mutation::Delete(id) => authorize_and_mutate(&tx, collection, id, sieve, true, false, false,
            |c| do_delete(c, collection, id))?,
    }
}
```

**D-B5-6, one line.** In `do_put`'s upsert (`sqlite.rs:179-180`), drop
`creator_id = excluded.creator_id` from the `ON CONFLICT(id) DO UPDATE SET`
list. `payload` and `updated_at` keep updating; `creator_id` joins
`created_at` as create-time-only.

**`DbCommand`** (`sqlite.rs:1140-1195`) — add
`sieve: Option<Box<CompiledSieve>>` to `Put`, `Patch`, `Delete`,
`BatchMutate`. Boxed, for the `clippy::large_enum_variant` reason `DeleteMany`
already documents at `sqlite.rs:1182`. Writer-loop arms
(`sqlite.rs:1271-1290`) call the `do_authorized_*` wrappers with `&mut conn`
and `sieve.as_deref()`.

### 3.3 `crates/data_db/src/traits.rs` — `ServiceStore` signatures

Four methods gain a trailing `auth: Option<&QueryAuth<'_>>`, matching
`delete_many`'s existing shape (`traits.rs:213`):

```rust
async fn put(&self, collection: &str, value: &RecordWriteValue,
             creator_id: &str, auth: Option<&QueryAuth<'_>>) -> Result<(), DataLayerError>;
async fn patch(&self, collection: &str, id: &str, patch_json: &[u8],
               auth: Option<&QueryAuth<'_>>) -> Result<(), DataLayerError>;
async fn delete(&self, collection: &str, id: &str,
                auth: Option<&QueryAuth<'_>>) -> Result<(), DataLayerError>;
async fn batch_mutate(&self, collection: &str, mutations: &[Mutation],
                      creator_id: &str, auth: Option<&QueryAuth<'_>>) -> Result<(), DataLayerError>;
```

Doc comments must state: `auth: None` is unfiltered (policy-absent services,
lifecycle contexts, benches, tests); `Some` applies the ADR-0017 §4 Mode-A
write check as `data-layer/write`, pre-image and post-image per D-B5-2, plus
CLS per D-B5-7 — **and the idempotency change in §4**.

`SqliteServiceStore`'s impl (`sqlite.rs:1711`/`1729`/`1816`/`1844`) compiles
the sieve exactly as `delete_many` does today (`sqlite.rs:1830`):

```rust
let sieve = compile_sieve_for_op(auth, collection, Ability::DATA_LAYER_WRITE, Mode::Filter)?
    .map(Box::new);
```

`Mode::Filter`, not `Mode::PointInTime` — one compile per call, with the `id`
predicate appended per row in `row_reachable`. This is what makes a
200-mutation batch cost one compile, not 200.

The `Arc<SqliteServiceStore>` forwarding impl
(`sqlite.rs:1959`/`1968`/`2004`/`2017`) forwards the new argument.

### 3.4 Call sites

**Production — 2 files, 8 sites** *(revision 1 said "4 files" over a 2-file
table)*:

| Site | Change |
|---|---|
| `sandbox_wasm/src/host_capabilities.rs:621` (`put`) | resolve write auth, pass through |
| `sandbox_wasm/src/host_capabilities.rs:639` (`patch`) | same |
| `sandbox_wasm/src/host_capabilities.rs:802` (`delete`) | same |
| `sandbox_wasm/src/host_capabilities.rs:911` (`batch_mutate`) | same |
| `control_plane/src/synsvc_native.rs:710` (`"put"`) | same |
| `control_plane/src/synsvc_native.rs:725` (`"patch"`) | same |
| `control_plane/src/synsvc_native.rs:869` (`"delete"`) | same + **fix** `internal` → `data_layer_error` (`:879`) |
| `control_plane/src/synsvc_native.rs:904` (`"batch-mutate"`) | same + **fix** `internal` → `data_layer_error` (`:939`) |

`host_capabilities` pattern (all four identical in shape):

```rust
if self.read_only { return Err(DataLayerError::PermissionDenied); }
// Owned locals FIRST -- `resolve_query_auth` takes `&mut self` and the
// returned `QueryAuth<'_>` borrows it, so nothing may touch `self`
// afterwards. Same discipline `get`/`query` already document at :662.
let creator_id = self.caller.write_attribution(&self.component_id);   // D-B5-5
let store = open_store(self.component_id.clone(), self.key_store.clone(),
                       self.storage_provider.clone()).await?;
let query_auth = self.resolve_query_auth(
    &collection,
    &Ability(Ability::DATA_LAYER_WRITE.to_string()),
    Mode::Filter,
).await?;
store.put(&collection, &value, &creator_id, query_auth.as_ref()).await
```

`synsvc_native` pattern:

```rust
let auth = self.resolve_query_auth(
    &invocation, &req.collection,
    &Ability(Ability::DATA_LAYER_WRITE.to_string()), Mode::Filter,
).await.map_err(data_layer_error)?;
store.put(&req.collection, &req.value, &creator, auth.as_ref())
    .await.map_err(data_layer_error)?;
```

> **The `LocalElevated`/`LocalReadOnly` exemption is ingress-specific, not a
> property of "the ingress" generally** *(revision 1 overstated this)*.
> `host_capabilities.rs:242` has the carve-out;
> `synsvc_native.rs:419`'s `resolve_query_auth` has **no** `AuthLevel`
> carve-out and documents that as deliberate. Harmless today —
> `CallerContext::local_elevated` is constructed only at `engine.rs:1010`,
> inside the WASM path, so a `LocalElevated` caller never reaches
> `synsvc_native` — but the plan must not claim otherwise, and a future
> lifecycle-over-native-dispatch path would need the carve-out added there.

**Pre-existing bug fixed in passing:** `synsvc_native.rs` maps `delete`
(`:879`), `delete-many` (`:901`), and `batch-mutate` (`:939`) errors through
`internal(e.to_string())`, so `delete_many`'s stage-4 `PermissionDenied`
(`sqlite.rs:256`) already surfaces today as an internal error rather than a
permission denial. All three become `data_layer_error`.

**Tests/benches — 55 sites** *(revision 1 said "~48")*, mechanical `None`
addition: `data_db/src/tests_crud.rs` (29), `data_db/src/tests_fdae.rs` (8),
`sandbox_wasm/benches/data_layer_bench.rs` (4),
`router/tests/proxy_dispatch.rs` (4),
`sandbox_wasm/tests/abac_integration.rs` (3),
`data_db/benches/fdae_bench.rs` (3),
`sandbox_wasm/tests/data_layer_integration.rs` (2),
`data_db/benches/security_config_bench.rs` (2).

### 3.5 `creator_id` unification (D-B5-5)

Add to `crates/rpc/src/native.rs`:

```rust
impl CallerContext {
    /// The DID a row this caller writes should be attributed to. Synthesized
    /// substrate contexts have no external principal, so the service owns the
    /// row; everyone else is attributed to the principal they act for
    /// (`anchor_did` before `subject_did` -- the same precedence
    /// `compile::terminal_value` and `RemoteFetch.principal_did` use).
    #[must_use]
    pub fn write_attribution(&self, service_id: &str) -> String { ... }
}
```

Called from `host_capabilities.rs:621`/`:911` (replacing
`self.component_id.clone()`) and `synsvc_native.rs:710`/`:904` (replacing the
`app_instance ?? caller_did` expression).

### 3.6 WIT — doc comments only

No new guest-visible function or field; `put`/`patch`/`delete`/`batch-mutate`
already return `data-layer-error`, whose `permission-denied` variant is what a
denied write returns. Doc comments to add on
`wit_interfaces/wit/data-layer/data-layer.wit`:

- `put`/`patch`/`delete`/`batch-mutate` may now return `permission-denied`
  under an FDAE policy; `batch-mutate` denies the whole batch on the first
  unauthorized mutation.
- **`delete` is no longer unconditionally idempotent under a policy**, and
  `patch` of a missing row returns `permission-denied` rather than
  `schema-violation` — see §4.
- **`check-access` answers about an existing row and is not a create
  pre-check.** For an id that does not exist yet it returns `false` (Mode A
  finds no row; with no policy it falls back to an existence probe,
  `traits.rs:255`), while B5 will *allow* the create if the post-image is
  reachable. A guest that pre-checks before creating would get "no" and then
  succeed anyway. Documented rather than changed: making `check-access`
  answer a hypothetical post-image needs the payload it does not take, which
  is a WIT signature change with no current consumer.

### 3.7 Test-fixture remediation — **required, lands with Phase 4**

Revision 1 inventoried Rust `ServiceStore::*` call sites but not **writes
dispatched through an ingress B5 newly gates**. These are currently green and
break at *seeding*, before their own assertions:

| Fixture | Why it breaks | Fix |
|---|---|---|
| `router/tests/native_dispatch_identity.rs:1023`, `:1204`, `:1295` | Three FDAE tests build the service with `Some(policy)`, then seed via `json_rpc_body("put", …)` under a capability-less `test_caller`. The comment at `:1034` states the assumption outright — "`put`/`create-collection` carry no FDAE gate (write-side Tier 3 is Slice B5-fdae), so any verified caller can seed fixture rows" — which B5 invalidates. | Seed via the store directly with `auth: None`; delete the now-false comment. Keeps the tests testing read filtering rather than accidentally testing write authorization. |
| `substrate/tests/federated_fdae_e2e.rs:347`/`:371-386`, `:427`/`:450-465`, `:579`/`:620` | `deploy(…)` installs the policy *before* the seeding `put`s, and both fixture policies grant `allows: ["data-layer/read"]` only — so `plan_read` for `data-layer/write` compiles `deny_all()` and every seed denies. Not `#[ignore]`d. | Add a seeding permission to the fixture policies: `"seed": {"allows": ["data-layer/write"], "paths": []}`. Realistic (a service owner seeding its own data) and it cannot widen the read assertions, since `allows` omits read. **Verify during Phase 4** that the seeding client actually holds a write-entailing capability on that resource — Node A is owned (`:285-300`), so this needs checking rather than assuming. |
| `router/tests/proxy_dispatch.rs:678` (`guest_self_proxy_data_layer_returns_empty_when_policy_present`) | Seeds via `self_proxy_call(…, "put", …, None)` — a `None`-caller self-proxy write under a loaded policy, precisely the `AuthLevel::System` write D-B5-3 denies. | Seed via the store directly with `auth: None`. The neighbouring test at `:770-795` already uses exactly that pattern. |

---

## 4. Ordering, semantics, and interaction notes

- **Idempotency and not-found semantics change under a policy** *(review
  finding; revision 1 asserted neither)*. `do_delete` is deliberately
  idempotent (`sqlite.rs:234`, pinned by
  `tests_crud.rs::test_delete_missing_record_is_idempotent`). With
  `require_pre_image = true`, deleting a **non-existent** id under a policy
  returns `PermissionDenied` instead of `Ok` — the pre-image check cannot
  distinguish "absent" from "present but unreachable", and **must not**: that
  distinction is exactly the existence oracle CLS-masking already refuses to
  provide (`sqlite.rs:470-477`). Same class: `patch` of a missing row flips
  from `SchemaViolation("record not found")` to `PermissionDenied`.
  **Accepted deliberately**, documented on the WIT (§3.6) and on
  `traits.rs`, and pinned by its own tests (§5.1). The policy-absent path
  (`auth: None`) keeps today's behavior exactly, so
  `test_delete_missing_record_is_idempotent` stays green unmodified.
- **Mode/operation.** `Ability::DATA_LAYER_WRITE`, never `DATA_LAYER_READ`.
  Entailment (`data-layer/admin` ⊇ `write` ⊇ `read`) means a read-only
  permission cannot authorize a write — the D2 rule `delete_many` already
  documents at `sqlite.rs:1828`.
- **Cross-service fetch.** Handled by the ingress's existing
  `resolve_query_auth`; a fetch failure is already mapped to
  `PermissionDenied` (`host_capabilities.rs:278`). Writes inherit B3's
  deny-on-timeout with no new mechanism — matrix row 6 extends to writes.
- **Watchdog.** `row_reachable` installs and drops `install_watchdog` per
  check, so a runaway predicate aborts to `Ok(false)` ⇒ `PermissionDenied` ⇒
  rollback. That is matrix row 5's "transaction rolled back, default-denied"
  satisfied literally on the write path, on the writer connection.
- **`delete_many` is untouched.** Already sieved as `data-layer/write` in
  `Mode::Filter`; no per-row check needed.

---

## 5. Tests

### 5.1 Unit — `crates/data_db/src/tests_fdae.rs`

Real `SqliteServiceStore`, real policy, one collection, two principals:

| Test | Asserts |
|---|---|
| `mode_a_write_denies_patch_of_an_unreachable_row` | `PermissionDenied`, row unchanged |
| `mode_a_write_denies_delete_of_an_unreachable_row` | `PermissionDenied`, row still present |
| `mode_a_write_denies_put_update_of_an_unreachable_row` | `PermissionDenied`, payload unchanged |
| `mode_a_write_allows_patch_of_a_reachable_row` | `Ok`, payload merged |
| `put_create_is_allowed_when_the_new_row_is_reachable` | `Ok`, row present (D-B5-2 `WITH CHECK`) |
| `put_create_is_denied_when_the_new_row_would_be_unreachable` | `PermissionDenied`, **no row inserted** (proves rollback, not just the error) |
| `patch_is_denied_when_the_post_image_escapes_the_callers_reach` | reachable→unreachable rewrite denied; original payload intact |
| `batch_mutate_rolls_back_entirely_when_one_mutation_is_unauthorized` | 3 mutations, #2 denied, **none** applied |
| `a_read_only_permission_does_not_authorize_a_write` | `allows: [data-layer/read]` only ⇒ denied |
| `writes_are_unfiltered_when_no_definition_matches_the_collection` | policy present, collection undefined, non-strict ⇒ `Ok` |
| `a_stage4_opted_permission_denies_single_row_writes_closed` | D-B5-4 |
| `fdae_watchdog_interrupt_denies_a_write_and_rolls_back` | matrix row 5 on the write path |
| `a_system_caller_write_is_denied_under_a_policy` | D-B5-3, with a comment naming the follow-up |
| `delete_of_a_missing_row_denies_under_a_policy_but_stays_idempotent_without_one` | §4's semantic change, both directions |
| `patch_of_a_missing_row_denies_rather_than_reporting_not_found` | §4, no existence oracle |
| `an_upsert_by_a_teammate_does_not_steal_creator_id` | **D-B5-6** — B `put`s over A's row on a shared-path policy; write succeeds, `creator_id` still A |
| `a_masked_field_cannot_be_written_on_create` | **D-B5-7** |
| `a_masked_fields_value_cannot_be_changed_on_update` | **D-B5-7**, including add and remove |

### 5.2 Unit — `crates/sandbox_wasm/src/host_capabilities.rs`

Alongside the existing `fdae_*` tests: guest `put`/`patch`/`delete` for an
`fdae_caller` who cannot reach the row ⇒ `PermissionDenied`; the same caller
against a reachable row ⇒ `Ok`.

### 5.3 Integration

- `router/tests/native_dispatch_identity.rs` —
  `native_fdae_policy_authorizes_writes_for_one_verified_caller_and_denies_another`,
  mirroring the read-side
  `native_fdae_policy_row_filters_and_masks_for_two_distinct_verified_callers`.
- `sandbox_wasm/tests/data_layer_integration.rs` — a guest-originated write
  with a real caller forwarded through `execute_wasm_json`'s `caller` param
  (the B3.5 wiring), authorized for one caller and denied for another.
- `router/tests/proxy_dispatch.rs` — update
  `guest_self_proxy_put_attributes_creator_id_to_the_real_caller_not_the_service`
  per D-B5-5 (it already asserts the chosen behavior).

### 5.4 Bench — `crates/data_db/benches/fdae_bench.rs`

New group: authorized `patch` and a 50-mutation authorized `batch_mutate`
against the unauthorized baseline. Claim to establish: the per-mutation
`EXISTS` (2 per row, 1 for `delete`, plus 2 payload reads under CLS) does not
dominate write latency. Record in `PERF_SUMMARY.md`; add a Performance
Budgets row to `task.md`.

---

## 6. Things in `task.md` / the ADRs that are stale or under-specified

1. **`put` is an upsert, not a create** (`sqlite.rs:176`,
   `INSERT … ON CONFLICT DO UPDATE`). `task.md` files *all* of `put` under the
   D-04-02-f-blocked create branch, but `put` on an existing id is an
   **update**, which the doc's own rule ("`patch`/`delete` of an existing row
   → Mode A") already covers. The blocked half is narrower than claimed — and
   the update branch has its own hazard the doc never anticipates (D-B5-6).
2. **`task.md` pre-commits D-04-02-f to a schema amendment** while ADR-0017
   §2.1's own Postgres analogy points at `WITH CHECK`, which needs none. A
   solution baked into a problem statement.
3. **ADR-0017 §2.1's Postgres analogy is half-drawn.** It maps `GRANT`→
   capability and `CREATE POLICY`(RLS)→`permissions:` and stops. RLS's
   `USING`/`WITH CHECK` split *is* D-04-02-f.
4. **ADR-0017 §4 describes Mode A as a *check primitive*** ("a new
   `check`-style host function"), never as host-enforced write authorization.
   B5 goes further than the ADR text. Amendment required.
5. **Tier 3 is "RLS + CLS" throughout `task.md`, but the B5 bullet is
   row-only.** It says "authorize single-row mutations … so a row a caller
   cannot reach is also one they cannot write" and never mentions columns —
   leaving a caller able to write a field they cannot read. D-B5-7 closes it;
   the task doc's scope line needs the column half added.
6. **"Thread `caller.session` into the host write methods (they don't today)"
   overstates the work.** Both ingresses already build a `data-layer/write`
   `QueryAuth` for `delete_many` via `resolve_query_auth`. The missing piece
   is the four `ServiceStore` signatures, not ingress plumbing.
7. **`host_capabilities.rs:205` cites `synsvc_native.rs::query_auth`** — that
   function is `resolve_query_auth`, and it contains **no** `AuthLevel::System`
   carve-out to "deliberately refuse" (it simply has none, which is the
   sentence's actual point). Stale name + misleading phrasing.
8. **`traits.rs:255`'s `check_access` doc** ("a policy-absent caller falls
   back to an existence check (D3)") is right for Mode A reads and **wrong as
   a write gate** — a create has no row, so an existence fallback would deny
   every create. Hence `row_reachable` rather than reusing `do_check_access`.
   The converse also matters: after B5, `check-access` and the host gate
   answer the same question two ways for a create (§3.6).
9. **Native dispatch has no `check-access` method.** `dispatch_data_layer` has
   arms for every other `store` function but not `check-access`, so an
   external native caller cannot ask Mode A at all — only WASM guests can, via
   WIT. `task.md` refers to "B2's `check_access` Mode-A primitive" as if it
   were universally reachable.
10. **`task.md`'s Failure/Security matrix says "All nine rows are done"** —
    B5 adds write-path rows, so that sentence and the count go stale.
11. **D-B5-3 is not mentioned anywhere.** Neither `task.md` nor ADR-0017
    observes that write enforcement makes policy-covered collections
    unwritable from every `System`-caller ingress.
12. **D-04-02-f's "a caller who cannot *see* a row can still
    `delete(id)`/`patch(id)` it"** is accurate, and the asymmetry is sharper
    than stated: `delete_many` with a filter matching exactly that row is
    already sieved and denies. Two ways to delete the same row, opposite
    answers.
13. **`native_dispatch_identity.rs:1034`'s comment** ("`put`/`create-collection`
    carry no FDAE gate … so any verified caller can seed fixture rows") is a
    correct statement of today that B5 falsifies. Delete it with the fixture
    fix (§3.7).

---

## 7. Deferred-backlog updates (mandatory, per AGENTS.md)

| Action | Row |
|---|---|
| Move to "Recently resolved" | "FDAE write-side Mode-A authorization" |
| Move to "Recently resolved" | "Self-proxy write attribution disagrees with the direct-WIT write path" — closed by D-B5-5 |
| Extend | "Stage-4 ABAC widens `aggregate`/`delete_many` denial" — add single-row `put`/`patch`/`delete`/`batch_mutate` (D-B5-4) |
| **New** | "`System`-caller writes deny closed under a policy" (D-B5-3): what breaks (guest-mediated writes only — the native ingress already refuses an anonymous caller), why deny-closed was chosen, and the follow-up (thread a principal into the anonymous-connection and proxied-WASM ingresses). Target: M04B follow-on |
| **New** | "`check-access` cannot answer a create pre-check" (§3.6): documented, not fixed; a hypothetical post-image answer needs a payload argument the WIT signature does not carry, with no current consumer. Target: TBD |
| **New**, only if D-B5-7 is dropped | "CLS is not enforced on the write path" |

---

## 8. Completion checklist

- [x] D-B5-2, D-B5-3, D-B5-5 answered (2026-07-26)
- [x] D-B5-6, D-B5-7 raised by independent review and resolved (2026-07-26)
- [ ] §3.7's three fixture families fixed **in the same commit** as §3.4
- [ ] `cargo +nightly fmt --all`
- [ ] `cargo clippy --workspace --all-targets --all-features` clean
- [ ] `cargo test --workspace` green
- [ ] `mise run test:e2e` green
- [ ] ADR-0017 amendment: write-side model (§2.1 `WITH CHECK`, §4 enforcement,
      §7 stage-4 write deny, CLS on writes), dated, in the Amendments section
- [ ] `task.md`: B5 marked complete; matrix rows added and the "All nine rows
      are done" line corrected; Performance Budgets row added; D-04-02-f
      flipped from ⛳ Open to resolved; the B5 scope line extended to columns
- [ ] `deferred-backlog.md` per §7
- [ ] `traceability-matrix.md` M4B row updated
- [ ] `PERF_SUMMARY.md` write-check numbers
- [ ] Import cleanup pass over every edited file (AGENTS.md)
