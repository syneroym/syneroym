# Design Note: Reputation management in Syneroym

## 1. Purpose

A P2P ecosystem such as Syneroym needs a reputation mechanism that helps participants decide whom to trust without creating a central reputation authority.

The mechanism should be:

* **P2P and portable** — reputation should not belong to a single marketplace or aggregator.
* **Privacy-preserving** — participants should retain control over what interaction details and feedback they disclose.
* **Authentic** — disclosed reputation evidence should be difficult to fabricate or alter.
* **Fair to both sides** — both provider and consumer should be able to build reputation.
* **Resistant to selective self-presentation** — selectively presenting only favorable evidence should reduce the strength of the resulting claim rather than being treated as equivalent to a large body of evidence.
* **Constructive** — reputation should encourage good long-term behavior rather than permanently punish mistakes.
* **Transparent** — users should be able to understand the evidence behind a reputation and, where appropriate, how ranking is derived.

The central design principle is:

> **Syneroym should provide verifiable reputation evidence, not pronounce a universal reputation score.**

Applications, communities and individual users may derive their own reputation views from that evidence.

---

## 2. Reputation is Evidence, Not a Score

A conventional marketplace tends to reduce reputation to:

> 4.8 / 5 stars

This loses important information.

A better representation is a collection of independently verifiable facts and opinions:

* completed interactions;
* feedback given by counterparties;
* positive/negative/neutral outcomes;
* repeat interactions;
* disputes and their resolutions;
* endorsements and vouches;
* number and diversity of counterparties;
* age and continuity of the relationship;
* other domain-specific signals.

A reputation view can then be derived from this evidence.

For example:

> **4.8 / 5**
> 1,240 disclosed evaluations
> 860 distinct counterparties
> 4 years of activity

is materially different from:

> **5.0 / 5**
> 10 disclosed evaluations
> 10 distinct counterparties
> 2 months of activity

Neither should necessarily be artificially adjusted. The underlying evidence should simply be visible.

---

## 3. No Requirement for Complete Disclosure

A participant is not required to disclose every interaction or every piece of feedback.

Suppose a provider has actually served 100 customers and chooses to disclose 10 positive evaluations.

The system should allow this.

The provider may truthfully say:

> "I have 10 disclosed positive evaluations."

But the system should not treat this as equivalent evidence to:

> "I have 90,000 positive evaluations out of 100,000 disclosed transactions."

The latter represents a much larger body of evidence.

This avoids the need to solve the fundamentally difficult problem of discovering every private interaction a participant has ever had.

### Principle

> **Selective disclosure is permitted; selective disclosure does not receive the same evidentiary weight as a large, independently auditable history.**

A participant who discloses only a small amount of evidence can still have an excellent reputation, but the reputation is correspondingly less established.

---

## 4. Transaction Evidence

A transaction should produce a cryptographically verifiable interaction record.

Conceptually:

```text
Transaction
    |
    +-- Provider identity
    +-- Consumer identity
    +-- Transaction details / commitment
    +-- Provider attestation
    +-- Consumer attestation
    +-- Outcome
    +-- Feedback, if provided
```

The precise contents can vary by application.

The important property is that reputation evidence should be tied to a genuine interaction rather than being an arbitrary statement made by the subject.

A provider should not be able to manufacture:

> "Customer X gave me five stars"

without evidence attributable to Customer X.

Likewise, a customer should not be able to manufacture:

> "I successfully completed 500 purchases"

without corresponding transaction evidence.

---

## 5. Feedback is Owned by the Reviewer

Feedback is an opinion of the participant giving it.

Therefore:

* the reviewer controls whether the feedback is disclosed;
* the reviewed party controls whether they disclose feedback they received;
* either party may disclose information they legitimately possess, subject to applicable privacy and legal constraints;
* the system should not require negative feedback to become public;
* the system should not assume that undisclosed feedback is either positive or negative.

This creates an important distinction:

> **Existence of evidence, disclosure of evidence, and interpretation of evidence are separate concepts.**

Cryptography can establish provenance:

> "Alice made this evaluation concerning Bob."

It cannot establish the subjective truth of:

> "Bob was a good provider."

That distinction should remain explicit.

---

## 6. Reputation Commitments

Large reputations cannot practically be represented by publishing every underlying transaction to every requester.

A provider with 100,000 transactions should not need to transmit 100,000 records merely to establish that the history exists.

Instead, the provider can maintain a cryptographically committed history.

Conceptually:

```text
T1
T2
T3
...
T100000
     |
     v
Authenticated data structure
     |
     v
Root commitment
```

The root commits to the complete collection/order of records without requiring the records themselves to be published.

The provider can then selectively disclose records together with proofs that those records belong to the committed history.

---

## 7. Random Audit

A critical property is that the provider should not be able to choose which records constitute its evidence sample after making its reputation claim.

A requester can therefore challenge a reputation commitment.

For example:

> "You claim 90,000 positive evaluations out of 100,000. Show me randomly selected records 3,821, 18,492, 47,201, ..."

The provider provides the selected leaves and the cryptographic proofs necessary to establish that they belong to the committed history.

