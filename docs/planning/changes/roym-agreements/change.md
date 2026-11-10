---
status: draft
living-docs-touched:
demo:
---
# Roym agreements

*Reader: developers who plan and build M08C (deal core and extensions), and the project owner, who makes the product decisions listed under [Open questions](#open-questions).*

This is the **agreements module** change document of the [Roym Value Exchange](../roym-value-exchange/change.md) (the overview). The overview gives the rules. This document gives the exact formats: the records, their fields and limits, the states, the disclosure encoding, the event log, the statement and the methods. It follows the overview's rules. Where it refines or differs from the overview or a sibling change, the difference is listed in [Differences from the overview and sibling changes](#differences-from-the-overview-and-sibling-changes), with the document that is updated at close-out.

Related documents:

- [Roym extensions](../roym-extensions/change.md): the `details` block, step records, and the reserve, confirm and release hooks. This document defines only the points where they connect to agreements.
- [Roym message forwarding](../roym-message-forwarding/change.md): forwarding a quote as itself (FW-7).
- [Roym multilingual foundation](../roym-multilingual/change.md): language tags, author translations, and the two "what I viewed" fields of an acceptance (ML-18).
- [Integrated experience spec](../../../roym-integrated-experience-spec.md): today's transaction rules. Terms such as **quote author** and **counterparty** are defined in [TERMINOLOGY.md](../../../TERMINOLOGY.md).

## Terms

Terms from the overview (participant, resource, resource kind, commitment, commitment kind, declared value, unit of account, enquiry, bid, quote author, agreement, economic event, event log, statement, EEP) keep their meaning. This document does not use the overview's term "barter". Every agreement has one shape, with any mix of resources on each side, and an agreement without money follows no special rule. New terms here:

- **Hub:** the Roym app screens a participant uses. Each participant's own node serves them, from its `web` service (`crates/roym_web/ui`). The Hub is not a shared server or meeting place. When this document says "the Hub shows", it means the participant's own app computes and shows it.
- **Deal:** one quote id (`quo_…`) with all its versions, the enquiry it answers, and the agreement if one forms. The quote id stays the same across versions. One enquiry can lead to several deals, for example two different offers, and each deal can form its own agreement. See [Deal rules](#deal-rules).
- **Agreement id:** the `record_id` (`rec_…`) of the quote version that both parties accepted. Today's code already uses this value as the agreement id (`payment.agreement`, `fulfilment.agreement`).
- **Writer:** the quote author's `transaction` service. It is the only node that writes the agreement's state.
- **Progress snapshot:** a signed record, written only by the writer, that states the agreement's state and each commitment's state at one moment. It replaces today's `booking-progress`.
- **Half:** one party's signed record of a two-party fact, for example one side of the agreement receipt. "Signed by both" means one half from each party, as today.
- **Hidden field:** a field of a quote whose value is not in the signed record. The record holds only a hash of it.
- **Salt:** a random value mixed into a hidden field's hash, so the value cannot be guessed from the hash.
- **Disclosure:** the salt, the field name and the value of one hidden field, encoded as one string. Whoever holds a disclosure can show that field to someone else. The format is SD-JWT's (see [Selective disclosure](#selective-disclosure)).
- **Digest:** the hash of one disclosure. The signed record holds the digests.
- **Presentation:** what a party gives a third party to show a deal: signed records plus the disclosures it chooses.
- **Reservation:** a short hold on a resource (for example a seat in a slot), taken by the node of the party that gives the resource (see [Who reserves](#who-reserves)). See [Roym extensions](../roym-extensions/change.md).

## Problem

Today a Roym deal has one shape. The counterparty pays money. The quote author does the work. The quote holds one `AgreedTerms` block with one amount, one scope and one optional slot (`crates/roym_core/src/transaction.rs:153`). Both halves of the `agreement-receipt` carry a plain copy of that block (`transaction.rs:234`). So does each `fulfilment-receipt` half (`crates/roym_core/src/fulfilment.rs:36`), and each `payment-acknowledgement` carries the amount, the method and the payment reference in plain form (`crates/roym_core/src/payment.rs:94`).

This has four results:

- **One deal cannot hold several things on each side.** A teacher cannot trade a lesson for a sack of wheat plus ₹100. The only shape is money for work.
- **A party cannot show how a deal went without showing what it cost.** Any receipt it shows carries the whole price. Reputation (M08G) needs outcomes without amounts.
- **There is no record of what a participant gave and received over time.** A participant who needs a statement for tax has to collect it by hand.
- **The deal core knows about slots.** The booking track, seat claims and slot ids are inside `transaction`. The [Roym extensions](../roym-extensions/change.md) change moves them out. The deal core then needs a generic shape that an extension can attach data to.

## What a participant can do after this change

- Make one agreement with several commitments on each side: a service, money, goods, and later EEPs.
- Exchange any mix of services, goods, money and later EEPs, on either side. Money is not required. Each commitment has a declared value in the agreement's one currency, so the two sides can be compared. Before signing, each party sees what it gives, what it receives, and the difference.
- See the state of each commitment, not only of the whole deal. When something fails, see which commitment failed and whose step is missing.
- Show a deal to someone else with chosen fields hidden, for example the price. The signatures still verify.
- Export a statement for a period: every commitment given and received, with its declared value.

## Scope and non-goals

- In scope:
  - The record shapes of enquiry, quote (and bid), agreement receipt, progress snapshot, deal decline, commitment failure, and the per-commitment settlement records.
  - Resource kinds, commitment kinds, declared values, and one currency per agreement.
  - The `details` block's place inside a commitment.
  - States and transitions for the deal, the agreement and each commitment kind, with their failure ends.
  - The single-writer rule, and how the counterparty's copy stays in step.
  - The agreement side of reserve, confirm and release.
  - Selective disclosure on the quote, the agreement receipt and the settlement records.
  - Mixed exchanges: who writes the declared values, and how the Hub shows a difference between the two sides.
  - The event log and the statement export, with a defined section for EEPs that M08E fills.
  - Every new or changed `transaction` method, and whether it is wire-reachable.
  - What replaces today's quote, receipt and booking records, in place.
- Not in scope:
  - Extension manifests, step records, widgets, extension services and deal grants. They are in [Roym extensions](../roym-extensions/change.md). This document only names where they connect.
  - Message forwarding and signed message origins. They are in [Roym message forwarding](../roym-message-forwarding/change.md).
  - EEPs: their records, issue, redemption, decline, split, transfer and the `obligation` service. They are in the obligations change document (M08E, extended in M08H). This document reserves the three EEP commitment kinds and the statement's EEP section.
  - Wants, bids on wants, and the exchange (M08D, M08F). This document reserves the quote field that names a want.
  - Evidence, opinions, and the grant that lets a third party see amounts (M08G and its disclosure ADR).
  - The two C0 ADRs (deal grants; signed message origins and the forward mode). See [What the C0 ADRs must decide](#what-the-c0-adrs-must-decide).
  - Moving money, checking that a payment happened, or computing tax. Unchanged from today and from the overview.
  - More than one currency in one agreement.
  - Data migration. The product is not released. Records change in place.

## Acceptance criteria

Each row refines an overview criterion (`VX-*`), an extensions criterion (`EX-*`) or a multilingual criterion (`ML-*`), or adds a rule this document needs. "Test" names the kind of test that proves it. File names are filled in when the tests are written.

Kinds of test:

- **Unit:** a test of a pure function in `roym_core` (no host).
- **Service:** a test of one `transaction` service against a test host, with a controllable clock.
- **Two-node:** a substrate integration test with two nodes (`crates/substrate/tests`, using `SubstrateTestContext`).
- **E2E:** a Playwright test of the Hub.
- **One node only:** the rule can only be checked on one party's own node, because only that node holds the facts. A test on another node can only check that the rule is shown as "not checked".
- **Reference code only:** the rule is about what a node chooses not to do. Another node cannot observe it. The test runs against Roym's own code.

| ID | Criterion | Test |
| --- | --- | --- |
| **Participants and resources** | | |
| AG-1 | One identity is the quote author of one deal and the counterparty of another, on the same node, at the same time, with no role setting (VX-1). | Two-node: A quotes B, and B quotes A; both agreements reach `agreed`. |
| AG-2 | Each built resource kind can be given by either party: for every pair (giver, resource kind) the quote validator accepts the commitment (VX-2). M08C builds `service-in-slot`, `goods-or-service` and `currency`. M08E adds `eep` to the same table. | Unit: a table test over giver × resource kind. Two-node: a service-for-service agreement with no currency commitment that closes. |
| AG-3 | A quote that answers a want and a quote that answers an enquiry produce the same agreement receipt and the same progress snapshot. The receipt does not carry the `answers` field (VX-6). M08C builds only enquiries. M08F adds the Want-Bid end-to-end test. | Unit: build both quote forms from the same terms, with a fixed salt source. The two receipt payloads are equal except `quote_record_id`, and the two first snapshots are equal except `agreement`. |
| **Agreements and commitments** | | |
| AG-4 | One agreement holds 1 to 16 commitments. Each names its giver, its resource kind, its commitment kind and its declared value. All declared values are in the agreement's one currency (VX-7). | Unit: validation table (0, 1, 16, 17 commitments; missing fields). Two-node: an agreement with a service, a currency commitment and a goods commitment forms and closes. |
| AG-37 | The service assigns commitment ids `c1` to `c16`, by position in the list. `quote.set` and `verify_quote` refuse any other id. The quote author never chooses one. | Unit. |
| AG-5 | No commitment carries its own currency. A quote with a currency that `money::currency_minor_exponent` does not know is refused (one currency per agreement). | Unit. |
| AG-6 | Each commitment moves only through the states of its kind, and every kind has a failure end. The agreement closes only when every commitment is final (VX-8). | Unit: a table test of the pure transition function over every (state, event) pair of the transition tables in [Commitments](#commitments), for every kind built in M08C. The obligations document adds the EEP kinds' tables. |
| AG-7 | A commitment that has not settled when its track window ends becomes `ended-unconfirmed`. The progress snapshot says whether the giver's half exists (`claimed`) (VX-8, VX-12). | Service: controllable clock. |
| AG-8 | Each final failure state says what is known with no disclosures: `ended-unconfirmed` carries `claimed`, and `failed` and `cancelled` carry a reason code. The Hub words the two `ended-unconfirmed` cases differently: "the giver says it delivered; the receiver did not confirm" (`claimed: true`) and "nobody recorded anything" (`claimed: false`) (VX-12). | Unit: verify a presentation that holds only the progress snapshot and the receipt halves. |
| AG-9 | Only the quote author cancels, and only while no half of any commitment exists. A later cancel is refused with `commitment-moved`. | Service. |
| AG-10 | Either party can decline with a signed `deal-decline` record: the counterparty declines a quote or withdraws its own enquiry, and the quote author declines an enquiry. Each node moves the deal (or the enquiry) to `declined` when it files the signed decline. A decline of any version ends the whole quote id. A repeated decline returns the same record. | Two-node. |
| **Single writer** | | |
| AG-11 | Only the writer writes the agreement state. The counterparty's node files a progress snapshot only when the writer signed it, and only when its `seq` is higher than the one it holds (VX-9). | Unit and service. |
| AG-12 | A second, conflicting change gets a named conflict, never a merge. `agreement.accept` of a version that is not the latest one the counterparty holds answers `quote-superseded`. When the writer refuses an acceptance it files, it sends a snapshot with `seq` 1, state `not-formed` and a code from [Reason codes](#reason-codes). Two snapshots with the same `seq` and different content → `progress-fork`, and the first one is kept (VX-9). | Service for the method error. Two-node: the counterparty accepts a superseded version through a test hook that skips the method check, and its node then shows the refusal with `quote-superseded` on that version's card, the deal is still `quoted`, and an acceptance of the latest version then reaches `agreed`. Unit for the fork. |
| AG-35 | Every end of a deal ends its quote id. After `declined`, `expired`, `not-formed`, `cancelled` or `closed`, `quote.set` refuses a new version of that quote id with `quote-ended`. A new offer is a new quote id. Two quote ids that answer one enquiry may both form agreements. | Service. |
| AG-43 | `quoted → expired` and `accepted → expired` are computed on each node from `quote_expires_at_secs`. The writer refuses to countersign after the expiry by its own clock (`not-formed`, `quote-expired`). A countersigned half whose `issued_at_secs` is before the expiry moves the deal to `agreed`, even when it arrives after the expiry. | Service with a controllable clock. |
| AG-41 | `agreement.accept` on the quote author's own node is refused with `writer-countersigns`. The counterparty always signs first. The writer countersigns only in `transaction.sync`. | Service. |
| AG-13 | Every state-changing call is idempotent. A repeated `agreement.accept`, `fulfilment.sign`, `payment.acknowledge` or `deal.decline` returns the record already made (today's `D-C8-6`). | Service. |
| AG-42 | After a restore from an old backup, the writer's next snapshot has a `seq` higher than any of its own snapshots in the conversation. | Service: restore an old backup, then sync. |
| **Reserve, confirm, release (agreement side)** | | |
| AG-14 | When the writer gives a resource that needs a reservation, it takes the fence and reserves in the order of [The writer's decision](#the-writers-decision). Two counterparties who accept quotes for the last seat: one agreement reaches `agreed`, the other ends `not-formed` with reason `slot-taken` (VX-10, EX-6). A crash after any step ends, after the next sync, in the same state as no crash, with two exceptions: a recovery after the quote's expiry ends `not-formed` with `quote-expired` and countersigns nothing, and a recovery whose `confirm` is refused ends `not-formed` with the extension's reason. | Two-node: today's booking race test, moved to the scheduling extension. Service: a test host that fails the next write after each step. The same test with a controllable clock moved past the expiry before the recovery, and with an extension stub that refuses `confirm`. |
| AG-15 | When the counterparty gives a resource that needs a reservation (for example a service in a booked slot), its node reserves it before it signs its acceptance. If the reserve fails, it signs nothing, and `agreement.accept` answers `reserve-failed` with the extension's reason (VX-10, EX-5). | Service. |
| AG-16 | If the counterparty's confirm is refused after its reservation expired and the resource was given away, its node sends a signed `commitment-failure`. The writer marks that commitment `failed` with reason `reservation-lost` (EX-22). | Two-node with a controllable clock. |
| AG-36 | When a commitment ends `failed`, the writer ends every other commitment of the agreement that is still `promised` or `payment-requested` as `cancelled`, with reason `other-commitment-failed`, in the same snapshot. A commitment that already has a half keeps its own path. | Unit for the transition tables; service for the snapshot. |
| **Selective disclosure** | | |
| AG-17 | The quote, the agreement receipt and the settlement records carry no hidden value in plain form. A test serialises each signed payload and checks that no hidden value appears in it (VX-11). | Unit. |
| AG-18 | A party shows its agreement to a third party with the price hidden and the scope shown. The third party's node verifies both receipt halves and the shown field (VX-11). | Unit, and two-node with a third node that only verifies. |
| AG-34 | A party shows a quote, before any agreement, with the price hidden and the scope shown. The third party's node verifies the quote and the shown field. When a presentation holds a quote and a receipt half, the verifier checks that their `terms` are equal (VX-11). | Unit, and two-node with a third node that only verifies. |
| AG-38 | A presentation verifier accepts a progress snapshot as the writer's only when its issuer equals the `signer_did` of the quote author's receipt half, or of the quote, in the same presentation. Otherwise it shows the snapshot as "writer not checked". | Unit. |
| AG-19 | The digest and the verifier follow RFC 9901. (1) Our digest function gives `X9yH0Ajrdm1Oij4tWso9UzzKJvPoDxwmuEcO3XAdRC0` for the RFC's disclosure string `WyJfMjZiYzRMVC1hYzZxMktJNmNCVzVlcyIsICJmYW1pbHlfbmFtZSIsICJNw7ZiaXVzIl0` (RFC 9901, section 4.2.3). (2) Our disclosure decoder accepts that string as written, with its spaces. The 16-byte salt rule (AG-21) is a Roym record rule, not a decoder rule, because the RFC's example salt is not 16 bytes. (3) Our encoder's output for a fixed salt is pinned by our own vector. (4) The verifier refuses a disclosure whose digest is not in the record, a digest that appears twice, a name that is not a hidden field of that object, a name that equals a visible field, and a value with a non-integer number. | Unit: test vectors. |
| AG-20 | Both receipt halves carry a `terms` object that is byte-identical, after canonicalisation, to the quote's `terms`. A half whose `terms` differ is refused when it is filed. | Unit. |
| AG-21 | Every quote version uses fresh salts. Each salt is 16 bytes (a Roym rule on top of RFC 9901). The counterparty's node refuses a quote with a salt of another length, or a salt used twice inside one record. Reuse across versions is the author's duty and is not checked. | Unit. |
| AG-40 | The `details` disclosure of one commitment is at most 8 KiB, encoded. All disclosures of one record are at most 96 KiB, encoded. `quote.set` refuses a larger quote with `quote-too-large`, and the Hub's quote form shows the size against the limit before the person signs. | Unit; E2E for the Hub's size display. |
| **Mixed exchanges and declared values** | | |
| AG-22 | An agreement with no currency commitment forms and closes through the same code path as one with money. It still names a currency, and every commitment still has a declared value. No validator, state or card checks whether a currency commitment exists. | Two-node, and a unit test that runs the same quote with and without a currency commitment. |
| AG-23 | A declared value is an integer from 0 to 10^15 minor units. A `currency` commitment's value is at least 1. Before a party signs, the Hub shows the declared value of every commitment, the total that party gives, the total it receives, and the difference when they are not equal. A difference never blocks signing. No record stores the difference. | Unit for the range; E2E for the Hub, with one agreement whose totals differ. |
| AG-33 | The receiver can settle a commitment with a `settlement_note` that says how it was really handed over. The commitment settles as usual, its declared value does not change, and `statement.export` shows the note on that commitment's row. The note is hidden like every other free text (AG-17). | Service, and unit for AG-17's check on the new field. |
| **EEP rules checked on the agreement side (built in M08E and M08H)** | | |
| AG-24 | **One node only.** The issuer's node refuses to sign a quote or an acceptance that makes it the giver of an `eep-issue` commitment while it has no published offering (VX-13). The receiving node cannot know the issuer's full offering list. It shows "not checked" unless it found an offering of the issuer that is valid now. | Service, on the issuer's node. Built in M08E. |
| AG-25 | **One node only.** The issuer's node refuses an `eep-issue` commitment whose face-value currency does not appear in its own published offerings, or whose face value is not an integer in minor units (VX-14). The receiving node checks only the integer rule, and shows the currency rule as "not checked". | Service, on the issuer's node. Built in M08E. |
| AG-26 | (a) **Reference code only.** The refusal reason type of Roym's reissue code has exactly the three values of VX-27. The reissue handler returns a reissued EEP, a signed refusal with one of the three reasons, or an internal error. The rule that any issuer refuses only for these reasons cannot be checked from another node. (b) On the writer's node, an `eep-transfer` commitment ends `refused` only when the writer holds the issuer's signed refusal with one of the three reasons. An unsigned refusal, or one with an unknown reason, leaves the commitment `endorsed`. | (a) Unit: a table test over the handler's inputs, against the `obligation` code. (b) Service, on the writer's node. Built in M08H. |
| **Event log and statement** | | |
| AG-27 | Every agreement event this node takes part in is appended to its event log exactly once. Running `transaction.sync` again, or `eventlog.rebuild`, adds no duplicate (VX-45). | Service. |
| AG-28 | Nothing on the deal path reads the event log. A full deal runs under a test host that records every data-layer read, and no read touches the `event_log` collection (VX-45). | Service. |
| AG-29 | `statement.export` for a period lists every commitment settled in that period, by the signed time of its settling half (`occurred_at_secs`), so both parties put it in the same period. It lists commitments given and received, with resource kind, commitment kind, counterparty, currency and declared value, and totals per currency. Commitments of agreements with and without money appear the same way (VX-46). | Service, and E2E for the Hub's export. |
| AG-30 | The statement has an `eeps` section. Before the `obligation` service exists, it says `not-available`. With an `obligation` sibling, it lists the EEPs outstanding (issued and held) at the start and at the end of the period. If the sibling call fails, it says `unavailable` with the reason. It is never left out and never empty in place of an error. | Service, with a fake `obligation` sibling. |
| **Methods and language** | | |
| AG-31 | Every `transaction` method answers `-32013` when called from another node. | Service: extend the admission test table. |
| AG-32 | Each acceptance half carries `viewed_lang` and `viewed_via_mt` outside `terms`. The two halves of one agreement may differ only in `role`, `viewed_lang` and `viewed_via_mt` (ML-18, kept). | Unit. |
| AG-39 | Every `LocalText` in one record has the same set of translation languages. `quote.set` and the verifier refuse a record that breaks this. | Unit. |

## Design

### Facts checked in the code

The design builds on these facts about today's code (checked 2026-11-10 on `main` at `4830bdf7`):

| Fact | Where |
| --- | --- |
| A quote's id is `content_digest("quo_", {conversation, issuer, sequence})`, so it stays the same across versions. A version supersedes the previous one. | `crates/roym_core/src/transaction.rs:524`, `crates/roym_transaction/src/app/quote_ops.rs:285` |
| The agreement is two `agreement-receipt` halves over the quote's `record_id`. For a quote with a slot, the counterparty signs first, and the quote author's node countersigns during `transaction.sync`, after it decides the booking. For a quote with no slot, the quote author may also call `agreement.accept` first. | `crates/roym_transaction/src/app/agreement_ops.rs:99`, `:373`, `:402` |
| The writer's fence is a unique insert: `AppDataLayer::create` writes `decision:<agreement>` and `step:<agreement>:1`, plus `seat:<slot>:<n>` for a slot quote, together or not at all, and names the first row that already exists. All these rows are in `transaction`'s one `ledger` collection. A `create` or `batch-mutate` call works on one collection of one service. | `crates/roym_transaction/src/app/ledger.rs:52`, `:105`, `:116`, `crates/wit_interfaces/wit/data-layer/data-layer.wit:161`, `:174` |
| `booking-progress` is signed by the node's service key (`Principal::Service`), not by the person. The reader binds it to the quote's signer. | `crates/roym_transaction/src/app/progress.rs:32`, `crates/roym_core/src/booking.rs:327` |
| A track settles when the party that receives the resource signs its half ("against interest"). The giver's half alone is a claim. | `crates/roym_core/src/booking.rs:260` |
| Declining a quote is local and unsigned. The quote author is never told. | `crates/roym_transaction/src/app/quote_ops.rs:508` |
| Two versions of one quote can both become agreements, by two paths (from reading the code, not run; confirmed in review). (1) `quote.set` lets the author revise a quote that was already accepted, and the decision row is keyed by the version's `record_id`, not by the quote id. (2) `agreement.accept` loads any held version by its `record_id` and does not check that it is the latest, and the Hub shows "Accept" on every version's card. | `quote_ops.rs:285`, `ledger.rs:52`, `agreement_ops.rs:168`, `crates/roym_web/ui/src/screens/messages.ts:598` |
| A new quote version clears a decline. One enquiry can get several quote ids. | `quote_ops.rs:610`, `:310` |
| A payment acknowledgement can be corrected with `supersedes`. | `crates/roym_transaction/src/app/sync/receipts.rs:452` |
| The track window ends at `window.latest_secs` (or the scheduling time when there is no window) plus `TRACK_WINDOW_SECS` (30 days). | `crates/roym_core/src/booking.rs:189` |
| A signed payload is canonical key-sorted `serde_json`, not full RFC 8785. Ids are z-base-32 SHA-256. A payload is at most 64 KiB. A card body is at most 160 KiB. | `crates/signed_record/src/envelope.rs:213`, `crates/roym_core/src/card.rs:63` |
| Every `transaction` method is local-only (`admit::require_internal`). | `crates/roym_transaction/src/app.rs:278` |
| The Hub has seven card types. | `crates/roym_core/src/card.rs:9` |
| A record may carry at most 4 author translations. | `crates/roym_core/src/lang.rs:7` |
| ML-18's `viewed_lang` and `viewed_via_mt` are not in the code yet. | `transaction.rs:234` |

### The flow

```text
 Counterparty node                               Quote author node (writer)
 ─────────────────                               ──────────────────────────
 enquiry.set ── enquiry card ─────────────────▶  files it                     deal: requested
                                                 quote.set (salts, digests)
             ◀──────────────── quote card + all disclosures ──                deal: quoted
 verifies quote and disclosures
 agreement.accept
   reserve own resources (if any)
   sign acceptance half ── receipt half card ──▶ sync files the half          deal: accepted
                                                   fence decision:<quote_id> (deciding)
                                                   reserve own resources (idempotent)
                                                   ok:   countersign          agreement: agreed
                                                   fail: release, snapshot    deal: not-formed
             ◀───── countersigned half + progress snapshot seq 1 ──
 confirm own reservations
   refused? ── commitment-failure card ────────▶ marks commitment failed
 settlement halves per commitment ◀──────────▶  writer applies each half,
                                                 sends a new snapshot          agreement: closed
                                                                               when all final
```

All traffic between the two parties is Conversation cards, as today. No step needs a wire-reachable method.

### Common rules for every record

- **Integers only.** A signed payload holds no non-integer number, as today. Money is minor units with a currency. This holds inside disclosures too.
- **Text fields are language-tagged.** A free-text field is a `LocalText` object:

  ```text
  LocalText {
    lang:          LangTag            // the original's language, canonical BCP 47
    text:          string             // the original; this text binds
    translations:  [ { lang: LangTag, text: string } ]   // 0 to 4, author translations
  }
  ```

  The language and translation rules are M08B's (`check_text`, `check_translation_langs`). This replaces today's quote-level `lang` and `translations` list, because each text field can now be hidden on its own, and a translation must be hidden with its original. The byte limit applies to `text` and to each translation separately.

  **One set of translation languages per record.** Every `LocalText` in one record has the same set of translation languages. So the Hub's `bestVersion` (ML-12) still picks one version for the whole record, and one `viewed_lang` per acceptance half (ML-18) still says which text the person read.
- **Ids.** `quote_id` and `enquiry_id` are derived as today. A `commitment_id` is assigned by the service: `c1` to `c16`, by the commitment's position in the list. `quote.set` and `verify_quote` refuse any other id. The quote author never chooses it, because a visible free id (for example `inr-5000`) could carry a price. The Hub shows what changed between versions by comparing the disclosed descriptions, not the ids.
- **DIDs.** Every party DID is a `did:key` master DID (`person::is_did_key`).
- **Signer.** Every record is signed by a person through the `signing` host interface, except the progress snapshot, which is signed by the writer's service key, as `booking-progress` is today.

### Enquiry

Today's `request` record, renamed `enquiry`, version 1, signed by the counterparty. The fields are today's (`RequestPayload`, `transaction.rs:181`), with three changes:

| Field | Type | Limit | Change |
| --- | --- | --- | --- |
| `enquiry_id` | string | | renamed from `request_id`, prefix `enq_` |
| `description` | `LocalText` | 4096 bytes | replaces `description` + `lang` + `translations` |
| everything else | | | unchanged: `conversation`, `sequence`, `listing_id`, `categories`, `area`, `window`, `data_use_notice` |

An enquiry has no hidden fields. It holds an approximate area only, never an address, as today. A new version of the enquiry is a change request, as today.

### Quote

Record type `quote`, version 1 (the shape changes in place). Signed by the quote author. It has an envelope expiry equal to `terms.quote_expires_at_secs`, from 5 minutes to 90 days after signing, as today.

```text
QuotePayload {
  quote_id:          string        // content_digest("quo_", {conversation, issuer, sequence})
  conversation:      string
  sequence:          u32           // from 1
  answers:           Answers       // what this quote answers
  counterparty_did:  string        // did:key
  terms:             DealTerms     // copied byte for byte into both receipt halves
}

Answers =
  | { kind: "enquiry", record_id: "rec_…" }            // List-Search; built in M08C
  | { kind: "want", intent_id: string, version: u32 }  // Want-Bid: a bid; reserved, built in M08F
```

`DealTerms` holds the terms both parties sign. Some of its fields are visible. The others are hidden: only their digests are in `_sd`.

| Field | Type | Limit | Visible or hidden |
| --- | --- | --- | --- |
| `currency` | string | ISO 4217, a code `money::currency_minor_exponent` knows | visible |
| `quote_expires_at_secs` | u64 | must equal the envelope expiry | visible |
| `home_exchange` | string or absent | a DID; set by M08F, absent in M08C | visible |
| `commitments` | list of `Commitment` | 1 to 16 | visible structure; see below |
| `_sd` | list of strings | sorted ascending | the digests of the five hidden fields below |
| `listing_id` | string or null | as today | hidden |
| `scope` | `LocalText` | 4096 bytes | hidden |
| `cancellation_terms` | `LocalText` | 2048 bytes | hidden |
| `refund_terms` | `LocalText` | 2048 bytes | hidden |
| `dispute_path` | `LocalText` | 2048 bytes | hidden |

### Commitment

One commitment is one party giving one resource to the other.

| Field | Type | Limit | Visible or hidden |
| --- | --- | --- | --- |
| `commitment_id` | string | `c1` to `c16`, by position; assigned by the service | visible |
| `giver` | `"quote-author"` or `"counterparty"` | | visible. The receiver is the other party. |
| `resource_kind` | see the kind table | | visible |
| `kind` | see the kind table | | visible |
| `extension` | `{ id: string, version: u32 }` or absent | id `[a-z0-9-]{1,32}` | visible |
| `after` | list of `commitment_id` | 0 to 15, each in this quote, no cycles | visible. A hint for the Hub ("pay after the work"), never a gate. Replaces `payment_timing`. |
| `_sd` | list of strings | exactly 10, sorted ascending | the digests of the ten hidden fields below |
| `description` | `LocalText` | 2048 bytes | hidden. What is given. |
| `declared_value_minor` | i64 | 0 to 10^15; at least 1 for `currency` | hidden |
| `details` | `Details` or null | its disclosure at most 8 KiB, encoded | hidden |
| `window` | `TimeWindow` or null | `latest_secs >= earliest_secs` | hidden. When the work or delivery happens. |
| `location` | `QuoteLocation` or null | address 512 bytes | hidden. Today's `QuoteLocation`; an `address` only when the work is at the receiver's place. |
| `payee` | string or null | 256 bytes; currency only | hidden |
| `payment_methods` | list or null | 16 codes of `[a-z0-9-]{1,32}`; currency only | hidden |
| `tax_minor` | i64 or null | currency only; `tax + fees <= declared value` | hidden. A breakdown, never an addition. |
| `fees_minor` | i64 or null | currency only | hidden |
| `eep` | object or null | M08E fixes the shape | hidden. For the three EEP kinds only. |

**Every commitment has all ten hidden fields,** with `null` for the ones its kind does not use. So the number of digests says nothing about which fields are used. The validator refuses a currency-only field that is not `null` on a non-currency commitment, and an `eep` field that is not `null` on a non-EEP commitment.

**For a `currency` commitment, the declared value is the amount.** It is at least 1. There is no separate amount field. `payee` must not be empty. This keeps today's rule that the agreement binds the payee.

**Deposits and part payments** become two currency commitments, for example "₹200 before" and "₹800 after". Each settles with its own acknowledgement. Today's rule of one payment per quote (`ONE_PAYMENT_NOTICE`) becomes one payment per currency commitment.

#### Resource kinds and commitment kinds

| `resource_kind` | `kind` | Settled by | Extension | Built in |
| --- | --- | --- | --- | --- |
| `service-in-slot` | `fulfilment` | `fulfilment-receipt` halves | required, and it must have a reserve hook (today: `scheduling`) | M08C |
| `goods-or-service` | `fulfilment` | `fulfilment-receipt` halves | optional (`one-shot`, or none for the plain enquiry flow) | M08C |
| `currency` | `currency` | `payment-acknowledgement` halves | none | M08C |
| `eep` | `eep-issue` | the issued EEP, version 1 | none | M08E |
| `eep` | `eep-transfer` | endorsement, then the issuer's reissue | none | M08H |
| `eep` | `eep-return` | endorsement back to the issuer, then the issuer's closing record | none | M08E |

Until a kind is built, `quote.set` and `verify_quote` refuse it with `commitment-kind-not-supported`. The list is fixed in code, as the overview requires.

#### The `details` block

The shape is the extensions change's (EX-1, EX-3):

```text
Details {
  extension:  string                   // equals the commitment's extension.id
  schema:     string                   // "<extension>/<name>@<version>", e.g. "scheduling/slot@1"
  data:       object                   // checked against the schema, integers only
  extra:      [ { label: string, value: string } ]   // 0 to 16; label 64 bytes, value 512 bytes
}
```

- `details` is one hidden field. It is shown or hidden as a whole. Hiding single fields inside `data` is not in v1.
- A commitment with an `extension` must have `details` whose `extension` matches. A commitment with no `extension` has `details: null`.
- Today's `slot_id` moves into the scheduling extension's `data`. The scheduling extension checks that the slot's start and end equal the commitment's `window`.
- Who checks the schema, and which check binds, is EX-2.

### Agreement receipt

Record type `agreement-receipt`, version 1 (in place). One half is signed by each party. The envelope's `subject` is the quote's `record_id`, and the half has no expiry, as today.

```text
AgreementReceiptPayload {
  quote_record_id:   "rec_…"      // the agreement id; also the envelope subject
  quote_author_did:  string
  counterparty_did:  string
  role:              "quote-author" | "counterparty"
  viewed_lang:       LangTag      // ML-18: the language of the text this party viewed
  viewed_via_mt:     bool         // ML-18: true if that text was a machine translation
  terms:             DealTerms    // byte-identical to the quote's terms, digests included
}
```

- **Both halves sign the same list of digests,** because both copy `terms` from the quote unchanged. The quote author chose the salts when it signed the quote. Neither half holds a plain value of a hidden field.
- A pair is complete when it has one half per role and the halves agree. `halves_agree` compares `quote_record_id`, the two DIDs and `terms`, and requires different roles. `viewed_lang` and `viewed_via_mt` may differ.
- The receipt does not carry `answers`. So a bid and a quote that answers an enquiry give the same receipt (AG-3).

### Deal decline

New record type `deal-decline`, version 1. Signed by the party that declines.

```text
DealDeclinePayload {
  declines:  "rec_…"            // the record_id of the enquiry or quote version declined
  deal:      string             // the quote_id, or the enquiry_id when no quote exists
  conversation: string
  reason:    LocalText or null  // 512 bytes
}
```

- The counterparty declines a quote, or withdraws its own enquiry. The quote author declines an enquiry.
- Neither party can decline once the counterparty has signed its acceptance. From `accepted` the deal waits for the writer, and the quote's expiry bounds the wait. After the agreement forms, only the quote author cancels.
- A decline of any version ends the whole quote id (see [Deal rules](#deal-rules)). Asking for changes is a message or a new enquiry version, not a decline.
- It replaces today's local, unsigned `quote.decline`. The other party now learns of it, and the deal ends `declined` on both nodes.

### Progress snapshot

Record type `agreement-progress`, version 1. It replaces `booking-progress`. Signed by the writer's service key. It holds no hidden value and no free text, so it can be shown to anyone.

**Binding to the writer.** The issuer of a snapshot is the service key's `did:key`, not the person. The counterparty's node accepts a snapshot only when its issuer equals the `signer_did` of the quote, as `booking-progress` is bound today (`receipts.rs:89`). A third party may have no quote. Its verifier requires that the issuer equals the `signer_did` of the quote author's receipt half, or of the quote, in the same presentation. When the writer's node changes key (for example after a restore to a new device), the binding fails, and the verifier shows the snapshot as "writer not checked".

```text
AgreementProgressPayload {
  agreement:          "rec_…"
  conversation:       string
  quote_author_did:   string
  counterparty_did:   string
  seq:                u32                 // from 1, raised by each change
  state:              AgreementState      // see States
  state_reason:       string or absent    // a reason code, e.g. "slot-taken"
  cancel_reason:      CancelReason or absent   // a code, never free text
  commitments: [ {
    commitment_id:        string
    state:                CommitmentState
    reason:               string or absent   // a reason code for "failed" or "cancelled"
    claimed:              bool or absent     // for "ended-unconfirmed": whether the giver's half exists
    halves:               [ "rec_…" ]        // the settlement records the writer holds
    track_ends_at_secs:   u64
  } ]
}
```

`CancelReason` is one of `cannot-deliver`, `schedule-conflict`, `counterparty-asked`, `other`. The author's own words, if any, go in the conversation as an ordinary message. They are not part of a signed record that may be shown to strangers.

### Commitment failure

New record type `commitment-failure`, version 1. Signed by the giver of one commitment, who also owns its reservation (see [Who reserves](#who-reserves)). In v1 its only use is EX-22: the counterparty, as giver, could not confirm its reservation. The writer confirms its own reservations before it countersigns, so it never needs one: a refused confirm ends the deal `not-formed` instead (see [The writer's decision](#the-writers-decision)).

```text
CommitmentFailurePayload {
  agreement:      "rec_…"
  commitment_id:  string
  reason:         "reservation-lost"     // the only code in v1
}
```

The writer accepts it only from that commitment's giver, and only while no settlement half for that commitment exists.

### Settlement records

Each settlement record now names one commitment. None carries a hidden value in plain form. This is needed because today a `fulfilment-receipt` carries the whole `AgreedTerms` and a `payment-acknowledgement` carries the amount, the method and the reference. Showing either one would show the price.

```text
PaymentRequestPayload {            // signed by the receiver of a currency commitment
  agreement:         "rec_…"
  conversation:      string
  commitment_id:     string
  quote_author_did:  string
  counterparty_did:  string
  _sd:               [ 1 digest ]  // hidden: note (LocalText or null, 512 bytes)
}

PaymentAcknowledgementPayload {    // signed by the payer (a claim) or the payee (settles)
  agreement, conversation, commitment_id, quote_author_did, counterparty_did
  role:              "giver" | "receiver"
  observed_at_secs:  u64
  supersedes:        "rec_…" or absent   // corrects this signer's earlier half, as today
  _sd:               [ 3 digests ] // hidden: method (code or null),
                                   //   reference (string or null, 512 bytes), settlement_note
}

FulfilmentReceiptPayload {         // signed by the giver (a claim) or the receiver (settles)
  agreement, conversation, commitment_id, quote_author_did, counterparty_did
  role:              "giver" | "receiver"
  _sd:               [ 1 digest ]  // hidden: settlement_note
}
```

- `_sd` sits at the top level of each payload. Every hidden field is always present in its disclosure, with `null` when it is not used, as for commitments.
- `role` is the signer's role for this one commitment: `giver` or `receiver`. It replaces today's `DealRole`, because either party can be the payer of one commitment and the payee of another.
- The two DIDs stay, as today (`payment.rs:94`, `fulfilment.rs:36`). They are visible in the receipt anyway, and they let a reader see who signed what from one record.
- `supersedes` keeps today's correction rule for acknowledgements (`receipts.rs:452`), per commitment: only the same signer, only for the same commitment.
- The amount of a payment is the commitment's declared value. It is not repeated.
- **`settlement_note`** (`LocalText` or null, 1024 bytes, hidden) says how the resource was really handed over, when that differs from the agreement. Example: the agreement says ₹1,000, and the payee accepts ₹600 and a sack of rice. Either half may carry one. The note on the receiver's half is the one that counts, because that half settles the commitment. By signing it, the receiver says "I accept this as full settlement". The declared value does not change. See [Mixed exchanges and declared values](#mixed-exchanges-and-declared-values).
- The hidden fields of a settlement record use the same encoding as the quote. The signer of that record picks its salts and sends the disclosures with the card.
- A settlement record does not repeat the terms. To read what was delivered, a reader needs the receipt and the disclosures. This is the price of hiding.
- Today's rule stays: the receiver's half settles a commitment, and the giver's half alone is a claim.

### Who signs what

| Record | Quote author | Counterparty | Writer's service key |
| --- | --- | --- | --- |
| `enquiry` | | signs | |
| `quote` | signs | | |
| `agreement-receipt` | one half | one half | |
| `deal-decline` | declines an enquiry | declines a quote, or withdraws its own enquiry | |
| `agreement-progress` | | | signs |
| `commitment-failure` | | as giver of the commitment (v1) | |
| `payment-request`, `payment-acknowledgement`, `fulfilment-receipt` | its own half, as giver or receiver | its own half, as giver or receiver | |

### States and transitions

#### Deal and agreement

```text
 requested ──▶ quoted ──▶ accepted ──▶ agreed ──▶ closed
     │           │           │            │
     ▼           ▼           ▼            ▼
  declined    declined    not-formed   cancelled
              expired     expired
```

| State | Means | Entered when | Written by |
| --- | --- | --- | --- |
| `requested` | an enquiry exists, and no quote answers it yet. This is the enquiry's state; each quote id that answers it is its own deal from `quoted` on. | the enquiry is filed | each node, locally |
| `quoted` | a quote version is open | the quote is filed | each node, locally |
| `accepted` | the counterparty signed its half, the writer has not countersigned. The counterparty cannot withdraw or decline from here. | the counterparty's half is filed | each node, locally |
| `agreed` | both halves exist | the writer countersigns | writer |
| `closed` | every commitment is final | the last commitment becomes final | writer |
| `declined` | ended by a `deal-decline` (also an enquiry the counterparty withdrew) | the decline is filed | each node, from the signed decline |
| `expired` | the quote expired before the writer countersigned | the node's clock passes `quote_expires_at_secs`, and it holds no countersigned half signed before that time | each node, computed |
| `not-formed` | the writer refused the acceptance with `quote-expired` or an extension's reason, in `state_reason`. A refusal with `quote-superseded` or `quote-already-agreed` is not this state: it ends only that acceptance (see [Deal rules](#deal-rules)). | the writer files an acceptance it cannot take (see [Reason codes](#reason-codes)) | writer, in a snapshot with `seq` 1 |
| `cancelled` | the quote author cancelled, with a `cancel_reason` code | `agreement.cancel` | writer |

- The states before `agreed` come from signed records each node holds, and from each node's clock for `expired`, so each node can show them on its own. From `agreed` on, the writer's snapshot is the state.
- **One way out of an end state.** A countersigned half whose `issued_at_secs` is before `quote_expires_at_secs` moves the deal from `expired` to `agreed`, even when it arrives later. The writer never countersigns after the expiry by its own clock. It sends `not-formed` with `quote-expired` instead. A counterparty that released its reservations at the expiry and then gets a late countersigned half confirms as usual; a refused confirm ends in `reservation-lost`.
- The overview lists `requested → quoted → agreed → closed, or expired, or cancelled`. This document adds `accepted`, `declined` and `not-formed` (today's `conflict`), because each is a real end or wait that a person sees.
- Today's `scheduled` and `in-progress` are not deal-core states. They become step records of the scheduling extension (rule 1 of the extensions change).

#### Reason codes

Every code a deal can end with or a method can refuse with, in one place.

| Code | Meaning | Carried in |
| --- | --- | --- |
| `quote-superseded` | the accepted version is not the latest one held | `agreement.accept` error on the counterparty's node; a `not-formed` snapshot for that version only, when the writer finds it. The deal keeps its state. |
| `quote-already-agreed` | another version of this quote id already formed an agreement. The version check and "no revision after acceptance" leave only a bug or a restored backup as a cause. | `not-formed` snapshot for that version only. The deal stays `agreed`. |
| `quote-expired` | the writer filed the acceptance after the quote's expiry | `not-formed` snapshot |
| the extension's reason, for example `slot-taken` | the writer's own reserve failed | `not-formed` snapshot, as `state_reason` |
| `reserve-failed` | the counterparty's own reserve failed; the extension's reason comes with it | `agreement.accept` error |
| `reservation-lost` | a confirm was refused after the reservation expired | `commitment-failure`, then the commitment's `reason` |
| `other-commitment-failed` | another commitment of the agreement failed | the commitment's `reason` (state `cancelled`) |
| `quote-already-accepted` | the author tried to revise a quote that has an acceptance | `quote.set` error |
| `quote-ended` | the author tried to revise a quote id whose deal has ended | `quote.set` error |
| `writer-countersigns` | the quote author called `agreement.accept` on its own quote | `agreement.accept` error |
| `commitment-kind-not-supported`, `quote-too-large` | see [Commitment](#commitment) and [Where disclosures travel](#where-disclosures-travel) | `quote.set` error |
| `progress-fork` | two snapshots with the same `seq` and different content | a local warning on the counterparty's node |

#### Deal rules

- A deal is one quote id. One enquiry can lead to several deals. Each may form its own agreement, for example when the author offers two options and the counterparty takes both.
- **Every end of a deal ends its quote id:** `declined`, `expired`, `not-formed`, `cancelled` and `closed`. Only `quote-expired` and an extension's reserve or confirm reason make a deal `not-formed`. `quote.set` refuses a new version of that quote id with `quote-ended`. A new offer is a new quote id in the same conversation. The Hub's "send a new quote" action fills it from the old one, so for the person this is one click.
- **A refused acceptance is not an end.** When the writer refuses an acceptance with `quote-superseded` or `quote-already-agreed`, its snapshot names the refused version's `record_id` as `agreement`, and it ends only that acceptance. The counterparty's node releases the reservations it took for that version. The Hub shows the refusal on that version's card. The deal keeps its state: `quoted` when no version has formed an agreement, so the latest version can still be accepted, or `agreed` when one has.
- So the fence row `decision:<quote_id>` is written once and never reused.
- This differs from today, where a new version clears a decline (`quote_ops.rs:610`) and a `conflict` quote can be revised. One rule for every end is simpler, and it keeps the fence a single unique insert.

#### Commitments

Every commitment starts `promised` when the agreement forms. "Final" states are marked **(final)**.

| Kind | Path to success | Failure ends |
| --- | --- | --- |
| `fulfilment` | `promised` → `claimed` (giver's half) → `fulfilled` **(final)** (receiver's half) | `ended-unconfirmed` **(final)** at the track end; `failed` **(final)** with a reason; `cancelled` **(final)** |
| `currency` | `promised` → `payment-requested` (optional `payment-request`) → `claimed` (payer's half) → `acknowledged` **(final)** (payee's half) | `ended-unconfirmed` **(final)**; `cancelled` **(final)** |
| `eep-issue` | `promised` → `issued` **(final)** | `ended-unconfirmed`, `cancelled` (M08E may add more) |
| `eep-transfer` | `promised` → `endorsed` → `reissued` **(final)** | `refused` **(final)** with one of the VX-27 reasons; `cancelled` |
| `eep-return` | `promised` → `returned` → `closed` **(final)** | `ended-unconfirmed`, `cancelled` |

A `currency` commitment has no `failed` end, because its only reason, `reservation-lost`, cannot happen to it.

**Events.** The transition function takes one commitment's state and one event:

- `giver-half`: the giver's settlement half is filed.
- `receiver-half`: the receiver's settlement half is filed.
- `payment-request`: the receiver of a currency commitment files a `payment-request`.
- `tick`: the writer's sync runs after `track_ends_at_secs`.
- `cancel`: `agreement.cancel`. It is allowed only when every commitment of the agreement allows it.
- `failure`: a `commitment-failure` for this commitment, from its giver. Only for a commitment whose extension has a reserve hook.
- `other-failed`: another commitment of the same agreement became `failed`.

"No change" in a final state means: the record is kept as evidence, and the state stays. "Refused" means the call or the filing is refused. An event a kind does not list (for example `payment-request` on a `fulfilment`) is refused.

`fulfilment`:

| State | `giver-half` | `receiver-half` | `tick` | `cancel` | `failure` | `other-failed` |
| --- | --- | --- | --- | --- | --- | --- |
| `promised` | `claimed` | `fulfilled` | `ended-unconfirmed`, `claimed: false` | `cancelled` | `failed`, `reservation-lost` | `cancelled`, `other-commitment-failed` |
| `claimed` | no change | `fulfilled` | `ended-unconfirmed`, `claimed: true` | refused | refused | no change |
| any final | no change | no change | no change | refused | refused | no change |

`currency`:

| State | `payment-request` | `giver-half` | `receiver-half` | `tick` | `cancel` | `other-failed` |
| --- | --- | --- | --- | --- | --- | --- |
| `promised` | `payment-requested` | `claimed` | `acknowledged` | `ended-unconfirmed`, `claimed: false` | `cancelled` | `cancelled`, `other-commitment-failed` |
| `payment-requested` | no change | `claimed` | `acknowledged` | `ended-unconfirmed`, `claimed: false` | `cancelled` | `cancelled`, `other-commitment-failed` |
| `claimed` | no change | no change (a `supersedes` correction replaces the held half) | `acknowledged` | `ended-unconfirmed`, `claimed: true` | refused | no change |
| any final | no change | no change | no change | no change | refused | no change |

- The receiver's half settles a commitment even with no giver's half, as today. A payee can acknowledge a payment the payer never claimed.
- `cancelled` comes from `agreement.cancel` or from `other-failed`. Neither undoes a half.
- `failed` has one reason code in v1: `reservation-lost`.
- **When one commitment fails,** the writer cancels every other commitment that is still `promised` or `payment-requested`, with `other-commitment-failed`, in the same snapshot. Without this rule, a payment for a service that could not be given would end `ended-unconfirmed` after 30 days, and evidence would point at the payer. A commitment that already has a half keeps its own path: a payment already claimed may still be acknowledged. Refunds are outside Roym, as today.
- **Track window.** A commitment's track ends at `max(window.latest_secs, agreed_at_secs) + TRACK_WINDOW_SECS`. The 30 days is today's value. The `max` is a change: today the end is `window.latest_secs`, or the scheduling time when there is no window, plus 30 days (`booking.rs:189`). With no window, the new rule uses `agreed_at_secs`. A deal recorded after the work (a window that ended before the agreement) still gets 30 days. The writer applies a `tick` on each sync, as today. `track_ends_at_secs` is in the snapshot, so the other party sees the date.
- The EEP rows give the names from the overview. The obligations document fixes their records and may add failure ends.

### Single writer and keeping the counterparty's copy in step

- **The writer is the quote author's `transaction` service, always.** It writes the agreement row, every commitment state and every progress snapshot. This keeps today's rule. In Want-Bid the writer is the bidder, because the bidder writes the quote.
- **The counterparty writes only its own signed records:** its enquiry, its acceptance half, its declines, its settlement halves and its commitment failures. It sends each as a card.
- **The fence.** The writer takes one row, `decision:<quote_id>`, with a unique insert. The key is the quote id, not the version's `record_id`, so at most one version of a quote becomes an agreement. The full order of steps, with crash recovery, is in [The writer's decision](#the-writers-decision).
- **The version check.** `agreement.accept` on the counterparty's node refuses a version that is not the latest one it holds (`quote-superseded`). The Hub shows "Accept" only on the card of the latest version of a quote id. The writer repeats the check when it files the half, because a card can arrive late. Together with the fence and the rule below, this closes both paths of the gap in today's code (see [Facts checked in the code](#facts-checked-in-the-code)).
- **The writer never signs first.** `agreement.accept` on the quote author's own node is refused with `writer-countersigns`. The writer countersigns only in `transaction.sync`, after its decision. Today the author of a quote with no slot may accept first (`agreement_ops.rs:373`). This is removed, so every deal has one order.
- **No revision after acceptance.** `quote.set` refuses to revise a quote id once the writer holds an acceptance of any version of it (`quote-already-accepted`).
- **How the counterparty's node files the writer's records** (in `transaction.sync`, which reads the conversation, as today):
  - A quote, a countersigned half or a snapshot is filed only when the writer signed it.
  - A snapshot replaces the held one only when its `seq` is higher.
  - Two snapshots with the same `seq` and different content are a `progress-fork`. The node keeps the first, keeps the second as evidence, and shows a warning. The writer's own code never makes a fork. A fork means a bug or a restored backup.
  - A missing `seq` (a lost card) is not an error. The next snapshot carries the whole state.
- **The writer's `seq` after a restore.** During sync, the writer reads its own snapshot cards in the conversation and continues from the highest `seq` it finds there or holds. So a writer restored from an old backup does not reuse a `seq` the counterparty already holds.
- **How the writer files the counterparty's records:** only from the counterparty DID the quote names, and only with the role that fits (for example, a payee half of a currency commitment only from that commitment's receiver).
- **What each node shows.** From `agreed` on, both nodes show the writer's snapshot. Each node also holds every signed half, so a person can always open the signed facts behind a state.

### The writer's decision

When the writer's `transaction.sync` files an acceptance half, it runs these steps in this order:

1. **Checks.** The half is signed by the counterparty the quote names. The accepted version is the latest one the writer holds; otherwise refuse this acceptance with `quote-superseded` (no fence row is written; see [Deal rules](#deal-rules)). The writer's clock is before the quote's expiry; otherwise `not-formed` with `quote-expired`.
2. **Fence.** `create` the row `decision:<quote_id>` = `{ state: "deciding", agreement: <version record_id>, at_secs }` in the `ledger` collection. If the row exists with the same agreement, this is a repeat: continue from the row's state. If it exists with another agreement: refuse this acceptance with `quote-already-agreed`; the row does not change.
3. **Reserve.** For each commitment the writer gives whose extension has a reserve hook, call the extension's local `reserve`, keyed by (agreement id, commitment id). Reserve is idempotent (EX-21). This is not one write with step 2: the reservation is in the extension service's own data, and a data-layer write covers one collection of one service.
4. **Finish.**
   - If every reserve succeeded: `confirm` each reservation. If a confirm is refused (the reservation expired and was given away, EX-20, EX-22), `release` the others, put the decision row to `not-formed` with the extension's reason, and send a snapshot with `seq` 1 and state `not-formed`. No agreement has formed yet, because `confirm` comes before the countersign, so no `commitment-failure` is needed. Otherwise store the countersigned half with a unique insert keyed `countersign:<agreement>`; if that row exists, use the half already stored, so a repeat never signs twice. Put the decision row to `agreed`. Store and send the countersigned half and a snapshot with `seq` 1 and state `agreed`.
   - If one reserve failed: `release` the others. Put the decision row to `not-formed` with the reason. Send a snapshot with `seq` 1 and state `not-formed`.

**Recovery.** On every sync, the writer finishes each decision row that is still `deciding`. If no `countersign:<agreement>` row exists yet, it first checks the expiry again by its own clock: past it, it releases every reservation, puts the row to `not-formed` with `quote-expired` and sends that snapshot. Otherwise it runs steps 3 and 4 again. This is safe, because reserve, confirm and release are idempotent and the countersigned half is reused. Two syncs at the same time are safe for the same reasons: the second one finds the row in step 2 and repeats. A reservation left by a crash that is never finished expires on its owner's node (EX-20). Only the writer writes the decision row, so the later `put` in step 4 needs no fence.

### Reserve, confirm and release: the agreement side

The hooks themselves (their idempotency, expiry and the owner's clean-up) are the extensions change's (EX-5, EX-6, EX-20 to EX-22). This section says when the deal core calls them.

#### Who reserves

**The giver of a commitment owns its reservation**, when the commitment's extension has a reserve hook. The giver's node reserves, confirms and releases. No record field names the owner, because this rule needs none. A case where the receiver owns the reservation (for example a wanter that books its own venue) is not in v1. If M08F needs it, it adds a visible `reserved_by` field.

**Each owner reserves on its own node, at the moment it signs.** There is no reserve call between nodes.

1. **The counterparty gives a resource that needs a reservation** (for example a service in a booked slot). `agreement.accept` calls the extension's local `reserve` for each such commitment, before it signs. If one fails, it calls `release` on the ones that succeeded, signs nothing, and answers `reserve-failed` with the extension's reason.
2. **The writer gives such a resource** (the usual List-Search case, today's slot booking). The writer reserves in step 3 of [The writer's decision](#the-writers-decision). If one reserve fails, it releases the others, signs nothing, and sends a snapshot with state `not-formed` and the reason. The counterparty's node then releases its own reservations. The writer forms the agreement on its own node, so it confirms its own reservations at once. They never wait for an expiry.
3. **Confirm.** When the counterparty's node files the countersigned half, it calls `confirm` for its own reservations. If a confirm is refused (EX-22: expired, and the resource was given to someone else), it sends a `commitment-failure` with `reservation-lost`, and the writer marks that commitment `failed`.
4. **Release** also happens when the deal ends `expired`, `not-formed`, `declined` or `cancelled`. A lost release is harmless, because each reservation expires on its owner's node (EX-20).

This differs from the extensions change's text, which says the writer "calls reserve … on the resource owner's node". Calling another node would need a new wire-reachable method. Reserving at signing time gives the same result with none. The owner agreed to this (open question 5). The extensions change's text is updated at its close-out.

### Selective disclosure

#### What is hidden

The hidden fields are fixed in code, per object, as listed in the field tables above. The quote author cannot choose to make a price visible. This matches the overview's rule: outcomes are visible, amounts and payment details are hidden.

Visible always: ids, the two parties, the currency, the expiry, the home exchange, and for each commitment its id, giver, resource kind, commitment kind, extension and `after` list. Hidden always: the free text, the offering, every declared value, details, window, location and every payment detail.

#### The encoding

The format is SD-JWT's disclosure and digest format, from RFC 9901, "Selective Disclosure for JSON Web Tokens" (November 2025). We take its disclosure, digest and `_sd` rules exactly, and leave out its JWT wrapper. Our own rules on top of RFC 9901 are marked "Roym rule".

For each hidden field of an object:

1. **Salt.** 16 bytes from the host's secure random source, encoded base64url with no padding (22 characters). A fresh salt for every field of every version. Never derived from data. (Roym rule. RFC 9901, section 4.2.1, requires only a string, and recommends 128 bits.)
2. **Disclosure.** The JSON array `[salt, name, value]`, serialised as compact UTF-8 JSON (no spaces), then encoded base64url with no padding. `name` is the field name, for example `"declared_value_minor"`. `value` is the field's value, or `null`.
3. **Digest.** SHA-256 over the US-ASCII bytes of the disclosure string (the base64url text, not the decoded JSON), encoded base64url with no padding (43 characters).
4. **Place.** The field is left out of the object. Its digest goes into that object's `_sd` list. RFC 9901 allows an `_sd` list at any level (section 4.2.4.1). The list is sorted ascending by byte value, so its order says nothing about the order of the fields. (Roym rule: sorted. RFC 9901 allows sorting or shuffling. We sort, so the canonical bytes are stable.)
5. **Algorithm.** No record carries `_sd_alg`. RFC 9901, section 4.1.1, allows it only at the top level, and an absent `_sd_alg` means SHA-256. Our verifier uses SHA-256 only.

The digest is over the disclosure string as sent. So our canonicaliser (key-sorted `serde_json`, not RFC 8785) never has to reproduce the JSON inside a disclosure.

What we leave out of SD-JWT, and why:

| SD-JWT part | Here | Why |
| --- | --- | --- |
| JWS signature and the `~` combined format | our signed envelope; disclosures travel in the card | Roym records are already signed envelopes with delegation certificates. |
| Key binding JWT | not used | The presenter does not have to prove it holds a key. Any holder may present. This is not a goal here. |
| Decoy digests | not used | Every object always has the same hidden field names (with `null` values), so the number of digests already says nothing. |
| Array element disclosures (`{"...": digest}`) | not used | No list element is hidden on its own. `payment_methods` is hidden as a whole. |
| Nested disclosures | not used in v1 | `details` is hidden as a whole. |
| `_sd_alg` | not carried | An absent `_sd_alg` means SHA-256 (RFC 9901, section 4.1.1), the only algorithm we use. |

#### Where disclosures travel

- The card wrapper (`crates/roym_core/src/card.rs:37`) gains one optional field: `disclosures`, a list of strings, empty by default. The card rule "nothing derived from the envelope" still holds. A disclosure is not derived from the envelope. The receiver checks each one against a digest.
- The quote card carries every disclosure of the quote. The counterparty's node checks them and stores them with the quote.
- A receipt half card carries none. Both parties already hold the quote's disclosures.
- A settlement card carries the disclosures of its own hidden fields.
- **Size limits.** All disclosures of one record together are at most 96 KiB, encoded. The `details` disclosure of one commitment is at most 8 KiB, so one large `details` cannot use the whole limit. `quote.set` refuses a larger quote with `quote-too-large`. The Hub's quote form shows the size against the limit before the person signs. The payload holds only digests, so it stays at a few KiB, and 96 KiB leaves room for it and the envelope in the 160 KiB card body. C2's plan checks this number against the real envelope size.
- Both parties keep all disclosures in their backup bundle. Without them, a party cannot show its own deal.

#### Verifying

Given an envelope and some disclosures, the verifier:

1. verifies the envelope as today;
2. for each disclosure: decodes it, checks it is a 3-element array whose salt is a string, computes its digest, and finds that digest in exactly one `_sd` list;
3. refuses if a digest appears twice anywhere in the payload, if the name is not a hidden field of that object, if the name equals a visible field, or if the value fails the field's type and limits (a non-integer number fails);
4. returns the payload with the disclosed fields put back, and every other hidden field marked "hidden".

The counterparty's node also checks, before it accepts a quote, that it received a disclosure for every digest, that every salt is 16 bytes (Roym rule), and that no salt repeats inside the record. It cannot check that a salt is random. An author who wants to leak a price can do so anyway, by telling someone.

#### Showing a deal to a third party

```text
Presentation {
  presentation_version: 1
  records:      [ envelope ]   // a quote version, and/or one or both receipt halves;
                               // optionally the progress snapshot and settlement
                               // records of the same deal
  disclosures:  [ string ]     // the ones the presenter chooses
}
```

- The verifier checks that every record names the same deal. A quote and receipt halves must have equal `terms`. Receipt halves, the snapshot and settlement records must name the same agreement.
- A quote alone can be shown, before any agreement, for example to ask a contact "is this a fair price?" (VX-11, AG-34).
- A progress snapshot counts as the writer's only when it is bound as in [Progress snapshot](#progress-snapshot). Otherwise the verifier shows it as "writer not checked".
- **Field paths** name what to show: `terms.<field>` for a field of `terms`, `terms.commitments[<commitment_id>].<field>` for a commitment's field, and `<record_id>.<field>` for a hidden field of a settlement record. Examples: `terms.scope`, `terms.commitments[c2].declared_value_minor`.
- A presentation with no disclosures still shows each commitment's kind, giver and outcome. That is what reputation needs without consent (overview principle 9).
- **What cryptography cannot stop.** Both parties hold every disclosure. Either can show any hidden field. The overview's rule that amounts need the other party's grant is kept by honest clients and judged by reputation. It is not enforced by the format. The M08G disclosure ADR defines the grant.
- M08G evidence records will embed a presentation. This document fixes only its format.

### Mixed exchanges and declared values

Every agreement is an exchange of resources for resources. It holds 1 to 16 commitments, and each commitment gives one resource from one party to the other. Either side may hold any mix: services, goods, money, and EEPs from M08E. No rule depends on whether money is in the agreement.

- **One unit of account.** `terms.currency` is always required. Every declared value is stated in it, so the two sides can be compared and reported. The Hub fills it from the offering's currency.
- **One writer for the values.** The quote author writes every declared value, including the values of what the counterparty gives, because it is the single writer of the quote. The counterparty agrees to each value by signing its half. So both parties always report the same value for one commitment.
- **Ranges.** A declared value is 0 to 10^15 minor units. 0 means "given for free". A `currency` commitment's value is its amount, so it is at least 1. A free service is an agreement with no currency commitment (see open question 9), not a currency commitment of 0.
- **The two sides' totals need not be equal.** An uneven agreement can be a gift, a discount, or a fair trade that the declared values do not capture well. Before a party signs, and on the agreement screen after, the Hub shows that party:
  - the total it gives and the total it receives, in the unit of account;
  - when they differ, the difference and which way it goes, for example "You give ₹100 more than you receive".
  The difference is shown whenever the totals differ. There is no threshold. It never blocks signing.
- **The difference is computed, never stored.** It is a sum of values both parties already signed. A stored copy could disagree with them. An author who wants to explain a difference writes it in `scope`, or in the conversation.
- **What the counterparty may give.** An offering says what its owner accepts in return. In M08C the listing still has only today's `PaymentTerms` (`crates/roym_core/src/listing.rs:77`), which names one currency. So the Hub pre-fills one currency commitment from the counterparty. M08D replaces `PaymentTerms` with the in-return terms (accepted currency methods, goods or services, EEPs), and the Hub pre-fills from them. Either way this is only a starting point. No node checks a quote against the offering: the quote author owns both, and may agree to something its offering does not list.
- **Settled in another form.** At handover, the parties may agree to a different form than the agreement says, for example part cash and part goods. The receiver settles the commitment as usual and writes the real form in `settlement_note`. Because each resource has a value in the unit of account, the declared value usually stays right, and only the form differs. If the receiver accepts less than the declared value, the note says so. The statement still shows the declared value, next to the note, and the participant corrects it by hand if needed. Roym does not record a second value per commitment.
- An agreement with a booked slot on each side uses both reserve paths (writer and counterparty) from the section above.
- The statement lists each commitment with its declared value. Its totals per currency and direction show what the participant gave and received over the period.

### Event log and statement

#### Where the event log lives

**Decision: the event log of agreements lives inside `transaction`, and so does the statement export. The `obligation` service (M08E) keeps its own log of EEP events. The statement reads it through one local call.**

This confirms the overview's "inside `transaction`" for agreements and the statement. It rejects one shared log written by two services, for these reasons:

- **One writer per record.** If `obligation` appended to a log that `transaction` owns, the log would have two writers.
- **Complete by derivation.** A log entry is not written in the same call as the state change, because every data-layer write covers one collection. Instead, every `transaction.sync` runs the same derive step as `eventlog.rebuild`: it derives the entries from the records and snapshots the node holds, and creates the missing ones. A crash between a state change and its entry is repaired on the next sync. A log in another service could only be repaired by calling it on every sync.
- **No new dependency on the deal path.** `obligation` would otherwise call `transaction` on every EEP event. Now only the statement export, which is a read, calls `obligation`.

#### The log entry

Collection `event_log` in `transaction`. Entries are only ever created, never changed or deleted. Each entry's row id is derived from the event, so creating it twice is refused by the data layer, and a repeat is a no-op.

```text
EventLogEntry {
  id:               "<agreement>:<commitment_id or '-'>:<event>"   // row id; unique
  occurred_at_secs: u64           // the issued_at_secs of the signed record behind the event;
                                  // the same on both nodes (see below)
  recorded_at_secs: u64           // indexed; when this node created the entry; the list cursor
  event:            "agreement-formed" | "agreement-closed" | "agreement-cancelled"
                  | "agreement-not-formed" | "commitment-settled" | "commitment-failed"
  agreement:        "rec_…"
  conversation:     string
  counterparty_did: string        // the other party, seen from this node
  commitment_id:    string or absent
  direction:        "given" | "received" | absent   // for commitment events
  resource_kind:    string or absent
  commitment_kind:  string or absent
  final_state:      CommitmentState or absent        // e.g. "fulfilled", "ended-unconfirmed"
  currency:         string
  declared_value_minor: i64 or absent
  settlement_note:  LocalText or absent              // from the receiver's settling half
  records:          [ "rec_…" ]   // the signed records behind the event
}
```

- The entry is local data, not a signed record. The signed records it points at are the proof.
- The entry holds the declared value in plain form. It never leaves the node, except in the participant's own statement and backup.
- **Two times.** `occurred_at_secs` is the `issued_at_secs` of the signed record behind the event: the receiver's settling half for `commitment-settled`, the countersigned half for `agreement-formed`, and the writer's snapshot for the other events. So both parties give one event the same time. Statement periods and dates use it. `recorded_at_secs` is local, and only orders `eventlog.list`, so an old event that is appended late is not skipped by the cursor.
- **Who appends.** Each node appends for its own deals: the writer when it changes a state, the counterparty when it files a snapshot. The counterparty derives the events by comparing the new snapshot with the one it held, so a lost snapshot loses no event.
- **Nothing on the deal path reads the log** (AG-28). Readers are `eventlog.list`, `statement.export` and, later, policy hooks.
- **Rebuild.** Every entry can be derived from the records and snapshots a node holds. `eventlog.rebuild` derives them again and creates only the missing ones. It helps after a restore from an old backup.

#### The statement

`statement.export { from_secs, to_secs }` returns:

```text
Statement {
  statement_version:  1
  owner_did:          string
  from_secs, to_secs: u64          // [from, to)
  generated_at_secs:  u64
  rows: [ {
    agreement, commitment_id
    agreed_at_secs, settled_at_secs     // occurred_at_secs of agreement-formed, commitment-settled
    direction:            "given" | "received"
    counterparty_did:     string
    counterparty_name:    string or absent    // from the owner's contacts; not signed
    resource_kind, commitment_kind
    description:          LocalText           // the original text binds
    currency:             string
    declared_value_minor: i64
    settlement_note:      LocalText or absent // from the receiver's settling half
    records:              [ "rec_…" ]
  } ]
  open_at_end: [ … same row shape, without settled_at_secs … ]   // agreed, not final at to_secs
  totals: [ { currency, direction, resource_kind, sum_minor: i64 } ]
  eeps: EepSection
}
```

- A row is a commitment whose `commitment-settled` event has its `occurred_at_secs` in the period. Because this time comes from a signed record, both parties put one commitment in the same period and show the same dates. The receiver writes it, as the signer of the settling half, so it is that party's statement of when it received the resource.
- Failed and cancelled commitments are not given or received, so they are not rows. (The owner agreed to this. See open question 10.)
- Totals are per currency. Roym never converts between currencies.
- The Hub offers the statement as a CSV download with one line per row, plus a JSON download of the whole object.

#### The EEP hook

```text
EepSection =
  | { status: "not-available" }                       // no obligation service on this node
  | { status: "unavailable", reason: string }         // the call failed
  | { status: "available",
      outstanding_at_start: [ EepPosition ],
      outstanding_at_end:   [ EepPosition ],
      events:               [ EepEvent ] }             // shape fixed by the obligations doc

EepPosition {
  eep_id, version: u32
  role:              "issuer" | "holder"
  issuer_did, holder_did
  currency, face_value_minor: i64
  expires_at_secs
  record_id
}
```

- `statement.export` gets the section from one local call to the `obligation` service: `obligation.statement-part { from_secs, to_secs }`, which returns the `available` form. The obligations document owns that method. This document fixes only its output shape and the three states.
- Until M08E, there is no `obligation` service, so the section is `not-available`. The statement never shows an empty list in place of "not available" or "unavailable".
- An EEP commitment inside an agreement (for example `eep-issue`) is a row like any other commitment. The EEP section adds positions; it does not repeat those rows.

### Methods

Every method below is **local-only**: it answers only calls from the same node (`admit::require_internal`), as every `transaction` method does today. **This change adds no wire-reachable method.** All traffic between the two parties is Conversation cards, which the overview's channel table already names for deals. The reserve hooks are local calls on each owner's node (see above), so they need none either.

| Method | Status | What it does |
| --- | --- | --- |
| `enquiry.set`, `.get`, `.list`, `.history`, `.verify` | renamed from `request.*` | as today, with the new `description` shape |
| `quote.set` | changed | input: `answers`, `counterparty_did`, `terms` with plain values. The service picks salts, builds digests, signs, and sends the card with all disclosures. The service assigns the commitment ids. Refuses `quote-already-accepted`, `quote-ended`, `commitment-kind-not-supported`, `quote-too-large`. |
| `quote.get`, `.list`, `.history` | changed | the view holds the plain values for the two parties, from the stored disclosures |
| `quote.verify` | changed | input: an envelope and optional disclosures. Returns the verdict and the disclosed fields. |
| `quote.decline` | removed | replaced by `deal.decline` |
| `deal.decline` | new | signs and sends a `deal-decline` for a quote version, an enquiry, or one's own enquiry (a withdrawal) |
| `agreement.accept` | changed | counterparty only: refuses on the quote author's node (`writer-countersigns`) and for a version that is not the latest held (`quote-superseded`). Reserves this node's own resources, then signs and sends this party's half. Input adds `viewed_lang`, `viewed_via_mt`. |
| `agreement.get`, `.list` | changed | the view holds every commitment with its state, from the latest snapshot |
| `agreement.verify` | changed | input: an envelope and optional disclosures |
| `agreement.cancel` | new (from `booking.cancel`) | quote author only, with a `CancelReason` code, while every commitment is `promised` or `payment-requested` |
| `agreement.progress` | new (from `booking.get` and `booking.history`) | the snapshots of one agreement, latest first |
| `deal.present` | new | input: a quote id or an agreement id, the records to include, and the [field paths](#showing-a-deal-to-a-third-party) to show. Returns a `Presentation`. |
| `deal.verify-presentation` | new | verifies a `Presentation` from someone else and returns what it shows |
| `payment.request` | changed | input adds `commitment_id` |
| `payment.acknowledge` | changed | input adds `commitment_id`; `method` and `reference` become hidden fields |
| `payment.get`, `.verify` | changed | per commitment |
| `fulfilment.sign` | changed | input adds `commitment_id` |
| `fulfilment.get`, `.verify` | changed | per commitment |
| `booking.*` | removed | the booking track moves to the scheduling extension; the deal parts move to `agreement.*` |
| `eventlog.list` | new | input: `after` (a `{recorded_at_secs, id}` cursor, or absent) and `limit`. Pages the log in `(recorded_at_secs, id)` order. |
| `eventlog.rebuild` | new | creates missing log entries from held records; returns the count made |
| `statement.export` | new | see [The statement](#the-statement) |
| `transaction.sync`, `.thread`, `.export`, `.import`, `.signing-status`, `.install-signing-certificate` | changed inside | sync files the new cards; export and import carry disclosures and the event log |
| the `*.ping` methods | `booking.ping` removed; `deal.ping`, `eventlog.ping`, `statement.ping` added | as today |

The reserve, confirm and release methods are the extension's, not `transaction`'s. They are local-only too. Their names and inputs are set with the extension manifest (extensions change).

### Card types

| Card | Change |
| --- | --- |
| `enquiry` | renamed from `request` |
| `quote` | new shape; the card carries disclosures |
| `agreement-receipt` | new shape |
| `agreement-progress` | renamed from `booking-progress`, new shape |
| `payment-request`, `payment-acknowledgement`, `fulfilment-receipt` | new shapes, per commitment |
| `deal-decline` | new |
| `commitment-failure` | new |

With the three generic cards of the extensions change (`step`, `widget`, `widget-answer`), the fixed card table grows from seven types to twelve.

### Where other changes connect

- **Extensions.** A commitment names its extension and carries its `details`. A `step` record names an `agreement` and a `commitment_id` from this document. Reserve and confirm are called at the points above.
- **Message forwarding (FW-7).** A forwarded quote travels as its signed envelope. Its hidden fields stay hidden unless the forward carries disclosures. In M08C, the forward action carries none. Whether a private forward may show the price is the M08G disclosure ADR's question.
- **EEPs (M08E, M08H).** The three EEP kinds, the `eep` hidden field, AG-24 to AG-26 and the statement's EEP section are reserved here. The obligations document fixes the `eep` field's shape and the settlement records. One open point for it: whether an EEP's face-value currency must equal the agreement's currency. The rule "one currency per agreement" suggests it must.
- **Intents and wants (M08D, M08F).** `answers.kind = "want"` and `home_exchange` are filled by M08F, and their places in the record exist now. `listing_id` is renamed in place when M08D renames listings to intents. The product is not released, so that change needs no migration.
- **Reputation (M08G).** Evidence embeds a `Presentation`. The progress snapshot and the receipt halves show outcomes with no disclosure.
- **Multilingual (M08B).** `LocalText` uses M08B's tags and limits. `viewed_lang` and `viewed_via_mt` (ML-18) sit in each acceptance half.

### What replaces today's records

The product is not released. Every shape changes in place, `SCHEMA_VERSION` of `transaction` goes from 4 to 5, and stored data from version 4 is not read. Development nodes start empty. No migration and no compatibility code is written.

| Today | After |
| --- | --- |
| `request` record, `request.*` methods | `enquiry` record, `enquiry.*` methods |
| `AgreedTerms` (one amount, one scope, one slot) | `DealTerms` with 1 to 16 commitments and hidden fields |
| quote `lang` + `translations` list | a `LocalText` per text field |
| `PaymentTiming` (`before-work`, `after-work`) | `after` on each commitment |
| `QuoteLocation`, `schedule` on the quote | `location`, `window` on each commitment |
| `slot_id` on the quote | the scheduling extension's `details.data` |
| `payee`, `payment_methods`, `tax_minor`, `fees_minor` on the quote | hidden fields of each currency commitment |
| `agreement-receipt` with plain `terms` | `agreement-receipt` with the digest-only `terms`, plus `viewed_lang`, `viewed_via_mt` |
| `booking-progress`, `BookingState`, `TrackState` | `agreement-progress`, `AgreementState`, `CommitmentState`; `scheduled`/`in-progress` become scheduling steps |
| `payment-*`, `fulfilment-receipt` per agreement, with plain terms | per commitment, no plain terms |
| local `quote.decline` | signed `deal-decline` |
| ledger rows `decision:<agreement>`, `seat:…`, `step:…` | `decision:<quote_id>` in `transaction`; seat and step rows move to the scheduling extension |
| collections `bookings`, `progress` | `agreement_progress`; new `event_log` |
| `ONE_PAYMENT_NOTICE` (one payment per quote) | one payment per currency commitment |

The Hub's quote, agreement and booking screens, `roymctl roym transaction`, the parity harness and the backup bundle sections change with these shapes.

### Differences from the overview and sibling changes

Each row is a rule from another document that this document changes. The last column names the document that is updated at close-out, so it describes the result.

| Rule there | This document | Why | Updated at close-out |
| --- | --- | --- | --- |
| VX-45 and the overview's "event log": one log holds every agreement and EEP event | Agreement events are in `transaction`'s log. EEP events are in `obligation`'s own log. The statement joins both through `obligation.statement-part`. | One writer per log, and each log is repaired by its own derive step. | The overview's VX-45. The obligations document owns VX-45 for EEP events and needs its own test row for it. |
| Overview: "each field of the quote is hashed with its own random salt"; amounts are "hidden fields by default" | A fixed set of hidden fields per object. Ids, parties, kinds and structure stay visible. The author cannot change the set. | Reputation needs outcomes with no disclosure. A choice by the author could make the price visible. | Overview, selective-disclosure paragraph. |
| Overview agreement life: `requested → quoted → agreed → closed, or expired, or cancelled` | Adds `accepted`, `declined`, `not-formed`. Every end ends the quote id. | Each is a real wait or end that a person sees. One rule for every end. | Overview; integrated spec. |
| Overview term "declared value": "what the giver would charge" | The quote author writes every value, and the other party agrees by signing (open question 1a). | One writer for the quote. | Overview Terms. |
| Overview term "barter" | Not used. Every agreement has one shape (open question 1). | Owner decision. | Overview Terms and text. |
| Extensions EX-5 ("the deal core calls the reserve hook of each extension"), and its text that the writer reserves on the owner's node | Each giver reserves on its own node when it signs. The writer reserves inside its decision (open question 5). | No wire-reachable method. | Extensions change. |
| Extensions change: the card table grows to 10 types | It grows to 12, with `deal-decline` and `commitment-failure`. | Two new records. | Extensions change. |
| FW-7: a forwarded quote "verifies as it does today" | It verifies, but shows only visible fields: no scope, no description, no value (open question 11). | Amounts need the other party's grant. | Forwarding change, FW-7. |
| Multilingual change: one `translations` list per record | A `LocalText` per field, with one set of translation languages per record. | Each field can be hidden on its own. One set keeps `bestVersion` and `viewed_lang` working. | Multilingual change. |
| M08C `task.md`: demo "request, quote and agreement"; slice C6 "Barter"; the content of slices C1 to C3 | "enquiry" (open question 8); "mixed exchanges"; the slice cut in [Tasks](#tasks) (open question 12). | | M08C `task.md`. |
| Overview line reference `transaction.rs:192` for `AgreementReceiptPayload` | It is now `transaction.rs:234`. | The code moved. | Overview. |

### Rejected options

- **Our own disclosure format.** SD-JWT's is reviewed and used elsewhere. The overview also rules this out.
- **Letting the quote author choose which fields are hidden.** An author could make the price visible, and every receipt the counterparty shows would carry it. The hidden set is fixed in code.
- **Salting only the receipt.** The receipt points at the quote. Showing the quote would show everything. (Also rejected in the overview.)
- **Leaving settlement records as they are.** A fulfilment receipt carries the whole terms today. Showing it would show the price.
- **One digest over all hidden fields.** Then a party could show all or nothing. Reputation needs to show the scope without the price.
- **A remote reserve call from the writer to the other owner.** It needs a new wire-reachable method and a call to a node that may be asleep. Reserving at signing time needs neither.
- **The giver declares its own commitment's value.** Two writers for one quote. The single writer writes, and the other party accepts by signing.
- **A separate barter mode, record or flag.** Every rule would then need two versions. An agreement without money is the same shape with no `currency` commitment.
- **Requiring the two sides' totals to be equal.** It forbids gifts and discounts, and it would force parties to adjust declared values to pass a check. The values would then report less truth, not more.
- **Storing the difference between the two sides in the agreement.** It is computed from signed values, and a stored copy could disagree with them.
- **Checking a quote against the offering's in-return terms.** The quote author owns both and may agree to more. The overview also says acceptance rules are never published in detail, so another node could not check it fully.
- **One shared event log for `transaction` and `obligation`.** Two writers, and the derive step on each sync could not repair it without a call to the other service. See [Where the event log lives](#where-the-event-log-lives).
- **A statement built from a scan of all records at export time, with no log.** Possible, but every later reader (policy hooks, insight extensions) would repeat the scan, and the overview asks for a log.
- **Linking the scheduling extension into `transaction` as a library, so the decision and the seat rows are one write.** It puts slot code back into `transaction`, against EX-17. The ordered steps with recovery give the same safety.
- **Reusing a quote id after `not-formed` or `declined`.** The fence row would need a second state machine for reuse. A new quote id costs the person nothing.
- **A free `commitment_id` chosen by the author.** A visible free id could carry a price.
- **A per-commitment currency.** It breaks "one currency per agreement", and totals would need a conversion rule.

## Open questions

Each has a recommendation. The owner decides. Questions 1 and 3 to 12 are decided. Question 2 belongs to a C0 ADR.

1. **Declared values in a mixed exchange.**
   Owner direction: there is no barter special case. An exchange is any mix of goods, services, EEPs and money. The offering says what its owner accepts in return. Each resource has a value in the unit of account. The totals need not match, and the Hub shows the difference.
   - **a. Who writes the value of what the counterparty gives?** The quote author writes every value, and the counterparty agrees by signing. Each commitment has one value, which both parties report. Rejected: a second, private value per party, because both parties normally agree on the value.
   - **b. Where does the difference appear?** Only on the Hub's screens, computed from the declared values. No record stores it. Rejected: a stored field (it could disagree with the values) and a separate free-text reason (`scope` and the conversation already hold such text).
   - **c. A handover in another form than agreed.** The receiver's settlement record carries an optional hidden `settlement_note`, and the declared value does not change. Rejected: a "value actually received" field (a second value per commitment, with its own signing rules; add it only if participants need exact statements), and changing the agreement after it formed (it undoes the rule that an agreed quote is fixed).
   Owner decision (2026-11-10): agreed, a, b and c as above.

2. **The forward mode default: `signed` or `not-forwardable`.**
   This belongs to C0's ADR on signed message origins. It is listed here only as a cross-reference. The forwarding change recommends `not-forwardable`.

3. **The extension manifest format, and where installed extensions are recorded.**
   Recommendation: a TOML manifest per extension, in the extension's crate, read at build time. The registry is a fixed list in `roym_core` of the extensions that ship with Roym. There is no runtime record of installed extensions in M8. An offering names its extension by `id` and `version`.
   Reason: M8 runs only extensions that ship with Roym, so a runtime registry would have no writer and nothing to install. A runtime record is added when third-party extensions are allowed.
   Owner decision (2026-11-10): agreed.

4. **Is a 15-minute reservation expiry (EX-20) acceptable when a phone sleeps right after it reserves?**
   Recommendation: yes, keep 15 minutes as the scheduling extension's value.
   Reason: with "each owner reserves when it signs", the common case (List-Search, the writer owns the slot) confirms on the same node at once and never waits for an expiry. The risk exists only when the counterparty owns a reserved resource and the writer does not countersign within 15 minutes. That case ends with a named `reservation-lost` failure, not a silent one. If it proves common, raise the value per extension. Do not tie it to the quote expiry (up to 90 days), which would block a slot for too long.
   Owner decision (2026-11-10): agreed.

5. **Reserve at signing time, or the extensions change's remote reserve call?** (New.)
   Recommendation: reserve at signing time, as this document designs, and update the extensions change's text at its close-out.
   Reason: no new wire-reachable method, and no call to a node that may be asleep. EX-5, EX-6 and EX-20 to EX-22 all still hold.
   Owner decision (2026-11-10): agreed.

6. **Which fields stay visible?** (New.)
   Recommendation: the list in [What is hidden](#what-is-hidden). In particular, keep `extension` visible and hide `listing_id`.
   Reason: a reader of evidence needs the commitment kind and outcome. The extension id ("scheduling") says little, and every node needs it to know who must reserve. The offering says exactly what was bought, which the subject may not want a stranger to see. If reputation later needs a category per commitment, add a visible taxonomy code then.
   Owner decision (2026-11-10): agreed.

7. **A signed `deal-decline` in place of today's local decline.** (New.)
   Recommendation: yes.
   Reason: VX-3 and VX-5 say each side "accepts or rejects". Today the quote author never learns of a decline, and its deal stays open until the quote expires.
   Owner decision (2026-11-10): agreed.

8. **Rename the `request` record to `enquiry`.** (New.)
   Recommendation: yes, in C1, together with the other shape changes.
   Reason: the overview's term is "enquiry". C1 changes the record's shape anyway, so the rename costs nothing extra now and more later.
   Owner decision (2026-11-10): agreed.

9. **May a deal have commitments on one side only?** (New.)
   Recommendation: yes. A quote needs at least one commitment in total, not one per side.
   Reason: today a quote with `amount_minor = 0` is valid (a free service). Requiring a commitment on each side would force a fake zero-value commitment.
   Owner decision (2026-11-10): agreed.

10. **Which commitments does a statement period contain?** (New.)
    Recommendation: a commitment is in the period when it settled in the period. Commitments agreed but not final at the end are listed apart (`open_at_end`). Failed and cancelled ones are not listed as rows.
    Reason: they were not given or received. The settlement time is the `issued_at_secs` of the receiver's settling half, so both parties use the same time (refined in review round 1). Accountants who need the agreement date have `agreed_at_secs` on every row.
    Owner decision (2026-11-10): agreed.

11. **Private forwards of a quote with the price shown.** (New.)
    Recommendation: in M08C, a forwarded quote carries no disclosures. The M08G disclosure ADR decides whether a private forward to a contact may show amounts.
    Reason: the overview's rule is that a third party sees amounts only with the other party's grant. A forward is the first place this rule is felt. It is better decided once, in the ADR, than quietly in the forward action.
    Owner decision (2026-11-10): agreed.

12. **How to cut slices C1 to C3.** (New, review round 1.)
    The M08C `task.md` puts the commitment model in C1, disclosure in C2, and the move of slots to the scheduling extension in C3. C1's shapes have hidden fields, but the encoder is in C2. C1 removes the booking track, but the extension that replaces it is in C3.
    Recommendation: C1 ships the commitment model with plain `DealTerms` (no `_sd` yet) and keeps today's slot path inside `transaction`, behind the new shapes, so booking keeps working. C2 adds `_sd`, the encoder, the hidden fields of every record and `deal.present`. C3 moves slots to the scheduling extension and adds `commitment-failure`.
    Reason: each slice ends with a working system and stays a size one review can cover. The cost is that C2 changes C1's canonical bytes once. The product is not released, so only test fixtures change. The other option is to merge C1 and C2 into one larger slice.
    Owner decision (2026-11-10): merge C1 and C2 if the merged slice stays manageable; otherwise keep them apart.
    Applied: keep them apart. C1 alone already carries about 25 acceptance rows (AG-1 to AG-15, AG-22, AG-23, AG-31 to AG-33, AG-35 to AG-37, AG-39, AG-41 to AG-43): every record shape, the state tables, the writer's decision and settlement. C2 adds 8 more (AG-17 to AG-21, AG-34, AG-38, AG-40) and a second hard area, the RFC 9901 encoding and its test vectors. One review should not have to check the state machine and the byte encoding at the same time.
    Check again when the C1 slice plan is written: if C1's plan turns out small (for example, if the settlement records or `deal-decline` move to a later slice), fold C2 into it.

## What the C0 ADRs must decide

This document does not write the two ADRs. They must decide:

**Deal grants** (counterparty access to an extension service):

- What the token is (the extensions change proposes a UCAN-style capability from `crates/ucan`) and who signs it: the person or the node's service key.
- What it names: the agreement id from this document, the counterparty DID, the methods, an expiry.
- When it is issued (when the writer countersigns, or when the agreement forms on each owner's node) and when it ends (when the agreement reaches `closed`, `cancelled` or `not-formed`), and whether the owner can extend it.
- How a call with a grant is admitted on the owner's node, given that every Roym method is local-only today. This is the first wire-reachable path into an extension service.
- Whether a grant can name a conversation with no agreement (the extensions change recommends not in M8).

**Signed message origins and the forward mode:**

- The shipped default mode: `signed` or `not-forwardable` (the forwarding change recommends `not-forwardable`).
- Whether a forwarded Roym record (a quote) follows the message's forward mode, or always travels as itself (FW-7), and what a forward of a quote shows given its hidden fields.
- The size of a `message-origin` with its certificate, and whether the certificate is carried once per conversation.

## Tasks

- [ ] Owner review of this draft and the open questions.
- [ ] Write the two C0 ADRs (owner decides).
- [ ] C1: commitment model with plain `DealTerms`, records, states and transition tables, the writer's decision, the version check, `deal-decline`, per-commitment settlement records. Today's slot path stays inside `transaction` behind the new shapes. Replace today's shapes in place. (Slice cut: open question 12; kept apart from C2.)
- [ ] C2: `_sd` and the hidden fields of every record, the disclosure encoder and verifier, `deal.present`; disclosures in the card wrapper and the backup bundle.
- [ ] C3 (with the extensions change): move slots to the scheduling extension; the reserve, confirm and release calls at the points above; `commitment-failure`.
- [ ] C6: mixed exchanges in the Hub (totals and difference), the event log, `eventlog.*`, `statement.export` with the EEP section, the Hub's statement screen.
- [ ] Demo story for enquiry, quote and agreement, as the M08C `task.md` asks.

## Deviations

## Close-out

Before merge, update the living docs (requirements, architecture, integrated spec, developer guide, terminology) so they describe the result. Then set `status: done` and fill in `living-docs-touched` and `demo`. Record anything you postponed in [deferred-backlog.md](../../deferred-backlog.md).

Expected living-doc changes:

- `docs/roym-integrated-experience-spec.md`: the Records table, the transaction state diagram, the cancellation and booking rules (`D-C8-2`, `D-C8-14`), and the payment rule (one payment per currency commitment).
- `docs/TERMINOLOGY.md`: enquiry, commitment, declared value, disclosure, presentation, progress snapshot, event log, statement.
- `docs/system-architecture.md`: the `transaction` service's collections and the event log.
- `docs/developer-guide.md`: the client guide's card section (new card types and the `disclosures` field).
