# D-04-01: UCAN Capability & Verification Model

**Status**: Accepted (amended 2026-07-16 — see "Amendments" below)

**Context**:

Milestone 4 introduces access control ([FND-IAM]). Requests arriving at the
substrate must be resolved into a verified, normalized set of **capabilities**
so that both coarse admission (may this caller invoke this method?) and the
FDAE data-plane sieve (which rows/columns may they see?) can evaluate against a
single, trustworthy context. `system-requirements-spec.md:976-978` specifies a
UCAN-based model: "the gateway mathematically verifies the UCAN chain… extracts
the proven claims, capabilities, and scopes," which the SQL compiler later
binds as parameters.

Today the codebase has ed25519 `did:key` identities (`crates/identity`), a
canonical-JSON + Ed25519 `DelegationCertificate` (master→temporary key, coarse
`scope: String` such as `"routing"`, [ADR-0001](0001-delegation-certificate-format.md)),
and DHT-based revocation checking in the handshake. It has **no** capability
token, no ability vocabulary, and no normalized session context. This ADR
decides the capability model, the token/verification mechanism, and the
normalized output. It is the prerequisite for M04A Slices B0 (Admin-capability
gate) and B1 (UCAN context extraction), and is **co-designed with**
[D-04-05](0016-native-dispatch-identity-threading.md) — the `SessionContext`
defined here is what `CallerContext` there carries — and with D-04-02 (the FDAE
compiler consumes these capabilities as bound SQL parameters).

**Decision**:

Adopt the **UCAN semantic model** — capabilities delegated along a signed
issuer→audience chain — implemented over the existing ed25519/`did:key`
primitives, in a new `crates/ucan` crate (`syneroym-ucan`) that depends on
`syneroym-identity`. We do **not** pull in the `rs-ucan`/JWT stack for M4 (see
Alternatives).

1. **Capability shape.** A capability is
   `Capability { with: ResourceUri, can: Ability, caveats: Option<Value> }`.
   - `ResourceUri` uses the same namespace the host already injects as the
     spoof-proof caller identity (`system-architecture.md:1830`):
     `synapp:<app_instance_id>:svc:<service_id>` for a service resource, and
     `substrate:<node_did>` for node-scoped authority.
   - `Ability` is a `/`-delimited hierarchy string. Initial vocabulary:
     `data-layer/read`, `data-layer/write`, `data-layer/admin` (gates
     `execute-ddl` **and** `query-raw`), `vault/reveal`, `messaging/publish`,
     `messaging/subscribe`, `blob/read`, `blob/write`, `blob/sign-url`,
     `app-config/read`, and `substrate/admin` (node-owner root). A parent
     ability entails its children (`data-layer/admin` ⊇ `data-layer/write` ⊇
     `data-layer/read`; `substrate/admin` ⊇ everything on that node).
   - **The "Admin UCAN capability"** referenced across M4 is concretely
     `data-layer/admin` on the target service resource (privileged
     DDL/raw-SQL), or `substrate/admin` for node-owner operations. This is the
     typed value that replaces the `is_init_context` scaffold.

2. **Token & verification.** Define a `CapabilityToken` — issuer DID, audience
   DID, granted `capabilities`, `proofs` (parent tokens forming the chain),
   `not_before`/`expires_at`, and an Ed25519 signature over the RFC-8785
   canonical-JSON body — **the same serialization/signing discipline as
   `DelegationCertificate`** ([ADR-0001](0001-delegation-certificate-format.md)),
   reusing `syneroym-identity` for keys and signatures. Chain verification:
   - each token's signature verifies against its issuer DID;
   - audience-of-parent == issuer-of-child (delegation continuity);
   - each child capability is **attenuated** — a subset of, or entailed by, a
     parent capability (no privilege escalation);
   - time bounds hold across the whole chain;
   - the root issuer is an authority the verifier trusts for that resource: the
     resource owner's DID for service capabilities, or the configured
     `[iam].admin_ucan_root` DID for `substrate/admin`;
   - revocation reuses the existing DHT-revocation check already used for
     delegation certs in `crates/router/src/handshake.rs`.

3. **Normalized output — `SessionContext`.** Verification yields
   `SessionContext { subject_did: String, capabilities: Vec<Capability>, claims:
   Map<String, Value>, verified_at_secs: u64 }`. This is the *verified,
   in-memory* result; it is never deserialized-and-trusted from the wire (peers
   send tokens, receivers verify them — see D-04-05). It is what B1 produces and
   what D-04-05's `CallerContext` embeds.

4. **Relationship to `DelegationCertificate` (complementary, not replacement).**
   The delegation cert remains the **transport-identity** proof (this temporary
   key may act for master DID X at the network layer); UCAN capability tokens
   are the **authorization** layer (what DID X may do to a resource). The
   handshake verifies the former; B1 verifies the latter. The cert's coarse
   `scope` string is not extended — fine-grained authority now lives in
   capabilities.

