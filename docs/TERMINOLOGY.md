# Syneroym Terminology

Canonical names and phrasings for the project, extracted from the
[thesis](../THESIS.md). Use these consistently across docs and external
communication; when a fuller statement is needed, link to the thesis rather
than restating it.

## Names

| Term | Meaning |
|---|---|
| **Syneroym** | The project and ecosystem as a whole. |
| **Substrate** | The truly peer-to-peer foundation for group communication and trust. Generic — anyone can build their own mini-app on it. |
| **SynApp** (mini-app) | An independent application that plugs into the substrate. "Mini-app" is the informal synonym. |
| **Roym** | Our flagship combined experience: mini-apps sharing one identity, one contact list, one set of groups, one trust model. The proof the substrate works — not the only thing it is for. |
| **Professional Services Guild** | Roym's first vertical (home services first); built as a deep, real vertical. |
| **Local Producer-Distributor Mesh** | Roym's second vertical (food + small retail); built thinner, for demonstration. |
| **Bootstrap nodes / relays** | Small community-run nodes that help peers find each other and relay — end-to-end encrypted — the small fraction of connections that cannot be made directly. They store nothing and own nothing. |

## Component names (engineering)

The names above are positioning vocabulary, drawn from the thesis. This section
is the engineering counterpart: internal component names that are easy to
confuse with each other, written down because several near-synonyms are already
taken by different things. Use these exactly; if a design discussion needs a new
component name, add it here rather than picking a word from this table's
neighbourhood.

