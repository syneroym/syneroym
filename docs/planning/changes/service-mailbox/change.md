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

## Scope and non-goals

- In scope:
  - The mailbox role and its native service.
  - The mailbox fields in the service record and the sealed message format.
  - Sender side: deposit, status polling, and new outbox states, shared by the proxy outbox and the conversation outbox.
  - Receiver side: register, pull, unseal, deliver, acknowledge.
  - Quotas, rate limits and fair eviction for open mailboxes.
  - Dedicated mailboxes through a passport.
  - A curated list of open mailboxes in the community registry.
  - Conversation 1-to-1 messages and group pushes.
- Not in scope (see the [deferred backlog](../../deferred-backlog.md)):
  - First contact in Conversation with an offline receiver. It still needs one live `prekey-bundle` call.
  - More than one mailbox per service, and automatic failover.
  - Rotation of the mailbox key.
  - Push notifications that wake a sleeping phone. The first version pulls at start, on a timer and on request.
  - Large payloads by blob reference.
  - Sealed sender.

## Acceptance criteria

Mailbox service

| ID | Criterion | Test |
| --- | --- | --- |
| M-1 | A substrate with `[roles.mailbox]` runs the mailbox as a native service with these methods: `info`, `register`, `unregister`, `deposit`, `claim`, `ack`, `status`. | |
| M-2 | The mailbox takes the caller identity from the verified transport identity (ADR-0016). It never trusts an identity written in a request body. | |
| M-3 | `register` succeeds only when the caller shows an instance certificate whose master DID is the `service_id` being registered. | |
| M-4 | `deposit` is accepted from any verified sender. It is refused for an unknown destination, a message above the size limit, a full queue after fair eviction, or a sender above its rate limit. Each case has its own error. | |
| M-5 | `claim` and `ack` work only for the destination service. `status` works only for the sender that made the deposit. | |
| M-6 | A claimed message that is not acknowledged becomes visible again after the visibility time. | |
| M-7 | A message past its time-to-live is removed. Its receipt then says `expired`. | |
| M-8 | When a queue is full, the mailbox removes messages from the sender that holds the most bytes first. The new message is refused only when that sender is the depositor. | |
| M-9 | A receipt (`stored`, `delivered`, `rejected`, `expired`) is kept for a time after the body is removed. | |
| M-10 | A queue that has not been claimed for the lease time is removed. | |
| M-11 | The mailbox stores no content, interface name, method name or parameters in clear text. | |
| M-12 | A dedicated mailbox admits `register` with a valid passport and raises the quotas by the passport caveats. Without a passport it applies the open limits. | |

Record and keys

| ID | Criterion | Test |
| --- | --- | --- |
| K-1 | `EndpointInfo` has an optional `mailbox` field: the mailbox `service_id` and the service's mailbox public key. It is covered by the existing signature. | |
| K-2 | The deployer creates the mailbox key with the member master. The private half is delivered to each instance with its certificate. After a relocation the new instance can open messages sent before the move. | |
| K-3 | `roymctl` deploy accepts `--mailbox <service-id>` or `--mailbox auto`. `auto` picks one entry from the registry's open list by the same ranking helper as the coordinator list. | |
| K-4 | The record, with a mailbox field, stays inside the size limit of the registry and the DHT. | |

Sender

| ID | Criterion | Test |
| --- | --- | --- |
| S-1 | A method is mailbox-eligible only if it opts in, takes an idempotency key and returns no data. The default is no. Conversation `deliver` is eligible by default. | |
| S-2 | For a receiver with a mailbox, the sender makes one direct attempt with a short timeout. If it fails, the sender deposits the message. | |
| S-3 | After a deposit, the sender sends no more direct attempts for that message. It polls `status` on its retry tick. | |
| S-4 | If the deposit fails because the mailbox is unreachable, the sender goes back to direct attempts and tries the deposit again later. | |
| S-5 | The sender keeps the payload until the receipt is `delivered`, `rejected` or `expired`. `expired` and `rejected` are failures. An unknown receipt after the receipt time counts as `expired`. | |
| S-6 | The outbox shows `deposited` as a state separate from `pending` and `delivered`. The conversation delivery state has the same new value. | |
| S-7 | The same code path serves the proxy outbox and the conversation outbox, including each member push of a group. | |
| S-8 | A receiver with no mailbox keeps today's behaviour with no change. | |

