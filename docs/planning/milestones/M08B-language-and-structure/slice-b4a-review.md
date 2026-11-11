# M08B slice B4a — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-b4a-implementation-plan.md](slice-b4a-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-11

Checked on `feat/m08b-b4a-full-text` at `a2ccadf2`, in the main checkout
(not a worktree: `.git` is a directory). `git pull` could not run: the
sandbox blocks `github.com` ("CONNECT tunnel failed, 403"). So I did not see
newer commits on `origin`. The local branch holds `main` at `ca9002f3` plus
the plan commit, which is the base the plan names.

**Who reviewed.** This session runs on Claude Sonnet 5.5. The `status.md` row
says the plan was written by Claude Opus 5.5. They are different models, as
the prompt says.

**Change docs.** The prompt lists one change doc,
[roym-multilingual](../../changes/roym-multilingual/change.md). `task.md`
names no other change doc for B4a. I agree: ML-20 and ML-29 are both in
roym-multilingual, so every B4a decision and deviation belongs there (plan
section 3 says the same).

**Summary:** 0 blocker, 4 major, 10 minor, 14 items. All 14 Fixed (e0aab8b8), with the owner's answers to Q1 to Q7 (2026-11-11).

Limits of this review: I read the plan, the docs and the code, and I ran
experiments outside the repository with Python's `sqlite3` module (SQLite
3.50.4, the same version number as the bundled SQLCipher, but not SQLCipher
and not the `rusqlite` build). The experiments are named in each finding.
I did not run any plan step, `cargo` or `mise run verify`, and I changed no
code.

### What was checked and found correct

- **File and line links.** Spot-checked and right: `schema.rs:17`, `:28`,
  `:43` to `:51`, `:52` to `:62`; `query.rs:105` to `:114`, `:229`, `:239` to
  `:241`; `mutation.rs:161`, `:183` to `:188`, `:205`, `:263`, `:470`;
  `sieve.rs:110`; `aggregate.rs:64`; `filter.rs:30`, `:86` to `:109`, `:286`
  to `:299`; `service_store.rs:124`, `:127`, `:155` to `:160`;
  `capabilities_store.rs:104` to `:115`, `:121` to `:133`; `host.rs:94`;
  `convert/data_layer.rs:17`, `:115`; `roym_directory/src/app.rs:107`, `:113`;
  `backup.rs:56`; `data-layer.wit:15` to `:18`, `:115`; backlog `:62`, `:565`,
  `:591`; ADR-0007 `:155` to `:157`, `:167`; requirements `:1045`, `:1046`;
  architecture `:568`, `:2761`; traceability `:34`; `TERMINOLOGY.md:72`;
  `Cargo.toml:127`, `:160`; fixture `app.rs:47`, `:319`; `abac_integration.rs:753`.
  Two small misses are in P14.
- **Counts.** `git grep -c 'CollectionSchema {' -- '*.rs'` gives 45 literals,
  as the plan says (in 30 files, not 32; P14). The oversized-file caps in
  `xtask/oversized-test-files.txt` match section 7.5: `tests.rs` 899,
  `tests_crud.rs` 819, `tests_fdae.rs` 1791, `abac_integration.rs` 811,
  parity `helpers.rs` 842.
- **Callers of `compile_filter`.** Four, as in section 2.2 point 1
  (`git grep` shows no other). The HTTP bridge
  (`router/.../http/parse.rs:26`) builds only string equality filters, so it
  cannot send `$search`.
- **Who writes collection rows.** Only `do_put`, `do_patch`, `do_delete`,
  `do_delete_many`, `do_batch_mutate`, `do_create`. No `INSERT OR REPLACE` on
  a collection table (that would change the `rowid`). `query-raw` is read
  only and `execute-ddl` is admin only (see P12).
- **Reader/writer split.** `fields()` on the reader pool and `declare` on the
  writer is a sound split. `data_db` sets no `trusted_schema`, defensive mode,
  `max_page_count` or `VACUUM`, so nothing blocks FTS5 virtual tables.
- **FTS5 behaviour (my experiments, SQLite 3.50.4).** The
  `unicode61 remove_diacritics 0 categories 'L* N* Co M*'` table with
  `prefix='1 2'` works. `f_a : "ek"*` finds `ek` and `ekdam`; `"नल"*` finds
  `नल वाला`; `"वा"*` finds it too; `"वा"*` and `"வா"*` keep Indic marks in one
  token (`"ल"*` finds nothing, as the plan says). The trigram table with
  `case_sensitive 1` finds `"umb"` and `"plumber's"` in `plumber's shop` and
  finds nothing for a 2-character phrase. `pragma_table_info('<fts>')` lists
  the declared columns and gives no rows for a missing table. A column
  filter `f_a : "abc"` works. This supports D-B4a-3, D-B4a-5 and Step 2.
- **D-B4a-9 is right and needed.** With a cursor and `ORDER BY id`, SQLite
  walks the `id` index: `SEARCH c USING COVERING INDEX sqlite_autoindex_c_1
  (id>?)`. With `+id > ?` and `ORDER BY +id` it uses `SEARCH c USING INTEGER
  PRIMARY KEY (rowid=?)` plus a temp B-tree for the order. I got the same
  plan with and without `ANALYZE`, for a long term, for two terms joined by
  `AND`, and for a `DELETE`. But see P1 for the shapes that still scan.
