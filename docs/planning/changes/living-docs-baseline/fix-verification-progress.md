# Stage 2: verification progress

Reader: the coordinator of the architecture fix, and any developer who resumes it.

Stage 2 checks every row of [fix-new-claims.md](fix-new-claims.md) against the code with an independent reader. A batch is `accepted` only when its report is complete, checked and committed. Row ids are `<commit>.<n>`: the n-th row of that fix commit in `fix-new-claims.md`.

| Batch | Fix commits | Rows | Status | Commit | Notes |
| --- | --- | --- | --- | --- | --- |
| S1 | 1, 2, 3, 4, 5 | 62 | accepted | 55111c11 | 62 rows: 55 CONFIRMED, 2 CITE-OFF, 5 PARTLY, 0 WRONG; 3 findings spot-checked in code |
| S2 | 6, 7 | 49 | accepted | 94254767 | 49 rows: 35 CONFIRMED, 13 PARTLY, 1 WRONG (6.24: messaging/stream targets have no caller check; doc says they reject anonymous); 2 findings spot-checked |
| S3 | 8, 9, 10 | 59 | accepted | c5fb4685 | 59 rows: 47 CONFIRMED, 2 CITE-OFF, 9 PARTLY, 1 WRONG (10.28: request_raw is a JSON-RPC call, not a raw byte stream); WRONG finding spot-checked in code |
| S4 | 11, 12 | 63 | accepted | c0bd83a8 | 63 rows: 58 CONFIRMED, 5 PARTLY; browser enc= only on WebSocket tunnel, not WebRTC data channel (11.21, 12.27); 2 findings spot-checked |
| S5 | 13, 14, 15 | 63 | accepted | 58d399ce | 63 rows: 54 CONFIRMED, 8 PARTLY, 1 WRONG (14.28: vault entry name is member-APP_INSTANCE_ID#SERVICE-INDEX); 2 findings spot-checked |
| S6 | 16, 17 | 63 | accepted | 0b6ace90 | 63 rows: 57 CONFIRMED, 1 CITE-OFF, 5 PARTLY; RECORD_TYPES table not used to verify (16.14); outbound node-to-node is Iroh only (16.21); 1 finding spot-checked |
| S7 | 18, 19 | 52 | accepted | 17f5c541 | 52 rows: 46 CONFIRMED, 6 PARTLY; 18.3: consumer never checks revocation (with_revocations only in tests); 19.21: no command revokes a person's delegated key; finding spot-checked |
| S8 | 20 | 50 | accepted | 437f3193 | 50 rows: 45 CONFIRMED, 1 CITE-OFF, 4 PARTLY; all negative claims held; 20.21 gateway does not sign streams with node key as doc says; 20.45 cron accepts 6/7 fields; cron finding spot-checked |
| S9 | 21, 22, 23, 24 | 57 | accepted | 410d2445 | 57 rows: 46 CONFIRMED, 2 CITE-OFF, 9 PARTLY; relay TLS and probe share one bind address (21.7); multi-hop claims 21.12-21.14 overstated; JSON-RPC everywhere (23.1) and relay-only-as-fallback (22.1) too broad; relay finding spot-checked |

## Result

Stage 2 is complete: all 518 rows were checked by an independent reader.

| Verdict | Rows |
| --- | --- |
| CONFIRMED | 443 |
| CITE-OFF (claim true, cite wrong) | 8 |
| PARTLY (doc says more than the code supports) | 64 |
| WRONG | 3 |
| UNVERIFIABLE | 0 |
| **Total** | **518** |

The three WRONG rows: 6.24 (messaging and stream targets have no caller check), 10.28 (`request_raw` is a JSON-RPC call, not a raw byte stream), 14.28 (the vault entry name format). Each report (`fix-verification-S1.md` to `S9.md`) has a "Findings" section with exact replacement wording for every row that is not CONFIRMED, and a section for doc statements that no row covers.

The doc text has not been changed because of these findings yet. That is a separate fix pass.
