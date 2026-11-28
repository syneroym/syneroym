# M08C slice C8 — review

*Reader: the developer who writes or fixes the C8 plan and code, and the
project owner. This file holds the review findings for slice C8
(close-out). Plan review findings are P1, P2, …. Code review findings are
R1, R2, …. Ids are never renumbered.*

## Plan review round 1 — Claude Sonnet 5.5, 2026-11-28

Plan reviewed: [slice-c8-implementation-plan.md](slice-c8-implementation-plan.md)
(written by Claude Opus 5.5, not committed when this round started).
Checkout: `feat/m08c-c7b-record-forward` at `05e9b164`. This branch holds
the C7b code (PR #400, merged according to status.md), so I checked the
plan's facts against the C7b tree, not only against `79344661`. `git
fetch` failed here (no network), so I could not compare with the real
`origin/main`.

**Change docs.** The four docs in the prompt (agreements, overview,
extensions, forwarding) were read. task.md's "Change docs" table also
names a fifth doc, [roym-multilingual](../../changes/roym-multilingual/change.md)
(`done`). It is not in the prompt's list. The plan already covers it
(D-C8-6), so I read the parts D-C8-6 cites and checked them. This is the
first round, so there are no earlier items to re-check.

### Checked and found correct

- **Test names.** I took all 146 backticked names of the plan and ran
  `git grep -w -F 'fn <name>'` for each. Every existing name resolves to
  one function (`setup_agreed_deal` has three definitions, one per test
  file). The names that do not resolve are exactly the five wrong names of
  section 2.2 point 4, the wrong names that the AG-49 row also holds, and
  the new tests T1 to T13. I also ran the same check over the Test column
  of every `AG-*`, `EX-*`, `FW-*` and `VX-*` row: the only missing names
  are the ones the plan lists (AG-44, AG-45, AG-49, AG-50, AG-51; see P13
  for AG-49).
- **Empty Test columns.** The eight empty `EX-*` rows (EX-4, 5, 6, 14,
  17, 20, 21, 22), the eight empty `VX-*` rows (VX-1, 5, 6, 8, 9, 10, 11,
  12) and the `AG-*` rows with a bare kind ("Unit.", "Service.") are as
  the plan says. On this branch every `FW-*` row is filled and its names
  exist.
- **Parity scenarios.** 48, 144, 157, 247, 251, 253, 254, 271 to 273, 275,
  276, 278, 289 and 292 exist. Scenario 255 does not exist (matches the
  backlog row).
- **Card count.** `CARD_TYPES` (`crates/roym_core/src/card.rs:11`) has 10
  entries. `RECORD_TYPES` has `deal-decline`, `step` and `payment-request`.
  The three sentences the plan wants to fix exist: agreements doc `:1026`
  and `:1072`, extensions doc `:149`. The integrated spec already says
  "ten templates".
- **AG-9.** `quote-author-only` is returned by `cancel_at`
  (`progress_ops.rs:241`) and no test checks it. `commitment-moved` is
  `progress_ops.rs:98`. The plan's D-C8-5 reading is right.
- **Differences table.** The table has 14 rows. D-C8-4 maps every one of
  them (it joins the two integrated-spec rows C12 and C14).
- **D-C8-6.** `QuoteTranslation` and `RequestPayload` no longer exist
  (`git grep -w` finds none). Listings (`listing.rs:197`) and profiles
  (`person.rs:45`) keep `translations`. `translation-set-mismatch` exists
  in `roym_core/src/deal.rs`.
- **check-change-docs.** For `status: done` it needs `living-docs-touched`
  and `demo:` to be non-empty with existing paths (`xtask/src/change_docs.rs`).
  It does not check that Tasks are ticked, so the plan's Tasks edits are
  by hand, as the plan says.
- **Size rules.** Current lengths match the plan (`as_listed_owner.rs`
  639, `cancel.rs` 319, `refuse/version.rs` 284, `recover.rs` 626,
  `extension.rs` 300, `roym_deal_e2e.rs` 605, `services/tests.rs` 250,
  `listings.test.ts` 211). No new test file is over 800 lines.
- **Section "How this is shown to an end user"** is present, right after
  the one-paragraph goal, in simple words, and says that an end user sees
  nothing. It gives the developer view. Good.
- **No product code change** is claimed and the step 8 `git diff` command
  would catch one. The new tests are native-only service tests; no WASM
  difference is hidden by this, because no product code changes and the
  parity binary runs in `mise run verify`.
- **Backlog rows.** Every row title of section 8 exists. The line numbers
  are 2 higher than the plan says (see P10).

### Counts

Round 1: blocker 0, major 3 (P1, P2, P3), minor 10 (P4 to P13).
**13 open.**

### Views on the owner's questions

- **Q1 (a), C8 also does the milestone close.** Agree. T8 is a fresh audit
  by a tool that did not write the work. Here the C8 code review is done
  by a different tool than the implementer, and C8 changes no product
  code, so a second session would repeat the first. Keep one condition:
  the C8 code review must use the T8 checklist and say so in the review
  file.
- **Q2 (a), C8 adds the missing tests.** Agree, with one change to the
  stop rule (see P1). T5 will hit the rule at once, and the plan has no
  way to tell "the code is wrong" from "the code refuses for another
  documented reason". Add that second case: the test asserts the real code
  and the doc gets a Deviation. Stop only when the code does not refuse, or
  does something else. The test count is also big for an "S" slice. Still,
  (c) (split C8a and C8b) would cost one more review cycle for little gain
  while the tests are test-only.
- **Q3 (a), keep the criterion text and add "Narrowed: see Deviations".**
  Agree. It keeps the record of what was asked.
- **Q4 (a), target "M08D" for the cut as-listed tests.** Agree. M08D D3
  puts the variant into `as_listed.variant` and `terms_from_listing`
  (M08D task.md, D3 row), so a two-node test written now would be rewritten.
- **Q5 (a), one Hindi table in the C8 row.** Agree, but the list in the
  plan is not complete (see P2). The table must be built from the diff of
  `hi.ftl`, not from the text of status.md.
- **Q6 (a), keep `roym-agreements/review.md`.** Agree. Four planning files
  link to it and the change doc cites decisions that only it records.
- **Q7 (a), branch `feat/m08c-c8-closeout`.** Agree. Every earlier M08C
  branch is `feat/m08c-…`, except the C6 and C7 branches that carry an
  `m08e` or `m08a` prefix, which is already confusing.

## New findings

### P1 — T5 will fail on three of its four cases: `quote.set` after an accepted deal answers `quote-already-accepted` (major)

*Where:* plan section 7.1, T5; section 3.1, row AG-35;
`crates/roym_transaction/src/app/quote_ops.rs:354-370` (`check_prior`),
`crates/roym_transaction/src/app/decline_ops.rs:352`
(`has_unrefused_counterparty_half`), `crates/roym_transaction/src/app.rs:680`
(`is_deal_ended`).

*Problem.* T5 says: for each end `not-formed`, `cancelled` and `closed`,
`quote.set` of a new version "answers `quote-ended`". In `quote.set` the
code checks `has_unrefused_counterparty_half` first. It answers
`quote-already-accepted` when the counterparty's half is filed and was not
refused. It checks `is_deal_ended` only after that. A deal that reached
`cancelled` or `closed` always has a filed counterparty half. A `not-formed`
deal with a deal-level reason (`slot-taken`, `quote-expired`, an extension
reason) also has one (`is_refused_by_snapshot` is false for these
reasons). So for all three ends the answer is `quote-already-accepted`.
The existing tests agree: `revision_after_an_end_is_refused` covers only
`expired` and `declined`, the two ends that have no counterparty half;
`revision_after_acceptance_is_refused` covers the other path. The new
refusal is real (a new version is refused), but its code is not the one
AG-35 names.

*Failure case.* The implementer writes T5 as the plan says. It fails on
`cancelled`. By the Q2 rule the implementer must stop, put the test under
`#[ignore]`, add a backlog row and ask the owner. The plan did not see this
coming, so the slice stops on its first new test.

*Fix.* Decide it in the plan now. Recommended: T5 asserts the real code for
each end (`quote-already-accepted` after an accepted deal ends, `quote-ended`
for `expired` and `declined`, already covered). Add a Deviation bullet in
the agreements doc: "**AG-35: after an accepted deal ends, `quote.set` is
refused with `quote-already-accepted`.** The row says `quote-ended`. The new
version is refused either way; the code names the stronger fact first."
Change the Q2 (a) text to: "If a new test shows another documented refusal
code, assert it and write a Deviation. Stop only if the code does not
refuse, or does something else." Also say in T5 how each end is reached in
the test (`cancelled`: one `agreement.cancel`; `closed`: `setup_agreed_deal`
and the close helper of `settle.rs`; `not-formed`: the `slot-taken` path of
`decide.rs` tests). A cheaper alternative the owner may prefer: keep
`quote-ended` for these ends and change the code order. That is a product
change and C8 does not do it; it would be a backlog row.

*Status:* Open

### P2 — The Hindi review table of Q5 leaves out the 46 keys of C3c2 and is built from text, not from the files (major)

*Where:* plan Q5; status.md rows C3c1, C3c2, C6.

*Problem.* Q5 lists keys "for these rows" that status.md marks as ready or
open. It does not list C3c2. The C3c plan (section 7.10, line 1600) adds
**46** new Hindi keys (`as-listed-*`, `listing-acceptance-instant`,
`listing-instant-needs-signing`, and so on). The C3c plan's exit checkbox
says "Owner reviewed the Hindi messages of C3c2 (… ready for owner review at
merge)". That line is ticked, but its own words say the review was not done
yet. The status.md C3c2 row has no word about Hindi. The C3c review file
holds a later Hindi change of the instant form (plan line 2153). The
plan's Q5 looks at status.md only, so a row with no Hindi note is missed.
The milestone would close with 46 messages that nobody checked, which is
exactly what Q5 is meant to prevent.

