---
status: draft
living-docs-touched:
---
# Service mailbox

*Reader: developers and operators.*

Decision record: [ADR-0026](../../../decisions/0026-receiver-chosen-mailbox.md). Read it first. This document gives the requirements and the design.

## Problem

A call to a service succeeds only when the sender and the receiver are online together. Today the sender keeps the message in its own durable outbox and retries. This works when the receiver comes back while the sender is still running.

It fails for a mobile receiver. The phone sleeps most of the time. The sender may also sleep when the phone wakes. The two never overlap, and the message is not delivered.

This change adds a **mailbox**: a service that holds sealed messages for another service for a limited time. A sender that cannot reach the receiver deposits the message there. A receiver that wakes up collects it.

## Terms

- **Member master:** the stable DID of one member of a service ([ADR-0020](../../../decisions/0020-stable-logical-service-identity.md)). The deployer holds its private key.
- **Instance key and instance certificate:** the key a hosting substrate makes for one running copy of a service, and the certificate by which the member master allows it. The supervisor renews the certificate (4 hours by default).
- **Service record:** the signed `EndpointInfo` that a registry holds for a service.
- **BEP44:** the mutable-item format of the Mainline DHT. A value is at most 1000 bytes.
- **Visibility time:** how long a claimed message stays hidden from other claims before it can be claimed again.
- **Queue lifetime:** how long a queue lives at the mailbox without a claim. (A *lease* in this change set always means a passport lease, see [operator-passport](../operator-passport/change.md).)

## Scope and non-goals

- In scope:
  - The mailbox role and its native service.
  - The mailbox fields in the service record, and keeping that record alive while the receiver sleeps.
  - The sealed message format.
  - Sender side: deposit, status polling, and new outbox states, shared by the proxy outbox and the conversation outbox.
  - Receiver side: register, pull, unseal, deliver, acknowledge.
  - Quotas, rate limits and fair eviction for open mailboxes.
  - A curated list of open mailboxes in the community registry.
  - Conversation 1-to-1 messages and group pushes.
  - A per-member setting for a longer instance certificate on mobile substrates.
  - Dedicated mailboxes through a passport. This is the last task and has its own criteria group.
- Not in scope (see the [deferred backlog](../../deferred-backlog.md)):
  - First contact in Conversation with an offline receiver. It still needs one live `prekey-bundle` call.
  - The refusal reason of Conversation on the mailbox path.
  - More than one mailbox per service, and automatic failover.
  - Rotation of the mailbox key.
  - Push notifications that wake a sleeping phone. The first version pulls at start, on a timer and on request.
  - Large payloads by blob reference.
  - Sealed sender.
  - A mailbox for a private service (see Design).

## Acceptance criteria

Mailbox service

| ID | Criterion | Test |
| --- | --- | --- |
| M-1 | A substrate with `[roles.mailbox]` runs the mailbox as a native service with these methods: `info`, `register`, `unregister`, `deposit`, `claim`, `ack`, `status`. | |
| M-2 | The mailbox takes the caller identity from the verified transport identity (ADR-0016). It never trusts an identity written in a request body. | |
| M-3 | `register` succeeds only when the caller shows an instance certificate whose master DID is the `service_id` being registered. | |
| M-4 | `deposit` is accepted from any verified sender. It is refused for an unknown destination, a message above the size limit, a full queue after fair eviction, or a sender above its rate limit. Each case has its own error. | |
| M-5 | `claim` and `ack` work only for the destination service. `status` works only for the sender that made the deposit. | |
| M-6 | A claimed message that is not acknowledged becomes visible again after the visibility time (5 minutes). | |
| M-7 | A message past its time-to-live is removed. Its receipt then says `expired`. | |
| M-8 | When a queue is full, the mailbox removes messages from the sender that holds the most bytes first. The new message is refused only when that sender is the depositor. | |
| M-9 | A receipt (`stored`, `delivered`, `rejected`, `expired`) is kept for 14 days after the body is removed. | |
| M-10 | A queue that has not been claimed for the queue lifetime (30 days) is removed. | |
| M-11 | The mailbox stores no content, interface name, method name or parameters in clear text. | |
| M-12 | For each queue the mailbox serves `claim` and `ack` only to the instance with the newest certificate it has seen (by `issued_at_secs`), and only if that certificate is not revoked. After a relocation, the old instance can no longer take messages. | |
| M-13 | A `delivered` receipt carries the receiver's signature (message id, state, time) made with its instance key. The mailbox stores it as sent. The mailbox cannot make a `delivered` receipt. | |
| M-14 | A message that was claimed 5 times without an acknowledgement is closed with the receipt `rejected`. The mailbox counts the claims. | |

