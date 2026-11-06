# M08B slice B1a — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-b1a-implementation-plan.md](slice-b1a-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-06

Checked on `feat/m08b-b1a-localization-mechanism` at `0a62d3b8`, in the main
checkout (not a worktree). `git pull` failed first in the sandbox (GitHub was
not allowed). It said "Already up to date" when GitHub was allowed. The commit
after the plan's base (`9e19b888`) is the plan itself, so the code facts below
hold.

**Who reviewed.** The prompt says the reviewer is Claude Opus 5.5 and the plan
author is Claude Sonnet 5.5. This session runs on Claude Sonnet 5.5, and the
`status.md` row says the plan was written by Claude Opus 5.5. I record the
model that really ran this review (Sonnet 5.5) here and in `status.md`. If the
prompt values are right and the row is wrong, the owner should correct the
"Plan by" column.

*Plan author's note (fix round):* the row is right. The plan was written by
Claude Opus 5.5, and this review by Claude Sonnet 5.5, so the two are
different models.

**Change docs.** The prompt lists one change doc,
[roym-multilingual](../../changes/roym-multilingual/change.md). `task.md` also
names [demo-stories](../../changes/demo-stories/change.md), in its "Demo"
paragraph (story 40). The plan says it meets no criterion of that doc and
records nothing there. I agree: the story follows its rules (a story file in
`demo/`, captions that name whose screen it is, a `demo:` pointer in the
roym-multilingual change doc). Every criterion (`ML-2` to `ML-6`) and every
decision belongs to roym-multilingual.

**Summary:** 0 blocker, 3 major, 9 minor, 12 items. All 12 Fixed (f10a0b15), with the owner's answers to Q1 to Q6 (all (a), 2026-11-06).

Limits of this review: I read the plan, the docs and the code. I ran `Intl`
checks in Node 22.11 and read the Rust tests, `verify.rs`, `assets.rs` and the
Hub source. I tried a small Vite build to check the `import.meta.glob` chunk
split, but the sandbox refused the command, so that part is still `unverified`
(see P10). I did not run any plan step.

### What was checked and found correct

- **File and line links.** Every link of section 2.2 points at the right code:
  `money.ts:66`, `dom.ts:12`, `messages.ts:96` and `:637`, `listings.ts:29`,
  `:487`, `:488`, `groups.ts:126`, `:563`, `rpc.ts:35`, `:42`, `:48`,
  `index.html:16`, `booking/tests.rs:296`, `group/tests.rs:189`, `group.rs:17`,
  `money.rs:133`, `roym_web/src/app.rs:288` to `:324`,
  `roym_conversation/src/app.rs:135` and `:137`, `envelope.rs:59` to `:67`,
  `runtime/roym.rs:535`, `ci.yml:78`, `hub-helpers.ts:11` and `:15`,
  `roym-hub.spec.ts:838`, `:846`, `:1091`, `:1095` to `:1098`,
  `roym-trust.spec.ts:168`, `roym-hub.spec.ts:559`. The living-doc lines also
  match (`system-architecture.md:1412`, `system-requirements-spec.md:1461`,
  `traceability-matrix.md:68`, `developer-guide.md:1620` and `:1745`,
  `deferred-backlog.md:508`). `demo.md` has the R3 section at line 1231.
- **Counts.** The 9 card `toISOString` calls (4 files) and the 4 `toLocale*`
  calls are exact. `ageWords` and `dateWords` have 10 callers. The 22 `.message`
  reads are exactly the 22 lines of the Step 4 table (I listed them with a
  search). `main.ts` is 396 lines, `messages.ts` 943.
- **Rust tests that read UI files.** There are seven, not two: `money.rs`,
  `router.rs` (`enrolment.ts`), `card.rs` (`registry.ts`),
  `membership/tests.rs` (`membership.ts`), `transaction/tests.rs`
  (`messages.ts`), `booking/tests.rs` (`wording.ts`), `group/tests.rs`
  (`words.ts`). The plan moves only the last two files' strings, and the other
  five files keep their constants in B1a. B1a edits `membership.ts` and
  `messages.ts` only on lines that do not hold the pinned constants
  (`DEFAULT_DATA_USE_NOTICE`, `ADDRESS_DISCLOSURE_NOTICE`, the two membership
  notices), so those tests keep passing. The 18 pinned sentences have no `{`,
  `}` or special line start, so Fluent keeps them as written.
- **`Intl` claims, run in Node 22.11** with `en-IN-u-nu-latn` and
  `hi-IN-u-nu-latn`: `₹1,00,000.50` (both), `$35.50`, `JP¥1,200`,
  `KWD 2.500`, `€1,200.00`, `-€45.00`, `₹9,00,71,99,25,47,409.91` for the
  string `"90071992547409.91"` (so the string form keeps every digit), and
  `15 Jan 2027, 1:30 pm IST` for `en`. `formatUtcDate(1790000000)` is
  `21 Sept 2026` in `en`. `new Intl.Locale("hi", {region:"IN", numberingSystem:"latn"})`
  gives `hi-IN-u-nu-latn`.
- **Both builds.** B1a changes no `roym_*` code, no WIT, no wire value and no
  config. The Hub is a static bundle that both the WASM and the native `web`
  service serve through the same asset table (`[services.web.assets]` in
  `roym.toml`). No Rust test reads `dist/` or `index.html`. So no WASM rebuild
  and no parity change is needed. The plan says the same.
- **Asset bundle limits.** `assets.rs` takes any file in the archive, guesses
  the type from the extension (`.js` is `text/javascript`), and limits the
  archive to 2 MiB and the file count. The current bundle is 34 KB. A second
  script file is served like `assets/index-*.js`. `script-src 'self'` allows it.
