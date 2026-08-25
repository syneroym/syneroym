# Milestone 6C: The Roym Product (M06C-roym-product)

> **Provenance.** The third and last of the three sub-milestones M6 split into
> on 2026-08-13 while reviewing
> [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md)
> against the tree — **M06A** (the HTTP surface any app needs to be a web app,
> Complete 2026-08-17), **M06B** (the substrate capabilities Roym needs,
> Complete 2026-08-24), **M06C** (this). Directory created 2026-08-24, after
> M06B's last slice closed.
>
> **What this milestone is.** The Roym product itself, built as one SynApp of
> six services on top of the foundations M06A and M06B shipped, following the
> spec's [First release scope](../../../roym-integrated-experience-spec.md#first-release-scope)
> table — the only normative scope statement in that document. Four releases:
> **R1** a usable local guild, **R2** the transaction vertical, **R3**
> cross-installation trust, **R4** private group chat. The journey and
> technical sections of the spec explain that table; they do not extend it,
> and neither does this document.
>
> **What it is deliberately not.** Not more substrate. Where a capability is
> genuinely missing, this document names it as a gap and gives it a slice —
> but the milestone is measured by whether a person can find a provider,
> agree on work, pay for it, and record that it was done, across three
> separate installations.

## Goal

By the end of M06C, three people on three separate Syneroym installations —
a consumer, a provider, and a SynOrg owner — can complete the whole loop the
spec describes. The consumer finds the provider through the SynOrg's
directory and verifies the evidence on their own node, not on the
directory's word. They talk. The need becomes a signed quote and a signed
agreement. The booking is decided by one named writer. Payment happens
outside Roym and both sides record what they saw, honestly labelled. The
work is signed off. Every one of those records is signed, versioned, and
append-only, and either party can export the whole thing and restore it on a
clean machine. Separately, a private group of at least three members holds a
conversation with no server in the path.

All of it through one JSON-RPC API served from one origin, from a SynApp
that builds both ways — `wasm32-wasip2` component and linked into
`syneroym-substrate` — with no direct host-crate call from the native build.

---

## Why this comes after M06B

