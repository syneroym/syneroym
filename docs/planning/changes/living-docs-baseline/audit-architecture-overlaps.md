# Audit: architecture doc, overlaps, questions and fix order

Reader: a developer who will apply the fixes to [system-architecture.md](../../../system-architecture.md).

This file merges sections 3 to 6 of the architecture audit reports. The reports are batches A, B, C, D, E, F, G1 and G2, the Layer 2 pilot ([audit-architecture-layer2.md](audit-architecture-layer2.md)), and the gap report ([audit-architecture-gaps.md](audit-architecture-gaps.md)). It is part of the [living docs baseline](change.md). I changed no code and no doc. I did not re-audit the code. Where a report did not say which copy the code supports, I read the code to decide. In every other row I used the report's verdict.

How to use it:

- Part 1 lists every statement that the doc writes in more than one place, or writes in two different ways. Each row names the copies by heading and first words, the copy the code supports, and the action.
- Part 2 lists every question with its default. A star marks a question that needs a user decision before you edit.
- Part 3 lists every place that cites a milestone ID, a commit hash or a dated note.
- Part 4 gives the order of the fix commits.
- Part 5 gives the merged, ranked gaps.

Notes that apply to everything:

- The reports quote old line numbers. The Layer 2 rewrite (#290, #291) moved them. Always find a place by its heading and first words.
- Layer 2 is already fixed. A row that cites Layer 2 means the current Layer 2 text. A few Layer 2 leftovers remain (they are listed).
- When one statement has several copies, fix all copies in the same pass. Part 4 groups commits by heading, so take from each row the copy that belongs to the heading you edit. Part 1 is the checklist that nothing is missed.
- "Roym spec" means `docs/roym-integrated-experience-spec.md`. "Req spec" means `docs/system-requirements-spec.md`.
- Report ids: `A-D3` is batch A, duplicate 3. `B-Q6` is batch B, question 6. `GAP-12` is row 12 of the gap report.

## 1. Duplicates and contradictions, merged

"Neither" in the code column means nothing is built, so both copies are plans. Then the action keeps the more detailed copy as the Envisioned design and marks the other.

### 1.1 Relay, multi-hop, bootstrap, transport

| # | Statement | Copy 1 | Copy 2 (and more) | Code supports | Action |
| --- | --- | --- | --- | --- | --- |
| O1 | Who implements multi-hop forwarding | Top warning > "Multi-hop relay routing (the Federated Coordinator model) is implemented in the coordinator crates" | Multi-Hop Relay > "Implemented in the coordinator crates (`crates/coordinator_iroh`), not the substrate" (A-D1) | Neither. Forwarding is in `crates/router` (A MH-01). | Fix copy 1 and copy 2. |
| O2 | Name `hop-relay` | Multi-Hop Relay > "The `hop-relay` subsystem lives in the Coordinator..." | Appendix > Scenario Entities > "**Cp** ... Acts as the `hop-relay`" and "**C** ... public `hop-relay`" (E-D1) | Neither. The code has no such name. | Fix both. Use "next-hop forwarding". |
| O3 | What the next hop is | Multi-Hop Relay > "forwards the stream to that target's local Coordinator" | Appendix > step 3, step 5 > "determines the final hop is the hidden substrate **Sz**" (A-D2, E-D2) | The Appendix: the next hop is the substrate. | Fix copy 1. |
| O4 | Cp is the entry point of a private substrate | Appendix > step 2 "a caller must route to the entry point **Cp**" and step 3 | Multi-Hop Relay (same idea) (E-D2, A-Q6) | Neither. It works only when a record names a coordinator. | Reword both: "when a record names a coordinator". |
| O5 | "No permanent connection" from Cp to C | Multi-Hop Relay > "only on demand (outbound-only, no permanent tunnel)" | Appendix > step 1 > "**Cp** does *not* maintain a permanent connection to **C**" (E-D3) | Neither fully (A MH-06, E S1-05). | Reword both to what the code does. |
| O6 | Cp registers in the registry | Appendix > step 1 > "**Cp** also registers itself in the global Registry **R**" | Appendix > step 2 > "**Cp** Registration: The private Coordinator **Cp** registers its Iroh key" (E-D4) | One registration, once, no heartbeat (E S1-04, S2-03). | Merge into step 2. Delete the step 1 sentence. |
| O7 | The end-to-end handshake is always present | Multi-Hop Relay > "Endpoints then run their own end-to-end handshake inside the forwarded stream" | Appendix > step 3 "complete an explicit End-to-End Diffie-Hellman handshake", step 5 (E-D5) | The handshake is optional (`?enc=ecdh-p256`) (E S3-10, S5-03). | Fix both: "when requested". |
| O8 | Substrates are endpoints only | Multi-Hop Relay > "Substrates are endpoints only; they never bridge network segments themselves" | Connectivity > Gateway Nodes > "Caller nodes connect to a gateway and request forwarding" (F-D4) | Substrates also forward by preamble (`io.rs`, A MH-02). A BLE/LoRa gateway is not built. | Fix copy 1. Mark Gateway Nodes Envisioned. |
| O9 | Relay list or per-node records in the DHT | Bootstrap Server & DHT Fallback > "**mirrors its relay registry** as `pkarr` signed packets" and the "mirror relay registry every 15 min" diagram label | Layer 3 > Discovery & Matching > "Relay Discovery: BEP 0044 Mainline DHT (via `pkarr`) resolves node/relay endpoints only"; Technology Stack > "pkarr + BEP 0044 DHT / SynApp registry + bootstrap fallback"; Connectivity > Discovery and Node Record > "BEP-0044 mutable records"; Resolved TBD row 16 > "pkarr signed packets mirrored to BitTorrent DHT; 24h local cache; community governance key" (A-D3, F-D3, D-13) | Layer 3 wording: per-node endpoint records (`dht_registry/types.rs`). | Fix copy 1, the Connectivity record format and TBD row 16. Keep the bootstrap design as one Envisioned block. |
| O10 | The same relay and bootstrap design in the Req spec | Layer 1 > P2P Networking, Relay Node Architecture, Bootstrap | Req spec "Relay ... TURN relay for WebRTC", `<relaynodeid>.syneroym.net`, "Bootstrap Server" (A-D11) | Neither built. | Fix both together. The Req spec is outside this doc: record it as a follow-up. |
| O11 | TURN for browsers | P2P Networking: Iroh diagram > "3d. WebRTC TURN (browser clients)" and Relay Node Architecture > "TURN Server for WebRTC" | Appendix > Scenario Entities > "C ... acting as public DERP/TURN relay"; Security > Encryption at Every Layer > "T3 Browser-to-service: WebRTC DTLS-SRTP or TLS over WebSocket" (A-D5) | The Security line (the two built paths). | Fix the Layer 1 and Appendix copies. |
| O12 | "iroh-net" and "DERP" | P2P Networking: Iroh > "`iroh` ... DERP relay" and the diagram labels "DERP Relay Node", "3c. DERP relay" | Technology Stack > "Iroh (iroh + iroh-net) ... DERP relay"; Glossary > "DERP - Designated Encrypted Relay Protocol"; Security > Encryption diagram > "T2 DERP relay: additional AES-256-GCM envelope" (A-D9, E-D11) | `Cargo.toml` has `iroh`, `iroh-base`, `iroh-relay`. No `iroh-net`. | Fix all four. Say "Iroh relay". |
| O13 | Executive Summary says no server is in the path | Executive Summary > "no server sits between participants" | Layer 1 > Multi-Hop Relay and Relay Node Architecture > "relay fallback" (A-D4) | Layer 1. | Fix copy 1 (Q-A1). |
| O14 | Relay metadata and privacy | Multi-Hop Relay > "the Coordinator relays encrypted bytes and cannot read them" | Roym spec "What is encrypted, and who can see what" (A-D12, E-D13) | The Roym spec is closer. | Add one sentence to copy 1: the relay sees who talks, sizes and timing. Do not move content. |
| O15 | Connect tries each mechanism in turn | Connectivity > Transport Layer / Connection Establishment > "try each path" | Addendum [TOP-ROB] > "Retry Logic Integration" (F-D6) | `[TOP-ROB]` is built for router and proxy hops (`net_iroh.rs`). The SDK has a timeout and no retry or fallback. | Fix Connectivity. See Q-F4 for the code question. |
| O16 | Protocol negotiation | Connectivity > Protocol Negotiation > "HELLO / service / protocol" | Addendum [LFC-VER] 2 > "Network Protocol Handshake & Capability Matrix"; the route preamble (`preamble.rs`) (F-D5) | The preamble. Both designs are unbuilt. | Delete HELLO. Keep `[LFC-VER]` 2 as Envisioned. |
| O17 | The socket API | Connectivity > Application Interface > "`node.listen/accept/connect`" | Layer 2 > Substrate API Surfaces > "The substrate has one API surface: **JSON-RPC 2.0**" (F-D8) | Layer 2. | Delete or mark copy 1 Envisioned (Q-F2, Q-F3). |
| O18 | Identity: `did:p2p` or `did:key` | Connectivity > Node DID / Service DID > "did:p2p:nodeA", "did:p2p:svc123" | Layer 3 > Identity > "A persistent `did:key` (Ed25519)"; Addendum [TOP-ADR] > "explicit `ServiceId`s (DID-keys)" (B-3.4, F-D1) | `did:key:h...` (`identity/src/substrate.rs`). | Fix Connectivity. |
| O19 | wRPC as a working protocol | Connectivity > Service Record > `"protocols": ["wrpc"]` and Protocol Adaptation > "JSON-RPC client -> wRPC service"; Appendix > step 5 "(e.g., wRPC frames)"; Technology Stack > "The goal is **wRPC**" | Top warning > "The **wRPC protocol layers/surface** is not yet implemented"; Layer 2 > Substrate API Surfaces (Envisioned note); Glossary "wRPC" (F-D2, E-D6) | The "unbuilt" copies. JSON-RPC 2.0 is the wire protocol. | Fix the Connectivity record example and Appendix. Mark Glossary "wRPC" Envisioned. |
| O20 | "Heteregenous" heading and broken anchors | Heading "Connectivity Substrate In Heteregenous networks" | Table of Contents > "Future: Heterogeneous Networks"; P2P Networking: Iroh note; Table of Contents > "MVP Phase 1 Scope & Acceptance Criteria" (no heading exists), "Open Questions" (heading is "Open Questions & Recommendations"); the table omits the Appendix and the addendum (A-D13, F-D7, pilot 3.3) | Neither. Both anchors break. | Fix the heading spelling. Fix or remove the three TOC links. Add the Appendix and addendum to the TOC. |

### 1.2 Names, tiers, stale terms, canonical copy

| # | Statement | Copy 1 | Copy 2 (and more) | Code supports | Action |
| --- | --- | --- | --- | --- | --- |
| O21 | Name of SynApp 1 | Executive Summary > "Home Services Guild + Food & Small Retailer Mesh"; System Layers Overview diagram > "SynApp 1: Service and Retail Spaces" | Layer 4 > "SynApp 1: Business, Professional & Retail Spaces" (A-D6) | The code calls it Roym (`roym.toml`). | Use one name in all three. Keep the verticals as Envisioned (Q-A2). |
| O22 | What an aggregator is | Conceptual Entity Model > "AGGREGATOR ||--o{ PROVIDER : hosts-for" | Phase 6 > "7. Aggregator Fuel Quotas" > "Aggregators are fundamentally just Providers offering a horizontal service" (A-D7, G2-8) | Neither. | Pick one (Q-A4, star). |
| O23 | The word "Tier" | Key Hardware Constraints > "Tier 1 - Minimal / Android phone" | Observability > "Tiered Observability Stack" (hardware tiers); Layer 3 > Identity (key tiers, "Tier 1" government identity); Addendum [EDG-MOB] (A-D8) | Three meanings. The phone table is a plan. | Rename two of the three uses. Mark the phone tier as a plan. |
| O24 | Stale terms `SYN-MOD`, `HOME_RELAY`, `SYN-SVC`, `Space`, `CRDT`, `DERP`, `MLS` | Conceptual Entity Model > "SYN-MOD", "HOME_RELAY" | Glossary > "**SYN-MOD** A reusable, independently deployable unit...", "HOME_RELAY", `SYN-SVC`, `Space`, `CRDT`, `DERP`, `MLS` (A-D10, G2-12) | None of them exists as a name. | Fix body and Glossary in one pass. Delete Glossary rows whose term leaves the body. |
| O25 | Stale `CRSQL` | Layer 2 diagram labels (pilot L2-13) | Other diagrams (not checked) | The code has no CRSQL. | Check every diagram for `CRSQL` when you edit its heading. |
| O26 | Which copy is canonical | Top > "Migration Note" > the addendum holds "the canonical Layer 1-4 definition" | The main Layers 1-4 above it; the addendum has a second `#` title "Syneroym: Substrate Feature Implementation Design" (pilot 3.1) | The main sections are canonical. | Fix the note. Demote the second `#` title. |

### 1.3 Identity, keys, messaging crypto, security

| # | Statement | Copy 1 | Copy 2 (and more) | Code supports | Action |
| --- | --- | --- | --- | --- | --- |
| O27 | Messaging crypto | Layer 3 > Messaging > "M2[Group Chat / Threads MLS RFC 9420]" and "**Libraries:** `libsignal-protocol-rust` ... `openmls`" | Security > Encryption at Every Layer > diagram M1, M2 and "1-to-1 chat: X3DH + Double Ratchet libsignal-protocol-rust"; Technology Stack > rows "libsignal-protocol-rust", "openmls", "Native bindings for `libsignal`"; Consumer App Architecture > `CRYPTO_CLIENT` "libsignal"; Glossary > "MLS" (B-3.2, D-2, E-D8, C) | `vodozemac` X3DH + Double Ratchet (`conversation/src/crypto.rs`) and an owner-distributed AES-256-GCM group key per epoch (`dag.rs`). ADR-0013 Amendment 1 replaced MLS. Roym spec "Resolved: O1" agrees. | Fix all six places. Name `vodozemac` and the epoch key. Link ADR-0013. |
| O28 | Master Anchor: built or deferred | Layer 3 > Identity Resolution & Revocation (The Master Anchor) > presents it as working | Addendum [TOP-DSC] > "Master Anchor Resolution (Phase 0 Contract)" > "Production Master Anchor DHT authorization and signed record formats are deferred to Phase 1 `[FND-IDT]`" (B-3.3, G1-9) | Built (`dht_registry/master_anchor.rs`), as a revocation (deny) list. | Fix `[TOP-DSC]`. Document the deny list (Q-B8). |
| O29 | Government identity as "Tier 1" and compromise fallback | Layer 3 > Identity (Tier 1, ZK plugin) | Req spec `[FND-IDT]`: no government ID is the universal root; recovery rotates delegates (B-3.5) | Neither built. | Keep the Req spec wording (Q-B1, star). |
| O30 | Attestation as a goal | Security > Substrate Integrity & Remote Attestation > `substrate.attest(nonce)` (presented as working) | Addendum [FND-SEC] > "The 'Unlock' Model ... optionally after attestation" (future); Req spec "Hardware Attestation (optional)" (D-4, G1-10) | Neither. No attestation code. | Add one Envisioned marker on the Security copy. Reword "wRPC" to JSON-RPC. |
| O31 | The word "attestation" | Security > "Attestation Quote" (hardware) | Layer 4 and Roym spec "Records and what each one proves" (signed receipts) (D-15) | Receipts are built (`roym_core/src/transaction.rs`). Different meaning. | Add one line to separate the two meanings. |
| O32 | Envisioned marker covers too much | Security > Encryption at Every Layer: the marker under the diagram reads as covering nine boxes | Only R2 (replication) is unbuilt, plus the stale names in O12 and O27 (D-16) | R2 only. | Move the marker next to R2. |
| O33 | Replication design: Litestream or WAL shipping | Layer 2 > Storage & Write Arbitration > "Replicated backups. A live copy..."; Security > Encryption at Every Layer > "Node R2 is a goal ... Litestream and Iroh WAL shipping are both options"; Technology Stack > "Option 1 is **Litestream**"; Developer Toolchain > "If the replication design uses Litestream, the toolchain adds the `litestream` CLI" (D-1, E-D7, G1-3, L2-Q1) | Addendum [PLT-RED] > "Database Replication Mechanism (Iroh WAL Shipping) ... Instead of relying on Litestream or FUSE-dependent LiteFS" | Neither. | Make the four "both options" notes follow `[PLT-RED]` (Q-1, star: commit #291 left the options open on purpose). |
| O34 | Native caller gate | Addendum [PLT-DAT] 2 > "Interim security posture ... does **not** currently require a delegation certificate" | Same part > "HTTP Passthrough" > "Gap closed ... `HandshakeVerifier::verify_preamble` is mandatory"; Layer 2 > Access control > "The caller's temporary key must carry a delegation certificate" (G1-2) | Copy 2 (`preamble.rs`). A caller with no certificate is accepted as its own master key (`handshake.rs`). | Delete the interim paragraph. Fix the Layer 2 sentence (leftover). |
| O35 | Isolation diagram | Security > Isolation Guarantees diagram > `APP1 <-> APP3` "direct access after substrate-vetted initialization" and a shared `DB2` | Layer 2 > Storage (one database per service) (D-Q4) | Layer 2. | Delete the edge and the shared node (Q-D4, star). |

### 1.4 Discovery, reputation, payments, orders, client

| # | Statement | Copy 1 | Copy 2 (and more) | Code supports | Action |
| --- | --- | --- | --- | --- | --- |
| O36 | Discovery: three designs | Layer 3 > Discovery & Matching > "Placement: a protocol-defined Routing Schema ... leaf index shards" | Federation Architecture > Cross-Substrate Discovery Flow > "Leaf Index Shards (rendezvous-hashed by routing descriptor)"; Minimum Federation Contract > item 2; Resolved TBD rows 14, 15, 17; Addendum [P2P-DSC] > "forwards the intent to those peers ... `Time-To-Live (TTL)`" (B-3.6, C, D-12, G2-3) | Neither. Built: the consumer fans out to directories it was given (`client_query.rs`). | Keep Layer 3 as the one Envisioned design. Mark `[P2P-DSC]` Envisioned and cross-link. Trim the shard detail from the Federation flow (Q-B4, star). |
| O37 | Reputation: two designs | Layer 3 > Trust & Reputation > "ReputationRecord signed by both parties. Anchored in DHT"; Resolved TBD rows 5-10; Federation contract item 4; Layer 2 write-rules row (already marked) | Addendum [P2P-REP] > "`score`: ... `0`, `1`, or `2`" and "never a jointly-written shared record"; Phase 6 > "4. Portable Data & Reputation Envelopes" (B-3.7, G2-4) | Only the independent-halves pattern, only for receipts (`roym_core/src/transaction.rs`). | Keep `[P2P-REP]`. Fix its "mutually signed entry" phrase under `interaction_receipt`. Mark Layer 3 Envisioned (Q-B3, star). |
| O38 | Vouch weight and moderation rows | Resolved TBD row 5 > "weight = `base × 0.5^hops`; max depth 3" and rows 7-10 | Layer 3 > Trust & Reputation > "Vouch weight formula: `effective_weight = base_weight × decay_factor^hop_count`", "Reputation portability", "Sybil resistance mechanisms" (D-7, D-9) | Neither, except contact and publication rate limits (`roym_core/src/safety.rs`). | Mark Envisioned. Add a status column to the TBD table (Q-D5). |
| O39 | Verifiable credential library | Resolved TBD row 6 > "`didkit` for issuance/verification" | Technology Stack > "Verifiable Credentials **ssi** (Rust)" (D-8) | Neither. | Name `ssi` in both (Q-E7). |
| O40 | Payments | Layer 3 > Payments > "Payment Strategy (MVP): ... redirection to external payment flows ... Verification is offline-delayed" | Layer 4 > Consumer Transaction Flow > "create Stripe PaymentIntent", "webhook: payment confirmed → PAID"; Phase 6 > "3. Flexible Payment Integration" > "abstract `PaymentIntent` interface"; Technology Stack > "Payment (MVP) **Stripe Connect SDK** + UPI deep links"; Resolved TBD rows 11, 12 (B-3.8, C, D-10, E-D9, G2-5) | Out-of-band signed records only (`roym_core/src/payment.rs`). Roym spec "Not in the first release": no payment processing. | Fix the Layer 4 flow and the Technology Stack row to match Layer 3. Mark Phase 6 item 3 Envisioned. Describe built payment records first (Q-2). |
| O41 | Escrow and dispute | Layer 4 > Order State Machine > "DISPUTE --> RESOLVED / REFUNDED" | Layer 3 > Payments > "Escrow and dispute-mediated fund custody are deferred"; Phase 6 > "6. Decentralized Escrow & Dispute Resolution" (C, G2-6) | The deferral text. | Fix the Layer 4 diagram. |
| O42 | Order state machine as working | Layer 4 > Order State Machine > "stateDiagram-v2 ... DRAFT" and "Provider cancel and consumer cancel both pending..." | Layer 2 > Storage & Write Arbitration > "Envisioned. Not built yet. Roym has no order entity" and "Order state. A provider action beats a same-instant consumer action" (C) | The Layer 2 note. Only the provider cancels. | Replace the diagram with the real booking machine (Q-C2, star). |
| O43 | Two ranking formulas | Layer 3 > Discovery & Matching > "Ranking: transparent weighted formula (keyword relevance, geo proximity, reputation, ad-boost, recency)" | Layer 4 > Recommendation Algorithm > "score(item, ...) = 0.4 × collaborative_signal ..."; Resolved TBD rows 13, 15; ad-boost cap 0.3 repeated in TBD rows 15, 17, Discovery & Matching and Trust & Reputation (B-3.9, C, D-11, D-12) | Neither. The code orders by recency (`search_ops.rs`). | Keep both (search versus recommendation) as Envisioned. State the difference once. Keep the cap in one place. |
| O44 | Recommender | Layer 4 > Recommendation Algorithm > "Catalog recommendations are client-side only" | Roym spec "Not in the first release" > "No AI assistant that searches, recommends, or acts" (C) | The Roym spec (no code). | Keep as Envisioned. Move it after the real search text (Q-C4). |
| O45 | Portability format | Federation > Minimum Federation Contract > "5. **Portability:** ... `SynExport` archive format"; Open Questions > OQ-5, OQ-6 | Resolved TBD > "Migration protocol" row 1 (Roym archive) (C) | The TBD row (`identity/src/backup.rs`). | Replace `SynExport` with the Roym archive in three places. |
| O46 | Native client shell | Consumer Experience > Consumer App Architecture > "Consumer App (Tauri Desktop / Native Mobile / PWA)" | Technology Stack > Consumer Frontend; Phase 6 > "0. Core Client Architecture" > "Desktop (Tauri)"; Open Questions > OQ-7 (C, E-D10) | None built. The Hub is a web UI. | Mark Envisioned in all four. Add a "Built today" paragraph. |
| O47 | Client link is WebSocket or gRPC | Layer 4 > Component Architecture > `PWA --> GW` "JSON-RPC / WebSocket"; Consumer App Architecture > `CONN[... WebSocket / WebRTC]`, "Option A/B/C" and "JSON-RPC / wRPC via FFI, WebSocket, or WebRTC" | Phase 6 > "0. Core Client Architecture" > "persistent, authenticated WebSocket/gRPC connection to the local substrate API"; Technology Stack > External API row (WebSocket "an option") (C-06, G2-7) | HTTP `POST /rpc` with a session token (`roym_web/ui/src/rpc.ts`). | Fix all copies together. |
| O48 | Identity path of the Hub | Phase 6 > "0. Core Client Architecture" and Open Questions > OQ-2 (a key made in the browser) | Consumer App Architecture; ADR-0024 (delegated key, auth service) (G2) | ADR-0024 (`roym_web/ui/src/session/login.ts`). | Fix both. |
| O49 | Gateway bind address | Roym spec > Client contract > "The gateway binds `127.0.0.1`" | `crates/client_gateway/src/gateway.rs:184` binds `0.0.0.0:<port>` (C, G2) | The code. | Do not state either in this doc. Report to the spec owner (Q-C6, star). |

### 1.5 Observability, supervisor, lifecycle, addendum bodies

| # | Statement | Copy 1 | Copy 2 (and more) | Code supports | Action |
| --- | --- | --- | --- | --- | --- |
| O50 | Observability storage | Observability Architecture > "Default backend: in-process circular buffer" | Addendum [ADV-OBS] > "A separate `metrics.db` SQLite database", "non-blocking `tokio::sync::mpsc` channels", and the name "Observability Engine" (D-3, G2-1) | Neither. A lock-based in-memory `MemoryRecorder` (`recorder.rs`). The real `ObservabilityEngine` only sets up logging, recorder and sampler. | Keep both as Envisioned. Cross-link. Rename the `[ADV-OBS]` engine. |
| O51 | Observability stack | Observability Architecture (`metrics` crate; Prometheus) | Technology Stack > "Observability: **OpenTelemetry** (OTLP) Traces + metrics + logs" (D-5, E-D12) | Neither fully. `tracing` and `metrics`; OTLP is config types only. | Fix the stack row. |
| O52 | Where visuals live | Addendum [ADV-OBS] > "The Substrate does not host its own visualizations" | Observability Architecture > "Built into the substrate's own HTTP server as a static HTML page ... `/admin`" (G2-1) | Neither is built. | Mark Envisioned. Pick one. |
| O53 | Provider health UI | Observability > Provider-Facing Status UI | `[ADV-OBS]` and the Roym spec have none (D-14) | Nothing built. | No overlap. Mark Envisioned (Q-D1). |
| O54 | Simulation scenario for every write rule | Observability > Simulation Testing and Replay Validation > "Each write rule in Storage & Write Arbitration has a corresponding scenario" | Layer 2 > Storage & Write Arbitration (rewritten) (pilot 3.7) | No simulation exists. | Delete the claim or mark Envisioned. |
| O55 | Resolved TBD rows 1-4 | Resolved Architecture TBD Items rows 1-4 | Layer 2 > SynApp Packaging (Backup and Restore) and Storage & Write Arbitration (D-6) | Both match the code. | No action. |
| O56 | Pub/sub model | Addendum [PLT-DAT] 2 > "P2P Overlay over QUIC (Pub/Sub)": "publishers append to a local state log, and subscribers pull/sync" | Same part > "Pub/Sub Execution Flow": "a local in-process `rumqttd` Tokio task" (G1-1) | Copy 2. | Fix copy 1. |
| O57 | DLQ exists or not | `[PLT-DAT]` 3 > Implementation status > "the DLQ described under 'Resilient RPC & Dead Letter Queues' below does not [exist]" | `[PLT-ASY]` > "Resilient RPC & Dead Letter Queues" > "local SQLite-backed DLQ" (G1-6) | Copy 2 (`async_queue/src/queue.rs`). | Fix the note. |
| O58 | `[PLT-ASY]` note and body | `[PLT-ASY]` > "Implementation status": "the outbox lives substrate-side", "`saga-undo-<operation>`", leases "replaced outright" | Same section body: "A client uses an outbox queue", "`undo_<operation>`", "nodes race to acquire a specific execution lease from the Registry" (G1-5) | The note. | Fold the note into the body. Delete the note. |
| O59 | Who owns promotion | `[PLT-RED]` > "Registry & Coordination Model": "The Registry Service acts as the authoritative control plane" | `[PLT-RED]` > "Control Plane vs Data Plane Isolation": "If the App Supervisor fails, the Control Plane freezes" (G1-4) | Neither built. Copy 2 names a real component. | Keep copy 2. Fix copy 1. |
| O60 | WAL | `[PLT-DAT]` 1 > "Database Isolation": "(and WAL) ... independent WAL lock" | Layer 2 > Storage & Write Arbitration > "Each database has one writer task" (G1-7) | The code sets no WAL pragma for service databases. | Remove "(and WAL)" until Q-G1-1 is settled. |
| O61 | Sharded routing | `[TOP-ADR]` (Sharded and three strategies as a working mode) | Layer 2 > One app on several hosts > Envisioned note: "The compiler never emits `Sharded`" (G1-8) | Layer 2. | Mark the `[TOP-ADR]` Sharded text Envisioned. Add one line: the resolver is built, the compiler does not emit it. |
| O62 | Connection reuse | `[TOP-ROB]` > "Iroh multiplexes over existing connections" | `docs/planning/deferred-backlog.md` > "Federated fetch has neither result caching ... nor connection reuse" (G1-12) | The backlog row (`proxy/hop.rs`). | Fix the `[TOP-ROB]` sentence. |
| O63 | Migration hook names | Addendum [LFC-VER] 1 > "(e.g., `execute_sql`)", "invokes its exported `init()` (or `migrate()`)" | `[PLT-DAT]` 1 > "exports `init()` ... and `migrate()` ... `execute-ddl`" (G2-2) | `[PLT-DAT]` (`lifecycle.rs`, `data-layer.wit`). | Fix `[LFC-VER]`. Mark snapshot, rollback and replication epoch Envisioned. |
| O64 | Word "controller" and supervisor interface | `[LFC-MGT]` 1 > "optional stateful controller" and "no service-facing directory interface" | `docs/TERMINOLOGY.md` > "Do **not** call the supervisor a controller"; Layer 2 > Deploy and lifecycle > "App Supervisor"; the `supervisor` WIT has `resolve` (G2-9) | Layer 2 and the WIT. | Fix `[LFC-MGT]`. Fix `TERMINOLOGY.md` (outside this doc). |
| O65 | Supervisor key custody | Layer 2 > roles table > "`supervisor` ... holds the desired state and the master keys" | `[LFC-MGT]` 1 (silent) and `[LFC-MGT]` 2 > "losing the supervisor's node costs a rebuild, not the app" (G2-10) | Layer 2. A rebuild is safe only if `export-master` ran. | Add the `export-master` sentence (Q-G2-3). |
| O66 | Replication needs `[PLT-RED]` | `[LFC-VER]` 1 > "the primary first records a replication epoch" | `[LFC-MGT]` 2 > "replication (Iroh WAL shipping) does not arrive until M7"; Layer 2 > Storage Envisioned note (G2-11) | Consistent in meaning. | Replace the milestone ID with the Envisioned marker (see Part 3). |
| O67 | Config flags and BLE/LoRa options that do nothing | Layer 1 (config text) | Connectivity sample config; `enable_signalling`, `enable_relay`, `transport_bridge`, `ble`, `lora` parsed and never read (A gap 5, F gap 8) | Never read. | Mark Envisioned in the doc. Add a backlog row, or remove the sample lines. |

## 2. Questions, merged

A star marks a question that needs a user decision before you edit. The default changes the product, deletes a vision item, or needs a code change. The rest are plain doc fixes: apply the default. "(backlog)" means the default also adds a row to `docs/planning/deferred-backlog.md`.

### 2.0 Decisions received (2026-10-07)

The owner answered all the ★ questions. These answers replace the defaults in 2.1 for the same questions. 

| Question | Decision | Effect on the fix commits |
| --- | --- | --- |
| Q-1 (Litestream) | Litestream stays an option we can consider. Do not remove it. | Do not collapse the "both options" text to Iroh WAL shipping. Keep both options open, as #291 left them. Fix only wrong statements (for example, a text that says Litestream is the chosen design when `[PLT-RED]` says otherwise). |
| Q-2 (matching, reputation, payments) | These are Roym features. | Apply the default: Roym wording, keep Envisioned, drop from substrate diagrams. |
| Q-C2, Q-C3 (order machine, component diagram) | Redraw and fix. | Apply the defaults: the real booking machine and the six Roym services. |
| Q-C6 (gateway bind address) | A detail. The gateway binds `127.0.0.1` today. `0.0.0.0` is fine once access control is in place. The gateway may be used by all. | Do not state a fixed bind address as an architecture rule. Say that the bind address is configurable, and that access control decides who may call. The code (`0.0.0.0` at `crates/client_gateway/src/gateway.rs:184`) and the Roym spec (`127.0.0.1`) still differ: record this in the backlog, not in this doc. |
| Q-B2 (router proof-of-possession and handshake gaps) | Fix the documentation. | Make the Identity and Security text say what the code does: the router checks the certificate chain, scope and revocation, and does not check that the caller holds the temporary key; the end-to-end handshake is one-sided and runs only when the caller sets `enc=ecdh-p256`. Mark the stronger behavior as Envisioned. Do not change code in this change. |
| Q-F4 (SDK fallback to a second mechanism) | Maybe later. Envisioned. | Mark "try each path until one succeeds" as Envisioned. Add a row to the deferred backlog. No code change now. |
| Q-A4 (aggregator) | An aggregator is like a SynOrg: a `directory` service (as in Roym `directory`). It aggregates provider data. It can federate with other aggregators and proxy queries to them. | Replace the "hosts-for" idea. Describe an aggregator as a directory-type SynOrg service. Say federation and query proxying between aggregators are Envisioned unless a report shows code. |
| Q-B1 (government identity tier) | Envisioned. | Keep, marked Envisioned, as in the default. |
| Q-B3 (reputation design) | Not frozen. It will be frozen later. Today there are only principles: decentralized, reliable, transparent, and under the owner's control of what is shared. | Do not choose between the Layer 3 design and `[P2P-REP]`. Write the principles. Mark both designs as candidates, Envisioned and not final. |
| Q-B4 (discovery design) | Discovery is what Roym `directory` does today (see `docs/roym-integrated-experience-spec.md`). Clients, SynOrgs, directories and aggregators each choose what they query. | Describe this model as the main design. Leaf shards and tag routing become Envisioned options, not the plan. |
| Q-D4 (Isolation diagram edge `APP1 <-> APP3`) | The edge shows that apps may talk to each other. | Keep the edge. Relabel it as cross-app communication through the platform, and say it is subject to access control. Still fix the shared `DB2`: databases are per service. |
| Q-F2, Q-F3 (`listen/accept` and the transport interface) | A server-side `listen/accept` API is not a goal. But Iroh QUIC and WebRTC have an internal listen/accept equivalent. Use general wording. | Delete the `dial/listen/capabilities` interface text. Say: the node accepts inbound streams on each transport (Iroh QUIC, WebRTC) and hands them to the router. Say that callers connect, and services never accept connections themselves. |
| Q-G1-1 (WAL mode) | Document what exists. Advanced tuning is Envisioned. | Remove "(and WAL)" claims. Say what the code does: one writer task per database, no WAL pragma set. Mark WAL and tuning as Envisioned. No code change. |
| Q-B2, remaining parts | Keep the default. | Document only what the code does. Add one backlog row for the three handshake points (see below). |

### 2.1 Questions that need a user decision first

Where section 2.0 has a decision for the same question, 2.0 wins.

| # | Question | Default | Sources |
| --- | --- | --- | --- |
| Q-1 ★ | Remove Litestream from the four "both options" places and make Iroh WAL shipping the one planned design? Commit #291 marked the redundancy options as open, so this may reverse a choice. | Yes. Keep `[PLT-RED]` (O33). | L2-Q1, D-1, E-Q5, G1-Q3 |
| Q-2 ★ | Are matching, reputation and payments substrate components, or Roym app features? Layer 3 is titled "Shared Substrate Utilities". | App features. Describe them under Roym wording, keep them Envisioned, and drop them from substrate diagrams. Describe built payment records first. | L2-Q2, B-Q7, G2-Q5 |
| Q-A4 ★ | What is an aggregator? A host for providers (Entity Model) or a provider offering a horizontal service (Phase 6 item 7)? | "A provider" (the later text). Delete the `hosts-for` edge. Nothing in code is called an aggregator. | A-Q4, G2-Q4 |
| Q-B1 ★ | Keep a government identity "Tier 1" and the ZK plugin? | Keep as Envisioned, reworded as an optional assurance credential. Delete "Tier 1 is the compromise fallback". | B-Q1 |
| Q-B3 ★ | Which reputation design stays: Layer 3 (one record signed by both) or `[P2P-REP]` (independent halves, EMA)? | `[P2P-REP]` independent halves for signing. Layer 3 vouching stays as the only vouching design. Both Envisioned. Change "the substrate maintains an EMA" to "the app". | B-Q3, G2-Q5 |
| Q-B4 ★ | Which discovery design stays: Layer 3 leaf shards or `[P2P-DSC]` tag routing? | Layer 3 is the main plan. `[P2P-DSC]` stays marked Envisioned. Optionally drop the "Additive, later" paragraph. | B-Q4, G2-3 |
| Q-C2 ★ | Replace the Layer 4 order state machine with the real booking machine? | Yes. Keep dispute, refund, review and consumer cancel as one short Envisioned list. | C-Q2 |
| Q-C3 ★ | Redraw the seven-component diagram with the six real Roym services? | Yes. Keep the DRM service and push notifications as Envisioned items. | C-Q3 |
| Q-C6 ★ | The Roym spec says the gateway binds `127.0.0.1`. The code binds `0.0.0.0`. Is that intended? | Report to the spec owner. Say neither in this doc. | C-Q6, G2 |
| Q-D4 ★ | Delete the Isolation diagram edge `APP1 <-> APP3` and the shared `DB2`? | Yes. One database per service. | D-Q4 |
| Q-F2 ★ | Is a server-side `listen/accept` API still the goal? | No. Keep `connect` and the router-hosted service model. | F-Q2 |
| Q-F3 ★ | Keep the `dial/listen/capabilities` transport interface? | Delete it. Re-add it with a third transport. | F-Q3 |
| Q-F4 ★ | Should the SDK try the next mechanism after a failed dial? This is a code change. | Yes, as a small code fix. Otherwise log it in the backlog. | F-Q4 |
| Q-G1-1 ★ | Should per-service SQLite files run in WAL mode? No pragma is set. This is a code change. | Check by a test. If confirmed, set WAL in code. Until then, remove "(and WAL)" (O60). | G1-Q1 |
| Q-B2 ★ | The router accepts a public key and certificate in the preamble with no proof of possession. Known gap with a plan? Also: the client key in the end-to-end handshake is not signed; the ECDH secret is used as the AES key with no KDF; `pubkey` means Ed25519 in one place and P-256 in another. | Document only what the router checks. Add backlog rows for all four. This is a security point for the owner. (backlog) | B-Q2, E-Q8, E gaps 2 and 3 |

### 2.2 Plain doc fixes (apply the default)

| # | Question | Default | Sources |
| --- | --- | --- | --- |
| Q-A1 | Executive Summary says "no server sits between participants". Reword? | Yes. Say relays and coordinators carry traffic as a fallback. | A-Q1 |
| Q-A2 | Are "Home Services Guild" and "Food & Small Retailer Mesh" still the first verticals? | Keep as product intent, marked Envisioned. Unify the SynApp 1 name (O21). | A-Q2 |
| Q-A3 | Are the RAM figures per tier requirements? | Sizing hints. Reword. | A-Q3 |
| Q-A5 | Keep the substrate version attribute in the Entity Model? | Delete the attribute. | A-Q5 |
| Q-A6 | Does a substrate forward for services it does not host? Should the registry name a coordinator as entry point for private substrates? | Incidental. Describe the entry-point case as "when a record names a coordinator". Backlog row if wanted. (backlog) | A-Q6 |
| Q-A7 | Keep the "community governance key" for the DHT? | Keep inside the bootstrap design as Envisioned. Drop it if that design is dropped. | A-Q7 |
| Q-A8 | Keep the Bootstrap Server design (relay list, `<relaynodeid>.syneroym.net`, 24 h cache, DHT mirror)? | Keep as one Envisioned block. Add a "built today" paragraph (fixed relay config plus community registry). | A-Q8 |
| Q-B5 | The "Catalog Matching" link points to an `ideas/` file. | Remove the link. AGENTS.md forbids it. | B-Q5 |
| Q-B6 | Keep MLS as a long-term option? | No. Delete MLS and `openmls`. Name `vodozemac`. Link ADR-0013 (O27). | B-Q6 |
| Q-B8 | Master Anchor: allow list or deny list? | The code decides. Document the deny list. | B-Q8 |
| Q-B9 | "OS enclave" key storage. | Envisioned note beside the built key file. | B-Q9 |
| Q-B10 | Merge the two "Layer 3 — Shared Substrate Utilities" headings? | Yes. One heading, five sub-sections (Identity, Discovery & Matching, Messaging, Trust & Reputation, Payments). Keep order. | B-Q10, pilot 3.2 |
| Q-C1 | Keep "adapted from the Beckn Protocol"? | Remove the sentence. | C-Q1 |
| Q-C4 | Keep the Recommendation Algorithm section? | Keep, Envisioned, after the real search text. | C-Q4 |
| Q-C5 | The "Key differences from SynApp 1" paragraph names a second SynApp with no heading. | Keep as Envisioned under a "Local Producer-Distributor Mesh" heading. | C-Q5 |
| Q-D1 | Keep the whole provider-facing observability design (`health-narrator`, status UI, Get Help, tiers 2 and 3, aggregator console)? | Yes. Envisioned, one marker, a "what is built" paragraph first. | D-Q1 |
| Q-D2 | Is `?enc=ecdh-p256` a general per-stream option? | Yes, on any transport. Drop "DERP relay". Say who can use it. | D-Q2 |
| Q-D3 | Keep Substrate Integrity & Remote Attestation? | Yes, Envisioned. Reword wRPC to JSON-RPC. | D-Q3 |
| Q-D5 | TBD table: annotate every row or remove rows 5-17? | Annotate. Add a status column. Rows 1-4 built. Rows 5-17 Envisioned, except where TBD-06, 08, 10, 11, 14 describe a smaller built part. | D-Q5 |
| Q-D6 | Merge `[ADV-OBS]` and the Observability section? | No. Mark both Envisioned. Cross-link (O50). | D-Q6, G2 |
| Q-D7 | Podman "rootless, non-root user": enforce or advice? Podman "4.x+": tested minimum? | Operator advice. Write "uses the host's Podman". Drop the version. | D-Q7, L2-Q7, E-Q6 |
| Q-D8 | `MemoryRecorder` never trims histograms. | Backlog row. No code change here. (backlog) | D-Q8 |
| Q-E1 | A guest's own outbound call through its coordinator; substrate finds a coordinator by registry; registry asks its parent on a miss. | Keep all three as Envisioned. Document the built case: SDK dial, `parent_coordinator.iroh.url`, push-only registry. | E-Q1, Q2, Q3 |
| Q-E4 | Delete the Hickory DNS row? | Delete. Iroh owns DNS. | E-Q4 |
| Q-E7 | Do Roym records follow W3C VC 2.0? | Keep `ssi` as Envisioned. Call the built records "signed Roym records". | E-Q7 |
| Q-F1 | One "Envisioned" banner for the Connectivity section? | Yes. Banner names the built parts (Iroh, DHT, preamble). Do not restructure. | F-Q1 |
| Q-F5 | Keep `did:p2p`? | No. Use `did:key:h...` (O18). | F-Q5 |
| Q-G1-2 | WASI env and preopened-file shims. | Keep as one Envisioned line. | G1-Q2 |
| Q-G1-4 | Keep the full Sharded text? | Keep as Envisioned plus one line on what the resolver does (O61). | G1-Q4 |
| Q-G1-5 | Route caches "keyed by `ServiceId + InterfaceName`". | Delete. The proxy re-resolves per call. | G1-Q5 |
| Q-G1-6 | Keep `[PLT-DAT]` parts 4-5 (Arrow, DataFusion, Substrait)? | Keep, Envisioned. Do not decide where it belongs. | G1-Q6 |
| Q-G1-7 | Mixed reader in the addendum bodies. | Keep formulas and WIT where they match code. Plans become Envisioned notes. | G1-Q7 |
| Q-G2-1 | One marker per phase or per block? | One per phase and per `[ADV-*]` block. | G2-Q1 |
| Q-G2-2 | Phase 6 against Roym. | Keep the long-term design as Envisioned. Add a "Built today" paragraph linking the Roym spec. Move nothing. | G2-Q2 |
| Q-G2-3 | Is "losing the supervisor's node costs a rebuild" true only with a master-key backup? | Yes. Add the `export-master` sentence (O65). | G2-Q3 |
| Q-G2-6 | OQ-1 (custom DHT) is resolved. | Mark resolved. Drop "custom". `pkarr` over BEP 0044 is built. | G2-Q6 |
| Q-G2-7 | Glossary stale rows. | Delete rows whose term leaves the body. Mark `wRPC` Envisioned. | G2-Q7 |
| Q-G2-8 | Keep `syneroym-dev-sdk` (mock SDK)? | Yes, Envisioned. | G2-Q8 |
| Q-L2 | Layer 2 pilot Q3-Q8: describe Roym arbitration rules; a secondary device is a client of the primary; operator-named placement is built and resource scheduling is planned; the Roym backup is built and a generic export is planned; "operator cannot override policy" is not claimed. | Already applied in #290. Confirm only. | L2-Q3 to Q8 |

## 3. Milestone IDs, commit hashes and dated notes

Each item breaks README rule 1 (living docs must not cite milestone IDs). Action for all: delete the ID, or replace it with the Envisioned marker or a statement of what is built. ADR references are allowed and stay. In doc order.

| Section (heading) | What it cites |
| --- | --- |
| Top, "Migration Note" | Anchor `#post-dd864a1-target-designs-addendum` (commit-style hash). Claims the addendum is "the canonical Layer 1-4 definition", which is no longer true (O26). |
| Top, Table of Contents | Link "MVP Phase 1 Scope & Acceptance Criteria" (no heading). |
| Layer 1 > P2P Networking: Iroh | Note "MVP focuses on connecting peers over IP". |
| Layer 3 > Discovery & Matching | "**M8 ships:**" (milestone ID). Also "Additive, later". |
| Layer 3 > Payments | "**Payment Strategy (MVP)**" and "MVP focuses on redirection"; link text "see ... in Phase 6". |
| Layer 3 > Identity | "Method A" and "Method B": design names, not IDs. Keep or rename. |
| Appendix > Scenario Entities | "**R** ... (community registry, future DHT)": "future" is wrong. |
| Appendix > step 5 | "the exact mechanism currently implemented in the frontend": a status note. |
| Consolidated Technology Stack | Row "Payment (MVP)". Row "Wasmtime (latest stable ...)": a moving word. |
| Security Architecture, Resolved TBD, Connectivity | None. (Encryption diagram ids M1, M2, M3 are labels, not milestones. Connectivity: only the heading typo.) |
| Addendum heading | "Post-DD864A1 Target Designs (Addendum)": a commit hash as a time marker. |
| Addendum intro | "Implementation Status (updated 2026-07-12, M0–M3B/M3C complete)", "M2", "M3A", "M3B Slice 5", "M3B/M3C Slices 6A/6B", "M3C Slice 7", "M4A/M4B", "M4+", a link to `meta-implementation-plan`. Stale. It is the only text that says the phases are targets. |
| Phase 0 > `[TOP-DSC]` > Master Anchor Resolution | "Phase 0 Contract", "deferred to Phase 1 `[FND-IDT]`". |
| Phase 1 > `[FND-SEC]` | "(M3, ADR-0006)", "**M4 (M04A Slice B6)**". |
| Phase 2 > `[PLT-DAT]` 1 | "deferred to M4", "(M7, see `[PLT-RED]`)", `[PLT-DAP-01]`. |
| Phase 2 > `[PLT-DAT]` 2 | `[PLT-DAP-04]`, `[PLT-DAP-06]`, "M3B", "M3B/M3C", "M4", "M5", "M7", "(M3B Slice 6A)", "(M3C)", "Slice 7; extended by M06A A2", `D-A2-7`, `D-A2-12`, "F5a", "Gap closed (M04A Slice B0)"; links to `M04A-.../status.md`, `M06A-.../slice-a2-implementation-plan.md`, `M03B-messaging/status.md`. |
| Phase 2 > `[PLT-DAT]` 3 | "Implementation status (M04A Slice A1, 2026-07-15)", "Decision Register A.5", `M04A-proxy-and-auth-foundation/task.md`, "(M5)". ADR-0016 §6 stays. |
| Phase 2 > `[PLT-DAT]` 4-5 | `[PLT-DAP-05]`, `[PLT-DAP-02]` (requirement ids). |
| Phase 2 > `[PLT-ASY]` | "Implementation status (M05B, 2026-08-07, ADR-0023)", "M05B slice B3", "M5's final phase". ADR-0023 stays. |
| Phase 2 > `[PLT-RED]` | "M3B", `[PLT-DAP-03]`, `[PLT-DAP-04]`. |
| Phase 3 > `[LFC-MGT]` 2 | "replication (Iroh WAL shipping) does not arrive until **M7**". |
| Phase 3 > `[LFC-VER]` | None in the doc. Code: `TODO(M5)` at `crates/sandbox_wasm/src/engine/lifecycle.rs:122` (backlog row at `deferred-backlog.md:71`). Same fix applies in code. |
| Phases 3 to 7 headings | "Phase 3" to "Phase 7" are planning-phase numbers used as section titles. Decide: rename by topic, or add one note that they are targets. |
| Open Questions > OQ-10 | "Sequenced after **Phase 1**": points at no document. |
| Design ids | `[PLT-RED]`, `[TOP-*]`, `[FND-*]`, `[PLT-DAP-nn]`: design and requirement ids. The anchors work. Keep the section ids. Decide on the `[PLT-DAP-nn]` requirement ids (G1 lists them as breaking the rule). |

## 4. Proposed order of fix commits

Rule: work from the bottom of the doc to the top. An edit near the end then never moves the line numbers of a heading you have not reached. Group by heading. New "gap" text (Part 5) goes into the commit of the heading that is its proposed home. Questions with a star are settled before the commit that needs them.

Line numbers are the current heading lines, as hints only.

| Commit | Scope (headings) | Main work |
| --- | --- | --- |
| 1 | Glossary (2370) and Open Questions & Recommendations (2353) | Delete stale Glossary rows (O24). Mark `wRPC` Envisioned. Resolve OQ-1. Fix OQ-5, OQ-6 (`SynExport`), OQ-7, OQ-2, OQ-10 (Part 3). |
| 2 | Phase 7 (2338) and Phase 6 (2286) | Markers. "Built today" paragraphs. Aggregator (Q-A4). Payments, escrow, WebSocket and Hub-login copies (O40, O41, O47, O48). |
| 3 | Phase 5 (2269): `[P2P-DSC]`, `[P2P-REP]` | Mark Envisioned. Fix "mutually signed". Change "the substrate" to "the app" (Q-B3, Q-B4). |
| 4 | Phase 4 (2204): `[ADV-OBS]`, `[ADV-AI]`, `[ADV-DEV]` | Markers. Cross-link with Observability (O50, O52). Rename the "Observability Engine". Add dual build and saga to `[ADV-DEV]` (gap 20). |
| 5 | Phase 3 (2159): `[LFC-MGT]`, `[LFC-VER]` | Fix "controller", `resolve`, key custody, `M7`, migration hooks (O63 to O66). Add supervisor verbs and topology documents (gaps 7, 8, 23). |
| 6 | Phase 2 (1956): `[PLT-RED]`, `[PLT-ASY]`, `[PLT-DAT]` 5 to 1 | Fix O56 to O60 and Part 3 IDs. Add MQTT namespace and `call_dedup` (gap 17). Keep Litestream as an option (Q-1 decision); fix only wrong statements. |
| 7 | Phase 1 (1905): `[FND-SEC]`, `[FND-CFG]`, `[FND-IAM]` | Part 3 IDs. Attestation wording (O30). Add stage-4 ABAC and FDAE masks (gap 18). |
| 8 | Phase 0 (1852): `[TOP-PRM]`, `[TOP-ADR]`, `[TOP-REG]`, `[TOP-DSC]`, `[TOP-ROB]` | Master Anchor (O28). Sharded (O61). Connection reuse (O62). Two-tier discovery (gap 7). |
| 9 | Addendum heading and intro (1841) | Remove "Post-DD864A1" and the dated status note (Part 3). Demote the second `#` title (O26). Keep one note that the phases are targets. |
| 10 | Connectivity Substrate (1373) | Rename "Heterogeneous". Banner. `did:key`, wRPC, HELLO, gateway, `listen/accept`, retry (O8, O15 to O20). Add registry-first discovery, freshness, visibility (gap 6). |
| 11 | Consolidated Technology Stack (1310) | Libraries (O27, O39, O40, O51), Hickory, Litestream, "iroh-net". Add toolchain list (gap 24). |
| 12 | Appendix: Multi-Hop Relay Walkthrough (1216) | O2 to O7, O11, O19. Merge step 1 and step 2 registration. |
| 13 | Resolved Architecture TBD Items (1190) | Status column. Fix rows 5-17 (O37 to O39, O43). TBD row 16 (O9). |
| 14 | Security Architecture (1107) | Messaging crypto (O27). Marker for R2 (O32). Attestation (O30, O31). Isolation diagram (Q-D4). Add key table and `enc` note (gaps 9, 12, 18). |
| 15 | Observability Architecture (999) | Marker. "What is built" paragraph. Simulation claim (O54). Add alerts and real metrics (gap 19). |
| 16 | Consumer Experience (970) and Federation Architecture (924) | Fix O36, O45, O46, O47. Trim shard detail. |
| 17 | Layer 4 (745): Component Architecture, Order State Machine, Consumer Transaction Flow, Recommendation Algorithm | O40 to O44, O47. Redraw (Q-C2, Q-C3). Add cards, booking tracks, slot claiming (gap 21). |
| 18 | Layer 3, second heading (576): Discovery & Matching, Messaging, Trust & Reputation, Payments | O9, O27, O36 to O40, O43. Remove `M8 ships` and the `ideas/` link. Add group chat controls, safety rules, data lifecycle, SynOrg (gaps 13, 22). |
| 19 | Layer 3, first heading (469): Identity, Method A, Method B, Master Anchor | Merge the two headings (Q-B10). O28, O29. Add signed records and certificate scopes (gap 11). |
| 20 | Layer 2 (238) leftovers | Access control "must carry a delegation certificate" (O34). Litestream note under Storage (Q-1). Add node ownership, gateway and auth, failure and shutdown, upgrade, limits, deployment profiles, local-only admission (gaps 1, 2, 3, 5, 10, 15, 16). |
| 21 | Layer 1 (141) | O1 to O5, O9 to O12, O67. Add browser path, relay and registry config, third-party relay default (gaps 14, 15). |
| 22 | System Layers Overview and Conceptual Entity Model (69), Architecture Goals & Constraints (46), Executive Summary (35) | O13, O21 to O25. Q-A1 to Q-A5. Stale entities. |
| 23 | Top matter (1 to 34): status legend, Migration Note, warning, Table of Contents | O1 warning. O26 note. Remove the hash anchor. Fix and extend the TOC (O20). |
| 24 | Outside the doc | `docs/TERMINOLOGY.md` "controller" (O64). Req spec relay and bootstrap (O10). Backlog rows (Q-B2, Q-D8, Q-A6, O67, Q-F4, Q-G1-1). Code `TODO(M5)` (Part 3). Roym spec gateway bind note (Q-C6). |

## 5. Gaps, merged and ranked

Ranked by risk to a reader: trust boundaries and failure behavior first, then wire formats, then product rules, then operations. Sources: `GAP-n` is a row of the gap report; `A-4.n` is gap n of batch A; `G1-4.n` is G1 section 4 item n.

| Rank | Gap | Sources | Proposed home | Status |
| --- | --- | --- | --- | --- |
| 1 | Node ownership: `ControllerAgreement`, `roymctl substrate claim`, `substrate/admin` for the controller, `admin_ucan_root` as fallback, `supervisor/resolve` grant. | GAP-1, B-4.1 | Layer 2 > Access control | Implemented |
| 2 | Client gateway identity modes (`Open`, `Login`, `Fixed`), the auth service (nonce, delegated-key login, session cookie the router verifies), Hub login (ADR-0024). | GAP-2, B-4.2, C-4.7, G2-4.5 | Layer 2, new "Client gateway and Auth service" | Implemented |
| 3 | Failure and shutdown: one `select!`; any component exit stops the substrate; shutdown awaits the supervisor loop only. | GAP-3 | Layer 2, new "Failure and shutdown" | Implemented |
| 4 | Messaging crypto that is actually built (`vodozemac`, owner-distributed epoch key, ADR-0013 Amendment 1). The doc names libraries that are in no `Cargo.toml`. | GAP-12, B-4.7 | Layer 3 > Messaging, Technology Stack | Implemented, differs from doc |
| 5 | Upgrade and versioning as built: `init()` and `migrate()` only, no snapshot or rollback, fixed ALPN `syneroym/0.1`, versioned formats (`master_anchor_v1`, `ENVELOPE_VERSION`, `IDENTITY_BACKUP_VERSION`). | GAP-4 | `[LFC-VER]` markers; Layer 2 "Upgrade and versioning" | Partial |
| 6 | Discovery as built: community registry first, DHT fallback with write-back, freshness (1 h republish, 2 h registry TTL, 30-day `not_after`, last writer wins), record visibility (`Private`, `Internal`, `Public`), one-shot coordinator registration. | GAP-5, GAP-6, A-4.3, E-4.1, E-4.5, F-4.3 to 4.5 | Layer 1, new "Community registry and record freshness" | Implemented |
| 7 | Two-tier logical discovery: `AppScope`, `AppDid`, signed `TopologyDocument` from `supervisor.resolve`, logical name resolution in the app supervisor, ADR-0022 (status Proposed, code exists). | GAP-7, F-4.1, G1-4.2, G2-4.3 | `[LFC-MGT]` 3; `[TOP-ADR]` | Implemented |
| 8 | Supervisor surface: about 17 verbs, per-instance generation stamp, master-key vault, mandatory `export-master`, `roymctl app health` and `app alerts`. | GAP-8, G2-4.1, 4.2, 4.4 | `[LFC-MGT]` 1-2 or Layer 2 "App Supervisor" | Implemented |
| 9 | Key inventory and loss behavior: node identity, person master and temporary keys, node KEK (`roymctl kek inject`), per-service DEK, per-instance HKDF KEK, vault, recovery key; anchor freshness duty (24 h, republish 12 h). | GAP-9, D-4.6, B-4.3 | Security Architecture, new table "Keys: location, use, loss" | Implemented |
| 10 | Local-only admission in Roym: every service but `directory` answers `-32013` to a non-local caller; `directory` admits four wire methods. | GAP-10, C-4.2 | Layer 2 > Access control | Implemented |
| 11 | Signed record envelope and `syneroym:signing` boundary; certificate scopes (`routing`, `session-auth`, `service-instance`, `record-signing`); UCAN chains in the preamble. | GAP-11, B-4.4 to 4.6 | Layer 3 > Identity, new "Signed records" | Implemented |
| 12 | `?enc=ecdh-p256`: no client requests it; secret used as the AES key with no KDF; `pubkey` has two meanings; cannot combine with `delegation`. | GAP-15, D-4.8, E-4.2, E-4.3, F-4.6 | Security > Encryption at Every Layer | Partial |
| 13 | Roym safety and data lifecycle: group epoch rekey, signed membership events, first-contact admission and holds, deletion requests, block, report, contact and publication limits; undeploy data rule unverified. | GAP-13, B-4.7, C-4.8, G2-4.7 | Layer 3 > Messaging; Layer 2 storage note | Implemented (undeploy unverified) |
| 14 | Browser fallback path: WebRTC coordinator bootstrap page, service worker, `peer-proxy.js`, WebSocket blind tunnel to an Iroh node. | GAP-14, A-4.1 | Layer 1, new "Browser path" | Implemented |
| 15 | Relay and deployment config as built: fixed `parent_coordinator` config, third-party N0 relay default, coordinator mode (mock registry, embedded MQTT), `/v1/info`, `max_connections`, `minimal` and `default` features, Docker image, TLS reload on `SIGUSR1`, `796x` ports; config flags that do nothing. | GAP-16, A-4.2, 4.4 to 4.7, E-4.4, F-4.8 | Layer 2, new "Deployment profiles"; Layer 1 | Implemented |
| 16 | Limits and budgets in one table: stream cap 8, proxy timeout 30 s, queued call 256 KiB, preamble line 256 KiB, scheduler (16 services, 10 s default, 30 s max, UTC cron), SDK connect 10 s. | GAP-17, F-4.7, G1-4.6 | Layer 2, new "Limits and budgets" | Implemented |
| 17 | Messaging namespace and receiver idempotency: topic prefix `svc/<service_id>/` (subscribe may cross, publish may not); `call_dedup`. | GAP-18, G1-4.3, 4.7 | `[PLT-DAT]` 2, `[PLT-ASY]` | Implemented |
| 18 | Isolation and row policy as built: pooling allocator, fuel and epoch limits, empty `WasiCtx`, node-interface denial, same-service gate; stage-4 ABAC `authorize-rows`; FDAE column masks and caveats. | D-4.5, G1-4.1, 4.4 | Security > Isolation Guarantees; `[FND-IAM]` | Implemented |
| 19 | Control-plane health and real metrics: `StatusQuery`, `AlertStore`, `AlertKind`; metric names (`substrate.*`), 1 s sampler; `[roles.observability]` config. | GAP-19, D-4.1 to 4.3 | Observability Architecture, new "Control-plane health" | Implemented |
| 20 | Dual build (WASM and native from one tree) and the saga primitive. | GAP-20, G2-4.8 | Layer 2 > Sandboxes; `[ADV-DEV]` | Implemented |
| 21 | Roym transaction rules: seven card types, payment and fulfilment tracks (30-day window, "against interest"), first-claim slot fence (`slot-taken`, `slot-unavailable`), single-writer `booking-progress`. | C-4.1, 4.3, 4.4, 4.6 | Layer 4 (with the real booking machine) | Implemented |
| 22 | SynOrg directory: membership credentials, revocation, suspend and lift, moderation records, consumer pins which group a directory speaks for. | B-4.8, C-4.5, G2-4.6 | Layer 3 > Trust & Reputation; Federation | Implemented |
| 23 | Binding-write epoch compare (`Applied`, `NoOp`, `Conflict`, `Stale`, ADR-0021) and the `websocket` HTTP route target. | G1-4.5, 4.8 | `[LFC-MGT]` 3; Layer 2 > ingress | Implemented |
| 24 | Toolchain and version drift: `mise.toml` tools, Playwright, Vitest, `wit-bindgen` 0.55 against 0.57, stale `webrtc` comment in `Cargo.toml`. | E-4.7, E-4.8 | Technology Stack > Developer Toolchain | Implemented |
| 25 | SDK dial: the record has a `WebRtc` mechanism but the Rust client skips it; no retry or fallback between mechanisms. | A-4.8, F-4.2, F-D6 | Connectivity; backlog (Q-F4) | Partial |
