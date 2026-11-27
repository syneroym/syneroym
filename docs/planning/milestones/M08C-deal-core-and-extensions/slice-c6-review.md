# M08C slice C6 — review

*Reader: the developer who fixes the C6 plan or code, and the project
owner. This file holds every review finding for slice C6 (parts C6a and
C6b). Plan review findings are P1, P2, …; code review findings are R1, R2,
…. Ids are never renumbered.*

## Plan review round 1 — Claude Sonnet 5.5, 2026-11-26

Checked [slice-c6-implementation-plan.md](slice-c6-implementation-plan.md)
on branch `feat/m08e-c6-barter-statement` at `38fbfeb1` (on top of `main` at
`edd71c11`). This is the main checkout, not a worktree (`git rev-parse
--git-dir` and `--git-common-dir` are both `.git`). `git pull` said "Already
up to date". The plan was written by Claude Opus 5.5. This review is by
Claude Sonnet 5.5.

Read: AGENTS.md ("Writing for readers" and the code rules);
session-strategy.md ("The review file"); the plan (all 1,509 lines);
task.md and status.md of M08C; both change docs in CHANGE_DOCS
(roym-agreements: AG-22, AG-23, AG-27 to AG-30, AG-33, AG-46 to AG-48,
AG-52, "Settlement records", "Mixed exchanges and declared values", "Event
log and statement", "Methods"; roym-value-exchange: the VX rows the plan
names). task.md's C6 section names the same two change docs, so no change
doc is missing from CHANGE_DOCS. There is no earlier round of this file. I
also read the C4 and C3c review files for the kinds of findings earlier plan
reviews raised, and I read the code that the plan's design stands on:
`progress_ops.rs`, `payment_ops.rs`, `fulfilment_ops.rs`, `parties.rs`,
`sync.rs`, `sync/receipts.rs`, `decide.rs`, `backup.rs`, `api.rs`,
`router.rs`, the test host, `deal/transition.rs`, `progress.rs`, and the
Hub's `messages.ts`, `quote_terms.ts` and `money.ts`.

**Summary: 15 findings: 0 blocker, 3 major, 12 minor. 0 open (15 fixed). Closure check completed 2026-11-26; P15 found in the second closure check and fixed in 87763d94.**

### What was checked and found correct

