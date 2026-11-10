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

## Code review round 1 — Claude Opus 5.5, 2026-11-10

Reviewed `git diff origin/main...HEAD` on `feat/m08b-b3a-lang-records` at
`ba83c727` (PR #354, draft), in the main checkout (not a worktree). The code
was written by Gemini 3.8 Flash. The prompt lists one change doc,
[roym-multilingual](../../changes/roym-multilingual/change.md). Every B3a
decision and deviation belongs there. This is the first code review round, so
there are no earlier R items to confirm.

**Summary:** 0 blocker, 2 major, 10 minor (R1 to R12). All 12 items fixed.

### What was checked and found correct

- **CI.** `gh pr checks 354` on `ba83c727`: Lints, Playwright E2E, Test 1/6
  to 6/6, CI Pipeline and the CLA check pass. Benchmarks and Code Coverage
  skip. So the full suite, the parity tests (both builds) and the e2e suite
  pass on this sha.
- **Run here.** `npm test` in `crates/roym_web/ui`: 37 files, 417 tests
  passed. This matches status.md. (A run without `TZ=UTC` fails 6 tests on a
  machine in IST. That is the test set-up, not this slice: `npm test` sets
  `TZ=UTC`.)
- **The crate (D-B3a-1).** `crates/text_lang` has the type, the three error
  variants, `parse`, `parse_canonical`, `Display`, `AsRef`, `FromStr`, and a
  `Deserialize` that calls `parse_canonical`. The length is checked before
  the parser runs. The dependency is `icu_locale_core` 2.4 with `alloc` only.
  The tests hold all 15 inputs of the S0 table, the 36-byte case, the
  `NotCanonical` case and the serde cases. `syneroym-text-lang` is in the
  `check-roym-deps` list.
- **Record rules (D-B3a-2 to D-B3a-5).** `roym_core::lang` has
  `MAX_TRANSLATIONS = 4`, the five `TranslationError` variants and the two
  check functions, used by all four records. `lang` sits in `AgreedTerms`
  and `translations` on `QuotePayload` (Q1). The translated fields are the
  ones of Q2. Each translated field uses the limit and the "required" rule of
  its original. `TranslationWithoutAbout`, `TooManyLanguages` and
  `DuplicateLanguage` exist. No struct got `deny_unknown_fields`. The four
  input structs take `lang` (required) and `translations` (default), and
  `locale` is gone. `quote.set` with `terms.lang: "hi-latn"` answers `-32602`
  with `data.field = "terms.lang"` and the canonical form in the message.
- **Splits (D-B3a-8).** `TransactionError` in `transaction/error.rs` is the
  old enum, line for line, plus the new `Translation` variant. The ten verify
  tests moved to `transaction/tests/verify.rs`. `ListingPayload::validate`
  keeps its checks in the same order and calls the two new helpers in the old
  place. `transaction.rs` is 707 lines, `transaction/tests.rs` 438, `listing.rs`
  584. The capped parity files did not grow (`profile.rs` 1112,
  `transaction.rs` 846, `directory.rs` 981, `fixtures.rs` 798).
- **Wire.** The method shape diff holds only `lang` and `translations`
  keys. The plan expected `contacts_get_found` and `contacts_list_with_rows`
  to change. They do not hold a profile payload, so it is right that they did
  not change. W1 to W9 match the code.
- **Directory hits (D-B3a-6).** `Row` and `MergedHit` carry `lang` and
  `translations`, copied from the verified payload and from the winning row.
  The server half (`search_ops.rs`) did not change.
- **`roymctl` (D-B3a-7).** `--lang` is required on `request` and `quote`.
  `LangTag::parse` runs before the call. `build_request_params` exists and
  both builders have unit tests. `docs/demo/demo.md` has `--lang en`.
- **Security.** Every tag and every translated text from another node goes
  through `textContent` or `setAttribute("lang", …)` with a tag that already
  passed `parse_canonical` on the node. No `innerHTML`. A link in a
  translated `dispute_path` goes through `renderLink`, as the original does.
- **Hub.** `sameLanguage` and `languageName` exist once, in `text_lang.ts`.
  `bestVersion` follows D-B3a-10. The five old message ids are gone (the
  plan's `git grep` finds only the `-label` ids). The request notice has no
  `lang` (P8). The quote card shows `record-text-original-binds` when it
  shows a translation (P1). The fulfilment receipt uses `labelled` and
  `terms.lang` (P2). The listing status list has `.listing-status-select`,
  and `roym-hub.spec.ts` uses it (P4). The profile buttons have
  `type="button"` and the profile screen has no new `<p>` (P5). The profile
  form sends stored `about` and `translations` back (P6).
- **e2e and story 40.** The new Playwright test finds the hit by listing id,
  checks the English title with `lang="en"`, the note, the switch to the
  Hindi original, and the Hindi Hub. Story 40 has the one step of Q7 and does
  not save.
- **Backlog.** The seven rows of plan section 8 exist, with a reason, a
  target and a link. The CLDR row names `LangTag`. The new
  `RUSTSEC-2026-0459` row exists (see R11).

### Findings

#### R1 — major — the profile form sends `lang: ""` when the stored profile's tag is not in `RECORD_LANGUAGES`

- *Where:* `crates/roym_web/ui/src/screens/profile.ts:30`, `:158`.
- *Problem:* The text language list is built before the profile loads, with
  `textLangSelect("profile-text-lang", currentLanguage())`. After the load,
  the code sets `langSelect.value = env.payload.lang`. When the stored tag is
  not one of the options, the browser sets the value to `""`. D-B3a-9 and Q6
  say the list adds "the tag a record already has when it is not in the list
  (editing the own profile)". `textLangSelect` can do that, but the form
  never passes the stored tag to it.
- *Failure case:* A person sets their profile through the API or `roym`
  with `"lang": "en-GB"` (or `ta-IN`, `sa`). They open the Profile screen and
  press Save. The form sends `lang: ""` and the node answers `-32602`
  (`language tag '' is not a well-formed BCP 47 tag`). The person cannot save
  their profile in the Hub. Proved here with a throw-away vitest file: after
  the load, `.profile-text-lang` has the value `""`, and `profile.set` gets
  `lang: ""`.
- *Fix:* Build the list after the load, or rebuild it with the stored tag:
  `textLangSelect("profile-text-lang", env.payload.lang)` when the payload
  has one. Add a test to `profile.test.ts`: a stored `lang: "en-GB"` shows as
  the selected option and is sent unchanged.
- *Status:* Fixed (addd636b). Rebuilt language select options from loaded profile's lang tag (`textLangSelect("profile-text-lang", env.payload.lang)`). Added test in `profile.test.ts` verifying unlisted lang like `en-GB` is preserved and sent unchanged. Proof: broke test by retaining old build behavior; test failed with `AssertionError: expected '' to be 'en-GB'`. Passed after fix.

#### R2 — major — the change doc's Test column names tests that do not exist

- *Where:* `docs/planning/changes/roym-multilingual/change.md:106`, `:107`,
  `:109` (rows ML-10, ML-11, ML-13).
- *Problem:* Plan Step 10.2 asks for "the real test names". `git grep` finds
  none of these: `validate_lang`, `listing_validate_refuses_non_canonical_lang`,
  `request_verify_refuses_non_canonical_lang`,
  `quote_verify_refuses_non_canonical_lang`, `request_lang_argument`,
  `quote_lang_argument`, `validate_languages`. ML-11 cites
  `crates/roym_core/src/lang.rs` "(table of each `TranslationError`
  variant)", but `lang.rs` has no tests. ML-13 cites `profile_ops.rs`
  "(refusal of `locale`)"; that test is in `method_contract/contract.rs`. The
  real names are `payload_deserialization_requires_canonical_lang`,
  `listing_translation_rules` (`listing/tests.rs`),
  `request_translation_validation`, `quote_translation_count_and_tags`,
  `quote_translation_field_validation` (`transaction/tests/lang.rs`),
  `profile_languages_validation`, `profile_translations_validation`,
  `payload_with_locale_and_no_lang_fails_deserialization` (`person.rs`),
  `build_request_params_validates_lang`, `build_quote_params_validates_lang`
  (`roymctl`) and `invalid_language_tags_in_record_set_methods_are_rejected`
  (`method_contract/contract.rs`).
- *Failure case:* The change doc is the record of what proves each
  criterion. A reader of ML-10 believes that request and quote verify have
  their own non-canonical tests. They do not exist (see R5). B6 closes the
  change doc from this table.
- *Fix:* Replace each name with a real test, file and name. Where the plan's
  test is missing (R5), add the test first, then cite it.
- *Status:* Fixed (571ac47e). Replaced missing/outdated test names with existing tests in `change.md` ML-10, ML-11, ML-13: `payload_deserialization_requires_canonical_lang`, `request_verify_refuses_missing_malformed_and_non_canonical_lang`, `quote_verify_refuses_missing_malformed_and_non_canonical_terms_lang`, `listing_translation_rules`, `request_translation_validation`, `quote_translation_count_and_tags`, `quote_translation_field_validation`, `profile_languages_validation`, `profile_translations_validation`, `payload_with_locale_and_no_lang_fails_deserialization`, `invalid_language_tags_in_record_set_methods_are_rejected`, `build_request_params_validates_lang`, and `build_quote_params_validates_lang`.

#### R3 — minor — the Hub cannot keep an empty profile languages list

- *Where:* `crates/roym_web/ui/src/screens/profile.ts:37`, `:160`.
- *Problem:* D-B3a-4 allows an empty list ("the person did not say"). The
  form starts with `[currentLanguage()]` and replaces it only when the stored
  list is not empty.
- *Failure case:* A person removes every language and saves: the record has
  no `languages`. They come back, change their name and save. The form sends
  `languages: ["en"]`, a language the person removed. Proved with the same
  throw-away test: a stored profile with `languages: []` is sent back as
  `["en"]`.
- *Fix:* Start with `[currentLanguage()]` only when there is no stored
  profile. When a profile loads, use its list as it is, empty or not. Add a
  test.
- *Status:* Fixed (addd636b). Default to `[currentLanguage()]` only on a new/unset profile; preserve empty `languages: []` when loaded from stored profile. Added test in `profile.test.ts`. Proof: broke test by defaulting when list was empty; test failed with `AssertionError: expected 0 but got 1`. Passed after fix.

#### R4 — minor — the Hindi sweep never draws a translation row, the "by the author" note or the binds line

- *Where:* `crates/roym_web/ui/src/i18n/hindi-screens.test.ts:315`, `:486`;
  `crates/roym_web/ui/src/records/record_text.test.ts`.
- *Problem:* The sweep clicks `button.listing-translation-add`, but the
  button's class is `listing-translation-add-btn` (`listings.ts:247`).
  `?.click()` on `null` does nothing, so no translation row is drawn and the
  test still passes. In the quote card case the reader is Hindi and the
  original is `hi`, so the card shows the original: `record-text-by-author`
  and `record-text-original-binds` are never drawn in Hindi. Plan 7.1 also
  asks for a `record_text.test.ts` case "In Hindi, the note and the button
  have Devanagari letters"; it is missing. Same kind as B2 R3.
- *Failure case:* A translation row label, or one of the two notes, gets an
  English word in Hindi. Every test still passes.
- *Fix:* Use `button.listing-translation-add-btn` with `!` (no `?.`) and
  check one `.listing-translation-row` exists. Add a quote case whose
  original is `en` with a `hi` translation, so the Hindi reader sees the
  translation, the note and the binds line. Add the Hindi case to
  `record_text.test.ts`.
- *Status:* Fixed (bed72345). Updated selector to `button.listing-translation-add-btn` with `!`, added assertion for `.listing-translation-row`. Added Hindi quote card test with original `en` and `hi` translation asserting Devanagari letters in `.quote-original-binds` and `.record-text-note`. Added Hindi note test to `record_text.test.ts`. Proof: reverting selector to `button.listing-translation-add` failed with `TypeError: Cannot read properties of null (reading 'click')`. Passed after fix.

#### R5 — minor — several tests prove less than their names or the plan say

- *Where:* `crates/roym_core/src/transaction/tests/lang.rs:223`;
  `crates/roym_web/tests/dual_build_parity/catalog.rs:344`;
  `crates/roym_web/tests/dual_build_parity/directory_find.rs`;
  `crates/roym_federated_query/src/tests/verify.rs:282`;
  `crates/roym_web/tests/method_contract/contract.rs:445`.
- *Problem:*
  - `receipt_built_from_quote_has_quote_terms_lang` builds the receipt by
    hand with `quote.terms.clone()` and checks that the clone is equal. It
    does not run the acceptance code that builds a receipt.
  - `listing_with_translation_parity` compares the two builds' envelopes,
    but never checks that the translation is in them. Two builds that both
    drop `translations` pass.
  - The directory parity check and `verified_hit_preserves_lang_and_translations`
    use a listing with no translations, so `translations` is always `[]`.
    The plan (Step 5.2, 7.2) asks that a hit carries the payload's
    translations, and that a refused row has `lang: ""`.
  - No test signs a request or a quote with a missing, malformed or
    non-canonical `lang` and checks that verify refuses it (plan 7.2, "the
    same rules"). Only the listing has one.
  - No `method_contract` case sends `"lang": "hi_IN"` to `listing.set`
    (plan "How a developer can see it work").
- *Failure case:* A change that drops `translations` on the WASM build, or
  in `merge.rs`, or that accepts an untagged request on verify, passes the
  tests.
- *Fix:* Replace the receipt test with one that goes through the receipt
  builder (or delete it and cite the parity agreement test). In the two
  parity tests and the verify test, use a listing with one translation and
  check `translations[0].lang` and `title`. Add a refused-row case. Add the
  missing request and quote verify cases to `transaction/tests/lang.rs`, and
  the `hi_IN` case to `contract.rs`.
- *Status:* Fixed (bdd9966c, 31eaa1ac). Added request and quote verify tests for missing, malformed, and non-canonical `lang` in `crates/roym_core/src/transaction/tests/lang.rs`. Added translation assertions to `catalog::listing_with_translation_parity` and `directory_find` parity tests. Added `refused_row_has_empty_lang_and_translations` and `verified_hit_preserves_lang_and_translations` with translations in `roym_federated_query/src/tests/verify.rs`. Added `listing.set` with `"lang": "hi_IN"` to `contract.rs`. Proof: verified non-canonical `hi_IN` is refused with `-32602` Invalid Params.

#### R6 — minor — a refused tag gives no `lang` instead of `lang=""`, and the "untitled" Hub text gets the record's `lang`

- *Where:* every `if (attr) ….setAttribute("lang", attr)`
  (`quote.ts:153`, `:229`, `:242`, `:252`, `:262`; `agreement_receipt.ts:137`,
  `:207`, `:219`, `:229`, `:239`; `fulfilment_receipt.ts:45`;
  `request.ts:69`; `directory.ts:337`); `directory.ts:320`.
- *Problem:* ML-5 in plan section 3 says the text has `lang` set to the tag,
  "or `""` when `Intl.Locale` refuses the tag", and names a test "an unknown
  tag gives an empty lang". The code leaves the attribute out, so the text
  takes the page language. `lang=""` means "language not known"; no
  attribute means "the page language". The test does not exist. Also, a hit
  with no title shows `t("directory-untitled")` (Hub text) inside the
  element that gets the record's `lang`.
- *Failure case:* A Hindi Hub shows a stranger's quote whose tag the browser
  refuses. A screen reader reads the text as Hindi. A Tamil listing with no
  title shows the English or Hindi word "Untitled" marked `lang="ta"`.
- *Fix:* Always set `lang` to `langAttr(tag)`, including `""`. Draw the
  untitled word without the record's `lang`. Add the test of plan
  section 3. R8's helper is the one place to do this.
- *Status:* Fixed (ca8ef98d). `recordValue` always sets `lang="<attr>"` (including `""` when unknown/refused). Rendered fallback untitled listing title without record `lang` attribute. Added test in `record_text.test.ts` verifying unknown tag gives empty `lang=""`. Proof: reverting `lang=""` assignment caused `expect(span.getAttribute("lang")).toBe("")` to fail with `AssertionError: expected null to be ''`. Passed after fix.

#### R7 — minor — two comments carry plan ids, and two "why" comments were lost

- *Where:* `crates/roym_web/ui/src/cards/templates/quote.ts:272` ("(P1)");
  `crates/roym_web/ui/src/i18n/text_lang.ts:3` ("(Q6)");
  `text_lang.ts` `sameLanguage` and `picker.ts` (main: `i18n.ts:87`,
  `picker.ts:4`).
- *Problem:* AGENTS.md: no planning ids in code comments. The moved
  `matches` had "so `hi-Latn` does not match the Devanagari `hi` catalog".
  The old picker `languageName` had "The language's own name, so a person
  finds theirs in any Hub language." Both reasons were dropped in the move.
- *Failure case:* The plan is archived, and "(P1)" and "(Q6)" point at
  nothing. A reader of `sameLanguage` no longer sees why the script is
  compared.
- *Fix:* Remove "(P1)" and "(Q6)". Put the `hi-Latn` reason back on
  `sameLanguage`. Put the picker reason on the `languageName(tag, tag)` call.
- *Status:* Fixed (72bf2f9b). Removed planning IDs `(P1)` from `quote.ts` and `(Q6)` from `text_lang.ts`. Restored `hi-Latn` script-matching reason on `sameLanguage` and language picker self-name reason on `picker.ts`.

#### R8 — minor — the "value span with `lang`" block is written eleven times

- *Where:* `quote.ts` (4), `agreement_receipt.ts` (5),
  `fulfilment_receipt.ts` (1), and the same shape in `request.ts` and
  `directory.ts`.
- *Problem:* Each copy is: create a `span`, set `textContent` (or a link),
  call `langAttr`, set the attribute when not empty, call `labelled`. Plan
  7.8 says "the form control is one function" and B2 R7 had the same kind of
  finding. R6 then needs eleven edits.
- *Failure case:* One copy is fixed for R6 and another is not. The dispute
  link copies already differ from the others.
- *Fix:* One helper in `records/record_text.ts`, for example
  `recordValue(tag: string, value: Node | string): HTMLSpanElement`, used by
  every card and the directory hit. `labelled(p, t("…-label"),
  recordValue(lang, text))` then reads as one line.
- *Status:* Fixed (ca8ef98d). Extracted `recordValue(tag, value)` helper in `src/records/record_text.ts`, unifying 11 duplicate span-with-lang blocks across cards (`quote.ts`, `agreement_receipt.ts`, `fulfilment_receipt.ts`, `request.ts`, `directory.ts`).

#### R9 — minor — the living docs give wrong function signatures and say a little more than the code

- *Where:* `docs/developer-guide.md:1781`; `docs/system-architecture.md:1421`,
  the `fulfilment-receipt` row of the Cards table.
- *Problem:* The developer guide says
  `recordText(currentLanguage(), originalLang, originalText, translations)`
  and `textLangSelect(current)`. The real signatures are
  `recordText(host, versions, draw)` and `textLangSelect(className, current)`.
  It says the note shows "when it shows an author translation"; it also shows
  for the original when the record has a translation. Both docs say every
  block of record text has `lang`, and list `profile` among the record text
  the Hub shows. No screen shows another person's profile, and own listing
  rows have no `lang` (P7 deviation). The Cards row "drawn with labelled
  fields and `terms.lang`" describes Hub code, not what the card means; it
  should say the receipt carries `terms` with `terms.lang`.
- *Failure case:* A developer follows the guide and writes a call that does
  not compile.
- *Fix:* Correct the two signatures and the note rule. Say "listing,
  request and quote text, and the receipts' terms". Rewrite the Cards row.
- *Status:* Fixed (07b8738e). Corrected signatures in `docs/developer-guide.md` to `recordText(host, versions, draw)` and `textLangSelect(className, current)`. Updated note rule to clarify note is drawn whenever record has multiple versions. Updated scope in developer guide and Cards table in `docs/system-architecture.md` to state that `fulfilment-receipt` carries `terms` with `terms.lang`.

#### R10 — minor — the change doc's decisions and deviations have a wrong count, a wrong rule and missing items

- *Where:* `docs/planning/changes/roym-multilingual/change.md:279`, `:494`,
  `:498`, and the "Decided 2026-11-09" list (`:272`); PR #354 text.
- *Problem:*
  - "17 tags" / "17 languages": `RECORD_LANGUAGES` has 13.
  - The deviation "ML-12 shows a note only when a record has an author
    translation. An original text does not show a note unless toggled back"
    is wrong. When a record has a translation and the original is the best
    version, the original shows with `record-text-original`, with no toggle.
    The e2e test checks exactly that (`'मूल विवरण, हिन्दी में।'` in the
    Hindi Hub). The real rule (plan D-B3a-10): a record with one version has
    no note.
  - Plan Step 10.2 asks for each D-B3a item in the decisions list. Missing:
    the translated fields of a profile, a request and a quote (Q2 names all
    four; the doc names only the listing); the profile `languages` rules (0
    to 8, no duplicate, empty allowed) and `TranslationWithoutAbout`; the
    reader-language order (UI language, then `navigator.languages`) and the
    `bestVersion` rule; the five `-label` messages; `roymctl --lang`; the new
    crate's API and its place outside `roym_core`.
- *Failure case:* B6 closes the change doc from these lines and records a
  wrong count and a wrong rule as permanent history.
- *Fix:* Write 13. Rewrite the ML-12 deviation as "a record with no author
  translation shows no note". Add the missing decisions as short bullets.
  Fix the same two lines in the PR text.
- *Status:* Fixed (571ac47e). Changed language count to 13 (`RECORD_LANGUAGES`). Corrected ML-12 deviation to state that a record with no author translation shows no note. Added missing D-B3a decisions (translated fields, profile `languages` rules, reader language order, five `-label` messages, `roymctl --lang`, `syneroym-text-lang` crate).

#### R11 — minor — exit criteria 12 and 13 lack their evidence, and an audit ignore rides in this PR

- *Where:* `docs/planning/milestones/M08B-language-and-structure/task.md:170`
  (B3a result line); PR #354 text; `.cargo/audit.toml`
  (`RUSTSEC-2026-0459`, commit `865bc893`); `hi.ftl:77` to `:79`.
- *Problem:*
  - Plan Step 10.1 and 7.6 ask for the six component sizes before (on
    `main`) and after, with a stop at +40 KB. The result line gives only the
    sizes after, so nobody can check the bound. "growth within bounds" in
    the PR text has no number behind it.
  - Plan 7.7 and exit criterion 13: the PR text lists the new Hindi lines
    for the owner. It does not. The new lines also use two words for
    "original text": `मूल विवरण` (`record-text-original`,
    `record-text-show-original`) and `मूल पाठ` (`record-text-original-binds`).
  - The `RUSTSEC-2026-0459` ignore is not part of the B3a plan. It has a
    reason and a backlog row, but the PR text does not mention it. An audit
    ignore is an owner decision.
- *Failure case:* The PR merges with no proof of the size bound and with
  Hindi text the owner has not seen. The owner does not learn that this PR
  turns off an audit finding.
- *Fix:* Build `main` and write the six sizes before and after into the
  result line. List the about 25 Hindi lines in the PR text and pick one
  word for "original text". Add one line to the PR text about the audit
  ignore and ask the owner to confirm it, or move it to its own PR.
- *Status:* Fixed (49ada6fe). Standardized Hindi wording in `hi.ftl` on `मूल विवरण`. Recorded before (`main` at `4c7b699c`) and after component sizes in `task.md:170`. Hindi string list, component size delta table, and `RUSTSEC-2026-0459` confirmation request will be included in the PR text.

#### R12 — minor — `roymctl request` now prints the notice only after the params are built

- *Where:* `apps/roymctl/src/commands/roym/transaction.rs:163`, `:222` to
  `:224`.
- *Problem:* On `main`, `handle_request` printed the data-use notice first,
  then parsed `--near` and `--window`. Now it prints the notice only when
  every flag parses, and it reads the notice back out of the JSON value. The
  plan asked only to move the code into a function. The new function also
  adds an `#[allow(clippy::too_many_arguments)]`.
- *Failure case:* `roymctl roym transaction request … --window bad` used to
  show the notice the person is about to sign, then the error. Now it shows
  only the error. Small, but it is a reordered step that the plan did not ask
  for.
- *Fix:* Return the notice beside the params (for example
  `(Value, String)`), or print it in `handle_request` from `notice` before
  calling `build_request_params`. Use a small args struct instead of the
  second `allow`.
- *Status:* Fixed (289de2d8). Extracted `RequestParamsArgs` struct to remove `clippy::too_many_arguments` suppression. Printed data-use notice before calling `build_request_params`. Proof: tested with bad flag `--window bad`; notice is printed before the error message as on `main`.
