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

**Summary:** 0 blocker, 4 major, 11 minor, 15 items (P1 to P15), all Open.

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

#### P15 — A wall-clock limit in a unit test will be flaky (minor)

- **Where:** Section 7.2 "text_lang" ("a 1 KiB input of each script (no panic,
  linear time: under 10 ms in a debug build)").
- **Problem:** A fixed time bound in a normal test fails on a busy CI runner or
  under the coverage build. B4a measured time only in `#[ignore]` tests.
- **Failure case:** `mise run verify` fails at random on the nextest gate.
- **Fix:** Keep the 1 KiB no-panic test, and check linear time by a count, not
  by time: for example, the key of a 64 KiB input is computed, and the
  measurement goes into an `#[ignore]` test with the number in the PR text.
- *Status:* Open
