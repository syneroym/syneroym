# M08B slice B3a — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-b3a-implementation-plan.md](slice-b3a-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-09

Checked on `feat/m08b-b3a-lang-records` at `453eeb36`, in the main checkout
(not a worktree: `.git` is a directory). `git pull` first failed in the
sandbox (GitHub was not allowed). With the sandbox off it said "Already up to
date". The only commit after the plan's base (`4c7b699c`) is the plan itself,
so the code facts below hold on `main`.

**Who reviewed.** This session runs on Claude Sonnet 5.5. The `status.md` row
says the plan was written by Claude Opus 5.5. They are different models, as
the prompt says.

**Change docs.** The prompt lists one change doc,
[roym-multilingual](../../changes/roym-multilingual/change.md). `task.md`
names no other change doc for B3a. The plan also reads
[demo-stories](../../changes/demo-stories/change.md) for context only. I agree:
B3a meets no criterion of that doc and writes nothing there. ML-5, ML-10,
ML-11, ML-12 and ML-13 are all in roym-multilingual, so every B3a decision
and deviation belongs there (plan section 3 says the same).

**Summary:** 0 blocker, 4 major, 9 minor, 13 items. 13 Open.

Limits of this review: I read the plan, the docs, the code and the e2e specs.
I ran one experiment: I added one line to `ListingPayload::validate`, ran
`cargo clippy -p syneroym-roym-core --lib`, and reverted the file (P3). I did
not run any plan step, the Hub tests or `mise run verify`.

### What was checked and found correct

- **File and line links.** Spot-checked and right: `person.rs:17` to `:27`,
  `:41`; `profile_ops.rs:34` to `:39`, `:136` to `:141`; `contacts.rs:127` to
  `:133`; `listing.rs:178` to `:212`, `:214`, `:507`; `listing_ops.rs:81`,
  `:423`; `transaction.rs:136`, `:163`, `:189`, `:262` to `:375`, `:611`;
  `request_ops.rs:90`; `quote_ops.rs:38`, `:57`, `:189`; `app.rs:504`;
  `verify.rs:138` to `:155`; `merge.rs:21`, `:232`; `search.ts:10`;
  `directory.ts:309` to `:310`; `request.ts:45` to `:51`; `picker.ts:4` to
  `:10`; `i18n.ts:89` to `:97`; `shapes.rs:154`; `xtask/src/main.rs:184`;
  `deny.toml:101`; `Cargo.lock` (`icu_locale_core` 2.4.0); backlog `:508` and
  `:535`; `demo.md:1290`, `:1294`; `developer-guide.md:1755`.
- **File sizes.** `transaction.rs` 776, `transaction/tests.rs` 799,
  `listing.rs` 549, `person.rs` 102, parity `profile.rs` 1112, `transaction.rs`
  846, `directory.rs` 981, `helpers.rs` 1290 (all four are in
  `xtask/oversized-test-files.txt` with these counts), `fixtures.rs` 798,
  `messages.ts` 923, `listings.ts` 587, `profile.ts` 72, `roym-hub.spec.ts`
  1357. `TransactionError` is 114 lines (`:262` to `:375`), so the split of
  Step 2 does bring `transaction.rs` to about 660.
- **Caller counts (point 9).** I counted the quoted method names again:
  `profile.set` 37 lines, `listing.set` 41, `request.set` 33, `quote.set` 35.
  The plan's per-file numbers match what I found, and the Playwright specs call
  none of them directly (only `roym-hub.spec.ts:334` watches the body of
  `listing.set`; it checks for integers, so `lang` does not break it).
- **No other caller outside Rust and the Hub.** I searched every non-Rust file
  outside `docs/planning` and `docs/archive` for the four method names and for
  `roymctl roym transaction`. Only the Hub, `demo.md:1290` and `:1294`, and the
  prose in the developer guide and architecture doc appear. No shell demo
  story sends a payload.
- **`locale` is used nowhere else.** `git grep locale` finds only
  `person.rs`, `profile_ops.rs`, `shapes.rs:154` and one requirements line.
- **The method layer gives the right error.** `method::parse_value` uses
  `serde_path_to_error`. A custom `Deserialize` error for `lang` or
  `terms.lang` gives `data.field = "lang"` or `"terms.lang"`, and
  `languages[2]` for a list item (`method.rs:108` to `:141`). So the
  `-32602` cases of section 7.2 work with no change to that layer. A tag that
  is 36 bytes or longer is refused before the error text can echo it (the plan
  checks the length first).
