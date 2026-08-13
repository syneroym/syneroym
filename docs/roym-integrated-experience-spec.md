---
title: RoyM: Integrated Experience
status: Draft
last_review: 2026-08-13
companion_documents:
  - "[VISION.md](../VISION.md)"
  - "[system-requirements-spec.md](../system-requirements-spec.md)"
  - "[meta-implementation-plan.md](meta-implementation-plan.md)"
  - "[ADR-0013](../decisions/0013-p2p-messaging-architecture.md)"
  - "[ADR-0015](../decisions/0015-ucan-capability-model.md)"
  - "[ADR-0023](../decisions/0023-durable-async-primitives.md)"
---

# RoyM: Integrated Experience

## What RoyM is

RoyM is one place to talk with people, find local help or goods, and arrange
work or purchases. It should feel as simple as a chat app, while letting small
businesses and local groups serve people directly.

It starts with two kinds of local offerings:

- **Home services:** people can find and work with local professionals, such as
  plumbers, cleaners, or electricians.
- **Food and small shops:** people can find local producers and sellers, ask
  questions, and arrange an order or delivery.

RoyM does not make people choose one role forever. The same person can chat
with friends, buy something, offer a service, or help run a local group.

## People who use RoyM

- **Customer (Consumer):** Finds people or shops, asks questions, agrees on
  work or a purchase, and keeps the conversation and record in one place.
- **Service business or seller (Provider):** Shows what they offer, speaks with
  customers, manages bookings or orders, and keeps their customer
  relationships. Runs a Syneroym substrate with the RoyM app.
- **Local group (SynOrg):** Helps members join, shares local knowledge,
  recommends trusted members, and deals with problems inside the group. The
  SynOrg Owner manages member providers, operates discovery, and maintains
  group rules.

**Terminology:** This document uses both "local group" (plain English) and
"SynOrg" (Syneroym Organization). They mean the same thing: an autonomous
administrative domain (guild, union, cooperative, etc.).

> **Note on roles:** A single person can hold more than one role. A SynOrg
> owner may also be a provider. A consumer may later become a provider. The
> participant journey below orders roles by logical dependency, not by the
> identity of people.

## How people and programs use RoyM

- **Web, desktop, and mobile apps:** A light RoyM app shows chats and action
  cards on the web, as an installable web app, or in desktop and mobile apps.
  These user interfaces use the JSON-RPC API.
- **Program API:** Other apps and scripts use JSON-RPC through the client
  gateway.
- **Command line:** `roymctl` extensions support simple automation and provider
  management from a terminal.

## Trust and records

- **Signed actions:** Each message and action is signed by the person's stable
  cryptographic identity, called a Master DID, so its author can be verified.
- **Recommendations:** People can share trusted contacts and provider
  recommendations in private chat.
- **Group membership proof:** A local group can issue a signed credential to an
  approved provider, proving that the provider belongs to that group.
- **Payment proof:** After an external payment, a signed receipt can be added
  to the conversation so both sides have the same proof of payment.
- **Fulfilment proof:** When a service is completed or goods are delivered, a
  mutually signed receipt can be added to the conversation to prove the work
  was done.
- **Community moderation:** A local group can suspend or remove members who
  break its rules, including removing their listings from its local search.

## Features

### Everyday chat

- **Your profile:** Add a name, short introduction, and picture so people know
  who they are talking with.
- **Contacts and favourites:** Add or share contact details, keep important
  people easy to find, and send the same update to a chosen list.
- **One inbox:** See personal chats, group chats, customer conversations, and
  order updates together in one place.
- **Private chats:** Have private one-to-one conversations where only the two
  people in the chat can read the messages.
- **Group chats:** Talk with a chosen group, such as a family, a team, or a
  neighbourhood group, without outsiders reading the conversation.
- **Text, photos, and files:** Send words, pictures, videos, and files in the
  same conversation.
- **Replies and separate topics:** Reply to one message or continue a side
  topic without mixing it into the main group conversation.
- **Forwarding:** Pass a useful message, file, or update from one conversation
  to another.
