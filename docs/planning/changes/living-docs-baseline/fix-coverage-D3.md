# Fix coverage D3: overlaps report (decisions, questions, milestone ids, gaps) and gaps report

Reader: the owner of the architecture fix, who must decide what is still missing.

Checked against: `docs/system-architecture.md` in the worktree (branch docs/architecture-fix), by reading the whole doc and by grep. No file was edited. Sources: `audit-architecture-overlaps.md` (sections 2.0, 2.1, 2.2, 3, 5), `audit-architecture-gaps.md` (ranked table, ADR list), and the Decisions and Deviations of `change.md`. Decisions win over audit defaults.

## 1. Summary

| Verdict | Count |
|---|---|
| APPLIED | 142 |
| PARTLY | 6 |
| NOT APPLIED | 0 |
| OVERTAKEN | 2 |
| NOT APPLICABLE | 2 |
| **Total rows** | **152** |

Rows per section:

| Section | Rows |
|---|---|
| A. Section 2.0 decisions (14 table rows) and the two extra decisions in change.md | 16 |
| B. Section 2.1 questions needing a decision (15 rows; 2.0 wins) | 15 |
| C. Section 2.2 plain doc fixes (39 rows) | 39 |
| D. Section 3 milestone ids, hashes, dated notes (25 rows) | 25 |
| E. Section 5 merged gaps (25 rows) | 25 |
| F. Gaps report (audit-architecture-gaps.md): ranked table, 20 rows | 20 |
| G. ADRs the gaps report lists as not (fully) reflected (12 rows) | 12 |

The two OVERTAKEN rows are one finding (gateway bind address, see "Owner question" after Misses). The PARTLY rows are four findings (Misses 1 to 4). The one NOT APPLICABLE row that was not done is Misses 5. The ADR-0012 row is NOT APPLICABLE because the gaps report itself says to omit it.

## 2. Items

### A. Section 2.0 decisions (14 table rows) and the two extra decisions in change.md

| Item | Verdict | Doc place | Note |
|---|---|---|---|
| 2.0 Q-1 Litestream | APPLIED | Layer 2 Storage & Write Arbitration, Envisioned note ("Litestream stays an option"); Security Encryption, note under diagram ("Litestream and Iroh WAL shipping ... both options"); Tech Stack Envisioned ("Option 1 is Litestream"); Developer Toolchain note; [PLT-RED] ("Litestream stays an option") | All four places keep both options. No text names one design as chosen. The Layers Overview note says "The replication design is open". |
| 2.0 Q-2 matching, reputation, payments | APPLIED | Layer 3 intro ("describe Roym features"); System Layers diagram (Roym box holds Discovery, Trust, Transactions); Layer 2 "Not substrate components" | Roym wording, Envisioned kept, removed from substrate diagrams. |
| 2.0 Q-C2, Q-C3 order machine, component diagram | APPLIED | Layer 4 "Booking State Machine" (six states); "Component Architecture" (six services, new flowchart) | DRM, push, payment adapters and review service sit in the Envisioned note after the diagram. |
| 2.0 Q-C6 gateway bind address | OVERTAKEN | Layer 2 "Client Gateway and Auth Service" first paragraph ("The gateway sets no rule about which machine may connect") | The decision says "say the bind address is configurable". The code makes only the port a setting: `crates/core/src/config/roles.rs:375-400` has `http_port` and no address field, and `crates/client_gateway/src/gateway.rs:184` is `format!("0.0.0.0:{}", self.port)`. The doc follows the code (change.md, commit 20) and states no address (no `127.0.0.1` in the doc). Backlog row exists: `deferred-backlog.md:267`. |
| 2.0 Q-B2 router proof of possession, handshake | APPLIED | Layer 2 Access control item 1; Identity Method A (Envisioned block); [FND-IAM] "Caller Identity at the Router"; Security "Optional end-to-end stream layer" bullets and "What the router checks about a caller" | States the certificate, scope and revocation checks, no proof of possession, one-sided handshake, only when `enc=ecdh-p256` is set. Stronger behaviour is in Envisioned blocks. |
| 2.0 Q-F4 SDK fallback | APPLIED | Connectivity "Connection Establishment" ("Try each path until one connects" in Envisioned block) | Backlog row `deferred-backlog.md:265`. |
| 2.0 Q-A4 aggregator | APPLIED | Entity Model text after diagram ("An aggregator is a `directory` service of a SynOrg"); Federation "Cross-Substrate Discovery Flow"; Phase 6 item 7 | No `hosts-for` edge. Federation and query proxying are in Envisioned blocks. |
| 2.0 Q-B1 government identity | APPLIED | Identity, "Zero-Knowledge Architecture (Method B)" (Envisioned) | "Tier 1 is the compromise fallback" is gone. Master Key Compromise points to [FND-IDT]. |
| 2.0 Q-B3 reputation not frozen | APPLIED | Trust & Reputation "Principles"; [P2P-REP] "Principles"; Resolved TBD rows 5 and 7 ("Candidate design, not final") | Both designs are candidates, Envisioned, not final. No choice made between them. |
| 2.0 Q-B4 discovery model | APPLIED | Cross-Substrate Discovery Flow "Built today"; Discovery & Matching; [P2P-DSC] "Built today" | Leaf shards and tag routing are "one option ... not the plan". |
| 2.0 Q-D4 Isolation edge | APPLIED | Security "Isolation Guarantees" diagram: edge `APP1 <-> APP3` "cross-app calls through the substrate proxy, subject to access control" | Edge kept and relabelled. `DB1` and `DB2` are each "one database per service". |
| 2.0 Q-F2, Q-F3 listen/accept, transport interface | APPLIED | Connectivity "Application Interface" and "Transport Layer" | No `dial/listen/capabilities` text. The node accepts inbound streams on Iroh QUIC and WebRTC and hands them to the router. Callers connect. Services never accept. |
| 2.0 Q-G1-1 WAL | APPLIED | Layer 2 Storage (Envisioned note); [PLT-DAT] 1 "Database Isolation" and Envisioned bullet "WAL mode and tuning" | One writer task, no WAL pragma. Code check: only `async.db` sets WAL (`crates/async_queue/src/queue.rs:619`); `crates/data_db` sets no `journal_mode`. Backlog `deferred-backlog.md:79`. |
| 2.0 Q-B2 remaining parts | APPLIED | Security Encryption bullets "Only the node is authenticated", "No key derivation step", "One field, two uses" | Backlog rows `deferred-backlog.md:172` and `:173` (the three handshake points). |
| change.md Decisions: [PLT-DAP-nn] ids | APPLIED | Headings and bullets of [PLT-DAT] and [PLT-RED] keep the ids | Kept as decided. |
| change.md Decisions: requirements spec relay text (O10) | APPLIED | Not in the architecture doc | `git diff main..HEAD` on `docs/system-requirements-spec.md` is empty. Backlog row open at `deferred-backlog.md:271`. |

