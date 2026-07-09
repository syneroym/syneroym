//! Segmented streaming AEAD for blob content at rest, per ADR-0009
//! Amendment 1: `aead::stream` `StreamBE32` over AES-256-GCM, 256 KiB
//! plaintext segments, per-blob subkey derived via HKDF-SHA256 from the
//! service DEK. Also HMAC-SHA256 presigned-URL signing, deriving its key
//! from the same DEK via a distinct HKDF `info` string (no separate
//! signing-key table).

use aead::{
    KeyInit,
    stream::{DecryptorBE32, EncryptorBE32},
};
use aes_gcm::Aes256Gcm;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use zeroize::Zeroize;

use crate::errors::BlobError;

/// Plaintext bytes per AEAD segment. One 16-byte GCM tag is added per
/// segment on top of this (~0.006% overhead).
pub const SEGMENT_SIZE: usize = 256 * 1024;
const CIPHERTEXT_SEGMENT_SIZE: usize = SEGMENT_SIZE + 16;

const MAGIC: &[u8; 4] = b"SYB1";
const VERSION: u8 = 1;
/// magic(4) + version(1) + segment_size(4) + salt(32)
pub const HEADER_LEN: usize = 4 + 1 + 4 + 32;

fn derive_segment_key_and_nonce_prefix(
    dek: &[u8; 32],
    salt: &[u8; 32],
    service_id: &str,
) -> ([u8; 32], [u8; 7]) {
    let hk = Hkdf::<Sha256>::new(Some(salt), dek);
    let info = format!("syneroym:blob:v1:{service_id}");
    let mut okm = [0u8; 39];
    #[allow(clippy::expect_used)]
    hk.expand(info.as_bytes(), &mut okm).expect("39 bytes is a valid HKDF-SHA256 output length");
    let mut key = [0u8; 32];
    let mut nonce_prefix = [0u8; 7];
    key.copy_from_slice(&okm[0..32]);
    nonce_prefix.copy_from_slice(&okm[32..39]);
    okm.zeroize();
    (key, nonce_prefix)
}

fn derive_url_hmac_key(dek: &[u8; 32], service_id: &str) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, dek);
    let info = format!("syneroym:blob-url:v1:{service_id}");
    let mut key = [0u8; 32];
    #[allow(clippy::expect_used)]
    hk.expand(info.as_bytes(), &mut key).expect("32 bytes is a valid HKDF-SHA256 output length");
    key
}

/// Incremental encryptor. Feed plaintext via [`Self::update`] in
/// arbitrary-sized chunks; call [`Self::finish`] exactly once at the end
/// (even for a zero-byte blob) to seal the final segment.
pub struct BlobEncryptor {
    inner: EncryptorBE32<Aes256Gcm>,
    buf: Vec<u8>,
}

impl std::fmt::Debug for BlobEncryptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlobEncryptor")
            .field("buffered_bytes", &self.buf.len())
            .finish_non_exhaustive()
    }
}

impl BlobEncryptor {
    /// Returns the encryptor plus the header bytes the caller must write
    /// before any ciphertext produced by `update`/`finish`.
    pub fn new(dek: &[u8; 32], service_id: &str) -> (Self, Vec<u8>) {
        let mut salt = [0u8; 32];
        rand::rng().fill_bytes(&mut salt);
        let (mut key, nonce_prefix) = derive_segment_key_and_nonce_prefix(dek, &salt, service_id);
        #[allow(clippy::expect_used)]
        let cipher = Aes256Gcm::new_from_slice(&key)
            .expect("derived key is always exactly 32 bytes for AES-256");
        key.zeroize();
        let inner = EncryptorBE32::from_aead(cipher, &nonce_prefix.into());

        let mut header = Vec::with_capacity(HEADER_LEN);
        header.extend_from_slice(MAGIC);
        header.push(VERSION);
        header.extend_from_slice(&(SEGMENT_SIZE as u32).to_be_bytes());
        header.extend_from_slice(&salt);

        (Self { inner, buf: Vec::with_capacity(SEGMENT_SIZE) }, header)
    }

