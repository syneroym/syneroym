# Fix coverage D2: overlaps report section 1 (rows O1 to O67)

Reader: a developer who checks that the architecture doc fixes match what the audit decided.

I checked each row of section 1 of [audit-architecture-overlaps.md](audit-architecture-overlaps.md) against the current text of `docs/system-architecture.md` (read in full), the Decisions table in [change.md](change.md), `docs/planning/deferred-backlog.md`, and `docs/TERMINOLOGY.md`. I did not trust the progress notes. I changed no doc and no code.

## 1. Summary

| Verdict | Count |
| --- | --- |
| APPLIED | 58 |
| PARTLY | 2 |
| NOT APPLIED | 0 |
| OVERTAKEN (doc follows the later decision) | 6 |
| NOT APPLICABLE (change is outside the doc) | 1 |
| Total | 67 |

Extra checks that passed: all in-document links of the doc resolve to a heading; all relative file links and cross-file anchors resolve; no other living doc links to a removed heading of the architecture doc.

## 2. Rows

| Item | Verdict | Doc place | Note |
| --- | --- | --- | --- |
| O1 | APPLIED | Multi-Hop Relay: "The connection router (`crates/router`) handles next-hop forwarding..." | Top warning now covers wRPC only. `relay_to_next_hop` is at `crates/router/src/route_handler/io.rs:471`. |
| O2 | APPLIED | Multi-Hop Relay; Appendix > Scenario Entities | `hop-relay` is gone. Both places say "next-hop forwarding". |
| O3 | APPLIED | Multi-Hop Relay: "The next hop is the target substrate, not another coordinator"; Appendix step 6 | |
| O4 | APPLIED | Multi-Hop Relay: "A coordinator is the entry point only when an SDK call names it..."; Appendix Envisioned notes | The "record names a coordinator" case is under Envisioned markers. |
| O5 | APPLIED | Multi-Hop Relay: "No code dials the parent on demand"; Appendix 1.2 "Cp makes no call to its parent" | "outbound-only" and "permanent" claims are gone. |
| O6 | APPLIED | Appendix "2. Registry Entries at Deployment", item 3 "Cp Registration" | The step 1 sentence is deleted. One registration, once, 2 hour expiry. |
| O7 | APPLIED | Multi-Hop Relay: "end-to-end encrypted ... only when the preamble asks for `enc=ecdh-p256`"; Appendix step 3.5 and "5. Data Transfer" | |
| O8 | APPLIED | Multi-Hop Relay (substrates forward); Connectivity > Gateway Nodes: "> **Envisioned.** Not built yet..." | |
| O9 | APPLIED | Bootstrap Server & DHT Fallback (built paragraph, then Envisioned block); Discovery; TBD row 16; Tech Stack "DHT / registry" | Row 16 has a Built part and an Envisioned part. |
| O10 | NOT APPLICABLE | Req spec outside the doc | Decision says no spec edit in this round. Backlog row exists: `deferred-backlog.md:271`. The doc side is done (Layer 1 Envisioned blocks). |
| O11 | APPLIED | P2P Networking diagram (no TURN edge); Relay Node Architecture (Envisioned); Appendix entity C; Browser Path Envisioned note "A TURN relay for WebRTC" | |
| O12 | APPLIED | P2P Networking: Iroh; Tech Stack "P2P / relay"; Security T2 | No "DERP" and no "iroh-net" left. Glossary DERP row is gone. |
| O13 | APPLIED | Executive Summary: "A direct connection between two participants needs no server in the data path..." | |
| O14 | APPLIED | Relay Node Architecture: "It still sees which endpoints talk, and the size and timing"; Multi-Hop Relay last paragraph | |
| O15 | APPLIED | Connection Establishment: Envisioned "Try each path until one connects" | Backlog row exists: `deferred-backlog.md:265`. |
| O16 | APPLIED | Protocol Negotiation (preamble text; HELLO gone); `[LFC-VER]` 2 Envisioned | |
| O17 | APPLIED | Application Interface: "A WASM service has no listen or accept call..." | Follows Q-F2/Q-F3 wording. Transport Layer repeats it. |
| O18 | APPLIED | Connectivity > Identity Model: `did:key:h<z-base-32 public key>` | No `did:p2p` left. |
| O19 | APPLIED | Service Record (`protocols` field under Envisioned); Protocol Adaptation Envisioned; Appendix 5.2; Tech Stack Envisioned note; Glossary wRPC note | |
| O20 | APPLIED | Heading "Connectivity Substrate In Heterogeneous networks"; Table of Contents | Spelling fixed. The three bad TOC links are gone. Appendix and addendum are in the TOC. All anchors resolve (script check). |
| O21 | APPLIED | Executive Summary; System Layers diagram; Layer 4 "SynApp 1: Roym" | Verticals are in an Envisioned block. |
| O22 | OVERTAKEN | Conceptual Entity Model text; Federation; Phase 6 item 7 | Q-A4: an aggregator is a `directory` SynOrg service. The `hosts-for` edge is gone. Doc follows the decision. |
| O23 | PARTLY | Key Hardware Constraints; Tiered Observability Stack; `[LFC-MGT]` 4 | See Misses 2. The identity use is gone and the phone tier is Envisioned. A new second meaning ("Tier 1/Tier 2" lookups) now exists. change.md Deviations admits this. |
| O24 | APPLIED | Conceptual Entity Model; Glossary (8 rows) | `SYN-MOD`, `HOME_RELAY`, `SYN-SVC`, `Space`, `DERP` rows and terms are gone. `CRDT` and `MLS` appear only in negative statements. |
| O25 | APPLIED | whole doc | No `CRSQL` left. |
| O26 | APPLIED | Migration Note; "Syneroym: Substrate Feature Implementation Design" is now level 3 | Note says Layer 1 to 4 are canonical. Old hash anchor is gone. |
| O27 | APPLIED | Layer 3 Messaging diagram and Libraries; Security diagram and bullets; Tech Stack rows; Consumer App diagram | `vodozemac` and the epoch key. ADR-0013 linked in three places. `libsignal` and `openmls` appear only in "not used" sentences. |
| O28 | APPLIED | `[TOP-DSC]` "Master Anchor Resolution"; Layer 3 Master Anchor | Deny list documented. "Phase 0 Contract" text is gone. |
| O29 | APPLIED | Layer 3 Identity Method B (Envisioned) | Reworded as an optional assurance credential. Matches `[FND-IDT]`. |
| O30 | APPLIED | Substrate Integrity & Remote Attestation: Envisioned marker; "over its JSON-RPC interface" | |
| O31 | APPLIED | Same section: "The word 'attestation' in this subsection means hardware proof..." | |
| O32 | APPLIED | Encryption at Every Layer: "Box R2 (replicated backups) only" | |
| O33 | OVERTAKEN | Layer 2 Packaging and Storage; Security; Tech Stack; Developer Toolchain; `[PLT-RED]` | Decision Q-1 reversed the default. All places keep Litestream as an open option, under Envisioned markers. `[PLT-RED]` says "not frozen". |
| O34 | APPLIED | Layer 2 Access control item 1; `[PLT-DAT]` 2 (interim paragraph deleted) | A key with no certificate is accepted as its own master key. |
| O35 | OVERTAKEN | Isolation Guarantees diagram | Q-D4: keep the edge, relabel. Doc: "cross-app calls through the substrate proxy, subject to access control". `DB2` is "one database per service". |
| O36 | OVERTAKEN | Discovery & Matching; Cross-Substrate Discovery Flow; `[P2P-DSC]`; TBD rows 14, 15, 17 | Q-B4: the `directory` model is the main design. Leaf shards and tag routing are Envisioned options. |
| O37 | OVERTAKEN | Trust & Reputation; `[P2P-REP]`; Federation contract item 4 | Q-B3: principles only, both designs are candidates. `[P2P-REP]` says "each in its own copy" and "The app maintains a rolling EMA". |
| O38 | APPLIED | Trust & Reputation (Envisioned block); TBD rows 5 to 10 with Status column | |
| O39 | APPLIED | TBD row 6; Tech Stack "Verifiable Credentials ssi" | Both name `ssi`. No `didkit`. |
| O40 | APPLIED | Layer 3 Payments; Consumer Transaction Flow; Tech Stack rows; Phase 6 item 3; TBD rows 11, 12 | Built records first. Stripe flow only in Envisioned notes. |
| O41 | APPLIED | Booking State Machine diagram | No DISPUTE or REFUNDED state. Dispute is in the Envisioned list. |
| O42 | APPLIED | Booking State Machine | Real six-state booking machine. "Order state" rule is Envisioned. |
| O43 | APPLIED | Discovery & Matching; Recommendation Algorithm; TBD rows 9, 15, 17 | The cap `0.3` is written once (row 15). The search-versus-recommendation difference is stated in three places (Discovery & Matching, Recommendation Algorithm, TBD row 13). This is harmless. |
| O44 | APPLIED | Recommendation Algorithm: "Built today" first, then Envisioned | |
| O45 | APPLIED | Minimum Federation Contract item 5; OQ-5; OQ-6 | "Roym archive" in all three. |
| O46 | APPLIED | Consumer App Architecture; Tech Stack Consumer Frontend; Phase 6 item 0; OQ-7 | Each has a "Built today" part and an Envisioned part. |
| O47 | APPLIED | Component Architecture diagram; Consumer App diagram; Phase 6 item 0; Tech Stack External API | All say JSON-RPC 2.0 over HTTP `POST /rpc`. No gRPC left. |
| O48 | APPLIED | Phase 6 item 0; OQ-2; Consumer App Architecture | Delegated key and ADR-0024. |
| O49 | OVERTAKEN | Client Gateway and Auth Service | Q-C6 plus the code fact (`crates/client_gateway/src/gateway.rs:184` binds `0.0.0.0`, the port is the only setting). The doc states no bind address. Backlog row exists: `deferred-backlog.md:267`. |
| O50 | APPLIED | Provider-Facing Observability (second design note); `[ADV-OBS]` (Envisioned, `Metrics Pipeline`) | Both Envisioned and cross-linked. Engine renamed. |
| O51 | APPLIED | Tech Stack "Observability"; Instrumentation Layer; Tech Stack OTLP note | |
| O52 | PARTLY | `[ADV-OBS]` 4 "Data Consumption"; Provider-Facing Status UI | The clash is narrowed to "the Metrics Pipeline". "Pick one" was not done. A stray "Instead," is left. See Misses 1. |
| O53 | APPLIED | Provider-Facing Observability: Envisioned marker | |
| O54 | APPLIED | Simulation Testing and Replay Validation | Whole section is Envisioned. The "each write rule" claim says no scenario exists. |
| O55 | APPLIED | TBD rows 1 to 4 | No action asked. Rows still match Layer 2 and Layer 4. |
| O56 | APPLIED | `[PLT-DAT]` 2 "Pub/Sub Execution Flow" | "P2P Overlay" text is gone. Log replication is in an Envisioned block. |
| O57 | APPLIED | `[PLT-DAT]` 3; `[PLT-ASY]` "Resilient RPC & Dead Letter Queues" | The "DLQ does not exist" note is gone. |
| O58 | APPLIED | `[PLT-ASY]` | Status note folded into the body. Leases are gone. Client outbox is Envisioned. |
| O59 | APPLIED | `[PLT-RED]` "Registry & Coordination Model" | "The App Supervisor, not a separate registry service, would act as...". |
| O60 | APPLIED | `[PLT-DAT]` 1 "Database Isolation"; Envisioned "WAL mode and tuning" | The `data_db` crate sets no `journal_mode` (grep of `crates/data_db/src`). `async_queue/src/queue.rs:619` sets WAL for `async.db`, as the doc says. |
| O61 | APPLIED | `[TOP-ADR]` Selection Topology and Envisioned block | |
| O62 | APPLIED | `[TOP-ROB]` "Connection Handling"; Envisioned "Connection reuse" | |
| O63 | APPLIED | `[LFC-VER]` 1 | `init()`/`migrate()`/`execute-ddl`. Snapshot, rollback and replication epoch are Envisioned. |
| O64 | APPLIED | `[LFC-MGT]` 1; `docs/TERMINOLOGY.md` App Supervisor entry | "controller" is used only for the node owner. The `resolve` verb is described. |
| O65 | APPLIED | Layer 2 Keys list; `[LFC-MGT]` 1 "Key custody" and 2 | `export-master` sentence is in both. |
| O66 | APPLIED | `[LFC-MGT]` 2: "replication is not built (see [PLT-RED])" | No milestone id left. |
| O67 | APPLIED | Relay and Registry Configuration Envisioned note; Connectivity Overview Envisioned note | Backlog row exists: `deferred-backlog.md:268`. |

