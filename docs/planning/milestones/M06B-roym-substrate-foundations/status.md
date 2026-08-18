# M06B Roym Substrate Foundations — Status

**Milestone:** [task.md](task.md) · **Designs of record:**
[slice-b1-implementation-plan.md](slice-b1-implementation-plan.md) (B1)

**Overall:** Slice B1 complete (2026-08-18).

---

## Slice status

| Slice | Scope | Status | Gate |
|---|---|---|---|
| B1 | Person identity at the client gateway (G3) | **Complete (2026-08-18)** — [implementation plan](slice-b1-implementation-plan.md), evidence below | None — independently mergeable |
| B2 | Declared service visibility (G4) | Ready for planning | None (ADR-0018 Accepted) |
| B3 | The dual-build shim (D2/D3) | Ready for planning | None |
| B4 | Durable messaging: interface and 1:1 delivery (G1 part 1, G2) | Pending | B3 |
| B5 | Group delivery (G1 part 2) | Pending | B4 |

---

## B1 — What shipped

Slice B1 enables local person identity at the client gateway, binding an authenticated local client to a person's DID under an owner→node delegation certificate instead of presenting the node's fallback self-asserted DID.

### 1. Protocol Constants & Assertions (`crates/core/src/protocol_utils.rs`)
- `GATEWAY_RESERVED_PATH_PREFIX = "/_syneroym/"` (D-B1-1)
- `SESSION_COOKIE_NAME = "syneroym_session"` (D-B1-6)
- `gateway_session_assertion(node_did, nonce, person_did)`: canonical JSON assertion payload `{"node_did":...,"nonce":...,"person_did":...}` (D-B1-4)

### 2. Configuration & Anchor Lookup (`crates/core/src/config.rs`, `crates/core/src/dht_registry.rs`, `crates/router/src/handshake.rs`)
- Added `session_ttl_secs` to `ClientGatewayRole` (default 28,800s / 8 hours, configurable, D-B1-10).
- Defined `MasterAnchorResolver` trait in `syneroym-core` with async `resolve_master_anchor(&self, master_did: &str) -> Result<MasterAnchorPayload>`.
- Implemented `MasterAnchorResolver` for `RegistryClient` (DHT + HTTP registry fallback). Re-exported from `syneroym-router::handshake`.

### 3. Session Store & Reserved Routing (`crates/client_gateway/src/session.rs`, `crates/client_gateway/src/gateway.rs`)
- Implemented `SessionStore` with bounded DashMap storage (`MAX_ACTIVE_SESSIONS = 64`, `MAX_PENDING_CHALLENGES = 64`), TTL expiration, oldest-expiry eviction, and single-use challenge nonces (`NONCE_TTL_SECS = 60`, D-B1-2, D-B1-8).
- Implemented challenge/login/logout/whoami lifecycle under reserved `/_syneroym/session/*` routes (D-B1-3).
- Implemented credential extraction (`extract_credential`) prioritizing `Cookie` over `Authorization: Bearer` (Plan §5.5, D-B1-7).
- Implemented credential stripping (`strip_credential`) that unconditionally removes `syneroym_session` pairs from `Cookie:` headers and drops matching `Authorization: Bearer <session-token>` lines, preserving unrelated authentication headers (D-B1-7).
- Added 10-second timeout on reading HTTP request headers and request body in `handle_connection`.
- Client Gateway routes requests under `/_syneroym/` to internal session handlers and blocks them from proxying to guest services (D-B1-1).
- Forwarded active `DelegationCertificate` in `SyneroymClient::passthrough_with_conn` to populate preamble delegation.

### 4. Control Plane Route & Asset Collision Guard (`crates/control_plane/src/http_routes.rs`, `crates/control_plane/src/assets.rs`)
- Reject any guest HTTP route or static asset bundle containing a path starting with `/_syneroym/` at deploy time with HTTP 400 (D-B1-1).

### 5. Roymctl CLI Session Commands (`apps/roymctl/src/commands/session.rs`)
- Added `roymctl session login`, `roymctl session status`, `roymctl session token`, `roymctl session logout` (D-B1-9).
- Saves session token locally under `<config-dir>/sessions/<sanitized-gateway-url>.json` with secure file permissions (0600 on Unix, D-B1-9).

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
   - `test_extract_credential`: Verifies Cookie priority over Bearer and UTF-8 safety.
   - `test_strip_credential`: Verifies unconditional session cookie stripping and bearer stripping.
   - `test_classify`: Verifies routing classification for proxy and session endpoints.

2. **Control Plane Reserved Prefix Unit Tests**:
   - `crates/control_plane/src/http_routes.rs`: `test_reject_reserved_syneroym_prefix` passes.
   - `crates/control_plane/src/assets.rs`: `test_reject_reserved_syneroym_prefix_in_assets` passes.

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
   - `test_30_roymctl_session_cli_error_handling`: `roymctl session` missing `--as` flag and nonexistent identity error handling.