Receiver

| ID | Criterion | Test |
| --- | --- | --- |
| R-1 | For each hosted service that has a mailbox, the substrate registers (again when needed), claims, unseals, delivers and acknowledges. It runs this at start, on a timer, and when asked. | |
| R-2 | The message enters the normal inbound path, including the receiver idempotency fence and admission rules. | |
| R-3 | The caller identity of a delivered message is the original sender, taken from the sealed proof. It is never the mailbox. | |
| R-4 | The sender's certificate is checked at delivery time. If it has expired or been revoked, the message is acknowledged as `rejected`. | |
| R-5 | A method that has not opted in is acknowledged as `rejected`. | |
| R-6 | A temporary local failure is not acknowledged. After a set number of claims the message is acknowledged as `rejected`. | |
| R-7 | The receiver idempotency memory is longer than the longest mailbox time-to-live. A message that arrives both direct and from the mailbox runs once. | |

## Design

### What the mailbox sees

It sees: the destination service, the sender service, the size, the deposit time, and the time-to-live. It does not see the interface, method, parameters or the sender's proof. It learns who talks to whom and when. The first version accepts this.

### Service record

`EndpointInfo` ([`crates/core/src/dht_registry/types.rs`](../../../../crates/core/src/dht_registry/types.rs)) gets:

```text
mailbox: Option<{ service_id, key }>
```

`service_id` is the mailbox service. A sender finds it with the normal registry lookup, so the relay design already covers reaching it. `key` is the public mailbox key of the receiving service (X25519). The deployer signs the record, so the choice is made at deploy time. Changing it means signing a new record. *Unverified:* how much room is left in the record before the BEP44 limit (1000 bytes). The field adds about 100 bytes.

### Mailbox key

The deployer creates one decrypt-only key pair for each service member, at the same time as the member master. The public half goes in the record. The private half goes to each instance in the deploy call, with the instance certificate, and is kept in the service's key store. When the supervisor relocates a member, it sends the private half to the new instance. The master key is not involved, so ADR-0020 still holds. *Unverified:* which part of the control-plane deploy call carries it, and whether the vault interface is the right place to keep it.

### Sealed message

The sender builds two parts.

- Outer, for the mailbox: destination, message id (the idempotency key), size, time-to-live. The sender is not a field. The mailbox reads it from the verified identity.
- Inner, sealed to the mailbox key (an authenticated sealed box, with a fresh key for each message): `interface`, `method`, `params`, the sender's instance certificate, a sender signature over the message id, the destination and the inner payload hash, and the send time.

The receiver checks the inner signature and the certificate chain after it opens the message. The mailbox cannot change the content without breaking the signature. The crate for the sealed box is an open item.

Conversation payloads are already encrypted end to end. We seal them again anyway. One format is simpler, and the mailbox then never sees the method name.

### Mailbox methods

The mailbox is a native service on the interface `mailbox`. It uses the existing native dispatch, so it adds no wire protocol.

| Method | Caller | What it does |
| --- | --- | --- |
| `info` | anyone | Returns limits and version. |
| `register` | destination instance | Creates the queue. Takes an optional passport. Returns limits and the lease end. |
| `unregister` | destination instance | Removes the queue. |
| `deposit` | any sender | Adds a sealed message. Returns `stored`. |
| `claim` | destination instance | Returns up to N messages and hides them for the visibility time. |
| `ack` | destination instance | Marks messages `delivered` or `rejected(code)`. Removes the body. |
| `status` | the depositing sender | Returns the receipt state for given message ids. |

### Store

A small SQLite database owned by the role, in WAL mode. Tables: `queues` (destination, limits, lease), `messages` (sequence, destination, sender, message id, size, body, deposited, expires, claimed-until), `receipts` (message id, sender, state, expires). It does not use `async_queue`. The main work is metadata queries: bytes per sender, fair eviction and expiry. `async_queue` attempt and backoff fields would not be used. Use the existing encrypted open helper if it costs little. The body is already ciphertext.

### Limits (open mailbox, first values to tune)

Message 64 KiB. Queue 5 MiB and 500 messages. Time-to-live at most 7 days. Receipt kept 14 days. Queue lease 30 days without a claim. A global storage cap and a limit on queues per owner. Deposit rate limits for each sender DID and each node.

### Dedicated mailbox and passport

