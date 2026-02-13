# High-Level Architecture: Identity-Native Peer Substrate

## Purpose
Define the conceptual blocks, trust boundaries, and end-to-end interactions for
Syneroym’s substrate without committing to specific implementation technologies.
This is the architecture frame used to lock Phase 1 scope and demos.

---

## Core Concepts (Actors)
- **Peer**: A uniquely addressable micro-app/service with its own cryptographic
  identity. Peers can interact with other peers and with external consumers.
- **Host/Node**: A logical machine (or subset of resources) that can run peers.
- **Substrate**: The overall system and product surface. It runs as a
  **substrate instance** on each host/node and can enable different roles
  (host control, service control, signaling, data relay, proxy) while using the
  same underlying components.
- **Substrate instance**: A running copy of the substrate on a host/node.
  Instances can enable specific roles based on owner intent and host policy.
- **Host Owner**: The user who installs a substrate instance and controls host
  policy.
- **Service Owner**: The user who installs and controls peers/services.
- **External Consumers**: Browsers or client apps that consume peer services by
  verifying peer identity.

---

## Architectural Blocks

### 1) Identity & Verification Layer
- Issues and manages cryptographic identities for peers and hosts.
- Verifies all externally obtained information.
- Enforces “no trust by position” and explicit trust boundaries.

### 2) Discovery & Hint Layer
- Produces and consumes **non-authoritative** discovery hints.
- Initial hint sources: DNS-based hints, PKARR, out-of-band tokens.
- Discovery outputs are candidates only; verification is mandatory.

### 3) Consent & Policy Layer
- Negotiates explicit, revocable **Hosting Consent Grants** between peers and
  hosts.
- Applies host owner policy (allow/deny lists, time/usage caps) via the Host
  Substrate.

### 4) Execution & Resource Control Layer
- Runs peers under explicit resource caps (CPU, memory, disk, GPU).
- Enforces capability gating and OS constraints.
- Supports deploy, move, suspend, resume, remove lifecycle operations.

### 5) Intent & Reconciliation Layer
- Stores peer/service intent.
- Continuously reconciles desired state vs. observed state without a central
  control plane.
- Handles churn, disconnections, and revocations.
  - Reconciliation is driven by any reachable substrate instance with service
    control enabled. An offline host’s substrate instance cannot reconcile on
    its own.

### 6) Transport Abstraction Layer
- Operates across intermittent connectivity and diverse transports.
- Supports non-IP and delay-tolerant links.

---

## Trust Boundaries
- **Peer ↔ Peer**: Trust is explicit and scoped per relationship.
- **Peer ↔ Host**: Hosting is explicit and revocable; execution does not imply
  trust escalation.
- **Discovery Sources**: Always untrusted; data is only a hint.
- **External Consumers**: Must verify peer identity and capabilities before use.

---

## Core Artifacts (Minimal)
- **Peer Identity**: Cryptographic identifier for a peer.
- **Host Identity**: Cryptographic identifier for a substrate instance/host.
- **Capability Profile**: Signed description of host substrate capabilities and
  limits (CPU, memory, disk, GPU, network constraints).
- **Peer Bundle**: Portable package for a peer, including manifest and resource
  caps.
- **Hosting Consent Grant**: Explicit, revocable approval to run a peer under
  defined limits.

---

## End-to-End Interaction Summary
1. **Host onboarding**: Host owner installs a substrate instance on a host,
   generates host identity, and advertises non-authoritative capability hints.
2. **Service deployment**: Service owner uses a substrate instance with service
   control enabled to discover candidate hosts, verify identities, negotiate
   consent grants, and deploy peer bundles.
3. **Service consumption**: Consumers discover peers by identity, verify
   identity and capabilities, then interact.
4. **Change & migration**: Service owners update peer characteristics; substrate
   propagates changes or migrates peers under explicit consent.

---

## Non-Goals (Architectural)
- No centralized authority, global registry, or consensus layer.
- No kernel-level integration or privileged control.
- No guaranteed availability or always-on connectivity.
- No built-in marketplace, social layer, or global naming system.

---

## Where Code Lives (Conceptual)
- **`libs/`**: Language-specific substrate libraries used by applications.
- **`apps/`**: Substrate applications (CLI/service/desktop/mobile shells).
- **`examples/`**: Reference integrations and demos.
