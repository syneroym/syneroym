# M08E: EEPs

*Reader: developers and the project owner, when they plan this sub-milestone.*

**Status: outline (2026-10-26).** This is a first outline. The full plan is written when M08D is near its end. Milestone overview and order: [M08-overview.md](../M08-overview.md).

## Goal

Earnest effort promises (EEPs), complete: issue, redemption, decline, set-off, split, expiry, transfer with the issuer's reissue, bundles, two-party swaps, and EEPs listed for trade. An EEP is a signed promise by its issuer to make an earnest effort to accept it against the issuer's own offerings, at a face value (see the Value Exchange change). There is no legal gate (owner decision, 2026-10-26). Features that present EEPs as investments stay out.

## Change docs

- [roym-value-exchange](../../changes/roym-value-exchange/change.md): EEPs (`VX-13` to `VX-29`, `VX-49`, `VX-50`).
- **To write first:** the obligations module change doc (exact EEP record, endorsement, refusal, demand, decline, swap; wire-reachable methods; the proof that a `stored` receipt covers a demand).

## Slices (first cut)

| Slice | Content | Needs |
| --- | --- | --- |
| E0 | Obligations module change doc. | — |
| E1 | Revocations gain `effective_from` and a reason, with the countersign rule after a key compromise (`VX-22`). | — |
| E2 | The mailbox signs its `stored` receipt (`VX-49`). | — |
| E3 | `obligation` service, deployed `public`: issue, holder acknowledgement, expiry, wallet screen. | E0, E1, M08C |
| E4 | Redemption with the claim rule, decline, set-off, split, restore rule, per-issuer limit, stated outstanding total. | E3 |
| E5 | Transfer: endorsement, reissue request through the mailbox, refusals, idempotency, double-spend proof. | E2, E4 |
| E6 | Bundles, two-party swaps, EEPs listed for trade. Close-out. | E5, M08D |

## Open before planning

- A refused EEP in a bundle: one payment commitment per EEP, or one payment with a refund rule (Value Exchange, open question).
- Until M08G, the Hub warns that a stranger's EEP has no published evidence.
