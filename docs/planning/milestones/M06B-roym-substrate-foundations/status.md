# M06B Roym Substrate Foundations — Status

**Milestone:** [task.md](task.md) · **Designs of record:**
[slice-b1-implementation-plan.md](slice-b1-implementation-plan.md) (B1),
[slice-b2-implementation-plan.md](slice-b2-implementation-plan.md) (B2)

**Overall:** Slices B1 and B2 complete (2026-08-18).

---

## Slice status

| Slice | Scope | Status | Gate |
|---|---|---|---|
| B1 | Person identity at the client gateway (G3) | **Complete (2026-08-18)** — [implementation plan](slice-b1-implementation-plan.md), evidence below | None — independently mergeable |
| B2 | Declared service visibility (G4) | **Complete (2026-08-18)** — [implementation plan](slice-b2-implementation-plan.md), evidence below | None (ADR-0018 Accepted) |
| B3 | The dual-build shim (D2/D3) | Ready for planning | None |
| B4 | Durable messaging: interface and 1:1 delivery (G1 part 1, G2) | Pending | B3 |
| B5 | Group delivery (G1 part 2) | Pending | B4 |

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
- **`syneroym-sdk`** (`deploy.rs`, `lib.rs`): `certify_placed_members` skips a `private` member and sets `is_private` correctly for `internal`/`public`; `Publication::split`; `SyneroymClient::new_with_record` verifies the signature and rejects a tampered record.
- **`syneroym-control-plane`** (`service/orchestration.rs`): `validate_publication` — 10 table-driven cases (§7 tests 1–10); `a_private_redeploy_removes_the_stored_endpoint_record_file` (test 36, direct access to `hosted_apps_dir`).
- **`syneroym-app-supervisor`** (`topology.rs`, `service.rs`): `service_topology_visibility` (declared/disagreement/short-hash); `handle_resolve` tests 24/26/27/28/29 (open bypass, non-existent service, retired instance, granted-caller `InvalidParams` unaffected, byte-identical document); `submit_is_refused_when_the_plan_declares_open_topology_over_a_private_service` (test 20, the `handle_submit` backstop entry point for `D-B2-14`).
- **`syneroym-core`** (`dht_registry.rs`): `test_dht_publication_skipped_for_private_record` (test 32).
- **`syneroym-data-db`** (`registry_store.rs`): `visibility` round-trips through `save_deploy_facts`/`load_all_deploy_facts`; a pre-existing row without the column loads as `None`.

### Integration / E2E Tests

- **`crates/substrate/tests/service_visibility_e2e.rs`** (new file, tests 33–36, 40): undeclared visibility publishes nothing; declaring `public` with no certificate is refused; `public` with a matching certificate publishes and resolves; a `public`→`private` redeploy is recorded as private; `orchestrator/list` distinguishes a private service from a public one.
- **`crates/substrate/tests/multi_substrate_placement_e2e.rs`** (new test, test 37): an app deployed with no visibility declaration publishes no member records, and a cross-node dial for a member fails to resolve.
- **`crates/substrate/tests/topology_document_e2e.rs`** (new tests, tests 41–42): an outside caller with no UCAN grant resolves an `open` app's members; the same caller is refused for a `restricted` service on the same app.
- **`crates/substrate/tests/gateway_hostname_e2e.rs`** (new tests, tests 43–44): a client gateway with neither credential resolves and proxies an `open` logical hostname, and still refuses a `restricted` one.
- **F10's fixture repairs** (group A/B/C): `master_endpoint_record_e2e.rs`, `multi_substrate_placement_e2e.rs`, `binding_push_e2e.rs`, `reference_scenario_e2e.rs`, `durable_outbox_e2e.rs`, `gateway_hostname_e2e.rs`, `topology_document_e2e.rs` all updated to declare `visibility = "internal"` where cross-node resolution is exercised, with matching `is_private` on their signed records.
- **One F10-group-C fixture the plan's own enumeration missed, found by a full workspace run**: `scheduled_task_e2e.rs`'s `worker` service is placed on a different substrate (`MANAGED_ALIAS`) from its supervisor and has no `depends_on`, so neither of F10's two grep patterns (`PlacementSelector::Substrate` + `depends_on`, or a direct member-DID dial) caught it — but the supervisor's own scheduled-tick push dials the worker by DID through the registry on every cron tick, which is exactly F10 group C's shape. Declaring `visibility = "internal"` fixes it, and both of that file's tests pass individually and together (309s total).

### Run Evidence

```
$ cargo test -p syneroym-app-orchestration --lib
test result: ok. 164 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p syneroym-sdk --lib
test result: ok. 65 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p syneroym-control-plane --lib
test result: ok. 230 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p syneroym-app-supervisor --lib
test result: ok. 295 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p syneroym-core --lib
test result: ok. 90 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p syneroym-data-db --lib
test result: ok. 180 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p roymctl --lib
test result: ok. 69 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

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