Record and keys

| ID | Criterion | Test |
| --- | --- | --- |
| K-1 | `EndpointInfo` has an optional `mailbox` field: the mailbox `service_id` and the service's mailbox public key. It is covered by the existing signature. The field adds about 140 bytes. | |
| K-2 | The deployer creates the mailbox key with the member master. The private half is delivered to each instance with its certificate. After a relocation the new instance can open messages sent before the move. | |
| K-3 | `roymctl svc deploy` and `roymctl app deploy` accept a mailbox: a service id, or `auto`. `auto` picks one entry from the registry's open list by the same ranking helper as the coordinator list. `app deploy` takes a mailbox for each member. | |
| K-4 | At `register` the receiver gives the mailbox its current signed service record. While the queue lives, the mailbox publishes that record to the registry every hour. The registry takes an unchanged record as a refresh. A record with an older timestamp is rejected, so a stale copy never overwrites a newer record. | |
| K-5 | A sender that reads a service record does not follow it on to the substrate record. It needs only the `mailbox` field. A deposit works after the receiver has slept longer than 2 hours (the registry lifetime of a record). | |
| K-6 | A sender stores, durably, the last signed record it saw for each peer, until the `not_after` of that record. A sender with a stored record deposits when the registry cannot be reached. | |
| K-7 | A private service publishes no record, so it has no mailbox field. A record exported with `--record-out` carries the field like any other record. | |
| K-8 | The instance certificate lifetime can be set for each member (for example 7 days on a mobile substrate). Revocation stays the main control. | |

Sender

| ID | Criterion | Test |
| --- | --- | --- |
| S-1 | A method is mailbox-eligible only if it opts in, takes an idempotency key, returns no data, carries no forwarded caller proof, and has a sealed size of at most 64 KiB. Anything else stays on direct delivery. The default is no. Conversation `deliver` is the one exception on data: on the mailbox path its refusal reason is not returned in v1. | |
| S-2 | For a receiver with a mailbox, the sender makes one direct attempt with a short timeout. If it fails, the sender deposits the message. | |
| S-3 | After a deposit, the sender sends no more direct attempts for that message. It polls `status` on its own interval. | |
| S-4 | If the deposit fails because the mailbox is unreachable, the sender goes back to direct attempts and tries the deposit again later. | |
| S-5 | The sender keeps the payload until it has a `delivered` receipt with a valid receiver signature, or the outbox's own age limit ends (30 days for Conversation). A `rejected` receipt fails the message. An `expired` or unknown receipt (also after the receipt time) sends the message back to direct attempts until that age limit. | |
| S-6 | The outbox shows `deposited` as a state separate from `pending` and `delivered`. The conversation delivery state, including its WIT enum, has the same new value. | |
| S-7 | The same code path serves the proxy outbox and the conversation outbox, including each member push of a group. | |
| S-8 | A receiver with no mailbox keeps today's behaviour with no change. | |
| S-9 | A `deposited` row is outside the attempt budget and the claim-count limit of its queue. It has its own poll interval and its own deadline. A deposited message is still `deposited` after the time at which the attempt budget would have run out. | |

Receiver

| ID | Criterion | Test |
| --- | --- | --- |
| R-1 | For each hosted service that has a mailbox, the substrate registers (again when needed), claims, unseals, delivers and acknowledges. It runs this at start, every 60 seconds, and when asked. If the instance certificate has expired, the substrate waits for the supervisor to renew it, then registers and pulls. | |
| R-2 | The message enters the normal inbound path, including the receiver idempotency fence and admission rules. | |
| R-3 | The caller identity of a delivered message is the original sender, taken from the sealed proof. It is never the mailbox. | |
| R-4 | The receiver accepts a message only if the sender's certificate was valid at the sealed send time and is not revoked now. It rejects a send time older than the longest mailbox time-to-live plus 24 hours of clock skew (the same allowance as Conversation). A rejected message is acknowledged as `rejected`. | |
| R-5 | A method that has not opted in is acknowledged as `rejected`. | |
| R-6 | A temporary local failure is not acknowledged. The mailbox closes the message after 5 claims (M-14). | |
| R-7 | For a service with a mailbox, the receiver idempotency memory lasts at least the longest mailbox time-to-live plus the clock skew allowance. When the row limit is reached first, the oldest-expiring rows go first (R-4 bounds the age of a message, so the limit is raised for such services). A message that arrives both direct and from the mailbox runs once. | |

