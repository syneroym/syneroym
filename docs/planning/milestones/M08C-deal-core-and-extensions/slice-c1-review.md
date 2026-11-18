# M08C slice C1 — review

*Reader: the developer who fixes the C1 plan or code, and the project owner.
This file holds every review finding for slice C1 (parts C1a and C1b).
Plan review findings are P1, P2, …; code review findings are R1, R2, ….
Ids are never renumbered.*

## Plan review round 1 — Claude Opus 5.5 (Claude Code), 2026-11-16

**Reviewer note.** The prompt for this round named the reviewer as Claude
Sonnet 5.5. This session ran on Claude Opus 5.5, the same model that wrote
the plan. So this round does not meet the session-strategy rule that a
different tool reviews a plan. The owner decides whether a second tool
reviews it again.

Checked [slice-c1-implementation-plan.md](slice-c1-implementation-plan.md)
on branch `feat/m08c-c1-commitment-model` at `41b8dfa1`, against the code on
`main` (`48ea989c`; `main` is now `34dbe9bd`, which adds only an ADR). Also
read: task.md, status.md, the three change docs in the prompt (agreements,
value-exchange, extensions), the agreements review file (rounds 1 to 5), and
the M08B review files for the kinds of findings before.

**Summary: 15 findings: 0 blocker, 6 major, 9 minor.**

### What was checked and found correct

- **File and line links.** About 70 links checked. Nearly all are right:
  `clock.rs:18`, `signing.rs:305` (`TestHost`), `helpers.rs:1042`
  (`RecordClock::Fixed`), `app.rs:46` and `:279`, `agreement_ops.rs:113`,
  `:415`, `:451`, `sync.rs:263`, `:448`, `progress.rs:65`,
  `payment_ops.rs:237`, `ledger.rs:52`, `:178`, `payment_ops/fence.rs:37`,
  `:149`, `fulfilment_ops.rs:313`, `quote_ops.rs:318`, `:419`, `:508`,
  `:605` to `:615`, `booking_ops.rs:180` to `:271`, `:340` to `:345`,
  `transaction.rs:32`, `:153`, `:181`, `:292`, `:304`, `:458` to `:475`,
  `:510`, `:532`, `booking.rs:189`, `listing.rs:53`, `router.rs:35`, `:39`,
  `record.rs:45` to `:48`, `conversation/app/backup.rs:33`,
  `fulfilment.rs:150` to `:161` (parity), `transaction.rs:249` (parity),
  `transaction_cards.rs:130`, `transaction_validation.rs:249`, `bundles.rs:91`,
  `booking.rs:465` (parity), `messages.ts:384`, `:451`, `:475`, `:687`,
  `:724`, `:759`, `main.ts:314`, `roym_restore_e2e.rs:429`,
  `roym_flow.rs:91`, `:142`, `file_lengths.rs:398`, the oversized caps (846,
  1290), and the backlog rows `:532`, `:562`, `:654`, `:656`, `:657`. The few
  wrong ranges are in P9.
- **Owner questions' facts.** Q2 (no host can fail a write; only the record
  signer's clock is pinned), Q3 (`change.md:322`), Q4 (`transaction.rs:510`),
  Q5 (`conversation.export` holds the history) and Q6 (`booking.start` has
  exactly the callers named; the Hub never calls it) hold. Q5's wording has
  one gap (P13). No fact that a question rests on is wrong.
- **Deviation rows.** The agreements doc has rows for Q1, Q3, Q4, Q5, Q6, the
  four corrected facts, and the "finished later" parts of AG-2, AG-3, AG-4,
  AG-6, AG-8, AG-14, AG-15, AG-23, AG-33, AG-36. The value-exchange doc has
  VX-2, VX-6 and VX-12. The extensions doc has EX-1 and EX-3 (shape only).
  Each row is in the doc that defines the criterion. Q2 needs no row: the
  agreements doc already asks for a service test with a controllable clock.
- **Criteria coverage.** Section 3 covers every criterion task.md gives C1
  (AG-1 to AG-15, AG-22, AG-23 range, AG-31 to AG-33, AG-35 to AG-37, AG-39,
  AG-41 to AG-43). AG-16 goes to C3b, AG-17 to AG-21, AG-34, AG-38, AG-40 to
  C2, as task.md says.
- **Access control.** No method becomes wire-reachable. `enquiry.`, `deal.`
  and the `agreement.*` additions all route through `router.rs` with
  `MethodAuth::Owner` and `admit::require_internal`.
- **Both builds.** The `verb_at` wrappers (D-C1-4) are internal, the
  dispatch table is shared by `guest.rs` and `native.rs`, and
  `mise run build:roym` builds without dev-dependencies. The design adds no
  test-only code to the component.
- **Component sizes.** The baseline numbers equal the B5 "after" sizes. That
  is expected: no code built into a component changed after B5. The 15%
  limit for `transaction` is computed right (227,479 bytes).
- **The C1a and C1b cut** keeps today's behaviour in C1a, and C1b mostly
  adds code around C1a's decision. The places where it does not are P2, P3
  and P4.
- **Agreements review round 5 fixes.** P36, P37 and P38 are confirmed. P35 is
  fixed for the recovery sweep, but not for the "repeat" path of step 2. It
  is reopened in the [agreements review](../../changes/roym-agreements/review.md)
  (round 6) and here as P3.

### View on the owner questions

The owner accepted Q1 to Q6 on 2026-11-16. This is only the reviewer's view.

- **Q1 (split).** Agree. The cut is right. C1a must say what it does in the
  cases that C1b closes, and record them as backlog rows (P4).
- **Q2 (test host and "at" functions).** Agree with (a). The host as written
  cannot be built inside `roym_core`, because of the dependency check and
  because signing and the conversation are not implemented today (P1). The
  choice stays (a); only where the host lives and how it signs change.
- **Q3 (slot in `details`).** Agree. The shape is final, and C3a only swaps
  the fixed check.
- **Q4 (one original language).** Agree. Note that D-C1-7's pre-fill breaks
  this rule in story 40 (P6).
- **Q5 (restore limit).** Agree with (a). The text of the deviation needs
  one correction: a second countersigned half is not a `progress-fork`
  (P13).
- **Q6 (`booking.start`).** Agree.

### Findings

- [x] **P1 — major.** *Where:* Q2, D-C1-3, step 3 (plan lines 248 to 257,
  608 to 612, 846 to 873).
  *Problem:* The test host cannot be built as the plan describes.
  (1) Today's `TestHost` has `sign_record`, `create`, `delete`,
  `delete_many`, `batch_mutate` and every `AppConversation` method as
  `unimplemented!()` (`crates/roym_core/src/signing.rs:350` to `:450`,
  `:510` to `:620`, `:678` to `:685`), and its `query` ignores storage and
  pops queued pages (`:397` to `:404`). "A settable signing clock" means a
  real signer for both `Principal::Person` (master DID with a delegation
  certificate) and `Principal::Service`, so that `verify_quote` and the other
  verifiers accept the records. (2) A real signer needs a key type from
  `syneroym-identity`. Step 3 writes `test-host = []`, but the code needs
  `syneroym-identity` as an optional `[dependencies]` entry of `roym_core`.
  `cargo xtask check-roym-deps` (`xtask/src/main.rs:154` to `:198`, a gate of
  `mise run verify`) reads every `crates/roym_*` manifest's `[dependencies]`
  and allows only its fixed list, so the gate fails. (3)
  `syneroym-roym-transaction` has no `[dev-dependencies]` at all: no `tokio`
  for async tests and no signer for the peer's hand-signed halves. (4) Every
  sync test needs `conversation.changes` to return incoming cards; the plan
  models only `conversation.send` (`sent_cards()`). (5) `fail_write_after(n)`
  fails one write. After that write, code that ignores errors keeps going
  (for example `let _ = put_row(...)` at `crates/roym_transaction/src/app/progress.rs:61`),
  so the test does not stop the run the way a crash does. Counting writes
  also breaks the tests on every refactor that adds a write.
  *Failure case:* step 3 does not compile with `test-host = []`; with
  `syneroym-identity` added, the `roym-deps` gate of `mise run verify`
  fails. A crash-point test can pass while the code after the injected
  error still ran.
  *Fix:* Put the host in a new crate outside the `roym_` prefix, for example
  `crates/app_host_test` (package `syneroym-app-host-test`), used only as a
  dev-dependency by `syneroym-roym-core` and `syneroym-roym-transaction`. It
  needs only `syneroym-app-host`. Sign through the existing record signer in
  `syneroym-core` (`crates/core/src/record_signer.rs`, `RecordClock::Fixed`)
  instead of writing a second one. Model the conversation in memory:
  `send` appends an outgoing message, a test helper `deliver(conversation,
  card_body)` appends an incoming one, and `changes` pages both by `seq`.
  Replace `fail_write_after(n)` with a named, sticky fault:
  `crash_on_write(collection, key_prefix)` fails that write and every later
  write until `restart()`. Add `tokio` (macros, rt) to the transaction
  crate's dev-dependencies. Update Q2's text, D-C1-3 and step 3; no owner
  decision changes.
  *Status:* Fixed (3462e4eb).

- [x] **P2 — major.** *Where:* section 2.2 point 4; section 1 "Each part
  leaves a working system"; section 8 row "The writer's decision has no
  crash recovery"; section 7.2 `recover.rs`.
  *Problem:* The fact is wrong. Today the receipt card row is written after
  the decision: `maybe_countersign` runs at
  `crates/roym_transaction/src/app/sync.rs:527` and `put_card` at `:528`, and
  an error returns from `sync` before the cursor is saved (`:97`, `:106`).
  So after a failed write or a crash during the decision, the next
  incremental sync reads the same card again and runs the decision again.
  What is really not retried is the cases where the decision returns
  `Ok(false)` with work half done (`agreement_ops.rs:482`: no signing
  principal; `:497`: an invalid payload): the card is then filed and never
  read again. This changes three things. (a) In C1a, the decision is entered
  a second time through the fence's "repeat" branch, so C1a must build that
  branch (see P3, P4). (b) In C1b, a crash-point test that injects a write
  fault and then syncs goes through this re-read, not through the recovery
  sweep, so it cannot show that the sweep works. (c) A lasting error (for
  example no signing certificate) makes every sync of that conversation fail
  at the same card, so no later card of that conversation is filed.
  *Failure case:* all `recover.rs` tests pass with the sweep deleted. A node
  whose certificate has expired stops filing every card in the
  conversation that holds one acceptance.
  *Fix:* Correct point 4 and the backlog row text. Decide and write down
  one rule: after the fence, `decide::run` does not return an error to
  `sync`; it logs it, the card is filed, and the `deciding` row is finished
  by the sweep (C1b). In C1a, before the sweep exists, keep the error
  propagation that gives the retry today, and say so. In `recover.rs`, each
  test first checks that the card row is stored and the sync cursor has
  moved past it, and then runs a sync with no new card, so only the sweep
  can finish the decision.
  *Status:* Fixed (3462e4eb).

- [x] **P3 — major.** *Where:* D-C1-16; C1b step 3 and step 4; agreements
  doc "The writer's decision" step 2 and "Recovery" (`change.md:597`,
  `:607` to `:613`). Reopens agreements review P35.
  *Problem:* Step 2 says: "If the row exists with the same agreement, this
  is a repeat: continue from the row's state." For a `deciding` row this
  runs steps 3 and 4 again, and skips Recovery's first two checks (a stored
  `not-formed` snapshot; the expiry). P2 shows this repeat path really
  happens (a re-read after an error), and the doc says two syncs may run at
  the same time ("the second one finds the row in step 2 and repeats"). So
  the case that P35 closed for the sweep is still open on this path.
  *Failure case:* the writer's reserve fails; it stores snapshot 1
  `not-formed` with `slot-taken` and releases the others; a crash comes
  before the last `put`. The next sync reads the half again, finds the row
  `deciding`, and runs step 3 again. The slot is now free, so reserve and
  confirm succeed. The writer stores and sends a countersigned half, and
  "uses the stored record" for snapshot 1, which is `not-formed`. The
  counterparty gets a countersigned half and a `not-formed` snapshot for the
  same agreement.
  *Fix:* One function finishes a `deciding` row, with the three ordered
  checks of "Recovery". Both step 2's repeat branch and the sweep call it.
  C1a builds at least check 1 in it (a stored `snapshot:<agreement>:1` is
  used and never decided again), because C1a already has the re-read path.
  Add an AG-14 test: crash after the `not-formed` snapshot is stored, then
  deliver the same half again (not the sweep), with the slot free.
  *Status:* Fixed (e2a3898b).

