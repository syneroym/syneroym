# M08C slice C3b — review

*Reader: the developer who fixes the C3b plan or code, and the project
owner. This file holds every review finding for slice C3b (parts C3b1 and
C3b2). Plan review findings are P1, P2, …; code review findings are R1, R2,
…. Ids are never renumbered.*

## Plan review round 1 — Claude Sonnet 5.5 (Claude Code), 2026-11-21

Checked [slice-c3b-implementation-plan.md](slice-c3b-implementation-plan.md)
on branch `feat/m08c-c3b-scheduling` at `d97e799b` (the plan commit and the
owner-answers commit, on top of `main` at `f91e7c32`). This is the main
checkout, not a worktree (`git rev-parse --git-dir` is `.git`). `git pull`
was blocked by the sandbox once and then ran outside it: "Already up to
date". The plan was written by Claude Opus 5.5; this round runs on Claude
Sonnet 5.5, so the reviewer is a different model from the author.

Read: AGENTS.md ("Writing for readers" and the code rules);
session-strategy.md ("The review file"); the plan (all 1,624 lines);
task.md and status.md of M08C; the three change docs of the prompt
(roym-extensions: criteria, "Reserve and release hooks", Tasks, Deviations;
roym-agreements: AG-14 to AG-16, AG-36, "Commitment failure", "One way out
of an end state", "The writer's decision", "Who reserves", Deviations;
roym-value-exchange: VX-10 and its design note); the C3a review file (kinds
of findings before; its section format). task.md's C3b section names the
same three change docs as the prompt, so no change doc is missing from the
list. There is no earlier round for C3b, so no item needed re-checking.

**Summary: 12 findings: 0 blocker, 3 major, 9 minor.**

### What was checked and found correct

- **The nine seat sites** (section 2.2, point 3). I found each by
  `git grep` and read it: `reserve_counterparty_slots`
  (`agreement_ops.rs:260`), the counterparty's release in `agreement.get`
  (`:508` to `:521`) and in `tick_conversation_at`
  (`progress_ops.rs:341` to `:354`), `reserve_author_holds`
  (`decide.rs:419`), `finalize_snapshot`'s release (`progress_ops.rs:104`
  to `:110`), `apply_newer_snapshot`'s release (`receipts.rs:289` to
  `:295`), `release_all_author_holds` and the recovery confirm
  (`recover.rs`), `prepare_terms` and `validate_and_apply_slot`
  (`quote_ops.rs:144`, `:419`). No tenth site exists in `src/` outside the
  `fulfil:` kind rule (`sections.rs:378`).
- **The existing `Reservations` trait** (`hold.rs:29` to `:52`), its two
  users (`decide::run_with_reservations`, the recovery tests) and the
  `hold.rs` size (196 lines) match the plan. `MAX_SLOT_CAPACITY` is in
  `hold.rs:18`.
- **Point 6 (a release can free another agreement's seat).** True: the
  release reads the `hold:` row and then deletes two rows in a batch
  (`hold.rs:180` to `:195`). The data layer has `batch-mutate` with
  `delete`, and no compare-and-delete. The "never delete a seat row" design
  answers it.
- **`catalog`'s facts.** `availability.rs` is 152 lines and self-contained:
  it never reads `listings`, so the move needs no new `depends_on` edge.
  `SCHEMA_VERSION` is 4 (`app.rs:25`), `transaction`'s is 6
  (`app.rs:57`), `LedgerKind::Hold` and `::Seat` are at `ledger.rs:11`
  and `:12`.
- **The routing facts.** `("availability.", CATALOG, Owner)` is at
  `router.rs:30`. `services.rs` has `ALL: [Service; 6]` and
  `SIBLINGS: [Service; 5]`. `transaction`'s only two `CATALOG` calls are the
  two slot calls (`hold.rs:107`, `quote_ops.rs:445`), so Q8 (a) is safe.
- **The data-layer fence.** `create` is all-or-nothing and names "the
  first existing id" (`data-layer.wit:186` to `:201`;
  `data_db/src/tests_create.rs:99`). This is the fact that P1 depends on.
