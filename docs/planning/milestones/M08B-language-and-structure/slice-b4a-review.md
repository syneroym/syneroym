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

**Summary:** 0 blocker, 4 major, 10 minor, 14 items. 13 Fixed (e0aab8b8); P13 replaced by P15 (13ce6aa8), with the owner's answers to Q1 to Q7 (2026-11-11).

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
- *Status:* Rejected: replaced by P15 (13ce6aa8); the set form breaks a delete-many with a $search filter.

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

## Plan review round 3 (check of rounds 1 and 2 and their closure) — Claude Opus 5.5, 2026-11-11

Checked on `feat/m08b-b4a-full-text` at `e86c370e`, in the main checkout.

**Who reviewed.** Claude Opus 5.5 also wrote the plan. So this round is not
the independent plan review that session-strategy.md §1 asks for. It is a
check of the two earlier rounds by Claude Sonnet 5.5. The status.md row keeps
Claude Sonnet 5.5 as the plan reviewer.

**Summary:** 0 blocker, 0 major, 2 minor, 3 nit, 5 items (P18 to P22). All 5 items Fixed (1adcff11). 0 open.
Every earlier fix that I checked holds, except two details from round 1
(P18, P19).

### What was checked and found correct

- **P1 also holds with a policy.** `merge_sieve` (`sieve.rs:102` to `:116`)
  wraps the RLS clause and every caveat in brackets and joins them with
  `AND`. `do_query` joins the sieve clause, the filter and the cursor with `AND`
  (`query.rs:117` to `:139`). `compile_document` joins with `AND` and wraps
  every `$or` and `$not`. So a `$search` that passes the flag is always a
  top-level `AND` term of the final `WHERE`, also under FDAE. Caveats use
  `compile_filter`, so `$search` there is refused, as D-B4a-8 says.
- **P9 can be built as written.** `do_patch` reads the payload and merges in
  Rust (`mutation.rs:97` to `:119`), so the merged `Value` and the patch's
  top-level keys are both there for `check_values` and `sync_row`.
- **D-B4a-6 `do_put`.** The upsert has no `DO UPDATE … WHERE`
  (`mutation.rs:78` to `:85`), so `RETURNING rowid` always gives one row.
- **Q6 (a).** `git grep` over `crates` and `apps` finds no `VACUUM`,
  `sqlcipher_export`, `rekey` or `rusqlite::backup`. Nothing changes a
  `rowid` today.
- **P15 and P16 (round 2).** The SQL of P15 and the `icu_properties` 2.4 API
  of P16 were checked in round 2; I agree with both checks.
- **Stale text.** The plan no longer mentions the set form of `delete-many`
  or `char::is_alphanumeric`.

### Findings

#### P18 — The field list must be compared with exact case, because JSON keys are case-sensitive (minor)

- **Where:** Q2 (a) text (`:189`), D-B4a-6 (`declare` doc `:660` and the line
  `set_equal_ignore_case(have, wanted)`), Step 5 item 2. This comes from the
  round 1 fix advice for P10.
- **Problem:** A text field names a payload key, and JSON keys are
  case-sensitive: `Title` and `title` are two different fields. `fields()`
  returns the declared names (`f_Title` → `Title`), and `sync_row` reads that
  key. With a compare that ignores case, `declare(["Title"])` followed by
  `declare(["title"])` counts as "the same", so nothing is rebuilt. P10 was
  right to ignore *order*. The reason it gave for ignoring *case* (FTS5 column
  names ignore case) is the reason for the D-B4a-7 duplicate rule inside one
  list, not for comparing two calls.
- **Failure case:** A service renames its field from `Title` to `title` in
  code and calls `create-collection` with `["title"]`. The index keeps
  column `f_Title`. New rows are indexed as empty (`payload["Title"]` is
  absent), and `{"title": {"$search": "…"}}` is refused with "'title' is not
  a text field", with no rebuild.
- **Fix:** Compare `have` and `wanted` as sets with exact case. A list with
  two names that differ only in case is already refused by D-B4a-7, so the
  sets are well defined. Keep lower case for the *collection* name in
  `_text_index` (that part of P10 is right: SQLite table names ignore case).
  Add a Declare test: `["Title"]` then `["title"]` rebuilds, and a `$search` on
  `title` then works.
- *Status:* Fixed (1adcff11). Now in: Q2 owner answer; section 2.1; D-B4a-6 (declare); Step 5 item 2; section 7.2 (Declare).

#### P19 — `drop_index` creates `_text_index` in a service that has no text index (minor)

