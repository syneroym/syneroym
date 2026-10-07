# Architecture fix progress

Reader: the coordinator of the architecture fix, and any developer who resumes it.

One row per fix commit. The order and scope come from section 4 of [audit-architecture-overlaps.md](audit-architecture-overlaps.md). A commit is `done` only after its check and its record commit.

| Commit | Scope | Status | Hash | Notes |
| --- | --- | --- | --- | --- |
| 1 | Glossary; Open Questions & Recommendations | done | 45b8967a | 9 new claims; 3 deviations; Beckn row deleted but Layer 4 still cites it until commit 17 |
| 2 | Phase 7; Phase 6 | done | ceda65ff | 15 new claims; 6 deviations (markers per item) |
| 3 | Phase 5 | done | 1bd7c14f | 8 new claims; no deviations; Layer 3 reputation wording to match in commit 18 |
| 4 | Phase 4 | done | 61297192 | 12 new claims; 4 deviations; Messaging section still has substrate.db and M3B/M7 (commit 6) |
| 5 | Phase 3 | done | 9409143d | 19 new claims; 7 deviations (also fixed G2-004,018,021,023,032); TODO(M5) left for commit 24 |
| 6 | Phase 2 | done | 13dc269b, 79172637 | 31 new claims; 9 deviations; follow-up restored substrate.db name; [ADV-OBS] line still says "state databases" (commit 4 wording); owner call: drop [PLT-DAP-nn] ids? |
| 7 | Phase 1 | done | 3dc2706a | 18 new claims; 7 deviations (audit G1-050 was wrong: custom_config schema is validated) |
| 8 | Phase 0 | done | 7bfd95f4 | 17 new claims; 7 deviations; Layer 2 "no manifest field selects it" (Sharded) is slightly loose, for commit 20 |
| 9 | Addendum heading and intro | done | ee81eaed | 0 new claims; spec link anchor post-dd864a1-... kept (valid explicit anchor in the spec) |
| 10 | Connectivity Substrate | done | a9623271 | 42 new claims; 0 deviations; code differs from audit CN-40 (two lookups) and CN-18 (addresses pruned) |
| 11 | Consolidated Technology Stack | done | c2c04fbd | 31 new claims; 0 deviations; stale webrtc comment at Cargo.toml:110 is code (commit 24 note); libsignal/openmls remain in Layer 3 Messaging and Security diagram (commits 14, 18) |
| 12 | Appendix: Multi-Hop Relay Walkthrough | done | f940e5e5 | 32 new claims; 0 deviations; Layer 1 Multi-Hop wording still for commit 21 |
| 13 | Resolved Architecture TBD Items | done | e627ac9a | 11 new claims; 2 deviations; table rows describe target wording that Layer 3 (commit 18) and Layer 1 (commit 21) must match; heading rename left to commit 23 |
| 14 | Security Architecture | done | 0863cf74 | 32 new claims; 0 deviations; handshake backlog row already exists (added in the audit PR), commit 24 must not duplicate it |
| 15 | Observability Architecture | running | | |
| 16 | Consumer Experience; Federation Architecture | not started | | |
| 17 | Layer 4 | not started | | |
| 18 | Layer 3 (second heading) | not started | | |
| 19 | Layer 3 (first heading), merge headings | not started | | |
| 20 | Layer 2 leftovers | not started | | |
| 21 | Layer 1 | not started | | |
| 22 | System Layers Overview, Goals & Constraints, Executive Summary | not started | | |
| 23 | Top matter and Table of Contents | not started | | |
| 24 | Outside the doc (TERMINOLOGY, requirements spec, backlog) | not started | | |
