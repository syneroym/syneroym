# ADR 0026: Receiver-chosen mailbox for deferred delivery

## Status
Proposed

## Related
- ADR-0013 (Decisions 3 and 4 amended by this ADR)
- ADR-0015 (UCAN tokens, used for dedicated mailboxes)
- ADR-0016 (caller identity on native calls)
- ADR-0020 (member master DID, instance key and certificate)
- ADR-0023 (outbox and receiver-side idempotency fence)

## Context
A message to a service can only be delivered when the sender and the receiver are online at the same time. ADR-0013 accepted this on purpose. It said that only the sender's own outbox may hold a 1-to-1 message.

This fails for mobile services. A phone sleeps for long periods. The sender may also sleep when the phone wakes. The sender's retries keep failing and the message is not delivered.

## Decision
1. **Opt-in, chosen by the receiver.** A service may name one mailbox in its signed service record. The deployer sets it at deploy time. A service with no mailbox works as before. A private service publishes no record, so it has no mailbox. The record must stay findable while the receiver sleeps: the mailbox publishes the signed record on the registry every hour while the queue lives, and each sender keeps the last record it saw for each peer.
2. **A mailbox is a service.** It runs as a native service under a substrate role. It does not need a public address. It is reached like any other service.
3. **The mailbox sees metadata only.** It sees the destination service, the sender service (from the verified caller identity), the size and the times. The interface, method, parameters and the sender's proof are sealed. Only the receiver can open them.
4. **A separate mailbox key.** The deployer creates a decrypt-only key for each service member. The public half is in the signed record. The private half goes to each instance with its certificate. The master key never reaches a hosting substrate (ADR-0020). Messages stay readable after the service moves to another substrate. ADR-0020 §3 lets the supervisor hold master keys for unattended relocation, so whoever relocates a member also holds its mailbox private key.
5. **Fire-and-forget only.** A method must declare that it accepts deferred delivery. It must also carry an idempotency key and return no data.
6. **Sender behaviour.** The sender makes one direct attempt. If it fails, the sender deposits the message and stops sending directly. It keeps the payload until the mailbox reports `delivered` with the receiver's signature, or the outbox's own age limit ends. `rejected` fails the message. `expired` or an unknown receipt sends the message back to direct attempts until that age limit.
7. **Receiver behaviour.** The substrate pulls for each hosted service with that service's instance certificate. It passes each message through the normal inbound path, including the idempotency fence and admission rules. It accepts a message only if the sender's certificate was valid at the sealed send time and is not revoked now. Then it acknowledges with a signature, so a mailbox cannot invent a delivery.
8. **Who may run a mailbox.** Open mailboxes are listed by a community registry operator. They have small quotas and rate limits. A dedicated mailbox admits a service owner who has a passport (a UCAN token issued by the mailbox owner).
9. **No sealed sender.** The mailbox knows the sender. The transport shows it anyway. A signed, known sender lets the mailbox rate-limit and block abuse.

## Consequences
- A sleeping receiver gets messages that arrived while it was away, as long as the mailbox is up and the message has not expired.
- The mailbox learns who talks to whom, how much, and when. We accept this in the first version.
- The sender sees a new state between pending and delivered: `deposited`. A UI must not show it as final success.
- On the mailbox path, the refusal reason of Conversation is not returned in the first version. The receipt passes through the mailbox, and a stranger would learn who refused whom.
- Only calls with no forwarded caller proof are eligible.
- A message can reach the receiver twice, once direct and once from the mailbox. The existing idempotency fence removes the duplicate. Its memory time must be longer than the longest mailbox time-to-live.
- Order is not guaranteed across the direct and mailbox paths. Services must not depend on it. Conversation already sorts by timestamp.
- A mailbox is chosen at deploy time. Changing it means signing a new record. There is no automatic failover.
- A service that first meets a peer through Conversation still needs one live exchange to fetch the prekey bundle.

## Alternatives considered
- **Sender outbox only** (the earlier decision). It fails when the two sides never overlap.
- **Mailbox sees content.** It is simpler and cannot be accepted: an open mailbox is run by a stranger.
- **Seal to the instance key.** Messages waiting at the mailbox become unreadable when the service moves.
- **Seal to the master key.** The substrate does not hold it, so it could never open the message.
- **Sealed sender.** It adds cost and gives little: the mailbox sees the connection anyway.
