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
- `SESSION_COOKIE_NAME = "syneroym_session"` (D-B1-12)
- `gateway_session_assertion(node_did, nonce, person_did)`: canonical JSON assertion payload `{"action":"gateway_session_login","node_did":...,"nonce":...,"person_did":...}` (D-B1-4)

### 2. Configuration & Anchor Lookup (`crates/core/src/config.rs`, `crates/core/src/dht_registry.rs`, `crates/router/src/handshake.rs`)
- Added `session_ttl_secs` to `ClientGatewayRole` (default 28,800s / 8 hours, configurable, D-B1-10).
- Defined `MasterAnchorResolver` trait in `syneroym-core` with async `resolve_master_anchor(&self, master_did: &str) -> Result<MasterAnchorPayload>`.
- Implemented `MasterAnchorResolver` for `RegistryClient` (DHT + HTTP registry fallback). Re-exported from `syneroym-router::handshake`.

### 3. Session Store & Reserved Routing (`crates/client_gateway/src/session.rs`, `crates/client_gateway/src/gateway.rs`)
- Implemented `SessionStore` with bounded DashMap storage (`MAX_ACTIVE_SESSIONS = 10,000`, `MAX_PENDING_CHALLENGES = 1,000`), TTL expiration, LRU eviction, and single-use challenge nonces (`NONCE_TTL_SECS = 300`, D-B1-2, D-B1-16).
- Implemented challenge/login/logout/whoami lifecycle under reserved `/_syneroym/session/*` routes (D-B1-3).
- Implemented credential extraction (`extract_credential`) prioritizing `Authorization: Bearer` over `Cookie` (D-B1-18).
- Implemented credential stripping (`strip_credential`) that removes the session token from proxied HTTP requests while preserving non-session credentials (D-B1-13).
- Client Gateway routes requests under `/_syneroym/` to internal session handlers and blocks them from proxying to guest services (D-B1-14).
- Forwarded active `DelegationCertificate` in `SyneroymClient::passthrough_with_conn` to populate preamble delegation.

### 4. Control Plane Route & Asset Collision Guard (`crates/control_plane/src/http_routes.rs`, `crates/control_plane/src/assets.rs`)
- Reject any guest HTTP route or static asset bundle containing a path starting with `/_syneroym/` at deploy time with HTTP 400 (D-B1-15).

### 5. Roymctl CLI Session Commands (`apps/roymctl/src/commands/session.rs`)
- Added `roymctl session login`, `roymctl session status`, `roymctl session token`, `roymctl session logout` (D-B1-8, D-B1-9).
- Saves session token locally in `~/.syneroym/session.json` with secure file permissions (0600 on Unix, D-B1-9).

---

## B1 — Verification Evidence

### Automated Unit and Integration Tests

1. **Client Gateway Unit Tests** (`crates/client_gateway/src/session.rs`):
   - `test_01_challenge_issue_and_expiration`: Verifies challenge nonce issuance and 300s expiration.
   - `test_02_login_consumes_nonce_once`: Verifies nonces cannot be replayed.
   - `test_03_login_verifies_signature`: Rejects forged assertions.
   - `test_04_login_verifies_delegation_scope_and_target`: Requires `routing` scope and matching node DID.
   - `test_05_login_verifies_delegation_not_expired`: Rejects expired delegation certs.
   - `test_06_login_checks_anchor_revocation`: Rejects revoked temporary keys.
   - `test_07_login_refuses_when_anchor_resolution_fails`: Fails closed on anchor lookup error.
   - `test_08_session_ttl_capped_by_delegation_expiry`: Caps session TTL at delegation expiry.
   - `test_09_session_lookup_and_expiration`: Verifies session store lookup and expiration.
   - `test_10_logout_removes_session`: Verifies explicit session revocation.
   - `test_11_max_sessions_capacity_eviction`: Verifies capacity bounds and LRU eviction.
   - `test_12_max_challenges_capacity_eviction`: Verifies challenge store capacity bounds.
   - `test_13_extract_and_strip_cookie`: Verifies session cookie extraction and selective stripping.
   - `test_14_extract_and_strip_bearer`: Verifies session bearer extraction and stripping.
   - `test_15_preserve_non_session_credentials`: Preserves non-session authorization headers.

2. **Control Plane Reserved Prefix Unit Tests**:
   - `crates/control_plane/src/http_routes.rs`: `test_reject_reserved_syneroym_prefix` passes.
   - `crates/control_plane/src/assets.rs`: `test_reject_reserved_syneroym_prefix_in_assets` passes.

3. **Substrate E2E Integration Tests** (`crates/substrate/tests/gateway_session_e2e.rs`):
   - `test_16_anonymous_request_sees_self_asserted_node_did`: Anonymous requests through gateway see self-asserted node DID.
   - `test_17_logged_in_request_via_cookie_sees_delegated_person_did`: Session cookie request sees verified person DID with delegated auth.
   - `test_18_logged_in_request_via_bearer_sees_delegated_person_did`: Bearer token request sees verified person DID with delegated auth.
   - `test_19_gateway_session_token_is_stripped_from_proxied_headers`: Session cookie/bearer is stripped from proxied request headers; non-session auth preserved.
   - `test_20_bearer_takes_priority_over_cookie`: Bearer token takes priority when both Bearer and Cookie are present.
   - `test_21_expired_gateway_session_falls_back_to_self_asserted_node_did`: Expired session falls back to self-asserted node DID.
   - `test_22_restart_clears_all_sessions`: Gateway restart clears in-memory session store; previous tokens fall back to self-asserted.
   - `test_23_reserved_path_challenge_login_whoami_logout_lifecycle`: Full challenge/login/whoami/logout lifecycle over HTTP gateway.
   - `test_24_reserved_path_is_never_proxied_to_guest`: Reserved path `/_syneroym/*` returns 404 from gateway and is never proxied to guest.
   - `test_25_roymctl_session_cli_lifecycle`: `roymctl session` login, status, token, and logout lifecycle.

```
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 78.26s
```
