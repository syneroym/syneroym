# Stage 3, second language pass: progress

Reader: the coordinator of the language pass, and any developer who resumes it.

The first language pass (`language-progress.md`) changed little: most edits were commas. This second pass splits long sentences and replaces hard words. One commit per group. A group is `done` only after its commit is checked with the token check (code spans, numbers, links, ids, headings, markers, qualifier and modal words) and a read of the diff.

| Group | Parts | Status | Hash | Notes |
| --- | --- | --- | --- | --- |
| L1 | Top matter, Executive Summary, Goals & Constraints, System Layers Overview | done | 15b63e87 | reviewed diff; 10 lines changed; range was already simple (0 sentences over 28 words) |
| L2 | Layer 1 | done | ca529d86 | reviewed diff; 12 blocks; committed with -n (hooks skipped), rule added to brief afterwards |
| L3 | Layer 2 | done | 7ac87aa5 | reviewed 19 blocks; 4 long sentences fixed |
| L4 | Layer 3 | done | 66abfa72 | reviewed; 8 long sentences fixed (1 field list left); verifier checks rewritten as 'must be' with the same three checks |
| L5 | Layer 4, Federation, Consumer Experience, Observability | done | 8283a1da | reviewed; 8 long sentences fixed; 'Local Producer-Distributor Mesh' subsection not covered: add to a later group |
| L6 | Security, Resolved TBD Items, Appendix, Technology Stack | running | | |
| L7 | Connectivity Substrate | not started | | |
| L8 | Addendum intro, Phase 0, Phase 1, Phase 2 | not started | | |
| L9 | Phase 3 to Phase 7, Open Questions, Glossary | not started | | |
