# Syneroym: Native CRUD Data-Layer Service — Design Notes

---

## 1. Purpose

A **platform-managed persistent store** that synapps and peer substrates use via a well-defined API.
The substrate owns the store; synapps borrow a namespaced view of it.
This is the canonical primitive for structured state — not raw SQLite access.

---

## 2. Resource Model

| Concept | Description |
|---|---|
| **Collection** | Named set of records within a SynApp's namespace. Declared at creation time with a lightweight schema. |
| **Record** | One item in a collection. Identified by a caller-supplied string `id`. Value is a JSON object. |
| **`creator_id`** | First-class field on every record. Set automatically by the data-layer service at write time (cannot be spoofed). Available to policy matchers. |
| **Schema** | Per-collection: field names + types (`string`, `number`, `bool`, `timestamp`, `json`). Indexed fields declared explicitly. Enforced loosely — unknown fields rejected; declared fields type-checked. |

---

## 3. CRUD Operations

```
create_collection(app_id, name, schema)
drop_collection(app_id, name)

put(app_id, collection, id, record)                        // upsert
patch(app_id, collection, id, partial)                     // field merge
get(app_id, collection, id)                                // fetch one
query(app_id, collection, filter, fields?, sort?, page?)   // list
delete(app_id, collection, id)                             // delete one
delete_many(app_id, collection, filter)                    // bulk delete
```

**`fields`** — optional projection (list of field names to return). No ABAC enforcement at field level.

---

## 4. Filter Model

Structured, not raw SQL. Translated to parameterised SQLite internally.

```rust
enum FilterExpr {
    Eq   { field, value },
    Ne   { field, value },
    Gt   { field, value },
    Gte  { field, value },
    Lt   { field, value },
    Lte  { field, value },
    In   { field, values },
    Contains { field, text },   // full-text on indexed fields
    And(Vec<FilterExpr>),
    Or(Vec<FilterExpr>),
}
```

**Pagination:** cursor-based. No offset.

---

## 5. Access Control (Casbin ABAC)

### Subjects
- `substrate:<id>` — peer substrate
- `synapp:<app_id>:<component_id>` — WASM component via host function
- `consumer:<identity_key>` — end user via client gateway token
- `provider:<identity_key>` — SynApp owner

### Objects
- `data:<app_id>:<collection>` — collection-level
- `data:<app_id>:<collection>:<record_id>` — row-level

### Actions
`read`, `write`, `delete`, `query`, `admin` (schema create/drop)

### Row-level ABAC — two composable patterns

**Pattern 1 — Explicit record grant** (precise, sparse)
```
p, consumer:alice, data:orders-app:orders:order-123, read, allow
```
Written by the substrate (or service owner) when a record is created or access is explicitly granted.

**Pattern 2 — Attribute predicate** (scalable, declarative)
```
allow if: lookup("local", record.creator_id, "department_id")
            == lookup("service:acme/org-service", caller.id, "manages_department_id")
```
For `get`: enforcer evaluates predicate against the fetched record's fields.
For `query`: predicate translated into an additional SQL filter; non-matching rows silently excluded.

Any `deny` wins. Both patterns compose freely.

### Policy storage
Casbin policies in a dedicated SQLite table in the substrate's own DB (not the SynApp's data DB).
Enforcer kept warm as `Arc<RwLock<Enforcer>>`.

### Policy management service
**At deploy time:** SynApp manifest ships initial policies. Substrate loads them scoped to `app_id`.

**At runtime:** a substrate-native service exposes:
```
add_policy(app_id, subject, object, action, effect, condition?)
remove_policy(app_id, subject, object, action)
list_policies(app_id)
```
Only identities holding `admin` on `data:<app_id>:*` can edit that app's policies.
Substrate operator (root key) can edit any.
Policy edits written to SQLite immediately; included in Litestream WAL replication to replica.

---

## 6. Entity Attribute Store & `lookup()`

### Entity attributes

Attributes needed for ABAC rule evaluation (department, role, manager, etc.) live with the **service that owns them** — not at the substrate level. Two scopes:

- **Local** — attributes managed by this SynApp's own data-layer collections.
- **Remote service** — attributes managed by another service (e.g. a shared `org-service`), accessed via its API over QUIC.

### `lookup()` custom Casbin matcher function

```
lookup("local", entity_id, field)                     // query this SynApp's own store
lookup("service:acme/org-service", entity_id, field)  // call out to a remote service
```

- `local` lookups are in-process (fast).
- `service:` lookups go over QUIC, authenticated, cached with a short TTL (~30s).
- Cache invalidated when the referenced service signals an update.

This keeps the platform generic — "department", "manager", "territory" are just fields in whatever service defines them. The platform provides the `lookup()` hook; SynApps populate the stores and write the policies.

---

## 7. Service Aliases

### Problem
Service IDs are DIDs (hash-derived). Policies and manifests need stable, human-readable names.

### Model
The **community registry** holds one alias record per owner, keyed by owner DID and signed by their keypair:

