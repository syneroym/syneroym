# Syneroym Meta-Implementation Plan

This document provides a high-level strategic roadmap for implementing the Post-DD864A1 architecture. It takes the pending items described across Phase 0 to Phase 7 and reorganizes them into logical, sequentially buildable **Milestones (Epics)**. 

There is no single "MVP" boundary or distinct "Pilot" launch. Instead, we build features phase by phase, treating every phase as its own incremental pilot that delivers immediate end-to-end value. This meta-plan avoids dependency cycles by slicing deep architectural features (like Security and Data) across multiple milestones. It emphasizes the "walking skeleton" principle by growing a reference SynApp (Professional Services Guild) through every milestone to prove out the infrastructure.

## Guiding Implementation Strategy
1. **Inside-Out Development:** We build the core local primitives first (routing, security, data) before expanding to multi-node federation and high-level applications.
2. **Continuous Walking Skeleton:** Do not defer product validation. Every milestone must expand a reference SynApp (focusing on the **Professional Services Guild**) to ensure we don't build isolated testing facades.
3. **Strict Boundaries:** No communication crossing a `SynSvc` trust boundary may bypass identity and authorization enforcement. Statically composed components are treated as one `SynSvc` boundary.
4. **Shared Orchestration:** Planning logic is not built independently inside `roymctl` and the Substrate. Instead, `crates/orchestration` acts as a pure manifest compiler producing an immutable `DeploymentPlan`. `roymctl` and the active controller act merely as effectful adapters around this shared planner.
5. **Continuous Observability & Tooling:** Observability instrumentation and developer tooling begin early and mature throughout the milestones.

## Standard Milestone Documentation Format
When we begin work on any milestone below, we will generate a dedicated `task.md` that strictly includes:
- **Requirement IDs** (e.g., `[TOP-PRM]`)
- **Explicit non-goals**
- **Dependency gates**
- **Migration impact**
- **A runnable reference scenario** and failure/security tests
- **Performance budgets**
- **Measurable exit criteria**, which must strictly enforce passing:
  - `cargo +nightly fmt --all`
  - `cargo clippy --workspace --all-targets --all-features`
  - `cargo test --workspace`
  - `mise run test:e2e`
  - Relevant `wasm32-wasip2` compilation
  - End-to-end reference-scenario and failure-recovery tests

---

## Milestone 0: Contract and Decision Gate
**Goal:** Formalize the traceability matrix and create a baseline API migration plan for the existing codebase before major structural changes begin.

**Implementation Approach:**
1. **Traceability Matrix:** Map every requirement and sub-requirement to its current implementation status, target milestone, and acceptance evidence.
2. **Decision Register:** Resolve only milestone-blocking ADRs (e.g., encryption implementation, DLN scope, manifest versioning boundaries). Maintain a decision register for non-blocking deferred questions (Milestones 9–10).
3. **DLN Scope Resolution:** Explicitly decide the target milestone for the Dynamic Ledger Network (DLN) and whether signed-interaction-receipts (and therefore robust `[P2P-REP]` reputation) are scheduled early or assigned to later milestones.
4. **SQLite Encryption ADR:** Build an ADR/feasibility prototype for the exact encrypted-SQLite mechanism to be used in M3.
5. **Baseline Migration Plan:** Draft the plan to migrate current `roymctl` contracts.

---

## Milestone 1: Local App Model and Lifecycle
**Goal:** Establish the fundamental execution boundary (`SynSvc`), the application grouping overlay (`SynApp`), and a shared orchestration planner.

**Feature Grouping:**
- `[TOP-PRM]` Core Primitives & Overlay
- `[TOP-ADR]` Service Addressing
- `[TOP-REG]` Registries (App & Endpoint)
- `[TOP-DSC]` Discovery Mechanisms
- `[LFC-MGT]` Standalone `roymctl` Deployment & Manifest parsing
- `[LFC-VER]` Manifest versioning

**Implementation Approach:**
1. **Baseline Migration:** Migrate the current CLI and dispatcher contracts to align with the new `SynApp` vs `SynSvc` terminology.
2. **Shared Orchestration:** Build the pure `DeploymentPlan` compiler in `crates/orchestration`.
3. **Topology Work:** 
   - Implement strongly typed IDs and logical references.
   - Build dependency graph compilation with cycle detection, explicitly differentiating `Spawn` vs `Bind`.
   - Create the Logical resolver that sits above the physical router.
   - Implement Static and Native registry modes with TTL/topology-epoch cache invalidation.
   - Implement the Phase 0 Master Anchor resolution contract.
4. **Deployment Journal:** Implement a crash-consistent local deployment journal for standalone deployments.

---

## Milestone 2: Reliable, Operable Node
**Goal:** Establish basic network transport robustness, node identity, and foundational deployment mechanics before introducing stateful persistence.

