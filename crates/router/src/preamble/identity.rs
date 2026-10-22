use std::fmt::{self, Formatter};

use anyhow::{Result, anyhow};
use ed25519_dalek::VerifyingKey;
use syneroym_identity::{DelegationCertificate, substrate};
use syneroym_rpc::OnBehalfProof;
use syneroym_ucan::CapabilityToken;

/// The identity parameters of a preamble. On a direct connection they are
/// part of the preamble line; on a stream with `enc=ecdh-p256` the client
/// sends them as one encrypted line after the handshake.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct IdentityFields {
    pub pubkey: Option<String>,
    pub delegation: Option<DelegationCertificate>,
    pub ucan: Option<CapabilityToken>,
    pub ts: Option<u64>,
    pub sig: Option<String>,
    pub on_behalf: Option<OnBehalfProof>,
}

/// A borrowed view, so `Display` of a preamble writes the same parameters
/// without cloning certificates and chains.
pub(crate) struct IdentityRef<'a> {
    pub pubkey: Option<&'a str>,
    pub delegation: Option<&'a DelegationCertificate>,
    pub ucan: Option<&'a CapabilityToken>,
    pub ts: Option<u64>,
    pub sig: Option<&'a str>,
    pub on_behalf: Option<&'a OnBehalfProof>,
}

impl IdentityRef<'_> {
    /// Pushes `key=value` for each present field, in wire order:
    /// pubkey, delegation, ucan, ts, sig, on_behalf.
    pub(crate) fn push_params(&self, params: &mut Vec<String>) {
        if let Some(pubkey) = self.pubkey {
            params.push(format!("pubkey={pubkey}"));
        }
        if let Some(delegation) = self.delegation
            && let Ok(json_str) = delegation.to_json()
        {
            params.push(format!("delegation={}", hex::encode(json_str)));
        }
        if let Some(ucan) = self.ucan
            && let Ok(json) = serde_json::to_string(ucan)
        {
            params.push(format!("ucan={}", hex::encode(json)));
        }
        if let Some(ts) = self.ts {
            params.push(format!("ts={ts}"));
        }
        if let Some(sig) = self.sig {
            params.push(format!("sig={sig}"));
        }
        if let Some(on_behalf) = self.on_behalf
            && let Ok(json) = serde_json::to_string(on_behalf)
        {
            params.push(format!("on_behalf={}", hex::encode(json)));
        }
    }
}

impl IdentityFields {
    pub(crate) fn view(&self) -> IdentityRef<'_> {
        IdentityRef {
            pubkey: self.pubkey.as_deref(),
            delegation: self.delegation.as_ref(),
            ucan: self.ucan.as_ref(),
            ts: self.ts,
            sig: self.sig.as_deref(),
            on_behalf: self.on_behalf.as_ref(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pubkey.is_none()
            && self.delegation.is_none()
            && self.ucan.is_none()
            && self.ts.is_none()
            && self.sig.is_none()
            && self.on_behalf.is_none()
    }

    /// The encrypted identity line: the parameters and `\n`. An empty
    /// identity is the line `\n`.
    #[must_use]
    pub fn to_line(&self) -> String {
        let mut params = Vec::new();
        self.view().push_params(&mut params);
        format!("{}\n", params.join("&"))
    }

    /// Parses an identity line. Only identity keys are allowed.
    pub fn parse_line(raw: &str) -> Result<Self> {
        let mut out = Self::default();
        let body = raw.trim_end_matches(['\n', '\r']);
        if body.is_empty() {
            return Ok(out);
        }
        for part in body.split('&') {
            let (k, v) = part
                .split_once('=')
                .ok_or_else(|| anyhow!("identity line: parameter without '='"))?;
            if !out.parse_param(k, v)? {
                return Err(anyhow!("identity line: '{k}' is not an identity parameter"));
            }
        }
        Ok(out)
    }

    /// Sets one identity parameter. `Ok(false)` when `k` is not one.
    pub(crate) fn parse_param(&mut self, k: &str, v: &str) -> Result<bool> {
        match k {
            "pubkey" => self.pubkey = Some(v.to_string()),
            "delegation" => self.delegation = decode_delegation(v),
            "ucan" => self.ucan = decode_ucan(v),
            "ts" => {
                let parsed = v.parse::<u64>().map_err(|_| anyhow!("invalid ts in preamble"))?;
                self.ts = Some(parsed);
            }
            "sig" => {
                if v.len() != 128
                    || !v.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
                {
                    return Err(anyhow!("invalid sig in preamble"));
                }
                self.sig = Some(v.to_string());
            }
            "on_behalf" => self.on_behalf = Some(decode_on_behalf(v)?),
            _ => return Ok(false),
        }
        Ok(true)
    }
}

pub(crate) fn decode_delegation(v: &str) -> Option<DelegationCertificate> {
    let bytes = hex::decode(v).ok()?;
    let json_str = String::from_utf8(bytes).ok()?;
    DelegationCertificate::from_json(&json_str).ok()
}

pub(crate) fn decode_ucan(v: &str) -> Option<CapabilityToken> {
    let bytes = hex::decode(v).ok()?;
    let json_str = String::from_utf8(bytes).ok()?;
    serde_json::from_str::<CapabilityToken>(&json_str).ok()
}

pub(crate) fn decode_on_behalf(v: &str) -> Result<OnBehalfProof> {
    let bytes = hex::decode(v).map_err(|e| anyhow!("invalid on_behalf hex: {e}"))?;
    let json_str = String::from_utf8(bytes).map_err(|e| anyhow!("invalid on_behalf utf-8: {e}"))?;
    serde_json::from_str::<OnBehalfProof>(&json_str)
        .map_err(|e| anyhow!("invalid on_behalf json: {e}"))
}

impl fmt::Debug for IdentityFields {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let caller_did = self.pubkey.as_deref().and_then(|pk_hex| {
            let bytes = hex::decode(pk_hex).ok()?;
            let array: [u8; 32] = bytes.try_into().ok()?;
            let vk = VerifyingKey::from_bytes(&array).ok()?;
            Some(substrate::derive_did_key(&vk))
        });
        let master = self.delegation.as_ref().map(|c| c.master_did.as_str()).unwrap_or("-");
        let sig = if self.sig.is_some() { "set" } else { "-" };
        let ucan = if self.ucan.is_some() { "set" } else { "-" };
        let on_behalf = if self.on_behalf.is_some() { "set" } else { "-" };
        f.debug_struct("IdentityFields")
            .field("caller", &caller_did.as_deref().unwrap_or("-"))
            .field("master", &master)
            .field("ts", &self.ts)
            .field("sig", &sig)
            .field("ucan", &ucan)
            .field("on_behalf", &on_behalf)
            .finish()
    }
}
