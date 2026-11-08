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
- *Status:* Fixed (baebc1de)

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
- *Status:* Fixed (baebc1de)

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

## Plan review round 2 — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-08

Checked on `feat/m08b-b2-taxonomy` at `147de113`, in the main checkout. I read
the diff from my round 1 commit (`5ed965d6`) to `HEAD` for the plan, the
change doc, `task.md`, `status.md` and this file, and I re-ran the commands
the new text uses against the code on `main`. I ran no plan step.

**Summary of this round:** 0 blocker, 0 major, 2 new minor items (P10, P11).
Round 1 items: 7 confirmed Fixed (P1, P3, P4, P5, P6, P7, P9), 2 sent back to
Open with a note (P2, P8). **Open now: 4 (P2, P8, P10, P11), all minor.** Fixed in baebc1de; 0 open.

### Round 1 items

- **P1 — confirmed Fixed.** Point 7 and Step 8.2 now name
  `roym-hub.spec.ts:984` and replace the fill with picks of `home-cleaning`
  and `deep-cleaning`. Both are children in D-B2-2. The grep that Step 8.2
  adds does not work (see P10); that is a new item, not a reopening, because
  the spec change itself is right.
- **P2 — back to Open.** `clear()` is in the return type, in the picker
  tests ("`clear()` empties the chips and `codes()` is empty") and in Step 7.1
  (`catsInput.value = ""` becomes `catsPicker.clear()`). The `messages.test.ts`
  case from my fix text is not in section 7.1 (`grep messages.test` on the plan
  finds nothing). Without it nothing fails if `messages.ts` forgets the call.
- **P3 — confirmed Fixed.** D-B2-5 (signature comment and text) and the
  `matchCategories` test ("a group's label finds its children") now say that a
  child also matches by its group's label or code and that no group is
  returned. D-B2-6's "each with its group label before it" still fits.
- **P4 — confirmed Fixed.** The false claim about `bundleFor` is gone. The
  catalog test now calls `addResource` for the string catalog and then for
  the taxonomy file and expects an empty error list each time. (The second
  call must be on the same bundle; the text implies it.)
- **P5 — confirmed Fixed.** `category_picker.ts` and `category_picker.test.ts`
  replace the old names in D-B2-6, Steps 5, 7.1, 7.5, 7.6, section 3 (ML-7 row)
  and section 9. No bare `picker.ts` or `picker.test.ts` is left in the plan
  (the language picker is not mentioned any more).
- **P6 — confirmed Fixed.** Section 9 names the `request`, `quote` and
  `payment-acknowledgement` card rows, adds `system-requirements-spec.md:730`,
  and moves the taxonomy paragraph to the listing text at `:918` and `:1355`.
- **P7 — confirmed Fixed.** `change.md` ("B. Language-neutral structure") and
  `task.md` row Y2 now say that codes and structure are in the data file and
  labels are in `src/i18n/taxonomy/<tag>.ftl`, and that M08D adds attribute
  labels to the same files. Step 9 still tells the implementer to rewrite the
  same sentences (see P11).
- **P8 — back to Open.** Points 1 and 2 are fixed: the own code is offered
  only when it is not already chosen, `codes()` is a set in order of choice,
  and the no-match message has a trigger and a test (English, with a
  non-code-shaped text, and Hindi). Point 3 is in D-B2-6 (Tab, Enter picks the
  only option, `aria-live="polite"`), but section 7.1 has no test for the
  Enter key or for the `aria-live` attribute.
- **P9 — confirmed Fixed.** D-B2-3 has `ParentAfterChild` and
  `GroupWithoutChildren`, the `validate` text lists both rules, and the Rust
  tests have a case for each. `base.json` as listed in D-B2-2 passes both:
  every group is first in its list and has 3 to 11 children.

### Owner answers

`status.md` and section 2.1 record Q1 to Q10 as all (a), 2026-11-07. I cannot
check the owner's answer from the repository; I only checked that the plan
is consistent with it. It is. Section 3 already lists the deviations that
(a) implies (Q1, Q3, Q4, Q5, Q7, Q9). One point to carry into Step 9: the
change doc bullet "The Hub category picker offers only taxonomy codes … in
any language the taxonomy has" (still there, just after the bullet that P7
changed) disagrees with Q4 (a) and Q7 (a), so the deviation must say it
replaces that bullet, not only add to it.

### New findings

#### P10 — The grep that Step 8.2 uses to prove "no fill is left" cannot match (minor)

- **Where:** Step 8.2, last sentence:
  `git grep -n "cats-input|categories')" crates/substrate/tests/e2e`.
