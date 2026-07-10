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

### 2. Multi-Device Sync (The Primary Writer Authority)
To avoid complex multi-master conflicts across a single user's devices:
- The Syneroym Registry maintains an immutable logical address for a user's service and designates a **Primary Substrate** (e.g., the user's Mac).
- All other substrates owned by the user (e.g., Mobile app) act as highly privileged *clients* of the Primary.
- The Primary has the final authority on applying state changes or CRDT merges from secondary devices' offline outboxes.

### 3. Asynchronous 1-to-1 P2P Messaging
To prevent the ecosystem from relying on pseudo-centralized "maildrop" servers:
- **Strict Direct Delivery:** If Alice messages Bob, the message stays in Alice's Primary outbox until Bob's Primary is online simultaneously.
- No third-party or mutual friend's substrate is used to temporarily buffer 1-to-1 messages. If Alice and Bob are never online together, the message is not delivered. This is an explicit trade-off to guarantee true decentralization and data sovereignty.

### 4. Asynchronous Group Messaging (Gossip DAG)
Group chats operate as decentralized shared logs maintained exclusively by group participants.
- **Epidemic Routing:** When a user sends a message to a group, their substrate pushes it to whichever group members are currently online.
- **Participant Relays:** Online members store the message. When an offline member comes online, they pull missed messages from any currently online peer in the group. No external storage is utilized; the group sustains its own data availability.

### 5. Deterministic Ordering via Relative Clocks
To ensure all participants in a group chat see the exact same message sequence despite asynchronous gossip paths and heavily drifting physical system clocks:
- **Per-Peer Clock Offsets:** Substrates calculate relative time offsets during Iroh handshakes (e.g., "Alice's clock is +4.2s relative to mine").
- **Adjusted Timestamps:** When a message arrives, the receiving substrate applies the known offset to the sender's raw timestamp.
- **Total Ordering:** The chat sequence is deterministically sorted globally by `(Adjusted_Timestamp, Sender_DID)`. We explicitly avoid reliance on external NTP servers to maintain offline-first robustness.

### 6. Layer 3 Primitives vs. Layer 4 Abstractions
- **Layer 3 Substrate:** Handles the core protocol (Iroh P2P routing, Relative Clock offset tracking, DAG sync algorithms).
- **MLS Integration:** Layer 3 utilizes Message Layer Security (MLS) to provide $O(\log N)$ scaling for group end-to-end encryption. The DAG guarantees *delivery ordering*, while MLS guarantees *cryptographic access*.
- **Layer 4 Application:** Chat itself is exposed as a default, lightweight "wrapper" SynApp service installed on the substrate. It consumes the Layer 3 primitives to expose structured API endpoints (e.g., `sendMessage`, `getHistory`) to user-facing frontends.

## Consequences

**Positive:**
- Complete immunity to centralized choke points or SaaS capture.
- Unbreakable message ordering even in fully air-gapped or localized mesh environments.
- Clean architectural boundary between routing (Layer 3) and application UX (Layer 4).

**Negative:**
- 1-to-1 asynchronous messages may face high latency or failure if peers have no overlapping online windows.
- Group availability relies entirely on the uptime overlap of its members.