- **Change-doc lines.** I checked `agreements:129` to `:132` (AG-14, AG-15,
  AG-16, AG-36), `:528`, `:620`, `:624`, `:656`, `:724`, `:745`, `:757`,
  `:974`, `:1009`, and `value-exchange:150`, `:337`: all hit the named item.
  The criteria-to-change-doc mapping in section 3 is right: AG rows to
  roym-agreements, EX rows to roym-extensions, VX-10 to
  roym-value-exchange.
- **The transition code the plan relies on.** `progress::apply` has
  `AgreementEvent::Failure` with the cascade (`progress.rs:323` to
  `:356`); `Step` tables have `reservation-lost` and
  `other-commitment-failed` (`transition.rs:115`, `:120`). The snapshot has
  a per-commitment `halves` list (`progress.rs:73`), so `release_ended`'s
  "skip a commitment with a half" rule is implementable.
- **The signing path.** The writer's countersign uses
  `signing::person_principal` with no session (`decide.rs:701`), as D-C3b-10
  says. `resolve_principal_and_owner` (`app.rs:288`) shows the three
  certificate errors that P5 is about.
- **No other caller of `availability.*` exists** outside the Hub
  (`listings.ts:597` to `:623`), the tests the plan lists, and the demo
  story 60. `roymctl` calls none.
- **The `check_roym_deps` rule** (`xtask/src/main.rs:155` to `:162`) lists
  the siblings by name, as the plan says; the new crate's dependencies
  (`app-host`, `roym-core`, `serde`, `serde_json`, and the target crates)
  are all in the allowed list.
- **Size rules.** `quote_ops.rs` 751, `decide.rs` 742, `sync.rs` 722,
  `backup.rs` 770 and `roym.rs` 602 lines: the plan's section 7.8 is right,
  and putting new code in sibling files is the right call.
- **"How this is shown to an end user"** exists after the goal, has 4
  bullets for C3b1 and 3 for C3b2, then the developer bullets. It does not
  invent a story. One missing fact is in P11.
- **Demo answer.** The owner said yes to `- none` for both parts (status.md
  row; plan section 2.1). The plan does not need to ask again.

### View on the owner questions

The owner answered Q1 to Q8 on 2026-11-21 (each recommendation). I do not
change those answers. My view, for the record:

- **Q1 (split).** Agree. The order (move first, behaviour change second) is
  safe, and the plan's reason for it is sound. P3 and P4 show two places
  where the split has a cost that the plan does not say yet.
- **Q2 (the manifest names its service).** Agree. It is the only option
  that keeps `transaction` free of an extension name, and the exit
  criterion (no `"scheduling"` string in `transaction`) can be checked.
- **Q3 (the `prepare` hook).** Agree. P7 adds the second caller
  (`quote.measure`) that the plan does not name.
- **Q4 (lazy expiry).** Agree, with the fence fix of P1 applied. Lazy
  expiry is only as safe as the single `create` that takes a seat.
- **Q5 (a, three-part proof).** Agree. A real expiry across two nodes would
  need a test-only clock or an operator setting. The recorded deviation is
  the right size.
- **Q6 (`- none` for both parts).** Agree for C3b1. For C3b2 the owner's
  explicit yes stands; the Hub unit tests and the two-node test (7.4) carry
  the proof, and the plan says so.
- **Q7 (keep the `availability.*` names).** Agree.
- **Q8 (replace the `catalog` edge).** Agree. I checked: after the move,
  `transaction` has no `CATALOG` call.
- **The owner's three risk measures.** All three are in the plan. The
  service-list test (D-C3b-15) is a good idea, but its table is
  incomplete (P2). The size check before wiring is good. The "`confirm`
  never takes a seat" rule is sound; P4 is about its effect on the late
  countersigned half.

### Findings

