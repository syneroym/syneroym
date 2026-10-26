---
status: draft
living-docs-touched:
---
# Roym intent matching

*Reader: developers who plan and build Milestone 8, and the project owner.*

This change says how Roym finds offerings that fit a want, and wants that fit an offering. It covers both what matches and when the parties learn about a match. A text search alone cannot do this. A match must also fit the category, the area, the time and the price. It must also fit what each side can give in return: cash, goods, services or EEPs.

Terms used here:

- **Participant, offering, want, intent, exchange, EEP, resource, friend exchange:** see the [Value Exchange](../roym-value-exchange/change.md) change. An **intent** is an offering or a want.
- **Browse query:** the shared query shape defined in the [shop](../roym-shop/change.md) change.
- **Taxonomy, attribute:** category codes and typed attributes with labels per language (see the [multilingual](../roym-multilingual/change.md) and shop changes).
- **In-return terms:** what an intent can take or give in return. An offering lists what it **accepts**. A want lists what its author can **give**. Each entry is a resource kind: currency (with payment method codes), EEPs, or goods and services (by category code).
- **Hard filter:** a rule that a pair must pass to be a match at all.
- **Relevance score:** an exchange's own number for how well a pair fits. It is never a trust score.
- **Standing want:** a want that stays open at an exchange until its close time, so that new offerings are matched against it.
- **Match notification:** a message from an exchange to a participant's node that names a new match.
- **`inbox`:** a small Roym service on every participant node that receives match notifications from exchanges. It is the only part of matching that other nodes can call on a participant's node.

## Problem

The Value Exchange change adds wants and bids, and says an exchange "answers searches". It does not say:

- which offerings fit a want, beyond text and category;
- how the in-return terms of both sides are compared, for example a farmer who wants a carpenter and can pay in grain or EEPs;
- when a match is found. A want is raised once, but a good offering may appear a day later;
- who is told, and how, when the other party is offline.

Today's directory search keeps an ASCII-folded `text` field and filters with SQL `LIKE` (`crates/data_db/src/filter.rs:286`). It filters only by category, area, text, `open_to` and booking mode.

## Scope and non-goals

- In scope:
  - The intent fields that matching reads, including the in-return terms.
  - The hard filters, the relevance score, and the final ranking on the participant's node.
  - Matching in both directions at the exchange, driven by events: a new want against current offerings, and a new or changed offering against standing wants.
  - Standing wants at friend exchanges (one hop).
  - Match notifications, delivered through the mailbox when the receiver is offline, accepted only from the receiver's own exchanges.
  - A hook for semantic (embedding) scores and a hook for query translation. Both do nothing in M8.
- Not in scope:
  - Barter cycles of three or more parties. EEPs cover that need (see the Value Exchange change, netting).
  - Ranking by reputation at the exchange. The exchange finds. It never decides trust.
  - Timed re-runs of a search. Matching runs on events only.
  - Auctions, or automatic acceptance of a bid.
  - Embedding search itself. It waits for local inference (M9A).

## Acceptance criteria

