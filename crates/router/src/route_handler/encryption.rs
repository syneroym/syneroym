//! Server-side stream encryption and decryption stage
//!
//! Encapsulates the destination side of the ECDH-P256 handshake, signing
//! the server ephemeral key and deriving channel keys via HKDF.

use aes_gcm::aead::OsRng;
use anyhow::{Result, anyhow};
use p256::{EncodedPoint, PublicKey, ecdh::EphemeralSecret};
use syneroym_identity::Identity;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::{
    e2e_channel::{
        self, OwnedStream, P256_POINT_LEN, ReaderWriter, SEC1_UNCOMPRESSED, derive as derive_kdf,
    },
    preamble::RoutePreamble,
    routing::EncryptionStage,
};

/// Applies the encryption stage to the stream, returning an `OwnedStream`
/// that downstream transport/service stages can use without knowing
/// whether the wire is encrypted.
pub async fn apply_encryption_stage<R, W>(
    reader: R,
    mut writer: W,
    stage: &EncryptionStage,
    preamble: &RoutePreamble,
    identity: &Identity,
) -> Result<OwnedStream>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    match stage {
        EncryptionStage::None => {
            Ok(ReaderWriter { reader: Box::new(reader), writer: Box::new(writer) })
        }
        EncryptionStage::TerminateEcdhP256 => {
            let epk_hex = preamble.epk.as_deref().ok_or_else(|| {
                anyhow!("Missing 'epk' query parameter required for ecdh-p256 encryption")
            })?;
            let client_epk =
                hex::decode(epk_hex).map_err(|e| anyhow!("Invalid hex epk in preamble: {e}"))?;
            if client_epk.len() != P256_POINT_LEN || client_epk[0] != SEC1_UNCOMPRESSED {
                return Err(anyhow!("epk must be a 65-byte uncompressed P-256 point"));
            }
            let client_point = PublicKey::from_sec1_bytes(&client_epk)
                .map_err(|e| anyhow!("Invalid epk point: {e}"))?;
            let secret = EphemeralSecret::random(&mut OsRng);
            let server_pk = EncodedPoint::from(secret.public_key());
            let shared = secret.diffie_hellman(&client_point);
            let keys = derive_kdf(shared.raw_secret_bytes(), &client_epk, server_pk.as_bytes())?;
            let signed = e2e_channel::handshake_signed_bytes(server_pk.as_bytes(), &client_epk);
            let signature = identity.sign(&signed);
            writer.write_all(server_pk.as_bytes()).await?;
            writer.write_all(&signature.to_bytes()).await?;
            writer.flush().await?;
            Ok(e2e_channel::wrap(reader, writer, &keys.server_to_client, &keys.client_to_server))
        }
    }
}

#[cfg(test)]
mod tests;
