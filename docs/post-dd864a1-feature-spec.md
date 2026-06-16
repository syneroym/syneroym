# Pending Features Master Spec
This document describes the pending features for Syneroym post git commit hash `dd864a18902bb8e71da0ff56bba4523688ad8ba1`. 

### Tag Legend
To ensure stable cross-referencing across commits and PRs, features are prefixed with category tags:
- **`[TOP]`**: **Topology** (Core Architecture Primitives)
- **`[FND]`**: **Foundation** (Core Infrastructure & Security)
- **`[PLT]`**: **Platform** (Data Layer & Resilience)
- **`[LFC]`**: **Lifecycle** (Substrate & Application Management)
- **`[ADV]`**: **Advanced** (Advanced Services & Tooling)
- **`[APP]`**: **Applications** (High-Level SynApps)
- **`[EDG]`**: **Edge** (Edge Expansion & Mobile)

## Phase 0: Core Architecture Implementation (SynApp & Topology)

This phase implements the architectural boundary between Syneroym Applications (`SynApp`) and Syneroym Services (`SynSvc`), and the pending addressing and registry systems required for robust service discovery.
*(Note: Service IDs, Global Identity Registries, and Endpoint Registries are currently implemented. The remaining app-level topologies, logical names, and orchestrators fall under this phase).*

### [TOP-PRM] Core Primitives (`SynSvc`) vs. Control Plane Overlay (`SynApp`)

#### `SynSvc` (The Execution Primitive)
The `SynSvc` is the absolute foundational primitive of the Syneroym Substrate. 
*   **Zero-Trust Execution:** Represents an isolated, zero-trust execution boundary (often a WASM component, but could also be a Podman container or a native OS service sitting behind a platform gatekeeper). It does not implicitly trust other services, even those deployed alongside it.
*   **State & Capabilities:** It owns its state and enforces capability-based security (FDAE ReBAC policies and UCANs) on all incoming requests.

#### `SynApp` (The Control Plane Overlay)
`SynApp` is removed as a runtime execution boundary and is redefined as a **Deployment Manifest and Control Plane Overlay**.
*   **Lifecycle Management:** Acts as a blueprint to deploy, update, and remove a cohesive graph of `SynSvcs` as a single unit.
*   **Capability Bootstrapping:** Orchestrates the initial injection of permissions (ReBAC relations/policies) that allow internal services within the app to communicate.
*   **Resource Accounting:** Serves as a logical grouping for tracking quotas, billing, and telemetry across a designated graph of services.
*   **SynApp Instances:** Unlike Erlang applications (which are singletons), deploying a `SynApp` manifest creates a unique **SynApp Instance** with an isolated namespace. A single substrate can host multiple distinct instances of the same `SynApp` (e.g., Personal Task Manager vs. Work Task Manager).
*   **UI Decoupling:** User Interfaces are simply specialized `SynSvcs` or external clients. A `SynApp` may contain zero UIs (headless processes), one UI, or multiple specialized UIs (Admin, Storefront, Mobile Gateway).

#### Composable SynApps (App Dependencies)
Similar to Erlang OTP applications, `SynApps` are highly composable. A `SynApp` manifest is not restricted to explicitly declaring raw `SynSvcs`; it can declare dependencies on other `SynApps`.
*   **Dependency Resolution:** If `SynApp: Retail Store` depends on `SynApp: Identity Core`, the Orchestrator evaluates the dependency graph during deployment. It will ensure `Identity Core` is instantiated (or bind to an existing instance) before deploying the `Retail Store` instance.
*   **Instance Mapping:** This maintains the crucial App (Blueprint) vs. App Instance distinction. A higher-level SynApp can compose multiple foundational SynApps into a unified, deployed ecosystem, passing necessary capabilities down the dependency tree.

---

### [TOP-ADR] Service Addressing and Resolution Topology

Services communicate using a multi-tiered addressing model to support mobility, redundancy, and explicit targeting.

