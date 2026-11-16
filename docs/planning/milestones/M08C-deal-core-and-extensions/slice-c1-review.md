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
