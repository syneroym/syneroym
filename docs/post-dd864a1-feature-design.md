# Syneroym: Substrate Feature Implementation Design

This document details the "How"—the concrete engineering designs and implementation strategies—that map to the features defined in the [Feature Specification](post-dd864a1-feature-spec.md).

> **Note:** Only sections with complex architectural considerations are expanded here. Trivial mappings are omitted.

---

## Phase 1: Foundation & Core Infrastructure

### [FND-SEC] Substrate Security
The Substrate relies on multiple cryptographic and hardware-level techniques to guarantee a zero-trust environment.

*   **Data at Rest & Envelope Encryption:** 
    *   **Design:** `sqlite_db` and blob storage are encrypted using Data Encryption Keys (DEKs). A Master Key (KEK) is injected into RAM at startup to unlock the DEKs, ensuring instant key rotation without massive re-encryption.
    *   **The "Unlock" Model:** Encryption keys are scoped to individual services, not the entire substrate. Keys are *never* stored on the substrate's disk. If encryption is required, the service remains locked upon node restart until the Service Owner provisions the key into RAM over the network.
*   **Memory Protection & RAM Dumping Mitigations:** 
    *   **Design:** Perfectly securing a key in RAM from a determined root-level attacker is theoretically impossible without hardware enclaves, but the substrate raises the bar significantly. The Substrate uses OS-level memory locking (`mlock`) to prevent swapping to disk, and `madvise(MADV_DONTDUMP)` to exclude the key from core dumps.
    *   **Key Splitting:** Keys can be obfuscated or split in the `zeroize` memory vault when not actively executing queries, ensuring a naive RAM scraper does not easily find contiguous valid key bytes.

### [FND-CFG] Service Configuration

*   **Dual-Target Configuration Delivery:** Configuration defined in the SynApp/Endpoint manifest is delivered seamlessly to both execution environments:
    *   **Podman**: Substrate injects configuration as standard environment variables (`-e`) and read-only volume mounts (`-v`).
    *   **WASM**: Substrate injects configuration using the Component Model's `wasi:cli/environment` and `wasi:filesystem` (pre-opened directories).
*   **Cold Restarts & State:** WASM components are fundamentally stateless. The `SessionContext` (containing UCAN capabilities and claims) is tied to the incoming request and held securely within the Wasmtime host's `Store`. Therefore, cold restarts to apply new configuration are perfectly safe and do not result in any loss of session state or security context.
*   **Secrets (MVP):** For the MVP, secrets are treated as standard environment variables defined in the manifest. Advanced secret stores (e.g., Vault integrations) are deferred.

### [FND-IAM] Access Control
The Access Control architecture relies on the Federated Data-Aware Authorization Engine (FDAE) to combine cryptographic capabilities with massive-scale relational data filtering.

*   **SynApp Access (Host Function):**
    *   **Design:** WASM applications do not query databases directly. The WASM linker exposes the data-layer as a strictly typed WIT import (`import syneroym:data-layer/store;`).
    *   **Identity Injection:** The host function implementation automatically injects the caller identity as `synapp:<app_id>:<component_id>`. This cannot be spoofed by the guest.
    *   **Execution:** It dynamically compiles the FDAE ReBAC policies directly into the SQL query before execution, and explicitly scopes all operations to the SynApp's namespace.
*   **Comprehensive Schema Specification (No-Parser AST Config):**
    *   **Design:** To eliminate the runtime overhead of string lexers in the Wasm host framework, the Domain Specific Language (DSL) is written as a fully structured configuration tree (YAML/JSON). Deserializing this file directly produces the actionable Abstract Syntax Tree (AST) for the query planner.
    *   **Registry:** Maps logical keys to physical storage engines (e.g., `sqlite_db` vs `hr_api` which triggers a Wasm Host Extension).
    *   **Hierarchies:** Maps unbounded graph pathways (e.g., `management_chain`).
    *   **Definitions:** Defines objects, data joins, and boolean permission paths (e.g., a `union` of direct ownership vs transitive manager chain checks).
*   **Engine Evaluation Steps & The Pushdown Sieve:**
    *   **Lookahead Optimization:** The engine reviews the permission block. If nodes share the same physical SQLite storage driver, it flags them for "Join Tree Collapse".
    *   **SQL Generation (The Pushdown):** The engine merges the relationship hops into a singular, cycle-protected `WITH RECURSIVE` SQLite query.
    *   **Global Logic Short-Circuiting:** If a path evaluates to a true state (e.g., under a `union` operator), the engine instantly returns an Allowed state, bypassing further external checks.
    *   **Cross-Service Parameter Fetch:** If a step crosses a boundary (e.g., requires external HR data), the engine halts local SQL execution, marshals the intermediate value across the Wasm runtime memory boundary, fires the native host extension function to fetch remote proofs, and resumes execution.
