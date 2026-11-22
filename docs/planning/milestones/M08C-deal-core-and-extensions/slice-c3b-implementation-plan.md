# M08C slice C3b — Scheduling extension and reserve hooks: implementation plan

*Reader: the developer (or AI) who implements slice C3b, and the project
owner, who answers the questions in section 2.1. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan written, 2026-11-21, by Claude Opus 5.5. Plan review round
1 (Claude Sonnet 5.5, [slice-c3b-review.md](slice-c3b-review.md), P1 to
P12) is folded in: P1, P2, P4 to P12 as proposed, and P3 with a narrower
fix (D-C3b-16). The owner answered Q1 to Q8 on 2026-11-21 (section 2.1) and asked for
three risk measures: a test that every list of services is complete
(D-C3b-15), a size check of the new component before any wiring (C3b1
step 3), and a `confirm` that never takes a seat again (D-C3b-5). No
question is open.**

**Size and PRs.** [task.md](task.md) sizes C3b as L. Checked against the
code, it is L: a seventh Roym service with its whole wiring (WIT world,
native build, substrate registration, manifest, build task, router,
backup), the move of `availability.*` out of `catalog`, a new reservation
model with fences, the move of every seat call in `transaction` behind a
generic hook client, and then expiry, confirm, a new signed record and its
card. This plan splits it in two parts. Each part is one PR that passes
`mise run verify` (Q1):

- **C3b1 — The scheduling service (M).** The new service
  `syneroym-roym-scheduling` with `availability.*` (moved from `catalog`)
  and the hooks `reservation.reserve`, `.confirm`, `.release` and
  `.prepare`. `transaction` calls the hooks of whatever extension a
  commitment names, and holds no slot code. A reservation never expires in
  C3b1, as a seat hold never expires today. Branch:
  `feat/m08c-c3b-scheduling` (this branch; it holds the plan commit).
- **C3b2 — Expiry and commitment failure (M).** A reservation expires
  after 15 minutes unless it is confirmed. The counterparty's node
  confirms its own reservations when it files the writer's countersigned
  half. A refused confirm gives a signed `commitment-failure` record and
  card. The writer marks the commitment `failed` and cancels the others in
  the same snapshot. Each giver releases the reservations of its failed and
  cancelled commitments. Branch: `feat/m08c-c3b2-reservation-expiry`, cut
  from `main` after C3b1 merges.

**Base.** Every file path and line number below was checked on `main` at
`f91e7c32` (2026-11-21). Links are relative to this folder.

