# M08B slice B1a — Hub localisation, the mechanism: implementation plan

*Reader: the developer (or AI) who implements slice B1a. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan written, 2026-11-06. The owner questions Q1 to Q6
(section 2.1) are open.**

**Size and PR.** M. One PR on `feat/m08b-b1a-localization-mechanism`, with
the commits of section 6 in that order. The branch starts from `main` at
`9e19b888` (S0 is merged). B1a adds the Hub's string catalog and its
lookup, a language setting, local formats for money, dates and times, and
error messages by error code. It moves the four word tables, the card text
and the Hub shell (`main.ts`) into the catalog. It changes no JSON-RPC
method, no WIT file, no signed record and no Rust code outside two tests
and one doc comment.

**Base.** Every file path and line number below was checked on `main` at
`9e19b888`. The task.md facts were checked on `76b52fe6`; the commits
between change only planning docs, so they still hold, except where
section 2.2 corrects them.

**Parallel work.** B4a (substrate only) may run beside B1a. It touches no
file that B1a touches ([task.md](task.md) section 4).

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`). Today it does not run the Hub's vitest suite (section
    2.2, point 2). Q5 asks to add it.
  - No planning ids in code or comments: no "M08B", "B1a", "ML-4" or
    "D-B1a-3" in `.ts`, `.ftl`, `.rs` files or test names. The gate
    `planning-refs` scans `.ts` comments. It does not scan `.ftl` files,
    but the rule still holds there.
  - Comments never cite size rules.
  - The import clean-up pass (Rust files only change in two tests here),
    the deferred backlog, the living-docs close-out and the demo close-out.
  - The communication style: the Hindi text is user text, so it must be
    plain, common Hindi, not formal or Sanskrit-heavy words.
- [task.md](task.md): section 1 (Y1), section 2 ("Hub"), the "Demo"
  paragraph, "B1a" and "B1b", section 6 (owner answers 2, 3 and 4).
- [roym-multilingual change doc](../../changes/roym-multilingual/change.md):
  "A. UI localisation", "Which catalog format", "Rules for every layer",
  criteria ML-2 to ML-6, "Open questions" (native digits, Hindi review),
  "Deviations".
- task.md names one more change doc,
  [demo-stories](../../changes/demo-stories/change.md), in its "Demo"
  paragraph. B1a adds story 40 under its rules ("Two kinds of story, one
  runner", "Link between slice plans, the demo and AGENTS.md"). B1a meets
  no criterion of that doc and records nothing there.
- [slice-s0-implementation-plan.md](slice-s0-implementation-plan.md) and
  [slice-s0-review.md](slice-s0-review.md). Section 7.8 lists how B1a
  avoids the kinds of findings S0 had.

### Terms used in this plan

- **Hub:** the Roym web UI in the browser (`crates/roym_web/ui`).
- **String catalog:** the file of all Hub text for one language. One file
  per language: `src/i18n/en.ftl`, `src/i18n/hi.ftl`. Always say "string
  catalog" in docs and comments. "Catalog" alone is also the name of a Roym
  service (`catalog`, listings).
- **Fluent:** Mozilla's message format. A `.ftl` file holds messages like
  `tab-profile = Profile`. `@fluent/bundle` is its JavaScript library. The
  owner chose it (task.md question 2).
- **Message id:** the name on the left of `=` in a `.ftl` file, for example
  `tab-profile`.
- **UI language:** the language of the Hub's own text. It is a device
  setting in this browser. It is not the profile's `languages` list (B3a),
  which tells other people what the person speaks.
- **Format locale:** the BCP 47 tag that `Intl` gets for numbers, money and
  dates. It is built from the UI language (section 5, D-B1a-4).
- **Shell:** the parts of the Hub that `src/main.ts` draws: title, session
  bar, tab buttons, sign-in box and the developer card gallery.

## 1. What B1a builds, in one paragraph

B1a builds the mechanism that every later slice uses for Hub text. A new
module `src/i18n/i18n.ts` loads Fluent string catalogs and gives one
function, `t(id, args)`, that returns the text for the UI language, with
English as the fallback. The UI language comes first from the browser's
preferred languages, and then from a choice the person makes in a new
language list in the session bar. The choice is kept in `localStorage`, so
it survives a reload. A second module, `src/i18n/format.ts`, formats
numbers, money, dates, times and ages with `Intl` and the UI language, with
Latin digits. The Hub sets `lang` on the page. Error messages come from the
string catalog by error code; the service's own English text appears only
as a detail. B1a moves the text of the four word tables, every card, and
the shell into the string catalog, with an English and a Hindi version of
each message. Every other screen keeps its English text until B1b.

## How this is shown to an end user

- Alice opens her Hub. In the bar at the top, next to "Log out", there is
  a new list of languages: "English" and "हिन्दी".
- She picks "हिन्दी". The page loads again. The Hub's title, the tab names,
  the sign-in box and every card in her chats are now in Hindi. The other
  screens still show English text; slice B1b changes them.
- Money shows like `₹1,00,000.50`, with Indian digit grouping. Dates and
  times show in her language's style, for example `15 Jan 2027, 1:30 pm IST`.
- She reloads the page, or opens the Hub again later in the same browser.
  It is still in Hindi.
- When the node refuses an action, the Hub says so in Hindi. The service's
  own English words show only after a "विवरण:" ("details:") label.
- Someone whose browser asks for Hindi first sees the Hub in Hindi from the
  start, with no choice to make.

How a developer can see it work:

- `mise run demo` (or `npm run test:demo` in `crates/substrate/tests/e2e`)
  runs story `demo/40-roym-languages.spec.ts`. It prints the captions "sign
  in", "switch the Hub to Hindi" and "reload: still Hindi", and passes.
- `mise run test:roym-ui` runs the Hub unit tests. The new files
  `i18n.test.ts`, `format.test.ts`, `catalog.test.ts` and `errors.test.ts`
  show the language choice, the formats (`₹1,00,000.50` in both
  languages) and the error messages.
- In a running Hub, `document.documentElement.lang` is `hi` after the
  switch, and `localStorage.getItem("roym-hub-language")` is `"hi"`.

## 2. Points for the owner

### 2.1 Questions

Each question names what it blocks. The recommendation is the first
option.

**Q1. Which region do the formats use? (blocks Step 3)**

ML-4 asks for `₹1,00,000.50` with Indian grouping "in both English and
Hindi". `Intl` takes the grouping from the region of the locale, not from
the currency. Checked in Node 22: `en-IN` gives `₹1,00,000.50`, `en-US`
gives `₹100,000.50`. Many people in India use a browser set to `en-US`
(it is the default install language of Chrome). Playwright also asks for
`en-US`.

- (a) **Always India.** The format locale is the UI language with region
  `IN` and Latin digits: `en-IN-u-nu-latn`, `hi-IN-u-nu-latn`. A new
  language gets `<tag>-IN` with no code change (ML-2). A person outside
  India sees Indian grouping for every currency (`€1,00,000.00`). A
  backlog row records a later "region" option. *Recommended:* it meets
  ML-4 for every browser, and Roym's first users are in India.
- (b) **The browser's region.** Use the region of the first browser
  language that matches the UI language; `IN` when it has none. An `en-US`
  browser then shows `₹100,000.50`, so ML-4 holds only for `en-IN`
  browsers.
- (c) **No region.** `Intl` picks: `en` gives Western grouping. Fails ML-4
  for English.

**Q2. Does B1a move the shell (`main.ts`) too? (blocks Step 6)**

task.md says "Move the four word tables and the card text into the
catalog. Screens follow in B1b." `main.ts` is not under `screens/`, and it
is where the language list goes. Without the shell, the Hindi demo of B1a
shows the language list and the page `lang`, but every visible word above
the cards stays English.

- (a) **Yes.** B1a moves the about 25 shell strings of `main.ts` (title,
  session bar, tab names, sign-in box, card gallery headings). The sample
  card data in `renderHome` stays data. English stays the same, so
  `hub-helpers.ts` keeps finding the tab buttons by name. *Recommended:*
  the demo then shows a Hindi Hub, and B1b is smaller.
- (b) **No.** The shell waits for B1b. Story 40 checks only the language
  list and `html[lang]`.

**Q3. How do cards show times? (blocks Step 5)**

Cards show schedule, window, expiry and payment times as UTC ISO strings
today (`new Date(...).toISOString()`, 6 call sites, section 2.2 point 6).
ML-4 asks for the Hub language's format.

- (a) **Local time with the zone name, inside a `<time>` element.** The
  text is `formatDateTime(secs)`, for example `15 Jan 2027, 1:30 pm IST`;
  the element has `datetime="<ISO string>"`. The ISO value is computed by
  the Hub from a number, never copied from a string field of the card, so
  the card rule "no attributes from data other than classes" is kept in
  spirit; the template comment says so. Tests read the `datetime`
  attribute, so they do not depend on `Intl` output that differs between
  Node and Chromium. *Recommended.*
- (b) **Keep UTC ISO strings.** Simple, but cards break ML-4.

**Q4. What happens when the person picks a language? (blocks Step 6)**

- (a) **Save the choice and reload the page.** The session token is in
  `sessionStorage` and survives a reload. A half-typed form is lost. The
  screens build their DOM once and keep no list of their texts, so a
  reload is the only simple way to redraw everything. *Recommended.*
- (b) **Redraw without a reload.** Every screen would need a redraw hook.
  Much more code for a rare action.

**Q5. Does `mise run verify` run the Hub unit tests? (blocks Step 2)**

The Hub's vitest suite runs in CI (`.github/workflows/ci.yml:78`, `mise
run test:roym-ui`) but not in `mise run verify`
([verify.rs](../../../../xtask/src/verify.rs) `GATES`). B1a adds most of
its tests there (the string catalog checks, the formats, ML-2 to ML-6). The
nextest gate already builds the Hub (`mise run build:roym` depends on
`build:roym-ui`, which runs `eslint`, `tsc` and `vite build`), so `npm ci`
has run by then.

- (a) **Add a gate `hub-ui` in B1a**, running `mise run test:roym-ui`,
  after `planning-refs` and before `nextest`, `skip_on_docs_only: true`.
  The suite has about 20 test files; its run time is not measured here.
  *Recommended:* a local verify then fails on the same Hub test that CI
  fails on.
- (b) **Leave it to CI.**

**Q6. Where does the service's English text go in a Hindi error? (blocks
Step 4)**

ML-6: no English service text for a Hindi user "except as an extra detail
line". `errText()` returns one string to 54 callers that put it into
`textContent`.

- (a) **The string catalog decides.** Each error message has a `$detail`
  variable. The English message is often just `{ $detail }`, so English
  text does not change. The Hindi message is a Hindi sentence, then
  `विवरण: { $detail }` at its end. A test checks that every Hindi error
  message starts with Devanagari text and uses `$detail` only after
  `विवरण:`. *Recommended:* no caller changes, and English e2e text stays.
- (b) **A real second line.** `errText()` returns `{ text, detail }`, and
  each of the 54 callers draws a second element. Much more change, all in
  screens that B1b moves anyway.
- (c) **No English at all** in Hindi. Loses the only clue for a support
  question.

### 2.2 Points found while reading

1. **Two Rust tests read the word-table files.**
   `the_ui_wording_matches_this_crate`
   ([booking/tests.rs:296](../../../../crates/roym_core/src/booking/tests.rs#L296))
   reads `cards/wording.ts` and checks that 8 Rust constants appear
   verbatim. `notice_constants_match_hub_ui_verbatim`
   ([group/tests.rs:189](../../../../crates/roym_core/src/conversation/group/tests.rs#L189))
   does the same for 10 group notices in `groups/words.ts`. The doc comment
   at [group.rs:17](../../../../crates/roym_core/src/conversation/group.rs#L17)
   names `words.ts`. When these strings move into `en.ftl`, both tests must
   read `src/i18n/en.ftl` instead (D-B1a-11). The other files that Rust
   tests read (`money.ts`, `cards/registry.ts`, `session/enrolment.ts`,
   `directory/membership.ts`, `screens/messages.ts`) keep their constants
   in B1a. `money.rs`'s test finds the first `EXPONENT_0` and `EXPONENT_3`
   in `money.ts` ([money.rs:133](../../../../crates/roym_core/src/money.rs#L133)),
   so the two sets stay the first use of those names in the file.
2. **The Hub's vitest suite is not in `mise run verify`.** See Q5.
3. **Fluent wraps every variable in Unicode isolation marks by default.**
   `new FluentBundle(locales, { useIsolating = true })` (checked in the
   `@fluent/bundle` 0.19.1 source, `esm/bundle.js:31`). Every `{ $x }`
   becomes `U+2068 … U+2069`. Then `Total: €45.00` in a test or an e2e
   check would not match, and a copied amount would carry invisible
   characters. The Hub has no right-to-left script (a non-goal), so B1a
   sets `useIsolating: false` and adds a backlog row (D-B1a-2).
4. **Fluent's runtime parser drops a broken message without an error.**
   `FluentResource` skips junk silently (`esm/resource.js:63`: "Don't
   report any Fluent syntax errors"), and `addResource` reports only
   overrides. A typo in `hi.ftl` would fall back to English with no sign.
   B1a's catalog test counts entry lines against parsed messages
   (section 7.1).
5. **Fluent formats a number variable with digit grouping.** A number arg
   becomes a `FluentNumber`, formatted with `Intl.NumberFormat`
   (`esm/resolver.js:130`). An error code `-32602` would print as
   `-32,602`. Rule: pass codes, ids and years as strings; pass numbers only
   for counts and amounts that should be formatted (D-B1a-3).
6. **More date call sites than task.md counts.** task.md lists the 4
   `toLocale*` calls. Cards also format 6 times with `toISOString()`:
   [quote.ts:154](../../../../crates/roym_web/ui/src/cards/templates/quote.ts#L154),
   `:155`, `:206`;
   [agreement_receipt.ts:179](../../../../crates/roym_web/ui/src/cards/templates/agreement_receipt.ts#L179),
   `:180`, `:226`;
   [request.ts:62](../../../../crates/roym_web/ui/src/cards/templates/request.ts#L62),
   `:63`;
   [payment_acknowledgement.ts:55](../../../../crates/roym_web/ui/src/cards/templates/payment_acknowledgement.ts#L55)
   (9 calls in 4 files). `directory/words.ts` has `ageWords` and
   `dateWords` (UTC date, `2026-09-29`), with 10 callers in
   `directory/membership.ts`, `screens/directory.ts` and
   `screens/synorg_members.ts`. B1a covers all of them (Step 3, Step 5).
7. **`errText` exists three times.**
   [dom.ts:12](../../../../crates/roym_web/ui/src/dom.ts#L12),
   [messages.ts:96](../../../../crates/roym_web/ui/src/screens/messages.ts#L96)
   and [listings.ts:29](../../../../crates/roym_web/ui/src/screens/listings.ts#L29).
   B1a keeps only `dom.ts`. The `listings.ts` copy also accepts
   `ListingInputError`, which is an `Error`, so `dom.ts` already covers it.
8. **The 22 `.message` reads.** The count in task.md is right. The list,
   and what B1a does with each, is in Step 4. One read is a check by
   English words: `msg.includes("quote-expired") || msg.includes("expired")`
   ([messages.ts:637](../../../../crates/roym_web/ui/src/screens/messages.ts#L637)).
   It stays a check on the raw service text; only the shown text changes.
9. **The `-32013` text lives in `rpc.ts`.** `call()` replaces the service
   message with `NOT_LOCAL_MESSAGE`
   ([rpc.ts:48](../../../../crates/roym_web/ui/src/rpc.ts#L48)). B1a keeps
   the raw service text in `RpcError.message` and moves the Hub's text to
   the string catalog (`error-not-local`). `rpc.test.ts:53` changes to
   match. An HTTP failure is `RpcError(res.status, "HTTP error <n>")`
   ([rpc.ts:35](../../../../crates/roym_web/ui/src/rpc.ts#L35)); B1a keeps
   the status as the code and the string catalog writes the sentence.
10. **e2e checks that change.** Money and the expiry date:
    [roym-hub.spec.ts:838](../../../../crates/substrate/tests/e2e/tests/roym-hub.spec.ts#L838)
    and `:846` (`Total: 45.00 EUR`), `:1091` (`Total: 1200.00 EUR`),
    `:1095` to `:1098` (ISO expiry date). No e2e check reads
    `NOT_LOCAL_MESSAGE`, an age other than `moments ago` and `ago`
    (`roym-trust.spec.ts:168`, `roym-hub.spec.ts:559`), or a date from
    `dateWords`. The login helper reads `Sign in` and `delegated`
    ([hub-helpers.ts:11](../../../../crates/substrate/tests/e2e/hub-helpers.ts#L11),
    `:15`); both stay in English.
11. **The tests use the machine's time zone.** This machine is in IST.
    Node and Chromium take the zone from the system. Date tests run with
    `TZ=UTC` (D-B1a-9).
12. **The Hub bundle is served file by file.** The runtime unpacks every
    file of `bundle.tar.gz` into the asset table
    ([runtime/roym.rs:535](../../../../crates/substrate/src/runtime/roym.rs#L535),
    `assets::unpack_asset_bundle`). A second JavaScript file (the Hindi
    chunk, D-B1a-1) is served the same way as `assets/index-*.js`, and the
    CSP `script-src 'self'` ([index.html:16](../../../../crates/roym_web/ui/index.html#L16))
    allows it.

## 3. What B1a meets

| Criterion | What B1a does | Test |
| --- | --- | --- |
| ML-2 | A new `src/i18n/<tag>.ftl` file is found by `import.meta.glob`; no code change. A message missing in that file falls back to English, never to an id. | `i18n.test.ts` "a third catalog with a missing message falls back to English"; `catalog.test.ts` (every id used in code is in `en.ftl`) |
| ML-3 | The language list in the session bar; the choice in `localStorage`; the first choice from `navigator.languages`. | `i18n.test.ts` (`pickLanguage` cases); story 40 |
| ML-4 | `format.ts`: money, numbers, dates, times, ages with the format locale and Latin digits. `formatMinor` stays the one place for minor units. | `format.test.ts`, `money.test.ts` (`₹1,00,000.50` in `en` and `hi`) |
| ML-5 (page `lang` only) | `document.documentElement.lang` is the UI language. | `i18n.test.ts`; story 40 |
| ML-6 | `errText()` maps error codes to string catalog messages; the service text is only `$detail`. | `errors.test.ts` |

B1a does not finish ML-1 (B1b), or ML-5 for record text (B3a).

**Where decisions and deviations are written.** Every criterion above is
in the roym-multilingual change doc, so every B1a decision and deviation
goes there and only there, at close-out (Step 8). Following S0, the change
doc has no `## Decisions` section: each decision goes into the design text
it is about ("A. UI localisation"), as "Decided 2026-11-xx: …". Deviations
from the design text or task.md go under `## Deviations`.