- **Problem:** `git grep` without `-E` uses basic regular expressions, where
  `|` is a plain character. I ran the command on `main`: it finds 0 lines,
  though 13 lines fill a category box (`git grep -n -E "(categories|cats)(-input)?'\)\.fill"`
  finds all 13). My round 1 fix text wrote `\|`, which `git grep` reads as
  "or"; the plan dropped the backslash. My pattern had a second flaw: even
  with `-E`, `categories')` does not match `listing-categories-input')`.
- **Failure case:** The implementer runs the check after Step 8, sees no
  output, and believes no fill is left. A missed fill is found only when the
  e2e run fails.
- **Fix:** Use
  `git grep -n -E "(categories|cats)(-input)?'\)\.fill" crates/substrate/tests/e2e`
  and say the expected result: no line in `roym-hub.spec.ts` or
  `roym-trust.spec.ts` that fills a picker box (the lines that remain must be
  `pickCategory` calls or `.fill` on a non-picker field).
- *Status:* Fixed (baebc1de)

#### P11 — Step 9 asks for edits that are already done, and the plan text carries review ids (minor)

- **Where:** Step 9.3 and 9.4 (rewrite the "B." sentence and the Y2 row);
  section 7.1 (`matchCategories` and picker bullets), Step 7.1, owner
  answers of Q4 to Q6.
- **Problem:**
  1. `change.md` and `task.md` row Y2 were already rewritten in `b8203efb`.
     Step 9 still says "rewrite the existing sentence that says its labels per
     language", but the sentence no longer says it. An implementer will look for
     text that is gone.
  2. The plan prose now contains review ids ("P1", "P2", "P3", "P7", "P8") in
     bullets that describe tests and code. The id rule forbids planning
     ids in code, comments and test names. The plan is not code, but an
     implementer who copies these bullets into a test title or a comment
     breaks the rule.
- **Failure case:** (1) Wasted time or a second edit that changes the right
  text. (2) A test named "a group's label finds its children (P3 …)".
- **Fix:** (1) Change Step 9.3 and 9.4 to: "`change.md` and `task.md` row Y2
  were edited in the plan fix; check that they still say so, and replace the
  bullet 'The Hub category picker offers only taxonomy codes …' to match Q4
  and Q7". (2) Remove the `(P…)` marks from section 7.1, Step 7.1 and the
  steps; keep them only in the owner-answer notes, or remove them there too,
  and add one sentence in section 0: "Do not copy review ids into code,
  comments or test names."
- *Status:* Fixed (baebc1de)

## Plan review round 3 — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-08

Checked on `feat/m08b-b2-taxonomy` at `2fb5984a`, in the main checkout. I read
the diff from `099c70ca` to `HEAD` and re-ran the plan's grep.

**Summary of this round:** 0 new items. The four items that round 2 left open
are confirmed Fixed. **0 open.**

- **P2 — confirmed Fixed.** Section 7.1 now has a `messages.test.ts` entry
  ("after sending a request, the request form's category chips are gone").
  `src/screens/messages.test.ts` exists. With `clear()` in D-B2-6 and the Step
  7.1 call, forgetting the call now fails a test.
- **P8 — confirmed Fixed.** Section 7.1 has a test for Enter picking the only
  visible option and one for `aria-live="polite"` on the message container.
  Points 1 and 2 were already fixed in round 2.
- **P10 — confirmed Fixed.** Step 8.2 now uses
  `git grep -n -E "(categories|cats)(-input)?'\)\.fill" crates/substrate/tests/e2e`
  and says what the remaining lines must be. On `main` it finds the 13 fills
  that Step 8 replaces, so after the step it must find none that fill a picker
  box.
- **P11 — confirmed Fixed.** Step 9.3 and 9.4 no longer ask for the rewrite
  that is already done: they say to check `change.md` and `task.md` row Y2 and
  to replace the bullet "The Hub category picker offers only taxonomy
  codes …" to match Q4 and Q7. Section 0 says not to copy review ids into
  code, comments or test names. No B2 review id is left in the plan prose. The
  four ids that remain (`B1a P8`, `B1b P2` and so on, in section 7.8) name
  findings of earlier slices and were there before the review.

## Code review round 1 — Claude Opus 5.5, 2026-11-08

