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
   - Implement the SQLite backend for the CRUD operations. Ensure SQLite connections are initialized with `PRAGMA journal_mode=WAL;` to prepare for Phase 8 (Litestream).
   - Ensure all operations are strictly sandboxed to the `app_id` retrieved from the host context.
2. **Filter & Query Engine**:
   - Implement translation of abstract `FilterExpr` into parameterized SQLite queries.

## Phase 3: Security & ABAC Engine

**Goal:** Secure the data layer using explicit claims and policies, establishing the foundation for a Consent-First UX.
**Why here:** We must secure the DB before deploying any real apps.
1. **Casbin Integration**:
   - Integrate the Casbin Rust crate into the Substrate.
2. **Enforcement Implementation**:
   - Intercept data layer calls, extract the `SessionContext`, and evaluate against Casbin before executing the SQLite query.
   - Implement the local `lookup("local", ...)` Casbin function (which relies on Phase 2's query engine).
3. **Consent-First UX APIs**:
   - Expose read endpoints to allow the UI to display human-readable permission states and allow dynamic revocation by users, directly supporting the "Consent-First UX" paradigm. *(Implementation Note: Extend policy storage to include human-readable metadata, not just raw Casbin rules).*

## Phase 4: SynApp Deployment Orchestrator

**Goal:** Automate the setup of a SynApp's database and security boundaries.
**Why here:** Before we can run a WASM component, the host must initialize its SQLite database, create its collections (schema), and load its initial Casbin policies. Doing this manually for testing is a massive refactoring risk later.
1. **Manifest Parser**:
   - Parse the SynApp `App Spec` manifest.
2. **Initialization Routine**:
   - On deployment, automatically execute `create_collection` for declared schemas and inject initial ABAC rules into the Casbin store.

## Phase 5: The First App Slice (`space-manager` & Trusted Rooms)

**Goal:** Prove the core architecture by modeling a "Trusted Room" (Context-Aware Thread) using a real WASM component.
**Why here:** We now have the DB, the Security, and the Deployment pipeline ready. It's the perfect time for an End-to-End test.
1. **Component Development**:
   - Create a new Rust project for the `space-manager` WASM component.
   - Implement a logic flow to create and manage a "Trusted Room" (e.g., initializing a space, inviting a provider, managing shared object state). *Constraint: Do not depend on external services yet to avoid needing Phase 7.*
2. **End-to-End Validation**:
   - Deploy via Phase 4 orchestrator. Test an authenticated request end-to-end.

## Phase 6: Action & Agent Gateway (MCP Server)

**Goal:** Expose the Substrate primitives to the "Multi-Surface UX", allowing Agentic Concierges (like Gemini/ChatGPT) to interact natively with Syneroym.
**Why here:** With a functional, secure data layer and the first app slice running, we need to prove the "Headless Substrate" concept before introducing complex inter-service routing.
1. **MCP Server Integration**:
   - Implement a Model Context Protocol (MCP) server that interfaces with the Substrate Host. *(Implementation Note: Define the local authentication model for the MCP Server to securely assume the user's SessionContext).*
2. **Agentic UI Exposure**:
   - Expose the capabilities of the `space-manager` (e.g., creating Trusted Rooms, interacting with objects) as tools that an external LLM can invoke securely on behalf of the user.

## Phase 7: Inter-Service Communication & Aliasing

**Goal:** Enable secure references to other services and cross-service attribute lookups, while ensuring resilience against network partitions.
**Why here:** Now that the local app slice works and is exposed via MCP, we can introduce the complexity of federation and external dependencies.
1. **Service Aliases**:
   - Implement the registry lookup mechanism to resolve aliases (e.g., `org-service`) to DIDs.
2. **Remote ABAC Lookups**:
   - Implement the remote `lookup("service:did...", ...)` Casbin function via Iroh QUIC.
3. **Offline Outbox & Retry Queue**:
   - Implement the local outbox pattern (SQLite + Tokio channel) for inter-service communication. Ensure that outgoing requests or messages initiated while offline are safely queued and automatically retried upon reconnection.

## Phase 8: High Availability, Replication & Backup Substrate

**Goal:** Ensure data durability and prepare for primary/replica failover using the Backup Substrate model.
**Why here:** Replication is an infrastructure concern that sits underneath the application logic. Building it last ensures it doesn't slow down the feature development cycle.
1. **Litestream Integration & Backup Pool**:
   - Configure Litestream to stream WAL frames to the Replica node.
   - Formalize the "Backup Substrate" (mutual torrent-style backup pool) concept where peers host each other's encrypted SQLite data.
2. **Liveness & Active Failover**:
   - Implement Iroh QUIC heartbeats.
   - Implement the manual and automated failover sequences, allowing a backup node to actively serve cached state on behalf of a downed peer.

## Phase 9: Content-Addressed Blob Storage

**Goal:** Support media and large file storage.
**Why here:** Lazy-pulling logic explicitly depends on the Primary/Replica architecture established in Phase 8.
1. **Local Blob Store**:
   - Implement SHA-256 backed local storage.
2. **Replication Sync**:
   - Implement lazy-pulling over QUIC for missing blobs on replicas.