- **CSP.** `default-src 'none'; script-src 'self'` allows a dynamic `import()`
  of a same-origin chunk and the `modulepreload` link that Vite adds.
  `style-src 'self'` is not hit: the picker sets no style attribute.
- **Security and access control.** No new method, record, config or wire field.
  The stored language is checked against the available list before use. The
  `lang` value comes from a parsed tag. A stranger's text goes only through
  `textContent`, and Fluent does not parse argument values. The one new
  attribute (`datetime`) is computed by the Hub from a number.
- **Behaviour-change audit (section 4).** The ten rows match the code. I found
  no missed Hub behaviour except those in P1 and P6. The e2e list in 2.2 point
  10 is complete: I searched all specs and demo files for money, date, age and
  error text and found no other check on a changed string. The new `<select>`
  does not break the three e2e `select` locators: all three are scoped to
  `.report-form`, `.listing-editor` or a card.
- **`mise run verify` and CI.** `GATES` has no test that lists gate names or
  their count (only uniqueness and the docs-only rules). A new `hub-ui` gate
  with `skip_on_docs_only: true` needs no test change. `test:roym-ui` runs its
  own `npm ci || npm install`.
- **Planning-refs gate.** It scans `.ts` comments and does not scan `.ftl`, as
  the plan says. Its patterns do not match the ids the plan wants in comments.
- **Change doc placement (section 3).** Right. All five criteria and all 14
  decisions go to roym-multilingual. No `dep-versions` rule applies to
  `package.json` (the xtask check reads Cargo manifests only).
- **Backlog and living docs.** The two new rows (§10) are the only deferrals the
  plan makes. The row at line 508 stays. Section 9 lists the right living docs.
- **"How this is shown to an end user".** Present, 6 plain bullets, 3 developer
  bullets. One example is not exact (P11).
- **Terms and writing.** Terms are defined where they first appear. The text
  is plain.

### Views on the owner's questions

I do not answer for the owner.

- **Q1. (a), always India.** I agree. ML-4 asks for `₹1,00,000.50` "in both
  English and Hindi", and only a fixed region meets that in every browser.
  Playwright and most Chrome installs ask for `en-US`, so (b) would fail ML-4
  on the demo machine. Two notes: a catalog file whose tag already has a
  region (`pt-BR.ftl`) must keep its own region or the rule must say it is
  replaced (P9), and the backlog row for a later region setting is the right
  record.
- **Q2. (a), move the shell.** I agree. Without it the Hindi demo shows Hindi
  only in the cards and the `lang` attribute. The cost is small (about 25
  strings) and English text does not change.
- **Q3. (a), `<time datetime>`.** I agree, with a change: it works for rows
  where the date is the last thing in the row, and it cannot work where text
  follows the date (P1). For those rows use plain text from `formatDateTime`.
- **Q4. (a), save and reload.** I agree. The Hub already reloads after login
  and logout, so the pattern exists. The failure case when `localStorage` is
  blocked needs a rule (P7).
- **Q5. (a), add the `hub-ui` gate.** I agree. CI runs the suite and a local
  `verify` does not, so a Hub test can fail in CI only. The reason the plan
  gives is wrong, but the decision is right (P11). Measure the run time in
  Step 2 and write it into the plan result.
- **Q6. (a), the catalog decides.** I agree. It needs no caller change and keeps
  every English string. The rule "Hindi message shows `$detail` only after
  `विवरण:`" is testable. P6 lists the errors that never reach the table.

### Findings

#### P1 (major) — A date inside a sentence cannot be built as "label + `<time>` + text"

- **Where:** Step 5 item 4; Q3 (a); section 7.3 and Step 5 item 7 (the e2e
  check of `.quote-expired-notice time`); `quote.ts:206` to `:216`,
  `agreement_receipt.ts:226` and the matching `expired` rows.
- **Problem:** Step 5 item 4 says a row becomes "text + `<time>` + text +
  `<time>`", with the label from `t()` and the separator from
  `t("terms-range-separator")`. That works for `Schedule: A - B`, `Window: A - B`
  and `Observed: A`, where the date is the end of the row. It does not work for
  two rows where text follows the date:
  `` `Expires: ${expDate} (expired)` `` and
  `` `This quote expired on ${expDate}. Ask for a new one.` ``. Fluent returns a
  string, not DOM nodes. To put a `<time>` in the middle, the code must cut the
  sentence in two messages (a "before" and an "after" part). That makes the
  translator write half sentences, and Hindi puts the date in a different place
  (`यह कोटेशन {date} को समाप्त हो गया…`). The plan does not say which way to go.
  The plan's own e2e check (`.quote-expired-notice time`) depends on the answer.
- **Failure case:** The implementer cuts `quote-expired-notice` into
  `…-before` and `…-after`. The Hindi text then has an English word order, or
  the translator cannot say it right. Or the implementer passes the date as a
  string and the e2e check at Step 5 item 7 finds no `<time>` and fails.
- **Fix:** In Q3 (a) and Step 5 item 4, split the rows in two kinds. (1) Rows
  where the date ends the row or stands alone (`Schedule`, `Window`,
  `Observed`, `Expires: <date>`): a label message and a `<time>` element.
  (2) Rows with text after the date (`(expired)` suffix, the expired sentence):
  one message with a `$date` argument that is `formatDateTime(secs)` as plain
  text, with no `<time>`. Write that rule in the plan, change the e2e check at
  Step 5 item 7 to compare the visible text with the same `Intl` output computed
  in the test (or with a `datetime` check only on the `Expires:` row), and say
  in the template comment which rows have `<time>`. Add a test that
  `timeElement` throws or refuses in the same way as today for a value that
  cannot be a date (today `toISOString()` throws a `RangeError` for it).
