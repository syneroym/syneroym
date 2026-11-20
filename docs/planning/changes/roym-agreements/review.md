# Roym agreements — review

*Reader: the author of the agreements change document, the fix session, and the project owner.*

Document: [change.md](change.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file". Finding ids are P1, P2, … and are never renumbered.

## Doc review round 1 — Claude Opus 5.5 (Claude Code), 2026-11-10

Checked against the code at `69d5a640` on
`claude/m08c-agreements-doc-review-1cd783` (the draft's branch tip; the code
is the same as `main` at `4830bdf7`). Line numbers below are at that commit.
"change.md:N" is a line of the draft. The draft was written by the same
model, so a second reviewer with another tool is still useful.

**Summary:** 1 blocker, 10 major, 13 minor.

**Verdict on the possible bug at `ledger.rs:52`:** confirmed, by reading the
code (not run). Two versions of one quote can both become agreements. See
"The `ledger.rs:52` bug" below.

What was checked and found correct:

- **Most facts in "Facts checked in the code".** These match the code:
  the quote id derivation
  ([transaction.rs:524](../../../../crates/roym_core/src/transaction.rs#L524));
  `agreement.accept` and `maybe_countersign`
  ([agreement_ops.rs:99](../../../../crates/roym_transaction/src/app/agreement_ops.rs#L99),
  [:402](../../../../crates/roym_transaction/src/app/agreement_ops.rs#L402));
  the unique insert `AppDataLayer::create`
  ([ledger.rs:116](../../../../crates/roym_transaction/src/app/ledger.rs#L116),
  [:236](../../../../crates/roym_transaction/src/app/ledger.rs#L236));
  `booking-progress` signed by `Principal::Service`
  ([progress.rs:32](../../../../crates/roym_transaction/src/app/progress.rs#L32))
  and bound to the quote's `signer_did`
  ([receipts.rs:89](../../../../crates/roym_transaction/src/app/sync/receipts.rs#L89));
  the "against interest" rule
  ([booking.rs:260](../../../../crates/roym_core/src/booking.rs#L260));
  the local, unsigned decline
  ([quote_ops.rs:508](../../../../crates/roym_transaction/src/app/quote_ops.rs#L508));
  key-sorted `serde_json` ids
  ([envelope.rs:213](../../../../crates/signed_record/src/envelope.rs#L213));
  the 160 KiB card body
  ([card.rs:63](../../../../crates/roym_core/src/card.rs#L63));
  `admit::require_internal` on every `transaction` method
  ([app.rs:278](../../../../crates/roym_transaction/src/app.rs#L278));
  seven card types
  ([card.rs:9](../../../../crates/roym_core/src/card.rs#L9));
  `MAX_TRANSLATIONS = 4`
  ([lang.rs:7](../../../../crates/roym_core/src/lang.rs#L7));
  no `viewed_lang` in `AgreementReceiptPayload`
  ([transaction.rs:234](../../../../crates/roym_core/src/transaction.rs#L234)).
  Three facts are slightly wrong. See P15.
- **The "Problem" section.** `AgreedTerms` is at `transaction.rs:153`. Both
  receipt halves, each `fulfilment-receipt` half
  ([fulfilment.rs:36](../../../../crates/roym_core/src/fulfilment.rs#L36)) and
  each `payment-acknowledgement`
  ([payment.rs:94](../../../../crates/roym_core/src/payment.rs#L94)) carry
  plain terms or amounts. So the draft is right that settlement records must
  also hide fields. The overview names only the quote and the receipt.
- **Limits and constants.** Scope 4096, terms text 2048, address 512,
  payee 256, 16 payment methods of 32 bytes
  ([transaction.rs:57-66](../../../../crates/roym_core/src/transaction.rs#L57)),
  quote life 300 s to 90 days (`:70-71`), `TRACK_WINDOW_SECS` 30 days
  ([booking.rs:18](../../../../crates/roym_core/src/booking.rs#L18)),
  `ONE_PAYMENT_NOTICE` (`booking.rs:35`), `SCHEMA_VERSION = 4`
  ([app.rs:46](../../../../crates/roym_transaction/src/app.rs#L46)),
  collections `bookings` and `progress` (`app.rs:56-57`), and the helper
  names `money::currency_minor_exponent`, `person::is_did_key`,
  `lang::check_text`, `lang::check_translation_langs`. All exist as named.
  A quote with `amount_minor = 0` is valid today
  ([transaction.rs:325](../../../../crates/roym_core/src/transaction.rs#L325)),
  so the reason for open question 9 holds.
- **Coverage of the overview.** Every VX criterion that the M08C `task.md`
  gives to this module (VX-1, VX-2, VX-6 to VX-12, VX-45, VX-46) maps to at
  least one AG row. VX-13, VX-14 and VX-27 are worded for one node or the
  reference code, as the overview's notes ask. All three notes under
  "Agreements" and the one note under "Event log and statement" are
  covered. Gaps: VX-11 for a quote (P8), and VX-45 for EEP events (P7).
- **SD-JWT basics.** The disclosure (`[salt, name, value]`, base64url of the
  UTF-8 JSON), the digest (SHA-256 over the US-ASCII bytes of the disclosure
  string, then base64url), the duplicate-digest rule, and the "name already
  exists at this level" rule match RFC 9901, sections 4.2.1, 4.2.3 and 7.1.
  The draft's choice to digest the string as sent, so our canonicaliser never
  rebuilds the JSON, is correct (RFC 9901, section 4.2.1: "no
  canonicalization needs to be performed"). An `_sd` key may sit at any level
  (section 4.2.4.1), so one `_sd` list per commitment object is allowed.
- **Who signs what.** The table at change.md:404-412 agrees with the record
  sections, except for the reservation owner (P6).
- **Counting.** 7 card types + `deal-decline` + `commitment-failure` + the
  extensions' 3 = 12. The draft's count is right. The extensions change still
  says 10 (P7).

### The `ledger.rs:52` bug

The draft's claim is right, and the bug is wider than the draft says. There
are two paths. Both are reachable from the Hub.

1. **Revise after an agreement.** `quote.set` with a `quote_id` checks only
   that the quote is the caller's, in the same conversation, for the same
   request ([quote_ops.rs:285-308](../../../../crates/roym_transaction/src/app/quote_ops.rs#L285)).
   It never reads `agreements`. So the author can send version 2 after
   version 1 formed an agreement. The counterparty accepts version 2. The
   author's `transaction.sync` files that half
   ([sync.rs:448-527](../../../../crates/roym_transaction/src/app/sync.rs#L448)).
   The `agreements` row is keyed by the version's `record_id`, so this is a
   new row. `maybe_countersign` calls `decide_booking`, which inserts
   `decision:<version 2 record_id>`
   ([booking_ops.rs:124-141](../../../../crates/roym_transaction/src/app/booking_ops.rs#L124),
   [ledger.rs:52](../../../../crates/roym_transaction/src/app/ledger.rs#L52)).
   That id does not exist yet, so the insert wins, and the author
   countersigns. Now one quote id has two agreements.
2. **Accept an old version.** Each version is its own card in the chat. The
   Hub shows "Accept" on every verified quote card for me that is not
   expired or declined. It does not check that the card is the latest
   version ([messages.ts:598-617](../../../../crates/roym_web/ui/src/screens/messages.ts#L598)).
   `agreement.accept` loads the version from `quote_history` by its
   `record_id` and never compares it with the `quotes` pointer
   ([agreement_ops.rs:168-214](../../../../crates/roym_transaction/src/app/agreement_ops.rs#L168)).
   The author's sync also does not check supersession
   ([sync.rs:465-495](../../../../crates/roym_transaction/src/app/sync.rs#L465)).
   So a counterparty can accept version 1 after version 2 arrived, and the
   author countersigns a superseded version. If the counterparty then also
   accepts version 2, there are two agreements, with no revision after
   acceptance at all.

The seat rows limit the damage in one case only. For a slot of capacity 1,
the second version gets `conflict`, because the seat key is per slot. For a
quote with no slot, or a slot with capacity 2 or more, both versions form
agreements. No test covers either path: no test in `roym_transaction` or
`crates/substrate/tests` accepts a superseded version.

The draft's fix (`decision:<quote_id>`, `quote-superseded`,
`quote-already-agreed`, no revision after acceptance) closes both paths on
the writer. It still has gaps: the counterparty is never told (P2), the key
makes `not-formed` and `declined` permanent for that quote id (P3), and the
fence cannot be one insert any more (P1). The Hub fix is also needed: show
"Accept" only on the latest version of a quote id. Path 2 is a bug in today's
code. It may deserve a backlog row now, even before M08C.

### Owner questions — reviewer view

I do not re-decide any question. These notes are only about the reasons.

- **Q1 (declared values).** Agree. One effect is not in the draft: the
  statement's dates and periods (Q10, AG-29) depend on which time is used for
  "settled". See P5.
- **Q3 (manifest and registry).** Agree. It makes P1 more urgent: if each
  extension is its own crate with its own reserve methods, it is its own
  service with its own data layer.
- **Q4 (15-minute expiry).** Agree. The reason holds only when the writer's
  reservation and decision are written together, or in a fixed order with a
  defined recovery. See P1.
- **Q5 (reserve at signing time).** Agree. But "the same result" is true
  only after P1 and P6 are fixed.
- **Q6 (visible fields).** Agree with the list. The visible
  `commitment_id` is a hole in it. See P10.
- **Q7 (signed decline).** Agree. The draft must say whether a decline ends
  the quote id or only one version. See P3.
- **Q9 (one side only).** Agree. Today's code allows `amount_minor = 0`.
- **Q10 (statement period).** Agree with "settled in the period". The
  draft's log uses the time the local node recorded the event, so the two
  parties can put one commitment in different periods. See P5.
- **Q11 (forwards carry no disclosures).** Agree. FW-7 in the forwarding
  change must then be reworded. See P7.

### Findings

- [x] **P1 — blocker.** *Where:* change.md:148, :464, :481, :705, :747; AG-14.
  Code: [ledger.rs:116](../../../../crates/roym_transaction/src/app/ledger.rs#L116),
  [ledger.rs:142](../../../../crates/roym_transaction/src/app/ledger.rs#L142),
  [data-layer.wit:161](../../../../crates/wit_interfaces/wit/data-layer/data-layer.wit#L161),
  [data-layer.wit:174](../../../../crates/wit_interfaces/wit/data-layer/data-layer.wit#L174).
  *Problem:* the fence says the writer inserts `decision:<quote_id>`
  "together with its own reservation rows, in one `AppDataLayer::create`
  call". That cannot be built with the rest of the design. `create` and
  `batch-mutate` work on one collection of one service. Today the seat rows
  are in `transaction`'s own `ledger` collection, and that is the only reason
  the claim is atomic. Slot data already lives in `catalog`, and `transaction`
  reads it with a sibling call (`ledger.rs:142`). The draft moves the seat
  rows to the scheduling extension (change.md:747), and makes reserve,
  confirm and release "the extension's" methods (change.md:705). EX-17 says
  `transaction` holds no slot code. So the reservation is in another
  service's data, and the two writes are separate. The draft says nothing
  about their order or about a crash between them. A junior developer cannot
  build AG-14 from this, and Q4's reasoning depends on it.
  *Fix:* replace the sentence with an exact order and a recovery rule. A
  proposal: (1) insert `decision:<quote_id>` with state `deciding` and the
  accepted version's `record_id`; (2) call each local `reserve`, keyed by
  (agreement id, commitment id), which EX-21 makes idempotent; (3) on success,
  countersign, confirm, and update the decision row to `agreed`; on failure,
  release, and update it to `not-formed` with the reason. On restart, a
  `deciding` row is finished by running steps 2 and 3 again, which is safe
  because reserve and release are idempotent. Say that a reservation left by
  a crash expires on its own (EX-20). Change "Facts" and AG-14 to match.
  If the owner wants the scheduling extension linked into `transaction` as a
  library with rows in the `ledger` collection, say that instead, and update
  EX-17 and change.md:705.
  *Status:* Fixed (f638722b). Agreed. New section "The writer's decision": (1) checks, (2) fence row `decision:<quote_id>` with state `deciding`, (3) idempotent local reserves, (4) confirm, countersign under a `countersign:<agreement>` unique insert (so a repeat never signs twice), then put the row to `agreed` or `not-formed`. Every sync finishes `deciding` rows. Facts row and AG-14 updated. Linking scheduling into `transaction` is listed as a rejected option (against EX-17).

- [x] **P2 — major.** *Where:* AG-12 (change.md:109); "Deal and agreement"
  table (change.md:426-438); change.md:464.
  *Problem:* the named conflicts have no carrier, and `accepted` has no
  defined way out on the counterparty's node. All traffic is cards
  (change.md:183). The writer finds `quote-superseded` or
  `quote-already-agreed` inside `transaction.sync`, where there is no caller
  to answer. The draft does not say what the writer sends. So the
  counterparty's node stays in `accepted` and keeps its own reservations until
  they expire. Also, `expired` is "written by writer" for both `quoted` and
  `accepted`. But change.md:438 says the states before `agreed` come from
  signed records each node holds. A writer that is offline for good can never
  write `expired`. AG-12 cannot be tested "two-node" as written, because no
  test can observe the error code. The draft also does not say if the
  counterparty may withdraw its acceptance, or decline while in `accepted`.
  *Fix:* (a) The writer answers a refused acceptance with an
  `agreement-progress` snapshot, seq 1, state `not-formed`, and
  `state_reason` one of `quote-superseded`, `quote-already-agreed`,
  `quote-expired`, `slot-taken`, `reserve-failed`. List these codes in one
  table. (b) `quoted → expired` and `accepted → expired` are computed on each
  node from `quote_expires_at_secs`. A countersigned half whose
  `issued_at_secs` is before the expiry still wins when it arrives later.
  (c) `agreement.accept` on the counterparty's node refuses a version that is
  not the latest one it holds, with `quote-superseded`, so the common case is
  a method error. (d) Say that a counterparty cannot withdraw an acceptance
  (or define how). Reword AG-12's test: two-node, and the counterparty's
  node shows `not-formed` with the reason.
  *Status:* Fixed (f638722b). Agreed with (a) to (d). New "Reason codes" table; the writer answers with a `not-formed` snapshot, `seq` 1. `expired` is computed on each node, and a countersigned half signed before the expiry still wins (AG-43). `agreement.accept` refuses a non-latest version, and the Hub shows "Accept" only on the latest card. `accepted` cannot be withdrawn or declined; the expiry bounds the wait. AG-12 reworded.

- [x] **P3 — major.** *Where:* change.md:24, :344-345, :464-465; the state
  diagram (change.md:418-424). Code:
  [quote_ops.rs:605-611](../../../../crates/roym_transaction/src/app/quote_ops.rs#L605).
  *Problem:* the key `decision:<quote_id>` plus "no revision after
  acceptance" makes some ends permanent for the whole quote id, and the draft
  does not say so. (a) After `not-formed` (for example `slot-taken`), the
  decision row exists, so no later version of that quote id can ever form an
  agreement. The author must start a new quote id. Today the author can send
  a new version with another slot. (b) `deal-decline.declines` names one
  version (change.md:337), but `declined` has no way out. Today a new version
  clears a decline ("New version: fresh offer, not declined",
  `quote_ops.rs:610`). VX-5 says either side can ask for changes until one
  side accepts or rejects. It is not clear if a decline rejects the deal or
  one version. (c) A "deal" is "one enquiry, the quotes that answer it" and
  also "named by its quote id" (change.md:24). One enquiry can get several
  quote ids today (`resolve_quote_sequence`, `quote_ops.rs:310-314`), and the
  fence allows each to form an agreement.
  *Fix:* state the rules in one place: whether `declined` ends the quote id
  or only the declined version; that after `not-formed` the author sends a
  new quote id (or that `not-formed` does not take the decision row); and
  whether two quote ids for one enquiry may both form agreements. Then define
  "deal" to match. Add these to AG-10 and AG-12.
  *Status:* Fixed (f638722b). Agreed. Chose the simplest rule, in a new "Deal rules" section: every end (`declined`, `expired`, `not-formed`, `cancelled`, `closed`) ends the quote id, `quote.set` answers `quote-ended`, and a new offer is a new quote id (one click in the Hub). So the fence row is never reused. A decline of any version ends the quote id. Several quote ids per enquiry may each form an agreement. "Deal" is redefined as one quote id. New AG-35.

- [x] **P4 — major.** *Where:* AG-7, AG-8 (change.md:103-104); snapshot
  `missing_half` (change.md:365); commitment table and rules
  (change.md:446-456); AG-16.
  *Problem:* the failure ends do not show "which party failed which
  commitment" (VX-12) well. (a) Only the receiver's half settles a
  commitment. So a commitment ends `ended-unconfirmed` only when the
  receiver's half is missing, and `missing_half` is always the receiver. The
  field is redundant, and a reader will read it as "the receiver failed",
  also when the giver never delivered. What matters is whether the giver
  claimed. Today the track keeps that as `TrackState::Claimed` against
  `None` ([booking.rs:57-62](../../../../crates/roym_core/src/booking.rs#L57)).
  The draft's `ended-unconfirmed` loses it. (b) When a service commitment
  ends `failed` with `reservation-lost`, the payment for it stays `promised`.
  Cancel is refused, because not every commitment is `promised`
  (change.md:455). So the payment ends `ended-unconfirmed` 30 days later,
  and points at the payer. Evidence then blames the payer for a deal the
  slot owner could not give.
  *Fix:* (a) replace `missing_half` with `claimed: bool` (the giver's half
  exists), and say in AG-8 how the Hub words each case ("the giver says it
  delivered; the receiver did not confirm" against "nobody recorded
  anything"). (b) Define what a `failed` commitment does to the others. One
  simple rule: when any commitment ends `failed`, every commitment that is
  still `promised` or `payment-requested` ends `cancelled` with reason
  `other-commitment-failed`, written by the writer. Add a test.
  *Status:* Fixed (f638722b). Agreed with both. (a) `missing_half` became `claimed: bool`, and AG-8 gives the two Hub wordings. (b) When a commitment fails, every commitment still `promised` or `payment-requested` becomes `cancelled` with `other-commitment-failed` (AG-36).

- [x] **P5 — major.** *Where:* "Where the event log lives" (change.md:579-583);
  `EventLogEntry.at_secs` (change.md:592); the statement rows
  (change.md:627-628); AG-27 to AG-29.
  Code: [data-layer.wit:161-174](../../../../crates/wit_interfaces/wit/data-layer/data-layer.wit#L161).
  *Problem:* (a) The main reason to keep the log inside `transaction` is
  "a service can write a log entry in the same data-layer call as the state
  change it records". The log is its own collection, `event_log`. Every
  data-layer write (`put`, `create`, `batch-mutate`) takes one collection. So
  the append is never in the same call as the progress row. The claim is
  false as designed. `eventlog.rebuild` is what really makes the log
  complete. (b) `at_secs` is "when this node recorded the event". The two
  parties record one event at different times. A counterparty that is
  offline across a period boundary puts the commitment in the next period.
  The statement's `agreed_at_secs` and `settled_at_secs` then differ between
  the two parties, and they differ from the signed records. A tax statement
  needs the signed time. (c) If entries are ordered by a signed time, an
  older event appended late is skipped by the `eventlog.list` cursor.
  *Fix:* (a) Either put log entries in the same collection as the state rows
  (the `ledger` collection, with a `kind`), or drop "atomic append" and say
  that sync calls the same "derive missing entries" step as
  `eventlog.rebuild` every time. Keep the decision; change the reason. (b)
  Add `occurred_at_secs`: the `issued_at_secs` of the signed record behind
  the event (the settling half; the countersigned half for
  `agreement-formed`). Statement periods and dates use it. (c) Keep
  `recorded_at_secs` for the cursor. Say this in AG-29 and in Q10.
  *Status:* Fixed (f638722b). Agreed. (a) Kept the decision, changed the reason: completeness comes from running the derive step on every sync, not from an atomic append. (b) Added `occurred_at_secs` from the signed record; statement periods and dates use it. (c) `recorded_at_secs` is the `eventlog.list` cursor. AG-29 and Q10's reason updated.

- [x] **P6 — major.** *Where:* AG-15 (change.md:113); "Reserve, confirm and
  release" (change.md:478-483); "Commitment failure" (change.md:374-384);
  AG-8; who-signs table (change.md:411).
  *Problem:* the draft never says which node owns the reservation of a
  commitment. It uses "the counterparty owns a resource" and "a bid where the
  wanter books its own slot", but no record field says who the owner is.
  "The giver of a `service-in-slot` commitment" is the natural answer, but
  the wanter's own slot (change.md:480) is on the receiver's side. Then
  `commitment-failure` "signed by the giver", and "the writer accepts it
  only from that commitment's giver", are wrong for that case. Also AG-15
  says "or it made a bid". In Want-Bid the bidder is the quote author and the
  writer (change.md:462), so the counterparty never made a bid.
  *Fix:* add one rule and one field. For example: a commitment with an
  extension that has a reserve hook names `reserved_by: "quote-author" |
  "counterparty"` (visible), or the rule "the giver always reserves". Then
  use that party in AG-15, AG-16, `commitment-failure` and the who-signs
  table. Remove "or it made a bid" from AG-15.
  *Status:* Fixed (f638722b). Agreed. Chose the rule over a field: the giver of a commitment owns its reservation (new "Who reserves" part). A receiver-owned reservation is not in v1; M08F adds `reserved_by` if it needs one. AG-15, AG-16, `commitment-failure` and the who-signs table use this. "Or it made a bid" removed.

- [x] **P7 — major.** *Where:* change.md:10; `## Deviations` (change.md:857,
  empty); and the places below.
  *Problem:* the overview says each module document "must follow the rules
  here, or record a deviation". The draft says differences are "listed in the
  report to the owner, not fixed in the overview" (change.md:10). That report
  is a working note outside the repository. The draft's own differences are
  spread over the text, and some are not marked at all:
  - VX-45 and the overview's term "event log" say one log has every
    agreement and EEP event. The draft splits EEP events into a second log in
    `obligation` (change.md:577). No AG row or named document covers VX-45 for
    EEP events.
  - The overview says "each field of the quote is hashed with its own random
    salt", and amounts are "hidden fields by default". The draft hides a
    fixed subset and makes it fixed in code (change.md:491).
  - `_sd_alg` inside `terms` breaks RFC 9901, section 4.1.1 ("MUST NOT be
    used in any object nested within the payload"). See P12.
  - New states `accepted`, `declined`, `not-formed` (change.md:439).
  - The overview defines declared value as "what the giver would charge";
    Q1a makes the quote author write every value.
  - The extensions change's reserve text (change.md:485), its card count of
    10, and EX-5 ("the deal core calls the reserve hook of each extension").
  - FW-7 says a forwarded quote "verifies as it does today". With no
    disclosures it shows almost nothing: no scope, no description, no value.
  - `LocalText` per field replaces the multilingual change's record-level
    `translations` (see P16).
  *Fix:* add a section "Differences from the overview and sibling changes",
  one row each: the rule, what this document does, why, and which document
  is updated at close-out. For VX-45, name the obligations document as the
  owner of EEP events, and say that the statement joins both logs.
  *Status:* Fixed (f638722b). Agreed. New section "Differences from the overview and sibling changes", one row per difference with the document updated at close-out. Line 10 now points at it. The `_sd_alg` row is gone, because P12's fix removes that difference. `## Deviations` stays empty, because it is for code-against-plan differences during implementation.

- [x] **P8 — major.** *Where:* "Showing a deal to a third party"
  (change.md:542-554); AG-18.
  *Problem:* VX-11 says "a party can show a **quote** or an agreement with
  chosen fields hidden". `Presentation.records` holds receipt halves, the
  snapshot and settlement records. It cannot hold a quote, and no AG row
  tests showing a quote. Before an agreement, a party has only the quote,
  for example when it asks a contact "is this a fair price?". Q11 covers
  forwards only. It does not decide `agreement.present`.
  *Fix:* allow a `quote` envelope in `Presentation.records` (named, for
  example, `deal.present`). When a quote and a receipt half are both present,
  the verifier checks that their `terms` are equal. Add AG-18b: show a quote
  with the price hidden and the scope shown, unit and two-node.
  *Status:* Fixed (f638722b). Agreed. `Presentation.records` may hold a quote. The method is renamed `deal.present` (and `deal.verify-presentation`). A quote and receipt halves must have equal `terms`. New AG-34.

- [x] **P9 — major.** *Where:* "Tasks" (change.md:851-854);
  [M08C task.md](../../milestones/M08C-deal-core-and-extensions/task.md),
  slices C1 to C3.
  *Problem:* the slices do not fit the doc as cut. (a) C1 "replaces today's
  quote and receipt shapes" with `DealTerms`, which holds `_sd` digests. The
  settlement records of C1 also have hidden fields. But the encoder is C2. So
  C1 either ships plain `DealTerms` and C2 changes the canonical bytes again,
  or C1 needs C2's encoder. (b) C1 removes `slot_id`, `booking-progress` and
  the booking track, and says a `service-in-slot` commitment needs an
  extension with a reserve hook. That extension is built in C3. Between C1
  and C3, slot booking either does not work or keeps old code that the draft
  does not mention. (c) `commitment-failure` exists only for EX-22, a C3
  rule, but the draft puts it in C1.
  *Fix:* say what each slice ships and what works after it. One option: C1
  ships the commitment model with plain `DealTerms` (no `_sd`) and keeps
  today's slot path inside `transaction` behind the new shapes; C2 adds
  `_sd`, disclosures and the per-commitment hidden fields; C3 moves slots out
  and adds `commitment-failure`. Or merge C1 and C2. Ask the owner which.
  *Status:* Fixed (f638722b), as an owner question. Agreed that the cut does not fit. The Tasks list now follows the reviewer's first option (C1 plain `DealTerms` and today's slot path; C2 adds `_sd`; C3 moves slots and adds `commitment-failure`), and new open question 12 asks the owner to confirm it or merge C1 and C2. `task.md` is not edited here; the differences table names it.

- [x] **P10 — major.** *Where:* "Ids" (change.md:199); `commitment_id`
  (change.md:256); the snapshot (change.md:362); rejected option
  (change.md:756).
  *Problem:* `commitment_id` is chosen by the quote author, is
  `[a-z0-9-]{1,16}`, and is visible in the quote, the receipt and every
  snapshot. An author can write `inr-5000` or `haircut-bob`. The draft
  rejects "letting the quote author choose which fields are hidden" because
  "an author could make the price visible, and every receipt the
  counterparty shows would carry it". A free visible id lets the author do
  exactly that. AG-17's test cannot catch it, because the value is not a
  hidden field.
  *Fix:* the service assigns the id: `c1` to `c16`, by position, and
  `quote.set` refuses any other form. The Hub shows what changed between
  versions from the disclosed `description`, not from the id. Drop "the
  author should keep a commitment's id across versions".
  *Status:* Fixed (f638722b). Agreed. The service assigns `c1` to `c16` by position, and `quote.set` and `verify_quote` refuse anything else (AG-37). The "keep ids across versions" advice is removed. Also listed as a rejected option.

- [x] **P11 — major.** *Where:* "Progress snapshot" (change.md:349, :360);
  "What is hidden" (change.md:493); AG-8 (change.md:104); presentation
  verifier (change.md:551). Code:
  [booking.rs:326-328](../../../../crates/roym_core/src/booking.rs#L326),
  [receipts.rs:89](../../../../crates/roym_transaction/src/app/sync/receipts.rs#L89),
  [signing.wit:11-13](../../../../crates/wit_interfaces/wit/signing/signing.wit#L11).
  *Problem:* (a) The snapshot is signed with the service key. Its issuer is
  that key's `did:key`, not the person. Today the counterparty binds it by
  comparing the issuer with the quote's `signer_did`. A third party that
  gets a presentation has no quote. The draft does not say how its verifier
  knows the snapshot came from the writer. AG-8 ("a reader can see this with
  no disclosures") cannot be tested until it does. (b) The snapshot "holds
  no hidden value, so it can be shown to anyone", and "hidden always: the
  free text". But `cancel_reason` is the author's free text, 512 bytes, and a
  plain `string`, not a `LocalText`. It can hold a price or private
  information.
  *Fix:* (a) state the rule: the snapshot's issuer must equal the
  `signer_did` of the quote author's receipt half in the same presentation.
  Say what happens when the author's node changes key (for example after a
  restore to a new device): the binding fails and shows "writer not
  checked". (b) make `cancel_reason` a reason code in the snapshot, and move
  the author's text into a hidden field of a signed record, or say plainly
  that it is visible and the Hub warns the author.
  *Status:* Fixed (f638722b). Agreed. (a) The binding rule is stated in "Progress snapshot" and in the presentation verifier: issuer equals the `signer_did` of the author's receipt half or of the quote; otherwise "writer not checked", also after a key change (AG-38). (b) `cancel_reason` is now a `CancelReason` code; the author's own words go in the conversation as a normal message.

- [x] **P12 — minor.** *Where:* change.md:497, :505, :518; AG-19.
  *Problem:* the SD-JWT reference is out of date. It is RFC 9901,
  "Selective Disclosure for JSON Web Tokens", November 2025 (checked on
  rfc-editor.org). Also: RFC 9901 section 4.1.1 says `_sd_alg` MUST appear
  only at the top level, and when it is absent the default is `sha-256`. The
  draft puts it inside `terms`, and settlement records, which also have
  hidden fields, get none. RFC 9901 section 4.2.1 says the salt "MUST be a
  string", with 128 bits recommended. The 16-byte check is our own rule.
  *Fix:* cite RFC 9901 and remove `unverified`. Drop `_sd_alg` from every
  record: our verifier accepts only SHA-256 anyway, and an absent `_sd_alg`
  means SHA-256 in the RFC. This removes a deviation. Label the 16-byte salt
  and the sorted `_sd` as "our rules on top of RFC 9901".
  *Status:* Fixed (f638722b). Agreed. Cites RFC 9901; `unverified` removed. `_sd_alg` dropped from every record. The 16-byte salt and the sorted `_sd` are marked "Roym rule".

- [x] **P13 — minor.** *Where:* AG-19 (change.md:118); encoding step 2
  (change.md:502).
  *Problem:* AG-19 says "our digest function gives the result of the worked
  example in the SD-JWT specification". The RFC's example disclosure is
  built from JSON with a space after each comma
  (`["_26bc4LT-ac6q2KI6cBW5es", "family_name", "Möbius"]`). Our encoder
  writes compact JSON. A test that builds the disclosure with our encoder
  from those three values gets a different digest, and fails.
  *Fix:* split AG-19: (1) the digest of the RFC's disclosure string
  `WyJfMjZiYzRMVC1hYzZxMktJNmNCVzVlcyIsICJmYW1pbHlfbmFtZSIsICJNw7ZiaXVzIl0`
  is `X9yH0Ajrdm1Oij4tWso9UzzKJvPoDxwmuEcO3XAdRC0` (RFC 9901, section
  4.2.3); (2) our verifier accepts that disclosure as written; (3) our own
  encoder's output for a fixed salt is pinned by our own vector.
  *Status:* Fixed (f638722b), with one change to the proposal. AG-19 is split into the RFC vector (digest checked locally: it matches), the decoder, our own vector, and the refusals. Part (2) needed a fix: the RFC's example salt is 23 characters, not 16 bytes, so our decoder only requires the salt to be a string. The 16-byte rule moved to the Roym record check (AG-21, "Verifying").

- [x] **P14 — minor.** *Where:* "Settlement records" (change.md:390-400);
  `agreement.present` (change.md:691).
  *Problem:* the settlement records are not exact enough to build. (a) The
  table lists hidden fields, but not where their `_sd` list sits (top level
  of the payload?), and not that every hidden field is always present with
  `null` (as change.md:274 says for commitments). (b) `role` is listed but
  not defined. Today it is `DealRole`. Per commitment, "payer" and "payee"
  are giver and receiver, and either party can be either. (c) Today's
  records carry `counterparty_did` and `quote_author_did`
  (`payment.rs:94`, `fulfilment.rs:36`). The new field list drops them, with
  no reason given. (d) Today an acknowledgement can be corrected with
  `supersedes` ([receipts.rs:452-500](../../../../crates/roym_transaction/src/app/sync/receipts.rs#L452)).
  The draft does not say if this stays, per commitment. (e) The "field
  paths" input of `agreement.present` has no syntax.
  *Fix:* give each settlement payload as a code block like the other
  records. Define `role` as `giver | receiver`. Say whether the DIDs and
  corrections stay. Give the path syntax, for example
  `terms.scope` and `terms.commitments[c2].declared_value_minor`.
  *Status:* Fixed (f638722b). Agreed. The three settlement payloads are now code blocks, with `_sd` at the top level and every hidden field always present. `role` is `giver | receiver`. The two DIDs stay. `supersedes` stays on `payment-acknowledgement`, per commitment. The field path syntax is given under "Showing a deal to a third party".

- [x] **P15 — minor.** *Where:* "Facts checked in the code" (change.md:147,
  :148); track window (change.md:457).
  Code: [agreement_ops.rs:373-377](../../../../crates/roym_transaction/src/app/agreement_ops.rs#L373),
  [ledger.rs:105-114](../../../../crates/roym_transaction/src/app/ledger.rs#L105),
  [booking.rs:189-191](../../../../crates/roym_core/src/booking.rs#L189).
  *Problem:* three facts are not quite right. (a) "The counterparty signs
  first" is true only for a quote with a slot. For a quote with no slot, the
  quote author may call `agreement.accept` first. The draft does not say if
  the new design keeps or forbids this, or what `agreement.accept` does on
  the writer's node. (b) The writer writes `decision`, `seat` and `step`
  together only for a slot quote. Without a slot it writes `decision` and
  `step`. (c) The track end is "`max(window.latest_secs, agreed_at_secs) +
  TRACK_WINDOW_SECS` (… today's value)". Today it is `window.latest_secs` (or
  the scheduling time, with no window) plus 30 days, with no `max`. Only the
  30 days is today's value.
  *Fix:* correct the three facts. Say that `agreement.accept` on the
  writer's node is refused (or is a manual countersign of a filed
  acceptance). Mark the `max` as a change.
  *Status:* Fixed (f638722b). Agreed. The three facts are corrected and two facts are added (decline cleared by a new version; the track window today). `agreement.accept` on the writer's node is refused with `writer-countersigns` (AG-41). The `max` is marked as a change, with the reason.

- [x] **P16 — minor.** *Where:* "Text fields are language-tagged"
  (change.md:188-198); AG-32.
  *Problem:* `LocalText` gives each free-text field its own translation
  list. The multilingual change gives each record one list, "with the same
  translatable fields as the record" (roym-multilingual change, line 242),
  and the Hub's `bestVersion` picks one version for the whole record (ML-12).
  With per-field lists, one quote can have an English translation of the
  scope and none of a description. Then one `viewed_lang` per acceptance
  half (ML-18) no longer says which text the person read.
  *Fix:* keep `LocalText`, but require that every `LocalText` in one quote
  has the same set of translation languages. Then `viewed_lang` stays one
  value, and `bestVersion` still works per record. Record this in the
  deviations section (P7).
  *Status:* Fixed (f638722b). Agreed. One set of translation languages per record (AG-39). Listed in the differences table.

- [x] **P17 — minor.** *Where:* `details` limit (change.md:265); "Where
  disclosures travel" (change.md:526).
  *Problem:* `details` is "inside the 64 KiB payload limit". But `details`
  is a hidden field, so the payload holds only its digest. Its size is
  bounded only by the card body (160 KiB, with base64url adding a third).
  With 16 commitments, each `description` of 2048 bytes plus 4 translations,
  the disclosures alone can pass 160 KiB. `quote-too-large` catches it, but
  the Hub gets no rule it can show before the person signs.
  *Fix:* give `details` its own byte limit (for example 8 KiB of encoded
  disclosure). Give one total limit for all disclosures of a record, and
  have the Hub show the size against it in the quote form.
  *Status:* Fixed (f638722b). Agreed. `details` disclosure at most 8 KiB, all disclosures of one record at most 96 KiB, both encoded. The Hub shows the size before signing (AG-40). C2's plan must check the 96 KiB against the real envelope size; the doc says so.

- [x] **P18 — minor.** *Where:* AG-3 (change.md:98).
  *Problem:* "build both quote forms from the same terms and compare the
  receipts byte for byte" cannot pass. The two quotes differ in `answers`,
  so their `record_id`s differ, and the receipt's `quote_record_id` and
  envelope `subject` differ. With random salts, the digests differ too.
  *Fix:* "with a fixed salt source, the two receipt payloads are equal
  except `quote_record_id`, and the two first snapshots are equal except
  `agreement`".
  *Status:* Fixed (f638722b). Agreed. AG-3's test uses the reviewer's wording.

- [x] **P19 — minor.** *Where:* AG-26 (change.md:128).
  *Problem:* "no other code path refuses" cannot be checked by a unit test.
  The row also misses a rule that one node can test: the agreement writer
  marks an `eep-transfer` commitment `refused` only when it holds the
  issuer's signed refusal with one of the three reasons.
  *Fix:* split AG-26: (a) reference code only: the refusal reason type has
  exactly three values, and the reissue handler returns either a reissued
  EEP, a signed refusal with one of them, or an internal error (a table test
  over the handler's inputs); (b) service, on the writer's node: an unsigned
  refusal, or one with an unknown reason, leaves the commitment `endorsed`.
  *Status:* Fixed (f638722b). Agreed. AG-26 is split into (a) reference code and (b) a writer-side service test.

- [x] **P20 — minor.** *Where:* AG-6 (change.md:102); commitment table
  (change.md:446-456).
  *Problem:* AG-6 asks for a table test "over every (state, event) pair",
  but the draft never lists the events (giver's half, receiver's half,
  payment request, tick, cancel, commitment failure, and the EEP ones).
  `failed` is listed for `currency`, but its only reason, `reservation-lost`,
  cannot happen to a currency commitment.
  *Fix:* add an event list and one transition table per kind, with the
  refused pairs. Remove `failed` from `currency` until it has a reason.
  *Status:* Fixed (f638722b). Agreed. An event list and one transition table for each M08C kind (`fulfilment`, `currency`), with refused pairs. `failed` removed from `currency`. The obligations document adds the EEP tables (AG-6).

- [x] **P21 — minor.** *Where:* AG-10 (change.md:106); `declined` row
  (change.md:433); `requested` (change.md:428).
  *Problem:* AG-10 says "the writer moves the deal to `declined`". The state
  table says each node writes it, from the signed decline. Also: the
  counterparty cannot decline (withdraw) its own enquiry, and `requested`
  has no end, so an unanswered enquiry stays open for ever.
  *Fix:* use the table's wording in AG-10. Add "the counterparty withdraws
  its enquiry" (a `deal-decline` of its own enquiry), or say it is not
  allowed.
  *Status:* Fixed (f638722b), with one pushback. AG-10 now uses the table's wording, and the counterparty can withdraw its own enquiry with a `deal-decline`. Pushback: no automatic end for an unanswered enquiry. It is like an unanswered message, it holds no reservation, and the person can withdraw it. An expiry rule would add a clock with nothing to release.

- [x] **P22 — minor.** *Where:* change.md:726; AG-21 (change.md:120).
  *Problem:* (a) "Their places in the record exist now, so the canonical
  bytes do not change again later" is not true for `listing_id`, which
  "becomes the intent id when M08D renames listings". (b) AG-21 "a salt used
  twice": inside one quote, or also across versions? Across versions needs
  the salts of earlier versions to be kept and compared.
  *Fix:* (a) name the field `intent_id` now, or drop the claim. (b) say
  "inside one record"; across versions is the author's duty and is not
  checked.
  *Status:* Fixed (f638722b), with one pushback. (a) Dropped the claim rather than naming the field `intent_id` now: M08C still has listings, and the product is not released, so M08D renames it in place. (b) Agreed: "inside one record"; reuse across versions is not checked (AG-21).

- [x] **P23 — minor.** *Where:* "How the counterparty's node files the
  writer's records" (change.md:466-470); "Who appends" (change.md:612).
  *Problem:* (a) after a writer restores an old backup, its next snapshot
  can reuse a `seq` the counterparty already holds. The counterparty keeps
  the first, shows `progress-fork`, and ignores the writer until its `seq`
  passes the old one. The draft does not say how the writer finds its last
  `seq` again. (b) The counterparty appends log events "when it files the
  snapshot that shows the change". When a snapshot is lost, the next one
  can show several changes.
  *Fix:* (a) the writer reads its own last snapshot card from the
  conversation during sync and continues from that `seq`. (b) "the
  counterparty derives events by comparing the new snapshot with the one it
  held".
  *Status:* Fixed (f638722b). Agreed with both. The writer continues from the highest `seq` it finds in the conversation or holds (AG-42). The counterparty derives events by comparing the new snapshot with the one it held.

- [x] **P24 — minor.** *Where:* M08C
  [task.md](../../milestones/M08C-deal-core-and-extensions/task.md) "Demo"
  and slice C6; change.md:21.
  *Problem:* `task.md` names the demo "request, quote and agreement", and
  slice C6 "Barter". The draft renames `request` to `enquiry` (Q8) and does
  not use "barter". This document cannot edit `task.md`.
  *Fix:* list both words in the deviations section (P7), so the M08C plan
  uses "enquiry" and "mixed exchanges".
  *Status:* Fixed (f638722b). Agreed. Both words are in the differences table, with `task.md` as the document to update.

## Fix session round 1 — Claude Opus 5.5 (Claude Code), 2026-11-10

Fixes are in `f638722b` on `claude/m08c-agreements-change-doc-1708e9`.
All 24 findings are fixed. Three take a different route from the
proposed fix, with the reason in the item's status: P13 (the RFC's
example salt is not 16 bytes), P21 (no automatic end for an enquiry)
and P22a (no early rename to `intent_id`). P9 is now open question 12
for the owner.

Not done here: the reviewer suggests a backlog row now for path 2 of
the `ledger.rs:52` bug (accepting an old version), because it is a bug
in today's code. That is the owner's call. This change only plans the
fix for M08C.

New AG rows: AG-34 to AG-43. Ids were appended, not renumbered, so
this file's references stay valid.

## Doc review round 2 — Claude Opus 5.5 (Claude Code), 2026-11-10

Checked `change.md` at `d29b7dfc` on
`claude/m08c-agreements-change-doc-1708e9` (fixes in `f638722b`, slice
cut in `d29b7dfc`). This round checks the fixes of round 1 only.

**Summary:** P1 to P24 confirmed fixed. 0 blocker, 0 major, 3 new minor
(P25 to P27), all small gaps that the new text added.

What was checked and found correct:

- **Each fix is in the text.** P1: "The writer's decision" (change.md:580-591)
  and the Facts row (:158). P2: "Reason codes" (:487-504), AG-12, AG-43,
  and the version check (:568). P3: "Deal rules" (:506-511), AG-35 and the
  new "Deal" term (:24). P4: `claimed` (:383), AG-8, AG-36 and the rule at
  :559. P5: "Complete by derivation" (:709) and the two times (:741). P6:
  "Who reserves" (:597-606). P7: the differences table (:884-900). P8:
  `Presentation.records` (:668) and AG-34. P9: open question 12 and Tasks
  (:981-987, :1011-1013). P10: AG-37 and "Ids" (:214). P11: "Binding to the
  writer" (:367), AG-38 and `CancelReason` (:390). P12: RFC 9901 (:620), no
  `_sd_alg` (:628). P13: AG-19. P14: the settlement code blocks
  (:410-444). P15: Facts (:157, :163-165) and AG-41. P16: AG-39 and :213.
  P17: AG-40 and :649. P18: AG-3. P19: AG-26. P20: the two transition
  tables (:539-554) and :525. P21: AG-10 and :358. P22: :857 and AG-21. P23:
  AG-42, :576 and :742. P24: the differences table (:899).
- **The removed text is gone.** No `unverified`, `missing_half`, free-text
  `cancel_reason`, "or it made a bid", "keep a commitment's id" or "same
  data-layer call" is left. `_sd_alg` appears only where the doc says no
  record carries it.
- **The RFC vector in AG-19 is right.** SHA-256 over the US-ASCII bytes of
  the RFC's disclosure string, base64url with no padding, gives
  `X9yH0Ajrdm1Oij4tWso9UzzKJvPoDxwmuEcO3XAdRC0` (recomputed here with
  `openssl`).
- **The three different routes are accepted.** P13: the RFC's example salt
  is 23 characters, so the 16-byte rule cannot be a decoder rule; moving it
  to the record check is correct. P21: an unanswered enquiry holds no
  reservation, and the counterparty can now withdraw it, so no clock is
  needed. P22a: dropping the claim is enough, because the product is not
  released.
- **The fence is now buildable.** Every step of "The writer's decision" is
  idempotent or guarded by a unique insert (`decision:<quote_id>`,
  `countersign:<agreement>`), so a crash or two syncs at the same time end
  in one result. The only gap is P26.

### Findings

- [x] **P25 — minor.** *Where:* "The writer's decision" step 1 and step 2
  (change.md:584-585); "Deal rules" (:509); the `not-formed` row (:479);
  "Reason codes" (:493-494).
  *Problem:* a `not-formed` snapshot with `quote-superseded` or
  `quote-already-agreed` refuses one acceptance, not the deal. But the
  text treats every `not-formed` as an end of the deal, and "every end of a
  deal ends its quote id". Two cases go wrong. (a) A late card carries an
  acceptance of an old version. Step 1 refuses it before the fence, so no
  decision row is written, and the writer can still form an agreement on the
  latest version. The counterparty's node files the `not-formed` snapshot
  and shows the whole deal as ended. (b) For `quote-already-agreed` the deal
  is `agreed`. A `not-formed` snapshot for the other version must not move
  it.
  *Fix:* say that the snapshot's `agreement` is the refused version's
  `record_id`, and that `quote-superseded` and `quote-already-agreed` end
  only that acceptance. The deal keeps its state, and the Hub shows the
  refusal on that version's card. Only `quote-expired` and an extension's
  reserve reason end the deal (and the quote id). Add the refused-version
  case to AG-12's two-node test: after the refusal, the latest version can
  still be accepted.
  *Status:* Fixed (56dc3a54). Agreed. New "A refused acceptance is not an end" rule in "Deal rules": `quote-superseded` and `quote-already-agreed` name the refused version's `record_id`, end only that acceptance, and the counterparty releases its reservations for it. The deal stays `quoted` or `agreed`. Only `quote-expired` and an extension's reserve or confirm reason make a deal `not-formed`. Steps 1 and 2, the `not-formed` state row and both reason-code rows say so. AG-12's two-node test now accepts the latest version after the refusal.

- [x] **P26 — minor.** *Where:* "Recovery" (change.md:591); step 4
  (:588); "Commitment failure" (:394); AG-43.
  *Problem:* recovery runs steps 3 and 4 again, but not step 1. So a writer
  that crashed in `deciding` and syncs again after the quote's expiry
  countersigns after the expiry. This breaks AG-43 and :483 ("the writer
  never countersigns after the expiry by its own clock"). Also, after a long
  crash the writer's own reservation may have expired and been given away
  (EX-20, EX-22). Then its `confirm` is refused, and step 4 does not say
  what happens. :394 says the writer "confirms its own reservations at once,
  so it never needs" a `commitment-failure`, which is not true after a
  crash.
  *Fix:* in "Recovery", when no `countersign:<agreement>` row exists yet:
  re-check the expiry first (past it → release, `not-formed`,
  `quote-expired`). In step 4, a refused `confirm` → release the others,
  `not-formed` with the extension's reason. Because `confirm` comes before
  the countersign, no agreement has formed yet, so no `commitment-failure`
  is needed. Reword :394 to "confirms its own reservations before it
  countersigns". Add both cases to AG-14's crash test.
  *Status:* Fixed (56dc3a54). Agreed. "Recovery" re-checks the expiry first when no `countersign:<agreement>` row exists (past it: release, `not-formed`, `quote-expired`). Step 4 handles a refused `confirm`: release the others, `not-formed` with the extension's reason, no `commitment-failure` because nothing has formed. "Commitment failure" reworded to "before it countersigns". AG-14 names both cases and adds them to its crash test.

- [x] **P27 — minor.** *Where:* "Who signs what", the `deal-decline` row
  (change.md:453).
  *Problem:* the counterparty column says "declines a quote". Since the P21
  fix, the counterparty can also withdraw its own enquiry (:358, AG-10).
  *Fix:* "declines a quote, or withdraws its own enquiry".
  *Status:* Fixed (56dc3a54). Agreed. The row now says "declines a quote, or withdraws its own enquiry".

## Fix session round 2 — Claude Opus 5.5 (Claude Code), 2026-11-10

P25 to P27 are fixed in `56dc3a54` on
`claude/m08c-agreements-change-doc-1708e9`. All three proposed fixes are
taken as written, with no pushback.

## Doc review round 3 — Claude Opus 5.5 (Claude Code), 2026-11-10

Checked the `56dc3a54` diff of `change.md` against P25 to P27.

**Summary:** P25 to P27 confirmed fixed. No new findings. Every finding
in this file (P1 to P27) is closed, and this review is complete.

- **P25.** A refusal with `quote-superseded` or `quote-already-agreed` now
  ends only that acceptance: step 1 writes no fence row, step 2 leaves the
  row as it is, the snapshot names the refused version, and the deal stays
  `quoted` or `agreed` ("Deal rules", the `not-formed` row, "Reason
  codes"). Only `quote-expired` and an extension's reason make a deal
  `not-formed`. AG-12's two-node test now accepts the latest version after
  the refusal.
- **P26.** "Recovery" re-checks the expiry when no countersigned half is
  stored, and step 4 ends `not-formed` on a refused `confirm`. A recovery
  with a stored countersigned half still finishes as `agreed`, which is
  right, because that half was signed before the expiry. AG-14 names both
  cases and tests them.
- **P27.** The `deal-decline` row names the withdrawal.

## Doc review round 4 — Claude Sonnet 5.5 (Claude Code), 2026-11-15

Checked `change.md` on commit `d2e08f1a` (branch `docs/m08c-start`) against the code on this checkout: the design text, AG-1 to AG-43, the facts table, the writer's decision, the disclosure encoding and the event log. I recomputed the RFC 9901 digest in AG-19 with `shasum` and it matches. P1 to P27 are closed and are not repeated. **Summary:** 7 findings: 0 blocker, 4 major, 3 minor.

### Findings

- [x] **P28 — major.** *Where:* "The writer's decision", step 4 and "Recovery" (change.md:589 and :592).
  *Problem:* Step 4 puts the decision row to `agreed` (or `not-formed`) and only then stores and sends the countersigned half and snapshot `seq` 1. Recovery finishes only rows that are still `deciding`. A crash after the `put` and before the snapshot is stored leaves a row in a final state with no snapshot and no half sent. Nothing repairs it, and the counterparty waits until the quote expires. AG-14 says a crash after any step gives the same end state as no crash, so this case breaks it.
  *Fix:* In step 4, make the `put` of the decision row the last write. Order: (a) store the countersigned half (`countersign:<agreement>`, unique insert); (b) store snapshot `seq` 1 under a unique key (`snapshot:<agreement>:1`) in the `agreement_progress` collection; (c) put the row to its final state; (d) send both cards. In "Recovery", also say that on every sync the writer re-sends (through `conversation.send`, which is idempotent per record) the half and snapshot of every row in a final state whose cards are not yet confirmed sent. Add a crash point "after the put, before the send" to AG-14's test.
  *Status:* Fixed (66f50eca). Agreed, with a simpler send rule. "The writer's decision" now has two rules for every step: each signed record is stored with a unique insert before it is sent (`countersign:<agreement>`, `snapshot:<agreement>:<seq>`), and the row that ends a step is written last, after the send. A crash before the last write makes the next sync repeat the step and send the same stored records again; the counterparty files them as repeats (same `seq`, same bytes, not a fork). So no "sent" flag is needed. AG-14 adds the crash point between the send and the last `put`.

- [x] **P29 — major.** *Where:* "The log entry" (change.md:723, :732, :742) and "The statement" (change.md:776, :777); AG-27, AG-29.
  *Problem:* The `event` list has no event for a commitment that ends `ended-unconfirmed` or `cancelled`. `commitment-failed` covers only `failed`. Yet `final_state` gives `ended-unconfirmed` as an example, and `occurred_at_secs` for `commitment-settled` is defined as the receiver's half, which does not exist in that case. AG-27 says "every agreement event" is logged, so a developer cannot tell what to write. The statement rule "failed and cancelled are not rows" also does not say what happens to `ended-unconfirmed`: it is not settled, so it is not a row, and it is not in `open_at_end` because it is final. It disappears from the statement.
  *Fix:* Add two events to the list: `commitment-ended-unconfirmed` and `commitment-cancelled`. Their `occurred_at_secs` is the `issued_at_secs` of the snapshot that shows the state. Remove `ended-unconfirmed` from the `final_state` example of settled commitments. In "The statement", say that `ended-unconfirmed` and `cancelled` commitments are not rows and not in `open_at_end`, and say so in open question 10 and AG-29 (or add a third list, `unconfirmed`, with `claimed`). Add a test line to AG-27 for each event.
  *Status:* Fixed (66f50eca). Agreed. New events `commitment-ended-unconfirmed` and `commitment-cancelled`; their time is the snapshot's, as the "Two times" rule already says for other events. `final_state` is described as the state the event ends in. "The statement" says `ended-unconfirmed` is not a row and not in `open_at_end`, and the log still holds all three ends. No third statement list: open question 10 counts only settled commitments as rows. AG-27 names the four commitment events and a test per kind.

- [x] **P30 — major (disclosure leak).** *Where:* "Progress snapshot" (change.md:365 and :385), "Track window" (change.md:561); AG-8, AG-17.
  *Problem:* The snapshot "holds no hidden value" and "can be shown to anyone". But `track_ends_at_secs` is `max(window.latest_secs, agreed_at_secs) + 30 days`, and `window` is a hidden field. When the window ends after the agreement time, anyone who sees the snapshot learns the hidden `window.latest_secs` exactly. AG-17 tests only the quote, the receipt and the settlement records, so no test would catch it.
  *Fix:* Remove `track_ends_at_secs` from the signed snapshot. Each node computes the date from the commitment's `window` disclosure (it holds all of them) and from `agreed_at_secs`, and the Hub shows it. Change "so the other party sees the date" to say this. Add the snapshot to AG-17's list of payloads that are checked for hidden values. State in "What is hidden" that the time of a `tick` transition (the snapshot's own `issued_at_secs`) still shows roughly the window end for an `ended-unconfirmed` commitment, and that this is accepted.
  *Status:* Fixed (66f50eca). Agreed. `track_ends_at_secs` is removed from the snapshot. Each party computes the date from its `window` disclosure and `agreed_at_secs`, now defined as the `issued_at_secs` of the countersigned half. AG-17 covers the snapshot and values derived from a hidden field. "What is hidden" names the accepted rough leak through the `issued_at_secs` of an `ended-unconfirmed` snapshot.

- [x] **P31 — major.** *Where:* "The writer's decision" step 4 (change.md:589), "Reserve, confirm and release" item 4 (change.md:607), "When one commitment fails" (change.md:560); AG-14, AG-36.
  *Problem:* Two gaps in release. (1) Step 4 confirms each reservation, and when one confirm is refused it must "release the others". The others are already confirmed, so they are firm. The extensions change defines release only for reservations that are not confirmed (EX-21 covers "unknown or already released"). It does not say a confirmed reservation can be released, so the writer's slot may stay taken by a deal that never formed. (2) Item 4 lists the ends after which a node releases (`expired`, `not-formed`, `declined`, `cancelled`). It does not list a commitment that becomes `failed` or `cancelled` by `other-commitment-failed` while the agreement stays `agreed`. The writer's own confirmed slot for such a commitment is never freed.
  *Fix:* (1) Add to step 4: "`release` of a confirmed reservation frees the resource, as long as no step or settlement half exists for that commitment". Record this as a required behaviour in the extensions change's EX-21 at close-out (add a row to "Differences") and add it to AG-14's stub test. (2) Add to item 4: the giver's node also releases the reservation of a commitment that ends `failed` or `cancelled`, including by `other-commitment-failed`. Add this case to AG-36.
  *Status:* Fixed (66f50eca). Agreed. (1) Step 4 releases every other reservation after a refused confirm, confirmed ones too, and a new paragraph "Release after confirm" makes this a rule for every reserve hook. A new row in "Differences" names EX-21 for the extensions close-out. (2) Release item 4 adds a commitment that ends `failed` or `cancelled` while the agreement goes on. AG-14 and AG-36 test both.

- [x] **P32 — minor.** *Where:* "Commitments" tables and events (change.md:534, :545, :555) and the agreement table (change.md:479, :480); AG-9.
  *Problem:* Three small conflicts. (a) `cancel` in a final state is `refused` in the `fulfilment` table and `no change` in the `currency` table, while the text says cancel "is allowed only when every commitment allows it". "No change" is not "allows". (b) AG-9 says cancel is allowed "while no half of any commitment exists", the method table says "while every commitment is `promised` or `payment-requested`". A final commitment (for example `ended-unconfirmed` next to a `promised` one with a later window) is not covered by either. (c) After `agreement.cancel` every commitment is `cancelled`, so every commitment is final. Then the rule for `closed` ("the last commitment becomes final") also applies, and the deal could be `closed` or `cancelled`.
  *Fix:* (a) Make `cancel` on any final state `refused` in both tables. (b) Define one rule in "Events": `agreement.cancel` is allowed only when every commitment is `promised` or `payment-requested`; use the same wording in AG-9 and the method table. (c) Add to the `closed` row: "when the agreement is `agreed` and every commitment is final. `agreement.cancel` ends the agreement as `cancelled`, not `closed`".
  *Status:* Fixed in part (66f50eca). (a) Rejected: both tables already say `refused` for `cancel` in a final state (the `fulfilment` row "any final" has `refused` in the `cancel` column, and so does the `currency` row). (b) Agreed: AG-9, the `cancel` event and the method table now all say "every commitment is `promised` or `payment-requested`", and `commitment-moved` is added to "Reason codes". (c) Agreed: the `closed` row excludes `agreement.cancel`, and the `cancelled` row says the agreement ends `cancelled`.

- [x] **P33 — minor.** *Where:* "The writer's decision" step 1 (change.md:585), "Deal rules" (change.md:510); AG-12, AG-13, AG-43.
  *Problem:* A refusal in step 1 (`quote-superseded`, `quote-expired`) writes no row, and the half stays filed. The doc does not say how the writer remembers that it has already refused this half. Each later `transaction.sync` runs step 1 again and sends a new snapshot card for the same refusal. AG-13 (idempotency) lists only methods, not this path. The writer's `not-formed` state for `quote-expired` also has no stored row, and `quote-ended` in `quote.set` has to find it from the snapshot.
  *Fix:* In step 1, store a row `refused:<half record_id>` (unique insert, in `ledger`) with the code, and the snapshot under `snapshot:<agreement>:1`, before sending. A repeat sync that finds the row sends nothing new. Say that `quote.set` reads `quote-ended` from the decision row or this row. Add "two syncs in a row send one refusal" to AG-12 and AG-43.
  *Status:* Fixed (66f50eca). Agreed. A refusal in step 1 (and `quote-already-agreed` in step 2) stores its snapshot, sends it, and last creates `refused:<half record_id>`; a later sync skips the half. A new paragraph "What `quote.set` reads" lists where `quote-ended` comes from. AG-12 and AG-43 test two syncs in a row.

- [x] **P34 — minor.** *Where:* "Facts checked in the code" (change.md:152 to :170).
  *Problem:* Several line references are stale on this checkout. Examples: `agreement_ops.rs:99` is now `AgreementListInput` (`accept` is at :113, `prepare_author_booking` at :415, `maybe_countersign` at :451), and `:168`, `:373` and `:402` no longer hold the cited code; `app.rs:278` is now :279 for `require_internal`; `booking.rs:189` is the function, the `saturating_add` is :190; `messages.ts:598` is now unrelated code (the Accept and Decline buttons are near :667); `envelope.rs:213` is a doc comment, not the canonical function. The "checked 2026-11-10 on `main` at `4830bdf7`" header is also old.
  *Fix:* Re-check every row against the checkout at the time of the fix. Prefer function names (`accept`, `maybe_countersign`, `decide_booking`, `decision_id`) next to or instead of line numbers. Update the date and commit in the heading.
  *Status:* Fixed (66f50eca). Agreed. The facts table was checked again at `502aac45`. Each row names the function, with the current line. The two stale references in the text (`receipts.rs:89`, `agreement_ops.rs:373`) and the one in "Track window" (`booking.rs:189`) now name functions.

## Fix session round 4 — Claude Opus 5.5 (Claude Code), 2026-11-15

P28 to P34 are handled in `66f50eca` on `docs/m08c-start`. Six are fixed as proposed or with a simpler form (P28 uses "write the last row after the send" in place of a sent flag). P32 (a) is rejected: the two tables already agree. A later reviewer checks these fixes.

## Doc review round 5 (fixes check) — Claude Sonnet 5.5 (Claude Code), 2026-11-15

Checked `66f50eca` (HEAD `6ff3909d`) against P28 to P34. All seven fixes solve the problem raised. The facts table rows match the code (every named function and line number checked). The fixes create 4 new findings: 1 major, 3 minor.

- **P28.** Confirmed fixed. "Store each signed record once, write the row that ends the step last" removes the final-state-without-send case, and the repeat rule (same `seq`, same bytes) needs no sent flag. Both stores are single unique inserts, one in `ledger` and one in `agreement_progress`, so the host can build them. See P35 for a gap in the rerun of the `not-formed` branch, and P36 for the restore case.
- **P29.** Confirmed fixed. The four commitment events cover every final state. `occurred_at_secs` is the snapshot's, and the statement says `ended-unconfirmed` is neither a row nor in `open_at_end`. AG-27 has a test per event. See P38 for open question 10.
- **P30.** Confirmed fixed. `track_ends_at_secs` is gone, `agreed_at_secs` is the countersigned half's `issued_at_secs` (the same on both nodes), and AG-17 now covers the snapshot and derived values. The rough leak through the `issued_at_secs` of an `ended-unconfirmed` snapshot is written down in "What is hidden".
- **P31.** Confirmed fixed. Step 4 releases confirmed reservations after a refused confirm. Release item 4 and AG-36 cover `failed` and `cancelled` commitments while the agreement goes on. The EX-21 row is in "Differences".
- **P32.** Confirmed, and the rejection of (a) is accepted. `fulfilment` columns are `giver-half, receiver-half, tick, cancel, failure, other-failed`: the `any final` row has `refused` in `cancel`. `currency` columns are `payment-request, giver-half, receiver-half, tick, cancel, other-failed`: the `any final` row has `refused` in `cancel` (its `no change` cells are `payment-request`, `tick` and the halves). My finding misread the column order. (b) and (c) are fixed: AG-9, the `cancel` event, the Methods table and `commitment-moved` agree, and `closed` excludes `agreement.cancel`.
- **P33.** Confirmed fixed. A refusal stores its snapshot, sends it, and creates `refused:<half record_id>` last. A repeat sync skips the half. "What `quote.set` reads" lists the sources. AG-12 and AG-43 test two syncs. See P37 for the lookup by quote.
- **P34.** Confirmed fixed. Facts rows now name functions with current lines. The sample checked: `accept` :113, `prepare_author_booking` :415, `maybe_countersign` :451, `claim_decision` :70, `claim_seat` :134, `track_window_end` :189, `verify_booking_progress` :330, `file_progress_card` :43, `create` :200 and `batch-mutate` :184 in the data-layer WIT, and `app.rs:279`. All are correct.

### Findings

- [x] **P35 — major.** *Where:* "The writer's decision", step 4, first branch (the refused `confirm`) and "Recovery" (change.md:601 and :606); AG-14.
  *Problem:* The refused-confirm branch releases every other reservation, stores the `not-formed` snapshot `snapshot:<agreement>:1`, sends it, and only then puts the decision row. Suppose a crash comes after the release or after the store and before the put. The row is still `deciding`, so recovery runs steps 3 and 4 again. Step 3 `reserve` runs on reservations that were just released. The extensions change does not say what `reserve` does for a key that was released. The rerun may also find that the refused `confirm` now succeeds, for example because the other holder's slot was freed. Then step 4 takes the `agreed` branch, stores a countersigned half, and "uses the stored record" for snapshot 1, which is `not-formed`. The writer would then send a countersigned half and a `not-formed` snapshot for the same agreement. AG-14 says a crash at any step ends in the same state as no crash. The tests it names do not cover this.
  *Fix:* In "Recovery", say that before step 3 the writer looks for `snapshot:<agreement>:1`. If it exists in state `not-formed`, the writer does not reserve or confirm again. It releases every reservation of the agreement (idempotent), sends the stored snapshot, and puts the row to `not-formed` with the stored reason. Add this crash point (after the release, before the last `put`, with the slot free again on the rerun) to AG-14's test.
  *Status:* Fixed (fdf5e729). Agreed, with one more change. Each `not-formed` branch of step 4 now stores the snapshot first: "This stored snapshot is the decision." Only then does it release and send. "Recovery" is now three ordered checks; the first finds a stored `snapshot:<agreement>:1` in state `not-formed` and finishes as `not-formed` with no new reserve or confirm. Storing first also closes the case of a crash after the release and before the store, which the proposed fix left open (a rerun reserve on a released key). AG-14 adds the crash point with the slot free on the rerun.
  **Reopened in round 6:** the repeat path of step 2 still skips Recovery's checks. *Status:* Fixed. Step 2's repeat of a `deciding` row runs Recovery's three checks, and AG-14 includes the case where the same half is delivered again after the `not-formed` snapshot is stored.

- [x] **P36 — minor.** *Where:* "The writer's decision", rule "A signed record is made once" and "The writer's `seq` after a restore" (change.md:589 and :579); AG-42.
  *Problem:* "Made once" relies on the rows `countersign:<agreement>` and `snapshot:<agreement>:<seq>` in the writer's own storage. After a restore from an old backup taken before the writer stored them, the rows are missing, but the cards were already sent. A decision row restored as `deciding` makes the writer sign a second countersigned half with a new `issued_at_secs`. The counterparty would then hold two different halves from the writer for one agreement. The restore rule and AG-42 cover only `seq`.
  *Fix:* Extend the restore rule: before it signs a countersigned half for a `deciding` row, the writer looks in the conversation for its own countersigned half over that quote version, and stores and reuses it. Add this to AG-42.
  *Status:* Fixed (fdf5e729). Agreed. New bullet "The writer's records after a restore" in "Single writer": with no stored row, the writer looks in the conversation for its own countersigned half over the same version, or its snapshot with the same `agreement` and `seq`, then stores and reuses it. This covers snapshots too, not only the half. The "made once" rule points to it. AG-42 adds the case.

- [x] **P37 — minor.** *Where:* "The writer's decision" step 1 and "What `quote.set` reads" (change.md:595 and :608).
  *Problem:* The row `refused:<half record_id>` holds `{ code, at_secs }`. `quote.set` must find a `quote-expired` refusal by quote id, but the row key is the half's id and the row has no quote id. The author cannot look it up without loading every `refused:` row and every half.
  *Fix:* Add `quote_id` to the row (`{ quote_id, code, at_secs }`), or key the `quote-expired` case another way that `quote.set` can read directly.
  *Status:* Fixed (fdf5e729). Agreed. The row is `{ quote_id, code, at_secs }`, and "What `quote.set` reads" looks for a `refused:` row with this `quote_id` and code `quote-expired`.

- [x] **P38 — minor.** *Where:* open question 10 (change.md:992 to :995) against "The statement" (change.md:796).
  *Problem:* The owner's decision on question 10 names "failed and cancelled" as not rows. The fix adds `ended-unconfirmed` to the list of commitments that are not rows and not in `open_at_end`. That extends a confirmed decision, and the question text does not say it. A reader of the question alone would still think an `ended-unconfirmed` commitment may be a row.
  *Fix:* Add one line to question 10: `ended-unconfirmed` commitments are neither rows nor `open_at_end`, and the log holds them. Mark it for the owner to confirm, as the question's decision line is dated 2026-11-10.
  *Status:* Fixed (fdf5e729). Agreed. Question 10 has a new line for `ended-unconfirmed`. The owner confirmed it on 2026-11-15.

## Fix session round 5 — Claude Opus 5.5 (Claude Code), 2026-11-15

P35 to P38 are fixed in `fdf5e729` on `docs/m08c-start`. P35 goes one step further than proposed: the `not-formed` snapshot is stored before any release, so a stored snapshot is the decision. The owner confirmed P38's added line on 2026-11-15. A later reviewer checks these fixes.

## Doc review round 6 (fixes check) — Claude Opus 5.5 (Claude Code), 2026-11-16

Done as part of the M08C C1 plan review ([slice-c1-review.md](../../milestones/M08C-deal-core-and-extensions/slice-c1-review.md), round 1). Checked `change.md` on `feat/m08c-c1-commitment-model` (`41b8dfa1`) against P35 to P38, and against the code on `main`. **Note:** this session ran on Claude Opus 5.5, the same model that made the round 5 fixes. So these fixes are still not confirmed by a different tool.

- **P35.** Confirmed fixed. Step 2's sentence updated to run Recovery's three checks, and AG-14 includes the case where the same half is delivered again after the `not-formed` snapshot is stored.
- **P36.** Confirmed fixed. "The writer's records after a restore" covers the countersigned half and each snapshot, and AG-42 tests both. Its limit (a full restore also rolls back the conversation) is recorded in `## Deviations` (C1 plan, Q5).
- **P37.** Confirmed fixed. The `refused:` row holds `quote_id`, and "What `quote.set` reads" uses it. The ledger needs an index on `quote_id` for that lookup; that is a build detail for C1b (C1 plan review P12).
- **P38.** Confirmed fixed. Open question 10 says that `ended-unconfirmed` commitments are neither rows nor `open_at_end`, with the owner's confirmation of 2026-11-15.

## PR #372 review round 1 — Claude Sonnet 5.5 (Claude Code), 2026-11-20

Checked branch `docs/deal-flow-simplification` at `e71daba9` (`git diff main...`: 8 files, docs only) against the code on `main` at `535e3893`. The change was written by Claude Opus 5.5. The six owner decisions of 2026-11-20 are not reopened. The questions here are whether the docs state them correctly, completely and consistently, and whether the design works. The branch was up to date with its remote (`git pull`), and the tree was clean.

**Summary:** 16 findings, P39 to P54: 1 blocker, 7 major, 8 minor. All fixed in c2ba3de8 (see each status); P41 has one push-back.

What was checked and found correct:

- **Signing with no session.** In `signing.wit`, `principal::delegated` needs a session only to match the caller's master; an internal call needs no session. The writer's countersign does sign with no session today: `sign_author_receipt_half` (`decide.rs:695`) calls `signing::person_principal` (`signing.rs:192`), which uses the stored certificate. So the claim "both sign through `signing` with the person's delegation" is true. It holds only while the stored certificate is installed and not expired (see P47).
- **Settlement from `promised`.** In `deal/transition.rs`, `GiverHalf` (a claim) and `ReceiverHalf` (a settlement) both leave `promised` directly, for `fulfilment` (`step_fulfilment_promised`) and for `currency` (`step_currency_promised`). A `payment-request` is not needed. VX-55 and AG-46 are right on this. `tick` ends a commitment as `ended-unconfirmed`, with the claim flag kept, as VX-55 says.
- **Today's listing.** `BookingMode`, `PaymentModel` and `PaymentTerms` (`listing.rs:77`) are as the docs cite them. The enquiry still requires `description` (`EnquirySetInput`).
- **Hub descriptions.** `hit-engage` (`directory.ts:438` to `:455`) only prints the owner's conversation address, so the backlog row and the Engage fix are described correctly. The quote form (`messages.ts:759` to `:857`) has about 17 inputs, already has built-in cancellation, refund and dispute text, and an expiry default of 24 hours. It has no currency default, so AG-48's `INR` is a new fallback. The labels today are "Quote this request" and "Accept these terms", as task.md says.
- **Numbering.** AG-44 to AG-48 follow AG-43, and VX-54 to VX-56 follow VX-53, with no gap and no reuse. Every new row is named in a slice: AG-44, AG-45 and VX-54 in C3c; AG-46 to AG-48, VX-55 and VX-56 in C6; SH-16 in D5; AG-48 and the mode fields in D0 and D1. The section 5 table, the M08D task, status.md (rows C3c and Engage fix) and the backlog row target agree with each other.
- **Planning ids.** The change is docs only. No code comment or test name is touched, and no living doc is touched.
- **Language.** The new text follows "Writing for readers", with the small exceptions in P54.

### Findings

- [x] **P39 — blocker.** *Where:* agreements "Agree as listed" (the flow diagram and "Signing with no person present"); value-exchange "Agree as listed" rules, "Waiting is shown as waiting"; VX-54; AG-44; M08C task.md C3c.
  *Problem:* Both docs say the owner's node answers "at once, with no action by the owner", and that "instant means instant only while the owner's node is online". In the code, nothing runs `transaction.sync` by itself. The only non-test caller is the Hub, in `fillThread` (`messages.ts:424`), each time it draws that conversation's thread. The event feed (`syneroym:events`) is a list that a reader polls. It calls no service. The writer's decision, and the filing of an acceptance half, run only inside `transaction.sync` (AG-41). The same holds on the other side: the counterparty's automatic acceptance needs a sync after the quote card arrives. So the owner's node signs the quote only when the owner's Hub draws that thread, and the agreement forms only when the counterparty's Hub does the same. "Signing with no person present" is right that the writer signs with no session, but it signs only after a person's Hub starts the sync. The planned tests would not show this. Multi-node tests in this repo move a deal by calling `transaction.sync` by hand (`common/roym_flow.rs`, `roym_deal_e2e.rs`; see also the agreements doc's note on delivery synchronization in multi-node tests). A two-node test of AG-44 passes with no driver.
  *Failure case:* An owner sets a stall offering to `instant`. The node runs on a small server and nobody has the Hub open. A customer taps "Agree as listed". The enquiry waits in the conversation. Nothing answers until the owner opens the Hub the next morning. The customer's Hub says "waiting for the owner's node" for the whole night, while the owner's node is online. The promise "no action by the owner" is false, and so is the sentence about "online".
  *Fix:* (1) Say in both docs that C3c adds a driver: a node-side trigger that runs `transaction.sync` for a conversation when a deal card arrives in it, on both nodes. The C3c plan chooses the mechanism. My recommendation is a small bounded sweep over conversations that hold an `as_listed` enquiry or an `accepted` row. It needs no new service edge, and it also gives `tick` and Recovery (AG-14) a trigger when no Hub is open. A call from `conversation` to `transaction` would be a cycle, because `transaction` already depends on `conversation`. (2) Add a stop rule to C3c next to the signing one: if no driver can be built in the slice, the plan stops and asks the owner. The fallback is confirm-first only, with the Hub as the trigger. (3) Replace "only while the owner's node is online" with what is true. (4) The AG-44 two-node tests for `instant` must make no `transaction.sync` call on either node. Add the case "both Hubs closed". (5) Size: see P53.
  *Status:* Fixed (c2ba3de8). Both docs now say that each node drives its own steps. The agreements doc has a section "The driver" with your bounded-sweep idea and the cycle limit, and new row AG-50. C3c has the stop rule and the confirm-first fallback. The sentence about "online" is replaced. The AG-44 two-node test makes no `transaction.sync` call, and has a "both Hubs closed" case.

- [x] **P40 — major.** *Where:* value-exchange "Complete terms" rule; AG-44; AG-48; M08D task.md D0.
  *Problem:* "Complete terms" cannot be derived from what a listing holds. (a) `ListingPayload` has `booking` (mode, lead time, a cancellation window in seconds), `payment` (`PaymentTerms`), `location` and more, but no cancellation text, no refund text, no dispute path and no quote expiry. `DealTerms` requires all four (`cancellation_terms`, `refund_terms`, `dispute_path`, `quote_expires_at_secs`). (b) The two docs disagree on where those four come from. The value-exchange doc says complete terms include "the defaults for cancellation, refund, dispute path and expiry" as things the offering states. AG-48 says they come from "the author's saved deal terms", which are "local settings of the author's node, not a record". They cannot be both. The counterparty's node cannot compare a term it cannot see, so AG-44's check "every term equals the version" cannot cover those four. (c) `per-hour` and `per-unit` prices need a quantity or a duration, and the enquiry has no such field. (d) A slot offering needs a `slot_id`. A slot is its own row, and a quote names it in `details.data.slot_id` (the slot check in `quote_ops.rs`). `as_listed` has only a version id. So "instant for a slot" has no slot to reserve. (e) An `at-receiver` location needs the counterparty's address, and `address_disclosure: on-agreement` says the address is shown only when an agreement needs it. (f) `quote-only` has no amount. (g) The comment on `PaymentTerms.payee` says the UI must never present it as agreed terms, yet the automatic quote copies it as the binding payee.
  *Failure case:* The C3c implementer cannot tell whether a "per-hour, at-receiver" listing can be agreed as listed. They build the quote with the author's saved refund text. The counterparty's node compares price and scope and signs. The person has accepted a refund text that no signed record of the offering ever stated and that they never saw.
  *Fix:* (1) Define "complete terms" in the agreements doc as one pure function in `roym_core` that both nodes use: `terms_from_listing(version, enquiry) -> Result<DealTerms, NotComplete(reason)>`. Give its inputs and every refusal reason. For v1: a `fixed` price with `amount_minor`; a slot offering only when the enquiry names a `slot_id`; `order` mode as one-shot. It refuses `per-hour`, `per-unit`, `quote-only` and `at-receiver`. (2) Put the four defaults on the offering. The listing gets one optional block, for example `deal_terms`, with the three texts as `LocalText` and `quote_valid_secs`. A term the counterparty must check has to be in a signed record the counterparty saw. AG-48's saved terms then fill only the quote form, never the as-listed path. (3) Give `as_listed` an optional `slot_id`. (4) Make the value-exchange definition match. (5) AG-44's unit test has one row per refusal reason. (6) The refused cases are deferrals. Add backlog rows when this is fixed (per-hour, per-unit and at-receiver "Agree as listed", target M08D or `TBD`).
  *Status:* Fixed (c2ba3de8). There is one pure function, `terms_from_listing`, with its inputs, outputs and refusal reasons. The listing gets `deal_terms`, so every checked term comes from the signed offering. AG-48's saved terms only fill the forms. `as_listed.slot_id` is added. The payee note is in the function's description. The overview definition matches. There is a backlog row for the refused cases.

- [x] **P41 — major.** *Where:* AG-44 ("every term of the quote equals that version's terms"); value-exchange "Agree as listed", step 3.
  *Problem:* "Equal" is not defined. (a) The counterparty's node needs the content of the version. The enquiry holds only a `record_id`. The Hub got the envelope from a directory run (`runEnvelope` in `directory.ts`), and the owner may replace it later. Nothing says the counterparty's node keeps the verified envelope at the tap. (b) Quote fields are hidden behind `_sd` digests with a salt per quote, so digests cannot be compared. The node must compare plain values, after it verifies each disclosure against its digest. A missing or unverifiable disclosure must count as "not equal". The doc does not say so. (c) For `LocalText`, it is not said whether the check compares the original `lang` and `text` only, or also the translations. The signed original wins (multilingual rules), and a translation is the author's own text. (d) Some terms cannot equal anything in a listing: `quote_expires_at_secs` (an absolute time), `commitment_id`, and `window` for a slot. (e) `DealTerms` and `Commitment` have fields that a listing has no counterpart for (`extension`, `details`, `after`, `tax_minor`, `fees_minor`, `eep`), and they will get more. A check written field by field passes a new field with no error.
  *Failure case:* The author adds a second commitment, or a `details` block, or `tax_minor`. The check compares price and scope. The node signs.
  *Fix:* Define the check as build and compare. The counterparty's node builds the expected `DealTerms` with the same `terms_from_listing` (P40), from the stored version and its own enquiry. It compares the whole struct, by `PartialEq`, with the quote's disclosed terms. A new field then fails closed. The only exceptions are listed: `quote_expires_at_secs` (it must lie between now and a stated maximum), `commitment_id`, and `LocalText.translations`. A missing or unverified disclosure shows the quote as a normal quote. The Hub stores the verified version envelope in the enquiry row at the tap. AG-44's unit table has one row per field of `DealTerms` and `Commitment`, built from the struct, so a test fails when a field is added without a row.
  *Status:* Fixed (c2ba3de8), with one push-back. The check is now build and compare: `PartialEq` on the whole `DealTerms`, a struct-driven unit table, and missing disclosure means a normal quote. The stored version is kept at the tap. Push-back: `commitment_id` and `LocalText.translations` need no exception. Ids are assigned by position, and the function copies the listing's translations, so both compare equal by construction. Only `quote_expires_at_secs` is checked by range. With fewer exceptions, more fields are checked.

- [x] **P42 — major.** *Where:* agreements "Time limit for confirm first" and "Signing with no person present"; value-exchange "Waiting is shown as waiting"; AG-44; AG-10.
  *Problem:* The tap is the person's consent to a later signature. The docs leave one part of that to the C3c plan: whether the Hub shows a time after which the person may withdraw. They say nothing about how long the consent lasts. The enquiry has no expiry, and no rule says that the automatic acceptance checks for a withdrawal. Also, the owner's node can drop the enquiry. `on_direct_message` (`inbox.rs`) drops a first contact with `rate-limited`, and an owner with `max_per_window: 0` refuses every stranger. Then the Hub waits for ever. A signing rule must be in the doc. It is not acceptable to leave it to the plan.
  *Failure case:* A person taps "Agree as listed" on a Friday, on a confirm-first offering. The owner is away. Three weeks later the owner confirms, the quote arrives, and the node signs the acceptance. The person has bought elsewhere or has forgotten. In the other case, the person withdrew (AG-10) and the acceptance still signs, because no rule says it must check.
  *Fix:* The enquiry's `as_listed` carries a signed `valid_until_secs`. The Hub's default is 1 hour for `instant` and 48 hours for `confirm-first`. The owner's node refuses an enquiry whose time has passed. The automatic acceptance signs only when the quote's `issued_at_secs` is not later than `valid_until_secs`, and when the person's node holds no decline of that enquiry. It checks this immediately before it signs. After the limit, the Hub says "no answer, the request ended" and offers a new tap. When the conversation reports a dropped first contact (`rate-limited`, `blocked`), the Hub shows that, not "waiting". Add a test for each: expired consent, withdrawal before the quote, and a drop.
  *Status:* Fixed (c2ba3de8). `as_listed.valid_until_secs` is signed, with Hub defaults of 1 hour and 48 hours. The owner's node declines with `consent-expired`. The counterparty's three checks run immediately before signing. The Hub has an ended line and a refused-first-contact line. This is new row AG-49, with tests for each case.

- [x] **P43 — major.** *Where:* agreements "Agree as listed" (the enquiry names a version; "the mode is a field of today's listing"); AG-44, AG-45; the quote's expiry rule.
  *Problem:* The enquiry names a version. The owner's node answers later: at the next sync for `instant`, when the owner acts for `confirm-first`. Meanwhile the owner can publish a new version (a new price, or a new mode), withdraw the listing (`ListingStatus::Withdrawn`), or change `not_limited`. The docs do not say which version the node builds the quote from, whether it checks that the listing is active, or which version's mode applies. The automatic quote also needs an expiry. A quote's expiry is an absolute time, from 5 minutes to 90 days after signing. Two waits depend on it: the counterparty's acceptance, and the writer's countersign (AG-43 refuses after the expiry). With a short value the quote ends as `quote-expired` before a Hub opens (P39). With a long value the owner is bound for weeks.
  *Failure case:* The owner raised the price from 500 to 600. An enquiry for the old version arrives. If the node builds from the old version, the owner is bound to 500. If it builds from the new version, the equality check fails on purpose and the deal needs a person again. Neither is written down. A withdrawn listing gets a quote anyway.
  *Fix:* The owner's node answers only from the current active version. If the named version is not the current one, it answers with a quote from the current version. That quote is a normal quote, because the check fails, and the Hub shows what changed with the AG-47 summary. If the listing is withdrawn, the node answers with a `deal-decline` of the enquiry with a named code, for example `offering-withdrawn`. The mode is read from the current version when the node answers. The quote's expiry is `now + quote_valid_secs` from the offering (P40, default 24 hours as in the Hub's form, maximum 7 days for this path). The Hub's waiting line ends at that time with "the owner's answer ran out". Tests: version changed, listing withdrawn, expiry between the quote and the acceptance.
  *Status:* Fixed (c2ba3de8). The node answers from the current version only. A changed version gives a normal quote with the AG-47 summary. A withdrawn listing gives a decline with `offering-withdrawn`. The mode is read at answer time. The expiry is `now + quote_valid_secs`, at most 7 days. These are in AG-49 and its tests.

- [x] **P44 — major (abuse).** *Where:* AG-45; value-exchange "The owner chooses the mode"; VX "Known limit: fake identities".
  *Problem:* In `instant` mode any stranger who can reach the owner's conversation address can form agreements on the owner's node, with no action by the owner. The docs give no limit. The limits that exist today do not cover this. The first-contact limit (`ContactLimits`: 3 per sender per day by default) applies to a new conversation only. Once a conversation is `accepted`, nothing limits the enquiries inside it, except the cap of 2,000 cards (`MAX_CARDS_PER_CONVERSATION`). A new identity costs nothing, so a per-sender limit is not a limit. `open_to` (`anyone`, `members`, `referral`, `past-counterparties`) is only a search filter. No node enforces it, and the listing's own comment says it states the rule only. For a slot offering, each as-listed agreement reserves and confirms a slot (AG-14) until its commitment ends (the track window runs to 30 days after the slot).
  *Failure case:* (1) A hostile client sends 500 `as_listed` enquiries for the 500 slots of an `instant` listing. The node forms 500 agreements and holds every slot. Real customers get `slot-unavailable`. The owner must cancel them one by one (`agreement.cancel`). (2) An owner sets `open_to: members` and `instant`. A stranger agrees, and the node signs.
  *Fix:* Add to AG-45: (a) `instant` applies only when `open_to` is `anyone`; otherwise the offering is confirm-first, because no node can check membership or a referral yet. (b) A cap on open instant agreements for each listing (a default, for example 20, that the owner can change). When it is reached, the listing behaves as confirm-first until agreements close or are cancelled. (c) At most one open instant agreement for each counterparty and listing. (d) The Hub's owner view can cancel unconfirmed instant agreements in one action. Each is tested. Add one sentence to the VX "Known limit" about this.
  *Status:* Fixed (c2ba3de8). Instant only for `open_to: anyone`. `instant_cap` defaults to 20, with a range of 1 to 500, which I added. One open instant agreement per counterparty and offering. The owner can cancel unconfirmed instant agreements in one action. These are in AG-45, the overview rules and VX-54.

- [x] **P45 — major (reputation).** *Where:* value-exchange "Only recorded deals count" and "Known limit: fake identities"; VX-55; agreements "Closing in one action".
  *Problem:* The new text argues that cheap agreeing and cheap closing help reputation, because only recorded deals count. It does not say that the same cheapness helps a farm of fake identities. Before, a fake deal needed an enquiry, a quote, an acceptance and a settling half for each commitment, on two identities. Now, with an `instant` offering, one tap makes the agreement, and one tap on each side closes it, and a script can do all of it. VX-55 also says an agreement that nobody closed "still counts as a recorded deal". The "Known limit" paragraph and its defences were not updated, and none of them says what an instant deal is worth. A second harm is griefing. A stranger can make agreements on an owner's identity that the owner never saw. Each ends `ended-unconfirmed`, and unless a rule says otherwise, M08G may show these as unconfirmed deliveries of the owner.
  *Failure case:* A farm of 200 identities runs 200 instant deals against one identity A. A's evidence shows 200 closed deals, all with new counterparties. In the other case a competitor sends 200 `as_listed` enquiries to owner B, closes none, and B's evidence shows 200 unconfirmed commitments.
  *Fix:* (1) Add the limit to "Known limit: fake identities": agreeing and closing now cost one call, and the defences are the reader-side ones. (2) Write one rule for M08G, which does not conflict with VX-55: an unconfirmed commitment with no claim by the giver is not negative evidence about the giver. A deal counts for more when the receiver also closed it and when it has many distinct counterparties. (3) Put one question to the owner: should the agreement receipt or the progress snapshot show that the agreement formed in `instant` mode, so that a reader can discount it? The field would be visible and carry no hidden value. I recommend yes.
  *Status:* Fixed (c2ba3de8). Points (1) and (2) are added to the overview, after "Known limit: fake identities". Point (3) is now owner question 14 in the agreements doc, with your recommendation (yes, as a weak signal). It is not decided here.

- [x] **P46 — major (completeness).** *Where:* agreements "Enquiry" record table (`:260` to `:270`), the Methods table (`enquiry.set` row and the `transaction` rows), "Differences from the overview and sibling changes", "Rejected options"; the listing record (`catalog`).
  *Problem:* The new behaviour is in the prose "Agree as listed" and in AG-44 to AG-48 only. The tables that a developer reads as the contract did not change. (a) The Enquiry section says "three changes", lists two, and says "everything else unchanged". `as_listed` is not in it. It does not say that `description` becomes optional (today `EnquirySetInput.description` is required), and it has no place for the consent time of P42. (b) The Methods table has no method for the owner's one confirm in confirm-first mode, and none for "I gave everything" and "I received everything". Each `transaction` method needs an admission row (AG-31). (c) The listing's new `acceptance` and `not_limited` fields have no shape: no values, no default, no validation, no `listing.set` input. The directory hit holds a projection (`search_ops.rs`). The projection must say whether "Agree as listed" is possible, so the Hub can show or hide the action. This is not mentioned. (d) "Differences" has no row for the enquiry's optional `description`, and none for the rule that a node can accept for a person. (e) "Rejected options" still rejects "checking a quote against the offering's in-return terms", because another node cannot check it fully. AG-44 now makes the counterparty's node check a quote against the offering. The two are different checks (equality with a version, against what the owner accepts in return), but a reader cannot tell.
  *Fix:* Add `as_listed`, the optional `description` and the consent time to the Enquiry table. Name the confirm method and the closing method(s) in the Methods table, with their admission rows. Describe the two listing fields and the projection flag. Add the two rows to "Differences". Add one sentence to the rejected option that says how the equality check differs.
  *Status:* Fixed (c2ba3de8). The Enquiry table has `as_listed` and the optional `description`. The consent time is inside `AsListed`. There are new methods `quote.from-listing` and `agreement.close`; the table already says every method is local-only, and AG-31 covers admission. There is a listing field table and the projection's `can_agree_as_listed`. Two new rows in Differences. One sentence added to the rejected option.

- [x] **P47 — minor.** *Where:* AG-45; value-exchange "Instant is possible only when…".
  *Problem:* AG-45 puts two kinds of condition in one sentence. "The extension can reserve at that moment" reads as a live check. "Otherwise `confirm-first`" then makes the owner confirm a slot that is full now, and the confirm ends as `not-formed` with `slot-unavailable`. For a one-shot offering without `not_limited`, the owner chose `instant` and the node silently uses `confirm-first`. The owner is told nothing. And `instant` needs a usable signing certificate: `person_principal` returns `NotEnrolled`, `Expired` or `Stale`. When it fails, instant listings stop answering with no warning.
  *Fix:* Split the rule. At save (static): `listing.set` accepts `instant` only for a slot offering of the scheduling extension, or when `not_limited` is true, and only when signing is installed. It refuses with a named reason. At answer time (dynamic): a full slot ends `not-formed` through the writer's decision (AG-14), with no `confirm-first` fallback. If signing has stopped working, the node treats the listing as `confirm-first` and the Hub tells the owner why. Each case has a test.
  *Status:* Fixed (c2ba3de8). The rule is split. At save, `listing.set` refuses with `instant-not-allowed` and a reason. At answer time, a full slot ends `not-formed` with `slot-taken` (the code AG-14 uses, not `slot-unavailable`). A failing certificate falls back to confirm first, and the owner's Hub says why.

- [x] **P48 — minor.** *Where:* SH-7, SH-16; agreements `as_listed`; M08D D3 and D5.
  *Problem:* SH-16 says the action "names the chosen variant", and SH-7 says an enquiry names one variant. `as_listed` is one `record_id`. A variant has its own price and SKU, so the check of P41 needs to know the variant. The M08D task does not add the variant to the enquiry or to `as_listed`. `not_limited` is one flag for the offering, while stock may differ for each variant.
  *Fix:* Write `as_listed` as `{ record_id, variant? }` in the agreements doc now (M08D renames it in place anyway). D3 adds the variant to the enquiry and to `as_listed`. D0 says whether `not_limited` is for the offering or for each variant. `terms_from_listing` (P40) takes the variant.
  *Status:* Fixed (c2ba3de8). `AsListed` reserves `variant`. M08D D3 fills it and `terms_from_listing` takes it. D0 decides whether `not_limited` is set for the offering or for each variant. SH-16 names the field.

- [x] **P49 — minor.** *Where:* agreements "Language"; ML-18; ML-26 (the listing hit hook point).
  *Problem:* The doc says the acceptance half "records the language of the offering version it saw". But the half is signed later, on the node, with no screen. The viewed language and the machine-translation flag exist only at the tap. A listing hit already has a translate action (`directory_translate.test.ts`), so the person can agree to a machine translation of the title. The doc does not say how the two values reach the later half. It also does not say that the quote's text must be the listing's original text, which ML-18's rules need (`receipt_refuses_mt_of_the_original`).
  *Fix:* The tap stores `viewed_lang` and `viewed_via_mt` with the enquiry row, and the automatic half uses them. The Hub shows the existing machine-translation warning before the one action, when the hit is shown in a translation, and the person confirms it. The quote's text is copied from the listing with its original `lang`. Test: a hit shown in machine translation gives a half with `viewed_via_mt: true`; the original gives `false`.
  *Status:* Fixed (c2ba3de8). The tap stores `viewed_lang` and `viewed_via_mt` on the counterparty's node, and the automatic half uses them. The Hub shows the machine-translation warning before the tap. The quote's texts are the listing's originals.

- [x] **P50 — minor.** *Where:* value-exchange principle 13, third sentence: "A deal that goes wrong (no delivery, a no-show) needs no more steps to record than one that goes well."
  *Problem:* Nothing in the design records "no delivery" in one step. The receiver's only actions are settling halves ("I received"). The `failure` event is only `reservation-lost` (`transition.rs`). A deal that goes wrong is shown by what does not happen: after the track window the commitment ends `ended-unconfirmed`, and VX-55 says evidence shows it as unconfirmed, not failed. The sentence promises a mechanism that does not exist, and a reader of M08G may rely on it.
  *Fix:* Say what is true: "A deal that goes wrong needs no extra step. A commitment that nobody confirms ends `ended-unconfirmed` and shows as unconfirmed, with the giver's claim if there is one. The reason belongs in the opinion." If the owner wants a one-action "not delivered" record, put it to the owner as a new question. Do not add it here.
  *Status:* Fixed (c2ba3de8). Principle 13 now says what is true, using your wording. A one-action "not received" record is owner question 15. My recommendation there is to decide it with M08G.

- [x] **P51 — minor.** *Where:* AG-46; "Closing in one action".
  *Problem:* "I received everything" signs a settling half for every commitment the person receives, directly from `promised`. That is correct. But `fulfilled` and `acknowledged` are final, and the tables give no way back. The text skips only commitments that are already settled or final. It does not skip a service whose window has not started.
  *Failure case:* A person books a lesson for next Saturday and pays now. They tap "I received everything" to confirm the goods. The lesson commitment is signed as received a week early, and it is final.
  *Fix:* The action lists what it will sign (each commitment and its value) before it signs, as one confirm step. It leaves out a commitment whose `window.earliest_secs` is in the future, unless the person picks it in the list. AG-46 gets a test for this.
  *Status:* Fixed (c2ba3de8). AG-46 and "Closing in one action" list the commitments before signing, and leave out a commitment whose window has not started. There is an E2E case for this.

- [x] **P52 — minor (consistency).** *Where:* M08C task.md "Change docs" table (`:19` to `:27`); value-exchange "Event log and statement" (the line "Acceptance happens before a deal and is decided by a person at quote time"); VX-56.
  *Problem:* (a) The task.md table lists the criteria this milestone builds. Its agreements row still reads `AG-1` to `AG-23`, `AG-27` to `AG-43` and "open questions 1 and 3 to 12 decided". Its value-exchange row has no VX-5, VX-54, VX-55 or VX-56. The new rows are named in C3c and C6, but not in this table. (b) The value-exchange line is false for agree as listed, where a node accepts. (c) VX-56 says "Before agreeing, each party sees what it gives, what it receives and the difference". On the as-listed path nobody sees a quote before the acceptance, only the listing.
  *Fix:* Update the table rows. Change (b) to "decided by a person at quote time, or by the person's tap on an offering as listed (VX-54)". For (c), say that on the as-listed path the offering page shows what the person gives and receives.
  *Status:* Fixed (c2ba3de8). The task.md change-docs rows are updated. The tax line now covers the tap. VX-56 covers the as-listed path.

- [x] **P53 — minor.** *Where:* M08C task.md C3c and section 4 ("Order and parallel work").
  *Problem:* (a) C3c is sized M. It holds a node-side driver (P39), signing with no session on two nodes, the mode rule, one equality function that the native and WASM builds share (parity tests, size reports), a new enquiry field, new listing fields, Hub work in three places (the hit and listing action, the owner's confirm, the waiting line), a demo and two-node and E2E tests. C2 was split for less (open question 12). The size is L. My recommendation: keep one slice, size it L, and review it in two passes by node: the owner's side (mode, driver, quote, confirm) and the counterparty's side (tap, check, waiting line). (b) The Engage rule says "not beside C3c", which is right. The Engage fix also changes the enquiry form in `messages.ts`. C6 changes the quote form in the same file and renames its buttons. The text says the fix "may run beside C3a, C3b, C4 or C7" and leaves C6 out, while the diagram says "any time". If the driver of P39 touches `conversation`, C7 must not run beside C3c either (C7 changes `conversation`). (c) The Engage fix has no change doc, so the Demo Close-out has no `demo:` list to write. Name the story that it extends and say that its close-out is its status row. (d) C6 renames "Quote" and "Accept" to "Offer" and "Agree", and C3c adds "Agree as listed". These are Hindi strings with legal weight. The earlier slices record "Owner reviewed the new Hindi messages" in status.md. Add that step to C3c and C6.
  *Fix:* Change the C3c size, the Engage and C7 rules, the story name, and add the Hindi step.
  *Status:* Fixed (c2ba3de8). C3c is sized L, with two review passes by node. The Engage fix must not run beside C3c or C6. C7 must not run beside C3c if the driver touches `conversation`. The Engage fix extends story 50, and its close-out is its status row. C3c, C6 and the Engage fix each have a Hindi review step.

- [x] **P54 — minor (language).** *Where:* AG-48 ("folded under 'More terms'"); the Hub names "Engage" (task.md, backlog) and "Ask" (SH-16); principle 13.
  *Problem:* "Folded" is an idiom. The docs call the same Hub action "Engage" in one place and "Ask" in another, and do not say whether the Engage fix renames it. Principle 13 has two long sentences with two ideas each ("A deal that is easy to record gets recorded, and reputation counts only recorded deals").
  *Fix:* Write "shown in a section 'More terms', closed at first". Say in task.md that the Engage fix keeps or renames the button, and use one name. Split the long sentences of principle 13.
  *Status:* Fixed (c2ba3de8). "Folded" is replaced with "shown in a section 'More terms', closed at first". The Engage fix renames the button to "Ask", one name with SH-16. The long sentences of principle 13 are split.

## PR #372 review round 2 (fixes check) — Claude Sonnet 5.5 (Claude Code), 2026-11-20

Checked `26ead04e` (fixes in `c2ba3de8`, close-out in `1e69c614`, owner answer to question 15 in `26ead04e`) against P39 to P54, and the new text against the code on `main`. The fix session changed only the status lines of the finding bodies, so the findings above are as I wrote them.

**Summary:** P39 to P54 are all confirmed fixed, and the push-back on P41 is accepted. The fixes create 6 new findings, P55 to P60: 0 blocker, 2 major, 4 minor. All are Open.

- **P39.** Confirmed fixed. AG-50 and "The driver" state the problem as it is (`messages.ts:424`, no other caller). They keep the `conversation` to `transaction` cycle limit and name the bounded sweep. C3c has the stop rule and the fallback. The "online" sentence is gone. The AG-44 test makes no `transaction.sync` call and has a "both Hubs closed" case.
- **P40.** Confirmed fixed. `terms_from_listing` has its inputs, its output and five refusal reasons. `deal_terms` is on the offering, so every checked term comes from a signed record. `slot_id` is in `AsListed`. The VX definition matches. The backlog row is added. See P57 for what the function description still lacks.
- **P41.** Confirmed fixed, and the push-back is accepted. Build and compare with `PartialEq` on the whole `DealTerms`, a struct-driven test table, a missing disclosure meaning a normal quote, and the stored version envelope are all there. My proposed exceptions for `commitment_id` and `LocalText.translations` were wrong. Ids are assigned by position (`c1`, `c2`) by the same function on both nodes, and the function copies the translations. If the author adds or changes a translation, the quote no longer equals the built terms, and the node falls back to a normal quote. That is stricter than my fix, because every exception is a field that the check does not cover. The remaining risk is only a version skew between the two builds of the function, which also fails closed. See P57 for the part that is still unwritten.
- **P42.** Confirmed fixed. `AsListed.valid_until_secs` is in the signed enquiry. The owner declines with `consent-expired`. The counterparty checks the quote time, the decline and withdrawal, and the terms immediately before it signs. The Hub has the ended line and the refused-first-contact line. AG-49 has a test for each. See P58 for the decline code.
- **P43.** Confirmed fixed. "The current version wins", `offering-withdrawn`, the mode read at answer time, and the expiry `now + quote_valid_secs` (at most 7 days) are in AG-49 and its tests.
- **P44.** Confirmed fixed in substance: `open_to: anyone`, `instant_cap` (1 to 500, default 20), one open instant agreement for each counterparty, and the owner's cancel in one action. See P59 and P60 for two gaps in how the cap is counted and tested.
- **P45.** Confirmed fixed. The overview now says a fake deal costs three calls, and gives the rule that an unconfirmed commitment with no giver claim is not negative evidence about the giver. Question 14 (`formed_as`) is open for the owner, as it should be.
- **P46.** Confirmed fixed. The Enquiry table has `as_listed` and the optional `description`. `quote.from-listing` and `agreement.close` are in the Methods table, and AG-31 covers their admission. The listing field table and `can_agree_as_listed` are there. The two "Integrated spec" rows in Differences point at real steps (C12, C14 in `roym-integrated-experience-spec.md`). The rejected option has the added sentence. See P58 and P56.
- **P47.** Confirmed fixed. The rule is split into the save-time refusals and the answer-time rules, with a full slot ending `not-formed` and no fallback. See P56: the signing check at save cannot be built as written.
- **P48.** Confirmed fixed. `AsListed.variant`, the D3 and D0 rows, and SH-16 agree.
- **P49.** Confirmed fixed in the rules (the tap stores `viewed_lang` and `viewed_via_mt`; the warning is before the tap; the quote's texts are the listing's originals). The test I asked for is not in any AG row (P60).
- **P50.** Confirmed fixed. Principle 13 now says what is true. Question 15 is decided by the owner (agreed, not in M08C).
- **P51.** Confirmed fixed. AG-46, "Closing in one action" and `agreement.close` list what will be signed and leave out a commitment whose window has not started. There is an E2E case.
- **P52.** Confirmed fixed. The task.md rows now read `AG-27` to `AG-50`, and `VX-5`, `VX-54` to `VX-56`. The tax line and VX-56 cover the as-listed path.
- **P53.** Confirmed fixed. C3c is L with two review passes. The Engage fix is not beside C3c or C6, and C7 is not beside C3c if the driver touches `conversation`. The story, the status-row close-out and the Hindi steps are named.
- **P54.** Confirmed fixed. "More terms" is closed at first, the button is "Ask", and principle 13 has short sentences.

### Findings

- [ ] **P55 — major.** *Where:* value-exchange "Complete terms" ("The participant must have seen every term it agrees to"); VX-56; M08C task.md C3c Hub bullet; AG-44.
  *Problem:* The consent for the automatic signature rests on the person having seen the terms. The overview says so, and it is the reason `deal_terms` is on the offering. But no row or Hub bullet says the Hub shows those terms before the tap. The C3c Hub list has the button, a slot picker and the machine-translation warning. VX-56 says the offering page shows what the person gives and receives, which covers the price only. Nothing says the person sees the payee, the three texts (cancellation, refund, dispute path), how long the owner's quote stays valid, or the consent time. The payee has a special rule: the comment on `PaymentTerms.payee` says the UI must never present it as agreed terms.
  *Failure case:* A person taps "Agree as listed" on a directory hit that shows a title, a price and a button. The node signs an acceptance that binds a refund text and a payee that the person never saw. The overview's reason for the whole design does not hold.
  *Fix:* Add to AG-44 and to C3c's Hub bullet: before the tap, the offering page or the hit shows, in the viewed language, the price, the payee, the three texts, the quote's validity and the consent time. For the payee, say "will be paid to" (it is shown as the terms of this deal, not as agreed terms until the receipt). Add an E2E check that these fields are on the screen where the tap is.
  *Status:* Open.

- [ ] **P56 — major.** *Where:* AG-45; the listing field table (`acceptance` row); C3c.
  *Problem:* `listing.set` is a method of `catalog`. AG-45 and the table say it refuses `instant` unless "signing is installed (`transaction.signing-status`)". Two things stop this. First, `catalog` cannot call `transaction`. `transaction` already depends on `catalog` (`roym.toml`, "a quote that names a slot reads that slot from the catalog"), and the same document states that cycle rule for `conversation`. Second, each Roym service has its own signing key and its own certificate: `catalog.signing-status` and `transaction.signing-status` are two different methods. A catalog that checks its own certificate says nothing about the certificate that `transaction` needs to sign quotes and acceptances.
  *Failure case:* The C3c implementer cannot build the refusal. Either they add a `catalog` to `transaction` call (a cycle), or they check the wrong certificate, and an owner whose `transaction` certificate has expired saves an instant listing that never answers.
  *Fix:* Remove the signing condition from `listing.set`. `catalog` refuses only what it can see: `open_to`, a slot offering or `not_limited`, and the ranges. The Hub's listing form reads `transaction.signing-status` and does not offer `instant` while signing is not installed. The answer-time fallback (the node treats the offering as confirm first, and the owner's Hub says why) is the guard that holds, so keep it and test it. Change the AG-45 test list to match.
  *Status:* Open.

- [ ] **P57 — minor.** *Where:* "Complete terms: one function".
  *Problem:* The function's description does not say how a listing becomes the terms. Both nodes must get the same value, so this is part of the contract, and AG-44's test table needs it. Not written: which listing field becomes `scope` and the commitment's `description` (title, summary or both); how a `ListingTranslation` (`title`, `summary`) becomes a `LocalText` translation (`text`); the `declared_value_minor` of the owner's commitment; how `tax_included`, `fees_minor` and `methods` become `tax_minor`, `fees_minor` and `payment_methods`; the order and the `giver` of the two commitments; `window` for a non-slot offering; and the currency commitment's other fields.
  *Failure case:* The native and WASM builds, or two versions of a node, map one field differently. The check fails, so the deal falls back to a normal quote. It is safe, but it makes the one-action path fail for no visible reason.
  *Fix:* Add a mapping table (listing field to `DealTerms` or `Commitment` field) to "Complete terms: one function". Say that a mismatch between two builds shows a normal quote and is not an error.
  *Status:* Open.

- [ ] **P58 — minor.** *Where:* "Deal decline" (`DealDeclinePayload.reason: LocalText or null`); "Reason codes"; AG-49; the methods table (`instant-not-allowed`, `as-listed-not-complete`).
  *Problem:* AG-49 and the owner rules say the owner's node declines with `consent-expired` and `offering-withdrawn`, as codes. A `deal-decline` has only a free `reason` text. The Hub cannot show a message in the reader's language from a free text, and a test cannot tell the two cases apart. The "Reason codes" table says it holds every code a deal can end with or a method can refuse with. It has none of the four new codes (`consent-expired`, `offering-withdrawn`, `instant-not-allowed`, `as-listed-not-complete`).
  *Fix:* Add an optional `code: string` to `DealDeclinePayload`, outside free text, with the two decline codes. Add the four codes to the "Reason codes" table with where each is carried. The Hub's ended and withdrawn lines use the code.
  *Status:* Open.

- [ ] **P59 — minor.** *Where:* AG-45; "Instant has limits"; the listing table's `instant_cap` row.
  *Problem:* "Open instant agreements" is not defined. Closing is optional, so an agreement that nobody closes stays open until its track window ends, which is up to 30 days after the window. If "open" means "not final", the default cap of 20 limits a busy `not_limited` shop to about 20 instant agreements in 30 days, and after that every customer waits for a confirm, with no visible reason. The cap is also silent for the owner: the Hub "says why" only for a failing certificate.
  *Fix:* Define "open" for the cap, for example: an instant agreement with no half of any commitment signed, formed in the last N days (the owner's own limit on how long an unclosed deal counts). Say the Hub shows the owner "your offering is waiting for you because the limit of N was reached".
  *Status:* Open.

- [ ] **P60 — minor.** *Where:* AG-44, AG-45, AG-49 test columns; C6 criteria.
  *Problem:* Three rules have no test or criterion. (a) The machine-translation case (P49): a hit shown in a translation gives a half with `viewed_via_mt: true`, the original gives `false`. (b) The owner's "cancel unconfirmed instant agreements" action, which is the owner's way out of P44's slot-holding case. (c) The quote card's "Suggest changes", which the fix added to C6 and the overview (step 2 of the flow). It is the way to ask for changes in chat that the owner decided, but no AG or VX row and no criterion names it.
  *Fix:* Add the three to the test columns, and add "Suggest changes" to the C6 criteria line.
  *Status:* Open.
