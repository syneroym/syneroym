# Multi-Hop Relay Architecture

> [!IMPORTANT]
> **Status: Proposed — not yet implemented.** This document captures a design agreed upon during architecture discussions. Implementation has not started. The design should be reviewed and updated as implementation begins and new constraints are discovered.

---

## Core Model (One-Line Summary)

> A tree of substrates where each node acts as both a **local registry** (routing table of downstream services) and a **transparent relay** (per-connection bidirectional tunnel to the correct neighbor). The caller only needs the address of the nearest reachable hop and the target service DID. The tree routes it the rest of the way.

---

## What This Is NOT

| Discarded approach | Why discarded |
|---|---|
| Onion / layered encryption (Tor model) | Caller would need to know the full path and construct nested crypto. Over-engineered for our goals. |
| Caller encodes the full hop chain in the request | Brittle to topology changes; leaks internal network structure to callers. |
| Shared persistent tunnel between hops (single pipe) | Head-of-line blocking; equivalent to the TCP-over-TCP problem. |

---

## Topology

```mermaid
flowchart TD
    subgraph INTERNET["Public Internet"]
        REG["🌐 Community Registry\n(pkarr / BEP-0044 DHT)\nKnows: service → edge relay address"]
        RELAY_PUB["Public DERP/TURN Relay"]
        CALLER["Remote Caller\n(Browser or Substrate)"]
    end

    subgraph TIER1["Tier-1: Edge Substrate\n(Public IP or reachable via public relay)"]
        E_ROUTER["connection_router\npreamble dispatch"]
        E_HOP["hop-relay-svc\nRouting table: {svc_did → next_hop_did}"]
        E_IDX["Local Cluster Index\n(all services in subtree)"]
        E_PUB["DHT Publisher\n(publishes subtree to global registry)"]
    end

    subgraph TIER2["Tier-2: Mid Substrate\n(Office LAN / CGNAT — outbound to Tier-1 only)"]
        M_ROUTER["connection_router"]
        M_HOP["hop-relay-svc\nRouting table: {svc_did → next_hop_did}"]
        M_IDX["Local Sub-cluster Index\n(all services in its subtree)"]
        M_PUB["Tier-1 Publisher\n(registers its subtree with Tier-1)"]
    end

    subgraph TIER3["Tier-3: Hidden Substrates\n(No outbound except to Tier-2 via local link)"]
        H1["Substrate A\npizza-shop svc"]
        H2["Substrate B\norders-svc"]
    end

    %% Registration / Discovery (upward arrows)
    H1 -->|"1a. Register services\nwith Tier-2"| M_IDX
    H2 -->|"1b. Register services\nwith Tier-2"| M_IDX
    M_PUB -->|"2. Gossip subtree\nto Tier-1"| E_IDX
    E_PUB -->|"3. Publish edge entry point\n+ service DIDs to global DHT"| REG

    %% Connection path (downward, per-connection tunnels)
    CALLER -->|"4. Resolve service DID\n→ edge relay address"| REG
    CALLER -->|"5. Connect to edge\n(Iroh / DERP)"| RELAY_PUB
    RELAY_PUB --> E_ROUTER
    E_ROUTER -->|"6. Preamble: hop-relay\nservice_did=pizza-shop"| E_HOP
    E_HOP -->|"7. Look up routing table\n→ next_hop=Tier-2\nOpen NEW QUIC stream to Tier-2"| M_ROUTER
    M_ROUTER -->|"8. Preamble: hop-relay\nservice_did=pizza-shop"| M_HOP
    M_HOP -->|"9. Look up routing table\n→ next_hop=Substrate-A\nOpen NEW QUIC stream"| H1

    style INTERNET fill:#0d1f35,color:#a8c8f0,stroke:#2E75B6
    style TIER1 fill:#0d2d0d,color:#a8f0a8,stroke:#548235
    style TIER2 fill:#1a2e1a,color:#a8f0a8,stroke:#548235
    style TIER3 fill:#2d1a0d,color:#f0c8a8,stroke:#C55A11
```

---

## The Tunnel Model — Per-Connection Bidirectional Streams

The key mechanism: each hop-relay opens a **new QUIC stream** (not a new QUIC connection) to the next hop for each incoming service connection. This reuses the existing Iroh QUIC connection between persistent neighbors (Iroh manages the connection lifecycle) but isolates each service call to its own stream, eliminating head-of-line blocking.

