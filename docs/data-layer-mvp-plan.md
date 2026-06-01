# Syneroym: Data Layer & Provider App MVP Plan

This document outlines a comprehensive, phased meta-plan to tackle the significant scope of building the native CRUD Data Layer, the Casbin ABAC engine, the Service Provider App MVP, and the associated infrastructure primitives defined in `data-layer-design.md`.

The strategy prioritizes vertical slicing: proving the end-to-end flow from a WASM component down to the database before building out complex horizontal features like replication or distributed search.

## Phase 1: The Contract & Wasmtime Scaffold

**Goal:** Establish the secure boundary between the stateless WASM components and the stateful Substrate host.
**Why here:** Everything depends on the shape of the WASM/Host API.
1. **Define the WIT Interface (`syneroym:data-layer/store`)**:
   - Write the `.wit` file detailing `create_collection`, `put`, `get`, `query`, `delete`.
   - Ensure no security context (`app_id`, `caller_id`) is passed as an argument.
2. **Substrate Host Scaffold**:
   - Define the Rust `SessionContext` struct mapped to incoming requests.
   - Implement the Wasmtime `Store<HostState>` pattern to securely hold this context.

## Phase 2: Core Data Persistence (SQLite)

**Goal:** Make the data layer persistent and namespaces work.
**Why here:** The ABAC engine (Phase 3) will need to execute raw local queries (`lookup()`) to evaluate rules, so the DB engine must exist first.
1. **SQLite Integration**:
   - Implement the SQLite backend for the CRUD operations. Ensure SQLite connections are initialized with `PRAGMA journal_mode=WAL;` to prepare for Phase 7 (Litestream).
   - Ensure all operations are strictly sandboxed to the `app_id` retrieved from the host context.
2. **Filter & Query Engine**:
   - Implement translation of abstract `FilterExpr` into parameterized SQLite queries.

## Phase 3: Security & ABAC Engine

**Goal:** Secure the data layer using explicit claims and policies.
**Why here:** We must secure the DB before deploying any real apps.
1. **Casbin Integration**:
   - Integrate the Casbin Rust crate into the Substrate.
2. **Enforcement Implementation**:
   - Intercept data layer calls, extract the `SessionContext`, and evaluate against Casbin before executing the SQLite query.
   - Implement the local `lookup("local", ...)` Casbin function (which relies on Phase 2's query engine).

## Phase 4: SynApp Deployment Orchestrator

**Goal:** Automate the setup of a SynApp's database and security boundaries.
**Why here:** Before we can run a WASM component, the host must initialize its SQLite database, create its collections (schema), and load its initial Casbin policies. Doing this manually for testing is a massive refactoring risk later.
1. **Manifest Parser**:
   - Parse the SynApp `App Spec` manifest.
2. **Initialization Routine**:
   - On deployment, automatically execute `create_collection` for declared schemas and inject initial ABAC rules into the Casbin store.

## Phase 5: The First App Slice (`space-manager`)

**Goal:** Prove the core architecture with a real component from the Service Provider App.
**Why here:** We now have the DB, the Security, and the Deployment pipeline ready. It's the perfect time for an End-to-End test.
1. **Component Development**:
   - Create a new Rust project for the `space-manager` WASM component.
   - Implement a simple logic flow (e.g., "Create a new Space Profile"). *Constraint: Do not depend on external services yet to avoid needing Phase 6.*
2. **End-to-End Validation**:
   - Deploy via Phase 4 orchestrator. Test an authenticated request end-to-end.

## Phase 6: Inter-Service Communication & Aliasing

**Goal:** Enable secure references to other services and cross-service attribute lookups.
**Why here:** Now that the local app slice works, we can introduce the complexity of federation and external dependencies.
1. **Service Aliases**:
   - Implement the registry lookup mechanism to resolve aliases (e.g., `org-service`) to DIDs.
2. **Remote ABAC Lookups**:
   - Implement the remote `lookup("service:did...", ...)` Casbin function via Iroh QUIC.

## Phase 7: High Availability & Replication

**Goal:** Ensure data durability and prepare for primary/replica failover.
**Why here:** Replication is an infrastructure concern that sits underneath the application logic. Building it last ensures it doesn't slow down the feature development cycle.
1. **Litestream Integration**:
   - Configure Litestream to stream WAL frames to the Replica node.
2. **Liveness & Failover**:
   - Implement Iroh QUIC heartbeats and manual failover sequence.

## Phase 8: Content-Addressed Blob Storage

**Goal:** Support media and large file storage.
**Why here:** Lazy-pulling logic explicitly depends on the Primary/Replica architecture established in Phase 7.
1. **Local Blob Store**:
   - Implement SHA-256 backed local storage.
2. **Replication Sync**:
   - Implement lazy-pulling over QUIC for missing blobs on replicas.
