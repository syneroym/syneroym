# Syneroym Ecosystem Requirements Specification

The [thesis](../THESIS.md) states the core bet: a truly peer-to-peer foundation for group communication and trust, on which independent mini-apps (SynApps) — chat, marketplace, social, AI — plug in and work together as one experience, with no central server in the middle. No blockchains or cryptocurrency.

Our flagship experience and reference application is **Roym** — mini-apps sharing one identity, one contact list, one set of groups, one trust model. Its first vertical is the Professional Services Guild, built entirely on the Syneroym substrate.

**Status:** Draft product baseline

**Companion documents:** [Thesis](../THESIS.md) · [Vision](./VISION.md) · [Architecture](./system-architecture.md)

This document is the canonical statement of **who Syneroym serves, what outcomes
it must enable, and which constraints a conforming implementation must honour**.
It expands the vision of *autonomous SynApps cooperating over a common
technology substrate* into testable product and ecosystem requirements.

### Requirement conventions

- **Must** denotes a release or conformance requirement.
- **Should** denotes an important default that may be deferred with a documented
  product or operational reason.
- **May** denotes an optional capability.
- Product releases are vertical, end-to-end increments. Capability backlog
  phases are engineering sequencing bands and do not independently constitute a
  usable product release.
- Open questions are tracked as architecture decision records or design questions.
This requirements spec is structured as follows:

- Philosophy & Design Constraints
- Product Outcomes, Guardrails, and Release Scope
- Requirements Overview
- Personas
- Glossary / Terminology
- Common Requirements
- Trust Model
- Conceptual Model
- Substrate Functionality
- Shared Utilities and Services
- SynApp Specs:
    - Reference Application: Business, Professional, and Retail Services (covering Home Services and Small Retail)
- Target Designs (Addendum)

---

## Philosophy & Design Constraints

