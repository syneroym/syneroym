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

- [ ] **P25 — minor.** *Where:* "The writer's decision" step 1 and step 2
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
  *Status:* Open.

- [ ] **P26 — minor.** *Where:* "Recovery" (change.md:591); step 4
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
  *Status:* Open.

- [ ] **P27 — minor.** *Where:* "Who signs what", the `deal-decline` row
  (change.md:453).
  *Problem:* the counterparty column says "declines a quote". Since the P21
  fix, the counterparty can also withdraw its own enquiry (:358, AG-10).
  *Fix:* "declines a quote, or withdraws its own enquiry".
  *Status:* Open.