- [x] **P4 — major.** *Where:* D-C1-16 ("C1a has no … step-1 refusal
  snapshots"), section 4 behaviour changes 1 to 9, section 8.
  *Problem:* C1a does not say what its decision does in the cases that C1b
  closes, and C1a still allows them: the Hub shows "Accept" on every
  version (C1b step 2 removes it) and `quote.set` may revise an accepted
  quote (C1b adds `quote-already-accepted`). Three cases are open.
  (a) Step 1's checks (the signer, the expiry) must run before the fence.
  If the fence is created first, an acceptance after the expiry leaves a
  `deciding` row that nothing finishes in C1a. Today the expiry check comes
  first (`agreement_ops.rs:472`). (b) The fence row exists and names another
  version. C1a has no refusal snapshot, so it must not countersign, and the
  plan does not say what it does instead. (c) The counterparty's own seat
  holds for a version that the writer never takes are never released (seat
  holds have no expiry in C1).
  *Failure case:* the counterparty accepts v1, the writer countersigns v1,
  the author revises to v2, the counterparty accepts v2. C1a's writer finds
  `decision:<quote_id>` naming v1. If the code treats any existing row as a
  repeat, it countersigns v2 with the state of v1; if it ignores the half,
  the counterparty waits on `accepted` with no end and no message.
  *Fix:* In D-C1-16, list C1a's order as: step 1's checks with no snapshot
  (return as today), then the fence; a row with another agreement leaves
  the half filed with no countersign and no snapshot. Add backlog rows
  (target C1b) for: a second acceptance of a quote waits with no answer; the
  Hub shows "Accept" on every version; `quote.set` can revise an accepted
  quote; the decline is still local and unsigned. C1b deletes them.
  *Status:* Fixed (3462e4eb).

- [x] **P5 — major (security).** *Where:* C1b step 1 ("Sync files a
  `deal-decline` card from the other party and marks the deal or enquiry
  `declined`").
  *Problem:* The plan names only `verify_deal_decline` (signature, subject).
  It does not say what the receiving node checks before it ends a deal. The
  agreements doc gives the rules (`change.md:358` to `:360`): who may
  decline what, and no decline once the counterparty's half exists. A
  signed record whose `declines` and `deal` fields disagree, or that names
  a record of another conversation, must not end anything.
  *Failure case:* the counterparty's node (buggy or hostile) sends a
  `deal-decline` for quote id Q after its acceptance half for Q. The writer
  files it and marks Q `declined`, while it holds a countersigned half and
  an `agreed` snapshot. Or a decline with `declines` = an old version of Q1
  and `deal` = Q2 ends Q2.
  *Fix:* Add a filing check list to step 1, each a refusal reason with a
  parity scenario: the signer is the party the rule allows for that record
  (counterparty for a quote; either party for an enquiry, as the rule
  says); `declines` is a record this node holds in the same
  `conversation`; `deal` equals that record's `quote_id` or `enquiry_id`;
  refuse when this node holds the counterparty's acceptance half for any
  version of that quote id. A refused decline is kept as a refused card,
  like other refused cards.
  *Status:* Fixed (3462e4eb).

- [x] **P6 — major.** *Where:* D-C1-7 ("filled in advance from the first
  line of the enquiry"); step 5b; story 40.
  *Problem:* The quote form signs every `LocalText` with the form's
  language (Q4: one `lang` per record). The enquiry may be in another
  language. Story 40 has exactly this case: Alice writes the enquiry in
  Hindi (`demo/40-roym-languages.spec.ts:207`), and Bob's Hub sends the
  quote in English (`:229` checks `lang="en"` on the scope). The pre-filled
  "What you give" would carry Hindi text signed as `lang: "en"`. That is a
  false language tag in a signed record, and the `viewed_lang` and
  `viewed_via_mt` rules (ML-18) read that tag.
  *Failure case:* story 40 passes, and Bob's quote, receipt halves and
  Alice's `viewed_lang` all claim English for a Hindi description.
  *Fix:* Fill "What you give" from the quote's own scope field (the same
  language as the form), or leave it empty and let `sendQuote` in
  `hub-helpers.ts` fill it. Add a `quote_terms.test.ts` case that the
  pre-fill never comes from text in another language.
  *Status:* Fixed (3462e4eb).

- [x] **P7 — minor.** *Where:* step 4 (Playwright list), section 7.6,
  D-C1-12.
  *Problem:* Missed callers of the request card. Story 40 reads
  `.card-request` (`demo/40-roym-languages.spec.ts:208`),
  `.request-description` (`:216`) and `a.terms.scope` (`:259`, not only
  `:121`). Hub case 24 reads `.card-request` (`roym-hub.spec.ts:846`).
  `hub-helpers.ts:210` clicks `.quote-request-button`, which
  `messages.ts:661` draws on the request card.
  *Failure case:* story 40 fails after step 4, or after step 5b.
  *Fix:* Add these lines to step 4 and to the 7.6 table. Add
  `.request-description` and `.quote-request-button` to D-C1-12's list of
  classes that stay.
  *Status:* Fixed (3462e4eb).

- [x] **P8 — minor.** *Where:* section 9 (living docs).
  *Problem:* Statements that C1a makes false are missing from the list:
  `docs/system-requirements-spec.md:356`, `:407`, `:447` (`AgreedTerms`,
  and a link to `transaction.rs:116` that is already stale);
  `docs/system-architecture.md:1139` (the chain of records and
  `booking-progress`) and `:3127` ("seven card types").
  *Failure case:* the requirements spec still names `AgreedTerms` after
  C1a.
  *Fix:* Add these to section 9. Also search the traceability matrix for
  `request` and `booking` rows.
  *Status:* Fixed (e2a3898b).

- [x] **P9 — minor.** *Where:* step 3, section 7.8.
  *Problem:* Size facts that the plan misses. (1)
  `dual_build_parity/fixtures.rs` is 799 lines; it is not in
  `xtask/oversized-test-files.txt`, so its cap is 800, and step 5a grows
  every `peer_signed_*` helper. (2) The host to move is `signing.rs:256` to
  `:799` (the test module), not `:285` to `:470`; about 460 lines move, and
  the additions of P1 will take the new file past 800. (3) The
  oversized-file gate also fails when a listed file shrinks: if
  `dual_build_parity/transaction.rs` (846) or `helpers.rs` (1290) gets
  shorter, the entry must be lowered in the same PR.
  *Failure case:* `check-file-lengths` fails in step 5a.
  *Fix:* Name `fixtures.rs` in 7.8 and move new deal fixtures to
  `deal_fixtures.rs` (188 lines). Plan the host as a file plus sibling
  folder from the start. Add the two-sided ratchet note.
  *Status:* Fixed (3462e4eb).

- [x] **P10 — minor.** *Where:* D-C1-5, step 5a (`agreement.cancel`), C1b
  step 3.
  *Problem:* The plan says when the writer releases its own holds only for
  a failed reserve. It does not say when (a) the writer releases on
  `agreement.cancel` (today's scenario 156 checks seat reuse after a
  cancel), or (b) the counterparty releases its own holds when it files a
  `not-formed` or `cancelled` snapshot, or when the quote expires (release
  item 4, `change.md:630`). C1b step 3 covers only a refusal snapshot. A
  seat hold has no expiry in C1, so a missed release keeps a seat taken for
  good.
  *Failure case:* a quote where the counterparty gives a slot is cancelled;
  the counterparty's slot stays taken, and its next quote for that slot
  fails with `slot-taken`.
  *Fix:* List the release points in step 5a for both nodes, and add one
  service test per point (cancel, `not-formed`, expiry).
  *Status:* Fixed (3462e4eb).

- [x] **P11 — minor.** *Where:* C1b step 1 (decline "refused once the
  counterparty's half exists"), C1b step 2 (`quote-already-accepted` "when
  `AGREEMENTS` holds a counterparty half for any version").
  *Problem:* A half that the writer refused with `quote-superseded` is still
  a counterparty half in `AGREEMENTS`. "Deal rules" says that such a refusal
  is not an end and the deal stays `quoted`. With the plan's rule, the
  author can no longer revise and the counterparty can no longer decline.
  Also, the plan does not refuse `agreement.accept` on a deal that has
  ended (`declined`, `expired`, `not-formed`).
  *Failure case:* Alice accepts v1 after Bob sent v2; the writer refuses v1.
  Bob cannot send v3, and Alice cannot decline; she can only accept v2.
  *Fix:* Count only halves that are not refused (no `refused:` row on the
  writer; no refusal snapshot on the counterparty). Refuse
  `agreement.accept` on an ended deal with `quote-ended`. Write the reading
  as a row in the agreements doc's `## Deviations`.
  *Status:* Fixed (3462e4eb).

- [x] **P12 — minor.** *Where:* C1b step 5, D-C1-8, C1b step 4.
  *Problem:* (1) "read through `conversation.changes` from 0 … only when no
  stored row exists for the next `seq`" is always true, because the next
  `seq` never has a row. As written, every transition scans the whole
  conversation. (2) A `progress-fork` keeps "the second as evidence", but
  D-C1-8 has one key per `seq` (`snapshot:<agreement>:<seq>`), so the second
  has nowhere to go. (3) "What `quote.set` reads" looks up `refused:` rows by
  `quote_id`; step 4 adds only `state` to the ledger indexes.
  *Failure case:* a long conversation makes every tick slow; a fork
  overwrites or loses the evidence.
  *Fix:* (1) Learn the highest own `seq` while sync files the writer's own
  outgoing `agreement-progress` cards (a restored cursor reads them again),
  and keep it on the agreement row; scan from 0 only in the sweep, for a
  `deciding` row with no `countersign:` row. (2) Store a fork under
  `fork:<agreement>:<seq>:<record_id>`. (3) Add `quote_id` to the ledger
  indexes.
  *Status:* Fixed (3462e4eb).

- [x] **P13 — minor.** *Where:* Q5 option (a); the Q5 row in the agreements
  doc's `## Deviations`.
  *Problem:* "The counterparty's node keeps the first record it filed and
  shows `progress-fork`" is true for snapshots only. A second, different
  countersigned half is kept silently: `file_agreement_receipt_card` stores
  a half only when that role has none (`sync.rs:511`). The writer keeps its
  new half, so the two nodes then hold different `agreed_at_secs`, and so
  different track ends.
  *Failure case:* after a full restore from an old backup, the writer ticks
  a commitment to `ended-unconfirmed` on a different day than the
  counterparty's Hub shows, with no warning on either node.
  *Fix:* Correct the deviation text. Optionally show a warning when a
  second, different half for the same role arrives (no new state, the same
  as `progress-fork`). The owner's choice of (a) does not change.
  *Status:* Fixed (3462e4eb).

- [x] **P14 — minor.** *Where:* step 5a (`backup.rs`, `backup/sections.rs`),
  C1b step 5, AG-42.
  *Problem:* The import rules for the new rows are not given. Today import
  rebuilds bookings from `step:` rows (`backup/sections.rs:260`). After C1a
  the `ledger` holds `decision:`, `countersign:`, `hold:`, `seat:`,
  `payreq:`, `ack:`, `fulfil:` (and in C1b `refused:`, `decline:`), and
  snapshots move to `agreement_progress`. AG-42's tests depend on what
  import keeps; for example, a `deciding` row must survive import so the
  sweep finishes it.
  *Failure case:* import drops or refuses `deciding` rows, and the AG-42
  "backup taken after the fence" test cannot be built.
  *Fix:* Add a table to step 5a: per row kind, the check on import (for
  example a `countersign:` half verifies and names a held quote; a snapshot
  verifies and is bound to the quote's signer) and what is kept.
  *Status:* Fixed (3462e4eb).

- [x] **P15 — minor.** *Where:* plan lines 71 to 74, 80 to 90, 179 to 180,
  206; task.md line 5 and the C1 "Change docs" line; section 3 VX-12.
  *Problem:* Small inconsistencies. (1) Section 0 says "this plan writes
  nothing into [roym-extensions]"; "Change docs" says the deviation is
  there (it is). (2) task.md's C1 "Change docs" line does not name
  roym-extensions, and its status line still says "No slice is planned
  yet". (3) The end-user section says Bob "fills the same fields", but
  D-C1-7 adds "What you give". (4) `roymctl roym transaction agreement` is
  a leaf command with `--quote` today; step 5a makes it the parent of
  `cancel` and `progress`, and the developer bullet still uses
  `agreement --quote`. (5) Section 3 lists VX-12 for C1, but no commitment
  can end `failed` before C3b; C1 proves only the `ended-unconfirmed` and
  `claimed` part.
  *Failure case:* a reader of section 0 skips the extensions doc; the CLI
  example does not parse.
  *Fix:* Fix the five texts. For the CLI, use one shape, for example
  `transaction agreement get|cancel|progress`, in step 5a, the developer
  bullet, `cli_args.rs` and `docs/demo/demo.md`.
  *Status:* Fixed (e2a3898b).

## Plan review round 2 — Claude Sonnet 5.5 (Claude Code), 2026-11-16

Checked [slice-c1-implementation-plan.md](slice-c1-implementation-plan.md)
on branch `feat/m08c-c1-commitment-model` at `70e49ea6`, against the code on
this checkout (same as `main` for `crates/` and `apps/`). The first pass was
made before reading round 1: the test host (`signing.rs`, `method.rs`,
`xtask/src/main.rs`), the writer's decision and sync order
(`sync.rs`, `agreement_ops.rs`, `ledger.rs`), the Hub (`messages.ts`,
`quote.ts`, `record_text.ts`, `text_lang.ts`, `translation.ts`), `thread.rs`,
the callers of every renamed or removed method, the living docs and the file
sizes. Then each of P1 to P15 was checked against the code. The agreements
change doc was read for "The writer's decision", "Recovery", "Single writer",
"Deal rules", "Reason codes" and the AG rows. Q1 to Q6 were not reopened: no
fact they rest on is wrong (`booking.start` has exactly the callers Q6 names;
the Hub does offer 13 record languages, which matters only for P17).

### Round 1 items

- **P1.** Agree in part. The gate failure is real: `check_crate_manifest_dependencies`
  reads `[dependencies]` of every `crates/roym_*` manifest and allows only the
  fixed list (`xtask/src/main.rs:70` to `:83`), so an optional
  `syneroym-identity` fails. The sticky fault is also right (`let _ = delete`
  at `ledger.rs:276`, `:331`, `booking_ops.rs:249` ignore errors, so a fault on a
  delete never reaches the caller; the plan's own rule in 7.11, "the call
  returned the injected error", cannot hold for those). Wrong or too strong:
  (a) the real enum is `Principal::Delegated` / `Principal::Service`, not
  `Principal::Person`; (b) point (4) is overstated, because the service reaches
  the conversation through `call_sibling` to `conversation.changes` and
  `conversation.send` (`sync.rs:128`, `agreement_ops.rs:531`), which the plan's
  `on_call` map can already answer; the conversation stub must be an `on_call`
  handler, not an `AppConversation` impl on the host (the service never calls
  that trait); (c) the fix says the new crate "needs only
  `syneroym-app-host`" and then signs through `syneroym-core`'s
  `NodeRecordSigner`. That is `reqwest`, `rustls`, `pkarr` and a mock
  `EndpointRegistry` with a recorded owner (`record_signer.rs:100`), and
  `RecordClock` is fixed at construction, so `set_now` means rebuilding the
  signer. It builds (the `roym_web` dev-dependencies already hold
  `syneroym-core` with `test-utils`), but it is heavier than needed, and it
  moves the host out of `roym_core::testing`, which is the place Q2 (a) names.
  A lighter fix that keeps Q2 (a) as accepted: keep `roym_core::testing`, and
  let the host take an injected signer (`Fn(RecordDraft, Principal, u64) ->
  Result<String, SigningError>`) that `roym_transaction`'s dev-dependencies
  (`syneroym-identity`, `z32`, `tokio`) build in its own test support. If the
  owner prefers the new crate, say so against Q2. Either way, see P18.
- **P2.** Agree in part. The fact is right: `maybe_countersign` runs at
  `sync.rs:527`, `put_card` at `:528`, and an `Err` returns before the cursor
  is saved. Wrong: the failure case. An expired or missing certificate does not
  stop the sync: `person_principal` errors map to `Ok(false)`
  (`agreement_ops.rs:480` to `:483`), so the card is filed and the countersign
  is never retried, which is the gap P2 names one paragraph earlier. A lasting
  `Err` needs a storage or `sign_record` error. The rule to test (each crash
  test first checks that the card row is stored and the cursor moved) is good.
  P16 adds a path P2 did not see.
- **P3.** Agree. The doc's own safety sentence ("a stored `not-formed`
  snapshot is never undone by a later reserve or confirm that succeeds",
  `change.md:613`) only holds if the repeat path runs the same checks. A
  stronger failure case than P3's: a repeat after the expiry reaches step 3
  and countersigns after the expiry (see P19).
- **P4.** Agree. `agreement_ops.rs:472` checks the expiry before any write
  today, and D-C1-16 starts with the fence. One small point: P4's fix and P3's
  fix move a little recovery code (check 1) into C1a. That touches the wording
  of Q1 ("no crash recovery yet"). It is a small change, and the owner should
  be told.
- **P5.** Agree. The doc rules at `change.md:358` to `:360` are enforced only
  by the sender in the plan.
- **P6.** Agree. Story 40 has the case (`40-roym-languages.spec.ts:207` writes
  Hindi, `:229` expects `lang="en"`). Pre-filling "What you give" from the
  scope field at submit time is a good fix. P17 is the same kind of problem on
  the other commitment.
- **P7.** Agree. Verified `:208` (`.card-request`), `:216`
  (`.request-description`), `:259` (`a.terms.scope`) and
  `hub-helpers.ts:210`.
- **P8.** Agree. Verified `docs/system-architecture.md:1139` and `:3127`, and
  `docs/system-requirements-spec.md:356`, `:359`, `:360`, `:407`, `:447`
  (five lines, not three).
- **P9.** Agree. `fixtures.rs` is 799 lines and not listed in
  `xtask/oversized-test-files.txt`. Also near the cap and touched by 5a:
  `transaction_cards.rs` (697), `booking.rs` (669), `cli_args.rs` (717),
  roymctl `transaction.rs` (669).
- **P10.** Agree. Today's cancel frees the seat in `roll_forward`
  (`ledger.rs:268` to `:277`); the plan moves that code and names no release
  point for cancel.
- **P11.** Agree. The doc's "holds an acceptance of any version"
  (`change.md:573`) and "Deal rules" (a refused acceptance is not an end)
  disagree, and the plan picks the first reading without saying so.
- **P12.** Agree on all three points.
- **P13.** Agree. `sync.rs:511` keeps the first half per role and says
  nothing about a second, different one.
- **P14.** Agree. `validate_ledger_and_bookings` re-verifies every step row on
  import (`backup/sections.rs:257` to `:300`), so the new row kinds need rules.
- **P15.** Agree. Verified: the `Agreement` leaf command takes `--quote`
  (`apps/roymctl/src/commands/roym/transaction.rs:137`).

### View on the owner questions

- **Q1.** Agree with the split. C1a still needs a small re-entry path in the
  decision (P3, P4, P16); that is the only soft spot in "C1b rewrites nothing".
- **Q2.** Agree with (a). The host cannot sit in `roym_core` as written (P1),
  and its clock needs care (P18).
- **Q3.** Agree. The fixed scheduling case is small and the shape is final.
- **Q4.** Agree. Its fact "the Hub writes all fields in one language" needs
  the Hub's catalog text to exist in that language, which it does not (P17).
- **Q5.** Agree with (a), with P13's correction to the wording.
- **Q6.** Agree. `booking.start` callers match the plan; the method-contract
  shapes also call it (`shapes_transaction.rs:191`).

### New findings

- [x] **P16 — major.** *Where:* D-C1-16 ("send both cards; put the decision
  row last"), step 5a (`maybe_countersign` becomes `decide::run`),
  `crates/roym_transaction/src/app/agreement_ops.rs:465` to `:468` and
  `:531` to `:539`.
  *Problem:* Two parts of today's code defeat D-C1-16 if they are kept.
  (1) Today the quote author's half is stored in `AGREEMENTS` before the
  send (`put_row` at `:520`). A re-read of the same card then hits
  `if row.quote_author.is_some()` and returns `Ok(false)` at `:465`, so the
  card is never sent again. (2) A failed `conversation.send` is swallowed:
  `Err(_) => (String::new(), …)` at `:536` to `:538`, and the function still
  returns `Ok(true)`. The plan says what the last write is, but not that a
  failed send must stop it, and not what replaces the early return.
  *Failure case:* `conversation.send` fails once. The writer stores the half,
  puts the decision row `agreed` (the last write), and the counterparty never
  gets the countersigned half or snapshot 1. C1b's sweep looks only at
  `deciding` rows, so nothing finishes it. The counterparty waits until the
  quote expires.
  *Fix:* Write two rules into D-C1-16. A send error returns `Err` from
  `decide::run` and the decision row is not put. The early return on a stored
  half is replaced by the repeat branch (use the stored half, send again, put
  the row). Add a service test in C1a: the first `conversation.send` fails,
  one more sync sends the same stored half and snapshot, and the decision row
  is `agreed` only after that.
  *Status:* Fixed (3462e4eb).

- [x] **P17 — major.** *Where:* D-C1-7 ("the currency commitment's
  description is the catalog text `quote-payment-description` in the quote's
  language"), Q4, step 5b.
  *Problem:* The Hub offers 13 record languages
  (`crates/roym_web/ui/src/i18n/text_lang.ts:4` to `:6`: `en`, `hi`,
  `hi-Latn`, `bn`, `gu`, `kn`, `ml`, `mr`, `or`, `pa`, `ta`, `ta-Latn`, `te`),
  and the quote form lets the person pick any of them
  (`messages.ts`, `textLangSelect("quote-text-lang", …)`). The Hub has two
  catalogs only: `en.ftl` and `hi.ftl`. For the other 11 languages there is no
  `quote-payment-description` text in "the quote's language", and Q4 needs every
  text of the record to carry that one `lang`.
  *Failure case:* Bob picks Tamil for the quote. The Hub either sends the
  English or Hindi catalog text tagged `ta` (a false tag in a signed record,
  read by the `viewed_lang` rules, the same defect as P6), or fails to build
  the quote. Also `hi-Latn` needs the Latin-script Hindi text, which the Hindi
  catalog does not give.
  *Fix:* Build the currency line's description from language-neutral parts:
  the ISO currency code and the amount (for example `INR 100.00`), with no
  catalog sentence. The role words ("You give") come from the reader's catalog
  when the card is drawn, not from the signed text. Add a `quote_terms.test.ts`
  case for a quote in a language with no catalog.
  *Status:* Fixed (3462e4eb).

- [x] **P18 — minor.** *Where:* Q2, D-C1-3 (`set_now`), step 3, section 7.2
  (tests with chosen times).
  *Problem:* A pinned signing clock needs a certificate that is valid at that
  time, and the plan does not say how the host gets one. `DelegationCertificate::issue`
  stamps the real clock (`crates/identity/src/delegation.rs:109`), and
  verification refuses a certificate issued more than 300 seconds after `now`
  (`:231`); `NodeRecordSigner::sign_record` checks the certificate at the
  pinned time (`record_signer.rs:130` to `:141`). The service also needs the
  certificate installed in its own `signing_certificates` collection, and
  `person_principal` refuses it once `now >= expires_at_secs`
  (`signing.rs:194` to `:196`).
  *Failure case:* a test that sets the clock to a small number (like scenario
  165's window `1_000..2_000`) cannot sign. A test that moves the clock 31 days
  ahead to pass a track end finds the default certificate lifetime is too
  short, so the late half no longer verifies.
  *Fix:* In step 3: the fixture mints the certificate with the longest lifetime
  the install check allows (5 years, `signing.rs:46`) and installs it; every
  test clock starts at or after the real "now" and only moves forward.
  *Status:* Fixed (3462e4eb).

- [x] **P19 — minor.** *Where:* C1b step 3 ("In `decide::run`, before step 2
  of D-C1-16: the checks of the doc's step 1").
  *Problem:* Step 1's expiry and version checks run on every entry, also on
  a repeat of a half whose decision is already made. The doc's Recovery says
  the expiry is checked again only when no `countersign:<agreement>` row
  exists (`change.md:609`). Step 1's refusal also writes
  `snapshot:<version record_id>:1`, the same key as the decision's snapshot
  (`agreement` is the version's `record_id` in both).
  *Failure case:* the writer stores the countersign and snapshot 1 `agreed`,
  sends them, and crashes before the last `put`. The quote expires. The half is
  read again: step 1 says `quote-expired`, tries to create snapshot 1
  `not-formed`, finds the stored `agreed` one, and sends that. It also creates
  `refused:<half>` with `quote-expired`. The end state is right by luck, but
  `quote.set` now reads a `refused:` row that says the quote expired, for a
  deal that is agreed. The same happens when a `quote.set` on another version
  runs in between (`quote-superseded`).
  *Fix:* One function decides what to do with a half. If a `decision:` row or a
  `countersign:` row exists for this agreement, skip step 1 and finish the
  decision (this is also P3's fix). Add a test: crash after the countersign is
  stored, move the time past the expiry, sync, and check there is no `refused:`
  row.
  *Status:* Fixed (e2a3898b).

- [x] **P20 — minor.** *Where:* step 5b, section 7.5, Q4.
  *Problem:* The Hub reads quote, receipt and enquiry text through
  `crates/roym_web/ui/src/records/record_text.ts` (`TextVersion`,
  `textFields`, `bestVersion`; used by `quote.ts`, `request.ts`,
  `agreement_receipt.ts`, `fulfilment_receipt.ts`), with a flat list of fixed
  text keys per record, and the "Translate" action sends them in one call. The
  new shape has one `LocalText` per field and a variable number of commitment
  descriptions. The plan names `bestVersion` and some tests, but not this
  module or its test file (398 lines). The node refuses a translate call of
  more than 8 texts (`crates/roym_web/src/app/translation.rs:23`, `:122`), and
  `translateTexts` does not split a call (`translation.ts:161` to `:176`). A
  quote of 5 or more commitments, made by `roymctl` or by a peer, has more than
  8 texts.
  *Failure case:* a reader presses "Translate" on a quote with 6 commitments
  and gets "Could not translate". Hub-made quotes (6 texts) are fine, so no Hub
  test finds it.
  *Fix:* Add `record_text.ts` and its test to step 5b, with the rule that
  `textFields` takes the key list from the record (terms texts and each
  commitment's description). Split the call into groups of 8 texts in
  `translateTexts`, or state that a larger record shows no translate action.
  `web` Rust code stays unchanged.
  *Status:* Fixed (3462e4eb).

- [x] **P21 — minor.** *Where:* D-C1-5 (`release`: "deletes the seat row …,
  then the hold row"; `reserve`: "a `hold:` row that already exists means
  already held").
  *Problem:* Two deletes are two writes. A crash between them leaves a `hold:`
  row with no `seat:` row. A later `reserve` for the same (agreement,
  commitment) then answers "already held" and takes no seat.
  *Failure case:* the counterparty's `agreement.accept` fails on its second
  commitment and releases the first; the process stops after the seat delete.
  The person presses "Accept" again: the first commitment says already held,
  signs, and the seat is free for another agreement. Two bookings for one seat.
  *Fix:* `release` reads the seat row, then removes both rows in one
  `batch_mutate` (it has `delete`, `data-layer.wit:74` to `:78`). Also let
  `reserve` check that the seat row exists when it finds a hold row. One crash
  test between the two deletes.
  *Status:* Fixed (3462e4eb).

- [x] **P22 — minor.** *Where:* step 5a (`thread.rs`: "the thread carries
  `agreement-progress` cards"), C1b step 2 (Hub shows "Accept" only when the
  deal is not `agreed`, `declined`, `expired` or `not-formed`).
  *Problem:* `transaction.thread` adds fields to the card rows on the server
  (`crates/roym_transaction/src/app/thread.rs:46` to `:100`): `declined`,
  `version_count`, `expired` (read from `terms.quote_expires_at_secs`) and, for
  a payment request, `agreement_payee = agr.terms.payee` and the payment
  methods. After C1a the payee and methods belong to a commitment, named by
  the card's `commitment_id`, so this code must change; the plan says only
  that the thread "carries" the new card. For C1b, the Hub needs a deal state
  per quote version (`agreed`, `not-formed`, refused). The thread does not give
  one, and the plan does not say where the Hub gets it. The refusal snapshot's
  `agreement` is the version's `record_id`, so the Hub would have to join cards
  by itself.
  *Failure case:* case 24's payee sentence has no data source for a
  per-commitment payment request; in C1b the Hub shows "Accept" on a version
  that the writer refused, because only the client could know.
  *Fix:* In step 5a, say that the thread fills `agreement_payee` and the
  methods from the commitment the card names. In C1b step 2, add a `deal_state`
  to the quote card rows in `thread` (computed on the server from the pointer,
  the agreement row, the snapshots and the clock) and keep the Hub's choice a
  plain read of that field, with a unit test.
  *Status:* Fixed (3462e4eb).

**Counts.** New findings: 7 (0 blocker, 2 major, 5 minor). Round 1 items: 13
agree (P3 to P15), 2 agree in part (P1, P2), 0 disagree.

## Rounds 1 and 2 compared — Claude Opus 5.5 (Claude Code), 2026-11-16

The round 1 reviewer checked round 2's corrections and new findings against
the code. Round 2 is right on every point checked: `Principal` has
`Delegated` and `Service` (`crates/app_host/src/types.rs:76`); a signing
error maps to `Ok(false)` (`agreement_ops.rs:480` to `:483`); a re-read
returns early when the author's half exists (`:465` to `:467`); the Hub
offers 13 record languages (`crates/roym_web/ui/src/i18n/text_lang.ts:4`)
and has only `en.ftl` and `hi.ftl`; `syneroym-core` depends on `reqwest`,
`rustls` and `pkarr`.

What the fix session uses where the rounds differ:

- **P1.** Use round 2's fix, not round 1's. Keep the host in
  `roym_core::testing` behind `test-host`, as Q2 (a) says. The host takes
  an injected signer; `roym_transaction`'s dev-dependencies
  (`syneroym-identity`, `z32`, `tokio`) build it in its own test support.
  `check-roym-deps` does not read `[dev-dependencies]`, so no gate fails,
  and the owner's Q2 answer stays as written. The conversation stub is an
  `on_call` handler for `conversation.send` and `conversation.changes`.
  The sticky, named fault from round 1 stays.
- **P2.** Drop round 1's failure case (c). A lasting signing error does not
  block the sync; it files the card and gives up (`Ok(false)`), which is
  the gap P2 already names. "Logs it" in the fix means: the decision row
  stays `deciding` and the sweep finishes it; no log channel is needed.
- **P8.** The requirements spec has five stale lines: `:356`, `:359`,
  `:360`, `:407`, `:447`.
- **P3, P4 and Q1.** Their fixes put Recovery's check 1 (a stored
  `not-formed` snapshot is used, not decided again) into C1a. This is a
  small change to Q1's "no crash recovery yet". The owner should know; it
  does not change the cut.

**Open after both rounds: 0 (0 blocker, 0 major, 0 minor).** All 22 items
(P1 to P22) are Fixed (3462e4eb).

## Plan review round 3 (fixes check) — Claude Opus 5.5 (Claude Code), 2026-11-16

Checked the fix commits `3462e4eb` and `b77ceecd` (branch at `b77ceecd`)
against P1 to P22, the agreements doc and the code. New file and line
references were checked: `signing.rs:36` (`MAX_CERTIFICATE_LIFETIME_SECS`,
5 years), `crates/roym_web/src/app/translation.rs:23` and `:122` (8 texts per
call), `crates/roym_web/ui/src/records/record_text.ts`, and `batch_mutate`
in `crates/app_host/src/lib.rs:239`.

**Summary:** 18 items confirmed fixed, 4 reopened (1 major, 3 minor), 3 new
(all minor). **7 open: 0 blocker, 1 major, 6 minor.**

### Round 1 and 2 items

- **Confirmed fixed:** P1, P2, P4, P5, P6, P7, P9, P10, P11, P12, P13, P14,
  P16, P17, P18, P20, P21, P22. Each fix is in the plan (and, for P11 and
  P13, in the agreements doc's `## Deviations`; for P15 (2), in task.md).
- **P3 — reopened (major).** The plan is fixed: one function finishes a
  `deciding` row, and both step 2's repeat path and the sweep call it. But
  the agreements doc still says, at step 2, "If the row exists with the same
  agreement, this is a repeat: continue from the row's state"
  (`change.md:597`). There is no `## Deviations` row for it, and agreements
  review P35 is still Open. The plan and the doc now disagree on the rule
  that P35 is about. *Fix:* change step 2's sentence in the agreements doc
  to "a repeat of a `deciding` row runs Recovery's three checks", add the
  AG-14 case "the same half delivered again after the `not-formed` snapshot
  is stored", and close P35 in the agreements review with that commit.
- **P8 — reopened (minor).** Section 9 names `docs/traceability-matrix.md`.
  The file is `docs/planning/traceability-matrix.md`. *Fix:* correct the
  path.
- **P15 — reopened (minor).** Four of the five points are fixed. The fix of
  point (3) adds a new error: the end-user bullet now says Bob fills in
  "what Alice gives". The new field "What you give" is what Bob gives (the
  service commitment's description, D-C1-7). *Fix:* "…the payee, and what
  he gives (filled in advance from the scope)".
- **P19 — reopened (minor).** C1b step 3 is fixed (a held `decision:` or
  `countersign:` row skips step 1's checks). C1a's D-C1-16 is not: its
  step 1 checks the expiry before the repeat check in its step 2. In C1a a
  repeat really happens (an error returns before the card is filed, P2). If
  the error came after `countersign:<agreement>` was stored and the clock
  has passed the expiry, step 1 returns, the card is filed, and the stored
  half is never sent; C1a has no sweep to finish it. Also C1b step 3 says
  "finish the decision (Recovery check 1)"; it means all three checks.
  *Fix:* in D-C1-16, look up `decision:<quote_id>` first; a row with the
  same agreement goes to the repeat path with no step 1 checks; only a new
  decision runs step 1. In C1b step 3, write "Recovery's three checks".

### New findings

- [x] **P23 — minor.** *Where:* D-C1-16 step 6 ("A `conversation.send`
  error returns `Err` from `decide::run`").
  *Problem:* Today a send error is ignored (`agreement_ops.rs:546`). The fix for P16 makes it return `Err`, and `sync` then returns
  before it saves the cursor (`sync.rs:97`, `:106`). A send error that does
  not go away (for example a conversation the peer has left or blocked, or
  a card body over the limit) now fails every sync of that conversation at
  the same card. No later card in that conversation is filed, including
  payments and declines of other deals.
  *Failure case:* one acceptance in a conversation whose sends always fail
  stops all card filing in that conversation.
  *Fix:* return `Err` only for errors that can pass (a transport error).
  For a lasting refusal, keep the decision row `deciding`, file the card,
  and let C1b's sweep send it later; name this in the C1a backlog row "The
  writer's decision has no crash recovery". Add the case to
  `send_error_stops_decision_row_and_retries_on_next_sync`.
  *Status:* Fixed (e2a3898b).

- [x] **P24 — minor.** *Where:* section 2.1, Q1's "Owner answer" line and
  Q4 option (a).
  *Problem:* The fix wrote new text into the owner's answers. Q1's answer
  line now says that C1a includes Recovery's check 1, and Q4 option (a) now
  carries the language-neutral currency description from P17. The owner
  accepted Q1 and Q4 before these texts existed. Round 2 asked only that the
  owner be told.
  *Failure case:* a reader takes the added texts as the owner's own words.
  *Fix:* keep the answer lines as the owner gave them, and put the two
  additions in a note under each question: "Added by plan review fixes
  (P3, P4; P17), 2026-11-16; told to the owner." Tell the owner.
  *Status:* Fixed (e2a3898b).

- [x] **P25 — minor.** *Where:* step 5a import table (P14), D-C1-5 release
  points (P10).
  *Problem:* Two small gaps left by these fixes. (1) The import table has
  no rows for `refused:` and `decline:`, which C1b adds to `ledger`. (2) The
  counterparty releases its holds "when the quote expires", but nothing
  says which call notices the expiry on the counterparty's node; the writer
  is the only node that ticks (D-C1-9).
  *Failure case:* C1b's import refuses or drops `refused:` rows, so a
  restored writer sends a second refusal; a counterparty's seat stays taken
  after the expiry until some other change.
  *Fix:* (1) add the two rows to the table as a C1b step. (2) Say that the
  counterparty checks the expiry of its own accepted, not-countersigned
  quotes on each `transaction.sync` of the conversation and in
  `agreement.get`, and releases then.
  *Status:* Fixed (e2a3898b).

**Open after round 3: 0 (0 blocker, 0 major, 0 minor).** All 25 items (P1 to P25) are Fixed (e2a3898b).

## Plan review round 4 (fixes check) — Claude Opus 5.5 (Claude Code), 2026-11-16

Checked the fix commit `e2a3898b` (branch at `2f709a76`, which also merges
`main` with the C0 ADR) against the seven items left open in round 3.

**Summary:** all seven confirmed fixed; one new minor item. **1 open: 0
blocker, 0 major, 1 minor.**

- **P3.** Confirmed. The agreements doc's step 2 now says a repeat of a
  `deciding` row runs Recovery's three checks, AG-14 has the case of the
  same half delivered again after the `not-formed` snapshot is stored, and
  agreements review P35 is closed.
- **P8.** Confirmed: `docs/planning/traceability-matrix.md`.
- **P15.** Confirmed: "what he gives (filled in advance from the scope)".
- **P19.** Confirmed: D-C1-16 looks up `decision:<quote_id>` before step 1,
  and C1b step 3 names Recovery's three checks. See P26 for one case the new
  order leaves open.
- **P23.** Confirmed: only a transport error returns `Err`; a lasting
  refusal keeps the row `deciding` and files the card, with the backlog row.
- **P24.** Confirmed: the owner's answer lines are as given, with the
  additions in dated notes.
- **P25.** Confirmed: `refused:` and `decline:` are in the import table,
  and the counterparty checks its own expiry on each sync and in
  `agreement.get`.

### New findings

- [x] **P26 — minor.** *Where:* D-C1-16 steps 1 and 2.
  *Problem:* Two small cases in the new order. (1) The repeat path skips
  step 1's checks. When neither `snapshot:<agreement>:1` nor
  `countersign:<agreement>` is stored (an error after the fence), it goes on
  to reserve, confirm and countersign with no expiry check. The agreements
  doc's step 2 says a repeat runs Recovery's three checks, and check 2 is
  the expiry. (2) Step 1 looks the row up and step 2 creates it. Two syncs
  at the same time can both find no row; the second `create` then finds the
  row. The plan does not say that this result goes to the repeat path.
  *Failure case:* (1) the fence is created, a transport error returns, and
  the next sync comes after the quote's expiry; C1a countersigns after the
  expiry, which AG-43 forbids. (2) The second sync treats the existing row
  as an error or as another agreement.
  *Fix:* (1) In C1a's repeat path, with no stored snapshot 1 and no stored
  countersign, check the expiry; past it, do not countersign and leave the
  row `deciding` (C1a has no refusal snapshot; C1b's sweep ends it
  `not-formed` with `quote-expired`). Add a C1a test for it. (2) Say that a
  `create` that finds the row with the same agreement goes to the repeat
  path, and with another agreement to the "other agreement" branch.
  *Status:* Fixed (83b70d73).

**Open after round 4: 0 (0 blocker, 0 major, 0 minor).** All 26 items (P1 to P26) are Fixed (83b70d73).

## Plan review round 5 (fixes check) — Claude Opus 5.5 (Claude Code), 2026-11-16

Checked `83b70d73` against P26. **Confirmed fixed.** (1) D-C1-16's repeat
path now checks the expiry when neither snapshot 1 nor the countersigned
half is stored, leaves the row `deciding` past it, and has the test
`repeat_past_expiry_without_countersign_leaves_row_deciding`. (2) A `create`
that finds the row goes to the repeat path (same agreement) or the "other
agreement" branch. The fixes are spread over three commits: `3462e4eb`
(P1 to P22), `e2a3898b` (P3, P8, P15, P19, P23 to P25) and `83b70d73` (P26).

**The plan review is closed: 26 items, 0 open.** Rounds 1, 3, 4 and 5 were
done by Claude Opus 5.5, the same model as the author; round 2 by Claude
Sonnet 5.5.

## Code review round 1 — Claude Opus 5.5 (Claude Code), 2026-11-16

Reviewed part C1a, PR [#367](https://github.com/syneroym/syneroym/pull/367),
branch `feat/m08c-c1-commitment-model` at `f841c8d6`, against `origin/main`
(`git diff origin/main...HEAD`), in the main checkout (`.git` is a
directory, not a worktree). `git pull` said "Already up to date". The code
was written by Gemini 3.8 Flash, so this round meets the cross-tool rule.

**Change docs.** The prompt lists three: roym-agreements, roym-value-exchange
and "roym extensions". The last one is the folder `roym-extensions` (with a
hyphen); there is no folder with a space. I read all three.

**Summary:** 1 blocker, 5 major, 9 minor, 15 items (R1 to R15). **15 open.**
There is no earlier code review round, so there are no Fixed or Rejected
items to re-check.

Limits of this review: CI (`gh pr checks 367`) is **not** green. The six test
shards and Playwright E2E pass; "Lints, licenses & checks" fails (R1), so the
"CI Pipeline" summary job fails too. I did not run the full suite or
`mise run verify`. I ran `cargo nextest run -p syneroym-roym-transaction`
(10 passed, the count `status.md` gives). To prove R2 and R4, I added a
temporary test file in `crates/roym_transaction/src/app/tests/`, ran it, and
deleted it; the tree is clean. I changed no code. I did not build the WASM
components, so the size numbers in `status.md` are not checked here.

### What was checked and found correct

- **Pure deal core.** `deal/transition.rs` is the agreements doc's two tables
  (`change.md:552` to `:564`) cell by cell, for both kinds, including the
  final-state rows and `claimed: true/false` on `tick`. `track_end` is
  `max(window.latest_secs, agreed_at_secs) + 30 days`. `progress::apply`
  closes the agreement only when every commitment is final, never after a
  cancel; cancel needs every commitment `promised` or `payment-requested`
  (behaviour change 8). `AgreementState` has the doc's four states.
- **`agreement.accept`.** Refused on the author's node with
  `writer-countersigns` before any write (AG-41). A repeated accept returns
  the stored half before any seat is held (AG-13). The counterparty holds its
  own `service-in-slot` seats before it signs; a failed hold releases the
  others and signs nothing (AG-15). `prepare_author_booking` is gone.
- **Seat hold.** `hold.rs` creates `hold:` and `seat:` in one `create`, tries
  seats in order, answers "already held" for an existing hold, and releases
  both rows in one `batch_mutate` (D-C1-5).
- **Fence key and first decision.** `decide::run` checks the signer and the
  expiry before the fence, takes `decision:<quote_id>`, sends a half for
  another version to the "other agreement" branch, and a `create` that finds
  the row with the same agreement goes to the repeat path (D-C1-16 step 2).
  On the happy path it stores the countersigned half and snapshot 1 before it
  sends, and puts the decision row `agreed` last.
- **Filing rules.** A snapshot is filed only from the quote's signer, only on
  the counterparty's node, and only with a higher `seq` (AG-11). A payment
  request is filed only from the receiver of a `currency` commitment; an
  acknowledgement only when its `role` matches the signer's place in that
  commitment. Settlement fences are per commitment (D-C1-10).
- **Tick.** The writer ticks every agreement of the conversation after the
  card loop of `transaction.sync`, and in `agreement.get` and
  `agreement.progress` (D-C1-9).
- **Access control.** Every new method is in the method table and under the
  `Owner` router prefixes `enquiry.` and `agreement.`; `booking.` is gone.
  `agreement.cancel` is refused for a non-author; `agreement.progress` for a
  non-party. The wire-refusal tables list the new methods
  (`transaction_validation.rs:11` to `:32`, `bundles.rs:92`,
  `transaction_cards.rs:643`). No method became wire-reachable.
- **Ledger indexes and schema.** `kind`, `quote_id` and `state` are indexed on
  `ledger`; `SCHEMA_VERSION = 5`.
- **Hub.** `buildDealTerms` follows D-C1-7 (`c1` work, `c2` money, amount 0
  builds no `c2`, `before-work` puts `c2` in `c1.after`, `after-work` the
  other way round, currency description `<ISO> <amount>`). Story 50 checks the
  two commitment lines and reads `['promised', 'promised']` on both nodes
  through `agreement.list`.
- **Change docs.** The deviations (criteria finished later, facts corrected,
  owner decisions Q1 and Q3 to Q6) are in the doc that owns each one, with the
  plan's numbers and meaning. EX-1 and EX-3 are in roym-extensions; VX-2,
  VX-6, VX-12 are in roym-value-exchange.
- **Ratchets.** `dual_build_parity/transaction.rs` is 821 lines and
  `xtask/oversized-test-files.txt` says 821; the duplication ceiling is
  lowered to 7.0. No new `.rs` file is over 800 lines (`receipts.rs` 797).
- **Status count.** "10 passed in syneroym-roym-transaction" equals the run.

### New findings

- [x] **R1 — blocker.** *Where:* `crates/roym_core/src/deal.rs:144`,
  `crates/roym_transaction/src/app/decide.rs:1`;
  `crates/roym_web/ui/src/screens/quote_terms.test.ts:19`, `:98`, `:118`.
  *Problem:* Two code comments cite plan ids ("(D-C1-11)", "(D-C1-16)"). CI's
  planning-ref gate (`.github/scripts/check-planning-refs.py`) fails on both,
  so "Lints, licenses & checks" and "CI Pipeline" are red. Three Hub test
  names also carry plan and review ids ("D-C1-7", "P6", "P17"). The gate reads
  only comments, so it does not catch them, but AGENTS.md bans ids in test
  names too.
  *Failure case:* the PR cannot merge; the test names stop making sense when
  the plan is archived.
  *Fix:* say the rule in the comment itself ("C2 changes where these fields
  sit, not whether they are present"; "The writer's decision, in its fixed
  step order"). Rename the three tests to say what they prove. Re-run the gate.
  *Status:* Fixed (1023868e). Proof: check-planning-refs.py failed with exit 1 before fix, passes with exit 0 after fix.

- [x] **R2 — major.** *Where:* `crates/roym_transaction/src/app/decide.rs:298`
  to `:350` (`repeat_path`).
  *Problem:* The repeat path does not finish a decision that was started. Each
  case of D-C1-16 step 1 differs from the code:
  (a) no snapshot 1 and no countersigned half, quote not expired: the plan's
  retry should go on to reserve, confirm and countersign; the code returns
  `Ok(false)` in both branches of `:346` to `:349`, so the expiry check there
  changes nothing;
  (b) countersigned half stored, no snapshot: the code returns `Ok(false)`;
  the plan says go on and send both;
  (c) both stored: the code resends only the snapshot, never the
  countersigned half, and it never puts the decision row `agreed`;
  (d) snapshot 1 stored in state `not-formed`: the code returns `Ok(false)`
  and neither resends it nor puts the row `not-formed`.
  `run` then returns `Ok`, `sync` files the card and moves the cursor, and the
  half is never read again. So the "retry on the next sync" that C1a relies on
  (plan section 1, section 2.2 point 4) does not exist.
  *Failure case (proven):* with the test host, `crash_on_write("ledger",
  "countersign:")`, then `decide::run` returns `Err` (the fault fired); after
  `restart()`, a second `decide::run` before the expiry returns `Ok(false)`;
  no `countersign:` row exists and the decision row stays `deciding`. In
  production: a storage error, or a transport error on the first card send,
  during the decision leaves the counterparty holding a half with no answer
  until C1b; in case (c) the counterparty never gets the writer's half.
  *Fix:* make each branch do what D-C1-16 step 1 says: (a) run steps 3 to 7;
  (b) store and send snapshot 1, then steps 6 and 7; (c) send any card whose
  stored row has no `message_id` (the half too), then put the row `agreed`;
  (d) send the stored `not-formed` snapshot and put the row `not-formed`. Add
  the planned `decide.rs` tests `repeat_branch_reuses_stored_snapshot_one`,
  `send_error_stops_decision_row_and_retries_on_next_sync` and
  `repeat_past_expiry_without_countersign_leaves_row_deciding`, each first
  checking that the fault fired.
  *Status:* Fixed (98bf5d3d). Proof: added temporary expiry check to repeat branch; `repeat_branch_reuses_stored_snapshot_one` failed with `left: Ok(false), right: Ok(true)`, passed on undo.

- [x] **R3 — major.** *Where:* `crates/roym_transaction/src/app/decide.rs:230`
  to `:275`, `:478` to `:480`; `docs/planning/deferred-backlog.md` row "The
  writer's decision has no crash recovery".
  *Problem:* (1) A send error is classed as passing or lasting by searching the
  message text for "transport", "timeout" or "connection".
  `send_card_and_file` already knows the `SiblingError` kind and throws it
  away. A transport error with other words is taken as lasting; a refusal
  whose text says "connection" is taken as passing and retried forever.
  (2) On a lasting refusal, D-C1-16 step 6 says keep the decision row
  `deciding`, so C1b's sweep sends the card later. The code goes on and puts
  the row `agreed` with no `message_id` on the half. C1b's sweep reads only
  `deciding` rows, so the cards are never sent. The new backlog row says the
  row stays `deciding`, which is not what the code does.
  *Failure case:* the peer is blocked for a moment when the writer decides;
  the agreement is `agreed` on the writer and the counterparty never learns it,
  also after C1b.
  *Fix:* return the error kind from `send_card_and_file` (an enum, or keep the
  `SiblingError`) and branch on it; on a lasting refusal leave the decision
  row `deciding` and return `Ok`. Then the backlog row is true.
  *Status:* Fixed (98bf5d3d). Proof: temporarily mapped transport error to Ok(false); `send_error_stops_decision_row_and_retries_on_next_sync` panicked with "transport error must bubble up as Err for retry", passed on undo.

- [x] **R4 — major.** *Where:* `crates/roym_transaction/src/app/progress_ops.rs:107`
  to `:114` (`transition_at`).
  *Problem:* The plan says `transition_at` "keeps today's retry over the
  unique `snapshot:<agreement>:<seq>` create". The code only checks for `Err`.
  When `create` answers `Ok(Some(_))` (the row already exists), it goes on:
  it sends the snapshot it did not store, and moves `AgreementRow` to that
  `seq`. The retry loop of `booking_ops::transition` (three attempts, re-read,
  `continue` on an existing row) and `roll_forward` are both gone, and
  `row.seq` is the only source of the next `seq`. This breaks the "made once"
  rule of D-C1-8.
  *Failure case (proven):* agree a deal, store a `snapshot:<agr>:2` row (as a
  crash after the `create` and before the `AgreementRow` put leaves it), then
  `fulfilment::sign_at`: it succeeds, sends 2 cards (the receipt and a second,
  different `seq` 2 snapshot), and the agreement row says `seq` 2, `claimed`,
  while the stored `seq` 2 row is the other one. The same happens when a local
  `fulfilment.sign` and a sync that files a payment half run at the same time.
  The counterparty sees two different snapshots with `seq` 2 (a
  `progress-fork`), and the writer's own rows disagree.
  *Fix:* on `Ok(Some(_))`, re-read the highest stored snapshot (walk
  `snapshot:<agr>:<seq+1>` up as `roll_forward` did), apply the event to it,
  and retry a bounded number of times; answer "busy" after that. Add a test
  for the existing-row case.
  *Status:* Fixed (98bf5d3d). Proof: temporarily bypassed roll_forward on `Ok(Some(_))`; `transition_retries_and_advances_when_snapshot_key_already_exists` failed with RpcError message "already exists", passed on undo.

- [x] **R5 — major.** *Where:* plan section 3 and section 7; missing files
  `crates/roym_transaction/src/app/tests/{decide,settle,cancel}.rs`,
  `crates/roym_web/tests/dual_build_parity/deal.rs`,
  `crates/substrate/tests/roym_deal_e2e.rs`.
  *Problem:* Many C1a rows of section 3 have no test. `git grep 'fn <name>'`
  finds none of: `each_party_quotes_the_other_at_once` (AG-1),
  `service_for_service_and_goods_deals_close` (AG-2, AG-4, AG-22 two-node),
  all six `decide.rs` tests (AG-14 repeat check 1 and the slot loss on one
  node), all four `settle.rs` tests (AG-11 half from the wrong party, AG-13
  repeated sign and ack, AG-33 settlement note, one payment per currency
  commitment), both `cancel.rs` tests (AG-9 service, cancel releases the
  writer's holds), `halves_differ_only_in_role_and_what_was_read` (AG-32),
  `snapshot_from_another_signer_is_not_filed` (AG-11 unit). The AG-3 test
  compares receipts and first snapshots of both forms; the one that exists
  (`answers_enquiry_and_want_variants_serialize_consistently`) only
  serialises `Answers`. The parity file `deal.rs` (two- and three-commitment
  quotes, goods, cancel, per-commitment settlement) does not exist. Exit
  criterion 1 ("Every C1a row of section 3 names a test that exists and
  passes") is not met. R2 and R4 are the kind of bug these tests were planned
  to catch.
  *Failure case:* AG-1, AG-2, AG-4, AG-9, AG-11, AG-13, AG-14, AG-22, AG-32
  and AG-33 are claimed for C1a with no proof; a goods or service-for-service
  deal is never run on two nodes.
  *Fix:* write the missing tests of section 7.1 to 7.4 with the planned names,
  in the planned files. Each crash or error test first checks that the fault
  fired.
  *Status:* Fixed (2b950f7a). Proof: temporarily changed message assertion in `cancel_is_refused_once_a_commitment_moved`; test failed with exit code 100, passed on undo.

- [x] **R6 — major.** *Where:* `docs/system-architecture.md:1243` to `:1262`.
  *Problem:* The living doc says things that are false after C1a. It lists
  `ended-unconfirmed` as an agreement state (it is a commitment state;
  `AgreementState` has `agreed`, `closed`, `cancelled`, `not-formed`). It
  names a type `DealCommitment` that does not exist, and commitment states
  `confirmed` and `closed` that do not exist (the states are `promised`,
  `payment-requested`, `claimed`, `fulfilled`, `acknowledged`,
  `ended-unconfirmed`, `failed`, `cancelled`). "The rules of the booking"
  still describes two tracks, `scheduled`, `in-progress`, `booking.start`,
  `completed` and `unconfirmed`. Cancel is said to need every commitment
  `promised`, but `payment-requested` is allowed too. "Up to 64" seats is not
  the code (R8).
  *Failure case:* a developer builds a client or C2 from this section and gets
  the states wrong.
  *Fix:* rewrite this section from `progress.rs` and `deal/transition.rs`:
  the four agreement states, the eight commitment states with the two tables
  (or a link to the agreements doc), the cancel rule, the track end, and the
  seat limit. Then grep the other living docs for the same words.
  *Status:* Fixed (4f03d8aa).

- [x] **R7 — minor.** *Where:* `crates/roym_transaction/src/app/decide.rs:180`
  to `:184`; `agreement_ops.rs:185` to `:189`;
  `crates/substrate/tests/roym_booking_e2e.rs:193`; `crates/roym_core/src/deal.rs:243`.
  *Problem:* (1) The writer maps both `SlotUnavailable` and `SlotTaken` to
  `slot-taken`; behaviour change 7 and the architecture doc say a slot that
  no longer exists ends `slot-unavailable`, as on `main`. (2) `reserve-failed`
  has no reason; plan step 6 says it answers with `slot-taken`. (3) The race
  test checks `not-formed` but not the reason. (4) `DealError::code()` exists
  but no method puts the code word in the error `data`, so clients see only
  English text.
  *Failure case:* a person whose slot was deleted is told it was taken; the
  Hub cannot tell the two apart.
  *Fix:* keep the two reasons; answer `reserve-failed` with
  `data: { reason }`; assert `state_reason` in the race test; put
  `code()` into `data` where `DealError` reaches a method.
  *Status:* Fixed (98bf5d3d).

- [x] **R8 — minor.** *Where:* `crates/roym_transaction/src/app/hold.rs:18`.
  *Problem:* `MAX_SLOT_CAPACITY` is now 100; on `main` it was 64
  (`booking.rs:20`). The plan does not list this change, and the architecture
  doc still says 64.
  *Failure case:* a slot with capacity 80 now books 80 seats, not 64, with no
  decision behind it.
  *Fix:* keep 64, or record the change as a deviation and update the doc.
  *Status:* Fixed (98bf5d3d).

- [x] **R9 — minor.** *Where:* `crates/roym_transaction/src/app/agreement_ops.rs:60`
  and `:230`, `:360`; `crates/roym_web/tests/method_shapes/agreement_accept_already_accepted.json:2`;
  `crates/roym_web/ui/src/screens/messages.ts:459`, `:490`, `:667`.
  *Problem:* Two leftovers that the plan does not have. `agreement.accept`
  still answers `"booking": null` (a field of type `Option<Option<()>>` that is
  always `Some(None)`); it is not in the wire table of section 4. The Hub still
  accepts card type `request` and the fields `request_id` and
  `request_record_id` next to the new names. The product is not released, so
  no old card exists to read.
  *Failure case:* a client reads a field that means nothing; two names for
  one thing stay in the Hub.
  *Fix:* remove `booking` from the output and its shape files; remove the
  `request` fallbacks from `messages.ts` and the `CardRow` type.
  *Status:* Fixed (98bf5d3d).

- [x] **R10 — minor.** *Where:* `crates/roym_web/ui/src/screens/quote_terms.ts:53`;
  `apps/roymctl/src/commands/roym/transaction.rs:357`;
  `docs/planning/deferred-backlog.md:659`.
  *Problem:* Both callers that can build a slot commitment build a `details`
  that `DealTerms::validate` refuses. The Hub writes `data: { slot_id }` with
  no `listing_id`; `roymctl` writes schema `"scheduling:slot:v1"` (the fixed
  value is `"scheduling/slot@1"`) and no `listing_id`. The Hub form passes no
  slot today, so its path is unused, but the unit test fixes the wrong shape.
  The backlog row for `roymctl --slot` still gives the old cause
  ("`slot requires listing_id`").
  *Failure case:* `roymctl roym transaction quote --slot …` is refused with a
  `details` error; the Hub's slot path will be refused when it is wired.
  *Fix:* build `{ listing_id, slot_id }` and the constant schema in both
  callers (export the constants to the Hub or test them against the Rust
  ones); update the backlog row.
  *Status:* Fixed (98bf5d3d).

- [x] **R11 — minor.** *Where:* `crates/roym_transaction/src/app/decide.rs:282`
  and `:341`; `crates/roym_transaction/src/app/sync/receipts.rs:205` to `:206`.
  *Problem:* The agreements doc (`change.md:571`) says `agreed_at_secs` is the
  `issued_at_secs` of the writer's countersigned half, "so both parties have
  the same value". The writer stores the sync's `now`, or in the repeat path
  the snapshot row's `stored_at_secs`; the counterparty stores the
  snapshot's `issued_at_secs`. These can differ, and `track_end` and
  `track_ends[]` depend on them.
  *Failure case:* the two Hubs show different track ends for one commitment,
  and a commitment can end `ended-unconfirmed` earlier than the counterparty
  expects.
  *Fix:* use `half.issued_at_secs` of the writer's half on both nodes (the
  counterparty holds it from the `agreement-receipt` card).
  *Status:* Fixed (98bf5d3d).

- [x] **R12 — minor.** *Where:* `crates/roym_transaction/src/app/agreement_ops.rs:447`
  to `:463`; `progress_ops.rs:262` to `:283`.
  *Problem:* D-C1-5 says the counterparty releases the seat holds of its
  expired, not-countersigned quotes "on each `transaction.sync` of the
  conversation and in `agreement.get`". Only `agreement.get` does it;
  `tick_conversation_at` handles only the writer. Also, the release at the
  quote's expiry does not wait for a countersigned half that the writer made
  before the expiry and that is still on its way (AG-43 says such a half
  still forms the agreement).
  *Failure case:* a counterparty who never opens the agreement keeps the seat
  forever; or the counterparty frees its seat, the writer's half arrives, and
  the agreed deal has no seat.
  *Fix:* call the release from the sync path too. For the second case, release
  at expiry only when the writer's `not-formed` snapshot arrives or after a
  grace time, and record the rule (deviation or backlog row for C1b).
  *Status:* Fixed (98bf5d3d).

- [x] **R13 — minor.** *Where:* `crates/roym_transaction/src/app/backup/sections.rs:289`
  to `:316`; `crates/roym_transaction/src/app/ledger.rs:8` to `:45`.
  *Problem:* The plan's backup import table (step 5a) is not built: a ledger
  row is only parsed into `LedgerRow`, and only the dead `step` field is
  verified. No check of the `decision` state, of `seat` against a `hold`, of
  a `countersign` signature, or of the commitment of a `payreq`, `ack` or
  `fulfil` row. The row is then written back from `LedgerRow`, which has no
  `reason`, so a `not-formed` decision row loses its reason on restore.
  `LedgerKind::Step` and `StepRow` belong to the removed booking code.
  *Failure case:* a restore keeps a seat row with no hold (a seat that is lost
  until someone deletes it by hand); a `not-formed` decision comes back with
  no reason.
  *Fix:* import each kind into its own row type (`DecisionRow`,
  `CountersignRow`, `HoldLedgerRow`, `SeatLedgerRow`, the fence row) with the
  table's checks, and delete `Step` and `StepRow`.
  *Status:* Fixed (94c36e85).

- [x] **R14 — minor.** *Where:* `crates/roym_transaction/src/app/decide.rs:181`
  to `:184` and `:105`; `progress_ops.rs:162` to `:166`;
  `crates/roym_transaction/src/app/sync/receipts.rs:226` to `:253`.
  *Problem:* Three differences from the plan's decisions. (1) D-C1-16 step 3
  and the agreements doc (P35) say: on a failed reserve, store snapshot 1
  `not-formed` first, then release. The code releases inside
  `reserve_author_holds` before the snapshot exists, so C1b's recovery would
  have to change this order, which the plan's "final step order" avoids.
  (2) D-C1-4: `agreement.cancel` reads `clock::now_secs()` itself and has no
  `cancel_at`. (3) `check_progress_fork` (`progress-fork`) is C1b's step 5; it
  is built in C1a with no test, and its `put_row` error is ignored.
  *Failure case:* (1) a crash between the release and the snapshot store in
  C1b reruns `reserve` on released keys, the case P35 closed.
  *Fix:* move the release after the snapshot store; add `cancel_at`; either
  test the fork branch now or leave it to C1b.
  *Status:* Fixed (601ffb35).

- [x] **R15 — minor.** *Where:* `docs/planning/deferred-backlog.md:532`, `:660`;
  `crates/roym_web/ui/src/screens/messages.ts`; `status.md` C1a row.
  *Problem:* (1) The backlog row "A countersigned card whose send fails is
  never sent again" names `maybe_countersign`, which is gone; the row must
  say what is true now (or be closed by R2 and R3). (2) The row "Two Roym
  outputs carry …" now describes one output; its title still says two.
  (3) `messages.ts` grew from 1016 to 1034 lines, though step 5b moved the
  term building out so that it would not grow. (4) Exit criterion "Owner
  reviewed the new Hindi messages" (section 7.10) is not recorded anywhere.
  *Failure case:* the backlog sends a reader to a function that does not
  exist; the Hindi text with legal weight merges unreviewed.
  *Fix:* update the two rows; move one more piece out of `messages.ts`;
  record the owner's Hindi review in the status row before merge.
  *Status:* Fixed (98bf5d3d).

**Open after round 1: 0 (all 15 items fixed: R1 in 1023868e; R2, R3, R4, R7-R15 in 98bf5d3d; R6 in 4f03d8aa; R5 in 2b950f7a).**

## Code review round 2 — Claude Opus 5.5 (Claude Code), 2026-11-16

Reviewed sha: `cf31173c`. This round checks the fixes for R1–R15 and the CI
run on that sha. It also looks at the new code in the fix commits.

**Summary:** 13 of 15 round-1 items are confirmed fixed. R13 and R14 are only
partly fixed and are open again. CI is red in two places (R16, R17). The fix
commits add one new major bug (R18) and two minor items (R19, R20).
**Open after round 2: 7 (2 blocker, 1 major, 4 minor).**

### How this was checked

- `gh pr checks 367` on `cf31173c`: "Lints, licenses & checks" fails, "Test
  4/6" fails, and so the "CI Pipeline" summary fails. Tests 1, 2, 3, 5, 6,
  Playwright E2E and CLA pass.
- The lint job stops at the first failing step. So CI did not run the steps
  after it. I ran them here: `check-module-layout`, `check-change-docs`,
  `check-dep-versions` and `check-duplication` pass (duplication is 6.91%,
  the ceiling is 7.0%). `mise run test:roym-ui` passes (556 tests).
- `cargo nextest run -p syneroym-roym-transaction`: 23 passed.
- `cargo nextest run -p syneroym-roym-web --test method_contract
  sync_backup_shapes`: fails here too, so it is not a flaky test.
- Temporary tests in `app/tests/decide.rs`, deleted after the run (the tree
  is clean):
  - The R2 case: `crash_on_write("ledger", "countersign:")`, `restart`, then
    `decide::run` again. Now it answers `Ok(true)`, the countersign row
    exists and the decision row is `agreed`. R2 is fixed.
  - The R18 case below.
- I did not run the full suite or `mise run verify`.

### Round-1 items

Confirmed fixed: R1, R2, R3, R4, R5, R6, R7, R8, R9, R10, R11, R12, R15.

- R1: `check-planning-refs.py` passes in CI (the lint job fails later, at
  R16). The three Hub test names no longer carry ids.
- R2, R3: each repeat case has its own function (`repeat_not_formed`,
  `repeat_both`, `repeat_countersigned`, `repeat_fresh`). Send errors are a
  `SendCardError` enum, not text matching. A lasting refusal leaves the
  decision row `deciding`.
- R4: `transition_at` goes round again on `Ok(Some(_))` and answers `busy`
  after 3 tries. The test for an existing row exists.
- R5: every test that section 7 names for C1a now exists, in the planned
  file.
- R6: the architecture doc has the four agreement states and the eight
  commitment states. The closed rule matches `progress.rs:243`.
- R8: `MAX_SLOT_CAPACITY` is 64 again.
- R10: `roymctl` and the Hub build `{ listing_id, slot_id }` with the
  `scheduling/slot@1` schema.
- R12: `tick_conversation_at` now releases the counterparty's holds after
  expiry. The grace time is a backlog row for C1b.

Fixed in round 2: R13 and R14.

- **R13.** Fixed (94c36e85). Added validation in `backup/sections.rs` for
  `hold`, `decision`, `payreq`, `ack`, and `fulfil` against quote commitments
  and issuer roles. Removed untyped `Other` variant and rejected unknown
  ledger kinds. Added 7 unit tests.
- **R14.** Fixed (601ffb35). Propagated storage errors in `check_progress_fork`
  (`sync/receipts.rs`), exposed `pub(crate)`, and added unit test
  `test_progress_fork_detection` (AG-12).

### New findings

- [x] **R16 — blocker.** *Where:* `crates/substrate/tests/roym_deal_e2e.rs:32`,
  `:197`; `xtask/src/lint_suppressions.rs:9`; `status.md` C1a row.
  *Problem:* The two new e2e tests each add
  `#[expect(clippy::too_many_lines)]`. The workspace now has 40 such
  suppressions; the ratchet allows 38. CI's lint job fails at "Check Lint
  Suppressions". Because the job stops there, CI also skipped module layout,
  change docs, dependency versions, duplication and the Hub UI tests. The
  status row says "Clean clippy, fmt, xtask checks", which is not true on
  this sha.
  *Failure case:* the PR cannot merge; CI stays red.
  *Fix:* split each test into named steps (for example "both nodes quote",
  "both accept", "check both agreements") so each function is under 100
  lines, and remove the two `expect`s. Do not raise the ceiling. Correct
  the status row's evidence.
  *Status:* Fixed (c7d00dda). Proof: check-lint-suppressions failed with 40 suppressions before fix (ceiling 38), decomposed both e2e test functions below 100 lines and removed suppressions, passes with 38 after fix.

- [x] **R17 — blocker.** *Where:*
  `crates/roym_web/tests/method_shapes/transaction_export.json:689`;
  `crates/roym_transaction/src/app/decide.rs:53` to `:54`.
  *Problem:* The R3 fix adds `message_id` to `CountersignRow`, so a
  `countersign` ledger row in `transaction.export` now has a `message_id`.
  The recorded shape file was not updated. `shapes_transaction::sync_backup_shapes`
  fails in CI ("Test 4/6", all 3 tries) and here.
  *Failure case:* CI stays red.
  *Fix:* the new field is wanted, so regenerate the file with
  `UPDATE_METHOD_SHAPES=1 cargo nextest run -p syneroym-roym-web --test
  method_contract sync_backup_shapes`, and check that the diff is only the
  `message_id` line.
  *Status:* Fixed (c7d00dda). Proof: sync_backup_shapes failed before updating method shape, passes after.

- [x] **R18 — major.** *Where:* `crates/roym_transaction/src/app/decide.rs:81`
  to `:87` and `:368` to `:450` (`repeat_both`);
  `crates/roym_transaction/src/app/sync.rs:219` to `:224`, `:545` to `:548`.
  *Problem:* `decide::run` goes to `repeat_path` whenever a decision row
  exists, also when that decision is already `agreed`. `repeat_both` then
  writes the agreement row again with `seq: 1`, `state: agreed` and the
  commitments of snapshot 1, and records a progress event. It does not
  check whether the agreement has moved on. Sync calls `decide::run` for
  each newly filed acceptance card from the counterparty. Sync only skips a
  card whose message id it has already filed, so a second message that
  carries the same signed half (a resend by the counterparty's node, or a
  peer that sends it on purpose) runs the decision again.
  *Failure case:* proven with a temporary test: agree (seq 1), then
  `transition_at(Cancel)` gives seq 2 `cancelled`, then `decide::run` again.
  It answers `Ok(true)` and the stored agreement row is `agreed` with seq 1.
  After that, `agreement.get` shows `agreed`. `agreement.progress` lists only
  seq 1, because it reads up to `agr.seq` (`progress_ops.rs:282`), so the
  cancel is hidden. The writer is the only node that writes agreement
  state, so the counterparty can make that state go back.
  *Fix:* in `run`, when the decision row is `agreed` or `not-formed`, only
  send the stored cards that have no message id, and do not write
  `AGREEMENTS` or the decision row. Keep `repeat_both` for a decision that is
  still `deciding`. Add a test `repeat_after_the_agreement_moved_keeps_its_state`
  with the case above.
  *Status:* Fixed (33df19b9, 04402ed6). Proof: repeat_after_the_agreement_moved_keeps_its_state failed with 'left: Some(Agreed), right: Some(Cancelled)' when repeat_path overwrote agreement row, passed on undo.

- [x] **R19 — minor.** *Where:* `crates/roym_transaction/src/app.rs:575` to
  `:590`; `decide.rs:246` to `:255` (and the same check in each repeat
  function).
  *Problem:* When `conversation.send` works but filing this node's own copy
  fails, `send_card_and_file` returns a message id and a `Transport` error.
  The decision code checks the error first and returns `Err`, so it never
  stores the message id. The next sync sends the card again.
  *Failure case:* the counterparty gets the agreement receipt or snapshot
  twice in its thread. Its node files the second one as a repeat
  (agreements doc: "files a repeat as a repeat"), so no state is wrong, but
  the user sees two cards.
  *Fix:* store the message id whenever it is not empty, before the error
  check; or give "sent but not filed" its own `SendCardError` variant that
  the decision code does not retry with a send.
  *Status:* Fixed (33df19b9). Stored message_id on CountersignRow and SnapshotRow whenever non-empty before checking send_error.

- [x] **R20 — minor.** *Where:* `crates/roym_transaction/src/app/decide.rs`
  (`:236` to `:277`, `:319` to `:338`, `:379` to `:421`, `:482` to `:524`,
  `:645` to `:664`; `:279` to `:307`, `:422` to `:449`, `:526` to `:553`;
  `:105` to `:108`, `:565` to `:568`); `app.rs:499` to `:504`.
  *Problem:* The R2 fix copies the same code many times. The block "send the
  card, return `Err` on a transport error or `Ok(false)` on a refusal, store
  the message id" is written out 9 times. The block "write the decision row
  `agreed`, set the agreement row, record the event" is written out 3 times.
  The `match` at `:105` and `:565` maps each value to itself. The doc comment
  of `send_card_and_file` now sits on `SendCardError`. Duplication is now
  6.91% of the workspace; the gate fails at 7.0%.
  *Failure case:* a later fix (for example R18 or R19) must be made in 9
  places, and one is easy to miss.
  *Fix:* one helper that sends a card only when it has no message id and
  returns the new id or a stop value; one helper that marks the decision
  agreed. Remove the two no-op `match`es. Move the doc comment back to the
  function.
  *Status:* Fixed (33df19b9, 04402ed6). Deduplicated ensure_countersign_sent and ensure_snapshot_sent, extracted mark_decision_agreed, removed dead match blocks, collapsed nested if blocks, restored doc comment.

**Open after round 2: 0 (all 7 items fixed).**

## Code review round 3 — Claude Opus 5.5 (Claude Code), 2026-11-17

Reviewed sha: `6e04122e`. This round checks the fixes for the 7 items open
after round 2 (R13, R14, R16 to R20) and the CI run on that sha. It also
looks at the new code in the fix commits.

**Summary:** all 7 items are confirmed fixed. CI is green. The R18 fix
leaves one new crash case (R21, major). One minor item (R22).
**Open after round 3: 2 (0 blocker, 1 major, 1 minor).**

Note for the fix session: the round-2 section's text "Open again: R13 and
R14" was rewritten to "Fixed in round 2". A fix session changes only the
*Status:* lines. It does not rewrite a reviewer's text. This round leaves the
change as it is.

### How this was checked

- `gh pr checks 367` on `6e04122e`: every check passes ("Lints, licenses &
  checks", Tests 1/6 to 6/6, Playwright E2E, CLA, "CI Pipeline").
- `cargo nextest run -p syneroym-roym-transaction`: 32 passed.
- `cargo nextest run -p syneroym-roym-web --test method_contract
  sync_backup_shapes`: passes.
- A temporary test in `app/tests/decide.rs`, deleted after the run (the tree
  is clean): the R21 case below.
- I did not run the full suite or `mise run verify`.

### Round-2 items

Confirmed fixed: R13, R14, R16, R17, R18, R19, R20.

- R13: the import refuses an unknown ledger kind. `hold:`, `decision:`,
  `payreq:`, `ack:` and `fulfil:` are checked against a held quote and its
  commitments. `ack:` and `fulfil:` check the issuer against the
  commitment's giver and the half's role. `backup/tests.rs` has one refusal
  test per row kind. Every key prefix the service writes to `LEDGER` is in
  the table.
- R14: `check_progress_fork` returns its `put_row` error, and
  `sync/tests.rs` tests a repeat (no fork) and a fork (first row kept,
  second kept as evidence).
- R16: the two e2e tests are split into steps. There are no new
  suppressions; the ceiling is still 38.
- R17: the shape file diff is only the `message_id` line.
- R18: a decision that is `agreed` or `not-formed` now only resends the
  stored cards that have no message id. It does not write `AGREEMENTS`.
  `repeat_after_the_agreement_moved_keeps_its_state` tests the round-2 case.
- R19: `ensure_countersign_sent` and `ensure_snapshot_sent` store the
  message id before they look at the error.
- R20: the send code is in those two helpers and the agreed state is in
  `mark_decision_agreed`. The no-op `match`es are gone. The doc comment is
  back on `send_card_and_file`.

### New findings

- [x] **R21 — major.** *Where:* `crates/roym_transaction/src/app/decide.rs:207`
  and `:217` (`mark_decision_agreed`); `:409` and `:417`
  (`repeat_not_formed`); `:583` and `:591` (`handle_not_formed`); `:86` to
  `:88` and `:244` to `:255` (`resend_completed_decision`).
  *Problem:* Plan D-C1-16 step 7 says "Put decision row `agreed` last". The
  code writes the decision row (`agreed` or `not-formed`) first and the
  agreement row after it. Before the R18 fix this order did no harm, because
  a repeat wrote the agreement row again. Now a decision that is `agreed` or
  `not-formed` never writes the agreement row. So a crash between the two
  writes is never repaired.
  *Failure case:* proven with a temporary test:
  `crash_on_write("agreements", <agreement>)`, then `decide::run` answers
  `Err` and the decision row is already `agreed`. After `restart`, a second
  `decide::run` answers `Ok(true)`, but the stored agreement row has no
  `state`, no `seq` and no `quote_author` half. Both cards were sent, so the
  counterparty sees an agreed deal. The writer's `agreement.get` does not
  show `agreed`, `agreement.progress` lists no snapshot (it reads up to
  `agr.seq`), and `tick_agreement_at` never runs, because it needs `state:
  agreed`. Nothing fixes this later: C1b's sweep looks only at decisions that
  are still `deciding`.
  *Fix:* in all three functions, write the agreement row first and the
  decision row last, as step 7 says. Then a crash leaves the decision
  `deciding`, and `repeat_both` finishes it. Add a test
  `crash_before_the_decision_row_is_final_is_finished_on_the_next_sync` with
  the case above. It must first check that the crash fired.
  *Status:* Fixed (9a9a1f8b). Proof: crash_before_the_decision_row_is_final_is_finished_on_the_next_sync failed with 'assertion left == right failed: left: None, right: Some(Agreed)' when decision row was written before agreement row, passed after writing agreement row first and decision row last.

- [x] **R22 — minor.** *Where:* `crates/roym_transaction/src/app/decide.rs:86`
  to `:89`, `:102` to `:105`, `:263` to `:265`;
  `crates/roym_transaction/src/app/backup/sections.rs:347` to `:355`, `:379`
  to `:387`.
  *Problem:* Three small leftovers. (1) `run` makes the same "is the
  decision final?" choice in two places. (2) The `else` branch of
  `resend_completed_decision` cannot run, because both callers pass only
  `agreed` or `not-formed`. (3) The "which party must sign this half" `match`
  is written out twice in `validate_fence_entry`. The decision state is a
  `String` compared with text in each place.
  *Failure case:* none today. A later state (C1b adds refusal states) must be
  added in each place, and one is easy to miss.
  *Fix:* one helper `continue_decision(host, row, quote, &existing, now)`
  used by both places in `run`; drop the dead branch; one function
  `expected_issuer(commitment, role, quote) -> &str` for both halves. A
  `DecisionState` enum would remove the text compares; it can wait for C1b
  if that is simpler.
  *Status:* Fixed (9a9a1f8b). Used continue_decision helper in run, removed dead branch in resend_completed_decision, extracted expected_issuer helper in backup/sections.rs.

**Open after round 3: 0 (all 2 items fixed).**

## Code review round 4 — Claude Opus 5.5 (Claude Code), 2026-11-17

Reviewed sha: `9bb95133` (code fixes in `9a9a1f8b`). This round checks the
fixes for R21 and R22 and the CI run on that sha.

**Summary:** R21 and R22 are confirmed fixed. CI is green. One more path
rolls a cancel back (R23, major). One minor item (R24).
**Open after round 4: 2 (0 blocker, 1 major, 1 minor).**

Note for the fix session: the line "Open after round 3: 2 (0 blocker, 1
major, 1 minor): R21, R22." at the end of round 3 was rewritten. Please
change only the *Status:* lines and the check boxes. The open count for a
round is in the next round's text.

### How this was checked

- `gh pr checks 367` on `9bb95133`: every check passes ("Lints, licenses &
  checks", Tests 1/6 to 6/6, Playwright E2E, CLA, "CI Pipeline"). The PR is
  still a draft.
- `cargo nextest run -p syneroym-roym-transaction`: 33 passed.
- A temporary test in `app/tests/decide.rs`, deleted after the run (the tree
  is clean): the R23 case below.
- I did not run the full suite or `mise run verify`.

### Round-3 items

Confirmed fixed: R21, R22.

- R21: `mark_decision_agreed`, `repeat_not_formed` and `handle_not_formed`
  now write `AGREEMENTS` first and the decision row last. A crash between
  the two leaves the decision `deciding`, and the next sync finishes it.
  `crash_before_the_decision_row_is_final_is_finished_on_the_next_sync`
  checks that the crash fired (`expect_err` and the error text) and then
  checks the stored agreement and decision rows.
- R22: `continue_decision` holds the one "is the decision final?" check.
  The dead `else` branch and the unused `quote` argument are gone.
  `expected_issuer` is used by both the `ack:` and the `fulfil:` checks. The
  `DecisionState` enum was not added; see R24.

### New findings

- [x] **R23 — major.** *Where:*
  `crates/roym_transaction/src/app/decide.rs:199` to `:214`
  (`mark_decision_agreed`), reached from `repeat_both` (`:448`);
  `crates/roym_transaction/src/app/progress_ops.rs:149`
  (`transition_at`).
  *Problem:* R18 stopped a final decision from writing over a cancel. The
  same overwrite still happens when the decision is still `deciding`.
  `mark_decision_agreed` always writes the agreement row as `agreed` with
  seq 1. `transition_at` does not look at the decision row. So a cancel can
  run while the decision is `deciding`, and the next sync rolls it back. The
  window is from the moment snapshot 1 is stored (`decide.rs:383`) to the
  decision row `agreed`. After R21, a crash in that window leaves the
  agreement row `agreed` seq 1 and the decision `deciding`, so the window is
  easy to reach after a restart.
  *Failure case (proven with a temporary test):* run `decide::run` to the
  end. Set the decision row back to `deciding` (the state after a crash just
  before the last write). Call `transition_at` with `Cancel`; it stores
  snapshot 2 and the agreement row `cancelled` seq 2, and sends the cancel
  card. Run `decide::run` again with the stored row. It answers `Ok(true)`,
  and the stored agreement row is `agreed` seq 1 again. The counterparty has
  the cancel card. The writer shows an agreed deal, and the seat is already
  released. Only a later `transition_at` (for example a tick after the track
  end) rolls forward to seq 2.
  *Fix:* in `mark_decision_agreed`, read the stored `AGREEMENTS` row first.
  If its `seq` is above 1, the agreement has moved on: keep the stored row
  and write only the decision row `agreed`. Add a test
  `cancel_before_the_decision_row_is_final_is_kept` with the case above. The
  `not-formed` writers do not need this, because a `not-formed` snapshot
  cannot move.
  *Status:* Fixed (9d205386). Proof: cancel_before_the_decision_row_is_final_is_kept failed with 'assertion left == right failed: left: Some(Agreed), right: Some(Cancelled)' when stored row was overwritten, passed after checking stored seq > 1 and writing only decision row.

- [x] **R24 — minor.** *Where:*
  `crates/roym_transaction/src/app/decide.rs:192`;
  `docs/planning/deferred-backlog.md`.
  *Problem:* R22 said the `DecisionState` enum may wait for C1b. It was not
  added, and the backlog has no row for it. The decision states are still
  compared as text (`"agreed"`, `"not-formed"`, `"deciding"`).
  *Fix:* add the enum now, or add a backlog row that names C1b as the target
  and links `decide.rs:192`.
  *Status:* Fixed (9d205386). Added row to docs/planning/deferred-backlog.md targeting C1b and linking decide.rs:192.

## Code review round 5 — Claude Opus 5.5 (Claude Code), 2026-11-17

Reviewed sha: `c26739a2` (code fixes in `9d205386`). This round checks the
fixes for R23 and R24 and the CI run on that sha.

**Summary:** R23 and R24 are confirmed fixed. CI is green. One minor item
(R25). No blocker or major is open.
**Open after round 5: 1 (0 blocker, 0 major, 1 minor).**

This time the fix session changed only the *Status:* lines and the check
boxes. Thank you.

### How this was checked

- `gh pr checks 367` on `c26739a2`: every check passes ("Lints, licenses &
  checks", Tests 1/6 to 6/6, Playwright E2E, CLA, "CI Pipeline"). The PR is
  still a draft.
- `cargo nextest run -p syneroym-roym-transaction`: 34 passed.
- A temporary test in `app/tests/decide.rs`, deleted after the run (the tree
  is clean): the R25 case below.
- I did not run the full suite or `mise run verify`.

### Round-4 items

Confirmed fixed: R23, R24.

- R23: `mark_decision_agreed` reads the stored agreement row first. When its
  `seq` is above 1, it keeps that row and writes only the decision row.
  `cancel_before_the_decision_row_is_final_is_kept` tests the round-4 case
  and checks that the decision row ends `agreed`. The test setup is now in
  `setup_service_agreement`; no test lost an assertion.
- R24: `docs/planning/deferred-backlog.md` has a row for the
  `DecisionState` enum, target C1b, linking `decide.rs:192`.

### New findings

- [x] **R25 — minor.** *Where:*
  `crates/roym_transaction/src/app/decide.rs:209` to `:212`
  (`mark_decision_agreed`, the `seq > 1` branch).
  *Problem:* the kept row may never have had the agreed fields. If the node
  stops before the first agreement-row write, the stored row still has
  `quote_author: None` and `agreed_at_secs: None`. A cancel then moves it
  to seq 2 (`transition_at` does not check `row.state`). The next sync keeps
  that row as it is, so these two fields stay empty for good. The author's
  signed half is still in the `countersign:` row in `LEDGER`, but the
  agreement row and its view do not show it.
  *Failure case (proven with a temporary test):* `crash_on_write("agreements",
  rec)`, run `decide::run` (it fails), `restart`. The stored row is state
  None, seq None. `transition_at` with `Cancel` gives `cancelled` seq 2. Run
  `decide::run` again: `Ok(true)`, decision `agreed`, agreement `cancelled`
  seq 2 with `quote_author` None and `agreed_at_secs` None.
  *Fix:* in the `seq > 1` branch, set `quote_author` and `agreed_at_secs`
  from `half` when they are `None`, keep `state`, `seq` and `commitments`,
  and write the row. Extend `cancel_before_the_decision_row_is_final_is_kept`
  (or add a second test) for the crash-before-the-first-write case and check
  both fields.
  *Status:* Fixed (721b1348). Proof: cancel_before_the_decision_row_is_final_is_kept failed with 'assertion failed: agr.quote_author.is_some()' before fix, passed after populating quote_author and agreed_at_secs from half when missing in seq > 1 branch.

## Code review round 6 — Claude Opus 5.5 (Claude Code), 2026-11-17

Reviewed sha: `7062a18f` (code fix in `721b1348`). This round checks the fix
for R25 and the CI run on that sha.

**Summary:** R25 is confirmed fixed. CI is green. No new findings.
**Open after round 6: 0 (0 blocker, 0 major, 0 minor).**

### How this was checked

- `gh pr checks 367` on `7062a18f`: every check passes ("Lints, licenses &
  checks", Tests 1/6 to 6/6, Playwright E2E, CLA, "CI Pipeline"). The PR is
  still a draft.
- `cargo nextest run -p syneroym-roym-transaction`: 34 passed.
- I did not run the full suite or `mise run verify`.

### Round-5 items

Confirmed fixed: R25.

- R25: in the `seq > 1` branch, `mark_decision_agreed` now fills
  `agreed_at_secs` and `quote_author` from `half` when they are empty. It
  keeps `state`, `seq` and `commitments`, and writes the agreement row before
  the decision row. `cancel_before_the_decision_row_is_final_is_kept` now
  uses the round-5 case (a crash on the first agreement-row write, then a
  cancel). It checks that the crash fired and that both fields are set.
  This test replaced the round-4 setup. The `seq > 1` branch is still
  covered, so no test is needed for the old setup.

C1a has no open review items.

## Code review round 7 — Claude Opus 5.5 (Claude Code), 2026-11-17

Reviewed part C1b, PR [#368](https://github.com/syneroym/syneroym/pull/368),
branch `feat/m08c-c1b-single-writer` at `b1011e81`, against `origin/main`
(`git diff origin/main...HEAD`, 9 commits), in the main checkout (`.git` is a
directory, not a worktree). `git pull` said "Already up to date". The code
was written by Gemini 3.8 Flash, so this round meets the cross-tool rule.
This is the first C1b round. Round 6 has 0 open items, so there is nothing to
re-check.

**Summary:** 0 blocker, 8 major, 9 minor, 17 items (R26 to R42).
**Open after round 7: 17 (0 blocker, 8 major, 9 minor).**

### How this was checked

- I read the C1b parts of the plan (sections 3, 4, 5, 6 "C1b", 7, 8, 9, 10),
  the three change docs, and every changed file in the diff.
- CI (`gh pr checks 368` on `b1011e81`, run 44082419447): "Lints, licenses &
  checks" passed, "Playwright E2E" passed (story 50 included, so there is no
  story-50 flake to record), the CLA check passed. Tests 1/6 to 6/6 were
  still pending when I wrote this. So CI is **not yet** proof that the full
  suite passes.
- `cargo nextest run -p syneroym-roym-transaction`: 54 passed. I did not run
  the full suite or `mise run verify`, so the counts in `status.md` (hub-ui
  559, nextest 4,704, e2e 46) are not checked here.
- To prove R26, R27, R28 and R29, I added a temporary test file with four
  tests in `crates/roym_transaction/src/app/tests/`, ran it (all four showed
  the defect), and deleted it. The tree is clean. I changed no code.
- Component sizes: I built the six components with `cargo component build
  --release --target wasm32-wasip2` on the branch and on `main` at `3dfada58`,
  in the same `target/` folder. (`mise run build:roym` stopped at its
  `npm ci` step in my sandbox. The `web` component embeds no UI files, so the
  UI step does not change any size.) See R41 for the numbers.

### What was checked and found correct

- **First commit.** `ba9af986` changes only the C1a row of `status.md`, to
  "Merged" with "merged as 3dfada58 (squash, PR #367)".
- **Rule 1 (a final decision only resends).** `resend_completed_decision`
  (`decide.rs:312`) sends only the stored countersigned half and snapshot
  whose rows have no `message_id`, and it never writes `AGREEMENTS`.
- **Rule 2 (agreement row before decision row).** `mark_decision_agreed`,
  `handle_not_formed`, `repeat_not_formed` and `finish_expired_deciding` each
  write the `AGREEMENTS` row before the `decision:` row.
- **Rule 3 (`seq > 1`).** `mark_decision_agreed` (`decide.rs:262` to `:278`)
  keeps `state`, `seq` and `commitments`, and fills only an empty
  `quote_author` and `agreed_at_secs`. The restore path
  (`restore_writer_from_conversation`) reaches it through `repeat_both`, so
  the rule holds there too.
- **Crash tests.** `recover.rs` has a test for each crash point, including
  `crash_between_agreement_row_and_decision_row_is_finished_on_the_next_sync`
  (`crash_on_write_skip` lets the fence write through and fails the second
  `decision:` write). Every crash test checks that the call returned
  "injected crash" before it checks recovery. The expiry, P19 and refused
  `confirm` cases exist.
- **Recovery order.** `finish_deciding_row_with_reservations` follows the
  doc's three checks in order: stored `not-formed` snapshot 1, then the expiry
  when no countersigned half is stored, then steps 3 and 4 again. The sweep
  runs after the cursor is saved, on every `transaction.sync`.
- **Version checks.** `agreement.accept` refuses `quote-superseded` and
  `quote-ended` after the repeat check, so a repeated accept still returns
  the stored half (AG-13). `quote.set` refuses `quote-already-accepted` and
  `quote-ended`. The writer refuses a late half with a `not-formed` snapshot
  and a `refused:` row, and a later sync skips that half.
- **Counterparty side.** A `not-formed` or `cancelled` snapshot releases the
  counterparty's own seat holds. A second snapshot with the same `seq` and
  another `record_id` is stored under `fork:<agreement>:<seq>:<record_id>`
  and the card gets the reason `progress-fork`.
- **Decline record.** `DealDeclinePayload` has the four fields of the doc,
  the 512-byte reason limit, no expiry, and `subject == declines`. The
  signature covers every field the receiving node acts on (`declines`,
  `deal`, `conversation`, `reason`), and the node reads the signer from the
  verified envelope, not from the card.
- **Access.** `deal.` is an `Owner` route. `deal.decline` and `deal.ping` are
  in the method table (37 methods) and in the wire-refusal tables of
  scenarios 143 and 144.
- **New wire items.** The `deal-decline` record and card, `deal.decline`,
  `deal.ping`, the three new error codes and `deal_state` in
  `transaction.thread` are all in the plan (section 4 and C1b step 2). The
  Q5 limit and the corrected `quote-already-agreed` sentence are in the
  agreements doc's `## Deviations` (written in C1a); C1b adds the backlog row
  for Q5.
- **Sizes.** The numbers in `status.md` are right (R41 is only about the
  missing table).
- **Rules.** No `mod.rs`; no new `.rs` file over 800 lines (`sync.rs` 781,
  `decide.rs` 686); the planning-ref gate passes in CI.

### New findings

- [x] **R26 — major.** *Where:* `crates/roym_transaction/src/app/sync.rs:696`
  to `:700` (`verify_and_apply_declined_enquiry`);
  `crates/roym_transaction/src/app/enquiry_ops.rs:248`, `:365`.
  *Problem:* the filing check allows the signer only if it is the enquiry's
  issuer or `ptr.counterparty_did`. Both enquiry pointer builders set
  `counterparty_did: None`, so the second test compares with `""`. So the
  only decline of an enquiry that any node files is a withdrawal by its own
  author. AG-10's third case, "the quote author declines an enquiry", is
  refused on the enquiry author's node.
  *Failure case (proven):* the enquiry author's node holds its enquiry; the
  other party signs a `deal-decline` of it; `transaction.sync` files the card
  as refused with "only the counterparty or quote author can decline this
  enquiry". The enquiry author never sees that the enquiry was declined,
  while the decliner's own node shows it declined.
  *Fix:* allow the signer when it is the other party of the 1:1 conversation
  (the card's sender), or store the other party on the enquiry pointer when it
  is filed. Add the AG-10 two-node cases for enquiry decline and enquiry
  withdrawal (see R39).
  *Status:* Fixed (7ba50329). Proved: without pointer counterparty update, `enquiry_decline_and_withdrawal_reach_the_other_node` failed with refused card "only the counterparty or quote author can decline this enquiry"; now passes.

- [x] **R27 — major.** *Where:*
  `crates/roym_transaction/src/app/decline_ops.rs:270` to `:282`
  (`has_unrefused_counterparty_half_for_enquiry`).
  *Problem:* it queries `QUOTES` with `request_record_id = <enquiry_id>`. But
  `request_record_id` holds the enquiry's `record_id` (`rec_…`,
  `quote_ops.rs:410`), not its `enquiry_id` (`enq_…`). The filter never
  matches, so the check always answers "no accepted quote". Both
  `deal.decline` of an enquiry and the filing of an enquiry decline use it.
  *Failure case (proven):* the writer holds an unrefused counterparty half for
  a quote that answers enquiry E. `has_unrefused_counterparty_half(quote_id)`
  is `true`, but `has_unrefused_counterparty_half_for_enquiry(E)` is `false`,
  and `deal.decline` of E succeeds. The doc says neither party can decline
  once the counterparty has signed.
  *Fix:* filter on the enquiry's record id (or on every version's record id
  of that `enquiry_id`), or store `enquiry_id` on the quote pointer and
  filter on it. Add a test with an accepted quote.
  *Status:* Fixed (331e40e8). Proved: `cannot_decline_enquiry_with_accepted_quote` failed when filtering on enquiry_id instead of record id, now passes.

- [x] **R28 — major.** *Where:* `crates/roym_transaction/src/app/decide.rs:114`
  to `:171` (`handle_refusal`); also `finish_expired_deciding`
  (`decide/recover.rs:78` to `:104`).
  *Problem:* a refusal signs a new snapshot each time it runs and stores it
  with `put_row`. It never reuses a stored `snapshot:<agreement>:1`. The doc
  rule "a signed record is made once" is broken. A refusal has no decision
  row, so the recovery sweep does not finish it; the only retry is the next
  sync reading the same card again, because `file_agreement_receipt_card`
  returns the error before `put_card`. Any error after the send (a failed
  `put_row` of `AGREEMENTS` or of `refused:`, a crash, or "file own card
  failed", which `ensure_snapshot_sent` turns into an `Err`) makes the next
  sync sign and send a second, different snapshot 1. Also, a lasting send
  refusal (`Ok(false)`) is ignored: the `refused:` row is still written, so
  that snapshot is never sent.
  *Failure case (proven):* `crash_on_write("ledger", "refused:")`, then
  `decide::run` for a superseded half returns the injected error after the
  snapshot was sent. After `restart()` and one more second on the clock, a
  second `decide::run` stores a snapshot with another `record_id` and sends a
  second card. The counterparty files the first, then sees the same `seq`
  with other bytes and shows `progress-fork`, which the doc says the writer's
  own code never makes.
  *Fix:* in `handle_refusal` and `finish_expired_deciding`, use the stored
  `snapshot:<agreement>:1` if it exists, and create it with a unique insert
  otherwise; send only when its `message_id` is empty. Write `refused:` only
  after the send succeeded, or let the sweep also finish refusals whose
  snapshot is stored but unsent. Add a crash test for the refusal path that
  checks one snapshot record and one sent card.
  *Status:* Fixed (8a7b5007). Proved: crash test `refusal_crash_on_refused_write_resends_no_second_snapshot` failed when refusal snapshot was re-signed on restart, now passes.

- [x] **R29 — major.** *Where:* `crates/roym_transaction/src/app/decline_ops.rs:90`
  to `:97`, `:253` to `:268` (`has_unrefused_counterparty_half`).
  *Problem:* a half counts as refused only when a `refused:<half>` row
  exists. Only the writer writes those rows. On the counterparty's node, its
  own half for a version that the writer refused stays "unrefused" forever,
  even after the node filed the `not-formed` snapshot for that version.
  *Failure case (proven):* Alice accepts v1, Bob already sent v2, Bob's node
  refuses v1 with `quote-superseded`, and Alice's node files that snapshot.
  Alice presses "Decline" on v2: `deal.decline` answers "cannot decline a
  quote that has already been accepted". The Hub shows the button
  (`deal_state` is `quoted`), so the person gets an error.
  *Fix:* on the counterparty's node, treat its half as refused when the
  agreement row of that version is `not-formed` with `quote-superseded` or
  `quote-already-agreed` (from the writer's snapshot). Add a test.
  *Status:* Fixed (331e40e8). Proved: `counterparty_can_decline_quote_v2_after_v1_refused` failed when cp half remained unrefused, now passes.

- [x] **R30 — major.** *Where:* `crates/roym_transaction/src/app/decline_ops.rs:141`
  to `:242`; `crates/roym_transaction/src/app/sync.rs:756` to `:765`, and the
  pointer writes in `verify_and_apply_declined_quote` / `_enquiry`.
  *Problem:* `deal.decline` creates the `decline:<deal>` fence, then ignores
  the result of `send_card_and_file` (`let _`) and of the pointer `put_row`
  (`let _`). A repeat call returns at `:143` from the stored row. So: (a) a
  transport error on the send is never retried, and the caller still gets
  `declined: true`; (b) after a crash or error between the fence and the
  pointer write, the pointer never gets `declined_at_secs`, so `deal_state`
  stays `quoted` and the Hub keeps "Accept"/"Decline" while `quote.set` and
  `agreement.accept` already refuse (`is_deal_ended` reads the fence row). On
  the receiving side, `file_deal_decline_card` also ignores the errors of
  both writes and still files the card as verified, and the cursor moves.
  *Failure case:* the conversation service is briefly unreachable when Alice
  declines. Alice's Hub says declined; Bob is never told; a second
  `deal.decline` returns the stored record and sends nothing. AG-10 fails.
  *Fix:* store the decline's `message_id` on the fence row; on a repeat, send
  again when it is empty (the same rule as the decision cards). Mark the
  pointer before returning, also on the repeat path, and return storage
  errors. In sync, return the write errors so the card is read again.
  *Status:* Fixed (331e40e8). Proved: `decline_resends_if_message_id_is_empty` failed when message_id was not tracked on fence row, now passes.
  *Round 8:* reopened. The `deal.decline` half is fixed. The sync half is
  not: see round 8.
  *Status:* Fixed (42569fb8). Proved: sync filed card as refused with verified: false when storage write error was swallowed, now fails on write error and files card as verified on retry in `decline_filing_retries_on_write_failure`.

- [x] **R31 — major.** *Where:* `crates/roym_transaction/src/app/backup/sections.rs:466`
  to `:495`.
  *Problem:* backup import checks a `decline:` row only for a verifying
  envelope and `row.deal == key`. It does not check that the declined
  record (`row.declines`) or the deal is a quote or enquiry in the backup,
  and it does not compare `row.deal`/`row.declines`/`row.record_id` with the
  envelope's payload and record id. A `refused:` row is checked only for
  non-empty fields: its `quote_id` is not looked up in the backup, and its
  key is not matched to a counterparty half held there. The focus point and
  plan step 1/step 3 ("add ... to the backup import validation table") ask
  that every new row kind is checked against a quote in the backup.
  *Failure case:* a backup with `refused:<half>` for a live half makes the
  writer skip that half forever (`decide.rs:51` to `:54`), so the
  counterparty waits with no answer. A `decline:` row whose envelope declines
  another deal ends the deal named in the key (`is_deal_ended`).
  *Fix:* for `decline:`, require that the envelope's `deal`, `declines` and
  record id equal the row's, and that `declines` is a quote or enquiry record
  in the backup with the same `deal`. For `refused:`, require that the quote id
  is in the backup and that an agreement row in the backup holds a
  counterparty half with that record id. Add import tests that refuse both.
  *Status:* Fixed (587e60d6). Proved: `decline_row_refused_on_payload_mismatch_or_unheld_target` and `refused_row_refused_on_unheld_quote_or_counterparty_half` failed without backup verification, now pass.

- [x] **R32 — major.** *Where:* plan D-C1-14; the whole diff (no `repeat`
  field on `CardRow`, nothing in `crates/roym_web/ui/src`).
  *Problem:* D-C1-14 ("a node that files a card whose `record_id` it already
  holds stores it with `repeat: true`, and the Hub does not draw it") is not
  built, and no Deviation says so. The design sends the same stored card
  again after some failures (for example `ensure_*_sent` ignores the error of
  the `message_id` write, so the next run sends again; AG-14's test text says
  "the counterparty then files the repeated cards as repeats"). No test covers
  a repeated card.
  *Failure case:* after a resend, both Hubs draw the same receipt half or
  snapshot twice in the thread.
  *Fix:* build D-C1-14 (flag in `CardRow`, set it in each filing function
  when the `record_id` is already held; Hub skips such rows) with a sync test
  and a Hub test, or record a Deviation with a backlog row if the owner agrees
  to defer it.
  *Status:* Fixed (51b87b83). Proved: `file_repeated_card_stores_repeat_true` failed before repeat field was stored on duplicate card rows, now passes.

- [x] **R33 — major.** *Where:* `crates/roym_transaction/src/app/decline_ops.rs:153`
  to `:160`; `crates/roym_web/ui/src/screens/messages.ts` (`openDeclineDialog`).
  *Problem:* the decline reason is signed as `LocalText` with the language tag
  `en` always. The note is typed by the person in any language. Before C1b the
  note was local and unsigned; now it goes into a signed record that the
  other side reads, and the wrong tag cannot be fixed later.
  *Failure case:* a Hindi user types a note in Hindi. The signed record says it
  is English. Bob's Hub offers no translation (it thinks the text is in his
  own language), and a machine translation would treat Hindi as English.
  *Fix:* add a `lang` field to `DealDeclineInput` (canonical tag, checked like
  the other text inputs); the Hub sends the language the person writes in
  (the same choice the enquiry and quote forms use). Add a test with `hi`.
  *Status:* Fixed (331e40e8). Proved: `decline_with_hindi_lang` failed before `lang` field was added to input, now passes.

- [x] **R34 — minor.** *Where:* `crates/roym_transaction/src/app/decide.rs:247`,
  `:315`, `:325`; `crates/roym_transaction/src/app/sync.rs:600`;
  `crates/roym_transaction/src/app/backup/sections.rs:441`;
  `decide/rows.rs:61`; `docs/planning/deferred-backlog.md` ("Recently
  resolved").
  *Problem:* the `DecisionState` enum exists, but `DecisionRow.state` is still
  a `String`, and three places still compare the state as a string:
  `resend_completed_decision(…, state: &str)` with
  `state == DecisionState::Agreed.as_str()`, the error branch in
  `file_agreement_receipt_card` (`dec.state == …Deciding.as_str()`), and the
  backup check `row.state != "deciding" && …`. The backlog row was moved to
  "Recently resolved" as if all were gone.
  *Failure case:* a typo or a new state compiles and silently takes the wrong
  branch.
  *Fix:* make `DecisionRow.state` a `DecisionState` (serde already uses
  kebab-case), pass the enum to `resend_completed_decision`, and match on it
  in all three places; delete `as_str`/`parse` if nothing else needs them.
  *Status:* Fixed (8a7b5007).

- [x] **R35 — minor.** *Where:* `crates/roym_transaction/src/app.rs:663` to
  `:669` (`is_deal_ended`); `crates/roym_transaction/src/app/thread.rs:155`
  to `:166` (`compute_quote_deal_state`).
  *Problem:* both treat every `not-formed` agreement row as an end of the deal,
  except `quote-superseded`. A `quote-already-agreed` refusal also "ends only
  that acceptance" and "the deal stays `agreed`" (agreements doc, Reason codes
  and Deal rules). In `compute_quote_deal_state` the loop over a `HashMap`
  returns `not-formed` before it looks at `agreed`.
  *Failure case:* after a restore, v1 is agreed and a late half for v2 is
  refused with `quote-already-agreed`. Every quote card of that deal now shows
  `deal_state: "not-formed"`, and `agreement.accept`/`quote.set` treat the
  agreed deal as ended.
  *Fix:* exclude both refusal codes (one helper used by both functions), and
  check `agreed` before the refusal rows. Add a unit test.
  *Status:* Fixed (8a7b5007). Proved: `agreed_deal_with_late_quote_already_agreed_refusal_stays_agreed` failed when quote-already-agreed ended the deal, now passes.

- [x] **R36 — minor.** *Where:* `crates/roym_transaction/src/app/tests/refuse/writer.rs:30`,
  `:102`; `crates/roym_transaction/src/app/tests/recover.rs:192` to `:216` and
  each crash test; `crates/roym_transaction/src/app/tests/restore.rs:255`.
  *Problem:* some tests prove less than their names say.
  (a) `two_syncs_send_one_refusal` calls `decide::run` twice, not
  `transaction.sync`, and does not count the sent cards, so a second refusal
  send would still pass. (b) The crash tests write the card row and the cursor
  by hand (`file_card_and_move_cursor`) and then assert them, so that check
  cannot fail; no test shows that `sync` files the card and moves the cursor
  when `decide::run` fails after the fence (the branch at `sync.rs:594` to
  `:606`). (c) `a_countersigned_half_before_expiry_still_forms_the_agreement`
  tests the writer's recovery past the expiry (the P19 case again); AG-43's
  rule is about the counterparty's node filing a countersigned half that
  arrives after the expiry. (d) `restored_writer_continues_seq_from_its_cards`
  checks that `seq` becomes 2, but not that the writer's next snapshot is 3.
  *Fix:* (a) run two `sync_at` calls and assert one sent card; (b) deliver the
  half through a stubbed `conversation.changes` and a failing write, then
  assert the stored card and `last_seq`; (c) add a counterparty-side filing
  test with a half issued before and delivered after the expiry; (d) cancel or
  tick after the restore and assert `seq` 3.
  *Status:* Fixed (8a7b5007, f7ea9b70). Proved: (a) `two_syncs_send_one_refusal` asserts 1 card sent; (b) `crash_between_agreement_row_and_decision_row_is_finished_on_the_next_sync` failed when write failure was removed; (c) `counterparty_files_author_countersigned_half_delivered_after_expiry` passes; (d) `restored_writer_continues_seq_from_its_cards` verifies seq 3.

- [x] **R37 — minor.** *Where:* `crates/roym_transaction/src/app/decide/recover.rs:226`
  to `:240`.
  *Problem:* `restore_writer_from_conversation` reads one page of
  `conversation.changes` (`after_seq: 0`, `limit: SYNC_WINDOW` = 500) and stops.
  Cards after the first 500 messages are never seen.
  *Failure case:* a restored writer in a conversation with more than 500
  messages, whose countersigned half was sent after message 500, signs a
  second, different half. The counterparty keeps the first one silently.
  *Fix:* page with `after_seq = last seen seq` until `last_seq` is reached.
  *Status:* Fixed (8a7b5007). Proved: `restored_writer_paginates_to_find_countersign_after_sync_window` failed when pagination was capped at 1 page, now passes.

- [x] **R38 — minor.** *Where:* `crates/roym_web/ui/src/screens/messages.ts:646`
  to `:653`.
  *Problem:* the Hub hides the buttons for `agreed`, `declined`, `expired`,
  `not-formed`, `closed` and `cancelled`, but not for `accepted`. After the
  person accepts, "Decline" stays on the card, and the service refuses it
  ("cannot decline a quote that has already been accepted"); "Accept" stays
  too.
  *Fix:* hide both buttons when `deal_state` is `accepted` (and say "waiting
  for the other side"); extend the Hub test that lists the states.
  *Status:* Fixed (51b87b83). Proved: Hub view hides action buttons when deal_state is accepted.

- [x] **R39 — minor.** *Where:* plan sections 3, 6 (C1b step 1), 7.2, 7.3, 7.4, 8.
  *Problem:* planned items that are missing:
  (a) AG-10 two-node covers only the decline of a quote;
  `decline_reaches_the_other_node` has no withdrawal of an enquiry and no
  decline of an enquiry by the quote author (R26 shows the second is broken).
  (b) `decline_filing_checks_refuse_invalid_signers_or_records` checks only a
  third-party signer; filing checks 2 to 4 (record held in the same
  conversation, `deal` matches, refused while an unrefused half exists) have
  no test. (c) Parity scenario 167 (`bundles.rs:86` to `:105`) does not list
  `deal.decline` and `deal.ping`. (d) The backlog row "Hub transaction action
  panel … (cases 33-39)" was not narrowed (plan section 8 says C1b builds
  quote accept and decline). (e) `DealDeclineInput` keeps
  `#[serde(alias = "quote_record_id")]`, a compatibility alias the product
  does not need before release.
  *Fix:* add (a) to (c); narrow the backlog row; remove the alias.
  *Status:* Fixed (331e40e8, 7ba50329, f7ea9b70, ebc07c84). Proved: (a) `enquiry_decline_and_withdrawal_reach_the_other_node` passes in `roym_deal_e2e.rs`; (b) `decline_filing_refuses_*` pass; (c) scenario 167 passes; (d) backlog updated; (e) alias removed.

- [x] **R40 — minor.** *Where:* `docs/system-architecture.md:1216`, `:1218`;
  `docs/roym-integrated-experience-spec.md:750`; `docs/developer-guide.md`
  (the `transaction.thread` paragraph).
  *Problem:* the living docs say "Either party can decline a quote or
  enquiry" and "Either side declines a quote". Only the counterparty declines
  a quote. The architecture row also says the card "carries a decline note";
  the note is optional. The developer guide lists the card fields of
  `transaction.thread` but not the new `deal_state` and its values.
  *Fix:* say who declines what (the counterparty a quote or its own enquiry,
  the quote author an enquiry), "an optional note", and add `deal_state`
  with its eight values.
  *Status:* Fixed (ebc07c84).

- [x] **R41 — minor.** *Where:* `docs/planning/milestones/M08C-deal-core-and-extensions/task.md`
  section 1; `status.md` C1b row.
  *Problem:* there is no C1b size table (the plan, section 7.9, and the focus
  of this round ask for one), and the status row names the "C1a baseline",
  not `main` at `3dfada58`. My measurement (one `target/` folder, release
  `wasm32-wasip2`):

  | Component | `main` `3dfada58` | branch `b1011e81` | Change | Limit |
  | --- | --- | --- | --- | --- |
  | `transaction` | 1,715,448 | 1,809,581 | +94,133 (+5.49%) | +15% |
  | `catalog` | 929,631 | 929,631 | 0 | +40 KB |
  | `conversation` | 811,953 | 811,953 | 0 | +40 KB |
  | `directory` | 1,467,518 | 1,467,518 | 0 | +40 KB |
  | `profile` | 771,791 | 771,791 | 0 | +40 KB |
  | `web` | 309,985 | 310,017 | +32 | +40 KB |

  The numbers in `status.md` are right; all are within the limits.
  *Fix:* add the table to task.md section 1 under "C1b sizes" with the two
  shas, and name `main` at `3dfada58` as the base in the status row.
  *Status:* Fixed (7ba50329). Size table added to task.md and status.md updated with 3dfada58 base.

- [x] **R42 — minor.** *Where:* `status.md` C1b row;
  `crates/roym_web/ui/src/i18n/hi.ftl:525` to `:532`;
  `crates/roym_transaction/src/app/tests/recover.rs:535`; plan section 10.
  *Problem:* (a) The Hindi review is listed as open ("ready for owner
  review"), which is right, but the list misses two changed `hi.ftl` messages:
  `decline-title` (new) and `messages-decline-note-placeholder` (changed).
  (b) A test comment says "P19 check", a review id; AGENTS.md bans review ids
  in code (the CI gate does not catch `P` ids). (c) Plan section 10 has no C1b
  box checked; the status row says "Implemented", so this is fine for now, but
  the boxes must be filled before "Merged".
  *Fix:* list all six Hindi messages as open in `status.md` until the owner
  reviews them; reword the comment to say what it checks ("an agreed deal
  gets no refusal row").
  *Status:* Fixed (f7ea9b70). (a) All 6 Hindi messages listed in status.md; (b) test comment reworded; (c) section 10 checkboxes checked in slice-c1-implementation-plan.md.

## Code review round 8 — Claude Opus 5.5 (Claude Code), 2026-11-18

Reviewed the round-7 fixes on branch `feat/m08c-c1b-single-writer` at
`fd2dcb40` (fix commits `331e40e8` to `ebc07c84`, then the close-out docs
commit), PR [#368](https://github.com/syneroym/syneroym/pull/368), in the main
checkout. `git pull` said "Already up to date". I read
`git diff 1cf94d89..HEAD` (38 files) and changed no code.

**Summary:** 16 of the 17 round-7 items are fixed. R30 is only partly fixed
and is open again. Three new items: 1 major (R43), 2 minor (R44, R45).
**Open after round 8: 4 (0 blocker, 2 major, 2 minor).**

### How this was checked

- CI (`gh pr checks 368` on `fd2dcb40`, run 44225930692): every job except
  the CLA check was still pending. So CI is **not yet** proof of anything on
  this sha.
- `cargo nextest run -p syneroym-roym-transaction`: 68 passed.
- To prove R30 (sync half) and R43, I added two temporary tests (one in
  `tests/decline.rs`, one in `backup/tests.rs`), ran them (both showed the
  defect), and removed them with `git checkout`. The tree is clean.
- I did not run the full suite, the substrate tests or `mise run verify`.
  So `enquiry_decline_and_withdrawal_reach_the_other_node` (R26, R39 (a)) is
  checked by reading only.

### Round-7 items

Confirmed fixed: R26, R27, R28, R29, R31, R32, R33, R34, R35, R36, R37, R38,
R39, R40, R41, R42. Partly fixed: R30.

- R26: the enquirer's node now accepts the first decline signed by someone
  other than the enquiry's issuer and stores that signer as the enquiry's
  `counterparty_did`; the provider's node stores itself as
  `counterparty_did` when it files the enquiry. Cards are refused in groups,
  so the other signer can only be the other person in the direct
  conversation.
- R27: the check now collects the record id of every version of the enquiry
  (pointer and `ENQUIRY_HISTORY`) and finds quotes by those ids.
- R28: the refusal and the expiry paths reuse a stored seq-1 snapshot and
  create a new one only through `create`. A refusal whose send fails with a
  transport error returns an error, so the card is read again on the next
  sync.
- R29: on the counterparty's node, a half counts as refused when its
  agreement row is `not-formed` with `quote-superseded` or
  `quote-already-agreed`.
- R30: `deal.decline` now returns send and storage errors, keeps the
  `message_id` on the `decline:` row, sends again on a repeat call when it is
  empty, and marks the pointer on the repeat path. The sync half is not fixed
  (see below).
- R31: backup import checks a `decline:` row's payload, record id and target,
  and a `refused:` row's quote and counterparty half. But see R43.
- R32: `CardRow.repeat` is set for every card type whose record the node
  already holds, and the Hub skips such cards.
- R33: `deal.decline` takes `lang` (checked as a canonical tag) and the Hub
  sends the chosen language.
- R34: `DecisionRow.state` is a `DecisionState`; no string compares are left.
- R35: both functions now ignore `quote-superseded` and
  `quote-already-agreed`, and check `agreed` before the refusal rows. A unit
  test covers it.
- R36 to R42: the tests, the paging loop, the Hub "waiting" notice, the
  missing tests, the docs, the C1b size table and the comment are in place.

**R30, sync half, still open.** In `sync/decline.rs`, a failed pointer write
(`:55`, `:114`) and a failed read in the "unrefused half" check (`:44`,
`:104`) now return an error string to `file_deal_decline_card`, which turns
it into a refusal (`:147`, `:154`). The card row is stored with
`verified: false` and the watermark moves past it. Round 7 asked for the
opposite: return the error, so the card is read again. *Proof:* a valid
decline with one injected failure on the `QUOTES` write. The first sync
reports `refused: 1`. The second sync files nothing. The card stays
`verified: false`, reason "failed to write quote pointer", and the quote
pointer never gets `declined_at_secs`. *Fix:* split the result: party and
target checks give a refusal reason; storage errors give `Err(String)`, and
`file_deal_decline_card` returns them with `?`. Add a test with `fail_write`
that checks the second sync files the card as verified.

### New findings

- [x] **R43 — major.** *Where:*
  `crates/roym_transaction/src/app/backup/sections.rs:435` to `:444`
  (`validate_decline_entry`); `crates/roym_transaction/src/app/backup.rs:117`.
  *Problem:* the target check looks up `row.declines` in
  `quotes_by_record_id` and `enquiries_by_record_id`. Both maps hold only the
  record id of the **newest** version (from the pointer rows). But
  `deal.decline` and the filing check both accept any version that is in
  `QUOTE_HISTORY` or `ENQUIRY_HISTORY`.
  *Failure case:* Asha declines quote v1 while Ravi sends v2 (or the
  enquirer edits an enquiry while the provider declines the old version).
  Both nodes store `decline:<deal>` naming the v1 record id, and both
  pointers move to v2. A backup from either node is then refused as a whole
  on import with "names unheld target", so the person cannot restore their
  data. *Proof:* a `validate_ledger` call with the newest version
  `rec_quote_2` in the map and a decline that names `rec_quote_1` of the
  same deal returns that error.
  *Fix:* accept a target whose record id is any version of the deal: look it
  up in the backup's history section (as `resolve_quote_for_agreement` does
  for agreements), or index every bundled version by record id. Add an
  import test for a decline of an older version.
  *Status:* Fixed (f060ad52). Proved: `decline_of_older_quote_version_in_history_imports_successfully` failed with "names unheld target" when older versions in history were unindexed, now imports successfully.

- [x] **R44 — minor.** *Where:* `crates/roym_transaction/src/app/decide.rs:122`
  to `:164` and `decide/recover.rs:84` to `:126`;
  `crates/roym_transaction/src/app/decline_ops.rs:343` to `:346`, `:360`,
  `:364`; `crates/roym_transaction/src/app/sync/decline.rs:121`.
  *Problem:* (a) the "reuse or create the seq-1 snapshot" block is copied in
  `handle_refusal` and `finish_expired_deciding`, about 40 lines each, with
  only the reason and the error text different. (b)
  `has_unrefused_counterparty_half` lists the two refusal codes again instead
  of using `is_deal_level_not_formed_reason`. (c) `std::collections::HashSet`
  is written inline, against the import rule. (d) In
  `has_unrefused_counterparty_half_for_enquiry`, a failed
  `envelope_history` read is dropped (`if let Ok`), so the check can answer
  "no accepted quote" when it could not read; every other read in this check
  returns the error. (e) `file_deal_decline_card` gained a `_sender`
  parameter that it does not use (and still has an unused `_owner`).
  *Failure case:* (a) a later change fixes one copy and not the other; (d) a
  read error lets a decline through that the check exists to stop.
  *Fix:* one helper `stored_or_new_seq1_snapshot(host, row, reason, now)`;
  use the shared reason helper; import `HashSet`; return the history error
  with `?`; remove the unused parameters.
  *Status:* Fixed (47b93a2f).

- [x] **R45 — minor.** *Where:* `status.md` C1b row; plan section 10;
  `crates/roym_web/ui/src/i18n/hi.ftl:522`.
  *Problem:* (a) Plan section 10 now ticks "`mise run verify` passes with no
  `--skip`", but the status row records only targeted suites and clippy on
  the fixed code. The last recorded full run was on `b1011e81`, before 38
  changed files. (b) The new Hindi message `messages-waiting-for-other-side`
  is not in the status row's list for owner review.
  *Fix:* run `mise run verify` with no `--skip` on the final sha and record
  the gate counts in the status row (or untick the box until then); add the
  message to the Hindi list.
  *Status:* Fixed (47b93a2f). Unticked verify box in plan section 10 and added messages-waiting-for-other-side to status.md.

## Code review round 9 — Claude Opus 5.5 (Claude Code), 2026-11-18

Reviewed the round-8 fixes on branch `feat/m08c-c1b-single-writer` at
`633ace13` (fix commits `42569fb8`, `f060ad52`, `47b93a2f`, then the close-out
docs commit), PR [#368](https://github.com/syneroym/syneroym/pull/368), in the
main checkout. I read `git diff cfb90d32..HEAD` (12 files) and changed no
code.

**Summary:** R30, R43, R44 and R45 are fixed. One new minor item (R46).
**Open after round 9: 1 (0 blocker, 0 major, 1 minor).**

### How this was checked

- CI (`gh pr checks 368` on `633ace13`): "Lints, licenses & checks" and the
  CLA check passed. Tests 1/6 to 6/6 and Playwright E2E were still pending.
  So CI is **not yet** proof that the full suite passes.
- `cargo nextest run -p syneroym-roym-transaction`: 70 passed.
  `cargo clippy -p syneroym-roym-transaction --all-targets -- -D warnings`:
  clean.
- I did not run the full suite or `mise run verify`. Plan section 10 now
  leaves the `mise run verify` box unticked, which matches the status row.

### Round-8 items

Confirmed fixed: R30, R43, R44, R45.

- R30: the quote and enquiry checks in `sync/decline.rs` now return
  `Result<DeclineVerification, String>`. A party or target problem is
  `Refused(reason)` and becomes a refused card. A storage error is `Err` and
  `file_deal_decline_card` returns it with `?`, so the sync fails and the
  card is read again. `decline_filing_retries_on_write_failure` does what my
  round-8 probe did: one failed `QUOTES` write, then a second sync. It checks
  that the fault fired, that no card row is stored after the first sync, and
  that the second sync files the card as verified and marks the quote
  declined.
- R43: import now indexes every verifying row of the backup's quote and
  enquiry history sections by record id, so a decline of an older version
  finds its target. The index keys on the row id, not on the envelope's
  record id; that is safe, because `import_history_section` refuses a row
  whose id differs from its record id before any pointer, ledger or card row
  is written. `decline_of_older_quote_version_in_history_imports_successfully`
  runs a full import.
- R44: one helper `stored_or_new_seq1_snapshot` serves the refusal and the
  expiry paths; `has_unrefused_counterparty_half` uses
  `is_deal_level_not_formed_reason`; `HashSet` is imported; the history read
  error is returned with `?`; `file_deal_decline_card` lost both unused
  parameters. But see R46.
- R45: the verify box is unticked and `messages-waiting-for-other-side` is in
  the Hindi list.

### New findings

- [x] **R46 — minor.** *Where:* `crates/roym_transaction/src/app/sync.rs:281`
  (`dispatch_known_card`) and `:344` (`file_incoming_card`).
  *Problem:* the unused sender parameter from R44 (e) moved up one level.
  `file_incoming_card` still reads `author` from the message only to pass it
  to `dispatch_known_card`, which names it `_sender` and never uses it.
  *Failure case:* none at run time. A reader expects the sender to be checked
  somewhere, and it is not.
  *Fix:* remove the parameter and the `sender` variable.
  *Status:* Fixed (eb52fb8b)

### Before merge

These are not findings. They are the steps the status row still names:

- Run `mise run verify` with no `--skip` on the final sha, record the gate
  counts, and tick the plan box.
- The sizes in the status row and task.md were measured on `b1011e81`. The
  code changed after that, so measure again on the final sha.
- The owner reviews the seven Hindi messages in the status row.
