---
status: in-progress
living-docs-touched:
---
# Living docs baseline

Reader: a developer who plans or does the docs round.

## Problem

The architecture doc ([system-architecture.md](../../../system-architecture.md))
has about 2,300 lines. It was written in stages, and the code has changed
since. We do not know which statements are still true. A reader who trusts the
doc can believe that a feature works when it does not.

The requirements, developer guide and traceability matrix may have the same
problem. The doc is also too big to review in one pass, and it has no end user
guide.

## Scope and non-goals

The whole docs round has four steps. This change doc tracks all of them.

1. **Audit.** Check every factual claim in a section against the code. Record
   the result in a report. The first audit is a pilot on one section.
2. **Split.** Move each section into small files, one per capability, each with
   a short index. This follows the plan in [docs/README.md](../../../README.md).
3. **Fix.** Correct the wrong statements. Mark planned work as `planned`. Remove
   milestone and slice IDs from living docs.
4. **User guide.** Write a guide for end users. It says what a person can do and
   how, with no internals.

- In scope now: the pilot audit of the "Layer 2 — Substrate Runtime" section.
  The report is [audit-architecture-layer2.md](audit-architecture-layer2.md).
- Not in scope now: editing the architecture doc, editing any code, and the
  later steps above.

## Acceptance criteria

| ID | Criterion | Test |
| --- | --- | --- |
| AC-1 | Every claim in the pilot section has one verdict. Every MATCHES verdict has a `file:line` citation. | Review of the report |
| AC-2 | The report lists structural problems of the whole file and the questions that need the user. | Review of the report |
| AC-3 | This change doc passes the change-doc check. | `cargo xtask check-change-docs` |

## Design

The audit uses only the code and the tests as proof. Planning docs under
`docs/planning/milestones` are not proof. ADRs help to understand intent.

Each claim gets one verdict: MATCHES, DIVERGES, NOT BUILT, STALE or UNCLEAR.
A claim with no citation counts as UNCLEAR.

Default rule for DIVERGES and NOT BUILT: if the code does it, the code is right
and the doc needs a fix. If the code does not do it and the doc clearly means
future work, the proposed status is `planned`. The user decides only when the
intent is unclear.

## Open questions

See section 4 of [audit-architecture-layer2.md](audit-architecture-layer2.md).

## Tasks

- [x] Audit "Layer 2 — Substrate Runtime" (pilot).
- [ ] Audit the other sections of the architecture doc.
- [ ] Audit the requirements, developer guide and traceability matrix.
- [ ] Split the architecture doc into capability files.
- [x] Fix and mark the wrong statements in "Layer 2 — Substrate Runtime" (pilot).
- [ ] Fix the wrong statements found by the audits in the other sections.
- [ ] Write the end user guide.

## Decisions

Owner decisions from the architecture audit, kept here so they stay after the audit
reports are archived. They win over the defaults in the audit reports.

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
| `[PLT-DAP-nn]` ids in headings (2026-10-07) | Keep them. They are the mapping from requirements to architecture. Code does not cite them; the link is kept in the doc and in PRs. | No change to the ids in headings or bullet names. |
| Requirements spec relay and bootstrap text (O10) | Not changed in the architecture round. The requirements spec is changed in a following round. | The deferred-backlog row stays open. No edit to `system-requirements-spec.md` in this change. |

## Deviations

- The pilot report carries 61 claims for about 140 doc lines, which is more than the
  "one claim per statement" plan expected, because the diagram nodes and the
  table rows each count as a claim.

Layer 2 fix (differences from the audit report and the plan):