## 4. Behaviour-change audit

### Wire changes

None. No JSON-RPC method, error code, WIT, signed record, config or CLI
change. `RpcError.message` now holds the raw service text for `-32013`,
but that is a Hub-internal object, not a wire value.

### Behaviour changes (Hub only)

| # | Before | After |
| --- | --- | --- |
| 1 | Money: `35.50 USD`, `1200 JPY`, `2.500 KWD` | `$35.50`, `JP¥1,200`, `KWD 2.500` (en-IN); `₹1,00,000.50` for INR. An unknown or malformed code from another node falls back to `<amount> <code>`. |
| 2 | Card times: UTC ISO, `2027-01-15T08:00:00.000Z` | Local time with zone name in a `<time datetime>` element (Q3) |
| 3 | Slot times, message times, group key date: browser default locale | The format locale (`listings.ts:487`, `:488`, `groups.ts:126`, `:563`) |
| 4 | Membership and SynOrg dates: `2026-09-29` (UTC) | `29 Sept 2026` (still UTC, D-B1a-5) |
| 5 | Ages: `1 hours ago`, `1 days ago` | `1 hour ago`, `1 day ago` (`Intl.RelativeTimeFormat`); `moments ago` stays |
| 6 | Errors | English text stays the same (Q6 (a)); Hindi users see Hindi |
| 7 | — | A language list in the session bar, on the sign-in page too |
| 8 | Page `lang="en"` always | `lang` is the UI language; `document.title` from the string catalog (same English text) |
| 9 | — | New `localStorage` key `roym-hub-language` |
| 10 | — | Hindi text for the shell, the cards and the word tables |

