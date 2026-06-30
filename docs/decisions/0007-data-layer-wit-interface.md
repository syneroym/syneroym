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

**Missing record semantics:**
- Returning `data-layer-error::not-found` for a missing record forces every
  caller to handle an error path for a normal, expected state. A missing record
  on a `get` is commonly a valid application state, not an exceptional condition.

## Decision

### Query Interface

**Use MongoDB-style JSON query strings at the WIT boundary** (Option B).

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
The M3 `query` function returns raw records only.

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
- AggregationPipeline must be tracked as an explicit M4 gate item.
- The `migrate()` WIT export must be added to the `data-layer-guest` world and
  the `host.wit` world in Slice 3A.
