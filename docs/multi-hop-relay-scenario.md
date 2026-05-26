# Multi-Hop Relay Scenario: End-to-End Flow

This document describes the sequence of operations for establishing a multi-hop relayed connection between two Synapp services across network boundaries.

> [!NOTE]
> **Architectural Alignment:** This scenario employs the "Federated Coordinator" model. The `hop-relay` subsystem operates within the **Coordinator**, unifying Layer 7 preamble forwarding for Iroh with the existing WebRTC fallback relay mechanism. Substrates do not act as network bridges; they act purely as endpoints connecting to their local Coordinator.

---

## Scenario Entities

*   **Public Infrastructure (Internet)**
    *   **C**: Global Coordinator (acting as public DERP/TURN relay, and public `hop-relay`).
    *   **R**: Global Registry (community registry, future DHT).
*   **Public/External Edge**
    *   **Sx**: Substrate with outbound internet access.
    *   **Ax**: Synapp deployed on **Sx**.
*   **Private Subnetwork Infrastructure**
    *   **Cp**: Private Coordinator (local relay). Acts as the `hop-relay` for the private network.
    *   **Rp**: Private Registry (community registry). Connects outbound to **R** to gossip records.
    *   **Sz**: Hidden Substrate. Resides purely in the private network with no external internet access.
    *   **Az**: Synapp deployed on **Sz**.

---

## 1. Startup and Configuration

1.  **Public Infrastructure Starts**: Coordinator **C** and Registry **R** are brought online on the public internet.
2.  **Private Infrastructure Starts**: 
    *   Coordinator **Cp** and Registry **Rp** are brought online within the private subnetwork.
    *   **Cp** exposes a lightweight HTTP discovery endpoint (e.g., `/v1/info`) that serves its Iroh Node ID and relay configuration.
    *   **Cp** also registers itself in the global Registry **R** as an available coordinator (controlled by a configuration switch to share its record).
    *   **Cp** does *not* maintain a permanent connection to **C**. It connects outbound to the public Coordinator **C** on-demand only when data transfer is needed.
    *   **Rp** is configured with **R** as its parent registry so it can query and publish records upward.
3.  **External Substrate (Sx) Starts**: 
    *   **Sx** connects outbound to Coordinator **C** and Registry **R**. 
4.  **Hidden Substrate (Sz) Starts**: 
    *   **Sz** starts in the private network and connects to its local Registry (**Rp**).
    *   To find a local coordinator, **Sz** first checks its config for a direct `discovery_url` (fetching the Iroh connection details via HTTP). If not provided, it queries its local Registry **Rp** (which forwards the lookup to **R**) to discover available coordinators. It dynamically selects one (e.g., **Cp**) and caches its Iroh details.

## 2. Registry Entries at Deployment

1.  **Ax Deployment**: 
    *   Synapp **Ax** is deployed on **Sx**. 
    *   The deployer of **Ax** (using `SyneroymClient::deploy_wasm`) generates a signed service record and publishes it directly to the global Registry **R**.
2.  **Az and Sz Deployment**: 
    *   Synapp **Az** is deployed on the hidden substrate **Sz**.
    *   The substrate **Sz** registers itself and its services (**Az**) with the local Registry **Rp**.
3.  **Cp Registration**: 
    *   The private Coordinator **Cp** registers its Iroh key and connection details (like relay endpoints) into the global Registry (**R**), assuming its configuration switch is set to share its record. This makes its Iroh endpoint dynamically discoverable for substrates relying on registry lookups.
4.  **Upward Gossip**: 
    *   **Rp** gossips the registration of both **Az** and **Sz** upward to the global Registry **R**.
5.  **Global Record State**: 
    *   The global Registry **R** now holds public records for **Az** and **Sz**. 
    *   Because **Az** is deployed on **Sz**, the record primarily obscures **Sz** (and indirectly **Az** via **Sz**). It states that to reach **Sz**, a caller must route to the entry point **Cp**.
    *   The record also copies over the private topology, allowing **Cp** to use a registry lookup to find the specific connection details for **Sz** when transferring data.

