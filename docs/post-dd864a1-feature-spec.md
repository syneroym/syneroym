# Pending Features Master Spec
This document describes the pending features for Syneroym post git commit hash `dd864a18902bb8e71da0ff56bba4523688ad8ba1`. 

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
- Deployment of Open Registry, Relay on lightsail

### [FND-SEC] Security
- Encryption at rest with key negotiation with service owner.
- Ensuring correct fingerprint of syneroym binary itself with TPM 2.0 and others

### [FND-CFG] Service configuration
- Environment vars, Config, Secrets (dynamically pulled from registry/vault?)

### [FND-IAM] Access Control
- Various Casbin scenarios. At synapp level, synapp management level,
- Consent first data and service access with delegation
- Support multiple authentication/authorization schemes

## Phase 2: Core Platform Capabilities

### [PLT-DAT] Data Layer
- All types REST, Pub/Sub, S3 blobs, Content addressed?

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

### [LFC-LES] SynApp Substrate lease app
- Allow substrate lease with configured criteria, quotas, capabilities etc

### [LFC-MGT] SynApp Deployment Management App
- Deploy SynApp resistry, inventory on any substrate as another SynApp
- Track services expected vs actual status 

### [LFC-VER] Versioning support overall
- Substrate upgrades
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