| Term | Meaning | Not to be confused with |
|---|---|---|
| **Community Registry** | Service discovery: signed `EndpointInfo` records mapping a **DID → endpoint** (how to reach it on the network). A live service, resolved per call. `crates/community_registry`. | App Registry — different mapping, different scope. |
| **App Registry** | The **logical service name → member master DIDs** mapping within an app instance (a member set, not a single DID). A *trait* (`AppRegistry`) plus its topology state, not a deployed service. `crates/app_orchestration/src/resolver.rs`. | Community Registry; and note there is deliberately no live "app registry service" — see [ADR-0021](decisions/0021-binding-propagation-and-app-supervisor.md). |
| **Control Plane** | The per-substrate RPC surface for `deploy` / `undeploy` / `list` and secret management. `crates/control_plane`. | The App Supervisor, which is a *client* of this. |
| **Controller** | The owning entity (person or organization) that controls a substrate, established by a `ControllerAgreement`. A principal, never a process. | The App Supervisor. Do **not** call the supervisor a controller. |
| **Orchestrator** | The native service on each substrate that manages that substrate's deployed-app catalog and runs its services. `crates/app_orchestration` + `crates/sandbox_wasm`. | The App Supervisor, which spans substrates and drives orchestrators. |
| **App Supervisor** | The component that holds an app's desired state across substrates and keeps it true: deploy-time checks and retries, health monitoring, bounded remediation, alerting, and pushing dependency bindings into services. Exposes one interface, `supervisor`, consumed by `roymctl` and by callers that resolve a logical service. | Everything above. Do **not** call the supervisor a "controller" or "stateful controller". It has an operator-facing read surface (status, alerts, convergence). It also has one `resolve` verb. `resolve` answers a caller that holds the grant (or any caller, if the service is open) with a signed topology document: the member master DIDs of one logical service. It is a signed read of the supervisor's state, not a live directory service. The SDK calls it from the client side. The supervisor pushes dependency bindings into services, so they do not call it to find each other. See [ADR-0022](decisions/0022-two-tier-logical-service-discovery.md). |
| **App Inventory** | What is deployed for an app instance: services, placement, current bindings, health. Held by the App Supervisor. | Substrate Inventory. |
| **Substrate Inventory** | The operator's list of *target* substrates — reachability, capabilities, and the deploy capability held on each. Held by the App Supervisor. | App Inventory. |
| **Master DID (member)** | The stable identity of **one member** of a logical service — one per member, not one per service, so that round-robin and sharding have distinct principals to select over (a three-member redundant service has three). What FDAE policy, stored row attribution, and capability audiences all key on. Survives that member being relocated, restarted, or crash-recovered. See [ADR-0020](decisions/0020-stable-logical-service-identity.md). | Instance DID; and a single identity for the whole logical service, which is the granularity ADR-0020 §1 rejects. |
| **Master DID (person/organization)** | The stable identity of a human or organizational principal, delegating to temporary device keys. The original sense of the term, unchanged. | Master DID (member). |
| **Instance DID** | The transport-level identity of one physical service instance, delegated from its master DID. Appears in the route preamble and the hosting substrate's keystore, never in the application model. See [ADR-0020](decisions/0020-stable-logical-service-identity.md). | Master DID. |
| **Home relay** | The one Iroh relay that a substrate endpoint stays connected to, providing fallback connectivity when direct peer-to-peer connections fail. Its URL is advertised in the node's signed endpoint record. It comes from `parent_coordinator.iroh.url`, or the substrate picks it from the coordinator list of its registry. Confirmation by net-report probe means Iroh selected the relay, but does not prove the relay admits this node. | The relay server running locally on a coordinator. |
| **Degraded mode** | Operating mode where a substrate runs with an empty relay map and no home relay. It happens when `parent_coordinator.iroh.url` is not set and the coordinator list of the registry gives no usable relay (or `substrate.registry_url` is not set). A node with a `registry_url` keeps asking for a relay in the background. It can dial peers through their relays, but peers cannot reach it through a relay, and it defers publishing its endpoint record until a home relay is confirmed. | Normal operating mode with a confirmed home relay. |
| **Curated list** | A hand-written list in the registry config that the registry serves over HTTP; the coordinator list and the open mailbox list are two. | Dynamic or self-registered directory entries. |
| **Passport** | A signed, time-limited UCAN token issued by an infrastructure service operator to a user or organization DID, granting an ability (such as `relay/home` or `mailbox/use`) with optional signed use limits (such as `max_nodes` for a relay, or `max_services` and `max_messages` for a mailbox). | A session token or delegation certificate. |
| **Child token** | A UCAN token signed by a passport holder that re-delegates the passport's ability to a specific subject (such as a substrate node DID for a relay, or a member master DID for a mailbox). Presented during registration with proof of possession. | The parent passport token. |
| **Lease** | A time-bounded grant recorded in a service's local database after successful registration, permitting the subject (a node for a relay, a member master for a mailbox) to use the restricted service until expiry. A mailbox lease also holds the signed limits of the passport. | A registry registration or permanent admission. |
| **Info URL** | The base HTTP/HTTPS URL of a coordinator's info server (serving `/v1/info` and registration endpoints). | The Iroh relay URL (`parent_coordinator.iroh.url`). |
| **Mailbox key** | The decrypt-only X25519 key pair of one service member. It is derived from the member master (HKDF-SHA256), so every deploy of the member gets the same key. The public half is in the signed service record, and senders seal messages to it. The private half is in the service's vault under a name only the substrate can read. See [ADR-0026](decisions/0026-receiver-chosen-mailbox.md). | The member master key, which signs the record and never reaches a hosting substrate. |
| **Reserved vault name** | A vault entry name that starts with `syneroym.`. The substrate keeps these for itself. A guest cannot read them: the vault interface answers `not-found`. | A guest's own secrets. |
| **Mailbox** | A substrate role running a native store-and-forward service (`mailbox`) that holds sealed messages for sleeping or offline receivers. The mailbox cannot read or tamper with message contents. See [ADR-0026](decisions/0026-receiver-chosen-mailbox.md). | An ordinary message broker or MQTT queue. |
| **Mailbox-eligible method** | A guest method that a service lists for delivery from its mailbox (`mailbox_methods`, text form `<interface>#<method>`), or Conversation `deliver` or `group-push`, which are always eligible. A message for any other method is rejected with a signed `not_eligible` receipt. See [ADR-0026](decisions/0026-receiver-chosen-mailbox.md). | A method that a caller may call directly. |
| **Queue lifetime** | The duration a destination queue stays active at a mailbox without a receiver registration or claim. Past this duration, the queue and its stored messages are swept. | Visibility timeout or message time-to-live. |
| **Sealed message** | An authenticated, HPKE-encrypted message envelope sealed to the receiver member's mailbox public key with the sender's instance certificate and signature inside. | Transport-level stream encryption or plaintext RPC payloads. |
| **Route preamble** | The initial header line framing every transport stream (`<scheme>://<interface>\|<service_id>?<params>`). Declares wire transport, application protocol, target service and interface, caller public key, proof-of-possession signature and timestamp, and optional delegations. See [ADR-0002](decisions/0002-handshake-authorization-point.md). | An HTTP request line. |
| **On-behalf proof** | A cryptographic proof carried in a route preamble showing that an intermediate service calls a downstream service on behalf of an originating user, verified against that user's anchor. See [ADR-0016](decisions/0016-native-dispatch-identity-threading.md). | A UCAN delegation token. |
| **Forwarder** | A coordinator node that accepts a transport stream for a service hosted on another substrate and relays it to the host destination (`relay_to_next_hop`) without inspecting caller credentials. See [ADR-0002](decisions/0002-handshake-authorization-point.md). | A destination host or proxy client. |
| **Direct connection** | A stream connection where the client dials the node hosting the target service directly. Identity parameters travel in the route preamble line. | Indirect connection. |
| **Indirect connection** | A stream connection routed through a forwarder coordinator to reach the target service. Caller identity parameters are sent inside an end-to-end encrypted channel after the host verifies itself. | Direct connection. |
| **Identity line** | The first encrypted line sent after the end-to-end handshake on an indirect or encrypted stream (`params.join("&") + "\n"`). Carries caller proof (`pubkey`, `sig`, `ts`, `delegation`, `ucan`, `on_behalf`) hidden from forwarders. | Route preamble line. |