*Failure case.* The owner answers Q5 (a), reviews the 7 + 6 + 1 + C7 keys
of the table, and M08C closes. The `as-listed-*` messages (the owner
answers an instant tap, the counterparty sees "waiting") stay unchecked.

*Fix.* Build the table from the files. In step 10, list every key that is
new or changed in `hi.ftl` since the first M08C commit (`git log -p
--first-parent <first M08C commit>..HEAD -- crates/roym_web/ui/src/i18n/hi.ftl`,
or a diff against the base). Remove the keys that a status.md row says the
owner reviewed (C1a, C2a, C3b, C4b, C6b). Put the rest in the C8 row with
the English and Hindi text, and add a plain sentence to C3c2's row that
its review was never recorded. Add the C3c2 keys to Q5's text so the owner
sees the true size of the question.

*Status:* Open

### P3 — T9 and T10 point at the wrong places: the owner's reason line, the instant gate and the cancel action live elsewhere, and the AG-45 Hub tests are dropped (major)

*Where:* plan section 7.1, T9 and T10; section 3.1, row AG-45; section 6,
steps 5 and 6;
`crates/roym_web/ui/src/screens/listings.ts:204-262`,
`crates/roym_web/ui/src/listings/as_listed_section.ts:32-46`,
`crates/roym_web/ui/src/screens/messages.ts:950-996`.