#### Addressing Types
1.  **Explicit Service ID (Physical ID):** 
    *   An immutable, cryptographic identifier (e.g., UUID, hash, or Ed25519 public key) representing a specific, running instance of a `SynSvc` (e.g., `svc_8a7b6c5d...`).
    *   Provider ownership of the service is proven via cryptographic certificates or UCANs, not the routing ID itself.
    *   Used for stateful interactions, direct replies, and underlying substrate routing.
2.  **Logical Service Name:** 
    *   A human-readable or contextual identifier (e.g., `profile-svc`, `ledger-primary`) representing a *role*.
    *   Used by developers in code to ensure high availability, load balancing, and decoupling.

#### Service Topologies
When registering a Logical Service Name, the local registry tracks its underlying topology:
*   **Singleton:** Maps to exactly one Explicit ID.
*   **Redundant (Load Balanced):** Maps to an array of Explicit IDs. The registry or caller load-balances requests across them.
*   **Sharded:** Maps to multiple Explicit IDs based on a routing key (e.g., `user_id % 3 > explicit_id_X`).

---

### [TOP-REG] Types of Registries in the Ecosystem

Rather than a strictly monolithic system, service discovery naturally emerges across different registry scopes:

1.  **Global Identity Registry (DHT):** Resolves top-level Provider/Node identities to physical network addresses (e.g., WebRTC sockets, Iroh ALPNs, IP addresses).
2.  **Contextual/App Registry:** Resolves Logical Service Names to Explicit Service IDs within a specific app overlay context or shared node namespace.
3.  **Endpoint/Router Registry:** The internal substrate routing table. Maps an Explicit Service ID to actual execution boundaries (e.g., local WASM host functions or remote network sockets).

---

### [TOP-DSC] Discovery Mechanisms and Inventory

#### The `syneroym-core/registry` Default Service
To provide "batteries-included" service discovery, the platform provides a canonical Registry `SynSvc`.
*   **Purpose:** Acts as the local source of truth for service inventory, logical-to-physical mapping, and health tracking.
*   **Manifest Configuration:** A `SynApp` manifest dictates how the Orchestrator handles this:
    1.  **Spawn:** Boot a dedicated, isolated instance of the Registry `SynSvc` exclusively for this app. To handle this correctly, the `SynApp` manifest must support a service dependency graph (either explicitly declared or inferred via references) so the Orchestrator boots the registry *before* the services that depend on it.
    2.  **Bind:** Register the app's services with a pre-existing, shared node-level Registry `SynSvc` (saving resources).
*   **Client Caching (Refresh-on-Failure):** Service clients query the registry to resolve a logical name, **cache** the resulting Explicit ID locally, and communicate directly with the Explicit ID. The cache is only evicted and refreshed if a connection failure occurs.

#### Static Deployment Inventory (`roymctl`)
Not all apps require a live, queryable registry at runtime (e.g., trivial background cron jobs or standalone static UIs).
*   For these trivial apps, the orchestrator/CLI (`roymctl`) maintains a static, local state file (or local host DB).
*   It records the mapping of `App Name > Explicit Service ID(s)` at deploy time.
*   This static inventory is sufficient for lifecycle management (listing, stopping, uninstalling) without the overhead of spinning up a live Registry `SynSvc`.

---

## Phase 1: Foundation & Core Infrastructure

