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

## Close-out

Before merge of the last step, update the living docs so they describe the
result. Then set `status: done` and fill in `living-docs-touched`. Record
anything postponed in [deferred-backlog.md](../../deferred-backlog.md).
