# Syneroym Ecosystem Spec [WIP]
This document expands on the vision described [here] (/VISION.md). Please go through that to understand the bigger picture. Following from there, our objective is to build a technology substrate that enables diverse classes of provider ecosystems to emerge through `Autonomous Mini-Apps Cooperating over a common technology substrate`. We also build initial mini-apps that kickstart these new ecosystems and demonstrate various interaction patterns. 

This requirements spec is structured as follows.

- Philosophy & Design Constraints
- Requirements Overview
- Conceptual Model
- Substrate Functionality: The core substrate layer for all participants
- Shared Utilities and Services: Common Utilities useful to multiple mini-apps
- Mini-App Specs:
    - Vertical 1: Home Services Guild (e.g. Electricians, Plumbers)
    - Vertical 2: Food and Small Retailer Mesh (e.g. Small restaurants, grocery stores)


## Philosophy & Design Constraints
A key difference in the newly envisioned provider ecosystem compared to large-scale consumer platforms is, the (often geographical) clustering of service providers and consumers. Global reach and scale from a single embedding source is not a fundamental requirement. Reach and scale are improved instead, by pre-established collaboration and coordination patterns across clusters of autonomous participants. Given this differentiator, we need to preserve benefits and reduce the drawbacks of large-scale consumer platforms like those listed in the [vision document](/VISION.md#background).

Following is a list of benefits of large-scale consumer platforms we need to preserve and drawbacks we need to reduce, and our approach to achieve those.

### Preserving Benefits
    - Technology enablement of business without managed infrastructure
    - Massive discovery & distribution
    - Streamlining, standardization of interaction patterns
    - Institutional trust
    - Security at scale
    - Fault tolerance
    - Legal shielding
    - Reputation aggregation
    - Economies of scale
    - Network effects

### Reducing Drawbacks
    - Non-availability (geographies, power/network/technology constrained scenarios)
    - Vendor lock-in
    - Governance asymmetry less freedom (but less decision making hassle) to participants
    - Flexibility to customize for localized scenarios
    - Data ownership loss
    - Sudden policy risk leading to unhappy participants
    - No transparency of how the internal systems/algorithms work
    - Strategic dependency
    - Not friendly to buildup of deep provider-client relationships, mostly transactional


## Requirements Overview
High level requirement highlights

### Personas in Syneroym ecosystem
The following are key personas in the Syneroym ecosystem. 

- Provider:
    - Individual Provider: E.g. Anyone providing some service to others. E.g. a plumber, photographer, blogger.
    - Self hosted Provider: Providers who hosts their online services themselves.
    - Provider Aggregator: They take up the responsibility of hosting online services of multiple providers.
- Infrastructure providers: They make their hardware infrastructure available for others to use/lease.
- Consumer: Entities consuming online services like a retail buyer, or person requesting plumbing service. 
- App Developer: Who builds business mini-apps and make those available for others to deploy on their infrastructure.

Of course, it is likely that a single Person/Org plays the role of different personas. 

### Common requirements
Following are common user requirements irrespective of business domain.

- Service Providers host business applications on PCs or Mobiles they control, and use those via UI, CLI or other means as applicable.
- Hardware infrastructure owners make hardware (old PCs, or cloud) available for service providers who then can then host applications or their parts (modules, services) on such leased infrastructure.
- Service Providers can monitor online service health, react to notifications about service status - Service Providers can move services and data across Infrastructure providers without restriction
- Service Providers can backup and restore app data
- Service Providers can access services/data from other providers and also control access to their data/services as per agreements and workflows with ecosystem partners. E.g. Medical service provider can provide Patients their latest medical records, or allow access to other providers if patient consents.

## Conceptual Model
The following diagram shows various conceptual entities in the Syneroym ecosystem and relationships between them. Will help establish common nomenclature too.

```mermaid
---
title: Syneroym Conceptual Model
config:
    layout: elk
---
erDiagram
    direction TB
    
    MOD ||--|{ MOD : depends-on
    MOD ||--|{ SVC : invokes
    MOD ||--|{ SVC : template-for
    SYNAPP ||--|{ SVC : comprises-of
    SVC }|--|| SVC-SB : runs-in
    HOST ||--o{ NODE : runs
    NODE ||--o{ SVC-SB : runs
    SUBSTRATE ||--|| NODE : runs-on
    SUBSTRATE ||--o{ SVC : manages-and-proxies
    NODE-OWNER ||--o{ SUBSTRATE : owns
    SYNAPP-OWNER ||--|{ SYNAPP : owns
    SYNAPP }o--|| SUBSTRATE : registers-at
    SERVICE }o--|{ HOME_RELAY : registers-at

    MOD[SYN-MOD]{}
    SVC[SYN-SVC]{}
    SYNAPP[SYN-APP]{}
    SUBSTRATE[SYN-SUBSTRATE]{}
    SVC-SB[SVC-SANDBOX]{}

```

## Core Substrate Functionality
Description of the core Syneroym substrate functionality, key protocols, important flows.

### Substrate Setup
- Node-owner Installs substrate on node
- Substrate creates admin keys
- Register to relay:
    - Get home relay to connect from bootstrap server
    - Insert node key and relay used in Pkarr signed packet (for node's control plane services like SYN-SVC deploy/remove) in BEP 0044 DHT
    - Start iroh quic server on that relay
- Substrate Identifies its capabilities (sandbox/container types, quota configurability), user configures limits (CPU, Mem, Disk) for Services
- Access control:
    - Register substrate pubkey with owner's primary substrate (i.e. owner owns multiple substrates), 
    - Enable necessary substrate access to owner primary key
    - Provide access control to various SYN-APP owner pubkeys for SYNAPP management APIs of substrate (deploy, remove, observe), and associated quotas

## Supporting Services
### Relay
- Iroh Relay, and/or TURN Relay (for WebRTC)
- Apply to register as community relay with syneroym bootstrap server if interested in contributing (refresh periodically)
- On successful registration, it is available as relaynodeid.syneroym.net, download certs

### Bootstrap
Register new relays:
- Register some officially maintained relays with capabilities (tcp relay, TURN)
- Accept new relay's Offer to register as community relay for capabilities it provides, accept after verification (offline, or real-time checks)
- Register the DNS entry for that relay with registrar for *.relaynodeid.syneroym.net
- Return a random set of relays from the registered ones based on relay capability requested, weight relays by their capacity (powerful relays are returned with higher probability)
- Periodically run relay checks and expire stale community relays

Relay Lookup for nodeid:
- For any nodeid lookup, check in internal cache or DHT fallback and return relay
- For HTTP url nodeid lookups (browsers), find the relay and send an HTTP redirect to it. 

## Synapp Lifecycle
### Development
- Build wasm components with wRPC for inter-component calling
- JSON-RPC Dispatch Code generation for non wRPC callers if needed (e.g. JSON-RPC for calls from browser). 
- Packaging in OCI containers
### Deployment
- Application specification composing components
- Provider Applies application spec to substrates available
### Runtime
- Substrates monitor application and provide health info, notifications, redeploy support

## Spec Vertical 1: Home Services Guild
## Spec Vertical 2: Food and Small Retailer Mesh

## Design considerations
- Iroh for p2p, hole punching, relay
- webrtc-rs for connection via browser WebRTC Datachannels
- Consider JSON-RPC and wasm components or wRPC. WIT as Canonical API and JSON-RPC derived from it