Providers and consumers cluster locally; global reach from one source is not required. Reach grows instead through federation across autonomous clusters — cooperation between independently owned peer clusters over shared protocols, not server federation; direct peer-to-peer connections need no intermediary server, while relays and coordinators provide fallback connectivity when direct paths fail. The system keeps the benefits and sheds the drawbacks of large platforms listed in [VISION.md](./VISION.md#background), and goes after the open problem stated in the [thesis](../THESIS.md): rich group activity at real scale, with no central server, where no participant's device needs to fully trust any other.

### Design Principles

The following principles guide design decisions throughout the system:

**Locality-first.** The system is optimised for scenarios where providers and consumers are geographically proximate. Federation between local clusters is important; undifferentiated global reach is not an initial goal.

**Progressive decentralisation.** A provider starts with a single device and no federation. Complexity is introduced incrementally as their needs grow. The system does not require full federation to be useful.

**Data sovereignty.** Authoritative private provider data lives on infrastructure
the provider controls or has explicitly chosen. Public listings and records the
provider deliberately shares may be cached or replicated under disclosed
retention rules. Syneroym-operated infrastructure receives no special right to
store or monetise provider data.

**Transparency over opaqueness.** Ranking, discovery, and reputation algorithms are either open source or provider-auditable. No hidden algorithmic black boxes determining outcomes for providers.

**Interoperability by convention.** SynApps cooperate through shared substrate primitives and open protocols. No SynApp requires a central coordinator to interoperate with another.

**Human-operable by default.** A provider must be able to start small, understand
the system's current state, recover from common failures, and leave without
specialist assistance. Decentralisation that only experts can operate does not
satisfy the product goal.

**User agency over ecosystem purity.** Self-hosting is always available, but
managed hosting, familiar payment rails, and lightweight consumer identities are
valid adoption paths when their trade-offs are explicit and exit remains
possible.

**End-to-end slices before platform breadth.** New substrate capabilities are
validated through a real provider-consumer workflow before adjacent generality
is added.

**Additive evolution.** Core contracts evolve without breaking existing deployments. Advanced capabilities — external payment rails, ledger primitives, AI assistance, hardware attestation, and government identity assurance — are Envisioned capabilities tracked in the deferred backlog (`docs/planning/deferred-backlog.md`). Each capability is sequenced to compose with shipped contracts without redesigning them.

---

## Product Outcomes, Guardrails, and Release Scope

### Product outcomes

Syneroym succeeds when it creates a credible third option between a large
centralised marketplace and running an isolated website or messaging account.
The product must deliver these outcomes:

1. **Provider autonomy without an operations burden.** A small provider or guild
   can publish, transact, retain its customer relationships, and change operators
   without rebuilding its digital business.
2. **A coherent consumer experience across independent operators.** A consumer
   can discover, assess, contact, agree with, and later return to providers
   without understanding substrates, relays, or federation.
3. **Cooperation without surrendering control.** Independent providers can share
   discovery, referrals, infrastructure, and composed workflows while retaining
   their own identity, data, policies, and right to exit.
4. **Useful operation under real local constraints.** Core workflows tolerate
   intermittent connectivity, modest hardware, and varying technical skill.
5. **An ecosystem developers can safely extend.** Stable contracts, conformance
   tests, capability negotiation, and transparent governance allow third parties
   to build interoperable SynApps without privileged access.


---

## Requirements Overview

High-level requirement highlights:

- Providers either self-host on commodity hardware or choose a managed operator,
  without requiring a cloud account or deep technical expertise.
- Providers federate with others to share infrastructure and improve resilience and discovery reach.
- Consumers discover and transact with providers through a unified experience regardless of which substrate hosts the provider.
- Consumers can participate with a lightweight device-bound identity; running a
  personal substrate is an optional upgrade path, not an entry requirement.
- The substrate provides shared contracts for identity, messaging, discovery,
  agreements, receipts, trust signals, and data portability. Payment execution
  remains pluggable and outside the system trust boundary.
- The system degrades gracefully under network partition — queuing, offline-first storage, and async workflows keep transactions progressing.
- All provider participants retain the ability to exit — migrating data and services to a different infrastructure provider or running independently.
- Ecosystem protocols are open, versioned, testable, and governed through a
  published process that does not privilege Syneroym-operated services.

---|---|---|
| **PRD-AUT** | Provider identity, data, policy, and operator choice remain under provider control. | Delegation-revocation and operator-migration journeys. |
| **PRD-CUX** | Consumers complete the reference journey without understanding hosting or federation. | Moderated task-success test and accessibility audit. |
| **PRD-FED** | Independent implementations interoperate without a mandatory central data-plane or authority. | Two-node federation and bootstrap-outage tests. |
| **PRD-OFF** | Safe workflows remain intelligible and converge after disconnection; unsafe retries fail explicitly. | Fault-injection, idempotency, and state-model tests. |
| **PRD-POR** | Participants can export, verify, and restore in-scope identity-linked data through versioned open formats. | Clean-node export/import drill and cross-version fixtures. |
| **PRD-TRU** | Trust evidence is sourced, scoped, fresh, explainable, and correctable; uncertainty remains visible. | Trust-display, revocation, omission, and abuse cases. |
| **PRD-OPS** | A non-specialist can install or join, understand health, recover, update, and exit within the declared operating profile. | Timed onboarding and incident-recovery exercises. |
| **PRD-EXT** | Third-party SynApps can declare capabilities and pass public compatibility tests. | Package inspection and protocol conformance suite. |
| **PRD-SAF** | Consent, data lifecycle, moderation boundaries, and responsible parties are explicit throughout a transaction. | Policy-version, grant, report, dispute, and deletion scenarios. |

### [GTW-IDT] Client Gateway Identity Modes & Session Authentication

The Client Gateway MUST provide configurable identity operating modes and session authentication for browser and external clients accessing substrate services.

- **Identity Operating Modes:** The client gateway MUST support three distinct operating modes (`crates/core/src/config/roles.rs:364`, `crates/client_gateway/src/gateway.rs:59`):
  - `Open`: Proxies client requests without authentication headers.
  - `Login`: Validates caller session tokens against an authentication service and optionally enforces an HTTP 401 connection gate (`connection_auth_gate`) when no valid session is presented (`crates/client_gateway/src/gateway.rs:115`).
  - `Fixed`: Injects a preconfigured person master DID (`fixed_identity_did`) and delegation certificate (`fixed_delegation`) onto all proxied requests (`crates/client_gateway/src/gateway.rs:117`).
- **Node Authentication Service:** The substrate MUST provide an authentication service issuing short-lived cryptographically signed session tokens via nonce challenge-response (`/challenge`, `/login`) for local and delegated keys (`crates/auth/src/service.rs:37`).
- **Session Cookie Ingress:** The client gateway and router MUST extract sessions from `syneroym_session` HTTP cookies or `Authorization` headers, verify token validity and expiration, and enforce revocation denylists (`crates/router/src/route_handler/http/auth.rs:55`, `crates/substrate/tests/gateway_session_e2e.rs:50`).

### [IDT-BAK] Encrypted Identity Backup & Clean-Node Restore Archive

The identity system MUST support exporting person master keys as versioned, authenticated encrypted backups and packaging them for clean-node disaster recovery.

- **Versioned Encrypted Master Key Backup:** Master identity keys MUST export as versioned `IdentityBackup` payloads (`IDENTITY_BACKUP_VERSION = 1`) encrypted under a 32-byte z-base-32 recovery key via HKDF-SHA256 and AES-256-GCM (`crates/identity/src/backup.rs:22`).
- **Cryptographic Binding:** The public person DID MUST be authenticated as additional authenticated data (AAD) in the AEAD cipher, preventing relabeling or tampering without decryption failure (`crates/identity/src/backup.rs:37`).
- **Sealed Disaster Recovery Archives:** Applications and operator tools (`roymctl roym backup`) MUST package application databases alongside the encrypted identity backup into sealed archives (`RoymArchive`), verifying complete data restoration on clean substrate nodes (`crates/roym_directory/src/app/backup.rs:25`, `apps/roymctl/src/commands/roym/backup.rs:37`, `crates/substrate/tests/roym_restore_e2e.rs:49`).

---

## Personas in the Syneroym Ecosystem

The following are key personas. A single person or organisation may play
multiple roles, but the product must make the active role and its powers clear.

| Persona | Primary job to be done | Adoption constraint |
|---|---|---|
| **Individual Service Provider** | Publish an offering, receive qualified local work, serve repeat customers, and retain business history. | Limited time and technical skill; intermittent connectivity. (Note: mobile phone substrate operation is an Envisioned capability; today substrates execute on desktop and server platforms, while phone users participate via web browsers or client gateways). |
| **Self-hosting Provider / Node Owner** | Run the provider's digital business without dependence on an aggregator. | Needs safe defaults, understandable health, backups, and recovery—not a miniature SRE role. |
| **Guild or Provider Aggregator** | Operate a directory-type SynOrg to aggregate provider listings, offer local discovery, and curate community trust signals. | Must earn trust without acquiring irrevocable control over provider identity, data, or hosting infrastructure. |
| **Infrastructure Provider** | Offer bounded compute, storage, and connectivity with auditable usage and responsibilities. | Needs isolation, quotas, abuse controls, and an explicit service agreement. |
| **Consumer** | Find a suitable provider, understand why they are trustworthy, agree terms, communicate, pay, and keep records. | Will not install infrastructure or learn federation concepts before receiving value. |
| **SynApp Developer** | Build once against stable contracts, test locally, distribute safely, and interoperate with other apps. | Needs concise contracts, compatibility signals, examples, and conformance tooling. |
| **SynApp Owner / Provider Manager** | Configure a provider or guild presence, policies, catalog, availability, and staff access. | Needs delegated permissions and an audit trail without access to unrelated provider data. |
| **Facilitator** | Offer an optional bounded service such as delivery, payment gateway, credential issuance, backup, or dispute handling. | Must disclose terms and authority; cannot become an implicit mandatory intermediary. |

---

## Glossary / Terminology

**Aggregator.** A directory-type SynOrg service that aggregates provider listings and credentials without hosting or controlling provider infrastructure (e.g. a guild directory or trade cooperative).

**App Developer.** A person or organisation that builds SynApps and publishes them for others to deploy. Does not necessarily host or operate any infrastructure.

**Bootstrap Server.** An envisioned centralized registry service coordinating active services and relays.
> **Envisioned.** Not built yet. Centralized bootstrap servers (`*.syneroym.xyz`) are not built. Today substrate nodes configure static parent coordinator relay URLs (`parent_coordinator.iroh.url`) and publish endpoints to local Community Registries and Mainline DHT.

**Consumer / General User.** Uses the Syneroym ecosystem to discover and purchase services or products, or to interact with other entities.

**Federation.** The process by which independent providers and infrastructure nodes interoperate to share discovery, reputation, and messaging capabilities without a central authority.

**Home Relay / Coordinator Relay.** A coordinator relay endpoint configured at the substrate node level (`parent_coordinator.iroh.url`) to provide fallback connectivity when direct peer-to-peer connections fail. Hosted services inherit the substrate node's relay connectivity rather than binding to individual relays.

**Infrastructure Provider.** A person or organisation that makes hardware or virtual infrastructure available for Service Providers to host applications on a leased basis.

**Operator.** The person or organisation responsible for administering a
Substrate or managed SynApp Instance. An Operator may also be a Provider,
Aggregator, or Infrastructure Provider, but those roles confer different duties.

**Node.** A physical or virtual machine running one Substrate instance. May run multiple SVC-Sandboxes.

**P2P.** Peer-to-peer. To denote direct interaction between 2 entities without any intermediate broker service.


**Provider.** Short for *Service Provider*. Provides a service to others — e.g. plumber, photographer, consultant. May self-host or publish listings through an Aggregator directory.

**Relay.** A service or coordinator node that provides encrypted transport fallback for substrates and services that cannot accept inbound connections directly (e.g. behind symmetric NAT or firewalls). Coordinates direct P2P connectivity where possible, and relays encrypted traffic when direct connections cannot be established.

**SynApp Instance / Catalog.** A named, provider-configured business context within a SynApp deployment (e.g. a plumber's catalog and booking service).

**SynApp Owner / Operator.** The person or provider responsible for configuring and operating a SynApp instance: catalog, branding, access control, and operational policies.

**Substrate (SYN-SUBSTRATE).** The core runtime layer on a NODE. Manages service deployment, lifecycle, discovery registration, messaging, and access control on behalf of the NODE-OWNER.

**Service Sandbox (SVC-SANDBOX).** The execution environment for a SYN-SVC. May be a Wasm runtime instance (`crates/sandbox_wasm`), a Podman container (`crates/sandbox_podman`), or a native host binary dispatch (`crates/app_host_native`). Provides isolation between services sharing a NODE.

**SynApp (SYN-APP, Syneroym Application).** A deployment manifest and control plane overlay that defines a cohesive graph of SYN-SVCs. It acts as a blueprint to deploy, update, and manage capabilities, quotas, and namespaces. It is not an execution boundary.

**SynApp Instance.** One deployment of a SynApp blueprint, with its own stable
instance identifier, namespace, bindings, policy grants, configuration, and
accounting context.

**SynApp Owner.** The provider who deploys a SynApp to provide services to their clients. Distinct from the Developer who develops it.

**Service Artifact.** A reusable, independently deployable unit of business logic. Packaged as a Wasm component (`wasm32-wasip2`) or native binary.

**Syneroym Service (SYN-SVC).** A running instance of a module, executing within a SVC-SANDBOX on a Node. It is the absolute foundational zero-trust execution primitive of the Syneroym Substrate. Managed and proxied by the Substrate.

**Verifiable Credential / Signed Record.** A cryptographically signed attestation issued by an entity (such as a provider, trade authority, or community directory) using canonical signed Roym record envelopes (`crates/signed_record/`). The consuming party decides which credential issuers they trust.
> **Envisioned.** Not built yet. Generic W3C VC 2.0 envelope schemas and SSI wallet integration. Today credentials use canonical JSON signed records with Ed25519 signatures.

**Vouching.** A trust mechanism where entities issue signed endorsements for other entities within their network, creating a verifiable web of trust.


---

## Ecosystem & Domain Model

The following diagram shows the high-level business entities in the Syneroym ecosystem and how they interact.

```mermaid
---
title: Syneroym Ecosystem Context
config:
    layout: elk
---
flowchart TD
    Consumer([Consumer])
    Provider([Provider])
    Aggregator([Aggregator: Directory SynOrg])
    InfraProv([Infrastructure Provider])
    
    ConsumerApp[Consumer App]
    SynApp[SynApp: Catalog / Services]
    SynSvc[SynSvc: Service Instance]
    Substrate[Syneroym Substrate]
    Node[Compute Node]

    Consumer -->|uses| ConsumerApp
    ConsumerApp -->|discovers & interacts via| Substrate
    
    Provider -->|manages| SynApp
    Aggregator -->|aggregates listings from| SynApp
    
    SynSvc -->|executes on| Substrate
    SynApp -->|orchestrates| SynSvc
    Substrate -->|deployed on| Node
    InfraProv -->|owns & operates| Node
```

*(Note: For the lower-level technical architecture diagram detailing service components, services, and sandboxes, please refer to the Architecture Design Document).*

---

## Common Requirements

These requirements apply across all business domains and SynApps.

### Infrastructure & Hosting

- Service Providers run business applications on supported commodity computers
  they control, or on infrastructure operated under an explicit agreement (`crates/identity/src/substrate.rs:48`), even
  when the host is behind NAT or a firewall (`crates/router/src/net_iroh.rs:94`).
- Infrastructure Providers make hardware (old PCs, cloud VMs, etc.) available for Service Providers to host applications or application components under explicit node agreements (`crates/identity/src/substrate.rs:48`).
  > **Envisioned.** Not built yet. Commercial leased hosting accounting and automated compute marketplaces. Today infrastructure nodes are claimed and authorized via ControllerAgreement and roymctl.
- Service Providers see a plain-language service status (`crates/coordinator_iroh/src/info_endpoint.rs:104`) and manage routine operations via web UI or CLI without requiring shell access (`crates/roym_web/ui/`).
  > **Envisioned.** Not built yet. Automated push notifications when provider intervention is required. Today operators inspect status via /v1/info endpoints and the Roym Hub web UI.
- Infrastructure Providers monitor infrastructure health and resource usage (`crates/observability/src/recorder.rs`), and control node access via Admin DIDs and UCAN delegation (`crates/control_plane/src/service/orchestration.rs:200`).
  > **Envisioned.** Not built yet. Incident-to-obligation impact mapping and automated SLA tracking during outages. Today health metrics are recorded in memory and node access is governed by UCAN delegation.
- App Developers package SynApps (e.g. as WASM modules or OCI images; `crates/app_orchestration/src/models/manifest.rs:16`), and Providers deploy them to matching container infrastructure (WASM runtime, Podman/Docker; `crates/sandbox_wasm/src/engine.rs:84`, `crates/sandbox_podman/src/engine.rs:238`).
- Consumers access Provider services through options the Provider makes available: app UI, browser, API, or command-line tools (`crates/roym_web/ui/`, `crates/client_gateway/src/gateway.rs:73`, `apps/roymctl/src/commands/`).
- Service Providers export and restore all in-scope data, configuration, grants,
  and signed history using documented, versioned formats (`ARCHIVE_VERSION = 1`). An
  export includes a manifest and completeness report; secrets transfer via
  encrypted archives (`crates/roym_directory/src/app/backup.rs:25`).
  > **Envisioned.** Not built yet. Cross-version migration test fixtures for exported data archives. Today same-version export and clean-node restore are verified by automated tests.
- Every production profile supports encrypted backup and tested restore. Backup
  destination is operator-selectable; peer backup pools are optional and must
  not be required for portability (`apps/roymctl/src/commands/roym/backup.rs:25`).
- Backup success is never inferred merely from upload success; restorability is tested on clean nodes in automated integration test suites (`crates/substrate/tests/roym_restore_e2e.rs:49`).
  > **Envisioned.** Not built yet. Automated reporting of recovery-point and recovery-time expectations before choosing a hosting profile. Today clean-node backup restore is verified in test harnesses.
- Multi-device clients may work offline using local caches and outboxes (`crates/async_queue/src/lib.rs:1`, `crates/substrate/tests/roym_group_offline_e2e.rs`). Running
  independent writable copies of the same authoritative service state is a later
  capability and must not be implied by the baseline requirement; single-writer serialization per service resolves this (`crates/data_db/src/sqlite/provider.rs:281`).
- Sharding and multi-node service placement are optional scale capabilities (`crates/app_supervisor/src/lib.rs`).
  They must not complicate the single-node deployment or portability contract (`crates/substrate/src/main.rs:178`).

### Connectivity & Offline Behaviour

- The substrate supports direct peer-to-peer connections without intermediary servers wherever direct paths exist (`crates/router/src/net_iroh.rs:94`), and falls back to relay-mediated encrypted connections through coordinators when NAT or firewall constraints prevent direct connectivity (`crates/coordinator_iroh/src/coordinator.rs:180`, `crates/router/src/route_handler/io.rs:471`).
- **Offline outbox and retry queue:** Operations explicitly declared safe for
  deferred delivery are stored durably (`crates/async_queue/src/queue.rs:50`), expose `pending`, `delivered`, or
  `failed` status to the user (`crates/conversation/src/store.rs:127`), and retry when connectivity returns. The UI must
  not present `pending` as final success.
- Automatic retries require an idempotency contract (ADR-0023 §4). The system must not replay
  a non-idempotent operation merely because a connection failed.
- Each transactional entity defines permitted state transitions, authority,
  expiry, idempotency, and conflict behaviour (`crates/roym_core/src/transaction.rs:86`). Reconnection either reaches the
  same valid final state for all parties or exposes a conflict requiring a named
  party's decision; silent last-write-wins is not acceptable for agreements,
  payments, fulfilment, or access grants. A single writer per service resolves
  this by replaying queued requests through per-entity arbitration rules — no
  multi-master merge is needed for these entities (`crates/data_db/src/sqlite/provider.rs:281`).
- Users can cancel a still-pending message or booking operation when doing so is safe (`crates/conversation/src/host_impl.rs:333`, `crates/roym_transaction/src/app/booking_ops.rs:358`), and can see
  when cancellation is no longer guaranteed because delivery may have occurred.
  > **Envisioned.** Not built yet. Generic user interface cancellation of arbitrary queued substrate outbox operations. Today message and booking cancellations are supported specifically.

### [WEB-PRX] Peer-Proxy Browser Fallback Tunneling

For browser clients lacking native QUIC or WebRTC direct peer connectivity, the system MUST provide a service worker fallback proxy and WebSocket blind tunnel to communicate with substrate nodes.

- **Bootstrap Assets & Service Worker:** WebRTC and gateway coordinators serve browser bootstrap assets (`peer-proxy.js`, `/sw.js`) that intercept outbound application network requests and proxy them over client WebSockets (`crates/coordinator_webrtc/src/bootstrap.rs:113`).
- **Opaque WebSocket Blind Tunneling:** Coordinators expose an opaque WebSocket blind tunnel endpoint (`/__syneroym/tunnel`) that reads the initial route preamble, resolves destination Iroh node endpoints via community registry lookups, and pipes raw binary frames bidirectionally between the browser Service Worker and target Iroh substrate nodes without inspecting decrypted payloads (`crates/coordinator_webrtc/src/bootstrap/tunnel.rs:11`).
- **Preamble and Endpoint Resolution:** The blind tunnel forwards the route preamble to the destination substrate node, maintaining full stream encapsulation and end-to-end transport encryption between the browser client and destination service (`crates/coordinator_webrtc/src/bootstrap/tunnel.rs:37`).

### Messaging & Data Sharing

- Providers, Consumers, and Services exchange messages only within an explicit
  conversation or capability context and subject to owner-approved access
  policy (`crates/conversation/src/lib.rs:56`, `crates/ucan/src/token.rs`).
- The system supports one-to-one text, attachments, and structured service cards
  for requests, quotes, agreements, status, receipts, and grants, as well as private group chat (`crates/roym_group`, `crates/conversation/src/dag.rs`).
  > **Envisioned.** Not built yet. Audio/video calls, social feeds, and collaborative editing. Today 1:1 messaging, structured cards, and group chat are supported.
- A structured message has a stable type, schema version, sender, intended
  recipients, creation time, idempotency identifier where applicable, and
  verification status (`crates/conversation/src/store.rs:116`). Clients render unknown types safely without executing
  arbitrary sender code.
- The product states which message content and metadata are end-to-end encrypted using `vodozemac` (X3DH and Double Ratchet) and owner-distributed epoch keys (`crates/conversation/src/crypto.rs:233`, `crates/conversation/src/dag.rs`), which operator can observe remaining metadata, and why. Transport encryption alone must not be described to users as end-to-end message privacy.
- Unsolicited contact is rate-limited and user-controlled. Recipients can block,
  report, and leave a conversation without surrendering their transaction
  records (`crates/roym_profile/src/app/safety_ops.rs:50`, `crates/roym_core/src/safety.rs:20`).
  > **Envisioned.** Not built yet. Sender-side visibility of inbound rate-limit refusal reasons. Today recipient nodes enforce local contact blocks and rate limits silently.

### Non-Functional Requirements

Unless superseded by a stricter vertical requirement, the reference client uses these
measurable baselines. Test profiles and measurement methods belong in the
Architecture and test plans.

- **Security:** All inter-node and client-node traffic is encrypted in transit (`crates/router/src/net_iroh.rs:94`). Sensitive production data and backups are encrypted at rest by default using SQLCipher KEK/DEK envelope encryption (`crates/data_keystore/src/key_store.rs:29`). No default credential is shared across installations. Critical actions are authenticated, authorised, and audit-recorded.
- **Identity security:** Routine key rotation and device loss do not require a new public identity. Revocation freshness, recovery authority, and the consequence of losing every recovery factor are shown to the owner (`crates/identity/src/delegation.rs:62`, `crates/core/src/dht_registry/master_anchor.rs:24`). Government identity is optional, never the universal root of participation.
- **Availability:** Local and cached reads remain available during temporary bootstrap or relay loss (`crates/data_db/src/sqlite/provider.rs:281`). Substrates configure static parent coordinator relay URLs (`parent_coordinator.iroh.url`) and publish to the community registry (`crates/community_registry`).
  > **Envisioned.** Not built yet. A 24-hour bootstrap outage test. Today substrates configure static parent coordinator relay URLs and publish to community registries.
- **Durability:** The disconnect/reconnect and process-restart suites lose no acknowledged in-scope transaction message (`crates/substrate/tests/saga_e2e.rs`, `crates/substrate/tests/roym_group_offline_e2e.rs`). Backup restore is verified on a clean node before initial release (`crates/substrate/tests/roym_restore_e2e.rs:49`).
- **Performance:** On documented standard-node profiles, local UI actions reach p95 under 1 second and remote browse, search, message acknowledgement, and request submission reach p95 under 3 seconds, excluding an offline peer or external payment provider (`crates/observability/src/recorder.rs:136`).
  > **Envisioned.** Not built yet. Mobile-network performance profile testing and automated mobile latency benchmarks. Today performance metrics are recorded in memory on desktop and server platforms.
- **Operability:** Health output identifies the affected user capability, likely cause, and safe next action (`crates/coordinator_iroh/src/info_endpoint.rs:104`). Schema migrations execute inside database transactions and roll back automatically on failure (`crates/data_db/src/sqlite/provider.rs:51`).
  > **Envisioned.** Not built yet. Automated binary update rollback across failed upgrades. Today schema migrations roll back within SQL transactions on failure.
- **Interoperability:** WIT interfaces define versioned semver contracts (`crates/wit_interfaces/wit/`), and the handshake rejects unsupported protocol versions. Unknown optional capabilities fail gracefully; incompatible mandatory capabilities are rejected before a workflow begins.
  > **Envisioned.** Not built yet. Published public protocol conformance test suites. Today interface compatibility is verified through Cargo test suites and WIT interface definitions.
- **Privacy:** The system minimises observable metadata, documents every operator-visible category (`crates/roym_core/src/policy.rs`), and provides purpose, retention, export, and deletion behaviour for personal data. Telemetry is local in-memory by default (`crates/observability/src/recorder.rs`).
- **Portability:** Export/import formats and identity-linked history are documented, versioned, and integrity-checked (`crates/roym_directory/src/app/backup.rs`, `apps/roymctl/src/commands/roym/backup.rs`).
  > **Envisioned.** Not built yet. Cross-version migration test fixtures for data archives. Today same-version export and clean-node restore are verified by automated tests.

### [TST-PRT] Dynamic Port Allocation Contract in Test Harnesses

Integration and end-to-end test harnesses MUST dynamically allocate network ports rather than relying on static or hardcoded port numbers, guaranteeing collision-free parallel test execution across test binaries.

- **Dynamic Port Reservation:** Test suites probe and allocate verified-free TCP and UDP ports below the OS ephemeral range (`18_000`–`32_768`) via `alloc_ports::<N>()` before spawning substrate nodes or gateway listeners (`crates/substrate/tests/common/mod.rs:109`).
- **Complete Listener Port Coverage:** For every spawned substrate node, test harnesses MUST allocate distinct ports for all bound network listeners, including the Iroh HTTP relay, community registry, client gateway, and QUIC transport endpoint (`crates/substrate/tests/common/node.rs:55`).
- **Ephemeral Port Enforcement:** Integration test suites MUST NOT use port literals in the OS ephemeral range (`32_768`–`60_999`), verified by static test syntax inspection (`crates/substrate/tests/no_ephemeral_port_literals.rs:1`).

---

## User Experience, Agency, and Accountability

### Onboarding and recovery

- A provider can choose a managed-guild path or a self-hosted path. Providers initialize self-hosted substrates via `roymctl substrate init` or bind to managed controllers via `ControllerAgreement` (`apps/roymctl/src/commands/substrate.rs:65`).
  > **Envisioned.** Not built yet. A guided onboarding wizard comparing control, cost, availability, privacy, and support trade-offs before committing. Today substrate initialization and management run via roymctl CLI commands.
- Joining a guild must not transfer ownership of the provider's root identity, signed history, or export rights to the guild. Delegated administration is scoped (`crates/identity/src/delegation.rs:25`), visible, revocable via Master Anchor deny lists, and audit-recorded via FDAE `DecisionTrace`.
- Consumer onboarding creates or imports a lightweight device-bound Ed25519 identity on the consumer's device mapped to short-lived session tokens (`crates/client_gateway/src/session.rs`). Users export encrypted identity backups (`crates/identity/src/backup.rs`). The recovery model does not claim self-sovereignty if an operator can unilaterally recover or impersonate the consumer; root keys are self-sovereign Ed25519 master DIDs.
  > **Envisioned.** Not built yet. Automated post-transaction backup prompts in the UI and optional government identity assurance credentials. Today consumers manage device-bound session keys and export encrypted backups on demand.
- Destructive actions state their scope and recovery consequences. Common recovery flows are available through an expert CLI (`roymctl roym backup restore`) with warnings on destructive actions.
  > **Envisioned.** Not built yet. A guided graphical UI recovery wizard for identity and database restore. Today disaster recovery flows execute through the roymctl CLI.

### Data rights and lifecycle

- Every durable record has a documented owner or controller, permitted writers, retention policy, export representation, and deletion or tombstone behaviour. Permissions are enforced by FDAE policies (`crates/fdae/src/policy.rs`), directory publications enforce retention pruning (`crates/roym_directory/src/app/publication_ops.rs`), and deleted messages leave tombstones in the conversation DAG (`crates/conversation/src/host_impl.rs:333`).
- Shared records such as agreements, receipts, and revocations cannot be unilaterally rewritten; they are tamper-proof canonical signed Roym records (`crates/signed_record/`). A participant may remove its local copy or personal presentation where law permits, while the protocol preserves the other party's legitimate signed record and records later corrections separately.
  > **Envisioned.** Not built yet. Public consumer reviews, reputation ratings, and generic W3C Verifiable Credentials 2.0 envelopes. Today shared records use canonical signed Roym records and bilateral agreement receipts.
- Access grants are purpose- and scope-limited, expire by default for sensitive data, and can be revoked via UCAN tokens (`crates/ucan/src/token.rs`), delegation certificates (`crates/identity/src/delegation.rs`), and Master Anchor deny lists (`crates/core/src/dht_registry/master_anchor.rs`).
  > **Envisioned.** Not built yet. A graphical UI dashboard displaying active access grants and explaining what revocation can and cannot retract from already-received data. Today capability grants and revocations are enforced cryptographically via UCAN tokens and Master Anchor deny lists.
- Export and account deletion are distinct actions. Deletion identifies data held by the provider, operator, backup destination, peers, and legally required records; the product must not promise deletion it cannot enforce. Users export data via sealed backup archives (`roymctl roym backup create`), while message deletion purges readable bodies locally with DAG tombstones (`crates/roym_profile/src/app/profile_ops.rs:25`).
  > **Envisioned.** Not built yet. Comprehensive multi-party account deletion coordinating erasure across providers, hosting operators, backup destinations, and peer nodes. Today users export data via backup archives and delete local message content via tombstones.

### Safety, support, and disputes

- Before production use, the system has a reviewed threat model and privacy data inventory covering malicious packages, peers, clients and operators; compromised keys; metadata leakage; denial of service; backup exposure; and recovery abuse. Residual risks and unsupported deployment profiles are stated.
  > **Envisioned.** Not built yet. Formal reviewed threat model documents and privacy data inventories. Today technical protections include Wasmtime fuel limits, mlock memory protection, TLS 1.3 QUIC transports, and Double Ratchet messaging encryption.
- Syneroym supplies evidence and workflow primitives; it does not imply that every listed provider, guild, credential issuer, or facilitator has been vetted by the Syneroym project.
- Provider terms, price, cancellation rules, data use, payment method, and named dispute path are captured before agreement in `AgreedTerms` (`crates/roym_core/src/transaction.rs:116`). A material change requires renewed consent and produces a new version.
- Users can report impersonation, fraud, harassment, unsafe service, and illegal content to the relevant operator or community. Reports have a status, an appeal or correction path where appropriate, and safeguards against publicising unverified allegations as fact. Reports are stored locally on the submitting node with status tracking (`crates/roym_profile/src/app/moderation.rs:188`).
  > **Envisioned.** Not built yet. Community-level appeal, dispute arbitration, and public correction workflows. Today moderation reports are stored locally with status tracked on the submitting node.
- Emergency response, guaranteed refunds, insurance, professional licensing, and legal arbitration are not implied platform services. A guild or facilitator offering them must state jurisdiction, limits, and responsible legal entity in `AgreedTerms`.
  > **Envisioned.** Not built yet. Decentralized dispute resolution, independent arbiter panels, and smart-contract escrow custody. Today agreements capture a named dispute path as a text policy term within AgreedTerms.

---

## Ecosystem Contracts and Governance

- The minimum federation contract consists of versioned identity resolution, endpoint discovery, provider and catalog publication, service requests, agreements, receipts, trust signals, and backup archive export schemas (`crates/roym_directory/src/app/backup.rs`).
  > **Envisioned.** Not built yet. Dynamic protocol capability negotiation and cross-vendor federation testing suites. Today protocols bind fixed ALPN `syneroym/0.1` and typed WIT interface packages.
- Normative contracts have stable identifiers in versioned WIT interface packages (`crates/wit_interfaces/wit/`) and dual-build parity tests.
  > **Envisioned.** Not built yet. Standalone third-party test vectors and an executable public conformance test suite. Today compatibility is verified via cargo test suites and dual-build parity tests.
- Architectural decisions and protocol modifications are documented in Architecture Decision Records (ADRs) with rationale, security impacts, and migration plans.
  > **Envisioned.** Not built yet. A formal external public proposal process with scheduled community review periods. Today architectural changes are proposed and tracked through internal ADRs.
- No public Syneroym-operated bootstrap, relay, registry, app store, model service, or certificate authority is the sole permitted implementation of its role. Substrates configure custom parent coordinator relay URLs (`parent_coordinator.iroh.url`), publish to self-hosted community registries, use self-sovereign Ed25519 keys, and support complete local data export.
- Compatibility claims are capability-specific, enforced by versioned WIT packages and manifest interface requirements.
  > **Envisioned.** Not built yet. A formal third-party capability certification and labeling program. Today compatibility is verified against versioned WIT package definitions.
- SynApp manifests declare publisher, interfaces, dependencies, resource bounds, FDAE policies, and lifecycle hooks; deployment requires owner UCAN authorization.
  > **Envisioned.** Not built yet. Standalone publisher-signed package distribution archives and interactive UI prompts for capability expansion during updates. Today SynApp components deploy via manifests and roymctl.
- SynOrgs operate as single-owner root entities managing membership credentials, directory listings, and revocation lists (`crates/roym_directory/src/app.rs`).
  > **Envisioned.** Not built yet. Multi-signature voting, token governance, and weighted community consensus. Today SynOrgs operate under single-owner cryptographic controller authority.
- The project's sustainable business model may charge for hosting, support, certification, or optional services, but protocol participation and data exit cannot depend on paying a mandatory Syneroym toll. Substrate runtime execution, P2P networking, and SQLite storage execute locally with zero licensing checks or network tolls.
- The project publishes a private vulnerability-reporting channel via GitHub Security Advisories (`SECURITY.md`), and cryptographic key revocations publish to Master Anchor deny lists.
  > **Envisioned.** Not built yet. Published supported-version policies, standardized security advisory formats, package-level revocations, and automated emergency update procedures. Today vulnerabilities are reported via GitHub Security Advisories and keys are revoked via Master Anchor deny lists.

### [DIR-SYN] SynOrg Directory Credential Management & Pinned Sources

A SynOrg directory service MUST manage signed membership credentials, suspensions, and revocations for its community members, providing authoritative standing verification over public wire protocols.

- **Credential and Revocation Lifecycle:** The SynOrg host issues canonical signed `MembershipCredential` records carrying subject DIDs, authorized categories, and expiration timestamps (`credential.issue`), and publishes signed revocation records (`revocation.issue`) when membership terminates or is suspended. Stored credentials and revocations MUST be queryable via wire-exposed RPC methods (`directory.standing`, `directory.info`).
- **Publication Admission Gating:** A directory MUST admit service publications (`directory.publish`) only from verified members holding an unexpired, unrevoked membership credential issued by the directory's owning SynOrg.
- **Pinned Directory Sources:** Consumer and provider nodes MUST maintain an explicit list of trusted directory sources (`SourceRow`), pinning the directory DID and the issuing SynOrg DID on first contact (`issuer_did`) to prevent directory spoofing. Client discovery queries MUST fan out strictly across configured sources and verify returned listing credentials against pinned directory issuers.

---

## Trust Model

Centralized platforms derive consumer trust from brand, legal accountability, and aggregated reviews. A truly peer-to-peer system — where no participant's device needs to fully trust any other — needs explicit mechanisms to establish equivalent trust without central authority.

### The Trust Problem

When a consumer discovers a provider through Syneroym, they have no prior relationship with either the provider or the infrastructure operator. The system gives the consumer sufficient signal to decide whether to transact. Conversely, providers have signal that consumers are not fraudulent.

Minimum requirement at transaction time:

- Before accepting an agreement, the consumer can inspect the provider's stable identity, the provenance and freshness of available trust signals, material policy terms in `AgreedTerms`, payment recipient, and dispute or cancellation path. Absence of a trust signal is shown as unknown, never converted into a positive default.
- The product avoids collecting stronger identity than risk warrants; consumers use lightweight device-bound Ed25519 session keys, and providers configure recipient contact rate limits and block lists (`crates/roym_core/src/safety.rs`).
  > **Envisioned.** Not built yet. Dynamic provider configuration of consumer-side gates (mandatory deposits, required prior receipts, verified external contacts, or named facilitators). Today consumers use lightweight session identities with recipient-configurable rate limits and block lists.
- Trust displays separate facts (credentials, completed interaction receipts, recency) rather than hiding them behind one universal score. Directory search ranking uses open, explainable deterministic recency and round-robin merging (`crates/roym_directory/src/app/client_merge.rs`).
  > **Envisioned.** Not built yet. Joint DHT reputation records, exponential moving average (EMA) score formulas, and consumer vouch graphs are unbuilt. Today trust relies on independent credentials and bilateral receipts without numerical reputation scores.

### Trust Layers

Trust in the Syneroym ecosystem operates at multiple levels:

**Layer 1: Cryptographic continuity.** A stable, issuer-neutral root identity (self-sovereign Ed25519 master DID) delegates authority to rotatable device and routing keys using signed `DelegationCertificate` credentials. Key revocations are published to Master Anchor deny lists on the DHT and community registry. Cryptographic continuity proves control of keys—not a person's legal name, quality, or honesty.

> **Envisioned.** Not built yet. Hardware protection (TPM or secure enclave), social recovery, and government identity credentials are unbuilt optional assurance mechanisms. Today root keys are self-sovereign Ed25519 keypairs delegating via software delegation certificates.

**Layer 2: Referral and vouching.** Guild entities issue signed, scoped, expiring statements about members (`MembershipCredential`). The display preserves who said what and in which context; an issued credential is not silently treated as objective verification of quality.

> **Envisioned.** Not built yet. Consumer referral vouches, recommendation statements, web-of-trust vouching graphs, and vouch decay formulas are unbuilt. Today trust relies on signed SynOrg membership credentials and revocations.

**Layer 3: Verifiable credentials.** Providers attach credentials (such as guild membership or trade certification) structured as canonical signed Roym records (`crates/signed_record/`). Verification checks Ed25519 signatures, issuer DID, scope, expiry timestamps, and issuer revocation lists (`crates/roym_directory/src/app/credential_ops.rs`). The consuming party or community decides which issuers it trusts; the UI does not reduce "valid signature" to "trusted claim".

> **Envisioned.** Not built yet. Generic W3C Verifiable Credentials 2.0 envelopes and external credential library integrations are unbuilt. Today all credentials use canonical signed Roym records.

**Layer 4: Interaction receipts and feedback.** Completed commercial interactions produce bilateral independent agreement receipts (`AgreementReceiptPayload`) and fulfilment receipts (`FulfilmentRecord`) signed and stored separately by each party. A receipt proves that both parties acknowledged a workflow event under identical agreed terms, not that every off-system claim is true. Export bundles preserve provenance with signed record manifests.

> **Envisioned.** Not built yet. Separately signed consumer feedback, public reviews, and selective disclosure (redactable zero-knowledge or field-level disclosure) are unbuilt. Today interactions produce bilateral independent agreement and fulfilment receipts.

**Layer 5: Community moderation.** Guilds and communities maintain signed membership revocation and suspension decisions (`crates/roym_core/src/membership.rs`). Abuse reports submitted by users remain scoped locally to recipient nodes (`crates/roym_profile/src/app/moderation.rs`) and do not broadcast as global truth. Consumers see the policy source.

> **Envisioned.** Not built yet. Published community warning lists and formal automated correction or appeal workflows are unbuilt. Today guilds publish signed revocation lists and nodes store local abuse reports.

Trust mechanisms enforce anti-replay protections via monotonic sequences and timestamps (`crates/signed_record/`), key compromise revocation via Master Anchor deny lists (`crates/core/src/dht_registry/master_anchor.rs`), and tying receipts directly to signed agreements. Tying receipts to signed interactions reduces casual spam but does not by itself solve Sybil attacks or collusion.

> **Envisioned.** Not built yet. Comprehensive Sybil attack mitigation, collusion defenses, and formal evaluation frameworks are unbuilt. Today anti-replay timestamps and Master Anchor key revocations protect against basic replay and compromised keys.

### Legal Liability Boundary

The system does not provide legal shielding in the way centralised platforms do — that shielding derives from the platform's legal personhood and terms of service. Syneroym infrastructure operators bear their own legal responsibility for services they host under applicable local law. The requirements are:

- The substrate makes it straightforward for a Provider or Aggregator (directory-type SynOrg) to display their own terms of service, cancellation policies, and refund rules to consumers through `AgreedTerms` structures.
- The substrate does not create an implicit representation to consumers that a federated node has been vetted by Syneroym.
- A separate document will outline recommended legal structures for Directory SynOrgs and Aggregators operating at scale. [Legal guidance: Out of scope for this spec]

---

## Conceptual Model

The following ER diagram shows the formal entity model for the Syneroym
ecosystem, with full relationship cardinalities. See the
[Glossary](#glossary--terminology) for definitions of all entities. See the
[Ecosystem & Domain Model](#ecosystem--domain-model) diagram for a higher-level
overview.

```mermaid
---
title: Syneroym Conceptual Model
config:
    layout: elk
---
erDiagram
    direction TB

    %% --- Module & Service structure ---
    SERVICE-ARTIFACT ||--o{ SERVICE : instantiates
    SERVICE }o--o{ SERVICE : invokes
    SYNAPP ||--|{ SERVICE : comprises-of
    SERVICE }|--|| SVC-SB : runs-in
    NODE ||--o{ SVC-SB : runs
    SUBSTRATE ||--|| NODE : runs-on
    SUBSTRATE ||--o{ SERVICE : manages-and-proxies

    %% --- Ownership & Placement ---
    PROVIDER ||--o{ SUBSTRATE : owns
    SYNAPP-OWNER ||--|{ SYNAPP : owns
    SYNAPP }o--|{ SUBSTRATE : placed-on

    %% --- Connectivity ---
    SUBSTRATE }o--o| RELAY : uses
    RELAY }o--|| BOOTSTRAP : registers-with

    %% --- Application & Commercial entities ---
    PROVIDER ||--o{ CATALOG : manages
    CATALOG }o--|| SYNAPP : runs-within
    AGGREGATOR ||--o{ CATALOG : aggregates-listings-from

    %% --- Consumer side ---
    CONSUMER ||--o{ CONSUMER-APP : uses
    CONSUMER-APP }o--|{ SUBSTRATE : connects-to
    CONSUMER ||--o{ PROVIDER : transacts-with

    %% --- Trust & Credentials ---
    PROVIDER }o--o{ VERIFIABLE-CRED : holds
    CONSUMER }o--o{ PROVIDER : vouches-for

    SERVICE-ARTIFACT[SERVICE-ARTIFACT]{}
    SERVICE[SERVICE]{}
    SYNAPP[SYN-APP]{}
    SYNAPP-OWNER[SYNAPP-OWNER]{}
    SUBSTRATE[SYN-SUBSTRATE]{}
    SVC-SB[SVC-SANDBOX]{}
    NODE[NODE]{}
    RELAY[COORDINATOR-RELAY]{}
    BOOTSTRAP[BOOTSTRAP-SERVER]{}
    PROVIDER[PROVIDER]{}
    AGGREGATOR[DIRECTORY-SYNORG]{}
    CATALOG[CATALOG-OR-LISTING]{}
    CONSUMER[CONSUMER]{}
    CONSUMER-APP[CONSUMER-APP]{}
    VERIFIABLE-CRED[SIGNED-ROYM-RECORD]{}
```

A SynApp is placed on one or more substrates. Each service within a SynApp defines its own execution artifact (`ServiceDefinition::source`, such as a WebAssembly component or container image) and runs inside a sandbox or native host runner. Stale entity concepts (`SYN-MOD`, `Space`, and `Space Manager`) do not exist in code: services manage catalogs and listings directly, and providers transact with consumers via structured quote and booking workflows.

An Aggregator is a directory-type SynOrg service (`crates/roym_directory/src/app.rs`) aggregating provider listings and credentials per Owner Decision Q-A4. Aggregators do not manage infrastructure or host services for providers. Node operators claim substrate ownership through a mutually signed `ControllerAgreement`.

Substrates connect to configured coordinator relays (`parent_coordinator.iroh.url`) and publish signed endpoint records to the Community Registry and Mainline DHT. Trust evidence uses canonical signed Roym records (such as guild membership credentials and bilateral agreement receipts) rather than centralized reputation scores or generic W3C VC 2.0 envelopes.

> **Envisioned.** Not built yet. Centralized bootstrap servers, dynamic relay DNS assignment, and consumer-to-provider vouching graphs. Today nodes configure static coordinator relays and trust relies on signed SynOrg membership credentials.


---

## Substrate Functionality

Description of the core Syneroym substrate functionality, key protocols, and important flows.

### Substrate Setup

- Node owner installs and initializes the substrate on a node (`roymctl substrate init`).
- Substrate creates a protected node key on first run. Administrative ownership requires an offline claim step producing a mutually signed `ControllerAgreement`. Routine administration uses revocable delegated credentials (`DelegationCertificate`, UCAN tokens) rather than exposing a root key.
  > **Envisioned.** Not built yet. Tested recovery method wizard before production use. Today initial node keys are initialized at boot and claimed offline.
- Substrate connects to a Relay:
  - Substrate configures a static coordinator Iroh relay URL (`parent_coordinator.iroh.url`).
    > **Envisioned.** Not built yet. Dynamic home relay assignment via bootstrap service.
  - Publishes its signed endpoint information (`SignedEndpointInfo`) containing its public key and relay routing details to the Community Registry and Mainline DHT via pkarr (used for the control plane and service deployment).
  - Starts a secure communication server listening via the assigned coordinator relay and direct peer-to-peer interfaces (Iroh QUIC).
- Substrate identifies its capabilities (sandbox types, quota configurability). Node owner configures capability limits (CPU, memory, instruction fuel) available to hosted services.
  > **Envisioned.** Not built yet. GPU allocation and dynamic disk quota enforcement.
- Access control setup:
  - Substrate access is granted to the controller's master DID via `ControllerAgreement`.
  - SynApp owner master DIDs sign UCAN capability tokens granting deployment, removal, and observation access (`orchestrator/{deploy,undeploy,status}`) with associated quotas.
  > **Envisioned.** Not built yet. Automated multi-substrate peering registration protocol.

### [OPS-CLM] Substrate Node Claiming & Ownership Binding

Substrate nodes boot unowned and fail closed on privileged operations until an operator claims the node via `roymctl substrate claim`, establishing cryptographic controller authority.

- **Initial Unowned State:** A newly initialized substrate generates a local Ed25519 node keypair (`did:key:...`). Until claimed, the node rejects remote administrative operations (`substrate/admin`) over the wire to prevent unauthorized adoption.
- **Mutual Controller Agreement:** An operator claims the node by executing `roymctl substrate claim`. This mints a `ControllerAgreement` record signed by both the node private key and the controller master key, binding the node DID to the controller DID.
- **Fail-Closed Authorization:** The substrate router verifies the controller DID on all privileged interfaces (including deployment, service orchestration, and security administration). Requests without a matching controller signature or valid delegated UCAN capability are rejected.

### Substrate Managing Services

- Substrate provides a secure end-to-end communication channel between clients and the services it manages via the Universal Proxy and Client Gateway.
- Substrate supports WASM (Wasmtime) and Podman sandbox environments at minimum.
- Substrate attempts direct client-service communication wherever possible; it falls back to external coordinator relay when intermediate network infrastructure does not permit direct connections.
- On mobile platforms:
  > **Envisioned.** Not built yet. Mobile platform background execution, OS throttling handlers, and push notification dispatch. Today offline operations rely on substrate SQLite durable outbox queuing and idempotent replay.

### Core Substrate Services

**Messaging.** The substrate supplies secure, typed, durable delivery primitives via an embedded MQTT broker (`crates/mqtt_broker`) and Double Ratchet end-to-end encrypted conversation primitives (`crates/conversation/`, ADR-0013). Conversation products such as chat and groups are SynApps built on those primitives.
> **Envisioned.** Not built yet. Social feeds and collaborative editing SynApps.

**Discovery.** The substrate exposes capability and endpoint discovery via Community Registry and pkarr Mainline DHT. Guild directories, referrals, and ranking are replaceable SynApps or services (`crates/roym_directory`) implementing common publication and query contracts with client-side merging; no one global index is required.

**Identity.** The substrate manages protected key storage (SQLCipher and `mlock` KEK protection), rotation, revocation, and delegation while separating stable master DIDs from ephemeral routing keys (ADR-0020). Master Anchor DHT records publish cryptographic revocation deny lists. Credentials use canonical signed Roym records.

**Access Control.** The substrate enforces deny-by-default policy on inter-service and client-service communication via Fine-grained Data Access Engine (FDAE) compiled ReBAC policies (`crates/fdae/`, ADR-0017) and UCAN capability tokens. The product accurately documents the infrastructure operator's technical powers; policy enforcement alone must not be presented as protection from a fully compromised host. Sensitive deployments may require owner-held encryption keys.
> **Envisioned.** Not built yet. Hardware attestation (TPM/SEV). Today substrate integrity relies on software signature validation.

---

## Supporting Ecosystem Entities

### Relay

- Acts as a coordination server for direct connections between peers using UDP hole punching.
- Acts as an encrypted TCP data relay when direct connection is not possible (no UDP, symmetric NAT, CGNAT) via embedded `iroh-relay`.
- Substrates connect to a statically configured coordinator relay URL (`parent_coordinator.iroh.url`) and publish signed endpoint records to the Community Registry or Mainline DHT via pkarr. Services inherit this substrate relay connectivity.
  > **Envisioned.** Not built yet. A WebRTC TURN relay server, dynamic bootstrap relay registration, and `<relaynodeid>.syneroym.net` DNS subdomains. Today relays do not provide TURN; browser clients connect through the Client Gateway HTTP proxy or WebRTC data channels with HTTP/WebSocket signaling.

### Bootstrap

- Accepts NodeId registration offers; with associated relay details as applicable.
- Maintains a list of officially operated relays with capability metadata (TCP relay, TURN, etc.).
- Accepts community relay registration offers; verifies capability claims (offline or real-time checks).
- Registers DNS entries for community relays under its domain e.g. `*.syneroym.xyz`.
- Returns a weighted random set of relays from the registry based on requested capability and relay capacity.
- Periodically audits registered relays and expires stale entries.
- For node ID lookups, checks internal cache or DHT fallback and returns the relay. For HTTP URL lookups from browsers, finds the relay and issues an HTTP redirect.
  > **Envisioned.** Not built yet. Centralized bootstrap servers and dynamic HTTP redirects. Today substrates configure a static coordinator relay URL (`parent_coordinator.iroh.url`) and publish signed endpoint records to the Community Registry (`crates/community_registry`) and pkarr BEP-0044 Mainline DHT (`crates/core/src/dht_registry`).

> **Single point of failure note.** A default bootstrap service is a governance
> and availability dependency. Its signed records must be exportable and
> publishable through alternative operators or discovery mechanisms. Known peers
> and cached routes must satisfy the bootstrap outage release gate; one
> Syneroym-controlled service must not be required to authorise continued use.
> Today nodes operate independently without central authorization.

### Consumer-Facing Aggregation

> This section addresses a gap in the prior spec. Centralised platforms provide consumers a single app. In Syneroym, providers may run on different substrates operated by different entities. The consumer experience remains coherent.

- An Aggregator is a directory-type SynOrg service (`crates/roym_directory/src/app.rs`) aggregating provider listings and credentials. Aggregators do not manage hosting or infrastructure for providers.
- A Consumer App (such as the Roym Hub browser web application) allows consumers to discover, browse, and transact with providers across multiple substrates and SynApps from a single interface.
- The Consumer App queries one or more community directories, and merges results via deterministic round-robin client-side merging (`crates/roym_directory/src/app/client_merge.rs`) with visible source attribution, freshness, and refusal reasons.
  > **Envisioned.** Not built yet. Peer referral links and vouching graphs. Today consumers discover providers via direct listing IDs or directory queries.
- A consumer's identity, signed receipts, grants, and preferences are portable and controlled from the consumer's device or designated store. Running a personal substrate is optional; consumers can connect via the Client Gateway.
- The Consumer App is a thin client; business logic runs on provider substrates. The Consumer App is not a privileged participant in the ecosystem.

### [GTW-PRX] Client Gateway Multi-Substrate Ingress Proxy

The Client Gateway bridges HTTP and WebSocket traffic from web browsers and external clients to internal and remote substrate RPC handlers.

- **Ingress Protocol Translation:** The gateway listens on port 7960 (configurable) and translates browser HTTP requests (`POST /rpc`, `GET /metrics`, `GET /blobs/*`) and WebSocket connections (`/__syneroym/tunnel`, `/__syneroym/ws`) into substrate router invocations.
- **Session Identity Propagation:** The gateway extracts caller credentials from session cookies (`syneroym_session`) or Authorization headers, validating them against the Node Auth Service and injecting verified caller DIDs into requests.
- **Multi-Substrate Routing:** Inbound requests for foreign substrate services route across the Iroh network overlay using local static inventories or registry lookups, without requiring browser runtimes to maintain native QUIC connections.
- **Static Asset Delivery:** Static user interface assets and application bundles stream directly from the filesystem or content-addressed blob storage without instantiating guest WebAssembly components.

---

## SynApp Lifecycle

### Development

- Developers build encapsulated components (using WebAssembly components or Podman container images) that define clear, strongly-typed interfaces for inter-component communication.
- The system supports automatically deriving external-facing APIs (such as JSON-RPC or HTTP passthrough) from these internal component interfaces via the Universal Proxy to support diverse clients like web browsers.
- Manifests reference compiled `.wasm` binaries or OCI container images in `ServiceDefinition::source`, and static web assets are bundled into `AssetBundle` records stored in content-addressed blob storage. Dual-build execution compiles SynApps as either sandboxed `wasm32-wasip2` components or statically linked native binaries (`syneroym-app-host-native`).
  > **Envisioned.** Not built yet. Standalone signed package bundle distribution format and public OCI registry distribution. Today deployment bundles are supplied as local file paths or artifact bundles.

### [APP-AST] Zero-Runtime Static Asset Passthrough

Service deployment packages bundle static web assets (`AssetBundle`) streamed directly by the HTTP proxy from blob storage without instantiating or executing guest WebAssembly components.

- **Asset Bundle Manifest Declaration:** Service manifests declare an optional `AssetBundle` specifying archive location, optional content hash, and visibility (`Visibility::Public` or `Visibility::Private`).
- **Blob Store Ingestion:** At deployment time, the control plane registers and unpacks asset bundles into the substrate content-addressed blob store.
- **Direct HTTP Streaming:** Inbound HTTP requests for static routes stream asset bytes directly from the blob store via the Universal Proxy. The substrate serves static assets with low latency and without allocating WebAssembly instance fuel or memory.

### Deployment

- An Application Specification (`SynAppManifest`) composes components into a SynApp and declares dependencies, resource limits (CPU, memory), and configuration schema.
- The specification declares service identifiers, versions, requested capabilities, permissions (FDAE policies), and migration lifecycle hooks (`init`, `migrate`).
  > **Envisioned.** Not built yet. Standalone package signatures, formal data classifications, automated backup declarations, and declarative rollback behaviour.
- A provider or operator applies the Application Specification to chosen substrate(s) using `roymctl app deploy` or the App Supervisor.
- Before deployment, the substrate validates UCAN deploy permissions, semver constraints, and configuration schemas.
  > **Envisioned.** Not built yet. Pre-deploy node-level resource capacity checks and interactive capability-expansion approval summaries.
- A partially failed deployment is cleaned up by the SQLite deployment journal, tracking reconciliation actions and rolling back partially registered services. Installation state is inspectable via `roymctl app status`.

### Monitoring

- Substrate monitors applications and provides health information through the App Supervisor's resident reconcile loop and HTTP health endpoints (`/health`, `/v1/info`), publishing failure alerts to MQTT topics.
  > **Envisioned.** Not built yet. Outbound push notifications to external notification services.
- Providers receive alerts through CLI inspection (`roymctl app status`, `roymctl app alerts`) and supervisor JSON-RPC `alerts` queries.
  > **Envisioned.** Not built yet. Webhook dispatch and real-time UI push alert notifications.
- Updates require valid controller UCAN signatures, and configuration generations are versioned in `config_generations`.
  > **Envisioned.** Not built yet. Automated snapshot and rollback to last known-good packages on post-update health check failure.

---

## Reference Vertical Contracts

Reference SynApps validate common ecosystem contracts without forcing unrelated domains into one generic application. They may share modules and schemas, but each has its own language, workflow, policy, and usability tests. Roym implements the primary reference vertical: the Professional and Home Services Guild.

> **Envisioned.** Not built yet. The second reference vertical (local producer-distributor or food and small retailer mesh). Today Roym is the single built reference SynApp.

### Home Services Guild

#### Provider and guild setup

- A guild operator deploys a signed release profile and creates a guild with public identity, service area, membership policy, support contact, dispute path, directory policy, and data retention policy.
- A provider controls a stable provider master DID and grants the guild only the administration rights needed for the chosen managed service via scoped delegation certificates.
  > **Envisioned.** Not built yet. Interactive invitation and application vetting workflows. Today operators manage members directly via roster administration.
- A provider publishes name, description, service categories, service area, availability, price style (fixed, range, or quote), cancellation policy, supported payment rails, and trust evidence. Required fields and provenance are machine-readable.
- One operator can manage multiple provider service instances without obtaining undeclared read access across their private conversations or histories. Conversations use Double Ratchet end-to-end encryption, and databases use derived per-instance encryption keys.
  > **Envisioned.** Not built yet. Externally provisioned per-instance Key Encryption Keys (Model B) protecting against an operator with host memory access.

#### Consumer-provider workflow

- **Discover.** Consumers reach a provider by direct link or guild directory queries with client-side deterministic merging. Results show source, freshness, and filters; paid placement is absent.
  > **Envisioned.** Not built yet. Peer referral links and vouching graphs. Today consumers discover providers via direct listing IDs or directory queries.
- **Assess.** Consumers see relevant services, price basis, availability, provider identity continuity, trust evidence, guild relationship, and material policies before sharing personal details.
- **Request and clarify.** A request captures category, description, approximate area, preferred window, attachments, and a data-use notice. Exact address disclosure is withheld until quote agreement via machine-readable policy. Parties clarify in the linked conversation.
- **Quote and agree.** A versioned quote states scope, price, taxes or fees, schedule, location, payment method, cancellation/refund terms, expiry, and dispute path. Both parties' acceptance produces a signed agreement receipt.
- **Fulfil `[VRT-SRV]`.** Permitted states and actors are explicit. Bookings follow discrete state tracks (`Scheduled`, `InProgress`, `Completed`, `Cancelled`, `Conflict`, `EndedUnconfirmed`). Transitions enforce actor roles: only the provider can cancel a booking, and only before any track has moved. Completed bookings require mutual confirmation on two independent tracks (`payment` and `fulfilment`). State changes produce append-only signed progress records (`BookingProgressPayload`), supersede previous envelopes idempotently, and never rewrite signed history.
  > **Envisioned.** Not built yet. Consumer-initiated cancellation and automated dispute arbiters. Today cancellation is provider-only and dispute resolution is handled out-of-band.
- **Settle `[VRT-PAY]`.** Payments use signed out-of-band payment records (`PaymentRequestPayload` and `PaymentAcknowledgementPayload`). Providers request payment stating currency and minor-unit amount. Both parties independently record signed payment acknowledgements. The system does not process funds directly and never treats an unverified return from a third-party payment app as final settlement.
  > **Envisioned.** Not built yet. Integrated payment processors (such as Stripe Connect SDK), in-app escrow custody, system coins, and mutual credit rails. Today transactions record signed out-of-band payment notices only.
- **Close and return.** Completion produces portable signed receipts (`InteractionReceipt` and `FulfilmentRecord`). Either party can start a repeat request in the existing conversation without re-entering consented information.
  > **Envisioned.** Not built yet. Consumer feedback and review submissions tied to receipts.

### [ROY-ADM] Application-Tier Local-Only Ingress Firewall

Private SynApp services enforce an application-tier admission firewall on all inbound invocations to prevent unauthorized remote network access to private data and APIs.

- **Caller Origin Inspection:** Inbound method dispatches inspect caller origin via the invocation host interface (`syneroym:invocation/invocation`). Calls originating from within the local substrate installation resolve to `CallerOrigin::Internal`.
- **Fail-Closed Rejection (`NOT_LOCAL`):** Inbound calls arriving from remote nodes (`CallerOrigin::Verified` or `CallerOrigin::Anonymous`) are rejected with JSON-RPC error code `-32013` (`NOT_LOCAL`: "this method is reachable only from inside this installation"). The refusal reveals no internal service identifiers or caller DIDs to unauthorized parties.
- **Explicit Wire Exceptions:** Public directory services define explicit wire exception tables (`WireRule`) allowing foreign callers:
  - `WireRule::Open`: Permits unauthenticated foreign callers for public read operations (`directory.search`, `directory.info`, `directory.standing`).
  - `WireRule::VerifiedOnly`: Permits remote callers whose identity was verified by the router for authenticated operations (`directory.publish`).
- **Default Isolation:** Services lacking explicit wire exception tables (such as `profile`, `catalog`, and `transaction`) remain entirely internal and reject all off-node invocations.

### [TXN-SLT] First-Claim Slot Reservation Concurrency Fence

When multiple consumers accept quotes for the same limited provider availability slot, the transaction ledger enforces single-writer arbitration to prevent double booking.

- **Atomic Seat Claims:** The provider transaction service arbitrates accepted quotes against catalog availability. For slot-based bookings, the service checks slot existence and remaining capacity, bounded by `MAX_SLOT_CAPACITY = 64`. It attempts to write an atomic seat record (`seat:<slot_id>:<seat_number>`) into the ledger.
- **First-Claim Decision:** Exactly one consumer claim succeeds for an available seat. That booking transitions to `Scheduled` with a signed initial progress snapshot (`BookingProgressPayload`).
- **Typed Conflict Refusal:** If all seats for the quoted slot are already claimed, or if the slot no longer exists, the competing booking transitions to `Conflict` with a machine-readable reason (`ConflictReason::SlotTaken` or `ConflictReason::SlotUnavailable`). The provider does not countersign conflicting bookings.
- **Idempotent Retry:** Repeated quote acceptance or sync requests on an already decided booking return cached results (`AlreadyDecided` or `already-accepted`) without altering previously committed seats or creating duplicate records.

### Service Variation Dimensions

The reference SynApp implements variation axes across workflows using seven strongly typed, optional named blocks in `ListingPayload`:

- **Booking (`BookingTerms`):** Event slots, consulting time slots, and open-ended job requests, modeled by `BookingMode` (`Slots`, `Order`, `Enquiry`). Slot bookings enforce capacity limits (`MAX_SLOT_CAPACITY = 64`) and first-claim concurrency fences.
- **Payment (`PaymentTerms`):** One-time quote-based payments with pre- or post-delivery timing, modeled by `PaymentModel` (`Fixed`, `PerHour`, `PerUnit`, `QuoteOnly`), currency, and minor-unit amounts.
  > **Envisioned.** Not built yet. Multi-part payments, subscriptions, decentralized escrow, system coins, and mutual credit networks. Today quotes support single out-of-band payments only.
- **Product type (`ProductDetail`):** Physical goods with unit, pack size, SKU, and condition (`New`, `Used`, `Refurbished`).
  > **Envisioned.** Not built yet. Time-bound prepared food spoilage timers and digital content streaming or DRM pipelines.
- **Service type (`ServiceDetail`):** Time-slot services, job-completion-based services, and location-based services with declared durations, inclusions, exclusions, and prerequisites.
- **Location (`LocationTerms`):** Fixed provider locations, customer locations, and remote digital services (`ServiceLocation`), with bounding service areas and machine-readable address disclosure policies (`AddressDisclosure::OnAgreement` and `AddressDisclosure::Public`).
- **Relationship type (`RelationshipTerms`):** Eligibility controls (`Anyone`, `Members`, `Referral`, `ExistingCustomers`) and guild membership requirements. Continuous shared history is preserved across engagements in durable conversation threads.
  > **Envisioned.** Not built yet. Automated recurring relationship agreements and retainer schedules.
- **Service record (`ServiceRecordTerms`):** Portable completion receipts, stated warranty durations, and declared record retention windows.
  > **Envisioned.** Not built yet. Real-time active GPS and delivery telemetry tracking feeds.




---

<a id="post-dd864a1-target-specifications-addendum"></a>
<a id="target-designs-addendum"></a>

## Target Designs (Addendum)

This section defines target specifications for the substrate and applications across functional areas. The preceding sections establish the system foundation; each phase section below details specific target capabilities.

> **The phases are targets.** A phase is a planned group of work, not a record that the work is done. Each phase section has built parts and Envisioned parts. Text with no marker is built. Text under the Envisioned marker is not built. The [traceability matrix](planning/traceability-matrix.md) gives the status of each requirement.

### Tag Legend
To ensure stable cross-referencing across commits and PRs, features are prefixed with category tags:
- **`[TOP]`**: **Topology** (Core Architecture Primitives)
- **`[FND]`**: **Foundation** (Core Infrastructure & Security)
- **`[PLT]`**: **Platform** (Data Layer & Resilience)
- **`[LFC]`**: **Lifecycle** (Substrate & Application Management)
- **`[ADV]`**: **Advanced** (Advanced Services & Tooling)
- **`[P2P]`**: **Peer-to-Peer** (Community Primitives)
- **`[APP]`**: **Applications** (High-Level SynApps)
- **`[EDG]`**: **Edge** (Edge Expansion & Mobile)


## Phase 0: Core Architecture Implementation (SynApp & Topology)

This phase implements the architectural boundary between Syneroym Applications (`SynApp`) and Syneroym Services (`SynSvc`), and the pending addressing and registry systems required for robust service discovery.
*(Current baseline: the codebase already has DID-key service identities, a community endpoint registry client backed by HTTP/pkarr, and an in-process local `EndpointRegistry`. This phase adds app-instance namespaces, topology-aware logical names, and orchestration on top of those primitives.)*

### [TOP-PRM] Core Primitives (`SynSvc`) vs. Control Plane Overlay (`SynApp`)

#### `SynSvc` (The Execution Primitive)
The `SynSvc` is the absolute foundational primitive of the Syneroym Substrate. 
*   **Zero-Trust Execution:** Represents an isolated, zero-trust execution boundary (often a WASM component, but could also be a Podman container or a native OS service sitting behind a platform gatekeeper). It does not implicitly trust other services, even those deployed alongside it.
*   **State & Capabilities:** It owns its state and enforces capability-based security (FDAE ReBAC policies and UCANs) on all incoming requests.
*   **Protocol Compatibility & Fixed ALPN:** Substrate peer-to-peer connections bind a fixed ALPN identifier (`syneroym/0.1`). Protocol and interface dispatch are negotiated per stream through route preambles; unsupported protocols fail fast with typed errors.
    > **Envisioned.** Not built yet. Protocol capability negotiation matrix and dynamic HELLO handshake exchanges. Today peer-to-peer connections bind fixed ALPN (`syneroym/0.1`), and unsupported protocols immediately fail fast on preamble parsing.
    >
    > Multi-substrate connections establish a capability matrix and negotiate supported protocol versions dynamically during transport connection establishment.

#### `SynApp` (The Control Plane Overlay)
`SynApp` is removed as a runtime execution boundary and is redefined as a **Deployment Manifest and Control Plane Overlay**.
*   **Lifecycle Management:** Acts as a blueprint to deploy, update, and remove a cohesive graph of `SynSvcs` as a single unit.
*   **Capability Bootstrapping:** Orchestrates the initial injection of permissions (ReBAC relations/policies) that allow internal services within the app to communicate.
*   **Resource Accounting:** Serves as a logical grouping for tracking quotas, billing, and telemetry across a designated graph of services.
*   **SynApp Instances:** Unlike Erlang applications (which are singletons), deploying a `SynApp` manifest creates a unique **SynApp Instance** with an isolated namespace. A single substrate can host multiple distinct instances of the same `SynApp` (e.g., Personal Task Manager vs. Work Task Manager).
*   **UI Decoupling:** User Interfaces are simply specialized `SynSvcs` or external clients. A `SynApp` may contain zero UIs (headless processes), one UI, or multiple specialized UIs (Admin, Storefront, Mobile Gateway).
*   **Terminology Reconciliation:** Older docs sometimes say a `SynApp` "runs on" a substrate. In this model, only `SynSvcs` execute. A `SynApp Instance` is the manifest, namespace, capability bootstrap, dependency graph, and accounting context for those executing services.

#### Composable SynApps (App Dependencies)
Similar to Erlang OTP applications, `SynApps` are highly composable. A `SynApp` manifest is not restricted to explicitly declaring raw `SynSvcs`; it can declare dependencies on other `SynApps`.
*   **Dependency Resolution:** If `SynApp: Retail Store` depends on `SynApp: Identity Core`, the Orchestrator evaluates the dependency graph during deployment. It will ensure `Identity Core` is instantiated (or bind to an existing instance) before deploying the `Retail Store` instance.
*   **Instance Mapping:** This maintains the crucial App (Blueprint) vs. App Instance distinction. A higher-level SynApp can compose multiple foundational SynApps into a unified, deployed ecosystem, passing necessary capabilities down the dependency tree.

---

### [NET-PRM] Preamble Routing Tokens & Delegation Verification

Inbound transport streams carry a structured route preamble before payload data, allowing the router to establish identity, check delegation, and configure streaming pipelines before dispatching to destination services.

- **Preamble Wire Format:** The stream preamble follows the grammar `<scheme>://<interface>.<service_id>[?query]`, terminated by a newline. Schemes overload wire transport (`binary`, `http`, `raw`) and application protocol (`json-rpc`, `wrpc`, `raw`). Preamble lines are bounded by a maximum size limit (`MAX_PREAMBLE_LINE_BYTES = 256 KiB`) and a pre-authentication timeout (`PRE_AUTH_READ_TIMEOUT = 5s`) to prevent unauthenticated resource exhaustion.
- **Transport Security & E2EE Query Parameters:** The preamble supports orthogonal query parameters:
  - `enc` and `pubkey`: Trigger an ephemeral ECDH-P256 key exchange wrapped in AES-256-GCM encryption before payload forwarding.
  - `dir`: Designates stream direction (`upload` or `download`) for raw streaming protocols.
- **Delegation Certificate Verification:** When a caller routes using a delegated identity, the preamble includes a hex-encoded `DelegationCertificate` (`?delegation=<hex>`). The router verifies that the certificate's temporary DID matches the preamble public key, checks validity timestamps, confirms that the scope covers transport permissions, and verifies that the audience key is not revoked in the issuer's Master Anchor DHT record.
- **UCAN Capability Token Verification:** The preamble supports an optional hex-encoded UCAN capability token (`?ucan=<hex>`). The router validates the token chain into `SessionContext` capabilities, checking each chain edge against issuer Master Anchor revocations. Unauthenticated or invalid UCAN tokens fail closed for native capability interfaces.

---

### [TOP-ADR] Service Addressing and Resolution Topology

Services communicate using a multi-tiered addressing model to support mobility, redundancy, and explicit targeting.

#### Addressing Types
1.  **Explicit Service ID (Physical ID):** 
    *   A stable cryptographic identifier for a deployed `SynSvc` instance. The current implementation uses DID-key identities derived from Ed25519 public keys; future encodings may wrap that in a shorter service identifier for ergonomics.
    *   Provider ownership of the service is proven via the service identity and its signed endpoint records, UCANs, or deployment certificates. The route to that service may change without changing the service identity.
    *   Used for stateful interactions, direct replies, and underlying substrate routing.
2.  **Logical Service Name:** 
    *   A human-readable or contextual identifier (e.g., `profile-svc`, `ledger-primary`) representing a *role* within a `SynApp Instance` namespace.
    *   Used by developers in code to ensure high availability, load balancing, and decoupling.

#### Service Topologies
When registering a Logical Service Name, the local registry tracks its underlying topology:
*   **Singleton:** Maps to exactly one Explicit ID.
*   **Redundant (Load Balanced):** Maps to an array of Explicit IDs. The resolver returns the eligible set and the caller/proxy selects a target using the manifest's policy.
*   **Sharded:** Maps to multiple Explicit IDs based on a stable routing key (for example, consistent hashing on `user_id`). The runtime resolver supports Sharded topology selection using range sharding and BLAKE3 rendezvous hashing.
    > **Envisioned.** Not built yet. Manifest compiler emission of Sharded topologies. Today manifests compile replicas greater than one to Redundant mode, and Sharded manifest syntax is not yet exposed in service manifests.

---

### [TOP-REG] Types of Registries in the Ecosystem

Rather than a strictly monolithic system, service discovery naturally emerges across different registry scopes:

1.  **Community Identity/Endpoint Registry (HTTP + pkarr/DHT):** Resolves top-level Provider, Node, and public Service identities to signed endpoint records (for example, Iroh endpoint addresses, WebRTC peer hints, or public gateway URLs).
2.  **Contextual/App Registry:** Resolves Logical Service Names to Explicit Service IDs within a specific `SynApp Instance` overlay context or shared node namespace. This mapping is **pushed into each service's configuration** by `roymctl` or the App Supervisor, not served by a queryable registry that services call at runtime — see [ADR-0021](decisions/0021-binding-propagation-and-app-supervisor.md).
3.  **Endpoint/Router Registry:** The internal substrate routing table. Maps an Explicit Service ID and interface to actual execution boundaries (for example, local WASM channels, native host functions, Podman sockets, TCP host/ports, or remote network sockets).

---

### [TOP-DSC] Discovery Mechanisms and Inventory

#### Service Inventory and Resolution Architecture
Per [ADR-0021](decisions/0021-binding-propagation-and-app-supervisor.md), intra-application service discovery does not use a queryable runtime registry service. Instead, service inventory and resolution follow a decoupled configuration-push and signed document architecture:
*   **Intra-App Binding Propagation:** The App Supervisor or `roymctl` pushes resolved logical-to-physical service bindings into each service's configuration at deploy time. Services load these bindings into an in-memory `StaticInventory`, resolving local dependencies without runtime network lookups.
*   **Health Tracking & Reconciliation:** Health tracking and readiness checks are performed by the App Supervisor's reconciliation loop rather than a runtime registry service.
*   **Inter-App Discovery:** Inter-application discovery uses signed `TopologyDocument` records issued by the target application's App Supervisor (see `[TOP-DOC]`) and published endpoint records from community registries or Mainline DHT.
*   **Client Caching (Refresh-on-Failure):** Resolvers cache resolved topologies in memory (`TopologyCache`) with an epoch or TTL. If a connection fails or an epoch changes, the client refreshes the entry on failure rather than polling continuously.
*   **Master Anchor Resolution (Revocation Handling):** To maintain secure identity revocation without breaking standard DHT signatures, the ecosystem enforces the **Master Anchor** pattern:
    *   Registries map Logical Service Names exclusively to the **Master Key** (DID).
    *   The Master Key publishes a `master_anchor_v1` record to pkarr DHT containing a cryptographic **revocation deny list** (`revoked_keys: Vec<String>`). Temporary routing keys prove authorization via signed delegation certificates (`DelegationCertificate`), not through an allowlist array in DHT.
    *   **Passive Revocation:** If a Temporary Key is compromised or rotated, the Master Key publishes the compromised key to its DHT `revoked_keys` deny list.
    *   Dependent clients and routers cache the route and delegation. Ingress routers and handshakes verify that the temporary routing key is not present in the issuer's Master Anchor deny list, rejecting revoked keys during connection setup.

#### Static Deployment Inventory (`roymctl`)
Not all apps require a live, queryable registry at runtime (e.g., trivial background cron jobs or standalone static UIs).
*   For these trivial apps, the orchestrator/CLI (`roymctl`) maintains a static, local state file (or local host DB).
*   It records the mapping of `SynApp Instance ID / logical role > Explicit Service ID(s)` at deploy time.
*   This static inventory is sufficient for lifecycle management (listing, stopping, uninstalling) without the overhead of spinning up a live Registry `SynSvc`.

---

### [TOP-ROB] Network & Connection Robustness

This defines the baseline resilience required for underlying node-to-node and client-to-node transport links, ensuring Syneroym handles transient network partitions gracefully before falling back to application-layer offline queues.

- **Transport Resilience & Retries:** 
  - The system must gracefully handle transient network drops. Any failed connection attempt must not immediately fail the higher-level request.
  - Implement automatic retries for establishing connections. The retry count should be configurable per service dependency or manifest default, defaulting to 3 retries with a simple exponential backoff.
  - Automatic request retries are only safe for connection setup, idempotent operations, or calls explicitly marked retryable with idempotency keys. Non-idempotent calls must surface failure or enter an opt-in outbox workflow.
- **Reactive Connection Management:**
  - Standard transport-level timeouts (e.g., QUIC idle timeouts, WebRTC SCTP timeouts) are used to detect dropped peers. We do not implement custom application-level ping/pong heartbeats to save bandwidth and complexity.
  - Stale connections are handled reactively: "evict when found out." If a read or write operation fails due to a disconnected peer, the connection is instantly marked as dead and retried or surfaced as an error.
- **Transport Modalities (Events vs Data):**
  - *(See `[PLT-DAP-04]` and `[PLT-DAP-05]` for decoupled event routing and data pipeline streams.)*

---

## Phase 1: Foundation & Core Infrastructure

### [FND-DEP] Deployment/Operations
- **Cloud-Agnostic Bare-Metal Deployment:** Single Rust binary deployed to a standard Linux instance (e.g., AWS Lightsail) to minimize virtualization overhead.
- **Packaging & Deployment:** Provisioning and deployment rely on multi-stage container images (`Dockerfile`), pre-configured community compose definitions (`deploy/docker-compose.community.yml`), and automated GitHub Actions release pipelines (`.github/workflows/release.yml`) compiling and publishing multi-architecture binaries and Docker images.
- **Native TLS:** Direct binding to port 443 within the Syneroym substrate using `rustls`. Certificates are fetched and renewed via `certbot`, and reloaded from disk via `SIGUSR1` signal handling without restarting the process.
- **Resource Protection:** Configuration parameters for connection caps and cache limits ensure the node gracefully refuses excess traffic instead of crashing (OOM).
- **Operator Experience:** SSH, `journalctl`, and local health endpoints (`/v1/info`) remain expert diagnostic tools. Encrypted backup and restore is managed via `roymctl`, and the App Supervisor reports active health alerts.
  > **Envisioned.** Not built yet. Interactive guided install and automated update/rollback CLI workflows. Today operations rely on standard CLI verbs and container lifecycle management.
- **Cross-Platform Distribution:** Automated build pipelines to compile and release Syneroym binaries for different architectures (Linux, macOS, Windows).
- **Dockerized Substrate:** Provide official Docker images of the Syneroym substrate for the community, pre-configured to point their local registries and coordinators to the public `syneroym.xyz` node.
- **Smoke Testing:** Automated integration/smoke tests that run against release candidates (binaries and Docker images) to verify they can successfully connect to and interact with the deployed coordinator and registry at `syneroym.xyz`.

### [FND-SEC] Substrate Security
- **Data at Rest Encryption (Envelope Encryption):** 
  - To prevent catastrophic re-encryption of gigabytes of data during key rotation, the substrate uses Envelope Encryption. Unique Data Encryption Keys (DEKs) are generated to encrypt the actual blobs and SQLite databases (via SQLCipher — see [ADR-0006](decisions/0006-sqlite-encryption-sqlcipher.md)).
  - The service owner negotiates and injects a Master Key (Key Encryption Key or KEK) securely into substrate RAM at startup. The KEK only encrypts the tiny DEKs stored on disk. Key rotation is instantaneous as only the DEKs are re-encrypted with the new KEK. KEK scope narrows progressively: a per-SynApp-Instance KEK *derived* from the master KEK (defense-in-depth, not tenant isolation), with IAM-gated per-instance/per-service *provisioning* (a distinct, separately-injected KEK per instance, gating production multi-tenant-at-rest security per [ADR-0006](decisions/0006-sqlite-encryption-sqlcipher.md)) as the still-outstanding eventual target; DEKs are per-service from day one.
  - **Secret Vault:** Application secrets (API keys, credentials) are stored securely inside a dedicated Vault table within the encrypted per-service SQLite database, rather than as vulnerable flat files on disk. Non-secret configuration may share the same encrypted store for convenience, but it is not treated as a secret unless marked as such.
  - Production profiles default local databases and all remote backups to
    encryption. Opt-out is limited to explicitly marked non-sensitive
    development profiles and produces a persistent insecure-state warning.
  - **Remote Backups:** Local backup archives are encrypted locally before storage or transit (see `[IDT-BAK]`).
    > **Envisioned.** Not built yet. Streaming live WAL frames or object snapshots to S3-compatible stores or peer backup substrates. Today backups are created as sealed encrypted archive files.
- **Hardware Attestation (optional; layers on without changing the security model):**
  > **Envisioned.** Not built yet. Substrate integrity relies on software signature validation.
  >
  > The substrate exposes a `substrate.attest(nonce)` API to the network. The App Deployer/Owner externally challenges the node (at deployment or periodically) and mathematically verifies the hardware quote (TPM, KeyAttestation, AppAttest). The deployer alone decides whether to deploy the service in a degraded trust environment or halt execution if attestation fails.
- **Memory Protection & Key Splitting:**
  - OS-level memory locking (e.g., `mlock`) prevents injected cryptographic keys from being swapped to disk.
  - The `zeroize` crate is used to explicitly wipe sensitive variables from RAM when dropped.
  - Key fragmentation may be investigated as defence in depth but is not treated
    as a security guarantee or release requirement. The threat model assumes a
    fully compromised host can observe plaintext while it is in use unless
    stronger hardware isolation is proven.
- **Resource Exhaustion & Quotas:**
  - Network edge protection: Strict connection and payload limits at the Iroh/QUIC boundary.
  - Runtime execution limits: The substrate enforces the physical capabilities of the host alongside strict quotas defined in the `SynApp` manifest (e.g., `max_memory`, `max_instructions`). Wasmtime's fuel metering deterministically traps components exceeding their gas limits without stalling the node.
- **Supply Chain Integrity:**
  Binary releases produce verifiable SHA-256 checksums, and SynApp packages use SHA-256 content hashes.
  > **Envisioned.** Not built yet. Offline cryptographic signing of binaries and SynApp packages, trust-root rotation, and verifiable publisher provenance. Today integrity verification relies on release SHA-256 checksums and content-addressed hashes.
  >
  > Released binaries and SynApp packages are signed and verifiable offline. Trust-root rotation, compromise recovery, publisher identity, provenance, and rollback protection are documented; one permanently hardcoded project key must not be the ecosystem's unrecoverable trust root.

> **Implementation Design:** For technical details covering Envelope Encryption and Memory Protection, see [Feature Design: FND-SEC](system-architecture.md#fnd-sec-substrate-security).

### [SEC-SGN] Host-Only Signing Isolation
Private cryptographic signing keys stay in substrate host memory. Guest WebAssembly components never access raw private keys, preventing key exfiltration.

- **Host Signing Boundary (`syneroym:signing`):** Guest components sign records only through the host WIT interface (`syneroym:signing`). The host provides no general "sign these bytes" interface and returns no private keys.
- **Envelope Creation & Draft Validation:** The host builds the canonical JSON signed record envelope (`ENVELOPE_VERSION = 1`) around the guest's record draft (`record-draft`). The host validates the draft structure, supplies the issuance timestamp, and sets the verified issuer DID. Guests cannot create timestamps or fake record issuers.
- **Principal Modes:** Signing supports two principal modes:
  - `service`: Signs with the service's derived signing key, using that key's `did:key` as the issuer.
  - `delegated`: Signs on behalf of another master DID (such as an organization or person) proven by a signed `DelegationCertificate` scoped for `record-signing`. The host checks the certificate on every call and rejects certificates that do not certify the service's signing key.

### [SEC-ISO] Wasmtime Sandbox Isolation & Resource Limits
Substrates run guest WebAssembly components inside Wasmtime sandboxes with strict resource boundaries and capability limits.

- **Pooling Allocator:** The Wasmtime engine uses a pre-allocated instance pooling allocator (`InstanceAllocationStrategy::Pooling`) with copy-on-write memory initialization (`memory_init_cow`). Total component instances, core instances, memories, and tables are bounded at substrate initialization to prevent memory exhaustion.
- **Deterministic Fuel Metering:** Wasmtime consumes fuel (`consume_fuel(true)`) on each executed instruction. Component invocations receive an instruction limit from the service manifest (`max_instructions`) or substrate default. If a component uses all its fuel, the host halts execution without blocking the runtime.
- **Epoch-Based Interruption:** An engine epoch ticker advances a periodic clock. Invocations bind epoch deadlines for request dispatch, lifecycle hooks, and ABAC evaluation (`dispatch_epoch_ticks`, `lifecycle_hook_epoch_ticks`, `abac_epoch_ticks`). Passing the deadline interrupts execution.
- **Empty WASI Context:** Sandboxes initialize with an empty `WasiCtx` (`WasiCtx::builder().build()`). Components have no access to host filesystems, environment variables, network sockets, or system clocks. All platform operations occur through explicit `syneroym:*` WIT host capability imports.

### [FND-IDT] Cryptographic Identity Primitives
- **Issuer-Neutral Key Hierarchy:** Implement stable owner-controlled identity
  anchors that delegate rotatable device and routing keys. Government IDs,
  community credentials, hardware keys, and social recovery are optional
  assurance or recovery methods; none is the universal Tier 1 root.
- **Identity Export & Recovery:** Securely export or recover identity authority
  without silently granting an operator impersonation power. Recovery rotates
  compromised delegates, publishes revocation, preserves an auditable continuity
  chain, and tells the user when continuity cannot be proven.
- **Lightweight Consumer Identity:** Support device-bound consumer keys and an
  encrypted backup/import path without requiring a personal substrate.
- **Privacy-Preserving Credential Plugins:**
  > **Envisioned.** Not built yet. Sandboxed extension points for zero-knowledge or external credential proof schemes.
  >
  > A sandboxed extension point may load proof schemes such as `anon-aadhaar` when a vertical and jurisdiction justify them. This is not release-blocking and must not enlarge the default trust base.


### [FND-CFG] Service Configuration

Given that Syneroym supports both native WASM components and legacy Podman containers, configuration and secret management use a dual-target approach:

- **Configuration Delivery**:
  - **WASM (Native)**: Services retrieve their hierarchical configuration on-demand via a standard host function (e.g., `syneroym:app-config/get`). WASI environment variables or pre-opened files may be exposed only as an explicit compatibility mode for non-secret values.
  - **Podman (Legacy)**: Because third-party containers expect specific formats, the `SynApp` manifest dictates how the orchestrator exposes the config. The orchestrator will either flatten the config into standard environment variables or serialize nested configurations (JSON/TOML/YAML) into temporary files and mount them read-only into the container.
- **Secret Management**:
  - **WASM (Native)**: Strictly adheres to `[FND-SEC]`. The service pulls secrets directly into locked RAM via `syneroym:vault/reveal`. Secrets never touch the filesystem or environment variables.
  - **Podman (Legacy)**: The orchestrator resolves the secret from the Vault at deployment and injects it as an environment variable or via an ephemeral `tmpfs` mount. This accepts a degraded security posture (secrets visible in process lists) as a necessary tradeoff for running legacy software.
- **Dynamic Updates & Restarts**: Configuration is versioned and immutable for a running invocation. For WASM, a new configuration generation applies to the next component invocation; already-running invocations continue with the generation they started with. For Podman, the orchestrator must gracefully restart/recreate the long-lived container to apply the new configuration or secrets.
- **App Composition (Bind vs. Spawn)**: When a parent `SynApp` depends on another app, the configuration resolves based on the dependency mode:
  - **Spawn**: If the dependency must be spun up alongside the parent, `roymctl` inlines the child manifest into the parent at deploy-time, creating a single flattened deployment graph.
  - **Bind**: If the parent depends on an *already running* app instance, the parent manifest references it. The target's Explicit Service IDs are resolved at deploy time and injected into the parent's configuration, rather than spawning new instances. A bound dependency's identity survives relocation and restart ([ADR-0020](decisions/0020-stable-logical-service-identity.md)), so it breaks only if that service is genuinely replaced — at which point the parent's owner owns the consequence. The parent's App Supervisor health-probes bound external dependencies on its normal poll loop so such a break surfaces as an alert rather than as user-visible failure.
- **Schema Validation & Defaults**: To prevent runtime crashes, `SynSvc` manifests can define a schema (e.g., JSON Schema) for their expected configuration. `roymctl` and the Orchestrator validate the user-provided configuration against this schema at deploy-time, catching missing keys or type mismatches early.
- **Out-of-Band Secret Rotation**: While regular configuration changes happen via explicit manifest deployments (which naturally trigger a restart), secrets live independently in the Vault. If a secret is rotated *out-of-band* by an admin, the manifest's `rotation_policy` dictates whether the orchestrator automatically restarts the affected service or waits for the next manual deployment.
- **Anti-Goal: "Helm-ification"**: The `SynApp` manifest is strictly a "dumb", fully-resolved document. Syneroym rejects complex in-manifest templating (like Helm). If developers need environment-specific overrides, they should use external tools (like `cue`, `ytt`, or simple scripts) to generate a static manifest *before* passing it to `roymctl deploy`. Manifests are strictly static and fully resolved before deployment.

> **Implementation Design:** For technical details regarding the dual-target configuration delivery and cold restart behavior, see [Feature Design: FND-CFG](system-architecture.md#fnd-cfg-service-configuration).

### [FND-IAM] Access Control
- **FDAE (Federated Data-Aware Authorization Engine):** Adopts the FDAE architecture, which decouples the authorization specification (the "What") from the environment-specific execution (the "How"). It avoids the traditional PBAC vs. ReBAC dilemma by acting as an intelligent, distributed routing engine. It utilizes a declarative, Zanzibar-style structured configuration (e.g., YAML/JSON) to map relationship chains across fragmented data sources. The Substrate directly deserializes this configuration into a typed policy model, avoiding custom string parsers while still giving the query planner a structured representation to execute.
- **Data-Centric Authorization (RLS/CLS):** As data becomes distributed across shards and replicas, security policies dictate access. Row-Level Security (RLS) and Column-Level Security are enforced at query execution time via the FDAE pushdown, preventing unauthorized rows from ever reaching the guest application. *(Note: Database replication itself relies on node-level authorization rather than row-level UCANs inside the WAL stream).*
- **Solving the Data Fetching Problem (Pushdown Sieve):** For local contiguous relationships, FDAE collapses the graph into a single, deeply nested query. By compiling ReBAC policies directly into SQL `WHERE EXISTS` clauses, the SQLite engine performs massive-scale relationship filtering at the C-level, handing only authorized rows back to the WASM guest.
- **Dual-Mode Execution:** FDAE natively handles both **Point-In-Time Evaluation** (returning a swift Allowed/Denied flag for a specific resource check) and **Relational Data Filtering** (applying the security policies as a global subquery to prune index-level datasets before they ever reach the Wasm guest).
- **UCAN Integration (Normalized Claims, Capabilities, Scopes):** Access control is a robust synthesis of cryptographic capabilities and relational data state.
  - **Context Initialization:** When a request arrives, the substrate mathematically verifies the UCAN chain, normalizing external authentications (OIDC, DIDs, WebAuthn) into internal DIDs. It extracts the proven **claims**, **capabilities**, and **scopes**.
  - **Relational Verification:** The SQL Compiler uses these normalized UCAN scopes and claims as bound parameters (`?`) for its query.
- **The Extensible 4-Stage Hybrid Pipeline (Federated + SQL + WASM):** The authorization pipeline handles complex cross-boundary logic seamlessly:
  1. **Pre-Step (Context & UCAN Verification):** The substrate verifies the UCAN chain into a secure execution context.
  2. **Cross-Service Parameter Fetch:** If a relationship step crosses an asset boundary (e.g., requires data from a centralized security or org-service), the engine pauses, triggers an RPC/Wasm host function to fetch the remote relationship proofs or parameters, and injects them into the local evaluation context.
  3. **SQL Execution (The Relational Sieve):** SQLite natively filters candidate rows based on the ReBAC policies, UCAN context, and any parameters fetched during the cross-service fetch.
  4. **After-Step (ABAC & Override Filter):** An optional custom WASM function performs fine-grained, non-relational ABAC checks on the candidate rows.

> **Implementation Design:** For technical details regarding the FDAE architecture and the 4-stage hybrid pipeline, see [Feature Design: FND-IAM](system-architecture.md#fnd-iam-access-control).

### [SEC-ABAC] Row-Level Authorization via ABAC Evaluation
The substrate data layer enforces row-level attribute-based access control (ABAC) on candidate data rows through a stage-4 post-filter evaluation.

- **Post-Filter Evaluation Hook (`authorize-rows`):** When an FDAE security policy declares ABAC permissions on a data collection, candidate rows that pass the relational SQL sieve pass to a stage-4 post-filter before reaching the caller. The host invokes the guest's exported `syneroym:data-layer/authorizer#authorize-rows` function.
- **Context Injection & Candidate Rows:** The host supplies an `auth-context` containing the verified caller DID, session attributes, tenant parameters, and operation metadata alongside candidate rows. The hook returns row-level decisions (`allow`, `deny`, or field redaction masks).
- **Fail-Closed Verification:** If a policy requires stage-4 ABAC evaluation but the target component does not export the `authorize-rows` interface, the substrate denies all read access to the protected collection.
- **Epoch Limits:** Stage-4 evaluation runs under dedicated epoch limits (`abac_epoch_ticks`) to prevent slow guest authorization logic from stalling queries.

## Phase 2: Core Platform Capabilities

### [PLT-DAP] Distributed Data Topology
The substrate models data as a distributed, programmable topology rather than isolated object state.

- **[PLT-DAP-04] Decentralized Pub/Sub:** The system MUST support an MQTT-compatible API for decoupled event routing. Cross-node access to `publish` and `subscribe` routes to whichever node hosts the target service via standard JSON-RPC dispatch, matching cross-node data-layer calls.
  > **Envisioned.** Not built yet. Broker topic-log pull-based replication over QUIC. Today event routing runs through a single-node in-process MQTT broker without multi-node log synchronization (see `[PLT-RED]`).
- **[PLT-DAP-06] Generic Bidirectional Streaming:** The system MUST provide a `syneroym:messaging` host boundary allowing a WASM guest to register interest in a stream protocol namespace and handle both directions of a peer-initiated stream: as source, handing the host a stateful iterator (`stream-cursor`) resource that the host pulls from asynchronously (e.g., file download); as sink, handing the host a stateful sink (`stream-sink`) resource that the host pushes chunks into asynchronously (e.g., file upload). Distinct from `[PLT-DAP-05]`, which is Arrow/Substrait-specific and reserved for the DataFusion pushdown pipeline.

- **[PLT-DAP-01] Logical Data Services:**
  > **Envisioned.** Not built yet. Logical data wrappers that abstract physical sharding across multiple substrates. Today each stateful service connects to an isolated, single-node SQLite database.
  >
  > The system MUST support logical data wrappers that abstract physical sharding across multiple substrates, allowing a single dataset definition to span nodes transparently.
- **[PLT-DAP-02] Active Storage Pushdown:**
  > **Envisioned.** Not built yet. WIT interfaces for deploying WASM modules directly to the data layer. Today data access occurs through host functions in `syneroym:data-layer`.
  >
  > The system SHOULD provide WIT interfaces (e.g., `syneroym:data/transform`) for deploying WASM modules directly to the data layer to enable controlled ETL/ELT logic execution directly where the data lives.
- **[PLT-DAP-03] Declarative Replication:**
  > **Envisioned.** Not built yet. Declarative replication states in deployment plans. Today manifests reject replica counts greater than one for stateful services (see `[PLT-RED]`).
  >
  > The `DeploymentPlan` MUST support a declarative topology mechanism to define replication states (e.g., Primary, Read-Replica, Cold Backup).
- **[PLT-DAP-05] Data Pipeline Streams:**
  > **Envisioned.** Not built yet. Point-to-point QUIC data pipeline streams with credit-based flow control. Today bulk data transfers use chunk transfers over stream resources or HTTP endpoints.
  >
  > The system MUST provide a distinct `syneroym:data/stream` interface for direct, high-throughput, point-to-point QUIC streams with native credit-based flow control (backpressure) for heavy data shuffling.

### [PLT-DAT] Data Layer
The Data Layer provides a complete foundation for distributed application state and communication, securely accessed via typed host functions or APIs without exposing raw database engines to the applications.

- **Structured Data Service (Document Database):**
  - **Single Source of Truth (SQLite):** To prevent stale-data consistency issues, the underlying physical data layer is *always* SQLite. We do not maintain separate duplicate copies of databases (e.g., one for OLTP and one for OLAP).
  - **Build-Time Profiles (OLTP vs OLAP):** The system provides Cargo feature gates to compile nodes with tailored weight. Currently, both `syneroym-oltp` and `syneroym-olap` profiles utilize standard SQLite for operations and querying.
    > **Envisioned.** Not built yet. Embedding analytical query engines like DuckDB via SQLite-scanner for analytical profiles. Today both `syneroym-oltp` and `syneroym-olap` execute on standard SQLite.
  - **Database Isolation (One DB per Service):** The canonical primitive for structured state (backed by SQLite). Instead of a monolithic combined database, every stateful `SynSvc` gets a fully isolated, separate SQLite database file (`<service_id>.db`). The substrate also maintains its own separate database (`substrate.db`). This guarantees true concurrent write scaling across services, isolates failure domains, and allows per-service data lifecycles.
  - **Concurrency Model:** Designed for high throughput using a Single-Writer Thread / Multiple-Reader Pool architecture per database. A dedicated background writer task processes mutations sequentially from an in-memory queue, while concurrent readers execute in parallel across a connection pool.
    > **Envisioned.** Not built yet. SQLite WAL mode and advanced pragma tuning for per-service databases. Today per-service SQLite connections operate with a single background writer task without setting `PRAGMA journal_mode = WAL`.
  - **Resource Model:** Collections with lightweight schemas (loose enforcement of types, explicit indexed fields) containing JSON records. The data layer automatically injects a spoof-proof `creator_id` into every record.
  - **Schema Initialization (DDL):** Stateful `SynSvcs` export `init()` (first deploy) and `migrate()` (re-deploy) lifecycle hooks; within these hooks the guest runs plain SQL DDL (e.g., `CREATE TABLE`, `CREATE VIEW`, `CREATE INDEX`) through the gated `execute-ddl` host function — see [ADR-0007](decisions/0007-data-layer-wit-interface.md). Starting with plain SQL is safe for trusted services because each service owns an isolated database, and access is gated by IAM. Views defined during init are instantaneous (no write-lock penalty, unlike index creation).
    > **Envisioned.** Not built yet. Structured declarative data-model alternative restricting arbitrary DDL for untrusted third parties. Today services execute plain SQL DDL via the `execute-ddl` host function.
  - **Operations & Queries:** Full CRUD operations (`create_collection`, `put`, `patch`, `get`, `delete`, `delete_many`). It also supports `batch_mutate` for atomic transactions across multiple records. The query engine translates a MongoDB-style JSON filter document (equality, `$gt`/`$gte`/`$lt`/`$lte`/`$ne`, `$in`/`$nin`, `$regex`, `$and`/`$or`/`$not`, dot-notation paths — see [ADR-0007](decisions/0007-data-layer-wit-interface.md)) and an `AggregationPipeline` (for projections, `$group`, `$having`) into parameterized SQL queries with cursor-based pagination.
    > **Envisioned.** Not built yet. Native full-text search operators and aggregation pipelines over logical views. Today queries support structured JSON filters, and aggregation targets physical collections only.
  - **WASM Serialization & WIT Boundary:** The `syneroym:data-layer/store` WIT boundary supports nested record serialization and deserialization, passing complex JSON object graphs between WASM components and the host.

- **Object Service (Content-Addressed Blobs):**
  - **S3-Compatible Storage:** Dedicated blob storage for large media and software artifacts, natively content-addressed (keyed by SHA-256).
  - **Data Integration:** Blob hashes are stored as standard string fields in the Structured Data Service records.
  - **HTTP File Serving:** Built-in HTTP serving of public/private objects (with signed URLs), supporting static website hosting and CDN-friendly delivery directly from the blob store.

- **MQTT Event Service (Asynchronous Coordination):**
  - **Embedded Event Broker:** The substrate embeds an in-process MQTT broker (`rumqttd`) supporting standard MQTT semantics (wildcard topics `+` and `#`, retained messages) for asynchronous communication and device workflows.
    > **Envisioned.** Not built yet. Decentralized peer-to-peer MQTT topic-log replication and change notifications across nodes. Today event dispatch runs through the local in-process broker without multi-node log synchronization.

- **Universal Proxy (Inter-Component RPC):**
  - **Typed Interactions:** Developers use strongly typed WIT imports (`import acme:booking/service;`) rather than generic untyped APIs.
  - **Interception & Instance Mapping:** The Substrate injects a proxy host function during component instantiation to satisfy the WIT import. It resolves the generic import to a specific running `service_id` using dependency bindings pushed into service configuration by the App Supervisor ([ADR-0021](decisions/0021-binding-propagation-and-app-supervisor.md)).
  - **Protocol Translation:** The substrate traps the WASM call and dynamically proxies it to the target instance using JSON-RPC 2.0 over HTTP or WebSockets.
    > **Envisioned.** Not built yet. Binary wRPC serialization over Iroh QUIC streams. Today inter-service and external calls use JSON-RPC 2.0.
  - **Static Composition Bypass:**
    > **Envisioned.** Not built yet. Automatic static component composition bypassing substrate proxy interception. Today dependencies route through the Universal Proxy.

> **Implementation Design:** For technical details regarding the embedded MQTT broker and the Universal Proxy architecture, see [Feature Design: PLT-DAT](system-architecture.md#plt-dat-data-layer).

### [MSG-TOP] Scoped MQTT Pub/Sub Topic Namespace
The embedded MQTT broker strictly isolates topic namespaces by prefixing topics with the calling service's identifier (`svc/<service_id>/`).

- **Publish Isolation:** Outbound publish requests unconditionally prefix the calling service identity (`svc/<service_id>/<topic>`), blocking any service from publishing into another service's namespace or spoofing topic origins.
- **Subscription Scoping:** Inbound subscribe requests default to the calling service's namespace (`svc/<service_id>/<topic>`). Subscriptions support explicit cross-service opt-in only when the caller specifies a fully-qualified topic (`svc/<other_service>/<topic>`), preventing unintended message snooping.

### [PLT-ASY] Asynchronous Operations & Scheduling

The Asynchronous Operations component ensures reliable execution of offline interactions, long-running workflows, and periodic tasks, even in the presence of network partitions or transient service failures.

- **Resilient RPC & Retries:**
  - **Configurable Policies:** Retry policies (e.g., exponential backoff, maximum attempts) are defined at the service level by default, but can be overridden per-request.
  - **Dead Letter Queue (DLQ):** When the maximum retry limit is reached, retryable or outbox-backed messages are routed to a Dead Letter Queue for auditing, manual intervention, or later replay, preventing silent data loss. Non-idempotent synchronous calls fail directly unless the caller supplied an idempotency key and opted into queuing.

- **Offline Message Semantics & Outbox:**
  - **Substrate Durable Outbox Queue:** Offline requests are durably stored in an owner-local SQLite outbox queue on the substrate and periodically flushed when connectivity is restored ([ADR-0023](decisions/0023-durable-async-primitives.md)).
  - **Return Value Constraints:** Offline-capable calls cannot synchronously return data (e.g., a server-generated ID). Applications must rely on client-generated identifiers (e.g., UUIDs) or design the interaction to not require immediate server responses.
  - **Optimistic Local Execution (Fire-and-Forget):**
    > **Envisioned.** Not built yet. Client-side outbox queue and optimistic offline UI execution in browser clients. Today the durable outbox is substrate-side, and client calls execute synchronously over HTTP or WebSocket sessions.
    >
    > Clients can trigger operations using a fire-and-forget flag (or wrapper API) indicating it is "ok to send later". The client UI treats the request as optimistically successful.

- **Long-Running Tasks:**
  > **Envisioned.** Not built yet. Uniform execution and in-memory state management for long-running workflows composed of multi-stage compute tasks. Today Wasm guest functions run within bounded dispatch timeouts, and long-running task restart or compensation rules are not implemented.
  >
  > - **Uniform Execution:** Long-running tasks composed of multiple compute and service calls are supported uniformly (e.g., executed as standard Wasm functions).
  > - **In-Memory State Management:** The request to start the task is durably recorded, but the active execution state resides in the asynchronous engine's memory. If the process is interrupted, the task restarts from the beginning only when the task is idempotent or explicitly restartable; otherwise it fails and runs compensations rather than resuming from a mid-execution disk snapshot.

- **Periodic & Scheduled Tasks:**
  - **Supervisor Local Overlap Guards:** Cron and periodic triggers execute through the App Supervisor on its resident reconciliation pass ([ADR-0023](decisions/0023-durable-async-primitives.md) §6). The supervisor selects runnable target member instances based on health sweep reports and enforces local execution overlap guards without distributed leases or Registry calls.
  - **Delegated Task Dispatch:** When a schedule is due, the supervisor dispatches the execution command to the selected healthy member on its host substrate.

- **Compensating Transactions (Saga Pattern):**
  - **Saga Compensation Interfaces:** To handle permanent failures in distributed scenarios without leaving the system in an inconsistent state, services expose compensating functions prefixed with `saga-undo-<operation>` in their WIT interfaces ([ADR-0023](decisions/0023-durable-async-primitives.md) §7). Compensating functions take the forward call's original parameters plus its return value. Because step logs are written before forward calls execute, an undo may run for an operation that never completed.
  - **Automated Rollback & Triggers:** If a step in a multi-stage workflow fails permanently or exceeds its deadline, the orchestrator executes the corresponding compensating functions in reverse order for previously completed steps. Compensations fire only from an explicit guest request or an expired saga deadline, never from a queued task.

> **Design Rationale:** 
> - **Offline vs Pessimistic:** Not all operations make sense offline. Pessimistic locking (synchronous execution waiting for connection) remains the standard path. The fire-and-forget outbox is strictly an opt-in pattern for offline-capable operations.
> - **In-Memory vs Durable Execution:** While "Durable Execution" (saving the exact intermediate execution state to a DB, like Temporal) assists with idempotency, it is highly complex to implement within the Wasm host and still fails if there are strict time constraints between I/O steps. We instead trade platform complexity for explicit workflow definition—our in-memory approach requires that if a process crashes mid-task, the task is fully aborted and compensated (via `saga-undo-<operation>`) rather than resumed.
> - **Saga Arguments:** The compensating `saga-undo-<operation>` functions accept the identical arguments as the original forward operation along with the forward call's return value to precisely reverse the specific action.

### [PLT-RED] Service Redundancy
Service redundancy guarantees data durability, service continuity, and split-brain prevention for the Syneroym network, strictly prioritizing Consistency over Availability (CP) during network partitions.

Today a service runs on a single substrate with an isolated database. Stateless services support redundant replica placement, while application manifests reject replica configurations greater than one for stateful services. The system provides manual encrypted backup and restore, local in-process MQTT messaging, and S3-compatible blob storage.

- **Control Plane vs Data Plane Isolation:** The Data Plane must be fully decoupled from the availability of the Control Plane for already-known healthy routes. This is satisfied structurally rather than by cache-staleness rules: services hold their dependency bindings in their own configuration and resolve endpoints through the community registry, so no data-plane call consults the App Supervisor at all ([ADR-0021](decisions/0021-binding-propagation-and-app-supervisor.md)). While the supervisor is unavailable, existing data-plane routing, MQTT message flows, and HTTP access are unaffected; new deployments, promotions, quarantine decisions, and the propagation of binding changes pause or fail closed until it returns.

> **Envisioned.** Not built yet. Multi-node database replication, broker topic-log replication, peer-to-peer blob replication, and quorum-based failover are not built. Today stateful services run as single instances with manual backup and restore, and manifests reject replica counts above one for stateful services. Both Litestream and Iroh WAL shipping remain open options for stateful replication.

- **Configurable Stateful Replication:** The replication factor (e.g., N=1, N=2, N=3) is configurable at service deployment time via the application manifest. For replicated setups (N>=2), there is exactly one Primary accepting write operations, while Secondaries maintain an identical read-only state.
- **Low-Latency Streaming Replication:** Replication streams committed database changes directly from the Primary to Secondaries without relying on high-latency batching or third-party storage intermediaries for the live replication path. Both Iroh multiplexed stream WAL shipping and Litestream remain open replication architecture options. The replication layer must respect SQLite file and shared-memory invariants without mutating live `-wal` or `-shm` files out of band.
- **Automated Disaster Recovery Backups:** In addition to live node-to-node replication, the system periodically streams asynchronous backups to external S3-compatible object storage to enable cold starts and disaster recovery.
- **Pub/Sub Log Redundancy:** The `syneroym:messaging` pub/sub broker's topic log replicates to peer nodes using pull-based log replication over multiplexed streams. A replica maintains an up-to-date copy of the topic log and retained messages for durability and failover. Cross-node pub/sub access operates independently through standard RPC and native-dispatch routing to whichever node hosts the target service.
- **Peer-to-Peer Blob Storage Redundancy:** When an external S3-compatible backend is configured, the backend manages blob redundancy. In pure peer-to-peer deployments without external storage, the substrate replicates content-addressed blobs across peer substrate nodes according to manifest topology.
- **App Supervisor Topology Management & Manual Promotion:** The App Supervisor acts as the authoritative control plane for cluster membership and application topology. To prevent split-brain scenarios, the system avoids automatic failover: if a Primary fails, an operator manually deposes the primary and promotes an active Secondary via the App Supervisor.
- **Strict Quarantining & Routing-Level Fencing:** When a failed node is deposed, the App Supervisor marks its Node ID as `QUARANTINED` for the current topology epoch. Quarantined nodes cannot rejoin data-plane service under the same identity until an explicit operator recovery clears the condition. Ingress routing drops requests to quarantined nodes, and egress routing rejects outbound requests originating from quarantined nodes.

> **Implementation Design:** For technical details regarding the redundancy architecture and routing-level fencing, see [Feature Design: PLT-RED](system-architecture.md#plt-red-service-redundancy).

## Phase 3: Substrate & Application Lifecycle

### [LFC-MGT] SynApp Lifecycle Management
- Orchestrates the deployment, configuration, and monitoring of SynApps and their constituent SynSvcs across a decentralized network of substrates.
- **Application Manifests**: A SynApp is defined by a declarative manifest containing:
  - A list of required `SynSvc` instances (WASM components, Podman containers, native host services, or external TCP/HTTP services).
  - Explicit configurations for each service, including resource quotas and limits.
  - **Explicit Bindings**: First-class declarations of logical network dependencies (e.g., `requires: backend_api`). The orchestrator uses these to construct a dependency graph and resolve physical addresses *before* injecting them into the service's configuration.
- **Substrate Inventory**: The control plane maintains a user-defined inventory of target substrates, tracking their known capabilities to facilitate intelligent deployment scheduling. Target substrates enforce access control to ensure only authorized deployments are accepted.
- **Operational Modes**: Lifecycle management is supported via two distinct operational modes utilizing a shared set of core orchestration libraries:
  1. **CLI Standalone Mode (roymctl)**: Designed for **one-shot decentralized deployment**. The CLI reads a manifest, synchronously deploys services to available online substrates, and records an installation trace in a local SQLite database. It does not queue tasks for offline substrates. Drift from the desired state is resolved manually via a single-pass `reconcile` command.
  2. **Active Control Plane Mode (App Supervisor)**: An optional long-running component enabled as a substrate role on an owner or cluster's network. It accepts manifests via an API (the `supervisor` interface, which also carries the operator-facing status/alert read surface), stores desired state in its own SQLite database, and runs a continuous background reconciliation loop that watches the actual state of services across **multiple** substrates, retries deploy-time failures, monitors health, applies bounded remediation, raises alerts, and pushes updated dependency bindings into constituent services. It is not a global central coordinator, and it is not on the data path: nothing queries it to make a call. Direct `roymctl`-to-substrate deployment remains permanently supported — it is how a supervisor is stood up and the recovery path when one is unavailable.

### [TOP-DOC] Two-Tier Logical Name Resolution & Topology Documents

Multi-substrate application deployments resolve logical service names through a two-tier overlay: Tier 1 resolves the application instance master DID via the registry or DHT; Tier 2 fetches a canonical `TopologyDocument` signed by the application-instance master key from the App Supervisor (`supervisor.resolve`).

- **Topology Document Structure:** The signed `TopologyDocument` specifies the application instance DID (`app_did`), logical service name, topology mode (`singleton`, `redundant`, or `sharded`), ordered member service master DIDs, sharding strategy (such as range sharding or rendezvous hashing), topology epoch, generation, issuance timestamp, validity window (`not_after`), and suggested cache TTL (`cache_ttl_ms`).
- **Signature Verification & Caching:** Callers outside the application instance verify the document signature against the application master DID and route requests to member service DIDs. Member service DIDs resolve to transport endpoints via per-call community registry lookups. Any node may relay the signed document, and recipients cache it until `not_after`.
- **Authorization & Visibility:** The App Supervisor verifies caller capabilities (`supervisor/resolve`) before returning a topology document for private or internal services. Requests to unpublished services without valid authorization fail closed without disclosing whether the application exists. Services declaring open topology visibility (`topology_visibility = "open"`, such as public directory services) are resolvable without prior capability grants.

### [APP-DUL] Dual-Build Application Execution Model

SynApps support compiling from a single Rust codebase into either sandboxed `wasm32-wasip2` WebAssembly components executing within Wasmtime or statically linked native binaries in the substrate (`syneroym-app-host-native`), maintaining identical behavioral semantics across both execution modes.

- **Unified Host Trait Abstraction:** Host capabilities—including structured data storage, blob storage, conversation streams, HTTP routing, cryptographic signing, and service configuration—are defined as traits in `syneroym-app-host`.
- **Runtime Dual Targets:** The WebAssembly guest target implements host traits via `wit-bindgen` bindings to host WIT interfaces (`syneroym-wit-interfaces`). The native target (`syneroym-app-host-native`) implements the identical traits by dispatching directly to substrate host engines (`HostState`), eliminating WebAssembly sandboxing overhead for embedded services.
- **Behavioral Parity Testing:** Dual-build integration test suites execute application flows against both WASM components and native shims, asserting identical execution results across both runtimes.

### [LFC-VER] Versioning support overall

- **Substrate Upgrades:** Substrate binaries are updated via conscious, operator-driven actions rather than automatic, unverified polling. Binary rollback is managed via operating system service managers or container tags; the CLI does not provide an automated binary rollback command.
- **SynApp/SynSvc Compatibility:** SynApp compatibility is based on WASM interface capabilities rather than rigid Substrate versions. While a SynApp manifest may list Substrate versions as advisory "tested-on" metadata (similar to browser compatibility), the Substrate ultimately decides to accept deployment based on whether it can satisfy the required WIT host interfaces.
- **Service Upgrades & Migrations:** Stateful WASM services export `init()` (on first deployment) and `migrate()` (on re-deployment) lifecycle hooks executed before accepting traffic. The substrate executes these hooks with elevated data-layer capabilities (`data-layer/admin`) allowing SQL DDL schema execution. If a deployment fails, the control plane rolls back configuration generations, static asset bundles, and FDAE policies.
  > **Envisioned.** Not built yet. Automatic filesystem-level SQLite database snapshotting and automatic schema rollback. Today stateful services export init and migrate hooks without automatic database snapshots or rollback, and failed migrations require manual operator intervention.
  >
  > The system must support automatic filesystem-level snapshotting of service state (SQLite) before an upgrade. If the new service version fails to initialize or migrate its schema, the Substrate must automatically rollback to the snapshot and the previous WASM binary.
- **Network Compatibility:** Substrate peer-to-peer connections use fixed ALPN `syneroym/0.1`. Route preambles specify target service and protocol identifiers; callee nodes return typed unsupported-protocol errors when a requested protocol is not recognized. Persisted and signed formats carry explicit version fields.
  > **Envisioned.** Not built yet. Dynamic Capabilities/Protocol Matrix negotiation during connection handshakes. Today connections use fixed ALPN (syneroym/0.1) without dynamic protocol profile exchange.
  >
  > Multi-substrate communication relies on a dynamic Capabilities/Protocol Matrix. During the connection handshake, nodes negotiate their supported protocols (e.g., `["syneroym/rpc/v1", "syneroym/rpc/v2"]`). The core team deprecates older protocols deliberately on a case-by-case basis, avoiding the brittleness of a rigid sliding-window (N-x) policy.

## Phase 4: Advanced Services & Tooling

### [ADV-OBS] Observability enhancements

The substrate captures in-memory counters, gauges, and latency histograms via a thread-safe `MemoryRecorder` (`syneroym-observability`) and exposes a JSON snapshot over HTTP `GET /metrics`. Dedicated SQLite metric storage, automated data rollups, and multi-tenant access control are envisioned.

- **Comprehensive Metric Types:** Beyond system-level resources, track application metrics (service call counts, error rates, response times) in an in-memory metrics recorder.
  > **Envisioned.** Not built yet. Network byte metering, connection duration tracking, relayed byte accounting, and GPU or LLM token counters. Today the substrate tracks in-memory counters, gauges, and latency histograms without per-stream byte counters or external token metering.
  >
  > Track Network Metering (bytes transferred, connection durations, relayed byte amounts for multi-hop routing). The metric pipeline is extensible to support future infrastructure additions like GPU usage, LLM token counts, or specific AI service utilization.
- **Granularity & Retention:**
  > **Envisioned.** Not built yet. Persistent SQLite `metrics.db`, raw event logging, and automatic data rollups. Today metrics are stored strictly in memory and reset upon process restart.
  >
  > Implement automatic data rollups to balance storage costs. The system defaults to storing raw events for 24 hours, which then roll up directly into 1-hour buckets retained for 30 days (skipping minute-level granularity for simplicity and storage efficiency).
- **Data Mashups & Flexible Metadata:**
  > **Envisioned.** Not built yet. Structured metadata tagging with Substrate ID, Service Owner DID, and dynamic JSON billing metadata. Today in-memory metric keys use plain metric names without DID tagging or dynamic billing schemas.
  >
  > Metrics are tagged with Substrate ID, Service Owner ID (DID), and Datetime. The underlying format incorporates extensible JSON properties to support dynamic metadata, allowing for future additions such as applying "agreed rates" for billing without rigid schema coupling.
- **Access Control Enforcement:**
  > **Envisioned.** Not built yet. Role-based metrics access control, per-service owner scoping, and relay billing logs. Today the metrics HTTP endpoint is unauthenticated and serves all recorded metrics.
  >
  > Substrate owners have root access to all metrics on their node. Service owners are restricted to viewing metrics exclusively for their deployed SynApps/SynSvcs. Relay providers have access to routing byte counts to log charges against source/destination nodes.
- **External API Strategy:** Substrates do not render internal dashboards. Instead, metric data is exposed as a JSON snapshot via HTTP `GET /metrics`.
  > **Envisioned.** Not built yet. Access-controlled RPC metric endpoints and dedicated metering visualization applications. Today the substrate exposes an unauthenticated local HTTP endpoint returning JSON metrics snapshots.
  >
  > Metric data is exposed securely via an access-controlled RPC endpoint, designed to be consumed by external visualization SynApps or dedicated metering applications.

### [OBS-ALT] Control-Plane Health Sweep & Alert Store

The substrate App Supervisor and operator tools maintain an isolated SQLite `AlertStore` (`alerts.db`) tracking discrete health failure modes (`AlertKind`).

- **Failure Mode Classification (`AlertKind`):** The store tracks distinct failure signals without collapsing them into generic errors:
  - `SubstrateUnreachable`: Target substrate failed to respond to the health poll.
  - `InstanceNotRunning`: Target substrate answered, but the service instance is down or absent.
  - `ProbeFailing`: The declared readiness probe for the service fails.
  - `CertificateNearExpiry`: Instance certificate is within 25% of its expiration window.
  - `CertificateExpired`: Instance certificate validity window has expired.
  - `SupervisorSuperseded`: A managed substrate reports a generation higher than this supervisor holds.
  - `RemediationExhausted`: Bounded restart-in-place exhausted maximum restart attempts without becoming healthy.
  - `BindingConflict`: A binding write arrived at the current epoch with conflicting content.
  - `PlacementChangeRefused`: A plan update attempted to relocate an existing service to a different substrate.
  - `OrphanedService`: A service continues running despite removal from the stored plan.
  - `VaultLocked`: Supervisor vault lacks the Key Encryption Key (KEK) needed to reissue instance certificates.
  - `InstanceRevoked`: An operator revoked the placement's instance key.
  - `RotationRestartPending`: Certificate renewal succeeded but the subsequent instance restart failed.
  - `DeliveryExhausted`: A queued binding write exhausted its delivery attempt budget.
  - `ScheduledRunFailed`: A scheduled cron tick failed or timed out.
  - `AppIdentityMismatch`: Vault app master key does not derive the expected application DID.
- **Lifecycle Tracking:** Alert records persist `first_seen_at`, `last_seen_at`, and `cleared_at` timestamps. Alerts remain active while failure conditions persist, and transition to cleared once the underlying fault resolves.

### [OBS-PRB] Diagnostic Health Probes & Operator Health CLI

The SDK and operator CLI provide read-only multi-substrate status polling and alert reporting without modifying deployment state.

- **Status Queries (`StatusQuery`):** The substrate client implements the `StatusQuery` trait (`crates/sdk/src/health.rs`) to query service statuses across target substrates, reporting instance phases, probe results, and node facts.
- **Operator Health CLI (`roymctl app health`):** Operators audit application instance health via `roymctl app health --instance-id <id>`. The command polls target nodes, evaluates readiness probes and certificate lifetimes, records active alerts in `alerts.db`, and exits non-zero if any service reports a fault. The `--watch <secs>` option enables periodic polling, and `--no-record` allows read-only inspection without persisting alert rows.
- **Alert Inspection (`roymctl app alerts`):** Operators inspect recorded alerts for an application instance via `roymctl app alerts --instance-id <id>`. The command displays active alerts and can include cleared alerts with `--all`.

### [ADV-AI] Advanced AI & Agentic Workflows

All advanced AI capabilities and concierge agent workflows are deferred and tracked in `docs/planning/deferred-backlog.md`. No AI engine, local inference service, MCP gateway, or vector database exists in the codebase today.

- **Local Model Inference Service:**
  > **Envisioned.** Not built yet. Local model inference service and Ollama runtime management. Today the substrate runs WASM services and native services without AI engine wrappers.
  >
  > A lightweight wrapper service within the substrate that manages the underlying AI engine (e.g., Ollama). It orders the engine to install/download specific base models from a strict allow-list defined by the node operator, and proxies inference calls to the desired agent/model combination. Supports dynamic model loading within the permitted list and is accessible by other `SynApp` services via the Universal Proxy.
- **Hardware-Gated Capabilities & Decoupling:**
  > **Envisioned.** Not built yet. GPU/NPU hardware detection gating and decoupled remote LLM inference routing. Today the substrate samples basic host CPU and RAM in `crates/observability/src/engine.rs`.
  >
  > Local model installation and inference are strictly gated by automatic hardware detection (e.g., GPU/NPU availability, RAM capacity) and explicit owner configuration overrides. Crucially, the Agent logic (lightweight WASM) and the LLM inference (heavy compute) are fully decoupled. If the local node lacks hardware for LLMs, it can still run the Concierge Agent locally while routing just the LLM inference requests to capable remote substrates. Alternatively, it can outsource both the Agent and the LLM entirely. The proxy agent service explicitly configures these upstream/remote endpoints to avoid any inverted dependency on application-layer aggregators.
- **The Concierge Agent (Rig-core):**
  > **Envisioned.** Not built yet. Native concierge agent and natural language intent pipeline. Today user interaction uses structured UI screens and Action Cards.
  >
  > The core agentic `SynSvc` running natively on the substrate. Frontend clients (like Trusted Rooms) send natural language intent directly to this agent.
- **Dynamic Tool Retrieval Loop:**
  > **Envisioned.** Not built yet. Dynamic tool retrieval loop with `search_ecosystem_tools` and semantic vector search. Today services are discovered via directory listings and explicit RPC method calls.
  >
  > The agent uses a dynamic "Retrieval Augmented Tool" approach to avoid context bloat:
  > 1. The loop starts by giving the LLM exactly **one** meta-tool: `search_ecosystem_tools`.
  > 2. The LLM calls `search_ecosystem_tools(query)`.
  > 3. The Concierge Agent executes a semantic search against its local Ecosystem Vector Directory.
  > 4. The agent dynamically injects matching tool schemas into the LLM's context.
  > 5. The LLM selects the best tool and generates the execution command.
  > 6. The agent invokes the target service via the Universal Proxy, returning **Action Cards**.
- **Human-in-the-Loop (HITL) Consent:**
  > **Envisioned.** Not built yet. Proposal Cards and agent execution pause-and-yield loops. Today user consent occurs via interactive UI screens and form submissions.
  >
  > For high-stakes tool calls, the Concierge Agent pauses execution and yields a "Proposal Card" to the Trusted Room. The user must cryptographically sign (consent) before the loop resumes. This is configured natively via tool arguments.
- **Agent Observability (Progress Streaming):**
  > **Envisioned.** Not built yet. Agent progress streaming and citation broadcasting. Today observability captures operational metrics and control-plane alerts.
  >
  > Configurable progress streaming where the Concierge Agent broadcasts structured status, tool calls, citations, and validation events back to the UI. Raw private model reasoning is not exposed as an application contract.
- **MCP Gateway:**
  > **Envisioned.** Not built yet. Model Context Protocol (MCP) server or gateway. Today external clients interact through JSON-RPC and HTTP endpoints.
  >
  > A headless gateway layer that exposes the substrate's local capabilities to *external* desktop clients using the Model Context Protocol (MCP).
- **Agent-to-Agent Delegation:**
  > **Envisioned.** Not built yet. Autonomous agent-to-agent negotiation protocol. Today inter-service communication uses typed WIT interfaces and RPC routing.
  >
  > The capability for a user's Concierge Agent to autonomously negotiate with external provider agents across the Syneroym substrate.
- **Ecosystem Vector Directory & Memory:**
  > **Envisioned.** Not built yet. Local `sqlite-vec` vector database and episodic agent memory. Today SQLite stores relational service records and deployment metadata.
  >
  > A specialized local data store (`sqlite-vec`) indexing available tools and storing episodic memory.
- **Orchestrated Loopcraft (Nested Agentic Loops):**
  > **Envisioned.** Not built yet. Nested Loopcraft agent methodology and specialized WASM critic sub-agents. Today WASM services execute single-invocation request/response and event workflows.
  >
  > To ensure high reliability on complex tasks, the Concierge Agent employs the "Loopcraft" methodology. Rather than executing a single, flat ReAct loop, the agent network utilizes pre-defined, specialized loops (e.g., a "Data Gathering Loop," a "Synthesis Loop," or a "Verification Loop"). These specialized loops are compiled and deployed as independent native WASM components (`SynSvcs`). The core Concierge Agent conditionally routes tasks through these stacked loops via the Universal Proxy based on the problem state. For instance, drafting an Action Card for a financial transaction will strictly route through a Verification Loop (a Critic sub-agent) before presenting it to the user. This orchestrated nesting allows for deep, self-correcting reasoning while maintaining strict zero-trust isolation between loops.

### [ADV-DEV] SynApp Developer Tooling & SDKs
- **Transparent Developer Experience**: Rather than providing a rigid CLI wrapper, Syneroym development embraces transparent, standard Rust tooling. Project templates (via `cargo generate`) are provided to set up standard `Cargo.toml` files and build scripts. This ensures compatibility with existing IDEs, Language Servers (LSP/rust-analyzer), and agentic coding tools.
- **Local Substrate for Integration & Dev**: To ensure zero-drift execution, developers use an actual local Syneroym node for local integration testing and development. Operations and developer workflows use existing `roymctl` subcommands (`claim`, `app`, `kek`, `supervisor`, `substrate`, `svc`). This completely avoids the massive engineering effort of duplicating the WASMTIME host, SQLite, and network logic into a standalone developer SDK. The SynApp couples to the Substrate entirely through standard WIT interfaces, not compile-time bindings.
- **Pure Mock SDK for Unit Testing**: We provide a minimal `syneroym-dev-sdk` intended exclusively for isolated unit testing. This SDK provides simple, purely in-memory mock implementations of the Substrate interfaces (Data, Blobs, AI). Developers can write `#[test]` functions that link against these mocks for fast, offline verification of application logic without needing to spawn a real Substrate node.

## Phase 5: Peer-to-Peer Community Primitives

Because Syneroym can be used as a general open cloud, this section separates foundational low-level network connectivity (`[TOP]`) from higher-level community-driven peer networking primitives.

### [P2P-DSC] Distributed Matching Fabric
Discovery uses directory services (`syneroym-roym-directory`) where clients, SynOrgs, and directories each choose what they query and publish. Providers publish signed publication records (listings and profiles) to chosen directories, and clients fan out queries across designated directory sources, deterministically merging and verifying results locally.

- **Publications, not a global index:** Providers, consumers, and services publish signed publication records (`SignedRecord` envelopes carrying listings, profiles, and intents). Client applications query designated directory services with bounded fan-out, deterministically merge results, and verify cryptographic signatures, timestamps, and validity windows before use.
  > **Envisioned.** Not built yet. Fully decentralized P2P index caches across arbitrary peer nodes. Today discovery uses client queries fanned out across designated directory SynOrgs, with client-side verification and merging.
  >
  > Distributed matching index caches across peer substrates, where indexes are distributed non-authoritative caches and every result is client-verified before use.
- **Deterministic placement:**
  > **Envisioned.** Not built yet. Deterministic routing schema and rendezvous hashing onto leaf index shards. Today providers publish directly to chosen directory SynOrg services.
  >
  > A protocol-defined Routing Schema (spatial cell, category, and attributes) plus rendezvous hashing maps each publication onto leaf index shards. Providers compute their own placement without coordinators.
- **Aggregators as directory services:** Directory applications ("Aggregators") operate as directory-type SynOrgs (`syneroym-roym-directory`) indexing provider listings and credentials without privileged substrate status. No aggregator is a required or privileged intermediary.
  > **Envisioned.** Not built yet. Leaf index shard opt-in and federation protocol. Today directory services run as standard SynApps, and clients configure which directory sources they query.
  >
  > Directory applications can opt in as leaf index shards in a decentralized matching fabric, presenting the identical standard interface as any other peer.
- **Hierarchical synopsis trees and query planning:**
  > **Envisioned.** Not built yet. Hierarchical synopsis trees, query planners, and cross-shard ranking. Today directories execute local SQL searches and clients merge results.
  >
  > A hierarchical synopsis tree and query planner (when leaf-shard count makes flat lookup expensive), composite routing descriptors, and cross-shard ranking layer on top without reworking the publication or placement contract.

### [P2P-REP] Peer Reputation & Trust

**Principles.** The reputation design is not frozen. It will be frozen later. Only these principles are fixed today: reputation is decentralized, reliable, transparent, and under the owner's control of what is shared.

**Built today.** Trust evidence relies on portable signed records and bilateral independent interaction receipts, rather than public ratings or reputation scores. Commercial transactions generate signed agreement receipts and fulfilment receipts, where each party signs its own independent attestation without requiring a joint multi-party transaction.

- **Coarse-Grained Satisfaction Signal:**
  > **Envisioned.** Not built yet. Coarse-grained numerical satisfaction scoring (0=Poor, 1=Decent, 2=Great). Today Roym produces no public numerical ratings or scores.
  >
  > Reputation is implemented as a low-resolution scale (e.g., 0=Poor, 1=Decent, 2=Great) to minimize cognitive load and mathematical complexity.
- **Cryptographic Tying & Bilateral Receipts:** Interaction agreements produce bilateral receipts (`AgreementReceiptPayload` and `FulfilmentReceiptPayload`), where each party signs and stores its own attestation in its own ledger without joint multi-party ceremonies.
  > **Envisioned.** Not built yet. Satisfaction signals referencing interaction receipts. Today bilateral receipts record completed agreements and fulfilments without attached review signals.
  >
  > To reduce unsolicited review spam, a verified-interaction satisfaction signal references a mutually signed interaction receipt. Other feedback is labelled separately. Receipt tying does not prevent collusion, coercion, selective disclosure, or identity farming.
- **Time-Decay:**
  > **Envisioned.** Not built yet. Time-decay algorithms and Exponential Moving Average (EMA) scoring formulas. Today no score calculations or decay mechanisms exist.
  >
  > A peer's reputation naturally decays to the center ("decent") over time, prioritizing recent interactions over historical legacy.
- **Incremental Rolling Summaries:**
  > **Envisioned.** Not built yet. Substrate-side rolling summary aggregations and moving averages. Today client nodes inspect individual signed records and credentials directly.
  >
  > The substrate runs continuous, compute-light incremental aggregations (e.g., updating total user counts, moving averages, and maintaining a small, tiered summary paragraph based on timeframes). This avoids the need to process heavy LLM summarization on massive blocks of raw text.
- **Portable Trust Evidence:** Trust evidence is not pushed to a global public DHT. Providers and SynOrgs serve portable signed records (membership credentials, revocations, and bilateral receipts), and client applications preserve provenance and verify signatures against pinned group sources. Provider hosting does not imply that the presented set is complete.
  > **Envisioned.** Not built yet. Joint DHT reputation records and public reputation score distribution. Today trust evidence consists of individual signed records and receipts verified by the recipient.
  >
  > Reputation signals and evidence are shared without a global public DHT, allowing clients to compare guild, consumer-held, or other authorised sources.

## Phase 6: High-Level Applications (SynApps)
Roym is the one SynApp built so far. It unites directory discovery, catalog listings, bookings, encrypted messaging, professional guilds, and trust verification into actor-centric workflows. Separate standalone mini-apps (such as independent Ledger or Marketplace applications) are not built.

### The Syneroym Hub (Core Client Application)
*The universal, multi-surface shell that connects the user to their local substrate and orchestrates all ecosystem activities.* The Hub is implemented as a browser web application and progressive web application (`crates/roym_web/ui`), served by the client gateway over HTTP and JSON-RPC (`POST /rpc`).

- **The Personal Data Homebase:** An interface managing the user's digital identity session, transaction history, and encrypted backups.
  > **Envisioned.** Not built yet. Active FDAE access grant management interface. Today identity sessions and backups are managed via the web UI and roymctl.
  >
  > A secure vault interface managing the user's digital identity, portable service history, and active FDAE access grants.
- **The Trusted Room Inbox:** A unified messaging view combining human-to-human social chats, professional guild groups, and interactive business-to-consumer service threads.
- **The Agentic Concierge (optional):**
  > **Envisioned.** Not built yet. Text or voice AI concierge. Today all Hub interactions use deterministic UI workflows without AI models.
  >
  > A text/voice interface powered by a local or user-chosen AI service. It remains subordinate to deterministic workflows, explicit consent, and a fully usable non-AI path.
- **The Opportunity & Discovery Radar:** Directory search supports geographic radius filtering (`--near`) to discover local providers and listings.
  > **Envisioned.** Not built yet. Visual map-based radar, mesh network browsing, and real-time opportunity streams. Today discovery uses directory search with text and radius parameters.
  >
  > A visual, map-based interface to browse local mesh networks, assess neighborhood trust proximity, and consume localized opportunity streams.
- **The Web Client and Action Card Renderer (`[HUB-APP]`):** A lightweight client running in a browser that contains zero business logic or private keys, acting as a renderer for versioned JSON Action Cards and communicating with the substrate over JSON-RPC.
  > **Envisioned.** Not built yet. Desktop native shells (e.g. Tauri) and native mobile shells. Today the Hub runs as a web application served by the substrate client gateway.
  >
  > A cross-platform native shell containing zero core business logic, acting purely as a thin renderer for JSON Action Cards securely pushed by the underlying local Substrate node.

### Everyday Users (Consumers)
*Activities focused on social connection, discovering services, secure negotiation, and seamless payment.*
- **Social & Group Messaging:** Chat with friends, family, or local community groups using an encrypted messaging interface to share messages, media, and recommendations.
- **AI-Assisted Discovery:**
  > **Envisioned.** Not built yet. AI assistant searching community directories. Today consumers search directories directly through text queries and location radius filters.
  >
  > Ask a personal AI assistant to find local services (like a plumber or doctor) by automatically searching community directories and understanding what each provider offers.
- **Service Bundling:**
  > **Envisioned.** Not built yet. Multi-service bundling into a single coordinated request. Today each service request is created and agreed independently.
  >
  > Combine multiple services into a single request (e.g., ordering food from a restaurant and requesting a separate delivery driver to pick it up).
- **Interactive Negotiation:** Chat directly with service providers in secure rooms to discuss details, negotiate prices, and instantly approve interactive Quote Cards dropped into chat.
- **Flexible Payments:** Finalize services by approving versioned payment request and acknowledgement cards. External or out-of-band rails remain the default until a separately validated ledger product is approved; every method identifies the responsible payment or settlement provider.
- **Portable Data & Privacy:** Export and import encrypted identity and service history archives when switching devices or providers.
  > **Envisioned.** Not built yet. Time-limited selective record sharing through consumer FDAE interfaces. Today full encrypted backups can be exported and restored.
  >
  > Securely share personal information (like medical records or delivery addresses) with a provider for a limited time, and seamlessly take your service history with you if you switch providers.
- **Trust Context:** Inspect sourced credentials, referrals, receipts, and community context without exposing private social graphs or implying a universal objective score.

### Service Creators (Primary Providers)
*Activities focused on setting up shop, generating leads, and delivering services.*
- **Digital Storefront Setup:** Create a business profile and publish service listings and catalogs to discovery directories.
- **Advertising & Outreach (evidence-gated):** Any cold outreach or paid placement requires recipient controls, rate limits, disclosure, and community policy. Digital stamps are one later hypothesis, not the default solution.
- **Lead Engagement:** Receive customer requests and respond by dropping interactive quote Action Cards directly into customer chat.
  > **Envisioned.** Not built yet. Network-wide live feed of public customer requests. Today providers receive requests sent directly to them by consumers.
  >
  > Browse a live feed of local customer requests and respond by dropping interactive forms, booking widgets, or quotes directly into the customer's chat.
- **Service Delivery & Billing:** Deliver services and push payment request cards into chat.
  > **Envisioned.** Not built yet. Automated debt-clearing ledgers and integrated external payment gateways. Today payment requests identify accepted settlement methods, and payments settle out-of-band.
  >
  > The invoice can feed directly into the network's automated debt-clearing ledger or route through traditional payment gateways depending on business configuration.
- **Professional Guilds:** Join private group chats with other professionals in your industry to share work, refer clients, and coordinate projects.
- **Reputation Building:** Collect portable signed agreement receipts, fulfilment receipts, and SynOrg membership credentials that preserve provenance.
  > **Envisioned.** Not built yet. Public feedback collection and review systems. Today trust evidence consists of cryptographic receipts and membership credentials.
  >
  > Collect portable, receipt-linked feedback and other signed trust evidence that preserves provenance when hosting changes.

### Network Enablers (Aggregators & Facilitators)
*Activities focused on making the market run smoothly, providing infrastructure, and resolving disputes.* An Aggregator is a directory-type SynOrg service (`syneroym-roym-directory`) that aggregates provider listings and credentials. It is not a hosting provider.

- **Discovery Directories (Aggregator):** Run search services and community directories that collect and index provider listings, making it easy for users to find services.
- **Spam Prevention (Aggregator):** Enforce disclosed publication rate limits, recipient block controls, and abuse response policies. Fuel quotas and economic costs remain optional mechanisms.
- **Trust Summaries (Aggregator):** Serve sourced membership credentials and verify trust records under a declared community policy.
  > **Envisioned.** Not built yet. Computed opinion scores and automated trust summary ratings. Today directory services return verified membership credentials and revocations without computing numerical scores.
  >
  > An aggregator's output is an opinion or computation, not a guaranteed reliable truth score.
- **Financial Gateways (Facilitator):**
  > **Envisioned.** Not built yet. Specialized financial services converting digital credits to fiat currency or integrating automated tax and accounting ledgers. Today payments are recorded out-of-band between parties.
  >
  > Provide specialized financial services, like converting digital network credits into traditional fiat currency (bank money), or automating tax and accounting records by plugging directly into a provider's local ledger.

## Phase 7: Edge Expansion

### [EDG-MOB] Mobile operation 
- **Platform Support:** Consumer and provider interfaces provide responsive web access for mobile viewports. Substrate binaries target Linux, macOS, and Windows.
  > **Envisioned.** Not built yet. Native Syneroym Substrate execution on Android and iOS. Today mobile devices access substrate services through the web client in a browser.
  >
  > Native substrate execution on Android and iOS must prove acceptable reliability, battery use, background behaviour, and key recovery on supported OS versions.
- **Background Execution & Throttling:** Substrate communication uses durable outbox queuing and retry mechanisms to handle intermittent connectivity.
  > **Envisioned.** Not built yet. Silent push wake-ups (APN/FCM) and mobile OS background window scheduling. Today the client gateway and substrate communicate over active HTTP/WebSocket sessions.
  >
  > For urgent requests, clients can optionally send an out-of-band push notification (APN/FCM) to silently wake the suspended mobile app. The woken app processes requests locally but defers outbound network responses until the mobile OS schedules a background task window.
- **Hardware Security (TPM 2.0 Equivalent):** Substrate cryptographic keys are stored in encrypted software keystores with memory protection. Guest components access signing only through host WIT calls (`syneroym:signing`).
  > **Envisioned.** Not built yet. Unified `SecureStorage` and `KeyManagement` WIT abstractions and host bridges to Android StrongBox, iOS Secure Enclave, and Linux TPM 2.0. Today cryptographic operations use software Ed25519 keys managed by the substrate host.
  >
  > Unified `SecureStorage` and `KeyManagement` WIT abstractions for SynApps, mapped by host implementations to Android StrongBox, iOS Secure Enclave, and Linux TPM 2.0.

---

## Appendix: Later-Phase Additions

These additions define envisioned extensions to the substrate and application contracts. They are tracked as a running list in [deferred-backlog.md](planning/deferred-backlog.md).

### [APP-A11Y] Accessibility and Localisation

- **Localisation:** `ProfilePayload` supports an optional `locale` field (`Option<String>`), with default `en-US`.

> **Envisioned.** Not built yet. Complete internationalisation (i18n) translation frameworks, resource bundles, and non-English UI translations. Today the Roym Hub UI contains hardcoded English text.
>
> The architecture must be internationalisation-ready (i18n) to support local community clusters with translated interfaces.

- **Accessibility:**

> **Envisioned.** Not built yet. The Roym Hub UI has no ARIA markup, no screen reader testing, and no WCAG 2.1 AA audit.
>
> Base substrate capability flows (onboarding, recovery, Hub UI) must target WCAG 2.1 AA to ensure independent operation by disabled users.

### [APP-IOT] Non-IP Mesh Transport Interconnectivity

- **IoT and Edge Networking:**

> **Envisioned.** Not built yet. Configuration fields `parent_coordinator.ble` and `parent_coordinator.lora` exist in `crates/core/src/config/base.rs` as stubs, but no network transport reads them.
>
> The system must support integrating non-IP mesh networks (e.g., Zigbee, Thread, Bluetooth Low Energy (BLE), LoRa) into the IP-based topology.

### [APP-ESC] Escrow, System Coins, and Mutual Credit

- **Payment Records:** Payments are recorded through signed out-of-band payment requests and acknowledgements (`crates/roym_core/src/payment.rs`).
- **Escrow:**

> **Envisioned.** Not built yet. External or out-of-band settlement is the only payment mechanism today. No escrow custody or dispute hold exists.
>
> Third-party or multi-signature custody of funds pending service completion or dispute resolution.

- **System Coins and Mutual Credit:**

> **Envisioned.** Not built yet. Syneroym has no blockchain token, cryptocurrency, or ledger coin.
>
> A native ledger token and bilateral IOU mutual credit system layered onto the Payment Abstraction Layer.

## Appendix: Substrate Feature Coverage Matrix
*(Validating core platform primitives across the Roym application suite and substrate runtime)*

| Substrate Capability | Primary App | How it is exercised |
| :--- | :--- | :--- |
| **[TOP-*] Routing & Relays** | **Substrate Core** | Establishing secure P2P connections across NATs and resolving cryptographic node IDs (`crates/router`, `crates/coordinator_iroh`). |
| **[PLT-ASY] Offline Operation** | **Roym Conversation** | Outbox message queuing and causal DAG syncing upon reconnection (`crates/conversation`, `crates/roym_conversation`). |
| **[PLT-DAT] Conversation DAG** | **Roym Conversation** | End-to-end encrypted messaging with Double Ratchet and causal DAG ordering via `syneroym:conversation` (`crates/conversation`). |
| **[PLT-DAT] Pub/Sub** | **Substrate Event Bridges** | Event notification delivery through the embedded MQTT broker (`crates/mqtt_broker`). |
| **[PLT-DAT] S3 Blobs** | **Roym Catalog** | Storing and retrieving content-addressed blobs via `blob-store` WIT capability and `crates/data_blob`. |
| **[FND-IDT/IAM] Identity & Access** | **Roym Hub / roymctl** | Generating root keypairs and enforcing authorization via `ControllerAgreement`, App Supervisors, and UCAN / FDAE policies (`crates/identity`, `crates/fdae`). |
| **[FND-CFG] Service Config (Secrets)** | **Roym Services** | Dynamically retrieving secrets from the encrypted vault via `syneroym:vault/reveal` (`crates/sandbox_wasm`). |
| **[FND-DEP] App Deployment** | **roymctl** | Compiling and deploying WASM SynApp components into the sandboxed Wasmtime runtime (`apps/roymctl`, `crates/app_orchestration`). |
| **[FND-VER] Schema Migrations** | **Substrate Runtime** | Executing stateful `init()` and `migrate()` SQL DDL lifecycle hooks (`crates/sandbox_wasm/src/engine/lifecycle.rs`). |
| **[P2P-DSC] Directory Search** | **Roym Directory** | Publishing signed listing records and resolving local service providers via directory query and deterministic client-side merging (`crates/roym_directory`). |
| **[P2P-REP] Bilateral Receipts** | **Roym Transaction** | Generating signed bilateral agreement and fulfilment receipts and rendering credential trust summaries in Roym Hub (`crates/roym_core/src/transaction.rs`, `crates/roym_web/ui`). |
| **[FND-OBS] Metrics Recording** | **Substrate Core** | In-memory metrics recording with `MemoryRecorder` exposed over HTTP `/metrics` (`crates/observability`). |
| **[LFC-*] Deploy Rollback** | **Control Plane** | Atomic deploy-time rollback of configuration generations, asset bundles, and FDAE policies upon deployment failure (`crates/control_plane`). |

> **Envisioned.** Not built yet. The following matrix targets describe planned capabilities not implemented in code:
>
> - **Typing Presence:** Ephemeral typing presence indicators in Roym Conversation. Today conversation delivers durable, causal messages only.
> - **Listing Media Blobs:** Storing and streaming high-resolution images and videos for catalog listings. Today listings store structured JSON text in SQLite.
> - **Immutable Ledger Blocks:** Native ledger application, immutable blocks, and content-addressed transaction receipts (`[PLT-DAT]`). Today transaction receipts are stored in service SQLite tables.
> - **AI Chat Participants:** AI participants with long-term memory and vector stores in group chats (`[APP-AGI]`). Today Roym has no AI runtime or vector store.
> - **AI Delegation & Trusted Rooms:** FDAE-governed AI delegation and room-level policy enforcement in conversation (`[FND-IAM]`). Today chat security uses symmetric epoch keys and DAG membership.
> - **Credit Network Sharding:** High availability and partition tolerance for decentralized credit networks (`[PLT-RED]`). Today multi-node database and broker replication are deferred.
> - **TPM 2.0 Hardware Signing:** Hardware-backed multi-party signing for high-value settlements (`[FND-SEC]`). Today all signing uses host software Ed25519 keys.
> - **Zero-Downtime Settlement Upgrades:** Zero-downtime stateful settlement rule upgrades (`[FND-VER]`). Today migrations execute SQL DDL hooks during deployment.
> - **Dynamic Node Leasing:** Dynamically leasing external substrate nodes to handle flash traffic spikes (`[FND-LEA]`). Today substrate resources use static local host configurations.
> - **Resource Utilization Billing:** Tracking exact resource utilization to bill storefront owners (`[FND-OBS]`). Today metrics record in-memory operational counters without billing logs.
> - **External Gateway Secret Integrations:** Dynamically pulling third-party shipping and fiat payment gateway API keys from the vault (`[FND-CFG]`). Today services retrieve internal service credentials only.
> - **Mesh Matching Fabric:** Placing and resolving signed publications across a rendezvous-hashed leaf shard mesh (`[P2P-DSC]`). Today discovery uses direct queries to directory SynOrgs.
> - **Peer Reputation Scoring:** Peer reputation scoring algorithms, time decay formulas, and rolling trust summaries (`[P2P-REP]`). Today trust is verified via signed SynOrg credentials and bilateral receipts.
> - **Automated Version Rollback:** Automated rollback of failed application version updates and SQLite filesystem snapshots (`[LFC-*]`). Today rollback is limited to control-plane deploy-time metadata.
> - **Mobile Push Wakeups:** Waking suspended iOS or Android clients via out-of-band push (APN/FCM) to receive incoming quotes (`[EDG-MOB]`). Today the Hub is a web application running in a browser.
