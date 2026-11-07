# M08B slice B1b — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-b1b-implementation-plan.md](slice-b1b-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-06

Checked on `feat/m08b-b1b-localization-screens` at `7dc2f6df`, in the main
checkout (not a worktree). `git pull` first failed in the sandbox (GitHub was
not allowed). With GitHub allowed it said "Already up to date". The commit
after the plan's base (`1a62c26a`) is the plan itself, so the code facts
below hold.

**Who reviewed.** This session runs on Claude Sonnet 5.5. The `status.md`
row says the plan was written by Claude Opus 5.5. They are different models,
as the prompt says.

**Change docs.** The prompt lists one change doc,
[roym-multilingual](../../changes/roym-multilingual/change.md). `task.md`
also names [demo-stories](../../changes/demo-stories/change.md), in its
"Demo" paragraph (story 40). The plan already says this (section 0). I agree:
B1b meets no criterion of that doc and records nothing there. ML-1 and ML-6
belong to roym-multilingual, so every B1b decision and deviation belongs
there too (plan section 3 says the same).

**Summary:** 0 blocker, 2 major, 7 minor, 9 items. All 9 Fixed in the plan; 0 open.

Limits of this review: I read the plan, the docs, the code and the e2e
specs. To test the check's rules I wrote a throwaway parser script in
`$TMPDIR` (not committed) that follows rule 1 and rule 2 of D-B1b-2 over
`crates/roym_web/ui/src` on this commit. I did not run any plan step, the
Hub tests or `mise run verify`.

### What was checked and found correct

- **File and line links.** The links of section 2.2 and Q2 to Q4 point at the
  right code: `messages.ts:17`, `:22`, `:26`, `:31`, `:35`, `:343`, `:370`,
  `:478`, `:489`, `:551`, `:763`, `:768`, `:773`; `transaction.rs:505`;
  `directory.ts:227`; `profile.ts:16`, `:23`; `groups.ts:496`; `backup.ts:7`;
  `hi.ftl:33`, `:34`; `transaction/tests.rs:779`; `membership/tests.rs:561`;
  the doc comments in `transaction.rs`, `membership.rs:26` and `:603`.
  `setup.ts:17` is the start of the `text(...)` call; the command is on
  `:19`. The living-doc lines also match (`system-requirements-spec.md:1462`
  and `:1464`, `system-architecture.md:1414`, `traceability-matrix.md:68`,
  `developer-guide.md:1745`, `demo.md:1260`, `deferred-backlog.md:508`).
- **Counts I could re-check.** `en.ftl` and `hi.ftl` both have 159 messages;
  `en.ftl` is 10,027 bytes. The 11 `ListingInputError` throws
  (`editor.ts:121`, `:134`, `:141`, `:186`, `:191`, `:195`, `:238`, `:255`,
  `:289`, `:292`, `:296`) and the 6 `login.ts` errors (`:147`, `:157`,
  `:234`, `:238`, `:268`, `:300`) are exact. `rpc.ts:69` is the only
  Hub-made English detail in `rpc.ts`. I did not re-count the 333 and 150
  figures of point 1; my own parse of the assignment and object-key sinks
  (`textContent`, `placeholder`, `label:` and so on) found 108 literal hits,
  which fits their split into literals and templates.
- **Rust tests that read Hub files.** A search for `ui/src` in `crates/**/*.rs`
  finds exactly the ones the plan lists: `money.rs`, `router.rs`, `card.rs`
  (constants, not text), the two B1a tests that already read `en.ftl`, and the
  two B1b changes. `crates/substrate`, `crates/roym_web/tests`, `docs/demo`,
  `xtask` and `apps` read no Hub source file (only `hub-helpers.ts` reads
  `.ftl` files).
- **The test hook.** `window.RoymMessages` is read by `roym-hub.spec.ts` only
  at `:1060`, `:1156` and `:1193`, and only for the three functions. The three
  notice fields have no reader.
- **Signed text and limits.** `MAX_NOTICE_LEN` and `MAX_TERMS_TEXT_LEN` are 2,048
  and are checked with `.len()` (bytes), as the plan's tests assume. The
  service compares the notice with nothing else. `request_ops.rs:99` stores it
  as given. The four Rust-pinned English notices contain no `{`, `}` or special
  first character.
- **e2e steps.** `selectOption('draft')` and `selectOption('harassment')` use
  option values, which stay the code. No e2e step reads a Hub placeholder,
  `title` or `aria-label`. The `hi-IN` describe only reads `html[lang]` and
  the login title. Story 40 can use `hubMessage('hi', …)`, which reads one-line
  messages.
- **The build.** `mise run build:roym-ui` is the only caller of `npm run
  build` (`mise.toml:50`), and it runs `npm ci || npm install` first, so
  `vitest` is there. The `hub-ui` gate is in `xtask/src/verify.rs:102`. The
  file-length gate looks at `*.rs` only, so `messages.ts` at 939 lines is
  no problem (point 17 is right).
- **No CSS text.** `style.css` has no `content:` rule, so no visible text hides
  in the style sheet.
- **Order of steps.** Step 3 adds `input()` and `checkbox()` to `dom.ts` while
  `listings.ts` still has its own copies until Step 6. That builds.
- **Writing for readers.** The reader is named, the "How this is shown to an
  end user" section has 5 bullets and then the developer bullets, and no code
  comment named in the plan cites a plan id.

### Views on the owner's questions

I do not answer for the owner.

- **Q1. (a), a vitest file that `npm run build` runs.** I agree. One parser, one
  implementation, tested like the rest, and the `hub-ui` gate runs it too. Two
  notes: the test runs twice in a full `verify` (once in the build, once in the
  gate), about 2 seconds each, which is fine; and the build output is vitest
  output, so P1 and P2 matter more, because a false report will block every
  Hub developer.
- **Q2. (a), sign the text that was shown.** I agree. A person must sign what
  they read, and a Hindi notice with an English signed copy would contradict
  the change doc's rule 1. The service checks only length, and the English
  constant stays for `roymctl` and the Rust tests. B3a must then tag the record
  with the Hindi language, so keep the backlog link to B3a in the change doc.
- **Q3. (a), defaults from the catalog.** I agree, for the same reason. The
  person can edit the fields.
- **Q4. (a), one label per code.** I agree. It keeps English and every e2e
  step the same. The inventory of codes is longer than the plan says (P3), and
  the screen-level check of it is weak (P2).
- **Q5. (a), the Hindi sweep.** I agree that it is needed: it is the only
  check that draws a screen. It needs two changes first (P2).
- **Q6. (a), a side-by-side review file.** I agree. A suggestion for the file:
  add a column that marks the messages with legal or safety weight (the two
  delete notes, the address notice, the data-use notice, the decline note, the
  two membership notices, the group delivery and join notices, the quote and
  agreement words). A wrong Hindi word there changes what a person agrees to,
  so the owner should read those rows first and slowly. The B1a messages are
  also still `owner review: pending`, so the table covers all 560 rows.
- **Q7. (a), one more step in story 40.** I agree. It is small, it reads the
  Hindi message from the file, and it is the only story that shows B1b.

### Findings

#### P1 (major) — Rule 2 reports 47 CSS class strings, and the plan allows no exception

- **Where:** D-B1b-2, rule 2 and "No exceptions in the code"; Step 2 item 2
  (`NOT_MOVED_YET`); Step 11; the helper `text(tag, value, className)` in
  `dom.ts:5`.