## 3. Misses

Both misses are low importance. No NOT APPLIED item was found.

1. **O52, "Where visuals live" (PARTLY).**
   - What is missing: the audit says "Pick one". The doc still has two unbuilt designs. `[ADV-OBS]` has no dashboards. Provider-Facing Observability has a status page at `/admin`. The wording no longer clashes, but the sentence in `[ADV-OBS]` 4 "Data Consumption" reads badly. It now starts the second sentence with "Instead," which no longer follows the first.
   - Exact change (safe, no design choice): in `[ADV-OBS]` 4, replace the bullet text with: "**Data Consumption**: The Metrics Pipeline does not host its own visualizations. Standalone SynApps or dedicated BI tools, acting as external clients, use the metric data. The small provider status page in [Provider-Facing Observability](#provider-facing-observability) is a separate design. It reads the `health-narrator` state, not `metrics.db`."
   - Code check: nothing to check. Neither design is built (no `/admin` route and no `metrics.db` in `crates/`).
   - Owner choice still open: whether one design replaces the other. Ask the owner. If no choice is wanted now, record "kept both" as a Deviation in change.md (it is already there under commit 4).

2. **O23, three meanings of "Tier" (PARTLY).**
   - What is missing: the identity use is gone and the phone tier is Envisioned. But `[LFC-MGT]` 4 now uses "Tier 1" and "Tier 2" for the two lookups, and the doc also uses "Tier 1/2/3" for hardware (Key Hardware Constraints, Tiered Observability Stack). change.md Deviations (commit 22) names this and left it.
   - Exact change: in `[LFC-MGT]` 4, line "finds a logical service through two tiers of lookup" becomes "finds a logical service through two lookups". Rename "**Tier 1: app DID → supervisor.**" to "**First lookup: app DID → supervisor.**" and "**Tier 2: topology document.**" to "**Second lookup: topology document.**". In the `roymctl app resolve` line, "runs Tier 1 and Tier 2" becomes "runs both lookups". In `[TOP-ADR]` "Callers Outside the App", "The two-tier lookup is in" becomes "The two lookups are in". Keep the ADR-0022 link text, because the ADR file name says "two-tier".
   - Code check: none needed (names only).