- **Table naming (D-B4a-4).** `_text_<c>_tri` and `_text_<c>_word` cannot
  equal an FTS5 shadow-table name of another collection (`…_data`, `_idx`,
  `_content`, `_docsize`, `_config`). `_text_index` is refused by the same
  prefix rule. The `LIKE '\_text\_%' ESCAPE` filter also removes the shadow
  tables from `list_collections`; its only caller is the deploy-time
  `strict:` warning (`deploy/commit.rs:222`).
- **Idempotence of the key.** I checked `NFC(lower(NFC(lower(c))))` against
  `NFC(lower(c))` for every Unicode scalar value with Python's tables: 0
  differences. The claim in D-B4a-2 holds (Rust's tables may differ a little
  by version, so the plan's fixed-input tests stay useful).
- **Transaction rules.** `do_batch_mutate` and `do_create` already hold a
  transaction, so a rolled-back batch rolls back the index rows if `sync_row`
  runs inside `do_put`/`do_patch`/`do_delete` (D-B4a-6). The ungated
  `create-collection` stays ungated in the WASM host, as plan point 5 says
  (but see P3 for the native route).
- **Both builds.** The WASM host passes the generated type through; the
  native shim converts it in `collection_schema_in`. `roym_*` crates are
  linked through symlinked WIT copies. The fixture and parity layout in
  D-B4a-13 matches the existing `create-fence` test.
- **Size and layout rules.** New files follow the sibling-file rule
  (`text_index.rs` + `text_index/tests.rs`, `search_key.rs` +
  `search_key/tests.rs`). The estimates in 7.5 are fair. The plan does not
  cite the rules in comments.
- **Backlog, exit criteria, "How this is shown to an end user".** Present and
  honest: no user-visible change, three developer bullets, a "Demo: none"
  line. The backlog rows in section 8 match the decisions. Only the ones
  these findings add are missing (P1, P12).
- **Both change-doc targets.** Every criterion (ML-20, ML-29) and every
  decision and deviation goes to roym-multilingual. `living-docs-touched` and
  `demo:` in its front matter already cover the files the plan names.

### Views on the owner's questions

(For the owner to decide. I do not answer for the owner.)

- **Q1 (how a collection declares text fields).** I would take (a). One call,
  one place, and the compiler finds all 45 builders. The cost is larger than
  the plan says: three vendored WIT copies are real files, not symlinks, so
  the field must reach them too (P2).
- **Q2 (what a `text-fields` list means when an index exists).** I would take
  (a). It matches how `indexes` work and tolerates the `ensure_coll(.., &[])`
  call sites. Two changes make it safe: the list must be compared as a set,
  not by order (P10), and the native route must not accept the list (P3).
- **Q3 (how a guest writes a text search).** I would take (a), the field
  operator. But the plan's argument for it ("works inside `$and`, `$or` and
  `$not`") does not hold: inside `$or` and `$not` the query scans the whole
  collection (P1). So take (a) together with a rule that `$search` is allowed
  only as a conjunct, and say so in the answer. The owner should know that
  the price is no two-field search in one call; a guest indexes one combined
  field instead.
- **Q4 (which rows match when a query has several words).** I would take (a),
  every word must match. It matches how people use a search box and it is what
  B4b's joined-word design needs. It works only if the terms are cleaned
  first (P4).
- **Q5 (a non-string value in a text field).** I would take (a): refuse on
  write, skip on rebuild. Add one more rule so an old row does not block
  unrelated patches (P9).
- **Q6 (how an index row points at its record).** I would take (a). It needs
  no extra table, the chat store already does it, and I found no code that
  changes a `rowid` or runs `VACUUM`. The backlog row is the right place for
  the `VACUUM` caveat. `execute-ddl` can break the link too (P12); it is
  admin only, so a backlog line is enough.
- **Q7 (how ADR-0007 is updated).** I would take (a). ADR-0007 is where the
  operator list lives, and one dated entry under `## Amendments` is the
  precedent (M04A B4). A new ADR is worth it only when ranking or more
  tokenizers arrive.

### Findings

#### P1 — `$search` inside `$or` or `$not` scans the whole collection, so the "no scan" promise is false (major)

- **Where:** Q3 (a) ("works inside `$and`, `$or` and `$not`" and "a search over
  two fields is an `$or`"), W3, D-B4a-8, D-B4a-9, section 7.2 ("Search":
  "`$search` with `$not`", "an `$or` of the two does"; "Plan"), exit criterion
  5, section 3 (ML-29 row: "No `$search` query scans the collection table").
- **Problem:** D-B4a-9 fixes the cursor case with `+id`. But the planner uses
  the `rowid` list from FTS5 only when the `rowid IN (…)` term is a
  top-level `AND` term. I compiled the planned SQL on SQLite 3.50.4 (tables
  as in D-B4a-3, 200 rows, after `ANALYZE`):
  - `WHERE T OR W ORDER BY +id` (two `$search` terms in an `$or`, which is
    the plan's own way to search two fields): `SCAN c USING COVERING INDEX
    sqlite_autoindex_c_1`.
  - `WHERE NOT (T) ORDER BY +id`: `SCAN c USING COVERING INDEX …`.
  - `WHERE (T OR json_extract(payload,?)=?)`: `SCAN c`.
  - `WHERE T AND +id > ?`, `T AND W AND json_extract(..)=? AND +id > ?`: no
    scan.

  The planned plan test covers only four shapes, all of the good kind. So the
  test would pass while the feature scans in the cases the plan advertises.
  ML-29 says "never runs a full table scan" for a query of short words, and
  exit criterion 5 says "No `$search` query plan scans the collection table".
- **Failure case:** A guest searches two fields with
  `{"$or": [{"title": {"$search": "ek"}}, {"summary": {"$search": "ek"}}]}`.
  Every call reads every row of the collection and tests `rowid IN` on each.
  On a large collection this is the scan ML-29 forbids, and it is not visible
  in any test.
- **Fix:** Pick one and write it into Q3, D-B4a-8, D-B4a-9 and the tests:
  1. (Recommended for B4a.) Allow `$search` only where it is a conjunct: at
     the top level or inside `$and`. Pass a flag down the five private
     functions of `filter.rs`; `$or` and `$not` set it false, and a `$search`
     met with the flag false is `schema-violation("$search cannot be used
     inside $or or $not")`. A guest that needs two fields indexes one combined
     field (B4b's directory does that anyway) or sends two queries. Add a
     backlog row (section 8) for a multi-field form.
  2. Or keep `$or`, but compile an `$or` whose branches are all `$search` to
     one `rowid IN (SELECT … UNION SELECT …)`, and refuse the mixed case. More
     code and more tests.

  Whatever is chosen: replace the two planned tests ("with `$not`", "an `$or`
  of the two") with the tests the rule needs; add plan-test shapes for an
  `$and` of two searches and for a search next to a plain equality and a
  sieve clause; run the plan test both with and without `ANALYZE` (production
  databases have no statistics); and change the wording of exit criterion 5
  and the ML-29 row to "when `$search` is an `AND` term".
- *Status:* Fixed (e0aab8b8). Now in: Q3 owner answer; section 1; ML-29 row in section 3; W3 in section 4; D-B4a-1, D-B4a-8, D-B4a-9; Step 6 items 2 and 5; section 7.2 (Filter, Search, Plan); section 8 (new backlog row "Multi-field text search"); exit criterion 5; section 11.

#### P2 — Three vendored copies of the data-layer WIT are real files and already out of date; the plan edits only the canonical file (major)

- **Where:** Step 4 (items 2, 3 and 5), section 2.2 point 4, W1;
  `test-components/saga-test/wit/deps/data-layer/data-layer.wit`,
  `test-components/miniapp-demo1-wasm/wit/deps/data-layer/data-layer.wit`,
  `test-components/websocket-guest-test/wit/deps/data-layer/data-layer.wit`.
- **Problem:** 14 other `data-layer.wit` copies (six `roym_*` crates, the host WIT
  deps and seven test components) are
  symlinks to `crates/wit_interfaces/wit/data-layer/data-layer.wit` (I read
  the link target and ran `diff`: identical). These three are regular files
  (`ls -l`), and `diff` shows they already lack the `create` function that
  `main` has (14 lines). They are built by `mise run build:test-components`
  (`mise.toml:45`). Step 4.2 edits only the canonical file. Step 4.3 then adds
  `text_fields: vec![]` to the sources of `saga-test` and `miniapp-demo1-wasm`
  (the plan lists both). Their bindings come from the old copy, which has no
  `text-fields`, so those two components do not compile. The plan finds this
  only at Step 4.5 and does not say what to do. `websocket-guest-test` does
  not import the data layer (its world has no such import), so its copy is
  dead, but it is the same trap.
- **Failure case:** The implementer follows Step 4: `mise run
  build:test-components` stops with "no field `text_fields`" in `saga-test`
  and `miniapp-demo1-wasm`. They may "fix" it by dropping the field from
  those two literals, and then the component, built against the old record
  type, fails to instantiate against the new host (a component's import of a
  shared record type must match).
- **Fix:** In Step 4.2, replace the three files with symlinks, the same way
  as the other 14 (the link text is
  `../../../../../crates/wit_interfaces/wit/data-layer/data-layer.wit`). Name
  them in section 2.2 point 4. In Step 4.5 add the tests that load these
  components (`saga`, `miniapp-demo1-wasm` and the websocket test in
  `crates/sandbox_wasm/tests/`). Add a one-line check to the exit criteria
  that `git ls-files -s` shows mode 120000 for every
  `wit/deps/data-layer/data-layer.wit`.
- *Status:* Fixed (e0aab8b8). Now in: Q1 owner answer; section 2.2 point 4; Step 4 items 2 and 5; section 7.2 (Step 4.5 component tests); exit criterion 11.

#### P3 — The native `create-collection` route would let a caller trigger a full index rebuild with one small request (major)

- **Where:** W7, section 2.2 point 14, Q2 (a), D-B4a-6 (`declare`), section
  2.2 point 5; `crates/control_plane/src/synsvc_native/data.rs:229` to `:265`
  (`data_create_collection`), `:215` (`require_data_layer_admin`).
- **Problem:** `data_create_collection` has no capability check of its own,
  while `drop-collection` and `execute-ddl` call `require_data_layer_admin`.
  The reason for leaving creation open (WASM host, `capabilities_store.rs:121`
  to `:133`) was that a *new* collection has no rows to damage. With Q2 (a),
  a call on an *existing* collection with a different list drops both FTS5
  tables and rebuilds them from every row, on the single writer thread, which
  blocks every write of that service (D-B4a-10). Plan point 5 says "a rebuild
  destroys no row, so the reason still holds", but it does not look at cost.
  Point 14 says no caller needs the native `text_fields` at all.
- **Failure case:** A remote caller that may reach the native data route
  alternates `text_fields: ["title"]` and `["title","summary"]`. Each call is a
  few bytes and costs a full rebuild of a large collection.
- **Fix:** Do not add `text_fields` to the native DTO (W7): leave `Req`
  as it is and set `text_fields: vec![]` in the schema it builds. Then only
  the service's own code (WASM guest or linked native build) can declare text
  fields, as the plan intends. Remove W7 from section 4 and the DTO line from
  Step 4.3. If the owner wants the native route to take the list later, it
  should be gated like `drop-collection` and have its own slice. Also add to
  the "Not tested" or backlog text that a guest can force a rebuild of its
  own index by changing its list (that is by design, and only affects the
  guest's own service).
- *Status:* Fixed (e0aab8b8). Now in: Q2 owner answer; section 1; section 2.2 points 4 and 14; section 3 deviations; W7 in section 4; Step 4 item 3; section 7.2 (`control_plane`); section 7.4; section 11.

#### P4 — `query_terms` keeps punctuation at the edge of a word, so a normal typed query misses (major)

- **Where:** D-B4a-2 ("Terms are split only on white space. Punctuation stays
  inside a term: `ab"c` is one term") and its test list ("`ab"c` is one long
  term"), D-B4a-8 (long/short split), section 7.2.
- **Problem:** A long term goes to the trigram table as a phrase, which is a
  substring match on the stored key. A term such as `plumber,` (a person
  typed "plumber, chennai") needs the comma to follow `plumber` in the
  stored text. The stored key is "chennai plumber", so there is no match,
  although the user typed the right words. The same holds for `"nal"`,
  `(ek)` and `wala.`. A 2-character term with punctuation also goes to the
  wrong table: `ek,` has 3 `char`s, so it is a long term and is matched as a
  substring including the comma; trimmed it would be short (`ek`) and go to
  the word table. The word table already ignores punctuation, so only the
  trigram side is wrong. The plan treats this as B4b's job ("B4b's folding
  may"), but B4a defines `query_terms`, its long/short rule and its tests,
  and B4b would have to change all three.
- **Failure case:** `{"title": {"$search": "plumber, chennai"}}` on a row
  titled "Chennai plumber" returns nothing. ML-20 and ML-29 are met only for
  queries typed without punctuation.
- **Fix:** In `query_terms`, after the whitespace split, trim leading and
  trailing `char`s that are not alphanumeric (`char::is_alphanumeric`), then
  classify long/short on the trimmed length, then drop empty terms. Keep
  interior punctuation (`plumber's`, `a-b`). Replace the `ab"c` test with
  tests for `plumber,` → long `plumber`; `(ek)` → short `ek`; `"nal"` → long
  `nal`; `plumber's` unchanged; `a-b` unchanged; and add a `$search` test for
  `plumber, chennai`. Update the "drop rule" sentence of D-B4a-2 (a term of
  only punctuation is now simply empty after trimming).
- *Status:* Fixed (e0aab8b8). Now in: Q4 owner answer; D-B4a-2; section 7.2 (`query_terms` and Search).

#### P5 — A NUL character in `$search` text becomes an `Internal` error (minor)

- **Where:** D-B4a-8 (check list), section 7.2 ("Filter"); `errors.rs:14`
  (`map_rusqlite_error`).
- **Problem:** I fed FTS5 the planned expression with a NUL inside the quoted
  term (`f_a : "a\x00b"`, on both tables): FTS5 answers "unterminated string".
  `map_rusqlite_error` maps that to `DataLayerError::Internal`. A guest or a
  remote caller can send a JSON string with `\u0000`. The plan's refusal list
  has no rule for it. Other control characters are harmless (they gave no
  error).
- **Failure case:** `{"title": {"$search": "a\u0000b"}}` returns
  `internal("…unterminated string")`, which looks like a host fault.
- **Fix:** In `compile_search`, refuse text that contains `'\0'` with
  `schema-violation("$search text must not contain NUL")`, in the check list
  after the `string` check. Add a test to the "Filter" list. Also run the same
  term through the stored-text path: `sync_row` must accept a NUL in a
  *stored* value (check it with a test; SQLite stores it, but the plan does
  not say so).
- *Status:* Fixed (e0aab8b8). Now in: D-B4a-6 (`sync_row`); D-B4a-8 (`compile_search`); Step 6 item 2; section 7.2 (Writes and Filter).

#### P6 — The Step 2 prefix check passes even when the prefix index is missing (minor)

- **Where:** Step 2 and section 7.2 ("The SQLite build", first bullet), D-B4a-3.
- **Problem:** An FTS5 prefix query (`"e"*`) returns correct rows whether or
  not `prefix='1 2'` created a prefix index: without it, SQLite reads every
  matching term instead. So "queries return the right rows" cannot prove that
  the index is there, and the purpose of ML-29 is cost, not correctness. I
  checked on 3,000 random rows: the `_data` shadow table holds 160,170 bytes
  without `prefix` and 256,505 bytes with `prefix='1 2'`.
- **Failure case:** A typo such as `prefix = '1,2'` or a tokenizer string that
  drops the option still passes Step 2 and every later test. Short queries
  then run slowly on large collections with no failing test.
- **Fix:** Add to the Step 2 tests (a) a check that `sqlite_master.sql` of the
  word table contains `prefix`, using the `WORD_TOKENIZE` text, and (b) a size
  check on the same rows: `sum(length(block))` of `<word>_data` is larger
  with the option than without. Say in the plan that (b) is the proof.
- *Status:* Fixed (e0aab8b8). Now in: D-B4a-3; Step 2 item 2; section 7.2 (The SQLite build).

#### P7 — Existing tests call `do_delete_many` with `&conn`, and the plan does not name them (minor)

- **Where:** D-B4a-6 (`do_delete_many(conn: &mut Connection, …)`), Step 5.4,
  section 7.5; `crates/data_db/src/sqlite/tests.rs:864` and `:870`.
- **Problem:** The signature change breaks the two calls in
  `fdae_watchdog_interrupts_do_delete_many_on_the_writer_conn`. That file is
  capped at 899 lines, so the plan's "about −9" estimate matters. The test
  also says what must stay true: the first call (a watchdog stop) is
  `QuotaExceeded`, and the second call on the same connection deletes 1 row.
  With `DELETE … RETURNING` inside a transaction, the stop appears while the
  statement runs; the plan says the error still goes through
  `map_query_raw_step_error`, but no test is named for it.
- **Failure case:** Step 5.4 does not build: `tests.rs` needs `let mut conn`
  and `&mut conn` in two places. If the `RETURNING` form maps the interrupt to
  `Internal`, the test fails in the final run.
- **Fix:** Name `tests.rs:860` to `:872` in Step 5.4 and in 7.5 (no line
  change: `let mut conn`, `&mut conn`). State that the test must pass
  unchanged, and that the transaction is dropped (rolled back) on the
  watchdog error before the next call.
- *Status:* Fixed (e0aab8b8). Now in: Step 5 item 4; section 7.5 (`sqlite/tests.rs`).

#### P8 — Living docs and WIT docs: one false sentence is not listed, and the write docs do not say a write can now be refused (minor)

- **Where:** Section 9, D-B4a-1, W4; `docs/system-architecture.md:2765`;
  `data-layer.wit` docs of `put`, `patch`, `batch-mutate`, `create`.
- **Problem:** Architecture line 2765 ("Schema & Indexing") says: "A write is
  checked only to be valid UTF-8 JSON: unknown fields are accepted and field
  types are not checked." After B4a, a write to a collection with text fields
  can be `schema-violation("text field 'x' must be a string")` (W4). Section 9
  lists `:2761`, a new bullet after `:2768`, and `:568`, but not `:2765`. The
  same refusal is not in the WIT docs of the four write functions, so a guest
  author does not learn it from the interface. The same line also says the
  index lock "blocks the single writer", which is the place to add that a text
  index rebuild does the same (D-B4a-10).
- **Failure case:** A reader of the architecture doc or the WIT believes a
  write cannot fail on field type.
- **Fix:** Add `:2765` to section 9 (change the sentence: field types are not
  checked, except the declared text fields). Put one line in the doc of
  `put`, `patch`, `batch-mutate` and `create`, and in the ADR-0007 amendment.
- *Status:* Fixed (e0aab8b8). Now in: D-B4a-1; section 9 (`docs/system-architecture.md:2765`, `data-layer.wit` docs).

#### P9 — An old row with a non-string text field blocks unrelated patches (minor)

- **Where:** Q5 (a), D-B4a-6 (`do_patch`: "`check_values` on the merged
  value"), section 7.2 ("Declare": "a stored row whose `title` is a number").
- **Problem:** Q5 (a) says a rebuild must not fail on rows written before the
  declaration, so it skips a non-string value. But `do_patch` checks the whole
  merged payload. A patch `{"status":"closed"}` on that old row then fails
  with `schema-violation("text field 'title' must be a string")`, although
  the patch did not touch `title`. The row can no longer be updated until the
  guest rewrites `title`. `put` is fine (it replaces the payload).
- **Failure case:** A service declares text fields on a collection that has
  `title: 5` in one old row. Every patch of that row fails, and the guest
  cannot tell why from the patch it sent.
- **Fix:** In `do_patch`, check only the text fields that the patch document
  names (top-level keys of the patch), and index the merged value as before
  (non-string is indexed as empty). Add a test: patch an old row with a
  numeric `title`, changing another field, succeeds; patching `title` to a
  number is refused.
- *Status:* Fixed (e0aab8b8). Now in: Q5 owner answer; D-B4a-6 (`do_patch`); Step 5 item 4; section 7.2 (Writes).

#### P10 — `declare` compares the field list by order and exact case, so two call sites thrash the index (minor)

- **Where:** D-B4a-6 (`declare`: `have == wanted`), D-B4a-7 (duplicates are
  checked "ignoring ASCII case"), D-B4a-4 (`_text_index.collection` is a
  `TEXT PRIMARY KEY`).
- **Problem:** Two call sites of one collection that list the same fields in
  different order (`["title","summary"]` and `["summary","title"]`) never
  equal each other, so each call drops and rebuilds. The plan already expects
  several call sites per collection (`ensure_coll`, 12 callers in the
  directory). FTS5 column names ignore case, and SQLite table names ignore
  case, but `_text_index.collection` is compared as exact text: `Foo` and
  `foo` are the same SQLite table with two `_text_index` rows, and
  `drop_index("foo")` leaves the `Foo` row.
- **Failure case:** Directory code with two helpers that list fields in
  different order rebuilds the index on every request.
- **Fix:** Compare `have` and `wanted` as sets, ignoring ASCII case. Keep the
  order of the existing index when they are equal. Store the collection name
  in `_text_index` in lower case (and look it up the same way). Add tests:
  same fields in another order do not rebuild (marker row survives); `Foo`
  and `foo` share one `_text_index` row.
- *Status:* Fixed (e0aab8b8). Now in: Q2 owner answer; D-B4a-4; D-B4a-6 (`declare`); Step 5 item 2; section 7.2 (Declare).

#### P11 — Every write of every service gets a schema read and a transaction, and nobody measures it (minor)

- **Where:** D-B4a-5 (`fields` runs on every write), D-B4a-6 (the no-policy
  path opens a transaction), section 4 (behaviour changes 1 to 3), D-B4a-10.
- **Problem:** The plan measures only a 10,000-row rebuild and one query. But
  every `put`, `patch` and `delete` of every collection, including all Roym
  services that declare no text field, now runs one more statement
  (`pragma_table_info` on a table that does not exist) and, without a policy,
  one explicit transaction. Both are small, but this is the hottest path of
  the data layer, and there are benches for it
  (`crates/data_db/benches/security_config_bench.rs`, put/get;
  `crates/sandbox_wasm/benches/data_layer_bench.rs`).
- **Failure case:** None known. The risk is a silent loss that nobody sees
  until a later bench run.
- **Fix:** Add to Step 6 (or 8) a before/after run of the `put` bench on a
  collection without text fields, and put both numbers in the PR text, as is
  done for the rebuild. The owner sets the stop limit; I suggest 10%. If it is
  over, cache the field list per collection on the writer thread (a
  `HashMap`, cleared by `declare`, `drop_index` and `execute-ddl`).
- *Status:* Fixed (e0aab8b8). Now in: D-B4a-10; Step 6 item 5; section 7.2 (Performance).

#### P12 — `execute-ddl` can leave a stale index, and the error then looks like "collection not found" (minor)

- **Where:** D-B4a-6 (`declare` rebuild conditions), Q6 (a), section 7.4
  ("Not tested"); `errors.rs:21` (`is_no_such_table`).
- **Problem:** `execute-ddl` runs any SQL (admin only). If it drops or
  recreates collection `c`, or drops one text table, the index is out of step.
  A recreated empty `c` that gets new rows reuses `rowid`s, so old FTS rows
  point at new records; `declare` sees `have == wanted`, the tables and the
  key version, and does not rebuild. If only `_text_c_tri` is missing,
  every write fails with "no such table: _text_c_tri", and
  `map_rusqlite_error` turns that into `CollectionNotFound`, although `c`
  exists.
- **Failure case:** An admin tool recreates a collection through
  `execute-ddl`; searches then return unrelated records until someone calls
  `create-collection` with a changed list.
- **Fix:** Say in the plan (section 11 and a backlog row) that `execute-ddl`
  on a text-indexed collection or on a `_text_` table is not supported and
  that the index must be rebuilt after it (set the key version to 0, then
  `create-collection`). Make `sync_row` and `remove_rows` map a missing
  `_text_` table to `Internal("text index of '<c>' is missing")` and not to
  `CollectionNotFound`. Add one test for that mapping.
- *Status:* Fixed (e0aab8b8). Now in: Q6 owner answer; D-B4a-6 (`sync_row`, `remove_rows`); Step 5 items 2 and 4; section 7.2 (Writes); section 7.4; section 8 (new backlog row); section 11.

#### P13 — `delete-many` can use two set statements instead of collecting every `rowid` (minor)

- **Where:** D-B4a-6 (`do_delete_many`: `RETURNING rowid`, collected, then
  `remove_rows` per rowid).
- **Problem:** The plan collects all deleted `rowid`s in a `Vec` and then runs
  two `DELETE` statements per row. For a large delete this is 2N statements
  and N rowids in memory, and the loop runs after the watchdog budget. A
  simpler form needs no `RETURNING` and no collection: inside the same
  transaction, run `DELETE FROM _text_c_tri WHERE rowid IN (SELECT rowid FROM
  c WHERE <same where>)`, the same for `_word`, then `DELETE FROM c WHERE
  <same where>` and take the row count from the last one. I checked that the
  `DELETE … WHERE rowid IN (subquery MATCH)` form plans as `SEARCH c USING
  INTEGER PRIMARY KEY (rowid=?)`.
- **Failure case:** None. This is about size, time and code (one helper less).
- **Fix:** Use the set form, and keep `map_query_raw_step_error` on all three
  statements so a watchdog stop is still `QuotaExceeded`. This also removes
  `RETURNING` from behaviour change 2. Keep `RETURNING rowid` for `put`,
  `patch` and `delete` (one row each).
- *Status:* Fixed (e0aab8b8), as I wrote it. My advice was wrong: the set form breaks a `delete-many` that has a `$search` filter. See P15, which replaces this fix.

#### P14 — Small facts to fix in the plan (minor)

- **Where:** Section 2.2 point 4 and Q1; section 2.2 point 13, Q6.
- **Problem:** (a) The plan says "45 places in 32 files". `git grep -c
  'CollectionSchema {' -- '*.rs'` gives 45 literals in 30 files. The plan's
  list also says "one bench" for `sandbox_wasm` and "two benches" for
  `data_db`; the benches hold 2 literals each (`data_layer_bench.rs`,
  `fdae_bench.rs`, `security_config_bench.rs`). (b) The chat store's
  `messages_fts` is at `crates/conversation/src/store/schema.rs:116`, not
  `:115`; `:115` is a blank line (task.md has the same old number).
  (c) Section 2.2 point 9 says `unicode61`'s `categories` option arrived in
  SQLite 3.25. I could not confirm that number; it works in 3.50.4, which is
  all that matters, so drop the version.
- **Failure case:** A reader who counts files finds 30 and doubts the rest.
- **Fix:** Correct the numbers and the link; remove the unconfirmed version.
- *Status:* Fixed (e0aab8b8). Now in: Q1 (a); Q6 (a); section 2.2 points 4, 9, 13.

## Plan review round 2 (check of the fixes) — Claude Sonnet 5.5, 2026-11-11

Checked on `feat/m08b-b4a-full-text` at `e0a7b8bf`, in the main checkout. I
read the new plan and compared it with this file and with the code. I ran two
small experiments outside the repository (Python `sqlite3`, SQLite 3.50.4, and
a one-file `rustc` program). I changed no code and did not run `cargo`.

**Summary:** P1 to P12 and P14 are confirmed fixed. P13 is fixed as written,
but my advice for it was wrong (P15). All 3 new items (P15, P16, P17) Fixed (13ce6aa8). 0 open.

### Confirmed fixed

- **P1.** Section 4 (W3), Q3, D-B4a-8, D-B4a-9, the Filter, Search and Plan
  tests, exit criterion 5 and the new backlog row all say "`AND` term only".
  `filter.rs` has exactly five private functions that build SQL
  (`compile_document`, `compile_entry`, `compile_logical_array`,
  `compile_field`, `compile_operator_map`), and the only operators that make
  a branch optional are `$or` and `$not`. There is no `$nor` and no field
  level `$not`. So the flag covers every case. The plan test now runs with and
  without `ANALYZE`.
- **P2.** `git ls-files -s` shows 14 symlinks and exactly the three real
  files named in the plan (all three have blob `5d91eff1`). Step 4.2 and
  exit criterion 11 are right. The canonical file is at
  `wit/data-layer/`, not `wit/deps/data-layer/`, so criterion 11 does not
  catch it by mistake. See P17 for the tests of Step 4.5.
- **P3.** W7, Step 4.3, section 2.2 points 4 and 14, section 7.4 and section 11
  all say the native route sends `text_fields: vec![]`.
- **P4, P5, P6, P7, P8, P9, P10, P11, P12.** Each is in the places the status
  lines name (D-B4a-2, -3, -4, -6, -8, -10, Steps 2, 5, 6, section 7.2, 8, 9).
  P4 has a flaw of its own: see P16.
- **P14.** The counts are right: `git grep -c 'CollectionSchema {'` gives 45
  literals in 30 files, and the `schema.rs:116` link and the removed SQLite
  version are done.
- **Sha.** `e0aab8b8` is on the branch and holds the plan change.

### Findings

#### P15 — The set form of `delete-many` does not delete when the filter has `$search` (major)

- **Where:** D-B4a-6 (`do_delete_many`), section 4 behaviour change 2, Step 5
  item 4, Step 6 item 3, section 7.2 ("Writes": "`delete-many` with a filter
  removes exactly the matching rows"). This comes from my own advice in P13.
- **Problem:** The plan runs three statements with the same `WHERE`:
  `DELETE FROM _text_c_tri WHERE rowid IN (SELECT rowid FROM c WHERE W)`, the
  same for `_word`, then `DELETE FROM c WHERE W`. When `W` contains `$search`,
  `W` reads the two FTS tables. The first statement removes the trigram rows,
  so `W` no longer matches in the next two statements. I ran this on SQLite
  3.50.4 with three rows and the filter "long term `plumber` and short term
  `ek`" (one row matches). Result: the trigram `DELETE` removed 1 row, the word
  `DELETE` removed 0 rows, the collection `DELETE` removed 0 rows. All three
  records stayed, and the matching record lost its trigram row. The order
  cannot be fixed by swapping the statements, because every order destroys
  something that a later `W` reads.
- **Failure case:** `delete-many` with `{"title": {"$search": "plumber"}}`
  returns `0` and deletes nothing, or deletes part of the index. The index is
  then out of step with the collection, with no error. A test of "`delete-many`
  with a filter" that uses a plain equality passes, so the plan's tests do not
  see it.
- **Fix:** Go back to the first design, which is correct: `DELETE FROM c WHERE W
  RETURNING rowid`, collect the rowids, then remove them from the two tables.
  To avoid 2N statements, remove them with one statement per table:
  `DELETE FROM _text_c_tri WHERE rowid IN (SELECT value FROM json_each(?1))`,
  with the rowids as one JSON array (the same for `_word`). Only do this when
  the collection has text fields. Keep `map_query_raw_step_error` on all
  statements. Restore `RETURNING rowid` in behaviour change 2. Add these tests:
  `delete-many` with a `$search` filter (and one with `$search` next to an
  equality) removes the right rows from the collection and from both FTS
  tables, and returns the right count; a watchdog stop is still
  `quota-exceeded`.
- *Status:* Fixed (13ce6aa8). Now in: behaviour change 2 in section 4; D-B4a-6 (do_delete_many); Step 5 item 4; Step 6 item 3; section 7.2 (Writes).

#### P16 — The term trim of P4 removes a trailing virama or nukta, and the plan says it does not (minor)

- **Where:** D-B4a-2 (`query_terms`; "The rule keeps every Devanagari or Tamil
  word, because their letters are alphanumeric"), section 7.2 (`query_terms`
  tests).
- **Problem:** The trim uses `char::is_alphanumeric`. I ran it: U+094D
  (Devanagari virama) and U+0BCD (Tamil pulli) and U+093C (nukta) are **not**
  alphanumeric; vowel signs (U+093E, U+0BC6, U+0BC8) and anusvara (U+0902) are.
  So a word that ends in a virama loses it: `நல்` (3 chars) becomes `நல`
  (2 chars), changes from a long term to a short term, and a `प्` becomes
  `प`. The rows that should match still match (the shorter text is a prefix or
  a substring), so this is not a miss. But it matches more rows than the
  user asked for, and the sentence in the plan is false. The sentence also
  says the rule "keeps every" word, which the new tests do not check.
- **Failure case:** A search for `नल्` also finds a row with `नल`, and the
  long/short split changes for Tamil words that end in a pulli.
- **Fix:** Trim only characters that are punctuation or symbols, not marks.
  The cheapest correct rule is to stop trimming at any `char` that is
  alphanumeric **or** has the Unicode general category Mark. `icu_properties`
  2.4 is already in `Cargo.lock` (through `idna`), and
  `GeneralCategoryGroup::Mark` answers that. Or trim only ASCII punctuation and
  a fixed list of Unicode quotes and dashes, and say so. Fix the sentence in
  D-B4a-2 and add tests: `நல்` stays one 3-char long term; `प्` stays 2 chars;
  `नल।` becomes `नल` (the danda is punctuation).
- *Status:* Fixed (13ce6aa8). Now in: Q4 owner answer; D-B4a-2; Step 3 item 1; section 7.2 (crates/text_lang/src/search_key/tests.rs).

#### P17 — The Step 4.5 test list does not name the tests that load `saga` and `miniapp-demo1-wasm` (minor)

- **Where:** Step 4.5 and section 7.2 ("Step 4.5 component tests").
- **Problem:** The plan says to "run the tests that load the updated
  components: `saga`, `miniapp-demo1-wasm`, and the websocket test in
  `crates/sandbox_wasm/tests/`". Only the websocket test is in that folder
  (`websocket_integration.rs`). The other two are
  `crates/substrate/tests/saga_e2e.rs` and
  `crates/substrate/tests/miniapp_demo1_wasm_e2e.rs`. The `-p` list of Step 4.5
  has no `syneroym-substrate`, so a developer who follows the list does not
  run them. This matters because P2 is about exactly these two components.
- **Failure case:** The symlink change is not checked until `mise run verify`,
  about 20 minutes later.
- **Fix:** Name the three files, and add
  `cargo nextest run -p syneroym-substrate --test saga_e2e --test
  miniapp_demo1_wasm_e2e` (sandbox off, as the other e2e tests need) to Step
  4.5.
- *Status:* Fixed (13ce6aa8). Now in: Step 4 item 5; section 7.2 (Step 4.5 component tests).