- **File and line links.** Every `path#Lnn` link in the plan points to an
  existing file and a line that holds what the plan says (I checked each one
  with a script, then read the doubtful ones). The two counts the
  plan gives are right: `transaction` routes 47 methods
  ([api.rs:31](../../../../crates/roym_transaction/src/api.rs#L31): 8 pings,
  5 + 7 + 4 + 6 + 4 + 3 + 3 enquiry/quote/deal/agreement/payment/fulfilment/
  step rows, 7 `transaction.*` rows), and `scenario_144` lists 44 of them and
  misses `quote.from-listing`, `deal.as-listed-status` and
  `transaction.drive`. The highest parity scenario is 266.
- **The data layer.** The real layer supports `$gte`
  (`crates/data_db/src/filter.rs:280`); only the test host lacks it
  ([testing/data_layer.rs:224](../../../../crates/roym_core/src/testing/data_layer.rs#L224)).
  `create` in the test host refuses the whole call when one id exists and
  names that id, as the plan says (D-C6-3). The query has no order. `paging`
  reads 500 rows a page.
- **The rows the log needs are held on both nodes.** `AgreementRow` has the
  plain `terms`, `state`, `seq`, `commitments`, `agreed_at_secs` and both
  receipt halves ([app.rs:170](../../../../crates/roym_transaction/src/app.rs#L170)).
  A snapshot row has the envelope, but no `issued_at_secs` (only
  `stored_at_secs`). `CommitmentProgress` has `halves: Vec<String>`, so the
  settling half of a final commitment can be named from a snapshot.
- **Final states cannot move.** `step_fulfilment_final` and
  `step_currency_final` never leave a final state for a half or a tick
  ([deal/transition.rs:85](../../../../crates/roym_core/src/deal/transition.rs#L85)).
  So "one event per final commitment state" (AG-27) is well defined: a
  commitment cannot give two final events.
- **The two ends of a deal.** `agreement.cancel` and `OtherFailed` can put
  several commitments into a final state in one snapshot; "the lowest-`seq`
  held snapshot that shows it" handles that.
- **Method tables.** `router.rs` has `agreement.` and `deal.` rows and no
  `eventlog.` or `statement.` row, so the two new rows are needed. No other
  routing table or method list in `crates/` or `apps/` names the methods
  (I searched for `agreement.progress` and `fulfilment.sign` in all non-doc
  files): the list is `api.rs`, `router.rs`, `contract.rs:83`, `scenario_144`,
  and the shape tests. The `roymctl` layout (`roym transaction agreement …`)
  exists as the plan describes it.
- **Both change docs are the right ones** for every criterion in section 3.
  I checked AG-23, AG-27 to AG-30, AG-33, AG-46 to AG-48 and AG-52 word for
  word against the plan's text. The Deviations list in section 9 covers the places
  where the plan differs from the agreements doc (Q4, Q5, Q6, D-C6-3 to
  D-C6-7, D-C6-11), apart from P1, P2 and P5 below.
- **Hub words.** No Playwright spec, demo story, shell script or Hub unit test
  selects a button by its English text "Accept", "Quote this request" or "Send
  quote" (I searched with `git grep`). So the rename in D-C6-15 touches the
  catalogue and the Hindi file, not other tests, as the plan says. Story 40
  keeps passing.
- **Size rules.** The files near the cap (`backup.rs` 788, `app.rs` 734,
  `payment_ops.rs` 733, `backup/sections.rs` 732) are named, and the plan
  puts new code in new files. `dual_build_parity/transaction.rs` (821) and
  `helpers.rs` (1,266) are on the oversized list; the plan does not grow them.
  The Q9 limit arithmetic is right for the rule as task.md states it (15% of
  the `main` size). The `transaction` file on disk in `target/` is 2,714,220
  bytes, 417 fewer than the plan's figure; a different build, and not
  important.
- **"How this is shown to an end user"** has six bullets for C6b, in simple
  words, and a developer part. C6a says plainly that the Hub shows nothing new.
- **Step order.** Each step builds before the next: the method rows (C6a step
  8) come after every function they call; C6b's pure parts come before the
  screens that call them.

### Views on the owner questions

The owner has not answered section 2.1. My view of each, for the record:

- **Q1 (two PRs).** Agree. The node part has a result a reviewer can run with
  `roymctl`; the Hub part needs the node API. Note that P1 and P2 change what
  C6a must define before the Hub panel is written.
- **Q2 (agreement panel in the thread, not a new tab).** Agree. A person
  settles a deal where they talk about it. But see P1: on the counterparty's
  node the panel cannot show the person's own action until the other node
  answers, and the plan must say how the panel looks in that time.
- **Q3 (a "Statement" tab).** Agree. A statement is not a backup.
- **Q4 (saved terms in a `transaction` row).** Agree. See P9: the section
  needs a constant in `roym_core` and a validation of the imported rows.
- **Q5 (the log is not in the bundle).** Agree. A row the import cannot check
  is worse than a rebuilt row. Two small corrections to the Q5 text: the claim
  that (a) keeps `backup.rs` from growing is not right (the import now calls
  the derive step and reads one more section, and the file has 12 free
  lines); the plan has the right fallback in step 7. And `recorded_at_secs`
  after an import is the import time for every row, so the first
  `eventlog.list` after a restore pages all rows as new. The plan names this
  loss; keep it.
- **Q6 (the Hub fills `counterparty_name`).** Agree. No new service edge for a
  display name.
- **Q7 (an ordinary text message).** Agree. (b) would change `conversation`,
  which C7 also changes.
- **Q8 (`INR` in both forms).** Agree. The English placeholders still say "e.g.
  EUR" (`quote-currency-placeholder`, `listing-placeholder-currency`); leave
  them, they are examples.
- **Q9 (+15% for `transaction`).** Agree, as an owner decision. task.md
  gives +15% for "new record code" and C6 adds no signed record. The owner
  should say in the answer that C6a uses the +15% limit although it adds no
  record, because the +40 KB limit would not fit a statement type, a log
  type and a derive step.
- **Q10 (a third case in story 50).** Agree. But see P3: the story's totals
  line only holds if the form's default declared values are right.

### Findings

#### [x] P1 — Only the quote author's node moves a commitment, so the other node's log, statement, panel and tests all wait for the other node (major)

*Where:* point 5 of section 2.2 ("The writer moves the commitment in
`progress_ops::transition_at` … called from the local methods"), D-C6-3
("Where it runs"), D-C6-7 ("a commitment settled seconds ago through a local
method is in the export without waiting for a sync"), D-C6-13 ("The panel
redraws after each action from `agreement.get`"), D-C6-6, parity 271, the
two-node test `both_sides_close_a_three_commitment_deal`, AG-29.

*Problem:* The plan treats the local settlement methods as if they moved the
caller's own commitment state. They do not, unless the caller is the quote
author. In both settlement methods the transition is guarded:
`if owner == agr.quote_author_did` in
[payment_ops.rs:325](../../../../crates/roym_transaction/src/app/payment_ops.rs#L325)
and `maybe_transition_author_fulfilment` in
[fulfilment_ops.rs:264](../../../../crates/roym_transaction/src/app/fulfilment_ops.rs#L264).
Only the quote author signs progress snapshots. The counterparty's
`AgreementRow` changes only when it files the author's next snapshot
([sync/receipts.rs:277](../../../../crates/roym_transaction/src/app/sync/receipts.rs#L277),
`apply_newer_snapshot`). So when Alice (the counterparty) calls
`agreement.close` or "Received":

- her half is signed and sent, but her `agreement.get` still shows the
  commitment as `promised` until Bob's node is online, files her half,
  writes a snapshot, and she syncs it;
- her derive step has nothing to derive, because the plan derives
  `commitment-settled` only from a final commitment state (D-C6-3: "the
  commitment is `fulfilled` or `acknowledged`"), so her `statement.export`
  does not list the commitment she settled seconds ago. D-C6-7's sentence is
  true only for the quote author;
- the panel (D-C6-13) redraws from `agreement.get`, so after Alice taps
  "Received" it shows the same button again, and "You said you delivered"
  (which reads `claimed` from the snapshot) does not appear for her own claim.
  A second tap gives `AlreadyRecorded`, so no harm is done, but the screen
  looks as if nothing happened;
- the two-node test says "Both nodes reach `closed`" right after the calls
  with no step for the round trip, and C6a step 11 / 7.5 do not say it must
  sync both ways.

If Bob is offline for a day, Alice's own statement is wrong for that day, and
that is the one statement that proves what she received.

*Failure case:* Alice taps "I received everything" for the wheat and the
lesson. `agreement.close` answers with two record ids. She opens
"Statement" for this month: no rows. She opens the deal: both lines still say
"promised".

*Fix:* Decide this in the plan; do not leave it to the implementer.

1. State the rule in section 2.2: only the quote author's node moves a
   commitment at once. The other node moves when it files the author's
   snapshot. Correct point 5 and D-C6-7.
2. For the panel, show a person's own signed half that the snapshot does not
   yet show. The node holds it: `fulfilment.get` and `payment.get` return the
   halves by role. Either the panel calls them for a commitment that is still
   open, or `agreement.get` gains a read-only field per commitment ("this
   node has signed its half: record id"). Draw it as "Sent. Waiting for the
   other person's node." with no button. Add this to the Hub tests (the
   button disappears after the action although the snapshot is old).
3. For the statement, choose one and write it in D-C6-3: (a) the entry waits
   for the snapshot, as now, and the plan says so in the user bullets and in
   a deviation; or (b) when the node holds the receiver's own half and the
   snapshot is older, the derive step makes `commitment-settled` from that
   half (the receiver's half alone settles a commitment, AG-46, so the rule is
   safe). I prefer (a): it keeps one source for the state, and the log never
   shows something the agreement screen does not. The cost must be written
   down.
4. Test 7.5: after each `agreement.close`, both nodes `transaction.sync`
   (or wait for the driver), then compare. Add one service test that checks
   the counterparty's statement before and after the author's snapshot is
   filed.

*Status:* Fixed (c8601f11).

#### [x] P2 — "A repeated `agreement.close` returns the same records" is false after the first call has made the commitments final (major)

*Where:* D-C6-6 ("A commitment that is already final is put in `skipped` …
So a repeated call returns the same records (AG-13)"; the check
`agreement-not-open`), section 3 (AG-46 row), parity 272, service test
`close.rs` ("a repeat returns the same records").

*Problem:* The plan's rules contradict each other and the code.

- A receiver's half settles the commitment. On the quote author's node the
  transition runs in the same call, so the commitment is final when the call
  ends. A second `agreement.close` then finds it final, and by the plan's own
  rule puts it in `skipped` with `final`, with no record. The records list is
  empty, not "the same".
- When every commitment became final, the agreement became `closed`. The plan's
  first check refuses an agreement that is not `agreed` with
  `agreement-not-open`. The underlying methods refuse it too, and they do it
  *before* they look for the stored half: `sign_at` calls
  `require_open_agreement` at
  [fulfilment_ops.rs:345](../../../../crates/roym_transaction/src/app/fulfilment_ops.rs#L345)
  before the `existing` check at `:117`, and `acknowledge` refuses with
  `agreement-terminal` at
  [payment_ops.rs:512](../../../../crates/roym_transaction/src/app/payment_ops.rs#L512)
  before `check_acknowledgement_version`. So a repeat after the last
  commitment ended is an error.
- Only a claim (a giver's half) stays non-final, so only a repeated claim
  returns the stored record (`AlreadyRecorded`).

So AG-13's "the same records" holds for claims only. Parity 272 (written
for receiver-side close of three commitments) would fail, or the test would be
written to the weaker rule without anyone deciding it.

The retry case matters for the Hub. The plan itself says "If one signature
fails … a retry makes only the missing ones". A double tap, or a retry after a
timeout when the call did succeed, gives a refusal for the last close of a
deal.

*Failure case:* Bob (the quote author) taps "I received everything" on the
last open side. The call succeeds and the agreement is `closed`. The Hub shows
a network error (the reply was lost), and Bob taps again. He gets
`agreement-not-open`.

*Fix:* Write the exact repeat rule, and make the repeat safe.

1. A commitment in a final state is `skipped` with reason `final` (as now),
   and nothing else. A commitment whose half this node already signed and
   that is not final returns that half in `records` (as now). Say that
   settling halves of a repeat are not returned, and change the sentence "so
   a repeated call returns the same records".
2. An agreement that is `closed` is not an error: every listed id (that is in
   the agreement and on the caller's side) is `skipped` with `final`. Refuse
   `agreement-not-open` only for `cancelled`, `not-formed` and an agreement
   that is not yet formed.
3. Rewrite scenario 272 and the AG-46 test row: a repeated close of three
   commitments gives `records: []` and three `skipped`; a repeated claim
   returns the stored records.
4. Record AG-46's sentence "returns the same records" as a deviation
   (section 9), with the reason.

*Status:* Fixed (c8601f11).

#### [x] P3 — The plan does not say what the author's service row is worth by default, so every ordinary offer would show a false difference (major)

*Where:* D-C6-9 (the rows, "declared value"), D-C6-10 (the totals block),
"How this is shown to an end user" (C6b), Q10, Playwright D1/D4, the `sendQuote`
helper (point 18).

*Problem:* Today `buildDealTerms` gives the author's service the same
declared value as the money:
`declared_value_minor: fields.amountMinor` for `c1`
([quote_terms.ts:42](../../../../crates/roym_web/ui/src/screens/quote_terms.ts#L42)),
and the money row has the same amount. So a simple offer ("lesson for ₹500")
has equal totals today, and no card says there is a difference. D-C6-9 gives
each row its own "declared value" but does not say what the form puts in it
when the person types only the scope and a price (the AG-48 case: "a quote
sent from a listing with only the scope typed"). If the service row starts at
0, then every simple offer shows "You receive ₹500 more than you give" to the
counterparty and "You give ₹500 more than you receive" to the author. That is
a false statement about every deal that is not a mixed one, and it appears
beside a money total. The plan also does not say what the empty form contains:
`sendQuote` (story 40, 50, `roym-hub.spec.ts`) fills `.quote-amount-input`,
`.quote-currency-input` and `.quote-payee-input` on a fresh form, so the fresh
form must already hold one author row and one money row.

*Failure case:* Story 40 (Alice asks, Bob quotes ₹500 for a lesson, Alice
accepts) is unchanged in its steps, but Alice's quote card now says "You
receive ₹500 more than you give".

*Fix:* In D-C6-9, add:

1. The fresh form has one author service row and one "They give: Money" row
   (these keep the classes of point 18), and the service row's declared value
   starts as the money row's amount and follows it until the author types in
   it. State this as the default, the same as today's value.
2. Say how the declared value of a goods or service row of the author is
   shown when the author does not want to value it: the field is not optional
   (every commitment has a value, AG-23), so a hint under it says "What this
   is worth in <currency>. Used only for the totals."
3. Add a Hub unit test (`quote_form.test.ts`): a form with only scope and
   amount builds equal totals and the card shows no difference line.

*Status:* Fixed (c8601f11).

#### [x] P4 — The derive step may pick a changed half or a local time for two of the times it calls "the same on both nodes" (minor)

*Where:* D-C6-3 (the table: `agreement-formed`, `commitment-settled`),
D-C6-4, the `SettlingHalf` and `FormedFact` inputs.

*Problem:* The plan says both nodes give `agreement-formed` and
`commitment-settled` the same time because "both times come from records both
nodes hold". Two details break this.

- `payments` rows hold a **list** of halves per party
  (`counterparty: Vec<ReceiptHalf>`, `quote_author: Vec<ReceiptHalf>`,
  [app.rs:262](../../../../crates/roym_transaction/src/app.rs#L262)), because a
  signer may correct an acknowledgement with `supersedes`. "The receiver half's
  `issued_at_secs`" is not one value when a correction exists. The snapshot
  names the half that moved the commitment (`CommitmentProgress.halves`); the
  derive must use that half, or the later correction changes the time of an
  entry between two derives (and between the original node and a restored
  node).
- `AgreementRow.agreed_at_secs` is not always the countersigned half's time.
  The counterparty sets it from the quote author's half when it holds it, but
  falls back to the snapshot's `issued_at_secs`, or to `ctx.now` (the local
  clock): see
  [sync/receipts.rs:305](../../../../crates/roym_transaction/src/app/sync/receipts.rs#L305).
  Crash recovery sets it to `cs_row.created_at_secs`
  ([decide/recover.rs:245](../../../../crates/roym_transaction/src/app/decide/recover.rs#L245)),
  also a local time. The plan's `FormedFact` says "`agreed_at_secs` + the two
  receipt half ids".

*Failure case:* Alice's node files the first snapshot before it holds Bob's
receipt half and stores its own clock as `agreed_at_secs`. Her statement shows
a different `agreed_at` than Bob's, and a deal agreed at 23:59 appears in
different months on the two statements.

*Fix:* In D-C6-3, take `occurred_at_secs` of `agreement-formed` from
`quote_author.issued_at_secs` (the countersigned half) and nothing else; if
the node does not hold it, the entry is deferred (`complete: false`), like
the other missing facts. Take the settling half from the half named in the
snapshot's `halves` for that commitment and role. Add two unit cases: a
corrected acknowledgement does not move `occurred_at_secs`; a row whose
`agreed_at_secs` differs from the half's time uses the half's time.

*Status:* Fixed (c8601f11).

#### [x] P5 — `open_at_end` uses the times that D-C6-4 says may differ between the two nodes (minor)

*Where:* D-C6-4 ("The statement uses only the two times that are equal, so
both parties' statements still match"), D-C6-7 (`open_at_end`: "no final
commitment entry that occurred before `to_secs`"), 7.1 (statement tests).

*Problem:* The two statements contradict each other. `open_at_end` asks
whether a final entry occurred before `to_secs`. "Final entry" includes
`commitment-failed`, `-ended-unconfirmed` and `-cancelled`, whose
`occurred_at_secs` is the lowest held snapshot's time and can differ between
nodes when a snapshot is lost (D-C6-4). Near the end of a period, one node's
`open_at_end` can list a commitment that the other node's does not. The claim
"both parties' statements still match" is then false for `open_at_end`.

*Failure case:* A commitment fails at 23:58 on the last day of the month. Bob
holds the snapshot from 23:58. Alice lost it and holds the next one, from
00:03. Bob's month shows no open commitment. Alice's month lists it as open
at the end.

*Fix:* Either (a) say that `open_at_end` is a view of this node, test only the
two equal times for matching, and write the difference in the deviation
(D-C6-4's statement then says "the rows match; `open_at_end` may differ"), or
(b) define `open_at_end` only through `commitment-settled` and
`agreement-formed` and list a failed or cancelled commitment as not open when
the node holds *any* final entry for it, whatever its time. (b) is simpler to
explain and matches the agreements doc ("agreed, not final at `to_secs`") for
the common case. Pick one and add the test case.

*Status:* Fixed (c8601f11).

#### [x] P6 — A statement total above 2^53 reaches the Hub as a wrong number (minor)

*Where:* D-C6-7 (`totals`, `statement-total-overflow` at 9.2 × 10^18), point 15
of section 2.2, D-C6-10, D-C6-14 ("Download JSON").

*Problem:* The plan sees the JavaScript limit for the quote totals (point
15, `BigInt`) but not for the statement's totals. `sum_minor` is an `i64`
that the service writes as a JSON number. Each declared value is at most
10^15, which JavaScript reads exactly (2^53 is about 9.007 × 10^15). The sum of
ten large values is not exact after `JSON.parse`. The Hub then draws it, and
"Download JSON" writes it again with `JSON.stringify`, so the file holds a
wrong number with no error. The service's guard (9.2 × 10^18) is far above
what every JSON reader can hold.

*Failure case:* A year of large deals for one currency adds up to
`12345678901234567` (about 1.2 × 10^16). The service writes that number.
`JSON.parse` in the Hub reads it as `12345678901234568`. The screen and the
JSON download both show the wrong total, with no error.

*Fix:* Make the wire exact for every reader, which costs nothing in practice:
the service refuses a group sum above 2^53 − 1 (9,007,199,254,740,991) with
`statement-total-overflow`, instead of 2^63. Change D-C6-7, the backlog row
text ("above 2^63"), and the unit test. Then no consumer needs `BigInt` for
the statement. Say in D-C6-14 that the Hub still computes its own totals from
the rows with `BigInt` only where it draws a quote (D-C6-10), not here.

*Status:* Fixed (c8601f11).

#### [x] P7 — `eventlog.list` does not run the derive step, so it is empty right after a local settlement (minor)

*Where:* D-C6-3 ("Where it runs"), D-C6-5, parity 267, the user bullet for
`roymctl roym transaction statement`.

*Problem:* The derive step runs at the end of `sync_direct`, in
`transaction.import`, in `eventlog.rebuild` and in `statement.export`. It does
not run in `eventlog.list`. A developer who runs `roymctl … agreement close`
and then `roymctl … eventlog list` sees nothing until a sync or the Hub's next
redraw (the driver does not run for a local settlement). Parity 267 says "the
writer settles both commitments; `eventlog.list` gives the same entries in
both builds"; it works only if the test syncs first, and the plan does not say
so. This also makes the "How a developer can see it work" bullet misleading.

*Failure case:* `agreement close` then `eventlog list` prints `entries: []`.
The developer thinks the log is broken.

*Fix:* Let `eventlog.list` run the marker-gated derive over every agreement
first, as `statement.export` does (D-C6-7). AG-28 still holds: the derive
step reads `event_log_state`, never `event_log`. Say so in D-C6-5, and add a
service case: settle, `eventlog.list` with no sync, entry present (on the
quote author's node; see P1 for the other node).

*Status:* Fixed (c8601f11).

#### [x] P8 — The derive step must not write its marker after a failed create (minor)

*Where:* D-C6-3 ("It writes the marker after the creates … A refused create
is a repeat and is not counted"), 7.4 (`fail_write` on `event_log`).

*Problem:* The plan separates "a refused create" (a repeat) from an error, but
only says what a repeat does. It does not say what a *failed* create does to
the marker. If the marker is written with `complete: true` after one create
failed, that agreement's entry is lost until `eventlog.rebuild` (the marker
hides it from every later sync). The test in 7.4 ("a `fail_write` on
`event_log` does not fail `transaction.sync`") checks the sync, not the
repair. `sync_direct` also returns early with an error before the derive step
when filing a message fails, so the derive never runs in that pass; the next
sync is the repair, which is fine, but the plan should say it.

*Failure case:* The data layer fails one `create` in a full disk moment. The
sync passes. The marker says done. The `commitment-settled` entry never
appears, and no statement lists the commitment, until someone runs
`eventlog.rebuild`.

*Fix:* In D-C6-3: write the marker only when every create answered "created"
or "repeat"; on an error, write no marker (or `complete: false`). Change the
7.4 test: after the `fail_write` is removed, the next ordinary sync makes the
entry.

*Status:* Fixed (c8601f11).

#### [x] P9 — Backup: a `roym_core` constant, five hard-coded schema versions, and no check on the imported defaults (minor)

*Where:* Q4 and Q5, D-C6-16, section 4 (`transaction.export`/`import`), step
7, section 6 step 8 (the test list).

*Problem:* Four things the plan does not list.

- **A `roym_core` change.** Section names are constants in
  [crates/roym_core/src/backup.rs](../../../../crates/roym_core/src/backup.rs#L39)
  (`SECTION_*`), not in `roym_transaction`. A `deal_defaults` section needs
  `SECTION_DEAL_DEFAULTS` there, and the test list in the same file
  (`each_new_section_passes_check_integrity_on_its_own`, near line 450) lists
  every section. The plan's C6a touches no file in `roym_core` for this.
- **Hard-coded schema version 8.**
  [dual_build_parity/transaction_cards.rs:348](../../../../crates/roym_web/tests/dual_build_parity/transaction_cards.rs#L348)
  to `:352` assert `schema_version` 8 for five sections. A bump to 9 fails
  them. C3b met exactly this failure (status.md, C3b1: "1 failed on a hard-coded
  schema version"). Name the file in step 7 or 9.
- **Imported rows are not checked.** The bundle is signed by the owner, but
  every other section is re-validated on import. The plan says the texts follow
  `check_text` (D-C6-16) for `deal.defaults-set`, but says nothing about the
  import. A bundle with a 100 KB cancellation text would be stored and then
  refused by every later `deal.defaults-set` round trip and shown in the form.
  The import must run the same check as the method.
- **The bump itself.** A new optional section does not need a version bump.
  `validate_bundle_header` compares every declared section version to
  `SCHEMA_VERSION`, so a bundle that this build writes and this build reads
  always matches. A version 8 bundle (no `deal_defaults` section) would still
  import and give "no saved terms". The bump only makes every saved version 8
  bundle fail. The plan's own rule for a bump is a changed stored shape, and
  no stored shape changes here. If the owner wants the bump anyway, list the
  callers above. My view: do not bump.
- The `TransactionImportOutput` gets `log_entries_made`. The Hub's backup
  screen and `roymctl` read this output
  ([backup.ts:35](../../../../crates/roym_web/ui/src/screens/backup.ts#L35),
  `backup.test.ts:93`); the field is additive, but add it to the method shape
  file (`method_shapes/`) and the Hub's type.

*Failure case:* `mise run verify` fails in `dual_build_parity` on
`schema_version 9 != 8` after the bump, and the plan did not list the file.

*Fix:* Add the constant, the five asserts (or drop the bump), the import
check for `deal_defaults` rows, and the shape file to steps 7 and 9, and to
the "Deviations" list for Q5 if the bump stays.

*Status:* Fixed (c8601f11).

#### [x] P10 — The CSV: dates in UTC for a statement chosen in local days, and the formula guard covers only two columns (minor)

*Where:* D-C6-14 (period picker, CSV columns, "CSV formula guard"), Playwright
D5.

*Problem:* Three small gaps.

- The period is made of the Hub's **local** days (local midnight), but dates in
  the CSV are "ISO 8601 in UTC". In India (UTC+5:30) a settlement at 00:30 on
  1 December is in December's statement and the CSV says `2026-11-30T19:00Z`.
  The person checks the file against the screen and the dates do not match.
- The guard text says "a description or note". The CSV also has
  `counterparty_name`, which comes from the person's contacts (a name that
  another person wrote and the person saved), and `commitment_id`/`agreement`
  (written by the service, safe). The guard must apply to every text cell
  except the numeric `declared_value`, not to two named ones.
- The `description` column is a `LocalText` in the JSON. The plan does not say
  which text goes in the CSV. The original text binds (agreements doc); say
  "the original text".

*Failure case:* A contact is saved with the name `=HYPERLINK("http://…","Pay")`.
The CSV opens in a spreadsheet and the cell runs.

*Fix:* Write every date in the CSV with the local offset (`2026-12-01T00:30:00+05:30`)
or add both a UTC and a local column; apply the formula guard to every cell
of the string columns, and test `counterparty_name`; say "original text" for
`description`.

*Status:* Fixed (c8601f11).

#### [x] P11 — Step 11 moves test code that says "barter" into a file where the plan says no code may say it (minor)

*Where:* The "Name" paragraph at the top ("No code, message or doc may say
'barter'"), step 11 ("move `setup_barter_fixture` … to `roym_flow.rs`").

*Problem:* The moved helpers are `setup_barter_fixture`, `BarterFixture` and
`exchange_barter_fulfilments`
([roym_deal_e2e.rs:232](../../../../crates/substrate/tests/roym_deal_e2e.rs#L232)
to `:294`), and the file header says "barter/goods deal completion"
(`roym_deal_e2e.rs:9`). A plain move carries the word into the shared harness.
`templates.test.ts:527` also uses the string `barter-tokens` as an unknown
payment method (a test value, not a claim about the product).

*Fix:* In step 11, rename when moving (`setup_mixed_fixture`,
`MixedFixture`, `exchange_mixed_fulfilments`) and fix the header line. Say that
the rule covers product names and text a person reads, and keep the
`barter-tokens` test value, or rename it; the plan must say which.

*Status:* Fixed (ec790108).

#### [x] P12 — The plan says every refusal has `data.code`, but the code in `transaction` uses three shapes (minor)

*Where:* 7.11 ("every new refusal has a code in `data.code`"), D-C6-6,
D-C6-7, D-C6-16.

*Problem:* Most refusals in `transaction` put the code only in the message:
`Response::invalid_params("no-such-commitment")`. Some add `data` as a bare
string (`.with_data(json!("quote-superseded"))`,
[agreement_ops.rs:260](../../../../crates/roym_transaction/src/app/agreement_ops.rs#L260))
and some as `{"code": …}` (the signing errors,
[app.rs:315](../../../../crates/roym_transaction/src/app.rs#L315)). The plan
names codes (`no-such-commitment`, `not-your-side`, `bad-commitment-list`,
`agreement-not-open`, `bad-period`, `statement-total-overflow`) but not the
shape, and says "-32602 and a code". The Hub must tell `not-your-side` from
`agreement-not-open`, and the signing error (`{"code": "signing-not-enrolled"}`)
from both. An implementer will pick one at random.

*Fix:* Say in D-C6-6: the message is the code (as for the neighbours), and
`data` is `{"code": "<same>"}` for every new refusal; the Hub reads
`data.code`. Add one contract test per new refusal that checks both.

*Status:* Fixed (ec790108).

#### [x] P13 — The `reads()` log in the test host: `query_raw` is `unimplemented!()` there (minor)

*Where:* Step 1 ("a `reads` log … filled by `get`, `query` and `query_raw`"),
point 9.

*Problem:* In the test host `query_raw` is `unimplemented!()`
([testing/data_layer.rs:204](../../../../crates/roym_core/src/testing/data_layer.rs#L204)).
Adding a log line there does nothing: the call panics. Nothing in
`roym_transaction` calls `query_raw`. Also, the real data layer is not under
test in AG-28's service test: the test shows no *call* reads `event_log`, not
that the real store never does. That is what the criterion asks, but the
plan's wording ("every read") is wider.

*Fix:* In step 1, log `get` and `query` only and say so; leave `query_raw`
`unimplemented!()`. In the test comment (not the plan), say what the test
proves: no call through the host's read functions names `event_log`.

*Status:* Fixed (c8601f11).

#### [x] P14 — New Hub forms are not in the thread's "open work" list, so a redraw can drop what the person typed (minor)

*Where:* D-C6-13 (the note form, the close list), D-C6-9, D-C6-12, step 4 of
C6b.

*Problem:* The thread redraws on every event and waits only while an element
matching `OPEN_WORK` is inside it
([messages.ts:39](../../../../crates/roym_web/ui/src/screens/messages.ts#L39):
`.quote-form, .decline-dialog, .delete-dialog, .accept-mt-box`). The
comment above it says "An element inside the thread that holds the person's
input belongs here." The plan adds a settlement-note form, the "This will
sign:" list with checkboxes, and moves the quote form to `quote_form/`, but
does not say these join the selector. The panel redraws after each action,
and the event feed redraws it whenever a card arrives. A note that Alice is
typing is thrown away when Bob's snapshot arrives.

*Failure case:* Alice taps "Received", starts to write how the wheat was
handed over, and Bob's progress card arrives. The thread redraws and the
form is gone.

*Fix:* In step 2 and step 4 of C6b: keep `.quote-form` on the moved form, give
the note form and the close list a class (for example `.settle-dialog`,
`.close-list`), add both to `OPEN_WORK`, and add a Hub test in
`thread_redraw.test.ts` style: a redraw does not replace an open settle
dialog.

*Status:* Fixed (ec790108).

### Closure check — Claude Sonnet 5.5, 2026-11-26

I compared plan commit `c8601f11` with each finding and its fix text, and
searched the plan for old statements the fixes made false. Checked in the
plan file only; no code was run.

- **Closed and confirmed (11):** P1 (counterparty entries wait for the
  author's snapshot, "Sent. Waiting" panel state, two-way sync in the tests),
  P2 (closed agreement skips with `final`, repeat gives `records: []`,
  scenario 272 and `close.rs` rewritten), P3 (service row value follows the
  amount, hint text, equal-totals test), P4, P5 (option b, test added), P6
  (2^53 − 1 in D-C6-7, backlog row and test), P7, P8, P9 (constant, five
  asserts, import check, shape; the bump to 9 is kept, which my fix text
  allowed if the callers are listed), P10, P13.
- **No old statement left behind.** No "same records", "9.2", "2^63" or
  AG-13 repeat claim remains; the AG-46 row and section 9 deviations match
  the new close rule.
- **Reopened (3, all small):** P11, P12 and P14 above. Each is a missing
  sentence or test name in the plan, not a design problem. All three fixed
  in ec790108 (Name paragraph scoped; refusal_with_data and refusal contract
  tests named in step 8, 7.3, 7.11; thread_redraw test named in step 4 and 7.6).

### Second closure check — Claude Sonnet 5.5, 2026-11-26

I read the plan change in `ec790108` against P11, P12 and P14 and checked
the names it cites against the code on this branch. P11 and P14 are closed:
the "Name" paragraph is scoped, and the `thread_redraw.test.ts` case is named
in step 4 and in the Hub test list. P12 is closed in substance (the shape and
a contract test per refusal are named), but the fix brought in the finding
below.

#### [x] P15 — The P12 fix names a helper that does not exist and a refusal code the plan never defines (minor)

*Where:* Step 8 and 7.11 (`refusal_with_data`), 7.3 (`bad-deal-defaults`),
D-C6-6, D-C6-16.

*Problem:* Step 8 and 7.11 say every new refusal "returns `refusal_with_data`".
No function of that name exists in `crates/` (`git grep` finds only the plan),
and the plan does not list adding one. The real calls are
`Response::invalid_params(code).with_data(json!({"code": code}))`
([envelope.rs:62](../../../../crates/roym_core/src/envelope.rs#L62), `:50`).
Also the 7.3 test list names the code `bad-deal-defaults`, but D-C6-16 only says
the texts follow `check_text` and step 7 gives no code; the code list in
D-C6-6 does not have it either.

*Failure case:* The implementer searches for `refusal_with_data`, finds
nothing, and writes a new helper in one place or inlines the two calls in
another. The refusal test for `deal.defaults-set` then asserts a code that no
section of the plan defines, so the code is chosen at random (or the test
checks the message the existing `check_text` path already gives, which is not
`bad-deal-defaults`).

*Fix:* Either name the real two-call form, or add one small helper to a named
file (for example `app/refusal.rs`) as part of step 8. Add `bad-deal-defaults`
to D-C6-6's code list and say in D-C6-16 and step 7 that both
`deal.defaults-set` and the import use it.

*Status:* Fixed (87763d94). Verified: D-C6-6, D-C6-16, step 7, step 8 and 7.11 now name the real two-call form (or a helper in `app/refusal.rs`), and `bad-deal-defaults` is in D-C6-6's list and used by both `deal.defaults-set` and the import.

## Code review round 1 — Claude Opus 5.5, 2026-11-26

This is the first code review of **part C6a** (event log, statement,
`agreement.close`, saved deal terms, `roymctl`, two-node test): plan
section 6.1, D-C6-1 to D-C6-8, D-C6-16, sections 7.1 to 7.5, 7.8, 7.9, 8
and 9. PR #396 (draft), branch `feat/m08e-c6-barter-statement` at
`dc9e7557`, main checkout (not a worktree), `git pull` up to date. Diff:
`git diff origin/main...HEAD` (68 files; merge base `edd71c11`). The code
was written by Gemini 3.8 Flash. There was no earlier code review round,
so there was no Fixed or Rejected item to check again. All plan review
items (P1 to P15) are closed. New findings start at R1.

**CI.** `gh pr checks 396` on `dc9e7557` ([run 45826164453](https://github.com/syneroym/syneroym/actions/runs/45826164453)):
"Lints, licenses & checks", Playwright E2E and the CLA check passed. The
six test shards were still **pending** when I read them, so CI is not yet
proof that the Rust suite passes. I ran the C6a tests myself (below).

**Tests I ran** (targeted only):

- `cargo nextest run -p syneroym-roym-core -p syneroym-roym-transaction`:
  826 passed, 0 skipped (`roym-core` 545, `roym-transaction` 281).
  Filtered to the new tests: `roym-core` `eventlog statement` 27 passed;
  `roym-transaction` `eventlog statement close defaults` 25 passed.
- `cargo nextest run -p roymctl`: 229 passed.
- After `mise run build:roym`: `dual_build_parity` scenarios 260 to 276
  and the `method_contract` test binary (results in the line below this
  list).
- One probe test, written in `roym_transaction/src/app/tests/`, run and
  deleted again (`git status` clean after): a receiver signs a fulfilment
  with a note, one sync runs while every read of `disclosures` fails, then
  the fault is removed and `eventlog.rebuild` and `eventlog.list` run. The
  marker says `complete: true` and the `commitment-settled` entry has no
  note, also after the rebuild (R1).

Results after `mise run build:roym` (built cleanly): `dual_build_parity`
scenarios 260 to 276: 19 passed, scenarios 267 to 276 all among them (10
of 10, none skipped for missing artifacts); `method_contract`: 40 passed,
0 skipped.

### Checked and found correct

- **Pure rules in `roym_core` (D-C6-1, D-C6-2).** `eventlog.rs` and
  `statement.rs` hold the types and the pure `derive` and `build`.
  `EventLogEntry` has the plan's fields, `deny_unknown_fields`, and reuses
  `LocalText`, `ResourceKind`, `CommitmentKind` and `CommitmentState`. Row
  ids are `<agreement>:<commitment_id or ->:<event>`.
- **Derive rules (D-C6-3).** `agreement-formed` takes only
  `quote_author.issued_at_secs` and waits when it is missing; the agreement
  ends and the three failure ends take the lowest-`seq` held snapshot that
  shows them; `commitment-settled` takes the receiver half's time and its
  note; a missing fact leaves the entry out and sets `complete: false`.
- **The marker and the creates.** `derive_agreement` skips an agreement
  only when its marker is complete and has the row's `seq` and `state`; it
  creates each entry alone with `create`, counts only "created", returns on
  a create error before the marker is written, and `eventlog.rebuild`
  ignores markers. The derive runs at the end of `sync_direct`, after the
  tick, with its result ignored (so also in `transaction.drive`), and in
  `eventlog.list`, `statement.export` and the import.
- **AG-28.** The derive step writes `event_log` only with `create` and
  never reads it. `TestHost` records `get` and `query` reads; `$gte` and
  `$lte` are in the test filter.
- **`eventlog.list` (D-C6-5).** Limit 1 to 500, default 100;
  `$gte` filter, full read, sort by `(recorded_at_secs, id)`, drop at or
  before the cursor, `next` only when more rows exist. Scenario 269 pages
  5 rows by 2.
- **`agreement.close` (D-C6-6).** Every id is checked before any signature
  (`bad-commitment-list`, `no-such-commitment`, `not-your-side`,
  `agreement-not-open` for cancelled, not-formed and not countersigned); a
  closed agreement skips every id as `final`; each id goes through
  `fulfilment_ops::sign_at` or the new `payment_ops::acknowledge_at` with
  no note, method or reference; a failure stops the call and keeps the
  halves made. `acknowledge_at` is a pure split: the body of `acknowledge`
  is unchanged.
- **`statement.export` (D-C6-7).** `[from, to)` by the settling half's
  `occurred_at_secs`; only `commitment-settled` makes a row; totals per
  `(currency, direction, resource_kind)` with `checked_add` and the
  2^53 - 1 check; `eeps` is always `not-available`; the three
  `EepSection` forms serialise to the agreements doc's JSON;
  `bad-period` before any read.
- **Saved deal terms (D-C6-16).** One row `defaults` in `deal_defaults`;
  the three texts through `validate` and `check_record_text`;
  `quote_valid_secs` in the `quote.set` range; every failure is
  `bad-deal-defaults`. Section `deal_defaults` in the export list, the
  constant in `roym_core::backup`, `SCHEMA_VERSION` 9 with its comment line,
  `log_entries_made` in the import output, the method shape and the Hub
  type. `validate_bundle_header` moved to `backup/header.rs` unchanged.
- **Refusal shape.** Every new refusal uses `refusal(code)`, which is
  `Response::invalid_params(code).with_data(json!({"code": code}))`. The
  method contract checks message and `data.code` for all seven planned
  codes. `agreement.close` answers an unknown agreement with
  `no-such-agreement`, the code that `fulfilment.sign`,
  `payment.acknowledge` and `step.add` already use (now with `data.code`).
- **Method table and router.** 55 rows (`contract.rs` asserts 55); router
  rows `eventlog.` and `statement.` with `MethodAuth::Owner`;
  `scenario_144` now lists every `transaction` method, the three it
  missed included.
- **No wire change.** No WIT, card type, record type, transition table or
  `CARD_TYPES`/`RECORD_TYPES` change in the diff. Every new method is
  local-only.
- **`roymctl` (D-C6-8).** `agreement close`, `eventlog list|rebuild`,
  `statement --from --to` (UTC midnight), `defaults get|set`. The
  `--close-as` name instead of `--as` (a global flag) is in the
  agreements doc's Deviations.
- **Two-node test.** `roym_statement_e2e.rs` uses the shared harness and
  `roym_flow` helpers, no port literal; the mixed fixture moved to
  `roym_flow.rs` with the new names; the first test closes a
  three-commitment deal from both sides and compares the three rows'
  `settled_at_secs`, value, currency and opposite direction.
- **Sizes and rules.** No new production file over 800 lines (`backup.rs`
  784, `app.rs` 751, `payment_ops.rs` 740); new test files under 800; no
  `mod.rs`; no `too_many_lines` suppression; no planning id in code or test
  names; "barter" only as a test fixture value (`barter-scope`), which the
  plan allows. WASM size of `transaction` reported as 2,866,281 bytes
  (+5.6%), inside the +15% limit (Q9).

Counts: **0 blocker, 5 major, 5 minor.**

### Findings

#### [x] R1 — The derive step turns read errors into missing facts, so a statement can lose a note or get a wrong time for good (major)

*Where:* `crates/roym_transaction/src/app/eventlog_ops/gather.rs:108`
(`if let Ok(Some(sr))`), `:149` and `:199` (`.ok()??`), `:230`
(`disclosures::load(...).ok()??`), `:50` (`unwrap_or(sr.stored_at_secs)`),
`:171` (`.or_else(|| receiver_halves.first())`).

*Problem:* D-C6-3 says "a missing fact defers the entry, never guesses
it", and "on any error or failure, no marker is written (or `complete:
false`)". The gather step does the opposite. A failed read of a snapshot,
a `payments` row, a `fulfilments` row or a `disclosures` row is treated as
"not held". A snapshot envelope that does not parse uses the local
`stored_at_secs`. A currency half that the snapshot does not name falls
back to the first stored half. Then `derive` creates the entry and the
marker says `complete: true`. Because the entry id is fixed and `create`
refuses a second row, neither the next sync nor `eventlog.rebuild` can
correct it.

*Failure case:* (proved with the probe test above) the receiver signs a
fulfilment with the note "All done". On the next sync one read of
`disclosures` fails. The `commitment-settled` entry is created with no
note, the marker is complete, and after the fault is gone
`eventlog.rebuild` makes 0 and the entry still has no note. The
statement row for that commitment has no note for ever (AG-33). In the
same way, a failed read of snapshot `seq` 1 makes the `agreement-closed`
or `commitment-failed` time come from a later snapshot, for good.

*Fix:* Make the gather functions return `Result` and pass every read
error up, so `derive_agreement` fails and writes no marker (the next sync
retries). Treat only `Ok(None)` as "not held". Drop the `stored_at_secs`
fallback: an envelope that does not parse is an error. Drop the
`.first()` fallback: when the snapshot does not name a receiver half,
the fact is missing (`complete: false`). Add a service test with
`fail_read` on `disclosures` and one on `agreement_progress`, each
followed by a clean sync, that checks the note and the time.

*Status:* Fixed (89301a1c). Proof: with the old swallow-error line in `load_note_from_disclosures` restored, `fail_read_disclosures_defers_derive_and_recovers_clean_sync` fails at "entry must not be created when disclosure read fails"; restored. Both new read-error tests pass.

#### [x] R2 — The statement uses the row's `agreed_at_secs` and other guesses, so the two parties' statements can differ (major)

*Where:* `crates/roym_core/src/statement.rs:189` (row `agreed_at_secs`
fallback), `:247` (`open_at_end` fallback `.or(agr.agreed_at_secs)`),
`:194`, `:219` and the other `unwrap_or` defaults in `build_rows`;
`crates/roym_transaction/src/app/statement_ops.rs:68`.

*Problem:* D-C6-3 says `AgreementRow.agreed_at_secs` "is not used because
it can fall back to the local clock or snapshot arrival time". D-C6-4
says `open_at_end` "uses only `agreement-formed` and
`commitment-settled`". `build` falls back to `agreed_at_secs` both for a
row's `agreed_at_secs` and for deciding which agreements are open. On the
counterparty node that value can be `ctx.issued_at` of the snapshot or
`ctx.now` (`sync/receipts.rs:301` to `:306`) when the countersigned half
is not held yet. `build_rows` also invents values when the join fails:
`Direction::Received`, an empty English description, `GoodsOrService`,
`Fulfilment`, value 0.

*Failure case:* Alice (counterparty) files Bob's snapshot (issued at
T2) before Bob's countersigned half (issued at T1, T1 < T2) arrives. Her
row gets `agreed_at_secs = T2` (or her local `now`). Bob's
`agreement-formed` time is T1. For a period that ends between T1 and T2,
Bob's statement lists the deal's commitments in `open_at_end` and Alice's
does not; for a later period both list them, with different
`agreed_at_secs`. AG-29's "both parties put it in the same period" fails
for `open_at_end`, and the time is not from a signed record.

*Fix:* Remove `agreed_at_secs` from `AgreementSource` and both fallbacks:
an agreement with no `agreement-formed` entry is not in `open_at_end`.
For rows, make `derive` hold back `commitment-settled` entries (with
`complete: false`) while it cannot make `agreement-formed`, so every
settled entry has a formed entry beside it. In `build_rows`, skip an entry whose agreement or
commitment is not found, instead of inventing fields. Add a unit test
with an agreement that has `agreed_at_secs` set but no formed entry, and
check it is not in `open_at_end`.

*Status:* Fixed (89301a1c). Proof: with the formed holdback removed, `settled_commitment_held_back_while_formed_cannot_be_made` fails at `d.entries.is_empty()`; restored. `agreement_without_formed_entry_is_not_in_open_at_end` and `entry_with_unknown_agreement_or_commitment_is_skipped` pass.

#### [x] R3 — The C6a decisions are not in the change docs' Deviations, and the test columns are not filled (major)

*Where:* `docs/planning/changes/roym-agreements/change.md:1510` to
`:1516` (the new block "Rules and differences in C6a");
`docs/planning/changes/roym-value-exchange/change.md:772`; the acceptance
rows AG-27 to AG-30 and AG-33 (`roym-agreements/change.md:148` to `:158`).

*Problem:* Plan section 9 lists what the C6a close-out writes in the
agreements doc's Deviations: Q4 (a) saved terms as a `transaction` row in
the backup; Q5 (a) the log is not in the bundle and the import rebuilds
it, so `recorded_at_secs` is the import time (the doc says export and
import "carry ... the event log", `:1009`); Q6 (a) `counterparty_name` is
never filled by the service; D-C6-3 the marker, and entries that wait for
a missing fact; D-C6-4 the times of non-settle events can differ between
nodes, against the doc's "both parties give one event the same time"
(`:915`); D-C6-6 `agreement.close`'s exact input, output and repeat rule
(the doc says "Returns the records made"); D-C6-7 `eeps` always
`not-available`, `bad-period`, `statement-total-overflow`, and the derive
in the export; and the new methods `deal.defaults-get` and
`deal.defaults-set`, which the doc's method table does not list. None of
these is written. The new block holds instead five notes that are not
differences from the doc (scenario 267's entry count, scenario 270's
totals rows, `SCHEMA_VERSION`, a test helper rename) and one real one
(`--close-as`). The value-exchange note says only what C6a builds; it does
not say VX-45's "agreement signed" event is `agreement-formed` and that
EEP events are M08E's. The AG and VX test columns do not name the new
test files.

*Failure case:* A reader of the agreements doc believes the backup bundle
carries the event log, that both nodes always give every event the same
time, and that `agreement.close` returns the records it makes on a
repeat. C6b and M08E build on those false statements. The AGENTS.md rule
"record every code-vs-plan difference in its `## Deviations` section"
fails.

*Fix:* Replace the C6a block with the items of plan section 9 that belong
to C6a (Q4, Q5, Q6, D-C6-3, D-C6-4, D-C6-6, D-C6-7, the two
`deal.defaults-*` methods and `--close-as`). Drop the notes that are not differences. Add the VX-45 and
VX-46 notes to the value-exchange doc. Fill the test columns of AG-27 to
AG-30, AG-33, AG-46, AG-48 (C6a parts) and VX-45, VX-46, VX-55 with the
test file names.

*Status:* Fixed (855b4713). The C6a block now holds only plan section 9's differences; the AG-30 and AG-33 test names point at tests that exist.

#### [x] R4 — The living docs give wrong method inputs and wrong facts (major)

*Where:* `docs/developer-guide.md:2721` to `:2725`;
`docs/TERMINOLOGY.md:97` to `:99`; `docs/system-architecture.md:1264`;
`docs/system-requirements-spec.md:699`.

*Problem:* The developer guide describes methods that do not exist in
this form:

- `eventlog.list` "filtered by optional `agreement`, `conversation`,
  `event`, and timestamp ranges (`from_occurred_at_secs`,
  `to_occurred_at_secs`)". The real input is `{after?: {recorded_at_secs,
  id}, limit?}` with `deny_unknown_fields`.
- `eventlog.rebuild` "resetting the watermark". There is no watermark;
  it ignores the per-agreement markers and creates only missing rows.
- `statement.export` "Takes `from_occurred_at_secs`, `to_occurred_at_secs`,
  and optional `currency`". The real input is `{from_secs, to_secs}`, no
  currency filter, and `deny_unknown_fields`.
- `agreement.close` "Repeated calls return stored records". A repeat of a
  receiver close returns `records: []` and every id in `skipped` with
  `final` (D-C6-6).
- `deal.defaults-*` fields include `payment_timing` and `currency`.
  `DealDefaults` has only the three texts and `quote_valid_secs`.
  TERMINOLOGY's "Saved deal terms" row has the same two wrong fields.

Also: the plan's developer guide items (the `transaction` method table,
the `event_log`, `event_log_state` and `deal_defaults` collections) are
not written. The architecture "Closed" line now says an agreement closes
"or when closed via `agreement.close`"; there is no separate close:
`agreement.close` only signs halves, and the agreement closes when every
commitment is final. The requirements line names `FulfilmentRecord`; the
record is `FulfilmentReceiptPayload` (`roym_core/src/fulfilment.rs:58`).

*Failure case:* A developer calls `statement.export {from_occurred_at_secs,
to_occurred_at_secs, currency}` as the guide says and gets `-32602`
(unknown field). A developer who reads the architecture doc expects
`agreement.close` as giver to close the agreement; it only claims.

*Fix:* Rewrite the five bullets from the code (inputs, outputs, refusal
codes, the marker), add the three collections and the methods to the
`transaction` table, fix the TERMINOLOGY row's field list, change the
architecture line back to "once every commitment reaches a final state"
and add one sentence that `agreement.close` signs one settlement half per
listed commitment, and use the real record name in the requirements line.

*Status:* Fixed (9307da76).

#### [x] R5 — The plan's time measurements and the seven-component size report are missing (major)

*Where:* plan section 10 (C6a exit criteria), 7.8 and 7.9;
`docs/planning/milestones/M08C-deal-core-and-extensions/status.md:25`;
`task.md` section 1 (not changed);
`crates/roym_core/src/statement.rs:186` to `:190` and `:260` to `:282`.

*Problem:* Two exit criteria of C6a are not met. (1) "The derive step's
time over 50 agreements and the export's time over 1,000 agreements
recorded on the WASM build, within the stop rules" (200 ms and 2 s). No
test or note records either. (2) "Report all seven components, before
and after, from one `target/` folder". status.md gives only `transaction`,
and task.md's size table is not updated. The first matters here, because
`build` is quadratic: for each settled entry it scans all entries for the
formed entry and all agreements for the join, and `build_open_at_end`
scans all entries twice for each commitment of each agreement. With 1,000
agreements of three commitments (about 5,000 entries) that is tens of
millions of string compares per export, on top of `derive_all`'s 1,000
marker reads.

*Failure case:* The export over 1,000 agreements goes over the 2 s stop
rule on the WASM build, and nobody knows until a person with a year of
deals opens the statement screen in C6b.

*Fix:* Build one map `agreement -> formed time` and one map
`(agreement, commitment_id) -> (settled time, has final)` in one pass
over the entries, and one map for the agreements; then each row and each
open commitment is one lookup. Add a parity scenario (like scenario 266
for `widget.list`) that seeds 50 closed agreements for one sync and 1,000
agreements for one export, prints both times, and fails only well above
the stop rules. Record the numbers and the seven component sizes in
status.md and task.md section 1.

*Status:* Fixed (0a1423f3, 855b4713). Scenario 277 prints 18 ms native / 20 ms WASM for the sync over 50 agreements and 195 ms native / 297 ms WASM for the export over 1,000; both are within the stop rules.

#### [x] R6 — A not-formed agreement never gets a complete marker, so it is derived again on every sync (minor)

*Where:* `crates/roym_transaction/src/app/eventlog_ops/gather.rs:41` to
`:42`; `crates/roym_core/src/eventlog.rs:138` to `:143`;
`crates/roym_core/src/eventlog/tests.rs:167`;
`crates/roym_transaction/src/app/tests/eventlog.rs:211`.

*Problem:* `gather` sets `agreed: Some(FormedFact { quote_author_issued_at_secs: None, .. })`
whenever the row holds the counterparty half. A not-formed row always
holds it (the author filed the acceptance, `sync.rs:600`, then
`decide::run` refused) and never gets a quote author half. So
`derive_formed` returns `false` and the marker is `complete: false` for
ever. Every `transaction.sync` and `transaction.drive` of that
conversation, every `eventlog.list` and every `statement.export` then
reads that agreement's snapshots, payments, fulfilments and disclosures
again. The marker exists to stop that (D-C6-3, "Cost"). Both tests miss
it: the unit test passes `agreed: None`, and the service test turns an
agreed deal (both halves) into `NotFormed`.

*Failure case:* A conversation where Bob sent five offer versions and
Alice's acceptance of each older one was refused (`quote-superseded`)
holds five not-formed rows. Each sync re-derives all five, for ever, and
the work grows with the history.

*Fix:* Set `agreed` only when `state` is `Agreed`, `Closed` or
`Cancelled` (or when the quote author half is held). Change the unit test
to pass the counterparty half for a not-formed input and assert
`complete: true`, and change the service test to build a not-formed row
with only the counterparty half and check the marker is complete.

*Status:* Fixed (89301a1c). Proof: with the old `agreed` condition and without the not-formed early return, `writer_derives_agreement_not_formed` fails at `marker.complete`; restored. The unit test now passes the counterparty half and asserts `complete`.

#### [x] R7 — The import writes the saved deal terms before it checks the bundle, and fails after its writes when the derive fails (minor)

*Where:* `crates/roym_transaction/src/app/backup.rs:123` and `:191`;
`crates/roym_transaction/src/app/backup/defaults.rs:19`.

*Problem:* (1) `import` checks every section first (`verify_imported_*`,
`validate_vertical_sections`) and writes after. `import_defaults_section`
both checks and writes, and it runs before those checks. (2) The import
ends with `rebuild_at(host, now).await?`. When the derive fails (for
example one `create` error), the call returns an error after every
section is written. Plan Q5 and VX-45 say the log never blocks: the
import should report what it wrote. Also the section reader guesses the
row shape (`r.get("payload")...unwrap_or_else(|| r.clone())`).

*Failure case:* (1) A bundle with good deal terms and one bad agreement
row is refused with the agreement's error, but the node's saved terms are
already replaced. (2) A restore that wrote every row answers an internal
error; the Hub shows the restore as failed and the person restores again.

*Fix:* Split the section into `validate_defaults_section` (called with
the other checks, before any write) and a write step after
`write_imported_cards`. Make the rebuild's error not fail the import:
log it and return `log_entries_made: 0`, as `sync_direct` ignores it (the
next sync repairs). Read the row in the one shape that `export` writes.

*Status:* Fixed (89301a1c). The import validates the saved terms before any write, writes them after the cards, and returns `log_entries_made: 0` when the rebuild fails. `fail_write_event_log_does_not_fail_sync_and_recovers_next_sync` reads the row directly and passes.

#### [x] R8 — Several tests prove less than their names (minor)

*Where:* `crates/roym_transaction/src/app/tests/eventlog.rs:414`;
`crates/substrate/tests/roym_statement_e2e.rs:188`;
`crates/roym_transaction/src/app/tests/statement.rs:220`;
`crates/roym_core/src/eventlog/tests.rs:483`;
`crates/roym_transaction/src/app/tests/eventlog_reads.rs:15`.

*Problem and failure case:*

- `fail_write_event_log_does_not_fail_sync_and_recovers_next_sync`
  checks the entry through `eventlog.list`, which runs the derive itself.
  The test passes when the second sync derives nothing.
- `the_driver_logs_with_no_hub` polls `eventlog.list`, which derives. So
  it proves that the wake files the snapshot, not that the wake writes the
  log (plan 7.5: "through the wake alone").
- `statement_export_with_and_without_money` has only a deal without
  money. A bug in currency rows passes it.
- `corrected_acknowledgement_does_not_move_occurred_at` gives `derive` the
  time directly; the choice of the half is in `gather` (`:166` to `:171`),
  which no test runs with a correction.
- `full_deal_path_never_reads_event_log_until_list_called` writes the
  deal rows with `setup_agreed_deal` and calls only `sign_at`,
  `acknowledge_at` and `derive_conversation`. Plan 7.4 asks for a full
  deal: enquiry, quote, accept, countersign, two settlements, close, with
  the read log on. `transaction.sync`, `agreement.accept` and
  `agreement.close` are not in it.
- Plan 7.4 also asks for each event kind on the counterparty's node; only
  `commitment-settled` is there.

*Fix:* Read `event_log` rows with `get_row` (not `list`) in the first two
tests. Add a currency commitment to the money test and assert its row and
total. Add a service test that files two receiver acknowledgements (the
second with `supersedes`) and checks the time is the first one's. Run the
read test through `quote.set`, `agreement.accept`, `transaction.sync`,
both settlements and `agreement.close`, and clear the read log only
before the first call. Add the counterparty cases for the agreement ends
and the three failure ends.

*Status:* Fixed (1d703ff1, 292cc2c6). The named tests now read `event_log` with `get_row`, run a full deal, cover the money row and total, the corrected acknowledgement, the not-formed marker and the counterparty ends. The driver test reads the row from the data layer.

#### [x] R9 — status.md test counts do not match what the tests print (minor)

*Where:* `docs/planning/milestones/M08C-deal-core-and-extensions/status.md:25`.

*Problem:* The C6 row says "roym-core 229 passed; roym-transaction 297
passed; ... method_contract 38 passed; roymctl 25 passed". On `dc9e7557`, `cargo nextest run -p
syneroym-roym-core` prints 545, `-p syneroym-roym-transaction` 281, and
`-p roymctl` 229, and the `method_contract` binary 40. 297 is more than the whole `roym-transaction` suite.
The filtered C6 counts are 27 (`roym-core` `eventlog statement`) and 25
(`roym-transaction` `eventlog statement close defaults`).

*Failure case:* A reviewer or the slice close uses the row as evidence of
what ran, and the numbers prove nothing.

*Fix:* Rerun and copy the numbers with the command that printed each one,
as the C4 rows do.

*Status:* Fixed (status.md C6 row, in this commit). Counts from the commands in the row: roym-core 548, roym-transaction 285, roymctl 229, method_contract 40, statement parity 11, roym_statement_e2e 2.

#### [x] R10 — Small rule and duplication points in the new code (minor)

*Where:* `crates/roym_core/src/eventlog.rs:148`, `:187`, `:235`, `:289`
(four copies of the `EventLogEntry` literal), `:221` (`unreachable!()`);
`crates/roym_transaction/src/app/eventlog_ops/gather.rs:28` and
`crates/roym_transaction/src/app/statement_ops.rs:53` (the same
owner-role and counterparty code twice);
`crates/roym_transaction/src/app/refusal.rs:4` (`#[allow(dead_code)]` on a
function that is used); `crates/roym_transaction/src/app/backup.rs:200`
(a `use` line at the end of the file);
`apps/roymctl/src/commands/roym/transaction.rs:693` (`_ =>
unreachable!()`).

*Problem:* AGENTS.md asks to look for an existing helper and to keep the
code without suppressions. The four entry literals differ only in a few
fields; a new field must be added in four places. The `allow` hides
nothing and stays in the code. The `use` at the end of `backup.rs` breaks
the import layout. The two `unreachable!()` replace a compile-time match
check with a run-time panic: in `roymctl`, a new `TransactionCommands`
variant that is added to neither match compiles and panics when used.

*Failure case:* A later field on `EventLogEntry` (for example an EEP id
in M08E) is set in three of the four places; one event kind writes it as
absent.

*Fix:* One helper `fn entry(input, event, commitment: Option<(&Commitment,
&CommitmentProgress)>, occurred_at, records, note) -> EventLogEntry`; map
the three failure states to their event with a small `fn` that returns
`Option<LogEvent>`. One helper for the owner role and the counterparty
DID, used by both files. Remove the `allow`. Move the `use` to the top.
In `roymctl`, keep one exhaustive `match` (the lifecycle arms call their
handlers and return).

*Status:* Fixed (89301a1c, 49c5946e). One `entry` builder and one `failure_event` map the entries; `owner_role_and_counterparty` is shared; the `allow` and the end-of-file `use` are gone; `roymctl` has one exhaustive match.

## Code review round 2 — Claude Opus 5.5, 2026-11-26

This round checks the fixes for R1 to R10. Branch
`feat/m08e-c6-barter-statement` at `23f10662`, main checkout (not a
worktree), `git pull` up to date. Fix commits, written by Gemini 3.8
Flash: `89301a1c`, `1d703ff1`, `49c5946e`, `0a1423f3`, `292cc2c6`,
`9307da76`, `855b4713` and `23f10662`. I read `git diff
02d5730a..23f10662` (29 files).

**CI.** `gh pr checks 396` on `23f10662` ([run 45848714549](https://github.com/syneroym/syneroym/actions/runs/45848714549)):
every job passed: "Lints, licenses & checks", Playwright E2E, the CLA
check and all six test shards. Benchmarks and Code Coverage were skipped,
as usual for a PR. So CI is proof that the full suite passes.

**Tests I ran** (targeted only):

- `cargo nextest run -p syneroym-roym-core -p syneroym-roym-transaction
  -p roymctl`: 1,062 passed (548 + 285 + 229, the numbers in status.md).
- `mise run build:roym` (built), then `cargo nextest run -p
  syneroym-roym-web --test dual_build_parity statement`: 11 passed
  (scenarios 267 to 277, none skipped); `--test method_contract`: 40
  passed.
- `cargo nextest run -p syneroym-substrate --test roym_statement_e2e`:
  2 passed.
- One probe for R1: I put the old line back in
  `load_note_from_disclosures` (`.await.ok().flatten()` in place of
  `.await?`). Then `fail_read_disclosures_defers_derive_and_recovers_clean_sync`
  fails at `eventlog.rs:517`. I restored the file (`git status` clean).

### Round 1 findings

- **R1 — fixed.** Every read in `gather.rs` now returns its error with
  `?`, and a snapshot envelope that does not parse is an error too. An
  error stops `derive_agreement` before it writes the marker, so the next
  sync derives again. The currency path no longer falls back to the first
  half; with no matching half the entry waits (`complete: false`). Two
  service tests cover a failed read of `disclosures` and of
  `agreement_progress`, each followed by a clean sync that checks the note
  or the time. My probe above shows the first test fails with the old
  line.
- **R2 — fixed.** `AgreementSource` has no `agreed_at_secs` any more.
  `build_rows` skips an entry whose agreement, commitment or formed entry
  is not found, and `open_at_end` lists only agreements with a formed
  entry. The derive step holds back `commitment-settled` while the
  formed entry cannot be made. The only values taken from the terms are
  resource kind, kind, declared value and direction, which are signed and
  the same on both nodes. The build now uses two maps and is linear in
  the number of entries.
- **R3 — fixed.** The C6a block in the agreements change doc now lists
  Q4, Q5, Q6, D-C6-3, D-C6-4, D-C6-6, D-C6-7, the two `deal.defaults-*`
  methods and the CLI flag. The notes that were not differences are gone.
  The test columns of AG-27 to AG-30, AG-33, AG-46 and AG-48 and of VX-45,
  VX-46 and VX-55 name test files and tests that exist. I checked the
  refusal codes named there (`bad-period`, `statement-total-overflow`,
  `bad-commitment-list`, `agreement-not-open`) against the code.
- **R4 — fixed.** The developer guide now gives the real inputs of
  `eventlog.list` (`after`, `limit`) and `statement.export` (`from_secs`,
  `to_secs`, no currency filter), the real repeat rule of
  `agreement.close` (it matches `close_at` and `sign_one_commitment`), and
  the four fields of `DealDefaults`. TERMINOLOGY has the same four fields.
  The architecture line no longer says an agreement is "closed via
  `agreement.close`". The requirements line names
  `FulfilmentReceiptPayload`, which exists in `roym_core::fulfilment`.
- **R5 — fixed.** Scenario 277 measures one sync of a conversation with 50
  closed agreements and one export over 1,000 agreements on both builds,
  and status.md and task.md section 1 record the times (20 ms and 297 ms
  on WASM) and all seven component sizes. The scenario seeds a complete
  marker for every agreement, so it measures the marker path. That is
  what plan section 9 asks ("one read of `event_log_state` per
  agreement"). The first derive with no markers (after an import or
  `eventlog.rebuild`) is not measured; the plan does not ask for it.
- **R6 — fixed.** `agreed` is now set only for an agreed, closed or
  cancelled row or when the author's half is held, and `derive_formed`
  returns early for a not-formed agreement. So a not-formed agreement gets
  a complete marker. The unit test passes the counterparty half and
  asserts `complete`; `writer_derives_agreement_not_formed` checks the
  marker.
- **R7 — fixed.** `validate_defaults_section` runs before any write and
  requires the `payload` field (the one shape that `export` writes).
  `write_defaults_section` runs after the cards. A failed rebuild no
  longer fails the import; it returns `log_entries_made: 0`, and R11
  removes the stderr print.
- **R8 — fixed.** The fail-write test and the driver test read the
  `event_log` row directly (`get_row`, and a new
  `RoymNode::get_service_row` that opens the service's encrypted
  `state.db`), so neither runs the derive itself. The read test now runs
  a full deal on two fixtures (`quote.set`, `agreement.accept`,
  `transaction.sync`, both settlements and `agreement.close`) and clears
  the read log only before the first call. The money test has a currency
  commitment and checks its row and total. A service test files two
  receiver acknowledgements, the second with `supersedes`, and checks the
  time and note of the first. The counterparty test covers the three
  agreement ends and the three failure ends.
- **R9 — fixed.** The counts in the C6 row (548, 285, 229, 40, 11, 2) match
  what the commands print in my run above.
- **R10 — fixed.** One `entry` builder and one `failure_event` map; no
  `unreachable!()` is left in `eventlog.rs` or in `roymctl`'s
  `transaction.rs`; `owner_role_and_counterparty` is shared by
  `gather.rs` and `statement_ops.rs`; the `allow(dead_code)` and the
  end-of-file `use` are gone. No added Rust line names a milestone,
  slice, plan section or finding.

### New findings

#### R11 — The import prints the rebuild error with `eprintln!` (minor)

*Where:* `crates/roym_transaction/src/app/backup.rs:197`.

*Problem:* When `rebuild_at` fails after an import, the code writes the
error with `eprintln!`. No other Roym service prints to stderr, and the
`AppHost` trait has no log call. In the native build the line goes
straight to the substrate's stderr, outside `tracing`, so the log level
and format do not apply. In the WASM build it depends on how the host
wires the guest's stderr. `sync_direct` ignores the same derive error
with no output (`sync.rs:163`).

*Failure case:* A node imports a bundle while one data-layer read fails.
The import returns `log_entries_made: 0` and an unformatted line appears
in the substrate's stderr, also when the operator set the log level to
`error` only. Nothing breaks, but the service now has two ways to report
the same ignored error.

*Fix:* Drop the print and ignore the error as `sync_direct` does (the
next sync repairs the log), for example
`.map_or(0, |r| r.made)`. If the owner wants the error seen, add a log
call to the host interface in its own change, and use it in both places.

*Status:* Fixed (168c9d1f). The import now ignores the rebuild error with
`map_or(0, |r| r.made)`, as `sync_direct` does; no stderr write remains.

**Counts of round 2:** R1 to R10 verified fixed. One new finding, R11
(minor). **1 open:** 0 blocker, 0 major, 1 minor.

## Code review round 3 — Claude Opus 5.5, 2026-11-26

This round checks the fix for R11, the one finding open after round 2.
Branch `feat/m08e-c6-barter-statement` at `37998b38`, main checkout (not
a worktree), `git pull` up to date. Fix commits: `168c9d1f` (code) and
`37998b38` (docs), written by Gemini 3.8 Flash. The author changed my
round 2 count line to "0 open". I put it back, because it records what
round 2 found, and I give the verdict here.

**CI.** `gh pr checks 396` on `37998b38` ([run 45861747461](https://github.com/syneroym/syneroym/actions/runs/45861747461)): every job passed:
"Lints, licenses & checks", Playwright E2E, the CLA check and all six
test shards. Benchmarks and Code Coverage were skipped, as usual for a
PR.

**Tests I ran** (targeted only): `cargo nextest run -p
syneroym-roym-transaction`: 285 passed. `cargo clippy -p
syneroym-roym-transaction --all-targets -- -D warnings`: clean.

### Round 2 findings

- **R11 — fixed.** `import` now reads
  `eventlog_ops::rebuild_at(host, now).await.map_or(0, |r| r.made)`, the
  same as the fix proposed. A failed rebuild gives `log_entries_made: 0`,
  and the next sync repairs the log, as in `sync_direct`. No `eprintln!`
  is left in any `crates/roym_*/src` file outside tests. The change
  touches only this line, so the import tests and CI cover it.

**Counts of round 3:** R11 verified fixed. No new findings. All 11
findings are closed. **0 open.** No blocker or major.

## Code review round 1 (part C6b) — Claude Opus 5.5, 2026-11-27

This is the first code review of **part C6b** (the deal screens in the
Hub): plan section 6.2 steps 1 to 8, "Hub changes (C6b)" in section 4,
D-C6-9 to D-C6-15, and the C6b tests in section 7.6. PR #397 (draft),
branch `feat/m08c-c6b-deal-screens` at `1433ecf1`, main checkout (not a
worktree), `git pull` up to date. Part C6a is merged in `main` as
`04a44689` (#396), so `git diff origin/main...HEAD` (60 files, 9 commits
from `dccd6ca2` to `1433ecf1`) is all C6b. The code was written by Gemini
3.8 Flash. Code review rounds 1 to 3 above are about C6a and are closed;
I read them for earlier mistakes and did not check them again. There was
no earlier C6b round, so there was no Fixed or Rejected item to check.
New findings start at R12.

**CI.** `gh pr checks 397` on `1433ecf1` ([run 45883494692](https://github.com/syneroym/syneroym/actions/runs/45883494692)):
"Lints, licenses & checks", all six test shards and the CLA check
passed. **Playwright E2E failed** (R12). The run on `c2330328` was
cancelled. No CI run on this branch has passed Playwright.

**Tests I ran:** the Hub Vitest suite once (`npm test` in
`crates/roym_web/ui`): **68 files, 741 passed**, the same as the C6b row
in status.md. Nothing else; I read the CI log for Playwright.

### Checked and found correct

- **No node code changes (focus point 1).** Every file outside
  `crates/roym_web/ui` and `crates/substrate/tests/e2e` is a doc:
  `docs/TERMINOLOGY.md`, `docs/demo/demo.md`,
  `docs/roym-integrated-experience-spec.md`,
  `docs/planning/traceability-matrix.md`,
  `docs/planning/deferred-backlog.md`, both change docs, and M08C
  `status.md` and `task.md`. All are close-out docs that plan section 9
  names. No Rust, WIT, record, card type or method changes. No wire
  change.
- **The renamed messages (focus point 2).** No message id is removed in
  `en.ftl` or `hi.ftl`, so no code uses a removed id. Both files have the
  same ids (63 new ids each). Every id has the same placeholders in both
  files. 39 English texts change, and the same 39 Hindi texts change.
  Each change is only "quote" to "offer" or "accept" to "agree" in text a
  person reads, as D-C6-15 asks: the ten ids it names plus the lower-case
  lines it lists. Ids stay. `listing-model-quote-only` shows "offer-only"
  for the wire value `quote-only`, which stays. The PR lists the 39
  changed ids. I did not judge the Hindi wording.
- **The listing defaults (focus point 2).** The listing editor's currency
  default is `INR` (`listings.ts:438`, Q8 (a)). The as-listed section
  fills its empty deal texts from `deal.defaults-get` (point 17), with
  two problems in R21.
- **Money in the totals (focus point 3).** `dealTotals` sums
  `declared_value_minor` as `BigInt` over every commitment, whatever its
  kind, with `difference = receive - give` (D-C6-10). The quote card, the
  form and the panel show "You give" and "You receive" from the reader's
  role, and the difference line only when it is not zero, with the right
  sign word for each side. `formatMinor` takes a `bigint`, and
  `minorToDecimal` keeps every digit; there is no rounding in display.
  The float problems are only in the form's input path (R16).
- **No unescaped sink (focus point 4).** The diff adds no `innerHTML`,
  `outerHTML`, `insertAdjacentHTML` or `document.write`. The settlement
  note, descriptions, counterparty names, the change summary and the
  statement rows are all set with `textContent` or `text()`. The one
  `href` is the `blob:` URL of `download.ts`.
- **The CSV writer (focus point 4).** `statement_csv.ts` has the plan's
  13 columns in order. RFC 4180 quoting doubles `"` and quotes a field
  with a comma, quote, CR or LF. Lines end with CRLF. The formula guard
  (D-C6-14) puts `'` before a cell that starts with `=`, `+`, `-`, `@`,
  tab or CR, on every text column including `counterparty_name`; the
  decimal value is not guarded, which is right. Dates carry the local
  offset. `declared_value` uses `minorToDecimal`.
- **Statement screen.** `statement.export` with `[from, to)` at local
  midnight, then `contacts.list` for names (Q6 (a)); the JSON download is
  the service object with `counterparty_name` filled; the EEP line covers
  all three forms. The backup screen now uses the shared `download`
  helper.
- **Close list (AG-46).** `closeList` leaves out final commitments and
  lists a commitment whose window starts after now unchecked with "Not
  started yet". The panel sends `agreement.close {agreement, as,
  commitments}`, which matches `AgreementCloseInput`. `payment.acknowledge`
  and `fulfilment.sign` get `{agreement, commitment_id,
  settlement_note?}`, which match their inputs.
- **Open work.** `.settle-dialog` and `.close-list` are in `OPEN_WORK`
  (`messages.ts`), as D-C6-13 asks.
- **Quote form shape (D-C6-9).** The form moved to `quote_form/`;
  `messages.ts` is 1,030 lines (was 1,226). It keeps the classes that
  `sendQuote` uses. Rows: either side, service, goods or money, 1 to 16
  rows. `buildDealTerms` builds one commitment per row in row order, and
  the `after` lists follow the timing choice as D-C6-9 says. "More terms"
  starts closed. The size line is attached.
- **Ports (focus point 6).** `roym-deal.spec.ts` reads its ports with
  `readE2EPorts()` in `beforeAll`; story 50 uses `readDemoEnv()`. There is
  no port literal.
- **Change doc status (focus point 7).** Both change docs stay
  `status: draft`. That is right: each one covers later work (the agreements
  doc C7 and C8 and its open tasks; the value-exchange doc M08D to M08H).
  Both `demo:` lists name story 50, which exists. The C6 task box in the
  agreements doc is checked.
- **Vitest count (focus point 7).** 68 files, 741 passed, as status.md
  says.

**Counts:** 1 blocker, 5 major, 7 minor. **13 open.**

### Findings

#### R12 — Playwright E2E fails in CI; two deal cases never pass and the demo stories never run (blocker)

*Where:* `crates/substrate/tests/e2e/tests/roym-deal.spec.ts:23` to `:80`
(D1) and `:239` to `:281` (D3); `crates/substrate/tests/e2e/playwright.config.ts:19`
(`globalTimeout: 300_000`); `crates/substrate/tests/e2e/demo/50-roym-deal.spec.ts:352`,
`:358`; status.md C6b row; PR description "How it was tested".

*Problem:* In CI run 45883494692, D1 fails on all three tries
(`.accept-quote-button` not found, 60 s each) and D3 fails on all three
tries (`.quote-suggest-changes-button` not found). The cause is the test
data. `renderCardActions` draws "Agree", "Decline" and "Suggest changes"
only when `card.verified` is true and `card.data.counterparty_did` equals
the viewer's DID (`messages.ts:843`, `:844`). Both test cards have no
`verified` field, and both set `counterparty_did: 'did:key:author'` while
the viewer is `did:key:viewer`. So the buttons can never appear; the
cases have never passed anywhere. The three D1 tries use 3 of the 5
minutes of `globalTimeout`, so the run stops with "38 did not run"
(`roym-hub.spec.ts` case 26 onwards). `npm test` runs `playwright test
&& … multihop … && npm run test:demo`, so the multihop specs and **all
demo stories, story 50 included, never ran in CI**. status.md and the PR
say "`roym-deal.spec.ts` 5 passed" and that story 50 "passes
end-to-end"; CI does not show either.

Story 50 Case 3 also looks broken: `openAgreementPanel(page, runTag)`
looks for `.agreement-panel-<runTag>` (the panel class uses the quote
record id, `agreement_panel.ts:74`) and then for a panel whose text has
`runTag`. The panel shows the commitment descriptions ("One lesson, one
hour", "One sack of wheat", the money), values and states, not the
scope, so no panel has `runTag` in its text and the step should time out
after 15 s. CI did not run it, so this is a reading, not a proof.

*Failure case:* The PR is merged on the status.md claim. Main's
Playwright job is red, and nobody knows whether story 50 works.

*Fix:* In D1 and D3, set `verified: true` and `counterparty_did:
'did:key:viewer'` on the card row (or pass the matching DID). In
`openAgreementPanel`, find the panel under the receipt card that has the
tag (for example `threadMessage(page, runTag)` then the next
`.agreement-panel`), or take the agreement id from `fetchAgreement`. Run
`mise run test:e2e` and `mise run demo` until both pass, and copy the
counts from the run into status.md and the PR.

*Status:* Open.

#### R13 — The quote form never fills from the listing, and the payee never falls back to the author (major)

*Where:* `crates/roym_web/ui/src/quote_form/form.ts:146` (`authorDid: ""`),
`:152` (`fillQuoteForm(null, null, builtIn)`), `:451` to `:468`;
`crates/roym_web/ui/src/quote_form/defaults.ts:70`.

*Problem:* AG-48 and D-C6-9 say the form fills from the listing first
(`listing.get` then `listing.verify`: currency, price, timing, location,
payee and the deal texts), then from the saved deal terms, then from
built-in text, and the payee falls back to the author's own DID. The
pure function `fillQuoteForm` does this, and its unit test passes. But
`openQuoteForm` calls it with `null` for the listing and the saved terms,
and with an empty author DID. Nothing in the form reads the enquiry's
`listing_id` or calls `listing.get`. The saved terms come later, from an
async `deal.defaults-get`, which writes the three texts and the hours
**without checking whether the person has already typed in them**. So:
an offer to an enquiry from a listing starts with `INR`, no price, no
payee and the built-in texts; the payee field is empty, so "Send offer"
fails with "payee required" until the author types their own DID. The
plan's "How this is shown" (first bullet) and story 50 Case 3 (it types
the currency, the amount and `bob.did` by hand) show this. D4 does not
catch it: it passes `preFill` straight into `openQuoteForm` and never
opens the form from a listing (R17). The agreements change doc ticks C6
"the quote form defaults" as done.

*Failure case:* Bob opens Alice's enquiry for his ₹500 lesson listing,
types the scope, and taps "Send offer". The form says the payee is
required. He types his DID, sends, and the offer has the built-in
cancellation text, not his listing's.

*Fix:* In `openQuoteForm`, when the enquiry card has a `listing_id`,
call `listing.get` and `listing.verify` (as `as_listed_tap.ts` does),
call `deal.defaults-get`, and pass both results and the signed-in DID
(`myDid`, which `renderCardActions` already has) to `fillQuoteForm`
before the rows are drawn. Fill a field only while it is empty. Change
D4 to open the form from an enquiry card with a `listing_id` and mocked
`listing.get`/`listing.verify`, and assert the price, the payee and the
listing's texts.

*Status:* Open.

#### R14 — On the counterparty's node, a commitment the author settled shows "Waiting" and no "Received" button; a half the counterparty just signed shows no waiting state (major)

*Where:* `crates/roym_web/ui/src/deals/agreement_panel.ts:131` to `:142`;
tests `crates/roym_web/ui/src/deals/agreement_panel.test.ts:140`,
`:168`.

*Problem:* The panel shows "Sent. Waiting for the other person's node."
when the viewer is the counterparty and `p.halves` is not empty. But
`halves` is filled only by the quote author's node, from both parties'
halves (`apply_half`, `roym_core/src/progress.rs:330`), and reaches the
counterparty only inside the author's snapshot. So the check is wrong in
both directions:

- Bob (author) taps "I delivered" on the lesson. His snapshot has
  `state: claimed, halves: [bob's half]`. When Alice's node files it,
  her panel shows "Waiting" for the lesson and **no "Received" button**.
  D-C6-13 says "A receiver can still settle a claimed commitment." Alice
  can only use "I received everything".
- Alice taps "I paid". Her node files her half but does not move its
  own row (plan point 5), so `halves` stays empty and she sees "I paid"
  again, not the waiting text. The state that D-C6-13 asks for is never
  shown.

The two tests pass because they use rows the service never returns: a
`claimed` commitment with no `halves` (`:140`) and a `promised` one with
a half (`:168`). Story 50 Case 3 does not tap "Received", so it does not
show the problem.

*Failure case:* As above: on Alice's Hub, after Bob delivers, she cannot
confirm that one lesson.

*Fix:* Decide "waiting" from this node's own halves, not from
`halves`. For each open commitment on the counterparty's node, read this
node's half for its role (`fulfilment.get` or `payment.get` with
`{agreement, commitment_id}`, which return the stored halves), and show
"Waiting" only when this node holds its own half and its `record_id` is
not yet in `p.halves`. Keep the "Received" button when only the other
party's half is in `halves`. Change the two tests to service-shaped rows:
`claimed` with the giver's half in `halves` (receiver still gets
"Received"), and `promised` with no `halves` plus a mocked own half
(waiting shown).

*Status:* Open.

#### R15 — "Send a new version" sends the quote's own record id as the enquiry, so `quote.set` refuses it (major)

*Where:* `crates/roym_web/ui/src/quote_form/form.ts:414` to `:416`;
`crates/roym_web/ui/src/screens/messages.ts:928` (the button).

*Problem:* For a quote card, the form reads
`data.request_record_id` and, when it is missing, falls back to the
card's own `record_id`. A quote card's data is the plain view of
`QuotePayload` (`roym_core/src/transaction.rs:164`): `quote_id`,
`conversation`, `sequence`, `answers: {kind: "enquiry", record_id}`,
`counterparty_did`, `terms`. It has no `request_record_id` (that field is
on `quote.get`'s pointer output, `quote_ops.rs:104`). So the call is
`quote.set {enquiry_record_id: <quote record id>, quote_id}`, and
`load_verified_enquiry` answers "no such enquiry"
(`quote_ops.rs:174`, `:299`). The only test
(`suggest_changes.test.ts:82`) checks that the form opens, not what it
sends.

*Failure case:* Alice taps "Suggest changes" and asks for a lower price.
Bob taps "Send a new version", changes the amount and taps "Send offer".
The form shows "Offer failed: no such enquiry". D-C6-12's answer path
does not work.

*Fix:* Read the enquiry id from `data.answers.record_id` when
`data.answers.kind === "enquiry"`. Add a Vitest case that opens the form
from a quote card, submits, and asserts `quote.set` gets the enquiry's
record id and the `quote_id`.

*Status:* Open.

#### R16 — The form's money input uses float maths and silent fallbacks (major)

*Where:* `crates/roym_web/ui/src/quote_form/rows.ts:264` to `:270`;
`crates/roym_web/ui/src/quote_form/form.ts:217`, `:86` to `:97`.

*Problem:* Focus point 3 asks for integer minor units and no float
maths.

- `rows.ts:268`: when `toMinorUnits` throws (bad input), the code catches
  it and uses `Math.round(Number.parseFloat(valStr) * 10 ** exp) || 0`.
  So "12.345" becomes 1235 (rounded), "-5" becomes -500, "1e3" becomes
  100000, and "abc" becomes 0. The "money-input-invalid" message that
  `toMinorUnits` builds is lost. The person sees a total that is not what
  they typed, with no error.
- `form.ts:217`: the service row's value follows the money row through
  `(minor / 10 ** exp).toFixed(exp)`, a float. `minorToDecimal` exists
  for this.
- `form.ts:86`: only money rows are checked for a value above 0. A
  service or goods row with an empty or 0 value is sent, although D-C6-9
  says the value is required for every commitment. The error key
  `quote-value-required` exists in `formatQuoteError` (`:33`) but nothing
  returns it.

*Failure case:* Bob types "12.345" as the wheat's value. The card says
₹12.35, and the signed terms say 1235. Or he types "abc": the line is
sent at ₹0 and the totals show a difference he did not intend.

*Fix:* Remove the `parseFloat` fallback. Let `read()` return the
`MoneyInputError` text, and show it in the form error line. Use
`minorToDecimal` in `syncAmountToService`. Return
`{ key: "quote-value-required" }` for a non-money row whose value is
empty, and add Vitest cases for each of the three inputs above.

*Status:* Open.

#### R17 — The Playwright deal cases and story 50 Case 3 prove less than their names (major)

*Where:* `crates/substrate/tests/e2e/tests/roym-deal.spec.ts` (all five
cases); `crates/roym_web/ui/src/cards/templates/quote.ts:108`;
`crates/substrate/tests/e2e/demo/50-roym-deal.spec.ts:339`, `:350` to
`:385`.

*Problem:*

- **No node and no person.** Every D case renders a card or a panel
  through `window.RoymRegistry`, `window.RoymMessages` or the new
  `window.RoymDeals`, with hand-written data, and answers every RPC with
  `page.route`. No case signs in (`loginWithDelegatedKey` is imported and
  never used) or uses a real node. So D1 to D5 do not test the Hub's
  real data path, the shapes `agreement.get` or `statement.export`
  return, or the thread wiring. R14 and R15 pass through because of this.
- **D3** puts a ready-made `summary` into the card's data. The quote
  template draws `data.summary` (`quote.ts:108`), a branch that only this
  test uses: the real summary is drawn in `messages.ts:602` from
  `quote.history`. So D3 tests neither `quote.history` nor
  `versionSummary`. The "three-version quote" has one commitment and no
  versions.
- **D4** passes `preFill` into `openQuoteForm` and does not open the form
  from a listing (R13). It asserts only the scope and `INR`.
- **D5** checks only the two file names, not one cell of the CSV or the
  JSON.
- **Story 50 Case 3** (plan Q10 (a) and "How this is shown"): the plan's
  deal is "You give ₹500. You receive ₹600. You receive ₹100 more", and
  then Alice taps "Received", "I gave everything" and confirms the list,
  each line ends, the agreement shows "closed", and the CSV has one line
  per commitment. The story uses values that give equal totals (₹300 and
  ₹300), so the difference line never shows. Bob taps "I delivered",
  Alice taps "I delivered" and "I paid", and then nobody taps "Received"
  or a closing action. So no commitment reaches an end, the agreement is
  not closed, and the statement has no settled row. The story only checks
  that two files with the right names download.

*Failure case:* The Hub sends a wrong shape to `agreement.close`, or the
statement CSV loses a column, or the counterparty cannot settle (R14):
D1 to D5 and story 50 still pass.

*Fix:* Move D1 to D3 to real nodes with the shared harness, as
`roym-hub.spec.ts` cases that use the gateway do: sign in, send a real
offer with `sendQuote`/`addCommitment`, agree, settle, and read the
panel. Remove the `data.summary` branch from `quote.ts` and make D3 send
three versions so the real summary draws. Make D4 open the form from a
listing (R13). In D5, read the downloaded CSV and assert the header and
one row. In story 50 Case 3, use the plan's values (lesson ₹600, wheat
₹400, money ₹100, so Alice receives ₹100 more), assert the difference
line, let Alice tap "Received" on the lesson and Bob "I received
everything", assert "closed" on both panels, and assert one CSV line per
commitment. Record in the agreements change doc's Deviations why any D
case stays mocked.

*Status:* Open.

#### R18 — "When" takes raw Unix seconds (minor)

*Where:* `crates/roym_web/ui/src/quote_form/rows.ts:142` to `:152`,
`:284` to `:290`; `en.ftl:955` ("From (secs)", "To (secs)").

*Problem:* The window fields are `type="number"` inputs for Unix
seconds. An end user cannot type "next week" as seconds since 1970. The
only way to build the "Not started yet" case of AG-46 in the Hub is to
compute such a number. The statement screen already uses local-day date
inputs (D-C6-14).

*Failure case:* Bob wants the lesson next Tuesday. He types "2" in
"From (secs)", and the window starts in 1970.

*Fix:* Use `datetime-local` (or `date`) inputs and turn local time into
seconds, as `statement.ts` does for days. Change the placeholders to the
plan's "When" words.

*Status:* Open.

#### R19 — The change summary does not say which side changed or what changed (minor)

*Where:* `crates/roym_web/ui/src/deals/version_summary.ts`
(`renderVersionSummary`); `crates/roym_web/ui/src/screens/messages.ts:602`
to `:622`.

*Problem:* D-C6-11 and the plan's story say "What changed since the
first offer" **for each side**, with the totals before and after. The
drawn summary has one heading and then lines "Added: X", "Removed: X",
"Changed: X" with no "You give" / "They give" label and no totals.
"Changed" gives only the new description, not the old and new value. A
money commitment's description is built as `"<currency> <amount>"`
(`quote_terms.ts`), so a changed price never shows as "Changed": it
shows as one money line removed and one added. `quote.history` errors
are dropped (`// ignore`), and the call runs again on every thread
redraw.

*Failure case:* Bob lowers his price from ₹500 to ₹400 in version 2.
Alice reads "Added: INR 400.00" and "Removed: INR 500.00", with no word
on whose side it is.

*Fix:* Draw a heading per side ("You give" / "They give") with "before →
after" totals; for `changed`, show the value (and other changed fields)
before and after. Match money rows by giver, kind and payee, not by the
built description. Show a short error line when `quote.history` fails.

*Status:* Open.

#### R20 — The agreement panel misses parts of D-C6-13 and hides load errors (minor)

*Where:* `crates/roym_web/ui/src/deals/agreement_panel.ts:54` to `:65`,
`:190` to `:217`, `:401` to `:409`.

*Problem:* D-C6-13 asks for the description "in the reader's best
language (`bestVersion`)"; the panel shows the original `text` only. It
asks that "A sent half shows its send state and a retry, as other cards
do"; the panel ignores the `state` in `HalfRecordedOutput` and has no
retry. When `agreement.get` fails, the panel stays empty with no message
(`// ignore get errors`); plan section 7.11 says every Hub action shows
its failure and a retry.

*Failure case:* Alice's node cannot reach Bob's when she taps "I paid".
The dialog closes and nothing says the half was not sent.

*Fix:* Use `bestVersion` for descriptions. After a settle, show the send
state from the result, with the retry other cards use. Show an error line
and a "Try again" button when `agreement.get` fails.

*Status:* Open.

#### R21 — The listing editor's fill from saved terms changes a listing's validity and can break its save (minor)

*Where:* `crates/roym_web/ui/src/listings/as_listed_section.ts:71` to `:95`.

*Problem:* The fill replaces the listing's "valid for (hours)" when it
is empty **or "1"**, so a listing that the owner set to 1 hour on
purpose is changed. The saved `quote_valid_secs` may be up to 90 days
(D-C6-16), but a listing allows 1 to 168 hours
(`listing-error-quote-valid-hours`), so a saved validity of, for
example, 10 days fills 240 and the listing save is refused. When the
editor opens an existing listing whose deal text is empty on purpose,
the fill writes the saved text into it, and the next save changes the
signed listing.

*Failure case:* Bob's saved terms have a 7-day validity. He creates a
listing; the hours field shows 168 (fine). With a 10-day validity it
shows 240, and "Save" fails with "Offer validity must be 1 to 168 hours."

*Fix:* Fill only new listings (not an edit of an existing one), only
empty fields (drop the `=== "1"` case), and clamp the hours to the
listing's 1 to 168 range.

*Status:* Open.

#### R22 — The statement's "To" day is left out without telling the person (minor)

*Where:* `crates/roym_web/ui/src/screens/statement.ts:218` to `:228`;
`en.ftl` `statement-to-label`.

*Problem:* `to_secs` is local midnight at the **start** of the "To" day,
so `[from, to)` leaves that whole day out. That follows D-C6-14, but the
label only says "To:". A person who picks 1 to 30 November expects 30
November to be in. "This month" works because it sets "To" to the first
of next month, which the person then sees in the field and may "fix".

*Failure case:* Alice picks 1 Nov to 30 Nov. A lesson settled on 30 Nov
is not in the CSV.

*Fix:* Either add one day to the "To" date before the call (the "To"
day is in), or change the label to "Up to (not included):". Add a test
for a row on the "To" day.

*Status:* Open.

#### R23 — Close-out docs over-claim and miss deviations (minor)

*Where:* M08C `status.md` C6b row; `docs/planning/changes/roym-agreements/change.md`
("Rules and differences in C6b"); `docs/planning/changes/roym-value-exchange/change.md:772`;
`docs/demo/demo.md` R4 case 3; `docs/roym-integrated-experience-spec.md`
C17 to C20.

*Problem:*

- status.md and the PR say Playwright "5 passed" and story 50 passes
  (R12). There is no `mise run verify` result with no `--skip`, which
  plan step 8 and AGENTS.md ask for.
- The agreements doc's C6b block has no deviation for what differs from
  the plan: the listing fill (R13), D1 to D5 run on mocked RPC, the new
  `window.RoymDeals` test hook in `main.ts`, the story 50 values and
  steps (R17), and the missing "This will sign:" message of plan 7.10.
- The value-exchange doc's C6b line (`:772`) is a summary of what was
  built, not a deviation.
- `demo.md` case 3 and the experience spec's C17 ("Each party can settle
  their given and received commitments through the agreement panel")
  claim more than works today (R14, R15). C17 also drops the payment
  request step, which still exists outside the Hub.

*Failure case:* A reader of the living docs or of status.md believes the
counterparty can settle any line in the Hub, and that CI proves it.

*Fix:* After R12 to R17, rerun `mise run verify` with no `--skip` and
copy its counts. Add the deviations above to the agreements doc (or fix
the code so they are not needed). Make the value-exchange line a real
deviation or remove it. Keep the payment request step in C17 with
"(not in the Hub)", and make demo.md say only what story 50 shows.

*Status:* Open.

#### R24 — Rule and duplication points in the new Hub code (minor)

*Where:* `crates/roym_web/ui/src/quote_form/form.ts:125` (`openQuoteForm`,
about 345 lines), `:171` and `:63` (`_rowHandles`);
`crates/roym_web/ui/src/deals/agreement_panel.ts:67`
(`renderAgreementPanelView`, about 325 lines), `:236` to `:258`;
`crates/roym_web/ui/src/screens/statement.ts:215` (the "Show" handler,
about 180 lines); `crates/roym_web/ui/src/cards/templates/quote.ts:270`;
`crates/roym_web/ui/src/deals/close_list.ts:1`, `:38`;
`crates/roym_web/ui/src/screens/thread_redraw.test.ts:5`;
`crates/roym_web/ui/src/i18n/en.ftl` and `hi.ftl` section headers;
`agreement_panel.ts` comment "Closing actions (AG-46)",
`quote_form/defaults.ts` doc comment "(AG-48)",
`agreement_panel.test.ts:198` ("AG-8");
`crates/substrate/tests/e2e/tests/roym-deal.spec.ts:2`.

*Problem:* AGENTS.md's 100-line function limit says "any function"; the
three functions above are 180 to 345 lines. The totals block (give,
receive, difference) is written out three times (quote card, form,
panel). The form keeps its row handles on the DOM element through
`as unknown as` casts. `CloseListItem` has two names for each of two
fields (`notStarted`/`not_started`, `value`/`declared_value_minor`) and
`CloseSide` has six spellings for two sides. `thread_redraw.test.ts`
copies the open-work list instead of using `messages.ts`'s
`OPEN_WORK`, so removing `.settle-dialog` there would not fail the test.
Comments and a test name cite change-doc criterion ids (AG-8, AG-46,
AG-48), and the `.ftl` headers cite "M08C C6b"; the comment rule allows
ADR ids only. `loginWithDelegatedKey` is imported and not used. The PR
says 62 new ids; there are 63.

*Failure case:* A new field in the totals block is added in two of the
three copies; the panel then shows different words from the card.

*Fix:* One `renderTotals(totals, currency)` helper used by all three.
Split the three long functions into named parts (rows, "More terms",
submit; line, actions, closing; fetch, rows, open, totals, EEP). Keep the
row handles in a closure or a `WeakMap`. One name per field and two
`CloseSide` values. Export `OPEN_WORK` and use it in the test. Reword
the comments and the test name without criterion ids, and drop the
milestone from the `.ftl` headers. Remove the unused import.

*Status:* Open.

**Counts of round 1 (part C6b):** 13 findings, R12 to R24: 1 blocker
(R12), 5 major (R13 to R17), 7 minor (R18 to R24). **13 open.**
