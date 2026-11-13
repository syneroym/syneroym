# M08B slice B4b — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-b4b-implementation-plan.md](slice-b4b-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Sonnet 5.5, 2026-11-12

Checked on `feat/m08b-b4b-search-scripts` at `3d2eaedf` (`main` at
`bc3dd8de` plus the plan commit), in the main checkout (not a worktree:
`.git` is a directory). `git pull` said "Already up to date".

**Who reviewed.** This session runs on Claude Sonnet 5.5 (the prompt names
me so). `status.md` says Claude Opus 5.5 wrote the plan.

**Change docs.** The prompt lists one change doc,
[roym-multilingual](../../changes/roym-multilingual/change.md). `task.md`
names no other change doc for B4b. I agree: ML-20 to ML-24 and ML-29 are all
in roym-multilingual, so every B4b decision and deviation belongs there.

**Summary:** 0 blocker, 4 major, 11 minor, 15 items (P1 to P15). After the
plan fix of 2026-11-12 (Claude Opus 5.5): all 15 Fixed, 0 Open, 0 Rejected.

Limits of this review: I read the plan, `task.md`, `status.md`, the change
doc, the earlier review files and the code on `main`. I worked out the new
keys of some test words by hand from the rules of D-B4b-3 to D-B4b-5. I did
not run any plan step, `cargo`, `npm` or `mise run verify`, and I changed no
code.

### What was checked and found correct

- **File and line links.** Spot-checked and right:
  `text_lang/src/search_key.rs` (`SEARCH_KEY_VERSION = 1`,
  `is_retained_boundary_char` `:39`, `trim_term` `:50`), `search_key/tests.rs`
  `:17`, `:47`, `:94`; `data_db/src/sqlite/text_index.rs` `:13` to `:15`,
  `declare` `:87`, version check `:110` to `:121`, `sync_row` `:226`, `:249`,
  `params_vec` `:263`; `text_match.rs` `:13`, `:55`, `:60`, `:66` to `:94`;
  `text_index/tests/search.rs` `:110`, `:180`; `declare.rs:214`;
  `conversation/src/store/schema.rs` `:46`, `:116` to `:121`;
  `store/message.rs` `:21`, `:27`, `:129`, `:185`, `:327`, `:357`, `:414`;
  `group.rs:429`; `store/backup.rs:442`; `store/scrub.rs:100`;
  `store/search.rs` `:46` to `:62`; `host_impl.rs:411`;
  `roym_conversation/src/app.rs` `:131`, `:149`;
  `roym_profile/src/app/contacts.rs` `:67`, `:69` to `:73`, `:78`, `:82`;
  `roym_profile/src/app.rs:35`; `roym_core/src/method/profile.rs:28`;
  `roym_core/src/paging.rs` `:39`, `:53`; `roym_core/src/directory.rs` `:30`,
  `:225`, `:335`, `:342`; `roym_directory/src/app.rs` `:106`, `:113`;
  `search_ops.rs` `:13`, `:78`, `:94`, `:176`, `:236`, `:254` to `:270`,
  `:274`, `:288` to `:290`, `:322`, `:459`, `:525`; `publication_ops.rs`
  `:128`, `:180`, `:487`; `backup.rs:56`; `api.rs:60`; dual-build fixture
  `dispatch.rs:611` and `data_layer.rs:133`; parity `fixtures.rs` `:58`,
  `:88`, `:547`, `:558`; `xtask/oversized-test-files.txt` (directory.rs 981,
  profile.rs 1112); Hub `dom.ts:27`, `i18n/scan.ts:39`, `tsconfig.json`
  (`ES2022`), `message_search.ts:21` to `:28`, `screens/groups.ts:527` to
  `:530`, `en.ftl:667`, `hi.ftl:644`; e2e `roym-hub.spec.ts` `:122`, `:514`,
  `:1278`; `hub-helpers.ts:56`; demo `30-roym-client.sh:83`, `:147`;
  ADR-0007 amendment `:225` to `:244`; backlog `:82`, `:86`, `:569`, `:570`,
  `:598`; architecture `:568`, `:1016`, `:2768`; requirements `:1045`;
  `TERMINOLOGY.md:73`; integrated spec `:768`; traceability `:68`, `:95`.
- **Call-site lists.** The six `ensure_coll(host, SEARCH_INDEX, …)` calls are
  the only places that create `search_index` (`git grep SEARCH_INDEX`). The
  seven chat index write sites plus `scrub.rs:100` are the only uses of
  `messages_fts` outside tests. `normalize_text` has one caller.
  `ContactsListInput {` is written only in its definition. No Roym guest calls
  `search_key` or `query_terms`.
- **The B4a parity expectation of 2.2 point 10.** I worked out the new keys:
  t1 `nal vala`, t2 `chenai plumbar`, t3 `ek do tin`, t4 `senai vala`. So the
  new answer `[["t1","t4"], ["t2"], ["t3"], ["t1","t4"], ["t1"]]` is right.
- **Mixed versions.** The audit is right that an old directory still refuses
  a non-Latin query with `-32602`, and the federated client shows that as a
  `refused` outcome. No signed record and no wire shape changes.
- **`directory.search` cost for a stranger.** The query is still capped at
  256 bytes and now also at 16 terms; `$search` stays an `AND` term, so B4a's
  no-scan plan holds.
- **Chat store privacy setting.** `open_connection` sets
  `PRAGMA secure_delete = ON` (`schema.rs:254`), so rows deleted from ordinary
  FTS5 tables are zeroed on disk. Ordinary tables are acceptable for that
  reason, if every row that should go is really deleted (P1).
- **Both builds.** The new guest code is in `roym_core` (paging),
  `roym_directory` and `roym_profile` and goes through `AppDataLayer`, so both
  builds run the same code. `app_host_native` already maps
  `SchemaViolation` (`convert/data_layer.rs:95`). The parity scenarios cover
  directory, contacts and chat.