```mermaid
sequenceDiagram
    participant C as Caller
    participant E as Edge hop-relay-svc
    participant M as Mid hop-relay-svc
    participant H as Hidden Substrate\nconnection_router
    participant S as pizza-shop SynSvc

    Note over C,E: Iroh QUIC connection (possibly via DERP relay)
    Note over E,M: Persistent Iroh QUIC connection (LAN)
    Note over M,H: Persistent Iroh QUIC connection (local link)

    C->>E: New QUIC stream\nPreamble: {type: hop-relay, service_did: "did:svc:pizza-shop"}
    
    E->>E: Routing table lookup:\n"did:svc:pizza-shop" → Mid substrate
    E->>M: Open new QUIC stream on existing conn\nPreamble: {type: hop-relay, service_did: "did:svc:pizza-shop"}
    
    M->>M: Routing table lookup:\n"did:svc:pizza-shop" → Hidden substrate A
    M->>H: Open new QUIC stream on existing conn\nPreamble: {type: service, service_did: "did:svc:pizza-shop"}
    
    H->>H: Preamble dispatch:\nroute to local pizza-shop SynSvc
    H->>S: Dispatch

    Note over C,S: Bidirectional byte stream now flows end-to-end
    Note over C,S: Application-level payload (wRPC/JSON-RPC) is\nopaque to all hops — hops only see the preamble once
    
    S-->>C: Response streams back through the same chain
    
    Note over C,S: On connection close: each hop closes its downstream\nstream independently. The underlying QUIC connections persist.
```

---

## How Each Hop Stays Blind to the Payload

Each hop reads exactly one preamble at connection open time:

```
[4-byte length] [preamble JSON/CBOR]
{
  "type": "hop-relay",         // or "service" at the final hop
  "service_did": "did:svc:pizza-shop"
}
```

After the preamble is consumed, the hop does **pure bidirectional byte copying** between the upstream stream and the downstream stream it opened. It never reads the payload again. This is identical to the existing blind tunnel behavior in the WebRTC coordinator — the hop is a transparent pipe.

The application-level payload (wRPC frames, JSON-RPC requests/responses) is protected by:
- **Transport encryption** per QUIC leg (Iroh QUIC / TLS 1.3)
- **Service-level Ed25519 signing** already present on all substrate messages

No additional onion encryption needed. Each hop simply cannot interpret the bytes it forwards.

---

## Service Registration — Upward Gossip

How a service on a hidden substrate becomes discoverable at the global registry:

```mermaid
sequenceDiagram
    participant H as Hidden Substrate A
    participant M as Mid Substrate
    participant E as Edge Substrate
    participant DHT as Global DHT / Registry

    Note over H,M: Initial pairing (manual config for now)
    H->>M: POST /cluster/register\n{node_did, pubkey, services: [{did, protocol, caps}]}
    M->>M: Verify Ed25519 sig\nStore in local routing table:\n  did:svc:pizza-shop → next_hop=H\nAdd to local cluster index
    M->>M: Sign attestation:\n"I forward for did:p2p:H"

    Note over M,E: Periodic gossip (or on-change push)
    M->>E: ClusterUpdate message\n{from: M, services: [{did:svc:pizza-shop, node: H}, ...]}\nsigned by M
    E->>E: Verify sig\nUpdate routing table:\n  did:svc:pizza-shop → next_hop=M\nUpdate edge cluster index
    E->>E: Sign edge attestation

    E->>DHT: pkarr publish:\n{service_did: did:svc:pizza-shop,\n entry_point: did:p2p:edge,\n relay: relay.syneroym.net}\nSigned by E
    DHT-->>DHT: Globally discoverable
```

**What the global DHT record contains:**

```json
{
  "service_did": "did:svc:pizza-shop",
  "entry_point": "did:p2p:edge-substrate",
  "relay": "https://relay-x.syneroym.net",
  "protocols": ["wrpc", "jsonrpc"],
  "signed_by": "edge-substrate-pubkey",
  "seq": 101
}
```

**What the record does NOT contain:** internal topology (mid-substrate, hidden substrate addresses). The tree structure below the edge is private. The caller only learns *"connect to this edge relay and ask for did:svc:pizza-shop"*.

---

## Connection Establishment — Caller Perspective

From the caller's view this is a simple two-step:

```
1. resolve(service_did)  →  {entry_point, relay}      [DHT lookup]
2. connect(entry_point)                                 [Iroh / DERP]
   send preamble: {type: hop-relay, service_did: ...}
   ← bidirectional stream to service ←
```

The caller has **no idea** how many hops exist between the entry point and the actual service. The complexity is entirely in the substrate tree.

---

## The hop-relay Subsystem

A native Rust subsystem built into the substrate binary with two responsibilities:

### Routing table (registry role)

```
Table: service_did → (next_hop_node_did, iroh_node_addr)

Populated by:
  - Direct registration from children (for Tier-N+1 → Tier-N)
  - Gossip updates from child hop-relay subsystems (for deeper subtrees)

Evicted by:
  - Iroh connection-close event from child substrate (no separate TTL/heartbeat needed)
  - Explicit deregister message from child
```

### Stream forwarder (relay role)