    /// Feeds `chunk` in; returns any complete ciphertext segments it was
    /// able to produce (may be empty if not enough plaintext has
    /// accumulated yet).
    pub fn update(&mut self, chunk: &[u8]) -> Result<Vec<u8>, BlobError> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while self.buf.len() >= SEGMENT_SIZE {
            let segment: Vec<u8> = self.buf.drain(..SEGMENT_SIZE).collect();
            let ciphertext = self
                .inner
                .encrypt_next(segment.as_slice())
                .map_err(|_| BlobError::Internal("blob encryption failed".to_string()))?;
            out.extend_from_slice(&ciphertext);
        }
        Ok(out)
    }

    /// Seals whatever remains (possibly empty) as the final segment.
    pub fn finish(self) -> Result<Vec<u8>, BlobError> {
        self.inner
            .encrypt_last(self.buf.as_slice())
            .map_err(|_| BlobError::Internal("blob encryption failed".to_string()))
    }
}

/// Incremental decryptor, the mirror of [`BlobEncryptor`]. Correctness
/// relies on never decrypting a segment via `decrypt_next` until it is
/// certain more ciphertext follows it -- otherwise a truncated stream could
/// be mistaken for a complete one. See module tests for the truncation and
/// reordering cases this structurally rejects.
pub struct BlobDecryptor {
    inner: DecryptorBE32<Aes256Gcm>,
    buf: Vec<u8>,
}

impl std::fmt::Debug for BlobDecryptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlobDecryptor")
            .field("buffered_bytes", &self.buf.len())
            .finish_non_exhaustive()
    }
}

impl BlobDecryptor {
    /// Parses `header` (exactly [`HEADER_LEN`] bytes) and prepares a
    /// decryptor for the segments that follow it.
    pub fn new(dek: &[u8; 32], service_id: &str, header: &[u8]) -> Result<Self, BlobError> {
        if header.len() != HEADER_LEN || &header[0..4] != MAGIC {
            return Err(BlobError::Internal("integrity check failed".to_string()));
        }
        if header[4] != VERSION {
            return Err(BlobError::Internal("integrity check failed".to_string()));
        }
        let segment_size = u32::from_be_bytes([header[5], header[6], header[7], header[8]]);
        if segment_size as usize != SEGMENT_SIZE {
            return Err(BlobError::Internal("integrity check failed".to_string()));
        }
        let mut salt = [0u8; 32];
        salt.copy_from_slice(&header[9..41]);

        let (mut key, nonce_prefix) = derive_segment_key_and_nonce_prefix(dek, &salt, service_id);
        #[allow(clippy::expect_used)]
        let cipher = Aes256Gcm::new_from_slice(&key)
            .expect("derived key is always exactly 32 bytes for AES-256");
        key.zeroize();
        let inner = DecryptorBE32::from_aead(cipher, &nonce_prefix.into());

        Ok(Self { inner, buf: Vec::with_capacity(CIPHERTEXT_SEGMENT_SIZE) })
    }

    /// Feeds ciphertext bytes (as read from storage) in; returns any
    /// plaintext it was able to safely decrypt (may be empty).
    pub fn update(&mut self, chunk: &[u8]) -> Result<Vec<u8>, BlobError> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        // Strictly greater-than: a segment is only decrypted via
        // decrypt_next once buffered bytes prove it is *not* the final
        // segment (i.e. at least one more byte follows it).
        while self.buf.len() > CIPHERTEXT_SEGMENT_SIZE {
            let segment: Vec<u8> = self.buf.drain(..CIPHERTEXT_SEGMENT_SIZE).collect();
            let plaintext = self
                .inner
                .decrypt_next(segment.as_slice())
                .map_err(|_| BlobError::Internal("integrity check failed".to_string()))?;
            out.extend_from_slice(&plaintext);
        }
        Ok(out)
    }

    /// Decrypts whatever remains as the final segment. Call exactly once,
    /// after the ciphertext source is exhausted.
    pub fn finish(self) -> Result<Vec<u8>, BlobError> {
        self.inner
            .decrypt_last(self.buf.as_slice())
            .map_err(|_| BlobError::Internal("integrity check failed".to_string()))
    }
}

