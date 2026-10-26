# M08E: EEPs between two parties

*Reader: developers and the project owner, when they plan this sub-milestone.*

**Status: outline (2026-10-26).** This is a first outline. The full plan is written when M08D is near its end. Milestone overview and order: [M08-overview.md](../M08-overview.md).

## Goal

Earnest effort promises (EEPs) between an issuer and one holder: issue, redemption, decline, set-off, split and expiry. An EEP is a signed promise by its issuer to make an earnest effort to accept it against the issuer's own offerings, at a face value (see the Value Exchange change). Transfer to a third person comes later, in [M08H](../M08H-eep-transfer/task.md), after discovery and reputation. There is no legal gate (owner decision, 2026-10-26). Features that present EEPs as investments stay out.

## Change docs

- [roym-value-exchange](../../changes/roym-value-exchange/change.md): EEPs between two parties (`VX-13` to `VX-24`, `VX-49`, `VX-50`).
- **To write first:** the obligations module change doc (exact EEP record, endorsement, refusal, demand, decline, swap; wire-reachable methods; the proof that a `stored` receipt covers a demand).

## Slices (first cut)

| Slice | Content | Needs |
| --- | --- | --- |
| E0 | Obligations module change doc. | — |
| E1 | Revocations gain `effective_from` and a reason, with the countersign rule after a key compromise (`VX-22`). | — |
| E2 | The mailbox signs its `stored` receipt (`VX-49`). | — |
| E3 | `obligation` service, deployed `public`: issue, holder acknowledgement, expiry, wallet screen. | E0, E1, M08C |
| E4 | Redemption with the claim rule, decline, set-off, split, restore rule, per-issuer limit, stated outstanding total. | E3 |
| E5 | Close-out. | E4 |

## Open before planning

- E2 (the signed `stored` receipt) is needed by redemption demands to an offline issuer, so it stays here, not in M08H.
