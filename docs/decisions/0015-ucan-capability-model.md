# D-04-01: UCAN Capability & Verification Model

**Status**: Accepted

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
