# ADR-0011: Privileged Raw-SQL Query Escape Hatch

## Status

Accepted (amended 2026-07-15, M04A Slice B5 — see "Amendments" below)

## Context

[ADR-0007](0007-data-layer-wit-interface.md) gives ordinary (non-lifecycle)
guest invocations a safe-by-construction JSON filter DSL — a whitelisted,
MongoDB-conventional operator vocabulary compiled to parameterised SQLite. That
DSL intentionally does not, and will not, cover the full expressivity of SQL:
arbitrary joins across collections, window functions, CTEs, and constructs the
whitelist doesn't anticipate.

Rather than continuously widening the JSON filter grammar to chase full SQL
expressivity (which would eventually require a general-purpose SQL-in-JSON
grammar and erode the "safe by construction, no SQL parsing required" property
that makes the default path low-risk), trusted services should instead have an
explicit, separately-gated path to full raw SQL. This mirrors the existing
`execute-ddl` design (ADR-0007): plain SQL is acceptable there because it is
strictly gated to a trusted lifecycle context (`init`/`migrate`), not because
DDL text itself is safe.

Design questions:
1. Should raw SQL be exposed at the WIT boundary at all, or stay host-internal
   (e.g. FDAE policy compilation, control-plane tooling)?
2. If exposed to guests, what gates access?
3. How does this interact with the per-service DB isolation model?

## Decision

### Exposed at the WIT Boundary, Gated Like DDL

Raw SQL querying **is** exposed to WASM guests, through a new function
distinct from `query`:

```wit
query-raw: func(sql: string, params: list<sql-value>) -> result<raw-query-result, data-layer-error>;

variant sql-value {
    text(string),
    integer(s64),
    real(f64),
    boolean(bool),
    null,
}

record raw-query-result {
    columns: list<string>,
    rows: list<list<sql-value>>,
}
```

`sql` is the guest-authored SQL text; `params` are always bound via `?`
placeholders — the host never interpolates guest-controlled values into the
SQL string, even in this privileged path. This preserves the injection-safety
property of ADR-0007 for *values* while relaxing the constraint on
*predicate/query shape*.

### Gate: Same Trust Boundary as `execute-ddl`

`query-raw` is only invokable when the caller holds the `data-layer/admin`
capability on the service's own resource (ADR-0015/0016) — the same gate M04A
Slice B0 wired for `execute-ddl`, replacing the interim `is_init_context`
scaffold this ADR originally proposed reusing directly. Outside that context,
`query-raw` returns `data-layer-error::permission-denied`, exactly as
`execute-ddl` does.

This is a deliberate reuse, not a new authorization concept: the platform
already accepts that a service's `init`/`migrate` lifecycle hooks are trusted
with arbitrary DDL against their own isolated database; extending that same
trust boundary to arbitrary DML/query SQL against the same database introduces
no new blast radius, since a trusted DDL author could already reshape the
schema and read/write data indirectly.

Ordinary (non-lifecycle) invocations of a service keep using the safe `query`
JSON filter DSL from ADR-0007 — `query-raw` does not replace it.

### Database Isolation Unaffected

`query-raw` executes against the same per-service, encrypted SQLite connection
as every other data-layer call. It cannot reach another service's database;
the existing envelope-encryption and DB-isolation model (ADR-0006) applies
unchanged.

### Return Shape

