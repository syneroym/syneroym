---
status: draft
living-docs-touched:
---
# Roym Value Exchange

*Reader: developers and the project owner, when they plan Milestone 8 (the Roym value exchange).*

This is an **overview** change document. It gives the features, the terms, the main design rules and the architecture of the Value Exchange. When Milestone 8 is planned, each module below gets its own change document with exact formats, methods and tests. Those documents must follow the rules here, or record a deviation. Detail that is correct to leave to them is listed in [Notes for the module documents](#notes-for-the-module-documents).

Read first: [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md) (Roym today), [reputation-design-note.md](../../../reputation-design-note.md) (reputation ideas), and the [service mailbox](../service-mailbox/change.md) change (store-and-forward delivery, which EEPs depend on).

Related Milestone 8 changes, which this document relies on:

- [Roym open foundation](../roym-open-foundation/change.md): the participant rename, the federated query library (fan-out, merge, verification), the event feed, remote clients.
- [Roym extensions](../roym-extensions/change.md): the split between the deal core and extensions, commitment details, step records, reserve and release hooks.
- [Roym shop](../roym-shop/change.md): images, attributes, variants, storefront, and the browse query.
- [Roym intent matching](../roym-intent-matching/change.md): in-return terms, hard filters, ranking, standing wants and match notifications.
- [Roym message forwarding](../roym-message-forwarding/change.md) and [Roym multilingual foundation](../roym-multilingual/change.md).

**Owner decision (2026-10-26): no legal gate.** Roym builds EEP transfer, bundles and listing EEPs for trade. They come in M08H, after discovery and reputation, for user value only (see [Tasks](#tasks)). Legal and tax questions about EEPs belong to the participants and operators. Roym's job is to keep faithful, complete, signed records with the details a statement needs. Features that present EEPs as investments are still left out, as a product choice (see [EEPs and investment](#eeps-and-investment)).

## Problem

Today Roym has two fixed roles. A **provider** lists offerings. A **consumer** searches a SynOrg directory, agrees terms and pays. Payment is always in a currency, outside Roym. Roym only records what both sides say about it (`payment-acknowledgement`).

This leaves three groups of people out:

- **People with skills or goods but little money.** A teacher and a farmer who could serve each other cannot trade if neither has cash now. Roym has no way to pay with a promise of future goods or work.
- **People who want something but cannot find it by search.** A consumer can search offerings. They cannot post a need and let providers come to them.
- **People outside one group.** Search goes to the directories a consumer added by hand. Directories do not know about each other. There is no reputation beyond membership credentials and the receipts each person holds.

The requirements already promise parts of this: mutual credit (`[APP-ESC]`), a matching fabric (`[P2P-DSC]`) and peer reputation (`[P2P-REP]`). They are not built, and their current text does not fit together.

This change merges them into one design: the **Roym Value Exchange**.

## What the reader can do after this change

- Anyone is a **participant**. There is no separate consumer or provider role. In every deal, each side gives something of value to the other.
- Everything that changes hands is a **resource**: a service, goods, currency, or an EEP. A deal is resources for resources. Paying in currency and bartering are the same kind of deal.
- A participant starts an exchange in one of **two ways**:
  - **Want-Bid:** publish a **want** (what they need). Others answer with **bids**. The participant compares bids, asks for changes, and accepts or rejects.
  - **List-Search:** publish an **offering** (what they can give). Others find it by search and send an **enquiry**. The two sides change the terms until one accepts or rejects.
- Both ways end in the same **agreement**. One agreement can hold several resources on each side.
- A participant can issue an **earnest effort promise (EEP)**: a signed commitment, with a face value, to make an earnest effort to accept it against the issuer's own offerings. EEPs can change hands, one at a time or as a bundle. Roym tracks them as first-class records.
- Before a deal, a participant can judge the other party from **evidence** (signed facts) and **opinions** (signed views). Both parties to a deal can write an opinion of the other.
- An exchange can forward a search to its **friend exchanges** and return its own and its friends' results together.
- A participant can export a **statement** of every resource they gave and received, with a declared value. This is the raw data for tax. Roym computes no tax.

## Terms

- **Participant:** a person or business with a Roym identity (a master DID). Replaces "consumer" and "provider" as roles. In one deal, "giver" and "receiver" are said about one resource, not about a person.
- **SynOrg:** an independent group (guild, cooperative, community) that runs services for its members. See [TERMINOLOGY.md](../../../TERMINOLOGY.md).
- **Membership authority:** a SynOrg role. It issues membership credentials, revocations and moderation decisions about members. This exists today as the trust half of the Directory.
- **Exchange:** a SynOrg role. It holds published offerings and wants, answers searches (its own and forwarded ones), stores published evidence and opinions, computes aggregates and lists friend exchanges. It replaces the search half of the Directory. It holds no balances and no money.
- **Resource:** anything of value that one party gives to another in a deal. There are four **resource kinds**: a service in a time slot, goods or a service without a slot, currency, and an EEP. (From Valueflows, an open vocabulary for economic networks, which calls it an "economic resource". In this document "resource" never means compute or storage.)
- **Unit of account:** the currency used to state values: prices, declared values and EEP face values. A currency can also be a resource that changes hands. The two roles are separate: an EEP of "₹300" uses rupees as its unit, but nobody owes rupees.
- **Offering:** an intent to give a resource, on stated terms (today's `listing`).
- **Want:** an intent to receive a resource, published so others can find it and bid.
- **Intent:** the common word for an offering or a want. (From Valueflows.)
- **Enquiry:** a request that answers an offering (today's `request`).
- **Bid:** a quote that answers a want.
- **Quote author:** the participant who writes the quote. In Want-Bid it is the bidder. In List-Search it is the owner of the offering.
- **Barter:** a deal in which no currency changes hands. It is not a separate mechanism or record. It is an agreement whose resources are all goods, services or EEPs.
- **Agreement:** the signed terms both parties accepted. It is made of commitments.
- **Commitment:** one item inside an agreement: one party gives one resource to the other. It may carry signed **details** defined by a Roym extension (see [Roym extensions](../roym-extensions/change.md)), such as the chosen slot. Examples: "P delivers the service", "C pays ₹500", "C issues an EEP of ₹300". Each commitment has a **kind**, set by the resource kind, and is settled by its own record.
- **Economic event:** the signed record that settles a commitment: a fulfilment receipt, a payment acknowledgement, an issued EEP.
- **Declared value:** the value of a commitment's resource in the unit of account, in minor units. For a resource that is not currency, it is what the giver would charge for it in currency.
- **Event log:** an append-only log on each participant's node of every agreement and EEP event it took part in. The statement is read from it.
- **Earnest effort promise (EEP):** a signed commitment by its **issuer** to make an earnest effort to accept it against what the issuer offers when it is redeemed (see [What an EEP promises](#what-an-eep-promises)). It names its current **holder**, a face value and an expiry. The holder can redeem it, or pass it on. Each change of holder is a new version of the signed EEP record. "EEP" is used in the product as well as in this document.
- **Home exchange:** the exchange that holds the intent a deal came from. It pairs and publishes the two parties' opinions of that deal.
- **Reissue:** the issuer signs a new version of an EEP that names a new holder. A transfer is final only when the issuer reissues.
- **Bundle:** several EEPs, from one or many issuers, given in one agreement.
- **Evidence:** signed facts about how a party behaved: receipts, declines, refusals, and published evidence records.
- **Opinion:** one party's signed, subjective view of a deal and of the other party. It is labelled as an opinion and is never mixed with evidence.
- **Standing card:** a signed summary a participant chooses to show a counterparty: time active, number of distinct counterparties, outstanding EEPs (stated by the issuer), and similar facts.
- **Friend exchange:** another exchange that an exchange chooses to forward searches to and to point its users to.
- **Wire-reachable method:** a service method that another node can call. Every other method answers only local calls (`roym_core::admit::require_internal`), as today.

## Principles

These hold for every module.

1. **Facts are hard, trust is soft.** A signature, a version or a record either verifies or it does not. Whether to accept a party, a resource or an EEP is always the participant's own choice. Roym ships no default acceptance policy and no score.
2. **No shared ledger.** No blockchain, no global balance, no clearing house. Each record has one writer.
3. **The exchange finds. It never decides trust.** The participant's own node verifies every signature, credential and record, also for results that came through a friend exchange. This is today's rule for directories, unchanged.
4. **Every deal is between two parties.** An EEP transfer also needs the issuer's reissue, which is a separate step by the issuer. Effects across many parties, such as netting, come only from bilateral steps.
5. **Both sides give.** Roym treats both sides of a deal the same way. Currency, goods, services and EEPs are all resources, recorded as commitments of the same shape.
6. **Evidence and opinions, not scores.** Exchanges may compute aggregates. They are labelled as one party's calculation, never as truth. Opinions are shown as their authors wrote them. Roym computes no number from them.
7. **No evidence is not good evidence.** A new identity with no history shows as "no evidence", never as neutral or positive.
8. **Published evidence carries its publisher's standing.** Every published evidence record and opinion is labelled with whether its publisher holds a credential from an authority the reader trusts. One without such a credential shows as "no trusted credential", never as equal.
9. **A party may tell others how a deal went, not what it cost.** Either party may disclose the outcome of each commitment without the other's consent. Amounts and payment details stay hidden unless the other party grants their disclosure. See [Disclosure of deal evidence](#disclosure-of-deal-evidence).
10. **EEPs and barter are enforced by reputation, not by Roym.** An EEP is a commitment to make an earnest effort, not a debt. Roym records how each one was honoured and makes the pattern visible. It makes no claim about legal enforceability.
11. **Credit grows only as far as people believe each issuer can deliver.** No interest, no issuance granted by the system, no pooled risk. Any participant with an offering can issue. Holders decide whose EEPs to accept.
12. **Language-neutral structure, language-tagged text.** Participants speak many languages. Structured fields (categories, units, areas, commitment kinds) are codes with a label per language, so people can find and trade with each other across languages with no translation. Every free-text field (a description, a want, a quote's scope, an opinion) carries a language tag. A machine translation is shown on the reader's own node, labelled, and never signed. The signed original always wins. The rules are in the multilingual foundation change document.

## Scope and non-goals

- In scope:
  - The participant role, resources, offerings and wants.
  - Both ways to start an exchange: Want-Bid and List-Search, with one shared negotiation and agreement flow.
  - Agreements made of commitments with any resource on either side, with declared values, in one currency per agreement.
  - Selective disclosure of quote and agreement fields.
  - EEPs between two parties: issue, redemption, decline, split, set-off, expiry.
  - EEP transfer and reissue, and bundles.
  - Revocations that take effect from a stated time, so honest old records stay valid.
  - Splitting the Directory into a membership authority and an exchange.
  - Friend exchanges: one-hop forwarding of searches, and a simple merge of own and friends' results.
  - Evidence from three sources, and opinions from both parties to a deal.
  - The event log and the statement export.
  - Exchange aggregates and suggestions computed on the participant's node.
  - A signed `stored` receipt from the mailbox.
- Not in scope:
  - Moving money, escrow, or verifying a currency payment. Currency and goods move outside Roym. Roym records what both sides say.
  - More than one currency in one agreement.
  - Policy hooks and policy packages (for example a tax package). The event log is built so hooks can read it later. Roym computes no tax.
  - A native token, a global credit balance, or a mutual-credit ledger. This replaces the "native ledger token" in `[APP-ESC]`.
  - Forwarding a search more than one hop (a friend of a friend). Hop limits and loop rules are later work.
  - Index sharding and deterministic placement across many exchange nodes (`[P2P-DSC]` leaf shards). Moved to Milestone 7.
  - Multi-party atomic swaps, and atomic bundles (see [Bundles](#bundles)).
  - A shared EEP pool that holds EEPs from many people and swaps them automatically (as in Sarafu commitment pools). See [Rejected options](#rejected-options).
  - Features that present EEPs as investments: yield, expected return, price history. See [EEPs and investment](#eeps-and-investment).
  - Pseudonymous reviewers and context keys. A person can make any number of keys, so a pseudonym would break the distinct-reviewer count. This waits until an authority can attest "one key per member per context".
  - Witness committees, notaries, trusted execution environments and zero-knowledge proofs.

## Acceptance criteria

These are overview criteria. Each module change document refines them into tests.

| ID | Criterion | Test |
| --- | --- | --- |
| **Participants and resources** | | |
| VX-1 | One identity can publish an offering, publish a want, and give and receive resources in deals, with no role switch. | |
| VX-2 | Any resource kind can be on either side of an agreement: a service for currency, a service for a service, goods for an EEP, an EEP for an EEP. | |
| **Want-Bid** | | |
| VX-3 | A want is published to an exchange and found by search. Another participant answers it with a bid. The wanter sees all bids side by side, can ask a bidder for a revised bid, and can accept or reject each one. | |
| VX-4 | A public want shows only an approximate area. The exact address is shared only inside a conversation, as today. | |
| **List-Search** | | |
| VX-5 | An offering found by search gets an enquiry. The owner answers with a quote. Either side can ask for changes, and each change is a new version, until one side accepts or rejects. | |
| VX-6 | Want-Bid and List-Search end in the same agreement record and the same commitment states. | |
| **Agreements** | | |
| VX-7 | One agreement can hold several commitments, for example a service, ₹300 in currency and ₹200 as a new EEP. Each commitment carries its kind, its two parties and its declared value in the agreement's one currency. | |
| VX-8 | Each commitment moves only through the states of its kind, and every kind has a failure end (see the table under Design). An agreement closes when every commitment has reached a final state. | |
| VX-9 | The quote author is the single writer of the agreement. A second, conflicting change gets a named conflict, never a merge. | |
| VX-10 | A booking reserves its slot under the slot owner's single-writer fence, through the scheduling extension's reserve hook, also when the slot owner is not the quote author. If the reserve fails, the agreement is not formed. | |
| VX-11 | A party can show a quote or an agreement with chosen fields hidden (for example the price), and the signatures still verify. Neither record carries the hidden terms in plain form. | |
| VX-12 | Reputation shows which party failed which commitment, not only that a deal failed. | |
| **EEPs between two parties** | | |
| VX-13 | A participant with no published offering cannot issue an EEP. | |
| VX-14 | An EEP's face value is in a currency that appears in the issuer's own offerings, in integer minor units. | |
| VX-15 | A holder who never talked to the issuer can send it a redemption demand through the obligation service, also when the issuer is offline and the demand waits in its mailbox. | |
| VX-16 | A holder redeems an EEP at no more than the issuer's signed list price at that moment. A decline is recorded with the issuer's optional reason. | |
| VX-17 | An EEP returned for redemption cannot be spent elsewhere. If the delivery then ends unconfirmed, the issuer reissues the EEP to the holder. | |
| VX-18 | The issuer signs every split. The parts' face values add up exactly to the original. | |
| VX-19 | A participant can set a private limit per issuer. A quote that would go over it shows a warning. It does not block. | |
| VX-20 | Two parties who hold each other's EEPs in the same currency can cancel them against each other in one bilateral step. | |
| VX-21 | An EEP still verifies after the delegated key that signed it has expired. | |
| VX-22 | A record signed before a revocation's effective time stays valid. After a key compromise, it stays valid only when another party countersigned or acknowledged it before that time. | |
| VX-23 | After a known restore, a holder's version that the issuer signed after the backup time brings the issuer up to date. A version dated before the backup time is not accepted this way. | |
| VX-24 | If an issuer reissues from stale state, the two issuer-signed EEPs with the same id and version are proof against the issuer, and both holders keep a valid claim. | |
| VX-50 | Every EEP shows the standard earnest-effort sentence of its record version. No EEP carries a text of its own that changes the promise. | |
| **EEP transfer** | | |
| VX-25 | A holder passes an EEP to a new holder. The issuer reissues it to the new holder, also when the issuer was offline and the request waited in its mailbox. | |
| VX-26 | If one EEP version is passed to two people, the issuer reissues only the first valid request it processes. The other receives a signed refusal plus the two signed endorsements as proof against the double spender. | |
| VX-27 | The issuer refuses a reissue only for a double spend, an expired EEP, or a version already claimed by a redemption, and every refusal is signed. | |
| VX-28 | A repeated reissue request returns the same reissued EEP. It never creates a second version. | |
| VX-29 | One agreement can transfer a bundle of EEPs from several issuers. Each EEP settles or is refused on its own, and a refused EEP does not undo the others. | |
| **Membership and exchange** | | |
| VX-30 | A SynOrg can run a membership authority without an exchange, and an exchange without a membership authority. | |
| VX-31 | An exchange can accept credentials from another SynOrg's authority. A revocation there removes the member's offerings from search within the stated cache time. The client's own check shows the revocation at its next look. | |
| VX-32 | A credential from an authority the client has not added shows as "unknown authority", never as positive. | |
| **Federation** | | |
| VX-33 | An exchange forwards a search to its friend exchanges and returns its own and its friends' results in one answer. Each result names its source exchange and keeps its original signature, and the client verifies it. | |
| VX-34 | A friend that receives a forwarded search answers from its own index only. It never forwards it on. | |
| VX-35 | A friend that does not answer in time is left out, and the answer names it. | |
| VX-36 | The client can also search a friend exchange directly. | |
| VX-51 | A read of evidence and opinions about a party is forwarded one hop to friend exchanges, and each answer keeps its source exchange and original signatures. | |
| **Reputation, evidence and opinions** | | |
| VX-37 | A counterparty can publish evidence or an opinion about a participant to an exchange only with an agreement that names both of them. | |
| VX-38 | Published evidence shows the outcome of each commitment. It shows amounts and payment details only with the subject's grant. | |
| VX-39 | Every published evidence record and opinion shows whether its publisher holds a credential from an authority the reader trusts. | |
| VX-40 | The subject can attach a signed response. A withdrawal by the publisher stays visible as a withdrawal. | |
| VX-41 | The reader sees evidence from the three sources side by side, with each source labelled, and opinions apart from evidence. They are never merged into one number. | |
| VX-42 | Both parties to an agreement can sign an opinion of the other. A held opinion is shown to a third party only when its author agrees. The subject cannot block it. | |
| VX-43 | An opinion sent to the home exchange stays hidden from everyone until the other party has also written one, or the writing window ends. Then each party sees the other's opinion, and the public does not. | |
| VX-44 | During the resolution window an author can withdraw or revise its opinion, and a revision cannot lower the rating. When the window ends, the exchange publishes the current versions, and a revised one carries its label. | |
| VX-52 | An agreement records its home exchange: the source exchange of the intent, not a relaying one. Both opinions of the deal are paired there, also when one party sends its opinion through a friend exchange. | |
| VX-53 | A deal with no home exchange publishes no opinions. Each author can still show its own opinion on request. | |
| **Event log, statement, analytics** | | |
| VX-45 | Every agreement and EEP event a participant takes part in is appended to its event log. Nothing in the log can block a deal. | |
| VX-46 | A participant exports a statement for a period from the event log: every commitment given and received, its resource kind and its declared value. | |
| VX-47 | An exchange shares an aggregate only when it covers at least a minimum number of participants. | |
| VX-48 | Suggestions for one participant are computed on that participant's node, never on the exchange. | |
| **Mailbox** | | |
| VX-49 | The mailbox signs its `stored` receipt, so a holder can prove that a message was deposited for the issuer at a given time. | |

## Design

### Architecture

Eight modules. They are separate in data and API, and they work closely together. "Where it runs" says which node holds the data.

| Module | Where it runs | Owns | Built as |
| --- | --- | --- | --- |
| **Intents** | every participant | offerings and wants, as one intent record of kind offer or want; prices; what each intent accepts or gives in return | `catalog`, extended |
| **Agreements** | every participant keeps a copy; the quote author is the writer | enquiries, bids, quotes, agreements, commitments and their states; the participant's own opinions | `transaction`, extended |
| **Event log and statement** | every participant | the append-only event log; the statement export | inside `transaction` (the module document confirms) |
| **Obligations** | every participant | EEPs it issued (as the single writer), EEPs it holds, endorsements, refusals, demands, declines | new `obligation` service |
| **Search client** | every participant | the participant's lists of exchanges and trusted authorities; fan-out, merge and verification of results | the federated query library from [Roym open foundation](../roym-open-foundation/change.md) owns fan-out, merging and duplicate removal; the client half of `directory` keeps only the lists and calls the library |
| **Reputation** | every participant | the reader's view: gathers evidence and opinions and shows them | client library, new |
| **Exchange** | a SynOrg that enables it | published intents, search index, forwarded search, published evidence and opinions, aggregates, friend list | new `exchange` service, from the search half of `directory` |
| **Membership** | a SynOrg that enables it | members, credentials, revocations, member-level moderation | new `membership` service, from the trust half of `directory` |

Analytics is not a module of its own. The exchange computes aggregates over what it holds. The participant's node computes suggestions from those aggregates and its own records. There is no policy service in v1.

```text
            ┌──────────── SynOrg ────────────┐  search  ┌──── friend SynOrg ────┐
            │  Membership        Exchange    │ ───────▶ │  Exchange             │
            │  credentials ──▶   intents     │  (1 hop) │  intents, evidence    │
            │  revocations       evidence    │ ◀─────── │  (never forwards on)  │
            │                    aggregates  │  results └───────────────────────┘
            └───────▲───────────────▲────────┘
       publish /    │               │ search (own + friends' results),
       standing     │               │ evidence, opinions, aggregates
            ┌───────┴───────────────┴────────┐
            │      Participant A node         │   Conversation: enquiry or
            │  Intents  Agreements  Event log │◀─ bid, quote, agreement ──▶ Participant B node
            │  Obligations                    │
            │  Search client  Reputation      │◀─ obligation methods ─────▶ Issuer node
            └─────────────────────────────────┘   (mailbox when offline)    (may never have met A)
```

All modules stay inside the Roym SynApp. The requirements rule out separate ledger or marketplace SynApps.

### Channels between nodes

Each kind of traffic has a named channel:

| Traffic | Channel | EEPs |
| --- | --- | --- |
| Enquiry, bid, quote, agreement, receipts between the two parties of a deal | Conversation cards, as today | First contact with an offline party still needs one live `prekey-bundle` call. The mailbox change leaves that out of scope, so first contact needs both parties online. |
| Reissue request, redemption demand, decline, reissued EEP, between a holder and an issuer who may never have talked | wire-reachable methods of the `obligation` service | Verified callers only: the caller's DID comes from the transport. Every such method is mailbox-eligible: it carries an idempotency key and returns no data. The answer comes back as a call to the other side's `obligation` service. |
| Publish, search, evidence, opinions, aggregates | wire-reachable methods of the `exchange` service | Like the four `directory` methods today (`crates/roym_directory/src/app.rs:73`). |
| Forwarded search, forwarded reads of evidence and opinions, forwarded opinions to a home exchange | `exchange` to `exchange` | One hop. See [Federation](#federation). |
| Member standing | wire-reachable method of the `membership` service | Moves from `directory.standing`. |
| Questions to contacts | Conversation | No new wire method. One level in v1. |

Rules:

- **Each new or split service lists its wire-reachable methods in its module document.** Every other method stays local-only, as today.
- **The `obligation` service is deployed `public`.** Only a public service can have a mailbox (mailbox change, `D-C3-4`), and holders and issuers are often offline. Its record shows that the participant takes part in EEPs. It shows no EEPs.
- `obligation` and `exchange` hold `CallTarget::Service`, because each talks to a stranger's node. Today `directory` is the only Roym service that does.

### Resources and intents

- Every participant runs the same services. "Provider" and "consumer" now mean only "the side that gives this resource" and "the side that receives it".
- A **resource** has a kind: a service in a time slot, goods or a service without a slot, currency, or an EEP. The kind decides how a commitment to give it is settled (see [Agreements made of commitments](#agreements-made-of-commitments)).
- **Currency has two roles.** As a resource, it is money that one side pays. As the unit of account, it is how every price, declared value and EEP face value is stated. A barter deal has no currency resource, but it still has a unit of account, so each side's resources can be compared and reported.
- **One currency per agreement in v1.** Every declared value in an agreement uses the same unit of account.
- An **offering** is today's `listing`: what the participant gives, its price in the unit of account, and what it **accepts in return**. Its `PaymentTerms` (`crates/roym_core/src/listing.rs:77`) becomes this list of accepted resource kinds: currency methods, goods or services (barter), EEPs. "Accepts EEPs" means EEPs at face value. Which issuer and how much are negotiated in the quote and fixed in the agreement. Acceptance rules are never published in detail.
- A **want** is the mirror of an offering: what the participant needs (categories, description), an approximate area, a time window, a close time, and what it can **give in return** (currency, its own goods or services, EEPs). Offerings and wants are one intent record in `catalog`, with a kind field.
- An EEP is a resource like any other. Once transfer is enabled, a holder can list an EEP in an offering, and a participant can publish a want for EEPs of one issuer.

### Two ways to start an exchange

An exchange starts when one participant publishes an intent and another responds to it. There are two ways. They differ only in **discovery**: who publishes, who searches and who writes the first quote. From the first quote on, both use one negotiation flow and one agreement record. There are not two protocols.

| | Want-Bid | List-Search |
| --- | --- | --- |
| Published intent | a want, by the participant who will receive | an offering, by the participant who will give |
| Who searches | givers search wants | receivers search offerings |
| First response | a **bid** (a quote) from a giver | an **enquiry** (a request) from a receiver |
| Quote author (single writer) | the bidder | the offering's owner |
| Replies per intent | many bids, compared side by side | one enquiry per interested party |
| Fits best | one-off or custom needs, where the wanter wants to compare | repeatable goods and services with a set price |
| Ends when | the wanter accepts a bid and closes the want, or the close time passes | never; the offering stays until its owner removes it |

**Shared negotiation.** In both ways:

1. The quote author sends a quote. A quote lists the commitments of both sides, with declared values.
2. The other side accepts it, rejects it, or asks for changes. A change request is a new version of the enquiry, or a message in the conversation, as today.
3. The quote author answers a change request with a new quote version. Each version supersedes the previous one. Only the quote author writes quotes, so there is one writer and no merge.
4. Acceptance produces the signed agreement. A quote that is not accepted before its expiry lapses.

A wanter can accept more than one bid for the same want, for example two farmers who each deliver half. Each accepted bid is its own agreement.

**Why one flow.** Two separate protocols would double the states, the records and the tests, and reputation could not compare deals made in different ways. Want-Bid adds only a new kind of published intent (the want) and a new name for a quote that answers it (the bid). Everything after the first quote is shared.

### Agreements made of commitments

An agreement is a set of commitments. Each commitment names who gives, who receives, the resource kind, the commitment kind and the declared value.

| Resource | Commitment kind | Settled by | States |
| --- | --- | --- | --- |
| Goods or a service, with or without a slot | fulfilment | `fulfilment-receipt` | promised → fulfilled; or ended unconfirmed |
| Currency | currency | `payment-acknowledgement` | requested → acknowledged; or ended unconfirmed |
| New EEP | eep-issue | the issued EEP, version 1 | promised → issued; or ended unconfirmed |
| EEP the giver holds | eep-transfer | endorsement, then the issuer's reissue | promised → endorsed → reissued; or refused |
| EEP returned to its issuer (redemption or set-off) | eep-return | endorsement back to the issuer, then the issuer's closing record | promised → returned → closed; or ended unconfirmed |

The agreement itself has a short life: requested → quoted → agreed → closed, or expired, or cancelled. It closes when every commitment is final.

**Work steps belong to extensions, not to commitment states** (owner decision, 2026-10-26). A booked slot, "started", "documents received" or "driver arrived" is a step record of a Roym extension (see [Roym extensions](../roym-extensions/change.md)). The first two extensions are **scheduling** (a service in a time slot, today's booking) and **one-shot** (pay and deliver, today's order). Both use the same `fulfilment` kind. Reputation reads only whether a commitment settled. An opinion can still point at one step.

Rules:

- **The list of kinds is fixed in code.** An offering names the extension that runs its work steps (for example scheduling or one-shot), not a kind of its own. Strangers can then compare "delivered" across all participants, and reputation and the statement can read every state. A configurable workflow engine is rejected (see [Rejected options](#rejected-options)). A new kind is added in code when a real need appears.
- **One writer per record.** The quote author's node writes the agreement and the commitment states. Both parties keep a copy. This generalises today's rule that the provider's Transaction service is the single writer. The issuer writes each EEP. One deal can therefore involve two writers, each for its own record.
- **The resource owner reserves its own resource.** Today a slot is claimed under the slot owner's single-writer fence when it files the acceptance (integrated spec, `D-C8-2`). This becomes the generic reserve hook of [Roym extensions](../roym-extensions/change.md) (EX-5, EX-6). In List-Search the quote author owns the slot, as today. In a bid, or a barter with a booking on each side, the quote author may not own every slot. Then each owner reserves its own resource as a separate step when the agreement is accepted, and the agreement writer records the result. If a reserve fails, the agreement is not formed, and the other reserves are released.
- **How an agreement and an EEP point to each other.** For `eep-issue`, the agreement names the issuer, the currency and the face value. The EEP does not exist yet, so the agreement cannot name it. The EEP's version 1 names the agreement's content hash, and the settlement record names the EEP id. For `eep-transfer` and `eep-return`, the EEP already exists, so the agreement names its id and version. The agreement writer marks an EEP commitment settled when it holds the issuer's signed EEP that names the right holder.
- **The agreement records its home exchange.** It is the exchange that holds the intent the deal came from: the source exchange named in the search result, not an exchange that relayed it. A deal that began from a direct link has no home unless the two parties name one in the agreement. See [Publishing an opinion](#reputation-evidence-and-opinions).
- **Cancellation:** only the quote author, and only while no commitment has moved. This generalises today's rule.
- **Every commitment carries a declared value** in the agreement's unit of account, in minor units. The statement export uses it.
- **Selective disclosure covers the quote and the agreement**, including each commitment's details block. Today `AgreementReceiptPayload` (`crates/roym_core/src/transaction.rs:192`) carries a plain copy of `terms`, and `quote_record_id`, a hash of the whole quote. Salting only the receipt would hide nothing, because showing the quote shows everything. So: each field of the quote is hashed with its own random salt, and the quote's signature covers the list of hashes. The receipt carries the same list of hashes and no plain terms. To show a deal to a third party, a party reveals the fields it chooses, with their salts. The others stay hidden, and both signatures still verify. This changes the canonical bytes of the quote and of `agreement-receipt`, so it is done in the first phase, before any EEP or evidence record depends on them. The product is not released, so the records change in place, with no migration.

### Disclosure of deal evidence

Today's rule (integrated spec, "Payment evidence is not a public listing field"): a `payment-acknowledgement` belongs to the two parties, and it is disclosed to a third party only by an explicit, scoped, expiring grant.

This change keeps that rule for amounts and changes it for outcomes:

- **Outcomes need no consent.** Either party may disclose, to a contact or to an exchange, the outcome of each commitment of a deal it was part of: done, delivered, acknowledged, ended unconfirmed, refused. Reputation is not possible without this. A rule that needs the other party's consent would let every party hide every bad outcome.
- **Amounts and payment details keep today's rule.** Price, declared values and payment method are hidden fields by default (selective disclosure). They are shown to a third party only with the other party's explicit, scoped, expiring grant.
- **Search results carry no evidence.** A reader fetches evidence about a party only when it asks for it, as today's rule requires.

This reverses part of a rule of record, so it gets its own ADR before the reputation phase. The integrated spec rule is updated at close-out.

### Earnest effort promises (EEPs)

#### What an EEP promises

Every EEP carries the same promise, in these words:

> *"I will make an earnest effort to accept this promise against what I offer when it is redeemed, at my listed prices, as far as my circumstances then allow."*

- **An earnest effort** means a sincere attempt within the issuer's real limits. It promises the effort, not the acceptance. An honest decline is an earnest effort, for example: "my wife fell ill, I sold most of my stock, and I need what is left for food". A false reason is not, for example "no stock" while selling the same stock to buyers who pay cash, and neither is charging an EEP holder more than the listed price.
- **What I offer when it is redeemed** means the issuer's offerings at that time, not at issue time. The EEP names no goods.
- **At my listed prices** gives the face value its meaning: "₹300" buys what ₹300 buys at the issuer's list prices.
- **As far as my circumstances then allow** says that capacity, health, stock and season can change.

The sentence belongs to the version of the EEP record type. EEP version 1 means this sentence. An EEP needs no extra field, and an issuer cannot add a weaker or stronger text of its own. If the wording ever changes, it becomes a new record version, and old EEPs keep their meaning. The Hub shows the sentence next to every EEP.

What follows from it:

- **A decline is normal, not a breach.** Readers see each decline with its optional reason, and judge the **pattern** of an issuer's redemptions, not one decline.
- **Giving a reason stays optional.** An earnest effort asks only that a reason given is true.
- **There is no personal message from the issuer.** Thanks between the first two people belong in an **opinion** of their deal. After a transfer, a personal message would mean nothing to a stranger.
- **The duty to reissue stays hard** (see [Transfer and reissue](#transfer-and-reissue)). Reissue only records who holds the EEP. It asks the issuer to deliver nothing.

#### The EEP record

An EEP is a signed record. Each version is a new record that supersedes the previous one.

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

- **Unit and thumb rule.** The currency is used **only as the unit of account**. The issuer does not owe money. "₹300 from X" means "what ₹300 buys at X's list prices when I redeem". The issuer sets the face value at what the same goods cost in that currency at its list price today.
- **The issuer must have an offering.** A participant with no published offering cannot issue. A promise must point to something the issuer offers.
- **Capabilities are indicators only.** "Wheat" or "carpentry" is what the issuer's offerings say, proven over time by redemptions. They are not part of the promise.
- **Every EEP can be transferred,** once transfer is enabled. There is no non-transferable flag.
- **No interest.** An EEP promises its face value. Any time value appears only as a discount that two parties choose when they trade an EEP.
- **The holder acknowledges each version.** The holder's node signs a short acknowledgement of each EEP version it receives and keeps it with the EEP. This lets the EEP survive a later compromise of the issuer's key (see [Signatures that outlive keys](#signatures-that-outlive-keys)).

#### Issue

The issuer gives a new EEP as a resource. It signs version 1, naming the receiver as holder, and sends it as the settlement of a `eep-issue` commitment.

#### Redemption, decline and set-off

- **List price is a maximum.** The holder redeems at no more than the issuer's signed list price that is current when the holder asks. The issuer may give a better deal. A refusal to go below the list price is normal and is not evidence against the issuer.
- **The issuer chooses which offerings to accept EEPs for, at that time.** For example, ten offerings may accept EEPs today and five next week. Fewer offerings is a **decline**, recorded with the issuer's optional reason. The reader decides whether it was good, bad or acceptable.
- **Who carries the risk.** A redemption is one agreement with a `eep-return` commitment from the holder and a `fulfilment` commitment from the issuer. When the agreement is signed, the holder signs the return endorsement, and the issuer records that version as claimed by this redemption. From then on the EEP cannot be spent elsewhere, so the issuer carries no double-spend risk. The issuer closes the EEP only when its own delivery is done. If the delivery ends unconfirmed, the issuer must reissue the EEP to the holder. So the holder carries only the risk that the issuer fails to deliver, and that failure leaves signed evidence: the return endorsement, the agreement, and the unconfirmed delivery.
- **No answer.** A demand the issuer never collected from its mailbox, with a delivery window that has ended, is weaker negative evidence than a signed decline. The mailbox's signed `stored` receipt proves that a message was deposited for the issuer at a given time. Proving that this message was the demand needs extra work, because the mailbox signs a hash of ciphertext it cannot read. The obligation change document designs it. The `delivered` receipt, signed by the receiver, proves the issuer's node received the message.
- **Set-off.** A holder who owes the issuer can return the issuer's EEP in payment. This is a `eep-return` commitment, the same as a redemption.
- **Split.** The issuer signs every split. A holder asks the issuer to split an EEP. The parts are new EEPs whose face values add up exactly to the original, in integer minor units, and the original version is closed. Only the issuer writes EEPs, so a split by the holder alone is not valid.
- **Expiry.** After expiry the issuer's duty ends, and it may delete its records of that EEP. A demand made inside the window and not met stays as evidence with the holder.

#### Transfer and reissue

Transfer is built in M08H, after discovery (M08F) and reputation (M08G). Before then, strangers cannot find each other, and a receiver has no published evidence to judge a stranger's EEP, so transfer would have little use. The order does not reopen the legal decision.

1. The holder signs an **endorsement**: "EEP N, version v, now held by H2". It is the settlement step of a `eep-transfer` commitment.
2. H2's node sends a **reissue request** to the issuer's `obligation` service: directly, or through the issuer's mailbox when the issuer is offline.
3. The issuer checks the chain from the last version it signed, and signs version v+1 naming H2. It sends the EEP back to H2's `obligation` service, through H2's mailbox if needed.

Rules for the issuer:

- **Duty to reissue.** The issuer must reissue the first valid request it processes for each version. It may refuse only when the version was already reissued (a double spend), the version is claimed by a redemption, or the EEP has expired. Every refusal is a signed record. A refusal for any other reason counts against the issuer as evidence.
- **First processed wins.** The mailbox gives no order between different senders, so "first" means the issuer's own processing order. That is enough, because the issuer is the only record of its EEPs. The losing holder receives the signed refusal plus the double spender's two endorsements.
- **Idempotent.** The key is `(EEP id, version, endorsement hash)`. A repeated request returns the same reissued EEP.
- **Claim one EEP version, never all of the issuer's records.**
- **Chain length.** Until the issuer reissues, each new holder adds an endorsement, so an EEP can pass through several hands while the issuer is offline. Each holder sees the earlier holders in the chain. A reissue resets the chain.

Rules for the receiver:

- **Accept now or wait.** The receiver chooses. It can deliver its own side at once and carry the risk until the reissue arrives. Or it can deliver after the reissue. Roym fulfilment usually comes after payment, so waiting fits most deals. The booking card shows "payment pending issuer confirmation".
- **Per-issuer limit.** A participant can set a private limit for each issuer, for example "at most ₹2,000 of X's EEPs". A quote that would go over it shows a warning. It never blocks. (This also applies to newly issued EEPs, before transfer exists.)

#### Restore from an old backup

An issuer that restored an old backup may not know about versions it signed later. Two rules:

- **Catch up only after a known restore.** The issuer's node records each restore with the backup's time. After a known restore, when a holder shows a version that is newer than the issuer's record, the issuer accepts it only when its own signature verifies, the version was signed after the backup time, and no revocation's effective time is earlier than the signing time. It then catches up. Without a known restore, an unknown newer version is not accepted. This stops a person with a leaked, expired delegated key from back-dating a forged "newer version": the verifier trusts the signer-written `issued_at_secs` (`crates/signed_record/src/verify.rs:204`), so the signing time alone proves nothing.
- **A stale reissue is visible, not prevented.** Until a holder shows the newer version, a restored issuer cannot know it is stale. So it may reissue a version it already reissued. The two issuer-signed EEPs with the same `(id, version)` and different holders are signed proof against the issuer. Both holders keep a valid claim on the issuer, because the issuer signed both.

#### Bundles

A participant can give several EEPs in one agreement, from one issuer or from many. Examples: a buyer collects EEPs from many holders and sells them on as one lot; a buyer buys a lot from one seller. Bundles need transfer, so they come with it.

- Each EEP in a bundle is its **own `eep-transfer` commitment**, with its own declared value, its own endorsement and its own reissue by its own issuer.
- A bundle is **not atomic**. Each issuer acts alone, so one EEP can be refused (for example a double spend) while the others are reissued. A refused EEP does not undo the others. Making a bundle atomic would need every issuer to act together, which principle 4 rules out.
- The agreement says what a refused EEP does to the other side's commitments. The simple choice is one payment commitment per EEP, which lapses when its EEP is refused. The obligation change document decides (see [Open questions](#open-questions)).
- The receiver can wait for every reissue before it delivers its own side, as for a single EEP.
- A seller who collected EEPs from many issuers carries the risk of each of them until the reissue. Readers judge the seller like any other party.

#### Netting

Netting comes from bilateral steps only. Example: A owes B, B owes C, C owes A.

1. B pays C with A's EEP, and C returns B's EEP. B is now even.
2. C now holds A's EEP, and A holds C's EEP. They set off against each other.

Step 1 needs transfer. Step 2 is a set-off between two parties and works without it. Each step is a fair trade on its own, so a chain that stops half-way leaves nobody worse off. No multi-party atomic swap is needed. Only a two-party swap is atomic: one swap record holds both endorsements and is valid only with both signatures. Netting works only between EEPs in the same currency. Across currencies, the two parties agree a rate for that step. The participant's node may suggest a step ("you hold X's EEP, and you owe X"). No node sees the whole graph.

#### Over-issuance

An issuer may knowingly issue more than it can redeem at once, as an airline overbooks. Roym does not prevent it. It makes it visible in two ways:

- **Redemption behaviour.** Each delivery, decline, refusal and unanswered demand is signed evidence from holders. This is the faithful signal.
- **A stated total.** The issuer's standing card can show the total face value of its outstanding EEPs. The issuer's node holds every EEP, so this costs nothing. The card labels the total "stated by the issuer, not checked".

A published Merkle commitment that holders check was considered and dropped (see [Rejected options](#rejected-options)).

#### EEPs and investment

An EEP is a claim on the issuer's future output. That is true of every EEP, so there is no separate "investment" record and no rule that can tell an investment from a normal EEP. A holder who buys an EEP at a discount and redeems it later at face value has, in effect, invested. Roym does not prevent this.

- **Transfer, bundles and listing EEPs for trade** are built. Legal and tax compliance is the participants' and operators' concern. Roym keeps faithful, signed records and the statement export (owner decision, 2026-10-26).
- **Features that present EEPs as investments** (an expected return or yield, a price history or chart for an EEP, ranking issuers by how much an EEP may gain) are not built. This is a product choice: they help nobody trade, and they invite speculation.

#### Signatures that outlive keys

EEPs can live for years, but the substrate signs with delegated keys that expire (spec D7). An EEP carries its delegation certificate. The verifier checks that the EEP was signed **inside the certificate's window**, not that the certificate is valid now. Today's verifier already does this: `crates/signed_record/src/verify.rs:204` compares the record's `issued_at_secs` with the certificate window. `verify_chain_at` (`crates/identity/src/delegation.rs:203`) uses the current time only to reject a certificate issued in the future.

**Revocations take effect from a stated time.** Today a revoked signing key fails every record it ever signed, including honest EEPs signed before the revocation (`crates/signed_record/src/verify.rs`, the `check_did` calls). This change adds an `effective_from` time and a reason to revocations:

- A record signed before `effective_from` stays valid.
- When the reason is a key compromise, the signer-written time cannot be trusted. A record signed with that key stays valid only when another party countersigned or acknowledged it, in a record of its own, before `effective_from`. For EEPs, that is the holder's acknowledgement of each version.
- This is a prerequisite. It is built before EEPs.

#### What the issuer sees

EEPs are not blind. The issuer sees every holder that asks it to redeem or reissue. Holders also see the earlier holders in a chain that has not yet been reissued. This is the price of accountability: every double spend produces signed proof against a named person. The UI says so.

### Membership and exchange

Today one node is both the membership authority and the directory. Search keeps only offerings whose member is in good standing with the directory's own issuer (`crates/roym_directory/src/app/search_ops.rs:210`). `directory.standing` serves a member's credential, revocations and suspensions (`crates/roym_directory/src/app.rs:78`). Splitting the roles gives these rules:

| Point | Rule |
| --- | --- |
| Joining rules | Belong to the SynOrg. The exchange has a **publishing policy** that may require membership of one or more authorities, for example "members of guild G may publish; anyone may search". The same policy covers offerings and wants. |
| Standing | The exchange gets a member's standing by calling the authority's open standing method remotely, and caches it for a stated time. The method moves from the directory to the membership service. |
| Removal time | One SynOrg running both roles removes a suspended member at once, as today. Across SynOrgs, removal takes up to the cache time. The client's own check still shows the revocation at its next look. |
| Moderation | The exchange signs decisions about an intent or an opinion on that exchange (spam, its own content rules). The authority signs decisions about a member. Both use `moderation-decision`, with a different signer and scope. |
| Scope | The publishing policy checks that the credential's scope covers the intent's category. |
| Client trust | The client keeps its own list of trusted authorities, separate from its list of exchanges. A credential from an authority not on the list shows as "unknown authority". This replaces pinning "which group a directory speaks for". |
| Open exchanges | Allowed. Their results show "no credential". |
| Dependencies | The exchange calls authorities and friend exchanges at runtime by DID, as clients call directories today. An authority depends on nothing. Forwarding is one hop only, so friends cannot form a loop. |

### Federation

- A participant can add many exchanges. The search client fans out to each one, as today.
- An exchange keeps a list of **friend exchanges**. It forwards each search, for offerings or for wants, to the friends it chooses, and answers with its own and its friends' results together.
- **One hop only.** A forwarded search carries a marker and the forwarding exchange's DID. A friend that receives it answers from its own index and never forwards it on. This needs no hop limit and no loop detection. Deeper forwarding is later work.
- **The friend sees the forwarding exchange, not the client.** This also hides who searched from the friend. The friend applies its own rate limit per forwarding exchange.
- **Timeout.** The exchange waits a stated time for each friend (a first value to tune). A friend that does not answer in time is left out, and the answer names it, so the client can search that friend directly.
- **Merge rule for v1.** Relevance scores from different exchanges cannot be compared, because each exchange ranks by its own rules. So the merge does not compare them:
  1. The exchange's own results first, in its own order.
  2. Then the friends' results, taken one from each friend in turn (round robin), each friend in its own order, up to a cap per friend.
  3. The same intent (same id and version) from several sources is shown once, with every source listed.
- **Every result keeps its source and its original signature.** The forwarding exchange can leave a friend's result out, but it cannot change one without breaking the publisher's signature. The search client verifies every record itself, and credentials from unknown authorities show as unknown, as for any result.
- **Reads of evidence and opinions are forwarded the same way.** A reader asks its own exchange about a party. That exchange answers from its own store and forwards the question one hop to its friends, with the same marker, timeout and merge rule. Every answer keeps its source exchange and its original signatures, and the client verifies them.
- The client can also search a friend exchange directly. An exchange may show its friend list for this, mainly when a search finds nothing.
- One exchange is one index. Sharding one exchange's index across many nodes moves to Milestone 7.

### Reputation, evidence and opinions

Roym keeps two kinds of input apart:

- **Evidence** is signed fact: a receipt, a decline, a refusal, an unanswered demand. It either verifies or it does not.
- **Opinion** is a party's signed, subjective view. It verifies as "this author said this", never as "this is true".

The reader's node gathers evidence from three sources and opinions from the parties to each deal. It shows them **side by side, labelled, never merged into one number**. What each source may show follows [Disclosure of deal evidence](#disclosure-of-deal-evidence).

**1. Records the participant shows.**
- Signed receipts, EEPs and declines that the participant holds, plus its standing card.
- The reader counts **distinct counterparties**, gives less weight to a counterparty that appears in many places in the reader's own view (inverse frequency), and counts mutual pairs (A rates B, B rates A) only once.
- How a reader can check that a participant shows its whole history, and not only the good part, is set out in [reputation-design-note.md](../../../reputation-design-note.md). That covers a signed count, receipt sequence numbers and random sampling. The reputation module document decides how much of it v1 needs.

**2. Evidence from the reader's contacts.**
- The reader's node asks its direct contacts over Conversation. v1 asks one level. Asking the contacts of contacts, up to three levels, is later work.
- Answers are **first-hand signed records only**: evidence from the contact's own deals, and opinions the contact wrote itself. A contact never passes on someone else's opinion as its own.
- Distinct sources count, not distinct paths, and each source has a cap.
- No answer is not a penalty.
- A contact chooses whether to answer and how much to show. Questions are rate-limited.

**3. Evidence published to an exchange.**
- **No agreement, no publish.** The publisher must be a party to an agreement that names the subject. The subject must be listed on that exchange. The publisher need not be a member. The exchange's publishing policy may require membership for evidence too.
- **Labelled by the publisher's standing.** Each record shows whether its publisher holds a credential from an authority the reader trusts (principle 8).
- The evidence record holds: the subject's signed agreement, with only the fields the publisher chooses to show and with amounts hidden unless the subject granted them; the outcome of each commitment; an optional opinion; and pointers to attachments.
- The exchange stores this **small signed record**, up to a size cap (to be measured). Large attachments stay with the publisher, behind a pointer.
- The subject can attach a **signed response**. This turns the evidence into a dispute.
- The publisher can **withdraw** only with a signed withdrawal, which stays visible with its label: settled otherwise, forgiven, or complaint withdrawn.
- The exchange never turns evidence into a verdict. Retention follows the exchange's stated policy.

**Opinions from both parties.** Each side of a deal has a view of the other, so both can record it.

- After an agreement, **each party may sign an opinion** of the deal and of the other party: a short text, and optionally a coarse rating (0, 1, 2 or none). The opinion names the agreement. The party that delivered can review the party that paid, in the same way. The rating lives only in the opinion. The fulfilment receipt stays a pure fact, as `[P2P-REP]` describes satisfaction as a separate signal that points to a receipt.
- **The author decides who sees it.** An opinion the author holds is shown to a third party only when that party asks and the author agrees.
- **Nobody can block an opinion about a real agreement.** If the subject had to agree, every bad opinion would be hidden, and the good ones would mean nothing. The subject is protected in other ways: the opinion must name a real agreement between the two, and the subject can attach a signed response, or show its own opinion of the same deal next to it.
- Opinions follow the same rules as other published evidence: one per party per agreement, a credential label, a signed response, a visible withdrawal.

**Publishing an opinion to an exchange.** Both opinions of one agreement go to its **home exchange**, because only one place can pair them and reveal them together.

- A party may send its opinion to **its own exchange**. If that exchange is a friend of the home exchange, it forwards the opinion there, one hop. Otherwise the party's node sends it to the home exchange directly.
- A deal with no home exchange has no published opinions. Each author keeps its opinion and shows it on request.

Three steps, in order:

1. **Write, sealed.** Each party sends its opinion inside a **writing window** after the agreement closes (14 days is a first value to tune). The home exchange shows it to nobody yet. "Sealed" means the exchange holds it back. It is not encrypted, so the exchange itself can read it, and the parties trust the home exchange for this.
2. **Reveal to each other.** When both parties have written, or the writing window ends, the exchange shows each party the other's opinion. The public still sees nothing. Neither party could read the other's opinion before writing its own, so neither can answer it with revenge.
3. **Resolve, then publish.** A **resolution window** follows (7 days is a first value to tune). The parties can talk, fix the problem, or agree a refund. In this window:
   - An author can withdraw its opinion, or replace it with a revised one. The published opinion is labelled "revised during resolution".
   - A revision can raise the coarse rating, never lower it. A party that read a bad opinion therefore cannot make its own worse in return. Text changes cannot be checked this way, so the label is the only signal for them.
   - The subject can write its signed response.

   When the window ends, the exchange publishes the current versions with their responses. After that, the normal rules apply: a response at any time, and a withdrawal stays visible with its label.

**Content rules (later).** An exchange may later enforce stated content rules on published opinions, for example no abusive language and no personal data such as a phone number. It hides an opinion that breaks a rule with a signed `moderation-decision` that names the rule. It may never hide an opinion because it is negative. A hidden opinion stays with its author, who can still show it on request. v1 has no content rules.

**What evidence says about EEPs.** Each event points at one named party:

| Event | Evidence | About |
| --- | --- | --- |
| Redeemed | the holder's receipt | issuer, positive |
| Declined, with or without a reason | the issuer's signed decline | issuer; the reader judges |
| Unanswered past the window | mailbox `stored` receipt and the demand | issuer, weaker negative |
| Returned, delivery ended unconfirmed, EEP not reissued | the return endorsement, the agreement and the unconfirmed delivery | issuer, negative |
| Refused reissue | the issuer's signed refusal | the double spender if the reason holds; otherwise the issuer |
| One version endorsed twice | the two endorsements | the endorser |
| Two issuer-signed EEPs with the same `(id, version)` and different holders | the two EEPs | issuer, negative |
| Released by the holder | a signed release with its label | issuer; the event stays, labelled, and fades over time |

**Known limit: fake identities.** A farm of one-time fake identities, each with one fake deal, can create good-looking evidence and opinions in source 1. It can also publish them to an exchange in source 3, because a publisher need not be a member and the farm's fake identities are each other's counterparties. The defences are:

- **Credential labels.** Evidence from publishers with no credential from an authority the reader trusts is labelled as such (principle 8). A farm must get its identities admitted by a real authority to look equal.
- **Contacts.** Source 2 is the strongest defence: a farm has no real links into the reader's network.
- **Distinct counterparties** and inverse frequency in the reader's own view.
- The UI shows when a party is supported only by sources 1 and 3 with no trusted credential.

### Event log and statement

- **The event log** is append-only, on the participant's node. It records: agreement signed; commitment settled or failed; EEP issued, redeemed, declined, split, set off, transferred, reissued, refused, expired. Each entry points at the signed record behind it.
- Nothing reads the log in a way that can block, change or delay a deal.
- **Statement export (built in):** for a chosen period, every commitment given and received, with its resource kind, counterparty, unit of account and declared value. Barter appears like any other deal: each side's resources with their declared values.
- **Policy hooks are later work.** A policy package (for example from an accountant or a public body) would read the event log as an observer, on the participant's node, after the participant installs it. A hook that can block a deal before signing is added only when a real policy needs it.
- **Tax is not an acceptance rule.** Acceptance happens before a deal and is decided by a person at quote time. Tax is computed after a commitment settles.

### Analytics

- An exchange computes aggregates only over what it holds: published intents, published evidence and opinions, and counts of searches (its own and forwarded ones). Examples: the median review rate per category, supply and demand gaps per category and area (offerings against wants), flagged clusters of fake reviewers, and redemption statistics per issuer from published evidence.
- An aggregate is shared, with users or with friend exchanges, only when it covers at least a minimum number of participants (10 is a first value to tune).
- **Suggestions for one participant are computed on that participant's node**, from the exchange's aggregates and the participant's own records. The exchange does not profile individuals.
- Aggregates are labelled as the exchange's calculation.

### Mailbox change

EEP redemption demands and reissue requests use the [service mailbox](../service-mailbox/change.md). One addition is needed. Today the mailbox answers `deposit` with `stored`, and the change document does not say that this receipt is signed (unverified against the code). This change asks the mailbox to **sign** its `stored` receipt: deposit id, destination, message hash and time. The mailbox is a third party, so its signature proves that a message was deposited for the destination at that time. It does not prove what the message said, because the message is sealed. M06D did not add it. It is built at the start of M08E, before EEPs.

### Lessons from existing systems

| System | What we take | What we avoid |
| --- | --- | --- |
| Community Exchange System (CES) | Each community runs itself; a trade between exchanges should feel like a local one | A central bridge and a central exchange-rate setter; unsettled imbalances between exchanges. Our exchanges hold no balances, so neither applies. |
| Credit Commons Protocol | A tiny, fixed set of states; rounding rules decided before code | A workflow that every node on a path must share; an admin who undoes completed deals |
| Circles | Each holder decides whom to accept, with a limit | Free value for every account, which made fake accounts pay off |
| XRP Ledger issued currencies | A holder limit per issuer; issuer powers that are visible | "Rippling", where balances moved through accounts not taking part in the payment. Only our issuer reissues, so this cannot happen. |
| Holochain mutual credit and Valueflows | The words resource, intent, commitment, economic event; both sides of a deal as the same shape | Countersigning, which does not stop double spends; importing the whole Valueflows model |
| Cashu and Fedimint | Reissue = invalidate the old EEP and issue the new one in one step; a final expiry ends the issuer's duty | Undefined half-finished swaps, and bugs in backup and restore. Hence the idempotency and restore rules above. |
| Multilateral trade credit set-off (MTCS), Cycles | Loops of debt can be cancelled with consent | One solver that sees everyone's debts |
| Sarafu commitment pools | EEPs from many local issuers can be swapped for each other | A shared pool that holds everyone's EEPs, with a steward who sets its rules. Bundles sold by one participant cover the same need without shared custody. |
| Two-sided reviews (Airbnb, eBay) | Both sides review each other; reviews revealed together | Retaliation: eBay stopped sellers giving buyers negative feedback in 2008 because of revenge reviews. Reveal-together rules address this without silencing one side. |
| SD-JWT (selective disclosure for signed tokens) | One salt per field and a signed list of hashes | Inventing our own disclosure format |

### Rejected options

- **Separate words for what is offered and how it is paid** ("offering" against "medium"). It made one side of every deal look like the seller and the other like the payer, and barter did not fit either word. Every side now gives resources. The words **asset** (suggests ownership and finance, and a service is not owned), **good** (excludes services in plain English) and **consideration** (a legal term, hard for many readers) were also considered for "resource".
- **Two separate protocols for Want-Bid and List-Search.** Double the states and records, and deals made in different ways could not be compared. One flow after the first quote.
- **Other names for the EEP.** Each one says more, or something else, than an earnest effort:
  - **IOU** stands for "I owe you" and reads as a debt.
  - **Voucher** reads as a prepaid entitlement that a shop must honour, like a gift voucher. In EU VAT rules a voucher is something there is an obligation to accept (from memory, not checked).
  - **Token** reads as a crypto token. In India, a cryptographic token with value that can be transferred may fall under the tax law's "virtual digital asset" category (from memory, not checked).
  - **Note** first means a currency note in India and in British English.
  - **Endeavour** is close in meaning, but in UK-style contract law "best endeavours" is a strong duty to take all reasonable steps.
  - **Good faith** is a legal term with a low bar: honesty, not effort.
  - **Coupon, chit, pledge, hundi, bond** each already name an entitlement or a regulated thing.
- **A strong promise to redeem** ("I will redeem this EEP"). It is worth more to a holder, but it looks like a debt and draws in more law and regulation. It would also make every honest hardship decline a breach. The earnest-effort sentence keeps the face value meaningful without that.
- **A personal text from the issuer on each EEP.** It would let issuers weaken or strengthen the standard promise, and readers could no longer compare EEPs. Thanks between the first two people belong in an opinion.
- **Opinions sent to any exchange.** The two opinions of one deal could land on different exchanges, and the sealed reveal could not pair them. They go to the home exchange, through a friend exchange if needed.
- **One global credit balance per identity (mutual credit).** It loses who owes whom. A balance can prove "gave at least X" but not "has X left", because nothing stops showing an old history.
- **Circles or a clearing service per group.** It needs groups and an operator. Acceptance is the peer's own choice.
- **A shared EEP pool** (as in Sarafu commitment pools). Many people deposit EEPs into one common reserve, and it swaps any EEP for another at set rates, with no person on the other side of each swap. It needs one shared record of the pool and someone who controls it: a shared ledger and a custodian, which principles 2 and 11 rule out. A participant who collects EEPs and sells them as a bundle gives most of the same use, and is supported once transfer is enabled.
- **Atomic bundles.** Every issuer in the bundle would have to act together. Each EEP settles on its own instead.
- **Accepting any newer EEP version whose issuer signature verifies.** A leaked, expired delegated key could sign a back-dated forgery. Catch-up is allowed only after a known restore and only for versions dated after the backup.
- **Salting only the agreement receipt.** The receipt points at the whole quote, so showing the quote would show every hidden field. Disclosure covers both records.
- **Requiring the other party's consent to disclose any deal evidence.** Every bad outcome would be hidden. Outcomes need no consent. Amounts still do.
- **Reviewer pseudonyms with free context keys.** A person can make any number of keys, so the distinct-reviewer count would break. Deferred until an authority can attest one key per member per context.
- **Policy hooks in v1.** No policy package exists yet, so hooks would have no users. The event log, which the statement needs anyway, is built instead, and hooks can read it later.
- **Witness committees for EEPs.** Waiting for the issuer's reissue already stops a double transfer. Witnesses cannot stop over-issuance, which is not a fork. They would also bring an unsolved problem: who chooses the witnesses.
- **A published Merkle sum-tree commitment of outstanding EEPs.** It is real work, and it proves only a lower bound. The outstanding total cannot separate "can deliver" from "holders went dormant". Redemption evidence is the faithful signal. Recorded as a later option.
- **A configurable state-machine engine per offering.** Strangers could not compare states, and evidence would lose meaning.
- **An independent unit of account.** The rule "redeem at list price" has no meaning unless EEPs and offerings use the same unit, and offerings are priced in a currency.
- **A non-transferable flag on EEPs.** Not needed. Transfer is always confirmed by the issuer.
- **A separate record for investment claims.** Every EEP is already a claim on future output. A second record would add nothing that an EEP does not have.
- **Multi-party atomic swaps.** Bilateral steps give the same netting, and each step is safe alone.
- **Forwarding searches more than one hop.** Later. It needs hop limits, loop rules and a merge across many rankings. One hop gives most of the reach.
- **Merging results by relevance score across exchanges.** Each exchange ranks by its own rules, so the scores mean different things. Own results first, then friends' in turn.
- **The subject's consent before an opinion is shown.** Every bad opinion would be hidden. The subject gets a response and its own opinion instead.
- **Opinions only from the party that received a service.** Both sides take a risk in a deal, so both can review.
- **Publishing opinions as soon as both are revealed.** The parties would have no time to fix a problem before it is public. The resolution window gives them that time.
- **Free revision during the resolution window.** A party could read a bad opinion and then lower its own rating in return. Revisions can only raise the rating.
- **A rating inside the fulfilment receipt.** It would put pressure on the receiver at signing time, give only one side a rating, and mix a fact with an opinion.
- **Pointer-only evidence on the exchange.** Evidence would vanish when the publisher is offline, often a phone. A subject could also press the publisher to delete it quietly.
- **A provider-side trusted execution environment or a notary to vouch for reviews.** The owner controls the hardware, and a notary learns who reviewed whom.
- **Tax as an acceptance rule.** It would give tax the power to block deals, which v1 does not need.

## Open questions

- **Legal review.** Closed by the owner on 2026-10-26: Roym builds transfer and keeps faithful records. Whether EEPs count as payment instruments or as "virtual digital assets" for tax in a country is for participants and operators to judge, not a gate on the build.
- **How far source 1 goes in v1.** Recommendation: distinct counterparties, time active, and counts labelled "stated by the participant". Signed counts, receipt sequence numbers and random sampling (from the reputation design note) later. The reputation module document decides.
- **Asking contacts.** How the asker hides which party it is interested in.
- **A refused EEP in a bundle.** One payment commitment per EEP, which lapses when its EEP is refused, or one payment for the bundle with a rule for refunds. The obligation change document decides.
- **Bids on one want.** Whether a want shows bidders how many bids it has, and whether a wanter can close a want without answering each bid.
- **First values to tune.** Starting points: standing cache 15 minutes; default EEP expiry 12 months; redemption notice at most 7 days (the mailbox's longest time-to-live); aggregates over at least 10 participants; opinion writing window 14 days and resolution window 7 days. Still to choose: the friend timeout and the cap per friend. The evidence size cap is measured, not guessed (see [Notes for the module documents](#notes-for-the-module-documents)).

## Notes for the module documents

These are correct to leave out of the overview. They are listed so the module authors see them.

**Agreements**
- Use the SD-JWT disclosure format instead of inventing one. Both receipt halves sign the same list of hashes, and the quote author picks the salts.
- Barter declared value: who declares it, and whether both sides must match.
- VX-13, VX-14 and VX-27 can be checked only on one party's node, or only against the reference code. Word their tests that way.

**Obligations**
- Partial transfer: the issuer reissues two EEPs in one step. This covers "pay ₹200 with a ₹300 EEP".
- Use a unique insert on `(EEP id, version)`, like the seat claim, not a lock.
- EEP expiry and the envelope's `expires_at_secs`: checking an expired EEP as evidence needs `allow_expired`.
- New record and card types: EEP, acknowledgement, endorsement, refusal, demand, decline, swap. The Hub has seven fixed card templates today.
- The proof that a `stored` receipt covers a given demand (for example, re-sealing with a seed the holder reveals).

**Exchange and membership**
- Standing during search: use one batched "changes since" feed per authority, not one call per member.
- Make the same-SynOrg case the same code path, with a cache time of 0.

**Reputation**
- Show the issuer's price history around redemptions, because an issuer can lower the value of its EEPs by raising prices.
- The evidence size cap: measure a real record with two receipt halves and their delegation certificates. 2 KB is probably too small.

**Event log and statement**
- The statement also lists the EEPs outstanding (issued and held) at the start and end of the period, not only the flows.

## Tasks

Milestone 8 is split into sub-milestones, in dependency order. Each has a folder under `docs/planning/milestones/` and a row in [roadmap.md](../../roadmap.md). Each module below gets its own change document before its sub-milestone starts.

- [ ] **M08A, open foundation.** [Roym open foundation](../roym-open-foundation/change.md): the participant rename, the federated query library, the event feed, remote clients, typed methods.
- [ ] **M08B, language and structure.** [Roym multilingual foundation](../roym-multilingual/change.md): string catalog, local formats, taxonomy with attributes, language tags, search keys and FTS5, translation hook points.
- [ ] **M08C, deal core and extensions.** Resource kinds. Commitments with kinds, states (each with a failure end), details and declared values, in one currency. Quote author as the single writer. Reserve and release hooks. Selective disclosure on the quote and the receipt. Barter. The event log and the statement export. [Roym extensions](../roym-extensions/change.md), with scheduling and one-shot. [Message forwarding](../roym-message-forwarding/change.md).
- [ ] **M08D, intents and shop.** Offerings and wants as one intent record in `catalog`, with in-return terms. [Roym shop](../roym-shop/change.md).
- [ ] **M08E, EEPs between two parties.** First: revocations gain `effective_from` and a reason, with the countersign rule after a key compromise, and the mailbox signs its `stored` receipt. Then the `obligation` service, deployed `public`, with its wire-reachable methods and mailbox paths: issue, redemption with the claim rule, decline, set-off, split signed by the issuer, expiry, holder acknowledgements, the restore rule, the per-issuer limit, the stated outstanding total. Work items: a wallet screen in the Hub and card types for EEP records.
- [ ] **M08F, discovery and matching.** Wants and bids (Want-Bid) next to offerings and enquiries (List-Search), with one negotiation flow. [Roym intent matching](../roym-intent-matching/change.md). Split the directory into `membership` and `exchange`. Friend exchanges with one-hop forwarding and the v1 merge rule.
- [ ] **M08G, reputation, evidence and opinions.** An ADR for the disclosure rule first. Evidence records, responses and withdrawals on the exchange, with credential labels. Opinions from both parties: sealed writing, reveal to each other, resolution window, then publish. Feedback that points at an extension step. Contact queries over Conversation, one level. The three-source view.
- [ ] **M08H, EEP transfer.** Endorsement, reissue, refusals, idempotency, bundles, two-party swaps, listing EEPs for trade. After M08G, so that a receiver can find an issuer's published evidence first.
- [ ] **Later: analytics and insight extensions.** Exchange aggregates with the minimum group size. Suggestions on the participant's node. Insight extensions (see [Roym extensions](../roym-extensions/change.md)).

**On approval**
- [ ] Add rows to [deferred-backlog.md](../../deferred-backlog.md) for each non-goal that is postponed: multi-hop forwarding, contact queries beyond one level, index sharding to Milestone 7, the Merkle commitment option, shared EEP pools, investment features for EEPs, policy hooks and the pre-sign hook, reviewer pseudonyms and context keys, more than one currency per agreement, and exchange content rules for opinions.
- [x] Replace the Milestone 8 row in [roadmap.md](../../roadmap.md) with M08A to M08H (2026-10-26).

## Deviations

## Close-out

Before merge, update the living docs so they describe the result, then set `status: done` and fill in `living-docs-touched`. Expected changes:

- `docs/system-requirements-spec.md`: replace the native ledger token in `[APP-ESC]` with EEPs; replace the EMA score in `[P2P-REP]` with the three-source evidence view and two-sided opinions; move the leaf shards of `[P2P-DSC]` to Milestone 7; add the participant role, resources, wants, the two ways to start an exchange, commitments, one-hop forwarding and the event log.
- `docs/roym-integrated-experience-spec.md`: roles, the Records table, the transaction state, search (D1), the "Not in the first release" list, and the rule "Payment evidence is not a public listing field", which this change narrows to amounts and payment details (see [Disclosure of deal evidence](#disclosure-of-deal-evidence) and its ADR).
- `docs/system-architecture.md`: the new and split services. Remove the legal review gate for mutual credit, by the owner decision above.
- `docs/TERMINOLOGY.md`: the terms in this document.
- `docs/reputation-design-note.md`: align it with the reputation rules here.
- `docs/planning/roadmap.md`: the M08A to M08H rows.

Record anything postponed in [deferred-backlog.md](../../deferred-backlog.md).