Reviewed `git diff origin/main...HEAD` on `feat/m08b-b2-taxonomy` at
`30142f64` (PR #353), in the main checkout (not a worktree). The code was
written by Gemini 3.8 Flash. The prompt lists one change doc,
[roym-multilingual](../../changes/roym-multilingual/change.md). Every B2
decision and deviation belongs there. This is the first code review round, so
there are no earlier R items to confirm.

**Summary:** 0 blocker, 1 major, 8 minor (R1 to R9). **9 open.**

### What was checked and found correct

- **CI.** `gh pr checks 353` on `30142f64`, when this round was written: CLA
  passes; Lints, Playwright E2E and Test 1/6 to 6/6 are still pending;
  Benchmarks and Code Coverage skip. So CI is not yet proof that the full
  suite passes. Check that every job is green before the merge. The parity
  test (both builds of `roym_core`) runs in those jobs.
- **Run here.** `cargo nextest run -p syneroym-roym-core`: 257 passed, 21 of
  them in `taxonomy::tests`. `npm test` in `crates/roym_web/ui`: 34 files, 369
  tests passed (the same as status.md). `npm run build`: passes (the
  visible-text check runs inside it). Sizes after `gzip -9`: main script
  49,207 bytes with no Devanagari letter and 2 hits for `bicycle-repair`;
  Hindi catalog 14,053; Hindi taxonomy chunk 2,518. These match task.md and
  are inside plan section 7.6 (+4,047 bytes, under 5 KB; chunk under 8 KB).
- **The data file (D-B2-1, D-B2-2).** `base.json` has exactly the 14 groups
  and 91 children of D-B2-2 in that order, the 17 units, the 6 methods and the
  6 attributes with 16 choices. Every group comes before its children. No
  version field and no labels.
- **The Rust module (D-B2-3).** The types, the `deny_unknown_fields` and the
  nine `TaxonomyError` variants are as in the plan. `validate` uses
  `listing::valid_token` with the record limits, checks duplicates, a parent
  that is a group, a group before its children, a group with children,
  attribute categories and number units. Each function is short. One test per
  variant, and each test reads the code in the error. No service calls
  `parse`, so no guest gets new work.
- **The code rule (W1 to W5, D-B2-10).** The five checks call
  `valid_token` with the right limit; the four renamed variants and
  `CategoryShape(String)` carry the value; `git grep` finds no old variant
  name. `transaction.rs` stays at 776 lines. The checks keep their order in
  each `validate`. The comment at `listing.rs:27` no longer names a review
  item or a slice (D-B2-13). `normalize_category` keeps `-`, so a hyphenated
  code is the same code in a credential scope, in a search and in a listing.
  No method, WIT file, typed input or record field changes.
- **Labels (D-B2-4, D-B2-5).** The ids follow the table of D-B2-4. The five
  label functions read `Map`s through `codeWord` (or `lookupText` for a
  choice), so `constructor` and `__proto__` show as they came. The catalog
  test proves both taxonomy files have exactly `taxonomyMessageIds()`, that
  every Hindi label has a Devanagari letter, that no string-catalog id starts
  with `taxonomy-`, and that `addResource` gives no error for either file.
- **The forms (D-B2-6 to D-B2-9).** The picker is in all four forms with the
  plan's class names and limits; `messages.ts` calls `clear()` after a send;
  the unit list starts at `piece`; the method boxes send the ticked codes in
  list order; `buildSetListingParams` sends the arrays as they are. The six
  placeholders are gone from both catalogs. The directory hit, the request
  card, the credential row, the issue check boxes and the payment
  acknowledgement card show labels, and a box `value` stays the code. All text
  goes through `textContent`; an own code must match `[a-z0-9-]{1,64}`.
- **e2e and story 40 (Step 8).** The plan's `git grep` finds no fill of a
  picker box. `hubMessage` reads the taxonomy file when the id is not in the
  string catalog. `pickCategory` checks the chip. Story 40 picks `gardening`
  by its Hindi label and does not save.
- **Backlog.** N-2 left the C5 nits row and is in "Recently resolved". The
  three new rows (group search, picker languages, no reader's view of a
  unit) have a reason, a target and a link. No new `TODO`.
- **Owner review (D-B2-12).** `taxonomy-review.md` has one row per code (150
  rows). `taxonomy/hi.ftl` still says `# owner review: pending`. This is not a
  finding; exit criterion 10 still stops the merge.

### Findings

#### R1 — major — the picker list is always open, so every form has about 90 buttons in its Tab order

- *Where:* `crates/roym_web/ui/src/taxonomy/category_picker.ts:41`, `:102`
  (`renderOptions`), `:224` (`clear`); `src/style.css` (`.picker-options`).
- *Problem:* the options box has no closed state. With an empty filter it
  always draws all 14 groups and 91 option buttons, in a 220px scroll box,
  under each of the four category fields. D-B2-6 says `clear()` "closes the
  options", which needs a closed state; the code cannot close it.
- *Failure case:* a keyboard user in the listing editor presses Tab from the
  category filter. The focus goes through 91 option buttons before it
  reaches the address field. The same happens in the request form inside a
  conversation and in the directory search, before the Search button. A
  screen reader user meets 105 extra items in each form.
- *Fix:* show `div.picker-options` only while the focus is inside the picker
  or the filter has text (a `focusin`/`focusout` pair on `root` that checks
  `relatedTarget`), and hide it in `clear()` and after Escape. Add picker tests:
  the options are hidden until the filter has focus; `clear()` hides them;
  Tab from the filter with no text reaches the next field. The Hindi sweep and
  the e2e steps focus or fill the filter first, so they still see the options.
- *Status:* Fixed (85894086). Added `openOptions` and `closeOptions` on `focusin`/`focusout` in `category_picker.ts`, `.picker-options[hidden] { display: none; }` in `style.css`, and unit tests in `category_picker.test.ts`. Proof: with initial closed state omitted, 3 tests failed with `AssertionError: expected false to be true`; with fix, all 13 tests pass.

#### R2 — minor — a language's two Fluent files load one after the other

- *Where:* `crates/roym_web/ui/src/i18n/i18n.ts:32` to `:45` (`LOADERS`).
- *Problem:* the loader awaits the string catalog, and only then starts
  loading the taxonomy chunk.
- *Failure case:* a Hindi user opens the Hub on a slow phone network. The
  first screen waits for two network round trips in a row instead of one.
- *Fix:* start both loads together:
  `const [catalog, taxonomy] = await Promise.all([loadCatalog(), loadTaxonomy?.() ?? ""]);`
  and join them only when `taxonomy` is not empty.
- *Status:* Fixed (0e687fb5). Parallelized catalog and taxonomy loading using `Promise.all([loadCatalog(), loadTaxonomy ? loadTaxonomy() : Promise.resolve("")])`. 22 tests in `i18n.test.ts` pass.

#### R3 — minor — the Hindi sweep's picker steps can pass without a chip, and the own-code fixture `x9` is missing

- *Where:* `crates/roym_web/ui/src/i18n/hindi-screens.test.ts:181` to `:186`
  and `:293` to `:299`.
- *Problem:* the steps are inside `if (filter) { … }` and use `opt?.click()`,
  and nothing checks that a chip was drawn. Plan section 7.1 asks for "the
  listing editor (with the picker open and one chip)". D-B2-11 says the
  sweep's own-code fixture is `x9`; no fixture in the sweep has an own code.
- *Failure case:* the class of the picker in the request form changes. The
  `if` skips the steps, the form is swept with no chip, and the test still
  passes. And a change that shows an own code through a Latin message
  (for example `category-picker-own-code` left in English) is never drawn on a
  Hindi screen.
- *Fix:* remove the `if` and the `?.`, and assert one
  `.picker-chip[data-code="bicycle-repair"]` after each pick. Add `x9` to one
  fixture that the sweep draws (the SynOrg settings `categories`, or a
  directory hit), so a raw own code is shown and passes.
- *Status:* Fixed (0e687fb5). Removed `if (filter)` and `?.` on picker clicks, asserted `.picker-chip[data-code="bicycle-repair"]` after each pick, and added `x9` raw custom code to `SETTINGS.categories`. Proof: intentional wrong selector `.request-cats-wrong` failed with `TypeError: Cannot set properties of null (setting 'value')`; with fix, all 17 tests pass.

#### R4 — minor — the ML-8 Playwright test compares titles, not listing ids

- *Where:* `crates/substrate/tests/e2e/tests/roym-hub.spec.ts`, test "a
  category search finds the same listings in English and in Hindi" (the last
  test of the file).