### B. Section 2.1 questions needing a decision (15 rows; 2.0 wins)

| Item | Verdict | Doc place | Note |
|---|---|---|---|
| Q-1 Litestream | APPLIED | see 2.0 Q-1 | 2.0 overrides the default (options stay open). |
| Q-2 Roym or substrate | APPLIED | see 2.0 Q-2 | Default and decision agree. |
| Q-A4 aggregator | APPLIED | see 2.0 Q-A4 | 2.0 overrides the default (directory-type SynOrg service, not "a provider"). |
| Q-B1 government identity | APPLIED | see 2.0 Q-B1 | Default and decision agree. |
| Q-B3 reputation design | APPLIED | see 2.0 Q-B3 | 2.0 overrides the default (no design chosen). |
| Q-B4 discovery design | APPLIED | see 2.0 Q-B4 | 2.0 overrides the default (directory model is the main design). |
| Q-C2 booking machine | APPLIED | see 2.0 Q-C2 | Default and decision agree. |
| Q-C3 component diagram | APPLIED | see 2.0 Q-C3 | Default and decision agree. |
| Q-C6 gateway bind | OVERTAKEN | see 2.0 Q-C6 | Default said "say neither". The doc says neither. The 2.0 text asks for "configurable", which the code does not allow. Same finding as 2.0 Q-C6. |
| Q-D4 Isolation edge | APPLIED | see 2.0 Q-D4 | 2.0 overrides the default (keep the edge). |
| Q-F2 server-side listen/accept | APPLIED | see 2.0 Q-F2 | Application Interface: "A server-side `listen` and `accept` interface is not part of the design." |
| Q-F3 transport interface | APPLIED | see 2.0 Q-F3 | Deleted. |
| Q-F4 SDK fallback | APPLIED | see 2.0 Q-F4 | 2.0 overrides the default (Envisioned and backlog, no code change). |
| Q-G1-1 WAL | APPLIED | see 2.0 Q-G1-1 | 2.0 overrides the default (no code change). |
| Q-B2 proof of possession and handshake | APPLIED | see 2.0 Q-B2 | Default and decision agree. |

### C. Section 2.2 plain doc fixes (39 rows)