1. **Every substrate gap the spec named is closed.** The spec's
   [Substrate work required](../../../roym-integrated-experience-spec.md#substrate-work-required)
   section lists G1–G4. M06B slices B1–B5 closed all four, plus the D2/D3
   dual-build shim the section did not number. Nothing in R1–R4 is now
   waiting on an interface that does not exist — the remaining gaps (below)
   are smaller and are M06C's own to close.
2. **The largest single item is already behind us.** G1, durable messaging,
   is named by the spec as the biggest thing in M6. It shipped as B4 and B5.
   Group chat is R4 here because it needs R1's 1:1 flow at the *product*
   level, not because the protocol is still open.
3. **The order inside M6 was chosen so the expensive discovery came first.**
   M06A found the HTTP gaps against a throwaway fixture; M06B found the
   messaging and identity gaps against another one. Both would have been
   found here, with five product services already built on top, at many
   times the cost.
4. **This is where the foundations get their first honest test.** A fixture
   proves an interface works. A product proves the interface was the right
   shape. If any M06B decision was wrong, M06C is where it shows.

---

## The gaps, verified against the tree

Verified 2026-08-24 by reading the tree, not by assuming. Line references are
to that date.

**Gap 1 — nothing lets an app sign a record.** Nine of the spec's record
types are signed
([Records](../../../roym-integrated-experience-spec.md#records-and-what-each-one-proves)):
`listing`, `membership-credential`, `revocation`, `request`, `quote`,
`agreement-receipt`, `payment-acknowledgement`, `fulfilment-receipt`,
`moderation-decision`. No host interface can produce one. A grep for `sign`
across every `.wit` in `crates/wit_interfaces/wit/` returns only prose in doc
comments and `blob-store`'s unrelated `signed-url`. `syneroym:vault` is one
function — `reveal: func(key: string) -> result<list<u8>, vault-error>`
([vault.wit](../../../../crates/wit_interfaces/wit/host/deps/vault/vault.wit))
— and the native-dispatch verb table confirms the same set from the other
side: `data-layer`, `vault`, `app-config`, `blob-store`, `messaging`,
`conversation`, and nothing else
([synsvc_native.rs:1620-1625](../../../../crates/control_plane/src/synsvc_native.rs#L1620)).
The signing primitives exist, but only native-side —
`Identity::sign`/`sign_json`/`derive_service_identity`
([keys.rs:195-240](../../../../crates/identity/src/keys.rs#L195)) — and D3
forbids Roym from calling them directly.

*Verification is a different question and is not blocked.* Checking an
ed25519 signature needs only a public key, so a guest can vendor the crypto
and do it itself, which is exactly what the spec's "the consumer's own node
verifies" rule requires. It is **signing** that has no path, because the
private key belongs in the substrate (D7) and must not be handed out.

**Gap 2 — the dual-build shim covers four interfaces; Roym needs at least
seven.** `AppHost` is
`AppDataLayer + AppBlobStore + AppMessaging + AppConversation`
([lib.rs:36-43](../../../../crates/app_host/src/lib.rs#L36)). Three of
Roym's own needs are outside it:

- **`syneroym:proxy`.** Every Roym service reaching a sibling uses
  `call-target::dependency`, and a consumer reaching a directory it chose
  after deployment uses `call-target::service(<did>)`
  ([proxy.wit:25-28](../../../../crates/wit_interfaces/wit/proxy/proxy.wit#L25)).
  The spec's own service-boundaries section says Roym needs both shapes.
  Neither has a trait, so the native build has no way to make a call at all.
- **`syneroym:http`'s `incoming-handler` / `websocket-handler`.** This is how
  the Web entrypoint receives a request. The WASM build exports it; the
  native build has no equivalent.
- **`syneroym:app-config`** and **`syneroym:vault`**, both small, both used
  by any real service for its own configuration and secrets.

**Gap 3 — conversation history can be read out, but never written back or
removed.** `syneroym:conversation`'s whole verb list is `open-direct`,
`conversations`, `send`, `history`, `delivery-status`, `outbox`, `retry`,
`create-group`, `add-member`, `remove-member`, `members`,
`membership-history`, `sync-now`
([conversation.wit](../../../../crates/wit_interfaces/wit/conversation/conversation.wit)).
There is no import, no insert, and no delete. Two consequences:

- R1's identity row ("Restore on a clean node reproduces identity and
  **history**") and R2's export row cannot be met through the host store,
  because nothing can put history back.
- The spec's [message deletion](../../../roym-integrated-experience-spec.md#messaging)
  rule ("Deleting writes a durable deletion record and removes the local
  copy") has no verb to remove the local copy with.

**Gap 4 — there is no safety primitive above the prekey ceiling.** A grep for
`rate_limit`/`RateLimit` across `crates/*/src` matches exactly one file,
`crates/conversation/src/store.rs`, and only for prekey-bundle requests. The
conversation interface has no block list, no per-sender contact ceiling, and
no admit/reject hook: `guest-api.on-message` is a notification delivered
*after* the message is already durably stored — the WIT says so in as many
words ("A component that exports neither still receives messages durably --
they are in the store either way"). So R1's safety row ("A blocked sender's
messages never reach the recipient's **inbox**") is a statement Roym makes
about *its own* inbox, not about the host store underneath it. That is
honest and workable, but it has to be said out loud rather than assumed.
[deferred-backlog.md](../../deferred-backlog.md) §10's `[PRD-SAF]` row is
aimed at exactly this and is still open, targeted at `M06 (R1)`.

**Gap 5 — a conversation names a service, not a person.** `peer-address` and
`author` are routing service ids, deliberately (`D-B4-5`, and
[ADR-0013](../../../decisions/0013-p2p-messaging-architecture.md) §3's own
implementation note). A person's Master DID cannot distinguish two services
one owner runs, and person→substrate resolution needs ADR-0013 §2's Primary
Substrate designation, which is unbuilt. So the person↔address mapping is
Roym's own to own, in Profile & Contacts. The journey step where a consumer
starts a conversation from a listing (the spec's own journey step C10) is
precisely where this bites: a
listing must carry the provider's Conversation *service* address, not only
their identity.

**Gap 6 — no backup, restore, or export machinery exists anywhere.**
`syneroym-data-keystore`'s entire public surface is `new`, `kek_is_loaded`,
`inject_kek`, `clear_kek`, `generate_dek`, `load_dek`, `rotate_kek`
([key_store.rs:65-197](../../../../crates/data_keystore/src/key_store.rs#L65)).
There is no encrypted export of a KEK, no backup bundle format, and no
restore path. `roymctl` has no backup verb either — its command set is
`app`, `identity`, `member_identity`, `registry`, `security`, `session`,
`substrate`, `supervisor`, `svc`. R1's identity row and R2's rows 4 and 5
all sit on top of this, and all of it is unbuilt.

**Gap 7 — search has no index surface, but SQLite already supplies one.**
`data-layer`'s `query-options` is `filter` / `limit` / `cursor`, and
`index-definition` allows only `string` / `numeric` / `boolean`
([data-layer.wit:3-40](../../../../crates/wit_interfaces/wit/data-layer/data-layer.wit#L3)).
Neither text search nor an area query is expressible. **But the same
interface already exposes `execute-ddl` and `query-raw`**, and the bundled
SQLite is compiled with `-DSQLITE_ENABLE_FTS5` and `-DSQLITE_ENABLE_RTREE`
(verified in `libsqlite3-sys` 0.36.0's own `build.rs`, the version
`Cargo.lock` pins — read from the vendored source, not from documentation).
So the Directory can create and query its own FTS5 and R\*Tree tables through
DDL, inside the same DEK-encrypted database, with **no new host interface**.
No `sqlite-vec` is present, and none is needed: free-text intent parsing is
explicitly out of the first release.

**Gap 8 — no product code exists.** Nothing in `crates/`, `apps/`, or
`test-components/` carries a listing, quote, agreement, membership
credential, revocation, or moderation record. This milestone starts from
nothing on the product side, which is the expected state, recorded so the
size is not underestimated.

### Carried forward from M06B, with eyes open

These shipped as accepted limits and the product will feel them. They are
already rows in [deferred-backlog.md](../../deferred-backlog.md) §5; they are
repeated here because a slice plan that does not know about them will design
around a substrate that does not exist.

| Limit | What the product feels |
|---|---|
| **`heads()` caps a DAG entry's parents at 8** (`MAX_PARENTS`, `crates/conversation/src/dag.rs`) with no fallback for a wider concurrent frontier | Nothing today, and provably so: `dag_parents` has two readers and neither uses parent links for correctness — ordering comes from `(sender_timestamp, author, entry_id)` and sync from the `seq` cursor. R4 must not introduce a feature that reads causal structure from parents |
| **A group message to a member removed while it is still pending settles `Failed` only after the full age window** (`drain_item`, `crates/conversation/src/outbox.rs`) | A removed member's undelivered messages sit `pending` for `max_pending_age_secs` before the sender sees `failed`. The R4 UI must not present that window as "still trying to reach a member of this group" |
| **A member's signing key is trust-on-first-use** — the group placeholder-row pin (`pinned_member_sig_key`) and the 1:1 pin (`D-B4-28`) both protect whoever pins first, not whoever is genuine | Roym must not present "verified" for a conversation peer with the same confidence it presents it for a signed record whose issuer chain it checked itself. Two different strengths, two different words in the UI |
| **No operator surface for the conversation queue's dead letters** — `roymctl svc proxy-dead-letters`/`proxy-replay` exist, the conversation equivalents do not | A node operator cannot see or replay a stuck conversation message. Only the guest's own `outbox()` shows it, as `failed` |
| **The gateway's person sessions are in-memory** — "Empty at boot and after every restart by design" ([gateway.rs:78](../../../../crates/client_gateway/src/gateway.rs#L78)) | The Hub must handle "the substrate restarted, log in again" as an ordinary state, not an error |

---

## Decisions

> **On identifiers.** `D-06C-n` are milestone-level decisions, matching the
> house `D-<scope>-<n>` shape. Bare `C1`–`C10` are **slices**, matching
> M06A's `A1`–`A5` and M06B's `B1`–`B5`. The two namespaces are separate; a
> reference to "C1" always means the slice.

| # | Decision | Why |
|---|---|---|
| D-06C-1 | **The spec's G5 — public contract versioning — is out of scope. Cross-installation schema versioning and cross-version fixtures are deferred. But every signed record carries an explicit version field from its first byte.** | The product is pre-release. There is no installed base to stay compatible with, so a version *ladder* — migration paths, compatibility shims, fixtures pinned at old versions — would be built against a population that does not exist, and would be wrong by the time one does. The field is a different thing entirely: adding a field to a signed structure later changes its canonical bytes, which invalidates every signature already produced. That is cheap now and expensive once data exists. **A field is not a compatibility shim.** G5 is currently assigned to no milestone at all — it appears once, in the spec, and nowhere in planning — so it gets a [deferred-backlog.md](../../deferred-backlog.md) row with the pickup trigger **"revisit before first external release"**, or it silently disappears |
| D-06C-2 | **D-06C-1 makes one acceptance test in the spec's normative table unmeetable as written. It is edited in the spec, not quietly failed.** **R2, export row**: *"Cross-version fixture test passes; import reproduces verification status"* → **"A same-version export/import round-trip passes; import reproduces verification status."** **R1, listing row** — *"Listing round-trips through export/import with schema version preserved"* — **survives unchanged**: a same-version round-trip that carries the version field through is not cross-version work, and D-06C-1 keeps the field. Confirmed, not assumed | An acceptance test nobody can pass is worse than one that was honestly narrowed: the first rots into "we know that one always fails", the second stays a real gate. Editing the spec is the house rule (`D-06B-4` amended M05C's S4 row the same way) rather than overriding it in silence |
| D-06C-3 | **The card type set is fixed for the first release, and the unknown-type rule is a client invariant with a named mechanism.** Seven types: `request`, `quote`, `agreement-receipt`, `booking-progress`, `payment-request`, `payment-acknowledgement`, `fulfilment-receipt`. Each renders through a fixed template chosen by `(type, version)`. **A card is data, never code**: no sender-supplied HTML, script, style, or markup is ever inserted into the page, and no URL a card carries is fetched, prefetched, or navigated to automatically — the one payment link is shown in full and followed only on an explicit human click. An unknown type, or a known type at an unknown version, renders as a neutral block naming the type and saying this client does not understand it, and never as an empty or a guessed card | The spec gives cards one paragraph, and the producer (Transaction, C7/C8) and the consumer (the Hub, C2) are in different slices. A rule that spans two slices and is decided inside either one gets decided twice, differently. Fixing the set here also bounds the renderer: the Hub ships seven templates plus a fallback, not a generic interpreter. **No ADR is needed** — with G5 out of scope (D-06C-1) there is no cross-version negotiation to record, and the set is small enough to live in the slice plans |
| D-06C-4 | **Signing gets a host interface. It is not solved by revealing a key to the app.** `syneroym:vault`'s `reveal` could hand a guest key bytes to sign with; it must not be used that way | D7 puts signing in the substrate under a delegated key precisely so private key material has one home. Copying a signing key into a WASM linear memory (or into a natively linked app's heap) gives it two, and the second one is inside the code most likely to have a bug. D3 closes the other escape route: the native build cannot reach `syneroym-identity` directly either. The shape of the interface — what it signs under, what it refuses to sign — is C3's to settle |
| D-06C-5 | **Roym keeps its own copy of conversation content in app-owned storage, written from `on-message` and from its own `send`.** The host's conversation store stays the delivery and ordering authority; Roym's copy is what is exported, searched, deleted, and restored | Forced by Gap 3: nothing can write history back into the host store, and nothing can remove a message from it. Without an app-owned copy, R1's "restore reproduces history", R2's export row, and the spec's own message-deletion rule are all unmeetable. The alternative — new host verbs for import and delete — is more substrate work for a problem the app can solve, and it would make Roym's export depend on a host format instead of its own versioned one (D-06C-1). The cost is honest and must be stated in the slice plan: two copies of every message body on disk |
| D-06C-6 | **The SynOrg / Directory service spans R1 and R3, and is never on the critical path.** R1 builds the **search half** (category / area / filter query, results carrying source and freshness). R3 builds the **trust half** (membership credentials, revocation lists, moderation decisions). **R2 has no Directory work at all.** Three constraints bind every Directory slice: **(a) it is optional by construction** — a consumer reaching a provider by direct link or referral must complete the entire R1 flow with no directory deployed anywhere, and a test must prove it; **(b) it is not the M8 Matching Fabric** — no shard placement, no signed Publications, no rendezvous hashing; **(c) finding is separate from trusting** — the consumer's own node verifies signature, issuer, scope, expiry, and revocation, and never takes the directory's word for a verification result | All three come straight from the spec: (a) is rule 2 under [What Roym is made of](../../../roym-integrated-experience-spec.md#what-roym-is-made-of), (b) is D1 and the meta plan's own feature-grouping note, (c) is the [Search](../../../roym-integrated-experience-spec.md#search) section's closing rule and R3's credential acceptance test. Splitting the service across two releases is what makes (a) enforceable: if R1 shipped credentials too, "optional" would quietly become "optional unless you want to trust anyone" |
| D-06C-7 | **M06C adopts the cross-node failure cases M06B left uncovered, with one named exception.** M06B's exit criterion 11 ("every failure-matrix row has a test") is **not** strictly met: `status.md` records 13 of slice B4's 17-row cross-node table as uncovered — dropped-ack retry, rate-limited prekey requests, per-conversation quota isolation under concurrent conversations, clock-skew rejection, the same-service exemption (`D-B4-26`, unit-tested but never over a real connection), cross-service capability-gate denial, and no-instance-certificate refusal. **C9 adopts all of these into its own matrix**, because R3 stands up three real installations — which is the harness they were missing. **The exception is alias canonicalization** (`D-B4-29`): it is not implemented, so there is nothing to test. It keeps its own backlog row and its own trigger, and C7/C9 must pass canonical DIDs, never registry aliases | Leaving them in M06B's backlog after M06C builds the harness that makes them cheap would be choosing not to test something for no reason. Saying so explicitly — rather than letting M06B's criterion 11 stand as met — is the point of this row |
| D-06C-8 | **Block is enforced by Roym's Conversation service, and the product says exactly what that means.** A blocked sender's message is refused at Roym's own inbox: it never appears in any conversation, never fires a notification, and is never counted. It **is** still in the host's conversation store underneath, because the host stores before any app code runs (Gap 4). Roym records a tombstone for it and drops the body from its own copy on the next pass. The UI never claims the sender "could not send" | The spec's own posture on deletion applies here verbatim: *"The product does not promise deletion it cannot enforce."* The alternative — an admit hook on `syneroym:conversation` — is real substrate work with a real design (what does the host do with a refused message; who pays for the storage it already used) and belongs behind a decision, not inside a product slice. Recorded as a backlog row with the trigger **"a second consumer wants inbound filtering"** |
| D-06C-9 | **Roym's crates live under `crates/<snake_case>/` with package names `syneroym-roym-<kebab-case>`.** One crate holds the shared record, card, and dual-build wiring; each service is its own crate | AGENTS.md states the rule without exception: *"a new crate always goes under `crates/<snake_case_name>/` with Cargo package name `syneroym-<kebab-case-name>`"*. `apps/` holds one binary, `roymctl`, and is not a precedent for library crates. The `syneroym-roym-` prefix reads awkwardly and is worth naming as a known cost rather than discovering in review; it is still preferable to a second, undocumented placement rule. Per-service crates, not one crate with features, because each service is separately deployed and separately certified, and a shared crate would make the WASM build of any one of them carry all six |
| D-06C-10 | **The shim grows its missing traits (`proxy`, `http` inbound, `app-config`, `vault`) in C1, before the first product service is written.** | The same reasoning `D-06B-3` used for B3, now with the consumer in hand: designing a trait with only one of its two implementations exercised is how the native build becomes second-class, which is the exact failure D3 exists to prevent. `proxy` is the one that cannot wait at all — without it the native build cannot make a single service-to-service call, and *every* Roym service makes them |
| D-06C-11 | **No slice may ask for a planning identifier in product code.** No `R1`, `C4`, `M06C`, or slice number appears in a crate name, module name, collection name, JSON-RPC method name, card type, record type, config key, metric name, or test name. Record and card types are named for what they are (`quote`, `agreement-receipt`), and comments explain the current constraint, not which slice introduced it | AGENTS.md's rule, restated here because M06C is the milestone most tempted to break it: it is organised by release, and a release number is exactly the kind of label that looks like a natural name and rots the moment the doc is archived. ADR references stay fine |

---

## Explicit non-goals

Everything the spec's [Not in the first release](../../../roym-integrated-experience-spec.md#not-in-the-first-release)
and [Beyond the first release](../../../roym-integrated-experience-spec.md#beyond-the-first-release)
sections list, plus what M06B ruled out and what this document adds:

- **Payment processing, holding money, or escrow.** Roym opens an external
  payment page and records what both sides say happened.
- **Public ratings or scores** computed from past work.
- **Any AI assistant** that searches, recommends, or acts for people. Search
  is category, area, and filters — never free-text intent parsing. AI
  participants in group chats (`[APP-AGI]`) depend on M9A and are sequenced
  after it.
- **Automatic discovery between independently run SynOrgs.** Groups may
  record peer relationships; the protocol is later.
- **Automated dispute workflow.** A consumer forwards complaint details to a
  SynOrg owner by hand.
- **The Distributed Matching Fabric (`[P2P-DSC]`)** — no shard placement, no
  signed Publications, no rendezvous hashing. M8.
- **Cross-version schema fixtures and migration paths (G5)** — D-06C-1.
- **Attachments, voice, and video.** Multi-device sync
  ([ADR-0013](../../../decisions/0013-p2p-messaging-architecture.md) §2).
  Chat polish: threads, polls, reminders, read receipts, typing indicators,
  broadcasts.
- **MLS.** Replaced by the owner-distributed per-epoch key
  (ADR-0013 Amendment 1, D5); still a one-module swap later.
- **Third-party mailboxes.** A message waits in the sender's own outbox
  (D4).
- **A hosted or remotely-bound client gateway** serving browser-only
  consumers (`D-06B-5`, D8). Every participant installs a substrate.
- **Cross-app `Bind`.** Stays M05C S4 on its own gate — Roym still supplies
  no consumer for it (its six services share one app instance, and
  everything outside is a runtime DID).
- **Vector search / `sqlite-vec`**, ranking, paid placement, media
  galleries, stock counting, recurring bookings, multi-part jobs, selective
  export, redaction, automatic cloud backup, automated moderation, appeals
  workflow, cross-group propagation, global blocklists, and unbounded
  history retention — each excluded by a named row of the spec's own scope
  table.
- **An inbound admit/reject hook on `syneroym:conversation`** — D-06C-8.

---

## Dependency gates

| Depends on | State |
|---|---|
| M06A A1–A5 — static assets from blobs, guest HTTP route target, inbound WebSocket, browser suite | **Complete (2026-08-17)** |
| M06B B1 — person identity at the client gateway (G3) | **Complete (2026-08-18)**. A guest HTTP handler behind an active person session sees `caller-auth = delegated` and the person's master DID ([http.wit:31-58](../../../../crates/wit_interfaces/wit/http/http.wit#L31)) |
| M06B B2 — declared service visibility (G4), both layers | **Complete (2026-08-18)**. `ServiceSpec.topology_visibility` (ADR-0022 §5) and `service-config.visibility` (ADR-0018) |
| **M05C S4's visibility half — the meta plan's one named external prerequisite for M06C** | **Met.** `D-06B-4` moved it into M06B as slice **B2**, which shipped **2026-08-18**. An app declaring `topology_visibility = open` plus a registered `visibility` is resolvable by a caller on an unaffiliated installation **with no pre-installed token**. This is no longer an open gate, and the meta plan's "External prerequisite" note needs the corresponding edit |
| M06B B3 — dual-build shim (D2/D3) | **Complete (2026-08-20)**, over `data-layer`, `blob-store`, `messaging`, and later `conversation`. Four of the seven interfaces Roym needs — Gap 2 |
| M06B B4 — durable messaging interface and 1:1 delivery, outbox state (G1 part 1, G2) | **Complete (2026-08-20)**, real X3DH + Double Ratchet via `vodozemac` |
| M06B B5 — group delivery: gossip DAG, ordering, owner-distributed epoch key (G1 part 2) | **Complete (2026-08-24)**, after an errata pass. Accepted limits carried forward above |
| M04A caller identity through native dispatch ([ADR-0016](../../../decisions/0016-native-dispatch-identity-threading.md)) | Shipped |
| M04B FDAE policy / access control | Shipped |
| M05B async primitives — `crates/async_queue`, [ADR-0023](../../../decisions/0023-durable-async-primitives.md) | Shipped; guest-reachable for conversations via B4 |
| M05C S1–S3 — registry record, signed topology document, gateway hostname scheme (ADR-0022 §7) | Shipped |
| [ADR-0013](../../../decisions/0013-p2p-messaging-architecture.md) + Amendment 1 | Accepted |
| [ADR-0015](../../../decisions/0015-ucan-capability-model.md) UCAN capability model | Accepted; `crates/ucan` shipped |
| [ADR-0018](../../../decisions/0018-service-record-visibility.md), [ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md), [ADR-0023](../../../decisions/0023-durable-async-primitives.md) | Accepted and implemented |
| M05C S4's **cross-app `Bind`** half | **Not a gate.** Roym's six services share one app instance and use `call-target::dependency`; everything outside is user-chosen after deployment and addressed by DID at runtime via `call-target::service`. Stays parked on `D-C-7` |

---

## Slices

| # | Scope | Release | Depends on |
|---|---|---|---|
| **C1** | **Complete the dual-build shim (Gap 2, D-06C-10).** The four traits Roym needs and `AppHost` does not have: `syneroym:proxy` (`call`/`enqueue`, both target shapes), `syneroym:http` inbound (`incoming-handler`, `websocket-handler`), `syneroym:app-config`, and `syneroym:vault` — each with a `wit-bindgen` guest implementation and an in-process native one, and each proven by the existing `test-components/dual-build-fixture` built both ways. No product code. The hard part is native inbound HTTP, which has no equivalent in the tree at all | — | M06A, M06B |
| **C2** | **The SynApp skeleton and the Hub shell.** One `SynAppManifest` with six services (`web`, `conversation`, `profile`, `catalog`, `transaction`, `directory`), sibling wiring by `depends_on` + `call-target::dependency`, and `topology_visibility` / `visibility` declared so a foreign caller can resolve what it should. The Web entrypoint as an ordinary WASM component (`D-06B-1`) serving the UI bundle and forwarding JSON-RPC from one origin — no business logic in it. The Hub shell: a person logs in through the gateway session and the guest sees *them*, plus the card renderer's fixed templates and its unknown-type fallback (D-06C-3) | — | C1 |
| **C3** | **Signed records: the host signing interface and the record envelope.** A host interface that signs under the person's delegated key or the service's instance key, and refuses to hand any key material out (D-06C-4, Gap 1). One canonical envelope — stable byte encoding, explicit `version` field (D-06C-1), issuer, subject, timestamps, signature — plus guest-side verification of signature, issuer, scope, expiry, and revocation. Every record type in R1–R4 rides this and none is defined before it | — | C1 |
| **C4** | **Identity, profile, contacts, and safety (R1 rows 1 and 6).** Device-bound consumer identity with encrypted backup and import, and a restore that reproduces identity on a clean node. Profile & Contacts, including the person→conversation-address mapping Gap 5 forces. Block, report, per-sender contact rate limits, publication limits, and policy disclosure — the `[PRD-SAF]` backlog row. Block is Roym-side and honestly described (D-06C-8) | **R1** | C3 |
| **C5** | **Catalog and conversation in the product (R1 rows 2 and 3).** The versioned listing schema across all seven dimensions the spec names (booking, payment, product, service, location, relationship, service-record), signed and editable. 1:1 conversation over B4: `pending`/`delivered`/`failed` visible and never optimistic, surviving a restart on both sides. Roym's own copy of conversation content (D-06C-5), which is what export, search, and delete act on | **R1** | C4 |
| **C6** | **Directory: the search half (R1 row 5).** The SynOrg service, its member list, provider-initiated listing publication (S7 — publishing is the *provider's* action), and `search` by category, area, and filters, built on FTS5 and R\*Tree through `execute-ddl`/`query-raw` (Gap 7). Results carry source and freshness. The consumer's node queries each directory it was given, in parallel, and merges. Missing evidence renders as unknown, never as a positive default. Optional by construction, proven by a test (D-06C-6a) | **R1** | C5 |
| **C7** | **A need becomes an offer, and the card contract (R1 row 4).** Signed `request` → `quote` → `agreement-receipt`, each versioned, with a material change producing a new version rather than an edit. The seven card types and the unknown-type rule land here on the producing side and in C1's renderer on the consuming side (D-06C-3). **R1's acceptance gate closes at the end of this slice** | **R1** | C5, C6 |
| **C8** | **The transaction vertical (R2, all five rows).** The state machine with one named writer on the provider's substrate, permitted transitions, expiry, idempotency keys, and a named conflict for a losing concurrent booking. `payment-acknowledgement`, separate from settlement, with the payee bound into the signed agreement and a UI that never says "verified". Mutually signed `fulfilment-receipt`. Versioned, integrity-checked export and import of conversations, agreements, and receipts. Encrypted backup with a restore path tested on a clean node | **R2** | C7 |
| **C9** | **Cross-installation trust (R3, all three rows) and the inherited cross-node cases.** The full R1+R2 flow with consumer, provider, and SynOrg owner on three separate installations, resolving each other through the discovery overlay. Signed `membership-credential` (issuer, scope, expiry) and signed `revocation`; the consumer's **own** node verifies, never the directory. Signed, scoped `moderation-decision`; a suspended member vanishes from that directory's results and cached copies show the revocation on next check — with the product saying plainly that instant removal is not promised. Adopts M06B's 13 uncovered cross-node cases except alias canonicalization (D-06C-7) | **R3** | C8 |
| **C10** | **Private group chat in the product (R4, all five rows).** Group conversations over B5: no server in the path, byte-identical transcripts, joiner and removed-member key boundaries, membership as visible events, offline catch-up. Product-side: group naming and roster UI, the owner's read access stated in the UI, and the carried-forward limits above surfaced honestly rather than hidden | **R4** | C5, C9 |

**Dependency shape.** **C2 and C3 are independent** and can run in parallel
once C1 lands — one builds the app's outside, the other its record format,
and neither reads the other. C3 → C4 is then a hard line: nothing signs
before the signing interface exists, and every product record is signed. C4
and C5 could themselves overlap once C3's envelope is frozen. C6 needs C5's
listings to have something to index. C7 needs both C5 (the conversation the
cards appear in) and C6 (the search that starts the flow), and closes R1.
C8 → C9 → C10 follow the spec's own release order, with C10 also depending on
C5 because R4 declares its dependency on R1's 1:1 messaging.

> **Why C1 is not folded into C2.** The tempting order is "build the app,
> and make the native build work as you go". `D-06B-3` refused that order
> once already, for B3, and the reasoning holds harder here: four traits
> designed against a running Hub would be designed against the WASM build
> alone, and the native build would inherit whatever shortcut made the Hub
> work. There is a second, sharper reason this time. Native inbound HTTP has
> no equivalent anywhere in the tree — the WASM build exports
> `incoming-handler`, and nothing answers it natively — so C1 carries real
> design risk. It must not sit inside a slice that also owes a working Hub,
> where the tempting resolution is to quietly exempt the entrypoint again.
> That exemption was retired on purpose (`D-06B-1`); re-earning it by
> accident is the failure to avoid.

**Why the releases are not collapsed.** The spec states the gate: *"Each
release must pass its acceptance tests before the next begins."* R1 with
R2's state machine folded in would let a transaction bug hide behind a
discovery bug. Keeping the gates keeps each failure attributable.

### Open design points for the slice plans

- **What the signing interface signs under, and what it refuses.** Three
  candidate principals exist: the person's master DID via the B1 delegation,
  the service's derived instance key
  (`resolve-instance-identity`, [control-plane.wit:206-229](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L206)),
  and the node. A `listing` is the provider *as a person*; a
  `membership-credential` is the SynOrg *as an organisation*; a
  `moderation-decision` is arguably the Directory *as a service*. C3 must
  pick which of these the interface exposes and how a caller names one,
  rather than exposing "sign these bytes" and letting each service decide.
  It must also decide what the host refuses to sign — an envelope it cannot
  parse is the obvious floor.
- **Where the app-owned message copy lives, and what it costs.** D-06C-5
  requires one. `data-layer` (structured, DEK-encrypted, queryable, the
  obvious home) versus `blob-store` for large bodies. The slice plan owes an
  honest statement of the on-disk duplication and a bound on it.
- **How the Directory's FTS5 and R\*Tree tables coexist with `data-layer`'s
  own collection tables.** `execute-ddl` runs an arbitrary batch against the
  service's own database ([sqlite.rs:139](../../../../crates/data_db/src/sqlite.rs#L139)),
  and `create-collection` also creates tables. C6 must settle naming and
  ownership so a `drop-collection` cannot orphan an index, and must decide
  whether the index is rebuildable from the collection or is itself the
  source of record.
- **What "area" means on the wire.** R\*Tree gives bounding boxes; the spec
  says "area" and "service area" without defining them. Geohash prefix,
  bounding box, named region, or radius are all defensible and produce
  different listing schemas. C5 defines it, because the listing carries it;
  C6 only queries it.
- **Which service owns the person→conversation-address mapping, and how a
  listing carries it.** Gap 5 puts it in Profile & Contacts, but a listing
  found through a directory must carry enough to start a conversation
  without a prior contact entry. C5 and C6 must agree on this, once.
- **How the native build receives an inbound HTTP request.** The WASM build
  exports `incoming-handler`. The native side has no equivalent anywhere in
  the tree, so C1 is inventing one, and C2's entrypoint is shaped by whatever
  it invents. This is the single largest unknown in the milestone's first two
  slices. Whatever the answer is, the same integration suite must run against
  both builds and pass identically — the rule the shim was built for.
- **Whether the export bundle is one format or several.** R2 exports
  conversations, agreements, and receipts. One envelope with a manifest, or
  a bundle per record family. Integrity checking (a signed manifest over
  content hashes) is required either way.
- **How a natively linked Roym gets an FDAE policy and a `RowAuthorizer`.**
  Two backlog rows already target `M06C` by name: a linked native app has no
  deploy record to load a compiled policy from, and the only `RowAuthorizer`
  in the tree is `AppSandboxEngine`, which a native app cannot reach
  (`crates/app_host_native/src/factory.rs`). C1 must decide whether Roym's
  native build runs policy-free — and say so loudly — or whether this
  milestone builds the native policy path. It is C1's rather than C2's
  because it is a property of linking an app in at all, not of Roym.

---

## Migration impact

- **A new host interface for signing** (Gap 1, D-06C-4). Additive, a new WIT
  package, and — following M06A's `syneroym:http` and M06B's
  `syneroym:conversation` — deliberately **not** added to the
  `host-environment` world by default, so a component that does not import
  it deploys exactly as before.
- **New traits on `AppHost`** for `proxy`, `app-config`, `vault`, and
  outbound `websocket` (with HTTP inbound as native `HttpSink`/`WebSocketSink`
  sink traits, D-06C-10). This is a **breaking change to `syneroym-app-host`'s
  own trait bound**: `AppHost`'s supertrait list grows, so every existing
  implementor must satisfy the new traits. Today there are exactly two
  (`GuestHost`, `NativeAppHost`), both in-tree, so the cost
  is bounded and known — but C1's plan must name it rather than discover it.
- **No wire-format change** to endpoint records, topology documents, gateway
  hostnames, the conversation envelope, or the group DAG. M06C consumes all
  of these and changes none.
- **New manifest content, not new manifest fields.** Roym is the first
  multi-service app to declare `depends_on` between siblings *and*
  `topology_visibility = open` on the services a stranger must reach. Both
  fields exist; nothing in the tree exercises them together at this size.
  Per `D-B2-15`, every member needing cross-node resolution must declare
  `visibility = "internal"` or the first cross-node call fails late with
  *"No valid Iroh mechanism found"*.
- **A versioned on-disk record format that is Roym's own**, not the host's
  (D-06C-1, D-06C-5). Pre-release, so there is nothing to migrate *from*;
  the version field exists so there is something to migrate *to*.
- **`roymctl` may grow a conversation dead-letter surface.** Not required by
  any acceptance test, but the gap is real and the operator has no other
  view. If a slice adds it, it mirrors `svc proxy-dead-letters` /
  `proxy-replay`; if none does, the backlog row stays open.

---

## Reference scenario (runnable)

Three substrates, three people, one Roym SynApp deployed on each.

```
 1. Deploy Roym on install X (consumer), Y (provider), Z (SynOrg owner).
    Build the app both ways; run one integration suite against both --
    identical results.
 2. Z creates the SynOrg: name, rules, area, categories, support contact,
    dispute path, retention policy. Runs the Directory service.
 3. Y creates a profile and a signed listing with availability. Y is live
    and reachable by direct link, in no directory yet.
 4. X reaches Y by direct link alone, with no Directory deployed anywhere,
    and completes steps 8-16 below                    -> the whole R1+R2
    flow passes with no directory in the path (D-06C-6a)
 5. Y applies to the SynOrg; Z reviews and issues a signed membership
    credential, scoped, with an expiry.
 6. Y chooses to publish the listing to Z's directory.
 7. X adds Z's directory, searches by category and area.
      -> results carry source and freshness
      -> X's own node verifies each listing signature and each credential:
         signature, issuer, scope, expiry, revocation
      -> a credential X cannot check renders as unknown, never as positive
 8. X starts a conversation from the listing, subject to Y's contact rate
    limit. Y is offline -> `pending`, held in X's own outbox.
 9. Restart X's substrate     -> still `pending`, exactly one item, and X
    must log in again (sessions do not survive a restart).
10. Y comes online           -> delivered; X sees `delivered`.
11. X sends a signed request card; Y sends a signed quote card with scope,
    price, taxes, schedule, location, payee, cancellation terms, expiry,
    and dispute path.
12. X accepts                -> a signed agreement receipt carrying every
    field the spec's Records section lists.
13. X books a slot. A second consumer books the same slot at the same
    moment -> exactly one `scheduled`, one named conflict, never two
    confirmations. The retry of a lost booking reaches the same final
    state.
14. Y requests payment. The payee comes from the signed agreement; a later
    chat message claiming a different payee changes nothing.
15. X pays outside Roym; both sides record a payment acknowledgement. The
    UI never labels either one as verified payment.
16. Y marks the work complete; both sign a fulfilment receipt. Neither can
    alter it afterwards; a correction is a new record.
17. Z publishes a signed suspension of Y     -> Y leaves that directory's
    results; X's cached copy shows the revocation on next check, and the
    product does not claim instant removal.
18. X exports everything and imports it on a clean node -> identity and
    history reproduce, and verification status reproduces with them.
19. X, Y, and Z form a private group. All three post from deliberately
    skewed clocks with no coordinator reachable -> byte-identical
    transcripts. Z goes offline, misses messages, returns, pulls the gap
    from any online peer. The owner removes Z and rekeys -> Z reads
    nothing after the removal, and the group sees the removal as an event.
20. A card of an unknown type arrives -> renders as a neutral block naming
    the type, executes nothing, fetches nothing.
```

Step 4 is the one to watch: if it fails, the Directory has become a required
hub and D-06C-6a is broken, whatever the rest of the scenario does.

---

## Failure and security matrix

| # | Case | Expected |
|---|---|---|
| 1 | A directory returns a listing with a forged or absent signature | The consumer's own node rejects it. Never displayed as trusted evidence, and never displayed as unknown-but-probably-fine |
| 2 | A directory asserts a credential is valid, and it is expired, out of scope, or revoked | The consumer's node's own verdict wins. The directory's assertion is never consulted for a verification result (D-06C-6c) |
| 3 | No Directory is deployed anywhere | The full R1+R2 flow completes by direct link. A test proves it (D-06C-6a) |
| 4 | A card arrives with an unknown type, or a known type at an unknown version | Renders as a neutral block naming the type. No sender-supplied markup is inserted, no URL is fetched or navigated, nothing is executed (D-06C-3) |
| 5 | A quote's payee is contradicted by a later chat message | The agreement's bound payee is what the UI shows. The chat message changes nothing |
| 6 | A payment acknowledgement is displayed | Never as verified payment. The UI states that both sides said the same thing and that money movement is unproven |
| 7 | Two consumers book the same slot at the same instant | One `scheduled`, one named conflict. Never two confirmations, and never last-write-wins |
| 8 | The same booking request is retried after a lost connection | Same final state, one booking. The idempotency key is the fence |
| 9 | An unaccepted quote is left open | It expires at its stated expiry rather than staying live |
| 10 | Either party tries to alter a signed receipt | Impossible. A correction is a separate record referencing the old one, and both remain |
| 11 | A blocked sender sends a message | It never appears in any Roym conversation, fires no notification, and is counted nowhere. The product does **not** claim the sender was prevented from sending, and X's transaction records with that party survive the block (D-06C-8) |
| 12 | A provider floods a directory with listings, or a stranger floods a recipient with first contacts | Refused by publication limits and per-sender contact rate limits respectively. Refusal is visible to the sender, not silent |
| 13 | An export is imported on a clean node | Identity, history, agreements, and receipts reproduce, **and verification status reproduces with them** — a record that verified before verifies after, and one that did not, does not (D-06C-2) |
| 14 | A record is produced with no version field | Impossible by construction: the envelope requires it, and a record without one fails to parse rather than defaulting (D-06C-1) |
| 15 | A suspended member's already-cached listing is checked again | It shows the revocation. The product never claims every cached copy is already gone |
| 16 | A group message is sent to a member removed while it was still pending | Settles `failed` after the age window, and the UI does not present that window as active progress toward a current member (carried-forward limit) |
| 17 | The substrate restarts mid-session | The Hub shows "log in again" as an ordinary state; no work in progress is lost, and no message is double-sent |
| 18 | A caller on an unaffiliated installation resolves Roym's Directory | It resolves with no pre-installed token, because the service declares `topology_visibility = open` and a registered `visibility`. A service that declares neither stays cleanly refused |
| 19 | Any interface behaves differently on the WASM build and the native build | The shared suite fails. *"A test that passes on one and fails on the other is a bug in the shim"* |
| 20 | The 13 cross-node cases M06B left uncovered — dropped-ack retry, prekey rate limiting, per-conversation quota isolation, clock-skew rejection, same-service exemption, cross-service capability denial, no-instance-certificate refusal | Covered by C9 against three real installations. Alias canonicalization is the one exception and stays a backlog row (D-06C-7) |

---

## Measurable exit criteria

1. The Roym SynApp builds **both ways** — every service as a
   `wasm32-wasip2` component, and linked into `syneroym-substrate` — from
   one source tree, with **no direct host-crate call in the native build**
   (D3). One integration suite runs against both and passes identically.
2. The UI is served by the Web entrypoint from **one origin**, and every
   capability the UI uses is a public JSON-RPC method. A second client
   (a script or `roymctl`) drives the same flow through the same API with no
   UI involved.
3. A person logs in locally; Roym's services see **that person's** DID with
   `caller-auth = delegated`. A second local process cannot obtain it.
4. Every record type in the spec's [Records](../../../roym-integrated-experience-spec.md#records-and-what-each-one-proves)
   table is produced signed, carries an explicit version field, and is
   verified by the receiving node itself — never by the service that served
   it.
5. **R1's acceptance tests pass**, all six rows, including the reworded
   ones (D-06C-2): identity restore on a clean node, listing round-trip with
   version preserved, message survives a restart on both sides and is never
   shown delivered while pending, a signed agreement receipt with every
   listed field, search results carrying source and age with missing
   evidence shown as unknown, and a blocked sender never reaching the
   recipient's inbox while transaction records survive.
6. **The whole R1+R2 flow completes with no Directory deployed anywhere.**
   This is a hard gate on D-06C-6a, not a nice-to-have.
7. **R2's acceptance tests pass**, all five rows: one confirmation and one
   named conflict from two concurrent bookings; an acknowledgement never
   labelled verified and a payee matching the agreement; an unalterable
   fulfilment receipt with corrections as separate records; a same-version
   export/import round-trip that reproduces verification status; and a
   restore on a clean node with no acknowledged transaction lost.
8. **R3's acceptance tests pass**, all three rows, with the three parties on
   **three separate installations**: the full R1+R2 flow end to end; the
   consumer's own node verifying signature, issuer, scope, and expiry; and a
   suspended member vanishing from that directory's results with cached
   copies showing the revocation on next check.
9. **R4's acceptance tests pass**, all five rows: no server in the path,
   byte-identical transcripts from skewed clocks, joiner and removed-member
   key boundaries plus a scheduled rekey that changes the key with stable
   membership, identical membership history on every member, and an offline
   member converging after pulling the gap from a peer.
10. The Hub renders all seven card types plus the unknown-type fallback, and
    a test proves that a card carrying markup, script, or a URL results in
    no execution, no insertion, and no fetch (D-06C-3).
11. Every row of the failure and security matrix has a test — **including
    row 20**, the cases M06B left uncovered, minus the one named exception
    (D-06C-7).
12. No planning identifier appears in any crate name, module, collection,
    JSON-RPC method, card type, record type, config key, metric, or test
    name (D-06C-11) — checked by a grep, not by reading.
13. `deferred-backlog.md` carries a row for G5 with the pickup trigger
    "revisit before first external release" (D-06C-1), and every deferral
    this milestone creates.
14. `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets
    --all-features`, `cargo test --workspace`, `cargo audit`, `cargo deny
    check licenses`, and `mise run test:e2e` are clean.

---

## Documents this milestone edits

Recorded here so nothing is missed. The upstream edits landed with this
document; the per-slice ones are owed as each slice completes.

**Done 2026-08-24, with this document:**

| Document | Edit |
|---|---|
| [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md) | R2's export-row acceptance test reworded to a same-version round-trip; R1's listing row confirmed to survive unchanged (D-06C-2). G5 marked out of scope for the first release with a pointer to the backlog row (D-06C-1). The `### Cards` paragraph given the fixed type set and the safety rule (D-06C-3). The four releases given M06C slice owners, the way the Substrate-work section was given M06B's — and the scope-table preamble's stale *"three releases"* corrected to four while there |
| [meta-implementation-plan.md](../../meta-implementation-plan.md) | M06B marked B1–B5 Complete; M06C given its directory link and slice list; the **"External prerequisite"** note corrected — M05C S4's visibility half is met, shipped as M06B B2 on 2026-08-18, and is no longer an open gate |
| [deferred-backlog.md](../../deferred-backlog.md) | **New row for G5** (§10 — it is a product-contract question spanning every record family, not a data-layer one), trigger "revisit before first external release" — it has no milestone owner today and would otherwise disappear. **New row for the conversation admit hook** (§5), trigger "a second consumer wants inbound filtering" (D-06C-8). §10's `[PRD-SAF]` row retargeted from `M06 (R1)` to slice **C4**. §5's M06C-targeted rows (native FDAE policy, native `RowAuthorizer`, native subscription replay) retargeted at slice **C1**. The 13-uncovered-cross-node-cases row retargeted at **C9**, minus alias canonicalization which keeps its own row (D-06C-7). **Two §7 rows this table did not predict**, both left over from M06B B1: the browser person-session login flow is retargeted at **C2** (the Hub is its first consumer), and the WebRTC peer-proxy browser path is left with no slice and no owner — Roym's client contract is HTTP through the client gateway (D6) and D8 puts every browser on its own substrate, so M06C supplies no consumer for it, the same shape as M05C S4's parked `Bind` half |
| [M06B task.md](../M06B-roym-substrate-foundations/task.md) | Exit criterion 11 annotated: not strictly met at sign-off, with the uncovered set named and its disposition pointed at D-06C-7 rather than left as a silent gap |

**Owed as slices land:**

| When | Edit |
|---|---|
| C1 completes | `status.md` in this directory, created with C1 and not before. The three M06C-targeted native-shim backlog rows (native FDAE policy, native `RowAuthorizer`, native subscription replay) resolved or restated with what actually shipped |
| C2 completes | Gap 2 recorded as closed. If the entrypoint needed any exemption from D2/D3 to work, that is a `D-06B-1` regression and is written down as one, not absorbed |
| C3 completes | Gap 1's resolution recorded; the new signing WIT package named in the architecture section of [CLAUDE.md](../../../../CLAUDE.md)/[AGENTS.md](../../../../AGENTS.md) if the interface list there is still enumerated |
| C4 completes | §10's `[PRD-SAF]` row moved to "Recently resolved" |
| C7 completes | R1 marked passed in the spec's scope table |
| C8 completes | R2 marked passed. Export/backup rows in the backlog resolved |
| C9 completes | R3 marked passed. The cross-node-coverage row moved to "Recently resolved", minus alias canonicalization |
| C10 completes | R4 marked passed. The B5 carried-forward limits re-examined against what the product actually hit, and either closed or restated with real evidence |
| Each slice | `status.md`, plus a `slice-cN-implementation-plan.md` in this directory |
