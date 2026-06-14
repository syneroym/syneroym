# Dynamic Ledger Network (DLN) Specification

## 1. Product Vision
An unstoppable, peer-to-peer state engine that digitizes organic community trust networks via a decentralized, tag-based, self-clearing cryptographic ledger. While its primary application is a cash-free financial network enabling individuals and businesses to trade, settle debts, and optimize liquidity without central banks, the underlying engine is highly generalized. It serves as a foundational primitive for any mutually signed state transition, including non-financial contract agreements and cryptographic delegations of authority.

## 2. Core Architectural Pillars

### Decentralized Local Logs (cr-sqlite)
There is no centralized database, global consensus network, or public blockchain token. Every user's device maintains a private, append-only, tamper-proof cryptographic log containing only the transactions they are directly involved in. These logs are stored locally using CRDT-backed databases (`cr-sqlite`).

### Mutual Cryptographic Consensus
A ledger entry is legally and mathematically valid within the network only when it carries the valid digital private-key signatures of both participating parties.

### Trust-Based Deterrence & Reputation Enforcement
The network does not attempt to mathematically prevent "double-spending" or track absolute systemic sum-totals via complex global consensus. Instead, it relies on conscious social trust backed by cryptographic deterrence. If a user defaults on a debt or maliciously double-promises their commitments, the aggrieved party submits the mutually-signed ledger entry as irrefutable proof to the broader Trust & Reputation network. The defaulting user faces immediate social and economic exclusion, ensuring the cost of reputation destruction far outweighs the short-term gain of defaulting.

### 100% Offline-First Transacting & Seamless Sync
Because enforcement relies on deterrence rather than global consensus, transacting parties do not need to be connected to the internet or a broader network. A transaction only requires the two parties to be locally connected (e.g., via Bluetooth or Local Wi-Fi) to exchange cryptographic signatures. The `cr-sqlite` CRDT engine handles these offline receipts effortlessly, ensuring seamless multi-device sync (e.g., merging a user's mobile phone and desktop ledgers) and automatic replication to backup substrates once internet connectivity is restored.

### Flat, Tag-Based Metadata Engine
Transactions are stored in a single flat ledger database on the device. Relationships, constraints, and business rules are applied dynamically via flexible metadata tags (e.g., `#Family`, `#Business`, `#Net30`, `#Kirana`), completely replacing rigid folder or account hierarchies. Hierarchies are treated strictly as user-interface visualization filters.

### A Fundamental State Engine (Beyond Economics)
Because the ledger payload is agnostic at the storage layer, it acts as a generic state channel. The exact same infrastructure used for mutual credit operates as an irrefutable log for:
- **Contract Agreements:** e.g., Mutually signing the exact terms of a service delivery.
- **Delegation of Authority:** e.g., Tracking the issuance and chain-of-custody of access control capabilities (UCANs).
- **Reputation & Vouching:** e.g., Undeniably recording when one entity vouches for another within the trust graph.

## 3. Core Engine Mechanics: Loop Discovery & Graph Pruning
The primary value proposition of the application is automated liquidity optimization through network graph clearing, executing two forms of settlement:

### A. Complete Nullification (Total Settlement)
- **The Loop:** A closed chain of identical debt obligations is discovered via peer-to-peer background routing probes (e.g., User A owes User B ₹5k → B owes C ₹5k → C owes A ₹5k).
- **The Settlement:** The system generates an additive, multi-party, multi-signature offset transaction. When signed by all participants, it simultaneously nets everyone's balance back to zero using zero physical cash. Historical entries are preserved as Settled for auditing integrity.

### B. Partial Graph Pruning (Fractional Settlement)
- **The Bottleneck:** When debt loops intersect with unequal values (e.g., A owes B ₹10k → B owes C ₹7k → C owes A ₹5k), the engine identifies the lowest common denominator (the bottleneck link, which is ₹5k).
- **The Execution:** The engine prunes the graph by shaving off the maximum possible overlapping liquidity (₹5k). It executes a partial multi-sig offset, instantly reducing systemic debt. It mutates the state of the active chain to updated residual values (A owes B ₹5k, B owes C ₹2k, C owes A ₹0) and feeds the leftovers back into the loop-discovery cycle.

## 4. Input Rules & Priority Logic
When partial pruning occurs across a single relationship with multiple active debt items, the backend execution engine must resolve which items to prune based on predefined configuration rules:

- **FIFO Protocol:** Auto-apply partial clearance to the oldest timestamped entry regardless of the tag context.
- **Urgency & Lifecycle Priority Sorting:** Prioritize legally or commercially binding tags (`#Business`, `#Net30`, `#GSTInvoice`) to preserve user credit reputations, leaving socially flexible tags (`#Family`, `#Social`) active.

*(Note: JSON Schema payloads, detailed UI/UX screen flows, and Security/Exception management frameworks are currently deferred for future specification. When designing the data model for payloads, the system will draw heavy inspiration from the ValueFlo.ws REA (Resource, Event, Agent) ontology to maximize flexibility and ecosystem interoperability, though it is not a strict dependency.)*
