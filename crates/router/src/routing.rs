//! Routing table and address resolver
//!
//! Resolves nicknames and short hashes, maps remote services to their public substrate address,
//! and performs access control checking.

/// The encryption stage for the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptionStage {
    /// No encryption negotiation; stream bytes are passed through as-is.
    None,
    /// This substrate is the terminating E2E endpoint. Perform ECDH-P256 handshake,
    /// sign the server public key with this substrate's identity key, and wrap the
    /// stream in AES-256-GCM framing for the remainder of the session.
    TerminateEcdhP256,
    /// This substrate is an intermediate relay hop on a multi-hop path.
    /// The ECDH handshake preamble and all subsequent encrypted bytes are forwarded
    /// opaquely to the next hop — this substrate never sees plaintext.
    ///
    /// NOTE: stub implementation only. Full multi-hop relay forwarding is not yet built.
    RelayOpaqueForward,
}

/// Specifies how discrete payloads are extracted from the (possibly encrypted) byte stream.
/// **This stage is framing-only** — in all cases the substrate is consuming an already-established
/// incoming stream. No network port is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportStage {
    /// HTTP/1.1 framing. The substrate runs hyper as an in-process HTTP server
    /// consuming the existing stream — not listening on a port.
    Http,
    /// Length-prefixed binary frames. Each message is a u32-length-prefixed byte slice.
    Binary,
    /// No framing. Raw bidirectional bytes — used for passthrough/proxy scenarios only.
    /// The AdaptationStage is skipped; control passes directly to ServiceStage.
    Raw,
}

/// Converts incoming request semantics to what the backing service natively expects.
/// This is purely about semantic mismatch — if client and service speak the same protocol,
/// None is used regardless of what ServiceStage is targeted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdaptationStage {
    /// No adaptation needed. The payload format already matches what the service expects.
    /// Example: a JSON-RPC client calling a native JSON-RPC service.
    None,
    /// Incoming JSON-RPC must be unmarshalled and re-invoked as a typed WASM component
    /// guest function call. Response values are marshalled back to JSON-RPC.
    /// Used when client speaks JSON-RPC but target is a WasmComponent service.
    JsonRpcToWasm,
    /// Incoming JSON-RPC is bridged to a wRPC channel (NOTE: not yet implemented).
    /// Used when client speaks JSON-RPC but target is a wRPC-native WASM component.
    JsonRpcToWrpc,
}

/// The physical entity that handles the fully-adapted request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceStage {
    /// In-process native Rust service, identified by its registered service_id.
    /// Runs directly inside the substrate — no host:port involved.
    NativeService { service_id: String },
    /// In-process WASM guest component, identified by its registered service_id.
    /// Runs inside the AppSandboxEngine in this substrate — no host:port involved.
    WasmComponent { service_id: String },
    /// Proxy raw bytes to an externally running TCP service at host:port.
    /// Covers Podman-managed containers, sidecar processes, or any TCP server.
    TcpProxy { host: String, port: u16 },
    /// Forward the stream to the next substrate in a multi-hop relay path.
    /// `next_hop_id` is the substrate DID of the next hop, if known.
    /// If None, the next hop must be resolved via a registry lookup at dispatch time.
    ///
    /// NOTE: stub — full multi-hop relay routing is not yet implemented.
    RelayHop { next_hop_id: Option<String> },
    /// No viable service was found or the combination is unsupported.
    /// The stream will be rejected with a diagnostic error.
    Unsupported,
}

/// The fully planned execution pipeline for an incoming stream.
/// Computed once from the preamble and registry lookup, then executed stage by stage.
/// Each field is an independent decision that can be understood in isolation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePipeline {
    pub encryption: EncryptionStage,
    pub transport: TransportStage,
    pub adaptation: AdaptationStage,
    pub service: ServiceStage,
}

impl RoutePipeline {
    /// Creates a pipeline that rejects the stream with an unsupported error.
    pub fn unsupported() -> Self {
        Self {
            encryption: EncryptionStage::None,
            transport: TransportStage::Raw,
            adaptation: AdaptationStage::None,
            service: ServiceStage::Unsupported,
        }
    }
}