5. **DIDs stay `String`** (`did:key:z6Mk…`), consistent with `crates/identity`
   and `DelegationCertificate`. External-auth normalization (OIDC/WebAuthn →
   internal DID, `system-requirements-spec.md:977`) is defined as a trait seam
   but only the `did:key` normalizer ships in M4.

**Consequences**:

- **Enables**: a single verified `SessionContext` for coarse admission (D-04-05
  Tier 1/2) and the FDAE sieve (D-04-02 Tier 3); a concrete Admin capability
  replacing `is_init_context`; attenuated delegation without privilege
  escalation; reuse of existing ed25519/canonical-JSON crypto and DHT
  revocation.
- **Defers**: `rs-ucan`/JWT/IPLD wire-format **interop** (M8 federation) — the
  semantic model is kept UCAN-1.0-compatible so an interop encoder can be added
  without changing the internal model; OIDC/WebAuthn external normalizers (M6+
  consumer surfaces); capability caveats beyond a passthrough `Value` (FDAE
  policy in D-04-02 is where rich constraints live).

**Implementation Notes**:

- New crate `crates/ucan` → package `syneroym-ucan`, depending on
  `syneroym-identity`. Types: `ResourceUri`, `Ability`, `Capability`,
  `CapabilityToken`, `SessionContext`; functions `issue`, `verify_chain`,
  `SessionContext::from_verified_chain`.
- Ability-entailment and attenuation are pure functions with exhaustive unit
  tests (escalation attempts must fail closed).
- `[iam].admin_ucan_root` is added to `SubstrateConfig` (M04A migration).
- Verification is host-side only (router/gateway); no `wasm32-wasip2` build of
  `syneroym-ucan` is required. `SessionContext` fields that cross the WIT
  boundary to a guest (for stage-4 ABAC, D-04-02) are expressed as WIT records
  there, not by exporting this crate to WASM.

**Alternatives considered**:

- **`rs-ucan` / JWT-encoded UCANs now.** Rejected for M4: the UCAN spec/tooling
  has churned (0.10 JWT vs 1.0 IPLD/CID), and M4 issues/verifies entirely within
  the Syneroym trust fabric (no external interop until M8). Adopting the semantic
  model over our existing signing avoids a churning dependency while keeping the
  door open to an interop encoder later.
- **Extending `DelegationCertificate.scope` into a capability string** instead of
  a token chain. Rejected: no attenuated delegation, no proof chain, and it
  overloads the transport-identity artifact with authorization semantics.

---

## Amendments

**2026-07-16 (grant-layer synthesis).** Everything in the original decision
stands and is shipped (`crates/ucan`, Slices B0/B1). This amendment **adds** the
grant-layer half of
[`docs/planning/access-control-design.md`](../planning/access-control-design.md),
which reconciled this ADR with the FDAE policy material — each was silent
exactly where the other spoke. Amended in place rather than superseded, per the
convention of ADR-0007/ADR-0011. Nothing below is implemented yet; **Slice B7 is
the first consumer** (items 1, 4, 6, 7).

### A1. `ResourceUri` gains an optional selector path

The original `with` is service-granularity
(`synapp:<app>:svc:<svc>`), so "may call `createOrder` but not `deleteOrder`" is
inexpressible. Extend to `synapp:<app>:svc:<svc>[/<selector>]` (and likewise for
`substrate:<node_did>`), where the selector is interface-shaped:
`collection/<name>[/<id>]`, `blob/<prefix>`, `topic/<pattern>`,
`rpc/<method>`, `orchestrator`'s `app/<name>`.

Matching is segment-wise prefix covering; a trailing `/` or `*` is a prefix
wildcard. **`with` stays the *what*, `can` the *verb*** — keeping selectors out
of `can` keeps entailment a pure string-hierarchy question. `Capability::covers`
extends from `self.with == other.with` to a prefix cover; the
`is_substrate_scope` wildcard rule is unchanged.

### A2. Two ability namespaces: a value and a reference

`can` accepts exactly two kinds of thing, and the difference is semantic:

- **Platform abilities — a *value*.** The existing closed, host-defined
  vocabulary (§1 above) with fixed entailment. Self-describing and immutable.
  `Ability::tier`'s `data-layer` hierarchy and flat-by-default rule stand
  unchanged.
- **App permissions — a *reference*.** `app/<type>.<permission>`, resolved
  against the target service's FDAE policy document (ADR-0017), which declares
  both the operations it covers and the rows it reaches.

Failing closed: an app permission never entails a platform ability; `can: app/X`
where the policy does not define `X` — or where the service has no policy — is
**denied**, never ignored.

**Late binding is deliberate.** An app permission's meaning is owner-mutable, so
a delegator hands over a reference, not a value. Pinning a policy version would
be worse: policy *tightening* would then never reach outstanding grants, and a
hole could never be closed. The delegator's defense is a `where` caveat (A3),
which conjoins regardless of what the policy later says.