- **Problem:** Rule 2 reports any literal with two letters and white space
  between them. It exempts `className`, `rel`, `classList.*` and the selector
  methods. It does not exempt the third argument of `text()`, which is a class
  list. I ran rule 2 over `src/` (without tests) on this commit. It reports 431
  literals. 47 of them are class strings or the head of a class template, for
  example `"button bundle-export"` (`backup.ts:82`), `"button check-membership"`
  (`directory.ts:44`), `"button-small retry-button"` (`groups.ts:124`),
  `` `message-status status-${…}` `` (`groups.ts:122`), `` `listing-status
  status-${…}` `` (`listings.ts:141`), `"message-body deleted"`
  (`messages.ts:543`) and `` `message-state state-${…}` `` (`messages.ts:551`).
  They are in `backup`, `directory`, `groups`, `listings`, `memberships`,
  `messages`, `safety`, `synorg` and `synorg_members`. None of them is text.
- **Failure case:** At Step 11 `NOT_MOVED_YET` is empty and the check still
  reports 47 class strings. The plan has no way to clear them: there is no
  marker (by design), and moving them into `t()` would be wrong (a class name
  is not a message). The implementer either adds an ad hoc exception, which
  D-B1b-2 forbids, or changes the CSS class helper calls in 47 places.
  Either way the step stops, or the rule gets a quiet hole that nobody reviewed.
- **Fix:** Change rule 2 in D-B1b-2 and add a test case. Exempt (a) the third
  argument of `text()` (the class list) and any literal or template part in it,
  and (b) the same for `el.className = …` with a template (already there).
  Prefer this to a "looks like a class" pattern: a rule by shape would also
  hide a lower-case English label such as `"pack size"` or `"tax included"`
  (`listings.ts:289`, `:273`). Add to the cases of section 7.1:
  `text("p", t("x"), "button bundle-export")` and
  ``text("span", v, `state-${s} on`)`` are not reported, and
  `text("p", "two words")` is.
- *Status:* Fixed (db01fa46). Now in: D-B1b-2 rule 2 (third argument of `text()` exempt by position, not by shape); section 7.1 (class-list cases).

#### P2 (major) — The Hindi sweep removes the fixtures first, so it cannot see a raw code, and it allows every Latin word in `hi.ftl`

- **Where:** D-B1b-12 ("Allowed"); Q5; Q4 (a); section 7.1 ("Code tables");
  exit criterion 3.
- **Problem:** The sweep strips "each fixture string" before it looks for Latin
  words. Many fixture strings are the codes the screens must translate: a
  message with `state: "delivered"`, a listing with `status: "active"`, a
  report with `category: "fraud"` and `status: "open"`. If a screen shows the
  code raw (`messages.ts:551` shows `m.state` as it came, `safety.ts:135`
  builds `${r.category} - ${r.subject_kind} … [${r.status}]` from fixture
  values, `listings.ts:141` shows `row.status`), the raw word is removed with
  the fixture and the test passes. The static check cannot see these either,
  because the fixed parts of such a template have no letters
  (`" - "`, `" ["`). The per-table unit tests (section 7.1, "Code tables") only
  show that the table gives a Hindi word. They do not show that the screen uses
  the table. So no test in the plan fails when a screen forgets a table, which
  is the failure Q4 exists to prevent.
  Second, the allowed Latin words are "the Latin words that `hi.ftl` itself
  contains", read from the file. `hi.ftl:33` and `:34` today contain `"Search"`,
  so the English word `Search` is allowed on every screen. A pasted English
  `Save` in any Hindi message would allow `Save` everywhere. The
  "every message has a Devanagari letter" test does not help: a message like
  `Save करें` passes and still adds `Save` to the allowed words.
  Third, the plan does not say what happens to Latin text that the `Intl`
  formats and the fake `/rpc` produce: the time zone name in `formatDateTime`
  (`UTC` under `TZ=UTC`), the unknown-currency fallback `<amount> <code>`, and
  the English `detail` of an error from the fake `/rpc`.
- **Failure case:** The implementer moves the visible words of the report list
  but keeps `${r.status}` raw. The sweep removes the fixture `open`, finds no
  Latin word, and passes. A Hindi person reads `fraud - person did:key:… [open]`
  between Hindi labels. The same happens for the message state and the listing
  status, and no test fails.
- **Fix:** In D-B1b-12: (1) remove only the data that a person types or another
  node sends (names, titles, bodies, DIDs, ids), and mark each of those fixture
  values with a prefix or a Latin marker that the test knows (for example the
  value `Zq-title`). Use the real code values (`delivered`, `active`, `fraud`,
  `open`, …) in the fixtures and do not remove them, so a raw code fails the
  sweep. (2) Replace "the Latin words of `hi.ftl`" by a short fixed list in the
  test file (`Roym`, `SynOrg`, `DID`, `roymctl`, and the words of the commands
  inside messages), each with a comment, plus the zone name, the currency code
  and the `detail` text of the fake `/rpc` named as separate allowed items. A
  new Latin word then needs an edit in the test, which a reviewer sees.
  (3) Add the code fixtures to the sweep of `renderMessages`, `renderListings`
  and `renderSafety`, and the credential-status line of `renderSynOrg`
  (P3).
- *Status:* Fixed (db01fa46). Now in: D-B1b-12 (`Zq` marker for data, real codes in fixtures, fixed allowed list in the test, `UTC`/currency/detail named); section 7.1 (raw-code self-check); exit criterion 3; Q4 and Q5 owner answers.

#### P3 (minor) — The list of code tables in Q4 and D-B1b-6 is shorter than what the screens show

- **Where:** Q4; D-B1b-6; Step 5 (`safety.ts`), Step 6, Step 10.
- **Problem:** Q4 lists the message state, the listing editor options, the
  listing block names and the report categories. The code finds more places
  where a code or a Hub-made English word is drawn as text:
  - `safety.ts:104-111`: the subject kind options (`person`, `listing`,
    `message`), with the same word as `option.value` and as its text.
  - `safety.ts:135`: the report row shows `r.category`, `r.subject_kind` and
    `r.status` (`open`, `withdrawn`) in one template.
  - `listings.ts:141`: `row.status` (`active`, `draft`, `withdrawn`) in the
    listing list.
  - `synorg_members.ts:244` to `:257`: the Hub makes the words `revoked`,
    `current` and `replaced` itself and draws them. The same word is also
    `dataset.status`, which must stay the code.
  - `synorg_members.ts:121`: the category codes in the credential form are
    drawn raw. This is B2's taxonomy work, so it can stay, but the plan should
    say so (it is a code, shown as it came, like an unknown category).
  - `directory.ts:395`: `r.reason || "signature did not verify"`. The reason
    comes from this node's verifier, and the fallback is a sentence (rule 2
    reports it). The plan should say that the `reason` stays as it came and
    only the fallback moves.
  D-B1b-6 says "any other option list the inventory finds", but no step names
  these, and the static check cannot find them (P2).
- **Failure case:** The implementer follows Q4 and D-B1b-6 as written, builds
  tables for the four named sets, and leaves `report.status`, the listing row
  status and the credential status in English. Without the fix to P2 nothing
  catches it.
- **Fix:** Add the list above to D-B1b-6 and to Steps 5, 6 and 10, with the
  table for each (subject kind, report status, listing status, credential
  status) and the three inherited-name cases. State that the `dataset.status`
  and class names keep the code, and that a node-made `reason` and a category
  code stay as they came.
