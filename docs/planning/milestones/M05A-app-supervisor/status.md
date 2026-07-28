# M05A App Supervisor — Status

**Milestone:** [task.md](task.md) · **Design of record:**
[ADR-0020](../../../decisions/0020-stable-logical-service-identity.md),
[ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md)

**Overall:** Design accepted 2026-07-27. Slices A0-A1 complete (2026-07-28);
A2-A6 not started.

## Slice status

| Slice | Scope | Status | Gate |
|---|---|---|---|
| P0 | `ControllerAgreement` creation tool — **pulled forward from M5 item 5** | Not started | None; gates A3 |
| A0 | Stable member identity (master DID per member + delegated instance keys + ingress `scope` enforcement) | **Complete (2026-07-28)** — [implementation plan](slice-a0-implementation-plan.md), evidence below | None — independently mergeable |
| A1 | Endpoint records published under the member master DID | **Complete (2026-07-28)** — [implementation plan](slice-a1-implementation-plan.md), evidence below | A0 |
| A2 | Host-side dependency resolution; bindings carry `expected_asserter_did` | Not started | A1 |
| A3 | Multi-substrate placement + substrate inventory | Not started | `ControllerAgreement` tool (see below) |
| A4 | Health declaration + read-only monitoring | Not started | A3 |
| A5 | Supervisor loop, best-effort delivery, operator read surface | Not started | A0–A4 |
| A6 | Durable delivery via outbox/DLQ | **Deferred, post-M5** | M5 item 1 Complete |

**A1 was added after design review** (2026-07-27) on finding that the
`master DID → endpoint` mapping ADR-0021 leaned on does not exist: the registry
verifies a record against the key resolved from the `service_id` it is keyed
under, so an instance key cannot publish under its master, and
`MasterAnchorPayload` is a revocation list with no forward index. Without A1,
relocation silently stops resolving — the exact failure A0 exists to prevent.

**A0 planning found four places where the design of record asserts something
the tree does not do** ([slice-a0-implementation-plan.md](slice-a0-implementation-plan.md)
§6), and **one of them changed a later slice's scope**:

- ADR-0020 §1 describes a service instance presenting its certificate on its
  route preamble "the same way a delegated client does today," but no service
  presents its own identity on an outbound call at all — a guest-originated
  remote call presents nothing (`router/src/proxy.rs`), and a substrate-internal
  one presents the *node's* key. A0 builds that arm rather than inheriting it.
- ADR-0020 §1's "this needs no change to FDAE" holds for the sieve and misses a
  second credential path: a `RelationshipProof` is signed with the *instance*
  key and checked for **exact** equality against the policy's
  `expected_asserter_did` (`rpc/src/relationship_proof.rs`), so a reinstantiated
  member silently stops satisfying every policy naming it. Republishing per
  restart is ruled out by the reference scenario's step 4. **Now in A2's scope**
  (declare `expected_asserter_did` as the member master; accept an instance
  signature carrying a delegation from it) with failure-matrix row 19.
- `ServiceId`'s meaning change is not purely semantic: today's plan ids are
  fabricated DIDs with no private key, which `resolve_did_key` rejects.
- The ingress scope check is necessarily an allowlist of the two transport
  scopes; the narrow single-value comparison lands at A1.

ADR-0020 needs an amendment on all four at A0 sign-off. **A5 also gained
explicit text** for what A0 deliberately left out — member-master vault custody,
unattended renewal, and `RotationPolicy`'s first real use — so the online-key
posture is a named deliverable rather than a backlog row pointing at a slice
that never mentions it.

## A0 — Verification evidence (2026-07-28)

A review pass over the implementation plan (before any code) found a fifth
inaccuracy beyond the four above (§6 item 6: the DHT endpoint-record path has
its own delegation check with the *inverse* keying of ADR-0020 §6, which names
only the HTTP registry path) plus three coverage corrections, folded into
[slice-a0-implementation-plan.md](slice-a0-implementation-plan.md) before
implementation started. [ADR-0020](../../../decisions/0020-stable-logical-service-identity.md)
now carries a dated amendment covering all seven.

**What shipped**, phase by phase (see the implementation plan's phase table for
the full gate history):

- `SCOPE_ROUTING`/`SCOPE_SERVICE_INSTANCE`/`TRANSPORT_SCOPES` and
  `DelegationCertificate::verify`'s now-required `accepted_scopes` argument
  (`crates/identity/src/delegation.rs`), enforced at the ingress
  (`crates/router/src/handshake.rs`).