- **Both builds.** `LangTag` is plain JSON, so no WIT file changes. All six
  guests link `roym_core`, so all six get `icu_locale_core` (plan 7.6 measures
  this). `crates/*` is a workspace glob, so the new crate joins with no edit
  to `members`. `cargo xtask check-roym-deps` needs the entry of Step 3.3
  because `roym_core` is a `roym_*` directory.
- **Snapshot list (point 16).** Every snapshot in
  `crates/roym_web/tests/method_shapes/` that holds a record payload is in the
  plan's list. The exports (`catalog_export`, `profile_export`,
  `directory_export`) hold only envelope strings, so they do not change.
  `transaction_export` and `transaction_thread` hold card data and do change.
- **Q1 (a) works with `halves_agree`.** `a.terms == b.terms` is a derived
  `PartialEq`, so `lang` is compared with no extra code. `AgreementView` and
  `AgreementRow` hold `terms`, so `agreement.get` and `agreement.list` show
  `terms.lang` (both snapshots are in the list).
- **Hub tests.** No unit test picks a `<select>` by position. The only
  positional select is in the e2e spec (P4).
- **Plan form.** "How this is shown to an end user" has 5 bullets for the end
  user and 3 for a developer. Each question names the step it blocks. Backlog
  rows have a reason, a target and a link.

### Views on the owner's questions

(For the owner to decide. I do not answer for the owner.)

- **Q1 (quote `lang` in `terms`, `translations` on the quote).** I would take
  (a). The receipt then says which language binds, and `halves_agree` needs no
  code. It makes P1 more important: the receipt holds only the original, so the
  quote card must say that.
- **Q2 (which fields are translated).** I would take (a). Names, addresses, the
  notice and the service lists stay out. The backlog row for the lists is
  right.
- **Q3 (canonical form only, refuse `HI-latn`).** I would take (a). One rule,
  and the signed bytes equal the struct. The cost is P10: a signed record is
  valid only while the ICU printer gives the same text. The S0 table as a unit
  test catches a change at upgrade time. The plan should say so.
- **Q4 (`lang` required in every input).** I would take (a). A node that
  guesses `und` hides a bug in the client. The cost is mechanical. Keep the
  count honest: it is about 146 call lines, and JSON built in helpers does not
  name the method, so the compiler will not find them. The test run in Step 4.7
  does.
- **Q5 (translations in the listing editor only).** I would take (a). See P6:
  the profile form must still not delete translations that came from the API.
- **Q6 (fixed language list).** I would take (a). Add P12: a test that every
  entry is in canonical form, because the node refuses anything else.
- **Q7 (one step in story 40).** I would take (a). It does not save, so the
  story runs again. See P4 for the editor markup that the step and the main
  suite touch.
- **Q8 (one PR).** I would take (a). The diff is large but mechanical. The
  split point the plan names (after Step 5) is a good one.

### Findings

#### P1 — The quote card shows the author's translation by default, next to "Accept", and says nothing about the original binding (major)

- **Where:** D-B3a-10 (the note texts), D-B3a-14 (quote card), section 3
  (deviations list); `crates/roym_web/ui/src/screens/messages.ts:577` to
  `:600` (the Accept button sits under the quote card).
- **Problem:** With Q1 (a), the receipt copies only `terms`. A translation is
  not part of what binds. But `bestVersion` shows the translation first to a
  reader whose language matches it, and the card has an Accept button. The
  note says only "Translated by the author from Hindi." It does not say that
  the Hindi text is the one that binds. The change doc says the original still
  binds ("Agreements across languages"), but it plans the warning only for
  machine translation (ML-19, B5). B5 also adds `viewed_lang`. Until B5, the
  Hub shows a text that does not bind, with no warning, beside the button that
  signs.
- **Failure case:** Alice signs a quote in Hindi with an English translation
  where the refund text is more generous. Bob reads the English text and taps
  Accept. His receipt holds the Hindi terms. Nothing on the card told him so.
