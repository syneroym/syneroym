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

- [x] **P1 — major.** *Where:* D-C3b-5, the `reserve` pseudo-code
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
  *Status:* Fixed (eacbaa91). As proposed: `rsv:` first in the `create` list (stated in the Rows table), `current(key)` read again after any `Some(id)`, and `two_reserves_of_one_key_return_one_reservation` in 7.2.

- [x] **P2 — major.** *Where:* D-C3b-2 (the table and the `git grep` check
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
  *Status:* Fixed (eacbaa91). As proposed: the search finds string forms and every hit is read; D-C3b-2 lists `node-boot.ts`, `service_visibility`, the three e2e lists, the five parity-harness places, `test_proxies.rs`' dispatch and `expected_schema_version` (`fixtures.rs:131`, which this finding did not name: `transaction` 7, `catalog` 5); D-C3b-15 has rows for them. The signing-only lists are named as not changed. Line numbers fixed.

- [x] **P3 — major.** *Where:* section 4 ("Behaviour changes", the
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
  *Status:* Fixed (eacbaa91), with a narrower fix than (a). New D-C3b-16: `transaction.import` reserves again (and confirms, when countersigned) every own seat of a restored agreement that is not ended, and lists refusals in its output; export order `transaction` then `scheduling`, import order `scheduling` then `transaction`; a wrong Hub order is reported, with the fix (import `transaction` again). `scheduling.import` refuses an `rsv:` row that exists with other content. Rejected part: releasing reservations that no agreement names (the `reservation.list` hook). That case is safe (one seat fewer, no double booking), needs one more hook, and a second `transaction` import would bring the agreement back after its seat was freed. It is a backlog row; the false "until released" sentence is gone.

- [x] **P4 — minor.** *Where:* D-C3b-5 ("`confirm` never takes a seat"),
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
  *Status:* Fixed (eacbaa91). As proposed: the roym-agreements Deviations line in section 9 (and `:620` corrected at close-out), the deviation stated in D-C3b-11, and `a_late_half_after_the_expiry_release_gives_a_commitment_failure` in 7.3 with a fake that refuses `confirm` for a released key.

- [x] **P5 — minor.** *Where:* D-C3b-11 (`make_failure`), section 7.3.
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
  *Status:* Fixed (eacbaa91). A signing error writes no row, and the next sync tries again. One correction: `SyncOutput` has no `send_error` field today (`sync.rs:46` to `:54`), so C3b2 adds `unsent_failures: u32` for unsigned and unsent failures. Test `a_failure_that_cannot_be_signed_is_retried_and_reported`.

- [x] **P6 — minor.** *Where:* D-C3b-6 (`ExtensionHooks` and the
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
  *Status:* Fixed (eacbaa91). As proposed, with the code form `[a-z0-9-]{1,64}`; test `an_extension_reason_that_is_not_a_code_is_an_error`.

- [x] **P7 — minor.** *Where:* D-C3b-6 and D-C3b-7 (`prepare_terms`),
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
  *Status:* Fixed (eacbaa91). As proposed: both callers named in D-C3b-6 and D-C3b-7; test `quote_measure_refuses_the_same_codes_as_quote_set`.

- [x] **P8 — minor.** *Where:* D-C3b-5 (the rows table, "The `seats` query
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
  *Status:* Fixed (eacbaa91). As proposed: `kind` on every row, `kind` and `slot_id` indexed, the bound and the later change stated in D-C3b-5; test `a_seat_query_counts_only_seat_rows`.

- [x] **P9 — minor.** *Where:* D-C3b-4 (`ReserveInput`), D-C3b-5 (`reserve`:
  "window must equal (slot.start_secs, slot.end_secs)").
  *Problem:* `window` is `Option<TimeWindow>`. The plan does not say what
  `None` means. The check is also new: today's `reserve` does not compare
  the window (`hold.rs:105` to `:132`), and the slot id is derived from the
  window (`derive_slot_id`), so a stored slot can never have another
  window. The check is dead code, and it adds a refusal code
  (`slot-unavailable`) with a confusing cause.
  *Fix:* either drop the check, or make `window` required in
  `ReserveInput` and keep the check with its own code. Say which.
  *Status:* Fixed (eacbaa91). The check is dropped, and `ReserveInput` has no `window` (the slot id already names its window).

- [x] **P10 — minor.** *Where:* section 2.2 point 3 (the counterparty's
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
  *Status:* Fixed (eacbaa91). As proposed: a `released:<agreement>` marker in `ledger` (`LedgerKind::Released`, checked on import); test `the_expiry_release_runs_once_per_agreement`.

- [x] **P11 — minor.** *Where:* "How this is shown to an end user" (C3b2),
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
  *Status:* Fixed (eacbaa91). The end-user section says the agreement shows "Closed" when no part was paid or delivered, and the test asserts `closed`.

- [x] **P12 — minor.** *Where:* section 9 (living docs), D-C3b-2.
  *Problem:* The living-docs list misses
  `docs/system-architecture.md:559` ("The six Roym services use this to be
  local-only"). It also misses the comment at
  `crates/roym_web/ui/src/screens/backup.ts:5` ("The five app-data
  bundles"), which the plan changes to six bundles. D-C3b-2 names the
  parity list at `helpers.rs:893`; the list is at `:890` to `:897`
  (D-C3b-15 has it right).
  *Fix:* add the two places to section 9 and fix the line number.
  *Status:* Fixed (eacbaa91). `system-architecture.md:559` is in section 9; the `backup.ts:5` comment is in D-C3b-2; the parity list line is fixed.

### Round 1 close-out — Claude Opus 5.5 (Claude Code), 2026-11-21

The plan author folded round 1 into the plan in `eacbaa91`. All 12 items
are closed: 11 fixed as proposed (P5 with one fact corrected), and P3
fixed with a narrower option than the reviewer's (a). The rejected part
of P3 and its reason are in P3's status and in the plan's D-C3b-16. The
plan's answers to the owner questions did not change.

**Open after round 1: 0.**

## Code review round 1 — Claude Opus 5.5 (Claude Code), 2026-11-21

Reviewed part C3b1, PR [#378](https://github.com/syneroym/syneroym/pull/378)
(draft), branch `feat/m08c-c3b-scheduling` at `1a149df7`, with
`git diff origin/main...HEAD` (139 files). This is the main checkout
(`git rev-parse --git-dir` is `.git`). `git pull` said "Already up to
date". The code was written by Claude Sonnet 5.5, so this round meets the
cross-tool rule. There is no earlier code review round of C3b, so there
are no Fixed or Rejected items to check again.

Read: AGENTS.md, session-strategy.md ("The review file"), the plan (all
1,847 lines), this file's plan review round, the code review rounds of
[slice-c3a-review.md](slice-c3a-review.md) (for the format and the kinds
of findings before), and the three change docs (roym-extensions,
roym-agreements, roym-value-exchange), with their C3b1 Deviations.

CI (`gh pr checks 378`, read twice during the review): "Lints, licenses &
checks", CLA and test shards 1, 2, 3 and 5 pass. **Test 4/6 and
Playwright E2E fail** (R1). Shard 6 was still running. I did not run the
full suite or `mise run verify`. I ran `cargo nextest run -p
syneroym-roym-scheduling` (43 passed, 0 skipped, the count of the status
row), and one temporary test to prove R2, which I removed afterwards (the
working tree is clean). I changed no code.

**Summary: 9 findings (R1 to R9): 1 blocker, 1 major, 7 minor.**

### What was checked and found correct

- **The reservation model (D-C3b-5).** One `reservations` collection.
  `rsv:` and `seat:` are created by one `create`, `rsv:` first
  (`claim_rows`, tested by
  `the_claim_lists_the_reservation_row_before_its_seat_row`). After any
  `Some(id)`, `claim` reads `current(key)` again and never decides by the
  id. A seat is held by round, and no seat row is deleted. `release`
  writes a `free:` fence, `confirm` writes a `decided:` fence and never
  creates a seat row; a confirm after a release is refused
  `reservation-lost`. `current` follows a lost `key:` pointer forward. I
  traced a confirm racing a release: it ends confirmed and freed, which
  is the same state as "release after confirm", so it is safe.
  `MAX_ATTEMPTS` (not in the plan) only bounds the `Claim::Again` loop.
  `availability.rs` moved with only its import line changed.
- **The hook client (D-C3b-6, D-C3b-7).** `needs_reservation` and
  `reserved_by` go through the registry; `hook_service` comes from the
  manifest. `refusal_reason` accepts only `[a-z0-9-]{1,64}` and turns any
  other answer into `Err`. Every one of the nine seat sites of section
  2.2 point 3 now calls the hooks. The writer's step 3 and step 4 keep
  their order; a confirm *refusal* still ends `not-formed`, and a confirm
  *fault* is now an `Err` that is tried again (the plan's rule). The
  `released:<agreement>` marker is written only when every release
  worked, and the import accepts it only for a held agreement.
- **Exit criteria greps.** `ServiceInSlot|slot_id|availability\.|MAX_SLOT_CAPACITY`
  over `roym_transaction/src` and `roym_catalog/src`, tests left out,
  finds only `backup/sections.rs:376`. `transaction` names neither
  `SCHEDULING` nor `"scheduling"`, and has no call to `CATALOG`.
- **Wiring (D-C3b-2).** `services.rs`, `router.rs` (`availability.` and
  `scheduling.` to `SCHEDULING`, no `reservation.` row), `roym.toml`
  (private, health check, `transaction` → `conversation`, `scheduling`;
  `web` gets `scheduling`), the native factory, the owner loop and the
  topology bindings in `substrate/src/runtime/roym.rs`, both signing and
  backup orders. The service-list test reads 15 places; with the owner's
  signing decision it also checks the three signing lists.
- **Access control.** Every `scheduling` method goes through
  `admit::require_internal`; `reservation.*` has no route, and
  `WIRE_REFUSED_VERBS` has a `scheduling` verb, so scenarios 67 and 68
  cover the new service on both builds.
- **Both builds.** Parity scenarios 247 to 250 run the hooks, the export
  and the import on WASM and native and compare them.
- **Backup (D-C3b-9, D-C3b-16).** Every rule of D-C3b-9 has a check in
  `backup/check.rs`, and `seat:` rows are also refused when they changed
  (a recorded deviation). `transaction.import` reserves again after its
  rows are written, and lists failures in `reservations_failed` instead
  of failing. `SCHEMA_VERSION` is 7 for `transaction` and 5 for
  `catalog`, each with its sentence.
- **Change docs.** The C3b1 Deviations are in the doc that owns each
  design: manifest keys, `confirm`, EX-5, EX-21, no expiry, no delete,
  signing, unrouted hooks, no log line and step order in roym-extensions;
  AG-14/AG-15, the restore, the late half and `released:` in
  roym-agreements. Their numbers and meaning match the plan's.
- **Living docs and backlog.** AGENTS.md, the architecture doc, the
  developer guide, the requirements, the integrated spec, TERMINOLOGY
  and demo.md say seven services and describe the hooks without
  over-claims. The four new backlog rows exist.
- **Sizes and rules.** New files are under 800 lines; `hold.rs` is gone;
  no planning ids in comments; the lints gate is green.

### New findings

- [ ] **R1 — blocker.** *Where:* `crates/substrate/src/runtime/tests.rs:110`;
  `crates/substrate/tests/e2e/tests/roym-hub.spec.ts:308`, `:314`.
  *Problem:* CI is red on `1a149df7`. Two tests count the Roym services
  and were not updated:
  `every_production_factory_in_runtime_is_fully_wired` expects 7
  `NativeHostFactory::new` sites and finds 8 (Test 4/6: "expected exactly
  7 NativeHostFactory::new sites in runtime, found 8"). Hub test 8,
  "shows the five service bundles", expects 5 `.bundle-row` elements and
  the screen now has 6 (Playwright E2E: 1 failed after 2 retries, 74
  passed). Neither place is in D-C3b-2's table or in the service-list
  test, because they hold a count and not a list of names. The status
  row says the Playwright stories and the full verify were left to CI.
  *Failure case:* the PR cannot pass `mise run verify`, so the exit
  criteria "Stories … and the Hub cases pass" and "`mise run verify`
  passes" fail.
  *Fix:* set 8 in `runtime/tests.rs` and 6 in `roym-hub.spec.ts` (rename
  the test to "six service bundles"). Run Test 4/6's failing test and the
  Hub spec locally, then let CI run again.
  *Status:* Open.

- [ ] **R2 — major.** *Where:* `crates/roym_scheduling/src/app/backup/check.rs:85`
  to `:101` (`reservation_rows`), `crates/roym_scheduling/src/app/backup.rs:83`
  to `:92`.
  *Problem:* `scheduling.import` writes all `rsv:` rows, then all `seat:`
  rows, then the pointers and the `decided:`/`free:` rows, in
  `batch_mutate` chunks of 100. Each chunk is written alone. If the
  import stops between chunks (a crash, or a storage error on a later
  chunk), some `rsv:` rows exist without their `seat:` rows. Such a
  reservation is live for its key (`current` finds it, no `free:` row),
  but no seat row shows it, so `read_seats` sees its seat as free. The
  comment says the order is chosen "so a stopped import never leaves a
  seat with no reservation", but a seat with no reservation is the safe
  case (the seat stays taken); a reservation with no seat is the unsafe
  one.
  *Failure case:* proved with a temporary test (removed). A slot of
  capacity 60 with 60 reservations is exported. The import into a fresh
  host is stopped on the 41st `seat:` row (`crash_on_write_skip("reservations",
  "seat:", 40)`): 60 `rsv:` rows and 40 `seat:` rows are stored. Then a
  new key's `reserve` answers `Held` on the full slot, and the repeat of
  `agr-59` (whose seat row is missing) also answers `Held`: a double
  booking. A second import of the same bundle then cannot repair it:
  `refuse_changed_rows` finds the new booking's `seat:` row where the
  bundle has another one, and refuses the whole import
  `reservation-differs`. `transaction.import` run in between also
  answers `Held` for such a commitment (D-C3b-16's restore trusts it).
  *Fix:* write the `seat:` rows before the `rsv:` rows (then `key:`,
  then `decided:`/`free:`; a `key:` row must still come after its `rsv:`
  row, because `current` treats a pointer to a missing `rsv:` row as
  corrupt). Correct the comment to name the safe direction. Add a test in
  `tests/backup.rs` with `crash_on_write_skip` that stops the import
  after the first chunk and checks that the full slot still answers
  `slot-taken`.
  *Status:* Open.

- [ ] **R3 — minor.** *Where:* `crates/roym_transaction/src/app/agreement_ops.rs:259`
  to `:285`, `crates/roym_transaction/src/app/hooks.rs:195` to `:209`.
  *Problem:* On `main`, `reserve_counterparty_slots` released the seats it
  had already held when a later reserve returned an error. Now
  `reserve_each` returns `Err` through `?` and drops the list of held
  commitments, and `accept` answers `internal_error` with no release.
  This behaviour change is not in the plan or in a Deviations section.
  *Failure case:* a quote has two slot commitments that the counterparty
  gives. The first reserve is held, the second gets a transport error.
  `agreement.accept` fails and signs nothing, but the first seat stays
  held. In C3b1 only the release at the quote's expiry frees it, and
  only once someone calls `agreement.get` or the sync tick after the
  expiry.
  *Fix:* let `reserve_each` give back the held commitments together with
  a fault (for example a struct with `held`, `refusal` and `fault`), and
  release them in `accept` for a refusal and for a fault, as `main` did.
  Add a test with the fake's `fail_nth` on the second `reservation.reserve`.
  *Status:* Open.

- [ ] **R4 — minor.** *Where:* `crates/roym_scheduling/src/app/backup/check.rs:169`
  to `:188` (`refuse_changed_rows`).
  *Problem:* D-C3b-5 says `decided:` and `free:` rows are created once and
  never overwritten. The import refuses a changed `rsv:` or `seat:` row,
  but writes `decided:` and `free:` rows with `put`, so a bundle can
  replace a stored one. `Decision::Lapsed` already parses in C3b1.
  *Failure case:* the store has `decided:<r> {confirmed}`; a bundle with
  `decided:<r> {lapsed}` replaces it. A later `confirm` then answers
  `reservation-lost` for a confirmed seat. In C3b2 that refusal makes the
  counterparty sign a `commitment-failure`.
  *Fix:* check every row except `key:` (the one row that is overwritten
  by design) in `refuse_changed_rows`, and add one refused case for a
  changed `decided:` row to `tests/backup.rs`.
  *Status:* Open.

- [ ] **R5 — minor.** *Where:* `crates/roym_core/app/roym.toml:78` to `:81`.
  *Problem:* The comment on `transaction`'s `depends_on` says a quote
  that names a slot "reads that slot from the scheduling service -- at
  quote time to bind it, and at acceptance time to read its capacity".
  `transaction` reads no slot and no capacity any more. It calls the
  reserve hooks of the service that a commitment's extension manifest
  names, and the edge exists because the scheduling manifest names
  `scheduling`. D-C3b-2 asked for a comment that says this.
  *Failure case:* a reader thinks `transaction` still holds slot code,
  which EX-17 removed; the next extension author does not learn that its
  hook service must be added to this edge.
  *Fix:* say that `transaction` calls the reserve hooks of each extension
  service that a manifest names (today `scheduling`), so every such
  service must be in this list.
  *Status:* Open.

- [ ] **R6 — minor.** *Where:* `crates/roym_core/src/extension/load.rs:92`
  (`check_hook_service`); `crates/roym_core/app/roym.toml:81`;
  `crates/substrate/src/runtime/roym.rs` (`wire_roym_topology`).
  *Problem:* The loader accepts any service of `services::ALL` but `web`
  as a hook service. The hook call is a sibling call from `transaction`,
  which works only for a service in `transaction`'s `depends_on` and its
  native binding. Nothing checks that the two agree.
  *Failure case:* a later manifest names `"service": "catalog"`. It
  loads, and every hook call fails with a transport error: `accept`
  answers `internal_error`, and the writer's decision is retried for
  ever. No test fails.
  *Fix:* a test in `roym_core` (next to the `roym.toml` tests in
  `router.rs`) that the `hook_service` of every manifest in the registry
  is in `transaction`'s `depends_on`. The native binding is covered when
  the D-C3b-15 test also reads the `save_binding` calls for
  `TRANSACTION` in `runtime/roym.rs`, or by a comment there that points
  to the manifest test.
  *Status:* Open.

- [ ] **R7 — minor.** *Where:* `docs/planning/changes/roym-value-exchange/change.md:733`
  (`## Deviations`).
  *Problem:* The plan's section 9 asks for a C3b1 line in this doc: VX-10
  is met through the scheduling extension's hook, with no text change.
  The branch changes only the extensions and agreements docs.
  *Failure case:* the value-exchange doc does not say where VX-10 was
  met, so its close-out has to find it again.
  *Fix:* one line under `## Deviations` (or a note in the VX-10 row's
  last column): the giver's node reserves through `reservation.reserve`
  of the scheduling extension, also when it is not the quote author;
  proved by `roym_booking_e2e` and `tests/accept.rs`.
  *Status:* Open.

- [ ] **R8 — minor.** *Where:* `docs/planning/milestones/M08C-deal-core-and-extensions/status.md:18`
  (the C3b1 row, risk measure 2).
  *Problem:* The row says "The service-list test (14 tests,
  `services/tests.rs`) passed on today's six services in c49eb567". At
  `c49eb567` the file has 13 `#[test]` functions; at `1a149df7` it has
  15.
  *Failure case:* the evidence does not match the code, so a reader
  cannot trust the other counts of the row.
  *Fix:* say 13 tests at `c49eb567`, and 15 now.
  *Status:* Open.

- [ ] **R9 — minor.** *Where:* `crates/roym_transaction/src/app/backup/reservations.rs:63`
  to `:96`; `crates/roym_transaction/src/app/tests/hooks_fake.rs:80`.
  *Problem:* `open_commitments` walks `reserved_by` and collects ids, and
  `reserve_again` walks `reserved_by` again and keeps the commitments
  whose id is in that list (`Vec::contains` in a loop). One walk that
  returns `&Commitment` does the same. The fake's doc comment says
  `install` registers "the four hooks, and `availability.get`", but it
  registers only the four hooks.
  *Failure case:* none at run time; the code is longer than it needs to
  be, and the comment is false.
  *Fix:* let `open_commitments` return `Vec<&Commitment>` and loop over
  it; drop "and `availability.get`" from the comment.
  *Status:* Open.

**Open after code review round 1: 9** (1 blocker, 1 major, 7 minor).