*   **Dual-Mode Capability:**
    *   **Mode A (Point-In-Time Evaluation):** When verifying a specific resource handle ("Can Alice view document 12?"), the engine appends an absolute constraint (`WHERE documents.id = ?`).
    *   **Mode B (Relational Data Filtering):** When requesting a dashboard index ("Show me all documents I can see"), the engine wraps the user's base command in the compiled `WHERE EXISTS` security block as a global subquery. This forces SQLite to perform index-level pruning before the data ever reaches the Wasm guest.
*   **Performance Safeguards & Security:**
    *   **Strict Parameter Isolation:** The query compiler strictly uses native parameterized binding (`?` or `:name`) to prevent SQL injection.
    *   **Deterministic Cycle Protections:** Every recursive configuration block includes a path concatenation tracker (`visited_track`) to break execution if cyclic loops are introduced in the user data.
    *   **Instruction Watchdogs:** Generated queries execute alongside an active instruction cycle watchdog (`sqlite3_progress_handler`). If execution takes longer than 15ms, the transaction is immediately rolled back with a "Default Denied" state.

---

## Phase 2: Core Platform Capabilities

### [PLT-DAT] Data Layer

The Data Layer provides a complete foundation for distributed application state and communication, securely accessed via typed host functions.

#### 1. REST Data Service (Structured Database)
A platform-managed persistent store that SynApps use via the API. The substrate owns the store; SynApps borrow a namespaced view of it.

*   **Resource Model:** 
    *   **Collection:** A named set of records within a SynApp's namespace, declared with a lightweight schema.
    *   **Record:** One JSON object identified by a caller-supplied string `id`.
    *   **`creator_id`:** A first-class field on every record, set automatically by the service at write time (spoof-proof).
    *   **Schema:** Declares indexed fields explicitly. Enforced loosely (unknown fields rejected; declared fields type-checked).
*   **CRUD Operations:** Operations include `create_collection`, `drop_collection`, `put` (upsert), `patch` (merge), `get`, `query` (list), `delete`, and `delete_many`. 
*   **Filter Model:** Queries use a structured model (e.g., `Eq`, `In`, `Contains`), not raw SQL. This is translated to parameterized SQLite internally. Pagination is strictly cursor-based (no offset).
*   **Service Aliases:** 
    *   **Problem:** Service IDs are DIDs. Policies need human-readable names.
    *   **Design:** The community registry holds an alias record signed by the owner DID (e.g., `org-service -> did:syn:serviceXYZ`). Resolution order is: `local cache → registry → manifest-pinned DID`.
*   **Object Service (Blob Store):** 
    *   **Design:** Stored content-addressed (keyed by SHA-256 hash). One blob store per SynApp. Blob hashes are stored as standard string fields in the REST Data Service records. Replicas pull missing blobs lazily from primaries on first access.

#### 2. MQTT Event Service (Asynchronous Coordination)
To provide real-time Pub/Sub without relying on heavy external infrastructure, the Substrate acts as the broker.

*   **Embedded Broker Design:** The Syneroym Rust host embeds an MQTT broker natively using the `rumqttd` crate. It runs as a background Tokio task alongside the Wasmtime engine.
*   **The WASM Boundary (WIT):** WASM applications interact with the broker via a lightweight host boundary:
    ```wit
    interface pubsub {
        publish: func(topic: string, payload: list<u8>);
        subscribe: func(topic: string);
    }
    ```
*   **Execution Flow:** When a WASM component calls `pubsub::publish`, the host traps the call and routes it directly into the embedded `rumqttd` broker. Subscriptions trigger an exported WASM callback to push payloads into the guest.

#### 3. Universal Proxy (Inter-Component RPC)
The Substrate guarantees zero-overhead, strictly typed networking between services.

*   **Protocol Translation Architecture:**
    *   **Design:** The Substrate traps the typed WIT function call. If the target is another native WASM component, it serializes the call into **wRPC** (a highly efficient binary streaming protocol) and transmits it over encrypted **Iroh QUIC** streams.
    *   **JSON-RPC Adapter:** If the target is a legacy Podman container or an external web/mobile client, the proxy dynamically translates the strict WIT calls into universal JSON-RPC 2.0 over HTTP/WebSockets.

### [PLT-ASY] Asynchronous Operations & Scheduling

The Substrate handles offline behavior, long-running execution, and periodic scheduling uniformly by delegating explicit workflow management to the business logic, rather than attempting to build complex infrastructure-level "durable execution".

*   **Resilient RPC & Dead Letter Queues (DLQ):**
    *   **Design:** The Universal Proxy automatically handles retries with exponential backoff for transient failures. The retry limits are configurable per service. If the maximum limit is reached, the proxy traps the failure and routes the serialized message into a local SQLite-backed DLQ for later analysis or replay.
