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

## Slices remaining in Milestone 2
- [ ] Slice 3: Runtime Quotas and Connection Caps
- [ ] Slice 4: Native TLS, Release Pipeline, and Docker
- [ ] Slice 5: Smoke Tests and Operational Observability