**Feature Grouping:**
- `[TOP-ROB]` Network & Connection Robustness
- `[FND-IDT]` Cryptographic Identity Primitives (Handshake authorization slice)
- `[FND-DEP]` Deployment/Operations
- `[FND-SEC]` Substrate Security (Runtime quotas, memory protection)

**Implementation Approach:**
1. **Robust Transport:** Implement automatic connection retries and idle timeouts. Ensure reactive eviction relies natively on Iroh’s connection pooling rather than a custom application-level cache.
2. **Identity Handshake:** Implement Master Key → Temporary Key delegation, handshake authorization, revocation, and the Master Anchor signed-record contract.
3. **Operational Baselines:** Integrate runtime quotas and memory protection bounds. Provide explicit support for native TLS and certificate lifecycle, official Docker images, cross-platform release pipelines, and deployed smoke tests.

---

## Milestone 3: Secure Stateful Services

To prevent dependency cycles and scope creep, the data layer and storage mechanisms are split into two sequential sub-milestones.

### Milestone 3A: Structured State and Security
**Goal:** Introduce the baseline SQLite data layer intimately paired with storage encryption and the secret vault.

**Feature Grouping:**
- `[PLT-DAT]` Data Layer (Structured SQLite DBs per service)
- `[FND-SEC]` Substrate Security (Storage encryption, Vault)
- `[FND-CFG]` Service Configuration Delivery

**Implementation Approach:**
1. **Encrypted Isolation:** Provision isolated, encrypted SQLite databases for each deployed `SynSvc` (based on the M0 prototype).
2. **Data Interface:** Implement schema initialization, CRUD/batch operations, structured filters and aggregations, pagination, concurrency architecture, and nested WIT serialization.
3. **Vault Integration:** Build the secret vault into the encrypted DB and implement `syneroym:vault/reveal`.
4. **Configuration Delivery:** Finalize the delivery mechanics (WASM host functions vs. Podman environment mapping).

### Milestone 3B: Objects and Events
**Goal:** Provide the remaining fundamental asynchronous data primitives.

**Feature Grouping:**
- `[PLT-DAT]` Blob S3 Integration & MQTT Broker

**Implementation Approach:**
1. **Blob Storage:** Implement the S3-compatible backend interface and signed HTTP object access.
2. **Event Broker:** Embed the MQTT pub/sub broker, implementing wildcard topics, retained messages, and real-time change notifications.

---

## Milestone 4: Typed Communication and Authorization
**Goal:** Bridge isolated services securely by introducing the Universal Proxy and layering the FDAE (Access Control) on top of the established Data Layer.

**Feature Grouping:**
- Universal Proxy / wRPC
- `[LFC-VER]` Protocol Negotiation
- `[FND-IAM]` Access Control (FDAE, UCAN context)

**Implementation Approach:**
1. **Universal Proxy:** Implement wRPC over Iroh QUIC to allow strongly typed cross-component calls, utilizing dynamic protocol negotiation.
2. **UCAN Context:** Extract and normalize UCAN scopes/claims upon request ingress.
3. **Local FDAE:** Implement the SQL Pushdown Sieve, compiling declarative policies into the SQLite engine.
4. **Federated FDAE:** Expand the pipeline to support cross-service parameter fetching via the Universal Proxy.

---

## Milestone 5: Async Lifecycle and Developer Experience
**Goal:** Handle background jobs, offline semantics, and continuous reconciliation, while finalizing developer toolchains.

**Feature Grouping:**
- `[PLT-ASY]` Asynchronous Operations
- `[LFC-MGT]` Active Control-Plane Mode
- `[LFC-VER]` Versioning Support (State snapshot/rollback)
- `[ADV-DEV]` SynApp Developer Tooling

**Implementation Approach:**
1. **Async Primitives:** Implement the Outbox queue, cron lease mechanisms, Dead Letter Queue (DLQ), long-running task restart rules, and compensating transactions (sagas).
2. **Active Controller:** Deploy the controller `SynApp` that continuously reconciles desired state. *(This is strictly a single-node controller initially).*
3. **Versioning:** Implement pre-upgrade SQLite snapshotting and automatic rollback mechanisms.
4. **Developer Tools:** Release the mock SDK, project templates, and the zero-drift `roymctl dev` local environment.

---

## Milestone 6: Initial Integrated Experience
**Goal:** Deliver the first cohesive product experience using the completed foundations, proving the value of the reference application.

**Feature Grouping:**
- Thin Syneroym Hub (Desktop/Web surface only)
- Professional Services Guild Application