The requester does not need to receive the other 99,950 records.

This produces:

```text
Commit
   ↓
Requester chooses unpredictable samples
   ↓
Provider reveals selected evidence
   ↓
Requester verifies
```

The randomness must be determined **after the commitment**, so that the provider cannot prepare a favorable sample.

---

## 8. Auditability and Privacy

A sampled transaction may not always be legally or practically disclosable.

Therefore an audit request can have several outcomes:

1. **Valid disclosure**
   The evidence is disclosed and verified.

2. **Invalid disclosure**
   The evidence does not support the claim.

3. **Privacy-protected / unavailable**
   The evidence cannot legitimately be disclosed.

The third outcome should not automatically be treated as misconduct.

However, a provider that cannot make a meaningful portion of its claimed history auditable should receive less evidentiary weight.

Thus:

> **Privacy is respected, but unverifiable claims are not given the same confidence as verifiable claims.**

Over time, privacy-preserving cryptographic techniques could allow a participant to prove properties of a transaction without revealing its sensitive details.

For example:

> "This is a genuine completed transaction and the customer gave a positive evaluation."

without revealing the customer's identity.

---

## 9. Random Sampling Does Not Establish Absolute Truth

Random audits do not mathematically prove that every transaction in a 100,000-record history is genuine.

They establish statistical confidence.

If a provider has fabricated a significant fraction of its claimed evidence, sufficiently large unpredictable samples make detection increasingly likely.

Therefore the system can let the requester choose an audit depth appropriate to the importance of the decision.

A casual consumer might inspect a few samples.

A participant entering a large commercial contract might perform a much larger audit.

This creates a useful property:

> **The cost of verification can scale with the value of the decision.**

---

## 10. Large-Scale Reputation Histories

A participant may have a very large number of transactions and evaluations. It should not be necessary to publish every underlying record merely to establish a reputation claim.

A participant can instead commit to a large body of reputation evidence using an authenticated data structure, such as a Merkle tree or similar append-only structure.

Conceptually:

```text
T1
T2
T3
...
T100000
     |
     v
Authenticated data structure
     |
     v
Root commitment
```

The root provides a compact commitment to the underlying history.

Individual records can subsequently be disclosed together with a cryptographic proof that they belong to the committed history. A requester can therefore verify selected records without downloading the entire history.

For example, a provider claiming:

> **90,000 / 100,000 customers were happy**

might publish the commitment to the 100,000-record history and allow requesters to select unpredictable leaf positions for audit.

The requester might ask for 20 randomly selected records. The provider supplies those records and their membership proofs. The requester can then verify that the records genuinely belong to the committed history and inspect the underlying evidence.

The provider cannot simply maintain a small collection of favorable records and repeatedly present those records as its evidence, because the requester chooses the records to be examined after the commitment has been made.

Random sampling does not mathematically prove the entire history, but it provides increasing confidence as the number of unpredictable samples increases. The requester can choose an audit depth appropriate to the importance of the decision.

A record that cannot legitimately be disclosed for privacy or legal reasons may remain undisclosed. Such a record simply cannot contribute directly to the requester's audit.

More advanced cryptographic techniques, including zero-knowledge proofs, may eventually allow aggregate properties of large committed histories to be verified without revealing individual records. Such techniques are optional extensions rather than fundamental requirements of the reputation model.

The fundamental mechanism is therefore:

> **Commit → Randomly challenge → Selectively disclose → Verify.**

---

## 11. No Trusted Aggregator is Required

The reputation system should not require a central organization to calculate or certify:

> "Parikshit has reputation 87."

Instead, the underlying evidence can be portable.

Different applications can calculate different views:

```text
Marketplace A:
    4.8 / 5

Community B:
    Trusted

Marketplace C:
    92 / 100

My personal trust model:
    Strong
```

These are views over evidence, not competing claims about a single canonical reputation.

A third party may provide an aggregation service, but it should be an **optional reputation interpreter**, not the owner of the underlying reputation.

---

## 12. Ranking Should Remain Transparent

The system should expose the principal dimensions used in ranking.

For example:

* rating;
* number of evaluations;
* number of unique counterparties;
* transaction volume;
* repeat-customer rate;
* reputation age;
* evaluator reputation;
* dispute history;
* auditability.

An application might offer:

### Highest rated

Prioritizes satisfaction.

### Most established

Prioritizes breadth and longevity of interaction.

### Most popular

Prioritizes adoption.

### Highly rated, established

Requires a minimum evidence threshold.

### Trusted by my network

Weights evaluations from people or communities trusted by the requester.

The important principle is:

> **The user should be able to understand why someone is ranked highly.**

A black-box universal Syneroym reputation score should be avoided.

---

## 13. Reputation of the Evaluator

Not every evaluation should necessarily have equal evidentiary weight.

Consider:

```text
New identity
    |
    +-- 3 transactions
    +-- rates 500 people five stars
```

versus:

```text
Established identity
    |
    +-- 2,000 transactions
    +-- long history
    +-- diverse counterparties
    +-- rates provider positively
```

The second evaluation can reasonably carry more weight.

This creates a recursive reputation graph:

```text
Alice ──evaluates──> Bob
  |
  └── Alice has reputation
          |
          └── based on Alice's history
```

This also helps mitigate simplistic Sybil attacks.

---

## 14. Sybil Resistance is a Separate Problem

Cryptographic authenticity does not prove that an identity represents a unique human or organization.

A malicious participant could create many identities and manufacture apparently legitimate interactions among them.

Therefore reputation algorithms should consider:

* identity age;
* independent counterparties;
* transaction history;
* network diversity;
* repeated relationships;
* community membership;
* economic cost where appropriate;
* endorsements/vouches;
* suspiciously correlated activity.

No single mechanism should be assumed to solve Sybil resistance universally.

Different applications can use different policies.

---

## 15. Vouching and Endorsement

New participants have a natural disadvantage: they have no transaction history.

A separate mechanism can provide a bootstrap without simply transferring another person's reputation.

For example:

> "Alice vouches for Bob."

This should mean:

> "Alice is willing to associate her reputation with the legitimacy or character of Bob."

It should **not** mean:

> "Bob inherits Alice's reputation."

Vouches can themselves have different strengths:

* knows the person;
* has personally transacted with them;
* recommends their professional competence;
* is willing to stand behind them.

Where appropriate, stronger vouches can carry consequences for the voucher if the relationship proves problematic.

---

## 16. Disputes are First-Class Evidence

A negative review should not necessarily be treated as an established fact.

Instead:

```text
Transaction
    |
    +-- Consumer claim
    +-- Provider response
    +-- Evidence
    +-- Dispute
    +-- Resolution
```

The system can preserve both sides.

For example:

> Consumer alleges non-delivery.
> Provider disputes allegation.
> Evidence submitted.
> Community arbitration resolved in provider's favour.

This is substantially more informative than simply assigning:

> ★☆☆☆☆

to the provider.

Disputes and their outcomes should therefore form part of the reputation evidence model.

---

## 17. Reputation Should Be Contextual

There should not necessarily be one global reputation.

A person may have:

```text
Provider
    ├── Software consulting
    ├── Photography
    └── Equipment rental

Consumer
    ├── Timeliness
    ├── Payment reliability
    └── Transaction conduct
```

Reputation in one domain should not automatically transfer to another.

Similarly, an evaluation from a highly relevant domain may deserve more weight than one from an unrelated domain.

Applications should therefore be able to define reputation contexts.

---

## 18. Reputation Should Evolve

Reputation should not become an immutable lifetime score.

Recent behavior should generally matter more than very old behavior.

Positive behavior should be capable of rebuilding reputation.

Likewise, a single mistake should not necessarily permanently destroy a participant.

This makes reputation a mechanism for **long-term cooperation**, rather than permanent social punishment.

---

## 19. The Fundamental Trust Model

The system can be understood as four layers:

```text
                 REPUTATION VIEW
              "What do I think of X?"
                       ▲
                       │
              reputation algorithms
                       │
                       ▲
                 EVIDENCE
      evaluations / transactions / disputes
                       ▲
                       │
             cryptographic provenance
                       │
                       ▲
                INTERACTION
            A  ←────────────→  B
```

Each layer has a different responsibility.

### Interaction layer

Establishes that parties interacted.

### Evidence layer

Records what participants attest about that interaction.

### Cryptographic layer

Makes provenance, integrity and membership verifiable.

### Reputation layer

Interprets the evidence according to the needs of a particular application or community.

No single layer needs to solve every problem.

---

## 20. Design Principles

The resulting Syneroym reputation system can be summarized by these principles:

1. **No universal reputation score.**
2. **Reputation consists of evidence and derived views.**
3. **Every disclosed claim should have verifiable provenance.**
4. **Participants control disclosure of their private interactions and feedback.**
5. **Selective disclosure is permitted but naturally provides less evidence.**
6. **Large histories should be represented compactly through cryptographic commitments.**
7. **Requesters can perform unpredictable random audits of committed histories.**
8. **Privacy-protected records can remain undisclosed, but unverifiable claims receive less confidence.**
9. **Aggregate claims can eventually be proven without exposing all underlying records.**
10. **Completeness of a participant's entire life/history is not assumed or required.**
11. **Evaluator reputation can contribute to the weight of an evaluation.**
12. **Sybil resistance is treated separately from reputation.**
13. **Disputes and their resolutions are first-class evidence.**
14. **Reputation is contextual rather than necessarily global.**
15. **Ranking algorithms should expose their principal inputs and assumptions.**
16. **Reputation should be portable across applications and communities.**
17. **Reputation should be capable of improving through subsequent behavior.**

## 21. Core Philosophy

The objective is not to construct a system that knows whether every person is "good" or "bad."

That is neither technically possible nor desirable.

The objective is to construct a system where a participant can say:

> **"Here is the evidence I am willing to stand behind."**

and another participant can ask:

> **"How much of that evidence can I independently verify, how strong is it, how diverse is its source, and how much do I trust the people providing it?"**

The system then gives participants the tools to answer those questions themselves.

> **Syneroym provides evidence and verifiability; communities and individuals provide judgment.**