- *Problem:* plan section 7.3 says to "keep the listing ids of the hits" and
  check that they are the same. The test compares `.hit-title` texts. A
  title is author text; two listings can share it, and the shared e2e node
  has several cycling listings from other tests.
- *Failure case:* the Hindi search returns a different listing with the
  same title (for example a second run of `ensureSynOrgWithListing` that
  made a new version or a second listing). The test passes, though the two
  searches did not give the same results.
- *Fix:* read the `directory.find` answers with
  `page.waitForResponse` on `/rpc` around each Search click, and compare the
  sorted `listing_id`s of the hits. Or record the change as a deviation in
  the change doc, with the reason.
- *Status:* Fixed (4234ffbf). Captured `/rpc` `directory.find` responses using `page.waitForResponse` in English and Hindi, extracted and sorted `listing_id`s, and asserted `expect(hindiListingIds).toEqual(englishListingIds)`. Proof: asserting against `[...englishListingIds, 'unexpected-id']` failed with `expect(received).toEqual(expected)` deep equality difference; with fix, test passes (29.2s).

#### R5 — minor — a test name carries a criterion id

- *Where:* `crates/roym_web/ui/src/screens/directory.test.ts:157`
  (`"ML-8: a category search sends the same query in English and in Hindi"`).
- *Problem:* plan section 0 and AGENTS.md: no planning id (it names "ML-7"
  as an example) in `.ts` files or test names. The planning-refs gate does
  not catch `ML-<n>`.
