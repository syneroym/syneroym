# Slice A0 Implementation Plan — Stable Member Identity

**Status:** 📋 Planned (2026-07-28, revised after review). Ten decisions below
(D-A0-1 … D-A0-10).
Planning found **four** places where the design of record asserts something the
tree does not do (§6 items 3, 4, 7, 11); they are called out rather than
silently worked around, and item 7 lands outside A0 — it moves A2's scope, so
it is written into A2's slice text and task.md matrix row 19, not left here.
A review pass then found a fifth (§6 item 6) plus three coverage errors, all
folded in: see D-A0-10, §3.1's corrected call-site inventory, and §3.4.

**All line anchors are against `ae07375`.**

**Source of record:** [ADR-0020](../../../decisions/0020-stable-logical-service-identity.md)
§1-§5, [task.md](task.md) Slice A0.
**Paired:** [ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md).
**Requirement:** `[FND-IDT]`.
**Slice order:** A0 → A1 → A2 → P0 → A3 → A4 → A5. A0 is first because it
carries the milestone's design risk. A6 is post-M5.

---

## 0. What this slice is, in one paragraph

Today a service's identity is the key the hosting substrate derived for it, so
moving or replacing a service changes who it is to FDAE and orphans it from
the rows it wrote. A0 gives each **member** of a logical service a master DID
held by the deployer, has the substrate derive an instance key as it already
does, and binds the two with the existing `DelegationCertificate` under a new
service-instance scope. It also adds the ingress check that makes that scope
mean anything: `scope` is signed today ([delegation.rs:19](../../../../crates/identity/src/delegation.rs#L19),
[:128](../../../../crates/identity/src/delegation.rs#L128)) and read by
nothing.

**The load-bearing surprise is that the presentation path does not exist.**
ADR-0020 §1 says the instance "presents that certificate on its route preamble
the same way a delegated client does today." It does not: a guest-originated
remote call presents **no identity at all**
([proxy.rs:373](../../../../crates/router/src/proxy.rs#L373)), deliberately.
A0 has to build that arm, not inherit it. See D-A0-4 and §6 item 3.

---

## 1. Design decisions

### D-A0-1 — Scope string, and where the ingress compares it

**Resolved: constants in `syneroym-identity`; the comparison goes in
`DelegationCertificate::verify`, as a required argument.**

The strings, next to the struct that carries them
([delegation.rs:14-21](../../../../crates/identity/src/delegation.rs#L14)):

```rust
/// A key delegated to route connections under its master's identity --
/// an operator's device key, a client session key.
pub const SCOPE_ROUTING: &str = "routing";
/// A substrate-derived instance key certified by a member master, so the
/// instance speaks as that member (ADR-0020 §1).
pub const SCOPE_SERVICE_INSTANCE: &str = "service-instance";
/// Scopes admissible as *transport* identity on an inbound connection.
pub const TRANSPORT_SCOPES: [&str; 2] = [SCOPE_ROUTING, SCOPE_SERVICE_INSTANCE];
```

Bare kebab-case, matching `"routing"`'s existing style. Rejected a namespaced
form (`syn:service-instance`): ceremony with no second issuer to disambiguate
from.

**The comparison lives in `verify`, not `verify_preamble`, not the preamble
layer.** The signature becomes:

```rust
pub fn verify(&self, expected_master_did: &str, accepted_scopes: &[&str]) -> Result<()>
```

| Candidate site | Why not |
|---|---|
| `HandshakeVerifier::verify_preamble` ([handshake.rs:43](../../../../crates/router/src/handshake.rs#L43)) | It is one of the certificate's consumers, not the only one. A1 admits endpoint records on a *different* scope requirement, and a supervisor validating a certificate before installing it is a third. Each would need its own copy of the check, and the one that forgets is a silent bypass. |
| The preamble layer (`RoutePreamble::parse`) | Parsing has no idea where the connection is headed, so it could only hard-code one scope for all traffic — which is wrong in both directions (D-A0-1's allowlist argument below). |
| `DelegationCertificate::verify` | The single function every consumer must already call. The check cannot be skipped because verification cannot be skipped. |

**Required argument, not `Option<&str>`.** An optional parameter defaulting to
"accept anything" reproduces today's silence at every future call site, which
is the exact failure mode this decision exists to close. The full call-site
inventory is in §3.1 — **two** production sites, not one, and the second needs
a decision of its own rather than a mechanical edit.

**What the ingress actually requires, stated honestly.** At
`handle_stream`'s verification ([io.rs:332](../../../../crates/router/src/route_handler/io.rs#L332))
the requirement is `TRANSPORT_SCOPES` — a **set of two**, not a single value.
Both a person's delegated device key and a service instance are doing the same
thing there: routing a connection under a master's identity. The router cannot
tell from the target which one legitimately belongs, because a service instance
and a human operator both call `data-layer` on a third service. Narrowing it to
one value at that site locks out one or the other.

That is weaker than task.md's matrix row 2 phrasing suggests, so name what the
check buys: **a certificate minted for any purpose outside transport is not
replayable onto a connection.** The narrow single-value comparison — "this
record may only be admitted by a `service-instance` certificate" — lands at A1's
endpoint-record admission, which is where the replay ADR-0020 §5 describes is
actually reachable: without it, a person's `routing` certificate could publish
endpoint records under their master DID. A0 builds the mechanism and closes the
unknown-scope hole; A1 uses it narrowly. See §6 item 11.

**Also fixed in passing:** `cert.verify(master_did)` at
[handshake.rs:66](../../../../crates/router/src/handshake.rs#L66) passes the
master DID read *from the certificate itself*, so delegation.rs:87's
"confused deputy prevention" is a tautology on the only production path. Not a
hole — the connection asserts "I am delegated by M", and M is whatever the
certificate says; binding to a *target* is a separate authorization question
resolved downstream on `master_did`. But it means `verify`'s first argument
does nothing there, and the new scope argument is the first thing it will
actually enforce. Add a doc comment saying so, so the next reader does not
"fix" it into a real check that has nothing to compare against.

### D-A0-2 — Where the member master is minted, and the CLI surface

**Resolved: `roymctl`. Not `crates/control_plane`, not the SDK mapper.**

- **`control_plane` runs on the substrate.** ADR-0020 §3 forbids the substrate
  ever holding a master private key; minting there is excluded by definition,
  not by preference.
- **The SDK mapper is a pure translation.** `map_deployment_plan_to_wit`
  ([mapper.rs:56](../../../../crates/sdk/src/mapper.rs#L56)) turns a
  `DeploymentPlan` into WIT records and reads local artifact files. It has no
  identity store, no network, and no key material. Giving a serialization
  function custody of the most backup-critical secret in the system is the
  wrong shape regardless of convenience.
- **`roymctl` already is the identity store.** `identity create` writes
  `<dir>/identities/<name>.key`
  ([identity.rs:80-96](../../../../apps/roymctl/src/commands/identity.rs#L80)),
  `identity delegate` issues certificates
  ([identity.rs:139-156](../../../../apps/roymctl/src/commands/identity.rs#L139)),
  and `client_for` already loads a named identity from that directory
  ([commands.rs:138-146](../../../../apps/roymctl/src/commands.rs#L138)). Minting
  a member master is an existing verb applied to a new subject, exactly as
  ADR-0020 §4 says.

**Naming convention.** A member master is an ordinary identity file; A0 fixes
only the name, so resolve-or-mint is deterministic and `identity list` stays
readable:

```
<dir>/identities/member-<app_instance_id>-<service_name>-<index>.key
```

`index` is the member ordinal, `0` for a `Singleton`. `LogicalServiceName`
already forbids `/` ([models.rs:103-105](../../../../crates/app_orchestration/src/models.rs#L103));
`AppInstanceId` has **no** validator ([models.rs:94](../../../../crates/app_orchestration/src/models.rs#L94)),
so the deploy verb rejects an instance id containing a path separator rather
than letting a name escape the identities directory.

**Three CLI surfaces, all additive:**

1. **`roymctl identity certify-instance`** — the primitive.
   ```
   roymctl identity certify-instance \
     --master <identity-name> --substrate <did> --service <did> [--expires-hours 24]
   ```
   Asks the substrate for the derived instance public key (D-A0-3), issues a
   `SCOPE_SERVICE_INSTANCE` certificate over it, installs it (D-A0-4). This is
   also the attended posture's renewal command (D-A0-6).

   **`identity delegate` cannot do this job**: it requires `--temp-did`
   ([identity.rs:32-33](../../../../apps/roymctl/src/commands/identity.rs#L32)),
   which is precisely what the operator does not have until the substrate
   reports it. A new verb, not a new flag on the old one. See §6 item 12.

2. **`roymctl svc deploy --master <identity-name>`** — the single-service path
   ([svc.rs:19-38](../../../../apps/roymctl/src/commands/svc.rs#L19)). Present ⇒
   `--svc-id` must equal that identity's DID (checked client-side with a clear
   error, since a mismatch produces an install-time rejection that is harder to
   read), then fetch, issue, attach. Absent ⇒ exactly today's behavior, which
   is what keeps pre-A0 services working (D-A0-9).

3. **`roymctl app deploy --mint-masters`** — the plan path
   ([app.rs:19-27](../../../../apps/roymctl/src/commands/app.rs#L19)). Resolves
   or mints one master per member, substitutes plan service ids (D-A0-7), then
   certifies each. **Without the flag, a plan needing masters fails with a
   message naming the missing identities** rather than minting silently.

   Silent minting is wrong here and the reason is ADR-0020 §4: losing one of
   these keys is unrecoverable and orphans the member's stored data. Creating
   one must be something the operator asked for, and the mint prints the backup
   warning at mint time — which §4 explicitly requires ("the deploy path must
   say so at mint time rather than letting an operator discover it later").

### D-A0-3 — How the substrate reports its derived instance public key

**Resolved: a read-only `orchestrator` method, called *before* deploy. Not a
value returned *from* deploy.**

The fact that makes this simple: `derive_service_identity`
([keys.rs:240-257](../../../../crates/identity/src/keys.rs#L240)) is a pure
HKDF over `(node identity, owner_did, service_id)`. `owner_did` is
`caller.caller_did` ([orchestration.rs:776](../../../../crates/control_plane/src/service/orchestration.rs#L776)),
which is `id.master_did` ([io.rs:257](../../../../crates/router/src/route_handler/io.rs#L257))
— the deploying operator's own DID. `service_id` under ADR-0020 §2 is the
member master DID. **The deployer knows two of the three inputs; only the
node's identity is private, and the derivation does not require the service to
exist.** So the key can be asked for ahead of time, and the chicken-and-egg
ADR-0020 §3 describes ("an instance key cannot be certified before it exists")
dissolves for the deploy case — it remains real only for a *relocation to a
node the operator has not yet contacted*, which is A5's problem.

Add to the `orchestrator` WIT interface
([control-plane.wit:1-158](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L1)):

```wit
/// The instance signing key this substrate derives for `service-id` under
/// the calling identity. Deterministic and answerable before the service is
/// deployed, which is what lets the master holder certify the key without
/// the substrate ever holding the master.
record instance-identity {
    /// did:key of the derived instance key.
    instance-did: string,
    /// Hex-encoded ed25519 public key -- the form a route preamble carries.
    pubkey-hex: string,
}
instance-identity: func(service-id: string) -> result<instance-identity, string>;
```

with an `"instance-identity"` arm in `ControlPlaneService::dispatch`
([service.rs:321-382](../../../../crates/control_plane/src/service.rs#L321)),
computing `self.node_identity.derive_service_identity(&caller.caller_did, &service_id)`.

**`caller.caller_did`, deliberately, not `registry.owner_of(&service_id)`.**
For a not-yet-deployed service there is no owner row at all, and the caller's
own DID is what makes the answer match what deploy will later derive at
[orchestration.rs:732](../../../../crates/control_plane/src/service/orchestration.rs#L732).
For an already-deployed service the two agree whenever the caller is the owner;
a non-owner gets a key that is useless to them, since they cannot deploy under
that `service_id` anyway
([orchestration.rs:467-474](../../../../crates/control_plane/src/service/orchestration.rs#L467)).

**Gate:** `ORCHESTRATOR_STATUS` on `substrate:<node>/app/<service_id>`, the same
two-step check `readyz` uses
([orchestration.rs:406-413](../../../../crates/control_plane/src/service/orchestration.rs#L406)) —
node-wide ability first, per-app resource second. Status, not deploy: this
returns a public key, not an authority. Enumerating many `(owner, service)`
pairs yields many HKDF-SHA256 outputs of the node secret, which does not expose
the PRK; and a derived public key is worthless without a certificate from a
master the caller does not hold.

**Rejected: return the pubkey from `deploy` and install the certificate in a
second call.** It makes deploy two-phase, and between the phases the service is
live holding no certificate — presenting nothing on outbound calls, silently
back to pre-A0 behavior, for an unbounded window. It also changes
`deploy: func(...) -> result<_, string>`'s return type, which the pre-query
does not.

**Rejected: have the substrate publish its instance DID somewhere the deployer
reads.** There is nothing for A0 to publish it *to* — that is A1's registry
work — and it adds a network hop where a direct RPC already exists.

The resulting deploy flow is a straight line, one extra round trip per member,
and only when a master is in play:

```
1. roymctl                : resolve-or-mint member master        -> master DID
2. roymctl -> substrate   : orchestrator/instance-identity(...)  -> pubkey_hex
3. roymctl                : DelegationCertificate::issue(master, pubkey,
                                ttl, SCOPE_SERVICE_INSTANCE)     -> cert
4. roymctl -> substrate   : orchestrator/deploy(master DID,
                                manifest { instance_certificate: cert })
```

### D-A0-4 — Installing the certificate, and presenting it

**Install: in `deploy-manifest`, beside `registry-certificate`.**

`registry-certificate: option<string>`
([control-plane.wit:119-120](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L119))
is the exact precedent — a client-signed credential attached to a deploy. Add:

```wit
/// JSON DelegationCertificate binding this substrate's derived instance
/// key to the member master that `service-id` names. Verified on receipt;
/// absent leaves the service its own master (ADR-0020 §1 fallback).
instance-certificate: option<string>,
```

The substrate verifies **at install**, and rejects the deploy on any failure:

1. parses as a `DelegationCertificate`;
2. `cert.master_did == service_id` — the certificate is for *this* member;
3. `cert.temporary_did == derive_did_key(derive_service_identity(caller_did, service_id).public_key())`
   — it certifies *this node's* derived key, not some other key the client
   chose;
4. `cert.verify(service_id, &[SCOPE_SERVICE_INSTANCE])` — signature, window,
   expiry, and the narrow scope.

Verifying at install rather than at first use turns a mis-issued certificate
into a deploy error the operator sees immediately, instead of a routing failure
hours later with nothing pointing at the cause.

**Store: `EndpointRegistry`, following B7a's owner-row shape exactly.**

| File | Change |
|---|---|
| [local_registry.rs:58-70](../../../../crates/core/src/local_registry.rs#L58) | `service_certs: Arc<DashMap<String, DelegationCertificate>>` alongside `service_owners` |
| [local_registry.rs:204-224](../../../../crates/core/src/local_registry.rs#L204) | `set_instance_cert` / `instance_cert` / `remove_instance_cert`, mirroring `set_owner`/`owner_of`/`remove_owner` |
| [local_registry.rs:98-111](../../../../crates/core/src/local_registry.rs#L98) | `load_from_db` loads them, as it already does owners |
| [storage.rs:32-38](../../../../crates/core/src/storage.rs#L32) | `load_all_certs` / `save_cert` / `remove_cert` on the `EndpointStorage` trait |
| [orchestration.rs](../../../../crates/control_plane/src/service/orchestration.rs) | `undeploy` removes the certificate where it removes the owner row |

**`EndpointStorage` has four implementors, and the trait declares no default
bodies, so every one of them must gain the three methods** (revision 1 named
only `MockStorage`, which would have left the real backend out):

| Implementor | Kind | Note |
|---|---|---|
| [storage.rs:62](../../../../crates/core/src/storage.rs#L62) `MockStorage` | in-memory test double | a third `DashMap`, same as `owners` |
| [registry_store.rs:96](../../../../crates/data_db/src/registry_store.rs#L96) `SqliteEndpointStorage` | **the production backend** — wired in at startup via `registry_store::init_store` ([runtime.rs:407](../../../../crates/substrate/src/runtime.rs#L407)) | needs a real table (D-A0-10). **Without this, instance certificates do not survive a substrate restart**, which is the whole point of persisting them |
| [orchestration.rs:2280](../../../../crates/control_plane/src/service/orchestration.rs#L2280) `FailingEndpointStorage` | test double | mechanical |
| [router/tests/service_ownership.rs:152](../../../../crates/router/tests/service_ownership.rs#L152) `RemoveOwnerFailingStorage` | test double | mechanical |

`syneroym-data-db` is therefore in A0's blast radius, which revision 1 missed
entirely — and **phase 3's gate had to change with it**: `cargo test -p
syneroym-core` cannot see a `syneroym-data-db` break, so the gate now names both
crates (§2). That ordering matters: with the old gate, the production backend
would have compiled fine locally and failed at `cargo test --workspace` three
phases later.

### D-A0-5 — Where member master keys live

**Resolved: `<roymctl --dir>/identities/<name>.key`, unchanged.** No new store,
no new format, no new permission model. `save_to_path` already writes `0o600`
on unix ([keys.rs:158-169](../../../../crates/identity/src/keys.rs#L158)), so
the file mode these keys need is already what they get.

Rejected for A0: a supervisor-held vault (ADR-0020 §4's end state). There is no
supervisor until A5; building custody for a component that does not exist means
building it twice. The naming convention in D-A0-2 is the forward-compatible
half — a supervisor adopting these later reads them by a name it can compute
rather than by scanning and guessing.

Out of A0 and already tracked in `deferred-backlog.md` §3 ("Member master key
custody, escrow, and loss"): escrow, master rotation, re-attribution after a
loss. A0's contribution is the mint-time warning, which is the only mitigation
ADR-0020 §4 asks for.

### D-A0-6 — Certificate renewal under both postures

**Resolved: A0 ships the attended posture only, and makes a missed renewal
visible before it becomes an outage. The online-key posture is A5's.**

- **Expiry granularity.** `identity delegate --expires-days` is whole days
  ([identity.rs:35](../../../../apps/roymctl/src/commands/identity.rs#L35),
  `expires_days * 24 * 3600`). Service-instance certificates want hours, so
  `certify-instance` takes `--expires-hours`, default 24. Not a change to
  `delegate`.

- **Attended posture (D-A0-2's `certify-instance`, run on the operator's
  cadence).** ADR-0020 §3 is explicit that a missed renewal is an *outage*, not
  a degradation — the handshake fails closed. A0's obligation is therefore
  visibility, not automation. Two pieces:

  1. **Expiry sweep on the existing heartbeat loop.** `publish_to_community_registry`
     ([runtime.rs:614-681](../../../../crates/substrate/src/runtime.rs#L614))
     already walks per-service state once per `HEARTBEAT_INTERVAL_SECS`
     (3600, [dht_registry.rs:26](../../../../crates/core/src/dht_registry.rs#L26)).
     Add a `warn!` for any installed certificate within 25% of its lifetime of
     expiring. No new task, no new timer.
  2. **`roymctl svc list` shows expiry.** So "when does this fall over" is
     answerable without reading logs. `deployed-service`
     ([control-plane.wit:123-127](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L123))
     gains `instance-certificate-expires-at: option<u64>`, and `svc list`'s
     table ([svc.rs:116-129](../../../../apps/roymctl/src/commands/svc.rs#L116))
     gains a column.

- **Online-key posture: deferred to A5, with a named owner.** Automatic renewal
  needs a component that holds master keys and runs unattended. That component
  is the supervisor. A substrate-side renewal timer would require the master key
  on the substrate, which ADR-0020 §3 forbids outright. Backlog row in §7,
  target M05A A5.

**Stated plainly because it is a real operating cost:** through A0–A4 the only
posture that exists is the attended one. The 25%-remaining warning and the
`svc list` column make it operable; they do not make it safe to forget.

**`RotationPolicy` is not used.** ADR-0020 §5 nominates it
([models.rs:190-198](../../../../crates/app_orchestration/src/models.rs#L190))
as the place to express whether a service tolerates in-place certificate
replacement. With no automatic renewal, replacement is always operator-initiated
and always in-place — the substrate re-reads the stored certificate on the next
outbound call, and there is nothing for a policy to choose between. It becomes
load-bearing at A5. Noted, not built. §6 item 13.

### D-A0-7 — Does `ServiceId`'s meaning change need a type change?

**Resolved: no type change anywhere. But it is not purely semantic either, and
the difference is the whole of A0's deploy-path work.**

*Unchanged:* `ServiceId`'s validator checks only the `did:key:` prefix
([models.rs:110-119](../../../../crates/app_orchestration/src/models.rs#L110)),
which a member master DID satisfies. `PlannedService.service_id`,
`PlannedService.resolved_dependencies: Vec<ServiceId>`
([models.rs:379-388](../../../../crates/app_orchestration/src/models.rs#L379)),
`DeploymentPlan` ([models.rs:393-399](../../../../crates/app_orchestration/src/models.rs#L393)),
and `TopologyEntry.members: Vec<ServiceId>`
([resolver.rs:168-181](../../../../crates/app_orchestration/src/resolver.rs#L168))
all keep their types, and the WIT carries these as `string` already
([control-plane.wit:142-146](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L142)).

*Changed:* **the values.** `derive_deterministic_service_id`
([compiler.rs:172-180](../../../../crates/app_orchestration/src/compiler.rs#L172))
fabricates a `did:key` by prepending the ed25519 multicodec to a SHA-256 of the
logical ref. Its own doc comment says what that is: "a temporary M1 hack …
produces a mock key where we do not have the private key, and the 32 bytes may
not be a valid Curve25519 point"
([compiler.rs:161-171](../../../../crates/app_orchestration/src/compiler.rs#L161)).
Under ADR-0020 §2 the value has to become a **real** public key, because
`DelegationCertificate::verify` resolves the master DID through
`substrate::resolve_did_key` ([delegation.rs:119](../../../../crates/identity/src/delegation.rs#L119)),
which rejects a non-point outright
([substrate.rs:90-96](../../../../crates/identity/src/substrate.rs#L90)).

**Shape: substitute after compiling, not inside the compiler.**

- `compile()` keeps producing the fabricated id as the plan's **internal graph
  key** — it is what `resolved_dependencies` is wired from
  ([compiler.rs:126-136](../../../../crates/app_orchestration/src/compiler.rs#L126)).
- `roymctl` substitutes each `PlannedService.service_id` with its resolved
  member master DID and rewrites `resolved_dependencies` through the same map:
  one function in `apps/roymctl`, `(DeploymentPlan, &BTreeMap<LogicalServiceRef, ServiceId>) -> DeploymentPlan`,
  applied between `compile` ([app.rs:90](../../../../apps/roymctl/src/commands/app.rs#L90))
  and `map_deployment_plan_to_wit` ([app.rs:107](../../../../apps/roymctl/src/commands/app.rs#L107)).

Rejected: threading an identity provider into `compile()`. It makes a pure
function impure and newly fallible, and it makes the deployment journal — which
is appended *before* the substitution point
([app.rs:103](../../../../apps/roymctl/src/commands/app.rs#L103)) — record
key-bearing plans. Rejected: keeping the fabricated id and carrying the master
DID as a second field. That is precisely the two-identity model ADR-0020 §2
exists to collapse, and every downstream reader would have to know which to use.

**Per member vs per planned service.** ADR-0020 §1 is one master per *member*,
and A0 mints on that basis. But nothing in the manifest can express more than
one member today: `PlannedService` carries `topology_mode` and no member count,
and `TopologyEntry.members` has no production writer at all (only tests, and
A2's future `.register()` callers — task.md A2). So in practice A0 mints exactly
one master per `PlannedService`, `index = 0`. The naming scheme carries the
index from day one so that A3's scale-out adds `-1` rather than renaming
anything.

### D-A0-8 — Test strategy

Covered in full in §5. The design commitment here is what the four named matrix
rows do and do not get:

| Row | A0's evidence |
|---|---|
| 1 — certificate expired ⇒ handshake fails closed | Already true ([delegation.rs:110-116](../../../../crates/identity/src/delegation.rs#L110)); A0 pins that a *service-instance* certificate is not special-cased |
| 2 — wrong `scope` rejected at ingress | New. Tested at both granularities: an unknown scope rejected at the handshake, and the narrow single-value comparison unit-tested on `verify` |
| 3 — attended posture, cadence missed | **Not a distinct code behavior** — the failure mode *is* row 1. Evidence is row 1's test plus the near-expiry warning test plus the operator note. Saying so beats inventing a test that restates row 1 |
| 14 — supervisor holds master keys and is compromised | **Split.** The A0-testable half is the bound: an instance certificate is revocable without touching the master, and short-lived. The other half ("blast radius bounded to the members it manages") needs a supervisor and is A5's |

### D-A0-9 — Migration: pre-A0 services

**Resolved: nothing regresses, and the reasons are structural rather than a
compatibility shim** (which the project would not accept pre-release anyway).

1. **The "no delegation ⇒ own master" fallback is untouched**
   ([handshake.rs:87-91](../../../../crates/router/src/handshake.rs#L87)). A
   pre-A0 service has no certificate to present, so D-A0-4's guest arm presents
   nothing exactly as today, and the destination derives `master_did =
   temporary_did` — or, for a guest-origin call, no identity at all, unchanged.
2. **`verify`'s new argument only bites on a *presented* certificate.** The
   no-delegation branch never calls `verify`. Every certificate that exists in
   the tree today carries `"routing"`, which is in `TRANSPORT_SCOPES`.
3. **Deploy without `instance-certificate` is unchanged.** The field is
   `option<>`; `None` skips every check in D-A0-4.
4. **`ServiceId` values change only for a plan run with `--mint-masters`.** A
   plan compiled and deployed without it produces the same fabricated ids as
   today, so `test_compile_deterministic_service_ids`
   ([compiler.rs:422](../../../../crates/app_orchestration/src/compiler.rs#L422))
   stays green unmodified.

**What is *not* safe, and task.md already says so:** a pre-A0 service that
already holds data and is later redeployed *with* a master gets a new
authorization identity and is orphaned from its own rows (ADR-0020 Context
problem 2). A0 would like to detect that and refuse, but **it cannot**: the
substrate has no logical-ref → service-id index to notice that "this logical
service was previously deployed under a different DID" — that index is A2's.
So A0 ships the mint-time warning and the task.md migration note, and the
detection is a backlog row (§7).

### D-A0-10 — `SqliteEndpointStorage`'s schema gate (new, from review)

The instance-certificate table is a third table in a store whose schema is
created **only when `PRAGMA user_version == 0`**, after which the ctor sets it
to `1` ([registry_store.rs:53-81](../../../../crates/data_db/src/registry_store.rs#L53)).
B7a added `service_owners` inside that same block, which was safe because no
database had ever reached version 1. A0 does not have that luxury: **every dev
and test database created since B7a is already at version 1**, so a table added
to the `version == 0` block would never be created there, and every certificate
write would fail at runtime with `no such table` on exactly the databases
developers already have.

**Resolved: drop the version gate and run the `CREATE TABLE IF NOT EXISTS`
statements unconditionally on open.** They are already idempotent, the gate
buys nothing the `IF NOT EXISTS` does not, and this makes every future
in-place schema addition correct by default — which is the standing pre-release
position (change schema in place, no compat shims, no version ladders; most
recently [ADR-0019](../../../decisions/0019-deploy-time-artifact-delivery.md)
§3). `PRAGMA user_version` stays written for information, or is dropped; it has
no reader.

Rejected: a `version < 2` block bumping to `2`. It works, but it is the first
rung of exactly the version ladder this project has said it does not want
pre-release, and it leaves the next person adding a table to guess which
convention applies.

The table mirrors `service_owners`
([registry_store.rs:73-78](../../../../crates/data_db/src/registry_store.rs#L73)):

```sql
CREATE TABLE IF NOT EXISTS service_instance_certs (
    service_id  TEXT PRIMARY KEY,
    certificate TEXT NOT NULL,   -- DelegationCertificate::to_json
    created_at  INTEGER NOT NULL
);
```

with `save_cert` an upsert (a renewal replaces in place, matching
`save_owner`'s `ON CONFLICT` at
[registry_store.rs:205](../../../../crates/data_db/src/registry_store.rs#L205)),
and tests mirroring the three `service_owners` tests at
[registry_store.rs:371-395](../../../../crates/data_db/src/registry_store.rs#L371) —
plus one the owners tests do not have and this needs:
`an_existing_database_gains_the_certificate_table_on_open`, which opens a store,
closes it, reopens it, and writes a certificate. That is the regression this
decision exists to prevent, and it fails against the version-gated shape.

Rejected: `hosted_apps_dir/<service_id>.json`, where the *registry* certificate
goes. The router reads the instance certificate on every outbound hop, and a
filesystem read per hop is not acceptable. `owner_of` is already the
in-memory-with-persistence pattern for a per-service fact, and `ProxyRouter`
already holds an `EndpointRegistry`
([proxy.rs:150](../../../../crates/router/src/proxy.rs#L150)).

**Present: a new arm in `ProxyRouter::invoke_remote_at`.**

Today's identity match
([proxy.rs:361-374](../../../../crates/router/src/proxy.rs#L361)):

| Origin / proof | Presents |
|---|---|
| `(Some(proof), Native)` | the caller's forwarded proof |
| `(None, Native)` | **the node's own key** |
| `(_, Guest { .. })` | **nothing** — anonymous at the destination |

A0 changes only the third row, and it does not weaken the reasoning behind it.
The comment at [proxy.rs:340-360](../../../../crates/router/src/proxy.rs#L340)
refuses to let a guest present *the caller's proof or the node's key*, because
the guest chooses `(interface, method, params)` freely and would be laundering
them under a privileged identity. Presenting the **service's own** instance key
grants the guest no privilege it did not already have as itself:

```rust
(_, CallOrigin::Guest { service_id }) => {
    // The service's own certified instance key -- never the node's, never
    // an inbound caller's proof. The guest still chooses the call; the
    // identity it travels under is the one this substrate derived for that
    // service and the member master certified, which is exactly the
    // authority the guest already has locally.
    if let Some(cert) = self.registry.instance_cert(service_id) {
        let owner = self.registry.owner_of(service_id);   // recorded at deploy
        let instance = self.node_identity.derive_service_identity(&owner?, service_id);
        preamble.pubkey = Some(hex::encode(instance.public_key().to_bytes()));
        preamble.delegation = Some(cert);
    }
}
```

Every input is already on `ProxyRouter`: `registry`, `node_identity`
([proxy.rs:150-156](../../../../crates/router/src/proxy.rs#L150)).

**This is a real behavior change on a path that previously always failed.** A
guest-originated remote call currently arrives anonymous and is rejected by the
native-dispatch arm. After A0, a guest of a service holding an instance
certificate arrives as its **member master**. That is the point of the slice —
and it needs its own test rather than riding on a happy-path one (§5).

For a service with **no** certificate the arm is byte-identical to today
(present nothing), which is D-A0-9's migration guarantee.

**Deliberately out of A0:** the `(None, Native)` arm, which presents the node
identity for substrate-internal calls (the FDAE relationship-proof fetch).
Switching it to the service instance identity changes what
`expected_asserter_did` resolves to mid-flight, and that belongs with A2's
binding work. Backlog row in §7.

---

## 2. Phase plan

| Phase | Content | Gate |
|---|---|---|
| 1 | Scope constants, `verify`'s required-scope argument, and **all ten call sites** across three crates (§3.1) — `syneroym-identity` (7 unit tests + the bench), `syneroym-core` (`dht_registry.rs:139`), `syneroym-router` (`handshake.rs:66`) | `cargo test -p syneroym-identity -p syneroym-core -p syneroym-router` green. **Not splittable by crate**: `verify` is an inherent method with no default, so the signature change breaks every caller in the same commit — a `-p syneroym-identity` gate would pass on a workspace that does not build |
| 2 | The new scope tests (§5.1, §5.2's handshake rows), the tautology doc comment, and the stale-comment fixes in `preamble.rs`/`io.rs` (§6 items 8-9) | `cargo test -p syneroym-router` green |
| 3 | `syneroym-core` **and `syneroym-data-db`**: instance-certificate store on `EndpointRegistry`/`EndpointStorage`, all four implementors, and the schema-gate change (§3.3, D-A0-10) | `cargo test -p syneroym-core -p syneroym-data-db` green. **Both crates, deliberately** — `-p syneroym-core` alone cannot see the production backend break |
| 4 | WIT + `syneroym-control-plane`: `instance-identity`, `deploy-manifest.instance-certificate`, install-time verification, `undeploy` cleanup, `deployed-service` expiry field (§3.4) | `cargo test -p syneroym-control-plane` green; `wasm32-wasip2` builds |
| 5 | `syneroym-router`: `ProxyRouter`'s guest-origin identity presentation (§3.5) | integration tests in §5.3 |
| 6 | `roymctl`: master naming, `identity certify-instance`, `svc deploy --master`, `app deploy --mint-masters`, plan service-id substitution (§3.6) | `cargo test --workspace` green |
| 7 | Heartbeat expiry sweep + `svc list` column (§3.7) | `cargo test --workspace` green |
| 8 | E2E (§5.4) and docs: ADR-0020 amendment, `task.md`, backlog, traceability (§8) | `mise run test:all` green |

Phases 1-3 are independently mergeable and land no behavior change beyond the
two scope checks — the ingress allowlist, and the DHT endpoint-record
tightening, which is unreachable today (§3.1). Phase 5 is where the behavior
change bites; it must not land before phase 4 stores anything for it to read.

---

## 3. Exact changes

### 3.1 `crates/identity` — scope constants and `verify`

`src/delegation.rs`: add the three constants (D-A0-1) above
`DelegationCertificate`; change

```rust
pub fn verify(&self, expected_master_did: &str, accepted_scopes: &[&str]) -> Result<()>
```

with the scope check placed **after** the master-DID match and **before** the
validity-window checks — a scope mismatch is a categorical rejection and should
not be masked by an expiry error on a certificate that was never admissible.
Error text names both sides: `"certificate scope '{}' is not accepted here
(accepted: {:?})"`.

Update the doc comment on `scope`
([delegation.rs:19](../../../../crates/identity/src/delegation.rs#L19)) from
`// e.g., "routing"` to a real statement: what the field means, that it is
signed, and that `verify` now enforces it against a caller-supplied set.

**Call-site inventory — ten, not the thirteen revision 1 claimed.** That count
was wrong in both directions: it invented two sites that only call `issue`, and
it missed a production site that will fail to compile.

| Site | Kind | New argument |
|---|---|---|
| [handshake.rs:66](../../../../crates/router/src/handshake.rs#L66) | **production** — ingress | `&TRANSPORT_SCOPES` (§3.2) |
| [dht_registry.rs:139](../../../../crates/core/src/dht_registry.rs#L139) | **production** — DHT endpoint record | `&[SCOPE_SERVICE_INSTANCE]`, see below |
| [benches/delegation.rs:29](../../../../crates/identity/benches/delegation.rs#L29) | bench | `&TRANSPORT_SCOPES` |
| `delegation.rs` 158, 171, 185, 197, 223, 226, 241 | unit tests (7) | mechanical |

Corrections to revision 1's list: `benches/delegation.rs:14` is an `issue` call,
not a `verify` — the `verify` is at `:29`.
`coordinator_iroh/tests/multi_hop_relay.rs:971` is also an `issue` call, and
that file has **no** `.verify()` call at all (it exercises the handshake
indirectly), so nothing there needs touching. `handshake.rs`'s tests call
`verify_preamble`, not `cert.verify`, so they need *new* tests (§5.2) but no
signature edit.

**The second production site, and why it is not mechanical.**
`SignedEndpointInfo::verify` ([dht_registry.rs:137-146](../../../../crates/core/src/dht_registry.rs#L137))
validates a delegation certificate attached to a **DHT-published endpoint
record** — structurally the same job ADR-0020 §6 assigns to the HTTP registry's
`verify_endpoint_signature`, on the BEP0044 path instead. Neither ADR-0020 §6
nor A1's task.md text mentions it; both cite only `registry.rs:234`. So it is
currently invisible to the slice that is supposed to own it. A0 pins the scope
and flags the rest:

- **A0 passes `&[SCOPE_SERVICE_INSTANCE]`.** Publishing an endpoint record is
  the service-instance role, and it is the requirement A1 will want anyway. The
  change is risk-free today: `EndpointInfo.delegation` is set to `Some(..)`
  **nowhere in the tree** — every construction site in production, tests, and
  fixtures passes `None` — so this branch is unreachable in practice and no
  existing record can be rejected by tightening it.
- **A0 does *not* reconcile its semantics, and says so.** This site checks
  `cert.temporary_did == self.info.service_id` — the record is keyed by the
  **temporary** DID, with the certificate attesting which master delegated it.
  ADR-0020 §6 specifies the **inverse**: keyed by the *master* DID, signed by
  the instance key. A1 has to change both registry paths and decide whether the
  DHT path adopts §6's shape or keeps its own. Recorded in §6 item 6 and §7 so
  A1 inherits it rather than rediscovering it.

### 3.2 `crates/router/src/handshake.rs` — the ingress requirement

[handshake.rs:66](../../../../crates/router/src/handshake.rs#L66) becomes
`cert.verify(master_did, &TRANSPORT_SCOPES)?`.

Add the doc comment explaining that `master_did` is read from the certificate
itself, so the first argument is a no-op on this path by design (D-A0-1), and
that the *scope* argument is what makes this call enforce anything.

No signature change to `verify_preamble`: the required set is a property of the
ingress, not of a caller's choice, and making it a parameter would invite a
future caller to pass something laxer.

### 3.3 `crates/core` — the instance-certificate store

Per D-A0-4's table. Two notes on shape:

- `service_certs` stores the parsed `DelegationCertificate`, not the JSON, so
  the expiry sweep (§3.7) and the `svc list` column read a field rather than
  re-parsing. `syneroym-core` already depends on `syneroym-identity` for
  `EndpointInfo.delegation`
  ([dht_registry.rs:71](../../../../crates/core/src/dht_registry.rs#L71)), so
  no new dependency edge.
- `EndpointStorage` persists the JSON (`to_json`), matching how the owner row
  persists a bare string.
- **All four implementors** gain the methods, `SqliteEndpointStorage`
  included — see D-A0-4's table and D-A0-10 for the schema.

### 3.4 WIT + `crates/control_plane` — query, install, verify, remove

- `control-plane.wit`: `instance-identity` record + function (D-A0-3);
  `deploy-manifest.instance-certificate` (D-A0-4);
  `deployed-service.instance-certificate-expires-at` (D-A0-6).
- `service.rs`: `"instance-identity"` dispatch arm
  ([service.rs:321-382](../../../../crates/control_plane/src/service.rs#L321)),
  gated as `readyz` is.
- `orchestration.rs`: the four-step install verification in `deploy` (D-A0-4),
  placed after the ownership/capability gates
  ([orchestration.rs:467-495](../../../../crates/control_plane/src/service/orchestration.rs#L467))
  and before any artifact work, so a bad certificate costs nothing; the
  `set_instance_cert` write next to `set_owner`
  ([orchestration.rs:776](../../../../crates/control_plane/src/service/orchestration.rs#L776))
  and under the same rollback; `remove_instance_cert` in `undeploy`;
  `instance-certificate-expires-at` populated in `list`.
- `OrchestratorInterface` ([orchestration.rs:36-48](../../../../crates/control_plane/src/service/orchestration.rs#L36)) —
  the trait's real name, implemented only by `ControlPlaneService` — gains
  `instance_identity(service_id, caller)`.

### 3.5 `crates/router/src/proxy.rs` — presentation

The `CallOrigin::Guest` arm per D-A0-4. `owner_of` returning `None` (a service
deployed before B7a) means the derivation input is unknown, so present nothing
— the same conservative default the arm has today, not a guess.

### 3.6 `apps/roymctl` — the three surfaces

- `commands/identity.rs`: `CertifyInstance` variant + handler; fix the
  planning-doc reference at
  [identity.rs:47-48](../../../../apps/roymctl/src/commands/identity.rs#L47)
  while editing the file (AGENTS.md).
- `commands/svc.rs`: `--master` on `Deploy`; the expiry column on `List`.
- `commands/app.rs`: `--mint-masters` on `Deploy`; the substitution function
  between [app.rs:90](../../../../apps/roymctl/src/commands/app.rs#L90) and
  [:107](../../../../apps/roymctl/src/commands/app.rs#L107).
- A shared `member_master_name(&LogicalServiceRef, index) -> String` +
  resolve-or-mint helper, used by both deploy paths.
- `crates/sdk`: `deploy_svc_wasm`/`deploy_svc_tcp`/`deploy_plan` carry the new
  optional field through. `mapper.rs` maps it if the plan carries one — the
  mapper *translates* a certificate, it never mints one (D-A0-2).

### 3.7 Renewal visibility

- `crates/substrate/src/runtime.rs`: the expiry sweep inside the existing
  heartbeat loop ([runtime.rs:617-680](../../../../crates/substrate/src/runtime.rs#L617)).
  It needs the `EndpointRegistry`, which the spawned task does not currently
  hold — pass it in, or (preferred) put the sweep in `RuntimeServices`'s own
  `tokio::select!` as a sibling interval rather than growing
  `publish_to_community_registry`'s argument list, which is already
  `#[allow(clippy::too_many_arguments)]`
  ([runtime.rs:603](../../../../crates/substrate/src/runtime.rs#L603)).

---

## 4. Ordering, semantics, and interaction notes

- **The scope check is fail-closed on an unknown scope, by construction.** A
  certificate whose scope is not in the accepted set is rejected before its
  window is examined, so an attacker cannot learn "the scope was wrong" versus
  "the certificate was expired" from timing — both are one error type.
- **A0 changes what `caller_did` is for a service-originated call.** Downstream,
  FDAE's `subject_did`/`caller_did` (`io.rs:169`, `:257`) become the member
  master rather than nothing-at-all. ADR-0020 §1's "no change to FDAE" is
  correct as written — FDAE already reads `master_did` — but the *value* it
  reads on this path changes from absent to present, which is a behavior change
  in policy evaluation for any policy that names a service as a principal.
- **Revocation now has a real subject.** `resolve_master_anchor`'s
  `revoked_keys` ([handshake.rs:80](../../../../crates/router/src/handshake.rs#L80))
  can kill a retired instance key without touching the member's identity. A0
  adds no new revocation mechanism; it adds the first thing worth revoking.
- **`enc=`/E2E handshake is orthogonal** and untouched: the ECDH pubkey and the
  identity pubkey are different preamble fields
  ([preamble.rs:36-40](../../../../crates/router/src/preamble.rs#L36)).
- **Preamble size.** `delegation=` is hex-encoded JSON and already counted in
  `MAX_PREAMBLE_LINE_BYTES`'s budget
  ([io.rs:35-40](../../../../crates/router/src/route_handler/io.rs#L35)). A
  service-instance certificate is the same size as a routing one; no bound
  changes.
- **The `TODO(M2/M3A)` at [compiler.rs:163](../../../../crates/app_orchestration/src/compiler.rs#L163)
  survives A0**, because the fabricated id remains the graph key and the
  no-master path still uses it end to end. Its backlog row needs the
  substitution described or it reads as though nothing moved (§7).

---

## 5. Tests

### 5.1 Unit — `crates/identity/src/delegation.rs`

| Test | Asserts |
|---|---|
| `a_certificate_verifies_against_an_accepted_scope` | `service-instance` cert + `[SCOPE_SERVICE_INSTANCE]` ⇒ `Ok` |
| `a_certificate_is_rejected_when_its_scope_is_not_accepted` | `routing` cert + `[SCOPE_SERVICE_INSTANCE]` ⇒ `Err`, message names both scopes |
| `an_unknown_scope_is_rejected_even_with_a_valid_signature` | scope `"vault-unseal"` against `TRANSPORT_SCOPES` ⇒ `Err` |
| `any_listed_scope_is_admitted` | both transport scopes pass `TRANSPORT_SCOPES` |
| `the_scope_cannot_be_edited_after_issue` | mutate `cert.scope` post-issue ⇒ signature failure, not a scope failure — pins that the new check is not bypassable by rewriting the field |
| `a_scope_mismatch_is_reported_before_expiry` | expired **and** wrong-scope ⇒ the scope error |

### 5.2 Unit — router, core, control-plane

`crates/router/src/handshake.rs`:

| Test | Asserts |
|---|---|
| `a_routing_scoped_certificate_is_accepted_on_a_connection` | unchanged behavior for every certificate that exists today |
| `a_service_instance_scoped_certificate_is_accepted_on_a_connection` | the new scope reaches `VerifiedIdentity` with the member master |
| `a_certificate_scoped_outside_transport_is_rejected_at_the_handshake` | **matrix row 2** |
| `an_expired_instance_certificate_fails_the_handshake_closed` | **matrix row 1**, on a service-instance certificate specifically |

`crates/core/src/local_registry.rs`: `an_instance_certificate_round_trips_through_storage`;
`removing_a_service_forgets_its_instance_certificate`.

`crates/data_db/src/registry_store.rs` — the production backend, mirroring the
three existing `service_owners` tests plus the one D-A0-10 exists for:

| Test | Asserts |
|---|---|
| `a_fresh_db_gets_the_certificate_table` | parallel to `test_fresh_db_gets_service_owners_table` |
| `saving_a_certificate_upserts` | a renewal replaces in place |
| `removing_a_service_removes_its_certificate` | teardown |
| `an_existing_database_gains_the_certificate_table_on_open` | **D-A0-10.** Open, drop, reopen, then write — fails against the version-gated schema, which is the regression the decision exists to prevent |

`crates/control_plane`:

| Test | Asserts |
|---|---|
| `the_derived_instance_identity_is_stable_across_calls` | determinism — the property the pre-deploy query depends on |
| `two_owners_get_different_instance_identities_for_the_same_service_id` | the `owner_did` half of the derivation is live (keys.rs:223-232's stated reason) |
| `a_deploy_is_rejected_when_the_certificate_certifies_a_different_key` | D-A0-4 check 3 |
| `a_deploy_is_rejected_when_the_certificates_master_is_not_the_service_id` | D-A0-4 check 2 |
| `a_deploy_is_rejected_when_the_certificate_carries_the_routing_scope` | D-A0-4 check 4 — **matrix row 2 at the install site** |
| `a_deploy_without_a_certificate_still_succeeds_and_stores_none` | **D-A0-9**, the migration guarantee |
| `undeploy_removes_the_instance_certificate_with_the_owner_row` | no stale credential after teardown |

### 5.3 Integration — `crates/router/tests`

| Test | Asserts |
|---|---|
| `a_guest_call_travels_under_its_services_member_master_not_the_node_identity` | **the slice's core claim.** Service with an installed certificate makes a guest-origin remote call; the destination's `verify_preamble` yields the member master and `build_caller` puts it in `caller_did` |
| `a_guest_call_from_a_service_without_a_certificate_is_still_anonymous` | the unchanged path — pins that A0 did not quietly start presenting the node key |
| `a_revoked_instance_key_is_rejected_while_the_member_master_still_certifies_a_new_one` | **matrix row 14's testable half.** Anchor revokes instance key 1; the same master certifies instance key 2 and that connection verifies |
| `a_second_instance_under_the_same_master_presents_the_same_authorization_identity` | reinstantiation preserves `caller_did` — the reference scenario's step 4, at unit scale |

### 5.4 Substrate — expiry visibility and end to end

- `crates/substrate`: `a_certificate_near_expiry_is_warned_about_on_the_heartbeat_sweep`
  (**matrix row 3**'s observability half) and
  `svc_list_reports_the_installed_certificates_expiry`.
- **E2E:** extend `crates/substrate/tests/federated_fdae_e2e.rs`'s two-real-node
  harness so Node B's caller presents a service-instance certificate and Node
  A's FDAE policy names the **member master** as principal; then reinstantiate
  Node B's service under a fresh instance key from the same master and assert
  the same rows are still reachable. That is the milestone's actual claim and it
  is testable in A0 without any supervisor.

  **Cost flagged up front:** that fixture is large and deploys its data-owning
  node as a **TCP** service (already recorded in `deferred-backlog.md` §3, "No
  genuinely cross-node proof of `resolve-relation`'s stage-4 deny"). If the
  extension turns out to need a WASM-typed node, it lands as a scoped-down
  variant and gets its own backlog row rather than expanding to fit.

### 5.5 CLI — `apps/roymctl/tests/cli_args.rs`

Parse coverage for `identity certify-instance`, `svc deploy --master`, and
`app deploy --mint-masters`, plus a unit test that
`member_master_name` rejects an `AppInstanceId` containing a path separator
(D-A0-2).

---

## 6. Things in the ADRs / `task.md` that are stale or under-specified

1. **ADR-0020 §1 and §5 call the ingress verifier `verify_identity`.** No such
   function exists. It is `HandshakeVerifier::verify_preamble`
   ([handshake.rs:43](../../../../crates/router/src/handshake.rs#L43)).
2. **ADR-0020 §5 cites `handshake.rs:145` as "the existing scope string in
   use".** That line is inside `test_handshake_success_delegated`. `"routing"`
   appears in **no production code path** — the only production issuer is
   `roymctl identity delegate`
   ([identity.rs:149](../../../../apps/roymctl/src/commands/identity.rs#L149)),
   which takes `--scope` as an unvalidated free string. `"routing"` is a
   convention in tests, not a constant. A0 makes it one.
3. **ADR-0020 §1: "the instance presents that certificate on its route preamble
   the same way a delegated client does today", citing `native.rs:52-53`.**
   That citation describes `CallerProof.delegation_json`
   ([native.rs:47-54](../../../../crates/rpc/src/native.rs#L47)) — the
   *forwarding* of an inbound caller's proof across a hop. **No service presents
   its own identity on an outbound call today**: `CallOrigin::Guest` presents
   nothing ([proxy.rs:373](../../../../crates/router/src/proxy.rs#L373)) and
   `CallOrigin::Native` with no proof presents the *node's* key
   ([proxy.rs:371](../../../../crates/router/src/proxy.rs#L371)). A0 builds this
   arm; the ADR reads as though it exists. **This is the single largest gap
   between the design of record and the tree.**
4. **ADR-0020 §2 calls the `ServiceId` change "a definition, not a new field".**
   True of the type, false of the values.
   `derive_deterministic_service_id` fabricates a DID for which no private key
   exists and whose bytes "may not be a valid Curve25519 point"
   ([compiler.rs:161-180](../../../../crates/app_orchestration/src/compiler.rs#L161)) —
   which `resolve_did_key` rejects
   ([substrate.rs:90-96](../../../../crates/identity/src/substrate.rs#L90)), and
   `DelegationCertificate::verify` calls it
   ([delegation.rs:119](../../../../crates/identity/src/delegation.rs#L119)). A
   real value substitution in the deploy path is required (D-A0-7).
5. **ADR-0020 §6's derivation description is one input short of complete.** It
   says instance keys derive from "the hosting node's identity plus `(owner_did,
   service_id)`", which is accurate, but `owner_did` is the **deployer's** DID
   ([orchestration.rs:776](../../../../crates/control_plane/src/service/orchestration.rs#L776)),
   not a property of the service. So relocating a member *and* redeploying it
   from a different operator identity changes the instance key for two reasons,
   and a substrate cannot re-derive a past instance key without knowing which
   owner deployed it. Harmless for A0; it matters when A5 reasons about
   revoking a key it did not issue.
6. **ADR-0020 §6 and A1's task.md text name only one endpoint-record
   verification path; there are two.** Both cite
   `community_registry/src/registry.rs:234`. But `SignedEndpointInfo::verify`
   ([dht_registry.rs:137-146](../../../../crates/core/src/dht_registry.rs#L137))
   validates a delegation certificate on a **DHT/pkarr-published** record, doing
   structurally the same job on the BEP0044 path — and it is the second
   production caller of `DelegationCertificate::verify` (§3.1). Worse than an
   omission: its semantics are the *inverse* of §6's. It requires
   `cert.temporary_did == info.service_id`, i.e. the record is keyed by the
   **temporary** DID with the certificate naming its master, whereas §6
   specifies a record keyed by the **master** DID and signed by the instance
   key. A1 must change both paths and decide whether the DHT one adopts §6's
   shape. A0 pins its scope argument and flags the rest (§3.1, §7).
7. **ADR-0020's "this needs no change to FDAE" (§1) holds for the sieve and
    misses a second credential path entirely.** The claim is that the router
    collapses both tiers to `master_did` before authorization sees them, which
    is true of `io.rs:169`/`:257`. But a service's **`RelationshipProof`** is
    signed with its *instance* key — `asserter_did` comes from
    `derive_service_identity(owner_did, service_id)`
    ([synsvc_native.rs:362](../../../../crates/control_plane/src/synsvc_native.rs#L362),
    [:656](../../../../crates/control_plane/src/synsvc_native.rs#L656),
    [relationship_proof.rs:85](../../../../crates/rpc/src/relationship_proof.rs#L85)) —
    and the fetching side requires **exact equality** with the policy-declared
    `expected_asserter_did`
    ([relationship_proof.rs:102-107](../../../../crates/rpc/src/relationship_proof.rs#L102)).
    So a member reinstantiated on another node silently stops satisfying every
    policy that names it: ADR-0020's own failure, on a path it never mentions.
    **Out of A0's scope** (A0 changes no FDAE credential) but **in A2's**, since
    A2 is what publishes `expected_asserter_did` per member — written into A2's
    slice text and task.md matrix row 19 rather than left in the backlog.
8. **`preamble.rs:39` claims the delegation certificate is checked "against
   `preamble.service_id`".** It is not — `verify_preamble` never reads
   `preamble.service_id`
   ([handshake.rs:43-92](../../../../crates/router/src/handshake.rs#L43)).
   Stale doc; correct it while A0 is in the file.
9. **`io.rs:350-353` cites a failure test "delegation cert for a different
   service's DID -> rejected".** No test matching that description exists. The
   real check is `cert.temporary_did != temporary_did`
   ([handshake.rs:68](../../../../crates/router/src/handshake.rs#L68)) — a
   pubkey mismatch, not a service-DID one. Stale comment.
10. **`cert.verify(master_did)` is self-referential on the only production
   path**, making `delegation.rs:87`'s confused-deputy check a tautology there
   (D-A0-1). Not a hole; needs a comment so nobody "fixes" it.
11. **task.md's matrix row 2 over-promises for A0 in isolation.** "Rejected at
   ingress" is true for a scope outside the transport set, but the *narrow*
   single-value comparison the row implies cannot live at `handle_stream`,
   where both transport scopes are legitimate. It lands at A1's endpoint-record
   admission. Row 2's wording should distinguish the two (D-A0-1).
12. **task.md's A0 bullet says masters are minted "through the existing `roymctl
    identity` storage".** The *storage* is reused; the *verb* cannot be —
    `identity delegate` requires `--temp-did`
    ([identity.rs:32-33](../../../../apps/roymctl/src/commands/identity.rs#L32)),
    which the operator does not have until the substrate reports it (D-A0-3).
13. **ADR-0020 §5 nominates `RotationPolicy` as "the natural place" for
    in-place-vs-restart certificate replacement.** A0 does not use it: without
    automatic renewal there is nothing for a policy to choose between (D-A0-6).
    It becomes load-bearing at A5.
14. **`identity.rs:47-48` cites "M04A Slice B7b" in a doc comment**, which
    AGENTS.md's no-planning-doc-references rule forbids. Pre-existing; fixed in
    passing since A0 edits that file.

---

## 7. Deferred-backlog updates (mandatory, per AGENTS.md)

**None of these is "deferred and forgotten."** Two of them are picked up inside
this milestone, and both have been written into the slice that owns them
(task.md A2 and A5) rather than left to be rediscovered from the backlog — a
backlog row targeting a slice whose own text never mentions it is exactly how
work goes missing. One is genuinely optional, with a stated trigger. Each row
below says which it is.

| Action | Row |
|---|---|
| **New**, §3 | "Online-key posture: unattended instance-certificate renewal." A0 ships the attended posture only — `roymctl identity certify-instance` on an operator cadence, plus a near-expiry warning on the heartbeat sweep and an expiry column on `svc list`. Automatic renewal needs a component that holds member master keys and runs unattended, and a substrate-side timer would put the master key on the substrate, which ADR-0020 §3 forbids. Through A0-A4 a missed cadence is an outage (matrix row 3). **Picked up in A5, and now named in A5's slice text** alongside vault custody and `RotationPolicy`'s first real use. Target **M05A A5** |
| **New**, §3 | "A service's signing identity is still its instance key, so `expected_asserter_did` does not survive reinstantiation." Two halves: (a) `CallOrigin::Native` still presents the node's key on the wire (A0 changes only the guest arm); (b) the load-bearing half — `RelationshipProof::sign` stamps `asserter_did` from `derive_service_identity`, and `verify` demands exact equality, so a reinstantiated member breaks every policy naming it. Republishing per restart is ruled out by the reference scenario's step 4. **Picked up in A2, and now named in A2's slice text plus matrix row 19.** Target **M05A A2** |
| **New**, §3 | "No detection of a pre-A0 service adopting a member master." The CLI cannot warn: the substrate has no logical-ref → service-id index to notice the previous DID. **Genuinely optional — nothing later in M05A or M5 depends on it** — but nearly free once A2 builds that index, so the row carries that as its pickup trigger rather than a slice assignment. Target **TBD (trigger-driven)** |
| **Extend**, §3 | "Member master key custody, escrow, and loss" — A0's only mitigation is the mint-time warning, and keys live at `<roymctl --dir>/identities/member-<instance>-<service>-<index>.key` at `0o600`. Note the split: **vault custody is A5's** (ADR-0020 §4, now in A5's text), while escrow, rotation, and re-attribution stay genuinely out of the milestone per task.md's non-goals |
| **Extend**, §11 | `crates/app_orchestration/src/compiler.rs:163` marker row — A0 substitutes real member master DIDs *after* compilation, so the fabricated id survives as the plan's internal graph key and on the no-master path. Stays open past this milestone by design: the supervisor path always mints masters (A5), so the placeholder is only ever reached by a bare `roymctl app deploy` |
| **New**, §3 | "The DHT endpoint-record path has its own delegation check, and A1's design names only the HTTP one." `SignedEndpointInfo::verify` (`core/src/dht_registry.rs`) validates a certificate on a pkarr-published record with the **inverse** keying of ADR-0020 §6 (keyed by the temporary DID, certificate naming the master; §6 wants keyed by master, signed by instance). A0 pins its scope argument to `SCOPE_SERVICE_INSTANCE`; reconciling the two paths is A1's. **Picked up in A1, and now named in A1's slice text.** Target **M05A A1** |
| **New**, §3, **only if** §5.4's e2e extension is scoped down | "No two-real-node proof that a reinstantiated member keeps its authorization identity" |

---

## 8. Completion checklist

- [ ] D-A0-1 … D-A0-10 confirmed against the tree at merge time (line anchors
      re-checked if the branch has advanced)
- [ ] `cargo +nightly fmt --all`
- [ ] `cargo clippy --workspace --all-targets --all-features` clean
- [ ] `cargo test --workspace` green
- [ ] `mise run test:e2e` green
- [ ] `wasm32-wasip2` compilation (the WIT changes in §3.4 cross the guest
      boundary)
- [ ] ADR-0020 amendment, dated, covering every §6 item that lands on the ADR:
      the `verify_identity` → `verify_preamble` naming fix (item 1), §5's stale
      scope citation (item 2), §1's presentation-path correction (item 3),
      §2's value-substitution correction (item 4), §6's incomplete derivation
      inputs (item 5), §6's second endpoint-record path (item 6), and §1's
      "no change to FDAE" versus `RelationshipProof` (item 7)
- [ ] `task.md`: A0 marked complete; matrix rows 1/2/3/14 pointed at their
      evidence, with row 2's two granularities (§6 item 11) and row 14's split
      stated; the A0 bullet's "existing `roymctl identity` storage" claim
      corrected per §6 item 12
- [ ] `status.md`: A0 row flipped, with the verification evidence
- [ ] `deferred-backlog.md` per §7
- [ ] `traceability-matrix.md` `[FND-IDT]` row extended (**not** flipped to
      Complete — A1 and A2 are still outstanding under the same requirement)
- [ ] Stale comments in §6 items 8, 9, 14 corrected in the files A0 touches
- [ ] Import cleanup pass over every edited file (AGENTS.md)
