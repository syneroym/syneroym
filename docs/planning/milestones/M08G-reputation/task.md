# M08G: Reputation

*Reader: developers and the project owner, when they plan this sub-milestone.*

**Status: outline (2026-10-26).** This is a first outline. The full plan is written when M08F is near its end. Milestone overview and order: [M08-overview.md](../M08-overview.md).

## Goal

A participant can judge another party before a deal, from evidence (signed facts) and opinions (signed views), shown side by side and never merged into one number. Both parties to a deal can write an opinion. Opinions of one deal are sealed, revealed to each other, given a resolution window, then published at the deal's home exchange. Feedback can point at one extension step.

**Demo:** TBD at planning. The three-source view in the Hub is visible, but the plan for the first stories does not cover it yet.

## Change docs

- [roym-value-exchange](../../changes/roym-value-exchange/change.md): reputation, evidence and opinions (`VX-37` to `VX-44`, `VX-51` to `VX-53`).
- [roym-extensions](../../changes/roym-extensions/change.md): feedback on a step (`EX-9`).
- **To write first:** an ADR for the disclosure rule (outcomes need no consent; amounts do), then the reputation module change doc.

## Slices (first cut)

| Slice | Content | Needs |
| --- | --- | --- |
| G0 | Disclosure ADR. Reputation module change doc. | — |
| G1 | Evidence records on the exchange: publish with an agreement, credential labels, responses, withdrawals. | G0, M08F exchange, M08C disclosure |
| G2 | Opinions from both parties: sealed writing, reveal, resolution window, publish at the home exchange (through a friend if needed). | G1 |
| G3 | Contact queries over Conversation, one level. | G0 |
| G4 | The three-source view in the Hub. Forwarded reads to friend exchanges. Close-out. | G1 to G3 |

## Open before planning

- How far source 1 (records the participant shows) goes in v1 (Value Exchange, open question).
- How an asker hides which party it asks its contacts about.
- The evidence size cap, measured on a real record.