- *Failure case:* the change doc is archived or its criteria are renumbered,
  and the test name points at nothing.
- *Fix:* drop the `ML-8: ` prefix. The change doc's ML-8 row already names
  the test by its text.
- *Status:* Fixed (49d7308a). Removed `ML-8: ` prefix from test title in `directory.test.ts:157`. All 7 directory screen tests pass.

#### R6 — minor — new Hindi text spells nasal vowels differently from the catalog

- *Where:* `crates/roym_web/ui/src/i18n/hi.ftl:730`
  (`category-picker-remove = { $label } हटाएं`), `:733` (`… श्रेणियां …`);
  `src/i18n/taxonomy/hi.ftl:62` (`जांच`), `:81` (`सेवाएं`), `:96`
  (`सब्जियां`), `:101` (`वस्तुएं`).
- *Problem:* `hi.ftl` writes these words with candrabindu: `हटाएँ` (for
  example `common-remove`, `messages-delete-button`), `श्रेणियाँ` (the label
  of the same field, `listing-label-categories`, `synorg-label-categories`),
  and `जाँच` (26 times, never `जांच`). Plan section 7.7: use the same Hindi
  word for the same thing.
- *Failure case:* the listing editor shows the field label "श्रेणियाँ" and,
  at the limit, the message "… 8 श्रेणियां चुन सकते हैं" right under it.
- *Fix:* write `हटाएँ`, `श्रेणियाँ`, `जाँच`, `सेवाएँ`, `सब्ज़ियाँ`,
  `वस्तुएँ`, and update the matching rows of `taxonomy-review.md`, so the owner
  reviews the final text.
- *Status:* Fixed (c39df5ae). Standardized candrabindu spellings to `हटाएँ`, `श्रेणियाँ`, `जाँच`, `सेवाएँ`, `सब्ज़ियाँ`, `वस्तुएँ` across `hi.ftl`, `taxonomy/hi.ftl`, and `taxonomy-review.md`. All 25 catalog tests pass.

#### R7 — minor — the same loop is written three times, and the label ids twice

- *Where:* `crates/roym_core/src/taxonomy.rs:152` (`validate_units`), `:165`
  (`validate_payment_methods`), `:220` (the choice loop);
  `crates/roym_web/ui/src/taxonomy/taxonomy.ts:122` (`taxonomyMessageIds`).
- *Problem:* the three Rust loops are the same "`valid_token`, then a
  `HashSet` for duplicates" with a different kind and limit. AGENTS.md: look
  for the same shape before writing a new helper. In `taxonomy.ts`,
  `taxonomyMessageIds` builds every id again with its own string prefixes,
  instead of reading the five `Map`s that the label functions use.
- *Failure case:* a later change (M08D adds attributes) changes the id form
  in one place. The catalog test then compares the label files with ids the
  label functions do not use, and passes while every label shows its raw
  code.
- *Fix:* one `fn check_codes(kind: &'static str, codes: &[String], max: usize) -> Result<(), TaxonomyError>`
  used by units, methods and choices. `taxonomyMessageIds()` returns
  `[...CATEGORY_IDS.values(), ...UNIT_IDS.values(), ...METHOD_IDS.values(), ...ATTRIBUTE_IDS.values(), ...CHOICE_IDS.values()]`.
- *Status:* Fixed (81f8b13a). Consolidated repeated code token validation loop into `check_codes` in `roym_core/src/taxonomy.rs`, and consolidated `taxonomyMessageIds()` in `taxonomy.ts` to spread values from the 5 label maps. All 21 Rust tests and 30 UI taxonomy/catalog tests pass.

#### R8 — minor — the change doc leaves out decisions and real deviations, and one deviation is wrong

- *Where:* `docs/planning/changes/roym-multilingual/change.md:199` (the
  "Decided 2026-11-07, the base taxonomy" list), `:208` ("Units"), `:477`
  (the last deviation), `:104` (ML-9 Test column);
  `docs/planning/deferred-backlog.md:543`.