- **Q1 against "keep the vision".** The plan says to drop the backup pool and hot standby (Q1) and also to keep forward-looking text. I followed Q1. The mutual backup pool and hot standby (L2-33, L2-34, L2-35) are not in the text any more. Litestream is gone. The one envisioned replication design is `[PLT-RED]`, which already covers S3-compatible backups. Please confirm that dropping the pool idea is right.
- **L2-30 ("signed archive").** Read the code again. The archive is encrypted and authenticated with AES-GCM under the recovery key. It is not signed as a whole. Each service bundle inside has a manifest signed by the person, and restore checks it. The text says this.
- **L2-31 ("portable to any compatible version").** Restore accepts only archive version 1. There is no rule about substrate versions. The text states the archive version rule. After a restore the node has new addresses, so old conversations cannot continue. The text says this too.
- **L2-11, L2-45, L2-47** were settled by the decisions Q7, Q8 and Q4, not by new code reading.
- **Utilities (L2-18, L2-19, L2-20).** Q2 says they are Roym app features. I wrote a plain paragraph ("Not substrate components") and did not use an Envisioned marker. Roym provides discovery, payment records and signed receipts today. Roym has no reputation feature, and the paragraph does not claim one.
- **Order state and reputation rows (L2-40, L2-44).** Per Q3 they are in an Envisioned block. The rest of the table now shows the real Roym rules, including the listing and policy rows.
- **L2-12.** The doc now says encryption is a config switch (`storage.encryption`, on by default).
- **Client gateway edge.** The diagram has no arrow from the gateway. The gateway sends its stream to a target node, which may be another node.
- **Repeated claims.** Besides lines 1003 and 1098-1099, I also fixed rows 1 and 2 of the same index table (lines 1096-1097). They repeat the export and Litestream claims.
- **Line 1003.** No simulation harness or `proptest` exists in the repository. I replaced the one bullet with an Envisioned note. The sentence above it ("The substrate ships a multi-node simulation harness") is outside Layer 2 and still says it ships. That section needs its own audit.
- **Headings.** All Layer 2 headings keep their names, so links and the table of contents still work.
- **Mermaid.** Both diagrams in Layer 2 were checked with the Mermaid parser (version 11) and parse.

Owner decisions after the Layer 2 fix (docs/architecture-redundancy-options):

- **Replication design is open.** `[PLT-RED]` is one proposal and is not frozen. The design freezes when that work starts. This reverses the Q1 default. Litestream is back as a named option, always under an Envisioned marker. The Layer 2 text no longer calls `[PLT-RED]` "the design". The mutual backup pool and hot standby stay out of the text.
- **Rootless Podman is preferred, not required.** The text and `docs/developer-guide.md` say Syneroym prefers rootless Podman and does not check it. No check was added, because nobody asked for one.
- **Litestream outside Layer 2.** The four lines (layers diagram, security diagram, technology stack row, toolchain row) now carry an Envisioned marker. The two table rows were split out of their tables so the marker can sit under the table.
- **Commit hash in the Migration Note.** Removed from the prose of the note. The link still points at the heading, so the hash stays in the anchor until the heading is renamed.
- **Three technology rows.** The owner asked for these fixes after the audit. "External API" now says HTTP/1.1 and framed streams. "Inter-component calls" now says JSON-RPC 2.0 through the Universal Proxy, and wRPC is under an Envisioned marker split out of the table. The `syneroym` CLI row became `roymctl`.
- **Simulation section.** The Envisioned marker now sits directly under the heading. "ships" became "gets". The sentence about a "walking skeleton stage" was removed because it named a plan stage.

Found outside Layer 2 and not changed (they need a decision or a later pass):

- The commit hash `dd864a1` is still in the heading "Post-DD864A1 Target Designs (Addendum)" (line 1839) and in `docs/ROADMAP.md`, `docs/STATUS.md`, `docs/system-requirements-spec.md` and `docs/planning/meta-implementation-plan.md`. Other documents link to the heading anchors, so renaming belongs to the structure change.

Architecture fix, commit 1 (Glossary and Open Questions):

- **One marker for the Open Questions table.** The table cannot hold a blockquote marker per row. One Envisioned marker sits above it, and rows say "Built today" for the parts that exist. OQ-1 is marked resolved inside its row.
- **Beckn row deleted.** The overlaps report does not list it. Its default (Q-C1) removes the only body mention, in Layer 4, so the Glossary row leaves with it. Layer 4 still names Beckn until its own fix commit.
- **wRPC row moved out of the table.** A table row cannot carry the Envisioned marker, so the term now sits under the table, as the Litestream rows did.

