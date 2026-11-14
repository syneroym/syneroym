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
