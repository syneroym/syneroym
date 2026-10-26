# M08: The Roym value exchange — overview

*Reader: developers and the project owner, when they plan or build any part of Milestone 8.*

Milestone 8 turns Roym into an open product. Any client can use it. People can trade in many languages, in shops with images and filters, with deal flows that extensions can extend, with EEPs, with matching of wants to offerings, and with reputation. It is too large for one milestone, so it is split into eight sub-milestones, M08A to M08H. They are built in the order below. Each has its own folder with a `task.md` (the plan) and, once it starts, a `status.md` (progress).

This file gives the order and the reasons for it. It holds no design. The design is in the change docs.

## Change docs

| Change doc | Criteria IDs | Built in |
| --- | --- | --- |
| [roym-open-foundation](../changes/roym-open-foundation/change.md) | `OF-*` | M08A |
| [roym-multilingual](../changes/roym-multilingual/change.md) | `ML-*` | M08B |
| [roym-extensions](../changes/roym-extensions/change.md) | `EX-*` | M08C |
| [roym-message-forwarding](../changes/roym-message-forwarding/change.md) | `FW-*` | M08C |
| [roym-value-exchange](../changes/roym-value-exchange/change.md) (overview) | `VX-*` | M08C to M08H |
| [roym-shop](../changes/roym-shop/change.md) | `SH-*` | M08D |
| [roym-intent-matching](../changes/roym-intent-matching/change.md) | `IM-*` | M08D (intent fields), M08F |

The Value Exchange document is an overview. Before each of M08C, M08E, M08F and M08G starts, its module change doc is written (agreements, obligations, exchange and membership, reputation). M08H extends the obligations doc. Each module doc follows the overview's rules or records a deviation.

## Order

| # | Sub-milestone | Folder | Main content |
| --- | --- | --- | --- |
| 1 | M08A Open foundation | [M08A](M08A-open-foundation/task.md) | Participant rename, typed methods, batched outbound call, federated query library, event feed, remote clients |
| 2 | M08B Language and structure | [M08B](M08B-language-and-structure/task.md) | Hub string catalog and formats, taxonomy with attributes, language tags, search keys and FTS5, translation hook points |
| 3 | M08C Deal core and extensions | [M08C](M08C-deal-core-and-extensions/task.md) | Commitments with details, reserve hooks, selective disclosure, barter, event log, statement, extensions (scheduling, one-shot), widgets, forwarding |
| 4 | M08D Intents and shop | [M08D](M08D-intents-and-shop/task.md) | Offerings and wants as one intent record with in-return terms; images, attributes, variants, storefront, browse query |
| 5 | M08E EEPs between two parties | [M08E](M08E-eeps/task.md) | Revocation `effective_from`, signed mailbox `stored` receipt, the `obligation` service: issue, redemption, decline, set-off, split |
| 6 | M08F Discovery and matching | [M08F](M08F-discovery-and-matching/task.md) | Directory split into membership and exchange, wants and bids, matching, standing wants, notifications, friend exchanges |
| 7 | M08G Reputation | [M08G](M08G-reputation/task.md) | Disclosure ADR, evidence, two-sided opinions, contact queries, the three-source view |
| 8 | M08H EEP transfer | [M08H](M08H-eep-transfer/task.md) | Endorsement and reissue, bundles, two-party swaps, EEPs listed for trade |
| later | Analytics and insight extensions | — | Exchange aggregates, suggestions on the node, insight extensions. Backlog rows. |

## Why this order

Each arrow means "must exist before".

```text
M08A rename + typed methods ──▶ every later wire change (renamed once, typed once)
M08A spike (epoch deadline) ──▶ call-all, or the two-step fallback
M08A call-all + federated query ──▶ M08F exchange forwarding to friends
M08A event feed ──▶ M08C extension services, M08F match events
M08B taxonomy + attributes ──▶ M08D shop attributes and facets ──▶ M08F hard filters
M08B search keys + FTS5 ──▶ M08D browse query ──▶ M08F relevance
M08B language tags ──▶ M08C step records, widgets, forwarding origin
M08C commitments + details ──▶ M08E EEP commitments, M08G evidence per commitment
M08C selective disclosure ──▶ M08G published evidence (amounts hidden)
M08C reserve hooks ──▶ scheduling moved out once, before M08D reshapes listings
M08D intent record ──▶ M08F wants and bids
M08F exchange ──▶ M08G evidence and opinions published to the home exchange
M08F discovery + M08G reputation ──▶ M08H EEP transfer between strangers
```

Four choices in the order deserve a note:

- **The rename and typed methods come first.** Every later sub-milestone adds fields to the same methods. Doing the rename later means renaming everything that M8 added too. The rename (A1) is merged alone, before any other M08A PR opens, because it touches almost every Roym file.
- **The epoch-deadline spike runs first, and has a fallback.** The order call-all, then the federated query library, then M08F forwarding depends on a guest waiting in one host call without hitting its deadline. If the spike fails, M08A builds a two-step search instead (see the open-foundation change). Later sub-milestones use the library either way, so the order does not change.
- **Scheduling moves out of the core in M08C, not M08D.** The commitment model rewrites the booking flow anyway. Moving slots to an extension at the same time means the booking code is rewritten once.
- **Two-party EEPs (M08E) come before discovery (M08F). EEP transfer (M08H) comes after reputation (M08G).** Two-party EEPs need only agreements, and they help people who already trade. Transfer is useful only when strangers can find each other and judge an issuer from published evidence. The order is about user value only. It does not reopen the legal decision (no legal gate, owner decision 2026-10-26).

## What can run in parallel

- Inside M08A, the S0 spike runs while the rename (A1) is built. After A1 merges, the remote-client work and the batched call can run beside the typed-method work. See its `task.md`.
- M08B's Hub string catalog touches only the Hub. It can start while M08A's last slices run.
- Message forwarding (in M08C) depends only on M08B's language tags. It can be built at any time after M08B.

## Rules for every sub-milestone

- Each slice is one PR that passes `mise run verify`.
- Slice implementation plans are written just before the slice, in the sub-milestone folder, as in M06C and M06D.
- Code-vs-plan differences go in the change doc's `## Deviations` section at the moment they are found.
- Living docs (requirements, architecture, integrated spec, developer guide, terminology) are updated at each change doc's close-out.
