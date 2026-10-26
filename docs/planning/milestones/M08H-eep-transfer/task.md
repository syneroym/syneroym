# M08H: EEP transfer

*Reader: developers and the project owner, when they plan this sub-milestone.*

**Status: outline (2026-10-26).** This is a first outline. The full plan is written when M08G is near its end. Milestone overview and order: [M08-overview.md](../M08-overview.md).

## Goal

A holder can pass an EEP to someone else, and the issuer reissues it to the new holder. A participant can give several EEPs in one agreement (a bundle), swap EEPs with another participant, and list EEPs for trade. This comes after discovery (M08F) and reputation (M08G), so that strangers can find each other and a receiver can judge an issuer from published evidence before taking its EEPs. The order is about user value only. There is no legal gate (owner decision, 2026-10-26).

## Change docs

- [roym-value-exchange](../../changes/roym-value-exchange/change.md): transfer and reissue, bundles, netting (`VX-25` to `VX-29`).
- The obligations module change doc written for M08E, extended with endorsements, reissue requests, refusals and swaps.

## Slices (first cut)

| Slice | Content | Needs |
| --- | --- | --- |
| H0 | Extend the obligations module change doc. | M08E |
| H1 | Transfer: endorsement, reissue request through the mailbox, refusals, idempotency, double-spend proof. | H0 |
| H2 | Bundles and two-party swaps. | H1 |
| H3 | EEPs listed for trade (an EEP as a resource in an offering or a want). Close-out. | H2, M08F |

## Open before planning

- A refused EEP in a bundle: one payment commitment per EEP, or one payment with a refund rule (Value Exchange, open question).