Architecture fix, commit 2 (Phase 7 and Phase 6):

- **Markers per item, not per phase.** The default (Q-G2-1) says one marker per phase. Phase 6 items 0, 3, 4, 5 and 7 mix built and unbuilt text, so each has a "Built today" paragraph without a marker and its own Envisioned marker above the design text. Items 1 and 6 have a marker only. Item 2 is built, so it has no marker. Phase 7 has one marker.
- **Phase 6 intro paragraph.** I added one unmarked paragraph under the Phase 6 heading that says Roym is the one SynApp built so far.
- **Aggregator text.** Item 7 follows the owner decision for Q-A4: an aggregator is a SynOrg `directory` service. The federation and query proxying part sits in the Envisioned design text. The `publish_listing` and `search` names became `directory.publish` and `directory.search`, the real method names.
- **DLN.** The doc never defined DLN. Item 3 now expands it as Dynamic Ledger Network, the name used in the requirements spec.
- **Wording kept.** The design text of items 1, 3, 4, 5 and 6 is unchanged, including "central coordinator", "Invoice Card" and "Aggregator". The language pass decides those. The "Adaptive Cards" comparison and the "Forms" widget were removed, because the code has neither.
- **Phase headings.** "Phase 6" and "Phase 7" keep their names. The Part 3 decision on phase headings belongs to commit 9.

Architecture fix, commit 4 (Phase 4):

- **Where visuals live (O52, "Pick one").** Both designs are unbuilt. I kept both and removed the clash in wording: the `[ADV-OBS]` line now says the Metrics Pipeline hosts no dashboards, and points to the provider status page in Observability Architecture as a separate design. Commit 15 decides the final text of that page.
- **Marker split in `[ADV-OBS]` and `[ADV-DEV]`.** Both blocks mix built and unbuilt text, so each has an unmarked "built" part and a separate Envisioned marker. `[ADV-AI]` has one marker under its heading (Q-G2-1).
- **Renamed design name.** The planned background task is now `Metrics Pipeline`, so it does not clash with the built `ObservabilityEngine` (the audit left the new name open).
- **`substrate.db` and `authorization-engine`.** I used the defaults from the audit: "the node's state databases" and "FDAE" with a link to ADR-0017. The messaging section still says `substrate.db`: it belongs to a later commit.

Architecture fix, commit 5 (Phase 3):

- **Facts fixed beyond the commit row.** The audit rows G2-004 (WebRTC path), G2-018 ("how the supervisor itself is stood up"), G2-021 (`roymctl reconcile` is `roymctl app reconcile`), G2-023 (what the supervisor database holds) and G2-032 ("topology_epoch" is the per-dependent binding epoch) were wrong in the text, so I fixed them in the same commit.
- **"No background monitoring" kept, with a limit.** The audit marked it MATCHES. The code has `roymctl app health --watch`, which repeats in the foreground. The text says so.
- **`[LFC-VER]` order.** Steps 2 and 3 are built and steps 1 and 4 are not. To keep the built text free of a marker, the built steps come first and the Envisioned block holds steps 1 and 4, named by position ("before the hook", "after the hook"). The step numbers are gone.
- **`[LFC-VER]` part 2.** One Envisioned marker covers the whole design, including the case-by-case deprecation policy, which has no code to check. A short built paragraph comes first.
- **New subsection.** I added "4. Logical Discovery for Callers Outside the App" to `[LFC-MGT]` for gap 7. The text does not say ADR-0022 is still Proposed.
- **Not done here.** The `TODO(M5)` in `crates/sandbox_wasm/src/engine/lifecycle.rs` stays for commit 24. The `websocket` route target of gap 23 belongs to Layer 2 (commit 20). The "Upgrade and versioning" paragraph for Layer 2 (gap 5) is also left to commit 20: `[LFC-VER]` now has the built facts.
- **Replication wording.** "(Iroh WAL shipping) ... M7" became "replication is not built (see [PLT-RED])". Per the Litestream decision, no design is named as chosen.