- The instance-certificate store on `EndpointRegistry`/`EndpointStorage`,
  implemented by all four backends including the production SQLite one, whose
  schema creation now runs unconditionally on every open instead of gating on
  `PRAGMA user_version == 0` (D-A0-10) — the gate would have silently skipped
  the new table on every database that predates it.
- `orchestrator/resolve-instance-identity` (the pre-deploy pubkey query),
  deploy-time four-step certificate verification, and undeploy cleanup
  (`crates/control_plane/src/service/orchestration.rs`).
- `ProxyRouter`'s `CallOrigin::Guest` arm presenting a service's own certified
  instance key instead of going anonymous when one is installed
  (`crates/router/src/proxy.rs`) — the load-bearing gap the plan's §0 called
  out: no service presented its own identity on an outbound call before this.
- `roymctl identity certify-instance`, `svc deploy --master`, and
  `app deploy --mint-masters` (post-compile service-id substitution on a copy
  of the plan, taken *after* the deployment journal already recorded the
  fabricated ids, so the journal never holds master-DID-bearing plans)
  (`apps/roymctl/src/commands`).
- The heartbeat near-expiry warning and `svc list`'s expiry column
  (`crates/substrate/src/runtime.rs`).

**Tests added:** 6 new unit tests in `crates/identity/src/delegation.rs`
(scope enforcement), 6 in `crates/router/src/handshake.rs` (ingress scope +
revocation + reinstantiation), 8 in `crates/control_plane/src/service/
orchestration.rs` (instance-identity determinism + install verification), 4 in
`crates/router/src/proxy.rs` (guest-origin presentation, expired-certificate
fallback, and the node-level-interface deny), 4 in
`crates/data_db/src/registry_store.rs` (the schema-gate regression D-A0-10
exists to prevent, plus upsert/removal), 3 in `crates/core/src/local_registry.rs`,
1 in `crates/substrate/src/runtime.rs` (near-expiry warning), 11 in
`apps/roymctl` (naming, resolve/mint, CLI parsing, expiry formatting), and one
new two-real-substrate e2e test,
`a_member_master_authorizes_a_distinct_instance_key_on_each_real_node_it_deploys_to`
(`crates/substrate/tests/instance_identity_e2e.rs`) — proves live that
`instance-identity` derives a distinct key per real node for the identical
`(caller, service_id)` pair, that `deploy` verifies and installs a certificate
(rejecting a wrong-scope one), that `list` reports the installed certificate's
real expiry, and that the reference scenario's step-4 claim holds across two
independently-keyed real substrates: reinstantiating a member on a second node
yields a new instance key while the certified member master identity does not
change. **Not covered** by that fixture: a live, wire-level proof that a
guest-origin call presents its certified instance key across a real cross-node
QUIC hop (that needs a WASM guest; recorded in `deferred-backlog.md`) — the
guest-arm's own code path is proven at the router level instead.

**Gates, run 2026-07-28:**

- `cargo +nightly fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets --all-features`: clean, zero
  warnings.
- `cargo test --workspace` (sandboxed, matching this repo's established fast/
  deterministic default): green except the same category of pre-existing,
  environmental socket-bind failures documented throughout this milestone's
  and M04A/M04B's status docs (real port/DHT/relay binds the sandbox denies) —
  confirmed by a direct diff against an unmodified `main` checkout run the same
  way, which shows the identical failure set. The new e2e test above is in
  that same category under the sandboxed run (it needs real port binds) and
  was verified passing individually, sandbox disabled, twice in a row
  (~14-15s each).
- `mise run test:e2e` (sandbox disabled, required for real port binds): 12/12
  green (8 main + 4 multi-hop), unchanged from before this slice.
- `wasm32-wasip2`: `data-layer-test`, `greeter`, and `proxy-test` all still
  build clean — the WIT changes in this slice (`instance-identity`,
  `deploy-manifest.instance-certificate`, `deployed-service.instance-
  certificate-expires-at`) touch no interface any guest fixture imports.

## A1 — Verification evidence (2026-07-28)

