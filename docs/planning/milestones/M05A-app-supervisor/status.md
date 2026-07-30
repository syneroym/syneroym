# M05A App Supervisor — Status

**Milestone:** [task.md](task.md) · **Design of record:**
[ADR-0020](../../../decisions/0020-stable-logical-service-identity.md),
[ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md)

**Overall:** Design accepted 2026-07-27. Slices P0, A0-A2 complete
(2026-07-30); A3-A6 not started.

## Slice status

| Slice | Scope | Status | Gate |
|---|---|---|---|
| P0 | `ControllerAgreement` creation tool — **pulled forward from M5 item 5** | **Complete (2026-07-30)** — [implementation plan](slice-p0-implementation-plan.md), evidence below | None — clears A3's gate |
| A0 | Stable member identity (master DID per member + delegated instance keys + ingress `scope` enforcement) | **Complete (2026-07-28)** — [implementation plan](slice-a0-implementation-plan.md), evidence below | None — independently mergeable |
| A1 | Endpoint records published under the member master DID | **Complete (2026-07-28, design revised 2026-07-29 before merge)** — [implementation plan](slice-a1-implementation-plan.md), evidence below | A0 |
| A2 | Host-side dependency resolution; bindings carry `expected_asserter_did` | **Complete (2026-07-29)** — [implementation plan](slice-a2-implementation-plan.md), evidence below | A1 |
| A3 | Multi-substrate placement + substrate inventory | Not started | P0 (Complete) |
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

## P0 — Verification evidence (2026-07-30)

