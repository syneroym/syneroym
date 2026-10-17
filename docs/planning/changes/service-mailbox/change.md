---
status: approved
living-docs-touched:
  - docs/system-architecture.md
  - docs/system-requirements-spec.md
  - docs/developer-guide.md
  - docs/TERMINOLOGY.md
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
| M-13 | A `delivered` receipt carries the receiver's signature over the deposit id, the sender's member master DID, the message hash, the state and the time, made with its instance key, and the receiver's instance certificate. The sender knows only the receiver's member master DID, so it checks the certificate against that DID and then the signature. The mailbox stores the receipt as sent. The mailbox cannot make a `delivered` receipt. | |
| M-14 | A message that was claimed 5 times without an acknowledgement is closed with the receipt `rejected`. The mailbox counts the claims. This `rejected` receipt is unsigned. | |

Record and keys

| ID | Criterion | Test |
| --- | --- | --- |
| K-1 | `EndpointInfo` has an optional `mailbox` field: the mailbox `service_id` and the service's mailbox public key. It is covered by the existing signature. The field adds about 140 bytes. | `dht_registry/tests.rs` (`a_record_with_a_mailbox_signs_and_verifies`, `a_service_record_with_a_mailbox_fits_the_dht_size_limit`, and 3 more), `mailbox_record_e2e.rs` |
| K-2 | The deployer derives the mailbox key from the member master. The private half is delivered to each instance with its certificate. Every later deploy of the member, on any substrate, gets the same key, so a new instance can open messages sent before the move. | `keys/tests.rs` (derivation tests), `orchestration/tests/mailbox.rs` (`the_same_member_on_two_substrates_stores_the_same_key`), `mailbox_record_e2e.rs` |
| K-3 | `roymctl svc deploy` and `roymctl app deploy` accept a mailbox: a service id, or `auto`. `auto` picks one entry from the registry's open list by the same ranking helper as the coordinator list. `app deploy` takes a mailbox for each service (all members of a service share the mailbox host; each has its own key). `auto` is parsed but refused until the registry list exists. | `cli_args.rs`, `svc/tests.rs`, `app/tests/settings.rs`, `mailbox_arg/tests.rs`, `registry/tests.rs`. `auto`: open |
| K-4 | At `register` the receiver gives the mailbox its current signed service record. While the queue lives, the mailbox publishes that record to the registry every hour. The registry takes an unchanged record as a refresh. A record with an older timestamp is rejected, so a stale copy never overwrites a newer record. | |
| K-5 | A sender that reads a service record does not follow it on to the substrate record. It needs only the `mailbox` field. A deposit works after the receiver has slept longer than 2 hours (the registry lifetime of a record). | `net_iroh/tests.rs` (`resolve_peer`), `mailbox_record_e2e.rs` (`a_peer_with_a_mailbox_but_no_substrate_record_still_resolves`) |
| K-6 | A sender stores, durably, the last signed record it saw for each peer, until the `not_after` of that record. A sender with a stored record deposits when the registry cannot be reached. | `peer_records/tests.rs`, `net_iroh/tests.rs` (`an_unreachable_registry_falls_back_to_the_stored_record`), `proxy/tests/remote_dispatch.rs`, `mailbox_record_e2e.rs` (`a_stored_record_routes_a_direct_call_when_the_registry_has_none`) |
| K-7 | A private service publishes no record, so it has no mailbox field. A public record exported with `--record-out` carries the field like any other record. | `compiler/tests/member_settings.rs`, `orchestration/tests/mailbox.rs` (`validate_mailbox`), `svc/tests.rs`, `mailbox_record_e2e.rs` (`a_mailbox_on_a_service_that_is_not_public_is_refused`) |
| K-8 | The instance certificate lifetime can be set for each service (for example 7 days on a mobile substrate). Revocation stays the main control. | `compiler/tests/member_settings.rs` (range), `deploy/tests/mailbox.rs` (`member_cert_hours`), `tests/member_settings.rs` (supervisor renewal), `mailbox_record_e2e.rs` |

