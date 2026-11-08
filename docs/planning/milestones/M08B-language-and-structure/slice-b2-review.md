# M08B slice B2 — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-b2-implementation-plan.md](slice-b2-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-07

Checked on `feat/m08b-b2-taxonomy` at `ca4d54dc`, in the main checkout (not a
worktree: `.git` is a directory). `git pull` first failed in the sandbox
(GitHub was not allowed). With the sandbox off it said "Already up to date".
The only commit after the plan's base (`c44e055c`) is the plan itself, so the
code facts below hold on `main`.

**Who reviewed.** This session runs on Claude Sonnet 5.5. The `status.md` row
says the plan was written by Claude Opus 5.5. They are different models, as
the prompt says.

**Change docs.** The prompt lists one change doc,
[roym-multilingual](../../changes/roym-multilingual/change.md). `task.md`
names no other change doc for B2 except [demo-stories](../../changes/demo-stories/change.md)
(the "Demo" paragraph, story 40). The plan already says so (section 0), and
also reads [roym-shop](../../changes/roym-shop/change.md) for SH-5 and SH-11.
I agree with both: B2 meets no criterion of those two docs and writes nothing
there. ML-2, ML-7, ML-8, ML-9 and ML-28 are all in roym-multilingual, so every
B2 decision and deviation belongs there (plan section 3 says the same).

**Summary:** 0 blocker, 2 major, 7 minor, 9 items. 9 Open.

Limits of this review: I read the plan, the docs, the code and the e2e specs,
and I counted the taxonomy lists by hand. I did not run any plan step, the Hub
tests or `mise run verify`.

### What was checked and found correct

- **File and line links.** Spot-checked and right: `listing.rs:22`, `:24`,
  `:27` to `:36`, `:265` (`valid_token`), `:289` to `:296`, `:320` to `:334`,
  `:519`; `transaction.rs:441` to `:447`; `payment.rs:38`, `:137`, `:159`;
  `directory.rs:24`, `:131` to `:135`, `:248`; `membership.rs:203` to `:205`,
  `:473` to `:476`; `credential_ops.rs:83`; `search_ops.rs:94` to `:98`,
  `:242`, `:284` to `:286`; `listings.ts:231`, `:268`, `:282`; `messages.ts:317`;
  `directory.ts:215`, `:311` to `:313`; `synorg.ts:64`; `editor.ts:183`, `:206`,
  `:262`; `request.ts:53` to `:56`; `synorg_members.ts:121` to `:131`, `:256`;
  `payment_acknowledgement.ts:65` to `:69`; `quote.ts:28`; `search.ts:10`;
  `i18n.ts:16` to `:20`; `hindi-screens.test.ts:80`, `:316`, `:370`, `:385`;
  `latin-words.ts:24`, `:47`; `hub-helpers.ts:99`; `editor.test.ts:48`;
  roymctl `transaction.rs:35`, `booking.rs:94`, `directory.rs:56`;
  `deferred-backlog.md:272` and `:508`; the living-doc lines
  (`system-requirements-spec.md:685`, `:1462`; `system-architecture.md:1206`,
  `:1414`; `traceability-matrix.md:68`; `developer-guide.md:1702`, `:1716`,
  `:1745`; `demo.md:1260`). The N-2 row does link `listing/mod.rs`, a path
  that no longer exists, as point 10 says.
- **Counts.** 14 groups and 91 children (11+6+6+7+4+6+6+6+6+8+8+9+5+3), so 105
  categories, inside owner answer 5's 50 to 150. 17 units, 6 payment methods,
  6 attributes. No code appears twice inside one list. `transaction.rs` is 776
  lines, `listing.rs` 547, `payment.rs` 299, `directory.rs` 356.
- **Point 6 (fixtures use codes).** I searched every Rust, JSON, TypeScript
  and shell file for `unit`, `methods`, `payment_methods`, `method` and
  `categories` values. Every unit, payment method and SynOrg category is
  already `[a-z0-9-]` (`hour`, `cash`, `card`, `sepa`, `cycling`, `repair`,
  `services`, `outdoor`). The only non-code values are the two refusal tests
  (`Bad Cat`, `UPPERCASE`), which are about categories on records and do not
  change. `transaction/tests.rs:170` uses `m0` to `m16`, which are codes.
