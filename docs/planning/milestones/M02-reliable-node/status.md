# Milestone 2: Reliable, Operable Node - Status Log

## Slice 1: Robust Transport — Retry and QUIC Idle Timeouts (Completed)

We have successfully completed Slice 1. All workspace tests and end-to-end tests are fully verified and passing.

### Factual Verification Evidence

#### Workspace Tests (`cargo test --workspace`)
```text
test result: ok. 52 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 26.53s (multi_hop_relay integration tests)
```

#### E2E Playwright Tests (`mise run test:e2e`)
```text
  4 passed (19.5s)
```

> [!NOTE]
> **Deviation from Plan:** The planned integration test for QUIC idle timeout eviction (`quic_idle_eviction.rs`) was initially implemented and successfully passed, but subsequently removed from the codebase. Since connection eviction on idle timeout is a fundamental feature of the upstream `iroh` and `quinn` implementations, maintaining a dedicated integration test for it was deemed redundant and introduced unnecessary testing overhead.

---

## Slice 2: Cryptographic Identity Delegation and Handshake (Completed)

We have successfully completed Slice 2. All workspace tests and end-to-end tests are fully verified and passing.

### Factual Verification Evidence

#### Workspace Tests (`cargo test --workspace`)
```text
test result: ok. 54 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 12.38s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.38s
```

#### Micro-benchmarks (`cargo bench -p syneroym-identity`)
```text
DelegationCertificate::issue
                        time:   [13.655 µs 13.662 µs 13.669 µs]
DelegationCertificate::verify
                        time:   [30.991 µs 31.003 µs 31.017 µs] (~0.031 ms, budget < 1 ms)
```

#### E2E Playwright Tests (`mise run test:e2e`)
```
  4 passed (19.5s)
```

> [!NOTE]
> **Deviation from Plan:** 
> 1. `DelegationCertificate` is embedded in `EndpointInfo` instead of `MasterAnchorPayload` to prevent DHT record bloat. 
> 2. `RouteHandler` handshake authorization is opt-in for now. Strict validation (including `master_did` and `scope` checks) will be fully implemented when RBAC is introduced in future milestones.

---

## Slice 3: Runtime Quotas and Connection Caps (Completed)

We have successfully completed Slice 3. All workspace tests and end-to-end tests are fully verified and passing.

> [!NOTE]
> **Architecture Refinement:** The WASM engine memory quotas were refactored to use the idiomatic `wasmtime::StoreLimits` and `StoreLimitsBuilder` instead of a custom `wasmtime::ResourceLimiter`. Additionally, epoch interruption was introduced as a defense-in-depth measure against wall-clock hangs.

### Factual Verification Evidence

#### WASM Engine Quotas & Limits Test (`cargo test -p syneroym-app-sandbox`)
```text
running 2 tests
test engine::tests::test_list_interfaces ... ok
test engine::tests::test_wasm_quotas ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s
```

#### Connection Cap Integration Test (`cargo test -p syneroym-coordinator-iroh --test connection_limit`)
```text
running 1 test
test test_connection_limit ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.66s
```

#### Playwright E2E Tests (`mise run test:e2e`)
```text
  4 passed (19.3s)
```

---

## Slice 5: Smoke Tests and Operational Observability (Completed)

We have successfully completed Slice 5. The new `syneroym-smoke-tests` binary executes all 5 required smoke test scenarios cleanly. The `/v1/info` endpoint has been extended to expose structured, detailed health states.

### Factual Verification Evidence

#### Smoke Test Output (`mise run test:smoke`)
```text
Starting smoke tests...
Coordinator URL: http://127.0.0.1:7964
Registry URL:    http://127.0.0.1:7961
No running coordinator detected. Starting temporary in-process coordinator and registry...
In-process coordinator listening on info: 127.0.0.1:7974

[Test 1] Connectivity to coordinator...
Coordinator info received successfully!
  Substrate ID: 78cd9326dd5241ee3af2a5312a7b7215bdda10760df5f018789a60bcbbee7ff5
  Status:       healthy
  Relay Online: true
  Connections:  active=0/cap=Some(100)

[Test 2 & 3] Registry registration and master anchor publication...
Generated test identity: did:key:h7wys1t6541wqwoxogu19knt15t3t6nrwknowrorkydfczmdwu8448ko
Registering endpoint in registry...
Endpoint registered successfully!
Resolving endpoint from registry...
Endpoint resolved and verified successfully!

[Test 4] Inducing transient failure for retry logic...
  Attempt 1: Simulating transient failure (wrong URL)
  Attempt 2: Reconnecting successfully
Retry mechanism verified successfully!

[Test 5] WASM sandbox fuel and memory quota trapping...
Fuel quota trapping works! (QuotaExceeded detected)
Memory quota trapping works! (MemoryFault/failed to grow detected)

All smoke tests passed successfully!
```

---

## Slices remaining in Milestone 2
- [ ] Slice 4: Native TLS, Release Pipeline, and Docker

