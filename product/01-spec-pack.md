# Spec Pack: Identity-Native User-Space Peer Substrate

## 1) Narrative + user outcomes

### Narrative
This system is a user-space **substrate** that lets independently owned services
interact directly as peers without depending on central authorities, global
consensus, or permanent infrastructure. The substrate is an application that can
ship as a CLI, service, or mobile/desktop app. The overall system is the
**Substrate**, which runs as a **substrate instance** on each host/node and can
enable distinct roles (host control, service control, signaling, data relay,
proxy). A **peer** is a
uniquely addressable component, typically a micro-app/service. The **host owner**
installs and controls a substrate instance on a **host/node** (a logical machine
or subset of machine resources). The **service owner** operates a substrate
instance with service control enabled to deploy and manage peers across hosts.
Additional roles (signaling, relay, proxy) may be enabled as needed. End users
consume system services by interacting with peers; peers may also interact with
other peers.

Each peer is defined by a self-owned cryptographic identity. Discovery, trust
establishment, intent expression, and failure handling are intrinsic, not
bolt-ons. The substrate is designed for heterogeneous and adversarial
environments where connectivity is intermittent, transport types vary (including
non-IP), and trust is local and explicit.

This spec pack stays at a product/architecture boundary: it defines actors,
artifacts, and end-to-end behavior without prescribing internal APIs or wire
formats.

The substrate enables peers to cooperate and host portable micro-apps for one
another under explicit, revocable permission. It continuously reconciles
declared intent with observed reality, locally and cooperatively, without a
central control plane. The system prioritizes privacy by default, minimizing
metadata leakage, and maintains operational simplicity through uniform
shrink-wrapped components with capability gating rather than bespoke builds.

### User outcomes
- **Direct peer cooperation without central services**: Users can discover and
  interact with peers using identity, not addressability, even when discovery
  hints are absent, stale, or compromised.
- **Verifiable interactions**: All externally obtained information is
  cryptographically verifiable; trust is relationship-scoped and explicit.
- **Resilience under failure**: Peers remain functional across disconnections,
  partitions, and churn; local reconciliation keeps intent and observed state
  aligned over time without global coordination.
- **Portable services**: Micro-apps can be deployed, moved, suspended, resumed,
  and removed across peers with explicit consent, without implying trust
  escalation or ownership transfer.
- **Privacy-respecting operation**: Identity and intent disclosure are explicit
  and contextual; metadata leakage is minimized by default.
- **Low operational burden**: A uniform component set runs across deployments;
  differentiation is via capability gating, not custom builds or privileged
  infrastructure.

---

## 2) Functional scope (in/out)

### In scope
- **Identity-native peer model**: Peers (micro-apps/services) have self-owned
  cryptographic identities; identity is primary over network location.
- **Host/substrate model**: Peers run on host/nodes managed by a substrate
  instance with host control enabled; service owners use a substrate instance
  with service control enabled to deploy and manage peers.
- **Cryptographic verification**: All externally obtained data is verified; no
  trust by position or mediation.
- **Decentralized discovery**: Best-effort, hint-based discovery with optional
  non-authoritative registries (initially DNS-based, PKARR, and out-of-band
  tokens); clients verify all discovered data. Phase 1 discovery fallback order:
  OOB token → PKARR → DNS hints → fail.
- **Declarative intent & reconciliation**: Peers express desired state; substrate
  continuously reconciles intent vs. observed reality locally and
  cooperatively.
- **Portable execution units**: Micro-apps/services can be hosted across peers
  with explicit, revocable permission; lifecycle operations include deploy,
  move, suspend, resume, remove. Phase 1 also allows proxying existing services
  by assigning a peer identity and exposing them through the substrate.
- **Transport agnosticism**: Operates over intermittent connectivity and
  non-IP/delay-tolerant transports, including lossy links and
  store–carry–forward.
- **Privacy by default**: Minimizes identity correlation, topology inference,
  and behavioral signaling; disclosure is explicit and contextual.
- **Uniform component set**: Same shrink-wrapped components across deployments;
  capability gating enables role differences without custom binaries.
- **Topology composition**: Supports layered or bridged meshes, stars, and
  overlays with no primary topology; failures in one topology do not invalidate
  others.
