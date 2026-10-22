use anyhow::{Result, anyhow};
use hkdf::Hkdf;
use sha2::Sha256;

const KDF_SALT: &[u8] = b"syneroym-e2e-v1";

pub(crate) struct ChannelKeys {
    pub client_to_server: [u8; 32],
    pub server_to_client: [u8; 32],
}

/// HKDF-SHA256 over the ECDH secret. Each direction gets its own key, so
/// bytes sent one way never decrypt when sent back the other way. Both
/// public keys are in `info`, so the keys belong to this one handshake.
pub(crate) fn derive(shared_x: &[u8], client_epk: &[u8], server_pk: &[u8]) -> Result<ChannelKeys> {
    let hk = Hkdf::<Sha256>::new(Some(KDF_SALT), shared_x);
    let expand = |label: &[u8]| -> Result<[u8; 32]> {
        let info = [label, client_epk, server_pk].concat();
        let mut key = [0u8; 32];
        hk.expand(&info, &mut key).map_err(|e| anyhow!("hkdf expand: {e}"))?;
        Ok(key)
    };
    Ok(ChannelKeys { client_to_server: expand(b"c2s")?, server_to_client: expand(b"s2c")? })
}