- *Status:* Fixed (f10a0b15). Now in: Q3 owner answer; D-B1a-5 (two kinds of card row); Step 5 items 4, 6, 7; section 7.1 (`RangeError` test).

#### P2 (major) — `t()` has no English state before `initLanguage()`, and the existing tests and test hook need one

- **Where:** D-B1a-3 (`t()` returns the id and logs when English lacks it);
  Step 2 item 3 (`activate` builds the English bundle); Step 5 item 6 and
  section 7.1 ("Existing tests ... keep their English expectations"); the
  `vite.config.ts` test block (no `setupFiles`); `main.ts:62` and
  `roym-hub.spec.ts:237` (`window.RoymRegistry`, set at module load).
- **Problem:** The plan builds the English bundle inside `activate()`, which
  runs from `initLanguage()` in `main()`. Nothing calls it for a unit test that
  imports a card template, `groups/words.ts`, `money.ts` or `errText`. There are
  about 22 test files, and 8 of them are listed as "keeps its English
  expectations". With no active bundle, `t("terms-total", …)` returns the id and
  calls `console.error`. The plan names no vitest `setupFiles` and no default
  state. The same gap hits the page: `window.RoymRegistry.renderCard` is set at
  module load, and the e2e tests call it directly, so a call before `main()` has
  awaited `initLanguage()` would show ids.
- **Failure case:** After Step 5, `templates.test.ts` expects `Total:` and gets
  `terms-total`. About eight test files fail and the implementer adds a setup
  file ad hoc, or calls `activate("en")` in each test (async, so the tests need
  `await` in `beforeAll`).
- **Fix:** In D-B1a-3, make the English state synchronous and present at module
  load: parse `EN_SOURCE` once when `i18n.ts` loads, and start with
  `{ language: "en", active: english, english }`. `activate()` then only
  replaces `active` and `locale`. Say it in the plan: `t()` works in English at
  any time, never throws, and never returns an id for an id that `en.ftl` has.
  Keep a test-only reset (`activate("en")`) for tests that switched language.
  The unit tests then need no setup file, and `RoymRegistry` is safe.
- *Status:* Fixed (f10a0b15). Now in: section 2.2 point 14; D-B1a-3 (English state at module load); Step 2 item 4; section 7.1.

#### P3 (major) — A failed catalog load leaves the Hub blank

