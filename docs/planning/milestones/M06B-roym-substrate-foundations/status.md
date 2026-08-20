# M06B Roym Substrate Foundations — Status

**Milestone:** [task.md](task.md) · **Designs of record:**
[slice-b1-implementation-plan.md](slice-b1-implementation-plan.md) (B1),
[slice-b2-implementation-plan.md](slice-b2-implementation-plan.md) (B2),
[slice-b3-implementation-plan.md](slice-b3-implementation-plan.md) (B3)

**Overall:** Slices B1, B2, and B3 complete (2026-08-20). Slice B4a complete (2026-08-20); B4b (real X3DH + Double Ratchet) not started — see B4's own status below.

---

## Slice status

| Slice | Scope | Status | Gate |
|---|---|---|---|
| B1 | Person identity at the client gateway (G3) | **Complete (2026-08-18)** — [implementation plan](slice-b1-implementation-plan.md), evidence below | None — independently mergeable |
| B2 | Declared service visibility (G4) | **Complete (2026-08-18)** — [implementation plan](slice-b2-implementation-plan.md), evidence below | None (ADR-0018 Accepted) |
| B3 | The dual-build shim (D2/D3) | **Complete (2026-08-20)** — [implementation plan](slice-b3-implementation-plan.md), evidence below | None |
| B4 | Durable messaging: interface and 1:1 delivery (G1 part 1, G2) | **B4a Complete (2026-08-20)** — [implementation plan](slice-b4-implementation-plan.md), evidence below. **B4b (real crypto) not started.** | None for B4a — B3 met |
| B5 | Group delivery (G1 part 2) | Pending | B4b |

---

## B1 — What shipped

Slice B1 enables local person identity at the client gateway, binding an authenticated local client to a person's DID under an owner→node delegation certificate instead of presenting the node's fallback self-asserted DID.

### 1. Protocol Constants & Assertions (`crates/core/src/protocol_utils.rs`)
- `GATEWAY_RESERVED_PATH_PREFIX = "/_syneroym/"` (D-B1-11)
- `SESSION_COOKIE_NAME = "syneroym_session"` (D-B1-6)
- `gateway_session_assertion(node_did, nonce, person_did)`: canonical JSON assertion payload `{"typ": "syneroym:gateway-session-assertion:v1", "node_did":..., "nonce":..., "person_did":...}` (D-B1-1)

### 2. Configuration & Anchor Lookup (`crates/core/src/config.rs`, `crates/core/src/dht_registry.rs`, `crates/router/src/handshake.rs`)
- Added `session_ttl_secs` to `ClientGatewayRole` (default 28,800s / 8 hours, configurable, D-B1-8).
- Defined `MasterAnchorResolver` trait in `syneroym-core` with async `resolve_master_anchor(&self, master_did: &str) -> Result<MasterAnchorPayload>` (D-B1-14).
- Implemented `MasterAnchorResolver` for `RegistryClient` (DHT + HTTP registry fallback). Re-exported from `syneroym-router::handshake`.

### 3. Session Store & Reserved Routing (`crates/client_gateway/src/session.rs`, `crates/client_gateway/src/gateway.rs`)
- Implemented `SessionStore` with bounded DashMap storage (`MAX_ACTIVE_SESSIONS = 64`, `MAX_PENDING_CHALLENGES = 64`), TTL expiration, oldest-expiry eviction, and single-use challenge nonces (`NONCE_TTL_SECS = 60`, D-B1-8).
- Implemented challenge/login/logout/whoami lifecycle under reserved `/_syneroym/session/*` routes (D-B1-3, D-B1-11).
- Implemented credential extraction (`extract_credential`) prioritizing `Cookie` over `Authorization: Bearer` (D-B1-7).
- Implemented credential stripping (`strip_credential`) that unconditionally removes `syneroym_session` pairs from `Cookie:` headers and drops matching `Authorization: Bearer <session-token>` lines, preserving unrelated authentication headers (D-B1-7).
- Added 10-second timeout on reading HTTP request headers and request body in `handle_connection`.
- Client Gateway routes requests under `/_syneroym/` to internal session handlers and blocks them from proxying to guest services (D-B1-11).
- Forwarded active `DelegationCertificate` in `SyneroymClient::passthrough_with_conn` to populate preamble delegation.

### 4. Control Plane Route & Asset Collision Guard (`crates/control_plane/src/http_routes.rs`, `crates/control_plane/src/assets.rs`)
- Reject any guest HTTP route or static asset bundle containing a path starting with `/_syneroym/` at deploy time (validation error from `parse_http_routes` / `unpack_asset_bundle` surfaced as a deploy failure, D-B1-13).

### 5. Roymctl CLI Session Commands (`apps/roymctl/src/commands/session.rs`)
- Added `roymctl session login`, `roymctl session status`, `roymctl session token`, `roymctl session logout` (D-B1-9).
- Saves session token locally under `<config-dir>/sessions/<sanitized-gateway-url>.json` with secure file permissions (0600 on Unix, D-B1-15).

---

## B1 — Verification Evidence

### Automated Unit and Integration Tests

1. **Client Gateway Unit Tests** (`crates/client_gateway/src/session.rs`):
   - `test_session_login_lookup_success`: Verifies complete login and lookup flow.
   - `test_session_nonce_single_use`: Verifies nonces cannot be reused.
   - `test_session_expired_nonce`: Rejects login after nonce TTL expires.
   - `test_session_bad_signature`: Rejects forged assertion signatures.
   - `test_session_wrong_delegate`: Rejects delegations targeting a different node.
   - `test_session_bad_delegation_wrong_scope`: Rejects delegations without `routing` scope.
   - `test_session_bad_delegation_mismatched_master`: Rejects delegations signed by a non-master key.
   - `test_session_delegation_expired`: Rejects expired delegation certificates.
   - `test_session_anchor_unresolvable`: Refuses login with 409 when master anchor is unresolvable.
   - `test_session_expiry_clamped_to_min`: Verifies session expiry is `min(session_ttl, cert_expiry)`.
   - `test_session_lookup_expired`: Expired session lookup returns `None` and purges entry.
   - `test_max_pending_challenges_bound`: Verifies challenge store capacity bound.
   - `test_max_active_sessions_eviction`: Verifies active session capacity bound and oldest-expiry eviction.
   - `test_extract_credential`: Verifies Cookie priority over Bearer, empty cookie fallback, and UTF-8 safety.
   - `test_strip_credential`: Verifies unconditional session cookie stripping, bearer stripping, and non-UTF-8 header preservation.
   - `test_classify`: Verifies routing classification for proxy and session endpoints.

2. **Control Plane Reserved Prefix Unit Tests**:
   - `crates/control_plane/src/http_routes.rs`: `reserved_gateway_prefix_path_is_rejected` passes.
   - `crates/control_plane/src/assets.rs`: `asset_under_reserved_gateway_prefix_is_rejected` passes.

