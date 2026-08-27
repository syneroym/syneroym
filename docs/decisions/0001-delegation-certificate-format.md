# D-02-01: Delegation Certificate Format

**Status**: Accepted (amended 2026-08-27 — see the dated amendment note at
the end)

**Context**: 
The architecture specifies that the Master Key issues a "Delegation Certificate" to a Temporary Key. The `crates/identity` module already has a `ControllerAgreement` (a mutual-signature binding between a node DID and a controller DID). The `MasterAnchorPayload` in `crates/core/src/dht_registry.rs` currently lists authorized Temporary Key DIDs as bare strings without encoding a delegation expiry as a verifiable certificate. We needed to decide whether to reuse `ControllerAgreement` or define a new certificate structure, and what serialization to use.

**Decision**: 
We will define a new `DelegationCertificate` struct. It will use canonical JSON (RFC 8785) for serialization and be signed with Ed25519 signatures. It will include an absolute UNIX timestamp (`expires_at_secs`) to represent the expiry. Note: we keep the `master_anchor_v1` schema (no schema bump to `v2` is needed).

*Update:* We decided not to embed these certificates in the `MasterAnchorPayload` to avoid bloating the DHT record. Instead, the `DelegationCertificate` is embedded in the `EndpointInfo` payload for peers to do additional verification on a per-service basis.

**Consequences**: 
- **Enables**: Cryptographically verifiable expiry of Temporary Keys independent of the DHT record's own TTL. Allows the RouteHandler to reject expired keys even if the DHT record hasn't been purged.
- **Defers**: W3C Verifiable Credential Data Model compatibility is deferred.

**Implementation Notes**: 
- Create `DelegationCertificate` in `crates/identity/src/delegation.rs` with fields: `master_did`, `temporary_did`, `issued_at_secs`, `expires_at_secs`, `scope`, and `signature`.
- *Update:* The `MasterAnchorPayload` will remain with `"schema": "master_anchor_v1"` and a string array of `temporary_keys`. The `DelegationCertificate` will instead be embedded in the `EndpointInfo` (which is stored in the temporary key's DHT record). This avoids blowing up the master anchor DHT record size if more delegates are present.
- Handshake verification in `RouteHandler` will be opt-in, with strict validation enforced later when RBAC configuration is added for services.

**Amendment (2026-08-27, [ADR-0024](0024-client-gateway-identity-and-auth-service.md) §4a).**
A third carrier for a `DelegationCertificate` is added, alongside the
`EndpointInfo` embedding above: browser login. The person's master key mints a
short-TTL certificate for a freshly generated **temporary key that lives in the
browser**, scoped to session auth; the pair travels as a `session-key.json`
file (`roymctl session delegate`) and the certificate is sent in the body of a
`POST /_syneroym/session/login` request rather than published to the DHT or a
registry. The struct, its canonical-JSON encoding, its signature, and
`expires_at_secs` are unchanged — only where the certificate is carried and who
verifies it (the node auth service) are new.