- **Useful message cards:** Send a small card in a chat for an action such as
  answering a poll, reviewing a quote, or choosing a time.
- **Messages when someone is away:** Send a message even when the other person
  is offline; it is delivered when they are reachable again.
- **Typing and read signs:** See when someone is typing and, when they allow
  it, when they have read a message.
- **Use more than one device:** Keep chats and changes in step when the same
  person uses RoyM on more than one device.
- **Edit, delete, and keep messages:** Correct or remove messages, with
  conventional rules for how long chat records are kept.

### Finding local help and goods

- **Local search:** Search in plain words, such as "plumber near me" or "fresh
  bread nearby," and see relevant local people and shops.
- **Service and shop pages:** Each business can show what it offers, where it
  serves, when it is available, and any important details.
- **Work across independent installations:** Find, assess, and transact with a
  provider even when they use a different Syneroym installation.
- **Recommendations and membership proof:** See recommendations and proof that
  a business belongs to a local group before deciding whether to contact it.
- **Talk before you decide:** Start a chat from a listing to ask questions,
  explain a need, or decide whether the business is suitable.

### Booking, buying, and getting work done

- **Quotes in chat:** A business can send a clear offer showing the work,
  price, and terms; the customer can accept it or ask for changes in the chat.
- **Book a time:** See open times and choose an appointment without leaving the
  conversation.
- **Place an order:** Agree on goods, quantity, price, and delivery details in
  a single order conversation.
- **Track progress:** Both sides can see simple updates, from request through
  booking, work, delivery, and completion.
- **Pay using a familiar payment service:** Open a normal payment link, such as
  a card or bank-payment page, when payment is needed.
- **Keep payment proof:** Add proof of a completed payment to the conversation
  so both sides have the same record.
- **Keep fulfilment proof:** Add a signed receipt to the conversation when the
  work is finished or goods are delivered, so both sides have an agreed record
  of completion.
- **Ask for help with a problem:** Start a dispute from the conversation and
  share the agreed details with the local group that handles the case.

### For service businesses and sellers

- **Create and update listings:** Describe services or goods, prices, service
  area, opening times, available stock, and other useful details.
- **Describe an offer in a standard way:** A listing records booking, payment,
  product, service, location, relationship, and service-record details.
- **Manage availability:** Show when appointments can be booked or when goods
  can be supplied.
- **Manage customer work:** Use the conversation to follow each request from
  the first question to the final result.
- **Reach people through local groups:** Let chosen local groups show a listing
  to people who are looking for it.

### For local groups

- **Welcome members:** Check new businesses and give approved members a clear
  sign that customers can recognise.
- **Maintain local listings:** Help members' listings appear in local search
  and remove listings that no longer belong in the group.
- **Set and apply group rules:** Suspend or remove members who break the
  group's rules.
- **Share updates and knowledge:** Send announcements, guides, and important
  local information to members.
- **Link with nearby groups:** Maintain relationships with other local groups
  as the basis for future shared discovery, while each group keeps its rules.

### Extra tools in conversations

- **Attachments:** Share files and images when they help a conversation.
- **Reminders:** Create a reminder for an appointment, task, or follow-up.
- **Polls:** Ask a group a simple question and collect responses in the chat.
- **Order and service cards:** Add clear progress cards to a conversation so
  everyone can see what is happening next.
- **Share important proof safely:** Send proof of membership, permission, or
  identity only to the people who need to see it.

---

## Participant Journey

This section traces the sequence of actions that each participant performs when
using RoyM. All services, protocols, and data stores are treated as black
boxes. The focus is on *who does what, and when*.

The journey follows the order in which participants come into the picture:
providers first, then SynOrg owners, then consumers.

| Order | Participant | Why they come first |
|---|---|---|
| 1 | Provider | Must exist before anything can be found or bought |
| 2 | SynOrg Owner | Connects providers to discovery and adds trust (runs discovery pods on own node in the basic setup) |
| 3 | Consumer | Needs providers and a SynOrg in place to search and buy |

The Discovery Volunteer role appears later as the network scales (see
"Scale-out: Discovery Volunteers" below).

