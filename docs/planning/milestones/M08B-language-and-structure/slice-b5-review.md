# M08B slice B5 — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-b5-implementation-plan.md](slice-b5-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Sonnet 5.5, 2026-11-13

Checked on `feat/m08b-b5-translation` (`main` at `6083ddb3` plus the plan
commit), in the main checkout (not a worktree: `.git` is a directory).
`git pull` said "Already up to date".

**Who reviewed.** This session runs on Claude Sonnet 5.5 (the prompt names me
so). `status.md` says Claude Opus 5.5 wrote the plan.

**Change docs.** The prompt lists one change doc,
[roym-multilingual](../../changes/roym-multilingual/change.md). `task.md`
names no other change doc for B5. I agree: ML-15 to ML-19, ML-26 and ML-27
are all in roym-multilingual, so every B5 decision and deviation belongs
there. The plan is also right to treat roym-agreements AG-32 as M08C's: B5
only uses the same field names and types.

**Summary:** 0 blocker, 2 major, 9 minor, 11 items (P1 to P11). After the plan fix (7806e411): 9 Fixed, 2 back to Open in round 2 (P7, P8). See round 2 below.

Limits of this review: I read the plan, `task.md`, `status.md`, the change
doc, the B4b review (for the format and kinds of findings) and the code on
`main`. I did not run any plan step, `cargo`, `npm` or `mise run verify`, and
I changed no code.

### What was checked and found correct

- **File and line links.** Spot-checked and right:
  `app_host/src/lib.rs` `:7` to `:14`, `:40` to `:71`; `app_host/src/guest.rs:455`;
  `app_host_native/src/host.rs:716`, `convert.rs` module list `:75` to `:87`;
  `roym_core/src/signing.rs:697` (`TestHost`); `sandbox_wasm/src/engine/init.rs`
  `:397`, `:412`; `roym_web/src/app.rs` (`WHOAMI`, `admit`, `rpc`, `invoke`,
  `forward_rpc_call`); `roym_web/src/api.rs` (one row); `router.rs` `ROUTES`,
  `PUBLIC_METHODS`, `method_auth`, `route`, and its tests;
  `transaction.rs` `:58` and `:59` (`MAX_SCOPE_LEN` 4096,
  `MAX_TERMS_TEXT_LEN` 2048), `:233` (`AgreementReceiptPayload`, five fields,
  no `deny_unknown_fields`), `:490` (`validate`);
  `agreement_ops.rs` `accept`, `sign_agreement_receipt` (literal `:273`),
  `maybe_countersign` (literal `:436`), `QuoteRecordInput` (`deny_unknown_fields`,
  shared with `agreement.get` in `api.rs:48`);
  `sync.rs:448` (`file_agreement_receipt_card`, compares only `terms`);
  `Response::with_data` (`envelope.rs:50`); `ParamError { field }`
  (`method.rs:66`); `LangTag::parse_canonical` (`text_lang/src/lib.rs:100`);
  `listing.rs:21` and `:22`; Hub `record_text.ts`, `message_text.ts`,
  `main.ts:67` to `:110`, `dom.ts:18` to `:28`, `messages.ts` `:520` to `:550`
  and `:607` to `:655`, `directory.ts` `:211` to `:225` and `:334`;
  `testing/stub-rpc.ts`; backlog rows `:557` and `:611`; ADR numbers (0028 is
  the last, so 0029 is free); scenario 235 is the last parity scenario.
- **Call-site lists.** `impl AppEvents for` has exactly three implementors
  (`GuestHost`, `NativeAppHost`, `TestHost`), so `AppTranslation` needs the
  same three. `syneroym:events` appears in exactly the seven worlds, the seven
  `guest.rs`, the seven `Cargo.toml` and the host crates the plan lists; the
  only linker is `build_wasm_linker`. No code checks a component's imports
  against a manifest. `AgreementReceiptPayload {` literals in `roym_core` tests
  match the plan (`tests.rs:291`, `:347`, `:354`; `lang.rs:362`, `:369`;
  `verify.rs:42`, `:96`, `:315`; `wire_words.rs:342`). The three `json!`
  receipt payloads are exactly `fixtures.rs:337`, `:440` and `booking.rs:60`;
  the other `quote_author_did` literals in the parity files are fulfilment,
  booking and progress payloads, not agreement receipts. `agreement.accept` is
  called with only `quote_record_id` everywhere the plan lists, plus the Hub.
- **Router change is safe.** `translation.` is not a prefix of, and has no
  prefix in common with, any `ROUTES` entry (`transaction.` shares only
  `trans`). `admit` runs before `route`, `admit` treats an unknown prefix as
  `Yes`, so the `WEB_ROUTES` row (`Owner`) is needed and is placed right.
  `invoke` (the internal path) answers `-32010` for an `Owner` method, as the
  plan says.
- **`-32020` is free.** The Hub error table has no entry for it; `-32030` is
  the control plane's.
- **Both builds.** The one no-op is shared: `NativeAppHost` calls the same
  `HostState` functions (`app_host_native` already depends on
  `syneroym-sandbox-wasm`), so the builds cannot drift. Nothing in the
  substrate runtime, the factory, `SubstrateConfig` or the manifest changes.
- **Receipt checks.** `halves_agree` ignores the new fields, so AG-32 holds;
  `LangTag` rejects non-canonical tags at deserialise, so
  `verify_agreement_receipt` needs no new code for that.
