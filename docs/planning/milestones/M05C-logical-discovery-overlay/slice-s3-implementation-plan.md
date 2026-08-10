# Slice S3 Implementation Plan — the Logical Gateway Hostname, the Routing-Key Header, and the Coordinator Relay

**Status:** 📋 Planned (2026-08-09). Not started. Milestone:
[task.md](task.md). Milestone plan:
[implementation-plan.md](implementation-plan.md). Design of record:
[ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md) §7
(the hostname and the header) and §3 (the relay). Gate: **S2 Complete
(2026-08-08) — cleared**.

**Scope, from [task.md](task.md)'s slice table, verbatim:** "Gateway hostname
scheme (`-a…-s…-i…`) plus the routing-key request header; coordinator relay of
the document in the WebRTC bootstrap page."

**The one-sentence summary.** S2 gave a *program* a way to fetch and verify an
app's member set; S3 gives an ordinary HTTP client — including a browser, which
can hold no code of ours — the same answer, addressed entirely by hostname.

**Read before writing code:** ADR-0022 §7 in full, and
[slice-s2-implementation-plan.md](slice-s2-implementation-plan.md) §1 (D-S2-1,
D-S2-6, D-S2-7, D-S2-13), because S3 is the first consumer of all four.

---

## §0 — What ADR-0022, task.md, and the shipped tree leave open, understate, or state wrongly

Thirteen findings. **Four of them block the slice on their own** — 0.1, 0.2,
0.3, and 0.12 — and only the last of those is this plan's own defect rather
than the input documents'. Nothing in ADR-0022, task.md, or S2's plan states
any of the other three.

### 0.1 (Correctness, and it makes an exit criterion unsatisfiable as written) The hostname is parsed in two places and built in none

[task.md](task.md)'s migration-impact section and
[meta-implementation-plan.md:527](../../meta-implementation-plan.md) both say
the format is "centralised in `core::util` (build) and `core::protocol_utils`
(parse), so a client going through those helpers sees one change in one place",
and exit criterion 5 makes that testable: "S3's hostname change goes through
`core::util` / `core::protocol_utils` only, demonstrated by the diff."

Against the tree, both halves of that claim are false.

**Parse is duplicated four ways, not two.** The first revision of this section
said two; a review pass found the other half, in a language the Rust call-site
search never reaches.

