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
- **Simulation section.** The Envisioned marker now sits directly under the heading. "ships" became "gets". The sentence about a "walking skeleton stage" was removed because it named a plan stage.

Found outside Layer 2 and not changed (they need a decision or a later pass):

- The commit hash `dd864a1` is still in the heading "Post-DD864A1 Target Designs (Addendum)" (line 1839) and in `docs/ROADMAP.md`, `docs/STATUS.md`, `docs/system-requirements-spec.md` and `docs/planning/meta-implementation-plan.md`. Other documents link to the heading anchors, so renaming belongs to the structure change.
- The technology stack table says the external API is JSON-RPC over WebSocket and that inter-component calls use wRPC. The toolchain table lists a custom `syneroym` CLI. None of these matches the code (Layer 2 audit: L2-02, L2-03, L2-28).

## Close-out

Before merge of the last step, update the living docs so they describe the
result. Then set `status: done` and fill in `living-docs-touched`. Record
anything postponed in [deferred-backlog.md](../../deferred-backlog.md).
