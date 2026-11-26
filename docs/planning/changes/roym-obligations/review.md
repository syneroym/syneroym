# Roym obligations — review

*Reader: the author of the obligations change document, the fix session, and the project owner.*

Document: [change.md](change.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file". Finding ids are P1, P2, … and are never renumbered.

## Doc review round 1 — Claude Sonnet 5.5 (Claude Code), 2026-11-26

Checked against the code at `04d879b7` on
`claude/m08e-obligations-doc-review-f00454` (the draft's branch tip). The commit
changes only docs, so the code is the same as `main` at `933fb399`. Library
facts come from the cached crate source in `~/.cargo/registry`. "change.md:N"
is a line of the draft. The draft was written by Claude Opus 5.5, so this is a
review by a different model.

**Summary:** 2 blockers, 11 major, 11 minor.

**Open-question verdicts (details in P21).** Questions 2, 4 to 15 look right.
Question 1 gives a reason that is not true, and its limits clash with the
quote rules (P5). Question 3 is the owner's call, but its reason misreads the
overview and it leaves the holder no way to contest (P21).

**The M08H split (details in P20).** The EEP record leaves room for
endorsement, reissue, refusal, bundles and swaps. Two small changes now avoid
a second format change later. The one-line edit of
[M08E task.md](../../milestones/M08E-eeps/task.md) is correct and minimal.

What was checked and found correct:

- **Coverage of the overview.** Every VX criterion that M08E owns maps to an
  OB row: VX-13 (OB-12), VX-14 (OB-13), VX-15 (OB-17), VX-16 (OB-18, OB-19),
  VX-17 (OB-20, OB-21), VX-18 (OB-25), VX-19 (OB-26), VX-20 (OB-24), VX-21
  (OB-10), VX-22 (OB-1 to OB-3, OB-28), VX-23 (OB-29), VX-24 (OB-30), VX-49
  (OB-5, OB-6), VX-50 (OB-7), and VX-45 and VX-46 for EEP events (OB-34,
  OB-35). Each item under "Notes for the module documents → Obligations" is
  covered or moved to M08H on purpose: unique insert, `allow_expired`, new
  record and card types, the proof that a `stored` receipt covers a demand,
  and partial transfer (see P20, P22).
- **The "Facts checked in the code" table.** These match the code:
  - `VerifyOptions::new` uses `EmptyRevocations`, and no Roym caller passes a
    revocation source. `with_revocations` is called only in tests
    ([verify.rs:101](../../../../crates/signed_record/src/verify.rs#L101)).
    Callers such as `transaction.rs:410`, `payment.rs:231`, `verdict.rs:81`
    use `VerifyOptions::new`.
  - A revoked key fails every record with no time
    ([verify.rs:216-223](../../../../crates/signed_record/src/verify.rs#L216)).
  - The verifier checks `issued_at_secs` against the certificate's own window
    ([verify.rs:204](../../../../crates/signed_record/src/verify.rs#L204)).
    `verify_chain_at` uses the current time only to refuse a certificate
    issued in the future
    ([delegation.rs:230-234](../../../../crates/identity/src/delegation.rs#L230)).
    So OB-10 (VX-21) already holds today.
  - `MasterAnchorPayload.revoked_keys` is `Vec<String>`
    ([master_anchor.rs:27](../../../../crates/core/src/dht_registry/master_anchor.rs#L27)).
  - No WIT interface reads a master anchor. `anchor-did` in
    `authorizer.wit:24` is a caller field, not a read function.
  - `Receipt::Stored` holds only `at_secs`
    ([receipt.rs:117](../../../../crates/mailbox_format/src/receipt.rs#L117)).
    `delivered` and sender-fault `rejected` receipts are signed by the
    receiver (`is_sender_fault`, `receipt.rs:91`).
  - The mailbox stores only sealed bytes
    ([store/deposit.rs:57-79](../../../../crates/mailbox/src/store/deposit.rs#L57)).
    The message hash is made from the plaintext statement
    ([message.rs:270](../../../../crates/mailbox_format/src/message.rs#L270)).
  - `hpke` 0.15.1 has `single_shot_seal_with_rng`
    (`hpke-0.15.1/src/single_shot.rs:138`). It takes `&mut impl CryptoRng`
    from `rand_core` 0.10. See P24 for a point the draft leaves out.
  - `OPEN_MAX_TTL_SECS` is 7 days and `DEDICATED_MAX_TTL_SECS` is 30 days
    ([lib.rs:41-45](../../../../crates/mailbox_format/src/lib.rs#L41)).
    `MAX_SEALED_BYTES` is 64 KiB.
  - Eligibility is per `<interface>#<method>` (`HostedMailbox::accepts`,
    [mailbox.rs:92](../../../../crates/core/src/mailbox.rs#L92)). Every Roym
    service exports one `invoke`
    ([world.wit:4](../../../../crates/roym_transaction/wit/world.wit#L4)).
  - A verified caller's DID names the calling service, not a person
    ([admit.rs:32-36](../../../../crates/roym_core/src/admit.rs#L32)). The
    only wire table today is `directory`'s `WIRE_REACHABLE`
    ([api.rs:35](../../../../crates/roym_directory/src/api.rs#L35)).
  - `CARD_TYPES` has ten entries
    ([card.rs:11-22](../../../../crates/roym_core/src/card.rs#L11)).
  - `Commitment.eep` exists and any value but `null` is refused
    ([deal.rs:174](../../../../crates/roym_core/src/deal.rs#L174),
    [:361](../../../../crates/roym_core/src/deal.rs#L361)). `Answers` has
    only `Enquiry` and `Want`
    ([deal.rs:196](../../../../crates/roym_core/src/deal.rs#L196)).
  - `create` is all rows or none, and `MAX_BATCH_SIZE` is 200
    ([sqlite.rs:25](../../../../crates/data_db/src/sqlite.rs#L25)).
  - Seat rows are never deleted, and each state change is a new row
    ([rows.rs:1-5](../../../../crates/roym_scheduling/src/app/reservation/rows.rs#L1)).
    See P4 for what the draft did not copy from it.
  - The `bundle-manifest` is signed by the person
    ([backup.rs:79](../../../../crates/roym_core/src/backup.rs#L79)).
  - `transaction` depends on `conversation`, `scheduling` and `catalog`, and
    wakes every 30 seconds
    ([roym.toml](../../../../crates/roym_core/app/roym.toml)). `catalog`
    depends only on `profile`, so the new edge `obligation → catalog` makes no
    cycle.
- **Reuse of the agreements model.** The draft does not redefine the
  commitment model, the single writer, selective disclosure or the event log.
  It adds three things the agreements document allows or leaves open: the EEP
  kinds' transition tables, a receiver-owned hold, and `Answers.demand`. The
  separate `obligation` log follows the agreements decision. Each of these is
  in `## Deviations`.
- **The `stored` receipt idea.** Re-sealing the kept plaintext with a kept
  seed does bind the stored bytes to one plaintext. A holder cannot make a
  different message produce the same sealed hash, and cannot frame an issuer
  with random bytes. P1 shows what the check still lets through.
- **The seed.** The seed opens one message only. It does not reveal the
  mailbox secret key. The ephemeral key alone gives the message key.

## Findings

### P1 — blocker — A holder can build a valid deposit proof for a message the issuer will never accept

**Where:** change.md:511-524 (the check, steps 1 to 6, and "A holder cannot use
the proof to frame an issuer").

**Problem:** The draft says the proof cannot frame an issuer, because step 3
rebuilds the exact bytes. That stops random bytes. It does not stop a message
that is well formed but that the issuer's router must refuse. Step 4 only runs
`check_signature`. That function checks that the key named in the sender's
certificate signed the message. Its own comment says this is "not yet 'the
master allowed it': that is `check_sender`"
([message.rs:398-401](../../../../crates/mailbox_format/src/message.rs#L398)).
`check_sender` is the part that checks:

- the sender's certificate against its master, at `sent_at_secs`
  ([message.rs:440-452](../../../../crates/mailbox_format/src/message.rs#L440)):
  a failure is `BadCertificate`;
- `sent_at_secs` against the receiver's clock, with the TTL and the skew
  ([:455-472](../../../../crates/mailbox_format/src/message.rs#L455)): a
  failure is `TooOld` or `FromFuture`.

The mailbox cannot read the inner message. It checks only the depositor's own
identity (service mailbox change, `D-C4-20`). So a holder's node can seal an
inner message that has a made-up certificate, or a `sent_at_secs` seven days in
the past. The mailbox stores it and signs `stored`. The proof passes steps 1 to
6. The issuer's router then refuses it as `BadCertificate` or `TooOld`. Neither
is a sender fault, so the receiver signs nothing
([receipt.rs:35-40](../../../../crates/mailbox_format/src/receipt.rs#L35),
`is_sender_fault`). The issuer never sees the demand. The holder shows the
issuer a "weaker negative" record (the table at change.md:534-539, third row)
for a demand the issuer could never have collected. This is the framing the
draft says is impossible. It breaks the main claim of OB-6 and VX-49.

**Fix:** Make step 4 run the whole open check, not only the signature:

1. Export a pure `verify_message(plaintext, destination, deposit_id,
   stored_at_secs)` from `mailbox_format`. It runs `check_signature` and
   `check_sender` with `stored_at_secs` as "now" and the mailbox's TTL limit.
2. Add to the proof's "what it shows" that the sender's instance key may have
   been revoked, which no proof can show.
3. Add a test row to OB-6: one proof per refusal reason (`BadCertificate`,
   `TooOld`, `FromFuture`, wrong interface) that must not verify.

**Status:** Fixed (f72b2a8e). Agreed. Step 4 runs the whole open check through a new pure `verify_message(plaintext, destination, deposit_id, at_secs)` in `mailbox_format` (`check_signature` and `check_sender`, with the stored time as "now"). The text names `BadCertificate`, `TooOld` and `FromFuture` as cases it refuses, says a later revocation of the sender instance cannot be shown and is checked against the anchor at verify time, and OB-6 has one test per refusal reason. Step 6 also refuses a demand signed more than 24 hours before the stored time, so a back-dated demand cannot shorten the issuer's notice.

### P2 — blocker — The claim safety net can release the claim of an agreement that has formed

**Where:** change.md:405 (safety net), 398 (`claim-confirm`), 389
(counterparty claims in `agreement.accept`), 630 (claim table).

**Problem:** The safety net says: `obligation.drive` releases an unconfirmed
claim once the quote's expiry has passed. Look at the case where the issuer is
the counterparty (the holder wrote the quote):

1. The issuer's `agreement.accept` takes the claim and signs its half.
2. The holder's node is the writer. It files the half just before the quote
   expires and countersigns.
3. The countersigned half reaches the issuer's node after the expiry. A phone
   can be offline for hours. No clock skew is needed.
4. By then the issuer's drive released the claim, because the issuer confirms
   only "when it files the countersigned half" (agreements document, "Reserve,
   confirm and release", item 3).

Now the agreement is formed and the holder has returned version v. The claim row
is free. A second agreement can claim v. The "returned version cannot be spent
elsewhere" rule (VX-17, OB-20) fails. `claim-confirm` is also not defined for a
freed round: change.md:398 says it "never fails for a claim that exists", and
the round exists but is free. `obligation.resolve-return` "checks that the claim
round of `key:` holds" (change.md:632), so it fails too. The commitment can then
only reach `ended-unconfirmed`.

The writer-is-issuer case is safe. There the claim is confirmed before the
countersign.

The scheduling extension avoids this. A reservation there has its own expiry
(15 minutes), and a confirm after it gives `reservation-lost` and a
`commitment-failure`. The draft says a claim "never lapses on a timer"
(change.md:398) and then adds a timer.

**Fix:** Pick one rule and state it with a test:

- The expiry of the quote is the deadline for the writer's decision, not for
  the issuer to see the result. So release an unconfirmed claim only when
  `transaction` says the deal ended (it already runs `claim-release` for
  `expired`, `not-formed`, `declined` and `cancelled`), or after the quote's
  expiry plus the card retry time (30 days, as for conversation cards). Keep
  the drive's timer only as that last resort.
- Define `claim-confirm` on a freed round. If no other claim took the version,
  take a new round. If another claim did, answer `eep-claimed` so
  `transaction` can send a `commitment-failure` for the `eep-return` (the
  agreements document limits `commitment-failure` to extension hooks; add the
  EEP kinds there, as a deviation).
- Add a service test with a controllable clock: accept, advance past the quote
  expiry, file the countersign, confirm.

**Status:** Fixed (f72b2a8e). Agreed, as proposed. Nothing releases an unconfirmed claim at the quote's expiry. `transaction` releases it when the deal ends, and the drive only 30 days after the quote's expiry (the conversation card retry time). `claim-confirm` on a freed round takes a new round when the version is free, else answers `eep-claimed`, and the issuer signs a `commitment-failure` for the `eep-return`, signed by the receiver (new deviation). `eep-return` gains the `failed` end. New OB-39.

### P3 — major — The send path cannot do what the write order and the demand states need

**Where:** change.md:623 (`sent:` rows), 636, 495-496 (how the proof reaches
the guest), 668-688 (demand states `deposited`, `delivered`, `undeliverable`),
785.

**Problem:** The service mailbox change says a synchronous call never deposits
(line 242: "eligibility is the kind of call: a proxy-outbox row … A synchronous
`call` never deposits"). So every demand, acknowledgement, split request and
`deliver` must use `proxy.enqueue`. The WIT says the guest "never sees the
result … success here means 'accepted for delivery', not 'delivered'"
([proxy.wit:124-129](../../../../crates/wit_interfaces/wit/proxy/proxy.wit#L124)).
The router keeps only `DepositContext { mailbox, sent }` and no sealed bytes
([mailbox.rs:45-48](../../../../crates/router/src/proxy/mailbox.rs#L45)). Its
poll returns only an enum (`Waiting`, `Delivered`, `Rejected`, `Retry`;
[poll.rs:57-60](../../../../crates/router/src/proxy/mailbox/poll.rs#L57)). The
draft depends on four things that this path does not give:

1. **`sent:` rows.** "`obligation.drive` … creates `sent:` after the receiver
   answered or the mailbox stored it" (change.md:623). With `enqueue`, drive
   never learns either. If drive calls `enqueue` and creates `sent:` at once,
   the outbox is the retry queue and `out:` rows are redundant. If it does
   not, nothing ever creates `sent:`.
2. **The demand states.** `deposited`, `delivered` and `undeliverable` come
   from receipts. The guest sees none.
3. **The proof.** The seed, the plaintext, the `stored` receipt and the
   `delivered` receipt must be saved by the router and given to the guest. The
   draft leaves this to the E2 plan as "unverified". It is the main work of
   E2, not a detail.
4. **A retry makes a new seal.** `seal` "makes a new deposit id" on each call
   ([message.rs:275-277](../../../../crates/mailbox_format/src/message.rs#L275)).
   The proof for the first, lost deposit is gone, and the second has a later
   `at_secs`.

**Fix:** Add a short "Send path" section that says:

- Every obligation call uses `enqueue`, with the idempotency key of
  change.md:786.
- A new host feature, for example `proxy.deposit-result(idempotency-key)`,
  returns the state and the proof (seed, plaintext, receipts) for one queued
  call. The guest reads it from `obligation.drive`. Drop `out:`/`sent:` rows if
  `enqueue` is the queue, or keep them and say who owns retries.
- Which receipt rows are kept in the router's backup or not, and what a restore
  loses.
- Move the demand-state rows that depend on this out of OB-23 until the
  feature exists, or make E2 deliver it first.

**Status:** Fixed (f72b2a8e), with a change to the proposal: the `out:`/`sent:` rows stay, but only to cover the gap between storing a record and handing it to `enqueue`; `sent:` is written when `enqueue` accepts, and the proxy outbox owns retries. New section "The send path": every call is `proxy.enqueue`; E2 adds the "deposit first" option and a deposit status read that returns the state and one proof per deposit (a retry is a new seal); `obligation.drive` copies each proof into its own data, so it rides in `obligation`'s backup, and what a substrate-only restore loses is stated. OB-23 stays, and says its receipt states are built when the read exists, instead of moving them out. New OB-51.

### P4 — major — The claim key has no generation, so a claim after a release is undefined

**Where:** change.md:615 (`key:<agreement>:<commitment_id>`), 397 ("Idempotent on
that key"), 631 (release only "when the round is the one `key:` names").

**Problem:** The draft says its claim rounds copy the scheduling extension. The
scheduling extension also has a `Key` row with a `generation` that points at the
newest reservation of one (agreement, commitment)
([rows.rs:51-58](../../../../crates/roym_scheduling/src/app/reservation/rows.rs#L51),
[reservation.rs:93-116](../../../../crates/roym_scheduling/src/app/reservation.rs#L93)).
The reason: after a release, the same pair can be reserved again. The draft's
`key:` row has no generation. Take the flow where `agreement.accept` claims
EEP A, then another reserve fails, so it releases A and returns
`reserve-failed`. The person retries the same accept (same version, so same
agreement id). `claim` finds `key:` and, being "idempotent on that key",
answers success for a round that is free. The version is not locked, but the
caller thinks it is. The same happens in the writer's recovery
(change.md:630, "reruns the decision") after a release.

**Fix:** Give `key:` a generation, as the scheduling extension does, or say
that `claim` reads the round's `free:` row and takes a new round when it is
free. State what `claim` answers for a freed round that another agreement has
since taken (`eep-claimed`). Add both cases to OB-20.

**Status:** Fixed (f72b2a8e). Agreed. `key:` rows carry a generation, as the scheduling extension's `Key` rows do. A repeat answers success only while the newest generation's round holds; a claim after a release takes a new round, or answers `eep-claimed` if another agreement took the version. Both cases added to OB-20.

### P5 — major — Version 1's expiry can be too short, or already past, when the agreement forms

**Where:** change.md:256 (field rule), 291 (validator), 312 (what "matches"
means).

**Problem:** Two rules use two different start times.

- The validator (change.md:291) checks `expires_at_secs` against the **quote's**
  `issued_at_secs`: from 7 days to 5 years.
- The field rule (change.md:256) checks it against **version 1's**
  `issued_at_secs`, which is the time the agreement forms.

A quote lives 300 seconds to 90 days
([transaction.rs:79-80](../../../../crates/roym_core/src/transaction.rs#L79);
confirmed by the agreements review). Take a quote signed on day 0 with an EEP
expiry on day 8. The agreement forms on day 30. Version 1 would be signed with
an expiry that is already past, and the "matches" rule (change.md:312) forces
it, because the version's expiry must equal the commitment's. The issuer's
version 1 would break its own field rule. The holder gets an EEP that was born
expired. VX-14 and OB-11 do not cover this.

**Fix:** Pick one:

- Store a duration (`expires_in_secs`, from 7 days to 5 years) in
  `EepIssueTerms`, and let version 1 compute `expires_at_secs` from its own
  `issued_at_secs`. Then the "matches" rule compares the duration.
- Or keep the absolute time and make the writer refuse formation when the time
  left at formation is below 7 days, with a reason code (for example
  `eep-expiry-too-near`) in the agreements reason table.

Add a service test with a controllable clock for both.

**Status:** Fixed (f72b2a8e). Took the first option: `EepIssueTerms` holds a life (`expires_in_secs`, 7 days to 5 years), and version 1 computes `expires_at_secs` from its own signing time. The "matches" rule compares the life. OB-14 and the new OB-37 test it.

### P6 — major — `eep-return` ends `ended-unconfirmed` in the same sync that should close or reinstate it

**Where:** change.md:331 (track end), 322-327 (table), 330 ("A close that
arrives after the commitment ended `ended-unconfirmed` … Only the commitment's
state stays"), 409-417.

**Problem:** The `eep-return` track end is "the latest track end of the other
commitments the issuer gives" (change.md:331). The issuer resolves the return
when those commitments are final, and they become final by `tick` at their
track end. So at that moment `eep-return` ticks to `ended-unconfirmed` too. The
version (close or reinstatement) comes later, as a card. It arrives when the
commitment is already final, and "no change" applies. So on the unconfirmed
paths (rows 3 and 4 of the resolve table) the `eep-return` commitment will
nearly always read `ended-unconfirmed`, never `closed` or `reinstated`. The EEP
is correct (closed or live), but the commitment evidence is wrong and the
`reinstated` end is almost unreachable. M08G would read a holder's `eep-return`
as unconfirmed when the issuer in fact closed it.

**Fix:** Do not tick an `eep-return` that has a claim until the resolution is
done. For example: its track end is the latest other track end plus a fixed
resolution window (for example 7 days), and a `tick` only moves it if the
issuer's snapshot shows no version. Add an OB row (or extend OB-21) that checks
the commitment's state after each row of the resolve table, not only the EEP.

**Status:** Fixed (f72b2a8e). Agreed. The `eep-return` track end is the latest track end of the issuer's other commitments plus a 7-day resolution window, and its `tick` moves it only when no matching version is held. OB-21 now checks the commitment's state, not only the EEP.

### P7 — major — Closing a pure set-off at once races with `agreement.cancel`

**Where:** change.md:413 (row 1: "close at once"), 436, 403, and the agreements
document's `cancel` event ("allowed only when every commitment … is
`promised` or `payment-requested`").

**Problem:** When the issuer gives nothing else (row 1), its node closes the
returned EEP at the first snapshot (`agreed`, seq 1). The holder's
`eep-return` stays `promised` until the closing card reaches the writer. Until
then `agreement.cancel` is allowed, because every commitment is still
`promised`. Take a deal where the holder A is the writer and returns B's EEP,
and the issuer B is the counterparty and gives nothing else. B's node files
seq 1 and closes the EEP at once. A cancels before B's card arrives. A's
snapshot says `cancelled`. B's EEP is closed, and A lost it in a cancelled
deal. The draft only says a close after a final state "still ends the EEP"
(change.md:330), which makes this the intended result. A two-way set-off is
safer: if the writer is also an issuer, its own close moves a commitment on its
own node, so `cancel` is refused. The one-way case is not safe.

**Fix:** Pick one rule and test it:

- The issuer closes a pure set-off only after the writer's snapshot shows that
  every commitment has moved out of `promised` (so cancel is refused with
  `commitment-moved`), or
- `agreement.cancel` is refused whenever the agreement has an `eep-return`
  (add it to the agreements reason table), or
- the issuer waits for a short grace period after seq 1.

Add a two-node race test for set-off plus cancel.

**Status:** Fixed (f72b2a8e), with a change to the proposal, pending the owner: open question 18. None of the three options fits. Refusing cancel for every `eep-return` removes "cannot deliver" from every redemption; a grace period does not help an offline node; "wait for a moved commitment" never ends in a pure set-off, where nothing else moves. The race exists only when the counterparty receives an `eep-return`, so the writer refuses `agreement.cancel` with `eep-return-to-counterparty` in exactly that case. It narrows the agreements document's cancel rule, so it is an owner question. New OB-41, deviation added. **Decided 2026-11-26, owner:** open question 18 accepted as recommended (the narrow rule of round 2, P30).

### P8 — major — The resolve table is ambiguous and incomplete

**Where:** change.md:411-417.

**Problem:**

- Rows 3 and 4 both say "any". An agreement with one issuer-given commitment
  `ended-unconfirmed` with `claimed: true` and another with `claimed: false`
  matches both rows ("close" and "reinstate"). Nothing says which wins.
- An `eep-issue` commitment that the issuer gives (for example the change EEP
  of "Partial redemption", change.md:424-432) can end `ended-unconfirmed`. But
  "`claimed` is absent on the EEP kinds" (change.md:329), so rows 3 and 4 do not
  apply. The case is not in the table. A redemption with goods fulfilled and a
  change EEP never issued has no result.
- The table lists `issued` for row 2 but says nothing for a mix of `fulfilled`
  and `ended-unconfirmed`.

**Fix:** Write the rule as a function with a clear order, for example: any
`failed`/`cancelled` is handled elsewhere; otherwise reinstate if any
issuer-given commitment is `ended-unconfirmed` and not claimed (an `eep-issue`
that was not issued counts as not claimed); otherwise close. Test it with a
table over all combinations, not "one case per row" (OB-21 says one case per
row, which would miss mixes).

**Status:** Fixed (f72b2a8e). Agreed. The table is now a function with an order: failed or cancelled is handled elsewhere; reinstate if any issuer-given commitment is unconfirmed and "not claimed" (an `eep-issue` that was never issued counts as not claimed); otherwise close. Mixes decide by the worst case. OB-21 tests every combination of up to three commitments. Option B of open question 3 is one line of the function.

### P9 — major — A confirmed claim has no exit when the writer's snapshots stop

**Where:** change.md:658 ("A claim always ends"), 409, 331.

**Problem:** A confirmed claim ends only through [Resolving a
return](#resolving-a-return), which runs when the issuer "writes or files a
snapshot" showing the other commitments final. When the issuer is the
counterparty, the final states reach it only through snapshots from the writer
(the holder). The writer's `tick` makes a commitment final. If the writer never
syncs again (a lost phone, an uninstalled app), the issuer's node holds a
confirmed claim for ever, and the holder's EEP is locked. The text says "every
commitment becomes final at its track end". That is true only on the writer's
node. The "ways out" column of the states table says a claim always ends. That
is the check this review was asked to make: every state has a way out.

**Fix:** Let the issuer's node apply `tick` itself for the commitments it gives,
at the track end it computes from its own disclosures (the agreements document
says each party computes the date), plus a grace period. Then it resolves with
`claimed` read from the last snapshot it holds. State this as the last-resort
rule and add a test (OB-21 variant) where the writer stops.

**Status:** Fixed (f72b2a8e). Agreed. At the `eep-return`'s own track end plus 7 days, the issuer's node resolves from its own view: a commitment its snapshots do not show final counts as unconfirmed, and `claimed` is whether the issuer itself signed a giver's half, which it always knows. The states table lists it as a way out. New OB-40.

### P10 — major — The wire methods let any verified caller make the receiver sign, store and send

**Where:** change.md:356-363 (automatic declines), 782-787 (admission and
limits), 795 (`deliver`), 762.

**Problem:** A verified caller is only a service DID. Any node can have one.
The limit is 60 calls per hour per caller (change.md:787), and one call can
carry 16 records. So one caller can make 960 demands an hour. Each demand
for an unknown EEP makes the node sign an `eep-decline` with `eep-unknown`,
create `decline:` and `out:` rows, and send a call to the demand's `reply_to`.
The `reply_to` is chosen by the demand's signer, and the signer can be any
fresh key. So:

- Storage and signing work grow with caller count, which costs an attacker
  nothing.
- The node sends calls to any address the attacker names (a reflection
  channel; the sent records are small, but the number is not capped by caller).
- For `obligation.deliver` the draft says "Verifies each against the EEP and the
  issuer, stores it, acknowledges new versions" (change.md:795). On the
  holder's node, an EEP version of an EEP it never heard of has nothing to be
  verified "against". The node would store any issuer's record and acknowledge
  it. The wallet fills with junk from strangers.

**Fix:**

- For `eep-unknown`, `not-holder` and any decline that needs no record of the
  EEP, answer in the call's own reply and sign nothing. Sign a decline only for
  a demand from the holder of a known EEP.
- Add the acceptance rule for `deliver` on the holder's node: store a record
  only when it answers something the node asked (a demand, a split request, a
  holdings request) or extends an EEP it already holds.
- Add a total cap on stored open demands per EEP and a limit that does not
  depend on the caller DID (one open demand per EEP version already exists;
  also add an unknown-EEP counter).
- Add an OB row for each.

**Status:** Fixed (f72b2a8e), with a change to the proposal: "answer in the call's own reply" only reaches a direct caller; on the mailbox path nothing comes back, which is acceptable for a caller that is not a holder. `eep-unknown` and `not-holder` sign and store nothing. `deliver` on the holder's node stores only records that answer a request it sent or extend a chain it holds. One open demand per (EEP, version) on the issuer. A node-wide limit of 100 such calls an hour, whatever the caller, because a fresh caller DID costs nothing. New OB-42, OB-43.

### P11 — major — A split is automatic for any holder and has no limit, and this is not in the deviations

**Where:** change.md:449 ("A split is mechanical … the node does it with no
person action, and a refusal … would count against the issuer"), 440-449, 258,
633; overview "Split".

**Problem:** The overview says "The issuer signs every split" and "A holder asks
the issuer to split". It does not say the issuer may not refuse. The draft adds
a rule that a refusal counts against the issuer, and makes the node sign with no
person present. This is a change from the overview, and it is not in
`## Deviations`. It is also an abuse path. A holder of a ₹300 EEP can split it
into 16 parts and split each part again. The only stop is 1 minor unit per
part. Each split costs the issuer 17 signatures, about 20 rows and a delivery,
and the parts raise the "outstanding" count and the issuer's wallet. The 60 per
hour limit is per caller DID and so does not stop it.

**Fix:**

- Add the rule to `## Deviations` and to the open questions, with the owner's
  decision: may an issuer refuse a split, and for which reasons?
- Add a limit (for example a smallest part, a largest number of live EEPs per
  holder per issuer, or a cooldown) and let the issuer's person decline a
  split above it with a coded decline.
- Add OB rows for the limit.

**Status:** Owner decision: open question 16. The doc now says the overview does not settle whether an issuer may refuse a split, records it in `## Deviations`, and applies the recommendation marked "pending open question 16": automatic inside limits (parts of at least 100 minor units or the whole value, at most 64 live EEPs of one issuer per holder), `split-limit` above them, and Roym does not mark a refusal as evidence. New OB-44. Applied in f72b2a8e. **Decided 2026-11-26, owner:** open question 16 accepted as recommended. The "pending" marks are removed.

### P12 — major — The countersign rule is weaker than the draft says for third parties, and it does not cover other records

**Where:** change.md:569-578, 137 (OB-28), 105 (OB-3).

**Problem:**

1. The draft says a forger with a stolen key cannot make a fake EEP valid
   because "the issuer's own store stops that case". That holds only for the
   issuer's own node. A third party that checks evidence (a reader in M08G, a
   new holder in M08H) has no access to the issuer's store. The forger holds the
   stolen key and can make a second identity of their own act as "holder". That
   identity signs an `eep-acknowledgement` of the forged version with a
   backdated `issued_at_secs` earlier than `effective_from_secs`. OB-3 only
   needs "an identity other than the master", a "countersign that verifies on its
   own", and a date. All three are met. The countersign's date is signer-written,
   like the date it protects.
2. OB-3 applies to any record signed with a compromised key. But the draft makes
   a countersign for EEP versions only (the holder's acknowledgement). An
   `eep-decline`, a `eep-demand`, a `eep-split-request` and a
   `eep-holdings-request` have no countersign in the design. So every demand or
   decline signed by a key that is later revoked as `compromised` stops
   verifying. A holder's "no answer" evidence (OB-23) and an issuer's decline
   would disappear after the first compromise. The acknowledgement itself (the
   countersign) has the same problem if the holder's key is the one that leaks.

**Fix:**

- State plainly that for a third party, a countersign by an identity the
  verifier has no other reason to trust is weak. Add a stronger anchor if one
  exists: a countersign is accepted only with a third-party-signed time (for
  example the mailbox `stored` receipt for the acknowledgement's message, or a
  `delivered` receipt). Otherwise record this as a known limit of the
  overview's rule, in `## Deviations`, with an owner decision.
- Say what happens to demands, declines and requests after a `compromised`
  revocation. Either add a countersign (for example the issuer's `delivered`
  receipt for a demand), or say that they stop counting as evidence and the
  Hub shows it.

**Status:** Point 1: Owner decision: open question 17. The doc states the limit plainly (a third party cannot see the issuer's store, and an acknowledgement's time is written by its signer) and recommends that a third party count an acknowledgement at full strength only with a mailbox `stored` time of its own deposit. Point 2: Fixed (f72b2a8e). New subsection "Countersigns for every record": the holder acknowledges declines as well as versions, and a demand, request, decline or acknowledgement sent "deposit first" has a third-party time through its deposit proof. A record with no countersign shows as "signed with a key revoked as compromised". New OB-49. The case where both keys are compromised is stated as a limit. **Point 1 decided 2026-11-26, owner:** open question 17 accepted as recommended. A third party counts an acknowledgement at full strength only with a third-party `stored` time.

### P13 — major — E1 changes the type that three other crates already read, and the draft does not list them

**Where:** change.md:547-576, 1010 (E1 task), 158-162.

**Problem:** `revoked_keys` is read as a plain list of strings in more places
than the draft names:

- [auth/service.rs:292-293](../../../../crates/auth/src/service.rs#L292) checks
  `revoked_keys.contains(temp_did)` and also the **hex** form of the public key.
  The draft's `RevokedKey.key` is "the delegated key's `did:key`" only.
- [mailbox/caller.rs:105](../../../../crates/mailbox/src/caller.rs#L105) checks
  the instance DID against the list.
- [core/dht_registry/revocation.rs:25](../../../../crates/core/src/dht_registry/revocation.rs#L25)
  checks the audience of a UCAN edge.
- `RegistryClient::revoke_key`
  ([client.rs:553-581](../../../../crates/core/src/dht_registry/client.rs#L553))
  writes the list, and the supervisor's `revoke-instance` writes it too
  ([supervisor.wit:225-243](../../../../crates/wit_interfaces/wit/supervisor/supervisor.wit#L225)).
  So does `mailbox/test_support.rs:147`.

Each of these must now read `RevokedKey.key`. For these callers a revoked key
stays refused at once. The `effective_from_secs` time only matters to the
record verifier. The draft also does not say how an existing entry is written
by `revoke-instance` (which reason? which time?). Nor how `RevocationSource`
(`check_did(&str)`, no master argument,
[verify.rs:14](../../../../crates/signed_record/src/verify.rs#L14)) knows whose
anchor lists a key. The existing `RevocationSet` is a flat bag of DIDs. If E1
fills one set from the issuer's anchor and the holder's anchor together, a
holder could list the issuer's delegated key in its own anchor and make the
issuer's records fail. Rule 1 (change.md:566) says "M's anchor", which is
right, but the trait cannot say it.

**Fix:** List every reader and writer of `revoked_keys` in E1, and say what each
does with the new entry (key forms accepted, reason chosen by `revoke-instance`
and `revoke_key`). Change `RevocationSource` so a check takes the master DID
(`check_key(master, key)`), and add a test that a key listed in the wrong
master's anchor has no effect.

**Status:** Fixed (f72b2a8e). Agreed. E1 lists every reader and writer of `revoked_keys` (auth with both key forms, the mailbox caller check, the UCAN edge check, `revoke_key`, `revoke-instance`, test support) and what each does: live-key checks still refuse at once; `revoke_key` takes a reason and a time (default `compromised`, now); `revoke-instance` writes `retired`. `RevocationSource` becomes `check_key(master, key)` over a map by master. New OB-50 tests that a key in the wrong master's anchor has no effect.

### P14 — minor — It is not said who verifies an `eep` card, or in what order the holder moves the commitment

**Where:** change.md:312-313, 340-341, 635, 820-821.

**Problem:** The commitment moves to `issued` when "the writer holds a version 1
that matches". `transaction` must read the card. OB-4 says `obligation` applies
the revocation rule to every EEP record. The draft does not say whether
`transaction` verifies the card itself (with `EmptyRevocations`, as today) or asks
`obligation`. If `transaction` moves the commitment first and `obligation.receive`
then fails (for example a retired key), the commitment is `issued` (final) but
no held EEP exists. The same holds for `obligation.issue` with a "found card"
(change.md:629): the draft does not say that `obligation` checks the card
equals what it would have signed (derived id, cause, holder, face value).

**Fix:** Say that `transaction` calls `obligation.receive` first and moves the
commitment only on its success, and that `obligation` returns the verdict. Say
that a found card is checked against the derived id and the commitment before it
is stored.

**Status:** Fixed (f72b2a8e). Agreed. The filing node calls `obligation.receive` (or `eep.verify`) first and moves the commitment only on success. A found card given to `obligation.issue` is checked against the derived id and the commitment before it is stored (OB-14).

### P15 — minor — The deposit proof is not complete

**Where:** change.md:498-518, 529-541.

**Problem:**

- Step 1 needs "an issuer-signed version of the EEP that D names" to tie the
  service id to the issuer. `DepositProof` carries no version, so the verifier
  has no way to get one. Add it to the proof, or say the bundle is
  `DepositProof` + demand + version.
- `delivered_receipt` is in the struct but no step checks it. The "negative"
  row of the table (change.md:537) depends on it. It needs: the certificate's
  master equals `destination`, the signature over the statement rebuilt from the
  plaintext (the receipt statement signs `sender`, `message_hash`, `state`,
  `at_secs`; [receipt.rs:137-158](../../../../crates/mailbox_format/src/receipt.rs#L137)),
  and `at_secs` rules as in `verify_signed`.
- A message that the issuer received but answered with an error ends as an
  unsigned `rejected(call_failed)` (see change.md:785 and `RejectCode::CallFailed`).
  The holder then reads it as "stored, never collected". The table should have a
  row for it, or the Hub should not say "never collected".
- `StoredStatement` has no `sender`, but the `delivered` statement does.
  Another holder who knows the sealed bytes could present the same stored
  receipt. Add `sender` (the depositor's master DID), as the delivered receipt
  does.
- `ttl_secs` of the demand's deposit is not said. Say it is the smaller of the
  mailbox's limit and `answer_by_secs` minus now, or why not.

**Fix:** Add these to the proof struct and the check, one test each.

**Status:** Fixed (f72b2a8e). Agreed, with one addition: for the `rejected(call_failed)` case the doc does both things the reviewer offered: a row in the "No answer" table that does not count it against the issuer, and Hub wording that never says "never collected" for it. `DepositProof` gains `eep_version` and `sender`; `StoredStatement` gains `sender`; step 7 checks a `delivered` receipt as `verify_signed` does; the deposit's time-to-live is the smaller of the mailbox limit and the time left to `answer_by_secs`.

### P16 — minor — Catch-up and restore details

**Where:** change.md:582-600.

**Problem:**

- Rule 3 allows `w.issued_at_secs` up to 24 hours after `restored_at_secs`. The
  text says a version after the restore "would be in the node's own new state".
  So rule 3 should have no skew. With it, a back-dated forgery between the
  restore time and 24 hours later is accepted if the node signed nothing for
  that EEP since (rule 5). The skew has no stated reason: both times come from
  the same node's clock.
- `backup_at_secs` is the manifest's `issued_at_secs`. The manifest is signed at
  the end of the export. A version signed during the export is in neither the
  bundle nor above `backup_at_secs`, so rule 2 refuses a true version. Use the
  time the export started.
- Rule 4 needs the anchor read. If the read fails, say whether catch-up is
  refused or allowed. It should be refused.
- The claim rows are lost in a restore too. A returned version can be claimed
  again by a second agreement until the holder shows it. This is the same class
  as the stale reissue, but the draft does not say so. Add a sentence and a
  backlog row.

**Fix:** Edit the rules as above.

**Status:** Fixed (f72b2a8e), with a change to the proposal: the manifest has no start time, so `obligation.export` writes `export_started_at_secs` in its own section, which the signed manifest digest covers. Rule 3 has no skew. Rule 4 never allows catch-up on an unreadable anchor: the node signs nothing and retries on the next drive. Lost claim rows are named, with a backlog row at close-out.

### P17 — minor — The 16-record limit does not keep a call under the size of a sealed message

**Where:** change.md:787, 272, 633.

**Problem:** A sealed message is at most 64 KiB, and the call's JSON carries each
envelope as an escaped string. An EEP version with its delegation certificate is
about 2 to 3 KB ("a few hundred bytes of payload plus the envelope and its
certificate", change.md:272). `obligation.acknowledge` carries up to 16
acknowledgements and 16 versions. A split's `deliver` carries the closing
version, 16 parts and up to 17 envelopes. Both can pass 64 KiB. A deposit over
the limit fails with `TooLarge` and is not retried. The cards in this repo
stay under `MAILBOX_CARD_BODY_BYTES` (13 KiB,
[card.rs:40](../../../../crates/roym_core/src/card.rs#L40)).

**Fix:** Give a byte limit per call (and split larger sets into several calls
with different idempotency keys). Measure an `eep` version in E3, as the draft
says, and write the number here.

**Status:** Fixed (f72b2a8e). Agreed. A call's request is at most 48 KiB of JSON; a larger set goes as several calls with their own keys. E3 measures an `eep` version and writes the number. New OB-48.

### P18 — minor — The idempotency key of `deliver` blocks a second legitimate delivery

**Where:** change.md:786, 762.

**Problem:** The key is "a hash of the sorted `record_id`s". The receiving node
answers a repeat of the same key from the first answer without running the call
(`proxy.wit:61-66`; [call_dedup.rs](../../../../crates/router/src/call_dedup.rs)).
A holder that lost its data a second time sends a new holdings request. The
issuer sends the same records. The key is the same, so the holder's node does
not run `deliver`, if the first answer is still in its fence. The hourly limit
makes it worse: the holder can ask only once an hour.

**Fix:** Include the request's `record_id` (or a counter) in the key of every
answer, so one logical answer per request.

**Status:** Fixed (f72b2a8e). Agreed. The key of every answer names the request it answers (plus a part number when split), so one logical answer per request.

### P19 — minor — The "above the listed price" check is not defined for every listing

**Where:** change.md:380, 125 (OB-19), 358.

**Problem:** OB-19 compares the quote's declared value with "that listing
version's fixed price". `PaymentTerms` has four models: `Fixed`, `PerHour`,
`PerUnit` and `QuoteOnly`, and `amount_minor` is optional
([listing.rs:68-90](../../../../crates/roym_core/src/listing.rs#L68)). For `PerUnit`
and `PerHour`, the declared value depends on a quantity the quote does not carry
as a field. The comparison gives a false warning. The listing's currency can
also differ from the agreement's (the EEP's currency only has to appear in
*one* active listing, change.md:336). The text says "A listing with no fixed
price shows 'price not checked'", which covers only two of the four cases.

**Fix:** Say the check runs only for `Fixed` listings in the same currency, and
shows "price not checked" in all other cases. Add both cases to OB-19.

**Status:** Fixed (f72b2a8e). Agreed. The check runs only for a `fixed` listing with an amount in the agreement's currency; every other case shows "price not checked". OB-19 has one case per model and one for another currency.

### P20 — minor — Two small changes would leave more room for M08H, and the task-file edit is correct

**Where:** change.md:743-751, 237-245, 907-922; M08E `task.md:16`.

**Problem:**

- The closing version lists parts with a `holder_did` each, so partial transfer
  can reuse it. But the **request** has `parts: [ i64 ]`
  (change.md:748), with no holder. For a partial transfer the request needs one
  holder per part, and the old holder's endorsement. So H0 would need a second
  version of `eep-split-request`.
- `Cause.split-part` has no field for an endorsement, and `Cause.transferred`
  is not defined. The overview also says each record type has a version that
  fixes the meaning. Adding a cause kind later is a change of version 1 for a
  node that refuses an unknown field (OB-7).
- The overview's partial-transfer note says "the issuer reissues two EEPs in one
  step". The draft's answer is sound: M08E builds one atomic write that closes
  a version and makes the parts, and partial redemption uses a change EEP. H0
  adds the endorsement, the double-spend rule and the refusal. That meets
  "without a reshape" if the request changes below.

**Fix:** Make `parts` an array of `{ face_value_minor, holder_did? }` now. An
absent holder means "the same holder", and M08E refuses any other value. Add
`endorsement: null` as a reserved optional field to `split-part`, or say in the
"Left for M08H" table that H0 raises `eep` and `eep-split-request` to version 2
and keeps version-1 EEPs valid. The `task.md` edit is correct: the new text
matches this document and the M08H task (H0 extends the module doc with
endorsement, reissue, refusal, swaps), and nothing else changed.

**Status:** Fixed (f72b2a8e), with a change to the proposal: the split request's `parts` are objects with `holder_did` (null in M08E), and the reserved `endorsement` is on the request, not on the `split-part` cause, because the old holder signs the request and the endorsement is its consent. `{ kind: "transferred", endorsement }` is reserved and refused by M08E nodes with `cause-not-supported`; if H0 needs more, it raises `eep` to version 2 with the same sentence. The "Left for M08H" table says so.

### P21 — minor — Two open-question recommendations need a correction

**Where:** change.md:947-949 (question 1), 954-957 (question 3).

**Problem:**

- **Question 1.** The reason says the 7-day lower limit means "a holder can
  always make one demand that the issuer must answer before the EEP ends". That
  is not true. A holder who signs a demand on day 6 of a 7-day EEP gets a
  notice that ends on day 13, but the EEP ends on day 7, so the node declines
  with `eep-expired` at once. The reason only holds if the limit is measured
  after the demand. Also see P5 for the start time of the limit.
- **Question 3.** Closing on an unconfirmed delivery that the issuer claimed
  shifts a new risk to the holder. The overview says "the holder carries only the
  risk that the issuer fails to deliver". With this rule the holder also carries
  the risk of a false claim of delivery, and the holder has no way in the
  protocol to say "not received" (the receiver's half only says "received"). The
  draft's reason ("not, on its own, negative evidence") answers a different
  point. This is the owner's call, but the document should say that the holder
  has no way to contest, and offer one: for example a holder's signed
  "not received" half that moves the result to reinstate.

The rest of the recommendations (2, 4 to 15) are consistent with the code and the
overview. For question 13, add what a node does when it can find no mailbox
host (the deploy should fail clearly or fall back).

**Fix:** Edit both texts, and ask the owner about the holder's "not received"
half in question 3.

**Status:** Question 1: Fixed (f72b2a8e). The reason is rewritten: a demand that reached the issuer or its mailbox before expiry must still be answered in its notice, and the lower limit is the open mailbox's longest hold. Question 13: Fixed (f72b2a8e): with no mailbox host the deploy fails with a clear error and does not fall back. Question 3: Owner decision: open question 3. It now lists options A (the draft), B (the overview's reading) and C (A plus the reviewer's signed "not received" half), with the reviewer's point on the risk to the holder. Recommendation: A now, and decide C together with the agreements document's open question 15, which the owner placed with M08G. **Decided 2026-11-26, owner:** question 3 accepted as recommended: option A in M08E, and option C is decided in M08G with the agreements document's open question 15. Question 13 is reversed by the owner: `obligation` is not required to have a mailbox, and a deploy without one succeeds with a warning. The "deploy fails" fix above no longer holds; see the owner decisions note at the end.

### P22 — minor — Some differences from the overview and the agreements document are not in `## Deviations`

**Where:** change.md:1016-1036.

**Problem:** The draft lists 17 deviations. Each one given is justified. These
are missing:

1. Endorsement, refusal, swap, and the bundle-refund decision (overview note
   "New record and card types" and open question "A refused EEP in a bundle")
   are moved to M08H by the owner. A short row keeps the overview's list in
   step.
2. A split is automatic and cannot be refused (P11).
3. EEP records carry no hidden fields (change.md:215). The overview's rule is
   that deal amounts are hidden by default. An `eep` version shows its face
   value to whoever receives it, and links to the agreement by `cause`.
4. `obligation` is the second wire-reachable service with its own wire table,
   and its first `VerifiedOnly`-only table (the AGENTS.md paragraph changes;
   close-out lists it, deviations do not).
5. New reason codes (`eep-claimed`, `eep-expired`, and the others) need rows
   in the agreements document's reason-code table. Close-out names the
   service mailbox change and the overview, but not the agreements document.
6. `commitment-failure` for the EEP kinds, if P2 is fixed that way.
7. The claim has a timer after all (P2), against "never lapses".

**Fix:** Add the rows, and add the agreements document to the close-out list.

**Status:** Fixed (f72b2a8e). Agreed. All seven rows are in `## Deviations` under "Added in review round 1", plus the life field, the track window, the issuer's own tick and the split of AG-24/AG-25. The close-out names the agreements document (reason codes, cancel rule, `commitment-failure`, card count).

### P23 — minor — OB rows missing, and the E3/E4 split of rows is wrong in one place

**Where:** change.md:100-148, 1012-1013.

**Problem:** These rules in Design have no OB row:

- Partial redemption with a change EEP ("What a participant can do", change.md:56;
  Design change.md:424-432).
- The holder's `held-check` before it signs an `eep-return`.
- The drive's release of a claim (P2) and the claim's `confirmed:` row.
- `demand-open` (change.md:726) and `answer_by_secs` (signing time plus 7 days).
- The limits of change.md:787 and the once-an-hour holdings answer (change.md:762).
- The Hub text "revocation not checked" is in OB-4, but not "price not checked",
  "above the listed price" without a price, or the three "no answer" levels
  (OB-23 covers one).
- OB-13 holds three rules in one row (the listing rule on the issuer's node,
  the integer range on every node, "not checked" on the receiver). Split it so
  each can fail on its own.
- The task list puts OB-32 (demand acceptance) and OB-31 (the wire table) in E3
  (change.md:1012), but the demand handler is in E4. E3 cannot test them.

**Fix:** Add the rows and move OB-31/OB-32 to E4, or build the wire table and
the demand acceptance in E3 and say so.

**Status:** Fixed (f72b2a8e), with a change to the proposal: instead of moving OB-31 to E4, E3 builds the wire table with all five entries and the `acknowledge` and `deliver` handlers (both are needed for E3's acknowledgements), and E4 adds the `demand`, `split` and `holdings` handlers with OB-32. New rows OB-37 to OB-51 cover partial redemption, `held-check`, the late confirm, `demand-open` and `answer_by_secs`, the limits, the Hub texts and the split of OB-13. OB-23 now names all "no answer" levels.

### P24 — minor — Small facts and wording

**Where:** several.

**Problem:**

- change.md:633 says "At most 21 rows" for a split. The list is 1 claim, 1
  closing version, 16 parts, 1 `req:` and 1 `out:`, which is 20.
- change.md:494 leaves the seeded generator to E2. `hpke` 0.15.1 needs
  `rand_core` 0.10, and the workspace pins `rand = "0.9"`, which uses
  `rand_core` 0.9. No `rand_chacha` for 0.10 is in the cache (only 0.3.1 and
  0.9.0). So E2 needs a new generator crate or a small wrapper. The seed can
  also feed `Kem::derive_keypair`, which `crypto.rs` already uses, and that may
  avoid the new crate. Say so in the E2 task.
- change.md:392-399: `obligation.claim` names the code "of the table above" for
  every failure, but a mismatch of face value or currency has no code in that
  table. Add one (for example `eep-terms-mismatch`).
- "Fence", "derive step", "catch-up" and "outstanding" are used without a
  definition in [Terms](#terms). The agreements document defines the first two.
  Link or define them (the writing rules ask for this).
- change.md:1009 says the first task is "Owner review", which is not a build
  task. It is fine, but it should be marked as the E0 slice of the task file.

**Fix:** Edit the lines.

**Status:** Fixed (f72b2a8e), with a change to one point. The split row count is 20. `eep-terms-mismatch` is added. Fence, derive step, catch-up and outstanding are defined in Terms. The first task is marked E0. On the generator: the facts are right, but `Kem::derive_keypair` does not help, because `single_shot_seal_with_rng` takes a random source, not a key pair, and the only API that takes an ephemeral key is `hpke`'s `danger` module, which says its API is unstable. (Corrected in round 2, P36: the `danger` module takes no ephemeral key either; no public `hpke` API does. The conclusion stays.) So E2 writes a small `rand_core` 0.10 generator that expands the seed with HKDF-SHA256, with no new crate.

## Fix session round 1 — Claude Opus 5.5 (Claude Code), 2026-11-26

Fixes are in `f72b2a8e` on `claude/m08e-obligations-change-doc-62dcfb`.
All 24 findings are handled. Every finding was checked against the code
first (P1 `check_sender`, P3 `proxy.wit` and the router's
`DepositContext`, P13 the `revoked_keys` readers, P19 `PaymentModel`,
P24 `hpke` 0.15.1); all are correct.

Changed from the proposal, with the reason in the item's status: P3
(the `out:` rows stay for the gap before `enqueue`), P7 (a narrower
cancel rule than any of the three options), P10 (an error reply reaches
only a direct caller), P15 (both the table row and the Hub wording),
P16 (the export start time lives in `obligation`'s own section), P20
(the reserved endorsement is on the request), P23 (E3 builds the whole
wire table) and P24 (no `derive_keypair`; a small HKDF generator).

Owner decisions: open question 3 (P21, with the reviewer's signed "not
received" half as option C), open question 16 (P11, may an issuer
refuse a split), open question 17 (P12 point 1, strength of an
acknowledgement for a third party) and open question 18 (P7, the cancel
rule). The design text applies each recommendation, marked "pending
open question N".

New OB rows: OB-37 to OB-51, appended; no id was renumbered. Open
question 1 has a new reason, and questions 7 and 13 have more text.


## Doc review round 2 (fixes check) — Claude Sonnet 5.5 (Claude Code), 2026-11-26

Checked `change.md` at `f2f3ffde` on `claude/m08e-obligations-change-doc-62dcfb`
(fixes in `f72b2a8e`, statuses in `f2f3ffde`). The round 1 text of this file
on that branch matches what I wrote (`782ace05` was cherry-picked as
`4478aa4b` with no change). This round checks the fixes of P1 to P24, and
reads the text the fixes added. Code facts are from the same checkout and
from the cached `hpke-0.15.1` and `rand_core-0.10.1` source.

**Summary:** P1 to P24 are all handled. 20 are confirmed fixed in the text.
4 depend on an owner decision that the text records and applies as a
recommendation: P7 (question 18), P11 (question 16), P12 point 1 (question
17) and P21 question 3. None of the 24 is wrong. The new text creates 13 new
findings, P25 to P37: 0 blocker, 4 major, 9 minor.

**Descriptions of the open questions.** Question 3 is described correctly:
A is the draft, B is the overview's VX-17 wording, C is A plus the signed
"not received" half. The agreements document's question 15 does exist and
the owner placed it with M08G on 2026-11-20 (agreements change.md:1184-1188).
Question 16 is described correctly: the overview says only that the issuer
signs every split. Question 17 is described correctly, with one limit in
P32. Question 18 describes the race correctly, but its recommended rule is
wider than the race (P30).

What was checked and found correct:

- **P1.** Fixed. Step 4 now runs `verify_message` (`check_signature` and
  `check_sender` with the stored time as "now", change.md:601). OB-6 has one
  proof per refusal reason. Step 6 adds the 24-hour rule. See P29 for one
  sentence in "What it does not show".
- **P2.** Fixed. Nothing releases an unconfirmed claim at the quote's expiry
  (change.md:445-447). The drive waits 30 days. `claim-confirm` on a freed
  round takes a new round or answers `eep-claimed`. OB-39 covers both. See P25
  and P31 for gaps the fix opens.
- **P3.** Fixed, and the route is sound. Keeping `out:`/`sent:` only for the
  gap before `enqueue` is right. The WIT says the guest never sees the result
  ([proxy.wit:124-129](../../../../crates/wit_interfaces/wit/proxy/proxy.wit#L124)),
  so a status read keyed by the idempotency key is the smallest change. The
  router keeps one proof per deposit, which fits `seal` making a new deposit
  id each time. Copying the proof into `obligation`'s own data puts it in the
  backup. Stating that E2 is the largest part is honest. See P34 for the task
  mapping.
- **P4.** Fixed. `key:<agreement>:<commitment_id>:<generation>`
  (change.md:439, 723) copies the scheduling `Key` row. OB-20 has both cases.
- **P5.** Fixed. `EepIssueTerms` holds `expires_in_secs`, version 1 computes
  `expires_at_secs` from its own signing time, and the "matches" rule compares
  the life (change.md:309, 321, 344). OB-37 and OB-14 test it.
- **P6.** Fixed. The `eep-return` track end has a 7-day resolution window, and
  its `tick` moves it only when no version is held (change.md:364-365). OB-21
  checks the commitment's state. See P35 for one unclear sentence.
- **P7.** The race is closed, pending question 18. The drafter's route is
  sound for the case it names, and the reasons given against my three options
  are right. See P30: the rule is wider than the race.
- **P8.** Fixed. `resolve(G)` is a function with an order (change.md:453-471).
  Mixes decide by the worst case, and an unissued change EEP counts as "not
  claimed". OB-21 tests every combination of up to three commitments.
- **P9.** Fixed. The issuer's own tick resolves a confirmed claim when the
  writer's snapshots stop (change.md:475, OB-40), and the states table lists
  it as a way out (:766).
- **P10.** Fixed, and the change from my proposal is right. "Answer in the
  call's own reply" reaches only a direct caller. On the mailbox path nothing
  comes back, and that is acceptable for a caller that is not a holder. The
  node-wide limit of 100 an hour is the right shape: a fresh caller DID costs
  nothing, so only a limit that ignores the caller bounds the work. It counts
  only the junk calls, so it cannot block a real holder's known EEP. See P26
  and P34 for gaps.
- **P11.** Recorded as question 16 and applied as a recommendation, with a
  deviation row and OB-44. See P33 for the limit values.
- **P12.** Point 2 is fixed: the new "Countersigns for every record"
  subsection covers declines and, through deposit proofs, demands and
  requests. Point 1 is stated plainly (change.md:682) and is question 17. See
  P28 and P32.
- **P13.** Fixed. Every reader and writer of `revoked_keys` is listed in a
  table (change.md:647-658), both key forms are kept, and `check_key(master,
  key)` over a map by master closes the wrong-anchor hole (:666, OB-50). I
  checked the list against `auth/service.rs:292`, `mailbox/caller.rs:105`,
  `revocation.rs:25`, `client.rs:553` and `supervisor.wit:243`. Nothing is
  missing.
- **P14.** Fixed (change.md:345, OB-14).
- **P15.** Fixed, and doing both the table row and the Hub wording is right
  (change.md:620, 624). `DepositProof` has `eep_version` and `sender`, step 7
  checks the `delivered` receipt, and the deposit's time-to-live is set.
- **P16.** Fixed, and the route is sound. The signed `Bundle` manifest holds
  one digest per section (`SectionDigest`,
  [backup.rs:54](../../../../crates/roym_core/src/backup.rs#L54)), so a time
  written inside `obligation`'s own section is covered by the person's
  signature. Rules 3 and 4 are as proposed (change.md:688-696). The lost
  claim rows are named (:704).
- **P17.** Fixed (48 KiB per call, OB-48). See P37.
- **P18.** Fixed (change.md:903).
- **P19.** Fixed (change.md:418, OB-19).
- **P20.** Fixed, and the route is sound. The reserved `endorsement` is on the
  request, and the old holder signs the request, so the field is its consent.
  `transferred` is reserved and refused with `cause-not-supported`
  (change.md:272-275, 860-868). The "Left for M08H" table matches. The
  one-line change to `M08E-eeps/task.md` stays correct.
- **P21.** Question 1 and question 13 are fixed. Question 3 is rewritten as
  options A, B and C, with the recommendation moved to M08G's decision.
- **P22.** Fixed. All seven rows are in `## Deviations` (change.md:1186-1202),
  and the close-out names the agreements document.
- **P23.** Fixed, and the route is sound. Building the whole wire table in E3
  is better than moving OB-31. See P34 for two follow-ups. All 51 OB rows
  appear in exactly one slice of the task list.
- **P24.** Fixed. The row count is 20 (change.md:741), `eep-terms-mismatch` is
  added, and the terms are defined. **The pushback is right in its result.**
  `Kem::derive_keypair` gives a recipient key pair from key material, not an
  ephemeral key. The public `Kem` trait has only `encap` and `encap_with_rng`
  (`hpke-0.15.1/src/kem.rs:133-155`). `encap_with_eph` and `encap_det` exist
  only inside `cfg(test)` and `kat` code (`kat_tests.rs:29-37`). So a seeded
  random source is the only public route, and it fits
  `single_shot_seal_with_rng(.., &mut impl CryptoRng)` (`single_shot.rs:138-144`)
  from `rand_core` 0.10. `hkdf` 0.12 and `sha2` 0.10 are already direct
  dependencies of `mailbox_format`. One detail is wrong: see P36.

### P25 — major — A confirmed claim cannot be released when the deal ends `not-formed`

**Where:** change.md:437 (`claim-release`), 739 (Release row), 443; agreements
change.md:750 ("Release after confirm").

**Problem:** The agreements document says that in the writer's decision step 4
a failed confirm releases "every other reservation of this agreement, also the
ones already confirmed", and that `release` "must free a reservation in any
state, confirmed too". The new text says `claim-release` frees a claim "only
while … the claim is not confirmed", with one exception: `cancelled: true`,
which `transaction` sends for an `eep-return` that "ended `cancelled`"
(change.md:739). A `not-formed` deal has no commitments, so that flag is never
set. Take a writer that is the issuer, with an `eep-return` and a slot
reservation. The claim is taken and confirmed. The slot confirm is refused
(EX-22). The writer stores a `not-formed` snapshot and releases every
reservation, but `claim-release` refuses the confirmed claim. The holder's
version stays `claimed` for ever: nothing resolves it, because no agreement
formed and no `eep-return` exists. A later agreement that returns the same
version gets `eep-claimed`.

**Fix:** Let `claim-release` free a confirmed claim when `transaction` says the
deal ended `not-formed` or `expired` (for example one flag `deal_ended: true`),
and keep the rule "no `ver:<id>:<v+1>`". Add a service test: confirm a claim,
fail the next confirm, check that the version is free and can be claimed again.

**Status:** Fixed (2e402ac1), with a change to the proposal: one input `why` (`deal-ended` or `commitment-cancelled`) replaces round 1's `cancelled: true`, instead of adding a second flag. `deal-ended` frees a confirmed round, as the agreements document's "release after confirm" requires; the rule "no `ver:<id>:<v+1>`" stays, and the drive's safety net still frees only an unconfirmed round. New OB-52 (confirm, fail the next confirm, claim again).

### P26 — major — Catch-up and `not-holder` need a version that a demand and a split request do not carry

**Where:** change.md:396-402 (automatic checks), 690 (catch-up: "in a demand, a
split request or an acknowledgement"), 913 (`obligation.demand` input
`{ demand }`), 823-835 (`EepDemandPayload`), 860-866 (split request).

**Problem:** Catch-up rule 1 needs the version w "verifies, with `issuer_did`
equal to this node's person". An acknowledgement call carries `versions:
[envelope]`, so it can show w. A demand carries only `eep_record`, a record id,
and the `obligation.demand` input is `{ demand }`. So a demand cannot show w. The
text for `not-holder` says "by the version the demand carries" (change.md:401),
which also has nothing to read. There is a second problem with the order.
`eep-unknown` is the first check and answers "error only, no record". After a
known restore, an EEP made after the backup is wholly unknown, and that is the
common loss. The holder's demand gets an error, and catch-up never runs.
Catch-up is defined only for a version "newer than the issuer's latest stored
version of that EEP", which is empty here. So a holder cannot redeem an EEP that
the restored issuer lost. This was already so in round 1; the new first-two-checks
rule makes it final.

**Fix:** Make the input `{ demand, version }` and `{ request, version }`, where
`version` is the latest issuer-signed version the holder holds. Run the checks in
this order: (1) verify the demand and the version together; (2) with a known
restore, an unknown EEP or a newer version goes to catch-up first; (3) only
then `eep-unknown` and `not-holder` as error only. Say that catch-up also
accepts the case where the issuer has no version of that EEP. Add OB-29 cases:
an EEP created after the backup, shown in a demand.

**Status:** Fixed (2e402ac1). Agreed, as proposed. `obligation.demand` takes `{ demand, version }` and `obligation.split` takes `{ request, version }`. The node verifies the demand and the version together, runs catch-up first when it has a known restore, and only then `eep-unknown` and `not-holder`. Catch-up also accepts an EEP with no record at all. New OB-53.

### P27 — major — The issuer's service cannot see the time the mailbox stored a demand

**Where:** change.md:405 (`eep-expired`: "'Reached' is the `stored` time when the
demand came from the issuer's mailbox"), 407 (`demand-late`), 410, OB-11.

**Problem:** A guest sees only `invocation.caller()`, which answers `internal`,
`verified(did)` or `anonymous`
([invocation.wit:14-33](../../../../crates/wit_interfaces/wit/invocation/invocation.wit#L14)).
Nothing tells it that the call came from a mailbox, or when the mailbox stored
it. The mailbox change (`D-C6-6`) gives the caller no time either. So
`obligation.demand` cannot compute "reached" for a mailbox demand. It can only
use the time of the call, which is the pull time. Then a demand that was
stored on day 6 of a 7-day EEP and collected on day 8 is declined with
`eep-expired`, against the rule that it "must still be answered in its notice"
(change.md:410). The same gap breaks `demand-late`.

**Fix:** Pick one: (a) add the stored time (and a flag "from a mailbox") to the
caller context, as one more E2 host feature, covered by a test; or (b) say that
`obligation` uses the demand's own signed `issued_at_secs`, which step 6 of the
deposit check already ties to within 24 hours of the stored time, and say that
a demand is "reached" at `issued_at_secs` for both codes. Option (b) needs no
host feature, but a holder could back-date by up to 24 hours. State that limit.
Add an OB-11 case with a controllable clock.

**Status:** Fixed (2e402ac1) with option (a), and a pushback on option (b). The code supports (a): the mailbox's `claim` answer already gives the puller `deposited_at_secs` for each message (`ClaimAnswer` in `crates/mailbox/src/store/claim.rs`), so E2 only passes it on as `from-mailbox: option<stored-at-secs>` in the delivery caller context. Option (b) does not hold as stated: on the issuer's node the only bound on a demand's signed `issued_at_secs` is the open check's time-to-live plus skew (8 days), not 24 hours. The 24-hour rule is in the third-party proof check, which the issuer does not run. A deviation from `D-C6-6` is added. New OB-54. Round 3 correction (f9f65008): the type is `ClaimedMessage` in `crates/core/src/mailbox.rs`, not `ClaimAnswer`. And the bound on a demand's own time is its `answer_by_secs` (about 7 days), not the open check's 8 days, which bounds the sealed message's send time. The conclusion (use the mailbox's time) stands; the text is corrected.

### P28 — major — The deposit proof check is written only for a demand, but it is also used as the countersign of every other record

**Where:** change.md:677-678 (countersign table), 596-604 (the check), 583-584
(`eep_version`).

**Problem:** To fix P12 point 2, the draft says a deposit proof is the
countersign of "any record the signer sent … with deposit first: demands,
split and holdings requests, declines, acknowledgements". The check has steps
that only fit a demand:

- Step 1 needs `P.destination` to equal the `issuer_address` of an issuer-signed
  version. A decline is sent to the holder, and an acknowledgement goes to the
  issuer. Only the second fits. A decline's destination is the holder's
  obligation address (`reply_to`), which no version names.
- Step 5 needs the method `obligation.demand` with a byte-identical `demand`.
  A decline travels in `obligation.deliver { records }`, and an
  acknowledgement in `obligation.acknowledge { acknowledgements }`.
- Step 6 needs `D.answer_by_secs`, which only a demand has.

So the countersign by deposit proof, and the "third-party time" of question 17,
cannot be checked as written for four of the five record types.

**Fix:** State a general check: "P proves that a mailbox stored a message that
carries record R, sent to destination X at time T". X is the issuer's address
for demands, split requests, holdings requests and acknowledgements, and the
`reply_to` of the answered request for declines and delivered versions. The method
is the one that record type uses (a small table). The 24-hour rule applies to
every record that has a signed time. Keep steps 1 to 4 and 7 as they are. Move
step 5 and 6 into the per-type table. Add one valid and one changed proof per
record type to OB-6 or OB-49.

**Status:** Fixed (2e402ac1). Agreed. The check is now written for any record R: steps 1 to 4 and 7 stay, step 1 binds the destination through `P.binding` (renamed from `eep_version`), and steps 5 and 6 read R's row of a new table that gives, for each of the five record types, its signer, its destination and what binds it, and its method and place. For a decline or a delivered version, the destination is the `reply_to` of the holder-signed request it answers. The 24-hour rule applies to every type, and `answer_by_secs` only to a demand. New OB-55.

### P29 — minor — The proof check fails every earlier proof after a routine instance revocation

**Where:** change.md:608 ("a revoked sender instance makes the proof fail"),
655 (`revoke-instance` writes `retired`, `effective_from_secs` = now).

**Problem:** The text says a verifier checks the sender master's anchor and a
revoked sender instance "makes the proof fail". It uses no time. But the new
rule says a `retired` key is valid for records signed before `effective_from_secs`,
and `revoke-instance` now writes exactly that. After any redeploy that revokes
the holder's `obligation` instance, every earlier proof (and so every "no
answer" evidence) would fail. That throws away the evidence this design builds.

**Fix:** Use the same rule as for records: the sender's instance key is valid
for a message whose send time is before `effective_from_secs` when the reason
is `retired`. For `compromised`, the stored receipt itself is the third-party
time and is enough when it is before `effective_from_secs`. Add a case to OB-6.

**Status:** Fixed (2e402ac1). Agreed. The sender instance key uses the same time rule as records: `retired` is valid for a send time before `effective_from_secs`, and `compromised` only when the third-party `stored` time is before it. A routine `revoke-instance` writes `retired`, so it keeps earlier proofs. New OB-56.

### P30 — minor — The cancel rule of question 18 is wider than the race it fixes

**Where:** change.md:477, 945, 1066, 1151-1153, OB-41, deviation row at :1199.

**Problem:** The race exists only when the issuer is the counterparty and gives
nothing else, so that it closes at the first snapshot (change.md:477, 1151:
"gives nothing else"). With another commitment in `G`, the issuer does not
close until `G` is final, so a cancel before that releases the claim and is safe.
But the rule, the question's recommendation, OB-41 and the deviation row all
refuse `agreement.cancel` whenever the counterparty receives an `eep-return`.
That includes the common redemption in which the holder writes the quote ("I
give ₹300 EEP, you give 2 kg of wheat") and the issuer accepts. The holder can
never cancel, even while the issuer has done nothing. The rejected-options line
(:1066) says the other options "took away 'cannot deliver'", and this rule does
the same for the holder in this case.

**Fix:** Narrow the rule to: refuse `agreement.cancel` with
`eep-return-to-counterparty` only when the counterparty receives an
`eep-return` and the issuer gives no other commitment (`G` is empty). Change
the rule text, OB-41 (add a case with a non-empty `G` that cancels) and the
deviation row. Question 18 is the owner's to decide, so the owner should see
the narrower rule as an option.

**Status:** Fixed (2e402ac1). The recommendation is narrowed, and it is still the owner's call (open question 18). The rule applies only when the counterparty receives an `eep-return` and gives no other commitment. Reason it is safe: a counterparty issuer that gives something else closes only after the writer's snapshot shows that commitment final, and by then cancel is already refused (`commitment-moved`). Question 18 now shows both the narrow rule (recommended) and round 1's wide rule as options. OB-41 has both cases, and the deviation row is narrowed.

### P31 — minor — A `failed` `eep-return` signed by the issuer reads as the holder's failure

**Where:** change.md:331, 339, 447, 1195; agreements document (VX-12: the
snapshot names every failed commitment, and the agreement names its giver).

**Problem:** When a late countersign finds the version taken by another claim,
the issuer signs a `commitment-failure` and the `eep-return` ends `failed`. The
giver of an `eep-return` is the holder. Reputation (M08G) reads a failed
commitment as the giver's. Here the cause is on the issuer's side: its own
drive released the claim, or it lost a row. A holder who did nothing wrong gets
a failed commitment in its evidence.

**Fix:** Say that the evidence of this failure is attributed by the signer: the
failure record is signed by the receiver, and M08G must not count it against the
giver. Or give the end its own name (for example `claim-lost`) so a reader can
tell it from the giver's `failed`. Add a sentence to "Left for M08H" or to the
M08G task that readers attribute by signer.

**Status:** Fixed (2e402ac1) with the first option. A failure is attributed to the signer of its `commitment-failure`, not to the commitment's giver, and reason `eep-claimed` is only ever signed by the receiver. I did not add a `claim-lost` state: it would need its own rows in every transition table and its own `other-failed` behaviour, while the signer and the reason already tell the cases apart. The rule is a deviation from VX-12's reading, and the close-out passes it to the M08G reputation change document.

### P32 — minor — `revoke_key` defaults to `effective_from = now` for `compromised`

**Where:** change.md:654 (the `revoke_key` row), 682, 1147 (question 17's
reason).

**Problem:** For a `compromised` key, `effective_from_secs` is the earliest time
that records of the key can no longer be trusted without a countersign. Setting
it to "now" by default means that everything signed from the real compromise
until today still counts if it has a countersign. Question 17's reason says the
mailbox `stored` time is "the only time in the design that the forger cannot
write". That is true, but the forger can deposit a forged record at a mailbox
at any time between the compromise and `effective_from`. A default of "now"
makes that window as long as possible.

**Fix:** Make the time a required argument when the reason is `compromised` (no
default). Say in the Hub text and in `roymctl` help that the person should give
the earliest time it suspects. Add one sentence to question 17's reason that
the protection is only as good as `effective_from_secs`.

**Status:** Fixed (2e402ac1). Agreed. For `compromised`, `effective_from_secs` is required with no default, and the Hub and `roymctl` ask for the earliest suspected time; `retired` defaults to now. Open question 17's reason and the countersign section now say the protection is only as good as `effective_from_secs`. New OB-61.

### P33 — minor — The split limits are not defined per currency, and OB-25 now disagrees

**Where:** change.md:513-514 (limits), 136 (OB-25), 161 (OB-44), 1142.

**Problem:** "At least 100 minor units (for example ₹1)" is one number for
every currency. A currency has 0, 2 or 3 minor digits
(`money::currency_minor_exponent`), so 100 minor units is 100 yen, ₹1 or 0.1
dinar. The words "or the whole face value when that is less" are unclear: a
part cannot be the whole value. OB-25 still says "each at least 1 minor unit",
while OB-44 declines under 100. A test cannot satisfy both.

**Fix:** Define the smallest part as `10^max(exponent-2, 0)` minor units (one
major unit of a 2-digit currency) or as a table per currency, and say what
happens when the face value is less than twice the smallest part (the split is
refused with `split-limit`). Change OB-25 to "each at least the smallest part
of OB-44", or say OB-25 is the validator and OB-44 is the policy.

**Status:** Fixed (2e402ac1), with a change to the proposal: the formula `10^max(exponent-2, 0)` gives 1 paisa for INR (exponent 2), which is not the "one major unit" the proposal means. The smallest part is one major unit of the currency, `10^exponent` minor units (₹1, ¥1, 1 dinar), and a face value under twice that cannot be split. OB-25 says it holds the validator's rules, and OB-44 holds the policy limits with one test per exponent (0, 2, 3). Open question 16 uses the same words.

### P34 — minor — Task mapping and test rows

**Where:** change.md:1159-1161 (E2, E3, E4), 159 (OB-42), 165 (OB-48), 797
(demand states).

**Problem:**

- OB-51 is in E2, but it tests `obligation` "with a fake proxy", and the service
  is built in E3. E2 can test the host function only.
- E3 builds the wire table with all five entries but only the `acknowledge` and
  `deliver` handlers. The text does not say what `demand`, `split` and
  `holdings` answer until E4, so OB-31 is not fully testable in E3.
- OB-42 says "sign nothing and store nothing" and also "counts such calls".
  A counter is a stored row. Say "no record and no row except one counter per
  hour".
- OB-48 holds three separate rules (the size, 60 an hour per caller, one
  holdings answer an hour). Split it.
- The demand diagram (change.md:776-783) does not show the new `call-failed`
  state, although the table does.
- The 100-an-hour limit counts only unknown-EEP and non-holder calls. A real
  holder of a closed or claimed EEP can still make the node sign a decline for
  each demand (up to 60 calls of 16 records an hour, per caller, and callers
  are free). Say whether a per-holder limit applies.

**Fix:** Move OB-51's service test to E3 (keep a unit test of the host function
in E2). Add "an unbuilt method answers `not-built` until E4". Edit OB-42,
OB-48 and the diagram. Add the per-holder limit or say why it is not needed.

**Status:** Fixed (2e402ac1). Agreed on all six. OB-51 is split: a host-function test in E2 and the service test in E3. Until E4, `demand`, `split` and `holdings` answer `not-built` (new OB-60). OB-42 says "no record and no row, except one counter per hour". OB-48 is split into OB-48 (size), OB-57 (60 an hour per caller) and OB-58 (one holdings answer an hour). The diagram shows `call-failed`. For the per-holder limit: a coded decline is signed once per (EEP, version, code), and a later demand gets the stored one again (new OB-59). So a holder of a closed or claimed EEP cannot make the node sign without limit, and no separate per-holder limit is needed. Round 3 (f9f65008): the sixth point's answer (one stored decline per code) was unsound, see P39. It is replaced by one signed decline per demand, with a cap of 5 per (EEP, version) per day.

### P35 — minor — "Track end plus 7 days" is used for two different things

**Where:** change.md:364, 475, 766, 1198, OB-40.

**Problem:** Line 364 says the `eep-return`'s own track end already includes a
7-day resolution window. Line 475 says the issuer's own tick runs "at the
`eep-return`'s own track end plus 7 days". A reader cannot tell whether this is
7 days or 14 days after the other commitments end. The states table (:766) and
OB-40 repeat the phrase.

**Fix:** Name two constants, for example `RESOLUTION_WINDOW_SECS` (7 days, in the
track end) and `ISSUER_TICK_GRACE_SECS` (the extra time before the issuer
resolves alone), and use the names in the text and in OB-40.

**Status:** Fixed (2e402ac1). Agreed. Two names: `RESOLUTION_WINDOW_SECS` (7 days, inside the `eep-return` track end) and `ISSUER_TICK_GRACE_SECS` (7 more days before the issuer resolves alone). The text, OB-40 and the states table use them, and the issuer's own tick says the track end already holds the resolution window. Round 3 (f9f65008): the two leftovers (the states table and the round 1 deviation row) now use the constant names, see P40.

### P36 — minor — E2 details for the seeded generator

**Where:** change.md:576, 1159; the status of P24 above.

**Problem:** The fix status says "the only API that takes an ephemeral key is
`hpke`'s `danger` module". That is not right. `danger::streaming_enc` builds
encryption contexts from a given AEAD key and nonce
(`hpke-0.15.1/src/danger.rs`). It takes no ephemeral key. No public `hpke` API
does (see P24 above). The conclusion stays the same. The E2 task also needs:

- `mailbox_format` gains a direct dependency on `rand_core` 0.10 (it is already
  in `Cargo.lock`, so "no new crate" is right, but it is a new line in the
  crate's `Cargo.toml`, with `default-features = false` and the major.minor
  rule of `check-dep-versions`).
- The generator implements `TryRng<Error = Infallible>` and `TryCryptoRng`. The
  `Rng` and `CryptoRng` traits that `hpke` asks for follow from blanket
  implementations (`rand_core-0.10.1/src/lib.rs:49-97`, `:249`).
- A known-answer test: the same seed, key and plaintext give the same sealed
  bytes, and a different seed gives different bytes that open to the same
  plaintext.

**Fix:** Correct the sentence and add the three items to the E2 task.

**Status:** Fixed (2e402ac1). Agreed: my round 1 sentence about the `danger` module was wrong, and P24's status now says so. The doc now says no public `hpke` API takes an ephemeral key (`Kem` has only `encap` and `encap_with_rng`; deterministic encapsulation is test code only), and the E2 task lists the three items: a direct `rand_core = "0.10"` dependency of `mailbox_format` with `default-features = false`, a generator that implements `TryRng<Error = Infallible>` and `TryCryptoRng`, and the known-answer test.

### P37 — minor — The 48 KiB limit must be measured on the escaped request

**Where:** change.md:905, 903, OB-48.

**Problem:** A Roym request reaches the mailbox as the one string argument of
`invoke` (the wake manifest shows the shape: `[ "{\"method\":…}" ]`). The
envelopes inside it are JSON with many quotes, and each quote is escaped when
the request is a string value. That adds about 10 to 20 percent. 48 KiB of
request becomes about 55 to 58 KiB of message, plus the sender certificate, the
signature and the HPKE overhead of 48 bytes. This is under 64 KiB but with
little room.

**Fix:** State that the 48 KiB is the size of the request string as sent, not of
the unescaped records, and let E3 measure the real margin (the draft already
plans to measure one `eep` version).

**Status:** Fixed (2e402ac1). Agreed. The 48 KiB is the size of the request string as sent (the escaped argument of `invoke`), and the text says what the escaping and the sealed message add. E3 measures the margin. OB-48 says the same.

## Fix session round 2 — Claude Opus 5.5 (Claude Code), 2026-11-26

Fixes are in `2e402ac1` on `claude/m08e-obligations-change-doc-62dcfb`,
which fast-forwarded to the round 2 review commit `de9bd0f4` first.
All 13 findings, P25 to P37, are fixed. Each was checked against the
code; P27 led to a new fact: the mailbox's `claim` answer already holds
`deposited_at_secs`.

Fixed differently from the proposal, with the reason in the item's
status: P25 (one `why` input instead of a second flag), P27 (option (a);
option (b)'s 24-hour bound does not hold on the issuer's node, where only
the 8-day open check bounds a demand's time), P31 (attribution by
signer, no `claim-lost` state) and P33 (the proposed formula gives 1
paisa for INR; the smallest part is one major unit, `10^exponent` minor
units). P36 also corrects my round 1 status of P24.

Open questions: question 18 is narrowed (P30): the recommended rule now
refuses cancel only when the counterparty receives an `eep-return` and
gives nothing else, and round 1's wider rule is shown as the other
option. Question 16 uses the per-currency smallest part (P33). Question
17's reason says the protection is only as good as
`effective_from_secs` (P32). No new open question. Questions 3, 16, 17
and 18 still wait for the owner.

New OB rows: OB-52 to OB-61, appended; no id was renumbered. New
deviations are under "Added in review round 2".


## Doc review round 3 (fixes check) — Claude Sonnet 5.5 (Claude Code), 2026-11-26

Checked `change.md` at `1d2d9768` on `claude/m08e-obligations-change-doc-62dcfb`
(fixes in `2e402ac1`, statuses in `1d2d9768`). My branch was fast-forwarded to
that tip first and already held my round 2 commit `de9bd0f4`. This round checks
P25 to P37 and reads the text the fixes added: OB-52 to OB-61, the new tables,
and the round 2 deviations.

**Summary:** 11 of P25 to P37 are confirmed fixed. P34 is fixed except for
one point whose replacement is unsound (P39), and P35 is half applied (P40).
The new text also reopens part of P2 (P38). The new findings are P38 to P41: 0
blocker, 2 major, 2 minor. So this file has open items, and it cannot yet say
that every finding is closed.

**Open question 18.** It now matches the race. The race needs three things:
the counterparty is the issuer, it receives an `eep-return`, and its `G` is
empty, so it closes at the first snapshot. The new rule and the new option
text say exactly that. They also give the reason that the other case is safe
(a non-empty `G` closes only after a commitment has moved, and then cancel is
refused with `commitment-moved`). I checked this against the states: it holds.
The wide rule is kept as the second option, described correctly. One wording
detail is in P40. I did not decide questions 3, 16, 17 or 18.

What was checked and found correct:

- **P25.** Confirmed fixed. One input `why` (`deal-ended` or
  `commitment-cancelled`) replaces the second flag, which is simpler than my
  proposal. `deal-ended` frees a confirmed round, the rule "no `ver:<id>:<v+1>`"
  stays, and the drive frees only an unconfirmed round (change.md:460, the
  Release row). OB-52 is the test I asked for. See P38 for how this
  interacts with `expired`.
- **P26.** Confirmed fixed. `obligation.demand { demand, version }` and
  `obligation.split { request, version }` (change.md, Methods table). The order
  is: verify both together, catch-up with a known restore, then `eep-unknown`
  and `not-holder` as error only. Catch-up accepts an EEP with no record at all.
  OB-53 tests both.
- **P27.** Confirmed fixed with option (a), and the code supports it. The
  mailbox's `claim` answer carries `deposited_at_secs` for each message
  (`ClaimedMessage`, [mailbox.rs:198](../../../../crates/core/src/mailbox.rs#L198);
  built at `store/claim.rs:125`), so the puller has the time. E2 only has to pass
  it on. The cited type is `ClaimedMessage` in `core/mailbox.rs`, not
  `ClaimAnswer` in `claim.rs`; this is a small slip. OB-54 tests it. **The
  pushback on option (b) is right in its result, but the reason is wrong.** See
  the next bullet and P40.
- **The 8-day claim.** The draft says the only bound on a demand's signed
  `issued_at_secs` on the issuer's node is "the open check's time-to-live plus
  the skew (8 days)". In the code, `check_sender` bounds the sealed message's
  `sent_at_secs`, not the demand record's `issued_at_secs`
  ([message.rs:440-472](../../../../crates/mailbox_format/src/message.rs#L440)).
  A holder can sign a demand and send it days later. The real bound is the
  demand's own `answer_by_secs = issued_at_secs + 7 days`: after that, the node
  answers `demand-late`. So a holder can back-date a demand by up to about 7
  days, which would cut the issuer's notice to almost nothing if the demand's
  time were used as "reached". The number is 7 days, not 8, and it comes from
  `answer_by_secs`. The conclusion (use the mailbox's time) stands.
- **P28.** Confirmed fixed. The check works for any record R. `P.binding`
  replaces `eep_version`, and a table gives, for each of the five types, its
  signer, its destination and what binds it, its method and its place. OB-55
  has one valid and one changed proof per type.
- **P29.** Confirmed fixed. The sender instance key follows the same time rule
  as records, and a routine `revoke-instance` (`retired`) keeps earlier proofs.
  OB-56 tests both reasons.
- **P30.** Confirmed fixed, and OB-41 has both cases. See P40 for one word.
- **P31.** Confirmed fixed, and I accept the route. Attributing a failure to
  the signer of its `commitment-failure`, with reason `eep-claimed` only ever
  signed by the receiver, needs no new state. A `claim-lost` state would add a
  row to every transition table and its own `other-failed` behaviour, and the
  signer and the reason already tell the two cases apart. The three deviation
  rows (VX-12, the demand carrying a version, and the M08G pass-on) are
  recorded.
- **P32.** Confirmed fixed. `compromised` needs an explicit
  `effective_from_secs`, and `retired` defaults to now. Question 17's reason and
  the countersign paragraph both say the protection is only as good as that
  time. OB-61 tests it.
- **P33.** Confirmed fixed, and **the pushback is right**. `10^max(exponent-2,0)`
  gives 1 paisa for INR (exponent 2), which is not "one major unit". The
  correct value is `10^exponent` minor units: ₹1 = 100 paise, ¥1 = 1, 1 dinar =
  1000 fils (`money::currency_minor_exponent` returns 2, 0 and 3). My formula
  was wrong. A face value under twice that cannot be split. OB-25 now says it
  holds the validator's rules, OB-44 the policy limits, with one test per
  exponent (0, 2, 3).
- **P34.** Five of the six points are confirmed fixed: OB-51 split between E2
  and E3, `not-built` (OB-60), OB-42 reworded, OB-48 split into OB-48, OB-57 and
  OB-58, and the diagram shows `call-failed`. The sixth point, a limit per
  holder, is replaced by "a coded decline is signed once per (EEP, version,
  code)" (OB-59). That replacement is not sound: see P39.
- **P35.** Fixed in the Commitments section and in OB-40 (`RESOLUTION_WINDOW_SECS`
  and `ISSUER_TICK_GRACE_SECS` are used and defined), but two places still say
  "track end plus 7 days": the states table and the round 1 deviation row. See
  P40.
- **P36.** Confirmed fixed. The status of P24 is corrected, and the text now
  says no public `hpke` API takes an ephemeral key (`Kem` has only `encap` and
  `encap_with_rng`; `encap_with_eph` and `encap_det` are test code,
  `hpke-0.15.1/src/kat_tests.rs:29-37`). The E2 task has the direct `rand_core`
  0.10 dependency with `default-features = false`, `TryRng<Error = Infallible>`
  plus `TryCryptoRng`, and the known-answer test.
- **P37.** Confirmed fixed (change.md, Limits). The 48 KiB is the request
  string as sent, and OB-48 says the same.
- **OB-52 to OB-61.** Each is testable as written. OB-57 and OB-58 are now one
  rule each. Every row appears in exactly one slice of the task list.

### P38 — major — A release for `expired` still frees an unconfirmed claim at the quote's expiry

**Where:** change.md:460 (Release: "when `transaction` sees the deal end
`expired`, …"), 462 ("So nothing releases an unconfirmed claim at the quote's
expiry"), the Release row of the write-order table, OB-39; agreements
change.md "Deal and agreement" (`expired` is "computed" by each node's clock).

**Problem:** The text says two opposite things. Line 462 says nothing releases an
unconfirmed claim at the quote's expiry. Line 460 says `transaction` releases it
when it sees the deal end `expired`. In the agreements document, `expired` is
not an event that a node waits for. Each node computes it from its own clock
when `quote_expires_at_secs` passes and it holds no countersigned half signed
before that time. So an issuer that is the counterparty gets `expired` at the
quote's expiry, `transaction` calls `claim-release { why: deal-ended }` at once,
and the unconfirmed claim is freed. That is the case P2 was about. A countersigned
half signed before the expiry may still arrive later (the agreements document
lets it move the deal from `expired` to `agreed`). Now `claim-confirm` takes a
new round if the version is free. If another agreement took it in between, the
formed agreement's `eep-return` ends `failed` by the issuer's own
`commitment-failure`. The holder returned the version in a formed agreement and
it was claimed elsewhere first. That is the "cannot be spent elsewhere" rule of
VX-17 failing for this agreement, and OB-39 does not catch it: the test frees the
claim, then confirm takes it back, and passes.

**Fix:** Pick one rule and write it once:

- For a counterparty that holds an `accepted` row, treat `expired` as a release
  trigger only after the quote's expiry plus the card retry time (30 days), the
  same bound as the drive's safety net. Before that, `expired` frees nothing.
  `not-formed`, `declined` and `cancelled` are certain ends and release at once.
- Or accept the outcome and say so: an `expired` deal releases at expiry, and a
  late countersigned half may then fail the `eep-return` with `eep-claimed`.
  Then change line 462 and the P2 text, and add a test with a competing claim
  between the expiry and the late half.

I recommend the first. It matches what this document already says in line 462.

**Status:** Fixed (f9f65008) with the first option, as recommended. Release at once only on certain ends (`not-formed`, `declined`, `cancelled`, a cancelled `eep-return`). For a claim taken as the counterparty, `expired` releases only after the quote's expiry plus 30 days, the same bound as the drive's safety net. The text adds why this rarely holds the version that long: a writer that files the acceptance after the expiry sends a `not-formed` snapshot with `quote-expired`, which is a certain end. The `why: deal-ended` input, the states table and the round 1 deviation row say the same, and there is a new deviation row against the agreements document's `expired`. New OB-62, with a competing claim between the expiry and the late half.

### P39 — major — A decline signed once per (EEP, version, code) is sent again for a different demand, and it no longer answers that demand

**Where:** change.md:427 (Repeated declines), OB-59 (change.md:177),
`EepDeclinePayload` (change.md:872-873: `answers` names the one demand declined),
the demand states (a decline "was filed").

**Problem:** This replaces my proposal for a limit per holder. A decline carries
`answers: "rec_…"`, the record id of the one demand it declines. OB-59 says a
later demand that hits the same code "gets the stored decline sent again". That
decline's `answers` names the first demand, not this one. The holder's node
matches a decline to a demand by `answers`, so the second demand never reaches
the state `declined`. After 7 days it ends `unanswered`, and the evidence may
read "received, not answered", or "stored, never collected". That counts against
an issuer that did answer. The time can also be wrong: a stored `eep-claimed`
decline may be weeks old, and the claim it states may be gone, so the second
demand would get an old fact. The text also reads as if the issuer never
signs for the same code twice, but a decline for a different holder of the same
EEP and version (after a transfer in M08H) would be skipped as well.

**Fix:** Keep the aim (the node cannot be made to sign without limit) and keep
`answers` true. For example:

- Sign a coded decline for every demand, but allow at most N signed declines
  per (EEP, version) per day (for example 5). Past that, answer in the direct
  reply with an error and sign nothing, and say that an honest holder's evidence
  in that case is its demand and the issuer's error. The 60-call limit per
  caller still applies.
- Or let the decline name more than one demand (`answers` becomes a list). Then a
  stored decline can be extended only by signing again, so this is not cheaper.

Change OB-59 to match, with a test that a second demand gets a decline whose
`answers` names it, and that the N+1th gets an error only.

**Status:** Fixed (f9f65008) with the first option. Every demand that hits a coded decline gets its own signed decline, whose `answers` names it and whose time is the time of this answer. A stored decline is never sent for another demand. At most 5 coded declines are signed per (EEP, version) per day; past that, the direct reply carries an error and nothing is signed. The cap is per (EEP, version), whatever the holder, so it also holds in M08H. OB-59 is rewritten in place with the two tests (a second demand gets a decline naming it; the sixth gets an error only). I kept the OB id, because the row's rule is the same one, corrected; its text marks the change.

### P40 — minor — Wording and leftovers

**Where:** change.md:425 (the stored-time paragraph), 158 (OB-41), 630 (the
table row for `eep-decline` and `eep` versions), 794 (the states table), 1234
(a deviation row).

**Problem:**

- Line 425 gives "8 days" and the open check as the bound on a demand's
  `issued_at_secs`. See the bullet above: the bound is `answer_by_secs`, 7 days.
  Fix the number and the reason.
- OB-41 and the rule say "gives no other commitment". In a two-way set-off the
  counterparty also gives an `eep-return`, so the words read as "the rule does
  not apply". That is the right result (the writer is also an issuer, and its
  own close moves a commitment, so cancel is refused), but the text should say
  "no other commitment than an `eep-return`" and state this case in OB-41.
- The row for an `eep` version sent by `deliver` is subject to the 24-hour rule.
  An old version sent again in answer to a holdings request is older than 24
  hours, so its proof never verifies. This is harmless, because
  acknowledgements are the countersign for versions, but then the row is not
  needed. Remove the row or exempt it.
- The states table (change.md:794) and the deviation row (:1234) still say "track
  end plus 7 days" (P35). Use the two constant names. The states table also lists
  only "the release of a cancelled `eep-return`" as an end of a confirmed claim.
  Add `deal-ended`.

**Fix:** Edit the five places.

**Status:** Fixed (f9f65008). All five places: the stored-time paragraph now gives the bound as `answer_by_secs` (about 7 days), with the reason that the open check bounds the sealed message, not the demand; the rule, OB-41, the open question and the deviation row say "no commitment other than `eep-return` commitments", and OB-41 tests the two-way set-off; the `eep` version is removed from the deposit proof table, because its countersign is the holder's acknowledgement; the states table and the round 1 deviation row use `RESOLUTION_WINDOW_SECS` and `ISSUER_TICK_GRACE_SECS`, and the states table lists both `why` values as ends of a confirmed claim.

### P41 — minor — The stored-time feature needs a named host change and test host

**Where:** change.md:425, E2 task, deviation row for `D-C6-6`.

**Problem:** The text says the delivery caller context "gains `from-mailbox:
option<stored-at-secs>`, which the guest reads with the caller". Today the guest
reads the caller through `caller: func() -> caller-origin`, a variant whose
`verified` arm holds one string
([invocation.wit:14-33](../../../../crates/wit_interfaces/wit/invocation/invocation.wit#L14)).
A new field needs either a new variant arm payload or a new function. It also
touches `AppInvocation::caller` in the dual-build shim, `roym_core::admit::Caller`
and the shared `TestHost`, which the Roym service tests use. The E2 task lists
the feature in one phrase.

**Fix:** Name the WIT shape (for example a new function
`delivery: func() -> option<delivery-info>`, so no existing guest changes), and
add the shim, `admit` and `TestHost` to the E2 task. OB-54's "fake mailbox
caller context" then has a concrete place.

**Status:** Fixed (f9f65008). Agreed, with the proposed shape: a new function `delivery: func() -> option<delivery-info>` in the `invocation` interface, with `delivery-info { mailbox, stored-at-secs }`, so `caller()` and every existing guest stay unchanged. The guest reads it with `roym_core::admit::delivery(host)`. The E2 task lists the WIT, the puller, the dual-build shim's `AppInvocation`, `admit` and a setter on the shared `TestHost`. New OB-63.

## Fix session round 3 — Claude Opus 5.5 (Claude Code), 2026-11-26

Fixes are in `f9f65008` on `claude/m08e-obligations-change-doc-62dcfb`,
which fast-forwarded to the round 3 review commit `271b04e9` first.
P38 to P41 are fixed, and so are the open parts of P34 and P35. Their
statuses (and P27's, for its two slips) have a "Round 3" note. Every
finding P1 to P41 is now closed in the text, except the four that wait
for the owner: P7 (question 18), P11 (question 16), P12 point 1
(question 17) and P21 question 3.

Fixed as proposed: P38 (first option), P39 (first option), P40 and
P41. One choice to note: OB-59 keeps its id and is rewritten in place,
because the row's rule is the same rule, corrected, and its text says
so. All other new rows are appended: OB-62 and OB-63.

Open questions: no new one, and none changed in substance. Question 18's
rule now says "no commitment other than `eep-return` commitments", which
is the same narrow rule in clearer words and names the two-way set-off.


## Doc review round 4 (fixes check) — Claude Sonnet 5.5 (Claude Code), 2026-11-26

Checked `change.md` at `2f5dd5f6` on `claude/m08e-obligations-change-doc-62dcfb`
(fixes in `f9f65008`, statuses in `2f5dd5f6`). My branch was fast-forwarded to
that tip first. This round checks P38 to P41, the open parts of P34 and P35,
and reads only the text that the fixes added.

**Summary:** P38 to P41 are confirmed fixed, and the open parts of P34 and P35
are closed. The new text creates 1 new minor finding, P42. Every finding in this
file is closed, apart from the owner questions (3, 16, 17 and 18) and P42.

What was checked and found correct:

- **P38.** Confirmed fixed. `expired` now releases a claim that this node took
  as the counterparty only when the quote's expiry plus 30 days has passed
  (change.md, Release and `claim-release` rows). `not-formed`, `declined` and
  `cancelled` are named as certain ends and release at once. The text says why:
  each node computes `expired` from its own clock, and a countersigned half
  signed before the expiry can still move the deal to `agreed` (agreements
  change.md, "One way out of an end state"). The sentence "when the writer files
  the acceptance after the expiry, it sends `not-formed` with `quote-expired`"
  matches the agreements document, so in practice a release comes sooner. The
  writer-is-issuer case needs no `expired` rule, because the writer writes the
  `not-formed` snapshot itself. The states table, line 462 and the deviation
  rows now say the same thing. OB-62 tests exactly the case I described: accept,
  move past the expiry, a competing claim, then the late countersign, and the
  competitor gets `eep-claimed`. It is in E4.
- **P39.** Confirmed fixed. Every demand that hits a coded decline gets its own
  signed decline, and its `answers` names that demand and its time is the time
  of this answer. At most 5 coded declines are signed per (EEP, version) per
  day. Past that, the demand gets an error in the direct reply and nothing is
  signed (OB-59). A non-holder cannot use up the 5, because `not-holder` and
  `eep-unknown` are error only. On the mailbox path the error ends as an unsigned
  `rejected(call_failed)`, which the "No answer" table already says is not counted
  against the issuer. So an honest holder is not harmed.
- **P34 and P35.** Closed. OB-59 is the sound version of P34's sixth point. The
  states table and the deviation row now use `RESOLUTION_WINDOW_SECS` and
  `ISSUER_TICK_GRACE_SECS`, and the states table lists `why: deal-ended` and
  `why: commitment-cancelled` as ends of a confirmed claim.
- **P40.** Confirmed fixed, all five. The stored-time paragraph now says the open
  check bounds the sealed message's send time and not the demand's own time, and
  that the real bound is `answer_by_secs` (about 7 days). OB-41 and the rule say
  "no commitment other than `eep-return` commitments", name the two-way set-off,
  and OB-41 has three cases. The `eep` version row is gone from the proof table
  (the countersign table never listed it). The states table and the deviation
  row are updated.
- **P41.** Confirmed fixed. `invocation.delivery()` is a new function with a
  `delivery-info` record (`mailbox`, `stored-at-secs`) that answers `none` for
  every call that did not come from a mailbox. A new function in the same
  interface leaves `caller()` and every existing guest unchanged, which is the
  claim the text makes. The E2 task now names the WIT, the puller, the
  `AppInvocation` shim, `roym_core::admit::delivery` and a `TestHost` setter,
  and OB-63 tests it.
- **OB-59 rewritten in place.** This is acceptable. The rule "ids are never
  renumbered" keeps references stable: a finding, a task or a deviation that
  says "OB-59" must still point at the same rule. The corrected row is the same
  rule (what a repeated demand gets), the id is unchanged, and the row says
  "(Changed in review round 3, P39)". This document is still a draft with no
  built tests, so no test file names OB-59 yet. Earlier rounds did the same for
  OB-6, OB-13, OB-14, OB-19, OB-21, OB-25, OB-40, OB-41, OB-42, OB-44 and OB-48.
  I would add a new id only when a row changes what it tests, for example if
  OB-59 were split in two. The older status lines of P34 in this file still
  describe the first OB-59 ("signed once per (EEP, version, code)"). They stay as
  history, and the round 3 note replaces them.
- **Open question 18.** Still matches the race, now with the clearer words "no
  commitment other than `eep-return` commitments", which includes the two-way
  set-off. I did not decide it.

### P42 — minor — The "per day" window of OB-59 is not defined

**Where:** change.md, "Repeated declines" and OB-59 ("at most 5 coded declines
are signed per (EEP, version) per day"; test: "the sixth in one day gets an
error only"). The same gap is in OB-42, OB-57 and OB-58 ("an hour").

**Problem:** A day or an hour can be a fixed bucket (`floor(now / 86400)`) or a
rolling window (the last 86 400 seconds). The two give different answers at the
edge: a fixed bucket allows 10 declines in two minutes across midnight, and a
rolling window does not. A test cannot be written without the choice, and the
counter row differs (one row per bucket, or a list of times). The counter is a
stored row, which OB-42 already allows ("one counter per hour").

**Fix:** Say once, in the Limits list, that every "per hour" and "per day" limit
in this document uses a fixed bucket of `floor(now / 3600)` or
`floor(now / 86400)` seconds, counted on the receiving node's clock, with one
counter row per key and bucket. Say that the doubled burst at a bucket edge is
accepted. Add the boundary case to OB-59's test.

**Status:** Fixed (08e80be3). Agreed, as proposed. The Limits list says once that every per-hour and per-day limit (OB-42, OB-57, OB-58, OB-59) uses a fixed bucket on the receiving node's clock, with one counter row per key and bucket, and that the doubled burst at a bucket edge is accepted. OB-59's test adds the bucket-edge case.

## Fix session round 4 — Claude Opus 5.5 (Claude Code), 2026-11-26

The fix is in `08e80be3` on `claude/m08e-obligations-change-doc-62dcfb`,
which fast-forwarded to the round 4 review commit `30bf777b` first. P42 is
fixed as proposed. No open question changed. Every finding P1 to P42 is
closed, except the four that wait for the owner (questions 3, 16, 17, 18).


## Doc review round 5 (fixes check) — Claude Sonnet 5.5 (Claude Code), 2026-11-26

Checked `change.md` at `ab5ae3c9` on `claude/m08e-obligations-change-doc-62dcfb`
(fix in `08e80be3`, status in `ab5ae3c9`). My branch was fast-forwarded to that
tip first. This round checks P42 and reads only the changed text.

**Summary:** P42 is confirmed fixed. The changed text creates 1 new minor
finding, P43. So this file is not yet closed.

- **P42.** Confirmed fixed. The Limits list now has one bullet, "How limits
  count" (change.md:948), that says every "per hour" and "per day" limit uses a
  fixed bucket, `floor(now / 3600)` or `floor(now / 86400)`, on the receiving
  node's own clock, with one counter row per key and bucket. It names OB-42,
  OB-57, OB-58 and OB-59, says that a rolling window is not used, and accepts a
  burst of up to twice the limit at a bucket edge. I searched the document for
  every limit with a time unit: OB-42 and the Methods bullet (100 an hour,
  node-wide), OB-57 and its bullet (60 an hour per caller), OB-58 and its bullet
  (one holdings answer per holder per hour), the one-per-hour counter of the
  automatic checks, and OB-59 (5 per day). All of them are covered by that
  bullet, and none is left with another definition.
- **Testability.** OB-42, OB-57, OB-58 and OB-59 can be tested as written. A
  service test has a controllable clock (see "Kinds of test"), so a test can fill
  a bucket and step over its edge. OB-59 now says exactly that: a second demand
  gets a decline naming it, the sixth in one bucket gets an error only, and
  after `floor(now / 86400)` changes the next demand is signed again. OB-57 and
  OB-58 say only "Service", but their window is now defined, so the edge case
  follows from the bullet.

### P43 — minor — The counter rows of the limits are never removed

**Where:** change.md:948 ("one counter row per key and bucket, for example
`count:<key>:<bucket>`"), OB-57.

**Problem:** A fixed bucket needs one row per key and bucket. Nothing says when
an old row goes. For OB-58 and OB-59 the number of keys is bounded by the
holders and the EEPs the node knows. For OB-57 the key is the caller DID, and a
caller DID costs nothing (the reason given for the node-wide limit at
change.md:951). Each new DID that makes one call adds a row, and the 60-call
limit never stops it, because each DID stays under the limit. So the number of
rows grows with the number of distinct callers for as long as the node runs.
The rest of the design says "no row is ever deleted" only for claims and the
event log, which need it. These counters do not.

**Fix:** Say that `obligation.drive` deletes counter rows older than the
current bucket (one bucket back is enough), in a bounded number per run, as
`transaction.drive` bounds its work. Add one case to OB-57: a row from an old
bucket is gone after the next drive.

**Status:** Fixed (0500d51f). Agreed, as proposed, plus one more bound. `obligation.drive` deletes every counter row of a bucket older than the current one, at most 500 rows per run (OB-64, with the old-bucket case). Deleting old buckets alone still lets one hour hold a row per fresh caller DID, so a bucket holds at most 10,000 caller-DID rows, and a new caller in a full bucket gets the rate-limit error and makes no row (OB-65). Counters live in their own collection, `counters`.

## Fix session round 5 — Claude Opus 5.5 (Claude Code), 2026-11-26

The fix is in `0500d51f` on `claude/m08e-obligations-change-doc-62dcfb`, which
fast-forwarded to the round 5 review commit `0d241311` first. P43 is fixed,
with one extra bound (a cap of caller-DID rows per bucket). Every finding
P1 to P43 is closed, except the four that wait for the owner (questions 3,
16, 17, 18). No open question changed.

**The check of every row type.** A new section, "Rows that grow, and their
cleanup", lists every row type with what bounds it or what removes it. What
the check found:

- **Counter rows:** had no cleanup (P43). Now deleted by bucket, and capped
  per bucket for caller DIDs. A holdings request from someone who holds no
  live EEP of this issuer now makes no counter row.
- **`out:`/`sent:` rows:** had no cleanup. Now deleted 1 day after `sent:`
  exists, because the proxy outbox owns the retries.
- **Issuer-side demand rows:** had no cleanup. Now deleted 30 days after a
  final state. The holder keeps the evidence.
- **Coded decline rows:** capped per day, but kept for ever, so they grew
  with time. Now deleted 30 days after the demand's `answer_by_secs`. A
  person's decline is kept.
- **`eep_log`:** append-only, and coded declines would have added up to 5
  entries per EEP version per day for ever. Coded declines, and the demands
  they answer, now make no log entry. This is a deviation from VX-45,
  recorded.
- **Bounded already, kept:** versions and forks (only the issuer's own
  signatures, splits limited); claim, free, confirmed and key rows (only
  agreements this node signs; they are the fence); split `req:` rows (one
  per version); restore rows; the holder's held, ack and demand rows and
  proofs (only accepted records, one open demand per version); limit
  settings; the router's deposit status rows (outbox lifetime plus 14 days).
- **Junk wire calls and junk deliveries:** make no row beyond the counters
  (OB-42, OB-43).

New OB rows: OB-64 and OB-65 (E3), OB-66 (E4). Two deviation rows are
under "Added in review round 5".

## Owner decisions, 2026-11-26

Recorded by Claude Opus 5.5 (Claude Code). The owner's words: "I agree to the 4 recommendations. Plus for 13, the mailbox criteria provides limited benefit anyways. The issuer can later drop the mailbox, mailbox owner could drop the receiver, etc. so I am ok to not enforce mailbox, or maybe just a warning."

**Accepted as recommended.**

- Question 3: option A in M08E. Option C (a signed "not received" half) is decided in M08G, together with the agreements document's open question 15. (P21)
- Question 16: the node splits automatically inside the limits and declines above them with `split-limit`. (P11)
- Question 17: a third party counts an acknowledgement at full strength only with a third-party `stored` time. (P12 point 1)
- Question 18: the narrow cancel rule. (P7)

Each is marked "Decided 2026-11-26, owner" in the change doc, and every "pending open question N" mark is removed from the design text, the OB rows and the deviations.

**Question 13 reversed.** `obligation` may run without a mailbox. What changed in the change doc:

- **Where it runs:** the operator chooses the mailbox at deploy, as for any public service. The deploy no longer fails without one.
- **New section "An issuer or holder with no mailbox":** every call goes direct; a demand waits in the holder's own outbox and arrives only when both nodes are online at the same time; the "no answer" evidence has strength "none" for a third party; acknowledgements have no third-party time, so by question 17 a third party never counts them at full strength; a demand that went direct has no countersign. It also covers a holder with no mailbox, and an issuer that drops its mailbox later.
- **A code fact this found:** the proxy outbox ends a call to a receiver with no mailbox after its attempt budget, about 10 hours (54 attempts, backoff capped at 15 minutes). Without a change, a demand to an issuer offline for longer would be lost. The send path gains a "retry until" `enqueue` option (E2): retry until `answer_by_secs` for a demand, 30 days for other records. A retry past the receiver's dedup window may run twice, which is harmless because every wire method is idempotent.
- **Warning (recommendation):** both places. `roymctl svc deploy` and `app deploy` warn for any public service that lists `mailbox_methods` and has no mailbox (a generic rule). The holder's Hub warns on the issuer's EEPs and on the demand screen, from a new `receiver_mailbox` field of the deposit status read. A warning on the issuer's own wallet, and one before accepting an `eep-issue`, are left out: they need a read of a service record's mailbox that no guest has today. Backlog.
- **"No answer" table and OB-23:** two new levels, "received by the issuer's node, no proof" and "could not reach the issuer" (not counted against the issuer).
- **OB rows:** OB-17 is scoped to an issuer with a mailbox. OB-23 gains the two levels. New OB-67 (deploy warning), OB-68 (retry until `answer_by_secs`), OB-69 (Hub warning), OB-70 (a proof made before the mailbox was dropped still verifies).
- **States:** no new state. A demand to an issuer with no mailbox never reaches `deposited` or `call-failed`.
- **VX-15 does not hold as written.** It holds only for an issuer with a mailbox. This is in `## Deviations` under "Added by the owner's decisions, 2026-11-26", with three more rows (VX-49 evidence, the acknowledgement rule, the outbox attempt budget).
- **New open question 19:** two ways to give back some evidence without a mailbox (a deposit to any mailbox only for a timestamp; a signed "received" receipt in the direct reply). Recommendation: neither in M08E.

**For the owner to look at again.**

- A redemption demand to an issuer with no mailbox works only when both nodes are online at the same time before the 7-day notice ends. Two phones may never overlap. Redemption inside a conversation still works.
- "Stored, never collected" reads the same whether the issuer dropped its mailbox or the mailbox operator dropped the issuer's queue.

## Doc review round 6 — Claude Sonnet 5.5 (Claude Code), 2026-11-26

Checked `change.md` at `82ad0464` on `claude/m08e-obligations-change-doc-62dcfb`
(P43 fix in `0500d51f`, its status in `215bb668`, owner decisions in
`82ad0464`). My branch was fast-forwarded to that tip first. This round checks
P43 and the whole-document check of rows that grow, the four owner decisions
(questions 3, 16, 17, 18), and the reversal of question 13 through the whole
document. I did not re-decide any of them.

**Summary:** P43 is confirmed fixed, with one side effect (P45). The four
decisions are marked and applied. The reversal of question 13 is followed
through most of the document and is correct against the proxy outbox code, but
it leaves 4 new findings, P44 to P47: 0 blocker, 2 major, 2 minor. So this file
has open items.

What was checked and found correct:

- **P43 and the check of every row type.** Confirmed. The new section "Rows that
  grow, and their cleanup" (change.md:807-826) lists every row type with who can
  make it and what bounds or removes it. Counter rows are deleted by bucket
  (OB-64). Old `out:`/`sent:` pairs, issuer-side demand rows and coded decline
  rows are deleted (OB-66). Fence rows (`ver:`, `claim:`, `free:`, `confirmed:`,
  `key:`) are kept on purpose, with the reason that deleting them would reopen
  the race. I went through the doc for a row type that is missing: the router's
  deposit status rows, the restore rows, the holder's rows and limit settings are
  all listed, and the `eep_log` is bounded because coded declines make no entry
  (a deviation from VX-45, recorded under "Added in review round 5"). I found no
  missing row type. See P45 for a side effect of the cap in OB-65.
- **Questions 3, 16, 17 and 18.** Each is marked "Decided 2026-11-26, owner:
  as recommended" (question 3 gives option A for M08E and sends option C to
  M08G with the agreements document's question 15). I searched the whole file
  for "pending": the word is left only as the state name `pending` in the
  deposit status read (change.md:226, 583). The marks "pending open question N"
  are gone from the design text, OB-41 and OB-44, and the deviations. The reason
  lines of questions 17 and 18 still say "so the owner decides", followed by the
  decided line. That reads as history and is fine.
- **The outbox claim, checked against the code.** Correct. The default attempt
  budget is 54 (`default_supervisor_queue_max_attempts`,
  [roles.rs:130](../../../../crates/core/src/config/roles.rs#L130)), the backoff
  starts at 100 ms, doubles, and stops at 900 s (`:137`). The doubling reaches the
  ceiling after 14 steps (about 27 minutes in all), and the other 40 attempts
  wait 15 minutes each, so the total is about 10.5 hours: "about 10 hours" is
  right. A call to a receiver with no mailbox ends then, and only a call to a
  receiver with a mailbox can reach the row age limit of 30 days
  (`MAX_ROW_AGE_MS`,
  [proxy_outbox.rs:55](../../../../crates/router/src/proxy_outbox.rs#L55);
  `settle_pending`,
  [outbox_forwarding.rs:133-138](../../../../crates/router/src/proxy/outbox_forwarding.rs#L133)).
  The receiver's dedup window is the retry window plus one visibility timeout
  (`DedupConfig::derive`,
  [dedup.rs:127-134](../../../../crates/async_queue/src/dedup.rs#L127)), so a
  retry past it can run twice. The doc's answer, that every wire method is
  idempotent by its own key, holds for each method: `demand` stores by record
  id, `decline:<demand>` exists, `acknowledge` and `deliver` are keyed by record
  ids, and a repeated holdings answer only sends the same records again.
- **"Retry until" and the new levels.** The option is a real need (without it a
  demand to an issuer offline for more than 10 hours is lost) and is placed in E2.
  The two new "No answer" levels are right, and "could not reach the issuer" is
  never counted against the issuer, in the table, in OB-23 and in the Hub text.
  OB-17 is scoped to an issuer with a mailbox, and OB-68 covers the other case.
  OB-23, OB-68 and OB-70 can be tested as written. OB-67 and OB-69 cannot: see
  P44 and P47.
- **Question 17 with no mailbox.** Correct. A holder's acknowledgement goes to the
  issuer's mailbox. So an issuer with no mailbox gives no third-party time for any
  of its holders' acknowledgements, and a holder with no mailbox does not change
  this. The "A holder with no mailbox" paragraph says the issuer's declines
  wait in the issuer's outbox, and that the holder's acknowledgement of a decline
  still counts, which is right if the issuer has a mailbox.
- **The VX-15 deviation rows.** Four rows are under "Added by the owner's
  decisions" (VX-15, VX-49 evidence, the acknowledgement rule, the outbox
  attempt budget). Together with the facts rows (change.md:226-227) they cover
  every change the reversal makes in the design text.

### P44 — major — "Deploying with no mailbox succeeds" is false today, and the change it needs is not listed

**Where:** change.md:188 (OB-67), 701 (the deploy warning), 1007, 1278 (E3), and
the facts row at :227; `crates/app_orchestration/src/compiler.rs:174-177`.

**Problem:** OB-67 says deploying `obligation` with no mailbox succeeds and only
prints a warning. But the manifest compiler refuses exactly this: "A service
lists `mailbox_methods` only with a mailbox" (`check_mailbox_methods`, which
pushes the error `'<name>' lists mailbox_methods but has no mailbox`,
[compiler.rs:167-177](../../../../crates/app_orchestration/src/compiler.rs#L167)).
The design needs `obligation` to list `syneroym-roym:obligation/api@0.1.0#invoke`
in `mailbox_methods` (change.md:1007), and the manifest `roym.toml` is static,
with the mailbox chosen only at deploy (service mailbox change, K-3). So the
first deploy of the Roym app without a mailbox fails at the compile step, with
this error. The rule comes from `D-C6-5` ("only with a mailbox"). The facts row
at :227 cites K-3, `D-C7-6`, `D-C5-2` and `D-C6-10` as if a public service
without a mailbox already works, and says nothing about this check. E3 lists "the
deploy warning" but not the change of the compiler. The `roymctl svc deploy
--mailbox-method` path and the SDK mapper (`crates/sdk/src/mapper.rs:404`) must
be checked for the same rule.

**Fix:** Say plainly that the compiler's error becomes a warning (or is removed)
for a service that lists `mailbox_methods` and has no mailbox, and that the
substrate and `svc deploy` paths are checked for the same rule. Add it to the
facts table as a code fact, to the E3 task, and as a deviation from the service
mailbox change (`D-C6-5`). OB-67 then tests the compiler: an app with
`mailbox_methods` and no mailbox compiles with one warning.

**Status:** Fixed. Agreed, checked in the code. The refusal is in `check_mailbox_methods`, called from `validate_plan_member_settings`, which runs in both `compile` and the supervisor's `submit`. `roymctl app deploy` applies `--mailbox` to the manifest before `compile`, so only a deploy with no mailbox at all fails. Nothing else depends on the refusal: the SDK mapper sends the methods only inside `MailboxCredentials`, which exist only with a mailbox key (`mapper.rs:402-406`), and no Roym manifest lists `mailbox_methods` today. The one test that expects the refusal (`compiler/tests/member_settings.rs`) changes. E3 removes the refusal from `validate_plan_member_settings` (both paths), and `app deploy` warns. `roymctl svc deploy` keeps "--mailbox-method needs --mailbox": there the operator types the methods for one service, so it is a usage error, and Roym is not deployed that way. So I chose "removed" with a warning in `app deploy`, not a compiler warning, because `compile` has no warning channel. New facts row, E3 task, OB-67 rewritten (compiler, plan check, `app deploy`), and a deviation from `D-C6-5` under "Added in review round 6".

### P45 — major — The cap of caller rows lets 10,000 false callers block every new caller for an hour

**Where:** change.md:813 (counter rows), OB-65 (change.md, "Added in review round
5").

**Problem:** The fix for P43 says that inside one hour bucket at most 10,000
caller-DID counter rows exist, and "a call from a caller with no row in a full
bucket gets the rate-limit error and makes no row". The caller DID costs
nothing: any key can certify itself as a master (service mailbox change,
`D-C5-22`), and every call through a mailbox arrives with such a verified DID.
An attacker who makes 10,000 calls from 10,000 fresh DIDs (about 3 a second)
fills the bucket. For the rest of the hour, every caller that has no row yet is
refused. That is every holder who contacts this node for the first time, which
is the case VX-15 exists for ("a holder who never talked to the issuer"). The
cost to the attacker is low, and the harm repeats each hour.

**Fix:** Do not refuse a new caller because the bucket is full. Use a counter
table of fixed size instead of one row per caller: for example 4,096 slots per
bucket, with the slot chosen by a hash of the caller DID. Two callers may then
share a slot, which makes the 60-call limit slightly stricter for them, but no
caller is ever refused for being new, and the storage is bounded without a
cap on callers. Change OB-65 to test that a full table still answers a new
caller, and OB-57 for a shared slot.

**Status:** Fixed, as proposed, with one addition. 4,096 slots per hour bucket. The slot is a keyed hash, HMAC-SHA256 with a 32-byte node salt that `obligation` keeps in its data, so a caller cannot choose another caller's slot. With a plain hash an attacker could pick DIDs that land in one victim's slot and lock that holder out with 60 calls. With the salt, blocking one honest caller needs every slot full, about 245,000 calls an hour, which the doc names as a flood that no per-caller limit stops. OB-57 tests a shared slot, OB-65 tests that a new caller is accepted after 10,000 callers, and the rows table says at most 4,096 rows per bucket.

### P46 — minor — A mailbox that the issuer drops does not leave a demand "never collected"

**Where:** change.md:697 ("When the issuer drops its mailbox later"), OB-70, the
"No answer" table (rows 4 and 5).

**Problem:** The text says a demand still waiting in the dropped mailbox "is
never collected", so the level reads "stored, never collected". In the code, an
`unregister` turns every message of the queue into an `Expired` receipt
(`unregister`,
[queue.rs:126-150](../../../../crates/mailbox/src/store/queue.rs#L126)). The
sender's router treats an `Expired` receipt as one that does not verify
(`judge`,
[poll.rs:108-121](../../../../crates/router/src/proxy/mailbox/poll.rs#L108): the
`verify_signed` error becomes `retry`), and the outbox then goes back to direct
attempts. The issuer's record now names no mailbox, and "retry until" keeps the
call alive until `answer_by_secs`. So the same demand can be delivered directly
when both nodes are online. The holder then has a `stored` receipt, an `expired`
one, and a successful direct call. The table has no rule for this mix: row 4
("stored … or an `expired` one") would still read "never collected", and row 5
says a direct delivery has strength "none". The first is wrong, because the
issuer did receive it.

**Fix:** Say what the router does after an `Expired` receipt, and add the order of
precedence: a later successful direct delivery replaces "stored, never
collected" by the row for a direct delivery (row 5). Add this case to OB-70
(deposit, drop the mailbox, bring both nodes online, check the level).

**Status:** Fixed. Agreed, checked in the code: `unregister` (`crates/mailbox/src/store/queue.rs`) writes `Expired` for each queued message, and `judge` (`crates/router/src/proxy/mailbox/poll.rs`) answers "retry" when `verify_signed` refuses it, so the outbox goes back to direct attempts, and "retry until" keeps the call alive to `answer_by_secs`. The "drops its mailbox" paragraph now says this. The "No answer" table has an order: the first row that applies, from the top, sets the level, and row 4 now says "and no later direct delivery". So a direct delivery after an `expired` receipt reads "received by the issuer's node, no proof". I added one limit: a holder can show a third party only the deposit proof and hide the direct delivery, and nothing the issuer holds corrects that. OB-70 covers both outcomes (online in time, and the issuer kept offline).

### P47 — minor — `receiver_mailbox` cannot tell "no mailbox" from "record not read"

**Where:** change.md:583 ("`receiver_mailbox`, or none"), 702, OB-69.

**Problem:** The warning "this issuer has no mailbox" is shown when the
`receiver_mailbox` of the last call is none. A call can fail before the router
reads the issuer's service record (the record cannot be fetched, or it is
expired). The status read has no mailbox then, too. The holder's Hub would tell
the holder that an issuer has no mailbox when the router only could not find the
record. OB-69 tests only the case "none".

**Fix:** Give the field three values: the mailbox DID, "none" (a record that was
read and names no mailbox), and "unknown" (no record was read). The warning shows
only for "none". Add the case "unknown shows no warning" to OB-69.

**Status:** Fixed, as proposed. `receiver_mailbox` is the mailbox DID, `none` (a valid record read, naming no mailbox) or `unknown` (no valid record read). The warning shows only after a `none`, and an `unknown` leaves the shown state unchanged. OB-69 tests each value and `none` then `unknown`, and E2 gains a router test that a failed lookup gives `unknown`.

## Fix session round 6 — Claude Opus 5.5 (Claude Code), 2026-11-26

Fast-forwarded to the review commit `15ea0f32`. All four findings are fixed in
`change.md`. No owner question is new.

- **P44:** checked in the code. The compiler and the supervisor's plan check
  share the refusal. E3 removes it; `app deploy` warns; `svc deploy` keeps its
  own usage refusal. Nothing at runtime depends on it. New facts row, E3 task,
  OB-67 rewritten, deviation from `D-C6-5`.
- **P45:** a fixed table of 4,096 slots per hour bucket, chosen by a salted
  HMAC of the caller DID, so a caller cannot aim at another caller's slot.
  OB-57 and OB-65 rewritten.
- **P46:** checked in the code. A dropped mailbox gives `Expired`, the sender
  goes back to direct attempts, and "retry until" can still deliver. The "No
  answer" table now has an order of rows, and OB-70 covers both outcomes.
- **P47:** `receiver_mailbox` has three values (DID, `none`, `unknown`); the
  warning shows only after `none`. OB-69 rewritten.

OB ids are unchanged: OB-57, OB-65, OB-67, OB-69 and OB-70 are changed in
place, each with a "(Changed in review round 6, Pn.)" note. No new OB row.
