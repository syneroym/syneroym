# M08C slice C1 — Commitment model and deal records: implementation plan

*Reader: the developer (or AI) who implements slice C1, and the project
owner, who answers the questions in section 2.1. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan written, 2026-11-15, by Claude Opus 5.5. The owner accepted
the recommendation of every question, Q1 to Q6, on 2026-11-16 (section
2.1). No plan review yet.**

**Size and PRs.** C1 is the largest slice of M08C. This plan splits it in
two parts, and each part is one PR that passes `mise run verify` (Q1,
accepted):

- **C1a — Deal shapes (L).** Every deal record gets its new shape, in
  place, and every caller uses it: the Rust service, the Hub, `roymctl`,
  the parity harness, the two-node tests and the Playwright tests. The
  writer's decision runs in its final order, with no crash recovery yet.
  Branch: `feat/m08c-c1-commitment-model` (this branch; it holds the plan
  commit).
- **C1b — Single writer and signed decline (M).** The version checks, the
  refusal snapshots, the crash recovery of the writer's decision, the
  restore rules, and the signed `deal-decline`. Branch:
  `feat/m08c-c1b-single-writer`, cut from `main` after C1a merges.

**Base.** Every file path and line number below was checked on `main` at
`48ea989c` (2026-11-15). Links are relative to this folder.

**Dependencies.** [task.md](task.md) section 4: C1 depends on no other
slice. C0 (docs only) may run at the same time in a git worktree. C1 does
not touch `docs/decisions/` or the message-forwarding change doc, and it
changes only the C1 row of [status.md](status.md). The second-tool review
of the agreements doc (task.md section 3) is partly done: rounds 4 and 5 were
reviewed by Claude Sonnet 5.5, but the fixes of rounds 4 and 5 (P28 to P38)
are not yet confirmed by a different tool. Section 2.2, point 1, lists what
this plan found in them; Q5 is the one that needs the owner.

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish each PR (background job, about 20
    minutes, no `--skip` in the final run).
  - No planning ids in code, comments or test names: no "M08C", "C1a",
    "AG-14", "D-C1-4", "Q3" or review ids (`P4`, `R7`) in `.rs`, `.ts`,
    `.wit`, `.ftl`, `.json` files.
  - Comments never cite size rules.
  - **File size.** Production `.rs` files at most 800 lines; new test `.rs`
    files at most 800 lines; a file in `xtask/oversized-test-files.txt` is
    capped at its recorded count (`dual_build_parity/transaction.rs` is at
    846, `helpers.rs` at 1290). The gate reads only `.rs` files
    (`xtask/src/file_lengths.rs:398`), so `messages.ts` (1016 lines) and
    `roym-hub.spec.ts` (1535 lines) are not checked; do not make them
    longer without need.
  - **Function size.** `clippy::too_many_lines` at 100. Match arms short.
  - **Module layout.** `foo.rs` beside `foo/`, never `mod.rs` in `src/`.
  - The import clean-up pass, the deferred backlog, the living-docs
    close-out, the demo close-out and the ratchet updates.
  - Every `roym_*` crate change needs `mise run build:roym` before you
    trust a parity run ([task.md](task.md) section 1).
  - Do not commit on `main`.
- [task.md](task.md): section 1 (Z1 and the component size limit), section
  2 (facts), "C1", section 4, section 5 (criteria finished later), section
  7 (backlog rows).
- [roym-agreements change doc](../../changes/roym-agreements/change.md):
  all of it. This plan follows its design. Where this plan differs, the
  difference is a numbered decision (section 5) and a row in that doc's
  `## Deviations`.
- [roym-value-exchange change doc](../../changes/roym-value-exchange/change.md):
  criteria VX-1, VX-2, VX-6 to VX-9 and VX-12 (`:133` to `:147`). The
  agreements doc refines each of them through `AG-*` rows.
- [roym-extensions change doc](../../changes/roym-extensions/change.md):
  only for the `details` block (EX-1, EX-3), because Q3 brings its shape
  into C1. Task.md does not list it as a C1 change doc; this plan writes
  nothing into it (see "Change docs" below).
- The previous slice plans: M08C has none yet, so this plan uses the
  layout of [M08B slice B5](../M08B-language-and-structure/slice-b5-implementation-plan.md),
  the last M08B slice that changed `transaction`. Section 7.11 lists how C1
  avoids the kinds of findings in the M08B review files.

**Change docs.** The prompt lists two: roym-agreements and
roym-value-exchange. Task.md names the same two for C1. Each criterion,
decision and deviation is written in the doc that defines it:

- `AG-*` rows, the record shapes and the writer's rules: roym-agreements.
- `VX-*` rows that a later sub-milestone finishes (VX-6, VX-12):
  roym-value-exchange.
- Q3 moves the *shape* of `details` (EX-1, EX-3) into C1. That deviation
  belongs to roym-extensions, which is not in the prompt's list. The owner
  accepted Q3 as written, including where it goes, so the deviation is in
  roym-extensions' `## Deviations` (2026-11-16).

**Do not copy ids into code.** The ids `D-C1-n`, `Qn` and `AG-n` in this
plan are for the plan and the change docs only.

### Terms used in this plan

- **Deal core:** the records, states and rules every Roym deal has,
  whatever is exchanged. Defined in the agreements doc ("Terms").
- **Commitment:** one party giving one resource (a service, goods or
  money) to the other. A quote holds 1 to 16.
- **Writer:** the quote author's `transaction` service. It is the only node
  that writes the agreement's state.
- **Fence:** a row created with the data layer's unique insert
  (`AppDataLayer::create`). The first caller wins; a second caller learns
  that the row exists. `crates/wit_interfaces/wit/data-layer/data-layer.wit:200`.
- **Snapshot:** the signed `agreement-progress` record. It states the
  agreement's state and each commitment's state at one moment. It replaces
  today's `booking-progress`.
- **Seat hold:** C1's local stand-in for the scheduling extension's
  reservation: two ledger rows in `transaction` that keep one seat of one
  slot for one commitment (D-C1-5). C3b replaces it with calls to the
  scheduling extension.
- **Track end:** the time after which an unsettled commitment ends
  `ended-unconfirmed`: `max(window.latest_secs, agreed_at_secs) + 30 days`.
- **Parity harness:** `crates/roym_web/tests/dual_build_parity`. It runs
  each scenario against the native build and the `wasm32-wasip2` build of
  every Roym service and compares the answers.
- **Test host:** an in-memory `AppHost` for native unit tests of service
  code (D-C1-3, Q2).

## 1. What C1 builds, in one paragraph