The passport is the token from [operator-passport](../operator-passport/change.md). The mailbox owner is the issuer. The audience is the owner DID of the service. Ability `mailbox/use`. Caveats: `expires_at`, `max_bytes`, `max_messages`, `max_ttl_secs`, `max_services`. `register` carries the passport and the instance certificate. The core flow (challenge, proof of possession, lease, deny list) is specified once in [operator-passport](../operator-passport/change.md#core). Here the key that signs the challenge is the service instance key, and the lease sets the queue quotas. *Open:* what proof lets the mailbox link the service master to the owner DID (see open questions).

### Registry list

The registry config gets a second curated list, `[[roles.community_registry.mailboxes]]`, with `service_id` and `operator`. The endpoint is `GET /mailboxes`. It works like the coordinator list ([registry-listed-coordinators](../registry-listed-coordinators/change.md)). The ranking helper is shared. For `--mailbox auto`, the key is `hash(service_id, mailbox_service_id)`. Only open mailboxes are listed.

### Sender flow

```text
pending --direct fails--> deposit --stored--> deposited --delivered--> delivered
                              |                    |--rejected / expired--> failed
                              '--mailbox down--> pending (direct again, deposit later)
```

The hook is in `ProxyRouter`, below both outboxes. `call_peer` in Conversation and `deliver_queued` in the proxy outbox both end in `invoke_remote_at` ([`router.rs`](../../../../crates/router/src/proxy/router.rs)). A mailbox-eligible request returns `Delivered` or `Deposited(receipt)`. A second call polls the receipt. Both outboxes add the `deposited` state and poll on their tick. A group push is one outbox row for each member, so it uses the same path. Group sync pulls are queries and stay live.

### Receiver flow

A single substrate worker serves all hosted services. For each service with a mailbox it presents that service's instance certificate (the same one used for outbound calls), registers, claims, opens, and sends each message through `dispatch_json_rpc_once`. This is the entry the wire calls use, so the idempotency fence and admission apply. The caller identity comes from the sealed proof. The worker then acknowledges. A control-plane call `mailbox.pull-now` lets a mobile shell trigger a pull when the app returns to the foreground.

### Rejected options

See the alternatives in the ADR. Also rejected here: cumulative sequence windows for receipts (per-message status is simpler, and the sender needs per-message answers); implicit queue creation on first deposit (anyone could fill the disk); the substrate choosing the mailbox (it cannot sign the record).

## Open questions

- What proof lets the mailbox check that a service master DID belongs to an owner DID? A delegation from owner to member master would work. *Unverified:* that one exists today.
- Where is mailbox eligibility declared for a guest method: the app manifest, or the WIT interface? Find where `idempotency_key` and queued-call rules are declared.
- Which crate gives the sealed box? Prefer one already in the workspace.
- Can the receiver idempotency memory (`DedupConfig::ttl_ms`) be 7 days without a storage problem?
- Does the substrate know it was woken (for `pull-now`) on mobile, or does the shell call it?
- Time-to-live and size values above are starting points. Confirm after a first test.

## Tasks

- [ ] Resolve open questions. Amend ADR-0015 for `mailbox/use` if needed.
- [ ] Record field, mailbox key creation and delivery, `roymctl` flags.
- [ ] Sealed message format with tests (tamper, wrong key, replay).
- [ ] Mailbox role, native service, store, quotas, fair eviction, sweeps.
- [ ] `ProxyRouter` deposit and status poll. New outbox states in both outboxes. New `ConversationDeliveryState` value.
- [ ] Substrate pull worker and injection with sender identity.
- [ ] Registry list and `GET /mailboxes`. Shared ranking helper.
- [ ] Passport use for dedicated mailboxes.
- [ ] Integration tests with `common::alloc_ports`: offline receiver, offline sender, relocation, duplicate delivery, expiry, over quota, group push.
- [ ] Update living docs.

## Deviations

## Close-out

Living docs to update: [system-architecture.md](../../../system-architecture.md) (messaging and offline sections, roles and config, registry), [system-requirements-spec.md](../../../system-requirements-spec.md) (offline outbox section and `PLT-ASY`), [developer-guide.md](../../../developer-guide.md) (new role and ports if any). ADR-0013 and ADR-0026 are already written. Add the deferred items to [deferred-backlog.md](../../deferred-backlog.md).
