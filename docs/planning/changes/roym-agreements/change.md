---
status: draft
living-docs-touched:
demo:
---
# Roym agreements

*Reader: developers who plan and build M08C (deal core and extensions), and the project owner, who makes the product decisions listed under [Open questions](#open-questions).*

This is the **agreements module** change document of the [Roym Value Exchange](../roym-value-exchange/change.md) (the overview). The overview gives the rules. This document gives the exact formats: the records, their fields and limits, the states, the disclosure encoding, the event log, the statement and the methods. It follows the overview's rules. Where it refines or differs from the overview, it says so, and the difference is listed in the report to the owner, not fixed in the overview.

Related documents:

- [Roym extensions](../roym-extensions/change.md): the `details` block, step records, and the reserve, confirm and release hooks. This document defines only the points where they connect to agreements.
- [Roym message forwarding](../roym-message-forwarding/change.md): forwarding a quote as itself (FW-7).
- [Roym multilingual foundation](../roym-multilingual/change.md): language tags, author translations, and the two "what I viewed" fields of an acceptance (ML-18).
- [Integrated experience spec](../../../roym-integrated-experience-spec.md): today's transaction rules. Terms such as **quote author** and **counterparty** are defined in [TERMINOLOGY.md](../../../TERMINOLOGY.md).

## Terms

Terms from the overview (participant, resource, resource kind, commitment, commitment kind, declared value, unit of account, enquiry, bid, quote author, agreement, economic event, event log, statement, EEP) keep their meaning. This document does not use the overview's term "barter". Every agreement has one shape, with any mix of resources on each side, and an agreement without money follows no special rule. New terms here:

- **Hub:** the Roym app screens a participant uses. Each participant's own node serves them, from its `web` service (`crates/roym_web/ui`). The Hub is not a shared server or meeting place. When this document says "the Hub shows", it means the participant's own app computes and shows it.
- **Deal:** one enquiry, the quotes that answer it, and the agreement if one forms. A deal is named by its quote id (`quo_…`), which stays the same across quote versions.
- **Agreement id:** the `record_id` (`rec_…`) of the quote version that both parties accepted. Today's code already uses this value as the agreement id (`payment.agreement`, `fulfilment.agreement`).
- **Writer:** the quote author's `transaction` service. It is the only node that writes the agreement's state.
- **Progress snapshot:** a signed record, written only by the writer, that states the agreement's state and each commitment's state at one moment. It replaces today's `booking-progress`.
- **Half:** one party's signed record of a two-party fact, for example one side of the agreement receipt. "Signed by both" means one half from each party, as today.
- **Hidden field:** a field of a quote whose value is not in the signed record. The record holds only a hash of it.
- **Salt:** a random value mixed into a hidden field's hash, so the value cannot be guessed from the hash.
- **Disclosure:** the salt, the field name and the value of one hidden field, encoded as one string. Whoever holds a disclosure can show that field to someone else. The format is SD-JWT's (see [Selective disclosure](#selective-disclosure)).
- **Digest:** the hash of one disclosure. The signed record holds the digests.
- **Presentation:** what a party gives a third party to show a deal: signed records plus the disclosures it chooses.
- **Reservation:** a short hold on a resource (for example a seat in a slot), taken by the resource owner's node. See [Roym extensions](../roym-extensions/change.md).

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
| AG-3 | A quote that answers a want and a quote that answers an enquiry produce the same agreement receipt and the same progress snapshot. The receipt does not carry the `answers` field (VX-6). M08C builds only enquiries. M08F adds the Want-Bid end-to-end test. | Unit: build both quote forms from the same terms and compare the receipts byte for byte. |
| **Agreements and commitments** | | |
| AG-4 | One agreement holds 1 to 16 commitments. Each names its giver, its resource kind, its commitment kind and its declared value. All declared values are in the agreement's one currency (VX-7). | Unit: validation table (0, 1, 16, 17 commitments; missing fields). Two-node: an agreement with a service, a currency commitment and a goods commitment forms and closes. |
| AG-5 | No commitment carries its own currency. A quote with a currency that `money::currency_minor_exponent` does not know is refused (one currency per agreement). | Unit. |
| AG-6 | Each commitment moves only through the states of its kind, and every kind has a failure end. The agreement closes only when every commitment is final (VX-8). | Unit: a table test of the pure transition function over every (state, event) pair, for every kind. |
| AG-7 | A commitment that has not settled when its track window ends becomes `ended-unconfirmed`. The progress snapshot names which party's half is missing (VX-8, VX-12). | Service: controllable clock. |
| AG-8 | Each final failure state names the party it is about: `ended-unconfirmed` names the role whose half is missing, and `failed` names the giver and a reason code. A reader can see this with no disclosures (VX-12). | Unit: verify a presentation that holds only the progress snapshot and the receipt halves. |
| AG-9 | Only the quote author cancels, and only while no half of any commitment exists. A later cancel is refused with `commitment-moved`. | Service. |
| AG-10 | Either party can decline: the counterparty declines a quote, the quote author declines an enquiry. The decline is a signed `deal-decline` record. The writer moves the deal to `declined`. A repeated decline returns the same record. | Two-node. |
| **Single writer** | | |
| AG-11 | Only the writer writes the agreement state. The counterparty's node files a progress snapshot only when the writer signed it, and only when its `seq` is higher than the one it holds (VX-9). | Unit and service. |
| AG-12 | A second, conflicting change gets a named conflict, never a merge: accepting a quote version that a newer version superseded → `quote-superseded`; accepting a second version of a quote that already formed an agreement → `quote-already-agreed`; two snapshots with the same `seq` and different content → `progress-fork`, and the first one is kept (VX-9). | Two-node for the first two; unit for the third. |
| AG-13 | Every state-changing call is idempotent. A repeated `agreement.accept`, `fulfilment.sign`, `payment.acknowledge` or `deal.decline` returns the record already made (today's `D-C8-6`). | Service. |
| **Reserve, confirm, release (agreement side)** | | |
| AG-14 | When the writer owns a reserved resource, it reserves it under its own fence when it files the counterparty's acceptance. Two counterparties who accept quotes for the last seat: one agreement reaches `agreed`, the other ends `not-formed` with reason `slot-taken` (VX-10, EX-6). | Two-node: today's booking race test, moved to the scheduling extension. |
| AG-15 | When the counterparty owns a reserved resource (for example it gives a service in a booked slot, or it made a bid), its node reserves it before it signs its acceptance. If the reserve fails, it signs nothing, and `agreement.accept` answers `reserve-failed` with the extension's reason (VX-10, EX-5). | Service. |
| AG-16 | If a non-writer owner's confirm is refused after its reservation expired and the resource was given away, the owner's node sends a signed `commitment-failure`. The writer marks that commitment `failed` with reason `reservation-lost` (EX-22). | Two-node with a controllable clock. |
| **Selective disclosure** | | |
| AG-17 | The quote, the agreement receipt and the settlement records carry no hidden value in plain form. A test serialises each signed payload and checks that no hidden value appears in it (VX-11). | Unit. |
| AG-18 | A party shows its agreement to a third party with the price hidden and the scope shown. The third party's node verifies both receipt halves and the shown field (VX-11). | Unit, and two-node with a third node that only verifies. |
| AG-19 | The encoder and verifier follow SD-JWT: our digest function gives the result of the worked example in the SD-JWT specification. The verifier refuses a disclosure whose digest is not in the record, a digest that appears twice, a name that is not a hidden field of that object, a name that equals a visible field, and a value with a non-integer number. | Unit: test vectors. |
| AG-20 | Both receipt halves carry a `terms` object that is byte-identical, after canonicalisation, to the quote's `terms`. A half whose `terms` differ is refused when it is filed. | Unit. |
| AG-21 | Every quote version uses fresh salts. Each salt is 16 bytes. The counterparty's node refuses a quote with a salt of another length or a salt used twice. | Unit. |
| **Mixed exchanges and declared values** | | |
| AG-22 | An agreement with no currency commitment forms and closes through the same code path as one with money. It still names a currency, and every commitment still has a declared value. No validator, state or card checks whether a currency commitment exists. | Two-node, and a unit test that runs the same quote with and without a currency commitment. |
| AG-23 | A declared value is an integer from 0 to 10^15 minor units. A `currency` commitment's value is at least 1. Before a party signs, the Hub shows the declared value of every commitment, the total that party gives, the total it receives, and the difference when they are not equal. A difference never blocks signing. No record stores the difference. | Unit for the range; E2E for the Hub, with one agreement whose totals differ. |
| AG-33 | The receiver can settle a commitment with a `settlement_note` that says how it was really handed over. The commitment settles as usual, its declared value does not change, and `statement.export` shows the note on that commitment's row. The note is hidden like every other free text (AG-17). | Service, and unit for AG-17's check on the new field. |
| **EEP rules checked on the agreement side (built in M08E and M08H)** | | |
| AG-24 | **One node only.** The issuer's node refuses to sign a quote or an acceptance that makes it the giver of an `eep-issue` commitment while it has no published offering (VX-13). The receiving node cannot know the issuer's full offering list. It shows "not checked" unless it found an offering of the issuer that is valid now. | Service, on the issuer's node. Built in M08E. |
| AG-25 | **One node only.** The issuer's node refuses an `eep-issue` commitment whose face-value currency does not appear in its own published offerings, or whose face value is not an integer in minor units (VX-14). The receiving node checks only the integer rule, and shows the currency rule as "not checked". | Service, on the issuer's node. Built in M08E. |
| AG-26 | **Reference code only.** An `eep-transfer` commitment ends `refused` only with one of the three refusal reasons of VX-27, read from the issuer's signed refusal. The rule that an issuer refuses only for these reasons cannot be checked from another node. The test checks Roym's own reissue code: the refusal reason type has exactly three values, and no other code path refuses. | Unit, against the `obligation` code. Built in M08H. |
| **Event log and statement** | | |
| AG-27 | Every agreement event this node takes part in is appended to its event log exactly once. Running `transaction.sync` again, or `eventlog.rebuild`, adds no duplicate (VX-45). | Service. |
| AG-28 | Nothing on the deal path reads the event log. A full deal runs under a test host that records every data-layer read, and no read touches the `event_log` collection (VX-45). | Service. |
| AG-29 | `statement.export` for a period lists every commitment settled in that period, given and received, with resource kind, commitment kind, counterparty, currency and declared value, and totals per currency. Commitments of agreements with and without money appear the same way (VX-46). | Service, and E2E for the Hub's export. |
| AG-30 | The statement has an `eeps` section. Before the `obligation` service exists, it says `not-available`. With an `obligation` sibling, it lists the EEPs outstanding (issued and held) at the start and at the end of the period. If the sibling call fails, it says `unavailable` with the reason. It is never left out and never empty in place of an error. | Service, with a fake `obligation` sibling. |
| **Methods and language** | | |
| AG-31 | Every `transaction` method answers `-32013` when called from another node. | Service: extend the admission test table. |
| AG-32 | Each acceptance half carries `viewed_lang` and `viewed_via_mt` outside `terms`. The two halves of one agreement may differ only in `role`, `viewed_lang` and `viewed_via_mt` (ML-18, kept). | Unit. |

## Design

### Facts checked in the code

The design builds on these facts about today's code (checked 2026-11-10 on `main` at `4830bdf7`):

| Fact | Where |
| --- | --- |
| A quote's id is `content_digest("quo_", {conversation, issuer, sequence})`, so it stays the same across versions. A version supersedes the previous one. | `crates/roym_core/src/transaction.rs:524`, `crates/roym_transaction/src/app/quote_ops.rs:285` |
| The agreement is two `agreement-receipt` halves over the quote's `record_id`. The counterparty signs first. The quote author's node countersigns during `transaction.sync`, after it decides the booking. | `crates/roym_transaction/src/app/agreement_ops.rs:99`, `:402` |
| The writer's fence is a unique insert: `AppDataLayer::create` writes rows `decision:<agreement>`, `seat:<slot>:<n>` and `step:<agreement>:1` together, or none, and names the first row that already exists. | `crates/roym_transaction/src/app/ledger.rs:52`, `:134` |
| `booking-progress` is signed by the node's service key (`Principal::Service`), not by the person. The reader binds it to the quote's signer. | `crates/roym_transaction/src/app/progress.rs:32`, `crates/roym_core/src/booking.rs:327` |
| A track settles when the party that receives the resource signs its half ("against interest"). The giver's half alone is a claim. | `crates/roym_core/src/booking.rs:260` |
| Declining a quote is local and unsigned. The quote author is never told. | `crates/roym_transaction/src/app/quote_ops.rs:508` |
| `quote.set` lets the author revise a quote that was already accepted, and the decision row is keyed by the agreement (the version's `record_id`), not by the quote id. From reading the code (not run), two versions of one quote can both become agreements. | `quote_ops.rs:285`, `ledger.rs:52` |
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
                                                   unique insert decision:<quote_id>
                                                   reserve own resources under its fence
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
- **Ids.** `quote_id` and `enquiry_id` are derived as today. A `commitment_id` is chosen by the quote author: 1 to 16 characters of `[a-z0-9-]`, unique inside the quote. The author should keep a commitment's id across versions, so the Hub can show what changed. This is not enforced.
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
| `_sd_alg` | string | always `"sha-256"` | visible |
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
| `commitment_id` | string | `[a-z0-9-]{1,16}`, unique in the quote | visible |
| `giver` | `"quote-author"` or `"counterparty"` | | visible. The receiver is the other party. |
| `resource_kind` | see the kind table | | visible |
| `kind` | see the kind table | | visible |
| `extension` | `{ id: string, version: u32 }` or absent | id `[a-z0-9-]{1,32}` | visible |
| `after` | list of `commitment_id` | 0 to 15, each in this quote, no cycles | visible. A hint for the Hub ("pay after the work"), never a gate. Replaces `payment_timing`. |
| `_sd` | list of strings | exactly 10, sorted ascending | the digests of the ten hidden fields below |
| `description` | `LocalText` | 2048 bytes | hidden. What is given. |
| `declared_value_minor` | i64 | 0 to 10^15; at least 1 for `currency` | hidden |
| `details` | `Details` or null | inside the 64 KiB payload limit | hidden |
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

- The counterparty declines a quote. The quote author declines an enquiry. Neither can decline after the agreement forms. After that, only the quote author cancels.
- It replaces today's local, unsigned `quote.decline`. The other party now learns of it, and the deal ends `declined` on both nodes.

### Progress snapshot

Record type `agreement-progress`, version 1. It replaces `booking-progress`. Signed by the writer's service key, and bound to the writer as `booking-progress` is today. It holds no hidden value, so it can be shown to anyone.

```text
AgreementProgressPayload {
  agreement:          "rec_…"
  conversation:       string
  quote_author_did:   string
  counterparty_did:   string
  seq:                u32                 // from 1, raised by each change
  state:              AgreementState      // see States
  state_reason:       string or absent    // a reason code, e.g. "slot-taken"
  cancel_reason:      string or absent    // 512 bytes, the quote author's text
  commitments: [ {
    commitment_id:        string
    state:                CommitmentState
    reason:               string or absent   // a reason code for "failed"
    missing_half:         role or absent     // for "ended-unconfirmed": whose half is missing
    halves:               [ "rec_…" ]        // the settlement records the writer holds
    track_ends_at_secs:   u64
  } ]
}
```

### Commitment failure

New record type `commitment-failure`, version 1. Signed by the giver of one commitment. In v1 its only use is EX-22: a resource owner that is not the writer could not confirm its reservation.

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

| Record | Signed by | Fields | Hidden fields |
| --- | --- | --- | --- |
| `payment-request` v1 | the receiver of a currency commitment | `agreement`, `conversation`, `commitment_id` | `note` (`LocalText` or null, 512 bytes) |
| `payment-acknowledgement` v1 | the payer (a claim) or the payee (settles) | `agreement`, `conversation`, `commitment_id`, `role`, `observed_at_secs` | `method` (code or null), `reference` (string or null, 512 bytes), `settlement_note` |
| `fulfilment-receipt` v1 | the giver (a claim) or the receiver (settles) | `agreement`, `conversation`, `commitment_id`, `role` | `settlement_note` |

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
| `deal-decline` | declines an enquiry | declines a quote | |
| `agreement-progress` | | | signs |
| `commitment-failure` | as giver | as giver | |
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
| `requested` | an enquiry exists, no quote | the enquiry is filed | each node, locally |
| `quoted` | a quote version is open | the quote is filed | each node, locally |
| `accepted` | the counterparty signed its half, the writer has not countersigned | the counterparty's half is filed | each node, locally |
| `agreed` | both halves exist | the writer countersigns | writer |
| `closed` | every commitment is final | the last commitment becomes final | writer |
| `declined` | ended by a `deal-decline` | the decline is filed | each node, from the signed decline |
| `expired` | the quote expired before the writer countersigned | the writer's clock passes `quote_expires_at_secs` with no countersign | writer |
| `not-formed` | a reserve failed, with `state_reason` | the writer's reserve fails | writer |
| `cancelled` | the quote author cancelled, with `cancel_reason` | `agreement.cancel` | writer |

- The states before `agreed` come from signed records each node holds, so each node can show them on its own. From `agreed` on, the writer's snapshot is the state.
- The overview lists `requested → quoted → agreed → closed, or expired, or cancelled`. This document adds `accepted`, `declined` and `not-formed` (today's `conflict`), because each is a real end or wait that a person sees.
- Today's `scheduled` and `in-progress` are not deal-core states. They become step records of the scheduling extension (rule 1 of the extensions change).

#### Commitments

Every commitment starts `promised` when the agreement forms. "Final" states are marked **(final)**.

| Kind | Path to success | Failure ends |
| --- | --- | --- |
| `fulfilment` | `promised` → `claimed` (giver's half) → `fulfilled` **(final)** (receiver's half) | `ended-unconfirmed` **(final)** at the track end; `failed` **(final)** with a reason; `cancelled` **(final)** |
| `currency` | `promised` → `payment-requested` (optional `payment-request`) → `claimed` (payer's half) → `acknowledged` **(final)** (payee's half) | the same three |
| `eep-issue` | `promised` → `issued` **(final)** | `ended-unconfirmed`, `cancelled` (M08E may add more) |
| `eep-transfer` | `promised` → `endorsed` → `reissued` **(final)** | `refused` **(final)** with one of the VX-27 reasons; `cancelled` |
| `eep-return` | `promised` → `returned` → `closed` **(final)** | `ended-unconfirmed`, `cancelled` |

- The receiver's half settles a commitment even with no giver's half, as today. A payee can acknowledge a payment the payer never claimed.
- `cancelled` happens only through `agreement.cancel`, which is allowed only while every commitment is `promised` (or `payment-requested`). So a cancel never undoes a half.
- `failed` has one reason code in v1: `reservation-lost`.
- **Track window.** A commitment's track ends at `max(window.latest_secs, agreed_at_secs) + TRACK_WINDOW_SECS` (30 days, today's value). The writer applies a tick event on each sync, as today. `track_ends_at_secs` is in the snapshot, so the other party sees the date.
- The EEP rows give the names from the overview. The obligations document fixes their records and may add failure ends.

### Single writer and keeping the counterparty's copy in step

- **The writer is the quote author's `transaction` service, always.** It writes the agreement row, every commitment state and every progress snapshot. This keeps today's rule. In Want-Bid the writer is the bidder, because the bidder writes the quote.
- **The counterparty writes only its own signed records:** its enquiry, its acceptance half, its declines, its settlement halves and its commitment failures. It sends each as a card.
- **The fence.** When the writer files an acceptance half, it inserts a row `decision:<quote_id>`, together with its own reservation rows, in one `AppDataLayer::create` call. The key is the quote id, not the version's `record_id`, so at most one version of a quote becomes an agreement. A later acceptance of another version gets `quote-already-agreed`. An acceptance of a version that a newer one supersedes gets `quote-superseded`. This closes the gap found in today's code (see [Facts checked in the code](#facts-checked-in-the-code)).
- **No revision after acceptance.** `quote.set` refuses to revise a quote id once the writer holds an acceptance of any version of it (`quote-already-accepted`).
- **How the counterparty's node files the writer's records** (in `transaction.sync`, which reads the conversation, as today):
  - A quote, a countersigned half or a snapshot is filed only when the writer signed it.
  - A snapshot replaces the held one only when its `seq` is higher.
  - Two snapshots with the same `seq` and different content are a `progress-fork`. The node keeps the first, keeps the second as evidence, and shows a warning. The writer's own code never makes a fork. A fork means a bug or a restored backup.
  - A missing `seq` (a lost card) is not an error. The next snapshot carries the whole state.
- **How the writer files the counterparty's records:** only from the counterparty DID the quote names, and only with the role that fits (for example, a payee half of a currency commitment only from that commitment's receiver).
- **What each node shows.** From `agreed` on, both nodes show the writer's snapshot. Each node also holds every signed half, so a person can always open the signed facts behind a state.

### Reserve, confirm and release: the agreement side

The hooks themselves (their idempotency, expiry and the owner's clean-up) are the extensions change's (EX-5, EX-6, EX-20 to EX-22). This section says when the deal core calls them.

**Each owner reserves its own resource, on its own node, at the moment it signs.** There is no reserve call between nodes.

1. **The counterparty owns a resource** (for example it gives a service in a booked slot, or a bid where the wanter books its own slot). `agreement.accept` calls the extension's local `reserve` for each such commitment, before it signs. If one fails, it calls `release` on the ones that succeeded, signs nothing, and answers `reserve-failed` with the extension's reason.
2. **The writer owns a resource** (the usual List-Search case, today's slot booking). When the writer files the acceptance half, it calls `reserve` for each such commitment, under its fence. If one fails, it releases the others, signs nothing, and sends a snapshot with state `not-formed` and the reason. The counterparty's node then releases its own reservations. The writer forms the agreement on its own node, so it confirms its own reservations at once. They never wait for an expiry.
3. **Confirm.** When the counterparty's node files the countersigned half, it calls `confirm` for its own reservations. If a confirm is refused (EX-22: expired, and the resource was given to someone else), it sends a `commitment-failure` with `reservation-lost`, and the writer marks that commitment `failed`.
4. **Release** also happens when the deal ends `expired`, `not-formed`, `declined` or `cancelled`. A lost release is harmless, because each reservation expires on its owner's node (EX-20).

This differs from the extensions change's text, which says the writer "calls reserve … on the resource owner's node". Calling another node would need a new wire-reachable method. Reserving at signing time gives the same result with none. The owner agreed to this (open question 5). The extensions change's text is updated at its close-out.

### Selective disclosure

#### What is hidden

The hidden fields are fixed in code, per object, as listed in the field tables above. The quote author cannot choose to make a price visible. This matches the overview's rule: outcomes are visible, amounts and payment details are hidden.

Visible always: ids, the two parties, the currency, the expiry, the home exchange, and for each commitment its id, giver, resource kind, commitment kind, extension and `after` list. Hidden always: the free text, the offering, every declared value, details, window, location and every payment detail.

#### The encoding

The format is SD-JWT's disclosure and digest format, from the IETF specification "Selective Disclosure for JWTs (SD-JWT)" (`draft-ietf-oauth-selective-disclosure-jwt`; `unverified`: the final RFC number, to be checked when C2 is planned). We take its disclosure, digest and `_sd` rules exactly, and leave out its JWT wrapper.

For each hidden field of an object:

1. **Salt.** 16 bytes from the host's secure random source, encoded base64url with no padding (22 characters). A fresh salt for every field of every version. Never derived from data.
2. **Disclosure.** The JSON array `[salt, name, value]`, serialised as compact UTF-8 JSON (no spaces), then encoded base64url with no padding. `name` is the field name, for example `"declared_value_minor"`. `value` is the field's value, or `null`.
3. **Digest.** SHA-256 over the US-ASCII bytes of the disclosure string (the base64url text, not the decoded JSON), encoded base64url with no padding (43 characters).
4. **Place.** The field is left out of the object. Its digest goes into that object's `_sd` list. The list is sorted ascending by byte value, so its order says nothing about the order of the fields. (SD-JWT allows sorting or shuffling. We sort, so the canonical bytes are stable.)
5. **Algorithm.** `terms._sd_alg` is `"sha-256"`. A verifier refuses any other value.

The digest is over the disclosure string as sent. So our canonicaliser (key-sorted `serde_json`, not RFC 8785) never has to reproduce the JSON inside a disclosure.

What we leave out of SD-JWT, and why:

| SD-JWT part | Here | Why |
| --- | --- | --- |
| JWS signature and the `~` combined format | our signed envelope; disclosures travel in the card | Roym records are already signed envelopes with delegation certificates. |
| Key binding JWT | not used | The presenter does not have to prove it holds a key. Any holder may present. This is not a goal here. |
| Decoy digests | not used | Every object always has the same hidden field names (with `null` values), so the number of digests already says nothing. |
| Array element disclosures (`{"...": digest}`) | not used | No list element is hidden on its own. `payment_methods` is hidden as a whole. |
| Nested disclosures | not used in v1 | `details` is hidden as a whole. |
| `_sd_alg` at the top of the payload | inside `terms` | `terms` is the object both records carry, so the receipt needs it there. |

#### Where disclosures travel

- The card wrapper (`crates/roym_core/src/card.rs:37`) gains one optional field: `disclosures`, a list of strings, empty by default. The card rule "nothing derived from the envelope" still holds. A disclosure is not derived from the envelope. The receiver checks each one against a digest.
- The quote card carries every disclosure of the quote. The counterparty's node checks them and stores them with the quote.
- A receipt half card carries none. Both parties already hold the quote's disclosures.
- A settlement card carries the disclosures of its own hidden fields.
- The quote with its disclosures must fit in one card body (160 KiB). `quote.set` refuses a larger quote with `quote-too-large`.
- Both parties keep all disclosures in their backup bundle. Without them, a party cannot show its own deal.

#### Verifying

Given an envelope and some disclosures, the verifier:

1. verifies the envelope as today;
2. for each disclosure: decodes it, checks it is a 3-element array with a 16-byte salt, computes its digest, and finds that digest in exactly one `_sd` list;
3. refuses if a digest appears twice anywhere in the payload, if the name is not a hidden field of that object, if the name equals a visible field, or if the value fails the field's type and limits (a non-integer number fails);
4. returns the payload with the disclosed fields put back, and every other hidden field marked "hidden".

The counterparty's node also checks, before it accepts a quote, that it received a disclosure for every digest and that no salt repeats. It cannot check that a salt is random. An author who wants to leak a price can do so anyway, by telling someone.

#### Showing a deal to a third party

```text
Presentation {
  presentation_version: 1
  records:      [ envelope ]   // one or both receipt halves; optionally the progress snapshot
                               // and settlement records of the same agreement
  disclosures:  [ string ]     // the ones the presenter chooses
}
```

- The verifier checks that every record names the same agreement and, for receipt halves, carries the same `terms`.
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
- **Atomic append.** A service can write a log entry in the same data-layer call as the state change it records. A call to another service cannot be part of that write. A crash between the two would lose the entry.
- **No new dependency on the deal path.** `obligation` would otherwise call `transaction` on every EEP event. Now only the statement export, which is a read, calls `obligation`.

#### The log entry

Collection `event_log` in `transaction`. Entries are only ever created, never changed or deleted. Each entry's row id is derived from the event, so creating it twice is refused by the data layer, and a repeat is a no-op.

```text
EventLogEntry {
  id:               "<agreement>:<commitment_id or '-'>:<event>"   // row id; unique
  at_secs:          u64           // indexed; when this node recorded the event
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
- **Who appends.** Each node appends for its own deals: the writer when it changes a state, the counterparty when it files the snapshot that shows the change.
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
    agreed_at_secs, settled_at_secs
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

- A row is a commitment whose `commitment-settled` event falls in the period. Failed and cancelled commitments are not given or received, so they are not rows. (The owner agreed to this. See open question 10.)
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
| `quote.set` | changed | input: `answers`, `counterparty_did`, `terms` with plain values. The service picks salts, builds digests, signs, and sends the card with all disclosures. Refuses `quote-already-accepted`, `commitment-kind-not-supported`, `quote-too-large`. |
| `quote.get`, `.list`, `.history` | changed | the view holds the plain values for the two parties, from the stored disclosures |
| `quote.verify` | changed | input: an envelope and optional disclosures. Returns the verdict and the disclosed fields. |
| `quote.decline` | removed | replaced by `deal.decline` |
| `deal.decline` | new | signs and sends a `deal-decline` for an enquiry or a quote version |
| `agreement.accept` | changed | reserves this node's own resources, then signs and sends this party's half. Input adds `viewed_lang`, `viewed_via_mt`. |
| `agreement.get`, `.list` | changed | the view holds every commitment with its state, from the latest snapshot |
| `agreement.verify` | changed | input: an envelope and optional disclosures |
| `agreement.cancel` | new (from `booking.cancel`) | quote author only, while every commitment is `promised` or `payment-requested` |
| `agreement.progress` | new (from `booking.get` and `booking.history`) | the snapshots of one agreement, latest first |
| `agreement.present` | new | input: an agreement and a list of field paths to show. Returns a `Presentation`. |
| `agreement.verify-presentation` | new | verifies a `Presentation` from someone else and returns what it shows |
| `payment.request` | changed | input adds `commitment_id` |
| `payment.acknowledge` | changed | input adds `commitment_id`; `method` and `reference` become hidden fields |
| `payment.get`, `.verify` | changed | per commitment |
| `fulfilment.sign` | changed | input adds `commitment_id` |
| `fulfilment.get`, `.verify` | changed | per commitment |
| `booking.*` | removed | the booking track moves to the scheduling extension; the deal parts move to `agreement.*` |
| `eventlog.list` | new | input: `after` (an `{at_secs, id}` cursor, or absent) and `limit`. Pages the log in `(at_secs, id)` order. |
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
- **Intents and wants (M08D, M08F).** `listing_id` becomes the intent id when M08D renames listings. `answers.kind = "want"` and `home_exchange` are filled by M08F. Their places in the record exist now, so the canonical bytes do not change again later.
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
- **One shared event log for `transaction` and `obligation`.** Two writers, and no atomic append. See [Where the event log lives](#where-the-event-log-lives).
- **A statement built from a scan of all records at export time, with no log.** Possible, but every later reader (policy hooks, insight extensions) would repeat the scan, and the overview asks for a log.
- **A per-commitment currency.** It breaks "one currency per agreement", and totals would need a conversion rule.

## Open questions

Each has a recommendation. The owner decides. Questions 1 and 3 to 11 are decided. Question 2 belongs to a C0 ADR.

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
    Reason: they were not given or received. The settlement time is the time both parties signed that the resource changed hands. Accountants who need the agreement date have `agreed_at_secs` on every row.
    Owner decision (2026-11-10): agreed.

11. **Private forwards of a quote with the price shown.** (New.)
    Recommendation: in M08C, a forwarded quote carries no disclosures. The M08G disclosure ADR decides whether a private forward to a contact may show amounts.
    Reason: the overview's rule is that a third party sees amounts only with the other party's grant. A forward is the first place this rule is felt. It is better decided once, in the ADR, than quietly in the forward action.
    Owner decision (2026-11-10): agreed.

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
- [ ] C1: commitment model, records, states, single writer, `deal-decline`, `commitment-failure`, per-commitment settlement records. Replace today's shapes in place.
- [ ] C2: the disclosure encoder, verifier and presentation; disclosures in the card wrapper and the backup bundle.
- [ ] C3 (with the extensions change): the reserve, confirm and release calls at the points above.
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
