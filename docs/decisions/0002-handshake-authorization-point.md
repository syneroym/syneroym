# D-02-02: Handshake Authorization Point

**Status**: Accepted (amended 2026-10-19 — see **Amendments** at the end)

**Context**: 
The architecture describes an End-to-End (E2E) ECDH handshake inside the established stream, which is already used in multi-hop relay mode to ensure intermediate Iroh nodes don't see data. The Iroh transport layer provides QUIC TLS 1.3 for the underlying transport. We needed to determine where the application-level identity handshake (checking the delegation certificate) should occur in M2, and whether it requires the full E2E ECDH key derivation.

**Decision**: 
The application-level authorization handshake will be implemented inside the `RouteHandler` (`crates/router/src/route_handler.rs`) by checking the identity chain presented in the connection preamble. For M2, verifying the delegation certificate chain against the Master Anchor is sufficient to pass this specific authorization handshake. The E2E ECDH handshake remains responsible for data privacy. It is separate from this authorization check. 

**Consequences**: 
- **Enables**: Immediate authorization and rejection of unauthorized or expired Temporary Keys at the router level before passing the stream to the application sandbox.
- **Defers**: Extending the E2E ECDH session key derivation into new layers; we rely on the existing multi-hop ECDH for privacy and add this certificate verification for trust.

**Implementation Notes**: 
- Implement a `HandshakeVerifier` in `crates/router/src/handshake.rs`.
- Integrate `HandshakeVerifier::verify_preamble` into `RouteHandler::on_connection` to validate the `RoutePreamble`.

## Amendments

### A1. A caller proves it holds its key; a forwarder never sees the identity

**Added 2026-10-19.** The Decision changes as follows.

1. **Checking the certificate chain is not enough.** A delegation
   certificate is a bearer credential: a service sends its instance
   certificate on every remote call, so every node it calls holds a copy. The
   caller now also signs `service_id`, `interface` and `ts` (Unix seconds)
   with the key in `pubkey`. Each field is length-prefixed, after the tag
   `syneroym-preamble-v1`. The router refuses a preamble that has a `pubkey`
   and no valid signature, or a `ts` more than 15 minutes from its own clock.
   This holds with and without a certificate. The router keeps no replay
   state: `ts` limits how long a party that received the proof can use it.
2. **`pubkey` is only the identity key.** The P-256 key of the end-to-end
   handshake moves to its own field, `epk`.
3. **Only coordinators forward.** On a local registry miss, a coordinator
   forwards the stream to the host and checks nothing about the caller. Any
   other node refuses the stream (`not_hosted_here`). The client then looks
   the service up again.
4. **On an indirect connection, the identity travels only inside the
   end-to-end channel.** A connection is indirect when the client dials a node
   that is not the host. There, the preamble carries only the route and
   `epk`. The client checks the host's signature over the handshake keys with
   the host key it expects, from the target's signed record. Only then does it
   send its identity, encrypted. With no known host key, it sends no
   identity. On a direct connection QUIC is end to end, so the identity stays
   in the preamble and there is no handshake.
5. **The sender is responsible for its own encryption.** A forwarder only
   routes. It can copy or replay encrypted bytes, but each handshake uses a
   fresh server key, so a replay does not decrypt. A sender that puts its
   identity in plaintext through a forwarder exposes only its own identity.

Why: the Context above says the end-to-end handshake "is already used in
multi-hop relay mode". That was true only for the browser. The Rust client
never ran it, so a forwarder read every Rust call. And because the preamble
is sent before the handshake, even an encrypted stream showed the caller's
certificate to the forwarder. Rules 1 to 5 make authorization depend on the
caller's key, and keep the identity away from anyone but the destination.

Rejected: a nonce and a replay cache (state, for a case only a destination
can cause), a challenge from the server (one more round trip on every
stream), binding the proof to the QUIC connection (breaks forwarders), and a
forwarder that refuses plaintext streams (a malicious forwarder ignores it).

Landing slices: **M06D C4b** (rules 1 to 3) and **M06D C4c** (rules 4 and
5) — [signed-route-preamble change doc](../planning/changes/signed-route-preamble/change.md).