- **Renamed variants.** `git grep` finds `UnitTooLong`, `PaymentMethodTooLong`
  and `InvalidPaymentMethod` only in `crates/roym_core` (the enum, the checks
  and `transaction/tests.rs:173`, `payment/tests.rs:96`). No Hub code, no
  `method_contract` row and no e2e file reads these error texts.
- **`normalize_category` keeps hyphens.** It keeps `[a-z0-9-]` and drops the
  rest (`directory.rs:248`). So `bicycle-repair` and `two-wheeler-repair`
  survive the credential check, the search filter and the scope check
  (`credential_ops.rs:83`, `search_ops.rs:285`, `membership.rs:204`, `:475`).
- **No new wire surface.** The five checks keep their typed inputs and their
  `-32602` path. `crates/roym_web/tests/method_shapes/*.json` hold shapes
  (`"unit": "string"`), not values, so they do not change.
- **Both builds.** `taxonomy` is a new module with a `pub const BASE_JSON` and
  derive-only types. No service calls `parse`, so a guest does not link the
  file (a `pub const` that nothing uses is not emitted). `roym_core` already
  has `serde`, `serde_json` and `thiserror`. The five checks run in shared
  `roym_core` code, so native and WASM answer the same. The parity fixtures
  already use `hour` and `cash`.
- **Hub claims.** The listing editor has no prefill or edit path, so no stored
  listing is loaded into the new unit list. The Listings and Directory tabs
  are in the fixed tab list (`main.ts:159` to `:160`), so story 40 can open
  them. `tsconfig.json` has `resolveJsonModule`, and with `moduleResolution:
  bundler` a default import of the JSON file type-checks. The path
  `../../../../roym_core/taxonomy/base.json` from `src/taxonomy/` reaches
  `crates/roym_core/taxonomy/base.json`. `t()` already falls back to the
  English bundle per message. `codeWord`/`lookupText` are Map-based. The id
  scan (`findMessageIds`) only reads `t("…")` and `messageId("…")` calls, so
  the computed ids of D-B2-5 need no `messageId` call and cannot trip it.
- **Size rules.** `check-file-lengths` has no `.ts` rule, so the TypeScript
  line counts in 7.5 are not gated. `transaction.rs` already uses
  `valid_token` for request categories (`:495` to `:501`), so W3 needs no new
  import and the file does not grow.
- **e2e lines of point 7.** The `cycling` fills at `roym-hub.spec.ts:476`,
  `:506`, `:565`, `:607`, `:645`, `:666`, `:731`, `:792`, `:823`, the two
  `credential.issue` calls (`:490`, `:741`) and `roym-trust.spec.ts:42`,
  `:82`, `:113` are right. One more fill is missing (P1).
- **Behaviour-change audit.** W1 to W5 and H1 to H3 match the code. No spec or
  unit test reads the text of a shown category or payment method (searched
  `hit-categories`, `request-categories`, `credential-covers`,
  `issue-category`, `payment-ack-method`).
- **Criteria and change docs.** ML-7, ML-8, ML-9 and ML-28 are written in
  roym-multilingual, and ML-2 names "taxonomy labels". The plan puts every
  decision and deviation there. Owner answers 3 and 5 are quoted correctly.
- **"How this is shown to an end user".** Four plain bullets for a person and
  three for a developer, right after the goal. Matches AGENTS.md.
- **Backlog and exit criteria.** The four backlog moves of section 8 cover
  what B2 postpones. Exit criteria 1 to 14 cover each step and each
  criterion.

### Views on the owner's questions

(For the owner to decide. I do not answer for the owner.)

- **Q1 (labels in Fluent files).** I would take (a). It reuses the B1a
  fallback, the catalog tests and the owner review flow. (b) forces a second
  fallback and lazy-load path. (c) needs exceptions in the catalog test for
  about 250 ids. Cost of (a): task.md Y2 and a sentence of the change doc say
  "labels per language" in the data file, so both are edited (P7).
- **Q2 (Vite imports the JSON).** I would take (a). One file, no RPC, no
  copy. The shape is checked by Rust, and the Hub reads the same bytes.