- **TypeScript is not under the file-length gate**
  (`xtask/src/file_lengths.rs` reads `*.rs` only), so `messages.ts` (957) and
  `messages.test.ts` (738) can grow.
- **Existing Playwright mocks.** The `page.route('**/rpc')` mocks in
  `roym-hub.spec.ts` fall back to the node for other methods, so the new
  `translation.targets` call at start-up does not break them.
- **"How this is shown to an end user".** Present, simple, honest that nothing
  is visible with v1, and names the developer commands.
- **Steps build in order.** Step 2 adds the host before the guests; the
  `AppHost` change and the seven worlds are in one commit (2.3 and 2.4 are
  not separate commits), so the wasm32 builds are never half-wired. Step 4
  changes every receipt literal in the step that makes the fields required.

### Views on the owner questions

- **Q1 (search hook in the Hub).** I would take (a). The person sees and can
  change the translated query, and only `web` talks to the host. (b) would
  change search results silently. Record the deviation as the plan says.
- **Q2 (all six components and the fixture).** I would take (a). It is the
  proven `events` pattern, and the `generate!` remap rule in
  `app_host/src/lib.rs:7` to `:14` makes a "web only" feature fragile.
- **Q3 (optional inputs).** I would take (a). Every existing caller states the
  truth with no change. The Hub always sends both fields (see P1 for what it
  sends).
- **Q4 (which `viewed_lang` values).** I would take (a). Note for the owner:
  with (a) a client can reach a state the node refuses (P4); the fix is in the
  Hub, not in the rule.
- **Q5 (receipt card shows what each party read).** I would take (a). Nothing
  makes `viewed_via_mt` true in v1, and M08C redraws the card.
- **Q6 (keep `detect-language`).** I would take (a). The change doc draft has
  it, and adding a function later changes the package. The cost is one unused
  function that the parity test exercises.
- **Q7 (ADR-0029).** I would take (a). Also state in it that `translation.*`
  has no per-call cost or rate limit in v1 and that a later engine must add
  one before the owner can name a remote engine.

### Findings

#### P1 — The accept handler reads the shown version from an element that is not drawn when a quote has one version, and `renderCardActions` is also called with no card in it (major)