```
On incoming stream:
  1. Read preamble → extract service_did
  2. Look up routing table → next_hop
  3. Open new QUIC stream to next_hop with same preamble
  4. tokio::io::copy_bidirectional(upstream, downstream)
  5. On either side close → close both
```

> [!NOTE]
> **Implementation:** `hop-relay` is a **native Rust subsystem built into the substrate binary**, sitting alongside `connection_router`. WASM sandboxing would add per-byte overhead on every forwarded connection — the wrong tradeoff for infrastructure-level byte shuttling. It uses the same Iroh endpoint and Tokio runtime the substrate already owns:
> ```rust
> // Accept inbound stream (from upstream hop or caller)
> let inbound = iroh_endpoint.accept().await?;
> let service_did = read_preamble(&inbound).await?;
>
> // Look up routing table → next hop
> let next_hop = routing_table.get(&service_did)?;
>
> // Open outbound stream to next hop
> let outbound = iroh_endpoint.connect(next_hop, &service_did).await?;
>
> // Pure byte forwarding — substrate is a transparent pipe
> tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await?;
> ```
> Being native also means it can share the substrate's existing Iroh `Endpoint` directly — no capability boundary crossing needed.

---

## Routing Table Propagation — Like BGP, But Simpler

Each hop only knows about **its own routing table** — it does not know the full path. When service `did:svc:pizza-shop` is registered:

| Hop | Routing table entry |
|---|---|
| Mid Substrate | `did:svc:pizza-shop → Hidden Substrate A` |
| Edge Substrate | `did:svc:pizza-shop → Mid Substrate` |
| Global DHT | `did:svc:pizza-shop → Edge Substrate` (public record) |

Each node only knows the **next hop**, not the full path. This is exactly the IP routing model applied to service DIDs.

---

## Comparison: This Design vs. the Discarded Onion Model

| Aspect | Discarded (Onion/Layered) | This Design (Hop-Relay Tree) |
|---|---|---|
| Caller must know full path | ✅ Yes | ❌ No — just entry point + service DID |
| Crypto per hop | Layered Ed25519 encrypt per hop | None extra — per-hop QUIC TLS + existing service signing |
| Routing responsibility | Caller | Each hop (local routing table) |
| Adding a new hop depth | Caller must update | Only the new hop registers with its parent |
| Implementation complexity | High | Low — reuses blind tunnel + preamble dispatch |
| Topology leakage to caller | Full path visible | Only entry point visible |
| Head-of-line blocking risk | Low (per-hop layered) | Low (per-connection QUIC streams) |

---

## What Changes in Existing Code

| File / Component | Change |
|---|---|
| `router/connection_router.rs` | Add `HopRelay` preamble variant; on match, delegate to the `hop_relay` subsystem |
| `crates/substrate/` (new module) | `hop_relay`: native Rust subsystem — routing table, Iroh connection-close listener, gossip sender/receiver, `copy_bidirectional` forwarder |
| `pkarr` / registry publish logic | Edge substrate publishes subtree service records to global DHT; record format gains `entry_point` field |
| WIT interfaces | `hop-relay` WIT interface for the *management plane* only: `register-service`, `deregister-service`, `gossip-update` (the data plane is all native, no WIT involved) |
| Config / CLI | `syneroym hop-relay enable` — activates the native subsystem; `syneroym join-hop <parent-addr>` for initial pairing |

---

## Resolved Design Decisions

| Question | Resolution |
|---|---|
| **Routing table TTL** — if a hidden substrate goes offline silently, how is its routing table entry evicted? | **Iroh handles this.** The Iroh connection between the child substrate and its parent drops on disconnect. The `hop-relay` subsystem listens for connection-closed events from Iroh and evicts the corresponding routing table entries immediately. Propagation upward follows the same path as registration. No separate heartbeat/TTL mechanism needed. |
| **Duplicate service DID** — same service DID registered at two different tree branches. | Shouldn't happen — service DIDs are derived from keypairs, so each service has a unique DID by construction. If it somehow does occur, each hop's routing table keeps the **first-registered** entry (stability preferred over last-writer-wins). A duplicate detection log entry is emitted so operators can investigate. |
| **Native vs. WASM** — should `hop-relay` be a SynSvc or a substrate subsystem? | **Native Rust in the substrate binary.** Shares the substrate's Iroh `Endpoint` and Tokio runtime directly. WASM overhead on a tight `copy_bidirectional` loop is the wrong tradeoff for infrastructure-level byte forwarding. |
| **Connection upgrade** — if two substrates previously connected via N hops discover they can reach each other directly, does the chain short-circuit? | **On new connections only.** When a child substrate re-registers with a shorter/direct path, the routing table is updated. Existing in-flight connections drain naturally over the old path. New connections use the updated path automatically. No mid-connection rerouting. |