- *Status:* Fixed (db01fa46). Now in: D-B1b-6 (table of 8 code tables, report row as one message, what keeps the code, category codes and verifier `reason` stay as they came).

#### P4 (minor) — Step 9 removes notices that `synorg_members.ts` (Step 10) still imports

- **Where:** Step 9 item 2; Step 10; `synorg_members.ts:1` and `:74`;
  `screens/memberships.ts:6`, `:8`, `:54`, `:55`, `:111`;
  `screens/directory.ts:12`, `:68`; `memberships.test.ts:2`.
- **Problem:** Step 9 turns `NO_INSTANT_REMOVAL_NOTICE`,
  `WITHHELD_REVOCATION_NOTICE` and `ISSUER_CHANGED_CHECK_WORDS` into messages
  and says "their users call `t()`". It lists `directory.ts`, `membership.ts`
  and `memberships.ts`. `synorg_members.ts` also imports
  `NO_INSTANT_REMOVAL_NOTICE` (line 1, used at line 74), and it is a Step 10
  file. After Step 9 the export is gone and `tsc` fails in `synorg_members.ts`.
  The plan says each step builds. Steps 3 and 4 also change files that are in
  `NOT_MOVED_YET` (`rpc.ts`, `dom.ts`, `directory/search.ts`, `login.ts`,
  `main.ts`) but do not say to remove them from the list. The "list cannot go
  stale" test will fail on the first run and send the implementer to find out.
- **Failure case:** The implementer commits Step 9 and `npm run build` stops
  with "Module has no exported member NO_INSTANT_REMOVAL_NOTICE".
- **Fix:** In Step 9 item 2 add `synorg_members.ts` line 74 (the call to
  `t("membership-no-instant-removal")` or the chosen id) as part of the change,
  and say that the rest of that file moves in Step 10. Add "remove the file
  from `NOT_MOVED_YET`" to Steps 3 and 4 for the files they finish.
- *Status:* Fixed (db01fa46). Now in: Step 9 item 2 (`synorg_members.ts:74` and its import); Step 3 item 7 and Step 4 item 4 (remove finished files from `NOT_MOVED_YET`).

#### P5 (minor) — The two Rust tests should check the message line, not the whole file

- **Where:** D-B1b-8; Steps 7 and 9; section 7.2; exit criterion 6.
- **Problem:** The plan changes `the_ui_notices_match_this_crate` and the
  membership test to `content.contains(CONSTANT)` on all of `en.ftl`. The old
  transaction test read the value of a named constant. The new one passes when
  the text is anywhere in the file: in a comment, in a second message, or in a
  message that is no longer used. The B1a booking and group tests already use
  this weaker form, so this is a repeat of an accepted pattern. But Q2 makes the
  data-use notice a signed text that the Hub draws from one message id, and the
  test is the only link between the Rust constant and that id.
- **Failure case:** A later edit changes `request-data-use-notice` and leaves
  the old sentence in a comment above it. The test passes. The Hub signs the
  new text while the Rust constant, `roymctl` and the e2e tests have the old one.
- **Fix:** Look for the whole line `<id> = <CONSTANT>` followed by a line end,
  with the ids named in the plan (`request-data-use-notice` and the other
  three). One `assert!(content.lines().any(|l| l == format!("{id} = {text}")))`
  per notice. The "fails when one sentence differs" check of Steps 7 and 9 then
  also covers a changed id.
- *Status:* Fixed (db01fa46). Now in: D-B1b-8 (named ids, whole `<id> = <text>` line); section 7.2; exit criterion 6.

#### P6 (minor) — `login.ts:268` mixes a Hub-made text and a service text, and Hindi shows `विवरण:` twice

- **Where:** Step 4 item 1; D-B1b-4; `session/login.ts:261-268`.
- **Problem:** The plan turns the six `login.ts` errors into `HubError(t(…))`
  and uses `login-error-http` for `:268`. That throw has two sources: the Hub
  text `HTTP <status>` (made at `:261`, and it passes the static check because
  its fixed part ends in a space) and `String(body.error)`, which is the
  service's own text. Step 4 says the service text "stays as it came", but a
  plain `Error` goes through `error-unexpected`, which in Hindi adds a Hindi
  sentence and `विवरण:`, and `login-failed` adds another sentence and
  `विवरण:` around it. The person reads two "details" lines.
- **Failure case:** A Hindi person with a revoked key sees
  `साइन-इन नहीं हुआ। विवरण: कुछ अनपेक्षित हुआ। विवरण: key revoked`.
- **Fix:** Say which class each branch throws. The Hub branch throws
  `HubError(t("login-error-http", …))`. The service branch throws a `HubError`
  whose message is the service text, or the code shows the service text as the
  `$detail` of `login-failed` directly, without going through
  `error-unexpected`. Add a `login.test.ts` case for the Hindi text of both
  branches and check that `विवरण:` appears once.
- *Status:* Fixed (db01fa46). Now in: D-B1b-4 ("The login reply with a service text", two branches); Step 4 item 3 (Hindi test, `विवरण:` once).

#### P7 (minor) — Single-word field names in the editor errors are invisible to the check, and only one error has a Hindi test

- **Where:** Step 6 items 2 and 3; D-B1b-9; D-B1b-2 ("What it cannot see");
  `editor.ts:158-160`, `:205`, `:214`, `:246`, `:247`, `:294`.
- **Problem:** The check reports the multi-word field names (`"lead time"`,
  `"cancellation window"`, `"max per booking"`, `"pack size"`) by rule 2. It does
  not report the single words `"duration"`, `"warranty"`, `"retention"`
  (`requireWhole`) and `"capacity"` (`toWholeCount`, `editor.ts:294`): they
  are not at a known sink. They are visible text inside an error. Step 6 says
  "the field names … and the others become `t("listing-field-…")`", but the
  only Hindi test is "no title in Hindi" (item 3).
- **Failure case:** The implementer moves the four multi-word names, misses the
  four single words, and nothing fails. A Hindi person who leaves "duration"
  empty reads `duration की ज़रूरत है` (a mix of Hindi and English).
- **Fix:** Name all eight field names in Step 6 item 2. Replace the one Hindi
  test of item 3 by a table test over every `ListingInputError` path (the 11
  throws and the 8 field names): each, in Hindi, has a Devanagari letter and no
  Latin word except the allowed list.
- *Status:* Fixed (db01fa46). Now in: Step 6 item 2 (all eight field names, with lines); Step 6 item 3 (table test over every `ListingInputError` path).

#### P8 (minor) — Two Hindi messages name the Search button by hand; nothing keeps them equal to the button

- **Where:** Section 2.2 point 10; Step 9 item 1; `hi.ftl:33`, `:34`;
  `en.ftl:35`, `:36`.
- **Problem:** B1b renames the button to a Hindi message and edits the two
  search messages to quote the same Hindi word. If the owner changes the button
  text in the review (Q6), the two messages keep the old word and tell the
  person to press a button that does not exist. The same risk holds for English
  (`Press Search`), though English does not change.
- **Failure case:** The owner changes `खोजें` to `तलाश करें` in the review and
  edits the button message only. Two error messages name a button that is not
  there.