### Stays the same

- Every English string that B1a moves keeps its exact text, except rows 1
  to 5 above. The e2e steps that find buttons by English text keep
  working.
- The Rust constants and their values. The two Rust tests read a
  different file (section 2.2 point 1).
- The CSP tags. Hindi uses system fonts (change doc "Fonts").
- Every screen under `src/screens/` keeps its own text, except the lines
  that B1a edits for another reason (Step 4 and Step 5; D-B1a-10).

## 5. Decisions this plan makes

These are written into the change doc at close-out (section 3).

- **D-B1a-1. Files and loading.** `src/i18n/en.ftl` and `src/i18n/hi.ftl`
  hold the messages. `src/i18n/i18n.ts` imports `en.ftl` with `?raw`
  (bundled in the main script, always present, the fallback). Every other
  `.ftl` file is found with
  `import.meta.glob(["./*.ftl", "!./en.ftl"], { query: "?raw", import: "default" })`
  and loaded on demand, so each extra language is its own small script
  file. A new file in `src/i18n/` is a new language with no code change
  (ML-2). The file name (without `.ftl`) is the language tag; it must
  parse with `new Intl.Locale`.
- **D-B1a-2. The bundles.** One `FluentBundle` for English and one for the
  active language, each built with
  `new FluentBundle([formatLocale], { useIsolating: false })`. A backlog
  row records that isolation must come back with right-to-left scripts.
