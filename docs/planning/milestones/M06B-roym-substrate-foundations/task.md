# Milestone 6B: Roym Substrate Foundations (M06B-roym-substrate-foundations)

> **Provenance.** The second of the three sub-milestones M6 split into on
> 2026-08-13 while reviewing
> [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md)
> against the tree — **M06A** (the HTTP surface any app needs to be a web app,
> Complete 2026-08-17), **M06B** (this), **M06C** (the Roym product itself).
> Directory created 2026-08-18, after M06A closed.
>
> **What this milestone is.** The four capabilities Roym needs from the
> substrate that do not exist today, plus the packaging rule that makes them
> usable from both of Roym's builds. These are gaps **G1–G4** in the experience
> spec's [Substrate work required](../../../roym-integrated-experience-spec.md#substrate-work-required)
> section. Every one was verified against the tree again while writing this
> document; §"The gaps" records what is actually there.
>
> **What it is deliberately not.** Not the Roym product. No Conversation,
> Profile, Catalog, Transaction, or Directory service is built here — those are
> M06C, and they are the consumers this milestone exists to serve.

## Goal

By the end of M06B, a SynApp written once against host interfaces can be built
two ways — as a `wasm32-wasip2` component and linked into
`syneroym-substrate` — and both builds can hold a durable, ordered, encrypted
conversation with another person on another installation, on behalf of a person
the substrate can actually name, against services whose reachability and
publication are declared rather than incidental.

---

## Why this comes after M06A and before the product

1. **M06A proved the app surface; M06B proves the app's substrate.** M06A
   answered "can a component be a web app". M06B answers "can a component be
   *Roym*". The two questions are independent, which is why they are separate
   milestones.
2. **Every gap here is a hard blocker on product code, not a nice-to-have.**
   The experience spec's own [Packaging](../../../roym-integrated-experience-spec.md#packaging-one-source-two-builds)
   section states the constraint plainly: *"Roym cannot use any capability that
   does not exist as a host interface."* Two of the capabilities it needs do
   not exist. Product work cannot start around them.
3. **The largest single item in M6 lives here.** G1 — durable messaging — is
   named as such by the spec. Discovering its shape under product code, with
   four other services already built on top, is the expensive order.

---

## The gaps, verified against the tree

Re-verified 2026-08-18. Line references are to the tree at that date.

**Gap 1 — `syneroym:messaging` cannot carry a conversation.** The whole
interface is `publish` / `subscribe` / `unsubscribe` plus
`register-stream-protocol`, with a `handle-message` guest export
([messaging.wit](../../../../crates/wit_interfaces/wit/messaging/messaging.wit)).
There is no conversation, no recipient, no delivery state, no history, no key
handling. And it is not a matter of extending it:
[ADR-0013](../../../decisions/0013-p2p-messaging-architecture.md) §6 forbids it
by name — *"Durable message content and history … never depend on the
`syneroym:messaging` MQTT-style broker."* The broker MAY carry ephemeral UX
signals (typing indicators, an "arrived" nudge); it is never load-bearing.

What is missing underneath is larger than the interface. `grep` for `gossip`,
`GossipDag`, `X3DH`, or `DoubleRatchet` across `crates/` and `apps/` returns
**nothing**. Direct exchange, DAG sync, ordering, and group key distribution are
all unbuilt.

