use std::{fmt, io, time::Duration};

use aes_gcm::aead::OsRng;
use anyhow::Result;
use ed25519_dalek::{Signature, VerifyingKey};
use p256::{EncodedPoint, PublicKey, ecdh::EphemeralSecret};
use syneroym_identity::substrate;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    time,
};

use crate::{
    e2e_channel::{
        P256_POINT_LEN, SEC1_UNCOMPRESSED, cipher,
        cipher::{OwnedStream, ReaderWriter},
        kdf,
    },
    preamble::{ENC_ECDH_P256, IdentityFields, RoutePreamble},
};

pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub enum Path {
    Direct,
    Indirect(VerifyingKey),
}

/// Computes the path for reaching the target service.
///
/// `dialed`: the Iroh endpoint id the connection reached (32 bytes).
/// `host`: the node key the client expects to host `target`.
pub fn path(
    dialed: &[u8; 32],
    host: Option<&VerifyingKey>,
    target: &str,
) -> Result<Path, E2eError> {
    match host {
        Some(h) if h.as_bytes() == dialed => Ok(Path::Direct),
        Some(h) => Ok(Path::Indirect(*h)),
        None => Err(E2eError::HostKeyUnknown { target: target.to_string() }),
    }
}

/// A stream ready for the request, or one the destination refused before
/// any handshake. A refused stream holds the refusal, unread, in the
/// framing the preamble asked for.
pub enum Opened {
    Ready(OwnedStream),
    Refused(OwnedStream),
}

impl fmt::Debug for Opened {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready(_) => f.debug_tuple("Opened::Ready").finish(),
            Self::Refused(_) => f.debug_tuple("Opened::Refused").finish(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum E2eError {
    #[error(
        "no host key for '{target}': the client cannot tell a direct connection from a forwarder, \
         so it sends no identity; connect through the registry or give the host with `with_host`"
    )]
    HostKeyUnknown { target: String },
    #[error(
        "the node that answered did not prove it is host '{host}' of '{target}' (bad handshake \
         signature); no identity was sent. If '{target}' is a service and not a node, give its \
         host with `with_host` or a registry URL"
    )]
    BadHostSignature { host: String, target: String },
    #[error("no handshake reply within {0:?}; no identity was sent")]
    Timeout(Duration),
    #[error("the stream closed before the handshake; no identity was sent")]
    Closed,
    #[error("invalid handshake reply: {0}")]
    BadReply(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Writes `preamble` (with its identity fields set and signed) on a new
/// stream. On a direct path it writes it as it is. On an indirect path it
/// moves the identity out of the preamble, runs the handshake, checks the
/// host's signature, and sends the identity as one encrypted line.
pub async fn open_stream<R, W>(
    reader: R,
    mut writer: W,
    mut preamble: RoutePreamble,
    path: &Path,
) -> Result<Opened, E2eError>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let host = match path {
        Path::Direct => {
            writer.write_all(preamble.to_preamble_line().as_bytes()).await?;
            return Ok(Opened::Ready(plain(reader, writer)));
        }
        Path::Indirect(host) => host,
    };
    let identity = preamble.take_identity();
    let secret = EphemeralSecret::random(&mut OsRng);
    let client_epk = EncodedPoint::from(secret.public_key());
    preamble.enc = Some(ENC_ECDH_P256.to_string());
    preamble.epk = Some(hex::encode(client_epk.as_bytes()));
    writer.write_all(preamble.to_preamble_line().as_bytes()).await?;
    writer.flush().await?;

    let mut reader = BufReader::new(reader);
    let reply = time::timeout(HANDSHAKE_TIMEOUT, read_reply(&mut reader))
        .await
        .map_err(|_| E2eError::Timeout(HANDSHAKE_TIMEOUT))??;
    let Some((server_pk, sig)) = reply else {
        return Ok(Opened::Refused(plain(reader, writer)));
    };
    let params = HandshakeParams {
        secret,
        client_epk,
        server_pk,
        sig,
        host,
        target: &preamble.service_id,
        identity,
    };
    finish_handshake(reader, writer, params).await
}

struct HandshakeParams<'a> {
    secret: EphemeralSecret,
    client_epk: EncodedPoint,
    server_pk: [u8; 65],
    sig: [u8; 64],
    host: &'a VerifyingKey,
    target: &'a str,
    identity: IdentityFields,
}

async fn finish_handshake<R, W>(
    reader: BufReader<R>,
    writer: W,
    params: HandshakeParams<'_>,
) -> Result<Opened, E2eError>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let signed = super::handshake_signed_bytes(&params.server_pk, params.client_epk.as_bytes());
    let signature = Signature::from_bytes(&params.sig);
    params.host.verify_strict(&signed, &signature).map_err(|_| E2eError::BadHostSignature {
        host: substrate::derive_did_key(params.host),
        target: params.target.to_string(),
    })?;
    let server_point = PublicKey::from_sec1_bytes(&params.server_pk)
        .map_err(|e| E2eError::BadReply(e.to_string()))?;
    let shared = params.secret.diffie_hellman(&server_point);
    let keys =
        kdf::derive(shared.raw_secret_bytes(), params.client_epk.as_bytes(), &params.server_pk)
            .map_err(|e| E2eError::BadReply(e.to_string()))?;
    let mut stream = cipher::wrap(reader, writer, &keys.client_to_server, &keys.server_to_client);
    stream.writer.write_all(params.identity.to_line().as_bytes()).await?;
    stream.writer.flush().await?;
    Ok(Opened::Ready(stream))
}

async fn read_reply<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Result<Option<([u8; 65], [u8; 64])>, E2eError> {
    let first = reader.fill_buf().await?;
    match first.first() {
        None => return Err(E2eError::Closed),
        Some(&b) if b != SEC1_UNCOMPRESSED => return Ok(None),
        Some(_) => {}
    }
    let mut buf = [0u8; P256_POINT_LEN + 64];
    reader.read_exact(&mut buf).await?;
    let mut server_pk = [0u8; P256_POINT_LEN];
    let mut sig = [0u8; 64];
    server_pk.copy_from_slice(&buf[..P256_POINT_LEN]);
    sig.copy_from_slice(&buf[P256_POINT_LEN..]);
    Ok(Some((server_pk, sig)))
}

fn plain<R, W>(reader: R, writer: W) -> OwnedStream
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    ReaderWriter { reader: Box::new(reader), writer: Box::new(writer) }
}