Architecture fix, commit 6 (Phase 2):

- **Requirement ids kept.** `[PLT-DAP-01]` to `[PLT-DAP-06]` stay in the headings and the bullet names of `[PLT-DAT]` and `[PLT-RED]`. They are requirement ids of the requirements spec, like `[PLT-RED]`, and one heading link uses them. Part 3 of the overlaps report lists them as a rule break but also says "decide". The owner can still remove them.
- **`substrate.db`.** The code names the file (`crates/data_db/src/sqlite/provider.rs:71`, table at `:140`), so the text names it too, in the messaging subscriptions line and in Database Isolation. The vaguer wording "the node's state databases" stays only where the code names no one file.
- **Lease-based scheduling deleted, not marked.** ADR-0023 section 6 replaced it, so it is not part of the vision. The text now describes the App Supervisor scheduler. The same holds for the "P2P Overlay over QUIC" paragraph: the code has one local broker with no overlay, and the log replication idea is kept in an Envisioned block.
- **Moved text.** `[PLT-DAT]` part 1 has one Envisioned block at its end (logical data services, DuckDB, WAL mode and tuning, aggregation over views, structured data model, peer-to-peer blob replication). The original sentences moved there unchanged, so the built text above has no marker.
- **Removed bullets in the Universal Proxy.** "Instance Routing" is now in the built paragraph (the host resolves a dependency name per call). "JSON-RPC Adapter" is deleted: JSON-RPC 2.0 is the one call surface, not an adapter.
- **Interim security paragraph deleted.** The paragraph "does not currently require a delegation certificate" was wrong. The self-asserted public key caveat is left for Layer 2 "Access control" (commit 20). The `public` flag text stays in the HTTP bridge bullet.
- **`[PLT-RED]` shape.** The built paragraph and the Control Plane vs Data Plane bullet come first. The rest is under one Envisioned marker. "Registry Service" became "App Supervisor" in the node states, fencing and promotion bullets (overlaps O59). The Iroh WAL shipping text is called one proposal, and Litestream stays an option (Q-1). No design is named as chosen.
- **WAL (Q-G1-1).** No code change. The text says the data layer sets no WAL pragma on `state.db`. WAL mode and tuning are in the Envisioned block.
- **Aggregation over views (G1-090).** The WIT file says aggregation targets physical collections only and views are deferred, so the text says that.

Architecture fix, commit 7 (Phase 1):

- **Schema validation of configuration (audit G1-050 was wrong).** The audit says no schema validation exists. The code validates `custom_config` against a manifest `schema` when one is declared (`crates/control_plane/src/service/orchestration/deploy/manifest.rs:36-65`), and the deploy fails on a violation. The text now says this. "Fully resolved" became the real step: flattening to text keys.
- **Optimizations moved, not deleted.** "Lookahead Optimization" (Join Tree Collapse) and "Global Logic Short-Circuiting" are not built, and the audit proposes to delete the second. They are optimization ideas, so I kept both under one Envisioned marker at the end of `[FND-IAM]`. I replaced the "SQL Generation" text with the real emit rule.
- **Q-B2.** A new "Caller Identity at the Router" bullet says what the router checks and that it does not check that the caller holds the temporary key. Proof of possession is Envisioned. The three other handshake points (unsigned client key in the end-to-end handshake, no key derivation step, two meanings of `pubkey`) belong to Security Architecture and are not in this section.
- **Key scope wording.** "Per-SynApp-Instance KEK" is kept. The code derives the key per `service_id`, and ADR-0006 says `service_id` is the app-instance id.
- **Config rule moved.** "Long-running tasks follow the `[PLT-ASY]` restart or compensation rules" now sits in the Envisioned note of `[FND-CFG]`, because the restart rules are Envisioned in `[PLT-ASY]`.
- **Podman secret injection.** One copy only, in the Envisioned note of `[FND-SEC]`. `[FND-CFG]` points to it.
- **Intro sentence of `[FND-SEC]`.** "Hardware-level" became "operating-system-level", because the code uses no hardware feature.