- *Problem:*
  1. The decided list has none of: the Hub reads `base.json` through a Vite
     import and there is no `web` method (Q2, D-B2-5); the label id forms and
     the joined loader (D-B2-4); the owner review of the taxonomy labels
     before the merge (D-B2-12); the error variants and `-32602` of the five
     checks (D-B2-10). Plan section 3 says every B2 decision goes there.
  2. The code differs from the plan in ways that `## Deviations` does not
     list: R1 (no closed state), R3 (no `x9`), R4 (titles, not ids), and the
     new `tEnglish` export in `i18n.ts`. Each is either fixed or written down.
  3. The deviation at `:477` and the backlog row at `:543` say "the unit
     appears in the listing editor and payment acknowledgement cards". The
     payment acknowledgement card shows the payment method, not the unit
     (plan Q9 (a)).
  4. "Units" (`:208`) says the unit "becomes a code from a short unit list".
     The service accepts any well-formed code (`bundle-of-5` passes,
     `listing/tests.rs`); only the Hub offers the list.
  5. The ML-9 Test column names `templates.test.ts`, which has no unit test.
- *Failure case:* a later slice reads the change doc to learn how the Hub
  gets the codes, finds nothing, and adds a `taxonomy.get` method that Q2
  rejected.
- *Fix:* add the four decisions to the list; add the deviations (or fix
  R1, R3, R4); in `:477` and the backlog row say "the unit appears only in the
  listing editor"; in "Units" say "a code (any well-formed code; the Hub
  offers the base unit list)"; remove `templates.test.ts` from ML-9.
- *Status:* Fixed (472a6e90). Changed choice ID pattern in `change.md:203` to `taxonomy-choice-<attribute-word>_<choice-word>` with underscore, matching code, label files, and D-B2-4.

#### R9 — minor — the living docs and task.md say a little more, or other, than the code

- *Where:* `docs/system-requirements-spec.md:730` and `:1462`;
  `docs/system-architecture.md:920`; `docs/developer-guide.md:1761`;
  `docs/TERMINOLOGY.md:71`;
  `docs/planning/milestones/M08B-language-and-structure/task.md:157`.
- *Problem:*
  - Requirements `:730`: "The unit is a code from the base unit list". The
    record rule is the shape only (as in R8 point 4). `:1462` says the same
    for categories, units and methods: "are codes from a base taxonomy".
  - Architecture `:920`: the base taxonomy is "checked and validated by the
    Rust module". Only a test runs the check; no service calls `parse`.
  - Developer guide `:1761`: the taxonomy label files are "lazy-loaded
    alongside their language's string catalog". The English file is in the
    main script and is never lazy.
  - TERMINOLOGY `:71`: "codes in `crates/roym_core/taxonomy/base.json`,
    with labels per language" reads as if the labels are in that file.
  - task.md `:157`: "12 Rust taxonomy tests" (21 run) and "limit +10 KB"
    (plan section 7.6 sets 5 KB).
- *Failure case:* a developer reads the requirements, expects the service to
  refuse an unknown unit, and builds a client check on that.
- *Fix:* requirements: "a code of `[a-z0-9-]`; the Hub offers the base unit
  list" (and the same for `:1462`); architecture: "checked by a test in
  `roym_core::taxonomy`"; developer guide: "the English file is in the main
  script; another language's file loads with its string catalog"; TERMINOLOGY:
  "codes in `base.json`; their labels are in the Hub's taxonomy label files";
  task.md: 21 tests, limit 5 KB.
- *Status:* Fixed (cce1815f). Aligned living docs and task.md with implementation: requirements `:730` & `:1462` state "a code of `[a-z0-9-]`; the Hub offers the base unit list / taxonomy", architecture `:920` states "checked by a test in `roym_core::taxonomy`", developer guide `:1761` clarifies English in main script and other languages loading with catalog, TERMINOLOGY `:71` specifies `base.json` codes and Hub taxonomy label files, and task.md `:157` records 21 Rust taxonomy tests and 5 KB script growth limit.

## Code review round 2 — Claude Opus 5.5, 2026-11-08

Checked the fix commits `85894086` to `4d235dc0` (`git diff 1d7ff7fa..4d235dc0`)
on `feat/m08b-b2-taxonomy`, in the main checkout. I checked every R item of
round 1 against its fix commit, then read the fix commits for new problems.

**Summary:** round 1 items: 8 confirmed Fixed, 1 back to Open (R8). New: 0
blocker, 1 major (R10), 1 minor (R11). **3 open** (R8, R10, R11).

### What was checked and found correct

- **CI.** `gh pr checks 353` on `4d235dc0`: Lints, licenses & checks,
  Playwright E2E, Test 1/6 to 6/6, CI Pipeline and CLA pass; Benchmarks and
  Code Coverage skip. This is the proof that the full suite passes on the fix
  head.
- **Run here.** `npm test`: 34 files, 372 tests passed (369 plus the 3 new
  picker tests; the same as status.md). `npm run build` passes, with the
  visible-text check (28 passed). `cargo nextest run -p syneroym-roym-core taxonomy`:
  21 passed.