- **Where:** Step 6 item 1 (`await initLanguage()` as the first line of
  `main()`); D-B1a-1 (every non-English catalog is a separate script loaded
  with a dynamic `import()`); Step 2 item 3 (`activate`: "load, build bundle,
  set state").
- **Problem:** `import()` of the Hindi chunk is a second network request. It
  can fail: a poor mobile connection, an extension that blocks the request,
  a redeploy of Roym between the page load and the request (the old chunk name
  is gone). The plan does not say what `activate` does then. As written, the
  promise rejects, `main()` stops before it draws anything, and the page is
  empty, with one console error. The people who need Hindi are the ones with
  this extra failure point.
- **Failure case:** A person with `roym-hub-language = "hi"` opens the Hub on a
  weak connection. The chunk request fails. They see a white page. A reload
  fixes it only when the network does. They cannot reach the picker to choose
  English.
- **Fix:** In D-B1a-7 and Step 2, say that `initLanguage()` never rejects. On
  any error from the loader it calls `console.error`, keeps English active,
  and returns. `activate()` sets the state and `<html lang>` only after the
  load and the parse succeed, so `lang` never says `hi` while the text is
  English. Do not delete the stored choice on a failure (the next load may
  work). Add a test in `i18n.test.ts` with a loader that rejects: `t()` returns
  English and `lang` is `en`.
- *Status:* Fixed (f10a0b15). Now in: D-B1a-7 ("A failed load"); Step 2 item 4; Step 6 item 1; section 7.1.

#### P4 (minor) — Step 3 changes `formatMinor`, but the tests that read its output change in Step 5

- **Where:** Step 3 items 1 and 4; Step 5 item 6; the intro of section 6 ("Each
  step builds ... and its tests pass").
- **Problem:** Step 3 changes `formatMinor` to the `Intl` form. Step 5 item 6
  is the first place that fixes `cards/templates/templates.test.ts`. That file
  asserts `"4500.00 USD"`, `"360.00 USD"`, `"50.00 USD"`, `"250.00 EUR"`,
  `"125.00 EUR"` and three direct `formatMinor` checks (lines 78 to 80, 149,
  202, 248 and 361 to 363). Step 3 item 4 lists only `money.test.ts`.
- **Failure case:** After the Step 3 commit, `mise run test:roym-ui` fails. The
  commits are then not a safe base for `git bisect`, and the plan says every
  step passes.
- **Fix:** Add `cards/templates/templates.test.ts` (the money lines) to Step 3
  item 4. Keep its date lines (`toISOString`) for Step 5.
- *Status:* Fixed (f10a0b15). Now in: Step 3 item 5.

#### P5 (minor) — Word tables keyed by a stranger's value return inherited properties

- **Where:** Step 5 items 2 and 3 (`STATE_WORDS`, `CONFLICT_SENTENCES`, the
  label records become id tables); `booking_progress.ts:72` and `:95`;
  `labels.ts` (`serviceLocationCardLabel`).
- **Problem:** The tables are plain objects and the lookup is
  `TABLE[data.state] ?? data.state`. The value comes from a card from another
  node. A value such as `constructor` or `toString` finds an inherited
  function, so today the card shows function source text. After B1a the plan
  passes the looked-up value to `t()`. A function there is not an id: `t()`
  gets a non-string and, with the plan's rule, logs and returns it.
- **Failure case:** A card with `state: "constructor"` makes `t(Function)` run
  inside the card renderer. It does not crash, but it shows junk text, and the
  console fills with errors.
- **Fix:** In Step 5 items 2 and 3, require an own-property lookup
  (`Object.hasOwn(TABLE, value)` or a `Map`) before `t()`, and the "shown as it
  came" rule otherwise. Add one test per table with `constructor` and
  `__proto__`. Make `t()` return the id for a non-string argument and not
  throw.
- *Status:* Fixed (f10a0b15). Now in: section 2.2 point 15; D-B1a-15 (`Map` tables, `lookupText`); D-B1a-3 (non-string id); Step 5 items 2, 3, 6.

#### P6 (minor) — Some errors never reach the table, and the plan says "every code"

- **Where:** D-B1a-12 (the table and "The codes are every code that the Roym
  services and the `web` proxy return today"); `roym_web/src/app.rs:154` and
  `:163`; `rpc.ts` (`fetch` and `res.json()`); ML-6.
- **Problem:** The table has 12 rows. `app.rs` also returns `-32700` (parse
  error) and `-32600` (invalid request). They fall into the `other` row, which
  is fine, but the sentence "every code" is not true, and the Hindi `other`
  message must read well for them. Also, `call()` can throw errors that are not
  `RpcError`: `fetch` throws `TypeError` ("Failed to fetch" in Chrome, a
  different English sentence in Firefox) when the node is down, and
  `res.json()` throws `SyntaxError` for a non-JSON reply. D-B1a-12 sends these
  to "another `Error`: `err.message`" and calls them "Hub-made text". They are
  browser-made English text, and a Hindi user sees them on the most common
  failure (the node is not reachable).
- **Failure case:** The laptop sleeps, the node is off, and a Hindi user clicks
  a button. The Hub shows `Failed to fetch` in English, next to Hindi labels.
  This is the case ML-6 is about.
- **Fix:** Change the sentence in D-B1a-12 to "every code the Roym services
  and the `web` proxy return today, and the rest go to `error-service`". In
  `call()`, wrap a thrown `fetch` or `json()` error as
  `RpcError(0, String(error), "Other")`, and add `error-network` (code `0`) to
  the table with `$detail`. Check the callers that catch a non-`RpcError` from
  `call()` first (for example `whoami` and `login`). If one needs the old
  type, handle it there. Add the case to `errors.test.ts`.
- *Status:* Fixed (f10a0b15). Now in: Q6 owner answer; section 2.2 point 13; D-B1a-12 (code `0`, `error-network`, corrected sentence); section 4 row 6b; Step 4 item 1; section 7.1.

#### P7 (minor) — Blocked `localStorage` makes the picker do nothing

- **Where:** D-B1a-7 and D-B1a-8 (`saveLanguage` in `try`/`catch`, then
  `location.reload()`); Q4.
- **Problem:** When storage is blocked (some privacy modes), `saveLanguage`
  fails silently and the page reloads. `initLanguage` then picks the language
  from `navigator.languages` again. The person picks Hindi, the page reloads,
  and it is English again, with no message.
- **Failure case:** A person in a browser that blocks `localStorage` picks
  हिन्दी three times and sees no change.
- **Fix:** Let `saveLanguage` return a boolean. If it is `false`, do not
  reload: call `activate(tag)` and show a one-line note, or keep the choice in
  a module variable and tell the person it lasts until the tab closes. Add a
  test with a storage stub that throws. Optionally add the backlog row "the
  language switch reloads the page" (Q4 (a) is a deliberate shortcut, and the
  rule in AGENTS.md asks to record shortcuts).
- *Status:* Fixed (f10a0b15). Now in: Q4 owner answer; D-B1a-8 (`saveLanguage` returns a boolean, note, no reload); Step 6 item 5; section 8 (new backlog row).

#### P8 (minor) — The first-choice-from-browser path (ML-3) is tested only by a unit test

- **Where:** section 3 (ML-3 row); section 7.1 (`pickLanguage` cases);
  section 7.3; the end-user bullet "Someone whose browser asks for Hindi
  first sees the Hub in Hindi".
- **Problem:** `navigator.languages` is read in `initLanguage`, and no test
  runs that code with a real browser setting. The unit test calls
  `pickLanguage` with arrays. Playwright can set `locale`, so a real test is
  cheap. Story 40 only covers the switch and the reload.
- **Failure case:** A wrong call, for example `navigator.language` (one value)
  instead of `navigator.languages`, passes every unit test and fails only for
  a person with two preferred languages.
- **Fix:** In Step 7 or in `roym-hub.spec.ts`, add one test with
  `test.use({ locale: 'hi-IN' })` that opens the Hub and checks
  `html[lang]` is `hi` before any click. Add it to the list in section 7.3.
- *Status:* Fixed (f10a0b15). Now in: Step 7 item 3 (`hi-IN` describe in `roym-hub.spec.ts`); sections 3 and 7.3.

#### P9 (minor) — The format locale and the catalog file name have no rule for a region or a bad name

- **Where:** D-B1a-1 (the file name is the tag; "must parse with
  `new Intl.Locale`"); D-B1a-4 (`new Intl.Locale(uiLanguage, { region: "IN", … })`);
  ML-2 ("a new language needs no code change").
- **Problem:** Two gaps. (1) `Intl.Locale` with `region: "IN"` replaces any
  region in the tag (checked: `en-GB` gives `en-IN`), so a file `pt-BR.ftl`
  formats with `pt-IN`. Q1 (a) says "UI language with region `IN`", which does
  not say if a region the file already has is kept. (2) The plan says the
  name must parse, but nothing checks it. A file named `Hi_IN.ftl` or `hi.FTL`
  would be a language tag that `pickLanguage` cannot match or `Intl.Locale`
  refuses at load.
- **Failure case:** A later slice adds `ta-IN.ftl` and the picker shows an
  option whose tag never matches a browser tag in the same case, or the format
  locale falls back silently.
- **Fix:** Decide the rule in D-B1a-4 (recommended: add `IN` only when the tag
  has no region). Add a `catalog.test.ts` case: each `src/i18n/*.ftl` name,
  without the extension, equals `new Intl.Locale(name).toString()` (the
  canonical form) and is not `und`. This keeps the "no code change" promise
  honest.
- *Status:* Fixed (f10a0b15). Now in: Q1 owner answer; D-B1a-1 (canonical file name); D-B1a-4 (region kept when the tag has one); section 7.1.

#### P10 (minor) — Nothing checks the real chunk split, and the test runner and the build use different Vite versions

- **Where:** D-B1a-1; Step 8 item 1 (size report); section 7.6; section 7.1.
- **Problem:** `vitest` 4.1.11 brings its own Vite (6.4.3 in
  `node_modules/vitest/node_modules/vite`), while `npm run build` uses Vite
  5.4.21. Both support `import.meta.glob` with `query` and `import`, so the
  unit tests will run. But a unit test cannot show what the build produces:
  that `hi.ftl` is a separate script, that `en.ftl` is in the main script, and
  that `bundle.tar.gz` holds both. The plan's whole size story (section 7.6)
  and ML-2 ("a new file is a new language") depend on it. I could not check it
  here (see "Limits").
- **Failure case:** The build puts the Hindi text in the main script (for
  example because of the `!./en.ftl` pattern or a plugin option). Tests pass.
  The size report shows it only if the implementer reads the file list.
- **Fix:** Add to Step 8 item 1 a hard check, with output in the PR: after
  `npm run build`, `dist/assets` has one script that contains `हिन्दी` or a
  Hindi message and one that does not, the main script has no Devanagari
  letter from `hi.ftl`, and `tar -tzf bundle.tar.gz` lists both. Do this
  check at the start of Step 2 with two dummy messages, before the rest is
  built on it. Note in the plan that the unit tests and the build use
  different Vite versions.
- *Status:* Fixed (f10a0b15). Now in: section 2.2 point 16; Step 2 item 1 (split check first); Step 8 item 1; exit criterion 2.

#### P11 (minor) — Small facts in the plan text that are wrong or unclear

- **Where:** the "Size and PR" paragraph; Q5; "How this is shown to an end
  user"; section 7.1.
- **Problem:** Four items.
  (a) "It changes ... no Rust code outside two tests and one doc comment" is
  not true if Q5 (a) is taken: `xtask/src/verify.rs` is Rust. Exit criterion
  11 lists it, so the paragraph disagrees with the list.
  (b) Q5 says the new gate may run before `nextest` because "the nextest gate
  already builds the Hub, so `npm ci` has run by then". The gate is placed
  before `nextest`, so that is not the order. It is not needed: `test:roym-ui`
  runs its own `npm ci || npm install`. The effect is a second `npm ci`.
  (c) The end-user text gives `15 Jan 2027, 1:30 pm IST` as the example for
  "her language's style". In Hindi the date is `15 जन॰ 2027, 1:30 pm IST`
  (checked). The English example is right.
  (d) Section 7.1 lists `dom.test.ts` among the tests that change. It has no
  `errText` test (a search of the test files finds none), so the change is an
  addition, not an edit.
- **Failure case:** None breaks the code. A new reader follows (a) or (b) and
  spends time on a wrong premise.
- **Fix:** Fix the four sentences. In (b) say the gate needs only
  `mise run test:roym-ui`.
- *Status:* Fixed (f10a0b15). Now in: (a) "Size and PR" paragraph; (b) Q5 text and owner answer; (c) end-user bullet with the Hindi date; (d) section 7.1, `dom.test.ts` sentence.

#### P12 (minor) — The catalog test scans source text and can be fooled

- **Where:** section 7.1 (`catalog.test.ts`: "Every id in a `t("…")` or
  `messageId("…")` call", "No id is unused").
- **Problem:** The test reads all `.ts` files with `?raw` and matches `t("…")`
  with a regular expression. Three cases are not covered. (1) A comment or
  doc string that shows `t("some-id")` (the plan wants such an example in
  `i18n.ts` and the developer guide) counts as a use, so a dead id looks used.
  (2) `t(\`error-${x}\`)` or `t(id)` with a variable is not seen at all, so a
  missing id fails only at run time. (3) The regular expression `t(` also
  matches `format(`, `list(`, `set(`, because a word ends in `t`. A missing
  word boundary would produce false ids.
- **Failure case:** A dead Hindi id stays in the file for months because an
  example comment keeps it "used". Or a typo in an id built by hand shows as
  the id on a card.
- **Fix:** In the plan, say: strip comments before the scan, use a word
  boundary (`(?<![\w.])t\(`), reject a non-literal first argument in any
  `t(` call outside `i18n.ts` and the `messageId` tables (the test lists them),
  and keep the `messageId` rule as the only way to pass an id by variable.
  This is also the first half of the B1b build check, so write it once.
- *Status:* Fixed (f10a0b15). Now in: section 7.1 ("How the source scan works", `src/i18n/scan.ts`); D-B1a-3 (`lookupText`, literal ids only).

## Plan review round 2 — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-06

Checked on `feat/m08b-b1a-localization-mechanism` at `85ce9a85`, in the main
checkout (not a worktree). `git diff 6e842834..HEAD` shows only three docs
files: the plan, this review file and `status.md`. No code changed, so the
code facts of round 1 still hold.

**Summary:** P1 to P12 are all confirmed fixed. 2 new items, both minor:
0 blocker, 0 major, 2 minor. **2 open (P13, P14).** Both fixed in `96e0120e`; 0 open.

Limits: I read the whole new plan text (1,235 lines) and checked it against
the code. I ran the `Intl` and Fluent claims again, now on Node 20.20.2 (the
version `mise.toml:26` pins; round 1 used 22.11). I did not run a Vite build
of the Hub (the sandbox refused it in round 1), so I read Vite's source
instead (see P10 below). I ran no plan step.

### Round 1 items — confirmed

- **P1 — confirmed fixed.** D-B1a-5 splits the rows in two kinds. The kind 2
  rows are the ones I named: `quote.ts:210` (`Expires: … (expired)`) and
  `quote.ts:219` (the expired sentence). `agreement_receipt.ts:226` has no
  expired variant, so it is kind 1 (`Quote expiry:`), as the plan says. The e2e
  check at Step 5 item 7 no longer needs a `<time>` in a sentence. The e2e
  expiry in `roym-hub.spec.ts:1095` is `1600000000`, which is 13 September 2020
  in every zone, so the year check `2020` holds. The unexpired quote at
  `:840` has `Expires:`, so the added `time[datetime]` check applies. The
  `RangeError` claim holds: `new Date(NaN).toISOString()` and
  `Intl.DateTimeFormat(…).format(new Date(NaN))` both throw `RangeError`
  (run on Node 20).
- **P2 — confirmed fixed.** D-B1a-3 now starts with English at module load,
  Step 2 item 4 says the same, and section 7.1 has the fresh-module test.
  The 8 test files and `window.RoymRegistry` (`main.ts:62`) need no setup.
- **P3 — confirmed fixed.** `initLanguage()` never rejects, `activate()` sets
  the state and `lang` only after load and parse, the stored choice stays,
  Step 6 item 1 repeats it, and section 7.1 has the rejecting-loader test.
- **P4 — confirmed fixed.** Step 3 item 5 lists exactly the money lines of
  `templates.test.ts`: `:78` to `:80`, `:149`, `:202`, `:248` and
  `:361` to `:363` are all right. No other unit test asserts a money string (a
  search for amounts with USD, EUR, INR, GBP, JPY or KWD finds only this file
  and `money.test.ts`).
- **P5 — confirmed fixed.** `STATE_WORDS` and `CONFLICT_SENTENCES`
  (`booking_progress.ts:34`, `:43`) are the only tables read with a card
  value; the label records in `labels.ts` are read with a Hub value, except
  `serviceLocationCardLabel`, which Step 5 item 2 keeps with its rule. All
  become `Map`s read with `lookupText`. The users of the old records are
  `wording.test.ts:49` and `:50` and `labels.test.ts`, which Step 5 item 6
  already lists.
- **P6 — confirmed fixed.** Code `0` and `error-network` are in D-B1a-12,
  section 4 row 6b, Step 4 item 1 and section 7.1. The seven catch-all callers
  that section 2.2 point 13 names exist (`memberships.ts:32`, `safety.ts:23`,
  `:69`, `:148`, `:206`, `synorg_members.ts:36`, `messages.ts:406`; each is a
  bare `catch {`). No source or test checks for `TypeError` or `SyntaxError`.
  The one place that reads the text of a thrown error, `messages.ts:637`, gets
  the same words from `RpcError(0, …)` as from the old `String(err)` for a
  failed fetch. `search.test.ts:85` and `:104` assert English sentences that
  stay the same.
- **P7 — confirmed fixed.** D-B1a-8, Step 6 item 5, the end-user bullet and a
  third backlog row all cover blocked storage. The backlog row is right: Q4 (a)
  is a deliberate shortcut.
- **P8 — confirmed fixed.** The nested `hi-IN` describe is safe: the outer
  `beforeEach` of `Roym Hub` (`roym-hub.spec.ts:47`) only asserts an
  environment variable, `playwright.config.ts` sets no `locale`, and
  `.login-picker h2` exists (`main.ts:183` to `:185`). `pickLanguage(null,
  ["hi-IN"], ["en","hi"])` matches (`language` `hi`, script `Deva` on both).
- **P9 — confirmed fixed.** The rule in D-B1a-4 is right: `pt-BR` gives
  `pt-BR-u-nu-latn` with the plan's code, and `pt-IN-u-nu-latn` without it (run
  on Node 20). `new Intl.Locale("Hi_IN")` and `("hi.FTL")` throw, `("hi")` and
  `("ta-IN")` give themselves, so the `catalog.test.ts` rule works (a throw
  also fails the test).
- **P10 — confirmed fixed.** Step 2 item 1, Step 8 item 1 and exit criterion 2
  carry the check. I read Vite 5.4.21: its minifier is set to `charset:
  "utf8"` (`node_modules/vite/dist/node/chunks/dep-BK3b2jBa.js:19270`), so
  Devanagari letters stay as letters in the bundle and the `grep -c
  '[ऀ-ॿ]'` check is meaningful. The grep works in the default (empty) locale and
  does not match `₹`, `…` or `é` (negative control run).
- **P11 — confirmed fixed.** The four sentences are right. `mise run
  test:roym-ui` is a complete task (`mise.toml:96`), `nextest` reaches `npm
  ci` through `build:roym` → `build:roym-ui`, and the Hindi date example
  `15 जन॰ 2027, 8:00 am UTC` matches the engine's output (the plan shows
  `1:30 pm IST`; same form).
- **P12 — confirmed fixed, with one more point (P14).** The helper, word
  boundary, comment stripping and non-literal rule are in section 7.1. The
  comment stripper itself needs one more rule, below.

**Other things I checked again.**

- **Owner answers.** All six are (a), as I recommended. Each answer is written
  under its question, and sections 4 to 9 match (for example: Q3 → D-B1a-5,
  Q4 → D-B1a-8 and the third backlog row, Q5 → Step 2 item 9, Q6 → D-B1a-12).
- **The `hub-ui` gate.** The `Gate` fields in `verify.rs:18` to `:29` are the
  ones the plan uses, `planning-refs` (`:97`) comes before `nextest` (`:103`),
  and CI already runs the same task (`ci.yml:78`).
- **Node 20.** The repo pins Node 20, and the plan's numbers were run on 22.
  I ran them again on 20.20.2: `₹1,00,000.50` and `₹9,00,71,99,25,47,409.91`
  (en and hi), `$35.50`, `JP¥1,200`, `KWD 2.500`, `-€45.00`, `1,00,000.5`,
  `21 Sept 2026`, `5 minutes ago`, `1 hour ago`, `3 days ago`, the Hindi
  relative forms, the RangeError for a bad currency code, `हिन्दी` and
  `English` from `DisplayNames`. All equal the plan, so the hard-coded
  `21 Sept 2026` in `format.test.ts` also holds on Node 20.
- **Fluent behaviours (0.19.1, run).** A missing variable gives
  `{$detail}` in the text plus one error, so `t()` must log and the
  `errors.test.ts` and `catalog.test.ts` sample args matter, as planned. An
  argument value such as `constructor` is printed as text. A trailing space in
  a message is removed (P13).
- **Demo contexts.** Each demo story gets a fresh browser context
  (`demo/steps.ts:80` and the Playwright default), so a stored `hi` does not
  reach other stories, as the plan says.

### New findings

#### P13 (minor) — Fluent removes the space at the end of a message, so labels and the range separator lose their spaces

- **Where:** D-B1a-5 "Card rows with a time" (a label message
  `terms-schedule-label = Schedule:`, a `t("terms-range-separator")` text node
  between two times); Step 5 items 1 and 4; D-B1a-3. The five existing labels
  that end with a space and are followed by another node:
  `quote.ts:140` and `:192`, `agreement_receipt.ts:166` and `:215`,
  `payment_request.ts:55` (`"Payee, as agreed in this quote: "`,
  `"Dispute path: "`).
- **Problem:** Fluent removes spaces at the start and end of a message value.
  I ran `@fluent/bundle` 0.19.1: `label = Schedule: ` (with a trailing space)
  gives `"Schedule:"`, and with two trailing spaces too. So a separator
  message written as ` - ` cannot keep its spaces. Today's code has the space inside the string (`"Dispute path: "`,
  `` `Schedule: ${start} - ${end}` ``). The plan moves these into messages and
  appends the next node, but names no rule for the space. The plan also says
  English output does not change.
- **Failure case:** The implementer writes `terms-dispute-label = Dispute path:`
  and appends the link. The card shows `Dispute path:https://…`. The kind 1 rows
  show `Schedule:15 Jan 2027, 1:30 pm IST-16 Jan 2027, …`. No Hub unit test
  checks this text (a search for `Payee, as agreed` and `Dispute path` in
  `*.test.ts` finds nothing). Only the e2e checks on `Payee, as agreed in this quote: …`
  (`roym-hub.spec.ts:839`, `:847`, `:1092`) would fail, late, at the end of
  Step 5. Hindi has the same trap.
- **Fix:** Add one rule to D-B1a-3 and Step 5 item 1: a message never starts or
  ends with a space, because Fluent removes it. Two ways to keep a space, pick
  one and write it down: (1) the template adds it (a small helper
  `labelled(labelId, ...nodes)` that appends the label, one `" "` text node,
  then the nodes); (2) the message keeps it with a string-literal placeholder
  (`terms-range-separator = { " - " }`, `label = Payee:{ " " }`; I ran both and
  the spaces stay). Recommended: (1) for labels, (2) for the range separator,
  because a translator may want a different separator. Add two checks: a test
  in `catalog.test.ts` that no entry line of a `.ftl` file ends with
  whitespace (the parser would silently drop it, like junk), and a unit test
  that compares the full `textContent` of the quote's `Payee`, `Dispute path`
  and `Schedule` rows with the old English output.
- *Status:* Fixed (96e0120e). Now in: D-B1a-3 ("Spaces at the ends of a message": `labelled()` for labels, `{ " - " }` for the separator); D-B1a-5 kind 1 rows; Step 5 items 1, 4 and 6 (full-row `textContent` test); section 7.1 (`.ftl` trailing-whitespace test).

#### P14 (minor) — The comment stripper of the catalog test is not string-aware

- **Where:** section 7.1 "How the source scan works", step 1 ("Remove comments
  first (`//` to end of line, and `/* … */`)"); `src/i18n/scan.ts`.
- **Problem:** Cutting everything after `//` also cuts text after a `//` that
  sits inside a string. Five lines in the cards that B1a edits have one:
  `if (t.payee.startsWith("http://") || t.payee.startsWith("https://"))`
  (`quote.ts:142`, `:194`, `agreement_receipt.ts:168`, `:217`,
  `payment_request.ts:57`). Today no `t(` follows on those lines, so nothing
  breaks. A line such as `x.startsWith("http://") ? t("a") : t("b")` would hide
  both ids: the test then reports `a` and `b` as unused, or (worse) a typo in
  one of them passes the "every id is in `en.ftl`" check. A regex literal
  or a template string with `//` has the same effect.
- **Failure case:** A later slice (B1b moves hundreds of lines) writes a
  condition with a URL scheme before a `t()` call on one line. The "no id is
  unused" test fails for a good reason nobody can see, or the missing-id check
  misses a typo.
- **Fix:** Do not use a regular expression to find comments. `typescript` is
  already a dev dependency (`package.json:19`): in `scan.ts` parse each file
  with `ts.createSourceFile` and walk the call expressions. A call whose callee
  is the identifier `t` or `messageId` (not `x.t`) is a use; its first
  argument must be a string literal (or a no-substitution template). This
  ignores comments, strings and `format(` for free, and the "word boundary" and
  "non-literal" rules become simple tests of the syntax tree. Keep the four
  helper tests of section 7.1, and add one with `"http://"` before a `t("x")`
  on the same line. If the plan keeps a text scan instead, write that a
  `//` counts as a comment only when it is at the start of the line (after
  spaces) or follows a space, and that string and template contents are
  skipped first.
- *Status:* Fixed (96e0120e). Now in: section 7.1 ("How the source scan works": `ts.createSourceFile`, plain-identifier callee, renamed imports refused, `"http://"` helper test). Plan-author note: the parser also exposed local variables named `t` (`quote.ts:77`, `agreement_receipt.ts:108`, `render.ts:22`, `money.ts:50`, `listings/editor.ts:118`, `:131`) that would hide the imported `t()`; Step 5 item 2 renames them.

## Plan review round 3 — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-06

Checked on `feat/m08b-b1a-localization-mechanism` at `c4f13195`, in the main
checkout (not a worktree). The fix commit `96e0120e` changes only the plan,
this review file and `status.md`, so no code changed.

**Summary:** P13 and P14 are confirmed fixed. 1 new item, minor: 0 blocker,
0 major, 1 minor. **1 open (P15).** Fixed in `78069aca`; 0 open.

### Round 2 items — confirmed

- **P13 — confirmed fixed, with one new point (P15).** D-B1a-3 now has the
  rule "a message never starts or ends with a space" and the two ways to keep
  a space. I ran both again on `@fluent/bundle` 0.19.1: `label = Schedule: `
  gives `"Schedule:"`, and `terms-range-separator = { " - " }` gives `" - "`.
  The five labels in Step 5 item 1 are the right ones (`quote.ts:140`,
  `:192`, `agreement_receipt.ts:166`, `:215`, `payment_request.ts:55`). No
  selector in the unit tests or e2e specs reads the label `<span>` (they read
  `.quote-payee a` and `.quote-payee` text), so `labelled()` may drop it.
  The full-row `textContent` test (Step 5 item 6) and the `.ftl`
  trailing-whitespace test (section 7.1) are the right checks.
- **P14 — confirmed fixed.** The scan now parses with `ts.createSourceFile`
  (`typescript` is a dev dependency, `package.json:19`), skips comments and
  strings, and has the `"http://"` helper test. The shadowing list that the
  plan author added is complete: a search of all non-test sources finds local
  names `t` only at `quote.ts:77`, `agreement_receipt.ts:108`, `render.ts:22`,
  `money.ts:50` and `listings/editor.ts:118`, `:131`. Local `t` in a file that
  calls `t()` is a type error (`AgreedTermsData` and `string` are not
  callable), so the build catches a missed one. `tsconfig.json` has
  `include: ["src"]`, so `scan.ts` is type-checked by `tsc` and linted, and
  `vite build` leaves it out of the bundle because `main.ts` never imports it.

### New findings

#### P15 (minor) — `labelled(parent, labelId, …)` calls `t()` with a variable, which the plan's own scan refuses

- **Where:** D-B1a-3 "Spaces at the ends of a message", rule 1;
  D-B1a-5 kind 1 rows (`labelled(row, "terms-schedule-label", …)`); section
  7.1 "How the source scan works", step 2.
- **Problem:** `labelled` lives in `src/dom.ts` and "appends `t(labelId)`".
  That is a `t()` call with a non-literal first argument. The scan refuses
  that everywhere except `src/i18n/i18n.ts`. Also, the callers pass the id as
  a plain string (`"terms-schedule-label"`). The scan sees no `t("…")` or
  `messageId("…")` at the call site, so the id is never counted as used.
- **Failure case:** The catalog test fails on `dom.ts` the first time it runs
  (so the implementer adds an exception for `dom.ts`, which weakens the rule),
  or reports `terms-schedule-label` and the other label ids as unused. If the
  exception is added, a typo in a label id passes the "every id is in
  `en.ftl`" check and shows the raw id on a card.
- **Fix:** Make `labelled` take the translated text:
  `labelled(parent, label: string, ...nodes)`. Callers write
  `labelled(row, t("terms-schedule-label"), …)`. Every `t()` call then has a
  literal id at the call site, `dom.ts` needs no exception, and the scan counts
  the id. Change rule 1, the kind 1 text and Step 5 item 1 to match.
- *Status:* Fixed (78069aca). Now in: D-B1a-3 rule 1 (`labelled(parent, label: string, …)`, caller writes `t("…-label")`); D-B1a-5 kind 1 rows; Step 5 item 1.