Architecture fix, commit 8 (Phase 0):

- **`[TOP-ROB]` rationale removed.** The text said "we do not build a connection cache because Iroh pools connections". The code does not support the reason: a proxied call and a forwarded stream each open a new QUIC connection, and the WebRTC bootstrap has its own locked cache. The "Discarded Alternative" bullet for the cache is gone. Connection reuse is now one Envisioned item with the choice left open. The heartbeat rationale stays, because the code has no heartbeat.
- **`[TOP-ROB]` first bullet renamed.** "Idiomatic Iroh Connection Pooling" became "Connection Handling". No link points at the old name.
- **`[TOP-ADR]` Sharded split.** The resolver code for sharding is built, but nothing can reach it from a manifest. So the sharded strategies sit in one Envisioned block with one line on what the resolver does. Rendezvous hashing, which keyed `Redundant` calls use, stays outside the block with the four-field formula.
- **`[TOP-ADR]` and `[TOP-REG]` deletions.** The route cache, the connection-failure cache trigger, and the health, eligibility and lease fields were deleted. The code has none of them and no vision text asks for them (ADR-0021 removes the live registry).
- **Rollback in `[TOP-DSC]`.** The text promised a rollback. ADR-0021 §5 decides against rollback of a stateful service, so I did not mark it Envisioned. I wrote that no code rolls back, and that the journal states `ROLLING_BACK` and `ROLLED_BACK` exist and are never written.
- **Layer 2 wording not touched.** The Envisioned note under "One app on several hosts" says "no manifest field selects it". The manifest has a `sharding_strategy` field that nothing reads. The note is still right in meaning. It belongs to the Layer 2 commit.
- **`[TOP-DSC]` provider discovery bullet.** I added a short bullet, "Finding Providers and Listings", that says discovery is what the Roym `directory` service does (decision Q-B4). It links to `[P2P-DSC]`.
- **Resolved Architecture TBD Items heading kept.** Audit row TBD-00 suggests renaming the section to "Design decisions index". I fixed the intro and the column header and kept the heading, because the Table of Contents link and the order of fix commits are not part of this step. Rename it with the Table of Contents fix if wanted.
- **Ad boost cap in one place.** Rows 9 and 17 of that table point to row 15 for the cap value (0.3), so the value is written once in the table (overlap O43).
- **Layer 4 headings.** "Order State Machine" is now "Booking State Machine", because Roym has no order entity (Q-C2). No link pointed at the old heading. The heading "SynApp 1: Business, Professional & Retail Spaces" is now "SynApp 1: Roym", because "Space" is a retired name. The Beckn sentence is gone (Q-C1). I replaced it with a short description of the Roym record chain, as the Layer 4 audit row proposes. I added a "Cards" subsection with the seven card types and who signs each, because the audit lists the card set as a gap (gap 21).
- **Mesh heading.** The "Key differences from SynApp 1" paragraph moved under a new heading "Local Producer-Distributor Mesh" (default of Q-C5). It is marked Envisioned. I did not claim that the Mesh is "built thinner", because no code for it exists.
- **Recommendation Algorithm.** The section starts with what search does today (newest first by `issued_at_secs`, merged one hit from each directory in turn). The formula is Envisioned, as Q-C4 asks.

Architecture fix, commit 19 (Layer 3 merge, Identity):

