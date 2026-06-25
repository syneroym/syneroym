# D-02-01: Delegation Certificate Format

**Status**: Accepted

**Context**: 
The architecture specifies that the Master Key issues a "Delegation Certificate" to a Temporary Key. The `crates/identity` module already has a `ControllerAgreement` (a mutual-signature binding between a node DID and a controller DID). The `MasterAnchorPayload` in `crates/core/src/dht_registry.rs` currently lists authorized Temporary Key DIDs as bare strings without encoding a delegation expiry as a verifiable certificate. We needed to decide whether to reuse `ControllerAgreement` or define a new certificate structure, and what serialization to use.

**Decision**: 
We will define a new `DelegationCertificate` struct. It will use canonical JSON (RFC 8785) for serialization and be signed with Ed25519 signatures. It will include an absolute UNIX timestamp (`expires_at_secs`) to represent the expiry. These certificates will be embedded directly in a `v2` schema of the `MasterAnchorPayload`.

**Consequences**: 
- **Enables**: Cryptographically verifiable expiry of Temporary Keys independent of the DHT record's own TTL. Allows the RouteHandler to reject expired keys even if the DHT record hasn't been purged.
- **Defers**: W3C Verifiable Credential Data Model compatibility is deferred.

**Implementation Notes**: 
- Create `DelegationCertificate` in `crates/identity/src/delegation.rs` with fields: `master_did`, `temporary_did`, `issued_at_secs`, `expires_at_secs`, `scope`, and `signature`.
- Update `MasterAnchorPayload` in `crates/core/src/dht_registry.rs` to use `"schema": "master_anchor_v2"` and replace the `temporary_keys` array of strings with an array of embedded `DelegationCertificate` JSON objects.