*Problem.* Four facts differ from what the plan assumes.

1. The AG-45 row already names two Hub test files:
   `listings/as_listed_section.test.ts` and `listings/editor.test.ts`. The
   plan's AG-45 line lists unit and service tests only, and T10 adds the
   new Hub cases to `screens/listings.test.ts`. The Test column would lose
   two files that exist.
2. "The Hub's `instant` gate on signing" is built in
   `listings/as_listed_section.ts:32-46`. Its test file
   (`as_listed_section.test.ts`, 68 lines) stubs `signing-status` as
   `installed` in all three cases and never as not installed. The new case
   belongs there, not in `screens/listings.test.ts`.
3. "The owner's reason line" is drawn from `asListedReasonText`
   (`screens/messages.ts:950`), for six reasons. `screens/listings.ts`
   draws only one line (`cap-reached`). No Hub test mentions any of the
   reason keys. T10 case (2) says "shows the reason from
   `deal.as-listed-status`", which is the `listings.ts` line only. It would
   leave five reasons without a test, and the plan does not say where they
   are drawn.
4. There is no single node method for "cancel every unconfirmed instant
   agreement". The Hub loops over `status.open_instant` and calls
   `agreement.cancel` once for each (`listings.ts:236-247`); the agreements
   doc says the same ("each through `agreement.cancel`"). So T9's first test
   ("the method `listings.ts:204` calls; step 5 finds its name") has no
   method to find. At the service level it is "`agreement.cancel` on each
   agreement of `deal.as-listed-status`.open_instant, then
   `deal.as-listed-status` has an empty `open_instant` and the cap line is
   gone". The Hub test is the loop (one call per id, a refused one is not
   counted).