- **R1 — confirmed Fixed (85894086).** The options start `hidden`, open on
  `focusin` or input, close on `focusout` to outside the picker with an empty
  filter, on Tab from an empty filter, on Escape and in `clear()`.
  `.picker-options[hidden]` has `display: none`, so the hidden buttons leave
  the Tab order. The three tests check this. The fix causes a new problem in
  some browsers: R10.
- **R2 — confirmed Fixed (0e687fb5).** The loader starts both loads with
  `Promise.all` and joins them only when the taxonomy text is not empty.
- **R3 — confirmed Fixed (0e687fb5).** The `if` and `?.` are gone, each pick
  asserts one chip, and `x9` is in the SynOrg settings fixture, so the
  settings chip and the issue check box draw a raw own code in Hindi.
- **R4 — confirmed Fixed (4234ffbf).** The test reads the `directory.find`
  answer around each Search click (the Hub's one search call,
  `directory/search.ts:92`), sorts the `listing_id`s, checks the English list
  is not empty and that the Hindi list is equal. Playwright E2E passes on CI.
- **R5 — confirmed Fixed (49d7308a).** No `ML-<n>` is left in a test name.
- **R6 — confirmed Fixed (c39df5ae).** The six words use candrabindu (and
  `सब्ज़ियाँ` has its nukta), and `taxonomy-review.md` has the same text.
- **R7 — confirmed Fixed (81f8b13a).** `check_codes` serves units, methods
  and choices with the same kinds and limits as before, and every error still
  carries the code. `taxonomyMessageIds()` reads the five `Map`s, so the
  catalog test now checks the ids the label functions use.
- **R8 — back to Open.** Points 3 to 5 and the `tEnglish` deviation are
  right. Point 1 is wrong in one place: see the note on R8 above. Point 2:
  R1, R3 and R4 are now fixed in the code, so they need no deviation.
- **R9 — confirmed Fixed (cce1815f).** Requirements `:730` and `:1462`,
  architecture `:920`, developer guide `:1761`, TERMINOLOGY `:71` and task.md
  `:157` now say what the code does.

### New findings

#### R10 — major — on Safari and iOS, a click on a listed category does nothing when the filter is empty

- *Where:* `crates/roym_web/ui/src/taxonomy/category_picker.ts`, the `root`
  `focusout` handler added in 85894086 (`if (!root.contains(next) && !filter.value.trim()) closeOptions();`).
- *Problem:* Safari (macOS and iOS) and Firefox on macOS do not give a
  `<button>` the focus when it is clicked or tapped. So a press on an option
  moves the focus from the filter to `<body>`, and `focusout` arrives with
  `relatedTarget` null. With an empty filter the handler hides the options at
  once, so the button is `display: none` before the click, and the click
  lands on nothing. Chrome gives the button the focus, so Chrome and the
  Playwright tests (Chromium only, and `pickCategory` always types first) do
  not see it. I checked the handler's part in jsdom: after `filter.focus()`,
  a `focusout` with `relatedTarget: null` and an empty filter sets
  `options.hidden` to true.
- *Failure case:* Alice uses the Hub on an iPhone. She taps the category box,
  scrolls the list and taps "Gardening". The list closes and no chip
  appears. She can pick a category only by typing first.
- *Fix:* keep the focus in the filter while the person presses an option:
  `options.addEventListener("mousedown", (e) => e.preventDefault());`. Then no
  `focusout` fires, the click reaches the button, and after a pick the focus
  is still in the filter, so the person can pick the next code. Add a picker
  test: a `mousedown` on an option is `defaultPrevented`, and after it a
  `click` on that option adds the chip.
- *Status:* Fixed (25654cf9). Added `options.addEventListener("mousedown", (e) => e.preventDefault());` in `category_picker.ts` so focus remains in `filter` when clicking an option. Added test in `category_picker.test.ts`. Proof: without the fix, the test failed with `AssertionError: expected false to be true` checking `mousedownEvent.defaultPrevented`; with fix, all 14 tests pass.

#### R11 — minor — task.md still gives the old Hub test count

- *Where:* `docs/planning/milestones/M08B-language-and-structure/task.md:157`
  ("Unit tests (369 UI tests, …)").
- *Problem:* the fix commits added 3 picker tests. `npm test` now runs 372,
  and status.md says 372.
- *Failure case:* a reader compares task.md with status.md and sees two
  counts for the same run.
