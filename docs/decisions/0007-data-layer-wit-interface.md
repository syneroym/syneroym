# ADR-0007: Data-Layer WIT Interface Design (D-03-02)

## Status

Accepted

## Context

`[PLT-DAT]` requires a typed WIT host function surface for structured data
operations. WASM guest components import `syneroym:data-layer/store` to perform
CRUD, batch mutations, and schema initialisation on per-service SQLite databases.

Two design axes required decisions:

**Filter/query representation:**
- Option A: Typed WIT `variant filter-expr { eq(...), in-list(...), ... }` —
  type-safe at the WIT boundary but verbose and difficult to evolve without WIT
  version bumps.
- Option B: MongoDB-style JSON string — the guest serialises a filter document
  as a JSON string; the host parses and compiles it to parameterised SQL. Full
  flexibility, familiar developer experience, handles complex joins, aggregates,
  and arbitrary filter combinations without WIT surface changes.

  MongoDB's operator vocabulary (equality, `$gt`/`$gte`/`$lt`/`$lte`/`$ne`,
  `$in`/`$nin`, `$and`/`$or`/`$not`) is not a Mongo-specific idiosyncrasy — it is
  the same minimal predicate algebra that REST filtering conventions (OData,
  JSON:API implementations, ad-hoc bracket-notation query params) converge on,
  and that SQL `WHERE` clauses express natively. Choosing Mongo's spelling of
  that algebra is deliberate for two reasons beyond familiarity: (1) it already
  matches common REST filter conventions, and (2) unlike "REST filtering" (which
  is not itself a standard), MongoDB's query and aggregation language is an
  actual mature, versioned specification (`$group`, `$project`, `$lookup`,
  `$unwind`, `$facet`, ...) that gives the M4 `AggregationPipeline` work a
  concrete, well-precedented target to translate into SQLite constructs, rather
  than inventing a bespoke aggregation DSL feature-by-feature. Precedent for
  "Mongo query/aggregation language, non-Mongo engine underneath" translation
  layers exists (e.g. FerretDB on PostgreSQL/SQLite).

**Missing record semantics:**
- Returning `data-layer-error::not-found` for a missing record forces every
  caller to handle an error path for a normal, expected state. A missing record
  on a `get` is commonly a valid application state, not an exceptional condition.

## Decision

### Query Interface

**Use MongoDB-style JSON query strings, compiled to parameterised SQLite, at the
WIT boundary** (Option B). The underlying engine is — and remains — always
SQLite; "MongoDB-style" describes the wire syntax and operator vocabulary of
the `filter` string only, never the execution engine. Every reference to
"MongoDB-style" in this document and downstream docs should be read with that
pairing implicit.

The `query` function accepts a `filter: option<string>` where the string is a
JSON-encoded filter document following MongoDB query operator conventions:

```wit
query: func(
    collection: string,
    filter:     option<string>,   // JSON query document, e.g. {"age": {"$gt": 18}}
    options:    query-options,
) -> result<query-result, data-layer-error>;
```

The host parses the JSON filter document and compiles it to a parameterised
SQLite `WHERE` clause. All values extracted from the JSON document are bound as
`?` parameters — no interpolation of untrusted input into SQL strings.

Supported operators in M3A scope:
- Field equality: `{"field": value}`
- Comparison: `$gt`, `$gte`, `$lt`, `$lte`, `$ne`
- Set membership: `$in`, `$nin`
- String pattern: `$regex` (compiled to SQLite `LIKE` with `%` anchors; full
  regex deferred)
- Logical: `$and`, `$or`, `$not`
- JSON path access for nested fields: dot notation (`"address.city": "London"`)
  compiled to `json_extract(payload, '$.address.city')`

**AggregationPipeline** (`$group`, `$having`, projections) is **deferred to M4**.
The M3 `query` function returns raw records only. When designed, it should
translate MongoDB aggregation-pipeline stages to SQLite constructs (`GROUP BY`,
`HAVING`, views) rather than invent independent syntax — see the rationale
above. Implemented in M04A Slice B4; see "Amendments" below.

### Full-Power Escape Hatch (Trusted Contexts)

The safe, whitelisted JSON filter DSL above is deliberately the only query
surface available to ordinary (non-lifecycle) guest invocations — it is safe by
construction and requires no SQL-grammar validation. Services that need full
SQL expressivity beyond the whitelisted operator set (arbitrary joins, window
functions, CTEs) may use a raw-SQL query path gated by the **same trust
boundary already used for `execute-ddl`** (`HostState.is_init_context` today;
an Admin UCAN capability in M4 — see the M4 TODO tracked in
`docs/planning/milestones/M03-sss/task.md`). This is a distinct WIT host
function, not an extension of `query`'s filter grammar, and is specified in
[ADR-0011](0011-privileged-raw-sql-query.md).

### Missing Record Semantics

- `get(collection, id) -> result<option<record-value>, data-layer-error>`
  — returns `Ok(None)` when the record does not exist. `Err(...)` is reserved
  for storage failures, permission errors, and schema violations.
- `query(...)` returns an **empty list** (not an error) when no records match
  the filter.

This aligns with the principle that "no result" is a valid query outcome, not
an exceptional condition requiring error-path handling in every caller.

### WIT Package and World

```wit
package syneroym:data-layer@0.1.0;
world data-layer-guest {
    import store;
}
```

### Schema Lifecycle Exports (Slice 3A)

Two separate guest exports define the schema lifecycle:

- `init()` — called by the host on **first deploy only** (fresh database).
  Has access to `execute-ddl` capability. Used for `CREATE TABLE`, `CREATE
  INDEX`, and seed data insertion.