### [FND-DEP] Deployment/Operations
- **Cloud-Agnostic Bare-Metal Deployment:** Single Rust binary deployed to a standard Linux instance (e.g., AWS Lightsail) using native `systemd` to minimize virtualization overhead.
- **In-Repo Provisioning & Deployment:** `scripts/deploy/setup_linux.sh` handles initial machine setup (certbot, limits, systemd), while `scripts/deploy/deploy.sh` handles local compilation and rsync transfer. GitHub Actions (`.github/workflows/deploy.yml`) acts merely as a trigger to run the local deploy script.
- **Native TLS:** Direct binding to port 443 within the Syneroym substrate using `rustls`. Certificates are fetched/renewed via an OS-level `certbot` timer. The substrate restarts to reload certificates.
- **Resource Protection:** Configuration parameters for connection caps and cache limits ensure the node gracefully refuses excess traffic instead of crashing (OOM).
- **Observability:** Phase 1 relies purely on SSH access. Operators monitor via `journalctl -u syneroym -f` and local `curl` requests against built-in endpoints (e.g., Iroh relay metrics).
- **Cross-Platform Distribution:** Automated build pipelines to compile and release Syneroym binaries for different architectures (Linux, macOS, Windows).
- **Dockerized Substrate:** Provide official Docker images of the Syneroym substrate for the community, pre-configured to point their local registries and coordinators to the public `syneroym.xyz` node.
- **Smoke Testing:** Automated integration/smoke tests that run against release candidates (binaries and Docker images) to verify they can successfully connect to and interact with the deployed coordinator and registry at `syneroym.xyz`.

### [FND-SEC] Substrate Security
- **Data at Rest Encryption (Envelope Encryption):** 
  - To prevent catastrophic re-encryption of gigabytes of data during key rotation, the substrate uses Envelope Encryption. Unique Data Encryption Keys (DEKs) are generated to encrypt the actual blobs and `cr-sqlite` databases.
  - The service owner negotiates and injects a Master Key (Key Encryption Key or KEK) securely into substrate RAM at startup. The KEK only encrypts the tiny DEKs stored on disk. Key rotation is instantaneous as only the DEKs are re-encrypted with the new KEK.
  - **Secret Vault:** Application secrets (API keys, credentials) and configurations are stored securely inside a dedicated Vault table within the encrypted `cr-sqlite` database, rather than as vulnerable flat files on disk.
  - The `SynApp` manifest includes configuration flags (e.g., `encrypt_local_db: true`, `encrypt_backups: true`) to allow opting out of encryption overhead when performance is prioritized over secrecy.
  - Remote backups (e.g., Litestream WAL frames) are streamed to other S3-protocol-compatible substrates, encrypted locally before transit if configured.
- **Hardware Attestation (Deployer-Led):** 
  - The substrate exposes a `substrate.attest(nonce)` API to the network.
  - The App Deployer/Owner externally challenges the node (at deployment or periodically) and mathematically verifies the hardware quote (TPM, KeyAttestation, AppAttest).
  - The deployer alone decides whether to deploy the service in a degraded trust environment or halt execution if attestation fails. 
- **Memory Protection & Key Splitting:**
  - OS-level memory locking (e.g., `mlock`) prevents injected cryptographic keys from being swapped to disk.
  - The `zeroize` crate is used to explicitly wipe sensitive variables from RAM when dropped.
  - Keys in substrate memory are split or fragmented to mitigate extraction via buffer over-read vulnerabilities.
- **Resource Exhaustion & Quotas:**
  - Network edge protection: Strict connection and payload limits at the Iroh/QUIC boundary.
  - Runtime execution limits: The substrate enforces the physical capabilities of the host alongside strict quotas defined in the `SynApp` manifest (e.g., `max_memory`, `max_instructions`). Wasmtime's fuel metering deterministically traps components exceeding their gas limits without stalling the node.
- **Supply Chain Integrity:**
  - Syneroym binaries are distributed with simple, native Ed25519 signatures. The public key is hardcoded, and the auto-updater mathematically verifies the signature before applying any new binary.