| Item | Verdict | Doc place | Note |
|---|---|---|---|
| Q-A1 "no server sits between participants" | APPLIED | Executive Summary | Reworded: relay and coordinator carry traffic as a fallback. |
| Q-A2 verticals | APPLIED | Executive Summary, Envisioned note ("Professional Services Guild (home services first)"); System Layers diagram "SynApp 1: Roym" | One name for SynApp 1 in all places. |
| Q-A3 RAM figures | APPLIED | Key Hardware Constraints ("sizing hints, not tested requirements") |  |
| Q-A5 substrate version attribute | APPLIED | Conceptual Entity Model (`SUBSTRATE { string public_key }`) | Attribute gone. |
| Q-A6 entry point for private substrate | APPLIED | Multi-Hop Relay ("A coordinator is the entry point only when an SDK call names it"); Envisioned note | Backlog row `deferred-backlog.md:269`. |
| Q-A7 community governance key | APPLIED | Bootstrap Server & DHT Fallback, Envisioned block (step 4) | Kept inside the bootstrap design. |
| Q-A8 Bootstrap Server design | APPLIED | Bootstrap Server & DHT Fallback (built-today paragraph, then one Envisioned block); "Relay and Registry Configuration" |  |
| Q-B5 "Catalog Matching" link to ideas/ | APPLIED | Discovery & Matching | No `ideas/` string in the doc. |
| Q-B6 MLS | APPLIED | Messaging "Libraries"; Security Messaging bullets; Tech Stack rows | `vodozemac` and ADR-0013 Amendment 1. `openmls` and `libsignal` only appear as "not used". Glossary MLS row removed. |
| Q-B8 Master Anchor allow or deny list | APPLIED | Identity Resolution & Revocation ("The anchor is a **deny list**") |  |
| Q-B9 OS enclave | APPLIED | Identity, Envisioned note under the tier diagram |  |
| Q-B10 merge Layer 3 headings | APPLIED | Layer 3 (one heading, five sub-sections) | Order kept: Identity, Discovery & Matching, Messaging, Trust & Reputation, Payments. |
| Q-C1 Beckn | APPLIED | Layer 4 SynApp 1: Roym | Sentence removed; Glossary row removed (zero `Beckn` hits). |
| Q-C4 Recommendation Algorithm | APPLIED | Layer 4 "Recommendation Algorithm" ("Built today" first, formula in Envisioned block) |  |
| Q-C5 second SynApp heading | APPLIED | "Local Producer-Distributor Mesh" (Envisioned) |  |
| Q-D1 provider-facing observability | APPLIED | Observability "Built today" paragraph; "Provider-Facing Observability" (one Envisioned marker) | Sub-sections under the one marker. "Tiered Observability Stack" starts with "Today there is one level". |
| Q-D2 `enc=ecdh-p256` general option | APPLIED | Security Encryption, box T2 and bullet "Who uses it" | "any transport". No DERP. |
| Q-D3 attestation | APPLIED | Substrate Integrity & Remote Attestation (Envisioned; "JSON-RPC interface") | Also separates the two meanings of "attestation" (overlap O31). |
| Q-D5 TBD table status | APPLIED | Resolved Architecture TBD Items (Status column) | Rows 1-4 Built. 5-17 Envisioned except 6, 8, 10, 11, 14 and 16 (built in part), as the default asks. |
| Q-D6 do not merge [ADV-OBS] | APPLIED | Provider-Facing Observability intro and [ADV-OBS] Envisioned note | Cross-linked both ways. |
| Q-D7 Podman rootless and version | APPLIED | Layer 2 Sandboxes; Tech Stack "Container runtime"; Isolation bullet "Container" | No "4.x". Operator advice. |
| Q-D8 MemoryRecorder histograms | APPLIED | Observability "Instrumentation Layer" ("never trimmed") | Backlog row `deferred-backlog.md:402`. |
| Q-E1 coordinator items, built case | APPLIED | Appendix steps 1-4 Envisioned notes | Built case documented. |
| Q-E4 Hickory DNS | APPLIED | Tech Stack | Zero `Hickory` hits. |
| Q-E7 W3C VC and `ssi` | APPLIED | Tech Stack "Verifiable Credentials ssi" (Envisioned); "Signed records" row | TBD row 6 names `ssi`. |
| Q-F1 Connectivity banner | APPLIED | Connectivity Substrate, opening "Built today" paragraph, plus Envisioned block per unbuilt part | The default says one banner. The status legend needs a marker on every unbuilt block, so each block has one. The opening paragraph names the built parts (Iroh, DHT, preamble). |
| Q-F5 `did:p2p` | APPLIED | Connectivity "Identity Model" | `did:key:h...`; zero `did:p2p` hits. |
| Q-G1-2 WASI shims | APPLIED | [FND-CFG] Envisioned bullet "WASM compatibility shims" |  |
| Q-G1-4 Sharded | APPLIED | [TOP-ADR] Envisioned block + the line "The compiler never emits it today" |  |
| Q-G1-5 route caches | APPLIED | [TOP-ADR] "Caching and Invalidation" ("There is no second, route-level cache") |  |
| Q-G1-6 [PLT-DAT] parts 4-5 | APPLIED | [PLT-DAT] 4 and 5 (Envisioned at top) |  |
| Q-G1-7 mixed reader | APPLIED | Addendum bodies | Plans are in Envisioned notes; formulas and WIT kept. |
| Q-G2-1 one marker per phase | APPLIED | Phases 4-7 | Deviation recorded in change.md (commit 2): markers per item, not per phase, because items mix built and unbuilt text. This follows the status legend. |
| Q-G2-2 Phase 6 against Roym | APPLIED | Phase 6 intro and items 0-7 ("Built today") | Links the Roym spec. |
| Q-G2-3 `export-master` sentence | APPLIED | Layer 2 "Keys"; [LFC-MGT] 2 ("A rebuild needs the master-key backups") |  |
| Q-G2-6 OQ-1 | APPLIED | Open Questions OQ-1 ("Resolved: `pkarr` over BEP 0044") | No "custom". |
| Q-G2-7 Glossary stale rows | APPLIED | Glossary (8 rows) and Envisioned wRPC block | Remaining terms still appear in the body. |
| Q-G2-8 `syneroym-dev-sdk` | APPLIED | [ADV-DEV] Envisioned block |  |
| Q-L2 Layer 2 pilot Q3-Q8 | APPLIED | Layer 2 Storage; Multi-Device Sync; Sandboxes; Backup and Restore | Roym write rules were later moved to Layer 4 (change.md, stage 3 follow-up); Layer 2 points there. |