- *Fix:* write 372 (and the new count after R10's test).
- *Status:* Fixed (472a6e90). Updated `task.md:157` test count to 373 UI tests.

## Code review round 3 — Claude Opus 5.5, 2026-11-08

Checked the fix commits `25654cf9` to `065d4ad5` (`git diff 48f783d8..065d4ad5`)
on `feat/m08b-b2-taxonomy`, in the main checkout.

**Summary:** the 3 open items (R8, R10, R11) are confirmed Fixed. No new
items. **0 open.**

- **R8 — confirmed Fixed (472a6e90).** `change.md:203` now writes the choice id
  as `taxonomy-choice-<attribute-word>_<choice-word>`, with `_`, as the code
  and both label files do (`taxonomy-choice-diet_veg`).
- **R10 — confirmed Fixed (25654cf9).** `options` cancels `mousedown`, so a
  press on any option (the own-code option too) does not move the focus out
  of the filter. No `focusout` fires, the list stays open until the click
  lands, and the focus stays in the filter for the next pick. Wheel and touch
  scrolling of the list do not use `mousedown`, so they still work. The new
  test checks that the `mousedown` is cancelled and that the click then adds
  the chip.
- **R11 — confirmed Fixed (472a6e90).** task.md says 373 UI tests. Here,
  `npm test` gives 34 files, 373 tests passed; `npm run build` passes, with
  the visible-text check (28 passed).
- **CI.** `gh pr checks 353` on `065d4ad5`, when this round was written:
  Lints, licenses & checks and CLA pass; Playwright E2E and Test 1/6 to 6/6
  are still pending. The full run was green on `4d235dc0`. Since then, the
  only code change is the one `mousedown` line and its unit test. Check that
  the run on the head is green before the merge.
- **Before the merge.** The owner review of the taxonomy codes and labels
  (D-B2-12, exit criterion 10) was completed on 2026-11-08 with no changes:
  `taxonomy/hi.ftl` says `# owner review: done 2026-11-08`, and
  `taxonomy-review.md` was deleted.

## Code review round 4 — Claude Opus 5.5, 2026-11-08

Round 3 said 0 open while CI on the fix head was still running. That run
finished red. This round records why. Checked on `13f72668` (only docs since
`065d4ad5`).

**Summary:** 0 blocker, 1 major (R12). **1 open.**

- **CI.** Run 42486386143 on `13f72668`: Test 1/6 to 6/6, Lints and CLA pass;
  Playwright E2E fails. One test fails all 3 tries: `roym-hub.spec.ts:971`,
  "26. messages tab: sending a request posts a card …", at `:988`: the request
  form is still visible 15 s after the Submit click. The CI screenshot shows
  both chips ("Home Cleaning", "Deep Cleaning"), the list closed, no error
  text, and the form still open: the Submit click never reached the button.
  The full run was green on `4d235dc0`; the only code change since then is
  the `mousedown` line of 25654cf9.

#### R12 — major — after a pick, the next click below the picker is lost

- *Where:* `crates/roym_web/ui/src/taxonomy/category_picker.ts`, the
  `options` `mousedown` handler (25654cf9) together with the `root`
  `focusout` handler and the in-page `.picker-options` box (85894086).
- *Problem:* the R10 fix keeps the focus in the filter during a pick. So
  after a pick the filter still has the focus, the filter is empty, and the
  list stays open (all groups, up to 220px high). The next `mousedown` on a
  control below the picker moves the focus out, `focusout` hides the list,
  and everything below the picker moves up by the list's height before
  `mouseup`. The click does not land on the control. Before 25654cf9, in
  Chrome the clicked option was removed from the page during the pick, the
  focus left the picker, the list closed at once, and the next click worked.
- *Proof:* a page with only the picker and a button below it, driven by
  Playwright Chromium (the vite dev server, a throwaway page, deleted after).
  Fill "Plumbing", click the `plumbing` option, click the button. On
  `065d4ad5`: the list is open after the pick, and the button's click handler
  does not run. With the `mousedown` line removed: the list is closed after
  the pick, and the click handler runs.
- *Failure case:* Bob picks "Home Cleaning" in the request form and clicks
  "Submit request". Nothing happens. A second click works. The same happens
  for any control under the picker in the four forms (Save, Search), and also
  when a person opens the list and then clicks a control below it without
  picking.
- *Fix:* do two things, and keep the `mousedown` line (R10 needs it).
  1. Make the list float, so that opening or closing it never moves the page:
     `.category-picker { position: relative; }` and
     `.picker-options { position: absolute; left: 0; right: 0; top: 100%; z-index: 10; }`
     (or place it right under the filter).
  2. Close the list after each pick (`closeOptions()` in `pick()`), and open
     it again on a click in the filter (`filter.addEventListener("click", openOptions)`)
     and on input, as now. Then the next control is not under the open list.
  Test 26 is the guard (it fails today). Add one picker unit test: after a
  pick, `options.hidden` is true, and a click on the filter opens it again.
  Run the whole Playwright suite and story 40.
- *Status:* Open.