/// Computes an HMAC-SHA256 presigned-URL string for `hash`, valid until
/// `now_unix + ttl_secs`. Does not verify the blob exists; callers should
/// check that separately.
#[must_use]
pub fn sign_url(
    dek: &[u8; 32],
    service_id: &str,
    hash: &str,
    ttl_secs: u32,
    now_unix: u64,
) -> String {
    let exp = now_unix + u64::from(ttl_secs);
    let mut key = derive_url_hmac_key(dek, service_id);
    #[allow(clippy::expect_used)]
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(&key).expect("HMAC-SHA256 accepts any key length");
    key.zeroize();
    mac.update(format!("{service_id}|{hash}|{exp}").as_bytes());
    let sig = mac.finalize().into_bytes();
    let sig_hex = hex::encode(sig);
    format!("blobs/{hash}?svc={service_id}&exp={exp}&sig={sig_hex}")
}

/// Recomputes and constant-time-compares the HMAC, and checks `exp` against
/// `now_unix`. Pure/stateless so it's testable without any live serving
/// endpoint (none exists yet -- see status.md).
pub fn verify_signed_url(
    dek: &[u8; 32],
    service_id: &str,
    hash: &str,
    exp: u64,
    sig_hex: &str,
    now_unix: u64,
) -> Result<(), BlobError> {
    if exp <= now_unix {
        return Err(BlobError::Internal("signed url expired".to_string()));
    }
    let expected_sig =
        hex::decode(sig_hex).map_err(|_| BlobError::Internal("malformed signature".to_string()))?;
    let mut key = derive_url_hmac_key(dek, service_id);
    #[allow(clippy::expect_used)]
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(&key).expect("HMAC-SHA256 accepts any key length");
    key.zeroize();
    mac.update(format!("{service_id}|{hash}|{exp}").as_bytes());
    mac.verify_slice(&expected_sig)
        .map_err(|_| BlobError::Internal("invalid signature".to_string()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn roundtrip(dek: &[u8; 32], service_id: &str, plaintext: &[u8], chunk_size: usize) -> Vec<u8> {
        let (mut enc, header) = BlobEncryptor::new(dek, service_id);
        let mut ciphertext = header.clone();
        for chunk in plaintext.chunks(chunk_size.max(1)) {
            ciphertext.extend(enc.update(chunk).unwrap());
        }
        ciphertext.extend(enc.finish().unwrap());

        let mut dec = BlobDecryptor::new(dek, service_id, &ciphertext[0..HEADER_LEN]).unwrap();
        let body = &ciphertext[HEADER_LEN..];
        let mut plaintext_out = Vec::new();
        for chunk in body.chunks(chunk_size.max(1)) {
            plaintext_out.extend(dec.update(chunk).unwrap());
        }
        plaintext_out.extend(dec.finish().unwrap());
        plaintext_out
    }

    #[test]
    fn round_trip_small_single_chunk() {
        let dek = [7u8; 32];
        let plaintext = b"hello syneroym blob store";
        let out = roundtrip(&dek, "svc-1", plaintext, plaintext.len());
        assert_eq!(out, plaintext);
    }

    #[test]
    fn round_trip_empty_blob() {
        let dek = [7u8; 32];
        let out = roundtrip(&dek, "svc-1", b"", 1024);
        assert_eq!(out, Vec::<u8>::new());
    }

    #[test]
    fn round_trip_multi_segment_arbitrary_chunking() {
        let dek = [3u8; 32];
        // 2.5 segments, fed in small, non-aligned 777-byte writes.
        let plaintext: Vec<u8> =
            (0..(SEGMENT_SIZE * 2 + SEGMENT_SIZE / 2)).map(|i| (i % 256) as u8).collect();
        let out = roundtrip(&dek, "svc-multi", &plaintext, 777);
        assert_eq!(out, plaintext);
    }

    #[test]
    fn round_trip_exact_segment_boundary() {
        let dek = [9u8; 32];
        let plaintext = vec![42u8; SEGMENT_SIZE];
        let out = roundtrip(&dek, "svc-exact", &plaintext, SEGMENT_SIZE);
        assert_eq!(out, plaintext);
    }

    #[test]
    fn truncated_ciphertext_is_rejected() {
        let dek = [1u8; 32];
        let plaintext: Vec<u8> = (0..(SEGMENT_SIZE * 2)).map(|i| (i % 256) as u8).collect();
        let (mut enc, header) = BlobEncryptor::new(&dek, "svc");
        let mut ciphertext = header;
        ciphertext.extend(enc.update(&plaintext).unwrap());
        ciphertext.extend(enc.finish().unwrap());

        // Drop the final sealed (empty) segment -- the last CIPHERTEXT_SEGMENT_SIZE
        // worth of bytes is the true final segment's tag; simulate loss of
        // the tail of the stream.
        let truncated = &ciphertext[0..ciphertext.len() - 8];

        let mut dec = BlobDecryptor::new(&dek, "svc", &truncated[0..HEADER_LEN]).unwrap();
        let body = &truncated[HEADER_LEN..];
        let mut result = Ok(Vec::new());
        for chunk in body.chunks(4096) {
            match dec.update(chunk) {
                Ok(pt) => {
                    if let Ok(acc) = &mut result {
                        acc.extend(pt);
                    }
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        // Either update() already failed, or the final finish() must fail
        // because the stream was cut mid-segment / missing its true last
        // sealed block.
        if result.is_ok() {
            let dec = dec;
            assert!(dec.finish().is_err(), "truncated stream must not decrypt cleanly");
        }
    }

    #[test]
    fn reordered_segments_are_rejected() {
        let dek = [5u8; 32];
        let plaintext: Vec<u8> = (0..(SEGMENT_SIZE * 3)).map(|i| (i % 256) as u8).collect();
        let (mut enc, header) = BlobEncryptor::new(&dek, "svc");
        let mut segments = Vec::new();
        for chunk in plaintext.chunks(SEGMENT_SIZE) {
            let ct = enc.update(chunk).unwrap();
            if !ct.is_empty() {
                segments.push(ct);
            }
        }
        let last = enc.finish().unwrap();

        // Swap the first two full segments.
        segments.swap(0, 1);
        let mut ciphertext = header;
        for s in &segments {
            ciphertext.extend_from_slice(s);
        }
        ciphertext.extend(last);

        let mut dec = BlobDecryptor::new(&dek, "svc", &ciphertext[0..HEADER_LEN]).unwrap();
        let body = &ciphertext[HEADER_LEN..];
        let mut saw_error = false;
        for chunk in body.chunks(CIPHERTEXT_SEGMENT_SIZE) {
            if dec.update(chunk).is_err() {
                saw_error = true;
                break;
            }
        }
        assert!(saw_error, "reordered segments must fail nonce/tag verification");
    }

    #[test]
    fn corrupted_byte_is_rejected() {
        let dek = [11u8; 32];
        let plaintext = b"some plaintext content to corrupt";
        let (mut enc, header) = BlobEncryptor::new(&dek, "svc");
        let mut ciphertext = header;
        ciphertext.extend(enc.update(plaintext).unwrap());
        ciphertext.extend(enc.finish().unwrap());

        let flip_at = ciphertext.len() - 1;
        ciphertext[flip_at] ^= 0xFF;

        let mut dec = BlobDecryptor::new(&dek, "svc", &ciphertext[0..HEADER_LEN]).unwrap();
        let body = &ciphertext[HEADER_LEN..];
        assert!(
            dec.update(body).is_err() || {
                let dec = dec;
                dec.finish().is_err()
            }
        );
    }

    #[test]
    fn sign_and_verify_url_round_trip() {
        let dek = [4u8; 32];
        let url = sign_url(&dek, "svc-a", "deadbeef", 60, 1_000);
        // Parse exp/sig back out of the generated URL for the test.
        let exp: u64 =
            url.split("exp=").nth(1).unwrap().split('&').next().unwrap().parse().unwrap();
        let sig = url.split("sig=").nth(1).unwrap();
        assert!(verify_signed_url(&dek, "svc-a", "deadbeef", exp, sig, 1_030).is_ok());
    }

    #[test]
    fn expired_url_is_rejected() {
        let dek = [4u8; 32];
        let url = sign_url(&dek, "svc-a", "deadbeef", 60, 1_000);
        let exp: u64 =
            url.split("exp=").nth(1).unwrap().split('&').next().unwrap().parse().unwrap();
        let sig = url.split("sig=").nth(1).unwrap();
        assert!(verify_signed_url(&dek, "svc-a", "deadbeef", exp, sig, 2_000).is_err());
    }

    #[test]
    fn tampered_signature_is_rejected() {
        let dek = [4u8; 32];
        let url = sign_url(&dek, "svc-a", "deadbeef", 60, 1_000);
        let exp: u64 =
            url.split("exp=").nth(1).unwrap().split('&').next().unwrap().parse().unwrap();
        assert!(verify_signed_url(&dek, "svc-a", "deadbeef", exp, "00", 1_030).is_err());
    }
}