**Dependencies.** [task.md](task.md) section 4: C3b comes after C3a.
[status.md](status.md) says "Merged" for C3a1 (`629f8f28`, PR #374) and
C3a2 (`fa8ac577`, PR #376). C3c depends on C3b, because `instant` needs a
slot that the scheduling extension can reserve. C3b does not touch
`conversation`, the forwarding change doc or `docs/decisions/`, so C7 may
run beside it ([task.md](task.md) section 4). The Engage fix is merged.

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish each PR (background job, about 20
    minutes, no `--skip` in the final run).
  - No planning ids in code, comments, test names or `.wit`, `.toml`,
    `.json`, `.ftl` files: no "M08C", "C3b1", "EX-20", "AG-16",
    "D-C3b-4", "Q3" or review ids.
  - Comments never cite size rules.
  - **New crate naming.** `crates/roym_scheduling/`, package
    `syneroym-roym-scheduling`.
  - **File size.** Production `.rs` files at most 800 lines; new test `.rs`
    files at most 800 lines. Section 7.8 lists the files near the cap.
    Put new code in new files.
  - **Function size.** `clippy::too_many_lines` at 100. Add no suppression.
  - **Module layout.** `foo.rs` beside `foo/`, never `mod.rs` in `src/`.
  - **Ports.** The two-node tests use `common::alloc_ports` and
    `SubstrateTestContext`, never a literal.
  - The import clean-up pass, the deferred backlog, the living-docs
    close-out, the demo close-out and the ratchet updates.
  - Every `roym_*` crate change needs `mise run build:roym` before you
    trust a parity run ([task.md](task.md) section 1). C3b1 adds a seventh
    component to that build.
  - Do not commit on `main`.
- [task.md](task.md): section 1 (Z5 and the component size limit: "C3b
  adds a seventh component (scheduling); it reports its size alone"),
  "C3b", section 4, section 7.
- [roym-extensions](../../changes/roym-extensions/change.md): EX-4 to EX-6,
  EX-14, EX-17, EX-20 to EX-22, "Reserve and release hooks", "Scheduling
  and one-shot", "Extension services and the deal grant", "Deviations".
- [roym-agreements](../../changes/roym-agreements/change.md): AG-14 to
  AG-16, AG-36 (`:129` to `:132`); "Commitment failure" (`:528`); "Reason
  codes" (`:624`); "Commitments" (`:656`, the `failure` and `other-failed`
  events); "The writer's decision" (`:724`); "Reserve, confirm and
  release: the agreement side" and "Who reserves" (`:757` to `:772`);
  "Methods" (`:974`, the last line); "Card types" (`:1009`); "Rejected
  options" (`:1083`, `:1091`); open questions 4 and 5 (`:1115`, `:1120`);
  "Deviations" (`:1233` to `:1245`, the C1 notes on AG-14, AG-15, AG-36).
- [roym-value-exchange](../../changes/roym-value-exchange/change.md):
  VX-10 (`:150`), "The resource owner reserves its own resource" (`:337`).
- The C3a plan ([slice-c3a-implementation-plan.md](slice-c3a-implementation-plan.md))
  for the layout, the registry (D-C3a-4, D-C3a-5) and the list of seat
  sites it left for C3b (its section 4, "Stays the same").
- The C1, C2 and C3a review files. Section 7.11 says how this plan avoids
  the kinds of findings they hold.

### Terms used in this plan

- **Roym extension:** a package that adds work steps or data to a deal. It
  never changes the deal core's states (extensions doc, "The rule").
- **Extension service:** the service part of an extension. It is one more
  service in `roym.toml` (EX-14). The scheduling extension is the first one.
- **Hook:** a local method of an extension service that the deal core
  calls at a fixed point: `prepare` (before the quote is signed),
  `reserve`, `confirm` and `release`.
- **Reservation:** a hold on one resource (here: one seat in one slot),
  taken by the node of the party that gives the resource, for one
  (agreement, commitment) pair. **Confirmed:** the agreement formed and
  the giver's node said so. **Released:** freed by the giver's node.
  **Lapsed (C3b2):** not confirmed within 15 minutes, and then taken over
  by another booking.
- **Fence:** a unique insert (`AppDataLayer::create`): of two calls that
  race, only one creates the row
  ([data-layer.wit:186](../../../../crates/wit_interfaces/wit/data-layer/data-layer.wit#L186)
  to `:201`).
- **Writer:** the quote author's `transaction` service, the only writer of
  the agreement state (agreements doc, "Single writer").
- **Giver:** the party that gives the resource of one commitment. It owns
  that commitment's reservation (agreements doc, "Who reserves").

## 1. What C3b builds, in one paragraph

Today a slot booking works like this. `catalog` stores slots
(`availability.*`,
[api.rs:36](../../../../crates/roym_catalog/src/api.rs#L36) to `:39`,
[availability.rs](../../../../crates/roym_catalog/src/app/availability.rs),
152 lines). `transaction` reads a slot from `catalog` when it signs a
quote ([quote_ops.rs:419](../../../../crates/roym_transaction/src/app/quote_ops.rs#L419)
to `:470`), and again when it takes a seat
([hold.rs:105](../../../../crates/roym_transaction/src/app/hold.rs#L105)).
The seat is two rows in `transaction`'s own `ledger`: `hold:<agreement>:<commitment>`
and `seat:<slot>:<n>`, created together
([hold.rs:86](../../../../crates/roym_transaction/src/app/hold.rs#L86) to
`:169`). A hold never expires, and `confirm` does nothing (`:171` to
`:178`). Nine places in `transaction` ask "is this a slot commitment?" by
matching `ResourceKind::ServiceInSlot` (section 2.2, point 3). C3b1 moves
all of this into a new Roym service, `scheduling`. `transaction` then asks
the registry whether a commitment's extension has a reserve hook, and calls
that extension's service. C3b2 adds the 15-minute expiry, the counterparty's
confirm, and the signed `commitment-failure` with the cascade that cancels
the rest of the agreement.

### Why C3b is split (Q1)

- C3b1 is a move across a service boundary. Its reviewer's question is
  "does booking still work exactly as before, through the new service, and
  is every seat call moved". Nothing a person sees changes.
- C3b2 changes behaviour: a reservation can now be lost, and a new signed
  record travels between nodes. Its reviewer's question is "is a lost
  reservation always ended with a named failure, made once, and is every
  seat freed".
- The cut is safe in this order and not the other. If expiry came without
  the counterparty's confirm, a counterparty's seat could lapse after the
  agreement formed, with no failure record: a double booking with no trace.
  So C3b1 keeps today's "never expires", and C3b2 brings expiry and
  confirm together.
- Sizes: C3b1 is about 1,000 lines of production code (the new crate about
  700, of which 152 move from `catalog`; `transaction` loses `hold.rs`, 196
  lines, and gains a hook client of about 200) and 1,000 of tests (most
  move or change). C3b2 is about 600 and 900. Each is about the size of
  C3a1.

## How this is shown to an end user

C3b1:

- A shop owner adds slots to a listing in the Hub, as today. Nothing on
  the screen changes.
- A person books a slot, and the shop owner's node agrees, as today. If
  two people take the last seat at the same time, one gets the booking and
  the other sees "Another booking took this slot first", as today.
- On the backup screen there is one more file to download and to restore:
  "Scheduling" (the slots and the seats taken).
- So an end user sees one new backup line and nothing else.

C3b2:

- In the common booking, nothing changes: the shop owner's node takes and
  confirms the seat at once.
- In the rare case where the person who gives a slot is the one who
  accepts (for example, a tutor accepts a student's offer for a lesson),
  and the other side does not answer for more than 15 minutes, the tutor's
  seat can go to another booking. Then both people see a new card in the
  chat: "This part of the deal cannot go ahead: the slot was given to
  someone else." The agreement card shows that part as "Failed" and the
  other parts as "Cancelled". When no part has been paid or delivered yet,
  every part is now ended, so the card also shows the whole agreement as
  "Closed". A part that was already paid or delivered keeps its own path,
  and the agreement stays open until that part ends.
- The demo is the owner's choice (Q6). The plan recommends no new story,
  because this case needs a wait of 15 minutes.

How a developer can see it work:

- `cargo nextest run -p syneroym-roym-scheduling` runs the reservation
  tests: two reserves for the last seat (one wins), repeat reserves,
  release, confirm, and (C3b2) a lapse and a takeover with a set clock.
- `cargo nextest run -p syneroym-substrate --test roym_booking_e2e` runs
  today's race for the last seat on three nodes, now through the
  scheduling service: one agreement `agreed`, the other `not-formed` with
  `slot-taken`.
- `cargo nextest run -p syneroym-roym-transaction commitment_failure`
  (C3b2) runs the counterparty's refused confirm: the node signs one
  `commitment-failure`, and the writer's next snapshot has `failed` and
  `cancelled` with their reasons.

## 2. Points for the owner

### 2.1 Questions

Each question has options and a recommendation. The plan is written for
the recommended answer.

**Owner answers, 2026-11-21.** Q1 (a) to Q8 (a): the recommendation of each.
For Q6 this is the owner's explicit yes to `- none` for both parts. In the
same answer the owner chose three risk measures from the planning talk:

- **A test that every list of services is complete** (D-C3b-15), so a
  missed place fails a test, not a review.
- **The size of the new component is measured before it is wired in**
  (C3b1 step 3), so a size problem stops the slice early.
- **`confirm` never takes a seat again** (D-C3b-5). A confirm finds the
  reservation that exists and confirms it, or refuses `reservation-lost`.
  After a release (for example the counterparty's release at the quote's
  expiry, followed by a late countersigned half), the confirm is refused.
  This is narrower than EX-22's "a confirm after the expiry succeeds if
  the resource is still free": it still succeeds after the expiry while
  nobody took the seat (Q4), but not after a release. The extensions doc's
  Deviations record it.

#### Q1. Split C3b into C3b1 (the service) and C3b2 (expiry and failure)?

- **(a) Split, as above.** Two PRs, C3b1 first. Recommended: a move and a
  behaviour change have different risks, and the order keeps every PR
  safe (section 1, "Why C3b is split").
- (b) One PR, about 1,600 lines of production code and 1,900 of tests,
  across `roym_core`, three service crates, `substrate`, `roymctl` and the
  Hub.

If (b): sections 6.1 and 6.2 run on one branch, and the exit criteria are
joined.

#### Q2. How does `transaction` find the service of an extension's hooks?

The manifest says which hooks an extension has (`"hooks": ["reserve",
"confirm", "release"]`,
[scheduling.json](../../../../crates/roym_core/extensions/scheduling.json)).
It does not say which service answers them.

- **(a) A new manifest key `service`, the name of a Roym service.** It is
  required when `hooks` is not empty, and absent otherwise. The loader
  checks that the name is in `services::ALL`. `transaction` calls
  `reservation.<hook>` on that service. Recommended: the deal core then
  names no extension in code, which is what EX-17 and the extensions doc's
  rule 1 ask. A later extension with a reserve hook needs a manifest and a
  service, and no change in `transaction`.
- (b) `transaction` calls `services::SCHEDULING` for every reserve hook.
  Less code (about 20 lines), but the deal core names one extension.

#### Q3. Who checks the slot when the quote is signed?

Today `quote.set` reads the slot from `catalog` for each slot commitment
the author gives. It refuses `no-such-slot`, `slot-not-in-listing` and
`schedule-differs-from-slot`, and it fills the commitment's `window` from
the slot when the input has none
([quote_ops.rs:144](../../../../crates/roym_transaction/src/app/quote_ops.rs#L144)
to `:156`, `:419` to `:470`). `roymctl roym transaction quote --slot`
relies on the fill: `--schedule` is optional
([transaction.rs:356](../../../../apps/roymctl/src/commands/roym/transaction.rs#L356)
to `:358`). This is slot code in `transaction`, so EX-17 moves it.

- **(a) A fourth hook, `prepare`, optional in the manifest.** `quote.set`
  calls it for each commitment the author gives whose extension declares
  it. Input: the commitment's `details` and `window`. Output: the window to
  sign, or a refusal with the same three codes. Recommended: today's
  errors and `roymctl`'s behaviour stay, and `transaction` holds no slot
  code. Cost: about 60 lines and one more sibling call when a quote has a
  slot.
- (b) No quote-time check. The caller must give the window. The schema
  check `slot-matches-window` already proves that the slot id belongs to
  that window and listing (`listing::derive_slot_id`). A slot that does not
  exist then shows only at the decision, as `not-formed` with
  `slot-unavailable`, after the counterparty has accepted.

#### Q4. How does a reservation expire? (C3b2)

There is no timer in a Roym service, and the data layer has no
compare-and-delete (section 2.2, point 6).

- **(a) Lazy.** A reservation carries `expires_at_secs` (15 minutes after
  it is taken). An unconfirmed reservation past its expiry is not freed at
  once. It counts as free when another reserve needs the seat: that
  reserve ends the old reservation (`lapsed`) under a fence, and takes the
  seat. A confirm succeeds while nobody has taken the seat, also after the
  expiry. Recommended: it needs no timer and no delete, and it is exactly
  EX-22's rule ("a confirm after the expiry succeeds if the resource is
  still free"). EX-20's "its node releases the reservation on its own" is
  met in effect: no other node is needed, and the seat is free for every
  new booking.
- (b) Eager. Every call to the scheduling service first ends every expired
  reservation. More writes on every call, the same fences, and still no
  call when nobody uses the service.

#### Q5. The two-node test of AG-16 needs a controllable clock

AG-16's test is "Two-node with a controllable clock". A substrate node's
services read `clock::now_secs()`, which nothing outside the component can
pin ([clock.rs:1](../../../../crates/roym_core/src/clock.rs#L1) to `:12`).
The shortest quote lifetime is 300 seconds
(`MIN_QUOTE_LIFETIME_SECS`,
[transaction.rs:77](../../../../crates/roym_core/src/transaction.rs#L77)),
and the reservation expiry is 900 seconds.

- **(a) Prove AG-16 in three parts, and record a deviation.** (1) The
  scheduling service's tests run reserve, lapse, takeover and the refused
  confirm with a set clock (the `_at(now)` pattern of every Roym
  service). (2) A `transaction` service test runs the counterparty's
  refused confirm (a hook stub) to the signed `commitment-failure`, and
  the writer's filing of it to the `failed` snapshot. (3) A two-node test
  sends a hand-signed `commitment-failure` from the counterparty's node to
  the writer's node as a card, and checks both nodes' snapshots and the
  release (section 7.4). This is how C1 tested AG-12 across two nodes (a
  hand-signed half delivered as a card; agreements doc Deviations, "AG-12's
  test", `:1254`). Recommended: no test-only production code, and each part
  runs in seconds.
- (b) The expiry comes from the scheduling service's app config
  (`reservation_ttl_secs`, default 900), and the two-node test deploys with
  2 seconds. A real expiry runs across two nodes. Cost: the first Roym
  service that reads `syneroym:app-config`; the deploy path of a config
  value for a test node must be checked first, and an operator setting
  appears that no person asked for.

#### Q6. Demo

The Mandatory Demo Close-out asks for a story when a viewer can see
something new. C3b1 adds one line to the backup screen. C3b2 adds a card
that shows only after a 15-minute wait.

- **(a) C3b1: `- none: booking looks the same; the scheduling backup line
  is covered by the Hub test`. C3b2: `- none: the failure card needs a
  reservation older than 15 minutes; the Hub unit tests and the two-node
  test cover it`.** Stories 60 (slot booking) and 50 keep passing, and now
  book through the scheduling service. Recommended. The owner said in C3a
  that `none` needs an explicit yes, so this question asks for it.
- (b) Extend story 60 with the backup screen's scheduling line in C3b1.
  It shows a download link, not booking.

#### Q7. The names of the slot methods

- **(a) Keep `availability.set`, `.get`, `.list`, `.remove` with today's
  inputs and outputs.** Only the service that answers them changes. The
  Hub's listing screen
  ([listings.ts:597](../../../../crates/roym_web/ui/src/screens/listings.ts#L597)
  to `:623`) and every test call keep working through the router.
  Recommended.
- (b) Rename them to `scheduling.slot.*`. Every caller changes, for no
  gain to a person.

#### Q8. `transaction`'s `depends_on` edge to `catalog`

After C3b1, `transaction` calls `catalog` nowhere: both calls were slot
calls (`hold.rs:105`, `quote_ops.rs:444`). C3c's `quote.from-listing`
will read the current listing from `catalog` again (agreements doc,
"Methods", `:981`).

- **(a) Replace the edge in C3b1: `depends_on = ["conversation",
  "scheduling"]`.** The manifest comment then says only what is true. C3c
  adds `catalog` back, with its reason. Recommended: a declared edge that
  no code uses gives the service a reach it does not need.
- (b) Keep `catalog` and add `scheduling`: `["conversation", "catalog",
  "scheduling"]`. One line less churn; the comment must still change.

### 2.2 Points found while reading

1. **task.md's facts are older than the code.** Section 2 says the slot
   capacity limit is in `roym_core`'s `booking.rs:20`; `booking.rs` is
   gone and `MAX_SLOT_CAPACITY` is in `transaction`
   ([hold.rs:18](../../../../crates/roym_transaction/src/app/hold.rs#L18)).
   The track window is deal core now (`TRACK_WINDOW_SECS`,
   [transition.rs:10](../../../../crates/roym_core/src/deal/transition.rs#L10)).
   So "the booking track" that EX-17 and task.md move is gone already:
   C1a replaced it with the commitment states. What is left to move is the
   slot store, the seat claim and the quote-time slot check. No action:
   task.md section 2 is a record of its own date.
2. **`PeerStep` is gone.** task.md section 7 says C3b moves `PeerStep`
   (in `booking_ops.rs`). Neither exists on `main`; the backlog row
   ("One Roym output carries another node's card data as an untyped
   `Value`", [deferred-backlog.md:537](../../deferred-backlog.md)) already
   says so. No action in C3b; C8's close-out fixes task.md's table.
3. **Nine seat sites in `transaction`.** Each matches
   `ResourceKind::ServiceInSlot` and the giver's role:
   - `reserve_counterparty_slots` ([agreement_ops.rs:260](../../../../crates/roym_transaction/src/app/agreement_ops.rs#L260),
     called at `:179`);
   - the counterparty's release at quote expiry in `agreement.get`
     (`agreement_ops.rs:508` to `:521`);
   - `reserve_author_holds` ([decide.rs:419](../../../../crates/roym_transaction/src/app/decide.rs#L419)
     to `:452`), and the confirms at `:122` to `:127` and `:587` to `:592`;
   - `handle_not_formed`'s release (`decide.rs:658` to `:660`);
   - `release_all_author_holds` ([recover.rs:28](../../../../crates/roym_transaction/src/app/decide/recover.rs#L28)
     to `:38`), and the confirm in recovery (`recover.rs:179` to `:190`);
   - the writer's release on cancel in `finalize_snapshot`
     ([progress_ops.rs:104](../../../../crates/roym_transaction/src/app/progress_ops.rs#L104)
     to `:110`);
   - the counterparty's release at expiry in `tick_conversation_at`
     (`progress_ops.rs:341` to `:354`);
   - the counterparty's release on a `not-formed` or `cancelled` snapshot
     ([receipts.rs:289](../../../../crates/roym_transaction/src/app/sync/receipts.rs#L289)
     to `:295`);
   - `prepare_terms` and `validate_and_apply_slot` (`quote_ops.rs:150`,
     `:419`).
   The tenth match, the `fulfil:` kind check on import
   ([sections.rs:378](../../../../crates/roym_transaction/src/app/backup/sections.rs#L378)),
   is a deal-core kind rule and stays.
4. **A `Reservations` trait already exists.** `hold.rs:29` to `:52`
   defines it, `decide::run_with_reservations` takes it (`decide.rs:41`),
   and the recovery tests use a stub that refuses `confirm`
   ([recover.rs tests:242](../../../../crates/roym_transaction/src/app/tests/recover.rs#L242)
   to `:275`). C3b keeps the trait as the one place where `transaction`
   meets an extension, and gives it a production implementation that calls
   the extension service. No new machinery for the seam.
5. **The counterparty never confirms today.** No code calls `confirm` for
   a reservation the counterparty took. That is harmless while a hold
   never expires. C3b2 adds it (D-C3b-11), in the same PR as the expiry.
6. **Today's release can free another agreement's seat.** `SeatLedger::release`
   reads the `hold:` row, then deletes the `hold:` and `seat:` rows in one
   batch (`hold.rs:180` to `:195`). Two releases of one reservation that
   run at the same time both read the hold. If a new reserve takes the
   freed seat between the first delete and the second, the second batch
   deletes the new agreement's `seat:` row. The data layer has `create`
   (a fence), `put`, `patch` and `delete`, and no compare-and-delete
   (`data-layer.wit:74` to `:78`, `:166`, `:184`, `:200`). So the new
   reservation model never deletes a seat row (D-C3b-5).
7. **The Hub has no words for three commitment states.** The progress
   card's table has `promised`, `payment-requested`, `fulfilled`,
   `acknowledged`, `ended-unconfirmed` and `other-failed`
   ([agreement_progress.ts:47](../../../../crates/roym_web/ui/src/cards/templates/agreement_progress.ts#L47)
   to `:54`). `CommitmentState` has `claimed`, `failed` and `cancelled`
   too, and no `other-failed`
   ([transition.rs:15](../../../../crates/roym_core/src/deal/transition.rs#L15)
   to `:24`). `other-commitment-failed` is a reason, not a state. C3b2 is
   the first slice where `failed` can happen, so it fixes the table
   (D-C3b-13). `claimed` and `cancelled` can happen today; the Hub shows the
   raw code.
8. **Three lists of services outside `roym_core`.** `roymctl`'s backup
   order ([backup.rs:33](../../../../apps/roymctl/src/commands/roym/backup.rs#L33)
   to `:35`), the Hub's backup screen
   ([backup.ts:7](../../../../crates/roym_web/ui/src/screens/backup.ts#L7)
   to `:38`) and the substrate's owner loop
   ([roym.rs:105](../../../../crates/substrate/src/runtime/roym.rs#L105)
   to `:116`) each name the services one by one. `roymctl`'s signing list
   (`signing.rs:13`) names the services that sign; scheduling signs nothing
   and is not added there (D-C3b-2).
9. **The router decides which service answers a method.** `availability.`
   routes to `catalog` ([router.rs:30](../../../../crates/roym_core/src/router.rs#L30)).
   The router's tests check every routed service against `SIBLINGS` and
   against `roym.toml` (`router.rs:89` to `:230`). So the router row, the
   service list and the manifest change in one commit.
10. **No Roym service reads `syneroym:app-config` yet.** The host has it
    (`AppAppConfig`, [lib.rs:104](../../../../crates/app_host/src/lib.rs#L104)),
    but no `crates/roym_*/src` file calls it. This is why Q5 (b) costs more
    than it looks.

## 3. What C3b meets

| Change doc | Criterion | Part | Where it is proved (section 7) |
| --- | --- | --- | --- |
| roym-agreements | AG-14 | C3b1: the writer reserves and confirms through the extension service in the fixed step order; the last-seat race ends `agreed` and `not-formed` / `slot-taken`; the crash and expiry recovery tests pass through the hook client. C3b2: a refused `confirm` in recovery (real lapse) | two-node `roym_booking_e2e`; service `tests/recover.rs`, `tests/decide.rs`; scheduling unit tests |
| roym-agreements | AG-15 | C3b1: the counterparty reserves through the extension before it signs; a refusal answers `reserve-failed` with the extension's reason and signs nothing | service `tests/accept.rs` (`accept_refuses_reserve_failed_and_signs_nothing`, renamed from today's case) |
| roym-agreements | AG-16 | C3b2: a refused confirm on the counterparty's node gives one signed `commitment-failure`; the writer marks the commitment `failed` with `reservation-lost` | service `tests/commitment_failure.rs`; scheduling `lapse` tests; two-node `roym_reservation_e2e` (Q5) |
| roym-agreements | AG-36 | C3b2: the snapshot part (the cascade in the same snapshot) and the release by each giver | service `tests/commitment_failure.rs`; the transition table part is C1's |
| roym-extensions | EX-4 | C3b1: one agreement with scheduling and one with one-shot pass through the same states | service `tests/extension.rs` (`scheduling_and_one_shot_pass_the_same_states`) |
| roym-extensions | EX-5 | C3b1: each giver's node calls the reserve hook of each extension named in its commitments; one failure releases the others and forms nothing | service `tests/accept.rs`, `tests/decide.rs` |
| roym-extensions | EX-6 | C3b1: the reservation is a fence on the resource owner's own node, also when the owner is the counterparty | scheduling unit tests; service `tests/accept.rs` |
| roym-extensions | EX-14 | C3b1: the scheduling service ships in `roym.toml` and is deployed with the app | two-node tests deploy it; `router.rs` manifest tests |
| roym-extensions | EX-17 | C3b1: slots, seat capacity and the seat claim run in the scheduling service; `catalog` and `transaction` hold no slot code | exit criterion grep (section 10); `roym_booking_e2e` |
| roym-extensions | EX-20 | C3b2: a 15-minute expiry set by the owner's node; the owner's node frees the seat for any new booking with no other node (Q4) | scheduling `lapse` tests |
| roym-extensions | EX-21 | C3b1: reserve and release are idempotent by (agreement, commitment); release of an unknown or released reservation succeeds; release also frees a confirmed one | scheduling unit tests |
| roym-extensions | EX-22 | C3b2: firm only after confirm; a confirm after the expiry succeeds while nobody took the seat, and is refused after a takeover or a release (narrower, see section 2.1) | scheduling unit tests; service `tests/commitment_failure.rs` |
| roym-value-exchange | VX-10 | C3b1: the reserve goes through the scheduling extension's hook, under the slot owner's fence, also when the owner is not the quote author | as AG-14 and AG-15 |

Not met here: EX-15, EX-16 (moved out of M08C). AG-44, AG-45 (C3c builds
`instant` on top of these hooks).

## 4. Behaviour-change audit

### Wire changes (between nodes)

- **None in C3b1.** No record, card or wire-reachable method changes. A
  quote with a slot has the same signed bytes as before.
- **C3b2: a new record and card, `commitment-failure` version 1** (agreements
  doc, "Commitment failure"). It is in `RECORD_TYPES`
  ([record.rs:18](../../../../crates/roym_core/src/record.rs#L18)) and
  `CARD_TYPES` ([card.rs:11](../../../../crates/roym_core/src/card.rs#L11))
  and the Hub's card registry. The card carries no disclosures. An older
  node files it as a card of an unknown type, as for every new card type.
- **C3b2: the counterparty's node sends one more card on its own**, during
  `transaction.sync`, when its confirm is refused.

### Local method and manifest changes (one node)

- **`availability.*` is answered by `scheduling`** (C3b1, Q7). Same names,
  same inputs and outputs. The router row changes (`router.rs:30`). A
  sibling caller of `catalog`'s `availability.*` gets `-32601` from
  `catalog`; there is none after C3b1.
- **New local-only methods on `scheduling`** (C3b1): `reservation.prepare`,
  `reservation.reserve`, `reservation.confirm`, `reservation.release`,
  `scheduling.ping`, `scheduling.export`, `scheduling.import`. All through
  `admit::require_internal`. The router routes `availability.` and
  `scheduling.` to it. It does **not** route `reservation.`: only
  `transaction` calls the hooks, so a Hub call gets `-32601`.
- **Two outputs of `transaction` gain fields.** `transaction.import`:
  `reservations_restored` and `reservations_failed` (C3b1, D-C3b-16).
  `transaction.sync`: `unsent_failures` (C3b2, D-C3b-11). Both are added
  fields; a client that does not read them sees no change.
- **`roym.toml`** (C3b1): a seventh service `scheduling`
  (`visibility = "private"`, no `depends_on`); `transaction`'s `depends_on`
  becomes `["conversation", "scheduling"]` (Q8); `web`'s gets `scheduling`.
- **The extension manifest** (C3b1): a `service` key (Q2), and `prepare` as
  an optional fourth hook (Q3). `scheduling.json` gets both.
- **Backup** (C3b1): `catalog.export` no longer has the `availability`
  section, and `catalog`'s `SCHEMA_VERSION` goes from 4 to 5
  ([app.rs:25](../../../../crates/roym_catalog/src/app.rs#L25)). A new
  `scheduling` bundle has `availability` and `reservations`.
  `transaction`'s bundle no longer has `hold:` or `seat:` rows, and its
  `SCHEMA_VERSION` goes from 6 to 7
  ([app.rs:54](../../../../crates/roym_transaction/src/app.rs#L54)). An old
  bundle is refused by the version check, by the owner's pre-release rule
  (no migration). C3b2 adds `failure:` and `confirmed:` ledger rows to
  `transaction`'s bundle with no new version (they are new kinds that an
  older import refuses anyway; the version already moved in C3b1 of the
  same milestone, and no release lies between).

### Behaviour changes

- **C3b1: none that a person sees.** Holds never expire, as before. The
  refusal codes (`slot-taken`, `slot-unavailable`, `reserve-failed`,
  `no-such-slot`, `slot-not-in-listing`, `schedule-differs-from-slot`) are
  the same.
- **C3b1: a release can no longer free another agreement's seat** (section
  2.2, point 6). This fixes a race; nothing else changes.
- **C3b1: the scheduling backup is a separate bundle.** Today the seat
  rows are checked against the quotes in the same `transaction` bundle
  (`sections.rs:498` to `:520`, `:600` to `:615`). After the move,
  `scheduling.import` checks only its own rows (D-C3b-9), and
  `transaction.import` reserves again the seat of every restored agreement
  (D-C3b-16). Its output gains `reservations_restored` and
  `reservations_failed`. A restored seat with no agreement stays taken
  (a backlog row).
- **C3b2: an unconfirmed reservation older than 15 minutes can be taken by
  another booking** (Q4). The writer confirms its own at once, so this
  happens only to a counterparty that gives a slot and whose writer has not
  countersigned within 15 minutes (agreements doc, open question 4).
- **C3b2: the counterparty's node confirms** its reservations when it
  files the writer's countersigned half, and signs a `commitment-failure`
  when the confirm is refused.
- **C3b2: a release now happens for one commitment** that ends `failed` or
  `cancelled` while the agreement goes on (AG-36). Before, a release
  happened only when the whole agreement ended `not-formed` or
  `cancelled`.
- **C3b2: the Hub's progress card** shows words for `claimed`, `failed` and
  `cancelled`, and the reasons `reservation-lost` and
  `other-commitment-failed`.

### Stays the same

- Every deal state, transition, reason code and settlement rule. The
  transition tables already have `failure` and `other-failed`
  (`transition.rs:34`, `:35`), and `progress::apply` already has
  `AgreementEvent::Failure` with the cascade
  ([progress.rs:236](../../../../crates/roym_core/src/progress.rs#L236)).
- The writer's decision: its step order, its rows (`decision:`,
  `countersign:`, `refused:`, `snapshot:`), its recovery. Only the calls
  inside step 3 and step 4 go to another service.
- The signed bytes of every record of C1 to C3a. The `scheduling/slot@1`
  schema and its check `slot-matches-window`.
- The Hub's listing editor and slot list; the quote and enquiry forms;
  stories 40, 50 and 60.
- `conversation`, `profile`, `directory`, `web`'s Rust code (only its
  manifest `depends_on` line changes), the WIT interfaces.

## 5. Decisions this plan makes

**D-C3b-1. The crate.** `crates/roym_scheduling/`, package
`syneroym-roym-scheduling`, the same shape as `crates/roym_profile/`:

```text
crates/roym_scheduling/
  Cargo.toml          # as roym_profile/Cargo.toml: app-host, roym-core, serde, serde_json;
                      # wit-bindgen + syneroym-wit-interfaces on wasm32; syneroym-rpc on native
  wit/world.wit       # package syneroym-roym:scheduling@0.1.0; the same imports as
                      # roym_profile/wit/world.wit, because syneroym-app-host's guest host
                      # binds every one; export api
  wit/deps/           # the same twelve folders as roym_profile/wit/deps
  src/lib.rs          # cfg-gated guest / native, as roym_profile/src/lib.rs
  src/guest.rs        # wit-bindgen glue, as roym_profile/src/guest.rs
  src/native.rs       # NativeScheduling, as roym_profile/src/native.rs
  src/bindings.rs     # generated by cargo component
  src/api.rs          # service_methods! table
  src/app.rs          # SCHEMA_VERSION = 1, collections, ensure_* helpers, status
  src/app/availability.rs   # moved from roym_catalog, unchanged but for imports
  src/app/reservation.rs    # reserve / confirm / release / lapse (D-C3b-5)
  src/app/reservation/rows.rs
  src/app/prepare.rs        # the quote-time slot check (Q3)
  src/app/backup.rs
  src/app/tests.rs + src/app/tests/*.rs
```

`api.rs`:

```rust
syneroym_roym_core::service_methods! {
    "scheduling.ping": ping [NoParams => PingOutput],
    "availability.set": availability::set [AvailabilitySetInput => AvailabilitySetOutput],
    "availability.get": availability::get [SlotKeyInput => Option<SlotRow>],
    "availability.list": availability::list [AvailabilityListInput => AvailabilityListOutput],
    "availability.remove": availability::remove [SlotKeyInput => AvailabilityRemoveOutput],
    "reservation.prepare": prepare::prepare [PrepareInput => PrepareOutput],
    "reservation.reserve": reservation::reserve [ReserveInput => ReservationView],
    "reservation.confirm": reservation::confirm [ReservationKey => ReservationView],
    "reservation.release": reservation::release [ReservationKey => ReleaseOutput],
    "scheduling.export": backup::export [NoParams => Bundle],
    "scheduling.import": backup::import [ImportInput => backup::SchedulingImportOutput],
}
```

Every method goes through `admit::require_internal`, as in every Roym
service. `scheduling` signs nothing, so it has no `signing-status` or
`install-signing-certificate` verb and is not in `roymctl`'s signing list.

**D-C3b-2. Wiring a seventh service.** One list per place, each found on
`main`:

| Place | Change |
| --- | --- |
| `crates/roym_core/src/services.rs:12` to `:28` | `SCHEDULING = Service { name: "scheduling", interface: "syneroym-roym:scheduling/api@0.1.0" }`; `ALL: [Service; 7]` (after `TRANSACTION`, before `DIRECTORY`, in manifest order); `SIBLINGS: [Service; 6]` |
| `crates/roym_core/src/services.rs` | `pub fn by_name(name: &str) -> Option<Service>` over `ALL`, for D-C3b-3 |
| `crates/roym_core/src/router.rs:30` | `("availability.", SCHEDULING, Owner)`, and a new `("scheduling.", SCHEDULING, Owner)` |
| `crates/roym_core/app/roym.toml` | `[services.scheduling]` (wasm source `target/wasm32-wasip2/release/syneroym_roym_scheduling.wasm`, `visibility = "private"`, a health check on `status`); `web` and `transaction` `depends_on` (Q8); `transaction`'s comment says it reads slots and reserves through the scheduling extension |
| root `Cargo.toml:99` | `syneroym-roym-scheduling = { path = "crates/roym_scheduling" }` |
| `crates/substrate/Cargo.toml:44`, `:108` | optional dep and the `roym` feature |
| `crates/substrate/src/runtime/roym.rs` | `init_roym_scheduling`, a copy of `init_roym_profile` (`:209` to `:246`) with the scheduling names; added to the factory list (`:87` to `:95`) and the owner loop (`:105` to `:116`); in `wire_roym_topology` (`:453`), `transaction`'s binding to `scheduling` in place of `catalog` (Q8) |
| `crates/core/src/test_constants.rs:179` | `roym_scheduling_wasm_path()` |
| `crates/roym_web/Cargo.toml:55` | dev dependency, for the parity and method-contract harnesses |
| `crates/roym_web/tests/dual_build_parity/helpers.rs` | five places: the `status` arm (`:377` to `:384`), the native-service arm (`:645` to `:651`; it `panic!`s on an unknown name), the WASM path list (`:890` to `:897`), the factories and native services (`:1092` to `:1117` and the `Arc::new(Native…)` blocks after them), and the wire factory list (`:1183` to `:1215`) |
| `crates/roym_web/tests/dual_build_parity/test_proxies.rs` | the import (`:14`) and the dispatch by DID (`:150` to `:160`) |
| `crates/roym_web/tests/dual_build_parity/fixtures.rs:131` to `:138` | `expected_schema_version`: `transaction` 7, `catalog` 5 (`scheduling` falls to the default 1, which is its version) |
| `crates/substrate/tests/e2e/node-boot.ts:32` | `ROYM_SERVICES` gets `'scheduling'`: the Playwright setup builds each component from this list (`:115` to `:117`) |
| `crates/substrate/tests/common/roym.rs:81` | `service_visibility` gets `"scheduling" => Visibility::Private`, so the test deploy matches `roym.toml` |
| `crates/substrate/tests/roym_restore_e2e.rs:27`, `:28` | `EXPORT_SERVICES` and `IMPORT_ORDER`, in the orders of D-C3b-16 |
| `crates/substrate/tests/roym_trust_e2e.rs:340` | `DATA_SERVICES` |
| `crates/roym_web/tests/method_contract/contract.rs:11`, `:29`, `:138` | `scheduling_api::method_table()` in the per-service table |
| `mise.toml:64` | `scheduling` in the `build:roym` loop |
| `xtask/src/main.rs:155` to `:162` | `"syneroym-roym-scheduling"` in `check_roym_deps`'s `siblings` |
| `apps/roymctl/src/commands/roym/backup.rs:33` to `:35` | `scheduling` in both orders: after `transaction` when exporting, before it when importing (D-C3b-16) |
| `crates/roym_web/ui/src/screens/backup.ts:5` to `:38` | a sixth bundle `scheduling` with `backup-bundle-scheduling`, above `transaction` (D-C3b-16); the comment at `:5` ("The five app-data bundles") says six |
| `crates/substrate/tests/e2e/global-setup.ts:131` to `:138`, `node-boot.ts:58` | the "six" in comments and the log line become "seven" |

Lists that are **not** changed, because they name the services that sign
and `scheduling` signs nothing: `roymctl`'s `signing.rs:13`, the test
harness's `SIGNING_SERVICES` (`common/roym.rs:38`), the Hub's
`session/enrolment.ts:7`, and the router test
`every_certificate_mounted_service_routes_under_its_own_name`
(`router.rs:273`).

Before the step that adds the service, this search is run, and **every
hit is read** and checked against this table. It finds the identifier
forms and the string forms (`['…', 'directory']`), which a search for
identifiers alone misses:

```bash
git grep -nE "['\"]directory['\"]|roym[_-]directory|DIRECTORY\b" -- crates apps xtask mise.toml .github scripts
```

Most hits are not service lists (a directory hit's `sources[].directory`
field, for example). A hit that is a list and that the table misses is
added to the table and to D-C3b-15 in the same step.

**D-C3b-3. The manifest names its service and its hooks (Q2, Q3).**
`scheduling.json`:

```json
{
  "id": "scheduling",
  "version": 1,
  "kind": "workflow",
  "service": "scheduling",
  "resource_kinds": ["service-in-slot"],
  "hooks": ["prepare", "reserve", "confirm", "release"],
  "details": [ ... unchanged ... ],
  "steps": [ ... unchanged ... ]
}
```

- `Hook` gets a fourth value, `Prepare`
  ([extension.rs:62](../../../../crates/roym_core/src/extension.rs#L62)).
- The loader's hook rule changes from "all three or none" (D-C3a-4) to:
  `reserve`, `confirm` and `release` are all present or all absent;
  `prepare` is allowed only with them.
- `service` is required when `hooks` is not empty, absent otherwise, and
  must name an entry of `services::ALL` that is not `web`. One failing
  load test per rule.
- `Manifest` gets `pub service: Option<String>` and
  `pub fn hook_service(&self) -> Option<Service>` (through
  `services::by_name`), and `pub fn has_prepare_hook(&self) -> bool`.
- The Hub's `extensions/registry.ts` reads the same JSON; it ignores
  `service` and `hooks`. Its test that the file list equals Rust's needs no
  change.

**D-C3b-4. The hook inputs and outputs** live in a new
`crates/roym_core/src/method/scheduling.rs`, beside
`method/catalog.rs`. `SlotKeyInput` and `SlotRow` move there from
`method/catalog.rs:24` to `:39`, and `MAX_SLOT_CAPACITY` (64) moves there
from `hold.rs:18`. The hook types are generic: they carry the commitment's
`details` and `window`, never a slot id, so `transaction` reads no slot
field.

```rust
/// Names one reservation: the agreement is the quote version's record id.
#[serde(deny_unknown_fields)]
pub struct ReservationKey { pub agreement: String, pub commitment_id: String }

#[serde(deny_unknown_fields)]
pub struct PrepareInput { pub details: Details, pub window: Option<TimeWindow>,
                          pub listing_id: Option<String> }  // the terms' listing_id
pub struct PrepareOutput { pub window: TimeWindow }

/// No `window`: the slot id already names its window (D-C3b-5).
#[serde(deny_unknown_fields)]
pub struct ReserveInput { pub agreement: String, pub commitment_id: String,
                          pub details: Details }

pub enum ReservationState { Held, Confirmed }   // kebab-case on the wire
pub struct ReservationView {
    pub agreement: String, pub commitment_id: String,
    pub state: ReservationState,
    pub expires_at_secs: Option<u64>,   // None in C3b1: never expires
}
pub struct ReleaseOutput { pub released: bool }  // false: unknown or already released
```

A refusal is `invalid_params` with `data.code`, as `listing.set` and
`quote.set` refuse today (`extension_rules.rs:15` to `:18`):

| Code | From | Meaning |
| --- | --- | --- |
| `no-such-slot` | prepare | the slot is not in this node's store |
| `slot-not-in-listing` | prepare | the slot belongs to another listing |
| `schedule-differs-from-slot` | prepare | the given window is not the slot's |
| `slot-unavailable` | reserve | the slot is gone, or its capacity is 0 |
| `slot-taken` | reserve | every seat is held |
| `reservation-unknown` | confirm | no reservation for this key |
| `reservation-lost` | confirm (C3b2) | the reservation lapsed and its seat was taken |

`prepare` keeps today's text for its three codes (they are the message of
today's `invalid_params`, `quote_ops.rs:436`, `:459`, `:465`) and adds
`data.code`.

**D-C3b-5. The reservation rows.** One collection, `reservations`, so that
every fence is one `create` (a `create` works on one collection,
`data-layer.wit:186`). No row is ever deleted or overwritten, except the
pointer row `key:`. The rows:

| Row id | Payload | Written |
| --- | --- | --- |
| `rsv:<r>` | `{ kind: "rsv", agreement, commitment_id, generation, slot_id, seat, round, created_at_secs, expires_at_secs }` | created with its seat row, **first in the `create` list** |
| `seat:<slot_id>:<seat>:<round>` | `{ kind: "seat", reservation: r, slot_id, seat, round }` | created with its `rsv:` row, second in the list |
| `key:<agreement>:<commitment_id>` | `{ kind: "key", generation }` | `put` after the claim; a pointer only |
| `decided:<r>` | `{ kind: "decided", state: "confirmed" \| "lapsed", at_secs }` | created once |
| `free:<r>` | `{ kind: "free", why: "released" \| "lapsed", at_secs }` | created once |

Every row has `kind`, as today's `hold:` and `seat:` rows do
(`hold.rs:64`, `:74`). The collection indexes `kind` and `slot_id`, and
the seat query filters on both, because `rsv:` rows carry `slot_id` too.

**The order of the claim matters.** When any id in a `create` exists,
`create` writes nothing and names the *first* existing id in the list
(`data-layer.wit:186` to `:201`;
`data_db/src/tests_create.rs:99`,
`create_writes_nothing_and_names_the_first_existing_id`). `rsv:<r>` goes
first, so a second caller with the same key gets the `rsv:` id back, not
the seat id. Today's code uses the same order: the hold row first
(`hold.rs:145` to `:163`). And the id alone never decides: after any
`Some(id)`, `reserve` reads `current(key)` again, and returns the
reservation if it now exists.

- `r = content_digest("rsv_", { agreement, commitment_id, generation })`
  (`signed_record::content_digest`, the id rule of every Roym record).
  `generation` starts at 1 and grows by one when a reservation for the same
  key is taken again after a release.
- **A seat is held by round.** Seat `n` of a slot has rows for rounds
  1, 2, …. The row with the highest round is the seat's occupant. The seat
  is free when it has no row, or when its occupant's `free:<r>` exists.
  Taking a free seat creates the next round. Two takers of one seat race on
  the same `create`; one wins. This needs no delete, so no release can ever
  free a seat that another reservation holds (section 2.2, point 6).
- **The current reservation of a key.** Read `key:` (generation `g`, or 0
  when absent); while `rsv:<r(g+1)>` exists, `g += 1`. This finds a
  reservation whose claim succeeded but whose `key:` put was lost in a
  crash. The loop runs at most once in practice.

Pseudo-code (C3b1; the C3b2 parts are marked):

```text
reserve(input, now):
  slot_id  = input.details.data["slot_id"]            # checked by the schema already
  (g, cur) = current(input.agreement, input.commitment_id)
  if cur exists and free:<cur> absent:                # a repeat (EX-21)
      return view(cur)
  slot = availability row slot_id; None or capacity 0 -> refuse slot-unavailable
  r = rid(agreement, commitment_id, g + 1)
  seats = query reservations where kind = "seat" and slot_id = slot_id  (all rounds)
  for n in 1..=min(slot.capacity, MAX_SLOT_CAPACITY):
      top = highest round of seat n in `seats` (0 if none)
      if top > 0 and not is_free(seats[n][top], now):    # C3b2: lapse_if_due here
          continue
      match create([rsv:<r>, seat:<slot_id>:<n>:<top+1>]):   # rsv first
          None -> put key {g + 1}; return view(r)
          Some(_) ->
              (_, now_cur) = current(key)                 # never decide by the id alone
              if now_cur exists and free:<now_cur> absent:
                  put key; return view(now_cur)            # a same-key caller won, or a crash
              continue                                     # another key won seat n
  refuse slot-taken

is_free(row, now) = free:<row.reservation> exists
                    # C3b2: or lapse(row.reservation, now) succeeds

confirm(key, now):
  (_, cur) = current(key); none -> refuse reservation-unknown
  if free:<cur> exists and decided:<cur> absent:       # released before any confirm
      refuse reservation-lost                          # never takes a seat again
  match create([decided:<cur> {confirmed}]):
      None -> return view(cur, confirmed)
      Some(_) -> decided says confirmed -> return view(cur, confirmed)  # a repeat
                 decided says lapsed    -> refuse reservation-lost      # C3b2

release(key, now):
  (_, cur) = current(key); none -> return { released: false }
  match create([free:<cur> {released}]):
      None    -> return { released: true }
      Some(_) -> return { released: false }            # already freed (EX-21)

lapse(r, now):                                         # C3b2
  if now < rsv:<r>.expires_at_secs: return false
  create([decided:<r> {lapsed}, free:<r> {lapsed}]) is None   # fails if confirmed or freed
```

- `release` frees a confirmed reservation too, as the agreements doc
  requires ("Release after confirm", `:745`). The rule "only while no
  settlement half exists" is the deal core's: `transaction` never calls
  `release` for a commitment that has a half (D-C3b-7).
- In C3b1 `expires_at_secs` is `None` and `lapse` is never called. So a
  reservation never ends but by `release`, as today.
- **`confirm` never takes a seat** (owner answer, section 2.1). It writes
  only a `decided:` row for the reservation that exists. Only `reserve`
  creates `seat:` rows. A confirm after a release is refused
  `reservation-lost`, even when the seat is free. A repeat confirm of a
  confirmed reservation that was later released (the writer's step 4
  releases confirmed ones) answers its stored `confirmed`; no caller asks
  for it, because `transaction` confirms only `promised` commitments.
- The `seats` query uses the `kind` and `slot_id` indexes (`ensure_coll`
  with indexes, as `transaction`'s collections do). It reads every round of
  every seat of one slot, and then at most 64 `get`s of `free:` rows (one
  per seat's top round). Rounds are never removed, so the read grows with
  the number of bookings that one slot ever had. A slot is one time window
  of one listing, so this stays small: each new round needs a release or a
  lapse first. If a slot ever needs many rounds, the change is to read only
  the top round of each seat by `get`, from a stored top round per seat;
  it is not built now.
- No `window` check in `reserve` (P9 of the review). The slot id is
  `derive_slot_id(listing_id, start_secs, end_secs)`, and the schema check
  `slot-matches-window` has already proved that the commitment's window
  gives that id. So the stored slot cannot have another window.
  `ReserveInput` has no `window` field (D-C3b-4).
- `availability.remove` does not touch reservations, as today it does not
  touch holds.

**D-C3b-6. The hook client in `transaction`.** `hold.rs` is replaced by
`app/hooks.rs`:

```rust
/// Whether the deal core must reserve this commitment's resource.
pub(crate) fn needs_reservation(c: &Commitment) -> bool {
    c.extension.as_ref()
        .and_then(|e| extension::registry().workflow(e))
        .is_some_and(Manifest::has_reserve_hook)
}

/// The commitments of `terms` that `role` gives and must reserve.
pub(crate) fn reserved_by(terms: &DealTerms, role: DealRole) -> impl Iterator<Item = &Commitment>;

pub enum ReserveOutcome { Held, Refused { reason: String } }
pub enum ConfirmOutcome { Confirmed, Refused { reason: String } }

pub trait Reservations {           // today's trait, new signatures
    async fn reserve<H: AppHost>(&self, host: &H, agreement: &str, c: &Commitment)
        -> Result<ReserveOutcome, String>;
    async fn confirm<H: AppHost>(&self, host: &H, agreement: &str, c: &Commitment)
        -> Result<ConfirmOutcome, String>;
    async fn release<H: AppHost>(&self, host: &H, agreement: &str, c: &Commitment)
        -> Result<(), String>;
}

/// Calls `reservation.<hook>` on the service the manifest names.
pub struct ExtensionHooks;
```

- `ExtensionHooks` maps `SiblingError::Refused(resp)` with a `data.code`
  to `Refused { reason: code }`; a `Refused` with no code, and
  `BadReply`, to `Err` (a bug, not a refusal); `Transport` to `Err`, as
  `hold.rs:114` to `:117` does today for the catalog call.
- **The reason is checked before it is used** (review P6). It goes into a
  signed `not-formed` snapshot (`state_reason`) and into the
  `reserve-failed` error, and `progress::validate` checks only that
  `state_reason` is present for `not-formed` (`progress.rs:152`). Today the
  reasons are two fixed strings (`decide.rs:443`, `:446`). So
  `ExtensionHooks` accepts a code only when it is `[a-z0-9-]{1,64}`, the
  form of every Roym reason code; any other code is `Err` (a bug in the
  extension, not a refusal). A unit test covers a good code, a 65-byte code
  and a code with an upper-case letter.
- `reserve_counterparty_slots` (`agreement_ops.rs:260`) becomes
  `reserve_own(host, agreement, terms, DealRole::Counterparty, &hooks)`.
  On a refusal it releases what it reserved and answers `reserve-failed`
  with `data.reason`, as today (`:290` to `:291`).
- `reserve_author_holds` (`decide.rs:419`) returns
  `(Vec<String>, Option<String>)`: the reason is the extension's code, not a
  `&'static str`. The not-formed snapshot's `state_reason` is that code
  (`slot-taken` or `slot-unavailable` from scheduling: the same as today).
- Every release site of section 2.2, point 3 calls
  `release_own(host, agreement, terms, role, &hooks)` or the per-commitment
  form of D-C3b-12. No site matches `ResourceKind::ServiceInSlot` any more.
- `prepare_terms` (`quote_ops.rs:144`) calls `reservation.prepare` for
  each commitment the author gives whose manifest has a prepare hook, and
  sets `window` from the answer. It has two callers, `quote.set`
  (`quote_ops.rs:190`) and `quote.measure` (`measure_at`, `:254`), so both
  make the call and both answer the same refusal codes (review P7). `validate_and_apply_slot` (`:419` to
  `:470`) and the `SlotKeyInput`/`SlotRow` imports go. This needs the
  manifest from the plain `CommitmentInput`; the registry lookup uses its
  `extension`.
- The test fixture's default `catalog.availability.get` stub
  ([tests.rs:139](../../../../crates/roym_transaction/src/app/tests.rs#L139)
  to `:155`) becomes a stub of the four `scheduling` hooks over an
  in-memory seat map (`tests/hooks_fake.rs`). Tests that need a refusal
  pass their own `Reservations` value, as `RefusingConfirmReservations`
  does today. The real seat rules are tested in the scheduling crate and in
  the two-node and parity tests, not twice.

**D-C3b-7. When `transaction` calls each hook.** The agreements doc's
points ("The flow", "The writer's decision", "Who reserves"), mapped to
code:

| Point | Node | Call | Code |
| --- | --- | --- | --- |
| `quote.set`, `quote.measure` | author | `prepare` for each own commitment with the hook | `prepare_terms` |
| `agreement.accept` | counterparty | `reserve` own; on a refusal `release` the others, answer `reserve-failed` | `reserve_own` |
| decision step 3 | writer | `reserve` own | `reserve_author_holds` |
| decision step 4 | writer | `confirm` own; on a refusal store `not-formed`, `release` all own | `decide.rs:122`, `:587`; `recover.rs:185` |
| not-formed, recovery 1 and 2 | writer | `release` all own | `handle_not_formed`, `release_all_author_holds` |
| `agreement.cancel` | writer | `release` all own | `finalize_snapshot` |
| quote expired, no countersign | counterparty | `release` all own | `agreement_ops.rs:508`, `progress_ops.rs:341` |
| `not-formed` or `cancelled` snapshot filed | counterparty | `release` all own | `apply_newer_snapshot` |
| countersigned half filed (C3b2) | counterparty | `confirm` own; on a refusal sign `commitment-failure` | D-C3b-11 |
| a commitment ends `failed` or `cancelled` (C3b2) | each giver | `release` that commitment | D-C3b-12 |

**The release at the quote's expiry runs once per agreement** (review
P10). `agreement.get` (`agreement_ops.rs:508`) and `tick_conversation_at`
(`progress_ops.rs:341`) run on every call after the expiry. Today that
costs one local `get` that misses. With the hook client each call would be
one sibling call per slot commitment, and a log line whenever
`scheduling` answers an error. So after the first release that succeeds
for every own commitment, the counterparty's node writes
`released:<agreement>` in `ledger`, and both sites skip an agreement that
has it. A lost marker costs one more idempotent release. The C3b1 import
accepts the row when it names a held agreement.

`release` is never called for a commitment that has a settlement half: the
cascade leaves such a commitment on its own path (AG-36), and a cancel is
allowed only while every commitment is `promised` or `payment-requested`
(AG-9). D-C3b-12 checks it again before each per-commitment release.

**D-C3b-8. `catalog` after the move.** `app/availability.rs`, the
`AVAILABILITY` collection and `ensure_availability` (`app.rs:29`, `:68`),
the four rows in `api.rs:36` to `:39`, and the `availability` section of
its backup (`backup.rs:42`, `:75`) go. `SCHEMA_VERSION` 4 to 5, with one
sentence in its doc comment: "Version 5 moves slots to the scheduling
service." (the C3a review's R5 asks for this). `SECTION_AVAILABILITY`
stays in `roym_core::backup`, now used by `scheduling`.

**D-C3b-9. The scheduling backup.** `scheduling.export` writes the
sections `availability` and `reservations` with
`backup::collect_sections` and `export_signed_bundle`, as `catalog` does
(`backup.rs:40` to `:49`). `scheduling.import` checks, before it writes
anything:

- every row's id has a known prefix (`rsv:`, `seat:`, `key:`, `decided:`,
  `free:`) and a payload that parses;
- every `seat:` row names an `rsv:` row of the bundle with the same slot,
  seat and round, and every `rsv:` row has its `seat:` row;
- every `decided:` and `free:` row names an `rsv:` row of the bundle;
- `r` of each `rsv:` row equals `content_digest` of its key and
  generation; no two `rsv:` rows hold one (slot, seat, round);
- every `availability` row parses as `SlotRow` and its `slot_id` equals
  `derive_slot_id(listing_id, start_secs, end_secs)`.

It cannot check a reservation against an agreement: that is in
`transaction`'s bundle, and `scheduling` has no dependency on it. D-C3b-16
says how a restore keeps the two in step.

`scheduling.import` refuses an `rsv:` row whose id already exists in the
store with other content (`reservation-differs`). Import merges rows into
the store, as every Roym import does, so this is what stops a bundle from
moving a restored reservation to another seat.

**D-C3b-16. A restore keeps every agreed seat** (review P3). Today the seat
rows and the agreements are in one bundle and are checked together
(`sections.rs:498` to `:520`, `:600` to `:615`). After the move they are
two bundles, exported one after the other, and the Hub imports each file
on its own, in any order (`backup.ts:7` to `:38`). Two cases can then
happen:

| Case | Effect | How C3b handles it |
| --- | --- | --- |
| An agreement is restored, its reservation is not | the seat looks free: a second booking can take it — a double booking | **fixed**: `transaction.import` reserves again (below) |
| A reservation is restored, its agreement is not | one seat stays taken by nobody | **documented**: safe, and visible as one seat fewer; a backlog row |

What `transaction.import` does, after it has written its rows:

```text
for each restored agreement that is not ended
        (not declined, expired, not-formed, cancelled or closed),
    for each commitment that this node gives and that needs_reservation,
        with no settlement half, and whose state is promised (or no snapshot yet):
        reserve(agreement, c)            # idempotent: a restored reservation is found
        if the agreement holds the writer's countersigned half: confirm(agreement, c)
        a refusal or an error -> listed in the import output, not a failed import
ImportOutput gains: reservations_restored: u32, reservations_failed: [{agreement, commitment_id, reason}]
```

- When `scheduling` was imported first, every `reserve` finds its restored
  reservation and changes nothing.
- When `transaction` is imported first, the slots are not there yet, so
  each `reserve` is refused `slot-unavailable` and is listed. The fix is
  to import `scheduling`, then `transaction` again; an import of the same
  bundle again is a repeat (rows are merged, and every reserve is now
  found). The Hub's backup screen lists `scheduling` above `transaction`,
  and its import line for `transaction` shows the failed list with that
  sentence (`backup-reservations-failed`, a new message).
- `roymctl roym backup` imports in the fixed order, `scheduling` before
  `transaction`, and prints the failed list.
- **Export order:** `transaction` first, then `scheduling`. A booking that
  is made between the two exports then has its seat in the archive and no
  agreement: the safe case. The other order would give the double booking.
  The `EXPORT_ORDER` in `roymctl` gets a comment that says this.

**Why not also release a reservation with no agreement.** The reviewer
proposed this too: `transaction.import` lists every reservation through a
new local `reservation.list` and releases the ones no restored agreement
names. It is rejected for C3b. The case is safe (no double booking), it
needs one more hook, and a `transaction` import would then free seats of
agreements that a second, later `transaction` import would bring back. The
backlog row "A restored seat with no agreement stays taken" records it.

**D-C3b-10 (C3b2). The `commitment-failure` record.** A new module
`crates/roym_core/src/commitment_failure.rs` (with `commitment_failure/tests.rs`),
modelled on `decline.rs`:

```rust
pub const RECORD_COMMITMENT_FAILURE: &str = "commitment-failure";
pub const COMMITMENT_FAILURE_VERSION: u32 = 1;

#[serde(deny_unknown_fields)]
pub struct CommitmentFailurePayload {
    pub agreement: String,       // the quote version's record id; also the envelope subject
    pub conversation: String,
    pub commitment_id: String,
    pub reason: FailureReason,   // one value in v1: "reservation-lost"
}

pub fn verify_commitment_failure(envelope: &str, now: u64) -> FailureVerdict;
```

- `conversation` is not in the agreements doc's sketch (`:533` to
  `:537`). Every other deal record carries it, and `sync` files a card by
  its conversation; the change doc's Deviations record it.
- Signed by the person (the giver), with no session, through
  `signing::person_principal` as the writer's countersign is
  (`decide.rs:701`). No hidden field, no disclosure. The card refuses
  disclosures, as a decline card does (C2 Deviations, "Cards that must not
  carry disclosures").
- `("commitment-failure", 1)` in `RECORD_TYPES` and `CARD_TYPES` and the
  Hub's card registry, in one commit (`card.rs`'s
  `the_ui_card_registry_matches_this_crate` compares them). A
  `wire_words.rs` sample.

**D-C3b-11 (C3b2). The counterparty confirms.** A new `sync/confirm.rs`,
run in `sync_at` after the cards are filed and before the tick, for each
agreement of the conversation where:

- this node is the counterparty;
- the agreement row holds the writer's countersigned half
  (`quote_author.is_some()`), and its state is not terminal;
- a commitment that this node gives `needs_reservation`, its state in the
  agreement row is `promised`, and it has neither a
  `confirmed:<agreement>:<commitment_id>` nor a
  `failure:<agreement>:<commitment_id>` row in `ledger`.

The `promised` rule saves calls and keeps the meaning clear: a commitment
that ended `failed` or `cancelled` was released, and a confirm of it
could only be refused. `confirm` itself never takes a seat (D-C3b-5), so
even a wrong call cannot book a seat again. A test checks the rule
(`an_ended_commitment_is_never_confirmed`, 7.3).

**A late countersigned half after the counterparty's release.** The
counterparty releases its own reservations when the quote expires with no
countersign (D-C3b-7). The writer may still have countersigned before the
expiry, and that half may arrive later (agreements doc, "One way out of an
end state", `:620`). The deal then moves to `agreed`, and the confirm pass
gets `reservation-lost`: the counterparty signs a `commitment-failure`, as
for a lapse. The agreements doc already says "a refused confirm ends in
`reservation-lost`" for this case.

For each such commitment:

```text
match hooks.confirm(agreement, c):
  Ok(Confirmed)              -> put ledger confirmed:<agr>:<cid> { at_secs }
  Ok(Refused { reason })     -> make_failure(agr, c, reason)      # see below
  Err(transport or bug)      -> leave it; the next sync tries again
make_failure:
  stored = get ledger failure:<agr>:<cid>
  if none:
      sign the record
        signing error (NotEnrolled, Expired, Stale) -> write no row;
              count it in SyncOutput.unsent_failures; the next sync tries again
      create failure:<agr>:<cid> { envelope, record_id, message_id: None }
           (a lost race on the create reuses the stored row)
  if message_id is None: send the card; patch message_id
        send error -> keep the row; count it in SyncOutput.unsent_failures
```

**A failure that cannot be signed or sent is reported** (review P5).
Signing uses the person key with no session, and it can fail with
`NotEnrolled`, `Expired` or `Stale` (`resolve_principal_and_owner`,
`app.rs:288` to `:312`). No row is written then, so no unsigned record can
ever be sent, and the next sync confirms again (refused again) and tries to
sign again. `SyncOutput` (`sync.rs:46` to `:54`) has no error field today.
C3b2 adds `unsent_failures: u32`: the number of commitment failures this
sync could not sign or send. The Hub needs no change for it; `roymctl roym
transaction sync` prints the field with the others. The method shape
`transaction_sync` is regenerated.

**The deviation this rule makes** (review P4). The agreements doc says a
counterparty "that released its reservations at the expiry and then gets a
late countersigned half confirms as usual; a refused confirm ends in
`reservation-lost`" (`:620`). Because `confirm` never takes a seat (owner
answer, section 2.1), that confirm is always refused, also when the seat is
free. So such a deal always ends with the commitment `failed`
`reservation-lost`. The roym-agreements Deviations record it (section 9),
and `:620` is corrected at its close-out.

The record is made once: the row is stored before the send, and a repeat
sends the stored record again (the pattern of `step.add`,
`step_ops.rs:301` to `:350`, and C3a2's Deviations, "A repeat of a step
whose first send failed sends the card again").

The confirmed marker is a local row with no signature. It only saves a
local call on each later sync. A lost marker costs one more `confirm`,
which is idempotent.

**D-C3b-12 (C3b2). The writer files a `commitment-failure`.** A new
`sync/failure.rs` and one arm in `dispatch_known_card`
([sync.rs:281](../../../../crates/roym_transaction/src/app/sync.rs#L281)).
The writer files the card only when all of these hold; otherwise the card
is refused with a named reason (`refuse_card`, `sync.rs:422`):

1. `verify_commitment_failure` passes, and the card has no disclosures.
2. This node is the writer of that agreement; the agreement is `agreed`.
3. The commitment exists, its giver is the counterparty, and the envelope's
   signer is the counterparty DID of the agreement (agreements doc: "only
   from that commitment's giver").
4. The commitment `needs_reservation`.
5. No settlement half exists for that commitment (agreements doc, `:540`).

Then it runs `transition_at(Failure { commitment_id, reason })`
(`progress_ops`), which signs and sends the next snapshot with the
commitment `failed` and every other `promised` or `payment-requested`
commitment `cancelled` with `other-commitment-failed`
(`progress::apply`, `:236`; AG-36). A repeat card for a commitment already
`failed` gives `Ok(None)` and no new snapshot. The writer stores the
filed record as `failure:<agreement>:<commitment_id>` in its own `ledger`
(a create; a repeat finds it).

The counterparty's node files its own card back from the conversation as
its own (sent) card, with no state change.

**Per-commitment release.** `finalize_snapshot` (writer,
`progress_ops.rs:97`) and `apply_newer_snapshot` (counterparty,
`receipts.rs:256`) both call one function after they store a snapshot:

```rust
/// Releases this node's reservations whose commitments the snapshot shows
/// `failed` or `cancelled`, or all of them when the agreement is
/// `not-formed` or `cancelled`.
pub(crate) async fn release_ended<H: AppHost, R: Reservations>(
    host: &H, hooks: &R, terms: &DealTerms, own_role: DealRole,
    snapshot: &AgreementProgressPayload) -> ();
```

It skips a commitment whose snapshot entry lists a settlement half
(`halves` not empty), so `release` never frees a resource that was
claimed. This replaces the two agreement-state checks at
`progress_ops.rs:105` and `receipts.rs:290`.

**D-C3b-13 (C3b2). The Hub.**

- `cards/templates/commitment_failure.ts`: a title and one sentence per
  reason, with the commitment's description from the agreement's
  disclosures when the reader holds them (as `step.ts` names its
  commitment), else the commitment id. `render.ts`, `samples.json`, the
  registry, `templates.test.ts`, the gallery count in
  `tests/roym-hub.spec.ts`.
- `agreement_progress.ts:47` to `:54`: add `claimed`, `failed`,
  `cancelled`; remove `other-failed`. A commitment line with a `reason`
  adds the reason's sentence (`reservation-lost`,
  `other-commitment-failed`).
- New messages (en and hi): `commitment-failure-title`,
  `commitment-failure-reservation-lost`, `commitment-state-claimed`,
  `commitment-state-failed`, `commitment-state-cancelled`,
  `commitment-reason-reservation-lost`,
  `commitment-reason-other-commitment-failed`. Removed:
  `commitment-state-other-failed`. C3b1 adds `backup-bundle-scheduling`.

**D-C3b-14. The expiry value (C3b2, Q4).** `RESERVATION_TTL_SECS = 900`
in `crates/roym_scheduling/src/app/reservation.rs`, with a doc comment
that gives the reason: the writer confirms its own reservation at once,
and a counterparty's reservation waits only for the writer's countersign
(agreements doc, open question 4). It is not tied to the quote's expiry.

**D-C3b-15. One test checks every list of services** (owner answer,
section 2.1). D-C3b-2's table has about 15 places that list the Roym
services one by one. A `git grep` before the step finds them once; a test
keeps them right later. A new file
`crates/roym_core/src/services/tests.rs` (with `#[cfg(test)] mod tests;`
in `services.rs`, and `services.rs` beside `services/`, the module layout
rule) reads each file as text, by a path from `CARGO_MANIFEST_DIR`, and
checks that each service of `services::ALL` appears in it in the form that
file uses:

| File | Form checked for each service `s` (and `web` left out where noted) |
| --- | --- |
| `app/roym.toml` | a `[services.<s>]` table, and no other service table |
| `../../mise.toml` | `<s>` inside the `build:roym` task's `for pkg in …` line |
| `../../xtask/src/main.rs` | `"syneroym-roym-<s>"` |
| `../core/src/test_constants.rs` | `syneroym_roym_<s>.wasm` |
| `../../apps/roymctl/src/commands/roym/backup.rs` | `"<s>"` in both order lists (not `web`) |
| `../roym_web/ui/src/screens/backup.ts` | `exportMethod: "<s>.export"` (not `web`) |
| `../substrate/src/runtime/roym.rs` | `fn init_roym_<s>(` (all six exist today, `roym.rs:132` to `:388`) |
| `../roym_web/tests/dual_build_parity/helpers.rs` | `("<s>", test_constants::roym_<s>_wasm_path())`, `web` included (the list at `helpers.rs:890` to `:897` has all six today), and `"<s>" => self.` (the two match arms) |
| `../substrate/tests/e2e/node-boot.ts` | `'<s>'` inside the `ROYM_SERVICES = [` line |
| `../substrate/tests/common/roym.rs` | for each service whose `roym.toml` `visibility` is not `public`: `"<s>" => Visibility::<Private or Internal>` in `service_visibility`, matching the manifest |
| `../substrate/tests/roym_restore_e2e.rs` | `"<s>"` in `EXPORT_SERVICES` and in `IMPORT_ORDER` (not `web`) |
| `../substrate/tests/roym_trust_e2e.rs` | `"<s>"` in `DATA_SERVICES` (not `web`) |

- Each row is one small check with a message that names the file and the
  missing service, so a failure says what to add.
- The checks are text checks on purpose. The test needs no new
  dependency: `toml` is already a dev-dependency of `roym_core`
  (`router.rs:155` uses it), and the rest is `str::contains` on one line or
  one block. It follows the pattern of the Hub card registry test, which
  also reads a file of another crate as text (`card.rs`,
  `the_ui_card_registry_matches_this_crate`).
- The test is written first, in C3b1 step 4, **before** the new service
  is added: on `main`'s six services it must pass. Then `scheduling` is
  added to `ALL`, the test fails and names every place still missing, and
  the step adds them until it passes.
- `roymctl`'s signing list (`signing.rs:13`) is not checked: it lists the
  services that sign, and `scheduling` signs nothing (D-C3b-1).

## 6. Step-by-step changes

Each step builds, and its tests pass, before the next starts.

### 6.1 C3b1

#### Step 1 — Commit this plan

Done with this commit.

#### Step 2 — Hook types and the manifest (`roym_core`, pure)

- `method/scheduling.rs` (D-C3b-4); move `SlotKeyInput`, `SlotRow` from
  `method/catalog.rs` and fix their imports (`roym_catalog/src/api.rs:10`,
  `availability.rs:9`, `hold.rs:11`, `quote_ops.rs`).
- Manifest: `Hook::Prepare`, the new hook rule, `has_prepare_hook`, and
  `"prepare"` in `scheduling.json`'s hooks (D-C3b-3). The `service` key
  waits for step 4: the loader checks it against `services::ALL`, which
  gets `scheduling` only in step 4. Tests: section 7.1.

#### Step 3 — The scheduling crate, alone

- The crate of D-C3b-1. `availability.rs` moves with `git mv`-like history
  (copy, then delete from `catalog` in step 5), so the diff shows the move.
- `reservation.rs` and `rows.rs` (D-C3b-5, no lapse), `prepare.rs` (Q3),
  `backup.rs` (D-C3b-9). Unit and service tests of section 7.2 on
  `roym_core::testing::TestHost`.
- `cargo component build --release --target wasm32-wasip2 -p syneroym-roym-scheduling`
  builds the component.
- **Measure its size now, before step 4 wires it in** (owner answer,
  section 2.1). Record the size of `syneroym_roym_scheduling.wasm` and of
  `syneroym_roym_profile.wasm` from the same `target/` folder in the C3b1
  status row. `profile` is the nearest service in shape (the same host
  glue, a small method set): 772,732 bytes at C3a2 ([task.md](task.md),
  section 1). If `scheduling` is more than 40 KB larger than `profile`,
  stop and tell the owner before step 4, with the size of each crate it
  links (`cargo tree -p syneroym-roym-scheduling --target wasm32-wasip2 -e normal`).

#### Step 4 — Wire the seventh service

- First the service-list test of D-C3b-15, on today's six services. It
  must pass before anything else in this step.
- Every row of the table in D-C3b-2, after the `git grep` check it names.
  The test of D-C3b-15 then fails until every place has `scheduling`.
  `SCHEDULING`, `ALL`, `SIBLINGS`, `by_name`, the router rows and
  `roym.toml` change in one commit, because the router's tests compare
  them (section 2.2, point 9).
- The manifest's `service` key, its loader rules and `hook_service`
  (D-C3b-3), and `"service": "scheduling"` in `scheduling.json`.
- `roym.toml` gets the service. `transaction` still depends on `catalog`
  here; step 5 swaps it.
- Parity: one scenario per `availability.*` method now answers from
  `scheduling` on both builds (the existing scenarios in
  `dual_build_parity/catalog.rs:270` to `:346` and `deal_fixtures.rs:153`
  pass unchanged through the router). `wire_origin.rs:95` to `:102` keeps
  its rows; it now proves `scheduling` refuses them off the node.
- `method_contract`: the scheduling table, its shapes
  (`method_shapes/scheduling_*.json`, regenerated with
  `UPDATE_METHOD_SHAPES=1`), and the move of the availability shapes from
  `shapes_catalog.rs:73` to `:99` to a new `shapes_scheduling.rs`.
- `mise run build:roym`, then the parity and method-contract suites.

#### Step 5 — `transaction` calls the hooks; `catalog` loses slots

- `app/hooks.rs` (D-C3b-6); delete `hold.rs`. Every site of section 2.2,
  point 3, in the order of D-C3b-7. `prepare_terms` calls `prepare`.
- `LedgerKind::Released` for the marker of D-C3b-7, with its import check.
- `LedgerKind::Hold` and `::Seat` go (`ledger.rs:11`, `:12`), with
  `HoldLedgerRow`, `SeatLedgerRow` and their import checks
  (`sections.rs:498` to `:520`, `:600` to `:615`). `SCHEMA_VERSION` 6 to 7,
  with its doc sentence.
- `roym.toml` and `wire_roym_topology`: `transaction` depends on
  `conversation` and `scheduling` (Q8).
- `transaction.import` reserves again after it writes its rows, and its
  output gains `reservations_restored` and `reservations_failed`
  (D-C3b-16); the code goes in a new `backup/reservations.rs`, because
  `backup.rs` is at 770 lines. Regenerate the `transaction_import` method
  shape.
- `catalog`: D-C3b-8.
- The tests that read `hold:` and `seat:` rows change to read the fake's
  state or the outcome (section 7.3): `tests/accept.rs:289` to `:474`,
  `tests/cancel.rs:296` to `:324`, `tests/decide.rs:299` to `:419`,
  `tests/recover.rs:242` to `:655`, `tests/refuse/writer.rs:391` to `:445`,
  `backup/tests.rs:118` to `:288`.
- After this step,
  `git grep -n 'ServiceInSlot\|slot_id\|availability\.' crates/roym_transaction/src crates/roym_catalog/src`
  shows only the `fulfil:` kind rule (`sections.rs:378`) and tests (exit
  criterion).

#### Step 6 — `roymctl`, Hub and two-node

- `roymctl roym backup` orders (D-C3b-2).
- Hub: the backup screen's sixth bundle, listed above `transaction`;
  `backup-bundle-scheduling` and `backup-reservations-failed` ("Some booked
  slots could not be restored. Import the Scheduling file, then import the
  Transaction file again.") in `en.ftl` and `hi.ftl`; the backup screen's
  test, including the failed-list line (D-C3b-16).
- Two-node: `roym_booking_e2e.rs` passes unchanged (its calls go through
  the router). `roym_restore_e2e.rs` exports and imports each service by
  name from `EXPORT_SERVICES` and `IMPORT_ORDER` (`:27`, `:28`; the
  functions at `:33` to `:42`): add `scheduling` in the orders of
  D-C3b-16, and check after the import that the booked seat is still taken
  (a second booking of the last seat ends `not-formed` with `slot-taken`).
- `roym_app_e2e.rs:10`, `:149` (comments that say six services).

#### Step 7 — Sizes, docs and close-out of C3b1

- `mise run build:roym`; component sizes before and after (section 7.9).
- Deviations, backlog rows, living docs (sections 8 and 9). The extensions
  doc's Tasks: tick "Move scheduling out of `catalog` and `transaction`".
- The C3b1 row in status.md records the demo answer (Q6).
- `mise run verify` with no `--skip`.

### 6.2 C3b2

#### Step 1 — Expiry and lapse in `scheduling`

- `RESERVATION_TTL_SECS` (D-C3b-14); `expires_at_secs` set at reserve;
  `lapse` and its call in `is_free`; `confirm` refuses `reservation-lost`
  after a lapse and after a release, and never takes a seat (D-C3b-5).
  Tests: 7.2, "lapse".

#### Step 2 — The record (`roym_core`, pure)

- `commitment_failure.rs` and its tests (D-C3b-10); `RECORD_TYPES`,
  `CARD_TYPES` and the Hub registry in one commit; a `wire_words.rs`
  sample.

#### Step 3 — The counterparty's confirm and failure

- `sync/confirm.rs` (D-C3b-11); `LedgerKind::Confirmed` and `::Failure`;
  the call in `sync_at` (`sync.rs:69`). Service tests: 7.3.

#### Step 4 — The writer's filing and the per-commitment release

- `sync/failure.rs`, the `dispatch_known_card` arm (D-C3b-12);
  `release_ended` in `progress_ops` and `receipts`.
- Backup: the `failure:` and `confirmed:` rows in `transaction`'s import
  (a `failure:` row's envelope verifies and its signer is the giver of that
  commitment of a quote in the same bundle; a `confirmed:` row names a held
  agreement and commitment); the `commitment-failure` arm in the re-verify
  match ([backup.rs:735](../../../../crates/roym_transaction/src/app/backup.rs#L735)
  area) and in `rebuild_card_data` (`backup/disclosures.rs`). The test
  that C3a2 wrote for the decline card is the pattern.

#### Step 5 — Hub

- D-C3b-13. `templates.test.ts`, `hindi-screens.test.ts`, the gallery count.

#### Step 6 — Two-node, sizes, docs and close-out of C3b2

- `crates/substrate/tests/roym_reservation_e2e.rs` (section 7.4).
- As C3b1 step 7. Tick "Reserve, confirm and release hooks, with expiry,
  idempotency and owner-side cleanup" in the extensions doc's Tasks.

## 7. Tests

### 7.1 Unit tests (`roym_core`)

Manifest (`extension/tests.rs`): `scheduling` loads with `service` and four
hooks; failing loads for: hooks with `service` absent; `service` with no
hooks; `service` that names no Roym service; `service = "web"`; `prepare`
alone; two of the three reserve hooks. `hook_service` of `scheduling` is
`services::SCHEDULING`; `one-shot` has none. `services::by_name` finds each
of `ALL` and nothing else.

Service lists (`services/tests.rs`, D-C3b-15): one test per file of its
table, for example `every_service_is_in_the_roym_build_task`,
`every_service_has_a_backup_line_in_the_hub`. Each passes on six services
before the new service is added, and on seven after.

Router (`router.rs` tests, unchanged in shape): `availability.set` routes to
`scheduling`; `reservation.reserve` routes nowhere; the manifest tests pass
with seven services.

C3b2, `commitment_failure/tests.rs`: `validate` (empty ids, an unknown
reason); `verify_commitment_failure` with a good, a tampered and a
wrong-type envelope; the subject equals `agreement`.

### 7.2 Scheduling service tests (`crates/roym_scheduling/src/app/tests/`)

On `TestHost`, with `_at(now)` entry points.

`reserve.rs` (C3b1):

- `the_last_seat_goes_to_one_of_two_reserves`: capacity 1, two keys, one
  `Held`, one `slot-taken`.
- `two_reserves_of_one_key_return_one_reservation` (EX-21, review P1): the
  first caller's claim is written between the second caller's `seats` read
  and its `create` (call the claim function with the stale `seats` list,
  if `TestHost` has no hook to run a write between two calls). Both get
  the same reservation; there is one `rsv:` row and one `seat:` row.
- `a_seat_query_counts_only_seat_rows`: capacity 2, one booking: a second
  key still gets `Held` (the `rsv:` row, which also has `slot_id`, is not
  counted as a seat).
- `a_repeat_reserve_returns_the_same_reservation` (EX-21), also after a
  crash between the claim and the `key:` put (`crash_on_write("reservations",
  "key:")`, then a repeat finds the `rsv:` row).
- `release_frees_the_seat_and_a_second_release_changes_nothing` (EX-21).
- `release_of_an_unknown_key_succeeds`.
- `release_frees_a_confirmed_reservation` (agreements doc, `:745`).
- `a_second_release_cannot_free_a_seat_taken_again`: reserve A, release A,
  reserve B on the same seat, release A again: B still holds the seat
  (section 2.2, point 6).
- `a_reserve_after_release_takes_a_new_generation`.
- `capacity_is_capped_at_64`; `capacity_0_is_slot_unavailable`; a missing
  slot is `slot-unavailable`.
- `confirm_of_an_unknown_key_is_reservation_unknown`; a repeat confirm
  succeeds.

`prepare.rs` (C3b1): the three refusals and the window fill, one case each,
with the same text as today's `quote.set` errors.

`backup.rs` (C3b1): export then import keeps a held and a confirmed
reservation and its seat; one refused case per rule of D-C3b-9; an `rsv:`
row that exists with other content is refused `reservation-differs`.

`transaction` restore (C3b1, `tests/restore_reservations.rs`, D-C3b-16):
`import_reserves_again_the_seat_of_an_agreed_deal` (the fake's store is
empty: one reserve and one confirm, `reservations_restored: 1`);
`import_finds_a_restored_reservation` (no change);
`import_before_scheduling_lists_the_failed_seats` (`slot-unavailable`
listed, the import itself succeeds); `an_ended_agreement_is_not_reserved`.

`lapse.rs` (C3b2):

- `an_unconfirmed_reservation_lapses_when_its_seat_is_needed`: reserve A at
  `t`, reserve B at `t + 901`: B holds the seat, A's `decided:` is
  `lapsed`.
- `a_reservation_before_its_expiry_is_not_taken`: the same at `t + 899`:
  `slot-taken`.
- `a_confirmed_reservation_never_lapses`.
- `confirm_after_the_expiry_succeeds_while_nobody_took_the_seat` (EX-22).
- `confirm_after_a_takeover_is_reservation_lost` (EX-22).
- `confirm_after_a_release_is_reservation_lost_and_takes_no_seat`: reserve,
  release, confirm: refused `reservation-lost`, and the slot has no held
  seat after it (agreements doc, "One way out of an end state", `:620`;
  owner answer on `confirm`).
- `confirm_never_creates_a_seat_row`: across every confirm case above, the
  count of `seat:` rows does not change.
- `two_takers_of_one_lapsed_seat_give_it_to_one`.

### 7.3 In-crate service tests (`crates/roym_transaction/src/app/tests/`)

C3b1, with the fake hooks of D-C3b-6:

- The existing slot cases keep their meaning, with new checks in place of
  `hold:` and `seat:` reads: `tests/accept.rs` (the counterparty reserves
  before it signs; a refusal answers `reserve-failed` with
  `data.reason = "slot-taken"` and signs nothing; two commitments where the
  second refuses: the first is released), `tests/decide.rs` (the writer's
  reserve, confirm, not-formed and release), `tests/recover.rs` (every
  crash point, the expiry recovery, `RefusingConfirmReservations` with the
  new trait), `tests/cancel.rs` (cancel releases the writer's own),
  `tests/refuse/writer.rs` (a refusal of a superseded version releases).
- `quote_set_fills_the_window_from_the_prepare_hook`, and one refusal
  passed through with its code.
- `quote_measure_refuses_the_same_codes_as_quote_set` (review P7): the
  three `prepare` refusals, each through both methods.
- `an_extension_reason_that_is_not_a_code_is_an_error` (review P6): the
  fake answers a 65-byte code and an upper-case code; `agreement.accept`
  answers an internal error and signs nothing; a good code passes through
  as `reserve-failed` with `data.reason`.
- `the_expiry_release_runs_once_per_agreement` (review P10): after the
  quote's expiry, a first `agreement.get` releases (one hook call) and
  writes `released:<agreement>`; a second `agreement.get` and a sync make
  no hook call.
- `scheduling_and_one_shot_pass_the_same_states` (EX-4): one agreement with
  a scheduling commitment and one with a one-shot commitment; both go
  `accepted` → `agreed` → `closed` with the same snapshot states; only the
  scheduling one calls the hooks (the fake counts calls).
- `a_commitment_with_no_reserve_hook_calls_no_hook`.
- `the_hooks_go_to_the_service_the_manifest_names`: the fake records the
  service name of each call.
- Admission: the existing table test needs no change; `transaction` gets
  no new method.

C3b2, new file `tests/commitment_failure.rs`:

- `a_refused_confirm_signs_one_failure_and_sends_it` (AG-16): the
  counterparty files the countersigned half; the fake refuses `confirm`
  with `reservation-lost`; one `failure:` row and one card; a second sync
  sends nothing new.
- `a_send_error_resends_the_stored_failure`: the first send fails; the next
  sync sends the same record id.
- `a_transport_error_on_confirm_retries_on_the_next_sync`, and writes no
  failure.
- `a_failure_that_cannot_be_signed_is_retried_and_reported` (review P5):
  the signing certificate is expired; the sync writes no `failure:` row,
  answers `unsent_failures: 1`, and sends nothing; after a new certificate
  is installed, the next sync signs and sends one record.
- `a_late_half_after_the_expiry_release_gives_a_commitment_failure`
  (review P4): the counterparty accepts; its clock passes the quote's
  expiry and `agreement.get` releases its seat; then the writer's
  countersigned half (signed before the expiry) is filed. The deal moves
  to `agreed`, the confirm is refused `reservation-lost`, and one failure
  is signed and sent. The fake models the real rule: a released key
  refuses `confirm`.
- `a_confirmed_commitment_is_not_confirmed_again` (the marker row).
- `an_ended_commitment_is_never_confirmed`: a counterparty commitment that
  the snapshot shows `cancelled`, with no marker row, gets no `confirm`
  call (the fake counts calls).
- `the_writer_marks_the_commitment_failed_and_cancels_the_others` (AG-16,
  AG-36): a three-commitment agreement (the counterparty's slot, the
  author's service, a currency commitment the counterparty pays); the
  writer files the failure; its next snapshot has `c1 failed
  reservation-lost`, `c2 cancelled other-commitment-failed`, `c3 cancelled
  other-commitment-failed`, and the agreement's state is `closed`, because
  every commitment is now final (`progress.rs:243`; review P11).
- `a_commitment_with_a_half_keeps_its_path`: c3's payer half is filed
  first; c3 stays `claimed`.
- `the_writer_refuses_a_failure_from_the_wrong_signer`, `…_for_a_commitment_with_no_reserve_hook`,
  `…_with_a_settlement_half`, `…_before_the_agreement_formed`, `…_with_disclosures`.
- `a_repeat_failure_card_makes_no_second_snapshot`.
- `each_giver_releases_the_reservations_of_ended_commitments` (AG-36): the
  writer's own slot in a cancelled commitment is released on the writer's
  node; the counterparty releases its failed one when it files the
  snapshot; a commitment with a half is not released.
- Backup: export then import keeps the `failure:` row and the card, and
  `verified: true`; a `failure:` row whose signer is not the giver is
  refused.

### 7.4 Two-node (`crates/substrate/tests`)

- C3b1: `roym_booking_e2e.rs` (three nodes, the last-seat race), unchanged,
  now through `scheduling` (AG-14, EX-6, VX-10). `roym_step_e2e.rs` and
  `roym_restore_e2e.rs` (with the step of 6.1 step 6).
- C3b2: `roym_reservation_e2e.rs` (Q5 (a)).
  `the_writer_marks_a_failed_slot_and_both_nodes_release`:
  1. Node Y has a slot of capacity 1. Node X sends Y a quote with two
     commitments: c1, Y gives a lesson in that slot (scheduling); c2, X
     pays Y (currency). X is the writer.
  2. Y accepts: Y's node reserves its own seat. Both nodes sync until the
     agreement is `agreed` on both. Y's node confirms its seat (the marker
     row exists).
  3. The test signs a `commitment-failure` for c1 with Y's person key and
     sends it from Y's node to X as a card (the hand-signed pattern of C1's
     AG-12 test). It cannot make Y's confirm fail for real: no node clock
     can move 15 minutes (Q5).
  4. X syncs: its next snapshot shows c1 `failed` `reservation-lost` and c2
     `cancelled` `other-commitment-failed`. Y syncs and files it.
  5. Y's seat is free again: a second agreement for the same slot reaches
     `agreed`.
  Ports from `alloc_ports`. Q5 (b) would replace step 3 with a real
  2-second expiry.

### 7.5 Parity (both builds agree)

`mise run build:roym` first. C3b1: the availability scenarios now hit
`scheduling` on both builds; the booking scenarios
(`dual_build_parity/booking.rs`) pass; a new
`dual_build_parity/scheduling.rs` with reserve, a repeat reserve, confirm,
release and a second release, with equal outputs. `bundles.rs`' method and
section lists get the scheduling bundle. C3b2: a lapse needs a clock the
harness cannot set, so parity covers the `commitment-failure` card filing
only (a scenario that files a hand-signed failure on both builds).

### 7.6 Hub unit tests (`npm test`) and Playwright

- C3b1: the backup screen lists six bundles (`backup.test.ts`, or the
  screen test that exists).
- C3b2: `templates.test.ts` (`renderCommitmentFailure`; the progress card
  with `failed`, `cancelled`, `claimed` and both reasons);
  `hindi-screens.test.ts`; `catalog.test.ts` (no orphan
  `commitment-state-other-failed`).
- Playwright: the gallery count (C3b2). Stories 40, 50 and 60 pass
  unchanged in both parts.

### 7.7 Not tested

- A reservation that lapses on a real node in test time (Q5).
- Two extensions with reserve hooks in one agreement: only scheduling
  ships. The fake hooks run a two-extension case in 7.3
  (`the_hooks_go_to_the_service_the_manifest_names`) with a test-only
  manifest.

### 7.8 Size and time risks

Production files near the 800-line cap that this slice touches:
`backup.rs` (770: one re-verify arm in C3b2, nothing else), `quote_ops.rs`
(751: shrinks by about 40 when `validate_and_apply_slot` goes),
`decide.rs` (742: signatures change, no growth), `backup/sections.rs`
(734: shrinks by about 50 when the hold and seat checks go; the C3b2 rows
go in a new `backup/failure.rs`), `payment_ops.rs` (733: not touched),
`sync.rs` (722: one match arm and one call; the code goes in
`sync/confirm.rs` and `sync/failure.rs`). `agreement_ops.rs` (563) and
`progress_ops.rs` (358) change by a few lines.

Test files: `tests/decide.rs` (706), `tests/accept.rs` (690),
`tests/recover.rs` (668), `backup/tests.rs` (690). Their slot cases change
in place and should not grow; a new case goes in a new file.

`substrate/src/runtime/roym.rs` gains one `init_roym_*` function of about
40 lines; check its length first, and put the new function in a sibling
file if it would pass 800.

Time: the parity and method-contract harnesses build one more native
service and load one more component. Expect a few seconds more per run.

### 7.9 Component size

[task.md](task.md) section 1: each part reports every component's size,
before and after, from one build folder, and C3b reports `scheduling`
alone. Expected: C3b1 makes `catalog` and `transaction` smaller (the slot
code leaves) and adds `scheduling` (its own size, likely near `profile`'s
772,732 bytes, because both link the same host glue). The first
measurement is in C3b1 step 3, before the wiring; its stop rule is there.
The size table at close-out repeats the measurement on the finished
branch. C3b2 grows
`transaction` (new record code, limit +15%) and `scheduling` (lapse, limit
+40 KB). Over a limit: stop and tell the owner.

### 7.10 Hindi text

New messages for the owner to review before each merge. C3b1:
`backup-bundle-scheduling`, `backup-reservations-failed`. C3b2: the seven of D-C3b-13, and the removal of
`commitment-state-other-failed`. Each has a Devanagari letter
(`catalog.test.ts:119` to `:131`).

### 7.11 How C3b avoids the kinds of findings in the C1, C2 and C3a reviews

- **A step that does not finish after a crash** (C1 R2, R21): the hook
  calls keep the writer's fixed order, and every new state (`free:`,
  `decided:`, `failure:`) is a fence row, so a repeat finds it.
- **Made once broken by a resend** (C1 R4, C3a2's resend deviation):
  D-C3b-11 stores before it sends and resends the stored record.
- **A delete that frees the wrong row** (C1 P21): D-C3b-5 never deletes a
  seat row.
- **Errors classed by text** (C1 R3): `ExtensionHooks` branches on
  `SiblingError`'s kind and on `data.code`, never on a message.
- **Silent failures** (C2 R21): every new `get_row` matches `Ok(None)` and
  `Err` apart; a `release` error is logged, not dropped with `let _`
  (today's sites drop it: `progress_ops.rs:107`, `receipts.rs:292`).
- **Who signed is not checked** (C2 R24, R26): D-C3b-12 rule 3.
- **Import trusts the bundle** (C1 R13, C2 R31): D-C3b-9 and C3b2 step 4
  check every row.
- **A service list left behind** (new risk here): D-C3b-2's table, its
  `git grep` check, and the test of D-C3b-15, which keeps the lists right
  after this slice too.
- **A size surprise found at the end** (new risk here): the new component
  is measured in C3b1 step 3, before it is wired in.
- **A hook that books a seat by accident**: `confirm` never creates a seat
  row (D-C3b-5, `confirm_never_creates_a_seat_row`).
- **Tests that prove less than their names** (C3a R3, R4): each name in
  section 7 says the outcome it checks; `git grep 'fn <name>'` before a
  change doc names it.
- **Living docs left false** (C1 R6, C2 R40, C3a R6): section 9 names the
  lines.

## 8. Deferred backlog

[deferred-backlog.md](../../deferred-backlog.md):

| Row | C3b action |
| --- | --- |
| "Extension services cannot receive events or take calls from a counterparty" (`:548`) | Unchanged. Scheduling uses local hook calls only. |
| "One Roym output carries another node's card data as an untyped `Value`" (`:537`) | Not targeted. No file name of the row changes (`PeerStep` is gone already, section 2.2 point 2). |
| "`RECORD_TYPES` is not read by any verifier" | Not targeted. C3b2 adds `commitment-failure` to the table; note it in the row. |
| "Hub transaction action panel …" | Unchanged. The failure card is drawn, and needs no action. |
| New: "A restored seat with no agreement stays taken" (D-C3b-16) | C3b1 adds it. A reservation restored from the `scheduling` bundle whose agreement is not in the restored `transaction` bundle holds its seat for good (a confirmed one never lapses). It is safe (no double booking) and shows as one seat fewer; the workaround is to raise the slot's capacity. Target: TBD, with a composed signed bundle (the Hub's backup comment, `backup.ts:5`). |
| New: "A reservation lapses only when another booking needs its seat" (Q4 (a)) | C3b2 adds it, as a note on the design, not a defect: the seat is free for every new booking, and nothing frees it eagerly. Target: none unless a screen shows free seats. |
| New: "A reservation's expiry cannot be tested across two nodes" (Q5 (a)) | C3b2 adds it. Target: TBD, when nodes get a test clock. |
| New: "The receiver of a commitment cannot own its reservation" | C3b1 adds it from the agreements doc ("Who reserves", `:763`): a visible `reserved_by` field if M08F needs it. Target: M08F. |

No new `TODO` or `FIXME` marker is planned. If one is added, it gets a row
in "Open in-code markers".

## 9. Living docs to update at close-out

Each PR fixes the statements it makes false. C8 does the full pass.

- `AGENTS.md:263` (Architecture, Roym product): "a SynApp of six services"
  becomes seven, with `scheduling` in the crate list and one sentence on
  what it holds (C3b1).
- `docs/system-architecture.md:559` ("The six Roym services use this to be
  local-only": seven) (C3b1).
- `docs/system-architecture.md:1143`, `:1180`, `:3096` (six services; add
  `scheduling`, its collections and that `transaction` reaches it through
  the reserve hooks) (C3b1); the card types count near `:3123`
  (`commitment-failure`, C3b2).
- `docs/developer-guide.md:1627`, `:1661` (six services); the method
  tables: `availability.*` under `scheduling`, the `reservation.*` hooks as
  local-only, the scheduling bundle; the "Roym extensions" section gets the
  manifest keys `service` and `prepare` (C3b1); `commitment-failure` in the
  card list (C3b2).
- `docs/system-requirements-spec.md:718` ("The transaction service of the
  quote author arbitrates accepted quotes against catalog availability"):
  the giver's node reserves through the scheduling extension (C3b1).
- `docs/roym-integrated-experience-spec.md:101` (catalog holds
  availability): slots move to the scheduling service; the slot rule
  `D-C8-2`; the Records and card tables (C3b2).
- `docs/demo/demo.md:141`, `:1147`, `:1169` (six services).
- `docs/TERMINOLOGY.md`: reservation, extension service, hook (check which
  exist first).
- `docs/planning/traceability-matrix.md`: the rows of section 3.
- Change docs (not living docs, but owned here):
  - roym-extensions `## Deviations`: the manifest keys `service` and
    `prepare` (Q2, Q3); lazy expiry (Q4); the `availability.*` names kept
    (Q7); EX-5's text (each giver reserves on its own node, already
    recorded by the agreements doc's "Differences" table, `:1066`); EX-21's
    "release also frees a confirmed reservation" (`:1068`); EX-22 is
    narrower: a confirm after a release is refused even when the seat is
    free, because `confirm` never takes a seat (owner answer, 2026-11-21).
  - roym-agreements `## Deviations`: AG-14 and AG-15 now run through the
    scheduling extension (closes the C1 note at `:1233`); AG-36's snapshot
    and release part (closes the C1 note at `:1243`); `conversation` in
    `CommitmentFailurePayload` (D-C3b-10); AG-16's test kind (Q5); a late
    countersigned half after the counterparty's expiry release ends in
    `reservation-lost`, because `confirm` never takes a seat (owner answer
    on `confirm`; `:620` is corrected at close-out); `transaction.import`
    reserves again the seats of restored agreements (D-C3b-16).
  - roym-value-exchange `## Deviations`: VX-10 met through the hook (no
    text change).
  - [task.md](task.md): the C3b result line (as C3a's), and section 1's
    size table for C3b1 and C3b2.

## 10. Exit criteria

C3b1:

- [x] Every C3b1 row of section 3 names a test that exists and passes.
- [x] `git grep -n 'ServiceInSlot\|slot_id\|availability\.\|MAX_SLOT_CAPACITY'`
  over `crates/roym_transaction/src` and `crates/roym_catalog/src`, tests
  left out, shows only the `fulfil:` kind rule (`backup/sections.rs`).
  `transaction` has no `call_sibling` to `CATALOG`.
- [x] No `transaction` code names `services::SCHEDULING` or the string
  `"scheduling"`; it reaches the service through the manifest (Q2).
- [x] `roym_booking_e2e` (the last-seat race) passes through the
  scheduling service.
- [x] Stories 40, 50 and 60 and the Hub cases pass unchanged. (The Hub unit cases pass locally; Playwright passed in CI on 2f814435.)
- [x] Parity and method-contract pass after `mise run build:roym`, with
  seven services.
- [x] The service-list test of D-C3b-15 passes, and it passed on six
  services before `scheduling` was added (the step 4 commit order shows
  it).
- [x] The size of `scheduling` was recorded in step 3, before the wiring,
  next to `profile`'s.
- [x] Component sizes recorded, with `scheduling` alone, and within limits.
- [x] Deviations, backlog rows and living docs done.
- [x] A restore of the two bundles in either order ends with every agreed
  seat taken, after the documented repeat (D-C3b-16).
- [x] Owner reviewed `backup-bundle-scheduling` and
  `backup-reservations-failed` in Hindi (and the changed `backup-bundle-catalog`).
- [x] `mise run verify` passes with no `--skip`. (The owner accepted CI in place of a local run, as for C3a1 and C3a2.)

C3b2:

- [x] Every C3b2 row of section 3 names a test that exists and passes.
- [x] A refused confirm always ends in one signed `commitment-failure`,
  also after a send error (`a_send_error_resends_the_stored_failure`).
- [x] Every release site calls `release_ended` or the hook client; no
  `let _ =` drops a release error without a log line. (No Roym service can log, so a release that fails at a snapshot is tried again by the next sync; see the extensions change, Deviations.)
- [x] The two-node test of 7.4 passes with no hardcoded port.
- [x] Component sizes recorded and within limits.
- [x] Owner reviewed the Hindi messages of 7.10.
- [x] `mise run verify` passes with no `--skip`. (The owner accepted CI on 82c45089 in place of a local run, as for C3b1.)

## 11. What C3b does not do

- Events to an extension service, deal grants, or any call from another
  node into `scheduling` (EX-15, EX-16; moved out of M08C).
- `instant` and "Agree as listed" (C3c), which use these hooks.
- A Hub action to book, start, confirm or release; the Hub's slot screens
  stay as they are.
- A screen that shows free seats.
- A reservation owned by the receiver (`reserved_by`, M08F).
- A composed backup bundle across services.
- Any wire-reachable `transaction` or `scheduling` method.
- A signed "not received" record (agreements doc, open question 15).