```
owner:  did:syn:acme
aliases:
  org-service  → did:syn:serviceXYZ
  billing      → did:syn:serviceABC
```

- Only `acme`'s keypair can write to `acme`'s alias namespace — no conflicts, no central governance needed.
- A provider's deployed services naturally live under their owner DID.
- Cross-service reference: `service:did:syn:acme/org-service` (unambiguous).

### Resolution & caching
- **Pull on startup** — substrate fetches owner alias records for all declared dependencies; caches locally.
- **Error-triggered refresh** — if a service call fails (DID unreachable), re-fetch the owner's alias record from registry and retry.
- **Fallback** — manifest declares alias → DID as authoritative fallback if registry is unreachable.

Resolution order: `local cache → registry → manifest-pinned DID`.

---

## 8. SynApp Access (Host Function)

WASM linker exposes the data-layer as a WIT import:

```wit
import syneroym:data-layer/store;
```

The host function implementation:
- Injects caller identity as `synapp:<app_id>:<component_id>` automatically (cannot be spoofed).
- Enforces ABAC before touching the DB.
- Scopes all operations to the SynApp's namespace (`app_id` is not caller-supplied).
- Resolves `lookup("service:...")` calls on behalf of the WASM component.

---

## 9. Replication & Failover

### Topology
N=2: one **primary** (read-write), one **replica** (read-only). One SQLite DB per SynApp.
Plain SQLite + WAL mode. Litestream streams WAL frames from primary → replica continuously.

### Liveness
- Primary sends heartbeat to replica every ~1s over Iroh QUIC.
- Replica flags service degraded if heartbeat missing for ~5s.
- **No automatic failover.** Manual trigger via `roymctl` / admin UI to avoid split-brain.

### Failover sequence (manual trigger)
1. Replica stops WAL receiver → reopens DB as read-write.
2. Writes a **fencing token** (epoch + its substrate ID) to DHT.
3. Starts Litestream WAL streamer and heartbeats.
4. Old primary, on reconnect, sees higher epoch in DHT → demotes to replica, pulls WAL from new primary.
5. Connection router on new primary rejects calls from fenced substrate ID.

### Starting service on new replica
- Receive SynApp deployment manifest (normal deploy path).
- Replica already holds WAL frames streamed before primary went down — this is the starting state.
- WASM binary deployed normally (stateless; state lives in the store).
- If primary still alive: streams WAL from primary from current position.
- If primary already down: restore from latest WAL on disk → manually promote to primary.

---

## 10. Blob Store (UC1 — simple blobs)

- Stored **content-addressed** (keyed by SHA-256 hash). One blob store per SynApp.
- Blob hash stored as a field in a data-layer record (e.g. `{ "photo": "sha256:abc..." }`).
- Replica pulls missing blobs lazily from primary (or any peer that has them) on first access.
- If primary is down: blobs already on replica are served; missing blobs return 404 until recovered.

---

## 11. Component Configuration & Secrets

### Delivery Mechanism
Configuration defined in the SynApp/Endpoint manifest is delivered seamlessly to both execution environments:
- **Podman**: Substrate injects configuration as standard environment variables (`-e`) and read-only volume mounts (`-v`).
- **WASM**: Substrate injects configuration using the Component Model's `wasi:cli/environment` and `wasi:filesystem` (pre-opened directories). 

### Secrets (MVP)
For the MVP, secrets are treated as standard environment variables defined in the manifest. Advanced secret stores (e.g., Vault integrations) are deferred to later phases.

### Cold Restarts & State
WASM components are fundamentally stateless. The `SessionContext` (containing ABAC permissions and claims) is tied to the incoming request and held securely within the Wasmtime host's `Store`. Therefore, **cold restarts to apply new configuration are perfectly safe** and do not result in any loss of session state or security context.

---

## 12. Inter-Component RPC (The Universal Proxy)

The Syneroym Substrate acts as a dynamic, protocol-translating proxy, shielding WASM developers from heterogeneous networking complexities.

### The Developer Experience
Developers do not use generic, untyped call interfaces (which would defeat the safety and performance of WASM). Instead, they use strongly typed WIT imports (e.g., `import acme:booking/service;`). `wit-bindgen` generates native types for the developer.

### The Substrate Translation (wRPC vs JSON-RPC)
When the WASM component calls an imported function, the Wasmtime engine traps to the Substrate. The Substrate intercepts the in-memory `WASM Val` and dynamically translates it based on the destination:
- **Target is another Substrate (WASM)**: The host serializes the `WASM Val` into **wRPC** (binary, fast) and transmits it over Iroh QUIC.
- **Target is a Podman Container (Non-WASM)**: The host serializes the `WASM Val` into **JSON-RPC** (text, universal) and transmits it over HTTP/WebSocket.

This ensures P2P WASM execution remains incredibly fast while maintaining full interoperability with legacy/heterogeneous container services.

---

## 13. Out of Scope (for now)

- cr-sqlite CRDT (revisit for multi-device sync use case)
- Field-level ABAC
- Auto-failover
- Chunk/shard store for large distributed files — separate design
- N > 2 replication