## Key phrasings

- **One-liner:** "A truly peer-to-peer foundation for group communication and
  trust, on which independent mini-apps — chat, marketplace, social, AI — plug
  in and work together as one experience, with no central server in the middle."
- **The hard problem:** quote verbatim from
  [the thesis](../THESIS.md#the-hard-problem-we-are-solving) or link to it; in
  short: rich group activity at real scale, with no central server, where no
  participant's device needs to fully trust any other.
- **Federation:** cooperation between independently owned peer clusters over
  shared protocols. It is *not* server federation — no server sits between
  participants. Use "federation" only for cluster-to-cluster interop, never as
  a description of Syneroym itself.
- **The exit rule:** "You can always leave, taking your data, history, and
  reputation with you."
- **Posture toward platforms:** an alternative for people who want one —
  chosen, not imposed. Not a replacement for existing platforms.

## Say / don't say

| Instead of | Say |
|---|---|
| "federated ecosystem" (as self-description) | "truly peer-to-peer"; "peer clusters cooperating through federation" |
| "platform monopolies", anti-platform rhetoric | the benefits/drawbacks framing in [VISION.md](VISION.md#background) |
| "Provider Space" (retired name) | **Roym** |
| "Federated Professional Services Guild" (retired name) | **Professional Services Guild**, a Roym vertical |
| anything implying blockchain or crypto | "no blockchains or cryptocurrency"; Syneroym Coin is "not a cryptocurrency or blockchain-based token" |
| "controller" or "stateful controller" (for the supervisor process) | **App Supervisor** |

## Which document owns what

- [THESIS.md](../THESIS.md) — positioning: the bet, the hard problem, what
  this is not. Stated once; everything else links to it.
- [VISION.md](VISION.md) — what we will pursue and build; platform
  benefits/drawbacks background.
- [system-requirements-spec.md](system-requirements-spec.md) — canonical
  product and ecosystem requirements.
- [system-architecture.md](system-architecture.md) — architecture, technology
  stack, and component design.