- [ ] **P1 — major.** *Where:* D-C3b-5, the `reserve` pseudo-code
  (`create([seat:<slot_id>:<n>:<top+1>, rsv:<r>])` and its two `Some(...)`
  arms).
  *Problem:* The plan lists the rows in the order seat first, `rsv:` second.
  `create` writes all rows or none, and when any id exists it returns *the
  first existing id in the order of `values`* (WIT doc at
  `data-layer.wit:186` to `:201`; `tests_create.rs:99`,
  `create_writes_nothing_and_names_the_first_existing_id`). The two rows of
  one reservation are created together, so one exists only when the other
  does. If a second call with the same key loses the race, the first
  existing id is the *seat* row, and the plan reads that as "lost the race
  for seat n, continue". The `Some(rsv:<r>)` arm ("an earlier crash")
  cannot be reached in this order. Today's code uses the other order: the
  hold row first, and it tests `id == h_key` (`hold.rs:136` to `:163`).
  The plan reverses it.
  *Failure case:* the plan says two syncs at the same time are safe
  (agreements doc, "The writer's decision", and D-C3b-7). Slot capacity 1.
  Two overlapping syncs of one decision both pass step 2 (the fence row
  exists with the same agreement, so both go on) and both call `reserve`
  for the same key. Call A takes the seat. Call B's `create` returns the
  seat id; B tries the next seat; there is none; B answers `slot-taken`.
  B's run then stores a `not-formed` snapshot (`seq` 1), while A's run
  countersigns. The two outcomes fight over one fence-protected row, and
  the loser's cards (a `not-formed` snapshot for an agreement that A made)
  may have been sent already. EX-21 says "a repeated reserve returns the
  same reservation"; here it returns a refusal.
  *Fix:* (1) Put `rsv:<r>` first in the list, so a same-key race returns
  the `rsv:` id. (2) Do not decide by the id alone. After any
  `Some(id)`, run `current(key)` again: if it now finds the reservation,
  return it; only then go on to the next seat. This also covers a seat
  row and a reservation row that were created by two different callers.
  (3) Add a test that makes the race happen on `TestHost`
  (`two_reserves_of_one_key_return_one_reservation`: the host runs the
  first caller's `create` between the second caller's read and its own
  `create`; if `TestHost` has no such hook, test the claim function with a
  stale `seats` list). Also state the row order in the Rows table.
  *Status:* Open.

- [ ] **P2 — major.** *Where:* D-C3b-2 (the table and the `git grep` check
  below it), D-C3b-15 (the table of files), C3b1 steps 4 and 6.
  *Problem:* The plan's check misses places that list the services one by
  one. The `git grep` pattern
  (`roym_directory\|roym-directory\|DIRECTORY\b`) finds only identifier
  forms. It does not find string lists such as `['…','directory']`, and
  the D-C3b-15 table has no row for them. I ran the pattern and also a
  search for the string forms. These places are missing:
  1. `crates/substrate/tests/e2e/node-boot.ts:32`,
     `const ROYM_SERVICES = ['profile', …, 'directory', 'web']`. Line `:115`
     to `:117` loops over it and runs
     `cargo component build -p syneroym-roym-${svc}`. This is how the
     Playwright suite builds the components. The plan changes only the
     comment at `node-boot.ts:58` ("the six Roym service components").
  2. `crates/substrate/tests/common/roym.rs:81`, `service_visibility`:
     `"web"` internal, `"profile"` private, every other name public. The
     plan gives `scheduling` `visibility = "private"` in `roym.toml`, so
     the shared test harness would deploy it as public. The two-node tests
     would then not match the real deploy.
  3. `crates/substrate/tests/roym_restore_e2e.rs:27` and `:28`
     (`EXPORT_SERVICES`, `IMPORT_ORDER`), and
     `crates/substrate/tests/roym_trust_e2e.rs:340` (`DATA_SERVICES`). The
     plan says "`roym_restore_e2e.rs:33` to `:42`": those lines are the two
     functions, the lists are at `:27` and `:28`.
  4. The parity harness has three more by-name places than the plan lists
     (`helpers.rs:890` to `:897` is the only one it names): the `status`
     arm (`:377` to `:384`), the native-service arm (`:645` to `:651`,
     which `panic!`s on an unknown name) and the wire factory list
     (`:1185` to `:1215`), and the factories at `:1092` to `:1117`.
  *Failure case:* an implementer follows the table. The unit test of
  D-C3b-15 passes. In the 20-minute `mise run verify`, the Playwright
  global setup does not build `syneroym_roym_scheduling.wasm`, the deploy
  fails, and every story fails. Or the parity harness panics with
  "unknown service scheduling". The point of D-C3b-15 is that a missed
  place fails a test early; these are the places it would not catch.
  *Fix:* (1) Change the `git grep` to include string forms, for example
  `git grep -nE "['\"]directory['\"]|roym[_-]directory|DIRECTORY\b"` over
  `crates apps xtask mise.toml .github scripts`, and say that every hit is
  read. (2) Add rows to the D-C3b-15 table: `node-boot.ts`
  (`ROYM_SERVICES` has `'<s>'`), `crates/substrate/tests/common/roym.rs`
  (`service_visibility` names `scheduling` as private; this one needs a
  small check that a private service in `roym.toml` is private there), and
  the three e2e lists (`EXPORT_SERVICES`, `IMPORT_ORDER`, `DATA_SERVICES`).
  (3) List the parity harness's four places by line in D-C3b-2. (4) Fix
  the line numbers named in this finding.
  *Status:* Open.