Dedicated mailbox (last task)

| ID | Criterion | Test |
| --- | --- | --- |
| D-1 | A dedicated mailbox admits `register` with a valid passport and raises the quotas by the facts in the passport. Without a passport it applies the open limits. | |
| D-2 | The passport audience is the owner DID. The owner's child token names the member master. The instance proves its link to the master with its existing service-instance certificate. | |
| D-3 | `max_services` counts distinct member masters for one passport. | |

## Design

### What the mailbox sees

It sees: the destination service, the sender service, the size, the deposit time, and the time-to-live. It does not see the interface, method, parameters or the sender's proof. It learns who talks to whom and when. The first version accepts this.

### Service record and its life

`EndpointInfo` ([`crates/core/src/dht_registry/types.rs`](../../../../crates/core/src/dht_registry/types.rs)) gets:

```text
mailbox: Option<{ service_id, key }>
```

`service_id` is the mailbox service. A sender finds it with the normal registry lookup. `key` is the public mailbox key of the receiving service (X25519). The deployer signs the record, so the choice is made at deploy time. Changing it means signing a new record. Size: service records carry empty `mechanisms` and stay near 400 bytes. The field adds about 140 bytes, well inside the 1000-byte BEP44 limit. (Checked in the review, `crates/sdk/src/deploy/certify.rs:164`.)

**Why the record needs help.** Only the hosting substrate publishes the record, once an hour. The registry removes it after 2 hours (`crates/community_registry/src/registry.rs`), and the DHT copy expires after 1 hour. A phone that sleeps for 3 hours has no record. A sender then gets `ServiceNotFound` and never reaches the mailbox. Two measures fix this:

- **(A) The mailbox keeps the record alive.** At `register` the receiver gives the mailbox its signed record. The mailbox publishes it every hour while the queue lives. `admit_endpoint` accepts an unchanged record as a refresh and rejects an older timestamp, so the mailbox cannot overwrite a newer record the substrate published.
- **(B) The sender keeps the last record.** Each sender stores the last signed record it saw for each peer, until its `not_after`. Today a sender keeps none (`crates/core/src/dht_registry/client.rs:195-215`).

The sender must read the service record without following it on to the substrate record, because the substrate record has expired as well. The lookup (`resolve_iroh_addr`) drops the record today. It changes to return the `mailbox` field.

**Private and internal services.** Under [ADR-0018](../../../decisions/0018-service-record-visibility.md) a private member publishes no record, and an internal one is marked private. They have no mailbox field and use direct delivery only. A record exported with `--record-out` carries the field like any other record.

### Mailbox key

The deployer creates one decrypt-only key pair for each service member, at the same time as the member master. The public half goes in the record. The private half goes to each instance in the deploy call, with the instance certificate, and is kept in the service's key store. When the supervisor relocates a member, it sends the private half to the new instance. The master key never reaches a hosting substrate, so a stolen host cannot sign as the member. But ADR-0020 §3 lets the supervisor hold master keys for unattended relocation. So whoever relocates a member also holds its mailbox private key. *Unverified:* which part of the control-plane deploy call carries the private half, and whether the vault interface is the right place to keep it.

### Instance certificate lifetime

A renewed instance certificate lasts 4 hours (`crates/core/src/config/roles.rs`). Only the holder of the member master key can make a new one, and the supervisor pushes it. A phone that wakes after a night has an expired certificate and cannot register. Two measures: a per-member setting for a longer certificate (K-8), and, when a certificate has expired anyway, the phone waits for the supervisor to renew it and then pulls (R-1).

### Sealed message

The sender builds two parts.

- Outer, for the mailbox: destination, a random `deposit_id`, size, time-to-live. The sender is not a field. The mailbox reads it from the verified identity. The outer id is random, not the idempotency key: Conversation ids come from the content, and a mailbox could confirm a guessed message by its id.
- Inner, sealed to the mailbox key with an anonymous sealed box (a fresh key for each message; the inner signature gives the authenticity): `interface`, `method`, `params`, the idempotency key, the sender's instance certificate, a sender signature over the destination, the idempotency key and the payload hash, and the send time.

The receiver checks the inner signature and the certificate after it opens the message (R-4). The mailbox cannot change the content without breaking the signature. The crate for the sealed box is an open item.

