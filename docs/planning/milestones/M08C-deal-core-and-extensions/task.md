# M08C: Deal core and extensions

*Reader: developers who build this sub-milestone, and the project owner.*

**Status: started (2026-11-15). C1 plan written and reviewed (2026-11-16). The owner answered every question in section 6 on 2026-11-15.** Progress is in [status.md](status.md). Milestone overview and order: [M08-overview.md](../M08-overview.md).

## Goal

Every Roym deal gets one fixed **deal core**. A deal core agreement is made of 1 to 16 **commitments**. A commitment is one party giving one resource (a service, goods or money) to the other. Each commitment has a declared value in the agreement's one currency. The quote author's node is the only writer of the agreement's state. A party can show a deal to someone else with the price hidden. Each node keeps an event log of its deals, and a participant can export a statement for a period. Business work steps move out of the core into **Roym extensions**: scheduling (today's slots) and one-shot (today's orders) are the first two. Chat gets widgets (for example a poll) and message forwarding with a signed origin.

Terms (commitment, declared value, writer, disclosure, presentation, progress snapshot and others) are defined in the [roym-agreements](../../changes/roym-agreements/change.md) change doc, section "Terms", and in [TERMINOLOGY.md](../../../TERMINOLOGY.md).

**Demo** (owner confirmed, 2026-11-15)**:** a new story `demo/50-roym-deal.spec.ts`. Story 40 already shows one person ask for a quote, the other send it, and the first accept it (in Hindi). So story 50 shows what is new: Bob offers a lesson for a sack of wheat plus ₹100. Alice sees what she gives, what she receives and the difference, and accepts. Both settle their commitments in the Hub, and Alice downloads her statement. C1 starts it (the deal with two commitments, accepted). C6 finishes it (totals, settlement, statement). C4 adds a poll and C7 a forward step to story 20 (chat).

## Change docs

Read them first. Each slice below names the change docs it builds part of.

| Change doc | Status | Criteria built here |
| --- | --- | --- |
| [roym-agreements](../../changes/roym-agreements/change.md) | draft, open questions 1 and 3 to 12 decided by the owner (2026-11-10), 3 review rounds | `AG-1` to `AG-23`, `AG-27` to `AG-43`. `AG-24` to `AG-26` are built in M08E and M08H. |
| [roym-value-exchange](../../changes/roym-value-exchange/change.md) (overview) | draft | `VX-1`, `VX-2`, `VX-6` to `VX-12`, `VX-45`, `VX-46`, through the `AG-*` rows that refine them |
| [roym-extensions](../../changes/roym-extensions/change.md) | draft, reviewed (no review file was kept) | `EX-1` to `EX-8`, `EX-10` to `EX-14`, `EX-17` to `EX-22`. `EX-11` for inline definitions only. `EX-9` (an opinion points at a step) is built in M08G. `EX-15` and `EX-16` move out of M08C (section 6, question 3). `EX-19` is the manifest field only. |
| [roym-message-forwarding](../../changes/roym-message-forwarding/change.md) | draft, reviewed (no review file was kept) | `FW-1` to `FW-15` |
| [roym-multilingual](../../changes/roym-multilingual/change.md) | done | the rest of `ML-26` (the widget-answer hook point) and `ML-28` (widget labels and extension step names), as its `## Deviations` says |

The agreements doc is the module doc that the old outline listed as "to write first". It is written. It changes parts of the other three docs. Its section "Differences from the overview and sibling changes" lists each change and the doc that is updated at close-out (C8).

## 1. Shared parts

| # | Shared part | Used by | Built in |
| --- | --- | --- | --- |
| Z1 | **Deal core types** in `roym_core`: `DealTerms`, `Commitment`, `LocalText`, the resource and commitment kind table, the agreement and commitment states, and the pure transition function. | Every slice that touches a deal; EEP kinds in M08E | C1 |
| Z2 | **Disclosure encoder and verifier** (RFC 9901 disclosures, digests, `_sd`), and the `Presentation` format. | Every signed deal record (C2); evidence in M08G | C2 |
| Z3 | **Extension manifest and the fixed extension registry** in `roym_core`. A TOML manifest in each extension's crate, read at build time (agreements doc, open question 3). | Scheduling, one-shot, widgets, step names | C3a |
| Z4 | **Generic field view** in the Hub: shows labelled fields from a schema's field types, with labels from the string catalog. | `step` card (C3a), `details` of an unknown extension (C3a), widgets (C4) | C3a |
| Z5 | **Reserve, confirm and release** local methods of an extension, with expiry and owner-side clean-up. | Scheduling (C3b); later any extension with a reserve hook | C3b |

**Component size limit.** The M08B rule stays (see [M08B task.md](../M08B-language-and-structure/task.md), section 1; owner decision, 2026-11-15). Each slice that changes a `roym_*` crate reports the size of every Roym component for `wasm32-wasip2`, before and after, from one build folder. A component with no new record code may grow by at most 40 KB. A component with new record code may grow by at most 15%. Over a limit, stop and tell the owner. C3b adds a seventh component (scheduling); it reports its size alone.

**C1a sizes** (`mise run build:roym`, one build folder, 2026-11-17; before on `main` at `34dbe9bd`, after on `feat/m08c-c1-commitment-model` at `f44b0e72`), in bytes:

| Component | Before | After | Change | Limit |
| --- | --- | --- | --- | --- |
| `transaction` | 1,516,528 | 1,715,448 | +198,920 (+13.1%) | +15% (227,479), new record code |
| `catalog` | 929,558 | 929,631 | +73 | +40 KB |
| `conversation` | 811,880 | 811,953 | +73 | +40 KB |
| `directory` | 1,467,413 | 1,467,518 | +105 | +40 KB |
| `profile` | 771,718 | 771,791 | +73 | +40 KB |
| `web` | 310,033 | 309,985 | −48 | +40 KB |

**C1b sizes** (release `wasm32-wasip2`, one `target/` folder, 2026-11-18; base on `main` at `3dfada58`, branch at `14b96999`), in bytes:

| Component | `main` `3dfada58` | branch `14b96999` | Change | Limit |
| --- | --- | --- | --- | --- |
| `transaction` | 1,715,448 | 1,833,741 | +118,293 (+6.90%) | +15% (257,317), new record code |
| `catalog` | 929,631 | 929,631 | 0 | +40 KB |
| `conversation` | 811,953 | 811,953 | 0 | +40 KB |
| `directory` | 1,467,518 | 1,467,518 | 0 | +40 KB |
| `profile` | 771,791 | 771,791 | 0 | +40 KB |
| `web` | 309,985 | 310,017 | +32 | +40 KB |

Every component is within its limit.

**Parity runs need a fresh WASM build.** The dual-build parity harness loads pre-built `wasm32-wasip2` components. Every slice that changes a `roym_*` crate runs `mise run build:roym` before it trusts a parity run.

## 2. Facts checked in the code for this plan

Checked on `main` at `1b61d2c3` (2026-11-15). The agreements doc checked its own facts at `4830bdf7` (2026-11-10). M08B B3b to B6 merged after that. They moved some lines, and they changed no fact that the agreements doc relies on.

**Deal records** (`crates/roym_core`)

- `AgreedTerms` (`src/transaction.rs:153`) has one amount, one scope and one optional slot. `RequestPayload` is at `:181`, `AgreementReceiptPayload` at `:234`, `ReceiptHalf` at `:252`. Each receipt half already signs `viewed_lang` and `viewed_via_mt` (`:245`, M08B B5).
- The card table `CARD_TYPES` (`src/card.rs:9`) has seven types: `request`, `quote`, `agreement-receipt`, `booking-progress`, `payment-request`, `payment-acknowledgement`, `fulfilment-receipt`. A card body is at most 160 KiB (`MAX_CARD_BODY_BYTES`, `:63`). The card wrapper is `Card` (`:37`). It has no `disclosures` field.
- `PaymentAcknowledgementPayload` (`src/payment.rs:94`) and `FulfilmentReceiptPayload` (`src/fulfilment.rs:35`) carry the deal parties and a `role` of `quote-author` or `counterparty`, not a giver and a receiver.
- The track window ends at `window.latest_secs`, or the scheduling time, plus `TRACK_WINDOW_SECS` (30 days) (`src/booking.rs:18`, `:190`). `MAX_SLOT_CAPACITY` is 64 (`:20`).
- A listing has `BookingMode` (`src/listing.rs:53`: `Slots`, `Order`, `Enquiry`) and `PaymentTerms` (`:77`). The slot id is derived in `roym_core` (`derive_slot_id`, `:428`), so the catalog and the quote derive the same id.
- `money::currency_minor_exponent` (`src/money.rs:48`) knows the currency codes. A record carries at most 4 author translations (`MAX_TRANSLATIONS`, `src/lang.rs:7`).

**The `transaction` service** (`crates/roym_transaction`)

- `SCHEMA_VERSION` is 4 (`src/app.rs:46`). Every method is local-only through `admit::require_internal` (`src/app.rs:279`).
- `agreement.accept` is `accept` (`src/app/agreement_ops.rs:113`). The quote author of a quote with no slot may still accept first (`prepare_author_booking`, `:415`). The countersign runs in sync (`maybe_countersign`, `:451`).
- `quote.set` is at `src/app/quote_ops.rs:163`. A new quote version clears a decline (`:605` to `:615`). `quote.decline` is local and unsigned (`:508`).
- The writer's fence is a unique insert in the `ledger` collection: rows `decision:<agreement>`, `seat:<slot>:<n>` and `step:<agreement>:<seq>` (`src/app/ledger.rs:53`, `:57`, `:61`). The key is the version's `record_id`, not the quote id. This is the gap the agreements doc closes (its section "Facts checked in the code").
- `transaction` reads slots from `catalog` with a local `availability.get` call (`src/app/ledger.rs:145`, `src/app/quote_ops.rs:331`). `booking-progress` is signed by the service key (`src/app/progress.rs:32`).
- Slot code today: `catalog` has `availability.set`, `.get`, `.list`, `.remove` (`crates/roym_catalog/src/api.rs:36` to `:39`, `src/app/availability.rs`, 152 lines). `transaction` has `src/app/booking_ops.rs` (591 lines) and the seat rows in `ledger.rs`.

**The Roym app** (`crates/roym_core/app/roym.toml`)

- Six services: `web`, `profile`, `conversation`, `catalog`, `transaction`, `directory`. `transaction` depends on `conversation` and `catalog`. A scheduling extension service is a seventh service and a new `depends_on` edge from `transaction`.

**Hub** (`crates/roym_web/ui`)

- The Hub calls only four deal methods: `request.set`, `quote.set`, `quote.decline` and `agreement.accept`. It has no action for booking, payment or fulfilment (backlog row "Hub transaction action panel and Playwright transaction spec (cases 33-39) are not built").
- The quote card shows "Accept" on every version's card (`src/screens/messages.ts`, the `agreement.accept` call at `:687`). Hub case 31 checks the sentence "the other side is not told" after a decline. C1 changes both.
- Card templates: `src/cards/templates/` has one file per card type (seven).
- Demo story 40 (`demo/40-roym-languages.spec.ts`) already runs request, quote and accept to a signed agreement, and reads the state with `agreement.list`. Every shape change in C1 must keep it passing.

**Host capabilities**

- `syneroym:events@0.1.0` (`crates/wit_interfaces/wit/events/events.wit`) has `record` and `read` only. `read` never waits, and the host keeps events in memory only, so they are lost on restart. There is no push to a guest (backlog row "Services can only pull events"). EX-15 says an extension service "does not poll". No M08C extension reads events, so this stays open (section 6, question 3).
- The conversation `message` record has `content-type: string` (`crates/wit_interfaces/wit/conversation/conversation.wit:50`). A forward mode and an origin can go in a Roym message body without a WIT change. The C7 plan checks this.
- The newest ADR is 0029. The C0 ADR gets the next free number.

## 3. Slices

Every slice is one PR that passes `mise run verify`. Sizes are S, M and L, as in M08A and M08B.

**The agreements doc gets one more review first** (owner decision, 2026-11-15). It was written and reviewed by the same model (Claude Opus 5.5, 3 rounds). Session strategy asks for a reviewer that is a different tool than the author. So a different tool reviews it before the C1 plan, as a new round in [its review file](../../changes/roym-agreements/review.md). The extensions and forwarding docs were reviewed when they were written (owner, 2026-11-15). No review file was kept for them, and they need no new review.

### C0 — Forwarding ADR (docs only, S)

- ADR: signed message origins and the sender's forward mode (option E of the forwarding doc). It records the shipped default (the forwarding doc recommends `not-forwardable`), whether a forwarded quote follows the message's mode or travels as itself (FW-7), and how a `message-origin` carries its certificate. The size of an origin is measured in C7, and the ADR says what to do if it is too large.
- The deal-grant ADR is not in M08C. It moves with C5 (section 6, question 3).
- C0 blocks only C7. The other slices need no ADR, because the agreements doc fixes their design.
- Demo: none (docs only).

Change docs: roym-message-forwarding.

### C1 — Commitment model and deal records (L)

- Z1. Replace today's shapes in place (`SCHEMA_VERSION` 4 to 5, no migration): `request` becomes `enquiry`; `AgreedTerms` becomes `DealTerms` with 1 to 16 commitments, still with plain values (no `_sd` yet); a `LocalText` per text field; `after` replaces `PaymentTiming`.
- The deal and agreement states with `accepted`, `declined` and `not-formed`; the commitment transition tables for `fulfilment` and `currency`; the track window rule `max(window.latest_secs, agreed_at_secs) + 30 days`.
- The single writer: the fence `decision:<quote_id>`, the version check (`quote-superseded`), no revision after acceptance, the writer never signs first (`writer-countersigns`), the countersign row, the writer's decision steps with recovery, and the writer's `seq` after a restore.
- `agreement-progress` replaces `booking-progress`. Signed `deal-decline` replaces the local `quote.decline`. `agreement.cancel` and `agreement.progress` replace the deal parts of `booking.*`.
- Per-commitment settlement records with `giver` and `receiver` roles and `settlement_note`. One payment per currency commitment.
- Today's slot path stays inside `transaction`, behind the new shapes, so booking keeps working until C3b (agreements doc, open question 12).
- Hub: the quote form builds `DealTerms` with one service and one currency commitment (today's case); "Accept" only on the latest version; decline sends `deal.decline` (Hub case 31's sentence changes); the card templates for the new shapes. The receipt card gets a line that says what each party read (backlog row "The receipt card does not show what each party read").
- `roymctl roym transaction`, the parity harness, the backup bundle and the `method_contract` rows change with the shapes.
- Criteria: AG-1 to AG-13, AG-22, AG-31 to AG-33, AG-35 to AG-37, AG-39, AG-41 to AG-43. AG-14 and AG-15 with today's slot path inside `transaction`; C3b moves them to the extension. AG-23 for the range rule only; C6 adds the Hub part. AG-2 and AG-4 for the three M08C resource kinds.
- Demo: starts story 50 (Bob quotes a lesson for wheat plus ₹100; Alice accepts).
- The plan checks the size first. This is the largest slice. If the plan finds a clean cut that keeps a working system after each part, it splits C1, and records why.
- **Split (owner decision, 2026-11-16, [C1 plan](slice-c1-implementation-plan.md), Q1):** C1a (deal shapes: every record and caller on the new shapes, the writer's decision in its final order) and C1b (single writer and signed decline: version checks, refusal snapshots, recovery, restore rules, `deal-decline`). Each is one PR.

Change docs: roym-agreements, roym-value-exchange (VX-1, VX-2, VX-6 to VX-9, VX-12), roym-extensions (EX-1, EX-3 shape only).

### C2 — Selective disclosure (L)

- Z2. `_sd` and the fixed hidden fields of the quote, each commitment and each settlement record. Fresh 16-byte salts per field and version. The RFC 9901 test vectors.
- `disclosures` in the card wrapper; the counterparty's node checks and stores them; both parties keep them in the backup bundle. Views show plain values from the stored disclosures.
- Size limits (8 KiB per `details`, 96 KiB per record). The plan checks 96 KiB against a real envelope in the 160 KiB card body. The Hub's quote form shows the size against the limit.
- `deal.present` and `deal.verify-presentation`. `quote.verify` and `agreement.verify` take disclosures.
- Criteria: AG-17 to AG-21, AG-34, AG-38, AG-40. AG-33's check that `settlement_note` is hidden.
- Demo: the plan decides. An end user sees only the size line on the quote form.

Change docs: roym-agreements, roym-value-exchange (VX-11).

### C3a — Extension frame, step records and one-shot (M)

- Z3: the manifest format, the fixed registry, and the `insight` kind as a manifest field only (EX-19).
- The `details` block in a commitment (EX-1, EX-3) and the schema check before signing (EX-2: "not checked on this device" for an unknown extension; the writer's check binds).
- The `step` record and card (EX-7, EX-8), and Z4 in the Hub. Step names come from the string catalog (the step-name half of ML-28).
- The one-shot extension (EX-18). A listing names its extension in place of `BookingMode::Order`, and `Enquiry` becomes the plain enquiry flow with no extension. The plan decides whether one-shot needs a service part or only a manifest.
- Step records come here, not in C4 as the outline had it, because C3b turns today's `scheduled` and `in-progress` booking states into scheduling steps.
- Criteria: EX-1, EX-2, EX-3, EX-7, EX-8, EX-18, EX-19; ML-28 (step names).
- Demo: the plan decides (one-shot looks the same as today's order to a person).

Change docs: roym-extensions, roym-multilingual (ML-28).

### C3b — Scheduling extension and reserve hooks (L)

- A new Roym service for the scheduling extension (`crates/roym_scheduling`, package `syneroym-roym-scheduling`). It takes `availability.*` from `catalog`, and the seat rows and the booking track from `transaction`. `catalog` and `transaction` keep no slot code (EX-17). Linking it into `transaction` as a library is rejected (agreements doc, "Rejected options").
- Z5: `reserve`, `confirm` and `release`, idempotent by (agreement id, commitment id), with a 15-minute expiry and clean-up on the owner's node (EX-20, EX-21, EX-22). Each giver reserves on its own node when it signs (agreements doc, "Who reserves"; this changes EX-5's text).
- The `commitment-failure` record and card, and the `other-commitment-failed` cancel in the same snapshot.
- Today's booking tests pass through the extension, including the race for the last seat.
- Criteria: AG-14, AG-15, AG-16, AG-36 (snapshot part); EX-4, EX-5, EX-6, EX-17, EX-20, EX-21, EX-22.
- Demo: the plan decides. A person who books a slot sees the same steps as today.

Change docs: roym-extensions, roym-agreements, roym-value-exchange (VX-10).

### C4 — Widgets (L)

- Widget definitions (EX-10), the `widget` and `widget-answer` records and cards (EX-12), the generic renderer on Z4, and the poll renderer with a count per option.
- Inline definitions up to 8 KB (EX-11). A larger definition is refused. Fetch by hash from the sender's node would need a new wire-reachable method, so it is a backlog row (section 6, question 4).
- Widget labels from the string catalog (EX-13, the widget half of ML-28). The widget-answer translation hook point (the last M08C part of ML-26).
- Widgets work in any chat, so this slice changes `conversation` and the Hub chat, not `transaction`.
- With C1 to C4, the card table has twelve types: `enquiry`, `quote`, `agreement-receipt`, `agreement-progress`, `payment-request`, `payment-acknowledgement`, `fulfilment-receipt`, `deal-decline` (C1), `step` (C3a), `commitment-failure` (C3b), `widget`, `widget-answer` (C4).
- Criteria: EX-10, EX-11 (inline only), EX-12, EX-13; ML-26 (widget answer), ML-28 (widget labels).
- Demo: extend story 20 with a poll in the chat (the plan confirms).

Change docs: roym-extensions, roym-multilingual (ML-26, ML-28).

### C5 — moved out of M08C

The owner moved C5 out on 2026-11-15 (section 6, question 3). It was: an extension service receives deal and conversation events (EX-15), and the counterparty calls its declared methods with a deal grant (EX-16), with an ADR for deal grants. No M08C extension needs either one. Scheduling confirms and releases through local calls from `transaction` on its own node, and no counterparty calls it. Both come with the first extension that needs them. EX-14 (an extension service ships in the Roym SynApp) is met by C3b. The id C5 is not reused.

### C6 — Mixed exchanges, event log, statement and settlement (L)

- Hub: a quote form with any mix of commitments on each side; before signing and on the agreement screen, the total each party gives and receives and the difference (AG-23). The agreement screen shows each commitment's state from the latest snapshot.
- The `event_log` collection, the derive step on every sync, `eventlog.list` and `eventlog.rebuild` (AG-27, AG-28).
- `statement.export` with the EEP section (`not-available` until M08E) (AG-29, AG-30). The Hub's statement screen with a CSV and a JSON download.
- Settlement actions in the Hub (section 6, question 2): on each commitment, the giver's claim ("I delivered" or "I paid") and the receiver's confirm ("received"), with an optional `settlement_note`. Booking steps and payment requests stay out of the Hub.
- Criteria: AG-23 (Hub part), AG-27 to AG-30.
- Demo: finishes story 50.

Change docs: roym-agreements, roym-value-exchange (VX-2, VX-7, VX-45, VX-46).

### C7 — Message forwarding (M)

- The forward mode (`signed` or `not-forwardable`) in each message body, the default-mode setting, and the composer control. The `message-origin` record, signed at send for `signed` messages and checked at receive. The forward method and the Hub action, which refuse a `not-forwardable` message. The display rules, the first-message choice and the group notice.
- Measure the size of an origin with its delegation certificate (forwarding doc, open question "Size"), and follow the C0 ADR if it is too large.
- A forwarded quote travels as its signed envelope and carries no disclosures, so it shows only visible fields (agreements doc, open question 11). This part needs C2.
- Criteria: FW-1 to FW-15.
- Demo: extend story 20 with a forward ("Forwarded. First written by …").

Change docs: roym-message-forwarding.

### C8 — Close-out (S)

- Check that every `AG-*`, `EX-*` and `FW-*` row built here names a test on `main`, or a later sub-milestone (section 5).
- Update the living docs that each change doc's close-out lists (integrated spec, terminology, architecture, developer guide, requirements). Update the overview and the extensions and forwarding docs where the agreements doc's "Differences" table says so.
- Set `roym-agreements`, `roym-extensions` and `roym-message-forwarding` to `status: done`. The overview stays `draft`; tick its M08C task.
- Close the deferred-backlog rows in section 7.

Change docs: all four.

## 4. Order and parallel work

```text
C1 ─▶ C2 ─▶ C3a ─▶ C3b ─▶ C4 ─▶ C6 ─▶ C8
C0 (docs) + C2 ─▶ C7 (may run beside C3a to C6, but not beside C4)
```

- C1 can start before C0: no ADR is needed for it. The second-tool review of the agreements doc comes before the C1 plan.
- C1, C2, C3a, C3b and C6 run in order, because they all change `roym_core`'s deal types, `transaction` and the Hub's deal screens.
- C4 changes `conversation`, the card table and the Hub chat. It comes after C3a, which builds the generic field view it uses.
- C6 needs only C2. It comes last, so that the deal screens are changed once more, at the end, with every commitment kind and extension in place.
- C7 changes `conversation` and the Hub chat, like C4. So C7 and C4 must not run at the same time.

## 5. Criteria that a later sub-milestone finishes

Record each in the change doc's `## Deviations` when its slice is planned.

| Criterion | Part built in M08C | Part finished later |
| --- | --- | --- |
| AG-2, AG-4, AG-6 | The kinds `service-in-slot`, `goods-or-service` and `currency` | The EEP kinds: M08E (`eep-issue`, `eep-return`), M08H (`eep-transfer`) |
| AG-3, VX-6 | Quotes that answer an enquiry; the unit test that compares both quote forms | The Want-Bid end-to-end test: M08F |
| AG-24 to AG-26 | Nothing (reserved names only) | M08E, M08H |
| AG-30 | The EEP section says `not-available` | The `obligation` sibling fills it: M08E |
| VX-12 | Each failed commitment names its giver in the snapshot | Reputation shows it: M08G |
| EX-9 | Nothing | An opinion points at a step: M08G |
| EX-11 | Inline definitions up to 8 KB (C4) | Fetch by hash for a larger definition: TBD (backlog row) |
| EX-15, EX-16 | Nothing | Events to an extension service and deal grants: the first sub-milestone whose extension needs them (backlog row) |
| EX-19 | The `insight` manifest field | Insight extensions: after M08H |
| ML-26 | The widget-answer hook point (C4) | Opinion: M08G. Offering, want and storefront display: M08D. |

## 6. Questions for the owner

All seven were answered on 2026-11-15. The owner accepted each recommendation.

1. **The demo story.** The outline asked for a new story with request, quote and agreement. Story 40 already shows exactly that. **Answer:** story 50 as in the Demo line above (a mixed exchange, totals, settlement, statement). C4 adds a poll and C7 a forward step to story 20.
2. **Settlement actions in the Hub.** The Hub had no action to mark a commitment delivered or a payment acknowledged (backlog row "Hub transaction action panel … not built"). AG-29's Hub test and story 50 need settled commitments. **Answer:** C6 adds the giver's claim and the receiver's confirm on each commitment. The backlog row shrinks to booking steps and payment requests.
3. **Keep C5 in M08C?** No extension built in M08C needs events or a deal grant. Building them now means building the first wire-reachable path into an extension, and a push for events, with only a test extension to use them. **Answer:** C5 and the deal-grant ADR move to the first sub-milestone whose extension needs them (for example a documents extension). A backlog row records this, and the row "Services can only pull events" moves with it. EX-15 and EX-16 are deviations of the extensions doc.
4. **Widget definitions larger than 8 KB.** EX-11 fetches them by hash from the sender's node. That is a new wire-reachable method on `conversation`, for a sender that may be offline. **Answer:** in M08C, a definition larger than 8 KB is refused. A backlog row records fetch by hash. A poll and small forms fit in 8 KB.
5. **The C3 split.** The outline had one C3 and put step records in C4. **Answer:** the extension frame, step records and one-shot come first (C3a), then scheduling (C3b), because the scheduling move needs step records.
6. **The component size rule from M08B.** **Answer:** it stays for M08C (section 1).
7. **Reviews of the change docs.** Session strategy asks that every change doc a milestone depends on is reviewed before its first slice plan, by a different tool than its author. The agreements doc was written and reviewed by the same model. **Answer:** a different tool reviews the agreements doc before the C1 plan (section 3). The extensions and forwarding docs were already reviewed; only their review files were not kept.

Questions the old outline listed are now answered in the agreements doc: the declared value in a mixed exchange (open question 1), the manifest format and registry (open question 3), and the reservation expiry (open question 4, 15 minutes). The size of a `message-origin` is measured in C7.

## 7. Deferred-backlog rows for M08C

| Row | Section | What M08C does |
| --- | --- | --- |
| The payment and fulfilment records name the deal parties, not the giver and receiver of the resource | §12 | C1 closes it (per-commitment records with `giver` and `receiver`). |
| The receipt card does not show what each party read | §10 | C1 adds the line when it redraws the receipt card. |
| Widget labels and extension step names do not come from the catalog yet | §10 | C3a (step names) and C4 (widget labels) close it. |
| Translation hook points for widget answers, opinions, and offering, want and storefront display | §10 | C4 builds the widget-answer hook point. The row keeps M08D and M08G. |
| Services can only pull events | §10 | Not in M08C. Its target moves with the new row "Extension services cannot receive events or take calls from a counterparty" (question 3). |
| Extension services cannot receive events or take calls from a counterparty (new, 2026-11-15) | §10 | Records C5's move (question 3). |
| A widget definition larger than 8 KB is refused (new, 2026-11-15) | §10 | Records question 4. C4 builds the refusal. |
| Hub transaction action panel and Playwright transaction spec (cases 33-39) are not built | §12 | C1 changes the quote and decline actions. C6 adds the settlement actions (question 2). The row then keeps booking steps and payment requests. |
| More demo stories | §13 | Story 50 adds one. C4 and C7 extend story 20. |
| Two Roym outputs carry another node's card data as an untyped `Value` | §10 | Not targeted. C3b moves `PeerStep` (in `booking_ops.rs`) to the scheduling extension; it updates the row's file names. |
| `RECORD_TYPES` is not read by any verifier | §12 | Not targeted. C1, C3a, C3b and C4 add record types to the table. |

## 8. Not in this plan

- EEP kinds, the `obligation` service and the statement's EEP data (M08E, M08H).
- Wants, bids, `answers.kind = "want"` and `home_exchange` values (M08F). Their places in the records exist from C1.
- Renaming listings to intents and in-return terms (M08D). The Hub pre-fills one currency commitment from today's `PaymentTerms`.
- Evidence, opinions and the grant that lets a third party see amounts (M08G). A forwarded quote carries no disclosures.
- Third-party extensions, extension UI as code, and insight extensions beyond the manifest field.
- Events to an extension service, deal grants and their ADR (the old C5), and fetching a widget definition by hash.
- More than one currency in one agreement. Moving money, checking a payment, or computing tax.