- **Fix:** On the quote card (and the receipt card, which shows only the
  original), when the shown text is an author translation, add one line: "The
  original text, in { $language }, is what binds." Make it one more message
  (`record-text-original-binds`, English and Hindi) used by the quote card
  only, and test it in `templates.test.ts`. Or, if the owner wants the line in
  every `recordText` note, change the note text instead. Then list in section 3
  that B3a shows author translations before `viewed_lang` exists (B5), and say
  how the line covers the gap.
- *Status:* Fixed (ffdacc8d)

#### P2 — The fulfilment receipt also carries `terms`, but the plan misses its wire change, its card and its message id (major)

- **Where:** section 4 (W6, "Wire changes"), D-B3a-14, Step 8.1;
  `crates/roym_core/src/fulfilment.rs:41` (`pub terms: AgreedTerms`);
  `crates/roym_web/ui/src/cards/templates/fulfilment_receipt.ts:37` to `:40`.
- **Problem:** `FulfilmentReceiptPayload` holds an `AgreedTerms`, so it gets
  `terms.lang` too, but W6 names only the quote and the agreement receipt. The
  fulfilment card draws `data.terms.scope` with `t("terms-scope", …)`. D-B3a-14
  removes `terms-scope` ("The old ids go") and Step 8 lists `request.ts`,
  `quote.ts` and `agreement_receipt.ts`, not `fulfilment_receipt.ts`. Also
  `payment/tests.rs` and `fulfilment/tests.rs` build `AgreedTerms` fixtures
  (Step 4.2 lists their files, but the wire table does not).
- **Failure case:** Step 8 renames `terms-scope`. `fulfilment_receipt.ts` still
  calls it. `catalog.test.ts` fails ("every used id is in `en.ftl`"), and on a
  real card `t()` returns the raw id and logs an error. The scope of a
  fulfilment receipt also has no `lang`, so ML-5 is not met there.
- **Fix:** Add the fulfilment receipt to W6 and to the "What B3a meets" row for
  ML-5. Add `fulfilment_receipt.ts` to Step 8.1 and D-B3a-14: scope drawn with
  `labelled(…, t("terms-scope-label"), valueSpan)` and `lang = terms.lang`. Add
  a case to `templates.test.ts` and to the Hindi sweep. Run
  `git grep -n "terms-scope\|terms-address\|terms-cancellation\|terms-refund\|request-notice"`
  at the end of Step 8 and expect no match outside the `-label` ids.
- *Status:* Fixed (ffdacc8d)

#### P3 — `ListingPayload::validate` is exactly at the clippy line limit, so the translation check breaks the clippy gate (major)