### D. Section 3 milestone ids, hashes, dated notes (25 rows)

| Item | Verdict | Doc place | Note |
|---|---|---|---|
| Top: Migration Note | APPLIED | Migration Note; link now `#target-designs-addendum` | No hash in the anchor. "The Layer 1 to 4 sections are the canonical definition." Matches O26. |
| Top: Table of Contents "MVP Phase 1 ..." | APPLIED | Table of Contents | Link gone. Every internal anchor link in the doc resolves (script check). |
| Layer 1 P2P Networking: "MVP focuses on ..." | APPLIED | P2P Networking: Iroh note | Now "connectivity works over IP networks". |
| Layer 3 Discovery: "M8 ships", "Additive, later" | APPLIED | Discovery & Matching | `M8` gone. "Additive, later:" stays inside the Envisioned option (a time word, not an id). |
| Layer 3 Payments: "(MVP)", "in Phase 6" | APPLIED | Payments | "MVP" gone. "in Phase 6" stays and is legal now: the Addendum note says phases are targets. |
| Layer 3 Identity: Method A, Method B | APPLIED | Identity | Kept (design names). |
| Appendix Scenario Entities: "future DHT" | APPLIED | Scenario Entities, **R** | Now says the node publishes to the DHT when `enable_bep0044_dht` is on. |
| Appendix step 5: "currently implemented in the frontend" | APPLIED | 5. Data Transfer Characteristics | Rewritten with code paths. |
| Tech Stack: "Payment (MVP)", "Wasmtime (latest stable ...)" | APPLIED | SynApp & Crypto Libraries; Core Infrastructure | "Payment processing ... Envisioned"; "Wasmtime 46.x" matches `Cargo.toml:197`. |
| Security, Resolved TBD, Connectivity: none | APPLIED | Connectivity heading | Heading now spells "Heterogeneous". |
| Addendum heading "Post-DD864A1" | APPLIED | "Target Designs (Addendum)" | One `post-dd864a1` string remains, in a link target to the requirements spec (`#post-dd864a1-target-specifications-addendum`, a real anchor at `system-requirements-spec.md:774`). It is outside this doc. |
| Addendum intro: dated status, M-ids, plan link | APPLIED | Target Designs (Addendum) intro | Replaced by "The phases are targets" note. Second `#` title demoted to `###`. |
| Phase 0 [TOP-DSC]: "Phase 0 Contract", "deferred to Phase 1" | APPLIED | Master Anchor Resolution | Gone. |
| Phase 1 [FND-SEC]: "(M3, ADR-0006)", "M4 (M04A Slice B6)" | APPLIED | [FND-SEC] | ADR-0006 kept. Milestone ids gone. |
| Phase 2 [PLT-DAT] 1 | APPLIED | [PLT-DAT] 1 | `[PLT-DAP-01]` kept by decision. |
| Phase 2 [PLT-DAT] 2 | APPLIED | [PLT-DAT] 2 | No M-ids, slice ids, `D-A2-*`, `F5a`, or links to status.md. |
| Phase 2 [PLT-DAT] 3 | APPLIED | [PLT-DAT] 3 | No "Implementation status (M04A ...)", no Decision Register, no task.md link. ADR-0016 section 6 kept. |
| Phase 2 [PLT-DAT] 4-5 | APPLIED | [PLT-DAT] 4 and 5 | Requirement ids kept. |
| Phase 2 [PLT-ASY] | APPLIED | [PLT-ASY] | ADR-0023 kept. M05B text gone. |
| Phase 2 [PLT-RED] | APPLIED | [PLT-RED] | `M3B` gone. DAP ids kept. |
| Phase 3 [LFC-MGT] 2: "M7" | APPLIED | [LFC-MGT] 2 | Now "replication is not built (see [PLT-RED])". |
| Phase 3 [LFC-VER]: code `TODO(M5)` | NOT APPLICABLE | Outside the doc | Not done. `crates/sandbox_wasm/src/engine/lifecycle.rs:122-124` still reads `TODO(M5) ... deferred to M5 [LFC-VER] ... in M3A`. The backlog row exists (`deferred-backlog.md:521`, line number correct). The row said to fix the code comment too. See Misses 5. |
| Phases 3 to 7 headings | APPLIED | Target Designs (Addendum) intro: "The phases are targets." | Option "add one note" chosen. |
| Open Questions OQ-10: "after Phase 1" | APPLIED | OQ-10 | Now "This follows the core platform." |
| Design ids `[PLT-RED]`, `[TOP-*]`, `[FND-*]`, `[PLT-DAP-nn]` | APPLIED | Phase headings | Kept by decision. |

