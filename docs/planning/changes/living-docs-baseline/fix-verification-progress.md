# Stage 2: verification progress

Reader: the coordinator of the architecture fix, and any developer who resumes it.

Stage 2 checks every row of [fix-new-claims.md](fix-new-claims.md) against the code with an independent reader. A batch is `accepted` only when its report is complete, checked and committed. Row ids are `<commit>.<n>`: the n-th row of that fix commit in `fix-new-claims.md`.

| Batch | Fix commits | Rows | Status | Commit | Notes |
| --- | --- | --- | --- | --- | --- |
| S1 | 1, 2, 3, 4, 5 | 62 | accepted | 55111c11 | 62 rows: 55 CONFIRMED, 2 CITE-OFF, 5 PARTLY, 0 WRONG; 3 findings spot-checked in code |
| S2 | 6, 7 | 49 | running | | |
| S3 | 8, 9, 10 | 59 | not started | | |
| S4 | 11, 12 | 63 | not started | | |
| S5 | 13, 14, 15 | 63 | not started | | |
| S6 | 16, 17 | 63 | not started | | |
| S7 | 18, 19 | 52 | not started | | |
| S8 | 20 | 50 | not started | | |
| S9 | 21, 22, 23, 24 | 57 | not started | | |
