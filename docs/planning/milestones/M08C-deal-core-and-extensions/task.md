# M08C: Deal core and extensions

*Reader: developers and the project owner, when they plan this sub-milestone.*

**Status: outline (2026-10-26).** This is a first outline. The full plan, with shared parts, checked facts and slice details, is written when M08B is near its end. Milestone overview and order: [M08-overview.md](../M08-overview.md).

## Goal

One fixed deal core for every deal: agreements made of commitments, with signed details, one writer, selective disclosure, barter, an event log and a statement. Business work steps move into Roym extensions. Scheduling (today's slots) and one-shot (today's orders) are the first two. Chat gets widgets and forwarding with a signed origin.

## Change docs

- [roym-value-exchange](../../changes/roym-value-exchange/change.md): agreements, commitments, selective disclosure, barter, event log, statement (`VX-1`, `VX-2`, `VX-6` to `VX-12`, `VX-45`, `VX-46`).
- [roym-extensions](../../changes/roym-extensions/change.md) (`EX-*`).
- [roym-message-forwarding](../../changes/roym-message-forwarding/change.md) (`FW-*`).
- **To write first:** the agreements module change doc (exact record shapes, the SD-JWT-style disclosure format, states, methods), as the Value Exchange overview asks.

## Slices (first cut)

| Slice | Content | Needs |
| --- | --- | --- |
| C0 | Agreements module change doc. ADR for deal grants (counterparty access to an extension service). ADR for signed message origins (deniability and the group rule, an owner decision). | — |
| C1 | Commitment model: resource kinds, commitment kinds, states with failure ends, declared values, one currency, `details` block, quote author as single writer. Replaces today's quote and receipt shapes. | C0 |
| C2 | Selective disclosure on the quote and the receipt (salted field hashes). | C1 |
| C3 | Reserve, confirm and release hooks, with reservation expiry and owner-side cleanup. Move slots, seat claims and the booking track into the scheduling extension. One-shot extension. | C1 |
| C4 | Step records and the `step` card. Widget definitions (inline up to a size limit), the `widget` and `widget-answer` cards, the generic renderer, the poll renderer. | C1, M08B labels |
| C5 | Extension services: event subscription (M08A feed), deal grants. | C0, C4 |
| C6 | Barter. Event log. Statement export. Hub screens for multi-commitment agreements and the statement. | C1, C2 |
| C7 | Message forwarding with a signed origin, as the ADR decides. | C0's origin ADR, M08B language tags |
| C8 | Close-out: living docs, change docs `done`. | all |

## Open before planning

- Barter declared value: who declares it, and must both sides match (Value Exchange, notes for module docs).
- Extension manifest format and where installed extensions are recorded.
- The reservation expiry (EX-20, 15 minutes as a first value). A phone that sleeps after it reserves can let it expire, and the commitment then fails after both parties signed (extensions change, open question).
- The members-only key for option D of the forwarding ADR, if the ADR chooses D.
- Size of a `message-origin` record with its certificate (forwarding change, open question).