Sender

| ID | Criterion | Test |
| --- | --- | --- |
| S-1 | A method is mailbox-eligible only if it opts in, takes an idempotency key, returns no data, carries no forwarded caller proof, and has a sealed size of at most 64 KiB. Anything else stays on direct delivery. The default is no. Conversation `deliver` is the one exception on data: on the mailbox path its refusal reason is not returned in v1. | |
| S-2 | For a receiver with a mailbox, the sender makes one direct attempt with a short timeout. If it fails, the sender deposits the message. | |
| S-3 | After a deposit, the sender sends no more direct attempts for that message. It polls `status` on its own interval. | |
| S-4 | If the deposit fails because the mailbox is unreachable, the sender goes back to direct attempts and tries the deposit again later. | |
| S-5 | The sender keeps the payload until it has a `delivered` receipt with a valid receiver signature, or the outbox's own age limit ends (30 days for Conversation). A `rejected` receipt with a valid receiver signature fails the message (ADR-0026 A1). An `expired` or unknown receipt (also after the receipt time), an unsigned `rejected` receipt, or a receipt whose signature does not verify sends the message back to direct attempts until that age limit. Each new deposit is sealed again, with a new deposit id and send time. | |
| S-6 | The outbox shows `deposited` as a state separate from `pending` and `delivered`. The conversation delivery state, including its WIT enum, has the same new value. | |
| S-7 | The same code path serves the proxy outbox and the conversation outbox, including each member push of a group. | |
| S-8 | A receiver with no mailbox keeps today's behaviour with no change. | |
| S-9 | A `deposited` row is outside the attempt budget and the claim-count limit of its queue. It has its own poll interval and its own deadline. A deposited message is still `deposited` after the time at which the attempt budget would have run out. | |

Receiver

| ID | Criterion | Test |
| --- | --- | --- |
| R-1 | For each hosted service that has a mailbox, the substrate registers (again when needed), claims, unseals, delivers and acknowledges. It runs this at start, every 60 seconds, and when asked. If the instance certificate has expired, the substrate waits for the supervisor to renew it, then registers and pulls. A service deployed with `svc deploy` has no supervisor, so nothing renews its certificate. Its pull stays blocked, with a warning in the log and a metric every hour, until a new certificate is installed. Deposits for it still wait at the mailbox until the queue lifetime ends. Use the per-member certificate lifetime (K-8) for such a service. | |
| R-2 | The message enters the normal inbound path, including the receiver idempotency fence and admission rules. | |
| R-3 | The caller identity of a delivered message is the original sender, taken from the sealed proof. It is never the mailbox. | |
| R-4 | The receiver accepts a message only if the sender's certificate was valid at the sealed send time and is not revoked now. It rejects a send time older than the longest mailbox time-to-live plus 24 hours of clock skew (the same allowance as Conversation). A rejected message is acknowledged as `rejected`. A rejection for a send time that is too old is unsigned, because a mailbox can delay a message. A rejection for a sender certificate that does not verify is unsigned, because the certificate may be forged and then the sender is not proven. | |
| R-5 | A method that has not opted in is acknowledged as `rejected`. | |
| R-6 | A temporary local failure is not acknowledged. The mailbox closes the message after 5 claims (M-14). | |
| R-7 | For a service with a mailbox, the receiver idempotency memory lasts at least the longest mailbox time-to-live plus the clock skew allowance. The row limit for such a service is 100,000 (the default is 10,000). When it is reached first, the oldest-expiring rows go first, and R-4 bounds the age of a message. This extends the existing backlog row "A resent envelope can be refused after the receiver's dedup record expires". A message that arrives both direct and from the mailbox runs once. | |

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

`service_id` is the mailbox service: the node DID of the mailbox host, reached on interface `mailbox`. A sender finds it with the normal registry lookup. `key` is the public mailbox key of the receiving service (X25519). The deployer signs the record, so the choice is made at deploy time. Changing it means signing a new record. Size: service records carry empty `mechanisms` and stay near 400 bytes. The field adds about 140 bytes, well inside the 1000-byte BEP44 limit. (Checked in the review, `crates/sdk/src/deploy/certify.rs:164`.)