- **Q3 (the code rule on five fields).** I would take (a). It is the same
  helper (`valid_token`) and every fixture already passes it. The cost is that
  a stranger's old listing with `unit: "Kilogram"` fails `verify_envelope` on
  a searcher's node. The plan says so (section 4). The product is not
  released, so I accept it.
- **Q4 (own-code choice in the picker).** I would take (a). Without it, owner
  answer 5 ("a group may use its own codes at once") cannot be used from the
  Hub. See P8 for two gaps in the option as specified.
- **Q5 (only children can be picked).** I would take (a). The search matches
  each category exactly (`search_ops.rs:284` to `:286`), so a group tag would
  find nothing. See P3: the filter should still find a group by its label.
- **Q6 (picker in all four forms).** I would take (a). One component, one
  vocabulary everywhere. P1 and P2 are the two places where the plan misses
  how the request form uses the box.
- **Q7 (UI language plus English plus code).** I would take (a). With two
  languages, a Hindi user is fully covered. The gap is for an English user who
  types a Hindi label, which is rare.
- **Q8 (six attributes).** I would take (a). It lets ML-28 have a test and
  costs about 25 more Hindi messages for the owner to read. M08D then adds
  attributes to a format that is already checked.
- **Q9 (a unit is not shown to a reader).** I would take (a). A unit shows on
  no reader screen today, and (b) is a wire change for one line. ML-9 is then
  met in part: say so in the ML-9 row and in the deviation.
- **Q10 (story 40 shows the picker in the editor).** I would take (a). The
  Listings tab exists for Alice. A search would need a published listing. The
  ML-8 Playwright test already covers the search side.

### Findings

#### P1 — A request-form e2e step types categories that the picker cannot take (major)

- **Where:** [slice-b2-implementation-plan.md](slice-b2-implementation-plan.md)
  section 2.2 point 7 and Step 8.2;
  `crates/substrate/tests/e2e/tests/roym-hub.spec.ts:984`.
- **Problem:** Point 7 lists every place that fills a category box. It lists
  nine `cycling` fills in `roym-hub.spec.ts` and three in `roym-trust.spec.ts`.
  It misses test 26 ("messages tab: sending a request posts a card …"), which
  does `page.locator('.request-cats-input').fill('gutters, cleaning')`. After
  B2 `.request-cats-input` is a `div`, so `fill` throws. The two values also
  cannot be picked: `gutters` is not in the taxonomy and `cleaning` is a group
  (Q5 (a)).
- **Failure case:** Step 8 changes the twelve listed steps. `mise run test:e2e`
  then fails on test 26, and `mise run verify` fails at the e2e gate.
- **Fix:** Add `roym-hub.spec.ts:984` to point 7 and to Step 8.2. Replace the
  fill with two picks that exist in the base list, for example
  `pickCategory(page.locator('.request-cats-input'), 'home-cleaning')` and
  `'deep-cleaning'`, so the test still sends a list of two. Then run
  `git grep -n "cats-input\|categories')" crates/substrate/tests/e2e` once
  more at the end of Step 8 to prove no fill is left.
- *Status:* Fixed (b8203efb)

#### P2 — The category picker has no way to clear itself, but the request form is reused (major)

- **Where:** D-B2-6 (the return value `{ root, codes() }`); Step 7.1;
  `crates/roym_web/ui/src/screens/messages.ts:306` (the toggle button) and
  `:358` (`catsInput.value = ""`).
- **Problem:** The request form is built once per open conversation. After a
  successful send, `messages.ts` resets every field (`descInput.value = ""`,
  `catsInput.value = ""`, `listingInput.value = ""`) and hides the form. The
  form is shown again by `.toggle-request-form`. The picker API only has
  `codes()`, so the caller cannot empty the chips.
- **Failure case:** Alice sends a request tagged `plumbing`, then opens the
  form again for a second request. The old chip is still there and
  `codes()` returns `plumbing`. The second request is sent with a category she
  did not choose, and no test sees it.
- **Fix:** Give the picker a `clear(): void` (empties the chips and the
  filter, closes the options), and call it where `catsInput.value = ""` is
  today. Add a `picker.test.ts` case ("clear removes the chips and
  `codes()` is empty") and a `messages.test.ts` case (after a send, the form's
  chips are gone). Update D-B2-6's signature.
- *Status:* Fixed (b8203efb)

#### P3 — Typing a group's name finds nothing (minor)

