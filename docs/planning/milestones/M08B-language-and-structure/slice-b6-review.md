# M08B slice B6 — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-b6-implementation-plan.md](slice-b6-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Sonnet 5.5, 2026-11-14

Checked on `feat/m08b-b6-close-out` (`main` at `22331a3f` plus the plan
commit `1da11290`), in the main checkout (not a worktree: `.git` is a
directory). `git pull` said "Already up to date" (it needed the network, so I
ran it outside the sandbox).

**Who reviewed.** This session runs on Claude Sonnet 5.5 (the prompt names me
so). `status.md` says Claude Opus 5.5 wrote the plan.

**Change docs.** The prompt lists one change doc,
[roym-multilingual](../../changes/roym-multilingual/change.md). `task.md`
names no other change doc for B6 ("Change doc: roym-multilingual"). I agree:
every `ML-*` row, every decision and every deviation B6 touches is in that
doc.

**Summary:** 0 blocker, 1 major, 6 minor, 7 items (P1 to P7). 0 open: all 7 fixed in 580774ab.

Limits of this review: I read the plan, `task.md`, `status.md`, the whole
change doc, `session-strategy.md`, T7 of `prompt-templates.md`, the B5 review
(for the format) and the code and docs on this branch. I did not run `cargo`,
`npm`, `mise run demo` or `mise run verify`, and I changed no code and no
plan. The booking test is checked by reading the Hub source, the helpers and
the Rust methods it calls, not by running it.

### What was checked and found correct

- **Test column names.** I ran a script over the 72 backticked test names and
  28 quoted titles in the change doc's "Acceptance criteria" table and
  searched each in `crates/` and `apps/`. Every Rust name exists. The only
  titles that do not exist are the ones the plan lists in section 2.2 point 1
  (ML-2, ML-7, ML-8, ML-28). The corrected titles and lines are right:
  `taxonomy.test.ts:23`, `:53`, `:64`; `directory.test.ts:159`;
  `listings.test.ts:88` ("unit list starts at piece and has one option per
  unit with its label"); `groups.test.ts:596` ("draws group members in
  sortByName order"); `hindi-screens.test.ts:329`;
  `roym-hub.spec.ts:1360`, `:1447`, `:1477`; `listing/tests.rs:322`
  (`unit_code_rules`); `search_scripts.rs:208` (scenario 234).
- **Rows that claim more than their tests show** (section 2.2 point 2). ML-24
  is right: `contacts.test.ts` has no sort test (it has `upsertParams` and a
  search test only), and `sortByName` is at `screens/contacts.ts:64`. The
  ML-26 per-file split and the ML-12 and ML-21 rewording are consistent with
  the files I read.
- **Story 40 and helpers.** `fixtures.ts:4` exports only `Browser` and `Page`
  while `hub-helpers.ts:3` imports `Locator` from it (point 7 is right). The
  helper names, line numbers and signatures the plan cites (`ensureOwnSynOrg`
  `:65`, `publishListing` `:106`, `threadMessage` `:161`, `hubMessage` `:186`,
  `pickCategory` `:207`, `chooseHubLanguage` `:224`, `directoryAddedStatus`
  `:170`) match. `newSession` is at `demo/steps.ts:78`, and story 20 drives
  two pages the same way.
- **Bob's directory exists before story 40** (point 9). Story 30 runs
  `roym directory serve ... --category gardening` on Bob's node
  (`30-roym-client.sh:80`), and story 20 sets `BOB_DIRECTORY_DID`,
  `BOB_ADDR`, `BOB_DID`, `BOB_KEY`, `BOB_HUB_URL` (`20-roym-setup.sh:26` to
  `:48`). `directory.add-source` is idempotent (`already_present` in
  `client_sources.rs:175`), so adding Bob's directory again from the Hub is
  safe.
- **Hub selectors and messages.** These exist and mean what the plan says:
  `.toggle-request-form`, `.request-desc-input`, `.request-cats-input`,
  `.send-request-button`, `.request-form` (`messages.ts:330` to `:410`);
  `.quote-request-button` (`:654`), `.accept-quote-button` (`:672`),
  `.accept-mt-box` (`:624`); the quote form classes (`:752` to `:968`);
  `.quote-total` (`cards/templates/quote.ts:162`);
  `.request-description` with `lang` (`request.ts:66`);
  `.card-agreement-receipt` (`agreement_receipt.ts:57`); `.search-hit` with
  `data-listing-id` (`directory.ts:356`), `.hit-title` with `lang`,
  `.hit-categories`, `.hit-engage`, `.hit-engage-out` (`:437` to `:452`).
  `hi.ftl:99` `terms-total = कुल: { $amount }`, `messages-accept-button`,
  `taxonomy-category-gardening = बागवानी` all exist. The card wrapper is
  `.message` (`messages.ts:520`), so `threadMessage(...)` finds the request
  card and its quote button.
- **Methods the test calls.** `agreement.list` and `agreement.verify` are
  routed to the transaction service for the owner
  (`router.rs:37`, `api.rs:50` to `:51`). `VerifyInput` is `{ envelope }`.
  `AgreementView` has `counterparty`, `quote_author`, `pair`, `terms`;
  `ReceiptHalf` has the `envelope` string; `PairState` serialises as
  `{"state":"none"|"half"|"complete"}`. `RecordVerdict.payload` is the
  `AgreementReceiptPayload` with `viewed_lang` and `viewed_via_mt`. Alice is
  the quote's counterparty, so `counterparty.envelope` is Alice's half.
- **Countersignature path** (point 10). `maybe_countersign` runs from
  `transaction.sync` (`sync.rs:527`) and needs the signed-in owner to be the
  quote author (`agreement_ops.rs:469`), which is Bob on his own page.
- **Hub quote flow gives the values the plan expects.** The quote form
  defaults `lang` to the Hub language (English for Bob), has `INR` with
  exponent 2, and `at-giver` hides the address box. Alice's reader languages
  are the Hub language and `navigator.languages` (Playwright's default
  `en-US`), so she reads the English original, and `shownVersion` sends
  `viewed_lang = "en"` and `viewed_via_mt = false` (`messages.ts:615`,
  `:677`).
- **Contact sort expectation.** `names.ts` puts the Hub's own script first,
  then Latin, Devanagari, Tamil. So `Ram, Sita, राम` in English and
  `राम, Ram, Sita` in Hindi are right, and the "break `sortByName`" check
  fails both cases.
- **Living-doc facts.** The false sentences at `system-architecture.md:921`
  and `:1362`, the stale i18n sentence at `system-requirements-spec.md:1468`,
  the `[APP-A11Y]` row at `traceability-matrix.md:68`, the missing languages
  section in the integrated spec, backlog rows `:515`, `:516`, `:547`, and
  `docs/demo/demo.md` "R3b The Hub in Hindi" all exist as described.
  `change.md:503`, `task.md:5`, `:122`, `:217` and the ML-28 deviation
  (`change.md:601`) are as described. `living-docs-touched` already holds
  every file of section 9, so Step 7.7 only has to check it.
- **`check-change-docs` rules** (`xtask/src/change_docs.rs`): `status: done`
  needs the `living-docs-touched` and `demo:` lists filled and every path
  present. The plan's end state meets that.
- **Rules the plan names.** No port literals (the test reads the demo env),
  no planning ids in test names, `!` instead of `?.`, no product code, so no
  wire change, no component size and no import clean-up in Rust.

### Views on the owner questions

I answer none of them for the owner. My view on each:

- **Q1 (where the booking test runs).** (a) is right. Story 40 already has
  the two-node environment, runs in the verify e2e gate, and is the M08B
  story. (c) is a large change for one test. (d) proves nothing about the
  Hindi Hub. One thing the owner should know when choosing: the quote form
  and the Accept button have no browser test across two nodes today.
  `roym-hub.spec.ts` tests 26, 29 and 30 only draw them against a dummy
  address or a fake card. So this test is the first browser run of that path
  (P7).
- **Q2 (what "books" means).** (a). The Hub has no slot field, so a slot
  booking would have to leave the Hub for `roymctl`, and it has nothing to do
  with languages. Say in the demo text and the PR text that the story ends at
  the signed agreement, so nobody reads "books" as "has a time slot".
- **Q3 (the request does not name the listing).** (a). A Hub change in a
  close-out slice needs new Hindi text and an owner review. The new backlog
  row is the right record. The demo doc and the story captions must not say
  the request is "for" the listing: the request names only the category.
- **Q4 (Latin search half of the task.md bullet).** (a). Story 40's last
  step (Latin to Devanagari) and `roym-hub.spec.ts:1447` (both directions)
  already show it. Name both in the ML-21 row.
- **Q5 (`[APP-A11Y]` split).** (a), because the matrix has two status values
  and one row cannot be both. But the plan does not say where the new
  `[APP-L10N]` section goes, and the obvious place is wrong (P3).
- **Q6 (owner review of the B5 Hindi text).** (a). `status: done` should
  follow the owner's reading of the two messages with legal weight. Put the
  date in the B5 status row (B5 R6). If it is late, the rest of B6 can still
  be built, and only the last commit waits.
- **Q7 (rows met in part).** (a). task.md section 5 planned this, and each
  later part has an owner (M08C C4, M08D, M08G). One deviation and one
  backlog row for each is the right record. `status: done` here does not hide
  anything.
- **Q8 (does B6 close the milestone).** (a). T8 asks for a fresh audit, and
  T7 is the right template for a slice.

### Findings

#### P1 — Major: the listing's conversation address is Bob's directory DID, so step 5 fails

- **Where:** plan Step 4 steps 2 and 5; Step 3.2 (`publishListing`);
  `hub-helpers.ts:106` to `:122`; `screens/listings.ts:324`, `:471`;
  `directory.ts:437` to `:452`.
- **Problem:** `publishListing` does
  `page.locator('.listing-address-input').fill(directoryDid)`. That input is
  the listing's `conversation_address` (`listings.ts:324`, sent as
  `conversation_address` at `:471`). The helper uses the directory DID there
  for every listing, which is fine for the old callers (they never read it).
  A directory hit shows `hit.conversation_address` after "Message this
  listing owner" (`directory.ts:445`). So Bob's hit will show Bob's directory
  DID, not `bob.addr`. Step 5 says `.hit-engage-out` contains `bob.addr`.
  Step 6 then opens the chat with `bob.addr`, which is not what the person
  would copy from the hit.
