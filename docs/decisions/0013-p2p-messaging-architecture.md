# ADR 0013: P2P Messaging & Identity Architecture

## Status
Proposed

## Context
Syneroym requires a messaging architecture that strictly adheres to locality-first, offline-first, and data sovereignty principles. A core challenge in true P2P networks is handling asynchronous messaging and multi-device sync without falling back to centralized SaaS-like database structures or "always-on" third-party maildrops. 

We need to firmly define how Substrate (infrastructure) identity differs from Actor (User/Group) identity, and how Layer 3 messaging routes, orders, and secures messages across these identities under extreme network partitioning constraints.

## Decisions

### 1. Separation of Actor and Infrastructure Identity
We decouple "Who is acting?" from "Where is it running?".
- **Actor Identity (Master DID):** Represents the user or organization. Used for holding reputation, owning SynApps, and signing delegations. It is *not* directly used for Iroh routing.
- **Infrastructure Identity (Substrate Node DID):** Represents the specific physical device or server hosting the substrate.
- **Delegation:** A Master DID issues a cryptographic delegation to a Substrate Node DID, authorizing it to act and receive messages on its behalf. One user can authorize multiple substrates (e.g., Phone, Mac, Home Server).

### 2. Multi-Device Sync (Primary as Preferred, Not Exclusive)
To avoid complex multi-master conflicts across a single user's devices, without making a single device a hard dependency for reachability:
- The Syneroym Registry maintains an immutable logical address for a user's service and designates a **Primary Substrate** (e.g., the user's Mac) as the *preferred* target and final arbiter — not the only reachable one.
- All substrates owned by the user (e.g., Mobile app, Home Server) are directly reachable, like replicas behind a load balancer; a sender delivers to whichever is online, preferring the Primary when it is.
- The Primary has final authority on reconciling/merging state changes or CRDT merges from secondary devices' local logs and offline outboxes, including messages a secondary received directly while the Primary was unreachable.
- **Primary Promotion:** No automatic failover (consistent with `[PLT-RED]`'s CP-over-AP posture). Promoting a different substrate to Primary is a manual operator action reusing `[PLT-RED]`'s existing Registry-driven manual-promotion workflow, exposed via a `roymctl` command (e.g. `roymctl identity promote-substrate <substrate-id>`) — this ADR does not introduce a separate promotion mechanism.
- **Dependency:** this relies on the App Registry (which stores the Primary designation) preserving its own `ServiceId` across its own recovery — see Consequences and Open Questions below.

### 3. Asynchronous 1-to-1 P2P Messaging
To prevent the ecosystem from relying on pseudo-centralized "maildrop" servers:
- **Strict Direct Delivery:** If Alice messages Bob, the message stays in Alice's own outbox until *any* of Bob's known substrates is reachable (preferring Bob's Primary when several are online simultaneously), per Decision 2.
- No third-party or mutual friend's substrate is used to temporarily buffer 1-to-1 messages — only the sender's own outbox. If none of Bob's substrates are ever online while Alice holds the message, it is not delivered. This is an explicit trade-off to guarantee true decentralization and data sovereignty.

### 4. Asynchronous Group Messaging (Gossip DAG)
Group chats operate as decentralized shared logs maintained exclusively by group participants.
- **Epidemic Routing:** When a user sends a message to a group, their substrate pushes it to whichever group members are currently online.
- **Participant Relays:** Online members store the message. When an offline member comes online, they pull missed messages from any currently online peer in the group. No external storage is utilized; the group sustains its own data availability.

### 5. Deterministic Ordering via Raw Sender Timestamps
To ensure all participants in a group chat see the exact same message sequence despite asynchronous gossip paths and heavily drifting physical system clocks, without needing relative-clock negotiation or NTP:
- **Raw Sender Timestamp:** Each message carries a timestamp set by its own sender's local clock, signed as part of the message, and taken at face value by every receiver — no per-peer clock-offset correction.
- **Total Ordering:** The chat sequence is deterministically sorted globally by `(Sender_Timestamp, Sender_DID)`. Because every peer sorts on the same immutable, signed value regardless of how many relay hops the message took to arrive, ordering is consistent across all participants by construction — clock skew affects chronological *accuracy* (a message may sort earlier/later than it was "really" sent in wall-clock terms), never cross-peer ordering *consistency*. We explicitly avoid reliance on external NTP servers to maintain offline-first robustness.
- **MLS Commits Use the Same Rule:** Group membership/key-rotation Commits are ordinary DAG entries, ordered by the identical rule above rather than a separate mechanism. A Commit that loses the race against a concurrent Commit is rejected per MLS's standard concurrent-commit handling; its sender observes the rejection via gossip and re-proposes against the new epoch. Members who haven't yet received the winning Commit remain on the prior epoch (unable to decrypt new messages) until it propagates — an expected, transient convergence window consistent with this architecture's offline-first, eventually-consistent posture elsewhere.

### 6. Layer 3 Primitives vs. Layer 4 Abstractions
- **Layer 3 Substrate:** Handles the core protocol (Iroh P2P routing, sender-timestamp DAG ordering, DAG sync algorithms).
- **MLS Integration:** Layer 3 utilizes Message Layer Security (MLS) to provide $O(\log N)$ scaling for group end-to-end encryption. The DAG guarantees *delivery ordering*, while MLS guarantees *cryptographic access*.
- **Not Built on `syneroym:messaging` Pub/Sub:** Durable message content and history are delivered and ordered entirely by the mechanism above — direct exchange or participant-relay gossip — and never depend on the `syneroym:messaging` MQTT-style broker ([ADR-0010](0010-mqtt-broker-rumqttd.md)). That broker MAY optionally be used as a side-channel for purely ephemeral, best-effort UX signals with no durability requirement (e.g. typing indicators, or a "new message arrived" nudge to an already-online peer) — matching the requirements-spec's Substrate Feature Coverage Matrix entry for `[PLT-DAT]` Pub/Sub — but it is never load-bearing for message delivery, ordering, or history.
- **Layer 4 Application:** Chat itself is exposed as a default, lightweight "wrapper" SynApp service installed on the substrate. It consumes the Layer 3 primitives to expose structured API endpoints (e.g., `sendMessage`, `getHistory`) to user-facing frontends.

## Consequences

**Positive:**
- Complete immunity to centralized choke points or SaaS capture.
- Unbreakable message ordering even in fully air-gapped or localized mesh environments.
- Clean architectural boundary between routing (Layer 3) and application UX (Layer 4).
- Multi-device delivery no longer depends on one specific device (Primary) being online — any authorized substrate can receive directly, while the Primary still owns final reconciliation.
- No clock-synchronization infrastructure (NTP, per-peer handshake offset exchange) is required for consistent ordering.

**Negative:**
- 1-to-1 asynchronous messages may face high latency or failure if *none* of the recipient's substrates have an overlapping online window with the sender.
- Group availability relies entirely on the uptime overlap of its members.
- Chronological display order can be inaccurate (though still globally consistent across peers) if a sender's device clock is significantly skewed, since raw timestamps are trusted without correction.
- Primary Substrate promotion is manual only, per `[PLT-RED]`'s CP-over-AP posture — requires explicit operator/user action and a brief availability gap while a new Primary takes over.
- This design assumes the App Registry preserves its own `ServiceId` across recovery; see Open Questions.

## Open Questions
- **App Registry identity continuity:** This ADR assumes the App Registry (which stores the Primary Substrate designation) always recovers under the same `ServiceId`/key, in which case Iroh's key-based routing plus standard reactive-retry already handles relocation to a new physical node with no new mechanism needed. If that invariant is ever broken (the App Registry must come back under a *new* identity), propagating the new identity to already-running services is a general `[TOP-REG]` problem, not solved by this ADR.
- **Raw-timestamp plausibility bounds:** Trusting a sender's raw timestamp (Decision 5) is trivially gameable (a sender can misreport it to jump the ordering queue). Whether to add a coarse sanity bound — e.g. rejecting/quarantining messages whose timestamp is implausibly far from the receiver's own clock — is left open for a follow-up decision.
- **Primary loss/unavailability beyond promotion mechanics:** Decision 2 covers *how* a promotion is invoked (`roymctl`, reusing `[PLT-RED]`), but not the operational runbook for detecting a lost Primary or the exact CLI/UX surface — left for the implementing milestone.