### Phase 1 — Provider sets up

The provider is the first to arrive. Without providers, there is nothing to
discover or buy.

```
P1.  Provider installs a Syneroym substrate with the RoyM app
P2.  Provider creates their profile (name, description, picture)
P3.  Provider creates a service/product catalog
       — describes offerings, prices, service area, hours, stock
       — the catalog is a signed listing that proves authorship
         (signing proves who published it, not that it is trustworthy —
          trust evidence comes separately from SynOrg membership,
          recommendations, and payment history)
P4.  Provider configures availability (appointment slots or supply schedule)
P5.  Provider is now live but not yet discoverable through any SynOrg
       — can still be found by direct contact or recommendation sharing
```

### Phase 2 — SynOrg Owner establishes a local group

The SynOrg owner creates the organizational layer that connects providers to
consumers through curated, trusted local search. In the basic setup, the
SynOrg runs discovery pods on its own node.

#### SynOrg setup

```
S1.   SynOrg Owner installs a Syneroym substrate with the RoyM app
S2.   SynOrg Owner creates the SynOrg (name, rules, area, categories)
S3.   SynOrg Owner configures discovery on their node
        — enables discovery pod roles on their own substrate
        — the node stores index data and answers search queries
        — the node can also act as a mailbox for offline message delivery
```

#### Provider joins a SynOrg

```
S4.   Provider requests to join a SynOrg
S5.   SynOrg Owner reviews the provider (vetting, rule check)
S6.   SynOrg Owner approves the provider
        — issues a signed membership credential to the provider
S7.   Provider's catalog listings are published to the SynOrg's discovery
        — listings carry the membership proof as trust evidence
S8.   Provider can receive SynOrg announcements, guides, and updates
```

#### SynOrg Owner's everyday work

```
S9.   SynOrg Owner reviews a newly published listing from a member
        — checks that descriptions, prices, and area are accurate
        — confirms the listing follows group rules
S10.  SynOrg Owner shares updates and local knowledge with members
        — announcements, guides, seasonal information
S11.  SynOrg Owner notices a listing that violates group rules
        — contacts the member to resolve the issue
        — if unresolved, suspends the member
        — suspended member's listings disappear from the SynOrg's search
S12.  SynOrg Owner receives a consumer complaint forwarded from a conversation
        — reviews the complaint details and the transaction history
        — contacts both the consumer and the provider
        — takes action according to group rules (warning, suspension, removal)
S13.  SynOrg Owner removes a member who repeatedly breaks rules
        — revokes the membership credential
        — all of that member's listings disappear from the SynOrg's search
```

### Phase 3 — Consumer arrives

The consumer is the last participant to need things in place. Providers and
SynOrgs must exist before search and transactions work.

#### Setup

```
C1.   Consumer gets access to RoyM:
        C1a.  Installs a Syneroym substrate with the RoyM app (full access)
              — or —
        C1b.  Accesses a SynOrg's client gateway through a browser
              (limited to provider discovery and basic chat, no local data)
C2.   Consumer creates their profile (name, description, picture)
C3.   Consumer adds one or more SynOrgs to search through
```

> **Note:** How a consumer discovers which SynOrgs exist is out-of-band:
> word-of-mouth, a shared link, a recommendation from a contact, or a
> well-known directory. Which SynOrg to trust is the consumer's own decision,
> based on signals such as who recommended it, how many providers it has, and
> its reputation.

#### Finding a provider

This is where RoyM's most distinctive capability shows: distributed search
across independent installations without a central index.

```
C4.   Consumer enters an intent in natural language
        — e.g. "plumber near me", "fresh bread nearby"
C5.   Consumer's node interprets the request
C6.   The request is sent to the consumer's configured SynOrg(s)
C7.   Each SynOrg routes the query to its discovery pods
        — pods may be on the SynOrg's own node or on volunteer nodes
        — pods on different installations can answer if they hold
          matching listings (this is cross-installation search)
C8.   Discovery pods return matching listings to the SynOrg
C9.   The SynOrg combines results and returns them to the consumer
C10.  Consumer sees results: provider pages with offerings, area, hours
        — results may include providers from different installations
C11.  Consumer checks trust evidence on results
        — membership proof (provider belongs to a SynOrg)
        — recommendations from contacts
        — payment proof history (when available)
        — trust evidence is verified by the consumer's own node,
          not by the SynOrg or the discovery pods
```