### E. Section 5 merged gaps (25 rows)

| Item | Verdict | Doc place | Note |
|---|---|---|---|
| Gap 1 node ownership | APPLIED | Layer 2 "Node ownership." paragraph | Has `ControllerAgreement`, `roymctl substrate claim`, `substrate/admin`, `admin_ucan_root` fallback, `grant_resolve_to_node_did` (`supervisor/resolve` only). |
| Gap 2 gateway modes, auth service | APPLIED | Layer 2 "Client Gateway and Auth Service" | Modes `open`, `login`, `fixed`; nonce; delegated-key login; cookie the router verifies; Hub login; ADR-0024. |
| Gap 3 failure and shutdown | APPLIED | Layer 2 "Failure and Shutdown" | One `select!`, any exit stops the node, shutdown waits for the supervisor loop only. |
| Gap 4 messaging crypto | APPLIED | Layer 3 Messaging "Libraries"; Tech Stack crypto rows | `vodozemac`, owner-distributed epoch key, ADR-0013 Amendment 1. |
| Gap 5 upgrade and versioning | APPLIED | Layer 2 "Upgrade and Versioning"; [LFC-VER] Envisioned markers | `init()`/`migrate()`, no snapshot, ALPN `syneroym/0.1`, `ENVELOPE_VERSION`, `IDENTITY_BACKUP_VERSION`, `master_anchor_v1`. |
| Gap 6 registry first, DHT second, freshness, visibility | APPLIED | Connectivity: "Registry first, DHT second", "Record freshness", "Record visibility"; Layer 1 links to them | All facts present (1 h republish, 2 h TTL, 30-day `not_after`, last writer wins, three visibility values, one-shot coordinator registration). The home differs from the proposal (Layer 1); change.md commit 21 records it. |
| Gap 7 two-tier logical discovery | APPLIED | [LFC-MGT] "4. Logical Discovery for Callers Outside the App"; [TOP-ADR] | `AppScope`, `AppDid`, signed topology document, `supervisor.resolve`, ADR-0022. |
| Gap 8 supervisor surface | APPLIED | [LFC-MGT] 1 "Supervisor verbs", "Generations", "Key custody" | 17 verbs, generation stamp, vault, mandatory `export-master`; `roymctl app health/alerts` in Observability. |
| Gap 9 key inventory | APPLIED | Security "Keys: Location, Use, Loss" | Table of 8 key rows plus the anchor duty (24 h, republish 12 h). |
| Gap 10 local-only admission in Roym | APPLIED | Layer 2 Access control item 2 | `-32013`, four `directory` methods, `syneroym:invocation` `caller`. |
| Gap 11 signed records, scopes | APPLIED | Layer 3 Identity: "Signed Records"; Method A scope list; "Capability tokens" |  |
| Gap 12 `enc=ecdh-p256` | APPLIED | Security Encryption, "Optional end-to-end stream layer" | Who uses it, no KDF, `pubkey` two meanings, no delegation with it. Backlog rows exist. |
| Gap 13 Roym safety and data lifecycle | PARTLY | Layer 3 Messaging: "Group chat controls", "Safety rules", "Data lifecycle" | Layer 3 part is complete. The proposed "Layer 2 storage note" is missing: the doc says nothing about what undeploy does to a service's data. See Misses 2. |
| Gap 14 browser fallback path | APPLIED | Layer 1 "Browser Path (WebRTC and WebSocket Tunnel)" | Bootstrap page, service worker, `peer-proxy.js`, blind tunnel. |
| Gap 15 relay and deployment config | PARTLY | Layer 1 "Relay and Registry Configuration"; Layer 2 "Deployment Profiles" | Everything is there except "coordinator mode (mock registry, embedded MQTT)". See Misses 3. |
| Gap 16 limits and budgets | APPLIED | Layer 2 "Limits and Budgets" | Stream cap 8, 30 s, 256 KiB twice, scheduler limits, SDK connect 10 s. |
| Gap 17 MQTT namespace, `call_dedup` | APPLIED | [PLT-DAT] 2 "Topic Namespace"; [PLT-ASY] "Receiver-Side Idempotency" |  |
| Gap 18 isolation and row policy | APPLIED | Security Isolation Guarantees "How a service is confined today"; [FND-IAM] "Column Masks and Caveats", "Stage-4 ABAC After-Step" |  |
| Gap 19 control-plane health, real metrics | APPLIED | Observability "Instrumentation Layer" and "Control-Plane Health and Alerts" | Metric families, 1 s sampler, `[roles.observability]`, alert kinds. The type name `StatusQuery` is not used; the behaviour is described. |
| Gap 20 dual build and saga | PARTLY | [ADV-DEV] "Local Substrate Integration" and "Saga Primitive for Guests"; Layer 2 roles table row `roym` | Content exists in [ADV-DEV], which Part 4 names. The Layer 2 "Sandboxes" paragraph in the proposal is missing. Low importance. See Misses 4. |
| Gap 21 Roym transaction rules | APPLIED | Layer 4 "Cards", "Booking State Machine", "Slot claiming" | Seven cards, two tracks, 30-day window, against interest, slot fence, single-writer `booking-progress`. |
| Gap 22 SynOrg directory | APPLIED | Trust & Reputation "SynOrg standing"; Federation | Credentials, revocation, suspend/lift, moderation, consumer pin. |
| Gap 23 binding epoch compare, `websocket` target | APPLIED | [LFC-MGT] 3 (four answers); Layer 2 Ingress paragraph; [PLT-DAT] 2 HTTP Passthrough |  |
| Gap 24 toolchain and version drift | APPLIED | Developer Toolchain | `wit-bindgen` 0.57 vs 0.55 guests, Playwright, Vitest, `mise` tools all present. The stale `webrtc` comment is in code (`Cargo.toml:110`), not the doc. |
| Gap 25 SDK dial skips WebRtc, no fallback | APPLIED | Connectivity "Node Record" (mechanism table), "Connection Establishment" | Backlog `deferred-backlog.md:265`. |