- **Threat-model aware behavior**: Assumes malicious peers, unreliable networks,
  platform suspension/termination, and privacy threats.

### Out of scope
- **Central authorities or global coordinators**: No centralized identity
  providers, global registries, permanent bootstrap services, or trusted
  coordinators.
- **Global consensus or total ordering**: No dependency on replicated global
  state or blockchain-like ledgers.
- **Kernel or privileged OS control**: User-space only; no kernel modifications.
- **Always-on infrastructure**: High availability or permanent connectivity is
  not assumed.
- **Marketplace or economic layers**: No built-in marketplace, social network,
  or economic primitives.
- **Global naming/DNS replacement**: Identity does not imply global naming or
  DNS-like systems.
- **Anonymous-by-default guarantees**: Privacy by default is required, but full
  anonymity is not a system goal.
- **Zero-trust panacea**: The substrate provides containment mechanisms, not
  correctness guarantees for untrusted peers.

---

## 3) System invariants (must/must-not)

### Must
- **Identity and verification**
  - Every entity is uniquely identified by a self-controlled cryptographic
    identity.
  - All externally obtained information MUST be cryptographically verifiable.

- **Discovery and registries**
  - Discovery mechanisms are decentralized, best-effort, and hint-based.
  - Registries, if present, are optional and non-authoritative.
  - Clients MUST independently verify all discovered data.
  - Failure or compromise of discovery MUST NOT compromise system correctness.

- **Declarative intent and reconciliation**
  - The substrate supports declarative expression of service intent.
  - The system continuously reconciles intent against reality locally,
    cooperatively, and interruptibly, without centralized control.
  - Reconciliation is driven by any reachable substrate instance with service
    control enabled; an offline host’s substrate instance cannot reconcile on
    its own.

- **Portable execution**
  - Micro-apps/services are portable execution units that can be moved across
    peers.
  - Hosting is explicitly permitted by peers and revocable at any time.
  - Execution DOES NOT imply trust escalation or ownership transfer.

- **Network and transport agnosticism**
  - The system MUST operate under intermittent connectivity.
  - The system MUST function over non-IP and delay-tolerant transports.

- **Privacy by default**
  - Metadata leakage must be minimized by default (identity correlation,
    topology inference, behavioral signaling).
  - Identity, availability, and intent disclosure is explicit and contextual.

- **Uniform shrink-wrapped components**
  - All deployments use the same component set with capability gating.
  - No deployment requires custom builds or bespoke variants.
  - Operational complexity MUST remain low with minimal configuration burden.

- **Topology composition**
  - Multiple heterogeneous topologies can be composed, layered, or bridged
    without centralized coordination.
  - Failure, partition, or compromise of one topology MUST NOT invalidate
    operation over others.

### Must-not
- Must not rely on centralized identity providers, global registries, or trusted
  coordinators.
- Must not depend on global consensus, total ordering, or replicated global
  state for correctness.
- Must not assume always-on, low-latency IP connectivity.
- Must not require kernel modification or privileged OS control.
- Must not equate execution with trust escalation or ownership transfer.
- Must not rely on primary or authoritative network topology.
- Must not leak identity or intent metadata by default.

---

## 4) End-to-end flows (2–3)

### Core artifacts (minimal)
- **Peer Identity**: A cryptographic identifier for a micro-app/service peer,
  used as the primary addressability reference across discovery and transport.
- **Host Identity**: A cryptographic identifier for a host/node substrate.
- **Capability Profile**: A signed description of host substrate capabilities
  and limits (CPU, memory, disk, GPU, network constraints).
- **Peer Bundle**: A portable package for a micro-app/service peer, including a
  manifest with required capabilities, resource caps, and optional policy
  constraints.
- **Hosting Consent Grant**: An explicit, revocable approval from a host
  substrate to run a peer under specified limits.

### Flow A: Host onboarding and capability advertisement
1. A host/node owner installs a substrate instance on Host H, making a resource
   slice available for peers.
2. The substrate instance creates or loads its host identity and local policy (resource
   caps, allowed peers, exposure rules).
