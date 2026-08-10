# ADR-0022: Two-Tier Logical Service Discovery

**Status**: Proposed (2026-08-02, amended 2026-08-04 — see the dated
amendment note under §1). Depends on
[ADR-0020](0020-stable-logical-service-identity.md) (member master DIDs, and
§6's endpoint records published under them) and pairs with
[ADR-0021](0021-binding-propagation-and-app-supervisor.md) (the supervisor,
generations, and binding propagation). Discharges the "Design TBD to resolve
before M5" flag on shard discovery and data-routing tables in
[meta-implementation-plan.md](../planning/meta-implementation-plan.md).

**Context**:

A caller inside an app instance can already reach a dependency by logical name.
A caller *outside* it cannot.

Four pieces exist today and work:

1. Every member of a logical service has its own stable master DID
   (`ServiceId`), minted one per `PlannedService`
   ([keys.rs:261](../../crates/app_supervisor/src/keys.rs#L261)).
2. Every placed member publishes a registry record under that DID naming the
   substrate that hosts it, signed by the master key
   ([deploy.rs:404](../../crates/sdk/src/deploy.rs#L404)). This survives
   relocation: the DID is unchanged and only `substrate_id` moves, which is one
   ordinary compare-and-swap at the registry.
3. `LogicalResolver` already turns a `LogicalServiceRef` plus an optional
   routing key into a `ServiceId`, with `Singleton` / `Redundant` / `Sharded`
   selection, BLAKE3 rendezvous hashing, range chunk maps, epochs, and TTL
   caching ([resolver.rs](../../crates/app_orchestration/src/resolver.rs)).
4. The supervisor already distributes the topology those selections read — by
   push, as bindings.

The gap is one sentence: **`prepare_binding` refuses any write naming a
different app instance**
([orchestration.rs:191](../../crates/control_plane/src/service/orchestration.rs#L191)),
so nothing outside the app instance can obtain the `TopologyEntry` — mode,
member DID list, shard map, epoch — that every one of those selections needs.

It is worth being precise about what is *not* broken, because the obvious
framing ("the Community Registry cannot keep up with topology churn") points at
the wrong component. The registry handles a member changing substrate fine;
that is exactly what it is for. What the registry cannot express is (a) a *set*
of members under one logical name, and (b) which member owns which key. Those
two facts are the churn, and neither belongs in a store designed for
slow-moving, cacheable, self-signed identity records.

**Decision**:

## 1. An app instance has a master DID; `app_instance_id` stays the human name

An app instance gets a **master DID** as its network identity, minted at
`adopt` and held in the supervisor's vault alongside member masters.

`app_instance_id` is unchanged. It stays the short human name, and stays
embedded where it already is: vault names
(`member-<app_instance_id>#<service_name>-<index>`,
[keys.rs:261](../../crates/app_supervisor/src/keys.rs#L261)), alert topics
(`supervisor/alerts/<app_instance_id>`,
[service.rs:639](../../crates/app_supervisor/src/service.rs#L639)), and the
display form of every `LogicalServiceRef`
([models.rs:176](../../crates/app_orchestration/src/models.rs#L176)).

This is the same split the codebase already uses for substrates: a
`SubstrateAlias` humans type, and a `substrate_did` that travels on the wire.
Name for people and local storage; DID for the network.

**Why the app, and not the supervisor, holds this identity.** The supervisor
has no identity of its own today — `status` reports `self.node_did`
([service.rs:1978](../../crates/app_supervisor/src/service.rs#L1978)), the DID
of the substrate it runs on. If the app's address *were* the supervisor's DID,
then handing an instance to a different supervising node would change the app's
address for every external caller. The app DID keeps the app stable while the
supervisor underneath changes.

**Custody follows from the registry's keying rule, not from preference.** A
pkarr record must be signed by the key its own `service_id` resolves to
([dht_registry.rs:118](../../crates/core/src/dht_registry.rs#L118),
[registry.rs:234](../../crates/community_registry/src/registry.rs#L234)). A
UCAN delegation cannot stand in — the signature has to verify against the DID
itself. So whichever component publishes the Tier-1 record must hold the app
master key, and that is the supervisor. Handover is therefore a key move,
through the `export-master` / `import-master` verbs that already exist
([supervisor.wit:119](../../crates/wit_interfaces/wit/supervisor/supervisor.wit#L119)).

A second benefit, beyond addressing: an app-level DID is the natural subject
for access grants. "Grant this caller access to app X" is stable, where a grant
against each member DID changes on every rebalance.

**Amendment (2026-08-04, after M05A Slice A7 implementation, S0 of this
overlay).** Three facts this section did not carry, all recorded rather
than changing the decision itself — see the
[implementation plan](../planning/milestones/M05A-app-supervisor/slice-a7-implementation-plan.md)
§0/§1 for the reasoning:

- **The vault name actually chosen is `app-<app_instance_id>`, with one
  variable segment, needing no separator guard.** Unlike the member-master
  name below (which joins two variable-length segments and needed the `#`
  boundary), the app master's name has exactly one variable segment that is
  the whole remainder of the string — the map from instance id to name is
  injective by construction — and its fixed `app-` prefix is disjoint from
  `member-` at position 0, so no member name can ever equal an app name.
  Neither argument depends on what `AppInstanceId`'s validator permits.
- **The handover this section calls "a key move" needs an ordering rule,
  which this ADR did not state.** The intended sequence on a new
  supervisor is `submit`, `import-master`, `adopt`. Running `adopt` first
  mints a *second* app identity under the same name, which ADR-0021 §4's
  generation fence does not catch — that fence resolves two writers over
  *one* record, and the wrong order produces two records that never meet.
  A7 documents the order and makes `adopt` self-correcting (it
  resolves-and-records on every call, so a later `import-master`/`adopt`
  repairs the wrong order), rather than enforcing it — enforcement is
  S1's, the moment a wrongly-minted DID has an external consumer.
- **This section's own parenthetical vault-name form for a *member*
  master, `member-<app_instance_id>-<service_name>-<index>`, was already
  stale when this ADR was written** — M05A Slice A5e had replaced the `-`
  boundary with `#` one day earlier, closing a collision between two
  different (instance, service) pairs. Corrected above; A7 found and fixed
  the same stale copy that had also survived into shipped WIT.

## 2. Tier 1 — the registry maps the app DID to its supervisor, generation-fenced

The supervisor publishes one registry record under the app master DID, signed
by that key, naming the substrate it runs on. This reuses `EndpointInfo`
([dht_registry.rs:76](../../crates/core/src/dht_registry.rs#L76)) unchanged:
`service_id` is the app DID, `substrate_id` is the supervising node. "Where is
the supervisor for this app" is answered by the same lookup shape as every
other DID in the system.

**No new registry semantics.** In particular there is no `name → DID` record
type. A name is not a key, so a name record could not be self-signed, and
admitting one would mean both a second admission rule in a component that has
exactly one, and a name-allocation surface — the DNS problem this design exists
to avoid.

**The record carries `generation`.** Two supervisors both holding the app
master would otherwise fight over one record under last-writer-wins. That
split-brain is already a named hazard — two supervisors racing an `adopt` is
exactly the case
[ADR-0021 §4 and its 2026-08-01 amendment](0021-binding-propagation-and-app-supervisor.md)
work through. This record does not create the hazard, but it makes it visible
in a new place, and a reader needs the generation to tell which answer is
current.

## 3. Tier 2 — the supervisor signs a topology document; the RPC is one way to fetch it

The supervisor produces a **signed topology document** per logical service:

```
{ app_instance_id, app_did, service_name, mode, members: [ServiceId],
  sharding_strategy, shard_map, epoch, generation, not_after }
```

signed with the app master key. A `resolve` RPC on the supervisor returns it,
but the RPC is a transport, not the trust boundary.

**Why a signed document rather than a plain RPC answer**, which is the same
reasoning ADR-0020 §6 applies to endpoint records:

- **It is verifiable without the connection.** Trust comes from the signature
  against the app DID the caller already resolved in Tier 1. Any party may
  cache and relay it — a peer substrate, the client gateway, or the WebRTC
  coordinator serving it inside a bootstrap page. Under a bare RPC, a relayed
  copy would be unverifiable and every relay would become trusted
  infrastructure.
- **The supervisor stops being on the availability path.** A mandatory
  per-resolution RPC would mean a healthy, fully-replicated app becomes
  unreachable to new callers whenever one control-plane process is down. With a
  document plus `not_after`, that degrades to "the topology may be stale".
- **It keeps a latency-sensitive surface off the process holding every master
  key.** The supervisor vault holds every member master and the app master. A
  document that anyone may serve means the fetch does not have to terminate
  there.

**Caching is one rule, with no error taxonomy.** Cache with TTL; on expiry try
to refresh; if the refresh fails, keep using the previous document until
`not_after`; past `not_after`, fail. `LogicalResolver` already has the TTL,
epoch, and explicit-eviction machinery this needs, and the document feeds
straight into `LogicalResolver::register`.

**Early invalidation reuses the alert path.** The supervisor already publishes
to namespaced MQTT topics
([service.rs:635](../../crates/app_supervisor/src/service.rs#L635)). Publishing
epoch bumps there lets subscribed callers drop a cached document before its TTL
instead of polling.

**Resolution happens substrate-side, not in the guest.** The host capability
already carries the resolver
([host_capabilities.rs:141](../../crates/sandbox_wasm/src/host_capabilities.rs#L141)).
The substrate fetches, verifies, and registers a foreign app's document there,
so no guest re-implements Tier 1 and Tier 2 for itself.

## 4. Tier 2 returns member DIDs; Tier 3 is unchanged

The document names **member master DIDs**, never physical addresses. Turning a
member DID into a network location stays what it is today: the registry records
`certify_placed_members` already writes
([deploy.rs:359](../../crates/sdk/src/deploy.rs#L359)), read through the
existing lookup path.

Three things depend on this split:

- **Failover stays out of Tier 2.** A member moving substrate is absorbed by
  Tier 3, where a single record update already handles it. If Tier 2 answered
  with a location, every relocation would invalidate every cached answer and
  pull the supervisor into the failover path.
- **Load balancing stays with the caller.** `Redundant` mode round-robins
  unkeyed calls. If the supervisor picked, it would become the load balancer
  for the whole app and would have to see every request.
- **One fetch serves many routing keys.**

## 5. Visibility is per logical service, and all-or-nothing

The app owner declares, per logical service, who may fetch its topology
document: open to all, or requiring a UCAN, in the same manner as any other
service's access control. That declaration is part of the desired state
submitted through `submit` — not node-local supervisor config, which would be
neither reproducible nor able to survive a handover.

**A filtered member list is forbidden.** Handing a caller 3 of 8 shard members
does not restrict it; it corrupts it. Rendezvous hashing over 3 members returns
a confident, wrong answer for most keys, with no error raised — the caller
simply talks to the wrong shard. So a caller either receives the full member
set and mode, or receives a clean denial. Partial topology is worse than no
topology.

This costs the owner nothing real: the control that matters is which logical
services a caller may see at all, and that is preserved exactly.

## 6. The topology epoch is a fencing token on the data path

A caller may resolve under epoch N and send its request while a rebalance moves
the key to a different member at epoch N+1. For a data service that is a silent
write to the wrong shard — data corruption, not a retryable error.

So a request carries the topology epoch it was resolved under, and a member
rejects or redirects a request whose epoch no longer entitles it to that key.
The counter already exists: the supervisor stamps per-dependent binding epochs
and compares written against observed to judge convergence
([supervisor.wit:55](../../crates/wit_interfaces/wit/supervisor/supervisor.wit#L55)).
What is new is carrying it on the request.

**The field ships before it is enforced.** Enforcement only matters once
rebalancing exists, but adding a field to a wire format is free before anyone
depends on the format and expensive afterwards.

## 7. The gateway hostname carries identity; the routing key is a header

For a browser, the hostname *is* the security boundary — same-origin policy is
the only isolation an untrusted page has, and a header can be set by any script
on that page while a hostname cannot be forged past the browser. So the
hostname must carry everything that decides *what a caller may reach*.

The gateway host becomes:

```
<nickname>-a<app-did-hash>-s<logical-service-name-hash>-i<interface-hash>.localhost
```

parsed by the existing right-to-left, letter-prefixed scheme
([gateway.rs:274](../../crates/client_gateway/src/gateway.rs#L274)), which is
what lets a nickname contain dashes. The component is a hash of the **logical
service name**, not of a `ServiceId`: the caller does not know which member it
will reach — that is what Tier 2 is for. Four hashed components plus a nickname
is close to the 63-character DNS label limit, so truncation lengths are chosen
deliberately rather than inherited.

**Amendment (2026-08-10, M05C slice S3 implementation).** The printed form
above is superseded; the decision itself — the hostname carries what decides
reachability, the routing key is a header — is unchanged. Three corrections,
worked out in [the slice's own plan](../planning/milestones/M05C-logical-discovery-overlay/slice-s3-implementation-plan.md):

- **A trailing `-roym1` format marker.** The grammar had no way to tell a
  Syneroym host from a mistyped one, and no way to change the grammar again
  without guessing. The marker is popped first (right to left), so a future
  format ships as `-roym2` and both are served side by side.
- **`-p` is renamed `-s`.** `p` meant "pubkey hash", a distinction that stops
  existing once the app DID is *also* a pubkey; `-s` says directly what the
  segment names ("which service"), whether that is a concrete `ServiceId` or
  a logical name, decided by whether `-a` is present.
- **`-a` and `-i` are both optional**, not four hashed components as this
  section's own arithmetic (now corrected) implied: the real form has three
  hashed segments (`a`, `s`, `i`) plus the marker, leaving 27 characters for
  the nickname on an app-scoped host (37 with no `-i`). An omitted `-i`
  resolves at the destination to the service's one app-declared interface,
  rather than matching nothing as the pre-S3 code actually did.

The corrected grammar:

```
<nickname>-s<service-did-hash>[-i<interface-hash>]-roym1.<domain>
<nickname>-a<app-did-hash>-s<logical-service-name-hash>[-i<interface-hash>]-roym1.<domain>
```

The **routing key travels in a request header**, absent meaning unkeyed. It
fails the hostname test twice over:

- **Cardinality.** A routing key is a user, tenant, or entity id — unbounded.
  In a hostname each distinct key becomes a distinct origin, with its own
  connection (browser pools are keyed by origin), cookie jar, `localStorage`
  partition, and CORS preflights. One page making 50 keyed calls would open 50
  connections to one logical service.
- **It decides nothing about authority.** By §5 a caller entitled to a logical
  service is entitled to every member of it, so a wrong routing key cannot
  reach anything the caller could not already reach. It sends a legitimate
  request to the wrong place, which is what §6 defends against.

`generation` and the topology epoch fail the same test for the same reason —
they change on reconcile, and putting them in the hostname would break origin
stability.

With this, `Singleton` and `Redundant` services work over plain HTTP with no
client change at all, and a `Sharded` service without the header fails with the
resolver's existing, specific error rather than silently picking a member
([resolver.rs:675](../../crates/app_orchestration/src/resolver.rs#L675)).

**Test for anything added later:** if getting it wrong lets a caller reach
something it should not, it belongs in the hostname. If getting it wrong only
sends a legitimate request to the wrong place, a header is right.

## 8. Rejected: register logical services in the Community Registry

Publishing a member set under a logical name would put a fast-moving,
multi-valued record into a store built for slow-moving, single-valued,
self-signed identity records, and would need the `name → DID` admission rule §2
rejects. The registry's job stays "which node is this DID at", which it already
does well and which relocation does not disturb.

## 9. Rejected: Tier 2 returns a resolved physical address

Covered in §4: it re-couples failover to the control plane, moves load
balancing into the supervisor, and makes every answer cacheable for exactly one
routing key.

## 10. Rejected: make `app_instance_id` itself a DID

It would remove the second name and make the Tier-1 record self-signed with no
new concept. Rejected on ergonomics: `app_instance_id` is embedded in vault
names, MQTT topics, journal rows, `roymctl` arguments, and the display form of
every `LogicalServiceRef`. A `did:key:z6Mk…` in each of those makes vault names
unreadable and alert topics untypeable, for no gain that §1's two-level split
does not already deliver.

## 11. This is not the live directory ADR-0021 §6 rejected, and its trigger has not fired

ADR-0021 §6 leaves a pull-side directory unbuilt and names a **measured**
trigger for revisiting: binding convergence across dependents failing to stay
inside budget by delivery retry alone. That trigger has not fired, and nothing
here claims it has.

What §6 rejected is a directory that *intra-app* dependents query on the hot
path, replacing push. Push stays exactly as ADR-0021 specifies for every
dependent inside an app instance. This adds a path for callers **outside** the
instance, who have no push relationship to replace and today have no way in at
all. What they fetch is a signed, cacheable document with a TTL, not a hot-path
query against a live service.

The ADR this does change is ADR-0021 §7, which reasons about cross-app `Bind`
under the condition "no directory exists for A to observe B through". Once §3's
document exists, one does. §7's rule — A's owner owns the consequence of
depending on something it does not control — is unaffected; only the
observability premise improves.

**Consequences**:

- An app instance becomes addressable from outside itself, by any peer with a
  valid delegation — another app, or an ordinary client program — without the
  registry learning anything about application topology.
- The supervisor gains a second custody duty (the app master) on top of member
  masters, and handover becomes a key move rather than a config change. This is
  the price of the registry's one keying rule, and it is charged against
  machinery `export-master`/`import-master` already provide.
- Supervisor downtime degrades external resolution to "possibly stale" rather
  than "unavailable", but only for callers that have fetched the document
  before. A caller resolving an app for the first time while its supervisor is
  down still fails.
- `Sharded` becomes reachable end to end for the first time: a strategy
  expressible in a manifest, a member set discoverable from outside, a routing
  key expressible over HTTP, and an epoch that makes a mid-rebalance request
  detectable. None of those four is usable alone.
- Cross-app dependency binding becomes an authorization question rather than a
  structural one. `prepare_binding`'s intra-app refusal
  ([orchestration.rs:191](../../crates/control_plane/src/service/orchestration.rs#L191))
  can be replaced by a UCAN check, which is what the `dependency-binding` WIT
  record's separate `app-instance-id` field was reserved for.
