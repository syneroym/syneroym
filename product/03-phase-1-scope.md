# Phase 1 Scope: Substrate Baseline

## Goal
Deliver a minimal, end-to-end substrate slice that proves identity-native peer
interaction, verifiable discovery hints, explicit hosting consent, and bounded
execution across at least two hosts.

---

## In Scope (Phase 1)

### Core behaviors
- Peer and host identity creation and verification.
- Hint-based discovery using initial sources:
  - DNS-based hints
  - PKARR
  - Out-of-band tokens
- Explicit, revocable hosting consent between service owner and host owner’s
  substrate.
- Deployment lifecycle: deploy, suspend, resume, remove.
- Resource cap enforcement (CPU, memory, disk) with observable limits.
- Intent storage and local reconciliation for simple cases (single peer, single
  host replacement on failure).
- Peer discovery by identity for external consumers.

### Minimal artifacts
- Peer identity
- Host identity
- Capability profile
- Peer bundle with manifest
- Hosting consent grant

### Packaging target
- CLI-based substrate reference app (for host onboarding and peer deployment).
- Minimal SDK surface in one language (choose at implementation time; can be
  extended later).

---

## Out of Scope (Phase 1)
- Complex multi-topology bridging beyond basic hint sources.
- Advanced reconciliation strategies (global optimization, multi-host orchestration).
- Built-in marketplaces, payment, or economic layers.
- Always-on availability guarantees or SLAs.
- Kernel-level controls or privileged OS integrations.
- Full anonymity or obfuscation systems.
- Rich UI clients beyond a reference CLI.

---

## Phase 1 Demos (Success Criteria)
1. **Host onboarding**: Host owners install substrate on two hosts, publish
   capability profiles via DNS/PKARR or OOB tokens, and verify identities.
2. **Service deployment**: Service owner discovers both hosts, negotiates consent
   grants, deploys a peer bundle, and confirms enforced resource caps.
3. **Service consumption**: An external client discovers the peer by identity
   hash and successfully connects after verification.
4. **Revocation and recovery**: Host revokes consent, peer is suspended, and
   redeployed to the second host via reconciliation.

---

## Phase 1 Exit Criteria
- All demos above are repeatable in a controlled environment.
- Security checks validate cryptographic verification for all external data.
- Documentation clearly describes how to run the demo end-to-end.