#### Engaging a provider

The provider may be on the same installation or a different one. The
experience is the same — messages are end-to-end encrypted regardless.

```
C12.  Consumer starts a chat from a listing
        — asks questions, explains the need
C13.  Provider responds in the same conversation
        — both see this in their unified inbox
        — if the provider is offline, the message waits in the consumer's
          outbox (or a mailbox node if configured) until the provider
          is reachable
C14.  Provider sends a quote card in the chat
        — shows work description, price, terms
C15.  Consumer reviews the quote
        — accepts, rejects, or requests changes inside the chat
```

#### Booking and ordering

```
C16.  Consumer books an appointment (chooses from provider's open slots)
        — or —
C16'. Consumer places an order (agrees on goods, quantity, price, delivery)
C17.  Both sides see a progress card in the conversation
        — status moves through: request → confirmed → in-progress →
          delivered → completed
```

#### Payment

```
C18.  Provider sends a payment link in the chat
        — links to an external payment service (card, bank transfer, etc.)
C19.  Consumer completes payment using the external service
C20.  Both sides confirm payment in the conversation:
        — each party provides their confirmation and whatever proof
          they have (screenshot, transaction ID, etc.)
        — the other party accepts or rejects the confirmation
        — once both accept, a mutual payment receipt is recorded
          in the conversation
```

#### Fulfilment and completion

```
C21.  Provider marks the work or delivery as complete
C22.  Both sides sign a fulfilment receipt in the conversation
        — this is a mutually signed record that the agreed work was done
        — combined with the payment receipt, both sides now have a
          complete, verifiable transaction record
C23.  Consumer can recommend the provider to contacts
        — shares a recommendation in a private chat
```

### Phase 4 — Ongoing everyday use (all participants)

Once set up, all participants share a common set of everyday features.
Features are grouped by priority: **core** features are needed for the basic
demo; **polish** features improve the experience but are not required to
show RoyM's value.

#### Core features

**Unified inbox**

```
E1.   All participants see everything in one inbox:
        — personal chats
        — group chats
        — customer/provider conversations
        — order and booking updates
```

**Messaging**

```
E2.   Send text, photos, videos, files, and other attachments
E3.   Reply to a specific message
E4.   Forward a message, file, or update to another conversation
E5.   Edit or delete sent messages
E6.   Send messages to offline people — delivered when they come back online
E7.   Use RoyM on multiple devices — chats stay in sync
```

**Private group chat** — a priority showcase feature: group messaging where
only members can read the messages, with no central chat server involved.

```
E8.   Create or join group chats (family, team, neighbourhood)
E9.   Group messages are private — only members can read them
E10.  Messages arrive consistently even when delivery order varies
```

**Contacts**

```
E11.  Add and manage contacts and favourites
E12.  Share contact details with others
E13.  Share provider recommendations in private chat
```

**Transaction cards**

```
E14.  Use order and service progress cards in conversations
E15.  Share proof of membership or permission (only to people who need it)
```

#### Polish features

These features improve everyday use but are not needed for the core demo.

```
E16.  Start side threads from a message (like Discord or ChatGPT threads)
E17.  See typing indicators and read receipts (when the other person allows)
E18.  Send a broadcast update to a chosen list of contacts
E19.  Set reminders for appointments, tasks, or follow-ups
E20.  Create polls in group chats
```

### Scale-out: Discovery Volunteers

In the basic setup (Phase 2), the SynOrg runs discovery pods on its own node.
As the network grows, dedicated volunteers can take on discovery and mailbox
roles to distribute the load.