### F. Gaps report (audit-architecture-gaps.md): ranked table, 20 rows

| Item | Verdict | Doc place | Note |
|---|---|---|---|
| GAP-1 node ownership | APPLIED | Layer 2 Access control, "Node ownership." | As Section 5 gap 1. |
| GAP-2 gateway modes and auth | APPLIED | Layer 2 "Client Gateway and Auth Service" | The old one-row "Login and session tokens" is now a full subsection. |
| GAP-3 failure and shutdown | APPLIED | Layer 2 "Failure and Shutdown" |  |
| GAP-4 upgrade and versioning | APPLIED | [LFC-VER] markers; Layer 2 "Upgrade and Versioning" | Snapshot, rollback and negotiation now carry Envisioned markers. |
| GAP-5 registry first, DHT second | APPLIED | Connectivity "Registry first, DHT second", "Record freshness" | The `"protocols": ["wrpc"]` record example is fixed (only in an Envisioned note). |
| GAP-6 record visibility | APPLIED | Connectivity "Record visibility" | ADR-0018 cited. |
| GAP-7 two-tier discovery | APPLIED | [LFC-MGT] 4 |  |
| GAP-8 supervisor surface | APPLIED | [LFC-MGT] 1 |  |
| GAP-9 key inventory | APPLIED | Security "Keys: Location, Use, Loss" |  |
| GAP-10 local-only admission | APPLIED | Layer 2 Access control item 2 |  |
| GAP-11 signed record envelope | APPLIED | Layer 3 "Signed Records" | The old "deferred" claim is gone. |
| GAP-12 messaging crypto | APPLIED | Layer 3 Messaging; Security; Tech Stack |  |
| GAP-13 conversation history, data lifecycle | PARTLY | Layer 3 Messaging "Data lifecycle" | Layer 2 storage note missing (same as Section 5 gap 13). See Misses 2. |
| GAP-14 browser fallback path | APPLIED | Layer 1 Browser Path |  |
| GAP-15 `enc=ecdh-p256` | APPLIED | Security Encryption |  |
| GAP-16 deployment topology | APPLIED | Layer 2 "Deployment Profiles" | Features, `profile`, Docker image, `SIGUSR1`, ports. |
| GAP-17 limits and budgets | APPLIED | Layer 2 "Limits and Budgets" | Links `performance-and-robustness-spec.md`. |
| GAP-18 MQTT namespace, `call_dedup` | APPLIED | [PLT-DAT] 2; [PLT-ASY] |  |
| GAP-19 control-plane health | APPLIED | Observability "Control-Plane Health and Alerts" |  |
| GAP-20 dual build and saga | PARTLY | [ADV-DEV] | No Layer 2 Sandboxes paragraph (same as Section 5 gap 20). See Misses 4. |

### G. ADRs the gaps report lists as not (fully) reflected (12 rows)

