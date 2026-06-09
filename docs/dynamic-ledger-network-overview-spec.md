# Dynamic Ledger Network (DLN) Specification

## 1. Product Vision
An unstoppable, cash-free, peer-to-peer financial network that enables individuals and businesses to trade, settle debts, and optimize liquidity without relying on central bank currency or traditional banking infrastructure. It digitizes organic community trust networks via a decentralized, tag-based, self-clearing cryptographic ledger.

## 2. Core Architectural Pillars

### Decentralized Local Logs
There is no centralized database or public blockchain token. Every user's device maintains a private, append-only, tamper-proof cryptographic log containing only the transactions they are directly involved in.

### Mutual Cryptographic Consensus
A ledger entry is legally and mathematically valid within the network only when it carries the valid digital private-key signatures of both participating parties.

### Flat, Tag-Based Metadata Engine
Transactions are stored in a single flat ledger database on the device. Relationships, constraints, and business rules are applied dynamically via flexible metadata tags (e.g., `#Family`, `#Business`, `#Net30`, `#Kirana`), completely replacing rigid folder or account hierarchies. Hierarchies are treated strictly as user-interface visualization filters.

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

*(Note: JSON Schema payloads, detailed UI/UX screen flows, and Security/Exception management frameworks are currently deferred for future specification.)*
