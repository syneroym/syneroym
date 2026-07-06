# ADR-0011: Privileged Raw-SQL Query Escape Hatch

## Status

Proposed

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
query-raw: func(sql: string, params: list<sql-value>) -> result<query-result, data-layer-error>;

variant sql-value {
    text(string),
    integer(s64),
    real(f64),
    boolean(bool),
    null,
}
```

`sql` is the guest-authored SQL text; `params` are always bound via `?`
placeholders — the host never interpolates guest-controlled values into the
SQL string, even in this privileged path. This preserves the injection-safety
property of ADR-0007 for *values* while relaxing the constraint on
*predicate/query shape*.

### Gate: Same Trust Boundary as `execute-ddl`

`query-raw` is only invokable when `HostState.is_init_context` is `true` (the
same flag that gates `execute-ddl` today per ADR-0007) **or** — once available
— an Admin UCAN capability, per the M4 TODO already tracked at every
`is_init_context` call site (`docs/planning/milestones/M03-sss/task.md`, Open
Question #2). Outside that context, `query-raw` returns
`data-layer-error::permission-denied`, exactly as `execute-ddl` does today.

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

## Consequences

- `syneroym:data-layer/store` gains `query-raw` and `sql-value` in the WIT
  package version that ships this ADR (a minor version bump, not `@0.1.0`
  in place — additive, non-breaking).
- `query-raw` is **not** available in M3A. It ships no earlier than M3B/M4,
  alongside (or after) the M4 Admin UCAN capability work, since the interim
  `is_init_context` gate is explicitly a temporary M3 scaffold slated for
  replacement.
- Full MongoDB operator/aggregation compatibility remains explicitly out of
  scope for `query` (ADR-0007 unchanged); `query-raw` is the intended relief
  valve for expressivity `query` will never grow, rather than a reason to keep
  expanding the JSON filter grammar indefinitely.
- Tests must cover: `query-raw` rejected with `permission-denied` outside a
  trusted context; parameter binding never interpolates guest values into SQL
  text (same injection test shape as ADR-0007's filter compiler); `query-raw`
  cannot reference another service's database or files (isolation holds).
- Must appear as an explicit gate item in the M4 (or M3B, whichever ships it)
  milestone `task.md`, alongside `AggregationPipeline`.