3. **Substrate E2E Integration Tests** (`crates/substrate/tests/gateway_session_e2e.rs`):
   - `test_16_anonymous_request_sees_self_asserted_node_did`: Anonymous request through gateway sees self-asserted node DID.
   - `test_17_logged_in_request_via_cookie_sees_delegated_person_did`: Session cookie request sees verified person DID with delegated auth.
   - `test_18_two_people_logged_in_each_whoami_and_echo_returns_own_did`: Two people on one node: each `/whoami` and `/echo` returns own DID.
   - `test_19_second_local_process_without_token_while_session_live_sees_self_asserted`: Second local process with no token sees self-asserted node DID.
   - `test_20_forged_login_is_rejected_with_401`: Forged login signature is rejected with HTTP 401.
   - `test_21_gateway_session_token_is_stripped_from_proxied_headers`: Session cookie/bearer is stripped from proxied headers; non-session auth is preserved.
   - `test_22_cookie_takes_priority_over_bearer`: Cookie takes priority over Bearer when both are present.
   - `test_23_login_with_no_published_anchor_is_refused_with_409`: Login with unresolvable anchor fails with HTTP 409.
   - `test_24_expect_100_continue_handshake`: Expect: 100-continue completes over raw TCP socket.
   - `test_25_expired_gateway_session_falls_back_to_self_asserted_node_did`: Expired session falls back to self-asserted node DID with credentials stripped.
   - `test_26_restart_clears_all_sessions`: Substrate restart clears in-memory session store.
   - `test_27_reserved_path_challenge_login_whoami_logout_lifecycle`: Full challenge/login/whoami/logout lifecycle over HTTP gateway.
   - `test_28_reserved_path_is_never_proxied_to_guest`: Reserved path `/_syneroym/*` returns 404 from gateway and is never proxied to guest.
   - `test_29_roymctl_session_cli_lifecycle`: `roymctl session` login, status, token, logout lifecycle, file existence, and 0600 permissions.
   - `test_30_roymctl_session_cli_error_handling`: `roymctl session` missing `--as` flag, nonexistent identity, and unresolvable anchor error handling.
   - `test_31_oversized_and_invalid_http_requests_return_400`: Oversized headers (>8KB) and invalid HTTP requests return HTTP 400.

### Run Evidence

```
$ cargo test -p syneroym-client-gateway
running 20 tests
test gateway::tests::a_gateway_with_neither_credential_warns_at_init_naming_both_config_keys ... ok
test session::tests::test_classify ... ok
test session::tests::test_extract_credential ... ok
test gateway::tests::resolve_target_passes_an_unscoped_host_through_unresolved ... ok
test gateway::tests::resolve_target_routes_an_app_scoped_host_through_the_resolver_and_surfaces_its_error ... ok
test session::tests::test_session_anchor_unresolvable ... ok
test gateway::tests::parsing_an_unscoped_host_yields_the_expected_alias_and_interface ... ok
test session::tests::test_max_pending_challenges_bound ... ok
test session::tests::test_session_bad_delegation_wrong_scope ... ok
test session::tests::test_session_bad_delegation_mismatched_master ... ok
test session::tests::test_session_bad_signature ... ok
test session::tests::test_session_expired_nonce ... ok
test session::tests::test_session_nonce_single_use ... ok
test session::tests::test_session_login_lookup_success ... ok
test session::tests::test_session_lookup_expired ... ok
test session::tests::test_strip_credential ... ok
test session::tests::test_session_wrong_delegate ... ok
test session::tests::test_session_expiry_clamped_to_min ... ok
test session::tests::test_max_active_sessions_eviction ... ok
test session::tests::test_session_delegation_expired ... ok
test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.11s

$ cargo test -p syneroym-control-plane
running 219 tests
test result: ok. 219 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.92s

$ cargo test -p syneroym-substrate --test gateway_session_e2e
running 16 tests
test test_16_anonymous_request_sees_self_asserted_node_did ... ok
test test_17_logged_in_request_via_cookie_sees_delegated_person_did ... ok
test test_18_two_people_logged_in_each_whoami_and_echo_returns_own_did ... ok
test test_19_second_local_process_without_token_while_session_live_sees_self_asserted ... ok
test test_20_forged_login_is_rejected_with_401 ... ok
test test_21_gateway_session_token_is_stripped_from_proxied_headers ... ok
test test_22_cookie_takes_priority_over_bearer ... ok
test test_23_login_with_no_published_anchor_is_refused_with_409 ... ok
test test_24_expect_100_continue_handshake ... ok
test test_25_expired_gateway_session_falls_back_to_self_asserted_node_did ... ok
test test_26_restart_clears_all_sessions ... ok
test test_27_reserved_path_challenge_login_whoami_logout_lifecycle ... ok
test test_28_reserved_path_is_never_proxied_to_guest ... ok
test test_29_roymctl_session_cli_lifecycle ... ok
test test_30_roymctl_session_cli_error_handling ... ok
test test_31_oversized_and_invalid_http_requests_return_400 ... ok
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 146.46s
```

---

## B2 — What shipped

Slice B2 implements both halves of declared service visibility (G4): ADR-0018's
publication declaration (`ServiceConfig.visibility`, both layers) and ADR-0022
§5's per-logical-service topology-resolution declaration
(`topology_visibility`), so publication and cross-installation resolution are
both explicit statements of intent instead of accidents of which flag was
passed or which grant an operator happened to install.

### 1. The declarations (`crates/app_orchestration/src/models.rs`, `crates/wit_interfaces/wit/control-plane/control-plane.wit`)
- `ServiceConfig.visibility: Visibility` (`public`/`internal`/`private`, default `private`) — the field ADR-0018 is actually about; the enum itself shipped with M06A A1.
- New `TopologyVisibility` enum (`restricted`/`open`, default `restricted`) on `ServiceSpec` and `PlannedService` (ADR-0022 §5) — binary by construction, since a topology document is never registered anywhere and §5 forbids a filtered member list.
- WIT `service-config.visibility` and `deployed-service.visibility` (two additive fields, no new enum, no new package).
- `sdk::mapper` maps both `ServiceConfig.visibility` and `AssetBundle.visibility` through one shared `map_visibility` function.

### 2. Substrate-side validation and storage (`crates/control_plane/src/service/orchestration.rs`, `crates/core/src/storage.rs`, `crates/core/src/local_registry.rs`, `crates/data_db/src/registry_store.rs`)
- `validate_publication`: refuses a deploy on any of the four ADR-0018 §4 mismatches — `public`/`internal` with no certificate; `private` with a certificate; a certificate whose `is_private` disagrees with the declaration; a certificate naming a different `service_id`.
- A `private` redeploy deletes any previously stored endpoint-record file, so the heartbeat sweep stops republishing it (`D-B2-5`, the defect F2 found).
- `DeployFacts` widened to a 4-tuple (`service_type`, `health_check_json`, `manifest_hash`, `visibility`); `service_deploy_facts` gains a `visibility` column via an idempotent `ALTER TABLE`. Reported by `DeployedService.visibility` and, from there, `orchestrator/list` / `roymctl svc list`.
- One `info!` at deploy naming the declared visibility.

### 3. The client side (`crates/sdk/src/deploy.rs`, `crates/sdk/src/lib.rs`, `apps/roymctl/src/commands/svc.rs`)
- `certify_placed_members` mints no endpoint record for a member declaring `private`; `internal`/`public` members get one with `is_private` matching the declaration (`D-B2-7` — this is where "undeclared = unpublished" becomes true for the app-deploy path, F1).
- `SyneroymClient`'s `Publication` enum (`Private`/`Public(record)`/`Internal(record)`) replaces the old `Option<SignedEndpointInfo>` on `DeploySvcOptions`/`deploy_svc_wasm`/`deploy_svc_tcp`/`deploy_container` — the three legal pairings are the only ones a caller can express.
- `SyneroymClient::new_with_record` (ADR-0018 §2): connects using a privately-shared, verified `SignedEndpointInfo`.
- `roymctl svc deploy` gains `--visibility <public|internal|private>` (default `private`) and `--record-out <path>`; validates `--identity`'s DID against `--svc-id` before building anything; `svc list` gains a `VISIBILITY` column.