| ID | Criterion | Test |
| --- | --- | --- |
| **Intent fields** | | |
| IM-1 | An offering carries in-return terms (`accepts`): currency with payment method codes, EEPs (yes or no), and goods or services by category code. A want carries `gives` with the same shape, and an optional budget in one currency. | |
| IM-2 | A want carries a category, optional required attribute values, an approximate area, a time window and a close time. | |
| **Hard filters** | | |
| IM-3 | A want W and an offering O match only when all of these hold: O's category is W's category or under it; O, or one of its variants, has every attribute value W requires; O's service area overlaps W's area, or O is remote; O is active and not withdrawn or expired before W's time window starts; O's price is within W's budget when both are stated in the same currency, or O is quote-only; and W's `gives` and O's `accepts` share at least one resource kind. | |
| IM-21 | The time filter checks only what the exchange holds: the offering's status and validity dates against the want's window. Whether a slot is free in that window is checked by the offerer's node when it writes a bid or a quote (the scheduling extension). The exchange never claims a slot is free. | |
| IM-4 | For goods and services in return, the kinds are shared when a category in W's `gives` is the same as, or under, a category in O's `accepts`. | |
| IM-5 | A pair that fails a hard filter is never returned, whatever its text score. | |
| **Ranking** | | |
| IM-6 | The exchange orders matches by its relevance score: text relevance over title, summary and description in any script, plus distance and freshness. Reputation is not part of the score. | |
| IM-7 | Results from several exchanges are merged by the rule in the Value Exchange change. Scores from different exchanges are never compared. | |
| IM-8 | The participant's node ranks the merged list again with its own data: contacts, past deals with each party, the languages each party speaks, and distance. This ranking never leaves the node. | |
| **When a match is found** | | |
| IM-9 | When a want is published, the exchange matches it once against current offerings and returns the first page to the wanter. | |
| IM-10 | Until its close time, a want is a standing want. When an offering is published or changed, the exchange matches it against standing wants and notifies each matching wanter once per offering version. | |
| IM-11 | When a want is published, the exchange also notifies the owners of matching offerings who opted in for that offering. | |
| IM-12 | An exchange forwards a want once to its friend exchanges. A friend keeps it as a standing want until its close time, and sends matches back to the forwarding exchange, which notifies the wanter. A friend never forwards it on. | |
| IM-13 | Closing a want, or its close time passing, ends all matching for it at every exchange that holds it. | |
| **Notifications** | | |
| IM-14 | A match notification is a call to a wire-reachable `notify` method on the receiver's `inbox` service, a small Roym service that does nothing else. It carries the matched intent's signed record and an idempotency key (want id, offering id, offering version). A repeated notification has no new effect. | |
| IM-15 | When the receiver is offline, the notification waits in the receiver's mailbox and is delivered when it wakes. Its time-to-live is the shorter of the want's remaining time and the mailbox's maximum. | |
| IM-16 | The receiver accepts a notification only from an exchange on its own list. Others are refused. | |
| IM-17 | Each exchange is rate-limited per receiver. A receiver can turn off offerer notifications for each offering. | |
| IM-18 | The receiver's node turns each accepted notification into a `match` event on its event feed (see [Roym open foundation](../roym-open-foundation/change.md)). | |
| **Hooks** | | |
| IM-19 | The exchange's scoring has a slot for a semantic score. In M8 the slot is empty and changes no order. | |
| IM-20 | Search calls the translation interface for the query text (see the multilingual change). In M8 it answers `unavailable`, and search uses the original text. | |

## Design

### Three steps

```text
  at the exchange                          on the participant's node
  ┌──────────────────┐   ┌──────────────┐   ┌─────────────────────────┐
  │ 1. hard filters  │──▶│ 2. relevance │──▶│ 3. personal ranking      │
  │ (browse query +  │   │ (text, area, │   │ (contacts, past deals,   │
  │  in-return fit)  │   │  freshness)  │   │  languages, distance)    │
  └──────────────────┘   └──────────────┘   └─────────────────────────┘
```

