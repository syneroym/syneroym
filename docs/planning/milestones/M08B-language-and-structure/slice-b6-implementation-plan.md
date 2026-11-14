# M08B slice B6 — Close-out: implementation plan

*Reader: the developer (or AI) who implements slice B6. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan written, 2026-11-14. Owner questions Q1 to Q8 are open
(section 2.1).**

**Size and PR.** S. One PR on `feat/m08b-b6-close-out`, with the commits of
section 6 in that order. The branch starts from `main` at `22331a3f` (B5 is
merged, [PR #362](https://github.com/syneroym/syneroym/pull/362)) and
already holds the plan commit.

B6 ends the roym-multilingual change. It adds one browser test on the two
demo nodes: Alice uses the Hub in Hindi, finds Bob's English listing by its
category, asks Bob for the work, gets his quote and accepts it. Both nodes
then hold a signed agreement. B6 also checks every `ML-*` row of the change
doc against the code: each row must name tests that exist and that prove
what the row says, or name the later sub-milestone that finishes it. It
fixes the living docs that are now false or incomplete, moves the
localisation part of the backlog row to "Recently resolved", and sets the
change doc to `status: done`. B6 changes no product code: no Rust code, no
Hub screen and no message in a string catalog.

**Base.** Every file path and line number below was checked on `main` at
`22331a3f`. Links are relative to this folder.

**Dependencies.** [task.md](task.md) section 4: B6 comes after B5. Every
slice S0 to B5 is "Merged" in [status.md](status.md).

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`). The demo stories run inside its e2e gate (`npm test` runs
    `npm run test:demo`, `crates/substrate/tests/e2e/package.json:6`).
  - "Mandatory Living-Docs Close-out" and "Mandatory Demo Close-out": B6 is
    the slice that does them for the whole change doc.
  - "Mandatory Deferred-Backlog Update".
  - No planning ids in code, comments or test names. This also covers
    `ML-<n>`: the planning-refs gate does not catch it (B2 R5), so a
    reviewer must. No "M08B", "B6", "ML-18", "Q3", "P4" or "R7" in `.ts`
    or `.rs` files.
  - "Writing for readers": the living-doc text you write is for its named
    reader, in short sentences, with no change-doc or slice ids.
  - No hardcoded ports in e2e code. The demo stories read every address
    from the demo env file (`readDemoEnv`, `requireEnv`).
- [task.md](task.md): section 3 "B6", section 5 (criteria that a later
  sub-milestone finishes), section 7 (backlog rows for M08B).
- [roym-multilingual change doc](../../changes/roym-multilingual/change.md):
  all of it. B6 owns the whole "Acceptance criteria" table, "Tasks",
  "Open questions", "Deviations" and "Close-out".
- [prompt-templates.md](../../prompt-templates.md) T7 (close a slice) and
  T8 (close a milestone): B6 is a slice. It does not close the milestone
  (Q8).
- [slice-b5-implementation-plan.md](slice-b5-implementation-plan.md) (the
  layout of this plan) and the nine review files. Section 7.6 lists how B6
  avoids their kinds of findings.

**Change docs.** The prompt lists one change doc, roym-multilingual.
[task.md](task.md) names no other change doc for B6. Every criterion B6
checks is in roym-multilingual (section 3), and every B6 decision and
deviation is written there, and only there.

### Terms used in this plan

- **Demo nodes:** the two substrate nodes that `mise run demo` boots, one
  for Alice and one for Bob. Story 20 deploys Roym on both and writes their
  addresses to the demo env file (`demo/20-roym-setup.sh:26` to `:48`:
  `<NAME>_HUB_URL`, `<NAME>_KEY`, `<NAME>_DID`, `<NAME>_DIRECTORY_DID`,
  `<NAME>_ADDR`).
- **Story 40:** `crates/substrate/tests/e2e/demo/40-roym-languages.spec.ts`,
  the M08B demo story. Today it has one test (`:17`).
- **Booking (in this plan):** the Hub path that ends in a signed agreement:
  a request card, a quote card, Accept, and both halves of the
  `agreement-receipt` (Q2). The Hub quote form has no slot field
  (`crates/roym_web/ui/src/screens/messages.ts:752` to `:968`), so a slot
  booking with `booking-progress` is not part of it.
- **Acceptance half:** one party's signed `agreement-receipt`. The stored
  wrapper is `ReceiptHalf` (`crates/roym_core/src/transaction.rs:252`), with
  the signed envelope as a string.
- **Pair state:** `PairState` (`transaction.rs:261`), serialised as
  `{"state": "none" | "half" | "complete"}`. `complete` means both halves
  exist.
- **Test column:** the "Test" column of the change doc's "Acceptance
  criteria" table (`change.md:94` to `:135`).

## 1. What B6 builds, in one paragraph

A second test in story 40 (Q1): Bob, in an English Hub on his node,
publishes an English listing in category `gardening` to his own directory.
Alice, in a Hindi Hub on her node, adds Bob's directory, searches by the
Hindi label of `gardening`, finds Bob's listing, presses "Message this
listing owner" and reads Bob's address, sends Bob a request in Hindi, and
accepts Bob's quote in rupees. The test checks that Alice's half signs
`viewed_lang = "en"` and `viewed_via_mt = false`, and that the pair becomes
`complete` on both nodes. Three small test helpers support it in
`hub-helpers.ts`. One Hub unit test closes a gap in ML-24 (the contact list
sort). Then the close-out: every `ML-*` row of the change doc gets correct
test names and full paths, the missing deviations are added, the living
docs say what M08B built, the backlog is updated, and the change doc gets
`status: done` (Q6, Q7).

## How this is shown to an end user

- Alice opens her Hub and chooses Hindi. Every screen is in Hindi.
- In the Directory screen she picks "बागवानी" (gardening) and presses
  search. She sees Bob's listing "Hedge trimming". The title stays in
  English, as Bob wrote it, and the category shows in Hindi.
- She presses "इस लिस्टिंग के मालिक को संदेश भेजें" (message this listing
  owner) and sees Bob's address. She opens a chat with Bob and sends him a
  request, written in Hindi.
- Bob uses his Hub in English. He sees Alice's request and sends a quote of
  ₹1,00,000.50.
- Alice sees the quote with "कुल: ₹1,00,000.50" (Indian digit grouping) and
  presses "ये शर्तें स्वीकार करें" (accept these terms). Both Hubs show the
  signed agreement.

Nothing in this story is new for the person. B1a to B5 built each part. B6
shows them working together, across two nodes, in one story.

How a developer can see it work:

- `mise run demo` runs story 40. The new test is "Roym languages story:
  Alice, in Hindi, books Bob's English listing". `DEMO_MODE=present mise
  run demo` shows it in a headed browser, step by step.
- `cd crates/roym_web/ui && npx vitest run src/screens/contacts.test.ts`
  runs the new contact sort test.
- `cargo xtask check-change-docs` passes with the change doc at
  `status: done`.

**Demo:** story 40 grows by one test (Q1). The change doc's `demo:` list
already names story 40 (`change.md:15` to `:16`).

## 2. Points for the owner

### 2.1 Questions

Each question names the step it blocks. The recommendation is the first
option.

**Q1. Where does the booking test run? (blocks Steps 3 and 4)**

A booking needs two people. The main Playwright suite boots one node and
one owner (`crates/substrate/tests/e2e/global-setup.ts:32`), and every test
there signs in as that owner. The B5 plan found the same limit (B5 P2) and
proved acceptance on two nodes in Rust only
(`crates/substrate/tests/roym_transaction_e2e.rs:498`). The demo has two
nodes with a Hub each, and story 20 already drives two pages
(`demo/20-roym-chat.spec.ts:25`, `newSession` at `demo/steps.ts:78`).

- (a) A second `test(...)` in story 40, on the two demo nodes. It runs in
  `mise run demo`, in `npm test`, in the verify e2e gate and in the CI job
  "Playwright E2E". It is the M08B story, so a viewer sees the whole
  language path in one place. **Recommended.**
- (b) A new story file `41-roym-booking-languages.spec.ts`. Same nodes and
  same code. It only splits the M08B story in two.
- (c) A second Roym node in the main suite's global set-up. Large: a new
  node, a second Roym deploy and a second session key for every run of the
  main suite. No other test needs it today.
- (d) Rust only (`roym_transaction_e2e`). It has no Hub, so it proves
  nothing about the Hindi Hub.

**Q2. What does "books" mean in this test? (blocks Step 4)**

task.md B6 says "finds and books an English listing". The Hub quote form
has no slot (`messages.ts:752` to `:968`). A slot booking with
`booking-progress` runs only through `roymctl` and the Rust tests
(`crates/substrate/tests/roym_booking_e2e.rs:197`).

- (a) The booking ends when the agreement is complete: request, quote,
  Accept, and `PairState` `complete` on both nodes. **Recommended.** It is
  the Hub's whole transaction path today (integrated spec C12 to C14,
  `docs/roym-integrated-experience-spec.md:391` to `:399`).
- (b) Also schedule a slot through `roymctl`. It leaves the Hub, and the
  slot path has nothing to do with languages.

**Q3. The request does not name the listing. (blocks Step 4)**

A directory hit has a "Message this listing owner" button
(`crates/roym_web/ui/src/screens/directory.ts:437` to `:452`). It shows the
owner's conversation address and nothing else. The request form has a
listing id box (`messages.ts:350` to `:352`), but the hit does not show the
listing id to the person. It is only in `data-listing-id` on the hit
element (`directory.ts:356`).

- (a) The request names the category only, and the listing id box stays
  empty. A new backlog row records that a request made from a directory hit
  does not name the listing. **Recommended.** B6 is a close-out; a Hub
  change here needs new strings, unit tests and an owner review of the
  Hindi text.
- (b) Change the Hub: the button opens the chat and fills the request form
  with the listing id. A product change in a close-out slice.
- (c) The test copies `data-listing-id` into the box. Rejected: no person
  can do this, so the test would show a path that does not exist.

**Q4. The Latin-letter search half of task.md B6. (blocks Step 4)**

task.md B6 also asks for "a Latin-letter search finds a Devanagari title".
Two tests already show it: story 40's last step (`40-roym-languages.spec.ts:85`
to `:103`, Latin to Devanagari) and `roym-hub.spec.ts:1447` "a Latin search
finds a Devanagari listing, and the other way" (both directions).

- (a) B6 adds no new search test and names these two in the ML-21 row.
  **Recommended.**
- (b) Repeat the search in the booking test. It adds time to a story that
  is already the longest, and proves nothing new.

**Q5. How do the requirements and the matrix show a half-built
`[APP-A11Y]`? (blocks Step 6)**

`[APP-A11Y]` is "Accessibility and Localisation"
(`docs/system-requirements-spec.md:1459`). After M08B, localisation is
built, but accessibility (WCAG 2.1 AA, screen reader tests) and a
translation engine are not. The matrix row
(`docs/planning/traceability-matrix.md:68`) says status `Envisioned` and,
in its description, "UI translation bundles ... are Envisioned", while its
evidence says the bundles exist. The matrix uses only two status values,
`Implemented` and `Envisioned`.

- (a) Split the requirement in two: `[APP-A11Y]` Accessibility (stays
  `Envisioned`) and a new `[APP-L10N]` Localisation (`Implemented`; the
  translation engine stays an `Envisioned` sub-part). Each gets its own
  matrix row. **Recommended.** Each status is then true with no qualifier.
  `git grep -n "APP-A11Y"` today finds the requirements, the matrix, the
  backlog (`deferred-backlog.md:515`, `:516`), the change doc, task.md and
  plan files. Step 6 updates the three living docs; plans and the change
  doc are history and keep the old id.
- (b) One row, status `Implemented`, and the description says that
  accessibility and the engine are Envisioned. This is the form of
  `[PRD-POR]` (`traceability-matrix.md`, "Cross-version migration fixtures
  are Envisioned"). A reader who reads only the status column thinks WCAG
  is done.
- (c) One row, status `Envisioned`, and the description says what is
  built. This is the form of `[PRD-CUX]`. A reader who reads only the
  status column thinks localisation is not built.

**Q6. The open owner review of the B5 Hindi text. (blocks Step 7)**

status.md says the owner review of the 13 new Hindi messages of B5 is
still open: the nine of the slice (two with legal weight,
`messages-accept-mt-warning` and `messages-accept-mt-confirm`) and the four
refusal messages. The B5 review says to put the date in the B5 status row,
not in the `hi.ftl` header (B5 R6).

- (a) The change doc gets `status: done` only after this review. B6 puts
  the date in the B5 status row in the same PR. If the owner asks for
  changes, B6 makes them (a change to `hi.ftl` only). **Recommended:**
  "every criterion is met" includes Hindi text that the owner has read,
  and the two legal messages bind a person.
- (b) Set `status: done` now and keep the review as a backlog row.

**Q7. Criteria with a part that a later sub-milestone builds. (blocks
Step 7)**

Three rows are met in part, by design:

- ML-26: four hook points now; widget answer (M08C C4), opinion (M08G),
  offering, want and storefront display (M08D). The deviation exists
  (`change.md:600`).
- ML-28: taxonomy attribute names and choices now; widget labels and
  extension step names in M08C C4 ([task.md](task.md) section 5,
  `task.md:258`). **No deviation records this yet.** The row says
  "M08C/M08D", which differs from task.md.
- ML-9: the unit label shows in the listing editor; a reader's view of
  another node's product unit is in M08D (deviation at `change.md:561`).
- ML-23: "shop name" has no record yet; the shop is M08D. No deviation
  records this.

The options:

- (a) `status: done`. Each row names its later part and its milestone in
  the Test column, the Deviations list holds one entry for each, and the
  backlog holds one row for each, so the later milestone finds it.
  **Recommended.** task.md section 5 planned exactly this.
- (b) Keep `status: draft` until M08C, M08D and M08G are done. The change
  doc then stays open for months for parts that other change docs build.

This plan commit adds the missing ML-28 deviation now, because task.md
section 5 says to record it "when its slice is planned" (section 9). The
ML-23 deviation is added in Step 7.

**Q8. Does B6 also close milestone M08B? (blocks nothing in B6)**

- (a) No. B6 is a slice and follows T7. The milestone close (T8: the
  M08-overview row, the roadmap row, the task.md header, the audit of
  every plan and review file) is a separate session after B6 merges.
  **Recommended:** T8 asks for a fresh audit by another tool.
- (b) Yes, in the same PR.

### 2.2 Points found while reading

1. **Wrong test names in the Test column.** A check of every name with
   `git grep` found these (all other Rust, Playwright and Vitest names
   exist):
   - ML-2: "falls back to English for missing taxonomy labels" does not
     exist. Real: `crates/roym_web/ui/src/taxonomy/taxonomy.test.ts:53`
     "falls back to English when a language lacks taxonomy labels".
   - ML-7: "a category shows its label in each language" is `:23` "shows
     English labels, and in Hindi different text with a Devanagari
     letter"; "falls back to the English label" is `:53`; "shows an unknown
     code as it came" is `:64` "shows unknown codes as they came, for each
     of the five label functions" (the `constructor` cases are at `:71`).
   - ML-8: "the same category gives the same query in English and Hindi"
     is `crates/roym_web/ui/src/screens/directory.test.ts:159` "a category
     search sends the same query in English and in Hindi".
   - ML-28: "an attribute and its choices show their labels" does not
     exist. The checks are inside `taxonomy.test.ts:23`, `:53` and `:64`;
     the Rust checks are in `crates/roym_core/src/taxonomy/tests.rs`.
2. **Rows that claim more than their tests show.**
   - ML-9: `listings/editor.test.ts` has no unit-option test (it sets
     `unit = "piece"` at `:165`). The unit options are tested in
     `crates/roym_web/ui/src/screens/listings.test.ts:88` "unit list starts
     at piece and has one option per unit with its label", in the active
     language only. The Hindi labels are in `taxonomy.test.ts:23` and the
     Hindi sweep (`i18n/hindi-screens.test.ts:329`).
   - ML-12: story 40 does not show the best-version choice or "open the
     original". It only adds a translation in the editor (`:62`). The
     Playwright proof is `roym-hub.spec.ts:1360`.
   - ML-21: story 40 shows only Latin to Devanagari (`:85`).
     `roym-hub.spec.ts:1447` shows both directions.
   - ML-24: `crates/roym_web/ui/src/screens/contacts.test.ts` has no sort
     or script test (`:11`, `:40`). The contact list sort
     (`screens/contacts.ts:64`, `sortByName`) is shown only by the
     Playwright test `roym-hub.spec.ts:1477` (English Hub: Latin before
     Devanagari, `:1527` to `:1533`). Step 2 adds a unit test.
   - ML-26: the row says each hook point has a no-engine case, an
     exact-params case and an `unavailable` case. That is true per hook
     point, not per file: `cards/templates/templates.test.ts` (describe at
     `:553`) has no `unavailable` case, the chat block of
     `screens/messages_translate.test.ts` (`:170`) and
     `screens/groups.test.ts` (`:639`) have neither. Those cases are in
     `records/record_text.test.ts:217`, `:249` and
     `chat/message_text.test.ts:85`, `:137`. The row must say which file
     holds which case.
   - ML-4: `formatTime` is tested only for an invalid date
     (`i18n/format.test.ts:50`); time of day is tested inside
     `formatDateTime` (`:37`). The row stays true ("times use the Hub
     language's format"), because `formatDateTime` prints the time. No
     change.
   - ML-5: "so ... screen readers read it in the right language" is the
     reason, not a claim; the tests check the `lang` attribute. No change.
3. **Paths.** The Test column mixes full paths, partial paths
   (`hindi-screens.test.ts`, `roym-hub.spec.ts`, `listing/tests.rs`) and
   bare paths without backticks (ML-20 to ML-24 and ML-29). Step 7 writes
   every path in backticks, from the repository root, once per row.
4. **Row order.** ML-29 sits between ML-25 and ML-26 (`change.md:130`).
   Step 7 moves it after ML-28 within the table, or keeps it under
   "Search" with ML-20 to ML-24. Decision D-B6-6: under "Search", after
   ML-20, because the table is grouped by topic.
5. **Living docs that are false.**
   - `docs/system-architecture.md:921`: "`product.unit`, `payment.methods`,
     quote `terms.payment_methods`, payment acknowledgement `method`, and
     SynOrg settings `categories` must strictly be valid codes". The code
     checks only the form `[a-z0-9-]` (`listing::valid_token`,
     `crates/roym_core/src/listing.rs:281`, called at `:309`, `:340`,
     `:348`; `transaction.rs:360`, `:414`; `payment.rs:138`;
     `directory.rs:132`). No field is checked against the taxonomy (the
     change doc rejects that, `change.md:201`).
   - `docs/system-architecture.md:1362`: "the Hub provides pickers and
     displays translated labels". A label is not a translation of
     anything; it is the label of the code in the reader's language.
   - `docs/system-requirements-spec.md:1466` to `:1468`: "The architecture
     must support internationalisation (i18n). This allows local community
     clusters to use translated interfaces." It is in an "Envisioned"
     block, but it is built.
   - `docs/system-requirements-spec.md:1461` to `:1464`: the "built" text
     does not name search in every script, contact search, or name sorting
     by script (B4b).
   - `traceability-matrix.md:68`: contradicts itself (Q5) and names no
     Playwright test and no `data_db` text-search test.
   - `docs/roym-integrated-experience-spec.md`: no section says that the
     Hub speaks more than one language, that categories are codes with
     labels, or that records carry a language tag. Only search
     (`:776` to `:779`) and the `agreement-receipt` row (`:476`) mention
     languages.
6. **Stale text in planning docs.**
   - `change.md:503`: the spelling test data line says "owner check
     pending". The check was done 2026-11-13 (`spelling-pairs.tsv:4`). The
     source of record is now the crate copy,
     `crates/text_lang/src/search_key/tests/spelling-pairs.tsv`; the copy
     in this folder is the same file (`diff` is empty).
   - `change.md:512` to `:534`: 14 of 19 tasks are unticked, though their
     work is done.
   - `task.md:5`: "No slice is planned yet." (False since S0.) The
     milestone status line itself belongs to T8 (Q8); B6 only removes the
     false sentence.
   - `task.md:122`: the S0 result says the owner check of the spelling file
     is "still to do".
   - `task.md:217`: the B5 result line sits above the "### B5" heading
     (`:219`). Step 7 moves it under the B5 bullets.
7. **A type error in the e2e helpers.** `hub-helpers.ts:3` imports the type
   `Locator` from `./fixtures`, which exports only `expect`, `Browser` and
   `Page` (`fixtures.ts:3` to `:4`). Playwright strips types, so nothing
   fails today; `tsc` would. B4b's code review saw it and ruled it "not a
   B4b item". B6 uses `Locator` in new helpers, so Step 3 adds `Locator` to
   the type export of `fixtures.ts`.
8. **Missing backlog rows for open questions of the change doc.** The
   change doc leaves four things open or later, and the backlog has no row
   for them: a bundled Indic font (`change.md:507`), the code system for
   `Area::Named.code` (`:506`), native digits as a user option (`:508`),
   and catalogs for languages other than English and Hindi (`:90`). Step
   7 adds them (section 8). Right-to-left is already in the bidi row
   (`deferred-backlog.md:547`); Step 7 adds "and the layout is not checked
   for it" to that row.
9. **Bob's directory exists before story 40.** Story 30 makes Bob's node a
   SynOrg directory, "Bob's guild", with category `gardening`
   (`demo/30-roym-client.sh:80` to `:82`), and Alice's node already asks
   it with `directory.find` across nodes (`:147` to `:149`). So
   `ensureOwnSynOrg` (`hub-helpers.ts:65`) on Bob's page finds a SynOrg
   and does not issue a credential (`:72` to `:93` run only when there is
   none). A publish needs a membership credential for the category
   (`:84` to `:86`, the comment). Step 3 adds a helper that issues one.
10. **How the countersignature happens.** Alice's `agreement.accept`
   (`crates/roym_transaction/src/app/agreement_ops.rs:113`) signs her half
   and sends it as a card (`:157`). Bob's node signs its half in
   `maybe_countersign` (`:451`), called from `transaction.sync`
   (`crates/roym_transaction/src/app/sync.rs:527`). The Hub calls
   `transaction.sync` each time it draws a thread (`messages.ts:406`). So
   Bob's thread must be drawn after Alice's half arrives. Bob keeps the
   conversation open; the Hub draws it again when the event feed reports
   the new message (as in story 20, `demo/20-roym-chat.spec.ts:75` to
   `:80`).

## 3. What B6 meets

B6 adds no criterion. It checks all 29 and proves the end-to-end path of
three of them in a browser, on two nodes:

| Criterion | What B6 adds | Where |
| --- | --- | --- |
| ML-7, ML-8 | A Hindi reader finds another node's English listing by the Hindi label of its category, and the hit shows the category in Hindi. | story 40, new test (Step 4) |
| ML-4 | The quote total shows `₹1,00,000.50` in a Hindi Hub, from real records. | story 40, new test |
| ML-18 | Alice's signed half says `viewed_lang = "en"`, `viewed_via_mt = false`, read back through `agreement.verify`; the pair is `complete`. | story 40, new test |
| ML-24 | The contact list is sorted by script, with the Hub's own script first. | `screens/contacts.test.ts` (Step 2) |

All other rows: B6 corrects the Test column (section 2.2, points 1 to 3)
and adds the missing deviations (Q7). The table in section 7.3 is the
checklist.

**Where decisions and deviations are written.** Every D-B6-n and owner
answer goes into a new list "Decided <date>, close-out" at the end of the
change doc's section "Agreements across languages" (the booking test) and,
for the doc-only decisions, into `## Deviations`. Use the date of the
commit that makes the decision (B3a P14).

## 4. Behaviour-change audit

### Wire changes

None. B6 changes no Rust code, no WIT, no method, no record and no stored
data.

### Behaviour changes

| # | Before | After |
| --- | --- | --- |
| 1 | `fixtures.ts` exports the types `Browser` and `Page` | also `Locator` (test code only) |
| 2 | `publishListing` returns nothing | it returns the listing id it saved (test code only; the old callers ignore it) |
| 3 | Story 40 has one test | two tests; the demo takes longer (section 7.5) |

### Stays the same

- Every Hub screen, every Hub string in `en.ftl`, `hi.ftl` and the taxonomy
  label files (unless the owner asks for changes in Q6 (a)).
- The main Playwright suite (`playwright.config.ts`) and every test in it.
- Stories 10, 20 and 30, and the first test of story 40.
- Every Rust crate; so every Roym component and its size.

## 5. Decisions this plan makes

**D-B6-1. The booking test is a second test in story 40 (Q1 (a)).** Its
title is "Roym languages story: Alice, in Hindi, books Bob's English
listing". It uses `test.describe.configure({ mode: 'serial' })`, which the
file already has (`40-roym-languages.spec.ts:12`). It opens Bob's page with
`newSession(browser, testInfo)` (`demo/steps.ts:78`), as story 20 does, and
closes it in `finally`. Each step is a `step(page, "Alice's screen: ...")`
or `step(bobPage, "Bob's screen: ...")`, so the presenter mode shows who
acts.

**D-B6-2. The listing is in `gardening`, on Bob's own directory.** Bob's
guild already serves `gardening` (section 2.2, point 9). Alice's own
directory also holds Hindi `gardening` listings from the first test, so the
test finds Bob's listing by its id, never by its title alone (B2 R4): the
hit is `.search-hit[data-listing-id="<id>"]` (`directory.ts:356`), and the
`directory.find` answer (read with `page.waitForResponse`, as
`roym-hub.spec.ts:1329` does) holds the id.

**D-B6-3. Every check in Alice's Hindi Hub is by message id.** Use
`hubMessage('hi', id)` (`hub-helpers.ts:186`) for every label and button.
For a message with an argument, compare the text before the first `{`, as
`directoryAddedStatus` does (`:170`) (B4b P12). Never compare with English
text on Alice's page.

**D-B6-4. A run tag in every text.** `Date.now().toString(36)` in the
listing title, the request description and the quote scope, so the story
can run again on the same nodes (story 20 does this, `:15`).

**D-B6-5. The test reads Alice's signed half through the node.** After the
pair is `complete`, it calls `agreement.list` on Alice's page with
`rpcCall` (`hub-helpers.ts:23`), takes the agreement whose
`terms.scope` holds the run tag, and calls `agreement.verify` with
`{"envelope": <counterparty.envelope>}` (`agreement_ops.rs:408`,
`VerifyInput` at `crates/roym_core/src/method.rs:49`). It checks the
verdict's payload: `viewed_lang == "en"`, `viewed_via_mt == false`. Check
the exact shape of `RecordVerdict` in the code before you write the
assertion.

**D-B6-6. The Test column format.** Every path in backticks, from the
repository root. A Vitest or Playwright title in double quotes, exactly as
in the file. A Rust test as `` `fn_name` ``. A later part as "Later:
<what> (<milestone>)", with the deviation it comes from. ML-29 moves under
"Search", after ML-20.

**D-B6-7. No product change.** If a test of Step 4 shows a product bug,
stop and tell the owner. Do not fix it in B6 without an owner answer; a fix
may need its own slice.

## 6. Step-by-step changes

Each step builds and its tests pass. Paths are from the repository root.
Keep the shell at the repository root.

### Step 1 — Commit this plan

Done in the planning session: this file, the B6 row of
[status.md](status.md), and the ML-28 entry in the change doc's
`## Deviations` (Q7, section 9).

### Step 2 — The contact sort unit test

1. `crates/roym_web/ui/src/screens/contacts.test.ts`: a new
   `describe("renderContacts order", ...)` with two cases. Use the shared
   fake `/rpc` (`src/testing/stub-rpc.ts`) as the search test at `:40`
   does. `contacts.list` answers three contacts in this order: `Sita`,
   `राम`, `Ram`.
   - "lists Latin names first in an English Hub": the drawn rows are
     `Ram`, `Sita`, `राम`.
   - "lists Devanagari names first in a Hindi Hub": activate Hindi as the
     other screen tests do (find the helper with
     `git grep -n "activateWith\|initLanguageWith" crates/roym_web/ui/src/screens/*.test.ts`);
     the rows are `राम`, `Ram`, `Sita`.
   - Assert each row's text with `!`, never `?.` (B2 R3).
   - Prove the test can fail: change `sortByName` at `contacts.ts:64` to
     keep the input order, run the test, see both cases fail, and undo.
2. `cd crates/roym_web/ui && npm test` (redirect to a file).
3. Commit (`test(roym-web): contact list order by script`).

### Step 3 — Test helpers

All in `crates/substrate/tests/e2e/`.

1. `fixtures.ts:4`: `export type { Browser, Locator, Page } from '@playwright/test';`
   (section 2.2, point 7).
2. `hub-helpers.ts`, `publishListing` (`:106`): return the listing id. It
   is the `lst_...` text that `.listing-save-result` already shows (`:123`).
   The id is `content_digest("lst_", ...)` (`derive_listing_id`,
   `crates/roym_core/src/listing.rs:419`); read the digest alphabet in
   `content_digest` and write the regular expression from it (for example
   `/lst_[A-Za-z0-9_-]+/`). The id is derived from the issuer and the slug,
   so the run tag in the title must also reach the slug: check how the
   editor makes the slug. The return type becomes `Promise<string>`. The
   existing callers ignore it; change none of them.
3. `hub-helpers.ts`: a new helper `issueOwnCredential(page, category)`. It
   calls `session.whoami` and then `credential.issue` with
   `member_did`, `categories: [category]` and `expires_at_secs` one year
   ahead, and asserts a `record_id`. Move the same five lines out of
   `ensureOwnSynOrg` (`:89` to `:95`) into it and call it there, so there is
   one copy (AGENTS.md "Look for an existing helper").
4. `hub-helpers.ts`: two new helpers for the transaction forms, each with
   a short doc comment:
   - `sendRequest(page, description, category, tag = 'en')`: clicks
     `.toggle-request-form`, fills `.request-desc-input`, calls
     `pickCategory(page.locator('.request-cats-input'), category, tag)`,
     clicks `.send-request-button`, and waits until `.request-form` is
     hidden (as `roym-hub.spec.ts:996` does).
   - `sendQuote(page, requestText, quote: { scope, currency, amount, payee })`:
     finds the request card's message with `threadMessage(page,
     requestText)` (`:161`), clicks its `.quote-request-button`, fills
     `.quote-scope-input`, `.quote-currency-input`, `.quote-amount-input`
     and `.quote-payee-input`, selects `at-giver` in `.quote-where-select`
     (so no address is asked), clicks `.submit-quote-button`, and waits
     until `.quote-form` is gone. Class names: `messages.ts:769` to `:851`.
   Do not change tests 26 and 29 of `roym-hub.spec.ts` to use them; that
   is a clean-up outside B6.
5. `npx tsc --noEmit -p crates/substrate/tests/e2e` must report no error in
   `hub-helpers.ts` or `fixtures.ts`. Write any other error it reports
   into the PR text, and do not fix it.
6. `mise run test:e2e` (redirected): the main suite still passes.
7. Commit (`test(e2e): helpers for a request, a quote and a credential`).

### Step 4 — The booking test in story 40

`crates/substrate/tests/e2e/demo/40-roym-languages.spec.ts`. Read the env
with `requireEnv(env, key, SCRIPT)` for `ALICE_HUB_URL`, `ALICE_KEY`,
`ALICE_ADDR`, `BOB_HUB_URL`, `BOB_KEY`, `BOB_DID`, `BOB_ADDR` and
`BOB_DIRECTORY_DID`. Steps, in order:

1. **Bob's screen: sign in.** `bobPage.goto(bob.hub)`,
   `loginWithDelegatedKey(bobPage, bob.key)`. His Hub stays English.
2. **Bob's screen: publish an English listing about garden work.**
   `ensureOwnSynOrg(bobPage, bob.directoryDid, 'gardening', 'en')`, then
   `issueOwnCredential(bobPage, 'gardening')`, then
   `const listingId = await publishListing(bobPage, \`Hedge trimming ${runTag}\`, 'gardening', bob.directoryDid, 'en')`.
   The text language select keeps `en`.
3. **Alice's screen: sign in and switch the Hub to Hindi.**
   `loginWithDelegatedKey(page, alice.key)`, `chooseHubLanguage(page,
   'hi')`. A new browser context starts in English (the comment at
   `:14` to `:16`).
4. **Alice's screen: find Bob's listing by the Hindi name of its
   category.** Directory tab (`hubMessage('hi', 'tab-directory')`). Add
   `bob.directoryDid` as a source and wait for
   `directoryAddedStatus('hi')` (story 40 does this at `:93` to `:97`).
   `pickCategory(page.locator('.search-categories'), 'gardening', 'hi')`.
   Start `page.waitForResponse` for `directory.find`, click the search
   button (`hubMessage('hi', 'directory-search-button')`, `exact: true`).
   Assert:
   - the answer's `result.hits` holds `listingId`;
   - `.search-hit[data-listing-id="<listingId>"]` has count 1;
   - its `.hit-title` has the text `Hedge trimming <runTag>` and
     `lang="en"`;
   - its `.hit-categories` contains `hubMessage('hi',
     'taxonomy-category-gardening')`.
5. **Alice's screen: ask how to reach the listing owner.** In that hit,
   click `.hit-engage`. `.hit-engage-out` contains `bob.addr`.
6. **Alice's screen: send Bob a request in Hindi.** `openMessages(page,
   'hi')`, `openConversation(page, bob.addr, 'hi')`, then
   `sendRequest(page, \`बगीचे की बाड़ काटनी है ${runTag}\`, 'gardening', 'hi')`.
   The request card (`.thread-messages .card-request`) with that
   description is visible.
7. **Bob's screen: read the request and send a quote in rupees.**
   `openMessages(bobPage)`, `openConversation(bobPage, alice.addr)`. Wait
   for the request text (timeout 60 s, as story 20's `DELIVERY_WAIT_MS`).
   Its `.request-description` has `lang="hi"`. `sendQuote(bobPage,
   requestText, { scope: \`Trim the garden hedge ${runTag}\`, currency:
   'INR', amount: '100000.50', payee: bob.did })`.
8. **Alice's screen: read the quote in Hindi, with the price in rupees.**
   Wait (60 s) for the quote card with the scope text. Its `.quote-total`
   has the text `` `${hubMessage('hi', 'terms-total').split('{')[0]}₹1,00,000.50` ``
   (check the exact space after `कुल:` in `hi.ftl:99`). The scope text has
   `lang="en"`.
9. **Alice's screen: accept the quote.** Click
   `hubMessage('hi', 'messages-accept-button')` in that card. No
   `.accept-mt-box` appears (she reads the original). A
   `.card-agreement-receipt` (`cards/templates/agreement_receipt.ts:57`)
   appears in her thread.
10. **Both screens: the agreement is complete.** Bob keeps his thread
    open; wait for a `.card-agreement-receipt` there (60 s). Then poll
    `agreement.list` on Alice's page with `expect.poll` (60 s) until the
    agreement with the run tag has `pair.state == "complete"`. Then the
    D-B6-5 check of `viewed_lang` and `viewed_via_mt`.

Then:

- Run `mise run demo` (redirected) and confirm all stories pass. Write the
  run time of story 40 before and after into the PR text.
- If step 10 fails because Bob's node does not countersign, stop and tell
  the owner (D-B6-7).
- Commit (`test(demo): a Hindi reader books an English listing`).

### Step 5 — Demo doc

`docs/demo/demo.md`, section "R3b The Hub in Hindi" (starts at `:1262`):
add one paragraph after the search part (`:1279` to `:1281`), in the style
of the section: Bob publishes an English garden listing; Alice, in Hindi,
finds it by category, messages Bob, sends a request, and accepts his quote
in rupees; both Hubs show the agreement. Add a **Say:** line. Keep the
script line at `:1285`. Commit with Step 6.

### Step 6 — Living docs

Each statement says what the system does now, with no change-doc or slice
ids and no milestone names (docs/README.md rule 1).

1. `docs/system-requirements-spec.md` (`:1459` to `:1473`), by Q5. With
   (a): a new `### [APP-L10N] Localisation` with the built text of
   `:1461` to `:1464` plus one sentence on search in every script (a Latin
   word finds the same word in Devanagari or Tamil; contact search; the
   Hub sorts names by script), and an Envisioned block for the engine
   only. `[APP-A11Y]` keeps only "Accessibility". Delete the stale i18n
   sentence (`:1468`). Use the exact title form of the other `### [APP-*]`
   headings.
2. `docs/planning/traceability-matrix.md:68`, by Q5. With (a): two rows.
   The `[APP-L10N]` row is `Implemented`; its evidence keeps the links of
   today's row and adds `crates/substrate/tests/e2e/tests/roym-hub.spec.ts`,
   `crates/substrate/tests/e2e/demo/40-roym-languages.spec.ts`,
   `crates/data_db/src/sqlite/text_index/tests/search.rs` and
   `crates/roym_web/ui/src/names.test.ts`. Its target milestone is the
   one the matrix uses for other M8 rows (check how `[PRD-FED]` writes
   it). The `[APP-A11Y]` row stays `Envisioned`, with no localisation
   evidence.
3. `docs/system-architecture.md:921`: "must strictly be valid codes"
   becomes "must be well-formed codes of `[a-z0-9-]`; no field is checked
   against the taxonomy, so a node accepts a code that a newer taxonomy
   adds". `:1362`: "displays translated labels" becomes "shows the label of
   each code in the reader's language, or the code itself when the
   taxonomy has no label for it".
4. `docs/roym-integrated-experience-spec.md`: a new `### Languages` under
   "Technical design", after `### Search` (`:768` to `:795`), for its
   reader (developers and the owner). Five short points: the Hub speaks
   English and Hindi, and the choice stays on the device; categories,
   units and payment methods are codes, and each reader sees labels in
   their own language; each record says the language of its text, and the
   author may add translations, but the original binds; a chat message
   may name up to 3 languages; there is no translation engine yet, and the
   Hub shows no translate action. Link ADR-0029. In the records table
   (`:470` to `:476`), no change: the `agreement-receipt` row already says
   what each party read.
5. `docs/planning/deferred-backlog.md`: section 8.
6. Check-only, change if false: `docs/developer-guide.md` "Hub text and
   languages" (`:1760` to `:1792`), `docs/TERMINOLOGY.md:70` to `:78`. B6
   adds no helper the guide names.
7. Commit (`docs: describe the multilingual Hub and records`).

### Step 7 — Change doc, task.md, status.md

1. **Test column**, every row, by D-B6-6 and section 2.2 points 1 to 3. Run
   `git grep -n` on every name you write; paste the command list into the
   PR text (B3a R2, B3b R3).
2. **ML-24** names the new `contacts.test.ts` cases and
   `roym-hub.spec.ts` "contact search finds a name in another script".
   **ML-7, ML-8, ML-18** add the new story 40 test.
3. **Deviations**: add (a) ML-23: "shop name" has no record before the
   shop (M08D); a contact name is stored as written
   (`scenario_234_contacts_list_text_finds_names_across_scripts_parity`
   stores a decomposed name and reads it back); a profile display name is
   free text in a signed record and is not searched; (b) the booking test
   ends at the agreement, with no slot (Q2); (c) the request from a
   directory hit does not name the listing (Q3). The ML-28 entry is in
   from Step 1.
4. **Decided <date>, close-out**: D-B6-1 to D-B6-7 and the owner answers.
5. **Open questions** (`:503`): the spelling line says the owner check was
   done on 2026-11-13 and that the data is
   `crates/text_lang/src/search_key/tests/spelling-pairs.tsv`. "Named area
   codes" and "Font size" stay open and name their backlog rows.
6. **Tasks** (`:512` to `:534`): tick each task whose work is merged.
   Leave none unticked; if one is not done, it is a deviation and a
   backlog row.
7. **Front matter**: add every file of section 9 that is not in
   `living-docs-touched`. `demo:` stays story 40.
8. **Q6**: when the owner has reviewed the 13 B5 Hindi messages, put the
   date in the B5 row of status.md (Evidence column). Make any change the
   owner asks for in `hi.ftl`, run `npm test` in `crates/roym_web/ui`.
9. **`status: done`** last, after Q6 and Q7 are answered and every row is
   checked. Run `cargo xtask check-change-docs`.
10. **task.md**: remove "No slice is planned yet." (`:5`); fix the S0 line
    (`:122`, "the owner checked it on 2026-11-13"); move the B5 result
    (`:217`) under the B5 bullets; add a short "B6 result" line under B6
    (test counts, story 40 run time, no component change).
11. **status.md**: B6 row, by the T4 to T7 steps.
12. The import clean-up pass: B6 edits no Rust file; check the imports of
    the edited `.ts` files (no unused import; `Locator` from
    `./fixtures`).
13. `mise run verify` with no `--skip`, as a background job, sandbox off.
14. PR text: section 10's checklist.
15. Commit (`docs(planning): close out roym-multilingual`).

## 7. Tests

### 7.1 Hub unit tests

| File | Case | Proves |
| --- | --- | --- |
| `crates/roym_web/ui/src/screens/contacts.test.ts` | "lists Latin names first in an English Hub" | ML-24 for the contact list, in English |
| same | "lists Devanagari names first in a Hindi Hub" | ML-24, the Hub's own script first |

### 7.2 Rust

None new. Every Rust test named in the change doc exists and is not
`#[ignore]` (checked on `22331a3f`).

### 7.3 Playwright

- Story 40, new test (Step 4). It needs both demo nodes, so it runs only in
  `mise run demo` and `npm test`, not in the main suite.
- Main suite: unchanged, must still pass after Step 3.

The criteria checklist for Step 7 (each row: the tests that prove it after
B6, and its later part):

| Row | Proof after B6 | Later part |
| --- | --- | --- |
| ML-1 to ML-3, ML-5, ML-6, ML-10, ML-11, ML-13 to ML-17, ML-19, ML-22, ML-25, ML-27 | as today, with full paths | — |
| ML-2, ML-7, ML-8, ML-28 | as today, with the real titles of section 2.2 point 1 | ML-28: widget labels, extension step names (M08C C4) |
| ML-4 | as today; plus story 40 booking (the `₹1,00,000.50` total) | — |
| ML-7, ML-8 | plus story 40 booking (a Hindi reader finds another node's listing by category) | — |
| ML-9 | `screens/listings.test.ts:88`, `taxonomy.test.ts:23`, `hindi-screens.test.ts:329`, `listing/tests.rs` `unit_code_rules` | a reader's view of another node's unit (M08D) |
| ML-12 | `record_text.test.ts`, `roym-hub.spec.ts:1360`; story 40 removed | — |
| ML-18 | as today; plus story 40 booking (Alice's half, read back) | — |
| ML-20, ML-29 | as today, with full paths | — |
| ML-21 | as today; story 40 named as "Latin to Devanagari only" | — |
| ML-23 | as today, plus `roym-hub.spec.ts:1477` with its full path | shop name (M08D) |
| ML-24 | `names.test.ts`, `screens/contacts.test.ts` (new), `screens/groups.test.ts:596`, `roym-hub.spec.ts:1477` | — |
| ML-26 | as today, with the per-file cases of section 2.2 point 2 | widget answer (M08C C4), opinion (M08G), offering, want and storefront display (M08D) |

### 7.4 Not tested

- A slot booking from the Hub (Q2): the Hub has no slot field.
- The listing named in a request made from a hit (Q3).
- Screen reader output (ML-5 checks the `lang` attribute only).

### 7.5 Size and time risks

| What | Now | Risk |
| --- | --- | --- |
| Demo run time | `playwright-demo.config.ts:17` `globalTimeout: 900_000`, `:18` `timeout: 180_000` per test | The new test has three cross-node deliveries and one countersignature. On the demo's 1-second outbox tick each takes seconds, but a slow CI machine may need more. If the test takes more than 120 s, report it in the PR text; do not raise a timeout without an owner answer. |
| `40-roym-languages.spec.ts` | 104 lines | +120. TypeScript is not under the file-length check (`xtask/src/file_lengths.rs:398` lists only `*.rs`). |
| `hub-helpers.ts` | 227 lines | +50 |
| Hub bundle, Roym components | — | unchanged: no Hub source and no Rust change |

### 7.6 How B6 avoids the kinds of findings S0 to B5 had

- **Wrong names in the Test column** (B3a R2, B3b R3): Step 7.1 runs
  `git grep` on every name; section 2.2 lists the seven wrong titles found
  while planning.
- **Rows that prove less than they say** (B2 R8, B4a R4, B5 R3): section
  2.2 point 2 and the 7.3 checklist; ML-12, ML-21 and ML-26 are reworded to
  what their tests show.
- **Criteria met in part marked "done"** (B3a P7, B4b P13, B5 P11): Q7,
  one deviation and one backlog row for each later part.
- **Living docs left false** (B2 P6, B2 R9, B5 R1): section 2.2 point 5
  names each false sentence with its line.
- **Titles instead of ids in e2e** (B2 R4): D-B6-2.
- **English checks on a Hindi page** (B4b P12): D-B6-3.
- **Copied helpers** (B4b R4, B3a R14): Step 3 moves the credential code
  into one helper and adds the form helpers to `hub-helpers.ts`.
- **`?.` in tests** (B2 R3, B3a R4, B3b R9): every element is asserted
  with `!` or a count.
- **Tests that cannot fail** (B4b P2, B5 R3): Step 2.1 breaks the sort and
  watches the test fail.
- **Wrong dates** (S0 R2, B3a P14): dates from the commits.
- **Status ahead of facts** (B1a R7, B2 round 5): status.md says "Merged"
  only after the merge; "verify passed" only after a full run with no
  `--skip`.
- **The open Hindi review** (B5 R6): Q6, date in the B5 status row.
- **Planning ids in test names** (B2 R5): the new test title and step
  names have no `ML-`, `B6`, `Q` or `D-` id.

## 8. Deferred backlog

Changes to [deferred-backlog.md](../../deferred-backlog.md):

**Change:**

- `:515` "Accessibility (WCAG 2.1 AA) & localisation/i18n": split. The
  localisation part moves to "Recently resolved" ("Resolved in M08B: the
  Hub has English and Hindi catalogs, local formats, codes with labels,
  language-tagged records and chat messages, and search in every script").
  The accessibility part stays, with its target "later phase" and the
  requirement id of Q5.
- `:547` "Fluent bidi isolation is off in the Hub": add that the Hub
  layout is not checked for a right-to-left language either
  (`change.md:89`).

**New rows:**

| Item | Why deferred | Target | Source |
| --- | --- | --- | --- |
| A request made from a directory hit does not name the listing | The hit shows only the owner's address; the listing id box stays empty (Q3) | TBD | `crates/roym_web/ui/src/screens/directory.ts:437` |
| Widget labels and extension step names from the catalog | Widgets and extensions do not exist yet | M08C C4 | change doc ML-28 |
| A reader's view of another node's product unit | Directory hits do not carry product details yet | M08D | change doc ML-9 |
| A shop name in any script, with search keys | No shop record yet | M08D | change doc ML-23 |
| Translation hook points for widget answers, opinions, and offering, want and storefront display | Those records do not exist yet | M08C C4, M08G, M08D | change doc ML-26 |
| No bundled font for Indic scripts | System fonts were enough on the test devices; a font also needs `font-src` in the CSP | TBD | change doc "Open questions" |
| No code system for `Area::Named.code` | Not decided (PIN code or the Local Government Directory) | TBD | `crates/roym_core/src/area.rs:29` |
| Native digits are not a user option | Latin digits by default; users not asked yet | TBD | `crates/roym_web/ui/src/i18n/format.ts` |
| String catalogs only for English and Hindi | Adding one is a data change only (a new `.ftl` file and taxonomy labels) | TBD | `crates/roym_web/ui/src/i18n/` |

Check first with `git grep` that none of these rows exists in another
form; extend a row rather than add a second one. No new `TODO` or `FIXME`
marker is written.

## 9. Living docs to update at close-out

- `docs/system-requirements-spec.md:1459` to `:1473` (Q5).
- `docs/planning/traceability-matrix.md:68` (Q5).
- `docs/system-architecture.md:921`, `:1362`.
- `docs/roym-integrated-experience-spec.md`: new `### Languages` after
  `:795`.
- `docs/demo/demo.md`, "R3b The Hub in Hindi" (`:1262`).
- `docs/planning/deferred-backlog.md` (section 8).
- Check only: `docs/developer-guide.md:1760` to `:1792`,
  `docs/TERMINOLOGY.md:70` to `:78`.
- Change doc (this plan commit): one new `## Deviations` entry: "ML-28 is
  met in M08B for taxonomy attribute names and choices only. Widget labels
  and extension step names come from the string catalog in M08C C4, when
  widgets and extensions exist." (task.md section 5).
- Change doc (Step 7): the rest of section 7.3, the deviations of Step
  7.3, the decisions, the tasks, the open questions, `status: done`.

## 10. Exit criteria

- Every test of section 7 exists and passes; `mise run demo` passes with
  the new story 40 test.
- Every name in the change doc's Test column is found by `git grep` in the
  file the row names (the command list is in the PR text).
- Every row with a later part names it, and has a deviation and a backlog
  row (Q7).
- The change doc has `status: done` and `cargo xtask check-change-docs`
  passes (after Q6).
- The living docs of section 9 are updated; `git grep -n "translated
  labels\|must strictly be valid codes" docs/system-architecture.md` finds
  nothing; the stale i18n sentence of the requirements is gone.
- The backlog of section 8 is updated.
- `mise run verify` passes with no `--skip`.
- The PR text has: the CLA line; "no product code changes"; the story 40
  run time before and after; the `git grep` list of Step 7.1; the owner's
  Hindi review date (Q6); any `tsc` error found in Step 3.5 and left
  alone; test counts.

## 11. What B6 does not do

- No change to a Hub screen, a Hub string, a Rust crate or a WIT file
  (D-B6-7). So no component size and no Hub size to measure.
- No slot booking in the Hub (Q2).
- No change that carries the listing from a hit into a request (Q3).
- No new search test (Q4).
- No milestone close: the M08-overview row, the roadmap row and the
  task.md status line stay for T8 (Q8).
- No clean-up of tests 26 and 29 of `roym-hub.spec.ts` to use the new
  helpers.
- No widget or extension labels, no hook points for later records, no
  shop names (M08C, M08D, M08G).
