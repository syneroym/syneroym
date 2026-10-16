//! `home_relay_passport.json`: keeps the home-relay passport and child token
//! across restarts. Written with mode 0600.

use std::{fs, io::ErrorKind, path::Path};

use anyhow::Context;
use iroh::RelayUrl;
use serde::{Deserialize, Serialize};
use syneroym_core::curated_list;
use syneroym_passport::{CapabilityToken, PresentedToken, RELAY_HOME, wire::HomeRelaySetRequest};
use syneroym_router::net_iroh;
use syneroym_rpc::{RpcError, RpcResult};
use tracing::warn;

use super::client::MAX_CA_CERT_BYTES;

/// The file content. Holds the child token and so the passport: written
/// with mode 0600.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct StoredPassport {
    /// Normalised with `curated_list::normalize_url`.
    pub(super) relay_url: String,
    /// Normalised, `https`.
    pub(super) info_url: String,
    pub(super) token: CapabilityToken,
    #[serde(default)]
    pub(super) ca_cert_pem: Option<String>,
}

impl StoredPassport {
    /// Checks a control-plane request. Every error is `InvalidParams` with a
    /// message the owner can act on.
    pub(super) fn from_request(
        request: HomeRelaySetRequest,
        node_did: &str,
        fixed_url: &RelayUrl,
        now_secs: u64,
    ) -> RpcResult<(Self, PresentedToken)> {
        let info_url = curated_list::normalize_url(&request.info_url)
            .map_err(|e| RpcError::InvalidParams(format!("info_url: {e}")))?;
        if !info_url.starts_with("https://") {
            return Err(RpcError::InvalidParams(
                "info_url must be https: the token is sensitive".into(),
            ));
        }
        let relay_url = match request.relay_url {
            Some(raw) => net_iroh::parse_relay_url("relay_url", &raw)
                .map_err(|e| RpcError::InvalidParams(format!("relay_url: {e}")))?,
            None => fixed_url.clone(),
        };
        let relay_url = curated_list::normalize_url(relay_url.as_str())
            .map_err(|e| RpcError::InvalidParams(format!("relay_url: {e}")))?;
        if let Some(pem) = &request.ca_cert_pem {
            if pem.len() > MAX_CA_CERT_BYTES {
                return Err(RpcError::InvalidParams(format!(
                    "ca_cert_pem is longer than {MAX_CA_CERT_BYTES} bytes"
                )));
            }
            if !pem.contains("-----BEGIN CERTIFICATE-----") {
                return Err(RpcError::InvalidParams(
                    "ca_cert_pem does not contain a PEM certificate".into(),
                ));
            }
            let cert = reqwest::Certificate::from_pem(pem.as_bytes())
                .map_err(|e| RpcError::InvalidParams(format!("ca_cert_pem: {e}")))?;
            reqwest::Client::builder()
                .add_root_certificate(cert)
                .build()
                .map_err(|e| RpcError::InvalidParams(format!("ca_cert_pem: {e}")))?;
        }
        let presented =
            syneroym_passport::inspect_child_token(&request.token, &RELAY_HOME, node_did, now_secs)
                .map_err(|e| RpcError::InvalidParams(format!("{}: {e}", e.code())))?;
        Ok((
            StoredPassport {
                relay_url,
                info_url,
                token: request.token,
                ca_cert_pem: request.ca_cert_pem,
            },
            presented,
        ))
    }

    /// Whether this passport is for the configured relay.
    pub(super) fn is_for(&self, fixed_url: &RelayUrl) -> bool {
        let Ok(fixed) = curated_list::normalize_url(fixed_url.as_str()) else {
            return false;
        };
        let Ok(stored) = curated_list::normalize_url(&self.relay_url) else {
            return false;
        };
        fixed == stored
    }
}

/// A missing file is `None`. An unreadable or invalid file is `None` with a
/// warning; the next `set` replaces it.
pub(super) fn load(path: &Path) -> Option<StoredPassport> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == ErrorKind::NotFound => return None,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "cannot read the home-relay passport; ignoring it");
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(stored) => Some(stored),
        Err(e) => {
            warn!(path = %path.display(), error = %e, "home-relay passport file is not valid; ignoring it");
            None
        }
    }
}

pub(super) fn save(path: &Path, stored: &StoredPassport) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(stored).context("encoding the home-relay passport")?;
    super::super::write_file_atomic(path, &bytes)
}
