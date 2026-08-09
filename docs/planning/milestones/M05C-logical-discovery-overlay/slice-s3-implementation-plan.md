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

Nine findings. Three of them (0.1, 0.2, 0.3) each block the slice on their own,
and none of the three is stated anywhere in the input documents.

### 0.1 (Correctness, and it makes an exit criterion unsatisfiable as written) The hostname is parsed in two places and built in none

[task.md](task.md)'s migration-impact section and
[meta-implementation-plan.md:527](../../meta-implementation-plan.md) both say
the format is "centralised in `core::util` (build) and `core::protocol_utils`
(parse), so a client going through those helpers sees one change in one place",
and exit criterion 5 makes that testable: "S3's hostname change goes through
`core::util` / `core::protocol_utils` only, demonstrated by the diff."

Against the tree, both halves of that claim are false.

**Parse is duplicated.**
[`core::protocol_utils::parse_target_host`](../../../../crates/core/src/protocol_utils.rs#L68)
and
[`client_gateway::parse_target_service_and_interface`](../../../../crates/client_gateway/src/gateway.rs#L254)
are the same right-to-left algorithm written twice, line for line, including
the same `-p`/`-i` prefix checks and the same `parts.join("-")` nickname
reassembly. The shared one is used by exactly one caller — the WebRTC bootstrap
page
([bootstrap.rs:174](../../../../crates/coordinator_webrtc/src/bootstrap.rs#L174))
— and the client gateway, the component the format exists *for*, does not use
it. Changing only `core::protocol_utils` would change the browser path and
leave the gateway path on the old grammar.

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

The fix is small and is the one `roymctl` already uses: an operator-supplied
`CapabilityToken` on disk, loaded at role init, presented on the fetch. It is
config, not a new authorization concept. Decided in D-S3-6.

**What it does not fix, and must be said:** a gateway can only resolve apps
whose supervisor's operator issued it a grant. Cross-organisation resolution
stays out of reach until S4 adds ADR-0022 §5's "open to all" manifest
declaration (already a backlog row, targeted S4). S3 makes the *mechanism*
work; the *reach* is S4's.

### 0.4 (Correctness, and it is the same defect the S2 post-merge review already found once) A `-a` host resolves through the one registry-lookup branch that is not bound to what was asked

`RegistryClient::lookup` checks that the record it got back is the record that
was asked for **only when the lookup key is a full DID**
([dht_registry.rs:372](../../../../crates/core/src/dht_registry.rs#L372)) — the
comment there says a shorthash alias lookup "cannot be checked this way by
construction". That check exists because S2's post-merge review found its
absence (finding 4, [status.md](status.md)).

A `-a<app-did-hash>` hostname carries a hash, not a DID, so the only lookup it
can do is by alias — landing in exactly the unchecked branch. A registry that
answers the alias `chat-p<hash>` with some other app's perfectly valid,
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

### 0.6 (Ambiguous, and the answer decides whether the relay half is worth building) Does the browser verify the relayed document?

ADR-0022 §3 lists "the WebRTC coordinator serving it inside a bootstrap page"
as one of the relays that justify the document form over a plain RPC answer,
and gives the reason: "Under a bare RPC, a relayed copy would be unverifiable
and every relay would become trusted infrastructure."

Today the page trusts the coordinator completely: `TARGET_SERVICE_ID` is
interpolated into the HTML by
[`handle_bootstrap`](../../../../crates/coordinator_webrtc/src/bootstrap.rs#L204)
and used verbatim to build the route preamble
([peer-proxy.js:510](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L510),
[:866](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L866)).
If S3 relays the document but the page does not check it, the coordinator is
still trusted infrastructure, and the relay carries bytes nobody reads.

The work to make it real is bounded and testable: Ed25519 `crypto.subtle`
verify, a ~40-line canonicalizer mirroring
[`canonicalize_json_value`](../../../../crates/identity/src/substrate.rs#L180),
and SHA-256 + z-base-32 to check the app DID against the `-a` segment the URL
already carries. All four steps have a Playwright suite to prove them in.

Recommended: **verify in the page.** It is what makes the hostname the security
boundary ADR §7 says it is, and it is the whole argument for §3's document
form. Structured as its own phase so it can be dropped as a unit if the
requester disagrees — see §8 question 2.

### 0.7 (Stale, minor, but it drives a real limit) ADR-0022 §7's own length arithmetic does not match the form it prints

§7: "Four hashed components plus a nickname is close to the 63-character DNS
label limit, so truncation lengths are chosen deliberately rather than
inherited." The form printed three lines above has **three** hashed components
(`a`, `s`, `i`), not four, and no truncation length is stated anywhere.

The real budget: `short_hash` is `z32::encode(&sha256[..5])` = exactly 8
characters, so `<nickname>-a<8>-s<8>-i<8>` is `nickname + 30`, leaving **33
characters for the nickname**. The nickname on a logical host is the app's
`AppInstanceId` (see D-S3-2), and `AppInstanceId` has no length validation at
all ([models.rs:98](../../../../crates/app_orchestration/src/models.rs#L98)).

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

### 0.9 (Ambiguous, and reading it the obvious way breaks the tree) `-p` is not replaced

ADR-0022 §7 says "The gateway host **becomes**:" and prints only the
`-a…-s…-i…` form. Read as a replacement, that deletes physical addressing —
which is what
[basic_lifecycle.rs:428/607](../../../../crates/substrate/tests/basic_lifecycle.rs#L428),
[tcp_proxy_latency.rs:100](../../../../tests/perf/src/scenarios/tcp_proxy_latency.rs#L100),
`roymctl alias`, and the entire Playwright suite (via `global-setup.ts`) use,
and which is the only way to address a service that belongs to no app instance.

The two forms are disjoint by their prefix letters and coexist. task.md's own
migration note agrees implicitly ("**Nothing is dropped or renamed**"). Stated
as a decision because the ADR's wording invites the other reading.

---

## §1 — S3 decisions

| ID | Decision |
|---|---|
| **D-S3-1** | **Centralise before extending.** Phase 1 moves *both* the build and the parse of every gateway host into `syneroym-core` — a new `TargetHost` enum and one `parse_target_host` in `core::protocol_utils`, and `generate_service_host`/`generate_logical_host` in `core::util` — and **deletes** `client_gateway::parse_target_service_and_interface` (§0.1). No new segment is added until every producer and consumer in the tree goes through those four functions. This is what makes exit criterion 5 ("demonstrated by the diff") true rather than aspirational. |
| **D-S3-2** | **The logical host is `<nickname>-a<short_hash(app_did)>-s<short_hash(service_name)>-i<short_hash(interface)>.localhost`, and `<nickname>` is the app's `AppInstanceId`.** Not free-form: `<nickname>-p<a_hash>` must equal the registry alias the Tier-1 record was admitted under, which `register_endpoint` derives as `generate_alias(info.nickname, service_id)` ([registry.rs:208](../../../../crates/community_registry/src/registry.rs#L208)) from the record's own `nickname`, which `sign_tier1_record` sets to the app instance id ([tier1.rs:184](../../../../crates/app_supervisor/src/tier1.rs#L184)). This is what lets a `-a` hostname be resolved with **no new registry surface at all** — the app DID is recovered by an ordinary alias lookup. A wrong nickname simply fails to resolve. |
| **D-S3-3** | **`resolve` accepts a logical service name *or* its `short_hash`** (§0.2), reversed against the instance's own plan, the same way an interface hash is reversed at its destination. An exact name match always wins over a hash match; a hash matching two names in one plan is **refused** (`InvalidParams`), never resolved arbitrarily. `TopologyFetcher::fetch`'s signature is unchanged — a `LogicalServiceName` carrying a hash is still a valid `LogicalServiceName` — so no client-side type changes and no WIT signature change. |
| **D-S3-4** | **The routing key is the request header `X-Syneroym-Routing-Key`**, absent meaning unkeyed, its raw UTF-8 bytes passed to `LogicalResolver::resolve`'s `routing_key`. The name is defined once as `core::protocol_utils::ROUTING_KEY_HEADER` (there is no existing `X-Syneroym-*` header in the tree to be consistent with). The gateway **does not strip or rewrite it**: the forwarded bytes stay the bytes that arrived (§0.8), so a member can observe the key it was routed on. |
| **D-S3-5** | **Every hop is bound to what the hostname asked for** (§0.4). After the Tier-1 lookup: `short_hash(record.info.service_id) == a_hash`, or refuse. After the Tier-2 fetch: `short_hash(document.service_name) == s_hash`, or refuse — `SignedTopologyDocument::verify` checks the signer and the expiry, never *which service*. Both checks are the caller's, in the same shape as S2 post-merge finding 4's fix, and both are refusals, not warnings. |
| **D-S3-6** | **Each S3 caller presents an operator-supplied `CapabilityToken` loaded from a config path** (§0.3): `[roles.client_gateway] resolve_ucan` and `[roles.coordinator] resolve_ucan`, both `Option<PathBuf>`, both absent by default. Absent means logical hostnames resolve only where a grant is not needed, which today is nowhere, so absence is a **startup warning naming the config key**, in the shape S1's no-registry warning already uses ([D-S1-8](slice-s1-implementation-plan.md)). Deliberately not a new authorization concept: it is the same `--ucan` file `roymctl app resolve` takes, read at role init. |
| **D-S3-7** | **The client gateway owns its own `LogicalResolver` over its own `StaticInventory`** (§0.5), and its own `RegistryTopologyFetcher`. Not the node's — the construction order forbids it and the key spaces are disjoint. This is the substrate-side `TopologyFetcher` holder D-S2-13 deferred, and closing that backlog row is S3's, not S4's. |
| **D-S3-8** | **The gateway re-fetches on any resolver miss, including expiry** — that is ADR-0022 §3's "on expiry try to refresh; if the refresh fails, keep using the previous document until `not_after`", which the S2 post-merge review recorded as implemented nowhere. It falls out of the gateway's own request path rather than needing a scheduler: a miss or an expiry error from `LogicalResolver::resolve` triggers `fetch_and_register` and one retry, so the refresh happens on demand at the exact moment an answer is needed. Nothing new is scheduled, and the "no network call after the first fetch" budget still holds inside `cache_ttl`. |
| **D-S3-9** | **The host builder refuses a label over 63 characters rather than truncating** (§0.7). `generate_logical_host` returns `Result<String>`; the error names the 63-character DNS label limit and the 33-character nickname budget. The **parser never enforces a length** — a host that arrived is a host that arrived. A backlog row proposes the cap belongs on `AppInstanceId`, where `submit` would refuse it years before a browser does. |
| **D-S3-10** | **Both host forms coexist** (§0.9), disjoint by prefix letter. `-p` addressing is untouched: same grammar, same registry lookup, same tests. |
| **D-S3-11** | **The coordinator relays the signed document into the bootstrap page, and the page verifies it** (§0.6): signature against the app DID, `not_after`, and `short_hash(app_did) == a_hash` taken from `window.location.hostname` rather than from anything the coordinator said. Member selection then happens **in the page**, from the verified member list, so the coordinator stops being trusted for the answer. Phase 4 is self-contained so this decision can be reversed to relay-only without touching phases 1-3 — see §8 question 2. |
| **D-S3-12** | **No MQTT epoch-bump subscription in S3.** D-S2-10 targeted its backlog row at "S3, if a subscriber appears". The gateway is a subscriber-shaped thing, but subscribing would add an MQTT client to a component that has none, to shorten a 5-minute staleness window that D-S3-8's on-demand refresh already bounds by `cache_ttl`. The row stays open with its target re-pointed at "a consumer that needs sub-`cache_ttl` convergence", which nothing in this milestone is. |

---

## §2 — Phase plan

Five phases, strictly ordered. Phases 1-3 are the slice's spine; phase 4 is
separable (D-S3-11); phase 5 is the operator surface and the proof.

| Phase | What | Why this order |
|---|---|---|
| **1** | One builder and one parser in `syneroym-core`, both host forms, every call site moved, the gateway's duplicate parser deleted | §0.1: extending two grammars is writing the new one twice |
| **2** | `handle_resolve` accepts a hashed service name | §0.2: nothing downstream can fetch until this exists |
| **3** | The client gateway's logical path: config, credential, resolver, fetcher, routing-key header | The slice's actual deliverable |
| **4** | Coordinator relay + in-page verify + in-page member selection | Separable by D-S3-11; depends on 1 and 2 only |
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

/// What a gateway host names. The two forms are disjoint by the prefix
/// letter of their leading hashed segment, and both remain valid
/// (ADR-0022 §7 prints only the logical one, but physical addressing is
/// how every service outside an app instance is reached).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetHost {
    /// `<nickname>-p<service-id-hash>-i<interface-hash>` -- one concrete
    /// service, resolved by an ordinary registry alias lookup.
    Physical {
        /// `<nickname>-p<hash>`, ready to pass to `RegistryClient::lookup`.
        lookup_alias: String,
        /// The `short_hash` of the interface name, or `""` when the `-i`
        /// segment was absent.
        interface: String,
    },
    /// `<nickname>-a<app-did-hash>-s<service-name-hash>-i<interface-hash>`
    /// -- a logical service of an app instance (ADR-0022 §7). Resolved
    /// through Tier 1 (this alias) and Tier 2 (the supervisor it names).
    Logical {
        /// `<nickname>-p<app_did_hash>`: the alias the app's Tier-1 record
        /// was admitted under, reconstructed so no new registry surface is
        /// needed. `community_registry::register_endpoint` derives it as
        /// `generate_alias(info.nickname, service_id)`, and the Tier-1
        /// record's `nickname` is the app instance id.
        app_lookup_alias: String,
        /// Kept beside the alias so a caller can bind the record it gets
        /// back to the host it was asked for -- `RegistryClient::lookup`
        /// cannot check an alias lookup itself, by construction.
        app_did_hash: String,
        service_name_hash: String,
        interface: String,
    },
}

impl TargetHost {
    #[must_use]
    pub fn interface(&self) -> &str {
        match self {
            Self::Physical { interface, .. } | Self::Logical { interface, .. } => interface,
        }
    }
}
```

Replace `parse_target_host`'s body. Signature changes from
`Option<(String, String)>` to `Option<TargetHost>`. Pseudo-code (the port
strip, the `.localhost` strip, the `localhost`/`127` guard, and the
right-to-left pop are all lifted verbatim from today's body):

```
fn parse_target_host(host) -> Option<TargetHost>:
    host_str = strip trailing ":<digits>" if present
    host_base = host_str.strip_suffix(".localhost").unwrap_or(host_str)
    subdomain = host_base.split('.').next()
    if subdomain in {"localhost", "127"}: return None

    parts = subdomain.split('-').collect::<Vec<_>>()

    interface = pop_prefixed(&mut parts, 'i').unwrap_or("")

    // Logical first: its `-s` segment is what tells the two forms apart,
    // and a `-p` host can never carry one.
    if let Some(s_hash) = pop_prefixed(&mut parts, 's'):
        a_hash = pop_prefixed(&mut parts, 'a')?      // `-s` without `-a` is malformed
        nickname = parts.join("-")                    // may be empty
        return Some(Logical {
            app_lookup_alias: if nickname.is_empty() { format!("p{a_hash}") }
                              else { format!("{nickname}-p{a_hash}") },
            app_did_hash: a_hash, service_name_hash: s_hash, interface,
        })

    pubkeyhash = pop_prefixed(&mut parts, 'p').unwrap_or("")
    nickname   = parts.join("-")
    lookup_alias = if nickname.is_empty() { format!("p{pubkeyhash}") }
                   else { format!("{nickname}-p{pubkeyhash}") }
    return Some(Physical { lookup_alias, interface })

// `pop_prefixed(parts, c)`: if the last element starts with `c` and is
// longer than one character, pop it and return the remainder; else None.
// This is today's inlined `if let Some(last) = parts.last() && ...` block,
// named once instead of written four times.
```

Two behaviours are preserved exactly so no `-p` caller changes: an absent `-i`
yields `interface == ""`, and an absent `-p` yields the `"{nickname}-p"` alias
today's code produces.

#### 1b. `crates/core/src/util.rs` — the builders

`generate_alias` and `short_hash` are unchanged (`generate_alias` is still the
registry's own admission-side alias derivation and must not move). Add:

```rust
/// The longest a single DNS label may be. A hostname segment over this is
/// rejected by resolvers and by browsers before anything of ours sees it.
pub const MAX_DNS_LABEL_LEN: usize = 63;

/// `<nickname>-p<service-id-hash>[-i<interface-hash>].localhost` -- the
/// physical gateway host, for a service addressed by its own id.
///
/// # Errors
/// The label exceeds `MAX_DNS_LABEL_LEN`.
pub fn generate_service_host(
    nickname: Option<&str>,
    service_id: &str,
    interface: Option<&str>,
) -> anyhow::Result<String>;

/// `<nickname>-a<app-did-hash>-s<service-name-hash>-i<interface-hash>.localhost`
/// -- the logical gateway host (ADR-0022 §7).
///
/// `nickname` must be the app instance's `AppInstanceId`: `<nickname>-p<app
/// did hash>` is the registry alias the app's Tier-1 record was admitted
/// under, and reconstructing it is what lets a reader recover the app DID
/// with no new registry record type.
///
/// # Errors
/// The label exceeds `MAX_DNS_LABEL_LEN` (the three hashed segments cost 30
/// characters, so the nickname budget is 33).
pub fn generate_logical_host(
    nickname: &str,
    app_did: &str,
    service_name: &str,
    interface: &str,
) -> anyhow::Result<String>;
```

Both build the label, then `ensure!(label.len() <= MAX_DNS_LABEL_LEN, ...)`
with a message naming the limit and the actual length, then append
`.localhost`. `interface: Option` on the physical builder preserves
`roymctl alias`'s existing "no `--interface` prints the bare alias" branch,
which stays in `commands.rs` because it prints an alias, not a host.

#### 1c. Call sites

| File:line | Change |
|---|---|
| [gateway.rs:254-310](../../../../crates/client_gateway/src/gateway.rs#L254) | **Delete `parse_target_service_and_interface` entirely.** `handle_connection` reads the `Host` header itself (the loop it lives in already has the parsed `Request`) and calls `protocol_utils::parse_target_host` |
| [gateway.rs:178](../../../../crates/client_gateway/src/gateway.rs#L178) | `let target = match parse_target_host(host_header) { Some(t) => t, None => return write_json_rpc_error(&mut stream, 400, "Missing or invalid Host header").await }` — the 400 body is unchanged |
| [bootstrap.rs:174](../../../../crates/coordinator_webrtc/src/bootstrap.rs#L174) | Matches on `TargetHost` instead of destructuring a tuple; the `None` arm's `(host.clone(), "")` fallback is preserved |
| [commands.rs:214-222](../../../../apps/roymctl/src/commands.rs#L214) | `Commands::Alias` prints `util::generate_service_host(nickname, &service_id, Some(&iface))?` when `--interface` is given; the no-interface branch still prints `generate_alias(...)` |
| [basic_lifecycle.rs:426-428](../../../../crates/substrate/tests/basic_lifecycle.rs#L426) | `util::generate_service_host(Some("tcp-demo-app"), &app_service_id, Some("default")).unwrap()` |
| [basic_lifecycle.rs:605-607](../../../../crates/substrate/tests/basic_lifecycle.rs#L605) | `util::generate_service_host(Some(nickname), app_service_id, Some(GREETER_INTERFACE_NAME)).unwrap()` |
| [tcp_proxy_latency.rs:98-100](../../../../tests/perf/src/scenarios/tcp_proxy_latency.rs#L98) | `util::generate_service_host(Some("tcp-perf"), &app_service_id, Some("default")).unwrap()` |

After this phase `short_hash` has no caller that formats a host by hand, which
is exit criterion 5's "demonstrated by the diff".

### Phase 2 — the supervisor accepts a hashed service name

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

### Phase 3 — the client gateway's logical path

#### 3a. Config

`crates/core/src/config.rs`, `ClientGatewayRole`
([config.rs:928](../../../../crates/core/src/config.rs#L928)):

```rust
pub struct ClientGatewayRole {
    pub http_port: u16,
    /// Path to a `CapabilityToken` granting `supervisor/resolve` on the
    /// apps this gateway may resolve logical hostnames for (ADR-0022 §5,
    /// D-S2-7). Absent means every logical hostname is refused by the
    /// supervisor it reaches, since `resolve` is authorized and this
    /// gateway's node DID holds nothing by default -- a startup warning
    /// names this key when a registry is configured and this is not.
    /// Physical (`-p`) hostnames are unaffected.
    #[serde(default)]
    pub resolve_ucan: Option<PathBuf>,
}
```

`Default` gains `resolve_ucan: None`. `#[serde(default)]` on the struct is
already present, so no existing config file changes.

#### 3b. `crates/client_gateway/Cargo.toml`

Add `syneroym-app-orchestration.workspace = true`. No cycle:
`app_orchestration` depends only on `syneroym-identity`, and `sdk` (already a
dependency here) already depends on `app_orchestration`.

#### 3c. `GatewayState`

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
    /// `short_hash(app_did)` -> the app DID a Tier-1 lookup returned,
    /// so a repeat request does not re-resolve Tier 1. Bound to the hash
    /// at insert time (D-S3-5), so a cache hit is as checked as a miss.
    app_dids: DashMap<String, AppDid>,
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
        None => warn!(
            "client gateway has no `roles.client_gateway.resolve_ucan`; logical \
             (-a…-s…) hostnames will be refused by any supervisor they reach. \
             Physical (-p) hostnames are unaffected."
        ),
    }
    Some(f)
}
```

`read_capability_token` mirrors `roymctl`'s own `--ucan` loading (read the
file, `serde_json::from_str::<CapabilityToken>`); `syneroym-rpc` is already a
dependency of this crate and re-exports `CapabilityToken`.

#### 3d. `handle_connection`

The `Ok(Status::Complete(_))` arm becomes:

```
let host_header = header value of "host", or 400 as today
let target = parse_target_host(host_header) or 400 as today
let routing_key: Option<Vec<u8>> = header ROUTING_KEY_HEADER's raw value, cloned

let (service_id, interface) = match target {
    TargetHost::Physical { lookup_alias, interface } => (lookup_alias, interface),
    TargetHost::Logical { app_lookup_alias, app_did_hash, service_name_hash, interface } => {
        match resolve_logical(&state, &app_lookup_alias, &app_did_hash,
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

`resolve_logical`, the only new non-trivial logic:

```
async fn resolve_logical(state, app_lookup_alias, a_hash, s_hash, routing_key)
    -> Result<String /* member ServiceId */>
{
    let fetcher = state.fetcher.as_ref()
        .context("no community registry configured; logical hostnames need Tier 1")?;

    // ── Tier 1 (cached) ──────────────────────────────────────────────
    let app_did = match state.app_dids.get(a_hash) {
        Some(d) => d.clone(),
        None => {
            let registry = RegistryClient::new(false, Some(state.registry_url.clone()));
            let rec = registry.lookup(app_lookup_alias, false).await
                .with_context(|| format!("Tier 1 alias lookup '{app_lookup_alias}' failed"))?;
            rec.verify().context("Tier-1 record failed to verify")?;
            // D-S3-5: `RegistryClient::lookup` cannot bind an *alias*
            // lookup to what was asked for, so bind it here.
            ensure!(util::short_hash(&rec.info.service_id) == a_hash,
                "registry answered alias '{app_lookup_alias}' with '{}', whose hash is not \
                 the '-a{a_hash}' this host named", rec.info.service_id);
            let did = AppDid::try_new(rec.info.service_id.as_str())?;
            state.app_dids.insert(a_hash.to_string(), did.clone());
            did
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
    let signed = fetcher.fetch(&app_did, &LogicalServiceName::try_new(s_hash)?).await?;
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

### Phase 4 — the coordinator relay and the in-page verify

#### 4a. `crates/coordinator_webrtc/Cargo.toml`

Add `syneroym-sdk.workspace = true` and
`syneroym-app-orchestration.workspace = true`. No cycle: `sdk` depends on
`core`/`rpc`/`router`/`wit-interfaces`/`identity`/`app-orchestration`, none of
which reach `coordinator_webrtc`; `coordinator_iroh` already depends on `sdk`
the same way.

#### 4b. `BootstrapState` and `CoordinatorRole`

`CoordinatorRole` gains `resolve_ucan: Option<PathBuf>` with the same doc and
default as 3a. `BootstrapState` gains, built in `coordinator.rs` where the
state is assembled:

```rust
/// Tier-1 → Tier-2 fetcher for logical (`-a…-s…`) hostnames. `None` when
/// no registry is configured, matching `registry_url`'s own condition.
pub topology_fetcher: Option<RegistryTopologyFetcher>,
```

The coordinator has no persistent identity today; the fetcher gets one via
`RegistryTopologyFetcher::with_identity` over a per-process
`Identity::generate()` held beside it, so the `resolve_ucan` token's
`audience_did` can be pinned to it. **This is the one place S3 mints a new
identity**, and it is worth naming: the token an operator issues must name that
DID, which means the coordinator must be able to *print* it. Handled by logging
it once at init, in the shape the substrate already logs its own node DID.

#### 4c. `handle_bootstrap`

```
let target = parse_target_host(&host).unwrap_or(Physical{ lookup_alias: host.clone(), interface: "" });

match target {
    Physical { lookup_alias, .. } => { …today's alias-resolution body, unchanged… }
    Logical { app_lookup_alias, app_did_hash, service_name_hash, .. } => {
        // Same two binding checks as the gateway (D-S3-5). The page
        // re-does both, so these are a fast failure, not the trust
        // boundary.
        let (app_did, signed) = fetch_for_bootstrap(&state, &app_lookup_alias,
                                                    &app_did_hash, &service_name_hash).await?;
        // The page picks the member; the coordinator only needs a peer to
        // *reach*, so it resolves Tier 3 for member 0 purely to fill
        // `target_peer_id`. A page that selects a different member
        // re-resolves through the tunnel, which is already how every
        // request after the first works.
        target_peer_id  = tier3_substrate_of(signed.document.members[0]);
        topology_document_json = serde_json::to_string(&signed)?;
        app_did_str = app_did.to_string();
    }
}
```

`PeerProxyTemplate` gains two fields, both empty strings on the physical path:

```rust
struct PeerProxyTemplate {
    target_peer_id: String,
    target_service_id: String,
    signaling_server_url: String,
    http_version: String,
    target_pubkey_hex: String,
    /// The verbatim `SignedTopologyDocument` JSON (ADR-0022 §3's relay).
    /// Empty on a physical (`-p`) host. The page verifies this against
    /// the app DID it derives from its own hostname, so this template
    /// value is data, never authority.
    topology_document_json: String,
    /// Empty on a physical host. Advisory only: the page checks it
    /// against `-a<hash>` from `window.location.hostname` before use.
    app_did: String,
}
```

`peer-proxy.html` gains, inside the existing inline `<script>`:

```html
const TOPOLOGY_DOCUMENT = {{ topology_document_json|json }};
const APP_DID = "{{ app_did }}";
```

(`askama`'s `json` filter, so a document containing a quote cannot break out of
the literal. Verify the filter is enabled for this askama version; if not, the
Rust side base64-encodes and the page decodes.)

#### 4d. `templates/peer-proxy.js`

Four additions, in `syneroym-topology` order:

1. **`z32Encode(bytes)`** — the same 32-character z-base-32 alphabet
   `z32::encode` uses, 5 bits per character. ~15 lines.
2. **`canonicalJson(value)`** — mirrors
   [`canonicalize_json_value`](../../../../crates/identity/src/substrate.rs#L180)
   followed by `serde_json::to_string`: recursive key sort, arrays in order,
   scalars unchanged. **The escaping must match `serde_json`, not
   `JSON.stringify`** — `serde_json` emits ``/`` where
   `JSON.stringify` emits `\b`/`\f`, and both use the short forms for
   `\n \r \t \" \\`. Pinned by test 78, which compares against a
   Rust-generated fixture rather than being reasoned about.
3. **`verifyTopologyDocument(signed, expectedAppDid)`** —
   `resolve_did_key`'s inverse in JS (strip `did:key:h`, z32-decode, drop the
   two-byte multicodec prefix `0xed 0x01`, leaving 32 raw bytes), then
   `crypto.subtle.importKey('raw', pk, {name:'Ed25519'}, false, ['verify'])`
   and `crypto.subtle.verify('Ed25519', key, z32Decode(signed.signature),
   new TextEncoder().encode(canonicalJson(signed.document)))`. Then
   `signed.document.not_after > Date.now()/1000`.
4. **The hostname binding and member selection**, run once at page load
   when `TOPOLOGY_DOCUMENT` is non-empty:

```
const label = location.hostname.replace(/\.localhost$/, '').split('.')[0];
const aSeg = label.match(/-a([^-]+)-s/)?.[1];
if (!aSeg || (await shortHash(APP_DID)) !== aSeg) fail("app DID does not match this hostname");
const signed = JSON.parse(TOPOLOGY_DOCUMENT);
if (!await verifyTopologyDocument(signed, APP_DID)) fail("topology document did not verify");
// Singleton/Redundant only in S3: an unkeyed page has no routing key to
// offer, and `Sharded` is compiled by nothing (backlog).
TARGET_SERVICE_ID = signed.document.members[
    signed.document.mode === 'Singleton' ? 0 : Math.floor(Math.random()*signed.document.members.length)];
```

`fail(msg)` replaces the page body with the message and stops — a page that
cannot verify must not silently fall back to `TARGET_SERVICE_ID` as the
coordinator supplied it, which is the entire point of the relay.

`shortHash(s)` = `z32Encode(sha256(utf8(s)).slice(0,5))`, matching
`core::util::short_hash`.

The two existing `serviceId = TARGET_SERVICE_ID` sites
([peer-proxy.js:510](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L510),
[:866](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L866))
need no edit — they read the same `let` the block above reassigns.

### Phase 5 — the operator surface, the tests, the docs

- **`roymctl alias`** ([commands.rs:62](../../../../apps/roymctl/src/commands.rs#L62))
  gains `--service <LOGICAL_SERVICE_NAME>`. With it, `service_id` is read as
  the **app DID** and `--nickname` becomes required (it must be the
  `AppInstanceId`, D-S3-2); the output is
  `util::generate_logical_host(nickname, &service_id, &service, &iface)?`.
  Without it, behaviour is exactly today's. A clap-level check refuses
  `--service` without `--nickname` and `--service` without `--interface`,
  each naming why.
- **`docs/developer-guide.md`**: the gateway-hostname section gains the
  logical form, the `X-Syneroym-Routing-Key` header, and the two
  `resolve_ucan` config keys with the warning they suppress.
- **`docs/planning/deferred-backlog.md`**: §5's rows per §5 below.

---

## §4 — S3 tests

**e2e cases are marked; everything else is a unit test.** Numbering is
per-milestone and continues from S2's 59.

**Phase 1 — the shared builder and parser:**

60. `a_physical_host_parses_exactly_as_it_did_before` — the `-p`/`-i` form,
    including a nickname containing dashes, an absent `-i`, and an explicit
    port; asserts the `lookup_alias` string byte for byte against what the
    deleted duplicate produced (D-S3-10, the no-regression pin)
61. `a_logical_host_parses_into_an_alias_a_hash_and_two_more_hashes`
62. `a_logical_host_with_a_dashed_nickname_reassembles_the_nickname` — the
    property the right-to-left grammar exists for, on the new form
63. `an_s_segment_without_an_a_segment_is_not_a_logical_host` — malformed
    input returns `None` rather than a half-built `Logical`
64. `the_reconstructed_app_alias_matches_what_the_registry_admitted` — builds
    an `EndpointInfo` the way `sign_tier1_record` does, derives the alias the
    way `register_endpoint` does (`generate_alias`), builds the host with
    `generate_logical_host`, parses it, and asserts the two alias strings are
    equal. **This is D-S3-2's whole load-bearing claim**, and it is the one
    test that would catch either side drifting
65. `a_host_label_over_the_dns_limit_is_refused_not_truncated` — D-S3-9, on
    both builders, asserting the message names 63
66. `a_built_host_round_trips_through_the_parser` — both forms, property-style
    over a handful of nickname/name/interface shapes

**Phase 2 — the supervisor's hash reversal:**

67. `a_service_name_resolves_to_itself` — the exact-match path is unchanged
68. `a_short_hash_of_a_service_name_resolves_to_that_name`
69. `an_exact_name_wins_over_a_hash_that_matches_a_different_name` — construct
    a plan with a service literally named `short_hash("other")`
70. `two_service_names_sharing_a_short_hash_are_refused` — `AmbiguousHash`,
    with both names in the message. Built by searching a small space of
    generated names for a five-byte SHA-256 collision at test-fixture
    construction is impractical, so this test constructs the collision by
    stubbing the candidate set, not by finding one — asserted as "the branch
    exists and refuses", which is what it is
71. `an_unknown_hash_is_invalid_params_not_internal_error` — the mapping S2's
    post-merge finding 12 established for `NoSuchService`, extended
72. `resolve_answers_a_hashed_service_name_with_a_document_naming_the_real_name`
    — the property phase 3's D-S3-5 check depends on: the signed document
    never echoes the hash back

**Phase 3 — the gateway:**

73. `a_physical_host_reaches_the_same_service_it_did_before` — the gateway's
    own regression pin, at unit scale over the parse + target selection, no
    network
74. `a_tier1_record_whose_hash_does_not_match_the_a_segment_is_refused` —
    D-S3-5's first half, and the reason it exists: a registry answering an
    alias with another app's valid record
75. `a_document_naming_a_different_service_than_the_s_segment_is_refused` —
    D-S3-5's second half
76. `a_second_request_for_the_same_logical_host_makes_no_network_call` —
    task.md **budget 1** at the gateway, asserted as a fetch count against a
    counting `TopologyFetcher`, not as a timing
77. `an_expired_entry_triggers_one_refetch_rather_than_a_failure` — D-S3-8,
    ADR-0022 §3's "on expiry try to refresh", which nothing implemented before
    this slice
78. `a_routing_key_header_selects_a_member_and_its_absence_does_not` — over a
    `Redundant` document; the same key twice returns the same member, no
    header returns members in round-robin
79. `a_sharded_service_with_no_routing_key_fails_with_the_resolvers_own_error`
    — ADR-0022 §7's closing sentence, asserted on the error text
80. `a_gateway_with_no_resolve_ucan_warns_at_init_naming_the_config_key` —
    D-S3-6, in the shape S1's no-registry warning test uses

**Phase 4 — the coordinator and the page:**

81. `a_logical_bootstrap_request_renders_the_signed_document_into_the_page` —
    an axum-level test over `app(state)`, asserting the rendered HTML contains
    the document JSON and the app DID
82. `a_physical_bootstrap_request_renders_an_empty_document_field` — the
    no-regression half
83. `the_js_canonicalizer_matches_serde_json_for_a_fixture_document` — a Rust
    test that writes `canonicalize_json_value` + `to_string` output for a
    document containing every escaping case (`"`, `\`, newline, tab, ``,
    a non-ASCII character) into a fixture file the Playwright suite loads and
    compares against `canonicalJson`. Split across the two suites because
    neither can run the other's code, and §0.6 names this as the one step
    most likely to be silently wrong
84. **(Playwright)** `a_logical_hostname_verifies_the_relayed_document_and_reaches_a_member`
    — the browser half of the reference scenario: navigate to a
    `-a…-s…-i…` host, assert the page's own verification passed and the app
    responded
85. **(Playwright)** `a_tampered_relayed_document_stops_the_page` — the
    coordinator serves a document with one member DID edited; the page must
    show the failure and must **not** fall back to `TARGET_SERVICE_ID`.
    ADR-0022 §3's relay claim, tested from the side that matters

**Phase 5 — operator surface and end to end:**

86. `roymctl_alias_with_a_service_prints_the_logical_form`
87. `roymctl_alias_with_a_service_and_no_nickname_is_refused` — the
    `AppInstanceId` requirement D-S3-2 rests on
88. **(e2e)** `an_http_client_reaches_an_apps_logical_service_by_hostname_alone`
    — two real substrates and a real registry, in
    `topology_document_e2e.rs`'s shape: submit + adopt an app with
    `replicas > 1`, build the host with `generate_logical_host`, POST through
    the gateway, assert the app answered. The milestone's cross-app half,
    from an ordinary HTTP client
89. **(e2e)** `a_keyed_request_reaches_a_stable_member_and_an_unkeyed_one_spreads`
    — the routing-key header over the wire, against a real `Redundant`
    service
90. **(e2e)** `a_logical_hostname_for_an_app_this_gateway_holds_no_grant_for_is_refused`
    — matrix row 7 at the hostname layer: a clean 502 with the denial logged,
    and no member DID anywhere in the response

**Matrix coverage after S3.** No row of task.md's failure/security matrix is
newly S3's — S1 closed 1, 2, 3, 11 and S2 closed 4-10. Three rows gain a second
named test at a new layer, which is the point of this slice rather than an
extension of coverage: row 6 (expiry) → 77, row 7 (clean denial) → 90, row 10
(the epoch carried and preserved) → 72, which pins that a hashed request still
produces a document carrying the real name and the real epoch.

**Performance-budget coverage.** Budget 1 ("resolution after the first fetch:
no network call") is re-asserted at the gateway by test 76 — S2 proved it for a
program holding a `LogicalResolver` directly, and the gateway is the first
caller that reaches it through a hostname, where a per-request Tier-1 lookup
would be the easy mistake. Budget 3 (verify once per fetch) holds unchanged:
`register_verified` is still the only caller of `verify`. Budgets 2 and 4 are
S1's and are untouched.

---

## §5 — Backlog rows this slice creates, and the two it closes

**Closed** (delete the code marker if any, move the row to *Recently
resolved*):

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
  (§0.7, D-S3-9). The DNS label budget leaves 33 characters for the nickname,
  and `generate_logical_host` refuses past it — at host-build time, long after
  `submit` accepted the instance. Target: **TBD**; the fix is a validator on
  `AppInstanceId`, which is a refusal at `submit` rather than at a browser.
  Source: `crates/app_orchestration/src/models.rs`;
  `crates/core/src/util.rs`.
- **A gateway or coordinator can only resolve apps whose operator issued it a
  grant** (§0.3, D-S3-6). `resolve_ucan` is a static, operator-installed token,
  so a browser reaching an app on an unaffiliated node still gets a denial.
  Closes when S4 adds ADR-0022 §5's "open to all" manifest declaration.
  Target: **S4**. Pairs with the existing *"`resolve`'s visibility is a
  capability check with no manifest declaration"* row, which it should link
  to rather than duplicate.
- **The bootstrap page selects a member without a routing key** (phase 4). A
  browser has no way to express one — ADR-0022 §7's header is an HTTP request
  header, and the page's own selection happens before any request is built.
  `Singleton` and `Redundant` are correct; a `Sharded` app is not addressable
  from a browser at all. Not reachable today (`Sharded` is compiled by
  nothing), and it joins that row's dependents. Target: **S5**.
- **The WebRTC coordinator now mints a per-process identity** (phase 4b). It
  had none, and the `resolve_ucan` token must name it, so a coordinator
  restart invalidates an operator's token. Target: **TBD**; the fix is a
  persisted coordinator key file, the shape
  `load_or_generate_node_identity` already uses in the client gateway.

**Updated, not closed:** *"No early cache invalidation for a Tier-2 topology
document"* — D-S3-12 declines to subscribe, and the row's "S3, if a subscriber
appears" target moves to "a consumer needing sub-`cache_ttl` convergence".

---

## §6 — What closing S3 closes

| Against | Closed by S3 | Note |
|---|---|---|
| task.md slice S3 | Fully | Hostname, routing-key header, coordinator relay |
| `[TOP-ADR]` Service Addressing | The external form changes | Existing row, `Complete` at the M1 logical-ref level; this is the S3 change task.md flags |
| `[PLT-DAP-01]` cross-app half | The last third | S1 published Tier 1, S2 made it fetchable by a program, S3 makes it reachable by an ordinary HTTP client and a browser. Recorded `Complete` with S1-S3 evidence, per exit criterion 4 |
| ADR-0022 §3's relay claim | Tested rather than asserted | Test 85 is the first place a relayed document is checked *by the relay's own consumer* |
| ADR-0022 §7 | Fully, except its epoch sentence | §6's on-request epoch is §0.8's declared omission |
| D-S2-13's backlog row | The gateway half | The guest half stays open for S4 |

**Explicitly not closed:** the epoch on the request (S5); per-service
visibility declaration (S4); cross-app `Bind` (S4); shard rebalancing (S5);
the browser's inability to express a routing key (S5).

---

## §7 — The milestone's exit criteria, against this slice

| # | Criterion | S3's part |
|---|---|---|
| 1 | Reference scenario end to end | Tests 88-90 extend it to the HTTP-client entry point; steps 1-8 themselves are S1/S2's and stay green |
| 2 | Every matrix row has a named test | No new rows; three gain a second named test at the hostname layer (§4) |
| 3 | Every budget has a measurement | Budget 1 re-measured at the gateway (test 76) as a fetch count, not a timing |
| 4 | `[PLT-DAP-01]` cross-app half recorded Complete with S1-S3 evidence | **This slice completes the evidence.** Update `traceability-matrix.md` at closeout |
| 5 | The hostname change goes through `core::util` / `core::protocol_utils` only | **Phase 1 is what makes this true** (§0.1) — today it is not. The diff must show `client_gateway`'s duplicate parser deleted and every hand-formatted host replaced |
| 6-9 | fmt / clippy / `cargo test --workspace` / `mise run test:e2e` | All four. **`mise run test:e2e` genuinely matters for the first time in this milestone** — S1 and S2 both recorded it as "unaffected, no client-gateway or WebRTC surface touched", and phase 4 touches both |
| 10 | `wasm32-wasip2` components rebuild against changed WIT | `supervisor.wit` is unchanged (phase 2 is a doc-comment change only), so this is a no-op — worth confirming rather than assuming |

---

## §8 — Questions for the requester

1. **Is §0.3's `resolve_ucan` config the right shape, or should the gateway
   instead be granted implicitly when the app's supervisor is on the same
   node?** The implicit form would make the single-node case work with no
   config at all, which is most local development. It costs a second
   authorization path in a component whose own `TODO(post-B0)` already says
   its caller DID "holds nothing node-wide", and it does nothing for the
   two-node case S3 exists to serve. I recommend the config file and would
   add the implicit case only if single-node friction turns out to matter.
2. **Should the bootstrap page verify the relayed document, or only carry
   it?** §0.6 and D-S3-11 recommend verifying, because relay-without-verify
   leaves the coordinator trusted and removes the reason ADR-0022 §3 chose a
   document over an RPC answer. The cost is phase 4d — a JS canonicalizer that
   must match `serde_json`'s escaping exactly (test 83 exists because that is
   the part most likely to be silently wrong) and a WebCrypto Ed25519
   dependency, which needs Chrome 137+ / Safari 17+ / Firefox 130+. Dropping
   it is a clean cut: phases 1-3 and 5 are unaffected, and the row moves to
   the backlog.
3. **Does `-p` addressing stay indefinitely, or is it deprecated once M6's
   shell exists?** D-S3-10 keeps both. `-p` is the only way to address a
   service that belongs to no app instance, which is every `roymctl svc
   deploy`, so I assume it is permanent — worth confirming, since the answer
   decides whether the developer guide presents two forms or one form and a
   legacy note.