A0 shipped a master DID per member and delegated instance keys, but nothing
could turn a master DID into a network address: the registry verified a
record against the key resolved from the `service_id` it was keyed under, so
an instance key could not publish under its master, and there was no
substrate-side publish path for a service endpoint record at all — only a
replay of an operator-signed file on the hourly heartbeat. Planning found
thirteen more places where ADR-0020 §6 and task.md described the tree
inaccurately or asserted a property it did not have (four review passes;
[slice-a1-implementation-plan.md](slice-a1-implementation-plan.md) §1's
thirteen numbered decisions and §6's corrections list), folded in before
implementation started; ADR-0020 now carries a second dated amendment.

**What shipped**, by crate:

- **`crates/identity`:** `DelegationCertificate::verify_chain` split out from
  `verify` — the master match, scope, and signature, without the validity
  window — so a *reader* of an already-admitted record can check the trust
  chain without re-adjudicating a live credential's expiry (D-A1-10).
- **`crates/core/src/dht_registry.rs`:** `RecordTrust::{Publishing, Reading}`
  and `SignedEndpointInfo::verify`'s rewrite implementing ADR-0020 §6's second
  keying shape (a record keyed by a master DID, signed by an instance key
  presenting a certificate from that master); `EndpointInfo::sign_as_instance`;
  whole-struct `PartialEq` on `EndpointInfo` and `MasterAnchorPayload` so
  `verify` authenticates the entire record/anchor rather than one field
  (D-A1-9, D-A1-13); `RegistryClient::register`'s DHT-leg skip for a
  delegation-signed record, since BEP0044 keys a packet by its signing key and
  can never hold one under its master DID (D-A1-2); `SignedMasterAnchor::
  verify_signature`/`fetch_own_master_anchor`/`refresh_master_anchor`, a
  read-modify-write that carries every stateful anchor field forward rather
  than wiping revocations on every renewal (D-A1-7, D-A1-12).
- **`crates/community_registry`:** `verify_endpoint_signature` simplified to
  one call plus a registry-local, best-effort revocation check at admission
  (D-A1-6) — defence in depth; the real gate stays the handshake.
- **`crates/core/src/endpoint_publisher.rs`** (new): `EndpointPublisher`,
  built from the D-A1-4 decision table — a certified, unexpired service
  publishes a fresh instance-signed record; an expired certificate or a
  drifted owner row publishes nothing (warned); no certificate replays the
  stored file, but only after verifying it still self-verifies.
- **`crates/control_plane`:** `ControlPlaneService::set_endpoint_publisher`
  (`OnceLock`, mirroring `service_proxy`'s two-phase wiring) and the
  publish-on-deploy hook in `deploy`, so a reinstantiated member becomes
  resolvable promptly rather than waiting for the hourly heartbeat.
- **`crates/substrate/src/runtime.rs`:** `setup_router` builds the
  `EndpointPublisher` and wires it into the control plane; the heartbeat's
  hosted-apps block collapses to `publisher.publish_all_services().await`.
- **`apps/roymctl`:** `svc deploy`'s three deploy shapes now all carry a
  nickname without requiring `--identity` (D-A1-8, including an ephemeral
  signing key on the `--instance-certificate` path, which has no operator key
  to reach for); `--registry-url` on `identity certify-instance`, `svc
  deploy`, and `app deploy`, calling `refresh_master_anchor` once per master
  and warning loudly when omitted (D-A1-7).

**Tests added:** 10 new unit tests in `crates/identity/src/delegation.rs`
region (verify/verify_chain split, existing six still green), 15 in
`crates/core/src/dht_registry.rs` (record verification shapes, D-A1-9's
tamper tests, D-A1-2's DHT-leg error, D-A1-10's publish/read split, D-A1-12's
stale-anchor split, D-A1-13's whole-payload anchor tamper tests), 6 in
`crates/community_registry/src/registry.rs` (delegation-signed
register/lookup, revoked-key admission rejection, alias-by-master, and three
`refresh_master_anchor` regression guards including the stale-anchor and
unreadable-anchor cases), 10 in `crates/core/src/endpoint_publisher.rs`
(`build_record`'s full decision table plus the sweep's union-and-survive-one-
failure behavior), 1 in `crates/control_plane/src/service.rs`
(`set_endpoint_publisher` is set-once), and 2 CLI parse-level tests in
`apps/roymctl/src/commands/svc.rs` (D-A1-8's two flag-carrying shapes) — 44
new unit/CLI tests total. One new two-real-substrate e2e test,
`a_member_master_did_resolves_to_an_address_and_follows_the_member_across_nodes`
(`crates/substrate/tests/master_endpoint_record_e2e.rs`), Node A hosting the
shared community registry and Node B pointed at it (D-A1-2's requirement,
proven rather than merely stated): certifies and deploys a member master on
node B, resolves it *by the master DID* via `RegistryClient::lookup(resolve =
true)` and `net_iroh::resolve_iroh_addr`, asserting the returned mechanisms
and address are node B's own and the record's certificate names node B's
derived instance key; cleanly relocates the same master to node A
(`undeploy` before the second `deploy`, deliberately, per D-A1-11) and
re-resolves, showing the same DID now yields node A's address with no
operator republish action — the reference scenario's step 4, live; and posts
a hand-forged record (keyed by the master, signed by an uncertified key)
straight to the registry, asserting `401` — failure-matrix row 4 over the
wire.

**Gates, run 2026-07-28:**

- `cargo +nightly fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets --all-features`: clean, zero
  warnings.
- `cargo test --workspace` (sandboxed): green except the same category of
  pre-existing, environmental socket-bind failures as A0 above, confirmed by
  a direct diff against an unmodified `main` checkout run the same way (main:
  12 failing targets; this branch: the identical 12 plus exactly two new
  ones — `syneroym-community-registry --lib` and `syneroym-substrate --test
  master_endpoint_record_e2e`, both needing real port binds and both verified
  passing individually with the sandbox disabled, the latter twice in a row,
  ~14-15s each).
- `mise run test:e2e` (sandbox disabled, required for real port binds): 12/12
  green (8 main + 4 multi-hop), unchanged from before this slice.
- `wasm32-wasip2`: `data-layer-test`, `greeter`, and `proxy-test` all still
  build clean — A1 touches no WIT interface.

## Dependencies pulled in

1. **`ControllerAgreement` creation tool + the two items B7 pairs with it**, all
   three as **Slice P0** rather than left in M5. Decided 2026-07-27; see P0 in
   `task.md` for the reasoning and for why the three cannot be separated.

   Why it moved: M5 item 5 had no scheduled position at all — the 2026-07-16
   resequencing front-loads item 1 and defers items 2-4, never mentioning item 5
   — so A3 would have been gated on something with no date. And the exposure is
   concrete rather than theoretical: an unowned substrate issues
   `orchestrator/deploy` to every verified caller, and this milestone is what
   turns that contained bootstrap posture into a fleet of unattended, networked
   deploy targets.

## Decisions carried in from design (2026-07-27)

- Push, not pull: no service-facing directory; the trigger for revisiting is a
  measured convergence budget, recorded in ADR-0021 §6 and in this milestone's
  exit criteria. An operator-facing read surface does exist and is required.
- One master DID per **member**, not per logical service — otherwise a redundant
  service's member list collapses to a repeated DID and round-robin and sharding
  have nothing distinct to select over.
- The generation stamp is minted by an operator `adopt` action, never
  self-incremented; it is a tiebreaker among authorized writers, not an
  authorization mechanism.
- Substrate role, not a WASM `SynApp` — deviates from the pre-2026-07-27 text in
  `system-architecture.md` §LFC-MGT, which has been corrected.
- Master keys are per member, and an operator picks one of two postures
  (ADR-0020 §3), because certificate *renewal* needs the same key relocation
  does — attended mode reschedules the online key rather than avoiding it:
  **online-key** (supervisor holds member masters, short-lived certificates,
  issues and renews unattended) or **attended** (long-lived certificates where
  revocation is the control, operator issues on a cadence, and a missed renewal
  is an outage rather than a degradation).
- Remediation is restart-in-place only until M7 replication lands.
- The registry-trust-model ADR that M04A B7 recorded as owed (§6.2 / F9 option
  2) is **discharged** by ADR-0020 §6 plus slice A1, not scheduled separately —
  it is the same change to `verify()`'s contract, reached from the opposite
  direction, and B7's "needs a real consumer" gate is met by A1.

## Superseded work

The *Interstitial: Live App-Context Registry* placeholder
([meta-implementation-plan.md](../../meta-implementation-plan.md), reserved
2026-07-24) is superseded by this milestone. Both of its goals are carried
forward in A2: logical-name resolution backed by real deployment state, and
`expected_asserter_did` publication (M04B Slice B3's D-B3-8 residual).
