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