- **Where:** D-B3a-3 (the rules in `validate`), section 7.5 ("`listing.rs`
  about +30"); `crates/roym_core/src/listing.rs:272` to `:379`.
- **Problem:** The plan adds a call to `check_translation_langs` and four
  `check_text` calls to `ListingPayload::validate`. That function is already
  at the limit. I added one line (`let _probe1 = 1;`) and ran
  `cargo clippy -p syneroym-roym-core --lib`. Result: `this function has too
  many lines (101/100)` at `listing.rs:272`. I reverted the file; the tree is
  clean. AGENTS.md forbids raising the threshold or adding a non-test
  `expect`.
- **Failure case:** Step 4.1 adds the checks. Clippy with `-D warnings` fails,
  and so does the `clippy` gate of `mise run verify`. The same risk is smaller
  for the other records: `RequestPayload::validate` is 40 lines, `QuotePayload`
  about 40, `ProfilePayload` about 20.
- **Fix:** Say in D-B3a-3 that `ListingPayload::validate` first moves its
  `location` and `relationship` blocks (about 25 lines) into private
  `validate_location` and `validate_relationship`, with no change of
  behaviour, and then adds `self.validate_translations()?`. Put the move in
  Step 2 beside the other no-change splits, so the diff of Step 4 stays about
  the new rules. Update the size row for `listing.rs`.
- *Status:* Fixed (ffdacc8d)

#### P4 — A Playwright step takes the first `<select>` of the listing editor, and the plan says no step changes (major)

- **Where:** section 7.3 ("no step changes for it"), D-B3a-11 (no place is
  named for the control), D-B3a-13;
  `crates/substrate/tests/e2e/tests/roym-hub.spec.ts:764`,
  `crates/roym_web/ui/src/screens/listings.ts:239` to `:247`.
- **Problem:** Test "listings … draft" does
  `page.locator('.listing-editor select').first().selectOption('draft')`. Today
  the first `<select>` of the editor is the status list (it is the last
  `field(...)` of the first block). The plan adds `textLangSelect` in a
  `field(t("listing-text-language"), …)`. The natural place is next to the
  title or summary, that is, before the status. Then `.first()` is the text
  language list and `selectOption('draft')` fails (no such option). The plan
  checked the profile screen (point 14) but not this step. The category picker
  of B2 is not a `<select>`, so nothing else has moved the order before.
- **Failure case:** `mise run test:e2e` fails at the e2e gate of
  `mise run verify`, or the listing step is edited late, in the code-review
  round.
- **Fix:** Add `roym-hub.spec.ts:764` to section 2.2 (a new point, next to 14).
  Either place the text language field after `status` in the editor (then
  `.first()` is still the status), or change the step to a class
  (`.listing-status-select`) and add the class in `listings.ts`. Add one more
  `git grep -n "\.first()\|nth(" crates/substrate/tests/e2e` at the end of
  Step 9 to prove no positional pick is left on a screen that gained a
  control. Section 7.3 then says which steps change.
- *Status:* Fixed (ffdacc8d)

#### P5 — The new profile controls can submit the form or break the status line step (minor)

- **Where:** D-B3a-12, section 2.2 point 14;
  `crates/substrate/tests/e2e/hub-helpers.ts:46` to `:53`,
  `crates/roym_web/ui/src/screens/profile.ts:29` to `:33` (the submit button).
- **Problem:** Point 14 says the new controls must not be text inputs. Two more
  rules apply. (1) "move up", "remove" and "add" are `<button>` elements inside
  the `<form>`. A button with no `type` is a submit button, so a click would
  save the profile (and the status line would say "Saved"). (2)
  `saveProfile` waits on `.profile-screen p` with `toContainText`. Playwright
  runs that locator in strict mode, so a second `<p>` (for example an "empty
  list" line) makes the step fail.
- **Failure case:** A person taps "move up" and the form posts. Or story 20
  and the Hub suite fail on a strict-mode error.
- **Fix:** In D-B3a-12 say: every new button has `type="button"`, and no new
  element in `.profile-screen` is a `<p>` (use `<ul>`, `<li>`, `<span>`). Add a
  `profile.test.ts` case ("move up does not submit the form") and keep the
  existing `saveProfile` step unchanged.
- *Status:* Fixed (ffdacc8d)

#### P6 — Saving the profile in the Hub erases `about` and the translations that came from the API (minor)

- **Where:** D-B3a-11, D-B3a-12, Q5 (a);
  `crates/roym_web/ui/src/screens/profile.ts:38` to `:70`,
  `crates/roym_profile/src/app/profile_ops.rs:34` to `:39`, `:136` to `:141`.
- **Problem:** `profile.set` builds a new payload from its input every time.
  The Hub form sends only `display_name` and `conversation_address`, and
  reads only those two from the stored profile. So today a Hub save drops
  `about`. With B3a it also drops `translations`. A profile translation needs
  `about` (D-B3a-3), and Q5 (a) says profile translations come "through the
  API". A person who set them that way and then fixes a typo in the Hub loses
  both, with no message.
- **Failure case:** An API client sets `about` and an English translation. The
  person opens the Profile screen, changes the display name and saves. The new
  record has no `about` and no `translations`.
- **Fix:** In Step 7, make the form load `about` and `translations` from the
  stored envelope (it already reads `env.payload`) and send them back
  unchanged. Add a `profile.test.ts` case ("save sends the stored about and
  translations back"). The form still shows no field for them; that stays in
  the backlog row.
- *Status:* Fixed (ffdacc8d)

#### P7 — Some ML-5 and ML-12 limits are real but not in the deviation list (minor)

- **Where:** section 3 (table row ML-5, ML-12; list of deviations),
  D-B3a-10.
- **Problem:** The ML-5 row says "Every block of record text in the Hub has
  `lang` set". It is not true for: the own listing rows (`listings.ts:125`
  shows `row.title`, from `listing.list`, which has no `lang`), names (display
  names, group names), and the free text of records that get no tag in B3a (the
  note of a payment request, the reference of a payment acknowledgement). The
  plan names the own listing row in point 7, but not as a deviation. For ML-12
  ("always shows which one it shows"), D-B3a-10 shows no note when a record has
  one version. That is a reading of the criterion, and a reasonable one, but
  it is a deviation from the wording.
- **Failure case:** At close-out the change doc ML-5 and ML-12 rows say "done",
  and a later reader finds a block with no `lang`. B2 had the same kind of
  finding (B2 R8).
- **Fix:** Add two lines to the deviations list of section 3: ML-5 covers the
  text of the four records at the places of D-B3a-14 (and the fulfilment
  receipt, P2), not names, own listing rows or untagged records (they follow
  with the records that get a tag: opinion, M08G; chat, B3b); ML-12 shows a note
  only when a record has an author translation. Reword the ML-5 row of the
  table to match.
- *Status:* Fixed (ffdacc8d)

#### P8 — The request notice and the default quote terms are tagged with the record's language, but they are written in the UI language (minor)

- **Where:** D-B3a-14 ("the notice with `lang` = the record's `lang`"),
  section 2.2 point 17.
- **Problem:** Point 17 says that if the person picks a text language that is
  not the UI language, the record mixes two languages: the description is in
  the text language and the notice is in the UI language. D-B3a-14 then draws
  the notice with the record's `lang`. That tag is wrong in exactly the mixed
  case: an English notice is marked as Hindi. A screen reader reads it with a
  Hindi voice. The same holds for the three default quote terms when the person
  keeps them.
- **Failure case:** An English-UI Hub user writes a Hindi request. A Hindi
  reader's screen reader reads "Notice: …" (English words) in a Hindi voice.
- **Fix:** Do not set `lang` on the notice element, and say why in D-B3a-14:
  its language is the writer's UI language, which the record does not store.
  The page language is then the best default. The `lang` stays on the
  description and the translations. Keep the backlog row of section 8 and add
  this reason to it.
- *Status:* Fixed (ffdacc8d)

#### P9 — The largest quote with four translations is about 55 KB, and the signing error for a too-large payload is an internal error (minor)

- **Where:** D-B3a-2 (the "about 11 KB" estimate, a listing only), section 7.2;
  `crates/signed_record/src/envelope.rs:7` (`MAX_PAYLOAD_BYTES` = 64 KiB),
  `:105` to `:106`, `crates/roym_transaction/src/app/quote_ops.rs:381` to
  `:383`.
- **Problem:** The limits count bytes of text, but the 64 KiB ceiling counts
  canonical JSON bytes, where `"` and a newline take two. A quote may hold
  `scope` 4,096 and three terms of 2,048 bytes, in five versions (the original
  and four translations): 51,200 bytes of text, plus about 3 KB of other
  fields. That is about 84% of the ceiling before escaping. A request is about
  20 KB, a listing about 20 KB (with the service lists), a profile 5 KB. When a
  quote goes over, the host refuses it at signing, and `quote_ops` returns
  `internal_error` with the message of `DraftError::PayloadTooLarge`. The
  person sees an "unexpected error", and no test covers it.
- **Failure case:** A shop writes long multi-line terms in Hindi and four
  translations. `quote.set` answers `-32603`, not a message about size.
- **Fix:** Add to section 7.2 a test in `transaction/tests/lang.rs`: a quote
  with all five versions at the maximum length, plain ASCII, signs and
  verifies. State the worst-case sizes in D-B3a-2 in place of "about 11 KB".
  Then choose one: (a) accept the internal error and add a backlog row, or (b)
  in `QuotePayload::validate` (and the request) refuse a payload whose
  `serde_json::to_vec` is over `MAX_PAYLOAD_BYTES` with a typed error, so the
  answer is `-32602`. (b) is about 8 lines.
- *Status:* Fixed (ffdacc8d)

#### P10 — Refusing a non-canonical tag on the verify path ties old signed records to the version of the ICU printer (minor)

- **Where:** Q3 (a), D-B3a-1 (`Deserialize` calls `parse_canonical`).
- **Problem:** `LangTag` deserializes only the canonical text. That is the
  rule for a stranger's record too. If a later `icu_locale_core` release prints
  one tag differently (for example a new rule for a deprecated subtag), a
  record signed today with the old canonical text becomes "not canonical" and
  fails to verify. Stored listings, chat cards and backups are signed, so they
  cannot be rewritten. The S0 table, run as a unit test (section 7.2), would
  fail at the upgrade, which is the right alarm, but the plan does not say
  what to do then.
- **Failure case:** A dependency update changes the printed form of an
  extension tag. Every stored record that carries it is refused as "not in
  canonical form".
- **Fix:** Add one paragraph to D-B3a-1 and to the change doc deviation: the
  canonical text is part of what a signature covers, so the S0 table is a
  golden test, and an `icu_locale_core` upgrade that changes it needs an
  owner decision (for example: on the verify path, accept any well-formed tag
  as received, and keep the strict rule for the input path only). No code
  change is needed now.
- *Status:* Fixed (ffdacc8d)

#### P11 — `roymctl roym transaction request` has no function to unit-test, so the planned test has no place to stand (minor)

- **Where:** D-B3a-7, section 7.2 (`roymctl` test);
  `apps/roymctl/src/commands/roym/transaction.rs:159` to `:195`.
- **Problem:** `handle_request` builds its params and calls the RPC in one
  function. The plan says a bad `--lang` "stops before the call" and plans a
  unit test of that. The quote path has `build_quote_params`, which is
  testable. The request path does not, and `handle_request` already has 10
  arguments with `#[allow(clippy::too_many_arguments)]`; an eleventh is added.
- **Failure case:** The implementer has to extract a function during Step 4 to
  write the test, or writes no test.
- **Fix:** In D-B3a-7, say: extract `build_request_params(…) -> Result<Value>`
  (the code of `handle_request` before `call_and_print`), give it `lang`, and
  test it there. Pass `lang` into `build_quote_params` in the same way.
- *Status:* Fixed (ffdacc8d)

#### P12 — Nothing checks that every `RECORD_LANGUAGES` entry is in canonical form (minor)

- **Where:** D-B3a-9, section 7.1 (`text_lang.test.ts`).
- **Problem:** The node refuses any tag that is not canonical (Q3 (a)), and the
  Hub list is a hand-typed constant. One entry typed `hi-latn` or `ta_Latn`
  would show in the list and then fail every save with `-32602`. The tests of
  7.1 check order and names, not the form.
- **Failure case:** A later edit adds `kn-latn`. The unit tests pass, and the
  first person who picks it gets an error from the node.
- **Fix:** Add to `text_lang.test.ts`: for each entry of `RECORD_LANGUAGES`,
  `new Intl.Locale(tag).toString()` equals the entry; and the same for every
  catalog tag from `availableLanguages()` (the catalog test already checks the
  file names). The two parsers agree on the S0 table, so this catches a
  typo.
- *Status:* Fixed (ffdacc8d)

#### P13 — A few statements in the living-docs list and the backlog list are wrong or become wrong (minor)

- **Where:** section 8 (first row), section 9.
- **Problem:**
  1. Section 9 says `system-architecture.md` has a "Roym records table
     (`:1206` to `:1210`, and the `profile` and `listing` rows)". Lines
     `:1204` to `:1216` are the **Cards** table (request, quote,
     agreement-receipt, …). The doc has no table with `profile` or `listing`
     rows; the listing record is described in prose (`:918`, `:1138`).
  2. The backlog row at `deferred-backlog.md:535` says "The node checks a
     language tag and stores it in syntax form (`HI-latn` becomes
     `hi-Latn`)". After Q3 (a) this is false: the node refuses `HI-latn`. The
     plan only adds a link to that row.
  3. The change doc "Tag validation" paragraph and its ML-10 row say the same
     as the backlog row, and are false after Q3 (a). The plan records a
     deviation, but does not say that the paragraph is rewritten.
  4. Section 9 gives `:2541` for the developer guide "Unknown fields"
     sentence. It is at `:2542`.
- **Failure case:** The close-out step edits the wrong place, or leaves a
  living-doc sentence that says the node rewrites tags.
- **Fix:** In section 9: name the Cards table at `:1204` and the prose at
  `:918`/`:1138` (put the `lang` sentence in the prose, and one clause in the
  `request` and `quote` rows). In section 8: change the text of the `:535`
  row ("the node refuses a tag that is not in canonical form and does not
  replace old codes") as well as the link. In Step 10.2: rewrite the "Tag
  validation" sentence of the change doc, not only add a deviation. Fix `:2542`.
- *Status:* Fixed (ffdacc8d)

## Plan review round 1, verification — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-09

Checked on `feat/m08b-b3a-lang-records` at `3bd56437`. I read the diff of the
plan (`453eeb36..HEAD`), the diff of the change doc, and the code facts behind
each fix. I did not run any plan step.

**Summary:** 13 items confirmed fixed. 2 new minor items (P14, P15). 2 Open.

### Confirmed

| Item | Where the plan now answers it |
| --- | --- |
| P1 | D-B3a-10 and D-B3a-14: `record-text-original-binds` on the quote card, shown with an author translation. In the message-id list. Test case in section 7.1. Deviation in section 3 names the gap until B5. (D-B3a-10 also says "and receipt card". The receipt never shows a translation, so this is harmless.) |
| P2 | W6, the ML-5 row, D-B3a-14, Step 8.1 (`fulfilment_receipt.ts`, the `git grep`), the tests of 7.1 and the Hindi sweep. |
| P3 | Point 4 of section 2.2, D-B3a-3, D-B3a-8, Step 2 item 3 (the split moves to Step 2) and the size row. The size row has an error (P15). |
| P4 | Point 18, D-B3a-11 (`.listing-status-select`, `.listing-text-lang`), Step 7.1, Step 9.2 (the spec line and the `git grep`), section 7.3. |
| P5 | Point 14, D-B3a-12 (`type="button"`, no `<p>`), Step 7.2 and the test in 7.1. |
| P6 | D-B3a-12, Step 7.2, and the test in 7.1 ("save preserves stored `about` and `translations`"). |
| P7 | The deviation list of section 3 and the reworded ML-5 row. |
| P8 | D-B3a-14: the notice has no `lang`, with the reason. Step 8.1. The backlog row of section 8 has the reason. |
| P9 | D-B3a-2 has the worst-case sizes. The test of 7.2 is added. The plan chooses (a): a backlog row in section 8. |
| P10 | D-B3a-1 paragraph, the deviation list, the change doc deviation (added), and the "Tag validation" sentence and ML-10 row of the change doc (rewritten). |
| P11 | D-B3a-7: `build_request_params`, and the tests of 7.2. A new `allow(clippy::too_many_arguments)` is fine: `build_quote_params` already has one, and `xtask/src/lint_suppressions.rs` does not count it. |
| P12 | D-B3a-9 and the test in 7.1. |
| P13 | Section 9 (Cards table `:1204`, prose `:918`/`:1138`, `:2542`), section 8 (the `:535` row text), Step 10.2 (the rewrite). |

### New findings

#### P14 — The owner answers are dated 2026-11-05, but the plan was written and the answers were given on 2026-11-09 (minor)

- **Where:** plan lines 6, 172, 197, 222, 246, 265, 289, 312 and 330
  (the status line and every "Owner answer: (a), 2026-11-05").
- **Problem:** The plan was committed on 2026-11-09 (`453eeb36`) with "Owner
  questions open". The answers came with the fix in `ffdacc8d`, also on
  2026-11-09. The date 2026-11-05 is the date of the owner decisions in
  `task.md` (the demo, the crate, the catalog format), and was copied by
  mistake. Line 295 ("owner confirmed 2026-11-05") is correct: it is about
  the `task.md` demo paragraph.
- **Failure case:** The plan says the owner answered four days before the plan
  existed. At close-out, the change doc "Decided <date>" list copies the wrong
  date.
- **Fix:** Change the nine dates to 2026-11-09. Leave line 295.
- *Status:* Fixed (8f8f2085)

#### P15 — The size row for `listing.rs` says the file shrinks by 25 lines, but a move into helper methods does not shrink it (minor)

- **Where:** plan line 1243 (section 7.5, row `listing.rs`).
- **Problem:** The row says "-25 (Step 2 decomposition), then about +35".
  Moving the `location` and `relationship` blocks into two methods keeps the
  same lines and adds two function signatures and two calls: about +6. The
  `transaction.rs` row is right to say "-115", because that code leaves the
  file.
- **Failure case:** The implementer or a reviewer looks for a 25-line drop
  after Step 2, does not find it, and doubts the step. The ratchet gate does
  not care (549 is far from 800), but the number is wrong.
- **Fix:** Write "about +6 (Step 2, helper methods), then about +35. Limit
  800".
- *Status:* Fixed (8f8f2085)
