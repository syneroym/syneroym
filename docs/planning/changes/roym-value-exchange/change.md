---
status: draft
living-docs-touched:
---
# Roym Value Exchange

*Reader: developers and the project owner, when they plan Milestone 8 (federation and trust).*

This is an **overview** change document. It gives the features, the terms, the main design rules and the architecture of the Value Exchange. When Milestone 8 is planned, each module below gets its own change document with exact formats, methods and tests. Those documents must follow the rules here, or record a deviation.

Read first: [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md) (Roym today), [reputation-design-note.md](../../../reputation-design-note.md) (reputation ideas), and the [service mailbox](../service-mailbox/change.md) change (store-and-forward delivery, which IOUs depend on).

## Problem

Today Roym has two fixed roles. A **provider** lists offerings. A **consumer** searches a SynOrg directory, agrees terms and pays. Payment is always in a currency, outside Roym. Roym only records what both sides say about it (`payment-acknowledgement`).

This leaves three groups of people out:

- **People with skills or goods but little money.** A teacher and a farmer who could serve each other cannot trade if neither has cash now. Roym has no way to pay with a promise of future goods or work.
- **People who want something but cannot find it by search.** A consumer can search offerings. They cannot post a need and let providers come to them.
- **People outside one group.** Search goes to the directories a consumer added by hand. Directories do not know about each other. There is no reputation beyond membership credentials and the receipts each person holds.

The requirements already promise parts of this: mutual credit (`[APP-ESC]`), a matching fabric (`[P2P-DSC]`) and peer reputation (`[P2P-REP]`). They are not built, and their current text does not fit together.

This change merges them into one design: the **Roym Value Exchange**.

## What the reader can do after this change

- Anyone is a **participant**. A participant can offer, want, buy, sell and issue promises. There is no separate consumer or provider role.
- A participant lists **offerings** and **wants** on one or more **exchanges**, and searches them.
- A deal can be paid in several **mediums** at once: currency, barter, a new IOU, or an IOU the payer already holds.
- A participant can issue an **IOU**: a signed promise, with a face value, that can be redeemed against the issuer's own offerings. IOUs can change hands. Roym tracks them as first-class records.
- Before a deal, a participant can judge the other party from **evidence**: records the party shows, records from their own contacts, and records that earlier counterparties published to the exchange.
- Exchanges can be **friends** with other exchanges, so a search that finds nothing locally can continue elsewhere.
- A participant can export a **statement** of everything they gave and received, in every medium, with a declared value. This is the raw data for tax. Roym computes no tax.

## Terms