T9's second test has the same problem. It says "an instant quote for a
slot whose last seat is gone ends `not-formed`, `slot-taken`, with no
confirm-first fallback". The `as_listed_owner.rs` fixtures answer an
enquiry; they have no scheduling stub, and the `not-formed` end comes from
the writer's decision (AG-14), not from the answer. So the test as written
either needs the hooks stub or repeats `losing_the_last_seat_ends_not_formed`.
The AG-45 part that is not tested is only the first half: the owner's node
answers with an instant quote although the slot is full (no `waiting`, no
confirm-first).

*Failure case.* The implementer adds the cases to `screens/listings.test.ts`
as the plan says. The reason lines stay untested, the two named Hub files
are not in the Test column, and the cancel test cannot find its method.

*Fix.* Rewrite T9 and T10 with the real surfaces: (a) `as_listed_section.test.ts`:
one case with `signing-status` not installed, `instant` disabled and the note
shown; (b) `messages.test.ts` (or a new sibling test file): one case per
reason of `asListedReasonText`, or a table case over the six keys, and the
counterparty's waiting line (`messages.ts:996`, `deal_state: accepted`);
(c) `listings.test.ts`: the cap line and the cancel loop (one
`agreement.cancel` per id, a refused id is not counted); (d) service: the
cancel of each `open_instant` id through `agreement.cancel`; and the
answer-with-full-slot case defined at the answer step only (instant quote
issued, `deal.as-listed-status` has no `waiting`), with the scheduling stub
of `hooks_fake.rs` if the fixture needs it. Keep the AG-45 Hub tests
(`as_listed_section.test.ts`, `editor.test.ts`) in the Test column. Check
`messages.test.ts` length before adding (the 800-line rule).

*Status:* Open

### P4 — T6 needs a stub that can refuse only the second `confirm`; the plan says no harness change (minor)

*Where:* plan T6 and D-C8-9;
`crates/roym_transaction/src/app/tests/hooks_fake.rs:120`
(`refuse_confirms`), `:130` (`fail_call`).

*Problem.* `FakeScheduling::refuse_confirms(code)` refuses **every**
`confirm`. T6 needs "confirms the first and refuses the second".
`fail_call(method, nth)` fails the nth call with a transport error, which is
a different path (a fault, not a refusal). D-C8-9 says "no new harness".

*Failure case.* The implementer uses `refuse_confirms`; the first confirm is
refused too, so the test does not exercise "release a confirmed one" and
passes for the wrong reason (the existing `refused_confirm_stub_ends_not_formed`
already shows that).

*Fix.* Say in T6 that `hooks_fake.rs` (304 lines) gets one small method, for
example `refuse_confirm_number(n, code)`, and that the test asserts
`is_released` for both commitments, the confirmed one too. Change D-C8-9 to
"no new test host".

*Status:* Open

### P5 — T7 says the half is "not filed", but the code files it as a refused card (minor)

*Where:* plan T7; `crates/roym_transaction/src/app/sync.rs:489-500` and
`:523` (`validate_receipt_against_quote`, `refuse_card`).

