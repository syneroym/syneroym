---
status: draft
living-docs-touched:
demo:
---
# Roym obligations

*Reader: developers who plan and build M08E (EEPs between two parties), and the project owner, who makes the product decisions listed under [Open questions](#open-questions).*

This is the **obligations module** change document of the [Roym Value Exchange](../roym-value-exchange/change.md) (the overview). The overview gives the rules. This document gives the exact formats for earnest effort promises (EEPs) between two parties: the records, their fields and limits, the states, the write order, the methods and the checks. It follows the overview's rules. Where it refines or differs from the overview, the difference is listed in [Deviations](#deviations).

**What an EEP is.** An EEP is a signed promise by its **issuer** to make an earnest effort to accept it against the issuer's own offerings, at a face value. The person who holds it is the **holder**. The holder can return it to the issuer for goods or services (a **redemption**), or use it to pay a debt to the issuer (a **set-off**).

Related documents:

- [Roym agreements](../roym-agreements/change.md): the commitment model, the single-writer rule for agreements, selective disclosure and the event log. An EEP is given or returned inside an agreement, as a commitment of resource kind `eep`. This document uses that model. It does not define it again.
- [Service mailbox](../service-mailbox/change.md): store-and-forward delivery to an offline service, the `stored` and `delivered` receipts, and the time-to-live of a message.
- [TERMINOLOGY.md](../../../TERMINOLOGY.md): mailbox, mailbox key, master DID, instance DID, on-behalf proof.

Transfer of an EEP to a third person comes later, in M08H. This document designs the EEP record so that transfer fits later without a change of format. See [Left for M08H](#left-for-m08h).

## Terms

Terms from the overview (participant, resource, commitment, declared value, unit of account, EEP, issuer, holder, reissue, set-off) and from the agreements document (deal, agreement id, writer, half, hidden field, disclosure, progress snapshot, Hub) keep their meaning. New terms here:

- **Version:** one signed record of one EEP. Version 1 creates the EEP. Each later change (closing, reinstatement, and in M08H a transfer) is a new version, signed by the issuer. Each version names the one before it.
- **Version chain:** all versions of one EEP, from version 1 to the latest.
- **Live version:** the latest version of an EEP whose `status` is `live`. Only a live version can be returned or split.
- **Closing version:** a version with `status: closed`. It ends the EEP. Nothing follows it.
- **Claim:** the issuer's lock on one live version, taken when an agreement that returns it is signed, or when a split is made. While a claim holds, nothing else can use that version.
- **Claim round:** one attempt to claim a version. A released claim is never deleted. The next claim of the same version is a new round.
- **Reinstatement:** a new live version with the same holder, signed by the issuer after a redemption that did not work out. It gives the EEP back to the holder.
- **Demand:** a holder's signed request to redeem an EEP, sent to the issuer through the `obligation` service, also when the two have never talked.
- **Decline:** the issuer's signed answer that it does not redeem now, with an optional reason.
- **Redemption notice:** the time the issuer has to answer a demand: 7 days (see [Open questions](#open-questions)).
- **Obligation address:** the service id (member master DID) of a participant's `obligation` service. Other nodes call this address.
- **Deposit proof:** what a holder keeps when its demand waits in the issuer's mailbox, so it can later prove which message the mailbox stored.
- **Known restore:** a restore from a backup that the node itself recorded, with the backup's time.
- **Fork:** two different versions of one EEP with the same id and the same version number, both signed by the issuer.

## Problem

Today Roym has no EEPs. A participant with skills or goods but little money cannot pay with a promise of future goods or work. The overview designs EEPs at the level of rules. Before M08E can be built, it needs exact formats.

Three facts in today's code also make the overview's plan harder than it reads:

- **Roym checks no key revocation.** Every Roym record is verified with no revocation source (`VerifyOptions::new(now)`, which uses `EmptyRevocations`). A person's revoked keys are a plain list of DIDs in the person's master anchor (`MasterAnchorPayload.revoked_keys`), with no time and no reason. So a revocation would either be ignored, or, if it were checked, it would fail every record the key ever signed.
- **The mailbox signs no `stored` receipt.** `Receipt::Stored` holds only a time. The mailbox sees only sealed bytes. It cannot know which message it stored, so a holder cannot prove that a waiting message was its demand.
- **There is no `obligation` service, and no Roym guest can read a master anchor.** Every new piece needs a home and a path.

## What a participant can do after this change

- Give an EEP as part of a deal: "a lesson for ₹300 in money and ₹200 as an EEP".
- See every EEP it issued and every EEP it holds on a wallet screen, with the standard earnest-effort sentence, the face value, the expiry and the state.
- Ask an issuer to redeem an EEP, also when the issuer is offline, and also with no earlier conversation. Get a quote back, or a signed decline with an optional reason.
- Redeem part of an EEP: return a ₹300 EEP for goods worth ₹200, and get a new ₹100 EEP back in the same deal.
- Cancel two EEPs against each other with another participant (set-off).
- Ask the issuer to split an EEP into smaller EEPs.
- Set a private limit per issuer, and see a warning when a deal would go over it.
- As an issuer, see the total face value of its outstanding EEPs.
- Keep proof of what happened: signed EEP versions, the holder's acknowledgements, declines, and the mailbox's signed receipt when the issuer did not answer.

## Scope and non-goals

- In scope:
  - **Prerequisites, built first:** key revocations gain `effective_from` and a reason, with the countersign rule after a key compromise; the mailbox signs its `stored` receipt; the deposit proof.
  - The EEP record and its full version chain, including the fields transfer will need: the holder, the version, the link to the prior version, and the issuer as the single writer.
  - The acknowledgement, demand, decline, split request and holdings request records.
  - The `eep-issue` and `eep-return` commitment kinds: their hidden `eep` field, their transition tables and how each settles.
  - Issue, redemption with the claim rule, decline, "no answer", set-off, split, expiry, reinstatement.
  - Catch-up after a known restore, and fork detection.
  - The per-issuer limit and the stated outstanding total.
  - The new `obligation` service: its data, its methods, which of them are wire-reachable, and how the `transaction` service calls it.
  - The EEP part of the event log and the statement's EEP section.
  - The `eep` card, and the wallet screen.
- Not in scope:
  - **Transfer to a third person and everything around it:** endorsement, reissue request, refusal, double-spend proof, bundles, two-party swaps, EEPs listed for trade, and partial transfer to another holder. These are M08H's slice H0. See [Left for M08H](#left-for-m08h).
  - **Features that present EEPs as investments:** an expected return or yield, a price history or chart for an EEP, ranking issuers by how much an EEP may gain. They are a product choice of the overview, not a legal gate.
  - A legal gate. There is none (owner decision, 2026-10-26).
  - Reputation views of EEP evidence (M08G). This document fixes the evidence records. M08G decides how a reader shows them.
  - A standing card that shows the outstanding total to strangers (M08G). This document gives the total to the issuer's own Hub.
  - Settling a fork. A fork is detected and proved here. A fork needs a stale reissue to another holder, which only transfer makes possible, so settling one is M08H's.
  - Key revocation checks for records of other Roym services. Only `obligation` reads revocations in M08E (see [Open questions](#open-questions)).
  - Recovery after the compromise of a person's master key. This document handles the compromise of a delegated signing key only.
  - Data migration. The product is not released. Formats change in place.

## Acceptance criteria

Each row refines an overview criterion (`VX-*`) or an agreements criterion (`AG-*`), or adds a rule this document needs. "Test" names the kind of test that proves it. File names are filled in when the tests are written.

Kinds of test, as in the agreements document:

- **Unit:** a test of a pure function in `roym_core`, `signed_record` or `mailbox_format` (no host).
- **Service:** a test of one service against a test host, with a controllable clock.
- **Two-node:** a substrate integration test with two nodes (`crates/substrate/tests`, using `SubstrateTestContext`). "Three-node" adds a node that runs the mailbox role.
- **Hub:** a test of the Hub's UI code (`crates/roym_web/ui`).
- **One node only:** the rule can only be checked on one party's own node, because only that node holds the facts. A test on another node can only check that the rule is shown as "not checked".
- **Reference code only:** the rule is about what a node chooses not to do. Another node cannot observe it. The test runs against Roym's own code.

| ID | Criterion | Test |
| --- | --- | --- |
| **Prerequisite: revocations (slice E1)** | | |
| OB-1 | A key revocation entry names the key, `effective_from_secs`, `revoked_at_secs` and a reason (`retired` or `compromised`). An entry with `effective_from_secs` later than `revoked_at_secs` is refused when the anchor is written (VX-22). | Unit. |
| OB-2 | A record signed with a `retired` key verifies when its `issued_at_secs` is before `effective_from_secs`, and fails from that time on (VX-22). | Unit: a table over (signed before, at, after). |
| OB-3 | A record signed with a `compromised` key verifies only with a countersign: a record by another identity that names the record's `record_id`, that verifies on its own, whose own key needs no countersign, and whose `issued_at_secs` is before `effective_from_secs`. A countersign by the same master, a countersign dated at or after `effective_from_secs`, and a countersign that itself needs a countersign are each refused (VX-22). | Unit: a table over each case. |
| OB-4 | `obligation` verifies every EEP record against the issuer's and the holder's current revocations, read from their master anchors. If an anchor cannot be read, the record verifies with revocation status `unknown`, as today, and the Hub shows "revocation not checked". | Service, with a fake anchor reader. |
| **Prerequisite: the signed `stored` receipt (slice E2)** | | |
| OB-5 | The mailbox signs every `stored` receipt: domain, mailbox DID, destination, deposit id, the SHA-256 of the sealed bytes, the time-to-live and the time. A repeated deposit returns the same signed receipt. `Receipt::verify_stored` refuses a receipt for other sealed bytes, another destination or another mailbox (VX-49). | Unit (`mailbox_format`); service (`mailbox`). |
| OB-6 | A deposit proof shows that a given demand was the stored message: the verifier re-seals the kept plaintext with the kept seed and the mailbox key in the issuer's signed service record, and compares the hash. It refuses a proof whose plaintext, seed, destination, record or demand differs in any byte. | Unit: one valid proof and one changed copy per field. Three-node: a demand to an offline issuer, then the proof verifies on a fourth identity's verifier. |
| **The EEP record** | | |
| OB-7 | Every EEP version has exactly the fields of [The EEP record](#the-eep-record). It has no free-text field. A version with an unknown field is refused. The Hub shows the standard earnest-effort sentence of the record version next to every EEP, from the string catalog, in the reader's language (VX-50). | Unit; Hub. |
| OB-8 | The EEP id is derived from the issuer and the cause of version 1. Issuing twice for the same commitment returns the same stored version 1 and signs nothing new. | Unit for the derivation; service for the repeat. |
| OB-9 | The issuer writes each version with a unique insert on `(eep_id, version)`. Of two different writes for the same pair, exactly one is stored, and the other signed record is never sent. | Service: a test host that runs two writes for one pair. |
| OB-10 | An EEP version still verifies after the delegated key that signed it has expired, when it was signed inside the certificate's window (VX-21). | Unit. |
| OB-11 | An EEP whose `expires_at_secs` has passed still verifies as a record, and shows `expired`. The issuer's node refuses a claim or a split of it, and declines a demand for it with code `eep-expired`. A claim taken before the expiry still resolves after it. | Service with a controllable clock. |
| **Issue** | | |
| OB-12 | **One node only.** The issuer's node refuses to sign a quote or an acceptance that makes it the giver of an `eep-issue` commitment while it has no active listing (VX-13, AG-24). | Service, on the issuer's node. |
| OB-13 | **One node only.** The issuer's node refuses an `eep-issue` commitment whose currency does not appear in one of its active listings (VX-14, AG-25). Every node refuses an `eep` commitment whose face value is not an integer from 1 to 10^15, or differs from its declared value. The receiving node shows the listing rule as "not checked" unless it found an active listing of the issuer in that currency. | Service on both nodes; unit for the validator. |
| OB-14 | When an agreement with an `eep-issue` commitment forms, the issuer's node signs version 1 with no action by the person, and sends it as an `eep` card. The writer moves the commitment to `issued` only when it holds a version 1 whose id, issuer, holder, currency, face value and expiry match the commitment. A version that does not match is kept as evidence, and the commitment does not move. | Two-node; service for the mismatch cases. |
| OB-15 | The holder's node signs an acknowledgement of each version that names it as holder, once, and sends it to the issuer's obligation address. The issuer stores it. A repeat is a no-op. | Two-node. |
| OB-16 | The transition tables of `eep-issue` and `eep-return` in [Commitments](#commitments) hold for every (state, event) pair (AG-6 for the EEP kinds). | Unit. |
| **Redemption, decline, no answer** | | |
| OB-17 | A holder sends a demand to the issuer's obligation address while the issuer is offline. The demand waits in the issuer's mailbox and is handled when the issuer comes back. The holder needs no conversation with the issuer for this (VX-15). | Three-node: the holder's node has no conversation thread with the issuer. |
| OB-18 | The issuer answers a demand with a quote that names the demand (`answers.kind = "demand"`), or with a signed decline. A decline by the person has an optional reason and no code. The node itself declines with a code, at once, a demand for an EEP that is unknown, expired, closed, claimed, or held at another version, or one signed by someone who is not the holder (VX-16). | Service for each code; two-node for a decline by the person. |
| OB-19 | The holder's node never accepts a quote that answers a demand on its own. When the issuer's declared value for a commitment that names the demanded listing is higher than that listing version's fixed price, the Hub shows "above the listed price" before the person signs. It does not block (VX-16). | Service; Hub. |
| OB-20 | A version claimed by one agreement cannot be used again. Of two agreements that return the same version, exactly one forms, and the other ends `not-formed` with `eep-claimed` (writer side) or `reserve-failed` with `eep-claimed` (counterparty side). A split of a claimed version is declined with `eep-claimed` (VX-17). | Service; two-node for the writer case. |
| OB-21 | When every commitment the issuer gives in a redemption, other than `eep-return` commitments, is final, the issuer's node closes or reinstates the returned version by the rule in [Resolving a return](#resolving-a-return). A cancelled or unformed agreement releases the claim, and the holder's version stays live (VX-17). | Service with a controllable clock, one case per row of the rule. |
| OB-22 | A restored issuer signs no second, different version 1, closing version or reinstatement for a place it already filled. It takes the record from its own `eep` card in the conversation first. | Service: restore an old backup, then sync. |
| OB-23 | After the redemption notice ends with no answer, the holder's Hub shows the demand as "no answer", with the evidence the holder holds: the mailbox's signed `stored` receipt and the deposit proof, and the issuer's signed `delivered` receipt if there is one. It labels this as weaker than a signed decline. A late answer changes the state to `answered-late`. | Service with a controllable clock; Hub. |
| **Set-off and split** | | |
| OB-24 | Two parties who hold each other's EEPs in the same currency return them in one agreement. Both versions are closed when the agreement forms, with no other step (VX-20). | Two-node. |
| OB-25 | Only the issuer splits. A split has 2 to 16 parts, each at least 1 minor unit, and their face values add up exactly to the original. The closing version and every part are stored in one write. A repeated split request returns the same parts. A split request signed by anyone but the holder of that version is declined with `not-holder` (VX-18). | Unit for the validator; service for the write; two-node for the round trip. |
| **Limits and totals** | | |
| OB-26 | A participant sets a private limit per (issuer, currency). A quote that would make the EEPs it holds from that issuer exceed the limit shows a warning. It never blocks (VX-19). | Service; Hub. |
| OB-27 | The issuer's Hub shows the total face value of its live EEPs per currency, labelled "stated by you". The total counts both versions of a fork. | Service. |
| **Keys and restore** | | |
| OB-28 | An EEP version signed with a key revoked as `compromised` verifies when the holder's acknowledgement of that version is dated before `effective_from_secs` (VX-22). | Unit; service. |
| OB-29 | After a known restore with backup time B, the issuer accepts a holder's newer version only when the rules in [Restore](#restore-from-an-old-backup) hold. It refuses a version dated at or before B with `version-unknown`. Without a known restore, it refuses every unknown newer version (VX-23). | Service: one case per rule. |
| OB-30 | Two versions with the same `(eep_id, version)`, both signed by the issuer and with different `record_id`, are a fork. `eep.fork-proof` verifies such a pair. The issuer's node records a fork when it sees one. Both holders' versions stay valid claims (VX-24). | Unit; service. |
| **Methods** | | |
| OB-31 | Exactly the five methods marked wire-reachable in [Methods](#methods) admit a call from another node, and only from a verified caller. Every other `obligation` method answers `-32013` from another node. An anonymous caller gets `-32013` on every method. | Service: extend the admission test table. |
| OB-32 | The issuer accepts a demand from any verified caller when the demand's own signature is the holder's. It sends the answer to the demand's `reply_to`, never to the caller. | Service. |
| OB-33 | A holder whose node lost its EEPs (for example after a restore) sends a holdings request. The issuer sends back the latest version of every live EEP that names that holder. | Two-node. |
| **Event log and statement** | | |
| OB-34 | Every EEP event this node takes part in is appended to `obligation`'s own log exactly once. A repeated call or `eeplog.rebuild` adds no duplicate. Nothing on the deal path reads the log (VX-45 for EEP events). | Service. |
| OB-35 | `obligation.statement-part` returns the `available` form of the statement's EEP section: the EEPs outstanding at the start and at the end of the period, by signed times, and the EEP events of the period (AG-30, VX-46). | Service; service of `transaction` with a real `obligation` sibling. |
| **Cards** | | |
| OB-36 | The card table has the `eep` card. The Hub renders it in the conversation with the sentence, the face value and the version. | Unit; Hub. |

## Design

### Facts checked in the code

These facts were checked on `main` at `933fb399` (2026-11-26). Each row names the function or constant, so that it stays useful when lines move.

| Fact | Where |
| --- | --- |
| Roym records are verified with no revocation source, so revocation status is always `unknown`. | `VerifyOptions::new` uses `EmptyRevocations` (`crates/signed_record/src/verify.rs`); every Roym caller, for example `crates/roym_core/src/transaction.rs`, `payment.rs`, `verdict.rs` |
| If a revocation source is given, a revoked signing key or issuer fails every record, with no time. | `verify` (`check_did` on the signer and on the issuer) in `crates/signed_record/src/verify.rs` |
| The verifier checks that the record's signer-written `issued_at_secs` is inside the delegation certificate's own window. `verify_chain_at` uses the current time only to refuse a certificate issued in the future. | `verify` in `crates/signed_record/src/verify.rs`; `verify_chain_at` in `crates/identity/src/delegation.rs` |
| A person's revoked keys are a list of DIDs in the master anchor, a pkarr record signed by the master. There is no time and no reason per entry. | `MasterAnchorPayload.revoked_keys` (`crates/core/src/dht_registry/master_anchor.rs`); `RegistryClient::revoke_key` (`client.rs`) |
| No WIT interface lets a guest read a master anchor. | `crates/wit_interfaces/wit/` (no anchor or revocation function) |
| A `stored` receipt is not signed. A `delivered` receipt, and a `rejected` one for a sender fault, is signed by the receiver's instance key. | `Receipt` and `sign` (`crates/mailbox_format/src/receipt.rs`) |
| The message hash a receipt names is computed from the plaintext message statement by the sender and the receiver. The mailbox sees only the sealed bytes. | `message_hash`, `seal_unchecked`, `check_signature` (`crates/mailbox_format/src/message.rs`); `Deposit` (`deposit.rs`) |
| Sealing uses HPKE (X25519, HKDF-SHA256, AES-256-GCM) with a fresh random ephemeral key. The `hpke` crate at the version in `Cargo.lock` (0.15.1) also has `single_shot_seal_with_rng`, which takes the random source as an argument. | `seal_to` (`crates/mailbox_format/src/crypto.rs`); the cached `hpke-0.15.1/src/single_shot.rs` |
| A sealed message is at most 64 KiB. An open mailbox keeps a message at most 7 days; a dedicated one at most 30 days. | `MAX_SEALED_BYTES`, `OPEN_MAX_TTL_SECS`, `DEDICATED_MAX_TTL_SECS` (`crates/mailbox_format/src/lib.rs`) |
| A message delivered from a mailbox reaches the service with the sender's member master as a verified caller, and no on-behalf proof. | Service mailbox change, `D-C6-6` |
| Mailbox eligibility is per WIT method (`<interface>#<method>`). Every Roym service exports one `invoke` method that carries the JSON method name inside. So eligibility covers all of a Roym service's JSON methods at once, and the admission table is the real gate (from reading, not run). | `mailbox_methods` (service mailbox change, `D-C6-5`); `crates/roym_transaction/wit/world.wit` |
| `admit::admit` takes a table of wire-reachable methods (`Open` or `VerifiedOnly`). A verified caller's DID names the calling service, never a person. | `crates/roym_core/src/admit.rs`; the only table today is `WIRE_REACHABLE` in `crates/roym_directory/src/api.rs` |
| The Roym app has seven services. `transaction` depends on `conversation`, `scheduling` and `catalog`, and has a wake every 30 seconds. | `services::ALL` (`crates/roym_core/src/services.rs`); `crates/roym_core/app/roym.toml` |
| The commitment model has no EEP kinds yet. A commitment's `eep` field exists, and the validator refuses any value but `null`. A quote answers an `Enquiry` or a `Want`. | `ResourceKind`, `CommitmentKind`, `Commitment.eep`, `DealError::EepField`, `Answers` (`crates/roym_core/src/deal.rs`) |
| The card table has ten types today. | `CARD_TYPES` (`crates/roym_core/src/card.rs`) |
| A unique insert is `AppDataLayer::create`: all rows or none, on one collection of one service, at most 200 rows. | `create` (`crates/wit_interfaces/wit/data-layer/data-layer.wit`); `MAX_BATCH_SIZE` (`crates/data_db/src/sqlite.rs`) |
| The scheduling extension's seat claim never deletes a row. Each state change is a unique insert, and a freed seat is taken again in a new round (`seat:<slot>:<seat>:<round>`). | `crates/roym_scheduling/src/app/reservation.rs`, `reservation/rows.rs` |
| A backup bundle's manifest is signed by the person as a `bundle-manifest` record. | `Bundle.manifest_signature` (`crates/roym_core/src/backup.rs`) |
| A profile and a listing each carry the owner's `conversation_address`, signed. | `ProfilePayload` (`crates/roym_core/src/person.rs`); `ListingPayload` (`listing.rs`) |

### Where it runs

- **A new service, `obligation`**, in the Roym SynApp, on every participant's node. It is deployed `public`, because only a public service can have a mailbox (service mailbox change, `D-C3-4`). Roym deploys it with a mailbox (see [Open questions](#open-questions)).
- `obligation` holds `CallTarget::Service`, because it calls the `obligation` service of a node it may never have met. Today `directory` is the only Roym service that does.
- `obligation` depends on `catalog` (to check that the issuer has an active listing). `transaction` and `web` gain a dependency on `obligation`. `obligation` never calls `transaction`, so there is no cycle. This matches the agreements document: only `transaction` calls `obligation`, never the other way.
- `obligation` has a wake every 60 seconds (`obligation.drive`) to send queued records and to release lapsed claims. It uses the same wake mechanism as `transaction.drive` ([ADR-0031](../../../decisions/0031-node-local-wake.md)).

**Two channels, one writer.**

| Traffic | Channel | Why |
| --- | --- | --- |
| EEP versions made inside an agreement (version 1 of an issued EEP, a closing version, a reinstatement) | `eep` cards in the agreement's conversation. `transaction` sends and files them, and hands each one to the local `obligation` service. | The deal already has a conversation, and the writer of the agreement needs the version to move the commitment. No address is needed. |
| Demands, declines, split requests and their parts, holdings requests, acknowledgements | Wire-reachable methods of `obligation`, through the mailbox when the other node is offline. | The holder may have no conversation with the issuer (VX-15). |

`obligation` is the only writer and keeper of EEP records on each node. `transaction` only carries `eep` cards. It never signs an EEP record.

### Who signs what

Every record here is signed by a person (the issuer or the holder) through the `signing` host interface, with the person's delegated key. Some are signed by the node with no person present, as the as-listed path already does for quotes. "Auto" in the table means the node signs on its own, because the record only states a fact the person already agreed to.

| Record | Signer | Auto or person |
| --- | --- | --- |
| `eep`, version 1 of an issued EEP | issuer | auto, when the agreement forms |
| `eep`, closing version after a return, or a reinstatement | issuer | auto, by [Resolving a return](#resolving-a-return) |
| `eep`, closing version and parts of a split | issuer | auto, on a valid split request |
| `eep-acknowledgement` | holder | auto, for each version it receives |
| `eep-demand` | holder | person |
| `eep-decline` without `code` | issuer | person |
| `eep-decline` with `code` | issuer | auto |
| `eep-split-request` | holder | person |
| `eep-holdings-request` | holder | person |

### Common rules for every record

- The rules of the agreements document hold: integers only, `LocalText` for free text, every party DID is a `did:key` master DID, canonical key-sorted JSON, payload at most 64 KiB.
- **Envelope expiry.** A record with an expiry also sets the envelope's `expires_at_secs` to it, as the quote does. So every verifier of EEP evidence uses `allow_expired` (`VerifyOptions::allowing_expired`) and then applies the EEP's own rule. An expired EEP is still a valid record. It only shows that the issuer's duty ended.
- **Revocations.** Every EEP record is verified with the time rule in [Revocations that take effect from a time](#revocations-that-take-effect-from-a-time).
- **No hidden fields.** EEP records carry no `_sd` list. They are private between the issuer and the holder, and they show amounts by nature: an EEP is its face value. A party shows them to a third party only on purpose. Inside an agreement, the `eep` commitment field stays hidden, as the agreements document says.
- **Record types.** `eep`, `eep-acknowledgement`, `eep-demand`, `eep-decline`, `eep-split-request` and `eep-holdings-request` join `RECORD_TYPES`, each at version 1.

### The EEP record

Record type `eep`, version 1. Signed by the issuer. One record is one version.

```text
EepPayload {
  eep_id:            string         // "eep_…"; see "The EEP id"
  version:           u32            // 1, 2, 3 …
  prior:             "rec_…" | null // record_id of the version before; null only for version 1
  status:            "live" | "closed"
  issuer_did:        string         // did:key; must equal the envelope issuer
  holder_did:        string         // did:key; the holder. In a closing version, the last holder.
  issuer_address:    string         // the issuer's obligation address, where holders call
  currency:          string         // ISO 4217, a code money::currency_minor_exponent knows
  face_value_minor:  i64            // 1 to 10^15
  expires_at_secs:   u64            // the issuer's duty ends at this time
  cause:             Cause          // why this version exists
}

Cause =
  | { kind: "issued",     agreement: "rec_…", commitment_id: string }  // version 1, from eep-issue
  | { kind: "split-part", parent: "eep_…", parent_version: u32,
                          part: u32, parts: u32 }                     // version 1 of one part
  | { kind: "returned",   agreement: "rec_…", commitment_id: string }  // closing, after eep-return
  | { kind: "reinstated", agreement: "rec_…", commitment_id: string }  // live, same holder, after a failed return
  | { kind: "split",      request: "rec_…",
                          parts: [ { eep_id, face_value_minor, holder_did } ] }  // closing
```

Field rules:

| Field | Rule |
| --- | --- |
| `version`, `prior` | Version 1 has `prior: null`. Every later version has `version` one higher than the version `prior` names. |
| `status` | `live` for `issued`, `split-part` and `reinstated`. `closed` for `returned` and `split`. Nothing follows a closing version. |
| `holder_did` | Must differ from `issuer_did`. An issuer cannot hold its own EEP: a returned EEP is closed, not held. |
| `issuer_address` | The issuer's current obligation address. Every version repeats it, so the latest version a holder has always says where to call. |
| `currency`, `face_value_minor` | The same in every version of one EEP. A split makes new EEPs; it never changes a face value. |
| `expires_at_secs` | From 7 days to 5 years after version 1's `issued_at_secs`; the Hub's default is 12 months (open question 1). The same in every version, except that a reinstatement may move it later (see [Resolving a return](#resolving-a-return)). The envelope expiry equals it. |
| `cause` | Version 1 has `issued` or `split-part`. A later version has `returned`, `reinstated` or `split`. |
| `split.parts` | 2 to 16 entries. Each `face_value_minor` is at least 1. The sum equals the closing version's `face_value_minor`. In M08E every `holder_did` equals the closing version's `holder_did`. Each `eep_id` is the derived id of that part. |

**No free text, no promise text.** The record has no field for a message or a promise. The promise is the standard sentence of the record version: record type `eep` at version 1 means the overview's sentence, word for word. The Hub shows it from its string catalog, in the reader's language, next to every EEP. If the wording ever changes, it becomes record version 2, and every version-1 EEP keeps its meaning (VX-50).

**The EEP id.** The id is derived, not random:

```text
eep_id = content_digest("eep_", { issuer_did, origin })
  origin for "issued":     { agreement, commitment_id }
  origin for "split-part": { parent, parent_version, part }
```

A derived id means a crash, a repeat or a restore can never make two EEPs for one commitment: the same inputs give the same id, and the unique insert on `(eep_id, 1)` refuses a second version 1. It also means the agreement does not have to name an id that does not exist yet. The overview's "id = hash of version 1" is replaced by this (see [Deviations](#deviations)).

**Size.** A version is a few hundred bytes of payload plus the envelope and its delegation certificate. It fits in a card and in a sealed message with room to spare. The E3 plan measures it.

### The `eep` commitment field

The agreements document reserves a hidden `eep` field on every commitment. This document fixes it.

```text
for eep-issue:   EepIssueTerms  { face_value_minor: i64, expires_at_secs: u64 }
for eep-return:  EepReturnTerms { eep_id: "eep_…", version: u32, eep_record: "rec_…",
                                  face_value_minor: i64 }
for eep-transfer: reserved for M08H
```

Validator rules, on every node (`quote.set`, `verify_quote`):

- `resource_kind` is `eep`, and `kind` is `eep-issue` or `eep-return`. `eep-transfer` stays `commitment-kind-not-supported` until M08H.
- `face_value_minor` equals the commitment's `declared_value_minor`. An EEP's value in the unit of account is its face value. A discount shows in the other side's values, not in a second value here.
- The EEP's currency is the agreement's `terms.currency`. So "one currency per agreement" holds, and the open point of the agreements document is closed (open question 4).
- `window`, `location`, `details`, `extension` and every currency-only field are `null`.
- For `eep-issue`: `expires_at_secs` is from 7 days to 5 years after the quote's `issued_at_secs`.
- For `eep-return`: the giver is the holder and the receiver is the issuer. The validator cannot check this from the quote alone. The issuer's node checks it when it claims (see [Redemption](#redemption)), and the holder's node checks it holds that live version before it signs.

### Commitments

The agreements document gives the overview's names for the EEP kinds and says this document fixes their tables. Every commitment starts `promised` when the agreement forms. "Final" states are marked **(final)**.

| Kind | Path to success | Failure ends |
| --- | --- | --- |
| `eep-issue` | `promised` → `issued` **(final)** | `ended-unconfirmed` **(final)**; `cancelled` **(final)** |
| `eep-return` | `promised` → `closed` **(final)** | `reinstated` **(final)**; `ended-unconfirmed` **(final)**; `cancelled` **(final)** |

The overview's `returned` state is dropped. The claim, taken when the agreement is signed, already makes the version unusable elsewhere, and the holder's signed agreement half already says "I return this version". A separate return endorsement would add a record and a step that change nothing (see [Deviations](#deviations) and open question 5).

**Events** (added to the agreements document's list):

- `eep-version`: the writer holds an EEP version signed by the issuer that matches this commitment, as below.
- `tick`, `cancel`, `other-failed`: as in the agreements document. `giver-half`, `receiver-half`, `payment-request` and `failure` are refused for the EEP kinds.

**What "matches" means.**

- For `eep-issue`: a version 1 whose `eep_id` is the derived id for (agreement, commitment), `issuer_did` is the giver, `holder_did` is the receiver, `status` is `live`, and `currency`, `face_value_minor` and `expires_at_secs` equal the commitment's.
- For `eep-return`: a version whose `prior` is the commitment's `eep_record`, whose `cause` names this agreement and commitment, and whose `issuer_did` is the receiver. `status: closed` with `cause.kind = "returned"` is a close. `status: live` with `cause.kind = "reinstated"` and `holder_did` equal to the giver is a reinstatement.

`eep-issue`:

| State | `eep-version` | `tick` | `cancel` | `other-failed` |
| --- | --- | --- | --- | --- |
| `promised` | `issued` | `ended-unconfirmed` | `cancelled` | `cancelled`, `other-commitment-failed` |
| any final | no change | no change | refused | no change |

`eep-return`:

| State | `eep-version` (close) | `eep-version` (reinstatement) | `tick` | `cancel` | `other-failed` |
| --- | --- | --- | --- | --- | --- |
| `promised` | `closed` | `reinstated` | `ended-unconfirmed` | `cancelled` | `cancelled`, `other-commitment-failed` |
| any final | no change | no change | no change | refused | no change |

- `claimed` is absent on the EEP kinds in the progress snapshot. They have no giver's half.
- "No change" in a final state keeps the version as evidence. A close that arrives after the commitment ended `ended-unconfirmed` still ends the EEP on both nodes. Only the commitment's state stays.
- **Track end.** `eep-issue` ends at `agreed_at_secs + TRACK_WINDOW_SECS`. `eep-return` ends at the latest track end of the other commitments the issuer gives in the agreement, or at `agreed_at_secs + TRACK_WINDOW_SECS` when there are none. Without this, a return could end `ended-unconfirmed` while the issuer's delivery still had time. Each party computes it from the disclosures it holds, as the agreements document says.
- The progress snapshot's `halves` lists the EEP version record behind each settled EEP commitment.

### Issue

1. **At signing.** When this node will be the giver of an `eep-issue` commitment, it calls `obligation.can-issue { currency }` before it signs the quote (as quote author) or its acceptance (as counterparty). `can-issue` answers `no-active-listing` when the person has no active listing in its own `catalog`, and `currency-not-offered` when no active listing is priced in that currency. The signing is then refused with that code (OB-12, OB-13).
   "Active listing" means a listing with status `active` in the issuer's own catalog. It need not be published to a directory: a stranger may reach it by a direct link, and the node cannot know what other nodes found (open question 9).
2. **At formation.** When the issuer's node sees the agreement formed (the writer after its countersign, the counterparty when it files the countersigned half), its `transaction` calls `obligation.issue`. `obligation` signs version 1 and stores it (see [Write order](#write-order-and-crash-recovery)). No person acts: the person already agreed to the EEP when it signed the agreement (open question 8).
3. `transaction` sends the version as an `eep` card in the conversation.
4. The holder's `transaction` files the card and calls `obligation.receive`. `obligation` verifies it, stores it as held, signs an acknowledgement, and queues the acknowledgement for the issuer's obligation address.
5. The writer moves the commitment to `issued` when it holds a matching version 1: the issuer's own when the issuer is the writer, the filed card when the holder is the writer.

The holder learns the issuer's obligation address from the version itself (`issuer_address`). The issuer needs no address of the holder for an issue.

### Redemption

A redemption is an ordinary agreement. The holder gives an `eep-return` of the issuer's EEP. The issuer gives what the holder asked for, usually a `fulfilment`. There are two ways to start one:

- **The two already talk.** The holder asks in the conversation, as for any deal, and the issuer writes a quote with an `eep-return` commitment.
- **A demand.** The holder sends a signed demand to the issuer's obligation address (VX-15). This works when the issuer is offline and when the two have never talked.

#### The demand

1. The holder picks a live EEP it holds and one active listing of the issuer that it saw. `demand.send` signs an `eep-demand` and stores it.
2. `obligation` calls `obligation.demand` on the issuer's obligation address. When the issuer is offline, the call waits in the issuer's mailbox (service mailbox change, S-2). The holder's node keeps the deposit proof (see [The mailbox side](#the-mailbox-side)).
3. The issuer's `obligation.demand` verifies the demand and runs the automatic checks below. If one fails, the node signs a decline with that code at once and sends it to the demand's `reply_to`. Otherwise it stores the demand, and the issuer's Hub shows it.
4. The issuer's person answers in one of two ways:
   - **A quote.** The Hub opens a conversation with the holder at the demand's `conversation_address` and fills a quote: an `eep-return` from the holder, and a commitment from the issuer that names the demanded listing at its current price. The quote's `answers` is `{ kind: "demand", record_id }`. `transaction` calls `demand.answered` on its own node. The holder's `transaction` does the same when it files the quote. From here it is a normal deal.
   - **A decline**, with an optional reason (`demand.decline`). The issuer's node sends it to `reply_to`.
5. If neither happens before `answer_by_secs`, the demand ends `unanswered` on both nodes (see [No answer](#no-answer)).

Automatic decline codes, checked in this order:

| Code | When |
| --- | --- |
| `eep-unknown` | the issuer has no record of this EEP id |
| `version-unknown` | the demand names a version newer than the issuer's latest, and the catch-up rules of [Restore](#restore-from-an-old-backup) do not accept it |
| `version-superseded` | the demand names a version older than the issuer's latest |
| `not-holder` | the demand's signer is not the holder of that version |
| `eep-closed` | the version is closed |
| `eep-expired` | the EEP's `expires_at_secs` has passed |
| `eep-claimed` | a claim on the version holds |
| `demand-late` | the demand arrived after its own `answer_by_secs` |
| `offering-withdrawn` | the listing the demand names is not active |

A first contact in Conversation with an offline party still needs one live `prekey-bundle` call (service mailbox change). So the quote of step 4 reaches the holder only when the holder's node is online once. The conversation outbox retries for 30 days. The demand itself does not have this limit.

#### List price is a maximum

The demand names the listing version the holder saw (`listing_record`). When the holder's node files a quote that answers a demand, it compares each issuer-given commitment whose hidden `listing_id` is the demanded listing with that listing version's fixed price. When the declared value is higher, the Hub shows "above the listed price", with both values, before the person signs. It never blocks: the holder may accept. A listing with no fixed price shows "price not checked". The holder's node never accepts a quote that answers a demand on its own (OB-19).

The quote and the demand are both signed, so a price above the list price is visible evidence. A refusal to go below the list price is normal.

#### The claim

The claim is the issuer's lock on the returned version. It is taken at signing time, on the issuer's own node, as the agreements document's reservations are:

- **The issuer is the writer** (the usual redemption: the issuer owns the listing and writes the quote). The claim is part of step 3 of the writer's decision: the writer reserves its own resources, and it also claims each `eep-return` it receives.
- **The issuer is the counterparty** (for example in a set-off where the other party wrote the quote). `agreement.accept` claims each `eep-return` this node receives before it signs, in the same place as its own reservations.

So for `eep-return` the **receiver** owns the hold, not the giver. The rule is fixed by the commitment kind, so no record field names the owner. This is the case the agreements document left out ("a case where the receiver owns the reservation is not in v1"). See [Deviations](#deviations).

`obligation` answers three local methods with the same meaning as an extension's reserve hooks:

| Method | Meaning |
| --- | --- |
| `obligation.claim` | Takes the claim for (agreement, commitment). Idempotent on that key. Checks, in order: the EEP is this node's own; the version is the latest and `live`; `holder_did` is the commitment's giver; `face_value_minor` and the currency match; the EEP has not expired; no claim holds. A failure answers the code of the table above. |
| `obligation.claim-confirm` | Marks the claim as part of a formed agreement. It never fails for a claim that exists. A claim never lapses on a timer, so there is no `reservation-lost` for EEPs. |
| `obligation.claim-release` | Frees the claim, while the version is still the one claimed. Idempotent. |

The writer's decision and recovery from the agreements document are unchanged, with "reservation" read as "reservation or claim". A failed claim ends the deal `not-formed` with the code as `state_reason` on the writer's side, or answers `reserve-failed` with the code on the counterparty's side.

**Release** happens when the deal ends `expired`, `not-formed`, `declined` or `cancelled`, and when the `eep-return` commitment ends `cancelled` (also by `other-commitment-failed`). The holder's version stays live and needs no new signature.

**A safety net.** `obligation.drive` releases a claim that was never confirmed once the quote's expiry has passed. A claim stores that time. This covers a `transaction` that crashed between the claim and the end of the deal and lost its row.

#### Resolving a return

When the issuer's node writes or files a snapshot in which every commitment the issuer gives, other than `eep-return` commitments, is final, it resolves each claimed `eep-return` of that agreement:

| Issuer-given commitments (other than `eep-return`) | Result |
| --- | --- |
| none (a pure set-off) | close at once |
| every one `fulfilled`, `acknowledged` or `issued` | close |
| any `ended-unconfirmed` with `claimed: true` (the issuer says it delivered, the holder did not confirm) | close (open question 3) |
| any `ended-unconfirmed` with `claimed: false` | reinstate |
| any `failed` or `cancelled` | not reached: `other-commitment-failed` has already cancelled the `eep-return`, and the claim is released |

- **Close**: `obligation.resolve-return` signs version v+1 with `status: closed`, `cause: returned`, and the agreement and commitment. `transaction` sends it as an `eep` card.
- **Reinstate**: version v+1 with `status: live`, the same holder, `cause: reinstated`. Its `expires_at_secs` is the later of the original and 30 days after signing, so a holder whose redemption failed near the end still has time to try again. `transaction` sends it as an `eep` card. The holder's node acknowledges it.
- `eep-return` commitments are left out of the rule, because they cannot fail once claimed. Without this, two returns in one set-off would each wait for the other forever.
- The overview says "if the delivery ends unconfirmed, the issuer reissues the EEP to the holder". The third row narrows that to the case where the issuer never claimed delivery. Closing is optional for the receiver (VX-55, a later owner decision), so the overview's rule would give a holder who never confirms both the goods and the EEP back. See open question 3.

### Partial redemption

"Pay ₹200 with a ₹300 EEP" between the holder and the issuer needs no split and no new mechanism. One agreement holds three commitments:

- the holder gives `eep-return` of the ₹300 EEP;
- the issuer gives the goods, declared ₹200;
- the issuer gives `eep-issue` of a new ₹100 EEP to the holder, as change.

The issuer is the writer, so the agreement forms in one decision, and both EEP records follow by the rules above. The Hub's redemption form offers this when the chosen goods cost less than the face value. The statement shows each commitment with its value, and the two sides balance.

### Set-off

Two parties who hold each other's EEPs in the same currency return them in one agreement: A gives `eep-return` of B's EEP, and B gives `eep-return` of A's EEP (VX-20). Each issuer claims its own EEP on its own node when it signs: the writer in its decision, the counterparty in `agreement.accept`. Neither issuer gives anything else, so each closes its EEP when the agreement forms (the first row of the rule).

When the face values differ, the party with the larger debt adds an `eep-issue` of the difference, or either party splits first. This is the same as partial redemption.

### Split

Only the issuer splits (VX-18).

1. The holder's `split.request` signs an `eep-split-request` and sends it to `obligation.split` on the issuer's obligation address.
2. The issuer's node checks: the request is signed by the holder of that version; the version is the latest and `live`; it has not expired; no claim holds; the parts follow the rules of [The EEP record](#the-eep-record). A failure answers a signed decline with the code, sent to `reply_to`.
3. The node signs the closing version (`cause: split`) and one version 1 for each part (`cause: split-part`), with the same currency, holder and expiry. It stores them all, together with a claim of the version by the split, in one unique insert (see [Write order](#write-order-and-crash-recovery)).
4. It delivers the closing version and the parts to `reply_to` with `obligation.deliver`. The holder's node stores them and acknowledges each part.

A split is mechanical, like a reissue. It changes nothing the issuer owes in total. So the node does it with no person action, and a refusal for any reason but the ones above would count against the issuer.

### Expiry

- After `expires_at_secs`, the issuer's duty ends. The node refuses claims and splits, and declines demands with `eep-expired`.
- A claim taken before the expiry still resolves after it, by [Resolving a return](#resolving-a-return).
- A demand signed before the expiry and not answered in its notice stays as evidence with the holder, also after the expiry.
- Neither node deletes EEP records in M08E. The overview allows the issuer to delete them after expiry. Keeping them is cheaper than a pruning rule, and the statement reads them. Pruning is a backlog item.
- `obligation.drive` appends an `eep-expired` log entry once for each EEP whose expiry has passed with no live claim.

### Per-issuer limit and the stated total

- **Limit.** `limit.set { issuer_did, currency, max_minor }` stores a private limit on the holder's own node. When the Hub shows a quote in which this party receives an `eep-issue` from that issuer, it calls `limit.check { issuer_did, currency, add_minor }`. The answer says whether live EEPs held from that issuer plus the new face value exceed the limit. The Hub then shows a warning. It never blocks (VX-19). In M08H the same check covers `eep-transfer`.
- **Stated total.** `obligation.outstanding` returns, per currency, the count and total face value of the live versions this node issued, counting both versions of a fork. The issuer's wallet screen shows it, labelled "stated by you". In M08G the standing card shows it to others as "stated by the issuer, not checked". This document adds no signed record for it.

### The mailbox side

#### The signed `stored` receipt

The overview asks the mailbox to sign its `stored` receipt (VX-49). The mailbox only ever sees sealed bytes, so it signs what it sees:

```text
StoredStatement {
  domain:       "syneroym-mailbox-stored-v1"
  mailbox:      string     // the mailbox node DID that the receiver's service record names
  destination:  string     // the receiver's member master DID
  deposit_id:   string     // 32 lowercase hex characters
  sealed_hash:  string     // lowercase hex SHA-256 of the sealed bytes, exactly as deposited
  ttl_secs:     u64
  at_secs:      u64        // when the mailbox stored it
}

Receipt::Stored { at_secs, proof: MailboxProof }    // proof is new
MailboxProof { signer_certificate: DelegationCertificate or absent, signature: string }
```

- The signature chains to the mailbox DID that the receiver's signed service record names. Whether the mailbox signs with its node key directly or with an instance key and a certificate is the E2 plan's choice (unverified). The verifier rule is the same.
- A repeated deposit with the same deposit id returns the stored receipt, with the same signature (service mailbox change, `D-C4-20`).
- `delivered` and `rejected` receipts stay as they are: signed by the receiver.
- A sender that reads a `stored` receipt with a missing or bad signature handles it as today: as not delivered. Only the EEP evidence needs the signature.

#### Which demand the receipt covers

The receipt proves "these sealed bytes were stored for this destination at this time". To prove that the bytes were a given demand, the holder must be able to rebuild them. Today the sealed bytes depend on a random ephemeral key that nobody keeps. So:

1. **Seal from a seed.** For a call that needs proof, the sender's router draws a random 32-byte `seal_seed` and seals with `hpke::single_shot_seal_with_rng`, using a cryptographic random generator seeded from `seal_seed`. HPKE then gives the same bytes for the same seed, key and plaintext. The E2 plan picks the seeded generator that matches the `rand_core` version of `hpke` 0.15.1 (unverified).
2. **Keep the proof.** The sender keeps a deposit proof and hands it to the calling guest (the holder's `obligation`). How the router hands it over is the E2 plan's choice (unverified; today a guest sees no receipt).

```text
DepositProof {
  proof_version:    1
  receiver_record:  string       // the issuer's signed service record the sender used (SignedEndpointInfo)
  destination:      string
  deposit_id:       string
  ttl_secs:         u64
  seal_seed:        string       // base64url, 32 bytes
  plaintext:        string       // base64url of the exact sealed plaintext (the SealedContent JSON)
  stored_receipt:   Receipt      // Stored, with its proof
  delivered_receipt: Receipt or absent   // the issuer's signed delivered receipt, when it came
}
```

**The check.** Given a demand envelope D and a deposit proof P, a verifier on any node accepts "the issuer's mailbox stored D for the issuer at `P.stored_receipt.at_secs`" only when every step passes:

1. `P.receiver_record` verifies as a signed service record, and was inside its validity at `at_secs`. Its service id equals `P.destination`, and also equals the `issuer_address` of an issuer-signed version of the EEP that D names. It names a mailbox M and a mailbox key K.
2. `P.stored_receipt` verifies as a `StoredStatement` signed for M, with `destination`, `deposit_id` and `ttl_secs` equal to P's.
3. Sealing `P.plaintext` to K with `P.seal_seed` gives bytes whose SHA-256 equals `sealed_hash`.
4. `P.plaintext` passes the receiver's own signature check (`check_signature` with `P.destination` and `P.deposit_id`, made public as a verify function in `mailbox_format`). So it is a real message, signed by the sender's service instance, for this destination and this deposit.
5. Its interface is `obligation`'s, its method is `invoke`, and the JSON request inside is `obligation.demand` with a `demand` byte-identical to D.
6. D verifies, its signer is the holder of the version it names, and `at_secs` is not later than `D.answer_by_secs`.

What this shows a third party: the demand's content, which is the point. The seed opens this one message only. It reveals nothing about the mailbox key. A sealed message is 64 KiB at most, so the proof is small.

What it does not show: that the issuer's node ever collected the message. Only the `delivered` receipt, signed by the issuer's instance, shows that.

A holder cannot use the proof to frame an issuer. Step 3 rebuilds the exact bytes, so garbage that the issuer could not open does not verify. The remaining trust is in the issuer's signed service record: if the issuer named a mailbox, it is responsible for it while the record is valid.

#### Direct delivery leaves no third-party proof

When the issuer is online, the router delivers directly and nothing is deposited. The holder then has no third-party evidence at all. So the holder's node asks the router to deposit a demand even when a direct attempt would work, whenever the issuer has a mailbox (a "deposit first" option on the call). The issuer still gets it within one pull (60 seconds). This is a small new router option, decided in E2 (open question 7).

#### No answer

A demand's `answer_by_secs` is its signing time plus the redemption notice (7 days). When it passes with no decline and no quote that answers it, the demand is `unanswered` on both nodes. The evidence the holder holds decides how strong it is:

| Evidence the holder holds | What it shows | Strength |
| --- | --- | --- |
| a signed decline, with or without a reason | the issuer answered "not now" | the reader judges; normal |
| the issuer's signed `delivered` receipt, and no answer | the issuer's node received the demand and did not answer | negative |
| the mailbox's signed `stored` receipt and a deposit proof, and no `delivered` receipt | the demand waited for the issuer, who never collected it | weaker negative |
| the demand only (direct delivery, or no mailbox) | nothing a third party can check | none |

The Hub shows each level with its own words. It never shows "no answer" as a decline. The overview's rule "no answer is weaker negative evidence than a signed decline" is the third row.

### Revocations that take effect from a time

Built in slice E1, before EEPs.

**The entry.** Each entry of `MasterAnchorPayload.revoked_keys` changes from a DID string to:

```text
RevokedKey {
  key:                 string   // the revoked delegated key's did:key
  revoked_at_secs:     u64      // when the master wrote this entry
  effective_from_secs: u64      // at or before revoked_at_secs
  reason:              "retired" | "compromised"
}
```

- `retired`: the key was taken out of use, for example a device was replaced. Its owner still trusts what it signed before.
- `compromised`: someone else may have the key. Signer-written times from it cannot be trusted.
- A lost device with no sign of misuse is `compromised`. Two reasons give two behaviours. More reasons would add none (open question 11).
- `effective_from_secs` cannot be after `revoked_at_secs`. A revocation into the future would let a key keep signing.
- The master anchor is a pkarr DNS record, which is small. Each entry now holds about 120 bytes. The E1 plan checks how many entries fit, and how `revoke_list_registry` takes the rest (unverified).

**The rule.** For a record R with signing key K (the certificate's `temporary_did`), issuer master M and signer-written time t = `R.issued_at_secs`:

1. If no entry of M's anchor names K, R is checked as today.
2. If the entry is `retired`: R is valid only when t < `effective_from_secs`.
3. If the entry is `compromised`: R is valid only when the verifier holds a countersign C with all of these:
   - C names R's `record_id` (for an EEP version: the holder's `eep-acknowledgement`, whose envelope subject is the version's `record_id`);
   - C's issuer is not M;
   - C verifies by this rule with no countersign of its own (a countersign that needs a countersign does not count, so the check never chains);
   - `C.issued_at_secs` < `effective_from_secs`.

`RevocationSource::check_did` returns the entry instead of a bare `Revoked`, and `verify` applies steps 2 and 3 to the signing key. The check on the issuer DID itself keeps today's meaning. Steps 1 to 3 are in `signed_record`, so every caller gets them. In M08E only `obligation` passes a revocation source and countersigns (open question 12).

**Reading the anchor.** A guest cannot read a master anchor today. E1 adds one host function that returns the revoked-key entries of a master DID, with a short cache, so that `obligation` can build a `RevocationSource`. An anchor that cannot be read gives revocation status `unknown`, as today (OB-4).

**What the countersign rule protects.** An honest holder's EEP survives a later compromise of the issuer's key, because the holder acknowledged each version when it got it. A forger with the stolen key can also make a fake "holder" acknowledge a fake EEP before `effective_from_secs`. The issuer's own store stops that case: the issuer is the single writer, so its node declines a demand for an EEP it never wrote with `eep-unknown`. The only weak point left is an issuer that lost its store, which [Restore](#restore-from-an-old-backup) handles.

### Restore from an old backup

**Recording a restore.** `obligation.import` restores a backup bundle. It then creates a row `restore:<n>` with `backup_at_secs` (the `issued_at_secs` of the bundle's signed `bundle-manifest`) and `restored_at_secs` (now). Only these rows make a restore "known". A node with no such row has never restored.

**Catch-up.** A holder shows the issuer a version w that is newer than the issuer's latest stored version of that EEP: in a demand, a split request or an acknowledgement. The issuer accepts w as its new latest version only when all of these hold, against the latest known restore:

1. w verifies, with `issuer_did` equal to this node's person.
2. `w.issued_at_secs` > `backup_at_secs`. A version from before the backup should be in the backup, so an unknown one is a forgery or a bug.
3. `w.issued_at_secs` < `restored_at_secs` + the clock skew allowance (24 hours). A version after the restore would be in the node's own new state.
4. No revoked-key entry of the issuer has `effective_from_secs` ≤ `w.issued_at_secs` for the key that signed w, whatever its reason.
5. The issuer has signed no version of this EEP since the restore. If it has, w and the issuer's newer version both continue from stale state: this is a fork (see below), not a catch-up.

When all hold, the node stores w as the latest version, marked `caught-up`, and appends an `eep-caught-up` log entry. The versions between the issuer's old latest and w stay missing. The node does not need them: w alone is the issuer's signed statement of who holds the EEP now. When a rule fails, the node declines with `version-unknown`, and the holder keeps the issuer-signed newer version as proof that the issuer's records are behind.

Without a known restore, every unknown newer version is declined with `version-unknown`. This is the overview's rule. It stops a person with a leaked, expired delegated key from writing a back-dated "newer version", because the verifier trusts the signer-written time (OB-29).

**Known limit.** A key that leaked and is not revoked, whose certificate window ends after the backup time, can still sign a version dated between the backup and the end of that window. The window is short (a delegated key's certificate), and it is open only right after a restore. Revoking such a key as `compromised` closes it.

**In-agreement records after a restore.** Before `transaction` asks `obligation` to sign version 1, a closing version or a reinstatement, it looks in the conversation for its own `eep` card with the same `(eep_id, version)`. If it finds one, `obligation` stores that record and signs nothing (OB-22). This is the agreements document's rule "the writer's records after a restore", applied to EEP versions.

**The holder's restore.** A holder that lost its held versions sends `obligation.holdings` with a signed `eep-holdings-request`. The issuer answers with `obligation.deliver`: the latest version of every live EEP whose holder is the request's signer (OB-33).

#### Forks

A fork is two versions with the same `(eep_id, version)`, both signed by the issuer, with different `record_id` (VX-24).

- `eep.fork-proof { a, b }` verifies such a pair, on any node, and returns the proof.
- The issuer's node checks for a fork whenever it receives a version of its own EEP: in a card, in a holder's message, or in catch-up rule 5. It stores the other version as a fork row and appends an `eep-forked` log entry. Its Hub shows a warning.
- Both versions stay valid claims on the issuer, because the issuer signed both. The outstanding total counts both.
- In M08E a fork can only come from a bug or a restore, because only one holder ever holds an EEP. Settling a fork (for example by a new EEP to the second holder) is left for M08H, where transfer makes stale reissues possible.

### Single writer and uniqueness

- **The issuer's `obligation` service is the only writer of the issuer's EEPs.** It writes every version and every claim. No other service and no other node writes them. The holder's node only stores copies, and signs acknowledgements.
- **The fence is a unique insert on `(eep_id, version)`**: row `ver:<eep_id>:<version>` in the collection `eeps`. Every new version, of every cause, is created with this row. Of two writes for one version (for example a close and a reinstatement after a bug, or a split and a close), exactly one succeeds (OB-9).
- **Claims are claim rounds**, as the scheduling extension's seats are. Row `claim:<eep_id>:<version>:<round>` holds the claim, with its owner (an agreement and commitment, or a split request), its quote expiry and its state. Row `free:<eep_id>:<version>:<round>` releases it. No row is ever deleted. To claim, the node reads the highest round. If that round has a `free` row, or no round exists, it creates the next round. Of two claims that race, the unique insert lets only one succeed. Row `key:<agreement>:<commitment_id>` names the round for one commitment, so a repeated claim finds it.
- **All the issuer's rows are in one collection, `eeps`.** A `create` covers one collection of one service, so this is what lets a split store its claim, its closing version and its parts in one write.

### Write order and crash recovery

Two rules from the agreements document hold for every step here:

- **A signed record is made once.** The node signs, then stores the record with a unique insert before it sends it. If the row already exists, it uses the stored record and signs nothing new.
- **The row that ends a step is written last.** Sending is driven by stored `out:` rows, which are created in the same write as the record. `obligation.drive` sends each `out:` row that has no `sent:` row, and creates `sent:` after the receiver answered or the mailbox stored it. A crash before `sent:` makes the next drive send again, and the receiver files a repeat as a repeat.

Where a step spans more than one service, the order is:

| Operation | Steps, in order | Crash recovery |
| --- | --- | --- |
| **Issue** (issuer node) | 1. `transaction`, on the formed agreement, looks for its own `eep` card of version 1 in the conversation. 2. It calls `obligation.issue`, with the found card if any. 3. `obligation`, if `ver:<id>:1` exists, returns it. Else it uses the found card, or signs version 1, and creates `ver:<id>:1`. 4. `transaction` sends the `eep` card (`conversation.send` is idempotent per record). | A crash before step 3's create signs nothing that was sent. A crash after it reruns step 2 on the next sync, which returns the stored version. A crash after the send, then a restore from an old backup, finds the card in step 1. |
| **Claim** (issuer node, inside the writer's decision or `agreement.accept`) | 1. `transaction` calls `obligation.claim`. 2. `obligation` creates `claim:<id>:<v>:<r>` and `key:<agreement>:<commitment>` in one write. 3. `transaction` continues with the agreements document's steps (other reservations, then confirm, then countersign). 4. `obligation.claim-confirm` marks the claim with a new row `confirmed:<id>:<v>:<r>`. | The agreements document's recovery reruns the decision. `claim` is idempotent on `key:`, `claim-confirm` on `confirmed:`. An unconfirmed claim past its quote expiry is released by `obligation.drive`. |
| **Release** | `obligation.claim-release` creates `free:<id>:<v>:<r>`, only when the round is the one `key:` names and no `ver:<id>:<v+1>` exists. | Idempotent. A release that fails is tried again by the next sync of `transaction`, as for reservations. |
| **Resolve a return** (issuer node) | 1. `transaction` sees the rule of [Resolving a return](#resolving-a-return) satisfied, and looks in the conversation for its own `eep` card of version v+1 of that EEP. 2. It calls `obligation.resolve-return { agreement, commitment_id, outcome }`, with the found card if any. 3. `obligation` checks that the claim round of `key:` holds, then creates `ver:<id>:<v+1>`. No `out:` row is needed: the version goes as a card. 4. `transaction` sends the `eep` card. | As for issue. If `ver:<id>:<v+1>` exists with another cause, the node records a conflict in its log and keeps the stored version. Its own code never makes one: the claim is the only path to v+1. |
| **Split** (issuer node, on `obligation.split`) | 1. Verify the request and run the checks. 2. If `req:<request record_id>` exists, re-queue its stored results and stop. 3. Sign the closing version and every part. 4. `create` in `eeps`, in one call: `claim:<id>:<v>:<r>` (owned by the request), `ver:<id>:<v+1>`, `ver:<part>:1` for each part, `req:<request>`, and one `out:` row naming all the records for `reply_to`. At most 21 rows. 5. `obligation.drive` delivers. | If step 4 fails because a row exists (a claim or a version won the race), the signed records were never stored or sent, and are dropped. The node answers the request with a decline (`eep-claimed` or `version-superseded`). After step 4, a repeat request finds `req:` in step 2. |
| **Decline** (issuer node) | 1. Sign the decline. 2. `create` `decline:<demand record_id>` and its `out:` row. 3. Drive sends. | A repeat finds `decline:` and signs nothing new. |
| **Receive a version** (holder node) | 1. `transaction` files the `eep` card, or `obligation.deliver` receives a record. 2. `obligation` verifies it and creates `held:<id>:<version>` in collection `held`. 3. It signs an acknowledgement and creates `ack:<record_id>` with its `out:` row in the same collection. | A repeat finds `held:` and `ack:`, and signs nothing new. |
| **Demand** (holder node) | 1. `demand.send` signs the demand and creates `demand:<record_id>` and its `out:` row in `held`. 2. Drive sends it, deposit first. 3. When the router hands over a deposit proof, the node stores it on the demand row. | A crash before the send makes the next drive send. The receiver dedups by the idempotency key (the demand's `record_id`). |

### States

#### The EEP, on the issuer's node

```text
              claim                         close (v+1, closed)
   live ──────────────▶ claimed ───────────────────────────────▶ closed (final)
    ▲  │                   │  │
    │  │ expires_at passes │  │ reinstate (v+1, live, same holder)
    │  ▼                   │  └──────────────────────────────▶ live (version v+1)
    │ expired (final for   │
    │  the issuer's duty)  │ release (no new version)
    └──────────────────────┘
   live ── split (claim and closing version in one write) ──▶ closed (final);
                                                              each part starts live
```

| State | Means | Ways out |
| --- | --- | --- |
| `live` | the latest version is `live`, no claim holds, the expiry has not passed | claim → `claimed`; split → `closed`; the expiry passes → `expired` |
| `claimed` | a claim round holds for this version | release → `live` (same version); close → `closed`; reinstate → `live` (v+1). A claim always ends: an unconfirmed one by release at the deal's end or by the drive's safety net after the quote expiry; a confirmed one by [Resolving a return](#resolving-a-return), which runs once the issuer's other commitments are final, and every commitment becomes final at its track end. |
| `closed` | the latest version is `closed` | none: final |
| `expired` | `expires_at_secs` has passed with no claim | none: final. A claim taken before the expiry stays in `claimed` and resolves as usual. |

A fork is a flag on the EEP, not a state. Both versions keep their own state.

On the holder's node, the same states are shown from the versions the holder holds. The holder does not see the issuer's claim rows. It sees "returned in agreement X" from its own agreement.

#### The demand

```text
 sent ──▶ deposited ──▶ delivered ──▶ quoted     (final)
   │          │             │   └──▶ declined    (final)
   │          │             └──────▶ unanswered ──▶ answered-late (final)
   │          └────────────────────▶ unanswered
   └───────────────────────────────▶ unanswered
 any state before an answer ──▶ undeliverable (final)   // a signed rejected receipt
```

| State | Means | Ways out |
| --- | --- | --- |
| `sent` | the holder's node signed and queued it | deposited, delivered, quoted, declined, unanswered at `answer_by_secs`, undeliverable |
| `deposited` | the issuer's mailbox signed a `stored` receipt | delivered, quoted, declined, unanswered at `answer_by_secs`, undeliverable |
| `delivered` | the issuer's signed `delivered` receipt came, or a direct call succeeded | quoted, declined, unanswered at `answer_by_secs` |
| `quoted` | a quote that answers it was filed | none: final. The deal goes on as a deal. |
| `declined` | a signed decline was filed | none: final |
| `unanswered` | `answer_by_secs` passed with no answer | a late quote or decline → `answered-late` |
| `answered-late` | an answer came after `answer_by_secs` | none: final. The answer is kept and shown with its date. |
| `undeliverable` | a signed `rejected` receipt came (for example the issuer's key was revoked, or the method is not eligible) | none: final |

The issuer's node keeps the states that apply to it: `received`, then `quoted`, `declined`, `unanswered` (its Hub shows "you did not answer"), or `answered-late`.

### Records

All records are version 1. Limits follow the agreements document where a field is the same.

**`eep`.** See [The EEP record](#the-eep-record).

**`eep-acknowledgement`.** Signed by the holder named in the version. The envelope's `subject` is the version's `record_id`. No expiry.

```text
EepAcknowledgementPayload {
  eep_id:      "eep_…"
  version:     u32
  eep_record:  "rec_…"      // the version acknowledged; equals the envelope subject
  issuer_did:  string
  holder_did:  string       // equals the envelope issuer and the version's holder_did
}
```

**`eep-demand`.** Signed by the holder of the version it names. Envelope expiry equals `answer_by_secs`.

```text
EepDemandPayload {
  eep_id:               "eep_…"
  version:              u32
  eep_record:           "rec_…"
  issuer_did:           string
  listing_id:           string           // the issuer's listing the holder wants
  listing_record:       "rec_…"          // the listing version the holder saw
  note:                 LocalText | null // 512 bytes; for example "2 kg, after Monday"
  conversation_address: string           // where the issuer may open a conversation to quote
  reply_to:             string           // the holder's obligation address
  answer_by_secs:       u64              // issued_at_secs + REDEMPTION_NOTICE_SECS
}
```

- `note` is not hidden. A demand may become evidence, so the Hub tells the holder that the note may be shown to others.
- One open demand per (EEP, version) on the holder's node. A second `demand.send` for the same version while one is open is refused with `demand-open`.

**`eep-decline`.** Signed by the issuer.

```text
EepDeclinePayload {
  answers:     "rec_…"          // the demand or split request declined
  eep_id:      "eep_…"
  version:     u32
  holder_did:  string           // the signer of the declined request
  code:        string | absent  // from the automatic decline codes; absent for a person's decline
  reason:      LocalText | null // 512 bytes; a person's optional reason
}
```

A decline with a `code` states a fact the node checked. A decline without one is the issuer's choice, which a reader judges by the earnest-effort rule.

**`eep-split-request`.** Signed by the holder.

```text
EepSplitRequestPayload {
  eep_id, version, eep_record, issuer_did
  parts:     [ i64 ]   // 2 to 16 face values, each at least 1, sum = the version's face value
  reply_to:  string
}
```

**`eep-holdings-request`.** Signed by the holder.

```text
EepHoldingsRequestPayload {
  issuer_did:  string
  reply_to:    string
}
```

The issuer answers at most once per hour per holder, to bound the work a request can cause.

### Methods

All methods are on the new `obligation` service, reached through `invoke` like every Roym method. Admission uses `roym_core::admit::admit` with a table, as `directory` does today:

```text
pub const WIRE_REACHABLE: &[(&str, WireRule)] = &[
    ("obligation.demand",      WireRule::VerifiedOnly),
    ("obligation.split",       WireRule::VerifiedOnly),
    ("obligation.acknowledge", WireRule::VerifiedOnly),
    ("obligation.holdings",    WireRule::VerifiedOnly),
    ("obligation.deliver",     WireRule::VerifiedOnly),
];
```

Every other method is local-only (`-32013` from another node).

**How wire calls are admitted.**

- **Verified only, never open.** Each of these methods stores something on the receiving node. A verified caller lets the node limit the rate per caller and refuse a caller it blocks. A message from a mailbox always arrives with a verified caller (service mailbox change, `D-C6-6`), so this costs the offline path nothing.
- **The caller is not the principal.** The verified DID names the calling service, never a person (`admit.rs`). The principal is the signer of the record inside the call: the holder for a demand, a split request, a holdings request or an acknowledgement, the issuer for a delivered record. The receiving node checks that signature against the EEP. It does not compare the caller's DID with anything. This follows the standing rule: any verified immediate caller may present a principal's proof, and the receiver is responsible for checking that proof.
- **Answers go to the signed `reply_to`,** never to the caller. A replayed request only sends the answer back to the holder's own service.
- **Mailbox eligibility.** The manifest lists `syneroym-roym:obligation/api@0.1.0#invoke` in `mailbox_methods`. Eligibility cannot be narrower than `invoke`, so the admission table is the real gate. A deferred call to a local-only method is refused with `-32013`, and the mailbox closes it with an unsigned `rejected(call_failed)`.
- **Every wire method is idempotent and returns `{}`.** The idempotency key is the `record_id` of the record it carries (for `acknowledge` and `deliver`, a hash of the sorted `record_id`s). The answer, if any, comes back as a call to the other side's `obligation.deliver`.
- **Limits.** A call carries at most 16 records. Each caller may make at most 60 wire calls per hour (a first value to tune). Over the limit, the call is refused with a rate-limit error, and the sender retries later.

| Method | Wire | Caller | What it does |
| --- | --- | --- | --- |
| `obligation.demand` | **yes** | a holder's node | Input `{ demand }`. Verifies, runs the automatic decline checks and catch-up, stores the demand. **Why wire:** a holder who never talked to the issuer, or whose issuer is offline, must reach it (VX-15). |
| `obligation.split` | **yes** | a holder's node | Input `{ request }`. See [Split](#split). **Why wire:** splitting needs the issuer, who may be offline. In M08H the holder may be a stranger, so the conversation is not a path. |
| `obligation.acknowledge` | **yes** | a holder's node | Input `{ acknowledgements: [envelope], versions: [envelope] }`. Stores each acknowledgement of a version this node wrote. A version the node does not know goes to catch-up. **Why wire:** the acknowledgement must reach the issuer for VX-22, and it may arrive while the issuer is offline. |
| `obligation.holdings` | **yes** | a holder's node | Input `{ request }`. Queues the latest live versions for that holder. **Why wire:** a holder who lost its data may have no conversation left. |
| `obligation.deliver` | **yes** | an issuer's node | Input `{ records: [envelope] }`: EEP versions, declines. Verifies each against the EEP and the issuer, stores it, acknowledges new versions. **Why wire:** this is how answers to the four methods above come back. |
| `obligation.can-issue` | no | `transaction` | Input `{ currency }`. Answers `ok`, `no-active-listing` or `currency-not-offered`. |
| `obligation.issue` | no | `transaction` | Input: agreement, commitment id, holder, currency, face value, expiry, optional found card. Returns version 1. |
| `obligation.claim`, `.claim-confirm`, `.claim-release` | no | `transaction` | See [The claim](#the-claim). |
| `obligation.resolve-return` | no | `transaction` | Input: agreement, commitment id, `outcome: close \| reinstate`, optional found card. Returns version v+1. |
| `obligation.receive` | no | `transaction` | Input: an `eep` card's envelope from a conversation. Stores it as held, acknowledges it. |
| `obligation.held-check` | no | `transaction` | Input: an EEP id and version. Answers whether this node holds it as a live version. The holder's node calls it before it signs an `eep-return`. |
| `eep.get`, `eep.list` | no | Hub | The EEPs this node issued or holds, with their versions, state, acknowledgements and demands. |
| `eep.verify` | no | Hub | Verifies an EEP version from anyone, with the revocation rule, and returns the verdict. |
| `eep.fork-proof` | no | Hub | Verifies two versions as a fork. |
| `demand.send`, `.get`, `.list` | no | Hub | The holder's demands, with their state and evidence. |
| `demand.decline` | no | Hub | The issuer's person declines, with an optional reason. |
| `demand.answered` | no | `transaction` | Marks a demand `quoted` when a quote answers it. |
| `split.request` | no | Hub | The holder asks for a split. |
| `holdings.request` | no | Hub | The holder asks an issuer for its current versions. |
| `limit.set`, `.list`, `.check` | no | Hub | The per-issuer limit. |
| `obligation.outstanding` | no | Hub | The stated total per currency. |
| `obligation.statement-part` | no | `transaction` | See [The statement hook](#the-statement-hook). |
| `eeplog.list`, `eeplog.rebuild` | no | Hub | As `eventlog.*` in `transaction`. |
| `obligation.drive` | no | the wake | Sends `out:` rows, releases lapsed claims, appends expiry entries. Bounded per run, as `transaction.drive` is. |
| `obligation.export`, `.import`, `.signing-status`, `.install-signing-certificate`, `obligation.ping` | no | Hub, `roymctl` | As in every Roym service. `import` records the restore. |

`transaction` changes, all local-only (AG-31 still holds):

- `quote.set` and `verify_quote` accept the two EEP kinds and the `eep` field, by [The `eep` commitment field](#the-eep-commitment-field). `Answers` gains `{ kind: "demand", record_id }`.
- `quote.set` and `agreement.accept` call `obligation.can-issue` for an `eep-issue` this node gives, `obligation.held-check` for an `eep-return` this node gives, and `obligation.claim` for an `eep-return` this node receives.
- `transaction.sync` sends and files `eep` cards, calls `obligation.issue`, `.receive` and `.resolve-return`, and applies the `eep-version` event.

### Card types

| Card | Change |
| --- | --- |
| `eep` | new. One EEP version, sent by `transaction` in the conversation of the agreement that made it. The Hub shows the earnest-effort sentence, the face value and currency, the expiry, the version, and the cause ("issued in this deal", "returned", "given back to you"). |

Demands, declines, split requests, acknowledgements and holdings requests are not cards. They travel on the obligation channel, and the Hub shows them on the wallet screen. A quote that answers a demand is an ordinary `quote` card.

The agreements document's card table has twelve types once the extensions' `widget` and `widget-answer` land (ten in `CARD_TYPES` today). With `eep` it has thirteen.

**The wallet screen** is a new Hub screen with two lists: "EEPs I hold" and "EEPs I issued". Each row shows the issuer or holder, the face value, the expiry, the state and the sentence. A held EEP offers "Ask to redeem", "Ask to split" and "Request my EEPs again". An issued EEP shows its demands and acknowledgements. The screen also holds the per-issuer limits, the stated total, and the open demands to answer. It shows no yield, no price history and no ranking of issuers.

### The event log

`obligation` keeps its own log, as the agreements document decided ("The `obligation` service keeps its own log of EEP events").

Collection `eep_log`. Entries are only ever created. Each entry's row id is derived from the event, so creating it twice is refused, and a repeat is a no-op. Every call that writes a record runs the same derive step as `eeplog.rebuild`, so a crash between a write and its entry is repaired on the next call.

```text
EepLogEntry {
  id:               "<eep_id>:<version>:<event>" | "<demand record_id>:<event>"
  occurred_at_secs: u64        // issued_at_secs of the signed record behind the event
  recorded_at_secs: u64        // indexed; when this node created the entry; the list cursor
  event:            "eep-issued" | "eep-received" | "eep-returned" | "eep-reinstated"
                  | "eep-split" | "eep-split-part" | "eep-expired" | "eep-caught-up"
                  | "eep-forked" | "demand-sent" | "demand-received" | "demand-declined"
                  | "demand-quoted" | "demand-unanswered"
  role:             "issuer" | "holder"
  eep_id:           string
  version:          u32
  counterparty_did: string     // the holder, seen from the issuer; the issuer, seen from the holder
  currency:         string
  face_value_minor: i64
  agreement:        "rec_…" | absent
  records:          [ "rec_…" ]
}
```

- `eep-expired` and `demand-unanswered` have no signed record behind them. Their `occurred_at_secs` is `expires_at_secs` or `answer_by_secs`, so both parties still give the event the same time.
- Nothing on the deal path reads the log (OB-34).
- The log holds face values in plain form. It never leaves the node, except in the participant's own statement and backup.

### The statement hook

The agreements document fixes the statement's `eeps` section and its three states, and leaves the format of `events` and the method to this document. `transaction`'s `statement.export` calls `obligation.statement-part { from_secs, to_secs }` once, and puts the answer in `eeps` with `status: "available"`.

```text
StatementPart {
  outstanding_at_start: [ EepPosition ]   // at from_secs
  outstanding_at_end:   [ EepPosition ]   // at to_secs
  events:               [ EepEvent ]      // occurred_at_secs in [from_secs, to_secs)
}

EepPosition {                             // as fixed by the agreements document
  eep_id, version: u32
  role:             "issuer" | "holder"
  issuer_did, holder_did
  currency, face_value_minor: i64
  expires_at_secs
  record_id                               // the version current at that time
}

EepEvent {
  occurred_at_secs: u64
  event:            one of the EepLogEntry events, except "eep-received" and the demand events
  role, eep_id, version, counterparty_did, currency, face_value_minor
  agreement:        "rec_…" | absent
  records:          [ "rec_…" ]
}
```

**Outstanding at a time T.** An EEP is outstanding at T, from this node's view, when:

1. it has a version with `issued_at_secs` < T (it existed);
2. the latest such version is `live` (it was not closed before T);
3. `expires_at_secs` > T (it had not expired);
4. this node was its issuer, or the holder named in that version.

The position is that latest version. Times are the signed `issued_at_secs` of the versions, so the issuer and the holder list the same EEPs for the same period. A fork gives two positions.

- The events list only the EEP events. An EEP given or returned inside an agreement is already a commitment row of the statement. The section does not repeat it, and adds the positions and the events that have no agreement (a split, an expiry).
- Demands are not in the statement. Nothing changed hands.
- If `obligation` is missing, the section stays `not-available`. If the call fails, `unavailable` with the reason, as the agreements document says.

### Left for M08H

M08H's slice H0 extends this document with the transfer flows. This document leaves these hooks, so that H0 adds causes and records, not a new format:

| M08H needs | Hook left here |
| --- | --- |
| A new holder after a transfer | Every version has `holder_did`, `version` and `prior`. A transfer is version v+1 with a new `holder_did` and a new cause `{ kind: "transferred", endorsement: "rec_…" }`. |
| "First processed wins" for two transfers of one version | The unique insert on `(eep_id, version)` and the claim rounds. A reissue takes a claim round, then creates `ver:<id>:<v+1>`, as a close does. The loser finds the row. |
| Refusal only for a double spend, a claimed version, or an expired EEP (VX-27) | The three facts are already rows: `ver:<id>:<v+1>` (double spend), a held claim round (claimed), `expires_at_secs` (expired). |
| Idempotent reissue, key `(eep_id, version, endorsement hash)` | The `req:` row pattern of the split. |
| Partial transfer: "pay ₹200 with a ₹300 EEP" to a third person | The split's closing version lists `parts` with a `holder_did` per part. M08E requires every part's holder to be the current holder. H0 lets one part name the new holder. |
| Requests from strangers, while the issuer is offline | The wire-reachable methods, the mailbox path and the deposit proof. |
| An `eep-transfer` commitment | Its kind is reserved by the agreements document. Its `eep` field is reserved here. |
| A refused EEP in a bundle | Not needed in M08E. Decided in H0 (open question 2). |

**Partial transfer is M08H's transfer, built on M08E's split.** A partial transfer to a third person is one issuer step that closes the version and makes two EEPs: one for the new holder, one back to the old holder. That is the split's write, with a new holder on one part. What makes it a transfer is the endorsement, the double-spend rules and the refusal, and those are M08H's. The same payment to the issuer itself needs neither: it is a partial redemption with a change EEP (see [Partial redemption](#partial-redemption)), which M08E builds. So M08E builds the mechanism (the split's single write with a holder per part), and M08H builds the flow that names another holder.

### Rejected options

- **A separate return endorsement record for `eep-return`.** The claim at signing time already locks the version, and the holder's agreement half already says it returns it. A second record would add a holder step that changes nothing.
- **A random EEP id, or the hash of version 1.** A random id lets a crash or a restore make two EEPs for one commitment. A hash of version 1 cannot be named before version 1 exists. A derived id solves both.
- **A closing record that is not a version.** Then the fence for "the version after v" would need a second unique row. With closing as a version, one unique insert on `(eep_id, version)` orders every change.
- **Deleting a released claim row.** Two releases that race could delete a newer claim. Claim rounds never delete, as the scheduling extension's seats do not.
- **EEP versions on the obligation channel inside an agreement.** The issuer would need the holder's obligation address, which no record of the deal carries. The conversation is already there.
- **Acknowledgements as cards.** Split parts and holdings answers come with no conversation, so acknowledgements would need two paths. One path, the obligation channel, serves every version.
- **A plain hash of the demand in the deposit, signed by the mailbox.** The mailbox cannot check it. A holder could deposit garbage with the hash of a real demand and frame the issuer. Re-sealing from a seed binds the stored bytes to the demand.
- **A full version chain in every catch-up.** The latest issuer-signed version already says who holds the EEP now. A chain adds size and a second round trip, and helps only to find a fork, which rule 5 finds without it.
- **Automatic fork settlement in M08E.** A fork needs a stale reissue to another holder, which only transfer makes possible. Settle it where it can happen.
- **Comparing the caller's DID with the record's signer, or narrowing who may carry a demand.** The verified caller names a service, not a person. Any verified caller may carry a principal's signed proof, and the receiver checks the proof.
- **A signed outstanding total record in M08E.** Nothing reads it yet. M08G's standing card is where a stranger sees it.
- **A personal message field on an EEP.** Rejected by the overview. Thanks belong in an opinion.
- **Showing a demand as a decline when it got no answer.** No answer is weaker evidence than a decline, and a reader must see the difference.
- **One shared event log for `transaction` and `obligation`.** Rejected by the agreements document: two writers, and no derive step could repair it.
- **Holder-side locks on a held version.** The issuer's claim is the one fence. A second lock on the holder's node could disagree with it.
- **Asking the person to sign version 1.** The person already agreed to the EEP when it signed the agreement. A second action would only let an issuer agree and then not issue.

## Open questions

Each has a recommendation and a reason. The owner decides.

1. **The default EEP expiry, and its limits.**
   Recommendation: default 12 months, the overview's first value. Allowed from 7 days to 5 years after issue.
   Reason: 12 months covers a season of goods or services. The lower limit equals the redemption notice, so a holder can always make one demand that the issuer must answer before the EEP ends. The upper limit keeps old promises from outliving the people's plans. The overview says EEPs "can live for years", which 5 years allows.

2. **A refused EEP in a bundle: one payment commitment per EEP, or one payment with a refund rule.**
   Recommendation: decided in H0. M08E has no bundles and no refusals, so it does not need the answer.

3. **A redemption whose delivery the holder never confirmed.** (New.)
   The overview (VX-17) says that if the issuer's delivery ends unconfirmed, the issuer reissues the EEP to the holder. A later owner decision made closing optional (VX-55). So a holder who never confirms would get both the goods and the EEP back.
   Recommendation: close the EEP when the issuer's delivery ends `ended-unconfirmed` with `claimed: true` (the issuer says it delivered). Reinstate only when the issuer made no claim. A holder who did not get the goods says so in an opinion, and the evidence shows the issuer's claim with no confirmation.
   Reason: it keeps "closing is optional" true for both sides. It matches the overview's other rule that an unconfirmed commitment with a giver's claim is not, on its own, negative evidence. The risk moves to the holder only when the issuer has signed a claim of delivery, which is evidence against it if false.

4. **Must an EEP's currency equal the agreement's currency, and its face value equal the declared value?** (New, from the agreements document.)
   Recommendation: yes, both.
   Reason: "one currency per agreement" needs it, and the statement then shows an EEP at its face value with no second number. A discount shows in the values of the other side's commitments.

5. **Drop the overview's `returned` state and the return endorsement.** (New.)
   Recommendation: yes. `eep-return` goes `promised` → `closed`, with `reinstated` as a failure end.
   Reason: the claim at signing time and the holder's agreement half already do what the endorsement would do. One record and one step fewer. M08H still has endorsements for transfers, where the issuer is not a party.

6. **The redemption notice.**
   Recommendation: 7 days, fixed, written into each demand as `answer_by_secs`.
   Reason: it equals the open mailbox's longest time-to-live (`OPEN_MAX_TTL_SECS`), so a demand that the issuer did not collect in time has also expired at the mailbox, and the evidence is clean. A dedicated mailbox may keep a message 30 days (`DEDICATED_MAX_TTL_SECS`). That is not a reason for a longer notice: a fixed notice makes evidence comparable between issuers.

7. **A "deposit first" option for demands.** (New.)
   Recommendation: yes. When the issuer has a mailbox, the holder's node deposits a demand even when a direct call would work.
   Reason: a direct delivery leaves the holder no evidence that a third party can check. The cost is a delay of at most one pull (60 seconds) and one small router option, built in E2.

8. **Issue version 1 with no person action.** (New.)
   Recommendation: yes. The issuer's node signs version 1 when the agreement forms.
   Reason: the person agreed to the EEP when it signed the agreement. A second step would only let an issuer agree and then not issue (principle 13: the least ceremony).

9. **What "has a published offering" means for VX-13.** (New.)
   Recommendation: an active listing in the issuer's own catalog, published to a directory or not.
   Reason: the issuer's node can check it alone and at once. A stranger may reach a listing by a direct link, and the node cannot know what other nodes found. M08D renames listings to intents, and the rule moves with them.

10. **Partial redemption by a change EEP, not by a split.** (New.)
    Recommendation: the Hub builds "pay ₹200 with a ₹300 EEP" to the issuer as one agreement with a return and a new ₹100 EEP. A split stays for a holder who wants smaller parts.
    Reason: one agreement, one decision on the issuer's node, no extra wire call, and no new mechanism.

11. **Two revocation reasons only: `retired` and `compromised`.** (New.)
    Recommendation: yes. A lost device is `compromised`.
    Reason: the rule has two behaviours. More reasons would be labels with no effect, and a wrong label could weaken a check.

12. **Revocation checks only in `obligation` in M08E.** (New.)
    Recommendation: the time rule goes into `signed_record` for every caller. Only `obligation` reads anchors and passes a revocation source. The other Roym services keep `EmptyRevocations` until a backlog item wires them.
    Reason: EEPs are the records that live for years. Wiring every service is a larger change, and today no other Roym record depends on it.

13. **`obligation` always deployed with a mailbox.** (New.)
    Recommendation: yes. Roym's deploy gives `obligation` a mailbox (`auto`), as it can for any public service.
    Reason: without one, a demand to an offline issuer cannot wait anywhere, VX-15 fails, and no `stored` receipt can exist.

14. **Mailbox eligibility covers all of `invoke`.** (New, from the code.)
    Recommendation: accept it for M08E. The admission table is the gate, and a deferred call to a local-only method is closed with an unsigned `rejected(call_failed)`.
    Reason: per-JSON-method eligibility would need the mailbox puller to read inside Roym's request. Nothing reaches a local-only method this way.

15. **Should the demand carry the holder's prekey bundle, so the issuer can quote while the holder is offline?** (New.)
    Recommendation: not in M08E. The issuer's quote waits in the conversation outbox until the holder's node is online once. Add a backlog row.
    Reason: first contact for an offline party is a Conversation feature that the mailbox change left out. Solving it inside `obligation` would duplicate it.

## Tasks

- [ ] Owner review of this draft and the open questions.
- [ ] E1: revocation entries with `effective_from_secs` and a reason; the time rule and the countersign rule in `signed_record`; the host function that reads an anchor's entries (OB-1 to OB-4).
- [ ] E2: the signed `stored` receipt; sealing from a seed; the deposit proof and its verifier; the "deposit first" option (OB-5, OB-6).
- [ ] E3: the `obligation` service, deployed `public` with a mailbox: the EEP record, issue, acknowledgements, expiry, `eep-issue`, the `eep` card, the wallet screen (OB-7 to OB-16, OB-31, OB-32, OB-36).
- [ ] E4: demands, declines, the claim, `eep-return`, resolving a return, partial redemption, set-off, split, the restore rule, forks, holdings requests, the per-issuer limit, the stated total, the event log and the statement part (OB-17 to OB-30, OB-33 to OB-35).
- [ ] E5: close-out, and a demo story for issue and redemption.

## Deviations

Each row is a rule of the overview or the agreements document that this design changes. Code-vs-plan differences found while building are added here too, at the moment they are found.

- **Overview, the EEP record: "id = hash of version 1".** The id is derived from the issuer and the cause of version 1 ([The EEP id](#the-eep-record)). A derived id is known before version 1 exists, and a repeat or a restore cannot make two EEPs for one commitment.
- **Overview, the EEP record: fields "provisional".** Fixed here, with three more: `prior`, `status` and `issuer_address`. `agreement` becomes part of `cause`.
- **Overview, commitment table: `eep-return` goes `promised → returned → closed`; "the holder signs the return endorsement".** No `returned` state and no return endorsement in M08E. `reinstated` is a new failure end (open question 5).
- **Overview, VX-17: "if the delivery then ends unconfirmed, the issuer reissues the EEP".** Reinstate only when the issuer made no claim of delivery (open question 3, pending the owner).
- **Overview, "the issuer closes the EEP only when its own delivery is done".** Refined: only the issuer's commitments other than `eep-return` count, so a set-off closes at once.
- **Overview, "a split is new EEPs; the original version is closed".** Kept, and the closing version lists the parts with a holder each, so that M08H's partial transfer uses the same write.
- **Overview, "No answer": proof that a `stored` receipt covers a demand.** Designed as re-sealing from a kept seed ([The mailbox side](#the-mailbox-side)). Adds a "deposit first" option, because a direct delivery leaves no third-party proof (open question 7).
- **Overview, mailbox change: the `stored` receipt signs "deposit id, destination, message hash and time".** The mailbox cannot know the message hash, which is computed from the plaintext. It signs the hash of the sealed bytes instead, plus the mailbox DID and the time-to-live.
- **Overview, revocations: "today a revoked signing key fails every record it ever signed".** True only for a caller that passes a revocation source. No Roym caller does today, so today no Roym record is checked against revocations at all. M08E wires it into `obligation` only (open question 12).
- **Overview, channel table: EEP traffic between a holder and an issuer uses the obligation methods.** EEP versions made inside an agreement travel as `eep` cards in that agreement's conversation. Everything else uses the obligation methods.
- **Overview, VX-13: "a participant with no published offering cannot issue".** "Published" means an active listing in the issuer's own catalog (open question 9).
- **Overview, "after expiry the issuer may delete its records".** No node deletes EEP records in M08E. Pruning is a backlog item.
- **Agreements, "Who reserves": the giver owns its reservation; a receiver-owned hold is not in v1.** For `eep-return` the receiver (the issuer) owns the claim. The kind fixes it, so no `reserved_by` field is added.
- **Agreements, the writer's decision, step 3, and AG-15: reserve each resource this node gives.** Also claim each `eep-return` this node receives.
- **Agreements, `Answers`: `enquiry` or `want`.** Adds `{ kind: "demand", record_id }`.
- **Agreements, open point: must an EEP's currency equal the agreement's currency?** Yes, and its face value equals its declared value (open question 4).
- **Agreements, the card table grows to twelve.** It grows to thirteen, with `eep`.

## Close-out

Before merge, update the living docs so they describe the result, then set `status: done` and fill in `living-docs-touched` and `demo`. Expected changes:

- `docs/system-requirements-spec.md`: EEPs under `[APP-ESC]`, the revocation time rule, the signed `stored` receipt.
- `docs/roym-integrated-experience-spec.md`: the wallet screen, the `eep` card, the EEP commitment kinds, demands and declines.
- `docs/system-architecture.md`: the `obligation` service, its wire-reachable methods, and its place in the Roym SynApp; the revocation entry format of the master anchor; the mailbox's signed `stored` receipt.
- `AGENTS.md`: the Roym architecture paragraph (eight services; `obligation` also holds `CallTarget::Service` and has a wire table).
- `docs/TERMINOLOGY.md`: the terms in this document.
- The service mailbox change and the overview, where [Deviations](#deviations) names them.

Record anything postponed in [deferred-backlog.md](../../deferred-backlog.md): pruning expired EEPs, revocation checks in the other Roym services, the prekey bundle in a demand, fork settlement (M08H), and anything the slices defer.