**Why the record needs help.** Only the hosting substrate publishes the record, once an hour. The registry removes it after 2 hours (`crates/community_registry/src/registry.rs`), and the DHT copy expires after 1 hour. A phone that sleeps for 3 hours has no record. A sender then gets `ServiceNotFound` and never reaches the mailbox. Two measures fix this:

- **(A) The mailbox keeps the record alive.** At `register` the receiver gives the mailbox its signed record. The mailbox publishes it every hour while the queue lives. `admit_endpoint` accepts an unchanged record as a refresh and rejects an older timestamp, so the mailbox cannot overwrite a newer record the substrate published.
- **(B) The sender keeps the last record.** Each sender stores the last signed record it saw for each peer, until its `not_after`. Today a sender keeps none (`crates/core/src/dht_registry/client.rs:195-215`).

The sender must read the service record without following it on to the substrate record, because the substrate record has expired as well. The lookup (`resolve_iroh_addr`) drops the record today. It changes to return the `mailbox` field.

**Private and internal services.** Under [ADR-0018](../../../decisions/0018-service-record-visibility.md) a private member publishes no record, and an internal one is marked private. They have no mailbox field and use direct delivery only. A mailbox needs a public record: a mailbox republishes the record, which a private or internal service must keep to itself.

### Mailbox key

The deployer derives one decrypt-only key pair for each service member from the member master: HKDF-SHA256 over the master key with the fixed `info` string `syneroym:mailbox-key:v1` ([ADR-0026](../../../decisions/0026-receiver-chosen-mailbox.md), amendment A2). Every holder of the master gets the same key, so roymctl and the supervisor need no new key storage, and every later deploy of the member, on any substrate, gets the same key. The public half goes in the record. The private half goes to the substrate in the `mailbox` field of the deploy manifest (`mailbox-credentials`), with the instance certificate. The substrate checks that it matches the record, and keeps it in the service's own encrypted vault under the reserved name `syneroym.mailbox-secret`. Guests cannot read names that start with `syneroym.`: the guest `reveal` answers `not-found`, guest SQL cannot touch the `_vault` table, and the entry name is bound into the ciphertext. The master key never reaches a hosting substrate, so a stolen host cannot sign as the member. But ADR-0020 §3 lets the supervisor hold master keys. So whoever deploys a member also holds its mailbox private key.

### Instance certificate lifetime

A renewed instance certificate lasts 4 hours (`crates/core/src/config/roles.rs`). Only the holder of the member master key can make a new one, and the supervisor pushes it. A phone that wakes after a night has an expired certificate and cannot register. Two measures: a per-member setting for a longer certificate (K-8), and, when a certificate has expired anyway, the phone waits for the supervisor to renew it and then pulls (R-1).

### Sealed message

The sender builds two parts.