> **Implementation Design:** For technical details covering Envelope Encryption and Memory Protection, see [Feature Design: FND-SEC](post-dd864a1-feature-design.md#fnd-sec-substrate-security).



### [FND-CFG] Service Configuration

Given that Syneroym supports both native WASM components and legacy Podman containers, configuration and secret management use a dual-target approach:

- **Configuration Delivery**:
  - **WASM (Native)**: Services retrieve their hierarchical configuration on-demand via a standard host function (e.g., `syneroym:config/get`).
  - **Podman (Legacy)**: Because third-party containers expect specific formats, the `SynApp` manifest dictates how the orchestrator exposes the config. The orchestrator will either flatten the config into standard environment variables or serialize nested configurations (JSON/TOML/YAML) into temporary files and mount them read-only into the container.
- **Secret Management**:
  - **WASM (Native)**: Strictly adheres to `[FND-SEC]`. The service pulls secrets directly into locked RAM via `syneroym:vault/reveal`. Secrets never touch the filesystem or environment variables.
  - **Podman (Legacy)**: The orchestrator resolves the secret from the Vault at deployment and injects it as an environment variable or via an ephemeral `tmpfs` mount. This accepts a degraded security posture (secrets visible in process lists) as a necessary tradeoff for running legacy software.
- **Dynamic Updates & Restarts**: Configuration is immutable. For WASM, configuration changes instantly apply to the next component invocation. For Podman, the orchestrator must gracefully restart/recreate the long-lived container to apply the new configuration or secrets.
- **App Composition (Bind vs. Spawn)**: When a parent `SynApp` depends on another app, the configuration resolves based on the dependency mode:
  - **Spawn**: If the dependency must be spun up alongside the parent, `roymctl` inlines the child manifest into the parent at deploy-time, creating a single flattened deployment graph.
  - **Bind**: If the parent depends on an *already running* app instance, the parent manifest references it. The orchestrator resolves the target's Explicit Service IDs via the App Registry and injects those connection details into the parent's configuration, rather than spawning new instances.
- **Schema Validation & Defaults**: To prevent runtime crashes, `SynSvc` manifests can define a schema (e.g., JSON Schema) for their expected configuration. `roymctl` and the Orchestrator validate the user-provided configuration against this schema at deploy-time, catching missing keys or type mismatches early.
- **Out-of-Band Secret Rotation**: While regular configuration changes happen via explicit manifest deployments (which naturally trigger a restart), secrets live independently in the Vault. If a secret is rotated *out-of-band* by an admin, the manifest's `rotation_policy` dictates whether the orchestrator automatically restarts the affected service or waits for the next manual deployment.
- **Anti-Goal: "Helm-ification"**: The `SynApp` manifest is strictly a "dumb", fully-resolved document. Syneroym rejects complex in-manifest templating (like Helm). If developers need environment-specific overrides, they should use external tools (like `cue`, `ytt`, or simple scripts) to generate a static manifest *before* passing it to `roymctl deploy`. The only dynamic variables supported are standard host parameters (e.g., `SYNEROYM_NODE_IP`) that the orchestrator inherently injects at runtime.

> **Implementation Design:** For technical details regarding the dual-target configuration delivery and cold restart behavior, see [Feature Design: FND-CFG](post-dd864a1-feature-design.md#fnd-cfg-service-configuration).

### [FND-IAM] Access Control
- **FDAE (Federated Data-Aware Authorization Engine):** Adopts the FDAE architecture, which decouples the authorization specification (the "What") from the environment-specific execution (the "How"). It avoids the traditional PBAC vs. ReBAC dilemma by acting as an intelligent, distributed routing engine. It utilizes a declarative, Zanzibar-style structured configuration (e.g., YAML/JSON) to map relationship chains across fragmented data sources. The Substrate directly deserializes this configuration—avoiding custom parsers or ASTs—and acts as a distributed query planner.
- **Solving the Data Fetching Problem (Pushdown Sieve):** For local contiguous relationships, FDAE collapses the graph into a single, deeply nested query. By compiling ReBAC policies directly into SQL `WHERE EXISTS` clauses, the SQLite engine performs massive-scale relationship filtering at the C-level, handing only authorized rows back to the WASM guest.
- **Dual-Mode Execution:** FDAE natively handles both **Point-In-Time Evaluation** (returning a swift Allowed/Denied flag for a specific resource check) and **Relational Data Filtering** (applying the security policies as a global subquery to prune index-level datasets before they ever reach the Wasm guest).
- **UCAN Integration (Normalized Claims, Capabilities, Scopes):** Access control is a robust synthesis of cryptographic capabilities and relational data state.
  - **Context Initialization:** When a request arrives, the gateway mathematically verifies the UCAN chain, normalizing external authentications (OIDC, DIDs, WebAuthn) into internal DIDs. It extracts the proven **claims**, **capabilities**, and **scopes**.
  - **Relational Verification:** The SQL Compiler uses these normalized UCAN scopes and claims as bound parameters (`?`) for its query.
- **The Extensible 4-Stage Hybrid Pipeline (Federated + SQL + WASM):** The authorization pipeline handles complex cross-boundary logic seamlessly:
  1. **Pre-Step (Context & UCAN Verification):** The substrate verifies the UCAN chain into a secure execution context.
  2. **Cross-Service Parameter Fetch:** If a relationship step crosses an asset boundary (e.g., requires data from a centralized security or org-service), the engine pauses, triggers an RPC/Wasm host function to fetch the remote relationship proofs or parameters, and injects them into the local evaluation context.
  3. **SQL Execution (The Relational Sieve):** SQLite natively filters candidate rows based on the ReBAC policies, UCAN context, and any parameters fetched during the cross-service fetch.
  4. **After-Step (ABAC & Override Filter):** An optional custom WASM function performs fine-grained, non-relational ABAC checks on the candidate rows.

> **Implementation Design:** For technical details regarding the FDAE architecture and the 4-stage hybrid pipeline, see [Feature Design: FND-IAM](post-dd864a1-feature-design.md#fnd-iam-access-control).

## Phase 2: Core Platform Capabilities

### [PLT-DAT] Data Layer
The Data Layer provides a complete foundation for distributed application state and communication, securely accessed via typed host functions or APIs without exposing raw database engines to the applications.

- **REST Data Service (Structured Database):**
  - **Namespaced Views:** The canonical primitive for structured state (backed by SQLite). The substrate owns the store; SynApps borrow a securely namespaced view (`app_id`).
  - **Resource Model:** Collections with lightweight schemas (loose enforcement of types, explicit indexed fields) containing JSON records. The data layer automatically injects a spoof-proof `creator_id` into every record.
  - **Operations & Queries:** Full CRUD operations (`create_collection`, `put`, `patch`, `get`, `delete`, `delete_many`). The query engine translates an abstract `FilterExpr` (supporting `Eq`, `In`, `Contains` for full-text, etc.) into parameterized SQL queries with cursor-based pagination.
  - **WASM Serialization & WIT Boundary:** Expand the `syneroym:data-layer/store` WIT boundary to support robust nested record serialization/deserialization. Currently, only basic types are supported; this enables seamless passing of complex JSON object graphs between WASM components and the host.

- **Object Service (Content-Addressed Blobs):**
  - **S3-Compatible Storage:** Dedicated blob storage for large media and software artifacts, natively content-addressed (keyed by SHA-256).
  - **Data Integration:** Blob hashes are stored as standard string fields in the REST Data Service records.
  - **HTTP File Serving:** Built-in HTTP serving of public/private objects (with signed URLs), supporting static website hosting and CDN-friendly delivery directly from the blob store.

- **MQTT Event Service (Asynchronous Coordination):**
  - **Pub/Sub Broker:** Handles asynchronous communication, state propagation, and device workflows.
  - **Features:** Supports wildcard topics, retained messages, and real-time change notifications to trigger workflows or invalidate caches.

- **Universal Proxy (Inter-Component RPC):**
  - **Typed Interactions:** Developers use strongly typed WIT imports (`import acme:booking/service;`) rather than generic untyped APIs.
  - **Protocol Translation:** The substrate traps the WASM call and dynamically proxies it. It serializes the call into fast, binary **wRPC** (over Iroh QUIC) if the target is another WASM substrate, or universal **JSON-RPC** (over HTTP/WebSocket) if targeting a legacy Podman container.

> **Implementation Design:** For technical details regarding the embedded MQTT broker and the wRPC Universal Proxy architecture, see [Feature Design: PLT-DAT](post-dd864a1-feature-design.md#plt-dat-data-layer).

### [PLT-ASY] Asynchronous Operations & Scheduling

The Asynchronous Operations component ensures reliable execution of offline interactions, long-running workflows, and periodic tasks, even in the presence of network partitions or transient service failures.

- **Resilient RPC & Retries:**
  - **Configurable Policies:** Retry policies (e.g., exponential backoff, maximum attempts) are defined at the service level by default, but can be overridden per-request.
  - **Dead Letter Queue (DLQ):** When the maximum retry limit is reached, failed messages are routed to a Dead Letter Queue for auditing, manual intervention, or later replay, preventing silent data loss.

- **Offline Message Semantics & Outbox:**
  - **Optimistic Local Execution (Fire-and-Forget):** Clients can trigger operations using a fire-and-forget flag (or wrapper API) indicating it is "ok to send later". The client UI treats the request as optimistically successful. 
  - **Return Value Constraints:** Offline-capable calls cannot synchronously return data (e.g., a server-generated ID). Applications must rely on client-generated identifiers (e.g., UUIDs) or design the interaction to not require immediate server responses.
  - **Outbox Queue:** Offline requests are durably stored in an outbox queue and periodically synced when connectivity is restored.

- **Long-Running Tasks:**
  - **Uniform Execution:** Long-running tasks composed of multiple compute and service calls are supported uniformly (e.g., executed as standard Wasm functions).
  - **In-Memory State Management:** The request to start the task is durably recorded, but the active execution state resides in the asynchronous engine's memory. If the process is interrupted, the task restarts or fails rather than resuming from a mid-execution disk snapshot.

- **Periodic & Scheduled Tasks:**
  - **Lease-Based Scheduler:** To prevent load skew and overlapping executions in a clustered environment, cron/periodic triggers use an execution lease mechanism backed by the Registry.
  - **Delegated Execution:** The node that wins the lease acts only as the "Orchestrator" for that tick. It selects a target worker node (e.g., randomly or via load metrics) and dispatches the actual execution command to it.
  - **Overlap Prevention:** The lease is held in the Registry until the executing node completes the work. If the task exceeds its cron interval, subsequent timer ticks across the cluster will fail to acquire the lease and safely skip the run.

- **Compensating Transactions (Saga Pattern):**
  - **Undo Interfaces:** To handle permanent failures in distributed scenarios without leaving the system in an inconsistent state, services can expose `undo_<operation>()` endpoints in their WIT interfaces.
  - **Automated Rollback:** If a step in a multi-stage workflow fails permanently, the orchestrator executes the corresponding compensating functions for the previously completed steps to rollback changes.

> **Design Rationale:** 
> - **Offline vs Pessimistic:** Not all operations make sense offline. Pessimistic locking (synchronous execution waiting for connection) remains the standard path. The fire-and-forget outbox is strictly an opt-in pattern for offline-capable operations.
> - **In-Memory vs Durable Execution:** While "Durable Execution" (saving the exact intermediate execution state to a DB, like Temporal) assists with idempotency, it is highly complex to implement within the Wasm host and still fails if there are strict time constraints between I/O steps. We instead trade platform complexity for explicit workflow definition—our in-memory approach requires that if a process crashes mid-task, the task is fully aborted and compensated (via `undo`) rather than resumed.
> - **Saga Arguments:** The compensating `undo` functions generally accept the identical arguments as the original forward operation (along with the generated resource ID) to precisely reverse the specific action.

### [PLT-RED] Service Redundancy
This feature guarantees data durability, service continuity, and split-brain prevention for the Syneroym network, strictly prioritizing Consistency over Availability (CP) during network partitions.

**Database Redundancy**
- **Configurable Stateful Replication:** The replication factor (e.g., N=1, N=2, N=3) is configurable at service deployment time via the application manifest. For replicated setups (N>=2), there is exactly one Primary accepting write operations, while the Secondaries maintain an identical read-only state. The Registry reflects the topology defined by the manifest.
- **Low-Latency Streaming:** Replication must be near-instantaneous. Changes committed to the Primary must be streamed directly to the Secondary without relying on high-latency batching or third-party storage intermediaries for the live replication path.
- **Disaster Recovery:** In addition to live node-to-node replication, the system must support periodic asynchronous backups to external object storage (S3-compatible) to enable cold starts and disaster recovery.

**Blob Storage Redundancy**
- Large files and blobs are not replicated via SQLite. Instead, the platform relies on external, configurable S3-compatible storage backends. The underlying S3 provider is responsible for ensuring the redundancy of these blobs.

**Registry & Topology Management**
- **Single Source of Truth:** The Registry Service is the authoritative control plane for all cluster membership and routing topology.
- **Manual Promotion (CP Focus):** To prevent split-brain scenarios, there is strictly no automatic failover. If a Primary fails, the system deliberately drops its redundancy level (Availability impact) to preserve data Consistency. Promoting a Secondary to Primary requires explicit operator intervention via the Registry.
- **Strict Quarantining:** When a failed node is deposed, the Registry must permanently mark its Node ID as `QUARANTINED`.
- **Routing-Level Fencing:** The system must enforce split-brain prevention at the routing layer. `QUARANTINED` nodes must be completely isolated:
    - *Ingress:* All other nodes and clients must clear their routing caches and immediately cease sending requests to the quarantined node.
    - *Egress:* Any outbound requests originating from a quarantined node (e.g., if it is merely network-partitioned) must be strictly rejected by all receiving services.

**Control Plane vs Data Plane Isolation**
- The Data Plane must be fully decoupled from the availability of the Control Plane. If the Registry service goes offline, all data plane routing, MQTT message flows, and HTTP access must continue functioning indefinitely using the last known cached topology until the control plane is restored.

> **Implementation Design:** For technical details regarding the Iroh-based WAL replication and routing-level fencing, see [Feature Design: PLT-RED](post-dd864a1-feature-design.md#plt-red-service-redundancy).

## Phase 3: Substrate & Application Lifecycle

### [LFC-MGT] SynApp Lifecycle Management
- Orchestrates the deployment, configuration, and monitoring of SynApps and their constituent SynSvcs across a decentralized network of substrates.
- **Application Manifests**: A SynApp is defined by a declarative manifest containing:
  - A list of required `SynSvc` instances (WASM components).
  - Explicit configurations for each service, including resource quotas and limits.
  - **Explicit Bindings**: First-class declarations of logical network dependencies (e.g., `requires: backend_api`). The orchestrator uses these to construct a dependency graph and resolve physical addresses *before* injecting them into the service's configuration.
- **Substrate Inventory**: The control plane maintains a user-defined inventory of target substrates, tracking their known capabilities to facilitate intelligent deployment scheduling. Target substrates enforce access control to ensure only authorized deployments are accepted.
- **Operational Modes**: Lifecycle management is supported via two distinct operational modes utilizing a shared set of core orchestration libraries:
  1. **CLI Standalone Mode (roymctl)**: Designed for **one-shot decentralized deployment**. The CLI reads a manifest, synchronously deploys services to available online substrates, and records an installation trace in a local SQLite database. It does not queue tasks for offline substrates. Drift from the desired state is resolved manually via a single-pass `reconcile` command.
  2. **Active Control Plane Mode (Server SynApp)**: A specialized SynApp deployed onto the network that acts as a continuous controller. It accepts manifests via an API, stores the desired state in its replicated SQLite database, and runs a continuous background reconciliation loop to watch the actual state of services and trigger deployments/undeployments automatically.

### [LFC-VER] Versioning support overall
- **Substrate Upgrades**: Substrate binaries are updated via conscious, operator-driven actions rather than automatic, unverified polling. The system must support rollback to previous binaries (e.g., via `roymctl revert`) in case an upgrade introduces instability or fails health checks.
- **SynApp/SynSvc Compatibility**: SynApp compatibility is based on WASM interface capabilities rather than rigid Substrate versions. While a SynApp manifest may list Substrate versions as advisory "tested-on" metadata (similar to browser compatibility), the Substrate ultimately decides to accept deployment based on whether it can satisfy the required WIT host interfaces.
- **Service Upgrades & Migrations**: SynSvcs (WASM components) require automatic, state-safe upgrades. The system must support automatic filesystem-level snapshotting of service state (SQLite) before an upgrade. If the new service version fails to initialize or migrate its schema, the Substrate must automatically rollback to the snapshot and the previous WASM binary.
- **Network Compatibility**: Multi-substrate communication relies on a dynamic Capabilities/Protocol Matrix. During the connection handshake, nodes negotiate their supported protocols (e.g., `["syneroym/rpc/v1", "syneroym/rpc/v2"]`). The core team deprecates older protocols deliberately on a case-by-case basis, avoiding the brittleness of a rigid sliding-window (N-x) policy.

## Phase 4: Advanced Services & Tooling

### [ADV-OBS] Observability enhancements
- Dashboard of Overall service/app, resource utilization, metering

### [ADV-AI] AI
- Ollama local
- rig-core agent with vector store (in sqlite?) and Mem0 for long term memory

## Phase 5: High-Level Applications (SynApps)

### [APP-CHT] Chat
- Agentic flow, agent-human or agent-agent chats, UI in chat context, trusted rooms

### [APP-LDG] Dynamic ledger network app
- Mutual credit ledger and chains, with continuous/periodic cyclic settlement based on settlement rules and multi-party-signing, tags with tag-hierarchies

### [APP-MKT] Marketplace App
- Listings, Intents, Offers, Transactions with various Payment types
- Discovery, scoring/reputation within trust network, 
- Special case provider: - Allow substrate lease with configured criteria, quotas, capabilities etc

## Phase 6: Edge Expansion

### [EDG-MOB] Mobile operation 
- Syneroym on Android/IOS
- Additional things like TPM 2.0 equivalent on mobile

---

## Substrate Feature Coverage Matrix
*(Ensuring core platform primitives are battle-tested across the application suite)*

| Substrate Capability | Primary App | How it is exercised |
| :--- | :--- | :--- |
| **Data Layer: Pub/Sub** | **Chat** | Real-time message delivery and typing presence. |
| **Data Layer: S3 Blobs** | **Marketplace** | Storing and serving high-res images/videos for listings. |
| **Data Layer: Content Addressed** | **Ledger** | Storing immutable blocks and transaction receipts. |
| **Offline Operation** | **Chat** | Outbox message queuing and syncing upon reconnection. |
| **AI (Agents/Vector Store)** | **Chat** | AI participants with long-term memory in group chats. |
| **Access Control (FDAE/Consent)** | **Chat** | Enforcing read/write rules for trusted rooms and AI delegation. |
| **Service Redundancy (Sharding)** | **Ledger** | High availability and partition tolerance for the credit network. |
| **Security (TPM 2.0)** | **Ledger** | Hardware-backed multi-party signing for high-value settlements. |
| **Versioning & Migrations** | **Ledger** | Upgrading complex, stateful settlement rules without downtime. |
| **Substrate Lease & Quotas** | **Marketplace** | Dynamically leasing external nodes to handle flash-sale traffic spikes. |
| **Observability (Metering)** | **Marketplace** | Tracking exact resource utilization to bill storefront owners. |
| **Service Config (Secrets)** | **Marketplace** | Dynamically pulling external API keys (shipping/fiat gateways) from the vault. |