- **Fix:** Give both messages a `$button` argument and pass
  `t("directory-search-button")` at the call site. English keeps
  `Press { $button } to try again.` (the output is the same). A catalog test
  then needs no pairing rule.
- *Status:* Fixed (db01fa46). Now in: Section 2.2 point 10; Step 9 item 1 (`$button` variable from `directory-search-button`).

#### P9 (minor) — `index.html` has the English `<title>Roym Hub</title>`, and the plan does not say it is out of scope

- **Where:** ML-1; D-B1b-2 ("Files"); section 3 (the ML-1 row);
  `crates/roym_web/ui/index.html:16`.
- **Problem:** ML-1 says every user-visible string in the Hub comes from the
  catalog. The check reads `.ts` files only. `index.html` has one visible text,
  the page title, as English. `main.ts:79` replaces it with `t("hub-title")`
  once the language loads, so the English title shows only until then (and in
  any place that reads the file, such as a crawler). This is a small case, but a
  reviewer who reads ML-1 against the check will ask.
- **Failure case:** None for a person. A reviewer finds the title and asks if
  the check is incomplete.
- **Fix:** Write one sentence in D-B1b-2 ("Files") and in the change doc at
  close-out: `index.html` holds only the page title, which `main.ts` replaces
  with `t("hub-title")` at start-up; no other file holds visible text, and the
  check does not read it. No backlog row is needed.
- *Status:* Fixed (db01fa46). Now in: D-B1b-2 (`index.html` sentence).

## Plan review round 2 — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-06

Checked on `feat/m08b-b1b-localization-screens` at `4917db83`, in the main
checkout (not a worktree). `git pull` said "Already up to date". Since round
1 the branch has the fix commit `db01fa46`, a merge of `main` (`8b39b525`, an
auth fix for unique session tokens) and the status commit. I read the whole
plan diff (`f9420b45..HEAD`), then checked each fix against the code, as in
round 1. The change docs are the same as in round 1: roym-multilingual, with
the `demo-stories` note unchanged.

**Summary:** round 1: 9 items, all confirmed Fixed. Round 2: 0 blocker,
0 major, 3 minor new items (P10 to P12). **3 open.** Fixed in ae8f0f54; 0 open.

### Round 1 items — confirmed

- **P1 — confirmed.** D-B1b-2 now exempts the third argument of `text()` by
  position and says why a shape rule is wrong. I re-ran my parser: of the
  47 class strings, 42 are the third argument of `text()`, 3 are the third
  argument of `textNode()` (which Step 5 turns into `text()`) and 18 more
  class lists are `className` assignments, which were already exempt. The other
  three hits of my filter are error sentences (`editor.ts:191`, `:195`,
  `login.ts:147`) that must be reported. Section 7.1 has the cases.
- **P2 — confirmed.** D-B1b-12 uses the `Zq` marker for data and real codes in
  the fixtures, a fixed allowed list that is not read from `hi.ftl`, and
  names `UTC`, the currency codes and the fake `/rpc` detail. Section 7.1 and
  exit criterion 3 have the raw-code self-check. The fixtures name the states
  and statuses that the screens show (`delivered`, `deposited`, `failed`,
  `active`, `draft`, `fraud`, `open`, `person`, a revoked and a current
  credential). One gap is in P10 (where the list lives).
- **P3 — confirmed.** The table in D-B1b-6 matches the code: the select lines
  `listings.ts:241`, `:254`, `:269`, `:290`, `:318`, the seven block names
  (`booking`, `payment`, `product`, `service`, `location`, `relationship`,
  `service_record`), `safety.ts:104`, `:135`, `synorg_members.ts:250`, `:257`,
  `:301`. The `safety-report-row` message gives today's English text exactly.
- **P4 — confirmed.** Step 9 item 2 now lists `synorg_members.ts:74` and its
  import. Step 3 item 7 and Step 4 item 4 remove the finished files from
  `NOT_MOVED_YET`.
- **P5 — confirmed.** D-B1b-8 names the four ids and checks the whole
  `<id> = <text>` line. The `format!("{id} = {CONSTANT}")` form compiles (an
  inline argument may name a constant). Section 7.2 also runs the test with a
  changed id.
- **P6 — confirmed.** D-B1b-4 and Step 4 item 3 describe both branches, and the
  service text is a `HubError`, so `विवरण:` appears once. One wording point is
  in P12.
- **P7 — confirmed.** Step 6 lists all eight field names with their lines and
  adds the table test. A small point about `capacity` is in P11.
- **P8 — confirmed.** Both search messages take `$button`; `directory/search.ts`
  passes `directory-search-button`; the English output is unchanged.
- **P9 — confirmed.** D-B1b-2 has the `index.html` sentence, with the right line
  (`index.html:17`; my round 1 text said `:16`, which was wrong).

The owner answers (all (a), with the weight column for Q6) match my views. The
weight list in D-B1b-13 uses real ids: `terms-*`, `receipt-*`, `quote-*`,
`group-delivery-*` and `group-join-*` exist in `en.ftl`.

### New findings

#### P10 (minor) — The allowed Latin word list is made in Step 11, but Step 6 already needs it

- **Where:** D-B1b-12 ("Allowed Latin words … a fixed list in the test file");
  Step 6 item 3 ("no Latin word outside the sweep's allowed list"); Step 11
  item 4 (`hindi-screens.test.ts` is created there); Step 3 item 5
  (`src/testing/stub-rpc.ts`).
- **Problem:** The Hindi table test of the listing editor errors (Step 6) must
  use the same allowed list as the sweep. The list is defined in
  `hindi-screens.test.ts`, which does not exist until Step 11. A test in
  `editor.test.ts` that imports from another test file is also a bad shape
  (importing a `*.test.ts` file runs its tests again).
- **Failure case:** The implementer copies the list into `editor.test.ts` in
  Step 6. Then there are two lists, and a word added to one is missing in the
  other. Or the Step 6 test uses its own looser regex.
- **Fix:** Put the list in a small helper, `src/testing/latin-words.ts`
  (an exported array with the comments, and a function that removes the `Zq`
  words and the allowed words from a text), and create it in Step 3 next to
  `stub-rpc.ts`. Step 6 and Step 11 import it. Say in D-B1b-12 that the list
  is in this helper, not in the test file.
- *Status:* Fixed (ae8f0f54). Now in: D-B1b-12 ("Allowed Latin words": the list and `leftoverLatin()` live in `src/testing/latin-words.ts`); Step 3 item 5 (created there, with its own test); Step 6 item 3 (imports it); section 7.5.

#### P11 (minor) — Step 6's table test names a path that `capacity` does not have

