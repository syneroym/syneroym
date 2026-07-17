# ADR-0018: Declared Service Record Visibility

**Status**: Proposed — design note for agreement before implementation. Surfaced
during M04A Slice B7 planning; see
[plans/B7.md](../planning/milestones/M04A-proxy-and-auth-foundation/plans/B7.md)
flag F9 and §6.2 for the findings this builds on.

**Context**:

Whether a deployed service's record reaches the community registry is today
**incidental, not declared**. A service is published if and only if a pre-signed
`registry_certificate` happened to be supplied in its deploy manifest —
`roymctl svc deploy --identity <name>` builds one
([svc.rs:74-89](../../apps/roymctl/src/commands/svc.rs#L74)); without the flag
`cert = None` and nothing is ever published. Publication is thus a side effect of
*which flag you passed*, not a statement of intent, and "deployed but
undiscoverable" is indistinguishable from "deployed and deliberately private".

Three things do not exist (verified, not assumed):

1. **A visibility flag at any layer.** `ServiceConfig`
   ([models.rs:210](../../crates/app_orchestration/src/models.rs#L210)) has no
   such field; WIT `service-config`
   ([control-plane.wit:23](../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L23))
   has none. `deploy-manifest` carries only `registry-certificate:
   option<string>`.
2. **Any way to share a record privately.** `SignedEndpointInfo` is a
   self-contained signed blob that *could* be handed to a peer, but nothing
   exports or imports one. The only out-of-band connect path,
   `SyneroymClient::new_with_mechanisms`
   ([sdk/src/lib.rs:178](../../crates/sdk/src/lib.rs#L178), short-circuiting the
   lookup at :234-245), takes **raw mechanisms** — no signature, nickname, TTL,
   or attribution.
3. **Cross-node reachability for an unpublished service.**
   `ProxyRouter::invoke_remote` resolves *only* through the registry
   ([proxy.rs:322](../../crates/router/src/proxy.rs#L322) →
   [net_iroh.rs:133](../../crates/router/src/net_iroh.rs#L133)).

### The finding that shapes everything

**A `Service` record's payload is just the mapping `service_id → substrate_id`.**
It carries `mechanisms: vec![]`; the addressing is grafted on at lookup time:

```rust
// dht_registry.rs:307-312
if resolve && info.info.endpoint_type == EndpointType::Service {
    let sub_info = Box::pin(self.lookup(&info.info.substrate_id, false)).await?;
    info.info.mechanisms = sub_info.info.mechanisms;
}
```

So reaching a service needs two facts: **where it lives** (the Service record)
and **how to reach that node** (the substrate's own record — which is public,
since a substrate publishes itself on every heartbeat).

This collapses the problem. "Private" does **not** need a new addressing or
transport mechanism. It needs *somewhere other than the public registry to put a
one-line mapping*, and a way to hand that mapping to whoever should have it. A
privately-shared service record is a signed assertion that "X lives on S" —
nothing more.

### The constraint that bounds everything

The registry requires a record for DID X to be **signed by X's own key**:
`SignedEndpointInfo::verify()`
([dht_registry.rs:101-117](../../crates/core/src/dht_registry.rs#L101)) resolves
the expected pubkey from `info.service_id` and hard-fails on mismatch;
`verify_endpoint_signature` calls it on every `/register`. **The substrate
cannot sign a record for a service it hosts** — deliberately, since that is what
stops a hostile substrate publishing for services it does not host. This ADR
does not relax it.

Consequence: the *signer* is the service key holder (today, `roymctl` with
`--identity`), so anything inside the signed payload — including `is_private` —
is decided at signing time, not by the substrate.

**Decision**:

### 1. Visibility is declared in the manifest, as a three-valued enum

Add `visibility` to `ServiceConfig` (Rust) and `service-config` (WIT), kept in
sync:

```rust
/// Whether this service's endpoint record is published, and how far it
/// travels. Declared, not inferred from whether a certificate was supplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Registered, and propagated to parent registries. `is_private: false`.
    Public,
    /// Registered with this substrate's registry only; **not** propagated
    /// upward. `is_private: true`.
    Internal,
    /// Never registered anywhere. The record exists and is signed, but is
    /// shared out of band (§2) and resolved locally (§3).
    #[default]
    Private,
}
```

```wit
// control-plane.wit, record service-config
/// Endpoint-record visibility (ADR-0018). Defaults to private.
visibility: option<visibility>,

enum visibility { public, internal, private }
```

**Three values, not a bool**, because the middle tier already exists in the
registry and is honored (§4) — a bool would either strand it or silently
redefine it.

**Default `private`** — argued in §5. `option<visibility>` in WIT is for
tolerant decoding of a field older callers omit, **not** a compatibility
concession: `None` means `private`, same as a caller who said so.

### 2. A private record is exported and imported as a `SignedEndpointInfo`

It is already the right artifact: signed, self-contained, independently
verifiable via `verify()`, and it carries nickname/TTL/`delegation` — everything
`new_with_mechanisms` throws away.

- **Export.** `roymctl` already *builds* the certificate at deploy time; it
  writes it to a file instead of (or as well as) handing it to the substrate:
  `roymctl svc deploy … --visibility private --record-out ./svc.record.json`.
  No new substrate RPC.
- **Import, client.** `SyneroymClient::new_with_record(SignedEndpointInfo)` —
  `verify()` it, read `substrate_id`, resolve *that* through the registry
  (public), connect. This supersedes `new_with_mechanisms` for every case that
  has a record; keep the raw-mechanisms constructor for genuinely
  registry-less/bootstrap use.
- **Import, peer substrate.** §3.

**Deliberately not a substrate RPC** (`get-record <svc_id>`): it would make the
substrate a distributor of private records, which immediately raises "who may
read one?" — a Tier-1/ownership question that belongs to B7 and that this design
does not need. The record holder is whoever deployed it; let them share it.

### 3. Resolution consults a local record store before the registry

This is the load-bearing part: without it "private" silently means "same-node
only".

Add a **known-records store** — privately imported `SignedEndpointInfo`s,
verified on import *and* on load, persisted next to the endpoint registry
(`endpoints.db`, a `known_records` table alongside B7a's `service_owners`).
Resolution becomes:

```
resolve(target):
    if let Some(rec) = known_records.get(target):       # privately shared
        substrate_id = rec.info.substrate_id
    else:
        rec = registry.lookup(target, resolve = true)   # today's path
        return rec.info.mechanisms
    # the mapping was private; the *node* is public
    return registry.lookup(substrate_id, false).mechanisms
```

Threaded into `net_iroh::resolve_iroh_addr`, which is the single chokepoint
`ProxyRouter::invoke_remote` uses. Note the second hop is exactly what
`lookup(_, resolve=true)` already does internally — this reuses the mechanic
rather than inventing one.

**Scope note that shrinks this considerably:** *same-node* sibling services
already work — `ProxyRouter` tries `registry.lookup` on the **local**
`EndpointRegistry` first and only falls through to `invoke_remote`
([proxy.rs:422-426](../../crates/router/src/proxy.rs#L422)) — and a
`DeploymentPlan` deploys to one substrate, so an app's services are same-node by
construction. The store is therefore needed for genuinely **cross-node**
private targets (federated apps, cross-app calls), not for the common intra-app
case.

### 4. `is_private` composes; it is not subsumed

`is_private` today means *"do not propagate to the parent registry"*
([registry.rs:226](../../crates/community_registry/src/registry.rs#L226)) —
federation scope, not publication. It is the wire encoding of exactly one tier:

| `visibility` | Certificate supplied to substrate? | `is_private` in the signed record | Result |
|---|---|---|---|
| `public` | yes (required) | `false` | registered, propagated upward |
| `internal` | yes (required) | `true` | registered here, not propagated |
| `private` | **no** | n/a — never on the wire | not registered; shared out of band |

Because `is_private` lives *inside the signed payload*, it must be set by the
signer at deploy time. The substrate therefore **validates rather than decides**:

- `visibility` is `public`/`internal` but no certificate → **fail the deploy**
  with a clear error. This is the whole complaint fixed: "you said public and
  gave me nothing to publish" stops being silence.
- certificate present but `info.is_private != (visibility == Internal)` → **fail
  the deploy**; the declaration and the signed artifact disagree.
- `visibility` is `private` but a certificate was supplied → **fail the
  deploy**; refuse to guess which the operator meant.

The substrate stores the declared `visibility` per service (alongside B7a's
owner row) so `list` can report it and the heartbeat can honor it. The publish
loop needs no change beyond that: `private` services never place a certificate in
`hosted_apps_dir`, so the existing relay naturally skips them.

### 5. No migration

**The product is unreleased.** There is no compatibility to preserve and no
migration to write, and inventing one would be permanent complexity paying for
users who do not exist. Pick the default on its merits and change the behavior.
(The `PRAGMA user_version` mechanism stays in place for the first migration that
has real users.)

**Default `private`, argued on merits alone:** publication is a privacy
decision, and defaulting it *on* contradicts the project's sovereignty grain.
More concretely, `public` would preserve the exact accident this ADR exists to
remove — publication would still follow from *holding a key* (having passed
`--identity`) rather than from intent.

The one behavior change worth knowing (not a migration, no strategy required):
`roymctl svc deploy --identity X` publishes today and will fail after this ADR
until `--visibility public` is added — deliberately loud rather than silently
reverting to unpublished. **Verified blast radius: nil.** Nothing in the tree
passes `svc deploy --identity` — both e2e setups
(`global-setup.ts:155`, `global-setup-multihop.ts:274,288`) deploy without it,
and no smoke test uses that path. Nothing publishes a service record today.

**Consequences**:

- **Enables**: publication as a declared property of a service rather than an
  artifact of its deploy invocation; private services that remain reachable by
  clients and by cross-node peers holding their record; a real (signed,
  verifiable) out-of-band record-sharing path in place of raw mechanisms; and a
  loud failure where a mis-declared service is currently silent.
- **Behavior change (one path, deliberately)**: `svc deploy --identity` without
  `--visibility public` now fails instead of publishing. No compat shim — the
  product is unreleased and nothing in the tree exercises that path (§5).
- **Non-goals / defers**: relaxing the signer constraint so a substrate may
  publish for a service it hosts (its own ADR — and note M04A B7's F9 found the
  *motivation* for it unsound); revocation/expiry of a shared private record
  beyond the TTL `EndpointInfo` already carries; using `delegation` to let an
  owner's master key vouch for a per-service key (it already verifies —
  [dht_registry.rs:138-145](../../crates/core/src/dht_registry.rs#L138) — but no
  flow populates it).

**Implementation Notes**:

- **The WIT change is *not* guest-facing** — `control-plane.wit` is absent from
  `wit/host/deps/` and not imported by `host.wit`, so it drives no
  `wasm32-wasip2` guest build (contrast `data-layer`, which has two synced
  copies). It is the orchestrator's JSON-RPC contract: one file, one additive
  field. Still verify `wasm32-wasip2` per the milestone gate.
- Flow for the app path: `ServiceConfig.visibility` → `compile` →
  `DeploymentPlan` → `mapper::map_deployment_plan_to_wit`
  ([sdk/src/mapper.rs:15](../../crates/sdk/src/mapper.rs#L15)) → WIT
  `service-config`. `svc deploy` needs a `--visibility` flag since it builds a
  `DeployManifest` directly with no `SynAppManifest`.
- **Prerequisite worth surfacing early: a private service still needs a
  keypair**, because its record must be signed by its own key to be verifiable
  by an importer. Today that key is `--identity` on `svc deploy` and does not
  exist at all on the `app deploy` path. Any rollout must answer where
  per-service keys come from; `EndpointInfo.delegation` (owner master →
  per-service temporary key) is the shape the code already anticipates.
- **Do first, independent of this ADR**: `svc deploy` must validate that
  `--identity`'s `did:key` equals `--svc-id`. Today a mismatch silently builds a
  certificate the registry rejects at `/register` forever, while printing
  success — see B7.md §6.2.

**Alternatives considered**:

- **A bool `public: bool`.** Rejected: `is_private` already implements a
  distinct middle tier that is honored by the registry today; a bool would
  strand it or redefine it silently.
- **Keep visibility client-side (roymctl decides; nothing crosses the WIT).**
  Rejected: the substrate could never report visibility in `list`, every deploy
  path would reimplement the rule, and "no certificate" would remain
  indistinguishable between *private* and *forgotten* — which is the exact defect
  this ADR exists to remove.
- **Let the substrate hold and serve private records over an RPC.** Rejected for
  now: it makes the substrate a distributor of private records and immediately
  raises "who may read one?", pulling B7's ownership/Tier-1 work into a
  discoverability change. The deployer already holds the record.
- **Give `invoke_remote` a raw provided-mechanisms escape hatch** (mirroring
  `SyneroymClient::new_with_mechanisms`) instead of a record store. Rejected:
  unsigned and unattributed, it would let any local misconfiguration silently
  redirect a service DID to an arbitrary node — the store's `verify()`-on-import
  is the point, not an accident.