```
V1.  Volunteer installs a Syneroym substrate with the RoyM app
V2.  Volunteer enables extra substrate roles:
       — Discovery pod: stores index data, answers search queries
       — Mailbox: stores messages for offline recipients
V3.  Volunteer's node is now available as a discovery pod
       — it does not choose which categories or areas to handle
       — what the pod stores and serves is determined by the
         organizing SynOrg's configuration (hash ring position,
         leaf index vs. proxy role, visibility, permissions)
V4.  The volunteer's node is associated with a SynOrg:
       — the SynOrg Owner could be configured in the volunteer,
         with automated checks to establish the relationship
       — exact mechanism to be designed later
V5.  The volunteer's node now receives listings and answers queries
       on behalf of the SynOrg, reducing load on the SynOrg's own node
```

### Interaction Map

The diagram below shows how participants interact. The key routing model is:
Consumer → SynOrg → Discovery Pods → results back to SynOrg → Consumer.
The consumer never queries discovery pods directly.

```
                          ┌─────────────────────────┐
                          │        SynOrg            │
                          │                          │
                          │  • member management     │
                          │  • discovery endpoint    │
                          │  • rules & moderation    │
                          │  • announcements         │
                          │                          │
                          │    ┌─────────────────┐   │
                          │    │  Discovery Pods  │   │
                          │    │  (own node or    │   │
                          │    │   volunteer      │   │
                          │    │   nodes)         │   │
                          │    └─────────────────┘   │
                          └──────┬──────────┬────────┘
                 credential,     │          │    routes queries,
                 announcements   │          │    returns results
                                 │          │
               ┌─────────────────┘          └──────────────────┐
               │                                               │
               ▼                                               ▼
  ┌──────────────────┐                            ┌──────────────────┐
  │    Provider      │                            │    Consumer      │
  │                  │◄──────────────────────────▶│                  │
  │  • catalog       │   chat, quotes, orders,    │  • search        │
  │  • availability  │   payment, fulfilment      │  • chat          │
  │  • fulfilment    │   (direct, end-to-end      │  • book / order  │
  │                  │    encrypted)               │  • pay           │
  └──────────────────┘                            └──────────────────┘
```

---

## Priority showcase features

These are the first end-to-end demonstrations of what RoyM and Syneroym make
possible. They take priority over familiar details such as profile pictures.

1. **Distributed local search:** Find a nearby service without a central index;
   local groups route the search to the right places and combine the results.
2. **One inbox:** See social chats, local-group chats, and active customer or
   order conversations in one place.
3. **Structured listings:** Publish services and goods in a consistent format,
   including booking, delivery, location, and other relevant details.
4. **Interactive order cards:** Negotiate a quote, confirm an order, and follow
   its progress through cards in the same conversation.
5. **Private group chat:** Demonstrate group messaging with centralized key management,
   or MLS and Gossip DAG delivery, so members can communicate without a central server.
6. **Offline direct messages:** Send a message or order request while a
   provider is offline; it stays in the sender's outbox or receiver mailbox
   until delivery works.
7. **Trust and payment proof:** Add verified external-payment proof to a chat,
   completing the shared record of a customer-provider interaction.

### Showcase scenarios

These scenarios make the priority showcase features concrete. Each shows a
specific capability working end-to-end.

#### Scenario A: Distributed local search

Shows that search works across independent installations with no central
index.

```
A1.  Consumer on installation X searches "electrician near me"
A2.  The request goes to the consumer's configured SynOrg (on installation Y)
A3.  The SynOrg routes the query to its discovery pods
A4.  One pod holds listings from a provider on installation X
     Another pod holds listings from a provider on installation Z
A5.  Both pods return their matching results to the SynOrg
A6.  The SynOrg combines the results and sends them to the consumer
A7.  The consumer sees providers from two different installations
       in one result list, with trust evidence for each
```

#### Scenario B: Offline direct message and delivery

Shows that messages reach their destination even when the recipient is
offline, with no central message server.

```
B1.  Consumer sends a booking request to a provider who is currently offline
B2.  The message is stored in the consumer's local outbox
       (or forwarded to a configured mailbox node)
B3.  Hours later, the provider comes online
B4.  The message is delivered — the provider sees the booking request
B5.  The provider responds — the consumer sees the response in their inbox
```