- **D-B1a-3. `t()`.** `t(id: string, args?: Record<string, string | number>): string`.
  It formats the active bundle's message; if the active bundle lacks it, the
  English one; if English lacks it too, it returns the id and calls
  `console.error` (a bug; the catalog test makes it impossible for a
  literal id). Formatting errors also go to `console.error`. Codes, ids and
  years are passed as strings, not numbers (section 2.2 point 5).
  `messageId(id: string): string` returns its argument; it marks ids that
  are kept in a table and passed to `t()` later, so the catalog test can
  find them (section 7.1). `t()` is never called when a module loads, only
  when the Hub draws; otherwise the text would stay English after a switch.
- **D-B1a-4. Format locale.** Q1 (a): `new Intl.Locale(uiLanguage, { region: "IN", numberingSystem: "latn" }).toString()`,
  for example `hi-IN-u-nu-latn`. `FORMAT_REGION = "IN"` is one constant in
  `i18n.ts`. Latin digits follow owner answer 4 in task.md.
- **D-B1a-5. The formats** (`src/i18n/format.ts`):
  - `formatNumber(n: number): string` — `Intl.NumberFormat(formatLocale())`.
  - `formatDateTime(secs: number): string` — local zone,
    `{ year: "numeric", month: "short", day: "numeric", hour: "numeric", minute: "2-digit", timeZoneName: "short" }`
    (`dateStyle` cannot be combined with `timeZoneName`; checked).
  - `formatTime(ms: number): string` — `{ hour: "numeric", minute: "2-digit" }`.
  - `formatUtcDate(secs: number): string` — `{ dateStyle: "medium", timeZone: "UTC" }`.
    It replaces `dateWords`, which says a date is "spelled the same for
    everyone"; UTC keeps that meaning.
  - `formatAge(secs: number): string` — replaces `ageWords` with the same
    thresholds (under 90 s: `t("age-moments")`; then minutes under 90,
    hours under 48, else days), through
    `Intl.RelativeTimeFormat(formatLocale(), { numeric: "always" })`.
  - `timeElement(secs: number): HTMLTimeElement` — a `<time>` with
    `datetime` set to `new Date(secs * 1000).toISOString()` and text
    `formatDateTime(secs)`.
  - Each `Intl` object is created once per format locale and kept.
  `directory/words.ts` is deleted; its callers use `formatAge` and
  `formatUtcDate`.
- **D-B1a-6. Money.** `formatMinor(minor, currency)` stays in `money.ts`
  with its signature. It builds the exact decimal string with today's digit
  split (`"-35.50"`), then calls
  `Intl.NumberFormat(formatLocale(), { style: "currency", currency: c, minimumFractionDigits: exp, maximumFractionDigits: exp }).format(decimal)`.
  The string form keeps every digit (TypeScript 5.9 types `format()` with
  `StringNumericLiteral`; checked in `lib.es2023.intl.d.ts:51`). The
  exponent still comes from the Rust-pinned tables. If `Intl` throws (a
  malformed code such as `E1` from another node; checked: `RangeError`),
  it returns `<formatNumber-style decimal> <code>`. `MoneyInputError`'s
  text moves to the string catalog (`money-input-invalid`).
- **D-B1a-7. Language choice.** `pickLanguage(stored, preferred, available)`
  is a pure function. Order: the stored choice if it is in `available`;
  else the first tag in `preferred` (`navigator.languages`) that matches an
  available tag; else `en`. A tag matches when `new Intl.Locale(tag)` does
  not throw, its `language` equals the catalog tag's `language`, and
  `maximize().script` is the same (so `hi-Latn` does not pick the
  Devanagari `hi` catalog; checked: `hi` → `Deva`, `hi-Latn` → `Latn`).
  Every `new Intl.Locale` is in `try`/`catch` (D-S0-1). The stored key is
  `roym-hub-language`; reads and writes are in `try`/`catch`, like the
  `sessionStorage` code in `session/login.ts:39`.
- **D-B1a-8. The language list.** `languagePicker(): HTMLSelectElement` in
  `src/i18n/picker.ts`: class `language-picker`, `aria-label` from
  `t("language-picker-label")`, one `<option value="<tag>">` per available
  language. Each option shows the language's own name,
  `new Intl.DisplayNames([tag], { type: "language" }).of(tag)` (checked:
  `हिन्दी`, `English`), or the tag if that throws. On change: save, then
  `location.reload()` (Q4 (a)). `main.ts` puts it in the session bar in
  both branches (signed in and not).
- **D-B1a-9. Tests run in UTC.** `package.json` `"test": "TZ=UTC vitest run"`.
- **D-B1a-10. What moves in B1a.** Every visible string in `src/cards/`,
  `cards/wording.ts`, `groups/words.ts` (the notices and the two word
  functions; the content types and `shortAddress` stay), `listings/labels.ts`,
  `directory/words.ts`, `money.ts`, `rpc.ts`, `dom.ts`, and `main.ts`
  (Q2 (a)). In a screen file, B1a changes only the lines it must touch:
  the uses of moved constants, the 4 `toLocale*` calls, and the `.message`
  reads of Step 4. A literal on a line that B1a edits moves too (for
  example `Upsert failed: ` in `contacts.ts:91`). Everything else in
  `src/screens/`, `src/session/`, `src/directory/membership.ts`,
  `src/directory/search.ts` (except its two `.message` reads) and
  `src/listings/editor.ts` waits for B1b.
- **D-B1a-11. Rust-pinned English.** The 8 booking sentences and 10 group
  notices keep their exact English text as `en.ftl` messages on one line
  each. The two Rust tests read `../roym_web/ui/src/i18n/en.ftl` and keep
  `content.contains(value)`. None of these sentences holds `{`, `}` or a
  line that starts with `[`, `*` or `.`, so Fluent keeps them as written.