3. The substrate instance advertises non-authoritative hints to discovery sources
   (DNS-based, PKARR, or an out-of-band token) that describe the host identity
   and declared substrate capabilities (CPU, memory, disk, GPU, etc.).
4. Peers or service owners that encounter these hints treat them as candidates
   and must cryptographically verify the host identity before use.

**Outcome**: A host becomes discoverable with declared capabilities, without
becoming authoritative or trusted by default.

### Flow B: Service owner finds hosts and deploys peers
1. A service owner wants to deploy a set of micro-app/services (peers) with
   specific resource requirements and policies; the peer bundle includes a
   manifest describing required capabilities and resource caps. Phase 1 assumes
   a WASM-based peer bundle format with a minimal manifest; other formats are
   deferred.
2. The service owner queries discovery hints (DNS/PKARR/OOB token, or direct
   negotiation) to find candidate hosts advertising compatible capabilities.
3. For each candidate, the service owner verifies the host identity and
   negotiates a Hosting Consent Grant via a substrate instance with host control
   enabled.
4. The service owner deploys the peer bundle to the selected hosts; each host’s
   substrate instance enforces declared resource limits and capability gates,
   within OS constraints.
5. If a host revokes consent or becomes unreachable, the service owner’s
   substrate instance (service control enabled) suspends the affected peer and
   attempts to reconcile by rehosting on other eligible hosts.

**Outcome**: Peers are deployed onto compatible hosts with explicit consent,
enforced resource limits, and local reconciliation on failure.

### Flow C: Service consumption and ongoing change
1. External consumers (browsers, client apps) or other peers discover a service
   by its unique identity hash via hint-based discovery.
2. The consumer verifies the service identity and any advertised capabilities
   before interaction; unverified data is discarded.
3. The service owner updates peer characteristics (policy, resource profile, or
   version). The substrate propagates the change to deployed instances via the
   host substrate instances.
4. If required, the substrate migrates a peer to a new host under explicit
   consent, preserving identity while changing execution location.

**Outcome**: Services are consumable via verifiable identities and can evolve or
move without centralized coordination.

---

## 5) Acceptance criteria (demo + tests)

### Demo criteria (Phase 1 baseline)
- **Host onboarding**: Install substrate on two hosts, publish capability
  profiles via DNS/PKARR or OOB tokens, and verify identities.
- **Service deployment**: Discover both hosts, negotiate hosting consent
  grants, deploy a peer bundle, and confirm enforced resource caps.
- **Service consumption**: An external client discovers the peer by identity
  hash and successfully connects after verification.
- **Revocation and recovery**: Host revokes consent, peer is suspended, and
  redeployed to the second host via reconciliation.

### Test criteria (Phase 1 baseline)
- **Identity verification tests**
  - Reject any externally obtained data that fails cryptographic verification.
  - Confirm that identity is independent of network location.

- **Discovery tests**
  - Ensure discovery remains best-effort and hint-based.
  - Verify that compromised or failed discovery does not affect correctness.
  - Ensure all discovered data requires independent verification.

- **Intent reconciliation tests**
  - Validate that declared intent is stored and reconciled locally.
  - Ensure reconciliation proceeds without centralized coordination.
  - Confirm interruptibility and safe resumption after suspension.

- **Portable execution tests**
  - Validate deploy/move/suspend/resume/remove lifecycle across peers.
  - Ensure hosting requires explicit permission and is revocable.
  - Confirm execution does not confer trust escalation or ownership transfer.

- **Transport tests**
  - Validate operation under intermittent connectivity.
  - Verify transport abstraction allows non-IP in the future; Phase 1 may
    demonstrate delay tolerance via simulated store-and-forward.

- **Privacy tests**
  - Confirm metadata-minimizing defaults (identity correlation, topology
    inference, behavioral signaling).
  - Ensure identity/intent disclosure is explicit and contextual.

- **Uniform component tests**
  - Verify identical component binaries across deployments.
  - Confirm capability gating differentiates roles without custom builds.

- **Topology composition tests**
  - Validate bridging or layering of heterogeneous topologies.
  - Confirm failure of one topology does not invalidate others.

- **Threat model tests**
  - Simulate malicious peers, byzantine behavior, and replay/delay attacks.
  - Validate containment boundaries prevent propagation beyond trust scopes.