Today a Roym deal has one shape: the counterparty pays money and the quote
author does the work (`AgreedTerms`, `crates/roym_core/src/transaction.rs:153`).
C1 replaces it, in place, with the **deal core** of the agreements doc: an
`enquiry` (today's `request`), a quote whose `DealTerms` hold 1 to 16
commitments with a declared value each, an agreement receipt that copies
those terms, an `agreement-progress` snapshot with one state per commitment,
and settlement records (`payment-request`, `payment-acknowledgement`,
`fulfilment-receipt`) that each name one commitment and a `giver` or
`receiver` role. The quote author's node stays the only writer. It takes the
fence `decision:<quote_id>`, so at most one version of a quote becomes an
agreement, and it never signs first. Booking a slot keeps working: the seat
rows stay in `transaction` until C3b, behind the new shapes. Values stay in
plain form; C2 hides them. C1b then adds the rules that make the single
writer safe under late cards, crashes and restores, and replaces today's
local, unsigned decline with a signed `deal-decline` that the other side
receives.

### Why C1 is split (Q1)

Measured scope (section 2.2, point 9): the change touches about 5,500 lines
of `transaction`, 2,000 lines of `roym_core` deal types, about 4,500 lines of
parity and contract tests, 69 JSON shape files, 1,700 lines of two-node
tests, 9 Hub files with their unit tests, 2 Playwright files, and
`roymctl`. One review cannot check a change of that breadth and also the
crash and concurrency rules of the writer.

The cut follows the two kinds of work:

- **C1a is wide and mostly mechanical.** New shapes everywhere; behaviour
  as today, except where a rule needs no new machinery (the fence key, no
  author-first accept, per-commitment settlement, the track rule).
- **C1b is narrow and deep.** It changes only `transaction`, its tests, and
  two Hub actions. It needs the fault-injecting test host.

Each part leaves a working system. After C1a, every deal can be made,
booked, paid, fulfilled and cancelled through the new shapes, with the same
robustness as today (today's writer has no crash recovery either; section
2.2, point 4). After C1b, the agreements doc's single-writer design is
complete. C1a builds the writer's decision in its final step order, so C1b
adds recovery around it and rewrites nothing.

Rejected cuts:

- **A pure `roym_core` library first, then the switch.** It adds types that
  nothing uses, and it leaves the hard part (the switch across every
  layer) in one piece.
- **Shapes with today's writer logic, then the new writer.** C1b would
  rewrite C1a's decision code. Building the final step order in C1a avoids
  that.
- **One C1.** See above. The agreements doc's open question 12 asked the
  same question about C1 and C2 and kept them apart for the same reason.

## How this is shown to an end user

C1a:

- Bob gets an enquiry from Alice and writes a quote in the Hub, as today.
  He fills the same fields: what he will do, the price, the payee.
- Alice sees the quote as a short list: "Bob gives: one lesson" and "You
  give: ₹100", each with its value.
- Alice presses "Accept". Both Hubs show the signed agreement with the same
  list. The receipt card now also says, for each person, which language
  they read the terms in.
- Booking a time slot still works the same way for a person.
- Demo story 50 starts here: Bob offers a lesson for ₹100, and Alice
  accepts. (C6 adds the sack of wheat, the totals, settlement and the
  statement.)

C1b:

- When Alice declines a quote, Bob now sees it in his Hub. The decline
  dialog says that Bob will be told.
- "Accept" shows only on the newest version of a quote.
- If Alice accepts a version that Bob has already replaced, her Hub shows
  that the acceptance was refused, and she can accept the new version.

How a developer can see it work:

- `mise run test:e2e`: story 40 (Hindi accept) and the new story 50 pass;
  Hub cases 3, 4, 24 to 31 pass with the new card types and shapes.
- `cargo nextest run -p syneroym-roym-transaction`: the new in-crate service
  tests, including (in C1b) a crash after each step of the writer's
  decision.
- `cargo run --bin roymctl -- roym transaction agreement --quote <rec>`
  prints the agreement with one line per commitment and its state.

## 2. Points for the owner

### 2.1 Questions

Each question has options and a recommendation. The plan is written for the
recommended answer. **The owner accepted every recommendation on
2026-11-16** (each question's "Answer" line). The answer-dependent rows are
in the change docs' `## Deviations`.

#### Q1. Split C1 into C1a and C1b?

**Answer (2026-11-16): (a).** The status.md row is split into C1a and C1b,
and task.md's C1 section names the two parts.

Task.md asks the plan to split C1 only if each part leaves a working
system.

- **(a) Split as section 1 describes: C1a (deal shapes everywhere), C1b
  (single-writer rules and signed decline).** Recommended. Each part works
  on its own and has one kind of risk.
- (b) Keep one C1. One PR of roughly 15,000 changed lines, and a review
  that must check the shapes and the crash rules together.
- (c) Another cut (section 1, "Rejected cuts").

With (a), the C1 row of [status.md](status.md) becomes two rows (C1a,
C1b), and task.md's C1 section names the two parts.

#### Q2. How do tests control time and simulate a crash?

**Answer (2026-11-16): (a).**

The service reads time from `clock::now_secs()`
(`crates/roym_core/src/clock.rs:18`). Its own comment says a test cannot pin
it. The parity harness pins only the record signer's clock
(`RecordClock::Fixed`, `crates/roym_web/tests/dual_build_parity/helpers.rs:1042`).
No test host can fail a chosen write. But AG-7, AG-14, AG-42 and AG-43 need
both, and parity scenario 165 breaks under the new track rule (section 2.2,
point 2).

- **(a) A native, in-memory test host and "at" functions.** Recommended.
  Each verb becomes a thin wrapper: `verb(host, input)` calls
  `verb_at(host, input, clock::now_secs())`. Tests call `verb_at` with any
  time. The test host is today's `TestHost` in `roym_core`
  (`crates/roym_core/src/signing.rs:305`), moved to a `testing` module
  behind a `test-host` feature and extended: `create`, `delete`, filtered
  `query`, call routing by service and method, a settable signing clock,
  and "fail the n-th write". No production test knob. The parity harness
  still proves that both builds agree; the in-crate tests prove timing and
  crash behaviour.
- (b) A clock the host provides: a WASI wall-clock override in the sandbox
  plus a native override, so parity scenarios can move time. Two new
  mechanisms in substrate code, and still no fault injection.
- (c) A hidden test parameter on the methods (`now_secs`). A production
  input that only tests use; a caller could fake time.

#### Q3. Where does a slot go in C1?

**Answer (2026-11-16): (a).** Deviation written in roym-extensions.

The agreements doc moves `slot_id` into the scheduling extension's
`details.data` (`change.md:322`). But task.md puts `details` in C3a (EX-1,
EX-3), and C1 must keep booking working.

- **(a) C1a adds the commitment's `extension` and `details` fields in their
  final shape, with one fixed case checked in code:** `extension` =
  `{ id: "scheduling", version: 1 }` and `details.schema` =
  `"scheduling/slot@1"`, `details.data` = `{ "listing_id": …, "slot_id": … }`.
  Any other extension is refused with `commitment-kind-not-supported`.
  Recommended. The shape never changes again; C3a replaces the fixed check
  with the manifest, the registry and the schema check (EX-2), and C3b
  moves the slot to the scheduling service. A deviation is written into
  roym-extensions: EX-1 and EX-3 (shape only) are built in C1a. That doc
  is not in this slice's change-doc list, so the owner confirms where it
  goes.
- (b) A temporary visible `slot_id` on the commitment, removed in C3a. A
  field the design does not have, and a second shape change.

#### Q4. One original language per record?

**Answer (2026-11-16): (a).** AG-39's refinement is in the agreements
doc's `## Deviations`.

The agreements doc gives every free-text field its own `LocalText` with its
own `lang`, and AG-39 requires one set of *translation* languages per
record. It does not say that every field's *original* language is the
same. Two rules need one original language: `viewed_via_mt` is refused when
`viewed_lang` equals "the original's language"
(`crates/roym_core/src/transaction.rs:510`), and the Hub's `bestVersion`
picks one version for the whole record.

- **(a) Every `LocalText` in one record has the same `lang` and the same set
  of translation languages.** Recommended. Checked with AG-39 by the same
  function (`check_record_text`). The Hub writes all fields in one
  language anyway. The agreements doc's AG-39 gets this refinement.
- (b) Leave the original languages free, and define `viewed_lang` against
  the quote's `scope`. A person who reads a mixed record has no single
  "language they read".

#### Q5. The restore rules rest on conversation history that a full restore also rolls back

**Answer (2026-11-16): (a).** The limit is in the agreements doc's
`## Deviations`; C1b adds the backlog row.

AG-42 and review fix P36 say: after a restore from an old backup, the
writer continues its `seq` from its own snapshot cards in the conversation,
and reuses its own countersigned half or snapshot found there
(`change.md:579`, `:580`). But the conversation history is in the same
backup set: `conversation.export` writes it (`crates/roym_conversation/src/app/backup.rs:33`).
After a full restore from an old backup, the writer's cards sent after that
backup are gone from its own node too, and a 1:1 conversation does not
fetch them again from the peer. So the rule works only when `transaction`
is restored alone, or when the conversation data is newer.

- **(a) Build the rules as designed (C1b), and write the limit down:** a
  full restore from an old backup can produce a second countersigned half
  or a reused `seq`. The counterparty's node keeps the first record it
  filed and shows `progress-fork` (AG-12), so nothing is merged in silence.
  A deferred-backlog row and a sentence in the agreements doc record the
  limit. Recommended.
- (b) Add a "send me my records again" card that the restored writer sends
  and the counterparty answers. A new card type and a new sync path.
- (c) Make each counterparty card carry the highest `seq` it holds, so the
  writer learns it. A field in every settlement record.

#### Q6. `booking.start`, `scheduled` and `in-progress` until C3b

**Answer (2026-11-16): (a).**

The agreements doc says `scheduled` and `in-progress` are not deal-core
states; they become scheduling steps (`change.md:486`), and step records
arrive in C3a and C3b. `booking.start` is called by `roymctl roym
transaction booking start` (`apps/roymctl/src/commands/roym/booking.rs:159`
to `:181`), the restore two-node test (`crates/substrate/tests/roym_restore_e2e.rs:429`)
and parity scenarios (`booking.rs:465`, `fulfilment.rs`, `bundles.rs`). The
Hub never calls it.

- **(a) Remove `booking.start` in C1a, with no stand-in.** Recommended. From
  C1a to C3b nobody can mark "work started"; nothing depends on it, because
  a commitment settles from `promised` directly. A backlog row with target
  C3b records it.
- (b) Keep a "started" flag on the agreement until C3b. A field the design
  does not have.

### 2.2 Points found while reading

1. **The review fixes P28 to P38.** Checked against the code for this plan.
   Q5 is the only one the plan cannot build as written. The others hold,
   with four small notes that C1b handles without an owner decision:
   - P28's rule "send again after a crash, the counterparty files a repeat
     as a repeat" makes the conversation hold two messages with the same
     card. The Hub would draw both. D-C1-14: a node files a card whose
     `record_id` it already holds with `repeat: true`, and the Hub hides it.
   - P28's rule covers the decision. For later transitions (a half, a tick,
     a cancel), the step that is retried is not run again by anyone: the
     card that caused it is already filed, and `transaction.sync` reads
     incrementally. Today's code handles this with a `message_id` on the
     stored step and `resend_if_unsent` (`crates/roym_transaction/src/app/progress.rs:65`).
     C1a keeps that for snapshots after `seq` 1 (D-C1-8).
   - "`quote-already-agreed` … leave only a bug or a restored backup as a
     cause" (`change.md:495`) is not quite true: a `quote.set` that runs at
     the same time as a sync can sign a new version after the writer
     countersigned an older one. The fence still refuses the new version's
     acceptance, so the outcome is safe. C1b corrects the sentence in
     `## Deviations`.
   - The settlement sketch puts `supersedes` in the payment acknowledgement
     payload (`change.md:425`). Today it is the envelope's `supersedes`
     (`crates/roym_transaction/src/app/payment_ops.rs:237`), and the doc
     says "as today". C1a keeps the envelope field.
2. **Parity scenario 165 relies on a window in 1970.** It sets the quote's
   schedule to `1_000..2_000` so that the track window has ended
   (`crates/roym_web/tests/dual_build_parity/fulfilment.rs:158` to `:161`).
   Under the new rule the end is at least `agreed_at_secs` plus 30 days, and
   `agreed_at_secs` is the pinned record clock (`wall_now + 240`). So the
   scenario can never reach `ended-unconfirmed` again in the parity
   harness. It moves to an in-crate test with a chosen time (Q2).
3. **The tick does not run "on each sync, as today".** The agreements doc
   says so (`change.md:563`). Today the writer ticks only in `booking.get`
   (`crates/roym_transaction/src/app/booking_ops.rs:340` to `:345`). C1a
   ticks on every `transaction.sync` of the conversation and in
   `agreement.get` and `agreement.progress` on the writer (D-C1-9). A
   deviation records the corrected fact.
4. **Today's decision has no crash recovery.** The decision runs only when
   a receipt card is filed (`maybe_countersign`,
   `crates/roym_transaction/src/app/agreement_ops.rs:451`, called from
   `file_agreement_receipt_card`, `sync.rs:448`). A crash after the fence
   and before the countersign leaves the deal waiting, because the next
   incremental sync does not file that card again. C1a keeps this gap; C1b
   closes it with the recovery sweep. A backlog row with target C1b records
   the interim.
5. **AG-12's two-node test asks for "a test hook that skips the method
   check"** (`change.md:110`). That is a production code path only tests
   use. The parity harness already signs peer records by hand
   (`peer_counterparty_receipt`, `fixtures.rs:264` to `:455`). C1b uses a
   hand-signed half for a superseded version, delivered as a peer card, and
   adds no hook (D-C1-15). The two-node part checks the refusal shown on
   the counterparty's node with the same hand-signed half delivered over
   the real conversation.
6. **The Hub's "superseded" check is about enquiry versions, not quote
   versions.** `isQuoteSuperseded` compares the enquiry a quote answers
   with the newest enquiry version (`crates/roym_web/ui/src/screens/messages.ts:475`
   to `:495`). "Accept only on the latest version" needs the newest version
   of each `quote_id`. C1b generalises `computeNewestRequestMap` (`:451` to
   `:473`) into one `newestVersionMap(cards, cardType, idField)` and uses it
   for both.
7. **Pinned English sentences.** `booking/tests.rs:296` to `:320` reads
   `en.ftl` and pins seven notices, including `PROGRESS_NOTICE` and
   `ONE_PAYMENT_NOTICE`; `transaction/tests.rs:427` to `:445` pins two more.
   C1a keeps the pin pattern for the notices it keeps, and changes
   `ONE_PAYMENT_NOTICE` to the per-commitment rule.
8. **`BookingMode::Enquiry` already exists** (`crates/roym_core/src/listing.rs:53`).
   It is a listing's booking mode, not the record. The two names do not
   clash in code (`listing::BookingMode::Enquiry` against
   `transaction::EnquiryPayload`). C3a removes the booking mode.
9. **Measured size of the change.** Files and line counts (`wc -l`) of what
   C1 changes:
   - `crates/roym_transaction/src/` (without `bindings.rs`): 5,557 lines in
     16 files; nearly every file changes.
   - `crates/roym_core/src/`: `transaction.rs` 715, `booking.rs` 418,
     `payment.rs` 301, `fulfilment.rs` 131, `card.rs` 198, `record.rs` 55,
     `router.rs` 305, `backup.rs` 576, and their tests (about 2,000 lines).
   - Parity: `transaction.rs` 846, `transaction_cards.rs` 697, `booking.rs`
     669, `transaction_validation.rs` 537, `payment.rs` 424, `bundles.rs`
     310, `events.rs` 264, `fulfilment.rs` 252, `agreement_lang.rs` 230,
     `deal_fixtures.rs` 188, `fixtures.rs` 799, `normalize.rs` 90.
   - Method contract: `shapes_transaction.rs` 443, `contract.rs` 663, and 69
     JSON shape files in `crates/roym_web/tests/method_shapes/`.
   - Two-node: `common/roym_flow.rs` 356, `roym_transaction_e2e.rs` 571,
     `roym_booking_e2e.rs` 313, `roym_restore_e2e.rs` 608,
     `roym_trust_e2e.rs` 406.
   - Hub: `screens/messages.ts` 1016, seven card templates, `registry.ts`,
     `render.ts`, `samples.json`, `en.ftl` 803, `hi.ftl` 780, and eight unit
     test files.
   - Playwright: `tests/roym-hub.spec.ts` 1535 (cases 3, 4, 24 to 31),
     `demo/40-roym-languages.spec.ts` 271, `hub-helpers.ts` 284.
   - `roymctl`: `transaction.rs` 669, `booking.rs` 260, `tests/cli_args.rs`
     (`:439` to `:490`), `tests/backup.rs` (`:30` to `:45`).
10. **Component sizes on `main` at `48ea989c`** (`cargo component build
    --release --target wasm32-wasip2`, one build folder, 2026-11-15):
    `transaction` 1,516,528; `catalog` 929,558; `conversation` 811,880;
    `directory` 1,467,413; `profile` 771,718; `web` 310,033 bytes.
    `transaction` gets new record code, so its limit is +15% (227,479
    bytes). The other five get none, so +40 KB each.

## 3. What C1 meets

From [roym-agreements](../../changes/roym-agreements/change.md). "Part" says
which PR proves it. "Later" names the part another slice finishes; each
"Later" entry is a row in that doc's `## Deviations` (written with this plan,
as task.md section 5 asks).