- `migrate()` — called by the host on **re-deploy** (existing database).
  Has access to `execute-ddl` capability. Used for additive `ALTER TABLE ADD
  COLUMN`, new `CREATE INDEX`, and data transformations. Both additive and
  destructive DDL are permitted; the developer is responsible for safety.
  (Full snapshot/rollback safety net is deferred to M5 `[LFC-VER]`.)

Both hooks execute with `HostState.is_init_context = true`, which is the gate
that allows `execute-ddl` to forward SQL to the `ServiceDb`.

### Host-Injected Fields

The following fields on every record are **set exclusively by the host** and
cannot be overridden by the WASM guest:

- `creator_id` — set to `HostState.component_id` (the service's DID-key string).
- `created_at` — set to the host's current Unix timestamp in milliseconds on
  first `put`. Immutable thereafter.
- `updated_at` — set to the host's current Unix timestamp in milliseconds on
  every `put` or `patch`. The guest-supplied value, if any, is silently
  discarded.

## Consequences

- The JSON filter compiler in the host (`crates/data-layer`) must be robust
  against malformed JSON, unsupported operators, and deeply nested documents
  (add a maximum nesting depth guard, e.g., 10 levels).
- Error messages from the JSON compiler must be structured
  (`data-layer-error::schema-violation(string)`) to give guests actionable
  diagnostics without leaking internal query structure.
- Full MongoDB operator compatibility is explicitly out of scope. The host only
  implements the operators listed above. Attempting an unsupported operator
  returns `data-layer-error::schema-violation("unsupported operator: $lookup")`.
- AggregationPipeline must be tracked as an explicit M4 gate item, and its
  design should map MongoDB aggregation-pipeline stages onto SQLite constructs
  rather than invent parallel syntax.
- The privileged raw-SQL escape hatch ([ADR-0011](0011-privileged-raw-sql-query.md))
  is additive WIT surface, not a modification of `query`'s filter grammar, and
  ships no earlier than M3B/M4.
- The `migrate()` WIT export must be added to the `data-layer-guest` world and
  the `host.wit` world in Slice 3A.

## Amendments

**2026-07-16 (M04A Slice B4).** `AggregationPipeline` was deferred above and
in `task.md` as "on the `query` function"; implementation (plan:
`docs/planning/milestones/M04A-proxy-and-auth-foundation/plans/B4.md`) recorded
three shape decisions before coding, following B5's precedent of confirming
return-shape/DSL choices with the requester rather than guessing:

- **Separate `aggregate` function, not an extension of `query`.** `query`
  returns `query-result` = `list<record-read-value>`, a fixed
  `{id, payload, creator-id, created-at, updated-at}` shape that cannot
  represent a grouped/projected result (`SELECT category, count(*) …`) — the
  same mismatch B5 hit and resolved with `raw-query-result` (see ADR-0011).
  `aggregate: func(collection: string, pipeline: string) ->
  result<raw-query-result, data-layer-error>` reuses that same record rather
  than inventing a second one. "On the `query` function" (task.md, and the
  original wording above) is read as "on the `query` capability/DSL family",
  not literally the `query` WIT function.
- **The DSL is a single JSON object, not an ordered pipeline array.** The
  document has optional keys `$match`/`$group`(required)/`$having`/`$project`/
  `$sort`/`$limit`/`$skip`, compiled in one deterministic pass. Narrower than
  MongoDB's ordered `[{…},{…}]` pipeline, but covers exactly the operators
  this milestone names (`$group`/`$having`/projections) and is far simpler to
  compile and validate than a general multi-stage pipeline.
- **Physical collections only; init-defined logical views are deferred.**
  Field access (`_id`, accumulator arguments, `$match`) assumes the physical
  `{id, payload}` row shape (`json_extract(payload, '$.field')`), which does
  not hold for an arbitrary `CREATE VIEW`. Aggregating over init-defined
  logical views, named in `task.md`'s original scope, is deferred to a
  follow-on (would need `PRAGMA table_info` introspection to distinguish bare
  columns from `json_extract`-backed ones).

Two further notes carried into `status.md`'s B4 section rather than repeated
here in full:

- **Payload-only field access is deliberate consistency with `query`, not a
  narrowing of it** — `filter.rs`'s existing filter DSL (reused verbatim for
  `aggregate`'s `$match`) has zero physical-column awareness either; neither
  DSL can currently filter/group by `creator_id` or the other host-injected
  columns. If host-column access is wanted later, it should be added to both
  `filter.rs` and the aggregation compiler together, to keep the two DSLs
  symmetric.
- **Forward seam for M04B FDAE.** `aggregate` is a second, independent read
  path into `payload` that does not flow through `query`'s compiler. M04B's
  FDAE RLS/CLS pushdown sieve (`docs/planning/milestones/M04B-fdae-policy/task.md`)
  is currently scoped only to `data-layer::query`; unless M04B also wraps
  `aggregate`, a caller row-restricted on `query` could read the same rows in
  aggregate form via this path. `aggregate`'s `$match` stage already compiles
  through `filter::compile_filter`, the same seam M04B's sieve hooks for
  `query`, so wiring an injected RLS predicate into `aggregate` when M04B
  lands is a matter of remembering to do it, not a structural blocker. This
  slice records the gap; it does not close it.

No signature or gate change to any function this ADR already covers
(`query`, `execute-ddl`, `query-raw`) — `aggregate` is additive WIT surface,
requires no capability gate (same trust level as `query`), and ships no
earlier than M4A.