- [ ] **P3 — major.** *Where:* section 4 ("Behaviour changes", the
  "scheduling backup is restored on its own" bullet), D-C3b-9, section 8
  (the new backlog row), the `roymctl` export order.
  *Problem:* Today the seat rows and the agreements are in one bundle, and
  the import checks one against the other (`sections.rs:498` to `:520`,
  `:600` to `:615`). After the move they are two bundles, exported one
  after the other, and the plan says only "a restored reservation of an
  unknown agreement holds its seat until released (C3b1) or until it
  lapses (C3b2, if never confirmed)". That sentence is not true in C3b1:
  nothing can release it. No agreement names it, `reservation.*` is not
  routed (so the owner has no call), `availability.remove` followed by
  `availability.set` gives the same slot id and the same seat rows, and a
  *confirmed* orphan never lapses in C3b2 either. The other direction is
  worse: if an agreement is in the restored `transaction` bundle and its
  reservation is missing from the restored `scheduling` bundle, the seat is
  free again for an agreed booking, which is a double booking. The plan
  puts `scheduling` before `transaction` in the export order, so the
  window "booked between the two exports" gives exactly this second case.
  *Failure case:* the owner runs `roymctl roym backup create` while a
  customer's booking completes. The archive has the agreement and no seat.
  After a restore, a second customer books the same last seat.
  Or: the archive has a seat and no agreement; the seat is lost for good
  until the owner raises the slot's capacity.
  *Fix:* the owner should decide, so there are two options. (a) A small
  reconcile at the end of `transaction.import`: it asks `scheduling`
  (a local `reservation.list`, not routed to the Hub) for every
  reservation, releases those that no non-ended agreement of the restored
  bundle names as an own-giver commitment, and calls the idempotent
  `reserve` for the ones that are missing (a refusal is reported in the
  import output). About 80 lines and 3 tests. (b) Keep the plan, but state
  both failure cases in section 4 and in the backlog row, drop the false
  "until released" sentence, give the export order a reason (I suggest
  `transaction` first, so a booking in the window leaves an orphan seat,
  which is visible and safe, and not a double booking), and name the
  workaround (raise the capacity). I prefer (a); (b) is acceptable if the
  owner wants the slice smaller.
  *Status:* Open.