- **Where:** D-B2-5 (`matchCategories`), D-B2-6 ("When the filter has text,
  only the matching children show").
- **Problem:** `matchCategories` looks at the label and the code of a child
  only. A group is a heading, never an option. A person who types "Health",
  "Education" or "Food" (a group label) matches no child label, so the picker
  says "No category matches" although the group has six to eight children. A
  person who is looking for the area, not the exact trade, is the usual user
  of a picker.
- **Failure case:** In Hindi, typing the group word for "health" shows no
  option. Typing a child word ("योग") works. The test "a group code is never
  returned" passes, so the gap is not tested.
- **Fix:** In `matchCategories`, a child also matches when its group's label
  (UI language or English) or its group's code holds the text. The picker
  shows these children under their group heading. Keep the rule "a group is
  never returned". Add a `taxonomy.test.ts` case ("a group's label finds its
  children").
- *Status:* Fixed (b8203efb)

#### P4 — D-B2-4 says `bundleFor` logs a duplicate id, but it does not (minor)

- **Where:** D-B2-4, last sentence of the "Loading" paragraph;
  `crates/roym_web/ui/src/i18n/i18n.ts:41` to `:45`.
- **Problem:** The plan says "`bundleFor` logs it with `console.error`".
  `bundleFor` calls `bundle.addResource(new FluentResource(source))` and drops
  the returned error list. `catalog.test.ts` also ignores the return value
  (`:78`). Fluent returns an error for a message id that is added
  twice (`allowOverrides` is false), but nothing reads it today.
- **Failure case:** An implementer trusts the sentence, writes no log and no
  check, and a taxonomy id that also exists in `en.ftl` silently keeps the
  first copy. The plan's test "the English bundle and the Hindi bundle add both
  files with no error" is the only guard, and it is written as if the code
  already reported the error.
- **Fix:** Say what to do: in the new catalog test, call `addResource` on the
  string catalog and then on the taxonomy file and expect an empty error list
  from each. Optionally make `bundleFor` log the returned list (one line) with
  a test. Remove the claim that it already logs.
- *Status:* Fixed (b8203efb)

#### P5 — The new `picker.ts` and `picker.test.ts` share names with the language picker (minor)

- **Where:** D-B2-6 (`src/taxonomy/picker.ts`), section 7.1, section 3 table;
  the existing `src/i18n/picker.ts` and `src/i18n/picker.test.ts`.
- **Problem:** `src/i18n/picker.ts` is the language picker, and
  `picker.test.ts` is named in the ML-3 row of the change doc. B2 adds a second
  file with the same base name. The ML-7 row of section 3 names `picker.test.ts`
  without a folder, so a reader of the change doc cannot tell which one is
  meant. CSS classes `.language-picker` and `.category-picker` are fine.
- **Failure case:** `vitest run picker` runs both. A review or the traceability
  matrix that cites `picker.test.ts` is ambiguous.
- **Fix:** Name the new files `category_picker.ts` and
  `category_picker.test.ts` (the `screens/` folder already uses
  `synorg_members.ts`). Update D-B2-6, Steps 5 to 7, section 3, section 7.1,
  section 9 and the change doc rows. The exported function stays
  `categoryPicker`.
- *Status:* Fixed (b8203efb)

#### P6 — Section 9 points at a row that does not exist and misses two that become false (minor)

- **Where:** Section 9, the `docs/system-architecture.md` bullet;
  `docs/system-architecture.md:1206` to `:1209`;
  `docs/system-requirements-spec.md:730`.
- **Problem:** The bullet says "the `listing` and `request` rows near line
  1206". Line 1206 is the `request` card row; there is no `listing` row in
  that table. Two nearby rows become wrong with W3 and W4: the `quote` row
  says payment methods without saying they are codes, and the
  `payment-acknowledgement` row says "an optional method and reference as
  text". After B2 the method is a code. The requirements spec line 730
  ("Product type … with unit, pack size, SKU") does not say the unit is a code.
- **Failure case:** The living-docs close-out (AGENTS.md) leaves a sentence
  that says a payment method is free text, which is false after this change.
- **Fix:** In section 9: change the architecture bullet to name the `request`,
  `quote` and `payment-acknowledgement` card rows and a paragraph near the
  listing text at `:918`/`:1355` for the taxonomy; add
  `system-requirements-spec.md:730` (unit is a code from the base unit list).
- *Status:* Fixed (b8203efb)

#### P7 — task.md Y2 and a sentence of the change doc still say the data file holds the labels (minor)

- **Where:** [task.md](task.md) section 1, row Y2 ("… unit codes,
  payment-method codes, labels per language"); change doc "B. Language-neutral
  structure" ("a data file … with each code, its parent code … and its labels
  per language"); Step 9 and section 3.
- **Problem:** With Q1 (a) the labels are Fluent files in the Hub, not part of
  `base.json`. Section 3 records a deviation, and Step 9 adds a "Decided" list.
  But the old sentences stay in the change doc and in the Y2 row, and Step 9
  edits only the "B2 result line" of task.md. The roym-shop change doc (SH-5,
  "labels per language") also needs M08D to add labels to the Hub taxonomy
  files, not to the data file.
- **Failure case:** A later reader of the change doc reads two statements that
  disagree. M08D plans to put attribute labels into `base.json`.
- **Fix:** In Step 9: rewrite the "B. Language-neutral structure" sentence and
  the Y2 row so they say "codes and structure in the data file, labels in the
  Hub's taxonomy label files", and add one line to the deviation: "M08D adds
  attribute labels to the same label files". If the owner picks Q1 (b) or (c)
  this item changes with it.
- *Status:* Fixed (b8203efb)

#### P8 — The picker spec has three open edges (minor)

- **Where:** D-B2-6; section 7.1 `picker.test.ts`.
- **Problem:**
  1. The own-code option shows when the text is a valid code that is not a
     taxonomy code. It does not say "and not already chosen". Typing the same
     own code twice makes two chips with the same code, and `codes()` returns
     it twice.
  2. The message `category-picker-no-match` is defined, but D-B2-6 never says
     when it shows (no children match and no own-code option?), and 7.1 has no
     test for it. The "no unused message" test only needs one use.
  3. "It works with a keyboard (Tab and Enter) with no extra code" is true for
     the buttons. With an empty filter there are about 105 option buttons
     between the filter and the next field, and Enter in the filter does
     nothing. The no-match and full messages are plain text, so a screen reader
     is not told. WCAG 2.1 AA is a separate backlog row, but the claim in the
     plan is too strong.
- **Failure case:** (1) A request sent with `["my-code", "my-code"]` is valid
  (no duplicate rule), so it is accepted and indexed twice. (2) Two
  implementations of the message differ, and the Hindi sweep cannot check it.
- **Fix:** (1) Offer the own code only when it is not already chosen, and have
  `codes()` return a set in order of choice; test both. (2) Say: the no-match
  message shows when the filter has text and there is no option at all; add a
  test, in English and in Hindi. (3) Change the sentence to "the buttons can be
  reached with Tab"; make Enter in the filter pick the first option when
  there is exactly one; set `aria-live="polite"` on the message line. Add a
  backlog sentence to the existing WCAG row if the owner wants more.
- *Status:* Fixed (b8203efb)

#### P9 — `Taxonomy::validate` does not check two rules that the file format relies on (minor)

- **Where:** D-B2-1 ("a group comes before its children"; two levels; Q5
  (a)), D-B2-3 (`validate`), section 7.2.
- **Problem:** D-B2-1 says order in the file is the order the Hub shows and "a
  group comes before its children". D-B2-3's checks do not test it. They also
  do not catch a top-level category that has no children. Under Q5 (a) only
  children can be picked, so such a category can never be chosen and shows as a
  heading with nothing below it. The Hub's `CATEGORY_GROUPS` builder is not
  specified for a child that comes before its group.
- **Failure case:** Someone adds `{ "code": "plumbing", "parent": "home-repair" }`
  above `home-repair`, or adds a group with no children. Rust passes. The
  Hub either drops the child or shows an empty heading, and no test says why.
- **Fix:** Add two `TaxonomyError` variants (`ParentAfterChild`,
  `GroupWithoutChildren`) and one test each, or write in D-B2-5 that
  `CATEGORY_GROUPS` is built in two passes and that an empty group is not
  drawn, with a `taxonomy.test.ts` case. I prefer the Rust check: it fails
  where the file is edited.
- *Status:* Fixed (b8203efb)