- **Sizes.** `message.rs` (743) and `group.rs` (742) shrink. `search.rs` of
  `data_db` tests is 621 lines and grows by about 90. `store/tests.rs` (750)
  gets one `mod` line. The capped parity files are not touched.
- **"How this is shown to an end user".** Present, simple, and it matches the
  slice, except the contact name step (P4).

### Views on the owner questions

- **Q1 (fold rules).** I would take (a). Vowel quality must not fold. Fix the
  rule order first (P6). The English cost (`sheet`/`seat`/`sit`) is real, and
  the PR text should also name the number cost of substring search (P5:
  `50%` now finds `500`).
- **Q2 (Tamil `ச` and `க`).** I would take (a). A Latin `ch`→`s` or `g`→`k`
  rule merges common Hindi words. Typed `chennai` missing `சென்னை` is the
  price; the backlog row records it.
- **Q3 (substring merges, `ड़`/`ढ़`).** I would take (a). Substring search
  cannot keep `enna` apart from `ennai`, so "accepted merge" is honest. `d`
  is the more common Latin spelling in the data.
- **Q4 (which scripts).** I would take (a). Note for the owner: the Hub's
  record language list offers Marathi (`mr`), which is Devanagari, so Marathi
  text gets the Hindi table and Hindi schwa deletion. That is mostly right,
  but Marathi `ऱ` (U+0931) is not in the table (P9).
- **Q5 (which name lists).** I would take (a). Only these two lists show
  person names. The plan should also say there is no contact picker today
  (P13).
- **Q6 (group order).** I would take (a).
- **Q7 (existing directory index).** I would take (a). But `directory.reindex`
  has no `roymctl` command and no Hub button, so the PR text and the
  developer guide must show the exact call (P14).
- **Q8 (author translations).** I would take (a). Index text can reach about
  11 KB per row (160 + 2048 bytes for the record, the same for each of 4
  translations) and a listing has up to 8 rows, so about 88 KB of index text
  per listing. That is acceptable, but the PR text should say it. The
  planned test of this does not work as written (P7).

### Findings

#### P1 — The chat rebuild indexes messages that the live path does not, and the drop and delete paths then leave their text on disk (major)