*Problem.* A half whose `terms` differ gets the reason "terms differ from
the quote". The card is **stored**, marked `refused`, with that reason
(`FileCardResult { filed: true, refused: true }`). The plan says "`transaction.sync`
does not file it".

*Failure case.* The implementer asserts that the card is absent, and the
test fails. Or the implementer asserts nothing about the reason and the test
would also pass for any other refusal.

*Fix.* T7 asserts: the card row exists, `refused` is true, `reason` is
"terms differ from the quote"; the agreement row and the progress snapshot do
not change; nothing is countersigned. Make the change by one field of
`terms` (as the plan says) so the other checks of that function (`names the
wrong parties`, `accepted after the quote expired`) are not what fails.

*Status:* Open

### P6 — T4 does not say which card's `deal_state` is "still quoted", and half of T4 already exists (minor)

*Where:* plan T4; `crates/substrate/tests/roym_deal_e2e.rs:517-605`.

*Problem.* The existing test already accepts version 2 and asserts
`agreed` with `pair.state == complete` (lines 576-604). The only missing
assert is "the deal is still `quoted`". The plan says "on both nodes" but
not on which card. After the refusal, X's agreement row for **version 1** is
`not-formed` with `quote-superseded` (the test asserts it at line 591), so
"the deal is not-formed" is also true for version 1.

*Fix.* Say: after the refusal shows, `transaction.thread` on X and on Y shows
the version-2 quote card with `deal_state: "quoted"`, while the version-1
card shows the refusal. Drop "then accept the latest version" from T4: it is
already there.

*Status:* Open

### P7 — New test files need a `mod` line, and T2 can go in the file that already holds its sibling test (minor)

*Where:* plan T2, T7, T9, section 7.3.

*Problem.* `crates/roym_transaction/src/app/tests/refuse.rs` lists its
children (`mod helpers; mod version; mod writer;`), so `refuse/terms.rs`
needs `mod terms;`. The same holds for `presentation/tests/outcomes.rs`
(`presentation/tests.rs` is 657 lines) and for a sibling
`as_listed_cancel.rs` (`app/tests.rs`). The plan does not say so. For T2,
`presentation/tests.rs` already has the helpers (`setup_test_deal`,
`sign_test_record`) and the nearest test
(`presentation_without_disclosures_shows_kinds_and_outcomes`, which already
asserts that `claimed` and `reason` reach the view). About 70 more lines
stay under 800.

*Fix.* Name the `mod` line for each new file. For T2, prefer the existing
file unless it would pass 800 lines. Also say in T2 that the new case adds
only the three states the existing test does not hold
(`ended-unconfirmed` with `claimed: false`, `failed`, `cancelled`).

*Status:* Open

### P8 — T12 proves less than AG-50 says (minor)

*Where:* plan T12; AG-50.

*Problem.* AG-50 says "the driver adds no call from `conversation` to
`transaction`". T12 reads `roym.toml` and checks that `conversation` does
not list `transaction` in `depends_on`. That is a good check only if the
host refuses a sibling call to a service that is not declared, in both
builds. The plan states this ("a sibling call needs a declared dependency")
but does not show it. The driver itself runs in the substrate
(`WakeWorker`) and calls `transaction`; that is not a `conversation` call.

*Fix.* Step 1 finds the place where the declared-dependency rule is
enforced (WASM host and native shim) and names it in T12's comment-free
test description or in the AG-50 Test column. If it is not enforced in the
native build, add a source check instead: no `transaction` call target in
the `conversation` crates. Also name the test so it says what it checks
("a sibling call to `transaction` is not declared").

*Status:* Open

### P9 — The living-doc list misses statements that M08C made false (minor)

*Where:* plan section 9; agreements doc, "Close-out" list
(`change.md:1537-1541`); `docs/roym-integrated-experience-spec.md:541-546`,
`:564-567`; `docs/planning/changes/roym-extensions/change.md:165`.

*Problem.*