- **Messaging is not only a Roym feature.** The decision says Discovery, Messaging, Trust & Reputation and Payments are Roym features. The code has a substrate host capability for conversation (`syneroym:conversation`, `crates/conversation`), and the Messaging section already says so. The note under the merged heading says Messaging combines both: the substrate owns history and delivery, Roym decides which messages to accept. Relay discovery inside Discovery & Matching is also a substrate part.
- **Method A step numbers.** Only the certificate check and the revocation check are built, so they are steps 1 and 2. The "Temporary Key signed the request" check and the government-identity check are in one Envisioned block under the list.
- **Diagrams redrawn.** The tier diagram, the resolution diagram, the revocation diagram and the delegation diagram showed an allow list, a temporary-key DHT record and a UCAN issued by a temporary key. They now match the code. The government-identity tier is out of the diagram and only in the Envisioned text.
- **New subsection "Signed Records"** (anchor `#signed-records`) holds the signed record envelope and the `syneroym:signing` boundary. Existing headings keep their names.
- **Master Key Compromise** keeps its design as an Envisioned block, reworded to follow `[FND-IDT]`. The text "Tier 1 is the compromise fallback" is deleted, as Q-B1 says.

Architecture fix, commit 20 (Layer 2 leftovers and new subsections):

- **Gateway bind address (Q-C6).** The decision says to write that the bind address is configurable. The code is not: the gateway binds `0.0.0.0:<port>` and only the port is a setting (`crates/client_gateway/src/gateway.rs:184`). So the doc states no bind address and does not say "configurable". It says the gateway sets no rule about which machine may connect, and that the identity mode and the checks at the target decide what a caller may do. The code and Roym spec difference still belongs in the backlog (commit 24).
- **Replicas and database schema.** The Layer 2 text said each replica has its own database and did not mention that the manifest check refuses `replicas` above 1 for a service with a database schema (`manifest.rs:90-101`). I added that sentence, because `[PLT-RED]` already says it.
- **`websocket` route target.** The Layer 2 Ingress text already named it, so I added nothing for it.
- **`profile` setting.** The audit lists a `profile` setting under deployment profiles. The code only logs it and reads no `profiles` table, so the Deployment Profiles subsection says that.
- **Expiry sweep not listed.** The `select!` has an arm for the certificate expiry sweep, but that loop never returns (`-> !`), so the "any component exit stops the substrate" list leaves it out.
- **Supervisor verbs.** `[LFC-MGT]` already lists the 17 verbs and the `export-master` rule (commit 5). Layer 2 only points to it from the Keys list.
- **Write rule names.** The two Layer 2 rows now name `AlreadyDecided` as the code's internal outcome (a later attempt gets the first result) and `slot-taken` as the wire form of `SlotTaken`, matching Layer 4.

Architecture fix, commit 21 (Layer 1):

- **Third-party relay default.** The audit (gap 15) says an Iroh endpoint with no relay URL uses the n0 preset. The code is narrower. A substrate with no `[parent_coordinator.iroh]` section builds no Iroh endpoint. The preset applies only to a coordinator that has no relay URL (no parent and no relay of its own), to the WebRTC coordinator without a parent relay, and to a URL that does not parse. The SDK client never uses the preset. The text says this.
- **New subsections.** I added "Browser Path (WebRTC and WebSocket Tunnel)" and "Relay and Registry Configuration" to Layer 1 (gaps 14 and 15). Registry-first discovery and freshness (gap 6) stay in the Connectivity Substrate section, and Layer 1 links to them.
- **Original diagrams.** The built diagram under "P2P Networking: Iroh" replaces the first diagram. The bootstrap, home relay and TURN edges of the old diagram are now in the Envisioned diagram under "Relay Node Architecture" and in the Envisioned text under "Bootstrap Server & DHT Fallback". "DERP" became "Iroh relay".
- **Outside this commit.** The Requirements spec still has the relay and bootstrap design (O10), and the backlog still links the old heading anchor `#connectivity-substrate-in-heteregenous-networks` (`docs/planning/deferred-backlog.md:257`). Both are for commit 24. The warning at the top of the doc still says "implemented in the coordinator crates" (commit 23).

Architecture fix, commit 22 (System Layers Overview, Conceptual Entity Model, Goals & Constraints, Executive Summary):

