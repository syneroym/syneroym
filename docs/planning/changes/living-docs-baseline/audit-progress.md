# Architecture audit progress

Reader: the coordinator of the audit, and any developer who resumes it.

This file tracks which batches of the architecture audit are done. A batch is `accepted` only when its report is complete, checked and committed. A `.partial` file is never trusted.

| Batch | Headings covered | Status | Commit | Notes |
| --- | --- | --- | --- | --- |
| A | Executive Summary, Goals & Constraints, System Layers Overview, Layer 1 | accepted | PENDING | 97 claims; 5 spot checks passed |
| B | Layer 3 — Shared Substrate Utilities (both copies) | not started | | |
| C | Layer 4, Federation Architecture, Consumer Experience Architecture | not started | | |
| D | Observability, Security, Resolved Architecture TBD Items | not started | | |
| E | Appendix: Multi-Hop Relay Walkthrough, Consolidated Technology Stack | not started | | |
| F | Connectivity Substrate In Heteregenous networks | not started | | |
| G1 | Addendum: Phase 0, Phase 1, Phase 2 | not started | | |
| G2 | Addendum: Phase 3 to Phase 7, Open Questions, Glossary | not started | | |
| GAP | Whole doc and the code (gap analysis) | not started | | |
| OVERLAP | All accepted reports (overlap map) | not started | | |

Layer 2 (Substrate Runtime) was audited earlier: see `audit-architecture-layer2.md`.
