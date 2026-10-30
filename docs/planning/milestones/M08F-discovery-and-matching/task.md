# M08F: Discovery and matching

*Reader: developers and the project owner, when they plan this sub-milestone.*

**Status: outline (2026-10-26).** This is a first outline. The full plan is written when M08E is near its end. Milestone overview and order: [M08-overview.md](../M08-overview.md).

## Goal

The directory splits into a membership authority and an exchange. Participants publish wants and receive bids. Exchanges match wants and offerings in both directions as they change, keep standing wants until their close time, notify the parties through the mailbox, and forward searches and wants one hop to friend exchanges.

**Demo:** New story for the directory. A viewer sees a participant publish to an exchange and another participant find the listing by search.

## Change docs

- [roym-value-exchange](../../changes/roym-value-exchange/change.md): Want-Bid, membership and exchange, federation (`VX-3` to `VX-5`, `VX-30` to `VX-36`).
- [roym-intent-matching](../../changes/roym-intent-matching/change.md) (`IM-3` to `IM-20`).
- **To write first:** the exchange and membership module change doc (publishing policy, standing feed per authority, wire-reachable method lists).

## Slices (first cut)

| Slice | Content | Needs |
| --- | --- | --- |
| F0 | Exchange and membership module change doc. | — |
| F1 | Split `directory` into `membership` and `exchange`. Publishing policy. Standing by remote call with a cache. Client list of trusted authorities. | F0 |
| F2 | Wants and bids: publish a want, bid, compare bids, revise, accept. Hub screens. | M08D intents, M08C agreements |
| F3 | Hard filters (browse query, time window, in-return check) and FTS5 relevance at the exchange. Personal ranking on the node. | F1, M08D browse |
| F4 | Standing wants, matching in both directions, the `inbox` service (`public`, one wire method `notify`) with the mailbox, the exchange allow-list, rate limits, `match` events. | F3, M08A feed |
| F5 | Friend exchanges: one-hop forwarding of searches and wants with the federated query library, the v1 merge rule, timeouts. Close-out. | F4, M08A `call-all` |

## Open before planning

- Bids on one want: does a want show the bid count, and can a wanter close a want without answering each bid (Value Exchange, open question).
- Friend timeout and cap per friend.
- Opt-in default and rate limits for match notifications (matching change).
- Showcase and fuel. May an exchange show a collage of offerings with links to provider shops, and let providers pay for slots with **fuel**? Fuel would be a balance held at one exchange, used only for that exchange's services. It could not be sent to another participant and could not pay for goods or services. Points to check before planning:
  - Fuel buys showcase space and delivery (for example offerer notifications). It must never change match results or their order. Promoted slots are marked and kept in a block apart from matches.
  - How fuel is obtained. At first, the operator may grant it by hand, with no payment inside Roym.
  - Where the balance lives (likely the exchange's member record, with one ledger row per spend), and what happens to unused fuel on leave or removal.
  - Friend exchanges charge their own fuel, or none. Fuel is local.
  - Keep it separate from EEPs and currency, so it does not become a second currency by accident.