- **"Tier" kept for hardware.** The overlaps report (O23) says to rename two of the three uses of "Tier". The Layer 3 identity use is already gone. Observability uses "Tier 1/2/3" for the same hardware tiers as the Key Hardware Constraints table, so I kept the name there and made the table header say "Hardware tier". The two-step discovery in the Phase 0 text ("Tier 1: app DID", "Tier 2: topology document") is a second meaning that this commit did not touch.
- **Entity Model: Provider, Consumer and Aggregator left the diagram.** Provider and Consumer are Roym transaction roles and an aggregator is a SynOrg `directory` service (Q-A4, Q-2), so none is a substrate entity. They are described in the text under the diagram. A `PERSON` entity (a master DID that accesses a SynApp) replaces the `CONSUMER accesses` edge. The `PROVIDER owns-or-uses` edge is dropped: the code shows no ownership link between a person and an app instance.
- **Layers diagram: Layer 2 and Layer 1 boxes.** "Key Management" became "Key Stores (KEK, DEK, vault)" because Layer 2 says there is no single key manager. I added the Connection Router (Layer 2) and the Browser Path (Layer 1). "Bootstrap Server" left the diagram and is named in the Envisioned note.
- **Vertical names.** The Executive Summary uses "Professional Services Guild (home services first)" and "Local Producer-Distributor Mesh", the names in `docs/TERMINOLOGY.md` and in Layer 4, in place of "Home Services Guild" and "Food & Small Retailer Mesh".

Architecture fix, commit 24 (outside the architecture doc):

- **Requirements spec relay and bootstrap (O10): not changed.** The overlaps report only says "fix both together" and "record it as a follow-up". It does not say which words to change. The spec's Relay, Bootstrap and Bootstrap Server text (`docs/system-requirements-spec.md` lines 171, 177, 192, 589-603, 632-648, and the HOME_RELAY entity diagram) describes a design that is not built. Open question for the owner: should the spec mark that text Envisioned, or should it stay as the target requirement? I added a backlog row (section 7) and made no spec edit.
- **TERMINOLOGY.md "Controller" entry (O64): kept.** The entry is correct. A Controller is the owning principal, and the architecture doc now uses the word the same way (node ownership). The part that was wrong was the App Supervisor entry: it said the supervisor has no service-facing directory interface. It has a `resolve` verb. I fixed that entry only.
- **Backlog rows that already existed.** Q-D8 (`MemoryRecorder`), Q-B2 (proof of possession, handshake), Q-F4 (SDK mechanism fallback) and Q-C6 (gateway bind address) already had rows. I added none for them. The `TODO(M5)` marker already had a row in "Open in-code markers"; I only corrected its line number (116 to 122).
- **Links.** The only broken link was the old `heteregenous` anchor in the backlog (fixed). The links to `#layer-3--shared-substrate-utilities` and `system-requirements-spec.md#post-dd864a1-target-specifications-addendum` resolve (the second is an explicit anchor in the spec).

Stage 2 fix, S1:

- **Supervisor rebuild sweep.** The doc said a rebuild uses "a sweep of the target substrates". No such code exists: `adopt` reads only the held generation. The text now describes the manual rebuild (`submit`, then `adopt`). The sweep is under an Envisioned marker and has a backlog row.
- **`import-master` order.** Member keys are minted at `submit`, so they must be imported before the first `submit`. Only the app instance key is minted at `adopt`. The text and the Keys table row say so.
- **Federation of directories.** The doc said a SynOrg, directory or aggregator chooses which directories it queries. Only the client half queries. The sentence now says "client", the Envisioned note says directories and aggregators do not query others, and the backlog has a row.
- **Same payment sentence twice.** It also stood in the Roym integrated experience text, so I fixed both places.

Stage 2 fix, S2:

