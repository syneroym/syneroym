# Slice B2 — Local FDAE (SQL Pushdown Sieve): Implementation Plan

> Planning artifact for M04B Slice B2. Grounds every change against `main`
> @ `bfc18a5`. Reads ADR-0017 (FDAE schema/compilation) and ADR-0015
> (UCAN/grant layer) as the pair they were designed as. **Not yet implemented.**
> Section 12 lists ambiguities/ADR gaps that must be pinned before or during
> coding — read it first if you are executing this.

---

## 0. Scope of B2

In: the policy **model** (typed, versioned, JSON-Schema-validated), the
ReBAC→SQL **compiler** (Mode A point-in-time + Mode B relational filter, RLS
row-pruning + CLS column masking, `WHERE EXISTS`/`WITH RECURSIVE`, path-concat
cycle guard, `sqlite3_progress_handler` watchdog with default-deny on timeout,
strict `?` binding), the **merge** with the ADR-0007 `CompiledFilter`, the
per-request threading of the caller `SessionContext` + service policy into the
data-layer read path, `strict:` mode, policy-internal default-deny, and the
decision trace. Reference-scenario step 22 (transparent Mode-B filtering).

Out (later slices, keep seams open): cross-service relation resolution / stage-2
fetch (B3 — parse `service:` relations but fail-closed if evaluation needs a
remote hop); stage-4 `authorize-rows` WASM ABAC (B4-fdae); `anchor`/`path`
terminals (need ADR-0015 A5's unshipped `SessionContext` fields — see §12.4);
replication/staleness (M7).

---

## 1. Key architectural decisions (recommendations)

### 1.1 New crate `syneroym-fdae` (`crates/fdae/`) — recommended

The policy model + JSON-Schema validation + ReBAC→SQL compiler is a substantial,
self-contained unit that depends on `syneroym-ucan` (`SessionContext`,
`Capability`, `Ability`, `ResourceUri`). Putting it in its own crate keeps
`data_db` a storage crate (today it depends on neither `ucan` nor `jsonschema`)
and lets the compiler be unit-tested in isolation.

- `crates/fdae/Cargo.toml` → package `syneroym-fdae`. Deps: `syneroym-ucan`,
  `serde`, `serde_json`, `rusqlite` (for `rusqlite::types::Value` params —
  identical to `CompiledFilter` so the merge is glue-free), `jsonschema` (0.46,
  already a workspace dep), `thiserror`, `anyhow`. No `wasm32-wasip2` build
  required (host-side only, same as `ucan`).
- Add `syneroym-fdae = { path = "crates/fdae" }` to the root `Cargo.toml`
  `[workspace.dependencies]` block (after line 57, next to `syneroym-ucan`).
- `data_db` gains `syneroym-fdae.workspace = true`. No cycle:
  `fdae → ucan → identity`; `data_db → data_keystore → identity` and
  `data_db → fdae`. `ucan`/`identity`/`fdae` never depend back on `data_db`.

**Alternative considered:** modules inside `data_db` (`src/fdae/policy.rs`,
`src/fdae/compile.rs`) + a `ucan` dep on `data_db`. Slightly less glue (the
compiler's only consumer is `do_query` in the same crate, same param type), but
widens `data_db`'s dependency surface with policy/schema concerns and a `ucan`
dep. Recommend the crate; this is the one structural call worth a human nod
before coding.

### 1.2 Compile per request, from a parse-once model

The policy is parsed+validated **once** per service (deploy time), then held as
a typed `Policy` (`Arc<Policy>`). The SQL sieve is generated **per request**
because its bound `?` values (caller DID, caveat/claims scalars) vary per caller;
the `WHERE EXISTS` text itself is cheap string assembly over the pre-parsed
model. (Budget: +5 ms p99 per the perf table — per-request assembly is well
inside it. Caching the parameterized SQL *template* string keyed by
`(object_type, permission_set)` is a noted follow-up, not required for B2.)

### 1.3 Threading: compile in the async store method, before the blocking closure

`do_query` runs inside `conn.interact(move |conn| …)` on the reader pool, so
anything it touches must be owned + `Send + 'static`. Therefore compile the
sieve to an owned `CompiledSieve { where_clause: String, params: Vec<Value> }`
**in the async `query`/`check_access` method** (which can borrow the
`SessionContext`), then move it into the closure alongside `opts`. The
`SessionContext`/`Policy` never cross the closure boundary — only the compiled,
owned SQL does. This mirrors how `opts` is already moved in today
(`sqlite.rs:1341`).

### 1.4 Policy source at runtime: persist like `config_generations`

`open_service_db`/`HostState` are constructed per request/instantiation and the
deploy-time policy file is not on disk at query time. Persist the validated
policy document in `substrate.db` exactly as app config is
(`save_config_generation`/`get_latest_config_generation`, `sqlite.rs:1036-1135`)
and load it at component instantiation (next to `config_generation`,
`engine.rs:634-669`). See §9. This is the largest plumbing chunk and is
**Phase 4** — the compiler + store integration (Phases 1–3) can land and be
tested with a policy injected directly in tests before deploy plumbing exists.

---

## 2. The policy model — `crates/fdae/src/policy.rs`

Typed deserialization target for the `fdae/v1` document (ADR-0017 §1). No runtime
string lexers.

```rust
use std::collections::BTreeMap;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Policy {
    pub version: String,                       // must equal "fdae/v1" (validated)
    #[serde(default)]
    pub strict: bool,                          // D-04-02-c, top-level, default false
    pub definitions: BTreeMap<String, Definition>, // key = logical object type
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Definition {
    pub table: String,                         // physical collection/table name
    /// Column on `table` whose value is the principal DID a `caller`/`anchor`
    /// terminal compares against, when this object type is a path terminal's
    /// target (see §12.1). Reserved-name aware: "creator_id"/"id" map to the
    /// physical column, any other name maps to json_extract(payload,'$.<name>').
    #[serde(default)]
    pub principal_column: Option<String>,
    #[serde(default)]
    pub relations: BTreeMap<String, Relation>,
    #[serde(default)]
    pub permissions: BTreeMap<String, Permission>,
    /// D-04-02-b: names the permission applied when a caller reaches this
    /// object via a grant but no permission is otherwise selected. Absent ⇒
    /// default-deny within the policy.
    #[serde(default)]
    pub default: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Relation {
    pub target: String,                        // object type OR (remote) a service ref
    /// Remote relation (ADR-0017 §1/§6): names a logical service resolved via
    /// the app-context registry. B2 PARSES it but fails closed at compile if a
    /// selected path traverses it (B3 territory). Mutually exclusive with a
    /// local join.
    #[serde(default)]
    pub service: Option<String>,
    // -- local single-hop join --
    #[serde(default)]
    pub join_column: Option<String>,           // column on THIS table -> target key
    // -- recursive self-join (folds old `hierarchies`, ADR-0017 §1) --
    #[serde(default)]
    pub from_key: Option<String>,
    #[serde(default)]
    pub to_key: Option<String>,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Permission {
    #[serde(default)]
    pub allows: Vec<String>,                   // platform ops, e.g. ["data-layer/read"]
    #[serde(default)]
    pub operator: Operator,                    // across this permission's paths
    #[serde(default)]
    pub paths: Vec<Vec<String>>,               // each: [relation..., terminal]; [] == public
    #[serde(default)]
    pub conditions: Vec<Condition>,            // attribute predicates (D-04-02-a claims)
    #[serde(default)]
    pub includes: Vec<String>,                 // declared entailment (never derived)
    #[serde(default)]
    pub fields: Option<FieldsPolicy>,          // CLS
}

/// An attribute predicate binding a caller `claim` (or capability caveat
/// scalar) against a row column — the mechanism D-04-02-a's "claims (scalars)
/// bind directly as `?`" requires. `column` names a row column/JSON path
/// (§3.4 `<col>` rules); `claim` names a key in `session.claims`. Compiles to
/// `<col(def, column)> <op> ?` with `?` bound to `session.claims[claim]`.
/// A referenced claim absent from `session.claims` ⇒ the condition is false
/// (fail-closed), never skipped.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub column: String,
    pub claim: String,
    #[serde(default)] pub op: CondOp,          // eq|ne|gt|gte|lt|lte, default eq
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CondOp { #[default] Eq, Ne, Gt, Gte, Lt, Lte }

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Operator { #[default] Union, Intersection, Exclusion }

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FieldsPolicy {                      // ADR-0015 A3 shape
    #[serde(default)] pub allow: Option<Vec<String>>,
    #[serde(default)] pub deny: Option<Vec<String>>,
}
```

Terminal tokens inside a `paths` entry: reserved string `"caller"` (B2, binds
`session.subject_did`); `"anchor"` is **deferred to B3** (§12.4) — a policy using
it in B2 is a compile-time `Semantic` error. Everything else in a path is a
relation name. `principal_column` names an existing column the guest created at
init/migrate — the policy only references it, never creates it, so its existence
is validated lazily at query time (fail-closed with a trace entry if missing),
not at parse time (the table may not exist yet when the policy is parsed).

### 2.1 `Policy::from_str` + validation — `policy.rs`

```rust
pub fn parse_and_validate(doc: &str) -> Result<Policy, PolicyError> {
    // 1. JSON-Schema validate the raw doc against the embedded fdae/v1 schema
    //    (crates/fdae/schema/fdae-v1.json, include_str! + jsonschema::validator_for,
    //    the exact pattern control_plane/service/orchestration.rs:309 uses).
    // 2. serde_json::from_str::<Policy> (accept JSON; YAML input is converted to
    //    JSON by the deploy tool — B2 stores/loads JSON, matching config blobs).
    // 3. Semantic validation (schema can't express these):
    //    - version == "fdae/v1"
    //    - every Relation has exactly one shape: local join_column XOR
    //      (from_key+to_key+recursive) XOR remote `service`.
    //    - every relation `target` resolves to a definition key (unless remote).
    //    - every path: non-terminal segments are relation names on the walked
    //      object type; final segment is a terminal ("caller"/"anchor").
    //    - `includes`/`default` name real permissions; includes is acyclic.
    //    - any object type used as a path-terminal target declares
    //      `principal_column` (else error — §12.1).
    //    - strict-mode author-time warning is emitted by the DEPLOY tool, not
    //      here (§10).
}
```

`PolicyError` (`thiserror`): `Schema(String)`, `Semantic(String)`,
`UnsupportedVersion(String)`.

---

## 3. The compiler — `crates/fdae/src/compile.rs`

### 3.1 Public API

```rust
use rusqlite::types::Value;
use syneroym_ucan::SessionContext;

/// The compiled security block, shaped exactly like data_db's CompiledFilter so
/// the two AND together with no conversion.
#[derive(Debug, Clone)]
pub struct CompiledSieve {
    pub where_clause: String,   // a boolean SQL expr over the base table's columns
    pub params: Vec<Value>,     // bound values, in binding order
    pub masked_fields: Vec<String>, // CLS: payload JSON fields to strip post-fetch (§6)
}

pub enum Mode { Filter, PointInTime { id: String } } // Mode B / Mode A

/// Compile the row-security block for a read of `collection` by `session`
/// under `policy`. Returns:
///   Ok(None)            -> no definition for this collection AND policy not
///                          strict: grant layer already admitted, no filtering.
///   Ok(Some(sieve))     -> apply this block (may be a deny-all `0=1`).
///   Err(PolicyError)    -> malformed/compile failure -> caller treats as deny.
pub fn compile_read(
    policy: &Policy,
    collection: &str,
    session: &SessionContext,
    mode: Mode,
) -> Result<Option<CompiledSieve>, PolicyError>;
```

The data-layer read operation maps to platform ability `data-layer/read`
(`Ability::DATA_LAYER_READ`). `compile_read` uses that as the operation for
branch selection.

> **Implementation note (Phase 1 landed, PR #86):** the shipped signature is
> `compile_read(policy, collection, session, service_id, operation, mode)` --
> two params beyond this sketch. `service_id` is needed to build the
> collection-qualified resource §3.2 itself requires; `operation: &Ability`
> is needed so Mode A can distinguish a read check from a write check (the
> signature above has no way to do that, which would let a read-only
> capability pass a write-mode `check-access`). `CompiledSieve` also gained
> `where_caveats: Vec<serde_json::Value>` per §6's decision. Treat
> `crates/fdae/src/compile.rs` as ground truth over this section.

### 3.2 Branch selection (grant ∩ policy) — the D-04-02-a/-b core

```
compile_read(policy, collection, session, mode):
  def = definition whose key == collection OR whose .table == collection
  if def is None:
      return Ok(None) unless policy.strict           # §2.1, ADR-0017 §2.1
      if policy.strict: return Ok(Some(deny_all()))  # unknown resource denied

  # The resource a capability is checked against MUST be the
  # collection-QUALIFIED resource, not the bare base:
  resource = ResourceUri(
      ResourceUri::service(service_id, service_id).0 + "/collection/" + collection)
      # CRITICAL (review finding): checking against the BARE
      # ResourceUri::service(service_id, service_id) breaks the grant∩policy
      # intersection. `covers_resource` (capability.rs:87) with the shipped test
      # `selector_scoped_capability_does_not_cover_the_bare_base`
      # (capability.rs:547) means a caller holding a legitimately narrowed
      # `.../collection/orders` selector grant (ADR-0015 A1's canonical
      # data-layer example) does NOT cover the bare base -> the compiler would
      # deny them regardless of policy, i.e. FDAE overriding the grant layer
      # instead of intersecting it. The collection-qualified resource covers
      # BOTH: a bare/service-granularity grant still covers every selector on
      # the base (test `no_selector_covers_every_selector_on_the_same_base`),
      # AND a `/collection/<name>` selector grant covers the exact collection.
      # Use `Capability::grants(resource, ability)` (which internally applies
      # covers_resource) with this qualified resource.

  applicable = {}                                    # set of permission names
  for (pname, perm) in def.permissions:
      entitled =
          # (a) platform-ability route: some held capability grants a platform
          #     ability A on `resource` with A ∈ perm.allows (entailment via
          #     Ability::entails)
          any cap in session.capabilities:
              any a in perm.allows: cap.grants(resource, Ability(a))
        OR
          # (b) app-permission route: held `can: app/<objtype>.<pname>`
          any cap in session.capabilities:
              cap.grants(resource, Ability(format!("app/{objtype}.{pname}")))
      if entitled: applicable.insert(pname)
  applicable = close_over_includes(applicable, def)  # add P if an applicable perm includes P

  if applicable empty:
      if def.default is Some(dp) and caller-can-reach(dp): applicable = {dp}
      else: return Ok(Some(deny_all_with_mode(mode)))   # D-04-02-b default-deny

  # RLS predicate = OR over each applicable permission's path predicate
  # (union across permissions; ADR-0017 §2/§3).
  rls = OR_j( compile_permission(def, applicable[j], session, &mut params) )

  # Mode A: AND the id equality (ADR-0017 §4)
  if mode == PointInTime{id}: rls = "({rls}) AND {table}.id = ?"; params.push(id)

  # Caveat `where` filters (D-04-02-a): each applicable capability's
  # caveats.where (a MongoDB filter) compiled via data_db filter DSL and ANDed.
  # (Compiled in data_db, not here — see §5/§6, to avoid a second filter
  # compiler in fdae; fdae returns rls+masked_fields, data_db ANDs the caveat
  # where + CompiledFilter.)  <-- decision: keep caveat-where merge in data_db.

  masked = compile_cls(def, applicable, session)     # §6

  return Ok(Some(CompiledSieve { where_clause: rls, params, masked_fields: masked }))
```

`deny_all()` → `CompiledSieve { where_clause: "0=1".into(), params: vec![],
masked_fields: vec![] }` (Mode B: zero rows; Mode A: boolean false).

### 3.3 `compile_permission` — paths → SQL, with operator

```
compile_permission(def, perm, session, params):
  # Path predicate (ReBAC reachability)
  if perm.paths is empty: path_pred = "1=1"      # `public` (ADR-0017 §1)
  else:
    clauses = [ compile_path(def, p, session, params) for p in perm.paths ]
    match perm.operator:
      Union        -> path_pred = "(" + clauses.join(" OR ")  + ")"
      Intersection -> path_pred = "(" + clauses.join(" AND ") + ")"
      Exclusion    -> path_pred = "(" + clauses[0] + " AND NOT (" + clauses[1..].join(" OR ") + "))"
  # Attribute conditions (D-04-02-a: bind session.claims[k] as ?). ANDed onto
  # the path predicate — they only ever further-restrict.
  for c in perm.conditions:
    val = session.claims.get(c.claim)
    if val is None: return "0=1"                  # fail-closed: referenced claim absent
    params.push(json_value_to_sql(val))
    path_pred = path_pred + " AND " + col(def, c.column) + sql_op(c.op) + " ?"
  return path_pred
```

### 3.4 `compile_path` — the relation walk (the genuinely hard part)

A path is `[rel_1, rel_2, …, rel_n, terminal]`. Starting object type = `def`
(the base table `T`). Walk each relation, accumulating a correlated `EXISTS`
subquery that ends by comparing the reached object's `principal_column` to the
bound caller DID.

Single-hop example — path `[creator, caller]` on `document`
(`creator: { target: user, join_column: creator_uuid }`,
`user: { table: users, principal_column: did }`):

```sql
EXISTS (
  SELECT 1 FROM users AS a1
  WHERE a1.id = <col(document, creator_uuid)>        -- correlation to base row
    AND <col(user, did)> = ?                          -- bind session.subject_did
)
```

`<col(objtype, name)>`: if `name ∈ {id, creator_id, created_at, updated_at}`
→ `"{table_alias}.{name}"` (reserved physical column); else
→ `"json_extract({table_alias}.payload, '$.{name}')"` (§12.2). The base row's
columns are correlated by the outer table name (`documents.…`), inner rows by
alias `a1, a2, …`.

Multi-hop with a recursive relation — path `[creator, management_chain, caller]`
(`management_chain: { target: user, from_key: id, to_key: manager_id,
recursive: true }`):

```sql
EXISTS (
  WITH RECURSIVE mc(id, prin, depth, seen) AS (
      -- seed: the creator user
      SELECT u.id, <col(user, did)>, 0, '/' || u.id || '/'
        FROM users u
        WHERE u.id = <col(document, creator_uuid)>
    UNION ALL
      -- step: follow to_key (manager_id) upward
      SELECT u2.id, <col(user, did)>, mc.depth + 1, mc.seen || u2.id || '/'
        FROM users u2 JOIN mc
          ON u2.id = (SELECT <col(user, to_key=manager_id)> FROM users WHERE id = mc.id)
        WHERE mc.depth < ?                              -- bind MAX_RECURSION_DEPTH
          AND instr(mc.seen, '/' || u2.id || '/') = 0   -- visited_track cycle guard
  )
  SELECT 1 FROM mc WHERE mc.prin = ?                    -- bind session.subject_did
)
```

Cycle guard = ADR-0017 §8 path-concatenation `visited_track`: `seen` is the `/`-
delimited id path; a node already in it is not re-expanded. `depth < ?` is the
second backstop. `MAX_RECURSION_DEPTH` is a compile constant (propose 64) bound
as a param, not interpolated.

> **Implementation note (Phase 1 landed, PR #86):** this worked example
> groups `management_chain` under `document.relations`, but its `from_key`/
> `to_key` are self-join columns on `user`, not `document` -- the shipped
> policy declares it under `user.relations` instead (relation lookup
> transitions to the current type after each hop; `creator` reaches `user`,
> then `management_chain` is looked up *there*). The compiled SQL is
> unchanged. Also: `<col(...)>` here is illustrative string interpolation --
> the shipped `col()` binds a non-reserved field name's JSON path as a `?`
> param instead (`json_extract(qualifier.payload, ?)`), never splicing it
> into the string literal, after a security review found the interpolated
> form let a malformed policy-authored field name break out of the SQL
> string.

Design notes for the walk:
- Non-recursive relations compose as nested/joined `EXISTS` correlated to the
  previous alias.
- A recursive relation in the middle of a path (not just at the end) wraps the
  remainder in the CTE's result set — keep B2's first cut to **at most one
  recursive relation, and only as the last relation before the terminal**
  (covers the ADR's example); reject other recursive placements at compile with
  a clear `Semantic` error (widen later if a real policy needs it — §12.7).
- If any relation on the path is remote (`service:`): return
  `Err(Semantic("remote relation <x> requires B3"))` → caller treats as deny
  (fail-closed, ADR-0017 §6).

### 3.5 Watchdog / default-deny on timeout

Not compiled into SQL — installed on the connection around the query, exactly
like `do_aggregate` already does (`sqlite.rs:393`,
`conn.progress_handler(N, Some(|| true))` + a guard that clears it). Add a
`policy`-configurable op-budget (default reuse `QUERY_RAW_MAX_VM_OPS` or a new
`FDAE_MAX_VM_OPS`). When the handler aborts, rusqlite returns `SQLITE_INTERRUPT`
(`OperationInterrupted`) → **default-deny**: Mode B/aggregate/delete_many →
`Err(QuotaExceeded)` (matching the shipped `sqlite.rs:472-474` mapping), Mode A →
`Ok(false)` — **not** a silent empty success (§12.8, resolved). The time budget
is "configurable, not hard-coded" (ADR-0017 §8) — thread from policy or substrate
config, default conservative.

---

## 4. Reused/related existing code

- `crates/data_db/src/filter.rs` — `compile_filter(Option<&str>) ->
  Option<CompiledFilter>` and `CompiledFilter { where_clause, params }`. The
  sieve ANDs with this. **Reuse** it (unchanged) for caveat `where` filters.
- `crates/data_db/src/sqlite.rs:319` `do_query` — the merge site (§5).
- `crates/data_db/src/sqlite.rs:393` `do_aggregate` — the progress-handler
  watchdog pattern to copy for §3.5.

---

## 5. `data_db` integration — merge + threading

### 5.1 New auth-context type — `crates/data_db/src/lib.rs` (or a new `auth.rs`)

```rust
pub struct QueryAuth<'a> {
    pub policy: &'a syneroym_fdae::Policy,
    pub session: &'a syneroym_ucan::SessionContext,
    pub service_id: &'a str,     // for ResourceUri::service(service_id, service_id)
}
```

### 5.2 `ServiceStore` trait signature changes — `crates/data_db/src/traits.rs`

Add an optional auth arg to the read paths (and delete-many, which filters).
`Option<&QueryAuth>` = `None` preserves today's unfiltered behavior (policy-
absent services, and every non-guest caller that has no session).

```rust
async fn get(&self, collection: &str, id: &str, auth: Option<&QueryAuth<'_>>)
    -> Result<Option<host_store::RecordReadValue>, host_store::DataLayerError>;

async fn query(&self, collection: &str, opts: &host_store::QueryOptions,
               auth: Option<&QueryAuth<'_>>)
    -> Result<host_store::QueryResult, host_store::DataLayerError>;

async fn delete_many(&self, collection: &str, filter: Option<&str>,
                     auth: Option<&QueryAuth<'_>>)
    -> Result<u64, host_store::DataLayerError>;

/// Mode A point-in-time check (ADR-0017 §4). New method.
async fn check_access(&self, collection: &str, id: &str, operation: &str,
                      auth: Option<&QueryAuth<'_>>)
    -> Result<bool, host_store::DataLayerError>;
```

`put`/`patch`/`batch_mutate`/`query_raw`/`create_collection`/`drop_collection`/
`execute_ddl` are **unchanged** in B2 (write-path RLS via Mode A `check_access`
at the guest is possible but is a follow-up; B2's failure tests are
read/Mode-A/CLS only). `query_raw`/`execute_ddl` stay `data-layer/admin`-gated
(Tier 2) — an admin caller is not row-filtered, by design.

**`aggregate` MUST be sieve-aware (security fix — review finding).** `aggregate`
is an **ungated** read path — no capability gate, "open to any caller, like
`query`" (`sqlite.rs:378-385`, host at `host_capabilities.rs:446`). If `query`
is filtered but `aggregate` is not, any caller bypasses RLS by aggregating.
Signature gains `auth: Option<&QueryAuth<'_>>`; enforcement:
- **RLS:** inject the compiled `where_clause` into `aggregate`'s inner base query
  (`aggregate.rs:76` already appends a `WHERE` built from `$match`) as an
  additional `AND`. Reuse the same `compile_read` sieve.
- **CLS:** column masking over a `$group`/`$project` projection is ill-defined
  (grouped/derived columns, not the fixed record `payload`). So when the
  collection's policy carries active CLS (`masked_fields` non-empty for this
  caller), **fail-closed: deny the aggregate** (`DataLayerError::PermissionDenied`)
  rather than risk leaking a masked field through an aggregate expression.
  (Refining CLS-aware aggregation is a later slice; deny is the safe B2 default.)

### 5.3 `SqliteServiceStore` impl — `crates/data_db/src/sqlite.rs`

`query` (async, ~line 1330): compile the sieve **before** the interact closure.

```rust
async fn query(&self, collection, opts, auth) -> … {
    let sieve = match auth {
        Some(a) => syneroym_fdae::compile_read(a.policy, collection, a.session,
                                               fdae::Mode::Filter)
            .map_err(|e| DataLayerError::Internal(e.to_string()))?,   // fail-closed
        None => None,
    };
    // caveat `where`: compile each applicable capability's caveats.where via
    // filter::compile_filter and collect (see §6). For B2's first cut this can
    // be threaded through `sieve` or computed here from `auth.session`.
    let conn = self.reader_pool.get().await?;
    conn.interact(move |conn| do_query(conn, &collection, &opts, sieve.as_ref()))
        .await …
}
```

`do_query` (line 319) gains `sieve: Option<&CompiledSieve>` and, after building
`where_clauses` from `CompiledFilter` + cursor:

```rust
if let Some(s) = sieve {
    where_clauses.push(format!("({})", s.where_clause));
    bound_params.splice(0..0, s.params.iter().cloned()); // preserve binding order
}
```

**Binding-order caution:** `?` params bind positionally across the whole SQL. The
final param order must match the concatenated clause order (sieve clause first,
then filter, then cursor, then the LIMIT param). Build `where_clauses` and
`bound_params` in lockstep — push the sieve clause+params first. Re-audit
`do_query`'s existing order (`sqlite.rs:328-345`): filter → cursor → limit;
insert sieve at the front of both.

CLS masking (`sieve.masked_fields`) is applied to each returned row's `payload`
after fetch — parse JSON, remove named paths, re-serialize (§6).

`do_get` (line 284) — **also** sieve-aware (§5.2 adds `auth` to `get`). Compile
with `Mode::PointInTime { id }` and wrap:
`SELECT … WHERE id = ?1 AND EXISTS(<sieve where_clause>)`; a row that fails the
sieve returns `Ok(None)` (unauthorized get is a miss, not an error — ADR-0007).
Apply CLS masking to the returned record's payload as in `do_query`.

`check_access` → new `do_check_access(conn, collection, id, operation, sieve)`:
compile with `Mode::PointInTime { id }` (operation → the platform ability for
branch selection), run `SELECT EXISTS(SELECT 1 FROM {t} WHERE id=? AND {sieve})`,
return the bool. Deny (false) on any compile/exec error.

`delete_many` — **NOT `do_query`'s path.** It runs through
`send_write_command` → `DbCommand::DeleteMany` (enum variant `sqlite.rs:798`,
dispatch `sqlite.rs:1376`) → handled by the **writer task** at `sqlite.rs:894`
(`do_delete_many`, `sqlite.rs:238`), not `conn.interact`. Thread the sieve
through as owned data: compile it in the async `delete_many` method, add a
`sieve: Option<CompiledSieve>` field to the `DbCommand::DeleteMany` variant, and
prepend it (clause + params) to `do_delete_many`'s `WHERE` in the writer arm —
same binding-order discipline as `do_query`.

### 5.4 Every call site of the changed methods

The `ServiceStore` trait is implemented **twice** and both must change or the
crate won't compile:

- **`impl ServiceStore for SqliteServiceStore`** (`sqlite.rs:1234`) — the real
  impl (§5.3).
- **`impl ServiceStore for Arc<SqliteServiceStore>`** (`sqlite.rs:1421`) — a
  **forwarding** impl (`self.as_ref().get(...)` etc.). Add the new `auth`
  params + `check_access` here too, forwarding them through.

Callers of `{get,query,delete_many,aggregate}` (`check_access` is new — no
existing callers):

- `crates/sandbox_wasm/src/host_capabilities.rs` — `store::Host::{get,query,
  delete_many,aggregate}` (lines 418/432/470/446). **Primary seam** — pass
  `Some(&QueryAuth)` built from `self.caller.session` + `self.fdae_policy` (§7).
- `crates/control_plane/src/synsvc_native.rs` — native SynSvc dispatch:
  `store.get` (`:284`), `store.query` (`:295`), `store.delete-many` (`:311`
  arm), `store.aggregate` (`:421` arm). Native-service calls; pass `None` for B2
  (native services are Tier-1/2 gated, not guest-Tier-3) unless a native service
  should also be sieve-filtered — see §12.9. Default `None`, preserving behavior.
- Benches (`data_db/benches`, `sandbox_wasm/benches`) and `tests_crud.rs` — pass
  `None` (add the trailing arg). *(Note: `service.rs:780` is **not** a call site
  — it is `open_service_db` + `write_secret`, a vault test; earlier draft cited
  it in error.)*

Mechanical: every existing `.query(&c, &opts)` → `.query(&c, &opts, None)`, etc.,
across both trait impls, all callers, benches, and tests.

---

## 6. RLS vs CLS, caveat merge (ADR-0017 §4, ADR-0015 A3)

- **RLS** = the `where_clause` row-pruning subquery above.
- **CLS** = payload field masking. Compute `masked_fields` = fields denied by
  the intersection of (policy `Permission.fields`) and (each applicable
  capability's `caveats.fields`, per ADR-0015 A3: allow-lists intersect,
  deny-lists union). Apply as a **host-side final projection** (§6.1 — so the
  B4-fdae stage-4 hook can slot in above it without relocation): for each
  returned record, `serde_json::from_slice(payload)`, remove each masked JSON
  path (dot-notation → nested removal), re-serialize. (`json_remove` in the
  SELECT is the in-DB alternative but would put CLS below stage-4 — §12.5.)
- **Caveat `where` merge** = each applicable capability's `caveats.where`
  (a MongoDB filter doc) → `filter::compile_filter` → ANDed with RLS +
  `CompiledFilter`. All three are intersective (ADR-0017 §4). Keep this compile
  in `data_db` (it already owns `filter.rs`); `fdae` only needs to hand back
  *which* capabilities are applicable, or `data_db` re-derives from
  `auth.session`. **Decision:** have `compile_read` return the applicable
  capabilities' relevant caveats alongside the sieve to avoid duplicating branch
  selection — extend `CompiledSieve` with `where_caveats: Vec<serde_json::Value>`
  (the raw `where` docs), which `do_query` compiles via `filter::compile_filter`.

### 6.1 CLS ⇄ stage-4 ordering — the B4-fdae hand-off

The pipeline is `stage 3 (SQL sieve, this slice)` → `stage 4 (WASM ABAC,
B4-fdae)`, stage 4 operating on the sieve's "candidate rows" (task.md pipeline;
ADR-0017 §7). CLS masking removes columns; stage-4 `authorize-rows` may need
those same columns to reach its `allow`/`deny`/`redact` decision. **Ordering
contract to bake in now so B4 slots in cleanly:**

> RLS (SQL, in-DB) → candidate rows → **stage-4 ABAC** (host calls the guest
> export, sees RLS-filtered but **pre-CLS** rows) → **CLS masking (always the
> final projection)** → WIT response.

Where B4's hook lands: `host_capabilities.rs`, after `store.query` returns and
before the `query-result` is handed back to the guest — the host invokes the
guest's `authorize-rows` export there, then applies CLS. **Implication for B2:**
because CLS must remain the *last* step and B4 inserts *above* it, CLS is
cleanest as a **host-side final projection**, not buried inside `do_query`. B2
should therefore return `masked_fields` out of the store (extend `QueryResult`
with a host-only `masked_fields`, or carry it on the `CompiledSieve` the host
already holds) and apply masking in `host_capabilities.rs` — so B4 needs no
relocation, only an insertion. *(If B2 lands CLS inside `do_query` for
expedience instead, this section is the recorded debt B4 must repay by lifting
CLS to the post-stage-4 host step. Prefer host-side from the start.)* This is
the B4-fdae counterpart to §12.4's explicit `anchor`→B3 hand-off — recorded, not
silently deferred.

---

## 7. Host / WIT plumbing — `crates/sandbox_wasm`, `crates/wit_interfaces`

### 7.1 `HostState` gains the policy — `host_capabilities.rs`

```rust
pub struct HostState {
    …
    pub fdae_policy: Option<Arc<syneroym_fdae::Policy>>,   // None = policy-absent
}
```
`HostState::new` gains a trailing `fdae_policy: Option<Arc<Policy>>` param
(line 120-160). Build `QueryAuth` in each read method:

```rust
async fn query(&mut self, collection, opts) -> … {
    let store = open_store(self.component_id.clone(), self.key_store.clone(),
                           self.storage_provider.clone()).await?;
    let auth = self.fdae_policy.as_ref().map(|p| QueryAuth {
        policy: p, session: &self.caller.session, service_id: &self.component_id,
    });
    store.query(&collection, &opts, auth.as_ref()).await
}
```
(same for `get`, `delete_many`, `aggregate`). Note the compiler matches
capabilities against the **collection-qualified** resource
`synapp:<id>:svc:<id>/collection/<collection>` (§3.2), *not* the bare base that
`execute_ddl`/`query_raw` gate against (`host_capabilities.rs:505` and `:527`
respectively — two separate call sites); the qualified resource is what makes a
`/collection/<name>` selector grant work.

### 7.2 New Mode-A WIT function — `wit/data-layer/data-layer.wit`

Additive (minor bump, non-breaking, §Migration WIT Boundary). The guest asks
"may I <operation> record <id>?":

```wit
/// Mode A point-in-time authorization check (ADR-0017 §4). Returns whether the
/// caller's compiled FDAE policy authorizes `operation` on record `id` in
/// `collection`. Fail-closed: any evaluation error returns false, never an error
/// that could be read as allow. `operation` is a platform ability string
/// (e.g. "data-layer/read", "data-layer/write") or an app permission ref.
check-access: func(collection: string, id: string, operation: string)
    -> result<bool, data-layer-error>;
```
Regenerate host/guest bindings (`syneroym-wit-interfaces` `bindgen!`). No manual
mirror step: `wit/host/deps/data-layer/data-layer.wit` is a **symlink** to this
one file (verified), so editing the single source suffices. Implement
`store::Host::check_access` in `host_capabilities.rs` (build `QueryAuth`, call
`store.check_access`). **Confirm the `operation` shape** — §12.3.

### 7.3 `engine.rs` — load policy at instantiation

Next to the `config_generation` fetch (`engine.rs:634-646`), fetch+parse the
persisted policy for `service_id` and pass `Some(Arc<Policy>)` (or `None`) into
`HostState::new` (both call sites, lines 662 and 1199). See §9 for the source.

---

## 8. `Ability` / operation vocabulary

No new abilities needed — `Ability::DATA_LAYER_READ/WRITE/ADMIN` already exist
(`capability.rs:116-118`). `Permission.allows` strings are matched against held
capabilities via the existing `Capability::grants` + `Ability::entails`. The
app-permission route uses `app/<objtype>.<perm>` ability strings, which are
already the ADR-0015 A2 reference form (no `Ability` table entry — they're
resolved by the policy, per A2 "an app permission never entails a platform
ability").

---

## 9. Manifest + deploy + persistence plumbing (Phase 4)

### 9.1 Manifest — `policy_path` must cross **two** `ServiceConfig` types

There are **two** `ServiceConfig`s and both need the field, or `policy_path`
never reaches the deploy code (this is what makes E2E step 22 pass — getting
only the first is unreachable code):

1. **Rust manifest** — `crates/app_orchestration/src/models.rs:210` (there is no
   `ServiceManifest` struct — §12.6):
   ```rust
   #[serde(default, skip_serializing_if = "Option::is_none")]
   pub fdae: Option<FdaeManifest>,
   #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
   pub struct FdaeManifest { pub policy_path: String }
   ```
   `#[serde(default)]` ⇒ existing manifests parse unchanged (Migration Strategy).
2. **WIT `service-config`** — `crates/wit_interfaces/wit/control-plane/control-plane.wit:23`
   (the record `orchestration.rs`'s deploy path actually receives). Add
   `fdae-policy-path: option<string>,` next to `schema-path:` (line 31).
3. **The mapper** — `crates/sdk/src/mapper.rs:12` `map_deployment_plan_to_wit`
   field-copies the Rust `ServiceConfig` into `WitServiceConfig` (it copies
   `schema_path` at line 23). Add the analogous copy:
   `fdae_policy_path: svc.config.fdae.as_ref().map(|f| f.policy_path.clone())`.
   Without this line the Rust `fdae` field is silently dropped at the WIT
   boundary and `orchestration.rs` never sees it.

### 9.2 Deploy — `crates/control_plane/src/service/orchestration.rs`

Alongside the `schema_path` read+validate (lines 287-323), the deploy path reads
the **WIT** `manifest.config.fdae_policy_path` (per §9.1, not the Rust field):
if set, path-traversal-guard it (same checks as `schema_path`), read the file,
`fdae::parse_and_validate`, and persist the validated JSON. Emit the `strict:`
author-time warning here (§10) when the manifest declares collections a policy
lacks definitions for.

### 9.3 Persist + load — `StorageProvider` (mirror config generations)

Add to the `StorageProvider` trait + `SqliteStorageProvider`
(`sqlite.rs:1036-1135` is the config-generation analog):

```rust
async fn save_fdae_policy(&self, service_id: &str, policy_json: &str) -> Result<()>;
async fn load_fdae_policy(&self, service_id: &str) -> Result<Option<String>>;
```
Storage: a new `fdae_policies(service_id TEXT PRIMARY KEY, policy_json TEXT,
updated_at INTEGER)` table in `substrate.db` (created in the schema-init block
near `config_generations`, `sqlite.rs:~655-685`; the `save`/`load` fns mirror
`save_config_generation`/`get_latest_config_generation` at `sqlite.rs:1034` and
`1112`). `engine.rs` calls `load_fdae_policy` at instantiation,
`fdae::parse_and_validate`, wraps in `Arc`.

> **Phasing note:** Phases 1-3 (crate + compiler + store/WIT/host wiring) are
> testable with a policy injected directly (integration tests build a `Policy`
> and a `QueryAuth` by hand). Phase 4 (manifest→deploy→persist→instantiate) is
> what makes the E2E reference step 22 pass. Land 1-3 first.

---

## 10. `strict:`, default-deny, decision trace

- **`strict: true`** (D-04-02-c): off by default, additive. Implemented inside
  this slice, not ahead of it. Effect: in `compile_read`, a `collection` with no
  matching definition returns `deny_all()` instead of `Ok(None)`. Author-time
  warning (a known collection lacking a definition) is emitted by the deploy path
  (§9.2), not the compiler.
- **Policy-internal default-deny** (D-04-02-b, ADR-0017 §8): empty applicable-
  permission set and no reachable `default` ⇒ `deny_all()`.
- **Decision trace** (ADR-0017 §9): "ships with the first slice." Produce a
  structured record on every deny (and optionally allow) with `tier: 3`, `held`
  (which grant was evaluated), `operation_admitted` (did any `allows` match),
  `rows_reached` (did any path succeed — for Mode A) / for Mode B the
  distinction is per-row so record the compiled predicate + applicable perms,
  `path_failed`, `caveats_applied`. B2 scope: emit it via `tracing`
  (structured fields) at debug/info from `compile_read`/`check_access`; a
  queryable trace API is later. Define a `fdae::DecisionTrace` struct and log it.

---

## 11. Tests

- **`crates/fdae` unit** (`compile.rs` tests): single-hop `WHERE EXISTS`;
  multi-hop; recursive `WITH RECURSIVE` + cycle guard (feed a cyclic manager
  graph, assert termination + correct membership); `union`/`intersection`/
  `exclusion` operator SQL; RLS deny-all (no applicable perm); CLS field
  intersection; strict-mode unknown-collection deny; remote-relation compile
  error; SQL-injection attempt in a policy value is bound not interpolated
  (mirror `filter.rs:369`); branch selection (platform-ability vs app-permission
  route; `includes` closure); **collection-selector grant is honored** (a caller
  holding only `.../collection/orders` is admitted for `orders`, denied for
  another collection — the §3.2 finding, guards against the `covers_resource`
  bare-base regression); **`conditions`/claims binding** (a `claim`-referencing
  condition binds `session.claims[k]` as `?`; absent claim ⇒ deny). Run generated
  SQL against an in-memory `rusqlite::Connection` with seeded rows to assert
  *actual* row visibility, not just string shape.
- **Decision-trace tests** (ADR-0017 §9 / access-control-design §7.2 — the trace
  is *non-optional* and "ships with the first slice"): assert the `DecisionTrace`
  emitted on a deny has the right fields — `tier: 3`, the `held` grant that was
  evaluated, `operation_admitted` vs `rows_reached` split, `path_failed`. One
  test per deny reason (operation not admitted; rows not reached; strict unknown;
  claim absent). Without these the trace can rot silently despite the ADRs
  treating it as load-bearing.
- **`data_db` integration** (`tests_crud.rs` or new `tests_fdae.rs`): Mode B
  query excludes unreachable rows (empty, not error — ADR-0007); Mode A
  `check_access` deny; `get` of an unreachable row returns `None`; **`aggregate`
  does not bypass RLS** (an aggregate over a policy-protected collection is
  row-filtered identically to `query`, and is denied when CLS is active — §5.2
  security finding); `delete_many` is row-filtered through the writer path; CLS
  column masked out of payload; sieve ⊕ CompiledFilter ⊕ cursor pagination
  binding order correct; watchdog timeout → deny (a policy with a pathological
  recursion vs. a tiny op-budget).
- **`sandbox_wasm` integration**: a guest `query` transparently filtered
  (reference step 22); policy-absent service unchanged; `check_access` guest
  import round-trips.
- **Failure/security matrix** (task.md §"Failure and Security Tests"): every row
  1-7 that is B2 (rows referencing B3/stage-4 are later). Document outcomes.
- **`criterion` bench**: FDAE pushdown query, 100 records single-hop ReBAC,
  assert < 25 ms p99 (perf table). Add to `crates/fdae/benches` or
  `data_db/benches`.
- **fmt/clippy/test/e2e**: exit-criteria gate; `wasm32-wasip2` unbroken (WIT
  change is additive; rebuild `test-components`).

---

## 12. Ambiguities / ADR gaps / stale items — RESOLVE BEFORE/DURING CODING

1. **Terminal principal ↔ DID column (biggest gap).** ADR-0017's YAML shows
   relations + paths ending in `caller`, but never says how the reached object
   row's identity is compared to the caller's DID. ADR-0015 A6 mandates "join on
   DID, not email." Proposed: each object type usable as a path terminal declares
   `principal_column`, and the terminal binds `session.subject_did` against
   `col(target, principal_column)`. **Needs an explicit nod / ADR footnote.**
2. **`join_column` / key fields: reserved column vs JSON payload path.**
   Collections are `(id, creator_id, created_at, updated_at, payload JSON)`.
   ADR's `join_column: creator_uuid` is not a physical column. Proposed
   convention (§3.4 `<col>`): reserved names → physical column; else
   `json_extract(payload,'$.<name>')`. Confirm.
3. **Mode-A `check-access` signature.** ADR only says "a new explicit
   check-style host function." §7.2 proposes
   `check-access(collection, id, operation) -> result<bool, _>`. Confirm the
   `operation` argument (a platform ability string) and whether Mode A also
   needs to cover non-`data-layer` operations (delete/publish/invoke — the ADR
   §4 examples). B2 could ship `data-layer/{read,write}` only.
4. **`anchor` terminal — deferred to B3 (resolved).** ADR-0015 A5's
   `anchor_did`/`path` is accepted in the ADR but implemented nowhere: B7 shipped
   using the `DelegationCertificate`'s `master_did` and explicitly deferred
   `anchor_did` to "real UCAN chains" (B7.md:1119-1124). It is **not M7 scope**
   (M7 is D-04-02-d, stale-relationship data). B2 is a *local* slice with no
   multi-hop chain, so in every B2-reachable request `anchor_did == subject_did`
   — the field would be provably-equal-to-`caller` dead weight and the `anchor`
   terminal would only be testable against synthetic hand-built contexts. **B3
   (cross-service chains) is A5's first real, e2e-testable consumer** — that is
   where `caller ≠ anchor` becomes real and where "enforce at the data-owning
   node against the original principal" (the confused-deputy defense A5 exists
   for) actually bites. **Decision: B2 supports `caller` only; `anchor` (the
   `SessionContext.anchor_did` field + population in `from_verified_chain`, and
   the compiler's `anchor` terminal) moves to B3.** A5's full `path` *list*
   binding stays deferred beyond B3 (no near-term consumer). Recorded against B3
   in `task.md`'s Slice B3 section and at `access-control-design.md:996`.
5. **CLS mechanics: post-fetch JSON masking vs SQL `json_remove` projection.**
   §6 proposes post-fetch masking for B2 simplicity. Confirm acceptable (it
   loads then strips — fields never *reach the guest*, satisfying the security
   test, but they *are* read from disk; a defense-in-depth reviewer may prefer
   `json_remove` in the SELECT).
6. **Migration doc says `[services.my-svc.fdae]` / `ServiceManifest`** — there is
   **no `ServiceManifest` struct**; the real type is `ServiceConfig`
   (`app_orchestration/src/models.rs:210`), flattened into `ServiceSpec`. The
   task's TOML snippet is directionally right but the struct name in "Migration
   Strategy → ServiceManifest Extension" is stale. §9.1 targets `ServiceConfig`.
7. **Recursive relation placement.** §3.4 restricts B2 to a single recursive
   relation as the last hop before the terminal (covers the ADR example). A
   policy with recursion elsewhere → compile error. Confirm this is acceptable
   for B2 (widen later).
8. **Timeout → deny surfacing. ✅ Resolved: `QuotaExceeded`.** A *timeout* must
   be distinguishable from "legitimately zero rows," not a silent empty success
   (ADR-0017 §8 default-deny; ADR-0007's "no result is valid" governs row
   filtering, not a compute-budget abort). Reuse the shipped watchdog mapping:
   the existing progress-handler backstop already maps `OperationInterrupted →
   QuotaExceeded` (`sqlite.rs:472-474`), so **Mode B (query/aggregate/
   delete_many) timeout → `Err(QuotaExceeded)`**, identical to `aggregate`/
   `query_raw` today; **Mode A (`check_access`) timeout → `Ok(false)`**
   (fail-closed deny — no error to surface on a point check). No new WIT variant;
   the real reason lives in the decision trace (§10). (A dedicated
   `policy-timeout` variant was considered and rejected — widens the WIT surface
   for something the trace already explains, unwarranted pre-release.)
9. **Native-service (Tier-1/2) callers pass `None`.** §5.4 — native SynSvc
   dispatch (`synsvc_native.rs`) is not guest-Tier-3; B2 passes `None` (no
   sieve). Confirm no native service is expected to be row-filtered in B2.
   (`control_plane/src/service.rs:162` `TODO(M04B/FDAE)` for security-op authz is
   a *different* concern — grant-layer/B7 per ADR-0017 Open §, not B2.)
10. **`operator` default + `public`.** §2 defaults `operator: union`; a
    permission with empty `paths` is `public` (`1=1`). Matches ADR-0017 §1/§3.
11. **`verified_at_secs` / staleness** — explicitly M7 (D-04-02-d), not B2.
12. **Stage-4 `authorize-rows`** — B4-fdae, not B2. Keep the WIT export unadded
    in B2. CLS/stage-4 ordering hand-off recorded in §6.1.
13. **`router/src/proxy.rs:192` seam is B3, not B2 (record, don't silently skip).**
    task.md lists it under "In-code seams." Its `check_native_capability_gate`
    (`proxy.rs:207`) refuses *guest-originated cross-service* native-capability
    calls, and its own doc comment already scopes the replacement to **Slice B3's
    relationship-proof fetch** ("a guest-originated cross-service `data-layer`
    read becomes expressible… once M04B lands"). Cross-service is out of B2's
    *local* scope, so B2 does **not** touch this gate — but do not widen it
    either. B3 loosens it as part of wiring stage-2. (This is the proxy analog of
    the §12.9 native-dispatch punts; earlier drafts named `dispatch.rs`/
    `service.rs` but omitted `proxy.rs`.)

---

## 13. Suggested execution order (phases)

1. **Phase 1 — `syneroym-fdae` crate — landed, PR #86 (branch
   `feat/m04b-slice-b2-fdae-sieve`).** model (`policy.rs`) + schema JSON +
   `parse_and_validate` + compiler (`compile.rs`) + full unit tests (SQL run
   against in-memory sqlite). No other crate touched. Resolve §12.1/12.2 first.
   See the implementation notes under §3.1 and §3.4 for where the shipped
   code deviates from this section's pseudocode.
2. **Phase 2 — `data_db` integration**: `QueryAuth`; trait sig changes on
   **both** impls (`SqliteServiceStore` + `Arc<…>` forwarder); `do_query` merge +
   binding order; `do_get`; `do_check_access`; `delete_many` via the
   `DbCommand::DeleteMany` writer path; **`aggregate` sieve/deny** (RLS bypass
   fix); CLS masking; watchdog; integration tests. Update all `None` call sites
   (host, `synsvc_native.rs`, benches, tests).
3. **Phase 3 — WIT + host**: `check-access` WIT + bindings; `HostState.fdae_policy`
   + read-method `QueryAuth` wiring (incl. `aggregate`); host-side CLS projection
   (§6.1); `sandbox_wasm` integration tests.
4. **Phase 4 — deploy/persist**: the field on **both** `ServiceConfig` types +
   `mapper.rs` copy (§9.1); deploy read+validate; `save/load_fdae_policy`;
   `engine.rs` load-at-instantiation; E2E step 22.
5. **Phase 5 — strict mode, decision trace, bench, failure/security matrix,
   fmt/clippy/test/e2e green, traceability-matrix row + status.md evidence.**
