# M08B slice B1b — Hub localisation, every screen string and the build check: implementation plan

*Reader: the developer (or AI) who implements slice B1b. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan written, 2026-11-06. Plan review rounds 1 and 2
([slice-b1b-review.md](slice-b1b-review.md), P1 to P12) are folded in. The
owner answered Q1 to Q7 on 2026-11-06, all (a); the answer is under each
question.**

**Size and PR.** M. One PR on `feat/m08b-b1b-localization-screens`, with
the commits of section 6 in that order. The branch starts from `main` at
`1a62c26a` (B1a is merged, [PR #349](https://github.com/syneroym/syneroym/pull/349))
and already holds the plan commits. `main` at `8b39b525` (an auth fix)
is merged in; it changes no line this plan names (it edits
`docs/system-architecture.md` near line 711, one line for one line, so
`:1414` does not move). B1b moves every visible English string
that is left in the Hub into the string catalogs `en.ftl` and `hi.ftl`. It
adds the check that fails the Hub build when a string literal is set as
visible text outside the catalog (ML-1). English text stays the same, byte
for byte. It changes no JSON-RPC method, no WIT file and no record schema.
Its Rust changes are two tests and three doc comments in `roym_core`.

**Base.** Every file path and line number below was checked on `main` at
`1a62c26a`. The task.md facts were checked on `76b52fe6`; B1a changed the
Hub after that, so section 2.2 gives the numbers on `1a62c26a`.

**Parallel work.** B4a (substrate only) may run beside B1b. It touches no
file that B1b touches ([task.md](task.md) section 4). B2 must wait for
B1b: it adds new Hub strings that must go through the check.

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`). Its `hub-ui` gate runs the Hub unit tests (B1a).
  - No planning ids in code or comments: no "M08B", "B1b", "ML-1" or
    "D-B1b-3" in `.ts`, `.ftl`, `.json`, `.rs` files or test names.
  - Comments never cite size rules.
  - The import clean-up pass (the Rust changes are two tests and three
    doc comments), the deferred backlog, the living-docs close-out and the
    demo close-out.
  - The communication style: the Hindi text is user text. It must be plain,
    common Hindi, not formal or Sanskrit-heavy words.
- [task.md](task.md): section 1 (Y1), section 2 ("Hub"), the "Demo"
  paragraph, "B1a" (and its result line) and "B1b", section 6 (owner
  answers 2, 3 and 4).
- [roym-multilingual change doc](../../changes/roym-multilingual/change.md):
  "A. UI localisation" (with "Decided 2026-11-06, how the Hub does it"),
  "Rules for every layer", criteria ML-1 and ML-6, "Deviations". Every
  B1b decision and deviation is written there, and only there (section 3).
- task.md names one more change doc,
  [demo-stories](../../changes/demo-stories/change.md), in its "Demo"
  paragraph. B1b extends story 40 under its rules ("Link between slice
  plans, the demo and AGENTS.md": the plan says what the viewer sees; the
  story file holds the steps). B1b meets no criterion of that doc and
  records nothing there.
- [slice-b1a-implementation-plan.md](slice-b1a-implementation-plan.md)
  (the mechanism: D-B1a-1 to D-B1a-15) and
  [slice-b1a-review.md](slice-b1a-review.md). R6 of the B1a code review
  says B1b changes the Search button and two Hindi search messages
  together (section 2.2 point 10). Section 7.8 lists how B1b avoids the
  kinds of findings that S0 and B1a had.
- The code you change most: `crates/roym_web/ui/src/i18n/` (all of it),
  `src/dom.ts`, and the screen files of section 2.2 point 1.

### Terms used in this plan

- **Hub:** the Roym web UI in the browser (`crates/roym_web/ui`).
- **String catalog:** the file of all Hub text for one language,
  `src/i18n/en.ftl` and `src/i18n/hi.ftl`. Say "string catalog" in docs and
  comments, because "catalog" alone is also a Roym service.
- **Message id, `t()`, `messageId()`, `lookupText()`, `labelled()`:** as in
  B1a (D-B1a-3). `t("id", args)` returns the text for the UI language.
- **Visible sink:** a place in the code where a value becomes text the
  person sees: `el.textContent = …`, `text("p", …)`, a `placeholder`, an
  `aria-label`, and so on. Section 5, D-B1b-2, has the full list.
- **Hub-made error:** an `Error` that the Hub itself throws, with text that
  the Hub writes (for example "a title is required"). Not an `RpcError`,
  which comes from a service, and not an error that the browser throws.
- **Hindi sweep:** a new unit test that draws each screen in Hindi and
  fails if English words are left (D-B1b-12).

## 1. What B1b builds, in one paragraph

B1a built the mechanism (`t()`, the two string catalogs, the language
list, formats and error messages) and moved the cards, the word tables and
the shell. B1b moves the rest: every screen under `src/screens/`, the
listing editor, the directory and membership words, the sign-in errors and
the Hub-made error messages. About 400 new messages go into `en.ftl`, each
with a Hindi version in `hi.ftl`. English stays the same, so every e2e step
that finds a button by its English name still works. B1b then adds a check,
`findVisibleText()` in `src/i18n/scan.ts`, that parses each source file and
reports a string literal that reaches a visible sink, or a literal that
reads like a sentence anywhere. `npm run build` runs the check and fails
when it reports anything, so from this slice on every new Hub string must
go through `t()`. A second new test draws each screen in Hindi and fails
when an English word is left. The owner reviews all of `hi.ftl` before the
PR merges.

## How this is shown to an end user

- Alice's Hub is set to Hindi (she picked "हिन्दी" in the session bar
  after slice B1a).
- She opens each tab: Profile, Contacts, Messages, Groups, Listings,
  Directory, Memberships, SynOrg, Safety and Backup. Every heading,
  button, field label, hint and help text is now in Hindi. Before B1b only
  the tab names and the cards were Hindi.
- When she makes a mistake in a form, for example she saves a listing with
  no title, the Hub tells her in Hindi.
- Her own data (names, listing titles, message text) stays as she wrote
  it. Codes from the system, such as a message state, show as Hindi words.
- A person who uses the Hub in English sees no change at all.

How a developer can see it work:

- `npm run build` in `crates/roym_web/ui` runs the visible-text check
  before `vite build`. Add `el.textContent = "Hello"` to any screen and
  run it again: the build fails and names the file and line.
- `mise run test:roym-ui` (and the `hub-ui` gate of `mise run verify`)
  runs `visible-text.test.ts` and the Hindi sweep,
  `hindi-screens.test.ts`, which draws each screen in Hindi.
- `mise run demo` runs story `demo/40-roym-languages.spec.ts`, which now
  also opens one more screen in Hindi.

**Demo:** story 40 gains one step: after the reload, Alice opens the
Contacts screen and its button is in Hindi (Q7).

## 2. Points for the owner

### 2.1 Questions

Each question names the step it blocks. The recommendation is the first
option.

**Q1. Where does the build check run? (blocks Step 2 and Step 11)**

ML-1 says "A check fails the UI build". `npm run build` is
`eslint src/ && tsc && vite build`
([package.json:8](../../../../crates/roym_web/ui/package.json#L8)).
`mise run build:roym-ui` (used by `build:roym`, so by `nextest` and the
e2e run) and CI call it. B1a already parses every source file with the
TypeScript compiler in `src/i18n/scan.ts` (`findMessageIds`), for the
catalog test.

- (a) **A vitest file that `npm run build` runs.** The check is a function
  `findVisibleText()` in `scan.ts`, next to `findMessageIds()`. A test file
  `src/i18n/visible-text.test.ts` runs it over every source file. The build
  becomes `eslint src/ && tsc && vitest run src/i18n/visible-text.test.ts && vite build`.
  The `hub-ui` gate runs it too, with all the other tests. One
  implementation, one parser, tested like the rest of the Hub. It adds
  about 2 seconds to a build. *Recommended.*
- (b) **A Vite plugin.** `vite.config.ts` gets a plugin with
  `apply: "build"` that calls `findVisibleText()` and `this.error(…)`. Same
  check, but the plugin code itself is not covered by a unit test, and
  `vite.config.ts` is outside `tsc` and `eslint` (both look at `src/` only).
- (c) **An ESLint rule.** A local rule loaded with `--rulesdir`. It shows
  the error in an editor, but it is a second implementation on a different
  syntax tree (ESTree, not the TypeScript tree that `scan.ts` uses), and
  `--rulesdir` is deprecated in ESLint 8.

*Owner answer:* (a), a vitest file that `npm run build` runs.

**Q2. Which text does a Hindi user sign as the data-use notice? (blocks
Step 7)**

A request shows the notice and then signs it into the record: the Hub
draws `DEFAULT_DATA_USE_NOTICE` above the form
([messages.ts:343](../../../../crates/roym_web/ui/src/screens/messages.ts#L343))
and sends the same text as `data_use_notice` in `request.set`
([messages.ts:370](../../../../crates/roym_web/ui/src/screens/messages.ts#L370)).
The service checks only that the field is not empty and not over
`MAX_NOTICE_LEN` (2,048 bytes)
([transaction.rs:505](../../../../crates/roym_core/src/transaction.rs#L505)).
Nothing compares it with the English constant.

- (a) **Sign the text that is shown, in the UI language.** A Hindi user
  reads a Hindi notice and signs that Hindi notice. The English catalog
  text stays equal to the Rust constant, and `roymctl` keeps the English
  constant. This follows the change doc's rule "the signed original always
  wins": a person signs what they read. The record has no `lang` tag until
  B3a adds one. *Recommended.*
- (b) **Show Hindi, sign English.** The record stays the same in every
  language, but the person signs words they did not read.
- (c) **Keep the notice in English in every language until B3a.** The one
  English sentence left on a Hindi screen, with a named exception in the
  check.

*Owner answer:* (a), sign the text that was shown.

**Q3. What are the default quote terms in Hindi? (blocks Step 7)**

The quote form fills three fields with English defaults
([messages.ts:763](../../../../crates/roym_web/ui/src/screens/messages.ts#L763),
`:768` and `:773`: "24 hours notice required for full refund" and two more). The
person can edit them, and the result is signed into the quote's `terms`.

- (a) **Defaults from the string catalog.** A Hindi user gets Hindi
  defaults, edits them, and signs what the fields hold. Same reason as Q2
  (a). *Recommended.*
- (b) **English defaults in every language.**
- (c) **No defaults.** Empty fields with a placeholder only. This changes
  the English Hub too (a behaviour change for English users).

*Owner answer:* (a), defaults from the string catalog.

**Q4. How are codes from the system shown? (blocks Steps 6 to 10)**

Some screens show a code as a word: the message state (`delivered`,
`deposited`; [messages.ts:551](../../../../crates/roym_web/ui/src/screens/messages.ts#L551)
shows `m.state` as it came), the listing status and other `<select>` options
in the listing editor (`active`, `draft`, `per-hour`, …;
[listings.ts:241](../../../../crates/roym_web/ui/src/screens/listings.ts#L241)),
the listing block names (`booking`, `payment`, …;
[listings.ts:77](../../../../crates/roym_web/ui/src/screens/listings.ts#L77),
where the same word is also the CSS class `block-<name>`), and the safety
report categories ([safety.ts:89](../../../../crates/roym_web/ui/src/screens/safety.ts#L89)).
e2e steps read some of them by English text: story 20 checks
`.message-state` has the text `delivered`
([20-roym-chat.spec.ts:76](../../../../crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts#L76)),
and `roym-hub.spec.ts` picks options by value (`selectOption('draft')`,
`:764`; `selectOption('harassment')`, `:447`).

- (a) **A label per code, from the string catalog.** Each table becomes a
  `Map` of `messageId(…)` values read with `lookupText()`, as B1a did for
  booking states (D-B1a-15). The English label is the text shown today
  (the code itself, for example `delivered`), so English and every e2e
  step stay the same. The Hindi label is a Hindi word. A code that is not
  in the table is shown as it came. The option `value` stays the code.
  *Recommended.* It meets ML-1 and keeps the comment's rule at
  `messages.ts:549` ("the state word is exactly what the API returned"):
  one code gives one word, never an inferred one.
- (b) **Keep the raw codes in every language.** They are codes, like an
  unknown category in ML-7. But a Hindi screen then has English words in
  its main controls.

*Owner answer:* (a), a label per code. The list of tables is longer than
the first plan said (review P3); D-B1b-6 now has the full list. The
screen-level check was weak (review P2); D-B1b-12 now makes a raw code fail
the Hindi sweep.

**Q5. Is there a Hindi sweep test? (blocks Step 11)**

The build check (Q1) reads source code. It cannot see a single word that
is kept in a variable or a table and drawn later, for example a
placeholder `input("title")` or a select option list. A test can: draw each
screen in Hindi with a fake `/rpc`, collect every visible text, and fail if
an English word is left.

- (a) **Add `src/i18n/hindi-screens.test.ts`** (D-B1b-12). It draws the 11
  screens and the 3 dialogs in Hindi. Words in Latin letters that `hi.ftl`
  itself uses (Roym, SynOrg, DID, and a command) and the fake data are
  allowed. *Recommended.* It catches what the static check cannot, and it
  stays useful for B2 and later screens.
- (b) **Only the static check.** Review must find the rest.

*Owner answer:* (a), the Hindi sweep, with the review P2 changes
(D-B1b-12).

**Q6. How does the owner review the Hindi text? (blocks Step 13 and the
merge)**

Owner answer 3 in task.md: the owner reviews `hi.ftl` before B1b merges.
After B1b, `hi.ftl` has about 560 messages (about 160 from B1a, about 400
new).

- (a) **A side-by-side review file.** The implementer writes
  `hi-review.md` in this folder: one table row per message (id, English,
  Hindi, the screen where it shows). The owner marks the rows to change.
  The fix session changes `hi.ftl`, sets the line at the top of `hi.ftl`
  to `# owner review: done <date>`, and deletes `hi-review.md` before the
  merge. *Recommended:* one table is quicker to read than two files.
- (b) **The owner reads `hi.ftl` in the PR diff.**
- (c) **The owner checks the screens in a running Hub.**

*Owner answer:* (a), the side-by-side file, with one more column that marks
the legal and safety notices, so the owner reads those rows first
(D-B1b-13).

**Q7. Does B1b extend demo story 40? (blocks Step 12)**

task.md's demo plan builds story 40 in B1a, B2 and B4b. B1b adds something
a viewer can see: every screen in Hindi (AGENTS.md "Mandatory Demo
Close-out").

- (a) **One more step in story 40.** After the reload, Alice opens the
  Contacts tab, and its button text is the Hindi message (read with
  `hubMessage('hi', …)`, so an owner edit of the Hindi does not break the
  story). *Recommended.* Small, and it shows the slice.
- (b) **No change.** The change doc's `demo:` already lists story 40.

*Owner answer:* (a), one more step in story 40.

### 2.2 Points found while reading

1. **What is left to move.** A parse of every non-test `src/**/*.ts` file
   with the TypeScript compiler on `1a62c26a` (the same parser as
   `scan.ts`) finds 333 string literals with a letter that go straight to
   a visible sink: 174 `text(…, "…")`, 93 `textContent =`, 52
   `placeholder =` and 14 `field("…", …)`. By file:
   `screens/messages.ts` 59, `screens/directory.ts` 59, `screens/synorg.ts`
   50, `screens/synorg_members.ts` 44, `screens/listings.ts` 32,
   `screens/groups.ts` 29, `screens/safety.ts` 17, `screens/memberships.ts`
   14, `screens/contacts.ts` 8, `screens/backup.ts` 8, `screens/setup.ts`
   7, `screens/profile.ts` 6. 77 of them are templates with values
   (`` `Save failed: ${…}` ``). About 150 more sentence-like literals are
   not at a sink: returned by a word function
   ([directory/membership.ts:57](../../../../crates/roym_web/ui/src/directory/membership.ts#L57)
   to `:162`; [directory.ts:66](../../../../crates/roym_web/ui/src/screens/directory.ts#L66)
   to `:83`; `stateHint` and `refusalText` at
   [messages.ts:478](../../../../crates/roym_web/ui/src/screens/messages.ts#L478)
   and `:489`), passed to a local helper (`input("…")`, `checkbox("…")`
   and a local `labelled("…", …)` in `screens/listings.ts`, 51 literals),
   thrown as a Hub-made error (point 6), or kept in a table
   ([backup.ts:7](../../../../crates/roym_web/ui/src/screens/backup.ts#L7):
   five `label` values). Single words kept in a table or a variable are
   on top of that (point 5). Files with no visible text left: the cards,
   `main.ts` (except the sample data, point 7), `events.ts` (only
   `console.error`), `screens/message_search.ts`, `session/enrolment.ts`,
   `csp.ts`.
2. **Four copies of the `text()` helper and one copy of `field()`.**
   `dom.ts:5` has `text(tag, value, className)`. The same function is
   written again in
   [listings.ts:30](../../../../crates/roym_web/ui/src/screens/listings.ts#L30),
   [backup.ts:40](../../../../crates/roym_web/ui/src/screens/backup.ts#L40),
   [messages.ts:99](../../../../crates/roym_web/ui/src/screens/messages.ts#L99)
   and, as `textNode`, in
   [safety.ts:82](../../../../crates/roym_web/ui/src/screens/safety.ts#L82).
   The local `labelled(labelText, control)` of
   [listings.ts:37](../../../../crates/roym_web/ui/src/screens/listings.ts#L37)
   builds the same DOM as `field(labelText, control)` in `dom.ts`
   (`label.field` with a `span` and the control), and is a different
   function from `dom.ts`'s `labelled(parent, label, …nodes)`. B1b deletes
   the copies (D-B1b-5), so the check has one name per sink.
3. **A notice that is shown and then signed.** See Q2.
   `ADDRESS_DISCLOSURE_NOTICE` and `DECLINE_NOTE`
   ([messages.ts:31](../../../../crates/roym_web/ui/src/screens/messages.ts#L31),
   `:35`) and the two delete notes (`:17`, `:22`) are only shown.
4. **Two more Rust tests read Hub files that B1b changes.**
   `the_ui_notices_match_this_crate`
   ([transaction/tests.rs:779](../../../../crates/roym_core/src/transaction/tests.rs#L779))
   parses `DEFAULT_DATA_USE_NOTICE` and `ADDRESS_DISCLOSURE_NOTICE` out of
   `screens/messages.ts` with a small TypeScript-string parser.
   `no_instant_removal_and_withheld_revocation_notices_match_the_hub_verbatim`
   ([membership/tests.rs:561](../../../../crates/roym_core/src/membership/tests.rs#L561))
   reads `directory/membership.ts`. When the texts move, both tests read
   `src/i18n/en.ftl` with `content.contains(…)`, as the two B1a tests do
   (D-B1b-8). The doc comments that name the old files:
   [transaction.rs:33](../../../../crates/roym_core/src/transaction.rs#L33)
   to `:45`, [membership.rs:26](../../../../crates/roym_core/src/membership.rs#L26)
   and [membership.rs:602](../../../../crates/roym_core/src/membership.rs#L602).
   The other Rust tests that read Hub files (`money.rs`, `card.rs`,
   `router.rs`) read constants that are not text; they do not change.
5. **Codes shown as words.** See Q4. Also
   `text("span", m.state, \`message-state state-${m.state}\`)`: the class
   keeps the code in every language.
6. **Hub-made errors reach the person through `errText()`.** For an error
   that is not an `RpcError`, `errText()` returns `err.message`
   ([dom.ts:40](../../../../crates/roym_web/ui/src/dom.ts#L40)). The Hub
   throws English text in 18 places: 11 `ListingInputError` in
   `listings/editor.ts` (`:121` to `:296`; `:169` only re-wraps a
   `MoneyInputError`), 6 `Error` in
   `session/login.ts` (`:147`, `:157`, `:234`, `:238`, `:268`, `:300`; shown
   after "Sign-in failed:"), and `rpc.ts:69` (`"RPC error"`, the detail when
   a service sends an empty message). `money.ts` and `directory/search.ts`
   already use `t()`. Errors that the browser throws (a `SyntaxError` from
   `JSON.parse` of a bad backup file at
   [backup.ts:112](../../../../crates/roym_web/ui/src/screens/backup.ts#L112),
   a `DOMException` from `crypto.subtle`) also reach `errText()` with
   English text. D-B1b-4 handles both kinds.
7. **Sample card data in `main.ts`.** `renderHome` draws a gallery of
   sample cards with English data
   ([main.ts:318](../../../../crates/roym_web/ui/src/main.ts#L318) to
   `:391`). It is data, like a listing from another node, and B1a kept it
   as data. B1b moves it to a JSON file, so the sentence rule of the check
   needs no exception (D-B1b-11).
8. **The test hook shows three notices.** `window.RoymMessages` carries
   `DEFAULT_DATA_USE_NOTICE`, `ADDRESS_DISCLOSURE_NOTICE` and `DECLINE_NOTE`
   ([main.ts:66](../../../../crates/roym_web/ui/src/main.ts#L66) to `:73`).
   No e2e spec reads them (the specs call only `openQuoteForm`,
   `renderCardActions` and `openDeclineDialog`, `roym-hub.spec.ts:1060`,
   `:1156`, `:1193`). As catalog messages they can no longer be constants
   made when the module loads (D-B1a-3), so B1b removes the three fields.
9. **e2e steps that read English text.** `hub-helpers.ts` finds buttons by
   name (`Profile`, `Contacts`, `Upsert Contact`, `Open conversation`, …),
   and the specs check whole notices (`roym-hub.spec.ts:395`, `:1036` to
   `:1040`, `:1063` to `:1066`, `:1166`; `roym-trust.spec.ts:12`). Playwright
   asks for `en-US`, so the Hub is English there. All of them keep working
   because English does not change. No e2e step reads a placeholder of a
   Hub screen (the `Write a comment...` placeholders in `webrtc.spec.ts` and
   others belong to other test apps).
10. **The Search button and two Hindi messages (B1a review R6).**
    The button is drawn as `Search`
    ([directory.ts:227](../../../../crates/roym_web/ui/src/screens/directory.ts#L227)).
    The Hindi messages `directory-search-failed` and
    `directory-search-not-started` tell the person to press `"Search"`
    ([hi.ftl:33](../../../../crates/roym_web/ui/src/i18n/hi.ftl#L33),
    `:34`). B1b moves the button label and gives the two messages a
    `$button` variable that holds the button's own message (Step 9).
11. **Labels with spaces at the ends.** `"Display Name: "` and
    `" Conversation Address: "`
    ([profile.ts:16](../../../../crates/roym_web/ui/src/screens/profile.ts#L16),
    `:23`) and `" (owner)"`
    ([groups.ts:496](../../../../crates/roym_web/ui/src/screens/groups.ts#L496)).
    Fluent removes spaces at the start and end of a message (D-B1a-3). B1b
    keeps them as separate `" "` text nodes, so the English DOM text stays
    the same.
12. **A command inside a sentence.**
    [setup.ts:17](../../../../crates/roym_web/ui/src/screens/setup.ts#L17)
    holds `roymctl roym enrol-signing --master <identity> --registry-url <registry>`.
    B1a put a command inside a message too (`login-intro`, `en.ftl:180`).
    B1b does the same: the command is part of the message text, the same
    in both files (a translator keeps it). No exception in the check.
13. **Counts inside sentences.** `` `Imported: ${count} record(s).` ``
    ([backup.ts:116](../../../../crates/roym_web/ui/src/screens/backup.ts#L116),
    where `count` falls back to the word `"bundle"`), `` `${n} messages` ``
    (`groups.ts:276`, `messages.ts:216`), `` `Members (${n})` ``,
    `` `${n} directories searched.` ``. D-B1b-9 has the rule.
14. **Who reads the build.** `npm run build` runs `eslint`, `tsc` and
    `vite build`. `mise run build:roym-ui` runs `npm ci || npm install`
    and `npm run build`. The `hub-ui` gate runs `mise run test:roym-ui`
    (`npm test`, which is `TZ=UTC vitest run`). So a check in a vitest
    file runs in the gate, and a check in the build script runs in every
    build (Q1).
15. **Three copies of the fake `/rpc` helper in tests.** `stubRpc` in
    [groups.test.ts:14](../../../../crates/roym_web/ui/src/screens/groups.test.ts#L14),
    [memberships.test.ts:9](../../../../crates/roym_web/ui/src/screens/memberships.test.ts#L9)
    and [synorg_members.test.ts:7](../../../../crates/roym_web/ui/src/screens/synorg_members.test.ts#L7).
    The Hindi sweep needs the same helper, so B1b moves it to one shared
    test file (D-B1b-12).
16. **One file per language stays right.** `en.ftl` has 159 messages in
    200 lines (10,027 bytes) today. After B1b it has about 560 messages in
    about 650 lines. Fluent and the B1a loader take one file per language
    (D-B1a-1); B1b keeps that and orders each file in sections with a
    `##` heading per screen.
17. **`messages.ts` is over 800 lines** (939). TypeScript files have no
    length gate. B1b removes three constants and the local `text()` copy,
    and replaces literals with `t()` calls, so the file stays about the
    same size. B1b does not split it.

## 3. What B1b meets

| Criterion | What B1b does | Test |
| --- | --- | --- |
| ML-1 | Every visible string in the Hub comes from a string catalog. `npm run build` runs `findVisibleText()` over every source file and fails on a literal at a visible sink or a sentence-like literal anywhere. | `visible-text.test.ts` ("no source file sets visible text outside the string catalog", and the cases for each rule); `hindi-screens.test.ts` (each screen drawn in Hindi has no English word left); the build fails on a planted literal (Step 11, shown in the PR text) |

B1b also extends ML-6 to Hub-made errors and browser errors (D-B1b-4).
ML-6 is already met by B1a; B1b adds test cases to `errors.test.ts` and
names them in the ML-6 row.

**Where decisions and deviations are written.** ML-1 and ML-6 are criteria
of the roym-multilingual change doc, so every B1b decision and deviation
goes there and only there, at close-out (Step 13): decisions into "A. UI
localisation", under the "Decided 2026-11-06, how the Hub does it" list (or
a new dated line); deviations under `## Deviations`.

## 4. Behaviour-change audit

### Wire changes

No method, error code, WIT, record schema, config or CLI changes. Two
signed text fields can now hold Hindi text when the person uses the Hub in
Hindi (Q2 (a), Q3 (a)):

| # | Field | Before | After |
| --- | --- | --- | --- |
| W1 | `request.data_use_notice` | Always the English constant | The notice in the UI language. English Hub and `roymctl`: unchanged |
| W2 | `quote.terms.cancellation_terms`, `refund_terms`, `dispute_path` | English defaults unless edited | Defaults in the UI language unless edited |

Both fields are free text that the service checks only for length. The
Hindi texts must stay under the limits (2,048 bytes each; Hindi is 3 bytes
per character in UTF-8). A test checks this (section 7.1).

### Behaviour changes (Hub only)

| # | Before | After |
| --- | --- | --- |
| 1 | Every screen in English in every language | Every screen in the UI language. English is unchanged, byte for byte |
| 2 | Codes shown raw (message state, select options, block names, report categories) | A label per code (Q4 (a)). English label = today's text; unknown code shown as it came |
| 3 | A Hub-made error shows its English text | The text in the UI language (D-B1b-4) |
| 4 | A browser error (bad backup file, crypto failure) shows the browser's English text | English: unchanged (`{ $detail }`). Hindi: a Hindi sentence, then `विवरण:` and the browser text (`error-unexpected`, D-B1b-4) |
| 5 | A count of 1,000 or more in a sentence shows as `1000` | `1,000` (Indian grouping from 1,00,000), because counts are Fluent number arguments (D-B1b-9). Below 1,000 nothing changes |
| 6 | `window.RoymMessages` has three notice strings | They are removed (point 8 of section 2.2); the three functions stay |
| 7 | The Hindi search messages name a `"Search"` button | They name the Hindi button text (section 2.2 point 10) |
| 8 | The sample card gallery data is in `main.ts` | In `src/cards/samples.json`; the gallery looks the same |

### Stays the same

- Every English string, including spaces and punctuation, except row 5.
  Every e2e step that reads English text keeps working.
- The DOM structure and every class name. The deleted helper copies build
  the same elements as the `dom.ts` helpers (section 2.2 point 2).
- Option values, form field values sent to services, and every RPC call.
- The Rust constants and their values. Two Rust tests read `en.ftl`
  instead of a `.ts` file.
- The B1a mechanism: `t()`, loading, the language list, formats, error
  codes.

## 5. Decisions this plan makes

These are written into the change doc at close-out (section 3).

- **D-B1b-1. What moves, and the one rule for English.** Every string a
  person can see in the Hub comes from `t()`: text, labels, placeholders,
  `title` and `aria-label` values, option labels, help paragraphs, empty
  states, progress lines ("Searching..."), status lines ("Saved."), errors
  that the Hub makes, and words for codes. Data does not: a name, a
  listing title, a DID, a message body, an amount (it goes through
  `formatMinor`), a date (it goes through `format.ts`). The English message
  text is exactly the text the code shows today. A sentence that the code
  builds from parts today becomes one message with variables, and its
  English output stays the same. Improving English wording is not part of
  B1b.
- **D-B1b-2. The check, `findVisibleText(source, fileName): string[]`** in
  `src/i18n/scan.ts`. It parses the source with `ts.createSourceFile` (as
  `findMessageIds` does) and returns one line per problem,
  `<file>:<line>: "<text>" is visible text; use t()`.
  - **The parts of an expression.** For an expression `E`, its literal
    parts are: `E` itself when it is a string literal or a template with
    no `${}`; the fixed parts of a template; and, through `( )`, `as`,
    `satisfies`, `!`, the parts of both branches of `c ? a : b`, both
    sides of `+`, and the right side of `??`, `||` and `&&`. A part fails
    when it has a letter (`/\p{L}/u`). So `" · "`, `"—"` and `""` pass.
  - **Rule 1, visible sinks.** A failing part of `E` in any of these:
    1. `x.<p> = E` where `p` is `textContent`, `innerText`, `placeholder`,
       `title`, `alt`, `ariaLabel` or `label`. `value` and `defaultValue`
       are not in the list: they hold codes too (`currencyInput.defaultValue = "EUR"`,
       `messages.ts:707`). A sentence default, such as the quote terms of
       Q3, is caught by rule 2.
    2. `x.setAttribute(N, E)` where `N` is the literal `aria-label`,
       `aria-description`, `aria-placeholder`, `title`, `placeholder`,
       `alt` or `label`.
    3. Every argument `E` of a call of `append`, `prepend`,
       `replaceChildren`, `before`, `after`, `replaceWith`,
       `createTextNode`, `alert`, `confirm` or `prompt` (as a method or a
       plain function).
    4. The Hub helpers, by name: `text(_, E)`, `field(E, _)`,
       `labelled(_, E, …rest)` (`E` and each `rest`), `input(E, _)` and
       `checkbox(E, _)` (D-B1b-5).
    5. `new Option(E, …)`, and `new X(E, …)` where `X`'s name ends in
       `Error` (a Hub-made error, D-B1b-4).
    6. `{ k: E }` in an object literal where `k` is `label`, `text`,
       `title`, `placeholder` or `hint` (a table of labels such as
       `BUNDLES` in `backup.ts`).
  - **Rule 2, sentences anywhere.** Any string literal or template part,
    outside a type, whose text has two letters with white space between
    them (`/\p{L}\s+\p{L}/u`), unless it is:
    - an argument of a `console.*` call;
    - the right side of an assignment to `className` or `rel`;
    - the third argument of `text(tag, value, className)` (a class list,
      review P1: 47 such strings today, for example
      `"button bundle-export"` and `` `message-state state-${…}` ``);
    - an argument of `classList.add`, `.remove`, `.toggle`, `.replace`,
      `querySelector`, `querySelectorAll`, `closest` or `matches`.
    The exemptions are by position, never by the shape of the text: a
    rule such as "lower case words joined by spaces look like classes"
    would also hide a label such as `"pack size"` (`listings.ts:289`).
    This catches text that a word function returns, text passed to a
    helper the check does not know, and text kept in a constant.
  - **Files.** Every `src/**/*.ts` file except `*.test.ts`, the shared
    test helpers in `src/testing/`, and the two mechanism files
    `src/i18n/i18n.ts` and `src/i18n/scan.ts` (their own texts go only to
    `console.error` or to tests).
  - **`index.html`** is not read. Its only visible text is
    `<title>Roym Hub</title>` (`index.html:17`), which `main.ts:79`
    replaces with `t("hub-title")` at start-up (review P9).
  - **No exceptions in the code.** There is no comment or marker that turns
    the check off. Text that must not be translated (a command, a product
    name) is still a message, with the same text in each file (section 2.2
    point 12). Data that looks like text lives in a `.json` file
    (D-B1b-11).
  - **What it cannot see.** A word with no space that is kept in a
    variable, a table or an array and drawn later (`input(name)` with
    `const name = "title"`). The Hindi sweep (D-B1b-12) covers the screens
    it draws. A backlog row records the limit (section 8).
- **D-B1b-3. Where the check runs.** Q1 (a): `visible-text.test.ts` runs
  `findVisibleText` over every file (with `import.meta.glob("../**/*.ts", { query: "?raw", import: "default", eager: true })`,
  as `catalog.test.ts` does), and `package.json` gets
  `"build": "eslint src/ && tsc && vitest run src/i18n/visible-text.test.ts && vite build"`.
  The `hub-ui` gate runs it with the other tests.
- **D-B1b-4. Hub-made errors and browser errors.**
  - A new file `src/hub-error.ts`: `export class HubError extends Error {}`
    with `name = "HubError"`. A `HubError` is a failure whose message the
    Hub shows as it is, with no wrapper: either Hub-made text in the UI
    language, or a service text that the Hub shows as the detail of its
    own message (the login case below). The class comment uses these words
    (review P12), so no later change treats every `HubError` message as
    translated. Its own file, so `money.ts`,
    `listings/editor.ts`, `session/login.ts` and `directory/search.ts` can
    import it without an import cycle through `dom.ts` and `rpc.ts`.
  - `MoneyInputError` and `ListingInputError` extend `HubError` (their
    names and `instanceof` checks stay). `session/login.ts` and
    `directory/search.ts` throw `HubError`. Every Hub-made error text comes
    from `t()` at the place it is thrown.
  - `errText(err)` in `dom.ts`: `RpcError` → as in B1a; `HubError` →
    `err.message` as it is (no wrapper); any other `Error` →
    `t("error-unexpected", { detail: err.message })`; anything else →
    `t("error-unexpected", { detail: String(err) })`.
    English `error-unexpected = { $detail }`, so English output does not
    change. The Hindi message is a Hindi sentence that ends with
    `विवरण: { $detail }`, like the other error messages (D-B1a-12).
  - `rpc.ts:69`: `json.error.message || t("error-no-detail")`, English
    `error-no-detail = RPC error`.
  - `login.ts` errors that show a status code pass it as a string
    (`{ status: String(res.status) }`), as B1a does for codes.
  - **The login reply with a service text** (`login.ts:261` to `:268`,
    review P6) has two branches. With no `body.error`, it throws
    `HubError(t("login-error-http", { status }))`. With `body.error`, it
    throws `HubError(String(body.error))`: the service text as it came, as
    a `HubError`, so `errText` does not wrap it in `error-unexpected`, and
    `login-failed` shows it as its one `$detail`. So a Hindi person reads
    `विवरण:` once.
- **D-B1b-5. One copy of each helper.** Delete the local `text()` in
  `listings.ts`, `backup.ts`, `messages.ts` and `textNode()` in `safety.ts`;
  import `text` from `dom.ts`. Replace the local `labelled(label, control)`
  of `listings.ts` with `field(label, control)` from `dom.ts` (same DOM).
  Move `input(placeholder, value)` and `checkbox(label, checked)` from
  `listings.ts` to `dom.ts` unchanged, so the check knows them by name and
  other screens can use them. `select(options, value)` stays in
  `listings.ts` but takes only `[value, label]` pairs (the plain-string
  form, where a code was also its label, is removed), so every option has
  a label from the string catalog. `block(name)` becomes
  `block(kind, label)`: `kind` keeps the class `block-<kind>`, `label` is
  the shown text.
- **D-B1b-6. Codes shown as words.** Q4 (a). Each set of codes is a
  `Map<string, string>` of `messageId(…)` values, read with
  `lookupText(table, code) ?? code` (D-B1a-15). The English text of each
  message is the word the Hub shows today. The tables (review P3):

  | Table | Codes | Where the code is drawn |
  | --- | --- | --- |
  | message state | `delivered`, `deposited`, `failed`, … | `messages.ts:551`, and `groups.ts` if it shows states |
  | listing status | `active`, `draft`, `withdrawn` | the editor select (`listings.ts:241`) and the listing row (`listings.ts:141`) |
  | booking mode, pricing model, condition, address disclosure | the editor's select codes | `listings.ts:254`, `:269`, `:290`, `:318` |
  | listing block | `booking`, `payment`, `product`, `service`, `location`, `relationship`, `service_record` | `block(kind, label)` (D-B1b-5) |
  | report category | `impersonation`, `fraud`, … | `safety.ts:89` (select) and the report row (`safety.ts:135`) |
  | report subject kind | `person`, `listing`, `message` | `safety.ts:104` (select) and the report row |
  | report status | `open`, `withdrawn` | the report row (`safety.ts:135`) |
  | credential status | `revoked`, `current`, `replaced` (made by the Hub) | `synorg_members.ts:257` |

  The report row becomes one message with variables
  (`safety-report-row = { $category } - { $kind } { $subject } [{ $status }]`),
  where `category`, `kind` and `status` are table texts and `subject` is
  the id as it came. The code stays wherever it is not shown text: option
  `value`s, class names (`status-${…}`) and `dataset.status`
  (`synorg_members.ts:250`, `:301`). Two things stay as they came: a category code
  (`synorg_members.ts:121`, and the category list inside the
  `covers …` row at `:253`, whose word "covers" does move), because
  the taxonomy and its labels are B2's work and an unknown code is shown
  raw (ML-7); and a verifier `reason` from this node
  (`directory.ts:395`); only its fallback sentence
  `"signature did not verify"` moves. The existing `listings/labels.ts`
  tables (B1a) are reused, not copied. Each new table gets the three
  `constructor`, `toString`, `__proto__` cases.
- **D-B1b-7. Text the Hub writes into a signed record.** Q2 (a) and Q3 (a):
  the data-use notice and the three default quote terms come from `t()`,
  and the Hub signs the text that the person was shown or left in the
  field. The English messages equal today's text; `request-data-use-notice`
  equals the Rust constant `DEFAULT_DATA_USE_NOTICE` (a Rust test checks
  it, D-B1b-8). The doc comment at `transaction.rs:33` changes to say the
  constant is the English text and the `roymctl` default, and the Hub
  signs the notice in its UI language.
- **D-B1b-8. Rust-pinned English.** The four notices that Rust holds
  (`DEFAULT_DATA_USE_NOTICE`, `ADDRESS_DISCLOSURE_NOTICE`,
  `NO_INSTANT_REMOVAL_NOTICE`, `WITHHELD_REVOCATION_NOTICE`) become one
  `en.ftl` message each, on one line, with the exact Rust text, under the
  ids `request-data-use-notice`, `quote-address-disclosure-notice`,
  `membership-no-instant-removal` and `membership-withheld-revocation`. The
  two tests of section 2.2 point 4 read `../roym_web/ui/src/i18n/en.ftl`
  and check for the whole line, not only the text (review P5):
  `assert!(content.lines().any(|l| l == format!("{id} = {CONSTANT}")))`,
  one per notice. A sentence left in a comment, or under another id, then
  does not pass. The small TypeScript-string parser in
  `transaction/tests.rs` (`parse_ts_const_string`) is deleted. None of the
  four sentences holds `{`, `}` or a line that starts with `[`, `*` or `.`.
  The e2e checks of these texts (section 2.2 point 9) stay.
- **D-B1b-9. Sentences, values and spaces.**
  - One message per sentence. A value goes in as a variable. A sentence is
    never put together from two translated parts, with one exception: a
    field name inside a listing editor error (`{ $field } is required`) is
    passed as a translated noun (`t("listing-field-lead-time")`), because
    it is a plain noun that fits any sentence in Hindi.
  - A label followed by a control or a value uses `field()` or
    `labelled()` (D-B1a-3). A space at the start or end of today's text is
    a separate `" "` text node (section 2.2 point 11).
  - **Counts** are number arguments, so a translator can add plural
    variants and the digits are grouped (row 5 of section 4). The English
    message keeps today's words (`record(s)`). The backup fallback word
    `"bundle"` becomes its own message (`backup-imported-unknown`, English
    `Imported: bundle record(s).`).
  - **Codes, ids, DIDs, statuses and years** are string arguments
    (D-B1a-3).
- **D-B1b-10. Shared generic messages.** A message is shared between two
  places only when the English text and the meaning are the same and the
  sentence is generic: `common-load-failed` (`Could not load: { $detail }`),
  `common-save-failed` (`Save failed: { $detail }`), `common-saved`
  (`Saved.`), `common-cancel` (`Cancel`), `common-remove` (`Remove`).
  Everything else has its own id under its screen's prefix.
- **D-B1b-11. Sample data is data.** The `samples` array of `renderHome`
  moves to `src/cards/samples.json`, unchanged, and `main.ts` imports it.
  `tsconfig.json` gets `"resolveJsonModule": true` (Vite reads JSON with no
  setting; `tsc` needs it). The visible-text check reads only `.ts` files.
- **D-B1b-12. The Hindi sweep** (Q5 (a)), `src/i18n/hindi-screens.test.ts`:
  - `beforeAll`: `await activate("hi")`; `afterAll`: `await activate("en")`.
  - The fake `/rpc`: the `stubRpc(handlers)` function of the three test
    files (section 2.2 point 15) moves to `src/testing/stub-rpc.ts`, and
    the three files import it. The sweep gives each screen small fixtures
    with one row per list, so rows draw too.
  - Drawn: `renderProfile`, `renderContacts`, `renderMessages` (with a
    conversation and messages in the states `delivered`, `deposited` and
    `failed`), `renderGroups`,
    `renderListings` (with an `active` and a `draft` listing),
    `renderDirectory`, `renderMemberships`,
    `renderSynOrg` (with settings, so the members panel draws, and a
    revoked and a current credential),
    `renderSafety` (with an `open` report), `renderBackup`, `renderSetup`, and the dialogs
    `openQuoteForm`, `openDeclineDialog` and the request form.
  - Collected: every text node, and the `placeholder`, `title` and
    `aria-label` attributes, and `<option>` text.
  - **Data and codes in the fixtures** (review P2). A value that a person
    types or another node sends (a name, a title, a body, a DID, an id)
    holds the marker `Zq` (for example `Zq-title`, `did:key:zZqAlice`).
    The test removes each word that holds `Zq`. A code is never marked:
    the fixtures use the real codes (`delivered`, `deposited`, `active`,
    `draft`, `fraud`, `open`, `person`, and a revoked and a current
    credential), so a screen that shows a code raw fails the sweep.
  - **Allowed Latin words** are a fixed list in a shared test helper,
    `src/testing/latin-words.ts` (review P10), not in the test file. It
    exports the list, each item with a comment, and
    `leftoverLatin(text): string[]`, which removes the `Zq` words and the
    allowed words and returns the Latin words that are left. Step 3
    creates it, and Step 6 and the sweep import it. The list: `Roym`, `SynOrg`, `DID`, `roymctl` and the words of the
    commands inside messages (`--master`, `--registry-url`, …), and, as
    separate named items, the time zone name `UTC` (the tests run with
    `TZ=UTC`), the currency codes of the fixtures and the `detail` text
    of a fake `/rpc` error. The list is not read from `hi.ftl`: an English
    word pasted into a Hindi message would then be allowed everywhere. A
    new Latin word needs an edit of the list, which a reviewer sees.
  - Fails: a remaining word of two or more Latin letters
    (`/[A-Za-z]{2,}/`). The failure lists the screen and the text.
  - A catalog test makes sure the allowed words do not hide English: every
    `hi.ftl` message has at least one Devanagari letter, except the ids in
    `NO_WORDS` (`terms-range-separator`, `tab-synorg` and the command
    messages; each with a comment).
- **D-B1b-13. The Hindi text and its review.** An agent writes it (owner
  answer 3), under the rules of section 7.7. Q6 (a): `hi-review.md` in this
  folder for the owner, with the columns id, English, Hindi, screen and
  **weight**. The weight column marks the messages with legal or safety
  weight, and the table lists those rows first: the two delete notes, the
  address-disclosure notice, the data-use notice, the decline note, the two
  membership notices, the group delivery and join notices, the
  quote-expiry and quote-accept texts, and the agreement and terms words
  of the cards (`terms-*`, `receipt-*`). A wrong word there changes what a
  person agrees to. The table covers all of `hi.ftl`, B1a's messages too
  (they are still `owner review: pending`); the fix session applies the owner's changes, sets
  `# owner review: done <date>` at the top of `hi.ftl`, and deletes
  `hi-review.md`. The PR does not merge before that.
- **D-B1b-14. Message ids** follow D-B1a-13: lower case, `-` between
  words, the screen's prefix first: `profile-`, `contacts-`, `messages-`,
  `request-`, `quote-`, `group-`, `listing-`, `slot-`, `directory-`,
  `membership-`, `synorg-`, `safety-`, `backup-`, `setup-`, `login-`,
  `common-` (D-B1b-10), `error-`. Inside a prefix, the place comes next
  (`listing-editor-title`, `listing-field-title`,
  `listing-error-title-required`).
- **D-B1b-15. Each `.ftl` file is ordered by screen**, with a `## <screen>`
  comment line before each section, in the same order in both files. B1a's
  messages stay where they are.

## 6. Step-by-step changes

Each step builds (`npm run build` in `crates/roym_web/ui`; until Step 11
the build does not run the check yet), and its tests pass
(`mise run test:roym-ui`). Paths are under `crates/roym_web/ui/` unless
they start with `crates/` or `docs/`.

For every step that moves text: add each message to `en.ftl` and `hi.ftl`
in that screen's section (D-B1b-15); replace the literal with `t()`; run
the step's tests; then check English by a diff: every literal removed from
a `.ts` file in the step appears, with its variables, in the `en.ftl`
lines added in the step. A small throwaway script in `$TMPDIR` that reads
`git diff` does this; paste its "all found" line in the PR text. Do not
commit the script.

### Step 1 — Commit this plan

Done in the planning session: this file and the B1b row of
[status.md](status.md).

### Step 2 — The check, in report mode

1. `src/i18n/scan.ts`: add `findVisibleText(source, fileName)` (D-B1b-2).
   Keep `findMessageIds` as it is. About 100 more lines.
2. `src/i18n/visible-text.test.ts`:
   - The scanner cases of section 7.1.
   - "no source file sets visible text outside the string catalog": runs
     over every file of D-B1b-2. Until Step 11 it skips the files in a
     list `NOT_MOVED_YET` (every file the first run reports). Each later
     step removes its files from the list. The test also fails when a
     listed file has no problem left, so the list cannot go stale.
3. `npm test` passes. The build does not run the check yet.

### Step 3 — Shared pieces

1. `src/hub-error.ts` (D-B1b-4). `money.ts`: `MoneyInputError extends
   HubError`. `listings/editor.ts`: `ListingInputError extends HubError`.
   `directory/search.ts`: throw `HubError`.
2. `dom.ts`: `errText` as in D-B1b-4. Add `input()` and `checkbox()` from
   `listings.ts` (D-B1b-5).
3. `en.ftl` / `hi.ftl`: `error-unexpected`, `error-no-detail`, and the
   `common-` messages of D-B1b-10.
4. `rpc.ts:69`: `t("error-no-detail")`.
5. `src/testing/stub-rpc.ts` (D-B1b-12); the three test files import it.
   `src/testing/latin-words.ts` (D-B1b-12, review P10), with a small test
   of `leftoverLatin` in `src/testing/latin-words.test.ts`.
6. `errors.test.ts`: the new cases of section 7.1.
7. Remove `rpc.ts`, `dom.ts`, `money.ts` and `directory/search.ts` from
   `NOT_MOVED_YET` if they are on it and now clean (review P4). The
   "listed file is already clean" check tells you which.

### Step 4 — Sign-in, gallery data, shell leftovers

1. `session/login.ts`: the 6 errors → `HubError(t(…))`
   (`login-error-odd-hex`, `login-error-key-length`,
   `login-error-challenge` with `{ status }`, `login-error-no-assertion`,
   `login-error-http` with `{ status }`, `login-error-methods` with
   `{ status }`). When the login reply has `body.error`, that service text
   stays as it came (it is a detail, shown after `login-failed`).
2. `main.ts`: the samples → `src/cards/samples.json` (D-B1b-11);
   `tsconfig.json` `"resolveJsonModule": true`. Remove the three notice
   fields from `window.RoymMessages` and its type (section 2.2 point 8).
3. `login.test.ts` keeps its English expectations. Add the two branches
   of D-B1b-4 ("The login reply with a service text") in Hindi: the text
   shown by `login-failed` has `विवरण:` once (review P6).
4. Remove `session/login.ts` and `main.ts` from `NOT_MOVED_YET`.

### Step 5 — The small screens

`screens/profile.ts` (6 literals; the two labels with edge spaces, section
2.2 point 11), `screens/contacts.ts` (8), `screens/backup.ts` (8 and the 5
`BUNDLES` labels; counts by D-B1b-9; local `text()` removed),
`screens/setup.ts` (7; the command, section 2.2 point 12),
`screens/safety.ts` (17; `textNode` removed; report categories as an id
table, D-B1b-6). Remove these files from `NOT_MOVED_YET`. Update
`contacts.test.ts` only where it imports a moved name.

### Step 6 — Listings and the listing editor

1. `screens/listings.ts` (32 literals at sinks, 51 more in helper calls):
   delete the local `text`, `labelled`, `input`, `checkbox`
   (D-B1b-5); `block(kind, label)`; `select` with pairs only; option label
   tables for status, booking mode, pricing model, condition and address
   disclosure (D-B1b-6). `serviceLocationEditorLabel` and `openToLabel`
   (B1a) are reused.
2. `listings/editor.ts`: the 11 `ListingInputError` texts → `t()`. All
   eight field names become `t("listing-field-…")` (D-B1b-9; review P7):
   `lead time`, `cancellation window`, `max per booking` (`:158` to
   `:160`), `pack size` (`:205`), `duration` (`:214`), `warranty`,
   `retention` (`:246`, `:247`) and `capacity` (`:294`). The four
   one-word names are not seen by the check (D-B1b-2, "What it cannot
   see"), so this list is the rule.
3. Tests: `listings.test.ts` and `editor.test.ts` keep their English
   expectations. Add one table test in Hindi over every `ListingInputError`
   path: the 11 throws, the "whole number" error for each of the 8 field
   names, and the "is required" error for the 7 names that go through
   `requireWhole` (`capacity` goes through `toWholeCount` only, and an
   empty capacity gives the slot error at `editor.ts:296`, one of the 11;
   review P11). Each message has a Devanagari letter, and
   `leftoverLatin()` from `src/testing/latin-words.ts` returns nothing
   (review P7, P10).
4. Remove the two files from `NOT_MOVED_YET`.

### Step 7 — Messages, requests, quotes

1. `screens/messages.ts` (59 literals, 21 more): the delete notes, the
   decline note, `ADDRESS_DISCLOSURE_NOTICE` and the data-use notice
   become messages (D-B1b-7, D-B1b-8); the exported constants are removed
   and their users call `t()`. `stateHint`, `refusalText`, `"Unfiled card"`,
   `"(no body)"`, `"(message deleted)"` → `t()`. The state word → id table
   (D-B1b-6). The quote form defaults → `t()` (Q3). Local `text()` removed.
2. `request.set` sends `data_use_notice: t("request-data-use-notice")` —
   the same call that draws the notice text (Q2 (a)).
3. Rust: `the_ui_notices_match_this_crate` reads `en.ftl` (D-B1b-8);
   update the doc comments at `transaction.rs:33` to `:45`. Before the
   change, run the test once against an `en.ftl` with one notice changed,
   to see it fail. Run
   `cargo nextest run -p syneroym-roym-core the_ui_notices_match_this_crate`.
4. Tests: `messages.test.ts` imports `t()` where it used the constants.
   Add the length test of section 7.1.
5. Remove the file from `NOT_MOVED_YET`.

### Step 8 — Groups

`screens/groups.ts` (29 literals, 4 more: `"Unnamed group"` three times,
`" (owner)"`). Counts by D-B1b-9. The message state table of Step 7 is
reused if groups show states. `groups.test.ts` keeps its English
expectations. Remove the file from `NOT_MOVED_YET`.

### Step 9 — Directory and memberships

1. `screens/directory.ts` (59 literals, 29 more): `checkOutcomeWords`,
   `sourceErrorWords` and the help paragraphs. The Search button label
   becomes `directory-search-button`. The two search messages get a
   `$button` variable, and `directory/search.ts` passes
   `{ button: t("directory-search-button") }` (review P8): English
   `… Press { $button } to try again.` gives the same text as today, and
   an owner edit of the button text changes all three at once.
2. `directory/membership.ts` (25): the word functions return `t(…)`;
   `NO_INSTANT_REMOVAL_NOTICE`, `WITHHELD_REVOCATION_NOTICE` and
   `ISSUER_CHANGED_CHECK_WORDS` become messages, and every user calls
   `t()`: `directory.ts:68`, `memberships.ts:54`, `:55`, `:111`,
   `memberships.test.ts:2`, and also `synorg_members.ts:74` (with its
   import at `:1`), although the rest of that file moves in Step 10
   (review P4). Otherwise `tsc` fails after this step. A verdict reason that comes from another node stays a `$reason`
   variable.
3. `screens/memberships.ts` (14, 7 more).
4. Rust: the membership notice test reads `en.ftl`; update the doc
   comments at `membership.rs:26` and `:602`. Run
   `cargo nextest run -p syneroym-roym-core notices_match_the_hub_verbatim`
   (and once against a changed `en.ftl`, to see it fail).
5. Tests: `directory.test.ts`, `membership.test.ts`, `memberships.test.ts`
   and `search.test.ts` keep their English expectations; they import `t()`
   where they used a removed constant.
6. Remove the files from `NOT_MOVED_YET`.

### Step 10 — SynOrg

`screens/synorg.ts` (50, 9 more) and `screens/synorg_members.ts` (44, 5
more). `synorg_members.test.ts` keeps its English expectations. Remove the
files from `NOT_MOVED_YET`; the list is now empty.

### Step 11 — The check on, and the Hindi sweep

1. Delete `NOT_MOVED_YET` from `visible-text.test.ts`.
2. `package.json`: the build line of D-B1b-3.
3. Show that the build fails: add `el.textContent = "Hello"` to one
   screen, run `npm run build`, keep the output (it names the file and
   line) for the PR text, and remove the line.
4. `src/i18n/hindi-screens.test.ts` (D-B1b-12) and the `NO_WORDS` test in
   `catalog.test.ts`. Fix every word the sweep finds.
5. `mise run test:roym-ui` passes. `mise run build:roym-ui` passes.

### Step 12 — e2e and story 40

1. Run `mise run test:e2e`. Every spec passes unchanged; English did not
   change. If a step fails on text, the English of a message is wrong:
   fix the message, not the spec.
2. Story 40 (Q7 (a)): one more step after the reload. Alice opens the
   Contacts tab (found by `hubMessage('hi', 'tab-contacts')`), and the
   upsert button has the text `hubMessage('hi', '<its id>')`. The story
   file holds the exact step. `hubMessage` reads only one-line messages,
   so the button message stays on one line.
3. Run `mise run demo`; keep the output for the PR text.

### Step 13 — Hindi review file, docs, size report and close-out

1. `hi-review.md` in this folder (Q6 (a), D-B1b-13): a table of every
   message: id, English, Hindi, screen, weight. The rows with legal or
   safety weight come first; the rest follow in file order. Generate it with a
   throwaway script in `$TMPDIR` from the two `.ftl` files.
2. Size (section 7.6): before (on `main`) and after, `npm run build`, then
   for each `dist/assets/*.js` the raw bytes and `gzip -9 -c | wc -c`.
   Check that the main script still has no Devanagari letter
   (`grep -c '[ऀ-ॿ]'` gives 0). Write the numbers into task.md (B1b
   result line) and the change doc ("Which catalog format").
3. Change doc: decisions D-B1b-1 to D-B1b-15 into "A. UI localisation";
   `## Deviations` for every difference from this plan; the ML-1 Test
   column with real test names; the ML-6 Test column gains the new
   `errors.test.ts` cases. `demo:` stays story 40. `status` stays `draft`
   (B6 sets `done`). `living-docs-touched` lists the files of section 9.
4. Living docs (section 9), deferred backlog (section 8), task.md B1b
   result line, status.md.
5. The import clean-up pass over the two Rust test files.
6. `mise run verify` with no `--skip`, as a background job.

## 7. Tests

### 7.1 Hub unit tests (vitest, `TZ=UTC`)

`src/i18n/visible-text.test.ts`

- The whole source: "no source file sets visible text outside the string
  catalog" (D-B1b-3). After Step 11 it has no skip list.
- Rule 1, one case per sink kind: `el.textContent = "Save"`,
  `el.placeholder = "x y"`, `el.setAttribute("aria-label", "Close")`,
  `el.append("Done")`, `text("p", "Hi")`, `field("Name", c)`,
  `labelled(p, "Payee:", a)`, `input("title")`, `new Option("Draft", "draft")`,
  `throw new HubError("bad")`, `{ label: "Profile" }`: each is reported.
- The expression parts: `c ? "Yes" : t("no")` is reported once;
  `name ?? "Unnamed"` is reported; `` `Saved ${id}` `` is reported;
  `"Total: " + x` is reported; `el.textContent = ""`, `= " · "`,
  `= t("x")`, `= name` and `` = `${a} ${b}` `` are not.
- Class lists (review P1): `text("p", t("x"), "button bundle-export")`
  and ``text("span", v, `state-${s} on`)`` are not reported;
  `text("p", "two words")` is.
- Not sinks: `el.value = "draft"`, `el.className = "card card-quote"`,
  `a.rel = "noopener noreferrer"`, `console.error("feed listener threw")`,
  `el.setAttribute("data-verified", "false")`, `querySelector(".a .b")`,
  `call("profile.get")`: nothing reported.
- Rule 2: `const NOTE = "This is a sentence."` is reported;
  `return "never checked"` is reported; `"Bearer " + token` is not (one
  word); a sentence in a comment is not.
- The problem text has the file and the line.

`src/i18n/hindi-screens.test.ts` (D-B1b-12)

- One test per screen and dialog: drawn in Hindi, no Latin word is left
  after the `Zq` data words and the allowed list are removed.
- A raw code fails: a test-only row that draws `m.state` without the
  table makes the sweep report `delivered`. This shows that codes are not
  removed with the data (review P2).
- A self-check: with `activate("en")`, the same sweep of one screen does
  find English words (so the sweep can fail).

`src/i18n/catalog.test.ts` (added)

- Every `hi.ftl` message has a Devanagari letter, except the ids in
  `NO_WORDS` (D-B1b-12).
- The B1a tests stay: the id sets are equal, variables are a subset,
  every message formats, no unused message (`KEPT_FOR_RUST` stays as it
  is).

`src/i18n/errors.test.ts` (added, D-B1b-4)

- A `HubError("x")` gives `x` in both languages.
- `new SyntaxError("Unexpected token")` gives `Unexpected token` in
  English; in Hindi it starts with Devanagari and has the detail only
  after `विवरण:`.
- A string `"oops"` thrown gives `oops` in English and the Hindi sentence
  in Hindi.
- An `RpcError` with an empty message gives `RPC error` as the detail.

Length of signed text (in `messages.test.ts`, D-B1b-7)

- In `en` and `hi`: `new TextEncoder().encode(t("request-data-use-notice")).length`
  is at most 2,048 (`MAX_NOTICE_LEN`), and each of the three default quote
  terms is at most 2,048 (`MAX_TERMS_TEXT_LEN`).

Code tables (D-B1b-6)

- Each new id table: a known code gives its English word in `en` and a
  different text in `hi`; `constructor`, `toString` and `__proto__` are
  shown as they came.

Existing tests

- Every existing screen test keeps its English expectations (they run in
  English unless they call `activate`). Only imports change, where a test
  used a constant that B1b removes.

### 7.2 Rust

`the_ui_notices_match_this_crate` and
`no_instant_removal_and_withheld_revocation_notices_match_the_hub_verbatim`
read `en.ftl` and look for the whole `<id> = <text>` line (Steps 7 and 9,
D-B1b-8). Before each change, run the test once against an `en.ftl` with
one sentence changed, and once with the id changed, to see it fail. No
`roym_*` code changes (only tests and comments), so no WASM rebuild is
needed for parity.

### 7.3 Playwright

- Every spec runs unchanged in English (Step 12). That checks that the
  English text did not change.
- Story 40 gains one step (Q7 (a)).
- The `hi-IN` describe of `roym-hub.spec.ts` (B1a) is unchanged.

### 7.4 Not tested

- A single word kept in a variable and drawn only on a path that the Hindi
  sweep does not draw (D-B1b-2, "What it cannot see"). Backlog row in
  section 8.
- Whether each Hindi sentence is good Hindi. The tests check form only;
  the owner review checks meaning (D-B1b-13).
- Glyph shaping of Devanagari on each OS (as in B1a).

### 7.5 Size risks

| File | Today | Risk |
| --- | --- | --- |
| `src/screens/messages.ts` | 939 | About ±0 (three constants and a helper go; `t()` calls with arguments add a little). TypeScript has no length gate. Do not let it grow by more than about 20 lines; if it would, move the quote form to `screens/quote_form.ts`. |
| `src/screens/groups.ts` | 677 | About ±0 |
| `src/screens/listings.ts` | 568 | About −50 (five helpers go) |
| `src/dom.ts` | 82 | About +25 (`input`, `checkbox`, `errText` branch) |
| `src/i18n/scan.ts` | 37 | About +100 (`findVisibleText`) |
| `src/i18n/en.ftl`, `hi.ftl` | 200, 189 | About 650 each |
| `src/i18n/visible-text.test.ts` | new | About 150 |
| `src/i18n/hindi-screens.test.ts` | new | About 300 (mostly fixtures) |
| `src/testing/stub-rpc.ts` | new | About 25 |
| `src/testing/latin-words.ts` | new | About 40 (the list with comments, `leftoverLatin`) |
| `crates/roym_core/src/transaction/tests.rs` | — | About −35 (the TS-string parser goes) |
| `crates/roym_core/src/membership/tests.rs` | — | ±0 (a path string) |

No Rust file grows. The Rust test change removes code, so the duplication
ratchet (`cargo xtask check-duplication`, Rust only) may move the other
way: if its gate says so, lower `MAX_EXACT_DUPLICATE_PERCENT` as the gate
message says (AGENTS.md "Mandatory Ratchet Update").

### 7.6 Bundle size

The English text is already in the main script as string literals today.
Moving it into `en.ftl` adds only the ids (in `en.ftl` and at each `t()`
call) and the Fluent syntax. The Hindi text goes into the Hindi chunk,
which loads only for Hindi. After B1a: main script 39,819 bytes and Hindi
chunk 4,763 bytes after `gzip -9` (task.md, B1a result). Expected after
B1b: the main script grows by under 10 KB `gzip -9`, and the Hindi chunk
is under 30 KB `gzip -9`. Step 13 reports the real numbers. If either
limit is passed, stop and tell the owner.

### 7.7 Hindi text

As in B1a (its section 7.7): common words that a Hindi speaker reads on a
phone, product names (Roym, SynOrg, DID) in Latin letters, commands as
they are, and each message about as long as the English one, so buttons
and table columns do not grow much. A message that shows a service's
English text puts it last, after `विवरण:`. Hindi may use Fluent plural
variants for a count (`{ $count -> [one] … *[other] … }`); English keeps
today's words. Use the same Hindi word for the same English word across
all screens (for example one word for "listing", one for "directory");
`hi-review.md` makes this easy to check.

### 7.8 How B1b avoids the kinds of findings S0 and B1a had

- **A state that must exist before setup** (B1a P2): no `t()` call when a
  module loads. Every removed constant becomes a `t()` call where the Hub
  draws, and the test hook drops the three notice strings (section 2.2
  point 8) instead of computing them at load.
- **A failure path with no rule** (B1a P3, P6, P7, R1): Hub-made and
  browser errors now have a rule (D-B1b-4), with tests for each kind.
- **Data from other nodes used as a key** (B1a P5): every new code table
  is a `Map` read with `lookupText`, with the three inherited-name tests
  (D-B1b-6).
- **A test that cannot see the real build** (B1a P8, P10, R7): the check
  runs inside `npm run build` and is shown to fail on a planted literal
  (Step 11); `mise run verify` runs it in the `hub-ui` gate.
- **A text scan that can be fooled** (B1a P12, P14): the check parses with
  the TypeScript compiler; comments and the `"http://"` case are covered;
  there is no marker that turns it off (D-B1b-2).
- **Fluent removes edge spaces** (B1a P13): the labels with edge spaces
  are listed (section 2.2 point 11) and kept as text nodes; the existing
  `.ftl` trailing-space test stays.
- **Literals on an edited line left in English** (B1a R6): the check
  makes this impossible after Step 11, and `NOT_MOVED_YET` fails when a
  listed file is already clean.
- **A claim not checked in the code** (S0 P1, B1a review): every count in
  section 2.2 came from a parse of the source on `1a62c26a`; every Rust
  test that reads a Hub file was found by a search for `ui/src/` in
  `crates/**/*.rs`.
- **Scratch files left behind** (S0 R5, B1a R2): the two throwaway scripts
  (Step 2 English diff check, Step 13 review table) live in `$TMPDIR`;
  `hi-review.md` is deleted before merge (D-B1b-13).

## 8. Deferred backlog

Add in [deferred-backlog.md](../../deferred-backlog.md), §10:

- **"The Hub text check reads only literals"**: `findVisibleText` sees a
  literal at a visible sink and a sentence-like literal anywhere. A single
  word kept in a variable or a table and drawn later passes it; the Hindi
  sweep finds it only on the screens and paths it draws. A later change
  can make visible text a branded TypeScript type that only `t()` returns,
  so `tsc` refuses any other string at a sink. Target `TBD`. Link
  `crates/roym_web/ui/src/i18n/scan.ts` (`findVisibleText`).

No other deferral. The Hindi text in signed fields (Q2, Q3) gets its
language tag in B3a, which is planned work. The row "Accessibility (WCAG
2.1 AA) & localisation/i18n" (§10, line 508) stays: B6 moves its
localisation part. The row "Service-specific refusals show a generic Hindi
sentence" stays. No new `TODO` or `FIXME` marker.

## 9. Living docs to update at close-out

- **`docs/system-requirements-spec.md:1462`** (`[APP-A11Y]`, "Hub
  localisation (built)"): every Hub screen is in the string catalog, in
  English and Hindi, and the Hub build fails on visible text outside it.
  Remove "Most Hub screens still have English text in the code."
  **`:1464`** ("Envisioned"): remove "Hub screens other than the shell and
  the cards in the string catalog, non-English text for them"; keep
  "translated record text".
- **`docs/system-architecture.md:1414`**: add one sentence: the Hub build
  fails when a source file sets visible text that is not from a string
  catalog.
- **`docs/planning/traceability-matrix.md:68`** (`[APP-A11Y]`): evidence
  adds `visible-text.test.ts` and `hindi-screens.test.ts`; remove "Text for
  the other Hub screens". Status stays as it is until B6.
- **`docs/developer-guide.md:1745`** ("Hub text and languages"): a new
  paragraph "The build check": `npm run build` fails on a literal at a
  visible sink or a sentence-like literal (what counts, and that there is
  no marker to turn it off); a command or a product name is still a
  message; data that looks like text goes in a `.json` file; codes shown
  as words use a `Map` id table; a Hub-made error is a `HubError` with
  `t()` text; `src/testing/stub-rpc.ts` for screen tests; the Hindi sweep
  and how to add a screen to it.
- **`docs/demo/demo.md:1260`** (R3b): "the title, the tab names, the
  sign-in box and every card" becomes "every screen"; remove "The other
  screens still show English text for now."; mention the Contacts screen
  check.
- **The change doc**, **task.md**, **status.md**: Step 13.

`docs/TERMINOLOGY.md`: no change ("String catalog (Hub)" exists since
B1a).

## 10. Exit criteria

B1b is done when all of these are true:

1. The owner answered Q1 to Q7, and sections 4 to 9 match the answers.
2. `findVisibleText` reports nothing for any source file, and
   `npm run build` runs it and fails on a planted literal (Step 11 output
   in the PR text) (ML-1).
3. The Hindi sweep passes for every screen and dialog of D-B1b-12, with
   real codes in its fixtures, and its two self-checks show it can fail
   (English, and a raw code).
4. English is unchanged: the Step 2 diff check finds every removed literal
   in `en.ftl`; every e2e spec and demo story passes with no change to a
   text it reads.
5. Hub-made and browser errors follow D-B1b-4, with the
   `errors.test.ts` cases of section 7.1.
6. The two Rust tests read `en.ftl`, check the whole `<id> = <text>`
   line, pass, and fail when one sentence or one id differs.
7. Every new code table is a `Map` with the three inherited-name tests.
8. The signed Hindi texts fit their byte limits (section 7.1).
9. Story 40 passes with its new step in `mise run demo`.
10. The bundle sizes are reported and within section 7.6.
11. The owner reviewed `hi.ftl` with `hi-review.md` (weighted rows
    first); its first comment says
    `# owner review: done <date>`; `hi-review.md` is deleted.
12. The change doc, the living docs, the backlog row, task.md and
    status.md are updated as in sections 8 and 9.
13. Changed paths: `crates/roym_web/ui/` (not `dist/`), two Rust test
    files and three Rust doc comments in `crates/roym_core`,
    `crates/substrate/tests/e2e/demo/40-roym-languages.spec.ts`, and docs.
14. `mise run verify` passes with no `--skip`.
15. One PR.

## 11. What B1b does not do

- No taxonomy, no unit or payment-method codes, no category picker (B2).
  The listing editor's free-text category, unit and method fields keep
  their (now translated) placeholders until B2 replaces them.
- No `lang` on records and no profile `languages` (B3a). The Hindi
  data-use notice and quote terms are not tagged until then.
- No change to English wording, even where it could be better
  (`Upsert Contact`, `record(s)`).
- No new language beyond English and Hindi.
- No redraw without a reload, no native digits, no web font, no
  right-to-left layout.
- No localised text from the substrate or `roymctl`. A service's own
  English text stays a `$detail`.
- No split of `screens/messages.ts` unless section 7.5's limit is passed.
- No change to the signed record schemas, the Rust constants or any wire
  value other than the two text fields of section 4.