## 3. Communication Flow: Ax connecting to Az (Inbound to Private)

1.  **Packet Transmission**: Synapp **Ax** uses the `SyneroymClient` to send a packet to **Az**. The client initiates a connection to the next hop.
2.  **Global Resolution**: The client queries the global Registry **R** for **Az**.
3.  **Discovery**: Registry **R** responds with the routing information: target entry point is **Cp** (whose public connection details are also provided).
4.  **Connection to Cp**: 
    *   The client establishes a connection to the private Coordinator **Cp** (transparently using the Iroh SDK, which leverages public relay **C** internally).
    *   The client opens a stream and directly sends a connection preamble to **Cp**, containing the target service DID (**Sz** / **Az**) and the calling substrate's public identity (**Sx**'s public key or an ephemeral key).
5.  **Routing (Cp to Sz)**: 
    *   **Cp** receives the stream and reads the preamble.
    *   **Cp** performs a registry lookup to find the connection details for **Sz** (no in-memory routing table caches are used).
    *   It determines the final hop is the hidden substrate **Sz**. **Cp** establishes an Iroh connection and forwards the stream to **Sz**.
6.  **Target Dispatch and Handshake (Sz)**: 
    *   **Sz** receives the stream and reads the preamble to recognize the target is its local Synapp **Az**.
    *   **Sz** and **Sx** complete an explicit End-to-End Diffie-Hellman handshake inside the stream.
    *   Once the secure channel is established, **Sz** dispatches the application payload to **Az**.

## 4. Communication Flow: Az connecting to Ax (Outbound to Public)

1.  **Packet Transmission**: Synapp **Az** asks its host substrate **Sz** to send a packet to **Ax**.
2.  **Resolution**: The client on **Sz** queries the local Registry **Rp**.
    *   **Rp** does not have a local record for **Ax**, so it queries its parent, the global Registry **R**.
    *   **R** returns **Ax**'s location (reachable directly via **Sx** on the public internet).
3.  **Outbound Routing**: Because **Sz** has no outbound internet access, it cannot connect to **Sx** directly. It utilizes the **Cp** Iroh connection details it retrieved at startup (either via HTTP discovery or via the **Rp** -> **R** registry lookup), and prepares to route the connection request through **Cp**.
4.  **Stream Setup**: 
    *   **Sz** connects to **Cp** and sends the preamble for **Ax** (including **Sz**'s public key or an ephemeral public key).
    *   **Cp** reads the preamble, realizes the target is on the public network, and connects outbound to deliver the stream to **Sx** (potentially via relay **C**).
5.  **Data Transfer**: The bidirectional stream is established. Because **Cp** initiated an *outbound* connection on-demand, it natively bypasses the inbound reachability limitations (NATs/Firewalls) that constrain the Ax -> Az flow.

## 5. Data Transfer Characteristics

1.  **End-to-End (E2E) Encryption Handshake**:
    *   While intermediate transport legs are protected by Iroh, the Coordinators must route via preambles.
    *   To ensure true privacy, once the multi-hop stream is connected, the endpoints (**Sx** and **Sz**) perform an explicit E2E encryption handshake inside the established stream.
    *   This handshake utilizes the exact mechanism currently implemented in the frontend (`peer-proxy.html` / `verifyAndDeriveSharedSecret`): an explicit ECDH key exchange where the ephemeral keys are signed by the permanent Ed25519 identity keys. This provides mutual authentication and establishes an AES-GCM symmetric cipher state.
2.  **Opaque Forwarding**: 
    *   Following the E2E handshake, the application payload (e.g., wRPC frames) is encrypted at **Sx** and decrypted only at **Sz** (or vice versa).
    *   The Coordinator **Cp** acts purely as a blind Level 7 pipe. It copies the E2E-encrypted bytes back and forth between streams and cannot read the application payload.
3.  **Teardown**: 
    *   Once the communication finishes, either endpoint closes the stream. 
    *   Each hop independently closes its respective stream segment.