Planning found eleven places where `task.md` described a tree that did not
exist, left a decision unmade, or understated the blast radius — recorded in
[slice-p0-implementation-plan.md](slice-p0-implementation-plan.md) §0's
eleven numbered findings across two review rounds, five of which changed
what P0 had to build (most consequentially §0.3/§0.3a: the fail-closed flip's
blast radius is nine harnesses, not "reconsider a default," and the
performance harness (`tests/perf`) is invisible to every other gate this
slice's own gate list would otherwise have relied on). §1's twelve decisions
(D-P0-1 … D-P0-12) took those findings as given.

**What shipped**, by phase:

- **Phase 1 (the tool, independently mergeable):** `roymctl substrate claim`
  (aliased `roymctl node claim`) mints a mutually-signed `ControllerAgreement`
  from two local key files — `ControllerAgreement::issue`
  (`crates/identity/src/substrate.rs`) rejects a self-owned agreement
  (D-P0-4, §0.1) and canonicalizes/signs both proofs over the
  proof-less payload `SubstrateIdentityState::init` reconstructs.
  `setup_substrate_identity` (`crates/substrate/src/identity.rs`) discovers
  `<app_data_dir>/agreement.json` implicitly when `[identity].agreement` is
  unset (D-P0-5), and treats a present-but-unparseable agreement — explicit
  or discovered — as a hard boot failure rather than silently unowned
  (D-P0-6, §0.2). `SubstrateIdentityState::init` gained two verification
  tightenings: `agreement_type` must equal the literal `"ControllerAgreement"`
  (D-P0-7), and a present-but-unparseable `expiresAt` is now an error instead
  of fail-open "no expiry" (§0.9). `roymctl substrate init` now writes
  `substrate.key`, not `identity.key` (§0.5, `DEFAULT_SUBSTRATE_KEY_FILE`),
  closing the trap that both e2e Playwright configs previously papered over
  with an explicit override.
- **Phase 2 (gate `security` on `substrate/admin`):** the `TODO(M04B/FDAE)`
  block in `ControlPlaneService::dispatch`
  (`crates/control_plane/src/service.rs`) is replaced with a
  `has_node_wide_ability(caller, Ability::SUBSTRATE_ADMIN)` check ahead of
  every `security` method (`inject-kek`/`rotate-kek`/`set-secret`), denying
  with `syneroym_rpc::PERMISSION_DENIED_CODE` (`-32010`, D-P0-9, promoted
  from the literal `synsvc_native.rs` already used) rather than a generic
  internal error, so a caller can assert *denial* without string-matching.
  No exemption for substrate-injected callers (D-P0-8, §0.10) — nothing
  inside the substrate dispatches to `security`.
- **Phase 3 (fail closed):** `build_caller`
  (`crates/router/src/route_handler/io.rs`) no longer issues the three
  `orchestrator/*` abilities to every verified caller when `admin_root` is
  `None` — an unowned substrate now grants **no** node-wide capability at
  all (D-P0-10: unconditional, no `[iam].allow_unowned_deploy` escape
  hatch). The boot-time `warn!` in `crates/substrate/src/runtime.rs` was
  rewritten to describe the fail-closed posture and point at `roymctl
  substrate claim` as the remedy.
- **Call-site sweep (§5), all nine harnesses that reached the removed free
  grant:** the three single-node integration harnesses
  (`crates/substrate/tests/common/mod.rs`, `basic_lifecycle.rs`,
  `podman_lifecycle.rs`) now mint an owner `Identity`, set
  `admin_ucan_root`, and build their `substrate_client` with
  `SyneroymClient::new_with_identity` (D-P0-11 — every existing
  `deploy`/`inject_kek` call site downstream needed no edit).
  `instance_identity_e2e.rs` and `master_endpoint_record_e2e.rs`'s `Node::boot`
  now take an owner identity and own the node before any deploy/KEK call.
  `federated_fdae_e2e.rs` is the non-mechanical one (§0.4): Node B gets its
  own, *distinct* owner (not Node A's, not alice's — naming her would grant
  `substrate/admin`, which entails `data-layer/write` everywhere on Node B
  and defeat the file's whole point), and `alice_deployer`/`bad_app_deployer`
  instead present an app-scoped `orchestrator/{deploy,undeploy,status}`
  grant issued by that owner (`app_deploy_grant`, all three abilities
  together per the `undeploy` rollback interaction documented at
  `orchestration.rs`'s `undeploy_impl`). Both Playwright e2e configs
  (`global-setup.ts`, `global-setup-multihop.ts`) now run the real
  `identity create`/`substrate claim` flow before starting the substrate
  (D-P0-12) and pass `--as owner` on their `svc deploy` calls; the
  multi-hop config claims only `sz`/`sx` (nothing deploys to `c`/`cp`).
  `tests/perf`'s `TestEnvironment::new` (§0.3a, §5.7) mints an owner and a
  `ControllerAgreement` from the already-generated node identity before
  `start_substrate`, and passes `run --agreement <path>` (the existing flag);
  `owner_key: [u8; 32]` is exposed (not `Identity`, which is not `Clone`) so
  each of the five orchestrator-targeting scenario clients — including
  `soak.rs`'s deploy-churn loop, running inside a spawned task — can
  reconstruct one with `Identity::from_bytes`. The six app-targeting
  scenario clients (dialing a deployed service's own `service_id`, not the
  substrate) were deliberately left alone.
- **New e2e test:** `crates/substrate/tests/substrate_ownership_e2e.rs`'s
  `a_claimed_substrate_admits_its_controller_and_denies_everyone_else` is the
  only test exercising discovery, the handshake, and both gates together —
  `ControllerAgreement::issue` writes `agreement.json` into
  `app_data_dir` *before* the substrate ever boots, then a single real boot
  must come up `Verified` with no `[identity].agreement` config line at all;
  the controller deploys and injects a KEK, an unrelated verified identity
  is denied both.
- **Comment sweep (§3.3):** every stale `unowned`/`F4` reference this slice's
  own code changes made false was corrected in the same pass — the
  `has_node_wide_ability` and `build_caller` doc comments, the
  `undeploy_impl`/takeover-check/list-visibility comments in
  `orchestration.rs`, `crates/ucan/src/capability.rs`'s `is_substrate_scope`
  doc, `crates/router/src/proxy.rs`'s node-level-interface denial comments,
  and `crates/router/tests/service_ownership.rs` (including renaming
  `unowned_substrate_lists_every_app_to_any_caller` to
  `node_wide_authority_lists_every_app`, since the assertion no longer
  describes an unowned substrate). `crates/client_gateway/src/gateway.rs`'s
  `TODO(post-B0)` is corrected, not resolved (flagged, not fixed, §0.10):
  the gateway still presents the node's own DID, which now holds nothing
  node-wide — harmless only because the gateway never proxies to
  `orchestrator`/`security`, a routing accident recorded as its own backlog
  row.

**Tests added: 15 new unit/CLI/e2e tests, plus 2 existing tests renamed and
given updated bodies (their caller shape and/or assertions changed to match
the post-P0 posture) — counted directly from `git diff main`, not asserted**
— 7 in `crates/identity/src/substrate.rs` (`issue`'s round trip, self-owned
rejection, wrong-`controlled` rejection, expiry, unparseable-expiry,
unknown-type, and a tampered-field/re-signed-payload test); 2 in
`crates/substrate/src/identity.rs` (discovered-agreement load,
malformed-discovered-agreement hard failure); 3 CLI tests in
`apps/roymctl/src/commands/substrate.rs` (`claim` writes a verifiable
agreement, refuses to overwrite without `--force`, reports a missing
substrate key with the `init` hint — factored into a testable `claim()`
function per `commands.rs`'s `client_for_rejects_ucan_without_as`
precedent); 1 in `crates/router/src/route_handler/io.rs`
(`an_unowned_substrate_grants_no_node_wide_capability`, **matrix row 17**,
replacing the test that used to assert the opposite); 1 in
`crates/control_plane/src/service.rs`
(`security_is_denied_without_substrate_admin`, **matrix row 16**, asserting
`PERMISSION_DENIED_CODE` rather than string-matching) plus
`security_is_allowed_for_a_substrate_admin_caller` (renamed from
`test_security_dispatch_returns_sdk_statuses`, caller swapped to a new
`substrate_admin_caller` helper kept deliberately distinct from
`node_wide_caller`); `crates/router/tests/service_ownership.rs`'s
`node_wide_authority_lists_every_app` (renamed, assertions unchanged) and
`deploy_grant.rs`'s `deploy_denied_without_an_orchestrator_grant` (module
doc extended in place, per the plan's own note that this test already
covers matrix row 17 at the `ControlPlaneService` level — no duplicate
test added); 1 new two-real-substrate e2e test,
`a_claimed_substrate_admits_its_controller_and_denies_everyone_else`
(`crates/substrate/tests/substrate_ownership_e2e.rs`).

**Gates, run 2026-07-30:**

- `cargo +nightly fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets --all-features`: clean, zero
  warnings.
- `cargo test --workspace --no-fail-fast` (sandboxed): 15 failing targets,
  the same pre-existing environmental category documented throughout this
  milestone's status — real socket/port binds the sandbox denies outright:
  `syneroym-community-registry --lib`, `syneroym-coordinator-iroh`
  (`connection_limit`/`multi_hop_relay`/`tls_rotation`),
  `syneroym-mqtt-broker --lib`, `syneroym-sdk --test connect_timeout`, and
  every `syneroym-substrate` e2e test (`basic_lifecycle`,
  `federated_fdae_e2e`, `http_passthrough_e2e`, `instance_identity_e2e`,
  `master_endpoint_record_e2e`, `messaging_client_e2e`,
  `stream_client_e2e`, and the new `substrate_ownership_e2e`). Every one of
  these was independently re-verified passing with the sandbox disabled
  this pass, run individually or in small groups
  (`instance_identity_e2e`/`master_endpoint_record_e2e` together, 18-25s
  each; `federated_fdae_e2e` 89s; `community_registry`'s 16 tests,
  `mqtt-broker`'s 11, `sdk`'s `connect_timeout`, and `coordinator-iroh`'s
  three tests all green; the six single-node substrate e2e/basic/podman
  tests green). `syneroym-router --test proxy_dispatch` does **not**
  belong in this category — review found it fails intermittently under
  parallel execution (`--test-threads=1`: 8/8 every time; default
  threading: fails on alternating runs, on a WASM trap inside
  `cabi_realloc` surfaced as `-32603`, not a bind error) and it was last
  touched by A2, not this slice. Tracked as a flake in
  `deferred-backlog.md` §1, not this sandbox-bind list. No target outside
  the corrected list above was affected.
- `mise run test:e2e` (sandbox disabled, required for real port binds):
  12/12 green (8 main + 4 multi-hop) — the real end-to-end operator claim
  flow (`identity create` + `substrate claim` + `--as owner svc deploy`)
  exercised live in both Playwright configs, unchanged pass count from
  before this slice.
- `mise run bench:latency` (§0.3a's own required gate, sandbox disabled):
  the ownership-harness fix is proven — `orchestrator.deploy` succeeds via
  the owner identity `TestEnvironment::new` now mints and passes through
  `run --agreement`, visible live in the log
  (`Orchestrator received dispatch: orchestrator.deploy` /
  `Deploying TCP service …`). Review found a second, unrelated bug the run
  then hit: `Error: wasm trap: all fuel consumed by WebAssembly` inside
  `wasm_latency.rs`'s in-process baseline, which builds its `Store`s by
  hand and never calls `store.set_fuel` the way the real dispatch path
  does (`engine.rs`'s `prepare_wasm_execution`) -- wasmtime's
  `consume_fuel(true)` engine option then traps at zero fuel on the first
  call. Not the generic, already-tracked "WASM fuel metering: not yet
  configured" gap (`deferred-backlog.md` §8), which is about production
  dispatch and was a mis-attribution: that path already sets fuel from
  `SandboxWasmConfig::default_max_instructions`. Fixed by adding
  `store.set_fuel(BASELINE_FUEL)` (mirroring that same production default,
  10 billion instructions) to both baseline loops in `wasm_latency.rs`.
  `bench:concurrency`/`bench:soak` share the same harness fix and were not
  run separately (§6's own note: one scenario is enough to prove it).
- `mise run test:smoke`: passes, unaffected (§5.8's own claim, verified
  rather than assumed) — no `orchestrator`/`security` call in that binary.
- `wasm32-wasip2`: `greeter`, `data-layer-test`, and `proxy-test` all build
  clean — P0 touches no WIT interface and no WASM-facing code.

**Not covered — recorded as a backlog row, not silently dropped** (see
`deferred-backlog.md` §3/§7/§8): no remote `ControllerAgreement` claim (the
tool is local-and-offline by construction, §0.8); claiming a running
substrate needs a restart (no `SIGUSR1`-style hot-reload); the client
gateway still presents the node's own DID, which now holds nothing
node-wide; `Capability::grants` still wildcards any bare `substrate:`
resource regardless of which node's DID follows it (defense-in-depth, not a
live hole); ownership transfer/revocation has no mechanism beyond `claim
--force`; and A5's supervisor, if it provisions secrets on substrates it
manages, will need node-wide `substrate/admin` on each — directly tensioning
against failure-matrix row 14's blast-radius claim, flagged for evaluation
before A5 commits to its custody model.

### Post-merge review (2026-07-30), all fifteen findings incorporated

An independent review of the merged commit, re-running every gate rather
than trusting the evidence above, found fifteen issues the implementation
pass missed. All were agreed with and fixed in the same pass; none were
pushed back on.

**Correctness/security (high):**
- `roymctl substrate init` overwrote an existing `substrate.key` with no
  guard, silently orphaning everything attributed to the old node DID and
  invalidating any `agreement.json` already claiming it -- now a hard
  `--force`-gated bail, mirroring `identity create`'s existing guard.
- The same command wrote the key with `fs::write` (world-readable, `0644`)
  instead of `Identity::save_to_path` (`0600`, zeroizing) -- the key that
  now grants `substrate/admin` via a self-signed `ControllerAgreement`.
  Fixed to use `save_to_path`.
- `substrate_ownership_e2e.rs`'s stranger-`inject_kek` assertion passed
  even with the `security` gate deleted, because the controller's own
  earlier `inject_kek` call made a second injection fail with
  `KekAlreadyInjected` (`-32603`) regardless of the gate. Fixed to
  downcast to `JsonRpcError` and assert `PERMISSION_DENIED_CODE`
  specifically. The review's suggested fix also proposed asserting the
  same code on the sibling `deploy` assertion; verified against a live run
  that `orchestrator/deploy`'s Tier-1 admission denial has no distinct
  code of its own (`ControlPlaneService::dispatch`'s `"deploy"` arm maps
  every cause through `.map_err(RpcError::InternalError)`, unlike
  `security`'s explicit `RpcError::Custom(PERMISSION_DENIED_CODE, ..)`),
  so that half of the suggested fix was corrected to check the denial
  message instead of a code that does not exist on that path.
- `--expires-days` panicked on a large value (`Duration::seconds(secs as
  i64)` and `now + duration` both overflow-panic in chrono 0.4.45) --
  reproduced, then fixed with `checked_mul`/`try_seconds`/
  `checked_add_signed` throughout, returning a descriptive error instead.

**Correctness (medium/low):**
- `claim` printed "Substrate claimed." without verifying what it actually
  wrote to disk -- a serde round-trip bug or field rename would have given
  a success message for a node that boots unowned. Fixed: read the file
  back, parse it, and run it through `SubstrateIdentityState::init`,
  bailing unless `Verified`. The CLI test now does the same read-back
  rather than trusting the in-memory value.
- Agreement expiry is checked once, at boot, and never again for the
  process's lifetime -- `--expires-days`'s help text now says so, and a
  backlog row records the gap.
- A discovered `agreement.json` can silently outrank a configured
  `controller_did` with no warning, since the exclusivity check in
  `main.rs` only sees the *configured* path. Fixed: `warn!` once when this
  happens.

**Test/gate integrity:**
- `tests/perf/src/scenarios/wasm_latency.rs`'s in-process baseline built
  its `Store`s by hand and never called `store.set_fuel`, unlike the real
  dispatch path -- `mise run bench:latency` traps on "all fuel consumed"
  every run. `status.md` had mis-attributed this to the generic, already
  -tracked "WASM fuel metering: not yet configured" backlog row,
  which is about production dispatch (already correctly fuel-metered) and
  did not fit. Fixed with `store.set_fuel` on both baseline loops,
  matching `SandboxWasmConfig::default_max_instructions`'s production
  default; the mis-attribution above is corrected. (A second, independent
  cause behind the same gate is fixed in the follow-up pass below.)
- `syneroym-router --test proxy_dispatch` was listed among the
  sandbox-port-bind failures in this doc's `cargo test --workspace` gate
  notes; it is actually a genuine flake under parallel execution (8/8 with
  `--test-threads=1`, intermittent otherwise, on an unrelated WASM trap),
  pre-existing and last touched by Slice A2. Moved out of that list here
  and tracked as a flake in `deferred-backlog.md` §1.
- A stale comment in `federated_fdae_e2e.rs` still called Node B unowned,
  one slice after this change gave it an owner; the conclusion (alice
  needs a grant) survived, but the stated reason was false. Fixed.
- Matrix row 17 ("deploy to an unowned substrate is rejected") had two
  unit-level proofs that never met a real unowned substrate over the
  wire. Added `an_unowned_substrate_rejects_a_deploy` to
  `substrate_ownership_e2e.rs`, which boots a node with no agreement at
  all and asserts a real deploy is denied specifically for lack of a
  grant (see the note above on `deploy` having no distinct denial code).

**Bookkeeping:**
- 76 planning-doc references (milestone/slice IDs, `D-P0-*` decision
  IDs) had been added to code comments, doc comments, and a test name,
  against AGENTS.md's *No Planning-Doc References in Code* rule. Stripped
  from every line this diff added, keeping the underlying *why* intact;
  pre-existing violations in code this diff did not touch were left alone.
- `docs/developer-guide.md`'s WASM and TCP deploy walkthroughs still read
  as runnable `curl` examples that fail post-P0 (the warning above them
  was correct, the examples below it were not updated). Replaced with
  their `roymctl --as owner svc deploy` equivalents; the container-deploy
  example has no CLI equivalent yet (tracked separately), so it stays a
  raw JSON-RPC reference with the same caveat noted inline.
- Two backlog rows added: the deploy-only-grantee rollback-denial gap
  `ControllerAgreement` makes reachable (`orchestration.rs`'s own comment
  already documented the mechanism in detail, just not as a backlog row),
  and the boot-time-only expiry check above. A third existing row
  (`remove_owner` non-atomicity) gained a sentence noting P0 makes its
  "ID squatting" consequence bite an ordinary caller on any claimed
  substrate, not only a hypothetical one.

**Gates, re-run 2026-07-30 after the fixes above:** `cargo +nightly fmt --all
--check`, `cargo clippy --workspace --all-targets --all-features`, and
`cargo check --workspace --all-targets` all clean. `cargo test --workspace
--no-fail-fast` (sandboxed): every failure independently confirmed to carry
the sandbox's "Operation not permitted (os error 1)" bind-denial signature,
plus the one pre-existing `proxy_dispatch` flake (now tracked); re-run
unsandboxed, `substrate_ownership_e2e` (including the new
`an_unowned_substrate_rejects_a_deploy`), `basic_lifecycle`,
`syneroym-identity`, and `roymctl` are all green -- this is also where the
`deploy`-denial-code correction above was actually caught (a live run, not
a read of the code). `mise run test:e2e` (sandbox disabled): 12/12 green (8
main + 4 multi-hop), unchanged.

### Follow-up review pass (2026-07-30): two problems introduced by the fixes

A second independent review, re-running the gates above against the
uncommitted fix set, found two issues the fixes themselves introduced
(labeled R1/R2 to distinguish from the F-numbered findings above). Both
fixed and reverified live.

- **R1 -- the new unowned-substrate test's ports collided with the claimed
  one's, ten ports up.** `UNOWNED_IROH_PORT` was `8610`; the iroh
  coordinator binds a *second* listener for `/v1/info` at
  `http_bind_address.port() + 10`
  (`crates/coordinator_iroh/src/coordinator.rs`), and the claimed test's
  port is `8600` -- so `8600 + 10 == 8610` collided with the unowned
  test's own primary port whenever both tests in the binary ran
  concurrently (the default). Whichever lost the bind race panicked before
  `node.teardown()` ran, leaking a live substrate holding those ports and
  its tokio tasks. Every other multi-node harness in this crate spaces its
  port blocks by 100 for exactly this reason
  (`8000`/`8100`, `8200`/`8300`, `8400`/`8500`); the new block follows
  suit at `8700`/`8701`/`8702`. Reverified: both tests in the binary now
  pass together under default (concurrent) threading.
- **R2 -- the fuel fix above was necessary but not sufficient for
  `bench:latency`.** `build_wasm_engine` sets `epoch_interruption(true)`
  unconditionally, alongside `consume_fuel(true)`
  (`crates/sandbox_wasm/src/engine.rs`); the real dispatch path sets a
  deadline for both right next to `set_fuel`
  (`store.epoch_deadline_trap()` / `store.set_epoch_deadline(..)`), but
  the perf harness's fuel fix above only mirrored the fuel half. A `Store`
  with epoch interruption enabled and no deadline set traps immediately
  (`wasm trap: interrupt`) on the first call after the fuel trap was
  fixed, so the gate stayed red through a second, independent cause behind
  the first. Fixed by adding the same two calls next to both
  `set_fuel` sites; the harness's own engine has no epoch ticker running
  (that ticker only exists inside `AppSandboxEngine::init`, which this
  baseline never calls), so any positive deadline is safe and will never
  trip. Reverified: `mise run bench:latency` exits 0, both the TCP and the
  WASM Component scenarios print their full comparison tables.
- Also finished in this pass, not required but cheap: F12's five
  remaining planning-doc references (found during this review) were
  stripped too, bringing the added-line count to zero.

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

**Tests added:** 3 new unit tests in `crates/identity/src/delegation.rs`
(`verify_chain`'s own direct coverage: a lapsed-but-once-valid window is
accepted for reading and still rejected for a live-credential check, a
non-positive window and a future-issued certificate are rejected at both
trust levels — added in post-review fix-up, see below; existing thirteen
tests still green), 15 in `crates/core/src/dht_registry.rs` (record
verification shapes, D-A1-9's tamper tests — now asserting both trust levels
— D-A1-2's DHT-leg error, D-A1-10's publish/read split, D-A1-12's
stale-anchor split, D-A1-13's whole-payload anchor tamper tests), 9 in
`crates/community_registry/src/registry.rs` (delegation-signed
register/lookup, revoked-key admission rejection, alias-by-master, three
`refresh_master_anchor` regression guards including the stale-anchor and
unreadable-anchor cases, two D-A1-12 `master_id`-equality regression tests
against a validly-signed anchor served under the wrong master, and a live
`publish_all_services` sweep test proving the id-union and per-service
failure containment — the last three added in post-review fix-up), 10 in
`crates/core/src/endpoint_publisher.rs` (`build_record`'s full decision
table — the sweep's own union/failure-containment behavior is proven in
`community_registry`, not here, see below), 1 in
`crates/control_plane/src/service.rs` (`set_endpoint_publisher` is set-once),
and 2 CLI parse-level tests in `apps/roymctl/src/commands/svc.rs` (D-A1-8's
two flag-carrying shapes) — **40 new unit/CLI tests total**, counted directly
from `git diff main` rather than asserted (a prior revision of this section
overcounted by naming ten phantom `delegation.rs` tests before any existed
there). One new two-real-substrate e2e test,
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
  pre-existing, environmental sandbox failures as A0 above — real socket
  binds the sandbox denies outright, plus an intermittent mainline-DHT
  actor-thread crash (`actor thread unexpectedly shutdown`, `mainline`
  crate) that a handful of DHT-touching test binaries hit under sandboxed
  parallel execution. **The exact failing-target count varies run to run by
  about ±1 for this reason on both `main` and this branch** — measured twice
  each: `main` showed 11 and 12 failing targets across two runs (the
  variable member is `syneroym-router --test native_dispatch_identity`,
  which panics inside the `mainline` crate at `dht.rs:143` and hits
  different tests each run — three *isolated* sandboxed reruns gave pass,
  pass, fail, and it passes 39/39 unsandboxed every time; A1 touches no code
  in that crate); this branch showed 13 and 14 across two runs, consistently
  `main`'s set plus exactly the same two new targets needing real port binds
  — `syneroym-community-registry --lib` and `syneroym-substrate --test
  master_endpoint_record_e2e` — both independently verified passing with the
  sandbox disabled, the latter twice in a row (~14-15s each). No target
  outside that DHT-actor-flake set differs between `main` and this branch.
- `mise run test:e2e` (sandbox disabled, required for real port binds): 12/12
  green (8 main + 4 multi-hop), unchanged from before this slice.
- `wasm32-wasip2`: `data-layer-test`, `greeter`, and `proxy-test` all still
  build clean — A1 touches no WIT interface.

**Independent review (2026-07-28).** A post-merge review found fifteen
findings, none blocking the slice's own claims. All fifteen were
incorporated rather than argued against:

- **Trust-window bug (high):** `verify_chain` was skipping the
  non-positive-window and future-issuance checks entirely, not just
  wall-clock expiry as D-A1-10 intended — a certificate that was never a
  live credential at all (e.g. a zero-length window) would pass a `Reading`
  check. Fixed: those two structural checks stay in `verify_chain`
  regardless of trust level; only wall-clock expiry moves to `verify`. Three
  new direct `delegation.rs` tests cover it.
- **Evidence-accuracy bug (high):** this section previously claimed ten new
  `delegation.rs` tests where the commit added none, inflating the stated
  total to 44 against an actual 34. Corrected above to a `git diff`-verified
  40 (34 pre-review-fix, plus 6 the fixes themselves added).
- **D-A1-12's `master_id`-equality regression test asserted the wrong
  failure** (an unresolvable literal DID, not the equality guard against a
  validly-signed anchor served under the wrong master). Renamed for honesty
  and paired with two new real regression tests against
  `fetch_own_master_anchor` and `resolve_master_anchor`.
- **`publish_all_services` — the recovery path D-A1-3 explicitly asked for
  its own test — had none**; the test bearing its name only called
  `build_record`. Renamed for honesty; a real sweep test against a live
  registry now proves the id-union and per-service failure containment.
- **No HTTP timeout anywhere in `RegistryClient`** (five bare
  `ReqwestClient::new()` sites), so an unresponsive registry could stall
  `deploy` for an OS-level connect timeout. Fixed: one `reqwest::Client`
  with a 10 s timeout, built once and reused.
- **`refresh_master_anchor` forced a synchronous mainline-DHT publish onto
  `roymctl`'s deploy paths.** Switched to fire-and-forget for the DHT leg;
  the HTTP publish — the guarantee D-A1-2 actually requires — stays
  synchronous.
- Six low-severity fixes: D-A1-9's tamper tests now assert both trust
  levels; the heartbeat sweep's directory scan regained its `is_file()`
  guard it had before this slice; `build_record` moved off a blocking
  `std::fs` call inside an async fn; a stale doc comment on
  `all_instance_certs`; a duplicate `RegistryClient` in `runtime.rs`
  (substrate now builds one pkarr DHT client, not two); dead/unreachable
  code in `svc.rs`'s `--master` arm; and a silent no-op when `--nickname` is
  given with nothing to sign the envelope with now warns.

**Follow-up review round (2026-07-28).** Three further comments; one was a
real hole, two were already closed by the fix-up above and are recorded here
so the disagreement is not re-litigated:

- **The new sweep test could not fail on half of what it claimed.**
  `publish_all_services` walks a `BTreeSet`, so ids run in ascending byte
  order and the deliberately-failing service was named `svc-2` — the *last*
  iteration. Every assertion still held under an implementation that aborted
  on the first error, so only the id-union half was really covered. Renamed
  to `aaa-revoked`, which sorts ahead of both `did:key:` and `svc-`, and the
  comment now records that the name is load-bearing. Verified by mutation:
  with the sweep changed to return on first error the test fails on the
  `svc-1` assertion, and passes again when reverted.
- **Test counts** were already corrected in the fix-up commit and
  independently re-verified here by counting `#[test]`/`#[tokio::test]`
  attributes added since `4e76f9d`: 35 in the slice commit plus 6 in the
  fix-up = 41, of which 1 is the e2e test, giving the **40** unit/CLI stated
  above and **3** in `delegation.rs`. No change needed.
- **The sandboxed baseline** was likewise already restated as a category
  rather than a fixed count. The follow-up round's sharper evidence — three
  isolated sandboxed reruns of the variable target giving pass, pass, fail —
  is folded into the gates section above.

**Fifth pass (2026-07-29): the design itself reopened, before merge, on an
operator question.** D-A1-2 treated "the hosting substrate signs the
record" as fixed; it is not — the deployer already holds the member master
key and can sign the record directly. That single change collapsed most of
what the first four passes built to make delegation-signed records work at
all. Full reasoning is in
[slice-a1-implementation-plan.md](slice-a1-implementation-plan.md)'s own
"fifth pass" note and its D-A1-1/2/5/6/8/10/11/14/15; ADR-0020 §6 is rewritten
to match. Summary of what's true now, replacing the "What shipped, by crate"
bullets above (kept for history, not current):

- **`crates/core/src/dht_registry.rs`:** `EndpointInfo` carries no
  `delegation` field; `EndpointInfo::sign_as_instance` and the `RecordTrust`
  enum are deleted. `SignedEndpointInfo::verify` takes no trust-level
  argument, checks the single self-signed keying shape uniformly, and returns
  the packet's own pkarr/BEP44 timestamp for the compare-and-swap below.
  `EndpointInfo` gains a required `not_after: u64` field (30-day default,
  `DEFAULT_ENDPOINT_NOT_AFTER_SECS`), checked in `verify`. `RegistryClient::
  register`'s DHT-leg refusal for a delegation-signed record is deleted —
  every record now has a DHT home.
- **`crates/core/src/endpoint_publisher.rs`:** `build_record` no longer reads
  an instance certificate, derives an instance key, or signs anything. It
  reads the stored, deployer-signed file and replays it verbatim if it still
  verifies (self-signature and `not_after`); otherwise it publishes nothing.
  `EndpointPublisher::new` drops the `EndpointRegistry`/node-identity/node-DID
  parameters it no longer needs. `publish_all_services`'s id source is the
  hosted-apps directory scan only — there is no second, certificate-derived
  source of ids to union anymore.
- **`crates/community_registry/src/registry.rs`:** `verify_endpoint_signature`
  is `payload.verify()`, full stop — the registry-local revocation check is
  deleted (nothing left to check; revocation is a handshake-only concern
  now). `register_endpoint` and `register_master_endpoint` are
  compare-and-swap on the record's/anchor's own timestamp via `DashMap::
  entry`, last-writer-wins with an explicit equal-and-identical refresh
  case — the blind `insert` this replaced accepted a rollback outright.
- **`apps/roymctl/src/commands/svc.rs`:** `--master` always signs the
  endpoint record now, unconditionally on `--nickname` — `signing_identity`
  collapses to `named_identity.as_ref().or(master_identity.as_ref())`, and
  the ephemeral-envelope-key shape for `--instance-certificate` is deleted
  (a throwaway-signed record can never verify under the master's own
  `service_id`, so it bought nothing once the substrate stopped re-signing).
- **`crates/substrate/src/runtime.rs`, `crates/coordinator_iroh/src/coordinator.rs`,
  `crates/smoke-tests`, `tests/perf`:** every remaining `EndpointInfo`
  construction site (the substrate's own self-record, the coordinator's
  self-record, smoke tests, perf-harness fixtures) threads `not_after`
  through.
- **`crates/control_plane`:** unchanged — the publish-on-deploy hook and
  `set_endpoint_publisher` wiring do not depend on who signs the record.

**Tests, `git diff` against the pre-fifth-pass tip (`d1f0eb9`):** net **-10**
unit tests (`dht_registry.rs` 17→13, `endpoint_publisher.rs` 10→5,
`community_registry/registry.rs` 17→16 — the certificate-shaped decision
table shrank along with the code), all re-derived to match the design above
rather than trimmed for count; the `delegation.rs` tests (3) and the e2e test
(1, `master_endpoint_record_e2e.rs`, substantially rewritten internally —
same name, same count) are untouched by the net change. **30 unit/CLI tests,
1 e2e**, both counted by running the suites, not asserted: `cargo test -p
syneroym-core --lib` (dht_registry.rs + endpoint_publisher.rs, 50 total in
that crate including untouched modules), `cargo test -p syneroym-community-
registry --lib` (16), `cargo test -p syneroym-substrate --test
master_endpoint_record_e2e` (1, sandbox disabled for the real port binds).
New coverage specific to the fifth pass: `verify_returns_the_packets_own_
timestamp` and `a_self_signed_record_registers_to_the_dht_with_no_http_
registry_configured` (`dht_registry.rs`); `publish_all_services_survives_a_
record_rejected_by_admission` (`community_registry/registry.rs`, proving the
compare-and-swap end to end against a live registry, including that a
rejected record does not stop the sweep from publishing the others).

**Gates, re-run 2026-07-29:**

- `cargo +nightly fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets --all-features`: clean, zero
  warnings.
- `cargo test --workspace` (sandboxed, `--no-fail-fast`): 14 failing targets,
  the identical category and count already documented above for this branch
  — real port/socket binds the sandbox denies, plus the same
  `native_dispatch_identity` DHT-actor flake. Every target in that set was
  independently re-verified passing with the sandbox disabled this pass,
  including the ones the fifth pass touched most directly:
  `master_endpoint_record_e2e` (the flagship test, full scenario including
  the relocation and the forged-record rejection), `basic_lifecycle`,
  `federated_fdae_e2e`, `community_registry --lib`, `coordinator-iroh`'s full
  suite (`multi_hop_relay`, `connection_limit`, `tls_rotation`, and the rest),
  and every other `syneroym-substrate` e2e test
  (`http_passthrough_e2e`, `instance_identity_e2e`, `messaging_client_e2e`,
  `stream_client_e2e`). No target differs from the documented baseline.
- `mise run test:e2e` (sandbox disabled, required for real port binds): 12/12
  green (8 main + 4 multi-hop), unchanged.

**Sixth pass (2026-07-29): an independent review of the fifth pass's own
commit (`89f96cf`)** found one high-severity gap the fifth pass introduced
and three smaller ones. All four fixed directly, no pushback:

- **High: `app deploy --mint-masters` published no endpoint record at all —
  the reference scenario's own primary deploy path.** The fourth-pass design
  let the substrate build a record from an installed instance certificate,
  so `map_deployment_plan_to_wit` hardcoding `registry_certificate: None`
  cost nothing; once the substrate stopped building records, that hardcoded
  `None` meant an app-deployed member's master DID could never resolve to an
  address, silently. Fixed: `substitute_and_certify_members`
  (`apps/roymctl/src/commands/member_identity.rs`) now signs an
  `EndpointInfo` per master alongside the instance certificate it already
  mints; `map_deployment_plan_to_wit` (`crates/sdk/src/mapper.rs`) gained a
  `registry_certificates` parameter mirroring `instance_certificates`
  exactly, threaded through from `app.rs`'s deploy command.
- **Medium: the DHT read path never called `verify()`**, so it never
  checked `not_after` — unreachable under the fourth-pass design (a
  delegation-signed record had no DHT home at all), exposed by this
  design's own DHT reversal. Fixed by extracting the DHT branch's
  packet-parsing into `extract_verified_endpoint_from_packet`
  (`crates/core/src/dht_registry.rs`), which now calls `verify()` before
  returning a candidate; two new unit tests exercise it directly (a signed
  packet built in-memory, no live DHT needed).
- **Medium: `not_after` had no near-expiry warning surface**, unlike the
  instance certificate's `warn_on_near_expiry_instance_certs`. Added
  `EndpointPublisher::warn_on_near_expiry_records`
  (`crates/core/src/endpoint_publisher.rs`), called from the same heartbeat
  loop that already calls `publish_all_services`, with its own fixed 7-day
  window (a record has no `issued_at` to compute a lifetime fraction from,
  unlike a certificate); three new unit tests.
- **Medium: doc accuracy** — D-A1-3 and D-A1-4 in the implementation plan
  were the only two fifth-pass decisions left without a supersession marker.
  Both rewritten; D-A1-4's obsolete decision table (keyed entirely on
  `registry.instance_cert(service_id)`, which `build_record` no longer
  reads) is now the single row the shipped code actually has.
- **Low, two precision fixes, no behavior change to one:** language in a few
  places overstated the DHT side as "compare-and-swap" (the DHT side is
  monotonic ordering via mainline's own unconditional sequence-number
  rejection, `dht.publish(&packet, None)` — no `cas` requested; the HTTP
  registry's `admit_endpoint` is the one genuine compare-and-swap) —
  corrected in the two summary passages that conflated them. And: a stored
  file that fails to *parse* (rather than fails to *verify*) used to be
  silently indistinguishable from a missing file
  (`build_record`'s `.ok()` chain); now reads, then parses, as two separate
  steps, each warning distinctly on failure, with a `NotFound` read error
  the sole silent case (the normal, common state for a service deployed
  without `--identity`/`--master`).

**Gates, re-run 2026-07-29 after the sixth pass:** `cargo +nightly fmt --all
-- --check` clean; `cargo clippy --workspace --all-targets --all-features`
clean, zero warnings; `cargo test -p syneroym-core -p
syneroym-community-registry -p roymctl -p syneroym-sdk -p
syneroym-control-plane` (sandbox disabled) all green, including 6 new unit
tests (2 in `dht_registry.rs`, 4 in `endpoint_publisher.rs`); `cargo test -p
syneroym-substrate` (sandbox disabled) all green, including the flagship
`master_endpoint_record_e2e` and the previously-unaffected
`a_certificate_near_expiry_is_warned_about_on_the_heartbeat_sweep` (proving
the new near-expiry sweep call didn't disturb the existing one); `mise run
test:e2e` 12/12, unchanged.

## A2 — Verification evidence (2026-07-29)

Planning found seven places where `task.md`/ADR-0021 described a tree that
does not exist or left an open choice — recorded in
[slice-a2-implementation-plan.md](slice-a2-implementation-plan.md) §0's seven
numbered findings (two review rounds, all incorporated before implementation
started), most consequentially: `StaticInventory` had **no callers at all**
anywhere in the workspace (not "no real ones" — `task.md`'s own phrasing
overstated how much already existed), a cross-app `Bind` dependency has no
manifest surface to be named through, and `{member_master_did,
expected_asserter_did}` were one value written twice rather than two real
fields. §1's fourteen decisions (D-A2-1 … D-A2-16) took those findings as
given; ADR-0020/ADR-0021 still need a dated sign-off amendment covering all
of §0, not yet written into the ADRs themselves (tracked as this section's
own follow-up, not a separate backlog row).

**What shipped**, by phase:

- **Phase 3 (relationship-proof trust chain, shipped independently first
  per the plan's suggested merge order):** `RelationshipProof` gains an
  optional `delegation` field (a JSON `DelegationCertificate`) inside the
  signed payload; `sign` asserts under the certificate's master when one is
  given, falling back to the pre-ADR-0020 self-asserted shape otherwise;
  `verify` checks the certificate with `DelegationCertificate::verify`
  (live-credential strictness, not A1's reader-level `verify_chain`) and a
  fixed `[SCOPE_SERVICE_INSTANCE]` accepted-scope set before ever checking
  the outer signature (`crates/rpc/src/relationship_proof.rs`).
  `SynSvcNativeService` carries an `instance_cert: Option<DelegationCertificate>`
  field, populated from `deploy`'s already-four-times-verified certificate
  (no second load or re-verification), and forwards it into every
  `sign_relationship_proof` call (`crates/control_plane/src/synsvc_native.rs`).
- **Phase 4 (transport half):** `CallOrigin::Native` gains `service_id:
  Option<String>` (`crates/rpc/src/proxy.rs`); `ProxyRouter::invoke_remote_at`
  gains a `(None, Native { service_id: Some(sid) })` arm presenting that
  service's certified instance key (falling back to the node identity, not
  anonymous, on an absent/expired certificate — a substrate-internal call has
  always presented *something*), leaving the `(Some(proof), Native)` arm's
  verbatim-forwarding behavior untouched per D-B3-9
  (`crates/router/src/proxy.rs`). `resolve_fetches`/`resolve_one_fetch` gain a
  `local_service_id: &str` parameter threaded from
  `sandbox_wasm::host_capabilities` and `synsvc_native.rs` so the FDAE
  relationship-proof fetch travels as the asking service, not the node.
- **Phase 0 (registry ownership, persistence, startup replay — no behavior
  change on its own):** `EndpointStorage` gains five methods
  (`{load_all,save,remove}_app_context`-shaped plus `save_binding`/
  `load_all_bindings`), implemented across all four backends (`MockStorage`,
  `SqliteEndpointStorage` with two new tables created unconditionally per
  D-A0-10's precedent, and the two test doubles in
  `control_plane/src/service/orchestration.rs` and
  `router/tests/service_ownership.rs`). `EndpointRegistry` gains the
  in-memory mirror plus `set_app_context`/`app_context_of`/
  `remove_app_context`/`save_binding`/`all_bindings`
  (`crates/core/src/local_registry.rs`). `LogicalResolver::register` (new,
  `crates/app_orchestration/src/resolver.rs`) makes "write the registry, evict
  the cache" one atomic step — the module's own doc comment previously
  overclaimed that a cache **hit** compares epochs against the registry; it
  does not, so this method is the only thing that keeps a scale-out visible
  before `cache_ttl` elapses, which the milestone's 5s convergence budget
  depends on before the supervisor is even written. `AppSandboxEngine` and
  `ControlPlaneService` both gain a `logical_resolver: Arc<LogicalResolver>`
  field/constructor parameter, sharing one `StaticInventory` over one
  `EndpointRegistry` (`crates/sandbox_wasm/src/engine.rs`,
  `crates/control_plane/src/service.rs`). `crates/substrate/src/runtime.rs`'s
  composition root replays every persisted binding into a fresh
  `StaticInventory` before the router starts accepting connections
  (`replay_persisted_bindings`, extracted as its own function so it is
  unit-testable without a full composition-root harness), warning and
  skipping a row that fails to parse rather than panicking substrate startup
  (D-A2-15). Four crates (`sandbox_wasm`, `substrate`, `router` dev-deps,
  `coordinator_iroh` dev-deps) gained a `syneroym-app-orchestration`
  dependency; a `#[doc(hidden)] empty_resolver()` helper was added to
  `app_orchestration` so the **56** `AppSandboxEngine::init` (the plan's own
  tally of 55, plus one more this slice's own new
  `router/tests/proxy_dispatch.rs` dependency-resolution test added) and
  **37** `ControlPlaneService::init` test call sites, both counted directly
  by `grep`, didn't need 93 copies of
  `Arc::new(LogicalResolver::new(Arc::new(StaticInventory::new())))`.
- **Phase 1 (bindings on the wire):** `control-plane.wit` gains
  `topology-mode`, `dependency-binding`, and `app-context` records, plus one
  `option<app-context>` field on `planned-service` — **not** on
  `deploy-manifest` (D-A2-6: a 4-literal-site cost against 71, and the
  correctness reason that `deploy-manifest` is a full-reinstall carrier
  while `planned-service.app-context` is deliberately the *initial-deploy*
  carrier only, never A5's push channel). `PlannedService.resolved_dependencies`
  changes from `Vec<ServiceId>` to `BTreeMap<LogicalServiceName, Vec<ServiceId>>`
  (`crates/app_orchestration/src/models.rs`), keyed by declared name so a
  binding can carry the name the guest will actually ask for; `compiler.rs`
  builds the map, `journal.rs`/`reconcile.rs`/`sdk/mapper.rs`'s test fixtures
  updated to match. `syneroym_sdk::mapper::map_deployment_plan_to_wit` gains
  an `emit_bindings: bool` parameter and builds one `dependency-binding` per
  `depends_on` entry, reading the **target's** topology mode (not the
  dependent's — every service in the pre-A2 tree happened to be `Singleton`,
  which would have hidden this bug until the first `Redundant` service);
  `emit_bindings: false` (the non-`--mint-masters` path) publishes an empty
  binding list rather than the compiler's fabricated `did:key:h...` ids
  (D-A2-16) — publishing those would let `dependency(...)` resolve and then
  fail one layer down as `service-not-found`, destroying the distinction
  `dependency-not-bound` exists to draw. `apps/roymctl/src/commands/app.rs`
  passes `emit_bindings: *mint_masters` and prints a warning when deploying
  a `depends_on`-declaring manifest without `--mint-masters`.
  `member_identity.rs`'s `substitute_and_certify_members` substitutes the
  map's **values** (the member DIDs), preserving the dependency names as
  keys, so a binding never carries the compiler's fabricated ids once
  `--mint-masters` runs. `ControlPlaneService::deploy` is split into a thin
  trait-facing wrapper and an inherent `deploy_with_context` carrying the
  entire original body plus the binding-write block (D-A2-6's naming);
  `deploy_plan` calls `deploy_with_context` directly with
  `service.app_context`. The binding-write block validates every
  caller-supplied string with `try_new`, never `new` (D-A2-15 — `new` panics
  on a bad value, which would let an authorized-but-buggy deploy caller kill
  the control-plane task), writes the app context and every binding through
  `EndpointRegistry`, and registers each one through
  `LogicalResolver::register` in the same step so the write and the cache
  eviction can never be separated. `undeploy`'s teardown gains a
  `remove_app_context` call (persisted rows only; the in-memory
  `StaticInventory` entry stays per D-A2-9).
- **Phase 2 (guest names a dependency, host resolves):** `syneroym:proxy/proxy`
  gains a `call-target` variant (`service`/`dependency`) replacing the bare
  `service: string` parameter, a `dependency-not-bound` error variant, and
  `call-options.routing-key: option<string>`
  (`crates/wit_interfaces/wit/proxy/proxy.wit` — the single real file behind
  the two committed symlinks, confirmed via `git ls-files -s`).
  `HostState` gains `app_instance_id: Option<String>` and
  `logical_resolver: Arc<LogicalResolver>`; `proxy::Host::call`'s
  `CallTarget::Dependency` arm resolves host-side, before a `ProxyRequest`
  ever exists, so a guest never holds the resolved DID and cannot snapshot
  it past a re-push — `LogicalResolver::resolve` is synchronous and
  lock-free on a cache hit, so this adds no `.await` and keeps the
  no-network-hop budget true by construction
  (`crates/sandbox_wasm/src/host_capabilities.rs`). The one WIT-visible
  interface change meant rebuilding the one fixture that imports it:
  `test-components/proxy-test` (`call-peer` gains a `target-kind` argument
  selecting which variant it builds) — confirmed building clean for
  `wasm32-wasip2`, along with `greeter` and `data-layer-test` (neither
  imports `syneroym:proxy/proxy`, verified by `grep -rln proxy
  test-components/`).

**Tests added: 34 new unit/CLI/integration tests, counted directly from `git
diff main` by counting `#[test]`/`#[tokio::test]` attributes added, not
asserted** — 6 in `crates/rpc/src/relationship_proof.rs` (delegation-carrying
sign/verify: master-vs-instance-DID, a certificate naming a different master
than the proof claims, an expired certificate, a routing-scoped certificate,
a tampered delegation field breaking the outer signature); 4 in
`crates/router/src/proxy.rs` (the transport half: presents the service's
instance key on its behalf, forwards an existing caller proof verbatim
per D-B3-9, falls back to the node identity with no `service_id` and on an
expired certificate); 1 in `crates/app_orchestration/src/resolver.rs`
(`register_through_the_resolver_evicts_the_cached_topology`, pinning the
§3.4 landmine a plain `AppRegistry::register` would have reintroduced); 5 in
`crates/data_db/src/registry_store.rs` (the two new tables created on a
database that predates them — the D-A0-10 regression one table over — plus
upsert/removal semantics); 3 in `crates/sdk/src/mapper.rs` (one binding per
`depends_on` entry with the target's mode, an empty binding list for a
dependency-free plan, and `emit_bindings_false_publishes_no_fabricated_member_dids`
pinning D-A2-16); 7 in `crates/control_plane/src/service/orchestration.rs` (a
deploy carrying an app context registers a resolvable binding, a redeploy
dropping a dependency leaves no stale row, an invalid member DID/dependency
name/app instance id each fail the deploy with `try_new`'s `Err` rather than
`new`'s panic, undeploy clears the persisted rows); 1 in
`crates/substrate/src/runtime.rs`
(`an_unreadable_persisted_binding_is_skipped_not_fatal`, a stored row with a
`/` and a row with unparseable JSON both warn-and-skip while a well-formed
row alongside them still replays); 6 in
`crates/sandbox_wasm/src/host_capabilities.rs` (dependency resolution before
the `ProxyRequest` is built, `dependency-not-bound` for an unbound name and
for a standalone component with no app context, a raw-DID target unchanged,
deterministic routing-key selection across a two-member binding, and a
dependency that resolves to the component's own service still forwarding
the real caller); 1 in `crates/router/tests/proxy_dispatch.rs`
(`guest_dependency_target_reaches_the_bound_member_and_a_re_registration_takes_effect_on_the_next_call`,
a **real WASM guest** driving `call-peer` with `target-kind = "dependency"`
through a live `RouteHandler` composition, then re-registering the same
declared name onto a different, nonexistent target and proving the very
next call reaches it — the guest never held a snapshot of the old
resolution).

**Not covered by unit/single-node-integration tests — recorded as a backlog
row, not silently dropped** (see
[deferred-backlog.md](../../deferred-backlog.md) §3 "A2's own two-substrate
e2e coverage is partial"): a `dependency_binding_e2e.rs` two-real-substrate
test (modelled on `master_endpoint_record_e2e.rs`) proving the reference
scenario's step 4 from the dependent's side, and `federated_fdae_e2e.rs`'s
extension so the responding service holds an instance certificate and its
`RelationshipProof` verifies against the member master over a real Iroh
hop (failure-matrix row 19, live). Both are real two-substrate harness
builds; the underlying mechanisms they would prove live are fully covered
at the unit level (delegation-carrying `RelationshipProof`, host-side
resolution, cache-eviction-on-write) and at the single-node integration
level (the real-WASM-guest test above).

**Gates, run 2026-07-29:**

- `cargo +nightly fmt --all`: clean.
- `cargo clippy --workspace --all-targets --all-features`: clean, zero
  warnings.
- `cargo test --workspace --no-fail-fast` (sandboxed; the plain
  `cargo test --workspace` form exits 101 and stops at the first failing
  crate, so `--no-fail-fast` is required to see the rest of the suite, not
  optional detail): every target that binds a real socket or starts the
  mainline DHT actor fails outright, all in the same pre-existing,
  environmental category documented throughout this milestone's status —
  `syneroym-community-registry --lib`, `syneroym-coordinator-iroh`
  `connection_limit`/`multi_hop_relay`/`tls_rotation`, `syneroym-mqtt-broker
  --lib`, `syneroym-sdk --test connect_timeout`, every `syneroym-substrate`
  e2e test (`basic_lifecycle`, `federated_fdae_e2e`, `http_passthrough_e2e`,
  `instance_identity_e2e`, `master_endpoint_record_e2e`,
  `messaging_client_e2e`, `stream_client_e2e`), plus whichever of
  `syneroym-router --test native_dispatch_identity` /
  `syneroym-router --test proxy_dispatch` happen to start the mainline DHT
  actor under sandboxed load that run (`.../mainline-6.2.0/src/dht.rs:143`
  panics, "actor thread unexpectedly shutdown" — the same bind-denial
  category as the rest, not an A2 defect): **14-15 failing targets observed
  across runs, count drifting with machine load** — a pinned number here
  would itself be a stale, brittle claim, so this is deliberately a category
  description instead. Every target in the category was independently
  re-verified passing with the sandbox disabled this pass (`cargo test -p
  syneroym-substrate -p syneroym-community-registry -p syneroym-coordinator-iroh
  -p syneroym-sdk -p syneroym-mqtt-broker -p syneroym-router`, all green,
  including `federated_fdae_e2e` at 72s, the other e2e tests at 10-56s each,
  and `proxy_dispatch` 8/8 on three consecutive solo runs) — no target
  outside this documented sandbox-bind category differs from expected.
- `mise run test:e2e` (sandbox disabled, required for real port binds):
  12/12 green (8 main + 4 multi-hop), unchanged from before this slice.
- `wasm32-wasip2`: `greeter`, `data-layer-test`, and `proxy-test` all build
  clean; `proxy-test` is the one fixture whose WIT import changed
  (`call-peer` gained a `target-kind` argument), rebuilt and confirmed.

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
