# D-02-02: Handshake Authorization Point

**Status**: Accepted

**Context**: 
The architecture describes an End-to-End (E2E) ECDH handshake inside the established stream, which is already used in multi-hop relay mode to ensure intermediate Iroh nodes don't see data. The Iroh transport layer provides QUIC TLS 1.3 for the underlying transport. We needed to determine where the application-level identity handshake (checking the delegation certificate) should occur in M2, and whether it requires the full E2E ECDH key derivation.

**Decision**: 
The application-level authorization handshake will be implemented inside the `RouteHandler` (`crates/router/src/route_handler.rs`) by checking the identity chain presented in the connection preamble. For M2, verifying the delegation certificate chain against the Master Anchor is sufficient to pass this specific authorization handshake. The E2E ECDH handshake remains responsible for data privacy but is orthogonal to this authorization check. 

**Consequences**: 
- **Enables**: Immediate authorization and rejection of unauthorized or expired Temporary Keys at the router level before passing the stream to the application sandbox.
- **Defers**: Extending the E2E ECDH session key derivation into new layers; we rely on the existing multi-hop ECDH for privacy and add this certificate verification for trust.

**Implementation Notes**: 
- Implement a `HandshakeVerifier` in `crates/router/src/handshake.rs`.
- Integrate `HandshakeVerifier::verify_preamble` into `RouteHandler::on_connection` to validate the `RoutePreamble`.