Conversation payloads are already encrypted end to end. We seal them again anyway. One format is simpler, and the mailbox then never sees the method name.

**Forwarded proofs.** The proxy outbox forwards the original caller's proof for a substrate-internal call (ADR-0016 §6). A sealed message carries the certificate of the sending service, so the receiver would authorize a different caller. A call with a forwarded proof is therefore not eligible (S-1).

### Mailbox methods

The mailbox is a native service on the interface `mailbox`. It uses the existing native dispatch, so it adds no wire protocol.

| Method | Caller | What it does |
| --- | --- | --- |
| `info` | anyone | Returns limits and version. |
| `register` | destination instance | Creates the queue and stores the signed record (K-4). Takes an optional passport. Returns limits and the queue lifetime end. |
| `unregister` | destination instance | Removes the queue. |
| `deposit` | any sender | Adds a sealed message. Returns `stored`. |
| `claim` | newest destination instance | Returns up to N messages and hides them for the visibility time. |
| `ack` | newest destination instance | Marks messages `delivered` (with the receiver signature) or `rejected(code)`. Removes the body. |
| `status` | the depositing sender | Returns the receipt state for given deposit ids. |

### Store

A small SQLite database owned by the role, in WAL mode. Tables: `queues` (destination, limits, queue lifetime, newest certificate seen, signed record), `messages` (sequence, destination, sender, deposit id, size, body, deposited, expires, claimed-until, claim count), `receipts` (deposit id, sender, state, signature, expires). It does not use `async_queue`. The main work is metadata queries: bytes per sender, fair eviction and expiry. `async_queue` attempt and backoff fields would not be used. Use the existing encrypted open helper if it costs little. The body is already ciphertext.

### Limits (open mailbox, first values to tune)

Message 64 KiB. Queue 5 MiB and 500 messages. Time-to-live at most 7 days. Receipt kept 14 days. Visibility time 5 minutes. Queue lifetime 30 days without a claim. A global storage cap. Deposit rate limits for each node.

An identity costs nothing to make, so a per-sender share is only a weak limit. The real limits of an open mailbox are the global storage cap, the per-node rate limits and the per-queue cap. A limit on queues per owner cannot be enforced, because the mailbox cannot prove who the owner is. It exists only for passport queues (`max_services`).

### Dedicated mailbox and passport

The passport is the token from [operator-passport](../operator-passport/change.md). The operator is the controller of the mailbox's host substrate, or its node DID. The audience is the owner DID of the service. The owner re-delegates it as a child token whose audience is the **member master DID**, which stays the same when the service moves. The instance proves its link to the master with its existing service-instance `DelegationCertificate`, which the transport already checks. The key that signs the challenge is the instance key.

Ability `mailbox/use` on `substrate:<operator did>/mailbox`. Facts: `max_bytes`, `max_messages`, `max_ttl_secs`, `max_services`. The facts raise the quotas of that queue.

The owner key signs one child token for each member master at deploy time (`roymctl svc deploy` or `app deploy` with a passport file).

### Registry list

The registry config gets a second curated list, `[[roles.community_registry.mailboxes]]`, with `service_id` and `operator`. The endpoint is `GET /mailboxes`. It works like the coordinator list ([registry-listed-coordinators](../registry-listed-coordinators/change.md)), including the hop limit. The ranking helper is shared. For `auto`, the key is `SHA-256` over the service id and the mailbox service id with the same layout as the coordinator ranking. Only open mailboxes are listed.

### Sender flow

```text
pending --direct fails--> deposit --stored--> deposited --delivered (signed)--> delivered
                              |                    |--rejected--> failed
                              |                    '--expired / unknown--> pending (direct again, until the outbox age limit)
                              '--mailbox down--> pending (direct again, deposit later)
```

The hook is in `ProxyRouter`, below both outboxes. `call_peer` in Conversation and `deliver_queued` in the proxy outbox both end in `invoke_remote_at` ([`router.rs`](../../../../crates/router/src/proxy/router.rs)), through `invoke_inner`, which tries a local service first. A mailbox-eligible request returns `Delivered` or `Deposited(receipt)`. A second call polls the receipt. Both outboxes add the `deposited` state and poll on their own interval. A group push is one outbox row for each member, so it uses the same path. Group sync pulls are queries and stay live.

**Attempt budget.** A Conversation item fails when `claim_count > max_attempts` (`crates/conversation/src/outbox.rs:173`), and the proxy outbox dead-letters after 54 attempts, about 10 hours. The mailbox time-to-live is 7 days. If a deposited message were polled on every tick, it would fail long before its time-to-live. So a deposited row needs its own state in the shared `async_queue` crate, outside the attempt budget (S-9). The supervisor also uses `async_queue`, so this change must keep its behaviour.