- **Outbox budget role.** The report says the outbox attempt budget comes from the supervisor role. The guest outbox reads `roles.app_sandbox` (`crates/router/src/route_handler.rs:254`). The supervisor role feeds only the supervisor's own outbox. The doc names `roles.app_sandbox`.
- **Cron wording.** The proposal added "once per reconcile pass". I left it out because it adds no fact about the field count. The doc says five fields, plus the leading seconds and trailing year that the parser accepts. `docs/developer-guide.md` still says "standard five-field"; it is correct for the standard form, so I did not change it.
- **Config-schema marker.** The `replicas` refusal tests the config `schema`, not a database schema. The doc says so. The residual case (data layer use without a `schema`) already has a backlog row. `docs/developer-guide.md` calls `schema` the marker of structured data; I did not edit that guide here.
- **Identical redeploy.** The no-op needs a full deploy of the service by the running process (`full_deploy_completed`), so the first redeploy after a restart is not a no-op. The doc says so.
- **Backlog.** One new row: the declared index type is accepted and never used.

Stage 2 fix, S3:

- **Discovery choice (Q-B4).** The decision says clients, SynOrgs, directories and aggregators each choose what they query. The code has only the client half of the `directory` service, which a node keeps for itself (at most 8 sources). The doc now says a node keeps its own list of directories. A SynOrg or an aggregator choosing sources has no code, and the federation text already marks that Envisioned.
- **Wire protocol wording.** "JSON-RPC is the wire protocol everywhere" was also in the Implementation Note at the top and in the Glossary wRPC text. HTTP routes, raw streams and TCP copies are not JSON-RPC, so all three places now say "the only RPC wire protocol".
- **Relay and DHT-only use of the SDK.** The SDK builds its Iroh endpoint with no address lookup and uses a relay only from the record's `relay_url`. The doc says what the code does. One backlog row records the gap.
- **Reactive eviction.** The doc named a general eviction mechanism. Only the WebRTC bootstrap has a connection cache, so the doc now describes that cache and the proxy retry rule.

Stage 2 fix, S4:

- **Cp record expiry.** The registry deletes an entry after 2 hours without a refresh, and `Cp` registers only once with no `ttl`. The doc says so and states what the code does. A fix (re-register on a timer) is a code change, so it is in the backlog and not built.
- **`wasm-tools`.** No task uses it. The doc says it is installed and available by hand. It is not described as "optional" any more.
- **Browser handshake path.** The same words ("the browser asks for it") were also in the Multi-Hop Relay summary and in Encryption at Every Layer. All three places now name the WebSocket tunnel path.

Stage 2 fix, S5:

- **X3DH wording, all places.** `vodozemac` implements Olm (3DH plus Double Ratchet). The code comment in `crates/conversation/src/crypto.rs` says X3DH, but the library does not. I fixed six places in `docs/system-architecture.md`: the Layer 3 Messaging diagram box M1, its key agreement step (the node id `X3DH` became `KA`), its Libraries line, the Security diagram box M1, the Security 1-to-1 chat bullet, and the Technology Stack row. The Glossary and `docs/TERMINOLOGY.md` had no X3DH text. ADR-0013 was not edited.
- **14.16 (who sets `enc=ecdh-p256`).** Already fixed by the stage 2 fix in S4 (the text names the WebSocket tunnel path). No new edit.
- **Revoking a person's key (14.24).** The doc now says what the code does: `roymctl` has no command that adds a person's key to `revoked_keys`. Backlog row added. The built behavior (router rejects a listed key) stays.
- **DHT fallback and anchor age (14.31).** The doc says only the registry path checks the 24-hour age. Backlog row added.
- **Not-covered item 6 and 7.** `crates/substrate/config.sample.toml`, `config.dev.toml` and `AGENTS.md` say "Prometheus" for the metrics endpoint. They are outside the files this pass may edit. The architecture doc is correct (JSON snapshot, no Prometheus export). Someone should fix those three files.
- **Not-covered item 3 (zero key with encryption off).** The doc states the fact. I added no backlog row, because `storage.encryption = false` is an explicit option that is on by default, and a related row already exists for this setting.

## Close-out

Before merge of the last step, update the living docs so they describe the
result. Then set `status: done` and fill in `living-docs-touched`. Record
anything postponed in [deferred-backlog.md](../../deferred-backlog.md).