| Criterion | Part | What C1 proves | Later |
| --- | --- | --- | --- |
| AG-1 | C1a | Two-node: A quotes B and B quotes A at the same time; both reach `agreed`. | |
| AG-2 | C1a | Unit table over giver × resource kind for `service-in-slot`, `goods-or-service`, `currency`. Two-node: a service-for-service agreement closes. | EEP kinds: M08E, M08H |
| AG-3 | C1a | Unit: both `Answers` forms give equal receipt and first-snapshot payloads, except the named fields. Only enquiries are accepted by `quote.set`. | Want-Bid end to end: M08F |
| AG-4 | C1a | Unit validation table (0, 1, 16, 17 commitments, missing fields). Two-node: service + currency + goods agreement forms and closes. | EEP kinds: M08E |
| AG-5 | C1a | Unit. | |
| AG-6 | C1a | Unit table of `step` over every (state, event) pair for `fulfilment` and `currency`. | EEP tables: M08E, M08H |
| AG-7 | C1a | Service (in-crate, chosen time). | |
| AG-8 | C1a | Unit: snapshot-only view words `claimed: true/false`, `failed`/`cancelled` with reason. | Presentation verify: C2 |
| AG-9 | C1a | Service: cancel allowed / refused with `commitment-moved`. | |
| AG-10 | C1b | Two-node: decline of a quote, withdrawal of an enquiry, decline of an enquiry. | |
| AG-11 | C1a | Unit and service: only writer-signed snapshots with a higher `seq` are filed. | |
| AG-12 | C1b | Service: `quote-superseded` on accept; refusal snapshot once over two syncs; fork. Two-node with a hand-signed half (D-C1-15). | |
| AG-13 | C1a, C1b | C1a: repeated `agreement.accept`, `fulfilment.sign`, `payment.acknowledge` (today's behaviour, new shapes). C1b: `deal.decline`. | |
| AG-14 | C1a, C1b | C1a: the slot race (two-node, today's booking race test) through the seat hold. C1b: every crash point, recovery past the expiry, a refused `confirm` (stub). | Moved to the scheduling extension: C3b |
| AG-15 | C1a | Service: the counterparty's own seat hold before it signs; `reserve-failed`. | Moved to the extension: C3b |
| AG-22 | C1a | Unit (same quote with and without money) and two-node. | |
| AG-23 | C1a | Unit: the value range. | Hub totals: C6 |
| AG-31 | C1a, C1b | Wire-refusal tables list every new method. | |
| AG-32 | C1a | Unit. | |
| AG-33 | C1a | Service: a receiver's half with `settlement_note` settles; value unchanged. | Hidden-field check: C2. Statement row: C6 |
| AG-35 | C1b | Service. | |
| AG-36 | C1a | Unit: the cascade in the transition tables. | Snapshot and release: C3b (no `failed` can happen before `commitment-failure` exists) |
| AG-37 | C1a | Unit. | |
| AG-39 | C1a | Unit (with Q4). | |
| AG-41 | C1a | Service. | |
| AG-42 | C1b | Service, with the limit of Q5. | |
| AG-43 | C1b | Service with a chosen time, two syncs after the expiry. | |

From [roym-value-exchange](../../changes/roym-value-exchange/change.md),
through the rows above: VX-1 (AG-1), VX-2 (AG-2, three kinds), VX-6 (AG-3,
enquiry form only), VX-7 (AG-4), VX-8 (AG-6, AG-7), VX-9 (AG-11, AG-12),
VX-12 (the snapshot names each failed commitment and its giver; shown by
reputation in M08G).

Not in C1: AG-16 (C3b), AG-17 to AG-21, AG-34, AG-38, AG-40 (C2), AG-24 to
AG-26 (M08E, M08H), AG-27 to AG-30 (C6).

## 4. Behaviour-change audit

### Wire changes

Every Roym method stays local-only (`admit::require_internal`,
`crates/roym_transaction/src/app.rs:279`). No method becomes wire-reachable.
"Wire" here means the records and cards that travel between the two parties,
and the local JSON-RPC methods the Hub and `roymctl` call.

**Records and cards (C1a):**

| Today | After C1a |
| --- | --- |
| record and card `request` v1, `RequestPayload` (`transaction.rs:181`) | `enquiry` v1, `EnquiryPayload`: `enquiry_id` (`enq_…`), `description: LocalText` |
| `quote` v1 with `request_record_id`, `listing_id`, `slot_id`, `terms: AgreedTerms`, `translations` | `quote` v1: `answers`, `counterparty_did`, `terms: DealTerms` |
| `agreement-receipt` v1 with `terms: AgreedTerms` | `agreement-receipt` v1 with `terms: DealTerms`; `viewed_lang`, `viewed_via_mt` unchanged |
| `booking-progress` v1 (`booking.rs:93`) | `agreement-progress` v1: `state`, `state_reason`, `cancel_reason`, `commitments[]` |
| `payment-request` with `currency`, `amount_minor`, `note` | `payment-request` with `commitment_id`, `note: LocalText` |
| `payment-acknowledgement` with `role: DealRole`, `currency`, `amount_minor` | `commitment_id`, `role: giver|receiver`, `settlement_note`; no amount |
| `fulfilment-receipt` with `role: DealRole`, `terms` | `commitment_id`, `role: giver|receiver`, `settlement_note`; no terms |
| event kind `transaction.booking` | `transaction.progress` |

**Records and cards (C1b):** new `deal-decline` v1 record and card. The card
table has eight types after C1b.

**Methods (C1a):**

| Today | After C1a |
| --- | --- |
| `request.set/.get/.list/.history/.verify/.ping` | `enquiry.*` |
| `quote.set` input `request_record_id`, `listing_id`, `slot_id`, `terms` (flat), `translations` | `enquiry_record_id`, `terms: DealTermsInput` (D-C1-6) |
| `agreement.accept` on either party | counterparty only; `writer-countersigns` on the author's node; reserves the counterparty's own slots |
| `agreement.get/.list` view `terms: AgreedTerms`, `pair` | adds `state`, `seq`, `commitments[]` (with state), `agreed_at_secs`, `track_ends[]` |
| `booking.get/.list/.start/.cancel/.history/.verify/.ping` | removed; `agreement.cancel {agreement, reason: CancelReason}`, `agreement.progress {agreement}` |
| `payment.request/.acknowledge/.get` take `agreement` | take `agreement` and `commitment_id`; `acknowledge` adds `settlement_note` |
| `fulfilment.sign/.get` take `agreement` | take `agreement` and `commitment_id`; `sign` adds `settlement_note` |
| `transaction.export/.import` sections `requests`, `request_history`, `bookings`, `progress` | `enquiries`, `enquiry_history`, `agreement_progress` |

**Methods (C1b):** `quote.decline` removed; `deal.decline` and `deal.ping`
added. `agreement.accept` adds `quote-superseded`; `quote.set` adds
`quote-already-accepted` and `quote-ended`.

### Behaviour changes

C1a:

1. The quote author can no longer accept first (`prepare_author_booking`,
   `agreement_ops.rs:415`, is deleted). Every deal has one order.
2. The fence is `decision:<quote_id>` (today `decision:<agreement>`,
   `ledger.rs:52`). At most one version of a quote becomes an agreement.
3. A commitment's track end is `max(window.latest_secs, agreed_at_secs) +
   30 days` (today `window.latest_secs` or the scheduling time, plus 30
   days, `booking.rs:189`).
4. The writer ticks on every sync and in `agreement.get` and
   `agreement.progress` (today only `booking.get`).
5. Payment and fulfilment settle per commitment. "One payment per quote"
   becomes "one payment per currency commitment".
6. The progress card names commitments and their states, not two tracks.
   `scheduled` and `in-progress` are gone (Q6).
7. A slot quote that loses the race ends `not-formed` with `slot-taken` or
   `slot-unavailable` (today `conflict`).
8. Cancel is allowed while every commitment is `promised` or
   `payment-requested`, with a reason code (today: a free-text reason while
   both tracks are `none`).
9. The Hub's receipt card says which language each party read.

C1b:

10. A decline is signed and sent; the other node moves the deal to
    `declined`. A new quote version no longer clears a decline.
11. `agreement.accept` refuses a version that is not the latest one held.
12. The writer refuses late or superseded acceptances with a signed
    `not-formed` snapshot.
13. `quote.set` refuses a revision after an acceptance, and after any end.
14. A crash at any step of the writer's decision ends, after the next sync,
    as if no crash happened.

### Stays the same

- Every method is local-only. No wire-reachable method is added.
- Quote ids (`derive_quote_id`, `transaction.rs:532`), the canonical bytes
  of a signed payload, the 64 KiB payload limit and the 160 KiB card body
  limit.
- The `viewed_lang` and `viewed_via_mt` rules of ML-18, the 4-translation
  limit, the address disclosure notice.
- The snapshot is signed by the writer's service key and bound to the
  quote's signer (`file_progress_card`, `receipts.rs:43`).
- The receiver's half settles; the giver's half alone is a claim.
- Payment corrections by `supersedes` on the envelope.
- The Hub's quote form fields, the request (enquiry) form fields, and the
  CSS classes the Playwright helpers use (D-C1-12).
- `catalog`'s `availability.*` methods and the seat rows in `transaction`'s
  `ledger` (until C3b).
- `conversation`, `profile`, `directory` and `web` Rust code.

## 5. Decisions this plan makes

Each decision is written into the agreements doc's `## Deviations` when it
differs from the doc's text (marked "deviation").

- **D-C1-1. Module layout in `roym_core`.** Edit in place to keep history:
  `transaction.rs` keeps the enquiry, quote and receipt records;
  `booking.rs` is renamed `progress.rs` (snapshot, states, verify);
  `payment.rs` and `fulfilment.rs` keep their records. New: `deal.rs`
  (`DealTerms`, `Commitment`, `LocalText`, kinds, validators) and
  `deal/transition.rs` (the pure transition function and the track end).
  `deal/tests.rs` and `deal/transition/tests.rs` hold the unit tests.
- **D-C1-2. Reuse `DealRole` for `giver`.** Its wire words are
  `quote-author` and `counterparty`, as the doc's `giver` field needs. A new
  `SettlementRole { Giver, Receiver }` is the role of a settlement record.
- **D-C1-3. The test host (Q2 a).** `roym_core::testing::TestHost`, behind
  `cfg(any(test, feature = "test-host"))`. `syneroym-roym-transaction` takes
  `syneroym-roym-core` with `test-host` as a dev-dependency feature. The
  existing `TestHost` in `signing.rs` moves there, so one host serves every
  crate.
- **D-C1-4. "At" functions.** Every verb that reads the clock becomes
  `pub(crate) async fn verb_at(host, input, now: u64)`, and the dispatch
  table calls the wrapper `verb(host, input)`. Only the wrapper reads
  `clock::now_secs()`.
- **D-C1-5. The seat hold, behind a small trait.**
  `crates/roym_transaction/src/app/hold.rs` defines
  `trait Reservations { reserve, confirm, release }`, keyed by (agreement
  id, commitment id), and `SeatLedger`, its implementation over today's
  `seat:` rows. `reserve` creates `hold:<agreement>:<commitment_id>` and
  `seat:<slot>:<n>` in one `create`, trying `n = 1..=capacity` as
  `try_claim_seat` does today (`ledger.rs:178`); a `hold:` row that already
  exists means "already held" (idempotent). `confirm` does nothing in C1,
  because a seat hold has no expiry. `release` deletes the seat row if it
  names this agreement and commitment, then the hold row. C3b swaps
  `SeatLedger` for calls to the scheduling service. C1b's tests use a stub
  that refuses `confirm`.
- **D-C1-6. `quote.set` input.** `{ enquiry_record_id, quote_id?,
  expires_in_secs, terms: DealTermsInput }`. `DealTermsInput` is `DealTerms`
  without `quote_expires_at_secs` and without `commitment_id`s; the service
  assigns `c1` to `c16` by position (AG-37) and sets the expiry.
- **D-C1-7. Today's single-amount quote maps to two commitments.** The Hub
  form and `roymctl quote` build: `c1`, the quote author gives the work
  (`goods-or-service`, or `service-in-slot` with a slot), declared value =
  the amount; `c2`, the counterparty gives `currency`, value = the amount,
  with `payee`, `payment_methods`, `tax_minor`, `fees_minor`. An amount of 0
  builds no `c2` (AG-22: a free service has no currency commitment).
  `payment_timing` maps to `after`: `before-work` puts `c2` in `c1.after`,
  `after-work` puts `c1` in `c2.after`. The service commitment's
  `description` comes from a new one-line form field, "What you give",
  filled in advance from the first line of the enquiry, so existing helpers
  need not type it; the currency commitment's description is the catalog
  text `quote-payment-description` in the quote's language.
- **D-C1-8. Snapshot storage.** One collection, `agreement_progress`, holds
  every snapshot this node signed or filed, under
  `snapshot:<agreement>:<seq>`. The writer creates the row with a unique
  insert before it sends (the "made once" rule), and stores the
  `message_id` after the send; `resend_if_unsent` stays for snapshots after
  `seq` 1. The `AgreementRow` (collection `agreements`) gains `state`,
  `seq`, `commitments`, `agreed_at_secs`. The collections `bookings` and
  `progress` go away.
- **D-C1-9. Tick (deviation).** The writer ticks each agreement of the
  conversation on every `transaction.sync`, and the agreement asked for in
  `agreement.get` and `agreement.progress`.
- **D-C1-10. Ledger rows in C1a.** `decision:<quote_id>` = `{ kind:
  "decision", state: "deciding" | "agreed" | "not-formed", agreement,
  reason?, at_secs }`; `countersign:<agreement>` = the stored half;
  `hold:<agreement>:<commitment_id>` and `seat:<slot>:<n>` (D-C1-5);
  `payreq:<agreement>:<commitment_id>`,
  `ack:<agreement>:<commitment_id>:<role>:<ver>` and
  `fulfil:<agreement>:<commitment_id>:<role>` (today's keys at
  `payment_ops/fence.rs:37`, `:149`, `fulfilment_ops.rs:313`, plus the
  commitment id). C1b adds `refused:<half record_id>`.
- **D-C1-11. Hidden-to-be fields are always serialised.** Every commitment
  carries all ten fields that C2 will hide (`description` to `eep`), with
  `null` when unused, so C2 changes only where they sit, not whether they
  are present.
- **D-C1-12. The Hub keeps its visible words for the enquiry.** The record
  and methods are renamed; the person still sees "Request" in English and
  today's Hindi word, and the form keeps its CSS classes
  (`.toggle-request-form`, `.request-desc-input`, `.send-request-button`).
  Only the card's class follows its type (`.card-enquiry`). No new Hindi
  text for the rename.
- **D-C1-13. `Answers::Want` is refused** by `quote.set` and the verifier
  with `answers-kind-not-supported` until M08F.
- **D-C1-14. Repeat cards (C1b).** A node that files a card whose
  `record_id` it already holds stores it with `repeat: true`, and the Hub
  does not draw it. A repeated snapshot with the same `seq` and the same
  bytes is a repeat, not a `progress-fork`.
- **D-C1-15. No test hook (C1b, deviation).** AG-12's test uses a
  hand-signed half delivered as a peer card, not a hook that skips the
  method check.
- **D-C1-16. Ordered decision in C1a.** The writer runs the agreements
  doc's steps 2 to 4 in order: create `decision:<quote_id>` (`deciding`);
  reserve its own `service-in-slot` commitments through `Reservations`;
  confirm; store `countersign:<agreement>`; create
  `snapshot:<agreement>:1`; send both cards; put the decision row last.
  On a failed reserve: store the `not-formed` snapshot first, release, send,
  put. C1a has no recovery sweep and no step-1 refusal snapshots; C1b adds
  them around this code.

## 6. Step-by-step changes

Each step ends with `cargo build --workspace`, the named tests passing, and
a commit. Steps 5a and 5b are one pair: after 5a the Rust tests pass, and
the Playwright tests pass again only after 5b. Run `mise run build:roym`
before every parity run.

### C1a

#### Step 1 — Commit this plan

This plan, the deviation rows of section 3 in the two change docs, and the
C1 row of status.md (done on `feat/m08c-c1-commitment-model`).

#### Step 2 — The pure deal core (`roym_core`)

Additive; nothing calls it yet.

`crates/roym_core/src/deal.rs`:

```rust
pub const MAX_COMMITMENTS: usize = 16;
pub const MAX_DECLARED_VALUE_MINOR: i64 = 1_000_000_000_000_000; // 10^15
pub const MAX_COMMITMENT_DESCRIPTION_LEN: usize = 2048;
pub const MAX_AFTER: usize = 15;
pub const MAX_EXTENSION_ID_LEN: usize = 32;
pub const SCHEDULING_EXTENSION: &str = "scheduling";
pub const SCHEDULING_SLOT_SCHEMA: &str = "scheduling/slot@1";

pub struct LocalText { pub lang: LangTag, pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub translations: Vec<TextTranslation> }
pub struct TextTranslation { pub lang: LangTag, pub text: String }

#[serde(rename_all = "kebab-case")]
pub enum ResourceKind { ServiceInSlot, GoodsOrService, Currency }
#[serde(rename_all = "kebab-case")]
pub enum CommitmentKind { Fulfilment, Currency }

pub struct ExtensionRef { pub id: String, pub version: u32 }
pub struct Details { pub extension: String, pub schema: String,
    pub data: serde_json::Value, #[serde(default)] pub extra: Vec<DetailsExtra> }
pub struct DetailsExtra { pub label: String, pub value: String }

pub struct Commitment {
    pub commitment_id: String,
    pub giver: DealRole,
    pub resource_kind: ResourceKind,
    pub kind: CommitmentKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension: Option<ExtensionRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
    // The ten fields below are always serialised (D-C1-11).
    pub description: LocalText,
    pub declared_value_minor: i64,
    pub details: Option<Details>,
    pub window: Option<TimeWindow>,
    pub location: Option<QuoteLocation>,
    pub payee: Option<String>,
    pub payment_methods: Option<Vec<String>>,
    pub tax_minor: Option<i64>,
    pub fees_minor: Option<i64>,
    pub eep: Option<serde_json::Value>,
}

pub struct DealTerms {
    pub currency: String,
    pub quote_expires_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_exchange: Option<String>,
    pub commitments: Vec<Commitment>,
    pub listing_id: Option<String>,
    pub scope: LocalText,
    pub cancellation_terms: LocalText,
    pub refund_terms: LocalText,
    pub dispute_path: LocalText,
}

#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Answers { Enquiry { record_id: String }, Want { intent_id: String, version: u32 } }

pub fn commitment_id(position: usize) -> String;          // "c1".."c16"
impl DealTerms { pub fn validate(&self) -> Result<(), DealError>; }
pub fn check_record_text<'a>(texts: impl Iterator<Item = &'a LocalText>)
    -> Result<(), DealError>;                              // Q4 + AG-39
```

`DealError` (in `deal.rs`, `thiserror`) has one variant per rule, each with a
stable code word through a `code()` method: `commitment-count`,
`commitment-id`, `commitment-kind-not-supported`, `kind-mismatch`,
`declared-value-range`, `currency-value-zero`, `currency-only-field`,
`eep-field`, `payee-empty`, `breakdown-exceeds-value`, `after-unknown`,
`after-cycle`, `extension-shape`, `details-mismatch`, `slot-details`,
`home-exchange-not-supported`, `text` (wraps `TranslationError`),
`text-lang-mismatch`, `translation-set-mismatch`, `currency-unknown`,
`window`, `location`.

Rules in `validate` (each one a small helper, so no function nears 100
lines):

- 1 to 16 commitments; `commitments[i].commitment_id == commitment_id(i)`.
- Kind table: (`service-in-slot`, `fulfilment`, extension `scheduling` v1
  required), (`goods-or-service`, `fulfilment`, no extension in C1),
  (`currency`, `currency`, no extension). Anything else:
  `commitment-kind-not-supported`.
- `details` present if and only if `extension` is present; for the
  scheduling case: `details.extension == "scheduling"`, `schema ==
  "scheduling/slot@1"`, `data` is exactly `{ listing_id: string, slot_id:
  string }`, `window` is present with `latest > earliest`, and
  `listing::derive_slot_id(listing_id, window.earliest_secs,
  window.latest_secs) == slot_id` (today's check, `transaction.rs:458` to
  `:475`). `extra` has at most 16 entries, label 64 bytes, value 512 bytes.
- `0 <= declared_value_minor <= 10^15`; a `currency` commitment's value is
  at least 1.
- `payee`, `payment_methods`, `tax_minor`, `fees_minor` are `null` unless
  `kind == currency`; for `currency`, `payee` is present and not empty (256
  bytes), methods follow today's token rule (`transaction.rs:356` to
  `:363`), `tax + fees <= declared value` with non-negative parts.
- `eep` is `null`.
- `after`: at most 15 ids, each an id of this quote, not itself, no cycle.
- `window.validate()`, `location.validate()` (today's helpers,
  `transaction.rs:292`, `:304`).
- `currency` is known to `money::currency_minor_exponent`; `home_exchange`
  is absent.
- `scope` up to 4096 bytes, the three terms texts and each `description` up
  to 2048 bytes, none empty; then `check_record_text` over every
  `LocalText` of the terms: same `lang`, same translation language set,
  `check_translation_langs` and `check_text` per translation.

`crates/roym_core/src/deal/transition.rs`:

```rust
#[serde(rename_all = "kebab-case")]
pub enum CommitmentState { Promised, PaymentRequested, Claimed, Fulfilled,
    Acknowledged, EndedUnconfirmed, Failed, Cancelled }
pub enum CommitmentEvent { GiverHalf, ReceiverHalf, PaymentRequest, Tick,
    Cancel, Failure, OtherFailed }
pub enum Step { To { state: CommitmentState, reason: Option<&'static str>,
    claimed: Option<bool> }, NoChange, Refused }

pub fn step(kind: CommitmentKind, state: CommitmentState, e: CommitmentEvent) -> Step;
pub fn is_final(state: CommitmentState) -> bool;
pub fn track_end(window: Option<&TimeWindow>, agreed_at_secs: u64) -> u64;
```

`step` is the two tables of the agreements doc (`change.md:544` to `:557`),
written as one `match` on `(kind, state, event)` that reads like the table.
`track_end` is `window.map_or(agreed_at, |w| w.latest_secs.max(agreed_at))
.saturating_add(TRACK_WINDOW_SECS)`.

Unit tests (`deal/tests.rs`, `deal/transition/tests.rs`): section 7.1.

#### Step 3 — The shared test host

1. Move `TestHost`, `RecordedCall`, `DummyWriter`, `DummyReader` from the
   test module of `crates/roym_core/src/signing.rs` (`:285` to `:470`) to
   `crates/roym_core/src/testing.rs`, `pub` under
   `#[cfg(any(test, feature = "test-host"))]`. `signing.rs` and `admit.rs`
   tests import it from there.
2. Extend it:
   - `create` (all-or-nothing, returns the first existing id), `delete`,
     `delete_many`, and `query` with an equality filter on top-level JSON
     fields and an `$gt`/`$lt` on numbers (only what `paging::query_all`
     and the service use). Keep `push_query_page` for `admit`'s tests.
   - `on_call(service, method, handler)`: a map from (service, method) to a
     closure that answers `AppProxy::call`. `call_sibling` goes through it.
   - `set_now(secs)`: the time the host's `sign_record` stamps into
     `issued_at_secs`.
   - `fail_write_after(n)`: the n-th write (`put`, `create`, `delete`,
     `batch_mutate`) from now returns `DataLayerError::Internal`.
   - `sent_cards()`: every `conversation.send` body, in order, through the
     `on_call` map's default handler for `conversation.send`.
3. `crates/roym_core/Cargo.toml`: feature `test-host = []`.
   `crates/roym_transaction/Cargo.toml`: dev-dependency
   `syneroym-roym-core = { workspace = true, features = ["test-host"] }`.
4. `crates/roym_transaction/src/app/tests.rs` (with `app/tests/`) holds a
   `fixture()` that builds a host with an owner, an enrolled signer, a
   `catalog.availability.get` stub and a `conversation.send` stub.

Test: `cargo nextest run -p syneroym-roym-core`.

#### Step 4 — Rename `request` to `enquiry`, everywhere

One self-contained rename, so the big switch in step 5 is smaller. Every
layer, in one commit:

- `roym_core`: `EnquiryPayload` (`enquiry_id`, `description: LocalText`;
  `lang` and `translations` fold into it), `ENQUIRY_ID_PREFIX = "enq_"`,
  `derive_enquiry_id`, `verify_enquiry`, `RECORD_ENQUIRY`, `CARD_TYPES` and
  `RECORD_TYPES` rows, `router.rs:35` (`enquiry.`), backup sections
  `enquiries`, `enquiry_history`. `DEFAULT_DATA_USE_NOTICE` keeps its text.
- `transaction`: `request_ops.rs` → `enquiry_ops.rs`; collections
  `enquiries`, `enquiry_history`; `api.rs` rows; `file_request_card` →
  `file_enquiry_card`; `quote.set` takes `enquiry_record_id` (only the name
  in this step).
- Hub: `request.set` → `enquiry.set` (`messages.ts:384`) with
  `description: { lang, text }`; card type `enquiry` in `registry.ts`,
  `render.ts`, `samples.json`, `templates/request.ts` → `enquiry.ts`
  (reads `description.text`); unit tests.
- `roymctl`: `transaction request` → `transaction enquiry`, with no alias
  (the product is not released); `tests/cli_args.rs:446` to `:467`.
- Tests: every `request.*` call and `"request"` card type in parity,
  contract shapes (10 `request_*` JSON files renamed `enquiry_*`), two-node
  helpers (`common/roym_flow.rs:91`, `:142`), Playwright cases 4, 26, 29
  (`.card-request` → `.card-enquiry`; the form classes stay, D-C1-12).

Test: full `cargo nextest run --workspace`, `npm test` in the Hub,
`mise run test:e2e`.

#### Step 5a — The deal switch, Rust side

`roym_core`:

- `transaction.rs`: `QuotePayload { quote_id, conversation, sequence,
  answers, counterparty_did, terms: DealTerms }`; `AgreementReceiptPayload`
  with `terms: DealTerms`; `validate` and `verify_quote` call
  `DealTerms::validate`; `viewed_via_mt` check against the record's one
  original language (Q4). Delete `AgreedTerms`, `PaymentTiming`,
  `QuoteTranslation` and `RequestTranslation`. `PairState` stays.
- `booking.rs` → `progress.rs`: `AgreementProgressPayload` as the doc's
  sketch (`change.md:369` to `:386`), `AgreementState`, `CancelReason`,
  `CommitmentProgress`, `RECORD_AGREEMENT_PROGRESS = "agreement-progress"`,
  `verify_agreement_progress` (subject = `agreement`, no expiry), and
  `apply(snapshot, terms, agreed_at_secs, &AgreementEvent) ->
  Result<Option<AgreementProgressPayload>, TransitionError>` built on
  `deal::transition::step`. It closes the agreement when every commitment is
  final (not after a cancel), and runs the `other-failed` cascade. The
  notices keep their pinned text, except `ONE_PAYMENT_NOTICE` (now "Each
  payment in this quote is paid once. A deposit is its own payment."),
  `TRACK_UNCONFIRMED` and `PROGRESS_NOTICE` unchanged. Delete
  `BookingState`, `TrackState`, `NextStep`, `next_step`.
- `payment.rs`, `fulfilment.rs`: the C1 payloads (plain; C2 hides the
  marked fields):

  ```rust
  pub struct PaymentRequestPayload { agreement, conversation, commitment_id,
      quote_author_did, counterparty_did, note: Option<LocalText> }
  pub struct PaymentAcknowledgementPayload { agreement, conversation,
      commitment_id, quote_author_did, counterparty_did, role: SettlementRole,
      observed_at_secs: u64, method: Option<String>, reference: Option<String>,
      settlement_note: Option<LocalText> }
  pub struct FulfilmentReceiptPayload { agreement, conversation,
      commitment_id, quote_author_did, counterparty_did, role: SettlementRole,
      settlement_note: Option<LocalText> }
  ```

  `matches_terms` becomes `commitment_accepts(commitment, method)` (the
  method is one the commitment lists, when it lists any). Note limits:
  `note` 512 bytes, `settlement_note` 1024 bytes, `reference` 512 bytes.
- `card.rs`, `record.rs`: `agreement-progress` replaces `booking-progress`
  in `CARD_TYPES`; `RECORD_AGREEMENT_PROGRESS` stays out of `RECORD_TYPES`
  like today's progress record (`record.rs:45` to `:48`).
- `router.rs:39`: remove `booking.`.
- `backup.rs`: `SECTION_AGREEMENT_PROGRESS`; remove `bookings`, `progress`.
- `transaction.rs:32`: `EVENT_PROGRESS = "transaction.progress"`.

`transaction` service:

- `app.rs`: `SCHEMA_VERSION = 5`; collections (D-C1-8); `AgreementRow`
  with `terms: DealTerms`, `state`, `seq`, `commitments`,
  `agreed_at_secs`; delete `BookingRow`, `ProgressRow`, `BookingWriter`.
- `quote_ops.rs`: `set_at` builds `QuotePayload` from `DealTermsInput`
  (D-C1-6); for each `service-in-slot` commitment the quote author gives,
  check the slot in its own catalog (today's `validate_and_apply_slot`,
  `quote_ops.rs:318`, per commitment, filling `window` from the slot). A
  slot the counterparty gives is checked only for shape here (its node
  checks it when it reserves). Split the file: `quote_ops/input.rs` holds
  `DealTermsInput` and its conversion.
- `agreement_ops.rs`: `accept_at` refuses on the author's node with
  `writer-countersigns`; calls `Reservations::reserve` for every
  `service-in-slot` commitment the counterparty gives (step 6 adds the
  tests; the code is here); on a failed reserve releases the others and
  answers `reserve-failed` with the reason; signs and sends the half.
  Delete `prepare_author_booking`. `maybe_countersign` becomes
  `decide::run(host, row, quote, now)` in a new `app/decide.rs`, the ordered
  steps of D-C1-16.
- `booking_ops.rs` → `progress_ops.rs`: `transition_at(host, agreement,
  event, now)` keeps today's retry over the unique `snapshot:<agreement>:<seq>`
  create (`booking_ops.rs:180` to `:271`), with the new payload;
  `agreement.cancel` and `agreement.progress`; `tick_conversation_at` for
  sync. `ledger.rs` keeps only the decision row helpers; the seat code moves
  into `hold.rs` (D-C1-5).
- `payment_ops.rs`, `payment_ops/fence.rs`, `fulfilment_ops.rs`: every input
  takes `commitment_id`; the signer's role is computed from the commitment
  (`giver` if `commitment.giver` is this node's role, else `receiver`); a
  payment method or request is refused on a non-`currency` commitment and a
  fulfilment on a non-`fulfilment` one; fence keys per commitment
  (D-C1-10); the transition event is `Half { commitment_id, role }`.
- `sync.rs`, `sync/receipts.rs`: `dispatch_known_card` (`sync.rs:263`)
  files `agreement-progress` and the per-commitment settlement cards; the
  writer files a settlement half only from the party the commitment names
  in that role; the counterparty files a snapshot only from the quote's
  signer and only with a higher `seq` (AG-11); after the card loop, the
  writer ticks the conversation's agreements (D-C1-9).
- `backup.rs`, `backup/sections.rs`: the new sections and their checks
  (`validate_ledger_and_bookings` becomes `validate_ledger_and_progress`).
- `thread.rs`: the thread carries `agreement-progress` cards.

Callers:

- `roymctl`: `transaction quote` builds `DealTermsInput` from today's flags
  (D-C1-7); `booking` subcommands → `transaction agreement cancel
  --reason <code>` and `transaction agreement progress`; `payment` and
  `fulfilment` take `--commitment <id>` (default: the only commitment of
  the right kind, if there is exactly one); `tests/cli_args.rs`,
  `tests/backup.rs:30` to `:45`.
- Parity: `fixtures.rs` (`sample_quote_terms`, `peer_signed_*` with the new
  payloads), `deal_fixtures.rs`, and each scenario file in section 2.2
  point 9. Scenario 129 (`transaction.rs:249`) changes from "author accepts
  first" to "author's accept is refused with `writer-countersigns`".
  Scenario 165 (`fulfilment.rs:150`) keeps only the parity of a late half;
  its `ended-unconfirmed` part moves to the in-crate tests (section 7.2).
  `normalize.rs:20` to `:39` follows the card rename. `transaction.rs` must
  not grow past its recorded 846 lines; new scenarios go in a new file
  `dual_build_parity/deal.rs`.
- Method contract: `shapes_transaction.rs` and the JSON shapes (rename
  `booking_*` to `agreement_progress_*`/`agreement_cancel_*`; delete
  `agreement_accept_author_with_slot.json`, add
  `agreement_accept_writer_countersigns.json`); `shapes.rs:31` to `:38`
  allow-list; `contract.rs:550` to `:595` bad-param rows.
- Two-node: `common/roym_flow.rs` (`quote_terms`, `accept_quote`,
  `wait_and_verify_winner_scheduled` → `wait_and_verify_winner_agreed`,
  `complete_winner_lifecycle` per commitment, `assert_booking_completed` →
  `assert_agreement_closed`), `roym_transaction_e2e.rs`,
  `roym_booking_e2e.rs` (the race: loser ends `not-formed`, `slot-taken`),
  `roym_restore_e2e.rs` (drop `booking.start`, Q6), `roym_trust_e2e.rs`.

Test: `mise run build:roym`, then `cargo nextest run --workspace` and
`cargo test --workspace --doc`.

#### Step 5b — The deal switch, Hub side

- `screens/messages.ts`: the quote form (`openQuoteForm`, from `:759`)
  keeps its fields and adds "What you give" (`.quote-give-input`); it
  builds `DealTermsInput` (D-C1-7). Move the form's term building into a new
  `screens/quote_terms.ts` (`buildDealTerms(fields, lang)`), with its own
  unit test, so `messages.ts` does not grow. `agreement.accept` and
  `quote.decline` calls are unchanged in C1a.
- `cards/templates/quote.ts`: one line per commitment: who gives
  ("{name} gives" or "You give"), the description (`bestVersion`), the
  declared value with the currency, the payee on a currency line (case 24
  keeps "Payee, as agreed in this quote: …" on that line), the window and
  place when set. Keep the classes `.quote-scope` and `.quote-total` (story
  40 reads them; `.quote-total` is the sum of currency commitments the
  reader gives).
- `templates/agreement_receipt.ts`: the same commitment list, plus the line
  "{name} read this in {language}{, machine translation}" for each half
  (closes the backlog row "The receipt card does not show what each party
  read").
- `templates/booking_progress.ts` → `agreement_progress.ts`
  (`.card-agreement-progress`): the agreement state and one line per
  commitment with its state word.
- `templates/payment_request.ts`, `payment_acknowledgement.ts`,
  `fulfilment_receipt.ts`: name the commitment (its description from the
  agreement) and the role words "giver" / "receiver".
- `registry.ts`, `render.ts`, `samples.json`, the gallery (`main.ts:314`).
- `en.ftl`, `hi.ftl`: new keys for commitment lines, agreement and
  commitment states, the read-in line, `quote-give-label`,
  `quote-payment-description`; `progress-title` becomes "Agreement
  progress". Section 7.10 lists them for the owner's Hindi review.
- Unit tests: section 7.5.
- Playwright: cases 3, 24, 25, 27, 29 and 30 (section 7.6);
  `demo/40-roym-languages.spec.ts:112` to `:122` reads
  `terms.scope.text`; `hub-helpers.ts` `sendQuote` unchanged (the new field
  is filled in advance).

Test: `npm test` in `crates/roym_web/ui`, `mise run test:e2e`.

#### Step 6 — The counterparty's own seat hold (AG-15)

The code is in step 5a. This step adds the in-crate tests: a quote where
the counterparty gives a `service-in-slot`; `agreement.accept_at` holds the
seat before signing; a taken seat answers `reserve-failed` with
`slot-taken` and signs nothing; a repeated accept does not hold twice.

#### Step 7 — Time: tick, track end and expiry

In-crate tests with chosen times (section 7.2): AG-7 (both
`ended-unconfirmed` cases), AG-8's words in the snapshot, a tick before the
end changes nothing, the late half after the end is kept as evidence and
changes nothing (scenario 165's second half).

#### Step 8 — Demo story 50 (start)

`crates/substrate/tests/e2e/demo/50-roym-deal.spec.ts`, using the two demo
nodes and `hub-helpers.ts`, as story 40 does:

1. Alice sends Bob an enquiry: "Hindi lessons for my son".
2. Bob opens the quote form, writes "One lesson, one hour" in "What you
   give", the price 100, currency INR, payee himself, and sends.
3. Alice's Hub shows two lines: "Bob gives: One lesson, one hour" and "You
   give: ₹100".
4. Alice accepts. Both Hubs show the agreement with two commitments, each
   "promised" (read through `agreement.list`, as story 40 does).

C6 changes step 2 to add the sack of wheat, and adds the totals, settlement
and statement. Add the story to the `demo:` list of the agreements doc at
C8; until then this plan's exit criteria name it.

#### Step 9 — Sizes, docs and close-out of C1a

- Component sizes before and after (section 7.9), into task.md section 1
  and the status row.
- Living docs (section 9) for every statement C1a makes false.
- Deferred backlog (section 8).
- Agreements doc `## Deviations`: the decisions marked "deviation".
- `mise run verify`, no `--skip`.

### C1b

Branch `feat/m08c-c1b-single-writer` from `main` after C1a merges.

#### Step 1 — Signed `deal-decline`

- `roym_core`: `crates/roym_core/src/transaction/decline.rs`:
  `DealDeclinePayload { declines: String, deal: String, conversation:
  String, reason: Option<LocalText> }` (512 bytes), `DEAL_DECLINE_VERSION =
  1`, `verify_deal_decline` (subject = `declines`, no expiry);
  `RECORD_DEAL_DECLINE` in `RECORD_TYPES`, `deal-decline` in `CARD_TYPES`;
  `router.rs`: `deal.`.
- `transaction`: `app/decline_ops.rs` with `decline_at`: who may decline
  what (`change.md:358` to `:359`): the counterparty a quote version or its
  own enquiry, the quote author an enquiry; refused once the counterparty's
  half exists; idempotent through a fence row `decline:<deal>` that holds
  the record (AG-13). Sync files a `deal-decline` card from the other party
  and marks the deal or enquiry `declined`. Delete `quote_ops::decline`
  (`quote_ops.rs:508`) and the "new version clears a decline" code
  (`store_received_quote`, `:605` to `:615`).
- Hub: `openDeclineDialog` (`messages.ts:724` to `:757`) calls
  `deal.decline`; the note becomes "They will see that you declined. To ask
  for changes, send a message instead." (`messages-decline-note`, en and hi);
  a `deal-decline` card template; the gallery has eight types.
- `roymctl`: `transaction decline` calls `deal.decline`; its doc comment
  (`transaction.rs:104` to `:115`) no longer says "the other side is not
  told".
- Tests: parity scenarios 140c (`transaction_cards.rs:130`) and 147
  (`transaction_validation.rs:249`) become decline-record scenarios; the
  wire-refusal tables (`transaction_validation.rs:11` to `:45`,
  `transaction_cards.rs:647` to `:680`, `bundles.rs:91` to `:101`) list
  `deal.decline` and `deal.ping`; Hub case 31 checks the new sentence and
  that the other node files the card; two-node AG-10.

#### Step 2 — Version checks

- `agreement.accept_at`: refuse with `quote-superseded` when the version is
  not the record id in the `QUOTES` pointer row of its `quote_id` (that row
  always names the latest version; `build_quote_pointer`,
  `quote_ops.rs:419`).
- `quote.set_at`: `quote-already-accepted` when `AGREEMENTS` holds a
  counterparty half for any version of this `quote_id` (index the
  agreement row by `quote_id`); `quote-ended` from the sources in "What
  `quote.set` reads" (`change.md:615`).
- Hub: "Accept" and "Decline" only on the newest version of a `quote_id`
  (point 6), and not when the deal is `agreed`, `declined`, `expired` or
  `not-formed`.

#### Step 3 — The writer's refusals

In `decide::run`, before step 2 of D-C1-16: the checks of the doc's step 1.
A refusal stores `snapshot:<version record_id>:1` (`not-formed`, the code
in `state_reason`), sends it, and creates `refused:<half record_id>` =
`{ quote_id, code, at_secs }` last. Step 2's fence that finds another
agreement refuses with `quote-already-agreed` the same way. The
counterparty files a refusal snapshot for that version only: it releases
the seat holds it took for that version, and the Hub shows the refusal on
that version's card. Expiry: `quote-expired` as AG-43.

#### Step 4 — Recovery of `deciding` rows

On every `transaction.sync`, query `ledger` for `kind = "decision"` and
`state = "deciding"` (add `state` to the ledger's indexes,
`app.rs:357` to `:362`) and finish each one in the doc's three-check order
(`change.md:607` to `:611`). The crash-point tests use
`fail_write_after(n)` for every write of the decision and the stub
`Reservations` that refuses `confirm` (section 7.2).

#### Step 5 — Restore rules and forks

- The writer's next `seq` is one more than the highest of: its stored
  snapshots, and its own `agreement-progress` cards in the conversation
  (read through `conversation.changes` from 0 for that conversation only
  when no stored row exists for the next `seq`).
- Before it signs a countersigned half or a snapshot with no stored row, it
  looks for its own card with that record in the conversation, and stores
  and reuses it.
- The counterparty: same `seq` and same bytes → repeat (D-C1-14); same
  `seq`, other bytes → keep the first, store the second as evidence, and
  show a `progress-fork` warning on the card.
- Write Q5's limit into the agreements doc and the backlog.

#### Step 6 — Close-out of C1b

As C1a step 9.

## 7. Tests

Test names say what they prove, with no planning ids.

### 7.1 `roym_core` unit tests (C1a)

`deal/tests.rs`:

- `every_giver_can_give_every_built_resource_kind` (AG-2).
- `commitment_count_table` with 0, 1, 16, 17 (AG-4).
- `missing_or_wrong_commitment_fields_are_refused` (AG-4).
- `commitment_ids_must_follow_their_position` (AG-37).
- `unknown_currency_is_refused` (AG-5).
- `declared_value_range` with -1, 0, 10^15, 10^15 + 1, and a currency of 0
  (AG-23 range).
- `currency_only_fields_on_a_service_are_refused`, `payee_is_required_on_money`,
  `tax_and_fees_never_exceed_the_value`.
- `after_names_known_ids_with_no_cycle`.
- `scheduling_details_must_name_the_slot_of_the_window` (Q3).
- `every_text_shares_one_language_and_one_translation_set` (AG-39, Q4).
- `a_quote_with_and_without_money_validate_the_same_way` (AG-22).
- `quote_for_an_enquiry_and_for_a_want_give_equal_receipts` (AG-3): builds
  both forms; the receipts are equal except `quote_record_id`, the first
  snapshots except `agreement`; `quote.set`'s refusal of `want` is a
  service test.

`deal/transition/tests.rs`:

- `fulfilment_table_covers_every_state_and_event` and
  `currency_table_covers_every_state_and_event` (AG-6): one row per (state,
  event), written as the doc's table.
- `track_end_uses_the_later_of_window_and_agreement`.
- `progress_apply_closes_only_when_every_commitment_is_final` (AG-6).
- `a_failed_commitment_cancels_the_open_ones` (AG-36 tables).
- `cancel_needs_every_commitment_open` (AG-9 rule).
- `ended_unconfirmed_says_whether_the_giver_claimed` (AG-8).

`transaction/tests*.rs`, `payment/tests.rs`, `fulfilment/tests.rs`,
`progress/tests.rs` (renamed from `booking/tests.rs`): today's tests on the
new shapes; `halves_differ_only_in_role_and_what_was_read` (AG-32);
`snapshot_from_another_signer_is_not_filed` (AG-11, verify part);
`record/tests/wire_words.rs` for the new enum words.

C1b: `decline` verify tests; `same_seq_other_bytes_is_a_fork` (AG-12 unit).

### 7.2 In-crate service tests (`crates/roym_transaction/src/app/tests/`)

C1a (each file under 800 lines):

- `accept.rs`: `authors_own_accept_is_refused` (AG-41);
  `repeated_accept_returns_the_same_half` (AG-13);
  `counterparty_holds_its_own_seat_before_signing`,
  `taken_seat_answers_reserve_failed_and_signs_nothing` (AG-15).
- `decide.rs`: `writer_countersigns_in_sync_and_sends_snapshot_one`;
  `losing_the_last_seat_ends_not_formed` (AG-14, one node, two quotes).
- `time.rs`: `unsettled_commitment_ends_unconfirmed_after_its_track_end`,
  with and without a giver's half (AG-7); `tick_before_the_end_changes_nothing`;
  `a_late_half_after_the_end_is_kept_and_changes_nothing`.
- `settle.rs`: `receiver_half_settles_with_a_settlement_note` (AG-33);
  `one_payment_per_currency_commitment`; `repeated_sign_and_ack_return_the_same_record`
  (AG-13); `a_half_from_the_wrong_party_is_not_filed` (AG-11).
- `cancel.rs`: `cancel_is_refused_once_a_commitment_moved` (AG-9).

C1b:

- `refuse.rs`: `accepting_a_superseded_version_is_refused` (AG-12);
  `two_syncs_send_one_refusal` (AG-12, AG-43);
  `a_countersigned_half_before_expiry_still_forms_the_agreement` (AG-43);
  `revision_after_acceptance_is_refused`, `revision_after_an_end_is_refused`
  (AG-35).
- `recover.rs`: one test per crash point of D-C1-16 (after the fence, after
  each reserve, after the countersign row, after the snapshot row, between
  the send and the last `put`), each followed by one sync and compared with
  the no-crash end state; the same with the time moved past the expiry;
  the stub that refuses `confirm` after another succeeded; that stub with a
  crash after the `not-formed` snapshot is stored and the slot free again
  on the rerun (AG-14).
- `restore.rs`: `restored_writer_continues_seq_from_its_cards` and
  `restored_writer_reuses_its_sent_countersign` (AG-42, Q5 scope).
- `decline.rs`: `repeated_decline_returns_the_same_record` (AG-13).

### 7.3 Parity (both builds agree)

C1a: every scenario of section 2.2 point 9 on the new shapes, plus a new
`deal.rs` with a two-commitment quote, a three-commitment quote with goods,
cancel, and per-commitment settlement. Admission tables list every method
(AG-31). C1b: the decline and refusal scenarios.

### 7.4 Two-node (`crates/substrate/tests`)

C1a: `roym_transaction_e2e.rs` adds `each_party_quotes_the_other_at_once`
(AG-1) and `service_for_service_and_goods_deals_close` (AG-2, AG-4, AG-22),
each in the shared harness (`SubstrateTestContext`, `alloc_ports`), in a new
file `roym_deal_e2e.rs` so the existing file stays under 800 lines;
`roym_booking_e2e.rs` keeps the race (AG-14). C1b: `decline_reaches_the_other_node`
(AG-10) and `refused_superseded_acceptance_shows_on_that_version` (AG-12).

### 7.5 Hub unit tests (`npm test`)

- `quote_terms.test.ts`: the D-C1-7 mapping (amount 0, before-work,
  after-work, slot).
- `templates.test.ts`: quote, receipt (with the read-in line), progress,
  payment, fulfilment templates; the old `renderBookingProgress` block
  (`:481` to `:528`) becomes the progress template's.
- `wording.test.ts`: roles and progress words (`:21`, `:100` to `:130`).
- `messages*.test.ts`, `hindi-screens.test.ts`, `messages_translate.test.ts`:
  fixtures on the new shapes.
- C1b: `newestVersionMap`; Accept only on the latest; decline calls
  `deal.decline`.

### 7.6 Playwright

| Case | File:line | C1a change | C1b change |
| --- | --- | --- | --- |
| 3 | `roym-hub.spec.ts:200` | `agreement-progress` in the 7 types | 8 types with `deal-decline` |
| 4 | `:224` | `enquiry` card | |
| 24 | `:840` | sample cards on new shapes; payee sentence on the money line | |
| 25 | `:870` | `renderCard` data on new shapes | |
| 26 | `:973` | `.card-enquiry` | |
| 27 | `:1001` | none (refused card) | |
| 29 | `:1029` | quote form opened from an `enquiry` card | |
| 30 | `:1073` | expired quote data on new shapes | |
| 31 | `:1125` | none | new decline sentence; the other side is told |
| story 40 | `demo/40-roym-languages.spec.ts:112` | `terms.scope.text` | |
| story 50 | new | two commitments, accepted | |

### 7.7 Not tested

- A slot the counterparty gives, from the Hub: the Hub form builds only
  today's case (D-C1-7). Service tests cover it.
- A full restore from an old backup (Q5).

### 7.8 Size and time risks

- `transaction.wasm` may pass +15%: it gains deal validation and the
  transition tables, and loses the booking code. Check after step 5a; if
  over, stop and tell the owner.
- `dual_build_parity/transaction.rs` is capped at 846 lines and
  `helpers.rs` at 1290: add new scenarios in new files.
- `messages.ts` is 1016 lines: step 5b moves term building out.
- The decision code: keep each step a function of its own in `decide.rs`
  (`fence`, `reserve_all`, `confirm_all`, `countersign`, `first_snapshot`,
  `finish`), so none nears 100 lines.
- The Hub's thread redraw removes an open quote form (M08B B6 R6, R13). Story
  50 uses `hub-helpers.ts` `sendQuote`, which already waits for this.

### 7.9 Component size

Build the six Roym components with `mise run build:roym` on `main` and on the
branch, in the same build folder, and write before and after into task.md
section 1 and the status row. Baseline in section 2.2, point 10. Expected:
`transaction` changes; the other five change by a few bytes or not at all.
C1b reports again.

### 7.10 Hindi text

New `hi.ftl` messages need the owner's review before merge, as in M08B: C1a
the commitment line, the read-in line, the agreement and commitment state
words, `quote-give-label`, `quote-payment-description`; C1b the decline
note (it has legal weight: it says the other side is told) and the refusal
words (`quote-superseded`, `quote-expired`, `progress-fork`).

### 7.11 How C1 avoids the kinds of findings in the M08B reviews

- **Rows that prove less than they say** (B2 R8, B4a R4, B5 R3, B6 R4):
  section 3 says which part each row proves and names the "Later" part.
- **Criteria met in part marked done** (B3a P7, B4b P13, B5 P11): each later
  part is a deviation row now, as task.md section 5 asks.
- **Wrong test names in the Test column** (B3a R2, B3b R3): fill the
  agreements doc's test file names only after the tests exist, with
  `git grep` on each.
- **Living docs left false** (B2 P6, B5 R1): section 9 names the sections.
- **Copied helpers** (B4b R4, B3a R14): one test host (D-C1-3), one
  `newestVersionMap`, one `Reservations` trait.
- **Tests that cannot fail** (B4b P2, B5 R3): each crash-point test first
  checks that the fault fired (the call returned the injected error).
- **`?.` in tests** (B2 R3, B3a R4): assert with `!` or a count.
- **A redraw removes an open form** (B6 R6, R13): see 7.8.
- **Status ahead of facts** (B1a R7): "Merged" only after the merge.
- **Planning ids in names** (B2 R5): none in test names or code.

## 8. Deferred backlog

[deferred-backlog.md](../../deferred-backlog.md):

| Row | C1 action |
| --- | --- |
| "The payment and fulfilment records name the deal parties, not the giver and receiver" (`:657`) | C1a moves it to "Recently resolved". |
| "The receipt card does not show what each party read" (`:562`) | C1a moves it to "Recently resolved". |
| "Hub transaction action panel and Playwright transaction spec (cases 33-39) are not built" (`:654`) | C1b: quote accept and decline are built; the row keeps booking steps, payments and fulfilment (C6 shrinks it further). |
| "`RECORD_TYPES` is not read by any verifier" (`:656`) | C1a and C1b update the type list in the row. |
| "Two Roym outputs carry another node's card data as an untyped `Value`" (`:532`) | C1a: `PeerStep` is gone; `agreement.progress` returns typed snapshots. Update the row's file names. |
| New: "The writer's decision has no crash recovery" | C1a adds it, target C1b; C1b resolves it. |
| New: "Nobody can mark work as started" (Q6) | C1a adds it, target C3b. |
| New: "The restore rules do not cover a full restore from an old backup" (Q5) | C1b adds it, target TBD. |
| New: "The Hub's quote form builds only one service and one payment" | C1a adds it, target C6. |

No new `TODO` or `FIXME` marker is planned. If one is added, it gets a row
in "Open in-code markers".

## 9. Living docs to update at close-out

Each PR fixes the statements it makes false. C8 does the full pass of the
change docs' close-out lists.

- `docs/roym-integrated-experience-spec.md`: the R2 transaction table
  (`:244` to `:253`), the Records table (`:463` to `:524`; request at
  `:475`, booking-progress at `:500`), the transaction state section
  (`:525` to `:559`), the Cards table (`:733` to `:760`).
- `docs/developer-guide.md`: the event kinds table (`:2489` to `:2490`),
  "Show cards" (`:2537` to `:2550`, the card types and `agreement.accept`
  params).
- `docs/system-architecture.md`: Cards (`:1203` to `:1217`, `:1217` names
  `quote.decline`, C1b), the Booking State Machine (`:1219` to `:1269`), the
  flow diagram (`:1271` to `:1324`).
- `docs/TERMINOLOGY.md`: enquiry, commitment, declared value, progress
  snapshot (`:64` to `:67`; `:67` says the code names the parties, not giver
  and receiver, which C1a makes false).
- `docs/demo/demo.md`: the `roymctl` transaction commands (`:1311` to
  `:1327`).

## 10. Exit criteria

C1a:

- [ ] Every C1a row of section 3 names a test that exists and passes.
- [ ] Story 40 and story 50 pass; Hub cases 3, 4, 24 to 30 pass.
- [ ] Component sizes recorded; `transaction` within +15%, others within
  +40 KB.
- [ ] Deviations, backlog rows and living docs done.
- [ ] `mise run verify` passes with no `--skip`.
- [ ] Owner reviewed the new Hindi messages.

C1b:

- [ ] Every C1b row of section 3 names a test that exists and passes,
  including one test per crash point.
- [ ] Hub case 31 checks the new sentence.
- [ ] Q5's limit is written in the agreements doc and the backlog.
- [ ] `mise run verify` passes with no `--skip`.

## 11. What C1 does not do

- Hide anything: no `_sd`, salts, disclosures or `deal.present` (C2).
- Move slots out of `transaction`, or give seat holds an expiry (C3b).
  `confirm` never fails in C1, so `commitment-failure` and
  `reservation-lost` do not exist yet (C3b).
- Read a manifest or check an extension schema, or build one-shot (C3a).
- Step records, so no "work started" signal (C3a, C3b; Q6).
- A Hub form for any mix of commitments, totals, settlement actions, the
  event log or the statement (C6).
- EEP kinds, wants and bids (M08E, M08F, M08H).
- Any wire-reachable method.