*   **The Outbox & Fire-and-Forget Semantics:**
    *   **Design:** Offline-capable endpoints are strictly opt-in. A client uses an outbox queue and sends a fire-and-forget message, marking the operation as optimistically successful in its local UI.
    *   **Client IDs:** To support this stateless offline creation, the Data Layer's CRUD operations support client-generated UUIDs (rather than strictly database-generated serial IDs).
*   **Long-Running Tasks (In-Memory Execution):**
    *   **Design:** Long-running workflows run as native asynchronous Tokio tasks executing WASM guest functions. The platform guarantees the *intent* to run is recorded durably, but the active WASM memory state is ephemeral. If the substrate crashes, the task is aborted and either restarted or compensated upon recovery. This avoids the massive engineering overhead of building event-sourced deterministic memory snapshotting.
*   **Periodic & Scheduled Tasks (Lease-Based Delegation):**
    *   **Design:** To eliminate load skew caused by clock drift in a replicated environment, cron-triggered execution is decoupled from scheduling. When a cron timer ticks across the cluster, nodes race to acquire a specific execution lease from the Registry.
    *   **Execution:** The node that wins the lease acts purely as the scheduler for that tick. It selects a target node from the active cluster (randomly or via load metrics) and dispatches the execution payload. The lease is held in the Registry until execution finishes, which naturally prevents overlapping execution if the task spans multiple cron periods.
*   **Saga Compensations (`undo` endpoints):**
    *   **Design:** Services participating in distributed workflows or offline operations expose standard `undo_<operation>` functions in their WIT boundary.
    *   **Execution:** When a multi-step operation or queued task hits a terminal failure, the orchestrator invokes the corresponding `undo_` function for each previously completed step. The orchestrator passes the exact same arguments (plus the generated resource ID) to the `undo` endpoint to accurately reverse the specific state change.

### [PLT-RED] Service Redundancy

*   **Database Replication Mechanism (Iroh WAL Shipping):** The data layer utilizes an N=2 topology (one primary, one read-only replica) for SQLite state. Instead of relying on Litestream or FUSE-dependent LiteFS for live node-to-node replication, Syneroym leverages its native Iroh transport for near-instant, zero-batching replication.
    *   **The Shipper (Primary):** A background Rust task monitors the `database.db-wal` file. Unlike Litestream which batches frames into segments to save on S3 `PUT` requests, our tailer reads the 24-byte frame header and 4KB page the moment it is committed by SQLite. It pipes these raw bytes directly into an open, persistent **Iroh multiplexed stream**.
    *   **The Lifter (Secondary):** The Secondary receives the byte stream from Iroh and directly appends it to its own local `database.db-wal` file. To ensure the local SQLite process instantly sees the new data without restarting, the Secondary runs a tiny routine to safely update the SQLite Shared Memory (`-shm`) index header, immediately making the new frames visible to read-only queries.
*   **Disaster Recovery:** While live, low-latency replication uses Iroh, periodic full backups and WAL segments are still asynchronously pushed to an external S3-compatible object store using a library like `wal-backup` for cold starts and disaster recovery.
*   **Registry & Coordination Model:** The Registry Service acts as the authoritative control plane for membership and topology.
    *   **Node States:** Nodes progress through strict states: `ACTIVE` → `SUSPECT` (communication issues) → `QUARANTINED` (no new work/traffic, but management allowed) → `RETIRED`. Quarantine is a global registry decision, not a local node guess.
    *   **Routing-Level Fencing:** Split-brain mitigation is handled purely at the network/routing layer. When a primary is deposed due to failure, the Registry marks it `QUARANTINED` and propagates this topology update to all clients and routers.
        *   *Ingress Protection:* Clients and upstream services clear their caches and stop routing writes to the deposed primary.
        *   *Egress Protection:* Even if the deposed primary is network-partitioned but still alive, any outbound requests or I/O it attempts to make to other services are rejected because its Node ID is verified against the cluster-wide blacklist.
    *   **Failure Philosophy (CP > AP):** Syneroym strictly prioritizes Consistency over Availability. There is no automatic failover. The promotion workflow requires an operator to manually quarantine the failed node in the Registry, wait for the topology update to propagate across the cluster, and then manually promote the existing Secondary to `ACTIVE` Primary. Finally, the operator provisions a new Secondary to restore N=2 redundancy.
*   **Control Plane vs Data Plane Isolation:**
    *   If the Registry itself fails, the Control Plane freezes (no new deployments or topology changes). However, the Data Plane continues unaffected indefinitely. Existing routing paths, MQTT flows, and HTTP access operate seamlessly using the last known cached registry state.