- **Participant:** a person or business with a Roym identity (a master DID). Replaces "consumer" and "provider" as roles. The words remain useful for the side someone takes in one deal.
- **SynOrg:** an independent group (guild, cooperative, community) that runs services for its members. See [TERMINOLOGY.md](../../../TERMINOLOGY.md).
- **Membership authority:** a SynOrg role. It issues membership credentials, revocations and moderation decisions about members. This exists today as the trust half of the Directory.
- **Exchange:** a SynOrg role. It holds published offerings and wants, answers searches, stores published evidence, computes aggregates and lists friend exchanges. It replaces the search half of the Directory. It holds no balances and no money.
- **Offering:** what a participant offers (today's `listing`).
- **Want:** what a participant needs, published so others can find it and bid.
- **Intent:** the common word for an offering or a want. (From Valueflows, an open vocabulary for economic networks.)
- **Agreement:** the signed terms both parties accepted. It is made of commitments.
- **Commitment:** one promise inside an agreement, from one party to the other: "P delivers the service", "C pays ₹500", "C issues an IOU of ₹300". Each commitment has a **kind** and is settled by its own record.
- **Economic event:** the signed record that settles a commitment: a fulfilment receipt, a payment acknowledgement, an issued note.
- **Medium:** how a commitment is paid: currency, barter, IOU issue, IOU transfer, IOU return.
- **Declared value:** the value of a commitment in the agreement's currency, in minor units. For barter it is what the deliverer would charge in currency.
- **IOU (note):** a signed promise by its **issuer**. It names its current **holder**, a face value and an expiry. The holder can redeem it against the issuer's offerings, or pass it on.
- **Reissue:** the issuer signs a new version of a note that names a new holder. A transfer is final only when the issuer reissues.
- **Evidence:** signed records about how a party behaved: receipts, declines, refusals, and published evidence records.
- **Standing card:** a signed summary a participant chooses to show a counterparty: time active, number of distinct counterparties, outstanding IOUs (stated by the issuer), and similar facts.
- **Friend exchange:** another exchange that an exchange chooses to point its users to.
- **Policy hook:** a fixed point in the deal flow where a policy package (for example a tax package) may observe what happened.

## Principles

These hold for every module.

1. **Facts are hard, trust is soft.** A signature, a version or a record either verifies or it does not. Whether to accept a party, a medium or an IOU is always the participant's own choice. Roym ships no default acceptance policy and no score.
2. **No shared ledger.** No blockchain, no global balance, no clearing house. Each record has one writer.
3. **The exchange finds. It never decides trust.** The participant's own node verifies every signature, credential and record. This is today's rule for directories, unchanged.
4. **Every deal is between two parties.** An IOU transfer also needs the issuer's reissue, which is a separate step by the issuer. Effects across many parties, such as netting, come only from bilateral steps.
5. **Evidence, not scores.** Exchanges may compute aggregates. They are labelled as one party's calculation, never as truth.
6. **No evidence is not good evidence.** A new identity with no history shows as "no evidence", never as neutral or positive.
7. **IOUs and barter are enforced by reputation, not by Roym.** Roym records them and makes kept and broken promises visible. It makes no claim about legal enforceability.
8. **Credit grows only as far as people believe each issuer can deliver.** No interest, no free issuance, no pooled risk.

## Scope and non-goals

- In scope:
  - The participant role, offerings and wants, bids on wants.
  - Agreements made of commitments in several mediums, with declared values.
  - Selective disclosure of agreement fields.
  - IOUs: issue, transfer and reissue, redemption, split, set-off, expiry.
  - Splitting the Directory into a membership authority and an exchange.
  - Friend exchanges (shown to clients; no forwarding).
  - Evidence and reputation from three sources.
  - Policy hooks (observe only) and the statement export.
  - Exchange aggregates and suggestions computed on the participant's node.
  - A signed `stored` receipt from the mailbox.
- Not in scope:
  - Moving money, escrow, or verifying a currency payment. Currency and barter stay out of band.
  - Computing tax, or any policy package. v1 installs none.
  - A native token, a global credit balance, or a mutual-credit ledger. This replaces the "native ledger token" in `[APP-ESC]`.
  - An exchange forwarding a query to its friends.
  - Index sharding and deterministic placement across many exchange nodes (`[P2P-DSC]` leaf shards). Moved to Milestone 7.
  - Multi-party atomic swaps.
  - Pools of IOUs (as in Sarafu commitment pools).
  - Trading claims on future output as investments. This needs legal review first.
  - Witness committees, notaries, trusted execution environments and zero-knowledge proofs.
  - Pseudonymous participants. Only reviewers can be pseudonymous (see [Reputation](#reputation-and-evidence)).

## Acceptance criteria

These are overview criteria. Each module change document refines them into tests.

| ID | Criterion | Test |
| --- | --- | --- |
| **Participants and intents** | | |
| VX-1 | One identity can publish an offering, publish a want, buy and sell, with no role switch. | |
| VX-2 | A want is published to an exchange and found by search. Another participant answers it with a quote, and the flow continues as for any quote. | |
| VX-3 | A public want shows only an approximate area. The exact address is shared only inside a conversation, as today. | |
| **Agreements** | | |
| VX-4 | One agreement can hold commitments in several mediums, for example a service, ₹300 in currency and ₹200 as a new IOU. Each commitment carries its kind, its two parties and its declared value. | |
| VX-5 | Each commitment moves only through the states of its kind (see the table under Design). An agreement closes when every commitment has reached a final state. | |
| VX-6 | The participant who wrote the quote is the single writer of the agreement. A second, conflicting change gets a named conflict, never a merge. | |
| VX-7 | A party can show an agreement with chosen fields hidden (for example the price), and the signature still verifies. | |
| VX-8 | Reputation shows which party failed which commitment, not only that a deal failed. | |
| **IOUs** | | |
| VX-9 | A participant with no published offering cannot issue an IOU. | |
| VX-10 | An IOU's face value is in a currency that appears in the issuer's own offerings, in integer minor units. | |
| VX-11 | A holder passes a note to a new holder. The issuer reissues it to the new holder, also when the issuer was offline and the request waited in its mailbox. | |
| VX-12 | If one note version is passed to two people, the issuer reissues only the first valid request it processes. The other receives a signed refusal plus the two signed endorsements as proof against the double spender. | |
| VX-13 | The issuer refuses a reissue only for a double spend or an expired note, and every refusal is signed. | |
| VX-14 | A repeated reissue request returns the same reissued note. It never creates a second version. | |
| VX-15 | An issuer restored from an old backup never reissues from its stale state. A holder's newer version, signed by the issuer, brings the issuer up to date. | |
| VX-16 | A note still verifies after the delegated key that signed it has expired. | |
| VX-17 | Splitting a note gives parts whose face values add up exactly to the original. | |
| VX-18 | A holder redeems a note at no more than the issuer's signed list price at that moment. A decline is recorded with the issuer's optional reason. | |
| VX-19 | A participant can set a private limit per issuer. A quote that would go over it shows a warning. It does not block. | |
| VX-20 | Two parties who hold each other's notes in the same currency can cancel them against each other in one bilateral step. | |
| **Membership and exchange** | | |
| VX-21 | A SynOrg can run a membership authority without an exchange, and an exchange without a membership authority. | |
| VX-22 | An exchange can accept credentials from another SynOrg's authority. A revocation there removes the member's offerings from search within the stated cache time. The client's own check shows the revocation at its next look. | |
| VX-23 | A credential from an authority the client has not added shows as "unknown authority", never as positive. | |
| **Federation** | | |
| VX-24 | When a search finds nothing, the client can see the friend exchanges that this exchange chooses to show, and search them directly. | |
| **Reputation and evidence** | | |
| VX-25 | A counterparty can publish evidence about a participant to an exchange only with an agreement that names both of them. | |
| VX-26 | The subject can attach a signed response. A withdrawal by the publisher stays visible as a withdrawal. | |
| VX-27 | The reader sees evidence from the three sources side by side, with each source labelled. They are never merged into one number. | |
| VX-28 | A reviewer can sign evidence with a context key, so readers do not learn the reviewer's master DID. | |
| **Policy, statement, analytics** | | |
| VX-29 | With no policy package installed, every deal works and nothing is blocked. | |
| VX-30 | A participant exports a statement for a period: every commitment given and received, its medium and its declared value. | |
| VX-31 | An exchange shares an aggregate only when it covers at least a minimum number of participants. | |
| VX-32 | Suggestions for one participant are computed on that participant's node, never on the exchange. | |
| **Mailbox** | | |
| VX-33 | The mailbox signs its `stored` receipt, so a holder can prove a demand was deposited for the issuer. | |

## Design

### Architecture

Seven modules. They are separate in data and API, and they work closely together. "Where it runs" says which node holds the data.

| Module | Where it runs | Owns | Built from today's |
| --- | --- | --- | --- |
| **Intents** | every participant | offerings, wants, prices, accepted mediums | `catalog` |
| **Agreements** | the quote author's node | requests, quotes, agreements, commitments and their states | `transaction` |
| **Obligations** | every participant | notes it issued (as the single writer), notes it holds, endorsements, refusals | new |
| **Exchange** | a SynOrg that enables it | published intents, search index, published evidence, aggregates, friend list | search half of `directory` |
| **Membership** | a SynOrg that enables it | members, credentials, revocations, member-level moderation | trust half of `directory` |
| **Reputation** | every participant | the reader's view: gathers evidence from three sources and shows it | client half of `directory`, plus new |
| **Policy** | every participant | hook points, installed packages, the statement export | new |

Analytics is not a module of its own. The exchange computes aggregates over what it holds. The participant's node computes suggestions from those aggregates and its own records.

```text
            ┌──────────── SynOrg ────────────┐       ┌──── friend SynOrg ────┐
            │  Membership        Exchange    │       │  Exchange             │
            │  credentials ──▶   intents     │       │  intents, evidence    │
            │  revocations       evidence    │◀─────▶│  (listed as a friend) │
            │                    aggregates  │       └───────────────────────┘
            └───────▲───────────────▲────────┘
       publish /    │               │ search, evidence, aggregates
       standing     │               │ (client fans out to each exchange)
            ┌───────┴───────────────┴────────┐
            │      Participant A node         │      direct, end to end
            │  Intents  Agreements            │◀───────────────────────▶ Participant B node
            │  Obligations  Reputation        │   quote, agreement,
            │  Policy (hooks, statement)      │   endorsements, reissue
            └─────────────────────────────────┘   (mailbox when offline)
```

All modules stay inside the Roym SynApp. The requirements rule out separate ledger or marketplace SynApps.

Proposed service changes, to be confirmed in the module documents:

- `roym_directory` splits into a **membership** service and an **exchange** service. The client half (sources, fan-out, merge) moves with the exchange service.
- A new **obligation** service on every participant.
- `catalog` gains wants and accepted mediums. `transaction` gains commitments.

### Participants, offerings and wants

- Every participant runs the same services. "Provider" now means "the side that delivers in this deal".
- An **offering** is today's `listing`. Its `PaymentTerms` (`crates/roym_core/src/listing.rs:77`) gains the list of **accepted mediums**: currency methods, barter, IOUs. "Accepts IOUs" means IOUs at face value. Which issuer and how much are negotiated in the quote and fixed in the agreement. Acceptance rules are never published in detail.
- A **want** is a public request: categories, description, approximate area, time window, and the mediums the participant can pay in. It is published to exchanges like an offering.
- A **bid** is a quote that answers a want. It uses the normal quote → agreement flow. Bidders find wants by searching, in the same way that buyers find offerings.
- A holder can list a note for trade as an offering. The deal that follows has an IOU-transfer commitment against any other commitment: currency, barter, or another note.

### Agreements made of commitments

An agreement is a set of commitments. Each commitment names who owes it, who receives it, its kind and its declared value.

| Medium | Commitment kind | Settled by | States |
| --- | --- | --- | --- |
| Service in a time slot | booking | `fulfilment-receipt` | scheduled → in progress → done; or ended unconfirmed |
| Goods or service without a slot, including barter | delivery | `fulfilment-receipt` | promised → delivered; or ended unconfirmed |
| Currency | currency | `payment-acknowledgement` | requested → acknowledged; or ended unconfirmed |
| New IOU | iou-issue | the issued note, version 1 | promised → issued |
| IOU the payer holds | iou-transfer | endorsement, then the issuer's reissue | promised → endorsed → reissued; or refused |
| Note returned to its issuer (redemption or set-off) | iou-return | endorsement back to the issuer, then the issuer's closing record | promised → returned → closed |

The agreement itself has a short life: requested → quoted → agreed → closed, or expired, or cancelled. It closes when every commitment is final.

Rules:

- **The list of kinds is fixed in code.** Offerings choose a kind (booking or delivery). Strangers can then compare "delivered" across all providers, and reputation and the statement can read every state. A configurable workflow engine is rejected (see [Rejected options](#rejected-options)). A new kind is added in code when a real need appears.
- **One writer per record.** The quote author's node writes the agreement and the commitment states. This generalises today's rule that the provider's Transaction service is the single writer. The issuer writes each note. One deal can therefore involve two writers, each for its own record. The agreement's IOU commitment names the note id and version. The note names the agreement's content hash. The agreement writer marks an IOU commitment settled when it holds the issuer's signed note that names the right holder.
- **Cancellation:** only the quote author, and only while no commitment has moved. This generalises today's rule.
- **Every commitment carries a declared value** in the agreement's currency, in minor units. The statement export and the policy hooks use it.
- **Selective disclosure.** Each agreement field is hashed with its own random salt, and the signature covers the list of hashes. To show the agreement to a third party, a party reveals the fields it chooses, with their salts. The others stay hidden, and the signature still verifies. This changes the canonical bytes of `agreement-receipt`, so it is done before any IOU or evidence record depends on it. The product is not released, so the record changes in place, with no migration.

### IOUs

#### The note

A note is a signed record. Each version is a new record that supersedes the previous one.

```text
id          hash of version 1
issuer      master DID of the issuer
holder      master DID of the current holder
version     1, 2, 3 …   raised by each reissue
currency    ISO 4217 code, one that appears in the issuer's own offerings
face_value  integer, minor units
expiry      the issuer's duty ends at this time
agreement   content hash of the agreement that created this version
```

The fields are provisional. The obligation change document fixes them.

- **Unit and thumb rule.** The currency is used **only as a unit of account**. The issuer does not owe money. "₹300 from X" means "what ₹300 buys at X's list prices when I redeem". The issuer sets the face value at what the same goods cost in that currency at its list price today.
- **The issuer must have an offering.** A participant with no published offering cannot issue. A promise must point to something the issuer offers.
- **Capabilities are indicators only.** "Wheat" or "carpentry" is what the issuer's offerings say, proven over time by redemptions. They are not part of the promise.
- **Every note can be transferred.** There is no non-transferable flag.
- **No interest.** A note promises its face value. Any time value appears only as a discount that two parties choose when they trade a note.

#### Issue

The payer is the issuer. It signs version 1, naming the payee as holder, and sends it as the settlement of an `iou-issue` commitment.

#### Transfer and reissue

1. The holder signs an **endorsement**: "note N, version v, now held by H2". It is the settlement step of an `iou-transfer` commitment.
2. H2's node sends a **reissue request** to the issuer: directly, or through the issuer's mailbox when the issuer is offline. The request is mailbox-eligible: it carries an idempotency key and returns no data.
3. The issuer checks the chain from the last version it signed, and signs version v+1 naming H2. It sends the note back to H2, through H2's mailbox if needed.

Rules for the issuer:

- **Duty to reissue.** The issuer must reissue the first valid request it processes for each version. It may refuse only when the version was already reissued (a double spend) or the note has expired. Every refusal is a signed record. A refusal for any other reason counts against the issuer as evidence.
- **First processed wins.** The mailbox gives no order between different senders, so "first" means the issuer's own processing order. That is enough, because the issuer is the only record of its notes. The losing holder receives the signed refusal plus the double spender's two endorsements.
- **Idempotent.** The key is `(note id, version, endorsement hash)`. A repeated request returns the same reissued note.
- **Lock one note, never all of the issuer's records.**
- **Restore from an old backup.** An issuer that restored an old backup may not know about versions it signed later. When a holder shows a version that is newer than the issuer's record, and the issuer's own signature verifies, the issuer accepts that version as the truth and catches up. It never reissues from its stale state.
- **Chain length.** Until the issuer reissues, each new holder adds an endorsement, so a note can pass through several hands while the issuer is offline. Each holder sees the earlier holders in the chain. A reissue resets the chain.

Rules for the payee:

- **Accept now or wait.** The payee chooses. It can deliver at once and carry the risk until the reissue arrives. Or it can deliver after the reissue. Roym fulfilment usually comes after payment, so waiting fits most deals. The booking card shows "payment pending issuer confirmation".
- **Per-issuer limit.** A participant can set a private limit for each issuer, for example "at most ₹2,000 of X's notes". A quote that would go over it shows a warning. It never blocks.

#### Redemption, decline and set-off

- **List price is a maximum.** The holder redeems at no more than the issuer's signed list price that is current when the holder asks. The issuer may give a better deal. A refusal to go below the list price is normal and is not evidence against the issuer.
- **The issuer chooses which offerings to accept notes for, at that time.** For example, ten offerings may accept notes today and five next week. Fewer offerings is a **decline**, recorded with the issuer's optional reason. The reader decides whether it was good, bad or acceptable.
- **No answer.** A demand the issuer never collected from its mailbox, with a delivery window that has ended, is weaker negative evidence than a signed decline. The mailbox's signed `stored` receipt proves the demand was deposited. Its `delivered` receipt, signed by the receiver, proves the issuer's node received it.
- **Set-off.** A holder who owes the issuer can return the issuer's note in payment. This is an `iou-return` commitment, the same as a redemption.
- **Split.** A note can be split into parts. The parts' face values add up exactly to the original, in integer minor units, and each part is a new version.
- **Expiry.** After expiry the issuer's duty ends, and it may delete its records of that note. A demand made inside the window and not met stays as evidence with the holder.

#### Netting

Netting comes from bilateral steps only. Example: A owes B, B owes C, C owes A.

1. B pays C with A's note, and C returns B's note. B is now even.
2. C now holds A's note, and A holds C's note. They set off against each other.

Each step is a fair trade on its own, so a chain that stops half-way leaves nobody worse off. No multi-party atomic swap is needed. Only a two-party swap is atomic: one swap record holds both endorsements and is valid only with both signatures. Netting works only between notes in the same currency. Across currencies, the two parties agree a rate for that step. The participant's node may suggest a step ("you hold X's note, and you owe X"). No node sees the whole graph.

#### Over-issuance

An issuer may knowingly issue more than it can redeem at once, as an airline overbooks. Roym does not prevent it. It makes it visible in two ways:

- **Redemption behaviour.** Each delivery, decline, refusal and unanswered demand is signed evidence from holders. This is the faithful signal.
- **A stated total.** The issuer's standing card can show the total face value of its outstanding notes. The issuer's node holds every note, so this costs nothing. The card labels the total "stated by the issuer, not checked".

A published Merkle commitment that holders check was considered and dropped (see [Rejected options](#rejected-options)).

#### Signatures that outlive keys

Notes can live for years, but the substrate signs with delegated keys that expire (spec D7). A note carries its delegation certificate. The verifier checks that the note was signed **inside the certificate's window**, not that the certificate is valid now. Today's verifier already does this: `crates/signed_record/src/verify.rs:204` compares the record's `issued_at_secs` with the certificate window. `verify_chain_at` (`crates/identity/src/delegation.rs:203`) uses the current time only to reject a certificate issued in the future. One gap remains: a revoked signing key fails every record it ever signed, including honest notes signed before the revocation. See [Open questions](#open-questions).

#### What the issuer sees

Notes are not blind. The issuer sees every holder that asks it to reissue. Holders also see the earlier holders in a chain that has not yet been reissued. This is the price of accountability: every double spend produces signed proof against a named person. The UI says so.

### Membership and exchange

Today one node is both the membership authority and the directory. Search keeps only offerings whose member is in good standing with the directory's own issuer (`crates/roym_directory/src/app/search_ops.rs:210`). `directory.standing` serves a member's credential, revocations and suspensions (`crates/roym_directory/src/app.rs:78`). Splitting the roles gives these rules:

| Point | Rule |
| --- | --- |
| Joining rules | Belong to the SynOrg. The exchange has a **publishing policy** that may require membership of one or more authorities, for example "members of guild G may publish; anyone may search". |
| Standing | The exchange gets a member's standing by calling the authority's open standing method remotely, and caches it for a stated time. The method moves from the directory to the membership service. |
| Removal time | One SynOrg running both roles removes a suspended member at once, as today. Across SynOrgs, removal takes up to the cache time. The client's own check still shows the revocation at its next look. |
| Moderation | The exchange signs decisions about a listing on that exchange (spam, its own rules). The authority signs decisions about a member. Both use `moderation-decision`, with a different signer and scope. |
| Scope | The publishing policy checks that the credential's scope covers the offering's category. |
| Client trust | The client keeps its own list of trusted authorities, separate from its list of exchanges. A credential from an authority not on the list shows as "unknown authority". This replaces pinning "which group a directory speaks for". |
| Open exchanges | Allowed. Their results show "no credential". |
| Dependencies | The exchange calls authorities at runtime by DID, as clients call directories today. An authority depends on nothing. There is no cycle. |

### Federation

- A participant can add many exchanges. The client fans out to each one, as today.
- An exchange keeps a list of **friend exchanges** and chooses which ones to show. A client sees them mainly when a search finds nothing. The client then searches them directly and verifies everything itself.
- An exchange **does not forward** queries. Forwarding, hop limits and loop rules are later work.
- One exchange is one index. Sharding one exchange's index across many nodes moves to Milestone 7.
- Results from a friend exchange follow the same rules: credentials from unknown authorities show as unknown.

### Reputation and evidence

The reader's node gathers evidence from three sources and shows them **side by side, labelled, never merged into one number**.

**1. Records the participant shows.**
- Signed receipts, notes and declines that the participant holds, plus its standing card.
- The reader counts **distinct counterparties**, gives less weight to a counterparty that appears in many places in the reader's own view (inverse frequency), and counts mutual pairs (A rates B, B rates A) only once.
- How a reader can check that a participant shows its whole history, and not only the good part, is set out in [reputation-design-note.md](../../../reputation-design-note.md). That covers a signed count, receipt sequence numbers and random sampling. The reputation module document decides how much of it v1 needs.

**2. Evidence from the reader's contacts.**
- The reader's node asks its direct contacts, and they may ask theirs, up to three levels.
- Answers are **first-hand signed records only**, never opinions.
- Weight falls with each level. Distinct sources count, not distinct paths, and each source has a cap.
- No answer is not a penalty.
- A contact chooses whether to answer and how much to show. Questions are rate-limited.

**3. Evidence published to an exchange.**
- **No agreement, no publish.** The publisher must be a party to an agreement that names the subject. The subject must be listed on that exchange. The publisher need not be a member.
- The evidence record holds: the subject's signed agreement, with only the fields the publisher chooses to show; the outcome of each commitment; an optional short text; and pointers to attachments.
- The exchange stores this **small signed record**, up to a size cap (2 KB is a first value to tune). Large attachments stay with the publisher, behind a pointer.
- The subject can attach a **signed response**. This turns the evidence into a dispute.
- The publisher can **withdraw** only with a signed withdrawal, which stays visible with its label: settled otherwise, forgiven, or complaint withdrawn.
- The exchange never turns evidence into a verdict. Retention follows the exchange's stated policy.

**Reviewer pseudonyms.** A reviewer can sign agreements and evidence with a **context key**: a separate key for one context, such as a service category. The key's link to the reviewer's master DID is a delegation certificate that only the reviewer holds. Readers see a stable pseudonym, so the distinct-reviewer check still works. For a high-stakes decision, a reader can ask the reviewer to reveal the link. The answer is one of bound, refused or unavailable. Refusing is not misconduct.

**What evidence says about IOUs.** Each event points at one named party:

| Event | Evidence | About |
| --- | --- | --- |
| Redeemed | the holder's receipt | issuer, positive |
| Declined, with or without a reason | the issuer's signed decline | issuer; the reader judges |
| Unanswered past the window | mailbox `stored` receipt and the demand | issuer, weaker negative |
| Refused reissue | the issuer's signed refusal | the double spender if the reason holds; otherwise the issuer |
| One version endorsed twice | the two endorsements | the endorser |
| Released by the holder | a signed release with its label | issuer; the event stays, labelled, and fades over time |

**Known limit.** A farm of one-time fake identities, each with one fake deal, can still create good-looking evidence in source 1. Sources 2 and 3 are the defence: a farm has no real links into the reader's network, and no real counterparties who publish. The UI shows when only source 1 supports a party.

### Policy

- **Hook points, fixed in code:** agreement signed; commitment settled; note issued, transferred, returned.
- **v1 has observers only.** An observer receives the event and may write annotations and add lines to the statement. It cannot block, change or delay a deal.
- **Where it runs:** on the participant's node, because tax is the participant's duty and only the participant's node sees the full deal.
- **Who supplies a package:** later, an exchange, an accountant or a public body can supply a signed policy package. The sandbox runs it as a WASM component, and the participant chooses to install it. v1 installs none.
- **Pre-sign check:** a hook that can block a deal before signing, for example when a jurisdiction requires a tax id on invoices. It is added only when a real policy needs it.
- **Tax is not an acceptance rule.** Acceptance happens before a deal and is decided by a person at quote time. Tax is computed after a commitment settles.
- **Statement export (built in):** for a chosen period, every commitment given and received, with its medium, counterparty, currency and declared value.

### Analytics

- An exchange computes aggregates only over what it holds: published intents, published evidence and counts of searches. Examples: the median review rate per category, supply and demand gaps per category and area, flagged clusters of fake reviewers, and redemption statistics per issuer from published evidence.
- An aggregate is shared, with users or with friend exchanges, only when it covers at least a minimum number of participants (a first value to tune).
- **Suggestions for one participant are computed on that participant's node**, from the exchange's aggregates and the participant's own records. The exchange does not profile individuals.
- Aggregates are labelled as the exchange's calculation.

### Mailbox change

IOU reissue and redemption demands use the [service mailbox](../service-mailbox/change.md). One addition is needed. Today the mailbox answers `deposit` with `stored`, and the change document does not say that this receipt is signed (unverified against the code). This change asks the mailbox to **sign** its `stored` receipt: deposit id, destination, message hash and time. The mailbox is a third party, so its signature makes "demand sent, never collected" checkable by others.

### Lessons from existing systems

| System | What we take | What we avoid |
| --- | --- | --- |
| Community Exchange System (CES) | Each community runs itself; a trade between exchanges should feel like a local one | A central bridge and a central exchange-rate setter; unsettled imbalances between exchanges. Our exchanges hold no balances, so neither applies. |
| Credit Commons Protocol | A tiny, fixed set of states; rounding rules decided before code | A workflow that every node on a path must share; an admin who undoes completed deals |
| Circles | Each holder decides whom to accept, with a limit | Free value for every account, which made fake accounts pay off |
| XRP Ledger issued currencies | A holder limit per issuer; issuer powers that are visible | "Rippling", where balances moved through accounts not taking part in the payment. Only our issuer reissues, so this cannot happen. |
| Holochain mutual credit and Valueflows | The words intent, commitment, economic event | Countersigning, which does not stop double spends; importing the whole Valueflows model |
| Cashu and Fedimint | Reissue = invalidate the old note and issue the new one in one step; a final expiry ends the issuer's duty | Undefined half-finished swaps, and bugs in backup and restore. Hence the idempotency and restore rules above. |
| Multilateral trade credit set-off (MTCS), Cycles | Loops of debt can be cancelled with consent | One solver that sees everyone's debts |

### Rejected options

- **One global credit balance per identity (mutual credit).** It loses who owes whom. A balance can prove "gave at least X" but not "has X left", because nothing stops showing an old history.
- **Circles or a clearing service per group.** It needs groups and an operator. Acceptance is the peer's own choice.
- **Witness committees for notes.** Waiting for the issuer's reissue already stops a double transfer. Witnesses cannot stop over-issuance, which is not a fork. They would also bring an unsolved problem: who chooses the witnesses.
- **A published Merkle sum-tree commitment of outstanding notes.** It is real work, and it proves only a lower bound. The outstanding total cannot separate "can deliver" from "holders went dormant". Redemption evidence is the faithful signal. Recorded as a later option.
- **A configurable state-machine engine per offering.** Strangers could not compare states, and evidence would lose meaning.
- **An independent unit of account.** The rule "redeem at list price" has no meaning unless notes and offerings use the same unit, and offerings are priced in a currency.
- **A non-transferable flag on notes.** Not needed. Transfer is always confirmed by the issuer.
- **Multi-party atomic swaps.** Bilateral steps give the same netting, and each step is safe alone.
- **Exchange forwarding of queries.** Later. Client fan-out first.
- **Pointer-only evidence on the exchange.** Evidence would vanish when the publisher is offline, often a phone. A subject could also press the publisher to delete it quietly.
- **A provider-side trusted execution environment or a notary to vouch for reviews.** The owner controls the hardware, and a notary learns who reviewed whom. Context keys with signatures need neither.
- **Tax as an acceptance rule.** It would give tax the power to block deals, which v1 does not need.

## Open questions

- **Revocation and old records.** A revoked signing key fails every record it signed, including honest notes signed before the revocation (`crates/signed_record/src/verify.rs`, the `check_did` calls). Should a revocation carry a "revoked since" time, so that records signed before it stay valid? This matters most for notes, which live for years.
- **Satisfaction in the fulfilment receipt.** Should the consumer-signed fulfilment receipt carry a coarse rating (0, 1, 2 or none)? Hiding a bad rating would then mean hiding the whole fulfilment. The cost is pressure on the consumer at signing time.
- **How far source 1 goes in v1.** Signed counts, receipt sequence numbers and random sampling (from the reputation design note), or only distinct-counterparty counts over what the participant shows.
- **Asking contacts.** Which transport carries a contact query, and how the asker hides which party it is interested in.
- **Context keys in the substrate.** Today the substrate signs as the master DID under delegation (spec D7). Context keys need a second kind of key whose link to the master is not published. Where is it stored, and how is it backed up?
- **Pseudonyms and notes together.** A note names its issuer and holder by master DID. If a reviewer signs an agreement with a context key and the same agreement has an IOU commitment, the note id in published evidence links the pseudonym to a master DID. Published evidence should then hide the IOU fields by default. Confirm this in the reputation module document.
- **First values to tune:** the evidence size cap, the standing cache time, the minimum group size for aggregates, the default note expiry, the redemption notice period.
- **Service names and boundaries:** whether wants live in `catalog` or in their own service, and whether reputation is a service or a client library.
- **Legal review** of trading notes between strangers, before any feature that presents notes as investments.

## Tasks

Phases are in build order. Each phase gets its own change document when Milestone 8 is planned.

- [ ] **Phase 1, agreements.** Commitments with kinds, states and declared values. Quote author as the single writer. Selective disclosure in `agreement-receipt`. Statement export.
- [ ] **Phase 2, intents and exchange.** Participant role. Wants and bids. Accepted mediums in offerings. Split the directory into membership and exchange, with the rules in the membership and exchange table. Friend exchange list.
- [ ] **Phase 3, obligations.** Notes: issue, transfer and reissue, refusals, redemption, decline, set-off, split, expiry. Mailbox paths. Idempotency and the restore rule. Per-issuer limit. Stated outstanding total on the standing card. Signed `stored` receipt in the mailbox.
- [ ] **Phase 4, reputation and evidence.** Evidence records, responses and withdrawals on the exchange. Context keys. Contact queries. The three-source view.
- [ ] **Phase 5, policy and analytics.** Observer hooks. Exchange aggregates with the minimum group size. Suggestions on the participant's node.
- [ ] When this document is approved: add rows to [deferred-backlog.md](../../deferred-backlog.md) for each non-goal that is postponed (exchange forwarding, index sharding to Milestone 7, the Merkle commitment option, IOU pools, future-output trading after legal review, the pre-sign policy hook), and update the Milestone 8 row in [roadmap.md](../../roadmap.md).

## Deviations

## Close-out

Before merge, update the living docs so they describe the result, then set `status: done` and fill in `living-docs-touched`. Expected changes:

- `docs/system-requirements-spec.md`: replace the native ledger token in `[APP-ESC]` with IOUs; replace the EMA score in `[P2P-REP]` with the three-source evidence view; move the leaf shards of `[P2P-DSC]` to Milestone 7; add the participant role, wants, commitments and the policy hooks.
- `docs/roym-integrated-experience-spec.md`: roles, the Records table, the transaction state, search (D1), and the "Not in the first release" list.
- `docs/system-architecture.md`: the new and split services.
- `docs/TERMINOLOGY.md`: the terms in this document.
- `docs/reputation-design-note.md`: align it with the reputation rules here.

Record anything postponed in [deferred-backlog.md](../../deferred-backlog.md).