**Implementation Approach:**
1. **Headless Native Shell:** Build the thin desktop/web UI shell that renders JSON Action Cards.
2. **Product Polish:** Finalize the `SynSvcs` necessary for the Professional Guild to operate end-to-end.
3. **Exclusions:** Native ledger/mutual credit and integrated escrow are explicitly excluded at this stage.

---

## Milestone 7: Resilience and Operability
**Goal:** Harden the system for production by adding high-availability database replication and deep observability.

**Feature Grouping:**
- `[PLT-RED]` Service Redundancy
- `[FND-SEC]` Encrypted Backups, Attestation & Supply-chain signing
- `[ADV-OBS]` Advanced Observability

**Implementation Approach:**
1. **SQLite Replication Feasibility:** Validate the SQLite-safe replication mechanism through a bounded prototype with correctness, crash-recovery, and performance exit criteria.
2. **Production Replication:** Implement live, reliable SQLite WAL replication across Substrate nodes based on the validated prototype.
3. **HA Upgrade:** Upgrade the M5 active controller's database to rely on replicated HA storage.
4. **Topology Control:** Implement Registry topology epochs, manual promotion workflows, and strict bidirectional quarantine fencing.
5. **Security Hardening:** Add Attestation API and verification flows, binary signature verification, and support for scheduled, encrypted remote backups (with tested restore paths).
6. **Metrics Pipeline:** Finalize data rollups and expose metrics via secured RPCs.

---

## Milestone 8: Federation and Trust
**Goal:** Expand the single-node capability into a robust, community-driven mesh network.

**Feature Grouping:**
- `[P2P-DSC]` Federated Tag-Routed Discovery
- `[P2P-REP]` Peer Reputation & Trust

**Implementation Approach:**
1. **Tag Routing:** Implement hierarchical tag routing to push discovery intents. Include discovery TTL, request deduplication, fan-out limits, authentication, and tag privacy/abuse protections.
2. **Reputation CRDT:** Implement `[P2P-REP]` only if M0 selects and schedules a mutually signed interaction-receipt mechanism (DLN); otherwise move `[P2P-REP]` to the deferred roadmap without weakening its cryptographic prerequisite.

---

## Milestone 9: Expansion Track - AI Concierge

Due to its complexity, the AI expansion track is subdivided into foundational and advanced stages.

### Milestone 9A: Agentic Foundations
**Goal:** Connect the node to local AI capabilities and manage tools.
**Implementation Approach:**
1. **Hardware Gating:** Implement hardware detection and strict model allow-lists.
2. **Inference Execution:** Integrate local LLM inference wrappers and implement remote inference fallback logic.
3. **Tool Directory:** Implement `sqlite-vec` indexing for the Ecosystem Directory and basic tool retrieval loops.

### Milestone 9B: Advanced Orchestration
**Goal:** Execute complex, autonomous workflows safely.
**Implementation Approach:**
1. **Concierge App:** Deploy the core Concierge `SynSvc`, integrating strict HITL (Human-in-the-Loop) consent, pause, and resume workflows for agentic execution.
2. **Loopcraft:** Deploy specialized sub-agents and verification loops via WASM to orchestrate complex reasoning.
3. **Observability:** Implement streaming progress telemetry back to the Hub UI.
4. **External Integration:** Implement the MCP headless gateway and support for agent-to-agent delegation.

---

## Milestone 10: Expansion Track - Mobile Edge
**Goal:** Bring full native substrate execution to mobile devices.

**Feature Grouping:**
- `[EDG-MOB]` Mobile Operation

**Implementation Approach:**
1. **Toolchain & Targets:** Establish cross-compilation for WASM runtime and substrate against iOS/Android targets.
2. **Mobile Lifecycle:** Integrate robustly with OS-level background task windows and push notification wakes to defer network responses.
3. **Secure Hardware:** Implement Android StrongBox and iOS Secure Enclave bindings for the WIT security interfaces.

---

## Explicitly Deferred Work & Future Product Phases
To strictly enforce focus and ensure achievable milestones, the following roadmap features are excluded from Milestones 1–10 and will be scheduled in subsequent roadmap iterations:
- **`[FND-IDT]` Extensions:** Master Key export/recovery, Tier-1 Fallback processing, and Method B Zero-Knowledge (ZK) plugin verification.
- **Phase 6 Product Expansion:** 
  - The Producer-Distributor Mesh application vertical.
  - Complete, rich Syneroym Hub UI surfaces.
  - Dedicated marketplace, aggregator, and facilitator `SynSvcs`.
- **Financial & Escrow Services:** Any native settlement, mutual credit ledger operations, and integrated transaction escrow dependent on the full Dynamic Ledger Network.

---

## Next Steps
Once you are ready to begin execution, we will trigger **Milestone 0: Contract and Decision Gate** to construct the traceability matrix and resolve the blocking ADRs.
