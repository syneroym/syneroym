# Traceability Matrix

This document maps the requirements from the System Requirements Specification (`system-requirements-spec.md`) and related documents to their target milestones, current implementation status, and acceptance evidence.

## Core Requirements

| Requirement ID | Description | Target Milestone | Status | Acceptance Evidence |
| --- | --- | --- | --- | --- |
| `[PRD-AUT]` | Provider identity, data, policy, and operator choice remain under provider control. | TBD | Pending | Delegation-revocation and operator-migration journeys. |
| `[PRD-CUX]` | Consumers complete the reference journey without understanding hosting or federation. | M6 | Pending | Moderated task-success test and accessibility audit. |
| `[PRD-FED]` | Independent implementations interoperate without a mandatory central data-plane or authority. | M8 | Pending | Two-node federation and bootstrap-outage tests. |
| `[PRD-OFF]` | Safe workflows remain intelligible and converge after disconnection; unsafe retries fail explicitly. | M5 | Pending | Fault-injection, idempotency, and state-model tests. |
| `[PRD-POR]` | Participants can export, verify, and restore in-scope identity-linked data through versioned open formats. | TBD | Pending | Clean-node export/import drill and cross-version fixtures. |
| `[PRD-TRU]` | Trust evidence is sourced, scoped, fresh, explainable, and correctable; uncertainty remains visible. | M8 | Pending | Trust-display, revocation, omission, and abuse cases. |
| `[PRD-OPS]` | A non-specialist can install or join, understand health, recover, update, and exit within the declared operating profile. | M2 | Pending | Timed onboarding and incident-recovery exercises. |
| `[PRD-EXT]` | Third-party SynApps can declare capabilities and pass public compatibility tests. | M5 | Pending | Package inspection and protocol conformance suite. |
| `[PRD-SAF]` | Consent, data lifecycle, moderation boundaries, and responsible parties are explicit throughout a transaction. | M4 | Pending | Policy-version, grant, report, dispute, and deletion scenarios. |

## Substrate Capabilities

| Requirement ID | Description | Target Milestone | Status | Acceptance Evidence |
| --- | --- | --- | --- | --- |
| Requirement ID | Description | Target Milestone | Status | Acceptance Evidence |
| --- | --- | --- | --- | --- |
| `[TOP-PRM]` | Core Primitives & Overlay | M1 | Complete | Domain models implemented in [models.rs](file:///Users/pari/gitSyneroym/syneroym/crates/app_orchestration/src/models.rs); compiled manifests deployable via [service.rs](file:///Users/pari/gitSyneroym/syneroym/crates/control_plane/src/service.rs). |
| `[TOP-ADR]` | Service Addressing | M1 | Complete | Logical service references, resolver topologies, and rendezvous hashing implemented in [resolver.rs](file:///Users/pari/gitSyneroym/syneroym/crates/app_orchestration/src/resolver.rs) and [models.rs](file:///Users/pari/gitSyneroym/syneroym/crates/app_orchestration/src/models.rs). |
| `[TOP-REG]` | Registries (App & Endpoint) | M1 | Complete | `AppRegistry` and `StaticInventory` implemented in [resolver.rs](file:///Users/pari/gitSyneroym/syneroym/crates/app_orchestration/src/resolver.rs); DHT registry Master Anchor endpoints implemented in [dht_registry.rs](file:///Users/pari/gitSyneroym/syneroym/crates/core/src/dht_registry.rs) and [registry.rs](file:///Users/pari/gitSyneroym/syneroym/crates/community_registry/src/registry.rs). |
| `[TOP-DSC]` | Discovery Mechanisms | M1 | Complete | Top-level `resolve_master_anchor` implementation resolving master keys to authorized temporary keys in [dht_registry.rs](file:///Users/pari/gitSyneroym/syneroym/crates/core/src/dht_registry.rs); logical resolver endpoint caching. |
| `[LFC-MGT]` | Standalone `roymctl` Deployment & Manifest parsing | M1 | Complete | Parse logic in [models.rs](file:///Users/pari/gitSyneroym/syneroym/crates/app_orchestration/src/models.rs), reconcile loops in [reconcile.rs](file:///Users/pari/gitSyneroym/syneroym/crates/app_orchestration/src/reconcile.rs) and sqlite journal storage in [journal.rs](file:///Users/pari/gitSyneroym/syneroym/crates/app_orchestration/src/journal.rs), CLI integration in [app.rs](file:///Users/pari/gitSyneroym/syneroym/apps/roymctl/src/commands/app.rs). |
| `[LFC-VER]` | Manifest versioning | M1 | Complete | Semver validation on manifests implemented in [models.rs](file:///Users/pari/gitSyneroym/syneroym/crates/app_orchestration/src/models.rs). |