1. The close-out list names "the cancellation and booking rules". The plan
   handles booking. The cancellation rule (`:564-567`: "only while all
   commitments remain open (`promised`)") is not in section 9. AG-9 also
   allows `payment-requested`.
2. The state diagram (`:541-546`) shows `ended-unconfirmed` as an end of the
   agreement. `AgreementState` has only `agreed`, `closed`, `cancelled` and
   `not-formed` (`progress.rs:39`). `ended-unconfirmed` is a commitment
   state. The plan only adds `accepted` and `declined`; it must also fix
   this, or the diagram stays false.
3. The close-out list names "the `transaction` service's collections **and
   the event log**" for the architecture doc. Section 9 names collections
   only.
4. "barter": the grep of step 9 covers the seven living docs. But
   `roym-extensions/change.md:165` says "a barter with a booking on each
   side", two lines above the `:167` sentence that step 8.2 edits, and the
   Differences row for the term says "Overview Terms and text". It is not
   in the plan.

*Fix.* Add the four items to section 9 and step 8/9.

*Status:* Open

### P10 — File and line references are already stale on this branch (minor)

*Where:* plan sections 2.2, 3, 8, 9.

*Problem.* The plan says it read `main` at `79344661`, and step 0 tells the
implementer to re-read after C7b. Several lines have moved already: `progress_ops.rs:238` is `:241` and
`:92` is `:98`; the backlog rows are 2 lines lower (`:690` is `:692`,
`:666` is `:668`, `:672` is `:674`, `:716` is `:718`); the developer guide's
`deal_state` sentence is `:2703`, not `:2691`; the integrated spec's
"`request`" record name is `:874`, and `:862` is now the "### Languages"
heading. The plan also says "13 rows" have a gap (section 2.1 Q2): there are
12 (11 AG rows and EX-4); there are 13 new tests.

*Fix.* In the plan, name the function, constant or text instead of a line
number where the doc will change, or add one line "line numbers are for the
C7b tree; re-check each in step 0". Fix the count.

*Status:* Open

### P11 — The plan's decision ids `D-C8-1` to `D-C8-9` collide with ids that already exist (minor)

*Where:* plan section 5; integrated spec; agreements doc `change.md:1538`;
EX-6.

*Problem.* `D-C8-2` and `D-C8-14` are decisions of an older slice, cited by
the agreements doc's close-out list and by EX-6 ("`D-C8-2` in the integrated
spec"). The mailbox change doc also has `D-C8-1` to `D-C8-26`. A search for
"D-C8-2" finds three different things.

*Fix.* Use another prefix in this plan, for example `D-M08C8-1`. The plan
references them only inside itself.

*Status:* Open

### P12 — T11's second and third test are one code path (minor)

*Where:* plan T11;
`crates/roym_transaction/src/app/as_listed_ops/accept.rs:82` and `:214-225`.

*Problem.* The counterparty's node treats "a held decline" and "a withdrawal"
the same way: one `declined()` function (pointer `declined_at_secs` or a
decline row) shows the quote as a normal quote with the reason `withdrawn`.
Two tests named "declined" and "withdrawn" suggest two paths.

*Fix.* Either set up the two storage forms (pointer field and ledger row)
as two cases of one test, or write one test and say why. State the
expected reason code in the test description.

*Status:* Open

### P13 — Section 2.2 point 4 miscounts the wrong names of AG-49 (minor)

*Where:* plan section 2.2 point 4; section 3.1, row AG-49.

*Problem.* The AG-49 row names seven tests. Six do not exist
(`a_missing_offering_declines_offering_withdrawn`,
`a_withdrawn_offering_declines_offering_withdrawn`,
`an_expired_consent_declines_consent_expired`,
`a_changed_offering_answers_from_current_version_with_quote`,
`a_quote_issued_after_consent_valid_until_is_shown_consent_expired`,
`a_declined_enquiry_is_never_auto_accepted`). The plan says "five names" for
AG-49. Section 3.1 maps three of them to real tests and gives the other two to
T11, which is right; the text should say six, not five.

*Fix.* Correct the count in 2.2 point 4. In step 1, say that the AG-49 and
AG-45 Test columns get the real names plus the new names of T9 and T11 (so
the cancel test is written once, under the name T9 gives it).

*Status:* Open