**Gap 2 — the outbox has no guest surface.** `crates/async_queue` is a complete
durable queue with a dead-letter queue ([ADR-0023](../../../decisions/0023-durable-async-primitives.md),
Accepted): `enqueue`, `claim_due`, `complete`, `fail`, `dead_letters`, `replay`
([lib.rs:312-696](../../../../crates/async_queue/src/lib.rs#L312)). None of it
appears in `crates/wit_interfaces/wit/` — the directory holds `app-config`,
`blob-store`, `control-plane`, `data-layer`, `host`, `http`, `messaging`,
`proxy`, and `supervisor`, and no queue. So a guest cannot read
`pending`/`delivered`/`failed`, which R1's acceptance test requires be visible
to the user.

**Gap 3 — the gateway cannot name the person asking.** It binds `127.0.0.1`
with a standing `TODO` saying so
([gateway.rs:175-177](../../../../crates/client_gateway/src/gateway.rs#L175)),
authenticates no client, and presents the **node's** own identity as the caller
DID for every proxied request
([gateway.rs:82-88](../../../../crates/client_gateway/src/gateway.rs#L82)). Two
consequences, both live: any local process can ask the substrate to sign as the
user, and on a shared node the substrate cannot tell which person is asking.

M06A made this sharper rather than leaving it theoretical. Its own comment
block records why
([gateway.rs:57-67](../../../../crates/client_gateway/src/gateway.rs#L57)): a
"deployed service" can now mean guest code branching on `caller.did`, so a guest
HTTP handler reached through this gateway sees the node's DID — identical for
every visitor, never an end user's. A2 labelled it `self-asserted` in the WIT
(`D-A2-12`) precisely so a guest cannot mistake it for a verified identity.
**B1 is what lets that label become an honest `verified`.**

**Gap 4 — visibility is incidental at two separate layers.**

*Publication.* [ADR-0018](../../../decisions/0018-service-record-visibility.md)
records that a service reaches the community registry if and only if a
pre-signed `registry_certificate` happened to be in its deploy manifest — so
"deployed but deliberately private" and "deployed and undiscoverable by
accident" are the same state from outside. **Accepted 2026-08-18.**

Smaller than it looks, because half of it already shipped. M06A slice A1 needed
a visibility value for a *different* question — whether asset bytes are readable
without a signature — and defined the ADR's three-valued enum on both sides
while it was there: `visibility` in
[control-plane.wit:64-72](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L64),
and `Visibility` in
[app_orchestration/src/models.rs:534-543](../../../../crates/app_orchestration/src/models.rs#L534),
both defaulting to `private` as the ADR argues. The WIT comment says so in as
many words. **What is left is `service-config.visibility` — the field the ADR is
actually about — and the publication path that reads it.**

*Resolution.* Across installations, a caller is refused. The gateway warns at
startup that without a pre-installed `resolve_ucan` token, app-scoped hostnames
"will be refused by any supervisor they reach", and that with only the same-node
gate they resolve "only for apps supervised by this node"
([gateway.rs:140-155](../../../../crates/client_gateway/src/gateway.rs#L140)).
The fix is [ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md)
§5's per-logical-service "open to all" declaration.

**Gap 5 — the dual-build shim does not exist.** Not one of the spec's G-numbers,
but the packaging rule that makes all four usable. D2 and D3 require one source
tree, two build targets, and *no* direct host-crate calls from the native build.
Nothing in the tree implements a host interface twice today; existing native
services call `syneroym-data-db` and friends directly, which D3 forbids Roym
from doing.

---

## Decisions

> **On identifiers.** `D-06B-n` are milestone-level decisions, matching the
> house `D-<scope>-<n>` shape. Bare `B1`–`B5` are **slices**, matching M06A's
> `A1`–`A5`. The two namespaces are separate; a reference to "B1" always means
> the slice.

| # | Decision | Why |
|---|---|---|
| D-06B-1 | **The Web entrypoint's native exemption is retired.** The experience spec's [Client contract](../../../roym-integrated-experience-spec.md#client-contract) gives two reasons the entrypoint is a native binary. **Reason 1 is now false** — M06A shipped blob-backed static assets (A1), a guest HTTP route target (A2), inbound WebSocket (A3), and proved all of it from a browser (A5). Reason 2 (one browser origin) is unaffected and still stands. So the entrypoint keeps existing as a single-origin aggregator, but it is a WASM component like everything else, and D2/D3 apply to it with no exemption. | M06A's own [task.md](../M06A-app-platform-surface/task.md) states this as the milestone's purpose: *"That exemption exists purely because a component cannot serve HTTP. Close the gap here and the exemption goes away instead of hardening into precedent."* Not recording the retirement is how it hardens anyway. The spec's client-contract section needs the corresponding edit. |
| D-06B-2 | **G2 folds into G1's interface. The queue gets no WIT package of its own.** | The spec already reasons this out: *"G1 is the better home, since message delivery state is what users actually see."* A separate `syneroym:queue` would make a guest correlate two interfaces to answer one user-visible question ("did my message send?"). A general queue surface can be lifted out later if a second consumer ever wants one. |
| D-06B-3 | **The dual-build shim ships before the new interface, and is proven against the interfaces that already exist.** | G1 is the largest new surface in M6. Designing it with only one of its two consumers in hand is how the shim becomes a retrofit and the native build becomes second-class — exactly what D3 exists to prevent. Proving the shim against `data-layer` / `blob-store` / `messaging` is cheap and needs no new design. |
| D-06B-4 | **M06B owns both halves of visibility. M05C slice S4 keeps only cross-app `Bind`.** B2 delivers ADR-0018's publication declaration *and* ADR-0022 §5's per-logical-service "open to all" resolution declaration. | They are one question — who may see and reach this service — split across two layers, and answering half leaves M06C blocked. The meta plan already separates S4's two halves and parks only the `Bind` half on its consumer gate (D-C-7); the visibility half's gate is now met, since Roym is that consumer. Leaving it in a milestone parked on a different gate is how it gets forgotten. **This requires editing M05C's S4 row**, not silently overriding it. |
| D-06B-5 | **Person identity at the gateway is a local session model. The bind stays `127.0.0.1`.** | D8 settles it: every participant installs a substrate, and there is no browser-only consumer path in the first release. Remote binding, TLS termination, and multi-tenant client auth are a hosted-gateway design with its own threat model. B1's job is to know *which local person* is asking, not to serve the internet. |
| D-06B-6 | **M06B builds its own throwaway fixture, not Roym.** A minimal two-party conversation fixture in `test-components/`, excluded from the workspace build graph, built both ways. | M06A's `miniapp-demo1-wasm` proved this works: a consumer cheap to throw away finds the gaps a real product would find later, at a fraction of the cost. Every gap it finds is a gap M06C would have found with four services already built on top. |

---

## Explicit non-goals

- **Third-party mailboxes.** A message waits in the sender's own outbox (D4,
  ADR-0013 §3). Mailboxes need their own ADR covering encryption, retention,
  abuse, deletion, and operator-visible metadata.
- **Multi-device sync / primary-substrate reconciliation.** ADR-0013 §2 is
  designed but explicitly out of the first release; the spec lists it under
  "Beyond the first release".
- **MLS.** Replaced by the owner-distributed per-epoch group key
  (ADR-0013 Amendment 1, D5). It stays available later as a swap of one module.
- **Attachments, voice, video, and chat polish** — threads, polls, reminders,
  read receipts, typing indicators, broadcasts.
- **A hosted or remotely-bound client gateway** serving browser-only consumers
  (D-06B-5, D8).
- **Cross-app `Bind`.** Stays M05C S4, parked on its own consumer gate.
- **The Distributed Matching Fabric (`[P2P-DSC]`).** M8.
- **The Roym product services.** M06C.

---

## Dependency gates

| Depends on | State |
|---|---|
| M06A A1–A5 (static assets, guest HTTP target, inbound WebSocket, browser suite) | **Complete (2026-08-17)** |
| M04A caller identity through native dispatch (ADR-0016) | Shipped |
| M04B FDAE policy / access control | Shipped |
| M05B async primitives — `crates/async_queue`, ADR-0023 | Shipped as a library; no WIT surface (that is G2) |
| M05C S1–S3 (registry record, signed topology document, gateway hostname scheme) | S1–S2 shipped; S3 substantially complete |
| ADR-0013 + Amendment 1 (messaging architecture, group key) | Accepted |
| ADR-0022 §5 (per-logical-service visibility) | Design accepted; **implemented by B2 (2026-08-18)** — `ServiceSpec.topology_visibility` / `PlannedService.topology_visibility` |
| ADR-0018 (declared service record visibility) | **Accepted 2026-08-18. Implemented by B2 (2026-08-18)** — §1/§4 in full, §2 (`--record-out`/`new_with_record`); §3 (peer-substrate known-records store) deferred, see [deferred-backlog.md](../../deferred-backlog.md) |

---

## Slices

| # | Scope | Status |
|---|---|---|
| **B1** | **Person identity at the client gateway (G3).** A local session model binding an authenticated local client to a person's identity, so the gateway presents that person's DID under an owner→node delegation instead of the node's own. Retires the `TODO(post-B0)` at [gateway.rs:46](../../../../crates/client_gateway/src/gateway.rs#L46) and lets A2's `self-asserted` `caller-auth` label become `verified` for this path. Bind stays `127.0.0.1` (D-06B-5) | **Complete (2026-08-18)** — [implementation plan](slice-b1-implementation-plan.md), [status & evidence](status.md) |
| **B2** | **Declared service visibility (G4), both layers.** ADR-0018 implemented: `service-config.visibility` plus a publication path that reads it, so publication becomes a declaration rather than a side effect of which flag was passed. The enum itself already landed with M06A A1. Plus ADR-0022 §5's per-logical-service "open to all" resolution declaration, so a caller on an unaffiliated installation is not refused without a pre-installed token (D-06B-4) | **Complete (2026-08-18)** — [implementation plan](slice-b2-implementation-plan.md), [status & evidence](status.md) |
| **B3** | **The dual-build shim (D2/D3).** One trait per host interface; two implementations — `wit-bindgen` guest bindings, and an in-process native shim linked into `syneroym-substrate` behind a Cargo feature. A fixture generic over them, built both ways, with one integration suite that runs against both. Proven against `data-layer`, `blob-store`, and `messaging` — interfaces that already exist (D-06B-3) | — |
| **B4** | **Durable messaging: interface and 1:1 delivery (G1 part 1, G2).** The `syneroym:conversation` host interface — conversations, direct delivery, delivery state, history — with outbox `pending`/`delivered`/`failed` folded in (D-06B-2). Layer 3 underneath: X3DH + Double Ratchet direct exchange, the sender's own outbox, strict direct delivery with no third-party buffering (D4). Durable content never touches the pub/sub broker (ADR-0013 §6) | B3 |
| **B5** | **Group delivery (G1 part 2).** Gossip DAG with epidemic routing and participant relays; total order by `(sender_timestamp, sender_did)`; offline catch-up pulled from any online peer; owner-distributed per-epoch group key with rekey on every join, every removal, and on a schedule. Membership changes are ordinary DAG entries (D5, ADR-0013 Amendment 1) | B4 |

**B1, B2, and B3 are independent** and can run in parallel. B4 needs B3 so the
largest new interface is designed against both builds from the first line. B5
needs B4 — the group key is distributed over B4's 1:1 channel, which is the same
dependency R4 declares on R1 in the product releases.

> **Why B3 is not last.** The tempting order is "build the capability, then make
> it work natively". D3 exists because that order produces a native build with
> shortcuts and a WASM build nobody exercises. Shipping the shim against three
> interfaces that already work costs a slice and removes the retrofit.

### Open design points for the slice plans

- **What the durable messaging WIT looks like.** Plain functions, or exported
  resources in the style `stream-types` already uses for
  `stream-cursor`/`stream-sink` (ADR-0014's resource mechanics)? A conversation
  handle is the obvious resource candidate, and the tree already has one
  working precedent for guest-exported resources. B4's plan should settle it
  against that precedent rather than inventing a third pattern.
- **Where the DAG is stored.** `data-layer` (structured, already
  DEK-encrypted), `blob-store` (content-addressed, right shape for immutable
  signed entries), or its own store. Bodies and entry metadata may want
  different answers. Whatever B5 picks must survive the restart test in the
  failure matrix.
- **What a person is bound to at the gateway.** A session token issued by a
  local login, the OS user from a Unix socket credential, or a client
  certificate. The `127.0.0.1` bind narrows the threat model a lot (D-06B-5) but
  does not by itself distinguish two processes run by two people on one machine
  — which is the case the spec names.
- **Whether the group key rides the 1:1 ratchet or its own channel.** ADR-0013
  Amendment 1 says the owner distributes it "over the 1:1 channel from Decision
  3". Reusing B4's ratchet is the cheap reading; whether key material should
  share a ratchet with message content is a question B5 should answer
  explicitly, not inherit.
- **How the native shim is selected.** A Cargo feature is stated in the spec's
  packaging table. Which crate owns the trait definitions — a new one, or
  `syneroym-sdk` — is open, and matters because both builds depend on it.

---

## Migration impact

- **A new WIT package**, `syneroym:conversation@0.1.0`. Additive, and
  deliberately not added to the `host-environment` world by default — a
  component that does not import it deploys exactly as before, matching how
  M06A added `syneroym:http`.
- **`syneroym:messaging` is unchanged.** B4 adds a package beside it; it does
  not extend, deprecate, or alter pub/sub. ADR-0013 §6 keeps the broker as a
  legitimate ephemeral side-channel.
- **A new declared visibility field** on the deploy manifest. Additive, but
  **the default changes behaviour**: today publication follows from whether a
  certificate was passed. B2's plan must state the migration rule explicitly and
  pick the safe default (undeclared = unpublished), since the alternative
  publishes records their owners never asked to publish. **This is loudest on
  the `svc deploy` path** (`--visibility public`/`internal` with no
  `--identity`/`--master` now fails at deploy, naming the fix) **and quietest
  on the app-deploy path**: `certify_placed_members` used to mint a record
  for every placed member unconditionally, so an app manifest written before
  this field existed now deploys successfully but publishes no member
  records, and the first cross-node call to an undeclared member fails later
  with *"No valid Iroh mechanism found"*. `validate_plan_visibility`
  (`D-B2-14`) catches the one contradiction that is statically detectable —
  a cross-substrate dependency on a `private` member — at compile/submit
  time; an app whose members simply omit the field and never depend on each
  other across substrates deploys with no warning and stops resolving
  cross-node. Every manifest that needs cross-node resolution must declare
  `visibility = "internal"` (`D-B2-15`).
- **A new caller-identity shape at the gateway.** The `caller-auth` label a
  guest sees on the gateway path changes from `self-asserted` to `verified`.
  Guests that branch on it see a value they already had to handle.
- **No wire-format change** to endpoint records, topology documents, or gateway
  hostnames.

---

## Reference scenario (runnable)

```
1.  Build the conversation fixture for wasm32-wasip2 AND into the substrate
2.  Run the same integration suite against both builds -- identical results
3.  Two substrates, two people, each with an owner->node delegation
4.  Person A logs in locally; the gateway presents A's DID, not the node's
5.  A second local process asks the gateway to sign as A  -> refused
6.  A messages B while B is offline    -> state `pending`, held in A's outbox
7.  Restart both processes             -> still `pending`, nothing lost
8.  B comes online                     -> delivered, state `delivered` on A
9.  A creates a group, adds B and C; owner distributes the epoch key
10. All three post from skewed clocks  -> byte-identical transcripts
11. C goes offline, misses messages, returns -> pulls the gap from B
12. Owner removes C, rekeys           -> C cannot read anything after removal
13. A service declares `topology_visibility: open` and a registered
    `visibility` (`internal`/`public`) -> a caller on install Z resolves it
    with no pre-installed token
```

Step 2 is the one to watch: a test that passes on one build and fails on the
other is a bug in the shim, not in the test.

---

## Failure and security matrix

| # | Case | Expected |
|---|---|---|
| 1 | A local process that is not the person's authenticated client asks the gateway to sign on their behalf | Refused. This is G3's whole point — today it succeeds |
| 2 | Two people use one node; each asks "who am I" through the gateway | Each gets their own DID, never the node's and never each other's |
| 3 | A message is displayed as `delivered` before the recipient acknowledged it | Impossible by construction. R1's acceptance test states it: *"never shown as delivered while pending"* |
| 4 | Process restart on both sides with a message in flight | Message survives on both sides; state is unchanged, not reset and not double-sent |
| 5 | Recipient is never reachable | Message stays `pending` in the sender's own outbox indefinitely; no third party ever holds it (D4) |
| 6 | Durable message content is routed through the pub/sub broker | Impossible — provable by a test, not by inspection. ADR-0013 §6 forbids it by name |
| 7 | A removed member decrypts messages sent after the removal | Fails. A joiner decrypting messages from before the join also fails |
| 8 | A group key reaches a party absent from the membership history | Impossible — membership changes are ordinary DAG entries every member observes, so the owner cannot admit a reader silently |
| 9 | Two members post at the same instant from heavily skewed clocks | Both transcripts sort identically. Clock skew affects displayed accuracy, never cross-peer ordering |
| 10 | A scheduled rekey runs with membership unchanged | The key still changes. This is what bounds an undetected compromise |
| 11 | A service is deployed with no visibility declaration | Not published and not resolvable across installations. Absence must not mean "publish it" |
| 12 | Unbounded outbox or DAG growth on one conversation | Bounded per conversation; exhausting it degrades that conversation, not the node — the same principle M06A's row 8 applies to SSE and guest HTTP |
| 13 | The native shim and the WASM build disagree on any interface | The shared suite fails. The spec's own rule: *"A test that passes on one and fails on the other is a bug in the shim"* |

---

## Measurable exit criteria

1. The conversation fixture builds **both ways** — `wasm32-wasip2` component
   and linked into `syneroym-substrate` — from one source tree, with no direct
   host-crate call in the native build (D3).
2. One integration suite runs against both builds and passes identically.
3. The client gateway presents a **person's** DID under delegation, and a
   second local process cannot obtain it.
4. A 1:1 message survives a process restart on both sides, and its
   `pending`/`delivered`/`failed` state is readable by guest code through the
   host interface.
5. A message is never reported `delivered` before acknowledgement — asserted by
   a test that holds the recipient offline.
6. No durable message content traverses `syneroym:messaging` — shown by a test
   that asserts on the broker's own traffic, not by inspection.
7. A group of at least three members converges to **byte-identical**
   transcripts after sync, with messages posted from deliberately skewed clocks
   and with no coordinator reachable.
8. A removed member cannot read messages sent after the removal; a joiner
   cannot read messages from before the join; a scheduled rekey with stable
   membership changes the key.
9. An offline member returns and pulls the gap from an online peer, converging
   to the same transcript.
10. A service that declares no visibility is neither published nor resolvable
    across installations; one that declares `topology_visibility = open`
    *and* a registered `visibility` (`internal`/`public`) is resolvable by a
    caller on an unaffiliated installation **with no pre-installed token**.
    (`topology_visibility = open` alone, with `visibility = private`, is a
    detectable mistake refused at compile/submit time — `D-B2-14`(b) — since
    it would name members nobody can dial.)
11. Every row of the failure and security matrix has a test.
12. `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets
    --all-features`, `cargo test --workspace`, and `mise run test:e2e` are
    clean.

**Stretch, not required:** the fixture's WASM build running under the M06A
browser suite, proving a conversation from a real browser. That is genuinely
M06C's job, but a pass here would retire the last doubt about D-06B-1.

---

## Documents this milestone edits

Recorded here so nothing is missed, the way M06A's §7 documentation pass nearly
was. The upstream edits landed with this document, on sign-off; the per-slice
ones are owed as each slice completes.

**Done 2026-08-18, with this document:**

| Document | Edit |
|---|---|
| [ADR-0018](../../../decisions/0018-service-record-visibility.md) | Proposed → **Accepted**, with a note that its enum shipped early via M06A A1 |
| [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md) | D6 revised; client-contract reason 1 struck; the D2/D3 exemption retired (D-06B-1); G1–G4 given slice owners; G4's two halves re-pointed |
| [M05C task.md](../M05C-logical-discovery-overlay/task.md) | S4's row narrowed — the visibility half moved here as B2; S4 keeps cross-app `Bind` only (D-06B-4) |
| [meta-implementation-plan.md](../../meta-implementation-plan.md) | M06A marked A1–A5 Complete; M06B given its directory link and slice list; the "prerequisite substrate gaps" note given slice owners |
| [deferred-backlog.md](../../deferred-backlog.md) | The four G1–G3 rows re-targeted from `M06`/`M06B` to the specific slice that closes each |

**Owed as slices land:**

| When | Edit |
|---|---|
| B1 completes | The two gateway-identity rows in [deferred-backlog.md](../../deferred-backlog.md) §1 and §3 move to "Recently resolved"; the `TODO(post-B0)` comment at [gateway.rs:46](../../../../crates/client_gateway/src/gateway.rs#L46) is deleted |
| B2 completes | ADR-0018's own status note updated from "implemented by B2" to the shipped state |
| B4 completes | The durable-messaging and outbox rows move to "Recently resolved" |
| Each slice | `status.md` in this directory — created with B1, not before |