#### Scenario C: Multi-device sync

Shows that a participant can use RoyM on multiple devices with consistent
state.

```
C-i.   Provider receives a booking notification on their phone
C-ii.  Provider opens RoyM on their laptop, sees the same booking
C-iii. Provider responds to the booking from the laptop
C-iv.  The response appears on the phone too
C-v.   Consumer sees one consistent conversation thread regardless
         of which device the provider used
```

#### Scenario D: Complete service transaction with trust proof

Shows the full trust chain: membership proof, transaction cards, payment
proof, and fulfilment proof in one conversation.

```
D1.  Consumer finds a provider whose listing carries SynOrg membership proof
D2.  Consumer chats with the provider and receives a quote card
D3.  Consumer accepts the quote — a booking progress card appears
D4.  Provider completes the work, sends a payment link
D5.  Consumer pays — both sides confirm payment with their proof
D6.  Both sides sign a fulfilment receipt
D7.  The conversation now contains: membership proof, quote, booking,
       payment receipt, and fulfilment receipt — a complete verifiable record
```

#### Scenario E: Private group chat

Shows group messaging where only members can read messages, with no central
chat server.

```
E-i.   SynOrg Owner creates a group chat for member providers
E-ii.  Members join the group — encryption keys are established
E-iii. A member sends a message — only group members can read it
E-iv.  The message spreads between members directly, not through a
         central server
E-v.   A new member joins — they can read new messages but not old ones
E-vi.  A member is removed — they can no longer read new messages
```

---

## Technical Design & Architecture

This section explains how RoyM provides the experience above. It is written for
software engineers who know common web, distributed-systems, and security
concepts, but do not yet know Syneroym.

### Terms used in this section

- **Syneroym node:** A running Syneroym installation, on a person's device or
  on infrastructure they have chosen.
- **Service:** An independently run part of an application with a defined API,
  such as chat, a business listing, or search.
- **SynApp:** A Syneroym application definition that groups several services
  into one product. It is the deployment and management boundary for RoyM.
- **Capability token:** A signed, limited permission. It says who may perform
  which action on which resource, and can be passed on with fewer permissions.
- **Gossip DAG:** A graph of group messages that are shared between members.
  Its links to earlier messages let clients order delayed messages consistently.
- **CRDT:** A data structure that merges compatible changes from multiple
  devices without a central lock or a fixed delivery order.
- **Local group:** An independent group that can run services for its members;
  the earlier documents call this a SynOrg or an aggregator.

### Packaging and deployment

- RoyM can run as a WebAssembly SynApp: a portable module executed by Syneroym
  with access only to the services it has been given.
- It can also be compiled into the Syneroym binary for installations that want
  it built in, selected through build feature flags. Both forms use the same
  RoyM code and the same service APIs.
- The host provides `syneroym:messaging`, `syneroym:data-db`, and
  `syneroym:blob-store` for messages, encrypted structured data, and files.
  RoyM does not depend on direct access to another app's database.

### Service boundaries and permissions

- RoyM is both a provider and a consumer of services. It exposes a chat API to
  other SynApps and calls listing or local-group services through their APIs.
- A cross-app `Bind` is RoyM's declared dependency on another app's service.
  It identifies the service RoyM may call instead of giving ambient access.
- Calls between services carry the caller's identity and a capability token.
  The receiving service verifies both before it permits an operation.
- In Syneroym, these capability tokens follow the UCAN format. The important
  property is scope: a plugin can receive permission for one chat or action,
  not unrestricted access to the user's data.

### Private messaging and group delivery

- A one-to-one chat establishes shared secret keys, then changes them as
  messages are exchanged. It uses X3DH and Double Ratchet through
  `libsignal-protocol-rust`, the Signal-style messaging design.
- A group chat uses Centralized Multicast Key Management by group Owner using
  a Logical Key Hierarchy (LKH). Alternatively MLS (RFC 9420) through
  `openmls` to change group keys as people join or leave. Each group member
  can read its messages; relays cannot. Need to freeze on the approach.