**Conversation 30 days.** A Conversation message stays pending for up to 30 days (`conversation_max_pending_age_secs`). A mailbox with a 7-day time-to-live must not shorten that. After `expired` the sender returns to direct attempts (S-5). The receiver idempotency memory covers a late duplicate (R-7).

### Receiver flow

A single substrate worker serves all hosted services. For each service with a mailbox it presents that service's instance certificate (the same one used for outbound calls), registers, claims, opens, and sends each message through `dispatch_json_rpc_once`. This is the entry the wire calls use, so the idempotency fence and admission apply. The caller identity comes from the sealed proof. The worker then acknowledges: it signs a `delivered` ack with the instance key. A control-plane call `mailbox.pull-now` lets a mobile shell trigger a pull when the app returns to the foreground.

**Conversation refusal.** `deliver` returns a `DeliveryAck` with an optional refusal reason, and the sender records it (`crates/conversation/src/transport.rs:241-255`). On the mailbox path this reason is not returned in v1. The receipt passes through the mailbox, and a stranger would learn "B refused A". The receiver still records the refusal locally. The sender sees `delivered` with no refusal note. This is allowed, and a deferred backlog row records it.

### Rejected options

See the alternatives in the ADR. Also rejected here: cumulative sequence windows for receipts (per-message status is simpler, and the sender needs per-message answers); implicit queue creation on first deposit (anyone could fill the disk); the substrate choosing the mailbox (it cannot sign the record); checking the sender certificate at delivery time only (a message that waits 6 hours would carry an expired certificate and be rejected).

**Residual window of R-4.** The sender sets the send time. A holder of an expired certificate could set a send time inside the old validity window. The exposure is bounded: the send time may be at most the longest time-to-live plus 24 hours old, and revocation still stops a compromised key. We accept this.

## Open questions

- Which part of the control-plane deploy call carries the mailbox private key, and is the vault the right place to keep it?
- Where is mailbox eligibility declared for a guest method: the app manifest, or the WIT interface? Find where `idempotency_key` and queued-call rules are declared.
- Which crate gives the sealed box? Prefer one already in the workspace.
- Does the substrate know it was woken (for `pull-now`) on mobile, or does the shell call it?
- Time-to-live, size and rate values above are starting points. Confirm after a first test.

## Tasks

- [ ] Resolve the open questions.
- [ ] Record field, mailbox key creation and delivery, per-member certificate lifetime, `roymctl svc deploy` and `app deploy` options.
- [ ] Lookup change: `resolve_iroh_addr` returns the mailbox field. Sender stores the last record. Mailbox republishes the record.
- [ ] Sealed message format with tests (tamper, wrong key, replay, old send time).
- [ ] Mailbox role, native service, store, quotas, fair eviction, sweeps, newest-certificate rule, signed receipts.
- [ ] `async_queue`: a `deposited` state outside the attempt budget. Keep the supervisor behaviour.
- [ ] `ProxyRouter` deposit and status poll. New state in both outboxes. New `ConversationDeliveryState` value, the WIT enum in `conversation.wit`, and the Hub UI.
- [ ] Substrate pull worker and injection with sender identity.
- [ ] Receiver idempotency memory for services with a mailbox.
- [ ] Registry list and `GET /mailboxes`. Shared ranking helper.
- [ ] Integration tests with `common::alloc_ports`: offline receiver, receiver asleep longer than 2 hours, offline sender, relocation, duplicate delivery, expiry then direct, over quota, group push, expired certificate.
- [ ] Update living docs.
- [ ] Last: dedicated mailboxes with a passport (D-1 to D-3).

## Deviations

## Close-out

Living docs to update:

- [system-architecture.md](../../../system-architecture.md): messaging and offline sections, roles and config, registry.
- [system-requirements-spec.md](../../../system-requirements-spec.md): offline outbox section and `PLT-ASY`.
- [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md): decision D4, "No third-party mailboxes", becomes false.
- [developer-guide.md](../../../developer-guide.md): new role and ports if any.
- [TERMINOLOGY.md](../../../TERMINOLOGY.md): add "mailbox", "queue lifetime", "sealed message".

ADR-0013 and ADR-0026 are already written. The deferred items are added to [deferred-backlog.md](../../deferred-backlog.md) with this change.