| Parser | Used by |
|---|---|
| [`core::protocol_utils::parse_target_host`](../../../../crates/core/src/protocol_utils.rs#L68) | the WebRTC bootstrap handler ([bootstrap.rs:174](../../../../crates/coordinator_webrtc/src/bootstrap.rs#L174)) — its only caller |
| [`client_gateway::parse_target_service_and_interface`](../../../../crates/client_gateway/src/gateway.rs#L254) | the client gateway — a line-for-line copy of the one above |
| [`peer-proxy.js:503-509`](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L503) | the browser page's raw-tunnel path, to build a route preamble |
| [`peer-proxy.js:858-865`](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L858) | the browser page's service-worker path, same job |

The two Rust copies are the same right-to-left algorithm written twice, line
for line. The two JS copies are a cut-down version of it — they take the last
dash-segment of `location.hostname` if it starts with `i` and use the rest as
the interface name.

**The JS half is what makes this a correctness finding rather than tidiness.**
D-S3-13 puts a `-roym1` marker at the *end* of the label, so the last segment
is never `-i…` any more, and both JS sites silently produce
`interfaceName = ''` for **every** host — including one that explicitly
carried `-i`. Worse, the Playwright suite would probably still pass:
`miniapp-demo1-web` has one app-declared interface, so D-S3-15's
empty-interface rule covers for it. An explicit `-i` would stop working with
no failing test. Settled in D-S3-16, which deletes both JS parsers rather than
teaching them the new grammar.

**Build does not exist.**
[`core::util::generate_alias`](../../../../crates/core/src/util.rs#L54) builds
`<nickname>-p<hash>` and stops. Nothing in `core` ever produces the full host.
The `-i<hash>.localhost` tail is hand-formatted at four sites:

| Site | Line |
|---|---|
| `roymctl alias` | [commands.rs:218](../../../../apps/roymctl/src/commands.rs#L218) |
| `basic_lifecycle` (TCP proxy case) | [basic_lifecycle.rs:428](../../../../crates/substrate/tests/basic_lifecycle.rs#L428) |
| `basic_lifecycle` (greeter case) | [basic_lifecycle.rs:607](../../../../crates/substrate/tests/basic_lifecycle.rs#L607) |
| perf harness | [tcp_proxy_latency.rs:100](../../../../tests/perf/src/scenarios/tcp_proxy_latency.rs#L100) |

So "anything formatting host strings by hand breaks" (task.md) already
describes every builder in the tree, including the one the Playwright suite
runs (`roymctl alias`, called from
[global-setup.ts:151](../../../../crates/substrate/tests/e2e/global-setup.ts#L151)).

**Consequence, and it is phase 1 of this slice rather than a cleanup at the
end**: the centralisation task.md describes as an existing property has to be
*created* by S3 before the new segments are added to it. Doing it in the other
order means writing the `-a`/`-s` grammar twice on purpose. Decided in D-S3-1.

### 0.2 (Scope-changing, and it is this slice's sharpest finding) `-s<logical-service-name-hash>` is a one-way hash, and `resolve` takes a name

ADR-0022 §7 fixes the middle segment as "a hash of the **logical service
name**". S2's `resolve` verb takes the name as a string
([service.rs:5288](../../../../crates/app_supervisor/src/service.rs#L5288),
`LogicalServiceName::try_new(&service_name_str)`), and
`TopologyFetcher::fetch` takes a `&LogicalServiceName`
([topology_document.rs:174](../../../../crates/app_orchestration/src/topology_document.rs#L174)).

`short_hash` is SHA-256 truncated to five bytes
([util.rs:43](../../../../crates/core/src/util.rs#L43)). A caller that reads
the hostname has the hash and cannot produce the name. So the chain
hostname → Tier 1 → Tier 2 is broken at Tier 2, and nothing in ADR-0022,
task.md, or S2's plan says how it closes.

Three ways to close it:

| Option | Cost |
|---|---|
| Put the plain name in the hostname instead of a hash | `LogicalServiceName` permits `-` (it forbids only `/` and `#`, [models.rs:140](../../../../crates/app_orchestration/src/models.rs#L140)), and the grammar is parsed right-to-left by splitting on `-`. A name containing a dash would be split and only its last fragment popped as the `s` segment. Also unbounded length against a 63-character DNS label. Rejected |
| The caller keeps a name↔hash map | It has to be populated from somewhere, and the only source of an app's service names is the app's own supervisor. This is the option below with an extra hop and a cache that can be wrong |
| **The supervisor reverses the hash against its own plan** | It already holds every name (`plan.services[].logical_ref.service_name`, the same list [`service_topology`](../../../../crates/app_supervisor/src/topology.rs#L61) filters). One `if` in `handle_resolve`, no new verb, no schema change |

The third is also the tree's existing idiom for exactly this problem: an
interface hash on a route preamble is reversed at the destination by comparing
`short_hash(name) == req.interface` against the known set
([proxy.rs:599](../../../../crates/router/src/proxy.rs#L599),
[call_dedup.rs:188](../../../../crates/router/src/call_dedup.rs#L188)). The
same rule (the party that knows the candidate set does the reversing) gives the
same answer here. Decided in D-S3-3.

**The property that makes it safe, and it must be stated rather than assumed:**
a `short_hash` collision between two *different* logical services **inside one
app** is the only ambiguity this can produce, and authorization is per-app
today (D-S2-7's `synapp:<app-did>` grant covers every service of the app), so a
collision cannot cross an authorization boundary. It can still return the wrong
service's members, so an ambiguous hash is **refused**, never resolved to
whichever name sorted first.

### 0.3 (Correctness, and it blocks the slice end to end) Both of S3's new callers hold no credential, and `resolve` is authorized

D-S2-7 gates `resolve` on `supervisor/resolve` over `synapp:<app-did>`
([service.rs:5317](../../../../crates/app_supervisor/src/service.rs#L5317)),
with unknown-app and unauthorized returning the identical denial. S3's two new
callers cannot satisfy it:

- **The client gateway** presents the node's own DID and nothing else. Its own
  source says so: "the gateway proxies as a DID that holds nothing node-wide"
  ([gateway.rs:44](../../../../crates/client_gateway/src/gateway.rs#L44)'s
  `TODO(post-B0)`). `SyneroymClient::with_ucan` exists and the gateway never
  calls it.
- **The WebRTC coordinator** has no identity at all. `BootstrapState`
  ([bootstrap.rs:49](../../../../crates/coordinator_webrtc/src/bootstrap.rs#L49))
  holds an iroh `Endpoint`, a registry client, and a connection cache — the
  blind tunnel is deliberately blind, and the crate does not depend on
  `syneroym-sdk` at all.

Written as scoped, S3 therefore ships a hostname format whose every resolution
ends in `PERMISSION_DENIED`. That is not a detail to discover in review.

**There are two cases and they need two different answers** (settled with the
requester, 2026-08-09):

- **The supervisor is on the same node as the gateway** — every single-node
  deployment, and all local development. Here a config gate at substrate
  startup is right, and it costs one branch on machinery that already exists.
  `build_caller` grants `substrate/admin` at exactly one site, to a caller
  whose verified DID equals `[iam].admin_ucan_root`
  ([io.rs:185](../../../../crates/router/src/route_handler/io.rs#L185)). A
  second, deliberately narrower branch beside it grants a **bare
  `substrate:<node_did>` capability whose ability is `supervisor/resolve`, not
  `substrate/admin`**, to a caller whose verified DID is the node's own. Bare
  substrate scope short-circuits `Capability::grants`
  ([capability.rs:192](../../../../crates/ucan/src/capability.rs#L192)), so it
  covers `synapp:<any-app-did>` — and nothing else, because the ability is
  the only other half of the check. The node key cannot deploy, undeploy, or
  administer anything through it.
- **The supervisor is on a different node** — the reference scenario, and the
  ordinary product shape (a user's own substrate reaching an app hosted
  elsewhere). No local config can help: the check runs on the *remote*
  supervisor. This needs an operator-supplied `CapabilityToken` on disk, the
  same file `roymctl app resolve --ucan` already takes, loaded at role init.

Both, therefore. The gate makes single-node work with no credential at all;
the token file is what makes the two-node case work. Decided in D-S3-6.

**What neither fixes, and must be said:** a gateway can only resolve a *remote*
app whose supervisor's operator issued it a grant. Cross-organisation
resolution stays out of reach until S4 adds ADR-0022 §5's "open to all"
manifest declaration (already a backlog row, targeted S4). S3 makes the
*mechanism* work; the *reach* is S4's.

### 0.4 (Correctness, and it is the same defect the S2 post-merge review already found once) A `-a` host resolves through the one registry-lookup branch that is not bound to what was asked

`RegistryClient::lookup` checks that the record it got back is the record that
was asked for **only when the lookup key is a full DID**
([dht_registry.rs:372](../../../../crates/core/src/dht_registry.rs#L372)) — the
comment there says a shorthash alias lookup "cannot be checked this way by
construction". That check exists because S2's post-merge review found its
absence (finding 4, [status.md](status.md)).

A `-a<app-did-hash>` hostname carries a hash, not a DID, so the only lookup it
can do is by alias — landing in exactly the unchecked branch. A registry that
answers the alias `chat-<hash>` with some other app's perfectly valid,
perfectly self-signed Tier-1 record redirects the caller to a different app's
supervisor, and every later step (fetch, verify, route) succeeds against the
*wrong* app.

It is checkable, just not inside `lookup`: the caller holds `a_hash` and the
answer carries `service_id`, so `short_hash(record.info.service_id) == a_hash`
closes it. The same shape applies one tier down — the document that comes back
carries `service_name`, so `short_hash(doc.service_name) == s_hash` must hold
too, since `SignedTopologyDocument::verify` checks the app DID and the expiry
and nothing about *which service* was asked for. Decided in D-S3-5.

### 0.5 (Understated) The gateway cannot be handed the node's `LogicalResolver`, and should not be

`ClientGateway::init` runs inside `RuntimeServices::init`
([runtime.rs:243](../../../../crates/substrate/src/runtime.rs#L243)), which
runs **before** `setup_connection_router`
([runtime.rs:151-153](../../../../crates/substrate/src/runtime.rs#L151)), and
that is where `logical_resolver` is constructed
([runtime.rs:938-939](../../../../crates/substrate/src/runtime.rs#L938)). The
order is load-bearing and documented with a measurement attached: swapping it
was "tried first, reverted" and cost ~6s → 15-30s of startup
([runtime.rs:138-150](../../../../crates/substrate/src/runtime.rs#L138)).

So injecting the node's resolver means either reordering (refused above) or a
`set_resolver` setter in the shape `set_supervisor` already uses. Neither is
needed, because sharing is wrong on the merits: the node's resolver holds
`AppScope::Local(...)` entries replayed from this node's own bindings
([runtime.rs:898](../../../../crates/substrate/src/runtime.rs#L898)), and the
gateway only ever holds `AppScope::Foreign(...)`. Two disjoint key spaces
(D-S2-1 made them disjoint *by type*) with two different lifetimes. The gateway
owns its own. Decided in D-S3-7.

### 0.6 (Ambiguous — **settled with the requester, 2026-08-09**) "Coordinator relay of the document" means wiring the resolved DIDs into the shell page, not verifying it there

task.md and the meta plan both phrase S3's third item as "coordinator relay of
the document in the WebRTC bootstrap page", and ADR-0022 §3 lists the
coordinator "serving it inside a bootstrap page" among the relays that justify
the document form. Read strictly, that suggests the page receives the signed
document and checks it.

**It does not, and it should not.** The bootstrap page is a shell whose only
job is to register a service worker that redirects later fetches over WebRTC
or a WebSocket tunnel. What it needs is the correct DIDs interpolated into it —
`TARGET_PEER_ID` (the substrate to dial) and `TARGET_SERVICE_ID` (the service
to name in the route preamble,
[peer-proxy.js:510](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L510)
and
[:866](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L866)).
The page has no way to reach a registry or a supervisor before its own tunnel
exists, so the two-tier resolve has to happen in the coordinator regardless;
having the page then re-verify a document the coordinator already acted on
would authenticate the *shell page load*, which is explicitly not a goal.

So phase 4 is small: the coordinator learns to resolve a logical host through
Tier 1 and Tier 2, and interpolates the member DID it selected. **No
`crypto.subtle`, no JS canonicalizer, no z-base-32 in the browser** — an
earlier revision of this plan proposed all three and was wrong about what the
page is for.

**The honest consequence, recorded rather than papered over:** on the browser
path the coordinator stays trusted for the answer. That is exactly today's
posture (`TARGET_SERVICE_ID` is already interpolated and used verbatim), so S3
neither improves nor worsens it — but ADR-0022 §3's "every relay would become
trusted infrastructure" argument is therefore *not* discharged by the browser
path, only by the SDK/gateway path where `register_verified` runs. Backlog row,
§5.

### 0.7 (Stale, minor, but it drives a real limit) ADR-0022 §7's own length arithmetic does not match the form it prints

§7: "Four hashed components plus a nickname is close to the 63-character DNS
label limit, so truncation lengths are chosen deliberately rather than
inherited." The form printed three lines above has **three** hashed components
(`a`, `s`, `i`), not four, and no truncation length is stated anywhere.

The real budget: `short_hash` is `z32::encode(&sha256[..5])` = exactly 8
characters, so `-a<8>-s<8>-i<8>` costs 30 and D-S3-13's `-roym1` marker another
6, leaving **27 characters for the nickname** (37 when `-i` is omitted, which
D-S3-15 makes a real option rather than a broken one). The nickname on an app-scoped
host is the app's `AppInstanceId` (see D-S3-2), and `AppInstanceId` has no
length validation at all
([models.rs:98](../../../../crates/app_orchestration/src/models.rs#L98)).

No truncation is introduced: truncating a hash weakens the binding in 0.4, and
truncating a nickname breaks the alias lookup. The builder refuses instead, and
a backlog row proposes the length cap belongs on `AppInstanceId` where it can
fail at `submit` rather than at host-build time. Decided in D-S3-9.

### 0.8 (Scope-narrowing, and it must be declared rather than silently omitted) S3 does not put the topology epoch on the request

ADR-0022 §6: "a request carries the topology epoch it was resolved under". S3
is the first slice where a *request* resolved under an epoch exists, so a
reader will reasonably expect the header here, next to the routing-key header
that §7 does put in scope.

It is deliberately not built, and the reason is that D-C-4's ship-before-enforce
argument does not apply. That argument is about **wire formats that are
expensive to change after something depends on them** — a signed document's
field, a manifest's schema. An HTTP request header added by one proxy and read
by one member is additive at any time, at zero cost. Building it now would mean
splicing a header into the raw request bytes the gateway forwards verbatim
([gateway.rs:229](../../../../crates/client_gateway/src/gateway.rs#L229)) for
a reader that does not exist and whose shape S5 has not fixed — designing
against an imagined caller, which D-C-7 rejects by name for S4.

Recorded as a backlog row targeted at S5, and as a §6 entry, not as an
omission.

### 0.9 (Ambiguous, and reading it the obvious way breaks the tree) An app-scoped host does not replace an unscoped one

ADR-0022 §7 says "The gateway host **becomes**:" and prints only the
`-a…-s…-i…` form. Read as a replacement, that deletes addressing for a service
that belongs to no app instance -- which is every `roymctl svc deploy`, and
which is what
[basic_lifecycle.rs:428/607](../../../../crates/substrate/tests/basic_lifecycle.rs#L428),
[tcp_proxy_latency.rs:100](../../../../tests/perf/src/scenarios/tcp_proxy_latency.rs#L100),
`roymctl alias`, and the entire Playwright suite (via `global-setup.ts`) use
today.

Both targets stay addressable, permanently. What §0.10 *does* change is the
spelling: the segment naming an unscoped service is no longer `-p`.

### 0.10 (Requester-directed cleanup, 2026-08-09) The grammar has no marker, every segment is optional, and `-p` names a distinction that stops existing the moment `-a` does

Three defects in one grammar, worth fixing in the pass that already touches it
-- §0.1 moves every producer and consumer through two functions anyway, so the
marginal cost is near zero, and the cost of a *second* external format change
later is not.

**`p` means "pubkey hash", and that stops being a distinction.** The variable
is literally named `pubkeyhash`
([protocol_utils.rs:95](../../../../crates/core/src/protocol_utils.rs#L95)),
and `generate_alias`'s own doc says `{nickname}-p{shorthash}`
([util.rs:52](../../../../crates/core/src/util.rs#L52)). It made sense when
every addressable thing was a `did:key` and "the pubkey" was unambiguous. Once
`-a` exists the app DID is *also* a pubkey, so `p` no longer names an axis --
it just means "the hash that is not the app one", which is what `-s` says
directly.

**Nothing marks a host as ours.** `parse_target_host` turns any hostname into
an alias; its only defence is a hardcoded `if subdomain == "localhost" ||
subdomain == "127"`
([protocol_utils.rs:80](../../../../crates/core/src/protocol_utils.rs#L80)),
which is a symptom rather than a design. A gateway cannot tell a mistyped host
from a syneroym one, and there is no way to introduce a format v2 later
without guessing.

**Every segment is optional**, so a malformed host does not fail -- it
produces a half-built alias like `"{nickname}-p"`
([protocol_utils.rs:107](../../../../crates/core/src/protocol_utils.rs#L107)),
which then 404s at the registry with a confusing message instead of being
refused at the parse.

**On the DNS half of the request, one correction.** A DNS wildcard label must
be *exactly* `*` (RFC 4592 §2.1.1), so `*-xxxxx.syneroym.net` is not a record
any nameserver will accept -- partial-label wildcards do not exist. The good
news is that nothing is needed for that purpose: this whole scheme lives in
**one label**, so a single `*.syneroym.net` A/AAAA record already points every
host in it at one coordinator, and the parser is already domain-agnostic (it
strips `.localhost`, then takes the first label, so `.syneroym.net` works
unchanged today). The single-label choice is also what keeps one
`*.syneroym.net` wildcard **TLS certificate** sufficient -- a two-label scheme
would need `*.*.syneroym.net`, which RFC 6125 wildcard matching does not
support either.

So a marker earns its place on the other two grounds -- refusing a host that
is not ours, and being able to change the format again -- not on DNS routing.
Decided in D-S3-13 and D-S3-14.


### 0.11 (Correctness, and it makes an already-advertised convenience real) An omitted `-i` produces an empty interface that resolves to nothing

Today's grammar treats `-i` as optional and yields `interface == ""`
([protocol_utils.rs:112](../../../../crates/core/src/protocol_utils.rs#L112)),
which the gateway forwards straight into the route preamble. At the
destination, `EndpointRegistry::lookup` tries an exact match on `""` and then
a hash match on `""`
([local_registry.rs:196](../../../../crates/core/src/local_registry.rs#L196)),
and nothing is ever registered under either. **So the optional `-i` that
already exists is not a default -- it is a guaranteed failure**, and the first
revision of this plan responded by making `-i` required, which removes a
convenience instead of fixing it.

The convenience is worth having, and it is the same rule the other two hashed
segments already follow: **the party holding the candidate set does the
resolving.** A caller off the network cannot know a remote service's interface
names, and the substrate hosting it knows all of them.

**The obvious implementation does not work, and the reason is easy to miss.**
"The service has only one interface, so use it" cannot be `lookup_by_service`
returning a single entry: every deployed service, of every type, is
automatically registered under all six `NATIVE_CAPABILITY_INTERFACES`
(`data-layer`, `vault`, `app-config`, `blob-store`, `messaging`,
`http-native`) at deploy
([orchestration.rs:1739](../../../../crates/control_plane/src/service/orchestration.rs#L1739)),
so a service with one declared interface has seven registered endpoints and
never exactly one. The rule has to be "the only **app-declared** interface",
filtering that reserved set -- which `list_impl` already does for the same
reason
([orchestration.rs:3056](../../../../crates/control_plane/src/service/orchestration.rs#L3056)).

Two or more app-declared interfaces with no `-i` is ambiguous and is
**refused**, exactly as D-S3-3 refuses an ambiguous service-name hash.
Decided in D-S3-15.


### 0.12 (Correctness, and it is a defect in this plan's own first draft) Popping an optional `-a` off a nickname's last segment silently invents an app

The parser pseudo-code pops `-i`, then `-s`, then an optional `-a`, and
`pop_prefixed` accepts "starts with the letter and is longer than one
character". `-a` is popped **last**, so what it inspects is the nickname's own
final segment. For an unscoped host built from the nickname `data-api`:

```
data-api-s12345678-roym1
  pop marker; pop -i (absent); pop -s  ->  parts = [data, api]
  pop -a: "api" starts with 'a'        ->  app_did_hash = "pi"
```

It becomes a `TargetHost::App` with a garbage app hash and the nickname
`data`. `my-app`, `chat-admin`, and `data-api` all hit it. Today's grammar was
immune because `-p` was mandatory and popped last, so nothing ever inspected a
nickname segment.

**The fix, and the reason it cannot be applied uniformly.** `-a` and `-s`
always carry a `short_hash`, which is exactly 8 characters, so those two
segments must be exactly 9 (letter plus hash) — which is the same fixed-width
argument D-S3-13 already makes for the marker. **`-i` must stay permissive**:
`EndpointRegistry::lookup` tries an exact interface-name match before a hash
match, and `docs/developer-guide.md:271` documents a working host that uses
it, `…-iorchestrator.localhost`. A width rule on `-i` would break that.

Leaving `-i` permissive is safe *because* `-s` is required: `-i` is popped
first, and the segment it inspects is either a real `-i` or the required `-s`,
never a nickname segment. Only `-a` can reach a nickname, and only `-a` needs
the rule.

One residue the width rule does not close: a nickname whose final segment is
literally `a` + 8 characters is genuinely ambiguous, and no parser can
resolve it. Refused at *build* time instead — D-S3-9's shape, a host that
would be misread must never be minted.

### 0.13 (Efficiency, and a stated budget nobody checked) An app-scoped resolve does the same Tier-1 lookup twice

`resolve_app_host` looks the app's alias up to turn `a_hash` into an app DID.
It then calls `TopologyFetcher::fetch`, which does its **own** Tier-1 lookup
by app DID ([topology.rs:79](../../../../crates/sdk/src/topology.rs#L79)) and
then a Tier-3 lookup to connect to the supervisor. Three registry round-trips
on a cold resolve, and `state.app_dids` caches only the first.

The second is entirely redundant: the alias lookup already returns the whole
`SignedEndpointInfo`, whose `substrate_id` *is* the supervising node the
second lookup goes looking for.

task.md's **budget 2** says the Tier-1 lookup must stay "within the existing
registry-lookup budget — it is the same lookup shape as any other DID; no new
cost is acceptable". The first revision of this plan asserted budget 1 (a
fetch count) and said nothing about budget 2, and no test counted registry
calls. Settled in D-S3-17.

---

## §1 — S3 decisions

| ID | Decision |
|---|---|
| **D-S3-1** | **Centralise before extending.** Phase 1 moves *both* the build and the parse of every gateway host into `syneroym-core` — a new `TargetHost` enum and one `parse_target_host` in `core::protocol_utils`, and `generate_service_host`/`generate_app_host` in `core::util`, plus a reshaped `generate_alias` — and **deletes** `client_gateway::parse_target_service_and_interface` (§0.1). No new segment is added until every producer and consumer in the tree goes through those four functions. This is what makes exit criterion 5 ("demonstrated by the diff") true rather than aspirational. |
| **D-S3-2** | **One grammar, with the app and interface segments optional** (§0.9, §0.10, §0.11; requester-directed 2026-08-09): `<nickname>-a<short_hash(app_did)>-s<short_hash(service_name)>[-i<short_hash(interface)>]-roym1.<domain>` for a logical service inside an app, and the same string without `-a` for a service that belongs to no app instance, where `-s` then holds `short_hash(service_did)`. **`-s` always means "which service"**; whether it is a name hash or a DID hash follows from whether `-a` is present, and so does how it is reversed (the supervisor's own plan, D-S3-3, versus the registry's alias index). `-p` is gone -- it meant "pubkey hash", which stops naming an axis once the app DID is also a pubkey. **`-s` is the only required segment**; `-a`, `-i`, and `<nickname>` are all optional, and an omitted `-i` means "the service's one app-declared interface" (D-S3-15). `<nickname>` on an app-scoped host must be the app's `AppInstanceId`: `<nickname>-<a_hash>` must equal the registry alias the Tier-1 record was admitted under, which `register_endpoint` derives via `generate_alias(info.nickname, service_id)` ([registry.rs:208](../../../../crates/community_registry/src/registry.rs#L208)) from the record's own `nickname`, which `sign_tier1_record` sets to the app instance id ([tier1.rs:184](../../../../crates/app_supervisor/src/tier1.rs#L184)). That is what lets an app-scoped hostname resolve with **no new registry surface at all**. A wrong nickname simply fails to resolve. |
| **D-S3-3** | **`resolve` accepts a logical service name *or* its `short_hash`** (§0.2), reversed against the instance's own plan, the same way an interface hash is reversed at its destination. An exact name match always wins over a hash match; a hash matching two names in one plan is **refused** (`InvalidParams`), never resolved arbitrarily. `TopologyFetcher::fetch`'s signature is unchanged — a `LogicalServiceName` carrying a hash is still a valid `LogicalServiceName` — so no client-side type changes and no WIT signature change. |
| **D-S3-4** | **The routing key is the request header `X-Syneroym-Routing-Key`**, absent meaning unkeyed, its raw UTF-8 bytes passed to `LogicalResolver::resolve`'s `routing_key`. The name is defined once as `core::protocol_utils::ROUTING_KEY_HEADER` (there is no existing `X-Syneroym-*` header in the tree to be consistent with). The gateway **does not strip or rewrite it**: the forwarded bytes stay the bytes that arrived (§0.8), so a member can observe the key it was routed on. |
| **D-S3-5** | **Every hop is bound to what the hostname asked for** (§0.4). After the Tier-1 lookup: `short_hash(record.info.service_id) == a_hash`, or refuse. After the Tier-2 fetch: `short_hash(document.service_name) == s_hash`, or refuse — `SignedTopologyDocument::verify` checks the signer and the expiry, never *which service*. Both checks are the caller's, in the same shape as S2 post-merge finding 4's fix, and both are refusals, not warnings. |
| **D-S3-6** | **Two credentials, for two different cases** (§0.3). **(a) A same-node gate**, `[iam].grant_resolve_to_node_did: bool` (default `false`): `build_caller` grants a caller whose verified DID is this node's own a **bare `substrate:<node_did>` capability with ability `supervisor/resolve`** — deliberately *not* `substrate/admin`, so the node key gains resolve and nothing else. One branch beside the existing `admin_ucan_root` grant ([io.rs:185](../../../../crates/router/src/route_handler/io.rs#L185)). This makes every single-node deployment work with no credential file at all. **(b) A cross-node token**, `[roles.client_gateway] resolve_ucan` and `[roles.coordinator] resolve_ucan`, both `Option<PathBuf>`, both absent by default — the same `CapabilityToken` file `roymctl app resolve --ucan` takes, read at role init and presented via `RegistryTopologyFetcher::with_ucan`. Neither is a new authorization concept. **Absent + gate off** is a startup warning naming both keys, in the shape S1's no-registry warning uses ([D-S1-8](slice-s1-implementation-plan.md)). |
| **D-S3-7** | **The client gateway owns its own `LogicalResolver` over its own `StaticInventory`** (§0.5), and its own `RegistryTopologyFetcher`. Not the node's — the construction order forbids it and the key spaces are disjoint. This is the substrate-side `TopologyFetcher` holder D-S2-13 deferred, and closing that backlog row is S3's, not S4's. |
| **D-S3-8** | **The gateway re-fetches on any resolver miss, including expiry** — that is ADR-0022 §3's "on expiry try to refresh; if the refresh fails, keep using the previous document until `not_after`", which the S2 post-merge review recorded as implemented nowhere. It falls out of the gateway's own request path rather than needing a scheduler: a miss or an expiry error from `LogicalResolver::resolve` triggers `fetch_and_register` and one retry, so the refresh happens on demand at the exact moment an answer is needed. Nothing new is scheduled, and the "no network call after the first fetch" budget still holds inside `cache_ttl`. |
| **D-S3-9** | **The host builder refuses a label over 63 characters rather than truncating** (§0.7). Both builders return `Result<String>`; the error names the 63-character DNS label limit and the remaining nickname budget -- **27 characters** on an app-scoped host (`-roym1` is 6, the three hashed segments are 30; 37 with no `-i`). The **parser never enforces a length**: a host that arrived is a host that arrived. A backlog row proposes the cap belongs on `AppInstanceId`, where `submit` would refuse it long before a browser does. |
| **D-S3-10** | **Both targets stay addressable, permanently** (§0.9): a logical service inside an app, and a service that belongs to none. They are one grammar differing by the presence of `-a`, not two grammars. The developer guide presents them as two forms of one scheme, not one form plus a legacy note. |
| **D-S3-11** | **The coordinator resolves an app-scoped host and interpolates the resulting values into the shell page; the page verifies nothing** (§0.6, settled with the requester). `handle_bootstrap` does Tier 1 → Tier 2 → member selection → Tier 3, then fills `TARGET_SERVICE_ID` with the selected member DID and `TARGET_PEER_ID` with the substrate hosting it. It applies D-S3-5's two binding checks itself, since nothing downstream will. **Corrected from the first revision, which claimed `templates/` needs no change at all** -- it does, one field (D-S3-16), because the page derives the *interface* from the hostname rather than from an interpolated value. |
| **D-S3-12** | **No MQTT epoch-bump subscription in S3.** D-S2-10 targeted its backlog row at "S3, if a subscriber appears". The gateway is a subscriber-shaped thing, but subscribing would add an MQTT client to a component that has none, to shorten a 5-minute staleness window that D-S3-8's on-demand refresh already bounds by `cache_ttl`. The row stays open with its target re-pointed at "a consumer that needs sub-`cache_ttl` convergence", which nothing in this milestone is. |
| **D-S3-13** | **Every gateway host carries a trailing `-roym1` format marker** (§0.10). **Trailing, not leading, and the grammar decides it**: every segment is popped right to left, so a marker at the end is read *first* -- the version is known before the segments whose meaning it governs, the nickname is simply whatever remains after the known segments are gone, and there is no special case at index 0. It also keeps the nickname where it has always been, at the front of the label, so an operator reading a `Host:` header still sees the app name first. Checking it is the first thing the parser does, so a host that is not ours is refused rather than turned into a half-built registry alias -- which also deletes the hardcoded `localhost`/`127` special case standing in for this today. No ambiguity with the segments beside it: `roym1` is a fixed 5-character literal and every hash is exactly 8, so no interface hash or nickname segment can be mistaken for it -- the same fixed-width argument §0.12 then applies to `-a` and `-s` themselves. `roym` is the CLI an operator already types (`roymctl`) and the distinctive half of the product name, spelled in full rather than clipped. **The digit is a format version, not a second subdomain**: it is the first dash-segment of the same single label, on the same domain, behind the same wildcard record and the same gateway. If the grammar ever changes again -- a fourth segment, a different hash length, a different meaning for `-s` -- new hosts are minted with `-roym2` and the parser dispatches on that segment, so `chat-a1234abcd-s5678efgh-roym1.example.com` and `…-roym2.example.com` resolve side by side instead of the older one breaking. **A version marker can only be introduced at a break**, because adding one later is itself the break it exists to avoid -- and S3 is already a break (every host string changes here). That is the whole argument for the digit: one character out of 63, spent at the only moment it can be spent. **It buys nothing for DNS** -- a wildcard label must be exactly `*` (RFC 4592 §2.1.1), and `*.<domain>` already matches every host in this single-label scheme -- and the plan says so rather than letting a reader assume otherwise. |
| **D-S3-14** | **The registry alias loses its letter: `generate_alias` produces `<nickname>-<hash>`** (or a bare `<hash>` with no nickname). The letter was `p`, and keeping it would mean the hostname's `-a` segment reconstructs an alias spelled `-p`, and its `-s` segment one spelled `-p` too -- one letter standing for two different roles at a layer where it names neither. Without it, **both** segments reconstruct their alias the same way, `format!("{nickname}-{hash}")`. Unambiguous because `short_hash` is always exactly 8 characters and always last, so `rsplit_once('-')` recovers the pair even when the nickname contains dashes -- the same property the `-p` prefix gave. **Zero migration cost**: `RegistryState.aliases` is an in-memory `DashMap` ([registry.rs:57](../../../../crates/community_registry/src/registry.rs#L57)) rebuilt from re-registrations, so a restart is the whole migration, and the project's pre-release policy forbids a compatibility shim anyway. |
| **D-S3-15** | **An omitted `-i` means "the service's one app-declared interface", resolved at the destination** (§0.11). `EndpointRegistry` grows `resolve_interface(service_id, name) -> Option<String>` carrying all three cases in one place: exact name, `short_hash` of a name (today's behaviour, moved), and **empty**, which filters `NATIVE_CAPABILITY_INTERFACES` out of `lookup_by_service` and succeeds only when exactly one app-declared interface remains. Zero or two or more is `None` -- refused, never guessed, the same rule D-S3-3 applies to an ambiguous name hash. **The canonicalization happens at the hop that terminates the route** ([io.rs:344](../../../../crates/router/src/route_handler/io.rs#L344)), and every downstream check there sees the canonical name. A *relay* hop deliberately forwards `preamble.interface` untouched ([io.rs:372](../../../../crates/router/src/route_handler/io.rs#L372)): it does not host the service, so it does not know its interface names, exactly as it already forwards a `short_hash` unresolved today. So the property is "nothing past the terminating lookup sees an empty interface", **not** "nothing past ingress" -- an earlier revision of this plan claimed the latter, which is false on the multi-hop path the WebRTC coordinator actually uses. Test 81 pins the true property and the relay case together. The guest proxy path is deliberately unchanged: its interface gate runs before `lookup` ([proxy.rs:599](../../../../crates/router/src/proxy.rs#L599)) and an empty interface simply fails it, so this convenience lands on the external entry point only, which is where a caller who cannot know the interface names actually is. **One trade, stated rather than left to be discovered**: deploying a *second* interface for that service later turns every previously-working `-i`-less host into a refusal. It fails loudly (a clean `None`) rather than routing to the wrong interface, which is what makes it acceptable -- but such a host's meaning then depends on the app, not on the host alone, which sits at an angle to ADR-0022 §7's rule that the hostname carries what decides reachability. Backlog row; the fix is a manifest-declared default interface, a surface S4 is already opening. |
| **D-S3-16** | **The coordinator interpolates `TARGET_INTERFACE`, and the two JS hostname parsers are deleted** (§0.1). The page's raw-tunnel and service-worker paths each re-derive the interface from `location.hostname` ([peer-proxy.js:503](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L503), [:858](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L858)) and would both silently yield `''` under the trailing marker. Teaching them the new grammar would make JS a *third* implementation of it; interpolating the value the coordinator already parsed removes them instead, so the label is parsed in exactly one place, in one language. The per-request parse was always redundant anyway -- the page's origin does not change between fetches. This is what makes exit criterion 5 true for the browser path as well as the HTTP one. |
| **D-S3-17** | **One Tier-1 lookup per cold app-scoped resolve, not two** (§0.13, task.md budget 2). The alias lookup already returns the record whose `substrate_id` is the supervising node, so `RegistryTopologyFetcher` gains an inherent `fetch_via(supervisor_did, app_did, service_name)` that skips its own Tier 1, and its trait `fetch` becomes `lookup` + `fetch_via` -- unchanged for every existing caller. The gateway caches `(app_did, supervisor_did)` together in `app_dids` and calls `fetch_via`. Cold cost is then one Tier-1 lookup plus the two Tier-3 lookups any call to a remote DID already pays (supervisor, then member); warm cost is zero, since `state.clients` caches the member connection. Pinned by a **registry-call count** assertion, which nothing in this milestone had. |

---

## §2 — Phase plan

Five phases, strictly ordered. Phases 1-3 are the slice's spine; phase 4 is the
same logical path applied to the coordinator; phase 5 is the operator surface
and the proof.

| Phase | What | Why this order |
|---|---|---|
| **1** | One builder and one parser in `syneroym-core`, both host forms, every call site moved, the gateway's duplicate parser deleted | §0.1: extending two grammars is writing the new one twice |
| **2** | The destination resolves what the caller could not: `handle_resolve` accepts a hashed service name, and `EndpointRegistry` resolves an empty interface | §0.2 and §0.11, one principle -- the party holding the candidate set does the reversing. Nothing downstream can fetch until the first exists |
| **3** | The client gateway's app-scoped path: the same-node grant, the credential, the resolver, the fetcher, the routing-key header | The slice's actual deliverable |
| **4** | The coordinator resolves an app-scoped host, interpolates the member DID **and the interface** into the shell page, and the two JS hostname parsers are deleted | Reuses phase 3's fetcher and both binding checks. The browser side is a net **deletion** (D-S3-16), but it is not untouched — an earlier revision of this row claimed it was |
| **5** | `roymctl alias --service`, the Rust e2e, the Playwright e2e, docs, backlog | Proof and operator surface |

---

## §3 — Exact changes

### Phase 1 — one builder, one parser, in `syneroym-core`

#### 1a. `crates/core/src/protocol_utils.rs` — the parser

Add above `parse_target_host`:

```rust
/// The request header carrying a logical service's routing key (ADR-0022
/// §7). Absent means unkeyed. A header rather than a hostname segment
/// because a routing key is unbounded in cardinality and decides nothing
/// about authority -- a wrong one sends a legitimate request to the wrong
/// member, which is what the topology epoch defends against, not a
/// privilege boundary.
pub const ROUTING_KEY_HEADER: &str = "X-Syneroym-Routing-Key";

/// The trailing segment every gateway host carries, and a format version.
/// The grammar is parsed right to left, so this is popped *first* -- the
/// version is read before the segments whose meaning it governs, and a host
/// that is not ours is refused rather than turned into a half-built registry
/// alias. A later grammar ships as `-roym2` and both can be served at once.
pub const HOST_FORMAT_MARKER: &str = "roym1";

/// What a gateway host names (ADR-0022 §7). One grammar:
///
/// ```text
/// <nickname>-a<app-did-hash>-s<service-name-hash>[-i<interface-hash>]-roym1
/// <nickname>-s<service-did-hash>[-i<interface-hash>]-roym1
/// ```
///
/// `-s` always names the service; `-a`'s presence decides whether that is
/// a logical name inside an app (reversed by the app's own supervisor) or
/// a concrete service DID (reversed by the registry's alias index). `-s`
/// is the only required segment; an omitted `-i` means "the service's one
/// app-declared interface", resolved at the destination (D-S3-15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetHost {
    /// A service that belongs to no app instance.
    Service {
        /// `<nickname>-<hash>`, ready to pass to `RegistryClient::lookup`.
        lookup_alias: String,
        /// The `short_hash` of the interface name, or `""` when the `-i`
        /// segment was absent -- "the service's one app-declared
        /// interface", which only the destination can resolve.
        interface: String,
    },
    /// A logical service of an app instance.
    App {
        /// `<nickname>-<app_did_hash>`: the alias the app's Tier-1 record
        /// was admitted under, reconstructed so no new registry surface is
        /// needed. `community_registry::register_endpoint` derives the same
        /// string via `generate_alias`, and the Tier-1 record's `nickname`
        /// is the app instance id.
        app_lookup_alias: String,
        /// Kept beside the alias so a caller can bind the record it gets
        /// back to the host it was asked for -- `RegistryClient::lookup`
        /// cannot check an alias lookup itself, by construction (§0.4).
        app_did_hash: String,
        service_name_hash: String,
        interface: String,
    },
}

impl TargetHost {
    #[must_use]
    pub fn interface(&self) -> &str {
        match self {
            Self::Service { interface, .. } | Self::App { interface, .. } => interface,
        }
    }
}
```

Replace `parse_target_host`'s body. Signature changes from
`Option<(String, String)>` to `Option<TargetHost>`:

```
fn parse_target_host(host) -> Option<TargetHost>:
    host_str = strip trailing ":<digits>" if present
    label    = host_str.split('.').next()?      // domain-agnostic: works for
                                                // `.localhost`, `.syneroym.net`,
                                                // or a bare label

    parts = label.split('-').collect::<Vec<_>>()

    // Everything is popped from the right, marker first: the version is read
    // before the segments it governs, and the nickname is simply whatever is
    // left over. One direction, no special case at index 0.
    if parts.pop() != Some(HOST_FORMAT_MARKER): return None      // not ours

    // From the right. `-s` is required, so a malformed host returns None
    // instead of a half-built alias; `-i` and `-a` are optional.
    interface = pop_prefixed(&mut parts, 'i', AtLeastOne).unwrap_or_default()
    s_hash    = pop_prefixed(&mut parts, 's', Hash)?
    a_hash    = pop_prefixed(&mut parts, 'a', Hash)    // optional

    nickname = parts.join("-")                         // whatever remains; may be empty

    match a_hash:
        Some(a) => Some(App {
            app_lookup_alias:  join_alias(&nickname, &a),
            app_did_hash:      a,
            service_name_hash: s_hash,
            interface,
        })
        None => Some(Service {
            lookup_alias: join_alias(&nickname, &s_hash),
            interface,
        })

// `pop_prefixed(parts, c, width)`: if the last element starts with `c` and
// satisfies `width`, pop it and return the remainder; else None. Today's
// inlined `if let Some(last) = parts.last() && ...` block, named once
// instead of written four times.
//
// `-a` and `-s` use `Width::Hash` (the segment must be exactly 9 characters:
// the letter plus `short_hash`'s invariant 8), because `-a` is optional and
// popped last, so without the rule it inspects the *nickname's* own final
// segment -- `data-api-s12345678-roym1` would parse as an app-scoped host
// with `app_did_hash = "pi"` (§0.12).
//
// `-i` uses `Width::AtLeastOne` and stays permissive: `EndpointRegistry`
// resolves an exact interface *name* before a hash, and
// `docs/developer-guide.md` documents a working `…-iorchestrator` host. It
// is safe permissive precisely because `-s` is required -- `-i` is popped
// first, so the segment it inspects is either a real `-i` or the required
// `-s`, never a nickname segment.
//
// `join_alias(nickname, hash)`: `format!("{nickname}-{hash}")`, or the bare
// hash when the nickname is empty. The one place the hostname and the
// registry alias meet, so `generate_alias` must produce the identical
// string -- pinned by test 64.
```

Deleted with this: the `.localhost` strip (`split('.').next()` already does
it), the `subdomain == "localhost" || subdomain == "127"` special case
(the marker check subsumes it), and the `"{nickname}-p"` fallback for a
missing hash (that host is now `None`).

#### 1b. `crates/core/src/util.rs` — the builders

`short_hash` is unchanged. **`generate_alias` changes shape** (D-S3-14):

```rust
/// The registry's alias for a nicknamed DID: `<nickname>-<short_hash(did)>`,
/// or a bare `<short_hash(did)>` when there is no nickname.
///
/// Unambiguous even when the nickname contains dashes, because
/// `short_hash` is always exactly 8 characters and always last. Carries no
/// role letter deliberately: a gateway host reconstructs this same string
/// from either its `-a` segment or its `-s` segment, and a letter here
/// would have to stand for both.
#[must_use]
pub fn generate_alias(nickname: Option<&str>, service_id: &str) -> String;
```

Add:

```rust
/// The longest a single DNS label may be. A host label over this is
/// rejected by resolvers and browsers before anything of ours sees it.
pub const MAX_DNS_LABEL_LEN: usize = 63;

/// `<nickname>-s<service-did-hash>[-i<interface-hash>]-roym1.<domain>` -- a
/// service that belongs to no app instance.
///
/// # Errors
/// The label exceeds `MAX_DNS_LABEL_LEN`.
pub fn generate_service_host(
    nickname: Option<&str>,
    service_id: &str,
    /// `None` omits the `-i` segment: the destination resolves it to the
    /// service's one app-declared interface (D-S3-15).
    interface: Option<&str>,
    domain: &str,
) -> anyhow::Result<String>;

/// `<nickname>-a<app-did-hash>-s<service-name-hash>[-i<interface-hash>]-roym1.<domain>`
/// -- a logical service of an app instance (ADR-0022 §7).
///
/// `nickname` must be the app instance's `AppInstanceId`:
/// `<nickname>-<app did hash>` is the registry alias the app's Tier-1
/// record was admitted under, and reconstructing it is what lets a reader
/// recover the app DID with no new registry record type.
///
/// # Errors
/// The label exceeds `MAX_DNS_LABEL_LEN` (`-roym1` costs 6 and the three
/// hashed segments 30, so the nickname budget is 27 -- 37 with no `-i`).
pub fn generate_app_host(
    nickname: &str,
    app_did: &str,
    service_name: &str,
    /// `None` omits the `-i` segment (D-S3-15).
    interface: Option<&str>,
    domain: &str,
) -> anyhow::Result<String>;
```

Both build the label, then apply two refusals before appending `.{domain}`:

1. `ensure!(label.len() <= MAX_DNS_LABEL_LEN, ...)`, naming the limit and the
   actual length.
2. **The nickname's final dash-segment must not look like an `-a` segment** —
   `a` followed by exactly 8 characters. §0.12's width rule makes every other
   nickname parse correctly, and this one case is genuinely ambiguous to any
   parser, so it is refused where it is minted rather than misread where it is
   used. The message names the offending segment. Only `a` needs the guard:
   `-s` is required and `-i` is popped before it, so neither can ever inspect
   a nickname segment.

`domain` is a parameter rather than a hardcoded `"localhost"` because the
same scheme is meant to work under a real wildcard record (§0.10):
`roymctl alias` takes `--domain` defaulting to `localhost`, and the e2e/perf
call sites pass `"localhost"` explicitly.

**`interface` is `Option` on both builders**, matching the grammar: `None`
omits the segment. `roymctl alias` keeps its existing "no `--interface`"
branch, which now prints a *host* with no `-i` rather than a bare alias --
strictly more useful, since that host works (D-S3-15) where the bare alias
was never a host at all.

#### 1c. Call sites

| File:line | Change |
|---|---|
| [gateway.rs:254-310](../../../../crates/client_gateway/src/gateway.rs#L254) | **Delete `parse_target_service_and_interface` entirely.** `handle_connection` reads the `Host` header itself (the loop it lives in already has the parsed `Request`) and calls `protocol_utils::parse_target_host` |
| [gateway.rs:178](../../../../crates/client_gateway/src/gateway.rs#L178) | `let target = match parse_target_host(host_header) { Some(t) => t, None => return write_json_rpc_error(&mut stream, 400, "Missing or invalid Host header").await }` — the 400 body is unchanged |
| [bootstrap.rs:174](../../../../crates/coordinator_webrtc/src/bootstrap.rs#L174) | Matches on `TargetHost` instead of destructuring a tuple; see phase 4c for the `None` arm, whose fallback must no longer apply to a failed app-scoped resolve |
| [registry.rs:208](../../../../crates/community_registry/src/registry.rs#L208) | No source change — `generate_alias`'s new output flows through. Its **collision check and the `aliases.retain` sweep are unaffected**, since the uniqueness property is unchanged |
| [commands.rs:214-222](../../../../apps/roymctl/src/commands.rs#L214) | `Commands::Alias` gains `--service` and `--domain`; see phase 5 |
| [commands/registry.rs:92](../../../../apps/roymctl/src/commands/registry.rs#L92) | Display only; picks up the new alias shape with no source change |
| [basic_lifecycle.rs:426-428](../../../../crates/substrate/tests/basic_lifecycle.rs#L426) | `util::generate_service_host(Some("tcp-demo-app"), &app_service_id, Some("default"), "localhost").unwrap()` |
| [basic_lifecycle.rs:605-607](../../../../crates/substrate/tests/basic_lifecycle.rs#L605) | `util::generate_service_host(Some(nickname), app_service_id, Some(GREETER_INTERFACE_NAME), "localhost").unwrap()` |
| [tcp_proxy_latency.rs:98-100](../../../../tests/perf/src/scenarios/tcp_proxy_latency.rs#L98) | `util::generate_service_host(Some("tcp-perf"), &app_service_id, Some("default"), "localhost").unwrap()` |
| [global-setup.ts:151](../../../../crates/substrate/tests/e2e/global-setup.ts#L151) | Unchanged source, new output — it already shells out to `roymctl alias --interface http`, so it picks up `-roym1` and `-s` for free. **The one call site that would silently keep working while testing the old format if `roymctl` were forgotten**, so phase 1 lands `roymctl` and this together |
| [global-setup-multihop.ts:279](../../../../crates/substrate/tests/e2e/global-setup-multihop.ts#L279), [:293](../../../../crates/substrate/tests/e2e/global-setup-multihop.ts#L293) | Same shape, same "unchanged source, new output". Its consumer is [multi-hop.spec.ts:18](../../../../crates/substrate/tests/e2e/tests/multi-hop.spec.ts#L18)/[:59](../../../../crates/substrate/tests/e2e/tests/multi-hop.spec.ts#L59), which navigates to `http://${demo1Alias}:7662/` — so the `roymctl alias` output **is** the bootstrap hostname, and this is the suite D-S3-16's JS fix is load-bearing for. Missed by the first revision of this table |
| [registry.rs:477](../../../../crates/community_registry/src/registry.rs#L477), [:612](../../../../crates/community_registry/src/registry.rs#L612), [:643](../../../../crates/community_registry/src/registry.rs#L643), [:693](../../../../crates/community_registry/src/registry.rs#L693) (tests) | Four hand-built aliases — `format!("alice-p{service_hash}")`, `format!("p{service_hash}")` twice, `format!("member-one-p{master_hash}")` — all broken by D-S3-14's reshape. Rebuild each through `util::generate_alias` rather than re-hand-writing the new spelling, so the next reshape moves one function. The `p{hash}`-only pair keeps its meaning (a nickname-less alias must not match a record registered *with* a nickname); it is now spelled `{hash}` |

After this phase, `short_hash` has no caller that formats a host by hand,
which is exit criterion 5's "demonstrated by the diff".


### Phase 2 — the destination resolves what the caller could not

#### 2a. The service name, at the supervisor

**File:** `crates/app_supervisor/src/topology.rs`.

```rust
/// Resolves the `service_name` a caller supplied against the names this
/// plan actually declares, accepting either the exact name or its
/// `short_hash` (ADR-0022 §7 puts a hash of the logical service name in
/// the gateway hostname, and a hash cannot be reversed by the caller --
/// the party holding the candidate set does the reversing, exactly as an
/// interface hash is reversed at its destination).
///
/// An exact match always wins. A hash matching two declared names is
/// refused rather than resolved to whichever sorted first: `short_hash` is
/// a five-byte SHA-256 prefix, and answering with the wrong service's
/// members is worse than answering with nothing.
pub fn resolve_service_name(
    plan: &DeploymentPlan,
    supplied: &LogicalServiceName,
) -> Result<LogicalServiceName, TopologyBuildError>;
```

`TopologyBuildError` gains one variant:

```rust
/// Two declared service names share a `short_hash`, so a hashed lookup
/// cannot name one of them.
AmbiguousHash(LogicalServiceName),
```

with a `Display` arm naming both colliding service names.

Pseudo-code:

```
fn resolve_service_name(plan, supplied):
    names: BTreeSet<&LogicalServiceName> = plan.services.iter()
        .map(|s| &s.logical_ref.service_name).collect()
    if names.contains(supplied): return Ok(supplied.clone())

    matches: Vec<_> = names.iter()
        .filter(|n| util::short_hash(n.as_str()) == supplied.as_str())
        .collect()
    match matches.len():
        0 => Err(NoSuchService(supplied.clone()))
        1 => Ok(matches[0].clone())
        _ => Err(AmbiguousHash(supplied.clone()))
```

**File:** `crates/app_supervisor/src/service.rs`, `handle_resolve`.

The supplied name must be canonicalised **inside** the two-attempt loop, since
each attempt re-reads `state.plan_json`
([service.rs:5345](../../../../crates/app_supervisor/src/service.rs#L5345)) and
a `submit` landing between attempts can change the declared names:

```
// was:   let t = topology::service_topology(&plan, &service_name)...
// now:
let resolved_name = topology::resolve_service_name(&plan, &service_name)
    .map_err(map_topology_err)?;
let t = topology::service_topology(&plan, &resolved_name).map_err(map_topology_err)?;
```

`resolved_name` then replaces `service_name` at every later use in the function
— the `initialise_topology_epoch` key, the `signed_documents` cache key, and
`TopologyDocument.service_name` — so the **document always names the real
service name, never the hash the caller sent**. That property is what lets the
gateway's D-S3-5 check (`short_hash(doc.service_name) == s_hash`) be meaningful
rather than tautological.

`map_topology_err` gains the new variant: `AmbiguousHash` is caller input, so
it maps to `RpcError::InvalidParams` alongside `NoSuchService`.

`supervisor.wit` is **unchanged**: `resolve`'s parameter is already a string
and its documentation is the only thing that moves (one sentence saying a
`short_hash` of the name is accepted).

#### 2b. The interface, at the substrate hosting it

**File:** `crates/core/src/local_registry.rs`.

`EndpointRegistry::lookup`'s two-branch body (exact, then hash) becomes three,
factored out so the rule has one home:

```rust
/// Canonicalizes the interface a caller named into one this service
/// actually registered. Three inputs, one rule -- the party holding the
/// candidate set does the reversing:
///
/// - an exact registered name;
/// - a `short_hash` of one (a caller off the network carries hashes, not
///   names);
/// - **empty**, meaning "this service's one app-declared interface"
///   (ADR-0022 §7's hostname omits `-i` when a caller has nothing to say
///   about it).
///
/// The empty case filters `NATIVE_CAPABILITY_INTERFACES`, which every
/// deployed service is registered under regardless of type, so "only one"
/// means only one the app itself declared. Zero or two or more is `None`:
/// an ambiguous interface is refused, never guessed.
#[must_use]
pub fn resolve_interface(&self, service_id: &str, interface_name: &str) -> Option<String>;
```

```
fn resolve_interface(service_id, interface_name):
    if interface_name.is_empty():
        declared = lookup_by_service(service_id)
            .filter(|(name, _)| !NATIVE_CAPABILITY_INTERFACES.contains(name))
        return if declared.len() == 1 { Some(declared[0].0) } else { None }

    if active_endpoints.contains((service_id, interface_name)):
        return Some(interface_name)

    interface_hashes.get((service_id, interface_name))    // the hash branch
```

`lookup` keeps its signature and becomes `resolve_interface` followed by one
`active_endpoints` read, so every existing caller is unchanged.

**File:** `crates/router/src/route_handler/io.rs`.

`preamble.interface` is canonicalized at the hop that **terminates** the route
([io.rs:344](../../../../crates/router/src/route_handler/io.rs#L344)), and the
canonical name is used for every downstream check and for dispatch there. That
is the reason the empty case lives here rather than in the gateway, which does
not have the target's registry — the target is on another node.

**A relay hop is deliberately different.** On a local miss the router forwards
the original preamble to the next hop untouched
([io.rs:372](../../../../crates/router/src/route_handler/io.rs#L372)), empty
interface and all, because it does not host the service and cannot know its
interface names — exactly as it already forwards a `short_hash` unresolved.
So the property is *"nothing past the terminating lookup sees an empty
interface"*, not *"nothing past ingress"*. An earlier revision of this plan
asserted the latter, which is false on the multi-hop path the WebRTC
coordinator actually uses. **Test 81** pins both halves.

**Deliberately not changed:** the guest proxy path
([proxy.rs:599](../../../../crates/router/src/proxy.rs#L599)). Its interface
gate runs before `lookup`, so an empty interface from a guest fails it, and
that is the right answer — a guest names a declared dependency and has no
excuse for not naming its interface. The convenience belongs to the external
entry point, where the caller genuinely cannot know the names.


### Phase 3 — the client gateway's app-scoped path

#### 3a. Config and the same-node grant

**`crates/core/src/config.rs`, `IamConfig`**
([config.rs:1136](../../../../crates/core/src/config.rs#L1136)):

```rust
pub struct IamConfig {
    pub admin_ucan_root: Option<String>,
    /// Grants a caller whose verified DID is **this node's own** the
    /// ability `supervisor/resolve`, node-wide (ADR-0022 §5, D-S2-7).
    ///
    /// This is what lets a same-node client gateway or WebRTC coordinator
    /// resolve a logical (`-a…-s…`) hostname for an app whose supervisor
    /// runs here, with no credential file. Deliberately **not**
    /// `substrate/admin`: the grant is a bare `substrate:<node_did>`
    /// resource, which short-circuits `Capability::grants` and therefore
    /// covers `synapp:<any-app-did>` -- but its *ability* is only
    /// `supervisor/resolve`, so the node's own key gains resolution and
    /// nothing else. Says nothing about apps supervised elsewhere; those
    /// need `resolve_ucan` below, because the check runs on the remote
    /// supervisor.
    #[serde(default)]
    pub grant_resolve_to_node_did: bool,
}
```

**`crates/router/src/route_handler/io.rs`**, immediately after the existing
`admin_root == Some(id.master_did.as_str())` branch
([io.rs:185](../../../../crates/router/src/route_handler/io.rs#L185)):

```rust
if grant_resolve_to_node_did && id.master_did == node_did {
    session.capabilities.push(Capability {
        with: ResourceUri::substrate(node_did),          // bare -> covers any resource
        can: Ability(Ability::SUPERVISOR_RESOLVE.to_string()),
        caveats: None,
    });
}
```

`grant_resolve_to_node_did` reaches here the same way `admin_ucan_root` does,
through the struct that already carries it to
[io.rs:319](../../../../crates/router/src/route_handler/io.rs#L319).

**`crates/core/src/config.rs`, `ClientGatewayRole`**
([config.rs:928](../../../../crates/core/src/config.rs#L928)):

```rust
pub struct ClientGatewayRole {
    pub http_port: u16,
    /// Path to a `CapabilityToken` granting `supervisor/resolve` on apps
    /// supervised by *other* nodes. Not needed for apps supervised by this
    /// node -- `[iam].grant_resolve_to_node_did` covers those. Absent, with
    /// that gate off too, means every logical hostname is refused by the
    /// supervisor it reaches; a startup warning names both keys. Unscoped
    /// (`-s` only) hostnames are unaffected either way.
    #[serde(default)]
    pub resolve_ucan: Option<PathBuf>,
}
```

Both `Default` impls gain the new field. `#[serde(default)]` is already on both
structs, so no existing config file changes.

#### 3b. `crates/sdk/src/topology.rs` — one Tier-1 lookup

`RegistryTopologyFetcher` gains an inherent method, and its trait `impl`
becomes a two-liner over it (D-S3-17):

```rust
impl RegistryTopologyFetcher {
    /// Tier 2 only, against a supervisor the caller has already resolved.
    /// A caller that reached this app through its Tier-1 record already
    /// holds `substrate_id`; making it round-trip the registry again to
    /// rediscover the same value is the duplication task.md's budget 2
    /// forbids.
    pub async fn fetch_via(
        &self,
        supervisor_did: &str,
        app_did: &AppDid,
        service_name: &LogicalServiceName,
    ) -> Result<SignedTopologyDocument>;
}

#[async_trait::async_trait]
impl TopologyFetcher for RegistryTopologyFetcher {
    async fn fetch(&self, app_did, service_name) -> Result<SignedTopologyDocument> {
        let tier1 = RegistryClient::new(false, Some(self.registry_url.clone()))
            .lookup(app_did.as_str(), false).await?;
        self.fetch_via(&tier1.info.substrate_id, app_did, service_name).await
    }
}
```

`fetch`'s body moves into `fetch_via` unchanged from the `SyneroymClient`
construction onward, so `roymctl app resolve` and every other S2 caller
behaves identically. **The existing `tier1.verify()` on that path is
redundant** for the same reason phase 3e's is
([dht_registry.rs:359](../../../../crates/core/src/dht_registry.rs#L359)
already verifies and fails fast) — left alone rather than removed, since it is
S2's line and harmless.

#### 3c. `crates/client_gateway/Cargo.toml`

Add `syneroym-app-orchestration.workspace = true`. No cycle:
`app_orchestration` depends only on `syneroym-identity`, and `sdk` (already a
dependency here) already depends on `app_orchestration`.

#### 3d. `GatewayState`

```rust
struct GatewayState {
    registry_url: String,
    clients: DashMap<String, Arc<Mutex<SyneroymClient>>>,
    identity: Identity,
    /// Foreign topology entries only (`AppScope::Foreign`), learned from
    /// verified Tier-2 documents. Deliberately not the node's resolver:
    /// `ClientGateway::init` runs before `setup_connection_router` builds
    /// that one, an order with a measured startup cost attached, and the
    /// two key spaces are disjoint by type anyway (`AppScope`).
    resolver: LogicalResolver,
    /// D-S2-13's substrate-side `TopologyFetcher` holder. `None` when no
    /// registry is configured, which is the same condition that makes
    /// Tier 1 unresolvable.
    fetcher: Option<RegistryTopologyFetcher>,
    /// `short_hash(app_did)` -> the app DID a Tier-1 lookup returned **and
    /// the substrate supervising it**, both from the one record, so a
    /// repeat request re-resolves neither (D-S3-17). Bound to the hash at
    /// insert time (D-S3-5), so a cache hit is as checked as a miss.
    app_dids: DashMap<String, (AppDid, String)>,
    /// `(app_did, short_hash(service_name))` -> the real service name, as
    /// carried by a verified document. Only ever written from a document
    /// that passed the `short_hash(name) == hash` check.
    service_names: DashMap<(AppDid, String), LogicalServiceName>,
}
```

`ClientGateway::init` builds `resolver: LogicalResolver::new(Arc::new(StaticInventory::new()))`,
and `fetcher` as:

```
if registry_url.is_empty() { None }
else {
    let mut f = RegistryTopologyFetcher::new(registry_url.clone())
        .with_identity(&identity);
    match config.roles.client_gateway.as_ref().and_then(|g| g.resolve_ucan.as_ref()) {
        Some(path) => f = f.with_ucan(read_capability_token(path)?),
        // Only a warning when neither credential exists. With the
        // same-node gate on, a gateway with no token still resolves every
        // app this node supervises, which is the whole single-node case.
        None if !config.iam.grant_resolve_to_node_did => warn!(
            "client gateway has neither `roles.client_gateway.resolve_ucan` nor \
             `iam.grant_resolve_to_node_did`; app-scoped (-a…-s…) hostnames will \
             be refused by any supervisor they reach. Unscoped (-s only) \
             hostnames are unaffected."
        ),
        None => debug!(
            "client gateway has no `resolve_ucan`; app-scoped hostnames will resolve \
             only for apps supervised by this node"
        ),
    }
    Some(f)
}
```

`with_identity(&identity)` matters for both credentials: the fetcher presents
the gateway's node DID, which is what `grant_resolve_to_node_did` matches on
and what a `resolve_ucan` token's `audience_did` must name.

`read_capability_token` mirrors `roymctl`'s own `--ucan` loading (read the
file, `serde_json::from_str::<CapabilityToken>`); `syneroym-rpc` is already a
dependency of this crate and re-exports `CapabilityToken`.

#### 3e. `handle_connection`

The `Ok(Status::Complete(_))` arm becomes:

```
let host_header = header value of "host", or 400 as today
let target = parse_target_host(host_header) or 400 as today
let routing_key: Option<Vec<u8>> = header ROUTING_KEY_HEADER's raw value, cloned

let (service_id, interface) = match target {
    TargetHost::Service { lookup_alias, interface } => (lookup_alias, interface),
    TargetHost::App { app_lookup_alias, app_did_hash, service_name_hash, interface } => {
        match resolve_app_host(&state, &app_lookup_alias, &app_did_hash,
                              &service_name_hash, routing_key.as_deref()).await {
            Ok(member_did) => (member_did, interface),
            Err(e) => {
                error!("gateway failed to resolve logical host {host_header}: {e:#}");
                return write_json_rpc_error(&mut stream, 502, "Bad Gateway").await;
            }
        }
    }
};
// everything from `state.clients.entry(service_id.clone())` onward is unchanged
```

A physical host's `service_id` is an alias the downstream `SyneroymClient`
resolves; a logical host's is already a member DID, which the same client
resolves through the same Tier-3 lookup. **The client cache keys on that string
either way, so a logical target caches per member DID** — which is what makes
`Redundant` round-robin produce two cached connections rather than one, and is
correct.

`resolve_app_host`, the only new non-trivial logic:

```
async fn resolve_app_host(state, app_lookup_alias, a_hash, s_hash, routing_key)
    -> Result<String /* member ServiceId */>
{
    let fetcher = state.fetcher.as_ref()
        .context("no community registry configured; logical hostnames need Tier 1")?;

    // ── Tier 1 (cached) ──────────────────────────────────────────────
    let (app_did, supervisor_did) = match state.app_dids.get(a_hash) {
        Some(e) => e.clone(),
        None => {
            let registry = RegistryClient::new(false, Some(state.registry_url.clone()));
            let rec = registry.lookup(app_lookup_alias, false).await
                .with_context(|| format!("Tier 1 alias lookup '{app_lookup_alias}' failed"))?;
            // No `rec.verify()` here: `RegistryClient::lookup` already
            // verifies and fails fast on both branches
            // (`dht_registry.rs:359`), so re-verifying would only suggest it
            // does not. The check below is the one thing genuinely being
            // added.
            // D-S3-5: `RegistryClient::lookup` cannot bind an *alias*
            // lookup to what was asked for, so bind it here.
            ensure!(util::short_hash(&rec.info.service_id) == a_hash,
                "registry answered alias '{app_lookup_alias}' with '{}', whose hash is not \
                 the '-a{a_hash}' this host named", rec.info.service_id);
            let did = AppDid::try_new(rec.info.service_id.as_str())?;
            // Cache the supervising node alongside the DID (D-S3-17): this
            // record's `substrate_id` is exactly what `fetch`'s own Tier-1
            // lookup would go and re-fetch.
            state.app_dids.insert(
                a_hash.to_string(), (did.clone(), rec.info.substrate_id.clone()));
            (did, rec.info.substrate_id)
        }
    };

    // ── Tier 2 (cached in the resolver) ──────────────────────────────
    // A known name lets the resolver answer with no network at all --
    // task.md budget 1. An unknown name, a cache miss, or an expiry
    // (`not_after` past) all fall through to one fetch: D-S3-8's
    // on-demand refresh.
    if let Some(name) = state.service_names.get(&(app_did.clone(), s_hash.to_string()))
        && let Ok(member) = state.resolver.resolve(
               &TopologyKey::foreign(app_did.clone(), name.clone()), routing_key)
    {
        return Ok(member.to_string());   // `ServiceId` has `as_str`/`Display`, no `into_inner`
    }

    // `fetch` takes a `LogicalServiceName`; a hash is a valid one (8 z32
    // characters, so non-empty and free of `/` and `#`), and the
    // supervisor reverses it (D-S3-3). `try_new`, not `new`: `new` panics,
    // and this value comes off the network.
    // `fetch_via`, not `fetch`: the supervising node came back with the
    // Tier-1 record above, so `fetch`'s own Tier-1 lookup would be the same
    // round-trip twice (D-S3-17, task.md budget 2).
    let signed = fetcher
        .fetch_via(&supervisor_did, &app_did, &LogicalServiceName::try_new(s_hash)?)
        .await?;
    // D-S3-5's second half: `verify` checks the signer and the expiry,
    // never *which service* was asked for.
    ensure!(util::short_hash(signed.document.service_name.as_str()) == s_hash,
        "supervisor answered '-s{s_hash}' with service '{}'",
        signed.document.service_name);
    let key = register_verified(&state.resolver, &signed, &app_did, None)?;
    state.service_names.insert(
        (app_did.clone(), s_hash.to_string()), signed.document.service_name.clone());

    Ok(state.resolver.resolve(&key, routing_key)?.to_string())
}
```

Note the `Sharded`-without-a-key case needs no handling here: the final
`resolver.resolve` returns the resolver's own specific error
([resolver.rs](../../../../crates/app_orchestration/src/resolver.rs)), which
surfaces as the 502 above with that message logged — ADR-0022 §7's "fails with
the resolver's existing, specific error rather than silently picking a member".

`ClientGateway::shutdown` is unchanged; the resolver and the caches are plain
in-memory state with nothing to close.

### Phase 4 — the coordinator resolves an app-scoped host

Mostly Rust-side. `templates/` gets **one** change, and it deletes code rather
than adding it (D-S3-16, §0.1).

#### 4a. `crates/coordinator_webrtc/Cargo.toml`

Add `syneroym-sdk.workspace = true` and
`syneroym-app-orchestration.workspace = true`. No cycle: `sdk` depends on
`core`/`rpc`/`router`/`wit-interfaces`/`identity`/`app-orchestration`, none of
which reach `coordinator_webrtc`; `coordinator_iroh` already depends on `sdk`
the same way.

#### 4b. `BootstrapState` and `CoordinatorRole`

`CoordinatorRole` gains `resolve_ucan: Option<PathBuf>` with the same doc and
default as 3a. `BootstrapState`
([bootstrap.rs:49](../../../../crates/coordinator_webrtc/src/bootstrap.rs#L49))
gains two fields, built in `coordinator.rs` where the state is assembled:

```rust
/// Tier-1 → Tier-2 fetcher for logical (`-a…-s…`) hostnames. `None` when
/// no registry is configured, matching `registry_url`'s own condition.
pub topology_fetcher: Option<RegistryTopologyFetcher>,
/// Foreign topology entries, so a second page load for the same logical
/// host makes no network call. Owned here for the same reason the client
/// gateway owns its own (D-S3-7).
pub resolver: LogicalResolver,
```

**The coordinator's identity, and it is the one new thing this phase needs.**
The coordinator has no identity today — the blind tunnel is deliberately
blind — but `resolve` is authorized, so the fetch needs one. It reuses the
node's own key file through the same helper the client gateway already has
(`load_or_generate_node_identity`,
[gateway.rs:44](../../../../crates/client_gateway/src/gateway.rs#L44) — lift it
into `syneroym-core` rather than copy it a second time). Using the **node key,
not a fresh one**, is what makes `[iam].grant_resolve_to_node_did` cover the
coordinator as well as the gateway with one config key, and it means a
coordinator restart does not invalidate an operator's `resolve_ucan` token.

#### 4c. `handle_bootstrap`

```
match parse_target_host(&host) {
    None | Some(Service { .. }) => { …today's body, unchanged… }
    Some(App { app_lookup_alias, app_did_hash, service_name_hash, .. }) => {
        // Exactly phase 3's `resolve_app_host`, including both D-S3-5
        // binding checks -- lifted into a shared helper rather than
        // written twice, since this is the consumer most likely to get
        // them subtly different. Returns the selected member DID.
        let member_did = resolve_app_host(&state, &app_lookup_alias,
                                         &app_did_hash, &service_name_hash, None).await?;

        // Tier 3, exactly as the physical path already does it: the
        // member DID's own endpoint record names the substrate hosting it.
        let rec = state.registry_client.lookup(&member_did, true).await?;
        target_peer_id    = rec.info.substrate_id;
        target_service_id = member_did;
    }
}
```

`target_pubkey_hex` and the signaling URL are computed from `target_peer_id`
afterwards, unchanged.

#### 4d. `TARGET_INTERFACE`, and deleting the two JS parsers

`PeerProxyTemplate` gains one field:

```rust
/// The interface the route preamble names, already resolved from the
/// hostname by `parse_target_host`. Empty when the host carried no `-i`,
/// which the destination resolves (D-S3-15).
///
/// Interpolated rather than re-derived in the page: the page used to parse
/// `location.hostname` itself, in two places, which made the browser a
/// third implementation of a grammar that now lives in one function.
target_interface: String,
```

filled from `TargetHost::interface()` on 4c's two parsed arms. **The third
arm needs stating**: an unparseable host falls back to treating the raw host
as a peer id, has no `TargetHost` at all, and so interpolates `""` — which is
the same value an `-i`-less host produces, and is resolved the same way by
D-S3-15 at the destination. That is a behaviour change from today, where the
page's own parser would have produced `''` for such a host anyway, so the two
agree. Surfaced in `peer-proxy.html` beside the existing constants:

```html
const TARGET_INTERFACE = "{{ target_interface }}";
```

Then **delete** the hostname parsing at
[peer-proxy.js:503-509](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L503)
and
[:858-865](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L858),
replacing each `interfaceName` with `TARGET_INTERFACE`. The two preamble
constructions on the following lines are otherwise unchanged. The per-request
parse was always redundant — the page's own origin does not change between
fetches — so this is a net deletion, and `sw.js` is untouched.

**One member per page load.** The coordinator selects once, and every fetch the
service worker later tunnels goes to that member for the life of the page —
sticky, which is right for `Singleton` and acceptable for `Redundant`. A
`Sharded` app is not addressable from a browser at all, because the page cannot
offer a per-request routing key before the target is chosen: the service worker
*does* forward request headers
([peer-proxy.js:925](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L925)),
so `X-Syneroym-Routing-Key` reaches the member, it just cannot change which
member. Backlog row, §5; not reachable today, since `Sharded` is compiled by
nothing.

**A failed resolve returns an error, never a fallback.** Today's unparseable-host
arm falls back to treating the raw host as a peer id
([bootstrap.rs:176](../../../../crates/coordinator_webrtc/src/bootstrap.rs#L176)).
A logical host that fails Tier 1, Tier 2, or either binding check must **not**
take that path — the fallback would dial whatever the hostname happened to
spell. `StatusCode::BAD_GATEWAY`, with the reason logged.

### Phase 5 — the operator surface, the tests, the docs

- **`roymctl alias`** ([commands.rs:62](../../../../apps/roymctl/src/commands.rs#L62))
  gains `--service <LOGICAL_SERVICE_NAME>` and `--domain <DOMAIN>` (default
  `localhost`). With `--service`, `service_id` is read as the **app DID**,
  `--nickname` becomes required (it must be the `AppInstanceId`, D-S3-2), and
  the output is
  `util::generate_app_host(nickname, &service_id, &service, &iface, &domain)?`;
  without it, `util::generate_service_host(...)`. `--interface` stays
  optional and now omits the `-i` segment rather than falling back to
  printing a bare alias -- the resulting host works, because the destination
  resolves it (D-S3-15), where the bare alias was never a host at all. A
  clap-level check refuses `--service` without `--nickname`, naming why.
- **Docs carrying the old format, all of which go stale on this slice.** The
  first revision named only the developer guide's gateway-hostname section;
  the full list:

  | File:line | What is stale |
  |---|---|
  | [AGENTS.md:110](../../../../AGENTS.md#L110) | The client-gateway architecture line documents `<nickname>-p<did-hash>-i<interface-hash>.localhost` as the contract. **The one an agent reads first**, so it is the one most expensive to leave wrong |
  | [developer-guide.md:239](../../../../docs/developer-guide.md#L239) | `roymctl registry lookup "alice-p<…>"` — broken by D-S3-14's alias reshape, not by the hostname change |
  | [developer-guide.md:271](../../../../docs/developer-guide.md#L271) | `Host: <NICKNAME>-p<…>-iorchestrator.localhost`. Also the evidence for §0.12's "`-i` stays permissive": this host passes a literal interface *name*, not a hash |
  | [developer-guide.md:1262](../../../../docs/developer-guide.md#L1262), [:1278](../../../../docs/developer-guide.md#L1278) | Two more `Host:` examples plus the comment line that states the format |

  The gateway-hostname section additionally gains the app-scoped form beside
  the unscoped one (two forms of one scheme, not one plus a legacy note --
  D-S3-10), the `X-Syneroym-Routing-Key` header, the omitted-`-i` rule
  (D-S3-15), and the three new config keys
  (`[iam].grant_resolve_to_node_did` and the two `resolve_ucan` paths) with
  the warning they suppress and which case each covers.
- **[ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md)
  §7 gets a dated amendment note**, in the shape §1's 2026-08-04 amendment
  already uses: the printed host form is superseded (marker, `-p`→`-s`,
  optional `-a`/`-i`), and its "four hashed components" arithmetic was wrong
  about the form it printed (§0.7). The decision itself is unchanged — the
  hostname still carries what decides reachability and the routing key is
  still a header — so this is a note, not a new ADR.
- **`test-components/miniapp-demo1-web` gains a second app-declared
  interface**, for test 104. Without it that test passes vacuously —
  D-S3-15's empty-interface rule would cover for a lost `-i`, which is
  precisely how §0.1's JS defect would have shipped unnoticed. The component
  cross-compiles to `wasm32-wasip2`, so it rebuilds (exit criterion 10), and
  it is excluded from the workspace build graph, so the rebuild is the
  `mise run test:e2e` path's, not `cargo test`'s.
- **`docs/planning/deferred-backlog.md`**: §5's rows per §5 below.

---

## §4 — S3 tests

**e2e cases are marked; everything else is a unit test.** Numbering is
per-milestone and continues from S2's 59.

**Phase 1 — the shared builder and parser:**

60. `an_unscoped_host_parses_into_an_alias_and_an_interface` — the `-s`/`-i`
    form, with a nickname containing dashes, with no nickname at all, and with
    an explicit port
61. `an_app_scoped_host_parses_into_an_alias_and_three_hashes`
62. `an_app_scoped_host_with_a_dashed_nickname_reassembles_the_nickname` — the
    property the right-to-left grammar exists for
63. **`a_nickname_ending_in_an_a_segment_is_not_read_as_an_app_host`** —
    §0.12's defect, the one this plan's own first draft shipped: `data-api`,
    `my-app`, and `chat-admin` as unscoped hosts must each parse back to their
    own nickname with `TargetHost::Service`, not to an `App` with a two-letter
    app hash. The width rule is what makes this pass, so **this test must be
    written against a fixture whose final nickname segment starts with `a`**;
    tests 62 and 66's generic "dashed nickname" cases would not catch it
64. `a_host_missing_the_marker_or_the_service_segment_is_refused` — D-S3-13
    and the required-arity half of D-S3-2, as one table: no `-roym1`, a wrong
    marker (`-roym2`), no `-s`, an `-s` of the wrong width, and a bare
    `www.example.com` all return `None` rather than a half-built alias.
    **Replaces the hardcoded `localhost`/`127` guard**, so `localhost` and
    `127.0.0.1` are two more rows of the same table rather than a special case
65. `an_interface_segment_may_carry_a_literal_name` — §0.12's asymmetry:
    `-iorchestrator` parses (it is what `docs/developer-guide.md:271`
    documents and what `EndpointRegistry`'s exact-match branch serves), while
    an `-a` or `-s` segment of that width does not
66. `a_host_with_no_interface_segment_parses_with_an_empty_interface` — the
    optional `-i`, on both shapes; the paired assertion is that this is **not**
    an error at the parse, because the destination is what resolves it
    (D-S3-15)
67. `a_domain_other_than_localhost_parses_identically` — §0.10's claim that the
    parser is domain-agnostic, so `*.syneroym.net` needs no code change
68. `the_reconstructed_app_alias_matches_what_the_registry_admitted` — builds
    an `EndpointInfo` the way `sign_tier1_record` does, derives the alias the
    way `register_endpoint` does (`generate_alias`), builds the host with
    `generate_app_host`, parses it, and asserts the two alias strings are
    equal. **This is D-S3-2's whole load-bearing claim**, and the one test that
    would catch either side drifting
69. `a_host_label_over_the_dns_limit_is_refused_not_truncated` — D-S3-9, on
    both builders, asserting the message names 63
70. `a_nickname_whose_last_segment_looks_like_an_app_hash_is_refused_at_build`
    — §0.12's irreducible residue: `a` plus exactly 8 characters is ambiguous
    to any parser, so the builder refuses to mint it
71. `a_built_host_round_trips_through_the_parser` — both forms, property-style
    over a handful of nickname/name/interface shapes, including a dashed
    nickname and an absent one
72. `an_alias_with_a_dashed_nickname_still_splits_at_the_hash` — D-S3-14's
    uniqueness claim now that the alias carries no letter: `short_hash` is
    always 8 characters and always last, so `rsplit_once('-')` recovers
    `(nickname, hash)` for `a-b-c` + hash and for `a` + hash alike

**Phase 2 — the destination's own reversals:**

73. `a_service_name_resolves_to_itself` — the exact-match path is unchanged
74. `a_short_hash_of_a_service_name_resolves_to_that_name`
75. `an_exact_name_wins_over_a_hash_that_matches_a_different_name` — construct
    a plan with a service literally named `short_hash("other")`
76. `two_service_names_sharing_a_short_hash_are_refused` — `AmbiguousHash`,
    with both names in the message. Searching for a real five-byte SHA-256
    collision at fixture-construction time is impractical, so this constructs
    the collision by stubbing the candidate set — asserted as "the branch
    exists and refuses", which is what it is
77. `an_unknown_hash_is_invalid_params_not_internal_error` — the mapping S2's
    post-merge finding 12 established for `NoSuchService`, extended
78. `resolve_answers_a_hashed_service_name_with_a_document_naming_the_real_name`
    — the property phase 3's D-S3-5 check depends on: the signed document never
    echoes the hash back
79. `an_empty_interface_resolves_to_the_only_app_declared_one` — D-S3-15, with
    the six `NATIVE_CAPABILITY_INTERFACES` also registered, which is what makes
    the naive "only one endpoint" rule wrong (§0.11)
80. `an_empty_interface_is_refused_when_two_are_declared_and_when_none_is` —
    the ambiguity and the empty-set halves, both `None`, never a guess
81. `the_terminating_hop_canonicalizes_before_any_capability_check` — the
    scoped form of D-S3-15's property: at the hop that resolves the service,
    the interface a downstream check sees is the registered name, exactly as
    on the hash path. **Its paired assertion is the relay case** — a hop that
    does not host the service forwards `preamble.interface` untouched, empty
    or hashed alike, because it does not know the names and must not guess.
    An earlier revision claimed "nothing past ingress sees an empty
    interface", which is false on the relay path and is what this pair pins
    correctly

**Phase 3 — the gateway:**

82. `an_unscoped_host_reaches_the_same_service_it_did_before` — the gateway's
    own regression pin, at unit scale over the parse + target selection, no
    network
83. `a_tier1_record_whose_hash_does_not_match_the_a_segment_is_refused` —
    D-S3-5's first half, and the reason it exists: a registry answering an
    alias with another app's valid record
84. `a_document_naming_a_different_service_than_the_s_segment_is_refused` —
    D-S3-5's second half
85. `a_second_request_for_the_same_app_scoped_host_makes_no_network_call` —
    task.md **budget 1** at the gateway, as a fetch count against a counting
    `TopologyFetcher`, not a timing
86. **`a_cold_resolve_makes_exactly_one_tier1_lookup`** — task.md **budget 2**,
    which nothing in this milestone measured before: a counting
    `RegistryClient` over one cold app-scoped resolve, asserting the alias
    lookup happens once and `fetch_via` adds none (D-S3-17). The paired warm
    assertion is zero
87. `an_expired_entry_triggers_one_refetch_rather_than_a_failure` — D-S3-8,
    ADR-0022 §3's "on expiry try to refresh", which nothing implemented before
    this slice
88. `a_routing_key_header_selects_a_member_and_its_absence_does_not` — over a
    `Redundant` document; the same key twice returns the same member, no header
    returns members in round-robin
89. `a_sharded_service_with_no_routing_key_fails_with_the_resolvers_own_error`
    — ADR-0022 §7's closing sentence, asserted on the error text
90. `a_gateway_with_neither_credential_warns_at_init_naming_both_config_keys` —
    D-S3-6, in the shape S1's no-registry warning test uses; the paired case
    asserts no warning when only the same-node gate is on
91. `the_node_did_is_granted_supervisor_resolve_only_when_the_gate_is_on` — in
    `route_handler/io.rs`'s own test module, beside the existing
    `admin_ucan_root` cases
92. `the_same_node_grant_does_not_confer_substrate_admin` — D-S3-6(a)'s whole
    safety claim, as a negative on the same `CallerContext`: the granted
    capability answers `supervisor/resolve` on any `synapp:` resource and
    `false` for `substrate/admin`, `orchestrator/deploy`, and
    `data-layer/admin`

**Phase 4 — the coordinator:**

93. `an_app_scoped_bootstrap_request_renders_the_resolved_member_did` — an
    axum-level test over `app(state)`, asserting `TARGET_SERVICE_ID` in the
    rendered HTML is a member DID from the document and `TARGET_PEER_ID` is the
    substrate hosting it
94. **`the_rendered_page_carries_the_hosts_interface`** — D-S3-16: a host with
    an explicit `-i` renders that hash into `TARGET_INTERFACE`, and one
    without renders the empty string. Without this, the JS deletion is
    unverified and the regression §0.1 describes (every host silently losing
    its interface) has no failing test anywhere
95. `an_unscoped_bootstrap_request_is_unchanged` — the no-regression half, on
    the same handler
96. `an_app_scoped_host_that_fails_to_resolve_does_not_fall_back_to_the_raw_host`
    — the phase-4c refusal: 502, and the raw hostname never reaches
    `resolve_did_key`

**Phase 5 — operator surface and end to end:**

97. `roymctl_alias_with_a_service_prints_the_app_scoped_form`, and without
    `--interface` prints the same host with no `-i` segment
98. `roymctl_alias_with_a_service_and_no_nickname_is_refused` — the
    `AppInstanceId` requirement D-S3-2 rests on
99. **(e2e)** `an_http_client_reaches_an_apps_logical_service_by_hostname_alone`
    — two real substrates and a real registry, in `topology_document_e2e.rs`'s
    shape: submit + adopt an app with `replicas > 1`, build the host with
    `generate_app_host`, POST through the gateway, assert the app answered. The
    milestone's cross-app half, from an ordinary HTTP client
100. **(e2e)** `a_keyed_request_reaches_a_stable_member_and_an_unkeyed_one_spreads`
     — the routing-key header over the wire, against a real `Redundant` service
101. **(e2e)** `an_app_scoped_hostname_for_an_app_this_gateway_holds_no_grant_for_is_refused`
     — matrix row 7 at the hostname layer: a clean 502 with the denial logged,
     and no member DID anywhere in the response. The gateway node has the
     same-node gate **off** and no `resolve_ucan`, against an app supervised
     elsewhere — the exact shape D-S3-6 says needs a token
102. **(e2e)** `a_same_node_gateway_resolves_with_no_credential_file` —
     D-S3-6(a) end to end: one substrate running both the supervisor and the
     gateway, `[iam].grant_resolve_to_node_did = true`, no `resolve_ucan`
     anywhere. The single-node developer path, and the case most likely to
     regress silently, since every other e2e in the milestone hands out an
     explicit grant
103. **(Playwright)** `an_app_scoped_hostname_loads_an_app_through_the_bootstrap_page`
     — extends `webrtc.spec.ts`: `global-setup.ts` builds an app-scoped host
     with `roymctl alias --service` instead of the unscoped one, and the page
     must render the app's own content. The first `mise run test:e2e` case in
     this milestone that is not a no-op
104. **(Playwright)** `an_explicit_interface_still_reaches_that_interface` —
     the regression §0.1 names: `miniapp-demo1-web` gains a second
     app-declared interface in the fixture, so D-S3-15's empty-interface rule
     can no longer cover for a lost `-i`, and a host naming one of them must
     reach that one. **Without the second interface this test passes
     vacuously**, which is precisely how the JS defect would have shipped
     unnoticed. `multi-hop.spec.ts` gets the same treatment on its own suite

**Matrix coverage after S3.** No row of task.md's failure/security matrix is
newly S3's — S1 closed 1, 2, 3, 11 and S2 closed 4-10. Three rows gain a second
named test at a new layer, which is the point of this slice rather than an
extension of coverage: row 6 (expiry) → 87, row 7 (clean denial) → 101, row 10
(the epoch carried and preserved) → 78, which pins that a hashed request still
produces a document carrying the real name and the real epoch.

**Performance-budget coverage.**

| Budget | Covered by |
|---|---|
| 1 — resolution after the first fetch makes no network call | Test 85's fetch count at the gateway. S2 proved it for a program holding a `LogicalResolver` directly; the gateway is the first caller reaching it through a hostname, where a per-request Tier-1 lookup is the easy mistake |
| 2 — the Tier-1 lookup stays within the existing registry-lookup budget | **Test 86**, new. §0.13 found the first revision of this plan spending two Tier-1 lookups per cold resolve and never measuring it; D-S3-17 removes the second and this counts them |
| 3 — verification once per fetch, not once per resolve | Unchanged: `register_verified` is still the only caller of `verify` |
| 4 — Tier-1 refresh cost on the supervisor | **S1's**, untouched by this slice |


## §5 — Backlog rows this slice creates, and the one it closes

**Closed** (move the row to *Recently resolved*; there is no code marker
for it):

- **"No substrate holds a `TopologyFetcher`, so ADR-0022 §3's substrate-side
  fetch does not exist yet"** — the client gateway holds one (D-S3-7), and its
  request path is the trigger the row said did not exist. The row's sharper
  half, "`on expiry try to refresh` has no implementation anywhere", is closed
  too, by D-S3-8. **Only for the gateway path**: a WASM guest reaching a
  foreign dependency still cannot, because `prepare_binding`'s intra-app
  refusal is S4's. Reword and re-target that remainder at S4 rather than
  deleting the row outright.

**Created:**

- **The topology epoch is not carried on a request** (§0.8, D-S3-8's sibling).
  ADR-0022 §6 says a request carries the epoch it was resolved under; S3
  resolves under one and forwards nothing. Target: **S5 (M7 `[PLT-RED]`)`**,
  which is also where the member-side check that gives it meaning is built.
  Source: this plan §0.8; `crates/client_gateway/src/gateway.rs`.
- **`AppInstanceId` has no length cap, and a logical hostname needs one**
  (§0.7, D-S3-9). The DNS label budget leaves 27 characters for the nickname
  once the `-roym1` marker is counted (37 with no `-i`), and
  `generate_app_host` refuses past it — at host-build time, long after
  `submit` accepted the instance. Target: **TBD**; the fix is a validator on
  `AppInstanceId`, which is a refusal at `submit` rather than at a browser.
  Source: `crates/app_orchestration/src/models.rs`;
  `crates/core/src/util.rs`.
- **A gateway or coordinator can only resolve a *remote* app whose operator
  issued it a grant** (§0.3, D-S3-6). The same-node gate covers apps supervised
  here; `resolve_ucan` is a static, operator-installed token for everything
  else, so a browser reaching an app on an unaffiliated node still gets a
  denial. Closes when S4 adds ADR-0022 §5's "open to all" manifest declaration.
  Target: **S4**. Pairs with the existing *"`resolve`'s visibility is a
  capability check with no manifest declaration"* row, which it should link to
  rather than duplicate.
- **An `-i`-less host's meaning depends on the app, not on the host** (§0.11,
  D-S3-15). "The one app-declared interface" is resolved at the destination,
  so deploying a *second* interface for that service turns every
  previously-working `-i`-less host into a refusal, with no version bump and
  no warning. It fails loudly rather than routing to the wrong interface,
  which is what makes it acceptable, but it sits at an angle to ADR-0022 §7's
  rule that the hostname carries what decides reachability. The fix is a
  manifest-declared *default* interface, which is stable across adding a
  second one -- a surface S4 is already opening for per-service visibility.
  Target: **S4**. Source: this plan §0.11, D-S3-15;
  `crates/core/src/local_registry.rs` (`resolve_interface`).
- **A keyed `Sharded` service is not addressable from a browser** (phase 4c).
  `Singleton` and `Redundant` both work: the coordinator selects one member at
  page load, which is `select_member`'s own answer for `Singleton` (member 0)
  and for `Redundant` with no key (round-robin), and every later fetch through
  the service worker is sticky to it -- correct for both, since any member of
  a `Redundant` service is a correct member. `Sharded` is different in kind:
  `select_member` **requires** a routing key and errors without one
  (`crates/app_orchestration/src/resolver.rs`), because picking a member
  without the key is a confident wrong answer rather than a slow one --
  ADR-0022 §5's reasoning about partial member sets, applied to selection.
  Making it work needs per-request selection *in the page*: the member list
  relayed to the browser, plus `rendezvous_select`'s BLAKE3 hashing in JS. The
  service worker already forwards `X-Syneroym-Routing-Key` to the member
  (`peer-proxy.js`), so only the selection half is missing. Not reachable
  today, since `Sharded` is compiled by nothing -- this joins that row's
  dependents. Target: **S5**.
- **ADR-0022 §3's "any party may relay" property is not exercised on the
  browser path** (§0.6). The coordinator resolves and the page trusts the
  result, which is unchanged from today's posture and is a deliberate scope
  decision (the shell page load is not authenticated), but it means the relay
  argument is discharged only by the SDK/gateway path, where
  `register_verified` runs. Target: **TBD**, and it needs a reason to exist
  before it needs a design. Source: this plan §0.6, D-S3-11;
  `crates/coordinator_webrtc/src/bootstrap.rs`.

**Updated, not closed:** *"No early cache invalidation for a Tier-2 topology
document"* — D-S3-12 declines to subscribe, and the row's "S3, if a subscriber
appears" target moves to "a consumer needing sub-`cache_ttl` convergence".

---

## §6 — What closing S3 closes

| Against | Closed by S3 | Note |
|---|---|---|
| task.md slice S3 | Fully | Hostname, routing-key header, coordinator resolution of a logical host |
| `[TOP-ADR]` Service Addressing | The external form changes | Existing row, `Complete` at the M1 logical-ref level; this is the S3 change task.md flags |
| `[PLT-DAP-01]` cross-app half | The last third | S1 published Tier 1, S2 made it fetchable by a program, S3 makes it reachable by an ordinary HTTP client and a browser. Recorded `Complete` with S1-S3 evidence, per exit criterion 4 |
| ADR-0022 §7 | Fully, except its epoch sentence | §6's on-request epoch is §0.8's declared omission |
| D-S2-13's backlog row | The gateway half | The guest half stays open for S4 |

**Explicitly not closed:** the epoch on the request (S5); per-service
visibility declaration (S4); cross-app `Bind` (S4); shard rebalancing (S5);
the browser's inability to express a routing key (S5); and ADR-0022 §3's
"any party may relay" property on the browser path, which S3 deliberately
does not exercise (§0.6).

---

## §7 — The milestone's exit criteria, against this slice

| # | Criterion | S3's part |
|---|---|---|
| 1 | Reference scenario end to end | Tests 99-104 extend it to the HTTP-client and browser entry points; steps 1-8 themselves are S1/S2's and stay green |
| 2 | Every matrix row has a named test | No new rows; three gain a second named test at the hostname layer (§4) |
| 3 | Every budget has a measurement | Budget 1 re-measured at the gateway (test 85) as a fetch count, and **budget 2 measured for the first time in this milestone** (test 86, registry-call count) — §0.13 found the first draft of this plan quietly doubling it |
| 4 | `[PLT-DAP-01]` cross-app half recorded Complete with S1-S3 evidence | **This slice completes the evidence.** Update `traceability-matrix.md` at closeout |
| 5 | The hostname change goes through `core::util` / `core::protocol_utils` only | **Phase 1 and phase 4d are what make this true** (§0.1) — today it is not, and there are four parsers, not the two the first draft counted. The diff must show `client_gateway`'s duplicate parser deleted, **both `peer-proxy.js` parsers deleted** in favour of `TARGET_INTERFACE`, and every hand-formatted host replaced |
| 6-9 | fmt / clippy / `cargo test --workspace` / `mise run test:e2e` | All four. **`mise run test:e2e` genuinely matters for the first time in this milestone** -- S1 and S2 both recorded it as "unaffected, no client-gateway or WebRTC surface touched", and phase 4 changes the coordinator's own bootstrap handler *and* `peer-proxy.js`, with phase 5 adding two Playwright cases (tests 103, 104) |
| 10 | `wasm32-wasip2` components rebuild against changed WIT | **Not a no-op**, though not for the reason the criterion names: `supervisor.wit` is unchanged (phase 2 is a doc-comment change only), but **test 104 adds a second app-declared interface to `test-components/miniapp-demo1-web`**, which cross-compiles to `wasm32-wasip2` and must rebuild. An earlier revision of this row read "no-op" on the WIT argument alone |

---

## §8 — Questions settled, and what a review pass changed

**Answered 2026-08-09, and folded into the decisions above rather than left
here as open questions.** Items 4-6 came from a second pass on the same day.

1. ~~**Is `resolve_ucan` the right shape, or should the gateway be granted
   implicitly on the same node?**~~ **Both** (D-S3-6). A substrate-startup
   config gate (`[iam].grant_resolve_to_node_did`) grants the node's own DID
   `supervisor/resolve` -- deliberately narrower than `substrate/admin`, so
   the node key gains resolution and nothing else -- which covers every app
   supervised on this node with no credential file. `resolve_ucan` stays for
   apps supervised elsewhere, where no local config can help because the
   check runs on the remote supervisor.
2. ~~**Should the bootstrap page verify the relayed document?**~~ **No**
   (§0.6, D-S3-11). The bootstrap page is a shell that registers a service
   worker; it needs the right DIDs wired into it and nothing more, and
   authenticating the shell page load is explicitly not a goal. Phase 4 lost
   its WebCrypto/canonicalizer/z-base-32 half entirely. `templates/` is
   **not** untouched, though — item 10 found the page deriving the interface
   from the hostname itself, so phase 4d interpolates `TARGET_INTERFACE` and
   deletes both JS parsers. That is a deletion, not verification logic. The
   consequence of not verifying -- ADR-0022 §3's relay argument is discharged
   only on the SDK/gateway path -- is recorded as a backlog row, not hidden.
3. ~~**Does `-p` addressing stay?**~~ **The capability stays; the spelling
   does not** (D-S3-10, D-S3-2). `-p` was
   `<nickname>-p<short_hash(service_id)>-i<short_hash(interface)>.localhost`,
   today's only form, where `p` stood for "pubkey hash" -- the concrete
   service DID. Addressing a service that belongs to no app instance is
   permanent (it is every `roymctl svc deploy` service, and what the whole
   Playwright suite uses), but per your point 3 that segment is now spelled
   `-s`, with `-a` absent. One grammar, two shapes.

4. ~~**Should `[iam].grant_resolve_to_node_did` default to `true`?**~~
   **No, `false`** (answered 2026-08-09). A grant is asked for, not assumed,
   and symmetry with `admin_ucan_root` is worth keeping.
5. ~~**Can the browser path support `Redundant`?**~~ **It already does**
   (answered 2026-08-09). `Singleton` and `Redundant` both work from a
   browser; only a keyed `Sharded` service does not, and that is correct
   rather than a gap -- see §5's backlog row for the reasoning and for what
   closing it would take.
6. ~~**Clean up the format ambiguity while we are here.**~~ **Done**
   (directed 2026-08-09): §0.10, D-S3-2, D-S3-13, D-S3-14. One grammar with
   an optional `-a`, `-p` renamed to `-s`, a required trailing `-roym1` marker,
   and the
   registry alias's letter dropped so both hash segments reconstruct it
   identically. The DNS half of that request is answered in §0.10 -- partial
   wildcards are not a DNS feature, and none is needed, because the whole
   scheme lives in one label.

7. ~~**`-i` should be able to mean the default interface.**~~ **Yes, and it
   did not work before** (answered 2026-08-09): §0.11, D-S3-15. `-i` is
   optional again, and an omitted one now resolves at the destination to the
   service's one app-declared interface. Note that today's "optional `-i`" was
   never a default -- it produced an empty interface that matched nothing --
   so this fixes a convenience rather than adding one. The naive rule does
   not work: every deployed service carries six auto-registered
   `NATIVE_CAPABILITY_INTERFACES`, so "only one endpoint" is never true and
   the filter is what makes the rule correct.
8. ~~**`roym1` instead of something dry.**~~ **Adopted** (2026-08-09).
   `roym` is `roymctl` spelled in full and the distinctive half of the
   product name; the digit is the format version, so `-roym2` can be served
   alongside it later (D-S3-13 explains what that means). Costs one character
   more than the clipped `roy1`, taking the nickname budget to 27 -- accepted
   deliberately, since the full word is what an operator already recognises.
   Two alternatives considered and not taken: `nym1` reads better still (a
   hostname *is* a name, and `-onym` is where the product name comes from)
   but collides with Nym Technologies, a real privacy-network brand in an
   adjacent space, which every hostname would then quietly suggest an
   association with; `sy1` saves characters and reads like nothing.

**Answered by a review pass on this plan, 2026-08-09** (three blocking, five
smaller; all incorporated rather than pushed back on, except one that was half
right):

9. **The parser invented an app from a nickname ending in `a…`** — §0.12,
    a real defect in this plan's first draft, fixed by a width rule on `-a`
    and `-s`, a build-time refusal for the irreducible case, and test 63,
    which is specified against an `a…` fixture because the generic
    dashed-nickname tests would not have caught it.
10. **The hostname is parsed in four places, not two** — §0.1. The two JS
    copies in `peer-proxy.js` would both have silently lost every explicit
    `-i` under the trailing marker, with the Playwright suite passing anyway.
    D-S3-16 deletes them in favour of an interpolated `TARGET_INTERFACE`,
    which also corrects D-S3-11's "templates need no change" claim.
11. **Two Tier-1 lookups per cold resolve, and budget 2 unmeasured** —
    §0.13, D-S3-17, test 86.
12. **Missed call sites and docs** — `global-setup-multihop.ts` (two sites,
    feeding `multi-hop.spec.ts`'s bootstrap URL), four hand-built aliases in
    `community_registry`'s tests, `AGENTS.md:110`, and four
    `developer-guide.md` sites. All now in phase 1c and phase 5.
13. **ADR-0022 §7 needs its own amendment note** — added to phase 5,
    alongside task.md's (item 17 below).
14. **`rec.verify()` was redundant** — removed; `RegistryClient::lookup`
    already verifies on both branches, and keeping it implied otherwise.
15. **"Nothing past ingress sees an empty interface" was half right.** The
    property is false as stated — a *relay* hop forwards `preamble.interface`
    untouched, which is correct and deliberate (it does not host the service
    and cannot know its interface names, exactly as it forwards a hash
    unresolved today). The design is unchanged; the claim was overstated, and
    test 81 now pins the true property plus the relay case.
16. **Test numbering was out of order** (`63`, `63c`, `63b`, `66b`,
    `72a-c`) — renumbered sequentially, 60 through 104.

**Still open:**

17. **task.md's migration note now understates this slice.** It says "An
   external format change, S3. The client gateway hostname **gains** app and
   service segments", which was true of the first draft and is not true of
   this one: `-p` is renamed and a marker is added, so **every** gateway host
   string in existence changes, not only new ones. Nothing persists one
   (`RegistryState.aliases` is an in-memory `DashMap`, and the Playwright
   suite builds its host from `roymctl` at setup), and the pre-release policy
   forbids a compatibility shim, so this is a wording fix in task.md at
   closeout rather than a design question -- flagged so it is not missed.