- **Where:** Step 6 item 3 ("the 'is required' and 'whole number' errors for
  each of the 8 field names"); `editor.ts:139-141`, `:293-296`.
- **Problem:** Seven of the eight names go through `requireWhole`, which can
  throw "is required" and "must be a whole number". `capacity` goes through
  `toWholeCount` (`editor.ts:294`), which can throw only "must be a whole number";
  an empty value gives the separate slot error "a slot needs a capacity of at
  least 1" (`:296`), which is one of the 11 throws. A test written as
  "required × 8" has one case that cannot happen.
- **Failure case:** The implementer writes a loop over 8 names × 2 errors,
  and the `capacity` × "is required" case fails or is skipped without a note.
- **Fix:** Write the test list as: the 11 throws, plus "whole number" for each
  of the 8 names, plus "is required" for the 7 names that use `requireWhole`.
- *Status:* Fixed (ae8f0f54). Now in: Step 6 item 3 (11 throws, "whole number" for 8 names, "is required" for the 7 `requireWhole` names; `capacity` explained).

#### P12 (minor) — Two statements in the plan are slightly wrong

- **Where:** Plan header ("`main` at `8b39b525` … changes no file this plan
  names"); D-B1b-4 ("The login reply with a service text") and the `HubError`
  description.
- **Problem:** (1) The merge of `main` changes `docs/system-architecture.md`
  (the auth paragraph near line 711), and the plan names that file (section 9,
  `:1414`). The line the plan edits does not move (one line was replaced by one
  line), so the plan is right in effect but not in words. (2) D-B1b-4 says a
  `HubError` message "is already in the UI language", and then throws a service
  text as a `HubError`. The service text is English. The behaviour is right (it
  is shown once, as the detail), but the class comment and `errText` branch will
  say "Hub text", and a later reader will rely on that.
- **Failure case:** (1) A reader who checks the claim finds the file changed.
  (2) A later change treats every `HubError` message as translated, for example
  to skip a `$detail` wrapper, and shows English where the person reads Hindi.
- **Fix:** (1) Say "changes no line this plan names". (2) In D-B1b-4, define
  `HubError` as "a failure whose message the Hub shows as it is, with no
  wrapper: Hub-made text in the UI language, or a service text that the Hub
  shows as the detail of its own message", and use the same words in the class
  comment.
- *Status:* Fixed (ae8f0f54). Now in: (1) plan header ("changes no line this plan names", with the architecture line); (2) D-B1b-4 (`HubError` defined as a message shown as it is with no wrapper, the same words in the class comment; `errText` branch reworded).

## Plan review round 3 — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-06

Checked on `feat/m08b-b1b-localization-screens` at `08bd75d8`, in the main
checkout. I read the plan diff `f28ac494..HEAD` and checked only the three
open items of round 2.

**Summary:** 0 blocker, 0 major, 0 minor new. **0 open.**

- **P10 — confirmed.** D-B1b-12 puts the list and `leftoverLatin()` in
  `src/testing/latin-words.ts`. Step 3 item 5 creates it with a small test
  (`latin-words.test.ts`), and Step 6 item 3 and the sweep import it. The
  check skips `src/testing/` and `*.test.ts`, so the helper's English words do
  not trip it. Section 7.5 lists the new file.
- **P11 — confirmed.** Step 6 item 3 now lists the 11 throws, "whole number"
  for the 8 names, and "is required" for the 7 `requireWhole` names, with the
  `capacity` reason (`editor.ts:296`).
- **P12 — confirmed.** The header says "changes no line this plan names" and
  gives the architecture line. `HubError` is defined as a message shown as it
  is, with no wrapper, and the `errText` branch uses the same words.

No new findings. The plan is ready for implementation.

## Code review round 1 — Claude Opus 5.5, 2026-11-06

Reviewed `git diff origin/main...HEAD` on
`feat/m08b-b1b-localization-screens` at `67ccd288` (PR #351), in the main
checkout (not a worktree). The code was written by Claude Sonnet 5.5. The
prompt lists one change doc,
[roym-multilingual](../../changes/roym-multilingual/change.md). Every B1b
decision and deviation belongs there, and that is where they are written.

**Summary:** 0 blocker, 0 major, 6 minor (R1 to R6). All 6 Fixed in `5fe574ef`; 0 open. No pushback.

### What was checked and found correct

- **CI.** `gh pr checks 351` on `67ccd288` (run 42232623857), when this
  round was written: Lints, licenses & checks, Playwright E2E and CLA pass;
  Test 1/6 to 6/6 still running; Benchmarks and Code Coverage skip. The
  only Rust change is in `roym_core` (two tests, three doc comments), and
  `cargo nextest run -p syneroym-roym-core` here gives 233 passed. Check
  that the six shards pass before the merge.
- **Hub tests and build, run here.** `npm test`: 31 files, 328 tests
  passed. `npm run build`: the visible-text test runs inside the build (28
  passed), then `vite build`. Main script 167,109 bytes raw, 44,965 after
  `gzip -9`, no Devanagari letter; Hindi chunk 14,000 after `gzip -9`.
  `en.ftl` and `hi.ftl` have 644 messages each. These match status.md,
  task.md, the change doc and the PR text, and are inside the limits of
  plan section 7.6.
- **English is unchanged (D-B1b-1), checked against `main`.** I took the
  fixtures of `hindi-screens.test.ts`, drew the same 15 screens and dialogs
  in English on a copy of `main` and on a copy of `67ccd288` (both in
  `$TMPDIR`, with a fixed clock), and collected every visible text the
  sweep collects. 534 texts on each side, 0 differences. I did the same
  for the word functions that became Fluent selectors (`membershipWords`
  for 15 verdicts, including `suspended` with and without a reason and an
  end date, `refusalWords`, `pinnedIssuerWords`, `checkedWords`,
  `publishRefusalWords`, `refusalText`, `stateHint`): 39 texts, 0
  differences. `src/cards/samples.json` is equal to the old `samples`
  array. The only English change I found is R1.
- **The check (D-B1b-2, D-B1b-3).** `findVisibleText` follows the parts
  of `?:`, `+`, `??`, `||`, `&&`, `as`, `satisfies`, `!` and templates;
  rule 1 has every sink of the plan; rule 2 exempts by position only (the
  `console.*` arguments, `className`/`rel`, the third argument of
  `text()`, `classList.*` and the selector calls); type nodes and comments
  are skipped; a literal seen by both rules is reported once. The file
  list is every `src/**/*.ts` except the tests, `src/testing/`, `i18n.ts`
  and `scan.ts`. `package.json` runs it in `npm run build`. I listed every
  string literal with a letter that is left in the checked files (480):
  they are codes, ids, method names, MIME types and CSS values. No English
  word that a person reads is left in a table or a variable.
- **Hub-made and browser errors (D-B1b-4).** `HubError` has the class
  comment of review P12. `MoneyInputError` and `ListingInputError` extend
  it; every `throw` in the checked files is a `HubError`, an `RpcError`,
  or the one `Error` in `i18n.ts` (console only). `errText` has the four
  branches; `rpc.ts` uses `error-no-detail`. The `login-error-service`
  deviation is right: the plan's `HubError(String(body.error))` would have
  shown the service text with no `विवरण:` at all.
- **Text that is signed (D-B1b-7, D-B1b-8).** The request form draws and
  sends the same `dataUseNotice` string; the three quote defaults come from
  `t()`. The byte-length test covers both languages. The two Rust tests
  look for the whole `id = text` line; the TypeScript-string parser is
  gone, and `transaction/tests.rs` (797 lines) left the oversized list.
  The three doc comments say what the code does. No WIT, method or record
  change; no `roym_*` code change, so the WASM build is not affected.
- **Codes shown as words (D-B1b-6).** Every new table is a `Map` of
  `messageId` values read with `lookupText`; the option `value`, the class
  name and `dataset.status` keep the code. `revocation_status` can only be
  `good` or `unknown` (`listing::revocation_status_word`), so the two-row
  revocation table is complete.
- **Helpers (D-B1b-5).** One `text`, `field`, `input` and `checkbox`, all
  in `dom.ts`; `stubRpc` is in `src/testing/stub-rpc.ts`, and every test
  that uses it imports it.
- **The Hindi sweep (D-B1b-12).** It draws the screens and dialogs of the
  plan with real codes in the fixtures, collects text nodes, attributes and
  field values, and has the two self-checks. The allowed list is a fixed
  list with a reason per item and whole-word matching.
- **No `t()` when a module loads.** `BUNDLES` labels are functions; no
  module-level constant calls `t()`.
- **Docs.** The requirements, architecture, traceability, developer guide
  and demo edits say what the code does. Two backlog rows (the literal-only
  check, Latin `am`/`pm`) have reason, target and link. `hi-review.md` has
  644 rows, and the 57 weighted rows include every message D-B1b-13 names.
  Story 40 reads the Hindi texts through `hubMessage`. No planning id in
  any added `.ts`, `.ftl`, `.json` or `.rs` line. status.md counts match
  what the tests print here.

### Findings

#### R1 (minor) — The backup import line shows `{$count}` for four of the five bundles

- **Where:** at `67ccd288`:
  [backup.ts:96](../../../../crates/roym_web/ui/src/screens/backup.ts#L96)
  to `:99`; [en.ftl:262](../../../../crates/roym_web/ui/src/i18n/en.ftl#L262).
- **Problem:** The code reads `res?.imported_records ?? res?.imported` and
  passes it as the Fluent number `$count`. No service returns a number
  there. `conversation.import` returns `imported: { messages, admissions }`,
  `catalog.import` and `directory.import` return `imported` as a map of
  counts, `transaction.import` returns `imported: true`, and
  `profile.import` returns neither field (Rust `*ImportOutput` structs in
  `crates/roym_{conversation,catalog,directory,transaction,profile}/src/app/backup.rs`).
  Fluent refuses an object or a boolean argument ("Variable type not
  supported") and prints `{$count}`. I checked this with the real `t()`:
  an object and `true` both give `Imported: {$count} record(s).`, and a
  number gives `Imported: 2 record(s).`. On `main` the line was
  `Imported: [object Object] record(s).` or `Imported: true record(s).`.
  That was already wrong, but B1b changes the English text, which D-B1b-1
  says must not change, and the type `imported?: number` hides the real
  shapes. No test imports a bundle, so nothing saw it.
- **Failure case:** A person imports their conversation bundle, in
  English or Hindi. The success line says `Imported: {$count}
  record(s).`, which looks like a broken page, and `console.error` logs a
  format error each time.
- **Fix:** Type the reply as the services send it, and pass a number only:
  a number as it is, a map of counts as the sum of its number values,
  anything else (`true`, absent) as `backup-imported-unknown`. Add a test
  in a new `backup.test.ts` with the five reply shapes (`stubRpc`), which
  checks the English line for each. Record under Deviations that the
  English line changes from `[object Object]`/`true` to a real count or
  the `bundle` text.
- *Status:* Fixed (5fe574ef). Now in: `importedCount()` in `backup.ts` (a number as it is, a map of counts added up, anything else as `backup-imported-unknown`); new `backup.test.ts` (the five reply shapes through `stubRpc`, and the helper); change doc Deviations (the English line changes from `[object Object]`/`true` to a count or `bundle`).

#### R2 (minor) — A failed sign-in in Hindi says "sign-in failed" twice

- **Where:** at `67ccd288`:
  [hi.ftl:32](../../../../crates/roym_web/ui/src/i18n/hi.ftl#L32)
  (`login-failed`), [:200](../../../../crates/roym_web/ui/src/i18n/hi.ftl#L200),
  [:202](../../../../crates/roym_web/ui/src/i18n/hi.ftl#L202),
  [:203](../../../../crates/roym_web/ui/src/i18n/hi.ftl#L203);
  [main.ts:231](../../../../crates/roym_web/ui/src/main.ts#L231) and
  `:254`; [login.test.ts](../../../../crates/roym_web/ui/src/session/login.test.ts)
  ("a failed login, as the person reads it").
- **Problem:** The person reads `t("login-failed", { detail: errText(e) })`.
  In English the inner messages are only the cause (`HTTP { $status }`,
  `{ $detail }`), so the line is `Sign-in failed: HTTP 500`. In Hindi the
  inner messages repeat the outer sentence: `login-error-http` is
  `साइन-इन नहीं हो सका: HTTP { $status }`, so the line is
  `साइन इन नहीं हो सका: साइन-इन नहीं हो सका: HTTP 500`.
  `login-error-challenge` and `login-error-service` also start with
  "sign-in" again. The two spellings `साइन इन` and `साइन-इन` sit side by
  side. The new tests check `errText(err)` only, not the line that
  `main.ts` draws, so they cannot see this. The owner review cannot see it
  either: `hi-review.md` shows each message alone, and each one alone
  reads well.
- **Failure case:** A Hindi user's sign-in is refused with HTTP 500. The
  Hub shows the same sentence twice in a row, with two spellings.
- **Fix:** Make the Hindi inner messages name only the cause, like the
  English ones: `login-error-http = HTTP { $status }`, and
  `login-error-challenge` and `login-error-service` without the leading
  "sign-in" words (for example `सेवा ने मना कर दिया। विवरण: { $detail }`).
  Use one spelling of "sign-in" in `hi.ftl`. In the Hindi cases of
  `login.test.ts`, check the composed line
  `t("login-failed", { detail: errText(err) })`: it holds the
  `login-failed` sentence once and `विवरण:` at most once.
- *Status:* Fixed (5fe574ef). Now in: `hi.ftl` (`login-error-http` is `HTTP { $status }`; `login-error-challenge`, `login-error-service`, `login-error-methods` name only the cause; one spelling `साइन इन`, also in `login-auth-unreachable`); `login.test.ts` (the Hindi cases check the composed `login-failed` line: the sentence once, `विवरण:` at most once); `catalog.test.ts` (`login-error-http` in `NO_WORDS`, with a reason); change doc Deviations.

#### R3 (minor) — Pinned sentences in `en.ftl` lost their warning, and the change doc's pinned count is old

- **Where:** at `67ccd288`:
  [en.ftl:508](../../../../crates/roym_web/ui/src/i18n/en.ftl#L508),
  `:509` (the delete notes); `:458` (`request-data-use-notice`), `:492`
  (`quote-address-disclosure-notice`), `:547`, `:548` (the membership
  notices); against [en.ftl:47](../../../../crates/roym_web/ui/src/i18n/en.ftl#L47)
  to `:49`; [change.md:169](../../changes/roym-multilingual/change.md#L169).
- **Problem:** (1) On `main`, `messages.ts` had a "why" comment above the
  two delete notes: they are shown before a delete runs, they match the
  `note` that `conversation.delete-message` returns, and the product must
  not claim the other side's copy is gone. B1b deleted the constants and
  the comment, and `en.ftl` has no comment there. (2) B1a put the
  Rust-pinned sentences under the comment "Sentences that the Rust side
  also holds … Do not wrap them or change a word without changing the
  Rust side". The four sentences B1b pins sit in their screen sections
  with no such comment. (3) The change doc bullet "Pinned English" still
  says 8 booking and 10 group sentences and "the two Rust tests"; there
  are now 22 sentences and four tests. (The new "Text that is signed"
  bullet names the four, but the "Pinned English" bullet is now
  incomplete.)
- **Failure case:** The owner review, or a later slice, improves the
  English of `messages-delete-note-received` in `en.ftl`. No test reads
  that line (the e2e check at `roym-hub.spec.ts:395` is for the sent
  note), so the Hub now says something different from what the service
  says the delete did. For the four Rust-pinned lines the Rust test fails,
  but only after the edit, with no warning in the file.
- **Fix:** Add a `#` comment above the two delete notes with the reason of
  the old comment. Add a `#` comment above each of the four Rust-pinned
  lines (or one per screen section), in the words of the B1a block
  comment. Update the "Pinned English" bullet of the change doc: 8
  booking, 10 group, the data-use and address-disclosure notices and the
  two membership notices, read by four Rust tests.
- *Status:* Fixed (5fe574ef). Now in: `en.ftl` (a comment above the two delete notes with the reason of the old comment, and above each of the four Rust-pinned lines); change doc "Pinned English" bullet (22 sentences, four Rust tests, the delete notes).

#### R4 (minor) — Six of the new code tables have no inherited-name test

- **Where:** at `67ccd288`:
  [listings.ts:38](../../../../crates/roym_web/ui/src/screens/listings.ts#L38)
  to `:57` (`BOOKING_MODE_LABELS`, `PRICING_MODEL_LABELS`,
  `CONDITION_LABELS`, `DISCLOSURE_LABELS`);
  [directory.ts:21](../../../../crates/roym_web/ui/src/screens/directory.ts#L21)
  (`REVOCATION_LABELS`);
  [synorg_members.ts:63](../../../../crates/roym_web/ui/src/screens/synorg_members.ts#L63)
  (`CREDENTIAL_STATUS_LABELS`).
- **Problem:** Plan D-B1b-6, section 7.1 ("Code tables") and exit
  criterion 7 ask, for each new table, that a known code gives its English
  word in `en` and a different text in `hi`, and that `constructor`,
  `toString` and `__proto__` are shown as they came. The tests do this for
  the listing status, the message state and the three report tables
  only. `REVOCATION_LABELS` is read with `hit.revocation_status`, a value
  that comes from the node's answer; the others are read with fixed codes
  today. The code is right (every table is a `Map` read with
  `lookupText`), so this is a missing test, not a bug.
- **Failure case:** A later change turns one of these tables into an
  object literal or reads it with `table[code]`. A directory answer with
  `revocation_status: "constructor"` then shows `function Object() …` on
  the screen, and no test fails.
- **Fix:** Add the cases: one for `REVOCATION_LABELS` in
  `directory.test.ts` (render a hit with `good` in `en` and `hi`, and with
  the three inherited names), one for `CREDENTIAL_STATUS_LABELS` in
  `synorg_members.test.ts`, and one for the four editor tables in
  `listings.test.ts` (every option label differs between `en` and `hi`).
  If R5 moves the read into one shared `codeWord`, the inherited-name part
  can be one test of that function, and each table test only needs the
  `en`/`hi` part.
- *Status:* Fixed (5fe574ef). Now in: `i18n.test.ts` (`codeWord`: known code, unknown code and the inherited names `constructor`, `toString`, `__proto__`, `hasOwnProperty`); `directory.test.ts` (revocation line in `en` and `hi`, and the three inherited names); `synorg_members.test.ts` (credential status words in `en` and `hi`, code kept in `dataset.status`); `listings.test.ts` (an option of each of the four editor tables differs between `en` and `hi`).

#### R5 (minor) — The same code-word helper is written twice, and the same read is written nine times

- **Where:** at `67ccd288`:
  [listings.ts:59](../../../../crates/roym_web/ui/src/screens/listings.ts#L59)
  and [safety.ts:102](../../../../crates/roym_web/ui/src/screens/safety.ts#L102)
  (two equal `codeWord(table, code)` functions);
  `messageStateWord` at [messages.ts:24](../../../../crates/roym_web/ui/src/screens/messages.ts#L24),
  and `lookupText(TABLE, code) ?? code` written out at
  [directory.ts:337](../../../../crates/roym_web/ui/src/screens/directory.ts#L337)
  and [synorg_members.ts:261](../../../../crates/roym_web/ui/src/screens/synorg_members.ts#L261);
  two ways to build a select of codes: `codeSelect` in `safety.ts` and
  `select(codeOptions(…))` in `listings.ts`.
- **Problem:** D-B1b-5 is "one copy of each helper", and AGENTS.md says to
  look for an existing helper before writing a new one. The rule "a code
  the table lacks is shown as it came" (D-B1b-6, ML-7) is now written in
  five places, so a later change of that rule (for example, when B2 shows
  an English label for a code with no Hindi label) must find all of them.
- **Failure case:** B2 changes the fallback in `listings.ts`'s
  `codeWord` and misses `safety.ts` and `directory.ts`; two screens then
  follow different rules for an unknown code.
- **Fix:** One exported `codeWord(table, code)` next to `lookupText` in
  `src/i18n/i18n.ts` (it is the same layer), used by every place above;
  `messageStateWord` can call it. Use one select builder for code
  options (move `select` and `codeOptions` to `dom.ts`, or make `safety.ts`
  use them). Remove the two local copies.
- *Status:* Fixed (5fe574ef). Now in: one `codeWord(table, code)` in `i18n.ts` used by `listings.ts`, `safety.ts`, `messages.ts` (`messageStateWord`), `directory.ts` and `synorg_members.ts`; `select` and `codeOptions` moved to `dom.ts` and used by `listings.ts` and `safety.ts`; the local copies and `codeSelect` are gone; developer guide and change doc name `codeWord`.

#### R6 (minor) — The change doc misses decision D-B1b-10 and the `NO_WORDS` change

- **Where:** at `67ccd288`:
  [change.md:166](../../changes/roym-multilingual/change.md#L166) to `:179`
  (the B1b decisions) and `:441` (`## Deviations`);
  [catalog.test.ts:92](../../../../crates/roym_web/ui/src/i18n/catalog.test.ts#L92).
- **Problem:** (1) The plan's Step 13 item 3 moves D-B1b-1 to D-B1b-15
  into the change doc. Every decision is there except D-B1b-10: a message
  is shared only when the English text and the meaning are the same and
  the sentence is generic (`common-load-failed`, `common-save-failed`,
  `common-saved`, `common-cancel`, `common-remove`); every other message
  has its own id. The "Message ids" bullet names the `common-` prefix
  only. (2) D-B1b-12 lists `NO_WORDS` as `terms-range-separator`,
  `tab-synorg` and the command messages. The code has no command message
  (every command sits inside a Hindi sentence) and adds
  `listing-version`, `listing-slot-line` and `safety-report-row`. This
  is a difference from the plan with no Deviations entry.
- **Failure case:** A later slice adds a new "Could not load …" text. It
  reads the change doc, finds no sharing rule, and reuses
  `common-load-failed` for a sentence with a different meaning in Hindi,
  or adds a fourth copy of it.
- **Fix:** Add a bullet "Shared messages" to the B1b decisions with the
  rule and the five ids. Add a Deviations entry: `NO_WORDS` holds the
  three messages that are only values and punctuation, and no command
  message, because every command sits inside a Hindi sentence.
- *Status:* Fixed (5fe574ef). Now in: change doc "Shared messages" bullet (the rule and the five `common-` ids), a "Code words" bullet, and Deviations entries for `NO_WORDS` (no command message; `listing-version`, `listing-slot-line`, `safety-report-row`, `login-error-http`), the backup import line and the Hindi sign-in messages.

## Code review round 2 — Claude Opus 5.5, 2026-11-06

Checked the round 1 fix commits `5fe574ef` (code) and `12e13f47` (docs) on
`feat/m08b-b1b-localization-screens` at `12e13f47`, in the main checkout.
The fix commits change the Hub UI (`crates/roym_web/ui/src`), the change
doc, the developer guide, `hi-review.md`, task.md, status.md and this file.
No Rust code changes.

**Summary:** R1 to R6 confirmed fixed. 1 new item: 0 blocker, 0 major,
1 minor (R7). R7 Fixed in `be6f5ca4`; 0 open.

**Evidence.** `gh pr checks 351`: every job passes on `12e13f47` (run
42241961224: Lints, licenses & checks, Test 1/6 to 6/6, Playwright E2E,
CI Pipeline; CLA passes; Benchmarks and Code Coverage skip). Here:
`npm test` 32 files, 337 tests passed; `npm run build` passes (the
visible-text test runs inside it); main script 45,160 bytes and Hindi chunk
13,980 bytes after `gzip -9`, main script with no Devanagari letter. These
match status.md and task.md. I repeated the English comparison with `main`
of round 1 on a copy of `12e13f47` (same fixtures; `hindi-screens.test.ts`
did not change): 534 screen texts and 39 sentences, 0 differences.

### Round 1 items

- **R1 — confirmed fixed (`5fe574ef`).** `importedCount()` returns a
  number as it is, the sum of the number values of a map, and `undefined`
  for `true`, `{}`, a missing field or `null`. That covers every reply
  shape of the five Rust `*ImportOutput` structs. `backup.test.ts` drives
  the five bundle rows through `stubRpc` with those shapes and checks the
  exact English line, so a `{$count}` line now fails a test. The change of
  English text is in Deviations.
- **R2 — confirmed fixed (`5fe574ef`).** The Hindi `login-error-http`,
  `-challenge`, `-service` and `-methods` name only the cause, and `hi.ftl`
  has one spelling (`साइन इन`; no `साइन-इन` is left). `login.test.ts` now
  checks the line that `main.ts` draws: `साइन इन नहीं हो सका: HTTP 500`
  exactly, `साइन` once, and `विवरण:` once for the service case.
  `login-error-http` is in `NO_WORDS` with a reason, and the change doc
  records both points.
- **R3 — confirmed fixed (`5fe574ef`, `12e13f47`).** Each of the four
  Rust-pinned lines has a comment in the words of the B1a block. The delete
  notes have a standalone comment with the old reason. The "Pinned
  English" bullet names the four new sentences and the four Rust tests.
- **R4 — confirmed fixed (`5fe574ef`).** `codeWord` has its own test for
  a known code, an unknown code and four inherited names. The revocation
  line is tested in `en` and `hi` and with the three inherited names; the
  credential status words are tested in `en` and `hi` with the code kept in
  `dataset.status`. The listings test checks one option of each of the four
  editor tables (and the status). Its name says "every option", but it
  checks five; every other option is in Hindi too (I compared the 15
  `listing-*` code messages, and the sweep draws the editor), so this is a
  note, not a finding.
- **R5 — confirmed fixed (`5fe574ef`).** One `codeWord` in `i18n.ts`, used
  by `listings.ts`, `safety.ts`, `messages.ts`, `directory.ts` and
  `synorg_members.ts`; `select` and `codeOptions` are in `dom.ts`; the
  local copies and `codeSelect` are gone. The two report selects keep the
  table order and set no value, as `codeSelect` did. But see R7.
- **R6 — confirmed fixed (`12e13f47`).** The change doc has the "Shared
  messages" bullet with the rule and the five ids, and Deviations entries
  for `NO_WORDS` (now with `login-error-http`), the backup line and the
  Hindi sign-in messages.

### New findings

#### R7 (minor) — The new `codeWord` comment and the change doc say it is the only reader of code tables, but five B1a reads do not use it

- **Where:** at `12e13f47`:
  [i18n.ts:182](../../../../crates/roym_web/ui/src/i18n/i18n.ts#L182)
  ("Every screen that shows a code as a word reads its table through this
  function"); [change.md:184](../../changes/roym-multilingual/change.md#L184)
  ("the one place that reads a table of code words"); against
  [listings/labels.ts:32](../../../../crates/roym_web/ui/src/listings/labels.ts#L32),
  `:38`, `:48` and
  [booking_progress.ts:67](../../../../crates/roym_web/ui/src/cards/templates/booking_progress.ts#L67),
  `:90`, which still write `lookupText(TABLE, code) ?? code`.
- **Problem:** The fix of R5 moved every B1b read to `codeWord`, as R5
  asked. R5 listed only the B1b places, but the new comment and the change
  doc say more: that every read goes through `codeWord`. Five B1a reads
  (the service location and audience words, the booking state and conflict
  words of the card) still use the old form. A doc comment that is not
  true is what AGENTS.md calls an over-claim.
- **Failure case:** B2 changes the rule in `codeWord` (for example, an
  English label for a code with no Hindi label), trusting the comment that
  every screen reads through it. The listing editor's location words and
  the booking card keep the old rule, and no test shows the difference.
- **Fix:** Use `codeWord` in the five places (each is one line; the
  functions in `labels.ts` then call it, and `booking_progress.ts` imports
  it). Their existing tests stay as they are. The other choice is to make
  the comment and the change doc say "the B1b screens", but one rule in
  one place is the point of R5.
- *Status:* Fixed (be6f5ca4). Now in: `labels.ts` (the three place and audience words) and `booking_progress.ts` (the state and conflict words) call `codeWord`; the `codeWord` comment and the change doc name the one other reader, `errText`, whose unknown error code has a message of its own and is not shown as it came. `main` (`3165d1a2`, the `vodozemac` upgrade) is merged in (`6ad7dfba`): clippy clean, 406 Rust tests passed in `roym_core` and `conversation`, Hub tests 337 passed.

## Code review round 3 — Claude Opus 5.5, 2026-11-07

Checked the round 2 fix commit `be6f5ca4` and the merge of `main`
(`6ad7dfba`) on `feat/m08b-b1b-localization-screens` at `39d607ca`, in the
main checkout.

**Summary:** R7 confirmed fixed. No new items: 0 blocker, 0 major, 0 minor.
**0 open.**

**Evidence.** `gh pr checks 351`: every job passes on `39d607ca` (CI run
42332060307: Lints, licenses & checks, Test 1/6 to 6/6, Playwright E2E,
CI Pipeline; CLA passes; Benchmarks and Code Coverage skip). Here: `npm
test` 32 files, 337 tests passed; `npm run build` passes; `cargo audit`
finds no vulnerability.

- **R7 — confirmed fixed (`be6f5ca4`).** The three functions of
  `listings/labels.ts` and the two reads of `booking_progress.ts` call
  `codeWord`, with the same result as before (`lookupText(…) ?? code`).
  The only `lookupText` call left outside `codeWord` is `errText` in
  `dom.ts`, whose fallback is a sentence (`error-service`), not the code.
  The `codeWord` comment and the change doc's "Code words" bullet now name
  that one exception, so both say what the code does.
- **The merge of `main` (`6ad7dfba`).** It brings only the three files of
  the `vodozemac` upgrade (PR #352: `Cargo.toml`, `Cargo.lock`,
  `crates/conversation/Cargo.toml`). No B1b file changes in it.