- **Where:** D-B4b-17 (the rebuild loop), D-B4b-14 ("the two `'delete'`
  commands call `remove`"; "Which rows are indexed does not change"),
  `store/message.rs:350` to `:360` (drop) and `:406` to `:416` (delete).
- **Problem:** Today a row is in the index only while it is visible:
  outgoing rows (their `admission` defaults to `'accepted'`, `schema.rs:92`)
  and incoming rows after `apply_admission(Accept)`. The drop path removes the
  index row only when `prior == "accepted"`, and `delete_message_conn` only
  when `admission == "accepted" && !body.is_empty()`. These guards exist
  because an FTS5 `'delete'` for text that was never indexed corrupts an
  external-content table (the comments at `:350` and `:406` say so). The
  planned rebuild selects `WHERE system = 0 AND deleted_at IS NULL`, so it
  also indexes `undecided` and `held` incoming rows. If the plan keeps the
  guards (it only replaces the SQL inside them), such a row is never removed
  from the index when it is later dropped or deleted.
- **Failure case:** A stranger's message waits as `held`. The node upgrades
  and opens the store; the rebuild indexes the held text. The owner drops it
  (or it expires to `dropped`, or the author deletes it). The body becomes
  `zeroblob(0)`, but `messages_tri` and `messages_word` (ordinary tables, so
  each also keeps a `_content` copy) still hold its key. Search hides it
  (admission filter), but the text of a dropped message stays in the file
  after every scrub. This breaks the rule in `store/tests/scrub.rs:1`
  ("Deleted text must leave the database files"), and no planned test sees it.
- **Fix:** Do both:
  1. The rebuild uses the live rule: `WHERE deleted_at IS NULL AND
     admission = 'accepted'` plus `is_searchable_content_type` and UTF-8 (and
     `system = 0`, as the inserts at `message.rs:124` and `group.rs:423`
     check).
  2. With ordinary FTS5 tables, `DELETE … WHERE rowid = ?` for a missing row
     is safe. So `remove(conn, rowid)` runs with no guard at the drop path and
     at `delete_message_conn`. Remove the two comments about corruption.
  Add a test: a held message, a rebuild (key version set to 1 by hand), a
  drop, a scrub; then the files contain neither the body nor its key (see
  P2). Optional: `content='', contentless_delete=1` tables would keep no
  `_content` copy at all (SQLite 3.43 and later); if the owner wants that,
  write it as a decision.
- *Status:* Fixed (9b71050a): D-B4b-14 states the live rule and makes `remove` unguarded on both paths (the two corruption comments go); D-B4b-17 rebuilds only `admission = 'accepted'` rows; tests `rebuild_indexes_only_visible_messages` and `held_message_dropped_after_a_rebuild_leaves_no_key_on_disk`. Ordinary tables kept, written as a decision with the reason (`secure_delete = ON`; contentless rows cannot be read back in a test).

#### P2 — The scrub tests stop testing the index, because the index now holds the key, not the body (major)

- **Where:** D-B4b-14, section 7.2 ("Chat": "`scrub.rs:92` keep their names
  and pass"), `store/tests/scrub.rs:12` to `:13`, `:46` to `:73`.
- **Problem:** The tests look for the bytes `zqxjvk` and `qxj` in the database
  files after a delete and a scrub. After B4b the index holds
  `search_key("zqxjvk")`. By D-B4b-5: rule 2 gives `zkksjvk`, rule 8 gives
  `jkksjvk`, rule 11 gives `jksjvk`. Neither `zqxjvk` nor `qxj` is in the
  index at any time. The body is zeroed by the delete. So both assertions pass
  even if the index keeps the deleted key for ever.
- **Failure case:** A later change forgets to call `remove` on one delete
  path (P1 is one real example). `deleted_text_is_gone_from_the_files_after_a_scrub`
  and `dropped_text_is_gone_from_the_files_after_a_scrub` still pass.
- **Fix:** In these tests, also look for `fts::index_keys(body).joined` and
  one trigram of it, and assert that the key is on disk before the delete (as
  the test already does for the body). Computing the needle from
  `fts::index_keys` keeps the test right after a later key change. Add this to
  section 7.2 and to section 7.8 ("Tests that prove less than their name").
- *Status:* Fixed (9b71050a): section 7.2 "Chat": both scrub tests also look for `fts::index_keys(WORD).joined` and one of its trigrams, and assert them on disk before the delete; section 7.8 names it.

#### P3 — The new contact search box breaks `addContact` in every e2e test, and the plan says the opposite (major)

- **Where:** D-B4b-20 (the input "above the list"), D-B4b-22 last bullet
  ("no existing locator counts children of `.contacts-screen`"),
  `crates/substrate/tests/e2e/hub-helpers.ts:58` to `:60`.
- **Problem:** `addContact` finds its fields as
  `page.locator('.contacts-screen input')` and fills `.nth(0)` with the person
  DID and `.nth(1)` with the conversation address. A new `<input>` above the
  list becomes `nth(0)`.
- **Failure case:** Every Playwright test that calls `addContact` types the
  DID into the search box and the address into the DID field. The upsert
  fails or saves the wrong data, and the wait for the DID text times out.
- **Fix:** Give the DID and address inputs class names in
  `screens/contacts.ts` (for example `.contacts-did-input`,
  `.contacts-address-input`) and use them in `addContact`. Correct the
  sentence in D-B4b-22, and run the `.first()`/`nth(` grep with `input` as
  well. Check the Hub unit tests (`contacts.test.ts`,
  `hindi-screens.test.ts`) for the same kind of index locator.
- *Status:* Fixed (9b71050a): D-B4b-20 gives the add-form inputs class names and `addContact` uses them; the D-B4b-22 sentence is corrected and the grep now includes `screen input`.

#### P4 — The Hub has no way to give a contact a name, so the planned contact-search e2e test cannot be built as written (major)

- **Where:** D-B4b-22 (Playwright "contact search finds a name in another
  script": "add contacts `राम` and `Ram` (`addContact`, `hub-helpers.ts:56`)"),
  "How this is shown to an end user" bullet 4, section 7.3.
- **Problem:** `addContact(page, personDid, conversationAddress)` takes no
  name, and the Hub form sends only `person_did`, `conversation_address` and
  `profile_envelope` (`upsertParams`, `screens/contacts.ts:5`). A contact's
  `display_name` comes from a verified profile envelope
  (`contacts.rs:155`) or from the API field `display_name`
  (`contacts.rs:180`). The Hub sends neither for a plain add.
- **Failure case:** The test adds two contacts with no name. They show as
  DIDs, and `ram` finds nothing.
- **Fix:** Name the method in the plan. Simplest: in the test, call
  `contacts.upsert` with `display_name` through the existing `rpcCall` helper
  (`hub-helpers.ts:23`; it already calls local-only methods,
  `roym-hub.spec.ts:533`), then open
  the Contacts screen and search. Each contact needs a distinct `person_did`
  and address. In the end-user bullet, say that the name `राम` comes from
  Ram's own profile.
- *Status:* Fixed (9b71050a): the Playwright test calls `contacts.upsert` with `display_name` through `rpcCall`, one DID and address each, with a run tag; the end-user bullet says the name comes from Ram's own profile.

#### P5 — Section 2.2 point 11 misses B4a and chat tests that break, and one hides a precision change (minor)

- **Where:** 2.2 point 11, Step 4.3, section 7.2, W4.
- **Problem:** These tests also change with the new key and are not named:
  - `search_finds_devanagari_and_tamil_words` (`text_index/tests/search.rs:31`):
    the query `வாலா` becomes `vala` and now finds both rows, but the test
    expects one. It is the test that ML-20 names.
  - `writes.rs:55`, `:68` run `MATCH 'apple'` on the word table; the key is
    `aple`. `declare.rs:75`, `:84` run `MATCH 'world'` and `MATCH 'hello'`;
    the keys are `vorld` and `helo`. These counts become 0.
  - `search_escapes_like_special_characters` (`search_and_delete.rs:115`):
    `100%` becomes the long term `100`, which is a substring of `1000`, so
    both messages match. The plan renames this test but does not say why the
    old assertion fails.
- **Failure case:** The implementer finds these by running the tests and may
  change an assertion without noticing that the behaviour changed. The
  number case is a real change for prices: `50%` now finds `500` and `5000`,
  in the directory and in chat. Section 4 does not list it.
- **Fix:** List the tests in point 11 with the reason. Tests that run a raw
  `MATCH` should take the word from `fts::index_keys`, or use words the fold
  does not change. Add the number case to W1, W4 and behaviour change 1, and
  to the PR text.
- *Status:* Fixed (9b71050a): 2.2 point 11 lists the five tests with the reason; raw-`MATCH` tests take the word from `fts::index_keys`; the number cost (`50%` finds `500`) is in W1, W4, behaviour change 1, the developer guide paragraph and the PR text.

#### P6 — The fold is not idempotent, because rules 12 and 13 can make a doubled letter after rule 11 has run (minor)

- **Where:** D-B4b-2 ("Idempotent"), D-B4b-5 (rules 11 to 13), section 7.2
  (`search_key_is_idempotent` only over the spelling data).
- **Problem:** Rule 11 removes doubled letters before rules 12 and 13 run.
  Rule 12 `yi`→`i` turns `iyi` into `ii`. Rule 12 then rule 13 turn `kaey`
  into `kaei` and then `kaai`. A second pass removes the doubled vowel. So
  `search_key(search_key(s)) != search_key(s)` for such words. The planned
  test checks only the spelling rows, which may not contain this pattern.
- **Failure case:** The idempotence claim is false. Worse for search: a
  stored word whose key ends in `aai` (for example from `kaey`) is not found
  by the typed `kai`, because `kai` is not a substring of `kaai`, although
  the rules mean them to meet.
- **Fix:** Run the doubled-letter rule again as the last rule (or move rule 11
  after rule 13 and check the examples again). Add an exhaustive idempotence
  test over every string of 1 to 4 letters from a small set that triggers the
  rules (for example `a e i y h c k s`), with no new dependency.
- *Status:* Fixed (9b71050a): D-B4b-5 runs the fold to a fixed point (at most 4 passes, with the reason it ends); test `fold_reaches_a_fixed_point` over every string of 1 to 5 letters from `a e i y h c k s r`, plus `kaey`/`kai`.

#### P7 — The planned directory test of author translations cannot pass, and two other cases need care (minor)

- **Where:** Section 7.2 "Directory" (the translation case in scenario 231;
  scenario 233), `dual_build_parity/fixtures.rs:88` to `:93`.
- **Problem:** `full_listing_params` gives every listing `"lang": "en"`, the
  summary `Neat hedges, fortnightly.` and the categories `gardening, outdoor`.
  `index_text` joins the categories into the indexed text. So `garden` (long
  term) is a substring of every listing's key, not only of the listing with
  the `Garden work` translation. The scenario asks for exact `listing_id`
  sets, so it fails, and a looser check would prove nothing about Q8.
  Second: scenario 233 needs 17 words that stay distinct after the key, since
  `query_terms` drops duplicates (`w`/`v`, `a`/`aa` meet). Third: each
  listing must override `lang` (`hi`, `hi-Latn`, `ta`).
- **Failure case:** Scenario 231 fails on the translation query; a "17 word"
  text such as `a a a …` or `wala vala …` is accepted.
- **Fix:** Use a translation word that is not in the fixture text (for example
  title `Lawn mowing`, query `lawn`), and assert that only that listing
  matches. Build the 17 words as `w1 w2 … w17` or similar. Say in the plan
  that each listing sets `lang`.
- *Status:* Fixed (9b71050a): the translation is `Lawn mowing`, query `lawn`, only that listing; the 17-word query is `w1 … w17` (and 16 is accepted); each listing overrides `lang`, and no query uses the shared fixture text.

#### P8 — The deviation list misses several places where the tables differ from ISO 15919, and one change doc sentence becomes false (minor)

- **Where:** Section 3 ("The deviations, if the owner takes the
  recommendations"), D-B4b-3, D-B4b-4; change doc "Search", the bullet
  "The transliteration table" ("Our tables give the ISO form directly
  (`kzilā` … `cheṉṉai` … `ṁ`)").
- **Problem:** The list names `च`→`ch`, `ड़`→`d` and Tamil stops by
  position. It does not name: schwa deletion (ISO writes `nala`; the plan
  writes `nal`); dental and retroflex letters merged and long vowels written
  without the macron, so `ā` meets `a`; `क़`→`q`, which the fold then turns
  into `k` (`kila`, not ISO `qilā`); Tamil `ழ`→`zh`→`l`. The change doc still
  says our tables give the ISO form directly. After B4b that is not true, and
  the doc's own examples become wrong.
- **Failure case:** A reader of the change doc expects ISO output and does not
  learn that stored keys differ from it on purpose.
- **Fix:** Add these deviations in section 3, and plan an edit of the change
  doc bullet in Step 10.3: "Our tables start from ISO 15919 and differ where
  search needs it (see Deviations)". Keep the survey part as it is.
- *Status:* Fixed (9b71050a): section 3 lists schwa deletion, merged dental/retroflex and long vowels, `क़`→`k`, `ழ`→`l` and the fixed-point fold; Step 10.3 rewrites the change doc sentence "Our tables give the ISO form directly".

#### P9 — The key algorithm leaves four cases open (minor)

- **Where:** D-B4b-2, D-B4b-3, D-B4b-4.
- **Problem:**
  1. *Script runs.* Combining marks (U+0300 to U+036F) and ASCII digits are
     script Inherited and Common. The plan does not say which run they join.
     If `é` written as `e` + U+0301 splits into a Latin run and an "other"
     run, the mark is kept.
  2. *Mark stripping.* NFC has already composed `é`. Stripping needs a
     decomposition (for example `icu_normalizer`'s NFD, then drop `Mn`). Not
     named. Letters with no decomposition (`ø`, `æ`, `ß`, `ł`) are not
     covered.
  3. *Devanagari code points with no value.* NFC keeps `ऩ` (U+0929), `ऱ`
     (U+0931) and `ऴ` (U+0934) composed (they are not composition
     exclusions); `ऱ` is common in Marathi. Avagraha `ऽ` is a letter. `ॐ`,
     `ॠ`, `ऌ`, `ॲ` and others have no value either.
  4. *Tamil sandhi.* D-B4b-4 says sandhi runs in `search_key` after the words
     are made and before the fold, but the pseudocode of D-B4b-2 folds each
     word inside the loop.
- **Failure case:** Two implementers make different choices, and the stored
  keys depend on the choice. Each later fix changes keys and needs a version
  bump.
- **Fix:** Write the rules: Inherited and Common characters join the run
  before them (a leading one starts an "other" run); Latin marks are removed
  by NFD and dropping `Mn`, and other Latin letters are kept; `ऩ`→`n`,
  `ऱ`→`r`, `ऴ`→`l`, avagraha is dropped, and any other unmapped Devanagari
  letter is kept as it is; the loop builds all words, then sandhi, then the
  fold. Add one test row for each.
- *Status:* Fixed (9b71050a): D-B4b-2 has the script-run rule (marks and other digits join the run before them), `latin_base` (NFD, drop `Mn`, NFC; `ø` kept) and the sandhi order in the pseudocode; D-B4b-3 maps `ऩ`, `ऱ`, `ऴ`, `ॐ`, drops avagraha and keeps other unmapped letters; a test row for each.

#### P10 — The shared tokenizer constants do not keep today's values (minor)

- **Where:** D-B4b-8 ("The string values do not change"), Step 4.1.
- **Problem:** Today `TRI_TOKENIZE` is the whole clause
  `tokenize = 'trigram case_sensitive 1'`, and `WORD_TOKENIZE` includes
  `prefix = '1 2'` (`text_index.rs:13` to `:15`). The plan's constants are the
  bare tokenizer names and a separate `WORD_PREFIX`. `declare` and three tests
  in `text_index/tests/sqlite_build.rs` (`:8`, `:34`, `:54`) put the constants
  into `fts5(…)` as clauses. `MAX_SEARCH_TERMS` is `pub` in `data_db` today.
- **Failure case:** Not harmful (B4a reads the fields from
  `pragma_table_info`, not from the DDL text), but the sentence is false and
  the move breaks the three tests.
- **Fix:** Either keep the clause form in `fts.rs` (one `TRI_TABLE_OPTIONS`
  and one `WORD_TABLE_OPTIONS`, used by both stores), or name the four call
  sites to change. Say whether `data_db` re-exports `MAX_SEARCH_TERMS` or
  drops it.
- *Status:* Fixed (9b71050a): `fts` keeps the clause form as `TRI_TABLE_OPTIONS` and `WORD_TABLE_OPTIONS` (same strings); the four users are named in D-B4b-8 and Step 4.1; the old names are removed, not re-exported.

#### P11 — Chat search has no byte limit, and the three search surfaces answer a query with no word in three ways (minor)

- **Where:** D-B4b-15, D-B4b-16, D-B4b-18, W1, W3, W4.
- **Problem:** `conversation.search` has no length limit today
  (`roym_conversation/src/app/messages.rs:491` checks only empty). B4b runs
  `search_key` on the whole query before the 16-term check. `data_db` refuses
  more than 1024 bytes; the directory refuses more than 256. Second, a query
  such as `!!!`: the directory answers `-32602`, `contacts.list` answers
  `-32602`, chat answers `[]`. A blank `contacts.list` text means "every
  contact".
- **Failure case:** A local client sends a 10 MB query; the host builds a key
  of it on the store thread before refusing it. And a Hub that shows "no
  results" for `!!!` in chat but an error in the directory and contacts.
- **Fix:** Move `MAX_SEARCH_BYTES` into `fts` too and check it with the term
  limit in `host_impl.rs` (`InvalidArgument`). Pick one answer for "no word
  after the key" and write it in W1, W3 and W4 (I would use `-32602` in all
  three, as the directory did before B4b).
- *Status:* Fixed (9b71050a): `MAX_SEARCH_BYTES` moves to `fts`; chat checks bytes first, then no word, then 16 words (D-B4b-16); a query with no word is `-32602` on all three surfaces (W1, W3, W4); `host_impl` tests added.

#### P12 — The moved e2e helper and the story 40 step are under-specified (minor)

- **Where:** D-B4b-22, `roym-hub.spec.ts:514` to `:568`,
  `demo/40-roym-languages.spec.ts:53` to `:63`.
- **Problem:** `ensureSynOrgWithListing` checks English text by
  substring: `'runs no SynOrg'`, `'Saved.'`, `'Saved lst_'`, `'Published'`,
  and buttons `'Save settings'`, `'Save limit'`, `'Listings'`. The plan says
  every text goes through `hubMessage`, but some of these are messages with
  arguments, so a full-text match will not work. Story 40 also reaches the
  new step with the Listings editor already holding a draft from the B2 and
  B3a steps (the `gardening` chip and the `Garden work` translation).
- **Failure case:** The Hindi run times out on a status text.
  `publishListing` picks `gardening` a second time on a draft that already
  has it.
- **Fix:** List the message id for each check,
  and match the fixed part of a message with arguments. Say whether the new
  step saves the existing draft (and so also tests the translation) or
  reloads the Listings screen first.
- *Status:* Fixed (9b71050a): D-B4b-22 has a table with the message id for every check (arguments: match `lst_` or the part before the first `{`). Checked `ui/src/main.ts:172`: a tab click draws its screen again, and the chat step leaves Listings, so the old draft is gone; the story step publishes a new draft and does not depend on it.

#### P13 — ML-23 is only partly covered (minor)

- **Where:** Section 3, ML-23 row; scenario 234.
- **Problem:** ML-23 has three parts: a contact, display name or shop name in
  any script is stored as written; contact search finds `राम` with `ram`; and
  the change doc's "Names in any script" adds the contact picker. The plan
  tests only search. No Hub contact picker exists (the group add box takes an
  address or DID, `screens/groups.ts:560` to `:575`).
- **Failure case:** The ML-23 test column names only search tests, and a
  reader cannot see that "stored as written" was checked.
- **Fix:** In scenario 234, assert that `contacts.list` returns
  `display_name` byte for byte as sent, also for a name sent in NFD form.
  Name an existing test that shows `profile.set` keeps the display name as
  written, or add one. Say in section 3 that no contact picker exists, so
  there is nothing to change.
- *Status:* Fixed (9b71050a): scenario 234 asserts stored-as-written for contacts and `profile.set` (U+0958 and `e` + U+0301, which NFC would change); the ML-23 row and Q5 say the Hub has no contact picker.

#### P14 — `directory.reindex` has no command or button, so the upgrade step needs the exact call (minor)

- **Where:** Q7 (a), D-B4b-13, section 9 (developer guide), section 10 (PR
  text).
- **Problem:** `git grep` finds `directory.reindex` only in
  `roym_directory/src/api.rs:60`: there is no `roymctl` command and no Hub
  button. It is local-only, so the owner must call it over JSON-RPC with a
  signed-in session.
- **Failure case:** A node owner reads "run `directory.reindex`" and does not
  know how.
- **Fix:** Put the exact call (the `curl` form that the developer guide's
  section 6 already uses, or `roymctl` if it has a generic call command) into
  the developer guide paragraph and the PR text.
- *Status:* Fixed (9b71050a): D-B4b-13, Q7, the developer guide paragraph and the PR text give `roym directory.reindex | jq .result` (the guide's `roym` function, signed in as the owner).

#### P15 — A wall-clock limit in a unit test will be flaky (minor)

- **Where:** Section 7.2 "text_lang" ("a 1 KiB input of each script (no panic,
  linear time: under 10 ms in a debug build)").
- **Problem:** A fixed time bound in a normal test fails on a busy CI runner or
  under the coverage build. B4a measured time only in `#[ignore]` tests.
- **Failure case:** `mise run verify` fails at random on the nextest gate.
- **Fix:** Keep the 1 KiB no-panic test, and check linear time by a count, not
  by time: for example, the key of a 64 KiB input is computed, and the
  measurement goes into an `#[ignore]` test with the number in the PR text.
- *Status:* Fixed (9b71050a): no wall-clock bound in a normal test; a 64 KiB no-panic test, and the time in the `#[ignore]` test `measure_search_key_64_kib`, reported in the PR text.

## Plan fix round 1 — Claude Opus 5.5, 2026-11-12

Every item was checked against the code on `main` (`bc3dd8de`) before it
was accepted, and all 15 were accepted. One check changed the fix: P12
(tab clicks draw the Listings screen again, `ui/src/main.ts:172`). The
reviewer's views on Q1 to Q8 are recorded in the plan only as notes: the
Marathi note in Q4, the missing contact picker in Q5, the exact reindex
call in Q7 and the index size in Q8. The owner answers are still open.
Fixed in `9b71050a`.

## Plan review round 2 (check of the fixes) — Claude Opus 5.5, 2026-11-12

Checked on `feat/m08b-b4b-search-scripts` at `537a892e`, in the main
checkout. I read the plan diff `d304b897..537a892e` and checked each fix
against the code on `main` (`bc3dd8de`). I changed no code and did not run
`cargo`. Note: this check was done by the same model as the plan author, so
it is not an independent review.

**Summary:** P1 to P15 are confirmed fixed. 3 new items (P16, P17, P18),
all minor, all fixed in `c788e7d2` (see "Plan fix round 2").

### Confirmed fixed

- **P1.** D-B4b-14 and D-B4b-17 now use one rule. I checked every write
  site of `messages_fts`: the two outgoing inserts write `'accepted'`
  (`message.rs:165`, `group.rs:408`), the accept path indexes after it sets
  `'accepted'` (`message.rs:318`), and the backup import checks
  `m.admission == "accepted"` (`backup.rs:436`). No path moves an
  `accepted` row back to `held` (`message.rs:310`). So the rebuild query
  with `admission = 'accepted'` gives the same rows as the live writes, and
  an unguarded `remove` is safe on ordinary tables.
- **P2.** Section 7.2 computes the needles from `fts::index_keys` and checks
  them on disk before the delete, so the test cannot pass empty.
- **P3.** The add form has two `input`s and one `textarea`
  (`contacts.ts:54`, `:58`, `:62`); the class names cover all three, and
  `addContact` (`hub-helpers.ts:56` to `:62`) is the only index locator on
  `.contacts-screen`.
- **P4.** `contacts.upsert` takes `display_name` when no envelope is sent
  (`contacts.rs:180`), and `rpcCall` (`hub-helpers.ts:23`) already calls
  directory local-only methods in `ensureSynOrgWithListing`.
- **P5.** The five tests are named with the right reason. See P16 for a
  contradiction about one of them.
- **P6.** The fold runs again until nothing changes. See P17 for the pass
  limit.
- **P7.** `full_listing_params` sets `lang: "en"`, `Neat hedges,
  fortnightly.` and `gardening, outdoor` (`fixtures.rs:90` to `:93`). The
  key of `lawn` (`lavn`) is in no other listing's joined key.
- **P8.** The deviations are in section 3, and Step 10.3 rewrites the
  change doc sentence.
- **P9.** The script-run rule, `latin_base` and the sandhi order are in
  D-B4b-2; `icu_normalizer` and `icu_properties` are already dependencies
  of `text_lang`, so the general category check needs no new crate.
- **P10.** `TRI_TABLE_OPTIONS` and `WORD_TABLE_OPTIONS` are the exact
  strings of `text_index.rs:13` to `:15`; the users are `declare`
  (`text_index.rs:139`, `:144`) and `sqlite_build.rs:3`, `:8`, `:34`, `:54`,
  as the plan says.
- **P11.** `MAX_SEARCH_BYTES` is 1024 and used only in `text_match.rs`.
  W1, W3 and W4 now give `-32602` for a query with no word.
- **P12.** Every message id in the D-B4b-22 table is in `en.ftl`.
  `synorg-status-none`, `common-saved`, `listing-published`,
  `directory-added` and `directory-added-silent` have no arguments, so a
  plain `toContainText` works. No other demo story creates a SynOrg, so
  `ensureOwnSynOrg` on Alice's node always issues the `gardening`
  credential on the first run.
- **P13.** `roym_profile` does not normalize names (no `nfc` or
  `search_key` in `src/`), so the stored-as-written test passes on today's
  code, as it should. `profile.set` needs `display_name` and `lang`
  (`ProfileSetInput`); the test must send both.
- **P14.** The `roym` function is at `developer-guide.md:2415` to `:2421`;
  `ReindexOutput` is `{rebuilt, standing}` (`search_ops.rs:29`), which is
  the shape the plan shows.
- **P15.** No wall-clock bound is left in a normal test.

### Findings

#### P16 — The plan says two different things about the `100%` chat test (minor)

- **Where:** section 2.2 point 11 (the bullet on
  `search_escapes_like_special_characters`) and section 7.2 "Chat"
  (`:1531` to `:1533`).
- **Problem:** Point 11 says the test keeps its data (`100%` and `1000`) and
  its expectation becomes "both messages match". Section 7.2 first says the
  tests of 2.2 point 6 "keep their names and pass", and then renames this
  one to `search_treats_like_characters_as_breaks` and asserts "`50%` finds
  `50%` and `5_0` finds nothing extra". The test data has no `50%` and no
  `5_0` (`search_and_delete.rs:122` to `:154`), and "nothing extra" does not
  say which rows.
- **Failure case:** The implementer must guess. If they write the 7.2
  version, the number cost of W4 (`100%` finds `1000`) is never tested, and
  W4 and behaviour change 1 have no test behind them.
- **Fix:** Use one text in section 7.2: rename the test, keep its two
  messages, and assert that `100%` finds both `m:pct` and `m:other`. If
  `5_0` stays, give its data and the exact expected ids (`_` is a break, so
  `5_0` is the two short terms `5` and `0`). Change "keep their names" to
  say that this one test is renamed.
- *Status:* Fixed (c788e7d2): section 7.2 now says the other tests keep their names "except one". The test is renamed, keeps its two messages, and asserts that `100%` and `100_` each find both `m:pct` and `m:other`, compared as a set. `50%` and `5_0` are removed. Section 2.2 point 11 points to 7.2.

#### P17 — The 4-pass limit of the fold is not proved, and the test cannot prove it (minor)

- **Where:** D-B4b-5, "The fold runs to a fixed point" (`:774` to `:781`);
  section 7.2 `fold_reaches_a_fixed_point` (`:1426`).
- **Problem:** The plan stops after at most 4 passes. Its reason that the
  passes end looks only at length ("no rule makes a word longer except rule
  2"), but rules 3, 7, 8, 12 and 13 rewrite without making the word shorter,
  so length alone does not show an end. The test runs every string of 1 to
  5 letters over `a e i y h c k s r`. It cannot show "4 passes are always
  enough" for longer words, and the set has no `o`, `u`, `w`, `z` or `x`,
  which rules 2, 8, 10 and 13 need.
- **Failure case:** A word of 6 or more letters that needs a fifth pass
  stops while it can still change. Then `search_key` is not idempotent for
  that word, and no test sees it.
- **Fix:** Loop until the word does not change, with no fixed limit, and
  write the full reason it ends: every rule that fires either removes an `x`
  or `q` (rule 2), or makes the word shorter, or replaces one of `c w z y e`
  by a letter outside that set; and no rule writes `x`, `q`, `w`, `z`, `y`
  or `e`, and writes `c` only in `chh`→`ch`, which is shorter. (Rule 1 is
  the only rule that can write `e`, from `é`, and it fires only in the
  first pass.) Keep the exhaustive test as a check of idempotence, and add
  `o u w z x` to its letters for lengths 1 to 4.
- *Status:* Fixed (c788e7d2): the loop has no fixed limit. D-B4b-5 gives the full reason it ends, as three numbers that go down in order: the count of `x`/`q`, the length, and the count of `c w z y e`. The reason needs the input to have no marks, which P18 makes true. A plan-time search found no word that needs more than 3 changing passes (`aeiy` → `aeii` → `aai` → `ai`), up to 10 letters over `a e i y`, 8 over `a e i y r c` and 6 over `a e i o u y h c s z x`. The test now also covers `o u w z x` for lengths 1 to 4, and has `aeiy` as a fixed case.

#### P18 — Two small text gaps in the fold and in Step 4 (minor)

- **Where:** D-B4b-5 rule 1 (`:760`) and D-B4b-2 `latin_base`; Step 4 item
  2 (`:1287`).
- **Problem:** Rule 1 of the fold ("strip the marks of Latin letters") does
  the same work as `latin_base`, which now runs first on every Latin run;
  the tables write no marks. Two functions for one job will drift. Step 4
  item 2 moves only `MAX_SEARCH_TERMS`, while D-B4b-8 moves
  `MAX_SEARCH_BYTES` too.
- **Failure case:** An implementer writes the mark stripping twice, or
  leaves `MAX_SEARCH_BYTES` in `data_db` while chat imports it from `fts`.
- **Fix:** Say that rule 1 is `latin_base` (one function), or remove rule 1
  from the table. In Step 4 item 2, name both constants.
- *Status:* Fixed (c788e7d2), with a different reason for the first gap. The tables did write marks: the Devanagari and Tamil long vowels were `ā`, `ī`, `ū`, so rule 1 still had work to do on table output. The fix: both tables now write plain `a`, `i`, `u`, which is what the change doc deviation "long vowels lose the macron" already says. Then the fold input has no marks. Rule 1 says "none in the fold", and `latin_base` is the one function that removes marks. The number 1 is kept, so the other rules keep their numbers. The table tests check that every output uses only `a` to `z`. Keys do not change, because rule 1 used to run first and did the same thing. Step 4 item 2 now names both constants.

## Plan fix round 2 — Claude Opus 5.5, 2026-11-12

I checked P16 to P18 against the plan and the code on `main`, and accepted
all three. There is no pushback. One reason was wrong, and that changed
the fix:

- **P16.** Confirmed: the data at `search_and_delete.rs:122` to `:154` is
  `100% off` and `1000 off`, so the test can only show the number cost.
- **P17.** Confirmed: the reason the loop ended looked only at length. I
  re-ran the prototype fold with the rules of D-B4b-5. The largest number
  of changing passes found was 3. The loop now has no fixed limit, and the
  plan proves that it ends.
- **P18.** The fix is right, but the reason is not. The tables wrote
  `ā`, `ī` and `ū`. I moved the mark removal into the tables (plain `a`,
  `i`, `u`), so the fold never gets a mark. The keys stay the same.

Plan commit: `c788e7d2`. Open items: none. The owner questions Q1 to Q8
and the owner check of `spelling-pairs.tsv` are still open.

## Plan review round 3 (check of the round 2 fixes) — Claude Opus 5.5, 2026-11-13

Checked on `feat/m08b-b4b-search-scripts` at `49834ee6`, in the main
checkout. I read the plan diff `5bf552a3..49834ee6` and checked it against
the code on `main`. I changed no code and did not run `cargo`. This check
was done by the same model as the plan author, so it is not an independent
review.

**Summary:** P16, P17 and P18 are confirmed fixed. 1 new item (P19, minor),
fixed in `5856d964` (see "Plan fix round 3").

### Confirmed fixed

- **P16.** Section 7.2 now has one version of the test, with the real data
  (`100% off`, `1000 off`). `_` is a break: `is_retained_boundary_char`
  (`search_key.rs:39`) keeps only alphanumeric chars and marks. So `100%`
  and `100_` are both the long term `100`, and both find the two messages.
  The test now covers the number cost of W4.
- **P17.** The loop has no fixed limit. I checked the proof rule by rule.
  No rule writes `x` or `q`. No rule writes `c`, `w`, `z`, `y` or `e`,
  except rule 5, which writes `c` and makes the word shorter. Every rule
  that changes a word makes (A, B, C) smaller in that order. So the proof
  holds. One rule is in the wrong group; see P19. The second test alphabet
  gives 41,370 strings, so "about 41,000" is right.
- **P18.** The author is right: my reason was wrong. The tables did write
  `ā`, `ī` and `ū`. The fix puts plain `a`, `i` and `u` in the tables.
  Rule 1 used to run first and did the same thing, so the keys do not
  change. Step 4 item 2 now names both constants.

### Findings

#### P19 — "The tables write only `a` to `z`" conflicts with the kept letters and the digits (minor)

- **Where:** D-B4b-3 (`:699`, "the table writes only the letters `a` to
  `z`") and D-B4b-4 (`:730`); section 7.2, the new bullet "every output of
  the table tests has only the letters `a` to `z`"; D-B4b-5, "This needs
  the input to have no marks" (`:801`) and the rule 3 item of "Why the
  passes end" (`:793`).
- **Problem:** The same plan keeps some letters as they are. Any Devanagari
  letter with no value (`ॠ`, `ऌ`, `ॲ`) is kept (`:712`), and section 7.2
  has the test row "`ॠ` kept as it is" (`:1448`). The digits of both
  scripts become `0` to `9` (`:669`). So `to_latin` does not write only
  `a` to `z`, and the new test fails on those rows. A Devanagari sign with
  no value, for example `ॄ` (U+0944), is a mark (`Mn`). If it is kept, the
  fold does get a mark. That does no harm now: rule 1 is gone, and the fold
  leaves every char outside `a` to `z` alone. But the sentence "This needs
  the input to have no marks" is then wrong, and it is no longer needed.
  Also, rule 3 (`sch`→`sk`) makes the word shorter, so it belongs with the
  rules that lower B, not with the rules that keep the length. The proof
  still holds.
- **Failure case:** The implementer writes the test as stated. It fails on
  the `ॠ` row and on the digit rows. Then the implementer either drops a
  planned row or changes the kept-letter rule to make the test pass.
- **Fix:** State the claim for mapped values only: "every value in the
  letter and sign tables is only `a` to `z`". Test the table entries, not
  every output of `to_latin`. Say what happens to a kept sign with no
  value. My view: keep it as it is, like a kept letter. Replace "This
  needs the input to have no marks" with "The fold changes only the
  letters `a` to `z`, so a char outside them (a digit, a kept letter or
  mark, another script) cannot affect the end." Move rule 3 to the list of
  rules that make the word shorter.
- *Status:* Fixed (5856d964): D-B4b-3 and D-B4b-4 now say "every value in the letter and sign tables is only `a` to `z`", and that chars with no value and digits are not table values. A sign with no value (`ॄ`) is kept as it is, like a kept letter, in both scripts. The section 7.2 test checks the table entries, not every output of `to_latin`, and adds a `ॄ` row. "This needs the input to have no marks" is replaced by the sentence you proposed. Rule 3 moved to the rules that make the word shorter. Rule 1 says "every table value", not "the tables write".

## Plan fix round 3 — Claude Opus 5.5, 2026-11-13

I checked P19 against the plan and accepted it. There is no pushback.
The kept letters (`ॠ`) and the digits are outputs of `to_latin` that are
not `a` to `z`, so the round 2 claim was too wide. `sch`→`sk` turns 3
letters into 2, so it lowers B. The proof is unchanged in substance.

Plan commit: `5856d964`. Open items: none. The owner questions Q1 to Q8
and the owner check of `spelling-pairs.tsv` are still open.