- **D-B1a-12. Errors.** `errText(err)` in `dom.ts`:
  - `RpcError` with `code > 0` (an HTTP status): `t("error-http", { code: String(err.code) })`.
  - `RpcError` with a known JSON-RPC code: the table below, with
    `{ detail: err.message, code: String(err.code) }`.
  - Any other `RpcError`: `t("error-service", { detail, code })`.
  - Another `Error`: `err.message` (Hub-made text; B1b localises it at the
    source). Anything else: `String(err)`.

  | Code | Message id | English (`en.ftl`) |
  | --- | --- | --- |
  | `-32001` | `error-service-unavailable` | `{ $detail }` |
  | `-32002` | `error-service-timeout` | `{ $detail }` |
  | `-32003` | `error-permission` | `{ $detail }` |
  | `-32004` | `error-quota` | `{ $detail }` |
  | `-32010` | `error-not-signed-in` | `{ $detail }` |
  | `-32011` | `error-not-owner` | `{ $detail }` |
  | `-32012` | `error-no-owner` | `{ $detail }` |
  | `-32013` | `error-not-local` | `this installation refused a request that did not come from you` |
  | `-32601` | `error-method-not-found` | `{ $detail }` |
  | `-32602` | `error-invalid-input` | `{ $detail }` |
  | `-32603` | `error-internal` | `{ $detail }` |
  | other | `error-service` | `{ $detail }` |
  | HTTP | `error-http` | `HTTP error { $code }` |

  The codes are every code that the Roym services and the `web` proxy
  return today: [roym_web/src/app.rs:288](../../../../crates/roym_web/src/app.rs#L288)
  to `:324`, [roym_conversation/src/app.rs:135](../../../../crates/roym_conversation/src/app.rs#L135)
  and `:137`, and the envelope helpers
  ([envelope.rs:59](../../../../crates/roym_core/src/envelope.rs#L59) to `:67`).
  English output is unchanged for every row (Q6 (a)). Hindi messages
  follow Q6 (a).
- **D-B1a-13. Message ids.** Lower case, words joined by `-`, starting
  with the area: `hub-`, `tab-`, `session-`, `login-`, `language-`,
  `gallery-`, `card-`, `quote-`, `receipt-`, `request-`, `progress-`,
  `payment-`, `fulfilment-`, `group-`, `place-`, `audience-`, `age-`,
  `money-`, `error-`, and for a screen line moved under D-B1a-10, the
  screen's name (`contacts-`, `safety-`, `backup-`, `profile-`,
  `directory-`, `messages-`). The quote and the receipt share terms rows
  (`Scope:`, `Total:`, …): one id each, `terms-scope`, `terms-total`, and
  so on.
- **D-B1a-14. The Hindi text.** An agent writes it (owner answer 3). The
  owner reviews `hi.ftl` before B1b merges, so B1a merges a draft. A
  comment at the top of `hi.ftl` says `# owner review: pending`; B1b's
  close-out changes it.

## 6. Step-by-step changes

Each step builds (`npm run build` in `crates/roym_web/ui`), and its tests
pass (`mise run test:roym-ui`; Steps 3, 5 and 7 also the e2e checks they
change). All paths below are under `crates/roym_web/ui/` unless they start
with `crates/` or `docs/`.

### Step 1 — Commit this plan

Done in the planning session: this file and the B1a row of
[status.md](status.md).

### Step 2 — The i18n core

1. `npm install @fluent/bundle@0.19` → `package.json` gets
   `"dependencies": { "@fluent/bundle": "^0.19.1" }`. Check that
   `package-lock.json` adds only `@fluent/bundle` (`npm ls vite rollup`
   unchanged, as in S0 Step 4).
2. `src/vite-env.d.ts`: `/// <reference types="vite/client" />` (types for
   `?raw` and `import.meta.glob`).
3. `src/i18n/i18n.ts` (about 150 lines):

   ```ts
   import { FluentBundle, FluentResource } from "@fluent/bundle";
   import EN_SOURCE from "./en.ftl?raw";

   export const FALLBACK_LANGUAGE = "en";
   export const FORMAT_REGION = "IN";
   const STORAGE_KEY = "roym-hub-language";
   const LOADERS = import.meta.glob<string>(["./*.ftl", "!./en.ftl"], {
     query: "?raw",
     import: "default",
   });

   export function availableLanguages(): string[]  // "en" first, then sorted tags from LOADERS
   export function pickLanguage(stored: string | null, preferred: readonly string[], available: readonly string[]): string
   export async function activate(tag: string): Promise<void>  // load, build bundle, set state, set <html lang>
   export async function initLanguage(): Promise<void>         // pickLanguage(readStored(), navigator.languages, available) then activate
   export function currentLanguage(): string
   export function formatLocale(): string
   export function t(id: string, args?: Record<string, string | number>): string
   export function hasMessage(id: string): boolean              // in the English bundle
   export function messageId(id: string): string
   export function saveLanguage(tag: string): void
   ```

   Keep the module state in one object (`{ language, locale, active, english }`).
   `activate` builds a new English bundle too, because the bundle's locale
   (D-B1a-2) follows the format locale. Export a test-only
   `createCatalogs(sources: Record<string, string>)` or take the loaders
   as a parameter of an internal function, so `i18n.test.ts` can add a
   third language without a file (ML-2 test).
4. `src/i18n/en.ftl` and `src/i18n/hi.ftl`: the `language-picker-label`
   message, `age-moments`, `money-input-invalid` and the error messages
   (D-B1a-12). Header comment of each file: what it is, how to add a
   message, and (in `hi.ftl`) `# owner review: pending`.
5. `src/i18n/format.ts` (D-B1a-5), about 90 lines.
6. Tests (section 7.1): `src/i18n/i18n.test.ts`, `src/i18n/format.test.ts`,
   `src/i18n/catalog.test.ts`.
7. `package.json`: `"test": "TZ=UTC vitest run"`.
8. If Q5 (a): add the `hub-ui` gate to `GATES` in
   [verify.rs](../../../../xtask/src/verify.rs) after `planning-refs`:
   `name: "hub-ui", skip_on_docs_only: true, docs_paths_that_run: &[], commands: &[&["mise", "run", "test:roym-ui"]]`.
   If `xtask/src/verify/tests.rs` (or the tests in `verify.rs`) list the
   gate names or count, update them.

Nothing calls the new modules yet, so the Hub does not change.

### Step 3 — Formats

1. `money.ts`: `formatMinor` as in D-B1a-6. Move the decimal-string code
   into a local helper `minorToDecimal(minor, exp): string`. `toMinorUnits`
   throws `new MoneyInputError(t("money-input-invalid", { input, example }))`.
   Keep `EXPONENT_0` and `EXPONENT_3` as the first uses of those names in
   the file (section 2.2 point 1).
2. The 4 `toLocale*` calls: `listings.ts:487`, `:488` → `formatDateTime`;
   `groups.ts:126` → `formatTime`; `groups.ts:563` → `formatDateTime(ms / 1000)`
   (or a `formatDateTimeMs`; pick one and use it once).
3. `ageWords` and `dateWords` callers → `formatAge` and `formatUtcDate`;
   delete `src/directory/words.ts`.
4. `money.test.ts`: the `formatMinor` cases become format-locale cases
   (`activate("en")` and `activate("hi")`), plus the ML-4 case
   `formatMinor(10000050, "INR")` is `₹1,00,000.50` in both, a malformed
   code, and a large value that keeps every digit:
   `formatMinor(Number.MAX_SAFE_INTEGER, "INR")` ends in `.91`
   (`9007199254740991` minor units; a float division would lose the last
   digit).
5. e2e: `roym-hub.spec.ts:838`, `:846` → `Total: €45.00`; `:1091` →
   `Total: €1,200.00`. These are en-IN outputs; Node and Chromium give the
   same text here (checked in Node 22; re-check in the Playwright run).

### Step 4 — Errors

1. `rpc.ts`: remove `NOT_LOCAL_MESSAGE`; `RpcError.message` is the raw
   service text, or `""` for an HTTP failure (the code holds the status).
   Update `rpc.test.ts:53` to check `errText(caught)` instead.
2. `dom.ts`: `errText` as in D-B1a-12, with the code table as a
   `Record<number, string>` of `messageId(...)` values.
3. Delete the copies in `screens/messages.ts:96` and
   `screens/listings.ts:29`; import `errText` from `dom.ts`.
4. The 22 `.message` reads:

   | Place | What B1a does |
   | --- | --- |
   | `dom.ts:13`, `:14` | The new `errText` |
   | `rpc.ts:42` | Stays: it reads the wire into `RpcError` |
   | `main.ts:233`, `:256` | `errText(e)`; the `Sign-in failed:` prefix → `login-failed` with `{ detail }` (Step 6 moves the rest of `main.ts`) |
   | `listings/editor.ts:169` | Stays: it re-wraps a `MoneyInputError`, whose text is now from the string catalog |
   | `directory/search.ts:98`, `:100` | `errText(err)`; the sentence `The search failed: … Press Search to try again.` → `directory-search-failed` with `{ detail }`; the `503` sentence at `:92` → `directory-search-not-started` |
   | `directory/search.ts:110` | Stays: it is a directory's per-source error text from the wire; B1b words the outcome line |
   | `screens/directory.ts:272` | `errText(err)` (the error is already a Hub sentence) |
   | `screens/messages.ts:97`, `:98` | Deleted with the copy |
   | `screens/messages.ts:637` | The `includes` check stays on `err.message`; the shown texts → `messages-quote-expired` and `messages-accept-failed` with `{ detail: errText(err) }` |
   | `screens/listings.ts:30`, `:31` | Deleted with the copy |
   | `screens/contacts.ts:91` | `contacts-upsert-failed` with `{ detail: errText(err) }` |
   | `screens/safety.ts:54`, `:172`, `:222` | `safety-block-failed`, `safety-report-failed`, `safety-save-failed`, each with `{ detail }` |
   | `screens/backup.ts:93`, `:116` | `backup-export-failed`, `backup-import-failed` |
   | `screens/profile.ts:47` | `profile-error` |

   The English text of every row stays the same.
5. `src/i18n/errors.test.ts` (section 7.1).

### Step 5 — Word tables and cards

1. `en.ftl` / `hi.ftl`: every string of `cards/wording.ts`, the notices and
   word functions of `groups/words.ts`, `listings/labels.ts`, and every
   visible string in `cards/refused.ts`, `cards/unknown.ts` and
   `cards/templates/*.ts`. About 110 messages. Use Fluent variables for
   the values (`terms-total = Total: { $amount }`, with `amount` already
   formatted by `formatMinor`, so a string).
2. `cards/wording.ts`: delete; uses → `t("payment-notice")` and so on.
   `groups/words.ts`: delete the notice constants; `deliveryWords` and
   `membershipEventWords` call `t()`; content types and `shortAddress`
   stay. `listings/labels.ts`: the three label records become id tables
   (`Record<ServiceLocationKind, string>` of `messageId(...)`) and two
   functions, `serviceLocationEditorLabel(kind)` and
   `openToLabel(kind)`; `serviceLocationCardLabel` keeps its "unknown
   value shown as it came" rule. Update the callers in
   `screens/groups.ts`, `screens/listings.ts`, `screens/messages.ts`,
   `listings/editor.ts` and the cards (the D-B1a-10 rule).
3. `booking_progress.ts`: `STATE_WORDS` and `CONFLICT_SENTENCES` become
   id tables; an unknown value is shown as it came (as today).
4. Card times (Q3 (a)): the 9 calls of section 2.2 point 6 use
   `timeElement(secs)`; a row like `Schedule: <start> - <end>` becomes a
   message with two placeables, built as text + `<time>` + text + `<time>`.
   To keep this simple, `t()` builds the label (`terms-schedule-label =
   Schedule:`) and the template appends the two `<time>` elements with a
   `t("terms-range-separator")` text node between them. Keep the card rule
   comment and add one line: "A `<time>` element's `datetime` is computed
   by the Hub from a number."
5. Rust: the two tests of section 2.2 point 1 read
   `../roym_web/ui/src/i18n/en.ftl`; fix the doc comment at
   `group.rs:17`, and any comment in `booking.rs` or `booking/tests.rs`
   that names `wording.ts`. Run
   `cargo nextest run -p syneroym-roym-core the_ui_wording notice_constants`.
   No `roym_*` code changes, so no WASM rebuild is needed for parity.
6. Hub tests: `cards/wording.test.ts`, `groups/words.test.ts`,
   `listings/labels.test.ts`, `cards/templates/templates.test.ts`,
   `screens/groups.test.ts` import `t()` or the new functions instead of
   the deleted constants. Date checks read `time[datetime]`.
7. e2e: `roym-hub.spec.ts:1095` to `:1098`: check
   `.quote-expiry time` has `datetime` equal to the ISO string, and the
   row contains `(expired)`; check `.quote-expired-notice time` the same
   way.

### Step 6 — The shell, the language list and the page `lang`

1. `main.ts`: `await initLanguage()` as the first line of `main()`, before
   anything is drawn. `document.title = t("hub-title")`. Every visible
   string → `t()`: title, `Logged in: …`, `Log out`, `Not logged in`,
   the 11 tab names (`tab-profile` … `tab-components`), the sign-in box
   (`Sign in`, the four status lines, the intro, the two buttons), the
   tab error line, and `renderHome`'s headings and ping lines. The sample
   card data stays as it is.
2. `src/i18n/picker.ts` (D-B1a-8), and append `languagePicker()` to the
   session bar in both branches.
3. `style.css`: a small rule for `.language-picker` (margin only).
4. `index.html` keeps `lang="en"` (the right value before the script
   runs).
5. A test `src/i18n/picker.test.ts`: options, selected value, and that a
   change saves the tag and calls `location.reload` (stub it).

### Step 7 — Demo story 40 and the e2e helper

1. `crates/substrate/tests/e2e/hub-helpers.ts`:
   - `chooseHubLanguage(page, tag)`: select `tag` in `.language-picker`,
     wait for the load, check `html[lang]` is `tag`.
   - `hubMessage(tag, id)`: read `crates/roym_web/ui/src/i18n/<tag>.ftl`
     and return the text of message `id` (a line `id = text`). The story
     uses it so an owner edit of the Hindi text does not break the story.
2. `crates/substrate/tests/e2e/demo/40-roym-languages.spec.ts`, one test,
   `test.describe.configure({ mode: 'serial' })` like story 20:
   - read `ALICE_HUB_URL` and `ALICE_KEY` with `requireEnv(…, '20-roym-setup.sh')`;
   - step "Alice's screen: sign in": `page.goto`, `loginWithDelegatedKey`;
   - step "Alice's screen: switch the Hub to Hindi":
     `chooseHubLanguage(page, 'hi')`; the Profile tab button (`.tab-button`
     first) has text `hubMessage('hi', 'tab-profile')`;
   - step "Alice's screen: reload; the Hub is still in Hindi":
     `page.reload()`; `html[lang]` is `hi`; the session bar still contains
     `did:key:` (still signed in); the tab text is still Hindi.
   No shell script: story 20 already deployed Roym. A new browser context
   starts in English, so stories 20 and 30 are not affected, and a second
   run in attach mode works.
3. Run `mise run demo` (or `npm run test:demo`) and keep the output for the
   PR text.

### Step 8 — Docs, size report and close-out

1. Size: in `crates/roym_web/ui`, before (on `main`) and after, `npm run
   build`, then for each `dist/assets/*.js`: raw bytes and `gzip -9 -c |
   wc -c`. Report the main script and the Hindi chunk separately and
   together, against S0's numbers (114,463 raw / 31,769 gzip on `main`
   then; +4,205 gzip for the library alone). Write the result into task.md
   (B1a result line) and the change doc ("Which catalog format", the size
   line).
2. Change doc: decisions D-B1a-1 to D-B1a-14 into "A. UI localisation"
   (section 3 rule); `## Deviations`: (a) the shell moves in B1a (Q2), (b)
   card times also change (section 2.2 point 6), (c) the two Rust tests
   read `en.ftl`, (d) Fluent isolation is off; the Test column for ML-2 to
   ML-6 with real test names from the run; `demo:` becomes
   `- crates/substrate/tests/e2e/demo/40-roym-languages.spec.ts`;
   `living-docs-touched` lists the files of section 9. `status` stays
   `draft` (B6 sets `done`).
3. Living docs (section 9), deferred backlog (section 8),
   [task.md](task.md) B1a result line, [status.md](status.md).
4. The import clean-up pass over the two Rust test files.
5. `mise run verify` with no `--skip`, as a background job.

## 7. Tests

### 7.1 Hub unit tests (vitest, `TZ=UTC`)

`src/i18n/i18n.test.ts`

- `pickLanguage`: stored `hi` wins; stored `xx` (not available) is
  ignored; `["en-US"]` → `en`; `["hi-IN", "en"]` → `hi`; `["hi-Latn"]` →
  `en` (script differs); `["x-private", "hi"]` → `hi` (a throwing tag is
  skipped); `[]` → `en`.
- `activate("hi")` sets `document.documentElement.lang` to `hi` and
  `formatLocale()` to `hi-IN-u-nu-latn` (ML-5, D-B1a-4).
- ML-2: with a third catalog `xx` that has only `tab-profile`,
  `t("tab-logout")` returns the English text, never the id.
- `t()` with a missing id returns the id and calls `console.error` (spy).
- `saveLanguage` then `initLanguage` picks the saved tag; `localStorage`
  that throws does not break `initLanguage`.

`src/i18n/catalog.test.ts` (reads the `.ftl` files with `?raw` and the
source files with `import.meta.glob("../**/*.ts", { query: "?raw", import: "default", eager: true })`,
skipping `*.test.ts`)

- Every entry line (`^-?[A-Za-z][A-Za-z0-9_-]*\s*=`) of each `.ftl` file is
  a parsed entry (`resource.body.length` equals the count): no silent junk
  (section 2.2 point 4).
- Every id in a `t("…")` or `messageId("…")` call in the source is in
  `en.ftl`.
- `hi.ftl` has exactly the ids of `en.ftl` (the owner asked for Hindi for
  every string; a later partial language is allowed, this test names only
  `hi`).
- For each id, the variables (`{ $name }`) of `hi.ftl` are a subset of
  those in `en.ftl`.
- Each message formats with no error in both languages, with sample args
  built from its variable names.
- No id is unused (each `en.ftl` id appears in the source). This keeps
  dead text out; drop the test if a later slice needs ids used only from
  data.

`src/i18n/format.test.ts`

- ML-4: `formatNumber(100000.5)` is `1,00,000.5` in `en` and `hi`;
  digits are Latin in `hi`.
- `formatDateTime(1800000000)` in `en`: compare with a value computed in
  the test from the same `Intl` options (never a hard-coded string that
  depends on the ICU version), and check that it contains `2027` and
  `UTC` (the zone name under `TZ=UTC`).
- `formatUtcDate(1790000000)` is `21 Sept 2026` in `en` (checked in Node 22).
- `formatAge`: 30 s → `moments ago`; 300 s → `5 minutes ago`; 3600 s →
  `1 hour ago`; 3 days → `3 days ago`; the same in `hi` is not English.
- `timeElement(secs)` has `datetime` = the ISO string.

`src/i18n/errors.test.ts`

- For each code of D-B1a-12, `errText(new RpcError(code, "raw text", "Other"))`
  in `en` equals today's English output (`raw text`, or the `-32013`
  sentence); in `hi` it starts with Devanagari and contains `raw text`
  only after `विवरण:` (ML-6).
- An HTTP failure `RpcError(503, "", "Other")` → `HTTP error 503`.
  Codes are not grouped: in a message that shows `{ $code }`, `-32602`
  never prints as `-32,602`.
- A plain `Error("x")` → `x`; a string → the string.

`src/i18n/picker.test.ts` — Step 6.

Existing tests changed in Steps 3 to 5: `money.test.ts`, `rpc.test.ts`,
`cards/wording.test.ts`, `groups/words.test.ts`, `listings/labels.test.ts`,
`cards/templates/templates.test.ts`, `screens/groups.test.ts`,
`dom.test.ts`. Each keeps its English expectations; only imports and
date or money expectations change.

### 7.2 Rust

`the_ui_wording_matches_this_crate` and
`notice_constants_match_hub_ui_verbatim` read `en.ftl` (Step 5). Before
the change, run them once against an `en.ftl` with one sentence changed,
to see that they fail.

### 7.3 Playwright

- `roym-hub.spec.ts`: the four money and date checks of section 2.2
  point 10 (Steps 3 and 5).
- Story 40 (Step 7): ML-3 and ML-5 end to end.
- Every other spec runs unchanged in English; that checks that the moved
  English text did not change.

### 7.4 Not tested

- Glyph shaping of Devanagari with system fonts on each OS. The change doc
  leaves fonts to a later test device check.
- Every browser's `Intl` output; the tests compare with values computed by
  the same engine.

### 7.5 Size risks

| File | Today | Risk |
| --- | --- | --- |
| `src/main.ts` | 396 | About +5 (strings become calls; the picker is its own file) |
| `src/screens/messages.ts` | 943 | About −5. Already over 800; TypeScript files have no length gate, but do not let it grow. B1b may split it. |
| `src/screens/groups.ts` | 685 | About ±0 |
| `src/cards/templates/agreement_receipt.ts` | 234 | About +5 (the `<time>` rows) |
| `src/i18n/i18n.ts` | new | About 150 |
| `src/i18n/format.ts` | new | About 90 |
| `src/i18n/en.ftl`, `hi.ftl` | new | About 200 lines each |
| `crates/roym_core/src/booking/tests.rs` | 342 | ±0 (a path string) |
| `crates/roym_core/src/conversation/group/tests.rs` | 217 | ±0 |

No Rust file grows. The duplication and suppression ratchets are Rust-only
and do not move.

### 7.6 Bundle size

The library is +4,205 bytes after `gzip -9` (S0). B1a adds the English
string catalog to the main script and the Hindi one as its own file.
Hindi text is about 3 bytes per character before `gzip`. Expected: under
+10 KB `gzip -9` in the main script after B1a. Step 8 reports the real
numbers. If the main script grows by more than 25 KB `gzip -9` in total
(library and English catalog), stop and tell the owner.

### 7.7 Hindi text

`hi.ftl` is a draft (D-B1a-14). The tests check form, not meaning. Use
common words that a Hindi speaker reads on a phone (`संदेश`, `संपर्क`,
`प्रोफ़ाइल`), keep product names (Roym, SynOrg, DID) in Latin letters, and
keep each Hindi message about as short as the English one, so buttons do
not grow much.

### 7.8 How B1a avoids the kinds of findings S0 had

- **A library claim not checked in its source** (S0 P1): the three Fluent
  behaviours this plan depends on (isolation, silent junk, number args)
  were read in the `@fluent/bundle` 0.19.1 source, with file and line.
  Every `Intl` output in this plan was run in Node 22.
- **Two parsers that disagree** (S0 P2): every `new Intl.Locale` is in
  `try`/`catch`, and the catalog tag match compares language and script
  (D-B1a-7).
- **A check that cannot see the error** (S0 P5): the junk test counts
  entries, because Fluent itself reports nothing.
- **Old base or step state** (S0 P9): Step 1 is done; the base is named.
- **Wrong counts** (S0 R4): the 22 `.message` reads are listed one by one
  (Step 4); the date calls are 4 `toLocale*` plus 9 `toISOString`.
- **Scratch files left behind** (S0 R5): the Fluent source check for this
  plan ran in `$TMPDIR/fluent-check`; the implementer needs no scratch
  folder.

## 8. Deferred backlog

Add (in [deferred-backlog.md](../../deferred-backlog.md)):

- §10 "Hub formats use the India region for every language" (if Q1 (a)):
  a person outside India sees Indian digit grouping. A later Hub setting
  can choose the region. Target `TBD`. Link `crates/roym_web/ui/src/i18n/i18n.ts`
  (`FORMAT_REGION`).
- §10 "Fluent bidi isolation is off in the Hub": needed again when the Hub
  gets a right-to-left language (Urdu). Target `TBD`. Link
  `crates/roym_web/ui/src/i18n/i18n.ts` (`useIsolating`).
The owner review of `hi.ftl` is planned work (owner answer 3, before B1b
merges), not a deferral, so it gets no backlog row; the `# owner review:
pending` line in `hi.ftl` tracks it (D-B1a-14).

The row "Accessibility (WCAG 2.1 AA) & localisation/i18n" (§10, line 508)
stays: B6 moves its localisation part. No new `TODO` or `FIXME` marker.

## 9. Living docs to update at close-out

- **`docs/system-architecture.md:1412`** ("The Hub holds little state…"):
  add that it keeps the UI language in `localStorage`
  (`roym-hub-language`), that its text comes from Fluent string catalogs
  (`en`, `hi`) with English fallback, and that it formats money, numbers
  and dates with `Intl` for the UI language with region India and Latin
  digits.
- **`docs/system-requirements-spec.md:1461`** (`[APP-A11Y]`, Localisation):
  the Hub has a string catalog mechanism, English and Hindi, a language
  setting and local formats. The sentence "Today the Roym Hub UI contains
  hardcoded English text" becomes "Most Hub screens still have English
  text in the code." (B1b removes it.)
- **`docs/planning/traceability-matrix.md:68`** (`[APP-A11Y]`): evidence
  links to `src/i18n/` and the new tests. Status stays as it is until B6.
- **`docs/developer-guide.md`**, under "The Roym Hub" (`:1620`), a new
  `##### Hub text and languages` before "Add a Roym method" (`:1745`):
  where the string catalogs are; add a message to `en.ftl` and `hi.ftl`
  and call `t("id")` (never at module load); `messageId()` for id tables;
  pass codes as strings; error messages and `$detail`; add a language by
  adding `src/i18n/<tag>.ftl`; `mise run test:roym-ui` runs the catalog
  checks.
- **`docs/demo/demo.md`**: one short step after R3 (chat), "The Hub in
  Hindi": pick हिन्दी in the session bar, reload. It names story 40 as the
  reference for the clicks, like the R3 text names story 20.
- **`docs/TERMINOLOGY.md`**: add "String catalog (Hub)", with the note that
  it is not the `catalog` service.
- **The change doc**, **task.md**, **status.md**: Step 8.

## 10. Exit criteria

B1a is done when all of these are true:

1. The owner has answered Q1 to Q6, and sections 4 to 9 match the answers.
2. `t()`, the two string catalogs and the language choice work as in
   D-B1a-1 to D-B1a-8; `i18n.test.ts` and `catalog.test.ts` pass (ML-2,
   ML-3).
3. `formatMinor(10000050, "INR")` is `₹1,00,000.50` in English and Hindi;
   the 4 `toLocale*` calls, the 9 card `toISOString` calls and
   `ageWords`/`dateWords` use `format.ts` (ML-4).
4. `document.documentElement.lang` is the UI language (ML-5, page only).
5. Every `RpcError` shown by the Hub goes through `errText`, and the
   Hindi error messages pass the `विवरण:` test (ML-6). The 22 `.message`
   reads are handled as in the Step 4 table.
6. No string literal is visible text in `src/cards/`, the four word
   tables or `main.ts` (review; the build check is B1b).
7. The two Rust tests read `en.ftl` and pass; they fail when one sentence
   differs.
8. Story 40 passes in `mise run demo`; `roym-hub.spec.ts` passes with the
   new money and date checks; every other e2e spec passes unchanged.
9. The bundle sizes are reported (Step 8) and within section 7.6.
10. The change doc, the living docs, the backlog, task.md and status.md
    are updated as in sections 8 and 9.
11. Changed paths: `crates/roym_web/ui/` (not `dist/`), two Rust test
    files and one Rust doc comment in `crates/roym_core`,
    `crates/substrate/tests/e2e/` (one spec, one helper file, one story),
    `xtask/src/verify.rs` (if Q5 (a)), and docs.
12. `mise run verify` passes with no `--skip`.
13. One PR.

## 11. What B1a does not do

- No build check for literals outside the string catalog (ML-1, B1b).
- No change to the text of `src/screens/`, `src/session/`,
  `src/directory/membership.ts`, `src/directory/search.ts` or
  `src/listings/editor.ts`, except the lines of D-B1a-10.
- No `lang` on record text, no profile `languages` (B3a).
- No taxonomy or code labels (B2).
- No native digits; Latin digits only (owner answer 4).
- No bundled web font, no CSP change.
- No right-to-left layout.
- No localised text from the substrate or `roymctl`.
- No change to the signed records, the Rust constants or any wire value.
- No owner review of the Hindi text (before B1b merges).
