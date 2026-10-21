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