| Item | Verdict | Doc place | Note |
|---|---|---|---|
| ADR-0001 delegation certificate format | APPLIED | Identity, Method A (five fields, Ed25519 over canonical JSON, scope list) | Reflected. ADR not cited. |
| ADR-0002 handshake authorization point | APPLIED | Layer 2 Access control item 1; [FND-IAM] "Caller Identity at the Router" | Check in the router before the sandbox, certificate chain only, E2E handshake separate. |
| ADR-0003 retry policy ownership | APPLIED | [TOP-ROB] "Retry Logic Integration"; [PLT-ASY] Configuration | "One node-wide `retry` policy" with the four fields. |
| ADR-0004 Docker image scope | APPLIED | Layer 2 "Deployment Profiles", Docker image bullet | Cited. One image, `syneroym-substrate` and `roymctl`. |
| ADR-0008 config host function | APPLIED | [FND-CFG] "Versioned Configuration Store" | "Running invocations keep the generation they started with." |
| ADR-0012 crate rename | NOT APPLICABLE | n/a | Naming only. The gaps report says omit. |
| ADR-0013 messaging, Amendment 1 | APPLIED | Messaging; Security; Tech Stack; Consumer App | Cited 6 times. Owner-distributed key replaces MLS. |
| ADR-0018 service record visibility | APPLIED | Connectivity "Record visibility" | Cited. |
| ADR-0019 deploy-time artifact delivery | PARTLY | [FND-CFG] "A volume that carries manifest-supplied files is mounted read-only." | Only the read-only mount is there. The delivery rule is missing. See Misses 1. |
| ADR-0022 two-tier logical discovery | APPLIED | [LFC-MGT] 4; [TOP-ADR] | Cited 3 times. |
| ADR-0024 gateway identity and auth service | APPLIED | Layer 2 "Client Gateway and Auth Service"; Consumer App | Cited 4 times. |
| ADR-0025 conversation owns history | APPLIED | Layer 3 Messaging, first paragraph | Cited. |

## 3. Misses

Ranked by importance. Each fix uses short sentences. Code facts were checked in the worktree.

### Misses 1. ADR-0019: how a deploy call delivers documents (PARTLY)

Why it matters: this is a trust rule. A volume file may not come from a host path. The gaps report listed ADR-0019 as "not reflected" and said it had not read the ADR body. The overlaps report (Section 5) has no gap row for it, so no fix commit looked at it. Only the read-only mount is in the doc today ([FND-CFG], "A volume that carries manifest-supplied files is mounted read-only").

Change: in "SynApp Packaging & API Pipeline", after the paragraph that starts "The substrate converts between JSON and WIT values", add:

> **What a deploy call carries.** A deploy call can carry all that a service needs. So a client can deploy to a substrate that has nothing staged on its disk. The WASM bytes travel in the call (`artifact-source` is `binary` or `url`). So do `custom_config`, the config `schema`, the FDAE policy and the files of a container volume. A document (`schema`, `fdae-policy`) is a `document-source`. It is either `inline` text in the call, or a `path` on the substrate host. In a manifest, a bare path is read by the client and sent inline. `{ remote_path = "..." }` names a file that the substrate host already holds. A container volume file must be `inline`. The Podman engine refuses a host `path` there, because the container could then read any file that the substrate can read. When a volume has files, the engine builds them in a new directory, swaps it in, and mounts the volume read-only ([ADR-0019](decisions/0019-deploy-time-artifact-delivery.md)).

Code checked: `crates/wit_interfaces/wit/control-plane/control-plane.wit:16-23` (`document-source`), `:98` (`schema`), `:103` (`fdae-policy`), `:138` (volume file `content`); `crates/sandbox_podman/src/engine.rs:72-76` (path refused for a volume file), `:85` (`:ro` when files exist), `:93-139` (staging directory); `crates/app_orchestration/src/models/service.rs:185-206` (`DocumentRef`: bare string or `remote_path`); `crates/sdk/src/mapper.rs:72` (client reads a local file).

### Misses 2. Gap 13 / GAP-13: what removal does to a service's data (PARTLY)

The Layer 3 part is done (group controls, safety rules, data lifecycle). The proposed "short Data lifecycle note in Layer 2 storage" is not in the doc. The doc says nothing about what undeploy does with the data.

Change: in "Storage & Write Arbitration", after the "Durable outbox" paragraph, add:

> **Removing a service.** Undeploy removes the component file, the endpoints, the messaging subscriptions, the FDAE policy, the asset blobs, and the owner and certificate records of the service. The code deletes no service database. The `state.db` file, the `dek_store` row of the service and its `async.db` outbox stay on disk.

