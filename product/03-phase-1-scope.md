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
- Discovery fallback order (Phase 1): OOB token → PKARR → DNS hints → fail.
- Explicit, revocable hosting consent between service owner (substrate instance
  with service control enabled) and host owner (substrate instance with host
  control enabled).
- Deployment lifecycle: deploy, suspend, resume, remove.
- Proxy existing services by assigning a peer identity and exposing them through
  the substrate (no redeploy required).
- Resource cap enforcement (CPU, memory, disk) with observable limits.
- Intent storage and local reconciliation for simple cases (single peer, single
  host replacement on failure).
- Peer discovery by identity for external consumers.
- Transport abstraction layer defined to allow non-IP transports; Phase 1
  implementations will include IP transport support, one simulated
  delay-tolerant transport (e.g., store-and-forward via local file exchange),
  and one functional non-IP transport.

### Minimal artifacts
- Peer identity
- Host identity
- Capability profile
- Peer bundle with manifest (Phase 1: WASM module/component + minimal manifest)
- Hosting consent grant

### Packaging target
- CLI-based substrate reference app (for host onboarding and peer deployment).
- Minimal SDK surface in one language; additional client language support may
  be included when required by platform access constraints (e.g., browser).

---

## Out of Scope (Phase 1)
- Non-WASM peer bundle formats (native binaries, containers, etc.).
- Complex multi-topology bridging beyond basic hint sources.
- Advanced reconciliation strategies (global optimization, multi-host orchestration).
- Built-in marketplaces, payment, or economic layers.
- Always-on availability guarantees or SLAs.
- Kernel-level controls or privileged OS integrations.
- Full anonymity or obfuscation systems.
- Rich UI clients beyond a reference CLI.
- Functional non-IP transports (e.g., LoRa) beyond BLE.

---

## Phase 1 Demos (Success Criteria)
1. **Host onboarding**: Host owners install substrate instances on two hosts, publish
   capability profiles via DNS/PKARR or OOB tokens, and verify identities.
2. **Service deployment**: Service owner uses a substrate instance with service
   control enabled to discover both hosts, negotiate consent grants, deploy a
   peer bundle, and confirm enforced resource caps.
3. **Service consumption**: An external client discovers the peer by identity
   hash and successfully connects after verification.
4. **Revocation and recovery**: Host revokes consent, peer is suspended, and
   redeployed to the second host via reconciliation.

---

## Phase 1 Exit Criteria
- All demos above are repeatable in a controlled environment.
- Security checks validate cryptographic verification for all external data.
- Documentation clearly describes how to run the demo end-to-end.
