//! The end-to-end channel inside a stream (`enc=ecdh-p256`): the client
//! side of the handshake, the key derivation and the chunk cipher that both
//! sides use (ADR-0002 A1 rules 4 and 5).
mod cipher;
mod client;
mod kdf;
#[cfg(test)]
mod tests;

pub(crate) use cipher::wrap;
pub use cipher::{OwnedStream, ReaderWriter};
pub use client::{E2eError, HANDSHAKE_TIMEOUT, Opened, Path, open_stream, path};
pub(crate) use kdf::derive;

/// The first byte of a SEC1 uncompressed point. The server's handshake
/// reply starts with it; a refusal written before the handshake never does.
pub(crate) const SEC1_UNCOMPRESSED: u8 = 0x04;
/// Length of a SEC1 uncompressed P-256 point.
pub(crate) const P256_POINT_LEN: usize = 65;

/// Tag in front of the bytes the host signs in the handshake. The node key
/// also signs preamble proofs and registry records, so the tag keeps a
/// handshake signature from ever matching another signed format.
const HANDSHAKE_SIGN_TAG: &[u8] = b"syneroym-e2e-handshake-v1";

/// The bytes the host signs with its node key: the tag, its one-time
/// P-256 key, then the client's. `peer-proxy.js` builds the same bytes.
pub(crate) fn handshake_signed_bytes(server_pk: &[u8], client_epk: &[u8]) -> Vec<u8> {
    [HANDSHAKE_SIGN_TAG, server_pk, client_epk].concat()
}
