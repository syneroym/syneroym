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

#### 1. Structured Data Service (Document API)
A platform-managed persistent store that SynApps use via the API.

*   **Database Isolation (One DB per Service):** Instead of a monolithic combined database, every service gets its own independent SQLite `.db` file (and WAL). The substrate also maintains its own separate database.
    *   *Benefits:* Maximizes write parallelism across the system (since each service has its own independent WAL lock), enables selective Iroh WAL replication per service, and isolates failure blast radiuses.
*   **Concurrency Architecture (Actor/Pool Model):** To handle high concurrency within a single service's database without hitting `SQLITE_BUSY` contention in Tokio, the platform utilizes **`rusqlite`** combined with **`deadpool-sqlite`**:
    *   *Why `rusqlite`:* The data layer requires raw access to the SQLite C API for advanced WAL/SHM replication mechanics and dynamic query generation (which invalidates `sqlx`'s compile-time macro benefits).
    *   *Reader Pool:* Read queries (e.g., `GET`, `LIST`) are dispatched across a `deadpool-sqlite` connection pool. This enables parallel, non-blocking reads and seamlessly bridges synchronous `rusqlite` calls into the Tokio runtime via `spawn_blocking`.
    *   *Single Writer Thread:* All mutations (`PUT`, `DELETE`) are routed via an `mpsc` channel to a single, dedicated background task holding an exclusive `rusqlite` write connection. This strictly aligns with SQLite's single-writer WAL design, eliminating lock contention entirely and allowing for optimized transaction batching.
*   **Resource Model:**  
    *   **Collection:** A named set of records within a SynApp's namespace, declared with a lightweight schema.
    *   **Record:** One JSON object identified by a caller-supplied string `id`.
    *   **`creator_id`:** A first-class field on every record, set automatically by the service at write time (spoof-proof).
    *   **Schema & Indexing:** Declares indexed fields explicitly. Enforced loosely (unknown fields rejected; declared fields type-checked). *Constraint:* In SQLite, `CREATE INDEX` requires an exclusive write lock. For very large collections, background schema evolution will temporarily block the single writer thread for that specific service, though read pools remain unaffected.
*   **CRUD & Batch Operations:** Operations include `create_collection`, `drop_collection`, `put` (upsert), `patch` (merge), `get`, `query` (list), `delete`, and `delete_many`. It also includes a `batch_mutate` function to perform atomic, multi-record mutations within a single SQLite transaction.
*   **Query & Aggregation Model:** Queries use a structured model (e.g., `Eq`, `In`, `Contains`) rather than raw SQL. This translates into parameterized SQLite internally with cursor-based pagination. It also supports an `AggregationPipeline` for advanced querying (`group_by`, `having`, `sum()`, projections) leveraging the full power of the WIT boundary. Aggregations can target both physical collections and logical views defined during init.
*   **Schema Initialization (DDL Variant):**
    *   *Design:* During the `init` hook of deployment, a SynApp provides DDL as a `variant ddl { sql(string), model(data-model) }`. The current implementation supports only the `sql(string)` arm, allowing standard SQLite DDL (`CREATE TABLE`, `CREATE VIEW`, `CREATE INDEX`).
    *   *Safety:* Plain SQL is safe to start with because each service has its own fully isolated `.db` file. A buggy or malicious DDL statement can only affect the service's own database, which is already gated by IAM access control.
    *   *Views in Init:* `CREATE VIEW` statements in the init DDL are instantaneous (zero write-lock penalty) and can be referenced by runtime `AggregationPipeline` queries just like physical collections.
    *   *Future:* The `model(data-model)` variant arm is reserved for when the platform is opened to untrusted third-party developers who should not be permitted to run arbitrary DDL. At that point, `sql(string)` can be restricted via IAM policy.
*   **Service Aliases:** 
    *   **Problem:** Service IDs are DIDs. Policies need human-readable names.
    *   **Design:** The community registry holds an alias record signed by the owner DID (e.g., `org-service -> did:syn:serviceXYZ`). Resolution order is: `local cache → registry → manifest-pinned DID`.
*   **Object Service (Blob Store):** 
    *   **Design:** Stored content-addressed (keyed by SHA-256 hash). One blob store per SynApp. Blob hashes are stored as standard string fields in the Structured Data Service records. Replicas pull missing blobs lazily from primaries on first access.

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
    *   **WIT Interception and Late Binding:** Dependencies are declared as generic WIT imports (e.g., `import acme:booking/service;`). At instantiation, the Substrate satisfies these imports by injecting dynamically generated proxy host functions. When the WASM component invokes the import, execution traps to the host proxy. 
    *   **Instance Routing:** The proxy relies on the application manifest and the Orchestrator's App Registry to resolve the generic WIT import to a specific deployed instance's `service_id`. It bakes this route into the proxy, meaning the developer codes against generic contracts, but the Substrate handles disambiguated instance routing automatically.
    *   **Design:** Once trapped, if the target is another native WASM component, the Substrate serializes the call into **wRPC** (a highly efficient binary streaming protocol) and transmits it over encrypted **Iroh QUIC** streams to the correct instance.
    *   **Native Host Service Proxying:** If the target is a native host service (e.g., the `syneroym:data-layer/store` WIT import) mapped to a remote instance, the Substrate behaves identically. It proxies the call via wRPC to the remote Substrate, which receives the call and executes its *own* native host service implementation (e.g., executing against its local `cr-sqlite` file).
    *   **JSON-RPC Adapter:** If the target is a legacy Podman container or an external web/mobile client, the proxy dynamically translates the strict WIT calls into universal JSON-RPC 2.0 over HTTP/WebSockets.
    *   **Static Composition Bypass:** If the component and its dependency are statically composed into a single `.wasm` binary prior to deployment (e.g., via `wasm-tools compose`), the import is satisfied internally. The Substrate never sees the import, no proxy is injected, and the call executes entirely within the WebAssembly sandbox with zero-overhead.

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

*   **Database Replication Mechanism (Iroh WAL Shipping):** The data layer utilizes a configurable N-replica topology (typically one primary, and zero or more read-only secondaries, as defined by the app manifest) for SQLite state. Instead of relying on Litestream or FUSE-dependent LiteFS for live node-to-node replication, Syneroym leverages its native Iroh transport for near-instant, zero-batching replication.
    *   **The Shipper (Primary):** A background Rust task monitors the `database.db-wal` file. Unlike Litestream which batches frames into segments to save on S3 `PUT` requests, our tailer reads the 24-byte frame header and 4KB page the moment it is committed by SQLite. It pipes these raw bytes directly into an open, persistent **Iroh multiplexed stream**.
    *   **The Lifter (Secondary):** The Secondary receives the byte stream from Iroh and directly appends it to its own local `database.db-wal` file. To ensure the local SQLite process instantly sees the new data without restarting, the Secondary runs a tiny routine to safely update the SQLite Shared Memory (`-shm`) index header, immediately making the new frames visible to read-only queries.
*   **Disaster Recovery:** While live, low-latency replication uses Iroh, periodic full backups and WAL segments are still asynchronously pushed to an external S3-compatible object store using a library like `wal-backup` for cold starts and disaster recovery.
*   **Registry & Coordination Model:** The Registry Service acts as the authoritative control plane for membership and topology.
    *   **Node States:** Nodes progress through strict states: `ACTIVE` → `SUSPECT` (communication issues) → `QUARANTINED` (no new work/traffic, but management allowed) → `RETIRED`. Quarantine is a global registry decision, not a local node guess.
    *   **Routing-Level Fencing:** Split-brain mitigation is handled purely at the network/routing layer. When a primary is deposed due to failure, the Registry marks it `QUARANTINED` and propagates this topology update to all clients and routers.
        *   *Ingress Protection:* Clients and upstream services clear their caches and stop routing writes to the deposed primary.
        *   *Egress Protection:* Even if the deposed primary is network-partitioned but still alive, any outbound requests or I/O it attempts to make to other services are rejected because its Node ID is verified against the cluster-wide blacklist.
    *   **Failure Philosophy (CP > AP):** Syneroym strictly prioritizes Consistency over Availability. There is no automatic failover. The promotion workflow requires an operator to manually quarantine the failed node in the Registry, wait for the topology update to propagate across the cluster, and then manually promote an existing Secondary to `ACTIVE` Primary. Finally, the operator provisions a replacement node to restore the desired redundancy level defined in the manifest.
*   **Control Plane vs Data Plane Isolation:**
    *   If the Registry itself fails, the Control Plane freezes (no new deployments or topology changes). However, the Data Plane continues unaffected indefinitely. Existing routing paths, MQTT flows, and HTTP access operate seamlessly using the last known cached registry state.

## Phase 3: Substrate & Application Lifecycle

### [LFC-MGT] SynApp Lifecycle Management Design
This section details the internal mechanics of the dual-mode Orchestration and Lifecycle Management system.

#### 1. Control Plane Architecture & Bootstrapping
Syneroym supports both a decentralized CLI workflow and a centralized stateful controller.
*   **CLI Standalone (`roymctl`)**: Acts as a thick client. It parses the SynApp manifest, directly initiates connections to target substrates via Iroh/WebRTC, and executes deployment commands.
*   **Server SynApp (Active Control Plane)**: A specialized, long-running SynApp that exposes a REST/RPC API for orchestration.
*   **Bootstrapping**: The Server SynApp is deployed identically to any other application via `roymctl`. Once deployed, the user configures their local `roymctl` to proxy subsequent commands to the Server SynApp's API endpoint rather than pushing to substrates directly.

#### 2. State Management (SQLite)
Both operational modes rely on an identical set of core orchestration libraries and utilize SQLite for state tracking, but with different semantics:
*   **Local Installation Trace (`roymctl`)**: In CLI mode, the local SQLite database acts merely as an installation trace. It records what was deployed, where, and when. This allows subsequent `roymctl reconcile` commands to compute the diff between the manifest and the known deployments, but there is no background monitoring.
*   **Authoritative Ledger (Server SynApp)**: The Server SynApp utilizes the platform's standard stateful replication (via Iroh WAL shipping) for its internal SQLite database. This database stores the authoritative Desired State (manifests) vs. Actual State (live telemetry from substrates), driving its continuous background reconciliation loop.

#### 3. Service Discovery & Logical Routing Resolution
A critical function of Lifecycle Management is translating Explicit Bindings defined in the manifest into physical, routable identities (e.g., Iroh Pubkeys or DIDs) for inter-service communication.
*   **Static Injection (CLI Mode)**: When operating in standalone mode, `roymctl` acts as a static compiler for routing. During deployment or manual reconciliation, it determines the physical identities of the target substrates. It then explicitly injects these physical IDs into the configuration payload of the dependent `SynSvc` components. Without a Server SynApp, there is no dynamic load balancing or self-healing routing—if a service moves, a new `roymctl reconcile` is required to push the updated configuration.
*   **Dynamic Pull (Server SynApp Mode)**: When the Server SynApp is active, it functions as a dynamic Registry Service. The client SDKs embedded within each `SynSvc` query this registry at runtime to resolve logical IDs to physical addresses. This enables dynamic load-balancing, auto-discovery of newly scaled instances, and seamless failover without requiring static configuration updates.

### [LFC-VER] Versioning & Migration Flow

#### 1. WASM Component Database Migration Lifecycle
To handle data schema evolution securely, the system utilizes a lifecycle hook within the WASM component, executed with elevated capabilities.

*   **1. Pause & Snapshot**: When an upgrade is initiated, the Substrate pauses traffic to the specific `SynSvc` endpoint. The internal router temporarily buffers requests or returns a `503 Service Unavailable`. The Substrate takes a filesystem-level snapshot of the component's underlying SQLite database.
*   **2. Elevated Init Execution**: The Substrate loads the new version of the WASM binary and invokes its exported `init()` (or `migrate()`) lifecycle hook. Crucially, the Substrate injects an elevated capability (e.g., an Admin UCAN) into this execution context.
*   **3. DDL and Transformation**: Within `init()`, the WASM code leverages generic data-layer host functions (e.g., `execute_sql`) to perform its schema changes (DDL) and any necessary data transformations. The elevated capability allows the execution of DDL, whereas standard REST/RPC invocations are sandboxed to restricted CRUD operations.
*   **4. Commit or Rollback**:
    *   If `init()` returns `Ok`, the Substrate considers the upgrade successful, drops the old database snapshot, and resumes routing traffic to the new component.
    *   If `init()` returns `Err` (or panics), the Substrate aborts the upgrade, unloads the new WASM module, restores the SQLite database from the snapshot, and resumes routing traffic to the previous WASM binary.

#### 2. Network Protocol Handshake & Capability Matrix
To avoid rigid (and brittle) version matching across a decentralized network, Substrates negotiate network capabilities dynamically.

*   **1. Handshake Negotiation**: During the initial connection phase over Iroh, node A and node B exchange a list of their supported protocol profiles (e.g., `["syneroym/rpc/v1", "syneroym/rpc/v2"]`).
*   **2. Capability Resolution**: The routing layer inspects the shared protocols and establishes communication using the most capable, mutually understood protocol. If the intersection is empty, the connection is cleanly rejected.
*   **3. Case-by-Case Deprecation**: Rather than an automatic sliding-window (N-x) deprecation policy, the core team removes older protocol handlers from the Substrate binary on a deliberate, case-by-case basis as the network matures.

## Phase 4: Advanced Services & Tooling

### [ADV-OBS] Observability enhancements

#### 1. Observability Pipeline & Non-Blocking Emission
To prevent observability from adding latency to the hot paths (WASM execution and Iroh/WebRTC network routing), the metrics pipeline relies on an asynchronous, decoupled architecture:
*   **Event Emitters**: The core components (Router, WASM runtime, and Gateway) emit raw metrics as lightweight data structs.
*   **MPSC Channels**: These structs are sent over non-blocking `tokio::sync::mpsc` channels to a dedicated, low-priority `Observability Engine` background task running within the Substrate.
*   **Buffering**: The channel buffers smooth out high-throughput spikes, preventing hot-path execution delays even during heavy load.

#### 2. Dedicated Time-Series Storage (metrics.db)
Observability data is high-volume and append-heavy. To prevent these operations from contending with the critical operational state of the network (`substrate.db`), metrics are directed to a dedicated embedded database:
*   **File Isolation**: A separate `metrics.db` SQLite database is maintained.
*   **Rollup Engine (Cron Task)**: A background Tokio cron task wakes up periodically (e.g., every 5 minutes) to perform aggregations. It selects raw events older than a certain threshold, aggregates them into 1-hour buckets, inserts the buckets into a `metrics_1h` table, and prunes the raw events to reclaim space.
*   **Extensible Schema**: The tables (`metrics_raw`, `metrics_1h`) feature an extensible JSON or BLOB column (`metadata`) to dynamically accommodate new attributes like AI/LLM token usage, GPU execution metrics, and future billing parameters ("agreed rates") without requiring strict schema migrations.

#### 3. Metering & Relays
For multi-hop scenarios and standard data routing, measuring data transfer is crucial:
*   **Stream Counting**: The routing proxy layer maintains byte counters (`bytes_tx`, `bytes_rx`) for every active stream.
*   **Identity Tagging**: These counters are strongly associated with the authenticated Peer IDs (DIDs) of the connection.
*   **Periodic Flush**: Counts are flushed to the `Observability Engine` when a stream closes or at set intervals for long-lived streams. Cryptographic receipts are intentionally excluded in this phase to maintain simplicity; logging the attested counts provides sufficient baseline trust for standard metering.

#### 4. Authorized Access
Accessing the `metrics.db` is securely gatekept by the unified `authorization-engine` via standard RPC endpoints:
*   **Root Capabilities**: The Substrate owner uses an administrative UCAN, resulting in queries running without restrictions against `metrics.db`.
*   **Scoped Capabilities**: SynApp/SynSvc owners invoking the metrics RPC present a UCAN bound to their identity. The engine transparently injects a `WHERE service_owner_did = ?` clause into the underlying SQL query.
*   **Data Consumption**: The Substrate does not host its own visualizations. Instead, the metric data is consumed by standalone SynApps or dedicated BI tools acting as external clients.

### [ADV-AI] Advanced AI & Agentic Workflows

*   **Local Inference Engine (Ollama / Candle):**
    *   **Design:** The substrate orchestrates **Ollama** as a managed process, exposing its API through the Universal Proxy. Alternatively, for tighter integration without external daemons, HuggingFace's **Candle** framework could be embedded directly into a Rust host extension for in-process inference of GGUF models.
*   **The Concierge Agent Architecture:**
    *   **Design:** A native WASM `SynSvc` built using **`rig-core`**. Crucially, `rig-core` remains the foundational bedrock for *all* agentic workflows. It provides the core abstractions for communicating with LLMs, generating embeddings, and defining MCP tools in Rust. The different "Architectures" below are simply routing logic built *on top* of `rig-core`'s completion API.
    *   **Workflow Architectures (Hybrid Approach):**
        *   **1. Plan-and-Solve (Generic Fallback):** For open-ended queries, the agent uses `rig-core` to generate a sequential plan, then executes a generic loop against that plan.
        *   **2. Finite State Machines (FSM):** For critical workflows (e.g., "Checkout Process"), developers define explicit stages. The Concierge Agent uses `rig-core` to execute the specific prompt and tool bindings for the active stage, guaranteeing safe recovery points.
        *   **3. Multi-Agent Delegation:** For highly compartmentalized tasks, the Concierge Agent uses `rig-core` to spin up and orchestrate specialized sub-agents.
*   **Retrieval Augmented Tool Discovery:**
    *   **Execution:** The agent provides a `search_tools` function. When called, it searches `sqlite-vec`, dynamically appends matching MCP definitions to the LLM's context array, and prompts the LLM to continue.
*   **Human-in-the-Loop & Execution Validation:**
    *   **Design:** HITL and Observability are strictly configuration-driven. If a tool schema explicitly defines `requires_consent=true`, the agent suspends its Tokio task and pushes a Consent Request.
*   **Agent-to-Agent Delegation Protocol:**
    *   **Design:** Agents utilize the Universal Proxy and Global Registry to establish encrypted Iroh streams and exchange structured negotiation intents.
*   **Vector Database & Long-Term Memory (sqlite-vec):**
    *   **Design:** The Ecosystem Vector Directory and Long-Term Memory are backed by **`sqlite-vec`**, integrating seamlessly with the existing `cr-sqlite` infrastructure.

## Phase 5: High-Level Applications (SynApps)

### [APP-AGG] The Aggregator SynApp Architecture

*   **Design:** Aggregators are fundamentally robust indexing engines. Providers and users push structured data to the Aggregator via standard RPC.
*   **Client Sync & Discovery:** Local nodes run background cron tasks to pull schemas from trusted Aggregator SynApps into their local `sqlite-vec` database.
*   **Fuel Quota Execution:** When a provider pushes a listing, the Aggregator consults its local SQLite database to check the provider's remaining "fuel" balance. If sufficient fuel exists, the listing is committed, and the fuel is decremented.