## 4. Other problems noticed

1. **Built text names an unbuilt UI.** Layer 2 > Substrate API Surfaces says the JSON-RPC surface "serves ... the CLI, browsers, the provider status UI and third-party integrations". The doc says elsewhere (Provider-Facing Observability, first block) that there is no status page. A grep for `"/admin` in `crates/*.rs` finds no route. Fix: delete "the provider status UI,".
2. **Built facts under an Envisioned marker.** Consumer App Architecture, the block after "Consumer identity options": the Envisioned blockquote ends with "Today a substrate can hold a delegated instance key for a member it hosts, and Roym has export and import. Every Hub method that `web` forwards, except `profile.policy`, needs an owner session. `session.whoami` is answered without a session." These are built facts. The same session fact is already in the Layer 4 Component Architecture table (`web` row). Fix: move the three sentences above the marker, or delete them.
3. **Repeated sentence.** Layer 2 Substrate Internal Architecture, Sandboxes, Podman bullet: "The substrate calls the host's `podman` command" appears twice in one bullet (first in parentheses with `podman run -d --network bridge`, then again after "does not check this"). Fix: delete the second copy.
4. **Glossary term not used.** Glossary row `LWW` defines an abbreviation that the body never uses (the body writes "last write wins" in words). Fix: delete the row, or use "LWW" once in Storage & Write Arbitration.
5. **Stray "Instead," in `[ADV-OBS]` 4.** Covered by Misses 1.