Check before you write it: I read `undeploy_impl` (`crates/control_plane/src/service/orchestration/lifecycle.rs:249-269`) and its helpers, and `remove_wasm` (`crates/sandbox_wasm/src/engine/lifecycle.rs:180-194`, deletes only `<id>.wasm` and `<id>.quota.json`). A grep for delete or purge of service data in `control_plane`, `data_db` and `sandbox_wasm` found nothing. This is a code reading, not a test. The gaps report also called it unverified. Run one undeploy test, or keep the last sentence as "No code deletes ..." only if the owner accepts a code reading.

### Misses 3. Gap 15: coordinator mode of the router (PARTLY)

Section 5 gap 15 lists "coordinator mode (mock registry, embedded MQTT)". The doc says only "a router handler in coordinator mode, which has no local services".

Change: in "Multi-Hop Relay (Federated Coordinator)", first paragraph, after "which has no local services", add: "In this mode the handler has an empty mock endpoint registry, no sandbox, a freshly generated identity and its own embedded MQTT broker."

Code checked: `crates/router/src/route_handler.rs:440-480` (`new_coordinator`: `EndpointRegistry::new_mock` at `:450`, `app_sandbox_engine: None` at `:454`, `Identity::generate()` at `:447`, `MqttBroker::new` at `:462-464`).

### Misses 4. Gap 20 / GAP-20: dual build in Layer 2 Sandboxes (PARTLY, low)

The content is in [ADV-DEV] ("Local Substrate Integration") and the `roym` feature is in Layer 2. The gaps report asked for one paragraph in Layer 2 "Sandboxes". Part 4 of the overlaps report names only [ADV-DEV], so this may be accepted as done.

Change, if wanted: at the end of the "Sandboxes" list in Layer 2, add: "- **Native build of Roym.** The Roym services build two ways from one source tree. As WASM components they run in Wasmtime. With the `roym` Cargo feature they link into `syneroym-substrate` and run in its process, not in the Wasmtime sandbox. The crates `syneroym-app-host` and `syneroym-app-host-native` give both builds the same traits. See [ADV-DEV](#adv-dev-synapp-developer-tooling--sdks)."

Code checked: `crates/app_host/src/lib.rs:1-5`, `crates/app_host_native` exists, `crates/substrate/Cargo.toml:95` (`roym` feature).

### Misses 5. Section 3: code comment `TODO(M5)` (NOT APPLICABLE, but not done)

The row says "Same fix applies in code". The doc is clean. The code comment still cites a milestone and a plan item, which AGENTS.md forbids in comments. The backlog row is correct (`deferred-backlog.md:521`, line 122).

Change (code, a separate small change): in `crates/sandbox_wasm/src/engine/lifecycle.rs:122-124`, replace the three comment lines with: `// TODO: a snapshot and rollback safety net for migrate() is not built. migrate() may run destructive DDL, and a failed migrate() is not rolled back.` Keep the backlog row and keep its line number right.

### Owner question (not a doc miss): Q-C6 premise

The decision says to write that the gateway bind address "is configurable". It is not. The only gateway setting is the port (`crates/core/src/config/roles.rs:375-400`). The address is fixed in code (`crates/client_gateway/src/gateway.rs:184`). The doc correctly states no address. If the owner wants the bind address to be a setting, that is a code change. The backlog row `deferred-backlog.md:267` already says "Make the code and the spec agree".

## 4. Other problems noticed

1. **"WebRTC relay" in "Minimal Initial Implementation".** The Transports list says "Iroh relay and WebRTC relay". Layer 1 says no TURN relay exists and only STUN is built (Browser Path, Envisioned note). Fix: replace "WebRTC relay" with "WebRTC data channels with STUN, and the WebSocket tunnel".
2. **Repeated sentence in the Layer 2 Podman bullet.** "The substrate calls the host's `podman` command" appears twice, and "Run Podman rootless" follows "prefers rootless" and "does not check". Fix: keep one sentence for the command and one for the advice.
3. **"heartbeat" is undefined in Appendix step 2.1.** "publishes it ... at deploy time and on every heartbeat". Elsewhere the doc says "every hour" (Record freshness), and [TOP-ROB] says the design uses no heartbeat. The code calls the hourly republish a heartbeat (`crates/core/src/dht_registry/types.rs:20`, `HEARTBEAT_INTERVAL_SECS = 3600`). Fix: write "at deploy time and every hour (see Record freshness)".
4. **Two subgraphs both called "SynApp 2" in the Isolation diagram.** `APP2` (Podman) and `APP3` (WASM) are both titled "SynApp 2". This was in the original (`git show main:docs/system-architecture.md`, lines 1160 and 1165). The Q-D4 edge `APP1 <-> APP3` is now labelled cross-app, so the titles matter. Fix: retitle `APP3` "SynApp 3 (WASM sandbox)" or merge the two boxes into one SynApp.
5. **Heading "Layer 3 — Shared Substrate Utilities" no longer matches its content.** Only Identity and Messaging are substrate utilities (the intro says so). A reader of the Table of Contents still sees five "substrate utilities". This is a naming choice for the owner. Renaming would break links from other documents.