### 4. Plan-level contradiction checks (`crates/app_orchestration/src/compiler.rs`, `crates/app_supervisor/src/service.rs`)
- `validate_plan_visibility` (`D-B2-14`), called from both `compile()` and `handle_submit`: refuses (a) a cross-substrate `depends_on` a `private` member, and (b) `topology_visibility = open` paired with `visibility = private` (F13's one real mistake — a caller would receive a signed member list with nothing it can dial).

### 5. Resolution: "open to all" (`crates/app_supervisor/src/topology.rs`, `crates/app_supervisor/src/service.rs`, `crates/core/src/config.rs`, `crates/sdk/src/topology.rs`, `crates/client_gateway/src/gateway.rs`, `crates/coordinator_webrtc/src/coordinator.rs`)
- `topology::service_topology_visibility` reads one logical service's declared posture from the plan, refusing (to `Restricted`) on a compiler-defect disagreement between members.
- `handle_resolve` reads it between the stored-plan read and the capability check: an `open` service answers any verified caller with no capability; every other refusal (unknown app, retired instance, unknown service, `restricted` service) stays indistinguishable, and the document served is byte-identical either way.
- `[iam].grant_resolve_to_node_did`'s doc, and the client gateway's / WebRTC coordinator's credential-warning text, both updated to name `open` as the third way in.

### 6. DHT publication respects `is_private` (`crates/core/src/dht_registry.rs`)
- `RegistryClient::register` now gates Mainline DHT publication on `!is_private` (`D-B2-16`) — before this, an `internal` record was globally resolvable on any node with `enable_bep0044_dht` on, the opposite of what the tier declares.
- `register` now reports failure when no channel actually published (`D-B2-17`), rather than returning `Ok(())` on the strength of a DHT arm that `D-B2-16` just made conditional.

---

## B2 — Verification Evidence

### Automated Unit Tests

- **`syneroym-app-orchestration`** (`compiler.rs`, `models.rs`): `validate_plan_visibility` — 13 tests covering both checks ((a) cross-substrate private dependency, (b) `open`+`private`), including the conservative `None`/partial-placement cases that must **not** false-positive.
- **`syneroym-sdk`** (`deploy.rs`, `lib.rs`): `member_registry_record` — the visibility -> record decision `certify_placed_members` and the `multi_substrate_placement_e2e.rs` harness both call — mints no record for `private` and sets `is_private` correctly for `internal`/`public`, each record's own signature verified, not just its shape (plan tests 11/12); `Publication::split` for all three variants; `SyneroymClient::new_with_record` verifies the signature and rejects a tampered record.
- **`syneroym-control-plane`** (`service/orchestration.rs`): `validate_publication` — 10 table-driven cases (§7 tests 1–10); `a_private_redeploy_removes_the_stored_endpoint_record_file` (test 36, direct access to `hosted_apps_dir`); `is_safe_service_id_for_path_rejects_traversal_and_admits_a_real_did` — the path-traversal guard both `deploy_with_context` and `undeploy_impl` call before joining `service_id` into a stored-record filename.
- **`syneroym-app-supervisor`** (`topology.rs`, `service.rs`): `service_topology_visibility` (declared/disagreement/short-hash); `handle_resolve` tests 24/26/27/28/29 (open bypass, non-existent service, retired instance, granted-caller `InvalidParams` unaffected, byte-identical document); `submit_is_refused_when_the_plan_declares_open_topology_over_a_private_service` (test 20, the `handle_submit` backstop entry point for `D-B2-14`); `resolve_open_service_over_a_private_member_still_serves_the_document` (test 44, F13's `(open, private)` row — a plan built directly, bypassing both of `D-B2-14`'s refusal points, the same way `adopted_instance` always does).
- **`syneroym-core`** (`dht_registry.rs`): `test_dht_publication_skipped_for_private_record` (test 32, two of its three cases — the public-record and private-record-refused HTTP-less cases).
- **`syneroym-community-registry`** (`registry.rs`): test 32's third case, `an_internal_record_registers_to_a_live_registry_with_the_dht_enabled_and_is_retrievable` — a live HTTP registry, not just the `Err`-on-no-channel case, since `syneroym-core` cannot depend on `syneroym-community-registry` to spin one up itself; `a_public_record_propagates_to_the_parent_registry_while_an_internal_one_does_not` (test 39, pairing with test 38 to prove the two published tiers stay distinguishable end to end).
- **`syneroym-data-db`** (`registry_store.rs`): `visibility` round-trips through `save_deploy_facts`/`load_all_deploy_facts`; a pre-existing row without the column loads as `None`.
- **`roymctl`** (`commands/svc.rs`): `signed_export_record` — the function both the private-with-`--record-out` and public/internal deploy arms call to build the record they sign — sets `is_private` correctly for all three visibilities; `a_record_out_file_round_trips_through_new_with_record` (test 45) — writes a record built by that same function to a temp file the way `--record-out` does and reads it back into `new_with_record`, since `svc deploy` itself cannot be driven from a test.

### Integration / E2E Tests

- **`crates/substrate/tests/service_visibility_e2e.rs`** (new file, tests 33–36, 40): undeclared visibility publishes nothing; declaring `public` with no certificate is refused; `public` with a matching certificate publishes and resolves; a `public`→`private` redeploy is recorded as private; `orchestrator/list` distinguishes a private service from a public one.
- **`crates/substrate/tests/multi_substrate_placement_e2e.rs`** (new test, test 37): an app deployed with no visibility declaration publishes no member records, and a cross-node dial for a member fails to resolve.
- **`crates/substrate/tests/topology_document_e2e.rs`** (new tests, tests 41–42): an outside caller with no UCAN grant resolves an `open` app's members; the same caller, against **one instance with two logical services** declaring different `topology_visibility`, gets a different answer for each (`an_outside_caller_gets_a_different_answer_per_logical_service_in_one_instance`, replacing an earlier version of test 42 that used two separate app instances and only re-proved that a `restricted` app refuses).
- **`crates/substrate/tests/gateway_hostname_e2e.rs`** (new tests, test 43 and its negative half): a client gateway with neither credential resolves and proxies an `open` logical hostname, and still refuses a `restricted` one.
- **F10's fixture repairs** (group A/B/C): `master_endpoint_record_e2e.rs`, `multi_substrate_placement_e2e.rs`, `binding_push_e2e.rs`, `reference_scenario_e2e.rs`, `durable_outbox_e2e.rs`, `gateway_hostname_e2e.rs`, `topology_document_e2e.rs` all updated to declare `visibility = "internal"` where cross-node resolution is exercised, with matching `is_private` on their signed records.
- **One F10-group-C fixture the plan's own enumeration missed, found by a full workspace run**: `scheduled_task_e2e.rs`'s `worker` service is placed on a different substrate (`MANAGED_ALIAS`) from its supervisor and has no `depends_on`, so neither of F10's two grep patterns (`PlacementSelector::Substrate` + `depends_on`, or a direct member-DID dial) caught it — but the supervisor's own scheduled-tick push dials the worker by DID through the registry on every cron tick, which is exactly F10 group C's shape. Declaring `visibility = "internal"` fixes it, and both of that file's tests pass individually and together (309s total).

### Run Evidence

```
$ cargo test -p syneroym-app-orchestration --lib
test result: ok. 164 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p syneroym-sdk --lib
test result: ok. 69 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p syneroym-control-plane --lib
test result: ok. 231 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p syneroym-app-supervisor --lib
test result: ok. 296 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p syneroym-core --lib
test result: ok. 90 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p syneroym-data-db --lib
test result: ok. 180 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p syneroym-community-registry --lib
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p roymctl --lib
test result: ok. 73 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p syneroym-substrate --test service_visibility_e2e
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p syneroym-substrate --test multi_substrate_placement_e2e
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 99.77s

$ cargo test -p syneroym-substrate --test binding_push_e2e --test durable_outbox_e2e --test reference_scenario_e2e
test result: ok. 2 passed  (binding_push_e2e)
test result: ok. 2 passed  (durable_outbox_e2e)
test result: ok. 1 passed  (reference_scenario_e2e)

$ cargo test -p syneroym-substrate --test gateway_hostname_e2e --test topology_document_e2e
test result: ok. 6 passed  (gateway_hostname_e2e)
test result: ok. 8 passed  (topology_document_e2e)

$ cargo test -p syneroym-substrate --test master_endpoint_record_e2e
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**Full workspace**: `cargo +nightly fmt --all` and `cargo clippy --workspace --all-targets --all-features` are clean. `cargo test --workspace --lib --bins` (34 test binaries, every unit/integration test outside `crates/*/tests/*_e2e.rs`) passes with 0 failures, run twice for confidence. `mise run build:test-components` (the `wasm32-wasip2` build check the milestone gate asks for — the WIT edit is not guest-facing, so this is a compile check, not a behavior one) is clean. `mise run test:e2e` (Playwright WebRTC browser suite) passes (18 + 4 tests, exit 0), matching F10's own finding that this suite is unaffected (it deploys with no `--identity` and publishes only via `roymctl registry register`, a path this slice does not touch).

A full `cargo test --workspace --tests` pass (needs real port binds, sandbox disabled) surfaced 6 failing test binaries; each was individually re-run in isolation afterward and investigated:
- **One real regression, found and fixed**: `scheduled_task_e2e.rs` (see the fixture-repair note above) — both its tests fail under the full run and pass individually once `visibility = "internal"` is declared; confirmed by re-running the whole file standalone (2/2, 307s).
- **Five confirmed environment artifacts, not regressions**: `guest_http_e2e`'s `test_trap_and_spin_return_500_and_a_new_stream_still_succeeds` (Iroh QUIC path-validation timeout), `http_passthrough_e2e`'s blob-GET performance-budget assertion, and `static_assets_e2e`/`topology_document_e2e`'s "substrate did not become available in time" / registry-registration timeouts, are the same class of flake the backlog's "CPU starvation, not flakiness" entry documents — this machine ran ~90 test binaries back-to-back with the sandbox disabled (needed for real socket binds), and by the time these ran, accumulated OS-level socket/registration pressure caused timeouts unrelated to any code path this slice touches. `service_visibility_e2e.rs` (this slice's own new file) hit the same class from the opposite direction — "No buffer space available (os error 55)" binding a relay socket — while the system was still under that same accumulated load; its `SubstrateTestContext::setup` already serializes its own tests via `tests/common/mod.rs`'s internal `SUBSTRATE_TEST_LOCK` (confirmed from the log: tests ran one at a time, each failing fast at the bind step, not concurrently), so no code change was needed there. Every one of these five files, and every `crates/substrate/tests/*_e2e.rs` file this slice touches, was re-run standalone after the full run finished and passes cleanly (evidence above).

---

## B3 — What shipped

Slice B3 implements the dual-build shim (D2/D3): one trait per host
interface (`data-layer`/`blob-store`/`messaging`), two implementations — a
`wit-bindgen` guest adapter and an in-process native shim linked into
`syneroym-substrate` behind a Cargo feature — proven by a fixture written
once, built both ways, with one integration suite driving both builds and
asserting byte-identical results.

### 1. Two new crates (`crates/app_host/`, `crates/app_host_native/`)

- `syneroym-app-host` (no dependency beyond `syneroym-wit-interfaces` and
  `async-trait`): `AppDataLayer`/`AppBlobStore`/`AppBlobWriter`/
  `AppBlobReader`/`AppMessaging` traits mirroring the three WIT interfaces
  function-for-function, plus `AppHost` (the combined bound) and
  `MessageSink` (the host→app delivery direction, mirroring
  `guest-api::handle-message`). `guest.rs` (wasm32-only) is `GuestHost`, the
  `wit-bindgen` implementation, calling `syneroym-wit-interfaces`'s
  pre-generated import bindings directly — **an app's own `generate!` call
  must remap these three interfaces onto `syneroym-app-host`'s bindings via
  wit-bindgen's `with:` option rather than regenerating them** (see finding
  below).
- `syneroym-app-host-native`: `NativeHostFactory` (long-lived: holds the
  providers, the broker, live subscriptions, the app's `MessageSink`) and
  `NativeAppHost` (per-invocation: a fresh `HostState`, exactly mirroring
  the sandbox's fresh `Store` per guest call). Every `AppDataLayer`/
  `AppBlobStore`/`AppMessaging` method delegates to `HostState`'s existing
  `Host` impl in `syneroym-sandbox-wasm` — one implementation of every gate,
  mask, and attribution rule, two callers. `NativeHostFactory::subscribe`
  mirrors the WASM path's `subscribe` + delivery pump step for step, except
  it does not persist the subscription (a stated, tested restart gap — see
  §4 below). `convert.rs` is a field-for-field guest⇄host type mapping, with
  a round-trip unit test per type.

### 2. The fixture (`test-components/dual-build-fixture/`, `syneroym-test-dual-build-fixture`)

A real **workspace member** (not excluded — `D-06B-6` amended in
[task.md](task.md)), `crate-type = ["cdylib", "rlib"]`, symlinked WIT deps.
`app.rs` is the whole shared behaviour (one JSON-in/JSON-out `run` verb,
application failures reported inside the payload rather than as a WIT
`Err`), compiled unchanged into both builds; `guest.rs`/`native.rs` are the
two thin, build-specific wirings around it. No `init`/`migrate` lifecycle
hook — schema is ensured lazily on first use. Ten request kinds exercise
data-layer CRUD, an admin-gated DDL denial, a data-layer read of a missing
id, a one-shot blob put, a streaming blob round trip through the writer/
reader resources, and a subscribe→publish→read-inbox messaging round trip.

### 3. Substrate wiring (`dual_build_fixture` Cargo feature, off by default)

`crates/substrate/src/runtime.rs`: `init_dual_build_fixture` builds a
`NativeHostFactory` + `NativeFixture`, registers the fixture under
`native_dispatch` (mirroring the `supervisor` role's shape exactly,
`SUPERVISOR_DISPATCH_ID`'s pattern), and registers exactly one endpoint
(`FIXTURE_INTERFACE`) — deliberately **not** a second `messaging` endpoint,
since `EndpointRegistry::register` is a silent last-write-wins insert and
`(node_did, "messaging")` already belongs to the supervisor. `SharedNodeHandles`
gains `blob_provider`/`logical_resolver` so the fixture's factory can reuse
what `build_route_handler_deps` already built.

### 4. The parity suite (`crates/app_host_native/tests/dual_build_parity.rs`)

Two fully independent host stacks (own `SqliteStorageProvider`, blob
provider, `MqttBroker`, temp dir) sharing **one** service id — the id is the
store namespace, the topic namespace, and the `data-layer/admin` gate
resource all at once, so the two builds must share it and therefore share
nothing else. Both driven under a real, identical, non-anonymous
`CallerContext`. `both_builds_produce_identical_results` compares six
sequential-body scenarios (store/read messages, admin-DDL denial, a missing
read, a one-shot blob put, a streaming blob round trip) verbatim across
both builds; a separate messaging test settles each build independently
(subscribe, publish, poll `read-inbox` until non-empty) before comparing.
`the_parity_comparison_detects_a_divergence` (a `Mutant` driver that
corrupts one field of the result) proves the comparison can actually fail —
without it, a green `both_builds_produce_identical_results` is not evidence
of anything. Two stated, tested permitted differences: a fresh
`ResourceTable` per native invocation (proven by two independent uploads
each landing in their own table, not clobbering each other), and
subscription persistence (WASM writes to `messaging_subscriptions` and
survives a restart; native deliberately does not — see the deferred-backlog
row).

### 5. `crates/substrate/tests/dual_build_fixture_e2e.rs`

One test, gated on the `dual_build_fixture` feature: boots a real
`SubstrateTestContext`, calls the linked-in native fixture through
`SyneroymClient::request` — the same client path any other native or WASM
service is reached through — and asserts the response. Proves §3's
registration; the in-process parity suite proves the shim itself.

### 6. Findings and fixes along the way

**Two genuine problems Step 1's de-risking spike was meant to catch, and
did — but not the one the plan anticipated.** The plan's own risk was "two
separate `wit_bindgen::generate!` invocations for the same import
interfaces, linked into one component, might not link." That was tested and
turned out fine on its own. The real, confirmed-against-the-real-toolchain
problem was broader:

- **`syneroym-wit-interfaces` could not be linked into a wasm32 component at
  all**, because it unconditionally compiles *seven* separate guest modules
  (`app_config`, `blob_store`, `control_plane`, `data_layer`, `messaging`,
  `supervisor`, `vault`), each running its own `generate!` for a different
  world, and wit-bindgen anchors each world's "component-type" custom
  section with a `#[used]` static specifically so `--gc-sections` cannot
  strip it — so *any* consumer of this crate on wasm32 pulls in all seven
  worlds' export requirements, most of them unsatisfiable by that consumer.
  `wasm-component-ld` failed to encode a component that only used
  `data-layer`/`blob-store`/`messaging`, reporting a spurious missing export
  (`init`) from `data-layer-guest`, an entirely different world. **Fixed**
  by gating each guest module behind its own Cargo feature
  (`crates/wit_interfaces/Cargo.toml`), default-on so every existing
  (host-only) consumer is unaffected, and having `syneroym-app-host`/the
  fixture opt into only the three they use.
- **`data_layer.rs`'s own `generate!` targeted `data-layer-guest`**, a world
  built for a *standalone* data-layer-only component (it requires exporting
  `init`/`migrate`), not for reuse as a pure import elsewhere. **Fixed** by
  adding a second, import-only world (`data-layer-import`) to
  `data-layer.wit` and retargeting `data_layer.rs` at it — additive, and
  safe because nothing used this module before B3 (confirmed by grep).

**A latent, previously-unreachable bug in `HostBlobWriter::finish`/`abort`**
(`crates/sandbox_wasm/src/host_capabilities.rs`), found by the parity
suite's first real run against the actual wasm32 component (not by
inspection): both are resource *methods*, so the canonical ABI hands the
host a *borrowed* `self_`, but the code called `self.table.delete(self_)`
directly — `ResourceTable::delete`'s own `debug_assert!(resource.owned())`
panics on a borrowed handle. Never triggered before B3 because nothing
called these functions through a real wasm guest (`blob-store`'s guest
bindings were dead code per the design finding in the implementation plan);
the crate's own existing unit tests construct an owned `Resource` by hand,
sidestepping the ABI distinction entirely. An initial fix re-derived an
owned handle from the same `rep()` before calling `table.delete` — this
silenced the `debug_assert!`, but a subsequent review (against
`wasmtime-46.0.2`'s own source, not documentation) found it traded the
panic for a worse, silent bug: deleting a still-borrowed entry frees its
table slot for reuse while the guest can still reference it, and a later
resource-drop for the original handle can then delete an unrelated,
currently-live resource that has since taken that slot. **Fixed** instead by
giving `HostUploadSession` an `Option<Box<dyn UploadSession>>` payload:
`finish`/`abort` `take()` the session out of a borrow and leave the table
entry itself alive (holding `None`), so only the resource's own destructor
(`drop`, which the ABI always hands a genuinely owned `rep`) ever deletes
the entry. The same review also found a related bug this shared code
enabled on the native build only — a dropped-without-finishing upload never
refunded its speculatively-reserved blob quota, since nothing native ever
triggered wasmtime's own resource-drop call — fixed by moving the refund
into a synchronous `Drop for ObjectStoreUploadSession`
(`crates/data_blob/src/object_store_impl.rs`), which now runs identically
on both builds regardless of which path (`finish`, `abort`, or an implicit
drop) ends the session. Verified against the pre-existing
`blob_store_integration.rs` suite (5/5, unaffected — it constructs owned
resources directly, so it never hit the original bug either way) and a new
`data_blob` unit test that drops a session mid-upload with neither `finish`
nor `abort` called and confirms its reservation is refunded.

---

## B3 — Verification Evidence

### Automated Tests

- **`crates/app_host_native/src/convert.rs`**: 13 unit tests, one round-trip
  per guest⇄host type (index definition, collection schema, record write
  value, patch mutation, every mutation/sql-value/data-layer-error/
  blob-error/messaging-error variant, record read value, query result, raw
  query result).
- **`crates/app_host_native/src/factory.rs`** (1 test):
  `read_only_host_denies_every_mutating_and_egress_call` — the `host_with`
  unit test the deferred-backlog row promised, added after a review found
  it was missing. 14 unit tests total in the crate.
- **`crates/app_host_native/tests/dual_build_parity.rs`** (13 tests):
  `both_builds_produce_identical_results`,
  `the_parity_comparison_detects_a_divergence`,
  `wasm_build_store_and_read_round_trip` /
  `native_build_store_and_read_round_trip`,
  `wasm_build_stream_blob_round_trips_the_body` /
  `native_build_stream_blob_round_trips_the_body`,
  `wasm_build_admin_ddl_is_denied` / `native_build_admin_ddl_is_denied`,
  `both_builds_deliver_a_published_message_to_their_own_inbox`,
  `each_native_invocation_gets_a_fresh_resource_table`,
  `only_the_wasm_stacks_subscription_is_persisted`,
  `malformed_request_json_errors_on_both_builds`,
  `malformed_params_frame_is_invalid_params_not_internal_error`. The shared
  `SCENARIOS` table that `both_builds_produce_identical_results`/
  `the_parity_comparison_detects_a_divergence` both run also grew from 6 to
  13 entries (added `patch`/`batch-mutate`/`delete-many`/`drop-collection`/
  `delete-blob`/`abort-upload`/`unsubscribe`) after a review found 12 of 26
  `AppDataLayer`/`AppBlobStore`/`AppMessaging` methods untested by either
  build; `aggregate`/`delete`/`query_raw`/`check_access` remain untested
  (deferred-backlog).
- **`crates/substrate/tests/dual_build_fixture_e2e.rs`** (1 test, feature-gated):
  `a_client_reaches_the_linked_native_fixture_through_the_router`.
- **`crates/sandbox_wasm/tests/blob_store_integration.rs`** (5 tests, pre-existing):
  re-run after the `HostBlobWriter::finish`/`abort` fix, unaffected.
- **`crates/data_blob/src/object_store_impl.rs`** (1 new test, 36 total):
  `dropping_a_session_without_finish_or_abort_still_refunds_reserved_quota`,
  covering the blob-quota-refund fix above.

### Component build (exit criterion 1)

```
$ cargo component build --release --target wasm32-wasip2 -p syneroym-test-dual-build-fixture
    Creating component target/wasm32-wasip1/release/syneroym_test_dual_build_fixture.wasm
$ wasm-tools validate --features component-model target/wasm32-wasip2/release/syneroym_test_dual_build_fixture.wasm
$ wasm-tools component wit target/wasm32-wasip2/release/syneroym_test_dual_build_fixture.wasm
# imports syneroym:data-layer/store, syneroym:blob-store/blob-store,
# syneroym:messaging/host-api (+ wasi:cli/wasi:io); exports
# syneroym:messaging/stream-types, syneroym:messaging/guest-api,
# syneroym-test:dual-build-fixture/test-driver -- confirms the linked
# component's actual export set, independent of the linker's own claims.

$ cargo build -p syneroym-substrate --features dual_build_fixture
    Finished `dev` profile [unoptimized + debuginfo] target(s)
```

### Run Evidence

```
$ cargo test -p syneroym-app-host-native --test dual_build_parity
running 11 tests
test permitted_differences::each_native_invocation_gets_a_fresh_resource_table ... ok
test permitted_differences::only_the_wasm_stacks_subscription_is_persisted ... ok
test wasm_build_admin_ddl_is_denied ... ok
test native_build_admin_ddl_is_denied ... ok
test native_build_store_and_read_round_trip ... ok
test both_builds_produce_identical_results ... ok
test the_parity_comparison_detects_a_divergence ... ok
test both_builds_deliver_a_published_message_to_their_own_inbox ... ok
test native_build_stream_blob_round_trips_the_body ... ok
test wasm_build_stream_blob_round_trips_the_body ... ok
test wasm_build_store_and_read_round_trip ... ok
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p syneroym-substrate --features dual_build_fixture --test dual_build_fixture_e2e
test a_client_reaches_the_linked_native_fixture_through_the_router ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p syneroym-sandbox-wasm --test blob_store_integration
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p syneroym-app-host-native --lib
test convert::tests::* (13 tests) ... ok
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

### Full workspace

`cargo +nightly fmt --all` clean. `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean (covers the `dual_build_fixture` feature). `mise run build:test-components` builds all 11 fixtures, including the new one, cleanly. `cargo check -p syneroym-app-host --target wasm32-wasip2` clean (the trait crate's wasm half is not reachable from any host-target command). `mise run test:e2e` (Playwright WebRTC browser suite) passes (18 + 4 tests, exit 0) — B3 touches no browser-facing surface.

`cargo test --workspace --lib --bins` (sandbox on, matching B1/B2's own convention) passes except for five crates whose failing tests all bind a real socket (`syneroym-community-registry`, `syneroym-control-plane`'s TCP/HTTP health probes, `syneroym-coordinator-webrtc`'s bootstrap-page HTTP tests, `syneroym-core`'s DHT registry tests, `syneroym-mqtt-broker`'s listener-binding test) — every one fails with `Operation not permitted (os error 1)`, the sandbox's socket-bind restriction, not a code defect. Confirmed by re-running exactly those five crates' `--lib` suites with the sandbox disabled: 358/358 pass (18+231+7+90+12).

**Genuine flake, pre-existing and already tracked**: `keys::tests::get_or_mint_warns_with_the_wording_matching_its_kind` (`syneroym-app-supervisor`) failed once under the full parallel run and passed cleanly (296/296) run alone immediately after — this is the exact, already-documented row in [deferred-backlog.md](../../deferred-backlog.md) §1 ("is flaky under load", thread-local `tracing` subscriber), unrelated to this slice.

---

## B4a — What shipped

Slice B4a implements the guest-facing `syneroym:conversation` host interface
(conversations, direct delivery, delivery state, history, outbox) and the
Layer 3 machinery underneath it — storage, the delivery outbox worker, and
peer-to-peer transport — against a **stated, non-production placeholder key
agreement**. Per `D-B4-1`, B4 is not complete until B4b replaces that
placeholder with real X3DH + Double Ratchet; everything else below (WIT
interface, storage schema, outbox semantics, dual-build shim, cross-node
delivery, signature-based attribution) is real and does not change shape
when B4b lands.

### 1. The WIT package (`crates/wit_interfaces/wit/conversation/conversation.wit`)

`syneroym:conversation@0.1.0`: `conversation` interface (`open-direct`,
`conversations`, `send`, `history`, `delivery-status`, `outbox`, `retry` —
seven functions, `D-B3-2`'s guest vocabulary only, no resources per
`D-B4-2`/`D-B3-6`) and `guest-api` interface (`on-message`,
`on-delivery-state` — optional host→guest exports). Three worlds
(`conversation-guest`, `conversation-import`, `conversation-host`), matching
`data-layer`/`messaging`'s own `*-import` precedent. **Not** added to
`host-environment` (additive, opt-in per component, matching the
`syneroym:http` precedent task.md names).

### 2. The `syneroym-conversation` crate (`crates/conversation/`, new)

- `store.rs` — `ConversationStore`: `conversation.db` per service, sharing
  one SQLite connection with its own `async_queue::Queue`
  (`D-B4-13`, `Queue::from_connection`) so the atomic `send` write
  (`D-B4-27`) is expressible at all. Tables: `conversations`, `messages`
  (ordered `(sender_timestamp, author, id)`, deduped `UNIQUE(author, id)`),
  `sessions` (TOFU-pinned signing key), `local_identity`, `prekey_requests`
  (rate limiting). Per-conversation/per-service bounds (`D-B4-16`).
- `ids.rs` — `derive_conversation_id` (order-independent over the address
  pair, `blake3`) and `derive_message_id` (content + nonce).
- `envelope.rs` — `DeliveryPayload` and its canonical byte encoding
  (`D-B4-22`): every message is ed25519-signed by the sender's own
  conversation signing key, length-prefixed so no field can be reassigned
  across the `body` boundary. Attribution is signature-based, never
  transport-based — replaces the transport-identity check the plan's own
  revision 1 found could never pass (F16: `CallerContext.caller_did` is the
  owner's Master DID, which cannot distinguish two services one owner
  runs).
- `crypto.rs` — the `SessionCrypto` trait (`D-B4-1`'s seam) and its **only
  implementation so far**, `StaticEcdhSessionCrypto`: one static ECDH-P256
  + AES-256-GCM session per peer, explicitly *not* a ratchet, no forward
  secrecy. `ConversationService::new` refuses to construct unless
  `ConversationConfig.allow_insecure_crypto` is `true` — a hardcoded,
  reviewable decision in `runtime.rs`, not a `SubstrateConfig` TOML field.
  Peer signing keys are pinned trust-on-first-use per address (`D-B4-28`);
  a later bundle presenting a different key for a pinned address is a hard
  failure, never a silent re-pin.
- `outbox.rs` — the delivery worker: mirrors `syneroym_router::proxy_outbox`'s
  three subtleties (F1) — a claim that never resolves is bounded by
  `claim_count`, not `attempts`; an unreadable payload is terminal; a
  target that no longer resolves is terminal. An unreachable peer
  `defer`s without charging the attempt budget (`D-B4-20`), bounded
  instead by `conversation_max_pending_age_secs` (default 30 days) from
  the message's own creation time. A dead letter gets a metric + `warn!`,
  matching the existing proxy outbox's own precedent — no `AlertStore`
  write (`D-B4-25`; that store is supervisor-scoped and a node running
  conversation with no supervisor role has nothing to write to).
- `transport.rs` — `deliver_one` (X3DH-placeholder prekey fetch on first
  contact, sign, encrypt, deliver, ratchet-commit only after a real `Ok`
  per `D-B4-18`) and `peer_deliver_impl` (decrypt, verify `author` isn't
  this service's own address — `D-B4-26`'s close of F17's same-service
  proxy exemption — verify the signature under the pinned key, reject an
  implausibly-future `sender_timestamp` per `D-B4-21`, atomic
  insert-or-ignore dedup + session commit). Outbound calls build
  `CallerContext::service_system` with `proof: None`
  (`D-B4-23`) and refuse up front if the sending service holds no
  unexpired instance certificate or recorded owner, naming the missing
  certificate rather than silently falling back to the node's own key
  (closing F10's silent-impersonation gap).
- `lib.rs` — `ConversationService` implementing `syneroym_rpc::ConversationHost`;
  `Weak<dyn ServiceProxy>`/notifier wiring both via `OnceLock`, since
  `ConversationService` is built before the real `ProxyRouter` exists
  (mirrors `AppSandboxEngine.service_proxy`'s own two-phase wiring).

### 3. `syneroym-async-queue` extensions (`D-B4-27`)

`Queue::transaction` (one lock, one `rusqlite::Transaction`, a `TxQueue`
handle that enqueues through it — `Queue::enqueue` itself would
self-deadlock if called inside a caller-held transaction on the same
connection, since `std::sync::Mutex` is not reentrant) and `Queue::defer`
(pushes a claimed item back to `visible_at` without charging the attempt
budget or the crashed-worker `claim_count` bound).

### 4. Host wiring

- `crates/sandbox_wasm/src/host_capabilities.rs`: `HostState.conversation:
  Weak<dyn ConversationHost>` (defaults `Weak::new()`, no `HostState::new`
  signature change — `D-B4-24`), `with_conversation` builder,
  `impl conversation::Host for HostState` (seven methods, delegating with
  `component_id`/`read_only`).
- `crates/sandbox_wasm/src/engine.rs`: `AppSandboxEngine.conversation:
  OnceLock<Weak<dyn ConversationHost>>` (same pattern as `service_proxy`,
  three construction sites, one `add_to_linker` line),
  `notify_guest_message`/`notify_guest_state` (instantiate-and-call with a
  4-attempt retry, mirroring `deliver_message`, using the existing generic
  `json_to_wasm_params` JSON→`Val` converter rather than hand-building
  `Val::Record` for a 10-field record), `impl ConversationNotifier for
  AppSandboxEngine`.
- `crates/control_plane/src/synsvc_native.rs`: sixth `dispatch` arm,
  `dispatch_conversation` (`prekey-bundle`/`deliver` only — the seven
  guest-facing verbs are unreachable through this arm). `SynSvcNativeService.
  conversation: OnceLock<Weak<dyn ConversationHost>>` (not a constructor
  parameter — `D-B4-24`'s reasoning applied to this struct's own ~26
  existing `::new` call sites, most of which never touch conversation).
  `ControlPlaneService.conversation` gains the matching `OnceLock`, set
  directly at construction (unlike `service_proxy`/`row_authorizer`,
  `ConversationService` already exists by then).
- `crates/core/src/local_registry.rs`: `NATIVE_CAPABILITY_INTERFACES`
  `[&str; 6]` → `[&str; 7]` (`conversation` added).
- `crates/substrate/src/runtime.rs`: constructs `ConversationService` in
  `build_route_handler_deps` (alongside the blob provider/logical
  resolver), wires the engine ↔ conversation notifier/host both ways via
  `OnceLock`, wires the real `ServiceProxy` in once `ConnectionRouter::init`
  produces it (`setup_router`), spawns the delivery worker beside
  `proxy_outbox_join` with matching cancellation/shutdown handling.

### 4. The dual-build surface (`D-B3` precedent)

- `crates/app_host/src/lib.rs`: `AppConversation` trait (seven methods,
  mirrors the WIT interface) and `ConversationSink` (host→app, mirrors
  `MessageSink`). `AppHost` widened to require `AppConversation` — the only
  two implementors (`GuestHost`, `NativeAppHost`) both gain it in this
  change (§14.5 of the plan: revisit if a third dual-build app exists
  later that never uses conversations).
- `crates/app_host/src/guest.rs`: `impl AppConversation for GuestHost`,
  calling the remapped `syneroym_wit_interfaces::conversation` bindings.
- `crates/app_host_native/src/{host,factory,convert}.rs`: `impl
  AppConversation for NativeAppHost` (delegates to `HostState`'s own
  `conversation::Host` impl, matching the data-layer/blob-store/messaging
  precedent exactly); six new guest⇄host type conversions
  (`ConversationError`/`DeliveryState`/`ConversationKind`/
  `ConversationSummary`/`Message`/`HistoryPage`), each with a round-trip
  unit test; `NativeHostFactory::new` gains an `Arc<ConversationService>`
  argument and registers itself as that service's notification target
  (`register_service_notifier`) so the delivery worker can wake a
  natively-linked app the same way `AppSandboxEngine` wakes a wasm-hosted
  one; `set_conversation_sink`/`impl ConversationNotifier for
  NativeHostFactory`, forwarding `syneroym-rpc`'s plain types to the guest
  vocabulary via two new `rpc_message_to_guest`/`rpc_delivery_state_to_guest`
  conversions. **Permitted difference, stated in code**: the native build's
  wake has no retry (a `Weak<dyn ConversationSink>` call), the WASM
  build's has four — the same shape B3 documented for `MessageSink`; what
  must match is the store contents afterward, which the parity suite
  compares.

### 5. The fixture (`test-components/dual-build-fixture/`)

`wit/deps/conversation/` (file symlink, matching the existing three),
`wit/world.wit` imports `syneroym:conversation/conversation@0.1.0` and
exports `syneroym:conversation/guest-api@0.1.0`. `app.rs` gained eight
`Request` variants (`open-conversation`, `send-message`, `read-history`,
`delivery-status`, `read-outbox`, `retry-message`,
`read-conversation-inbox`, `read-state-log`) and
`on_conversation_message`/`on_conversation_state` (persisted through
`data-layer`, never in-process state, `D-B3-12`) — collections named with
underscores (`conv_inbox`/`conv_state_log`), not hyphens, after a real
`data-layer` identifier-validation failure surfaced the hyphenated names
during test development. `guest.rs`/`native.rs` wire the two builds'
`guest-api`/`ConversationSink` implementations to the same two `app.rs`
functions.

### 6. Config (`crates/core/src/config.rs`)

Eight `AppSandboxRole` fields, beside the existing `queue_*` ones
(`D-B4-19`): `conversation_tick_secs` (5), `conversation_max_body_bytes`
(262,144), `conversation_max_pending_per_conversation` (1,000),
`conversation_max_messages_per_conversation` (100,000),
`conversation_max_pending_age_secs` (30 days),
`conversation_max_clock_skew_secs` (24 hours),
`conversation_prekey_pool_size` (100),
`conversation_prekey_requests_per_peer_per_hour` (20).

---

## B4a — Verification Evidence

### Automated Unit Tests

- **`syneroym-async-queue`** (`crates/async_queue/src/lib.rs`): 6 new
  tests — `transaction_commits_the_callers_write_and_the_enqueue_together`,
  `transaction_rolls_back_both_writes_on_err`,
  `defer_does_not_advance_attempts_and_uncounts_the_claim`,
  `a_deferred_item_is_invisible_to_claim_due_until_its_new_visible_at`,
  plus two existing suites unaffected. 68/68 pass.
- **`syneroym-conversation`** (new crate): 20 unit tests across
  `ids`/`envelope`/`store`/`crypto` — conversation-id order-independence
  and stability; message-id nonce sensitivity; the `send` transaction's
  atomicity under injected failure; the `(author, id)` dedup index;
  history ordering under a skewed clock with cursor paging; local-identity
  generate-once; prekey rate limiting; canonical-bytes signature
  round-trip and the length-prefix collision guard; a genuine
  self-consistent prekey bundle vs. a tampered one; both sides deriving
  the identical session and round-tripping a payload; `D-B4-28`'s
  re-presented-key refusal; the dropped-ack re-encrypt case. 20/20 pass.
- **`syneroym-app-host-native`** (`crates/app_host_native/src/convert.rs`):
  6 new round-trip tests for the conversation type conversions, alongside
  the pre-existing 13. 20/20 pass in the crate's `--lib` suite (13
  conversion + `read_only_host_denies_every_mutating_and_egress_call`,
  itself extended to construct a real `ConversationService`).

### The parity suite (`crates/app_host_native/tests/dual_build_parity.rs`)

Extended `SCENARIOS` (the byte-for-byte wasm-vs-native comparison table)
with five deterministic conversation entries (`open-direct`'s id is
derived solely from `(SERVICE_ID, peer_address)`, so — unlike
`send-message`, whose message id includes a random nonce — it belongs in
this table): `open-conversation`, and the deterministic-error shapes for
`retry`/`delivery-status`/`read-history` against an unknown id and an
empty `read-outbox`.

Five new dedicated tests (structural, not byte-comparison, since
`send-message`'s id is non-deterministic):
`open_direct_is_idempotent_on_both_builds`,
`send_writes_pending_and_appears_in_the_outbox_on_both_builds`,
`a_body_over_the_configured_limit_is_refused_on_both_builds`,
`retry_on_a_pending_message_is_invalid_argument_on_both_builds`, and
**`a_signed_delivery_from_an_external_peer_is_verified_and_notifies_the_app_on_both_builds`**
— drives a full prekey-fetch → X3DH-placeholder-session → sign → encrypt
→ `peer_deliver` exchange from an independent third `ConversationStore`
(standing in for a real peer) into *each* build's own `ConversationService`
directly (the peer-facing surface, unreachable from the guest `run()`
verb), and confirms the message lands `verified: true`, `state:
"delivered"` in that build's own history, and that the app's own
`on-message` export was actually called (read back through
`read-conversation-inbox`) — the host→app direction the SCENARIOS table
alone cannot exercise.

```
$ cargo component build --release --target wasm32-wasip2 -p syneroym-test-dual-build-fixture
    Creating component target/wasm32-wasip1/release/syneroym_test_dual_build_fixture.wasm
$ wasm-tools validate --features component-model target/wasm32-wasip2/release/syneroym_test_dual_build_fixture.wasm
$ wasm-tools component wit target/wasm32-wasip2/release/syneroym_test_dual_build_fixture.wasm
# world root: imports syneroym:data-layer/store, syneroym:blob-store/blob-store,
# syneroym:messaging/host-api, syneroym:conversation/conversation (+ wasi:*);
# exports syneroym:messaging/stream-types, syneroym:messaging/guest-api,
# syneroym:conversation/guest-api, syneroym-test:dual-build-fixture/test-driver
# -- confirms the linked component's actual import/export set, independent
# of the linker's own claims.

$ cargo test -p syneroym-app-host-native --test dual_build_parity
running 18 tests
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in ~10s
```

### Cross-node e2e (`crates/substrate/tests/conversation_e2e.rs`, new)

One test, `a_message_survives_a_restart_and_delivers_once_the_peer_exists`,
over two real `syneroym-substrate` instances (real iroh QUIC, self-hosted
relay, wasmtime), driving the dual-build-fixture's real WASM component
through an ordinary `SyneroymClient` — not a Rust-level fake. Proves the
milestone's reference-scenario steps 6-8 and failure-matrix rows 3-6 in
one flow: the peer is named (a deterministic DID) before it ever boots
(row 5, stronger than merely offline); several consecutive polls confirm
`pending`, never `delivered`, while the peer does not exist (row 3); the
sending substrate is torn down and rebooted with the message still
`pending`, and the outbox afterward holds exactly the same one item, not
zero and not two (row 4); the peer then comes up, delivery resumes on the
next worker tick, and the receiving side's own `read-history` shows
`verified: true`, `state: "delivered"`, with the exact body sent; the
app's own `on-message` export was called (`read-conversation-inbox`); and
both sides' pub/sub broker inbox (`read-inbox`) stays empty throughout —
asserted against the broker's own traffic, not by inspection (row 6,
ADR-0013 §6).

`Node`/`publish_endpoint`/the certified-instance-deploy pattern are copied
from `proxy_outbox_e2e.rs` (F13 of the plan: `multi_substrate_placement_e2e.rs`'s
`Node` pulls in the full `DeploymentPlan`/`compile`/`apply_plan`
app-placement machinery conversation delivery does not need — one
certified WASM service per node is enough).

```
$ cargo test -p syneroym-substrate --test conversation_e2e
running 1 test
test a_message_survives_a_restart_and_delivers_once_the_peer_exists ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 47.88s
```

**Not covered by this file**: 13 of the plan's §10.2 17-row table — dropped-ack
retry, rate-limited prekey requests, per-conversation quota isolation under
concurrent conversations, clock-skew rejection, alias canonicalization
(`D-B4-29`, not implemented — see the deferred-backlog row), the
same-service exemption (`D-B4-26`, unit-tested in `syneroym-conversation`
and the parity suite but not over a real cross-node connection), and
cross-service capability-gate denial. Rows 7-10 of the failure-and-security
matrix are B5's per the plan's own §10.5 ledger. Recorded in
[deferred-backlog.md](../../deferred-backlog.md) §5.

### Full workspace

`cargo +nightly fmt --all` clean. `cargo clippy --workspace --all-targets
--all-features` clean (zero warnings, not just zero errors — three real
findings fixed along the way: a `.expect()` that could have been a
`match` instead of a post-hoc unwrap in `transport.rs`'s session
establishment, a manual `Default` impl clippy could derive, and one
`field_reassign_with_default` in a test).

`cargo build -p syneroym-substrate --features dual_build_fixture` clean.
`cargo test -p syneroym-substrate --features dual_build_fixture --test
dual_build_fixture_e2e` passes (1/1) — proves the full `runtime.rs`
composition (construction, both-directions `OnceLock` wiring, worker
spawn) boots cleanly in a real substrate process, not just in isolation.

`cargo test --workspace --lib --bins` (sandbox on, matching B1/B2/B3's own
convention) passes except for the same five sandbox-restricted crates B3
already documented (`syneroym-community-registry`, `syneroym-control-plane`,
`syneroym-coordinator-webrtc`, `syneroym-core`, `syneroym-mqtt-broker`) —
each fails with `Operation not permitted (os error 1)`, the sandbox's
socket-bind restriction, not a code defect. Confirmed by re-running those
five crates' `--lib` suites with the sandbox disabled: 358/358 pass
(18+231+7+90+12) — the identical total B3 recorded, since this slice adds
no test to any of the five. **One flake, not a regression**:
`syneroym-fdae --lib` was reported failing inside the full concurrent
`--workspace` run but passes 99/99 standalone immediately after — the
same class of CPU-contention flake this milestone's own deferred-backlog
already documents for other crates under a full concurrent run; `fdae`
is untouched by this slice.

`mise run test:e2e` (Playwright WebRTC browser suite) passes (18 + 4
tests, exit 0) — the identical count B3 recorded; B4a touches no
browser-facing surface.