- **Failure case:** `mise run demo` fails at step 5 of the new test:
  `.hit-engage-out` has `did:key:...` of the directory, and `bob.addr`
  (`BOB_ADDR`, the conversation service id from `roymctl roym address`) is
  not in it. A test that passed by asserting the directory DID would show a
  path no person can use, which is the same fault the plan rejects in Q3 (c).
- **Fix:** give `publishListing` a new last optional parameter
  `conversationAddress = directoryDid` (the old callers keep today's
  behaviour) and fill `.listing-address-input` with it. Bob's call passes
  `bob.addr`. Step 5 keeps its assertion. Add a row to the behaviour-change
  audit (section 4) for the new parameter, next to the return-value row.
  Also name in Step 3.2 that the signature change is for this.
- *Status:* Fixed (580774ab). Step 3.2 adds `conversationAddress = directoryDid`; Step 4 step 2 passes `bob.addr`; audit row 3.

#### P2 — Minor: step 10 can wait on a sync nobody runs, and proves only one node

- **Where:** plan Step 4 step 10; sections 1 and 3 ("the pair becomes
  `complete` on both nodes"); `sync.rs:274`, `:527`;
  `agreement_ops.rs:388` (`list`).
- **Problem:** two points.
  - A receiver files the other party's `agreement-receipt` half inside
    `transaction.sync` (`sync.rs:274`). `agreement.list` only reads. Alice's
    pair becomes `complete` only after Alice's node runs a sync that sees
    Bob's half. The plan relies on the Hub redrawing Alice's open thread when
    the event feed reports the new message, as it does for Bob's countersign.
    That works only while Alice's page keeps the conversation open and
    redraws. Step 9 leaves it open, but nothing in step 10 says it must stay,
    and `expect.poll(agreement.list)` alone never causes a sync.
  - The plan says the pair is `complete` on both nodes. Step 10 polls only
    Alice's. Bob's `agreement.list` is never read.
- **Failure case:** on a slow run, Alice's redraw is missed or late, the poll
  runs 60 s on a state that no call can change, and the test fails with a
  `half` pair. Or the test passes while Bob's node is not `complete`, and the
  plan's claim in section 3 is not tested.
- **Fix:** inside the poll, call
  `rpcCall(page, 'transaction.sync', { conversation })` on Alice's page and
  then `agreement.list`, so the check does not depend on the Hub redraw (the
  conversation id is in the `agreement.list` row). Do the same poll on
  Bob's page. Assert `complete` on both. If the plan wants to keep one node,
  change sections 1 and 3 to say "on Alice's node".
- *Status:* Fixed (580774ab). Step 4 step 10 runs `transaction.sync` then `agreement.list` in the poll, on both pages. The conversation id comes from `conversation.open` (the call `openConversation` makes through the Hub), because Bob has no agreement row to read it from until the sync runs.

#### P3 — Minor: the plan does not say where `[APP-L10N]` goes, and the appendix is wrong for it

- **Where:** plan Q5 (a), Step 6.1 and 6.2, section 9;
  `system-requirements-spec.md:1453` and `:1355`;
  `traceability-matrix.md:58`, `:68`.
- **Problem:** Step 6.1 says to add `### [APP-L10N] Localisation` but gives
  no place. Today `[APP-A11Y]` sits in "Appendix: Later-Phase Additions"
  (`:1453`), a section of envisioned items. A built requirement belongs with
  the built ones: Roym's requirements are in "Phase 6 ... — [APP-ROY]"
  (`:1355`), and `[APP-DUL]` is a `###` inside its own phase (`:1162`). An
  implementer who adds the new heading under `[APP-A11Y]` puts an
  `Implemented` requirement into the appendix of "later-phase" items.
- **Failure case:** the requirements spec has a built requirement in the
  appendix of unbuilt ones, and the matrix row order and the spec order
  disagree. A reader who looks for what Roym does today does not find it in
  Phase 6.
- **Fix:** add to Step 6.1: put `### [APP-L10N] Localisation` in Phase 6
  after the `[APP-ROY]` text (read that phase first and keep its heading
  form), leave only accessibility and the translation-engine Envisioned
  block in the appendix `[APP-A11Y]`, and put the new matrix row next to
  `[APP-ROY]` (`traceability-matrix.md:58`) with the same target milestone as
  that row. State in section 9 that the backlog row `:516` keeps naming
  `[APP-A11Y]` for the WCAG audit.
- *Status:* Fixed (580774ab). Steps 6.1 and 6.2 and section 9 place `[APP-L10N]` in Phase 6 and next to `[APP-ROY]`; backlog `:516` keeps `[APP-A11Y]`.

#### P4 — Minor: Step 2 leaves out three facts that make the contact test pass or fail by accident

- **Where:** plan Step 2.1, section 7.1; `screens/contacts.ts:64` to `:73`;
  `contacts.test.ts:11`.
- **Problem:** three points.
  - The rows are bare `div`s with no class, and their text is
    `${name} ${favourite ? "★" : ""}`, so an unfavourited row ends with a
    space. "Assert each row's text" with an equality check fails on that
    space, and the plan does not say how to find the rows.
  - The Hindi case activates Hindi for the whole test file. `contacts.test.ts`
    resets only the stubbed globals (`afterEach`). The next test then runs in
    Hindi.
  - The Latin-first English case must run with the English language active.
    If a previous file or case left Hindi on, it passes or fails for the
    wrong reason.
- **Failure case:** the English case compares `"Ram "` with `"Ram"` and
  fails, or the Hindi case leaves the language on and a later test in the
  file fails with Hindi text for no visible reason.
- **Fix:** add to Step 2: read the rows as the children of the host the test
  passes to `renderContacts`, compare `textContent.trim()`, and call
  `activate("en")` (or `activateWith("en", ...)`) before each case and in
  `afterEach`. Use `activateWith("hi", { hi: async () => HINDI })` as
  `directory.test.ts:144` does.
- *Status:* Fixed (580774ab). Step 2.1 names the rows (the list div is the last child of `.contacts-screen`, not a child of the host), `textContent!.trim()`, and `activate("en")` / `activateWith("hi", ...)` with an `afterEach` reset.

#### P5 — Minor: three figures in the plan are wrong

- **Where:** plan section 2.2 point 6 and point 3; Step 5.
- **Problem:**
  - "14 of 19 tasks are unticked" (`change.md:512` to `:534`). The list has 21
    tasks: 5 are ticked and 16 are not.
  - Step 7.6 says to "tick each task whose work is merged". Two tasks name a
    place that is not where the work went: "`roym_core`: search keys ..." was
    built in `syneroym-text-lang` (a recorded deviation), and "Write the
    M08B slice plans ..." is only true as of B6's own plan. An implementer who
    ticks them without a word leaves the task text false.
  - Step 5 says "R3b The Hub in Hindi" "starts at `:1262`". The heading is at
    `docs/demo/demo.md:1260`; `:1262` is the "Automated:" line.
- **Failure case:** a reader trusts "14 of 19", finds 16 of 21, and doubts the
  other counts. A ticked task with `roym_core` in its text says the code is
  where it is not.
- **Fix:** write "16 of 21" and `:1260`. In Step 7.6, say to edit the text of
  the two tasks to name the real place (`syneroym-text-lang`; the slice plans
  are all written) when ticking.
- *Status:* Fixed (580774ab). "16 of 21" and `:1260`; Step 7.6 lists the four task texts to edit when ticking (the two named here, plus the e2e task and the `[APP-A11Y]` task, which the owner answers Q2, Q3 and Q5 change).

#### P6 — Minor: the plan does not say which build the demo story proves

- **Where:** plan section 3 (ML-18 row), Step 4, section 7.3;
  `20-roym-setup.sh:17` (`app deploy roym crates/roym_core/app/roym.toml`).
- **Problem:** the task asks for a check on "both builds (WASM and native)
  where it applies". The demo nodes deploy Roym from the manifest, so the
  story runs one build, and the plan does not name it. The ML-18 row then
  lists the story next to the parity test as if the story proved both.
- **Failure case:** a reviewer reads "ML-18: story 40 booking" and takes it
  as proof for the native build too. The story proves only the build the
  demo nodes run.
- **Fix:** in section 3 and section 7.3, say which build the demo nodes run,
  and that the other build is covered by `agreement_lang.rs`
  (`scenario_237_accept_records_what_was_viewed_parity`). Put that sentence
  in the ML-18 Test cell.
- *Status:* Fixed (580774ab). Section 3 and 7.3 ML-18 rows and Step 7.2 name the WASM build of the demo nodes and `scenario_237_accept_records_what_was_viewed_parity` for the native build.

#### P7 — Minor: the time budget and the "first browser run" risk are not measured against the whole demo

- **Where:** plan section 7.5, Step 4 ("Run `mise run demo`"), D-B6-7;
  `playwright-demo.config.ts:17` to `:18`.
- **Problem:**
  - `globalTimeout` is 900 s for all stories together and `timeout` 180 s per
    test. The plan compares only story 40 before and after and a 120 s limit
    on the new test. If the four stories already take most of the 900 s, the
    new test can push the run over, and the failure shows as a global timeout
    in whichever story is running.
  - No Playwright test drives the quote form and Accept across two nodes
    today (tests 26, 29, 30 draw them with a dummy address or a fake card).
    So this is the first browser run of that whole path. The plan's stop rule
    (D-B6-7) is right, but it does not say what B6 does if the booking test
    cannot pass: the close-out cannot be `done` without it (Q1, section 10).
- **Failure case:** the test passes alone and the demo run fails at the
  global limit on a CI machine. Or Step 4 finds a Hub bug, the owner has not
  answered, and the PR has no clear way to proceed.
- **Fix:** in Step 4, record the whole `mise run demo` time before the new
  test and after, and write both in the PR text with the 900 s limit. Add one
  sentence to D-B6-7: if the booking test finds a product bug, the PR keeps
  Steps 2, 3, 5, 6, 7 and the unit test, the owner chooses between a fix
  slice and a backlog row, and `status: done` waits for that answer.
- *Status:* Fixed (580774ab). Step 4 records the whole demo time before and after; D-B6-7 says what the PR keeps if the booking test finds a bug; section 7.5 and the PR checklist are updated.

## Code review round 1 — Claude Opus 5.5, 2026-11-14

Reviewed `feat/m08b-b6-close-out` at `7629c8f8` (10 commits on top of
`origin/main`: 4 plan commits, 2 test commits, 4 docs commits), in the main
checkout (not a worktree: `.git` is a directory). `git pull` said "Already up
to date".

**Who reviewed.** This session runs on Claude Opus 5.5, as the prompt says.
Claude Sonnet 5.5 implemented B6 (`status.md`), so the cross-tool rule holds.

**Change docs.** The prompt lists one change doc,
[roym-multilingual](../../changes/roym-multilingual/change.md). Every B6
decision and deviation belongs there, and it holds them.

**Summary:** 0 blocker, 0 major, 5 minor, 5 items (R1 to R5). **0 open:** 5 fixed in `6445ed52`, `165e157a` and `5c1b36e3`. No
earlier code review round exists, so there are no Fixed or Rejected items to
re-check. The plan review items P1 to P7 are all Fixed, and the code follows
their fixes (P4 row reading and language reset, P6 the build named in the ML-18
cell, P7 the whole demo time in the PR text and `task.md`).

**Not a finding, but it blocks the merge:** the change doc is still
`status: draft`. By Q6 (a) it becomes `done` only after the owner reads the 13
Hindi messages of B5, and the date goes in the B5 row of `status.md`. That last
commit (plan Step 7.9) is still to come. The PR is a draft, which is right.

Limits of this review: CI for the PR (`gh pr checks 363`) is fully green on
`7629c8f8` (lints, 6 test shards, Playwright E2E, CLA;
[run 43577526546](https://github.com/syneroym/syneroym/actions/runs/43577526546)).
I did not run the full suite, `mise run demo` or `mise run verify`. I ran the
Hub unit tests (`TZ=UTC npx vitest run` in `crates/roym_web/ui`, as `npm test`
does: 529 passed in 43 files) and `npx tsc --noEmit -p .` in
`crates/substrate/tests/e2e` (no error). I read the Playwright E2E job log. I
changed no code.

### What was checked and found correct

- **No product code.** The diff touches two test files, two e2e helper files
  and docs only. No Rust, WIT, Hub screen or `.ftl` file. So there is no wire
  change, no access-control change and no component size change, as the plan's
  section 4 says.
- **Step 2, the contact sort test.** `contacts.test.ts` draws three contacts in
  the order `Sita`, `राम`, `Ram` and expects `Ram`, `Sita`, `राम` in English and
  `राम`, `Ram`, `Sita` in Hindi. A sort that keeps the input order fails both
  cases. A byte-order sort fails the Hindi case. A sort that always puts
  Devanagari first fails the English case. `afterEach` now resets the language
  to `en`. The selector `.contacts-screen > div` finds the list: `querySelector`
  returns the first direct `div` child, and the list is appended before the
  add-contact form (`contacts.ts:45` to `:46`). The deviation in the change doc
  says this.
- **Step 3, the helpers.** `Locator` is exported from `fixtures.ts`.
  `issueOwnCredential` holds the five lines that were in `ensureOwnSynOrg`, and
  `ensureOwnSynOrg` calls it, so there is one copy; the "why" comment moved into
  its doc comment. `publishListing` keeps its old behaviour for old callers
  (`conversationAddress = directoryDid`) and returns the id. The regular
  expression `/lst_[a-z0-9]+/` matches the z-base-32 digest of
  `content_digest` (`crates/signed_record/src/envelope.rs:228`), and the run tag
  reaches the slug (`slug_from_title`, `listing.rs:444`). `sendRequest` and
  `sendQuote` use the class names of `messages.ts`. `openMessages` with
  `exact: true` is a recorded deviation, and the main suite still passes (75 in
  CI).
- **Step 4, the booking test.** It follows Steps 4.1 to 4.10 in order. Every
  check on Alice's page uses a message id or record data, never English UI text
  (D-B6-3). The hit is found by `data-listing-id` (D-B6-2). Every text has the
  run tag (D-B6-4). `pairState` calls `transaction.sync` before
  `agreement.list`, so it does not depend on a redraw (Step 4.10). It reads
  `counterparty.envelope`, which is Alice's half: `counterparty` is the party
  that accepts the quote (`agreement_ops.rs:251`). The test title has no
  planning id; no `.ts` line in the diff has `ML-`, `B6`, `Q<n>`, `D-B6` or
  `M08B`. All addresses come from the demo env file.
- **Which build.** `roym.toml` sets `service_type = "wasm"` for every service, so
  story 40 runs the WASM build, as the ML-18 cell says.
- **Counts.** CI: main suite `75 passed`, multi-hop `4 passed`, demo `7 passed
  (58.3s)`; the new test passed in 6.2 s. The Hub unit tests print 529 passed in
  43 files. These match `status.md` and `task.md` (`task.md` gives the local
  demo time, 1.2 min; CI took 58.3 s; both far under the 900 s limit).
- **Change doc.** The decisions list "Decided 2026-11-14, close-out" sits at the
  end of "Agreements across languages" and has D-B6-1 to D-B6-7 and Q1 to Q8
  with the plan's meaning. The deviations of Step 7.3 are there (ML-23, the
  booking test with no slot, the request that does not name the listing), plus
  the three "different from the plan" entries. Each of ML-9, ML-23, ML-26 and
  ML-28 has a "Later:" note, a deviation and a backlog row (Q7). ML-29 sits
  after ML-20. ML-12 no longer names story 40. ML-21 names both tests (Q4).
  Every Test cell has full paths in backticks. I checked the new and changed
  titles with `git grep` ("lists Latin names first in an English Hub", "lists
  Devanagari names first in a Hindi Hub", "falls back to English when a
  language lacks taxonomy labels", "a category search sends the same query in
  English and in Hindi", "the listing screen, in Hindi", "draws group members
  in sortByName order", the two story 40 titles); all exist. The tasks are
  ticked and the four task texts of Step 7.6 are edited. The Open questions
  name the backlog. `living-docs-touched` already lists every file of plan
  section 9.
- **Living docs.** The requirements have a new `[APP-L10N]` in Phase 6 (a
  recorded deviation on its place) and the appendix `[APP-A11Y]` keeps
  accessibility only; the stale i18n sentence is gone. The matrix has the
  `[APP-L10N]` row next to `[APP-ROY]`, with the four links of Step 6.2. The two
  false sentences of the architecture doc are fixed (`git grep "translated
  labels\|must strictly be valid codes"` finds nothing). The integrated spec has
  `### Languages` with a reader line and the ADR-0029 link. The demo text says
  the story ends at the signed agreement, with no slot, and that the request
  names the category only.
- **Backlog.** The `:515` row is split: localisation is in "Recently resolved",
  accessibility stays. The bidi row names the right-to-left layout. The ML-9
  row was extended, not copied. Eight new rows match plan section 8. No new
  `TODO` or `FIXME`.
- **`task.md`.** The false "No slice is planned yet." is gone, the S0 line has
  the owner check date, the B5 result is under the B5 bullets, and B6 has a
  result line.

### Findings

#### R1 — The receipt card checks are not tied to this run (minor)

- **Where:** `crates/substrate/tests/e2e/demo/40-roym-languages.spec.ts:236`
  and `:243` at `7629c8f8`.
- **Problem:** both checks wait for
  `.thread-messages .card-agreement-receipt` with `.first()`. That is any
  receipt card in the open thread, not the one of this run. D-B6-4 says the
  story can run again on the same nodes (`mise run demo:up` leaves them
  running). On a second run the thread between Alice and Bob already holds the
  receipt card of the first run.
- **Failure case:** second run on the same nodes. Alice's Accept fails, or Bob's
  Hub never redraws. Both card checks still pass at once on the old card. The
  step captions "her Hub shows the signed agreement" and "his Hub shows the
  agreement" then show nothing. (The later `pairState` check still catches a
  missing half, so the test fails later, with a less clear message. The card
  checks are the only proof that the Hub draws the receipt.)
- **Fix:** find the card of this run. The receipt card shows the scope
  (`agreement_receipt.ts:131` to `:135`), so use
  `page.locator('.thread-messages .card-agreement-receipt', { hasText: runTag })`
  with `toHaveCount(1)`, and the same on `bobPage`.
- *Status:* Fixed (6445ed52). Both receipt checks now use `runReceiptCard`, which matches the run tag. It keeps `.first()` and does not use `toHaveCount(1)`: each half is its own card, so a thread can hold two cards of one run. Proof: with the tag changed to a wrong one, `mise run demo` fails the Alice card check ("element(s) not found", 6 of 7 passed). Restored; the demo passes (7 passed).

#### R2 — The test cannot tell Alice's signed half from Bob's (minor)

- **Where:** `crates/substrate/tests/e2e/demo/40-roym-languages.spec.ts:263` to
  `:266` at `7629c8f8`; `crates/roym_transaction/src/app/agreement_ops.rs:485`
  to `:494`.
- **Problem:** the step "her signed half says she read the English original"
  checks `verified`, `viewed_lang == "en"` and `viewed_via_mt == false`. Bob's
  countersigned half has the same values: `maybe_countersign` signs
  `viewed_lang: q_payload.terms.lang` and `viewed_via_mt: false`, and the quote
  is in English. So the three checks pass for either half. Nothing checks who
  signed the envelope that was verified.
- **Failure case:** a change makes `agreement.list` fill `counterparty` with the
  quote author's half (or the test picks the wrong field). The test still
  passes, and the ML-18 cell ("Alice's signed half ... says `viewed_lang` is
  `en`") is no longer proven.
- **Fix:** read `ALICE_DID` from the env (story 20 does) and add
  `expect(verdict.result.issuer).toBe(alice.did)`. Also check
  `verdict.result.payload.role` is `"counterparty"` (`DealRole`, kebab-case,
  `crates/roym_core/src/transaction.rs:92`).
- *Status:* Fixed (6445ed52). The test reads `ALICE_DID` and checks `issuer` is `alice.did` and `payload.role` is `counterparty`. Proof: with the expected issuer changed to `bob.did`, `mise run demo` fails at that line (expected Bob's DID, received Alice's; 6 of 7 passed). Restored; the demo passes (7 passed).

#### R3 — "The Hub has no ARIA markup" is false, and B6 copied it to two more places (minor)

- **Where:** `docs/planning/traceability-matrix.md:69`;
  `docs/planning/deferred-backlog.md:515`; `docs/system-requirements-spec.md:1475`
  (older text, but B6 rewrote this section) at `7629c8f8`.
- **Problem:** the three places say the Hub has no ARIA markup. The matrix also
  says "Only the `lang` attribute is set". The Hub sets ARIA attributes today:
  `aria-label` in `screens/directory.ts:299`, `taxonomy/category_picker.ts:98`
  and `i18n/language_list.ts:36`, `:44`; `aria-live` in
  `taxonomy/category_picker.ts:38`; `role="group"` in
  `i18n/language_list.ts:35`. AGENTS.md asks for living docs without
  over-claims; this one claims less than is built, and a reader who plans the
  WCAG work starts from a false fact.
- **Failure case:** a developer reads "no ARIA markup" and adds a second label
  to the language list, or reports the matrix row as wrong in the WCAG audit.
- **Fix:** in all three places say what is true: "The Hub sets `lang` and a few
  ARIA labels (the language list, the category picker, the search translate
  list). It has no screen reader test and no WCAG 2.1 AA audit."
- *Status:* Fixed (165e157a). The matrix row, the requirements spec and the backlog row now say the Hub sets `lang` and a few ARIA labels, and has no screen reader test and no WCAG 2.1 AA audit.

#### R4 — ML-26 and ML-16 say every no-engine case checks that no call is made; three do not (minor)

- **Where:** `docs/planning/changes/roym-multilingual/change.md:132` (ML-26,
  "Each has a node-with-no-engine case that checks no action is drawn and no
  call is made") and `:117` (ML-16, the same claim for the listed files) at
  `7629c8f8`.
- **Problem:** B6 checks that every row names tests that prove what the row
  says. These no-engine cases check only that no action is drawn. They make no
  check on the calls:
  - `crates/roym_web/ui/src/screens/directory_translate.test.ts:121` (search
    hit);
  - `crates/roym_web/ui/src/screens/messages_translate.test.ts:216`;
  - `crates/roym_web/ui/src/screens/groups.test.ts:666`.
  The cases in `record_text.test.ts`, `message_text.test.ts:78`,
  `templates.test.ts:618` and the search box case
  (`directory_translate.test.ts:58`) do check `calls` with `toEqual([])`.
- **Failure case:** a change makes the hit, the chat screen or the group screen
  call `translation.translate` while it draws. These three tests still pass,
  and the rows say this is tested.
- **Fix:** either add `const calls = stubRpc(...)` and `expect(calls)` to the
  three cases (a test-only change), or reword both cells: "checks no action is
  drawn; the search box, record text, chat message and card cases also check
  that no call is made". The first is better, because ML-16 is about the call.
- *Status:* Fixed (5c1b36e3). The three cases now also check that no `translation.translate` call is made (first option), so the ML-16 and ML-26 cells are true as written and need no edit. Proof: with a stray `translation.translate` call added to `renderMessageRow`, `renderMessages` and `renderDirectory`, the three cases fail (7 tests failed in the 3 files, the 3 changed cases among them). Restored; the 3 files pass (33 tests).

#### R5 — `DELIVERY_WAIT_MS` and its comment are copied from story 20 (minor)

- **Where:** `crates/substrate/tests/e2e/demo/40-roym-languages.spec.ts:106` to
  `:108`; `crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts:10` to `:12`
  at `7629c8f8`.
- **Problem:** the same constant and the same two-line comment are in both
  files. AGENTS.md says to look for an existing helper before writing a new
  one. The demo stories already share `demo/steps.ts`.
- **Failure case:** the outbox tick of the demo nodes changes
  (`demo-nodes.ts:91`), someone raises the wait in one story, and the other
  story keeps the old value and the old comment.
- **Fix:** move `DELIVERY_WAIT_MS` with its comment to `demo/steps.ts`, export
  it, and import it in stories 20 and 40.
- *Status:* Fixed (6445ed52). `DELIVERY_WAIT_MS` and its comment are in `demo/steps.ts`; stories 20 and 40 import it.

## Code review round 2 (check of the fixes) — Claude Opus 5.5, 2026-11-14

Checked `feat/m08b-b6-close-out` at `cf800a65`: the three fix commits
`5c1b36e3`, `165e157a`, `6445ed52` and the review close commit `cf800a65`, on
top of the round 1 commit `747071d9`. Main checkout, `git pull` done.

**Summary:** all 5 round 1 items are fixed correctly. No new finding. 0
blocker, 0 major, 0 minor. **0 open.**

Limits: no CI run had finished on the fix commits when I wrote this. (I first
wrote here that run 43594245154 passed lints and Playwright on `cf800a65`.
That was wrong: that run was for `747071d9`, the round 1 commit, which has
none of the fixes, and a later push cancelled it. Corrected at close-out; the
close-out records the CI run on the final code.) I ran the Hub unit
tests (`TZ=UTC npx vitest run`: 529 passed in 43 files) and `npx tsc --noEmit
-p .` in `crates/substrate/tests/e2e` (no error). I did not run `mise run demo`
or `mise run verify`, and I changed no code.

### Round 1 items checked

- **R1 — Fixed, correct.** Both receipt checks use
  `runReceiptCard(page, runTag)`, which matches
  `.thread-messages .card-agreement-receipt` with the run tag in its text. The
  receipt card draws the scope (`agreement_receipt.ts:131` to `:135`), and the
  scope holds the run tag, so an old card from an earlier run no longer
  matches. Keeping `.first()` instead of `toHaveCount(1)` is right: each half
  can draw its own card, so one run can have two.
- **R2 — Fixed, correct.** The test reads `ALICE_DID` and checks
  `verdict.result.issuer == alice.did` and `payload.role == "counterparty"`
  (`DealRole` is kebab-case, `transaction.rs:92`). Bob's half has issuer
  `bob.did` and role `quote-author`, so the check now tells the halves apart.
  The comment says why.
- **R3 — Fixed, correct.** The matrix row, the requirements appendix and the
  backlog row now say the Hub sets `lang` and a few ARIA labels (the language
  list, the category picker, the search translate list) and has no screen
  reader test and no WCAG 2.1 AA audit. This matches the code
  (`language_list.ts:35` to `:44`, `category_picker.ts:38`, `:98`,
  `directory.ts:299`).
- **R4 — Fixed, correct (the first option).** The three cases now check the
  calls: `directory_translate.test.ts` returns `calls` from `drawn` through
  `withHit` and checks there is no `translation.translate`;
  `messages_translate.test.ts` returns the `stubRpc` calls from
  `conversation(...)` and checks the same; `groups.test.ts` stubs with `{}` and
  checks `calls` is empty. The three titles now say "and makes no call". So the
  ML-16 and ML-26 cells are true as written. The other cases of the changed
  describe blocks only take `{ host }` from `withHit`, with no other change.
- **R5 — Fixed, correct.** `DELIVERY_WAIT_MS` is exported once from
  `demo/steps.ts` with a doc comment, and stories 20 and 40 import it. No copy
  is left (`git grep "DELIVERY_WAIT_MS = "` finds one line).

### New problems in the fix commits

None. The fix commits touch only test code and three docs. No planning id in
any `.ts` line, no new helper that copies an old one, and the imports are used
(`Locator` in story 40 is the return type of `runReceiptCard`). `status.md`
counts match the test output above.

**Still before merge (not a finding):** the change doc is `status: draft`
until the owner reads the 13 Hindi messages of B5 (Q6), and the full
`mise run verify` with no `--skip` has not run yet.

## Close-out check — Claude Opus 5.5, 2026-11-14

The owner read the 13 Hindi messages of B5 on 2026-11-14 and asked for no
change (date in the B5 row of `status.md`). I then started the close (T7) at
`11b02da1`. Full CI is green on `11b02da1`
([run 43595402270](https://github.com/syneroym/syneroym/actions/runs/43595402270):
lints, 6 test shards, Playwright E2E with 75 + 4 passed and demo 7 passed).
`cargo xtask check-change-docs` and `check-file-lengths` pass.

The local `mise run verify` (no `--skip`, sandbox off) passed 15 of 16 gates
and failed the e2e gate: the new story 40 test timed out after 180 s in the
step "Bob's screen: read the request and send a quote in rupees". I then ran
`mise run demo` 7 more times on the same code. The test failed once more (the
first run, same step) and passed 6 times. So it fails in 2 of 8 local runs. It
passed in all 3 CI runs. By D-B6-7 the close stops here, and the change doc
stays `status: draft`.

**Summary:** 0 blocker, 1 major, 0 minor. R6 fixed in `475b8c30`; a check of the fix by another tool is still due.

#### R6 — A thread redraw removes an open quote form, so the new test hangs now and then (major)

- **Where:** `crates/roym_web/ui/src/screens/messages.ts:235` to `:264`
  (`onFeed`), `:402` (`fillHistory`), `:654` and `:752` (the quote form is
  drawn inside the message element); `crates/roym_web/ui/src/dom.ts:121`
  (`host.replaceChildren(into)`); `crates/substrate/tests/e2e/hub-helpers.ts`
  `sendQuote` at `11b02da1`.
- **Problem:** each event-feed event whose subject is the open conversation
  calls `fillHistory`, and `refiller` replaces every child of the thread. The
  quote form lives inside the request's message element, so the redraw
  removes it, with all the text the person typed. In the test, `sendQuote`
  clicks "Quote this request" and then fills the form. If an event for the
  conversation arrives between the click and the fills, the form is gone. A
  Playwright `fill` has no timeout of its own, so it waits until the 180 s
  test timeout. Every `expect` in that step has a shorter timeout, and none
  failed, which fits an action that waits for an element that never comes
  back. I could not catch the exact action: with tracing on, or with a
  temporary `actionTimeout`, the 6 runs all passed. The cause is therefore
  read from the code, not seen in a trace.
- **Failure case:** for a person: Bob types half a quote, Alice sends a chat
  message in the same conversation, and Bob's form disappears with his text.
  The same happens to the warning box before an acceptance (`.accept-mt-box`
  is in the message element too). For the suite: story 40 fails about 1 run
  in 4 on this machine, and the demo stories run in `npm test`, so
  `mise run verify` and the CI job "Playwright E2E" can fail at random.
- **Fix:** this is product code, so by D-B6-7 the owner chooses:
  - (a) **Recommended.** A small fix slice before B6 merges: the Hub keeps an
    open quote form (and an open warning box) across a thread redraw, for
    example by not redrawing the thread while one of them is open and
    redrawing when it closes. With a unit test that sends a feed event while
    the form is open. Then B6 rebases, and `mise run demo` runs at least 5
    times with no failure before the close.
  - (b) A backlog row for the Hub bug, and the test waits until the thread is
    quiet before it opens the form. This hides the bug from the test and
    keeps a race in the demo.
- *Status:* Open (round 3, 2026-11-14: the fix in 475b8c30 is partial.
  A feed redraw that is already running when the person opens the form
  still removes it. See "Code review round 3", item R6 again). Earlier
  note: Fixed (475b8c30), in B6 by owner decision (2026-11-14), not in
  a separate slice. Claude Opus 5.5 wrote the fix, so a check by another
  tool was still due (cross-tool rule). The fix: `waitingRedraw`
  (`crates/roym_web/ui/src/screens/thread_redraw.ts`) gives the thread two
  redraws. The event feed gets `later`, which waits while `.quote-form`,
  `.decline-dialog`, `.delete-dialog` or `.accept-mt-box` is in the thread
  and runs once when the last one leaves (a `MutationObserver` sees it). The
  person's own actions get `now`. This matters: delete and decline call
  their redraw while their dialog is still open. Tests: 5 in
  `thread_redraw.test.ts`, 3 new in `messages_feed.test.ts` (the moved feed
  test is there too; the fake `EventSource` moved to
  `src/testing/fake-event-source.ts`). Proof that they can fail: a feed
  redraw that does not wait fails the two "keeps" cases; `later` for the
  person's actions fails "a confirmed delete draws the thread again at
  once"; a helper that never waits fails 5 cases. Hub unit tests 537 passed
  in 45 files. `mise run demo` 5 times on the fix: 5 of 5 passed, 7 tests
  each, the booking test in 5.7 s to 7.0 s (before the fix it hung in 2 of 8
  local runs and in CI run 43600107630 on `8e4be392`).

## Code review round 3 (check of the R6 fix) — Claude Sonnet 5.5, 2026-11-14

Checked `feat/m08b-b6-close-out` at `8799efd2`: the Hub fix `475b8c30` for R6
and its doc commit `8799efd2`. Main checkout (not a worktree), `git pull` said
"Already up to date". Claude Opus 5.5 wrote the fix, and I am a different
tool, so the cross-tool rule holds. Change doc:
[roym-multilingual](../../changes/roym-multilingual/change.md). Rounds 1 and 2
are closed and I did not re-check them.

**Summary:** 0 blocker, 1 major, 2 minor. **3 open:** R6 (back to Open, the
fix is partial), R7, R8.

Limits: `gh pr checks 363` on `8799efd2` showed lints, Playwright E2E, CLA and
test shards 1, 4, 5 and 6 passed, and shards 2 and 3 still running when I
wrote this. I ran the Hub unit tests (`TZ=UTC npx vitest run` in
`crates/roym_web/ui`: 537 passed in 45 files, as `status.md` says) and
`npx tsc --noEmit -p .` (no error). I did not run `mise run demo` or
`mise run verify`. Three short tests of my own proved suspicions. Two changed
`messages.ts` for one run each and were undone, and one was a new test file
(kept in the scratchpad, not in the repo). The tree is clean.

### What was checked and found correct

- **Every redraw path of the Messages thread.** Callers of the thread fill are
  now: the first draw (`messages.ts:448`, direct, right: nothing is open yet),
  the two feed paths in `onFeed` (`:247` for a `reload` event and `:258` for a
  `kind` event; both call `activeFillHistory`, which is `redraw.later`, set at
  `:447`), and `redraw.now` for the person's own actions: compose send (`:324`),
  request send (`:396`) and the `refresh` argument of every message element
  (`:441`: retry, accept, decline, delete, quote submit). `grep` finds no other
  caller of `fillHistory`. `groups.ts` has its own thread and uses no quote
  form or dialog of this screen, so it is not part of this fix.
- **`OPEN_WORK` against the elements that hold input inside
  `.thread-messages`.** The quote form holds all its inputs and selects
  (`openQuoteForm`, `:759` on; including the text-language select and the
  address row). The decline dialog holds the note input. The delete dialog
  holds the "ask the peer" checkbox. The warning before an acceptance
  (`.accept-mt-box`) holds buttons only and is rightly listed, because a redraw
  would close it. The compose row, the request form, the search box and the
  conversation list are outside `.thread-messages` (`:287` to `:289` and
  `pane.appendChild`), so the thread redraw never touches them. The card
  templates draw no input. What is left out is view state, not typed input: the
  record-version switch of `recordText` and a message translation. A kept
  translation is drawn again from the page cache (`message_text.ts`), and
  `shownVersion` reads the screen at the click, so an acceptance records what
  is on the screen after a redraw. No missing selector.
- **The observer, in the cases I could construct.** Quote submit does
  `form.remove(); await refresh()`: `refresh` is `now`, which sets
  `waiting = false` in the same tick, before the observer's microtask runs, so
  there is one redraw, not two. Decline and delete call `refresh` while their
  dialog is open, so `now` is needed there, and the delete test shows it. Two
  feed events while a form is open set `waiting` once and give one redraw. A
  redraw's own swap fires the observer, but `waiting` is already false, so
  there is no loop. `now` clears `waiting`, so a form that closes later causes
  no stale redraw. The newest-wins check of `refiller` (`dom.ts:120`) still
  makes the last started run the only one that draws.
- **Tests do what their titles say, where they reach.** Hub unit tests pass.
  The moved feed test keeps its assertions (it now uses `body_encoding:
  "utf8"`, the value the code reads, instead of `"utf-8"`; better). The count
  is right: 529 − 1 (moved) + 1 + 3 + 5 = 537.
- **Docs.** The change doc Deviation, the `task.md` result, the backlog rows
  (the new "a chat thread shows no new message while a form is open" row, the
  "Recently resolved" row, and the removed sentence in the
  machine-translation row) and the `status.md` row agree with the code and
  with each other, except where R6 below says the claim is too wide. No
  planning id in the new code. Import lines are used. `thread_redraw.ts` and
  `fake-event-source.ts` are new files beside their users.

### Findings

#### R6 (again) — A feed redraw that is already running still removes a form opened after it started (major)

- **Where:** `crates/roym_web/ui/src/screens/thread_redraw.ts:30` to `:36`
  (`later` checks for open work only when it starts);
  `crates/roym_web/ui/src/dom.ts:120` to `:121` (`refiller` swaps the result
  in without looking at the thread again); `messages.ts:408` and `:446`.
- **Problem:** `fillHistory` makes three node calls (`transaction.sync`,
  `transaction.thread`, `conversation.history`) before it swaps the new thread
  in, and `transaction.sync` reads from the mailbox, so a redraw can run for
  a good while. `later` looks for an open form once, at the start. If the
  thread is quiet then, the redraw starts. A form that the person opens while
  the three calls run is removed by the swap, with its text. The observer
  cannot help: it only acts on a `waiting` redraw, and none was set. This is
  the same loss as R6, with a shorter window.
- **Proof:** a test of mine (not kept): draw the screen, set
  `conversation.history` to wait on a promise, emit a feed event for the
  conversation and let the burst timer run, so a redraw is running. Click
  "Quote this request", type in `.quote-scope-input`, then release the promise.
  `expect(screen.querySelector(".quote-scope-input")).toBe(scope)` fails: the
  value is `null`. The existing tests never see this because their node answers
  at once, so the redraw has finished before the test opens a form.
- **Failure case:** for a person: a message from the peer arrives and the Hub
  starts to fetch the thread. Bob clicks "Quote this request" and starts to type.
  The fetch ends and the form is gone. For the suite: in the booking test
  `sendQuote` opens the form right after the request card shows, while a second
  event of the same conversation (the delivery state, a sync result) can have a
  redraw running. This is the route by which the hang of R6 can still happen. The
  evidence of the fix is also thin: 5 of 5 demo runs passed, but a failure rate
  of 2 in 8 gives 5 clean runs by chance about one time in four. So the fix is
  not shown to have removed the hang.
- **Fix:** check again at the swap, not only at the start. For example, give
  `refiller` an optional guard that it calls just before `replaceChildren`
  (`keep?: () => boolean`, return `true` to skip the swap and resolve
  `undefined`), and make `waitingRedraw` pass a guard that sets `waiting = true`
  and returns `true` when `host.querySelector(openWork) !== null`. Then the
  observer redraws once the form closes, as for a redraw that waits at the
  start. Add a unit test that is my temporary test (a slow `conversation.history`),
  and a `thread_redraw.test.ts` case for the guard. After it, run
  `mise run demo` at least 10 times, and write the number in the status row
  (5 of 5 is not enough evidence). Update the change doc Deviation and the
  backlog row: both say a redraw "waits while such an element is open", which
  is true only at the start until this is fixed.
- *Status:* Open

#### R7 — A message the person sends, or a request, redraws at once and removes an open quote form (minor)

- **Where:** `messages.ts:324` (compose send) and `:396` (request send), both
  `redraw.now()`; also the retry and accept buttons through `redraw.now` at
  `:441`.
- **Problem:** `now` is right for decline and delete, because they run while
  their own dialog is open. It is not needed for an action whose form is
  outside the thread. A person who sends "one moment" in the compose row, or
  presses Retry or Accept on another card, while a quote form holds typed text
  loses that text. This is the same class as R6, started by the person.
- **Failure case:** Bob types half a quote, sends "what is your address?" in the
  compose row, and the quote form is gone.
- **Fix:** either (a) keep this as a decision: say in the change doc Deviation
  and in the backlog row that the person's own actions redraw at once and can
  close another open form, or (b) let these actions use a redraw that runs at
  once but keeps the open forms (for example, move the open elements from the old
  thread into the new one). (a) is enough for v1 if the owner agrees; it needs
  only the doc lines.
- *Status:* Open

#### R8 — The tests do not tie the real `OPEN_WORK` and the `now` calls to the behaviour (minor)

- **Where:** `crates/roym_web/ui/src/screens/thread_redraw.test.ts:6` (its own
  `OPEN` list), `messages_feed.test.ts` (covers only the quote form and the
  delete dialog), `messages.ts:21`.
- **Problem:** `OPEN_WORK` is not exported and the unit tests of `waitingRedraw`
  use a copy. Two mutations that I made and undid both left all 118 tests of
  `src/screens` green: (1) `OPEN_WORK` cut to `".quote-form, .delete-dialog"`
  (no test uses the decline dialog or the warning before an acceptance with a
  feed event); (2) `redraw.now()` replaced by `redraw.later()` in compose send
  and request send (no test sends while the thread waits). The review file says
  the tests were seen to fail for three changes; these two are not among them.
- **Failure case:** someone removes `.accept-mt-box` from `OPEN_WORK` when they
  tidy the selector. A feed event then closes the warning before an acceptance,
  which is the case that the removed sentence of the backlog row said was
  still open. Nothing fails.
- **Fix:** add two feed cases to `messages_feed.test.ts` in the style of the
  existing ones: a feed event while `.decline-dialog` is open, and while
  `.accept-mt-box` is open (a quote card for me, shown through machine
  translation; the unit tests of `messages_translate.test.ts` have the set-up
  for the warning). Add one case that a decline confirm redraws at once. Prove
  each can fail, as for the first three, and write it in the status of the
  item.
- *Status:* Open

### Not findings

- The observer is not disconnected when the thread is replaced. If the person
  switches conversation while a form of the old thread is open and `waiting`
  is true, a later removal inside the old, detached host (for example a send
  that ends) makes one redraw of the old conversation into the detached
  element. That is one extra set of node calls and no change on screen. The
  code comment says "no mutation reaches this observer again". Close enough
  for v1, but the comment is a little too sure.
- When the person confirms the warning before an acceptance, `box.remove()`
  can start the waiting redraw while `agreement.accept` is running, so the
  Accept button and its error line are drawn again. A failed accept then shows
  no error. A feed event during an accept without a warning does the same, so
  this is older than the fix and not a change by it.
- `events.test.ts` keeps its own `FakeEventSource` beside the shared
  `FakeFeedSource`. The two differ in what they offer; not worth a change.
