# Milestone 1: Local App Model and Lifecycle

## Goal
Establish the fundamental execution boundary (`SynSvc`), the application grouping overlay (`SynApp`), and a shared orchestration planner.

## Requirement IDs
- `[TOP-PRM]` Core Primitives & Overlay
- `[TOP-ADR]` Service Addressing
- `[TOP-REG]` Registries (App & Endpoint)
- `[TOP-DSC]` Discovery Mechanisms
- `[LFC-MGT]` Standalone `roymctl` Deployment & Manifest parsing
- `[LFC-VER]` Manifest versioning

## Explicit Non-Goals
- Active Control Plane Mode (Server SynApp) - Deferred to M5.
- Multi-node dynamic routing updates and dynamic scaling (M5/M7).
- Security Sandbox enforcement, Runtime quotas (M2).
- Distributed Registry (M8).
- Database persistence for SynSvcs (M3).

## Dependency Gates
- Milestone 0 Contract and Decision Gate (Completed).
- Unresolved Decisions below must be answered before implementing the slices.

## Decision Register

### 1. Deployment Journal Storage & Crate Naming
**Context:** Storage engine for `roymctl`'s standalone trace and naming for the core orchestration library.
**Status:** Resolved. The M1 trace will use plain SQLite (encryption deferred to M3). This SQLite schema will be shared between the standalone `roymctl` CLI and the future Active Controller mode. The shared crate will be named `crates/app_orchestration` to show parallels with Kubernetes' application controllers. While it acts as a "compiler" to produce the immutable `DeploymentPlan`, it also encompasses the core state-transition logic, dependency graph resolution, and the diffing engine (`reconcile`). The actual effectful execution (e.g., launching WASM, I/O) remains outside this crate.

### 2. Manifest Catalog Sourcing
**Context:** What remote dependencies does the `ManifestCatalog` fetch, and should we support it in M1?
**Status:** Resolved. Remote dependencies would involve fetching external `SynApp` blueprints (manifests) and the actual `SynSvc` WASM binaries/container images over HTTP. To maintain a tight scope for M1, we will **defer remote fetching entirely**. For M1, the catalog will strictly rely on local filesystem paths for composing app dependencies and loading WASMs. Remote package retrieval over HTTP/OCI is explicitly assigned to **Milestone 5 (Developer Tooling)**, and has been added to the M5 definition in the meta-implementation plan.

### 3. Master Anchor Contract
**Context:** Are the Master Anchor requirements fully defined for Phase 0?
**Status:** Resolved. Yes, the high-level architecture is fully defined for Phase 0 implementation. It relies on a two-step `pkarr` DHT resolution: Registry -> Master Key DID -> DHT (Array of Temporary Keys) -> DHT (Endpoints). However, the exact byte-layout of the signed payload needs to be specified. To ensure we don't forget this, an explicit design task has been added to Slice 5 in this plan.


## Ordered Implementation Slices
1. **[x] Slice 1: Domain Models & Topology Definitions**
   - [x] Implement strong types: `AppBlueprintId`, `AppInstanceId`, `LogicalServiceName`, `ServiceId`, `LogicalServiceRef`, `InterfaceName`.
   - [x] Implement `DeploymentPlan`, `SynAppManifest` parsers (TOML/JSON).
2. **[x] Slice 2: Manifest Compiler (`crates/app_orchestration`)**
   - [x] Implement `ManifestCatalog` trait (restricted to local filesystem paths for M1).
   - [x] Implement topological sorting and dependency graph compilation.
   - [x] Implement Cycle Detection for `Spawn` vs `Bind` directives.
3. **[x] Slice 3: Addressing & Resolution Overlay**
   - [x] Implement the Logical Resolver (sitting above the physical router).
   - [x] Implement `StaticInventory` mode for the `AppRegistry`.
   - [x] Implement topology cache keyed by `AppInstanceId + LogicalServiceName` with invalidation logic.
   - [x] Implement rendering of deterministic rendezvous hashing (BLAKE3) for sharded selection topology.
4. **[x] Slice 4: `roymctl` Standalone Journaling**
   - [x] Implement local Deployment Journal (`PLANNED`, `APPLYING`, `ACTIVE`, `ROLLING_BACK`, `ROLLED_BACK`).
   - [x] Implement `roymctl reconcile` to diff against the journal and compute configuration/routing updates.
5. **[x] Slice 5: Master Anchor Contract & Baseline Migration**
   - [x] **Design:** Draft and document the exact byte-layout schema for the Master Key `pkarr` payload (Array of Temporary Keys).
   - [x] Implement Phase 0 Master Anchor resolution distinguishing Master Key from Temporary Key.
   - [x] Refactor existing `roymctl` and dispatcher code to consume the new `DeploymentPlan`.

## Migration Strategy
- Current CLI commands will be wrapped in a backwards-compatibility layer during transition, eventually deprecating direct single-WASM deployments in favor of single-service `SynApp` manifests.
- Existing DID-key endpoint registry will remain functional while the logical overlay is built on top.

## Tests & Runnable Reference Scenario
- **Scenario:** Deploy the initial "Professional Services Guild" foundational app (e.g. Identity and Echo services) using a standalone `roymctl` manifest.
- **Failure Test:** Verify cycle detection prevents deploying a manifest with circular `Spawn` dependencies.
- **Failure Test:** Verify `roymctl reconcile` recovers a mocked `APPLYING` state deployment.

## Performance Budgets
- **Compilation Time:** `DeploymentPlan` compilation must be < 50ms for a graph of 50 services.
- **Resolution Overhead:** Logical-to-Physical resolution cache hit must add < 100ns latency to the physical routing path.

## Measurable Exit Criteria
- `cargo +nightly fmt --all` passes.
- `cargo clippy --workspace --all-targets --all-features` passes.
- `cargo test --workspace` passes.
- `mise run test:e2e` passes including the new reference scenario.
- Relevant `wasm32-wasip2` compilation is unbroken.
- `roymctl` successfully compiles and deploys the "Professional Services Guild" foundational manifest locally.
