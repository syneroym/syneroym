# Stage 2 fix pass: progress

Reader: the coordinator of the architecture fix, and any developer who resumes it.

This pass applies the findings of the stage 2 verification reports (`fix-verification-S1.md` to `S9.md`) to `docs/system-architecture.md`. One fix commit per report. A batch is `done` only after its commit is checked and recorded. New statements are appended to [fix-new-claims.md](fix-new-claims.md) with the batch label (`S1` to `S9`) in the first column, so a last independent check can find them.

| Batch | Covers (doc areas) | Status | Hash | Notes |
| --- | --- | --- | --- | --- |
| S1 | Glossary, Open Questions, Phase 7 to Phase 3 | done | ef695524 | 15 new S1 rows; 2 backlog rows (rebuild sweep, directory federation); cites in new rows may be a few lines off (recheck) |
| S2 | Phase 2, Phase 1 | done | 5ae482f3 | 21 new S2 rows; 1 backlog row (index type unused); verifier report had one wrong fact (outbox budget comes from roles.app_sandbox); developer-guide.md lines 1149-1153 noted, not changed |
| S3 | Phase 0, Addendum intro, Connectivity Substrate | done | b242930d | 15 new S3 rows; 1 backlog row (SDK relay URL/registry URL limits); 2 CITE-OFF cells corrected |
| S4 | Technology Stack, Multi-Hop Relay appendix | done | a19345df | 12 new S4 rows; 1 backlog row (Cp record expires ~2h after start) |
| S5 | Resolved TBD items, Security, Observability | done | 717eb6ca | 12 new S5 rows; 2 backlog rows (revoking a person's key; anchor age check on DHT path); X3DH fixed in all 6 places (so S7 need not repeat); Prometheus label in config.sample.toml, config.dev.toml, AGENTS.md left (outside scope) |
| S6 | Federation, Consumer Experience, Layer 4 | running | | |
| S7 | Layer 3 (discovery, messaging, trust, payments, identity) | not started | | |
| S8 | Layer 2 | not started | | |
| S9 | Layer 1, overview and entity model, top matter, TERMINOLOGY.md | not started | | |
| RECHECK | New rows labelled S1 to S9 | not started | | |
