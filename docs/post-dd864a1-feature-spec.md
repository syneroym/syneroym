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
*   **State & Capabilities:** It owns its state and enforces capability-based security (Casbin policies) on all incoming requests.

#### `SynApp` (The Control Plane Overlay)
`SynApp` is removed as a runtime execution boundary and is redefined as a **Deployment Manifest and Control Plane Overlay**.
*   **Lifecycle Management:** Acts as a blueprint to deploy, update, and remove a cohesive graph of `SynSvcs` as a single unit.
*   **Capability Bootstrapping:** Orchestrates the initial injection of permissions (Casbin policies) that allow internal services within the app to communicate.
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

### [FND-IAM] Access Control
- **FDAE-Lite (Policy-to-SQL Compiler):** Adopts a focused subset of the [Federated Data-Aware Authorization Engine (FDAE)](authorization-engine-spec.md) vision. It utilizes a declarative, Zanzibar-style structured configuration (e.g., YAML/JSON) to map relationship chains, but strictly restricts data sources to **local SQL databases** (the service's own SQLite DB and the core substrate's SQLite DB). By intentionally dropping FDAE's complex cross-boundary WASM lookups (heterogeneous sources), execution is kept entirely within the hyper-fast database engine. The Substrate directly deserializes this configuration—avoiding custom parsers or ASTs—and dynamically compiles it into parameterized `WITH RECURSIVE` SQLite queries.
- **Solving the Data Fetching Problem (Pushdown Sieve):** By compiling ReBAC policies directly into SQL `WHERE EXISTS` clauses, the engine eliminates the "Data Fetching Problem". The SQLite engine performs massive-scale relationship filtering at the C-level, handing only authorized rows back to the WASM guest, eliminating the need to synchronize data to an external graph.
- **UCAN Integration (Normalized Claims, Capabilities, Scopes):** Access control is a robust synthesis of cryptographic capabilities and relational data state.
  - **Context Initialization:** When a request arrives, the gateway mathematically verifies the UCAN chain, normalizing external authentications (OIDC, DIDs, WebAuthn) into internal DIDs. It extracts the proven **claims**, **capabilities**, and **scopes** (e.g., capability to read a specific document, or act on behalf of a delegator).
  - **Relational Verification:** The SQL Compiler uses these normalized UCAN scopes and claims as the bound parameters (`?`) for its query. The SQL query then verifies if the structural ReBAC rules (e.g., "is the UCAN delegator actually in the management chain of the resource creator?") legally support the cryptographic capability claimed in the token.
- **The Extensible 3-Stage Pipeline (WASM + SQL):** While core ReBAC relationship data remains strictly in SQLite for maximum performance, the authorization pipeline supports custom WASM plugins to handle dynamic ABAC logic or explicit overrides:
  1. **Pre-Step (Context & UCAN Verification):** The substrate verifies the UCAN chain, extracting normalized claims, capabilities, and scopes into a secure execution context. A WASM interceptor can further mutate this context before SQL compilation.
  2. **SQL Execution (The Relational Sieve):** SQLite natively filters candidate rows based on the ReBAC policies and the UCAN context.
  3. **After-Step (ABAC & Override Filter):** An optional custom WASM function performs fine-grained, non-relational ABAC checks on the candidate rows. This empowers developers to enforce dynamic rules (e.g., external API billing checks, time/location fencing) or implement explicit `should_override` logic before the data is returned to the guest.

## Phase 2: Core Platform Capabilities

### [PLT-DAT] Data Layer
- All types REST, Pub/Sub, S3 blobs, Content addressed?
- Support nested record serialization deserialization in wasm calls. Currently only some basic types are supported.

### [PLT-OFF] Offline operation
- Outbox and periodic sync

### [PLT-RED] Service Redundancy
- Support shards, replicated stateless services, primary-secondary backups
- Blob storage with redundancy
- Service registry, and manual replacement of failed instances, and quarantining failed instances in case they come up again. Epoch based ownership techniques.
- Stress on CP trading off Availability
- Data plane continues even if control plane down.
- Backup and restore of stateful data for cold start cases
- Ensure redundancy is maintained on failure with additional replication

## Phase 3: Substrate & Application Lifecycle

### [LFC-MGT] SynApp Deployment Management App
- Deploy SynApp resistry, inventory on any substrate as another SynApp
- Track services expected vs actual status 

### [LFC-VER] Versioning support overall
- Substrate upgrades, auto-upgrade
- Synapp, SynSvc upgrades, migration

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
- Listings, Intents, Offers, Transactions, 
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
| **Access Control (Casbin/Consent)** | **Chat** | Enforcing read/write rules for trusted rooms and AI delegation. |
| **Service Redundancy (Sharding)** | **Ledger** | High availability and partition tolerance for the credit network. |
| **Security (TPM 2.0)** | **Ledger** | Hardware-backed multi-party signing for high-value settlements. |
| **Versioning & Migrations** | **Ledger** | Upgrading complex, stateful settlement rules without downtime. |
| **Substrate Lease & Quotas** | **Marketplace** | Dynamically leasing external nodes to handle flash-sale traffic spikes. |
| **Observability (Metering)** | **Marketplace** | Tracking exact resource utilization to bill storefront owners. |
| **Service Config (Secrets)** | **Marketplace** | Dynamically pulling external API keys (shipping/fiat gateways) from the vault. |