1. **Hard filters** reuse the shop change's browse query, plus the in-return check. They run in SQL at the exchange.
2. **Relevance** uses SQLite FTS5 (the multilingual change replaces today's `LIKE` search), plus distance and freshness. Each exchange tunes its own score. Query words shorter than 3 characters follow the multilingual change's short-word rule (ML-29), because the trigram index cannot match them.
3. **Personal ranking** runs on the participant's node, after the federated merge (see the Value Exchange change, principle "the exchange finds, it never decides trust", and VX-48).

### In-return check

Both sides are written as resource kinds:

```text
offering.accepts: [ currency{methods:[upi, cash]}, eep, goods{categories:[grain]} ]
want.gives:       [ eep, goods{categories:[wheat]} ]
```

`wheat` is under `grain` in the taxonomy, so this pair shares `goods` and also `eep`. The check says only that a deal is possible. The amounts and the exact issuer of an EEP are agreed in the quote.

### Event-driven matching

The exchange already stores the published offerings and wants. It runs matching when one of them changes. It does not run it on a timer.

| Event at the exchange | Matching | Who is notified |
| --- | --- | --- |
| Want published | the want against current offerings | the wanter (the answer to publish); offerers who opted in |
| Offering published or changed | the offering against standing wants | each matching wanter |
| Want forwarded from a friend | as "want published", at the friend | the forwarding exchange, which notifies the wanter |
| Want closed or past its close time | none; the want is removed | nobody |

The cost is bounded: an offering change is matched against the standing wants in its category subtree and area, with an index on both.

### Notification delivery

- The receiving method is `notify` on a new, small Roym service, **`inbox`**. It is wire-reachable, mailbox-eligible (listed in the service's `mailbox_methods`) and returns no data. `inbox` is deployed `public`, because only a public service has a mailbox (mailbox change, `D-C3-4`).
- **Why not `catalog`.** `catalog` is local-only today (`roym_core::admit::require_internal`). Making it `public` would put its mailbox and a wire path on the network next to the participant's own offering data. `inbox` holds only received notifications. Its one wire method checks the caller and stores the notification. It reads nothing else.
- **Security notes.** `inbox` adds one network-facing service to every participant node. Its exposure is bounded by: the caller allow-list (the participant's own exchanges, IM-16), a rate limit per exchange (IM-17), the mailbox's own quotas, and a size cap per notification. It holds no keys and calls no other service except to raise the `match` event.
- The caller's DID comes from the transport, as for every wire call. The receiver checks it against its own list of exchanges.
- The notification carries the matched intent's signed record, so the receiver can verify it and show it even if the exchange is offline later.

### Hooks for later

- **Semantic score.** A field in the exchange's score, filled by an embedding model when local inference exists (M9A). Empty in M8.
- **Query translation.** Search passes the query text to the translation interface. With no engine, it answers `unavailable`, and search uses the original text.

## Rejected options

- **Text or vector search alone.** It ignores category, area, time, price and the in-return fit, so it returns pairs that cannot trade.
- **Timed re-runs of each want** by the wanter's node. It costs a search per want per period, often finds nothing new, and misses offerings between runs.
- **Matching on the wanter's node only.** The node does not see new offerings at an exchange until it asks.
- **Ranking by reputation at the exchange.** It would make the exchange a trust authority. Each reader judges trust on their own node.
- **Searching for barter cycles.** Rare in practice, costly, and needs one solver that sees everyone's intents. EEPs remove the need.

## Open questions

- **Opt-in default for offerer notifications.** Recommendation: off by default, turned on per offering.
- **Rate limits.** A first value: 20 match notifications per exchange per receiver per day.
- **Eligibility.** An offering open only to members of a group (`open_to = members`) cannot be checked at the exchange unless the wanter's credential is known there. Recommendation: the exchange checks it when the wanter is its own member, and otherwise the wanter's node checks it before a bid or enquiry.

## Tasks

- [ ] Write the slice plans (in M08F, together with wants and bids from the Value Exchange change).
- [ ] In-return terms on offerings and wants (built in M08D with the intent record).
- [ ] Hard filters and FTS5 relevance at the exchange.
- [ ] Standing wants, both match directions, forwarding to friends.
- [ ] The `inbox` service with its `notify` method, mailbox path, exchange allow-list, rate limits, size cap, `match` events.
- [ ] The time filter (IM-3, IM-21).
- [ ] Personal ranking on the participant's node.
- [ ] Hooks for semantic score and query translation.

## Deviations

## Close-out

Before merge, update the living docs (requirements `[P2P-DSC]`, integrated spec search section, architecture, terminology) so they describe the result. Then set `status: done` and fill in `living-docs-touched`. Record anything you postponed in [deferred-backlog.md](../../deferred-backlog.md).