- Group messages spread between members as a Gossip DAG rather than passing
  through one central chat server. Links to earlier messages give clients a
  consistent order when deliveries arrive in a different order.
- Connections to relays and coordinators use normal transport encryption, but
  those systems do not have the message keys. They can forward a message but
  cannot read its contents.

### Offline delivery and multiple devices

- Before sending a direct message, RoyM stores it in a durable local outbox.
  If the recipient is offline, the sender retries direct delivery later.
- Like any service has a home coordinator-relay, it could also have a home mailbox.
  Senders can then send messages to this mailbox if the receiver is offline.
  Receiver can retrieve from its configured mailbox when it comes online.
- A person may use more than one node. One chosen primary node is the preferred
  destination and makes the final decision when device changes conflict; it
  merges compatible changes with CRDTs from the other devices' local logs.
- Typing indicators and read receipts are short-lived signals. They use a
  separate `syneroym:messaging` publish-and-subscribe channel and are not part
  of durable message delivery.
- Message removal uses a durable deletion record and the relevant encryption
  keys are no longer available for reading the removed content. Retention rules
  decide when remaining stored data is deleted.

### Cards and plugins

- A card in a chat is versioned, typed JSON data. The client uses the version
  and type to render a quote, poll, booking, or progress view safely. The card
  can act as a container and renderer for multiple messages too.
- A plugin receives only the capability tokens it needs for the chat context or
  service it works with. It cannot inspect unrelated conversations by default.

### Distributed Intent Resolution and Discovery Fabric

The Discovery Fabric is RoyM's distributed search system. It helps a user turn
a request such as "plumber near me" into a useful set of local business
listings, without relying on one central search database.

- **Business-neutral data model:** The search system does not need separate
  code for plumbers, doctors, or shops. It works with signed listings,
  searchable attributes, user requests, and supporting proof.
- **Signed listings:** A business listing is a signed publication. It contains
  searchable facts, such as category and service area, plus evidence such as
  membership proof and payment proof where relevant.
- **One protocol, different roles:** The same discovery protocol runs on every
  Syneroym node. Configuration decides whether a node only sends searches,
  stores an index, answers searches, or does more than one of these jobs.
- **Predictable listing placement:** Category and location form a routing key.
  Rendezvous hashing, a deterministic selection algorithm, maps that key to
  the same suitable index nodes without a central directory deciding where the
  listing belongs.
- **Small routing advertisements:** Nodes tell connected peers what categories
  and areas they can help with, how to reach them, and basic trust details.
  They exchange these small summaries instead of copying every listing
  everywhere.
- **Step-by-step search:** A node interprets the request, checks the summaries
  it knows, asks the relevant peers, combines their answers, and returns the
  results. A local group can therefore ask other groups when it cannot answer
  a request itself.
- **Finding is separate from trust:** The network helps find possible matches;
  it does not declare that they are trustworthy. The user's node verifies
  signatures, membership proof, expiry dates, and payment proof before it
  presents trust information as valid.

### Data ownership and failure recovery

- Each RoyM installation keeps chat history, contacts, and listings in its own
  encrypted SQLite databases. These records can be exported and moved to a
  different installation.
- A user's identity is separate from a node's network identity. Replacing or
  moving a node does not change who signed a message or authorised an action.
- Important local state changes are recorded so an interrupted update can
  recover after a crash. Durable outboxes keep retrying eligible work after a
  network outage.
- RoyM must not persist unencrypted user data to the filesystem. Encryption
  keys remain separate from the encrypted files and databases they protect.

---

## What is not part of the first release

- RoyM does not process payments itself or hold money between a customer and a
  business. It has no native ledger or integrated escrow; it only opens a
  payment service and keeps the resulting proof.
- RoyM does not yet calculate public ratings from past work or payments.
- RoyM does not yet include an automated assistant that searches, recommends,
  or completes tasks for people.
- The protocol for automatic discovery between independently run local groups
  is deferred to a later release; groups can prepare peer relationships now.
- Automated dispute resolution workflows are deferred; consumers can forward
  complaint details to a SynOrg Owner manually.