- **Where:** D-B5-7 ("`recordText` writes the shown version on its host
  element after each draw"; "With one version and no target, the output is
  exactly as today"), D-B5-8 (`wrap.querySelector(...)!`),
  `ui/src/records/record_text.ts:43` to `:51` (the `versions.length === 1`
  early return), `ui/src/main.ts:61` to `:64` (`renderCardActions` is
  exported on `window.RoymMessages`), `roym-hub.spec.ts` tests 30 and 31
  (`:1073`, `:1125`).
- **Problem:** `recordText` returns early for one version and draws no meta
  box. The plan says the quote card "always has" a `data-shown-lang` element
  "after D-B5-7", but it does not say that the early-return path writes the
  two attributes. D-B5-7 even says the output there is "exactly as today". A
  quote with no author translation is the common case (and the only case in
  every existing e2e test). Also `renderCardActions(host, card, …)` is public
  to tests, and tests 30 and 31 call it with an empty `host` and a card with
  no `terms.lang`. Section 2.2 point 15 notes that no test clicks Accept.
- **Failure case:** Bob accepts a quote that has no translation.
  `wrap.querySelector("[data-shown-lang]")` is `null`, and `shown.dataset`
  throws a `TypeError` in the click handler. Nothing is sent and the button
  stays disabled. No planned unit test catches it: the accept tests of section
  7.1 start from a quote with an author translation.
- **Fix:** Say in D-B5-7 that `recordText` writes `data-shown-lang` and
  `data-shown-via` in the one-version path too (original, `terms.lang`).
  Make the handler robust instead of asserting: when no element is found, send
  the card's own `data.terms.lang` and `false` (the original). Add a unit test
  for Accept on a quote with one version (sends `terms.lang` and `false`), and
  one for a bare host with no drawn card. Keep the `!` only in tests.
- *Status:* Fixed (7806e411)

#### P2 — The Playwright test "accepting a quote signs the version the person read" has no set-up to reuse and cannot run on the one-node e2e (major)

- **Where:** Section 7.3 (second test), Step 6, section 3 (ML-18 test column).
- **Problem:** The plan says "Reuse the quote set-up of tests 30 and 31". Those
  tests do not set up a quote: they call `renderCardActions` on a bare `div`
  with a hand-written card object (`roym-hub.spec.ts:1138` to `:1159`), and
  they never click Accept. `global-setup.ts` boots one substrate node. A real
  acceptance needs a quote signed by Alice that is filed on Bob's node, and
  then `agreement.get` on Bob's node. No Playwright spec in `tests/` or
  `demo/` creates a quote (`git grep -n quote` finds none in `hub-helpers.ts`
  or the demo stories), and no demo story accepts one.
- **Failure case:** The implementer cannot write the test as described. They
  either build a two-node Playwright set-up (large, and not in the plan's size
  estimate) or write a weaker test (a synthetic card and a mocked `/rpc`) that
  does not show a signed `viewed_lang`. ML-18's only end-to-end proof is then
  missing.
- **Fix:** Put the signed-result proof where two nodes already exist:
  extend `crates/substrate/tests/roym_transaction_e2e.rs` (it already has
  Alice and Bob and accepts at `:221`) with a quote that has an author
  translation, accept with `viewed_lang`, and read both halves back from
  `agreement.get`. Keep the Hub half as unit tests (exact params, P1). Keep
  only the "no translate action with a v1 node" Playwright test, which needs
  one node. Update section 3, section 7.3 and Step 6.
- *Status:* Fixed (7806e411)

#### P3 — A second golden shape changes: `transaction_thread.json` has the agreement-receipt payload (minor)

- **Where:** Section 4 W6, Step 4.4 ("`UPDATE_METHOD_SHAPES=1` for
  `agreement_verify_good.json` (read the diff: exactly two new keys)"),
  `crates/roym_web/tests/method_shapes/transaction_thread.json`.
- **Problem:** `transaction_thread.json` holds a card of type
  `agreement-receipt` whose `data` is the parsed payload (`counterparty_did`,
  `quote_author_did`, `quote_record_id`, `role`, `terms`). It gets the two new
  keys too. W6 and Step 4.4 name only `agreement_verify_good.json`, and the
  instruction "exactly two new keys" in that one file will not match the real
  diff of two files.
- **Failure case:** The implementer regenerates one file, `method_contract`
  fails on the second, and the "read the diff" rule is applied to the wrong
  set. The Hub's card sample (`ui/src/cards/samples.json:55`) and the Hub card
  tests also hold receipt-shaped data; they type the fields as optional, so
  they pass, but the sample should show the new keys.
- **Fix:** Name both files in W6 and Step 4.4 (`git grep -l '"quote_author_did"'
  crates/roym_web/tests/method_shapes` then check which have a `role` and
  `terms` with `quote_record_id`). Add the two keys to `samples.json`'s
  receipt sample.
- *Status:* Fixed (7806e411)

#### P4 — The Hub can make a machine translation into the quote's own language, and then Accept is refused (minor)

- **Where:** D-B5-6 `targetFor(shownLang)`, D-B5-5 (Q4 (a) rule 2 and
  `ViewedMtOfOriginal`), D-B5-9.
- **Problem:** `targetFor` removes only the language of the version on screen.
  A quote has an original and author translations. If the person has the
  author translation on screen (original `hi`, translation `en`) and their
  first reader language that the engine supports is `hi`, `targetFor("en")`
  returns `hi`. The Hub offers "Translate into Hindi", and the result is an MT
  version with `lang = hi`. Accept then sends `viewed_lang: "hi"`,
  `viewed_via_mt: true`, and the node refuses it (`-32602`,
  `field: "viewed_via_mt"`). The person sees only a generic "Accept failed"
  and cannot tell why.
- **Failure case:** Any bilingual reader whose reader order is `["hi", "en"]`
  who switches the quote to the English author text and presses Translate. It
  needs an engine, so it cannot happen with v1, but it is exactly the state
  Q4 (a) says it guards against.
- **Fix:** `targetFor` for `recordText` must skip the language of every
  existing version of the record (original and author translations), not only
  the shown one: pass the list of version languages. The person already has
  the original one click away. Test: original `hi`, translation `en`, shown
  `en`, reader `["hi","en"]`, targets `["hi","en"]` gives no action.
- *Status:* Fixed (7806e411)

#### P5 — `targetFor` does not say which string it returns, and a reader tag like `en-US` would be sent as the target (minor)

- **Where:** D-B5-6 `targetFor`, `translateTexts`, D-B5-8, section 7.1
  (`translation.test.ts`).
- **Problem:** "The first of `readerLanguages()` that matches a target with
  `sameLanguage`" can mean the reader's tag or the target-list entry.
  `readerLanguages()` returns UI language plus `navigator.languages`
  (`en-US`, maybe non-canonical case). The engine lists `en`. If the reader's
  tag is returned, `target_lang` is `en-US` (an engine may answer
  `unsupported-pair`), and `viewed_lang` signed in the acceptance is `en-US`,
  a tag no author translation uses.
- **Failure case:** Reader `en-US`, targets `["en","hi"]`: the call sends
  `target_lang: "en-US"`; with a lowercase browser tag (`en-us`) `web` refuses
  it as non-canonical (`-32602`, `field: "target_lang"`).
- **Fix:** Return the matching entry of the target list (it is canonical by
  contract). Add the case to `translation.test.ts`: reader `["en-US"]`,
  targets `["en"]` gives `en`; reader `["en-us"]` gives `en`.
- *Status:* Fixed (7806e411)

#### P6 — `translation.rs` in `roym_web` names a crate that `roym_web` does not depend on (minor)

- **Where:** D-B5-3 (`syneroym_text_lang::LangTag::parse_canonical`).
- **Problem:** `crates/roym_web/Cargo.toml` depends on `syneroym-roym-core`
  but not on `syneroym-text-lang`. `roym_core::lang` re-exports `LangTag`
  (`lang.rs:5`).
- **Failure case:** The code does not build as written, or the implementer adds
  a dependency that `cargo shear` and the component size note do not expect.
- **Fix:** Write `syneroym_roym_core::lang::LangTag::parse_canonical` in the
  plan. No Cargo change.
- *Status:* Fixed (7806e411)

#### P7 — Where `loadTargets` runs is ambiguous, nothing tests that it runs, and it blocks the first draw (minor)

- **Where:** D-B5-6 ("`main.ts` awaits it after `whoami` and before
  `renderTabs` (`main.ts:110`), only for a signed-in person"), section 7.1,
  section 7.3.
- **Problem:** `main()` has two routes to `renderTabs`: directly (`:110`) and
  as the callback of `renderSetup` when an enrolment is pending (`:108` to
  `:112`). Putting the await "before `renderTabs` at `:110`" misses the setup
  route, so a person who finishes setup has no target list until a reload.
  There is no `main.test.ts`, and the unit tests only call `loadTargets`
  directly, so nothing tests the start-up order, or that an anonymous page
  makes no call. Also `loadTargets` is awaited before the tabs draw: one slow
  or hung `/rpc` delays the whole Hub, and `call` has no timeout in the plan.
- **Failure case:** With a later engine, a person who completes setup sees no
  translate action until they reload. A node that hangs on `/rpc` shows an
  empty Hub body.
- **Fix:** Call `loadTargets()` once, right after the session is known and
  before the `pending` check, and say so. Either draw the tabs without
  waiting (and let each hook point read the module at draw time, accepting
  that the first draw may miss the list), or wrap the call in a short
  timeout that gives `[]`. Add a unit test that drives that start-up code
  (extract it into a function if `main()` is not testable): signed-in calls
  `translation.targets` once, anonymous calls nothing.
- *Status:* Fixed (0cc3f59c)

#### P8 — The three-version case of `recordText` (original, author translation, machine translation) has no defined switch behaviour (minor)

- **Where:** D-B5-7 (click behaviour), `record_text.ts:57` to `:89`
  (`initialIdx`, `alternateIdx`, `currentIdx`), section 7.1.
- **Problem:** Today the switch button toggles between `initialIdx` and
  `alternateIdx` (original or the best version). After an MT version is
  appended there are three versions. The plan says the switch "reads
  `record-text-show-original`" and that "a second click on Translate does not
  call again (the MT version is kept)", but not what the switch does from the
  MT version, how the person goes back to the MT from the original, or what
  `data-shown-via` is after each step. These states feed the accept request.
- **Failure case:** Original `hi`, author translation `en` shown first, MT
  into `ta` added: switch goes to the original; the next switch goes back to
  `initialIdx` (the author translation), and the MT can only be shown by
  pressing Translate again. A different implementer may toggle the other way.
  Either way `data-shown-via` can be stale if it is not written on every
  state change.
- **Fix:** Add a short table to D-B5-7: for each button (switch, translate)
  and each current version, the new shown version and the attribute values.
  Add one unit test that walks original, author translation, MT, original and
  asserts both attributes at each step.
- *Status:* Fixed (0cc3f59c)

#### P9 — The chat hook offers Translate on a person's own messages and on a message tagged in the reader's language as its second tag (minor)

- **Where:** D-B5-7 (`messageText` bullet), section 7.1 (`message_text.test.ts`).
- **Problem:** The two callers pass `translatable: true` for "a non-deleted
  UTF-8 message", with no check of direction. Outgoing messages are the
  person's own text. And `targetFor(messageLang(languages))` looks only at the
  first tag (`messageLang` returns `langAttr(languages[0])`), but a message may
  carry several tags (B3b). A message tagged `["hi","en"]` read by an English
  reader gets a Translate button although English is one of its languages.
- **Failure case:** With an engine, Bob sees "Translate into English" under
  his own outgoing English message, and under a bilingual message that
  already has an English tag.
- **Fix:** Pass `translatable` only for incoming messages (direction
  `incoming`, and for groups an author that is not the reader), and make the
  check skip when any tag in `languages` matches a reader language. Add both
  cases to `message_text.test.ts` and the callers' tests. If the owner wants
  translate on own messages, say so in the plan.
- *Status:* Fixed (7806e411)

#### P10 — Machine translation of a quote's `dispute_path` can turn a link into different text, and the card draws it as a link (minor)

- **Where:** D-B5-7 (the three callers pass `texts`; quote `[scope,
  cancellation_terms, refund_terms, dispute_path]`), `quote.ts:239` to `:247`
  (`isLink` check on `startsWith("http")`, `renderLink`).
- **Problem:** `dispute_path` may be a URL. It is sent to the engine as
  text. A translation may change it, or a remote engine may return a different
  string that still starts with `https://`. The quote card then draws it with
  `renderLink`, in a version labelled as a translation of terms the person is
  about to accept. The binding original has the real link.
- **Failure case:** With an engine, the machine version of a quote shows a
  `dispute_path` link that is not the one the author signed. The ML-19 warning
  says only that the original binds.
- **Fix:** Do not send a field to the engine when it is a link (starts with
  `http://` or `https://`): `texts.get` omits it and `texts.set` keeps the
  original value. Or draw it as plain text in a machine version. Add a unit
  test with a link `dispute_path` (the exact `texts` array sent has no link).
- *Status:* Fixed (7806e411)

#### P11 — ML-26 says each hook point calls the interface and gets `unavailable`; the plan's v1 hook points call only `translation.targets`, and the deviation list does not say so (minor)

- **Where:** Section 3 (ML-26 row and "Where decisions and deviations are
  written"), change doc ML-26 and "Translation hook points", D-B5-6.
- **Problem:** ML-26 reads "Each hook point … calls the translation
  interface. With the v1 implementation, each call returns `unavailable`, and
  the flow continues with the original text." In the plan, with v1 no hook
  point calls `translate` at all. Each reads the list that `translation.targets`
  returned once. That is a good design (it satisfies ML-16 and ML-27, and it
  avoids a call per card), but it is a different reading of ML-26, and the list
  of deviations to record at close-out (section 3) does not include it. The
  `unavailable` case is covered only by a stub that returns `-32020`, which
  v1 never sends to a hook point.
- **Failure case:** A later reader checks ML-26 against the code, sees no
  hook point calling `translate` with v1, and cannot find the decision.
- **Fix:** Add a deviation to section 3: "Hook points ask `supported-targets`
  once through `translation.targets`; with an empty list they make no
  `translate` call. The `unavailable` answer is tested with a stub engine that
  answers it." Reword the ML-26 test-column text the same way.
- *Status:* Fixed (7806e411)

## Plan review round 2 (check of the round 1 fixes) — Claude Sonnet 5.5, 2026-11-13

Checked on `feat/m08b-b5-translation` at `ad7f6fb7` (the fix commits
`7806e411` and `ad7f6fb7`, on top of `5b36bca9`), in the main checkout.
`git pull` said "Already up to date". I read the full diff of the plan, the
review file and `status.md` since my round 1, and I checked the new claims
against the code on `main`. I ran no `cargo`, `npm` or `mise` step and changed
no code.

**Summary:** 0 blocker, 0 major, 1 new minor (P12). Of the 11 round 1 items,
9 are confirmed Fixed (P1 to P6, P9 to P11) and 2 go back to Open with a note
(P7, P8). **3 open: P7, P8, P12.** All three Fixed (0cc3f59c).

### Round 1 items checked

- **P1 (Fixed, confirmed).** D-B5-7 now says `recordText` writes
  `data-shown-lang` and `data-shown-via="original"` in the one-version
  early-return path (`record_text.ts:43` to `:51`) with no meta box. D-B5-8
  no longer asserts: with no element it sends the card's `data.terms.lang` and
  `false`. Section 7.1 has both tests (one version; bare host with no drawn
  card).
- **P2 (Fixed, confirmed, with P12 for the detail).** The Playwright
  accept test is gone (section 7.3, Step 6, ML-18 row). The proof moves to
  `roym_transaction_e2e` (it has two nodes, `quote.set` accepts
  `translations` (`quote_ops.rs:94`), and the file is 445 lines, not in the
  oversized list). How to write it is still thin: see P12.
- **P3 (Fixed, confirmed).** W6 and Step 4.4 now name
  `agreement_verify_good.json`, `transaction_thread.json` and
  `transaction_export.json`. I checked `transaction_export.json`: it holds two
  `agreement-receipt` card payloads (`:136` to `:143`, `:181` to `:187`), so
  naming it is right. The `git grep` recipe and the `samples.json:55` change
  are in.
- **P4 (Fixed, confirmed).** `targetFor(skipLangs)` skips every language of
  the record, and the test `skip ["hi","en"] … → null` is in section 7.1. Its
  interaction with machine versions is the cause of the P8 note.
- **P5 (Fixed, confirmed).** `targetFor` returns the target-list entry, and
  the `en-US` and `en-us` cases are in section 7.1.
- **P6 (Fixed, confirmed).** `syneroym_roym_core::lang::LangTag::parse_canonical`,
  no Cargo change.
- **P7 (back to Open).** See the note below.
- **P8 (back to Open).** See the note below.
- **P9 (Fixed, confirmed).** `translatable` is for incoming messages only
  (direct chat `direction`, group author not the reader), and
  `targetFor(languages)` takes all tags. Both cases are in section 7.1.
  `groups.ts` has `msg.direction` (line 108 area), so the group check can be
  written.
- **P10 (Fixed, confirmed).** A link-valued field is left out of the array and
  its original value is kept by `set`; the test asserts the array has no link.
  The implementer must keep the position mapping between `get` and `set` when
  an entry is left out; the exact-array test covers it.
- **P11 (Fixed, confirmed).** The ML-26 row and the deviation text in section 3
  say hook points ask `translation.targets` once and make no `translate` call
  with v1.

**Owner answers.** The plan now records "Owner answer: (a), 2026-11-13" for
Q1 to Q7, and each equals the view I gave in round 1. I cannot check that the
owner gave these answers; the owner should confirm them when they read the
plan.

### Notes on items sent back to Open

- **P7.** The fix adds `startTranslation(signedIn)`, called once before the
  `pending` check, a 3-second timeout that gives `[]`, and tests. What is
  missing is one sentence: does `main.ts` `await` it? The text says the
  timeout means a hanging node "cannot delay the first draw". With `await`
  the delay is at most 3 seconds, not zero. Without `await` the tabs can draw
  before the list arrives, and every hook point that has already drawn misses
  the translate action until the next draw. *Fix:* say that `main.ts` awaits
  `startTranslation` (v1 answers at once, so the cost is nothing; the worst
  case is 3 seconds), and change "cannot delay" to "delays by at most 3
  seconds".
- **P8.** The table assumes one machine version `M`, and a row
  "Translate | `M` exists | `M` (no call)". But D-B5-6 says `skipLangs`
  includes "machine versions already added", so after the first translation
  into `en`, `targetFor` no longer returns `en`. Then the Translate button
  either disappears, or, when the reader has a second supported language
  (`["en","ta"]`), it offers "Translate into Tamil" and a second call adds a
  second machine version. The table has no row for that, `data-shown-via` is
  the same for both, and the test "a second click on Translate makes no second
  call" cannot be written as it stands. *Fix:* decide one rule and write it
  in D-B5-7. The simplest: at most one machine version per record; once it
  exists the Translate button is not drawn, and the switch button is the only
  way back to it (the table's last row already does that). Remove the row
  "Translate | `M` exists", and change the test to "after one translation
  there is no Translate button, and the switch returns to `M`".

### New findings

#### P12 — The two-node e2e proof for ML-18 is described as an "extension" of one long test, and the existing test has an assertion that the new case breaks (minor)

- **Where:** Section 7.2 "Two nodes", Step 4.5, `crates/substrate/tests/roym_transaction_e2e.rs`
  (`an_offer_is_agreed_across_two_installations`, `:43` on, with
  `#[expect(clippy::too_many_lines)]`), `common/roym_flow.rs` (`send_quote`
  `:183`, `accept_quote` `:209`).
- **Problem:** The existing test is one function of about 400 lines. It has
  no author translation, no `viewed_lang`, and (Step 10) asserts that the two
  halves' payloads are equal after the role is replaced
  (`assert_eq!(c_payload, p_payload)`). With the plan's case (Bob accepts the
  translation tag, the author counter-signs `terms.lang`) the halves differ,
  as AG-32 allows, and that assertion fails. The shared helpers `send_quote`
  and `accept_quote` take no `translations` and no `viewed_*`, and both need a
  listing and a slot.
- **Failure case:** The implementer adds the case inside the existing test and
  breaks its equality check, or copies about 150 lines of set-up into a second
  test, which `cargo dupes` flags.
- **Fix:** Say in section 7.2 that the new case is a separate
  `#[tokio::test]` (the existing one stays as it is and keeps asserting equal
  payloads with the defaults of Q3 (a)). Build it on the `common/roym_flow.rs`
  helpers: add `translations` to a new variant of `send_quote` (or an optional
  argument) and `viewed_lang`/`viewed_via_mt` to a new variant of
  `accept_quote`; do not copy the set-up. Say that the test asserts the two
  halves differ in `viewed_lang` and that the pair is still `complete`.
- *Status:* Fixed (0cc3f59c)

## Code review round 1 — Claude Sonnet 5.5, 2026-11-13

Reviewed `feat/m08b-b5-translation` at `1327538c` (17 commits on top of
`origin/main`), in the main checkout (not a worktree: `.git` is a
directory). `git pull` said "Already up to date".

**Who reviewed.** The prompt names Claude Opus 5.5 as the reviewer. This
session runs on Claude Sonnet 5.5, which is also the tool that implemented B5
(`status.md`). I write the tool I really am. So this round does not meet the
cross-tool rule of [session-strategy.md](../../session-strategy.md) section 6.
The owner should decide: accept this round, or run round 2 with another tool.

**Change docs.** The prompt lists one change doc,
[roym-multilingual](../../changes/roym-multilingual/change.md). Every B5
decision and deviation belongs there, and it holds them (see below).

**Summary:** 0 blocker, 0 major, 6 minor, 6 items (R1 to R6). **6 open.**
No earlier code review round exists, so there are no Fixed or Rejected items
to re-check. The plan review items P1 to P12 are all Fixed; I confirmed in the
code that the fixes are in (P1 `shownVersion` fallback, P4 `targetFor(skip)`,
P5 target-list entry returned, P6 `roym_core::lang::LangTag`, P7
`startTranslation` awaited before the `pending` check, P8 one machine
version, P9 `translatable` for incoming messages only, P10 `textFields`
leaves links out, P12 separate two-node test on shared helpers).

Limits of this review: CI for the PR (`gh pr checks 362`) is fully green
(lints, 6 test shards, Playwright E2E, CLA). I did not run the full suite or
`mise run verify`. I ran only the Hub unit tests
(`npm --prefix crates/roym_web/ui test`: 515 passed in 43 files, as
`status.md` says). I read the built wasm sizes in `target/wasm32-wasip2/release/`;
they equal the numbers in `task.md` and the PR (web 310,033, transaction
1,516,528, and so on). I changed no code.

### What was checked and found correct

- **The code does what the plan says, and no more.** D-B5-1 to D-B5-11 against
  the diff: the WIT is the plan's text; `AppTranslation` is on all three
  implementors and no other (`git grep 'impl AppTranslation for'`: `GuestHost`,
  `NativeAppHost`, `TestHost`); the seven `wit/deps/translation/translation.wit`
  are symlinks (mode `120000`); the same seven guests that import `events`
  import `translation` (diff of the two file lists is empty).
- **One no-op, two builds.** `NativeAppHost` calls the same three `HostState`
  functions as the WASM host, with no lock, so the builds cannot drift. The
  host-side `bindgen!` is separate (`translation_host.rs`) and nothing in the
  runtime, factory, `SubstrateConfig` or manifest changed.
- **Router and access control.** `WEB_ROUTES` has the prefix `translation.`
  with `Owner`; `method_auth` reads `PUBLIC_METHODS`, `ROUTES`, then
  `WEB_ROUTES`; `admit` runs before `answered_by_web` in `rpc`, so a missing
  session gets `-32010` for `translation.targets`, `translation.translate`
  and an unknown `translation.x`. On the internal `invoke` path `admit(.., None)`
  gives `NoSession` (`-32010`) for an `Owner` method, as ADR-0029 says. No
  `ROUTES` prefix overlaps (`web_routes_do_not_overlap_sibling_routes`).
- **`translation.translate` input checks** run in the plan's order
  (count, each text, total, `target_lang`, `source_lang`), each `-32602` with
  `data.field`; limits equal the change doc and developer guide (8, 8,192,
  32,768). The engine's own text goes into the message, never into `data`.
- **Acceptance.** `AgreementReceiptPayload` has the two required fields after
  `terms`; `validate` refuses `viewed_via_mt` with `viewed_lang == terms.lang`;
  `verify_agreement_receipt` calls `validate`, so the receive path
  (`sync.rs`), `agreement.verify` and both backup paths (`backup.rs:449`,
  `:638`) all use it; `halves_agree` is unchanged. `viewed()` runs after
  `load_verified_quote_and_role` and before the agreement row is created (the
  deviation is written). The countersignature signs `terms.lang` and `false`.
  `agreement.get` keeps `QuoteRecordInput`. Every `AgreementReceiptPayload {`
  literal in code and tests has the two fields; the three hand-signed parity
  payloads and four golden shapes are updated.
- **Hub.** `startTranslation` is awaited once, before the `pending` check.
  `recordText` writes `data-shown-lang` and `data-shown-via` in the
  one-version path too. `shownVersion` falls back to the card's `terms.lang`
  and `false`. The warning, confirm and cancel work as D-B5-9 says. The search
  box, chat hook (incoming text messages only, all tags skipped) and group
  hook match D-B5-7 and D-B5-10. `quote.ts` and `textFields` use the same
  link rule (`http://` or `https://`). Nine new messages exist in both
  catalogs and the Hindi sweep draws them.
- **Change doc and plan agree.** The decision numbers cited in the change doc
  (D-B5-1/2 interface and wiring, D-B5-3/4 `web` methods and routing, D-B5-5
  acceptance, D-B5-6/7 Hub, D-B5-8/9 accept and warning) are the plan's. The
  new deviations (hook points ask once, search hook in the Hub, no
  `detect-language` method, every component imports it, optional `viewed_*`,
  `web` size, `readerLanguages` moved, check before the row, `viewed_lang`
  sent only when known, tests in new files) are all in `## Deviations`, and
  the old "binds line" deviation is rewritten. Every test name in the ML-15 to
  ML-19, ML-26 and ML-27 rows exists (`git grep 'fn <name>'` found each once).
- **AGENTS.md rules.** No planning id, line count or rule number in a code
  comment (grep of the added lines). No new dependency. Files are under the
  size limits (`fixtures.rs` 799, `agreement_ops.rs` 565, `transaction.rs`
  715); the new Rust files are small. Backlog has the six new rows and two
  changed rows of plan section 8, with sources that exist. The traceability
  row's links all resolve. The ADR is Accepted and states the no-cost,
  no-rate-limit rule.
- **Both builds.** The parity tests compare whole answers: scenario 236
  (methods), 237 (accept) and the app-host test.

### Findings

#### R1 — `system-architecture.md` still says `web` forwards every method and lists only `session.whoami` (minor)

- **Where:** `docs/system-architecture.md:1147` (the `web` row of the Roym
  services table) and `:1156` ("`web` lists only its own `session.whoami` and
  forwards every other method").
- **Problem:** After B5, `web` answers `translation.targets` and
  `translation.translate` itself, and its method table has three rows
  (`api.rs`). The developer guide was updated, the architecture doc was not.
  `:1147` says "It forwards each method" and gives no mention of the two
  methods; `:1156` is false. The close-out rule asks for living docs that are
  true after the change.
- **Failure case:** A developer reads the architecture doc, adds a method
  prefix and expects `web` to forward it, or looks for `translation.*` in a
  sibling service and does not find it.
- **Fix:** In `:1147` add "It answers `session.whoami`, `translation.targets`
  and `translation.translate` itself (the last two need the node owner)". Change
  `:1156` to "`web` lists its own three methods (`session.whoami`,
  `translation.targets`, `translation.translate`) and forwards every other
  method ...". `:1460` is still true; leave it.
- *Status:* Open

#### R2 — After a machine version is made, an author translation can no longer be reached with the switch button (minor)

- **Where:** `crates/roym_web/ui/src/records/record_text.ts:101` (`lastOther`),
  `:157` to `:160` (switch), `:185` (translate sets `lastOther = machineIdx`);
  the D-B5-7 table row "Switch | `O` | the last non-original version shown".
- **Problem:** The switch toggles only between the original and `lastOther`.
  Making the machine version sets `lastOther` to it. So author translation `A`
  is never reachable again in that card. Trace: versions `[O hi, A ta]`, reader
  reads `en`, engine target `en`. First draw shows `O` and the switch offers
  `A`. Press Translate: `M` is shown and `lastOther = 2`. Switch gives `O`,
  switch again gives `M`. `A` (index 1) is never shown.
  A worse case: reader UI `hi`, `O ta`, `A hi`, target `en`. The first draw
  shows `A` (the version the reader reads best). After Translate it cannot
  come back to `A` without a reload.
- **Failure case:** With a later engine, a bilingual reader who presses
  Translate loses the author's own translation that they could read. It also
  changes `data-shown-via` for a later Accept in a way the person cannot undo.
- **Fix:** Make the switch cycle through all versions in order (`O`, `A`, `M`),
  or keep `A` reachable by letting the switch from `M` go to the best version
  (`initialIdx`) when it is not `0`. Change the D-B5-7 table and the walk test
  to include `A` after `M`. Alternative: do not offer Translate when the best
  version is already in a language the reader reads.
- *Status:* Open

#### R3 — The Playwright test "no translate action with a v1 node" proves less than its name for the hit and the chat message (minor)

- **Where:** `crates/substrate/tests/e2e/tests/roym-hub.spec.ts:1423` to
  `:1451`; the ML-16 row of the change doc.
- **Problem:** The only line that depends on the node having no engine is
  `translation.targets` equal to `{"targets": []}` and the two search-box
  counts (`.search-translate`, `.search-translate-target`). The message
  checked is one the test sends (`direction` outgoing, to a peer that never
  answers), and an outgoing message never gets a translate button, with or
  without an engine. The listing hit has one version in the Hub's own language
  in this set-up, so it also gets no action with an engine. These two
  `toHaveCount(0)` checks would pass if the engine existed.
- **Failure case:** A change that draws a translate button for the wrong
  messages with an engine would pass this test; the ML-16 row lists the test
  as evidence for "no translate action anywhere".
- **Fix:** Either drop the chat and hit assertions and say in the test comment
  that the unit tests cover them, or make them real: stub `/rpc` with
  `page.route` so `translation.targets` answers `["hi"]` for one case and check
  that a Hindi listing hit gets `.record-text-translate` (and with `[]` it does
  not). Reword the ML-16 row to name only what the test proves.
- *Status:* Open

#### R4 — The `web` error mapping and the multi-text loop have no test for the three refusals v1 cannot produce (minor)

- **Where:** `crates/roym_web/src/app/translation.rs:88` to `:120` (`translate`)
  and `:154` (`translation_error`); the change doc and the developer guide both
  list `unsupported-pair`, `too-long` and `engine-failed` with their `data`.
- **Problem:** With the v1 host only `unavailable` can happen, so parity test
  236 covers one of four arms. `unsupported-pair` (`data.source`, `data.target`),
  `too-long` (`data.limit`), `engine-failed` (message with the engine text, `data`
  with only `kind`) and the loop (one call per text, in order, stop at the first
  error) are never run. `translation_error` is a pure function and the loop
  needs only a host with an engine, which `TestHost` can be made to be.
- **Failure case:** A wrong `data` key name or a swapped source and target is
  found only when M9A adds an engine, after the Hub (which reads `data.kind`
  through `errText`) is built on it. The documented wire shape has no test.
- **Fix:** Add a unit test in `translation.rs` (or beside it) for the four arms
  of `translation_error` (exact `code`, `message`, `data`), and one for the loop
  with a host whose `translate` answers by text (assert order, and that an error
  on the second of three texts stops the loop and returns that error).
- *Status:* Open

#### R5 — Two behaviours the PR text and ADR state have no test: a repeated `agreement.accept` with bad `viewed_*`, and `translation.*` on the internal `invoke` path (minor)

- **Where:** `crates/roym_transaction/src/app/agreement_ops.rs:124` (`viewed`
  runs before `already_accepted_response` at `:135`); `crates/roym_web/src/app.rs:295`
  to `:297` (`invoke` refuses an `Owner` method with `-32010`); ADR-0029
  decision 9 ("on the HTTP path only"); PR text, "agreement.accept now refuses
  bad `viewed_*` values even on a repeated call".
- **Problem:** Scenario 237 starts every refusal case on a quote that is not
  yet accepted ("these are refused before anything is signed, so one quote
  serves all"). No test accepts a quote and then repeats the call with bad
  values. No test calls `web`'s `invoke` with `translation.targets`. Both are
  stated as behaviour (and the first is called out as a new refusal), but both
  hold only because of the order of two statements.
- **Failure case:** Someone moves `viewed()` below `already_accepted_response`
  (it looks like a cheap tidy-up) and a bad repeat is silently answered as
  "already accepted"; nothing fails. The same for a change that routes
  `translation.*` in `invoke`.
- **Fix:** In scenario 237 (or a new small parity case) accept once, then repeat
  with `viewed_lang: "ta"` and assert `-32602` `data.field = "viewed_lang"`. Add
  a unit test in `roym_web` for `invoke` with `translation.targets` on a
  `TestHost` (expect `-32010`).
- *Status:* Open

#### R6 — `status.md` does not say that the owner review of the nine new Hindi messages is still open, and `hi.ftl` still says "owner review: done 2026-11-07" (minor)

- **Where:** `docs/planning/milestones/M08B-language-and-structure/status.md`
  (B5 row), `crates/roym_web/ui/src/i18n/hi.ftl:7`, plan section 7.7 and the PR
  text ("Hindi text for owner review").
- **Problem:** The PR text asks for the owner's read of nine messages, two of
  them with legal weight (`messages-accept-mt-warning`,
  `messages-accept-mt-confirm`). The status row, which is the record the next
  session reads, says only "owner answers Q1–Q7 recorded in the plan, owner to
  confirm". The Hindi review is not listed as open there, and the file header
  still reads as if the whole file was reviewed.
- **Failure case:** The slice close step merges with the Hindi text not
  reviewed, because the only place that says so is the PR text.
- **Fix:** Add to the B5 row: "owner review of nine Hindi messages (two with
  legal weight) open". At close-out, after the owner has read them, put the
  date in the status row (as B3a did), not in the `hi.ftl` header.
- *Status:* Open