`query-raw` returns `raw-query-result { columns: list<string>, rows:
list<list<sql-value>> }`, not `query`'s fixed `query-result { records:
list<record-read-value>, ... }` shape. `record-read-value` is a fixed
`{id, payload, creator-id, created-at, updated-at}` record and cannot
represent an arbitrary projection or aggregation (e.g. `SELECT category,
count(*) ... GROUP BY category`) — which is this ADR's own motivation and the
milestone's reference-scenario step 24 ("a report needing a join"). Returning
the original `query-result` signature here would have been self-contradictory
against that motivation, so `raw-query-result` — an arbitrary column/row
projection, positionally aligned — replaces it in this ADR's signature.

### Read-Only Enforced

`query-raw` accepts **read-only statements only**, enforced via two layers on
the concurrent reader pool:

1. `rusqlite::Statement::readonly()` (SQLite classifies the compiled
   statement — no SQL parser needed on the host side) rejects any statement
   that writes the database's *content*.
2. An authorizer callback additionally denies `ATTACH`/`DETACH`/`BEGIN`/a
   value-setting `PRAGMA`. `Statement::readonly()` reports `true` for all
   four — SQLite's own docs for `sqlite3_stmt_readonly()` note this gap,
   since none of them write the main database file's *content*, only the
   connection's configuration. `ATTACH DATABASE '<path>' AS x` in particular
   creates `<path>` on the host filesystem as a side effect (confirmed
   empirically) and would let an admin caller read or write outside its own
   per-service database, defeating the "Database Isolation Unaffected"
   guarantee above without layer 2.

Writes keep using `put`/`patch`/`batch-mutate`; schema changes keep using
`execute-ddl` (the single-writer loop). A rejected statement returns
`permission-denied`. This is a deliberate narrowing from this ADR's original
"arbitrary DML/query SQL" framing: `query-raw` is a *query* escape hatch, not
a raw-write path — the existing mutation surface already covers writes with
its own validation (creator-id attribution, batch-size limits), which a raw
write path would silently bypass.

A third mechanism bounds *compute* independent of the above: a
`Connection::progress_handler` interrupts a statement after a large but
finite number of virtual-machine instructions, since the row-count page cap
alone does not stop a recursive CTE or unconstrained cross join that does
unbounded work while returning few or no rows.

## Consequences

- `syneroym:data-layer/store` gains `query-raw`, `sql-value`, and
  `raw-query-result` in the WIT package version that ships this ADR (a minor
  version bump, not `@0.1.0` in place — additive, non-breaking).
- `query-raw` is **not** available in M3A. It ships in M04A Slice B5, gated by
  the Admin UCAN capability Slice B0 delivered — the interim `is_init_context`
  gate this ADR originally proposed reusing was replaced before `query-raw`
  itself shipped.
- Full MongoDB operator/aggregation compatibility remains explicitly out of
  scope for `query` (ADR-0007 unchanged); `query-raw` is the intended relief
  valve for expressivity `query` will never grow, rather than a reason to keep
  expanding the JSON filter grammar indefinitely.
- Tests must cover: `query-raw` rejected with `permission-denied` outside a
  trusted context; parameter binding never interpolates guest values into SQL
  text (same injection test shape as ADR-0007's filter compiler); a mutating
  statement rejected with `permission-denied` (read-only enforcement); an
  `ATTACH`/`DETACH`/`BEGIN`/value-setting-`PRAGMA` statement rejected with
  `permission-denied` and confirmed to create no file on the host filesystem
  (the `Statement::readonly()`-reports-true gap, layer 2 above); a BLOB
  column rejected with `schema-violation` (`sql-value` has no `blob` arm); a
  statement doing unbounded compute (e.g. an unterminated recursive CTE)
  interrupted with `quota-exceeded` rather than hanging the connection;
  `query-raw` cannot reference another service's database or files (isolation
  holds).
- Must appear as an explicit gate item in the M4 (or M3B, whichever ships it)
  milestone `task.md`, alongside `AggregationPipeline`.

## Amendments

**2026-07-15 (M04A Slice B5).** This ADR's original signature
(`query-raw: func(...) -> result<query-result, data-layer-error>`) and gate
(`HostState.is_init_context`) were never implemented as originally proposed;
both were superseded before implementation, and this ADR is amended in place
rather than superseded by a new one, per the implementation slice's own plan
(`docs/planning/milestones/M04A-proxy-and-auth-foundation/plans/B5.md` §0.1):

- **Return shape** changed from the fixed `query-result` to the new
  `raw-query-result` (see "Return Shape" above) — the original signature
  could not represent the arbitrary projections/aggregations this ADR's own
  motivation and the milestone's reference scenario require.
- **Gate** changed from `HostState.is_init_context` to the `data-layer/admin`
  Admin UCAN capability (ADR-0015/0016) — the M4 capability model this ADR
  always deferred to shipped in Slice B0, so `query-raw` (Slice B5) uses it
  directly rather than landing on the interim scaffold first.
- **Read-only enforcement** added — not present in the original Decision text,
  which gestured at "arbitrary DML/query SQL against the same database"
  without specifying enforcement. Writes/DDL keep their existing dedicated
  paths (see "Read-Only Enforced" above).
- Status moved from *Proposed* to *Accepted*, reflecting that Slice B5 has
  implemented and tested the (amended) decision.