- Outer, for the mailbox: destination, a random `deposit_id`, size, time-to-live. The sender is not a field. The mailbox reads it from the verified identity. The outer id is random, not the idempotency key: Conversation ids come from the content, and a mailbox could confirm a guessed message by its id.
- Inner, sealed to the mailbox key with HPKE ([RFC 9180](https://www.rfc-editor.org/rfc/rfc9180)) in base mode, which uses a fresh key for each message (the inner signature gives the authenticity): `interface`, `method`, `params`, the idempotency key, the sender's instance certificate, and a sender signature over the destination, the idempotency key, the payload hash and the send time. The signature covers the send time, which R-4 depends on.

The receiver checks the inner signature and the certificate after it opens the message (R-4). The mailbox cannot change the content without breaking the signature.

**Sealing algorithm.** HPKE base mode with `DHKEM(X25519, HKDF-SHA256)`, `HKDF-SHA256` and `AES-256-GCM`, from the `hpke` crate. The HPKE `info` is a fixed domain tag, `syneroym-mailbox-v1`. Reasons: X25519 is already in the build through `vodozemac`; the key is 32 bytes; HPKE is a standard with test vectors, so we do not write our own key derivation. Rejected: our own X25519 + HKDF + AES-GCM composition (no outside test vectors), and P-256 ECDH like the router stream handshake (that code is an interactive handshake, not a sealed box, and P-256 only helps browser senders, which a mailbox never has). *Unverified:* the `curve25519-dalek` and `rand_core` versions that `hpke` needs fit the workspace pins. If they do not, use our own composition and test it against the RFC 9180 test vectors.

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

The passport is the token from [operator-passport](../operator-passport/change.md). The operator is the controller of the mailbox's host substrate, or its node DID. The audience is the owner DID of the service. The owner re-delegates it as a child token whose audience is the **member master DID**, which stays the same when the service moves. The instance proves its link to the master with its existing service-instance `DelegationCertificate`, which the transport already checks. There is no challenge. `register` is a native call over Iroh, and the transport has already checked the instance key and its certificate (M-2, M-3). The passport core takes that verified key as the proof of possession (its transport form), and the instance certificate is the signer binding. The passport lease lasts at most 30 days, the same as the queue lifetime. The receiver renews it each time it registers, so the queue keeps its raised quota while it lives.

Ability `mailbox/use` on `substrate:<host node did>/mailbox`. Facts: `max_bytes`, `max_messages`, `max_ttl_secs`, `max_services`. The facts raise the quotas of that queue.

The owner key signs one child token for each member master at deploy time (`roymctl svc deploy` or `app deploy` with a passport file).

The `register` request carries the whole instance certificate in its body, because the router keeps only the two DIDs of the connection and does not pass the certificate on. The mailbox passes the transport's verified instance DID as the proof and this certificate as the signer binding; the passport core checks its `service-instance` scope and that it names that instance DID ([operator-passport](../operator-passport/change.md) `D-B1-17`). The mailbox reads the raised quotas of a queue from `LeaseStore::lease_for`. When a member master holds running leases under two passports, the newest registration wins (`D-B1-16`).

### Registry list

The registry config gets a second curated list, `[[roles.community_registry.mailboxes]]`, with `service_id` and `operator`. The endpoint is `GET /mailboxes`. It works like the coordinator list ([registry-listed-coordinators](../registry-listed-coordinators/change.md)), including the hop limit. The list uses the shared `list_max_age_secs` setting and the same forwarding and `502` rule. The ranking helper is shared. For `auto`, the key is `SHA-256` over the service id and the mailbox service id with the same layout as the coordinator ranking. Only open mailboxes are listed.

### Sender flow

```text
pending --direct fails--> deposit --stored--> deposited --delivered (signed)--> delivered
                              |                    |--rejected (signed)--> failed
                              |                    '--expired / unknown / unsigned rejected / bad proof--> pending (direct again, until the outbox age limit)
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

- Where is mailbox eligibility declared for a guest method: the app manifest, or the WIT interface? Find where `idempotency_key` and queued-call rules are declared. Resolve before the sender task.
- Does the substrate know it was woken (for `pull-now`) on mobile, or does the shell call it?
- Time-to-live, size and rate values above are starting points. Confirm after a first test.
- Parked rows count toward `max_pending_rows` (10 000). With a 7-day mailbox time-to-live, a sender with many sleeping receivers can fill its proxy outbox with parked rows and refuse new calls. Keep this bound, or give parked rows their own limit? (Resolve before the sender task.)
- How should the sender outbox resolve a past-deadline parked claim when a poll fails repeatedly or after expiry: should it unpark back to direct attempts, fail the row terminally, or apply the 30-day Conversation age limit on the parked path? (The async queue crate never dead-letters or unparks past-deadline parked items by itself.)
- How should worker concurrency be fenced when polling parked rows to prevent a second claim from racing with a slow poll response and clearing parked state via non-terminal fail?

## Tasks

- [ ] Resolve the open questions.
- [x] Record field, mailbox key creation and delivery, per-service certificate lifetime, `roymctl svc deploy` and `app deploy` options. (`auto` waits for the registry list.)
- [ ] Lookup change: `resolve_peer` returns the mailbox field (done). Sender stores the last record (done). Mailbox republishes the record (open).
- [x] Sealed message format with tests (tamper, wrong key, replay, old send time).
- [ ] Mailbox role, native service, store, quotas, fair eviction, sweeps, newest-certificate rule, signed receipts.
- [x] `async_queue`: a `deposited` state outside the attempt budget. Keep the supervisor behaviour.
- [ ] `ProxyRouter` deposit and status poll. New state in both outboxes. New `ConversationDeliveryState` value, the WIT enum in `conversation.wit`, and the Hub UI.
- [ ] Substrate pull worker and injection with sender identity.
- [ ] Receiver idempotency memory for services with a mailbox.
- [ ] Registry list and `GET /mailboxes`. Shared ranking helper.
- [ ] Integration tests with `common::alloc_ports`: offline receiver, receiver asleep longer than 2 hours, offline sender, relocation, duplicate delivery, expiry then direct, over quota, group push, expired certificate.
- [ ] Update living docs.
- [ ] Last: dedicated mailboxes with a passport (D-1 to D-3).

## Deviations

- `D-C1-1`: the crate calls the state "parked"; the outboxes show it as `deposited`.
- `D-C1-2`: the queue does not act at the parked deadline. It schedules the next poll no later than the deadline; the caller chooses `unpark` or `fail` (S-5 has two outcomes).
- `D-C1-3`: the deposit receipt is kept in the queue row as an opaque context blob, not in a separate table.
- `D-C1-4`: `unpark` keeps `attempts`, and there is no reset. This is what makes a proxy-outbox row that keeps getting `expired` receipts end (that outbox has no age limit).
- `D-C1-5`: `TxQueue::park` and `TxQueue::unpark` exist. C5 must set the Conversation `deposited` state and park the row in one `Queue::transaction`, because the Conversation worker deletes any row whose message is not `Pending` (`crates/conversation/src/outbox.rs:190`). Inside that closure, C5 must not call `ConversationStore::set_state`, `set_recipient_state` or `restart_pending` (`crates/conversation/src/store/message.rs:612-665`): each one locks the connection mutex that `Queue::transaction` already holds, and a `std::sync::Mutex` locked twice by one thread hangs it forever. C5 needs versions of them that write through the `&Transaction`, as `store/message.rs:73` already does with `tx.execute`. This holds for the `expired` → `unpark` path too.
- `D-C1-6`: the "Attempt budget" paragraph is partly stale. The Conversation outbox already calls `Queue::defer` for an unreachable peer (`crates/conversation/src/outbox.rs:227`), which charges no attempt and un-counts the claim. So for Conversation only the `Retry` path is bound by the attempt budget; an unreachable peer is bound by `conversation_max_pending_age_secs` (30 days). The parked state is still needed for S-3, its own poll interval and deadline, and S-6.
- `D-C1-7`: the queue does not act at the parked deadline, and parked claims never advance `claim_count`. A caller that does not resolve a row past its deadline receives it again after each visibility timeout indefinitely. C5 must resolve every past-deadline claim, and must apply the Conversation age limit on the parked path.
- `D-C2-1`: the sender signature also covers `deposit_id` and the sender's member master DID. A mailbox can then not give the receiver an existing message under another deposit id, and it cannot swap the sender certificate for one from another master (any master can issue a valid certificate for any instance key).
- `D-C2-2`: the outer part has no `size` field. The size is the length of the decoded sealed bytes.
- `D-C2-3`: the formats live in `crates/mailbox_format` (`syneroym-mailbox-format`). Mailbox keys are z-base-32 text with one accepted form (52 characters, zero spare bits), deposit ids are 32 lowercase hex characters, and sealed bytes are base64.
- `D-C2-4`: the receiver also refuses a send time more than 24 hours in the future, like Conversation. The age limit uses the queue's own time-to-live, passed in by the caller, not a fixed 7 days.
- `D-C2-5`: revocation of the sender's instance key is checked by the receiver after `open`, with the code `revoked`. It needs a network lookup, and the format crate has no I/O.
- `D-C2-6`: the mailbox secret key is the 32-byte `DeriveKeyPair` seed of RFC 9180.
- `D-C2-7`: receipts and rejections follow ADR-0026 A1. The receipt names the sender and the message hash. Only rejections for `from_future`, `revoked` and `not_eligible` are signed and final. `bad_certificate` is not: the certificate may be forged. Any other receipt is handled like `expired`. The rows M-13, M-14, S-5, R-4 and the Sender flow diagram were changed to match.
- `D-C2-8`: the `hpke` check. `hpke` 0.13 is used, not 0.14. Version 0.14 needs stable `sha2` 0.11, which conflicts with the exact pin `sha2 = "=0.11.0-rc.2"` of iroh 0.97. Version 0.13 adds no crate besides itself: it uses `x25519-dalek` 2, `aes-gcm` 0.10, `hkdf` 0.12, `sha2` 0.10 and `rand_core` 0.9, which are already in the build. This answers the "*Unverified:*" sentence in the Design section.
- `D-C2-9`: a deposit is sealed once and used once. A second deposit is a new `seal` call, with a new deposit id and send time.
- `D-C2-10`: reject codes are plain strings, an unknown receipt state reads as `unknown`, and deposits and the inner part ignore unknown fields. A newer version can then talk to this one.
- `D-C2-11`: a receipt must not be older than the message it answers. `SentDeposit` keeps the send time, and `verify_signed` refuses a receipt more than the clock skew before it (`BeforeSend`). Without this, an old receiver key with an expired certificate could still sign a receipt for a message it never delivered.
- `D-C2-12`: the workspace turns on the `float_roundtrip` feature of `serde_json`. The receiver hashes the parsed call parameters, and without this feature many floats change by one bit through serialize and parse, so the signature check fails.
- `D-C3-1`: the mailbox key is derived from the member master (HKDF-SHA256, `info` `syneroym:mailbox-key:v1`), not generated at random. K-2 needs the same key at every later deploy; the master is already stored and backed up by roymctl and the supervisor, so no new key file or backup duty is needed. HKDF is one-way, so a substrate that holds the mailbox key learns nothing about the master. Rotation means a new `info` version and a new signed record. See ADR-0026 amendment A2.
- `D-C3-2`: the deploy-call field is `deploy-manifest.mailbox: option<mailbox-credentials>`, a record with `secret-key` (z-base-32 seed). The record leaves room for the owner's child token of a dedicated mailbox. That token lives as long as the owner's passport, so it is renewed by an ordinary deploy, or by a new control-plane call; `deploy-manifest` and `renew-cert` do not change again.
- `D-C3-3`: the substrate keeps the key in the service's vault under the reserved name `syneroym.mailbox-secret`. Every vault name that starts with `syneroym.` is reserved: the WASM and native guest `reveal` answer `not-found`, and `security/set-secret` refuses it. This answers the open question about where the key is kept: the vault storage, not the guest vault interface.
- `D-C3-4`: only a `public` service may have a mailbox. The manifest compiler, `roymctl svc deploy`, `roymctl app deploy` and the substrate's deploy check all refuse the others. A mailbox republishes the record, which a private or internal record must never leave its registry for. Mailboxes for private and internal services are a backlog item.
- `D-C3-5`: the substrate checks the pair at deploy. When the record names a mailbox or a key is supplied, the record must verify (`SignedEndpointInfo::verify`), so the mailbox the receiver later pulls from is the one the master signed. The mailbox key must parse and must match the key in the record, the visibility must be `public`, and an instance certificate must be present. A record that names a mailbox without a key is refused too. The error text never contains the key.
- `D-C3-6`: the mailbox and the certificate lifetime are per service, not per member. The manifest has no per-member level: every member of a service comes from one `ServiceSpec`. The range is 1 to 720 hours, the substrate's install limit. `identity certify-instance --expires-hours` has the same range. The supervisor uses the same per-service value at renewal.
- `D-C3-7`: `resolve_peer` is new, and `resolve_iroh_addr` keeps its signature as a thin wrapper. `resolve_peer` reads the service record without following it to the substrate record. For a peer with a mailbox, the substrate lookup is bounded (`MAILBOX_PEER_ADDR_LOOKUP_TIMEOUT`, 3 s): a sleeping receiver has no substrate record, and the lookup would otherwise fall back to the DHT and wait many seconds.
- `D-C3-8`: the peer record store (`<storage.db_dir>/peer_records.db`) keeps only service records that have a mailbox. It is plaintext. A newer record without a mailbox deletes the stored one. A newer record without a mailbox keeps the row as a tombstone (the record with no mailbox), so an older record that still names the mailbox cannot bring it back. The newer-wins check and the write are one SQL statement under the connection lock, because calls to one peer are concurrent. A stored record may route a direct call when the registry lookup fails: the record is signed and inside its `not_after`, and the connection handshake still checks the receiver's certificate.
- `D-C3-9`: `MailboxChoice` is `auto` or a node DID. `auto` is parsed everywhere and refused until the registry list exists, at three places: the plan check, `svc deploy`, and `certify_placed_members`.
- `D-C3-10`: the `service_id` in a record's mailbox field is the node DID of the mailbox host. The mailbox service is on interface `mailbox` of that node, like `orchestrator` and the supervisor.
- `D-C3-11`: ADR-0026 amendment A2 records the key derivation.
- `D-C3-12`: `roymctl registry register` refuses a service whose current record in the registry names a mailbox. It signs a fresh, newer record without a mailbox, which would remove the mailbox for senders while the service still pulls from it. It has no `--mailbox` flag: it sends no key to the substrate and skips the deploy-time check.
- `D-C3-13`: "relocation" in K-2 means any later deploy of the member. The supervisor does not move a running member (`refuse_placement_change`); the only way to move one is undeploy, forget and deploy again.
- `D-C3-14`: the per-service checks (mailbox needs `public`, `auto` refused, lifetime range) run on the compiled **plan** (`validate_plan_member_settings`), not on the manifest. The supervisor receives a plan through `submit` and never sees a manifest, so a manifest check would leave that path silent. This is the same reason `validate_plan_visibility` reads the plan.
- `D-C3-15`: the vault binds the entry name into the encryption (AES-256-GCM associated data), and guest SQL cannot touch the `_vault` table (an SQLite authorizer on `execute-ddl` and `query-raw`; no collection may be named `_vault`). Without this, a guest with `data-layer/admin` (its `init` and `migrate` have it) could copy the ciphertext of a reserved entry to a name it may reveal. Vault entries written before this change do not decrypt; the product is not released, so there is no migration.
- `D-C3-16`: a deploy or undeploy without a mailbox clears a stored mailbox key only when the service already has a database. It never creates one.
- `D-C3-17`: `app deploy` refuses `mailbox` and `instance_cert_expires_hours` without `--mint-masters` for the services of every compiled plan, including the apps the manifest spawns.

## Close-out

Living docs to update:

- [system-architecture.md](../../../system-architecture.md): messaging and offline sections, roles and config, registry.
- [system-requirements-spec.md](../../../system-requirements-spec.md): offline outbox section and `PLT-ASY`.
- [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md): decision D4, "No third-party mailboxes", becomes false.
- [developer-guide.md](../../../developer-guide.md): new role and ports if any.
- [TERMINOLOGY.md](../../../TERMINOLOGY.md): add "mailbox", "queue lifetime", "sealed message".

ADR-0013 and ADR-0026 are already written. The deferred items are added to [deferred-backlog.md](../../deferred-backlog.md) with this change.