### A3. Caveats become a closed, evaluated set

The original decision deferred caveats entirely — a passthrough `Value`, unread
by `grants`/`covers`. That is now the blocking gap: without evaluated caveats the
grant layer cannot attenuate meaningfully. The governing line:

> A caveat may constrain anything the **issuer knows at issue time** or the
> **verifier can see in the request itself**. It may **never require a data
> lookup** to evaluate.

A data-dependent caveat would destroy offline verifiability, make a token mean
something different as data moves under it, make attenuation undecidable, and
duplicate in a second engine what the policy already expresses. Three forms:

| Caveat | Meaning | Composition along the chain |
|---|---|---|
| `where` | An ADR-0007 MongoDB-style JSON filter document — reuses `crates/data_db/src/filter.rs`, not a new language. **Data-layer only** for now. | Conjunction (`AND`) |
| `fields` | `{allow: [...]}` / `{deny: [...]}` — CLS. | Allows intersect, denies union |
| `can_delegate` | Bool. Absent ⇒ `true` (today's behavior). | Logical AND; once false, terminal |

### A4. Attenuation: check the shape, stack the constraints

`with` (prefix cover) and `can` (entailment) are **checked**. Caveats are **not
checked at all** — the verifier **conjoins** every caveat along the chain.

Proving one arbitrary filter is a subset of another is intractable; conjoining is
monotonically narrowing *by construction*, so a child cannot escalate and there
is nothing to verify. (Parent `{dept: 5}` + child `{}` → `dept=5`; + child
`{dept: {$in: [5,6]}}` → `dept=5`. The attempted widening is inert.) This is why
the caveat language must stay closed-form: conjunction is sound only because
every form is intersective.

Across *independent* grants, capabilities **unite** — more grants, more access.
A `where` caveat therefore binds only within its own chain; narrowing someone
means revoking the broader grant, not adding a narrower one.

### A5. `SessionContext` surfaces `anchor_did` and `path`

`from_verified_chain` discards the chain after `verify_chain`, keeping only
`subject_did` (the immediate caller). The chain **is** the provenance record:
the **anchor** is the audience of the first non-root token (the original
principal), the **origin** is the leaf's issuer (the previous service in the
call chain), and the **path** is the chain itself. Surface them; let policies
bind either `caller` or `anchor` as a terminal. This is confused-deputy
prevention at the cost of not discarding data already verified.

### A6. `is_trusted_root` becomes resource-scoped

Today's `|iss, _res| iss == admin_root` makes one node-wide admin DID the only
party that can root a chain or assert trusted facts — so a **service owner
cannot grant `data-layer/read` on their own service**. It must become:

```
is_trusted_root(issuer, resource) = issuer == admin_root         // node-wide
                                 || issuer == owner_of(resource) // per-service
```

This is what lets a service owner attest claims about *their own* users (an
external identity binding: `{iss: service_did, sub: did:key:X, email: …}` — a
**one-way attestation**; the user co-signing would not make it more true, and
mutual signing is a *consent* mechanism belonging with `[PRD-SAF]`) while being
unable to assert anything about another service's resources.

**`owner_of(resource)` does not exist yet — it is Slice B7's catalog owner
field.** This is precisely what `session.rs`'s existing `TODO(B7)` guards: its
synthetic `ResourceUri::substrate(leaf.issuer_did)` probe asks whether an issuer
is a root for a made-up resource named after itself, which under a
resource-scoped predicate reads the wrong scope and could wrongly trust facts.

Claims stay claims: external identifiers (email, username, OAuth `sub`) are
**never** subjects — signatures, attenuation, and revocation all need a key.
Note also that policy must join on **DID, not email**: emails get reassigned and
carry normalization traps, and inheriting those into authorization yields silent
misattribution.

### A7. Revocation checks the whole chain

The DHT revocation check (§2 above) must run against **every token in the proof
chain**, not just the leaf — the grant being revoked is a *proof* in some later
chain, not its leaf. This is what makes B7's revocable deploy grant actually
revocable.

### A8. Deferred, recorded

- **One DID per principal.** Pairwise/per-service DIDs defeat cross-service
  relationship joins and anchor propagation (A5). The threat they answer is
  privacy (correlation by colluding services), not access control, and has no M4
  consumer. If ever adopted, the app instance is the natural boundary — it
  already bounds the data namespace, blob namespace, and per-app KEK.
- **SCP-style node ceilings.** Conjunction (A4) *almost* gives them free — a
  ceiling is a caveat on the root grant — except only chains passing through
  that root inherit it, and under A6 a service-owner-rooted chain does not pass
  through the substrate owner. Deferred: benefit is not yet real (B7's
  marketplace is "eventually"), effort is structural, and the risk is
  philosophical — service owners would stop being independent roots and become
  the node's delegates. Interim: B7's binary deploy grant is a coarse ceiling.