- [ ] **P4 — minor.** *Where:* D-C3b-5 ("`confirm` never takes a seat"),
  D-C3b-11 ("A late countersigned half after the counterparty's
  release"), section 7.3, section 9 (roym-agreements Deviations).
  *Problem:* (1) The agreements doc says a counterparty "that released its
  reservations at the expiry and then gets a late countersigned half
  confirms as usual; a refused confirm ends in `reservation-lost`"
  (`agreements:620`). With the owner's rule, that confirm is *always*
  refused, also when the seat is free, because the release came first.
  So a deal that the writer countersigned before the expiry fails with
  `reservation-lost` whenever the half arrives after the counterparty's
  expiry release. The plan describes this in D-C3b-11 and records the
  `confirm` rule in the extensions Deviations, but its list of
  roym-agreements Deviations (section 9) does not include it, so the
  agreements doc would stay false at `:620`. (2) Section 7.3 has no test for
  this path. `a_refused_confirm_signs_one_failure_and_sends_it` uses a
  fake that refuses; none runs "quote expires, counterparty releases, late
  half arrives, failure is signed".
  *Failure case:* the doc says one thing, the code another, and the only
  test of the late-half path is the fake. A later change to the release
  site at quote expiry breaks the path with no test failing.
  *Fix:* add to section 9, roym-agreements Deviations: "a late
  countersigned half after the counterparty's expiry release ends in
  `reservation-lost` (owner answer on `confirm`)", and fix `:620` at
  close-out. Add to 7.3
  `a_late_half_after_the_expiry_release_gives_a_commitment_failure`, on the
  real scheduling semantics if possible (a second small fake that models
  "released, so refuse"), and the matching `agreement.get` call order.
  *Status:* Open.

- [ ] **P5 — minor.** *Where:* D-C3b-11 (`make_failure`), section 7.3.
  *Problem:* `make_failure` signs a record with the person key. Signing can
  fail with `NotEnrolled`, `Expired` or `Stale`
  (`resolve_principal_and_owner`, `app.rs:288` to `:312`). The pseudo-code
  says what to do for a confirm result, a transport error and a send error,
  and nothing for a signing error. A tester cannot tell what the node
  should do, or whether `transaction.sync` reports it.
  *Failure case:* the counterparty's signing certificate expired.
  Every sync calls `confirm` (refused again), tries to sign, fails. If the
  implementer treats it like a `Refused` with no row, it loops silently
  and the writer never learns about the lost seat; if it writes a
  half-made row, a later sync sends an unsigned record.
  *Fix:* state it: a signing error leaves no `failure:` row, is returned
  in the sync output as a named error (the way `send_error` is), and the
  next sync tries again. Add
  `a_failure_that_cannot_be_signed_is_retried_and_reported` to 7.3.
  *Status:* Open.

- [ ] **P6 — minor.** *Where:* D-C3b-6 (`ExtensionHooks` and the
  `ReserveOutcome::Refused { reason }` mapping), D-C3b-7.
  *Problem:* The reason comes from the extension's `data.code` and goes,
  unchecked, into the signed `not-formed` snapshot's `state_reason` and the
  `reserve-failed` error. `progress::validate` checks only that
  `state_reason` is present exactly when the state is `not-formed`
  (`progress.rs:152`), not its form or size. Today the reasons are two
  fixed `&'static str` values (`decide.rs:443`, `:446`). The plan removes
  that bound.
  *Failure case:* a buggy or later extension answers a 5 KB `data.code`.
  The writer signs a snapshot with it, and every counterparty must store
  it.
  *Fix:* `ExtensionHooks` accepts a code only if it is kebab-case ASCII of
  at most 64 bytes; otherwise it returns `Err` (a bug, not a refusal), and
  a unit test covers both sides.
  *Status:* Open.

- [ ] **P7 — minor.** *Where:* D-C3b-6 and D-C3b-7 (`prepare_terms`),
  section 7.3.
  *Problem:* `prepare_terms` has two callers: `quote.set`
  (`quote_ops.rs:190`) and `quote.measure` (`measure_at`, `:254`). The plan
  names only `quote.set`. `quote.measure` will also make the `prepare`
  call, now over a sibling call, and its fixtures use the catalog stub
  that goes away (`tests.rs:139` to `:155`).
  *Failure case:* the hook client's fake does not answer `prepare` in a
  `quote.measure` test, or a `measure` of a quote with an unknown slot
  gives a different error from `set`.
  *Fix:* name both callers, and add
  `quote_measure_refuses_the_same_codes_as_quote_set`.
  *Status:* Open.

- [ ] **P8 — minor.** *Where:* D-C3b-5 (the rows table, "The `seats` query
  uses an index on `slot_id`"), section 7.8.
  *Problem:* The seat rows have no `kind` field, and the `rsv:` rows also
  carry `slot_id`. The pseudo-code queries "`kind = "seat"` and
  `slot_id`", which matches nothing as written. Today's rows have `kind`
  (`hold.rs:64`, `:74`). Also, every release and lapse adds a round row that is
  never removed, and `reserve` reads all of them for the slot.
  *Failure case:* the implementer queries by `slot_id` only, and counts
  `rsv:` rows as seats, so a slot with capacity 2 shows as full after one
  booking.
  *Fix:* give every row a `kind` field (`rsv`, `seat`, `key`, `decided`,
  `free`) in the table, and index `slot_id` and `kind`. In 7.8 say the
  read is bounded by the number of bookings the slot ever had, and name
  what is done if that grows (for example, probing the next round by `get`
  starting from a stored top round).
  *Status:* Open.

- [ ] **P9 — minor.** *Where:* D-C3b-4 (`ReserveInput`), D-C3b-5 (`reserve`:
  "window must equal (slot.start_secs, slot.end_secs)").
  *Problem:* `window` is `Option<TimeWindow>`. The plan does not say what
  `None` means. The check is also new: today's `reserve` does not compare
  the window (`hold.rs:105` to `:132`), and the slot id is derived from the
  window (`derive_slot_id`), so a stored slot can never have another
  window. The check is dead code, and it adds a refusal code
  (`slot-unavailable`) with a confusing cause.
  *Fix:* either drop the check, or make `window` required in
  `ReserveInput` and keep the check with its own code. Say which.
  *Status:* Open.

- [ ] **P10 — minor.** *Where:* section 2.2 point 3 (the counterparty's
  release at quote expiry), D-C3b-7.
  *Problem:* The counterparty's release runs in `agreement.get`
  (`agreement_ops.rs:508` to `:521`) and in `tick_conversation_at`, on
  *every* call after the quote's expiry. Today that is one `get` that
  misses, after the first time. In the new model each call is one sibling
  call per slot commitment, and inside it a `key:` read, a probe of
  `rsv:<g+1>` and a `create` of `free:<r>` that always finds the row. The
  plan also asks for release errors to be logged, not dropped: on every
  read.
  *Failure case:* the Hub polls `agreement.get` for an expired quote every
  few seconds; each poll makes a cross-service call and, when the
  scheduling service is slow or down, a log line.
  *Fix:* release once per agreement: write a local `released:<agreement>`
  marker in `ledger` after the first successful release and skip later
  calls (a lost marker costs one extra idempotent call). Add a test that a
  second `agreement.get` makes no hook call.
  *Status:* Open.

- [ ] **P11 — minor.** *Where:* "How this is shown to an end user" (C3b2),
  section 7.3 (`the_writer_marks_the_commitment_failed_and_cancels_the_others`).
  *Problem:* After `apply_failure`, the check
  `n.state == Agreed && all commitments final` moves the agreement to
  `closed` (`progress.rs:243`; `failed` and `cancelled` are
  final, `transition.rs:47` to `:56`). In the plan's three-commitment test
  every other commitment is `promised` or `payment-requested`, so all
  three become final in one snapshot and the agreement is `closed`. The
  plan's end-user text says "failed" and "cancelled" and does not say that
  the deal is `closed`; the test names do not check the agreement state.
  *Failure case:* a Hub test or the Hub's card shows the deal as ended
  while the end-user text promised nothing about it; a later change to the
  closing rule changes the outcome with no test failing.
  *Fix:* state it in the end-user section, and assert
  `state == closed` in the test. Say what the Hub shows (it already draws
  `closed`).
  *Status:* Open.

- [ ] **P12 — minor.** *Where:* section 9 (living docs), D-C3b-2.
  *Problem:* The living-docs list misses
  `docs/system-architecture.md:559` ("The six Roym services use this to be
  local-only"). It also misses the comment at
  `crates/roym_web/ui/src/screens/backup.ts:5` ("The five app-data
  bundles"), which the plan changes to six bundles. D-C3b-2 names the
  parity list at `helpers.rs:893`; the list is at `:890` to `:897`
  (D-C3b-15 has it right).
  *Fix:* add the two places to section 9 and fix the line number.
  *Status:* Open.