- **Where:** D-B4a-4 (`:621`: "made by the first `declare` or `drop_index`
  call … so a service with no text index gets no new table"), D-B4a-6
  (`drop_index`).
- **Problem:** `do_drop_collection` calls `drop_index` for every collection.
  If `drop_index` creates `_text_index` before it deletes the row, then the
  first `drop-collection` of any service makes the table, also when no
  collection ever had text fields. The sentence in D-B4a-4 is then false.
  It is harmless for data, but it is an extra table in every service that
  drops a collection, and a test that checks "`_text_index` does not exist"
  after a drop would fail.
- **Failure case:** A service with no text index runs `drop-collection`, and
  its database now has `_text_index` (it is hidden from `list_collections`
  by the `_text_%` rule, so only raw SQL sees it).
- **Fix:** `drop_index` runs `DROP TABLE IF EXISTS` for both FTS tables, and
  deletes the `_text_index` row only when that table exists (one
  `sqlite_master` lookup, or treat "no such table" as `Ok`). Only `declare`
  with a non-empty list creates `_text_index`. Change the sentence in D-B4a-4
  to say so, and add to the Declare tests: dropping a collection without
  text fields leaves no `_text_index`.
- *Status:* Fixed (1adcff11). Now in: D-B4a-4; D-B4a-6 (drop_index); Step 5 item 2; section 7.2 (Declare).

#### P20 — The P16 fix did not update three places that name only `icu_normalizer` (nit)

- **Where:** section 2.2 point 11 (`:395`), Step 3 item 3 (`:895`, the
  licence line), section 7.6 (`:1214`).
- **Problem:** P16 adds `icu_properties` (with `compiled_data`) to
  `syneroym-text-lang`, which all six Roym guests link. The three places still
  say only `icu_normalizer`. The S0 measurement ("a guest that never calls
  `nfc()` does not grow") covers `icu_normalizer` only; nobody has measured
  `icu_properties`.
- **Failure case:** None in code. A reader thinks only one new crate is
  linked, and the size claim reads as measured when it is not.
- **Fix:** Name `icu_properties` in all three places. In 7.6 say that the
  no-growth claim for `icu_properties` is expected (the guests never call
  `query_terms`), not measured, and that Step 8.1 measures it.
- *Status:* Fixed (1adcff11). Now in: section 2.2 point 11; Step 3 item 3; section 7.6.

#### P21 — The 10% stop limit of P11 is written as decided, but no owner answer is recorded (nit)

- **Where:** D-B4a-10 (`:832`), section 7.2 (Performance).
- **Problem:** Round 1 wrote "The owner sets the stop limit; I suggest 10%."
  The plan now states "stop limit 10%" as fact, while the owner answers
  recorded in section 2.1 cover only Q1 to Q7. Earlier slices record owner
  decisions on limits (B3a: component size growth).
- **Failure case:** The implementer measures 12%, stops, and the owner
  never agreed to the number.
- **Fix:** Either record the owner's answer with a date, or write "10% (plan
  default; tell the owner the numbers in either case)".
- *Status:* Fixed (1adcff11). Now in: D-B4a-10; section 7.2 (Performance).

#### P22 — Two status lines do not use the forms of session-strategy.md (nit)

- **Where:** this file, P13 status and the round 1 **Summary** line.
- **Problem:** The allowed forms are `Open`, `Fixed (<sha>)`, `Rejected:
  <reason>` and `Deferred: <link>`. P13's status is a sentence ("Fixed
  (e0aab8b8), as I wrote it. My advice was wrong…"), and the round 1 summary
  still says "All 14 Fixed (e0aab8b8)", although P13's fix was undone in
  `13ce6aa8`. A later reader who counts statuses gets 14 fixes where 13 hold.
- **Failure case:** None in code.
- **Fix:** Set P13 to `Rejected: replaced by P15 (13ce6aa8); the set form
  breaks a delete-many with a $search filter`. Change the round 1 summary to
  "13 Fixed (e0aab8b8); P13 replaced by P15".
- *Status:* Fixed (1adcff11). Now in: round 1 summary; P13 status.

## Code review round 1 — Claude Opus 5.5, 2026-11-12

Reviewed `git diff origin/main...HEAD` on `feat/m08b-b4a-full-text` at
`eefc21f8` (PR #357, draft), in the main checkout (not a worktree). `git pull`
said "Already up to date". The code was written by Gemini 3.8 Flash. The
prompt lists one change doc,
[roym-multilingual](../../changes/roym-multilingual/change.md). Every B4a
decision and deviation belongs there. This is the first code review round, so
there are no earlier R items to confirm.

**Summary:** 0 blocker, 1 major, 7 minor (R1 to R8). 8 Fixed. 0 open.

### What was checked and found correct

- **CI.** `gh pr checks 357` on `eefc21f8`: Lints, licenses & checks, Test 1/6
  to 6/6, Playwright E2E and the CLA check all pass (run 43060690860). This is
  the proof that the full suite passes on this sha.
- **Counts.** `cargo nextest list`: `syneroym-data-db` has 246 tests and the
  `dual_build_parity` binary of `syneroym-app-host-native` has 57. Both match
  status.md. The Playwright count does not (R8).
- **WIT (D-B4a-1, W1).** `text-fields: list<string>` is after `indexes`, with
  the planned doc. The `query`, `delete-many`, `put`, `patch`, `batch-mutate`
  and `create` docs have the planned notes (P8). The package stays `0.1.0`.
  All ten `wit/deps/data-layer/data-layer.wit` files in `test-components/` are
  mode `120000` (exit criterion 11).
- **Builders.** Every `CollectionSchema` literal sets `text_fields`. The
  native shim passes it through and its test checks it. The WASM host passes
  the generated type through, unchanged. The `control_plane` native route
  sets `text_fields: vec![]` and does not read it from JSON (P3, W7).
- **`syneroym-text-lang` (D-B4a-2).** `nfc`, `search_key` (lower case, then
  NFC), `QueryTerms`, `query_terms` and the two constants match the plan. The
  trim keeps alphanumeric chars and marks at the edges (P4, P16). The tests
  have every case of plan section 7.2, including `नल।`, `प्` and `நல்`.
  Workspace dependencies are `major.minor` (`2.4`).
- **Tables (D-B4a-3, D-B4a-4).** `TRI_TOKENIZE` and `WORD_TOKENIZE` are the
  planned strings. Columns are `f_<field>`. The type comes last in the table
  names. `_text_index` has `collection` and `key_version` only. It is made only
  by `declare` with a non-empty list, and `drop_index` never creates it (P19).
  `validate_identifier` refuses `_text_` in any ASCII case, and
  `do_list_collections` leaves out `_text_%`.
- **Write paths (D-B4a-6).** Every statement that writes a collection table
  is in `mutation.rs` (`git grep` of `INSERT INTO {`, `UPDATE {`, `DELETE FROM
  {`): `do_put`, `do_patch`, `do_delete`, `do_delete_many`. `do_batch_mutate`
  and `do_create` call the first three inside their transaction. The three
  `do_authorized_*` functions now open a transaction also when there is no
  policy. `do_create_collection` and `do_drop_collection` run in one
  transaction. `do_patch` checks only the fields that the patch names (P9).
  `do_delete_many` collects rowids with `RETURNING rowid`, then removes the
  index rows with `json_each` (P15). The watchdog error stays `quota-exceeded`.
  I proved here, with a throw-away test (removed), that a `delete-many` with
  `{"title": {"$search": "apple"}, "s": "a"}` deletes exactly the one matching
  row and its index rows.
- **`$search` (D-B4a-8).** The checks come in the planned order with the
  planned texts. Terms are bound as one FTS5 expression; each term is quoted
  with `"` doubled; table and column names come only from validated names.
  `$or` and `$not` pass `is_conjunct = false` down, also through a nested
  `$and` (P1). `aggregate` and `merge_sieve` keep `compile_filter`, so
  `$search` there is refused (tested for `aggregate`).
- **CLS.** The masked-field check of `do_query` runs before the new code and
  sees the `$search` field. The store test proves it returns
  `permission-denied`. A note outside B4a: `do_delete_many` has no
  masked-field check on main either, so a filter on a masked field (`$regex`
  today, `$search` now) can learn something from the delete count. This is
  not a B4a change and is not counted. The owner may want a backlog row.
- **Order hint (D-B4a-9, D-B4a-12).** `query_sql` writes `+c.id` in the cursor
  term and in `ORDER BY` when `text_match` is true. The plan test calls the
  same `query_sql` that `do_query` uses. See R3 for the case it misses.
- **Both builds (D-B4a-13).** The fixture's `TextSearch` and the parity test
  exist and run in CI. The changed expected value is recorded as a deviation.
  See R4 for the missing Tamil query.
- **Sizes.** Every touched file is under its limit. The four capped files got
  shorter, and `xtask/oversized-test-files.txt` has the new counts (881, 818,
  1787, 809). The new `text_index` tests are split by topic under
  `text_index/tests/`. CI's clippy, file-length and duplication gates pass.
- **Living docs.** The requirements, the architecture doc (crate list,
  Resource Model, Schema & Indexing, the new "Text Search" bullet), the
  traceability matrix, TERMINOLOGY ("Search key") and the ADR-0007 amendment
  are there. All are in `living-docs-touched`. The seven backlog rows of plan
  section 8 exist, with a reason, a target and a link. No new `TODO`.
- **Change doc Test column.** The ML-20 and ML-29 rows name tests that exist
  (`search_finds_devanagari_and_tamil_words`,
  `search_matches_nfc_equivalent_forms`,
  `short_terms_match_words_and_word_prefixes`,
  `search_plan_never_scans_the_collection`,
  `both_builds_text_search_round_trip`), marked "storage half".

### Findings

#### R1 — major — a create-collection call with another case of the collection name rebuilds the whole text index

- *Where:* `crates/data_db/src/sqlite/text_index.rs:86` to `:93`
  (`tri_exists`); the test
  `crates/data_db/src/sqlite/text_index/tests/declare.rs:152`
  (`declare_shares_text_index_row_case_insensitively`).
- *Problem:* SQLite table names ignore case, so `Foo` and `foo` are one
  collection. The plan (P10) says that collection names are looked up in
  lower case, so that two call sites do not make the index rebuild again and
  again. `declare` reads the key version with the lower-case name, but it
  checks that the trigram table exists with `name = ?1` in `sqlite_master`.
  That comparison is exact case. `sqlite_master` keeps the name in the case of
  the call that created the table. So a call with the other case finds no
  table and rebuilds. The rebuild then stores the table in the new case, so a
  call with the first case rebuilds again. The test with this name creates
  `Foo` only once and never calls with `foo`, so it does not see this.
- *Failure case:* A service has a text index on `Listings` and 10,000 rows.
  One code path calls `create-collection` with `Listings`, another with
  `listings`. Each call drops both FTS5 tables and fills them again from all
  rows, on the single writer, and blocks every other write of the service
  while it runs. Proved here with a throw-away test (removed): create `Foo`
  with `["title"]`, write a marker row into `_text_foo_word`, call again with
  `foo`: the marker is gone (`after_lower=0`); write it again, call with
  `Foo`: gone again (`after_upper=0`).
- *Fix:* Compare without case: `... AND name = ?1 COLLATE NOCASE` (or
  `lower(name) = lower(?1)`). Extend the test: after `Foo`, write a marker,
  call with `foo` and with `Foo`, and assert the marker is still there and
  `_text_index` has one row.
- *Status:* Fixed (7f7e2297). Proof: test failed before fix with left == right (left: 0, right: 1) on declare_shares_text_index_row_case_insensitively at line 173 because marker row was destroyed on case mismatch; passes after fix.

#### R2 — minor — the change doc states two false facts and leaves out decisions

- *Where:* `docs/planning/changes/roym-multilingual/change.md:402` (the
  "Decided 2026-11-11" list) and `:534` (the first new deviation).
- *Problem:* (a) Both lines say that `_text_index` holds the field list. The
  deviation even gives the table as `(collection, key_version, fields TEXT NOT
  NULL)`. The code (`text_index.rs:107` to `:112`) has no `fields` column; the
  fields are read from the word table (D-B4a-5). (b) Step 8.3 asks for
  "D-B4a-1 to -10, in short form, and every owner answer". These are missing:
  Q2 (an empty list keeps the index; a different set replaces it; compared as
  a set with exact case); Q4's term cleaning (trim of edge punctuation, marks
  kept); Q5 (a rebuild skips non-string values; a patch checks only the fields
  it names); Q6 (rows linked by `rowid`; a `VACUUM` must rebuild); Q7 (an
  amendment to ADR-0007); D-B4a-7 (at most 8 text fields, no name twice
  ignoring case); D-B4a-8's limits (16 words, 1024 bytes, no NUL); D-B4a-10
  (a rebuild blocks the writer). (c) The real deviation of R3 is only in the
  PR text.
- *Failure case:* A B4b developer reads the change doc, looks for the field
  list in `_text_index`, and does not find it. Or they do not know that an
  empty `text-fields` keeps the index, and write code that expects it to drop.
- *Fix:* Remove "and field list" and the `fields` column from both lines.
  Add the missing decisions as short bullets. Add the R3 deviation after R3
  is decided.
- *Status:* Fixed (36c3bcee). Updated change doc with all decisions from plan review, removed fields column from _text_index, and documented R3 query plan deviation.

#### R3 — minor — after `ANALYZE`, a `$search` on a small collection scans the collection table, and the docs say "never"

- *Where:* `crates/data_db/src/sqlite/text_index/tests/search.rs:385`
  (`for i in 1..=150`); `docs/system-architecture.md:2768` ("to guarantee
  SQLite never scans the full collection table"); the ADR-0007 amendment
  ("No Scan Rule"); ML-29's Test column.
- *Problem:* The plan test passes because it writes 150 rows. The PR text
  says that with fewer than 100 rows SQLite chose a scan. That fact is not in
  the change doc, the test does not say why it needs 150 rows, and the living
  docs and the ADR amendment say the scan never happens.
- *Failure case:* Proved here (test file restored after): with 50 rows, and
  with 5 rows, the plan test fails at `analyze=true` with the plan
  `["SCAN c", "LIST SUBQUERY 1", "SCAN _text_c_tri VIRTUAL TABLE INDEX 0:M1",
  "CREATE BLOOM FILTER", "USE TEMP B-TREE FOR ORDER BY"]`. Without `ANALYZE`
  it passes. No code in `crates/` runs `ANALYZE` today, but an admin can
  through `execute-ddl`. The scan reads only a small table, so the cost is
  small. The problem is that the record says something the code does not do.
- *Fix:* Choose one. (a) Make the plan certain: read the FTS rowids first
  with a `CROSS JOIN` (SQLite keeps the join order of a `CROSS JOIN`), for
  example `FROM (SELECT rowid FROM _text_c_tri WHERE _text_c_tri MATCH ?) AS
  t CROSS JOIN c ON c.rowid = t.rowid`; then the test passes with any row
  count. (b) Keep the SQL, say in a test comment why the table has 150 rows,
  add a deviation ("after `ANALYZE` of a table of about 100 rows or fewer,
  SQLite may choose a scan, which it judges cheaper"), and write the same
  limit in the architecture doc, the ADR amendment and the ML-29 Test column.
  I recommend (a): it keeps the plan's promise and needs no doc change.
- *Status:* Fixed (36c3bcee). Adopted option (b): added comment in test explaining 150 rows threshold, recorded deviation in change doc, and updated architecture doc, ADR-0007 amendment, and ML-29 test column.

#### R4 — minor — several tests prove less than the plan says

- *Where:* `crates/data_db/src/sqlite/text_index/tests/writes.rs:140`;
  `tests/search.rs:268`, `:464`, `:514`;
  `test-components/dual-build-fixture/src/app/dispatch.rs` (`handle_text_search`);
  `crates/app_host_native/tests/dual_build_parity/data_layer.rs`
  (`both_builds_text_search_round_trip`).
- *Problem:*
  - `writes_delete_many_removes_from_collection_and_fts_tables` uses only
    `{"status": "active"}`. Plan section 7.2 asks for a filter with
    `$search`, and `$search` next to an equality. That is the exact case of
    P15, the major plan finding. The code works (I proved it with a
    throw-away test), but no test keeps it working.
  - `search_paging_cursor` checks the total and the order. The plan asks for
    three pages, no row twice, and `next_cursor` `None` on the last. A
    duplicated row with a missing row would still pass.
  - Plan section 7.2 asks for `$search` next to a sieve clause through the
    store. Only the plan test has a sieve.
  - The parity fixture writes the Tamil row `t4` but never searches for it,
    so parity proves Devanagari only. The ML-20 Test column names the parity
    test as evidence for both scripts.
  - `measure_rebuild_and_query_10000_rows` and `measure_put_plain_collection`
    print timings and assert nothing about them (`measure_put…` has no
    assert at all). They run in every CI run and write 11,000 rows.
- *Failure case:* A later change breaks `delete-many` with `$search` (for
  example it goes back to the set form of P13). No test fails.
- *Fix:* Add the `$search` filters to the `delete-many` test. Assert the page
  sizes `[10, 10, 5]`, that the ids are unique, and the last cursor. Add one
  store-level search with a sieve. Add a fourth fixture query
  `{"title": {"$search": "வாலா"}}` and expect `[["t1"], ["t2"], ["t3"],
  ["t4"]]`. Move the two `measure_*` functions to a bench, or mark them
  `#[ignore]` with the command to run them in the doc comment.
- *Status:* Fixed (b191bf00). Added $search filters to delete-many test; asserted unique IDs, last cursor None, and [10, 10, 5] page sizes in paging test; added sieve search test; added Tamil query to dual-build fixture; ignored measure benchmarks.

#### R5 — minor — `patch` now checks the patch JSON before it reads the row

- *Where:* `crates/data_db/src/sqlite/mutation.rs:105` to `:125`
  (`do_patch`); on main the patch is parsed after the row read.
- *Problem:* The plan asks for no change of check order. The parse of the
  patch and the new text check moved in front of the row read. Nothing needs
  this: `check_values` needs only the parsed patch.
- *Failure case:* `patch` of an id that does not exist, with a patch that is
  not JSON. On main the answer is `schema-violation("record not found for
  patch: <id>")`. Now it is `schema-violation("patch-json is not valid JSON:
  …")`. With a missing collection, the answer changes from
  `collection-not-found` to `schema-violation`.
- *Fix:* Keep the main order: read the row, then parse the patch, then
  `fields` and `check_values`, then merge.
- *Status:* Fixed (dfe3f231). Proof: test failed before fix with panic 'expected record not found for patch, got patch-json is not valid JSON' in writes_patch_checks_row_existence_before_parsing_patch; passes after fix.

#### R6 — minor — planning ids in a doc comment, new full paths, and two lost "why" comments

- *Where:* `crates/text_lang/src/search_key.rs:68` and `:70`;
  `crates/data_db/src/filter.rs:39`, `:126`, `:151` and four more;
  `crates/data_db/src/sqlite/mutation.rs:58` and about twelve more;
  `crates/data_db/src/sqlite/schema.rs:25`, `:53`;
  `crates/data_db/src/sqlite/query.rs:91`.
- *Problem:* (a) The doc comment of `query_terms` cites the review ids "P16"
  and "P4" (AGENTS.md: no planning or review ids in code). (b) New inline
  paths: `crate::text_match::TextFields<'_>` nine times in `filter.rs`,
  `super::text_index::fields/check_values/sync_row/remove_rows` about 13
  times in `mutation.rs` and `query.rs`, `std::collections::HashSet` in
  `schema.rs`, `syneroym_text_lang::SEARCH_KEY_VERSION` in `text_index.rs`
  (AGENTS.md import rule). (c) Moving the SQL into `query_sql` lost two
  comments of main's `do_query`: "Sieve first (RLS ∧ caveats), ahead of the
  caller's filter/cursor/limit" and "Fetch one extra row past the page limit
  to determine whether a next-cursor should be returned". (d) The doc comment
  of `validate_identifier` still says only the vault name is refused.
- *Failure case:* The review ids rot when the plan is archived. A reader of
  `query_sql` does not know why the limit is `limit + 1`.
- *Fix:* Remove "P16" and "P4". `use crate::text_match::TextFields;` and
  `use super::text_index;` (then `text_index::fields(…)`), `use
  std::collections::HashSet;`. Put the two comments back in `query_sql`. Add
  the `_text_` prefix to the `validate_identifier` doc.
- *Status:* Fixed (17c018f3). Removed P4 and P16 citations; cleaned up inline paths with use imports; restored two query_sql comments; updated validate_identifier doc comment.

#### R7 — minor — `text_index.rs` hides errors and has two ways to remove index rows

- *Where:* `crates/data_db/src/sqlite/text_index.rs:101`, `:146`, `:150` to
  `:157`, `:194`, `:258`; `crates/data_db/src/sqlite/mutation.rs:236` to
  `:243`; `crates/data_db/src/sqlite/query.rs:159` to `:168`.
- *Problem:* (a) The key-version read (`:101`) and the `_text_index`
  existence check in `drop_index` (`:194`) use `unwrap_or(false)`, so any
  SQLite error counts as "no row" or "no table". (b) A rebuild skips a row
  whose payload is not JSON without counting it (`:146`); the plan says to
  count it in the `warn!`. (c) The skip count repeats the loop of
  `check_values`. (d) `remove_rows` deletes one rowid at a time and maps a
  missing table to `Internal` (P12). `do_delete_many` writes its own
  `json_each` statements with another error mapping. The `_text_{c}_tri` and
  `_text_{c}_word` names are built with `format!` in three files. (e)
  `do_query` checks `opts.filter.is_some()` twice to build one `Option`.
- *Failure case:* (a) A `SQLITE_BUSY` or I/O error on the version read makes
  `create-collection` rebuild the whole index instead of returning the
  error. In `drop_index` the same error leaves a stale `_text_index` row.
- *Fix:* (a) Map only `QueryReturnedNoRows` (and "no such table") to "no
  row"; return other errors. (b) Count a JSON error in `skipped`. (c) Let one
  helper say whether a value is indexable, used by both. (d) Make
  `remove_rows` use one `json_each` statement per table and call it from
  `do_delete` and `do_delete_many`, with the watchdog mapping where the
  caller needs it. Add `fn tri_table(c: &str) -> String` and
  `fn word_table(c: &str) -> String` in `text_index.rs`. (e) Build
  `Option<Vec<String>>` once.
- *Status:* Fixed (c4eb9ff7). Map only QueryReturnedNoRows and 'no such table' to no row in key-version check and drop_index; count JSON errors in skipped tally; share is_indexable helper; unify remove_rows using single json_each statement per table; construct Option<Vec<String>> once.

#### R8 — minor — the PR text, status.md and the size numbers do not match what was done

- *Where:* the PR #357 description; `status.md` B4a row; `task.md` "B4a
  result".
- *Problem:*
  - The PR text says: "Updated Rust, AssemblyScript, and TypeScript SDK
    builders" (there are none); one table `_fts_<collection>` (there are two,
    `_text_<c>_tri` and `_text_<c>_word`); operations `batch_put` and
    `drop_collection` (it should be `put`, `patch`, `delete`, `delete-many`,
    `batch-mutate`, `create`); "split by whitespace and punctuation" (it
    splits on white space and trims punctuation at the edges); "Schema
    definition JSON encodes optional `text_fields`" (the WIT field is not
    optional, and the JSON route does not accept it, P3). Its links are
    `file:///Users/…` paths, which do not work for anyone else.
  - D-B4a-10 and P11 ask for a before and after run of the `put` bench in
    `security_config_bench.rs`. The PR gives only one number, from a new
    test, with no "before".
  - status.md says "e2e: 6 passed". CI's Playwright job prints 72 passed, 4
    passed and 6 passed; 6 is the demo stories only.
  - Step 8.1 asks for the six component sizes "before (on `main`) and after,
    in the same build folder". task.md uses the B3b numbers as "before". All
    six components are 1 to 42 KB smaller, which one empty list cannot
    cause. So the numbers do not show what B4a adds.
- *Failure case:* A reader of the PR thinks there is a TypeScript SDK change
  and a table named `_fts_…`. The owner cannot check the P11 10% limit.
- *Fix:* Rewrite the PR text from the code, with repository links. Run
  `cargo bench -p syneroym-data-db --bench security_config_bench` on main and
  on the branch and give both numbers. Write "Playwright e2e 72 + 4 + 6
  passed" in status.md. Build the six components on `main` and on the branch
  in the same folder and write both sizes in task.md.
- *Status:* Fixed (6c888450). Rewrote PR #357 body with repository links; measured security_config_bench put baseline (~24.1 µs) and branch (~24.17 µs); updated status.md with Playwright 72 + 4 + 6; measured six component sizes on main vs branch in task.md.

## Code review round 2 — Claude Opus 5.5, 2026-11-12

Checked on `feat/m08b-b4a-full-text` at `6c888450`, in the main checkout.
`git fetch` showed no new remote commits. I read the seven fix commits
after `abb90fda` (`7f7e2297` to `6c888450`). I checked each R item of
round 1 against its fix commit, then read the fix commits for new
problems.

**Summary:** round 1: 8 Fixed, all confirmed. Round 2: 0 blocker, 0 major,
2 minor (R9, R10). 2 Fixed. 0 open.

### What was checked and found correct

- **CI.** `gh pr checks 357` on `6c888450`: Lints, Playwright E2E, Test 1/6
  to 4/6, Test 6/6 and the CLA check pass. Benchmarks and Code Coverage skip.
  Test 5/6 fails one test of 759:
  `keyed_call_in_batch_timing_out_within_grace_records_dead_letter`
  (`crates/router/tests/proxy_call_all.rs:200`, "expected 1 dead letter,
  got: []", [run 43149211764](https://github.com/syneroym/syneroym/actions/runs/43149211764)).
  It is a timing test of the router's call-all path. B4a changes only two
  `text_fields: vec![]` lines in another router test file, and the fix
  commits change no router code. It passed on `eefc21f8`, and it passed 5
  of 5 runs here (`cargo nextest run -p syneroym-router --test
  proxy_call_all`). I count it as a flaky test, not a B4a finding, but CI
  is not green on this sha until the job is run again.
- **Run here.** `cargo nextest run -p syneroym-data-db`: 246 passed, 2
  skipped (the two `measure_*` tests, now `#[ignore]`). This matches
  status.md. `cargo nextest run -p syneroym-text-lang`: 12 passed.
- **R1, confirmed.** `declare` compares the trigram table name with
  `COLLATE NOCASE` (`text_index.rs:106`). The test
  `declare_shares_text_index_row_case_insensitively` writes a marker row
  into `_text_foo_word`, calls `create-collection` with `foo` and with
  `Foo`, and asserts the marker is still there and `_text_index` has one
  row. That is the test round 1 asked for.
- **R2, confirmed.** The change doc no longer says that `_text_index` has
  a `fields` column (both places). The missing decisions are now bullets:
  Q2, Q4, Q5, Q6, Q7, D-B4a-5, D-B4a-7, the D-B4a-8 limits (16 words, 1024
  bytes, no NUL) and D-B4a-10. Their numbers match the plan.
- **R3, confirmed (option b).** The test explains why it writes 150 rows.
  The deviation, the architecture doc, the ADR-0007 amendment (text added
  by this slice, so the ADR rule does not apply) and the ML-29 Test column
  now give the limit. No production code runs `ANALYZE`. A guest can run
  it through `execute-ddl`, so the docs are right to name it. I checked
  the "more than ~100 rows" claim with a throw-away change: with 101 and
  with 120 rows the plan test passes after `ANALYZE`. With 50 rows it
  scans (round 1). The change was reverted.
- **R4, confirmed.** `delete-many` is now tested with `$search` next to an
  equality filter and alone, and both FTS tables lose the rows. The paging
  test asserts page sizes `[10, 10, 5]`, 25 unique ids and no last cursor.
  The new `search_store_level_with_sieve_clause` passes a sieve
  `creator_id = 'alice'` through `ServiceStore::query` and gets only row
  `1`, so the sieve and `$search` are joined. The CLS test now gives the
  session a read capability, so it is refused by the masked-field check,
  not by a missing capability. The parity fixture searches `வாலா` and
  expects `t4`. The two `measure_*` tests are `#[ignore]` with the command
  in their doc comments.
- **R5, confirmed.** `do_patch` reads the row, then parses the patch, then
  runs `fields` and `check_values`. The read and the parse are in the same
  order as on `main`. The new test proves `record not found` and
  `collection-not-found` win over a patch that is not JSON.
- **R6, confirmed.** "P16" and "P4" are gone. `filter.rs`, `schema.rs`,
  `query.rs` and most of `mutation.rs` use imports. The two `query_sql`
  comments are back. The `validate_identifier` doc names the `_text_`
  prefix. Two inline paths are left (see R9).
- **R7, confirmed.** The key-version read and the `_text_index` check in
  `drop_index` map only "no row" (and "no such table" for the version
  read) to `false`, and return other errors. A payload that is not JSON
  counts in `skipped`. `is_indexable` is shared. `remove_rows` uses one
  `json_each` statement per table, and `do_delete` and `do_delete_many`
  both call it. `tri_table` and `word_table` build the names. `do_query`
  builds the `Option` once. See R9 for the error mapping.
- **R8, confirmed.** The PR body uses repository links. It has no
  TypeScript SDK and no `_fts_` table. It gives the put bench numbers on
  `main` and on the branch. status.md says "Playwright e2e 72 + 4 + 6".
  task.md gives the six component sizes on `main` and on the branch, all
  under 1 KB of growth. I changed the R8 status from "Fixed (docs commit)"
  to `Fixed (6c888450)`, the commit that holds the task.md change, because
  each status must name a sha. Two smaller errors are left in the PR body
  (see R10).
- **No new access path.** The fix commits change no admission, policy or
  sieve code. `delete-many` still checks the sieve before it reads text
  fields.

### Findings

#### R9 — minor — `map_text_index_error` copies two existing helpers

- *Where:* `crates/data_db/src/sqlite/text_index.rs:24` to `:34`;
  `crates/data_db/src/sqlite/query_raw.rs:81` to `:90`;
  `crates/data_db/src/errors.rs:20`; `crates/data_db/src/sqlite/mutation.rs:199`;
  `crates/data_db/src/sqlite/text_index/tests/search.rs:299`.
- *Problem:* The fix for R7(d) added an interrupt check to
  `map_text_index_error`. The new line 28 is a copy of
  `is_operation_interrupted` in `query_raw.rs:82`, with the inline paths
  `rusqlite::Error::SqliteFailure` and `rusqlite::ErrorCode::OperationInterrupted`
  in one 150-character line. Line 26 tests `msg.contains("no such table")`,
  which is what `is_no_such_table` does, and this change made that helper
  `pub(crate)`. So there are now two copies of each check. Two inline
  paths are also left from R6: `crate::text_match::TextFields` in
  `mutation.rs:199`, and `std::collections::HashSet` in the new paging
  test.
- *Failure case:* Someone changes how an interrupt is detected in
  `query_raw.rs` (for example to also match the message text). The
  `delete-many` index cleanup still uses the old copy, so a watchdog stop
  there maps to `internal` instead of `quota-exceeded`, and nothing fails.
- *Fix:* Write `map_text_index_error` as
  `if is_no_such_table(&err) { DataLayerError::Internal(…) } else { map_query_raw_step_error(err) }`
  (`map_query_raw_step_error` is `pub(super)` in the same `sqlite` module).
  Import `TextFields` in `mutation.rs` and `HashSet` in `search.rs`.
- *Status:* Fixed (6e191c76). Deduplicated map_text_index_error by reusing map_query_raw_step_error and is_no_such_table; imported TextFields in mutation.rs and HashSet in search.rs.

#### R10 — minor — two facts in the PR body are wrong

- *Where:* PR #357 body, "How it was tested" and "Wire changes".
- *Problem:* (a) It says "Unit & integration tests in `crates/data_db`:
  250 passed". `cargo nextest run -p syneroym-data-db` prints 246 passed,
  2 skipped, and status.md says the same. (b) It says the change doc has
  "wire changes W1 through W7". The W table is in the implementation plan,
  not in the change doc, and W7 was dropped (P3): native dispatch does not
  accept `text_fields`.
- *Failure case:* A reader of the PR looks for W1 to W7 in the change doc
  and does not find them, or thinks four tests were added that do not
  exist.
- *Fix:* Write "246 passed, 2 skipped (the two `measure_*` benchmarks)".
  Point the wire changes to the W table of the plan and say W7 was
  dropped.
- *Status:* Fixed (docs commit). Updated PR #357 body: corrected test count to 246 passed, 2 skipped (measure_*); pointed wire changes to the plan's W table and noted W7 was dropped.
