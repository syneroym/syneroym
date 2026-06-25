# D-02-03: Retry Policy Ownership and Configuration Schema

**Status**: Accepted

**Context**: 
Requirement `[TOP-ROB]` specifies configurable retry counts (default 3) and exponential backoff. Currently, connection establishment uses an ad-hoc 30-attempt polling loop in `coordinator.rs`. The `SubstrateConfig` does not yet expose a structured retry policy.

**Decision**: 
We will define a canonical `RetryPolicy` struct in `crates/core/src/config.rs`. The policy will be configured globally per-coordinator-connection in the `SubstrateConfig`. 

**Consequences**: 
- **Enables**: Consistent, exponential backoff for outbound connections and registry registrations across the substrate.
- **Defers**: Fine-grained per-request or per-service retry policies.

**Implementation Notes**: 
- The `RetryPolicy` struct will expose `max_attempts: u8`, `initial_backoff_ms: u64`, `backoff_multiplier: f64`, and `max_backoff_ms: u64`.
- Use `#[serde(default)]` to provide reasonable defaults so the config file does not need to specify them explicitly.
- Implement the retry logic using a utility like `retry_with_backoff` in `crates/core/src/retry.rs`.
